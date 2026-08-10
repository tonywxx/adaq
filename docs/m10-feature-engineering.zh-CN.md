# M10 Feature Engineering

[English](./m10-feature-engineering.md)

状态：已接受的架构与可执行交付基线。M9 已完成；M10 issues 实现本契约。

## 最终结果

M10 交付一个 Host-owned、Tauri-independent 的 `adaq-feature-engine`，把不可变 M9 Market Evidence 转换为因果且可复现的 Feature Evidence。用户可以发布 Feature Definition、拟合声明式 Transformation、冻结 Feature Plan、物化不可变 Feature Dataset、检查其 Provenance 与 Missingness，并在 M11 Factor Research 中复用 Completed Dataset。

Historical Batch Materialization 与 Stateful Observation Evaluation 使用同一 Plan 和 Operator State Machine。M10 证明二者等价，但不把 Online Evaluator 连接到 Paper Provider 或 Trading Bot。

## 边界

M10 包含：

- Pointwise、Time-Series 与 Cross-Sectional Feature Scope。
- 有限、版本化 Feature Operator Catalog，而不是 Script 或通用表达式语言。
- Feature Plan `2.0.0`，替换 pre-v1 consumer-only Plan `1.0.0`。
- 精确 Availability、Warmup、Missingness、Typed Error、Fitting、Immutable Evidence 与 User-scoped Lifecycle Record。
- 本地化 `/features` Workspace，包含 Definitions、Fitting Attempts、Materialization Attempts 与 Datasets。

M10 不包含：

- 任意 Python、JavaScript、Rust、Notebook 或 `adaq:feature` Component 执行。
- Factor Research/Promotion、Model Training、Strategy Construction、Paper Order、Bot 或 Marketplace。
- M11 或后续 Workflow 隐式触发 Fitting 或 Feature Materialization。
- Future-return Feature、future-known backward adjustment、silent imputation、forward-fill、drop row 或修改 Canonical Market Data。
- Feature Dataset export 与拖拽式 Graph Canvas。

自定义可部署分析逻辑仍属于 Factor Component；Completed Feature Dataset 是显式下游边界。

## 所有权

`adaq-feature-engine` 拥有 Feature Definition、Feature Operator Catalog、Feature Plan Validation/Canonical Identity、Evaluation、Missingness、Fitting Contract 与 Batch/Observation Equivalence。`adaq-indicator-engine` 保持 TA-Lib 专用 Subengine。`adaq-component-tooling` 把 Component Manifest 和有序 Feature Slot 适配为 Plan Input。Application Command 只验证 User-scoped Request 并创建或查询 Attempt；可取消 Blocking Worker 执行重型工作。

本决定只 supersede ADR 0012 与 ADR 0020 中 Plan Schema 和 Ownership 部分；既有 Canonical Hash、权威 Slot Order、Finite Dense Component Input 与 Indicator Engine Boundary 保持不变。

## Identity 与 Schema

- Feature Definition family 使用稳定随机 `definitionId`、正整数 `revision` 与 JCS SHA-256 `definitionHash`。
- 可变 Name、Description、Tags 是 User-scoped Presentation Metadata，不进入 Semantic Hash。
- Feature Plan 使用 RFC 8785 Canonical JSON、lowercase SHA-256 与 `planSchemaVersion: "2.0.0"`。
- Plan 冻结 Definition Revision、有序 Output、Feature Scope、Operator Parameter、Fitted Transformation Artifact、Warmup、Availability、Missingness、Feature Operator Catalog、Feature Engine、Indicator Engine、Target/Build Identity 与 Seed。
- Plan 可复用，不绑定单一 Snapshot、Universe 或 Observation Range。
- Feature Materialization Request 绑定 User、Plan、Market Data Snapshot、Point-in-Time Instrument Universe、Observation Range、Parameter 与 Seed。
- pre-v1 不兼容 Feature Schema 启动时拒绝并要求显式 device-level Reset；M10 不提供 migration、dual reader 或 automatic deletion。

Canonical Definition/Plan JSON 上限为 1 MiB、256 DAG nodes、64 ordered outputs、DAG depth 64、100,000 effective Warmup Bars。Dataset 与 Runtime 上限由 M10 benchmark 决定。

## Feature 语义

每个 Feature Observation 由 Feature Output、Instrument ID 与 Observation Time 标识，内容是带 Available At 的 finite analytical `f64`，或 typed Unavailable。Canonical Decimal Input 仍是权威值；转换为 analytical `f64` 经过 checked conversion，但不声称 decimal bit-exactness。

Feature Scope 显式区分：

- Pointwise 读取一个 Instrument Observation。
- Time Series 按因果 Observation Time 顺序读取一个 Instrument。
- Cross Sectional 在一个 Observation Time 读取完整 Point-in-Time Instrument Universe。

M10 只允许 Pointwise → Time Series → Cross Sectional 的依赖扩张；Cross-Sectional Output 必须是终端。Cross-Sectional Plan 绑定单一 Venue、Asset Class、Bar Interval、Price Basis 与 Valuation Currency。Observed 与 Reconstructed Universe 可以物化并保留精确 Evidence State；Unknown 使完整 Batch Unavailable。

Available At 是全部 Input 与 Fitted Transformation Artifact Availability 的最大值。Corporate Action 使用记录的发布与生效 Evidence；本地计算时间只是 Operational Metadata，不属于 Historical Identity。

Unavailable Input 只影响依赖分支；依赖的 Stateful Branch 不吸收该值并重新 Warmup，无关分支继续。稳定 Reason 包括：

- `warmup`
- `bar-gap`
- `missing-market-input`
- `missing-dependency`
- `unknown-universe`
- `insufficient-coverage`
- `undefined-arithmetic`
- `artifact-missing-instrument`
- `corporate-action-unavailable`

预期内的未定义运算是 Unavailable。非有限 Indicator/Feature Engine Output、错误 Shape、无效 Identity 或其他 invariant failure 是 Fatal Typed Feature Evaluation Error，并保留 Stage、Node、Instrument、Observation Time 与安全 Diagnostics。

## Feature Operator Catalog 1.0

首版 Catalog 包含：

- Market OHLCV Field 与 checked arithmetic。
- 通过 `adaq-indicator-engine` 执行的 TA-Lib Indicator。
- 仅 backward-looking Simple Return 与 Log Return。
- Full-window rolling mean、population standard deviation、minimum、maximum 与 Realized Volatility。
- Quote Volume、rolling Quote Volume、zero-volume state 与 unit-preserving Amihud Illiquidity；没有可信分母时不把 Quote Volume 称为 Turnover，也不发明 Turnover Ratio。
- Venue-local trading day of week、trading month、minutes from session open、minutes to session close、Session Progress、one-hot 与 sine/cosine encoding。
- Cross-Sectional Rank、Percentile、Z-score。
- Causal forward Split adjustment 与独立 Dividend Total Return Feature。
- Fitted Standardization 与 Winsorization。

Rolling Window 按同一 Continuous Bar Segment 内连续 eligible Closed Bar 计数，必须填满完整窗口，排除 Scheduled Closure，并在 Bar Gap 或 unavailable dependency 后重启。Realized Volatility 是 per-Bar；Annualization 是单独、calendar-bound 的 Operator。

Cross-Sectional Rank 使用 ascending average ties。Percentile 为 `(rank - 1) / (n - 1)`；`n = 1` 时 Unavailable，reverse order 是显式参数。Z-score 使用 population variance，zero variance 时 Unavailable。Coverage Policy 冻结 minimum count/coverage，默认 100% coverage，保留全部 Universe members；只有显式降低阈值才允许 available subset，并记录实际 coverage。

## Fitted Transformation

Transformation Fitting Protocol 绑定一个 fitted node、Input Feature、Snapshot、Universe、Fitting Scope、Fitting Window、Algorithm Parameter、Engine Identity 与必填 `minimumSamples`。Fitting Scope 为 Pooled Universe 或 Per Instrument。Walk-forward 每个 fold 创建独立 Protocol/Artifact；即时 Cross-Sectional Z-score 不是 fitted transformation。

Completed Fitting Attempt 每个 fitted node 发布一个 Immutable Artifact。Standardization 使用 population variance，constant input 为 Unavailable。Winsorization 冻结 lower/upper quantile 与 nearest-rank rule。样本不足时失败且不发布 Artifact。Materialization 只能 apply，不能 refit。

Artifact Eligible At 是 fitting inputs 的最大 Available At；Created At 记录 operational completion，不改变 historical identity。Paper Deployment 只能在 Artifact 实际存在并完成后续 qualification 后引用它。

## Attempt、Storage 与 Recovery

SQLite 保存 User-scoped Definition、Plan、Protocol、Artifact、Request、Attempt、Manifest、Reference、Lifecycle State 与 Presentation Metadata。Immutable Feature Dataset Row 使用 content-addressed Parquet，每行是 `(Instrument ID, Observation Time)`；每个 Output 通过 Canonical Manifest Mapping 保存 Value、Available At、State 与 versioned Reason Code。

Fitting 与 Materialization Attempt 经过 Pending、Running 和且仅一个 Terminal State：Completed、Failed 或 Cancelled。相同 Pending/Running Request 合并，相同 Completed Evidence 复用，Retry 创建新 Attempt 并保留 Source Evidence。

设备通过 persistent FIFO 一次只运行一个 heavy Feature Attempt。Pending 在重启后继续；遗留 Running 变为带 interruption evidence 的 Failed。Progress 只在完整 Feature Observation 完成后递增，不伪造剩余时间。

Materialization 先写私有 Staging，校验完整 Schema、Row、Hash 后原子发布 Payload，最后记录 Completed。Cancel/Crash 不暴露 Partial Dataset。被引用的 Dataset/Artifact 删除锁定；deduplicated bytes 只有在最后一个 User Reference 消失后才删除，且不会授予跨 User 可见性。

## Historical 与 Observation Execution

Pointwise/Time-Series Branch 按 Instrument 与 Continuous Bar Segment 流式执行；Cross-Sectional Branch 按完整 Observation-Time Batch 执行。Chunk Size 不进入 Identity，也不能改变 Output。Batch Materialization 与 Stateful Observation Evaluation 必须在 Chunk Boundary、Bar Gap、Missing Dependency 与 Restart Reconstruction 下产生等价 Feature Observation。

M11 只能消费 Completed Feature Dataset。M10 Definition 不能依赖 Factor Output；Component Adapter 继续支持现有 Strategy/Model Slot 绑定 External Factor，但不得产生 Definition Cycle。

## Feature Workspace

`/features` 立即 Paint，包含 Definitions、Fitting Attempts、Materialization Attempts、Datasets。每个 Control 拥有自己的 Loading；User-scoped Read List 可先显示 Current-session Cache，再后台刷新。

Definition Editor 使用 Accessible Ordered Node List，而不是 Canvas。每个 Node 显示 Operator、Inputs、Parameters、Output Names、Scope、Availability 与 Warmup。Preview 使用 Production Engine 和 bounded immutable Snapshot Selection，可限制 Observation Time，但 Cross-Sectional 必须保留完整 Universe；Preview 不执行 Fit，也不产生 Evidence Identity。

Dataset Inspection 显示 Manifest/Provenance、每 Output Coverage、Unavailable Reason Counts、Minimum、Maximum、Mean、Population Standard Deviation，以及按 Instrument、Time、Output、State 过滤的 50-row pagination。Rebuildable Summary 只是 Content Inspection，不是 Factor/Model Evaluation。

## 验收

Reference Journey：

1. OKX Spot：Return、RSI、Realized Volatility、Bar Gap、Chunk Equivalence。
2. 中国 A 股：Venue Calendar、午休 Session Progress、Split、Dividend、Causal Availability。
3. 美国股票：Point-in-Time Universe、Cross-Sectional Rank、Coverage、Reconstructed/Unknown Behavior。

Failure Coverage 包括 Fitting Leakage、Insufficient Samples、Undefined Arithmetic、Non-finite Engine Output、Cancellation、Interruption Recovery、Atomic Publication、Incompatible Schema Rejection、User Isolation、Deletion Lock 与 Batch/Observation Equivalence。

M10 Performance Acceptance 使用 1,000,000-Bar Time-Series Workload 与 10,000-Instrument × 252-Observation Cross-Sectional Workload，证明 bounded memory、cancellation、chunk equivalence 与 responsive GUI scheduling，并记录 canonical macOS ARM64 baseline，不预先编造 latency/RSS target。

每个 Child 把所有 Acceptance Criterion 映射到 Implementation 与独立 Evidence。最终 Gate 包括 Focused Test、`cargo fmt --all --check`、`cargo test --workspace`、`cargo check --workspace`、Frontend Jest、`pnpm run build`、Lint、`git diff --check`、双语 parity、Accessibility 与 supported-platform CI evidence。

## 交付切片

M10 通过十个 dependency-ordered slices 交付：

1. [#78 — Core Contract、Feature Operator Catalog、Plan 2.0、Identity](https://github.com/tonywxx/adaq/issues/78)。
2. [#79 — Pointwise/Time-Series Operator](https://github.com/tonywxx/adaq/issues/79)。
3. [#80 — Cross-Sectional Scope 与 Universe Operator](https://github.com/tonywxx/adaq/issues/80)。
4. [#81 — Fitting Protocol、Attempt、Artifact](https://github.com/tonywxx/adaq/issues/81)。
5. [#82 — Feature Dataset Materialization Lifecycle 与 Parquet Evidence](https://github.com/tonywxx/adaq/issues/82)。
6. [#83 — Batch/Observation Equivalence 与 Component Integration](https://github.com/tonywxx/adaq/issues/83)。
7. [#84 — User-scoped Native API 与 Background Runner](https://github.com/tonywxx/adaq/issues/84)。
8. [#85 — Localized Feature Workspace](https://github.com/tonywxx/adaq/issues/85)。
9. [#86 — Three-market Fixture、Benchmark、Hardening](https://github.com/tonywxx/adaq/issues/86)。
10. [#87 — Bilingual Cross-platform Acceptance](https://github.com/tonywxx/adaq/issues/87)。

M10 已发布为 [Parent Issue #77](https://github.com/tonywxx/adaq/issues/77)。依赖为 `1 → {2,4,5}`、`2 → 3`、`{2,3,4,5} → 6`、`{5,6} → 7`、`7 → 8`、`{6,7} → 9`、`{1…9} → 10`。M10.1 是唯一初始可执行 Frontier。
