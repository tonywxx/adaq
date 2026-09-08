# V1 Follow-up Evidence: #211

Observed 2026-09-07 against the current `origin/main`, a freshly packaged
Desktop build, the retained local evidence, and live GitHub Actions/release
state. This is a follow-up evidence record, not a V1 readiness assertion.

## Reviewed scope

| Field | Evidence |
| --- | --- |
| Repository | `tonywxx/adaq`, clean `main`, reviewed commit [`48cf110`](https://github.com/tonywxx/adaq/commit/48cf110489a0faae5f672e4d93a705e4c90f0ea8), equal to `origin/main` |
| Source delta since the prior product run | [`226dcf1`](https://github.com/tonywxx/adaq/commit/226dcf1) → `48cf110` contains the #209/#210 evidence documents and the `pnpm-lock.yaml` cleanup; no product source change occurred |
| Desktop package | Fresh local `src-tauri/target/debug/bundle/macos/adaq.app`, built from `48cf110` on 2026-09-07; arm64 Mach-O, version `0.9.7`, bundle ID `bid.adaq.desktop`, updater archive and signature emitted |
| Package identity | macOS app `CFBundleIdentifier=bid.adaq.desktop`, `CFBundleShortVersionString=0.9.7`, ad-hoc `codesign` CDHash `46b21eb5a5c49bfe8c70a3d5312562a5cc7f05b7`; updater archive SHA-256 `078a2bd8b01b89fb48d620977572d3ccfd28f61cc18a503337dc4aa4ec0d4d58` |
| Declared platforms | macOS ARM64 and Windows x86_64 remain the declared workflow/release set; no Linux readiness claim |
| Current workflow locales | `en-US` was inspected in the current packaged Desktop. A current-head `zh-CN` run was not obtained because no authenticated session was available without entering credentials; no credentials were entered or transmitted. The bilingual run in [#209](https://github.com/tonywxx/adaq/issues/209) remains historical evidence for `226dcf1`, not a current-head substitute. |
| Provider boundary | OKX Demo only; no Live provider, real-money action, new credential entry, Bot retry/start, flatten, or order action was issued |

## Current packaged Desktop evidence (`en-US`)

| Criterion | Observed retained evidence | Disposition |
| --- | --- | --- |
| Factor/Model/Strategy lineage | Bot `33927f58-1783-4724-b8cf-830dcd185545` references Bundle `84761179dc17774c51aff92e3a27c080217cec9af947618a96b2d8c75ef8d593`, Strategy Qualification `11529b4837415a230f19fd57c7633d5ff38ef1adf4940dfa8bf6db0b02041a01`, and Candidate `e3977b3b-af04-4ab1-8a62-c8981f5c60b9` revision 1. | Lineage identifiers are visible, but no qualifying decision sample is present. |
| Host-supervised Runtime Attempt | Current Bot is `Faulted · recovery required`; Attempt `5618cc5a-e86b-463b-8cce-db35d210c20b` shows `starting → reconciling → warmingUp → running → faulted` with `host_restart`. The Bot declares a `Closed Bar` Decision schedule. Seven worker-heartbeat records remain correlated; Decision batches and Paper orders are empty. | Fail-closed recovery is visible; no Decision Clock sample was emitted and no valid decision/order sample exists. |
| OKX Demo account | Account `723843360829982304`, currency `USDT`, observed 2026-09-05, remains `Reconciliation Required` after restart. The retained provider capability evidence is Demo with `read`, `trade`, and `simulated`; `clockSkewSeconds` is 5. | Account state is not currently usable for new risk. |
| Orders and fills | Four retained local orders (`order-1` through `order-4`) are `Cancelled` with `0 / 1`; the workspace states `No retained fills`. Two older retained provider acknowledgements have provider order IDs, while two records remain `Uncertain` with `provider_timeout` and no provider order ID. Older reconciliation records include both reconciled and mismatch outcomes; the account's latest state is still reconciliation-required. | No current acknowledged-and-filled order or fill identity is available; no order was issued to manufacture one. |
| Paper Feedback Snapshot | Snapshot `e8a36d32-602f-4e3d-9290-1feba8d9f970` binds Bundle `84761179dc17774c51aff92e3a27c080217cec9af947618a96b2d8c75ef8d593` to failed Attempt `5618cc5a-e86b-463b-8cce-db35d210c20b`; created 2026-09-05 for 2026-09-04 observations. The UI keeps the sample gate at `20` required observations. | Snapshot is retained and identity-bound, but has 0 realized observations. |
| Lens Reports | Execution `643974c4-b62c-4a82-b12f-1427be36f126`: 0 acknowledged orders, 0 fills, 0 provider evidence rows, unavailable rates/slippage. Strategy `59585375-1bc4-4bf7-a729-4445853bf817`: no retained account valuation series and 0 target samples. Model `0fb2d550-83d1-4712-9bdf-3790d1785628`: `no-compatible-model-target-pairs`, 0 realized rows. Factor `3fd898f6-2b1e-4bcf-885b-7a03e2d6dcaa`: `no-compatible-factor-output-pairs`, 0 realized rows. | All four reports remain explicitly non-ready; no directional conclusion is available. |
| Research Review Decision | The retained append-only Review Decision `096f9c18-c669-4f9e-8d51-51f790931639` (also retained in [#207](https://github.com/tonywxx/adaq/issues/207)) is `Pause or Stop Bot` and keeps the Bundle unchanged while requiring the next qualifying current Strategy/Runtime sample. The four Reports remain authoritative and no automatic retraining, challenger switch, or hot patch is allowed. | Human review is retained, but it cannot turn the absent realized sample into readiness. |
| Operations | Current Operations shows one unresolved critical condition: `market_data_context` is `Unknown`/acknowledged with `Skip decision`; Paper account and Risk/OMS dimensions are `Unknown`; four Research Review Required warnings remain active. | Safety and evidence visibility are demonstrated; readiness prerequisites remain unavailable. |

## Automation, CI, and release evidence

| Check | Result |
| --- | --- |
| `pnpm install --frozen-lockfile` | Passed on `48cf110`. |
| `pnpm run build` | Passed: TypeScript and Vite production build completed. |
| `pnpm run lint` | Passed: Biome checked 172 files. |
| `cargo check --manifest-path src-tauri/Cargo.toml --workspace` | Passed with 0 errors and 24 existing warnings. |
| `pnpm tauri build --debug --bundles app` | Passed: current-head macOS app, updater archive, and signature were emitted. This is local ad-hoc debug evidence, not a signed release. |
| `pnpm exec jest --watchman=false --runInBand` | Incomplete locally: remained silent for more than 9 minutes and was interrupted with exit 130. The current-head CI result is recorded below; the focused 4-suite run passed 17/17 tests. |
| `cargo test --manifest-path src-tauri/Cargo.toml --workspace` | Incomplete locally: remained silent for about 9 minutes and was interrupted with exit 130. The current-head Windows Acceptance job completed its workspace Rust tests successfully. |
| Current V1 Acceptance | [Run 34064148773](https://github.com/tonywxx/adaq/actions/runs/34064148773) completed on `48cf110` with Windows x86_64 passed and macOS ARM64 failed in the existing `metric-catalog` frontend suite: 46/47 suites and 164/167 tests passed; pointer, focus, and click cases each exceeded 5 seconds. |
| Current release | Tag `v0.9.7` points to `2e72ca5`, but no published `v0.9.7` release exists. The latest published release is `v0.9.6`; no current-head signed Windows package or updater manifest is available. |

## Decision

**Not Ready, with concrete blockers.** The required current-head Bot decision,
acknowledged order, fill, post-order reconciliation, and matured observation
sample is still absent. The current account remains reconciliation-required;
the current-head `zh-CN` workflow was not re-run; and the current-head CI/release
set is not green or published. These limitations are recorded rather than
weakened, backfilled, or replaced with fixture evidence.

No Live authority, real-money action, secret value, duplicate order, stale
Target replay, fabricated metric, automatic retraining, challenger switch, or
hot patch was used. The next valid readiness attempt requires a human-approved,
bounded OKX Demo run that produces a reconciled current Attempt, real provider
acknowledgement and Fill evidence, a matured horizon, both locale outcomes on
the exact reviewed commit, and green current-head package/release evidence.
