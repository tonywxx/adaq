# 中国 A 股数据路径

[English](a-share-data-path.md)

Issue #72 在 `adaq-data-core::a_share` 中提供与 Tauri 无关的 A 股连接器，
并在 `adaq-data-pipeline::a_share` 中提供持久化的 Source → Canonical 路径。

## 合同

- `akshare-rs` 固定为 `=0.1.14`；GUI 边界只接收资产中立 DTO，不会接收上游
  原始 payload 或本地证据路径。
- 日线、分钟线和公司行为的 wire bytes 在连接器边界先保留，再做标准化，
  因此十进制字符串和格式错误行不会从浮点 DTO 重新构造。
- 由于固定版本的 akshare-rs DTO 在这些方法中使用浮点字段，原始 wire 适配器
  单独固定为 `adaq-data-core-raw-wire-v1`。当前价、成交量和成交额以精确十进制
  字符串及采集时间保留。
- 每次采集保留连接器版本、实际上游和方法、标准化请求参数、采集时间、
  response/content hash、重试诊断、能力快照以及已知限制。
- 成功采集要求每个 response hash 都有匹配且非空的原始 wire 证据。回填只能
  使用当前用户已经记录的日历 snapshot；如果 catalog cutover 与 checkpoint
  完成之间发生崩溃，Source 发布会按幂等方式恢复。
- Instrument ID 是交易所身份（`sse`/`szse` 加六位代码）。冲突的供应商
  前缀会被拒绝。Master snapshot 不可变，回填按时间点选择。
- Canonical equity Bars 只允许 `Unadjusted`。复权值不会悄悄混入主序列；
  公司行为作为独立、不可变的 revision 证据保存。
- UTC instant 从 Asia/Shanghai 推导。日历记录 09:30–11:30、13:00–15:00
  两段交易时段、11:30–13:00 午间休市、周末、节假日，以及供应商对提前收市
  和临时闭市信息的限制。

## 供应商方法

| 证据 | akshare-rs 方法 | 上游 |
| --- | --- | --- |
| Instrument Master/当前值 | `stock_zh_a_spot` + raw-wire `Market_Center.getHQNodeData` | 新浪财经 |
| 日线 Bars | raw-wire `stock_zh_a_daily`，`adjust=""` | 东方财富 K 线 |
| 分钟 Bars | raw-wire `stock_zh_a_minute` | 新浪 KLineData |
| 公司行为 | raw-wire `stock_fhps_detail_em` | 东方财富 |
| 交易日 | raw-wire `tool_trade_date_hist` | 新浪 |

连接器使用有界重试次数、请求超时、重试间隔和供应商响应窗口限制。请求范围
外或尚未收盘的观察会被排除；格式错误的行会保留在 Source 证据中，并交给
canonicalizer 隔离。公司行为行也在保留证据边界内标记为 `Passed`、`Degraded`
或 `Rejected`，不会丢弃隔离行。取消或失败的回填保留 checkpoint 和失败证据，
不会发布不完整的 canonical 文件；重试或超时时也可以取消。
分钟行如果没有上游提供的精确成交额，会被隔离，不会推断成交额。
桌面命令暴露 master 采集/列表、时间点 membership、交易日历和公司行为证据、
有界回填/取消以及资产中立的 workspace DTO。重启后遇到旧的 `Running`
checkpoint 可以安全恢复，因为采集到的 Bars 会在发布前持久化，而且不会看到部分
Source 或 Canonical 发布。

## 本地验证

`src-tauri/fixtures/a-share/` 下的 fixture 只用于离线测试，不是交易数据。进入
`src-tauri` 后运行：

```sh
cargo test -p adaq-data-core a_share --lib
cargo test -p adaq-data-pipeline a_share::tests --lib
```

线上供应商可用性、限流、历史窗口、停牌区间以及上游之间的分歧都会作为证据
限制保留，不会用推断值补齐。

## 故障排查

- 日线或分钟线返回空响应时，系统会保留原始 hash，并报告为未确认的历史可用性，
  不会生成虚假的 Bar。
- 复权 payload 或格式错误行会被拒绝或隔离，同时保留原始响应。重试前先查看
  quality report 和 limitation 列表。
- 取消或失败的回填会保留 checkpoint 和采集文件。使用相同任务请求再次运行时
  会从这些证据恢复；请求改变时应使用新的 task ID。
- 原始文件、日历文件缺失或 hash 不一致属于本地证据完整性错误。应恢复本地
  证据存储或重新采集 snapshot，不要绕过校验。
