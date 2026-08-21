# ADAQ V1 Readiness Inventory

Status: working inventory for the expanded V1 rebaseline.

Date: 2026-08-21

## Decision

The target is the expanded V1 defined by `docs/v1-roadmap.md`. V1 is ready only through scoped `Readiness Assertion` records. Code or a passing unit test alone does not mark a capability ready.

Real Trading, a public Unified Data/Trading API, margin and short-selling A-share accounts, cloud Bot control, Marketplace infrastructure, and other items listed in the roadmap remain post-V1.

## Completion legend

- **Accepted foundation** — the milestone is documented as accepted and has implementation/verification evidence at its declared boundary.
- **Product remediation required** — core contracts exist, but the user-owned workflow or cross-workspace handoff is incomplete.
- **Core implemented; readiness unverified** — implementation exists in the current branch, while full GUI, recovery, bilingual, supported-platform, and manual acceptance evidence remains to be recorded.
- **Planned / downstream** — the capability depends on an earlier gate or has no complete product path yet.
- **Post-V1** — explicitly excluded from the V1 target.

## Milestone inventory

| Milestone | Scope | Current assessment | Evidence / blocker |
|---|---|---|---|
| M9 | Three-market data foundation, localization, secure connections, Markets workspaces | Core remediation implemented; product-run readiness pending | `docs/m13-entry-gate-acceptance.md` records passed automated gates. Desktop GUI/provider runs still need operation-level evidence for the three markets. |
| M10 | Host Feature Engine, Plans, transformations, immutable Feature Datasets | Accepted foundation | Existing Feature Engine contracts/tests and `docs/m10-manual-acceptance.md`. The next gap is Context selection and handoff, not a bottom-up Feature Engine rewrite. |
| M11 | Factor Lab, evaluation, Families/Trials, promotion decisions | Accepted foundation, with a product-surface remediation | Existing Factor evidence and `src/m11-manual-acceptance.test.ts`. Factor setup still exposes copied IDs/hashes/protocol data across boundaries. |
| M12 | Python SDK, managed runtime, Qlib Ridge Model Lab, tutorial | Accepted foundation | Roadmap states all seven M12 child issues closed. `docs/m12-python-research-manual-acceptance.md` explicitly defers Strategy to M13 and Component generation to M14. |
| M13 | Strategy Projects, Single-Instrument/Portfolio Backtest, Strategy selection and final evaluation | Entry gates automated; product acceptance pending | `docs/m13-entry-gate-acceptance.md` records the current automated evidence. Product acceptance still requires the desktop GUI/provider matrix and the existing Strategy-specific acceptance evidence. |
| M14 | Component generation, Build, Conformance, Equivalence, `.adaq` packaging/import | Core implemented; readiness unverified | Recent `54ebf6a` and `1d473b4` add qualification/conformance work. Full package trust, all allowed parameter combinations, equivalence, failure evidence, and GUI import acceptance remain to verify. |
| M15 | Secure Paper Accounts, provider adapters, Risk/OMS, orders/Fills, A-share simulator | Core implemented; readiness unverified | Recent `9272968` adds Paper Trading ledger core. Provider connection tests, reconciliation, uncertain outcomes, credentials, GUI journey, and three-market Paper acceptance remain. |
| M16 | Supervised Bot Runtime, workers, decision clocks, deadlines, recovery | Core contracts implemented; readiness unverified | Recent `e64c8c0` adds fail-closed Bot Runtime contracts. A complete running worker journey, crash/restart/reconciliation evidence, and Paper deployment qualification remain. |
| M17 | Health, Operational Events, Alerts, safety actions, Operations Dashboard | Core implemented; readiness unverified | Recent `20c7098` and `edd5ec7` add operational evidence and alerts. Full multidimensional failure matrix, notifications, drill-down, localization, and supported-platform acceptance remain. |
| M18 | Paper feedback, human review, hardening, accessibility, release acceptance | Partial implementation; acceptance not complete | Recent `062a750` adds Paper Feedback review contracts. The final three reference journeys, failure matrix, fault injection, release packaging, accessibility, performance, and scoped Readiness Assertions remain required. |

## Product gaps that must be done first

### 1. Data Foundation Workspace

Required behavior:

- User starts an Acquisition Operation.
- Source, Canonical, Quality, Snapshot, and Point-in-Time Universe state are visible.
- Acquisition history exposes retained checkpoints and lifecycle states.
- Cancellation preserves checkpoints and evidence.
- Retry creates a new operation/revision and does not overwrite prior evidence.
- Missing or degraded prerequisites block downstream research with an explicit reason.
- Markets remains an inspection surface; it must not silently own the acquisition workflow.

Relevant evidence:

- `docs/adr/0086-sequence-m9-m12-remediation-before-m13.md`
- `docs/adr/0087-make-data-foundation-workspace-explicit.md`
- `docs/m9-m12-current-head-gap-inventory.md`
- Existing Markets implementation and `9768c1e` / `006cb1d` history

### 2. Research Evidence Context handoff

The Host exposes establish/get commands, persists contexts in the local metadata store, and the Data Foundation page lets the User select a published Snapshot, Point-in-Time Universe, date range, Market, and Venue before establishing a Features Context. A shared preflight banner projects Context state in Features, Factors, and Models. Host stage-specific freeze is available from each banner. Model dataset generation and forecast evaluation freeze the Models Context before invoking their operation. Feature fitting/materialization and Factor materialization/evaluation perform the same stage freeze before invocation. Automated handoff and typed rejection coverage is recorded in `docs/m13-entry-gate-acceptance.md`; desktop GUI evidence remains pending.

Required behavior:

- Host owns a User-scoped Context binding Market, Venue, time range, Snapshot, required Universe, and evidence lineage.
- Context is selected from visible evidence, validated for compatibility, and frozen when an operation starts.
- Features, Factors, and Models consume the Context through Host orchestration.
- Stale, mixed-market, incomplete, or inaccessible evidence fails closed.
- Cross-workspace workflows stop requiring copied opaque IDs, hashes, or protocol JSON.

Relevant evidence:

- `docs/adr/0088-make-research-evidence-context-host-owned.md`
- `src-tauri/crates/adaq-factor-research/src/context.rs`
- `src/features/backtest/backtest-page.tsx` (current provenance view still exposes copyable identifiers)
- Existing M9–M12 gap inventory

### 3. Operations Evidence/Safety Foundation

This may proceed in parallel after the two gates are scoped. It should remain focused on shared Host evidence, lifecycle, health, fail-closed actions, and reconciliation contracts. Complete Paper/Bot/Dashboard journeys remain downstream of M14/M15/M16 dependencies.

Relevant evidence:

- `docs/adr/0089-rebaseline-m13-m18-with-remediation-and-operations-gates.md`
- `src-tauri/src/operations.rs`
- `src-tauri/src/paper_feedback.rs`
- `src-tauri/crates/adaq-bot-runtime/src/lib.rs`

## V1 journey status

| Journey | Status | Next honest action |
|---|---|---|
| Acquire and inspect OKX, A-share, and U.S. equity evidence | Automated gate passed; product run pending | Execute and record the three-market Data Foundation matrix |
| Define, materialize, and inspect Features | Context preflight implemented; product run pending | Execute and record the Features Context freeze |
| Research and promote Factors | Context preflight implemented; product run pending | Execute and record the Factors Context freeze |
| Train and evaluate Qlib-first Models | Context preflight implemented; product run pending | Execute and record the Models Context freeze |
| Build and backtest Strategies | Core implemented; blocked by entry gates | Complete Context gates, then M13 product acceptance |
| Generate/import qualified Components | Core implemented; downstream | Complete M13 and qualification evidence |
| Deploy Paper Accounts/Bots | Core implemented; downstream | Complete M14, then three Paper journeys |
| Monitor, review, and feed back Paper evidence | Partial; downstream | Complete M16, M17, then M18 hardening |
| Real-money trading | Post-V1 | No V1 action |

## Required V1 acceptance set

The release set must include scoped assertions for:

- OKX Crypto Paper journey
- China A-share local Paper journey
- Alpaca U.S. Equity Paper journey
- Missing data
- Provider disconnect
- Clock skew
- Worker crash
- Uncertain order state
- Credential rotation
- Restart reconciliation

Each assertion must bind the capability, journey, market/data context, platform, locale, reviewed commit, automated/manual evidence, limitations, and reviewer. A global green flag is insufficient.

## Recommended execution order

1. Extend and test the Data Foundation Workspace beyond its shared acquisition/readiness entry: publication detail, operation history, cancellation/retry evidence, and typed downstream blockers.
2. Verify restart recovery journeys and add attempt-specific evidence refresh/retry acceptance.
3. Connect Features → Factors → Models preflight and remove product-level opaque-ID handoff.
4. Verify the Operations Evidence/Safety Foundation in parallel.
5. Run the M13 Strategy/Portfolio acceptance slice.
6. Continue M14 → M15 → M16 → M17 → M18 in roadmap order.
7. Record scoped Readiness Assertions only after the three reference journeys and failure matrix pass.

## Sources

- `CONTEXT.md`
- `docs/v1-roadmap.md`
- `docs/m9-m12-current-head-gap-inventory.md`
- `docs/workflow-navigation.md`
- ADRs 0084, 0086, 0087, 0088, and 0089
- Current branch history through `1d473b4`

External GitHub issue state was unavailable in this environment; issue status is therefore not used as completion evidence.
