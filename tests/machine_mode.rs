use gem5rs::{
    cpu::Cpu,
    experiment::{Config, Experiment},
    m5,
    memory::*,
};

fn csr(address: u32, rd: u32, rs1: u32, op: u32) -> u32 {
    address << 20 | rs1 << 15 | op << 12 | rd << 7 | 0x73
}

fn run(words: &[u32], enabled: bool) -> (Cpu, RunResult) {
    let mut cpu = Cpu::new(0, 4096, 1)
        .with_machine_mode(enabled)
        .with_experiment(
            Experiment::new(Config {
                tick_femtoseconds: 1000,
                instruction_budget: 100,
            })
            .unwrap(),
        );
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
    bus.load_ram(
        0,
        &words
            .iter()
            .flat_map(|word| word.to_le_bytes())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    bus.add_source(0, 0, &mut cpu).unwrap();
    let result = bus
        .run(RunLimits {
            events: 2000,
            ticks: 1000,
        })
        .unwrap();
    (cpu, result)
}

#[test]
fn synchronous_traps_execute_guest_handler_and_return_without_retiring_fault() {
    for (word, cause, value) in [
        (0x0000_0073, 11, 0), // ecall
        (0x0010_0073, 3, 12), // ebreak
        (0xffff_ffff, 2, 0xffff_ffff),
        (0x0002_b383, 4, 99),   // ld t2,0(t0): misaligned
        (0x0001_3383, 5, 4096), // ld t2,0(sp): outside RAM
        (0x0001_3023, 7, 4096), // sd zero,0(sp): outside RAM
    ] {
        let mut words = vec![
            0x0400_0093,
            csr(0x305, 0, 1, 1),
            0x0630_0293,
            word,
            0x0070_0313,
            m5::Operation::Exit.instruction(),
        ];
        words.resize(16, 0);
        words.extend([
            csr(0x342, 20, 0, 2),
            csr(0x341, 21, 0, 2),
            csr(0x343, 22, 0, 2),
            0x004a_8b93,
            csr(0x341, 0, 23, 1),
            0x3020_0073,
        ]);
        let (cpu, result) = run(&words, true);
        assert_eq!(result.stop, StopReason::Exit(0));
        assert!(cpu.fault().is_none());
        assert_eq!(&cpu.registers()[20..23], &[cause, 12, value]);
        assert_eq!(cpu.registers()[5], 99);
        assert_eq!(cpu.registers()[6], 7);
        assert_eq!(cpu.committed(), 11);
        assert!(cpu.commits().iter().all(|commit| commit.pc != 12));
    }
}

#[test]
fn default_core_still_stops_and_unhandled_machine_traps_remain_bounded() {
    let (cpu, _) = run(&[0x0000_0073], false);
    assert!(cpu.fault().unwrap().reason.contains("EnvironmentCall"));
    assert_eq!(cpu.committed(), 0);
    let (cpu, result) = run(&[0xffff_ffff], true);
    assert!(matches!(
        result.stop,
        StopReason::TickLimit | StopReason::EventLimit
    ));
    assert_eq!(cpu.committed(), 0);
}
