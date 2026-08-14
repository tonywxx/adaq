"""Small, dependency-free public contracts for ADAQ Research Projects.

The Host supplies contexts and inputs after a trusted Project has been
created. This module intentionally contains no filesystem, process, network,
database, or protocol helpers.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum
from typing import Any, Callable, Iterable, Mapping, Protocol, Sequence


class ProjectKind(str, Enum):
    FACTOR = "factor"
    MODEL = "model"
    STRATEGY = "strategy"


class Scope(str, Enum):
    POINTWISE = "pointwise"
    TIME_SERIES = "time-series"
    CROSS_SECTIONAL = "cross-sectional"


@dataclass(frozen=True)
class Parameter:
    id: str
    value: Any


@dataclass(frozen=True)
class InputSlot:
    id: str
    role: str
    scope: Scope
    required: bool = True


@dataclass(frozen=True)
class Unavailable:
    reason: str


@dataclass(frozen=True)
class Output:
    id: str
    value_type: str = "finite-f64"
    required: bool = True


@dataclass(frozen=True)
class Target:
    id: str
    kind: str
    horizon_bars: int
    value_scale: str = "raw"


@dataclass(frozen=True)
class Signal:
    id: str
    kind: str
    value_scale: str = "raw"


@dataclass(frozen=True)
class ResourcePolicy:
    max_wall_ms: int = 30 * 60 * 1000
    max_memory_bytes: int = 4 * 1024 * 1024 * 1024
    max_threads: int = 64
    max_input_rows: int = 10_000_000
    max_output_rows: int = 10_000_000


@dataclass(frozen=True)
class FactorDefinition:
    """Portable Factor graph returned by ``define``.

    Nodes are host-owned dictionaries so the public SDK can evolve with the
    versioned Feature Operator Catalog without exposing a second DSL.
    """

    scope: Scope
    nodes: tuple[Mapping[str, Any], ...]
    outputs: tuple[str, ...]


@dataclass(frozen=True)
class FactorOutput:
    instrument_id: str
    event_time_ms: int
    value: float | Unavailable


@dataclass(frozen=True)
class FactorOutputBatch:
    rows: tuple[FactorOutput, ...]
    segment_id: str


@dataclass(frozen=True)
class Forecast:
    instrument_id: str
    prediction_time_ms: int
    value: float | Unavailable


@dataclass(frozen=True)
class ModelArtifact:
    schema: str
    payload: Mapping[str, Any]


@dataclass(frozen=True)
class ModelContext:
    parameters: tuple[Parameter, ...]
    seed: int
    inputs: Mapping[str, Any]


@dataclass(frozen=True)
class FactorContext:
    parameters: tuple[Parameter, ...]
    seed: int
    inputs: Mapping[str, Any]


@dataclass(frozen=True)
class StrategyContext:
    parameters: tuple[Parameter, ...]
    seed: int
    inputs: Mapping[str, Any]


class FactorProject(Protocol):
    kind: ProjectKind

    def define(self, context: FactorContext) -> FactorDefinition: ...

    def evaluate(
        self, context: FactorContext, batches: Iterable[Mapping[str, Any]]
    ) -> Iterable[FactorOutputBatch]: ...


class ModelProject(Protocol):
    kind: ProjectKind

    def fit(self, context: ModelContext) -> ModelArtifact: ...

    def predict(
        self, context: ModelContext, fitted_model: ModelArtifact
    ) -> Iterable[Forecast]: ...


class StrategyProject(Protocol):
    kind: ProjectKind


ProgressCallback = Callable[[str, int, int], None]
DiagnosticCallback = Callable[[str, str, Mapping[str, Any]], None]


@dataclass
class RuntimeCallbacks:
    """Optional host callbacks; they never provide a generic query surface."""

    progress_callback: ProgressCallback | None = None
    diagnostic_callback: DiagnosticCallback | None = None
    _diagnostics: list[tuple[str, str, Mapping[str, Any]]] = field(default_factory=list)

    def progress(self, phase: str, completed: int, total: int) -> None:
        if total < 0 or completed < 0 or completed > total:
            raise ValueError("progress values are outside the declared bounds")
        if self.progress_callback is not None:
            self.progress_callback(phase, completed, total)

    def diagnostic(
        self, level: str, message: str, fields: Mapping[str, Any] | None = None
    ) -> None:
        if len(message) > 4096:
            raise ValueError("diagnostic message exceeds the public bound")
        value = dict(fields or {})
        item = (level, message, value)
        self._diagnostics.append(item)
        if self.diagnostic_callback is not None:
            self.diagnostic_callback(level, message, value)


def finite(value: float) -> float:
    """Return a finite analytical scalar and reject NaN/infinity."""

    if not isinstance(value, (int, float)) or not float(value) == float(value):
        raise ValueError("analytical scalar must be finite")
    result = float(value)
    if result in (float("inf"), float("-inf")):
        raise ValueError("analytical scalar must be finite")
    return result


def create_factor_definition(
    scope: Scope, nodes: Sequence[Mapping[str, Any]], outputs: Sequence[str]
) -> FactorDefinition:
    if not nodes or not outputs:
        raise ValueError("Factor Definition requires nodes and outputs")
    return FactorDefinition(scope, tuple(dict(node) for node in nodes), tuple(outputs))
