# Market Workspaces Guide

[简体中文](./market-workspaces.zh-CN.md)

Status: V1 market-observation, navigation, provenance, and user-acceptance contract.

Related guides: [Operations Dashboard](./operations-dashboard.md), [Paper Trading Accounts](./paper-trading-accounts.md), and [Monitoring and Alerting](./monitoring-and-alerting.md).

## Why market views are part of V1

V1 acquires, validates, researches, and Paper trades Crypto, China A-shares, and U.S. equities. Users must be able to inspect the Instruments, session state, Bars, provider limitations, freshness, and quality behind those actions. A data connector with no inspectable market workspace would make stock support operationally incomplete.

V1 therefore includes basic three-market observation. It does not attempt to reproduce a Bloomberg-style terminal.

## Navigation

```text
Markets / 行情
├── Overview                 /markets
├── Crypto                   /markets/crypto
├── China A-shares           /markets/a-shares
└── U.S. Equities            /markets/us-equities
```

The existing Crypto Watchlist, Ticker, and Kline experience moves from the old `/` home to `/markets/crypto`. The `/` route becomes the Operations Dashboard.

## Shared page contract

Every market route provides the capabilities supported by its evidence:

- Venue-specific Instrument search and deterministic result identity.
- Current Venue-local Trading Date, Session Phase, open or closed status, and next known session boundary.
- The User's Watchlist filtered to the route's Market, with cross-market access through Overview.
- Ticker values such as last price, best Bid and Ask when available, volume, provider time, and observation age.
- Historical Closed-Bar chart and supported Bar Intervals.
- Instrument lifecycle status, applicable Market Rule summary, and explicit Unknown values.
- Market Data Provider, feed, Connector, capability, delay, coverage, provenance, and Data Quality State.
- Data refresh, stale, degraded, gap, and provider-error states.
- Links to applicable Dataset, research, Backtest, Strategy, or Bot configuration workflows.

Market pages inspect evidence and start workflows. They never submit an order directly or bypass Strategy, Host Risk, OMS, and Paper Execution Adapter boundaries.

## Markets Overview

`/markets` is a compact cross-market entry point, not a second Operations Dashboard. It shows:

- One asset-neutral Watchlist with Market and Venue labels.
- A-share, U.S. equity, and Crypto current Session Phase and data-health summary.
- Provider coverage, latest successful update, degraded or unavailable state, and links to the affected market.
- Recently viewed Instruments and direct navigation to each Market Workspace.

Overview does not add unlike prices, volumes, returns, currencies, or account balances into a false global metric.

## One asset-neutral Watchlist

The Watchlist stores full Instrument IDs, not display symbols. For example, an item retains its exact Venue plus native code. Two providers or Venues using the same visible symbol remain distinct.

- Overview may show every Watchlist item with Market and Venue filters.
- Crypto shows only eligible Crypto Instruments.
- A-shares shows only eligible China A-share Instruments.
- U.S. Equities shows only eligible United States Equity Instruments.
- Ordering remains User-scoped and stable when switching routes.
- A route filter never deletes items belonging to another Market.

## Crypto Workspace

`/markets/crypto` preserves the existing OKX Spot experience and extends its evidence display:

- OKX Spot Instrument search and lifecycle status.
- Existing Watchlist, selected Instrument, Ticker, and Kline chart.
- UTC continuous-market Bar grid and data age.
- Provider connection, stream, REST reconciliation, Instrument Master, and Data Quality status.
- Current selected-Instrument Trade and Level 2 health when active; V1 does not provide historical order-book replay.

Funding rates, futures, perpetuals, options, and derivatives remain outside the Spot Workspace.

## China A-share Workspace

`/markets/a-shares` presents A-share evidence obtained through `akshare-rs` while naming the actual upstream source and method whenever available. It includes:

- Venue and native Instrument code, board or segment, listing and suspension status.
- `Asia/Shanghai` Trading Date, auction, continuous, midday-break, and closed phases.
- Ticker and unadjusted historical Bars with explicit Price Basis.
- Corporate Action availability and derived-adjustment provenance when an adjusted view exists.
- Applicable price-limit, lot, T+1, special-treatment, fee, and other Market Rule summary.
- Explicit Unknown state when a provider or effective-time rule cannot establish a fact.

The Market Workspace does not imply that `akshare-rs` is the Venue or the original data owner, and it does not invent realtime coverage, queue data, or tradeability from an incomplete response.

## U.S. Equities Workspace

`/markets/us-equities` presents Alpaca Market Data as the primary V1 source and makes plan/feed limitations visible:

- Venue, native ticker, listing and trading status.
- `America/New_York` Regular, Extended, holiday, and early-close session evidence.
- Ticker and historical Bars with feed identity, observation time, and coverage or delay badge.
- Corporate Action evidence and declared Price Basis.
- Provider capability and streaming-symbol limits relevant to the current account.

Alpaca IEX-limited data is never labelled as consolidated whole-market realtime. Auxiliary `yfinance-rs` observations may be shown for supported history, Corporate Actions, fundamentals, or cross-checking only with their own provenance; they never silently repair or replace canonical realtime evidence.

## Time, formatting, and identity

- Canonical event and Bar boundaries remain UTC instants.
- Trading Dates, sessions, Bar alignment, and rules use each Venue's IANA time zone.
- The GUI defaults market time to Venue-local display and may additionally show device time or UTC without changing identity.
- English (US) and Simplified Chinese change labels and formatting, not Instrument IDs, times, prices, provenance, or evidence.
- Financial values remain exact Decimals; formatted strings never re-enter calculations.

## Loading and freshness

- Every Market route paints its shell immediately.
- Instrument search, Watchlist, Ticker, chart, rules, and quality panels own their Loading and Failure states.
- Current-session, User-scoped read data may appear immediately on re-entry and refresh in the background.
- Every live or cached value exposes its provider observation time and current age.
- A stale cache never changes Instrument Status, account truth, Bot eligibility, or order authority.
- Missing or degraded provider data stays visible; the GUI never substitutes another provider without an explicit new provenance identity.

## V1 boundary

V1 includes enough market observation to inspect its three data and Paper Trading paths. The following are post-V1:

- Advanced multi-condition stock and crypto screeners.
- Complete fundamentals and financial-statement terminals.
- News, sentiment, and alternative-data workspaces.
- Historical Level 2, order-book reconstruction, DOM, and queue analytics.
- Multi-window and linked multi-chart terminals, advanced drawing, and custom studies on the chart.
- Options, futures, perpetuals beyond the accepted Spot scope, and other derivatives.
- Cross-market arbitrage and consolidated smart-order-routing workspaces.

## V1 acceptance checks

1. Existing Crypto Watchlist, Ticker, and Kline behavior remains available at its new route.
2. All four routes paint immediately and isolate loading, empty, stale, degraded, and failure states by control.
3. One User-scoped Watchlist retains exact Instrument IDs and filters correctly across all three Markets.
4. Each market uses the correct Venue calendar and time-zone semantics across holidays, breaks, early closes, and daylight-saving changes.
5. Ticker and Bar values show provider identity, feed coverage or delay, observation time, and Data Quality State.
6. A-share Unknown rules and U.S. IEX-limited coverage are never presented as complete evidence.
7. Market workflow links never bypass research qualification, Host Risk, OMS, or Paper Execution Adapters.
8. English (US) and Simplified Chinese provide equivalent functionality and accessible labels.

## Verification journeys

Fixture-backed verification journeys cover the data foundation behind the three market routes (macOS ARM64 canonical; Windows uses PowerShell and `Get-FileHash -Algorithm SHA256`, Linux uses `sha256sum`):

1. **OKX Spot** — `cd src-tauri && cargo test -p adaq-data-pipeline --lib okx::tests -- --nocapture` covers Instrument Master, pagination, rate/retry handling, closed bars, checkpoints, restart/resume, gaps, revisions, REST/WebSocket reconciliation, bounded Trade retention, and non-persistent Level 2. In `/markets/crypto`, inspect the Instrument Master, resume an interrupted acquisition from its checkpoint, check Source/Canonical quality, derive a higher interval deterministically, and republish the immutable Snapshot.
2. **China A-share** — `cd src-tauri && cargo test -p adaq-data-pipeline --lib a_share` covers actual-upstream provenance, SSE/SZSE identity, exact decimals, sessions, corporate actions, quality, cancellation, and restart/resume. In `/markets/a-shares`, inspect acquisition provenance, unadjusted `PriceBasis` with `Asia/Shanghai` sessions, and the separate immutable Corporate Action evidence.
3. **U.S. equity** — `cd src-tauri && cargo test -p adaq-data-core --lib alpaca` covers fixed endpoints, exact values, IEX capability, DST/holiday/early-close calendars, symbol limits, and daily-bar anchoring; `cargo test -p adaq-data-pipeline --lib us_equity` covers the pipeline. In `/markets/us-equities`, inspect the Provider Capability Snapshot and session evidence with `America/New_York` semantics.
4. **Quality, lifecycle, and research evidence** — `cd src-tauri && cargo test -p adaq-data-pipeline --lib` covers Passed/Degraded/Rejected paths, quarantine, explicit gaps, cancellation, atomic cleanup, user isolation, revisions, and Snapshot publication. Scheduled closures are calendar evidence, not false Bar Gaps; genuine gaps are retained, never forward-filled; prior Source revisions remain append-only; referenced Snapshots are deletion-locked; old Snapshots replay their original immutable evidence.
5. **Connections and no-order invariant** — `cargo test --lib connections` covers fixture-backed save, test, rotation, deletion, user isolation, endpoint allowlisting, redaction, permissions, currency, and clock-skew handling; `cargo test --lib connections connection_test_never_requests_an_order_endpoint` proves every connection test is read-only (no `/orders` or trade endpoint). Alpaca Paper and OKX Demo are the only connection environments; Live or custom endpoints are rejected before network use.
6. **Localization and shell** — `pnpm exec jest --watchman=false --runInBand src/lib/i18n.test.ts src/bootstrap.test.ts` covers locale resolution, persistence boundaries, fallback, `Intl` formatting, and first-paint ordering; market routes stay keyboard-accessible at 1024 px with non-color state meaning.

Credentials, authorization headers, OTPs, tokens, private paths, and private market data never enter evidence; optional real-provider checks run only with maintainer-owned credentials entered in **Settings → Connections** and are deleted afterward.

## Implemented GUI and screenshot expectations

The desktop implementation now exposes the four routes above, keeps the existing Crypto workspace at `/markets/crypto`, and uses one Venue-plus-native-code Watchlist identity across the market filters. A-share and U.S. equity pages keep provider observations, calendar coverage, unavailable Bid/Ask values, and direct-provider Bar quality visible rather than upgrading them to canonical evidence.

The manual acceptance screenshot set should include each route in both supported locales, plus:

- the Overview Watchlist containing instruments from at least two Markets;
- an A-share or U.S. equity search result showing native code, Venue, and provider identity;
- a session/calendar card showing Venue-local Trading Date, time zone, and Unknown or Closed when evidence is unavailable;
- a ticker or chart card showing an explicit unavailable value, provider observation time, and Data Quality state;
- an empty, loading, degraded, or provider-error control state without hiding the surrounding route shell.

Screenshots are acceptance evidence only; they must not imply realtime, consolidated, adjusted, or canonical quality when the provider contract does not establish it.
