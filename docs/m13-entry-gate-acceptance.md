# M13 Entry Gate Acceptance Record

Status: **automated gates passed; product-run acceptance pending**

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
| Three-market acquisition visibility | Rust lifecycle tests and Data Foundation UI test | Pending |
| Cancellation and retry | Rust lifecycle tests and persisted operation ledger | Pending |
| Host restart recovery | Rust restart recovery test | Pending |
| Evidence retention and fail-closed quality | Pipeline, snapshot, and Context tests | Pending |
| Features → Factors → Models Context handoff | Context freeze and attempt-binding tests | Pending |
| Stale, mixed-market, incomplete, inaccessible evidence rejection | Context contract tests | Pending |
| Bilingual Data Foundation surface | i18n tests and UI test | Pending |

## Product-run evidence to record

For each market (OKX, China A-shares, U.S. equities), record the operation ID, market, venue, state transition, timestamps, visible error, and retained evidence. Execute success, cancellation, retry after failure, and host restart with an in-flight operation. Then select the published Snapshot and Point-in-Time Universe in Data Foundation, establish the Context, freeze it once in Features, Factors, and Models, and record the frozen revision, operation ID, and lineage. Repeat with stale, mixed-market, incomplete, and another-user evidence and record the visible typed blocker.

Provider credentials and a desktop GUI run are required for this section. The automated suite remains the source of truth for deterministic contract behavior.
