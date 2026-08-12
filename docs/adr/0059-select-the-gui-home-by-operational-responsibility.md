# Select the GUI home by operational responsibility

Status: accepted. This supersedes only the fixed `/` home selection in ADR 0052; the Operations Dashboard's operational scope remains accepted.

ADAQ renders the shared Workflow Guide at `/` when no Operational Responsibility exists and the Operations Dashboard whenever one does, so first-time and research-only Users see the ordered Research-to-Paper path while unresolved Bot, account, order, position, reconciliation, or Alert duties remain impossible to hide behind a help screen. The same Workflow Guide remains available at `/help/workflow`, uses `@antv/infographic` for the visual process with an equivalent semantic navigation view, keeps planned steps inspectable while gating their actions, derives progress from authoritative evidence without a mutable global workflow record, and requires explicit selection when more than one evidence lineage could continue.
