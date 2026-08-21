# ADAQ Domain

Domain language for market data, component-based quantitative research, reproducible strategy runs, and supervised local trading.

## Language

### System Boundaries

**ADAQ Host**:
The sole trusted authority for one ADAQ deployment, deriving Authenticated User Context at its boundary and owning authorization, authoritative evidence publication, secrets, Provider and Paper access, hard Risk and OMS, and supervision of sandboxed runtimes.
_Avoid_: Client Surface, GUI, Worker, Tauri command

**Client Surface**:
A human or automation-facing ADAQ entry point, such as the Desktop GUI or a future Mobile, Web, public API, AI Agent, or MCP surface, that requests bounded Host capabilities and consumes typed projections or events without owning domain authority.
_Avoid_: ADAQ Host, authoritative client, direct database client

**Embedded Interactive Surface**:
A human-facing Client Surface running on the same device as its ADAQ Host and using a local transport. Co-location never grants authority beyond its Surface Capability Profile and Authenticated User Context.
_Avoid_: ADAQ Host, trusted GUI, full-capability client

**Remote Interactive Surface**:
A human-facing Client Surface connected to an ADAQ Host on another runtime or device, with no direct access to that Host's local stores, secrets, Providers, or supervised processes.
_Avoid_: Browser database client, remote ADAQ Host, synchronized device

**Automation Surface**:
A machine-callable Client Surface such as a public API, AI Agent, or MCP adapter whose bounded, auditable Host capabilities are declared by its Surface Capability Profile.
_Avoid_: Quant Core, Bot Worker, autonomous authority

**Surface Capability Profile**:
The versioned allowlist of ADAQ Host capabilities available to one exact Client Surface deployment, separate from the Authenticated User Context's permissions. A capability absent from the Profile is unavailable even when the same User may access it through another Surface.
_Avoid_: User role, UI visibility, Desktop parity

**Host Capability Contract**:
The versioned, transport-independent set of bounded commands, queries, projections, and events an ADAQ Host exposes to Client Surfaces after applying Authenticated User Context and a Surface Capability Profile.
_Avoid_: Tauri command list, UI API, database API, Provider SDK

**Authenticated User Context**:
The Host-derived binding of one verified identity and session to an ADAQ User and its permitted capabilities. A request payload may not select or override this identity.
_Avoid_: Client-supplied user ID, User Profile, Venue account

**Host Session Binding**:
The process-memory association of one verified Supabase session with one Client Surface/window and its Authenticated User Context. It is established or refreshed through the Host authentication seam and cleared on sign-out, User change, verification failure, expiry, or window teardown; raw tokens are not persisted as domain data.
_Avoid_: Client session state, persisted token vault, User ID argument

**Strict Operation**:
A Host capability whose authority must reflect fresh online session state or immediate revocation, including User changes, credential and Secret Reference access, destructive reset, Bot/order work, and reconciliation. A Strict Operation fails closed when fresh validation is unavailable.
_Avoid_: every local read, UI-disabled action, provider request

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
Device-resident Watchlist, Component, Market Data, Feature Definition, Fitted Transformation Artifact, Feature Dataset, Run, and Validation records owned by one User. Reset operations affect only the current User, and shared files are removed only when no other User retains access.
_Avoid_: Account data, device-wide data

**Component Entitlement**:
A User-scoped right to view and execute an exact Component product under its licence and registered-device rules. Component Package bytes may be deduplicated on a device without sharing the entitlement.
_Avoid_: Component file, device ownership, global licence

### Storage and Control Plane

**Authoritative Evidence Store**:
The content-addressed Parquet evidence that is the source of truth for immutable, auditable, reproducible Market, Feature, Factor, Model, and Signal datasets. Its payloads are published atomically and are never replaced by a rebuildable query layer.
_Avoid_: Raw cache, analytical database, export file

**Local Metadata Store**:
The User-scoped SQLite source of truth for configuration, lifecycle and Attempt records, operational events, references, and local journals that govern access to device-resident evidence. It does not contain large immutable evidence rows or credential values.
_Avoid_: Research warehouse, secret database, browser storage

**Rebuildable Analytical Store**:
An optional DuckDB query and analysis layer derived from local evidence and safe to delete and recreate without losing facts, identities, or provenance.
_Avoid_: Authoritative database, write model, evidence store

**Rebuildable Projection**:
An index, summary, page, aggregate, or temporary join derived from authoritative SQLite metadata and Parquet evidence. It may be discarded and regenerated, and a mismatch is an integrity error rather than permission to rewrite the evidence.
_Avoid_: Evidence record, research conclusion, authoritative snapshot

**Atomic Evidence Publication**:
The ordered local commit boundary that validates and atomically publishes complete Parquet evidence before making its SQLite metadata visible; DuckDB projections remain outside the commit and may be stale or rebuilt.
_Avoid_: Cross-database transaction, best-effort write, partial dataset

**V1 Analytical Dependency Policy**:
V1 workflows must not depend on DuckDB or another rebuildable analytical engine for correctness, identity, or provenance. Such an engine may be added later only to address measured query or resource limits.
_Avoid_: Required warehouse, analytical source of truth, pre-provisioned scale layer

**Operating-System Secret Store**:
The device-protected authority for User-scoped credential values. ADAQ metadata stores only an opaque Secret Reference and redacted connection facts.
_Avoid_: Encrypted SQLite column, credential cache, masked secret

**Supabase Control Plane**:
The hosted identity, session, and future entitlement/licence/device-control boundary for ADAQ. It does not own authoritative research evidence or trading credential values.
_Avoid_: Cloud research warehouse, execution service, remote evidence store

**V1 Pre-Release Schema Policy**:
Before V1 acceptance, local schemas and persisted contracts may change incompatibly without migration, dual-read, or forward-compatibility guarantees. A changed schema may require discarding and recreating affected local data through an explicit reset or replacement workflow.
_Avoid_: Backward-compatible release contract, automatic migration, legacy dual-read

**Diagnostic Log**:
A bounded, User- and Attempt-scoped unstructured record retained for troubleshooting after Host redaction. It is not a protocol channel, authoritative domain evidence, or permission to persist secrets or private paths.
_Avoid_: Operational Event, stdout authority, permanent trace archive

**Operational Metric**:
A bounded numeric observation or deterministic rollup of host operation, retained under an explicit policy for trend and threshold evaluation. It is not a Market Data record, a universal score, or an unbounded time-series database.
_Avoid_: Research Metric, raw Tick archive, Health State

**Health Projection**:
A rebuildable current view derived from validated Operational Events and required domain evidence. It summarizes one dependency's Healthy, Degraded, Critical, or Unknown condition and never grants authority independently of Lifecycle, Risk, OMS, or Reconciliation gates.
_Avoid_: Operational Event, Alert acknowledgement, online boolean

**Provider-native Diagnostic**:
A redacted, bounded provider code, request/response meaning, sequence detail, or rejection reason retained with its original provider identity for diagnosis. It cannot silently become Canonical Market Data or replace an ADAQ-owned decision.
_Avoid_: Provider credential, normalized truth, translated error string

### Market Data

**Market Data Foundation**:
The evidence-grade V1 Data product that acquires, preserves, canonicalizes, grades, snapshots, and exposes three-market evidence required by Research and Paper Trading. It is not a comprehensive financial-information terminal or a promise of identical Provider coverage.
_Avoid_: Unified Data API, market-data terminal, all financial data

**Mandatory Market Data Baseline**:
The smallest per-Market set of identity, calendar, selected historical Closed-Bar, current-observation, evidence-lifecycle, and inspection capabilities required before that Market can be used as V1 Data Foundation input. It does not require every Provider-Graded capability or complete full-universe pre-download.
_Avoid_: full-market download, global readiness flag, provider feature parity

**Instrument**:
A tradable product listed by a specific venue and identified by its venue-native code. The term is asset-class-neutral.
_Avoid_: Coin, stock, symbol

**Venue**:
The exchange or market on which an Instrument is listed and traded.
_Avoid_: Provider, data source

**Market Data Provider**:
An external service that delivers observations about one or more Venues. Its identity remains separate from the Venue, the connector library, and the Instrument identities in its payloads.
_Avoid_: Venue, Market Data Connector, broker account

**Market Data Connector**:
Host-owned integration code that acquires records from one Market Data Provider and preserves the provider, upstream method, request, response, and connector-version provenance required to interpret them.
_Avoid_: Market Data Provider, unified data API, raw response

**Provider Capability Snapshot**:
An immutable, credential-free observation of the data an account or public provider access could legally and technically retrieve at one capture time, including covered Venues, feeds, record types, history, delay, rate limits, and streaming-symbol limits. Every Source Market Dataset binds the applicable Snapshot rather than assuming a plan name has stable capabilities.
_Avoid_: API key, static pricing-plan description, Data Quality Report

**Data Capability Class**:
The V1 scope class assigned to one Market Data capability: Required, Provider-Graded, Auxiliary Evidence, or Post-V1. It describes ADAQ's product promise, not whether a particular Provider currently supplies the capability.
_Avoid_: Provider feature flag, UI visibility, market completeness score

**Auxiliary Market Evidence**:
Separately identified market observations from a non-authoritative or secondary source used for cross-checking or enrichment. It never repairs, replaces, or silently merges into Canonical Market Data.
_Avoid_: fallback authority, backup provider, merged data

**Provider Capability State**:
The observed state of one Data Capability Class for a Provider Capability Snapshot: Available, Limited, Unavailable, or Unknown, with evidence and reasons retained for the capture.
_Avoid_: Boolean support, network-connected, guaranteed coverage

**Venue Time Zone**:
The Venue's IANA time-zone identity used to interpret its local trading rules, such as `Asia/Shanghai` or `America/New_York`; it is never replaced by a fixed UTC offset.
_Avoid_: Device time zone, current UTC offset, display preference

**Instrument ID**:
The ADAQ-wide identity composed of a Venue and that venue's native Instrument code.
_Avoid_: Symbol, ticker

**Instrument Source Mapping**:
The recorded provenance binding an Instrument ID to the provider-native symbol used in one Market Data Provider payload, including the provider, connector version, and capture time. Provider-native symbols are retained verbatim so payloads remain traceable to their Instrument identity.
_Avoid_: Symbol renaming, Instrument Master entry, inferred identity

**Watchlist**:
A User's asset-class-neutral ordered collection of venue-specific Instruments selected for monitoring. Market Workspaces filter the same Watchlist by Venue or Asset Class; identical display symbols never collapse distinct Instrument IDs.
_Avoid_: Symbol list, favorites

**Default Watchlist Seed**:
The ordered set of initial monitoring Instruments added once when a User's Watchlist is first created or upgraded. Removing a seeded Instrument is a User choice and does not trigger automatic refilling.
_Avoid_: Permanent favorites, automatic refill

**Active Instrument**:
The single Instrument currently in focus across market-data views. It is always present and need not belong to the Watchlist.
_Avoid_: Selected symbol, current ticker

**Instrument Status**:
The normalized lifecycle status of an Instrument: Live, Suspended, Pre-Open, Test, or Unknown. Unknown status is never treated as live or tradable.
_Avoid_: State, provider status

**Instrument Master Snapshot**:
An immutable record of every Instrument a Venue reports at one effective time, including its status, lifecycle times, assets, and trading constraints. It preserves what ADAQ could know then rather than rewriting history with today's listings.
_Avoid_: Current instrument list, mutable reference table

**Trading Calendar Snapshot**:
An immutable Venue calendar revision defining its Venue Time Zone, Trading Dates, holidays, early closes, scheduled Trading Sessions, and Session Phases for an exact effective range.
_Avoid_: Device calendar, weekday rule, mutable holiday list

**Scheduled Closure**:
A scheduled non-trading period recorded as calendar evidence for one Venue, such as a holiday, early close, special closure, or provider maintenance window. It is calendar state rather than a Bar Gap, and a missing Bar inside it never creates a false gap.
_Avoid_: Bar Gap, downtime log, silent missing data

**Market Rule Snapshot**:
An immutable, effective-time record of the Venue-, Instrument-, and account-specific rules required to validate and simulate trading, including sessions, auctions, order types, price limits, quantity units, settlement restrictions, fees, halts, and exceptional states.
_Avoid_: hard-coded exchange constants, Trading Calendar Snapshot, Execution Profile

**Trading Date**:
The Venue-local calendar date to which a session-based market observation belongs under an exact Trading Calendar Snapshot. It is an explicit identity and is not inferred from the UTC calendar date.
_Avoid_: UTC date, ingestion date, settlement date

**Trading Session**:
The ordered Venue-local windows during one Trading Date in which a defined class of market activity may occur. Scheduled auctions, continuous trading, breaks, and extended hours remain distinguishable rather than being flattened into one continuous UTC range.
_Avoid_: UTC day, Historical Bar Range, Bot uptime

**Session Phase**:
The Trading Calendar classification of an observation or order-eligibility window, such as pre-open, auction, continuous, break, extended-hours, or closed.
_Avoid_: Instrument Status, Bot state, inferred clock range

**Point-in-Time Instrument Universe**:
The exact set of Instruments eligible for one cross-sectional research observation under recorded inclusion rules and Instrument Master evidence available at that time. Missing historical membership evidence remains Unknown rather than being inferred from current listings.
_Avoid_: Watchlist, current listings, survivorship-biased universe

**Universe Evidence State**:
The provenance grade of a Point-in-Time Instrument Universe: Observed has contemporaneous Instrument Master evidence, Reconstructed is inferred from incomplete historical evidence, and Unknown lacks enough evidence to establish membership. Only Observed supports a claim of no known survivorship bias.
_Avoid_: Universe quality score, inferred certainty

**Ticker Snapshot**:
A Venue-published current market summary for one Instrument, containing the last trade, best bid and ask, rolling 24-hour statistics, volumes, and Venue timestamp. It may update without representing a new trade.
_Avoid_: Ticker when referring to Instrument identity, live price

**Market Trade**:
A Venue-published execution between market participants, identified by its Instrument, price, quantity, side classification, and Venue time. It is market evidence rather than an ADAQ account Fill.
_Avoid_: Fill, ticker update

**Level 2 Order Book**:
The current public bid and ask price levels for one Instrument reconstructed from a Venue snapshot and its ordered updates.
_Avoid_: Account orders, historical depth replay

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

**Price Currency**:
The currency in which an Equity Instrument's prices, turnover, fees, and account valuation are expressed.
_Avoid_: Quote Asset, account base currency

**OHLCV Bar**:
A time-bounded aggregate for one Instrument containing open, high, low, close, Base Volume, Quote Volume when available, and a declared Price Basis. A session-based Equity Bar also binds its Trading Date, Session Phase, and Trading Calendar Snapshot.
_Avoid_: Candle, K-line

**Price Basis**:
The declared corporate-action treatment of an Equity Bar. Unadjusted preserves provider-published historical prices; any adjusted series must identify its exact Corporate Action evidence and transformation method and remains derived data rather than Canonical Market Data.
_Avoid_: Boolean adjusted flag, normalization, silent back adjustment

**Corporate Action**:
An issuer or Venue event such as a split, dividend, rights issue, or symbol change that can alter share quantity, economic value, or historical comparability, recorded independently from OHLCV Bars with its effective-time provenance.
_Avoid_: Adjusted price, trading signal

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
An expected Bar Interval inside a scheduled trading phase for which no provider-confirmed Closed Bar exists; it remains missing rather than being synthesized. Holidays, scheduled breaks, and closed phases are calendar state rather than Bar Gaps.
_Avoid_: Filled bar, synthetic zero-volume bar

**Bar Series**:
Closed Bars for one Instrument and Bar Interval over a requested Historical Bar Range, accompanied by any contiguous Bar Gap ranges after continuous trading began. Bar Gaps do not make the query fail.
_Avoid_: Bare bar array

**Source Market Dataset**:
An immutable collection of provider-delivered records about a Venue, bound to its Market Data Connector and Provider Capability Snapshot and retained with acquisition provenance before ADAQ quality decisions change or exclude any record.
_Avoid_: Raw cache, mutable download

**Canonical Market Dataset**:
An immutable normalized market dataset derived from one Source Market Dataset under an exact recorded set of quality rules. Conflicting or invalid records remain quarantined evidence, and Bar Gaps remain explicit rather than being silently repaired.
_Avoid_: Cleaned table, gap-filled data

**Data Quality Report**:
Immutable evidence binding one Source Market Dataset to its Canonical Market Dataset and recording the applied rules, duplicates, conflicts, gaps, quarantined records, transformation outcomes, and Data Quality State.
_Avoid_: Cleaning log, mutable warning list

**Quarantined Market Record**:
A Source Market Dataset record excluded from Canonical Market Data because its identity, schema, or values conflict with the accepted quality rules. The original record and exact rejection reason remain evidence.
_Avoid_: Deleted row, repaired value

**Data Quality State**:
The publication outcome of a Data Quality Report: Passed has no detected quality issue, Degraded publishes usable Canonical Market Data with explicit gaps or warnings, and Rejected publishes no Canonical Market Dataset because trustworthy identity, schema, or valid data cannot be established.
_Avoid_: Model quality, silent partial success

**Continuous Bar Segment**:
A maximal uninterrupted sequence of Closed Bars for one Instrument and Bar Interval within a Bar Series. A Bar Gap separates adjacent Continuous Bar Segments.
_Avoid_: Gap-filled series, synthetic continuity

**Bar Open Time**:
The unique UTC instant at which an OHLCV Bar interval begins, represented at boundaries as Unix milliseconds. For a session-based market it is derived from the Venue-local Bar boundary under the exact Trading Calendar Snapshot.
_Avoid_: Local time, timestamp

**Bar Interval**:
The provider-neutral duration or calendar period used to group market activity into an OHLCV Bar. Supported values are `1s`, `1m`, `3m`, `5m`, `15m`, `30m`, `1h`, `2h`, `4h`, `6h`, `12h`, `1d`, `2d`, `3d`, `5d`, `1w`, `1mo`, and `3mo`. Continuous crypto markets align intervals to the recorded UTC grid; session-based markets align intraday intervals to Venue-local Trading Sessions and calendar intervals to Venue-local Trading Dates under an exact Trading Calendar Snapshot.
_Avoid_: Timeframe, provider bar string

**Multi-Day Interval**:
A `2d`, `3d`, or `5d` aggregation that is never anchored to the query start: continuous crypto markets use the recorded fixed UTC-day anchor, while session-based markets count consecutive Trading Dates and record their exact calendar and anchor Trading Date.
_Avoid_: Rolling query window, implicit calendar anchor

**Spot Instrument**:
A crypto instrument that exchanges a base asset for a quote asset without expiry or funding mechanics.
_Avoid_: Pair, spot pair

**Equity Instrument**:
A Venue-listed ownership security traded in shares and priced in one Price Currency, subject to Venue sessions, corporate actions, quantity rules, and settlement rules rather than a Spot Instrument's base/quote-asset mechanics.
_Avoid_: Company, Spot Instrument, generic symbol

**Financial Value**:
A price, quantity, volume, notional, balance, fee, or other amount whose base-10 representation must remain exact across domain and IPC boundaries.
_Avoid_: Float, approximate number

**Analytical Scalar**:
A Feature, Factor, or Forecast value represented at Python, Arrow, Host, and Component boundaries as one finite IEEE 754 binary64 value or an explicit Unavailable state with a reason. NaN, positive or negative infinity, object dtype, and pandas-inferred identity or numeric types are invalid boundary values; financial amounts, parameters, and target weights remain canonical Decimal strings instead.
_Avoid_: Financial Value, nullable object column, sentinel NaN

**Base Volume**:
The traded quantity in an Instrument's primary quantity unit during an OHLCV Bar: base-asset units for a Spot Instrument or shares for an Equity Instrument.
_Avoid_: Volume

**Quote Volume**:
The traded notional in a Spot Instrument's Quote Asset or an Equity Instrument's Price Currency during an OHLCV Bar; it remains unavailable when the provider does not publish trustworthy turnover.
_Avoid_: Turnover, volume

**FX Snapshot**:
Immutable, time-specific currency-conversion evidence with exact rates, sources, availability, and identities used to value a multi-currency portfolio. ADAQ V1 does not invent a global account total without this evidence.
_Avoid_: Current website rate, display conversion, fixed exchange rate

### Components

**Indicator**:
A common market-series calculation provided free by the ADAQ host and referenced by name and parameters.
_Avoid_: Factor, paid factor

**Indicator Catalog**:
The versioned host-owned registry of Built-in Indicators that ADAQ supports, including their inputs, parameters, outputs, and Warmup rules. It is an explicit public contract rather than every function present in the underlying analytical library.
_Avoid_: Raw TA-Lib function list, automatically discovered functions

**Factor Component**:
An independently packaged Component that declares exactly one Factor Scope plus ordered Feature Slots and transforms host-supplied, identity-preserving Factor Input Batches into one or more named scalar analytical values. Time-Series and Cross-Sectional Factors share the Factor product kind and package workflow but execute through different Factor ABI worlds.
_Avoid_: Indicator, strategy, trading plugin

**Factor Scope**:
The required Instrument cardinality and ordering contract for one Factor evaluation: Time Series processes one Instrument across ordered Observation Times, while Cross Sectional processes one Point-in-Time Instrument Universe at one Observation Time. Momentum, value, quality, volatility, and similar economic descriptions are Factor Tags rather than Scopes.
_Avoid_: Factor Tag, Model Scope, Asset Class

**Time-Series Factor**:
A Factor Component whose Instance processes observations for exactly one Instrument in causal Observation Time order, with state and Warmup isolated to that Instrument and reset at its Bar Gaps.
_Avoid_: Cross-Sectional Factor, time-series Model, Indicator

**Cross-Sectional Factor**:
A Factor Component that evaluates an explicitly identified Point-in-Time Instrument Universe at one Observation Time and returns Instrument-keyed outputs without substituting today's listings or silently dropping members.
_Avoid_: Cross-market validation, Time-Series Factor, portfolio ranking

**Cross-Sectional Factor Batch**:
The deterministic Instrument-ID-ordered input for one Cross-Sectional Factor evaluation, bound to one Observation Time, Point-in-Time Instrument Universe, Universe Evidence State, Feature contract, availability boundary, and explicit missingness.
_Avoid_: Anonymous matrix, Watchlist, current listings

**Factor Input Batch**:
A host-supplied table whose dense value columns match a Factor Component's ordered Feature Slots and whose rows preserve Instrument ID, Observation Time, availability, and missingness. A Time-Series Batch contains one Instrument in causal time order; a Cross-Sectional Factor Batch contains one Observation Time and its exact Universe.
_Avoid_: Raw provider payload, anonymous tensor, Component-owned data fetch

**Factor Dataset**:
An immutable collection of named Factor observations keyed by Instrument ID and Observation Time, bound to the exact Factor Package or research definition, Factor Scope, parameters, Feature Plan, Feature Dataset, Point-in-Time Instrument Universe when applicable, availability, missingness, runtime identity, and provenance.
_Avoid_: Mutable factor table, Feature Dataset, Validation Report

**Factor Candidate**:
An exact Declarative Factor Definition revision or non-imported Custom Factor Package selected for materialization and research before any Promotion Decision exists.
_Avoid_: Promoted Factor, Component Library package, editable draft

**Declarative Factor Definition**:
A versioned, immutable Factor specification built from an existing Feature Definition graph and frozen Feature Plan, ordered Feature Slots, Portable Parameter References, Feature Operator Catalog semantics, named outputs, and one Factor Scope. A Python Portable Definition builder constructs this canonical Host contract rather than introducing Python operators; because the Definition contains no arbitrary source code, ADAQ can evaluate it directly and deterministically generate an equivalent Rust SDK project.
_Avoid_: Custom Factor Project, notebook code, Factor Component

**Custom Factor Project**:
User-authored Rust source using the ADAQ Component SDK for Factor logic outside the Declarative Factor Definition operation set. ADAQ may build and execute a candidate in its sandbox, but never claims to translate arbitrary Python or notebook code into this project automatically.
_Avoid_: Declarative Factor Definition, imported Component Package, Python adapter

**Factor Tag**:
Optional descriptive metadata such as momentum, value, quality, volatility, liquidity, or size. Tags aid discovery and never select an ABI, evaluation method, or runtime behavior.
_Avoid_: Factor Scope, validation result, guaranteed exposure

**Factor Instance**:
A Run-scoped binding of one Factor Component to a unique alias, one exact parameter set, and the execution lifecycle required by its declared Factor Scope. The same Factor Component may have multiple Factor Instances in a Run.
_Avoid_: Factor copy, Component alias

**Strategy Component**:
An independently packaged Component that consumes host-supplied, identity-preserving inputs and emits a complete Target Decision or Portfolio Target under one declared Strategy Scope.
_Avoid_: Order executor, broker plugin

**Declarative Strategy Definition**:
A versioned, immutable Strategy specification assembled from one exact Declarative Strategy Operation Catalog, ordered required Input Slots, Portable Parameter References, and one Strategy Scope, producing scope-correct Target Decisions or Portfolio Targets. A Python builder may construct it, but arbitrary Python execution is not part of the Definition or its generated Component.
_Avoid_: Python Strategy Project, Strategy Component, order script

**Declarative Strategy Operation Catalog**:
The versioned Host-owned operation set that V1 limits to finite `weighted-sum`, deterministic `top-n`, `equal-weight`, and `cash-reserve` nodes. `top-n` sorts by descending finite score and then ascending Instrument ID, while loops, callbacks, optimizers, stops, orders, and Execution logic are not portable operations.
_Avoid_: Python standard library, Strategy ABI, plugin catalog

**Portable Parameter Reference**:
A typed Factor or Strategy Definition reference to one Manifest parameter whose V1 value set is finite and explicitly allowlisted. The selected research value becomes the generated Component default, every allowed combination must fit the Host cap and pass M14 conformance and equivalence, and Model training hyperparameters never become inference parameters through this mechanism.
_Avoid_: Arbitrary expression, floating parameter range, Model hyperparameter

**Strategy Scope**:
The declared Instrument dependency and decision cardinality of a Strategy: Single Instrument controls one Instrument independently, while Portfolio jointly allocates one exact Point-in-Time Instrument Universe.
_Avoid_: Strategy Architecture, Model Scope, Asset Class

**Single-Instrument Strategy**:
A Strategy Component that evaluates one Instrument and emits one complete Target Decision for its Strategy Instance.
_Avoid_: Portfolio Strategy, single-Instrument Training Universe

**Portfolio Strategy**:
A Strategy Component that jointly evaluates one exact Point-in-Time Instrument Universe and current Portfolio State within one Paper Portfolio and emits one complete Instrument-keyed Portfolio Target.
_Avoid_: collection of Single-Instrument Strategies, portfolio optimizer service

**Portfolio State**:
The host-observed cash, positions, reserved capital, pending-order exposure, and valuations controlled by one Strategy Instance within one Paper Portfolio at one decision time. It is decision input and never grants the Strategy Component custody or order authority.
_Avoid_: Portfolio Target, Venue account, mutable strategy memory

**Model Component**:
An independently packaged inference Component that consumes host-supplied Prediction Batches and emits Forecast Batches under a declared Model Scope and Model Deployment Profile. Its training engine is not part of the prediction contract.
_Avoid_: Training framework, Strategy Component, Signal Component

**Model Lab**:
The host-owned workflow that freezes training inputs and one Research Engine, launches controlled Model Training Attempts, validates and publishes Model Artifacts, generates Forecast Signal Datasets, evaluates them, and coordinates eligible exports.
_Avoid_: Training framework, notebook process, Model Component

**Model Training Runner**:
A versioned Model-kind execution contract that receives one frozen Model Training Protocol and local immutable inputs, runs its selected Research Engine Adapter through the Python Research Runner under explicit resources and cancellation, and returns candidate artifacts, metrics, diagnostics, and logs without writing ADAQ's authoritative records directly. It is not the separately qualified inference-only Local Qlib Paper runner.
_Avoid_: Model Component, arbitrary shell, Tauri command body

**Model Training Protocol**:
An immutable, content-addressed specification of the exact Feature and Factor inputs, Training Universe, Forecast Targets, training, validation and test windows, Host-owned fitting transformations, algorithm and hyperparameters, Research Engine, Model Research Adapter version, Seed, environment identity, and resource policy for one model experiment.
_Avoid_: Editable training form, Model Training Attempt, Model Artifact

**Model Training Attempt**:
A lifecycle record for executing one Model Training Protocol through Pending, Running, and exactly one terminal state: Completed, Failed, or Cancelled. Only a Completed Attempt whose outputs pass host validation may atomically publish a Model Artifact; partial checkpoints remain diagnostic evidence.
_Avoid_: Model Training Protocol, Model Artifact, Dataset Generation Attempt

**Model Exporter**:
A versioned adapter that converts one supported Model Artifact into deterministic inference payloads and packaging inputs for a WASI or ONNX Model Profile, declaring its supported algorithms, Artifact schema, numeric semantics, and runtime limits. An Artifact without a compatible export may qualify separately for Local Qlib Paper but never receives a false portable wrapper.
_Avoid_: Model Training Runner, External Model Adapter, generic serializer

**Model Research Adapter**:
A versioned, allowlisted bridge for one explicitly supported training algorithm and mode, defining its accepted typed parameters, Dataset view, fitting and prediction calls, canonical Artifact extraction and reload, random sources, repeatability rule, and eligible Model Exporters or Deployment Profiles. Importability or inheritance from a framework base class does not make an algorithm supported.
_Avoid_: Model Exporter, generic Python serializer, framework package

**Qlib Ridge Adapter**:
The first Model Research Adapter, supporting only Microsoft Qlib `LinearModel` in Ridge mode over the Qlib Dataset Bridge, one declared Continuous Forecast Target, and one Forecast Signal. It extracts and reloads a Linear Model Artifact rather than treating a Python pickle as authoritative.
_Avoid_: all Qlib models, Qlib data Provider, generic sklearn adapter

**Model Deployment Profile**:
The declared portability and runtime boundary under which a Model Artifact may perform inference: WASI Model, ONNX Model, or Local Qlib Paper. The Profile determines eligible execution contexts and required qualification evidence rather than model quality.
_Avoid_: Model Scope, Research Engine, performance tier

**WASI Model Profile**:
A portable Model Deployment Profile whose self-contained inference payload executes through the ADAQ Model Component contract.
_Avoid_: ONNX Model Profile, Local Qlib Paper Profile

**ONNX Model Profile**:
A portable Model Deployment Profile whose content-addressed ONNX graph and weights execute through a controlled ADAQ inference runtime while preserving the Model Component input and output contract.
_Avoid_: WASI Model Profile, arbitrary native library

**Local Qlib Paper Profile**:
A device-bound Model Deployment Profile that executes an exact original Qlib Model Artifact through a controlled local runner only for Paper Trading. It is not Marketplace, remote-deployment, or Real Trading eligible.
_Avoid_: Local Research Only, Python Bot, portable Model Component

**Model Runtime Qualification Report**:
Immutable evidence that one exact Model Artifact, Deployment Profile, runner and environment produce schema-valid, finite, timely Forecast Signals with the declared exact or tolerance-based replay behavior. Portable exports also require Component Equivalence; Local Qlib Paper qualifies the original runtime instead of claiming export equivalence.
_Avoid_: Forecast Evaluation Report, model quality score, successful process start

**Research-Only Model Artifact**:
A Model Artifact that lacks a valid Model Runtime Qualification Report for every Deployment Profile. It may produce offline research evidence but cannot supply a Bot.
_Avoid_: Local Qlib Paper Profile, failed export

**Model Instance**:
A configured inference resource created from one Model Component, its frozen Feature Slots, and a host-provided Inference Seed. It may retain only past causal context across ordered Prediction Batches, must produce bit-identical finite outputs for the same Package, Plan, Snapshot, parameters, Seed, and engine identity regardless of host chunk boundaries, and is rebuilt at Bar Gaps and Model Producer Segment boundaries.
_Avoid_: Model Artifact, training process, whole-history mutable predictor

**Model Scope**:
The declared Instrument dependency a Model requires for one inference unit: Single Instrument processes one Instrument independently, while Cross Sectional jointly processes one Point-in-Time Instrument Universe. It describes inference semantics rather than the Model's Training Universe.
_Avoid_: Asset Class, Strategy Instance, training universe

**Training Universe**:
The exact Instruments, point-in-time membership evidence, and observation range permitted to influence one Model's fitting and selection. A multi-Instrument Training Universe does not by itself make inference Cross Sectional.
_Avoid_: Model Scope, current listings, Backtest universe

**Single-Instrument Model**:
A Model whose prediction for one Instrument does not require observations from another Instrument in the same inference unit, even when it was fitted on a multi-Instrument Training Universe.
_Avoid_: single-Instrument training, Cross-Sectional Model

**Cross-Sectional Model**:
A Model whose prediction jointly depends on one exact Point-in-Time Instrument Universe at an Observation Time and returns Instrument-keyed Forecast Signals without substituting current listings or silently dropping members.
_Avoid_: panel-trained Single-Instrument Model, Cross-Sectional Factor

**Prediction Batch**:
A scope-correct ordered table of inference rows keyed by Instrument ID and Prediction Time with dense values matching a Model Component's Feature Slots. A Single-Instrument Batch contains one Instrument in causal Prediction Time order; a Cross-Sectional Batch contains one Prediction Time and its deterministic, identity-preserving Point-in-Time Instrument Universe.
_Avoid_: Strategy Feature Frame, anonymous tensor, training Dataset

**Forecast Batch**:
The ordered optional Forecast Signal Frames returned one-for-one for a Prediction Batch, preserving every Instrument ID, Prediction Time, and row position. The host attaches each Frame's Available At boundary; Multi-horizon predictions remain Signals on the originating row rather than generated future rows.
_Avoid_: Forecast Signal Dataset, arbitrary model response

**Available At**:
The earliest timestamp at which a Feature or Forecast Signal may legally be consumed. A derived Feature uses the latest Available At among its inputs and Fitted Transformation Artifact; provider publication evidence, rather than an event's effective date or local computation time, governs when external information became available. M8 native Closed-Bar inference uses the input Bar's close boundary for both Prediction Time and Available At, with execution no earlier than the next Bar.
_Avoid_: Prediction Time, file creation time, unrestricted execution time

**Observation Time**:
The event-time boundary that identifies one Feature observation, separate from Available At, which determines when that observation may legally be consumed.
_Avoid_: Available At, processing time, file creation time

**Bot Decision Schedule**:
The immutable rule identifying when a Trading Bot may evaluate its Strategy: either after a declared Bar Interval becomes a confirmed Closed Bar or at a declared Venue-local scheduled batch boundary. V1 has no Tick-driven or order-book-triggered Strategy schedule.
_Avoid_: Worker heartbeat, provider polling interval, order execution timing

**Decision Time**:
The scheduled event-time boundary at which one Strategy decision's eligible inputs, Point-in-Time Instrument Universe, and availability are frozen. Every consumed input must have `Available At <= Decision Time`; later arrival or processing never backdates information into that decision.
_Avoid_: Processing completion time, order submission time, wall-clock now

**Decision Batch**:
The complete scope-correct, identity-preserving input supplied for one Decision Time after Warmup and availability validation. A Time-Series Decision Batch belongs to one Instrument and Closed Bar, while a Cross-Sectional Decision Batch contains its exact deterministic Point-in-Time Instrument Universe and explicit missingness without silently dropping late members. If any required Strategy Input Slot is unavailable for any required member, the Host records `Run Pause::MissingInput` and does not invoke the Strategy; an optional Eligibility Definition would require a future explicit auditable contract.
_Avoid_: Tick callback, partial Watchlist, anonymous feature matrix

**Decision Deadline**:
The frozen wall-clock limit by which a live Worker result must be validated for one Decision Time. A result received after the Deadline is retained as late evidence but cannot authorize new risk for that decision.
_Avoid_: Decision Time, Model horizon, provider timeout

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
A content-addressed installable bundle containing a Component's exact runtime payloads and authoritative Component Meta. Validation Reports and trust records remain separate and reference the exact package identity.
_Avoid_: Bare WASM file, plugin DLL

**Component Build Attempt**:
An immutable background-work record binding exact generated or User-authored source, source hash, SDK and ABI versions, template or generator version, compiler and target identity, commands, status, diagnostics, logs, and candidate Package hash. Failed and cancelled Attempts publish no importable Package but retain their evidence.
_Avoid_: Component Package, terminal buffer, blocking UI action

**Component Equivalence Report**:
Immutable evidence that one candidate Package reproduces the approved research definition or artifact on exact frozen inputs, preserving row identities, availability, missingness, output contracts, and required numeric equality under the declared runtime identity.
_Avoid_: Factor Evaluation Report, Component validation, approximate manual comparison

**Generated Component Provenance**:
An immutable host record linking one generated Component Package hash to its exact Python Project Revision and canonical Declarative Definition or Linear Model Artifact, parameter schema, Promotion or Selection Decision, Build Attempt, Component Equivalence Report, generator, SDK, ABI, and toolchain identities. Generated `.adaq` packages contain the Rust-generated WASM and Component metadata, not Python source, Runtime, wheels, environment, or Dependency Lock; research metrics remain in their Reports rather than being copied into Component Meta.
_Avoid_: Component Meta, build log, performance claim

**Component Library**:
A User's device-resident collection of imported Component Packages together with their Component Entitlement records; it is where Component deletion locks and compatibility lookups are resolved.
_Avoid_: Component store, plugin folder

**Marketplace Model Candidate**:
An exact, immutable Model Artifact and proposed Deployment Profile submitted for future Marketplace review together with its provenance, rights, runtime, security, and qualification evidence. Submission does not make it installable, trusted, or eligible for trading.
_Avoid_: Local Qlib Paper model, uploaded Python project, Marketplace Model Product

**Marketplace Model Product**:
An exact version of a Model Component accepted for Marketplace distribution under one declared Deployment Profile, publisher identity, licence, entitlement policy, signed payload identities, and Marketplace Review Decision.
_Avoid_: Qlib framework, mutable model listing, training Dataset

**Marketplace Review Decision**:
An immutable Accepted, Rejected, Suspended, or Withdrawn decision citing the exact Marketplace Model Candidate and the technical, security, provenance, and redistribution-rights evidence applied to it. Acceptance grants distribution eligibility only and never implies profitability or Real Trading Qualification.
_Avoid_: Model Runtime Qualification Report, performance ranking, real-money approval

**Real Trading Qualification**:
A future risk and operational gate that is separate from research quality, Paper Trading, and Marketplace acceptance. No Model or Strategy becomes eligible for real funds merely because it is published or runs successfully in Paper Trading.
_Avoid_: Marketplace acceptance, Paper Trading success, profitability claim

**Component Meta**:
Stable information describing a Component's identity, versions, parameters, inputs, outputs, dependencies, warmup, and licensing.
_Avoid_: Backtest result, performance claim

**Validation Protocol Draft**:
The mutable, user-edited choice of exact Backtest Run provenance and method-specific inputs from which ADAQ can freeze a Validation Protocol. It has no persistent identity and is not research evidence.
_Avoid_: Validation Protocol, editable Protocol, pending Report

**Validation Protocol**:
An immutable, content-addressed specification of exact Run configuration, validation method, aggregation rule, and frozen method-specific data boundaries from which Validation Reports are produced.
_Avoid_: Validation Protocol Draft, Validation Report, saved form

**Validation Report**:
Historical evidence associated with an exact Component Package hash, data snapshot, configuration, period, and validation method; it does not guarantee future performance. Walk-forward and cross-market validation studies record their study reports under this same term while binding them to a Validation Protocol and Backtest Run evidence rather than to a Component Package hash.
_Avoid_: Component Meta, profitability guarantee

**Factor Evaluation Lens**:
One declared way of examining a Factor Dataset independently from its Factor Scope: Temporal measures within-Instrument predictive behavior through time, Cross Sectional measures same-time ranking behavior across a Point-in-Time Instrument Universe, and Economic runs a standardized cost-aware portfolio sort as diagnostic evidence rather than a deployable Strategy.
_Avoid_: Factor Scope, Strategy Backtest, metric preset name

**Factor Research Family**:
An immutable hypothesis and trial namespace grouping related Factor definitions, parameter searches, targets, Evaluation Protocols, and known derived Families so selection-bias adjustments account for what was tried rather than only the winning Report.
_Avoid_: Factor Tag, Component family, deletable experiment folder

**Factor Trial**:
One registered Candidate, parameter set, Target, market context, and Evaluation Protocol within a Factor Research Family lineage, retained whether it completes, fails, is cancelled, or is superseded.
_Avoid_: Winning Report, temporary run, deletable search result

**Factor Materialization Protocol**:
An immutable, content-addressed specification binding one Factor Candidate to exact Feature evidence, parameters, market context, runtime identity, observation range, and Seed for Factor Dataset production.
_Avoid_: Factor Evaluation Protocol, mutable Factor form, Feature Materialization Request

**Factor Materialization Attempt**:
A lifecycle record for executing one Factor Materialization Protocol through Pending, Running, and exactly one terminal state: Completed, Failed, or Cancelled. Only a Completed Attempt may atomically publish a Factor Dataset.
_Avoid_: Factor Dataset, Factor Evaluation Report, generic Job

**Factor Evaluation Protocol**:
An immutable, content-addressed specification binding one exact Factor Dataset output to its Forecast Target and horizons, Market Data and Feature evidence, Point-in-Time Instrument Universe, Research Engine, Evaluation Lenses, chronological or walk-forward windows, purge and embargo rules, neutralization, portfolio-sort and cost assumptions, and Factor Research Family Trial identity.
_Avoid_: Validation Protocol, editable Factor test form, Factor Evaluation Report

**Factor Evaluation Report**:
Immutable evidence produced from one Factor Evaluation Protocol, recording coverage, missingness, stability, decay, scope-appropriate Temporal or Cross-Sectional statistics, standardized Economic results, subperiod and regime behavior, multiple-testing adjustments, diagnostics, and Evaluation Evidence State. It never declares universal validity or future profitability.
_Avoid_: Validation Report, Factor Dataset, Factor Promotion Decision

**Factor Metric Catalog**:
The versioned authoritative registry of Factor evaluation metrics, formulas, directions, ranges, undefined states, and evidence requirements shared by every Research Engine and the GUI.
_Avoid_: UI metric labels, Qlib metric names, Strategy Metric Catalog

**Factor Orientation**:
The Positive or Negative direction frozen for one Factor output and Forecast Target in an Evaluation Protocol. It controls interpretation and Economic sorting without changing the Factor Dataset's raw values.
_Avoid_: Factor value sign, automatic validity, Dataset transformation

**Factor Regime Definition**:
An immutable partition of evaluation observations into deterministic buckets using one causal Feature and thresholds fitted only from an exact selection window and then applied unchanged.
_Avoid_: Subjective bull or bear label, current market state, mutable threshold

**Factor Promotion Policy**:
An immutable, versioned set of evidence-completeness, repeatability, and metric thresholds that a User applies when recording a Factor Promotion Decision. It constrains eligibility without automatically making the decision; a Python Candidate cannot be promoted from an exploratory result whose Repeatability State is Unverified or Divergent.
_Avoid_: Automatic promotion rule, universal profitability threshold, mutable preference

**Factor Promotion Decision**:
An immutable User decision of Rejected, Research Validated, or Component Eligible that cites exact Factor Evaluation Reports and the applied acceptance policy. A Candidate has no Promotion Decision yet; Research Validated evidence may be selected by Model research but cannot enter Component Generation, while Component Eligible also permits a User-controlled qualification and packaging attempt. The Decision does not alter its Reports or turn their historical evidence into a timeless `valid` property.
_Avoid_: Automatic validity flag, Factor Evaluation Report, Component trust

**Promoted Factor Library**:
A User-scoped read-only projection of exact Factor outputs with current Research Validated or Component Eligible Decisions. It references evidence in place and never copies Datasets, Reports, Decisions, or Component Packages.
_Avoid_: Component Library, latest Factor pointer, mutable factor store

**Forecast Evaluation Report**:
Immutable evidence of Forecast Signal quality bound to the exact Model Producer Segments, Forecast Signal Dataset, Market Data Snapshot, Forecast Targets, inference trust state, evaluation window, and evaluation configuration. It labels its Evidence State as Out-of-sample, Overlapping, or Unknown from the recorded training, fitting, and normalization windows; it evaluates predictions rather than Strategy decisions or profitability.
_Avoid_: Validation Report, Backtest result, profitability claim

**Evaluation Evidence State**:
The relationship between evaluation observations and the recorded research, training, fitting, normalization, parameter-selection, target-construction, and prior result-review windows that influenced a Factor, Model, or Strategy. Out-of-sample requires complete evidence, a frozen Parameter Selection Decision, and no overlap or prior use of its Final Evaluation results; Overlapping records known reuse or feedback, and Unknown preserves incomplete provenance without inventing an out-of-sample claim.
_Avoid_: Model trust state, performance grade, automatic validation

**Parameter Selection Decision**:
An immutable User choice of one exact Factor, Model, or Strategy Trial after comparing only its declared Selection Window evidence. It freezes the selected Project Revision, parameter set, inputs, lineage, and selection metrics before one disjoint Final Evaluation may expose its result; using that result to modify or choose another candidate creates derived lineage whose affected evidence is Overlapping.
_Avoid_: Factor Promotion Decision, automatic best Trial, editable winner flag

**Paper Feedback Snapshot**:
An immutable, time-bounded selection of one exact Bot Deployment Bundle's Paper market observations, Decision Batches, Feature and Forecast outputs, Strategy Targets, Risk Decisions, Orders, Fills, account and reconciliation events, operational conditions, realized outcomes, and completeness evidence. It references Canonical Market Data and execution journals without converting account events into market data.
_Avoid_: mutable live metrics, Market Data Snapshot, Paper Account Snapshot

**Paper Feedback Report**:
Immutable post-deployment evidence produced from one Paper Feedback Snapshot under one frozen Factor, Model, Strategy, or Execution feedback lens. It records realization horizons, sample sufficiency, missingness, comparability, metrics, drift or divergence diagnostics, and Evaluation Evidence State without automatically changing a Component, Bot, or promotion decision.
_Avoid_: Operational Alert, automatic retraining trigger, Backtest Report

**Research Review Decision**:
An immutable User decision citing exact Paper Feedback Reports and choosing No Change, Pause Bot, New Factor Evaluation, New Model Training, New Strategy Backtest or restructuring, or another explicit research action. Any changed logic proceeds through new Attempts, promotion, equivalence, validation, and Deployment Qualification and creates a new Bot Deployment Bundle.
_Avoid_: Alert acknowledgement, hot replacement, mutable Bot setting

**Component Dependency Mode**:
The origin and lifecycle of a Component input: Built-in is free host functionality, Embedded is compiled into the consuming Component, and External is a separately packaged Factor or Model Component resolved by the host.
_Avoid_: Component-to-component call, automatic runtime download

**Component Lock**:
The immutable record of the exact Component packages, hashes, contracts, and trust states selected for a Run.
_Avoid_: Latest version, floating dependency

**Feature Slot**:
A stable, uniquely named, ordered input declared by a Factor, Model, or Strategy Component together with its exact Market Field, Indicator, Factor, transformation, or Forecast Signal semantic requirement and bound by the host to one finite analytical scalar or explicit missing value before execution. A Strategy Forecast Signal Slot declares the required Prediction Kind, Forecast Target, horizon, and value contract rather than a Model or Dataset identity; the frozen Feature Plan records the selected Dataset, Signal name, and producing Model provenance. Component dependency graphs are acyclic, and users may configure only declared parameters and compatible bindings; they cannot add, rename, remove, reorder, or arbitrarily replace a Component's slots.
_Avoid_: JSON feature map, dynamic lookup

### Research and Execution

**Research Engine**:
The exact analytical backend selected by a frozen research protocol to produce Features, Factors, Models, or research simulation evidence. It never owns Canonical Market Data or grants deployment authority.
_Avoid_: Market Data Provider, Model Component, deployment runtime

**Manual Research Workflow**:
The User-directed, Host-owned sequence from frozen inputs and a research Attempt through immutable Dataset, Evaluation, Selection, Promotion, Qualification, and Deployment decisions. Each gate preserves its own evidence and requires the exact prerequisite rather than inferring completion from a score or a previous screen.
_Avoid_: Best-score pipeline, notebook flow, automatic winner selection

**AI Research Controller**:
A future Automation Surface that may propose research actions and request bounded Host Attempts using the Manual Research Workflow. It owns neither final-evaluation authority nor User Promotion, Qualification, Deployment, or Bot-Start decisions and cannot create a parallel evidence path.
_Avoid_: Autonomous researcher, promotion agent, deployment agent

**Advisory Artifact**:
A non-authoritative AI-generated suggestion or comparison that cites its input evidence and provenance but cannot become research evidence, a User Decision, or deployment authority by itself.
_Avoid_: Evaluation Report, automatic decision, model artifact

**Research Automation Grant**:
A User-scoped, expiring permission that lets an AI Research Controller perform bounded mechanical research actions for declared Projects, Revisions, Attempt kinds, and resource limits. It never grants authority over credentials, policies, evidence deletion, User Decisions, Risk, Execution, or Deployment.
_Avoid_: AI role, blanket trust, autonomous deployment permission

**Qlib Research Engine**:
The default Research Engine for ADAQ V1, using Microsoft Qlib semantics over immutable ADAQ inputs and preserving Qlib-labelled outputs and reports. It is not an independent source of authoritative market data or a direct trading runtime.
_Avoid_: Qlib data downloader, ADAQ deployment engine, Python Bot

**Qlib Dataset Bridge**:
The read-only `adaq.qlib` adapter view that converts Host-supplied Arrow partitions into stable pandas tables indexed by `(datetime, instrument)` and implements only the supported `DatasetH.prepare()` surface for `train`, `valid`, and feature-only `test` Segments. It never initializes a Qlib data Provider, downloads data, constructs Alpha158 inputs, queries a network, or makes Qlib storage authoritative.
_Avoid_: Qlib data directory, Provider URI, Dataset cache

**ADAQ Native Research Engine**:
The optional Research Engine that uses ADAQ's existing Feature, Factor, Model, and Run semantics. Its results remain distinct from Qlib results unless an explicit equivalence check establishes the claimed relationship.
_Avoid_: Legacy mode, Qlib compatibility mode

**Python Research SDK**:
The single versioned public contract through which a Python Research Project receives ADAQ-authorized frozen inputs and returns typed Factor, Model, or Strategy research outputs through kind-specific modules. Its source lives at `python/adaq-research-sdk/`, and its public wheel is distributed as `adaq-research-sdk` with the `adaq` import namespace and public `adaq.qlib` bridge, while ADAQ executes only the exact bundled SDK Artifact frozen by a Project Revision; it grants no generic data-query surface or direct access to authoritative stores, credentials, order APIs, or deployment authority.
_Avoid_: Component SDK, Qlib API, trading API

**Python Research Core**:
The Tauri-independent Rust crate at `src-tauri/crates/adaq-python-research` that owns Python Manifest, Archive, Revision, Trust, Protocol, Resource, and staged Result contracts without owning SQLite, files, processes, Factor evidence, Model evidence, or GUI state.
_Avoid_: Python Research SDK, Tauri command module, Factor Research Store

**Python Research Control Plane**:
The sole application orchestration boundary for Python Research: it owns Project, Trust, Runtime, Environment, Runner, Attempt, recovery, and reset lifecycles, coordinates their ordering, and attaches work to the shared Research Queue. Its persistence and runner details remain private; Factor and Model modules retain their evidence lifecycles and receive only validated Host results.
_Avoid_: Python Research Runner, Python Research Core, Tauri UI thread work

**Python Research Project**:
A User-authored, locally editable source project whose immutable lower-kebab-case ID begins with `py-factor-`, `py-model-`, or `py-strategy-` exactly matching its declared Kind. It exposes one stable Python Project Entry Point and uses the Python Research SDK for offline research. It may contain multiple Python modules and outputs, remains private unless explicitly exported, and a shared Project includes its source; notebooks may explore it but are not authoritative execution or publication sources.
_Avoid_: Python Component, Python Bot, notebook-to-WASM package, multipurpose research pipeline

**Python Project Layout**:
The fixed source-visible root containing `adaq-project.toml`, `pyproject.toml`, ADAQ-generated `pylock.toml`, `src/project.py`, `README.md`, and `LICENSE`, with optional additional declared Python modules under `src/` and declared localized documentation such as `README.zh-CN.md`. The default entry point is `project:create_project`; `setup.py`, `requirements.txt`, shell launchers, Datasets, environments, and results are not authoritative Project files.
_Avoid_: Python Project Environment, notebook folder, arbitrary application repository

**Python Project Entry Point**:
The exact Manifest `module:function` reference to a zero-argument `create_project()` function. After the User trusts the exact Revision, the Runner imports that module, calls the function once for the current lifecycle boundary, and requires the returned SDK object to match the declared Kind and Mode; it performs no file, class, decorator, or framework discovery, and supplies parameters, Seed, data, progress, and diagnostics only through later typed Context calls.
_Avoid_: Import-time project object, plugin scan, command-line entry point

**Python Project Kind**:
The single Factor, Model, or Strategy contract declared by a Python Research Project, determining its typed inputs, outputs, and evidence lifecycle without combining those domains into one executable project.
_Avoid_: Python package type, Research Engine, mixed Factor-Model-Strategy script

**Python Project Mode**:
The explicit authoring boundary of a Factor- or Strategy-kind Python Research Project: Imperative Python executes the Project for Research Only, while Portable Definition uses Python only to construct a canonical Declarative Factor or Strategy Definition for Host execution and later Component generation. Model Projects use registered Model Exporters instead of this Mode.
_Avoid_: Automatic Python transpilation, inferred portability, Model Deployment Profile

**Python Project Manifest**:
The exact-schema `adaq-project.toml` execution contract declaring one Project's immutable Kind-prefixed ID, Kind, Mode when applicable, Scope, stable entry point, SDK and Runtime Profiles, typed parameters, ordered Python Input Slots and outputs, Target or Signal contracts, Dependency Lock hash, and resource requests that cannot exceed the Host's Python Resource Policy. It declares portable requirements rather than local Dataset IDs. Unknown fields, enum values, and unsupported schema versions fail static validation and cannot Prepare or Run; incompatible source and historical evidence remain inspectable and exportable without automatic rewriting. Name, description, tags, README, and licence are presentation or distribution metadata rather than execution identity.
_Avoid_: Component Manifest, README, setup script

**Python Input Slot**:
One stable, ordered Manifest requirement for a scope-correct Market, Feature, Factor, Forecast Signal, Target, or Portfolio input contract. A Lab Run binds each Slot to an exact local Snapshot, Dataset, Universe, Promoted Factor, or other compatible evidence identity and freezes that binding in the Attempt; Project code cannot query a latest or similarly named local object dynamically.
_Avoid_: Local Dataset ID in source, dynamic data query, pandas column guess

**Python Project Working Copy**:
The User-scoped mutable Project directory created or copied into ADAQ local data and opened in the User's external editor. Its independently visible state is Clean, Dirty, or Invalid; file changes mark it Dirty but never execute automatically or alter evidence. Run freezes the current content as a new Python Project Revision, and an imported external directory is not kept in implicit synchronization. Project ID changes create a different Project rather than renaming historical identity.
_Avoid_: Python Project Revision, external linked folder, built-in template

**Python Project Revision**:
An immutable, content-addressed snapshot of one Python Research Project's Manifest and declared source that resolves its Runtime Profile to an exact platform-specific Runtime artifact. Editing the working Project creates another Revision and never changes a running or historical Attempt.
_Avoid_: Mutable project folder, notebook checkpoint, Git branch

**Python Project Archive**:
A content-addressed deterministic ZIP containing exactly one validated Python Project Layout, with no Dataset, result, environment, cache, secret, or undeclared file. Offline import rejects absolute or parent-traversing paths, symbolic or hard links, duplicate or case-fold-colliding paths, undeclared entries, count or size limit violations, and Lock hash mismatch before copying an inert Untrusted Working Copy; import never loads a module, prepares an Environment, or executes code. It is source-visible and uses no opaque ADAQ-specific binary extension.
_Avoid_: Component Package, Python wheel, project directory with local artifacts

**Python Project Licence**:
The required Manifest declaration and matching `LICENSE` content governing an exported Project Archive. Local private Projects may use `LicenseRef-Proprietary`; a future zero-price Community Python Project must instead use an explicit SPDX open-source licence that permits source redistribution, because zero price alone grants no rights.
_Avoid_: Component Entitlement, missing licence, zero-price permission

**Python Project Trust Decision**:
An explicit User authorization to execute one exact Python Project Revision locally. A tutorial may present several exact Revisions in one confirmation but records one Decision per Revision; import, installation, bundled-example provenance, Marketplace review, name continuity, and trust in another Revision do not grant it, and any source, entry-point, or Dependency Lock change requires a new Decision for only the changed Project.
_Avoid_: Package installation, publisher reputation, permanent project trust

**Python Dependency Lock**:
The ADAQ-generated `pylock.toml` containing the exact resolved Python runtime and wheel identities, versions, hashes, and platform constraints required to prepare a reproducible Project Environment. `pyproject.toml` records editable dependency intent; explicit Sync Environment resolves it under trusted-index, wheel-only, and hash policies and atomically replaces the Lock. Run never resolves or rewrites dependencies, and manual Lock edits must pass structural and hash validation. Dependency ranges alone, source distributions, the system Python, and a mutable virtual environment are not Locks.
_Avoid_: requirements range, active virtualenv, runtime pip install

**ADAQ Python Runtime**:
An exact, platform-specific CPython distribution installed on demand and managed only inside ADAQ local data for compatible Python Project environments. A system interpreter, User-selected executable, or silently upgraded Runtime is not an ADAQ Python Runtime.
_Avoid_: System Python, custom interpreter path, active virtualenv

**Python Runtime Profile**:
A versioned logical compatibility contract declared by a Python Project Manifest and resolved by ADAQ to one supported CPython version and exact platform Artifact hash. Publishing a new Profile never changes the Runtime identity frozen by an existing Project Revision or Attempt.
_Avoid_: Python version range, latest Python, local interpreter discovery

**Python Project Environment**:
A prepared, content-addressed binding of one exact ADAQ Python Runtime and Python Dependency Lock used to execute compatible Project Revisions. Its cached bytes may be removed and reconstructed without deleting the identities retained by historical evidence.
_Avoid_: Active virtualenv, system site-packages, Python Project Revision

**Python Environment Preparation Attempt**:
A lifecycle record created by explicit Sync or Prepare Environment that downloads, verifies, and stages one exact Python Project Environment before atomically publishing it as usable. It is distinct from a Python Research Attempt and never enters the heavy Research Queue; matching active requests coalesce, Failed or Cancelled staging is never executable, and Retry creates a new Attempt. Preparation does not grant Project execution trust.
_Avoid_: Python Research Attempt, pip install during Run, terminal setup

**Python Research Schema**:
The exact device-local `PYTHON_RESEARCH_SCHEMA_VERSION`, initially `1.0.0`, governing Project Revision, Attempt, Trust, local binding, and result metadata owned by the Python Research Control Plane. An incompatible value puts Python Research into a read-only ResetRequired state: source inspection, listing, and Archive export remain available, while lifecycle mutations and execution wait for an explicit device-level Reset Python Research Evidence; no migration, dual reader, or automatic deletion is implied.
_Avoid_: Factor Research Schema, Python SDK version, SQLite application version

**Reset Python Research Evidence**:
The explicit device-level operation that stops a User's Python research and preparation work, removes that User's Python Revision, Attempt, Trust, binding, and result metadata, and preserves User-authored Working Copies and exported Project Archives. Runtime, Wheelhouse, and Environment bytes remain shared independently evictable Cache and are not removed merely because one User resets; Factor evidence uses its separate Factor Research Reset.
_Avoid_: delete Python projects, remove all local data, cache cleanup

**Python Research Runner**:
The private bundled wheel sourced from `python/adaq-python-research-runner/` and launched by managed CPython in isolated mode as `-I -m adaq_runner` to execute exactly one Python Research Attempt for one Factor, Model, or Strategy Project Revision before exiting. It connects only to a random Host-owned loopback port using a one-time token, completes an exact Protocol, SDK, Revision, and Attempt handshake, and treats stdout and stderr only as capped logs. It never survives across Attempts, publishes authoritative records, exposes its protocol client as public SDK API, or serves as the separately qualified Local Qlib Paper inference runner.
_Avoid_: Python Project, Model Component, Local Qlib Paper runner

**Python Runner Protocol**:
The versioned private cross-process contract between the Host and one Python Research Runner. Length-prefixed canonical JSON carries bounded control messages, Arrow IPC carries typed tabular frames, and large artifacts pass only through a private Attempt Staging Area with declared hashes; incompatible handshakes fail before Project execution.
_Avoid_: Public network API, stdout JSON protocol, shared temporary file

**Python Attempt Staging Area**:
The private, Attempt-scoped filesystem area where a Runner may place declared candidate outputs before Host validation. The Runner cannot write ADAQ's database or final Dataset locations; the Host verifies identity, ordering, schema, availability, finite values, Decimal rules, size, and hashes before atomic publication, and discards non-diagnostic partial outputs after Failure or Cancellation.
_Avoid_: Final Dataset, Project workspace, shared artifact directory

**Python Resource Policy**:
The versioned Host-owned limits for Python wall time, memory, threads, input rows, columns and cells, protocol and artifact sizes, declared diagnostic checkpoints, logs, and Strategy decision deadlines. A Project may request smaller values but cannot raise Host caps; exact supported-platform values are fixed from implementation benchmarks rather than guessed in the Manifest.
_Avoid_: User-unbounded resources, Project-owned timeout, framework default

**Research Queue**:
The device-wide FIFO admission boundary for heavy Feature, Factor, Python, Model, and Strategy research work. A Pending Attempt enters the strict cross-kind order only at durable Queue admission; one heavy work item is scheduled at a time, while User-scoped Attempt ownership, cancellation, reset, recovery, and evidence remain with the relevant research domain. Environment preparation is separately serialized and does not create a competing heavy-research worker pool.
_Avoid_: Tauri command body, Python worker pool, UI loading queue

**Research Queue Admission**:
The durable, idempotent point at which a Pending Attempt receives its device-wide ordering position. FIFO is defined by Admission order rather than request time, Store-local timestamps, or UI submission time; a Retry always receives a new position.
_Avoid_: enqueue click, created-at ordering, automatic retry

**Python Research Attempt**:
An append-only execution record binding one Python Project Revision and prepared environment to exact frozen Input Slot bindings, one normalized parameter set, Seed, deterministic runtime settings, resource policy, status, outputs, diagnostics, and logs. Pending work survives restart in FIFO order, while stale Running work becomes Failed with an Interrupted reason rather than resuming. Cancel requests are cooperative for a bounded grace period and then terminate the process tree; Cancelled is recorded only after Runner exit and staging isolation. Source edits, parameter changes, and Retry create new Attempts, active duplicate Starts coalesce only on complete execution identity, and late Runner results never mutate prior evidence.
_Avoid_: Python process, terminal session, hidden parameter sweep

**Python Research Context**:
The kind- and phase-specific immutable SDK view of Host-authorized parameters, Seed, identities, frozen inputs, event times, and bounded `progress` and `diagnostic` emitters supplied after Project creation. It exposes no current wall clock, GUI object, or authoritative store; reading ambient time or using undeclared randomness is outside the reproducible contract. Stdout and stderr remain capped unstructured logs, while an uncaught exception or invalid required result fails the Attempt without falling back to historical output.
_Avoid_: Global application object, mutable Dataset handle, logging backend

**Python Parameter Grid**:
A finite Host-owned Cartesian product of typed Manifest parameter values. ADAQ normalizes and bounds the combinations, registers each as a separate Trial and Python Research Attempt in the appropriate Factor Family, Model Experiment, or Strategy study, and never accepts a hidden script-owned Sweep as equivalent evidence.
_Avoid_: Optuna study, Bayesian optimizer, loop inside one Attempt

**Python Repeatability Report**:
Immutable evidence from replaying one exact Project Revision, Environment, Input bindings, parameter set, and Seed in a fresh process, including permitted Batch partitions. Its Repeatability State is Unverified, Verified, or Divergent; Factor and Strategy require exact output equality, while a Model's registered Deployment Profile declares exact or bounded numeric tolerance. Exploratory outputs remain inspectable when Unverified or Divergent, but they cannot pass Promotion, Component Generation, or Runtime Qualification gates.
_Avoid_: Component Equivalence Report, successful first run, cross-version benchmark

**Python Tutorial Fixture**:
The Host-owned immutable `python-tutorial-a-share@1` synthetic Dataset fixture stored outside every Project Archive under `src-tauri/fixtures/python-tutorial/`. Its Manifest binds clearly fictional identities, Instrument Master, Calendar, and daily Bars for exactly 12 A-share-like Instruments across 180 Trading Sessions; it requires no network and makes no claim about an issuer, live market, or profitable pattern.
_Avoid_: bundled Project data, provider sample, historical A-share Dataset

**Python Research Tutorial**:
The guided bilingual Factor → Model → Strategy journey launched by Run Python Tutorial using the three bundled `py-` Projects and one Python Tutorial Fixture. It may automate validation, preparation, Attempts, and navigation but cannot create a Factor Promotion Decision, Parameter Selection Decision, Final Evaluation claim, or Project Trust Decision without explicit User confirmation at the relevant boundary.
_Avoid_: unattended benchmark, automatic promotion pipeline, profitability demo

**Python Factor Candidate**:
A Factor Candidate whose source binds one exact Factor-kind Python Project Revision and environment and whose execution materializes the standard Factor Dataset consumed by existing evaluation, Research Family, and Promotion workflows. A Portable Definition Project implements `define(context)` without Dataset access and returns one canonical Declarative Factor Definition. An Imperative Python Project implements `evaluate(context, batches)` and yields scope-correct Factor Output Batches without crossing Host-declared Continuous Bar Segment boundaries; a Bar Gap creates a new Project object. Arbitrary Python may become Research Validated after repeatability and existing evidence gates pass, but is not Component Eligible unless re-expressed through a supported portable definition.
_Avoid_: Declarative Factor Definition, Custom Factor Package, Python Factor Component

**Python Model Project**:
A Model-kind Python Research Project that trains against Host-frozen partitions and returns a typed Model Artifact and Forecast Signals through a declared Model Research Adapter. Its `fit(context)` phase can see only Train and Validation inputs and labels; the Adapter persists and reloads the exact candidate Artifact before `predict(context, fitted_model)` produces Validation or Test Forecasts. Test labels remain Host-only and all final evaluation metrics are Host-computed. The first Qlib Ridge Adapter accepts exactly one Target, horizon, and Forecast Signal per Project. Custom Python preprocessing or another framework remains Research Only unless its complete transformation provenance, Adapter, Exporter, and Deployment Profile are separately qualified.
_Avoid_: Qlib notebook, Model Component, Python trading runtime

**Python Strategy Project**:
A Strategy-kind Python Research Project whose `start(context)` creates one Strategy Session for a Host-declared continuous input segment. The Host then serially calls `decide(decision_batch, portfolio_state)` at each Decision Time and accepts only one complete Target Decision or Portfolio Target before applying Host-owned Risk, Execution, and Portfolio updates; the Project cannot prefetch future batches, and a Bar Gap creates a new Project and Session. It never emits orders, owns Risk or Execution, or turns a Qlib-native backtest into an ADAQ Backtest.
_Avoid_: Python Bot, order script, Qlib Research Backtest

**Python Strategy Session**:
The Attempt-local causal object returned by a Python Strategy Project's `start(context)` and invoked exactly once per ordered Decision Batch until its Host-declared segment ends. Supported mutable causal state lives only in the returned Project or Session object and is discarded at the segment boundary; mutable module globals, ambient disk caches, and cross-Attempt state are outside the contract. Calls are never pipelined; a missed deadline, exception, incomplete Target, identity change, or invalid value fails the whole Attempt and publishes no partial Backtest result.
_Avoid_: Trading Bot, Portfolio State, whole-history backtest function

**Community Python Project**:
A future Marketplace listing for one immutable, source-visible Python Research Project version under an explicit SPDX open-source licence and a fixed zero price. Listing, installation, and popularity grant no execution trust, research validity, Component eligibility, Paper Trading authority, or Real Trading Qualification.
_Avoid_: Marketplace Component Product, free trial, trusted Python package

**Research Engine Provenance**:
The immutable binding of a research result to its exact Research Engine, version, Adapter, environment, parameters, and input identities. Results from different engines are compared side by side rather than silently merged as equivalent evidence.
_Avoid_: Framework name, mutable environment note

**Research Backtest Report**:
Immutable portfolio-simulation evidence produced under one Research Engine's own semantics. It is not an ADAQ Backtest Run and cannot by itself qualify a Component or Bot for Paper Trading.
_Avoid_: Backtest Run, Validation Report, Deployment Qualification

**Deployment Qualification**:
The ADAQ-owned gate allowing an exact set of Models, Components, frozen inputs, Risk Policy, and Execution Profile to enter Paper Trading only when it cites the profile-appropriate Model Runtime Qualification, ADAQ-native replay, Component Equivalence where applicable, Backtest Run, and Validation evidence. Research Backtest Reports alone never satisfy it.
_Avoid_: Research ranking, import success, profitability guarantee

**Strategy Instance**:
A configured binding of one Strategy Component to its scope-correct Instrument or Point-in-Time Instrument Universe, decision schedule, parameters, allocation, and position mode.
_Avoid_: Strategy file, trading account

**Strategy Allocation**:
The exact capital assigned to a Strategy Instance at the start of a Run in its Paper Portfolio's Valuation Currency.
_Avoid_: Current equity, account balance

**Strategy Equity**:
The current mark-to-market value of a Strategy Instance's cash and controlled positions in its Paper Portfolio's Valuation Currency; it changes with fills, fees, and market prices.
_Avoid_: Initial allocation, Venue account equity

**Position Mode**:
The exposure constraint selected for a Strategy Instance: Long Only permits targets from zero through one, while Long–Short permits targets from negative one through one.
_Avoid_: Trade direction, signal type

**Feature**:
A finite `f64` analytical value associated with one Instrument and Observation Time, derived causally from evidence available no later than its declared availability boundary. Boolean, calendar, session, and categorical meanings require explicit versioned numeric encodings rather than a separate runtime value type.
_Avoid_: Future-known value, arbitrary model column

**Feature Observation**:
The identity-preserving outcome for one Feature, Instrument ID, and Observation Time: either one finite Available value or Unavailable with an exact typed reason. An Unavailable observation remains in evidence rather than being dropped, filled, or converted into a value.
_Avoid_: Nullable row, missing record, zero-filled Feature

**Feature Unavailability Reason**:
A versioned typed code explaining why one Feature Observation has no value, optionally accompanied by its exact Definition node and diagnostic detail. Codes, rather than free-form text, govern identity, propagation, inspection, and downstream behavior.
_Avoid_: Null, log message, fatal engine error

**Feature Scope**:
The observation dependency required to compute a Feature: Pointwise uses one observation, Time Series uses one Instrument's causal history, and Cross Sectional uses one exact Point-in-Time Instrument Universe at one Observation Time.
_Avoid_: Factor Scope, Model Scope, Asset Class

**Pointwise Feature**:
A Feature computed independently from one Instrument observation without earlier observations or simultaneous observations from other Instruments.
_Avoid_: Time-Series Feature, Cross-Sectional Feature

**Time-Series Feature**:
A Feature computed from one Instrument's observations in causal Observation Time order, with history and Warmup reset at Bar Gaps.
_Avoid_: Pointwise Feature, Cross-Sectional Feature, Time-Series Factor

**Rolling Feature Window**:
The exact count of consecutive eligible Closed Bars from one Continuous Bar Segment required by a Time-Series Feature. It becomes Available only when full, excludes Scheduled Closures, and restarts when a Bar Gap or unavailable dependency breaks the affected branch.
_Avoid_: Elapsed wall-clock window, partial rolling window, gap-filled history

**Cross-Sectional Feature**:
A Feature computed jointly for one exact Point-in-Time Instrument Universe at one Observation Time, preserving every member and explicit missingness. V1 binds that Universe to one Venue, Asset Class, Bar Interval, Price Basis, and Valuation Currency rather than ranking incomparable markets together.
_Avoid_: Pointwise Feature, Time-Series Feature, Cross-Sectional Factor

**Cross-Sectional Coverage Policy**:
The frozen minimum available-member count and coverage required before one Cross-Sectional Feature may be computed. Every Universe member remains in the output; members without inputs remain Unavailable, and any computation over the available subset records its actual coverage. Rank uses deterministic average ties, while Percentile and Z-score use their versioned Feature Operator Catalog formulas.
_Avoid_: Silent row filtering, current Watchlist, implicit threshold

**Feature Definition**:
A versioned, immutable declarative acyclic recipe for producing one through 64 ordered, uniquely named lower-kebab-case Features under one Feature Scope from Canonical Market Data, Built-in Indicators, declared host transformations, Fitted Transformation Artifacts, or compatible Factor outputs. A stable random Definition ID groups its positive integer revisions, while a content hash identifies exact semantics; any output addition, removal, rename, reorder, or other semantic edit creates a new revision. Arbitrary custom analytical logic belongs in a Factor Component.
_Avoid_: Ad-hoc notebook column, script, Feature Dataset

**Feature Definition Draft**:
The mutable, user-edited recipe from which ADAQ validates and freezes a Feature Plan. It has no immutable identity and is not research evidence.
_Avoid_: Feature Definition, Feature Plan, saved Dataset

**Feature Presentation Metadata**:
User-scoped mutable display information for one Feature Definition family, such as its name, description, and tags. It never changes Definition semantics, revision, content hash, Feature values, or another User's visibility.
_Avoid_: Feature Definition, shared catalog identity, semantic version

**Feature Operator Catalog**:
The versioned host-owned registry of declarative Pointwise, Time-Series, Cross-Sectional, fitted, calendar, liquidity, and corporate-action operations accepted in a Feature Definition. It is a finite public contract rather than a general expression or script language.
_Avoid_: Indicator Catalog, arbitrary function registry, notebook API

**Feature Engine Identity**:
The exact Feature Engine, Feature Operator Catalog, Indicator Engine, target, compiler, and build identity required to reproduce one Feature Plan's evidence. Identical builds require bit-identical output, while numerically equivalent platform builds retain distinct identities.
_Avoid_: Feature Plan hash, application version, Research Engine

**Feature Evaluation Error**:
A fatal, versioned failure containing an exact code, execution stage, Definition or node identity, applicable Instrument and Observation Time, and safe diagnostics. It terminates the Attempt and remains distinct from a Feature Unavailability Reason.
_Avoid_: Feature Unavailability Reason, localized message, debug string

**Feature Preview**:
A transient evaluation of one Feature Definition Draft against a bounded selection from an immutable Market Data Snapshot. It uses the production Feature Engine, preserves a complete Point-in-Time Instrument Universe for every Cross-Sectional batch, and may apply but never fit an Artifact; it has no persistent evidence identity and cannot replace Plan freezing or Feature materialization.
_Avoid_: Feature Dataset, validation evidence, sampled Backtest

**Feature Dataset**:
An immutable collection of Feature observations keyed by Instrument ID and Observation Time, bound to its exact Market Data Snapshot, Point-in-Time Instrument Universe, Feature Plan, availability, missingness, and provenance. Factor research selects a finalized Feature Dataset explicitly and never triggers implicit fitting or materialization.
_Avoid_: Mutable feature table, Forecast Signal Dataset

**Feature Dataset Summary**:
A rebuildable inspection projection over one exact Feature Dataset, recording per-output coverage, Feature Unavailability Reason counts, minimum, maximum, mean, and population standard deviation. It describes Dataset contents and is not Factor evaluation, Model evidence, or a performance claim.
_Avoid_: Factor Evaluation Report, mutable dashboard metric, Feature quality score

**Feature Materialization Request**:
The exact User-scoped binding of one frozen Feature Plan to a Market Data Snapshot, Point-in-Time Instrument Universe, observation range, parameters, and Seed for Feature Dataset production. The Plan remains reusable across compatible Requests, while every resulting Dataset retains its complete Request identity.
_Avoid_: Feature Plan, mutable form, Feature Materialization Attempt

**Feature Materialization Attempt**:
A lifecycle record for executing one frozen Feature Plan through Pending, Running, and exactly one terminal state: Completed, Failed, or Cancelled. Only a Completed Attempt may atomically publish an immutable Feature Dataset; partial values and diagnostics remain evidence but cannot be consumed downstream, and interrupted work becomes Failed with retained interruption evidence.
_Avoid_: Feature Dataset, generic Job, mutable Dataset

**Transformation Fitting Protocol**:
An immutable, content-addressed specification binding one fitted transformation node to its exact input Feature, Market Data Snapshot, Point-in-Time Instrument Universe, Fitting Scope, fitting window, algorithm parameters, and engine identity. Walk-forward research freezes a separate Protocol and Artifact for every fold.
_Avoid_: Full-history preprocessing, Model Training Protocol, mutable fit settings

**Fitting Scope**:
The Instrument grouping used to learn one Fitted Transformation Artifact: Pooled Universe learns one parameter set from all eligible observations, while Per Instrument retains an exact parameter set for every Instrument ID in the fitting Universe.
_Avoid_: Feature Scope, Cross-Sectional normalization, implicit grouping

**Transformation Fitting Attempt**:
A lifecycle record for executing one Transformation Fitting Protocol through Pending, Running, and exactly one terminal state: Completed, Failed, or Cancelled. Only a Completed Attempt may publish a Fitted Transformation Artifact; Feature materialization applies that Artifact and never refits it.
_Avoid_: Feature Materialization Attempt, Model Training Attempt, implicit fit

**Fitted Transformation Artifact**:
Immutable Host-owned transformation parameters learned only from an exact recorded fitting window and applied unchanged to later validation, test, inference, or Paper Trading observations. Its deterministic Eligible At is the latest Available At among its fitting inputs, while Created At records local operational completion without changing historical Feature identity. A Python script may explore custom preprocessing, but that path remains Research Only unless the operation and Artifact schema become an explicit supported transformation.
_Avoid_: Model Artifact, full-history scaler, mutable preprocessing state

**Causal Corporate Action Transformation**:
A provenance-bound forward transformation that applies Split or Dividend evidence only after it became available and effective. Split transforms subsequent price and quantity values onto the initial causal basis, while Dividend evidence updates a separate Total Return Feature; neither rewrites earlier Feature observations or Canonical Market Data.
_Avoid_: Back-adjusted history, Canonical data repair, Corporate Action

**Realized Volatility Feature**:
The population standard deviation of backward-looking log returns over one full Rolling Feature Window. Its base value is per Bar; any annualization is a separate explicit transformation bound to the exact Venue calendar, Bar Interval, and periods-per-year rule.
_Avoid_: Implied volatility, hidden annualization, future volatility

**Liquidity Feature**:
A declared analytical measure derived from available market evidence, such as Quote Volume, rolling Quote Volume, zero-volume state, or Amihud Illiquidity. ADAQ does not call Quote Volume `turnover` or produce a Turnover Ratio without a trustworthy shares, float, or market-value denominator.
_Avoid_: Turnover when meaning Quote Volume, liquidity score without units, inferred market cap

**Session Progress Feature**:
A Venue-calendar-bound value from zero through one measuring progress across eligible trading minutes in a Trading Date. Scheduled breaks and closures do not advance it and do not create synthetic market observations.
_Avoid_: UTC day progress, wall-clock time since open, device-local session progress

**Feature Plan**:
The fully validated, immutable `2.0.0` resolution of an acyclic Feature Definition graph or Feature Slots and their exact Market, Indicator, Factor, Forecast Signal Dataset, transformation, and Fitted Transformation Artifact sources, including scopes, parameters, availability, missingness, Warmup, and Feature Engine Identity. It freezes reusable computation semantics rather than one Market Data Snapshot, Point-in-Time Instrument Universe, or observation range. Historical materialization, Factor research, Model training or inference, Strategy Backtest, and Paper Trading each freeze the Plan they execute; a modular Backtest references an already finalized Forecast Signal Dataset and never invokes its producing Model implicitly.
_Avoid_: Indicator Plan, runtime feature lookup

**Warmup**:
The exact leading Closed Bars in each Continuous Bar Segment that prepare all bound analytical sources and Model contexts before a Strategy Instance may be invoked. Upstream Feature Warmup and Model Warmup compose without synthetic values; a Bar Gap starts a new segment and therefore rebuilds analytical state and restarts Warmup.
_Avoid_: Optional startup delay, approximate lookback

**Market Data Snapshot**:
An immutable identity for the exact Canonical Market Dataset selection used by reproducible research or execution evidence.
_Avoid_: Latest market data, mutable cache

**Model Artifact**:
The immutable fitted result of one successfully validated Model Training Attempt, bound to its exact Model Training Protocol, training evidence, payload hashes, schema, and provenance. It becomes deployable only through a qualified Model Deployment Profile.
_Avoid_: Model Component, mutable checkpoint, training project

**Linear Model Artifact**:
The canonical `adaq:linear-model@1` fitted payload containing ordered Python Input Slot identities, finite coefficients, intercept, numeric representation, exact Fitted Transformation Artifact, one Forecast Signal contract, and Qlib Ridge Adapter provenance. The Adapter must reload this data-only schema before producing published Forecasts; no Python pickle, executable object graph, Dataset, or training code belongs to the Artifact.
_Avoid_: Qlib pickle, ONNX graph, Model Component

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
An immutable execution record binding a Market Data Snapshot, Component Lock, Strategy parameters, Feature Plan, Risk Policy, Execution Profile, engine version, and seed.
_Avoid_: Editable backtest session

**Backtest Run Draft**:
The mutable, user-edited configuration from which ADAQ validates and prepares a Backtest Run. It may be restored from immutable Run provenance but is never research evidence and has no immutable identity.
_Avoid_: Backtest Run, editable Run, pending Run

**Backtest Preflight**:
A transient host-derived validation and preparation result bound to one exact revision of a Backtest Run Draft. Any Draft change invalidates it; it is neither persisted nor treated as research evidence.
_Avoid_: Backtest Run, saved Draft, validation evidence

**Target Exposure**:
The desired signed notional fraction of a Strategy Instance's current Strategy Equity: zero is flat, positive is long, and negative is short.
_Avoid_: Signal strength, order quantity, confidence score

**Strategy Target**:
The complete pre-risk investment intent emitted by a Strategy Component: a Target Decision for Single-Instrument Scope or a Portfolio Target for Portfolio Scope.
_Avoid_: Approved Target, order instruction, Forecast Signal

**Target Decision**:
The complete Target Exposure emitted by a Strategy Instance for one Closed Bar; returning the current target represents hold and returning zero represents close.
_Avoid_: Buy signal, sell signal, optional decision

**Portfolio Target**:
The complete Instrument-keyed target-weight allocation and cash reserve emitted by one Portfolio Strategy for an exact decision time and Point-in-Time Instrument Universe. It expresses desired allocation rather than orders, and omitted members are invalid rather than implicit holds or exits. V1 Portable Definitions are Long-only: every Universe member has a canonical Decimal target weight of zero or greater, Cash Reserve is zero or greater, and their exact sum is one; Short and Leverage require a later explicit Position Mode contract.
_Avoid_: sparse trade list, order basket, independent Target Decisions

**Risk Policy**:
The immutable, versioned host rules that enforce capital, exposure, concentration, loss, market, account, and operational limits against a Strategy Target. A Risk Policy may preserve or reduce requested risk but never create exposure the Strategy did not request.
_Avoid_: Strategy stop logic, Execution Profile, hidden guardrail

**Risk Decision**:
The recorded Approve, Constrain, or Reject outcome of applying one exact Risk Policy to one Strategy Target and Portfolio State, including machine-readable reasons. Constrain produces only a lower-risk target; Reject authorizes no risk-increasing order from the rejected intent.
_Avoid_: silent target mutation, Validation Report, trading recommendation

**Approved Target**:
The scope-correct Strategy Target retained or reduced by one Risk Decision and permitted to enter Execution. It always preserves the original Strategy Target and constraint reasons as separate evidence.
_Avoid_: Strategy Target, order basket, unrecorded clipped weight

**Execution Plan**:
The deterministic translation of one Approved Target and current Portfolio State under an exact Execution Profile into venue-valid order intentions, including thresholds, rounding, sequencing, expected costs, and order policy.
_Avoid_: Strategy logic, Fill, broker response

**Run Pause**:
The recorded absence of a Target Decision while a Run is warming up or lacks a required Feature or Forecast Signal; missing data is never replaced with a synthetic analytical value. A Run Pause does not mean flat or close, produces no new order intention, and leaves the current exposure unchanged.
_Avoid_: Zero signal, implicit skip

**Execution Profile**:
Host-owned rules that translate an Approved Target into simulated or submitted order intentions, including rebalance thresholds, order types, sequencing, maker and taker fees, slippage, precision, liquidity participation, and fill policy. Funding rates are outside the Spot execution model.
_Avoid_: Strategy order settings

**Simulated Order**:
A Backtest record of one host-derived Spot order intention and its created, filled, replaced, or cancelled lifecycle under the Run's frozen Execution Profile; it is never sent to a Venue.
_Avoid_: Target Decision, Fill, live order

**Fill**:
The completed execution of all or part of an order at an exact price and quantity, with its fee and maker or taker role. A Backtest Fill is simulated under the Run's frozen Execution Profile; a Paper Fill additionally retains its provider or local Paper Fill Evidence State.
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

**Interface Locale**:
The device-local GUI presentation preference selected in Settings > General and applied through `i18next` and `react-i18next`. V1 provides exactly two resource locales, English (United States) `en-US` and Simplified Chinese `zh-CN`, plus a System selection that resolves a Chinese system language to `zh-CN` and every other language to `en-US`. It changes translated interface copy and locale-aware display formatting immediately without changing canonical domain values, identifiers, exported evidence, provider payloads, Venue times, or user-authored names; missing translations fall back to `en-US` rather than blank copy.
_Avoid_: Data locale, Venue Time Zone, translation of research evidence

**Operational Responsibility**:
A current condition that requires the User's operational attention even when no Bot Worker is actively evaluating: any Bot Runtime Attempt in Starting, Reconciling, WarmingUp, Running, Pausing, Paused, Stopping, or Faulted; Reconciliation Required; a retained Paper position or non-terminal Paper Order; or an Active Warning or Critical Operational Alert. It ends only when every Bot is safely Stopped and all listed positions, orders, reconciliation conditions, and Alerts are resolved.
_Avoid_: Running process, Bot Lifecycle State, dashboard visibility flag

**Workflow Foundations**:
The unnumbered Markets and Data plus Feature Engineering prerequisites that feed the ten-step Research-to-Paper workflow. One research context is ready when it has a compatible readable Market Data Snapshot, Point-in-Time Instrument Universe, and Completed Feature Dataset; ADAQ does not require every supported market to be ready before Factor research can begin.
_Avoid_: Step 0, global readiness flag, three-market completion gate

**Workflow Guide**:
The localized, read-only presentation of ADAQ's Foundations and ordered ten-step Research-to-Paper workflow. It distinguishes Workflow Capability State from evidence-backed Workflow Step State, exposes the next eligible entry point, remains directly accessible from Help, and is the default GUI home only while no Operational Responsibility exists. Users may inspect downstream steps while their actions remain gated by exact prerequisites, and ambiguous evidence lineages require explicit selection rather than an inferred global project.
_Avoid_: Operations Dashboard, workflow record, global completion percentage

**Workflow Capability State**:
The release-level availability of one Workflow Guide entry: Available when its accepted end-to-end V1 action exists, Partial when only a proper subset exists, or Planned with its owning Milestone when the action is not yet delivered. A Partial entry may expose existing evidence counts but cannot claim the complete step is finished; Capability State never represents User progress or unlocks an action whose evidence prerequisites are missing.
_Avoid_: Workflow Step State, feature availability, optimistic completion

**Workflow Step State**:
A user-scoped read projection derived from authoritative domain records as Not Started, In Progress, Needs Review, Blocked, or Complete under that step's exact evidence gate. It is not a persisted generic workflow record; continuous Monitoring uses Bot Health States instead of pretending to become Complete.
_Avoid_: Workflow Capability State, mutable checklist, global completion percentage

**Product Readiness Gate**:
The release-level rule that every required acceptance criterion inside one declared capability scope must pass before the scope can be called product-ready. It is not a global V1 flag, a score, a permission, a Workflow Step State, or a runtime Health summary.
_Avoid_: global readiness flag, completion percentage, optimistic availability

**Readiness Assertion**:
An immutable, reviewable acceptance record for one exact Capability, Journey, Market/Data Context, Supported Platform, and Interface Locale scope. It binds each criterion to evidence, reviewed commit, platform, locale, limitations, reviewer, and timestamp and yields Ready for declared scope or Not Ready. Any in-scope change requires a new assertion; old assertions remain historical.
_Avoid_: mutable latest pointer, runtime database entity, User Decision, Health State

**Acceptance Reviewer**:
The designated V1 human Reviewer or Release Owner who may declare a Readiness Assertion Ready after checking every required gate. Tests, CI, domain Owners, Dashboard state, and AI surfaces provide evidence but cannot announce readiness or grant authority.
_Avoid_: automated promotion, domain self-approval, AI authority

**Declared Scope**:
The explicit Capability, Journey, Market/Data Context, Supported Platform, and Interface Locale boundary to which one Readiness Assertion applies. Limitations outside this boundary are disclosed rather than silently generalized to other markets, platforms, locales, or workflows.
_Avoid_: universal V1 support, provider parity, global completion

**Operations Dashboard**:
The localized Tauri and React GUI operational view that projects global Paper Trading, Bot, account, Alert, infrastructure, data, Factor, Model, Component-build, Backtest, and Validation status and exposes host-controlled emergency actions. It is the default GUI home whenever any Operational Responsibility exists and is an overview and drill-down entry point, not the Workflow Guide, Crypto market page, a TUI, an authoritative event store, or a direct provider client.
_Avoid_: Market Workspace, status database, remote operations console

**Dashboard Projection**:
A user-scoped, rebuildable read model derived from authoritative domain records and Operational Events for immediate GUI display. The route may reuse a current-session cache and accept incremental host updates, but stale display state never grants trading authority or replaces reconciliation.
_Avoid_: authoritative ledger, Worker state, frontend-owned trading state

**Market Workspace**:
The localized GUI navigation area under `/markets` for inspecting market and data evidence through Overview, Crypto, China A-share, and United States Equity routes. It presents one asset-neutral Watchlist, Instrument search, Venue-local session state, Ticker and Bar views, provider coverage, freshness, quality, and rule summaries without becoming the Operations Dashboard or an order-entry terminal.
_Avoid_: Operations Dashboard, Market Data Connector, full trading terminal

**Feature Workspace**:
The localized GUI area at `/features` for editing Feature Definition Drafts and inspecting published Definitions, Transformation Fitting Attempts, Feature Materialization Attempts, and Feature Datasets. Its Preview is transient and never becomes authoritative Feature evidence.
_Avoid_: Factor Lab, Model Lab, notebook editor

**Page Navigation History**:
The ordered sequence of ADAQ application pages and selected Backtest-result or Validation-report tabs visited in the current WebView session, used by Back and Forward to restore a previously visited page or tab, or revisit one after going back. A new page or tab visited after going back discards the Forward sequence. It excludes report selection, form state, external pages, other non-ADAQ history entries, and shareable URL state.
_Avoid_: Page stack, custom navigation session, WebView history, deep link

### Supervised Live Trading

**Trading Account**:
The broker-, Venue-, or ADAQ-managed custody and ledger boundary through which orders, cash, positions, and Fills are observed. Its identity is distinct from the ADAQ User and from a Strategy Instance.
_Avoid_: User account, Strategy, API credential

**Valuation Currency**:
The single currency in which one Paper Portfolio measures cash, equity, exposure, PnL, and risk limits. Values in different Valuation Currencies are never added without an exact FX Snapshot.
_Avoid_: display currency, Quote Asset, currency symbol

**Paper Trading Account**:
A Trading Account that produces simulated rather than real-money executions, either through an external Paper Provider or an ADAQ-owned simulator. Its exact Account Snapshots remain authoritative for the paper ledger it represents.
_Avoid_: Backtest Run, Real Trading Account, local balance preference

**A-share Ordinary Securities Account**:
The only A-share account profile simulated in V1: an ordinary securities account that buys from its own available cash and sells only eligible securities it already owns, subject to exact settlement, lot, price-limit, fee, and session evidence. It has no financing, securities lending, short sale, collateral, maintenance-margin, interest, or forced-liquidation capability.
_Avoid_: Cash-account approximation, Credit Account, margin account

**A-share Credit Account**:
An A-share financing and securities-lending account with distinct credit limits, collateral, liabilities, interest, maintenance ratios, repayment, and forced-liquidation rules. It is outside V1 and is never emulated by enabling negative cash or negative positions on an Ordinary Securities Account.
_Avoid_: Ordinary Securities Account, generic leverage flag

**Paper Execution Adapter**:
A provider-specific host integration that translates ADAQ Execution Plans into Paper orders and normalizes account, order, Fill, rejection, and connection evidence without discarding the provider-native identities or payload meanings needed for diagnosis.
_Avoid_: public unified trading API, Strategy Component, provider SDK type

**Paper Connection Profile**:
A User- and device-scoped host configuration that binds one Paper Execution Adapter to an exact provider environment, allowlisted endpoints, expected account, capabilities, and an opaque Secret Reference. It contains no credential value, is separate from the Paper Trading Account ledger, and cannot select a Live environment in V1.
_Avoid_: API key, Paper Trading Account, arbitrary base URL

**Secret Reference**:
A random opaque identifier stored in ADAQ metadata that locates one User-scoped credential entry in the operating-system secret store. It is safe to persist and compare as metadata but never reveals, derives, exports, or substitutes for the provider credential itself.
_Avoid_: encrypted credential column, masked secret, API key

**Paper Connection Test**:
A host-only, non-ordering authentication and capability check for one exact Paper Connection Profile. It retrieves the credential through its Secret Reference, verifies the Paper or Demo environment, account identity, Valuation Currency, permissions, clock, and provider capabilities, records only redacted evidence, and never places, cancels, or modifies an order.
_Avoid_: test order, Bot reconciliation, frontend API request

**Paper Execution Capability Snapshot**:
An immutable observation of one Paper Execution Adapter and account's supported Instrument classes, order types, sessions, precision, buying-power rules, short or margin behavior, fill assumptions, event streams, limits, and reset capabilities at one time.
_Avoid_: static provider feature list, Risk Policy, API credential

**Paper Account Reconciliation**:
The evidence-producing comparison of provider-authoritative Account Snapshots and ordered account events with ADAQ's local Paper ledger at startup, reconnect, or detected inconsistency. Unresolved differences block new risk rather than being silently overwritten or ignored.
_Avoid_: balance refresh, cache replacement, Backtest replay

**Paper Order**:
A simulated order with an append-only Submitted, Accepted, Partially Filled, Filled, Cancelled, or Rejected lifecycle under one Paper Execution Adapter. Provider-native and ADAQ identities, timestamps, requests, responses, reasons, and every partial Fill remain evidence.
_Avoid_: Execution Plan, Backtest Simulated Order, mutable order row

**Paper Fill Evidence State**:
The provenance class of a local simulated Fill: Trade Observed is bounded by a post-acceptance trade or auction result with quantity evidence, Quote Constrained is bounded by a post-acceptance executable quote, Bar Constrained uses only a post-acceptance bar under an explicit conservative participation policy, and Unavailable permits no Fill. The State describes simulation evidence rather than claiming exchange queue position.
_Avoid_: confidence score, provider Fill, hidden fill assumption

**A-share Paper Fill Engine**:
The ADAQ-owned event-driven simulator that advances Paper Orders only from observations received after order acceptance under exact Market Rule, Calendar, Execution, and liquidity-participation evidence. It never uses later full-Bar knowledge, an optimistic last price, or unavailable queue position to manufacture a Fill.
_Avoid_: Backtest Fill model, A-share broker, bar-crossing shortcut

**Paper Account Snapshot**:
An immutable observation of one Paper Trading Account's cash, positions, reserved funds, buying power, and provider-reported equity at one time. When an external Paper Provider differs from a configured funding target, the observed Snapshot remains authoritative and the difference stays visible.
_Avoid_: desired initial capital, cached balance, Portfolio State

**Paper Portfolio**:
The capital, positions, orders, and risk ledger assigned to one Paper Trading Account and one Valuation Currency. It may contain multiple Instruments available to that Account but never shares capital across Accounts or Valuation Currencies in V1.
_Avoid_: Global Portfolio, Watchlist, collection of account balances

**Paper Funding Target**:
The exact desired starting capital for a Paper Trading Account in its Valuation Currency. It configures an ADAQ-owned simulator or guides an external account reset, but never overrides a contradictory external Paper Account Snapshot.
_Avoid_: Account Snapshot, guaranteed buying power, display balance

**Trading Bot**:
The supervised V1 Paper Trading deployment that binds one qualified Strategy, its Models and Features, one Paper Portfolio, Risk Policy, Execution Profile, decision schedule, and exact runtime identities. A Trading Bot is the durable deployment and evidence boundary rather than generated Rust source or one operating-system process.
_Avoid_: Strategy Component, Bot Worker, broker connection

**Bot Supervisor**:
The host-owned Rust control plane that owns Trading Bot lifecycle, validated market-data distribution, account reconciliation, capital reservations, hard Risk, OMS, Paper Execution Adapters, credentials, durable journals, emergency controls, and Worker supervision. It alone can authorize an Approved Target to reach an Adapter.
_Avoid_: GUI page, Strategy Component, shared Worker

**Bot Worker**:
One supervised child-process instance of the prebuilt, versioned, bundled `adaq-bot-worker` Sidecar for one active Trading Bot. It verifies and loads that Bot's qualified Deployment Bundle, evaluates its frozen Feature, Model, and Strategy pipeline, and returns Strategy Targets and diagnostics; it has no credentials, account authority, Risk bypass, OMS, or order-submission capability.
_Avoid_: generated Bot executable, Paper Execution Adapter, Python Bot

**Bot Deployment Bundle**:
The immutable, content-identified binding of one Trading Bot's qualified Component and Model payloads, Component Locks, Feature Plan, Paper Portfolio, Risk Policy, Execution Profile, schedules, runtime versions, limits, and Deployment Qualification. Every Worker start verifies the Bundle before evaluation.
_Avoid_: generated source project, mutable Bot settings, Component Package

**Bot Runtime Attempt**:
The append-only operational record created by one explicit Trading Bot Start or Retry. It binds the exact Deployment Bundle and Worker identity and retains every lifecycle transition, heartbeat, decision, diagnostic, order relationship, recovery action, and terminal outcome; Stopped and Faulted are terminal for the Attempt, and Retry always creates a new identity.
_Avoid_: Trading Bot, Worker process, mutable current-state row

**Bot Lifecycle State**:
The exact control state of one current Bot Runtime Attempt: Stopped, Starting, Reconciling, WarmingUp, Running, Pausing, Paused, Stopping, or Faulted. Only Running may authorize a new risk-increasing Strategy Target; health severity and Reconciliation Required are separate conditions rather than hidden lifecycle aliases.
_Avoid_: Bot health, connection status, Strategy position

**Bot Health Dimension**:
One independently evaluated operational dependency of a Trading Bot: Market Data, Worker, Feature/Model/Strategy, Paper Account, Risk/OMS, Execution Adapter, Local System, or Research Feedback. Its evidence remains visible even when an Overall Bot Health is derived.
_Avoid_: Lifecycle State, one online flag, alert count

**Bot Health State**:
The current evidence-backed condition of one Health Dimension: Healthy, Degraded, Critical, or Unknown. Unknown is never promoted to Healthy and fails closed when the Dimension is required to authorize new risk.
_Avoid_: Bot Lifecycle State, Alert Severity, provider connection boolean

**Overall Bot Health**:
The worst current State among the exact Health Dimensions required by a Trading Bot's Deployment Bundle and Lifecycle State. It is a summary for triage and never hides the underlying Dimension States or independently grants order authority.
_Avoid_: average health score, Lifecycle State, profitability grade

**Operational Event**:
An append-only, timestamped, typed observation of a Bot, Worker, data feed, Model, account, Risk, OMS, Adapter, local system, alert, or operator action. Current Health and Dashboard views are projections from retained Events rather than mutable replacements for evidence.
_Avoid_: raw market tick archive, overwritten status row, debug string

**Operational Alert**:
A deduplicated operational incident derived by the host Monitoring Engine from one or more Operational Events under a frozen Alert Policy. It has Info, Warning, or Critical Severity and an Active, Acknowledged, or Resolved lifecycle; acknowledgement records that an operator saw it and never asserts recovery.
_Avoid_: Bot Health State, toast message, resolved failure

**Monitoring Safety Action**:
A host-authorized fail-closed response linked to the exact Alert and evidence that caused it, such as skipping one Decision, entering Pausing, making an Attempt Faulted with Reconciliation Required, or invoking Freeze All. Research drift may request review but never automatically replaces a Model or Strategy.
_Avoid_: alert notification, Strategy stop-loss, silent automation

**Reconciliation Required**:
A fail-closed operational condition indicating that authoritative Paper Account, order, Fill, position, or local-journal state has not yet been reconciled. It blocks entry or return to Running and cannot be cleared by overwriting evidence or assuming an order was cancelled.
_Avoid_: Reconciling lifecycle state, refresh required, warning-only mismatch

**Stop and Keep Position**:
The default Trading Bot stop operation that blocks new decisions, cancels eligible open orders, reconciles the account, terminates the Worker, and retains any resulting Position as an Unmanaged Position.
_Avoid_: Pause, Flatten, application exit

**Stop and Flatten**:
A separately confirmed Trading Bot stop operation that first blocks new risk and cancels open orders, then attempts host-controlled risk-checked liquidation and reconciliation before the Bot may become Stopped. Failure to establish a reconciled flat state remains Faulted or Stopping with explicit unresolved exposure.
_Avoid_: blind market order, Stop and Keep Position, Freeze All

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
