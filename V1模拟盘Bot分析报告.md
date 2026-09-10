# V1 模拟盘 Bot 分析报告

> 当前状态提示：下方原有 2026-09-04 内容是历史报告。当前头 2026-09-09 的真实状态见文末；当前 Bot 并非 running，Paper Account 并非 reconciled，不能把历史结论当作现在的运行结果。

报告日期：2026-09-04  
范围：ADAQ V1 / OKX Spot 研究链路 / OKX Demo Paper / Bot / Operations  
数据来源：打包 Desktop `bid.adaq.desktop` 的 Host 持久化证据与本轮界面实测

## 结论

OKX Demo 的 `50113 Invalid Sign` 已由用户修复。本轮正式对账成功，Paper Account 已恢复为 `reconciled`；Bot 已通过 Retry 创建新的 Runtime Attempt，并持续处于 `running`、Worker 心跳正常。

本轮没有成交或收益：当前 Attempt 尚无 Decision Batch；此前两个实际 Decision Batch 均为 `no-target`，对应 Host 已保留的 `market_data_context` 条件——当时没有完整的 Feature Dataset cross-section。Host 因输入不完整跳过决策，没有伪造 Target、没有绕过 Risk/OMS、没有重复提交订单。账户中保留的 4 个 Demo 订单均已取消，成交为 0。因此本报告的结论是“运行和安全门禁有效，但没有可归因的交易收益样本”，不是盈利或策略有效性结论。

## 1. 精确身份

| 项目 | 身份 / 状态 |
|---|---|
| Paper Account | `723843360829982304`，OKX Spot / USDT，`reconciled` |
| Bot | `33927f58-1783-4724-b8cf-830dcd185545` |
| 当前 Runtime Attempt | `5618cc5a-e86b-463b-8cce-db35d210c20b`，`running` |
| Strategy Qualification | `11529b4837415a230f19fd57c7633d5ff38ef1adf4940dfa8bf6db0b02041a01` |
| Strategy Candidate | `e3977b3b-af04-4ab1-8a62-c8981f5c60b9` revision 1 |
| Strategy package | archive `5495794adc23a564d37ad41d893e83d09d68ce7702df74a9ff3f9855d7c9810c`；WASM `eb7907c8052d7c4ce063afc06672e4419b8e202afcb4d9f2233cb840bb62650e` |
| Factor Component | Factor EMA 5/10 Crossover v0.1.1；WASM `380da335aefdbe4d7c7ff08cd0fa00a54619fbf571b4ba8cdadfcd2e63d1da8f` |
| Model Component | Qlib Ridge WASI Model v1.0.1；WASM `046148053122cee6848f7938dd7374e4fe25ce5aff1948d5c4be741b12c7f05e` |
| Market snapshot | `98a4621101b0019a65eb84aa9509ef0c75c199ce3b35c58767c43c33967b3c80` |
| Universe | `73c10943dd82e35d12f54151b3eb102492c89d6d542e4f1aebc43c33967b3c80`，BTC-USDT / ETH-USDT / SOL-USDT |

## 2. 策略与研究结果

因子输入为 `ema-5`、`ema-10`，输出为 `buy-signal`：第一次 EMA5 从不高于 EMA10 变为高于 EMA10 时记录交叉值；下一次再次上穿且当前 EMA5 高于前次记录值时输出买入信号，其余完整输入输出 0。当前冻结数据没有单独的 market `high` slot，所以“前期上穿高点”按交叉时 EMA5 值实现，不冒充不存在的 K 线最高价。

Factor Promotion Decision `d356cdaa-e795-4184-b8c2-219c9008bc4b` 已达到 Component Eligible，13/13 eligibility gates 通过。Strategy Qualification 为 `gate12Eligible=1`，并绑定精确 Backtest 与 Validation 证据。

采用的组合 Backtest Run 为 `portfolio-7fc86a3eb1525207e92d3b9e132f0f94a888948e8de3487f7a3bb4097c8c2a61`：

| 指标 | 结果 |
|---|---:|
| 初始资金 | 10,000 USDT |
| 最终权益 | 9,698.958479285032885672834661 USDT |
| 总收益率 | -3.01041520714967114327153389% |
| 最大回撤 | -5.56900653519571700946493329% |
| 总成本 | 18.699160620926320100857426853 USDT |
| 换手 | 1.8952064147777642084604159318 |
| 决策帧 | 5,759 |

Validation Report `e2bd87a88b73f519d887fa20535f6262369ec5457f877cc4ab2124530de511a3` 已封存；1 个窗口完成、0 个窗口失败。聚合结果：样本内平均收益约 -2.4983%，样本外平均收益约 -0.5521%，最差样本外回撤约 -4.1422%，样本外平均 Sharpe 为 0。该结果只证明流程与证据可复现，不证明策略有正向收益。

## 3. OKX Demo 账户结果

本轮 Desktop 在 Paper Trading 页面执行正式 Reconcile 并成功：

- 账户现金：`85006.29173147364` USDT。
- 持仓：OKB-USDT `100`，可卖 `100`。
- 订单：4，全部 `cancelled`，全部成交量 0。
- 成交：0。
- 保留订单分别为 3 笔 BTC-USDT Sell（限价 77560.3 / 77508.7 / 77567.7）与 1 笔 ETH-USDT Sell（限价 2435.25），数量均为 1。
- 没有把账户既有 OKB 持仓自动认领为 Bot 持仓。

本轮未再出现 HTTP 401 或 `50113 Invalid Sign`。

## 4. Bot 运行结果

当前 Attempt `5618cc5a-e86b-463b-8cce-db35d210c20b` 的启动证据顺序为：

1. `start-requested`：Host 从不可变 Bundle 创建 Runtime Attempt。
2. `account-reconciled`：启动风险前完成 OKX Demo 对账。
3. `warmup-started`：冻结管线进入 warmup。
4. `worker-heartbeat`：Worker Ready，Bot 进入并保持 `running`。

当前 Attempt：0 decisions、0 orders、0 unmanaged positions、`reconciliationRequired=false`。Bot 在验收结束时保持运行，没有为了制造结果执行 Stop 或 Flatten。

历史同一 Bot 的两个 Decision Batch：

| Attempt | Request | 结果 |
|---|---|---|
| `052d6544-3f12-4261-bca1-ac9f1fe6001f` | `accept-final-warmup-001` | `no-target` |
| `e4ce4fe5-fef9-40d5-8e3e-4cfd9ad8796d` | `accept-final-warmup-003` | `no-target` |

Operations 保留的对应关键条件为 `market_data_context`：`No complete Feature Dataset cross-section is available.`，安全动作为 `skipDecision`。这解释了没有订单的直接原因。

Gate 12 明确把 autonomous execution 排除在 V1 范围外；Start/Retry 负责监督 Worker 和生命周期，Host Decision Batch 由 Host 拥有并按调度输入触发。因此“Bot Running 但不会自行凭空生成 Decision”是当前 V1 合同，不是缺少前端定时器的 Bug。

## 5. 风险与结论

- 模拟盘累计成交样本为 0，不能计算真实 Bot realized PnL、滑点、胜率或成交延迟分布。
- 已验证的正向结论是：账户身份和对账有效；Bundle/Attempt/Worker 身份有效；生命周期和心跳有效；输入不完整时能够 `no-target` / `skipDecision`；没有重复订单或不明成交。
- 研究回测和 Validation 为负，不应把“流程通过”解释为“策略值得投入资金”。
- 当前适合继续收集新的、完整的 cross-section Decision Batch 和 Demo Fill，再在 Paper Feedback 中形成具有方向性的 Factor / Model / Strategy / Execution 报告。
- ADAQ V1 仍严格限于 OKX Demo；本报告不授予 Live Trading 或真钱交易权限。

## 6. 2026-09-09 当前头复验报告

### 当前结论

本轮自动化、研究证据、EMA5/EMA10 因子包和 cold Debug 构建均通过。直接执行二进制会在 Tao 初始化阶段 abort，但正常 LaunchServices 打开 `.app` 后进程和 WebView 均已建立；当前图形会话锁屏，无法完成可交互 UI 验收。因此没有完成本轮 Paper Reconcile、Bot 启动、调度 Decision、成交和 Paper Feedback，不能生成新的收益或策略有效性结论。

### 当前身份与状态

| 项目 | 当前事实 |
|---|---|
| EMA 因子包 | `Factor EMA 5/10 Crossover 0.1.1`，archive SHA-256 `ce8501d181d70157af92a7a6fcf4342436e75eb1a6fd07844d71a10c0a93b` |
| Bot 1 | `f029771f-5f51-4754-aa93-2d38a37edb6d`，当前 Attempt `67637f70-79f7-40f9-8cec-69edee425b6d`，`stopped` |
| Bot 2 | `33927f58-1783-4724-b8cf-830dcd185545`，当前 Attempt `5618cc5a-e86b-463b-8cce-db35d210c20b`，`faulted` |
| Paper Account | 当前 `reconciliation=required`，fills `0` |
| 当前反馈 Snapshot | `e8a36d32-602f-4e3d-9290-1feba8d9f970`，`failed`，realized observations `0/20` |
| 当前反馈 Reports | Factor / Model / Strategy / Execution 均为 `failed` |

数据库中现存的 4 个订单均已取消，成交为 0；当前没有新的 Decision、订单、成交或可计算的 Bot realized PnL。旧报告中的 `running`、`reconciled`、账户余额和历史 Invalid Sign 记录均不代表本轮当前状态。

### 研究结果与限制

现存组合回测 `portfolio-7fc86a3eb1525207e92d3b9e132f0f94a888948e8de3487f7a3bb4097c8c2a61` 的最终权益约为 9,698.958479 USDT，总收益率约 -3.0104%，最大回撤约 -5.5690%，决策帧 5,759；Validation 样本外平均收益约 -0.5521%，样本外平均 Sharpe 为 0。该结果证明研究证据可复现，不证明策略盈利。

### 产品阻塞

最终 Debug 包已经按当前源码重新构建。冷构建后直接执行二进制仍在 `NSApplication sharedApplication` 阶段 `SIGABRT`（`adaq-2026-09-09-032923.ips`），但通过 macOS LaunchServices 正常打开 `.app` 后，`adaq` 进程保持运行，日志确认 WebContent 进程、主文档和资源请求均已建立，窗口状态为 `visible=1`。本轮检查时图形会话处于 macOS 锁屏，无法通过 Accessibility/Computer Use 操作窗口；因此没有伪造 Bot 已运行或收益结果。上游 [Tauri issue #15517](https://github.com/tauri-apps/tauri/issues/15517) 仍是相关背景，但当前必须先在解锁会话中完成真实 UI 验收。

阻塞解除后的验证顺序是：恢复可绘制 Desktop → Paper Reconcile → Bot Start → 完整 cross-section Decision → Stop/Reconcile → 真实成交 → 四个 Paper Feedback reports → 仅依据 realized evidence 更新分析报告。

### 6.1 2026-09-09 后续 UI 复验与凭据规则修订

本轮重新构建并通过 macOS LaunchServices 打开 Debug `.app`，完成了以下真实 UI 操作：

- Paper Trading 页面曾返回 `OKX Demo Reconciled`，并新增 `reconcile-1788974659350` Provider evidence；但该旧实现路径读取了系统 Keychain。按用户在 2026-09-09 明确的新规则，这些 `reconcile-*` 只保留为历史诊断，不作为后续本地凭据校对的最终验收证据。
- Bot `33927f58-1783-4724-b8cf-830dcd185545` 的 `Stop · Keep position` 已成功，状态变为 `Stopped`，新增 `stopped-keep-position`，没有未托管持仓、成交或新 Decision；账户 lease 已释放。
- 该 Bot 与 `f029771f-5f51-4754-aa93-2d38a37edb6d` 的历史 Bundle 都使用旧 `adaq-bot-worker-ipc@1.0.0`，当前运行时是 `adaq-bot-worker-ipc@1.1.0`，所以没有用旧 Bundle 强行 Start；后续应通过 UI 部署带当前 Worker 绑定的新 Bot。

从现在起，所有本地 OKX Demo Account 校对只允许读取仓库根目录 `.env`，不得读取 macOS Keychain。`.env` 已被根 `.gitignore` 的 `.env` 与 `.env.*` 规则覆盖，且未被 Git 跟踪；本报告不记录 API Key、Secret、Passphrase 或其任何片段。后续本地测试还必须保持 fail-closed、仅访问 OKX Demo、只记录脱敏证据，并在提交前复核 `git status --short`、`git diff --check` 与 `git ls-files`。

因此当前报告仍不能宣称新的 Bot 运行、Decision、订单、成交或收益；下一步必须先以 `.env` 路径完成一次不读取 Keychain 的账户校对，再重新部署并启动 EMA5/EMA10 Bot。

### 6.2 Paper Reconcile 凭据边界最终确认（2026-09-09）

### 6.3 当前数据库只读复核（2026-09-09）

当前数据库已存在完整三品种 Feature Dataset `b29875b89d346b1b36390d26064635c4eb9590351c8fe1a1bebc2e74e5ab070a`，共 `16971` 行，冻结 Plan 为 `1692ddfbb91172143f158849c8b8fce087e9abbb5140ffe8a6e12f30392b8c78`，Universe 为 `universe-b7db46ad48e77f54f4bbd65e18c44ead646e512480ff8603f0470027af8e26f2`。但现有 Bot 仍绑定旧 Feature Plan `132adc…`，没有新的 Strategy Qualification 或 Bot Bundle 消费该完整 Dataset。

最新 Factor 包 `18f1fca0…` 的 Qualification 因同一 Component identity/version 冲突失败；保留的合格 Factor 包输出 `factor-value`，现有 Model 包输入 `momentum-score`。在没有明确、冻结的 Factor 输出契约前，不将两者隐式等同，也不产生订单。此次复核仅查询数据库，没有写入数据，也没有读取或输出凭据。

从本节起，Paper Reconcile 的本地测试固定使用 `local-env-credentials` Debug 构建；该 feature 在非 Debug 构建中直接编译失败。测试仅读取 Git 仓库根目录 `.env` 中的 OKX Demo 变量；缺少仓库根或变量时 fail closed，不读取 Keychain，不支持本地凭据写入/删除，也不记录任何凭据值。默认正式构建仍使用 `KeyringSecretStore`，因此正式 App 用户操作继续读取 Keychain。

本地 `.env` 已由根 `.gitignore` 覆盖且未被 Git 跟踪。默认构建和 `.env` 专用构建均通过 `cargo check`；非 Debug 构建启用该 feature 时按预期被 compile-time guard 拒绝。专用凭据测试 3 项通过，Debug 特性包构建完成，包内未包含 `.env`。显式本地验收测试 `connections::tests::local_env_paper_reconcile_against_demo_account` 已经真实完成 OKX Demo Paper Reconcile：Provider operation 为 `reconcile-1788978620179`，状态为 `reconciled`，账户 `723843360829982304`，fills 为 0、持仓为空，且未打印凭据值。此前 `.env` 专用 UI 测试得到 `reconcile-1788975831362`，仅作为早期 UI 诊断；旧的 `reconcile-1788950811378`、`reconcile-1788973893080`、`reconcile-1788974659350` 仅作为 Keychain 历史记录。

最终路径收紧后，macOS 当前仍只有进程而没有 Computer Use 可访问窗口，所以本轮没有虚构新的 Bot Deploy/Start、Decision、订单、成交或收益结论；这些仍待恢复可交互 Desktop 后按 `.env` 专用构建重新执行。

### 6.3 2026-09-09 末轮 `.env` Debug Desktop 复验

恢复可交互窗口后，最终 `local-env-credentials` Debug 包完成了真实产品复验：Paper Trading 页面显示 `OKX Demo Reconciled`，最新 Provider operation 为 `reconcile-1788997971055`；账户 `723843360829982304` 无持仓、无 fills，4 个历史订单均已取消。该过程只使用仓库根 `.env`，不读取 Keychain，也没有把凭据值写入日志、数据库或报告。

新 Bot `48f99976-83f5-42ba-8eaf-5b2c99bda524` 使用当前 Worker bundle 完成 Deploy、Start、账户对账、warmup 和 Worker heartbeat。Host Decision Batch 随后真实返回并持久化为 `no-target`，原因是 `No complete Feature Dataset cross-section is available.`；Bot 已通过 `Stop · Keep position` 安全停止，当前无 running Bot、无新订单、无成交。

这轮证明了 `.env` 凭据边界、账户身份校对、Bot 生命周期和不完整输入时的安全拒绝，但不是策略收益证明。由于当前冻结 Bot 上下文没有匹配的完整三品种 cross-section，不能伪造 Target、Fill 或 realized PnL；Paper Feedback 仍没有新的 realized 样本，四个方向性报告仍不能据此更新。
