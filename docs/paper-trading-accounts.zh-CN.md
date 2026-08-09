# Paper Trading Account 与 Portfolio 用户指南

[English](./paper-trading-accounts.md)

状态：V1 用户与证据契约。

相关指南：[Paper Provider 连接与 Credential](./paper-connections.zh-CN.md)、[Trading Bot 运行时](./bot-runtime.zh-CN.md) 与 [Strategy、Risk、Execution](./strategy-risk-execution.zh-CN.md)。

## 一个 Account、一种货币、一本账

每个 Paper Portfolio 只属于一个 Paper Trading Account 和一种 Valuation Currency。一个 Portfolio 可以协调该 Account 可交易的多个 Instrument，但不能借用或共享另一个 Account 的资金。

| 市场 | Paper Execution Adapter | 权威 Ledger | Valuation Currency | Funding Target |
| --- | --- | --- | --- | ---: |
| 中国 A 股普通证券账户 | `adaq-a-share-paper` | ADAQ 自有模拟器 | CNY | 1,000,000 CNY |
| 美国股票 | `adaq-alpaca-paper` | Alpaca Paper | USD | 1,000,000 USD |
| Crypto Spot | `adaq-okx-paper` | OKX Demo Trading | USDT | 1,000,000 USDT |

所有余额、数量、价格、费用和 PnL 都使用精确 Decimal。Funding Target 不代表可以伪造外部 Provider 的余额。

## 外部 Account 与 ADAQ 自有 Account

对于 ADAQ 自有模拟器，Funding Target 按明确的重置流程初始化新的 Paper Ledger。

对于 Alpaca 或其他外部 Paper Provider，最新且已验证的 Paper Account Snapshot 才是权威来源。如果 Provider 报告 100,000 USD，而期望目标是 1,000,000 USD，ADAQ 必须同时展示两个值和差异；不得把 1,000,000 USD 显示成可用现金，也不得静默执行远程重置。

Credential 永远不能进入 Paper Account Snapshot、Run 导出、日志、截图证据要求或 Component 输入。只能通过 [Paper Provider 连接与 Credential](./paper-connections.zh-CN.md) 中的 Host-owned 流程配置。

## Paper Execution Adapter

三个 Adapter 共享 ADAQ 内部的 Order、Fill、Account Snapshot、Error 与 Reconciliation 证据契约，但不能把 Provider 差异压平成虚假的最低共同模拟行为。

- **OKX Demo Trading** 使用模拟账户 API Key 和模拟交易请求模式。OKX Order ID、状态、错误、账户事件和 Fill 保持 Provider 权威。
- **Alpaca Paper** 使用 Paper Endpoint 和 Paper Account Credential。Alpaca 的确认、拒单、部分 Fill、Buying Power 和 Account Snapshot 保持 Provider 权威。
- **A-share Paper** 是 ADAQ 自有普通证券账户模拟器，因为 V1 没有选定可用的免费 A 股 Paper API。其 Append-only 本地 Ledger 和精确市场规则输入保持权威。信用账户、融资、融券、卖空和保证金负债均不可用。

每个 Account 都冻结 Paper Execution Capability Snapshot，覆盖支持的 Instrument、Order Type、Session、Extended Hours、Precision、Buying Power、Margin/Short 行为、Fill 假设、Event Stream、Rate Limit 和 Account Reset 能力。界面必须显示不支持的能力，而不是提供 Adapter 无法兑现的控制项。

Provider Paper 环境参考：[OKX API FAQ](https://www.okx.com/help/api-faq) 与 [Alpaca Paper Trading](https://docs.alpaca.markets/us/docs/paper-trading)。

## 启动或断线后的 Reconciliation

网络中断后，ADAQ 不得继续相信过期缓存账户。启动、重连、账户事件序列缺口或检测到差异时，系统必须：

1. 阻止受影响 Account 增加新风险。
2. 获取 Provider 权威 Account Snapshot 和 Open Order 状态。
3. 在 Provider 支持时重放或恢复有序的 Account、Order 和 Fill Event。
4. 比较 Cash、Buying Power、Position、Reserved Funds、Order、Fill 和 Event Identity 与本地证据 Ledger。
5. 记录每一项可解释修正和未解决差异。
6. 只有 Account 在冻结 Policy 下达到 Reconciled 状态后才恢复。

未解决差异不能通过覆盖 Provider 或删除本地历史来“修复”。Dashboard 必须把对应 Account 显示为 Unreconciled，并解释哪些值不一致。

## Portfolio 绑定

Paper Trading 启动前，一个 Strategy Instance 必须绑定一个 Paper Portfolio。该绑定冻结：

- Paper Trading Account 身份和 Provider。
- Valuation Currency。
- 合格 Venue 和 Instrument Universe。
- 起始 Paper Account Snapshot。
- Account 可用资金内的 Strategy Allocation。
- Risk Policy 与 Execution Profile。
- Calendar、结算、精度、费用和市场规则证据。

Portfolio Strategy 可以在 Alpaca Paper 内对多只美股排序和配置，在 ADAQ 模拟器内配置多只 A 股，或者在 Crypto Paper Account 内配置多个 Spot Instrument。V1 中不能生成同时包含 `600519`、`AAPL` 和 `BTC-USDT` 的 Portfolio Target。

## Dashboard 展示

Dashboard 必须分别展示每个 Account：

```text
A-share Paper     1,000,000 CNY target     observed CNY equity
Alpaca Paper      1,000,000 USD target     observed USD equity
Crypto Paper      1,000,000 USDT target    observed USDT equity
```

ADAQ 不得把三个数字相加成一个“总资产”。CNY、USD 和 USDT 是不同的经济单位。Account 卡片可以并列展示其原生 Equity、Cash、Buying Power、Reserved Capital、Exposure、PnL、Drawdown、连接状态和 Active Bots。

如果未来页面显示换算后的全球总资产，必须同时显示 Reporting Currency 以及精确 FX Snapshot、来源、时间、汇率和不可用换算。V1 不提供该总资产。

## 多个 Bot

多个 Bot 只有在明确预留资金并共享一个 Host-owned Account Risk Boundary 时，才可以使用同一个 Paper Trading Account。Strategy Allocation 总和不得超过可用资金；每个 Open Order 必须先预留资金，其他 Bot 才能计算 Buying Power。

不同 Paper Account 上的 Bot 在操作上彼此独立。一个 Account 的网络或 Broker 故障不会改写另一个 Account 的 Ledger；但 Freeze All 等全局操作可以有意暂停全部 Bot。

## 用户必须能检查的证据

- 不暴露 Credential 的 Paper Account 与 Provider 身份。
- Paper Funding Target 与实际起始 Account Snapshot。
- Valuation Currency 和精确 Decimal 余额。
- Strategy Allocation、Reserved Cash、Buying Power、Positions 和 Pending Orders。
- 每一笔改变余额的 Fill、Fee、Adjustment 和 Reset Event。
- Risk Policy、Execution Profile、Trading Calendar 和市场规则身份。
- 连接健康状态及最新 Account Snapshot 的年龄。
- Paper Execution Adapter 与 Capability Snapshot 身份。
- Reconciliation 状态、上次成功时间及未解决差异。
- Provider 报告 Equity 与本地推导 Equity 的任何差异。

## V1 明确阻止的行为

- 把不同货币相加成没有意义的全球总资产。
- 用本地偏好覆盖外部 Broker 余额。
- 一个 Portfolio 花费另一个 Account 中的资金。
- Component 获取 Account Credential 或直接修改 Ledger。
- 把 Backtest Balance 展示为 Paper Account Balance。

## 未来 Global Portfolio 的前提

可信的跨 Account 或跨 Currency Portfolio 至少需要：

- 不可变 FX Snapshot 和一种 Reporting Currency。
- Currency Conversion 与 Cash Transfer 规则。
- Settlement、Borrowing、Margin 与 Collateral 语义。
- 协调后的 Calendar 和 Decision Time。
- Cross-account Risk Reservation 与 Execution Recovery。
- Provider-specific Funding 与 Transfer 证据。

这些能力不属于 V1，也不能由只改变 UI 显示货币来模拟。
