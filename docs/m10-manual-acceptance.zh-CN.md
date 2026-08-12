# M10 人工验收

这是 M10 的规范人工复核路径。规范本地运行环境为 macOS ARM64；Windows x86_64 与 Linux x86_64 的命令替换记录在下方。逐行执行并在失败时保留要求的证据。M10 的边界止于 finalized immutable Feature Datasets 与等价的 Feature Engine；它不交付 Factor research（M11）、Model training、Strategies、Paper、Bots 或任何更后续的内容。

不得把凭证、授权 Header、OTP、Token、私有路径或私有市场数据放入 issue 评论、commit、截图、日志、导出文件或本记录。可选真实 Provider 检查只能使用维护者凭证，并且只能在 **Settings → Connections** 中输入；已提交的 Fixtures 与本地 Mock Server 才是规范验收路径。

<!-- m10-acceptance:scope -->
## 1. 范围与前置条件

| 精确操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| 在仓库根目录运行 `node --version`。 | 使用 Node.js 24 或更新版本，与 release baseline 一致；本次复核运行使用 Node v26.7.0。 | 完整输出和安装方式。 |
| 运行 `pnpm --version`。 | 可用 pnpm 11.20.0，与 `package.json` 的 `packageManager` 一致。 | 完整输出和安装方式。 |
| 运行 `pnpm install --frozen-lockfile`。 | 依赖与 `pnpm-lock.yaml` 一致。 | 完整输出和两个工具版本。 |
| 运行 `rustup toolchain install stable` 和 `rustup show`。 | stable Rust toolchain 对 feature engine workspace 可用。 | 完整输出和已安装 target 列表。 |
| 在 version control 之外提供 Supabase 变量后运行 `pnpm tauri dev`。 | Desktop shell 打开，且不暴露配置值。 | 截图和脱敏错误。 |
| 打开新的 device profile 并进入 **Settings → General**。 | 仅显示 System、English (US) 和 简体中文 三种 locale 选择；尚无任何 Feature evidence。 | 截图、平台和 locale 状态。 |

macOS 使用 `shasum -a 256 <path>`，Windows PowerShell 使用 `Get-FileHash -Algorithm SHA256 <path>`，Linux 使用 `sha256sum <path>`。Native file picker、data-folder 路径、显示缩放和 secret-store 提示属于平台差异。

<!-- m10-acceptance:definitions -->
## 2. Feature Definition 生命周期

| 精确操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| 运行 `cd src-tauri && cargo test -p adaq features`。 | `src-tauri/src/features/tests.rs` 中 Definition 生命周期、preview、fitting、materialization、runner、cancellation、restart、deletion-lock 和 reset 测试通过。 | Revision、完整输出和失败测试。 |
| 打开 `/features`，在 ordered node editor 中创建 Draft Definition。 | Draft 创建可完全用键盘操作：node 添加/删除/重排、parameter 编辑和 output 选择都不依赖 pointer。 | Route、focused control、截图和 accessibility tree 文本。 |
| 对存在 typed 缺陷（错误 scope、cycle 或 untyped signal provenance）的 draft 选择 **Validate**。 | Validation 以 typed error 失败并指明缺陷；不创建任何 evidence identity。 | Typed error、draft 状态和截图。 |
| 发布一个有效 draft，再发布同一 Definition family 的修改 revision。 | 发布构成 immutable revision chain：稳定 `definitionId` 不变，JCS SHA-256 `definitionHash` 变化，且 revision 必须递增。 | Definition ID、前后 revisions、hashes 和截图。 |
| 对已发布 Definition 运行 bounded Preview。 | Preview 不做拟合、不创建 evidence identity，只产生瞬时有界输出。 | Preview result、Dataset/Attempt ID 缺失和截图。 |
| 在 **English (US)** 与 **简体中文** 之间切换 `/features`。 | 每个 Definition control、state 和 error 都在 en-US 与 zh-CN 下本地化。 | Locale、missing key/label 和截图。 |
| 运行 `pnpm exec jest --watchman=false --runInBand src/features/features/features-data.test.ts`。 | Frontend Definition/adapter data contract 测试通过。 | Revision、suite/test 和完整输出。 |

<!-- m10-acceptance:fitting -->
## 3. Fitting Protocols 与 Artifacts

| 精确操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| 在 `/features` 中对一个 Completed Dataset 启动 Fitting Protocol。 | Fitting Attempt 启动并发布 immutable Fitted Transformation Artifact；重复请求合并到同一 evidence。 | Attempt ID、Artifact ID、protocol identity 和截图。 |
| 运行 `cd src-tauri && cargo test -p adaq-feature-engine --test fitting`。 | Engine fitting 测试通过，包括 `standardization_uses_population_variance_and_excludes_future_available_samples` 和 `per_instrument_parameters_are_exact_and_walk_forward_rejects_future_artifacts`。 | Revision、完整输出和失败测试。 |
| 运行 `cd src-tauri && cargo test -p adaq features fitting_publishes_an_artifact_and_coalesces_duplicates`。 | App 层生命周期发布 Artifact 并合并重复 fitting 请求。 | Revision、完整输出和失败测试。 |
| 在后续 fold 的 Artifact 存在后检查一个 walk-forward fold。 | Fold isolation 成立：fold 绝不观察 future Artifact，per-instrument parameters 保持精确。 | Fold ID、Artifact IDs、parameter evidence 和截图。 |
| 提交样本不足的 Fitting Protocol。 | Attempt 以 typed insufficient-sample error 失败；不发布 Artifact。 | Attempt ID、typed error 和 Artifact ID 缺失。 |
| 重试一个失败的 Fitting Attempt。 | Retry 保留原始 source evidence 并产生新的 Attempt identity。 | 新旧 Attempt IDs、保留的 source evidence 和错误。 |
| 取消一个 running Fitting Attempt。 | Cancellation 在 terminal evidence 写入前到达 running attempt。 | Attempt ID、cancellation 状态和截图。 |

<!-- m10-acceptance:materialization -->
## 4. Dataset materialization 与 attempts

| 精确操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| 运行 `cd src-tauri && cargo test -p adaq-feature-engine --test materialization`。 | Engine materialization 测试通过，包括 `materialization_publishes_immutable_wide_parquet_and_completed_metadata`、`staging_claim_allows_only_one_concurrent_writer` 和 `startup_marks_running_interrupted_and_removes_only_its_staging_file`。 | Revision、完整输出和失败测试。 |
| 为冻结 Plan 提交 Materialization Request。 | 发布是原子的：Completed Dataset 只在 staging 成功后连同 metadata 出现；不存在可消费的 partial Dataset。 | Attempt ID、Dataset ID、manifest hash 和截图。 |
| 中断一个 running Materialization Attempt 并重启应用。 | Interruption recovery 只删除该 attempt 自己的 staging file；其他 User 与 attempt 的 staging 不受影响。 | Attempt IDs、staging paths、recovery diagnostic 和平台。 |
| 在一个 pending 与一个 running Materialization Attempt 存在时重启应用。 | Pending Attempt 在 restart 后存活；running Attempt 恢复为 failed 并保留 source evidence。 | Attempt IDs、前后状态和保留的 evidence。 |
| 尝试删除被后续 Plan 或 Attempt 引用的 Artifact 或 Dataset。 | Deletion lock 以 typed reference error 拒绝操作并指明 dependent。 | Record ID、dependent ID 和 typed error。 |
| 在启动时呈现不兼容的 legacy Feature schema 或 pre-v1 evidence。 | Engine 要求显式 reset，绝不静默删除或迁移既有 evidence。 | Typed reset-required error、reset 状态和截图。 |
| 运行 `cd src-tauri && cargo test -p adaq features pending_attempts_survive_restart_and_running_recovers_to_failed`。 | App 层 restart recovery contract 通过。 | Revision、完整输出和失败测试。 |

<!-- m10-acceptance:datasets -->
## 5. Dataset 检查

| 精确操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| 在 `/features` 打开一个 Completed Dataset 并检查其 Manifest。 | Manifest 记录 provenance：Plan identity、Definition revisions、Artifact references、Snapshot、Universe、Observation Range、Parameters 和 Seed。 | Dataset ID、缺失 manifest 字段和截图。 |
| 检查同一 Dataset 的 per-output coverage。 | 每个 output 报告自己的 coverage，Unavailability reason counts 使用稳定 reason 词汇（`warmup`、`bar-gap`、`missing-market-input`、`missing-dependency`、`unknown-universe`、`insufficient-coverage`、`undefined-arithmetic`、`artifact-missing-instrument`、`corporate-action-unavailable`）。 | Output name、coverage 数值、reason counts 和截图。 |
| 检查一个 output 的数值摘要。 | 只对 available observations 报告 min、max、mean 和 population standard deviation。 | Output name、摘要数值和截图。 |
| 应用 filter 并翻页查看 Dataset rows。 | Row inspection 有界：每页最多 50 rows，两端 pagination 正确禁用，filter 不会扩大有界窗口。 | Filter、page index、row count 和截图。 |
| 再次提交完全相同的 Materialization Request。 | Completed evidence 被复用：dedup 返回既有 Dataset identity，不重新物化。 | Dataset ID、dedup result 和截图。 |
| 在带外破坏 Dataset 文件的 content hash，再检查它。 | Content-hash corruption 被拒绝，Dataset 不可消费。 | Dataset ID、typed corruption error 和脱敏路径。 |
| 运行 `cd src-tauri && cargo test -p adaq features materialization_completes_a_dataset_and_reuses_completed_evidence`。 | App 层 completion 与 dedup contract 通过。 | Revision、完整输出和失败测试。 |

<!-- m10-acceptance:okx-journey -->
## 6. OKX Spot 路径

| 精确操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| 运行 `cd src-tauri && cargo test -p adaq-feature-engine --test reference_fixtures`。 | 已提交的 `feature-reference-vectors.json` journeys 通过，包括 OKX Spot journey。 | Revision、完整输出和失败 journey。 |
| 从 exact M9 OKX Snapshot 物化 Return、RSI、Realized Volatility 和 Bar Gap outputs。 | 每个 output 都从 immutable Snapshot 以因果 Availability 物化；Bar Gap observations 是 typed Unavailable，绝不被填充。 | Dataset ID、output names、Availability evidence 和截图。 |
| 把同一 Observation Range 切成不同 chunk partitions 并重新评估。 | Chunk equivalence 成立：batch 结果在 chunk boundaries、gaps 和 restart reconstruction 下 bit-identical（`restart_replay_and_chunk_partitions_are_bit_identical_across_gaps_dependencies_and_calendar`）。 | Partition scheme、digest 相等性和错误。 |
| 将 quantized journey summary 与已提交 reference vectors 比对。 | 跨平台 quantized summaries 一致，journey 在 macOS ARM64、Windows x86_64 和 Linux x86_64 上确定性成立。 | Digest 值、平台和差异。 |

<!-- m10-acceptance:a-share-journey -->
## 7. 中国 A 股路径

| 精确操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| 运行 `cd src-tauri && cargo test -p adaq-feature-engine --test reference_fixtures` 并定位 China A-share journey。 | A-share journey 对已提交 reference vectors 通过。 | Revision、完整输出和失败 journey。 |
| 从 exact M9 A-share Snapshot 物化 Venue Calendar features。 | Calendar features 使用 venue-local Asia/Shanghai time 并排除 scheduled breaks（`calendar_features_use_venue_local_time_and_exclude_breaks`）。 | Dataset ID、venue、calendar evidence 和截图。 |
| 物化横跨 morning session、midday break 和 afternoon session 的 Session Progress。 | Midday break 不计入 progress；scheduled closures 绝不计数（`calendar_closures_are_excluded_from_session_progress`）。 | Output name、progress 数值、break evidence 和截图。 |
| 物化 corporate action 前后的 Split 与 Dividend features。 | Split/Dividend features 向前看，并在其记录的生效 evidence 处因果可用，绝不 backward-adjusted（`split_and_dividend_features_are_forward_and_causally_available`、`ashare_corporate_actions_retain_instrument_and_evidence_identity`）。 | Action evidence ID、Available At 和截图。 |
| 检查每个物化输入序列的 `PriceBasis`。 | 所有输入保持 Unadjusted；任何地方都不应用 backward adjustment。 | Series ID、basis 和错误。 |

<!-- m10-acceptance:us-equity-journey -->
## 8. 美国股票路径

| 精确操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| 运行 `cd src-tauri && cargo test -p adaq-feature-engine --test reference_fixtures` 并定位 U.S. equity journey。 | U.S. equity journey 对已提交 reference vectors 通过。 | Revision、完整输出和失败 journey。 |
| 对来自 exact M9 Snapshot 的 Point-in-Time Instrument Universe 物化 Cross-Sectional plan。 | Plan 绑定单一 Venue、Asset Class、Bar Interval、Price Basis 和 Valuation Currency；Universe membership 是 Point-in-Time（`cross_sectional_unknown_universe_is_complete_and_mixed_markets_are_rejected`）。 | Dataset ID、Universe ID、membership evidence 和截图。 |
| 物化 Cross-Sectional Rank outputs。 | Rank/percentile/z-score outputs 确定性且与输入顺序无关（`cross_sectional_rank_percentile_and_zscore_are_deterministic`）。 | Output name、digest 和错误。 |
| 在 Universe 成员缺少 observations 时检查 coverage。 | Coverage 保留 missing members 并记录 actual coverage，不编造数值（`cross_sectional_coverage_preserves_missing_members_and_actual_coverage`）。 | Member ID、coverage 和截图。 |
| 用 Reconstructed 或 Unknown Universe state 物化。 | Reconstructed evidence 保留精确 state；Unknown 使完整 batch Unavailable，而不是部分可用。 | Universe state、Unavailability 范围和截图。 |

<!-- m10-acceptance:semantics -->
## 9. 语义证明

| 精确操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| 运行 `cd src-tauri && cargo test -p adaq-feature-engine --test operators decimal_projection_and_backward_returns_are_causal`。 | 因果性成立：bar close 同时是 Observation Time 与 Available At；backward returns 绝不使用未来信息。 | Revision、完整输出和失败测试。 |
| 运行 `cd src-tauri && cargo test -p adaq-feature-engine --test operators rolling_variants_and_realized_volatility_use_full_windows`。 | Warmup 为 full-window：rolling outputs 在完整窗口观测到之前保持 Unavailable。 | 完整输出和失败测试。 |
| 运行 `cd src-tauri && cargo test -p adaq-feature-engine --test operators rolling_state_resets_on_gaps_but_not_scheduled_closures`。 | Analytical state 在 genuine Bar Gaps 处重置，但不在 scheduled closures 处重置。 | 完整输出和失败测试。 |
| 运行 `cd src-tauri && cargo test -p adaq-feature-engine --test operators cross_sectional_coverage_preserves_missing_members_and_actual_coverage`。 | Cross-Sectional 评估使用完整 Universe 并给出显式 coverage。 | 完整输出和失败测试。 |
| 运行 `cd src-tauri && cargo test -p adaq-feature-engine --test fitting standardization_uses_population_variance_and_excludes_future_available_samples`。 | Fitted folds 排除 future-available samples；任何 fold 都不消费后续 Artifact。 | 完整输出和失败测试。 |
| 运行 `cd src-tauri && cargo test -p adaq-feature-engine --test operators future_return_direction_is_rejected_at_definition_freeze`。 | Future-return Features 在 Definition freeze 时被拒绝；未来收益用法不会进入评估。 | 完整输出和失败测试。 |
| 检查任意物化输入序列。 | 不存在 backward adjustment（所有 `PriceBasis` 为 Unadjusted），且任何 Feature 操作都不修改 Canonical Market Data。 | Series ID、basis、前后 canonical hash 和错误。 |

<!-- m10-acceptance:isolation -->
## 10. User 隔离与证据边界

| 精确操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| 运行 `cd src-tauri && cargo test -p adaq features definitions_are_user_scoped_and_presentation_never_changes_the_hash`。 | User-scoped records 绝不跨 User；presentation metadata 绝不改变 semantic hash。 | Revision、完整输出和失败测试。 |
| 运行 `cd src-tauri && cargo test -p adaq features deletion_checks_references_and_dedup_grants_no_cross_user_visibility`。 | Content dedup 复用相同 evidence，但不授予跨 User 可见性。 | 完整输出和失败测试。 |
| sign out 后以第二个 test User sign in，再打开 `/features`。 | 第二个 User 看不到第一个 User 的 Definitions、Attempts、Artifacts 或 Datasets。 | 两个脱敏 User IDs、列表状态和截图。 |
| 触发一个 Materialization Attempt 并在运行中中断。 | Atomic publication 不留下可消费的 partial Dataset；interruption evidence 被保留。 | Attempt ID、staging 状态和 diagnostic。 |
| 提交会产生 non-finite 或错误 shape 结果的输入。 | Engine 报告 typed fatal evaluation error，包含 Stage、Node、Instrument、Observation Time 和安全 diagnostics——与预期的 typed Unavailable 明确区分。 | Typed error class、stage/node identity 和 diagnostics。 |
| 以拥有者 User 身份翻页查看 Dataset rows。 | Row inspection 保持有界，绝不暴露其他 User 的 evidence。 | Page index、row count 和截图。 |

<!-- m10-acceptance:features-gui -->
## 11. `/features` workspace GUI

| 精确操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| 直接打开 `/features`。 | Route 立即绘制；内容之前没有 page-level skeleton gate。 | Route、第一帧和 console 输出。 |
| 依次打开 Definitions、Fitting、Materialization、Datasets 各 tab。 | 每个 owning control 自管自己的 loading、error 和 empty state；loading feedback 出现在数据边界。 | Tab、loading owner、截图和 accessibility tree 文本。 |
| 离开后重新进入 `/features`。 | Read-only list metadata 可以从 current-session cache 立即绘制，并由所属 control 在后台刷新。 | Route、loading owner、cache state 和 timing。 |
| 只用键盘并开启 screen reader 操作 Definition editor、Attempt lists 和 Dataset inspection。 | 每个 control 可 focus、有 label 且不依赖 pointer 即可操作。 | Focused control、播报名称和截图。 |
| 检查状态指示是否只依赖颜色。 | 没有任何 state 只依赖颜色；每个状态都有文字/label 伴随。 | Control、state 和截图。 |
| 将 content area 设置为 1024 px 并重复 tab 检查。 | 布局保持可用：无被裁剪的 control、隐藏的操作或损坏的 pagination。 | 平台缩放、tab、截图和 accessibility tree 文本。 |
| 运行 `pnpm exec jest --watchman=false --runInBand src/loading-boundaries.test.ts src/lib/i18n.test.ts src/router.test.ts`。 | Loading boundaries、locale 覆盖和 `/features` route contract 通过。 | Revision、suite/test 和完整输出。 |
| 检查 `/features` 与 shell 中是否存在 Factor、Model-training、Paper、Bot 或 Live controls。 | M10 不存在这些 out-of-scope control。 | 如果出现 control，保留 Route 和截图。 |

<!-- m10-acceptance:performance-baselines -->
## 12. 性能基线

| 精确操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| 运行 `cd src-tauri && cargo test -p adaq-feature-engine --release --test benchmarks -- --ignored --test-threads=1`。 | 两个规范 workload 串行完成：1,000,000-Bar Time-Series workload 与 10,000-Instrument × 252-Observation Cross-Sectional workload。本次复核运行串行总耗时 ~171.8 s：Time-Series 17,469 ms（cancellation stop 150 ms）、Cross-Sectional 58,076 ms，进程 peak RSS 437,321,728 bytes。 | Revision、完整输出、workload 和平台。 |
| 与 `src-tauri/crates/adaq-feature-engine/fixtures/feature-benchmark-baseline.json` 对比结果。 | Baseline 使用 schema `adaq-feature-benchmark-baseline@1.0.0`，记录于 macOS ARM64（`aarch64-apple-darwin`），记录值：Time-Series 20155 ms，Cross-Sectional 64828 ms，peak RSS 439,386,112 bytes（进程 high-water mark，`ru_maxrss`）。Baseline 仅为记录：不设定虚构的延迟或 RSS 目标。本次复核运行的实测值持平或优于上述记录值，且 baseline 文件重新生成后无差异（`git diff --exit-code` clean）。 | Baseline 文件 hash、实测数值和平台差异。 |
| 在长时间 Materialization 或 Fitting Attempt 运行期间观察 GUI。 | GUI 绝不冻结：重型工作运行在 supervised worker 中，UI 保持响应。 | Attempt ID、UI 响应证据和截图。 |
| 在运行期间与结束后翻页查看大型 Completed Dataset。 | Dataset pagination 在负载下仍保持每页 50 rows 的有界窗口。 | Page index、row count 和截图。 |

<!-- m10-acceptance:regressions -->
## 13. 回归与边界检查

| 精确操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| 运行 `pnpm exec jest --watchman=false --runInBand`。 | 所有 frontend suites 通过，包括 M7/M8/M9 contract tests、locale、route、loading 和 feature data tests。 | Revision、suite/test 和完整输出。 |
| 运行 `cd src-tauri && cargo test --workspace`。 | 完整 Rust workspace 通过；M5–M9 journeys 保持规范且未改变。 | Revision、完整输出和失败测试。 |
| 打开 [`docs/m7-manual-acceptance.md`](m7-manual-acceptance.md)、[`docs/m8-manual-acceptance.md`](m8-manual-acceptance.md) 和 [`docs/m9-manual-acceptance.md`](m9-manual-acceptance.md)。 | 既有 Components、Backtests、Validation、Model Dataset、Forecast Evaluation 和 Markets 路径仍是规范回归路径。 | Guide section 和损坏/改变的路径。 |
| 检查 shell 与所有 routes 是否出现 Factor research、Model training、Strategy、Paper、Bot 或 Marketplace 能力。 | M11+ 能力不存在；M10 不新增任何此类能力。 | 如果出现能力，保留 Route 和截图。 |

<!-- m10-acceptance:automated-gates -->
## 14. 自动 release gates 与 CI

| 精确操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| 运行 `cd src-tauri && cargo fmt --all --check`。 | Rust formatting 通过。本次复核运行通过，无 diff。 | Revision 和完整 diff。 |
| 运行 `cd src-tauri && cargo test --workspace`。 | 所有 Rust workspace tests 与 doctests 通过；适用时记录被 ignored 的长时间 benchmark。本次复核运行通过：307 passed / 0 failed / 4 ignored，共 24 个 test binaries（ignored 为两个 `--ignored` benchmark workloads 与两个 generator-gated tests）。 | Revision、完整未过滤输出、失败测试和平台。 |
| 运行 `cd src-tauri && cargo check --workspace`。 | Native workspace type-check 通过。本次复核运行通过。 | Revision 和完整输出。 |
| 运行 `pnpm exec jest --watchman=false --runInBand`。 | 所有 frontend tests 通过。本次复核运行通过 23 个 suites、91 个 tests；此前唯一失败是 M10 README-link contract test，已由本步骤的 README 更新解决。 | Revision、suite/test 和完整输出。 |
| 运行 `pnpm run build`。 | Strict TypeScript check 与 Vite production build 通过。本次复核运行通过。 | Revision 和完整输出。 |
| 运行 `pnpm run lint`。 | Lint 通过；既有 warnings 与新增 findings 分开记录。本次复核运行通过，含 12 个既有 warnings，均不在四个 M10 acceptance 文件中。 | Revision、file/rule 和完整输出。 |
| 运行 `git diff --check`。 | 没有 whitespace errors。本次复核运行 clean。 | Revision 和完整输出。 |
| 运行 `gh workflow run "Indicator engine acceptance" --ref <reviewed-ref>`。 | reviewed ref 的三平台 native matrix 启动，并分别暴露 macOS ARM64、Windows x86_64 和 Linux x86_64 jobs。 | Workflow URL、SHA、job URLs、conclusion 和失败日志片段。 |
| 搜索仓库是否配置 secret scanner。 | 本 checkout 没有 secret-scan command；仍需人工确认 diff 不含 credential material 或 token-like fixture value。本次复核 diff 已经人工检查，不含任何 credential 或 token 材料。 | Command/output 和复核文件列表。 |
| 记录适用的 GitHub Actions `macOS ARM64`、`Windows x86_64` 和 `Linux x86_64` run URLs。 | 为 reviewed revision 或明确标识的 platform baseline 保留 native fixture/Rust gates 与 release packaging evidence。 | Run URL、SHA、job、conclusion 和失败日志片段。 |

Native matrix 定义在 [`.github/workflows/indicator-engine.yml`](../.github/workflows/indicator-engine.yml)，是可手动 dispatch、覆盖 macOS ARM64、Windows x86_64 和 Linux x86_64 的三平台 matrix；release packaging 定义在 [`.github/workflows/release.yml`](../.github/workflows/release.yml)。Local pass 不能替代要求的平台证据。Acceptance record 必须区分 reviewed M10 revision 与旧 platform baseline。

以下是 unchanged native/fixture 与 packaging path 的已记录平台证据：

| Workflow evidence | Revision | Jobs | Result |
| --- | --- | --- | --- |
| [Indicator engine acceptance run 31062405209](https://github.com/tonywxx/adaq/actions/runs/31062405209) | `34d63ab1688b7dbeff8f5cd394a848895381ec08` | macOS ARM64、Windows x86_64 | Success |
| [Release run 31282997179](https://github.com/tonywxx/adaq/actions/runs/31282997179) | `5d1d236999984ef4a8bcc646b8e927e37e9fb708` | Validate release、macOS ARM64、Windows x86_64、publish | Success |

#87 变更只包含文档、README/roadmap 和 frontend acceptance-contract tests，没有改变 Rust/provider、fixture 或 packaging code，因此上面的 local gates 验证 reviewed revision，已记录的 indicator-engine run 保留适用的跨平台 baseline，已记录的 release run 保留 unchanged release path 的 packaging baseline。Linux x86_64 matrix entry 是在该已记录 baseline 之后引入的，目前尚无任何完整三平台 run 成功过；完整 matrix 将由下一次 native（`src-tauri/**`）变更提供证据，在此之前 Linux x86_64 是一项明确记录的未解决风险。

<!-- m10-acceptance:acceptance-matrix -->
## 15. 最终验收矩阵

矩阵是证据，不是替代关闭 issue 的理由。每一行都指出 implementation boundary、focused check 和可重复的人工验收 section。

### Parent #77 criteria

| ID | 要求 | Implementation / focused evidence | Manual / broad evidence | 未解决风险 |
| --- | --- | --- | --- | --- |
| P1 | 十个 native sub-issues 按依赖顺序实现 M10 slices 并保留独立 evidence。 | 下方 slice matrix；`adaq-feature-engine/tests/contracts.rs`、`operators.rs`、`fitting.rs`、`materialization.rs`。 | Sections 2–12 与 slice rows。 | 所有 slice row 记录后无。 |
| P2 | `adaq-feature-engine` 拥有 Definition、Plan 2.0、operator、fitting、availability、missingness、evaluation 与 identity contracts；Indicator Engine 保持 subengine。 | `adaq-feature-engine/tests/contracts.rs`（`definition_and_plan_identities_are_canonical_and_replayable`、`plan_rejects_untyped_signal_provenance`）；`operators.rs`（`indicator_nodes_use_the_pinned_indicator_engine_and_validate_output`）。 | Sections 2、3、9。 | 无。 |
| P3 | Pointwise、Time-Series 与 Cross-Sectional Features 因果、scope 正确、有限或 typed Unavailable、跨 chunking 确定性，且不修改 Canonical Market Data。 | `operators.rs`（`dependency_slots_share_batch_and_stateful_evaluation`、`restart_replay_and_chunk_partitions_are_bit_identical_across_gaps_dependencies_and_calendar`、`pointwise_encoding_and_checked_division_are_typed`）。 | Sections 6–9。 | Committed fixtures 无。 |
| P4 | Fitting Protocols 与 Attempts 发布 immutable Artifacts 且无泄漏；materialization 只应用、绝不重新拟合。 | `fitting.rs`（`lifecycle_coalesces_reuses_retries_and_keeps_artifacts_user_scoped_and_locked`、`feature_evaluator_applies_bound_artifact_without_fitting_or_mutating_it`）。 | Sections 3、9。 | 无。 |
| P5 | Completed Feature Datasets 是 content-addressed immutable Parquet evidence，带 SQLite metadata、原子发布、恢复、User isolation 与 deletion locks。 | `materialization.rs`（`materialization_publishes_immutable_wide_parquet_and_completed_metadata`、`content_hash_corruption_is_not_consumable_and_dataset_references_lock_deletion`）；`src-tauri/src/features/tests.rs`（`artifact_deletion_is_locked_by_typed_references`）。 | Sections 4、5、10。 | 无。 |
| P6 | Batch 与 stateful observation 评估在 chunk boundaries、gaps、missing dependencies 和 restart reconstruction 下等价。 | `operators.rs`（`batch_and_stateful_observation_paths_are_identical`）；`materialization.rs`（`stage_events_uses_the_same_evaluator_as_stateful_observation`）；`fitting.rs`（`bound_artifact_evaluation_is_identical_across_batch_stateful_and_replay_paths`）。 | Sections 6、9。 | 无。 |
| P7 | `/features` 立即绘制，并暴露 accessible、本地化的 Definition、fitting、materialization、preview 和 inspection workflows。 | `src/loading-boundaries.test.ts`、`src/lib/i18n.test.ts`、`src/router.test.ts`、`src/features/features/features-data.test.ts`。 | Sections 2、11。 | OS assistive technology 差异按平台记录。 |
| P8 | OKX Spot、China A-share 和 U.S. equity reference journeys 与全部声明的 failure paths 通过。 | `adaq-feature-engine/tests/reference_fixtures.rs`（`committed_reference_vectors_match_the_three_market_journeys`）；`fixtures/feature-reference-vectors.json`；`reference_fixtures.rs` failures journey。 | Sections 6–8。 | Committed vectors 无。 |
| P9 | M11 只能选择 Completed Feature Datasets；M10 不新增 Factor research、Model training、Paper order、Bot、Marketplace、script engine 或 Feature Component ABI。 | Routes/DTOs 只暴露 Feature evidence；Section 11 与 scope statement。 | Sections 1、11、13。 | 无。 |
| P10 | English 与 Simplified Chinese 架构文档及最终人工验收文档语义等价。 | `docs/m10-manual-acceptance.md`、`.zh-CN.md`、`src/m10-manual-acceptance.test.ts`；`docs/m10-feature-engineering.md`、`.zh-CN.md`。 | Sections 1–15。 | Parity test 后无。 |
| P11 | 每条 criterion 都映射到 implementation 与 focused/broad evidence；issue closure 本身不是证据。 | 本节及下方 child matrix。 | Sections 1–14。 | Review 后无。 |
| P12 | Rust formatting/tests/checks、frontend Jest/build/lint、diff checks、accessibility review 和 supported-platform CI 在 final revision 通过。 | 上方 gate table；`src/loading-boundaries.test.ts` 覆盖 accessibility 相关状态。 | Sections 11、14。 | macOS ARM64 与 Windows x86_64 已由已记录 baseline run 覆盖；Linux x86_64 matrix entry 在该 baseline 之后引入，仍是明确记录的未解决风险，直到下一次 native（`src-tauri/**`）变更提供完整三平台 run 证据。 |
| P13 | 在关闭 parent 前发布英文 completion comment，记录 implementation、exact commands、results、revision 与 CI links。 | Issue #87 completion comment 引用 final revision 与 commands。 | 下方 Acceptance record。 | Parent closure 是 maintainer 明确要求的独立动作。 |

### M10.1–M10.10 slice matrix

| Slice | 已交付 boundary | Focused evidence | Final acceptance evidence | 未解决风险 |
| --- | --- | --- | --- | --- |
| M10.1 / #78 | Feature Engine contracts 与 Feature Plan 2.0：canonical identities、resource limits、reset-required legacy rejection。 | `adaq-feature-engine/tests/contracts.rs`。 | Section 2 与 Rust gates。 | 无。 |
| M10.2 / #79 | Pointwise 与 Time-Series Feature operators，带因果 Availability、full-window Warmup 与 gap reset。 | `adaq-feature-engine/tests/operators.rs`。 | Sections 6、9 与 Rust gates。 | Committed fixtures 无。 |
| M10.3 / #80 | Cross-Sectional Feature scope 与 Universe operators，带 coverage 与确定性。 | `adaq-feature-engine/tests/operators.rs`（`cross_sectional_*`）。 | Section 8 与 Rust gates。 | 无。 |
| M10.4 / #81 | Fitted Transformation Protocols 与 Artifacts，带 walk-forward fold isolation。 | `adaq-feature-engine/tests/fitting.rs`。 | Section 3 与 Rust gates。 | 无。 |
| M10.5 / #82 | Immutable Feature Dataset materialization 与保留的 Attempts，带 recovery 与 deletion locks。 | `adaq-feature-engine/tests/materialization.rs`。 | Sections 4、5 与 Rust gates。 | 无。 |
| M10.6 / #83 | Batch/observation equivalence 与同一 evaluator 下的 Component consumers。 | `operators.rs`（`batch_and_stateful_observation_paths_are_identical`）；`reference_fixtures.rs`。 | Sections 6、9 与 Rust gates。 | 无。 |
| M10.7 / #84 | User-scoped Feature APIs 与 FIFO background runner，带 cancellation 与 restart recovery。 | `src-tauri/src/features/tests.rs`；`src/tauri-command-scheduling.test.ts`。 | Sections 3、4、10 与两套 gate suites。 | 无。 |
| M10.8 / #85 | 本地化 `/features` workspace：Definitions、Fitting、Materialization、Datasets、Preview。 | `src/features/features/features-data.test.ts`；`src/lib/i18n.test.ts`；`src/loading-boundaries.test.ts`。 | Sections 2、11 与 full Jest/build gates。 | OS visual differences 仍需人工 evidence。 |
| M10.9 / #86 | 三市场 fixtures、benchmarks 与 hardening。 | `adaq-feature-engine/tests/reference_fixtures.rs`、`benchmarks.rs`；`fixtures/feature-reference-vectors.json`；`fixtures/feature-benchmark-baseline.json`。 | Sections 6–8、12 与 Rust gates。 | Benchmark 数值仅为记录式平台证据。 |
| M10.10 / #87 | 本双语、跨平台 acceptance contract 与 evidence record。 | `src/m10-manual-acceptance.test.ts`、全部 required gates。 | Sections 1–15 与 issue comment。 | Linux x86_64 仍是明确记录的未解决风险，直到完整三平台 run 成功；其余所有 row 已记录。 |

<!-- m10-acceptance:acceptance-record -->
## 16. 验收记录与清理

记录 reviewed revision、OS/architecture/display scale、Node/pnpm/Rust versions、command outputs、focused test counts、full-gate conclusions、Dataset/Artifact/Attempt IDs、content-hash 与 reference-vector digests、revision 与 deletion-lock evidence、脱敏 User IDs、Route screenshots、keyboard/accessibility observations，以及 exact CI run URLs/SHA/conclusions。同时记录 Linux x86_64 说明：该 matrix entry 在已记录 baseline 之后引入，在下一次 native（`src-tauri/**`）变更提供完整三平台 run 证据之前，保持为明确记录的未解决风险。不要在记录中保存 credentials、tokens、private paths 或 private market data。

Fixture acceptance 后，只删除本次运行创建的 disposable profiles、临时 acquisition directories、generated package/build outputs 和 test databases。不要删除 repository fixtures 或 finalized evidence。如果某个平台仍持有 file handle，先停止拥有它的进程再清理，并记录平台结果。

只有当上面所有 applicable rows 通过、所有 automated gates 绿色、required platform evidence 已记录且没有违反 M10 boundary 时，M10 才算 accepted。M11 Factor research 以及后续 Model、Strategy、Paper、Bot、Monitoring 和 feedback milestones 仍然 out of scope。
