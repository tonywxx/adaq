# Extend Bot decisions for the qualified EMA Trade-event workflow

The V1 Bot decision source remains confirmed Closed Bars and scheduled cross-sectional batches for every existing schedule. The qualified EMA double-cross schedule is the only scoped extension: the Host may submit retained, ordered OKX Trade observations together with confirmed UTC 15-minute bars to the existing Worker and Paper execution seams.

The extension preserves the ADR 0049 rules. The Host freezes one Decision Time, verifies source identity, availability, stream health, continuity, account reconciliation, and the next eligible execution event before authorizing a Target. A duplicate, late, out-of-order, unavailable, stale, truncated, or disconnected event batch produces evidence and no new risk. The Worker owns only deterministic EMA evaluation and returns Strategy Targets or structured NoTarget evidence; credentials, Risk, OMS, orders, and Paper state remain Host-owned.

Replay evidence is marked explicitly and cannot be silently shortened into an equivalent historical path. Restart or recovery reconstructs only from a complete retained interval within the Worker event limit; otherwise the Bot remains blocked for fresh warmup. Existing Closed-Bar and scheduled-batch schedules retain their original decision-source behavior and regression coverage.
