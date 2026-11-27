"""Exercise the public board interface with typed traffic sources."""

from collections.abc import Callable

from gem5rs import (
    AccessKind,
    Address,
    AddressRange,
    Board,
    ConfigurationError,
    DualBankRam,
    FixedRam,
    Machine,
    Read,
    Router,
    RunLimit,
    RunResult,
    Rv64Core,
    SimulationError,
    StopReason,
    Tick,
    Traffic,
    Write,
)
from valid import experiment

LIMIT = RunLimit(events=100)

type TraceRow = tuple[str, AccessKind, Address, Tick, Tick, bytes | None]


def trace(result: RunResult) -> tuple[TraceRow, ...]:
    return tuple(
        (tx.source, tx.kind, tx.addr, tx.accepted_at, tx.completed_at, tx.data)
        for tx in result.transactions
    )


def same_location(*, reverse: bool) -> Machine:
    board = Board()
    models = {
        "A": Traffic([Write(Tick(0), Address(0), b"A")]),
        "B": Traffic([Read(Tick(0), Address(0), 1)]),
    }
    order = ("A", "B") if reverse else ("B", "A")
    handles = {name: board.add(models[name], name=name) for name in order}
    router = board.add(Router(2, [AddressRange(Address(0), 64)], priority=[1, 0]))
    ram = board.add(FixedRam(64, Tick(10), 1))
    board.connect(handles["B"].requests, router.inputs[0])
    board.connect(handles["A"].requests, router.inputs[1])
    board.connect(router.outputs[0], ram.input)
    return board.build()


def different_banks(model: FixedRam | DualBankRam) -> Machine:
    board = Board()
    a = board.add(
        Traffic([Write(Tick(0), Address(0), b"A"), Read(Tick(21), Address(0), 1)]),
        name="A",
    )
    b = board.add(
        Traffic([Write(Tick(0), Address(8), b"B"), Read(Tick(21), Address(8), 1)]),
        name="B",
    )
    router = board.add(Router(2, [AddressRange(Address(0), 64)]))
    ram = board.add(model)
    board.connect(a.requests, router.inputs[0])
    board.connect(b.requests, router.inputs[1])
    board.connect(router.outputs[0], ram.input)
    return board.build()


def expect_graph_error(build: Callable[[], Machine], *labels: str) -> None:
    try:
        build()
    except ConfigurationError as error:
        assert all(label in str(error) for label in labels), (labels, str(error))
    else:
        raise AssertionError(f"Expected a graph error naming {labels}")


def check_graphs() -> None:
    board, other = Board(), Board()
    local = board.add(Traffic([]), name="local_cpu")
    foreign = other.add(Traffic([]), name="local_cpu")
    ram = board.add(FixedRam(32, Tick(10), 1), name="local_ram")
    board.connect(local.requests, ram.input)
    board.connect(foreign.requests, ram.input)
    expect_graph_error(board.build, "graph", "local_cpu", "requests")

    board = Board()
    src = board.add(Traffic([]), name="duplicate_src")
    first = board.add(FixedRam(32, Tick(10), 1), name="first_ram")
    second = board.add(FixedRam(32, Tick(10), 1), name="second_ram")
    board.connect(src.requests, first.input)
    board.connect(src.requests, second.input)
    expect_graph_error(board.build, "duplicate_src", "requests", "already connected")

    board = Board()
    left = board.add(Traffic([]), name="left")
    right = board.add(Traffic([]), name="right")
    ram = board.add(FixedRam(32, Tick(10), 1), name="duplicate_ram")
    board.connect(left.requests, ram.input)
    board.connect(right.requests, ram.input)
    expect_graph_error(board.build, "duplicate_ram", "input", "already connected")

    board = Board()
    board.add(Traffic([]), name="unbound_src")
    expect_graph_error(board.build, "unbound_src", "requests", "unconnected")

    board = Board()
    src = board.add(Traffic([]))
    router = board.add(
        Router(1, [AddressRange(Address(0), 32), AddressRange(Address(16), 32)]),
        name="overlap_router",
    )
    first = board.add(FixedRam(32, Tick(10), 1))
    second = board.add(FixedRam(32, Tick(10), 1))
    board.connect(src.requests, router.inputs[0])
    board.connect(router.outputs[0], first.input)
    board.connect(router.outputs[1], second.input)
    expect_graph_error(
        board.build, "overlap_router", "outputs[0]", "outputs[1]", " overlaps "
    )


def check_initial_state() -> None:
    board = Board()
    probe = board.add(
        Traffic(
            [
                Read(Tick(0), Address(4), 1),
                Write(Tick(4), Address(4), b"Z"),
                Read(Tick(8), Address(4), 1),
            ]
        )
    )
    ram = board.add(FixedRam(64, Tick(1), 1))
    board.connect(probe.requests, ram.input)
    machine = board.build()
    for _ in range(2):
        result = machine.run(LIMIT)
        assert tuple(tx.data for tx in result.transactions) == (b"\0", None, b"Z")
        assert (result.stop_reason, result.final_tick) == (StopReason.IDLE, Tick(9))


def check_causes() -> None:
    board = Board()
    core = board.add(Rv64Core(b"short", Address(32), Tick(1)), name="boot")
    ram = board.add(FixedRam(64, Tick(1), 1), name="sram")
    board.connect(core.requests, ram.input)
    try:
        board.build()
    except ConfigurationError as error:
        assert "boot.requests" in str(error)
        cause = error.__cause__
        assert cause is not None and "sram.input" in str(cause)
        assert cause.__cause__ is not None and "ELF64 header" in str(cause.__cause__)
        reader = cause.__cause__.__cause__
        assert reader is not None and "Invalid ELF header" in str(reader)
    else:
        raise AssertionError("Expected a truncated ELF header diagnostic")

    board = Board()
    probe = board.add(Traffic([Read(Tick((1 << 64) - 1), Address(0), 1)]))
    ram = board.add(FixedRam(64, Tick(1), 1))
    board.connect(probe.requests, ram.input)
    machine = board.build()
    try:
        machine.run(LIMIT)
    except SimulationError as runtime_error:
        assert "event queue" in str(runtime_error)
        assert "completion tick overflow" in str(runtime_error.__cause__)
    else:
        raise AssertionError("Expected an overflowing event completion diagnostic")


def check_traffic() -> None:
    assert experiment() == (10,), (
        "Single direct write completes after its 10-tick service"
    )

    expected = (
        ("A", AccessKind.WRITE, Address(0), Tick(0), Tick(10), None),
        ("B", AccessKind.READ, Address(0), Tick(10), Tick(20), b"A"),
    )
    for reverse in (False, True):
        machine = same_location(reverse=reverse)
        run = machine.run(LIMIT)
        assert run == machine.run(LIMIT), "Dual-source replay changed the run result"
        assert (run.stop_reason, run.final_tick) == (StopReason.IDLE, Tick(20)), run
        assert trace(run) == expected, trace(run)
        stats = run.stats
        assert (
            stats.accepted,
            stats.rejected,
            stats.retries,
            stats.reads,
            stats.writes,
        ) == (2, 1, 1, 1, 1), stats

    for model, tick, times in (
        (FixedRam(64, Tick(10), 1), Tick(41), {"A": (10, 31), "B": (20, 41)}),
        (DualBankRam(64, Tick(10), 8), Tick(31), {"A": (10, 31), "B": (10, 31)}),
    ):
        machine = different_banks(model)
        run = machine.run(LIMIT)
        assert run == machine.run(LIMIT), "Different-bank replay changed the run result"
        assert (run.stop_reason, run.final_tick) == (StopReason.IDLE, tick), run
        actual = sorted(
            (tx.source, tx.kind, tx.completed_at, tx.data) for tx in run.transactions
        )
        expected_banks = sorted(
            [
                ("A", AccessKind.WRITE, times["A"][0], None),
                ("A", AccessKind.READ, times["A"][1], b"A"),
                ("B", AccessKind.WRITE, times["B"][0], None),
                ("B", AccessKind.READ, times["B"][1], b"B"),
            ]
        )
        assert actual == expected_banks, actual
        print(
            f"{type(model).__name__}: {run.stop_reason}, tick={run.final_tick}, reads=A/B"
        )

    stopped = same_location(reverse=False).run(RunLimit(events=0))
    assert (stopped.stop_reason, stopped.final_tick, stopped.stats.events) == (
        StopReason.EVENT_LIMIT,
        Tick(0),
        0,
    ), stopped
    print(
        "Traffic: single write completed at tick 10; contention completed at ticks 10 and 20"
    )


if __name__ == "__main__":
    check_traffic()
    check_graphs()
    check_initial_state()
    check_causes()
    print(
        "Graph diagnostics: cross-board handles, duplicate bindings, missing output, overlapping ranges"
    )
