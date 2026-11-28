from __future__ import annotations

from collections.abc import Sequence
from dataclasses import dataclass
from enum import StrEnum
from typing import NewType

Address = NewType("Address", int)
Tick = NewType("Tick", int)


@dataclass(frozen=True, slots=True)
class Read:
    at: Tick
    addr: Address
    size_bytes: int


@dataclass(frozen=True, slots=True)
class Write:
    at: Tick
    addr: Address
    data: bytes


@dataclass(frozen=True, slots=True)
class Traffic:
    operations: Sequence[Read | Write]


@dataclass(frozen=True, slots=True)
class FixedRam:
    size_bytes: int
    latency: Tick
    capacity: int


@dataclass(frozen=True, slots=True)
class DualBankRam:
    size_bytes: int
    latency: Tick
    bank_bytes: int


@dataclass(frozen=True, slots=True)
class Exit:
    addr: Address
    width_bytes: int = 8


@dataclass(frozen=True, slots=True)
class SimpleUart:
    addr: Address
    size_bytes: int
    latency: Tick


@dataclass(frozen=True, slots=True)
class ExperimentConfig:
    tick_femtoseconds: int
    instruction_budget: int


@dataclass(frozen=True, slots=True)
class DifftestConfig:
    adapter: str
    timeout_ms: int = 5000
    expected_exit_code: int = 0
    expected_failure: bool = False


@dataclass(frozen=True, slots=True)
class Rv64Core:
    elf: bytes
    stack_pointer: Address
    cycle: Tick = Tick(1)
    experiment: ExperimentConfig | None = None
    stack_bytes: int = 16
    difftest: DifftestConfig | None = None


@dataclass(frozen=True, slots=True)
class AddressRange:
    start: Address
    size_bytes: int


@dataclass(frozen=True, slots=True)
class Router:
    inputs: int
    ranges: Sequence[AddressRange]
    priority: Sequence[int] | None = None


@dataclass(frozen=True, slots=True)
class RequestOut:
    _graph_id: int
    _node: int
    _port: int


@dataclass(frozen=True, slots=True)
class TargetIn:
    _graph_id: int
    _node: int
    _port: int


@dataclass(frozen=True, slots=True)
class RequestorHandle:
    requests: RequestOut


@dataclass(frozen=True, slots=True)
class TargetHandle:
    input: TargetIn


@dataclass(frozen=True, slots=True)
class RouterHandle:
    inputs: tuple[TargetIn, ...]
    outputs: tuple[RequestOut, ...]


@dataclass(frozen=True, slots=True)
class RunLimit:
    events: int
    ticks: Tick = Tick((1 << 64) - 1)
    trace: bool = True


class StopReason(StrEnum):
    IDLE = "idle"
    EVENT_LIMIT = "event_limit"
    TICK_LIMIT = "tick_limit"
    INSTRUCTION_LIMIT = "instruction_limit"
    EXIT = "exit"
    FAULT = "fault"
    FAIL = "fail"


class AccessKind(StrEnum):
    FETCH = "fetch"
    READ = "read"
    WRITE = "write"


@dataclass(frozen=True, slots=True)
class Fault:
    reason: str
    pc: Address | None
    addr: Address | None


@dataclass(frozen=True, slots=True)
class Transaction:
    id: int
    source: str
    addr: Address
    kind: AccessKind
    size_bytes: int
    accepted_at: Tick
    completed_at: Tick
    data: bytes | None
    fault: str | None


@dataclass(frozen=True, slots=True)
class RegisterWrite:
    register: int
    value: int


@dataclass(frozen=True, slots=True)
class Commit:
    at: Tick
    pc: Address
    instruction: int
    writeback: RegisterWrite | None


@dataclass(frozen=True, slots=True)
class Statistics:
    events: int
    accepted: int
    rejected: int
    retries: int
    fetches: int
    reads: int
    writes: int
    committed: int


@dataclass(frozen=True, slots=True)
class RunResult:
    stop_reason: StopReason
    final_tick: Tick
    exit_code: int | None
    fault: Fault | None
    transactions: tuple[Transaction, ...]
    commits: tuple[Commit, ...]
    stats: Statistics
    console: tuple[ConsoleByte, ...] = ()
    regions: tuple[Region, ...] = ()
    difftest: DifftestStatistics | None = None


@dataclass(frozen=True, slots=True)
class DifftestStatistics:
    protocol_version: int
    reference_commit: str
    compared: int
    synced_m5: int
    synced_console: int
    synced_exit: int
    passed: bool


@dataclass(frozen=True, slots=True)
class ConsoleByte:
    at: Tick
    value: int


@dataclass(frozen=True, slots=True)
class Region:
    id: int
    begin_tick: Tick
    end_tick: Tick
    committed: int
