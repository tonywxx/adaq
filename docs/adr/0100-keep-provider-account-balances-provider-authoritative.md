# Keep provider account balances provider-authoritative instead of re-simulating Fills

Status: accepted

## Context

`PaperLedger::reconcile` replaces the whole `AccountSnapshot` from a provider balance observation, so a provider-backed account's cash and positions are provider-authoritative. `apply_fill` nevertheless mutated that same account. An order that fills between two reconciliations makes the next snapshot already report the post-fill cash and position, so applying the retained Fill on top of it counted the fill twice: a 1 BTC @ 100 fill against a snapshot showing 999,900 USDT produced 999,800 instead of 999,900, and doubled the position. The same double application reached every path that syncs provider order evidence, including experiment refresh and Flatten, not only reconciliation.

## Decision

For a provider-backed market, cash and positions are provider-authoritative and change only through reconciliation. A Fill captured from provider terminal evidence updates the order's filled quantity, its terminal status, and the retained Fill evidence, and settles ADAQ's own reservation bookkeeping — it never mutates the account's cash or positions, and it does not re-validate the fill against local balances that the provider has already superseded. Only a locally authoritative account, which in V1 is the A-share simulator, keeps the full economic effect of a Fill on its own balances.

## Consequences

- A provider Fill is evidence and reservation bookkeeping, never a source of balance truth, so re-syncing the same provider order cannot drift the account.
- Reconciliation no longer produces a spurious `Reconciliation Required` cycle after every fill, because the adopted snapshot is no longer perturbed by the resolution that follows it.
- A pre-existing local order whose Fill evidence was lost can be repaired by recording provider terminal evidence alone, without a second balance movement.
- Provider-side affordability is no longer asserted locally at fill time. Local buying-power and risk checks continue to gate new orders against the provider-authoritative balance.
- A-share simulation behaviour is unchanged and remains covered by its own fill-engine evidence rules.