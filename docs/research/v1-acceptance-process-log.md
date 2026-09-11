# ADAQ V1 自动验收过程记录（归档）

> 本文件是历史归档，不再是当前权威结论。当前权威验收摘要见根目录 [V1自动验收文档.md](../../V1自动验收文档.md)。以下内容按原始日期与上下文保留，仅供追溯。

# ADAQ V1 自动验收文档

> 历史说明：本文前半部分记录的是 2026-09-04 的旧基线验收，不能作为当前运行状态。当前权威结果见文末“2026-09-09 当前头完整验证记录”；旧章节中的 `running`、`reconciled`、账户余额和 Desktop 界面结论均按历史证据理解。

验收日期：2026-09-04  
仓库：`https://github.com/tonywxx/adaq.git`  
验收起始基线：`75750c97787235eafd01ab6323ba5ad3e712f7f1`  
Desktop：`/Users/tony/github/adaq/src-tauri/target/debug/bundle/macos/adaq.app`  
Bundle ID：`bid.adaq.desktop`  
范围：OKX Spot 数据 → Snapshot → Feature → Factor → Model → Strategy → Backtest → Validation → OKX Demo Paper → Bot → Operations → Paper Feedback

## 0. 验收结论

- GitHub 当前 open issue 为 0，open PR 为 0；本轮开始时本地 `main` 与 `origin/main` 均为 `75750c9`。
- 用户已修复 OKX Demo `50113 Invalid Sign`。本轮正式 Reconcile 成功，账户状态为 `reconciled`。
- EMA5/EMA10 Factor、Model、Strategy Candidate、Portfolio Backtest、Validation、Strategy Qualification、Paper Account、Bot Bundle 与 Runtime Attempt 的精确证据链完整。
- Bot `33927f58-1783-4724-b8cf-830dcd185545` 已 Retry，当前 Attempt `5618cc5a-e86b-463b-8cce-db35d210c20b` 保持 `running` 且心跳正常。
- 当前 Attempt 尚无新 Decision；历史两个 Decision Batch 均安全返回 `no-target`。账户保留 4 个已取消订单、0 fills，所以不能声称实际 Bot 盈利。
- 自动化检查和 debug Desktop 重建均通过。最新旧基线 CI 的 macOS job 通过、Windows job 因测试退出与临时目录清理竞态失败；本轮已修复并用完整本地回归验证，最终远端 CI 以本验收提交关联的 V1 Acceptance 结果为准。
- 本轮 Desktop 自动化已直接覆盖 en-US 的 Dashboard、Data Foundation、Feature、Factor、Paper Trading 对账与 Bot Retry。之后 Computer Use 服务连续返回 `Sky Computer Use native pipe startup failed`；未绕过工具边界使用 AppleScript。Models、Strategy、Backtest、Validation、Paper Feedback 与完整 zh-CN 的当前数据复跑由 Host 持久化证据和自动测试覆盖，仍应按本文清单人工复核界面。

## 1. 开始前检查

1. 运行 `git remote -v`，确认仓库为 `tonywxx/adaq`。
2. 运行 `git status --short --branch`，确认从干净的 `main...origin/main` 开始。
3. 比较本地 HEAD 与 `origin/main`，均为 `75750c97787235eafd01ab6323ba5ad3e712f7f1`。
4. 查询 GitHub：所有 issue 与 PR 已关闭；`#141` 的旧 Readiness 结论仍是 Not Ready，因为记录时 50113 尚未修复且 CI pending。本文件记录修复后的新事实，不改写旧评论。
5. 查询最新 V1 Acceptance `33848349136`：macOS ARM64 job 全绿；Windows x86_64 唯一失败为 `dataset_generation::tests::interface_cancellation_is_user_scoped_and_terminal_after_exit` 删除临时目录时遇到文件占用。

## 2. 自动化与构建

### 2.1 发现并修复的问题

1. Windows 测试在数据库已显示 Cancelled 后立即删除临时目录，但后台线程尚未来得及退出并释放最后的文件句柄。
2. 在共用测试等待器中，终态不仅等待持久化状态，还等待 Attempt 从线程注册表移除。
3. 该改动位于 `#[cfg(test)]`，不改变生产取消、发布或 Bot 行为。
4. 定向测试通过：1 passed。
5. `format:check` 同时发现两个既有 Python Research 文件未按 Biome 格式化；只对这两个文件执行机械格式整理，不改行为。

### 2.2 当前结果

| 检查 | 结果 |
|---|---|
| `cd src-tauri && cargo fmt --all -- --check` | 通过 |
| Windows 竞态定向 Rust test | 通过，1 passed |
| `cd src-tauri && cargo test --workspace` | 通过，584 passed，13 ignored，42 suites |
| `cd src-tauri && cargo check --workspace` | 通过，0 errors，23 个既有 warnings |
| `pnpm exec jest --watchman=false --runInBand` | 通过，47 suites，168 tests |
| `pnpm run build` | 通过，严格 TypeScript + Vite |
| `pnpm run lint` | 通过，172 files |
| `pnpm run format:check` | 通过，172 files |
| `git diff --check` | 通过 |
| Factor example `cargo test --offline --locked` | 通过 |
| `pnpm tauri build --debug` | 通过，App / DMG / updater tar.gz / signature 均生成 |

Vite 仍报告一个已有的 ineffective dynamic import 和一个 822KB chunk 警告；Rust 保留 23 个 dead-code/unused warnings。这些不是失败，也未为本轮做无关重构。

## 3. 数据与 Snapshot

1. 打开 System Dashboard，确认根路径显示 Host 全局投影而不是伪造的全绿状态。
2. 进入 Data Foundation。
3. 确认 Source Datasets 12、Canonical Datasets 12、Degraded 0、Rejected 0。
4. 确认 BTC-USDT、ETH-USDT、SOL-USDT 的 OKX 1m published backfill 各 5,760 行，quarantine 0、gaps 0。
5. 确认研究使用的市场 Snapshot 为 `98a4621101b0019a65eb84aa9509ef0c75c199ce3b35c58767c43c33967b3c80`。
6. 确认 PIT Universe 覆盖 BTC-USDT、ETH-USDT、SOL-USDT，Universe identity 为 `73c10943dd82e35d12f54151b3eb102492c89d6d542e4f1aebc43c33967b3c80`。
7. 验收点：Source/Canonical/Quality/Snapshot 身份可追溯，0 quarantine 和 0 gap 不靠前端补值。

## 4. Feature Engineering

1. 进入 Feature Engineering。
2. 确认 `Research Evidence Context Ready`，Revision 2，市场/场所为 `crypto / okx`。
3. 确认观察范围 `1787881320000 → 1788220740000`。
4. 确认 Feature Definition 为 `EMA 5/10 Crossover Features` revision 2。
5. 确认输出包含 `ema-5`、`ema-10`。
6. 确认 Feature Dataset `0e6a973fe591d808270c4ec80018a5612cef798cb4dfd1565aa7ca59ee6e74e3`。
7. 确认 Feature Plan `132adc434d6cd699284cf339f3221128fd4bb98d7e14aa5648290cf964cfca73`。
8. 验收点：Feature Dataset、Plan、Snapshot、Universe 与 Context revision 同屏可追溯。

## 5. Factor

1. 进入 Factor Lab，确认当前 Candidate `EMA 5/10 Crossover Buy Signal v0.1.1`，Candidate hash 前缀 `b952970c...`，绑定 Context Revision 2。
2. 检查输入 slots：`ema-5`、`ema-10`；输出：`buy-signal`；warmup：1。
3. 逻辑：第一次 bullish crossover 记录交叉时 EMA5；第二次 bullish crossover 且 EMA5 高于前次记录值才输出 1，其余完整输入输出 0。
4. 进入 Factor Decisions，选择当前 Component Eligible Decision `d356cdaa-e795-4184-b8c2-219c9008bc4b`。
5. 确认 Evaluation Report `54b5cae...`、Factor Dataset `0207fdb...`、Policy `368f479...`，13/13 gates 通过。
6. 确认 Component Library 中 Factor EMA 5/10 Crossover v0.1.1 已 qualified；WASM hash `380da335aefdbe4d7c7ff08cd0fa00a54619fbf571b4ba8cdadfcd2e63d1da8f`。
7. 验收点：不能用 Raw Candidate 替代 Component Eligible Decision。

## 6. Model

1. 进入 Models，选择 Qlib Ridge WASI Model v1.0.1。
2. 确认 model component ID `0c6555b8-f9dc-4db0-8257-fde994c90a5c`。
3. 确认 package archive `10b0d7d14969e1331dc8506a5e007244775daed919a9a4cf20c26b88bc8b0ccf`。
4. 确认 WASM `046148053122cee6848f7938dd7374e4fe25ce5aff1948d5c4be741b12c7f05e`，qualification evidence 为 qualified。
5. 确认 Bot Bundle 的 model input 为 `momentum-score`，输出为 `forecast`。
6. 验收点：Model Component、Signal Dataset 与 Strategy binding 使用精确 hash，不使用 latest pointer。

## 7. Strategy、Backtest 与 Validation

1. 进入 Strategy Lab，选择 Candidate `e3977b3b-af04-4ab1-8a62-c8981f5c60b9` revision 1。
2. 确认 revision hash `0fbde482dea71206ee2f2f7ddae444b7aceba00957ad9b1746584062e9e35700`。
3. 确认 Qualification Attempt `6689b5e8-449c-408d-8ef0-ee1bdd214431` 为 `ready-for-review`。
4. 确认 Strategy Qualification `11529b4837415a230f19fd57c7633d5ff38ef1adf4940dfa8bf6db0b02041a01`，`gate12Eligible=1`。
5. 进入 Backtest，打开 `portfolio-7fc86a3eb1525207e92d3b9e132f0f94a888948e8de3487f7a3bb4097c8c2a61`。
6. 核对：initial 10,000；final equity 9,698.958479285032885672834661；total return -3.0104%；max drawdown -5.5690%；total costs 18.699160620926320100857426853；5,759 决策帧。
7. 进入 Validation，打开 Report `e2bd87a88b73f519d887fa20535f6262369ec5457f877cc4ab2124530de511a3`。
8. 确认 final evidence sealed、1 completed window、0 failed window；样本外平均收益约 -0.5521%，最差样本外回撤约 -4.1422%。
9. 验收点：Qualification 精确引用上述 Backtest Run 与 Validation Report；负收益不影响流程完整性，但不能解释为策略盈利。

## 8. OKX Demo Paper Trading

1. 进入 Paper Trading；页面进入时不应自动联系 Provider。
2. 点击 Reconcile，核对风险提示后确认。
3. 本轮正式对账成功，未再出现 401 / `50113 Invalid Sign`。
4. 确认 Account `723843360829982304`，OKX Spot / USDT，状态 `reconciled`。
5. 确认 cash `85006.29173147364` USDT；OKB-USDT position 100 / sellable 100。
6. 确认 4 个 retained orders 全部 cancelled，fills 为 0。
7. 验收点：本地账户证据与 Provider 对账一致；既有 OKB 不自动归属 Bot；没有手工造订单或成交。

## 9. Bot

1. 进入 Bots，选择 Bot `33927f58-1783-4724-b8cf-830dcd185545`。
2. 核对 Bundle：Strategy Qualification、Factor/Model hashes、Snapshot、Universe、Paper Account 与 `scheduled-cross-section` schedule 均为精确绑定。
3. 旧 Attempt 因 Host restart 为 Faulted 后，点击 Retry。
4. 确认新 Attempt `5618cc5a-e86b-463b-8cce-db35d210c20b`。
5. 确认证据依次出现 `start-requested`、`account-reconciled`、`warmup-started`、`worker-heartbeat`。
6. 确认 Bot 状态 `Running`，`reconciliationRequired=false`，Pause/Stop 控件只在允许状态显示。
7. 当前 Attempt 0 decisions / 0 orders；验收结束时保留 Bot Running，不执行 Stop/Flatten。
8. 历史两个 Decision Batch 均为 `no-target`，Operations 中原因是 `market_data_context` 缺少完整 cross-section，安全动作为 `skipDecision`。
9. 验收点：缺输入不会产生 Target 或订单；重复/过期 Target 不能越过 Host Risk/OMS；V1 不包含 autonomous execution。

## 10. Operations 与 System Dashboard

1. 返回 System Dashboard，等待至少一次 15 秒自动刷新周期。
2. 确认 Paper Account 显示 reconciled、Bot 显示 running，而不是旧的 restart 缓存状态。
3. 进入 Operations，确认 execution adapter healthy、Worker heartbeat 持续更新。
4. 确认 `market_data_context` Critical alert 为 acknowledged，诊断为没有完整 Feature Dataset cross-section，`skipDecision` 可见。
5. 确认旧 `worker_stopped`、`freeze_all_requested` 为 resolved；不得把 acknowledged Critical 显示成 Healthy。
6. 验收点：Dashboard 只读；不能在 Dashboard 内对账、下单、Stop、Flatten 或改变研究结论。

## 11. Paper Feedback

1. 进入 Paper Feedback，选择当前 Bot 和当前 Runtime Attempt。
2. 使用 Attempt 创建时间到当前时间作为 observation range，realization cutoff 不早于 end。
3. Required observations 保持 20，创建 Snapshot。
4. 因当前 Attempt 0 decisions / 0 fills，预期 Evidence State 为 `notYetRealized` 或 `insufficientEvidence`，不能显示方向性结论。
5. 对 Snapshot 分别生成 Factor、Model、Strategy、Execution 四个 Report。
6. 选择四个 Report，Action 选择 `No Change`，填写理由：“当前无成交样本，保持 Bot 运行并等待完整 cross-section 与 realized evidence。”
7. 记录 Research Review Decision。
8. 验收点：Snapshot/Report/Decision 均为不可变、User-scoped；Review Decision 不会自动改 Bot、下单、训练或部署。

## 12. zh-CN 完整人工复跑

1. Settings → General → Interface language，切换为 `简体中文`。
2. 从 `/` 开始按本文第 3–11 节完整重复一次。
3. 核对系统仪表盘、数据基础、特征工程、因子研究、模型、策略实验室、回测、验证、模拟交易、Bot 管理、运行仪表盘、模拟反馈的标题、状态、按钮、错误与 empty state 均为中文。
4. Reconcile 为真实 Provider 操作；如账户已是 reconciled，可再次明确确认后执行，结果不得丢失已有证据。
5. 不重复 Deploy 新 Bot；复核当前 Running Bot 与 Bundle/Attempt identities。
6. Paper Feedback 若已有相同 observation range 的记录，不删除、不改写，只查看既有不可变记录。
7. 最终切回需要的界面语言；语言切换不得改变 Host 证据或 Bot 状态。

## 13. 最终人工判定清单

- [ ] Data Foundation 12/12，0 degraded，0 rejected；三市场 1m 各 5,760 行。
- [ ] Snapshot / Universe / Feature Context identities 与本文一致。
- [ ] EMA Factor 当前 Decision 为 Component Eligible 13/13。
- [ ] Qlib Ridge Model qualified，hash 与 Bundle 一致。
- [ ] Strategy Qualification 精确绑定 Backtest 与 Validation。
- [ ] Backtest/Validation 指标显示负结果，不被 UI 粉饰为盈利。
- [ ] OKX Demo Reconcile 成功，无 50113。
- [ ] Paper Account cash/position/orders/fills 与本文一致。
- [ ] Bot 当前 Attempt Running，心跳更新，未要求 reconciliation。
- [ ] 缺 cross-section 时为 no-target / skipDecision，没有订单。
- [ ] Operations 与 Dashboard 在刷新后显示当前状态且保持只读。
- [ ] Paper Feedback 能生成四 lens Report，并保持非方向性结论。
- [ ] en-US 与 zh-CN 全流程无空白、崩溃、越权按钮或伪造 Healthy。
- [ ] 本地自动检查、debug bundle 和最终 GitHub V1 Acceptance 全部通过。

Bot 的运行、账户、订单、回测与风险解读见 [V1模拟盘Bot分析报告.md](../../V1模拟盘Bot分析报告.md)。

## 14. 2026-09-09 当前头完整验证记录（当前权威）

### 14.1 验证边界与目标

- 验证日期：2026-09-09。
- 当前 HEAD 与 `origin/main`：`2976f1f fix: improve tooltip collision avoidance in test environment`。
- 仓库：`https://github.com/tonywxx/adaq.git`；当前 GitHub open issue 为 0。
- Desktop 目标：`bid.adaq.desktop`，最终 Debug 包为 `/Users/tony/github/adaq/src-tauri/target/debug/bundle/macos/adaq.app`。
- 本轮只读检查本地数据库，不直接改数据库、不伪造 Bot 运行结果或成交结果。

本轮使用的最小因子定义如下：输入为 `ema-5` 与 `ema-10`。第一次 `EMA5` 从不高于 `EMA10` 变为高于 `EMA10` 时，记录当时的 `EMA5`；第二次再次上穿，且当前 `EMA5` 严格高于前次记录值时输出 `buy-signal=1`，否则输出 `0`。当前冻结输入没有独立的 K 线 `high` slot，因此“前期上穿高点”按前次上穿时的 `EMA5` 值实现，并在文档中明确，不冒充不存在的市场最高价。

### 14.2 第 0 步：GitHub、HEAD 与 CI

1. `git remote -v` 确认远端为 `tonywxx/adaq`。
2. 本地 `main` 与 `origin/main` 均为 `2976f1f`；开始时工作树干净。
3. GitHub open issue 为 0，所有历史 issue 均已关闭。
4. 最新 `V1 Acceptance` run `34286670673` 在当前 HEAD 上为 `completed / success`。

结论：代码交付面和 V1 Acceptance CI 通过。CI 通过不等价于 Desktop 产品运行通过，后续仍必须执行本地打包和产品运行验证。

### 14.3 第 1 步：发现格式问题、最小修复、重新 build 与复验

第一次执行 `pnpm run format:check` 发现 3 个前端文件存在格式漂移：

- `src/features/data-foundation/data-foundation-page.test.tsx`
- `src/features/data-foundation/data-foundation-page.tsx`
- `src/features/factors/factors-page.tsx`

只执行了机械格式整理，没有改变业务逻辑。修复后重新执行格式检查、前端构建、测试和 Desktop Debug 构建；所有通过。

| 检查 | 当前结果 |
|---|---|
| `pnpm run format:check` | 通过，172 files |
| `pnpm run lint` | 通过，172 files |
| `pnpm run build` | 通过，TypeScript strict + Vite 4,767 modules |
| `pnpm exec jest --watchman=false --runInBand` | 通过，47 suites / 167 tests |
| `cd src-tauri && cargo fmt --all -- --check` | 通过 |
| `cd src-tauri && cargo check --workspace` | 通过，0 errors |
| `cd src-tauri && cargo test --workspace` | 通过，598 passed / 13 ignored / 42 suites；`cargo clean` 后冷回归 372.33s |
| `git diff --check` | 通过 |
| `pnpm tauri build --debug` | 通过，重新生成 `.app`、`.dmg` 与 updater archive |

构建期间仅有既有 Rust dead-code、Vite 动态 import 和大 chunk 警告，没有编译错误。临时尝试过 `base: "./"` 以排查白屏，但重新构建并启动后现象不变，已回滚该实验配置；最终源码没有保留它。

### 14.4 第 2 步：因子包与运行时

1. `examples/components/factor-ema-crossover` 构建通过。
2. Factor 包验证通过：`Factor EMA 5/10 Crossover 0.1.1`，archive SHA-256 为 `ce8501d181d70157af92a7a6fcf4342436e75f75eb1a6fd07844d71a10c0a93b`。
3. 当前 Bot deployment bundle 中的 factor pipeline archive hash 与该 EMA 因子包一致。
4. Worker warmup → target 定向测试通过：1 passed。
5. Paper reservation/partial-fill、provider order update 和 supervisor failure 定向测试分别通过：1 passed、1 passed、6 passed。
6. `adaq-component` tooling 构建、Factor/Strategy 公共示例 build + verify 均通过；Indicator、Feature、Factor generated checks 均通过。

结论：指定 EMA 因子的定义、包身份、WASM/组件装载路径和 warmup/target 运行时证据通过；没有发现需要修改因子逻辑的 bug。

### 14.5 第 3 步：数据、研究、回测与验证证据

- Python SDK/managed wheel contract：4 passed。
- Python runner contract：5 passed。
- Tutorial parity：`fixture=1 projects=3 bilingual_docs=4 archives=0`。
- Python `compileall`：通过。
- 当前数据库已有完整的 Data Foundation、Feature、Factor、Model、Strategy、Backtest、Validation、Qualification 持久化记录；这些是历史/现存证据，不替代本轮 Desktop 重放。
- 组合回测 `portfolio-7fc86a3eb1525207e92d3b9e132f0f94a888948e8de3487f7a3bb4097c8c2a61`：初始 10,000 USDT，最终权益约 9,698.958479 USDT，总收益率约 -3.0104%，最大回撤约 -5.5690%，总成本约 18.6992 USDT，决策帧 5,759。
- Validation `e2bd87a88b73f519d887fa20535f6262369ec5457f877cc4ab2124530de511a3`：1 个窗口完成、0 个失败；样本内平均收益约 -2.4983%，样本外平均收益约 -0.5521%，最差样本外回撤约 -4.1422%，样本外平均 Sharpe 为 0。

结论：研究链路的可复现证据存在且负结果被保留；这不是策略盈利证明，也不能替代模拟盘成交样本。

### 14.6 第 4 步：Paper Account 与连接状态

本轮通过 read-only DB 检查当前状态：

- OKX Demo connection profile 的最近一次连接测试证据为 success，声明 capabilities 为 read/trade/simulated；这不是本轮新的正式 Reconcile。
- 当前 Paper Account 的 `reconciliation` 为 `required`，账户现金字段当前约为 `98084.28951395709` USDT，持仓为空，fills 为 0。
- 账户中保留 4 个历史订单，均为 `cancelled`，不能计为成交。
- 旧记录中的 `50113 Invalid Sign` 属于历史 flatten/订单证据；本轮没有把它误写成当前新错误，也没有在数据库中直接修正它。

结论：当前不能宣称 Paper Account 已完成本轮对账；需要 Desktop 恢复后从 Paper Trading 页面重新执行并记录正式 Reconcile。

### 14.7 第 5 步：Desktop 产品运行

1. 按既定规则只执行了一次 `pnpm tauri dev`；进程完成编译并启动，但 Computer Use 无法发现可操作的 Desktop 窗口。
2. 重新生成并启动最终 Debug `adaq.app`；macOS 的 CGWindow 可看到窗口和 traffic lights，但内容是纯白 WebView，没有 Dashboard 或路由内容。
3. 通过独立 `pnpm dev` 供给前端后再次启动，仍为纯白；临时改用相对 `base` 并重建也没有改变现象，随后已回滚。
4. 当前环境为 macOS 26.6.2 arm64、Tauri 2.11.5、Wry 0.55.1、Tao 0.35.3。上游 [Tauri issue #15517](https://github.com/tauri-apps/tauri/issues/15517) 描述了同一 macOS 26 + Tao 0.35.3 家族的 `did_finish_launching` 崩溃/白屏、WebView 不启动现象；这与本轮“有原生窗口但无前端内容”的症状相符。
5. 为确认不是日志缺失，直接启动最终未修改依赖的打包二进制并设置 `RUST_BACKTRACE=1 RUST_LOG=debug`；macOS 生成诊断报告 `~/Library/Logs/DiagnosticReports/adaq-2026-09-09-032225.ips`，结果为 `EXC_CRASH / SIGABRT`。主线程停在 `___RegisterApplication_block_invoke → _RegisterApplication → GetCurrentProcess → NSApplication sharedApplication → tao::platform_impl::platform::event_loop::EventLoop::new`，发生在 WebView 创建之前，因此前端路由、Vite `base` 和 React 页面尚未执行。
6. 做了一个最小、可回滚的上游相邻实验：仅把 Tao 0.35.3 已有的 `NSAutoreleasePool` 创建位置移到 `NSApplication sharedApplication` 之前；临时 Cargo patch 能通过 Rust 检查、Debug 构建，但实验包生成的 `adaq-2026-09-09-031716.ips` 仍为同一 `SIGABRT` 诊断栈，不能作为修复。
7. 实验完成后已移除临时 `[patch.crates-io]`、恢复 `Cargo.lock`，并已重新用未修改依赖的当前源码构建最终 `.app/.dmg`；仓库不保留该外部依赖覆盖。
8. 查到上游 #15517 的最新跟进建议先排除增量缓存；执行 `cargo clean` 清除 192,921 个生成文件（117.7 GiB），然后从零完成 cold build。冷构建包直接执行二进制仍生成 `adaq-2026-09-09-032923.ips` 并在 `NSApplication sharedApplication` 阶段 `SIGABRT`，因此该直接执行路径不能作为产品启动方式。
9. 改用 macOS 正常 LaunchServices 启动：`open -a .../adaq.app`。进程 `adaq` 保持存活；`sample` 显示主线程已进入 `NSApplication run`，统一日志显示 WebContent 进程 `16597`、主文档完成加载、资源请求返回 `200`，且 `window visible 1`。这证明正常 `.app` 启动已经越过前述直接二进制 abort 并创建 WebView。
10. 进行 UI 检查时显示器再次进入睡眠；唤醒后截图显示 macOS 登录锁屏，Accessibility/Computer Use 枚举不到应用窗口。未绕过登录凭据，也未把“进程和 WebView 已创建”误写成“页面已人工可操作”；Paper Reconcile、Bot 和反馈报告仍未执行。
11. 本轮恢复检查时，截图再次显示显示器睡眠/黑屏，冷构建 `.app` 进程仍存活（PID `16593`）；由于图形会话仍未解锁，没有重启进程、伪造点击或直接修改 Paper/Bot 数据。

结论：直接执行二进制的 abort 已被确认不代表正常 `.app` 启动结果；正常 LaunchServices 路径已创建 WebView，但本轮仍未完成可交互 Desktop 验收，因为当前图形会话处于锁屏。没有伪造 UI 通过，也没有保留未经验证的 workaround。

### 14.8 第 6 步：Bot、调度与真实结果

当前数据库真实状态：

| Bot | 当前 Attempt | 当前状态 |
|---|---|---|
| `f029771f-5f51-4754-aa93-2d38a37edb6d` | `67637f70-79f7-40f9-8cec-69edee425b6d` | `stopped` |
| `33927f58-1783-4724-b8cf-830dcd185545` | `5618cc5a-e86b-463b-8cce-db35d210c20b` | `faulted` |

当前没有 `running` Bot；当前 Attempt 没有新的 Decision 或订单。由于当前图形会话锁屏，无法从 UI 完成“Paper Reconcile → Start/Retry → 调度 Decision → Stop/Reconcile”的产品闭环，因此不能声称有新 Bot 结果、收益或运行心跳。旧 Attempt 的错误与旧文档中的 running/reconciled 状态全部保留为历史，不覆盖当前数据库事实。

### 14.9 第 7 步：Paper Feedback 与分析报告

- 当前 Snapshot `e8a36d32-602f-4e3d-9290-1feba8d9f970` 为 `failed`，required observations 为 20，realized observations 为 0。
- 当前 Factor、Model、Strategy、Execution 四个 Report 均为 `failed`；分别缺少可配对的 factor output、model target、历史账户估值序列或成交确认样本。
- 当前执行统计为 0 orders、0 fills、0 decision batches，risk decisions 为 7。

结论：本轮没有可归因的 Bot PnL、胜率、滑点或方向性反馈，不能生成“策略有效”分析。`V1模拟盘Bot分析报告.md` 已追加当前事实，并明确旧 2026-09-04 报告为历史记录。

### 14.10 当前验收清单

- ✅ 当前 HEAD、GitHub issue 状态、V1 Acceptance CI：通过。
- ✅ 前端、Rust、Python、组件工具、生成代码与完整测试：通过。
- ✅ EMA5/EMA10 因子包、identity、warmup/target 运行时：通过。
- ✅ 研究、回测、验证持久化证据：通过，且负结果未被粉饰。
- ✅ 最终源码对应的 Debug `.app/.dmg` 构建：通过。
- ⏸ Desktop 产品运行：直接执行二进制为 `SIGABRT`；正常 `.app` 启动已创建 WebView，但锁屏导致当前没有可交互窗口证据。
- ❌ 本轮 Paper Account 正式 Reconcile：未完成，当前为 `required`。
- ❌ 本轮 Bot running、Decision、订单、成交：未完成，当前 Bot 为 `stopped`/`faulted`。
- ❌ 本轮 Paper Feedback 方向性报告：未完成，当前样本为 0 且四个 Report 为 `failed`。

因此，V1 的自动化和构建验收通过，但 V1 **尚未完成最终产品验收**，不能宣称“全部功能已通过”或“Bot 已在模拟盘运行并产生结果”。

### 14.11 阻塞解除后的唯一复验顺序

1. 在已解锁的图形会话中重新打开当前 cold-built `.app`，确认窗口、Dashboard 和路由内容可见并可操作。
2. 在 en-US 完成 Paper Trading 正式 Reconcile，记录账户/持仓/订单/fills；随后按已有第 3–11 节完成产品链路，再按第 12 节复跑 zh-CN。
3. 启动当前 EMA 因子绑定的 Bot，等待完整 cross-section 调度 Decision；若风险门禁允许才观察订单/成交。
4. Stop 后再次 Reconcile，创建 Paper Feedback Snapshot，生成四个 lens reports，并仅基于真实 realized evidence 生成分析结论。

在上述 Desktop 阻塞解除前，继续重复构建不会产生新的产品证据；本轮已完成能安全完成的自动化、包构建、数据库审计和根因定位。

### 14.12 本地 OKX Demo 凭据规则（2026-09-09，用户明确要求）

从本节记录之后，所有本地 OKX Demo Account 校对都不得读取 macOS Keychain。仅允许使用仓库根目录 `.env` 中由用户提供的 OKX Demo API Key、Secret 和 Passphrase；这些值只服务于本机测试，不得进入公共仓库、文档、日志、命令输出、前端状态、SQLite 或 GitHub artifact。

本次安全检查只确认了文件与 Git 状态，没有读取或输出任何凭据值：

1. `.env` 文件存在。
2. 根 `.gitignore` 已覆盖 `.env` 与 `.env.*`。
3. `git ls-files` 未发现 `.env`、`.env.local`、`.env.development` 或 `.env.test` 被跟踪。
4. 后续提交前仍需执行 `git status --short`、`git diff --check` 与 `git ls-files` 检查；不得用 `git add -f`、复制、上传或在报告中粘贴敏感值。

下一次正式校对的凭据边界必须满足：

1. 本地测试进程启动时读取 `.env`，缺失变量立即 fail closed，不回退到 Keychain。
2. 凭据只在本地 Host 测试调用链内使用；不进入 React、命令行参数、日志、持久化证据或验收截图。
3. 请求只允许固定 OKX Demo endpoint 和账户读取/校对操作，不触碰 Live endpoint 或真实资金。
4. 验收证据只能记录脱敏后的账户 ID、状态、Provider operation ID 与余额/持仓摘要。

此前界面中看到的 `reconcile-1788950811378`、`reconcile-1788973893080` 和 `reconcile-1788974659350` 是旧实现通过系统 Keychain 完成的历史 UI 证据；按本条新规则，它们不作为新的本地凭据校对验收证据，后续必须用 `.env` 路径重新执行。

### 14.13 旧 Worker Bundle 的安全停止修复

本轮 UI 验证发现两个历史 Bot Bundle 持有 `adaq-bot-worker-ipc@1.0.0`，当前运行时要求 `adaq-bot-worker-ipc@1.1.0`。严格校验仍保留在 Deploy、Start、Transition 和 Fault 路径；只有 Host 已判定为故障后的 `complete_stop` 使用 feedback 校验，使旧 Bundle 能安全落为 `Stopped` 并释放账户 lease，不能借此启动旧 Worker。

- `complete_stop_releases_lease_for_legacy_worker_binding`：1 passed。
- `bot_operations::tests`：15 passed。
- `pnpm tauri build --debug`：重新生成 Debug `.app`、`.dmg` 与 updater archive。
- 真实 UI 中 Bot `33927f58-1783-4724-b8cf-830dcd185545` 已从 `Faulted` 变为 `Stopped`，新增 `stopped-keep-position` 证据；没有未托管持仓或新的成交。

该修复只解除旧故障实例的安全收尾阻塞；要继续 EMA5/EMA10 Bot 运行验收，仍需先完成不读取 Keychain 的 `.env` 校对入口，再通过 UI 部署带当前 Worker 绑定的新 Bot。

### 14.14 Paper Reconcile 的本地测试与正式应用凭据边界（2026-09-09，最终确认）

已确认并落实以下长期规则：

1. 本地 Paper Reconcile 测试只使用启用 `local-env-credentials` feature 的 Debug 构建；该 feature 在非 Debug 构建中直接编译失败。Debug 构建只定位 Git 仓库根目录的 `.env`，读取 `OKX_DEMO_API_KEY`、`OKX_DEMO_API_SECRET`、`OKX_DEMO_API_PASSPHRASE`，缺少仓库根或任一非空变量立即 fail closed。
2. 该本地凭据 store 是只读的，不支持写入或删除，也不回退到 Keychain；`local-env-credentials` 构建不编译 Keyring store 实现。
3. 正式应用和正式用户操作继续使用默认构建；默认 `ConnectionManager::open_production` 仍注入 `KeyringSecretStore`，因此正式用户凭据路径不变。
4. 本地 `.env` 只用于本机测试，凭据不进入 React、命令行、日志、SQLite、文档、截图、GitHub artifact 或公共仓库。

本轮验证：

- 默认 `cargo check`：0 errors；`local-env-credentials` `cargo check`：0 errors。
- `cargo check --release --features local-env-credentials` 按预期被 compile-time guard 拒绝，确保 `.env` feature 不能进入非 Debug 包。
- 默认 secret-store tests：1 passed；`connections::secret_store::local_env_tests`：3 passed；覆盖注释/引号解析、缺少变量时 fail closed，以及实际从仓库根 `.env` 加载但不写入/打印凭据。原先可用 `--ignored` 触发 Keychain 的手动测试入口已删除。
- `pnpm tauri build --debug -- --features local-env-credentials`：Debug `.app`、`.dmg` 和 updater archive 构建完成。
- 包内未发现 `.env`；特性包字符串检查只出现变量名和脱敏错误文本，没有凭据值；`git ls-files` 与工作树均未发现被跟踪或待提交的 `.env` 文件。
- `connections::tests::local_env_paper_reconcile_against_demo_account` 在显式本地验收开关下通过：只经仓库根 `.env` 路径完成真实 OKX Demo Paper Reconcile，Provider operation 为 `reconcile-1788978620179`，数据库状态为 `reconciled`，账户 `723843360829982304`，fills 为 0、持仓为空；测试未打印凭据值。该命令因访问本机应用数据库在沙箱外运行，未改变凭据来源。
- 在此前可交互的 `.env` 专用测试包中，Paper Trading 返回 `OKX Demo Reconciled`，Provider operation 为 `reconcile-1788975831362`；它仍保留作早期 UI 诊断。本次最终路径收紧后的包已完成同等编译/单测/包检查，但 macOS 当前无可访问窗口，未把无法复验的 UI 操作写成新证据。

此前 `reconcile-1788950811378`、`reconcile-1788973893080`、`reconcile-1788974659350` 均标记为 Keychain 历史证据，不覆盖本条规则。后续本地校对必须使用 `.env` 专用构建；正式用户流程则继续使用 Keychain。

### 14.15 2026-09-09 末轮 `.env` Debug Desktop 验收（当前权威状态）

本轮以 macOS LaunchServices 打开刚完成构建的 `local-env-credentials` Debug `.app`，确认最终包不含临时验收入口；未读取、打印或记录任何凭据值。

- Paper Trading 页面通过 `.env` 专用 Debug 路径完成正式 Reconcile，界面显示 `OKX Demo Reconciled`，最新 Provider operation 为 `reconcile-1788997971055`，账户 `723843360829982304`；当前无持仓、无 fills，保留的 4 个历史订单均为 `cancelled`。
- 新部署的 Bot `48f99976-83f5-42ba-8eaf-5b2c99bda524` 使用当前 Worker bundle，真实完成 Deploy、Start、`account-reconciled`、`warmup-started` 和 `worker-heartbeat`；当前数据库状态为 `stopped`，Attempt 为 `3e1cc5b7-2c0d-4d83-a8e4-31ab554e02b3`。
- Host Decision Batch 通过真实链路执行并持久化为 `no-target`，证据为 `decision-batch-unavailable`，原因是 `No complete Feature Dataset cross-section is available.`；没有伪造 Target、订单或成交。
- 随后使用 `Stop · Keep position` 安全停止 Bot；重新打开 App 后账户曾按重启规则进入 `Reconciliation Required`，再次从 Paper Trading 页面完成 Reconcile 后恢复为 `Reconciled`。最终没有 running Bot，也没有新的订单或成交。

本轮新增验收状态：

- ✅ `.env` 本地凭据边界、Debug feature guard、真实 OKX Demo Reconcile。
- ✅ 只读复核发现当前已有完整三品种 Feature Dataset：`b29875b89d346b1b36390d26064635c4eb9590351c8fe1a1bebc2e74e5ab070a`，`16971` 行，Plan `1692ddfbb91172143f158849c8b8fce087e9abbb5140ffe8a6e12f30392b8c78`，Universe `universe-b7db46ad48e77f54f4bbd65e18c44ead646e512480ff8603f0470027af8e26f2`。
- ✅ Worker Bundle 已冻结真实 Feature 输入列，并显式冻结 Strategy slot 到 Factor/Model 输出的映射；`cargo test --workspace` 通过 `601 passed / 12 ignored / 42 suites`，`cargo check --workspace` 为 0 errors，`local-env-credentials` Debug 包已重建。
- ✅ 可交互 Desktop、Paper Trading 页面、Bot Deploy/Start/Heartbeat/Stop 生命周期。
- ✅ 输入不完整时 Host 的 `no-target` 安全门禁。
- ❌ 该完整 Dataset 尚未被新的 Strategy Qualification/Bot Bundle 消费；现有 Bot 仍冻结旧 Feature Plan `132adc…`。最新 Factor 包 `18f1fca0…` 因同一 Component identity/version 被拒绝，现有合格 Factor 输出为 `factor-value`，现有 Model 输入要求 `momentum-score`，不能猜测映射后下单。
- ❌ V1 最终的 Target → Order → Fill 闭环仍未完成；当前 Bot 的不可变上下文没有匹配的完整三品种 Feature Dataset cross-section，不能安全地产生交易。
- ❌ Paper Feedback 仍没有新的 realized 样本；Factor / Model / Strategy / Execution 报告不能据此生成新的方向性结论。

本次重建后的 Debug Desktop 进程可以启动，但 macOS 当前仍返回 `cgWindowNotFound`，无法取得可交互窗口；因此没有把新的 UI 操作或 Target/Order/Fill 结果写成验收证据。代码与包校验不能替代这条最终产品证据链。

因此，V1 的自动化、构建、`.env` Reconcile、Desktop 和 Bot 安全生命周期证据已通过，但 V1 **仍未完成最终验收**。剩余唯一产品证据链是：为冻结上下文准备完整 current cross-section → 重新 Qualification/Deploy → Host Decision → 若风险门禁允许则真实 Demo Order/Fill → Stop/Reconcile → Paper Feedback reports。
