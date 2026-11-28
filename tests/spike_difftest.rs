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
            }
        } else {
            Platform {
                console: vec![4096],
                exit: vec![],
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
            .contains("address/data disagree")
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
            "read request\nprintf 'hello 1 {SPIKE_COMMIT} rv64i pc_gpr_mem\\n'\nread request\nprintf 'bye\\n'\nexit 42"
        ));
        let mut reference = Reference::start(&script.config()).unwrap();
        assert!(reference.finish().unwrap_err().contains("abnormal exit"));
    }
}
