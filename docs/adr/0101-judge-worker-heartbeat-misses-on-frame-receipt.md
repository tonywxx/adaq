# Judge a Worker heartbeat miss on frame receipt instead of Host processing

Status: accepted

## Context

`WorkerSupervisor` compared `Instant::now()` with `last_heartbeat`, and `last_heartbeat` was advanced by the *consumer* — inside the drain loop of `poll_health` and `receive_until` — every time a heartbeat frame was read out of the mailbox. The consumer is not a dedicated reader: the monitor thread that calls `poll_health` also persists each health event durably (bot attempt evidence plus an operational event, per heartbeat), and the decision path calls `receive_until`, whose staleness check runs *before* anything pending is drained. The quantity actually being measured was therefore "how long since the Host last got around to reading its mailbox", not "how long since the Worker stopped sending".

Live evidence (2026-09-18, OKX Demo account, 33 Worker runs) from the two sinks that already persist both sides of every heartbeat — the Worker's in-band `observedAtMs` and the Host's receipt time:

- The Worker's own send clock never gapped by 3s: worst consecutive gap 2,772ms across 414,172 observations, and 30 of 33 runs stayed within ~2s.
- The Host observed frames up to 317s late: p50 193ms, p90 336ms, p99 1,396ms, and 2,983 observations (0.7%) more than 3s late.
- All eight recorded `worker-heartbeat-missed` faults landed 0.5–1.6s after a heartbeat *observation*, whose receipt had in fact occurred more than one timeout earlier; the surrounding window showed the Worker sending at a clean 1.00s cadence throughout.

Worker pacing was sound in every case. The Host's own processing backlog was being reported as a Worker fault.

## Decision

A heartbeat miss is judged on the instant the Host process **received** the newest frame from the Worker, never on the instant the consumer processed it. `WorkerSupervisor::last_frame_received` is shared with the stdout reader thread and advanced by it on every received frame, before the frame can wait behind the consumer's work; `poll_health` and `receive_until` both decide silence through `worker_silent_since` against that receipt instant. The reader thread remains the only writer of that clock.

The fault code `worker-heartbeat-missed` and the timeout policy are unchanged, including the legacy default's 10s host grace in `host_runtime_policy`.

## Consequences

- A busy Host can no longer fault a Worker that is still talking: a frame waiting in the mailbox counts as received. This is the regression the runtime now pins with tests.
- The check now measures delivery silence over the whole frame channel rather than heartbeat frames specifically, because the reader classifies nothing and cannot decode frames. A Worker whose heartbeat thread stalled while other frames keep flowing is no longer caught by this check; a Worker that initializes and then goes quiet is still faulted, as are process exit, protocol faults, and decision-deadline breaches.
- Residual risk: if the reader thread itself is starved for longer than the timeout, silence can still be declared while frames wait in the pipe. The supervised run that follows must confirm the mis-report is gone and that genuine Worker silence still faults.
- No new telemetry was required. The Worker's in-band heartbeat timestamps and the Host's receipt times are already retained per beat, so this diagnosis was made from the existing database rather than from another observation window.