# M10 Manual Acceptance

This is the canonical human-reviewed M10 path. The reviewed local run is macOS ARM64; the command substitutions for Windows x86_64 and Linux x86_64 are recorded below. Perform one row at a time and retain the requested evidence on failure. M10 ends at finalized immutable Feature Datasets and the equivalent Feature Engine; it does not deliver Factor research (M11), Model training, Strategies, Paper, Bots, or anything later.

Never put credentials, authorization headers, OTPs, tokens, private paths, or private market data in issue comments, commits, screenshots, logs, exports, or this record. Optional real-provider checks are permitted only with maintainer-owned credentials entered in **Settings → Connections**; committed fixtures and local mock servers are the authoritative acceptance path.

<!-- m10-acceptance:scope -->
## 1. Scope and prerequisites

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| Run `node --version` from the repository root. | Node.js 24 or later is selected, matching the release baseline; the reviewed local run uses Node v26.7.0. | Complete output and installation method. |
| Run `pnpm --version`. | pnpm 11.20.0 is available, matching `packageManager` in `package.json`. | Complete output and installation method. |
| Run `pnpm install --frozen-lockfile`. | Dependencies match `pnpm-lock.yaml`. | Complete output and the two tool versions. |
| Run `rustup toolchain install stable` and `rustup show`. | The stable Rust toolchain is available for the feature engine workspace. | Complete output and installed-target listing. |
| Run `pnpm tauri dev` with Supabase variables supplied outside version control. | The desktop shell opens without exposing configuration values. | Screenshot and redacted error only. |
| Open a fresh device profile and select **Settings → General**. | System, English (US), and 简体中文 are the only locale choices; no Feature evidence exists yet. | Screenshot, platform, and locale state. |

Use `shasum -a 256 <path>` on macOS, `Get-FileHash -Algorithm SHA256 <path>` in Windows PowerShell, and `sha256sum <path>` on Linux. Native file pickers, data-folder paths, display scaling, and secret-store prompts remain platform-specific.

<!-- m10-acceptance:definitions -->
## 2. Feature Definition lifecycle

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| Run `cd src-tauri && cargo test -p adaq features`. | Definition lifecycle, preview, fitting, materialization, runner, cancellation, restart, deletion-lock, and reset tests in `src-tauri/src/features/tests.rs` pass. | Revision, complete output, and failing test. |
| Open `/features` and create a Draft Definition in the ordered node editor. | Draft creation is keyboard-operable: node add/remove/reorder, parameter editing, and output selection all work without a pointer. | Route, focused control, screenshot, and accessibility-tree text. |
| Select **Validate** on a draft with a typed defect (bad scope, cycle, or untyped signal provenance). | Validation fails with a typed error naming the defect; no evidence identity is created. | Typed error, draft state, and screenshot. |
| Publish a valid draft, then publish a changed revision of the same Definition family. | Publication is an immutable revision chain: the stable `definitionId` persists, the JCS SHA-256 `definitionHash` changes, and the revision must increase. | Definition ID, before/after revisions, hashes, and screenshot. |
| Run a bounded Preview for a published Definition. | Preview fits nothing, creates no evidence identity, and is transient bounded output only. | Preview result, absence of Dataset/Attempt IDs, and screenshot. |
| Switch `/features` between **English (US)** and **简体中文**. | Every Definition control, state, and error is localized in en-US and zh-CN. | Locale, missing key/label, and screenshot. |
| Run `pnpm exec jest --watchman=false --runInBand src/features/features/features-data.test.ts`. | Frontend Definition/adapter data contract tests pass. | Revision, suite/test, and complete output. |

<!-- m10-acceptance:fitting -->
## 3. Fitting Protocols and Artifacts

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| Start a Fitting Protocol from `/features` against a Completed Dataset. | The Fitting Attempt starts, publishes an immutable Fitted Transformation Artifact, and duplicate requests coalesce onto the same evidence. | Attempt ID, Artifact ID, protocol identity, and screenshot. |
| Run `cd src-tauri && cargo test -p adaq-feature-engine --test fitting`. | Engine fitting tests pass, including `standardization_uses_population_variance_and_excludes_future_available_samples` and `per_instrument_parameters_are_exact_and_walk_forward_rejects_future_artifacts`. | Revision, complete output, and failing test. |
| Run `cd src-tauri && cargo test -p adaq features fitting_publishes_an_artifact_and_coalesces_duplicates`. | The app-level lifecycle publishes an Artifact and coalesces duplicate fitting requests. | Revision, complete output, and failing test. |
| Inspect a walk-forward fold after a later fold's Artifact exists. | Fold isolation holds: the fold never observes a future Artifact, and per-instrument parameters remain exact. | Fold ID, Artifact IDs, parameter evidence, and screenshot. |
| Submit a Fitting Protocol with insufficient samples. | The Attempt fails with a typed insufficient-sample error; no Artifact is published. | Attempt ID, typed error, and absence of Artifact ID. |
| Retry a failed Fitting Attempt. | Retry preserves the original source evidence and produces a new Attempt identity. | Old/new Attempt IDs, retained source evidence, and error. |
| Cancel a running Fitting Attempt. | Cancellation reaches the running attempt before terminal evidence is written. | Attempt ID, cancellation state, and screenshot. |

<!-- m10-acceptance:materialization -->
## 4. Dataset materialization and attempts

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| Run `cd src-tauri && cargo test -p adaq-feature-engine --test materialization`. | Engine materialization tests pass, including `materialization_publishes_immutable_wide_parquet_and_completed_metadata`, `staging_claim_allows_only_one_concurrent_writer`, and `startup_marks_running_interrupted_and_removes_only_its_staging_file`. | Revision, complete output, and failing test. |
| Submit a Materialization Request for a frozen Plan. | Publication is atomic: the Completed Dataset appears with its metadata only after staging succeeds; no partial Dataset is consumable. | Attempt ID, Dataset ID, manifest hash, and screenshot. |
| Interrupt a running Materialization Attempt and restart the app. | Interruption recovery deletes only that attempt's own staging file; other users' and attempts' staging remain untouched. | Attempt IDs, staging paths, recovery diagnostic, and platform. |
| Restart the app with a pending and a running Materialization Attempt. | The pending Attempt survives the restart; the running Attempt recovers to failed with retained source evidence. | Attempt IDs, before/after states, and retained evidence. |
| Attempt to delete an Artifact or Dataset referenced by a later Plan or Attempt. | The deletion lock rejects the operation with a typed reference error naming the dependent. | Record ID, dependent ID, and typed error. |
| Present an incompatible legacy Feature schema or pre-v1 evidence at startup. | The engine requires an explicit reset and never silently deletes or migrates prior evidence. | Typed reset-required error, reset state, and screenshot. |
| Run `cd src-tauri && cargo test -p adaq features pending_attempts_survive_restart_and_running_recovers_to_failed`. | The app-level restart recovery contract passes. | Revision, complete output, and failing test. |

<!-- m10-acceptance:datasets -->
## 5. Dataset inspection

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| Open a Completed Dataset in `/features` and inspect its Manifest. | The Manifest records provenance: Plan identity, Definition revisions, Artifact references, Snapshot, Universe, Observation Range, Parameters, and Seed. | Dataset ID, missing manifest field, and screenshot. |
| Inspect per-output coverage for the same Dataset. | Every output reports its own coverage, and Unavailability reason counts use the stable reason vocabulary (`warmup`, `bar-gap`, `missing-market-input`, `missing-dependency`, `unknown-universe`, `insufficient-coverage`, `undefined-arithmetic`, `artifact-missing-instrument`, `corporate-action-unavailable`). | Output name, coverage numbers, reason counts, and screenshot. |
| Inspect the numeric summary of one output. | Min, max, mean, and population standard deviation are reported for available observations only. | Output name, summary values, and screenshot. |
| Apply a filter and page through Dataset rows. | Row inspection is bounded: at most 50 rows per page, pagination is disabled correctly at both ends, and filters never widen the bounded window. | Filter, page index, row count, and screenshot. |
| Submit an identical Materialization Request again. | Completed evidence is reused: dedup returns the existing Dataset identity instead of re-materializing. | Dataset ID, dedup result, and screenshot. |
| Corrupt the content hash of a Dataset file out of band, then inspect it. | Content-hash corruption is rejected and the Dataset is not consumable. | Dataset ID, typed corruption error, and redacted path. |
| Run `cd src-tauri && cargo test -p adaq features materialization_completes_a_dataset_and_reuses_completed_evidence`. | The app-level completion and dedup contract passes. | Revision, complete output, and failing test. |

<!-- m10-acceptance:okx-journey -->
## 6. OKX Spot journey

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| Run `cd src-tauri && cargo test -p adaq-feature-engine --test reference_fixtures`. | The committed `feature-reference-vectors.json` journeys pass, including the OKX Spot journey. | Revision, complete output, and failing journey. |
| Materialize Return, RSI, Realized Volatility, and Bar Gap outputs from an exact M9 OKX Snapshot. | Each output materializes from the immutable Snapshot with causal Availability; Bar Gap observations are typed Unavailable, never filled. | Dataset ID, output names, Availability evidence, and screenshot. |
| Split the same Observation Range into different chunk partitions and re-evaluate. | Chunk equivalence holds: batch results are bit-identical across chunk boundaries, gaps, and restart reconstruction (`restart_replay_and_chunk_partitions_are_bit_identical_across_gaps_dependencies_and_calendar`). | Partition scheme, digest equality, and error. |
| Compare the quantized journey summary against the committed reference vectors. | Cross-platform quantized summaries match, so the journey is deterministic across macOS ARM64, Windows x86_64, and Linux x86_64. | Digest values, platform, and mismatch. |

<!-- m10-acceptance:a-share-journey -->
## 7. China A-share journey

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| Run `cd src-tauri && cargo test -p adaq-feature-engine --test reference_fixtures` and locate the China A-share journey. | The A-share journey passes against the committed reference vectors. | Revision, complete output, and failing journey. |
| Materialize Venue Calendar features from an exact M9 A-share Snapshot. | Calendar features use venue-local Asia/Shanghai time and exclude scheduled breaks (`calendar_features_use_venue_local_time_and_exclude_breaks`). | Dataset ID, venue, calendar evidence, and screenshot. |
| Materialize Session Progress across the morning session, midday break, and afternoon session. | Midday break is excluded from progress; scheduled closures are never counted (`calendar_closures_are_excluded_from_session_progress`). | Output name, progress values, break evidence, and screenshot. |
| Materialize Split and Dividend features around a corporate action. | Split/Dividend features are forward-looking and causally available at their recorded effective evidence, never backward-adjusted (`split_and_dividend_features_are_forward_and_causally_available`, `ashare_corporate_actions_retain_instrument_and_evidence_identity`). | Action evidence ID, Available At, and screenshot. |
| Inspect `PriceBasis` for every materialized input. | All inputs remain Unadjusted; no backward adjustment is applied anywhere. | Series ID, basis, and error. |

<!-- m10-acceptance:us-equity-journey -->
## 8. U.S. equity journey

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| Run `cd src-tauri && cargo test -p adaq-feature-engine --test reference_fixtures` and locate the U.S. equity journey. | The U.S. equity journey passes against the committed reference vectors. | Revision, complete output, and failing journey. |
| Materialize a Cross-Sectional plan against a Point-in-Time Instrument Universe from an exact M9 Snapshot. | The plan binds one Venue, Asset Class, Bar Interval, Price Basis, and Valuation Currency; Universe membership is Point-in-Time (`cross_sectional_unknown_universe_is_complete_and_mixed_markets_are_rejected`). | Dataset ID, Universe ID, membership evidence, and screenshot. |
| Materialize Cross-Sectional Rank outputs. | Rank/percentile/z-score outputs are deterministic and input-order independent (`cross_sectional_rank_percentile_and_zscore_are_deterministic`). | Output name, digest, and error. |
| Inspect coverage when a Universe member lacks observations. | Coverage preserves missing members and records actual coverage instead of inventing values (`cross_sectional_coverage_preserves_missing_members_and_actual_coverage`). | Member ID, coverage, and screenshot. |
| Materialize with a Reconstructed or Unknown Universe state. | Reconstructed evidence retains its exact state; Unknown makes the complete batch Unavailable rather than partial. | Universe state, Unavailability scope, and screenshot. |

<!-- m10-acceptance:semantics -->
## 9. Semantic proofs

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| Run `cd src-tauri && cargo test -p adaq-feature-engine --test operators decimal_projection_and_backward_returns_are_causal`. | Causality holds: a bar close is both Observation Time and Available At; backward returns never use future information. | Revision, complete output, and failing test. |
| Run `cd src-tauri && cargo test -p adaq-feature-engine --test operators rolling_variants_and_realized_volatility_use_full_windows`. | Warmup is full-window: rolling outputs are Unavailable until the complete window is observed. | Complete output and failing test. |
| Run `cd src-tauri && cargo test -p adaq-feature-engine --test operators rolling_state_resets_on_gaps_but_not_scheduled_closures`. | Analytical state resets at genuine Bar Gaps but not at scheduled closures. | Complete output and failing test. |
| Run `cd src-tauri && cargo test -p adaq-feature-engine --test operators cross_sectional_coverage_preserves_missing_members_and_actual_coverage`. | Cross-Sectional evaluation uses the complete Universe with explicit coverage. | Complete output and failing test. |
| Run `cd src-tauri && cargo test -p adaq-feature-engine --test fitting standardization_uses_population_variance_and_excludes_future_available_samples`. | Fitted folds exclude future-available samples; no fold consumes a later Artifact. | Complete output and failing test. |
| Run `cd src-tauri && cargo test -p adaq-feature-engine --test operators future_return_direction_is_rejected_at_definition_freeze`. | Future-return Features are rejected at Definition freeze; no future return usage reaches evaluation. | Complete output and failing test. |
| Inspect any materialized input series. | No backward adjustment exists (`PriceBasis` Unadjusted everywhere), and no Feature operation mutates Canonical Market Data. | Series ID, basis, canonical hash before/after, and error. |

<!-- m10-acceptance:isolation -->
## 10. User isolation and evidence boundaries

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| Run `cd src-tauri && cargo test -p adaq features definitions_are_user_scoped_and_presentation_never_changes_the_hash`. | User-scoped records never cross Users; presentation metadata never changes the semantic hash. | Revision, complete output, and failing test. |
| Run `cd src-tauri && cargo test -p adaq features deletion_checks_references_and_dedup_grants_no_cross_user_visibility`. | Content dedup reuses identical evidence without granting cross-user visibility. | Complete output and failing test. |
| Sign out and sign in as a second test User, then open `/features`. | The second User sees no Definitions, Attempts, Artifacts, or Datasets of the first User. | Two redacted User IDs, list states, and screenshot. |
| Trigger a Materialization Attempt and interrupt it mid-run. | Atomic publication leaves no consumable partial Dataset; interruption evidence is retained. | Attempt ID, staging state, and diagnostic. |
| Submit an input that produces a non-finite or wrong-shape result. | The engine reports a typed fatal evaluation error with Stage, Node, Instrument, Observation Time, and safe diagnostics — distinct from expected typed Unavailable. | Typed error class, stage/node identity, and diagnostics. |
| Page through Dataset rows as the owning User. | Row inspection stays bounded and never exposes another User's evidence. | Page index, row count, and screenshot. |

<!-- m10-acceptance:features-gui -->
## 11. `/features` workspace GUI

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| Open `/features` directly. | The route paints immediately; there is no page-level skeleton gate before content. | Route, first visible frame, and console output. |
| Open each tab: Definitions, Fitting, Materialization, Datasets. | Each owning control manages its own loading, error, and empty state; loading feedback appears at the data boundary. | Tab, loading owner, screenshot, and accessibility-tree text. |
| Navigate away and return to `/features`. | Read-only list metadata can paint from current-session cache while the owning control refreshes in the background. | Route, loading owner, cache state, and timing. |
| Operate the Definition editor, Attempt lists, and Dataset inspection with the keyboard only, with a screen reader enabled. | Every control is focusable, labeled, and operable without a pointer. | Focused control, announced name, and screenshot. |
| Inspect status indicators for color-only meaning. | No state depends on color alone; text/labels accompany every status. | Control, state, and screenshot. |
| Set the content area to 1024 px and repeat the tab tour. | Layout remains usable: no clipped controls, hidden actions, or broken pagination. | Platform scale, tab, screenshot, and accessibility-tree text. |
| Run `pnpm exec jest --watchman=false --runInBand src/loading-boundaries.test.ts src/lib/i18n.test.ts src/router.test.ts`. | Loading boundaries, locale coverage, and the `/features` route contract pass. | Revision, suite/test, and complete output. |
| Inspect `/features` and the shell for Factor, Model-training, Paper, Bot, or Live controls. | No such out-of-scope control exists in M10. | Route and screenshot if any control appears. |

<!-- m10-acceptance:performance-baselines -->
## 12. Performance baselines

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| Run `cd src-tauri && cargo test -p adaq-feature-engine --release --test benchmarks -- --ignored --test-threads=1`. | Both canonical workloads complete serially: the 1,000,000-Bar Time-Series workload and the 10,000-Instrument × 252-Observation Cross-Sectional workload. The final reviewed run completed 2 workloads with 0 failures in 164.27 s. | Revision, complete output, workload, and platform. |
| Compare results with `src-tauri/crates/adaq-feature-engine/fixtures/feature-benchmark-baseline.json`. | The baseline uses schema `adaq-feature-benchmark-baseline@1.0.0`, recorded on macOS ARM64 (`aarch64-apple-darwin`), with recorded values: Time-Series 20155 ms, Cross-Sectional 64828 ms, and peak RSS 439,386,112 bytes (the process high-water mark, `ru_maxrss`). The baseline is record-only: no invented latency or RSS targets are asserted. The reviewed run measured at or better than these recorded values, and the baseline file regenerated with no diff (`git diff --exit-code` clean). | Baseline file hash, measured values, and platform difference. |
| Observe the GUI while a long Materialization or Fitting Attempt runs. | The GUI never freezes: the heavy work runs in a supervised worker while the UI stays responsive. | Attempt ID, UI responsiveness evidence, and screenshot. |
| Page through a large Completed Dataset during and after the run. | Dataset pagination stays bounded at 50 rows per page under load. | Page index, row count, and screenshot. |

<!-- m10-acceptance:regressions -->
## 13. Regressions and boundary checks

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| Run `pnpm exec jest --watchman=false --runInBand`. | All frontend suites pass, including the M7/M8/M9 contract tests, locale, route, loading, and feature data tests. | Revision, suite/test, and complete output. |
| Run `cd src-tauri && cargo test --workspace`. | The full Rust workspace passes; M5–M9 journeys remain canonical and unchanged. | Revision, complete output, and failing test. |
| Open [`docs/m7-manual-acceptance.md`](m7-manual-acceptance.md), [`docs/m8-manual-acceptance.md`](m8-manual-acceptance.md), and [`docs/m9-manual-acceptance.md`](m9-manual-acceptance.md). | Existing Components, Backtests, Validation, Model Dataset, Forecast Evaluation, and Markets paths remain the canonical regression paths. | Guide section and broken/changed path. |
| Inspect the shell and all routes for Factor research, Model training, Strategy, Paper, Bot, or Marketplace capabilities. | M11+ capabilities are absent; M10 adds none. | Route and screenshot if any capability appears. |

<!-- m10-acceptance:automated-gates -->
## 14. Automated release gates and CI

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| Run `cd src-tauri && cargo fmt --all --check`. | Rust formatting passes. The reviewed run passed with no diff. | Revision and complete diff. |
| Run `cd src-tauri && cargo test --workspace`. | All Rust workspace tests and doctests pass; ignored long-running benchmarks are recorded as ignored when applicable. The reviewed run passed: 307 passed / 0 failed / 4 ignored across 24 test binaries (the ignored set is the two `--ignored` benchmark workloads plus two generator-gated tests). | Revision, complete unfiltered output, failing test, and platform. |
| Run `cd src-tauri && cargo check --workspace`. | The native workspace type-checks. The reviewed run passed. | Revision and complete output. |
| Run `pnpm exec jest --watchman=false --runInBand`. | All frontend tests pass. The reviewed run passed 23 suites and 92 tests with no failures. | Revision, suite/test, and complete output. |
| Run `pnpm run build`. | Strict TypeScript checking and the Vite production build pass. The reviewed run passed. | Revision and complete output. |
| Run `pnpm run lint`. | Lint passes; any pre-existing warnings are listed separately from new findings. The reviewed run passed with 12 pre-existing warnings, none in the four M10 acceptance files. | Revision, file/rule, and complete output. |
| Run `git diff --check`. | No whitespace errors exist. The reviewed run was clean. | Revision and complete output. |
| Run `gh workflow run "Indicator engine acceptance" --ref <reviewed-ref>`. | The three-platform native matrix starts for the reviewed ref and exposes separate macOS ARM64, Windows x86_64, and Linux x86_64 jobs. | Workflow URL, SHA, job URLs, conclusion, and failed log excerpt. |
| Search the repository for a configured secret scanner. | No secret-scan command is configured in this checkout; manually verify the diff contains no credential material or token-like fixture values. The reviewed diff was manually checked and contained no credential or token material. | Command/output and reviewed file list. |
| Record the applicable GitHub Actions run URLs for `macOS ARM64`, `Windows x86_64`, and `Linux x86_64`. | Native fixture/Rust gates and release packaging evidence are retained for the reviewed revision or explicitly identified platform baseline. | Run URL, SHA, job, conclusion, and failed log excerpt. |

The native matrix is defined in [`.github/workflows/indicator-engine.yml`](../.github/workflows/indicator-engine.yml), a manually dispatchable three-platform matrix covering macOS ARM64, Windows x86_64, and Linux x86_64; release packaging is defined in [`.github/workflows/release.yml`](../.github/workflows/release.yml). A local pass never replaces required platform evidence. The acceptance record must distinguish a reviewed M10 revision from an older platform baseline.

Recorded platform evidence for the reviewed native/fixture and packaging paths:

| Workflow evidence | Revision | Jobs | Result |
| --- | --- | --- | --- |
| [Indicator engine acceptance run 31561728146](https://github.com/tonywxx/adaq/actions/runs/31561728146) | `f8a7053328da1f0de22f702d248d16043e84ea94` | [macOS ARM64](https://github.com/tonywxx/adaq/actions/runs/31561728146/job/94005262752), [Windows x86_64](https://github.com/tonywxx/adaq/actions/runs/31561728146/job/94005262760), [Linux x86_64](https://github.com/tonywxx/adaq/actions/runs/31561728146/job/94005262757) | Success (all three) |
| [Indicator engine acceptance run 31062405209](https://github.com/tonywxx/adaq/actions/runs/31062405209) | `34d63ab1688b7dbeff8f5cd394a848895381ec08` | macOS ARM64, Windows x86_64 | Success |
| [Release run 31282997179](https://github.com/tonywxx/adaq/actions/runs/31282997179) | `5d1d236999984ef4a8bcc646b8e927e37e9fb708` | Validate release, macOS ARM64, Windows x86_64, publish | Success |

The #87 implementation changes the bilingual acceptance documentation, README/roadmap, frontend acceptance-contract tests, Linux prerequisite installation in the workflow, and cross-platform test/resource cleanup. The current reviewed native/workflow revision is `f8a7053328da1f0de22f702d248d16043e84ea94`, and its indicator-engine matrix succeeded on macOS ARM64, Windows x86_64, and Linux x86_64. The release run remains the packaging baseline for the unchanged release path. Any final documentation, README, or frontend contract-test commit after this run does not alter the native/workflow inputs; its frontend gates are recorded locally.

<!-- m10-acceptance:acceptance-matrix -->
## 15. Final acceptance matrix

The matrix is evidence, not a substitute for a closed issue. Every row names the implementation boundary, a focused check, and the manual section that a reviewer can repeat.

### Parent #77 criteria

| ID | Requirement | Implementation / focused evidence | Manual / broad evidence | Unresolved risk |
| --- | --- | --- | --- | --- |
| P1 | Ten native sub-issues implement the dependency-ordered M10 slices and retain independent evidence. | Slice matrix below; `adaq-feature-engine/tests/contracts.rs`, `operators.rs`, `fitting.rs`, `materialization.rs`. | Sections 2–12 and the slice rows. | None after every slice row is recorded. |
| P2 | `adaq-feature-engine` owns Definition, Plan 2.0, operator, fitting, availability, missingness, evaluation, and identity contracts; the Indicator Engine remains a subengine. | `adaq-feature-engine/tests/contracts.rs` (`definition_and_plan_identities_are_canonical_and_replayable`, `plan_rejects_untyped_signal_provenance`); `operators.rs` (`indicator_nodes_use_the_pinned_indicator_engine_and_validate_output`). | Sections 2, 3, 9. | None. |
| P3 | Pointwise, Time-Series, and Cross-Sectional Features are causal, scope-correct, finite or typed Unavailable, deterministic across chunking, and cannot mutate Canonical Market Data. | `operators.rs` (`dependency_slots_share_batch_and_stateful_evaluation`, `restart_replay_and_chunk_partitions_are_bit_identical_across_gaps_dependencies_and_calendar`, `pointwise_encoding_and_checked_division_are_typed`). | Sections 6–9. | None for committed fixtures. |
| P4 | Fitting Protocols and Attempts publish immutable Artifacts without leakage; materialization applies but never refits them. | `fitting.rs` (`lifecycle_coalesces_reuses_retries_and_keeps_artifacts_user_scoped_and_locked`, `feature_evaluator_applies_bound_artifact_without_fitting_or_mutating_it`). | Sections 3, 9. | None. |
| P5 | Completed Feature Datasets are content-addressed immutable Parquet evidence with SQLite metadata, atomic publication, recovery, User isolation, and deletion locks. | `materialization.rs` (`materialization_publishes_immutable_wide_parquet_and_completed_metadata`, `content_hash_corruption_is_not_consumable_and_dataset_references_lock_deletion`); `src-tauri/src/features/tests.rs` (`artifact_deletion_is_locked_by_typed_references`). | Sections 4, 5, 10. | None. |
| P6 | Batch and stateful observation evaluation are equivalent under chunk boundaries, gaps, missing dependencies, and restart reconstruction. | `operators.rs` (`batch_and_stateful_observation_paths_are_identical`); `materialization.rs` (`stage_events_uses_the_same_evaluator_as_stateful_observation`); `fitting.rs` (`bound_artifact_evaluation_is_identical_across_batch_stateful_and_replay_paths`). | Sections 6, 9. | None. |
| P7 | `/features` paints immediately and exposes accessible localized Definition, fitting, materialization, preview, and inspection workflows. | `src/loading-boundaries.test.ts`, `src/lib/i18n.test.ts`, `src/router.test.ts`, `src/features/features/features-data.test.ts`. | Sections 2, 11. | OS assistive-technology differences remain recorded per platform. |
| P8 | OKX Spot, China A-share, and U.S. equity reference journeys and all declared failure paths pass. | `adaq-feature-engine/tests/reference_fixtures.rs` (`committed_reference_vectors_match_the_three_market_journeys`); `fixtures/feature-reference-vectors.json`; `reference_fixtures.rs` failures journey. | Sections 6–8. | None for committed vectors. |
| P9 | M11 can select only Completed Feature Datasets; M10 adds no Factor research, Model training, Paper order, Bot, Marketplace, script engine, or Feature Component ABI. | Routes/DTOs expose Feature evidence only; Section 11 and scope statement. | Sections 1, 11, 13. | None. |
| P10 | English and Simplified Chinese architecture and final manual-acceptance documents are semantically equivalent. | `docs/m10-manual-acceptance.md`, `.zh-CN.md`, `src/m10-manual-acceptance.test.ts`; `docs/m10-feature-engineering.md`, `.zh-CN.md`. | Sections 1–15. | None after parity test. |
| P11 | Every criterion is mapped to implementation plus focused and broad evidence; issue closure alone is never evidence. | This section plus the child matrix below. | Sections 1–14. | None after review. |
| P12 | Rust formatting/tests/checks, frontend Jest/build/lint, diff checks, accessibility review, and supported-platform CI pass for the final revision. | Gate table above; `src/loading-boundaries.test.ts` for accessibility-adjacent state. | Sections 11, 14. | None after the local gates and the current-SHA three-platform matrix. |
| P13 | An English completion comment records implementation, exact commands, results, revision, and CI links before the parent is closed. | Issue #87 completion comment references final revision and commands. | Acceptance record below. | Parent closure is a separate explicit action requested by the maintainer. |

### M10.1–M10.10 slice matrix

| Slice | Delivered boundary | Focused evidence | Final acceptance evidence | Unresolved risk |
| --- | --- | --- | --- | --- |
| M10.1 / #78 | Feature Engine contracts and Feature Plan 2.0: canonical identities, resource limits, reset-required legacy rejection. | `adaq-feature-engine/tests/contracts.rs`. | Section 2 and Rust gates. | None. |
| M10.2 / #79 | Pointwise and Time-Series Feature operators with causal Availability, full-window Warmup, and gap reset. | `adaq-feature-engine/tests/operators.rs`. | Sections 6, 9 and Rust gates. | None for committed fixtures. |
| M10.3 / #80 | Cross-Sectional Feature scope and Universe operators with coverage and determinism. | `adaq-feature-engine/tests/operators.rs` (`cross_sectional_*`). | Section 8 and Rust gates. | None. |
| M10.4 / #81 | Fitted Transformation Protocols and Artifacts with walk-forward fold isolation. | `adaq-feature-engine/tests/fitting.rs`. | Section 3 and Rust gates. | None. |
| M10.5 / #82 | Immutable Feature Dataset materialization and retained Attempts with recovery and deletion locks. | `adaq-feature-engine/tests/materialization.rs`. | Sections 4, 5 and Rust gates. | None. |
| M10.6 / #83 | Batch/observation equivalence and Component consumers under one evaluator. | `operators.rs` (`batch_and_stateful_observation_paths_are_identical`); `reference_fixtures.rs`. | Sections 6, 9 and Rust gates. | None. |
| M10.7 / #84 | User-scoped Feature APIs and the FIFO background runner with cancellation and restart recovery. | `src-tauri/src/features/tests.rs`; `src/tauri-command-scheduling.test.ts`. | Sections 3, 4, 10 and both gate suites. | None. |
| M10.8 / #85 | Localized `/features` workspace: Definitions, Fitting, Materialization, Datasets, Preview. | `src/features/features/features-data.test.ts`; `src/lib/i18n.test.ts`; `src/loading-boundaries.test.ts`. | Sections 2, 11 and full Jest/build gates. | OS visual differences remain manual evidence. |
| M10.9 / #86 | Three-market fixtures, benchmarks, and hardening. | `adaq-feature-engine/tests/reference_fixtures.rs`, `benchmarks.rs`; `fixtures/feature-reference-vectors.json`; `fixtures/feature-benchmark-baseline.json`. | Sections 6–8, 12 and Rust gates. | Benchmark values are record-only platform evidence. |
| M10.10 / #87 | This bilingual, cross-platform acceptance contract and evidence record. | `src/m10-manual-acceptance.test.ts`, all required gates. | Sections 1–15 and issue comment. | None after the current-SHA three-platform matrix and all required gates. |

<!-- m10-acceptance:acceptance-record -->
## 16. Acceptance record and cleanup

Record the reviewed revision, OS/architecture/display scale, Node/pnpm/Rust versions, command outputs, focused test counts, full-gate conclusions, Dataset/Artifact/Attempt IDs, content-hash and reference-vector digests, revision and deletion-lock evidence, redacted User IDs, route screenshots, keyboard/accessibility observations, and the exact CI run URLs/SHA/conclusions. The current #87 record includes native/workflow revision `f8a7053328da1f0de22f702d248d16043e84ea94`, run [31561728146](https://github.com/tonywxx/adaq/actions/runs/31561728146), and successful macOS ARM64, Windows x86_64, and Linux x86_64 jobs. The final documentation, README, and frontend contract-test commit does not alter those native/workflow inputs; its frontend gates are recorded locally. Keep credentials, tokens, private paths, and private market data out of the record.

Executed local GUI evidence: the signed-in fixture session opened `/features` at 1024×768 in the desktop browser shell. In both en-US and zh-CN, the route exposed a `Features` heading, four labeled tabs, a focusable tab panel, and owning-control `Retry loading` states. Keyboard focus plus ArrowRight/Enter activated Definitions, Fitting Attempts, Materialization Attempts, and Datasets in both locales; the accessibility tree exposed the corresponding tab/tab-panel roles and labels. No real credentials or provider data were used.

After fixture acceptance, delete only disposable profiles, temporary acquisition directories, generated package/build outputs, and test databases created for the run. Do not delete repository fixtures or finalized evidence. If a platform keeps a file handle open, stop the owning process before cleanup and record the platform result.

M10 is accepted only when every applicable row above passes, all automated gates are green, required platform evidence is recorded, and no M10 boundary is violated. M11 Factor research and all later Model, Strategy, Paper, Bot, Monitoring, and feedback milestones remain out of scope.
