# M15 Paper Trading Manual Acceptance

This guide covers the V1 OKX Demo Paper path only. Live endpoints, real credentials, margin, shorting, and equity adapters are not acceptance scope.

<!-- m15-acceptance:connection -->
1. Configure and test an OKX Demo profile in Settings → Connections. Confirm the profile shows `okx_demo`, `demo`, USDT, and no credential values.

<!-- m15-acceptance:account -->
2. Invoke `paper_account_reconcile` with an authenticated desktop session. Confirm the returned account snapshot, USDT cash, positions, reconciliation state, and provider evidence are inspectable.

<!-- m15-acceptance:order -->
3. Invoke `paper_order_submit` for one venue-valid `BTC-USDT` limit order with a fresh operation ID. Confirm Host Risk approval, local cash reservation, normalized order evidence, and the OKX Demo provider order ID.

<!-- m15-acceptance:fill-cancel -->
4. Invoke `paper_order_sync` to retain a provider partial fill, then `paper_order_cancel` for the remaining quantity. Confirm the Fill journal, released reservation, cancelled order, and provider evidence remain inspectable.

<!-- m15-acceptance:recovery -->
5. Restart the app or simulate an uncertain provider response. Confirm the account is `Required`, new orders fail closed, and only a successful OKX Demo reconciliation restores execution.

<!-- m15-acceptance:locales -->
6. Repeat steps 1–5 in `en-US` and `zh-CN`. The same OKX Demo-only commands and states must be available in both locales; no Live action or credential value may appear.

Verification commands:

```text
cd src-tauri && cargo test -p adaq --lib paper_trading -- --test-threads=1
cd src-tauri && cargo test -p adaq-paper-trading-core
cd src-tauri && cargo check -p adaq
pnpm run build
```
