# Rebaseline V1 to an OKX Spot end-to-end workflow

Status: accepted

## Context

The previous V1 target required complete crypto, China A-share, and U.S. equity data and Paper workflows. The current A-share and U.S. equity acquisition paths do not have stable, qualified sources, and their dependencies can make the shared build fail. At the same time, substantial asset-neutral research, Component, Paper, Bot, Operations, and feedback contracts already exist but have not been proven as one product workflow.

Treating OKX-only delivery as a temporary test slice leaves contradictory completion gates and makes it impossible to say what V1 completion means.

## Decision

V1 is the complete local, bilingual OKX Spot workflow:

`OKX Spot data → research evidence → Features → Factors → Models → Strategies/backtests → qualified Components → OKX Demo Paper account/execution → supervised Bots → Operations → immutable Paper feedback and human research review`.

Only OKX Demo Trading is a V1 execution endpoint. Live endpoints, real credentials, and real-money order submission remain prohibited.

China A-share and U.S. equity support move to **Post-V1 Market Expansion**. Existing code, evidence, and asset-neutral contracts are retained. Deferred market paths must be isolated from the default V1 build, runtime, navigation, acceptance, and readiness claims so they cannot break or misrepresent the supported OKX product.

M9 and M13–M18 are complete only when the OKX-scoped product journey and its failure/recovery matrix have current-head evidence. Closed issues and reusable core code are inputs to that work, not proof that V1 is complete.

This decision supersedes only the V1 market-scope and completion-gate portions of ADRs 0030, 0043, 0044, 0053, 0081, 0084, and 0089. Their remaining security, evidence, authority, and lifecycle decisions still apply.

## Consequences

- A single canonical recovery map owns the remaining path to V1 completion.
- The first work isolates deferred equity paths, repairs Host-derived User authority, and restores a current-head verification baseline.
- Internal domain contracts remain asset-neutral; V1 product surfaces and assertions are honestly OKX-specific.
- The offline A-share tutorial fixture may remain as deterministic research test data. It does not constitute A-share provider or product support.
- Adding A-share or U.S. equity support requires a separate post-V1 plan, qualified data source, adapter evidence, product workflow, and scoped readiness assertions.
- Deferred markets return through an opt-in feature build and a separate post-V1 product surface. That extension may reuse asset-neutral `Instrument`, calendar, evidence, quality, snapshot, and research-context contracts, but it must add provider-owned adapters, explicit capability snapshots, dedicated commands/routes, and scoped readiness assertions before re-entering the default product.
- The final V1 readiness decision remains a human Release Owner decision recorded through scoped Readiness Assertions; it is not inferred from issue closure or automated tests alone.
