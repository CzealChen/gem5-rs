//! Event-driven transactions shared by request sources and memory targets.

use crate::sim::Queue;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::num::NonZeroU64;

pub type SourceId = usize;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operation {
    Fetch { size: usize },
    Read { size: usize },
    Write { data: Vec<u8> },
}

impl Operation {
    fn size(&self) -> usize {
        match self {
            Self::Fetch { size } | Self::Read { size } => *size,
            Self::Write { data } => data.len(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub source: SourceId,
    pub id: u64,
    pub address: u64,
    pub operation: Operation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum Fault {
    ZeroSize,
    AddressOverflow,
    Unmapped,
    CrossesRoute,
    Misaligned,
    InvalidMmioAccess,
    ExitAlreadyRequested,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResponseResult {
    ReadData(Vec<u8>),
    WriteAck,
    Fault(Fault),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub source: SourceId,
    pub id: u64,
    pub result: ResponseResult,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceEvent {
    Wake,
    Response(Response),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceAction {
    Send {
        at: u64,
        id: u64,
        address: u64,
        operation: Operation,
    },
    WakeAt(u64),
    Stop(String),
    Exit(u64),
}

/// A request producer driven by its timers and transaction responses.
/// The machine holds each sent request until it is accepted and completed.
pub trait Source {
    fn on_event(&mut self, at: u64, event: SourceEvent) -> Vec<SourceAction>;
}

impl<S: Source + ?Sized> Source for &mut S {
    fn on_event(&mut self, at: u64, event: SourceEvent) -> Vec<SourceAction> {
        (**self).on_event(at, event)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    FixedRam {
        size_bytes: usize,
        latency: u64,
        capacity: usize,
    },
    BankedRam {
        size_bytes: usize,
        latency: u64,
        bank_bytes: usize,
    },
    ExitMmio {
        width: usize,
        latency: u64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Route {
    pub base: u64,
    pub target: Target,
}

/// Target behavior mounted into a single address range by [`Machine::add_target`].
/// Offsets are relative to the route's base. The machine checks that the complete
/// access fits inside the route before calling the target.
pub trait MemoryTarget {
    /// Exactly-once notification after acceptance and response scheduling.
    /// Rejected requests never reach this method.
    fn accepted(&mut self, _at: u64, _offset: usize, _request: &Request) {}
    /// Validate the request and choose a lane from the registered capacities at arrival.
    /// Retries use the same lane and preserve the original request.
    fn select_lane(&self, offset: usize, request: &Request) -> Result<usize, Fault>;

    /// Service time measured from the grant at `at`.
    fn service_latency(&self, at: u64, offset: usize, request: &Request) -> NonZeroU64;

    /// Apply the access at its completion tick and produce its terminal result.
    fn complete(&mut self, at: u64, offset: usize, request: &Request) -> TargetCompletion;

    /// Initialize a routed byte span during assembly.
    fn initialize(&mut self, _offset: usize, _data: &[u8]) -> Result<(), String> {
        Err("initialization requires a writable RAM target".into())
    }
}

pub enum TargetCompletion {
    Respond(ResponseResult),
    /// Acknowledge the write and request machine exit. The first exit code wins;
    /// the machine delivers all accepted responses before stopping.
    Exit(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunLimits {
    pub events: u64,
    pub ticks: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StopReason {
    Idle,
    Exit(u64),
    SourceStopped { source: SourceId, reason: String },
    EventLimit,
    TickLimit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transaction {
    pub request: Request,
    pub arrived_at: u64,
    /// Ticks at which this transaction competed for acceptance.
    pub attempts: Vec<u64>,
    /// Attempts following a capacity notification.
    pub retries: Vec<u64>,
    pub accepted_at: Option<u64>,
    pub completed_at: Option<u64>,
    pub result: Option<ResponseResult>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunResult {
    pub stop: StopReason,
    pub tick: u64,
    pub events: u64,
    pub transactions: Vec<Transaction>,
}

struct Ram {
    bytes: Vec<u8>,
    bank_bytes: Option<usize>,
    latency: NonZeroU64,
}

impl MemoryTarget for Ram {
    fn select_lane(&self, offset: usize, request: &Request) -> Result<usize, Fault> {
        if !request
            .address
            .is_multiple_of(request.operation.size() as u64)
        {
            return Err(Fault::Misaligned);
        }
        Ok(self.bank_bytes.map_or(0, |stripe| offset / stripe % 2))
    }

    fn service_latency(&self, _at: u64, _offset: usize, _request: &Request) -> NonZeroU64 {
        self.latency
    }

    fn complete(&mut self, _at: u64, offset: usize, request: &Request) -> TargetCompletion {
        let area = &mut self.bytes[offset..offset + request.operation.size()];
        TargetCompletion::Respond(match &request.operation {
            Operation::Write { data } => {
                area.copy_from_slice(data);
                ResponseResult::WriteAck
            }
            _ => ResponseResult::ReadData(area.to_vec()),
        })
    }

    fn initialize(&mut self, offset: usize, data: &[u8]) -> Result<(), String> {
        self.bytes[offset..offset + data.len()].copy_from_slice(data);
        Ok(())
    }
}

struct ExitRegister {
    width: usize,
    latency: NonZeroU64,
}

impl MemoryTarget for ExitRegister {
    fn select_lane(&self, offset: usize, request: &Request) -> Result<usize, Fault> {
        match &request.operation {
            Operation::Write { data } if offset == 0 && data.len() == self.width => Ok(0),
            _ => Err(Fault::InvalidMmioAccess),
        }
    }

    fn service_latency(&self, _at: u64, _offset: usize, _request: &Request) -> NonZeroU64 {
        self.latency
    }

    fn complete(&mut self, _at: u64, _offset: usize, request: &Request) -> TargetCompletion {
        let Operation::Write { data } = &request.operation else {
            unreachable!()
        };
        let mut code = [0; 8];
        code[..data.len()].copy_from_slice(data);
        TargetCompletion::Exit(u64::from_le_bytes(code))
    }

    fn initialize(&mut self, _offset: usize, _data: &[u8]) -> Result<(), String> {
        Err("targets MMIO".into())
    }
}

struct Lane {
    capacity: usize,
    occupied: usize,
    next_port: usize,
    pending: Vec<usize>,
    scheduled: bool,
}

impl Lane {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            occupied: 0,
            next_port: 0,
            pending: Vec::new(),
            scheduled: false,
        }
    }
}

struct Mounted<'a> {
    end: u64,
    target: Box<dyn MemoryTarget + 'a>,
    lanes: Vec<Lane>,
}

// Every same-tick completion precedes callbacks, arrivals, and arbitration.
const COMPLETE: u8 = 0;
const RESPOND: u8 = 1;
const SOURCE: u8 = 2;
const ARBITRATE: u8 = 3;

#[derive(Clone, Copy)]
struct Slot {
    base: u64,
    lane: usize,
}

enum Event {
    Wake(SourceId),
    Arrival(Request),
    Arbitrate(Slot),
    Complete {
        tx: usize,
        slot: Result<Slot, Fault>,
    },
    Respond {
        tx: usize,
        exit: Option<u64>,
    },
}

pub struct Machine<'a> {
    queue: Queue<Event>,
    ranks: BTreeMap<SourceId, usize>,
    sources: BTreeMap<SourceId, Box<dyn Source + 'a>>,
    routes: BTreeMap<u64, Mounted<'a>>,
    transactions: Vec<Transaction>,
    live: BTreeSet<(SourceId, u64)>,
    in_flight: usize,
    exit_latched: bool,
    stopping: Option<StopReason>,
}

impl<'a> Machine<'a> {
    pub fn new(port_order: Vec<SourceId>) -> Result<Self, String> {
        let mut ranks = BTreeMap::new();
        for (rank, id) in port_order.into_iter().enumerate() {
            if ranks.insert(id, rank).is_some() {
                return Err(format!("duplicate request port for source {id}"));
            }
        }
        Ok(Self {
            queue: Queue::new(),
            ranks,
            sources: BTreeMap::new(),
            routes: BTreeMap::new(),
            transactions: Vec::new(),
            live: BTreeSet::new(),
            in_flight: 0,
            exit_latched: false,
            stopping: None,
        })
    }

    pub fn add_source(
        &mut self,
        id: SourceId,
        first_at: u64,
        source: impl Source + 'a,
    ) -> Result<(), String> {
        if !self.ranks.contains_key(&id) {
            return Err(format!("source {id} has no request port"));
        }
        if self.sources.contains_key(&id) {
            return Err(format!("source {id} is already registered"));
        }
        self.queue.schedule(first_at, SOURCE, Event::Wake(id))?;
        self.sources.insert(id, Box::new(source));
        Ok(())
    }

    pub fn add_route(&mut self, Route { base, target }: Route) -> Result<(), String> {
        let (size, latency, capacities, bank_bytes) = match target {
            Target::FixedRam {
                size_bytes,
                latency,
                capacity,
            } if capacity > 0 => (size_bytes, latency, vec![capacity], None),
            Target::BankedRam {
                size_bytes,
                latency,
                bank_bytes,
            } if bank_bytes > 0 && bank_bytes < size_bytes => {
                (size_bytes, latency, vec![1, 1], Some(bank_bytes))
            }
            Target::ExitMmio { width, latency } if matches!(width, 1 | 2 | 4 | 8) => {
                (width, latency, vec![1], None)
            }
            _ => return Err(format!("invalid target configuration at {base:#x}")),
        };
        let Some(latency) = NonZeroU64::new(latency).filter(|_| size > 0) else {
            return Err(format!(
                "target at {base:#x} needs positive size and latency"
            ));
        };
        if matches!(target, Target::ExitMmio { .. }) && !base.is_multiple_of(size as u64) {
            return Err(format!(
                "exit register at {base:#x} needs {size}-byte alignment"
            ));
        }
        let end = self.route_end(base, size)?;
        let target: Box<dyn MemoryTarget + 'a> = match target {
            Target::ExitMmio { .. } => Box::new(ExitRegister {
                width: size,
                latency,
            }),
            _ => {
                let mut bytes = Vec::new();
                bytes
                    .try_reserve_exact(size)
                    .map_err(|err| err.to_string())?;
                bytes.resize(size, 0);
                Box::new(Ram {
                    bytes,
                    bank_bytes,
                    latency,
                })
            }
        };
        self.mount(base, end, &capacities, target);
        Ok(())
    }

    /// Register a Rust target with one positive capacity per lane, in lane-index order.
    pub fn add_target(
        &mut self,
        base: u64,
        size_bytes: usize,
        lane_capacities: &[usize],
        target: impl MemoryTarget + 'a,
    ) -> Result<(), String> {
        let end = self.route_end(base, size_bytes)?;
        if lane_capacities.is_empty() || lane_capacities.contains(&0) {
            return Err(format!(
                "target at {base:#x} needs lanes with positive capacities"
            ));
        }
        self.mount(base, end, lane_capacities, Box::new(target));
        Ok(())
    }

    fn route_end(&self, base: u64, size: usize) -> Result<u64, String> {
        let end = range_end(base, size).map_err(|fault| format!("route {base:#x}: {fault:?}"))?;
        if let Some((other, _)) = self
            .routes
            .range(..=end)
            .next_back()
            .filter(|(_, route)| route.end >= base)
        {
            return Err(format!("route {base:#x} overlaps route {other:#x}"));
        }
        Ok(end)
    }

    fn mount(
        &mut self,
        base: u64,
        end: u64,
        capacities: &[usize],
        target: Box<dyn MemoryTarget + 'a>,
    ) {
        self.routes.insert(
            base,
            Mounted {
                end,
                target,
                lanes: capacities.iter().copied().map(Lane::new).collect(),
            },
        );
    }

    /// Initialize a span inside one RAM route during assembly.
    pub fn load_ram(&mut self, address: u64, data: &[u8]) -> Result<(), String> {
        if data.is_empty() {
            return Ok(());
        }
        let base = self
            .route(address, data.len())
            .map_err(|fault| format!("RAM initialization at {address:#x}: {fault:?}"))?;
        let offset = (address - base) as usize;
        self.routes
            .get_mut(&base)
            .unwrap()
            .target
            .initialize(offset, data)
            .map_err(|err| format!("RAM initialization at {address:#x}: {err}"))
    }

    pub fn run(mut self, limits: RunLimits) -> Result<RunResult, String> {
        if let Some(id) = self.ranks.keys().find(|id| !self.sources.contains_key(id)) {
            return Err(format!("request port for source {id} is unbound"));
        }
        let mut events = 0;
        let mut tick = self.queue.now();
        let stop = loop {
            if self.in_flight == 0
                && let Some(stop) = self.stopping.take()
            {
                break stop;
            }
            if self.queue.is_empty() {
                debug_assert!(self.live.is_empty());
                break StopReason::Idle;
            }
            if events == limits.events {
                break StopReason::EventLimit;
            }
            let (at, event) = self.queue.pop().unwrap();
            if at > limits.ticks {
                tick = limits.ticks;
                break StopReason::TickLimit;
            }
            tick = at;
            events += 1;
            match event {
                Event::Complete { tx, slot } => self.complete(tx, slot)?,
                Event::Respond { tx, exit } => self.respond(tx, exit)?,
                _ if self.stopping.is_some() => {}
                Event::Wake(id) => self.notify(id, SourceEvent::Wake)?,
                Event::Arrival(request) => self.arrive(request)?,
                Event::Arbitrate(slot) => self.arbitrate(slot)?,
            }
        };
        Ok(RunResult {
            stop,
            tick,
            events,
            transactions: self.transactions,
        })
    }

    fn route(&self, address: u64, size: usize) -> Result<u64, Fault> {
        let end = range_end(address, size)?;
        let (&base, route) = self
            .routes
            .range(..=address)
            .next_back()
            .ok_or(Fault::Unmapped)?;
        if address > route.end {
            Err(Fault::Unmapped)
        } else if end > route.end {
            Err(Fault::CrossesRoute)
        } else {
            Ok(base)
        }
    }

    fn slot(&self, request: &Request) -> Result<Slot, Fault> {
        let base = self.route(request.address, request.operation.size())?;
        let offset = (request.address - base) as usize;
        let lane = self.routes[&base].target.select_lane(offset, request)?;
        Ok(Slot { base, lane })
    }

    fn notify(&mut self, id: SourceId, event: SourceEvent) -> Result<(), String> {
        let actions = self
            .sources
            .get_mut(&id)
            .unwrap()
            .on_event(self.queue.now(), event);
        if self.stopping.is_some() {
            return Ok(());
        }
        for action in actions {
            match action {
                SourceAction::Send {
                    at,
                    id: tx,
                    address,
                    operation,
                } => self.queue.schedule(
                    at,
                    SOURCE,
                    Event::Arrival(Request {
                        source: id,
                        id: tx,
                        address,
                        operation,
                    }),
                )?,
                SourceAction::WakeAt(at) => self.queue.schedule(at, SOURCE, Event::Wake(id))?,
                SourceAction::Stop(reason) => {
                    self.stopping = Some(StopReason::SourceStopped { source: id, reason });
                    break;
                }
                SourceAction::Exit(code) => {
                    self.stopping = Some(StopReason::Exit(code));
                    break;
                }
            }
        }
        Ok(())
    }

    fn arrive(&mut self, request: Request) -> Result<(), String> {
        if !self.live.insert((request.source, request.id)) {
            return Err(format!(
                "source {} reused in-flight transaction ID {}",
                request.source, request.id
            ));
        }
        let slot = self.slot(&request);
        let tx = self.transactions.len();
        self.transactions.push(Transaction {
            request,
            arrived_at: self.queue.now(),
            attempts: Vec::new(),
            retries: Vec::new(),
            accepted_at: None,
            completed_at: None,
            result: None,
        });
        match slot {
            Ok(slot) => {
                if slot.lane >= self.routes[&slot.base].lanes.len() {
                    return Err(format!(
                        "target at {:#x} selected unconfigured lane {}",
                        slot.base, slot.lane
                    ));
                }
                self.lane_mut(slot).pending.push(tx);
                self.schedule_arbitration(slot)
            }
            Err(fault) => {
                self.transactions[tx].attempts.push(self.queue.now());
                self.accept(tx, Err(fault), 1)
            }
        }
    }

    fn lane_mut(&mut self, slot: Slot) -> &mut Lane {
        &mut self.routes.get_mut(&slot.base).unwrap().lanes[slot.lane]
    }

    fn schedule_arbitration(&mut self, slot: Slot) -> Result<(), String> {
        let lane = self.lane_mut(slot);
        if !lane.scheduled && !lane.pending.is_empty() {
            lane.scheduled = true;
            self.queue
                .schedule(self.queue.now(), ARBITRATE, Event::Arbitrate(slot))?;
        }
        Ok(())
    }

    fn arbitrate(&mut self, slot: Slot) -> Result<(), String> {
        let at = self.queue.now();
        let route = self.routes.get_mut(&slot.base).unwrap();
        let lane = &mut route.lanes[slot.lane];
        lane.scheduled = false;
        let (mut available, mut cursor) = (lane.capacity - lane.occupied, lane.next_port);
        let retry_waiters = available > 0;
        let mut ready = BTreeMap::<usize, VecDeque<usize>>::new();
        let mut blocked = Vec::new();
        for tx in std::mem::take(&mut lane.pending) {
            let record = &mut self.transactions[tx];
            if record.attempts.is_empty() || retry_waiters {
                if !record.attempts.is_empty() {
                    record.retries.push(at);
                }
                record.attempts.push(at);
                ready
                    .entry(self.ranks[&record.request.source])
                    .or_default()
                    .push_back(tx);
            } else {
                blocked.push(tx);
            }
        }

        // Each port keeps FIFO order; each grant moves the cursor to the next port.
        let mut accepted = Vec::new();
        while available > 0 && !ready.is_empty() {
            let (&rank, _) = ready
                .range(cursor..)
                .next()
                .or_else(|| ready.first_key_value())
                .unwrap();
            let txs = ready.get_mut(&rank).unwrap();
            accepted.push(txs.pop_front().unwrap());
            if txs.is_empty() {
                ready.remove(&rank);
            }
            available -= 1;
            cursor = (rank + 1) % self.ranks.len();
        }
        blocked.extend(ready.into_values().flatten());
        lane.pending = blocked;
        lane.next_port = cursor;
        lane.occupied += accepted.len();
        for tx in accepted {
            let request = &self.transactions[tx].request;
            let offset = (request.address - slot.base) as usize;
            let latency = self.routes[&slot.base]
                .target
                .service_latency(at, offset, request);
            self.accept(tx, Ok(slot), latency.get())?;
        }
        Ok(())
    }

    fn accept(&mut self, tx: usize, slot: Result<Slot, Fault>, latency: u64) -> Result<(), String> {
        let at = self.queue.now();
        let complete_at = at
            .checked_add(latency)
            .ok_or_else(|| format!("completion tick overflow at {at}"))?;
        self.queue
            .schedule(complete_at, COMPLETE, Event::Complete { tx, slot })?;
        self.transactions[tx].accepted_at = Some(at);
        self.in_flight += 1;
        if let Ok(slot) = slot {
            let request = &self.transactions[tx].request;
            self.routes.get_mut(&slot.base).unwrap().target.accepted(
                at,
                (request.address - slot.base) as usize,
                request,
            );
        }
        Ok(())
    }

    fn complete(&mut self, tx: usize, slot: Result<Slot, Fault>) -> Result<(), String> {
        let at = self.queue.now();
        let record = &mut self.transactions[tx];
        let mut exit = None;
        let result = match slot {
            Err(fault) => ResponseResult::Fault(fault),
            Ok(slot) => {
                let route = self.routes.get_mut(&slot.base).unwrap();
                route.lanes[slot.lane].occupied -= 1;
                let offset = (record.request.address - slot.base) as usize;
                match route.target.complete(at, offset, &record.request) {
                    TargetCompletion::Respond(result) => result,
                    TargetCompletion::Exit(_) if self.exit_latched => {
                        ResponseResult::Fault(Fault::ExitAlreadyRequested)
                    }
                    TargetCompletion::Exit(code) => {
                        exit = Some(code);
                        self.exit_latched = true;
                        ResponseResult::WriteAck
                    }
                }
            }
        };
        record.completed_at = Some(at);
        record.result = Some(result);
        self.queue
            .schedule(at, RESPOND, Event::Respond { tx, exit })?;
        if let Ok(slot) = slot {
            self.schedule_arbitration(slot)?;
        }
        Ok(())
    }

    fn respond(&mut self, tx: usize, exit: Option<u64>) -> Result<(), String> {
        let record = &self.transactions[tx];
        let response = Response {
            source: record.request.source,
            id: record.request.id,
            result: record.result.clone().unwrap(),
        };
        self.live.remove(&(response.source, response.id));
        self.in_flight -= 1;
        if let Some(code) = exit {
            self.sources
                .get_mut(&response.source)
                .unwrap()
                .on_event(self.queue.now(), SourceEvent::Response(response));
            self.stopping = Some(StopReason::Exit(code));
            Ok(())
        } else {
            self.notify(response.source, SourceEvent::Response(response))
        }
    }
}

fn range_end(address: u64, size: usize) -> Result<u64, Fault> {
    let width = size.checked_sub(1).ok_or(Fault::ZeroSize)?;
    let width = u64::try_from(width).map_err(|_| Fault::AddressOverflow)?;
    address.checked_add(width).ok_or(Fault::AddressOverflow)
}
