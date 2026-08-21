# Define V1 product readiness as scoped assertions

Status: accepted

ADAQ V1 does not expose a global readiness flag. Product readiness is an immutable, reviewable `Readiness Assertion` for one declared `Capability × Journey × Market/Data Context × Supported Platform × Interface Locale` scope. The assertion is `Ready for declared scope` only when every required acceptance gate passes; otherwise it is `Not Ready` with explicit blockers. Capability availability remains `Workflow Capability State`, user evidence remains `Workflow Step State`, and runtime operation remains Health/Alert state.

The required gates cover domain scope and prerequisites; the complete setup → discovery → configuration → execution → progress → evidence → next-step journey; progress, cancellation, failure, restart, recovery, and retained evidence; Host authority, safety, and fail-closed behavior; immediate paint, control-owned pending/error/retry, localization, keyboard and assistive access; and supported-platform reproducibility. A hidden manual edit of SQLite, Parquet, configuration, Python output, or an external script is not an accepted step in a product journey.

Readiness evidence is recorded outside the runtime data model. It binds each criterion to its exact reviewed commit, platform, locale, commands or manual observations, domain evidence references, failure/recovery results, limitations, reviewer, and timestamp. Automated tests, CI, domain owners, and AI surfaces provide evidence but cannot declare readiness. Only the designated V1 Acceptance Reviewer or Release Owner may do so. Any change within the declared scope creates a new assertion; prior assertions remain historical and are never replaced by a mutable `latest` pointer.

An individual assertion may cover one market or context. The V1 release acceptance set must additionally contain the three reference journeys—OKX Crypto Paper, China A-share local Paper, and Alpaca U.S. Equity Paper—plus missing-data, provider-disconnect, clock-skew, Worker-crash, uncertain-order, credential-rotation, and restart-reconciliation failures. Shared GUI and Host capabilities require the supported platform and locale evidence defined by the acceptance matrix. Readiness assertions do not grant permissions or unlock actions.

## Consequences

- The Workflow Guide exposes capability scope, limitations, user-scoped evidence state, blockers, and the next honest action; it does not display a global V1 green light.
- The Operations Dashboard remains responsible for current Health, Alerts, Bot, Account, and reconciliation state, not release acceptance.
- No SQLite table, generic Workflow Project record, or second readiness state is added for V1.
- The existing M9–M12 acceptance matrices, deterministic fixtures, three-platform CI, bilingual checks, and manual OS observations remain the evidence mechanisms.
