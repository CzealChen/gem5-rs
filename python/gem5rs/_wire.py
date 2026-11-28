import json
from collections.abc import Mapping, Sequence

from ._types import (
    AccessKind,
    Address,
    Commit,
    ConsoleByte,
    Fault,
    Region,
    RegisterWrite,
    RunResult,
    Statistics,
    StopReason,
    Tick,
    Transaction,
)


def _record(value: object) -> Mapping[object, object]:
    if not isinstance(value, dict):
        raise ValueError("Native result: expected an object")
    return value


def _items(value: object) -> Sequence[object]:
    if not isinstance(value, list):
        raise ValueError("Native result: expected an array")
    return value


def _uint(value: object, maximum: int = (1 << 64) - 1) -> int:
    if (
        not isinstance(value, int)
        or isinstance(value, bool)
        or not 0 <= value <= maximum
    ):
        raise ValueError(f"Native result: expected an integer between 0 and {maximum}")
    return value


def _text(value: object) -> str:
    if not isinstance(value, str):
        raise ValueError("Native result: expected a string")
    return value


def _address(value: object) -> Address | None:
    return None if value is None else Address(_uint(value))


def _fault(value: object) -> Fault | None:
    if value is None:
        return None
    fault = _record(value)
    return Fault(
        reason=_text(fault["reason"]),
        pc=_address(fault["pc"]),
        addr=_address(fault["addr"]),
    )


def _transaction(value: object) -> Transaction:
    tx = _record(value)
    data = tx["data"]
    fault = tx["fault"]
    return Transaction(
        id=_uint(tx["id"]),
        source=_text(tx["source"]),
        addr=Address(_uint(tx["addr"])),
        kind=AccessKind(_text(tx["kind"])),
        size_bytes=_uint(tx["size_bytes"]),
        accepted_at=Tick(_uint(tx["accepted_at"])),
        completed_at=Tick(_uint(tx["completed_at"])),
        data=None if data is None else bytes(_uint(byte, 255) for byte in _items(data)),
        fault=None if fault is None else _text(fault),
    )


def _writeback(value: object) -> RegisterWrite | None:
    if value is None:
        return None
    write = _record(value)
    return RegisterWrite(
        register=_uint(write["register"], 31), value=_uint(write["value"])
    )


def _commit(value: object) -> Commit:
    step = _record(value)
    return Commit(
        at=Tick(_uint(step["at"])),
        pc=Address(_uint(step["pc"])),
        instruction=_uint(step["instruction"], (1 << 32) - 1),
        writeback=_writeback(step["writeback"]),
    )


def _statistics(value: object) -> Statistics:
    stats = _record(value)
    return Statistics(
        events=_uint(stats["events"]),
        accepted=_uint(stats["accepted"]),
        rejected=_uint(stats["rejected"]),
        retries=_uint(stats["retries"]),
        fetches=_uint(stats["fetches"]),
        reads=_uint(stats["reads"]),
        writes=_uint(stats["writes"]),
        committed=_uint(stats["committed"]),
    )


def decode_result(payload: str) -> RunResult:
    decoded: object = json.loads(payload)
    result = _record(decoded)
    exit_code = result["exit_code"]
    return RunResult(
        stop_reason=StopReason(_text(result["stop_reason"])),
        final_tick=Tick(_uint(result["final_tick"])),
        exit_code=None if exit_code is None else _uint(exit_code),
        fault=_fault(result["fault"]),
        transactions=tuple(_transaction(tx) for tx in _items(result["transactions"])),
        commits=tuple(_commit(step) for step in _items(result["commits"])),
        stats=_statistics(result["stats"]),
        console=tuple(_console(byte) for byte in _items(result.get("console", []))),
        regions=tuple(_region(region) for region in _items(result.get("regions", []))),
    )


def _console(value: object) -> ConsoleByte:
    byte = _record(value)
    return ConsoleByte(Tick(_uint(byte["at"])), _uint(byte["value"], 255))


def _region(value: object) -> Region:
    region = _record(value)
    return Region(
        _uint(region["id"]),
        Tick(_uint(region["begin_tick"])),
        Tick(_uint(region["end_tick"])),
        _uint(region["committed"]),
    )
