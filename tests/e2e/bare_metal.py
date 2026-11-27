"""Compare both installed-wheel ELF runs with the real Spike interpreter."""

import argparse
import struct
from pathlib import Path

from gem5rs import (
    AccessKind,
    Address,
    AddressRange,
    Board,
    Exit,
    FixedRam,
    Machine,
    Router,
    RunLimit,
    RunResult,
    Rv64Core,
    SimulationError,
    StopReason,
    Tick,
)
from spike_trace import (
    EXIT_ADDRESS,
    RAM_BASE,
    RAM_SIZE,
    Commit,
    ReferenceTrace,
    compare_trace,
    run_spike,
)

LIMIT = RunLimit(events=10_000)
CYCLE = Tick(2)
EXIT_LATENCY = Tick(1)


def build(elf: bytes, latency: Tick, cycle: Tick = CYCLE) -> Machine:
    board = Board()
    cpu = board.add(
        Rv64Core(elf, Address(RAM_BASE + RAM_SIZE - 16), cycle), name="core"
    )
    ram = board.add(FixedRam(RAM_SIZE, latency, capacity=1), name="ram")
    exit_port = board.add(Exit(Address(EXIT_ADDRESS), width_bytes=4), name="exit")
    router = board.add(
        Router(
            1,
            [
                AddressRange(Address(RAM_BASE), RAM_SIZE),
                AddressRange(Address(EXIT_ADDRESS), 4),
            ],
        )
    )
    board.connect(cpu.requests, router.inputs[0])
    board.connect(router.outputs[0], ram.input)
    board.connect(router.outputs[1], exit_port.input)
    return board.build()


def architectural_trace(run: RunResult) -> tuple[Commit, ...]:
    return tuple(
        Commit(
            commit.pc,
            commit.instruction,
            None
            if commit.writeback is None
            else (commit.writeback.register, commit.writeback.value),
        )
        for commit in run.commits
    )


def check_timing(run: RunResult, ref: ReferenceTrace, latency: Tick) -> int:
    # Reference instruction words identify the data operations independently of
    # the simulator's counters. Each instruction pays fetch + cycle, and loads
    # and stores pay one extra service. An exit store pays the MMIO service.
    tick = reads = writes = 0
    ref_commits = (*ref.commits, ref.exit_commit)
    transactions = iter(run.transactions)
    for index, (expected, actual) in enumerate(
        zip(ref_commits, run.commits, strict=True)
    ):
        fetch = next(transactions, None)
        assert fetch is not None, f"commit[{index}]: expected fetch"
        assert (fetch.kind, fetch.addr, fetch.size_bytes) == (
            AccessKind.FETCH,
            expected.pc,
            4,
        ), (index, fetch)
        assert (fetch.accepted_at, fetch.completed_at) == (tick, tick + latency), (
            index,
            fetch,
        )
        assert (fetch.data, fetch.fault) == (
            expected.instruction.to_bytes(4, "little"),
            None,
        ), (index, fetch)
        tick += latency + CYCLE
        opcode = expected.instruction & 0x7F
        if opcode in (0x03, 0x23):
            kind = AccessKind.READ if opcode == 0x03 else AccessKind.WRITE
            reads += kind is AccessKind.READ
            writes += kind is AccessKind.WRITE
            service = EXIT_LATENCY if index == len(ref.commits) else latency
            data = next(transactions, None)
            assert data is not None, f"commit[{index}]: expected {kind}"
            width = 1 << ((expected.instruction >> 12) & 3)
            assert (
                data.kind,
                data.size_bytes,
                data.accepted_at,
                data.completed_at,
                data.fault,
            ) == (kind, width, tick, tick + service, None), (index, data)
            if index == len(ref.commits):
                assert (data.addr, data.data) == (EXIT_ADDRESS, None), data
            else:
                assert RAM_BASE <= data.addr < RAM_BASE + RAM_SIZE, data
            tick += service
        assert actual.at == tick, (index, actual.at, tick)

    assert next(transactions, None) is None, (
        "Unexpected response after the terminal store"
    )
    assert run.final_tick == tick, (run.final_tick, tick)
    stats = run.stats
    count = len(ref_commits)
    assert (stats.fetches, stats.reads, stats.writes, stats.committed) == (
        count,
        reads,
        writes,
        count,
    ), stats
    assert (stats.accepted, stats.rejected, stats.retries) == (
        count + reads + writes,
        0,
        0,
    ), stats
    return count + reads + writes - 1


def check_cpu_errors() -> None:
    # One executable PT_LOAD containing an illegal instruction at the entry.
    header = struct.pack(
        "<16sHHIQQQIHHHHHH",
        b"\x7fELF\x02\x01\x01",
        2,
        243,
        1,
        RAM_BASE,
        64,
        0,
        0,
        64,
        56,
        1,
        0,
        0,
        0,
    )
    segment = struct.pack("<IIQQQQQQ", 1, 5, 120, RAM_BASE, RAM_BASE, 4, 4, 4)
    elf = header + segment + b"\xff" * 4
    run = build(elf, Tick(3)).run(LIMIT)
    assert run.stop_reason == StopReason.FAULT and run.fault is not None
    assert (run.fault.pc, run.fault.reason, run.final_tick) == (
        RAM_BASE,
        "IllegalInstruction(4294967295)",
        5,
    )
    try:
        build(elf, Tick(3), Tick((1 << 64) - 1)).run(LIMIT)
    except SimulationError as error:
        assert "execution tick overflow: 3 + 18446744073709551615" in str(error)
    else:
        raise AssertionError("Expected an overflowing CPU execution tick diagnostic")


def check_elf(path: Path, expected_ticks: tuple[int, int]) -> None:
    reference = run_spike(path)
    elf = path.read_bytes()
    runs: list[RunResult] = []
    for latency in (Tick(3), Tick(7)):
        machine = build(elf, latency)
        run = machine.run(LIMIT)
        runs.append(run)
        diff = compare_trace(reference, architectural_trace(run))
        stats = run.stats
        print(
            f"{path.name} (RAM latency={latency}): {run.stop_reason}, code={run.exit_code}, "
            f"commits={len(run.commits)}, tick={run.final_tick}, fetches={stats.fetches}, "
            f"reads={stats.reads}, writes={stats.writes}, accepted={stats.accepted}, "
            f"rejected={stats.rejected}, retries={stats.retries}; Spike: {diff or 'match'}",
            flush=True,
        )
        assert diff is None, diff
        assert (run.stop_reason, run.exit_code, run.fault) == (
            StopReason.EXIT,
            0,
            None,
        ), (run.stop_reason, run.exit_code, run.fault)
        assert run.exit_code == reference.exit_code, reference.exit_code
        ram_accesses = check_timing(run, reference, latency)
        assert run == machine.run(LIMIT), f"{path.name} replay changed the run result"

    fast, slow = runs
    assert (fast.final_tick, slow.final_tick) == expected_ticks, path.name
    assert architectural_trace(fast) == architectural_trace(slow), (
        "Changing RAM latency changed commits"
    )
    fast_responses = tuple((tx.addr, tx.kind, tx.data) for tx in fast.transactions)
    slow_responses = tuple((tx.addr, tx.kind, tx.data) for tx in slow.transactions)
    assert fast_responses == slow_responses, "Changing RAM latency changed responses"
    assert slow.final_tick - fast.final_tick == (7 - 3) * ram_accesses


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("elf_dir", type=Path)
    directory: object = parser.parse_args().elf_dir
    assert isinstance(directory, Path)
    check_cpu_errors()
    check_elf(directory / "rv64i.elf", (416, 772))
    check_elf(directory / "loop.elf", (742, 1402))


if __name__ == "__main__":
    main()
