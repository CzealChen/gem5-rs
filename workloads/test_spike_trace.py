from dataclasses import replace
from pathlib import Path
import unittest

from tools.spike_trace import Commit, compare_trace, parse_spike_log, run_spike


class SpikeTraceTests(unittest.TestCase):
    def test_reset_rom_and_exit_store_bracket_the_trace(self) -> None:
        reference = parse_spike_log([
            "warning: tohost and fromhost symbols not in ELF; can't communicate with target",
            "core   0: 3 0x0000000000001000 (0x00000297) x5  0x0000000000001000",
            "core   0: 3 0x0000000080000000 (0x00000293) x5  0x0000000000000000",
            "core   0: 3 0x0000000080000004 (0x00000713) x14 0xffffffffffffffff mem 0x0000000080000010",
            "core   0: 3 0x0000000080000008 (0x00a2a023) mem 0x0000000000100000 0x00000002",
            "core   0: 3 0x000000008000000c (0x0000006f)",
        ])
        self.assertEqual(reference.commits, (
            Commit(0x8000_0000, 0x00000293, (5, 0)),
            Commit(0x8000_0004, 0x00000713, (14, 2**64 - 1)),
        ))
        self.assertEqual(reference.exit_commit, Commit(0x8000_0008, 0x00A2A023))
        self.assertEqual(reference.exit_code, 2)

    def test_exit_access_width(self) -> None:
        with self.assertRaisesRegex(ValueError, "Expected a 32-bit exit store"):
            parse_spike_log([
                "core   0: 3 0x0000000080000000 (0x00a2b023) mem 0x0000000000100000 0x0000000000000000",
            ])

    def test_built_workloads(self) -> None:
        build = Path(__file__).resolve().parent / "build"
        asm = run_spike(build / "rv64i.elf")
        loop = run_spike(build / "loop.elf")
        self.assertEqual((asm.exit_code, loop.exit_code), (0, 0))
        self.assertEqual(asm.commits[0], Commit(0x8000_0000, 0x00000293, (5, 0)))
        self.assertEqual(sum(c.register_write == (10, 17) for c in loop.commits), 1)

        actual = [*asm.commits, asm.exit_commit]
        self.assertIsNone(compare_trace(asm, actual))
        self.assertIn("commit[0] length mismatch", compare_trace(asm, []) or "")
        self.assertIn("length mismatch", compare_trace(asm, actual[:-1]) or "")
        self.assertIn("length mismatch", compare_trace(asm, [*actual, asm.exit_commit]) or "")
        actual[0] = replace(actual[0], register_write=(5, 1))
        actual[1] = replace(actual[1], instruction=0)
        self.assertEqual(compare_trace(asm, actual),
                         "commit[0] register write mismatch: "
                         "Spike: PC=0x0000000080000000 insn=0x00000293 write=x5=0x0000000000000000; "
                         "simulator: PC=0x0000000080000000 insn=0x00000293 write=x5=0x0000000000000001")
