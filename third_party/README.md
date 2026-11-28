# Vendored AM build and benchmark inputs

All imported source files are byte-for-byte copies. Local documentation additions
are the catalog in `am-kernels/README.md`, the gem5-rs run instructions in
`am-kernels/benchmarks/microbench/README.md`, the additional
MD5 license file and this provenance note. No downloaded build artifacts are stored.

- `abstract-machine`: NJU-ProjectN/abstract-machine commit
  `068dc8cc150c63388fc5f457ff7b7dd70c5dd0f5`. Contains the upstream Makefile,
  AM/klib archive Makefiles, and only the headers needed by this platform.
  The upstream MIT license is preserved in `abstract-machine/LICENSE`.
- `am-kernels`: NJU-ProjectN/am-kernels commit
  `9c433f7cf4b4e05d09543bf7e269b23502d1b2b6`. Contains the complete tracked upstream tree: `benchmarks/`, `kernels/`,
  `tests/`, root documentation and build metadata. MicroBench lives at its
  upstream path, `benchmarks/microbench/`. Only MicroBench is currently wired
  into the gem5-rs build; importing other workloads does not enable them.
  `am-kernels/LICENSE` preserves its upstream MIT notice.
- MD5's source header attributes github.com/pod32g/md5. Its Apache-2.0 license
  from commit `27d3b3d87c9a82cfb1c140a404b1ac2585d95dd4` is additionally preserved
  in `am-kernels/benchmarks/microbench/src/md5/LICENSE`. Do not treat this component as
  exclusively covered by the AM repository's top-level MIT notice.

The runtime, formatting subset, software arithmetic and platform overlay are
project-owned code under `workloads/am`. They are copied into an isolated build
workspace; the vendored upstream files are never patched in place. The adapter
runner includes the original runner under a renamed main, reuses its exact
allocator/RNG/checksum helpers, and uses the complete upstream benchmark list with the original test settings.
The upstream score-printing main is removed by link-time section GC; the local
runner adds ROI markers and strict per-item validation. No CoreMark or upstream
template klib implementation is linked.

QuickLZ (`am-kernels/benchmarks/microbench/src/lzip/quicklz.c` and `.h`) retains its original
copyright and GPL 1/2/3-or-commercial licensing notice; it is not covered solely
by the top-level MIT license. This import uses the GPL option.

To reproduce the import, clone https://github.com/NJU-ProjectN/am-kernels.git,
checkout `9c433f7cf4b4e05d09543bf7e269b23502d1b2b6`, and copy its complete tracked
file tree into `am-kernels/` (without the nested `.git` directory). Preserve the
local root README.md, MicroBench README instructions and separately supplied MD5 license.
`third_party/am-kernels/` is the maintained source snapshot; `target/` is only
build/cache space and is not needed to recover these sources. All upstream
component copyright and licensing notices are preserved; the root MIT notice
does not replace component-specific terms.
