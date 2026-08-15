"""Minimal private Host/Runner framing loop.

The Host owns the socket, token, identities, process lifecycle, resource
policy, and result validation. This process only speaks bounded JSON frames.
"""

from __future__ import annotations

import json
import contextlib
import dataclasses
import enum
import hashlib
import importlib
import os
import pathlib
import random
import socket
import struct
import sys
import tomllib
from typing import BinaryIO

PROTOCOL = "adaq-python-runner@1"
MAX_FRAME = 16 * 1024 * 1024


class _HostArrowTable:
    def __init__(self, rows: list[dict[str, object]]) -> None:
        self._rows = rows

    def to_pylist(self) -> list[dict[str, object]]:
        return [dict(row) for row in self._rows]


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
    body = json.dumps(
        value,
        separators=(",", ":"),
        sort_keys=True,
        ensure_ascii=False,
        allow_nan=False,
    ).encode()
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
    if set(handshake) != set(expected):
        raise ValueError("runner-handshake-rejected")
    if any(handshake.get(key) != expected_value for key, expected_value in expected.items()):
        raise ValueError("runner-handshake-rejected")
    if any(
        not isinstance(expected[key], str) or len(expected[key]) != 64
        or any(character not in "0123456789abcdef" for character in expected[key])
        for key in ("sdkArtifactSha256", "revisionSha256", "environmentSha256")
    ):
        raise ValueError("runner-handshake-identities-invalid")
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
    if any(
        len(values[key]) != 64
        or any(character not in "0123456789abcdef" for character in values[key])
        for key in ("sdkArtifactSha256", "revisionSha256", "environmentSha256")
    ):
        raise ValueError("runner-handshake-identities-invalid")
    return values


def _factor_payload(
    project_factory: object,
    project_kind: str,
    project_root: pathlib.Path,
    execution: dict[str, object],
) -> dict[str, object]:
    mode = _read_manifest(project_root).get("mode")
    if mode == "portable-definition":
        project = project_factory()
        if getattr(project, "kind", None) != project_kind:
            raise ValueError("runner-project-kind-mismatch")
        define = getattr(project, "define", None)
        if not callable(define):
            raise ValueError("runner-factor-definition-missing")
        return {"definition": _jsonable(define(None))}
    if mode != "imperative-python":
        raise ValueError("runner-factor-mode-unsupported")
    input_value = execution.get("input")
    if not isinstance(input_value, dict):
        raise ValueError("runner-factor-input-missing")
    universe = input_value.get("universe")
    segments = input_value.get("segments")
    if not isinstance(universe, list) or not all(isinstance(item, str) for item in universe):
        raise ValueError("runner-factor-input-universe-invalid")
    if not isinstance(segments, list) or not segments:
        raise ValueError("runner-factor-input-segments-invalid")
    from adaq import FactorContext, Parameter

    parameters = tuple(
        Parameter(id=key, value=value)
        for key, value in sorted(execution.get("parameters", {}).items())
    )
    outputs = []
    for segment in segments:
        if not isinstance(segment, dict):
            raise ValueError("runner-factor-input-segment-invalid")
        segment_id = segment.get("segmentId")
        batches = segment.get("batches")
        if not isinstance(segment_id, str) or not segment_id:
            raise ValueError("runner-factor-input-segment-invalid")
        if not isinstance(batches, list) or not batches:
            raise ValueError("runner-factor-input-batches-invalid")
        project = project_factory()
        if getattr(project, "kind", None) != project_kind:
            raise ValueError("runner-project-kind-mismatch")
        evaluate = getattr(project, "evaluate", None)
        if not callable(evaluate):
            raise ValueError("runner-factor-evaluator-missing")
        public_batches = []
        for batch in batches:
            if not isinstance(batch, dict) or not isinstance(batch.get("rows"), list):
                raise ValueError("runner-factor-input-batch-invalid")
            rows = []
            for row in batch["rows"]:
                if not isinstance(row, dict):
                    raise ValueError("runner-factor-input-row-invalid")
                rows.append(
                    {
                        "instrumentId": row.get("instrumentId"),
                        "eventTimeMs": row.get("observationTimeMs"),
                        "inputs": {"close": row.get("close")},
                    }
                )
            public_batches.append({"segmentId": segment_id, "rows": rows})
        context = FactorContext(
            parameters=parameters,
            seed=execution.get("seed", 0),
            inputs={"universe": tuple(universe), "segmentId": segment_id},
        )
        result = evaluate(context, public_batches)
        outputs.extend(_jsonable(list(result)))
    return {"output_names": list(execution.get("outputNames", [])), "outputs": outputs}


def _read_manifest(project_root: pathlib.Path) -> dict[str, object]:
    with (project_root / "adaq-project.toml").open("rb") as manifest_file:
        return tomllib.load(manifest_file)


def _execute_project(
    project_root: pathlib.Path,
    entry_point: str,
    sdk_wheel: str | None,
    adapter_wheel: str | None,
    execution: dict[str, object],
) -> dict[str, object]:
    manifest = _read_manifest(project_root)
    project_id = manifest.get("project-id")
    project_kind = manifest.get("kind")
    if not all(isinstance(value, str) and value for value in (project_id, project_kind)):
        raise ValueError("runner-project-manifest-invalid")
    if manifest.get("entry-point") != entry_point:
        raise ValueError("runner-entry-point-mismatch")
    sys.path.insert(0, str(project_root / "src"))
    if sdk_wheel:
        sys.path.insert(0, sdk_wheel)
    if adapter_wheel:
        sys.path.insert(0, adapter_wheel)
    module_name, separator, function_name = entry_point.partition(":")
    if not separator or not module_name or not function_name:
        raise ValueError("runner-entry-point-invalid")
    with contextlib.redirect_stdout(sys.stderr):
        project_factory = getattr(importlib.import_module(module_name), function_name)
        if not callable(project_factory):
            raise ValueError("runner-entry-point-not-callable")
        if project_kind == "factor":
            payload = _factor_payload(project_factory, project_kind, project_root, execution)
            result = None
        elif project_kind == "model":
            result = project_factory()
            input_value = execution.get("input")
            if not isinstance(input_value, dict):
                raise ValueError("runner-model-input-missing")
            from adaq import ModelArtifact, ModelContext, Parameter
            from adaq.qlib import DatasetH

            transformation = input_value.get("transformation")
            if not isinstance(transformation, dict):
                raise ValueError("runner-model-transformation-missing")
            feature_names = transformation.get("featureNames")
            if not isinstance(feature_names, list) or not all(
                isinstance(name, str) for name in feature_names
            ):
                raise ValueError("runner-model-feature-schema-invalid")

            def public_rows(
                name: str, labels: tuple[object, ...] | None = None
            ) -> list[dict[str, object]]:
                result = []
                source_rows = input_value.get(name, [])
                if not isinstance(source_rows, list):
                    raise ValueError("runner-model-rows-invalid")
                if labels is not None and len(labels) != len(source_rows):
                    raise ValueError("runner-model-label-count-invalid")
                for index, row in enumerate(source_rows):
                    if not isinstance(row, dict) or not isinstance(row.get("features"), list):
                        raise ValueError("runner-model-row-invalid")
                    if len(row["features"]) != len(feature_names):
                        raise ValueError("runner-model-row-dimension-invalid")
                    result.append(
                        {
                            "datetime": row.get("datetime"),
                            "instrument": row.get("instrument"),
                            **dict(zip(feature_names, row["features"])),
                        }
                    )
                    if labels is not None:
                        result[-1]["label"] = labels[index]
                return result

            labels = {}
            for name in ("trainLabels", "validLabels"):
                source_labels = input_value.get(name, [])
                if not isinstance(source_labels, list):
                    raise ValueError("runner-model-labels-invalid")
                labels[name] = tuple(source_labels)
            dataset = DatasetH.from_arrow(
                {
                    "train": _HostArrowTable(
                        public_rows("train", labels["trainLabels"])
                    ),
                    "valid": _HostArrowTable(
                        public_rows("valid", labels["validLabels"])
                    ),
                    "test": _HostArrowTable(public_rows("test")),
                }
            )
            context_inputs = {
                "dataset": dataset,
                "transformation": transformation,
            }
            if isinstance(input_value.get("targetWindowEnd"), int):
                context_inputs["targetWindowEnd"] = input_value["targetWindowEnd"]
            context = ModelContext(
                parameters=tuple(
                    Parameter(id=key, value=value)
                    for key, value in sorted(execution.get("parameters", {}).items())
                ),
                seed=execution.get("seed", 0),
                inputs=context_inputs,
            )
            fitted_model = input_value.get("fittedModel")
            if fitted_model is None:
                fitted = result.fit(context)
                payload = {
                    "target": _jsonable(getattr(result, "target", None)),
                    "signal": _jsonable(getattr(result, "signal", None)),
                    "fit": _jsonable(fitted),
                }
            else:
                if not isinstance(fitted_model, dict):
                    raise ValueError("runner-model-fitted-artifact-invalid")
                fitted = ModelArtifact(
                    schema=fitted_model.get("schema"),
                    payload=fitted_model.get("payload"),
                )
                if not isinstance(fitted.schema, str) or not isinstance(fitted.payload, dict):
                    raise ValueError("runner-model-fitted-artifact-invalid")
                forecasts = result.predict(context, fitted)
                payload = {
                    "target": _jsonable(getattr(result, "target", None)),
                    "signal": _jsonable(getattr(result, "signal", None)),
                    "forecast": _jsonable(list(forecasts)),
                }
        else:
            result = project_factory()
            payload = None
    if payload is None:
        if isinstance(result, dict):
            payload = result
        else:
            payload = _project_payload(result, project_kind)
    return {
        "attemptId": os.environ.get("ADAQ_EXPECTED_ATTEMPT_ID", ""),
        "projectId": project_id,
        "projectKind": project_kind,
        "entryPoint": entry_point,
        "payload": payload,
    }


def _write_staged_result(
    result: dict[str, object], result_path: pathlib.Path, result_name: str
) -> dict[str, object]:
    body = json.dumps(
        result,
        separators=(",", ":"),
        sort_keys=True,
        ensure_ascii=False,
        allow_nan=False,
    ).encode()
    max_bytes = int(os.environ.get("ADAQ_MAX_ARTIFACT_BYTES", "134217728"))
    if len(body) > max_bytes:
        raise ValueError("runner-artifact-too-large")
    result_path.parent.mkdir(parents=True, exist_ok=True)
    result_path.write_bytes(body)
    return {
        "attemptId": os.environ.get("ADAQ_EXPECTED_ATTEMPT_ID", ""),
        "relativePath": result_name,
        "mediaType": "application/json",
        "byteSize": len(body),
        "sha256": hashlib.sha256(body).hexdigest(),
    }


def _validate_execution(value: object, entry_point: str) -> int:
    required = {
        "sdkArtifactSha256",
        "runtimeArtifactSha256",
        "entryPoint",
        "inputBindings",
        "parameters",
        "seed",
        "outputNames",
    }
    if (
        not isinstance(value, dict)
        or not required.issubset(value)
        or set(value) - required - {"input"}
    ):
        raise ValueError("runner-execution-contract-invalid")
    if value["entryPoint"] != entry_point:
        raise ValueError("runner-execution-entry-point-mismatch")
    if (
        not isinstance(value["sdkArtifactSha256"], str)
        or value["sdkArtifactSha256"]
        != os.environ.get("ADAQ_EXPECTED_SDK_SHA256")
        or not isinstance(value["runtimeArtifactSha256"], str)
        or len(value["runtimeArtifactSha256"]) != 64
        or any(character not in "0123456789abcdef" for character in value["runtimeArtifactSha256"])
    ):
        raise ValueError("runner-execution-identity-invalid")
    if (
        not isinstance(value["inputBindings"], dict)
        or not all(
            isinstance(key, str) and isinstance(item, str)
            for key, item in value["inputBindings"].items()
        )
        or not isinstance(value["parameters"], dict)
        or not all(
            isinstance(key, str) and isinstance(item, str)
            for key, item in value["parameters"].items()
        )
        or not isinstance(value["outputNames"], list)
        or not all(isinstance(item, str) for item in value["outputNames"])
    ):
        raise ValueError("runner-execution-contract-invalid")
    seed = value["seed"]
    if isinstance(seed, bool) or not isinstance(seed, int) or seed < 0:
        raise ValueError("runner-execution-seed-invalid")
    return seed


def _apply_resource_policy(seed: int) -> None:
    random.seed(seed)
    try:
        import numpy

        numpy.random.seed(seed % (2**32))
    except (ImportError, ValueError):
        pass
    try:
        import resource
    except ImportError:
        # Windows has no stdlib resource module; the Host still enforces wall,
        # protocol, artifact, log, and process-tree limits.
        return
    memory = int(os.environ["ADAQ_MAX_MEMORY_BYTES"])
    try:
        _, memory_hard = resource.getrlimit(resource.RLIMIT_AS)
        if memory_hard != resource.RLIM_INFINITY:
            memory = min(memory, memory_hard)
        resource.setrlimit(resource.RLIMIT_AS, (memory, memory))
    except (ValueError, OSError):
        # Some Unix hosts reject lowering an unlimited address-space limit.
        pass
    processes = int(os.environ["ADAQ_MAX_PROCESSES"])
    if hasattr(resource, "RLIMIT_NPROC"):
        current, hard = resource.getrlimit(resource.RLIMIT_NPROC)
        # RLIMIT_NPROC is per-user, so lowering it below the already-running
        # process count is invalid; the Host owns process-tree termination.
        if current != resource.RLIM_INFINITY and processes >= current:
            if hard != resource.RLIM_INFINITY:
                processes = min(processes, hard)
            resource.setrlimit(resource.RLIMIT_NPROC, (processes, processes))


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
    adapter_wheel: str | None,
    result_path: pathlib.Path,
    result_name: str,
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
            seed = _validate_execution(command.get("execution"), entry_point)
            _apply_resource_policy(seed)
            message = command
            while True:
                kind = message["kind"]
                if kind in {"cancel", "shutdown"}:
                    return 0
                if kind == "execute":
                    try:
                        result = _execute_project(
                            project_root,
                            entry_point,
                            sdk_wheel,
                            adapter_wheel,
                            command.get("execution", {}),
                        )
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
                    artifact = _write_staged_result(result, result_path, result_name)
                    _write_frame(stream, {"kind": "artifact", "artifact": artifact})
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
                    argument("--adapter-wheel", required=False),
                    pathlib.Path(argument("--result-path") or ""),
                    argument("--result-name", required=False) or "conformance-result.json",
                )
            )
        raise SystemExit(run())
    except (ValueError, json.JSONDecodeError) as error:
        raise SystemExit(str(error))
