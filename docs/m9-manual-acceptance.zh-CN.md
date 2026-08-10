# M9 人工验收

这是 M9 的规范人工复核路径。规范本地运行环境为 macOS ARM64；Windows x86_64 与 Linux x86_64 的命令替换记录在下方。逐行执行并在失败时保留要求的证据。M9 的边界止于可信的多市场观察、不可变研究证据、安全且不下单的连接、本地化和行情检查；它不提交 Paper 或 Live 订单，也不交付 M10–M18。

不得把凭证、授权 Header、OTP、Token、私有路径或私有市场数据放入 issue 评论、commit、截图、日志、导出文件或本记录。可选真实 Provider 检查只能使用维护者凭证，并且只能在 **Settings → Connections** 中输入；已提交的 Fixtures 与本地 Mock Server 才是规范验收路径。

<!-- m9-acceptance:scope -->
## 1. 范围与前置条件

| 精确操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| 在仓库根目录运行 `node --version`。 | 使用 Node.js 24 或更新版本，与 release baseline 一致。 | 完整输出和安装方式。 |
| 运行 `pnpm --version`。 | 可用 pnpm 11.20.0，与 `package.json` 一致。 | 完整输出和安装方式。 |
| 运行 `pnpm install --frozen-lockfile`。 | 依赖与 `pnpm-lock.yaml` 一致。 | 完整输出和两个工具版本。 |
| 运行 `rustup toolchain install stable`。 | stable Rust toolchain 可用。 | 完整输出和 `rustup show`。 |
| 运行 `rustup target add --toolchain stable wasm32-unknown-unknown`。 | Component fixture target 已安装。 | 完整输出和已安装 target 列表。 |
| 在 version control 之外提供 Supabase 变量后运行 `pnpm tauri dev`。 | Desktop shell 打开，且不暴露配置值。 | 截图和脱敏错误。 |
| 打开新的 device profile 并进入 **Settings → General**。 | 仅显示 System、English (US) 和 简体中文 三种 locale 选择。 | 截图、平台和 locale 状态。 |

macOS 使用 `shasum -a 256 <path>`，Windows PowerShell 使用 `Get-FileHash -Algorithm SHA256 <path>`，Linux 使用 `sha256sum <path>`。Native file picker、data-folder 路径、显示缩放和 secret-store 提示属于平台差异。

<!-- m9-acceptance:localization -->
## 2. 本地化、首次绘制与生命周期

| 精确操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| 从新的 profile 启动应用。 | React/native work 开始前先绘制 Loading HTML；没有空白窗口或 locale 闪烁。 | 平台、Route、第一帧和 console 输出。 |
| 在 **Settings → General** 选择 **English (US)**。 | 当前 Route 继续挂载，英文 copy 立即出现。 | Route、截图和可见 missing key。 |
| 在 **Settings → General** 选择 **简体中文**。 | 当前 Route 继续挂载，中文 copy 立即出现，document language 变为 `zh-CN`。 | Route、截图和 accessibility tree 文本。 |
| 设置为 **System**，sign out，再 sign in，并重新打开 **Settings → General**。 | System resolution 稳定；device-local preference 不属于 profile data。 | 脱敏 User ID、sign out 前后 locale 和截图。 |
| 打开 research-data reset confirmation 并取消。 | Reset 范围明确；locale preference 仍可用。 | Confirmation 文案和前后 locale。验收中不要执行破坏性 reset。 |
| 在两个 locale 中依次访问 **Markets**、**Components**、**Models**、**Backtest**、**Validation** 和 **Settings**。 | 不出现 missing visible label、empty state、error state、loading label 或 accessibility name。 | Route、locale、key/label、截图和技术详情。 |
| 运行 `pnpm exec jest --watchman=false --runInBand src/lib/i18n.test.ts src/bootstrap.test.ts`。 | Locale resolution、持久化边界、fallback、`Intl` formatting 和首次绘制顺序通过。 | Revision、suite/test 和完整输出。 |

<!-- m9-acceptance:connections -->
## 3. Provider Connections 与不下单不变量

| 精确操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| 打开 **Settings → Connections**。 | 只显示 Alpaca Paper 与 OKX Demo；不存在 Live 或 custom endpoint 字段。 | 截图和可见/技术错误。 |
| 运行 `cd src-tauri && cargo test --lib connections`。 | Fixture-backed save、test、rotation、deletion、User isolation、endpoint allowlist、redaction、permission、currency 和 clock-skew 测试通过。 | Revision、完整输出和失败测试。 |
| 运行 `cd src-tauri && cargo test --lib connections connection_test_never_requests_an_order_endpoint`。 | Request capture 只包含 account/time/config/balance 调用；不会请求 `/orders`、trade 或 order endpoint。 | 完整输出和脱敏 request paths。 |
| 保存 Alpaca Paper fixture profile 并选择 **Test connection**。 | Profile 只保存不透明 Secret Reference 与脱敏 metadata；Test 为只读。 | Profile ID、状态、typed error 和截图；不得保留 key pair。 |
| 保存 OKX Demo fixture profile 并选择 **Test connection**。 | Demo simulation Header、权限、currency、clock 和 capability evidence 被保留，但不含 secret 值。 | Profile ID、状态、typed error 和截图；不得保留 key/passphrase。 |
| 用无效 replacement 轮换每个 fixture profile。 | 之前可用的 profile 仍保持 active，失败 replacement 不遗留 secret。 | Profile ID、脱敏状态和技术错误。 |
| 在 dependent-runtime 检查后删除每个 fixture profile。 | 删除需要显式操作、移除 OS-store entry 并使 metadata 失效；active dependent runtime 会阻止删除。 | Profile ID、guard result 和脱敏 diagnostic。 |
| 在 fixture tester 中尝试 Live endpoint 或任意 custom endpoint。 | 在网络请求前被拒绝。 | Endpoint class 和 typed rejection。 |
| 如果执行可选的真实 Provider 检查，只能在 **Settings → Connections** 输入凭证并在之后删除 profile。 | 只使用 Provider 固定的 Paper/Demo 路径；issue evidence 不包含凭证值。 | Provider、timestamp、状态和脱敏错误；不得保留凭证或 Header。 |

<!-- m9-acceptance:crypto -->
## 4. OKX Spot 路径

| 精确操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| 运行 `cd src-tauri && cargo test -p adaq-data-pipeline --lib okx::tests -- --nocapture`。 | OKX fixture 测试覆盖 Instrument Master、pagination、rate/retry、closed Bars、checkpoint、restart/resume、gaps、revisions、REST/WebSocket reconciliation、有界 Trade retention 与非持久化 Level 2。 | Revision、完整输出和失败测试。 |
| 打开 `/markets/crypto` 并搜索 fixture-backed OKX Instrument。 | Instrument identity 使用 Venue 加 native code；Provider symbol 与 source mapping 仍可见。 | Route、Instrument ID、Provider symbol 和截图。 |
| 在选择 history range 前检查 Instrument Master record。 | 可见 effective time、status、full observed-universe evidence、Provider response hash 和 Point-in-Time 选择规则。 | Snapshot ID、effective time、evidence state 和缺失字段。 |
| 从中断的 fixture checkpoint 恢复 one-minute acquisition。 | Acquisition 恢复时不产生 duplicate records，也不覆盖既有 Source/Canonical revisions。 | Operation ID、checkpoint、revision 和 diagnostic。 |
| 检查已获取区间的 Source 与 Canonical quality。 | Provider/upstream、request、response/content hashes、exact values、gaps、quarantine、quality state 和 capability 分开且可检查。 | Dataset IDs、state、gap/quarantine counts 和截图。 |
| 从已接受的 one-minute evidence 派生更高 interval。 | Aggregation 确定性、calendar/grid 对齐、不可变并绑定 provenance。 | Source/Snapshot IDs、interval、hash 和错误。 |
| 发布或选择生成的 immutable Snapshot，再重新打开 `/markets/crypto`。 | Snapshot identity 与 quality 在重新进入后保持稳定；没有 order control。 | Snapshot ID、Route、截图和技术详情。 |

<!-- m9-acceptance:a-shares -->
## 5. 中国 A 股路径

| 精确操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| 运行 `cd src-tauri && cargo test -p adaq-data-pipeline --lib a_share`。 | Fixture 与 local-mock 测试覆盖 actual-upstream provenance、SSE/SZSE identity、exact decimals、sessions、corporate actions、quality、cancellation 和 restart/resume。 | Revision、完整输出和失败测试。 |
| 打开 `/markets/a-shares` 并搜索 fixture-backed ordinary equity。 | Venue、native code、Provider symbol、status 和 `akshare-rs` source mapping 可见。 | Route、Instrument ID、provider/method 和截图。 |
| 检查 acquisition provenance card。 | Actual upstream、method、request/response/content hashes、connector version、retrieval time 和 capability limitations 可见。 | Source ID、hashes、缺失字段和截图。 |
| 检查一组 unadjusted Canonical Bars。 | `PriceBasis` 为 Unadjusted；Asia/Shanghai Trading Date、morning session、midday break、afternoon session 和 quality/gaps 明确。 | Series ID、calendar ID、interval、basis 和错误。 |
| 检查同一 Instrument 的独立 corporate-action evidence。 | Actions 保持独立不可变 evidence，不静默合并到 Bars，也不用于修复。 | Action evidence ID、quality state 和截图。 |
| 发布或选择生成的 immutable Snapshot，再重新打开 `/markets/a-shares`。 | Snapshot identity、coverage、quality、limitations 和 source provenance 仍可检查。 | Snapshot ID、Route、截图和技术详情。 |

<!-- m9-acceptance:us-equities -->
## 6. 美国股票路径

| 精确操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| 运行 `cd src-tauri && cargo test -p adaq-data-core --lib alpaca`。 | Alpaca fixture 测试覆盖 fixed endpoints、exact values、IEX capability、DST/holiday/early-close calendars、symbol limits 和 daily-bar anchoring。 | Revision、完整输出和失败测试。 |
| 运行 `cd src-tauri && cargo test -p adaq-data-pipeline --lib us_equity`。 | Pipeline 测试覆盖 authenticated fixture acquisition、pagination/retry、checkpoints、Source/Canonical evidence、quality 和 Snapshot compatibility。 | Revision、完整输出和失败测试。 |
| 打开 `/markets/us-equities` 并搜索 fixture-backed active asset。 | Alpaca symbol、Venue identity、status、exchange、tradability 和 Instrument Source Mapping 可见。 | Route、Instrument ID、Provider symbol 和截图。 |
| 检查 Provider Capability Snapshot。 | Basic plan、IEX feed、history/delay/rate/stream limits、unavailable capabilities 和 capture time 明确；不出现 consolidated realtime 声明。 | Capability ID、feed、limitation 和截图。 |
| 检查一组 historical Bars 及其 session evidence。 | America/New_York Trading Date、DST、holiday/early-close state、UTC boundaries、`PriceBasis::Unadjusted`、quality 和 gaps 可见。 | Series ID、calendar ID、state、basis 和错误。 |
| 发布或选择生成的 immutable Snapshot，再重新打开 `/markets/us-equities`。 | Snapshot 与 provenance 保持稳定；如有 auxiliary observation，必须独立展示，绝不能修复 Canonical data。 | Snapshot ID、source/revision IDs 和截图。 |

<!-- m9-acceptance:quality-snapshot -->
## 7. Quality、生命周期与研究证据

| 精确操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| 运行 `cd src-tauri && cargo test -p adaq-data-pipeline --lib`。 | Passed、Degraded、Rejected、quarantine、explicit gaps、cancellation、atomic cleanup、User isolation、revisions 和 Snapshot publication 测试通过。 | Revision、完整输出和失败测试。 |
| 提交包含 scheduled closure 或 session break 的 fixture。 | Closure 是 calendar evidence，而不是错误的 Bar Gap。 | Venue、Trading Date、phase、quality report 和 gap list。 |
| 提交 continuous session 内缺少 Bar 的 fixture。 | Genuine Bar Gap 被保留，绝不 forward-fill、interpolate、clip 或手工修改。 | Gap range、quality report 和 canonical hash。 |
| 为现有区间提交一个 corrected Source revision。 | 旧 revision 仍是 append-only evidence，新 Canonical/Snapshot identity 独立存在。 | Source/revision IDs、hashes 和前后状态。 |
| 尝试删除被 Dataset、Run、Report 或后续 research object 引用的 Snapshot。 | Deletion lock 拒绝操作，并报告 dependent reference。 | Snapshot ID、dependent ID 和 typed error。 |
| 在存在更新 Source revision 后 replay 一个旧 Snapshot。 | 旧 Snapshot 返回原始 immutable evidence，不会被静默升级。 | 新旧 Snapshot IDs、hashes 和 replay result。 |
| sign out 后以第二个 test User sign in，再检查 Watchlist、pipeline、Snapshot 和 connection lists。 | User-scoped private records 与 secret references 不跨 User 泄漏；shared content 遵循其 access contract。 | 两个脱敏 User IDs、record IDs 和截图。 |

<!-- m9-acceptance:markets -->
## 8. Markets GUI、无障碍与边界

| 精确操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| 访问 `/`、`/markets`、`/markets/crypto`、`/markets/a-shares` 和 `/markets/us-equities`。 | Root 保持 Operations shell；所有 market routes 都有本地化 navigation，且没有重复 Watchlist store。 | Route、URL、截图和 console 输出。 |
| 向 Watchlist 添加一个 crypto、一个 A-share 和一个 U.S. equity Instrument。 | 一个 User-scoped、asset-neutral Watchlist 保留 Venue-plus-native-code identity；各 Route 正确过滤。 | 脱敏 User ID、item IDs、Route 和截图。 |
| 删除并重新添加一个 Watchlist item，再选择不同 Active Instrument。 | Limits、selection、Active Instrument 行为和 reset semantics 保持正确。 | Item IDs、Route、前后状态和错误。 |
| 访问其他 Route 后重新进入 market route。 | Read-only list/chart metadata 可以从 current-session cache 立即绘制，并由所属 control 在后台刷新；native validation 不改变。 | Route、loading owner、cache state 和 timing。 |
| 将 content area 设置为 1024 px，并用键盘访问每个 market route。 | Search、Watchlist、chart、provenance、quality、loading、error 和 empty controls 保持有 label、可 focus、可见且不依赖颜色。 | 平台缩放、Route、focused control、截图和 accessibility tree 文本。 |
| 搜索不可用的 quote 或 Provider field。 | Bid/Ask、realtime、consolidated coverage、adjusted basis 和 open-session claims 保持 unavailable，不被编造。 | Instrument、field、显示状态和截图。 |
| 检查每个 market route 的 order、Feature、Factor、Model-training、Paper、Bot 和 Live controls。 | M9 Route 不暴露这些 out-of-scope action。 | 如果出现 control，保留 Route 和截图。 |

<!-- m9-acceptance:regressions -->
## 9. M7/M8 回归与双语等价

| 精确操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| 运行 `pnpm exec jest --watchman=false --runInBand`。 | 所有 frontend suites，包括 M7/M8 research、locale、route、loading 和 market tests，通过。 | Revision、suite/test 和完整输出。 |
| 打开 [`docs/m7-manual-acceptance.md`](m7-manual-acceptance.md) 和 [`docs/m8-manual-acceptance.md`](m8-manual-acceptance.md)。 | 既有 Components、Backtests、Validation、Model Dataset 和 Forecast Evaluation 路径仍是规范回归路径。 | Guide section 和损坏/改变的路径。 |
| 运行 focused M8 guide contract test。 | 两份 M8 guides 仍可执行，两个 README 仍链接它们。 | Revision 和完整输出。 |
| 并排检查 English 与 Simplified Chinese M9 guides。 | Headings、操作顺序、预期结果、失败证据、清理/安全规则、平台替换、矩阵覆盖和边界声明语义等价。 | File、section、不一致文本和期望含义。 |

<!-- m9-acceptance:automated-gates -->
## 10. 自动 release gates 与 CI

| 精确操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| 运行 `cd src-tauri && cargo fmt --all --check`。 | Rust formatting 通过。 | Revision 和完整 diff。 |
| 运行 `cd src-tauri && cargo test --workspace`。 | 所有 Rust workspace tests 与 doctests 通过；适用时记录被 ignored 的 OS-keyring test。 | Revision、完整未过滤输出、失败测试和平台。 |
| 运行 `cd src-tauri && cargo check --workspace`。 | Native workspace type-check 通过。 | Revision 和完整输出。 |
| 运行 `pnpm exec jest --watchman=false --runInBand`。 | 所有 frontend tests 通过。 | Revision、suite/test 和完整输出。 |
| 运行 `pnpm run build`。 | Strict TypeScript check 与 Vite production build 通过。 | Revision 和完整输出。 |
| 运行 `pnpm run lint`。 | Lint 通过；既有 warnings 与新增 findings 分开记录。 | Revision、file/rule 和完整输出。 |
| 运行 `git diff --check`。 | 没有 whitespace errors。 | Revision 和完整输出。 |
| 搜索仓库是否配置 secret scanner。 | 本 checkout 没有 secret-scan command；仍需人工确认 diff 不含 credential material 或 token-like fixture value。 | Command/output 和复核文件列表。 |
| 记录适用的 GitHub Actions `macOS ARM64`、`Windows x86_64` 和 `Linux x86_64` run URLs。 | 为 reviewed revision 或明确标识的 platform baseline 保留 native fixture/Rust gates 与 release packaging evidence。 | Run URL、SHA、job、conclusion 和失败日志片段。 |

Native matrix 定义在 [`.github/workflows/indicator-engine.yml`](../.github/workflows/indicator-engine.yml)，release packaging 定义在 [`.github/workflows/release.yml`](../.github/workflows/release.yml)。Local pass 不能替代要求的平台证据。Acceptance record 必须区分 reviewed M9 revision 与旧 platform baseline。

以下是 unchanged native/fixture 与 packaging path 的已记录平台证据：

| Workflow evidence | Revision | Jobs | Result |
| --- | --- | --- | --- |
| [Indicator engine acceptance run 30439984251](https://github.com/tonywxx/adaq/actions/runs/30439984251) | `735240def735d7684ff9e4e8751fbe1498ead778` | macOS ARM64、Windows x86_64、Linux x86_64 | Success |
| [Release run 31282997179](https://github.com/tonywxx/adaq/actions/runs/31282997179) | `5d1d236999984ef4a8bcc646b8e927e37e9fb708` | Validate release、macOS ARM64、Windows x86_64、publish | Success |

#76 变更只包含文档、README/roadmap 和 frontend acceptance-contract test，没有改变 Rust/provider、secret-store、fixture 或 packaging code。因此上面的 local gates 验证 reviewed revision，已记录的 matrix 保留适用的跨平台 baseline。

<!-- m9-acceptance:acceptance-matrix -->
## 11. 最终验收矩阵

矩阵是证据，不是替代关闭 issue 的理由。每一行都指出 implementation boundary、focused check 和可重复的人工验收 section。

### Parent #66 criteria

| ID | 要求 | Implementation / focused evidence | Manual / broad evidence | 未解决风险 |
| --- | --- | --- | --- | --- |
| P1 | 发布语义等价的英文与简体中文 guides。 | `docs/m9-manual-acceptance.md`、`.zh-CN.md`、`src/m9-manual-acceptance.test.ts`。 | Sections 1–11。 | Parity test 后无。 |
| P2 | M9 完成后更新两个 README，并链接两份 guide。 | README 的 milestone、scope、feature 和 documentation entries。 | Section 9。 | 无。 |
| P3 | 在首次绘制前解析 locale，支持切换、持久化和审计。 | `src/lib/i18n.test.ts`、`src/bootstrap.test.ts`。 | Section 2。 | Bundled `en-US`/`zh-CN` 内无。 |
| P4 | 在 OS storage 中保存/测试/轮换/删除 Alpaca Paper 与 OKX Demo，且不泄漏。 | `src-tauri/src/connections/tests.rs`；`cargo test --lib connections`。 | Section 3。 | 真实凭证按策略不记录，且为 optional。 |
| P5 | 拒绝 Live/custom endpoint，并证明每次 test 都不下单。 | `endpoint_allowlist_is_fixed_and_never_custom`、`connection_test_never_requests_an_order_endpoint`。 | Section 3。 | Fixture path 无。 |
| P6 | 完成 OKX acquisition 与 Snapshot journey。 | `adaq-data-pipeline` OKX tests 与 immutable Snapshot owner。 | Section 4。 | Live rate/provider availability 不作为 acceptance dependency。 |
| P7 | 完成 A-share provenance、unadjusted Bars、calendar、actions、quality 与 Snapshot journey。 | A-share core/pipeline tests 与 fixtures。 | Section 5。 | Actual upstream availability 作为 evidence 表示，不作假设。 |
| P8 | 完成 Alpaca/IEX U.S. equity journey 与 capability disclosure。 | Alpaca core/pipeline tests 与 fixtures。 | Section 6。 | 真实 credential path 不作为 acceptance dependency。 |
| P9 | 证明 quality states、quarantine、gaps、revisions、replay 与 deletion locks。 | Pipeline、Snapshot 和 reference-lock tests。 | Section 7。 | Committed fixture path 无。 |
| P10 | 在 market routes 之间保留一个 User-scoped asset-neutral Watchlist。 | `src/features/markets/market-workspaces.test.ts`、router/Watchlist tests。 | Section 8。 | Tested local boundary 无。 |
| P11 | 重跑 M7/M8 smoke paths。 | Full Jest/Rust workspace gates 与既有 M7/M8 guide contract。 | Section 9。 | Deterministic regression gate 不依赖人工 Provider data。 |
| P12 | M9 不包含 Features、Factors、Model training、Strategy execution、orders、Bots 或 Live trading。 | Routes/DTOs 只暴露 observation 与 evidence；connection order test。 | Section 8 和 scope statement。 | 无。 |
| P13 | 完成本地化 accessibility 与 state review。 | Router、loading、i18n 和 market tests。 | Sections 2、8、9。 | OS assistive technology 差异按平台记录。 |
| P14 | 保留 macOS ARM64、Windows x86_64 和 Linux x86_64 CI evidence。 | Matrix/release workflows 与记录的 run URLs。 | Section 10。 | 记录 exact run URL/SHA/conclusion 前该行不完整。 |
| P15 | 保留 criterion-to-evidence matrix，不能只依赖 closed issue state。 | 本节及下方 child matrix。 | Sections 1–10。 | Review 后无。 |
| P16 | 发布英文 completion comment，所有 applicable gates 通过后才关闭 child。 | Issue #76 completion comment 引用 final revision 与 commands。 | 下方 Acceptance record。 | Parent closure 是 maintainer 明确要求的独立动作。 |

### M9.1–M9.10 slice matrix

| Slice | 已交付 boundary | Focused evidence | Final acceptance evidence | 未解决风险 |
| --- | --- | --- | --- | --- |
| M9.1 / #67 | 首次绘制双语 localization、持久化、fallback、`Intl` 和 accessible shell。 | `src/lib/i18n.test.ts`、`src/bootstrap.test.ts`。 | Section 2 与 full Jest/build gates。 | 无。 |
| M9.2 / #68 | Venue/Instrument identity、IANA calendars、sessions、UTC boundaries 和 scheduled-closure semantics。 | `adaq-data-core` market tests。 | Sections 4–7 与 Rust gates。 | Supported calendar contracts 无。 |
| M9.3 / #69 | Host-owned OS secret store、固定 Paper/Demo endpoints、redaction、lifecycle 与 non-ordering tests。 | `cargo test --lib connections`。 | Section 3 与 Rust gates。 | Real OS-store prompts 按平台不同。 |
| M9.4 / #70 | Source → Canonical → Quality → Snapshot immutable evidence pipeline。 | `cargo test -p adaq-data-pipeline --lib`。 | Section 7 与 Rust gates。 | Local fixtures 无。 |
| M9.5 / #71 | Full-universe OKX Spot evidence、resumable one-minute history 与 selected realtime evidence。 | OKX pipeline/core fixture tests。 | Section 4 与 Rust gates。 | Live provider behavior 不用于 deterministic acceptance。 |
| M9.6 / #72 | `akshare-rs` A-share path、actual-upstream provenance、unadjusted Bars、actions 与 sessions。 | A-share core/pipeline fixture tests。 | Section 5 与 Rust gates。 | Provider evidence limits 已显式展示。 |
| M9.7 / #73 | Alpaca Basic/IEX path、capability disclosure、calendars、Bars 与 stream evidence。 | Alpaca core/pipeline fixture tests。 | Section 6 与 Rust gates。 | Basic-plan limitations 已显式展示。 |
| M9.8 / #74 | Point-in-Time Universes、derived intervals、quality、revisions、locks 与 Snapshots。 | Pipeline/Snapshot/reference-lock tests。 | Section 7 与 Rust gates。 | Committed fixtures 无。 |
| M9.9 / #75 | Localized four-route Markets GUI 与 one user-scoped Watchlist。 | `market-workspaces.test.ts`、`router.test.ts`、loading tests。 | Section 8 与 full Jest/build gates。 | OS visual differences 仍需人工 evidence。 |
| M9.10 / #76 | 本双语、跨平台 acceptance contract 与 evidence record。 | `src/m9-manual-acceptance.test.ts`、全部 required gates。 | Sections 1–11 与 issue comment。 | 所有 row 记录后无。 |

<!-- m9-acceptance:acceptance-record -->
## 12. 验收记录与清理

记录 reviewed revision、OS/architecture/display scale、Node/pnpm/Rust/Python versions、command outputs、focused test counts、full-gate conclusions、Provider fixture hashes、Source/Canonical/Quality/Snapshot IDs、revision 与 deletion-lock evidence、脱敏 User IDs、Route screenshots、keyboard/accessibility observations，以及 exact CI run URLs/SHA/conclusions。不要在记录中保存 credentials、tokens、private paths 或 private market data。

Fixture acceptance 后，只删除本次运行创建的 disposable profiles、临时 acquisition directories、generated package/build outputs 和 test databases。不要删除 repository fixtures 或 finalized evidence。如果某个平台仍持有 file handle，先停止拥有它的进程再清理，并记录平台结果。

只有当上面所有 applicable rows 通过、可选真实 Provider 检查已在不含 secret evidence 的情况下通过或用完整脱敏 evidence 标记 unavailable、所有 automated gates 绿色、required platform evidence 已记录且没有违反 M9 boundary 时，M9 才算 accepted。M10 Feature Engineering 以及后续 Factor、Model、Strategy、Paper、Bot、Monitoring 和 feedback milestones 仍然 out of scope。
