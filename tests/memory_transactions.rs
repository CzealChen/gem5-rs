use gem5rs::memory::{
    Fault, Machine, MemoryTarget, Operation, Request, ResponseResult, Route, RunLimits, RunResult,
    Source, SourceAction, SourceEvent, SourceId, StopReason, Target, TargetCompletion, Transaction,
};
use std::num::NonZeroU64;

const LIMITS: RunLimits = RunLimits {
    events: 100,
    ticks: 100,
};

#[derive(Default)]
struct Probe {
    actions: Vec<SourceAction>,
    received: Vec<(u64, SourceId, u64, ResponseResult)>,
}

impl Source for Probe {
    fn on_event(&mut self, at: u64, event: SourceEvent) -> Vec<SourceAction> {
        match event {
            SourceEvent::Wake => std::mem::take(&mut self.actions),
            SourceEvent::Response(response) => {
                self.received
                    .push((at, response.source, response.id, response.result));
                Vec::new()
            }
        }
    }
}

fn fixed(size_bytes: usize, latency: u64, capacity: usize) -> Target {
    Target::FixedRam {
        size_bytes,
        latency,
        capacity,
    }
}

fn send(at: u64, id: u64, address: u64, operation: Operation) -> SourceAction {
    SourceAction::Send {
        at,
        id,
        address,
        operation,
    }
}

fn read(at: u64, id: u64, address: u64, size: usize) -> SourceAction {
    send(at, id, address, Operation::Read { size })
}

fn write(at: u64, id: u64, address: u64, data: &[u8]) -> SourceAction {
    send(
        at,
        id,
        address,
        Operation::Write {
            data: data.to_vec(),
        },
    )
}

fn record(run: &RunResult, source: SourceId, id: u64) -> &Transaction {
    run.transactions
        .iter()
        .find(|tx| tx.request.source == source && tx.request.id == id)
        .unwrap()
}

#[test]
fn external_ram_replacement_preserves_data_and_uses_its_own_service_times() {
    struct ExternalRam([u8; 2]);

    impl MemoryTarget for ExternalRam {
        fn select_lane(&self, _offset: usize, _request: &Request) -> Result<usize, Fault> {
            Ok(0)
        }

        fn service_latency(&self, _at: u64, _offset: usize, request: &Request) -> NonZeroU64 {
            NonZeroU64::new(match request.operation {
                Operation::Write { .. } => 3,
                _ => 5,
            })
            .unwrap()
        }

        fn complete(&mut self, _at: u64, offset: usize, request: &Request) -> TargetCompletion {
            TargetCompletion::Respond(match &request.operation {
                Operation::Write { data } => {
                    self.0[offset..offset + data.len()].copy_from_slice(data);
                    ResponseResult::WriteAck
                }
                Operation::Read { size } | Operation::Fetch { size } => {
                    ResponseResult::ReadData(self.0[offset..offset + size].to_vec())
                }
            })
        }

        fn initialize(&mut self, offset: usize, data: &[u8]) -> Result<(), String> {
            self.0[offset..offset + data.len()].copy_from_slice(data);
            Ok(())
        }
    }

    for (external, write_at, read_at) in [(false, 10, 20), (true, 3, 8)] {
        let mut writer = Probe {
            actions: vec![write(0, 0, 32, b"Z")],
            ..Probe::default()
        };
        let mut reader = Probe {
            actions: vec![read(0, 0, 32, 2)],
            ..Probe::default()
        };
        let mut machine = Machine::new(vec![0, 1]).unwrap();
        if external {
            machine
                .add_target(32, 2, &[1], ExternalRam([0; 2]))
                .unwrap();
        } else {
            machine
                .add_route(Route {
                    base: 32,
                    target: fixed(2, 10, 1),
                })
                .unwrap();
        }
        machine.load_ram(33, b"!").unwrap();
        machine.add_source(0, 0, &mut writer).unwrap();
        machine.add_source(1, 0, &mut reader).unwrap();
        let run = machine.run(LIMITS).unwrap();

        let read = record(&run, 1, 0);
        assert_eq!(read.attempts, [0, write_at]);
        assert_eq!(
            (read.accepted_at, read.completed_at),
            (Some(write_at), Some(read_at))
        );
        assert_eq!(
            writer.received,
            [(write_at, 0, 0, ResponseResult::WriteAck)]
        );
        assert_eq!(
            reader.received,
            [(read_at, 1, 0, ResponseResult::ReadData(b"Z!".to_vec()))]
        );
    }
}

#[test]
fn two_sources_contend_at_zero_in_explicit_port_order() {
    for registration in [[40, 3], [3, 40]] {
        let mut writer = Probe {
            actions: vec![write(0, 0, 0, b"A")],
            ..Probe::default()
        };
        let mut reader = Probe {
            actions: vec![read(0, 0, 0, 1)],
            ..Probe::default()
        };
        let mut machine = Machine::new(vec![40, 3]).unwrap();
        machine
            .add_route(Route {
                base: 0,
                target: fixed(16, 10, 1),
            })
            .unwrap();
        let mut sources = [(40, &mut writer), (3, &mut reader)];
        if registration == [3, 40] {
            sources.swap(0, 1);
        }
        for (id, source) in sources {
            machine.add_source(id, 0, source).unwrap();
        }
        let run = machine.run(LIMITS).unwrap();
        assert_eq!(
            (run.stop.clone(), run.tick, run.transactions.len()),
            (StopReason::Idle, 20, 2)
        );
        let a = record(&run, 40, 0);
        assert_eq!((a.accepted_at, a.completed_at), (Some(0), Some(10)));
        assert_eq!(a.attempts, [0]);
        let b = record(&run, 3, 0);
        assert_eq!((b.accepted_at, b.completed_at), (Some(10), Some(20)));
        assert_eq!(b.attempts, [0, 10]);
        assert_eq!(b.retries, [10]);
        assert_eq!(writer.received, [(10, 40, 0, ResponseResult::WriteAck)]);
        assert_eq!(
            reader.received,
            [(20, 3, 0, ResponseResult::ReadData(b"A".to_vec()))]
        );
    }
}

#[test]
fn retry_can_lose_again_and_round_robin_rotates_between_ports() {
    // The first two grants are A's write and B's read. A's pending write
    // changes the byte only at tick 30, so B still reads A at tick 20.
    let mut a = Probe {
        actions: vec![write(0, 0, 0, b"A"), write(0, 1, 0, b"B")],
        ..Probe::default()
    };
    let mut b = Probe {
        actions: vec![read(0, 0, 0, 1), read(0, 1, 0, 1)],
        ..Probe::default()
    };
    let mut machine = Machine::new(vec![0, 1]).unwrap();
    machine
        .add_route(Route {
            base: 0,
            target: fixed(16, 10, 1),
        })
        .unwrap();
    machine.add_source(0, 0, &mut a).unwrap();
    machine.add_source(1, 0, &mut b).unwrap();
    let run = machine.run(LIMITS).unwrap();
    let pending_write = record(&run, 0, 1);
    assert_eq!(pending_write.attempts, [0, 10, 20]);
    assert_eq!(pending_write.retries, [10, 20]);
    assert_eq!(pending_write.accepted_at, Some(20));
    assert_eq!(record(&run, 1, 1).attempts, [0, 10, 20, 30]);
    assert_eq!(
        a.received,
        [
            (10, 0, 0, ResponseResult::WriteAck),
            (30, 0, 1, ResponseResult::WriteAck)
        ]
    );
    assert_eq!(
        b.received,
        [
            (20, 1, 0, ResponseResult::ReadData(b"A".to_vec())),
            (40, 1, 1, ResponseResult::ReadData(b"B".to_vec())),
        ]
    );
}

#[test]
fn reads_sample_at_completion_even_when_admitted_before_the_write_completes() {
    let mut source = Probe {
        actions: vec![read(0, 0, 0, 1), write(3, 1, 0, b"X"), read(4, 2, 0, 1)],
        ..Probe::default()
    };
    let mut machine = Machine::new(vec![0]).unwrap();
    machine
        .add_route(Route {
            base: 0,
            target: fixed(16, 10, 3),
        })
        .unwrap();
    machine.load_ram(0, b"A").unwrap();
    machine.add_source(0, 0, &mut source).unwrap();
    let run = machine.run(LIMITS).unwrap();
    assert_eq!(
        (
            record(&run, 0, 2).accepted_at,
            record(&run, 0, 2).completed_at
        ),
        (Some(4), Some(14))
    );
    assert_eq!(
        source.received,
        [
            (10, 0, 0, ResponseResult::ReadData(b"A".to_vec())),
            (13, 0, 1, ResponseResult::WriteAck),
            (14, 0, 2, ResponseResult::ReadData(b"X".to_vec())),
        ]
    );
}

#[test]
fn two_banks_keep_data_and_overlap_different_banks_only() {
    let targets = [
        fixed(32, 10, 1),
        Target::BankedRam {
            size_bytes: 32,
            latency: 10,
            bank_bytes: 8,
        },
    ];
    for (target, second_at, third_at) in [(targets[0], 20, 30), (targets[1], 10, 20)] {
        let mut a = Probe {
            actions: vec![read(0, 0, 32, 1), read(0, 1, 48, 1)],
            ..Probe::default()
        };
        let mut b = Probe {
            actions: vec![read(0, 0, 40, 1)],
            ..Probe::default()
        };
        let mut machine = Machine::new(vec![0, 1]).unwrap();
        machine.add_route(Route { base: 32, target }).unwrap();
        machine.load_ram(32, b"A").unwrap();
        machine.load_ram(40, b"B").unwrap();
        machine.load_ram(48, b"C").unwrap();
        machine.add_source(0, 0, &mut a).unwrap();
        machine.add_source(1, 0, &mut b).unwrap();
        let run = machine.run(LIMITS).unwrap();
        assert_eq!(run.tick, third_at);
        assert_eq!(
            a.received,
            [
                (10, 0, 0, ResponseResult::ReadData(b"A".to_vec())),
                (third_at, 0, 1, ResponseResult::ReadData(b"C".to_vec())),
            ]
        );
        assert_eq!(
            b.received,
            [(second_at, 1, 0, ResponseResult::ReadData(b"B".to_vec()))]
        );
    }
}

#[test]
fn invalid_accesses_return_faults_and_keep_both_routes_intact() {
    let mut source = Probe {
        actions: vec![
            write(0, 0, 2, b"XXXX"),
            write(0, 1, u64::MAX, b"XX"),
            write(0, 2, 6, b"X"),
            write(0, 3, 1, b"XX"),
            read(0, 4, 0, 0),
            send(2, 5, 0, Operation::Fetch { size: 4 }),
            read(2, 6, 4, 2),
        ],
        ..Probe::default()
    };
    let mut machine = Machine::new(vec![0]).unwrap();
    machine
        .add_route(Route {
            base: 4,
            target: fixed(2, 3, 1),
        })
        .unwrap();
    machine
        .add_route(Route {
            base: 0,
            target: fixed(4, 3, 1),
        })
        .unwrap();
    assert_eq!(
        machine.load_ram(2, b"XXXX"),
        Err("RAM initialization at 0x2: CrossesRoute".into())
    );
    machine.load_ram(0, b"ABCD").unwrap();
    machine.load_ram(4, b"EF").unwrap();
    machine.add_source(0, 0, &mut source).unwrap();
    machine.run(LIMITS).unwrap();
    assert_eq!(
        source.received,
        [
            (1, 0, 0, ResponseResult::Fault(Fault::CrossesRoute)),
            (1, 0, 1, ResponseResult::Fault(Fault::AddressOverflow)),
            (1, 0, 2, ResponseResult::Fault(Fault::Unmapped)),
            (1, 0, 3, ResponseResult::Fault(Fault::Misaligned)),
            (1, 0, 4, ResponseResult::Fault(Fault::ZeroSize)),
            (5, 0, 5, ResponseResult::ReadData(b"ABCD".to_vec())),
            (5, 0, 6, ResponseResult::ReadData(b"EF".to_vec())),
        ]
    );
}

#[test]
fn response_drives_exit_write_and_ack_drives_commit() {
    #[derive(Default)]
    struct Core {
        committed_at: Option<u64>,
    }

    impl Source for Core {
        fn on_event(&mut self, at: u64, event: SourceEvent) -> Vec<SourceAction> {
            match event {
                SourceEvent::Wake => vec![read(at, 0, 0, 1)],
                SourceEvent::Response(response) => match (response.id, response.result) {
                    (0, ResponseResult::ReadData(code)) => vec![write(at, 0, 16, &[code[0], 0x12])],
                    (0, ResponseResult::WriteAck) => {
                        self.committed_at = Some(at);
                        vec![SourceAction::Stop("callback completed".into())]
                    }
                    other => panic!("unexpected response: {other:?}"),
                },
            }
        }
    }

    let mut core = Core::default();
    let mut machine = Machine::new(vec![0]).unwrap();
    machine
        .add_route(Route {
            base: 0,
            target: fixed(8, 3, 1),
        })
        .unwrap();
    machine
        .add_route(Route {
            base: 16,
            target: Target::ExitMmio {
                width: 2,
                latency: 7,
            },
        })
        .unwrap();
    machine.load_ram(0, &[0x34]).unwrap();
    machine.add_source(0, 0, &mut core).unwrap();
    let run = machine.run(LIMITS).unwrap();
    assert_eq!(
        (core.committed_at, run.stop, run.tick),
        (Some(10), StopReason::Exit(0x1234), 10)
    );
    assert_eq!(
        run.transactions
            .iter()
            .map(|tx| (tx.accepted_at, tx.completed_at))
            .collect::<Vec<_>>(),
        [(Some(0), Some(3)), (Some(3), Some(10)),]
    );
}

#[test]
fn exit_latches_once_and_accepted_transactions_still_get_responses() {
    let mut source = Probe {
        actions: vec![
            read(0, 0, 16, 2),
            write(0, 1, 16, &[7, 0]),
            write(0, 2, 16, &[8, 0]),
            write(0, 3, 32, &[9, 0]),
        ],
        ..Probe::default()
    };
    let mut machine = Machine::new(vec![0]).unwrap();
    for (base, latency) in [(16, 3), (32, 4)] {
        machine
            .add_route(Route {
                base,
                target: Target::ExitMmio { width: 2, latency },
            })
            .unwrap();
    }
    machine.add_source(0, 0, &mut source).unwrap();
    let run = machine.run(LIMITS).unwrap();
    assert_eq!((run.stop.clone(), run.tick), (StopReason::Exit(7), 4));
    assert_eq!(record(&run, 0, 2).attempts, [0]);
    assert_eq!(record(&run, 0, 2).accepted_at, None);
    assert_eq!(
        source.received,
        [
            (1, 0, 0, ResponseResult::Fault(Fault::InvalidMmioAccess)),
            (3, 0, 1, ResponseResult::WriteAck),
            (4, 0, 3, ResponseResult::Fault(Fault::ExitAlreadyRequested)),
        ]
    );
}

#[test]
fn tick_budget_preserves_partial_transaction_timing() {
    let mut source = Probe {
        actions: vec![write(0, 0, 0, b"X")],
        ..Probe::default()
    };
    let mut machine = Machine::new(vec![0]).unwrap();
    machine
        .add_route(Route {
            base: 0,
            target: fixed(16, 10, 1),
        })
        .unwrap();
    machine.add_source(0, 0, &mut source).unwrap();
    let run = machine.run(RunLimits { ticks: 9, ..LIMITS }).unwrap();
    assert_eq!(
        (run.stop.clone(), run.tick, run.events),
        (StopReason::TickLimit, 9, 3)
    );
    assert_eq!(
        (
            record(&run, 0, 0).accepted_at,
            record(&run, 0, 0).completed_at
        ),
        (Some(0), None)
    );
    assert!(source.received.is_empty());
}
