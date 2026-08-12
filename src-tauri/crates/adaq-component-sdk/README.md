# ADAQ Component SDK

[English](README.md) | [简体中文](README.zh-CN.md)

`adaq-component-sdk` is the Tauri-independent Rust SDK for implementing ADAQ Factor and Strategy Components. It owns the versioned WIT contracts, generated bindings, exact-decimal helpers, and export macros.

Start from an `adaq-component new` template instead of wiring the SDK manually. A generated Factor enables the `factor` feature; a generated Strategy enables `strategy`.

## Component contract

- A Factor ABI v2 Component declares one scope and ordered Feature Slots. Time-series Factors receive host-resolved rows for one instrument; Cross-sectional Factors receive ordered Point-in-Time Universe rows with typed unavailable cells. Prices, quantities, and volumes remain decimal in the host contract; convert to `f64` only for analytical output. Factor results preserve identity/order and return 1–64 declared finite outputs or typed absence.
- A Strategy receives pre-bound numeric Feature Slots and returns one complete Target Exposure decimal string per frame.
- Do not read files, the network, environment variables, clocks, or randomness. The host rejects ambient WASI imports and verifies deterministic replay and chunk independence.
- Keep `sdkVersion` and `abiVersion` in `manifest.json` unchanged unless the matching SDK and host contract are intentionally upgraded.

The SDK is a library, not a command. Build and package projects with the separate [`adaq-component` CLI](../adaq-component-tooling/README.md).

## Repository checks

```sh
cd src-tauri
cargo test -p adaq-component-sdk --all-features
```
