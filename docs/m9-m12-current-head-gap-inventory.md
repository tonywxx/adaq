# M9–M12 Current-HEAD Product Gap Inventory

Status: accepted inventory for the V1 rebaseline map.

Scope: current `main` HEAD, evaluated against the scoped Product Readiness Gate. This inventory separates accepted behavior, genuine product-readiness gaps, and deliberately deferred work. It does not reopen accepted M9–M12 engine contracts.

## Inventory

| Journey | Current-HEAD finding | Classification |
| --- | --- | --- |
| Data: Source → Canonical → Quality → Snapshot/Universe | M9 contracts and evidence cover the three-market pipeline, provenance, quality states, gaps, and recovery. The Markets workspace auto-acquires Instrument Master/Calendar when absent, shows pipeline summaries and evidence, but does not expose a complete user-owned acquisition, quality, publication, cancellation/retry, and Snapshot/Universe lifecycle. Its evidence card links directly to Models and Backtest. | Product gap: Data Foundation Workspace. |
| Feature: Definition → Fitting → Materialization → Dataset | Definitions, fitting, materialization, immutable datasets, progress, cancellation, retry, recovery, and deletion locks are represented by the accepted M10 behavior and current workspace controls. | Covered; retain the existing contract. |
| Factor: Candidate → Dataset → Evaluation → Family/Grid → Decision/Promotion | M11 evidence, attempt lifecycle, decisions, and promotion boundaries are present. Grid and materialization setup still require users to enter hashes, IDs, and protocol JSON across workspaces. | Product gap: Cross-workspace Context Handoff. |
| Model: Factor Decision → Model Dataset → Trial/Selection/Final Evaluation | M12 ends at inspectable Factor/Model evidence with explicit Trust, Selection, Promotion, and held-out evaluation. Strategy, generated/imported Components, Paper/Bot, and Monitoring are later milestones. | Covered to the M12 boundary; later work is deferred, not missing. |
| Failure and recovery | M9–M12 acceptance documents and current controls cover the relevant cancellation, restart, retry, redaction, and evidence cases. | Covered; no bottom-up engine rebuild indicated. |

## Genuine remediation scope

1. **Data Foundation Workspace** — make acquisition, quality, publication, and Snapshot/Universe readiness user-visible and user-controlled. Automatic acquisition must not be an unexplained prerequisite; the workspace needs state, cancellation/retry, evidence, and fail-closed onward navigation.
2. **Cross-workspace Context Handoff** — provide a shared, scoped context for market, time, and data evidence so Features, Factors, and Models can select compatible artifacts instead of requiring copied hashes/IDs or protocol JSON. Navigation and preflight must make prerequisite failures explicit.

## Explicitly not gaps in this inventory

- M13 Strategy, M14 Component generation/import, and M15–M18 Paper/Bot/Monitoring.
- Fixture/mock-server/environment setup required to run manual acceptance.
- Reimplementation of accepted M9–M12 backend contracts without new evidence.

## Evidence reviewed

- `src/features/markets/markets-page.tsx`
- `src/features/features/features-attempts.tsx`
- `src/features/factors/factors-page.tsx`
- `src/features/models/models-page.tsx`
- `src/features/backtest/backtest-page.tsx`
- `docs/m9-manual-acceptance.md`
- `docs/m10-manual-acceptance.md`
- `docs/m11-manual-acceptance.md`
- `docs/m12-python-research-manual-acceptance.md`

Focused verification at this HEAD passed:

```text
6 test suites, 13 tests passed
pnpm run build passed (tsc + Vite build)
```
