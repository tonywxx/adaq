# ADAQ Domain

Domain language for market data, component-based quantitative research, reproducible strategy runs, and supervised local trading.

## Language

### Identity and Access

**User**:
The registered ADAQ identity that owns private Run history, configuration, and Component Entitlements. Device-shared content does not make another User's private records or Components accessible.
_Avoid_: Device, Venue account

**Component Entitlement**:
A User-scoped right to view and execute an exact Component product under its licence and registered-device rules. Component Package bytes may be deduplicated on a device without sharing the entitlement.
_Avoid_: Component file, device ownership, global licence

### Market Data

**Instrument**:
A tradable product listed by a specific venue and identified by its venue-native code. The term is asset-class-neutral.
_Avoid_: Coin, stock, symbol

**Venue**:
The exchange or market on which an Instrument is listed and traded.
_Avoid_: Provider, data source

**Instrument ID**:
The ADAQ-wide identity composed of a Venue and that venue's native Instrument code.
_Avoid_: Symbol, ticker

**Watchlist**:
A user's ordered collection of venue-specific Instruments selected for monitoring.
_Avoid_: Symbol list, favorites

**Active Instrument**:
The single Instrument currently in focus across market-data views. It is always present and need not belong to the Watchlist.
_Avoid_: Selected symbol, current ticker

**Instrument Status**:
The normalized lifecycle status of an Instrument: Live, Suspended, Pre-Open, Test, or Unknown. Unknown status is never treated as live or tradable.
_Avoid_: State, provider status

**Ticker Snapshot**:
A Venue-published current market summary for one Instrument, containing the last trade, best bid and ask, rolling 24-hour statistics, volumes, and Venue timestamp. It may update without representing a new trade.
_Avoid_: Ticker when referring to Instrument identity, live price

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

**Open Bar**:
An OHLCV Bar for the current Bar Interval that may change until the Venue confirms it as a Closed Bar. It is suitable for live display but not reproducible historical queries.
_Avoid_: Live candle, unfinished K-line

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

### Components

**Indicator**:
A common market-series calculation provided free by the ADAQ host and referenced by name and parameters.
_Avoid_: Factor, paid factor

**Factor Component**:
An independently packaged Component that transforms declared host inputs into one or more named scalar analytical values.
_Avoid_: Indicator, strategy, trading plugin

**Factor Instance**:
A Run-scoped binding of one Factor Component to a unique alias and one exact parameter set. The same Factor Component may have multiple Factor Instances in a Run.
_Avoid_: Factor copy, Component alias

**Strategy Component**:
An independently packaged Component that consumes host-supplied values and emits a complete Target Decision for one Strategy Instance.
_Avoid_: Order executor, broker plugin

**Composed Strategy**:
A Strategy Component that embeds its own factor logic; it is a product label rather than a distinct Component contract.
_Avoid_: Trading Combo Component, third component type

**Component Package**:
An installable bundle containing a Component binary, its authoritative Component Meta, optional Validation Reports, integrity hashes, and trust information.
_Avoid_: Bare WASM file, plugin DLL

**Component Meta**:
Stable information describing a Component's identity, versions, parameters, inputs, outputs, dependencies, warmup, supported contexts, and licensing.
_Avoid_: Backtest result, performance claim

**Validation Report**:
Historical evidence produced for an exact Component, data snapshot, configuration, period, and validation method; it does not guarantee future performance.
_Avoid_: Component Meta, profitability guarantee

**Component Dependency Mode**:
The origin and lifecycle of a Strategy input: Built-in is free host functionality, Embedded is compiled into the Strategy Component, and External is a separately packaged Factor Component.
_Avoid_: Component-to-component call, automatic runtime download

**Component Lock**:
The immutable record of the exact Component packages, hashes, contracts, and trust states selected for a Run.
_Avoid_: Latest version, floating dependency

**Feature Slot**:
A pre-bound numeric position through which the host supplies one named Indicator or Factor value to a Component during a Run.
_Avoid_: JSON feature map, dynamic lookup

### Research and Execution

**Strategy Instance**:
A configured binding of one Strategy Component to one Instrument, one Bar Interval, parameters, allocation, and position mode.
_Avoid_: Strategy file, trading account

**Strategy Allocation**:
The exact Quote Asset capital assigned to a Strategy Instance at the start of a Run.
_Avoid_: Current equity, account balance

**Strategy Equity**:
The current mark-to-market value of a Strategy Instance's cash and position within a Run; it changes with fills, fees, and market prices.
_Avoid_: Initial allocation, Venue account equity

**Position Mode**:
The exposure constraint selected for a Strategy Instance: Long Only permits targets from zero through one, while Long–Short permits targets from negative one through one.
_Avoid_: Trade direction, signal type

**Indicator Plan**:
The immutable resolution of a Strategy Instance's declared Built-in Indicators, their parameters, warmup requirements, and Feature Slots.
_Avoid_: Runtime indicator request

**Market Data Snapshot**:
An immutable identity for the exact Closed Bar dataset used by a reproducible Run.
_Avoid_: Latest market data, mutable cache

**Backtest Run**:
An immutable execution record binding a Market Data Snapshot, Component Lock, Strategy parameters, Indicator Plan, Execution Profile, engine version, and seed.
_Avoid_: Editable backtest session

**Target Exposure**:
The desired signed notional fraction of a Strategy Instance's current Strategy Equity: zero is flat, positive is long, and negative is short.
_Avoid_: Signal strength, order quantity, confidence score

**Target Decision**:
The complete Target Exposure emitted by a Strategy Instance for one Closed Bar; returning the current target represents hold and returning zero represents close.
_Avoid_: Buy signal, sell signal, optional decision

**Run Pause**:
The recorded absence of a Target Decision while a Run is warming up or lacks a required input; missing data is never replaced with a synthetic analytical value.
_Avoid_: Zero signal, implicit skip

**Execution Profile**:
Host-owned rules that translate changes in Target Exposure into simulated or live order intentions, including thresholds, maker and taker fees, slippage, precision, and fill policy. Funding rates are outside the Spot execution model.
_Avoid_: Strategy order settings

**Simulated Order**:
A Backtest record of one host-derived Spot order intention and its created, filled, replaced, or cancelled lifecycle under the Run's frozen Execution Profile; it is never sent to a Venue.
_Avoid_: Target Decision, Fill, live order

**Fill**:
The completed execution of all or part of an order at an exact price and quantity, with its fee and maker or taker role. A Backtest Fill is simulated under the Run's frozen Execution Profile.
_Avoid_: Buy signal, sell signal, chart marker

**Recommended Context**:
An evidence-backed Instrument, Bar Interval, and parameter configuration in which a Component was historically evaluated.
_Avoid_: Best market, guaranteed configuration, highest future win rate

### Supervised Live Trading

**Supervised Local Execution**:
A live-trading mode in which the desktop application remains running and a human operator is available; it is not cloud or unattended execution.
_Avoid_: Cloud trading, autonomous 24/7 execution

**Live Control Lease**:
Exclusive permission for one ADAQ controller to manage one Venue account and Instrument at a time.
_Avoid_: Shared strategy ownership

**Unmanaged Position**:
An open Venue position deliberately retained after its controlling Strategy Instance has stopped.
_Avoid_: Active strategy position

**Freeze All**:
A host emergency action that pauses strategies, blocks new risk, and cancels open orders while retaining current positions.
_Avoid_: Flatten, application exit

**Flatten All**:
A separately confirmed host emergency action that first freezes execution and then attempts to close every controlled position with reconciliation.
_Avoid_: Freeze, blind market sell
