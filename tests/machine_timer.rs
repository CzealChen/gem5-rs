use gem5rs::{
    cpu::Cpu,
    experiment::{Config, Experiment},
    m5,
    memory::*,
    timer,
};
use std::{cell::RefCell, num::NonZeroU64, rc::Rc};

fn csr(address: u32, rd: u32, rs1: u32, op: u32) -> u32 {
    address << 20 | rs1 << 15 | op << 12 | rd << 7 | 0x73
}

#[test]
fn pending_during_memory_transaction_waits_for_retirement_and_returns_to_next_pc() {
    let line = Rc::new(RefCell::new(timer::Timer::new(NonZeroU64::new(1).unwrap())));
    let mut cpu = Cpu::new(0, 4096, 1)
        .with_machine_mode(true)
        .with_timer(line.clone())
        .with_experiment(
            Experiment::new(Config {
                tick_femtoseconds: 1000,
                instruction_budget: 100,
            })
            .unwrap(),
        );
    let mut words = vec![
        0x0800_0093,
        csr(0x305, 0, 1, 1), // handler at 128
        0x0000_11b7,
        0x0640_0213, // timer base 4096, deadline 100
        0x0041_b423, // sd x4,8(x3)
        0x0800_0293,
        csr(0x304, 0, 5, 2), // MTIE
        0x0000_23b7,         // x7=8192 slow RAM
        csr(0x300, 0, 8, 6), // MIE
        0x0003_b403,         // ld x8,0(x7): crosses timer deadline
        0x0070_0313,         // addi x6,7 (interrupted before execution)
        m5::Operation::Exit.instruction(),
    ];
    words.resize(32, 0);
    words.extend([
        csr(0x342, 20, 0, 2),
        csr(0x341, 21, 0, 2),
        0xfff0_0213,
        0x0041_b423, // mtimecmp=MAX acknowledges interrupt
        0x3020_0073,
    ]);
    let mut bus = Machine::new(vec![0]).unwrap();
    bus.add_route(Route {
        base: 0,
        target: Target::FixedRam {
            size_bytes: 4096,
            latency: 1,
            capacity: 1,
        },
    })
    .unwrap();
    bus.add_route(Route {
        base: 8192,
        target: Target::FixedRam {
            size_bytes: 8,
            latency: 100,
            capacity: 1,
        },
    })
    .unwrap();
    bus.add_target(
        4096,
        16,
        &[1],
        timer::Mmio {
            timer: line,
            latency: NonZeroU64::new(3).unwrap(),
        },
    )
    .unwrap();
    bus.load_ram(
        0,
        &words
            .iter()
            .flat_map(|w| w.to_le_bytes())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    bus.load_ram(8192, &42_u64.to_le_bytes()).unwrap();
    bus.add_source(0, 0, &mut cpu).unwrap();
    let result = bus
        .run(RunLimits {
            events: 2000,
            ticks: 1000,
        })
        .unwrap();
    assert_eq!(result.stop, StopReason::Exit(0));
    assert_eq!(cpu.timer_interrupts(), 1);
    assert_eq!(cpu.registers()[8], 42);
    assert_eq!(cpu.registers()[6], 7);
    assert_eq!(cpu.registers()[20], (1 << 63) | 7);
    assert_eq!(cpu.registers()[21], 40);
    let pcs: Vec<_> = cpu.commits().iter().map(|c| c.pc).collect();
    let load = pcs.iter().position(|pc| *pc == 36).unwrap();
    assert_eq!(pcs[load + 1], 128);
    assert_eq!(pcs.iter().filter(|pc| **pc == 40).count(), 1);
    assert_eq!(cpu.committed(), 17); // interrupt itself does not retire
}
