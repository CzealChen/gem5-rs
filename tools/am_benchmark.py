"""Build AM guests with Make and run them on the current blocking RV64 core."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import time
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Literal

from gem5rs import (
    Address,
    BareMetalPlatform,
    Board,
    DifftestConfig,
    ExperimentConfig,
    FixedRam,
    Router,
    RunLimit,
    RunResult,
    Rv64Core,
    SimpleUart,
    SimulationError,
    StopReason,
    Tick,
    am_test_platform,
)

ROOT = Path(
    os.environ.get("GEM5RS_REPO_ROOT", str(Path(__file__).resolve().parents[1]))
)
EXPECTED = ("qsort", "queen", "bf", "fib", "sieve", "15pz", "dinic", "lzip", "ssort", "md5")
DEFAULT_RAM_LATENCY = Tick(1000)
type Workload = Literal["hello", "microbench"]


@dataclass(frozen=True)
class GuestBuild:
    elf: Path
    key: str
    compiler: str
    elf_sha256: str


def build_guest(platform: BareMetalPlatform, workload: Workload) -> GuestBuild:
    """Content-address sources, overlay, layout and toolchain; let Make compile."""
    compiler = subprocess.check_output(
        ["riscv64-none-elf-gcc", "--version"], text=True
    ).strip()
    inputs = sorted(
        path
        for parent in (ROOT / "third_party", ROOT / "workloads/am")
        for path in parent.rglob("*")
        if path.is_file()
    )
    digest = hashlib.sha256()
    for path in inputs:
        digest.update(
            str(path.relative_to(ROOT)).encode() + b"\0" + path.read_bytes() + b"\0"
        )
    digest.update(Path(__file__).read_bytes())
    digest.update((ROOT / "devenv.lock").read_bytes())
    digest.update(compiler.encode())
    digest.update(platform.linker_script().encode() + workload.encode())
    key = digest.hexdigest()
    work = ROOT / "target/am" / key
    elf = work / "build" / f"{workload}-riscv64-gem5rs.elf"
    marker = work / "complete.json"
    if not marker.exists():
        # A failed build never becomes a cache hit. Do not reuse partial object files.
        if work.exists():
            shutil.rmtree(work)
        work.mkdir(parents=True)
        am = work / "abstract-machine"
        shutil.copytree(ROOT / "third_party/abstract-machine", am)
        (am / "scripts").mkdir()
        shutil.copyfile(
            ROOT / "workloads/am/riscv64-gem5rs.mk", am / "scripts/riscv64-gem5rs.mk"
        )
        runtime = am / "am/src/platform/gem5rs"
        runtime.mkdir(parents=True)
        for name in ("start.S", "runtime.c", "m5.S"):
            shutil.copyfile(ROOT / "workloads/am" / name, runtime / name)
        klib = am / "klib/src"
        klib.mkdir()
        shutil.copyfile(ROOT / "workloads/am/klib.c", klib / "klib.c")
        shutil.copyfile(ROOT / "workloads/am/softarith.c", klib / "softarith.c")
        upstream = ROOT / "third_party/am-kernels/microbench"
        shutil.copytree(upstream / "include", work / "include")
        shutil.copyfile(upstream / "src/bench.c", work / "upstream_bench.c")
        shutil.copytree(upstream / "src", work / "src")
        for name in ("runner.c", "hello.c"):
            shutil.copyfile(ROOT / "workloads/am" / name, work / name)
        script = work / "platform.ld"
        script.write_text(platform.linker_script())
        env = os.environ.copy()
        for name in (
            "CFLAGS",
            "CPPFLAGS",
            "CXXFLAGS",
            "ASFLAGS",
            "LDFLAGS",
            "MAKEFLAGS",
            "LIBS",
            "CROSS_COMPILE",
        ):
            env.pop(name, None)
        command = [
            "make",
            "-f",
            str(ROOT / "workloads/am/Makefile"),
            f"AM_HOME={am}",
            f"PLATFORM_LD={script}",
            f"WORKLOAD={workload}",
        ]
        subprocess.run([*command, "image"], cwd=work, env=env, check=True)
        # LP64 (soft-float), no RVC; Rust additionally validates segments at build().
        attributes = elf.with_suffix(".attributes").read_text()
        if not re.search(
            r'Tag_RISCV_arch:\s+"rv64i2p1"\s*$', attributes, re.MULTILINE
        ) or not re.search(r"Flags:\s+0x0\s*$", attributes, re.MULTILINE):
            raise ValueError(f"unexpected guest ISA/ABI attributes: {attributes}")
        symbols = subprocess.check_output(
            ["riscv64-none-elf-nm", "-n", str(elf)], text=True
        )
        addresses: dict[str, int] = {}
        for line in symbols.splitlines():
            fields = line.split()
            if len(fields) == 3 and fields[2] in (
                "_heap_start",
                "_heap_end",
                "_stack_top",
                "_console",
            ):
                addresses[fields[2]] = int(fields[0], 16)
        platform.heap_bounds(Address(addresses["_heap_start"]), 128 * 1024)
        if (addresses["_heap_end"], addresses["_stack_top"], addresses["_console"]) != (
            platform.stack_bottom,
            platform.stack_top,
            platform.console.start,
        ):
            raise ValueError("ELF symbols do not match the resolved platform")
        # Objdump reports m5 as data. No other undecoded instruction is allowed.
        stubs = (ROOT / "workloads/am/m5.S").read_text()
        m5_words = {
            int(word, 16) for word in re.findall(r"\.long (0x[0-9a-f]+)", stubs)
        }
        for line in elf.with_suffix(".disasm").read_text().splitlines():
            instruction = re.match(r"\s*[0-9a-f]+:\s+([0-9a-f]+)\s+(\S+)", line)
            if instruction is not None:
                word, mnemonic = instruction.groups()
                if len(word) != 8 or (
                    mnemonic.startswith(".") and int(word, 16) not in m5_words
                ):
                    raise ValueError(f"unexpected guest instruction: {line}")
        marker.write_text(
            json.dumps(
                {
                    "key": key,
                    "compiler": compiler,
                    "elf_sha256": hashlib.sha256(elf.read_bytes()).hexdigest(),
                },
                indent=2,
            )
        )
    elf_hash = hashlib.sha256(elf.read_bytes()).hexdigest()
    saved: object = json.loads(marker.read_text())
    if not isinstance(saved, dict) or saved.get("elf_sha256") != elf_hash:
        raise ValueError("cached ELF content no longer matches its build record")
    return GuestBuild(elf, key, compiler, elf_hash)


def run_guest(
    elf: bytes,
    platform: BareMetalPlatform,
    *,
    ram_latency: Tick = DEFAULT_RAM_LATENCY,
    trace: bool = False,
    difftest: DifftestConfig | None = None,
) -> RunResult:
    board = Board()
    core = board.add(
        Rv64Core(
            elf=elf,
            stack_pointer=platform.stack_top,
            cycle=platform.cpu_period,
            stack_bytes=platform.stack_bytes,
            difftest=difftest,
            experiment=ExperimentConfig(
                platform.tick_femtoseconds, platform.instruction_budget
            )
            if platform.m5_enabled
            else None,
        ),
        name="cpu",
    )
    ram = board.add(FixedRam(platform.ram.size_bytes, ram_latency, 1), name="ram")
    uart = board.add(
        SimpleUart(
            platform.console.start, platform.console.size_bytes, platform.pio_latency
        ),
        name="console",
    )
    router = board.add(Router(1, [platform.ram, platform.console]), name="bus")
    board.connect(core.requests, router.inputs[0])
    board.connect(router.outputs[0], ram.input)
    board.connect(router.outputs[1], uart.input)
    return board.build().run(
        RunLimit(platform.event_budget, platform.tick_budget, trace)
    )


def validate(result: RunResult, workload: Workload) -> list[dict[str, str]]:
    if result.stop_reason != StopReason.EXIT or result.exit_code != 0:
        raise ValueError(
            f"guest did not succeed: {result.stop_reason}, {result.exit_code}, {result.fault}"
        )
    output = bytes(byte.value for byte in result.console).decode("ascii")
    if workload == "hello":
        if not output.startswith("Hi\ntime=") or len(result.regions) != 1:
            raise ValueError(f"unexpected hello result: {output!r}")
        return [{"name": "hello", "status": "passed"}]
    actual: list[dict[str, str]] = []
    for line in output.splitlines():
        fields = line.split()
        if len(fields) != 3 or fields[0] != "AM_RESULT":
            raise ValueError(f"unexpected benchmark output: {line!r}")
        actual.append({"name": fields[1], "status": fields[2]})
    expected = [{"name": name, "status": "passed"} for name in EXPECTED]
    if actual != expected or tuple(region.id for region in result.regions) != tuple(
        range(1, len(EXPECTED) + 1)
    ):
        raise ValueError(f"benchmark execution/validation mismatch: {actual!r}")
    return actual


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--workload", choices=("hello", "microbench"), default="microbench"
    )
    parser.add_argument("--build-only", action="store_true")
    parser.add_argument("--display", action="store_true")
    parser.add_argument(
        "--difftest",
        action="store_true",
        help="compare each retirement with the pinned Spike process",
    )
    parser.add_argument(
        "--output", type=Path, default=ROOT / "target/am-results/latest.json"
    )
    args = parser.parse_args()
    raw_workload: object = args.workload
    if raw_workload not in ("hello", "microbench"):
        raise ValueError("unsupported workload")
    workload: Workload = "hello" if raw_workload == "hello" else "microbench"
    destination: object = args.output
    if not isinstance(destination, Path):
        raise TypeError("output must be a path")
    platform = am_test_platform()
    build = build_guest(platform, workload)
    if args.build_only:
        print(build.elf)
        return
    difftest = None
    if args.difftest:
        subprocess.run(["bash", str(ROOT / "tools/build-spike-adapter.sh")], check=True)
        difftest = DifftestConfig(str(ROOT / "target/spike/adapter"))
    start = time.perf_counter()
    try:
        result = run_guest(build.elf.read_bytes(), platform, difftest=difftest)
    except SimulationError as error:
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(
            json.dumps(
                {
                    "workload": workload,
                    "elf_sha256": build.elf_sha256,
                    "configuration": asdict(platform),
                    "reference_configuration": None
                    if difftest is None
                    else asdict(difftest),
                    "passed": False,
                    "simulation_error": str(error),
                },
                indent=2,
            )
            + "\n"
        )
        raise
    host_seconds = time.perf_counter() - start
    console = bytes(byte.value for byte in result.console).decode("ascii")
    if args.display:
        print(console, end="", flush=True)
    report: dict[str, object] = {
        "workload": workload,
        "mainargs": "test",
        "expected_items": ["hello"] if workload == "hello" else list(EXPECTED),
        "excluded_by_configuration": [],
        "build_key": build.key,
        "elf_sha256": build.elf_sha256,
        "compiler": build.compiler,
        "configuration": asdict(platform),
        "ram_latency": 1000,
        "cpu": "blocking",
        "cache": None,
        "difftest": None if result.difftest is None else asdict(result.difftest),
        "git_revision": subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True
        ).strip(),
        "git_dirty": bool(
            subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT)
        ),
        "devenv_lock_sha256": hashlib.sha256(
            (ROOT / "devenv.lock").read_bytes()
        ).hexdigest(),
        "host_seconds": host_seconds,
        "final_tick": result.final_tick,
        "stop_reason": result.stop_reason,
        "exit_code": result.exit_code,
        "fault": None if result.fault is None else asdict(result.fault),
        "statistics": asdict(result.stats),
        "regions": [asdict(region) for region in result.regions],
        "console": console,
    }
    destination.parent.mkdir(parents=True, exist_ok=True)
    try:
        report["items"] = validate(result, workload)
    except ValueError as error:
        report["validation_error"] = str(error)
        destination.write_text(json.dumps(report, indent=2) + "\n")
        raise
    destination.write_text(json.dumps(report, indent=2) + "\n")
    print(
        f"{workload}: validated; commits={result.stats.committed}; tick={result.final_tick}; report={destination}"
    )


if __name__ == "__main__":
    main()
