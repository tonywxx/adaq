"""Deterministic stdlib-only wheel backend for the managed Qlib adapter."""

from __future__ import annotations

import base64
import hashlib
import pathlib
import zipfile


NAME = "adaq_qlib_ridge_adapter"
VERSION = "1.0.0"
DIST_INFO = f"{NAME}-{VERSION}.dist-info"


def _wheel(wheel_directory: str) -> str:
    root = pathlib.Path(__file__).parent
    payload = {
        "adaq_qlib_adapter/__init__.py": (
            root / "src/adaq_qlib_adapter/__init__.py"
        ).read_bytes(),
        f"{DIST_INFO}/METADATA": (
            "Metadata-Version: 2.3\n"
            "Name: adaq-qlib-ridge-adapter\n"
            "Version: 1.0.0\n"
            "Summary: Managed Host-fed Qlib Ridge Adapter contract\n"
            "Requires-Python: >=3.12,<3.13\n"
            "License-Expression: Apache-2.0\n\n"
        ).encode(),
        f"{DIST_INFO}/WHEEL": (
            "Wheel-Version: 1.0\n"
            "Generator: adaq-managed-qlib-ridge-adapter\n"
            "Root-Is-Purelib: true\n"
            "Tag: py3-none-any\n\n"
        ).encode(),
    }
    records = []
    for name, content in sorted(payload.items()):
        digest = base64.urlsafe_b64encode(hashlib.sha256(content).digest()).rstrip(b"=").decode()
        records.append(f"{name},sha256={digest},{len(content)}")
    records.append(f"{DIST_INFO}/RECORD,,")
    payload[f"{DIST_INFO}/RECORD"] = ("\n".join(records) + "\n").encode()
    output = pathlib.Path(wheel_directory) / f"{NAME}-{VERSION}-py3-none-any.whl"
    with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for name, content in sorted(payload.items()):
            info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o644 << 16
            archive.writestr(info, content)
    return output.name


def build_wheel(
    wheel_directory: str,
    config_settings: object = None,
    metadata_directory: str | None = None,
) -> str:
    return _wheel(wheel_directory)


def prepare_metadata_for_build_wheel(
    metadata_directory: str,
    config_settings: object = None,
) -> str:
    destination = pathlib.Path(metadata_directory) / DIST_INFO
    destination.mkdir(parents=True, exist_ok=True)
    (destination / "METADATA").write_text(
        "Metadata-Version: 2.3\nName: adaq-qlib-ridge-adapter\nVersion: 1.0.0\n\n",
        encoding="utf-8",
    )
    (destination / "WHEEL").write_text(
        "Wheel-Version: 1.0\nRoot-Is-Purelib: true\nTag: py3-none-any\n\n",
        encoding="utf-8",
    )
    return DIST_INFO


def build_sdist(*_args: object, **_kwargs: object) -> str:
    raise RuntimeError("ADAQ managed Qlib Ridge Adapter is wheel-only")
