"""Installed-wheel AM acceptance, including trace invariance and failure cases."""

from dataclasses import replace

from am_benchmark import build_guest, run_guest, validate
from gem5rs import StopReason, Tick, am_test_platform


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
    disabled = run_guest(elf, replace(platform, m5_enabled=False))
    assert disabled.stop_reason == StopReason.FAULT
    assert disabled.fault is not None and "IllegalInstruction" in disabled.fault.reason
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
    assert len(items) == 4 and all(region.committed > 0 for region in result.regions)
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
    print(
        f"AM test: four self-checks passed, commits={result.stats.committed}, tick={result.final_tick}"
    )


if __name__ == "__main__":
    main()
