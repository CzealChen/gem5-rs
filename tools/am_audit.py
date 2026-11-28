"""Compile AM kernels (except NEMU) and audit RV64I objects without simulation."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
from pathlib import Path
from typing import cast

ROOT = Path(__file__).resolve().parents[1]


def object_reports(value: object) -> list[dict[str, object]]:
    if not isinstance(value, list):
        raise TypeError("expected object report list")
    result: list[dict[str, object]] = []
    for item in value:
        if not isinstance(item, dict) or not all(isinstance(k, str) for k in item):
            raise TypeError("expected string-keyed object report")
        result.append(cast(dict[str, object], item))
    return result


def names(value: object) -> set[str]:
    if not isinstance(value, list) or not all(isinstance(v, str) for v in value):
        raise TypeError("expected symbol names")
    return {str(v) for v in value}


def main() -> None:
    work = ROOT / "target/am-audit"
    work.mkdir(parents=True, exist_ok=True)
    stage = work / "sources"
    if stage.exists():
        shutil.rmtree(stage)
    am = stage / "abstract-machine"
    shutil.copytree(ROOT / "third_party/abstract-machine", am)
    (am / "scripts").mkdir()
    shutil.copyfile(
        ROOT / "workloads/am/riscv64-gem5rs.mk", am / "scripts/riscv64-gem5rs.mk"
    )
    kernels = stage / "kernels"
    shutil.copytree(ROOT / "third_party/am-kernels/kernels", kernels)
    subprocess.run(
        ["cargo", "build", "--locked", "--bin", "audit-instructions"],
        cwd=ROOT,
        check=True,
    )
    env = os.environ.copy()
    for name in (
        "CFLAGS",
        "CPPFLAGS",
        "CXXFLAGS",
        "ASFLAGS",
        "LDFLAGS",
        "MAKEFLAGS",
        "LIBS",
        "CROSS_COMPILE",
    ):
        env.pop(name, None)
    reports: list[dict[str, object]] = []
    for kernel in sorted(kernels.iterdir()):
        if not kernel.is_dir() or kernel.name == "nemu":
            continue
        command = [
            "make",
            "-k",
            "-f",
            "Makefile",
            "-f",
            str(ROOT / "workloads/am/audit.mk"),
            "audit-objects",
            "ARCH=riscv64-gem5rs",
            f"AM_HOME={am}",
        ]
        compiled = subprocess.run(
            command,
            check=False,
            cwd=kernel,
            env=env,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
        )
        log = work / f"{kernel.name}.log"
        log.write_text(compiled.stdout)
        objects = sorted(kernel.glob("build/**/*.o"))
        output = subprocess.check_output(
            [str(ROOT / "target/debug/audit-instructions"), *map(str, objects)],
            text=True,
        )
        decoded: object = json.loads(output)
        units = object_reports(decoded)
        defined: set[str] = set()
        undefined: set[str] = set()
        for unit in units:
            defined.update(names(unit["defined"]))
            undefined.update(names(unit["undefined"]))
        findings = [
            finding for unit in units for finding in object_reports(unit["findings"])
        ]
        report: dict[str, object] = {
            "kernel": kernel.name,
            "compilation_complete": compiled.returncode == 0,
            "compile_command": command,
            "compile_log": str(log.relative_to(ROOT)),
            "excluded_assembly": [
                str(p.relative_to(kernel)) for p in sorted(kernel.rglob("*.S"))
            ],
            "external_dependencies": sorted(undefined - defined),
            "objects": units,
        }
        reports.append(report)
        print(
            f"{kernel.name}: compile={'ok' if compiled.returncode == 0 else 'incomplete'}, objects={len(objects)}, instruction_findings={len(findings)}"
        )
    report_path = work / "report.json"
    report_path.write_text(
        json.dumps(
            {
                "isa": "rv64i",
                "abi": "lp64",
                "optimization": "O2",
                "compiler": subprocess.check_output(
                    ["riscv64-none-elf-gcc", "--version"], text=True
                ).strip(),
                "scope": "C/C++ objects only; no linking, runtime/device implementation or execution; all text sections, including potentially dead code",
                "excluded_kernels": ["nemu"],
                "kernels": reports,
            },
            indent=2,
        )
        + "\n"
    )
    print(f"Report: {report_path}")


if __name__ == "__main__":
    main()
