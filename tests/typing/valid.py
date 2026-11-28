from typing import assert_type

from gem5rs import (
    Address,
    AddressRange,
    Board,
    Commit,
    CoreHandle,
    DifftestStatistics,
    DualBankRam,
    Exit,
    Fault,
    FixedRam,
    MachineTimer,
    Read,
    RequestorHandle,
    Router,
    RouterHandle,
    RunLimit,
    Rv64Core,
    SimpleUart,
    SimulatedTimerClock,
    Statistics,
    StopReason,
    TargetHandle,
    Tick,
    TimerHandle,
    Traffic,
    Transaction,
    WallTimerClock,
    Write,
)


def experiment() -> tuple[int, ...]:
    board = Board()
    src = board.add(Traffic([Write(at=Tick(0), addr=Address(0), data=b"A")]))
    ram = board.add(FixedRam(size_bytes=64, latency=Tick(10), capacity=1))
    board.connect(src.requests, ram.input)
    result = board.build().run(RunLimit(events=100))
    return tuple(int(tx.completed_at) for tx in result.transactions)


def console_assembly() -> None:
    board = Board()
    source = board.add(Traffic([Write(Tick(0), Address(4096), b"A")]))
    console = board.add(SimpleUart(Address(4096), 4096, Tick(3)))
    assert_type(console, TargetHandle)
    board.connect(source.requests, console.input)


def assemble(elf: bytes) -> None:
    board = Board()
    traffic = board.add(Traffic([Read(Tick(0), Address(0), 1)]))
    core = board.add(Rv64Core(elf=elf, stack_pointer=Address(64), machine_mode=True))
    ram = board.add(DualBankRam(64, Tick(10), 8))
    exit_port = board.add(Exit(Address(64)))
    router = board.add(
        Router(
            2,
            [AddressRange(Address(0), 64), AddressRange(Address(64), 8)],
            priority=[1, 0],
        )
    )
    assert_type(traffic, RequestorHandle)
    assert_type(core, CoreHandle)
    assert_type(ram, TargetHandle)
    assert_type(exit_port, TargetHandle)
    assert_type(router, RouterHandle)
    board.connect(traffic.requests, router.inputs[0])
    board.connect(core.requests, router.inputs[1])
    board.connect(router.outputs[0], ram.input)
    board.connect(router.outputs[1], exit_port.input)
    result = board.build().run(RunLimit(events=100))
    assert_type(result.stop_reason, StopReason)
    assert_type(result.final_tick, Tick)
    assert_type(result.exit_code, int | None)
    assert_type(result.fault, Fault | None)
    assert_type(result.transactions, tuple[Transaction, ...])
    assert_type(result.transactions[0].data, bytes | None)
    assert_type(result.commits, tuple[Commit, ...])
    assert_type(result.commits[0].pc, Address)
    assert_type(result.stats, Statistics)
    assert_type(result.difftest, DifftestStatistics | None)


def timer_assembly(elf: bytes) -> None:
    board = Board()
    core = board.add(Rv64Core(elf, Address(0x80010000), machine_mode=True))
    timer = board.add(
        MachineTimer(Address(0x2000000), SimulatedTimerClock(Tick(1000)), Tick(10))
    )
    assert_type(timer, TimerHandle)
    ram = board.add(FixedRam(65536, Tick(1), 1))
    router = board.add(
        Router(
            1,
            [
                AddressRange(Address(0x80000000), 65536),
                AddressRange(Address(0x2000000), 16),
            ],
        )
    )
    board.connect(core.requests, router.inputs[0])
    board.connect(router.outputs[0], ram.input)
    board.connect(router.outputs[1], timer.input)
    board.connect_interrupt(timer.interrupt, core.timer_interrupt)
    # Wall time is an alternative clock configuration on the same device/ports.
    MachineTimer(Address(0x2000000), WallTimerClock(frequency_hz=1_000_000), Tick(10))
