"""Deterministic stdlib-only wheel backend for the managed public SDK."""

from __future__ import annotations

import base64
import hashlib
import pathlib
import zipfile


NAME = "adaq_research_sdk"
VERSION = "1.0.0"
DIST_INFO = f"{NAME}-{VERSION}.dist-info"


def _files(root: pathlib.Path) -> list[pathlib.Path]:
    return sorted(
        path
        for path in (root / "src").rglob("*")
        if path.is_file() and "__pycache__" not in path.parts and path.suffix not in {".pyc", ".pyo"}
    )


def _metadata() -> dict[str, bytes]:
    return {
        f"{DIST_INFO}/METADATA": (
            "Metadata-Version: 2.3\n"
            "Name: adaq-research-sdk\n"
            "Version: 1.0.0\n"
            "Summary: Public ADAQ Python Research contracts\n"
            "Requires-Python: >=3.12,<3.13\n"
            "License-Expression: Apache-2.0\n\n"
        ).encode(),
        f"{DIST_INFO}/WHEEL": (
            "Wheel-Version: 1.0\n"
            "Generator: adaq-managed-sdk\n"
            "Root-Is-Purelib: true\n"
            "Tag: py3-none-any\n\n"
        ).encode(),
    }


def _wheel(wheel_directory: str, metadata_directory: str | None = None) -> str:
    root = pathlib.Path(__file__).parent
    output = pathlib.Path(wheel_directory) / f"{NAME}-{VERSION}-py3-none-any.whl"
    payload: dict[str, bytes] = _metadata()
    for path in _files(root):
        payload[path.relative_to(root / "src").as_posix()] = path.read_bytes()
    records = []
    for name, content in sorted(payload.items()):
        digest = base64.urlsafe_b64encode(hashlib.sha256(content).digest()).rstrip(b"=").decode()
        records.append(f"{name},sha256={digest},{len(content)}")
    records.append(f"{DIST_INFO}/RECORD,,")
    payload[f"{DIST_INFO}/RECORD"] = ("\n".join(records) + "\n").encode()
    with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for name, content in sorted(payload.items()):
            info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o644 << 16
            archive.writestr(info, content)
    return output.name


def build_wheel(wheel_directory: str, config_settings: object = None, metadata_directory: str | None = None) -> str:
    return _wheel(wheel_directory, metadata_directory)


def prepare_metadata_for_build_wheel(metadata_directory: str, config_settings: object = None) -> str:
    destination = pathlib.Path(metadata_directory) / DIST_INFO
    destination.mkdir(parents=True, exist_ok=True)
    for name, content in _metadata().items():
        (destination / pathlib.Path(name).name).write_bytes(content)
    return DIST_INFO

def build_sdist(*_args: object, **_kwargs: object) -> str:
    raise RuntimeError("the ADAQ-managed Wheelhouse builds this SDK")
