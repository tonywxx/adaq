# ADAQ Market Data

Domain language for identifying financial instruments consistently across supported asset classes.

## Language

**Instrument**:
A tradable product listed by a specific venue and identified by its venue-native code. The term is asset-class-neutral.
_Avoid_: Coin, stock, symbol

**Venue**:
The exchange or market on which an Instrument is listed and traded.
_Avoid_: Provider, data source

**Instrument ID**:
The ADAQ-wide identity composed of a Venue and that venue's native Instrument code.
_Avoid_: Symbol, ticker

**Instrument Status**:
The normalized lifecycle status of an Instrument: Live, Suspended, Pre-Open, Test, or Unknown. Unknown status is never treated as live or tradable.
_Avoid_: State, provider status

**Listing Time**:
The UTC instant when an Instrument enters a Venue's listing or pre-open process; it may precede continuous trading.
_Avoid_: Trading start time

**Continuous Trading Time**:
The UTC instant when an Instrument becomes available for continuous trading.
_Avoid_: Listing time

**Price Increment**:
The smallest permitted price step for an Instrument.
_Avoid_: Tick size, decimal places

**Quantity Increment**:
The smallest permitted base-asset quantity step for an Instrument.
_Avoid_: Lot size, quantity precision

**Minimum Quantity**:
The smallest base-asset quantity accepted for an Instrument.
_Avoid_: Minimum size, minimum amount

**Asset Class**:
A broad category of instruments, such as crypto or equity.
_Avoid_: Market, provider

**Asset Code**:
A Venue-scoped native identifier for an asset; matching codes across venues do not imply global asset identity.
_Avoid_: Global asset ID, coin ID

**Base Asset**:
The asset whose quantity is bought or sold in a Spot Instrument.
_Avoid_: Base coin

**Quote Asset**:
The asset in which a Spot Instrument's price is denominated.
_Avoid_: Quote coin, currency

**OHLCV Bar**:
A time-bounded aggregate for one instrument containing open, high, low, close, base volume, and quote volume.
_Avoid_: Candle, K-line

**Bar Identity**:
The unique combination of Instrument ID, Bar Interval, and Bar Open Time. Identical duplicates collapse; conflicting duplicates are invalid provider data.
_Avoid_: Timestamp-only identity

**Closed Bar**:
An OHLCV Bar whose time interval has ended and is eligible for reproducible historical queries.
_Avoid_: Finished candle, confirmed candle

**Historical Bar Range**:
A UTC half-open interval `[start, end)` whose Closed Bars are returned in ascending Bar Open Time order.
_Avoid_: Provider cursor, inclusive end range

**Bar Gap**:
An expected Bar Interval for which no provider-confirmed Closed Bar exists; it remains missing rather than being synthesized.
_Avoid_: Filled bar, synthetic zero-volume bar

**Bar Series**:
Closed Bars for one Instrument and Bar Interval over a requested Historical Bar Range, accompanied by any contiguous Bar Gap ranges after continuous trading began. Bar Gaps do not make the query fail.
_Avoid_: Bare bar array

**Bar Open Time**:
The UTC instant at which an OHLCV Bar interval begins, represented at boundaries as Unix milliseconds.
_Avoid_: Local time, timestamp

**Bar Interval**:
The provider-neutral duration or calendar period used to group market activity into an OHLCV Bar. Supported values are `1s`, `1m`, `3m`, `5m`, `15m`, `30m`, `1h`, `2h`, `4h`, `6h`, `12h`, `1d`, `2d`, `3d`, `5d`, `1w`, `1mo`, and `3mo`; every interval is aligned to UTC boundaries. Weekly intervals open Monday at 00:00 UTC, and monthly intervals open on the first day of the corresponding UTC month.
_Avoid_: Timeframe, provider bar string

**Multi-Day Interval**:
A `2d`, `3d`, or `5d` fixed-length UTC-day window anchored to 1970-01-01 00:00 UTC rather than to the query start or calendar month.
_Avoid_: Rolling day window, month-anchored interval

**Spot Instrument**:
A crypto instrument that exchanges a base asset for a quote asset without expiry or funding mechanics.
_Avoid_: Pair, spot pair

**Financial Value**:
A price, quantity, volume, notional, balance, fee, or other amount whose base-10 representation must remain exact across domain and IPC boundaries.
_Avoid_: Float, approximate number

**Base Volume**:
The amount of an instrument's base asset traded during an OHLCV Bar.
_Avoid_: Volume

**Quote Volume**:
The amount of an instrument's quote asset exchanged during an OHLCV Bar.
_Avoid_: Turnover, volume
