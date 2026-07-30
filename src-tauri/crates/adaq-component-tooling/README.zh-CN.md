# ADAQ Component Tooling

`adaq-component-tooling` 提供了 Tauri 宿主和独立 `adaq-component` CLI 共同使用的共享包/合规库。CLI 与桌面应用分开分发。

## 安装

在 crate 发布之前，从 ADAQ 仓库检出安装：

```sh
rustup toolchain install stable
rustup target add --toolchain stable wasm32-unknown-unknown
cargo install cargo-component --locked
cargo install --path src-tauri/crates/adaq-component-tooling
```

发布之后，最后一条命令变为：

```sh
cargo install adaq-component-tooling --locked
```

Cargo 会将 `adaq-component` 放置在其二进制目录中，通常是 `$CARGO_HOME/bin`。只要该目录在 `PATH` 中，就可以从任何终端使用它；它不通过 Tauri 启动。

## 工作流

```sh
adaq-component new factor my-factor
adaq-component new strategy my-strategy
cd my-factor
adaq-component build
adaq-component verify dist/my-factor-0.1.0.adaq
adaq-component verify dist/my-factor-0.1.0.adaq --previous ../my-factor-0.1.0/manifest.json
```

`new` 创建 `Cargo.toml`、`src/lib.rs`、`manifest.json` 和一个项目 README。`build` 运行项目测试、执行 release `cargo component build`、运行与宿主相同的合规检查，并写入不可变的 `.adaq` 包。`verify` 检查现有包而不修改它；`--previous` 还会报告已确认的 Manifest SemVer 兼容性规则。

在发布组件之前，请阅读完整的 [Factor 和 Strategy 开发指南](../../../docs/components/developing-components.zh-CN.md)。

仅用于仓库开发，在 `new` 之前设置 `ADAQ_COMPONENT_SDK_PATH` 可使生成的项目使用本地 SDK 检出，而非精确的已发布 SDK 版本。
