# Make Research Evidence Context Host-owned and immutable at operation start

Status: accepted

ADAQ will carry one Host-owned, User-scoped Research Evidence Context across Features, Factors, and Models. The context binds Market, Venue, time range, exact Market Data Snapshot, stage-required Point-in-Time Universe, and the immutable evidence lineage required by the next operation; the Host resolves and revalidates those bindings rather than accepting copied IDs, hashes, or protocol JSON from a Client. A context becomes immutable when an operation starts, stale or incompatible evidence fails closed, and a new revision is required for any change. This preserves reproducibility and User isolation while allowing Provider-Graded or Degraded evidence to remain visible for stage-specific policy decisions.
