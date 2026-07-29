# ADAQ

[![Release](https://github.com/tonywxx/adaq/actions/workflows/release.yml/badge.svg)](https://github.com/tonywxx/adaq/actions/workflows/release.yml)

> **ADAQ** (Ada Quant) 是一个 AI 驱动的量化交易平台，支持股票和数字加密资产。

## 功能特性

- 可复现的本地市场数据研究与回测
- 沙箱化的 WebAssembly Factor 和 Strategy 组件
- 不可变、可验证的 `.adaq` 组件包

## 开发组件

组件源代码使用 Rust 编写。Tauri 应用导入并运行编译完成的 `.adaq` 包；它不提供 GUI 代码编辑器，也不捆绑 `adaq-component` CLI。

在本仓库中：

```sh
rustup toolchain install stable
rustup target add --toolchain stable wasm32-unknown-unknown
cargo install cargo-component --locked
cargo install --path src-tauri/crates/adaq-component-tooling

adaq-component new factor my-factor
cd my-factor
# 编辑 src/lib.rs 和 manifest.json。
adaq-component build
adaq-component verify dist/my-factor-0.1.0.adaq
```

使用 `new strategy my-strategy` 创建 Strategy 组件。将 `dist/` 中验证通过的文件导入 ADAQ 的组件库。

请参阅 [SDK 指南](src-tauri/crates/adaq-component-sdk/README.zh-CN.md)、[CLI 指南](src-tauri/crates/adaq-component-tooling/README.zh-CN.md) 和 [组件架构](CONTEXT.md)。这些 crate 目前从本仓库安装；发布之后，`cargo install adaq-component-tooling --locked` 将独立于桌面应用安装相同的 CLI。
