# M15 Paper Trading 人工验收

本指南仅覆盖 V1 OKX Demo Paper 路径。Live 端点、真实凭据、保证金、做空和股票适配器不在验收范围内。

<!-- m15-acceptance:connection -->
1. 在 Settings → Connections 配置并测试 OKX Demo Profile。确认显示 `okx_demo`、`demo`、USDT，且不显示任何凭据值。

<!-- m15-acceptance:account -->
2. 在已认证桌面会话中调用 `paper_account_reconcile`。确认账户快照、USDT 现金、持仓、reconciliation 状态和 provider evidence 可检查。

<!-- m15-acceptance:order -->
3. 使用新的 operation ID 调用 `paper_order_submit` 提交一个合法的 `BTC-USDT` limit order。确认 Host Risk 审批、本地现金 reservation、规范化订单 evidence 和 OKX Demo provider order ID。

<!-- m15-acceptance:fill-cancel -->
4. 调用 `paper_order_sync` 保留 provider partial fill，再对剩余数量调用 `paper_order_cancel`。确认 Fill journal、释放的 reservation、cancelled order 和 provider evidence 均可检查。

<!-- m15-acceptance:recovery -->
5. 重启应用或模拟 provider uncertain response。确认账户进入 `Required`，新订单 fail closed，只有成功的 OKX Demo reconcile 才能恢复执行。

<!-- m15-acceptance:locales -->
6. 在 `en-US` 和 `zh-CN` 重复步骤 1–5。两种 locale 必须提供相同的 OKX Demo-only commands 和状态；不得出现 Live 操作或凭据值。

验证命令：

```text
cd src-tauri && cargo test -p adaq --lib paper_trading -- --test-threads=1
cd src-tauri && cargo test -p adaq-paper-trading-core
cd src-tauri && cargo check -p adaq
pnpm run build
```
