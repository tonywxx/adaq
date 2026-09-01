# V1 模拟盘 Bot 分析报告

报告日期：2026-09-01
范围：ADAQ V1 / Issue #159 / OKX Demo simulated account
结论：当前头已完成 EMA 因子和 Strategy Qualification，但 OKX Demo 正式对账返回 HTTP 401 / `50113 Invalid Sign`；因此没有创建账户或 Bot，也没有伪造收益、订单或成交结果。

## 1. 当前头结果

| 项目 | 实际结果 |
|---|---|
| EMA Factor | 当前 Component eligible；输出 `buy-signal` |
| Strategy Candidate Revision | `e3977b3b-af04-4ab1-8a62-c8981f5c60b9` |
| Strategy Qualification | `11529b4837415a230f19fd57c7633d5ff38ef1adf4940dfa8bf6db0b02041a01`，`gate12Eligible=1`，`gate12ContinuationRequired=0` |
| Paper Account | 未创建；没有保留的 OKX Demo 对账账户证据 |
| Bot deployment / runtime | 未创建 / 未运行 |
| Orders / Fills / Positions / PnL | 无记录 |
| Provider side effect | 未提交订单；没有写入 env，也没有改变认证逻辑 |

## 2. Desktop 实测过程

使用精确打包 App `/Users/tony/github/adaq/src-tauri/target/debug/bundle/macos/adaq.app`（`bid.adaq.desktop`），复用本次已授权的同一个进程。没有增加 Keychain 缓存或密钥文件；重启后的本次测试没有再次弹出授权提示。

在中文 `模拟交易` 页面点击“对账”并确认后，Host 正式 reconciliation 返回：

`HTTP 401: {"msg":"Invalid Sign","code":"50113"}`

页面保留 fail-closed 结果：“对账失败。保留的证据未改变。OKX Demo 未返回已对账的账户证据。”没有账户证据就没有继续创建 Paper Account 或 Bot。

## 3. EMA 证据链

- 因子：`b0b5b98f-7e79-4c83-b5f7-7e654f3fbec8`，第一次 EMA5 上穿 EMA10 记录 EMA5，第二次上穿且超过前次记录值才输出 `buy-signal`。
- Candidate Revision：`e3977b3b-af04-4ab1-8a62-c8981f5c60b9`。
- Qualification Attempt：`6689b5e8-449c-408d-8ef0-ee1bdd214431`，完成 9 个参数网格 portfolio backtest 后进入 `ready-for-review`。
- Qualification 已通过 UI 的明确证据复核创建；这不等于 Paper Account 或 Bot 已创建。

## 4. 阻塞与安全边界

连接测试曾能读到 Demo 配置/余额，但正式 pending-order reconciliation 仍得到 `50113`，所以不能把连接测试成功当成可对账账户证据。对 pending endpoint 加 `instType=SPOT` 的 Host 请求和订单解析已有本地 mock 覆盖；同凭据的 history 诊断也返回同样的 401，正式代码已恢复 pending 路径。

没有通过本地假数据、历史无关 Strategy、Raw Candidate、env 密钥或认证绕过来启动 Bot。下一步前提是用户重新验证有效的 OKX Demo 私有接口凭据/环境；对账成功后，才可继续创建 Paper Account、部署 Bot，并分析 runtime、decision、order、fill、position、risk 和 PnL。

以下保留 2026-08-31 的历史研究记录，不能作为当前 Bot 已运行的证据。

## 历史记录：2026-08-31 运行结果

| 项目 | 实际结果 |
|---|---|
| Bot deployment | 未创建，`bots = 0` |
| Bot runtime attempt | 无可运行 Attempt |
| Paper Account | 未创建，`paper_accounts = 0` |
| Strategy Candidate | 未创建，`strategy_candidates = 0` |
| Strategy Qualification | 未创建，`strategy_qualifications = 0` |
| Orders / Fills / Positions / PnL | 没有 Bot 或账户，因此没有可分析记录 |
| Provider side effect | 未提交订单，未改变凭据或账户 |

Desktop Bot 页面实际显示：

- Strategy Qualification：`Select an eligible Qualification` / `选择可用的 Qualification`。
- OKX Demo connection selector 未形成可部署的精确绑定。
- `Deploy` / `部署` 按钮 disabled。
- `No Bots have been deployed.` / `尚未部署 Bot。`

## 2. 为什么没有运行

用户要求的 EMA 因子真实走完了 Factor 链路，但最终 Promotion Decision 是 `Rejected`，没有当前 Component Eligible Decision。Strategy Lab 同时要求：

1. 一个已接受的 Factor Component output；
2. 一个已接受的 Model qualification；
3. 绑定到这两类证据的 Strategy Candidate；
4. 后续 Gate 11 Strategy Qualification；
5. 精确验证过身份和账户状态的 OKX Demo Paper Account。

本次记录中第一项不成立，Strategy Candidate 和 Qualification 也都是 0。用 Raw Candidate、历史 unrelated Strategy 或伪造的账户证据绕过这个门禁会破坏 ADAQ 的 Host authority、可复现性和资金安全边界，因此没有这样做。

## 3. EMA 因子研究结果

### 3.1 定义

- 输入：冻结的 `ema-5`、`ema-10` Feature Slots。
- 事件：EMA5 从不高于 EMA10 变为高于 EMA10 的 bullish crossover。
- 第一次事件：记录当时 EMA5 值。
- 下一次事件：若当前 EMA5 高于已记录值，输出 `buy-signal = 1`。
- 其他行：输出 `0`；第一条交付行因 warmup 无输出。
- 边界：当前数据没有单独的 market `high`，所以记录值是 EMA5 crossover value，不是未提供的 high。

### 3.2 证据链

| 阶段 | Attempt / 结果 | 状态 |
|---|---|---|
| Candidate build | `52d32782-8000-42ee-83a8-1f8918f070f2` → `ca1c448b5c59cd55ba25ae61798df37bec58a60b3fc98a8d6e8aae913e8df829` | completed 1/1 |
| Factor Dataset materialization | `417daa2a-32fc-43d4-bbf4-c5d499d10791` → `748e01682169a69e6a9df2dc9b65664a0f335c5c134ff08a06f0ab514b1ce9d1` | completed 18/18 |
| Factor Evaluation | `5835d9cf-3a15-471a-bd6f-332c8655fc7b` → Report `553113b5d23b26ace299fbe3cc8a740bdbabd5b2685a0a0e9701589bdee389b9` | completed 1/1 |
| Promotion Decision | `17c1d3a8-2767-4df6-a54a-e94574a629ff` / hash `7aa2ead700a0243bdc12aee126acd1f7c1be74941a0a1e29185b7c9df7d893c9` | Rejected |

冻结 Context：Revision `3`，市场 `crypto`、场所 `okx`，观察范围 `1787881320000 → 1787882400000`，Snapshot `c4e2fe2e4eaaa5758ff7934cd479716668bb349ac4050cc48c9d050e5ea0eb53`，Feature Dataset `24987d6e7385a83800ddbf79248cf54aeee6d0c427f1ede7aa1d3b836c614310`，Feature Plan `f38b6a57d9e4ff1ee88225ce06a29afd68aaf70fa47810478e498b56b8770afe`，PIT Universe `universe-42a3cb9e36e30c96ee58ca13f2b17ce67551229e65d19d65af848fe057f5cc2e`。

### 3.3 Evaluation 解读

Evaluation 报告为 `out-of-sample`，但有效统计条件不足：

- coverage 的 available sample count 为 9，值为 0；
- missingness sample count 为 9，值为 1；
- sample-count 为 0；
- IC、rank-IC、stability：`insufficient-samples`；
- economic、turnover、decay：`no-eligible-observations`。

Promotion policy 要求最低样本数 30，并要求 `cross-sectional` 与 `economic` lenses。Eligibility 结果为：

- 通过：complete lineage、out-of-sample report、complete provenance；
- 失败：required lenses、minimum coverage、minimum samples、Holm-adjusted significance、subperiod sign consistency、cost-aware outcome、complete source provenance、deterministic execution、ABI v2 expressible、buildable。

因此 `Rejected / Not eligible` 是由证据不足和 Component eligibility 门控共同得出的真实结果，不是 UI 误报，也不是运行时故障。

## 4. 后续可运行条件

要在新的、用户明确批准的运行中安全启动模拟盘 Bot，需要先补齐：

1. 足够的真实冻结研究覆盖，至少满足当前 policy 的 30 个样本和 required lenses；
2. 完整 source provenance、deterministic execution、ABI v2 expressibility 和 buildable 证据；
3. 重新 Evaluation 并得到 Component Eligible Decision；
4. 用 accepted Factor Component 与 accepted Model qualification 创建 Strategy Candidate；
5. 完成 Gate 11 Strategy Qualification、Backtest 和按时间顺序的留出证据；
6. 重新验证 OKX Demo 账户身份、原生 USDT 账户证据和 reconciliation 状态；
7. 再由 Host 创建 Bot Deployment Bundle，并单独记录 runtime、decision、order、fill、position、risk 和 PnL 分析。

在这些条件满足前，保持没有 Bot、没有 Paper Account、没有订单的状态是正确的安全结果。
