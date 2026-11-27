#!/usr/bin/env bash
set -euo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
work=$(mktemp -d "${TMPDIR:-/tmp}/gem5rs-mvp.XXXXXXXX")
trap 'rm -rf -- "$work"' EXIT
work=$(cd -- "$work" && pwd -P)
unset PYTHONPATH PYTHONHOME PYTHONOPTIMIZE MYPYPATH

cd "$root"
cargo fmt --all -- --check
cargo clippy --locked --all-features --all-targets -- -D warnings
cargo nextest run --locked --all-features --all-targets
cargo test --locked --all-features --doc
ruff check python tests tools workloads
ruff format --check python tests tools workloads
mypy --strict -m tools.spike_trace -m workloads.test_spike_trace
py="${VIRTUAL_ENV:?Run inside devenv shell}/bin/python"
maturin build --locked --interpreter "$py" --out "$work/dist"
wheels=("$work"/dist/*.whl)
if [[ ${#wheels[@]} != 1 || ! -f ${wheels[0]} ]]; then
    printf 'Expected one gem5rs wheel in %s/dist\n' "$work" >&2
    exit 1
fi

cd "$work"
# The scripts and mypy resolve the wheel from their own working directory.
uv pip install --python "$py" --target "$work" --no-index --no-deps "${wheels[0]}"
cp -- "$root"/tests/typing/{check.py,valid.py,connections.py.case} .
cp -- "$root"/tests/e2e/*.py "$root/tools/spike_trace.py" .
printf '[mypy]\n' > mypy.ini
mypy --strict --python-executable "$py" -p gem5rs
mypy --strict --python-executable "$py" check.py installed.py traffic.py bare_metal.py spike_trace.py
"$py" -E -s installed.py
"$py" -E -s check.py --python-executable "$py"
"$py" -E -s traffic.py

bash "$root/workloads/build.sh" "$work/elf"
(cd "$root" && GEM5RS_ELF_DIR="$work/elf" "$py" -E -s -m unittest workloads.test_spike_trace)
"$py" -E -s bare_metal.py "$work/elf"
