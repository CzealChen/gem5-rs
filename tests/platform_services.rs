use gem5rs::{
    console,
    cpu::Cpu,
    experiment::{Config, Experiment},
    m5,
    memory::*,
};
use std::{cell::RefCell, num::NonZeroU64};

fn run(words: &[u32], enabled: bool) -> (Cpu, RunResult) {
    let mut cpu = Cpu::new(0, 4096, 2);
    if enabled {
        cpu = cpu.with_experiment(
            Experiment::new(Config {
                tick_femtoseconds: 1_000_000,
                instruction_budget: 100,
            })
            .unwrap(),
        );
    }
    let mut machine = Machine::new(vec![0]).unwrap();
    machine
        .add_route(Route {
            base: 0,
            target: Target::FixedRam {
                size_bytes: 4096,
                latency: 3,
                capacity: 1,
            },
        })
        .unwrap();
    machine
        .load_ram(
            0,
            &words
                .iter()
                .flat_map(|w| w.to_le_bytes())
                .collect::<Vec<_>>(),
        )
        .unwrap();
    machine.add_source(0, 0, &mut cpu).unwrap();
    let result = machine
        .run(RunLimits {
            events: 1000,
            ticks: 1000,
        })
        .unwrap();
    (cpu, result)
}

#[test]
fn final_exit_retires_once_and_does_not_fetch_younger_instruction() {
    let (cpu, result) = run(
        &[
            m5::Operation::Rpns.instruction(),
            0x00000513,
            m5::Operation::Exit.instruction(),
            0x00100073,
        ],
        true,
    );
    assert_eq!(result.stop, StopReason::Exit(0));
    assert_eq!(cpu.commits().len(), 3);
    assert_eq!(cpu.commits()[0].writeback.unwrap().value, 5);
    assert_eq!(cpu.registers()[10], 0);
    assert_eq!(cpu.pc(), 12);
    assert_eq!(result.transactions.len(), 3);
    assert_eq!(result.tick, 15);
}

#[test]
fn disabled_unknown_wrong_path_and_older_fault_keep_their_semantics() {
    let (cpu, _) = run(&[m5::Operation::Exit.instruction()], false);
    assert!(cpu.fault().unwrap().reason.contains("IllegalInstruction"));
    let (cpu, _) = run(&[0x7e00007b], true);
    assert!(cpu.fault().unwrap().reason.contains("IllegalInstruction"));
    // jal x0,+8 skips a fail operation on the wrong path.
    let (cpu, result) = run(
        &[
            0x0080006f,
            m5::Operation::Fail.instruction(),
            m5::Operation::Exit.instruction(),
        ],
        true,
    );
    assert_eq!(result.stop, StopReason::Exit(0));
    assert_eq!(cpu.commits().len(), 2);
    let (cpu, _) = run(&[0x00100073, m5::Operation::Exit.instruction()], true);
    assert!(cpu.commits().is_empty());
    assert_eq!(cpu.experiment().unwrap().exit(), None);
}

struct Writes;
impl Source for Writes {
    fn on_event(&mut self, _: u64, event: SourceEvent) -> Vec<SourceAction> {
        if !matches!(event, SourceEvent::Wake) {
            return vec![];
        }
        [b'A', b'B', 4]
            .into_iter()
            .enumerate()
            .map(|(id, value)| SourceAction::Send {
                at: 0,
                id: id as u64,
                address: 0x1000,
                operation: Operation::Write { data: vec![value] },
            })
            .collect()
    }
}

#[test]
fn console_outputs_at_acceptance_once_per_retry_and_eot_does_not_exit() {
    let output = RefCell::new(Vec::new());
    let mut machine = Machine::new(vec![0]).unwrap();
    machine
        .add_target(
            0x1000,
            4096,
            &[1],
            console::SimpleUart {
                latency: NonZeroU64::new(5).unwrap(),
                output: &output,
            },
        )
        .unwrap();
    machine.add_source(0, 0, Writes).unwrap();
    let result = machine
        .run(RunLimits {
            events: 100,
            ticks: 100,
        })
        .unwrap();
    assert_eq!(result.stop, StopReason::Idle);
    assert_eq!(
        *output.borrow(),
        [
            console::Byte { at: 0, value: b'A' },
            console::Byte { at: 5, value: b'B' },
            console::Byte { at: 10, value: 4 }
        ]
    );
    assert_eq!(
        result
            .transactions
            .iter()
            .map(|tx| tx.completed_at)
            .collect::<Vec<_>>(),
        [Some(5), Some(10), Some(15)]
    );
    assert!(!result.transactions[1].retries.is_empty());
}

#[test]
fn checked_in_stubs_match_shared_abi() {
    assert_eq!(include_str!("../workloads/am/m5.S"), m5::guest_assembly());
}

#[test]
fn failure_is_distinct_even_with_zero_code_and_preserves_all_bits() {
    for (setup, expected) in [(0x00000593, 0), (0xfff00593, u64::MAX)] {
        let (cpu, result) = run(&[setup, m5::Operation::Fail.instruction()], true);
        assert_eq!(result.stop, StopReason::Exit(expected));
        assert_eq!(
            cpu.experiment().unwrap().exit(),
            Some(gem5rs::experiment::Exit {
                code: expected,
                failed: true
            })
        );
        assert_eq!(cpu.registers()[10], 0);
        assert_eq!(cpu.commits().len(), 2);
    }
}

#[test]
fn speculative_and_faulting_paths_never_reach_console() {
    for first in [0x00c0006f, 0x00100073] {
        let output = RefCell::new(Vec::new());
        let mut cpu = Cpu::new(0, 4096, 1).with_experiment(
            Experiment::new(Config {
                tick_femtoseconds: 1000,
                instruction_budget: 100,
            })
            .unwrap(),
        );
        let mut machine = Machine::new(vec![0]).unwrap();
        machine
            .add_route(Route {
                base: 0,
                target: Target::FixedRam {
                    size_bytes: 4096,
                    latency: 1,
                    capacity: 1,
                },
            })
            .unwrap();
        machine
            .add_target(
                4096,
                4096,
                &[1],
                console::SimpleUart {
                    latency: NonZeroU64::new(1).unwrap(),
                    output: &output,
                },
            )
            .unwrap();
        // Skip (or fault before) lui a1,1; sb zero,0(a1).
        let words = [
            first,
            0x000015b7,
            0x00058023,
            m5::Operation::Exit.instruction(),
        ];
        machine
            .load_ram(
                0,
                &words
                    .iter()
                    .flat_map(|w| w.to_le_bytes())
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        machine.add_source(0, 0, &mut cpu).unwrap();
        machine
            .run(RunLimits {
                events: 100,
                ticks: 100,
            })
            .unwrap();
        assert!(output.borrow().is_empty());
    }
}

#[test]
fn console_rejects_read_fetch_wide_and_nonregister_accesses() {
    let output = RefCell::new(Vec::new());
    let console = console::SimpleUart {
        latency: NonZeroU64::new(1).unwrap(),
        output: &output,
    };
    for (offset, operation) in [
        (0, Operation::Read { size: 1 }),
        (0, Operation::Fetch { size: 4 }),
        (0, Operation::Write { data: vec![0; 8] }),
        (1, Operation::Write { data: vec![0] }),
    ] {
        assert_eq!(
            console.select_lane(
                offset,
                &Request {
                    source: 0,
                    id: 0,
                    address: 4096 + offset as u64,
                    operation
                }
            ),
            Err(Fault::InvalidMmioAccess)
        );
    }
    assert!(output.borrow().is_empty());
}
