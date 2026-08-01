# Developing ADAQ Components

ADAQ Components are local, deterministic WebAssembly Components. Use the `adaq-component` CLI; the desktop application imports verified `.adaq` archives but is not a code editor.

## Start, build, verify, package

```sh
adaq-component new factor close-change
adaq-component new strategy trend
cd close-change
adaq-component build
adaq-component verify dist/close-change-0.1.0.adaq
adaq-component verify dist/close-change-0.1.0.adaq --previous ../close-change-0.1.0/manifest.json
```

`build` runs the Component tests, builds `wasm32-unknown-unknown`, runs host conformance, and creates `dist/*.adaq`. Packages contain only `manifest.json` and `component.wasm`. `verify` validates an existing package without modifying it; `--previous` also checks the documented SemVer contract.

## Strategy Feature Slots

`manifestSchemaVersion` is exactly `1.0.0`. A Strategy's non-empty `featureSlots` array is its complete, ordered ABI contract: names are unique lower-kebab-case ASCII identifiers (at most 64 bytes), and order becomes the dense indexes received by the guest. Never emit `inputNames`.

```json
{
  "featureSlots": [
    {"name":"close","source":{"kind":"market","field":"close"}},
    {"name":"ema","source":{"kind":"builtin","indicator":"ema","output":"value","inputs":{"real-0":"close"},"parameters":{"time-period":20}}},
    {"name":"change","source":{"kind":"external","dependencyAlias":"change-5","output":"close-change"}}
  ],
  "dependencies": [{"componentId":"11111111-1111-4111-8111-111111111111","version":"^1.0.0","alias":"change-5"}]
}
```

- `market` binds one raw OHLCV field and has zero Warmup.
- `builtin` binds a documented Catalog Indicator output. Inputs select allowed OHLCV fields; parameters are typed literals or direct Strategy parameter references. Omitted parameters use Catalog defaults.
- `external` binds an output of the uniquely named Factor alias in `dependencies`. A Factor may be absent for a Bar; ADAQ does not invoke the Strategy until every Slot is present.
- `signal` declares a semantic Forecast Signal requirement: Prediction Kind, Forecast Target, horizon, and Value Scale. The Strategy does not pin a Model or Dataset; Backtest configuration binds an exactly compatible finalized Dataset signal.

All delivered Slot values are finite `f64`. They are analytical values only: do not treat them as authoritative financial values or accept NaN/infinity. Use `SlotIndexes` to bind a name once, then process the ordered frame values.

Warmup means the first Bar with an available output, not numerical convergence. The [Indicator Catalog reference](../reference/indicator-catalog.md) reports the upstream `Unstable Period` flag; it does not add hidden history. A Bar Gap starts a new Continuous Bar Segment and recreates Factors and the Strategy, so Warmup starts again.

## Models, Signals, and external research runtimes

An AdaQ Model Component is a deterministic sandboxed WASM inference package; its training engine is outside the Component ABI. A modular Strategy consumes an already finalized Forecast Signal Dataset and never invokes its producing Model implicitly. The frozen Backtest Feature Plan and Dataset Lock preserve the selected Dataset, signal contract, Producer provenance, and evidence state.

Large Python/PyTorch models use the [External Kronos Adapter boundary](../../examples/external-models/kronos/README.md): inference runs outside AdaQ and only canonical `.adaq-signals` evidence is imported. Microsoft Qlib may later train or prepare Artifacts through the same boundary. M8 does not embed Python or Qlib, provide training or a controlled Python Runner, claim Verified external inference, or publish external Artifacts to a Marketplace.

## Factors

Factors receive exact decimal-string OHLCV Bars and return declared named finite `f64` outputs. Keep financial arithmetic decimal until conversion is necessary for analysis. `outputNames` is ordered, unique lower-kebab-case, and limited to 64; every present result row must supply exactly that declaration and order. `warmupBars` declares initial absence, not convergence. Factors cannot declare Feature Slots.

Factor aliases identify Factor Instances, not Component IDs: the same package may occur more than once under different aliases and parameter bindings. Each alias is independently recreated after a Bar Gap.

## SemVer

For stable (`1.x+`) Components, removing, renaming, reordering, or changing a Feature Slot; changing its source; removing or changing a parameter; changing dependencies, Warmup, Manifest/ABI versions; or removing/renaming/reordering Factor outputs requires a major version. Appending a Factor output or adding a defaulted parameter requires a minor version. Algorithm fixes and documentation-only corrections are patch changes. `0.x` is intentionally development-unstable. Every changed package still needs a new Component version because ADAQ locks exact hashes.

## Troubleshooting

- `forbidden ambient imports`: build for `wasm32-unknown-unknown`; do not use WASI, filesystem, network, environment, clocks, or randomness.
- `Factor runtime schema does not match manifest`: make `describe()` output names and Warmup match `manifest.json` exactly.
- `Indicator Plan validation failed`: check Slot order/names, Catalog Indicator IDs, source inputs, parameter types/ranges, and External aliases/outputs.
- `Target Exposure is invalid`: return one finite decimal target per frame, in the selected Position Mode range.
- `chunk-boundary independent`: preserve Factor/Strategy state correctly across consecutive `process` calls; host verification compares whole and chunked execution.

See [Manifest reference](../reference/component-manifest.md), [JSON Schema](../reference/component-manifest.schema.json), and the executable [fixtures](../../src-tauri/fixtures/README.md).
