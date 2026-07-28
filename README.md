# ADAQ

[![Release](https://github.com/tonywxx/adaq/actions/workflows/release.yml/badge.svg)](https://github.com/tonywxx/adaq/actions/workflows/release.yml)

> **ADAQ** is a cross-platform Desktop AI Quant Trading Platform.

## Features

- Reproducible local market-data research and backtesting
- Sandboxed WebAssembly Factor and Strategy Components
- Immutable, verifiable `.adaq` Component Packages

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

Use `new strategy my-strategy` for a Strategy Component. Import the verified file from `dist/` into ADAQ's Component Library.

See the [SDK guide](src-tauri/crates/adaq-component-sdk/README.md), [CLI guide](src-tauri/crates/adaq-component-tooling/README.md), and [Component architecture](CONTEXT.md). The crates currently install from this repository; after publication, `cargo install adaq-component-tooling --locked` will install the same CLI independently of the desktop app.
