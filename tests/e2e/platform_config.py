"""Installed public platform configuration, bounds and physical time checks."""

import os
import subprocess
from dataclasses import replace
from pathlib import Path
from tempfile import TemporaryDirectory
from unittest import TestCase, main

from gem5rs import Address, AddressRange, BareMetalPlatform, Tick, am_test_platform


class PlatformTests(TestCase):
    def test_generated_layout_links_and_rejects_stack_overlap(self) -> None:
        platform = am_test_platform()
        with TemporaryDirectory() as directory:
            root = Path(directory)
            script = root / "platform.ld"
            source = root / "start.S"
            script.write_text(platform.linker_script())
            command = [
                os.environ.get("CC_RV64", "riscv64-none-elf-gcc"),
                "-march=rv64i",
                "-mabi=lp64",
                "-nostdlib",
                "-static",
                "-Wl,--build-id=none",
                "-T",
                str(script),
                str(source),
                "-o",
                str(root / "probe.elf"),
            ]
            start = ".section .text.start\n.global _start\n_start: addi a0,zero,0\n"
            source.write_text(start + ".section .bss\n.space 32\n")
            result = subprocess.run(
                command, capture_output=True, text=True, timeout=30, check=False
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            source.write_text(
                start + f".section .bss\n.space {platform.ram.size_bytes - 16}\n"
            )
            result = subprocess.run(
                command, capture_output=True, text=True, timeout=30, check=False
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("image/BSS overlaps reserved stack", result.stderr)

    def test_layout_and_heap(self) -> None:
        platform: BareMetalPlatform = am_test_platform()
        self.assertEqual(platform.ram.size_bytes, 16 * 1024 * 1024)
        self.assertEqual(platform.stack_top, Address(0x81000000))
        heap = platform.heap_bounds(Address(0x80000101), 1024)
        self.assertEqual(heap.start, Address(0x80000110))
        self.assertEqual(heap.start + heap.size_bytes, platform.stack_bottom)
        self.assertIn("_console = 0x10000000", platform.linker_script())
        with self.assertRaises(ValueError):
            platform.heap_bounds(platform.stack_top, 0)
        with self.assertRaises(ValueError):
            platform.heap_bounds(platform.stack_bottom, 1)

    def test_time_units_and_short_intervals(self) -> None:
        platform = am_test_platform()
        self.assertEqual(platform.nanoseconds(Tick(999)), 0)
        self.assertEqual(platform.nanoseconds(Tick(1000)), 1)
        self.assertEqual(platform.nanoseconds(Tick(1_000_000)), 1000)
        with self.assertRaises(ValueError):
            replace(platform, tick_femtoseconds=(1 << 64) - 1)

    def test_invalid_layout_and_budgets(self) -> None:
        platform = am_test_platform()
        with self.assertRaises(ValueError):
            replace(platform, console=platform.ram)
        with self.assertRaises(ValueError):
            replace(platform, ram=AddressRange(Address((1 << 64) - 4096), 4096))
        with self.assertRaises(ValueError):
            replace(platform, stack_bytes=platform.ram.size_bytes)
        with self.assertRaises(ValueError):
            replace(platform, cpu_period=Tick(0))
        with self.assertRaises(ValueError):
            replace(platform, event_budget=0)


if __name__ == "__main__":
    main()
