from __future__ import annotations

import io
import json
import os
import pathlib
import struct
import sys

ROOT = pathlib.Path(__file__).parents[1]
sys.dont_write_bytecode = True
sys.path.insert(0, str(ROOT / "adaq-python-research-runner/src"))
sys.path.insert(0, str(ROOT / "adaq-research-sdk/src"))
sys.path.insert(0, str(ROOT.parent / "examples/python/py-factor-cross-sectional-momentum/src"))

from adaq_runner.__main__ import PROTOCOL, _project_payload, run  # noqa: E402
from project import create_project  # noqa: E402


def frame(value: dict[str, object]) -> bytes:
    body = json.dumps(value, separators=(",", ":")).encode()
    return struct.pack(">I", len(body)) + body


def main() -> None:
    os.environ.update(
        {
            "ADAQ_EXPECTED_SDK_SHA256": "a" * 64,
            "ADAQ_EXPECTED_REVISION_SHA256": "b" * 64,
            "ADAQ_EXPECTED_ENVIRONMENT_SHA256": "c" * 64,
            "ADAQ_EXPECTED_ATTEMPT_ID": "attempt",
            "ADAQ_RUNNER_TOKEN": "x" * 32,
        }
    )
    hello = {
        "kind": "hello",
        "handshake": {
            "protocol": PROTOCOL,
            "sdkArtifactSha256": "a" * 64,
            "revisionSha256": "b" * 64,
            "environmentSha256": "c" * 64,
            "attemptId": "attempt",
            "loopback": True,
            "oneTimeToken": "x" * 32,
        },
    }
    output = io.BytesIO()
    assert run(io.BytesIO(frame(hello) + frame({"kind": "shutdown"})), output) == 0
    assert json.loads(output.getvalue()[4:]) == {"kind": "ready"}
    try:
        run(io.BytesIO(struct.pack(">I", 16 * 1024 * 1024 + 1)), io.BytesIO())
    except ValueError as error:
        assert str(error) == "runner-control-frame-too-large"
    else:
        raise AssertionError("oversized frames must fail closed")
    payload = _project_payload(create_project(), "factor")
    definition = payload["definition"]
    assert definition["scope"] == "cross-sectional"
    assert definition["outputs"] == ["momentum-score"]
    assert [node["op"] for node in definition["nodes"]] == [
        "market-close",
        "backward-simple-return",
        "cross-sectional-percentile",
        "rename",
    ]
    print("Runner contract checks: 3 passed")


if __name__ == "__main__":
    main()
