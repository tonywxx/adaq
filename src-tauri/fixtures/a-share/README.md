# A-share data-path fixtures

These small, committed fixtures exercise the provider-independent evidence
boundary without making network calls. They include normalized DTOs and raw
upstream responses retained by `adaq-data-core` and cover both exchanges, an
ordinary/suspended master row with exact current observations, exact decimal
bars, malformed bar and corporate-action rows for quarantine, a separate
cash-dividend record, adjusted and empty-history rejection, retry/error behavior, and
the Asia/Shanghai session contract. Pipeline tests also exercise cancellation,
durable acquisition checkpoints, and restart-safe publication.

The live connector is pinned to `akshare-rs` 0.1.14. Its actual methods and
upstreams are recorded in each acquisition:

- Instrument master/current values: `stock_zh_a_spot` + raw-wire `Market_Center.getHQNodeData` / Sina
- Daily Bars: raw-wire `stock_zh_a_daily` / Eastmoney kline
- Intraday Bars: raw-wire `stock_zh_a_minute` / Sina KLineData
- Corporate actions: raw-wire `stock_fhps_detail_em` / Eastmoney
- Trading dates: raw-wire `tool_trade_date_hist` / Sina

Fixture values are intentionally decimal strings. They are not production
market data and must not be used as a trading signal.
