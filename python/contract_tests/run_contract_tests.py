"""Managed SDK contract entry point used by CI and later M12 slices."""

from __future__ import annotations

import importlib.util
import hashlib
import pathlib
import sys
import tempfile
import zipfile


ROOT = pathlib.Path(__file__).parents[1]
sys.path.insert(0, str(ROOT / "adaq-research-sdk" / "src"))
TEST = pathlib.Path(__file__).with_name("test_sdk.py")
spec = importlib.util.spec_from_file_location("adaq_sdk_contract_test", TEST)
if spec is None or spec.loader is None:
    raise SystemExit("cannot load SDK contract tests")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
tests = sorted(name for name in dir(module) if name.startswith("test_"))
for name in tests:
    getattr(module, name)()
    print(f"PASS {name}")


def load_backend(path: pathlib.Path, name: str):
    backend_spec = importlib.util.spec_from_file_location(name, path)
    if backend_spec is None or backend_spec.loader is None:
        raise SystemExit(f"cannot load package backend: {path}")
    backend = importlib.util.module_from_spec(backend_spec)
    backend_spec.loader.exec_module(backend)
    return backend


def test_managed_wheels_are_deterministic_and_separated() -> None:
    sdk_backend = load_backend(
        ROOT / "adaq-research-sdk" / "adaq_build_backend.py", "adaq_sdk_backend"
    )
    runner_backend = load_backend(
        ROOT / "adaq-python-research-runner" / "adaq_runner_build_backend.py",
        "adaq_runner_backend",
    )
    adapter_backend = load_backend(
        ROOT / "adaq-qlib-ridge-adapter" / "adaq_qlib_adapter_build_backend.py",
        "adaq_qlib_adapter_backend",
    )
    with tempfile.TemporaryDirectory() as directory:
        first = pathlib.Path(directory) / "first"
        second = pathlib.Path(directory) / "second"
        third = pathlib.Path(directory) / "third"
        first.mkdir()
        second.mkdir()
        third.mkdir()
        sdk_backend.build_wheel(str(first))
        sdk_backend.build_wheel(str(second))
        first_bytes = next(first.glob("*.whl")).read_bytes()
        second_bytes = next(second.glob("*.whl")).read_bytes()
        assert first_bytes == second_bytes
        assert hashlib.sha256(first_bytes).hexdigest()
        with zipfile.ZipFile(first / "adaq_research_sdk-1.0.0-py3-none-any.whl") as archive:
            names = archive.namelist()
            assert all("runner" not in name.lower() for name in names)
        runner_backend.build_wheel(str(second))
        runner_wheel = next(
            path for path in second.glob("*.whl") if "runner" in path.name
        )
        with zipfile.ZipFile(runner_wheel) as archive:
            assert "adaq_runner/__main__.py" in archive.namelist()
        adapter_backend.build_wheel(str(third))
        adapter_wheel = next(third.glob("*.whl"))
        with zipfile.ZipFile(adapter_wheel) as archive:
            assert "adaq_qlib_adapter/__init__.py" in archive.namelist()
            metadata = archive.read(
                "adaq_qlib_ridge_adapter-1.0.0.dist-info/METADATA"
            ).decode()
            assert "Name: adaq-qlib-ridge-adapter" in metadata
            assert "Requires-Python: >=3.12,<3.13" in metadata


test_managed_wheels_are_deterministic_and_separated()
print(f"SDK and managed wheel contract checks: {len(tests) + 1} passed")
