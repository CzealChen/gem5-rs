from typing import assert_type

from gem5rs import (
    Address,
    AddressRange,
    Board,
    Commit,
    DualBankRam,
    Exit,
    Fault,
    FixedRam,
    Read,
    RequestorHandle,
    Router,
    RouterHandle,
    RunLimit,
    Rv64Core,
    Statistics,
    StopReason,
    TargetHandle,
    Tick,
    Traffic,
    Transaction,
    Write,
)


def experiment() -> tuple[int, ...]:
    board = Board()
    src = board.add(Traffic([Write(at=Tick(0), addr=Address(0), data=b"A")]))
    ram = board.add(FixedRam(size_bytes=64, latency=Tick(10), capacity=1))
    board.connect(src.requests, ram.input)
    result = board.build().run(RunLimit(events=100))
    return tuple(int(tx.completed_at) for tx in result.transactions)


def assemble(elf: bytes) -> None:
    board = Board()
    traffic = board.add(Traffic([Read(Tick(0), Address(0), 1)]))
    core = board.add(Rv64Core(elf=elf, stack_pointer=Address(64)))
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
    assert_type(core, RequestorHandle)
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
