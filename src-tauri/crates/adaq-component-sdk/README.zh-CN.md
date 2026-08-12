# ADAQ Component SDK

`adaq-component-sdk` 是一个独立于 Tauri 的 Rust SDK，用于实现 ADAQ Factor 和 Strategy 组件。它包含版本化的 WIT 合约、生成的绑定、精确十进制辅助函数和导出宏。

建议从 `adaq-component new` 模板开始，而不是手动接入 SDK。生成的 Factor 会启用 `factor` feature；生成的 Strategy 会启用 `strategy`。

## 组件约定

- Factor ABI v2 组件声明一个执行范围和有序功能槽位。时间序列因子接收主机解析的单 instrument 行；横截面因子接收带类型不可用单元格的、有序 Point-in-Time Universe 行。主机契约中的价格、数量和交易量保持十进制；仅在分析输出需要时转换为 `f64`。因子结果保留身份与顺序，并返回 1–64 个声明的有限输出或类型化缺失。
- Strategy 接收预先绑定的数值型 Feature Slots，每帧返回一个完整的目标敞口十进制字符串。
- 不要读取文件、网络、环境变量、时钟或随机数。宿主会拒绝 ambient WASI 导入，并验证确定性重放和分块独立性。
- 除非有意图地升级匹配的 SDK 和宿主合约，否则保持 `manifest.json` 中的 `sdkVersion` 和 `abiVersion` 不变。

SDK 是一个库，而非命令。请使用独立的 [`adaq-component` CLI](../adaq-component-tooling/README.zh-CN.md) 构建和打包项目。

## 仓库检查

```sh
cd src-tauri
cargo test -p adaq-component-sdk --all-features
```
