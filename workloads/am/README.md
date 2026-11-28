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

For interactive runs inside `devenv shell`:

```sh
maturin develop --release
python -m tools.am_benchmark --workload hello --display
python -m tools.am_benchmark --workload microbench --difftest --display --output target/am-results/test.json
python -m tools.am_benchmark --workload microbench --build-only
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
allow longer runs; defaults remain bounded for test inputs. Larger inputs have
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
records or missing ROI records also fail host acceptance.

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
next PC, all GPRs and memory effects. Completed m5/console/Exit operations synchronize
all GPRs and PC; the reference never executes those platform operations.

The option does not change guest build inputs, timing, console bytes, ROI or
end-to-end simulated statistics. Native errors stop at the first difference and
are saved in the requested JSON report. Reference timeouts/crashes and guest
faults never count as a passing run. The expected exit is success/code zero; the
public `DifftestConfig` also supports explicit failure/code expectations.

Python assembly can pass `DifftestConfig(adapter=...)` to `Rv64Core.difftest`.
`RunResult.difftest` is optional typed statistics; without an observer no reference
process or per-instruction communication is used. Protocol and supported-state
boundaries are documented in [the adapter README](../../tools/spike/README.md).
