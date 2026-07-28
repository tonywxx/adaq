# ADAQ Component Tooling

`adaq-component-tooling` provides the shared package/conformance library used by the Tauri host and the standalone `adaq-component` CLI. The CLI is distributed separately from the desktop app.

## Install

Until the crate is published, install it from the ADAQ checkout:

```sh
rustup toolchain install stable
rustup target add --toolchain stable wasm32-unknown-unknown
cargo install cargo-component --locked
cargo install --path src-tauri/crates/adaq-component-tooling
```

After publication, the last command becomes:

```sh
cargo install adaq-component-tooling --locked
```

Cargo places `adaq-component` in its binary directory, normally `$CARGO_HOME/bin`. It is then available from any terminal when that directory is on `PATH`; it is not launched through Tauri.

## Workflow

```sh
adaq-component new factor my-factor
adaq-component new strategy my-strategy
cd my-factor
adaq-component build
adaq-component verify dist/my-factor-0.1.0.adaq
```

`new` creates `Cargo.toml`, `src/lib.rs`, `manifest.json`, and a project README. `build` runs the project's tests, performs a release `cargo component build`, runs the same conformance checks as the host, and writes an immutable `.adaq` package. `verify` checks an existing package without changing it.

For repository development only, set `ADAQ_COMPONENT_SDK_PATH` before `new` to make the generated project use a local SDK checkout instead of the exact published SDK version.
