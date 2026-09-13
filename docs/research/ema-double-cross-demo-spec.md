## Problem Statement

The User wants to evaluate their EMA5/EMA10 double-cross Strategy on BTC-USDT, ETH-USDT, and SOL-USDT using equally funded OKX Demo executions, from data acquisition through Factors, Strategy qualification, supervised Bots, and a final comparison report. They want to know which Instrument performed best during the same experiment, with actual execution and feedback evidence rather than a synthetic profitability claim.

V1 engineering delivery does not supply this exact behavior: its Bot Decision Clock accepts confirmed Closed Bars and scheduled cross-sectional batches, while the requested Strategy evaluates a forming 15-minute bar and requires continuous 60-second confirmations. Fifteen-minute OHLC cannot prove the order of intrabar crossings, extrema, or interruptions. Model training is unnecessary and explicitly excluded.

## Solution

Deliver one supervised, local, OKX Demo-only experiment across the three Instruments. Acquire and retain the evidence needed to reconstruct forming-bar EMA values, build reusable EMA Factors, qualify the deterministic Strategy against event replay, then run three equally funded instrument-specific Bots under the existing Host authority, Risk, OMS, reconciliation, and operational controls. Produce a same-window report of net equity return, realized and unrealized PnL, fees, drawdown, completed trades, exposure, and evidence completeness, followed by a human Research Review Decision.

A missing signal, unavailable historical Trade coverage, incomplete feedback, or runtime interruption must remain visible. None permits manufacturing a Fill, relaxing the Strategy to obtain a trade, or substituting closed-bar sampling for the agreed behavior.

## User Stories

1. As a User, I want BTC-USDT, ETH-USDT, and SOL-USDT bound explicitly to the experiment, so that Watchlist changes cannot silently change its scope.
2. As a User, I want public OKX Spot observations kept separate from OKX Demo account evidence, so that I can trace both market inputs and simulated execution outcomes.
3. As a User, I want data acquired through the existing Data Foundation workflow, so that provenance and acquisition failures remain inspectable.
4. As a User, I want historical coverage and gaps reported before replay, so that I know which intervals can support the Strategy.
5. As a User, I want prospective Trade recording when adequate history is unavailable, so that missing evidence is not replaced with invented intrabar paths.
6. As a User, I want EMA5 and EMA10 evaluated on a forming 15-minute bar, so that signals implement the Strategy I described.
7. As a User, I want deterministic EMA warmup and bar boundaries, so that repeated evaluation of the same evidence agrees.
8. As a User, I want the first upward crossing confirmed for 60 continuous seconds, so that a temporary crossing does not establish a reference.
9. As a User, I want the highest traded price of that successful confirmation window retained, so that the later breakout has an auditable threshold.
10. As a User, I want an intervening downward crossing before the second upward crossing, so that repeated updates above the EMA are not counted as new crossings.
11. As a User, I want both the second breakout and positive EMA relation maintained for 60 seconds, so that the buy follows both conditions.
12. As a User, I want threshold equality and failed confirmation treated consistently, so that boundary cases do not produce accidental trades.
13. As a User, I want the first confirmed reference retained without a time-only expiry, so that the implementation follows the accepted Strategy.
14. As a User, I want an independent mirrored downward sequence for selling, so that exit behavior is as explicit as entry behavior.
15. As a User, I want sales limited to the Strategy's own Spot position, so that no short position or unrelated holding is sold.
16. As a User, I want no repeated additions while holding a position, so that each Instrument follows the same capital policy.
17. As a User, I want EMA Factors and a rule-based Strategy without a predictive Model, so that the workflow contains only necessary research components.
18. As a User, I want event replay and live Demo evaluation to share the same Strategy semantics, so that qualification applies to the deployed behavior.
19. As a User, I want late, duplicate, missing, and out-of-order observations handled explicitly, so that corrupted timing cannot authorize an order.
20. As a User, I want disconnections to invalidate unproven confirmation time, so that silence does not count as a successful 60-second hold.
21. As a User, I want restart recovery to preserve evidence without replaying stale orders, so that retries cannot duplicate exposure.
22. As a User, I want the Host to own credentials, Risk, OMS, and execution, so that a Strategy Component cannot submit orders directly.
23. As a User, I want a current reconciled Demo account checked before starting, so that an old balance is not treated as available capital.
24. As a User, I want equal initial USDT allocations, so that differences in initial capital do not distort the comparison.
25. As a User, I want identical fee reserves and instrument-aware quantity rounding, so that the comparison uses the same sizing policy without exceeding funds.
26. As a User, I want each Bot's capital reservations, orders, Fills, and PnL attributed separately, so that one Bot cannot spend another allocation.
27. As a User, I want one declared observation window for all three Instruments, so that reported returns are comparable.
28. As a User, I want supervised Start, Pause, Resume, Retry, and Stop behavior, so that runtime recovery stays under my control.
29. As a User, I want the experiment to stop taking new risk at its end, so that reporting does not leave unbounded autonomous activity.
30. As a User, I want ending holdings and unrealized PnL reported separately, so that a profitable-looking open position is not presented as a completed profitable trade.
31. As a User, I want net returns after actual fees and cash flows, so that the ranking reflects the experiment's observed outcome.
32. As a User, I want drawdown, completed trades, exposure time, and interruptions beside returns, so that I can assess the limits of the ranking.
33. As a User, I want no-trade and insufficient-evidence outcomes reported honestly, so that zero observations do not become a profitability conclusion.
34. As a User, I want immutable Paper Feedback Snapshots and Reports linked to a Research Review Decision, so that my next research iteration is traceable.
35. As a User, I want English and Simplified Chinese product flows with current Desktop evidence, so that engineering checks also correspond to usable behavior.

## Implementation Decisions

### Authority and scope

- This is a post-V1 capability specification, not an assertion that the current V1 runtime can already execute the Strategy. Explicitly extend the decision-source restriction in ADR 0049 for this qualified event-driven workflow; retain its causality rules and all remaining Host, Risk, OMS, and supervision boundaries.
- Reuse Market Data Foundation, Feature/Factor evaluation, Strategy qualification, Bot Supervisor/Worker, Paper Execution Adapter, Paper Account Reconciliation, and Paper Feedback boundaries. Avoid a second trading engine, ad-hoc credentialed runner, or parallel ledger outside the application.
- Add the smallest versioned event-decision contract needed for retained Trade observations, a forming 15-minute input, confirmation state, and an explicit Decision Time. Existing Closed-Bar and scheduled-batch behavior must remain valid and unchanged.
- The Worker receives evidence and returns Strategy Targets. Only the Host can authorize a target, reserve capital, and submit an OKX Demo order. Never use live trading endpoints or a client-supplied account identity as authority.
- Do not require a placeholder Model, synthetic Forecast Signal, or training run. The qualified deployment explicitly declares its Factor and Strategy dependencies without a predictive Model.

### Accepted trading behavior

- Evaluate each Instrument independently. Use EMA5 and EMA10 on the forming 15-minute close, with a first upward crossing sustained for 60 seconds to establish the reference high from traded prices in that successful window.
- A second upward crossing must follow an intervening downward crossing. Only after price exceeds the first reference high may the second 60-second confirmation begin. Both price above the reference and EMA5 above EMA10 must hold for the entire confirmation before a buy signal is emitted.
- Preserve the first confirmed reference without an age-only expiry. Clear the buy sequence after entry, and never add to an existing position.
- Selling mirrors the two-cross and confirmation behavior using EMA5 below EMA10 and price below the first downward confirmation's low. It closes only the Strategy's owned position; no shorting or leverage.

### Deterministic specification defaults

The following resolve implementation details not separately discussed with the User. They are proposed engineering defaults in this specification, not claims of prior user decisions; freeze them in the qualified Strategy identity and expose them in its evidence.

- Align 15-minute bars to the Crypto UTC grid. Seed each EMA with the arithmetic mean of its first N contiguous confirmed closes, then use the standard EMA recurrence. Do not arm signals until both EMAs are warmed up. For each forming-bar price, derive a provisional EMA from the previous confirmed EMA and that price; do not advance the EMA recurrence once per Trade. Commit the EMA only when that bar closes.
- Define an upward crossing as a transition from EMA5 <= EMA10 to EMA5 > EMA10, and downward crossing symmetrically. Equality fails a strict hold condition. Starting above or below the other EMA does not manufacture a crossing.
- Include the crossing observation and successful confirmation-end observation in the first window's extrema. Store the reference, timestamps, and evidence identities. No future observation may revise a frozen reference.
- A failed first confirmation needs a new crossing to restart. After a first reference is confirmed, retain it until entry or evidence-invalidating recovery. A failed second confirmation resets only its timer; while the second upward phase remains valid, a later price recross can restart the timer. If the EMA relation reverses, require a new upward crossing before another buy confirmation. Apply the symmetric rules to exits.
- Arm the sell sequence only after owned entry Fill evidence exists; reset exit state when flat. Pending and partially filled orders block duplicate entry, and account/order reconciliation governs transitions. A partial entry counts as a position and must not trigger a second entry order to fill the unused budget.
- Interpret continuous confirmation against a complete, ordered, qualified observation stream, not inaccessible exchange activity. Use provider observation time with retained arrival/availability metadata; the 60-second endpoint needs a qualifying observation at or after the deadline. A disconnected or unproven interval cannot satisfy the timer. Retain the source's verified continuity and freshness policy with the deployment rather than silently inventing a fixed gap threshold.
- Deduplicate by stable provider identity; handle ordering under the frozen connector contract. Gaps, unresolved ordering, clock failures, or unavailable required evidence block targets and invalidate pending confirmation. Recovery requires contiguous replay to reconstruct state, or an explicit fresh warmup with cleared references when reconstruction is impossible. Do not silently carry a partial timer across a gap.
- Persist sufficient versioned state and evidence identity for deterministic reconstruction. Restart, Resume, and Retry retain the existing reconciliation and warmup gates and never submit a stale pre-interruption signal. Duplicate decision delivery must not create another order.
- Preserve exact decimal prices, quantities, capital, fees, and thresholds. Reuse the existing qualified indicator convention for analytical EMA values and freeze its numeric comparison behavior. No implicit epsilon may change crossing semantics.

### Acquisition and qualification

- Qualify actual available historical Trade coverage and provenance before selecting the replay range. Confirmed 15-minute bars can initialize EMA state but cannot stand in for the intrabar confirmation evidence.
- Where suitable historical evidence is unavailable, begin retained prospective acquisition and replay only the verified interval. Report limited coverage; do not synthesize tick paths from OHLC or claim full historical equivalence.
- Replay and online operation use the same Strategy evaluator and frozen parameters. A replay Fill is simulated evidence and remains distinct from an actual OKX Demo Fill. Neither may be executed against an event earlier than the resulting decision.
- Run ordinary Component, Strategy, Backtest/Validation, and Deployment Qualification appropriate to the new decision mode. Qualification must cover the event semantics instead of relabelling a closed-bar backtest as equivalent.

### Equal capital and experiment lifecycle

- The User authorized three equal allocations from Demo funds. The inspected persisted reconciliation snapshot at 2026-09-11 23:13:11.638 UTC showed 98084.28951395709 USDT cash, zero reserved cash, positions, orders, and Fills; it is historical preparation evidence, not a future funding guarantee. No credential or private account identifier is needed in the public specification.
- Initial budget: 32694.76 USDT per Instrument; total 98084.28 USDT; 0.00951395709 USDT remains unallocated. Initial entry-notional cap: 32367.81 USDT per Instrument, leaving 326.95 USDT inside each allocation for fees and headroom. This is a sizing reserve, not an assumed provider fee rate.
- Refresh account reconciliation before deployment. If funding cannot support these budgets, block launch with the shortfall rather than silently reduce one Instrument or use leverage. Budget changes require a new explicit equal-capital experiment configuration.
- Maintain logical capital attribution under the existing account-scoped Paper Portfolio. Each instrument-specific Bot must reserve only its allocation; concurrent requests cannot overspend the shared account. No cross-allocation borrowing or rebalancing. Later entries use at most the lesser of the initial entry cap and 99% of that allocation's currently free cash, rounded down under current qualified instrument rules.
- Fees and Fills must use provider evidence, including non-USDT fee assets with explicit valuation evidence. Missing valuation prevents a purported exact net-return ranking. Enforce tick/lot/minimum-notional and price protection through existing execution rules.
- Require an explicit common UTC start/end observation window as a launch input; no duration has been selected in this conversation. Arm the experiment only after all three deployments are qualified, reconciled, and warmed up. Retain per-Instrument outages and effective exposure time within the common window.
- At window end, block new entries and apply existing Stop and Keep Position behavior: cancel eligible pending orders and reconcile; retain open positions as Unmanaged Positions. Do not flatten automatically. This is the default from ADR 0050, not permission for an additional exit trade. An unresolved stop remains visible and cannot be reported as a clean terminal state.
- Use the existing supervised local execution contract. Fault recovery requires operator Resume or Retry; a network reconnect alone does not restore trading authority. No unattended cloud service or indefinite monitoring daemon is introduced.

### Reporting

- Freeze a same-window report for all three allocations, with start capital, ending attributed cash/positions, realized PnL, unrealized PnL, actual fees, net equity return, maximum drawdown at the declared valuation cadence, completed trades, exposure time, and operational interruptions.
- Net equity return uses each identical initial allocation as denominator and a common end-time valuation with explicit freshness. Separate and flag external cash flows or unrelated account activity; do not silently treat them as Strategy profit. Do not call the sum of account and position values profit twice or deduct already-accounted fees twice.
- Rank only comparable, valid observed net returns. Report no-trade and insufficient-evidence states beside the result. A highest observed return is not a prediction or proof of profitability; negative returns are valid results.
- Retain immutable Paper Feedback Snapshots and lens-specific Reports with observation horizons and existing frozen sample requirements. Actual Demo Fills are necessary for the requested execution evidence, but do not by themselves satisfy every feedback horizon or sample gate.
- At the observation deadline, produce the available report even if no trade occurred; label full workflow acceptance incomplete where its evidence is missing. Do not extend the experiment indefinitely, fabricate success, or loosen signals to obtain Fills.
- The final human Research Review Decision may retain, revise, or reject the Strategy. It never automatically retrains, modifies the active bundle, or promotes a replacement.

## Testing Decisions

- Primary seam: the existing Host-supervised Strategy-to-Paper workflow. Drive it with retained deterministic Trade evidence and a controlled clock, and assert externally visible decisions, order intents, Risk rejections, ledger outcomes, lifecycle, and report values. Prefer this seam over testing private helpers or building a parallel engine.
- Use the existing Worker IPC and Paper Adapter boundaries to provide deterministic event/clock inputs and controlled execution outcomes. Add only the minimal clock/event injection required if the current boundaries cannot express the new decision mode.
- Test first and second confirmation, intervening reverse crossing, equality, extrema boundaries, failed timers, rearming, mirrored exits, partial Fills, no additions, and no shorting. Include a case with identical 15-minute OHLC but different intrabar ordering that must produce different outcomes.
- Verify provisional EMA is recalculated from the last confirmed EMA, not advanced per Trade, and that bar transitions and warmup reproduce the frozen convention.
- Replay duplicate, late, out-of-order, missing, and disconnected evidence; assert no unproven confirmation or stale target can submit an order. Restart and retry the same retained sequence and verify deterministic reconstruction and idempotent execution.
- Verify all three Bot allocations concurrently against one Demo-account ledger, exact decimal budget conservation, fee/headroom limits, partial execution, and insufficient account funds. Check reporting using known cash, Fills, fees, and end valuations, including non-USDT fees and unrelated cash-flow cases.
- Existing prior art includes Bot lease conflict and User-scope fail-closed tests, restart recovery requiring reconciliation, Worker decision deadline and target validation, and Factor/Strategy closed-bar alignment tests. Preserve those regressions and extend the same public seams for event decisions.
- Run one actual supervised OKX Demo experiment after deterministic qualification; retain provider acknowledgements, Fills when naturally produced, reconciliation, operations, and report provenance. Synthetic replay cannot replace this evidence.
- Verify Desktop controls and report flows in English and Simplified Chinese on supported macOS ARM64 and Windows x86_64 surfaces. Record the exact build and actual check results; build success is not Desktop-runtime evidence.
- No profitability assertion is a test expectation. Full workflow acceptance requires the agreed execution/feedback evidence and User review; a truthful no-trade report is a valid report but not a completed filled-execution acceptance.

## Out of Scope

- Predictive Model training, tuning, Forecast Signal placeholders, and automatic research promotion.
- Live Trading, real-money execution, leverage, shorting, market making, queue-position models, or general-purpose HFT support.
- Other Instruments, Providers, market expansion, external-user onboarding, cloud execution, and unattended operation.
- Forced trades, automatic end-window flattening, artificial intrabar data, fabricated Fills, or guaranteed returns.
- Broad runtime refactoring, a separate trading service, and compatibility machinery unrelated to the scoped versioned event contract.
- Immediate order submission or Bot deployment merely from publishing this specification.

## Further Notes

This specification synthesizes the accepted discussion and makes remaining engineering defaults explicit. The observation window is a required launch-time input, not a missing implementation requirement. Actual historical coverage remains an acquisition result with a defined prospective-recording fallback.

Relevant decisions: ADR 0002 (financial decimals), ADR 0049 (the V1 decision-source restriction being extended), ADR 0050 (supervised lifecycle and recovery), ADR 0054 (human-reviewed immutable feedback), ADR 0090 (OKX-only baseline), and the local post-V1 personal-use decision. The existing V1 completion statement is not reopened by this feature.

Publish this as a specification with `ready-for-agent`. It is the input to `to-tickets`, not a single all-in-one implementation ticket. Ticket decomposition must preserve serial dependencies and fresh module verification, User acceptance, and explicit continuation before the next module. No implementation children or production mutations are created by this specification step.
