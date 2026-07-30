# Strategy Momentum Trend

[English](README.md) | [简体中文](README.zh-CN.md)

这是由 `adaq-component` 生成的可执行教学 Strategy。它组合 Market close、Built-in EMA 与外部 `factor-close-momentum-5` 输出。

请对照阅读 `src/lib.rs` 与 `manifest.json`，理解有序 Feature Slot、External 依赖别名、Built-in Indicator 参数引用、`SlotIndexes` 和完整的 Long Only Target Exposure 决策。

```sh
adaq-component build
adaq-component verify dist/strategy-momentum-trend-0.1.0.adaq
```

`build` 会运行测试、编译 release WebAssembly Component、执行 conformance，并将不可变 Package 写入 `dist/`。先导入 Factor Package，再在 Backtest 中绑定 `momentum` 依赖。

继续阅读完整的 [English tutorial](../README.md) 或 [简体中文教程](../README.zh-CN.md)。
