"""Minimal private Host/Runner framing loop.

The Host owns the socket, token, identities, process lifecycle, resource
policy, and result validation. This process only speaks bounded JSON frames.
"""

from __future__ import annotations

import json
import contextlib
import dataclasses
import enum
import importlib
import io
import os
import pathlib
import socket
import struct
import sys
import tomllib
from typing import BinaryIO

PROTOCOL = "adaq-python-runner@1"
MAX_FRAME = 16 * 1024 * 1024


def _read_frame(stream: BinaryIO) -> dict[str, object] | None:
    header = stream.read(4)
    if not header:
        return None
    if len(header) != 4:
        raise ValueError("runner-control-frame-truncated")
    length = struct.unpack(">I", header)[0]
    if length > MAX_FRAME:
        raise ValueError("runner-control-frame-too-large")
    body = stream.read(length)
    if len(body) != length:
        raise ValueError("runner-control-frame-truncated")
    value = json.loads(body)
    if not isinstance(value, dict) or not isinstance(value.get("kind"), str):
        raise ValueError("runner-control-message-invalid")
    return value


def _write_frame(stream: BinaryIO, value: dict[str, object]) -> None:
    body = json.dumps(value, separators=(",", ":"), sort_keys=True).encode()
    if len(body) > MAX_FRAME:
        raise ValueError("runner-control-message-too-large")
    stream.write(struct.pack(">I", len(body)))
    stream.write(body)
    stream.flush()


def _validate_handshake(value: dict[str, object]) -> None:
    expected = {
        "protocol": PROTOCOL,
        "sdkArtifactSha256": os.environ.get("ADAQ_EXPECTED_SDK_SHA256"),
        "revisionSha256": os.environ.get("ADAQ_EXPECTED_REVISION_SHA256"),
        "environmentSha256": os.environ.get("ADAQ_EXPECTED_ENVIRONMENT_SHA256"),
        "attemptId": os.environ.get("ADAQ_EXPECTED_ATTEMPT_ID"),
        "loopback": True,
        "oneTimeToken": os.environ.get("ADAQ_RUNNER_TOKEN"),
    }
    handshake = value.get("handshake")
    if value.get("kind") != "hello" or not isinstance(handshake, dict):
        raise ValueError("runner-handshake-rejected")
    if any(handshake.get(key) != expected_value for key, expected_value in expected.items()):
        raise ValueError("runner-handshake-rejected")
    token = expected["oneTimeToken"]
    if not isinstance(token, str) or len(token) < 32:
        raise ValueError("runner-handshake-token-invalid")


def _environment_handshake() -> dict[str, object]:
    values = {
        "protocol": PROTOCOL,
        "sdkArtifactSha256": os.environ.get("ADAQ_EXPECTED_SDK_SHA256"),
        "revisionSha256": os.environ.get("ADAQ_EXPECTED_REVISION_SHA256"),
        "environmentSha256": os.environ.get("ADAQ_EXPECTED_ENVIRONMENT_SHA256"),
        "attemptId": os.environ.get("ADAQ_EXPECTED_ATTEMPT_ID"),
        "loopback": True,
        "oneTimeToken": os.environ.get("ADAQ_RUNNER_TOKEN"),
    }
    if any(not isinstance(value, str) for value in values.values() if value is not True):
        raise ValueError("runner-handshake-environment-invalid")
    return values


def _execute_project(
    project_root: pathlib.Path, entry_point: str, sdk_wheel: str | None
) -> dict[str, object]:
    with (project_root / "adaq-project.toml").open("rb") as manifest_file:
        manifest = tomllib.load(manifest_file)
    project_id = manifest.get("project-id")
    project_kind = manifest.get("kind")
    if not all(isinstance(value, str) and value for value in (project_id, project_kind)):
        raise ValueError("runner-project-manifest-invalid")
    if manifest.get("entry-point") != entry_point:
        raise ValueError("runner-entry-point-mismatch")
    sys.path.insert(0, str(project_root / "src"))
    if sdk_wheel:
        sys.path.insert(0, sdk_wheel)
    module_name, separator, function_name = entry_point.partition(":")
    if not separator or not module_name or not function_name:
        raise ValueError("runner-entry-point-invalid")
    stdout = io.StringIO()
    stderr = io.StringIO()
    with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
        result = getattr(importlib.import_module(module_name), function_name)()
    payload: object | None
    if isinstance(result, dict):
        payload = result
    else:
        payload = _project_payload(result, project_kind)
    return {
        "attemptId": os.environ.get("ADAQ_EXPECTED_ATTEMPT_ID", ""),
        "projectId": project_id,
        "projectKind": project_kind,
        "entryPoint": entry_point,
        "output": (stdout.getvalue() + stderr.getvalue())[:4096],
        "payload": payload,
    }


def _project_payload(project: object, project_kind: str) -> object:
    if getattr(project, "kind", None) != project_kind:
        raise ValueError("runner-project-kind-mismatch")
    if project_kind == "factor":
        define = getattr(project, "define", None)
        if not callable(define):
            raise ValueError("runner-factor-definition-missing")
        return {"definition": _jsonable(define(None))}
    if project_kind == "model":
        return {
            "target": _jsonable(getattr(project, "target", None)),
            "signal": _jsonable(getattr(project, "signal", None)),
        }
    if project_kind == "strategy":
        return {"deferred": True}
    raise ValueError("runner-project-kind-unsupported")


def _jsonable(value: object) -> object:
    if isinstance(value, enum.Enum):
        return value.value
    if dataclasses.is_dataclass(value) and not isinstance(value, type):
        return {
            field.name: _jsonable(getattr(value, field.name))
            for field in dataclasses.fields(value)
        }
    if isinstance(value, dict):
        return {str(key): _jsonable(item) for key, item in value.items()}
    if isinstance(value, (list, tuple)):
        return [_jsonable(item) for item in value]
    if value is None or isinstance(value, (bool, int, float, str)):
        return value
    raise ValueError("runner-project-payload-not-serializable")


def run_socket(
    address: str,
    project_root: pathlib.Path,
    entry_point: str,
    sdk_wheel: str | None,
) -> int:
    host, separator, port = address.rpartition(":")
    if separator != ":" or host != "127.0.0.1":
        raise ValueError("runner-loopback-address-invalid")
    with socket.create_connection((host, int(port)), timeout=5) as connection:
        with connection.makefile("rwb", buffering=0) as stream:
            _write_frame(stream, {"kind": "hello", "handshake": _environment_handshake()})
            command = _read_frame(stream)
            if command is None or command.get("kind") != "execute":
                if command and command.get("kind") in {"cancel", "shutdown"}:
                    return 0
                raise ValueError("runner-execute-required")
            message = command
            while True:
                kind = message["kind"]
                if kind in {"cancel", "shutdown"}:
                    return 0
                if kind == "execute":
                    try:
                        result = _execute_project(project_root, entry_point, sdk_wheel)
                    except Exception as error:  # noqa: BLE001
                        _write_frame(
                            stream,
                            {
                                "kind": "diagnostic",
                                "code": "runner-execution-failed",
                                "message": str(error)[:4096],
                            },
                        )
                        return 1
                    output = result.pop("output")
                    if output:
                        _write_frame(
                            stream,
                            {
                                "kind": "diagnostic",
                                "code": "runner-project-output",
                                "message": output,
                            },
                        )
                    _write_frame(
                        stream, {"kind": "conformance-result", "result": result}
                    )
                    message = _read_frame(stream)
                    if message is None:
                        return 1
                    continue
                if kind not in {"progress", "diagnostic", "result"}:
                    raise ValueError("runner-control-message-kind-unknown")
                message = _read_frame(stream)
                if message is None:
                    return 1


def run(input_stream: BinaryIO | None = None, output_stream: BinaryIO | None = None) -> int:
    input_stream = input_stream or sys.stdin.buffer
    output_stream = output_stream or sys.stdout.buffer
    message = _read_frame(input_stream)
    if message is None:
        return 1
    _validate_handshake(message)
    _write_frame(output_stream, {"kind": "ready"})
    for message in iter(lambda: _read_frame(input_stream), None):
        kind = message["kind"]
        if kind == "cancel" or kind == "shutdown":
            return 0
        if kind not in {"progress", "diagnostic", "result"}:
            raise ValueError("runner-control-message-kind-unknown")
    return 0


if __name__ == "__main__":
    try:
        if "--connect" in sys.argv:
            def argument(name: str, required: bool = True) -> str | None:
                try:
                    value = sys.argv[sys.argv.index(name) + 1]
                except (ValueError, IndexError):
                    if required:
                        raise ValueError(f"runner-argument-missing:{name}")
                    return None
                return value

            raise SystemExit(
                run_socket(
                    argument("--connect") or "",
                    pathlib.Path(argument("--project-root") or ""),
                    argument("--entry-point") or "",
                    argument("--sdk-wheel", required=False),
                )
            )
        raise SystemExit(run())
    except (ValueError, json.JSONDecodeError) as error:
        raise SystemExit(str(error))
