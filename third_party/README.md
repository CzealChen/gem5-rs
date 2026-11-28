# Vendored AM build and benchmark inputs

All imported files are byte-for-byte copies, except for the additional MD5
license file and this provenance note. No downloaded build artifacts are stored.

- `abstract-machine`: NJU-ProjectN/abstract-machine commit
  `068dc8cc150c63388fc5f457ff7b7dd70c5dd0f5`. Contains the upstream Makefile,
  AM/klib archive Makefiles, and only the headers needed by this platform.
  The upstream MIT license is preserved in `abstract-machine/LICENSE`.
- `am-kernels/microbench`: NJU-ProjectN/am-kernels commit
  `9c433f7cf4b4e05d09543bf7e269b23502d1b2b6`. Contains the benchmark header,
  original runner/helpers, Makefile and all ten C/C++ benchmark sources.
  `am-kernels/LICENSE` preserves its upstream MIT notice.
- MD5's source header attributes github.com/pod32g/md5. Its Apache-2.0 license
  from commit `27d3b3d87c9a82cfb1c140a404b1ac2585d95dd4` is additionally preserved
  in `am-kernels/microbench/src/md5/LICENSE`. Do not treat this component as
  exclusively covered by the AM repository's top-level MIT notice.

The runtime, formatting subset, software arithmetic and platform overlay are
project-owned code under `workloads/am`. They are copied into an isolated build
workspace; the vendored upstream files are never patched in place. The adapter
runner includes the original runner under a renamed main, reuses its exact
allocator/RNG/checksum helpers, and uses the complete upstream benchmark list with the original test settings.
The upstream score-printing main is removed by link-time section GC; the local
runner adds ROI markers and strict per-item validation. No CoreMark or upstream
template klib implementation is linked.

QuickLZ (`microbench/src/lzip/quicklz.c` and `.h`) retains its original
copyright and GPL 1/2/3-or-commercial licensing notice; it is not covered solely
by the top-level MIT license. This import uses the GPL option.

To reproduce the import, clone https://github.com/NJU-ProjectN/am-kernels.git,
checkout `9c433f7cf4b4e05d09543bf7e269b23502d1b2b6`, and copy
`benchmarks/microbench/` here. Preserve the separately supplied MD5 license.
The working clone lives in ignored `target/am-src/am-kernels`; builds use the
tracked copy so they need no network access.
