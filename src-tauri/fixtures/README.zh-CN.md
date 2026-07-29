# 可执行的 M5 组件示例

fixture crate 特意为 `wasm32-unknown-unknown` 构建：默认的 WASI Preview 1 适配器会添加禁止的 ambient 导入。

这些示例涵盖了纯 Market（`strategy`）、External（`external-strategy`）、混合 Market/Built-in/External（`mixed-strategy`）、多输出 Factor（`multi-output-factor`）和重复 Factor 别名（`repeated-factor-strategy`）的 Feature Slot Plan。

在运行宿主集成测试之前构建这些 fixture。在同时安装了 Homebrew Rust 和 rustup 的机器上，将 rustup 的 shim 放在前面，以便 cargo-component 能够看到已安装的 WASM 目标：

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
