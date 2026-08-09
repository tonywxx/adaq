# A 股数据路径 fixture

这些小型、提交到仓库的 fixture 用于在不访问网络的情况下验证与供应商
无关的证据边界。它们同时包含 `adaq-data-core` 保留的标准 DTO 和上游
原始响应，覆盖两家交易所、带精确当前观察的普通/停牌 master 行、精确十进制
Bar、用于隔离的格式错误 Bar 和公司行为行、独立现金分红、复权和空历史拒绝、
重试/错误行为以及 Asia/Shanghai 时段约束。Pipeline 测试还覆盖取消、持久化
采集 checkpoint 和可安全重启的发布。

线上连接器固定使用 `akshare-rs` 0.1.14，并在每次采集中记录实际方法和上游：

- Instrument Master/当前值：`stock_zh_a_spot` + raw-wire `Market_Center.getHQNodeData` / 新浪
- 日线 Bars：raw-wire `stock_zh_a_daily` / 东方财富 K 线
- 分钟 Bars：raw-wire `stock_zh_a_minute` / 新浪 KLineData
- 公司行为：raw-wire `stock_fhps_detail_em` / 东方财富
- 交易日：raw-wire `tool_trade_date_hist` / 新浪

fixture 数值故意使用十进制字符串。它们不是生产行情数据，不得用于交易信号。
