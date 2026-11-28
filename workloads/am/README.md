# AM test on the current blocking core

Run from the repository root:

```sh
devenv shell -- just check-am
```

This builds a release wheel, installs it into an isolated directory using the
existing devenv/uv environment, runs strict type checks, compiles guests through
Make, tests the platform, and writes `target/am-results/acceptance.json`.
`just check-mvp` retains the fast baseline; `just check-spike` checks the real
reference protocol, error injection and process failures. CI runs all three.
AM acceptance compares hello and all ten test algorithms with Spike, checks
difftest on/off invariance, and records protocol/source identity and sync counts.
It also checks bounded `yield-os` progress, trace invariance, and M-mode difftest. Single-hart `thread-os` acceptance checks timer
preemption, five-thread progress, deterministic replay, interrupt wiring, and an
AM atomic/register-preservation probe with both simulated and wall-clock difftest.

For interactive runs inside `devenv shell`:

```sh
maturin develop --release
python -m tools.am_benchmark --workload hello --display
python -m tools.am_benchmark --workload microbench --difftest --display --output target/am-results/test.json
python -m tools.am_benchmark --workload microbench --build-only
python -m tools.am_benchmark --workload yield-os --instruction-budget 3000000 --display --output target/am-results/yield-os.json
```

## Build and runtime boundary

Python resolves `BareMetalPlatform`, stages sources and generates `platform.ld`.
The local Makefile includes the **unchanged upstream AM Makefile**, which owns
compilation, archive creation, dependency tracking and linking. The platform
`.mk` supplies RV64I/LP64, `-O2`, medany, no relaxation/small-data/PIC and
`-fwrapv` (the upstream checksum uses wrapping signed arithmetic).

Guest inputs are vendored at fixed commits; see `../../third_party/README.md`
(relative to the repository root: `third_party/README.md`) and `sources.json`.
No source download is required during the checks.
`third_party/abstract-machine/klib` contains upstream headers and archive build
rules; the actual klib implementation lives in `workloads/am/klib.c` and
`softarith.c`. `amdev.h` is included by `am.h` and declares device interfaces;
only its uptime interface is implemented here. Other declarations do not add
simulated devices. The AM/klib headers, algorithms,
input settings, allocator, RNG and checksum helpers are unchanged. Project-owned
startup, runtime and klib implement the functions actually used; no panic template
or silent printf stub is linked. Unsupported formatting/IOE operations fail.

The Nix cross compiler's bundled libgcc is LP64D, so it cannot be linked into an
LP64 guest. `softarith.c` provides real RV64I software multiply/divide/remainder
helpers. The hello guest tests large unsigned operands, signed division/remainder,
zero divisors and signed overflow through the helper ABI. Final ELF flags,
architecture attributes, disassembly, stack/heap symbols and simulator load plan
are checked before execution. m5 words are checked against generated `m5.S`;
regenerate with `cargo run --bin m5-stubs > workloads/am/m5.S`.

Input selection is compiled into the runtime and included in the cache key.
`huge` defaults to 128 MiB RAM; other inputs use 16 MiB. `--ram-mib` overrides
this. Heap checks use the selected upstream requirements (128 KiB / 1 MiB /
10 MiB / 64 MiB). `--instruction-budget`, `--event-budget` and `--tick-budget`
set optional limits; all three default to unlimited in the CLI. Omitted limits
are recorded as `null` and use the native engine’s full u64 counter range.
The `am_test_platform()` fixture retains finite budgets for automated tests. Larger inputs have
build/startup coverage, not full-suite acceptance.

The build cache under `target/am/<sha256>/` includes all vendored and platform
sources, the adapter, lockfile, compiler version and generated linker script.
An unsuccessful build never gets a completion marker. Reusing an artifact checks
its ELF hash. Runtime CPU/PIO/RAM latency and budgets do not require recompilation;
layout changes generate a different artifact. Reports are saved separately.

## Experiment

The explicit profile has 16 MiB RAM at `0x80000000`, a 64 KiB reserved stack,
console at `0x10000000`, 1 ps ticks and a 1 ns CPU period. RAM and PIO latency
are each 1000 ticks. The profile enforces 12 million events, 2 million instructions and 10 billion
ticks. The ELF must leave at least 128 KiB heap for the largest test requirement.
This is a separate workload from the original 4 KiB Exit-MMIO examples.

The console accepts only byte writes at offset zero, cannot be fetched/read,
never caches data, and does not interpret EOT as exit. A byte is recorded when
the device accepts a write; its acknowledgement arrives after the PIO delay.
Host display happens after simulation and cannot change the guest's behavior.

`--input test|train|ref|huge` selects guest `mainargs` (default `test`). The complete upstream order is qsort, queen,
bf, fib, sieve, 15pz, dinic, lzip, ssort, md5. The runner uses the upstream
`BENCHMARK_LIST` directly, including the three C++ items and QuickLZ. Each
item retains prepare/run/validate and the upstream test checksum. Memory shortage
reports a skipped item and fails the run; missing/duplicate/non-passing result
records or missing ROI records also fail host acceptance. Human-readable upstream
output is retained alongside `AM_RESULT`: all inputs print scored and total time;
non-test inputs print per-item minimum time and score; ref/huge also print overall
Marks and the upstream reference CPU. The pinned upstream REPEAT is one.
Scored time surrounds run (including ROI marker overhead), while total time also
includes preparation, validation and loop console output. Both use simulated uptime,
not host wall time; Marks describe this model, not real hardware performance.

ROI contains only run plus the ordinary wrapper instructions between the markers.
The begin/end markers themselves are excluded from committed counts. Tick boundaries
are the marker execution/commit times; this does not remove all wrapper overhead.
Preparation and preceding items retain their normal state. The primary report is
raw ticks/commits, not a guest score. `uptime` floors ns to us, so a short valid
interval can print zero microseconds. Host seconds are reported separately.

## Current limits

This delivers all ten algorithm self-checks on the existing blocking CPU, as
requested before P2, with A3 streaming Spike comparison. A5 pipeline/cache
combinations and train/ref/huge acceptance remain outside this entry. ROI currently reports
ticks and commits; transaction counters are end-to-end, not ROI snapshots.
`trace=False` disables CPU commit storage and large public trace results. The
difftest driver consumes each retirement immediately and keeps only 16 diagnostic
instruction identities. The current Machine still retains internal transaction
records until the run ends, so whole-simulator bounded memory remains follow-up
work; large inputs therefore require host-memory planning.

## Differential execution

`--difftest` builds the pinned independent Spike adapter and attaches the Rust
observer to the same blocking CPU. Every ordinary instruction compares PC, word,
next PC, all GPRs and memory effects; machine mode also compares privilege and
mstatus/mie/mip/mtvec/mscratch/mepc/mcause/mtval. Completed m5/console/Exit/timer operations synchronize
all GPRs and PC; the reference never executes those platform operations.

With simulated clocks the option does not change guest build inputs, timing,
console bytes, ROI or end-to-end simulated statistics. Wall clocks are subject to
host execution overhead and cannot provide this invariance. Native errors stop at the first difference and
are saved in the requested JSON report. Reference timeouts/crashes and guest
terminal faults never count as a passing run. Handled machine traps continue
execution after independently comparing the trap transition. The expected exit is success/code zero; the
public `DifftestConfig` also supports explicit failure/code expectations.

Python assembly can pass `DifftestConfig(adapter=...)` to `Rv64Core.difftest`.
`RunResult.difftest` is optional typed statistics; without an observer no reference
process or per-instruction communication is used. Protocol and supported-state
boundaries are documented in [the adapter README](../../tools/spike/README.md).

## Cooperative OS workload

`yield-os` compiles the unchanged upstream kernel with RV64I + Zicsr/LP64 and
selects `Rv64Core(machine_mode=True)`; `thread-os` uses the same mode plus a timer.
The benchmark workloads keep the default RV64I fault-stopping behavior. The native call signatures are unchanged: this
optional typed core setting is serialized through the existing board graph.

The core supports M-only synchronous traps, direct `mtvec`, six CSR instruction
forms, and `mret`. Implemented CSRs are `mstatus`, fixed `misa`, `mtvec`, `mscratch`,
`mepc`, `mcause`, `mtval`, read-only hart-zero `mhartid`, `mie` (MTIE), and `mip`
(read-only hardware MTIP; writes do not acknowledge the timer). MPP is fixed to M;
unsupported status bits read zero, `mtvec` mode is fixed to direct and `mepc` is
four-byte aligned. Unknown CSR accesses trap; write attempts to `mhartid` trap.
Misalignment/access faults, illegal instructions, breakpoints and machine ecalls
enter the guest handler without retiring the faulting instruction. Simulator
protocol failures remain terminal. A connected machine timer can deliver interrupts.
There are no U/S privilege modes, MMU or multicore support in this feature.

`cte.c` implements `cte_init/kcontext/yield`; `trap.S` saves/restores all GPRs,
original SP and machine return state, and switches to the context selected by the
handler. Compile-time assertions verify the upstream `Context` layout. The
assembly derives from AbstractMachine `am/src/riscv/npc/trap.S` at the pinned
commit `068dc8cc150c63388fc5f457ff7b7dd70c5dd0f5`, copyright 2018–2021 Yanyan Jiang
and Zihao Yu, under [its MIT license](../../third_party/abstract-machine/LICENSE).
The original template's frame layout and SP/context switching are not copied
unchanged. CPU CSR/trap semantics follow the [RISC-V Machine-Level ISA](https://docs.riscv.org/reference/isa/priv/machine.html).

This upstream program never exits. The example explicitly limits execution to
3 million retired instructions, producing `ABABAB`. Without a budget it keeps
running. Acceptance requires an
instruction-budget stop, no fault, and at least four strictly alternating letters;
it reports `bounded-progress`, not successful guest termination. A smaller budget
may be useful for diagnosis but will fail acceptance if it shows too little progress.
Add `--difftest` for independent CSR/trap/mret and ordinary instruction comparison.
The OS CLI enables `DifftestConfig.allow_instruction_limit`; a verified prefix has
`difftest.bounded=true`, and still needs the workload progress checks. Without
this explicit public configuration, a difftest instruction-limit stop is an error.
Tick/event limits remain errors. No default limit is introduced.

```sh
python -m tools.am_benchmark --workload yield-os --instruction-budget 3000000 --difftest --display
python -m tools.am_benchmark --workload thread-os --instruction-budget 6000000 --timer-clock simulated --difftest --display
```

Timer MMIO is skipped only in the reference, after checking the configured device,
address, width, direction and instruction effects. Reads synchronize the DUT value;
CSR instructions and trap entry are not skipped. External MTIP level changes are
injected at the same instruction boundary, and Spike decides masking/delivery.
Device semantics remain covered by independent timer tests.

## Single-hart preemption and timer clocks

```sh
python -m tools.am_benchmark --workload thread-os --instruction-budget 6000000 --timer-clock simulated --display
python -m tools.am_benchmark --workload thread-os --instruction-budget 6000000 --timer-clock wall --timer-frequency-hz 1000000 --timer-interval-us 10000 --display
```

Both clock sources use one timer device with `mtime` at offset 0 (read-only) and
`mtimecmp` at offset 8, initially `UINT64_MAX`. Accesses must be aligned 64-bit
loads/stores; instruction fetches, partial accesses and writes to mtime fault.
MMIO state changes and reads occur at transaction completion, after PIO latency.
The AM profile places this 16-byte device at `0x02000000`; it is not a full CLINT.

Python wires the memory request path normally and connects the dedicated typed
ports with `board.connect_interrupt(timer.interrupt, core.timer_interrupt)`.
`Board.add(Rv64Core(...))` now returns `CoreHandle`, a `RequestorHandle` subclass
with the timer input; `Board.add(MachineTimer(...))` returns `TimerHandle` with
MMIO input and interrupt output. A timer must connect to the board's single
machine-mode core; missing, cross-board or wrong-direction wiring is rejected.
Public native call signatures and `.pyi` remain unchanged; the board graph carries
these models and connections. Installed-wheel typing checks cover both valid and
invalid interrupt wiring.

`SimulatedTimerClock(Tick(...))` selects an integer simulation-tick period.
`WallTimerClock(frequency_hz=...)` selects Rust's monotonic `Instant` elapsed time,
reset immediately before the event engine starts. Counter conversion uses integer
arithmetic and floors fractional counts. Frequency and scheduling interval are
independent: the CLI defaults to simulated time, 1 MHz, and 1000 μs; the wall
example uses a 10000 μs quantum to reduce host-dependent preemption overhead.
`BareMetalPlatform` resolves Hz to ticks and generates the authoritative timer
symbol and guest reload count; inexact simulated periods or reload counts fail
configuration validation. Clock selection alone does not change the guest ELF.

The timer level is sampled at instruction execution boundaries. Pending interrupts
survive masking; the in-flight memory instruction completes before any interrupt
is taken. MIE and MTIE gate entry, trap entry masks MIE, and mret restores it.
Only writing a future mtimecmp clears pending; CSR writes to mip do not. The
handler rearms the device and dispatches `EVENT_IRQ_TIMER` without advancing mepc.
This uses no per-timer-count events and does not implement WFI/idle wake-up.

The single-hart AM implementation supplies `cpu_count/cpu_current/mpe_init`,
`iset/ienabled` and an interrupt-protected `atomic_xchg` that restores the previous
MIE state. This is not cross-hart synchronization and requires no A extension.
The original thread-os source is unchanged. The examples explicitly limit execution to 6 million
retired instructions; without a budget it keeps running. Acceptance requires
an instruction-limit stop without faults, at least five timer interrupts, and
at least two complete CPU-0 output lines from each of A–E. A trailing incomplete
but valid line prefix is permitted; malformed or missing thread output is not.

`timer_interrupts` is part of typed run statistics. Reports record clock source,
frequency, interval, simulation ticks and host elapsed seconds separately. Wall
clock execution is intentionally nondeterministic: host scheduling, logging and
trace overhead change where interrupts occur. It does not pace the simulation to
wall time. Deterministic regression/trace invariance use simulated clocks; wall
clock runs may need a longer quantum or instruction budget to meet the same
progress criteria. Both modes support difftest; its host IPC overhead may require
a longer wall-clock quantum. Use simulated mode for reproducible comparisons.
