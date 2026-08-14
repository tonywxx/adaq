from __future__ import annotations

import importlib
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
