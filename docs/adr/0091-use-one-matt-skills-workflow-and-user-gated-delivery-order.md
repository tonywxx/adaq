# Use one Matt skills workflow and a User-gated delivery order

Status: accepted

ADAQ uses one planning and delivery workflow for every agent: `/grill-with-docs` to settle consequential decisions and domain language, `/to-spec` to publish the confirmed specification and testing seams, `/to-tickets` to publish dependency-ordered vertical slices, and one fresh `/implement <issue>` session per ticket. `planning-with-files`, repository-root `task_plan.md`, `findings.md`, `progress.md`, and alternative agent planning systems are not used because parallel planning authorities previously allowed issue state, reusable cores, and downstream work to outrun the executable product journey.

Delivery follows one strict sequence: Market Data Acquisition; Data Validation, Canonicalization, Quality, and Persistence; Feature Engineering; Factor Research Steps 1–3; Model Research Steps 4–6; Strategy Validation Steps 7–8; then Paper Operations Steps 9–10. Every module must consume the exact accepted output of its predecessor and obtain Workflow Module Acceptance plus explicit User Workflow Continuation Approval before the next module begins. Current-head automated checks, desktop product-run evidence, applicable failure/recovery paths, both Interface Locales, and supported-platform evidence are required; issue closure, historical comments, fixtures, reusable cores, green CI, or an open issue frontier cannot waive the gate.

## Consequences

- All agents and tools participating in this repository must follow the same workflow and delivery order.
- Existing roadmaps, issues, and implementation evidence remain useful inputs but do not independently authorize the next module.
- Only work inside the currently User-authorized module may proceed; downstream implementation remains paused even when technically unblocked.
- The User verifies each completed module and alone authorizes continuation.
