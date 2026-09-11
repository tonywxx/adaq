# AdaQ

[English](README.md) | [简体中文](README.zh-CN.md)

[![Release](https://github.com/tonywxx/adaq/actions/workflows/release.yml/badge.svg)](https://github.com/tonywxx/adaq/actions/workflows/release.yml)

> **AdaQ** (Ada Quant) 是一个 AI 驱动的量化交易平台，支持股票和数字加密资产。

## V1 状态：工程自动验收已成功

**AdaQ V1 工程自动验收已成功** —— 代码、构建、测试、组件、研究链路、Desktop、Paper Reconcile 与 Bot 安全生命周期已通过验收。最新一次自动化验收[在 macOS ARM64 与 Windows x86_64 上全绿](https://github.com/tonywxx/adaq/actions/runs/34547508572)，[release v0.9.8](https://github.com/tonywxx/adaq/releases/tag/v0.9.8) 已发布。

该结论有明确边界：它**不代表**策略盈利，也**不授权** Live Trading。当前 Demo Bot 仍无成交（0 fills）与 realized feedback 样本。AdaQ V1 是本地优先的研究、回测与模拟桌面应用，绝不执行真实账户订单；真实交易属于未来独立的、由主机控制的监督式里程碑。详细证据见 [V1 自动验收摘要](V1自动验收文档.md)与 [V1 模拟盘 Bot 分析报告](V1模拟盘Bot分析报告.md)。

## V1 工作流

AdaQ V1 是一条本地优先的闭环，按用户工作流组织：

**数据 → Feature / Factor / Model → Strategy / Backtest → Paper / Bot → Operations / Feedback**

| 阶段 | 当前可完成的工作 |
| --- | --- |
| **数据** | 检查 OKX Spot、中国 A 股与美国股票证据；采集、校验并冻结带 Source/Canonical/Quality provenance 的不可变 Market Data Snapshot，使用一个 User-scoped Watchlist。 |
| **Feature** | 在 `/features` 工作区发布因果 Feature Definition、拟合声明的 Transformation、冻结 Feature Plan，并物化不可变的 Parquet Feature Dataset。 |
| **Factor** | 在 `/factors` 工作区基于不可变 Factor Dataset 研究因子，进行因果评估、保留 Research Family lineage 与多重检验控制，并记录 User-owned Promotion Decision。 |
| **Model** | 在本地 Python 研究 Lab 训练 Qlib Ridge 模型；生成原生或外部（`.adaq-signals`）Forecast Signal Dataset 与不可变 Forecast Evaluation Report。 |
| **Strategy / Backtest** | 在不可变 Snapshot 上运行 Dataset-first 的沙箱化 Strategy Backtest，提供完整 provenance；用 chronological holdout、walk-forward 或 cross-market Protocol 做验证。 |
| **Paper / Bot** | 连接不下单的 Paper/Demo 账户（OKX Demo、Alpaca Paper、本地 A 股模拟器），完成 OKX Demo Paper 对账，并部署受监督、决策 fail-closed 的 Bot。 |
| **Operations / Feedback** | 在 Operations Dashboard 监控运行健康与告警；通过 Paper Feedback 与人工复核的 Research Review Decision 闭环。 |

底层能力：版本化 ABI 下的沙箱化 WebAssembly Factor/Strategy/Model 组件、可验证的内容寻址 `.adaq` 包、固定 C TA-Lib 指标引擎（160 个指标）、精确 Decimal 金融数值、不可变可审计的 Run，以及双语（English / 简体中文）Tauri 2 + React 19 桌面 GUI。

## AdaQ App

![AdaQ App](screenshots/adaq-app-ui-zh-CN.png)

## 快速开始

### 环境要求

- **Tauri 2 桌面构建工具链** —— 安装对应操作系统的 [Tauri 2 前置依赖](https://v2.tauri.app/start/prerequisites/)（WebKit/WebView2、C/C++ 构建工具链；macOS 需 Xcode Command Line Tools）。
- **Rust stable 工具链** —— 构建原生 Tauri 壳层所必需：

  ```sh
  rustup toolchain install stable
  ```

- **Node.js 20 LTS 或更新版本** 与 **pnpm 11**：

  ```sh
  npm install -g pnpm      # 或启用 corepack
  ```

- *（仅组件开发需要）* `wasm32-unknown-unknown` 目标，以及组件工具链 —— 见[开发组件](#开发组件)。

### 安装

```sh
pnpm install --frozen-lockfile
```

### 运行（开发模式）

```sh
pnpm tauri dev
```

该命令会启动 Vite 开发服务器（<http://localhost:1420>）并打开原生桌面窗口。

### 构建（生产 / 发布）

```sh
pnpm run build      # 严格 TypeScript 检查，然后构建前端
pnpm tauri build    # 为当前平台打包带签名的桌面安装包
```

发布打包（macOS ARM64、Windows x86_64）由 GitHub Actions `Release` 工作流自动完成，前提是先在 `package.json`、`src-tauri/Cargo.toml` 与 `src-tauri/tauri.conf.json` 中同步版本号。Linux 验证与打包暂缓。

### 校验（可选检查）

```sh
pnpm run build            # 前端 + 严格类型检查
cd src-tauri && cargo check   # Rust / Tauri
pnpm test                 # Jest
```

## 使用说明

### 登录

首次启动会显示由 Supabase 账户托管的登录界面。使用邮箱 + 密码（主路径）；首次可通过邮箱 OTP + 密码设置作为补充。绝不会要求任何真实交易凭证。

### 导入组件

1. 在侧边栏打开 **Components**。
2. 点击 **Import**，选择已验证的 `.adaq` 包 —— 例如你用 Component Developer Kit 构建的包，或 `examples/components` 中的示例。
3. 查看详情面板（参数、Feature Slots、依赖、Warmup、ABI/SDK/Manifest 版本、精确哈希）并确认。导入的组件会以兼容性与 Run-lock 状态出现在组件库中。

### 准备市场数据

回测运行于不可变的 Market Data Snapshot 之上。在 Backtest 的 **Data** 阶段，选择 Instrument 与 Bar Interval，然后复用已有 Snapshot（显示其区间、Bar 数量、来源与 ID），或冻结一个新的 Snapshot。Snapshot 来自导入/示例数据或外部适配器，例如 [Kronos 示例](examples/external-models/kronos/README.zh-CN.md)。

### 运行回测

Backtest 工作区在同一页面使用四个阶段：

1. **Data** —— 选择 Market Data Snapshot。
2. **Strategy** —— 选择 Strategy Component 并绑定其 Feature Slots / Forecast Signal Dataset；设置参数与 Position Mode（Long Only 或 Long–Short）。
3. **Execution** —— 选择 Execution Profile（费用、滑点、再平衡阈值等）。
4. **Results** —— 运行回测并查看四个标签页：
   - **Overview** —— 指标、权益、基准与回撤图表。
   - **Decisions** —— Target Decisions 与 Run Pauses。
   - **Execution** —— 分页的模拟订单、成交与费用。
   - **Provenance** —— Snapshot、Package、参数、Indicator Plan、Execution Profile、引擎身份、版本与 seed。

历史 Run 为只读。**Use as new configuration** 将某次 Run 的设置复制到一个全新的不可变 Run；任何改变后的执行都会创建新的 Run。

### 运行研究验证

1. 打开 **Validation**，选择一种方法：时间顺序留出（chronological holdout）、滚动前推（walk-forward）或跨市场。
2. 配置上下文并冻结一个 Validation Protocol。
3. 运行或恢复该 Protocol，然后查看 **Summary**、**Evidence** 与 **Provenance** 标签页。
4. 将 Report 导出为 **JSON** 或 **Markdown**。Recommended Contexts 仅是历史证据，绝不声称某种有利可图的未来配置。

### 设置与本地化

打开 **Settings → General** 可在 English (US)、简体中文与 System 之间切换 UI 语言；缺失的翻译会回退到英文。在 **Settings → Account** 可查看邮箱、修改密码并退出登录。

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
adaq-component verify dist/my-factor-0.1.0.adaq --previous ../my-factor-0.1.0/manifest.json
```

使用 `adaq-component new strategy my-strategy` 创建 Strategy 组件。将 `dist/` 中验证通过的文件导入 ADAQ 的组件库。`build` 会运行组件测试、构建 `wasm32-unknown-unknown`、执行主机 conformance 检查，并生成 `dist/*.adaq`。`verify` 在不修改原包的前提下校验已有包；`--previous` 还会检查文档化的 SemVer 契约。

请先阅读[可执行 Factor 与 Strategy 双语示例](examples/components/README.zh-CN.md)，再将 [SDK 指南](src-tauri/crates/adaq-component-sdk/README.zh-CN.md)、[CLI 指南](src-tauri/crates/adaq-component-tooling/README.zh-CN.md)和[组件架构](CONTEXT.md)作为参考。这些 crate 目前从本仓库安装；发布之后，`cargo install adaq-component-tooling --locked` 将独立于桌面应用安装相同的 CLI。

## 文档

文档按用户工作流组织。根目录状态文件：[V1 自动验收摘要](V1自动验收文档.md)与 [V1 模拟盘 Bot 分析报告](V1模拟盘Bot分析报告.md)（0 fills，非收益证明）。

| English | 简体中文 | 说明 |
| --- | --- | --- |
| [Research Workspaces](docs/research-workspaces.md) | — | Components、Backtest、Validation 工作区契约与人工验证路径 |
| [Market Workspaces](docs/market-workspaces.md) | [行情工作区中文](docs/market-workspaces.zh-CN.md) | 三市场观察、Watchlist、Provenance 与数据验证路径 |
| [Feature Engineering](docs/feature-engineering.md) | [特征工程中文](docs/feature-engineering.zh-CN.md) | Feature Definition、Plan、Fitting、Materialization 与 `/features` 工作区 |
| [Factor Research](docs/factor-research.md) | [因子研究中文](docs/factor-research.zh-CN.md) | Factor Lab、ABI v2、Evaluation、Promotion 与 `/factors` 工作区 |
| [Model Research](docs/model-research.md) | [模型研究中文](docs/model-research.zh-CN.md) | Forecast Signal Dataset、Forecast Evaluation、Python 研究 Lab 与 Qlib Ridge 路径 |
| [Strategy, Risk & Execution](docs/strategy-risk-execution.md) | [策略风险执行中文](docs/strategy-risk-execution.zh-CN.md) | Strategy Intent、Host Risk、OMS 与执行边界 |
| [Paper Trading Accounts](docs/paper-trading-accounts.md) | [模拟盘账户中文](docs/paper-trading-accounts.zh-CN.md) | Paper 账户、对账与 Currency Scoping |
| [Bot Runtime](docs/bot-runtime.md) | [Bot 运行时中文](docs/bot-runtime.zh-CN.md) | 受监督 Bot Worker、Attempt、调度与 fail-closed 安全设计 |
| [Operations Dashboard](docs/operations-dashboard.md) | [运行仪表盘中文](docs/operations-dashboard.zh-CN.md) | 首页选择、Operational Responsibility 与仪表盘边界 |
| [Monitoring & Alerting](docs/monitoring-and-alerting.md) | [监控告警中文](docs/monitoring-and-alerting.zh-CN.md) | 多维健康监控与 Append-only 告警 |
| [Research Feedback Loop](docs/research-feedback-loop.md) | [研究反馈闭环中文](docs/research-feedback-loop.zh-CN.md) | 把模拟盘证据闭环回人工复核的研究 |
| [A-share Data Path](docs/a-share-data-path.md) | [A 股数据路径中文](docs/a-share-data-path.zh-CN.md) | 中国 A 股采集与模拟器契约 |
| [A-share Paper Trading](docs/a-share-paper-trading.md) | [A 股模拟交易中文](docs/a-share-paper-trading.zh-CN.md) | 本地 A 股模拟器 Paper 执行 |
| [Alpaca Data Path](docs/alpaca-data-path.md) | [Alpaca 数据路径中文](docs/alpaca-data-path.zh-CN.md) | 通过 Alpaca 的美股数据采集 |
| [Paper Connections](docs/paper-connections.md) | [Paper 连接中文](docs/paper-connections.zh-CN.md) | Provider 连接、密钥存储与 No-order Invariant |
| [External Kronos Adapter](examples/external-models/kronos/README.md) | [外部 Kronos Adapter](examples/external-models/kronos/README.zh-CN.md) | 外部 `Kronos-small` 推理、规范 Forecast Signals、评估与 Dataset-first Backtest |
| [Component SDK](src-tauri/crates/adaq-component-sdk/README.md) | [Component SDK 中文](src-tauri/crates/adaq-component-sdk/README.zh-CN.md) | 用于实现 Factor 与 Strategy Component 的 Rust SDK |
| [CLI Tooling](src-tauri/crates/adaq-component-tooling/README.md) | [CLI 工具中文](src-tauri/crates/adaq-component-tooling/README.zh-CN.md) | 构建、验证与管理 `.adaq` 包 |
| [Component Template](src-tauri/crates/adaq-component-tooling/templates/README.md) | [组件模板中文](src-tauri/crates/adaq-component-tooling/templates/README.zh-CN.md) | 为生成的组件项目提供脚手架 README |
| [Executable Examples](examples/components/README.md) | [可执行示例中文](examples/components/README.zh-CN.md) | 端到端 Factor 与 Strategy SDK/CLI 教程 |
| [Test Fixtures](src-tauri/fixtures/README.md) | [测试固件中文](src-tauri/fixtures/README.zh-CN.md) | 供集成测试使用的 WASM 组件构建示例 |
| [Indicator Catalog](docs/reference/indicator-catalog.md) | [指标目录中文](docs/reference/indicator-catalog.zh-CN.md) | 160 个指标与 179 个输出，含输入、参数与 Warmup |
| [Research Metrics](docs/reference/research-metrics.md) | [研究指标中文](docs/reference/research-metrics.zh-CN.md) | 回测与研究绩效指标 |
| [Developing Components](docs/components/developing-components.md) | [开发组件中文](docs/components/developing-components.zh-CN.md) | Factor/Strategy 编写、Feature Slots 与 SemVer 规则 |

## 免责声明

**本软件仅供学习与研究目的使用（This software is for educational purposes only）。**

AdaQ 仅供学习与研究目的使用，不构成任何投资建议，其中的任何内容均不应被解释为买入、卖出或持有任何证券或数字资产的推荐。历史表现与回测模拟结果不代表未来收益。

使用本软件所产生的一切风险由使用者自行承担。在任何情况下，作者、贡献者与维护者均不对因使用或无法使用本软件而造成的任何直接、间接、附带、后果性或特殊损害（包括但不限于资金损失）承担任何责任。
