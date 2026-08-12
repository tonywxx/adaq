# M7 Manual Acceptance (macOS ARM64)

This is the canonical human acceptance path for the local-first research workspace. Perform the steps in order and retain the indicated evidence when a step fails. A completed Report is historical evidence, not a best Strategy, a profitability claim, Paper Trading, or Live Trading.

## 1. Prerequisites and sign-in

| Action | Expected result | On failure, capture |
| --- | --- | --- |
| On macOS ARM64, install the stable Rust toolchain, its `wasm32-unknown-unknown` target, `cargo-component`, and this repository's CLI: `rustup toolchain install stable`; `rustup target add --toolchain stable wasm32-unknown-unknown`; `cargo install cargo-component --locked`; `cargo install --force --path src-tauri/crates/adaq-component-tooling`. | `adaq-component` is on `PATH`. | Command, complete terminal output, and `rustc --version`. |
| Configure the desktop build environment with the names `VITE_SUPABASE_URL` and `VITE_SUPABASE_PUBLISHABLE_KEY`; do not put their values in this guide or commit them. Start the app with `pnpm tauri dev`. | The sign-in screen is shown, not the missing-configuration message. | The exact message and the variable *names* only. |
| Enter an existing account's email, choose the password path, enter its password, and select **Sign in**. | The Dashboard and sidebar appear. | The visible error and its expandable technical details, with no password or token. |

First-time setup is supplementary: enter a new email, use the emailed OTP, then create and confirm a strong password (eight or more characters, lower-case, upper-case, digit, and symbol). Expected result: the Dashboard appears and a later sign-in uses that password. On failure, capture the visible error and expandable technical details, but never the OTP, password, token, or Supabase value.

Windows uses the same steps, but run the commands in PowerShell and use `cargo install --force --path .\src-tauri\crates\adaq-component-tooling` from the repository root. Use `.\` paths in commands and select the package in the native Windows file picker.

## 2. Author and verify empty Components

Run these commands in an empty working directory; committed examples are only references or recovery aids.

```sh
adaq-component new factor m7-close-change
adaq-component new strategy m7-close-change-strategy
```

The SDK is not published separately yet. In each generated `Cargo.toml`, replace the `adaq-component-sdk` dependency with a local path to this checkout, keeping the generated feature: `adaq-component-sdk = { path = "<absolute-path-to-adaq>/src-tauri/crates/adaq-component-sdk", features = ["factor"] }` for the Factor and `features = ["strategy"]` for the Strategy. Expected result: Cargo resolves the SDK from this checkout. On failure, capture both `Cargo.toml` files and the complete Cargo error.

### Factor

In `m7-close-change/src/lib.rs`, replace the generated source with:

```rust
use core::cell::Cell;
use adaq_component_sdk::factor::time_series::{
    FactorResult, FactorSchema, FactorScope, FeatureSlot, Guest, GuestInstance,
    Instance as FactorInstance, NamedScalar, ParameterValue, TimeSeriesRow,
};

struct Component;
struct Instance { previous_close: Cell<Option<f64>> }

impl Guest for Component {
    type Instance = Instance;
    fn describe() -> Result<FactorSchema, String> {
        Ok(FactorSchema {
            scope: FactorScope::TimeSeries,
            schema_version: adaq_component_sdk::FACTOR_SCHEMA_VERSION.into(),
            feature_slots: vec![FeatureSlot { name: "close".into() }],
            parameters: vec![],
            output_names: vec!["close-change".into()],
            warmup_bars: 1,
        })
    }
    fn create(_feature_slots: Vec<FeatureSlot>, _parameters: Vec<ParameterValue>) -> Result<FactorInstance, String> {
        Ok(FactorInstance::new(Instance { previous_close: Cell::new(None) }))
    }
}
impl GuestInstance for Instance {
    fn process(&self, rows: Vec<TimeSeriesRow>) -> Result<Vec<FactorResult>, String> {
        rows.into_iter().map(|row| {
            let close = row.slots.first().ok_or("missing close Feature Slot")?.value;
            let output = self.previous_close.get().map(|previous| vec![NamedScalar {
                name: "close-change".into(), value: (close - previous) / previous,
            }]);
            self.previous_close.set(Some(close));
            Ok(FactorResult {
                instrument_id: row.instrument_id,
                observation_time_ms: row.observation_time_ms,
                values: output,
            })
        }).collect()
    }
}
adaq_component_sdk::factor::time_series::bindings::export_factor!(
    Component with_types_in adaq_component_sdk::factor::time_series::bindings
);
```

In `m7-close-change/manifest.json`, keep the generated `componentId`, `sdkVersion`, and `name`; make the complete Factor contract:

```json
{
  "manifestSchemaVersion": "1.0.0",
  "componentId": "<generated factor componentId>",
  "version": "0.1.0",
  "name": "M7 Close Change",
  "kind": "factor",
  "sdkVersion": "<generated sdkVersion>",
  "abiVersion": "2.0.0",
  "factorScope": "time-series",
  "featureSlots": [{"name": "close", "source": {"kind": "market", "field": "close"}}],
  "outputNames": ["close-change"],
  "warmupBars": 1
}
```

| Action | Expected result | On failure, capture |
| --- | --- | --- |
| `cd m7-close-change && adaq-component build` | Tests, Wasm build, conformance, and `dist/m7-close-change-0.1.0.adaq` succeed. | Full output and `manifest.json`; do not change the generated ID. |
| `adaq-component verify dist/m7-close-change-0.1.0.adaq` | Package verification succeeds. Record its archive hash. | Full verifier output and package path. |

### Strategy

Copy the generated Factor `componentId` into the Strategy Manifest. In `m7-close-change-strategy/src/lib.rs`, replace the source with:

```rust
use adaq_component_sdk::strategy::{FeatureFrame, FeatureSlot, Guest, GuestInstance, Instance as StrategyInstance, ParameterValue, SlotIndexes};
struct Component;
struct Instance { change: usize }
impl Guest for Component {
    type Instance = Instance;
    fn create(feature_slots: Vec<FeatureSlot>, _parameters: Vec<ParameterValue>) -> Result<StrategyInstance, String> {
        Ok(StrategyInstance::new(Instance { change: SlotIndexes::bind(&feature_slots)?.index("close-change")? }))
    }
}
impl GuestInstance for Instance {
    fn process(&self, frames: Vec<FeatureFrame>) -> Result<Vec<String>, String> {
        frames.into_iter().map(|frame| {
            let value = *frame.values.get(self.change).ok_or("feature slot count mismatch")?;
            Ok(if value > 0.0 { "1" } else { "0" }.to_owned())
        }).collect()
    }
}
adaq_component_sdk::strategy::bindings::export_strategy!(Component with_types_in adaq_component_sdk::strategy::bindings);
```

In `m7-close-change-strategy/manifest.json`, retain its generated `componentId` and `sdkVersion`, and replace `<factor-component-id>` exactly:

```json
{
  "manifestSchemaVersion": "1.0.0",
  "componentId": "<generated strategy componentId>",
  "version": "0.1.0",
  "name": "M7 Close Change Strategy",
  "kind": "strategy",
  "sdkVersion": "<generated sdkVersion>",
  "abiVersion": "1.0.0",
  "featureSlots": [{"name":"close-change","source":{"kind":"external","dependencyAlias":"change","output":"close-change"}}],
  "dependencies": [{"componentId":"<factor-component-id>","version":"^0.1.0","alias":"change"}]
}
```

| Action | Expected result | On failure, capture |
| --- | --- | --- |
| `cd ../m7-close-change-strategy && adaq-component build` | The Strategy package is created in `dist/`. | Full output and both Manifests. |
| `adaq-component verify dist/m7-close-change-strategy-0.1.0.adaq` | Verification succeeds. Record its archive hash. | Full verifier output and package path. |

## 3. Import and audit Components

| Action | Expected result | On failure, capture |
| --- | --- | --- |
| In the sidebar select **Components**, choose each verified `.adaq` through **Import Component Package**, starting with the Factor. | Both Packages appear and the import feedback identifies the imported component. | The visible error, expandable technical details, and selected file name. |
| Select each package. Review name, kind, version, compatibility, parameters, Factor outputs or Strategy Feature Slots/dependencies, Warmup, ABI/SDK/Manifest versions, and archive/Wasm hashes. | Exact identities are readable and copyable; the Strategy dependency is compatible with the imported Factor. | Screenshot plus copied IDs/hashes and technical details. |

## 4. Freeze data and execute a Backtest

| Action | Expected result | On failure, capture |
| --- | --- | --- |
| Select **Backtest**. In **Data and Strategy configuration**, choose an Instrument, Bar Interval, start, and end. Reuse a matching listed Market Data Snapshot when suitable; otherwise select **Download and freeze Snapshot** and wait for completion. | A selected Snapshot displays its range, Bar count, source, and exact ID. | Stage error, selected values, and Snapshot ID if created. |
| Select **M7 Close Change Strategy** and bind `change` to **M7 Close Change**. Review any Manifest parameters. | The compatible Factor is selected by readable identity. | The message and component hashes. |
| In **Execution and pre-Run review**, set initial quote allocation and inspect every Execution Profile field plus Snapshot, packages, parameters, Feature Slots, and Indicator Plan inputs. Select **Run Backtest**. | One immutable Run completes; duplicate clicks are disabled while it runs. | Review screen, typed error details, and any Run ID. |
| Inspect all result tabs: **Overview** (metrics/equity/benchmark/drawdown), **Decisions** (Target Decisions and Run Pauses), **Execution** (orders, fills, fees), and **Provenance** (Snapshot, packages, parameters, plan, profile, engine identities, versions, seed). | Each tab renders its evidence and copyable identities. | Screenshot of each failed tab and Run ID. |
| From **Provenance**, select **Use as new configuration**. | The immutable Run stays unchanged and its normalized settings populate the current form; changing and executing creates a distinct Run. | Both Run IDs and the unexpected mutation/error. |

## 5. Validate and export reports

For each method below, select **Validation**, choose the completed Backtest Run, configure the stated evidence, select **Freeze Validation Protocol**, expand **Review immutable Protocol**, then select **Run / resume**. Record every Protocol and Report ID.

| Method | Exact action | Expected result / failure evidence |
| --- | --- | --- |
| Chronological holdout | Choose **Chronological holdout** and enter a valid **Sample-out starts** boundary inside the frozen Snapshot. | A frozen `chronological-holdout@1` Protocol and Report; capture boundary and typed error if invalid. |
| Walk-forward | Choose **Walk-forward**, enter valid window size, step size, and minimum history; inspect the preview. | A frozen `walk-forward@1` Protocol and Report; capture the values and gate message if unavailable. |
| Cross-market | Choose **Cross-market**, add ordered frozen Snapshot contexts; use an override only when its Run uses that exact Snapshot. | A frozen `cross-market@1` Protocol and Report; capture ordered Snapshot IDs and mismatch error if any. |

For every completed Report, inspect all three tabs: **Summary** (aggregate returns, fees, trades, consistency/dispersion), **Evidence** (windows or markets, failures, Run Pauses, linked Runs, and Recommended Contexts as historical evidence), and **Provenance** (Protocol, Runs, packages, plans, snapshots, configurations, aggregation rules, versions). Select **Export JSON** and **Export Markdown** and retain the generated file names. A failed or interrupted Protocol remains available for **Run / resume**; capture its Protocol ID and technical error before retrying.

## 6. Automated verification and CI

| Action | Expected result | On failure, capture |
| --- | --- | --- |
| From `src-tauri`, run `cargo test --workspace` and `cargo check --workspace`; from the repository root, run `pnpm test` and `pnpm run build`. | All commands exit successfully. | Complete failing command output and revision. |
| After pushing the acceptance commit, record the URL, commit SHA, and conclusion of the applicable GitHub Actions run. | The applicable workflow succeeds for the reviewed revision. | Run URL, failed job name, and unredacted log excerpt. |

## 7. Acceptance record

Record macOS version/architecture, ADAQ revision, CLI/Rust versions, both package hashes, Snapshot IDs, Run IDs, Protocol IDs, Report IDs, and JSON/Markdown export file names. Redact credentials, OTPs, tokens, and Supabase values. Review this record with the maintainer before declaring M7 accepted.
