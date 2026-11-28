"""Check the assembly examples against the selected Python interpreter's package."""

import argparse
import re
import subprocess
import sys
from pathlib import Path
from tempfile import TemporaryDirectory


def _mypy(
    executable: str, interpreter: str, source: Path
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [
            executable,
            "--strict",
            "--no-incremental",  # Fixtures are rewritten in place between checks.
            "--python-executable",
            interpreter,
            "--no-error-summary",
            "--show-error-codes",
            str(source),
        ],
        capture_output=True,
        text=True,
        check=False,
    )


def _check_clean(run: subprocess.CompletedProcess[str]) -> None:
    if run.returncode != 0:
        raise SystemExit(run.stdout + run.stderr)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mypy", default="mypy")
    parser.add_argument("--python-executable", default=sys.executable)
    options = parser.parse_args()
    mypy: str = options.mypy
    interpreter: str = options.python_executable
    fixtures = Path(__file__).resolve().parent
    _check_clean(_mypy(mypy, interpreter, fixtures / "valid.py"))

    source = (fixtures / "connections.py.case").read_text(encoding="utf-8")
    expected = {
        number: line.split("# error: ", 1)[1]
        for number, line in enumerate(source.splitlines(), start=1)
        if "# error: " in line
    }
    with TemporaryDirectory() as work:
        case = Path(work) / "connections.py"
        case.write_text(source, encoding="utf-8")
        run = _mypy(mypy, interpreter, case)
        remaining = expected.copy()
        for line in run.stdout.splitlines():
            match = re.fullmatch(
                r".*connections\.py:(\d+): error: (.*?)\s+\[arg-type\]", line
            )
            if match is None:
                raise SystemExit(run.stdout + run.stderr)
            number = int(match[1])
            if remaining.pop(number, None) != match[2]:
                raise SystemExit(run.stdout + run.stderr)
        if run.returncode != 1 or remaining or run.stderr:
            raise SystemExit(
                f"Expected argument diagnostics on lines {list(expected)}:\n{run.stdout}{run.stderr}"
            )

        corrected = source.replace("latency=Address(10)", "latency=Tick(10)")
        corrected = corrected.replace(
            "one.requests, two.requests", "one.requests, ram.input"
        )
        corrected = corrected.replace(
            "router.inputs[0], banks.input", "router.outputs[0], banks.input"
        )
        corrected = corrected.replace(
            "one.requests, console)", "one.requests, console.input)"
        )
        corrected = corrected.replace('machine_mode="yes"', "machine_mode=True")
        corrected = corrected.replace(
            "connect_interrupt(core.requests,", "connect_interrupt(timer.interrupt,"
        )
        corrected = corrected.replace(
            "connect(timer.interrupt,", "connect(core.requests,"
        )
        case.write_text(corrected, encoding="utf-8")
        _check_clean(_mypy(mypy, interpreter, case))

    print(
        f"Typing checks passed: assembly, {len(expected)} diagnostics, and corrected assembly"
    )


if __name__ == "__main__":
    main()
