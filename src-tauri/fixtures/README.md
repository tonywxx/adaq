# Executable M5 component examples

[English](README.md) | [简体中文](README.zh-CN.md)

The fixture crates intentionally build for `wasm32-unknown-unknown`: the
default WASI Preview 1 adapter adds forbidden ambient imports.

The examples cover Market-only (`strategy`), External (`external-strategy`), Portfolio (`portfolio-strategy`), mixed Market/Built-in/External (`mixed-strategy`), time-series and cross-sectional Factor ABI v2 (`multi-output-factor`, `cross-sectional-factor`), and repeated Factor aliases (`repeated-factor-strategy`) Feature Slot Plans.

Build the fixtures before the host integration tests. On machines with both
Homebrew Rust and rustup, put the rustup shims first so cargo-component can see
the installed WASM target:

```sh
cd src-tauri/fixtures/factor
rustup run stable cargo component build --target wasm32-unknown-unknown
cd ../strategy
rustup run stable cargo component build --target wasm32-unknown-unknown
cd ../external-strategy
rustup run stable cargo component build --target wasm32-unknown-unknown
cd ../portfolio-strategy
rustup run stable cargo component build --target wasm32-unknown-unknown
cd ../mixed-strategy
rustup run stable cargo component build --target wasm32-unknown-unknown
cd ../multi-output-factor
rustup run stable cargo component build --target wasm32-unknown-unknown
cd ../cross-sectional-factor
rustup run stable cargo component build --target wasm32-unknown-unknown
cd ../repeated-factor-strategy
rustup run stable cargo component build --target wasm32-unknown-unknown
cd ../..
cargo test
```
