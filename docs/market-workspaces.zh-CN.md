# 行情工作区用户指南

[English](./market-workspaces.md)

状态：V1 行情观察、Navigation、Provenance 与用户验收契约。

相关指南：[Operations Dashboard](./operations-dashboard.zh-CN.md)、[Paper Trading Account](./paper-trading-accounts.zh-CN.md) 与 [实时监控和异常报警](./monitoring-and-alerting.zh-CN.md)。

## 为什么行情页面属于 V1

V1 会获取、验证、研究并 Paper Trade Crypto、中国 A 股和美股。用户必须能够检查相关 Instrument、Session State、Bar、Provider Limitation、Freshness 与 Quality。只有 Data Connector、没有可检查 Market Workspace 的股票支持在运行上并不完整。

因此 V1 包含三市场基础行情观察，但不尝试复制 Bloomberg 类型的完整终端。

## Navigation

```text
行情 / Markets
├── 行情总览 / Overview        /markets
├── 加密货币 / Crypto          /markets/crypto
├── A 股 / China A-shares      /markets/a-shares
└── 美股 / U.S. Equities       /markets/us-equities
```

现有 Crypto Watchlist、Ticker 与 Kline 体验从旧 `/` Home 移到 `/markets/crypto`；`/` Route 成为 Operations Dashboard。

## 共享页面契约

每个 Market Route 根据其证据能力提供：

- Venue-specific Instrument Search 与确定性 Result Identity。
- 当前 Venue-local Trading Date、Session Phase、Open/Closed State 与下一个已知 Session Boundary。
- 根据 Route Market 过滤的 User Watchlist，以及通过 Overview 访问跨市场内容。
- Ticker，包括可用时的 Last Price、Best Bid/Ask、Volume、Provider Time 与 Observation Age。
- Historical Closed-Bar Chart 与支持的 Bar Interval。
- Instrument Lifecycle Status、适用 Market Rule Summary 与显式 Unknown Value。
- Market Data Provider、Feed、Connector、Capability、Delay、Coverage、Provenance 与 Data Quality State。
- Data Refresh、Stale、Degraded、Gap 与 Provider Error State。
- 进入适用 Dataset、Research、Backtest、Strategy 或 Bot Configuration Workflow 的链接。

Market Page 用于检查 Evidence 和启动 Workflow，绝不能直接提交 Order 或绕过 Strategy、Host Risk、OMS 与 Paper Execution Adapter。

## 行情总览

`/markets` 是紧凑跨市场入口，不是第二个 Operations Dashboard。它显示：

- 带 Market 与 Venue Label 的统一 Asset-neutral Watchlist。
- A 股、美股与 Crypto 当前 Session Phase 和 Data-health Summary。
- Provider Coverage、Last Successful Update、Degraded/Unavailable State，以及进入受影响 Market 的链接。
- Recently Viewed Instrument 与各 Market Workspace 的直接导航。

Overview 不会把不同 Price、Volume、Return、Currency 或 Account Balance 相加成虚假 Global Metric。

## 一个 Asset-neutral Watchlist

Watchlist 保存完整 Instrument ID，而不是 Display Symbol。例如每个项目保留精确 Venue 与 Native Code；两个 Provider 或 Venue 使用相同可见 Symbol 时仍保持不同身份。

- Overview 可以显示全部 Watchlist Item，并按 Market 与 Venue 过滤。
- Crypto 只显示合格 Crypto Instrument。
- A 股只显示合格中国 A-share Instrument。
- U.S. Equities 只显示合格美国 Equity Instrument。
- 切换 Route 时 User-scoped Ordering 保持稳定。
- Route Filter 绝不能删除其它 Market 的 Item。

## Crypto Workspace

`/markets/crypto` 保留现有 OKX Spot 体验，并扩展证据展示：

- OKX Spot Instrument Search 与 Lifecycle Status。
- 现有 Watchlist、Selected Instrument、Ticker 与 Kline Chart。
- UTC Continuous-market Bar Grid 与 Data Age。
- Provider Connection、Stream、REST Reconciliation、Instrument Master 与 Data Quality Status。
- Active 时显示当前 Selected Instrument 的 Trade 与 Level 2 Health；V1 不提供 Historical Order-book Replay。

Funding Rate、Future、Perpetual、Option 与 Derivative 不属于 Spot Workspace。

## 中国 A 股 Workspace

`/markets/a-shares` 展示通过 `akshare-rs` 获得的 A 股证据，并在可用时明确 Actual Upstream Source 与 Method，包括：

- Venue 与 Native Instrument Code、Board/Segment、Listing 与 Suspension Status。
- `Asia/Shanghai` Trading Date、Auction、Continuous、Midday Break 与 Closed Phase。
- Ticker 与具有显式 Price Basis 的 Unadjusted Historical Bar。
- Corporate Action Availability，以及存在 Adjusted View 时的 Derived-adjustment Provenance。
- 适用 Price Limit、Lot、T+1、Special Treatment、Fee 与其它 Market Rule Summary。
- Provider 或 Effective-time Rule 无法确定事实时的显式 Unknown State。

Market Workspace 不会暗示 `akshare-rs` 是 Venue 或原始 Data Owner，也不会根据不完整 Response 虚构 Realtime Coverage、Queue Data 或 Tradeability。

## 美股 Workspace

`/markets/us-equities` 使用 Alpaca Market Data 作为 V1 Primary Source，并明确显示 Plan/Feed Limitation：

- Venue、Native Ticker、Listing 与 Trading Status。
- `America/New_York` Regular、Extended、Holiday 与 Early-close Session Evidence。
- 带 Feed Identity、Observation Time 与 Coverage/Delay Badge 的 Ticker 和 Historical Bar。
- Corporate Action Evidence 与声明 Price Basis。
- 当前 Account 相关 Provider Capability 与 Streaming-symbol Limit。

Alpaca IEX-limited Data 绝不能被标记为 Consolidated Whole-market Realtime。辅助 `yfinance-rs` Observation 只能在具有自身 Provenance 时用于支持的 History、Corporate Action、Fundamental 或 Cross-check，绝不能静默修复或替换 Canonical Realtime Evidence。

## Time、Formatting 与 Identity

- Canonical Event 与 Bar Boundary 保持 UTC Instant。
- Trading Date、Session、Bar Alignment 与 Rule 使用各 Venue IANA Time Zone。
- GUI 默认以 Venue-local 显示 Market Time，并可附加 Device Time 或 UTC，但不会改变 Identity。
- English (US) 与简体中文只改变 Label 与 Formatting，不改变 Instrument ID、Time、Price、Provenance 或 Evidence。
- Financial Value 保持精确 Decimal；Formatted String 绝不能重新进入 Calculation。

## Loading 与 Freshness

- 每个 Market Route 立即 Paint Route Shell。
- Instrument Search、Watchlist、Ticker、Chart、Rule 与 Quality Panel 各自拥有 Loading 与 Failure State。
- 同一 User Session 返回页面时，可以立即显示 User-scoped Read Cache，再在后台刷新。
- 每个 Live 或 Cached Value 都显示 Provider Observation Time 与 Current Age。
- Stale Cache 绝不能改变 Instrument Status、Account Truth、Bot Eligibility 或 Order Authority。
- Missing/Degraded Provider Data 保持可见；没有新的显式 Provenance Identity 时，GUI 绝不能替换 Provider。

## V1 边界

V1 提供足够行情观察能力，用于检查三条 Data 与 Paper Trading Path。以下能力属于 V1 之后：

- Advanced Multi-condition Stock/Crypto Screener。
- 完整 Fundamental 与 Financial-statement Terminal。
- News、Sentiment 与 Alternative-data Workspace。
- Historical Level 2、Order-book Reconstruction、DOM 与 Queue Analytics。
- Multi-window/Linked Multi-chart Terminal、Advanced Drawing 与 Chart Custom Study。
- Option、Future、已接受 Spot Scope 之外的 Perpetual 与其它 Derivative。
- Cross-market Arbitrage 与 Consolidated Smart-order-routing Workspace。

## V1 验收检查

1. 现有 Crypto Watchlist、Ticker 与 Kline 行为在新 Route 保持可用。
2. 四个 Route 都立即 Paint，并按 Control 隔离 Loading、Empty、Stale、Degraded 与 Failure State。
3. 一个 User-scoped Watchlist 保留精确 Instrument ID，并在三个 Market 正确过滤。
4. 各 Market 在 Holiday、Break、Early Close 与 Daylight-saving 变化下使用正确 Venue Calendar 和 Time-zone Semantics。
5. Ticker 与 Bar 显示 Provider Identity、Feed Coverage/Delay、Observation Time 与 Data Quality State。
6. A 股 Unknown Rule 与美股 IEX-limited Coverage 绝不能被展示为完整 Evidence。
7. Market Workflow Link 绝不能绕过 Research Qualification、Host Risk、OMS 或 Paper Execution Adapter。
8. English (US) 与简体中文提供等价功能与 Accessible Label。

## 已实现 GUI 与截图预期

桌面实现现在提供上述四个 Route，保留 `/markets/crypto` 的现有 Crypto Workspace，并在三个 Market Filter 之间使用同一个 Venue 加 Native Code 的 Watchlist Identity。A 股与美股页面会明确展示 Provider Observation、Calendar Coverage、不可用的 Bid/Ask 与 Direct-provider Bar Quality，不会把它们升级为 Canonical Evidence。

手工验收截图应覆盖两个支持的 Locale 下的每个 Route，并至少包括：

- 同时包含两个 Market Instrument 的 Overview Watchlist；
- 显示 Native Code、Venue 与 Provider Identity 的 A 股或美股 Search Result；
- 显示 Venue-local Trading Date、Time Zone，以及 Evidence 不可用时的 Unknown 或 Closed 的 Session/Calendar Card；
- 显示明确 Unavailable Value、Provider Observation Time 与 Data Quality State 的 Ticker 或 Chart Card；
- 在保留 Route Shell 的情况下展示 Empty、Loading、Degraded 或 Provider Error Control State。

截图只作为验收 Evidence；当 Provider Contract 没有建立 Realtime、Consolidated、Adjusted 或 Canonical Quality 时，截图不能暗示这些能力存在。
