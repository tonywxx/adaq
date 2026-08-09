# Alpaca Basic 美股数据路径

ADAQ 的美股路径使用经过认证的 Alpaca Market Data Basic 连接。固定行情
端点为 `https://data.alpaca.markets`；支持的行情流为
`wss://stream.data.alpaca.markets/v2/iex`。

## 设置

1. 在桌面应用打开 **设置 > 连接**。
2. 只在这里填写 Alpaca **Paper API Key ID** 和 **Paper Secret Key**。
3. 保存并测试连接。

不要把任一值粘贴到聊天、源文件、日志、诊断信息、数据管道 provenance
或 `.env` 文件中。Host 只从操作系统密钥库解析密钥对；GUI 和管道 DTO
只保留 Profile 身份和经过掩码的公开 Key 后缀。

## Basic 计划契约

- 行情源：**仅 IEX**。ADAQ 不会把它描述成综合市场或全市场实时行情。
- 历史 K 线：连接器声明从 2016 年开始，并在运行时记录最近 15 分钟的
  历史数据截止时间。
- 历史请求控制：每分钟最多 200 次，使用有界重试和分页。
- 行情流：Basic 路径最多一个连接、30 个标的；重连和提供商错误会作为
  行情流事件暴露。
- Provider Capability Snapshot 会保留不可用能力，包括综合实时行情、
  全市场成交量、最近 15 分钟内的历史 K 线和提供商公司行动数据。

## 证据边界

管道保留提供商响应哈希/原始响应、Instrument Master 修订、
`America/New_York` Trading Calendar Snapshot、UTC 会话边界、Source 修订、
Canonical 修订、质量报告、缺口、提供商错误和覆盖范围限制。K 线始终为
**Unadjusted**。公司行动不会合并、覆盖 K 线，也不会用于填补缺口。

如果未来加入可选的辅助历史/公司行动交叉校验，必须单独标记并保留独立
provenance；不得自动回退、合并或写入 Alpaca Canonical 证据。
