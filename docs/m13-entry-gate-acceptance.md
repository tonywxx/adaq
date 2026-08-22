# M13 Entry Gate Acceptance Record

Status: **automated gates passed; OKX-only product-run acceptance in progress; A-share and U.S. equity acceptance deferred**

## Current delivery scope

Until stable A-share and U.S. equity data sources are available, end-to-end development and product-run acceptance proceed through the OKX Spot path only. A-share and U.S. equity paths are explicitly **Not Tested / Deferred** for this delivery slice. This is a temporary scope deferral, not a V1 readiness claim or removal of the three-market target.

This record closes the executable part of the M13 prerequisite work. A reviewer still records the desktop GUI/provider run in the matrix below.

## Automated evidence

| Check | Result |
|---|---|
| `cargo test --workspace` | Passed |
| `cargo check --workspace` | Passed |
| `pnpm exec jest --watchman=false --runInBand` | Passed: 37 suites, 120 tests |
| `pnpm run build` | Passed |
| Biome focused check | Passed |
| `git diff --check` | Passed |
| `pnpm tauri dev` | Compiled and launched the desktop binary; timed out while the interactive app remained open |

## Gate matrix

| Gate | Automated coverage | Product run |
|---|---|---|
| OKX acquisition visibility | Rust lifecycle tests and Data Foundation UI test | Pending: OKX-only flow |
| OKX cancellation and retry | Rust lifecycle tests and persisted operation ledger | Pending: OKX-only flow |
| OKX Host restart recovery | Rust restart recovery test | Pending: OKX-only flow |
| OKX evidence retention and fail-closed quality | Pipeline, snapshot, and Context tests | Pending: OKX-only flow |
| OKX Features → Factors → Models Context handoff | Context freeze and attempt-binding tests | Pending: OKX-only flow |
| OKX stale, mixed-market, incomplete, inaccessible evidence rejection | Context contract tests | Pending: OKX-only flow |
| OKX bilingual Data Foundation surface | i18n tests and UI test | Passed for observed surface; OKX flow remains pending |
| A-share and U.S. equity product-run acceptance | Provider-backed GUI run | Not Tested / Deferred |

## Product-run evidence to record

For the current delivery slice, record the operation ID, market, venue, state transition, timestamps, visible error, and retained evidence for OKX Spot only. Execute success, cancellation, retry after failure, and host restart with an in-flight OKX operation. Then select the published Snapshot and Point-in-Time Universe in Data Foundation, establish the Context, freeze it once in Features, Factors, and Models, and record the frozen revision, operation ID, and lineage. A-share and U.S. equity scenarios are Not Tested / Deferred until stable data sources are available.

Provider credentials and a desktop GUI run are required for this section. The automated suite remains the source of truth for deterministic contract behavior.

## Exploratory observations (not current acceptance)

Reviewed on macOS desktop at commit `2144131` using the authenticated ADAQ GUI. No credentials, tokens, private paths, or full provider responses are recorded here.

| Scenario | Evidence | Result |
|---|---|---|
| OKX acquisition success | `crypto-foundation-997eacda-9824-4a30-822a-1c68b98a94ac` | Completed; OKX evidence remained visible in the ledger. |
| A-share cancellation | `a-shares-foundation-f08a0c4f-dfe7-4bb1-9872-e76363902d71` | Cancelled; the retained typed error was visible after GUI refresh. |
| A-share retry/provider failure | `a-shares-foundation-257b0c80-7ee5-47a0-8b9b-dbb3f1fbe81c` | Failed with a typed provider decode error and retained response hash; no false readiness was granted. |
| Retry after A-share failure | `a-shares-foundation-c1def5c9-e885-4a6c-a8f0-83700fd50f4f` | Retry created a new operation and retained the same typed provider decode failure. |
| U.S. equity current provider path | `us-equities-foundation-64e25661-709b-48d2-9d2f-ec5295988f04` | Failed with typed `not_found`; current Yahoo path did not produce an equity universe. A prior Alpaca operation completed, but it is not a fresh current-path success. |
| Host restart recovery | A-share operation was terminated during a second attempt, but the provider failed before termination could be observed as an in-flight restart case. | Not demonstrated; remains pending. |
| Context selection and freeze | Data Foundation showed no published Snapshot or Point-in-Time Universe; `Establish Context` was disabled. | Blocked before Features → Factors → Models freeze. |
| Simplified Chinese GUI | Settings switched the interface to `zh-CN`; Data Foundation labels, statuses, and retry controls remained visible and localized. | Passed for the observed surface. |

The exploratory A-share and U.S. equity observations above are retained only to explain the deferral; they are not acceptance evidence for the current OKX-only scope. Current product-run acceptance remains **Pending** until the complete OKX flow is exercised.
