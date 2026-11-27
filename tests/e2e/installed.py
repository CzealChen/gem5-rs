"""Verify the package artifacts available to an installed consumer."""

import importlib.machinery
import importlib.util
import sys
from importlib import resources
from pathlib import Path

import gem5rs


def main() -> None:
    assert sys.prefix != sys.base_prefix, sys.prefix
    assert gem5rs.__file__ is not None
    package = Path(gem5rs.__file__).resolve().parent
    assert package == Path(__file__).resolve().parent / "gem5rs", package
    spec = importlib.util.find_spec("gem5rs._native")
    assert spec is not None and spec.origin is not None, spec
    assert isinstance(spec.loader, importlib.machinery.ExtensionFileLoader), spec
    assert Path(spec.origin).resolve().parent == package, spec.origin
    files = resources.files(gem5rs)
    for name in ("py.typed", "_native.pyi"):
        assert files.joinpath(name).is_file(), f"Wheel is missing gem5rs/{name}"
    print(f"Wheel verified: interpreter={sys.executable} package={package} native={spec.origin}")


if __name__ == "__main__":
    main()
