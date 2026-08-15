from __future__ import annotations

import importlib
import inspect
import pathlib
import sys


ROOT = pathlib.Path(__file__).parents[1]
sys.path.insert(0, str(ROOT / "adaq-research-sdk" / "src"))


def test_public_namespace_has_no_private_runner_surface() -> None:
    adaq = importlib.import_module("adaq")
    assert adaq.__version__ == "1.0.0"
    assert not any("runner" in name.lower() or "protocol" in name.lower() for name in adaq.__all__)


def test_factor_contract_rejects_non_finite_values() -> None:
    from adaq import Scope, create_factor_definition, finite

    assert create_factor_definition(Scope.CROSS_SECTIONAL, [{"op": "return"}], ["score"])
    try:
        finite(float("nan"))
    except ValueError:
        pass
    else:
        raise AssertionError("NaN must not cross the public analytical boundary")


def test_qlib_surface_is_read_only_and_split_limited() -> None:
    from adaq import Unavailable
    from adaq.qlib import DatasetH, Partition

    assert list(inspect.signature(DatasetH.prepare).parameters) == ["self", "segment"]
    source = (ROOT / "adaq-research-sdk/src/adaq/qlib.py").read_text(encoding="utf-8")
    assert "Sequence" not in source
    assert "import qlib" not in source
    assert "from qlib" not in source

    dataset = DatasetH(
        {
            "train": Partition("train", ({"datetime": 1, "instrument": "AAA", "x": 1.0},), (1.0,)),
            "valid": Partition("valid", ({"datetime": 2, "instrument": "AAA", "x": 2.0},), (2.0,)),
            "test": Partition("test", ({"datetime": 3, "instrument": "AAA", "x": 3.0},)),
        }
    )
    view = dataset.prepare("test")
    assert view.index.names == ("datetime", "instrument")
    assert view.labels is None
    try:
        view.rows[0]["x"] = 4.0
    except TypeError:
        pass
    else:
        raise AssertionError("Host-fed Qlib rows must be read-only")

    class ArrowTable:
        def __init__(self, rows: list[dict[str, object]]) -> None:
            self.rows = rows

        def to_pylist(self) -> list[dict[str, object]]:
            return [dict(row) for row in self.rows]

    arrow_dataset = DatasetH.from_arrow(
        {
            "train": ArrowTable(
                [{"datetime": 1, "instrument": "AAA", "x": 1.0, "label": 1.0}]
            ),
            "valid": ArrowTable(
                [{"datetime": 2, "instrument": "AAA", "x": 2.0, "label": 2.0}]
            ),
            "test": ArrowTable([{"datetime": 3, "instrument": "AAA", "x": 3.0}]),
        }
    )
    assert arrow_dataset.prepare("train").labels == (1.0,)
    assert arrow_dataset.prepare("test").labels is None

    boundary_dataset = DatasetH.from_arrow(
        {
            "train": ArrowTable(
                [
                    {
                        "datetime": 1,
                        "instrument": "AAA",
                        "x": 1.0,
                        "label": {"reason": "target-window-boundary"},
                    }
                ]
            ),
            "valid": ArrowTable(
                [{"datetime": 2, "instrument": "AAA", "x": 2.0, "label": 2.0}]
            ),
            "test": ArrowTable([{"datetime": 3, "instrument": "AAA", "x": 3.0}]),
        }
    )
    assert boundary_dataset.prepare("train").labels == (
        Unavailable("target-window-boundary"),
    )
