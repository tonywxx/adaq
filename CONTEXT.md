# ADAQ Domain

Domain language for market data, component-based quantitative research, reproducible strategy runs, and supervised local trading.

## Language

### Identity and Access

**User**:
The registered ADAQ identity that owns private Run history, configuration, and Component Entitlements. Device-shared content does not make another User's private records or Components accessible.
_Avoid_: Device, Venue account

**User Profile**:
The User's editable presentation identity, currently limited to display name and avatar.
_Avoid_: Account, credentials

**Account Settings**:
The User's authentication details and session actions, currently limited to viewing the email address, changing the password, and signing out. Account deletion is excluded.
_Avoid_: User Profile, Venue account

**Local Research Data**:
Device-resident Watchlist, Component, Market Data, Run, and Validation records owned by one User. Reset operations affect only the current User, and shared files are removed only when no other User retains access.
_Avoid_: Account data, device-wide data

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

**Continuous Bar Segment**:
A maximal uninterrupted sequence of Closed Bars for one Instrument and Bar Interval within a Bar Series. A Bar Gap separates adjacent Continuous Bar Segments.
_Avoid_: Gap-filled series, synthetic continuity

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

**Indicator Catalog**:
The versioned host-owned registry of Built-in Indicators that ADAQ supports, including their inputs, parameters, outputs, and Warmup rules. It is an explicit public contract rather than every function present in the underlying analytical library.
_Avoid_: Raw TA-Lib function list, automatically discovered functions

**Factor Component**:
An independently packaged Component that transforms declared host inputs into one or more named scalar analytical values.
_Avoid_: Indicator, strategy, trading plugin

**Factor Instance**:
A Run-scoped binding of one Factor Component to a unique alias and one exact parameter set. The same Factor Component may have multiple Factor Instances in a Run.
_Avoid_: Factor copy, Component alias

**Strategy Component**:
An independently packaged Component that consumes host-supplied values and emits a complete Target Decision for one Strategy Instance.
_Avoid_: Order executor, broker plugin

**Model Component**:
An independently packaged inference Component that consumes host-supplied Prediction Batches and emits Forecast Batches under a declared Model Scope. Its training engine is not part of the runtime contract.
_Avoid_: Training framework, Strategy Component, Signal Component

**Model Instance**:
A configured inference resource created from one Model Component, its frozen Feature Slots, and a host-provided Inference Seed. It may retain only past causal context across ordered Prediction Batches, must produce bit-identical finite outputs for the same Package, Plan, Snapshot, parameters, Seed, and engine identity regardless of host chunk boundaries, and is rebuilt at Bar Gaps and Model Producer Segment boundaries.
_Avoid_: Model Artifact, training process, whole-history mutable predictor

**Model Scope**:
The declared Instrument cardinality a Model requires for one inference unit. M8 executes Single-Instrument Models; Cross-Sectional Models are a future scope over the same batch identities.
_Avoid_: Asset Class, Strategy Instance, training universe

**Prediction Batch**:
An ordered table of inference rows keyed by Instrument ID and Prediction Time with dense values matching a Model Component's Feature Slots. M8 restricts each Batch to one Instrument without removing row identity from the contract.
_Avoid_: Strategy Feature Frame, anonymous tensor, training Dataset

**Forecast Batch**:
The ordered optional Forecast Signal Frames returned one-for-one for a Prediction Batch, preserving every Instrument ID, Prediction Time, and row position. The host attaches each Frame's Available At boundary; Multi-horizon predictions remain Signals on the originating row rather than generated future rows.
_Avoid_: Forecast Signal Dataset, arbitrary model response

**Available At**:
The earliest timestamp at which a Forecast Signal Frame may legally be consumed by a Strategy. It is separate from Prediction Time, does not change Dataset row identity, and prevents a forecast from being used before its required inputs and inference schedule make it available. M8 native Closed-Bar inference uses the input Bar's close boundary for both Prediction Time and Available At, with execution no earlier than the next Bar.
_Avoid_: Prediction Time, file creation time, unrestricted execution time

**Forecast Path Artifact**:
A generated future time-series path such as predicted OHLCV Bars. It is distinct from Forecast Signals and is outside M8.
_Avoid_: Forecast Batch, realized market data, synthetic input rows

**Composed Strategy**:
A Strategy Component that embeds its own analytical, model, or signal logic; it is a product label rather than a distinct Component contract.
_Avoid_: Trading Combo Component, third component type

**Strategy Architecture**:
A derived UI description of a Strategy's actual Feature sources: Signal-driven consumes only Forecast Signals, Composed consumes Market, Indicator, or Factor Features without Forecast Signals, and Hybrid consumes Forecast Signals together with at least one of those other Feature sources. It is computed from Component Meta for a Package and from the frozen Feature Plan for a Run; it is not an author-declared Manifest field or a distinct Strategy ABI.
_Avoid_: Strategy kind, manifest architecture flag, compatibility mode

**Component Package**:
An immutable installable bundle containing a Component binary and its authoritative Component Meta. Validation Reports and trust records remain separate and reference the exact package hash.
_Avoid_: Bare WASM file, plugin DLL

**Component Meta**:
Stable information describing a Component's identity, versions, parameters, inputs, outputs, dependencies, warmup, and licensing.
_Avoid_: Backtest result, performance claim

**Validation Report**:
Historical evidence associated with an exact Component Package hash, data snapshot, configuration, period, and validation method; it does not guarantee future performance.
_Avoid_: Component Meta, profitability guarantee

**Forecast Evaluation Report**:
Immutable evidence of Forecast Signal quality bound to the exact Model Producer Segments, Forecast Signal Dataset, Market Data Snapshot, Forecast Targets, inference trust state, evaluation window, and evaluation configuration. It labels its Evidence State as Out-of-sample, Overlapping, or Unknown from the recorded training, fitting, and normalization windows; it evaluates predictions rather than Strategy decisions or profitability.
_Avoid_: Validation Report, Backtest result, profitability claim

**Evaluation Evidence State**:
The relationship between a Forecast Evaluation window and the Model's recorded training, fitting, and normalization reference windows. Out-of-sample requires complete evidence and no overlap, Overlapping records known reuse of evaluation observations, and Unknown preserves incomplete provenance without inventing an out-of-sample claim.
_Avoid_: Model trust state, performance grade, automatic validation

**Component Dependency Mode**:
The origin and lifecycle of a Component input: Built-in is free host functionality, Embedded is compiled into the consuming Component, and External is a separately packaged Factor or Model Component resolved by the host.
_Avoid_: Component-to-component call, automatic runtime download

**Component Lock**:
The immutable record of the exact Component packages, hashes, contracts, and trust states selected for a Run.
_Avoid_: Latest version, floating dependency

**Feature Slot**:
A stable, uniquely named, ordered input declared by a Model or Strategy Component together with its exact Market Field, Indicator, Factor, or Forecast Signal semantic requirement and bound by the host to one finite analytical scalar before execution. A Strategy Forecast Signal Slot declares the required Prediction Kind, Forecast Target, horizon, and value contract rather than a Model or Dataset identity; the frozen Feature Plan records the selected Dataset, Signal name, and producing Model provenance. A Model cannot consume another Model's Forecast Signal in M8; a Strategy may consume several Signal Datasets. Users may configure only declared parameters and compatible bindings; they cannot add, rename, remove, reorder, or arbitrarily replace the Component's slots.
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

**Feature Plan**:
The fully validated, immutable pre-execution resolution of Feature Slots and their exact Market, Indicator, Factor, or Forecast Signal Dataset sources, including parameters and exact Warmup requirements. Model inference and Strategy Backtest each freeze their own Plan; a modular Backtest references an already finalized Forecast Signal Dataset and never invokes its producing Model implicitly.
_Avoid_: Indicator Plan, runtime feature lookup

**Warmup**:
The exact leading Closed Bars in each Continuous Bar Segment that prepare all bound analytical sources and Model contexts before a Strategy Instance may be invoked. Upstream Feature Warmup and Model Warmup compose without synthetic values; a Bar Gap starts a new segment and therefore rebuilds analytical state and restarts Warmup.
_Avoid_: Optional startup delay, approximate lookback

**Market Data Snapshot**:
An immutable identity for the exact Closed Bar dataset used by a reproducible Run.
_Avoid_: Latest market data, mutable cache

**Model Artifact**:
The immutable fitted result of a reproducible model-training process, bound to its exact training evidence and provenance. It may be exported as a Model Component but is not itself a deployable Component.
_Avoid_: Model Component, mutable checkpoint, training project

**Model Provenance**:
The traceable origin and formation record of a Model Artifact, including exact source revisions, weight and preprocessing identities, runtime and Adapter versions, licence, and known training, fitting, validation, and normalization-reference windows. Unknown facts remain explicitly unknown.
_Avoid_: Model performance, inferred training claim, Package marketing

**Forecast Signal**:
A named, finite numeric prediction with a declared Prediction Kind, Forecast Value Scale, positive Bar horizon, and machine-readable Forecast Target, produced for one Instrument from information available at its prediction time. It is evidence consumed by a Strategy, not a Target Decision, position, or order.
_Avoid_: Target Exposure, buy signal, order instruction

**Forecast Signal Frame**:
The ordered set of one through 64 Forecast Signals emitted by one Model Component for the same Instrument and Prediction Time, with one host-controlled Available At boundary shared by the Frame.
_Avoid_: Feature Frame, Target Decision, arbitrary prediction object

**Forecast Signal Dataset**:
An immutable collection of Forecast Signal Frames with one stable Signal contract, bound through ordered Model Producer Segments to one or more exact Model Artifacts and to an exact Market Data Snapshot, prediction, availability, and Seed configuration, and inference trust state. Its row identity is Instrument ID plus Prediction Time; Available At is a required consumption boundary rather than part of that identity. A modular Backtest may consume a time subset but must use the same Snapshot identity, Instrument, Venue, and Bar Interval; AdaQ never silently resamples, fills, or approximately joins Signal rows.
_Avoid_: Mutable prediction file, Model Component, live Signal Feed

**Dataset Generation Attempt**:
A mutable lifecycle record for creating one Forecast Signal Dataset, progressing through Pending, Running, and exactly one terminal state: Completed, Failed, or Cancelled. Only a Completed Attempt may atomically publish an immutable Dataset; failed or partial rows remain diagnostic evidence and cannot enter Evaluation or Backtest.
_Avoid_: Forecast Signal Dataset, resumable Model state, partial research evidence

**Model Producer Segment**:
A non-overlapping Prediction Time range assigning Forecast Signal rows to one exact Model Artifact and its inference provenance. One Dataset may contain several ordered Segments for walk-forward retraining while retaining one unchanged Signal contract; M8's ordinary static-model case contains one Segment.
_Avoid_: Per-row repeated Artifact metadata, overlapping model ownership, mutable deployment pointer

**Forecast Signal Archive**:
The portable `.adaq-signals` container for one Forecast Signal Dataset, containing its canonical Manifest and Parquet evidence. Producer and trust state belong in the Manifest rather than the file extension.
_Avoid_: Component Package, source-specific extension, editable CSV

**External Model Adapter**:
A local integration that converts inference from a non-AdaQ runtime into a canonical Forecast Signal Dataset without making that runtime part of the AdaQ Component ABI.
_Avoid_: Model Component, arbitrary in-app Python execution, training engine ABI

**Externally Generated Signal Dataset**:
A Forecast Signal Dataset produced outside an AdaQ-controlled inference runtime. It remains usable research evidence, but AdaQ does not claim that its inference was reproduced or free of lookahead.
_Avoid_: Verified Model inference, Marketplace-ready Model Package

**Forecast Target**:
The stable, versioned Binary or Continuous outcome that a Forecast Signal claims to predict. A Built-in Forecast Target is host-verifiable, while a Custom Forecast Target preserves non-standard semantics without claiming standard evaluation.
_Avoid_: Target Exposure, free-form label, training note

**Built-in Forecast Target**:
A Forecast Target from AdaQ's versioned Catalog whose realized value the host derives within one Continuous Bar Segment. `Future Close Return@1` is `futureClose / originClose - 1`, while `Future Close Up@1` is one only when the future close is greater than the origin close; neither includes trading costs.
_Avoid_: Custom Forecast Target, user-defined expression

**Custom Forecast Target**:
A non-standard Forecast Target with a stable ID, version, description, and Binary or Continuous value type. A Model may emit it and a Strategy may consume it, but AdaQ does not report target-specific metrics unless verifiable realized labels are available.
_Avoid_: Built-in Forecast Target, automatically verified label

**Prediction Kind**:
The declared interpretation of a Forecast Signal's finite numeric value. Built-in kinds are Score, Probability, and Expected Value: Probability requires a Binary Forecast Target, Expected Value requires a Continuous Target, and Score accepts either; other semantics use a Custom Prediction Kind.
_Avoid_: Forecast Target, output name, numeric storage type

**Forecast Value Scale**:
The declared numeric representation required for safe Signal substitution. Probability is fixed to zero through one, Expected Value uses its Forecast Target's native unit, and Score must declare Percentile, Z-score, or a Custom Scale. The Model or External Model Adapter emits values in the final declared Scale and records its causal normalization reference; AdaQ validates but does not silently standardize them. A Strategy Forecast Signal Slot requires the same Scale contract as its bound Dataset Signal.
_Avoid_: Prediction Kind, display formatting, inferred normalization

**Custom Forecast Value Scale**:
A non-standard Signal scale with a stable ID, version, description, and optional finite bounds. It preserves raw Qlib or other engine-specific scores without claiming compatibility with Percentile, Z-score, or another Custom Scale.
_Avoid_: Undocumented raw score, automatic standardization

**Custom Prediction Kind**:
A non-standard Prediction Kind with a stable ID, version, description, and Custom Forecast Value Scale. AdaQ records and replays its finite values but reports no kind-specific metrics without a matching evaluator.
_Avoid_: Undocumented custom value, Built-in Prediction Kind

**Backtest Run**:
An immutable execution record binding a Market Data Snapshot, Component Lock, Strategy parameters, Feature Plan, Execution Profile, engine version, and seed.
_Avoid_: Editable backtest session

**Target Exposure**:
The desired signed notional fraction of a Strategy Instance's current Strategy Equity: zero is flat, positive is long, and negative is short.
_Avoid_: Signal strength, order quantity, confidence score

**Target Decision**:
The complete Target Exposure emitted by a Strategy Instance for one Closed Bar; returning the current target represents hold and returning zero represents close.
_Avoid_: Buy signal, sell signal, optional decision

**Run Pause**:
The recorded absence of a Target Decision while a Run is warming up or lacks a required Feature or Forecast Signal; missing data is never replaced with a synthetic analytical value. A Run Pause does not mean flat or close, produces no new order intention, and leaves the current exposure unchanged.
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

**Candidate Discovery**:
The process of producing Factor or Strategy Component candidates for evaluation without asserting their quality or future performance.
_Avoid_: Best-strategy generation, winner discovery

**Validation-ranked Candidate**:
A Component candidate's position under one exact Validation Protocol and scoring rule; the rank is historical evidence within that frozen study, not proof of global optimality or future profitability.
_Avoid_: Best Factor, best Strategy, guaranteed winner

### Application Navigation

**Page Navigation History**:
The ordered sequence of ADAQ application pages and selected Backtest-result or Validation-report tabs visited in the current WebView session, used by Back and Forward to restore a previously visited page or tab, or revisit one after going back. A new page or tab visited after going back discards the Forward sequence. It excludes report selection, form state, external pages, other non-ADAQ history entries, and shareable URL state.
_Avoid_: Page stack, custom navigation session, WebView history, deep link

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
