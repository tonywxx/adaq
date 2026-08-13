# M11 Manual Acceptance

This is the canonical human-reviewed M11 path. The native/workflow evidence revision is `36fc8467b16a357ed17a642f91919471a281f77d`; the final acceptance-only revision is recorded in the completion comment. M11 ends at immutable Factor research evidence and explicit User-owned promotion decisions. It does not deliver M12 Model training, M13 Strategies, M14 Component qualification/import, Paper, Bots, Marketplace, or real-money trading.

Use committed deterministic fixtures as the authoritative path. Never put credentials, authorization headers, tokens, private paths, private market data, or unredacted build diagnostics in comments, commits, screenshots, logs, or exports.

<!-- m11-acceptance:scope -->
## 1. Scope and prerequisites

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| Run `node --version` from the repository root. | Node.js 24 or later is selected. | Complete output and installation method. |
| Run `pnpm --version`. | pnpm 11.20.0 is available, matching `packageManager` in `package.json`. | Complete output and tool versions. |
| Run `pnpm install --frozen-lockfile`. | Dependencies match `pnpm-lock.yaml`. | Complete output and the two tool versions. |
| Run `rustup show` and `cargo component --version`. | Stable Rust and the component toolchain are available; no machine-local WIT path is required. | Complete output and installed target list. |
| Start a fresh local User and open **Settings → General**. | English (US) and 简体中文 are available; no M11 evidence is visible before creation. | Redacted User ID, locale, platform, and screenshot. |
| Hash any exported fixture with `shasum -a 256 <path>` on macOS, `Get-FileHash -Algorithm SHA256 <path>` in Windows PowerShell, or `sha256sum <path>` on Linux. | The recorded digest is reproducible without exposing the path or contents. | Redacted command output and platform. |

The supported-platform substitution is macOS ARM64 (`aarch64-apple-darwin`), Windows x86_64, or Linux x86_64. Provider credentials are unnecessary for this acceptance path; the committed fixtures and local evidence stores are authoritative.

<!-- m11-acceptance:contracts -->
## 2. Contracts, ABI, and evidence identity

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| Run `cd src-tauri && cargo test -p adaq-factor-research --lib -- --nocapture`. | Candidate, materialization, evaluation, Family, Policy, Decision, identity, missingness, and limit contracts pass. | Revision, failing test, and complete output. |
| Run `cd src-tauri && cargo test -p adaq-component-sdk -- --nocapture` and `cargo test -p adaq-component-tooling -- --nocapture`. | SDK bindings, manifest validation, package trust boundaries, ABI v2 worlds, and conformance pass. | Revision, failing test, and complete output. |
| Build the `factor`, `multi-output-factor`, `cross-sectional-factor`, `repeated-factor-strategy`, and `external-strategy` fixtures through the existing component path. | Fixture packages build without network, preserve declared scope/output identity, and remain subject to host verification. | Fixture, command, target, and redacted diagnostic. |
| Inspect `src-tauri/wit/factor/adaq-factor.wit`, `src-tauri/crates/adaq-component-sdk/wit/factor/adaq-factor.wit`, `docs/reference/component-manifest.schema.json`, and the generated Factor catalog. | ABI v2 uses host-resolved Feature Batches, Time-Series or Cross-Sectional scope, typed missingness, ordered identity, and no v1 compatibility layer. | File, stale claim, and revision. |
| Run `cd src-tauri && sh crates/adaq-factor-research/scripts/check_generated.sh`. | The Metric Catalog and reference artifacts regenerate without diff. | Revision, generated file, and complete output. |
| Inspect `CONTEXT.md`, ADR 0060–0062, `docs/m11-factor-research.md`, `docs/v1-roadmap.md`, the SDK/tooling guides, and the GUI copy. | Ownership, immutable evidence, native queue, ABI v2, M12 boundary, and `/factors` entry agree in both locales. | Document, paragraph, and contradictory claim. |

`reset-required` is the only compatibility path for stored Factor ABI v1 evidence: there is no migration, dual reader, or automatic deletion. Presentation names and tags are User-scoped metadata and never alter semantic hashes.

<!-- m11-acceptance:okx-journey -->
## 3. OKX Spot Time-Series journey

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| Select a Completed M10 Feature Dataset bound to an OKX Spot Snapshot and open `/factors`. | The Factor Lab shows the exact User, Feature Dataset, Snapshot, range, and market context before work is queued. | IDs, context, locale, and screenshot. |
| Publish a Declarative Candidate with one or more ordered Feature Slots and a deterministic parameter revision. | The positive revision has canonical JSON and a content hash; presentation edits do not change that hash. | Candidate revision/hash and screenshot. |
| Materialize the Candidate for the exact Dataset, then inspect the wide Factor Dataset. | Rows are keyed by `(Instrument ID, Observation Time)`; values preserve `Available At`, Warmup, Bar Gap, missingness, and Candidate/Feature/Engine provenance. | Attempt/Dataset IDs and missing manifest field. |
| Freeze chronological holdout and walk-forward Protocols with positive horizons, purge, embargo, Temporal and Economic Lenses. | Reports use `close[t+h] / close[t] - 1`, never random splits, and retain fold identities and Evaluation Evidence State. | Protocol/Report IDs, windows, and state. |
| Register the Candidate in a Research Family, run a bounded Grid, and inspect all Trials. | Completed, Failed, Cancelled, Rejected, and Superseded Trials remain visible; Holm correction uses the complete registered family. | Family/Trial IDs, status list, and correction population. |
| Freeze a Policy-satisfying Out-of-sample Report and record a User Decision for one exact output. | `Research Validated` is explicit, immutable, cited, and appears in exact M12 eligibility only with Dataset, Report, Policy, Decision, and Engine provenance. | Decision gates, cited hashes, and eligibility response. |
| Run `cd src-tauri && cargo test -p adaq-factor-research --test reference_fixtures`. | The committed OKX vectors pass for multi-horizon momentum, Warmup, Bar Gap restart, Temporal evidence, decay/stability, and costs. | Revision, test, and vector mismatch. |

<!-- m11-acceptance:a-share-journey -->
## 4. China A-share journey

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| Use the committed A-share fixture with venue-local Asia/Shanghai sessions and scheduled closures. | Session boundaries and closures are calendar evidence, not inferred Bar Gaps. | Venue, timestamps, calendar state, and vector digest. |
| Materialize a Time-Series Candidate across the morning session, midday break, and afternoon session. | State continues across a Scheduled Closure and resets only at a genuine Bar Gap; Warmup and missing inputs remain typed. | Segment IDs, gap/closure classification, and Dataset rows. |
| Inspect a verified Corporate Action around the target horizon. | Corporate Action evidence remains bound to the Instrument and effective evidence time; unavailable Close/target evidence is retained, never silently adjusted. | Action evidence ID, Price Basis, Available At, and typed reason. |
| Evaluate the same Candidate with causal holdout/walk-forward folds and the Economic Lens. | Target availability, purge/embargo, fold state, fees, slippage, rebalance, and cost-aware results remain explicit. | Report ID, fold evidence, assumptions, and metric state. |
| Run `cd src-tauri && cargo test -p adaq-factor-research --test reference_fixtures`. | The China A-share reference vectors pass for sessions, closures, Corporate Actions, target availability, Time-Series evaluation, and typed unavailable paths. | Revision, failing journey, and complete output. |

<!-- m11-acceptance:us-equity-journey -->
## 5. U.S. equity Cross-Sectional journey

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| Bind a Cross-Sectional Candidate to one complete Point-in-Time Universe at one Observation Time. | Full membership and deterministic order are retained; mixed Venue, Asset Class, Snapshot, Currency, or Universe context is rejected. | Universe ID, ordered members, context, and typed error. |
| Materialize with an unavailable member and then inspect the Dataset. | The member remains in the batch with typed missingness; the host never drops it or invents a value. | Member ID, reason code, row count, and manifest. |
| Evaluate Cross-Sectional and Economic Lenses with deterministic average ties and five quantile groups. | IC, Rank IC, turnover, Top-only, Top-minus-Bottom, fee/slippage, and rebalance evidence are ordered and reproducible. | Report ID, metric samples, and vector digest. |
| Add explicit nuisance Features and inspect neutralized results. | OLS runs per Observation Time with an intercept over complete cases while the complete Universe and missingness remain in evidence; insufficient or singular fits are typed Unavailable. | Nuisance identities, batch, sample count, and reason. |
| Add a causal Regime Feature. | Thresholds are fitted only on the selection window and applied unchanged to evaluation observations; bucket evidence is retained. | Threshold identity, selection range, and per-bucket output. |
| Freeze Policy gates and record a Decision for one named output in a multi-output Dataset. | Each output is independently decided; no output is promoted from Overlapping/Unknown evidence, and M12 eligibility requires the exact positive evidence chain. | Output name, gates, Decision, and eligibility. |
| Run `cd src-tauri && cargo test -p adaq-factor-research --test reference_fixtures` and `cargo test -p adaq-factor-research --test metric_golden`. | U.S. Cross-Sectional membership/order, missingness, ties, IC/Rank IC, turnover, neutralization, regimes, costs, and literal golden metrics pass. | Revision, test, and mismatch. |

<!-- m11-acceptance:candidate-paths -->
## 6. Declarative, Custom, and trust-boundary paths

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| Edit a Declarative Draft, publish two revisions, and change only presentation metadata. | Only the immutable semantic revision/hash changes; names, descriptions, and tags stay outside the hash. | Before/after hashes and User scope. |
| Build a private Custom Candidate from the fixed offline/locked project path. | Source hash, SDK, ABI, compiler/toolchain, target, commands, environment, resource policy, bounded diagnostic, and Package hash are frozen. | Attempt ID and redacted diagnostic. |
| Attempt a network access, custom build script, ambient file read, invalid scope/world, non-finite output, wrong row/order, or output mismatch. | The trust boundary rejects the package before evidence publication and retains a safe typed failure. | Typed error, Attempt ID, and no published Dataset proof. |
| Compare equivalent Declarative and private Custom Candidate fixture paths. | Factor Dataset and evaluation vectors are equivalent when declared semantics match; distinct Engine identities never collapse Report hashes. | Candidate/Engine hashes and vector diff. |
| Inspect a multi-output Candidate and the Component Library. | Outputs are decided independently; a Custom Package remains private and non-imported until M14 qualification, conformance, and import. | Output decisions, package state, and screenshot. |

<!-- m11-acceptance:failure-recovery -->
## 7. Failure, recovery, and retention paths

Exercise each row with committed fixtures or typed test setup. A failure is accepted only when the typed reason and safe retained evidence are visible and no partial evidence is consumable.

| Failure path | Expected evidence |
| --- | --- |
| ABI v1 package/evidence | `reset-required`; explicit device-level reset; no migration, dual read, or automatic deletion. |
| Candidate build failure, missing input, Bar Gap, non-finite output | Failed Attempt or typed Unavailable row; bounded redacted diagnostic; no partial Dataset. |
| Universe mismatch, missing member, singular neutralization, undefined required metric | Complete batch/report remains inspectable with typed reason; positive promotion is blocked. |
| Target leakage, overlapping/unknown fold, or omitted related Trial | Validation or promotion is rejected; lineage and window evidence remain retained. |
| Policy rejection or explicit User Rejection | Decision is immutable and output-specific; no automatic promotion or floating latest pointer appears. |
| Cancellation, crash/restart, queue contention | Pending survives, stale Running becomes typed Failed, cancellation publishes nothing partial, and the single FIFO remains User-scoped. |
| Atomic publication or corrupted Parquet/Report payload | Staging is discarded or isolated; hashes/schema reject the evidence before display. |
| User isolation, deletion lock, final-reference cleanup, explicit Reset | Another User cannot see the evidence; references block deletion; only the last User reference removes shared bytes; Reset is explicit. |

Focused coverage is retained in `src-tauri/src/factor_research/mod.rs`, `src-tauri/crates/adaq-factor-research/src/{abi,candidate,evaluation,promotion,research}.rs`, and the Factor integration suites.

<!-- m11-acceptance:factor-gui -->
## 8. `/factors` workspace and accessibility

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| Navigate directly to `/factors` with a signed-in fixture User. | The shell paints immediately before native reads complete; Families, Candidates, Datasets, Evaluations, and Decisions are visible. | Route timing, screenshot, and console error. |
| Enter a slow list, switch tabs, leave, and re-enter. | Current-session User-scoped cache renders first and background revalidation updates it; stale responses cannot overwrite newer User/page data. | User/resource/page, request order, and screenshot. |
| Start Candidate build, Grid registration, materialization, evaluation, cancellation, retry, or deletion. | Busy/progress/error/retry state belongs to the owning card/control; the whole page remains usable and no automatic Promote action exists. | Attempt ID, control state, `aria-busy`, and screenshot. |
| Use keyboard-only navigation through tabs, forms, tables, pagination, lineage, metric details, and decision gates in en-US and zh-CN. | Labels/descriptions, focus order, tab/table semantics, status announcements, narrow-window horizontal access, and localized formatting remain usable; canonical IDs/codes stay exact. | Platform, scale, focused element, accessibility tree, and screenshot. |
| Inspect failed Trials, typed unavailable metrics, raw/Holm statistics, provenance, deletion locks, and M12 eligibility. | The workspace exposes evidence boundaries rather than hiding failures or presenting historical results as guarantees. | Missing field, route, and redacted evidence. |

The source-level and Jest contracts are in `src/features/factors/factors-page.test.ts`, `factor-adapter.test.ts`, `factor-data.test.ts`, `src/loading-boundaries.test.ts`, `src/router.test.ts`, and `src/lib/i18n.test.ts`. OS assistive-technology differences must be recorded per supported platform; they are not replaced by a local browser pass.

<!-- m11-acceptance:boundary -->
## 9. M11 boundary and deferred capability checks

Inspect routes, commands, docs, and GUI copy for the following negative requirements:

- No Qlib/Python Runner, notebook execution, Model training, Strategy construction, Component Equivalence/import, Paper, Bot, Live, Marketplace, automatic promotion, adaptive optimization, or cross-market raw-sample pooling is exposed as M11 functionality.
- M11 uses only the ADAQ Native Research Engine and consumes Completed M10 Feature Datasets.
- Economic diagnostics are not Strategy Backtests; Component Eligible is not M14 qualification/import; Research Validated is not a profitability guarantee.
- No mutable `latest` evidence reference, universal IC/return/profitability threshold, hidden fitting, hidden imputation, or script runtime is added.
- M12 selects only an exact Completed Factor Dataset output with a current positive Decision and frozen Report, Policy, Promotion Protocol, and Engine Provenance.

<!-- m11-acceptance:performance-baselines -->
## 10. Performance baselines and resource ceilings

| Exact operation | Expected result | On failure, capture |
| --- | --- | --- |
| Run `cd src-tauri && cargo test -p adaq-factor-research --test benchmarks -- --test-threads=1`. | The non-ignored bounded workload checks Candidate execution and resource accounting. | Revision, workload, complete output, and platform. |
| Run `cd src-tauri && cargo test -p adaq-factor-research --release --test benchmarks -- --ignored --test-threads=1` on macOS ARM64. | The canonical 1,000,000-Bar Time-Series and 10,000 × 252 Cross-Sectional workloads complete with cancellation/chunk/recovery checks. | Runtime, high-water RSS, artifact hashes, and platform. |
| Compare `src-tauri/crates/adaq-factor-research/fixtures/factor-benchmark-baseline.json`. | Record-only macOS ARM64 baseline: 723 ms Time-Series, 1,548 ms Cross-Sectional, 29,884,416-byte high-water RSS, and the two Candidate Package hashes. These are measurements, not SLAs. | Baseline digest, measured values, and platform difference. |
| Inspect public ceilings before allocation. | 2,520,000 Dataset rows, 32 folds, 16 horizons, 5 lenses, 16 nuisance Features, and one device-wide worker are checked with checked arithmetic. | Requested limit, typed rejection, and allocation proof. |

<!-- m11-acceptance:regressions -->
## 11. Regression and boundary gates

| Exact operation | Expected result |
| --- | --- |
| Run `cd src-tauri && cargo test --workspace`. | M1–M10 behavior and all M11 native suites pass; ignored benchmark tests are explicitly recorded. |
| Run `pnpm exec jest --watchman=false --runInBand`. | Frontend routes, loading boundaries, locales, factor workspace contracts, and prior milestone suites pass. |
| Inspect README, workflow navigation, SDK/tooling docs, generated schemas, ADR 0060–0062, and both M11 architecture guides. | Links resolve, status is accepted, and M11 paths contain no stale planned/v1 ABI claim. |
| Run a retained-diagnostic scan over the final diff and generated evidence. | No credentials, tokens, absolute private paths, or unsafe diagnostic material is retained. |

<!-- m11-acceptance:automated-gates -->
## 12. Automated gates

Run these commands at the final acceptance revision:

```sh
(
  cd src-tauri
  cargo fmt --all --check
  cargo test --workspace
  cargo check --workspace
  cargo test -p adaq-factor-research --test reference_fixtures
  cargo test -p adaq-factor-research --test metric_golden
  cargo test -p adaq-factor-research --test benchmarks -- --test-threads=1
  cargo test -p adaq-factor-research --release --test benchmarks -- --ignored --test-threads=1
  sh crates/adaq-factor-research/scripts/check_generated.sh
)
pnpm exec jest --watchman=false --runInBand
pnpm run build
pnpm run lint
git diff --check
```

Expected result: every command exits zero. Record exact counts and any pre-existing warnings separately; a local pass never substitutes for the required platform matrix. The repository has no configured secret-scanner command, so the retained-diagnostic scan is a manual final-diff check unless a scanner is added by the project.

<!-- m11-acceptance:platform-evidence -->
## 13. Supported-platform evidence

The manually dispatchable [Indicator engine acceptance workflow](../.github/workflows/indicator-engine.yml) builds/verifies Factor fixtures, checks generated Factor references, runs the full Rust workspace, and runs the canonical Factor benchmark on macOS ARM64. The reviewed native/workflow revision is `36fc8467b16a357ed17a642f91919471a281f77d`.

| Workflow evidence | Revision | Jobs | Result |
| --- | --- | --- | --- |
| [Indicator engine acceptance run 31664792735](https://github.com/tonywxx/adaq/actions/runs/31664792735) | `36fc8467b16a357ed17a642f91919471a281f77d` | [macOS ARM64](https://github.com/tonywxx/adaq/actions/runs/31664792735/job/94336905508), [Windows x86_64](https://github.com/tonywxx/adaq/actions/runs/31664792735/job/94336905609), [Linux x86_64](https://github.com/tonywxx/adaq/actions/runs/31664792735/job/94336905553) | Success (all three) |

To repeat the dispatch, run `gh workflow run "Indicator engine acceptance" --ref <reviewed-ref>`, then record the workflow URL, SHA, job URLs, conclusion, and any failed log excerpt. Windows uses PowerShell hash commands and explicit handle cleanup; Linux installs the workflow's Tauri prerequisites; macOS ARM64 is the only platform for the ignored large benchmark.

<!-- m11-acceptance:acceptance-matrix -->
## 14. Final acceptance matrix

The matrix is evidence, not a substitute for issue state. Each row identifies the implementation boundary, focused evidence, broad/manual section, and remaining limitation. The child issue comments are independent evidence records; this matrix closes the cross-slice relationship.

### Parent #88 criteria

| ID | Criterion mapped to evidence | Implementation / focused evidence | Broad/manual evidence | Remaining limitation |
| --- | --- | --- | --- | --- |
| P1 | Eight dependency-ordered slices and independent evidence. | #92, #90, #89, #91, #95, #96, #94 comments and slice rows below. | Sections 2–13. | None after every child comment is present. |
| P2 | `adaq-factor-research` owns materialization, catalog, evaluation, Families, Policies, Reports, Decisions, and Library without absorbing Feature/tooling semantics. | Crate modules plus #92/#95 focused tests. | Sections 2, 6–7, 9. | None. |
| P3 | ABI v2 has scope-specific batches, identity, typed missingness, and explicit reset for incompatible v1 evidence. | SDK/tooling tests, WIT, `reset-required`, #92. | Sections 2, 7, 9, 13. | v1 evidence is intentionally reset-only. |
| P4 | Declarative and private Custom Candidates materialize immutable Datasets before evaluation. | #90 materialization and parity tests. | Sections 3, 6. | Custom Packages remain private until M14. |
| P5 | Causal holdout/walk-forward Reports retain targets, lenses, neutralization, robustness, costs, and evidence state. | #89 evaluation and golden tests. | Sections 3–5, 10. | No universal profitability threshold. |
| P6 | Research Families retain all Trials and lineage; omission cannot obtain promotion. | #91 registry, Holm, lineage, and omission tests. | Sections 3, 7, 14. | None. |
| P7 | Only an immutable User Decision can produce a positive output state. | #91 Policy/Decision/Library/M12 eligibility tests. | Sections 3, 5, 7, 9. | M14 qualification remains deferred. |
| P8 | M12 selects only exact Completed Dataset outputs with frozen evidence provenance. | `m12_eligibility` focused tests and #91/#96 evidence. | Sections 3, 5, 9. | M12 itself is out of scope. |
| P9 | SQLite/Parquet atomicity, recovery, User isolation, deletion locks, and shared FIFO pass. | #95 native tests and retained diagnostics. | Sections 7–8, 11–13. | OS file-handle behavior is platform-recorded. |
| P10 | `/factors` is localized, accessible, immediate-paint, control-owned, cached, and evidence-oriented. | #96 frontend tests and source contracts. | Section 8 and the manual guide. | OS assistive-technology differences remain platform-specific. |
| P11 | Three-market journeys and declared failure paths pass. | #94 reference fixtures, golden vectors, benchmarks, and #89/#90/#91/#95 tests. | Sections 3–7, 10, 13. | Fixtures are deterministic evidence, not live-provider coverage. |
| P12 | M11 excludes Qlib/Python Runner, training, Strategies, Component import, Paper/Bot/Live/Marketplace, auto-promotion, adaptive optimization, and pooling. | Boundary docs, route/source tests, and native command ownership. | Section 9. | M12+ remain future work. |
| P13 | Every criterion maps to implementation, focused, broad, manual, revision, and limitation evidence. | This matrix plus all child comments. | Sections 1–13 and issue comments. | Manual OS observations must be retained by the reviewer running them. |
| P14 | Formatting, Rust, frontend, generated-reference, lint, diff, secret, and supported-platform gates pass. | Section 12 commands and Section 13 run. | Sections 10–13. | No configured secret scanner exists in this checkout. |
| P15 | English completion evidence is posted before #93 and parent #88 close. | Completion comments link this guide, final SHA, commands, counts, and CI. | Section 15 and issue history. | Closure is a separate GitHub state transition. |

### M11.1–M11.7 slice matrix

| Slice | Every Acceptance Criterion mapped | Focused implementation evidence | Final/manual evidence | Limitation |
| --- | --- | --- | --- | --- |
| #92 / M11.1 | 1 workspace crate; 2 canonical contracts; 3 ABI v2 scope/slots/outputs; 4 Time-Series batch; 5 Cross-Sectional batch; 6 host validation; 7 typed `reset-required`; 8 Metric Catalog; 9 JSON/output/grid/WASM limits; 10 chunk identity; 11 SDK/WIT/schema/conformance updates; 12 canonical/invalid/conformance tests; 13 Rust/check/workspace gates. | `adaq-factor-research` contracts/ABI/catalog, SDK/tooling, WIT, manifest schema, fixtures. | Sections 2, 6–7, 9, 12–13. | Reset is explicit by design. |
| #90 / M11.2 | 1 Declarative revisions; 2 Feature semantics/no expression runtime; 3 presentation isolation; 4 Custom build provenance; 5 controlled private build; 6 protocol binding; 7 scope/missingness materialization; 8 immutable wide Dataset; 9 coalesce/reuse/retry; 10 atomic staging; 11 Dataset-only evaluation boundary; 12 isolation/locks/deletion; 13 focused lifecycle tests; 14 Rust/check/workspace gates. | Candidate/materialization modules, fixture parity, lifecycle and trust-boundary tests. | Sections 3, 6–8, 12. | Evaluation and GUI ownership is downstream. |
| #89 / M11.3 | 1 Future Close Return; 2 causal availability/typed target failures; 3 comparable context; 4 orientation; 5 holdout/walk-forward; 6 evidence state; 7 required Lenses; 8 OLS neutralization; 9 regimes; 10 Economic Lens; 11 metric evidence; 12 typed catalog undefined; 13 Engine/Report identity; 14 focused numeric tests; 15 Rust/reference/check gates. | `evaluation.rs`, metric catalog, golden/reference vectors. | Sections 3–5, 7, 10, 12. | Binary/Custom Targets are out of scope. |
| #91 / M11.4 | 1 Family/Trial registration; 2 retained statuses; 3 lineage; 4 bounded Grid; 5 raw/p/Holm; 6 complete lineage gate; 7 versioned Policy; 8 no universal thresholds/rule engine; 9 explicit output Decision; 10 OOS positive gate; 11 Component Eligible boundary; 12 append-only supersession/Library; 13 per-output multi-output promotion; 14 M12 eligibility; 15 focused tests; 16 Rust/check/workspace gates. | `research.rs`, `promotion.rs`, registry/lineage/Policy/Decision tests. | Sections 3, 5, 7, 9, 14. | M14 qualification/import is deferred. |
| #95 / M11.5 | 1 typed SQLite metadata; 2 Parquet payloads; 3 shared FIFO; 4 Attempt lifecycle; 5 restart/cancellation atomicity; 6 complete-unit progress; 7 non-blocking Tauri boundary; 8 bounded User APIs; 9 reference locks; 10 retained failures; 11 explicit reset; 12 redacted bounded diagnostics; 13 checked public ceilings; 14 queue/storage/API tests; 15 Rust/frontend/diff/secret gates. | `src-tauri/src/factor_research/mod.rs`, native lifecycle and queue tests. | Sections 7–8, 10–13. | Native UI inspection remains platform-specific. |
| #96 / M11.6 | 1 route/navigation/workflow metadata; 2 immediate paint/control loading; 3 cache/revalidation/stale guards; 4 Family lineage; 5 Candidate UI; 6 Dataset UI; 7 Evaluation UI; 8 metric result UI; 9 Decision UI; 10 no guarantees/auto-promotion; 11 bilingual copy; 12 accessibility/narrow window; 13 User isolation; 14 focused frontend tests; 15 Jest/build/lint/Rust/diff/manual gates. | Router, i18n, factor adapter/page/data, loading and route tests. | Section 8 and Sections 11–13. | OS assistive technology is not identical across platforms. |
| #94 / M11.7 | 1 OKX vectors; 2 A-share vectors; 3 U.S. vectors; 4 Declarative/Custom equivalence; 5 independent golden catalog vectors; 6 large workloads; 7 macOS baseline/ceilings; 8 bounded/chunk/cancel/recovery; 9 Engine identities/tolerance; 10 bounded property/failure tests; 11 diagnostic leakage checks; 12 generated references; 13 focused/broad/frontend/diff gates; 14 exact reviewed-SHA CI. | `reference_fixtures.rs`, `metric_golden.rs`, `benchmarks.rs`, fixtures, workflow. | Sections 3–7, 10, 12–13. | Baseline is record-only; CI does not replace GUI inspection. |

<!-- m11-acceptance:acceptance-record -->
## 15. Acceptance record and cleanup

Record the final acceptance commit, native/workflow revision, OS/architecture/display scale, Node/pnpm/Rust versions, focused counts, full-gate results, fixture/vector digests, redacted User IDs, route/accessibility observations, and exact CI URLs/SHA/conclusions. The current supported-platform evidence is run [31664792735](https://github.com/tonywxx/adaq/actions/runs/31664792735) at `36fc8467b16a357ed17a642f91919471a281f77d`, with successful macOS ARM64, Windows x86_64, and Linux x86_64 jobs. The acceptance-only files do not alter the native/workflow inputs; local frontend/docs gates are recorded at the final acceptance commit.

After acceptance, remove only disposable profiles, temporary build directories, generated package outputs, and test databases created for the run. Do not delete committed fixtures or finalized evidence. On Windows, release SQLite/file handles before cleanup and record the platform result.

M11 is accepted only when every applicable row above passes, the child comments and final matrix are posted in English, all local gates are green, the current native/workflow evidence is recorded, and no M11 boundary is violated. Close #93 first, then close parent #88; do not close any M12+ issue.
