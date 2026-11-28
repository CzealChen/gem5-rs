#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
spike_prefix=$(cd "$(dirname "$(command -v spike)")/.." && pwd -P)
# The devenv.lock recipe pins this package and its source commit. Reject an
# unrelated PATH install rather than mislabelling its reference identity.
case "$spike_prefix" in
  /nix/store/*-spike-1.1.0-unstable-2026-10-20) ;;
  *) printf 'Expected the locked devenv Spike package, got %s\n' "$spike_prefix" >&2; exit 1 ;;
esac
mkdir -p target/spike
"${CXX:-c++}" -std=c++20 -O2 -Wall -Wextra -Werror \
  '-DSPIKE_SOURCE_COMMIT="d140cbc0ae4ef4fc64ad323f3f175ff15f6dca56"' \
  -isystem "$spike_prefix/include" tools/spike/adapter.cc \
  -L"$spike_prefix/lib" -Wl,-rpath,"$spike_prefix/lib" -lriscv \
  -o target/spike/adapter
