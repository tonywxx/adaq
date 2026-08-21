# Sequence M9–M12 product remediation before M13

Status: accepted

Before M13 implementation begins, ADAQ must remediate the two current-head product gaps identified by the M9–M12 readiness inventory:

1. **Data Foundation Workspace** — expose acquisition, quality, publication, cancellation/retry, evidence, and Snapshot/Point-in-Time Universe readiness as a user-owned workflow. Automatic acquisition may be an implementation detail only when its state and result are visible; missing prerequisites fail closed before downstream research.
2. **Research Evidence Context Handoff** — carry a compatible Market, Venue, time range, Snapshot, Point-in-Time Universe, and artifact lineage through Feature, Factor, and Model workspaces with explicit preflight. Users must not copy opaque hashes, IDs, or protocol JSON to cross the product boundary.

The two slices are separate decision and implementation boundaries but both are M13 entry gates. The Host Authenticated User Context decision is a cross-cutting prerequisite for their implementation, not a reclassification of an M9–M12 domain gap. M13 Strategy must consume these boundaries rather than preserve the current implicit acquisition or manual handoff.

M9 Data provenance and quality contracts, M10 Feature lifecycle and Dataset evidence, M11 Factor evaluation and promotion evidence, M12 Python/Model evidence, storage authority, manual-first/AI-advisory policy, and Host authority remain unchanged. No earlier decision is superseded. Existing Markets links to Models/Backtest are corrected through explicit qualification navigation and preflight rather than by changing the underlying engine contracts.

## Required gates before M13

- Three-market deterministic fixtures can complete the Data Foundation workflow with visible state, evidence, cancellation/retry, and fail-closed missing prerequisites.
- A Research Evidence Context can be selected, validated for compatibility, handed from Features through Factors to Models, and rejected when stale, mixed-market, incomplete, or User-inaccessible.
- Existing M9–M12 acceptance matrices and failure/recovery evidence remain green after the product-surface changes.
- The Host-derived Authenticated User Context, not request payload identity, owns all User isolation during these workflows.

## Consequences

- The remediation is intentionally product-surface and orchestration work; it does not reopen accepted Rust/ABI/evidence schemas.
- M13 roadmap work can depend on two independently actionable contract tickets, while later M14+ work remains downstream.
- The cost is delaying Strategy implementation until the user can establish and carry an honest evidence context; this prevents a new Strategy boundary from freezing the current developer-style handoff.
