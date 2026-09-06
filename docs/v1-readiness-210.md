# V1 Readiness Assertion: #210

Observed 2026-09-06 against the current repository and the latest retained
Desktop, GitHub Actions, release, and issue evidence. This is a scoped
readiness record, not a global V1 flag.

## Declared scope

| Field | Evidence |
| --- | --- |
| Capability and journey | OKX Spot market data → research → supervised OKX Demo Paper → Operations → immutable Paper Feedback and human review |
| Reviewed source commit | [`226dcf1`](https://github.com/tonywxx/adaq/commit/226dcf1), current `origin/main`; the local `pnpm-lock.yaml` change remains unrelated and excluded |
| Package identity | Current source metadata is `0.9.7`; published [`v0.9.6`](https://github.com/tonywxx/adaq/releases/tag/v0.9.6) targets `46b1865` and contains macOS ARM64 and Windows x86_64 assets |
| Supported platforms | macOS ARM64 and Windows x86_64 as declared by the acceptance/release workflows; no Linux asset or readiness claim |
| Interface locales | `en-US` and `zh-CN` route workflow inspected in the packaged macOS Desktop; Windows remains automation/release evidence only |
| Desktop identity | `adaq.app`, bundle identifier `bid.adaq.desktop` |

## Criterion disposition

| Criterion | Evidence and disposition |
| --- | --- |
| OKX Demo reconciliation | Successful account-level reconciliation is retained in [#205](https://github.com/tonywxx/adaq/issues/205#issuecomment-5548577510) and [#206](https://github.com/tonywxx/adaq/issues/206#issuecomment-5557183105). The fresh #209 reconciliation failed closed because the OS secret store was unavailable, and the account remained reconciliation-required. |
| Bot decision/order/fill/maturity sample | Not met. The latest retained acceptance state has `0` qualifying current Bot decisions, `0` venue orders, `0` provider acknowledgements/fills, and `0` matured observations; [#206](https://github.com/tonywxx/adaq/issues/206#issuecomment-5557183105) records this explicitly. No Demo order was fabricated or issued to fill the gap. |
| Paper Feedback and human review | [#207](https://github.com/tonywxx/adaq/issues/207#issuecomment-5557533044) retains Snapshot `e8a36d32-602f-4e3d-9290-1feba8d9f970`, four non-ready lens Reports, and Review Decision `096f9c18-c669-4f9e-8d51-51f790931639`; the faulted Attempt has `0` realized observations. This demonstrates fail-closed review, not readiness. |
| Failure and recovery matrix | [#209](https://github.com/tonywxx/adaq/issues/209#issuecomment-5561417017) records the current macOS evidence and both locales. Clock-invalid, worker hang/deadline, successful fresh reconnect, successful credential rotation, automated WCAG, performance-budget, retention-duration, and interactive Windows checks remain unproven or unavailable. |
| CI and release | V1 Acceptance run [34049757460](https://github.com/tonywxx/adaq/actions/runs/34049757460) and Release run [34049784530](https://github.com/tonywxx/adaq/actions/runs/34049784530) both failed at `2e72ca5`; neither is current-head green evidence for `226dcf1`. [#208](https://github.com/tonywxx/adaq/issues/208#issuecomment-5561145816) was closed with release-page verification left for manual review. |
| Upstream Paper Feedback issues | [#202](https://github.com/tonywxx/adaq/issues/202#issuecomment-5557530848), [#203](https://github.com/tonywxx/adaq/issues/203#issuecomment-5557529243), and [#204](https://github.com/tonywxx/adaq/issues/204#issuecomment-5557529255) are closed with criterion-level implementation/product evidence; their evidence explicitly keeps the final readiness sample separate. |

## Explicit limitations and invariants

- No qualifying current portfolio/cross-sectional universe, accepted Strategy
  qualification, or matured decision sample is available. Strategy evidence
  therefore remains non-directional.
- Historical account equity and drawdown are not retained; the Strategy lens
  does not present fabricated PnL.
- ADR 0082 is satisfied by typed, bounded, append-only local SQLite metadata
  and owning Parquet evidence with redaction at the Host boundary; no cloud or
  duplicate evidence-row store is introduced. See
  [ADR 0082](./adr/0082-keep-v1-local-storage-authority-and-defer-cloud-services.md)
  and [ADR 0084](./adr/0084-define-v1-product-readiness-as-scoped-assertions.md).
- No Live provider, real-money action, credential exposure, stale Target
  replay, duplicate order, fabricated metric, automatic retraining, challenger
  switch, or hot-patch occurred. Required unknown or critical state remains
  fail-closed and append-only evidence remains inspectable.

## Decision

**Not Ready, with concrete blockers.** The missing current-head Demo
decision/order/fill/maturity evidence, incomplete failure/hardening coverage,
and absent current-head green CI/release evidence prevent a `Ready for declared
OKX V1 scope` assertion. Reviewer: Tony W (repository owner). Date:
2026-09-06.

Follow-up: [#211](https://github.com/tonywxx/adaq/issues/211) is the unstarted
manual acceptance task for capturing the missing current-head sample and
re-running the exact release evidence. Closing #210 records this explicit
Not Ready assertion and does not approve V1 readiness.
