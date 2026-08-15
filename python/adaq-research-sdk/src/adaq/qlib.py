"""The small, read-only DatasetH surface supplied by the Host.

This module deliberately has no Qlib, Provider, filesystem, or network
dependency.  ``from_arrow`` accepts a pyarrow-like table only at the boundary
and normalizes it into immutable rows before a Project can inspect a split.
"""

from __future__ import annotations

from dataclasses import dataclass
from math import isfinite
from types import MappingProxyType
from typing import Any, Literal, Mapping

from ._contracts import Unavailable


@dataclass(frozen=True)
class PandasIndex:
    values: tuple[tuple[int, str], ...]
    names: tuple[str, str] = ("datetime", "instrument")


@dataclass(frozen=True)
class PandasView:
    """Immutable pandas-shaped view with explicit two-level identity.

    The managed Wheelhouse may provide pandas to a future adapter, but the
    public SDK keeps this dependency-free view as the stable contract.  The
    adapter consumes rows and index values, never a mutable DataFrame.
    """

    index: PandasIndex
    columns: tuple[str, ...]
    rows: tuple[Mapping[str, Any], ...]
    labels: tuple[float | Unavailable, ...] | None = None

    def __iter__(self):
        return iter(self.rows)


@dataclass(frozen=True)
class Partition:
    name: Literal["train", "valid", "test"]
    rows: tuple[Mapping[str, Any], ...]
    labels: tuple[float | Unavailable, ...] | None = None

    def view(self) -> PandasView:
        values = tuple(
            (int(row["datetime"]), str(row["instrument"])) for row in self.rows
        )
        columns = tuple(
            key
            for key in self.rows[0].keys()
            if key not in {"datetime", "instrument", "label"}
        ) if self.rows else ()
        return PandasView(PandasIndex(values), columns, self.rows, self.labels)


class DatasetH:
    """Host-fed DatasetH with exactly ``train``, ``valid``, and ``test``."""

    def __init__(self, partitions: Mapping[str, Partition]):
        expected = {"train", "valid", "test"}
        if set(partitions) != expected:
            raise ValueError("qlib dataset requires train, valid, and test")
        values = {
            name: Partition(
                partition.name,
                tuple(MappingProxyType(dict(row)) for row in partition.rows),
                None
                if partition.labels is None
                else tuple(
                    label if isinstance(label, Unavailable) else float(label)
                    for label in partition.labels
                ),
            )
            for name, partition in partitions.items()
        }
        feature_names = _feature_names(values["train"].rows)
        for name, partition in values.items():
            if partition.name != name:
                raise ValueError("qlib partition name mismatch")
            if _feature_names(partition.rows) != feature_names:
                raise ValueError("qlib partition schema mismatch")
            _validate_rows(partition.rows, feature_names, name)
            if name == "test" and partition.labels is not None:
                raise ValueError("qlib test labels are host-only")
            if name != "test" and (
                partition.labels is None or len(partition.labels) != len(partition.rows)
            ):
                raise ValueError("qlib training labels are required")
            if partition.labels is not None and any(
                not _valid_label(label) for label in partition.labels
            ):
                raise ValueError("qlib labels are non-finite")
        identities = [
            (row["datetime"], row["instrument"])
            for partition in values.values()
            for row in partition.rows
        ]
        if len(identities) != len(set(identities)):
            raise ValueError("qlib partition identities overlap")
        self._partitions = values

    @classmethod
    def from_records(
        cls,
        train: tuple[Mapping[str, Any], ...],
        valid: tuple[Mapping[str, Any], ...],
        test: tuple[Mapping[str, Any], ...],
        train_labels: tuple[float, ...],
        valid_labels: tuple[float, ...],
    ) -> "DatasetH":
        return cls(
            {
                "train": Partition("train", train, train_labels),
                "valid": Partition("valid", valid, valid_labels),
                "test": Partition("test", test, None),
            }
        )

    @classmethod
    def from_arrow(cls, partitions: Mapping[str, Any]) -> "DatasetH":
        """Convert Host Arrow tables without opening Qlib storage."""

        values: dict[str, Partition] = {}
        for name in ("train", "valid", "test"):
            table = partitions[name]
            rows = tuple(_arrow_rows(table))
            labels = None
            if name != "test":
                labels = tuple(_arrow_label(row.pop("label")) for row in rows)
            values[name] = Partition(name, rows, labels)
        return cls(values)

    def prepare(self, segment: Literal["train", "valid", "test"]) -> PandasView:
        if segment not in {"train", "valid", "test"}:
            raise ValueError("qlib dataset split unsupported")
        return self._partitions[segment].view()


def _arrow_rows(table: Any) -> tuple[dict[str, Any], ...]:
    if not callable(getattr(table, "to_pylist", None)):
        raise TypeError("Host partition must provide to_pylist")
    rows = table.to_pylist()
    if not isinstance(rows, list):
        raise TypeError("Host partition rows invalid")
    return tuple(dict(row) for row in rows)


def _arrow_label(value: Any) -> float | Unavailable:
    if isinstance(value, dict):
        if set(value) != {"reason"} or not isinstance(value["reason"], str):
            raise ValueError("qlib target label unavailable reason invalid")
        return Unavailable(value["reason"])
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError("qlib target label invalid")
    result = float(value)
    if not isfinite(result):
        raise ValueError("qlib target label non-finite")
    return result


def _valid_label(value: float | Unavailable) -> bool:
    return (
        isinstance(value, Unavailable)
        and bool(value.reason.strip())
    ) or (
        not isinstance(value, bool)
        and isinstance(value, (int, float))
        and isfinite(float(value))
    )


def _feature_names(rows: tuple[Mapping[str, Any], ...]) -> tuple[str, ...]:
    if not rows:
        raise ValueError("qlib partition is empty")
    names = tuple(
        key for key in rows[0] if key not in {"datetime", "instrument", "label"}
    )
    if not names or len(set(names)) != len(names):
        raise ValueError("qlib feature schema invalid")
    return names


def _validate_rows(
    rows: tuple[Mapping[str, Any], ...],
    feature_names: tuple[str, ...],
    partition: str,
) -> None:
    identities = []
    for row in rows:
        if set(row) != {"datetime", "instrument", *feature_names}:
            raise ValueError("qlib row has undeclared columns")
        identity = (row.get("datetime"), row.get("instrument"))
        if not isinstance(identity[0], int) or not isinstance(identity[1], str):
            raise ValueError("qlib identity types invalid")
        if not identity[1]:
            raise ValueError("qlib instrument is empty")
        if any(
            isinstance(row.get(name), bool)
            or not isinstance(row.get(name), (int, float))
            or not isfinite(float(row[name]))
            for name in feature_names
        ):
            raise ValueError("qlib feature value invalid")
        identities.append(identity)
    if identities != sorted(identities) or len(set(identities)) != len(identities):
        raise ValueError(f"qlib {partition} identity order invalid")


__all__ = ["DatasetH", "PandasIndex", "PandasView", "Partition"]
