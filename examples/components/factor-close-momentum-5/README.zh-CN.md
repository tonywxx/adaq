# Factor Close Momentum 5

[English](README.md) | [简体中文](README.zh-CN.md)

这是由 `adaq-component` 生成的可执行教学 Factor。经过五根 Warmup Bar 后，它输出固定五根 Bar 窗口的收盘价动量百分比。

请对照阅读 `src/lib.rs` 与 `manifest.json`：`describe()` 与 Manifest 必须在 `close-momentum-5` 和 `warmupBars: 5` 上完全一致。金融数值保持精确十进制，直到最终转换为分析用 `f64`。

```sh
adaq-component build
adaq-component verify dist/factor-close-momentum-5-0.1.0.adaq
```

`build` 会运行测试、编译 release WebAssembly Component、执行 conformance，并将不可变 Package 写入 `dist/`。请从 ADAQ Component Library 导入该 `.adaq` 文件。

继续阅读完整的 [English tutorial](../README.md) 或 [简体中文教程](../README.zh-CN.md)。
