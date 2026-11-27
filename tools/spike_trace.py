"""Reference execution and commit comparison for the RV64I workloads."""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from collections.abc import Iterable
from dataclasses import dataclass
from itertools import zip_longest
from pathlib import Path

RAM_BASE = 0x8000_0000
RAM_SIZE = 0x1000
EXIT_ADDRESS = 0x10_0000

_HEADER = re.compile(r"core\s+0:\s+3\s+(0x[0-9a-f]+)\s+\((0x[0-9a-f]+)\)(.*)")
_EFFECTS = re.compile(
    r"(?:\s+x([0-9]+)\s+(0x[0-9a-f]+))?"
    r"(?:\s+mem\s+(0x[0-9a-f]+)(?:\s+(0x[0-9a-f]+))?)?\s*"
)


@dataclass(frozen=True)
class Commit:
    pc: int
    instruction: int
    register_write: tuple[int, int] | None = None

    def __str__(self) -> str:
        write = "—"
        if self.register_write is not None:
            index, value = self.register_write
            write = f"x{index}=0x{value:016x}"
        return f"PC=0x{self.pc:016x} insn=0x{self.instruction:08x} write={write}"


@dataclass(frozen=True)
class ReferenceTrace:
    commits: tuple[Commit, ...]
    exit_commit: Commit
    exit_code: int


def parse_spike_log(lines: Iterable[str]) -> ReferenceTrace:
    """Align to _start and collect commits through the first 32-bit exit store."""
    commits: list[Commit] = []
    for line in lines:
        if not line.startswith("core"):
            continue
        header = _HEADER.fullmatch(line.rstrip())
        if header is None:
            raise ValueError(f"Unrecognized Spike commit: {line.rstrip()}")
        pc, instruction = (int(header[i], 16) for i in (1, 2))
        if not commits and 0x1000 <= pc < 0x1020:
            continue
        if not commits and pc != RAM_BASE:
            raise ValueError(f"Expected ELF entry at 0x{RAM_BASE:x}, got PC=0x{pc:x}")
        if not RAM_BASE <= pc < RAM_BASE + RAM_SIZE or pc % 4 or instruction & 3 != 3:
            raise ValueError(f"Invalid RV64I workload commit: {line.rstrip()}")
        effects = _EFFECTS.fullmatch(header[3])
        if effects is None:
            raise ValueError(f"Unrecognized Spike effects: {line.rstrip()}")
        rd, value_hex, address_hex, data_hex = effects.groups()
        register_write = (
            (int(rd), int(value_hex, 16)) if rd not in (None, "0") else None
        )
        commit = Commit(pc, instruction, register_write)
        if address_hex is not None and int(address_hex, 16) == EXIT_ADDRESS and data_hex is not None:
            if instruction & 0x707F != 0x2023:  # SW: funct3=010, opcode=0100011
                raise ValueError(f"Expected a 32-bit exit store: {commit}")
            return ReferenceTrace(tuple(commits), commit, int(data_hex, 16) & 0xFFFF_FFFF)
        commits.append(commit)
    raise ValueError(f"No exit store at 0x{EXIT_ADDRESS:x} after {len(commits)} workload commits")


def run_spike(elf: Path, *, spike: str = "spike", max_instructions: int = 512) -> ReferenceTrace:
    """Run Spike with the workload's RAM and a page capturing the exit store."""
    if max_instructions <= 0:
        raise ValueError("Spike instruction budget must be positive")
    result = subprocess.run(
        [
            spike, "--isa=rv64i_zicsr", "--priv=m", "-p1",
            f"-m{RAM_BASE:#x}:{RAM_SIZE:#x},{EXIT_ADDRESS:#x}:{RAM_SIZE:#x}",
            f"--instructions={max_instructions}", "--log-commits", str(elf.resolve()),
        ],
        capture_output=True, text=True, timeout=30, check=False,
    )
    if result.returncode != 0:
        raise ValueError(f"Spike failed for {elf.name} ({result.returncode}): {result.stderr.strip()}")
    return parse_spike_log(result.stderr.splitlines())


def first_difference(expected: Iterable[Commit], actual: Iterable[Commit]) -> str | None:
    """Diagnose the earliest mismatched or missing commit, using zero-based indices."""
    for index, (want, got) in enumerate(zip_longest(expected, actual)):
        if want != got:
            field = "length"
            if want is not None and got is not None:
                field = (
                    "PC" if want.pc != got.pc else
                    "instruction" if want.instruction != got.instruction else
                    "register write"
                )
            return (
                f"commit[{index}] {field} mismatch: Spike: {want or '<end>'}; "
                f"simulator: {got or '<end>'}"
            )
    return None


def compare_trace(reference: ReferenceTrace, actual: Iterable[Commit]) -> str | None:
    """Compare the prefix, then check the terminal store's instruction boundary."""
    def with_exit_boundary() -> Iterable[Commit]:
        for index, commit in enumerate(actual):
            if index == len(reference.commits):
                yield Commit(commit.pc, commit.instruction)
            else:
                yield commit

    return first_difference((*reference.commits, reference.exit_commit), with_exit_boundary())


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("elf", type=Path, nargs="+")
    elfs: object = parser.parse_args().elf
    assert isinstance(elfs, list)
    for elf in elfs:
        assert isinstance(elf, Path)
        try:
            trace = run_spike(elf)
        except (ValueError, OSError, subprocess.TimeoutExpired) as error:
            print(error, file=sys.stderr)
            return 1
        print(f"{elf.name}: {len(trace.commits)} commits before exit; "
              f"PC=0x{trace.exit_commit.pc:x}; MMIO value={trace.exit_code}")
        if trace.exit_code != 0:
            return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
