# ADAQ Component Examples v0.9.x

[English](README.md) | [简体中文](README.zh-CN.md)

These executable examples teach the complete ADAQ Component workflow with the real Rust SDK and `adaq-component` CLI:

- [`factor-close-momentum-5`](factor-close-momentum-5/) computes the percentage change from the close five Closed Bars ago.
- [`strategy-momentum-trend`](strategy-momentum-trend/) combines raw Market close, a Built-in EMA, and the external momentum Factor to emit a Long Only Target Exposure.

They are deliberately small and are not profitability claims.

## 1. Create the projects

The checked-in projects were created with the same commands developers use:

```sh
adaq-component new factor factor-close-momentum-5
adaq-component new strategy strategy-momentum-trend
```

`new` creates `Cargo.toml`, `manifest.json`, `src/lib.rs`, and a project README. In this repository the examples use a relative path to the local SDK; a project created with an installed CLI pins the published SDK version instead.

## 2. Understand the Factor

The Factor receives exact decimal-string OHLCV Closed Bars. It keeps the latest five closes and emits:

```text
close-momentum-5 = (current close - close five Bars ago) / close five Bars ago
```

Its first five outputs are absent, matching `warmupBars: 5`. State continues across consecutive `process` calls, while ADAQ creates a new Factor Instance after a Bar Gap. The implementation converts to finite `f64` only after completing exact decimal arithmetic.

Read [`src/lib.rs`](factor-close-momentum-5/src/lib.rs) together with [`manifest.json`](factor-close-momentum-5/manifest.json). The schema returned by `describe()` must exactly match `outputNames` and `warmupBars` in the Manifest.

## 3. Understand the Strategy

The Strategy declares three ordered Feature Slots:

| Slot | Source | Purpose |
| --- | --- | --- |
| `close` | Market `close` | Demonstrates a raw Market field |
| `ema` | Built-in `ema.value` | Demonstrates the Indicator Catalog and a Strategy Parameter reference |
| `momentum` | External `momentum.close-momentum-5` | Demonstrates a separately packaged Factor dependency |

It has two parameters:

- `ema-period`, default `20`, is bound to the EMA `time-period` while the Indicator Plan is frozen.
- `minimum-momentum`, default `0`, is passed to the Strategy Component.

The Strategy returns Target Exposure `1` only when `close > ema` and `momentum > minimum-momentum`; otherwise it returns `0`. `SlotIndexes` binds names once during `create`, after which each Feature Frame is a dense array in Manifest order.

Read [`src/lib.rs`](strategy-momentum-trend/src/lib.rs) together with [`manifest.json`](strategy-momentum-trend/manifest.json). The dependency alias `momentum` connects the External Slot to the exact Factor Package selected by the host.

## 4. Build and verify

Install the Rust stable toolchain, `wasm32-unknown-unknown`, `cargo-component`, and the `adaq-component` CLI. From the repository root:

```sh
cargo install --path src-tauri/crates/adaq-component-tooling --bin adaq-component

cd examples/components/factor-close-momentum-5
adaq-component build
adaq-component verify dist/factor-close-momentum-5-0.1.0.adaq

cd ../strategy-momentum-trend
adaq-component build
adaq-component verify dist/strategy-momentum-trend-0.1.0.adaq
```

`build` runs the project tests, compiles a release WebAssembly Component for `wasm32-unknown-unknown`, runs host conformance, and packages only `manifest.json` and `component.wasm` into `dist/*.adaq`. `verify` rechecks an existing immutable Package without changing it.

## 5. Import and Backtest

1. Open **Component Library** in ADAQ.
2. Import `factor-close-momentum-5-0.1.0.adaq`, then `strategy-momentum-trend-0.1.0.adaq`.
3. Open **Backtest**, choose an Instrument and prepare a Market Data Snapshot.
4. Select **Strategy Momentum Trend**.
5. For the `momentum` dependency, select **Factor Close Momentum 5**.
6. Keep `ema-period = 20` and `minimum-momentum = 0` for the first Run.
7. Run the Backtest and inspect Warmup Pauses, Target Decisions, simulated Orders, Fills, Equity, fees, and metrics.

The effective Warmup is the maximum required by all Slots. A Bar Gap recreates the analytical instances and starts Warmup again; ADAQ never fills missing inputs with zero or NaN.

## 6. Learn by changing one thing

- Raise `minimum-momentum` and observe that fewer Bars target full exposure.
- Change `ema-period` and observe that the frozen Indicator Plan and Run identity change.
- Rename or reorder a Feature Slot and run `verify` to see why that is a versioned contract change.
- Change the Factor formula, bump its Component version, rebuild, and observe that the Strategy dependency resolves to a new exact Package in the Component Lock.

## Troubleshooting

- **`forbidden ambient imports`**: compile for `wasm32-unknown-unknown`; Components cannot use WASI, filesystem, network, environment, clocks, or randomness.
- **Factor runtime schema does not match Manifest**: keep `describe()`, `outputNames`, and `warmupBars` identical.
- **Indicator Plan validation failed**: check Slot order, EMA input and parameter binding, dependency alias, selected Factor identity, and output name.
- **chunk-boundary independent**: preserve analytical state across consecutive `process` calls; do not reset at a host chunk boundary.
- **Target Exposure is invalid**: return one finite decimal string in `[0,1]` for every delivered Long Only Feature Frame.

For the full contracts, see the [Component development guide](../../docs/components/developing-components.md), [Manifest reference](../../docs/reference/component-manifest.md), and [Indicator Catalog](../../docs/reference/indicator-catalog.md).
