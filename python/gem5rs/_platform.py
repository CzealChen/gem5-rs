"""Resolved bare-metal layout shared by guest build and future P2 assembly.

This configuration does not enable AM on the legacy Machine. Runtime models
consume it only after the P2 component/transaction interfaces are available.
"""

from dataclasses import dataclass

from ._types import Address, AddressRange, Tick

_U64_MAX = (1 << 64) - 1


def _positive_u64(name: str, value: int) -> None:
    if type(value) is not int or not 0 < value <= _U64_MAX:
        raise ValueError(f"{name} must be a positive u64")


def _end(name: str, region: AddressRange) -> int:
    _positive_u64(f"{name}.size_bytes", region.size_bytes)
    if type(region.start) is not int or not 0 <= region.start <= _U64_MAX:
        raise ValueError(f"{name}.start must be a u64 address")
    end = region.start + region.size_bytes
    if end > _U64_MAX:
        raise ValueError(f"{name} end overflows u64")
    return end


@dataclass(frozen=True, slots=True)
class BareMetalPlatform:
    """Physical RV64I/LP64 platform; console is byte-write-only, uncached MMIO.

    Time is an integer number of femtoseconds per tick. The explicit m5 flag
    belongs to the experiment, and is disabled unless requested by its caller.
    """

    ram: AddressRange
    console: AddressRange
    stack_bytes: int
    tick_femtoseconds: int
    cpu_period: Tick
    pio_latency: Tick
    tick_budget: Tick
    event_budget: int
    instruction_budget: int
    m5_enabled: bool = False

    def __post_init__(self) -> None:
        ram_end = _end("ram", self.ram)
        console_end = _end("console", self.console)
        if self.ram.start < console_end and self.console.start < ram_end:
            raise ValueError("RAM and console overlap")
        if self.ram.start % 4096 or self.ram.size_bytes % 4096:
            raise ValueError("RAM must be page aligned for the reference executor")
        for name, value in (
            ("stack_bytes", self.stack_bytes),
            ("tick_femtoseconds", self.tick_femtoseconds),
            ("cpu_period", self.cpu_period),
            ("pio_latency", self.pio_latency),
            ("tick_budget", self.tick_budget),
            ("event_budget", self.event_budget),
            ("instruction_budget", self.instruction_budget),
        ):
            _positive_u64(name, value)
        if self.stack_bytes % 16 or self.stack_bytes >= self.ram.size_bytes:
            raise ValueError(
                "stack must be 16-byte aligned and leave room for the image"
            )
        if type(self.m5_enabled) is not bool:
            raise ValueError("m5_enabled must be a bool")
        self.nanoseconds(self.tick_budget)

    @property
    def stack_top(self) -> Address:
        return Address(self.ram.start + self.ram.size_bytes)

    @property
    def stack_bottom(self) -> Address:
        return Address(self.stack_top - self.stack_bytes)

    def nanoseconds(self, ticks: Tick) -> int:
        if type(ticks) is not int or not 0 <= ticks <= _U64_MAX:
            raise ValueError("ticks must be a u64")
        result = ticks * self.tick_femtoseconds // 1_000_000
        if result > _U64_MAX:
            raise ValueError("nanoseconds overflow u64")
        return result

    def heap_bounds(self, loaded_end: Address, required_bytes: int) -> AddressRange:
        """Validate image/BSS high-water mark and required heap before execution."""
        if (
            type(loaded_end) is not int
            or not self.ram.start <= loaded_end <= self.stack_bottom
        ):
            raise ValueError("loaded image/BSS overlaps stack or lies outside RAM")
        if type(required_bytes) is not int or not 0 <= required_bytes <= _U64_MAX:
            raise ValueError("required heap bytes must be a u64")
        start = (loaded_end + 15) & ~15
        available = self.stack_bottom - start
        if required_bytes > available:
            raise ValueError("required heap does not fit below the reserved stack")
        return AddressRange(Address(start), available)

    def linker_script(self) -> str:
        """Generate guest layout; no independently maintained address constants."""
        return f"""OUTPUT_ARCH(riscv)
ENTRY(_start)
MEMORY {{ RAM (rwx) : ORIGIN = {int(self.ram.start):#x}, LENGTH = {self.ram.size_bytes:#x} }}
SECTIONS {{
  . = ORIGIN(RAM);
  .text : {{ KEEP(*(.text.start)) *(.text .text.*) }} > RAM
  .rodata : {{ *(.rodata .rodata.*) }} > RAM
  .data : {{ *(.data .data.*) *(.sdata .sdata.*) }} > RAM
  .bss (NOLOAD) : {{ *(.bss .bss.*) *(.sbss .sbss.*) *(COMMON) }} > RAM
  _heap_start = ALIGN(., 16);
  _stack_top = ORIGIN(RAM) + LENGTH(RAM);
  _stack_bottom = _stack_top - {self.stack_bytes:#x};
  _heap_end = _stack_bottom;
  _console = {int(self.console.start):#x};
  ASSERT(_heap_start <= _heap_end, "image/BSS overlaps reserved stack")
}}
"""


def am_test_platform() -> BareMetalPlatform:
    """Separate 16 MiB AM test profile; does not alter the 4 KiB baseline."""
    return BareMetalPlatform(
        ram=AddressRange(Address(0x8000_0000), 16 * 1024 * 1024),
        console=AddressRange(Address(0x1000_0000), 4096),
        stack_bytes=64 * 1024,
        tick_femtoseconds=1000,
        cpu_period=Tick(1000),
        pio_latency=Tick(1000),
        tick_budget=Tick(100_000_000_000),
        event_budget=1_000_000_000,
        instruction_budget=100_000_000,
        m5_enabled=True,
    )
