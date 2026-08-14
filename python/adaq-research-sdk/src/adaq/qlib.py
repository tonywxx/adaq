"""Host-fed Qlib compatibility names.

The actual bridge is implemented by the managed Adapter. This public module
only defines the read-only shape Projects can type against; it never imports
Qlib, opens a Provider, or discovers local data.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Mapping, Protocol, Sequence


@dataclass(frozen=True)
class Partition:
    name: str
    rows: tuple[Mapping[str, Any], ...]
    labels: tuple[float, ...] | None = None


class DatasetH(Protocol):
    def prepare(
        self, segments: str | Sequence[str], col_set: str = "feature"
    ) -> Any: ...


__all__ = ["DatasetH", "Partition"]
