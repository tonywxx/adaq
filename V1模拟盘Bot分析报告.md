# V1 模拟盘 Bot 分析报告

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
