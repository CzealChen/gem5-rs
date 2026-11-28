//! Real reference tests are opt-in; process-failure tests need no Spike library.
use gem5rs::{
    difftest::{Config, Driver, Platform, Reference, SPIKE_COMMIT},
    elf::{Image, Segment},
    experiment::Exit,
    m5,
    observation::{MemoryEffect, Observer, Retirement},
    riscv::{self, Access, Response, Step},
};
const BASE: u64 = 0x8000_0000;
fn config() -> Config {
    Config {
        adapter: std::env::var("GEM5RS_SPIKE_ADAPTER").expect("run just check-spike"),
        timeout_ms: 5000,
        expected_exit_code: 0,
        expected_failure: false,
        allow_instruction_limit: false,
    }
}
fn image(words: &[u32]) -> Image {
    Image {
        entry: BASE,
        stack: BASE + 4000..BASE + 4096,
        segments: vec![Segment {
            address: BASE,
            data: words.iter().flat_map(|w| w.to_le_bytes()).collect(),
            zero_fill: 4096 - words.len() as u64 * 4,
        }],
    }
}
fn ordinary(pc: u64, word: u32, regs: &mut [u64; 32], ram: &mut [u8]) -> Retirement {
    let mut memory = None;
    let commit = match riscv::execute(pc, word, regs).unwrap() {
        Step::Commit(commit) => commit,
        Step::Memory(pending) => match pending.access() {
            Access::Read { address, size } => {
                let data =
                    ram[(*address - BASE) as usize..(*address - BASE) as usize + size].to_vec();
                memory = Some(MemoryEffect {
                    address: *address,
                    write: false,
                    data: data.clone(),
                });
                pending.complete(Response::ReadData(&data)).unwrap()
            }
            Access::Write { address, data } => {
                ram[(*address - BASE) as usize..(*address - BASE) as usize + data.len()]
                    .copy_from_slice(data);
                memory = Some(MemoryEffect {
                    address: *address,
                    write: true,
                    data: data.clone(),
                });
                pending.complete(Response::WriteAck).unwrap()
            }
        },
    };
    let mut next_pc = pc;
    commit.apply(&mut next_pc, regs);
    Retirement {
        pc,
        instruction: word,
        next_pc,
        registers: *regs,
        memory,
        service: None,
        machine: None,
    }
}

#[test]
#[ignore = "run devenv shell -- just check-spike"]
fn real_reference_handles_normal_special_normal_and_consecutive_syncs() {
    let words = [
        0x00000097,
        0x00700513,
        0x10a0b023,
        0x1000b603,
        m5::Operation::Rpns.instruction(),
        m5::Operation::Rpns.instruction(),
        0x00c506b3,
        m5::Operation::Exit.instruction(),
    ];
    let image = image(&words);
    for _ in 0..2 {
        let mut driver =
            Driver::start(config(), BASE..BASE + 4096, &image, Platform::default()).unwrap();
        let mut regs = image.initial_registers();
        let mut ram = vec![0; 4096];
        for (index, word) in words.into_iter().enumerate() {
            let pc = BASE + index as u64 * 4;
            let record = if let Some(service) = m5::Operation::decode(word) {
                // Arbitrary full state, not only a0, exercises the synchronization contract.
                for (index, value) in regs.iter_mut().enumerate().skip(1) {
                    *value = index as u64 * 11;
                }
                Retirement {
                    pc,
                    instruction: word,
                    next_pc: pc + 4,
                    registers: regs,
                    memory: None,
                    service: Some(service),
                    machine: None,
                }
            } else {
                ordinary(pc, word, &mut regs, &mut ram)
            };
            driver.retired(&record).unwrap();
        }
        let stats = driver
            .finish(Some(Exit {
                code: 0,
                failed: false,
            }))
            .unwrap();
        assert_eq!(
            (stats.compared, stats.synced_m5, stats.passed),
            (5, 3, true)
        );
    }
}

#[test]
#[ignore = "run devenv shell -- just check-spike"]
fn injected_alu_branch_store_address_and_data_errors_fail_before_later_sync() {
    let words = [
        0x00000097,
        0x00700513,
        0x10a0b023,
        0x00000463,
        m5::Operation::Rpns.instruction(),
    ];
    for mutation in 0..4 {
        let image = image(&words);
        let mut driver =
            Driver::start(config(), BASE..BASE + 4096, &image, Platform::default()).unwrap();
        let mut regs = image.initial_registers();
        let mut ram = vec![0; 4096];
        let bad_index = [1, 3, 2, 2][mutation];
        for (index, &word) in words.iter().enumerate().take(bad_index + 1) {
            let mut record = ordinary(BASE + index as u64 * 4, word, &mut regs, &mut ram);
            if index == bad_index {
                match mutation {
                    0 => record.registers[10] ^= 1,
                    1 => record.next_pc += 4,
                    2 => record.memory.as_mut().unwrap().address += 8,
                    3 => record.memory.as_mut().unwrap().data[0] ^= 1,
                    _ => unreachable!(),
                }
                let failure = driver.retired(&record).unwrap_err();
                assert!(failure.contains("first difference"));
                let later = Retirement {
                    pc: record.next_pc,
                    instruction: m5::Operation::Rpns.instruction(),
                    next_pc: record.next_pc + 4,
                    registers: regs,
                    memory: None,
                    service: Some(m5::Operation::Rpns),
                    machine: None,
                };
                assert_eq!(driver.retired(&later).unwrap_err(), failure);
                assert_eq!(
                    driver
                        .finish(Some(Exit {
                            code: 0,
                            failed: false
                        }))
                        .unwrap_err(),
                    failure
                );
            } else {
                driver.retired(&record).unwrap();
            }
        }
    }
}

fn state(command: &str, pc: u64) -> String {
    format!("{command} {pc}{}", " 0".repeat(32))
}

#[test]
#[ignore = "run devenv shell -- just check-spike"]
fn architectural_faults_are_compared_and_never_become_successful_exits() {
    use riscv::{AccessKind, Fault, FaultReason};
    for (word, reason, address) in [
        (0x00100073, FaultReason::Breakpoint, None),
        (0x00000073, FaultReason::EnvironmentCall, None),
        (
            0xffffffff,
            FaultReason::IllegalInstruction(0xffffffff),
            None,
        ),
        (
            0x00003083,
            FaultReason::AccessFault(AccessKind::Read),
            Some(0),
        ),
        (
            0x00003023,
            FaultReason::AccessFault(AccessKind::Write),
            Some(0),
        ),
    ] {
        let image = image(&[word]);
        let mut driver =
            Driver::start(config(), BASE..BASE + 4096, &image, Platform::default()).unwrap();
        driver
            .faulted(&Fault {
                pc: BASE,
                reason,
                address,
            })
            .unwrap();
        assert!(
            driver
                .finish(Some(Exit {
                    code: 0,
                    failed: false
                }))
                .unwrap_err()
                .contains("fault matched reference")
        );
    }
    let image = image(&[0x00100513]); // Reference executes normally; DUT falsely faults.
    let mut driver =
        Driver::start(config(), BASE..BASE + 4096, &image, Platform::default()).unwrap();
    assert!(
        driver
            .faulted(&Fault {
                pc: BASE,
                reason: FaultReason::Breakpoint,
                address: None
            })
            .unwrap_err()
            .contains("first fault difference")
    );
}

#[test]
#[ignore = "run devenv shell -- just check-spike"]
fn console_and_legacy_exit_are_decoded_before_sync() {
    for legacy in [false, true] {
        let store = if legacy { 0x00a1b023 } else { 0x00a18023 };
        let words = [
            0x000011b7,
            0x04100513,
            store,
            0x00000513,
            m5::Operation::Exit.instruction(),
        ];
        let image = image(&words);
        let mut config = config();
        if legacy {
            config.expected_exit_code = 65;
        }
        let platform = if legacy {
            Platform {
                console: vec![],
                exit: vec![(4096, 8)],
                ..Default::default()
            }
        } else {
            Platform {
                console: vec![4096],
                exit: vec![],
                ..Default::default()
            }
        };
        let mut driver = Driver::start(config, BASE..BASE + 4096, &image, platform).unwrap();
        let mut regs = image.initial_registers();
        let mut ram = vec![0; 4096];
        for (index, &word) in words.iter().take(2).enumerate() {
            driver
                .retired(&ordinary(
                    BASE + 4 * index as u64,
                    word,
                    &mut regs,
                    &mut ram,
                ))
                .unwrap();
        }
        let mut record = Retirement {
            pc: BASE + 8,
            instruction: store,
            next_pc: BASE + 12,
            registers: regs,
            memory: Some(MemoryEffect {
                address: 4096,
                write: true,
                data: if legacy {
                    65u64.to_le_bytes().to_vec()
                } else {
                    vec![65]
                },
            }),
            service: None,
            machine: None,
        };
        driver.retired(&record).unwrap();
        if !legacy {
            driver
                .retired(&ordinary(BASE + 12, words[3], &mut regs, &mut ram))
                .unwrap();
            record = Retirement {
                pc: BASE + 16,
                instruction: words[4],
                next_pc: BASE + 20,
                registers: regs,
                memory: None,
                service: Some(m5::Operation::Exit),
                machine: None,
            };
            driver.retired(&record).unwrap();
        }
        let stats = driver
            .finish(Some(Exit {
                code: if legacy { 65 } else { 0 },
                failed: false,
            }))
            .unwrap();
        assert_eq!(
            (stats.synced_console, stats.synced_exit),
            if legacy { (0, 1) } else { (1, 0) }
        );
    }
    // An ordinary RAM store with a corrupted DUT address must not become a skip.
    let words = [0x00000097, 0x10a08023];
    let image = image(&words);
    let mut driver = Driver::start(
        config(),
        BASE..BASE + 4096,
        &image,
        Platform {
            console: vec![4096],
            exit: vec![],
            ..Default::default()
        },
    )
    .unwrap();
    let mut regs = image.initial_registers();
    let mut ram = vec![0; 4096];
    driver
        .retired(&ordinary(BASE, words[0], &mut regs, &mut ram))
        .unwrap();
    let mut store = ordinary(BASE + 4, words[1], &mut regs, &mut ram);
    store.memory.as_mut().unwrap().address = 4096;
    assert!(
        driver
            .retired(&store)
            .unwrap_err()
            .contains("address/width/data disagree")
    );
}

#[test]
#[ignore = "run devenv shell -- just check-spike"]
fn protocol_rejects_bad_isa_sync_and_reports_repeated_traps() {
    let mut reference = Reference::start(&config()).unwrap();
    assert!(
        reference
            .exchange(state("init rv64im 2147483648 4096", BASE))
            .unwrap_err()
            .contains("unsupported ISA")
    );
    let mut reference = Reference::start(&config()).unwrap();
    assert_eq!(
        reference
            .exchange(state("init rv64i 2147483648 4096", BASE))
            .unwrap(),
        "ok"
    );
    assert_eq!(
        reference.exchange(format!("load {BASE} 73001000")).unwrap(),
        "ok"
    );
    for _ in 0..2 {
        assert_eq!(reference.exchange(state("sync", BASE)).unwrap(), "ok");
        let fault = reference.exchange("step".into()).unwrap();
        assert!(fault.starts_with(&format!("fault {BASE} 1048691 3 {BASE} ")));
        assert_eq!(fault.split_whitespace().count(), 6);
    }
    assert!(
        reference
            .exchange(state("sync", BASE + 2))
            .unwrap_err()
            .contains("invalid PC/x0")
    );
    let mut reference = Reference::start(&config()).unwrap();
    let bad = format!("init rv64i {BASE} 4096 {BASE} 1{}", " 0".repeat(31));
    assert!(
        reference
            .exchange(bad)
            .unwrap_err()
            .contains("invalid PC/x0")
    );
}

#[cfg(unix)]
mod process_failures {
    use super::*;
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
        time::{Duration, Instant},
    };
    struct Script(PathBuf);
    impl Script {
        fn new(body: &str) -> Self {
            static SEQUENCE: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "gem5rs-reference-{}-{}",
                std::process::id(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            Self(path)
        }
        fn config(&self) -> Config {
            Config {
                adapter: self.0.to_str().unwrap().into(),
                timeout_ms: 2000,
                expected_exit_code: 0,
                expected_failure: false,
                allow_instruction_limit: false,
            }
        }
    }
    impl Drop for Script {
        fn drop(&mut self) {
            fs::remove_file(&self.0).unwrap();
        }
    }
    #[test]
    fn rejects_wrong_handshake_eof_oversize_and_timeout() {
        for (script, expected) in [
            (
                "printf 'hello 2 wrong rv64i pc_gpr_mem\\n'",
                "handshake mismatch",
            ),
            ("exit 42", "EOF"),
            ("printf '%20000s\\n' x", "oversized"),
            ("exec sleep 60", "timeout"),
        ] {
            let script = Script::new(script);
            let start = Instant::now();
            let error = Reference::start(&script.config())
                .err()
                .expect("reference must fail");
            assert!(error.contains(expected), "{error}");
            assert!(start.elapsed() < Duration::from_secs(3));
        }
    }
    #[test]
    fn abnormal_exit_after_bye_is_not_success() {
        let script = Script::new(&format!(
            "read request\nprintf 'hello 2 {SPIKE_COMMIT} rv64i_zicsr pc_gpr_mem_machine\\n'\nread request\nprintf 'bye\\n'\nexit 42"
        ));
        let mut reference = Reference::start(&script.config()).unwrap();
        assert!(reference.finish().unwrap_err().contains("abnormal exit"));
    }
}

fn csr(address: u32, rd: u32, rs1: u32, op: u32) -> u32 {
    address << 20 | rs1 << 15 | op << 12 | rd << 7 | 0x73
}

struct MachineObserver {
    driver: std::rc::Rc<std::cell::RefCell<Driver>>,
    corrupt: Option<&'static str>,
}
impl Observer for MachineObserver {
    fn retired(&mut self, retirement: &Retirement) -> Result<(), String> {
        let mut record = retirement.clone();
        if self.corrupt == Some("csr")
            || (self.corrupt == Some("csr_after_m5") && record.service.is_some())
            || (self.corrupt == Some("csr_after_mmio") && record.memory.is_some())
        {
            record.machine.as_mut().unwrap().mscratch ^= 1;
        }
        if let Some(memory) = &mut record.memory
            && !memory.write
        {
            match self.corrupt {
                Some("mmio_address") => memory.address += 8,
                Some("mmio_width") => {
                    memory.data.truncate(4);
                }
                Some("mmio_register") => record.registers[6] ^= 1,
                _ => {}
            }
        }
        self.driver.borrow_mut().retired(&record)
    }
    fn timer_pending(&mut self, pending: bool) -> Result<(), String> {
        self.driver
            .borrow_mut()
            .timer_pending(pending && self.corrupt != Some("irq"))
    }
    fn trapped(&mut self, trap: &gem5rs::observation::Trap) -> Result<(), String> {
        let mut record = trap.clone();
        if self.corrupt == Some("mepc") {
            record.machine.mepc += 4;
        }
        if self.corrupt == Some("mstatus") {
            record.machine.mstatus ^= 128;
        }
        self.driver.borrow_mut().trapped(&record)
    }
}

fn machine_run(
    words: &[u32],
    timer: bool,
    corrupt: Option<&'static str>,
) -> Result<gem5rs::difftest::Statistics, String> {
    use gem5rs::{cpu::Cpu, experiment, memory, timer as clock};
    use std::{cell::RefCell, num::NonZeroU64, rc::Rc};
    let image = image(words);
    let driver = Rc::new(RefCell::new(Driver::start(
        config(),
        BASE..BASE + 4096,
        &image,
        Platform {
            machine_mode: true,
            timers: if timer { vec![4096] } else { vec![] },
            ..Default::default()
        },
    )?));
    let mut cpu = Cpu::new(BASE, BASE + 4096, 1)
        .with_machine_mode(true)
        .with_experiment(
            experiment::Experiment::new(experiment::Config {
                tick_femtoseconds: 1000,
                instruction_budget: 1000,
            })
            .unwrap(),
        )
        .with_observer(MachineObserver {
            driver: driver.clone(),
            corrupt,
        });
    let mut bus = memory::Machine::new(vec![0]).unwrap();
    bus.add_route(memory::Route {
        base: BASE,
        target: memory::Target::FixedRam {
            size_bytes: 4096,
            latency: 1,
            capacity: 1,
        },
    })
    .unwrap();
    if timer {
        let line = Rc::new(RefCell::new(clock::Timer::new(NonZeroU64::new(1).unwrap())));
        cpu = cpu.with_timer(line.clone());
        bus.add_target(
            4096,
            16,
            &[1],
            clock::Mmio {
                timer: line,
                latency: NonZeroU64::new(3).unwrap(),
            },
        )
        .unwrap();
    }
    bus.load_ram(BASE, &image.segments[0].data).unwrap();
    bus.add_source(0, 0, &mut cpu).unwrap();
    let run = bus
        .run(memory::RunLimits {
            events: 20000,
            ticks: 10000,
        })
        .unwrap();
    if let Some(error) = cpu.observation_error() {
        // The first mismatch remains fatal even if someone attempts to finish.
        assert!(
            driver
                .borrow_mut()
                .finish(Some(Exit {
                    code: 0,
                    failed: false
                }))
                .is_err()
        );
        return Err(error.to_owned());
    }
    assert_eq!(run.stop, memory::StopReason::Exit(0), "{:?}", cpu.fault());
    driver
        .borrow_mut()
        .finish(cpu.experiment().and_then(experiment::Experiment::exit))
}

#[test]
#[ignore = "run devenv shell -- just check-spike"]
fn machine_csrs_synchronous_traps_and_mret_are_independently_compared() {
    let mut words = vec![
        0x00000097,
        0x08008093,
        csr(0x305, 0, 1, 1),
        csr(0x340, 0, 7, 5),
        csr(0x340, 8, 0, 2),
        0x00000073, // ecall
        0x00700313,
        m5::Operation::Exit.instruction(),
    ];
    words.resize(32, 0);
    words.extend([
        csr(0x342, 20, 0, 2),
        csr(0x341, 21, 0, 2),
        0x004a8a93,
        csr(0x341, 0, 21, 1),
        0x30200073,
    ]);
    let stats = machine_run(&words, false, None).unwrap();
    assert_eq!(stats.traps, 1);
    assert!(stats.csr_compared > 10);
    for corrupt in ["csr", "mepc", "mstatus", "csr_after_m5"] {
        assert!(
            machine_run(&words, false, Some(corrupt))
                .unwrap_err()
                .contains("difference")
        );
    }
}

#[test]
#[ignore = "run devenv shell -- just check-spike"]
fn timer_mmio_is_synchronized_but_interrupt_entry_and_return_are_compared() {
    let mut words = vec![
        0x00000097,
        0x08008093,
        csr(0x305, 0, 1, 1),
        0x000011b7,
        0x06400213,
        0x0041b423, // timer deadline 100
        0x08000293,
        csr(0x304, 0, 5, 2),
        csr(0x300, 0, 8, 6),
        0x0001b303, // ld x6,mtime
        0x06400393, // 100 iterations guarantee deadline crosses
        0xfff38393,
        0xfe039ee3,
        m5::Operation::Exit.instruction(),
    ];
    words.resize(32, 0);
    words.extend([
        csr(0x342, 20, 0, 2),
        csr(0x341, 21, 0, 2),
        0xfff00213,
        0x0041b423,
        0x30200073,
    ]);
    let stats = machine_run(&words, true, None).unwrap();
    assert_eq!(stats.interrupts, 1);
    assert_eq!(stats.synced_mmio, 3);
    for corrupt in [
        "mmio_address",
        "mmio_width",
        "mmio_register",
        "csr_after_mmio",
    ] {
        assert!(
            machine_run(&words, true, Some(corrupt))
                .unwrap_err()
                .contains("difference")
        );
    }
    assert!(
        machine_run(&words, true, Some("irq"))
            .unwrap_err()
            .contains("trap")
    );
}

#[test]
#[ignore = "run devenv shell -- just check-spike"]
fn skipping_exit_cannot_hide_an_eligible_timer_interrupt() {
    let mut reference = Reference::start(&config()).unwrap();
    let init = format!("init rv64i_zicsr {BASE} 4096 {BASE}{}", " 0".repeat(32));
    assert_eq!(reference.exchange(init).unwrap(), "ok");
    let words = [
        0x08000093,
        csr(0x304, 0, 1, 2),
        csr(0x300, 0, 8, 6),
        m5::Operation::Exit.instruction(),
    ];
    let bytes: String = words
        .iter()
        .flat_map(|w| w.to_le_bytes())
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(
        reference.exchange(format!("load {BASE} {bytes}")).unwrap(),
        "ok"
    );
    for _ in 0..3 {
        assert!(
            reference
                .exchange("step".into())
                .unwrap()
                .starts_with("state ")
        );
    }
    assert_eq!(reference.exchange("pending 1".into()).unwrap(), "ok");
    assert!(
        reference
            .exchange("inspect".into())
            .unwrap_err()
            .contains("pending timer interrupt")
    );
}
