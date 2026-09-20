# Capture provider Fills before writing any local terminal order state

Status: accepted; see ADR 0100 for how a captured provider Fill interacts with provider-authoritative balances.

## Context

The local Paper ledger replaces its `AccountSnapshot` wholesale from provider-authoritative balance observations, while its `orders` and `fills` remain ADAQ-owned. `PaperLedger::reconcile` compared only cash and positions, so it could declare `Reconciled` after the retained order and Fill evidence had already diverged from the provider. Because a filled order leaves the provider's open-order set, the missing-order sweep in `cancel_missing_provider_orders` read that absence as cancellation. An OKX Demo order that genuinely filled on 2026-09-15 was therefore recorded locally as `cancelled` with `filled_quantity: 0` and no Fill, while the provider held the resulting position and the cash had been spent.

## Decision

A provider-owned order never reaches a terminal local status from absent evidence. Before the ledger may mark a provider order Cancelled or Filled, the Host must fetch that order's terminal provider state and its exact per-trade Fill evidence and record them. Filled quantity and Fills enter the ledger only through `sync_provider_order_with_trades` under `FillEvidence::TradeObserved`; no Fill is synthesized locally for a provider order. If terminal provider evidence cannot be obtained, the order stays uncertain and the account blocks new risk until reconciliation succeeds — absence from the open-order set is never itself treated as terminal evidence.

`ReconciliationState::Reconciled` therefore asserts that cash, positions, orders, and Fills all carry terminal provider evidence, not merely that cash and positions agree.

## Consequences

- Reserved cash for a provider order is released only after its terminal evidence is captured, so a partially or fully filled order no longer silently releases its reservation.
- The entry path syncs the provider order immediately after submission, and reconciliation resolves any order that has since left the open set; both paths share one fill-capture routine.
- Realized PnL, FIFO cost basis, and Paper Feedback reports depend on exact Fill evidence, so their metrics remain Not Yet Realized until that evidence exists.
- Obtaining no Fill evidence during a supervised observation window is a diagnosis trigger. It is never a reason to relax the signal, the Risk Policy, or the observation window, and it does not by itself invalidate workflow acceptance.
- A pre-existing local order whose Fill evidence was lost must be repaired from provider terminal evidence rather than by editing the ledger by hand.