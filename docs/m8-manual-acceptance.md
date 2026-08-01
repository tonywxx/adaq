# M8 Manual Acceptance (macOS ARM64)

This is the canonical human-reviewed M8 path. Perform one row at a time and retain the requested evidence on failure. Forecast Evaluation measures prediction evidence; Backtest and Validation measure Strategy behavior. None is a profitability claim, live trading, Verified external inference, or Marketplace approval.

<!-- m8-acceptance:prerequisites -->
## 1. Prerequisites, sign-in, and storage

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| From the repository root run `pnpm install --frozen-lockfile`, `rustup toolchain install stable`, `rustup target add --toolchain stable wasm32-unknown-unknown`, `cargo install cargo-component --locked`, and `cargo install --force --path src-tauri/crates/adaq-component-tooling`. | Node dependencies and the stable Rust/Component toolchain install; `adaq-component --help` lists `new`, `build`, and `verify`. | The failing command, complete output, `node --version`, `pnpm --version`, and `rustc --version --verbose`. |
| Configure `VITE_SUPABASE_URL` and `VITE_SUPABASE_PUBLISHABLE_KEY` outside version control, then run `pnpm tauri dev`. | The desktop sign-in screen appears without a missing-configuration message. | The exact message and variable names only; never record values, passwords, OTPs, or tokens. |
| Sign in with an existing test account by email and password. | Dashboard, sidebar, and the current User's research data appear. | Visible error and expandable technical details, with secrets redacted. |
| Open **Settings → Data & Storage**, read the summary, then select **Open Data Folder** without resetting anything. | The current local-data counts are readable and the app data folder opens; account/login and device preferences are outside research-data reset scope. | Screenshot, visible/technical error, and OS version. |

Windows uses PowerShell, `py -3.12 -m venv .venv`, `.\.venv\Scripts\Activate.ps1`, and `cargo install --force --path .\src-tauri\crates\adaq-component-tooling`. Linux uses the same POSIX commands as macOS. Native file pickers and data-folder locations are platform-specific. The canonical reviewed run below is macOS ARM64.

<!-- m8-acceptance:components -->
## 2. Author, build, verify, import, and inspect Components

Create projects in a new empty directory; committed examples are references, not substitutes for generation.

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| Run `adaq-component new model m8-forecast-model`, `adaq-component new strategy m8-signal-strategy`, `adaq-component new strategy m8-hybrid-strategy`, and `adaq-component new strategy m8-composed-strategy --template composed`. | Four projects are created with distinct generated UUIDs; Model/default Strategy/composed Strategy use the correct SDK features and all Manifests remain `kind: model` or `kind: strategy`. | Command output and generated `Cargo.toml`/`manifest.json`; do not regenerate or shorten IDs. |
| In `m8-forecast-model`, replace `src/lib.rs` with the deterministic identity-preserving implementation from [`model-close-score`](../examples/components/model-close-score/src/lib.rs). For each row set `let normalized = (row.values.first().copied().unwrap_or_default() / 100.0).sin();` and return `values: vec![normalized / 100.0, (normalized + 1.0) / 2.0, (normalized + 1.0) / 2.0]`. Replace the generated output contract with three uniquely named horizon-1 outputs: `expected-return` = Expected Value + `future-close-return` + native scale; `up-probability` = Probability + `future-close-up` + probability scale; `return-score` = Score + `future-close-return` + percentile scale. Preserve the generated `componentId`, versions, market `close` Feature Slot, Single-Instrument scope, embedded Artifact SHA-256, and `warmupBars: 0`; add Artifact provenance strings `trainingWindow`, `fittingWindow`, and `normalizationWindow` with value `0..0`. | One real batch Model preserves Instrument ID, Prediction Time, row order, and emits varying finite deterministic values under valid contracts; its complete non-overlapping provenance can produce Out-of-sample evaluation evidence. | Complete source, Manifest, and verifier typed error. Compare field shapes with [`model-close-score/manifest.json`](../examples/components/model-close-score/manifest.json). |
| Leave `m8-signal-strategy` as generated. In `m8-hybrid-strategy`, add a market `close` Feature Slot beside `forecast-probability`, bind both indexes, and emit `1` only when probability is at least `0.5` and close is positive. Leave `m8-composed-strategy` as generated. | The three authoritative graphs are respectively Signal-driven, Hybrid, and Composed; no author-controlled Architecture field exists. | All three sources/Manifests and the exact conformance error. |
| In each project run `adaq-component build`, then run `adaq-component verify dist/<project-name>-0.1.0.adaq`. | Each self-contained package passes tests, Wasm build, conformance, size/integrity checks, and verification. Record each archive SHA-256. | Full output, package path/size, Manifest, and archive SHA-256. |
| In **Components**, use **Import Component Package** for the Model and all three Strategies, then select each item. | Exact IDs, versions, hashes, compatibility, Model outputs/Artifact provenance, Strategy Feature Slots, and derived Architecture are readable and copyable. | Selected file, visible error, expandable technical details, screenshot, and exact IDs/hashes. |

<!-- m8-acceptance:native-dataset -->
## 3. Native Forecast Signal Dataset

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| In **Backtest → Data**, choose `BTC-USDT`, `1h`, a contiguous range with at least 100 Bars, select **Prepare Snapshot**, and retain the exact Snapshot ID. | A user-scoped immutable Snapshot appears with venue, Instrument, interval, range, Bar count, gaps, and ID. | Inputs, progress/error, Snapshot ID if created, and expandable technical details. |
| In **Models → Create Dataset**, select `m8-forecast-model`, the exact Snapshot, then **Create Dataset** once. | A painted accessible busy state appears before native work; duplicate starts are suppressed; the Attempt moves Pending → Running → Completed and publishes exactly one Dataset. | Attempt ID, status/progress, diagnostic evidence, Model package hash, Snapshot ID, Seed, and technical error. |
| Open **Signal Datasets**, expand **Rows** and **Provenance**, and page once if more than ten rows exist. | Dataset/Parquet hashes, exact Snapshot, Feature Plan, Component Lock, three Signal contracts, Seed, `verified-package` trust, Artifact, one Producer Segment, engine identity, coverage, Present/unavailable rows, `availableAt`, Warmup/MissingInput, and Bar-Gap rules are inspectable and copyable. | Dataset ID, hashes, failed row/page, screenshot, and technical details. |

<!-- m8-acceptance:external-dataset -->
## 4. External Kronos evidence

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| From `examples/external-models/kronos`, create the documented Python 3.10–3.12 environment and run `python -m unittest test_adapter.py`. | Two deterministic fixture tests pass without downloads or a GPU; the golden `.adaq-signals` transformation remains byte-stable. | Python/platform versions, `python -m pip freeze`, command, complete traceback, and fixture SHA-256. |
| Follow [External Kronos Adapter](../examples/external-models/kronos/README.md) using the exact Snapshot ID from section 3, pinned `Kronos-small` and `Kronos-Tokenizer-base` revisions, Seed `7`, and CPU/MPS/CUDA explicitly selected; or, when hardware/network prevents execution, complete and retain the guide's download/runtime evidence record. | A Snapshot-aligned `kronos-small.adaq-signals` is produced, or the unavailable real-inference path is honestly documented without claiming it ran. The Tokenizer is not treated as the inference model. | Exact revision/URLs, licences, hashes, runtime/config, Seed, device, peak memory, elapsed time, full error, and no credentials/private data. |
| In **Models → Signal Datasets**, select **Import .adaq-signals**, choose the produced archive, inspect it, then **Export .adaq-signals** to a new path. | Import validates and atomically publishes the exact external Dataset; it remains **Externally Generated** with Producer Segment, Artifact/weight/Tokenizer/Adapter/preprocessing hashes, unknown training evidence, Snapshot alignment, availability policy, and identical authoritative export identity. | Archive SHA-256/size, reviewed Manifest, exact typed error, whether a Dataset appeared, Dataset ID, and export path/error. |

<!-- m8-acceptance:evaluation -->
## 5. Expected Value, Probability, and Score evaluation

Repeat the following operation for `expected-return`, `up-probability`, and `return-score` from the native Dataset. Use the Dataset coverage bounds and stability window `20`.

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| In **Models → Evaluation Reports**, select the Dataset and Signal, enter its full start/end milliseconds and `20`, then select **Create Report**. | Immutable Reports show common coverage/missingness/distribution/stability plus MAE/RMSE/bias/correlation for Expected Value, Brier/Log Loss/ROC AUC/calibration for Probability, and time-series Pearson IC/Spearman IC/window IC/ICIR/quantiles for Score. | Dataset/Signal/horizon/window, typed error, unavailable rows, and Report ID if created. |
| On every rendered metric, focus its adjacent information control with Tab, open it with keyboard, close it, then open it by click and pointer hover. | Meaning, formula, interpretation direction, range, caveat, undefined state, and reference link are available without color or hover alone and never turn prediction quality into Strategy profitability. | Metric label, interaction mode, screenshot, focus state, and accessibility-tree text. |
| Expand each Report's **Evidence** and **Provenance**, then select **Export JSON** and **Export Markdown** to new file names. | Producer-level evidence and unavailable results remain visible; exact Dataset/Snapshot/Segments/Artifacts/contracts/hashes/trust/versions are preserved; existing files are not overwritten. | Report ID, Evidence State, export name, visible/technical error, and exported file if safe. |
| Copy the Model project, keep its `componentId`, change version to `0.1.1`, change all three provenance windows to `0..9999999999999`, then build, verify, import, generate a second Dataset from the same Snapshot, and create one Report. Also create a Report from the imported Kronos Dataset. | The original native Reports are **Out-of-sample**, the `0.1.1` Report is **Overlapping**, and the Kronos Report is **Unknown**; warnings remain explicit and strong metrics never upgrade trust or evidence. | Both Package/Dataset/Report IDs, Segment windows, all three computed states, warnings, and provenance JSON. |

<!-- m8-acceptance:backtests -->
## 6. Signal-driven, Hybrid, and Composed Backtests

Run the next rows once for each imported Strategy, always using the exact section-3 Snapshot and a subset window inside it.

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| In **Backtest → Strategy**, choose the Strategy. For Signal-driven/Hybrid bind `forecast-probability` only to the compatible native `up-probability` Dataset Signal; leave the Composed Strategy with its generated Market slots. | The displayed Architecture is Signal-driven, Hybrid, or Composed; incompatible Dataset Signals are absent, and no Model is invoked by Backtest. | Strategy/package hash, slot, available candidates, Dataset/Signal contract, and gate message. |
| Select **Execution**, set allocation `10000`, Seed `48`, use the full default Spot Execution Profile, set a valid Dataset subset window, select **Validate inputs**, inspect **Authoritative inputs**, then **Run Backtest**. | One deterministic immutable Run completes. Signal values are consumed only when `availableAt <= decisionTime`; fills cannot precede the next Bar; unavailable aligned values produce `Run Pause::MissingInput`. | Preflight, typed error, Snapshot/Dataset/Signal binding, Run ID if created, and status/pauses. |
| Inspect **Overview**, **Decisions**, **Execution**, and **Provenance**; exercise every metric information control; then select **Use as new configuration**. | Results, decisions/pauses, orders/fills/fees, exact Feature Plan, Architecture, Component/Dataset Locks, Evidence State, Producer provenance, engine identities, seed, and run window are present; copy-as-new does not mutate the historical Run. | Run ID, failed tab/control, screenshot, copied values, and technical details. |
| In **Validation**, select each completed Run and confirm it can seed a new immutable Protocol without changing its Snapshot or Signal evidence. | Signal-driven, Hybrid, and Composed Runs are reusable validation evidence with their original immutable identities. | Run/Protocol ID, Snapshot ID, mismatch message, and technical details. |

<!-- m8-acceptance:negative-paths -->
## 7. Required negative paths

Perform these on disposable package/archive copies or new Attempts; never edit finalized evidence.

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| In a disposable Model Manifest set `horizonBars` to `0`, run `adaq-component build`, restore it, then set Probability to `future-close-return` and build again. | Both invalid Package contracts fail before import with stable technical evidence; no package is accepted. | Mutated Manifest, command, complete typed error, and whether `dist/` changed. |
| Copy a valid `.adaq-signals`, add an unexpected ZIP entry or alter `signals.parquet`, then import the copy. | The malformed/hash-mismatched archive is rejected atomically and no Dataset appears. | Archive SHA-256/size, ZIP listing or changed hash, exact error, and Dataset list state. |
| In Backtest select a different Snapshot/Instrument/interval from the Dataset, then try to bind it; separately try to bind Expected Value to the Probability slot. | Snapshot mismatch and incompatible Strategy binding fail before execution; no approximate join, resampling, forward-fill, or mixed Snapshot is offered. | Snapshot/Dataset/slot identities, candidate list, and exact gate/error. |
| Start a new native Dataset Attempt and select **Cancel** while Running; then select **Retry**. Separately run a disposable Model that returns a non-finite output. | Cancelled and Failed Attempts retain configuration, progress, bounded diagnostics, and publish no partial Dataset; Retry creates a new Attempt. | Both Attempt IDs, terminal states, diagnostics, Dataset list before/after, and technical errors. |
| Inspect a Dataset row marked Warmup/MissingInput or use an external Signal whose `availableAt` is after decision time, then run the compatible Strategy. | The Run records `Run Pause::MissingInput`; it never substitutes zero, flat exposure, a shifted row, or future evidence. | Row identity/status/availableAt, decision time, Run ID, pause evidence, and Dataset lock. |
| From `src-tauri`, run `cargo test datasets_lock_their_component_artifacts --lib`; then use **Settings → Data & Storage** to review the relevant reset confirmation without confirming it. | The focused lock check proves referenced Dataset/Artifact deletion is rejected; reset copy names deleted/preserved scopes and requires explicit confirmation while preserving account/login and device preferences. | Test output, referencing IDs, lock/reset message, summary counts, and technical details; do not perform a destructive reset for acceptance. |

<!-- m8-acceptance:regressions -->
## 8. Desktop regression review

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| At a 1024 px-wide app window, visit Dashboard, market-data views, Components, Models' three tabs, Backtest, Validation, and all Settings sections using keyboard navigation. | Content remains usable without clipped actions; focus is visible; tabs, controls, tables/cards, pagination, status, warnings, and errors are keyboard accessible and use text/icon meaning rather than color alone. | OS/display scale, page/tab, screenshot, focused element, and accessibility text. |
| Use titlebar Back/Forward across Models tabs and Backtest/Validation, then return after another page visit. | Route history and tab restoration return to the expected business page/tab without showing initialization. | Exact navigation sequence, expected/actual route and tab, screenshot, and console/technical error. |
| Sign out, sign in as a different test User, then revisit Components, Models, Backtest, Validation, Settings summary, and market-data views. | User-scoped Components, Attempts, Datasets, Reports, Runs, and Snapshot access do not leak; immutable IDs remain stable only for evidence the second User may legitimately access. | Both redacted User IDs, page, leaked/missing record ID, screenshot, and technical details. |

<!-- m8-acceptance:automated-gates -->
## 9. Automated release gates and CI

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| From `src-tauri` run `cargo test --workspace`, `cargo check --workspace`, and `cargo fmt --all --check`. | All Rust unit, integration, doc, type, and format checks pass. | Revision, failing command, complete unfiltered failure, test name, and backtrace if available. |
| From the repository root run `pnpm exec jest --watchman=false --runInBand`, `pnpm run build`, `pnpm run lint`, and `git diff --check`. | All Jest suites, strict TypeScript/Vite build, scoped lint gate, and whitespace check pass; record any explicitly pre-existing lint warnings. | Revision, command, complete error, suite/file, and warning delta. |
| From `examples/external-models/kronos` run `python -m unittest test_adapter.py`; build/verify the four generated projects again from clean `dist/` directories. | Adapter fixture and generated Model/Strategy package regressions pass. | Environment versions, complete output, package path/hash, and changed artifact listing. |
| After pushing the acceptance commit, record every applicable GitHub Actions run URL, commit SHA, platform/job, and conclusion. | Required multi-platform checks for the reviewed revision complete successfully; a local pass does not replace CI. | Run URL/SHA, failed job/platform, and relevant unredacted log excerpt. |

<!-- m8-acceptance:acceptance-record -->
## 10. Acceptance record

Record: macOS version/architecture and display scale; AdaQ revision; Rust/CLI/Node/pnpm/Python versions; four package hashes; User ID only in the private record; native/external Dataset and Parquet hashes; Snapshot, Attempt, Artifact, Producer Segment, Feature Plan, Report, Run, Protocol, and Validation Report IDs; three evaluation states; JSON/Markdown and `.adaq-signals` export names/hashes; negative-path evidence; accessibility/1024px review; and CI URLs/conclusions. Redact credentials, OTPs, tokens, Supabase values, private paths, and private market data.

The maintainer and agent review this record one operation at a time. M8 is accepted only after every row above has passed or an explicitly optional real-Kronos run has complete unavailable evidence, all automated gates and applicable CI are green, and the reviewed record contains no unresolved failure.

## Delivered scope boundary

M8 delivers offline Single-Instrument inference, immutable native/external Forecast Signal evidence, Forecast Evaluation, Dataset-first Signal-driven/Hybrid Backtests, and the existing Composed path. It does not deliver training/fitting/tuning, embedded Qlib/Python, Cross-sectional inference, generated future paths as realized data, live trading, Portfolio Optimization, OMS/EMS, a controlled GPU/ONNX Runner, Marketplace publishing, or future-profitability claims.
