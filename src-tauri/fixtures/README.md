# Executable M5 component examples

The fixture crates intentionally build for `wasm32-unknown-unknown`: the
default WASI Preview 1 adapter adds forbidden ambient imports.

The examples cover Market-only (`strategy`), External (`external-strategy`), mixed Market/Built-in/External (`mixed-strategy`), multi-output Factor (`multi-output-factor`), and repeated Factor aliases (`repeated-factor-strategy`) Feature Slot Plans.

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
cd ../mixed-strategy
rustup run stable cargo component build --target wasm32-unknown-unknown
cd ../multi-output-factor
rustup run stable cargo component build --target wasm32-unknown-unknown
cd ../repeated-factor-strategy
rustup run stable cargo component build --target wasm32-unknown-unknown
cd ../..
cargo test
```
