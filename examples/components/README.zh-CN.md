# ADAQ Component 示例

[English](README.md) | [简体中文](README.zh-CN.md)

这些可执行示例使用真实的 Rust SDK 和 `adaq-component` CLI，讲解完整的 ADAQ Component 开发流程：

- [`factor-close-momentum-5`](factor-close-momentum-5/) 计算当前收盘价相对五根 Closed Bar 前收盘价的百分比变化。
- [`strategy-momentum-trend`](strategy-momentum-trend/) 组合原始 Market close、Built-in EMA 和外部动量 Factor，输出 Long Only Target Exposure。

示例刻意保持简单，不构成任何盈利声明。

## 1. 创建项目

仓库内的项目使用开发者实际使用的命令创建：

```sh
adaq-component new factor factor-close-momentum-5
adaq-component new strategy strategy-momentum-trend
```

`new` 会生成 `Cargo.toml`、`manifest.json`、`src/lib.rs` 和项目 README。仓库内示例使用本地 SDK 的相对路径；使用已安装 CLI 创建的独立项目会锁定已发布的 SDK 版本。

## 2. 理解 Factor

时间序列 Factor 声明一个有序的 `close` 功能槽位。它接收主机解析的单 instrument 行，保留最近五个收盘价并输出：

```text
close-momentum-5 =（当前收盘价 - 五根 Bar 前收盘价）/ 五根 Bar 前收盘价
```

前五个输出缺失，与 `warmupBars: 5` 完全一致。连续 `process` 调用之间保留状态；出现 Bar Gap 后，ADAQ 会创建新的 Factor Instance。实现只在完成精确十进制运算后才转换为有限 `f64`。

请对照阅读 [`src/lib.rs`](factor-close-momentum-5/src/lib.rs) 与 [`manifest.json`](factor-close-momentum-5/manifest.json)。`describe()` 返回的 schema 必须与 Manifest 中的 `outputNames` 和 `warmupBars` 完全一致。

## 3. 理解 Strategy

Strategy 声明三个有序 Feature Slot：

| Slot | 来源 | 教学目的 |
| --- | --- | --- |
| `close` | Market `close` | 演示原始 Market 字段 |
| `ema` | Built-in `ema.value` | 演示 Indicator Catalog 和 Strategy Parameter 引用 |
| `momentum` | External `momentum.close-momentum-5` | 演示独立打包的 Factor 依赖 |

它有两个参数：

- `ema-period`，默认值 `20`，冻结 Indicator Plan 时绑定到 EMA 的 `time-period`。
- `minimum-momentum`，默认值 `0`，传入 Strategy Component。

仅当 `close > ema` 且 `momentum > minimum-momentum` 时，Strategy 返回 Target Exposure `1`，否则返回 `0`。`SlotIndexes` 在 `create` 时一次性绑定名称；之后每个 Feature Frame 都是按 Manifest 顺序排列的稠密数组。

请对照阅读 [`src/lib.rs`](strategy-momentum-trend/src/lib.rs) 与 [`manifest.json`](strategy-momentum-trend/manifest.json)。依赖别名 `momentum` 将 External Slot 连接到 Host 选择的精确 Factor Package。

## 4. 构建与验证

安装 Rust stable toolchain、`wasm32-unknown-unknown`、`cargo-component` 和 `adaq-component` CLI。在仓库根目录执行：

```sh
cargo install --path src-tauri/crates/adaq-component-tooling --bin adaq-component

cd examples/components/factor-close-momentum-5
adaq-component build
adaq-component verify dist/factor-close-momentum-5-0.1.0.adaq

cd ../strategy-momentum-trend
adaq-component build
adaq-component verify dist/strategy-momentum-trend-0.1.0.adaq
```

`build` 会运行项目测试、为 `wasm32-unknown-unknown` 编译 release WebAssembly Component、执行 Host conformance，并将仅包含 `manifest.json` 和 `component.wasm` 的 Package 写入 `dist/*.adaq`。`verify` 会重新检查已有的不可变 Package，不会修改它。

## 5. 导入并回测

1. 在 ADAQ 中打开 **Component Library**。
2. 依次导入 `factor-close-momentum-5-0.1.0.adaq` 和 `strategy-momentum-trend-0.1.0.adaq`。
3. 打开 **Backtest**，选择 Instrument 并准备 Market Data Snapshot。
4. 选择 **Strategy Momentum Trend**。
5. 为 `momentum` 依赖选择 **Factor Close Momentum 5**。
6. 第一次运行保留 `ema-period = 20` 和 `minimum-momentum = 0`。
7. 运行 Backtest，查看 Warmup Pause、Target Decision、模拟 Order、Fill、Equity、费用和指标。

有效 Warmup 是所有 Slot 所需 Warmup 的最大值。Bar Gap 会重新创建分析实例并重新开始 Warmup；ADAQ 不会使用零或 NaN 填补缺失输入。

## 6. 每次修改一个地方来学习

- 提高 `minimum-momentum`，观察目标满仓的 Bar 是否减少。
- 修改 `ema-period`，观察冻结的 Indicator Plan 和 Run identity 如何变化。
- 重命名或调整 Feature Slot 顺序，然后执行 `verify`，理解为什么这是带版本的合约变更。
- 修改 Factor 公式、提升 Component 版本并重新构建，观察 Component Lock 如何锁定新的精确 Package。

## 故障排除

- **`forbidden ambient imports`**：使用 `wasm32-unknown-unknown`；Component 不能使用 WASI、文件系统、网络、环境变量、时钟或随机数。
- **Factor runtime schema does not match Manifest**：保持 `describe()`、`outputNames` 和 `warmupBars` 完全一致。
- **Indicator Plan validation failed**：检查 Slot 顺序、EMA 输入和参数绑定、依赖别名、所选 Factor identity 与输出名。
- **chunk-boundary independent**：在连续 `process` 调用之间保留分析状态，不能在 Host chunk 边界重置。
- **Target Exposure is invalid**：为每个收到的 Long Only Feature Frame 返回一个位于 `[0,1]` 的有限十进制字符串。

完整合约请参阅 [Component 开发指南](../../docs/components/developing-components.zh-CN.md)、[Manifest 参考](../../docs/reference/component-manifest.zh-CN.md)和 [Indicator Catalog](../../docs/reference/indicator-catalog.zh-CN.md)。
