use super::graph::{Plan, Scheduled, Source};
use crate::{cpu::Cpu, memory};
use serde::Serialize;
use serde_json::json;
use std::{
    cell::RefCell,
    collections::{BTreeMap, VecDeque},
};

type DeliveryLog = RefCell<Vec<(usize, u64)>>;

struct Observe<'a, S> {
    source: S,
    deliveries: &'a DeliveryLog,
}

impl<S: memory::Source> memory::Source for Observe<'_, S> {
    fn on_event(&mut self, at: u64, event: memory::SourceEvent) -> Vec<memory::SourceAction> {
        if let memory::SourceEvent::Response(response) = &event {
            self.deliveries
                .borrow_mut()
                .push((response.source, response.id));
        }
        self.source.on_event(at, event)
    }
}

struct Traffic(Vec<Scheduled>);

impl memory::Source for Traffic {
    fn on_event(&mut self, _: u64, event: memory::SourceEvent) -> Vec<memory::SourceAction> {
        match event {
            memory::SourceEvent::Wake => self
                .0
                .drain(..)
                .map(|op| memory::SourceAction::Send {
                    at: op.at,
                    id: op.id,
                    address: op.address,
                    operation: op.operation,
                })
                .collect(),
            memory::SourceEvent::Response(_) => Vec::new(),
        }
    }
}

#[derive(Default, Serialize)]
struct Stats {
    events: u64,
    accepted: u64,
    rejected: u64,
    retries: u64,
    fetches: u64,
    reads: u64,
    writes: u64,
    committed: usize,
}

#[derive(Serialize)]
struct Transaction<'a> {
    id: u64,
    source: &'a str,
    addr: u64,
    kind: &'static str,
    size_bytes: usize,
    accepted_at: u64,
    completed_at: u64,
    data: Option<&'a [u8]>,
    fault: Option<String>,
}

pub(super) fn run(plan: Plan<'_>, events: u64) -> Result<String, String> {
    let mut cpu = plan.sources.iter().find_map(|req| match &req.source {
        Source::Core {
            image,
            stack_pointer,
            cycle,
        } => Some(Cpu::new(image.entry, *stack_pointer, *cycle)),
        _ => None,
    });
    let deliveries = DeliveryLog::default();
    let mut machine = memory::Machine::new(plan.sources.iter().map(|req| req.id).collect())?;
    for mount in plan.routes {
        machine
            .add_route(mount.route)
            .map_err(|err| format!("{}: {err}", mount.label))?;
    }
    let names = plan
        .sources
        .iter()
        .map(|req| (req.id, req.name))
        .collect::<BTreeMap<_, _>>();
    let mut core_source = cpu.as_mut();
    for req in plan.sources {
        let registration = match req.source {
            Source::Traffic(operations) => {
                let first_at = operations.iter().map(|op| op.at).min().unwrap_or(0);
                machine.add_source(
                    req.id,
                    first_at,
                    Observe {
                        source: Traffic(operations),
                        deliveries: &deliveries,
                    },
                )
            }
            Source::Core { image, .. } => {
                for segment in image.segments {
                    // RAM starts zeroed and validated ELF segments are disjoint.
                    machine
                        .load_ram(segment.address, segment.data)
                        .map_err(|err| format!("{}: {err}", req.label))?;
                }
                machine.add_source(
                    req.id,
                    0,
                    Observe {
                        source: core_source.take().unwrap(),
                        deliveries: &deliveries,
                    },
                )
            }
        };
        registration.map_err(|err| format!("{}: {err}", req.label))?;
    }
    let result = machine.run(memory::RunLimits {
        events,
        ticks: u64::MAX,
    })?;
    encode(&result, deliveries.into_inner(), &names, cpu.as_ref())
}

fn encode(
    result: &memory::RunResult,
    deliveries: Vec<(usize, u64)>,
    names: &BTreeMap<usize, &str>,
    cpu: Option<&Cpu>,
) -> Result<String, String> {
    let commits = cpu.map_or(&[][..], Cpu::commits).iter().map(|commit| json!({
        "at": commit.at,
        "pc": commit.pc,
        "instruction": commit.instruction,
        "writeback": commit.writeback.map(|wb| json!({"register": wb.register, "value": wb.value}))
    })).collect::<Vec<_>>();
    let mut stats = Stats {
        events: result.events,
        committed: commits.len(),
        ..Stats::default()
    };
    let mut responses = BTreeMap::<_, VecDeque<_>>::new();
    for tx in &result.transactions {
        let accepted = u64::from(tx.accepted_at.is_some());
        stats.accepted += accepted;
        stats.rejected += tx.attempts.len() as u64 - accepted;
        stats.retries += tx.retries.len() as u64;
        let (kind, size_bytes) = match &tx.request.operation {
            memory::Operation::Fetch { size } => {
                stats.fetches += accepted;
                ("fetch", *size)
            }
            memory::Operation::Read { size } => {
                stats.reads += accepted;
                ("read", *size)
            }
            memory::Operation::Write { data } => {
                stats.writes += accepted;
                ("write", data.len())
            }
        };
        let (Some(accepted_at), Some(completed_at), Some(response)) =
            (tx.accepted_at, tx.completed_at, tx.result.as_ref())
        else {
            continue;
        };
        let (data, fault) = match response {
            memory::ResponseResult::ReadData(bytes) => (Some(bytes.as_slice()), None),
            memory::ResponseResult::WriteAck => (None, None),
            memory::ResponseResult::Fault(fault) => (None, Some(format!("{fault:?}"))),
        };
        let id = tx.request.id;
        let source = tx.request.source;
        responses
            .entry((source, id))
            .or_default()
            .push_back(Transaction {
                id,
                source: names[&source],
                addr: tx.request.address,
                kind,
                size_bytes,
                accepted_at,
                completed_at,
                data,
                fault,
            });
    }
    // A completed operation can still be awaiting its response when the budget expires.
    let transactions = deliveries
        .into_iter()
        .map(|key| {
            responses
                .get_mut(&key)
                .and_then(VecDeque::pop_front)
                .ok_or_else(|| {
                    format!(
                        "{}.requests: response {} has no completed transaction",
                        names[&key.0], key.1
                    )
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let (stop_reason, exit_code, fault) = match &result.stop {
        memory::StopReason::Idle => ("idle", None, None),
        memory::StopReason::Exit(code) => ("exit", Some(*code), None),
        memory::StopReason::EventLimit | memory::StopReason::TickLimit => {
            ("event_limit", None, None)
        }
        memory::StopReason::SourceStopped { reason, .. } => {
            let details = cpu.and_then(Cpu::fault);
            (
                "fault",
                None,
                Some(json!({
                    "reason": details.map_or(reason.as_str(), |fault| fault.reason.as_str()),
                    "pc": details.map(|fault| fault.pc),
                    "addr": details.and_then(|fault| fault.addr),
                })),
            )
        }
    };
    serde_json::to_string(&json!({
        "stop_reason": stop_reason,
        "final_tick": result.tick,
        "exit_code": exit_code,
        "fault": fault,
        "transactions": transactions,
        "commits": commits,
        "stats": stats,
    }))
    .map_err(|err| err.to_string())
}
