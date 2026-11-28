#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
work=$(mktemp -d "${TMPDIR:-/tmp}/gem5rs-am.XXXXXXXX")
trap 'rm -rf -- "$work"' EXIT
work=$(cd "$work" && pwd -P)
unset PYTHONPATH PYTHONHOME PYTHONOPTIMIZE MYPYPATH
cd "$root"
bash tools/build-spike-adapter.sh
py="${VIRTUAL_ENV:?Run inside devenv shell}/bin/python"
maturin build --release --locked --interpreter "$py" --out "$work/dist"
wheels=("$work"/dist/*.whl)
[[ ${#wheels[@]} == 1 ]]
uv pip install --python "$py" --target "$work" --no-index --no-deps "${wheels[0]}"
cp tools/am_benchmark.py tests/e2e/am.py "$work/"
cd "$work"
printf '[mypy]\n' > mypy.ini
mypy --strict --python-executable "$py" -p gem5rs
mypy --strict --python-executable "$py" am_benchmark.py am.py
export GEM5RS_REPO_ROOT="$root"
"$py" -E -s am.py
"$py" -E -s am_benchmark.py --difftest --output "$root/target/am-results/acceptance.json"
