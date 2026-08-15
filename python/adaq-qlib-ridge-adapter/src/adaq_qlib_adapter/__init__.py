"""The registered, Host-fed Ridge-only Model Research Adapter.

The adapter receives an already prepared ``adaq.qlib.DatasetH`` and a
Host-fitted transformation.  It does not discover Qlib classes, initialize a
Provider, or serialize executable Python state.
"""

from __future__ import annotations

from dataclasses import dataclass
from math import isfinite

from adaq import Unavailable

ADAPTER_ID = "qlib-linear-ridge@1"
BRIDGE_VERSION = "adaq.qlib@1"
ARTIFACT_SCHEMA = "adaq:linear-model@1"
MODEL_CLASS = "LinearModel"
MODEL_MODE = "ridge"
NUMERIC_REPRESENTATION = "ieee754-binary64"
FORECAST_CONTRACT = "forecast:continuous-future-close-return:native@1"
RIDGE_ALPHAS = (0.1, 1.0, 10.0)


@dataclass(frozen=True)
class FitResult:
    alpha: float
    coefficients: tuple[float, ...]
    intercept: float
    input_slots: tuple[str, ...]
    transformation_sha256: str
    adapter_id: str = ADAPTER_ID
    artifact_schema: str = ARTIFACT_SCHEMA
    numeric_representation: str = NUMERIC_REPRESENTATION
    forecast_contract: str = FORECAST_CONTRACT


def registered(alpha: float) -> bool:
    return any(float(alpha).hex() == value.hex() for value in RIDGE_ALPHAS)


def fit(
    dataset: object,
    transformation: dict[str, object],
    alpha: float,
) -> FitResult:
    """Fit Ridge using only Train rows and labels supplied by the Host."""

    if not registered(alpha):
        raise ValueError("ridge alpha is not registered")
    names, means, scales, transformation_sha256 = _read_transformation(transformation)
    train = dataset.prepare("train")
    if train.labels is None or len(train.labels) != len(train.rows):
        raise ValueError("Train labels are unavailable")
    if tuple(train.columns) != names:
        raise ValueError("Train feature schema differs from transformation")
    matrix = []
    labels = []
    for row, label in zip(train.rows, train.labels):
        if isinstance(label, Unavailable):
            continue
        values = [1.0]
        values.extend(
            (float(row[name]) - mean) / scale
            for name, mean, scale in zip(names, means, scales)
        )
        if any(not isfinite(value) for value in values) or not isfinite(float(label)):
            raise ValueError("non-finite Ridge input")
        matrix.append(values)
        labels.append(float(label))
    if not matrix:
        raise ValueError("Train labels are unavailable")
    coefficients = _solve_ridge(matrix, labels, float(alpha))
    return FitResult(
        alpha=float(alpha),
        coefficients=tuple(coefficients[1:]),
        intercept=coefficients[0],
        input_slots=names,
        transformation_sha256=transformation_sha256,
    )


def predict(dataset: object, fitted: FitResult, transformation: dict[str, object]):
    """Predict from the reloaded data-only result; labels are never required."""

    if (
        fitted.adapter_id != ADAPTER_ID
        or fitted.artifact_schema != ARTIFACT_SCHEMA
        or fitted.numeric_representation != NUMERIC_REPRESENTATION
        or fitted.forecast_contract != FORECAST_CONTRACT
        or not registered(fitted.alpha)
        or not fitted.input_slots
        or len(fitted.coefficients) != len(fitted.input_slots)
        or any(not isfinite(value) for value in (*fitted.coefficients, fitted.intercept))
    ):
        raise ValueError("Fitted Ridge artifact is invalid")
    names, means, scales, transformation_sha256 = _read_transformation(transformation)
    if (
        fitted.input_slots != names
        or fitted.transformation_sha256 != transformation_sha256
    ):
        raise ValueError("Fitted Ridge transformation identity differs")
    test = dataset.prepare("test")
    if tuple(test.columns) != names or test.labels is not None:
        raise ValueError("Feature-only test partition is required")
    for row in test.rows:
        values = [
            (float(row[name]) - mean) / scale
            for name, mean, scale in zip(names, means, scales)
        ]
        value = fitted.intercept + sum(
            coefficient * feature
            for coefficient, feature in zip(fitted.coefficients, values)
        )
        if not isfinite(value):
            raise ValueError("non-finite Ridge forecast")
        yield {
            "datetime": row["datetime"],
            "instrument": row["instrument"],
            "value": value,
            "unavailableReason": None,
        }


def _read_transformation(
    transformation: dict[str, object],
) -> tuple[tuple[str, ...], tuple[float, ...], tuple[float, ...], str]:
    names = tuple(str(name) for name in transformation.get("featureNames", ()))
    means = tuple(float(value) for value in transformation.get("means", ()))
    scales = tuple(float(value) for value in transformation.get("scales", ()))
    transformation_sha256 = transformation.get("transformationSha256")
    if (
        not names
        or len(names) != len(means)
        or len(means) != len(scales)
        or len(set(names)) != len(names)
        or not isinstance(transformation_sha256, str)
        or len(transformation_sha256) != 64
        or any(character not in "0123456789abcdef" for character in transformation_sha256)
        or any(not isfinite(value) for value in means)
        or any(not isfinite(value) or value <= 0 for value in scales)
    ):
        raise ValueError("Host transformation is invalid")
    return names, means, scales, transformation_sha256


def _solve_ridge(matrix: list[list[float]], labels: list[float], alpha: float) -> list[float]:
    if not matrix or len(matrix) != len(labels):
        raise ValueError("Ridge matrix is empty")
    width = len(matrix[0])
    if not width or any(len(row) != width for row in matrix):
        raise ValueError("Ridge matrix dimensions are invalid")
    normal = [[0.0 for _ in range(width + 1)] for _ in range(width)]
    for row, label in zip(matrix, labels):
        for left in range(width):
            for right in range(width):
                normal[left][right] += row[left] * row[right]
            normal[left][width] += row[left] * label
    for index in range(1, width):
        normal[index][index] += alpha
    for pivot in range(width):
        selected = max(range(pivot, width), key=lambda row: abs(normal[row][pivot]))
        if abs(normal[selected][pivot]) < 1e-12:
            raise ValueError("Ridge normal matrix is singular")
        normal[pivot], normal[selected] = normal[selected], normal[pivot]
        divisor = normal[pivot][pivot]
        for column in range(pivot, width + 1):
            normal[pivot][column] /= divisor
        for row in range(width):
            if row == pivot:
                continue
            factor = normal[row][pivot]
            for column in range(pivot, width + 1):
                normal[row][column] -= factor * normal[pivot][column]
    result = [row[width] for row in normal]
    if any(not isfinite(value) for value in result):
        raise ValueError("Ridge coefficients are non-finite")
    return result


__all__ = [
    "ADAPTER_ID",
    "ARTIFACT_SCHEMA",
    "BRIDGE_VERSION",
    "FitResult",
    "FORECAST_CONTRACT",
    "MODEL_CLASS",
    "MODEL_MODE",
    "NUMERIC_REPRESENTATION",
    "RIDGE_ALPHAS",
    "fit",
    "predict",
    "registered",
]
