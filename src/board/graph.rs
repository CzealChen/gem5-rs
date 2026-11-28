use super::{ConfigError, ContextSnafu, DecodeSnafu, ImageSnafu};
use crate::{elf, memory};
use serde::Deserialize;
use serde_json::value::Value;
use snafu::ResultExt;
use std::{collections::BTreeMap, ops::Range};

pub(super) struct Graph {
    graph_id: u64,
    nodes: Vec<Node>,
    connections: Vec<Connection>,
    interrupt_connections: Vec<Connection>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGraph {
    graph_id: u64,
    nodes: Vec<Value>,
    connections: Vec<Value>,
    #[serde(default)]
    interrupt_connections: Vec<Connection>,
}

struct Node {
    name: String,
    model: Model,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Model {
    Traffic {
        operations: Vec<Value>,
    },
    Rv64Core {
        elf: Vec<u8>,
        stack_pointer: u64,
        cycle: u64,
        #[serde(default)]
        experiment: Option<crate::experiment::Config>,
        #[serde(default = "default_stack")]
        stack_bytes: u64,
        #[serde(default)]
        difftest: Option<crate::difftest::Config>,
        #[serde(default)]
        machine_mode: bool,
    },
    FixedRam {
        size_bytes: u64,
        latency: u64,
        capacity: u64,
    },
    DualBankRam {
        size_bytes: u64,
        latency: u64,
        bank_bytes: u64,
    },
    Exit {
        addr: u64,
        width_bytes: u64,
    },
    SimpleUart {
        addr: u64,
        size_bytes: u64,
        latency: u64,
    },
    MachineTimer {
        addr: u64,
        clock: crate::timer::ClockConfig,
        latency: u64,
    },
    Router {
        inputs: u64,
        ranges: Vec<Window>,
        priority: Vec<u64>,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Window {
    start: u64,
    size_bytes: u64,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Operation {
    Read { at: u64, addr: u64, size_bytes: u64 },
    Write { at: u64, addr: u64, data: Vec<u8> },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Connection {
    source: Endpoint,
    target: Endpoint,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Endpoint {
    graph_id: u64,
    node: u64,
    port: u64,
}

#[derive(Clone, Copy)]
enum Side {
    Input,
    Output,
}

type Port = (usize, u64);

pub(super) struct Plan {
    pub sources: Vec<Requestor>,
    pub routes: Vec<Mount>,
}

pub(super) struct Requestor {
    pub id: usize,
    pub name: String,
    pub label: String,
    pub source: Source,
}

pub(super) enum Source {
    Traffic(Vec<Scheduled>),
    Core {
        image: elf::Image,
        stack_pointer: u64,
        cycle: u64,
        experiment: Option<crate::experiment::Config>,
        difftest: Option<crate::difftest::Config>,
        machine_mode: bool,
    },
}

#[derive(Clone)]
pub(super) struct Scheduled {
    pub at: u64,
    pub id: u64,
    pub address: u64,
    pub operation: memory::Operation,
}

pub(super) struct Mount {
    pub target: MountTarget,
    pub label: String,
    range: Range<u64>,
}

pub(super) enum MountTarget {
    Timer {
        base: u64,
        clock: crate::timer::ClockConfig,
        latency: std::num::NonZeroU64,
    },
    Memory(memory::Route),
    Console {
        base: u64,
        size: usize,
        latency: std::num::NonZeroU64,
    },
}

fn default_stack() -> u64 {
    16
}

impl Graph {
    pub fn parse(json: &str) -> Result<Self, ConfigError> {
        let RawGraph {
            graph_id,
            nodes,
            connections,
            interrupt_connections,
        } = serde_json::from_str(json).context(DecodeSnafu {
            location: "board graph",
        })?;
        if graph_id == 0 {
            return Err("graph_id must be positive".into());
        }
        let nodes = nodes
            .into_iter()
            .enumerate()
            .map(|(id, mut raw)| {
                let name = raw
                    .as_object_mut()
                    .and_then(|fields| fields.remove("name"))
                    .and_then(|value| serde_json::from_value::<String>(value).ok())
                    .ok_or_else(|| format!("nodes[{id}].name: expected a string"))?;
                let model = serde_json::from_value(raw).with_context(|_| DecodeSnafu {
                    location: format!("{name} (nodes[{id}])"),
                })?;
                Ok(Node { name, model })
            })
            .collect::<Result<Vec<_>, ConfigError>>()?;
        let connections = connections
            .into_iter()
            .enumerate()
            .map(|(i, raw)| {
                serde_json::from_value(raw).with_context(|_| DecodeSnafu {
                    location: format!("connections[{i}]"),
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            graph_id,
            nodes,
            connections,
            interrupt_connections,
        })
    }

    fn endpoint(&self, endpoint: &Endpoint, side: Side) -> Result<Port, String> {
        let end = match side {
            Side::Input => "target",
            Side::Output => "source",
        };
        let id = usize::try_from(endpoint.node)
            .ok()
            .filter(|&id| id < self.nodes.len());
        if endpoint.graph_id != self.graph_id {
            let label = id.map_or_else(
                || format!("node {} port {}", endpoint.node, endpoint.port),
                |id| self.nodes[id].port(side, endpoint.port),
            );
            return Err(format!(
                "{end} {label} belongs to graph {}, expected graph {}",
                endpoint.graph_id, self.graph_id
            ));
        }
        let id = id.ok_or_else(|| {
            format!(
                "{end} node {} port {}: unknown instance",
                endpoint.node, endpoint.port
            )
        })?;
        let node = &self.nodes[id];
        if endpoint.port >= node.port_count(side) {
            let direction = match side {
                Side::Input => "input",
                Side::Output => "output",
            };
            return Err(format!(
                "{end} {}: invalid {direction} port {}",
                node.name, endpoint.port
            ));
        }
        Ok((id, endpoint.port))
    }

    pub fn plan(&self) -> Result<Plan, ConfigError> {
        self.check_interrupts()?;
        let mut outgoing = BTreeMap::new();
        let mut incoming = BTreeMap::new();
        for (i, edge) in self.connections.iter().enumerate() {
            let at = |message: String| ConfigError::Context {
                location: format!("connections[{i}]"),
                source: Box::new(message.into()),
            };
            let src = self.endpoint(&edge.source, Side::Output).map_err(at)?;
            let dst = self.endpoint(&edge.target, Side::Input).map_err(at)?;
            if outgoing.insert(src, dst).is_some() {
                return Err(at(format!(
                    "{} is already connected",
                    self.label(src, Side::Output)
                )));
            }
            if incoming.insert(dst, src).is_some() {
                return Err(at(format!(
                    "{} is already connected",
                    self.label(dst, Side::Input)
                )));
            }
        }

        let mut router = None;
        let mut requestors = Vec::new();
        for (id, node) in self.nodes.iter().enumerate() {
            if let Model::Router {
                inputs,
                ranges,
                priority,
            } = &node.model
            {
                node.check_router(*inputs, ranges, priority)?;
                if let Some(previous) = router.replace(id) {
                    return Err(ConfigError::from(format!(
                        "{} and {}: a board uses one router",
                        self.nodes[previous].port(Side::Input, 0),
                        node.port(Side::Input, 0)
                    )));
                }
            } else if node.is_requestor() {
                requestors.push(id);
            }
            for (side, edges) in [(Side::Output, &outgoing), (Side::Input, &incoming)] {
                for index in 0..node.port_count(side) {
                    if !edges.contains_key(&(id, index)) {
                        return Err(ConfigError::from(format!(
                            "{} is unconnected",
                            node.port(side, index)
                        )));
                    }
                }
            }
        }

        let mut routes = Vec::new();
        if let Some(rid) = router {
            let router = &self.nodes[rid];
            let Model::Router {
                ranges, priority, ..
            } = &router.model
            else {
                unreachable!()
            };
            for &id in &requestors {
                let dst = outgoing[&(id, 0)];
                if dst.0 != rid {
                    return Err(ConfigError::from(format!(
                        "{} connects to {}; all requestors on this board must enter {}",
                        self.nodes[id].port(Side::Output, 0),
                        self.label(dst, Side::Input),
                        router.name
                    )));
                }
            }
            requestors = priority
                .iter()
                .map(|&index| incoming[&(rid, index)].0)
                .collect();
            for (index, window) in ranges.iter().enumerate() {
                let dst = outgoing[&(rid, index as u64)];
                let label = router.port(Side::Output, index as u64);
                let target = &self.nodes[dst.0];
                if dst.0 == rid {
                    return Err(ConfigError::from(format!(
                        "{label} connects to {}; router outputs must connect to targets",
                        target.port(Side::Input, dst.1)
                    )));
                }
                routes.push(target.mount(Some((window, &label)))?);
            }
        } else if let Some(&id) = requestors.first() {
            if let Some(&other) = requestors.get(1) {
                return Err(ConfigError::from(format!(
                    "{} and {}: multiple requestors must connect through a shared router",
                    self.nodes[id].port(Side::Output, 0),
                    self.nodes[other].port(Side::Output, 0)
                )));
            }
            routes.push(self.nodes[outgoing[&(id, 0)].0].mount(None)?);
        }

        let mut has_core = false;
        let sources = requestors
            .into_iter()
            .map(|id| {
                let node = &self.nodes[id];
                let label = node.port(Side::Output, 0);
                let source = match &node.model {
                    Model::Traffic { operations } => Source::Traffic(
                        operations
                            .iter()
                            .enumerate()
                            .map(|(i, op)| {
                                Scheduled::parse(i, op).map_err(Box::new).with_context(|_| {
                                    ContextSnafu {
                                        location: format!("{label} operations[{i}]"),
                                    }
                                })
                            })
                            .collect::<Result<_, _>>()?,
                    ),
                    Model::Rv64Core {
                        elf,
                        stack_pointer,
                        cycle,
                        experiment,
                        stack_bytes,
                        difftest,
                        machine_mode,
                    } => {
                        if *machine_mode && difftest.is_some() {
                            return Err(ConfigError::from(
                                "machine_mode is not supported by the current difftest protocol"
                                    .to_owned(),
                            ));
                        }
                        if std::mem::replace(&mut has_core, true) {
                            return Err(ConfigError::from(format!(
                                "{label}: a board uses one RV64 core"
                            )));
                        }
                        if *cycle == 0 {
                            return Err(ConfigError::from(format!(
                                "{label}: cycle must be positive"
                            )));
                        }
                        Source::Core {
                            image: image(elf, *stack_pointer, *stack_bytes, &routes)
                                .map_err(Box::new)
                                .with_context(|_| ContextSnafu {
                                    location: label.clone(),
                                })?,
                            machine_mode: *machine_mode,
                            stack_pointer: *stack_pointer,
                            cycle: *cycle,
                            experiment: experiment
                                .map(crate::experiment::Config::validate)
                                .transpose()?,
                            difftest: difftest
                                .clone()
                                .map(crate::difftest::Config::validate)
                                .transpose()?,
                        }
                    }
                    _ => unreachable!(),
                };
                Ok(Requestor {
                    id,
                    name: node.name.clone(),
                    label,
                    source,
                })
            })
            .collect::<Result<_, ConfigError>>()?;
        Ok(Plan { sources, routes })
    }

    fn check_interrupts(&self) -> Result<(), ConfigError> {
        let timers: Vec<_> = self
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| matches!(n.model, Model::MachineTimer { .. }))
            .map(|(id, _)| id)
            .collect();
        if timers.len() > 1 {
            return Err("a single-hart board supports at most one machine timer".into());
        }
        if self.interrupt_connections.len() != timers.len() {
            return Err("each machine timer needs exactly one interrupt connection".into());
        }
        for edge in &self.interrupt_connections {
            for endpoint in [&edge.source, &edge.target] {
                if endpoint.graph_id != self.graph_id
                    || endpoint.port != 0
                    || endpoint.node >= self.nodes.len() as u64
                {
                    return Err(
                        "invalid interrupt endpoint or cross-board interrupt connection".into(),
                    );
                }
            }
            if !matches!(
                self.nodes[edge.source.node as usize].model,
                Model::MachineTimer { .. }
            ) || !matches!(
                self.nodes[edge.target.node as usize].model,
                Model::Rv64Core {
                    machine_mode: true,
                    ..
                }
            ) {
                return Err(
                    "interrupt connection must go from a machine timer to a machine-mode core"
                        .into(),
                );
            }
        }
        Ok(())
    }

    fn label(&self, (id, index): Port, side: Side) -> String {
        self.nodes[id].port(side, index)
    }
}

impl Node {
    fn is_requestor(&self) -> bool {
        matches!(self.model, Model::Traffic { .. } | Model::Rv64Core { .. })
    }

    fn port_count(&self, side: Side) -> u64 {
        match (&self.model, side) {
            (Model::Router { inputs, .. }, Side::Input) => *inputs,
            (Model::Router { ranges, .. }, Side::Output) => ranges.len() as u64,
            (_, Side::Input) => u64::from(!self.is_requestor()),
            (_, Side::Output) => u64::from(self.is_requestor()),
        }
    }

    fn port(&self, side: Side, index: u64) -> String {
        let name = &self.name;
        match (&self.model, side, index) {
            (Model::Router { .. }, Side::Input, i) => format!("{name}.inputs[{i}]"),
            (Model::Router { .. }, Side::Output, i) => format!("{name}.outputs[{i}]"),
            (_, Side::Input, 0) if !self.is_requestor() => format!("{name}.input"),
            (_, Side::Output, 0) if self.is_requestor() => format!("{name}.requests"),
            (_, Side::Input, i) => format!("{name}.input[{i}]"),
            (_, Side::Output, i) => format!("{name}.output[{i}]"),
        }
    }

    fn check_router(&self, inputs: u64, ranges: &[Window], priority: &[u64]) -> Result<(), String> {
        if inputs == 0 || priority.len() as u64 != inputs {
            return Err(format!(
                "{}.inputs: priority must list all ingress ports once and inputs must be positive",
                self.name
            ));
        }
        let mut seen = vec![false; priority.len()];
        for &index in priority {
            let port = self.port(Side::Input, index);
            if index >= inputs {
                return Err(format!("{port}: priority names an unknown ingress"));
            }
            if std::mem::replace(&mut seen[index as usize], true) {
                return Err(format!("{port}: duplicate ingress in priority"));
            }
        }
        if ranges.is_empty() {
            return Err(format!(
                "{}.outputs: router needs at least one address range",
                self.name
            ));
        }
        let mut windows = ranges
            .iter()
            .enumerate()
            .map(|(i, range)| {
                address_range(range.start, range.size_bytes)
                    .map(|range| (range, i))
                    .map_err(|err| format!("{}: {err}", self.port(Side::Output, i as u64)))
            })
            .collect::<Result<Vec<_>, _>>()?;
        windows.sort_unstable_by_key(|(range, _)| range.start);
        for pair in windows.windows(2) {
            let [(a, i), (b, j)] = pair else {
                unreachable!()
            };
            if a.end > b.start {
                return Err(format!(
                    "{} overlaps {}",
                    self.port(Side::Output, *i as u64),
                    self.port(Side::Output, *j as u64)
                ));
            }
        }
        Ok(())
    }

    fn mount(&self, window: Option<(&Window, &str)>) -> Result<Mount, String> {
        use memory::Target;

        let label = self.port(Side::Input, 0);
        let err = |message| format!("{label}: {message}");
        let positive = |size: u64, what| {
            usize::try_from(size)
                .ok()
                .filter(|&size| size != 0)
                .ok_or_else(|| err(format!("{what} must be positive and fit the platform")))
        };
        let ram_size = |bytes| {
            let size = positive(bytes, "size_bytes")?;
            (size <= isize::MAX as usize)
                .then_some(size)
                .ok_or_else(|| err("size_bytes exceeds the platform allocation limit".into()))
        };
        let router_base = window.map_or(0, |(window, _)| window.start);
        if let Model::MachineTimer {
            addr,
            clock,
            latency,
        } = self.model
        {
            if addr % 8 != 0 {
                return Err(err("timer address must be 8-byte aligned".into()));
            }
            let clock = clock.validate().map_err(err)?;
            let latency = std::num::NonZeroU64::new(latency)
                .ok_or_else(|| err("timer latency must be positive".into()))?;
            if let Some((window, _)) = window
                && (window.start, window.size_bytes) != (addr, 16)
            {
                return Err(err(
                    "timer window must match the 16-byte register range".into()
                ));
            }
            return Ok(Mount {
                target: MountTarget::Timer {
                    base: addr,
                    clock,
                    latency,
                },
                range: address_range(addr, 16).map_err(err)?,
                label,
            });
        }
        if let Model::SimpleUart {
            addr,
            size_bytes,
            latency,
        } = self.model
        {
            let latency = std::num::NonZeroU64::new(latency)
                .ok_or_else(|| err("latency must be positive".into()))?;
            let size = positive(size_bytes, "size_bytes")?;
            if let Some((window, _)) = window
                && (window.start, window.size_bytes) != (addr, size_bytes)
            {
                return Err(err("console window must match target range".into()));
            }
            return Ok(Mount {
                target: MountTarget::Console {
                    base: addr,
                    size,
                    latency,
                },
                range: address_range(addr, size_bytes).map_err(err)?,
                label,
            });
        }
        let (base, size, target) = match self.model {
            Model::FixedRam {
                size_bytes,
                latency,
                capacity,
            } => (
                router_base,
                size_bytes,
                Target::FixedRam {
                    size_bytes: ram_size(size_bytes)?,
                    latency,
                    capacity: positive(capacity, "capacity")?,
                },
            ),
            Model::DualBankRam {
                size_bytes,
                latency,
                bank_bytes,
            } => {
                if bank_bytes >= size_bytes {
                    return Err(err("bank_bytes must be smaller than size_bytes".into()));
                }
                (
                    router_base,
                    size_bytes,
                    Target::BankedRam {
                        size_bytes: ram_size(size_bytes)?,
                        latency,
                        bank_bytes: positive(bank_bytes, "bank_bytes")?,
                    },
                )
            }
            Model::Exit { addr, width_bytes } => {
                if !matches!(width_bytes, 1 | 2 | 4 | 8) || addr % width_bytes != 0 {
                    return Err(err("exit width must be 1, 2, 4 or 8 bytes and addr must be aligned to that width".into()));
                }
                (
                    addr,
                    width_bytes,
                    Target::ExitMmio {
                        width: width_bytes as usize,
                        latency: 1,
                    },
                )
            }
            _ => unreachable!(),
        };
        if matches!(
            target,
            Target::FixedRam { latency: 0, .. } | Target::BankedRam { latency: 0, .. }
        ) {
            return Err(err("latency must be positive".into()));
        }
        if let Some((window, output)) = window
            && (window.start, window.size_bytes) != (base, size)
        {
            return Err(format!(
                "{output} range must match {label} ({base:#x}, {size} bytes)"
            ));
        }
        let range = address_range(base, size).map_err(err)?;
        Ok(Mount {
            target: MountTarget::Memory(memory::Route { base, target }),
            range,
            label,
        })
    }
}

impl Scheduled {
    fn parse(id: usize, raw: &Value) -> Result<Self, ConfigError> {
        let op: Operation = serde_json::from_value(raw.clone()).context(DecodeSnafu {
            location: "operation",
        })?;
        let (at, address, operation) = match op {
            Operation::Read {
                at,
                addr,
                size_bytes,
            } => (
                at,
                addr,
                memory::Operation::Read {
                    size: usize::try_from(size_bytes)
                        .map_err(|_| "size_bytes exceeds platform size")?,
                },
            ),
            Operation::Write { at, addr, data } => (at, addr, memory::Operation::Write { data }),
        };
        Ok(Self {
            at,
            id: id as u64,
            address,
            operation,
        })
    }
}

fn address_range(base: u64, size: u64) -> Result<Range<u64>, String> {
    if size == 0 {
        return Err("address range must be nonempty".into());
    }
    let end = base
        .checked_add(size)
        .ok_or("address range end overflows u64")?;
    Ok(base..end)
}

fn image(
    bytes: &[u8],
    sp: u64,
    stack_bytes: u64,
    routes: &[Mount],
) -> Result<elf::Image, ConfigError> {
    const STACK_ALIGNMENT: u64 = 16;
    let bottom = sp
        .checked_sub(stack_bytes)
        .filter(|_| {
            stack_bytes > 0
                && stack_bytes.is_multiple_of(STACK_ALIGNMENT)
                && sp.is_multiple_of(STACK_ALIGNMENT)
        })
        .ok_or("stack_pointer needs one aligned 16-byte stack slot")?;
    let ram = routes
        .iter()
        .find(|mount| {
            matches!(
                mount.target,
                MountTarget::Memory(memory::Route {
                    target: memory::Target::FixedRam { .. } | memory::Target::BankedRam { .. },
                    ..
                })
            ) && mount.range.start <= bottom
                && sp <= mount.range.end
        })
        .ok_or("stack_pointer must have stack space in connected RAM")?;
    elf::load_at(bytes, ram.range.clone(), sp, stack_bytes).with_context(|_| ImageSnafu {
        location: ram.label.clone(),
    })
}
