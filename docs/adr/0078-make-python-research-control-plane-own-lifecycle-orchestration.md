# Make the Python Research Control Plane own lifecycle orchestration

The Python Research Control Plane is the sole application orchestration boundary for Project, Trust, Runtime, Environment, Runner, Research Attempt, recovery, and reset lifecycles. The existing in-process `PythonResearchState` remains the shared facade while Store and runner details become private; Tauri commands only adapt IPC and blocking work. Start requires an exact frozen Revision, prepared Environment, and explicit Trust Decision, while Environment Preparation Attempts remain separate from Python Research Attempts and the heavy Research Queue. Pending Attempts survive restart in FIFO order, stale Running Attempts become `Failed::generation-interrupted`, Running cancellation is terminal only after process exit, and active duplicate Starts coalesce only on complete execution identity while Retry creates a new linked Attempt. The Control Plane validates Runner identity, protocol, staged results, and cancellation before Factor and Model modules publish their own evidence; User-scoped Reset blocks new work, waits for active work to exit without holding the SQLite mutex, removes Python metadata, and preserves Working Copies, Archives, and shared reconstructible Cache.

The lifecycle does not require a cross-Store transaction: each Store and staged filesystem publication remains independently atomic, while the Control Plane uses fixed phase ordering, idempotent retries, and startup cleanup for incomplete staging. Factor and Model modules pull a typed Host-validated result rather than receiving reverse callbacks. The facade exposes stable error categories instead of Store or Runner strings. An incompatible Python Research Schema enters read-only `ResetRequired`: source inspection and Archive export remain available, lifecycle mutations and execution are blocked, and only explicit Reset may clear the condition.

The facade exposes explicit Project, Environment, Attempt, Reset, and status operations rather than Store accessors or an implicit `run_all` workflow. It remains a client of the existing Research Queue through a narrow work adapter: the Queue schedules and shuts down work, while the Control Plane owns Attempt transitions, Runner execution, and retained results. Environment Sync changes only the mutable Working Copy; existing Revision, Trust, and Attempt identities remain immutable, and a new Lock requires a new Freeze and Trust. The core lifecycle methods remain synchronous and Tauri-independent; commands adapt IPC and `spawn_blocking`, and Queue workers call the same methods.

The read boundary is a bounded typed projection for Project, Environment, Attempt, Runtime, and Reset status; UI notifications are invalidation hints and never authoritative state. Attempt progress and diagnostics are retained, while fine-grained Runtime download progress may remain volatile. In-memory completed-result maps are only handoff caches: Attempt metadata, result hash, and Attempt-scoped artifacts are authoritative and reconstructible after restart. A missing or mismatched artifact fails closed and never triggers an implicit rerun.

An Attempt Plan remains pinned to its explicit immutable Revision even if the Working Copy later changes. A durable cancellation request wins a race with a late Runner result, so no cancelled result can publish. Reset establishes the User barrier before cancelling Pending or Running work; a Queue worker rechecks state before Begin, and Reset waits for any work that already began. Reset may invalidate a completed result before another domain reads it; later typed reads return `reset` or `not_found` and cannot publish Factor or Model evidence.

Completed metadata is never rewritten when its artifact is missing or hash-invalid; result reads fail closed with an integrity error. Execution failures retain stable codes and bounded diagnostics, and only explicit Retry creates a linked new Attempt. Cache eviction excludes environments referenced by active Attempts; an unavailable binding fails closed as `environment_not_ready` and requires explicit Prepare followed by Retry. Progress, diagnostics, and logs remain bounded by the Host Resource Policy and carry truncation evidence where applicable.

## Acceptance Criteria

- Tauri commands call one deep facade and do not compose or expose Stores.
- Project, Trust, Runtime, Environment, Attempt, Reset, status, and typed result reads share the same lifecycle orchestration.
- Queue, Runner, Cancel, Retry, Restart Recovery, Reset, atomic publication, Schema Reset, and multi-User isolation have direct core and race coverage.
- Factor/Model evidence ownership and shared Research Queue scheduling remain unchanged.
- The core facade is testable without Tauri.

## Out of Scope

Research Queue redesign, Factor/Model Store migration, M11 Promotion changes, Runner Protocol changes, M13 Strategy lifecycle, new database migration, UI redesign, automatic Trust, automatic Retry, and automatic Schema Migration.

## Delivery Order

1. Introduce the Tauri-independent facade, private Stores, and narrow Queue port.
2. Move Project, Trust, Runtime, and Environment orchestration behind it.
3. Move Attempt, Runner, Cancel, Retry, Recovery, and Reset orchestration behind it.
4. Add typed status, stable errors, and the validated-result pull boundary.
5. Add concurrency, restart, atomic-publication, Schema Reset, and multi-User acceptance evidence.
