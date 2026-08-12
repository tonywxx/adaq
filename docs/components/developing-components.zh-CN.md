# 开发 ADAQ 组件

ADAQ 组件是本地的、确定性的 WebAssembly 组件。使用 `adaq-component` CLI；桌面应用导入已验证的 `.adaq` 归档文件，但它不是代码编辑器。

## 开始、构建、验证、打包

```sh
adaq-component new factor close-change
adaq-component new strategy trend
cd close-change
adaq-component build
adaq-component verify dist/close-change-0.1.0.adaq
adaq-component verify dist/close-change-0.1.0.adaq --previous ../close-change-0.1.0/manifest.json
```

`build` 运行组件测试、构建 `wasm32-unknown-unknown`、运行主机合规检查，并创建 `dist/*.adaq`。包仅包含 `manifest.json` 和 `component.wasm`。`verify` 验证现有包而不修改它；`--previous` 还会检查文档中定义的 SemVer 契约。

## 策略功能槽位

`manifestSchemaVersion` 恰好为 `1.0.0`。策略的非空 `featureSlots` 数组是其完整的、有序的 ABI 契约：名称是唯一的小写 kebab-case ASCII 标识符（最长 64 字节），顺序将成为客户端接收的密集索引。切勿输出 `inputNames`。

```json
{
  "featureSlots": [
    {"name":"close","source":{"kind":"market","field":"close"}},
    {"name":"ema","source":{"kind":"builtin","indicator":"ema","output":"value","inputs":{"real-0":"close"},"parameters":{"time-period":20}}},
    {"name":"change","source":{"kind":"external","dependencyAlias":"change-5","output":"close-change"}}
  ],
  "dependencies": [{"componentId":"11111111-1111-4111-8111-111111111111","version":"^1.0.0","alias":"change-5"}]
}
```

- `market` 绑定一个原始 OHLCV 字段，预热（Warmup）为零。
- `builtin` 绑定一个已文档化的目录指标输出。输入选择允许的 OHLCV 字段；参数是类型化的字面量或直接策略参数引用。省略的参数使用目录默认值。
- `external` 绑定 `dependencies` 中唯一命名的因子别名的输出。某个 K 线可能缺少因子；ADAQ 在所有槽位都存在之前不会调用策略。
- `signal` 声明 Forecast Signal 的语义要求：Prediction Kind、Forecast Target、horizon 与 Value Scale。Strategy 不固定 Model 或 Dataset；Backtest 配置绑定完全兼容的已完成 Dataset signal。

所有传递的槽位值均为有限的 `f64`。它们仅是分析值：不要将其视为权威金融值，也不接受 NaN/无穷大。使用 `SlotIndexes` 绑定名称一次，然后处理有序的帧值。

预热（Warmup）指第一个有可用输出的 K 线，而非数值收敛。[指标目录参考](../reference/indicator-catalog.zh-CN.md)报告了上游的`不稳定期`标志；它不添加隐藏历史。K线缺口（Bar Gap）会启动新的连续 K 线段，并重新创建因子和策略，因此预热重新开始。

## Model、Signal 与外部研究运行时

AdaQ Model Component 是确定性的沙箱化 WASM 推理包；其训练引擎不属于 Component ABI。模块化 Strategy 消费已完成的 Forecast Signal Dataset，绝不会隐式调用其生产 Model。冻结的 Backtest Feature Plan 与 Dataset Lock 保留所选 Dataset、signal contract、Producer provenance 与 evidence state。

大型 Python/PyTorch 模型使用[外部 Kronos Adapter 边界](../../examples/external-models/kronos/README.zh-CN.md)：推理在 AdaQ 之外运行，桌面应用只导入规范 `.adaq-signals` 证据。未来 Microsoft Qlib 可以通过相同边界训练或准备 Artifact。M8 不内嵌 Python 或 Qlib，不提供训练或受控 Python Runner，不宣称 Verified external inference，也不把外部 Artifact 发布到 Marketplace。

## 因子

Factor ABI v2 组件必须声明且只能声明一个 `factorScope`（`time-series` 或 `cross-sectional`）以及非空、有序的 `featureSlots`。时间序列因子接收主机解析的密集功能值，范围是单个 instrument 且按因果顺序排列；横截面因子接收完整、确定性排序的 Point-in-Time Universe 行，其中包括带类型的不可用单元格。两种范围都不能抓取数据或读取文件。将金融算术保持为十进制，只有在分析输出需要时才转换。`outputNames` 有序、唯一、小写 kebab-case，最多 64 个；每个结果必须保留行身份与顺序，并严格返回该声明的输出及有限值或类型化缺失。`warmupBars` 声明初始缺失，而不是收敛。Factor ABI v1 包不兼容，必须显式重置设备；ADAQ 不迁移也不双重读取旧包。

因子别名标识因子实例，而非组件 ID：同一包可以在不同别名和参数绑定下出现多次。每个别名在 K 线缺口后独立重新创建。

## 语义化版本

对于稳定版（`1.x+`）组件，移除、重命名、重新排序或更改功能槽位；更改其来源；移除或更改参数；更改依赖项、预热、清单/ABI 版本；或移除/重命名/重新排序因子输出需要主版本号。追加因子输出或添加带默认值的参数需要次版本号。算法修复和仅文档修正是补丁变更。`0.x` 版本为开发不稳定版。每个变更的包仍然需要新的组件版本，因为 ADAQ 锁定精确哈希。

## 故障排除

- `forbidden ambient imports`：为 `wasm32-unknown-unknown` 构建；不要使用 WASI、文件系统、网络、环境、时钟或随机数。
- `Factor runtime schema does not match manifest`：使 `describe()` 的输出名称和预热与 `manifest.json` 完全匹配。
- `Indicator Plan validation failed`：检查槽位顺序/名称、目录指标 ID、源输入、参数类型/范围以及外部别名/输出。
- `Target Exposure is invalid`：每帧返回一个有限的十进制目标值，在选定的仓位模式范围内。
- `chunk-boundary independent`：在连续的 `process` 调用间正确保持因子/策略状态；主机验证比较整体执行和分块执行的结果。

参见[清单参考](../reference/component-manifest.zh-CN.md)、[JSON Schema](../reference/component-manifest.schema.json) 和可执行的[测试固件](../../src-tauri/fixtures/README.md)。
