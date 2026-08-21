# Make the Data Foundation Workspace the explicit market-evidence workflow

Status: accepted

ADAQ will expose one User-owned Data Foundation Workspace as the explicit workflow for acquiring, inspecting, and publishing Market Data Foundation evidence. A User must start an Acquisition Operation before automatic prerequisite work may run; the Host may execute its stages automatically, but Source/Canonical/Quality state, Snapshot and Point-in-Time Universe readiness, evidence, cancellation/retry, and blockers remain visible. Cancellation preserves checkpoints and evidence, retry creates a new operation/revision without overwriting prior evidence, and downstream research consumes only published artifacts that pass the applicable typed readiness gate. This keeps the accepted M9 evidence contracts and Host authority intact while removing silent acquisition and developer-style handoff from the product surface.
