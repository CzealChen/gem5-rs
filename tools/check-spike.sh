#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
bash tools/build-spike-adapter.sh
GEM5RS_SPIKE_ADAPTER="$PWD/target/spike/adapter" cargo test --locked --test spike_difftest -- --include-ignored
