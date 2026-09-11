# Feature Engineering

[简体中文](./feature-engineering.zh-CN.md)

Status: accepted architecture for the host-owned Feature Engineering workspace.

## Outcome

Feature Engineering delivers one host-owned, Tauri-independent `adaq-feature-engine` that turns immutable market evidence into causal, reproducible Feature evidence. Users can publish Feature Definitions, fit declared transformations, freeze Feature Plans, materialize immutable Feature Datasets, inspect their provenance and missingness, and reuse finalized Datasets in Factor research.

The same Plan and operator state machine serves historical batch materialization and stateful observation evaluation. The engine proves their equivalence but does not connect the online evaluator to a Paper Provider or Trading Bot.

## Boundary

The feature engine includes:

- Pointwise, Time-Series, and Cross-Sectional Feature Scopes.
- A finite versioned Feature Operator Catalog rather than scripts or a general expression language.
- Feature Plan `2.0.0`, replacing the pre-v1 consumer-only Plan `1.0.0`.
- Exact availability, Warmup, missingness, typed errors, fitting, immutable evidence, and User-scoped lifecycle records.
- A localized `/features` workspace for Definitions, Fitting Attempts, Materialization Attempts, and Datasets.

It excludes:

- Arbitrary Python, JavaScript, Rust, notebook, or `adaq:feature` Component execution.
- Factor research, promotion, Model training, Strategy construction, Paper orders, Bots, and Marketplace work.
- Implicit fitting or Feature materialization from later workflows.
- Future-return Features, future-known backward adjustment, silent imputation, forward-fill, row deletion, or Canonical Market Data mutation.
- Feature Dataset export and a drag-and-drop graph canvas.

Custom deployable analytical logic remains a Factor Component. A finalized Feature Dataset is the explicit downstream boundary.

## Ownership

`adaq-feature-engine` owns Feature Definitions, the Feature Operator Catalog, Feature Plan validation and canonical identity, evaluation, missingness, fitting contracts, and batch/observation equivalence. `adaq-indicator-engine` remains the specialized TA-Lib subengine. `adaq-component-tooling` adapts Component Manifests and ordered Feature Slots into Plan inputs. Application commands validate User-scoped requests and create or query Attempts; cancellable blocking workers perform heavy work.

This supersedes only the Plan schema and ownership portions of ADR 0012 and ADR 0020. Existing canonical hashing, authoritative Slot order, finite dense Component inputs, and the Indicator Engine boundary remain.

## Identity and schemas

- A Feature Definition family has a random stable `definitionId`, positive integer `revision`, and JCS SHA-256 `definitionHash`.
- Mutable name, description, and tags are User-scoped presentation metadata outside the semantic hash.
- A Feature Plan is canonical RFC 8785 JSON with lowercase SHA-256 identity and `planSchemaVersion: "2.0.0"`.
- A Plan freezes Definition revisions, ordered outputs, Feature Scopes, operator parameters, Fitted Transformation Artifacts, Warmup, availability, missingness, the Feature Operator Catalog, Feature Engine, Indicator Engine, target/build identities, and Seed.
- A Plan is reusable and does not bind one Snapshot, Universe, or observation range.
- A Feature Materialization Request binds User, Plan, Market Data Snapshot, Point-in-Time Instrument Universe, observation range, parameters, and Seed.
- Pre-v1 incompatible stored Feature schemas are rejected with explicit device-level Reset guidance. The engine adds no migration, dual reader, or automatic deletion.

Canonical Definition and Plan JSON is limited to 1 MiB, 256 DAG nodes, 64 ordered outputs, DAG depth 64, and 100,000 effective Warmup Bars. Dataset and runtime ceilings come from recorded engine benchmarks.

## Feature semantics

Every Feature Observation is identified by Feature output, Instrument ID, and Observation Time. It contains either a finite analytical `f64` with Available At or a typed Unavailable state. Canonical Decimal inputs remain authoritative; checked conversion to analytical `f64` does not claim decimal bit-exactness.

Feature Scope is explicit:

- Pointwise reads one Instrument observation.
- Time Series reads one Instrument in causal Observation Time order.
- Cross Sectional reads one complete Point-in-Time Instrument Universe at one Observation Time.

Dependency expansion is limited to Pointwise → Time Series → Cross Sectional. Cross-Sectional outputs are terminal. A Cross-Sectional Plan binds one Venue, Asset Class, Bar Interval, Price Basis, and Valuation Currency. Observed and Reconstructed Universes may materialize with their exact evidence state; Unknown makes the complete batch Unavailable.

Available At is the latest availability among all inputs and any Fitted Transformation Artifact. Corporate Action facts use recorded publication and effective evidence. Local computation time is operational metadata, not historical identity.

Unavailable input affects only dependent branches. A dependent stateful branch does not consume it and restarts Warmup; independent branches continue. Stable reasons are:

- `warmup`
- `bar-gap`
- `missing-market-input`
- `missing-dependency`
- `unknown-universe`
- `insufficient-coverage`
- `undefined-arithmetic`
- `artifact-missing-instrument`
- `corporate-action-unavailable`

Expected undefined arithmetic is Unavailable. A non-finite Indicator or Feature Engine output, broken shape, invalid identity, or other invariant failure is a fatal typed Feature Evaluation Error with stage, node, Instrument, Observation Time, and safe diagnostics.

## Feature Operator Catalog 1.0

The initial catalog includes:

- Market OHLCV fields and checked arithmetic.
- TA-Lib Indicators through `adaq-indicator-engine`.
- Backward Simple Return and Log Return only.
- Full-window rolling mean, population standard deviation, minimum, maximum, and Realized Volatility.
- Quote Volume, rolling Quote Volume, zero-volume state, and unit-preserving Amihud Illiquidity. Quote Volume is not called turnover, and no Turnover Ratio is invented without a trustworthy denominator.
- Venue-local trading day of week, trading month, minutes from session open, minutes to session close, Session Progress, one-hot, and sine/cosine encodings.
- Cross-Sectional Rank, Percentile, and Z-score.
- Causal forward Split adjustment and a separate Dividend Total Return Feature.
- Fitted Standardization and Winsorization.

Rolling windows count consecutive eligible Closed Bars in one Continuous Bar Segment, require a full window, exclude Scheduled Closures, and restart after a Bar Gap or unavailable dependency. Realized Volatility is per Bar; annualization is a separate calendar-bound operator.

Cross-Sectional Rank uses ascending average ties. Percentile is `(rank - 1) / (n - 1)`; `n = 1` is Unavailable, and reverse order is explicit. Z-score uses population variance and zero variance is Unavailable. Coverage policy freezes minimum count and coverage, defaults to 100% coverage, preserves all Universe members, and records actual coverage when an explicit lower threshold permits an available subset.

## Fitted transformations

A Transformation Fitting Protocol binds one fitted node and exact fitted output Feature, input Feature, Snapshot, Universe, Fitting Scope, fitting window, algorithm parameters, engine identity, and required `minimumSamples`. Fitting Scope is Pooled Universe or Per Instrument. Walk-forward work creates one Protocol and Artifact per fold; instantaneous Cross-Sectional Z-score is not fitted.

A completed Fitting Attempt publishes one immutable Artifact per fitted node. Standardization uses population variance and constant input is Unavailable. Winsorization freezes lower and upper quantiles and the nearest-rank rule. Insufficient samples fail without publishing an Artifact. Materialization applies an Artifact and never refits it.

Artifact Eligible At is the latest Available At among fitting inputs. Created At records operational completion without changing historical identity. A Paper Deployment may reference an Artifact only after it actually exists and passes later qualification.

## Attempts, storage, and recovery

SQLite stores User-scoped Definitions, Plans, Protocols, Artifacts, Requests, Attempts, manifests, references, lifecycle state, and presentation metadata. Immutable Feature Dataset rows use content-addressed Parquet with one wide row per `(Instrument ID, Observation Time)`. Every output retains value, Available At, state, and versioned reason code through a canonical Manifest mapping.

Fitting and Materialization Attempts move through Pending, Running, and exactly one terminal state: Completed, Failed, or Cancelled. Exact Pending or Running requests coalesce, exact Completed evidence is reused, Retry creates a new Attempt retaining source evidence, and repeated Retry requests coalesce with an active Pending or Running retry.

The device runs one heavy Feature Attempt at a time through persistent FIFO. Pending survives restart. Stale Running becomes Failed with interruption evidence. Progress advances only after complete Feature Observations and exposes no invented remaining-time estimate.

Materialization writes private staging files, validates complete schema, rows, and hashes, atomically publishes the payload, and only then records Completed. Cancellation and crashes expose no partial Dataset. Referenced Datasets and Artifacts are deletion-locked; deduplicated bytes are removed only after the last User reference disappears, without granting cross-User visibility.

## Historical and observation execution

Pointwise and Time-Series branches stream by Instrument and Continuous Bar Segment. Cross-Sectional branches process one complete Observation-Time batch. Chunk size never enters identity or changes output. Batch materialization and stateful observation evaluation must produce equivalent Feature Observations across chunk boundaries, Bar Gaps, missing dependencies, and restart reconstruction.

Factor research consumes only Completed Feature Datasets. Feature Definitions cannot depend on Factor outputs; the Component adapter preserves existing Strategy and Model Slot bindings to external Factors without allowing a Definition cycle.

## Feature workspace

`/features` paints immediately and contains Definitions, Fitting Attempts, Materialization Attempts, and Datasets. Each control owns its loading state; User-scoped read lists may paint from current-session cache and refresh in the background.

The Definition editor is an accessible ordered node list, not a canvas. Each node exposes operator, inputs, parameters, output names, Scope, availability, and Warmup. Preview uses the production engine over a bounded immutable Snapshot selection, may restrict Observation Times, preserves the complete Universe for Cross-Sectional work, never fits, and creates no evidence identity.

Dataset inspection shows Manifest and provenance, per-output coverage, Unavailability reason counts, minimum, maximum, mean, population standard deviation, and filtered 50-row pagination by Instrument, time, output, and state. The rebuildable Summary is content inspection, not Factor or Model evaluation.

## Verification

Reference journeys are:

1. OKX Spot: Return, RSI, Realized Volatility, Bar Gap, and chunk equivalence.
2. China A-share: Venue Calendar, midday break Session Progress, Split, Dividend, and causal availability.
3. U.S. equities: Point-in-Time Universe, Cross-Sectional Rank, coverage, and Reconstructed/Unknown behavior.

Failure coverage includes fitting leakage, insufficient samples, undefined arithmetic, non-finite engine output, cancellation, interruption recovery, atomic publication, incompatible schema rejection, User isolation, deletion locks, and batch/observation equivalence.

Workspace verification covers the Definition lifecycle, Fitting Protocols and Artifacts, Dataset materialization and attempts, Dataset inspection, semantic proofs (batch/observation equivalence, causal availability, chunk and restart determinism), User isolation and evidence boundaries, and the localized accessible `/features` GUI at 1024 px.

Performance verification uses a 1,000,000-Bar Time-Series workload and a 10,000-Instrument × 252-Observation Cross-Sectional workload. It proves bounded memory, cancellation, chunk equivalence, and responsive GUI scheduling and records the canonical macOS ARM64 baseline without inventing an advance latency or RSS target.

Final gates include focused tests, `cargo fmt --all --check`, `cargo test --workspace`, `cargo check --workspace`, frontend Jest (`pnpm test`), `pnpm run build`, lint, `git diff --check`, bilingual parity, accessibility, and supported-platform CI evidence. Focused native suites live in `src-tauri/crates/adaq-feature-engine/tests/` (`contracts.rs`, `operators.rs`, `fitting.rs`, `materialization.rs`, `reference_fixtures.rs`).
