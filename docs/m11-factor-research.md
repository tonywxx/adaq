# M11 Factor Research and Promotion

[简体中文](./m11-factor-research.zh-CN.md)

Status: accepted M11 architecture and delivery, published as [parent issue #88](https://github.com/tonywxx/adaq/issues/88). M10 and M11.1-M11.7 are complete; M11.8 acceptance evidence is recorded in the [bilingual manual acceptance guide](./m11-manual-acceptance.md).

## Outcome

M11 delivers one host-owned Factor Lab and a Tauri-independent `adaq-factor-research` core. Users can publish Declarative Factor Definitions or build private Custom Factor Candidates, materialize immutable Factor Datasets from Completed M10 Feature Datasets, evaluate exact outputs under scope-correct and cost-aware Protocols, retain every Trial in its Research Family lineage, and record explicit Rejected, Research Validated, or Component Eligible Promotion Decisions.

Research Validated outputs become exact selectable evidence for M12 Model research. M11 does not guarantee future profitability, automatically promote a score, import generated Components, or execute Python or Qlib.

## Boundary

M11 includes:

- Time-Series and Cross-Sectional Factor Scopes under one Factor product.
- Declarative Factor Definitions using Feature Operator Catalog semantics and Custom Factor Projects built into private non-imported Candidate Packages.
- Factor ABI v2 with host-resolved ordered Feature inputs and Scope-specific batches.
- Immutable Factor Materialization and Evaluation evidence.
- Chronological holdout and walk-forward evaluation with exact purge and embargo.
- Temporal, Cross-Sectional, and standardized Economic Lenses; neutralization, robustness, decay, stability, regimes, and multiple-testing evidence.
- User-owned Promotion Decisions and a derived Promoted Factor Library.
- A localized `/factors` workspace for Families, Candidates, Datasets, Evaluations, and Decisions.

M11 excludes:

- Qlib, Python, notebook execution, notebook-to-Rust translation, or a second Research Engine.
- Model training, Forecast Signal Dataset production, Strategy construction, or Backtest changes.
- Component Equivalence, qualified Package import, Marketplace publication, Paper, Bot, Live, or real-money work.
- Automatic promotion, automatic Feature mining, Bayesian optimization, genetic search, and mutable “latest” references.
- Cross-market sample pooling or universal profitability thresholds.

## Ownership

`adaq-factor-research` owns Factor materialization contracts, Factor Dataset evidence, the versioned Factor Metric Catalog, Evaluation Protocols and Reports, Research Families and Trials, Promotion Policies and Decisions, and Promoted Factor Library projections. `adaq-feature-engine` remains authoritative for Feature Definitions, Feature Plans, Availability, Warmup, Missingness, and completed Feature inputs. `adaq-component-tooling` and the SDK own Factor ABI v2, Candidate Package validation, WASM sandboxing, and package contracts.

M11 uses the ADAQ Native Research Engine only. Every result freezes Research Engine Provenance. A future Qlib adapter may produce comparable evidence under a distinct Engine identity but may not silently claim formula or numeric equivalence.

## Candidates, revisions, and identity

A Factor Candidate is one exact Declarative Factor Definition revision or private Custom Factor Package:

- A mutable Declarative Draft has no evidence identity. Publication creates a positive integer revision with canonical RFC 8785 JSON and lowercase SHA-256 identity.
- Declarative logic reuses Feature Operator Catalog operations and Plan 2.0 semantics; M11 adds no parallel expression language.
- Name, description, and Factor Tags are User-scoped presentation metadata outside semantic hashes.
- A Custom Factor Project is exact User-authored Rust source. A Candidate Build Attempt freezes the source hash, SDK, ABI, toolchain, target, commands, environment, resource policy, logs, and resulting Package hash.
- Candidate builds use fixed host commands with no network or custom scripts. A successful package remains private and non-imported; M14 owns qualification and Component Library import.
- Parameter search is an explicit deterministic Cartesian Grid of at most 256 Trials. M11 does not perform adaptive optimization.

The same Candidate hash, Target, Universe, window, or explicit derivation creates lineage between Research Families. Families and lineage cannot be deleted to evade multiple-testing evidence.

## Factor ABI v2

Every Factor declares one Scope, ordered Feature Slots, parameters, one through 64 named outputs, and exact runtime limits. Components cannot fetch data, inspect ambient time, refit transformations, read files, use network or randomness, or silently substitute inputs.

- Time-Series execution receives dense Present rows for one Instrument in causal Observation Time order. Missing input or a Bar Gap is published by the host as Unavailable; the host rebuilds the Instance and restarts Warmup rather than sending a partial row.
- Cross-Sectional execution receives every deterministically ordered member of one Point-in-Time Instrument Universe at one Observation Time. Each Slot cell is Available or typed Unavailable. The Component returns one identity-preserving result per member and may not delete or reorder rows.
- The host validates membership, order, row and output counts, availability, finite values, determinism, fuel, memory, and output identity before publication.

Factor ABI v2 directly replaces pre-v1 Factor ABI v1. Incompatible stored packages and evidence are rejected with typed `reset-required` and explicit device-level Reset guidance; M11 adds no migration, dual reader, or automatic deletion.

## Materialization and storage

Evaluation never computes a Candidate implicitly. A Factor Materialization Protocol binds one Candidate to the exact User, Feature Dataset and Plan, parameters, Market Data Snapshot, Point-in-Time Instrument Universe, observation range, market context, runtime and engine identities, and Seed. Only a Completed Factor Materialization Attempt atomically publishes a Factor Dataset.

One Factor Dataset retains one through 64 outputs in a wide Parquet row keyed by `(Instrument ID, Observation Time)`. Every output retains its finite `f64` value or typed Unavailable state, Available At, reason, and provenance. SQLite stores User-scoped metadata, canonical Protocols, Attempts, manifests, references, presentation records, and lifecycle state; payload bytes are immutable and content-addressed.

Exact active requests coalesce, exact Completed evidence is reused, Retry creates a new Attempt referencing its source, and failed or cancelled attempts retain safe diagnostics. Publication uses private staging, complete validation, and atomic cutover. Referenced evidence is deletion-locked; shared bytes disappear only after the last User reference without granting cross-User visibility.

## Target and market context

M11 supports only `Future Close Return` with one or more positive integer Bar horizons:

`close[t + h] / close[t] - 1`

The Factor output must be available at `t`. Target evidence uses the same Instrument, Bar Interval, Market Data Snapshot, and Price Basis. A Bar Gap, missing Close, or unverifiable Corporate Action across the horizon makes the Label typed Unavailable; Scheduled Closures do not count as gaps. Binary and Custom Targets remain future work.

One Dataset and Report binds one comparable market context: Venue, Asset Class, Bar Interval, Price Basis, Valuation Currency, and Point-in-Time Instrument Universe. Cross-market robustness uses separate Reports in one Research Family; it never pools raw observations across markets.

Each evaluated output freezes Positive or Negative Factor Orientation in the Protocol. Orientation controls interpretation and Economic sorting without modifying raw Dataset values.

## Evaluation Protocols and evidence

A Factor Evaluation Protocol binds one exact Factor Dataset output, Target and horizons, market and Feature evidence, Research Engine, Factor Orientation, chronological or walk-forward windows, purge, embargo, Lenses, neutralization, Economic assumptions, regimes, and Research Family Trial identity.

Chronological holdout and walk-forward are supported; random splitting is not. Every fold freezes its selection and evaluation windows. Evaluation Evidence State is Out-of-sample only when recorded research, parameter-selection, fitting, normalization, Target-construction, and evaluation windows are complete and non-overlapping. Overlapping and Unknown Reports remain inspectable but cannot support Research Validated or Component Eligible Decisions.

Required Lens coverage is:

- Time-Series Factor: at least one Temporal Lens and one Economic Lens.
- Cross-Sectional Factor: at least one Cross-Sectional Lens and one Economic Lens.
- Additional compatible Lenses are optional and independent from computation Scope.

The Factor Metric Catalog is the versioned authority for formulas, directions, ranges, required samples, and typed undefined states. The Rust core produces the machine-readable Catalog; GUI and bilingual references derive from it. An external Research Engine may adapt results to the contract but cannot silently substitute a same-named formula.

An undefined metric is never encoded as zero. Insufficient samples, constant values, singular matrices, unavailable Targets, or broken requirements produce typed Unavailable evidence with the output, Lens, fold/window, and applicable sample counts. A Report may complete with unavailable metrics, but any required unavailable metric blocks promotion.

## Neutralization, regimes, and Economic Lens

M11 neutralization is Cross-Sectional ordinary least squares at each Observation Time with an intercept and Protocol-selected nuisance Features. Complete cases determine the fit while the complete Universe and missingness remain in evidence. Insufficient samples or a singular design matrix makes the batch Unavailable. M11 adds no generic Time-Series neutralization.

A Regime Definition selects one causal Feature and fits deterministic bucket thresholds only on the frozen selection window. Those thresholds apply unchanged to evaluation observations. Reports retain the Feature, Artifact or threshold identity, coverage, and per-bucket results; M11 does not invent mutable Bull/Bear labels.

The standardized Economic Lens uses deterministic average ranks, five quantiles, and equal weights and reports Top-only and Top-minus-Bottom evidence. A value available at `t` may act no earlier than the next eligible Bar. Rebalance rules, fees, slippage, costs, and Long/Short feasibility are frozen. This is diagnostic research evidence, not a Strategy Component or ADAQ Backtest Run.

## Robustness and multiple testing

Reports retain coverage, missingness, IC, Rank IC, turnover, decay, stability, subperiod, regime, neutralized, and cost-aware results where applicable. Every metric preserves ordered values and sample counts needed to interpret aggregation.

Research Families retain Completed, Failed, Cancelled, Rejected, and Superseded Trials. Reports store raw statistics and p-values plus Holm-Bonferroni family-wise adjustments. Registered Trials without a statistic are non-significant rather than disappearing. Promotion freezes the complete applicable Family lineage; omission of known related Trials blocks promotion.

## Promotion

A Factor Promotion Policy is immutable and versioned. A conservative system template requires explicit minimum coverage, sample size, Holm-adjusted significance, subperiod sign consistency, cost-aware outcome, required Lenses, and complete provenance, but M11 does not hard-code universal IC or return thresholds. A changed threshold set creates a new Policy identity.

The system checks eligibility; the User decides. Each Factor Promotion Decision targets one exact named output and is immutable:

- `Rejected` records that the cited evidence was not accepted.
- `Research Validated` requires at least one Policy-satisfying Out-of-sample Report and permits exact selection by M12.
- `Component Eligible` includes all Research Validated gates plus deterministic execution, complete source provenance, ABI v2 expressibility, and buildability. M14 still performs Build, Conformance, Equivalence, qualification, and import.

A later Decision may cite and supersede an earlier Decision without mutating it. The Promoted Factor Library is a User-scoped read-only projection of current Decisions, not copied evidence or a floating latest-version store. A multi-output Dataset promotes outputs independently; a multi-output Custom Package becomes M14-eligible only when every public output is Component Eligible.

M12 may select only one exact Completed Factor Dataset output with a current Research Validated or Component Eligible Decision and must freeze the Dataset, Report, Decision, Policy, and Research Engine Provenance. It may not implicitly recompute or promote a Factor.

## Attempts, queue, and native APIs

Candidate Build, Factor Materialization, and Factor Evaluation Attempts use `Pending → Running → Completed | Failed | Cancelled`. Retry creates a new identity, Pending survives restart, stale Running becomes Failed with typed interruption evidence, and progress advances only after a complete work unit without invented ETA.

Feature and Factor heavy work share one persistent device-wide research FIFO. Attempt ownership and visibility remain User-scoped. Tauri commands validate canonical requests, enqueue, cancel, retry, and query paginated evidence; blocking filesystem, SQLite, Parquet, WASM, build, and statistical work executes in workers rather than command bodies or the UI thread.

Dataset, Candidate, Policy, Report, and Decision references enforce deletion locks. Unreferenced completed User links may be removed; failed, cancelled, and superseded Trial metadata and safe diagnostics remain. Explicit User Reset may clear that User's local research data under the established reset contract.

## Factor workspace

`/factors` paints its shell immediately and contains Families, Candidates, Datasets, Evaluations, and Decisions. Each card or control owns its loading, build, run, cancellation, error, and retry state. User-scoped read lists render current-session cache first and refresh in the background without weakening validation.

The workspace exposes immutable identities, lineage, market context, missingness, Target availability, fold boundaries, Lens formulas, ordered metrics and samples, multiple-testing adjustments, Policy gates, Decision history, deletion locks, and M12 eligibility in English (US) and Simplified Chinese. It never labels historical evidence as guaranteed, hides failed Trials, or offers an automatic Promote control.

## Resource and numeric contracts

M11 retains the existing 1 MiB canonical JSON, 64-output, WASM fuel and memory limits and sets Grid Search to at most 256 Trials. Dataset-row, Fold, Horizon, Lens, nuisance-column, and worker ceilings are measured and frozen before public APIs are accepted. Checked arithmetic and limits run before allocation or evaluation.

The same Engine Identity, inputs, Protocol, Seed, and build must produce bit-identical evidence independent of chunking. Different target, compiler, or platform builds retain distinct Engine identities. Golden fixtures may establish exact or declared tolerance-based cross-platform equivalence, but Reports with distinct Engine identities never share a hash.

Performance acceptance uses a 1,000,000-observation Time-Series workload and a 10,000-Instrument × 252-Observation-Time Cross-Sectional workload. It proves bounded memory, cancellation, chunk equivalence, determinism, restart recovery, and responsive GUI scheduling and records a canonical macOS ARM64 baseline without inventing latency or RSS targets.

## M11.7 hardening evidence

Issue #94 freezes the following public ceilings from the canonical candidate workload rather than from a latency promise: 2,520,000 Factor Dataset rows (10,000 instruments × 252 observations), 32 evaluation folds, 16 horizons, 5 lenses, 16 nuisance Features, and one device-wide research worker. Checked arithmetic runs before Dataset or evaluation allocation.

The committed evidence is independent of the implementation under test:

- `src-tauri/crates/adaq-factor-research/fixtures/factor-reference-vectors.json` covers OKX Spot Time-Series, China A-share session and Corporate Action evidence, and U.S. Cross-Sectional Point-in-Time Universe and missingness journeys. Cross-platform floating-point comparison declares a `1e-12` metric tolerance and hashes the normalized vector, while identities, unavailable reasons, samples, and ordering remain exact.
- `factor-metric-golden.json` contains literal average-rank, undefined-state, singular-OLS, raw p-value, Holm, and cost vectors; `factor-metric-catalog.json` is regenerated by `scripts/check_generated.sh` and fails on drift.
- `factor-benchmark-baseline.json` records the canonical macOS ARM64 run: 1,000,000 Time-Series Bars in 723 ms, 10,000 × 252 Cross-Sectional observations in 1,548 ms, 29,884,416 bytes process high-water RSS, and the two Candidate Package hashes. These are recorded measurements, not SLA thresholds.

Reproducible commands are:

```sh
cd src-tauri
cargo test -p adaq-factor-research --test reference_fixtures
cargo test -p adaq-factor-research --test metric_golden
cargo test -p adaq-factor-research --test benchmarks -- --test-threads=1
cargo test -p adaq-factor-research --release --test benchmarks -- --ignored --test-threads=1
sh crates/adaq-factor-research/scripts/check_generated.sh
```

The native Factor tests retain Parquet atomic publication, cancellation, restart recovery, queue fairness, User isolation, deletion locks, and credential/path redaction in `src-tauri/src/factor_research/mod.rs`. The supported-platform workflow runs the generated-reference gate and the canonical benchmark on macOS ARM64, alongside the full workspace matrix on Windows x86_64 and Linux x86_64.

## Acceptance

Reference journeys cover:

1. OKX Spot Time-Series momentum with multiple horizons, Bar Gap restart, Temporal and cost-aware evidence.
2. China A-share Time-Series evidence across Venue sessions and Corporate Actions with causal Target availability.
3. U.S. equities Cross-Sectional evidence with Point-in-Time Universe membership, neutralization, Rank IC, turnover, regimes, and Unknown/Reconstructed Universe behavior.

Failure coverage includes Factor ABI v1 Reset, Candidate build failure, missing input, non-finite output, Universe mismatch, singular neutralization, undefined metrics, leakage, Family-lineage omission, Policy failure, cancellation, restart recovery, atomic publication, User isolation, and deletion locks.

Every child maps each Acceptance Criterion to implementation and independent evidence. Final gates include focused tests, `cargo fmt --all --check`, `cargo test --workspace`, `cargo check --workspace`, Factor ABI/component conformance, frontend Jest, `pnpm run build`, lint, `git diff --check`, bilingual parity, accessibility, secret scanning for retained build evidence, and supported-platform CI.

## Delivery slices

M11 is published through eight dependency-ordered slices:

1. [#92 — Core contracts, Factor ABI v2, and Factor Metric Catalog](https://github.com/tonywxx/adaq/issues/92).
2. [#90 — Declarative and Custom Candidate execution and Factor Dataset materialization](https://github.com/tonywxx/adaq/issues/90).
3. [#89 — Targets, Lenses, neutralization, Economic diagnostics, and robustness evaluation](https://github.com/tonywxx/adaq/issues/89).
4. [#91 — Research Families, Grid Search, multiple testing, Promotion Policies, and Decisions](https://github.com/tonywxx/adaq/issues/91).
5. [#95 — SQLite/Parquet evidence, shared research FIFO, and User-scoped native APIs](https://github.com/tonywxx/adaq/issues/95).
6. [#96 — Localized `/factors` workspace](https://github.com/tonywxx/adaq/issues/96).
7. [#94 — Three-market fixtures, benchmarks, resource limits, and hardening](https://github.com/tonywxx/adaq/issues/94).
8. [#93 — Bilingual cross-platform acceptance, manual guide, and roadmap closure](https://github.com/tonywxx/adaq/issues/93).

Dependencies are `#92 → #90 → #89 → #91 → #95 → #96`, `{#90,#89,#91,#95} → #94`, and `{#92,#90,#89,#91,#95,#96,#94} → #93`. #92 was the only initial executable frontier. The final cross-slice evidence and supported-platform record are maintained in the [M11 manual acceptance guide](./m11-manual-acceptance.md); #93 is the acceptance record for this completed milestone.
