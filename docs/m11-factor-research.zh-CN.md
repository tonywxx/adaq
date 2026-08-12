# M11 Factor Research and Promotion

[English](./m11-factor-research.md)

状态：已接受的架构与可执行交付基线，已发布为 [Parent Issue #88](https://github.com/tonywxx/adaq/issues/88)。M10 已完成；本文档不声称已有任何 M11 Production Implementation。

## 最终结果

M11 交付一个 Host-owned Factor Lab 和 Tauri-independent `adaq-factor-research` Core。用户可以发布 Declarative Factor Definition 或构建私有 Custom Factor Candidate，从 Completed M10 Feature Dataset 物化不可变 Factor Dataset，在 Scope-correct、Cost-aware Protocol 下评价精确 Output，在 Research Family Lineage 中保留每个 Trial，并显式记录 Rejected、Research Validated 或 Component Eligible Promotion Decision。

Research Validated Output 成为 M12 Model Research 可精确选择的 Evidence。M11 不保证未来盈利，不会根据分数自动晋升，不导入生成的 Component，也不执行 Python 或 Qlib。

## 边界

M11 包含：

- 一个 Factor Product 下的 Time-Series 与 Cross-Sectional Factor Scope。
- 复用 Feature Operator Catalog 语义的 Declarative Factor Definition，以及构建为私有、未导入 Candidate Package 的 Custom Factor Project。
- 使用 Host-resolved Ordered Feature Input 和 Scope-specific Batch 的 Factor ABI v2。
- 不可变 Factor Materialization 与 Evaluation Evidence。
- 带精确 Purge/Embargo 的 Chronological Holdout 与 Walk-forward Evaluation。
- Temporal、Cross-Sectional 与标准化 Economic Lens；Neutralization、Robustness、Decay、Stability、Regime 与 Multiple-testing Evidence。
- User-owned Promotion Decision 与派生的 Promoted Factor Library。
- 覆盖 Families、Candidates、Datasets、Evaluations、Decisions 的本地化 `/factors` Workspace。

M11 不包含：

- Qlib、Python、Notebook 执行、Notebook-to-Rust 转换或第二个 Research Engine。
- Model Training、Forecast Signal Dataset、Strategy Construction 或 Backtest 改造。
- Component Equivalence、Qualified Package Import、Marketplace、Paper、Bot、Live 或真实资金工作。
- Automatic Promotion、Automatic Feature Mining、Bayesian Optimization、Genetic Search 或 Mutable `latest` 引用。
- Cross-market 样本混合或 Universal Profitability Threshold。

## 责任归属

`adaq-factor-research` 拥有 Factor Materialization Contract、Factor Dataset Evidence、版本化 Factor Metric Catalog、Evaluation Protocol/Report、Research Family/Trial、Promotion Policy/Decision 与 Promoted Factor Library Projection。`adaq-feature-engine` 继续权威拥有 Feature Definition、Feature Plan、Availability、Warmup、Missingness 与 Completed Feature Input。`adaq-component-tooling` 和 SDK 拥有 Factor ABI v2、Candidate Package Validation、WASM Sandbox 与 Package Contract。

M11 只使用 ADAQ Native Research Engine。每个结果冻结 Research Engine Provenance。未来 Qlib Adapter 可以在不同 Engine Identity 下产生可比较 Evidence，但不能静默声称公式或数值等价。

## Candidate、Revision 与 Identity

Factor Candidate 是一个精确 Declarative Factor Definition Revision 或私有 Custom Factor Package：

- Mutable Declarative Draft 没有 Evidence Identity；Publish 产生正整数 Revision、canonical RFC 8785 JSON 与 lowercase SHA-256 Identity。
- Declarative Logic 复用 Feature Operator Catalog Operation 与 Plan 2.0 语义；M11 不增加平行 Expression Language。
- Name、Description、Factor Tag 是 Hash 之外的 User-scoped Presentation Metadata。
- Custom Factor Project 是精确 User-authored Rust Source。Candidate Build Attempt 冻结 Source Hash、SDK、ABI、Toolchain、Target、Command、Environment、Resource Policy、Log 与结果 Package Hash。
- Candidate Build 使用固定 Host Command，不允许 Network 或 Custom Script。成功 Package 保持 Private、Non-imported；M14 拥有 Qualification 与 Component Library Import。
- Parameter Search 仅是最多 256 个 Trial 的显式、确定性 Cartesian Grid；M11 不做 Adaptive Optimization。

相同 Candidate Hash、Target、Universe、Window 或显式 Derivation 会在 Research Family 之间建立 Lineage。不得删除 Family/Lineage 来规避 Multiple-testing Evidence。

## Factor ABI v2

每个 Factor 声明一个 Scope、Ordered Feature Slot、Parameter、1–64 个 Named Output 和精确 Runtime Limit。Component 不能 Fetch Data、检查 Ambient Time、Refit Transformation、读取 File、使用 Network/Randomness 或静默替换 Input。

- Time-Series Execution 接收一个 Instrument 按 Causal Observation Time 排序的 Dense Present Row。Missing Input 或 Bar Gap 由 Host 发布为 Unavailable；Host 重建 Instance 并重启 Warmup，不把 Partial Row 发送给 Component。
- Cross-Sectional Execution 接收一个 Observation Time 下确定排序的完整 Point-in-Time Instrument Universe。每个 Slot Cell 是 Available 或 Typed Unavailable；Component 必须为每个成员按原 Identity/Order 返回结果，不得删行或重排。
- Host 在发布前验证 Membership、Order、Row/Output Count、Availability、Finite Value、Determinism、Fuel、Memory 与 Output Identity。

Factor ABI v2 直接替换 pre-v1 Factor ABI v1。不兼容 Stored Package/Evidence 以明确 Device-level Reset Guidance 拒绝；M11 不提供 Migration、Dual Reader 或 Automatic Deletion。

## Materialization 与 Storage

Evaluation 不会隐式计算 Candidate。Factor Materialization Protocol 把 Candidate 绑定到精确 User、Feature Dataset/Plan、Parameter、Market Data Snapshot、Point-in-Time Instrument Universe、Observation Range、Market Context、Runtime/Engine Identity 与 Seed。只有 Completed Factor Materialization Attempt 才能原子发布 Factor Dataset。

一个 Factor Dataset 在按 `(Instrument ID, Observation Time)` 标识的 Wide Parquet Row 中保留 1–64 个 Output。每个 Output 保留 finite `f64` 或 Typed Unavailable State、Available At、Reason 与 Provenance。SQLite 保存 User-scoped Metadata、Canonical Protocol、Attempt、Manifest、Reference、Presentation Record 与 Lifecycle State；Payload Byte 是 Immutable、Content-addressed。

精确 Active Request Coalesce，精确 Completed Evidence Reuse；Retry 创建引用 Source 的新 Attempt；Failed/Cancelled Attempt 保留 Safe Diagnostic。Publication 使用 Private Staging、完整 Validation 与 Atomic Cutover。Referenced Evidence 被 Deletion Lock；Shared Byte 仅在最后一个 User Reference 消失后删除，且不授予 Cross-user Visibility。

## Target 与 Market Context

M11 只支持带一个或多个正整数 Bar Horizon 的 `Future Close Return`：

`close[t + h] / close[t] - 1`

Factor Output 必须在 `t` 时 Available。Target Evidence 使用相同 Instrument、Bar Interval、Market Data Snapshot 与 Price Basis。Horizon 内出现 Bar Gap、Missing Close 或无法验证 Corporate Action 时，Label 为 Typed Unavailable；Scheduled Closure 不算 Gap。Binary/Custom Target 留给未来。

一个 Dataset/Report 只绑定一个可比较 Market Context：Venue、Asset Class、Bar Interval、Price Basis、Valuation Currency 与 Point-in-Time Instrument Universe。Cross-market Robustness 在同一 Research Family 下使用独立 Report，绝不混合不同市场的 Raw Observation。

每个 Evaluated Output 在 Protocol 中冻结 Positive/Negative Factor Orientation。Orientation 控制解释与 Economic Sorting，不修改 Dataset Raw Value。

## Evaluation Protocol 与 Evidence

Factor Evaluation Protocol 绑定一个精确 Factor Dataset Output、Target/Horizon、Market/Feature Evidence、Research Engine、Factor Orientation、Chronological/Walk-forward Window、Purge、Embargo、Lens、Neutralization、Economic Assumption、Regime 与 Research Family Trial Identity。

支持 Chronological Holdout 与 Walk-forward，不支持 Random Split。每个 Fold 冻结 Selection/Evaluation Window。只有 Research、Parameter Selection、Fitting、Normalization、Target Construction 与 Evaluation Window 记录完整且不重叠时，Evaluation Evidence State 才是 Out-of-sample。Overlapping/Unknown Report 保持可检查，但不能支持 Research Validated 或 Component Eligible Decision。

最低 Lens 要求：

- Time-Series Factor：至少一个 Temporal Lens 和一个 Economic Lens。
- Cross-Sectional Factor：至少一个 Cross-Sectional Lens 和一个 Economic Lens。
- 其他 Compatible Lens 可选，且独立于 Computation Scope。

Factor Metric Catalog 是 Formula、Direction、Range、Required Sample 与 Typed Undefined State 的版本化权威来源。Rust Core 生成 Machine-readable Catalog；GUI 和双语 Reference 从它派生。External Research Engine 可以适配 Contract，但不能静默替换同名公式。

Undefined Metric 绝不编码为 0。Insufficient Sample、Constant Value、Singular Matrix、Unavailable Target 或 Broken Requirement 生成带 Output、Lens、Fold/Window 与 Sample Count 的 Typed Unavailable Evidence。Report 可以在部分 Metric Unavailable 时 Completed，但任一 Required Metric Unavailable 都阻止 Promotion。

## Neutralization、Regime 与 Economic Lens

M11 Neutralization 是每个 Observation Time 下带 Intercept 与 Protocol-selected Nuisance Feature 的 Cross-Sectional OLS。Complete Case 用于 Fitting，但完整 Universe 与 Missingness 保留在 Evidence 中。Insufficient Sample 或 Singular Design Matrix 使该 Batch Unavailable。M11 不加入 Generic Time-Series Neutralization。

Regime Definition 选择一个 Causal Feature，只在 Frozen Selection Window 拟合 Deterministic Bucket Threshold，再原样应用到 Evaluation Observation。Report 保留 Feature、Artifact/Threshold Identity、Coverage 与 Per-bucket Result；M11 不创建 Mutable Bull/Bear Label。

标准 Economic Lens 使用 Deterministic Average Rank、Five Quantiles、Equal Weight，并报告 Top-only 与 Top-minus-Bottom Evidence。在 `t` Available 的值最早只能在下一 Eligible Bar 生效。Rebalance、Fee、Slippage、Cost 与 Long/Short Feasibility 全部冻结。这是 Diagnostic Research Evidence，不是 Strategy Component 或 ADAQ Backtest Run。

## Robustness 与 Multiple Testing

Report 按适用性保留 Coverage、Missingness、IC、Rank IC、Turnover、Decay、Stability、Subperiod、Regime、Neutralized 与 Cost-aware Result。每个 Metric 保留解释 Aggregation 所需的 Ordered Value 与 Sample Count。

Research Family 保留 Completed、Failed、Cancelled、Rejected 与 Superseded Trial。Report 保存 Raw Statistic/P-value 与 Holm-Bonferroni Family-wise Adjustment。没有 Statistic 的 Registered Trial 按 Non-significant 处理，而不是消失。Promotion 冻结完整 Applicable Family Lineage；遗漏已知 Related Trial 会阻止 Promotion。

## Promotion

Factor Promotion Policy 是 Immutable、Versioned。保守 System Template 要求显式 Minimum Coverage、Sample Size、Holm-adjusted Significance、Subperiod Sign Consistency、Cost-aware Outcome、Required Lens 与 Complete Provenance，但 M11 不硬编码 Universal IC/Return Threshold。改变 Threshold Set 会产生新 Policy Identity。

系统检查 Eligibility；User 做决定。每个 Factor Promotion Decision 针对一个精确 Named Output 且不可变：

- `Rejected` 记录引用 Evidence 未获接受。
- `Research Validated` 要求至少一个满足 Policy 的 Out-of-sample Report，并允许 M12 精确选择。
- `Component Eligible` 包含全部 Research Validated Gate，再增加 Deterministic Execution、Complete Source Provenance、ABI v2 Expressibility 与 Buildability。M14 仍负责 Build、Conformance、Equivalence、Qualification 与 Import。

后续 Decision 可引用并 Supersede 旧 Decision，但不修改旧记录。Promoted Factor Library 是 Current Decision 的 User-scoped Read-only Projection，不是复制 Evidence 或 Floating Latest-version Store。Multi-output Dataset 按 Output 独立 Promotion；Multi-output Custom Package 只有在每个 Public Output 都 Component Eligible 后才可进入 M14。

M12 只能选择带 Current Research Validated/Component Eligible Decision 的精确 Completed Factor Dataset Output，并冻结 Dataset、Report、Decision、Policy 与 Research Engine Provenance。M12 不得隐式重算或晋升 Factor。

## Attempt、Queue 与 Native API

Candidate Build、Factor Materialization、Factor Evaluation Attempt 使用 `Pending → Running → Completed | Failed | Cancelled`。Retry 创建新 Identity；Pending 跨 Restart 保留；Stale Running 变成带 Typed Interruption Evidence 的 Failed；Progress 只在完整 Work Unit 后推进，不显示虚构 ETA。

Feature 与 Factor Heavy Work 共用一个 Persistent Device-wide Research FIFO。Attempt Ownership/Visibility 保持 User-scoped。Tauri Command 只做 Canonical Request Validation、Enqueue、Cancel、Retry 与 Paginated Query；Blocking Filesystem、SQLite、Parquet、WASM、Build 与 Statistical Work 在 Worker 中执行，不在 Command Body 或 UI Thread 中执行。

Dataset、Candidate、Policy、Report、Decision Reference 执行 Deletion Lock。Unreferenced Completed User Link 可删除；Failed、Cancelled、Superseded Trial Metadata 与 Safe Diagnostic 保留。Explicit User Reset 可按既有 Reset Contract 清除该 User 的 Local Research Data。

## Factor Workspace

`/factors` 立即 Paint Shell，包含 Families、Candidates、Datasets、Evaluations、Decisions。每个 Card/Control 拥有自己的 Loading、Build、Run、Cancellation、Error、Retry State。User-scoped Read List 先渲染 Current-session Cache，再后台 Refresh，且不削弱 Validation。

Workspace 以 English (US) 与简体中文展示 Immutable Identity、Lineage、Market Context、Missingness、Target Availability、Fold Boundary、Lens Formula、Ordered Metric/Sample、Multiple-testing Adjustment、Policy Gate、Decision History、Deletion Lock 与 M12 Eligibility。它不会把历史 Evidence 标成保证，不隐藏 Failed Trial，也不提供 Automatic Promote Control。

## Resource 与 Numeric Contract

M11 保留既有 1 MiB Canonical JSON、64-output、WASM Fuel/Memory Limit，并把 Grid Search 限制为最多 256 Trial。Dataset Row、Fold、Horizon、Lens、Nuisance Column 与 Worker Ceiling 必须在 Public API 验收前通过 Benchmark 测量并冻结。Allocation/Evaluation 前执行 Checked Arithmetic 与 Limit。

相同 Engine Identity、Input、Protocol、Seed 与 Build 必须生成不受 Chunking 影响的 Bit-identical Evidence。不同 Target、Compiler 或 Platform Build 保留不同 Engine Identity。Golden Fixture 可以建立 Exact 或声明的 Tolerance-based Cross-platform Equivalence，但不同 Engine Identity 的 Report 不共享 Hash。

Performance Acceptance 使用 1,000,000-observation Time-Series Workload 与 10,000-Instrument × 252-Observation-Time Cross-Sectional Workload，证明 Bounded Memory、Cancellation、Chunk Equivalence、Determinism、Restart Recovery 与 Responsive GUI Scheduling，并记录 Canonical macOS ARM64 Baseline，不预先编造 Latency/RSS Target。

## Acceptance

Reference Journey 包含：

1. OKX Spot Time-Series Momentum：Multi-horizon、Bar Gap Restart、Temporal 与 Cost-aware Evidence。
2. China A-share Time-Series Evidence：Venue Session、Corporate Action 与 Causal Target Availability。
3. U.S. Equity Cross-Sectional Evidence：Point-in-Time Universe Membership、Neutralization、Rank IC、Turnover、Regime 与 Unknown/Reconstructed Universe Behavior。

Failure Coverage 包含 Factor ABI v1 Reset、Candidate Build Failure、Missing Input、Non-finite Output、Universe Mismatch、Singular Neutralization、Undefined Metric、Leakage、Family-lineage Omission、Policy Failure、Cancellation、Restart Recovery、Atomic Publication、User Isolation 与 Deletion Lock。

每个 Child 把每条 Acceptance Criterion 映射到 Implementation 与 Independent Evidence。Final Gate 包括 Focused Test、`cargo fmt --all --check`、`cargo test --workspace`、`cargo check --workspace`、Factor ABI/Component Conformance、Frontend Jest、`pnpm run build`、Lint、`git diff --check`、Bilingual Parity、Accessibility、Retained Build Evidence Secret Scan 与 Supported-platform CI。

## Delivery Slices

M11 已通过八个 Dependency-ordered Slice 发布：

1. [#92 — Core Contract、Factor ABI v2 与 Factor Metric Catalog](https://github.com/tonywxx/adaq/issues/92)。
2. [#90 — Declarative/Custom Candidate Execution 与 Factor Dataset Materialization](https://github.com/tonywxx/adaq/issues/90)。
3. [#89 — Target、Lens、Neutralization、Economic Diagnostic 与 Robustness Evaluation](https://github.com/tonywxx/adaq/issues/89)。
4. [#91 — Research Family、Grid Search、Multiple Testing、Promotion Policy 与 Decision](https://github.com/tonywxx/adaq/issues/91)。
5. [#95 — SQLite/Parquet Evidence、Shared Research FIFO 与 User-scoped Native API](https://github.com/tonywxx/adaq/issues/95)。
6. [#96 — Localized `/factors` Workspace](https://github.com/tonywxx/adaq/issues/96)。
7. [#94 — Three-market Fixture、Benchmark、Resource Limit 与 Hardening](https://github.com/tonywxx/adaq/issues/94)。
8. [#93 — Bilingual Cross-platform Acceptance、Manual Guide 与 Roadmap Closure](https://github.com/tonywxx/adaq/issues/93)。

依赖为 `#92 → #90 → #89 → #91 → #95 → #96`、`{#90,#89,#91,#95} → #94`、`{#92,#90,#89,#91,#95,#96,#94} → #93`。#92 是唯一初始 Executable Frontier。Planning 不产生 Production Evidence；只有每个 Slice 通过自己的 Acceptance Gate，M11 才能声明实现。
