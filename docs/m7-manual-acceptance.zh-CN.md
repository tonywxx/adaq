# M7 人工验收（macOS ARM64）

这是本地优先研究工作区的标准人工验收路径。按顺序执行；若步骤失败，保留表中指定证据。完成的 Report 是历史证据，不是最佳 Strategy、盈利承诺、Paper Trading 或 Live Trading。

## 1. 前提条件与登录

| 操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| 在 macOS ARM64 安装 stable Rust、`wasm32-unknown-unknown` target、`cargo-component` 和本仓库 CLI：`rustup toolchain install stable`；`rustup target add --toolchain stable wasm32-unknown-unknown`；`cargo install cargo-component --locked`；`cargo install --force --path src-tauri/crates/adaq-component-tooling`。 | `adaq-component` 位于 `PATH`。 | 命令、完整终端输出和 `rustc --version`。 |
| 配置桌面构建环境，变量名为 `VITE_SUPABASE_URL` 和 `VITE_SUPABASE_PUBLISHABLE_KEY`；不要把值写入本指南或提交。用 `pnpm tauri dev` 启动应用。 | 出现登录界面，而不是缺少配置提示。 | 原始提示和变量**名称**，不要保存值。 |
| 输入已有账户邮箱，选择密码路径，输入密码并选择 **Sign in**。 | 显示 Dashboard 和侧边栏。 | 可见错误及可展开的技术详情；不得包含密码或 token。 |

首次设置是补充路径：输入新邮箱，使用邮件 OTP，然后创建并确认强密码（至少八个字符，含小写、大写、数字和符号）。预期结果：显示 Dashboard，之后可使用该密码登录。失败时保留可见错误和可展开的技术详情，但绝不记录 OTP、密码、token 或 Supabase value。

Windows 步骤相同，但在 PowerShell 中运行命令；在仓库根目录用 `cargo install --force --path .\src-tauri\crates\adaq-component-tooling`。命令使用 `.\` 路径，并在原生 Windows 文件选择器中选择包。

## 2. 从空项目编写并验证 Components

在空工作目录运行下列命令；已提交示例仅用于参考或恢复，不能替代本步骤。

```sh
adaq-component new factor m7-close-change
adaq-component new strategy m7-close-change-strategy
```

SDK 尚未单独发布。在每个生成的 `Cargo.toml` 中，把 `adaq-component-sdk` 依赖替换为指向当前 checkout 的本地 path，并保留生成的 feature：Factor 使用 `adaq-component-sdk = { path = "<absolute-path-to-adaq>/src-tauri/crates/adaq-component-sdk", features = ["factor"] }`，Strategy 使用 `features = ["strategy"]`。预期结果：Cargo 从当前 checkout 解析 SDK。失败时保留两个 `Cargo.toml` 和完整 Cargo error。

### Factor

在 `m7-close-change/src/lib.rs` 中用以下内容替换生成的源码：

```rust
use adaq_component_sdk::{decimal_to_f64, parse_decimal};
use adaq_component_sdk::factor::{ClosedBar, FactorSchema, Guest, GuestInstance, Instance as FactorInstance, NamedScalar, ParameterValue};
use core::cell::Cell;

struct Component;
struct Instance { previous_close: Cell<Option<adaq_component_sdk::Decimal>> }

impl Guest for Component {
    type Instance = Instance;
    fn describe() -> Result<FactorSchema, String> {
        Ok(FactorSchema { output_names: vec!["close-change".to_owned()], warmup_bars: 1 })
    }
    fn create(_parameters: Vec<ParameterValue>) -> Result<FactorInstance, String> {
        Ok(FactorInstance::new(Instance { previous_close: Cell::new(None) }))
    }
}
impl GuestInstance for Instance {
    fn process(&self, bars: Vec<ClosedBar>) -> Result<Vec<Option<Vec<NamedScalar>>>, String> {
        bars.into_iter().map(|bar| {
            let close = parse_decimal(&bar.close)?;
            let output = match self.previous_close.get() {
                Some(previous) => Some(vec![NamedScalar {
                    name: "close-change".to_owned(), value: decimal_to_f64(close - previous)?,
                }]),
                None => None,
            };
            self.previous_close.set(Some(close));
            Ok(output)
        }).collect()
    }
}
adaq_component_sdk::factor::bindings::export_factor!(Component with_types_in adaq_component_sdk::factor::bindings);
```

在 `m7-close-change/manifest.json` 中保留生成的 `componentId`、`sdkVersion` 和 `name`；完整 Factor 契约如下：

```json
{
  "manifestSchemaVersion": "1.0.0",
  "componentId": "<generated factor componentId>",
  "version": "0.1.0",
  "name": "M7 Close Change",
  "kind": "factor",
  "sdkVersion": "<generated sdkVersion>",
  "abiVersion": "1.0.0",
  "outputNames": ["close-change"],
  "warmupBars": 1
}
```

| 操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| `cd m7-close-change && adaq-component build` | 测试、Wasm 构建、合规检查以及 `dist/m7-close-change-0.1.0.adaq` 全部成功。 | 完整输出和 `manifest.json`；不得修改生成的 ID。 |
| `adaq-component verify dist/m7-close-change-0.1.0.adaq` | 包验证成功。记录 archive hash。 | 完整 verifier 输出和包路径。 |

### Strategy

将生成的 Factor `componentId` 复制到 Strategy Manifest。在 `m7-close-change-strategy/src/lib.rs` 中用以下源码替换：

```rust
use adaq_component_sdk::strategy::{FeatureFrame, FeatureSlot, Guest, GuestInstance, Instance as StrategyInstance, ParameterValue, SlotIndexes};
struct Component;
struct Instance { change: usize }
impl Guest for Component {
    type Instance = Instance;
    fn create(feature_slots: Vec<FeatureSlot>, _parameters: Vec<ParameterValue>) -> Result<StrategyInstance, String> {
        Ok(StrategyInstance::new(Instance { change: SlotIndexes::bind(&feature_slots)?.index("close-change")? }))
    }
}
impl GuestInstance for Instance {
    fn process(&self, frames: Vec<FeatureFrame>) -> Result<Vec<String>, String> {
        frames.into_iter().map(|frame| {
            let value = *frame.values.get(self.change).ok_or("feature slot count mismatch")?;
            Ok(if value > 0.0 { "1" } else { "0" }.to_owned())
        }).collect()
    }
}
adaq_component_sdk::strategy::bindings::export_strategy!(Component with_types_in adaq_component_sdk::strategy::bindings);
```

在 `m7-close-change-strategy/manifest.json` 中保留生成的 `componentId` 和 `sdkVersion`，并精确替换 `<factor-component-id>`：

```json
{
  "manifestSchemaVersion": "1.0.0",
  "componentId": "<generated strategy componentId>",
  "version": "0.1.0",
  "name": "M7 Close Change Strategy",
  "kind": "strategy",
  "sdkVersion": "<generated sdkVersion>",
  "abiVersion": "1.0.0",
  "featureSlots": [{"name":"close-change","source":{"kind":"external","dependencyAlias":"change","output":"close-change"}}],
  "dependencies": [{"componentId":"<factor-component-id>","version":"^0.1.0","alias":"change"}]
}
```

| 操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| `cd ../m7-close-change-strategy && adaq-component build` | Strategy 包创建在 `dist/`。 | 完整输出和两个 Manifest。 |
| `adaq-component verify dist/m7-close-change-strategy-0.1.0.adaq` | 验证成功。记录 archive hash。 | 完整 verifier 输出和包路径。 |

## 3. 导入并审计 Components

| 操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| 在侧边栏选择 **Components**，通过 **Import Component Package** 选择每个已验证的 `.adaq`，先导入 Factor。 | 两个 Packages 出现，导入反馈标识已导入 Component。 | 可见错误、可展开的技术详情和所选文件名。 |
| 依次选择每个 package。审阅 name、kind、version、compatibility、parameters、Factor outputs 或 Strategy Feature Slots/dependencies、Warmup、ABI/SDK/Manifest versions、archive/Wasm hashes。 | 精确身份可读且可复制；Strategy dependency 与已导入 Factor compatible。 | 截图、复制的 IDs/hashes 和技术详情。 |

## 4. 冻结数据并执行 Backtest

| 操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| 选择 **Backtest**。在 **Data and Strategy configuration** 中选择 Instrument、Bar Interval、start 和 end。若已有合适的 Market Data Snapshot 则复用，否则选择 **Download and freeze Snapshot** 并等待完成。 | 所选 Snapshot 显示 range、Bar count、source 和精确 ID。 | 阶段错误、所选值和新建时的 Snapshot ID。 |
| 选择 **M7 Close Change Strategy**，把 `change` 绑定到 **M7 Close Change**。审阅所有 Manifest parameters。 | 通过可读 identity 选择 compatible Factor。 | 消息和 component hashes。 |
| 在 **Execution and pre-Run review** 设置 initial quote allocation，并审阅全部 Execution Profile fields 及 Snapshot、packages、parameters、Feature Slots、Indicator Plan inputs。选择 **Run Backtest**。 | 一个 immutable Run 完成；运行中重复点击被禁用。 | 审阅界面、typed error details 和已有的 Run ID。 |
| 查看全部 result tabs：**Overview**（metrics/equity/benchmark/drawdown）、**Decisions**（Target Decisions 和 Run Pauses）、**Execution**（orders、fills、fees）、**Provenance**（Snapshot、packages、parameters、plan、profile、engine identities、versions、seed）。 | 每个 tab 均渲染证据与可复制 identities。 | 每个失败 tab 的截图和 Run ID。 |
| 在 **Provenance** 中选择 **Use as new configuration**。 | immutable Run 保持不变，其 normalized settings 填入当前表单；变更并执行会创建不同的 Run。 | 两个 Run IDs 和意外 mutation/error。 |

## 5. 验证并导出 Reports

对下面每种 method：选择 **Validation**，选择已完成的 Backtest Run，配置指定 evidence，选择 **Freeze Validation Protocol**，展开 **Review immutable Protocol**，然后选择 **Run / resume**。记录每个 Protocol 和 Report ID。

| Method | 精确操作 | 预期结果 / 失败证据 |
| --- | --- | --- |
| Chronological holdout | 选择 **Chronological holdout**，在冻结 Snapshot 内填写有效的 **Sample-out starts** boundary。 | 创建 `chronological-holdout@1` Protocol 和 Report；若无效，记录 boundary 与 typed error。 |
| Walk-forward | 选择 **Walk-forward**，填写有效 window size、step size、minimum history，并查看 preview。 | 创建 `walk-forward@1` Protocol 和 Report；若不可用，记录 values 与 gate message。 |
| Cross-market | 选择 **Cross-market**，添加有序 frozen Snapshot contexts；仅当 override 的 Run 使用该精确 Snapshot 时才使用 override。 | 创建 `cross-market@1` Protocol 和 Report；记录有序 Snapshot IDs，若不匹配记录 error。 |

对每个完成的 Report，查看三个 tabs：**Summary**（aggregate returns、fees、trades、consistency/dispersion）、**Evidence**（windows 或 markets、failures、Run Pauses、linked Runs，以及仅作为历史 evidence 的 Recommended Contexts）、**Provenance**（Protocol、Runs、packages、plans、snapshots、configurations、aggregation rules、versions）。选择 **Export JSON** 和 **Export Markdown**，保留生成的文件名。失败或中断的 Protocol 保持可用，可用 **Run / resume**；重试前记录 Protocol ID 和 technical error。

## 6. 自动验证与 CI

| 操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| 在 `src-tauri` 运行 `cargo test --workspace` 和 `cargo check --workspace`；在仓库根目录运行 `pnpm test` 和 `pnpm run build`。 | 全部命令成功退出。 | 完整失败命令输出和 revision。 |
| 推送验收 commit 后，记录适用 GitHub Actions run 的 URL、commit SHA 和 conclusion。 | 适用于已审阅 revision 的 workflow 成功。 | Run URL、failed job name 和未经删改的 log excerpt。 |

## 7. 验收记录

记录 macOS version/architecture、ADAQ revision、CLI/Rust versions、两个 package hashes、Snapshot IDs、Run IDs、Protocol IDs、Report IDs 和 JSON/Markdown export file names。隐藏 credentials、OTPs、tokens 和 Supabase values。与维护者一起审阅此记录后再宣布 M7 accepted。
