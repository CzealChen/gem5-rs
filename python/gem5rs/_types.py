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
class Rv64Core:
    elf: bytes
    stack_pointer: Address
    cycle: Tick = Tick(1)


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


class StopReason(StrEnum):
    IDLE = "idle"
    EVENT_LIMIT = "event_limit"
    EXIT = "exit"
    FAULT = "fault"


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
