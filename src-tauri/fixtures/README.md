# M1 component fixtures

The fixture crates intentionally build for `wasm32-unknown-unknown`: the
default WASI Preview 1 adapter adds forbidden ambient imports.

Build both fixtures before the host integration tests:

```sh
cd src-tauri/fixtures/factor
cargo component build --target wasm32-unknown-unknown
cd ../strategy
cargo component build --target wasm32-unknown-unknown
cd ../..
cargo test
```
