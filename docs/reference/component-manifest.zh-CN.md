# ADAQ 组件清单

生成自 `component-manifest.contract.json`；请勿编辑。

| 字段 | 契约 |
| --- | --- |
| `manifestSchemaVersion` | 清单契约版本。确切值：`1.0.0`。 |
| `componentId` | 不可变的组件标识。 |
| `version` | 组件语义化版本号。 |
| `name` | 人类可读的组件名称。 |
| `kind` | 组件 ABI 类型。值：`factor`、`strategy`、`model`。 |
| `sdkVersion` | 确切 SDK 版本。确切值：`0.1.0`。 |
| `abiVersion` | 按组件类型确定的确切 ABI 版本：Factor 为 `2.0.0`，Strategy/Model 为 `1.0.0`。 |
| `wasmSha256` | component.wasm 的 SHA-256；在打包时设置。 |
| `parameters` | 声明的组件参数，带类型化默认值。 |
| `factorScope` | Factor ABI v2 的唯一执行范围。值：`time-series`、`cross-sectional`。 |
| `featureSlots` | 有序的主机绑定功能槽位。Factor ABI v2、Strategy 和 Model 都需要一个或多个。 |
| `outputNames` | 有序的因子输出标识符。因子最多允许 64 个。 |
| `dependencies` | 外部因子依赖项，带唯一别名。 |
| `warmupBars` | 因子输出可用性的预热；并非收敛。 |
