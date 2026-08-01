# ADAQ Component Manifest

Generated from `component-manifest.contract.json`; do not edit.

| Field | Contract |
| --- | --- |
| `manifestSchemaVersion` | Manifest contract version. Exact value: `1.0.0`. |
| `componentId` | Immutable Component identity. |
| `version` | Component SemVer version. |
| `name` | Human-readable Component name. |
| `kind` | Component ABI kind. Values: `factor`, `strategy`, `model`. |
| `sdkVersion` | Exact SDK version. Exact value: `0.1.0`. |
| `abiVersion` | Exact Component ABI version. Exact value: `1.0.0`. |
| `wasmSha256` | SHA-256 of component.wasm; set during packaging. |
| `parameters` | Declared Component parameters with typed defaults. |
| `featureSlots` | Ordered Strategy Feature Slots. Strategy requires one or more; Factor forbids them. |
| `outputNames` | Ordered Factor output identifiers. Factor permits at most 64. |
| `dependencies` | External Factor dependencies with unique aliases. |
| `warmupBars` | Factor output availability Warmup; it is not convergence. |
| `modelScope` |  Values: `single-instrument`. |
| `modelArtifact` |  |
| `modelOutputs` |  |
