# EMA double-cross OKX Demo workflow

Status: strategy semantics and execution preparation implemented; supervised acceptance incomplete.

## Accepted scope

Use the User's three OKX Spot Watchlist instruments: BTC-USDT, ETH-USDT, and SOL-USDT (verified in Desktop on 2026-09-11). Acquire evidence, construct EMA Factors, qualify the rule-based Strategy, deploy supervised OKX Demo Bots, and produce an execution and Paper feedback report. Skip predictive Model training. Use long-only Spot positions without leverage or repeated additions to an existing position.

## Accepted signal semantics

- Evaluate EMA5 and EMA10 on the forming 15-minute bar.
- After the first upward crossing, require EMA5 > EMA10 continuously for 60 seconds; invalidate the pending confirmation if that condition fails. Record the highest traded price within the successful confirmation window.
- Require an intervening downward crossing before the second upward crossing.
- After the second upward crossing and a price breakout above the recorded high, require both price > recorded high and EMA5 > EMA10 continuously for 60 seconds before generating a buy signal.
- Do not expire the first confirmed reference solely because time passes. Clear the buy sequence after a buy; do not add while already holding a position.
- Sell using an independent mirrored sequence: two downward crossings with an intervening upward crossing, a break below the first confirmation window's lowest traded price, and 60 seconds continuously below both thresholds. Selling closes this Strategy's existing position; it does not open a short.

## Acceptance

Retain actual Demo Fill evidence, consistent account reconciliation, feedback satisfying its declared observation horizons and sample requirements, and a human Research Review Decision. Negative returns do not by themselves invalidate workflow acceptance. No fills require diagnosis, not relaxed signals or Risk controls. Never label insufficient observations as completed feedback.

## Historical implementation gap

ADR 0049 originally excluded Trade-triggered Strategy decisions. The qualified runtime now supports `TradeEvent` decisions, forming-bar EMA updates, and the 60-second confirmation state machine. Paper Experiment launch keeps the Experiment in `Preparing` while the three running Bots warm up; only after each has a `warmup-complete` evidence record does the Host move the experiment through `Armed` to `Running` and permit risk. Replacing this with closed-bar sampling would change the Strategy. Fifteen-minute OHLC alone cannot establish the intrabar sequence or continuous 60-second conditions.

After the User reset local data and reconciled the account, a read-only inspection of the local persisted Paper account found `okx_demo`, `reconciled`, execution `blocked=false`, cash 98084.28951395709 USDT, zero reserved cash, zero positions, zero orders, and zero Fills. The provider observation timestamp is 2026-09-11 23:13:11.638 UTC. This is persisted reconciliation evidence, not a new provider balance request. The Desktop control tool could not attach to the currently running development app. No new Bot was started and no order was submitted during this inspection.

## Equal-capital comparison

The User authorized allocating the available Demo USDT equally to compare the three instruments. Assign BTC-USDT, ETH-USDT, and SOL-USDT 32694.76 USDT each, totaling 98084.28 USDT; leave the 0.00951395709 USDT rounding remainder unallocated. Within each allocation, initially cap entry notional at 32367.81 USDT and retain 326.95 USDT for fees and execution headroom. This reserve is a sizing policy, not a fee estimate; actual instrument precision, fees, and price protection may lower executable size further.

Keep each instrument's capital and results separate, with no borrowing from another allocation. Use the same observation window and frozen Strategy parameters. Compare net equity return against the identical initial allocation, including fees and separately reported realized and unrealized PnL; also report drawdown, completed trades, and exposure time. No-trade or incomplete-feedback periods must remain explicitly identified. Recheck reconciliation and available funds before deployment; this document records a budget, not configured Bots or executed orders.

Reports are created only after the experiment reaches `Completed` or `Incomplete` and remain idempotently frozen once linked. Cash, FIFO cost, PnL, and fee totals use exact quote-denominated fee evidence; missing fee conversion keeps the result non-rankable. If a Bot recovers during a running experiment, Host execution remains blocked until every current Bot Attempt is warmed again.

## Unresolved execution inputs

- Supervised observation window and end-of-window handling of open positions.
- Exact EMA initialization, event ordering, continuity-gap handling, retry semantics after a failed second confirmation, and sequence persistence/recovery must be frozen before qualification.
- Historical Trade coverage and replay suitability must be verified before choosing a backtest interval or claiming historical equivalence.

## Current acceptance evidence (2026-09-12)

- The product qualification path has generated valid EMA Double-Cross qualifications for BTC-USDT, ETH-USDT, and SOL-USDT from the current complete 15m snapshots and shared frozen Universe Snapshot. No provider orders or Fills were created by this operation.
- Deterministic verification currently passes: the full Rust workspace has 639 passed and 12 ignored tests, and TypeScript no-emit, Biome lint, the Vite production build, and 8 focused Paper Experiment frontend tests pass. The current macOS ARM64 debug app bundle builds with bundle identifier `bid.adaq.desktop`.
- A local `cargo-xwin` attempt reaches the target dependency graph but cannot build the Windows vendored OpenSSL dependency on macOS because the `VC-WIN64A` path requires a Windows-style Perl implementation and `nmake.exe`. The supported Windows CI workflow provisions native MSVC tooling; no local Windows runtime claim is made.
- Per User direction on 2026-09-12, Windows validation is temporarily deferred for this round; continue macOS and local checks only, with no Windows acceptance claim.
- The macOS app was launched for process smoke only. Computer Use could not attach, so no Desktop control or report state is claimed from that launch.
- The actual supervised Demo observation has not started: an explicit common UTC start/end and User supervision are still required. Until its provider acknowledgements, natural Fills, reconciliation, report, feedback review, and Desktop evidence exist, full workflow acceptance remains incomplete.
