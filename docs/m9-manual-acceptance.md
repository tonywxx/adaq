# M9 Manual Acceptance

This is the canonical human-reviewed M9 path. The reviewed local run is macOS ARM64; the command substitutions for Windows x86_64 and Linux x86_64 are recorded below. Perform one row at a time and retain the requested evidence on failure. M9 ends at trustworthy multi-market observation, immutable research evidence, secure non-ordering connections, localization, and market inspection. It does not submit Paper or Live orders and does not deliver M10-M18.

Never put credentials, authorization headers, OTPs, tokens, private paths, or private market data in issue comments, commits, screenshots, logs, exports, or this record. Optional real-provider checks are permitted only with maintainer-owned credentials entered in **Settings → Connections**; committed fixtures and local mock servers are the authoritative acceptance path.

<!-- m9-acceptance:scope -->
## 1. Scope and prerequisites

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| Run `node --version` from the repository root. | Node.js 24 or later is selected, matching the release baseline. | Complete output and installation method. |
| Run `pnpm --version`. | pnpm 11.20.0 is available, matching `package.json`. | Complete output and installation method. |
| Run `pnpm install --frozen-lockfile`. | Dependencies match `pnpm-lock.yaml`. | Complete output and the two tool versions. |
| Run `rustup toolchain install stable`. | The stable Rust toolchain is available. | Complete output and `rustup show`. |
| Run `rustup target add --toolchain stable wasm32-unknown-unknown`. | The component fixture target is installed. | Complete output and installed-target listing. |
| Run `pnpm tauri dev` with Supabase variables supplied outside version control. | The desktop shell opens without exposing configuration values. | Screenshot and redacted error only. |
| Open a fresh device profile and select **Settings → General**. | System, English (US), and 简体中文 are the only locale choices. | Screenshot, platform, and locale state. |

Use `shasum -a 256 <path>` on macOS, `Get-FileHash -Algorithm SHA256 <path>` in Windows PowerShell, and `sha256sum <path>` on Linux. Native file pickers, data-folder paths, display scaling, and secret-store prompts remain platform-specific.

<!-- m9-acceptance:localization -->
## 2. Localization, first paint, and lifecycle

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| Launch the app from the fresh profile. | Loading HTML paints before React/native work; there is no blank window or locale flash. | Platform, route, first visible frame, and console output. |
| Select **English (US)** in **Settings → General**. | The current route remains mounted and English copy appears immediately. | Route, screenshot, and visible missing key. |
| Select **简体中文** in **Settings → General**. | The current route remains mounted, Chinese copy appears immediately, and the document language changes to `zh-CN`. | Route, screenshot, and accessibility-tree text. |
| Set **System**, sign out, sign in again, then open **Settings → General**. | System resolution is stable; the device-local preference is not profile data. | Redacted User ID, locale before/after sign-out, and screenshot. |
| Open the research-data reset confirmation and cancel it. | Research reset scope is explicit; the locale preference remains available. | Confirmation copy and before/after locale. Do not run a destructive reset for acceptance. |
| Navigate through **Markets**, **Components**, **Models**, **Backtest**, **Validation**, and **Settings** in both locales. | No missing visible labels, empty states, error states, loading labels, or accessibility names appear. | Route, locale, key/label, screenshot, and technical details. |
| Run `pnpm exec jest --watchman=false --runInBand src/lib/i18n.test.ts src/bootstrap.test.ts`. | Locale resolution, persistence boundaries, fallback, `Intl` formatting, and first-paint ordering pass. | Revision, suite/test, and complete output. |

<!-- m9-acceptance:connections -->
## 3. Provider Connections and no-order invariant

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| Open **Settings → Connections**. | Alpaca Paper and OKX Demo are the only connection environments shown; no Live/custom endpoint field exists. | Screenshot and visible/technical error. |
| Run `cd src-tauri && cargo test --lib connections`. | Fixture-backed save, test, rotation, deletion, user isolation, endpoint allowlisting, redaction, permissions, currency, and clock-skew tests pass. | Revision, complete output, and failing test. |
| Run `cd src-tauri && cargo test --lib connections connection_test_never_requests_an_order_endpoint`. | The request capture contains account/time/config/balance calls only; no `/orders`, trade, or order endpoint is requested. | Complete output and redacted request paths. |
| Save an Alpaca Paper fixture profile and select **Test connection**. | The profile stores only an opaque Secret Reference and redacted metadata; the test is read-only. | Profile ID, status, typed error, and screenshot; never the key pair. |
| Save an OKX Demo fixture profile and select **Test connection**. | Demo simulation headers, permission, currency, clock, and capability evidence are retained without secret values. | Profile ID, status, typed error, and screenshot; never the key/passphrase. |
| Rotate each fixture profile with an invalid replacement. | The prior usable profile remains active and the failed replacement leaves no orphaned secret. | Profile ID, redacted status, and technical error. |
| Delete each fixture profile after dependent-runtime checks. | Deletion is explicit, removes the OS-store entry, and invalidates the metadata; an active dependent runtime blocks it. | Profile ID, guard result, and redacted diagnostic. |
| Try a Live endpoint or arbitrary custom endpoint in the fixture tester. | The request is rejected before network use. | Endpoint class and typed rejection only. |
| If an optional real-provider check is run, enter credentials only through **Settings → Connections** and delete the profile afterward. | Only the provider's fixed Paper/Demo path is used; no credential value enters issue evidence. | Provider, timestamp, status, and redacted error; never credentials or headers. |

<!-- m9-acceptance:crypto -->
## 4. OKX Spot journey

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| Run `cd src-tauri && cargo test -p adaq-data-pipeline --lib okx::tests -- --nocapture`. | OKX fixture tests cover Instrument Master, pagination, rate/retry handling, closed bars, checkpoints, restart/resume, gaps, revisions, REST/WebSocket reconciliation, bounded Trade retention, and non-persistent Level 2. | Revision, complete output, and failing test. |
| Open `/markets/crypto` and search a fixture-backed OKX Instrument. | Instrument identity is Venue plus native code; provider symbol and source mapping remain visible. | Route, Instrument ID, provider symbol, and screenshot. |
| Inspect the Instrument Master record before selecting a history range. | The effective time, status, full observed-universe evidence, provider response hash, and Point-in-Time selection rule are visible. | Snapshot ID, effective time, evidence state, and missing field. |
| Resume a one-minute acquisition from an interrupted fixture checkpoint. | The acquisition resumes without duplicate records or overwriting prior Source/Canonical revisions. | Operation ID, checkpoint, revision, and diagnostic. |
| Inspect Source and Canonical quality for the acquired range. | Provider/upstream, request, response/content hashes, exact values, gaps, quarantine, quality state, and capability are separate and inspectable. | Dataset IDs, state, gap/quarantine counts, and screenshot. |
| Derive a higher interval from the accepted one-minute evidence. | Aggregation is deterministic, calendar/grid aligned, immutable, and provenance-bound. | Source/Snapshot IDs, interval, hash, and error. |
| Publish or select the resulting immutable Snapshot, then reopen `/markets/crypto`. | Snapshot identity and quality remain stable after re-entry; no order control is present. | Snapshot ID, route, screenshot, and technical details. |

<!-- m9-acceptance:a-shares -->
## 5. China A-share journey

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| Run `cd src-tauri && cargo test -p adaq-data-pipeline --lib a_share`. | Fixture and local-mock tests cover actual-upstream provenance, SSE/SZSE identity, exact decimals, sessions, corporate actions, quality, cancellation, and restart/resume. | Revision, complete output, and failing test. |
| Open `/markets/a-shares` and search a fixture-backed ordinary equity. | The Venue, native code, provider symbol, status, and `akshare-rs` source mapping are visible. | Route, Instrument ID, provider/method, and screenshot. |
| Inspect the acquisition provenance card. | Actual upstream, method, request/response/content hashes, connector version, retrieval time, and capability limitations are visible. | Source ID, hashes, missing field, and screenshot. |
| Inspect one unadjusted Canonical Bar series. | `PriceBasis` is Unadjusted; Asia/Shanghai Trading Date, morning session, midday break, afternoon session, and quality/gaps are explicit. | Series ID, calendar ID, interval, basis, and error. |
| Inspect the separate corporate-action evidence for the same Instrument. | Actions remain independent immutable evidence and are not silently merged into Bars or used for repair. | Action evidence ID, quality state, and screenshot. |
| Publish or select the resulting immutable Snapshot, then reopen `/markets/a-shares`. | Snapshot identity, coverage, quality, limitations, and source provenance remain inspectable. | Snapshot ID, route, screenshot, and technical details. |

<!-- m9-acceptance:us-equities -->
## 6. U.S. equity journey

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| Run `cd src-tauri && cargo test -p adaq-data-core --lib alpaca`. | Alpaca fixture tests cover fixed endpoints, exact values, IEX capability, DST/holiday/early-close calendars, symbol limits, and daily-bar anchoring. | Revision, complete output, and failing test. |
| Run `cd src-tauri && cargo test -p adaq-data-pipeline --lib us_equity`. | Pipeline tests cover authenticated fixture acquisition, pagination/retry, checkpoints, Source/Canonical evidence, quality, and Snapshot compatibility. | Revision, complete output, and failing test. |
| Open `/markets/us-equities` and search a fixture-backed active asset. | Alpaca symbol, Venue identity, status, exchange, tradability, and Instrument Source Mapping are visible. | Route, Instrument ID, provider symbol, and screenshot. |
| Inspect the Provider Capability Snapshot. | Basic plan, IEX feed, history/delay/rate/stream limits, unavailable capabilities, and capture time are explicit; no consolidated realtime claim appears. | Capability ID, feed, limitation, and screenshot. |
| Inspect one historical Bar series and its session evidence. | America/New_York Trading Date, DST, holiday/early-close state, UTC boundaries, `PriceBasis::Unadjusted`, quality, and gaps are visible. | Series ID, calendar ID, state, basis, and error. |
| Publish or select the resulting immutable Snapshot, then reopen `/markets/us-equities`. | Snapshot and provenance remain stable; auxiliary observations, if present, are visibly separate and never repair Canonical data. | Snapshot ID, source/revision IDs, and screenshot. |

<!-- m9-acceptance:quality-snapshot -->
## 7. Quality, lifecycle, and research evidence

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| Run `cd src-tauri && cargo test -p adaq-data-pipeline --lib`. | Passed, Degraded, and Rejected paths, quarantine, explicit gaps, cancellation, atomic cleanup, user isolation, revisions, and Snapshot publication tests pass. | Revision, complete output, and failing test. |
| Submit a fixture with a scheduled closure or session break. | The closure is calendar evidence, not a false Bar Gap. | Venue, Trading Date, phase, quality report, and gap list. |
| Submit a fixture with a missing Bar inside a continuous session. | A genuine Bar Gap is retained and never forward-filled, interpolated, clipped, or manually edited. | Gap range, quality report, and canonical hash. |
| Submit a corrected Source revision for an existing range. | The prior revision remains append-only evidence and the new Canonical/Snapshot identity is distinct. | Source/revision IDs, hashes, and before/after state. |
| Attempt to delete a Snapshot referenced by a Dataset, Run, Report, or later research object. | The deletion lock rejects the operation and reports the dependent reference. | Snapshot ID, dependent ID, and typed error. |
| Replay an old Snapshot after a newer Source revision exists. | The old Snapshot returns its original immutable evidence and is not silently upgraded. | Old/new Snapshot IDs, hashes, and replay result. |
| Sign out and sign in as a second test User, then inspect Watchlist, pipeline, Snapshot, and connection lists. | User-scoped private records and secret references do not cross Users; shared content remains subject to its access contract. | Two redacted User IDs, record IDs, and screenshot. |

<!-- m9-acceptance:markets -->
## 8. Markets GUI, accessibility, and boundaries

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| Visit `/`, `/markets`, `/markets/crypto`, `/markets/a-shares`, and `/markets/us-equities`. | Root remains the Operations shell; all market routes load with localized navigation and no duplicate Watchlist store. | Route, URL, screenshot, and console output. |
| Add one crypto, one A-share, and one U.S. equity Instrument to the Watchlist. | One User-scoped asset-neutral Watchlist preserves Venue-plus-native-code identity; each route filters it correctly. | Redacted User ID, item IDs, route, and screenshot. |
| Remove and re-add one Watchlist item, then select a different Active Instrument. | Limits, selection, active-Instrument behavior, and reset semantics remain intact. | Item IDs, route, before/after state, and error. |
| Re-enter a market route after visiting another route. | Read-only list/chart metadata can paint from current-session cache while the owning control refreshes in the background; native validation is unchanged. | Route, loading owner, cache state, and timing. |
| Set the content area to 1024 px and navigate each market route with the keyboard. | Search, Watchlist, chart, provenance, quality, loading, error, and empty controls remain labeled, focusable, visible, and non-color-dependent. | Platform scale, route, focused control, screenshot, and accessibility-tree text. |
| Search an unavailable quote or provider field. | Bid/Ask, realtime, consolidated coverage, adjusted basis, and open-session claims remain unavailable instead of being invented. | Instrument, field, displayed state, and screenshot. |
| Inspect each market route for order, Feature, Factor, Model-training, Paper, Bot, and Live controls. | No M9 route exposes those out-of-scope actions. | Route and screenshot if any control appears. |

<!-- m9-acceptance:regressions -->
## 9. M7/M8 regressions and bilingual parity

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| Run `pnpm exec jest --watchman=false --runInBand`. | All frontend suites, including M7/M8 research, locale, route, loading, and market tests, pass. | Revision, suite/test, and complete output. |
| Open [`docs/m7-manual-acceptance.md`](m7-manual-acceptance.md) and [`docs/m8-manual-acceptance.md`](m8-manual-acceptance.md). | Existing Components, Backtests, Validation, Model Dataset, and Forecast Evaluation paths remain the canonical regression paths. | Guide section and broken/changed path. |
| Run the focused M8 guide contract test. | Both M8 guides remain executable and both READMEs still link them. | Revision and complete output. |
| Check the English and Simplified Chinese M9 guides side by side. | Headings, operation order, expected results, failure evidence, cleanup/safety rules, platform substitutions, matrix coverage, and boundary claims are semantically equivalent. | File, section, mismatched text, and expected meaning. |

<!-- m9-acceptance:automated-gates -->
## 10. Automated release gates and CI

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| Run `cd src-tauri && cargo fmt --all --check`. | Rust formatting passes. | Revision and complete diff. |
| Run `cd src-tauri && cargo test --workspace`. | All Rust workspace tests and doctests pass; an ignored OS-keyring test is recorded as ignored when applicable. | Revision, complete unfiltered output, failing test, and platform. |
| Run `cd src-tauri && cargo check --workspace`. | The native workspace type-checks. | Revision and complete output. |
| Run `pnpm exec jest --watchman=false --runInBand`. | All frontend tests pass. | Revision, suite/test, and complete output. |
| Run `pnpm run build`. | Strict TypeScript checking and the Vite production build pass. | Revision and complete output. |
| Run `pnpm run lint`. | Lint passes; any pre-existing warnings are listed separately from new findings. | Revision, file/rule, and complete output. |
| Run `git diff --check`. | No whitespace errors exist. | Revision and complete output. |
| Search the repository for a configured secret scanner. | No secret-scan command is configured in this checkout; manually verify the diff contains no credential material or token-like fixture values. | Command/output and reviewed file list. |
| Record the applicable GitHub Actions run URLs for `macOS ARM64`, `Windows x86_64`, and `Linux x86_64`. | Native fixture/Rust gates and release packaging evidence are retained for the reviewed revision or explicitly identified platform baseline. | Run URL, SHA, job, conclusion, and failed log excerpt. |

The native matrix is defined in [`.github/workflows/indicator-engine.yml`](../.github/workflows/indicator-engine.yml); release packaging is defined in [`.github/workflows/release.yml`](../.github/workflows/release.yml). A local pass never replaces required platform evidence. The acceptance record must distinguish a reviewed M9 revision from an older platform baseline.

Recorded platform evidence for the unchanged native/fixture and packaging paths:

| Workflow evidence | Revision | Jobs | Result |
| --- | --- | --- | --- |
| [Indicator engine acceptance run 30439984251](https://github.com/tonywxx/adaq/actions/runs/30439984251) | `735240def735d7684ff9e4e8751fbe1498ead778` | macOS ARM64, Windows x86_64, Linux x86_64 | Success |
| [Release run 31282997179](https://github.com/tonywxx/adaq/actions/runs/31282997179) | `5d1d236999984ef4a8bcc646b8e927e37e9fb708` | Validate release, macOS ARM64, Windows x86_64, publish | Success |

The #76 change contains documentation, README/roadmap, and a frontend acceptance-contract test only; it does not alter Rust/provider, secret-store, fixture, or packaging code. The local gates above therefore validate the reviewed revision while the recorded matrix retains the applicable cross-platform baselines.

<!-- m9-acceptance:acceptance-matrix -->
## 11. Final acceptance matrix

The matrix is evidence, not a substitute for a closed issue. Every row names the implementation boundary, a focused check, and the manual section that a reviewer can repeat.

### Parent #66 criteria

| ID | Requirement | Implementation / focused evidence | Manual / broad evidence | Unresolved risk |
| --- | --- | --- | --- | --- |
| P1 | Publish equivalent English and Simplified Chinese guides. | `docs/m9-manual-acceptance.md`, `.zh-CN.md`, `src/m9-manual-acceptance.test.ts`. | Sections 1–11. | None after parity test. |
| P2 | Update both READMEs only after M9 delivery and link both guides. | README milestone, scope, feature, and documentation entries. | Section 9. | None. |
| P3 | Resolve, switch, persist, and audit locale before first paint. | `src/lib/i18n.test.ts`, `src/bootstrap.test.ts`. | Section 2. | None in bundled `en-US`/`zh-CN`. |
| P4 | Save/test/rotate/delete Alpaca Paper and OKX Demo profiles in OS storage without leaks. | `src-tauri/src/connections/tests.rs`; `cargo test --lib connections`. | Section 3. | Real credentials are optional and intentionally not recorded. |
| P5 | Reject Live/custom endpoints and prove every test is non-ordering. | `endpoint_allowlist_is_fixed_and_never_custom`, `connection_test_never_requests_an_order_endpoint`. | Section 3. | None for fixture path. |
| P6 | Complete the OKX acquisition and Snapshot journey. | `adaq-data-pipeline` OKX tests and immutable Snapshot owner. | Section 4. | Optional live rate/provider availability is not an acceptance dependency. |
| P7 | Complete the A-share provenance, unadjusted Bars, calendar, actions, quality, and Snapshot journey. | A-share core/pipeline tests and fixtures. | Section 5. | Actual upstream availability is represented as evidence, not assumed. |
| P8 | Complete the Alpaca/IEX U.S. equity journey and capability disclosure. | Alpaca core/pipeline tests and fixtures. | Section 6. | Optional real credential path is not an acceptance dependency. |
| P9 | Prove quality states, quarantine, gaps, revisions, replay, and deletion locks. | Pipeline, Snapshot, and reference-lock tests. | Section 7. | None for committed fixture paths. |
| P10 | Preserve one User-scoped asset-neutral Watchlist across market routes. | `src/features/markets/market-workspaces.test.ts`, router/Watchlist tests. | Section 8. | None in the tested local boundary. |
| P11 | Re-run M7/M8 smoke paths. | Full Jest/Rust workspace gates and existing M7/M8 guide contract. | Section 9. | Manual provider data is not required for deterministic regression gates. |
| P12 | Keep Features, Factors, Model training, Strategy execution, orders, Bots, and Live trading out of M9. | Routes/DTOs expose observation and evidence only; connection order test. | Section 8 and the scope statement. | None. |
| P13 | Complete localized accessibility and state review. | Router, loading, i18n, and market tests. | Sections 2, 8, and 9. | OS assistive-technology differences remain recorded per platform. |
| P14 | Retain macOS ARM64, Windows x86_64, and Linux x86_64 CI evidence. | Matrix/release workflows and recorded run URLs. | Section 10. | A row is incomplete until the exact run URL/SHA/conclusion is recorded. |
| P15 | Retain this criterion-to-evidence matrix with no closed-issue-only row. | This section plus child matrix below. | Sections 1–10. | None after review. |
| P16 | Post an English completion comment and close only after all applicable gates pass. | Issue #76 completion comment references final revision and commands. | Acceptance record below. | Parent closure is a separate explicit action requested by the maintainer. |

### M9.1–M9.10 slice matrix

| Slice | Delivered boundary | Focused evidence | Final acceptance evidence | Unresolved risk |
| --- | --- | --- | --- | --- |
| M9.1 / #67 | Pre-paint bilingual localization, persistence, fallback, `Intl`, and accessible shell. | `src/lib/i18n.test.ts`, `src/bootstrap.test.ts`. | Section 2 and full Jest/build gates. | None. |
| M9.2 / #68 | Venue/Instrument identity, IANA calendars, sessions, UTC boundaries, and scheduled-closure semantics. | `adaq-data-core` market tests. | Sections 4–7 and Rust gates. | None in supported calendar contracts. |
| M9.3 / #69 | Host-owned OS secret store, fixed Paper/Demo endpoints, redaction, lifecycle, and non-ordering tests. | `cargo test --lib connections`. | Section 3 and Rust gates. | Real OS-store prompts are platform-specific. |
| M9.4 / #70 | Source → Canonical → Quality → Snapshot pipeline with immutable evidence. | `cargo test -p adaq-data-pipeline --lib`. | Section 7 and Rust gates. | None for local fixtures. |
| M9.5 / #71 | Full-universe OKX Spot evidence, resumable one-minute history, and selected realtime evidence. | OKX pipeline/core fixture tests. | Section 4 and Rust gates. | Live provider behavior is not used for deterministic acceptance. |
| M9.6 / #72 | `akshare-rs` A-share path, actual-upstream provenance, unadjusted Bars, actions, and sessions. | A-share core/pipeline fixture tests. | Section 5 and Rust gates. | None beyond provider evidence limits already shown. |
| M9.7 / #73 | Alpaca Basic/IEX path, capability disclosure, calendars, Bars, and stream evidence. | Alpaca core/pipeline fixture tests. | Section 6 and Rust gates. | None beyond Basic-plan limitations already shown. |
| M9.8 / #74 | Point-in-Time Universes, derived intervals, quality, revisions, locks, and Snapshots. | Pipeline/Snapshot/reference-lock tests. | Section 7 and Rust gates. | None for committed fixtures. |
| M9.9 / #75 | Localized four-route Markets GUI and one user-scoped Watchlist. | `market-workspaces.test.ts`, `router.test.ts`, loading tests. | Section 8 and full Jest/build gates. | OS visual differences remain manual evidence. |
| M9.10 / #76 | This bilingual, cross-platform acceptance contract and evidence record. | `src/m9-manual-acceptance.test.ts`, all required gates. | Sections 1–11 and issue comment. | None after all rows are recorded. |

<!-- m9-acceptance:acceptance-record -->
## 12. Acceptance record and cleanup

Record the reviewed revision, OS/architecture/display scale, Node/pnpm/Rust/Python versions, command outputs, focused test counts, full-gate conclusions, provider fixture hashes, Source/Canonical/Quality/Snapshot IDs, revision and deletion-lock evidence, redacted User IDs, route screenshots, keyboard/accessibility observations, and the exact CI run URLs/SHA/conclusions. Keep credentials, tokens, private paths, and private market data out of the record.

After fixture acceptance, delete only disposable profiles, temporary acquisition directories, generated package/build outputs, and test databases created for the run. Do not delete repository fixtures or finalized evidence. If a platform keeps a file handle open, stop the owning process before cleanup and record the platform result.

M9 is accepted only when every applicable row above passes, optional real-provider checks are either passed without secret evidence or marked unavailable with complete redacted evidence, all automated gates are green, required platform evidence is recorded, and no M9 boundary is violated. M10 Feature Engineering and all later Factor, Model, Strategy, Paper, Bot, Monitoring, and feedback milestones remain out of scope.
