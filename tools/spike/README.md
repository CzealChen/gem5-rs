# Spike reference adapter and Rust streaming driver

Run `devenv shell -- just check-spike` to build the independent reference process
and execute protocol, synchronization, failure and error-injection tests.
`devenv shell -- just check-am` additionally compares both AM guests using the
installed wheel. Default Cargo builds do not link or launch Spike/C++.

The A0 probe has been removed. `adapter.cc` is the only reference implementation;
`src/difftest.rs` owns the client, comparison policy, deadline and diagnostics.
CPU models only expose read-only architectural observations through
`src/observation.rs`; they do not know the reference protocol or MMIO addresses.

## Fixed dependency

Spike is `riscv-software-src/riscv-isa-sim@d140cbc0ae4ef4fc64ad323f3f175ff15f6dca56`,
source NAR `sha256-chyTl7v7pZUGcRczzrawLOYxSYbbk7eWohcbduU7VBU=`, provided by
`devenv.lock` as `1.1.0-unstable-2026-10-20`. The build script requires that locked
Nix package name and uses its headers, library and runtime rpath together. The
Darwin SoftFloat warning patch is documented in `devenv.nix`. Changing this
source requires updating and revalidating the protocol handshake identity.

## Protocol version 2

One ASCII command and one response per newline, at most 16 KiB per message.
Numbers are unsigned decimal unless stated otherwise. `GPR` means exactly 32
u64 values in x0..x31 order. `MSTATE` is nine u64 values: privilege, mstatus,
mie, mip, mtvec, mscratch, mepc, mcause, mtval. Errors return `error <reason>` and exit nonzero.
Unexpected EOF, malformed messages and timeouts are reference infrastructure
failures, distinct from DUT faults or a guest failure exit.

| Request | Response | Meaning |
| --- | --- | --- |
| `hello` | `hello 2 <commit> rv64i_zicsr pc_gpr_mem_machine` | Exact version, source and state scope |
| `init <rv64i/rv64i_zicsr> <base> <size> <pc> GPR` | `ok` | Page-aligned RAM, up to 256 MiB, initially zero; explicit PC/GPR |
| `load <address> <hex bytes>` | `ok` | Up to 4096 bytes; only before stepping/synchronizing |
| `inspect` | `instruction <pc> <word>` | Check skipped instruction without executing it |
| `step` | `state <pc> <word> <next_pc> GPR <count> [<read/write> <address> <width> <value>]` | One independent Spike step, zero or one ordinary memory effect; machine mode appends `machine MSTATE` |
| `step` (trap) | `fault <pc> <word> <mcause> <mepc> <mtval>` | No retirement; faulting instruction is zero if fetch was unmapped |
| `sync <pc> GPR` | `ok` | Replace all architectural GPR/PC; reject nonzero x0 and unaligned PC; RAM unchanged |
| `pending <0/1>` | `ok` | Machine mode only: external MTIP input, no CSR/PC/GPR synchronization |
| `machine` | `machine <pc> GPR MSTATE` | Read reference state after trap entry or skipped platform operation |
| `quit` | `bye`, then exit 0 | Orderly shutdown; nonzero exit after bye still fails |

The Rust loader's validated ELF segments initialize the reference and DUT with
the same bytes. Both RAMs start zeroed, including BSS, and both start directly
at the ELF entry with the same registers. Spike's reset ROM is not executed.
`minstret` distinguishes the current step's trap, so consecutive identical trap
causes remain observable. Load bytes are read from reference RAM; store effects
come from Spike's commit log. No DUT RAM data is copied during synchronization.

## Comparison and failure rules

Each ordinary retirement compares execution PC/word, next PC, all GPRs and the
load/store address, width and bytes. Mismatches poison the driver immediately;
a subsequent synchronization cannot overwrite or clear the first difference.
Diagnostics retain only 16 recent instruction identities, not full trace snapshots.
Architectural faults compare PC, cause and any relevant fault address. A matched
terminal fault still fails acceptance. In machine mode, handled traps instead
compare cause, mepc, mtval, handler PC, GPRs and MSTATE, then execution continues.
They do not retire instructions. CSR instructions and mret execute in Spike; all
nine machine-state fields are compared after each retirement and trap. No blanket
CSR synchronization or CSR masking is used. Only the implemented M-only profile
is supported; Spike supports additional CSRs/WARL choices that the DUT does not.

Only successfully completed enabled m5 operations, console byte stores, legacy
Exit stores and configured MachineTimer accesses can synchronize. Timer reads
are 64-bit at base/+8; writes are 64-bit at +8 only. Addresses/widths come from the validated board.
Before skipping, the reference instruction is inspected; an eligible reference
timer interrupt prevents the skip, including at m5_exit. MMIO loads/stores are also
re-decoded against the preceding compared register state: address, width, direction,
store data and the instruction's GPR effects are checked. A load consumes the DUT's
device value (including sign extension), and a store does not update reference RAM.
An erroneous RAM access cannot be disguised as a platform operation. There are no PC/name-based
skip regions and no resynchronization after a comparison error.

The current driver supports one core, one contiguous RAM, no additional RAM
writers, and optional M-only Zicsr/trap execution. It rejects other topologies.
The DUT samples timer pending at its instruction boundary and sends level changes
to Spike. Spike independently decides interrupt eligibility, takes the trap and
executes the handler. Its timer does not run independently. This works for both
simulated and host-wall timers, but wall runs are nondeterministic and reference
IPC overhead can require a longer timer quantum.

Expected exit code and failure/success kind are checked before reporting passed.
`DifftestConfig.allow_instruction_limit` additionally permits a checked prefix to
finish at the instruction limit, reported as `bounded=true`; it does not accept
event/tick limits or faults. OS CLI runs opt into this and still require workload
progress validation. No limit is added automatically. Ordinary benchmark runs
still require a successful guest exit. Statistics separately count ordinary steps,
m5/console/Exit/timer synchronization, synchronous traps, interrupts and CSR
comparisons. U/S privilege, FP, vectors and multicore remain outside scope. A timeout covers request write plus response read; every
exit/error path kills or reaps the child and joins its I/O worker.

The driver is streaming; CPU commit logging is optional. The existing memory
engine still retains its transaction history through a run, independently of
reference communication. This does not claim the whole simulator has bounded
memory for arbitrarily long workloads. Spike validates architecture, not timing
or the skipped services; their independent platform tests remain required.
