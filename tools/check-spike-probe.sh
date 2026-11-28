#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
spike_prefix=$(cd "$(dirname "$(command -v spike)")/.." && pwd -P)
mkdir -p target/spike
"${CXX:-c++}" -std=c++20 -Wall -Wextra -Werror \
  -isystem "$spike_prefix/include" tools/spike/probe.cc \
  -L"$spike_prefix/lib" -Wl,-rpath,"$spike_prefix/lib" -lriscv \
  -o target/spike/probe
GEM5RS_SPIKE_PROBE="$PWD/target/spike/probe" cargo test --locked --test spike_probe -- --ignored
