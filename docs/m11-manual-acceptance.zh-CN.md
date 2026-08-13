# M11 人工验收

这是 M11 的权威人工复核路径。Native/Workflow Evidence Revision 为 `36fc8467b16a357ed17a642f91919471a281f77d`；最终 Acceptance-only Revision 在完成评论中记录。M11 的终点是不可变 Factor Research Evidence 与明确的 User-owned Promotion Decision，不交付 M12 Model Training、M13 Strategy、M14 Component Qualification/Import、Paper、Bot、Marketplace 或真实资金交易。

使用已提交的确定性 Fixture 作为权威路径。不要在评论、Commit、Screenshot、Log 或 Export 中写入 Credential、Authorization Header、Token、私有路径、私有市场数据或未脱敏的 Build Diagnostic。

<!-- m11-acceptance:scope -->
## 1. 范围与前置条件

| 精确操作 | 预期结果 | 失败时记录 |
| --- | --- | --- |
| 在 Repository Root 运行 `node --version`。 | 使用 Node.js 24 或更新版本。 | 完整输出与安装方式。 |
| 运行 `pnpm --version`。 | 使用与 `package.json` 的 `packageManager` 一致的 pnpm 11.20.0。 | 完整输出与工具版本。 |
| 运行 `pnpm install --frozen-lockfile`。 | 依赖与 `pnpm-lock.yaml` 一致。 | 完整输出与两个工具版本。 |
| 运行 `rustup show` 与 `cargo component --version`。 | Stable Rust 与 Component Toolchain 可用；不需要机器本地 WIT 路径。 | 完整输出与已安装 Target。 |
| 新建本地 User 并打开 **Settings → General**。 | 可选择 English (US) 与 简体中文；创建前看不到 M11 Evidence。 | 脱敏 User ID、Locale、Platform 与 Screenshot。 |
| macOS 使用 `shasum -a 256 <path>`，Windows PowerShell 使用 `Get-FileHash -Algorithm SHA256 <path>`，Linux 使用 `sha256sum <path>`。 | Exported Fixture 的 Digest 可重复，且不暴露路径或内容。 | 脱敏输出与 Platform。 |

Supported-platform Substitution 为 macOS ARM64（`aarch64-apple-darwin`）、Windows x86_64 或 Linux x86_64。不需要 Provider Credential；Committed Fixture 与本地 Evidence Store 是本验收路径的权威来源。

<!-- m11-acceptance:contracts -->
## 2. Contract、ABI 与 Evidence Identity

| 精确操作 | 预期结果 | 失败时记录 |
| --- | --- | --- |
| 运行 `cd src-tauri && cargo test -p adaq-factor-research --lib -- --nocapture`。 | Candidate、Materialization、Evaluation、Family、Policy、Decision、Identity、Missingness 与 Limit Contract 通过。 | Revision、失败 Test 与完整输出。 |
| 运行 `cd src-tauri && cargo test -p adaq-component-sdk -- --nocapture` 与 `cargo test -p adaq-component-tooling -- --nocapture`。 | SDK Binding、Manifest Validation、Package Trust Boundary、ABI v2 World 与 Conformance 通过。 | Revision、失败 Test 与完整输出。 |
| 通过现有 Component Path 构建 `factor`、`multi-output-factor`、`cross-sectional-factor`、`repeated-factor-strategy` 与 `external-strategy` Fixture。 | Fixture Package 构建不访问网络，保留 Scope/Output Identity，并继续经过 Host Verification。 | Fixture、Command、Target 与脱敏 Diagnostic。 |
| 检查 `src-tauri/wit/factor/adaq-factor.wit`、`src-tauri/crates/adaq-component-sdk/wit/factor/adaq-factor.wit`、`docs/reference/component-manifest.schema.json` 与生成的 Factor Catalog。 | ABI v2 使用 Host-resolved Feature Batch、Time-Series 或 Cross-Sectional Scope、Typed Missingness、Ordered Identity，且没有 v1 兼容层。 | 文件、过期说明与 Revision。 |
| 运行 `cd src-tauri && sh crates/adaq-factor-research/scripts/check_generated.sh`。 | Metric Catalog 与 Reference Artifact 可无 Diff 重新生成。 | Revision、Generated File 与完整输出。 |
| 检查 `CONTEXT.md`、ADR 0060–0062、`docs/m11-factor-research.zh-CN.md`、`docs/v1-roadmap.zh-CN.md`、SDK/Tooling Guide 与 GUI Copy。 | Ownership、Immutable Evidence、Native Queue、ABI v2、M12 Boundary 与 `/factors` Entry 在两种 Locale 中一致。 | 文件、段落与矛盾说明。 |

Stored Factor ABI v1 Evidence 只能走 `reset-required`：不提供 Migration、Dual Reader 或 Automatic Deletion。Presentation Name 与 Tag 是 User-scoped Metadata，不改变 Semantic Hash。

<!-- m11-acceptance:okx-journey -->
## 3. OKX Spot Time-Series Journey

| 精确操作 | 预期结果 | 失败时记录 |
| --- | --- | --- |
| 选择绑定 OKX Spot Snapshot 的 Completed M10 Feature Dataset 并打开 `/factors`。 | Factor Lab 在排队工作前展示精确 User、Feature Dataset、Snapshot、Range 与 Market Context。 | ID、Context、Locale 与 Screenshot。 |
| 发布带有序 Feature Slot 与确定性参数 Revision 的 Declarative Candidate。 | Positive Revision 有 Canonical JSON 与 Content Hash；Presentation 修改不改变 Hash。 | Candidate Revision/Hash 与 Screenshot。 |
| 为精确 Dataset 物化 Candidate，检查宽表 Factor Dataset。 | Row Key 为 `(Instrument ID, Observation Time)`；保留 `Available At`、Warmup、Bar Gap、Missingness 与 Candidate/Feature/Engine Provenance。 | Attempt/Dataset ID 与缺失 Manifest 字段。 |
| 冻结带正 Horizon、Purge、Embargo、Temporal 与 Economic Lens 的 Chronological Holdout 与 Walk-forward Protocol。 | Report 使用 `close[t+h] / close[t] - 1`，不使用 Random Split，并保留 Fold Identity 与 Evaluation Evidence State。 | Protocol/Report ID、Window 与 State。 |
| 在 Research Family 中注册 Candidate，运行 bounded Grid 并检查全部 Trial。 | Completed、Failed、Cancelled、Rejected、Superseded Trial 都保留；Holm Correction 使用完整 Registered Family。 | Family/Trial ID、Status 列表与 Correction Population。 |
| 冻结满足 Policy 的 Out-of-sample Report，并为一个精确 Output 记录 User Decision。 | `Research Validated` 是明确、不可变、带引用的 Decision；只有 Dataset、Report、Policy、Decision、Engine Provenance 齐全时才出现精确 M12 Eligibility。 | Decision Gate、引用 Hash 与 Eligibility Response。 |
| 运行 `cd src-tauri && cargo test -p adaq-factor-research --test reference_fixtures`。 | 已提交的 OKX Vector 通过：Multi-horizon Momentum、Warmup、Bar Gap Restart、Temporal、Decay/Stability 与 Cost Evidence。 | Revision、Test 与 Vector Mismatch。 |

<!-- m11-acceptance:a-share-journey -->
## 4. 中国 A 股 Journey

| 精确操作 | 预期结果 | 失败时记录 |
| --- | --- | --- |
| 使用带 Venue-local Asia/Shanghai Session 与 Scheduled Closure 的 A-share Fixture。 | Session Boundary 与 Closure 是 Calendar Evidence，不被推断为 Bar Gap。 | Venue、Timestamp、Calendar State 与 Vector Digest。 |
| 跨上午、午间休市与下午交易时段物化 Time-Series Candidate。 | State 跨 Scheduled Closure 延续，只在真实 Bar Gap 重置；Warmup 与 Missing Input 保持 Typed。 | Segment ID、Gap/Closure 分类与 Dataset Row。 |
| 检查 Target Horizon 附近的 Verified Corporate Action。 | Corporate Action Evidence 绑定 Instrument 与 Effective Evidence Time；不可用 Close/Target 保留，不静默调整。 | Action Evidence ID、Price Basis、Available At 与 Typed Reason。 |
| 使用 Causal Holdout/Walk-forward Fold 与 Economic Lens 评估。 | Target Availability、Purge/Embargo、Fold State、Fee、Slippage、Rebalance 与 Cost-aware Result 均明确。 | Report ID、Fold Evidence、Assumption 与 Metric State。 |
| 运行 `cd src-tauri && cargo test -p adaq-factor-research --test reference_fixtures`。 | China A-share Reference Vector 通过：Session、Closure、Corporate Action、Target Availability、Time-Series Evaluation 与 Typed Unavailable Path。 | Revision、失败 Journey 与完整输出。 |

<!-- m11-acceptance:us-equity-journey -->
## 5. 美国股票 Cross-Sectional Journey

| 精确操作 | 预期结果 | 失败时记录 |
| --- | --- | --- |
| 将 Cross-Sectional Candidate 绑定到一个 Observation Time 的完整 Point-in-Time Universe。 | 保留完整 Membership 与确定性 Order；Venue、Asset Class、Snapshot、Currency 或 Universe Context 混用会被拒绝。 | Universe ID、有序 Member、Context 与 Typed Error。 |
| 使用带不可用 Member 的输入物化，再检查 Dataset。 | Member 保留在 Batch 中并带 Typed Missingness；Host 不丢弃也不伪造值。 | Member ID、Reason Code、Row Count 与 Manifest。 |
| 使用确定性 Average Tie 与五个 Quantile Group 评估 Cross-Sectional 与 Economic Lens。 | IC、Rank IC、Turnover、Top-only、Top-minus-Bottom、Fee/Slippage 与 Rebalance Evidence 有序且可复现。 | Report ID、Metric Sample 与 Vector Digest。 |
| 加入显式 Nuisance Feature 并检查 Neutralized Result。 | 每个 Observation Time 带 Intercept 在 Complete Case 上执行 OLS；完整 Universe/Missingness 仍在 Evidence；Insufficient/Singular 为 Typed Unavailable。 | Nuisance ID、Batch、Sample Count 与 Reason。 |
| 加入 Causal Regime Feature。 | Threshold 只在 Selection Window Fitting，并原样用于 Evaluation Observation；保留 Bucket Evidence。 | Threshold Identity、Selection Range 与 Per-bucket Output。 |
| 冻结 Policy Gate，并为 Multi-output Dataset 的一个 Named Output 记录 Decision。 | 每个 Output 独立 Decision；Overlapping/Unknown 不能产生 Positive Promotion；M12 Eligibility 要求精确 Evidence Chain。 | Output Name、Gate、Decision 与 Eligibility。 |
| 运行 `cd src-tauri && cargo test -p adaq-factor-research --test reference_fixtures` 与 `cargo test -p adaq-factor-research --test metric_golden`。 | U.S. Cross-Sectional Membership/Order、Missingness、Tie、IC/Rank IC、Turnover、Neutralization、Regime、Cost 与 Literal Golden Metric 通过。 | Revision、Test 与 Mismatch。 |

<!-- m11-acceptance:candidate-paths -->
## 6. Declarative、Custom 与 Trust Boundary Path

| 精确操作 | 预期结果 | 失败时记录 |
| --- | --- | --- |
| 编辑 Declarative Draft，发布两个 Revision，只修改 Presentation Metadata。 | 只有 Immutable Semantic Revision/Hash 改变；Name、Description、Tag 留在 Hash 外。 | Before/After Hash 与 User Scope。 |
| 使用固定 Offline/Locked Project Path 构建 Private Custom Candidate。 | 冻结 Source Hash、SDK、ABI、Compiler/Toolchain、Target、Command、Environment、Resource Policy、Bounded Diagnostic 与 Package Hash。 | Attempt ID 与脱敏 Diagnostic。 |
| 尝试 Network Access、Custom Build Script、Ambient File Read、Invalid Scope/World、Non-finite Output、错误 Row/Order 或 Output Mismatch。 | Trust Boundary 在 Evidence Publication 前拒绝并保留安全 Typed Failure。 | Typed Error、Attempt ID 与无 Published Dataset 证明。 |
| 比较语义相同的 Declarative 与 Private Custom Candidate Fixture Path。 | 声明语义相同则 Factor Dataset/Evaluation Vector 等价；不同 Engine Identity 不合并 Report Hash。 | Candidate/Engine Hash 与 Vector Diff。 |
| 检查 Multi-output Candidate 与 Component Library。 | Output 独立 Decision；Custom Package 在 M14 Qualification、Conformance、Import 前保持 Private、Non-imported。 | Output Decision、Package State 与 Screenshot。 |

<!-- m11-acceptance:failure-recovery -->
## 7. Failure、Recovery 与 Retention Path

使用 Committed Fixture 或 Typed Test Setup 执行每行。只有 Typed Reason 与安全 Retained Evidence 可见、且没有 Partial Evidence 可消费时，Failure 才算通过。

| Failure Path | 预期 Evidence |
| --- | --- |
| ABI v1 Package/Evidence | `reset-required`、明确 Device-level Reset；不 Migration、Dual Read 或 Automatic Deletion。 |
| Candidate Build Failure、Missing Input、Bar Gap、Non-finite Output | Failed Attempt 或 Typed Unavailable Row；Bounded Redacted Diagnostic；无 Partial Dataset。 |
| Universe Mismatch、Missing Member、Singular Neutralization、Undefined Required Metric | Complete Batch/Report 仍可检查并带 Typed Reason；阻止 Positive Promotion。 |
| Target Leakage、Overlapping/Unknown Fold 或 Omitted Related Trial | Validation/Promotion 被拒绝；Lineage 与 Window Evidence 保留。 |
| Policy Rejection 或显式 User Rejection | Decision Immutable 且 Output-specific；无 Automatic Promotion 或 Floating Latest Pointer。 |
| Cancellation、Crash/Restart、Queue Contention | Pending 保留，Stale Running 变 Typed Failed，Cancellation 不发布 Partial Evidence，单一 FIFO 仍按 User 隔离。 |
| Atomic Publication 或 Corrupted Parquet/Report Payload | Staging 被丢弃或隔离；Hash/Schema 在 Display 前拒绝 Evidence。 |
| User Isolation、Deletion Lock、Final-reference Cleanup、Explicit Reset | 其他 User 不可见；Reference 阻止删除；仅最后一个 User Reference 删除共享 Bytes；Reset 必须显式。 |

Focused Coverage 保留在 `src-tauri/src/factor_research/mod.rs`、`src-tauri/crates/adaq-factor-research/src/{abi,candidate,evaluation,promotion,research}.rs` 与 Factor Integration Suite 中。

<!-- m11-acceptance:factor-gui -->
## 8. `/factors` Workspace 与 Accessibility

| 精确操作 | 预期结果 | 失败时记录 |
| --- | --- | --- |
| 使用 Signed-in Fixture User 直接导航到 `/factors`。 | Native Read 完成前 Shell 已立即 Paint；Families、Candidates、Datasets、Evaluations、Decisions 可见。 | Route Timing、Screenshot 与 Console Error。 |
| 进入 Slow List，切换 Tab，离开后重新进入。 | Current-session User-scoped Cache 先渲染，后台 Revalidation 更新；Stale Response 不覆盖新的 User/Page Data。 | User/Resource/Page、Request Order 与 Screenshot。 |
| 启动 Candidate Build、Grid Registration、Materialization、Evaluation、Cancellation、Retry 或 Deletion。 | Busy/Progress/Error/Retry 状态属于拥有它的 Card/Control；整页可用，没有 Automatic Promote Action。 | Attempt ID、Control State、`aria-busy` 与 Screenshot。 |
| 在 en-US 与 zh-CN 中仅用键盘经过 Tab、Form、Table、Pagination、Lineage、Metric Detail 与 Decision Gate。 | Label/Description、Focus Order、Tab/Table Semantics、Status Announcement、Narrow-window Horizontal Access、Localized Formatting 可用；Canonical ID/Code 保持精确。 | Platform、Scale、Focused Element、Accessibility Tree 与 Screenshot。 |
| 检查 Failed Trial、Typed Unavailable Metric、Raw/Holm Statistic、Provenance、Deletion Lock 与 M12 Eligibility。 | Workspace 展示 Evidence Boundary，不隐藏 Failure，不把历史结果显示为保证。 | 缺失字段、Route 与脱敏 Evidence。 |

Source-level 与 Jest Contract 位于 `src/features/factors/factors-page.test.ts`、`factor-adapter.test.ts`、`factor-data.test.ts`、`src/loading-boundaries.test.ts`、`src/router.test.ts` 与 `src/lib/i18n.test.ts`。OS Assistive Technology 差异必须按 Supported Platform 记录，不能用本地 Browser Pass 替代。

<!-- m11-acceptance:boundary -->
## 9. M11 Boundary 与 Deferred Capability Check

检查 Route、Command、Doc 与 GUI Copy，确认以下 Negative Requirement：

- M11 不暴露 Qlib/Python Runner、Notebook Execution、Model Training、Strategy Construction、Component Equivalence/Import、Paper、Bot、Live、Marketplace、Automatic Promotion、Adaptive Optimization 或 Cross-market Raw-sample Pooling。
- M11 只使用 ADAQ Native Research Engine，并消费 Completed M10 Feature Dataset。
- Economic Diagnostic 不是 Strategy Backtest；Component Eligible 不是 M14 Qualification/Import；Research Validated 不是 Profitability Guarantee。
- 不增加 Mutable `latest` Evidence Reference、Universal IC/Return/Profitability Threshold、Hidden Fitting、Hidden Imputation 或 Script Runtime。
- M12 只能选择带 Current Positive Decision、Frozen Report、Policy、Promotion Protocol 与 Engine Provenance 的精确 Completed Factor Dataset Output。

<!-- m11-acceptance:performance-baselines -->
## 10. Performance Baseline 与 Resource Ceiling

| 精确操作 | 预期结果 | 失败时记录 |
| --- | --- | --- |
| 运行 `cd src-tauri && cargo test -p adaq-factor-research --test benchmarks -- --test-threads=1`。 | Bounded Workload 检查 Candidate Execution 与 Resource Accounting。 | Revision、Workload、完整输出与 Platform。 |
| 在 macOS ARM64 运行 `cd src-tauri && cargo test -p adaq-factor-research --release --test benchmarks -- --ignored --test-threads=1`。 | Canonical 1,000,000-Bar Time-Series 与 10,000 × 252 Cross-Sectional Workload 完成 Cancellation/Chunk/Recovery 检查。 | Runtime、High-water RSS、Artifact Hash 与 Platform。 |
| 对比 `src-tauri/crates/adaq-factor-research/fixtures/factor-benchmark-baseline.json`。 | Canonical macOS ARM64 Baseline：Time-Series 723 ms、Cross-Sectional 1,548 ms、High-water RSS 29,884,416 bytes 与两个 Candidate Package Hash。它们是记录值，不是 SLA。 | Baseline Digest、Measured Value 与 Platform Difference。 |
| 在 Allocation 前检查 Public Ceiling。 | 2,520,000 Dataset Row、32 Fold、16 Horizon、5 Lens、16 Nuisance Feature 与一个 Device-wide Worker 通过 Checked Arithmetic。 | Requested Limit、Typed Rejection 与 Allocation Proof。 |

<!-- m11-acceptance:regressions -->
## 11. Regression 与 Boundary Gate

| 精确操作 | 预期结果 |
| --- | --- |
| 运行 `cd src-tauri && cargo test --workspace`。 | M1–M10 行为与所有 M11 Native Suite 通过；明确记录 Ignored Benchmark Test。 |
| 运行 `pnpm exec jest --watchman=false --runInBand`。 | Frontend Route、Loading Boundary、Locale、Factor Workspace Contract 与此前 Milestone Suite 通过。 |
| 检查 README、Workflow Navigation、SDK/Tooling Doc、Generated Schema、ADR 0060–0062 与两份 M11 Architecture Guide。 | Link 可用、Status 为 Accepted，M11 Path 没有过期 Planned/v1 ABI 说明。 |
| 对最终 Diff 与 Generated Evidence 运行 Retained-diagnostic Scan。 | 不保留 Credential、Token、绝对私有路径或不安全 Diagnostic。 |

<!-- m11-acceptance:automated-gates -->
## 12. Automated Gate

在最终 Acceptance Revision 运行：

```sh
(
  cd src-tauri
  cargo fmt --all --check
  cargo test --workspace
  cargo check --workspace
  cargo test -p adaq-factor-research --test reference_fixtures
  cargo test -p adaq-factor-research --test metric_golden
  cargo test -p adaq-factor-research --test benchmarks -- --test-threads=1
  cargo test -p adaq-factor-research --release --test benchmarks -- --ignored --test-threads=1
  sh crates/adaq-factor-research/scripts/check_generated.sh
)
pnpm exec jest --watchman=false --runInBand
pnpm run build
pnpm run lint
git diff --check
```

预期所有 Command 返回 0。分别记录精确 Count 与既有 Warning；Local Pass 不能替代 Required Platform Matrix。本 Checkout 没有配置 Secret Scanner，因此除非项目新增 Scanner，否则 Retained-diagnostic Scan 由最终 Diff 人工检查完成。

<!-- m11-acceptance:platform-evidence -->
## 13. Supported-platform Evidence

可手工 Dispatch 的 [Indicator engine acceptance workflow](../.github/workflows/indicator-engine.yml) 构建并验证 Factor Fixture、检查 Generated Factor Reference、运行完整 Rust Workspace，并在 macOS ARM64 运行 Canonical Factor Benchmark。Reviewed Native/Workflow Revision 为 `36fc8467b16a357ed17a642f91919471a281f77d`。

| Workflow Evidence | Revision | Jobs | Result |
| --- | --- | --- | --- |
| [Indicator engine acceptance run 31664792735](https://github.com/tonywxx/adaq/actions/runs/31664792735) | `36fc8467b16a357ed17a642f91919471a281f77d` | [macOS ARM64](https://github.com/tonywxx/adaq/actions/runs/31664792735/job/94336905508)、[Windows x86_64](https://github.com/tonywxx/adaq/actions/runs/31664792735/job/94336905609)、[Linux x86_64](https://github.com/tonywxx/adaq/actions/runs/31664792735/job/94336905553) | Success（三个平台全部通过） |

重复 Dispatch 使用 `gh workflow run "Indicator engine acceptance" --ref <reviewed-ref>`，然后记录 Workflow URL、SHA、Job URL、Conclusion 与失败 Log 摘要。Windows 使用 PowerShell Hash Command 并显式释放 Handle；Linux 安装 Workflow 的 Tauri Prerequisite；Ignored Large Benchmark 只在 macOS ARM64 运行。

<!-- m11-acceptance:acceptance-matrix -->
## 14. 最终 Acceptance Matrix

Matrix 是 Evidence，不等于 Issue State。每行识别 Implementation Boundary、Focused Evidence、Broad/Manual Section 与 Remaining Limitation。Child Issue Comment 是独立 Evidence Record；本 Matrix 关闭 Cross-slice 关系。

### Parent #88 Criteria

| ID | Criterion 与 Evidence 映射 | Implementation / Focused Evidence | Broad/Manual Evidence | Remaining Limitation |
| --- | --- | --- | --- | --- |
| P1 | 八个 Dependency-ordered Slice 与独立 Evidence。 | #92、#90、#89、#91、#95、#96、#94 Comment 与下方 Slice Row。 | Sections 2–13。 | 所有 Child Comment 存在后无。 |
| P2 | `adaq-factor-research` Ownership 覆盖 Materialization、Catalog、Evaluation、Family、Policy、Report、Decision、Library，不吸收 Feature/Tooling Semantics。 | Crate Module 与 #92/#95 Focused Test。 | Sections 2、6–7、9。 | 无。 |
| P3 | ABI v2 具 Scope-specific Batch、Identity、Typed Missingness，并对不兼容 v1 Evidence 显式 Reset。 | SDK/Tooling Test、WIT、`reset-required`、#92。 | Sections 2、7、9、13。 | v1 Evidence 按设计只能 Reset。 |
| P4 | Declarative 与 Private Custom Candidate 在 Evaluation 前物化 Immutable Dataset。 | #90 Materialization/Parity Test。 | Sections 3、6。 | Custom Package 在 M14 前保持 Private。 |
| P5 | Causal Holdout/Walk-forward Report 保留 Target、Lens、Neutralization、Robustness、Cost 与 Evidence State。 | #89 Evaluation/Golden Test。 | Sections 3–5、10。 | 无 Universal Profitability Threshold。 |
| P6 | Research Family 保留所有 Trial 与 Lineage；Omission 不能取得 Promotion。 | #91 Registry、Holm、Lineage、Omission Test。 | Sections 3、7、14。 | 无。 |
| P7 | 只有 Immutable User Decision 能产生 Positive Output State。 | #91 Policy/Decision/Library/M12 Eligibility Test。 | Sections 3、5、7、9。 | M14 Qualification 延后。 |
| P8 | M12 只能选择带 Frozen Evidence Provenance 的精确 Completed Dataset Output。 | `m12_eligibility` 与 #91/#96 Evidence。 | Sections 3、5、9。 | M12 本身不在范围内。 |
| P9 | SQLite/Parquet Atomicity、Recovery、User Isolation、Deletion Lock 与 Shared FIFO 通过。 | #95 Native Test 与 Retained Diagnostic。 | Sections 7–8、11–13。 | OS File Handle 按 Platform 记录。 |
| P10 | `/factors` Localized、Accessible、Immediate-paint、Control-owned、Cached 且面向 Evidence。 | #96 Frontend Test 与 Source Contract。 | Section 8。 | OS Assistive Technology 仍按 Platform 有差异。 |
| P11 | 三市场 Journey 与声明的 Failure Path 通过。 | #94 Reference Fixture/Golden/Benchmark 与 #89/#90/#91/#95 Test。 | Sections 3–7、10、13。 | Fixture 是确定性 Evidence，不是 Live Provider Coverage。 |
| P12 | M11 不包含 Qlib/Python Runner、Training、Strategy、Component Import、Paper/Bot/Live/Marketplace、Auto-promotion、Adaptive Optimization、Pooling。 | Boundary Doc、Route/Source Test、Native Command Ownership。 | Section 9。 | M12+ 是未来工作。 |
| P13 | 每个 Criterion 映射 Implementation、Focused、Broad、Manual、Revision 与 Limitation Evidence。 | 本 Matrix 与全部 Child Comment。 | Sections 1–13 与 Issue Comment。 | 执行人工复核者需保留 OS Observation。 |
| P14 | Format、Rust、Frontend、Generated-reference、Lint、Diff、Secret、Supported-platform Gate 通过。 | Section 12 Command 与 Section 13 Run。 | Sections 10–13。 | Checkout 没有配置 Secret Scanner。 |
| P15 | #93 与 Parent #88 关闭前先发布 English Completion Evidence。 | Completion Comment 链接本 Guide、Final SHA、Command、Count 与 CI。 | Section 15 与 Issue History。 | Close 是独立 GitHub State Transition。 |

### M11.1–M11.7 Slice Matrix

| Slice | 每个 Acceptance Criterion 映射 | Focused Implementation Evidence | Final/Manual Evidence | Limitation |
| --- | --- | --- | --- | --- |
| #92 / M11.1 | 1 Workspace Crate；2 Canonical Contract；3 ABI v2 Scope/Slot/Output；4 Time-Series Batch；5 Cross-Sectional Batch；6 Host Validation；7 `reset-required`；8 Metric Catalog；9 JSON/Output/Grid/WASM Limit；10 Chunk Identity；11 SDK/WIT/Schema/Conformance；12 Canonical/Invalid/Conformance Test；13 Rust/Check/Workspace Gate。 | `adaq-factor-research` Contract/ABI/Catalog、SDK/Tooling、WIT、Manifest Schema、Fixture。 | Sections 2、6–7、9、12–13。 | Reset 按设计显式执行。 |
| #90 / M11.2 | 1 Declarative Revision；2 Feature Semantics/no Expression Runtime；3 Presentation Isolation；4 Custom Build Provenance；5 Controlled Private Build；6 Protocol Binding；7 Scope/Missingness Materialization；8 Immutable Wide Dataset；9 Coalesce/Reuse/Retry；10 Atomic Staging；11 Dataset-only Evaluation；12 Isolation/Lock/Deletion；13 Lifecycle Test；14 Rust/Check/Workspace Gate。 | Candidate/Materialization Module、Fixture Parity、Lifecycle/Trust Boundary Test。 | Sections 3、6–8、12。 | Evaluation 与 GUI 属于下游。 |
| #89 / M11.3 | 1 Future Close Return；2 Causal Availability/Typed Target Failure；3 Comparable Context；4 Orientation；5 Holdout/Walk-forward；6 Evidence State；7 Required Lens；8 OLS Neutralization；9 Regime；10 Economic Lens；11 Metric Evidence；12 Typed Catalog Undefined；13 Engine/Report Identity；14 Numeric Focused Test；15 Rust/Reference/Check Gate。 | `evaluation.rs`、Metric Catalog、Golden/Reference Vector。 | Sections 3–5、7、10、12。 | Binary/Custom Target 不在范围内。 |
| #91 / M11.4 | 1 Family/Trial Registration；2 Retained Status；3 Lineage；4 Bounded Grid；5 Raw/p/Holm；6 Complete Lineage Gate；7 Versioned Policy；8 No Universal Threshold/Rule Engine；9 Explicit Output Decision；10 OOS Positive Gate；11 Component Eligible Boundary；12 Append-only Supersession/Library；13 Per-output Multi-output；14 M12 Eligibility；15 Focused Test；16 Rust/Check/Workspace Gate。 | `research.rs`、`promotion.rs`、Registry/Lineage/Policy/Decision Test。 | Sections 3、5、7、9、14。 | M14 Qualification/Import 延后。 |
| #95 / M11.5 | 1 Typed SQLite Metadata；2 Parquet Payload；3 Shared FIFO；4 Attempt Lifecycle；5 Restart/Cancellation Atomicity；6 Complete-unit Progress；7 Non-blocking Tauri Boundary；8 Bounded User API；9 Reference Lock；10 Retained Failure；11 Explicit Reset；12 Redacted Diagnostic；13 Checked Public Ceiling；14 Queue/Storage/API Test；15 Rust/Frontend/Diff/Secret Gate。 | `src-tauri/src/factor_research/mod.rs` 与 Native Lifecycle/Queue Test。 | Sections 7–8、10–13。 | Native UI Inspection 按 Platform 记录。 |
| #96 / M11.6 | 1 Route/Navigation/Workflow Metadata；2 Immediate Paint/Control Loading；3 Cache/Revalidation/Stale Guard；4 Family Lineage；5 Candidate UI；6 Dataset UI；7 Evaluation UI；8 Metric Result UI；9 Decision UI；10 No Guarantee/Auto-promotion；11 Bilingual Copy；12 Accessibility/Narrow Window；13 User Isolation；14 Frontend Focused Test；15 Jest/Build/Lint/Rust/Diff/Manual Gate。 | Router、i18n、Factor Adapter/Page/Data、Loading/Route Test。 | Section 8 与 Sections 11–13。 | OS Assistive Technology 不是跨平台完全相同。 |
| #94 / M11.7 | 1 OKX Vector；2 A-share Vector；3 U.S. Vector；4 Declarative/Custom Equivalence；5 Independent Golden Catalog；6 Large Workload；7 macOS Baseline/Ceiling；8 Bounded/Chunk/Cancel/Recovery；9 Engine Identity/Tolerance；10 Bounded Property/Failure；11 Diagnostic Leakage；12 Generated Reference；13 Focused/Broad/Frontend/Diff Gate；14 Exact Reviewed-SHA CI。 | `reference_fixtures.rs`、`metric_golden.rs`、`benchmarks.rs`、Fixture、Workflow。 | Sections 3–7、10、12–13。 | Baseline 是记录值；CI 不能替代 GUI Inspection。 |

<!-- m11-acceptance:acceptance-record -->
## 15. Acceptance Record 与 Cleanup

记录 Final Acceptance Commit、Native/Workflow Revision、OS/Architecture/Display Scale、Node/pnpm/Rust Version、Focused Count、Full-gate Result、Fixture/Vector Digest、脱敏 User ID、Route/Accessibility Observation 与 CI URL/SHA/Conclusion。当前 Supported-platform Evidence 为 [31664792735](https://github.com/tonywxx/adaq/actions/runs/31664792735)，Revision 为 `36fc8467b16a357ed17a642f91919471a281f77d`，macOS ARM64、Windows x86_64、Linux x86_64 Job 全部成功。Acceptance-only 文件不修改 Native/Workflow Input；Frontend/Docs Gate 在 Final Acceptance Commit 记录。

验收后只删除本次运行创建的 Disposable Profile、Temporary Build Directory、Generated Package Output 与 Test Database。不要删除 Committed Fixture 或 Finalized Evidence。Windows 必须先释放 SQLite/File Handle，再清理并记录 Platform Result。

只有当上方所有适用 Row 通过、Child Comment 与 Final Matrix 已用英文发布、Local Gate 全部成功、当前 Native/Workflow Evidence 已记录且没有违反 M11 Boundary 时，M11 才算接受。先关闭 #93，再关闭 Parent #88；不要关闭任何 M12+ Issue。
