from __future__ import annotations

import json
from dataclasses import dataclass
from itertools import count
from typing import Protocol, overload

from . import _native
from ._types import (
    CoreHandle,
    DualBankRam,
    Exit,
    FixedRam,
    InterruptIn,
    InterruptOut,
    MachineTimer,
    Read,
    RequestorHandle,
    RequestOut,
    Router,
    RouterHandle,
    RunLimit,
    RunResult,
    Rv64Core,
    SimpleUart,
    SimulatedTimerClock,
    TargetHandle,
    TargetIn,
    TimerHandle,
    Traffic,
    Write,
)
from ._wire import decode_result

type _Model = (
    Traffic
    | FixedRam
    | DualBankRam
    | Exit
    | SimpleUart
    | Rv64Core
    | Router
    | MachineTimer
)
type _Handle = RequestorHandle | TargetHandle | RouterHandle | TimerHandle

_graph_ids = count(1)


def _operation(operation: Read | Write) -> dict[str, object]:
    if isinstance(operation, Read):
        return {
            "kind": "read",
            "at": operation.at,
            "addr": operation.addr,
            "size_bytes": operation.size_bytes,
        }
    return {
        "kind": "write",
        "at": operation.at,
        "addr": operation.addr,
        "data": list(operation.data),
    }


def _node(model: _Model) -> dict[str, object]:
    match model:
        case Traffic(operations):
            return {
                "kind": "traffic",
                "operations": [_operation(op) for op in operations],
            }
        case FixedRam(size_bytes, latency, capacity):
            return {
                "kind": "fixed_ram",
                "size_bytes": size_bytes,
                "latency": latency,
                "capacity": capacity,
            }
        case DualBankRam(size_bytes, latency, bank_bytes):
            return {
                "kind": "dual_bank_ram",
                "size_bytes": size_bytes,
                "latency": latency,
                "bank_bytes": bank_bytes,
            }
        case Exit(addr, width_bytes):
            return {"kind": "exit", "addr": addr, "width_bytes": width_bytes}
        case SimpleUart(addr, size_bytes, latency):
            return {
                "kind": "simple_uart",
                "addr": addr,
                "size_bytes": size_bytes,
                "latency": latency,
            }
        case MachineTimer(addr, clock, latency):
            clock_config: dict[str, object]
            if isinstance(clock, SimulatedTimerClock):
                clock_config = {"kind": "simulated", "tick_period": clock.tick_period}
            else:
                clock_config = {"kind": "wall", "frequency_hz": clock.frequency_hz}
            return {
                "kind": "machine_timer",
                "addr": addr,
                "clock": clock_config,
                "latency": latency,
            }
        case Rv64Core(
            elf, stack_pointer, cycle, experiment, stack_bytes, difftest, machine_mode
        ):
            return {
                "kind": "rv64_core",
                "machine_mode": machine_mode,
                "elf": list(elf),
                "stack_pointer": stack_pointer,
                "cycle": cycle,
                "experiment": None
                if experiment is None
                else {
                    "tick_femtoseconds": experiment.tick_femtoseconds,
                    "instruction_budget": experiment.instruction_budget,
                },
                "stack_bytes": stack_bytes,
                "difftest": None
                if difftest is None
                else {
                    "adapter": difftest.adapter,
                    "timeout_ms": difftest.timeout_ms,
                    "expected_exit_code": difftest.expected_exit_code,
                    "expected_failure": difftest.expected_failure,
                    "allow_instruction_limit": difftest.allow_instruction_limit,
                },
            }
        case Router(inputs, ranges, priority):
            return {
                "kind": "router",
                "inputs": inputs,
                "ranges": [
                    {"start": window.start, "size_bytes": window.size_bytes}
                    for window in ranges
                ],
                "priority": list(range(inputs) if priority is None else priority),
            }


def _endpoint(
    port: RequestOut | TargetIn | InterruptOut | InterruptIn,
) -> dict[str, int]:
    return {"graph_id": port._graph_id, "node": port._node, "port": port._port}


class Board:
    """Declare models and connect their request outputs to target inputs."""

    def __init__(self) -> None:
        self._graph_id = next(_graph_ids)
        self._nodes: list[dict[str, object]] = []
        self._connections: list[dict[str, dict[str, int]]] = []
        self._interrupt_connections: list[dict[str, dict[str, int]]] = []

    @overload
    def add(self, model: Traffic, *, name: str | None = None) -> RequestorHandle: ...

    @overload
    def add(self, model: Rv64Core, *, name: str | None = None) -> CoreHandle: ...

    @overload
    def add(self, model: MachineTimer, *, name: str | None = None) -> TimerHandle: ...

    @overload
    def add(
        self,
        model: FixedRam | DualBankRam | Exit | SimpleUart,
        *,
        name: str | None = None,
    ) -> TargetHandle: ...

    @overload
    def add(self, model: Router, *, name: str | None = None) -> RouterHandle: ...

    def add(self, model: _Model, *, name: str | None = None) -> _Handle:
        """Add a model instance and return its typed ports."""
        index = len(self._nodes)
        node = _node(model)
        node["name"] = f"{node['kind']}[{index}]" if name is None else name
        graph_id = self._graph_id
        handle: _Handle
        if isinstance(model, Rv64Core):
            handle = CoreHandle(
                RequestOut(graph_id, index, 0), InterruptIn(graph_id, index, 0)
            )
        elif isinstance(model, MachineTimer):
            handle = TimerHandle(
                TargetIn(graph_id, index, 0), InterruptOut(graph_id, index, 0)
            )
        elif isinstance(model, Traffic):
            handle = RequestorHandle(RequestOut(graph_id, index, 0))
        elif isinstance(model, Router):
            handle = RouterHandle(
                tuple(TargetIn(graph_id, index, i) for i in range(model.inputs)),
                tuple(RequestOut(graph_id, index, i) for i in range(len(model.ranges))),
            )
        else:
            handle = TargetHandle(TargetIn(graph_id, index, 0))
        self._nodes.append(node)
        return handle

    def connect(self, source: RequestOut, target: TargetIn) -> None:
        """Connect a request output to a target input."""
        if not isinstance(source, RequestOut) or not isinstance(target, TargetIn):
            raise TypeError("connect expects a request output and a target input")
        self._connections.append(
            {"source": _endpoint(source), "target": _endpoint(target)}
        )

    def connect_interrupt(self, source: InterruptOut, target: InterruptIn) -> None:
        """Connect a machine timer's interrupt level to a core."""
        if not isinstance(source, InterruptOut) or not isinstance(target, InterruptIn):
            raise TypeError("connect_interrupt expects an interrupt output and input")
        self._interrupt_connections.append(
            {"source": _endpoint(source), "target": _endpoint(target)}
        )

    def build(self) -> Machine:
        """Validate and snapshot the current topology."""
        graph = json.dumps(
            {
                "graph_id": self._graph_id,
                "nodes": self._nodes,
                "connections": self._connections,
                "interrupt_connections": self._interrupt_connections,
            }
        )
        return _Machine(_native.build(graph))


class Machine(Protocol):
    """A validated topology; each run begins from its initial state."""

    def run(self, limit: RunLimit) -> RunResult:
        """Run until a model stops, events run out, or the budget is reached."""
        ...


@dataclass(frozen=True, slots=True)
class _Machine:
    _configuration: _native.Configuration

    def run(self, limit: RunLimit) -> RunResult:
        if not 0 <= limit.events <= (1 << 64) - 1:
            raise _native.ConfigurationError(
                "RunLimit.events must fit an unsigned 64-bit count"
            )
        if not 0 <= limit.ticks <= (1 << 64) - 1:
            raise _native.ConfigurationError(
                "RunLimit.ticks must fit an unsigned 64-bit count"
            )
        payload = self._configuration.run(limit.events, limit.ticks, limit.trace)
        try:
            return decode_result(payload)
        except (KeyError, ValueError, TypeError) as error:
            raise _native.SimulationError("Invalid simulation result") from error
