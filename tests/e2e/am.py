"""Installed-wheel AM acceptance, including trace invariance and failure cases."""

import shutil
import subprocess
from dataclasses import replace
from pathlib import Path
from tempfile import TemporaryDirectory

from am_benchmark import ROOT, InputSize, build_guest, run_guest, validate
from gem5rs import (
    Address,
    AddressRange,
    BareMetalPlatform,
    Board,
    ConfigurationError,
    DifftestConfig,
    MachineTimer,
    Rv64Core,
    SimulatedTimerClock,
    SimulationError,
    StopReason,
    Tick,
    am_test_platform,
)


def check_timer_abi(source: Path, platform: BareMetalPlatform) -> None:
    # Reuse the actual platform in an isolated finite AM ABI probe.
    with TemporaryDirectory() as directory:
        work = Path(directory)
        (work / "src").mkdir()
        am = work / "abstract-machine"
        shutil.copytree(
            source / "abstract-machine", am, ignore=shutil.ignore_patterns("build")
        )
        for name in ("cte.c", "trap.S", "timer-config.h", "platform.ld"):
            shutil.copyfile(source / name, work / name)
        shutil.copyfile(ROOT / "workloads/am/timer-check.c", work / "thread-os.c")
        subprocess.run(
            [
                "make",
                "-f",
                str(ROOT / "workloads/am/Makefile"),
                "WORKLOAD=thread-os",
                f"AM_HOME={am}",
                f"PLATFORM_LD={work / 'platform.ld'}",
                "image",
            ],
            cwd=work,
            check=True,
        )
        probe = run_guest(
            (work / "build/thread-os-riscv64-gem5rs.elf").read_bytes(),
            platform,
            machine_mode=True,
        )
        assert probe.stop_reason == StopReason.EXIT and probe.exit_code == 0
        assert probe.stats.timer_interrupts >= 3


def main() -> None:
    platform = am_test_platform()
    hello = build_guest(platform, "hello")
    elf = hello.elf.read_bytes()
    trace = run_guest(elf, platform, trace=True)
    summary = run_guest(elf, platform)
    validate(trace, "hello")
    assert summary == replace(trace, transactions=(), commits=())
    assert trace.stats.committed == len(trace.commits)
    assert run_guest(elf, platform) == summary
    reference = DifftestConfig(str(ROOT / "target/spike/adapter"))
    compared = run_guest(elf, platform, difftest=reference)
    assert compared.difftest is not None and compared.difftest.passed
    assert replace(compared, difftest=None) == summary
    try:
        run_guest(elf, platform, difftest=replace(reference, expected_exit_code=1))
    except SimulationError as error:
        assert "expected exit" in str(error)
    else:
        raise AssertionError("unexpected guest exit must fail difftest")
    disabled = run_guest(elf, replace(platform, m5_enabled=False))
    assert disabled.stop_reason == StopReason.FAULT
    assert disabled.fault is not None and "IllegalInstruction" in disabled.fault.reason
    try:
        run_guest(elf, replace(platform, m5_enabled=False), difftest=reference)
    except SimulationError as error:
        assert "fault matched reference" in str(error)
    else:
        raise AssertionError(
            "a matching illegal-instruction trap is not a successful benchmark"
        )
    limited = run_guest(elf, replace(platform, tick_budget=Tick(1)))
    assert limited.stop_reason == StopReason.TICK_LIMIT
    assert limited.stats.committed == 0
    assert not limited.console
    exhausted = run_guest(elf, replace(platform, instruction_budget=1))
    assert exhausted.stop_reason == StopReason.INSTRUCTION_LIMIT
    assert exhausted.fault is None
    assert exhausted.stats.committed == 1

    benchmark = build_guest(platform, "microbench")
    result = run_guest(benchmark.elf.read_bytes(), platform)
    items = validate(result, "microbench")
    compared = run_guest(benchmark.elf.read_bytes(), platform, difftest=reference)
    assert validate(compared, "microbench") == items
    assert replace(compared, difftest=None) == result
    assert compared.difftest is not None and compared.difftest.passed
    stats = compared.difftest
    assert (
        stats.compared + stats.synced_m5 + stats.synced_console + stats.synced_exit
        == result.stats.committed
    )
    assert stats.synced_m5 == 21 and stats.synced_console == len(result.console)
    assert len(items) == 10 and all(region.committed > 0 for region in result.regions)
    assert result.stats.committed > sum(region.committed for region in result.regions)
    # Runtime timing changes must reuse the same ELF and preserve all self-checks.
    slower = run_guest(benchmark.elf.read_bytes(), platform, ram_latency=Tick(2000))
    assert validate(slower, "microbench") == items
    assert tuple(region.committed for region in result.regions) == tuple(
        region.committed for region in slower.regions
    )
    assert result.console != slower.console  # Device timestamps reflect timing.
    assert bytes(byte.value for byte in result.console) == bytes(
        byte.value for byte in slower.console
    )
    try:
        validate(replace(result, regions=result.regions[:-1]), "microbench")
    except ValueError:
        pass
    else:
        raise AssertionError("missing ROI must fail acceptance")
    try:
        validate(replace(result, console=()), "microbench")
    except ValueError:
        pass
    else:
        raise AssertionError("zero exit without per-item validation must fail")
    # Larger inputs must produce distinct guests and remain budget bounded.
    sizes: tuple[InputSize, ...] = ("train", "ref", "huge")
    hashes = {benchmark.elf_sha256}
    for size in sizes:
        larger = (
            replace(platform, ram=AddressRange(platform.ram.start, 128 * 1024 * 1024))
            if size == "huge"
            else platform
        )
        guest = build_guest(larger, "microbench", size)
        assert guest.elf_sha256 not in hashes
        hashes.add(guest.elf_sha256)
        bounded = run_guest(
            guest.elf.read_bytes(), replace(larger, instruction_budget=5000)
        )
        assert bounded.stop_reason == StopReason.INSTRUCTION_LIMIT
        assert bounded.stats.committed == 5000
        assert bounded.fault is None
    try:
        build_guest(platform, "microbench", "huge")
    except ValueError as error:
        assert "required heap" in str(error)
    else:
        raise AssertionError("huge must reject insufficient RAM before simulation")
    os_platform = replace(
        platform, instruction_budget=3_000_000, event_budget=24_000_000
    )
    os_guest = build_guest(os_platform, "yield-os")
    os_result = run_guest(os_guest.elf.read_bytes(), os_platform, machine_mode=True)
    assert validate(os_result, "yield-os") == [
        {"name": "yield-os", "status": "bounded-progress"}
    ]
    assert os_result.stats.committed == os_platform.instruction_budget
    disabled = run_guest(os_guest.elf.read_bytes(), os_platform)
    assert disabled.stop_reason == StopReason.FAULT
    assert disabled.fault is not None and "IllegalInstruction" in disabled.fault.reason
    small = replace(os_platform, instruction_budget=2000)
    traced = run_guest(os_guest.elf.read_bytes(), small, machine_mode=True, trace=True)
    summary_os = run_guest(os_guest.elf.read_bytes(), small, machine_mode=True)
    assert summary_os == replace(traced, transactions=(), commits=())
    for incomplete in (
        replace(os_result, console=os_result.console[:1]),
        replace(os_result, stop_reason=StopReason.TICK_LIMIT),
    ):
        try:
            validate(incomplete, "yield-os")
        except ValueError:
            pass
        else:
            raise AssertionError(
                "OS progress requires repeated alternation and the expected stop"
            )
    try:
        run_guest(
            os_guest.elf.read_bytes(), small, machine_mode=True, difftest=reference
        )
    except ConfigurationError as error:
        assert "machine_mode" in str(error)
    else:
        raise AssertionError("unsupported OS difftest must reject before execution")
    thread_platform = replace(
        platform,
        timer=AddressRange(Address(0x0200_0000), 16),
        instruction_budget=6_000_000,
        event_budget=48_000_000,
        tick_budget=Tick(20_000_000_000),
    )
    threads = build_guest(thread_platform, "thread-os")
    thread_result = run_guest(
        threads.elf.read_bytes(), thread_platform, machine_mode=True
    )
    assert len(validate(thread_result, "thread-os")) == 5
    assert thread_result.stats.timer_interrupts >= 5
    assert (
        run_guest(threads.elf.read_bytes(), thread_platform, machine_mode=True)
        == thread_result
    )
    # Clock source is hardware configuration, not a change to the guest binary.
    assert (
        build_guest(
            replace(thread_platform, timer_clock="wall"), "thread-os"
        ).elf_sha256
        == threads.elf_sha256
    )
    quick = replace(thread_platform, instruction_budget=100_000, timer_interval_us=10)
    quick_guest = build_guest(quick, "thread-os")
    check_timer_abi(quick_guest.elf.parent.parent, quick)
    traced_timer = run_guest(
        quick_guest.elf.read_bytes(), quick, machine_mode=True, trace=True
    )
    assert traced_timer.stats.timer_interrupts > 0
    assert run_guest(quick_guest.elf.read_bytes(), quick, machine_mode=True) == replace(
        traced_timer, transactions=(), commits=()
    )
    try:
        validate(
            replace(
                thread_result, stats=replace(thread_result.stats, timer_interrupts=0)
            ),
            "thread-os",
        )
    except ValueError:
        pass
    else:
        raise AssertionError("thread progress requires timer interrupts")
    # IRQ configuration errors must fail before trying to run a guest.
    for connected, enabled, foreign in (
        (False, True, False),
        (True, False, False),
        (True, True, True),
    ):
        board = Board()
        core_board = Board() if foreign else board
        core = core_board.add(
            Rv64Core(
                threads.elf.read_bytes(),
                thread_platform.stack_top,
                machine_mode=enabled,
            )
        )
        timer = board.add(
            MachineTimer(
                Address(0x0200_0000), SimulatedTimerClock(Tick(1000)), Tick(1000)
            )
        )
        if connected:
            board.connect_interrupt(timer.interrupt, core.timer_interrupt)
        try:
            board.build()
        except ConfigurationError as error:
            assert "interrupt" in str(error)
        else:
            raise AssertionError("invalid timer IRQ wiring accepted")
    print("thread-os: five-thread timer preemption and deterministic replay passed")
    print("yield-os: repeated A/B context switches passed")
    print(
        f"AM test: ten self-checks passed, commits={result.stats.committed}, tick={result.final_tick}"
    )


if __name__ == "__main__":
    main()
