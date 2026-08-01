# AdaQ

[English](README.md) | [简体中文](README.zh-CN.md)

[![Release](https://github.com/tonywxx/adaq/actions/workflows/release.yml/badge.svg)](https://github.com/tonywxx/adaq/actions/workflows/release.yml)

> **AdaQ** (Ada Quant) is an AI-powered quantitative trading platform for equities and digital crypto assets.

AdaQ V1 is a local-first research, backtesting, and simulation desktop app. It does not execute real account orders; live trading is a separate future supervised, host-controlled milestone.

## Features

- Reproducible local market-data research and backtesting
- Sandboxed WebAssembly Factor and Strategy Components
- Immutable, verifiable `.adaq` Component Packages
- Host-owned TA-Lib Indicator Catalog and frozen Feature Slots
- Deterministic Spot simulation with immutable Backtest Runs
- Native Model Components plus externally generated Forecast Signal Datasets

## Screenshots

| Dashboard | Backtest |
|:---:|:---:|
| ![Dashboard](screenshots/1-dashboard.png) | ![Backtest](screenshots/2-backtest.png) |
| **Components** | **Validation** |
| ![Components](screenshots/3-components.png) | ![Validation](screenshots/4-validation.png) |

## Implemented Milestones

| Milestone | Delivered capability |
| ----------- | ---------------------- |
| M1 | Fixed WebAssembly Component ABI for `adaq:factor@1.0.0` and `adaq:strategy@1.0.0`. Factor Components transform Closed Bars into named scalar outputs; Strategy Components consume dense Feature Slots and emit complete Target Exposure decisions. |
| M2 | Deterministic in-memory Run Engine. The host validates Closed Bars, enforces sandbox limits, binds ordered Feature Slots, records warmup or missing-input pauses, and fails closed on invalid data or invalid targets. |
| M3 | Reproducible crypto Spot Backtest. A Backtest Run immutably binds a Market Data Snapshot, Component Lock, parameters, Indicator Plan, Execution Profile, engine version, and seed. Results persist locally with Target Decisions, simulated orders, fills, equity, fees, metrics, history, and charts. |
| M4 | Component Developer Kit. The Rust SDK, `adaq-component` CLI, templates, conformance checks, and `.adaq` packaging flow support `new`, `build`, and `verify` for Factor and Strategy Components. |
| M5 | TA-Lib Indicator Engine, Indicator Catalog, and Feature Slots. The host pins official C TA-Lib v0.7.1, exposes `adaq-indicator-catalog@1.0.0` with 160 Indicators and 179 outputs, freezes canonical Indicator Plans with `planHash`, supports Market, Built-in, and External Factor Slot sources, evaluates by Continuous Bar Segment, resets analytical state at Bar Gaps, and enforces typed Plan/Run errors plus fixed resource ceilings. |
| M6 | Executable Components and Research Validation. Bilingual executable Factor and Strategy examples teach the supported SDK and CLI workflow; replay-grade Backtest Run provenance preserves every authoritative input; immutable Validation Protocols and Reports support chronological holdout, walk-forward, and cross-market research with traceable evidence and JSON/Markdown exports. |
| M7 | Research Workspace Productization. Components, Backtest, and Validation provide guided, auditable desktop workflows over immutable local evidence; the [bilingual manual acceptance guides](docs/m7-manual-acceptance.md) cover the complete from-empty-project path. |
| M8 | Model research and Dataset-first Backtests. Native Model Components and external `.adaq-signals` evidence produce immutable Forecast Signal Datasets, Forecast Evaluation Reports, and compatible Signal-driven or Hybrid Strategy Runs. The [external Kronos Adapter guide](examples/external-models/kronos/README.md) documents the complete `Kronos-small` + `Kronos-Tokenizer-base` path. |

Together, M1-M8 provide the current closed loop: develop or import a Component, freeze exact market data and Feature Plans, produce or import immutable Forecast Signal evidence, evaluate predictions, run a Dataset-first sandboxed Strategy Backtest, inspect persisted provenance and results, and produce research-validation evidence.

## Develop a Component

Component source code is written in Rust. The Tauri app imports and runs the finished `.adaq` package; it does not provide a GUI code editor or bundle the `adaq-component` CLI.

From this repository:

```sh
rustup toolchain install stable
rustup target add --toolchain stable wasm32-unknown-unknown
cargo install cargo-component --locked
cargo install --path src-tauri/crates/adaq-component-tooling

adaq-component new factor my-factor
cd my-factor
# Edit src/lib.rs and manifest.json.
adaq-component build
adaq-component verify dist/my-factor-0.1.0.adaq
```

Use `adaq-component new strategy my-strategy` for a Strategy Component. Import the verified file from `dist/` into ADAQ's Component Library.

Start with the [executable Factor and Strategy examples](examples/components/README.md), then use the [SDK guide](src-tauri/crates/adaq-component-sdk/README.md), [CLI guide](src-tauri/crates/adaq-component-tooling/README.md), and [Component architecture](CONTEXT.md) as references. The crates currently install from this repository; after publication, `cargo install adaq-component-tooling --locked` will install the same CLI independently of the desktop app.

## Documentation

| English | 简体中文 | Description |
| --------- | ---------- | ------------- |
| [Component SDK](src-tauri/crates/adaq-component-sdk/README.md) | [Component SDK 中文](src-tauri/crates/adaq-component-sdk/README.zh-CN.md) | Rust SDK for implementing Factor and Strategy Components |
| [CLI Tooling](src-tauri/crates/adaq-component-tooling/README.md) | [CLI 工具中文](src-tauri/crates/adaq-component-tooling/README.zh-CN.md) | Build, verify, and manage `.adaq` packages |
| [Component Template](src-tauri/crates/adaq-component-tooling/templates/README.md) | [组件模板中文](src-tauri/crates/adaq-component-tooling/templates/README.zh-CN.md) | Scaffold README for generated component projects |
| [Executable Examples](examples/components/README.md) | [可执行示例中文](examples/components/README.zh-CN.md) | End-to-end Factor and Strategy SDK/CLI tutorial |
| [Test Fixtures](src-tauri/fixtures/README.md) | [测试固件中文](src-tauri/fixtures/README.zh-CN.md) | WASM component build examples for integration tests |
| [M7 Manual Acceptance](docs/m7-manual-acceptance.md) | [M7 人工验收中文](docs/m7-manual-acceptance.zh-CN.md) | Complete human-reviewed research-workspace acceptance path |
| [External Kronos Adapter](examples/external-models/kronos/README.md) | [外部 Kronos Adapter](examples/external-models/kronos/README.zh-CN.md) | External `Kronos-small` inference, canonical Forecast Signals, evaluation, and Dataset-first Backtest |

Microsoft Qlib integration is future work and will use the same External Model Adapter boundary. M8 does not include training, an embedded or controlled Python Runner, Verified external inference, or Marketplace publishing.
