"""Installed-wheel AM acceptance, including trace invariance and failure cases."""

from dataclasses import replace

from am_benchmark import ROOT, InputSize, build_guest, run_guest, validate
from gem5rs import (
    AddressRange,
    DifftestConfig,
    SimulationError,
    StopReason,
    Tick,
    am_test_platform,
)


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
    print(
        f"AM test: ten self-checks passed, commits={result.stats.committed}, tick={result.final_tick}"
    )


if __name__ == "__main__":
    main()
