# V1 Acceptance Evidence: #209

Observed 2026-09-06 against the local packaged Desktop build and the live
repository/release state. This is an evidence record, not a V1 readiness
assertion.

## Reviewed scope

| Field | Evidence |
| --- | --- |
| Repository | `tonywxx/adaq`, source/review commit [`226dcf1`](https://github.com/tonywxx/adaq/commit/226dcf1) on `main`; the pre-existing uncommitted `pnpm-lock.yaml` change was preserved and excluded |
| Desktop | macOS ARM64, local `src-tauri/target/debug/bundle/macos/adaq.app`, bundle ID `bid.adaq.desktop` |
| Locales | Full visible route pass in `en-US` and `zh-CN` across Operations, Paper Trading, Bots, Paper Feedback, and Settings/Connections |
| Provider boundary | OKX Demo only; no Live provider, real-money order, or new credential entry/rotation was used |
| Product actions | Read-only navigation and one confirmed account reconciliation attempt; no Bot start, retry, stop, flatten, or order action was issued |
| Evidence lineage | Market context `98a4621101b0019a65eb84aa9509ef0c75c199ce3b35c58767c43e255a21ef7c`; Bot `33927f58-1783-4724-b8cf-830dcd185545`; Bundle `84761179dc17774c51aff92e3a27c080217cec9af947618a96b2d8c75ef8d593`; Attempt `5618cc5a-e86b-463b-8cce-db35d210c20b` |

## Failure and recovery matrix

| Scenario | Commit / platform / locale | Expected fail-closed action | Observed action and retained artifact | Result |
| --- | --- | --- | --- | --- |
| Missing or stale market data | [`226dcf1`](https://github.com/tonywxx/adaq/commit/226dcf1) / macOS ARM64 / `en-US`, `zh-CN` | Keep the context `Unknown`, skip affected decisions, and retain redacted diagnostic evidence. | Operations showed `market_data_context` as critical/`Unknown`; the acknowledged alert recorded `Skip decision`. Evidence `0e6a973fe591d808270c4ec80018a5612cef798cb4dfd1565aa7ca59ee6e74e3` retained correlation `accept-final-warmup-003`, event IDs, owning route `/markets/crypto`, and diagnostic `No complete Feature Dataset cross-section is available.` Paper Feedback exposed unavailable metrics rather than fabricating them. | Demonstrated. See [#209](https://github.com/tonywxx/adaq/issues/209) for the English evidence record. |
| Provider disconnect and reconnect | [`226dcf1`](https://github.com/tonywxx/adaq/commit/226dcf1) / macOS ARM64 / `en-US`, `zh-CN` | Preserve provider uncertainty/account state and require successful reconciliation before relying on it or increasing risk. | Paper Trading retained `Uncertain` provider records and showed `Reconciliation Required`. A confirmed Reconcile contact to OKX Demo failed with `The operating-system secret store is unavailable`; the UI stated that retained evidence had not changed. Existing retained records include mismatch followed by reconciled outcomes. | Disconnect/fail-closed behavior demonstrated. A fresh successful reconnect was not demonstrated because the local secret-store failure prevented it. |
| Clock skew or invalid Decision Clock | [`226dcf1`](https://github.com/tonywxx/adaq/commit/226dcf1) / macOS ARM64 / `en-US`, `zh-CN` | Reject or skip the affected decision and retain the clock diagnostic; never emit a stale Target. | Provider evidence exposed `clockSkewSeconds: 5`, but this run did not create an explicit invalid-clock or over-tolerance Decision Clock attempt. | Not demonstrated; no readiness claim. |
| Worker crash, hang, deadline, or heartbeat loss | [`226dcf1`](https://github.com/tonywxx/adaq/commit/226dcf1) / macOS ARM64 / `en-US`, `zh-CN` | Supervisor must stop risk, retain the Runtime Attempt, and require recovery/reconciliation without replay. | Bot audit retained `running → faulted` with `host_restart`; the active Attempt was interrupted, the Bot required reconciliation, and seven `worker-heartbeat` entries remained inspectable. Operations retained worker-stopped evidence `d23d8299-4c53-4c14-9737-c994c355ce8d` with lifecycle `Active host → Resolved host`; no decision or order was listed. | Crash/restart/heartbeat behavior demonstrated. Hang and deadline injection were not separately demonstrated. |
| Uncertain order outcome and reconciliation recovery | [`226dcf1`](https://github.com/tonywxx/adaq/commit/226dcf1) / macOS ARM64 / `en-US`, `zh-CN` | Keep the outcome `Uncertain`, retain provider evidence, prevent duplicate submission, and require reconciliation. | Orders 1 and 2 retained `provider_timeout`/`unknown` evidence without provider order IDs. Paper Trading stated that uncertain evidence was retained; later reconciliation records include mismatch and reconciled states. No fill or duplicate order appeared. | Demonstrated in the observed state; current account still remained reconciliation-required after the failed fresh reconcile. |
| Credential rotation and secret redaction | [`226dcf1`](https://github.com/tonywxx/adaq/commit/226dcf1) / macOS ARM64 / `en-US`, `zh-CN` | Store credentials only in the OS secret store, never redisplay or log secret values, and require re-entry for rotation. | Connections showed only the redacted key suffix `•••• fcf7`; the saved credential was never redisplayed, rotation fields were empty, and Save/rotate remained disabled until re-entry. The same boundary and controls were visible in both locales. No secret was entered or rotated during this run. | Redaction and rotation boundary demonstrated. Successful rotation was not exercised. |
| Application restart with account/Bot reconciliation | [`226dcf1`](https://github.com/tonywxx/adaq/commit/226dcf1) / macOS ARM64 / `en-US`, `zh-CN` | Mark interrupted account/Bot state as requiring reconciliation and block new risk. | Bots retained `host-restart: Active Runtime Attempt was interrupted; reconciliation is required`. Paper Trading showed `Restart detected` and exposed Reconcile as the only recovery control; no new risk was enabled. | Demonstrated in both locales. |

For this read-only run, the visible evidence showed no Live authority, no new
order, no duplicate order, no stale Target replay, no fabricated feedback
metric, and no hidden success after the reconciliation failure. Because no new
risk or order action was issued, these are observations of the retained state,
not proof that every prohibited outcome is impossible. Raw provider IDs, error
codes, lifecycle transitions, and event IDs remained inspectable while
credential values stayed redacted.

## Product hardening and platform evidence

| Check | Observation | Limitation |
| --- | --- | --- |
| Operations/relevant workspace visibility | Operations, Paper Trading, Bots, Paper Feedback, and Connections exposed the health, alert, account, Bot, feedback, provider, and recovery states in `en-US` and `zh-CN`. | Raw domain identifiers and provider error names remain English by design for evidence inspection. |
| Accessibility | The macOS accessibility tree exposed named headings, alerts, links, buttons, fields, secure fields, and the locale selector, including recovery controls. | No automated WCAG audit was run. |
| Loading/error states | Route navigation painted immediately; Reconcile displayed a confirmation boundary and a visible typed failure while preserving state. | No formal performance-budget measurement was run. |
| Retention/diagnostics | Operations evidence, Bot lifecycle history, Paper provider records, reconciliation records, and Feedback report IDs remained visible after navigation and the failed Reconcile attempt. | No retention-pruning or long-duration diagnostic test was run. |
| Bilingual workflow | The complete inspected route workflow was repeated in Simplified Chinese after the English pass, including Settings/Connections and recovery surfaces. | This record is intentionally English for GitHub evidence; existing bilingual product documentation was not re-audited line by line. |
| Packaged release state | Local app bundle ID was `bid.adaq.desktop`; its `Info.plist` reported `0.9.6`. The GitHub `v0.9.6` release exposes macOS ARM64 and Windows x86_64 assets. | The local Settings page reported `0.9.5`, so this local target is not clean version evidence. No interactive Windows run was available, and no Linux asset is claimed. |

The claimed Windows platform remains automation/release evidence only. The
inspected V1 Acceptance run [34049757460](https://github.com/tonywxx/adaq/actions/runs/34049757460)
was not a green result, and the inspected Release run
[34049784530](https://github.com/tonywxx/adaq/actions/runs/34049784530) failed during
dependency installation with publication skipped. Historical green runs are
not substituted for current-head evidence.

The local retained-store IDs above are the exact artifact handles emitted by
`bid.adaq.desktop`; they do not have public URLs. The public links in this
record point to the source commit, issue evidence, workflow runs, and release
metadata, while the local app path and IDs preserve the precise offline
evidence boundary.

## Automated checks

| Command | Result |
| --- | --- |
| `pnpm run build` | Passed: TypeScript and Vite production build completed. |
| `pnpm run lint` | Passed: Biome checked 172 files. |
| `cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check` | Passed. |
| `cargo check --manifest-path src-tauri/Cargo.toml --workspace` | Passed with 24 existing warnings and 0 errors. |
| Focused Operations/Paper/Bots/Feedback/Connections/i18n Jest run | Passed: 5 suites, 22 tests. |
| Full frontend Jest run | Failed at the current baseline: 46/47 suites and 164/167 tests passed; the three `metric-catalog` pointer/focus/click tests each timed out at 5 seconds. Running that suite alone reproduced the same failure. |
| `cargo test --manifest-path src-tauri/Cargo.toml --workspace` | Incomplete: remained silent for about nine minutes and was interrupted with exit 130; not a pass. |

## Disposition

The failure-matrix evidence and bilingual Desktop route pass are recorded,
with the unproven clock-invalid, worker-hang/deadline, successful reconnect,
successful rotation, automated accessibility, performance-budget, and
interactive Windows limitations stated above. Closing #209 records completion
of this bounded acceptance exercise; it does not approve V1 readiness or
convert the failed/pending CI into a green result.
