# Finish delivery with Observability and Dashboard gates

Status: accepted

ADAQ extends ADR 0091's strict User-gated delivery order with two final modules after Paper Operations Step 10: Operational Observability and Monitoring, then the System Dashboard. Every earlier module emits typed operational evidence at its own boundary so Monitoring integrates rather than reconstructs evidence; the Dashboard is a rebuildable, authorized read projection that links to owning workspaces and never becomes an operational authority.

## Consequences

- The canonical delivery map contains fifteen serial Module Gates: three Workflow Foundations, ten Research-to-Paper Steps, Monitoring, and Dashboard.
- The canonical map owns fifteen Gate Parent issues; each Gate Parent owns independently actionable implementation children, and only those children enter fresh `/implement <issue>` sessions.
- All fifteen Gate Parents may be published for visibility, but detailed implementation children are created only for the current User-authorized gate.
- A Module Gate remains open after its implementation children finish until current-head evidence is reviewed and the User grants Workflow Continuation Approval.
- Market Data Acquisition stops at immutable Source evidence; the following gate alone owns validation, canonicalization, quality, persistence, Snapshot publication, and Point-in-Time Universe publication.
- Factor, Model, and Strategy qualification remains inside Steps 3, 6, and 8 respectively rather than forming a cross-product delivery gate.
- Paper Step 9 reaches the first safe OKX Demo Bot Runtime Attempt; Paper Step 10 owns operation, reconciliation, immutable feedback, and human research review while emitting typed evidence for the later system-wide Monitoring gate.
- Final Readiness Assertions review the accepted fifteen-gate journey and close the canonical map; they are not a sixteenth development gate.
- Every Gate Parent binds its exact accepted upstream identity and requires current-head criterion mapping, automated checks, desktop product-run evidence, applicable failure/recovery and fail-closed evidence, security boundaries, both Interface Locales, declared-platform evidence, and explicit User approval.
- Every gate runs automated checks on all declared supported platforms; routine manual acceptance uses the current primary platform, while platform-sensitive gates and final acceptance require the complete supported-platform manual matrix.
- A later change to an accepted output contract, identity, data semantic, or evidence boundary reopens the affected gate and pauses its downstream suffix; an internal change with an unchanged boundary requires scoped impact and regression evidence rather than an automatic full restart.
- `/to-tickets` creates implementation children only for current-head gaps in the current authorized gate; already-satisfied criteria receive fresh evidence instead of duplicate implementation issues.
