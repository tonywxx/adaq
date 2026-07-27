# M1 component fixtures

The fixture crates intentionally build for `wasm32-unknown-unknown`: the
default WASI Preview 1 adapter adds forbidden ambient imports.

Build both fixtures before the host integration tests. On machines with both
Homebrew Rust and rustup, put the rustup shims first so cargo-component can see
the installed WASM target:

```sh
cd src-tauri/fixtures/factor
rustup run stable cargo component build --target wasm32-unknown-unknown
cd ../strategy
rustup run stable cargo component build --target wasm32-unknown-unknown
cd ../..
cargo test
```
