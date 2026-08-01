# M8 Manual Acceptance (macOS ARM64)

This is the canonical human-reviewed M8 path. Perform one row at a time and retain the requested evidence on failure. Forecast Evaluation measures prediction evidence; Backtest and Validation measure Strategy behavior. None is a profitability claim, live trading, Verified external inference, or Marketplace approval.

<!-- m8-acceptance:prerequisites -->
## 1. Prerequisites, sign-in, and storage

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| Run `node --version` from the repository root. | Node.js 24.x is selected, matching release CI. | Complete output and the Node installation method. |
| Run `corepack prepare pnpm@11.18.0 --activate`. | The repository's pinned pnpm version is activated. | Complete output and `corepack --version`. |
| Run `pnpm install --frozen-lockfile`. | Dependencies match `pnpm-lock.yaml`. | Complete output, `node --version`, and `pnpm --version`. |
| Run `rustup toolchain install stable`. | The stable Rust toolchain is installed. | Complete output and `rustup show`. |
| Run `rustup target add --toolchain stable wasm32-unknown-unknown`. | The Component build target is installed for stable. | Complete output and `rustup target list --installed --toolchain stable`. |
| Run `cargo install cargo-component --locked`. | `cargo component --version` succeeds. | Complete output and `rustc --version --verbose`. |
| Run `cargo install --force --path src-tauri/crates/adaq-component-tooling`. | `adaq-component --help` lists `new`, `build`, and `verify`. | Complete output and `cargo --version`. |
| Configure `VITE_SUPABASE_URL` and `VITE_SUPABASE_PUBLISHABLE_KEY` outside version control, then run `pnpm tauri dev`. | The desktop sign-in screen appears without a missing-configuration message. | The exact message and variable names only; never record values, passwords, OTPs, or tokens. |
| Sign in with an existing test account by email and password. | Dashboard, sidebar, and the current User's research data appear. | Visible error and expandable technical details, with secrets redacted. |
| Open **Settings → Data & Storage**, read the summary, then select **Open Data Folder** without resetting anything. | The current local-data counts are readable and the app data folder opens; account/login and device preferences are outside research-data reset scope. | Screenshot, visible/technical error, and OS version. |

Windows uses PowerShell, `py -3.12 -m venv .venv`, `.\.venv\Scripts\Activate.ps1`, `cargo install --force --path .\src-tauri\crates\adaq-component-tooling`, backticks for multiline Adapter commands, `C:\path\to\adaq.db`, `$env:TEMP\kronos-small.adaq-signals`, and `Get-FileHash -Algorithm SHA256 <path>` instead of `shasum -a 256`. Linux uses `python3.12 -m venv .venv`, `. .venv/bin/activate`, `/path/to/adaq.db`, `/tmp/kronos-small.adaq-signals`, backslashes for multiline commands, and `sha256sum`; macOS uses `shasum -a 256`. Native file pickers and data-folder locations are platform-specific. The canonical reviewed run below is macOS ARM64.

<!-- m8-acceptance:components -->
## 2. Author, build, verify, import, and inspect Components

Create projects in a new empty directory; committed examples are references, not substitutes for generation.

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| Run `adaq-component new model m8-forecast-model`. | An empty Model project with a generated UUID and Model SDK feature is created. | Command output and generated `Cargo.toml`/`manifest.json`; do not regenerate or shorten the ID. |
| Run `adaq-component new strategy m8-signal-strategy`. | The default Signal-driven Strategy project is created with its own UUID. | Command output and generated `Cargo.toml`/`manifest.json`. |
| Run `adaq-component new strategy m8-hybrid-strategy`. | A second default Strategy project is created with a distinct UUID. | Command output and generated `Cargo.toml`/`manifest.json`. |
| Run `adaq-component new strategy m8-composed-strategy --template composed`. | A Composed Strategy project is created under `kind: strategy`. | Command output and generated `Cargo.toml`/`manifest.json`. |
| Replace `m8-forecast-model/src/lib.rs` with the identity-preserving implementation from [`model-close-score`](../examples/components/model-close-score/src/lib.rs), using `let normalized = (row.values.first().copied().unwrap_or_default() / 100.0).sin();` and `values: vec![normalized / 100.0, (normalized + 1.0) / 2.0, (normalized + 1.0) / 2.0]`. | The batch Model preserves Instrument ID, Prediction Time, and row order while emitting three varying finite deterministic values. | Complete source and compiler error. |
| Edit only `m8-forecast-model/manifest.json`: preserve generated identity fields; declare `expected-return` as Expected Value/native `future-close-return`, `up-probability` as Probability/probability `future-close-up`, and `return-score` as Score/percentile `future-close-return`, all horizon 1; add Artifact provenance windows `trainingWindow`, `fittingWindow`, and `normalizationWindow` as `0..0`. | The Manifest has one market `close` Slot, Single-Instrument scope, three valid unique outputs, embedded Artifact evidence, and `warmupBars: 0`. | Complete Manifest and schema error; compare with [`model-close-score/manifest.json`](../examples/components/model-close-score/manifest.json). |
| Inspect `m8-signal-strategy` without editing it. | Its sole Forecast Signal Slot makes the authoritative graph Signal-driven. | Source/Manifest and the unexpected field. |
| Edit only `m8-hybrid-strategy` to add a market `close` Slot, bind both indexes, and emit `1` only when probability is at least `0.5` and close is positive. | Its Signal plus Market Slots make the authoritative graph Hybrid without an Architecture field. | Source/Manifest and compiler error. |
| Inspect `m8-composed-strategy` without editing it. | Its generated Market-only Slots make the authoritative graph Composed. | Source/Manifest and the unexpected field. |

For each project, perform the following two rows separately, substituting its exact directory and package name.

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| From one generated project directory run `adaq-component build`. | Tests, Wasm build, conformance, and the self-contained package in `dist/` succeed. | Project name, full output, package path/size, and Manifest. |
| Run `adaq-component verify dist/<project-name>-0.1.0.adaq` for that same project. | Package integrity and contract verification succeed; record its SHA-256. | Project name, full output, package path, and SHA-256. |
| In **Components**, select **Import Component Package** and choose exactly one verified package. | That package is imported with readable exact identity and compatibility evidence. | Selected file, visible error, expandable details, and package hash. |
| Select the newly imported Component once. | Its IDs, versions, hashes, Model contract/Artifact or Strategy Slots, and derived Architecture are readable and copyable. | Screenshot, exact IDs/hashes, and failed detail. |

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
| From `examples/external-models/kronos`, run `python3.12 -m venv .venv` (Windows: `py -3.12 -m venv .venv`). | An isolated supported Python environment is created. | Python/platform version, command, and complete error. |
| Activate that environment, then run `python -m pip install -r requirements.txt`. | The exact Adapter dependencies install. | `python -m pip freeze`, command, and complete error. |
| Run `python -m unittest test_adapter.py`. | Two deterministic fixture tests pass without downloads or a GPU; the golden `.adaq-signals` transformation remains byte-stable. | Python/platform versions, complete traceback, and fixture SHA-256. |
| From `src-tauri`, run `cargo test kronos_fixture_reaches_import_evaluation_and_dataset_first_backtest --lib`. | The committed deterministic fixture is imported, evaluated as Unknown, bound to a compatible Strategy, Backtested Dataset-first, and retained in the Run Dataset Lock. | Complete output, failing stage/test, fixture archive SHA-256, and backtrace if available. |
| Run `hf download NeoQuasar/Kronos-small --revision 901c26c1332695a2a8f243eb2f37243a37bea320 --local-dir artifacts/Kronos-small`. | The exact inference Model Artifact is downloaded. | Command, revision/URL, HTTP error, size, and file listing. |
| Run `hf download NeoQuasar/Kronos-Tokenizer-base --revision 0e0117387f39004a9016484a186a908917e22426 --local-dir artifacts/Kronos-Tokenizer-base`. | The exact Tokenizer Artifact is downloaded and remains distinct from the inference model. | Command, revision/URL, HTTP error, size, and file listing. |
| Inspect the licences and run the platform-specific SHA-256 command from section 1 on the model weights, Tokenizer weights, and pinned preprocessing source. | Licence/source/revision and exact Artifact hashes are recorded without credentials. | Paths, licence text/location, and hash-command output. |
| Run the single Adapter command under **Forecast configuration and deterministic Seed** in [External Kronos Adapter](../examples/external-models/kronos/README.md), substituting the section-3 database/User/Snapshot values and retaining `--seed 7` plus one explicit `--device`. | A Snapshot-aligned `kronos-small.adaq-signals` is produced. When hardware/network prevents this optional real-weight operation, retain the guide's complete unavailable evidence and do not claim it ran. | Exact command, runtime/config, Seed, device, peak memory, elapsed time, full traceback, and no credentials/private data. |
| In **Models → Signal Datasets**, select **Import .adaq-signals** and choose only the produced archive. | Import validates and atomically publishes the external Dataset as **Externally Generated**. | Archive SHA-256/size, exact typed error, and Dataset list before/after. |
| Select the newly imported external Dataset once and expand **Provenance**. | Producer Segment, Artifact/weight/Tokenizer/Adapter/preprocessing hashes, unknown training evidence, Snapshot alignment, and availability policy are inspectable. | Dataset ID, reviewed Manifest, failed field, and screenshot. |
| Select **Export .adaq-signals** once and choose a new path. | The authoritative external evidence exports without changing Dataset identity or overwriting a file. | Dataset ID, export path, archive hash, and exact error. |

<!-- m8-acceptance:evaluation -->
## 5. Expected Value, Probability, and Score evaluation

Repeat the following operation for `expected-return`, `up-probability`, and `return-score` from the native Dataset. Use the Dataset coverage bounds and stability window `20`.

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| In **Models → Evaluation Reports**, select the Dataset and Signal, enter its full start/end milliseconds and `20`, then select **Create Report**. | Immutable Reports show common coverage/missingness/distribution/stability plus MAE/RMSE/bias/correlation for Expected Value, Brier/Log Loss/ROC AUC/calibration for Probability, and time-series Pearson IC/Spearman IC/window IC/ICIR/quantiles for Score. | Dataset/Signal/horizon/window, typed error, unavailable rows, and Report ID if created. |
| On every rendered metric, focus its adjacent information control with Tab, open it with keyboard, close it, then open it by click and pointer hover. | Meaning, formula, interpretation direction, range, caveat, undefined state, and reference link are available without color or hover alone and never turn prediction quality into Strategy profitability. | Metric label, interaction mode, screenshot, focus state, and accessibility-tree text. |
| Expand each Report's **Evidence** and **Provenance**, then select **Export JSON** and **Export Markdown** to new file names. | Producer-level evidence and unavailable results remain visible; exact Dataset/Snapshot/Segments/Artifacts/contracts/hashes/trust/versions are preserved; existing files are not overwritten. | Report ID, Evidence State, export name, visible/technical error, and exported file if safe. |
| Copy the Model project once, preserving `componentId`, change version to `0.1.1`, and change all three provenance windows to `0..9999999999999`. | The new project represents the same Component's next version with deliberately overlapping evidence. | Both Manifests and the exact changed fields. |
| Run `adaq-component build` in the `0.1.1` project. | The overlapping-evidence package builds and passes conformance. | Full output and Manifest. |
| Run `adaq-component verify dist/m8-forecast-model-0.1.1.adaq`. | The package verifies and has a new archive hash. | Full output, path, and archive hash. |
| Import only `m8-forecast-model-0.1.1.adaq` in **Components**. | Version `0.1.1` appears under the retained Component identity. | Selected file, visible/technical error, and package hash. |
| In **Models → Create Dataset**, create one Dataset from version `0.1.1` and the same Snapshot. | A distinct immutable Dataset completes with the overlapping provenance. | Attempt/Dataset IDs, status, diagnostics, and hashes. |
| Create one Report from the `0.1.1` Dataset. | Its Evidence State is **Overlapping**, while original native Reports remain **Out-of-sample**. | Dataset/Report IDs, Segment windows, states, warning, and provenance JSON. |
| Create one Report from the imported Kronos Dataset. | Its incomplete upstream windows produce **Unknown** and never upgrade trust. | Dataset/Report ID, state, warning, and provenance JSON. |

<!-- m8-acceptance:backtests -->
## 6. Signal-driven, Hybrid, and Composed Backtests

Run the next rows once for each imported Strategy, always using the exact section-3 Snapshot and a subset window inside it.

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| In **Backtest → Strategy**, choose exactly one of the three Strategies. | The displayed Architecture is Signal-driven, Hybrid, or Composed from its authoritative slots. | Strategy/package hash, expected/actual Architecture, and screenshot. |
| For a Signal-driven/Hybrid Strategy, bind `forecast-probability` only to the native `up-probability` Dataset Signal; for Composed, leave the generated Market Slots unchanged. | Only semantically compatible Dataset evidence can be selected and no Model is invoked. | Slot, candidate list, Dataset/Signal contract, and gate message. |
| In **Execution**, set allocation `10000`, Seed `48`, the default Spot Execution Profile, and one valid Dataset subset window. | The intended exact execution configuration is visible and editable. | Entered values, Snapshot/Dataset binding, and validation message. |
| Select **Validate inputs** once. | Preflight succeeds or returns one exact typed gate before execution. | Complete typed error and selected identities. |
| Inspect **Authoritative inputs** once. | Exact Feature Plan inputs, Package, Snapshot, Dataset Signal, Producer, schema/Catalog/engine identities, and window are frozen for review. | Copied preflight JSON and missing/wrong field. |
| Select **Run Backtest** once. | One deterministic immutable Run completes; `availableAt` is enforced, fills cannot precede the next Bar, and unavailable aligned values pause as MissingInput. | Run ID if created, status, typed error, and pauses. |
| Open **Overview** once and operate each adjacent metric information control one at a time. | Results and accessible metric explanations render without changing authoritative values. | Run ID, metric, interaction mode, screenshot, and error. |
| Open **Decisions** once. | Target Decisions, Signal evidence, and Run Pauses are inspectable. | Run ID, failed row, and technical details. |
| Open **Execution** once. | Orders, fills, fees, and next-Bar timing are inspectable. | Run ID, failed row, and technical details. |
| Open **Provenance** once. | Feature Plan, Architecture, Component/Dataset Locks, Evidence State, Producer provenance, engine identities, Seed, and run window are copyable. | Run ID, failed field, screenshot, and technical details. |
| Select **Use as new configuration** once. | The historical Run remains immutable and a new editable configuration is populated. | Original Run ID, copied values, and unexpected mutation/error. |
| In **Validation**, select exactly one completed Run. | Its original Snapshot and Signal evidence populate the Protocol form. | Run ID, Snapshot ID, and mismatch message. |
| Select **Freeze Validation Protocol** once. | A new immutable Protocol is created without changing the source Run. | Run/Protocol IDs and technical error. |

<!-- m8-acceptance:negative-paths -->
## 7. Required negative paths

Perform these on disposable package/archive copies or new Attempts; never edit finalized evidence.

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| In a disposable Model Manifest set `horizonBars` to `0`, then run `adaq-component build`. | The invalid horizon fails before import with stable typed evidence. | Mutated Manifest, complete output, and whether `dist/` changed. |
| Restore the horizon, set Probability to `future-close-return`, then run `adaq-component build`. | The invalid Kind/Target combination fails before import. | Mutated Manifest and complete typed error. |
| Add one unexpected ZIP entry to a copy of a valid `.adaq-signals`. | A malformed disposable archive exists without editing finalized evidence. | Source/copy paths, ZIP listing, size, and SHA-256. |
| Import only that malformed archive. | It is rejected atomically and no Dataset appears. | Exact error and Dataset list before/after. |
| Alter `signals.parquet` inside a second disposable archive and import it. | The hash mismatch is rejected atomically. | Archive SHA-256/size, changed Parquet hash, exact error, and Dataset list state. |
| In Backtest select a Snapshot/Instrument/interval different from the Dataset and inspect the compatible Signal candidates. | Snapshot mismatch prevents binding; no approximate join, resampling, forward-fill, or mixed Snapshot is offered. | Snapshot/Dataset identities, candidate list, and exact gate. |
| Try to bind an Expected Value Signal to the Probability slot. | The incompatible Strategy binding is absent or rejected before execution. | Slot/Signal contracts, candidate list, and exact error. |
| Start one new native Dataset Attempt and select **Cancel** while it is Running. | The Cancelled Attempt retains configuration, progress, and diagnostics and publishes no partial Dataset. | Attempt ID, terminal state, diagnostics, and Dataset list before/after. |
| Select **Retry** on that Cancelled Attempt. | Retry creates a new Attempt identity. | Old/new Attempt IDs, status, and technical error. |
| Run one disposable Model that returns a non-finite output. | A Failed Attempt retains bounded diagnostics and publishes no partial Dataset. | Attempt ID, diagnostic, Dataset list, and exact technical error. |
| Run a compatible Strategy across one inspected Warmup/MissingInput row or external Signal whose `availableAt` is after decision time. | The Run records `Run Pause::MissingInput`; it never substitutes zero, flat exposure, a shifted row, or future evidence. | Row identity/status/availableAt, decision time, Run ID, pause evidence, and Dataset lock. |
| From `src-tauri`, run `cargo test datasets_lock_their_component_artifacts --lib`. | The focused check proves referenced Dataset/Artifact deletion is rejected. | Complete test output, referencing IDs, and backtrace if available. |
| In **Settings → Data & Storage**, open the relevant reset confirmation and then select **Cancel**. | Copy names deleted/preserved scopes and requires explicit confirmation while preserving account/login and device preferences. | Reset message, summary counts, and technical details; do not perform a destructive reset for acceptance. |

<!-- m8-acceptance:regressions -->
## 8. Desktop regression review

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| Resize the app content area to 1024 px wide. | The narrow acceptance viewport is active without forcing page zoom. | OS/display scale, measured width, and screenshot. |
| Visit **Dashboard** once using keyboard navigation. | Content and actions remain visible; focus and non-color meaning are preserved. | Screenshot, focused element, and accessibility text. |
| Visit one market-data view once using keyboard navigation. | Chart/list/status content remains usable and focused controls are visible. | Route, screenshot, focused element, and technical error. |
| Visit **Components** once using keyboard navigation. | Import, list/detail, pagination, status, and errors remain usable at 1024 px. | Screenshot, focused element, and accessibility text. |
| Visit each **Models** top-level tab separately using keyboard navigation. | Create Dataset, Signal Datasets, and Evaluation Reports remain usable without clipped actions. | Tab, screenshot, focused element, and accessibility text. |
| Visit **Backtest** once using keyboard navigation. | Stages, forms, results tabs, status, and errors remain usable at 1024 px. | Stage/tab, screenshot, focused element, and technical error. |
| Visit **Validation** once using keyboard navigation. | Protocol/Report forms, tabs, exports, status, and errors remain usable at 1024 px. | Tab, screenshot, focused element, and technical error. |
| Visit each **Settings** section separately using keyboard navigation. | All controls, confirmations, status, and data summaries remain usable at 1024 px. | Section, screenshot, focused element, and accessibility text. |
| Use titlebar Back/Forward across Models tabs and Backtest/Validation, then return after another page visit. | Route history and tab restoration return to the expected business page/tab without showing initialization. | Exact navigation sequence, expected/actual route and tab, screenshot, and console/technical error. |
| Sign out once. | The authenticated research shell closes without exposing prior User data. | Redacted User ID, route, screenshot, and technical error. |
| Sign in once as a different test User. | A fresh authenticated shell appears for the second User. | Redacted User ID, route, and visible/technical error. |
| Revisit each scoped surface—Components, Models, Backtest, Validation, Settings summary, and market-data views—one at a time. | Components, Attempts, Datasets, Reports, Runs, and Snapshot access do not leak across Users. | Surface, both redacted User IDs, leaked/missing record ID, screenshot, and technical details. |
| Review these exact English/Chinese pairs: [`README`](../README.md)/[`README.zh-CN`](../README.zh-CN.md), [SDK](../src-tauri/crates/adaq-component-sdk/README.md)/[SDK zh-CN](../src-tauri/crates/adaq-component-sdk/README.zh-CN.md), [Component](components/developing-components.md)/[Component zh-CN](components/developing-components.zh-CN.md), [archive/Manifest](reference/component-manifest.md)/[archive/Manifest zh-CN](reference/component-manifest.zh-CN.md), [Metric](reference/research-metrics.md)/[Metric zh-CN](reference/research-metrics.zh-CN.md), [external model](../examples/external-models/kronos/README.md)/[external model zh-CN](../examples/external-models/kronos/README.zh-CN.md), and these two manual guides. | Delivered M8 scope is semantically equivalent and none claims training, embedded Qlib/Python, Cross-sectional inference, live trading, Portfolio Optimization, OMS/EMS, Marketplace publishing, or future profitability. | Exact file/link, quoted conflicting text, and expected scope statement. |

<!-- m8-acceptance:automated-gates -->
## 9. Automated release gates and CI

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| From `src-tauri` run `cargo test --workspace`. | All Rust unit, integration, and doc tests pass. | Revision, complete unfiltered failure, test name, and backtrace if available. |
| From `src-tauri` run `cargo check --workspace`. | All Rust workspace type checks pass. | Revision and complete output. |
| From `src-tauri` run `cargo fmt --all --check`. | All Rust files satisfy rustfmt. | Revision and complete diff. |
| From the repository root run `pnpm exec jest --watchman=false --runInBand`. | All Jest suites pass. | Revision, suite/test, and complete error. |
| Run `pnpm run build`. | Strict TypeScript and production Vite build pass. | Revision and complete error. |
| Run `pnpm run lint`. | The lint command succeeds; record any explicitly pre-existing warnings. | Revision, file/rule, complete output, and warning delta. |
| Run `git diff --check`. | No whitespace errors exist. | Revision and complete output. |
| From `examples/external-models/kronos` run `python -m unittest test_adapter.py`. | Both Adapter fixture tests pass in the pinned environment. | Environment versions and complete output. |
| Re-run `adaq-component build` once in each generated project after removing only its `dist/` directory. | All four generated-project regressions pass. | Project name, complete output, and artifact listing. |
| Re-run `adaq-component verify dist/<project-name>-0.1.0.adaq` once for each rebuilt project. | All four exact packages verify; hashes match the acceptance record. | Project name, full output, path, and hash. |
| After pushing the acceptance commit, record every applicable GitHub Actions run URL, commit SHA, platform/job, and conclusion. | Required multi-platform checks for the reviewed revision complete successfully; a local pass does not replace CI. | Run URL/SHA, failed job/platform, and relevant unredacted log excerpt. |

<!-- m8-acceptance:acceptance-record -->
## 10. Acceptance record

Record: macOS version/architecture and display scale; AdaQ revision; Rust/CLI/Node/pnpm/Python versions; four package hashes; User ID only in the private record; native/external Dataset and Parquet hashes; Snapshot, Attempt, Artifact, Producer Segment, Feature Plan, Report, Run, Protocol, and Validation Report IDs; three evaluation states; JSON/Markdown and `.adaq-signals` export names/hashes; negative-path evidence; accessibility/1024px review; and CI URLs/conclusions. Redact credentials, OTPs, tokens, Supabase values, private paths, and private market data.

The maintainer and agent review this record one operation at a time. M8 is accepted only after every row above has passed or an explicitly optional real-Kronos run has complete unavailable evidence, all automated gates and applicable CI are green, and the reviewed record contains no unresolved failure.

## Delivered scope boundary

M8 delivers offline Single-Instrument inference, immutable native/external Forecast Signal evidence, Forecast Evaluation, Dataset-first Signal-driven/Hybrid Backtests, and the existing Composed path. It does not deliver training/fitting/tuning, embedded Qlib/Python, Cross-sectional inference, generated future paths as realized data, live trading, Portfolio Optimization, OMS/EMS, a controlled GPU/ONNX Runner, Marketplace publishing, or future-profitability claims.
