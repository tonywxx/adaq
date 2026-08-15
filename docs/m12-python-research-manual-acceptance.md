# M12 Python Research manual acceptance

[简体中文](./m12-python-research-manual-acceptance.zh-CN.md)

Status: acceptance matrix for the implementation of the accepted 2026-08-13 Q93 contract for the [M12 Python Research SDK and Qlib-first Model Lab architecture](./m12-python-research-and-model-lab.md). The parent specification is [#97](https://github.com/tonywxx/adaq/issues/97); local checks are evidence only for the criteria they directly cover, and supported-platform acceptance remains explicit.

This guide separates three gates:

- M12 accepts the shared Python foundation, Python Factor, and Qlib Ridge Model path.
- M13 extends the same contract with Python Strategy and the complete tutorial Backtest.
- M14 extends it with generated WASI Components and equivalence.

Do not mark a deferred row as passed during an earlier milestone.

## 1. Acceptance record

Record before testing:

| Field | Required evidence |
| --- | --- |
| Reviewed revision | Full Git SHA with no unreviewed generated or local source changes |
| Application | ADAQ version and build identity |
| Platforms | macOS ARM64, Windows x86_64, Linux x86_64 |
| Toolchains | Rust, Node, pnpm, managed CPython, SDK, Runner, Qlib Adapter versions |
| Runtime artifacts | Platform, version, source, signature/hash, installed bytes |
| Wheelhouse | Manifest/signature hash and every selected wheel hash |
| Fixture | `python-tutorial-a-share@1`; Instrument `a6963ebf...fdaca`, Calendar `2e423b9b...978a9`, Bars `fd4dc3bc...bb4e`, Content `6d44423e...5d848` |
| Example revisions | Exact hashes for all applicable `py-*` Projects |
| Test User | Redacted User identity and clean/retained local-data state |
| CI | Workflow URL, revision, job URL, conclusion per platform |
| Manual environment | OS version, architecture, display scale, locale, assistive technology used |

Never copy credentials, provider tokens, signing keys, private absolute paths, or unbounded Python output into the acceptance record.

## 2. Preconditions and boundaries

Confirm before execution:

- M11 is complete and the exact Factor Research schema policy is understood.
- The App starts and non-Python routes work with no ADAQ Python Runtime installed.
- No system Python, Conda environment, active virtualenv, or User `PATH` interpreter is used by the test.
- The three examples are under `examples/python/`; the shared Dataset fixture is under `src-tauri/fixtures/python-tutorial/` and absent from every Project Archive.
- The fixture visibly identifies all 12 Instruments and all price history as synthetic.
- The test has a disposable User profile and enough disk for one Runtime, Wheelhouse, Project Environment, and retained evidence.
- Network is enabled only for the explicit Runtime or Wheel preparation step. Research Attempts run with no required network data.

Negative boundary inspection must find no embedded IDE, Jupyter server, generic Scripts page, Python order API, credentials in child environments, Qlib data downloader or Provider, Alpha158 implicit input, generic Qlib model promise, generic Python-to-WASM converter, or Marketplace publication UI.

## 3. Project and Archive contract

For each applicable example, exercise these operations:

| Operation | Expected result | Capture on failure |
| --- | --- | --- |
| Create from Example | A User Working Copy is created with the exact Kind-prefixed ID; an existing copy is not overwritten. | Requested ID, existing ID, path-safe diagnostic |
| Inspect root | Only the accepted Manifest, project metadata, Lock, `src/`, docs, and licence shape is authoritative. | Unexpected or missing path |
| Validate with no Runtime | Static validation completes without installing or importing Python. | Validation stage and diagnostic |
| Edit a declared source file | State becomes Dirty; no module loads and no Attempt starts. | Before/after source hash and process list |
| Introduce an unknown Manifest field or enum | State becomes Invalid; Prepare and Run are disabled; source remains open/exportable. | Schema version, field path, typed code |
| Change the Project ID prefix or Kind | Validation rejects the mismatch; historical identity is unchanged. | ID, Kind, Revision hash |
| Restore valid source and Run | One immutable Revision is frozen before execution; later edits do not affect it. | Frozen file list and hashes |

Archive validation must cover:

- Deterministic re-export of the same Revision produces identical ZIP bytes and hash.
- Private local export accepts `LicenseRef-Proprietary` only with matching `LICENSE` content.
- Bundled examples export with Apache-2.0 and include both language guides.
- A future Community eligibility check rejects proprietary or missing redistribution rights even though price is zero.
- Import copies an Untrusted Working Copy and performs no Runtime preparation, module import, or execution.
- Absolute paths, `..`, symlinks, hard links, duplicate paths, case-fold collisions, undeclared entries, count/size overflow, and Lock hash mismatch are rejected before copying.

## 4. Runtime, Lock, and Environment

Start from a device with no ADAQ Python Runtime:

| Operation | Expected result | Capture on failure |
| --- | --- | --- |
| Navigate through non-Python ADAQ workflows | They remain available and do not prompt for Python. | Route and unexpected prompt/process |
| Open a Python Project and Validate | Validation succeeds without Runtime download. | Project and validation evidence |
| Prepare Environment | The App shows exact CPython 3.12.x, platform, download size, disk requirement, source and hash before explicit preparation. | Artifact identity and UI state |
| Interrupt Runtime download or unpack | No partially staged Runtime becomes executable; Retry creates a new preparation Attempt. | Attempt IDs, staging path redacted, hash |
| Complete preparation | Signature/hash verification passes and atomic publication produces one exact Runtime and Environment identity. | Runtime/Environment hashes |
| Issue two matching preparations | Requests coalesce; no competing setup or research process pool appears. | Attempt linkage and process count |
| Edit `pyproject.toml` | Working Copy becomes Dirty; Run does not resolve or install anything. | Source/Lock hashes |
| Sync Environment | Trusted-index, wheel-only resolution atomically replaces `pylock.toml`; the resulting Revision is Untrusted. | Selected wheels and hashes |
| Insert a source distribution, build script, unsupported native wheel, or bad hash | Sync/Prepare rejects it without running package code. | Package identity and typed reason |
| Remove inactive Environment cache | Historical evidence remains readable; rerun reconstructs exact permitted bytes. | Before/after disk use and identity |
| Attempt a security-disabled old Runtime | Evidence remains inspectable but execution is blocked. | Profile, disabled reason, no process proof |

No Run may contact a package index, rewrite `pylock.toml`, use system site-packages, or select a newer public SDK wheel.

## 5. Trust, Runner, protocol, and lifecycle

| Operation | Expected result | Capture on failure |
| --- | --- | --- |
| Import or prepare an untrusted Project | No execution trust is granted. | Trust view and Revision hash |
| Choose Run | The exact Revision, entry point, Lock, source list, resource policy, and trusted-code warning are shown before confirmation. | Missing disclosure |
| Decline Trust | No Project module imports and no research Attempt starts. | Process/Attempt proof |
| Accept Trust | One Trust Decision binds the exact Revision only. | Decision and Revision hashes |
| Modify source, entry point, or Lock | Only that Project becomes Untrusted; other tutorial Decisions remain valid. | Before/after Revision and decisions |
| Start an Attempt | Host chooses a random loopback port and one-time token; exact Protocol/SDK/Revision/Attempt handshake completes before Project import. | Redacted handshake identities |
| Reuse a token, connect remotely, or mismatch a handshake field | Execution fails closed and publishes no result. | Typed code and no-publication proof |
| Emit stdout/stderr | Output appears only as bounded User-scoped logs and never as protocol data or automatic upload. | Log cap and redaction evidence |
| Retry a terminal Attempt | A new Attempt references the source Attempt; old evidence is immutable. | Attempt IDs |
| Restart the App with Pending and Running work | Pending remains queued; stale Running becomes Failed/Interrupted; no late result mutates it. | Before/after states |
| Cancel a cooperative and non-cooperative Project | Host requests cancellation, then terminates the process tree after grace; Cancelled is final only after exit and staging isolation. | Timings, process-tree proof, staging state |

Inspect the child environment and retained logs for absence of credentials, provider tokens, signing keys, order endpoints, SQLite paths, internal Parquet layout, and private absolute paths. Process isolation must not be described as a strong arbitrary-code sandbox.

## 6. M12 Python Factor journey

Use `py-factor-cross-sectional-momentum` and `python-tutorial-a-share@1`.

1. Validate that the Project is Kind Factor, Mode Portable Definition, Scope Cross Sectional, entry `project:create_project`, and Apache-2.0.
2. Confirm the Definition phase receives no Dataset and constructs only the existing Feature Operator Catalog graph:

   ```text
   close → backward-simple-return(lookback) → cross-sectional-percentile → momentum-score
   ```

3. Register the Host-owned Grid `lookback={5,20,60}`. Confirm three distinct Factor Trials and Attempts exist; no hidden Python Sweep result is accepted.
4. Bind the exact Snapshot, Point-in-Time Universe, Feature evidence, windows, Target, Seed, and Engine identities in the existing M11 protocols.
5. Materialize each standard Factor Dataset through the Python Candidate source. Confirm exact row identity, deterministic Universe order, finite binary64 or typed Unavailable values, and atomic publication.
6. Repeat in a fresh process and across allowed Batch partitions. The Repeatability Report must be Verified with exact output equality.
7. Run existing M11 scope-correct Evaluation and inspect Family lineage, every Trial, Selection and Final windows, missingness, metrics, and Evidence State.
8. Record the User's Parameter Selection Decision for lookback 20. The UI may suggest the tutorial default but cannot create the Decision automatically.
9. Inspect the Evaluation Report, then explicitly record Research Validated. Without this User Promotion Decision the Model step remains blocked.
10. Confirm the exact promoted Dataset output, Report, Policy, Decision, Revision, Environment, and Engine provenance are selectable by Model research.

Failure paths must cover invalid or custom portable operator, Dataset access during `define`, identity reordering, missing member, silent row deletion, NaN/infinity, wrong output count, exception, cancellation, Divergent replay, and incompatible `FACTOR_RESEARCH_SCHEMA_VERSION`. Imperative Python may remain inspectable Research Validated evidence but must never become Component Eligible without a supported portable representation.

## 7. M12 Qlib Ridge Model journey

Use `py-model-qlib-ridge-return` only after the exact Factor evidence receives a positive User Decision.

1. Validate one Model Kind, one Continuous Future Close Return Target, five-Bar horizon, one Forecast Signal, Qlib Ridge Adapter identity, and Apache-2.0.
2. Bind the same synthetic Snapshot and exact promoted Factor/Feature inputs through ordered Input Slots.
3. Verify the fixed windows:

   | Purpose | Sessions |
   | --- | --- |
   | Train | 1–100 |
   | Purge | 101–105 |
   | Selection Validation | 106–140 |
   | Embargo | 141–145 |
   | Final Evaluation | 146–180 |

4. Confirm a five-session Target crossing a boundary is Unavailable rather than shifted.
5. Inspect the `adaq.qlib` view: stable `(datetime, instrument)` order, only `train`, `valid`, and feature-only `test`, no Provider initialization, Qlib data directory, Alpha158, downloader, or network.
6. Confirm Host-owned standardization fits only on Train and freezes one Fitted Transformation Artifact applied unchanged later.
7. Register `alpha={0.1,1,10}` as three separate Trials and Attempts. The Host computes and records Selection MSE from Train-fitted transformations over Train/Selection Validation labels; no User-entered metric is authoritative.
8. Record one User Parameter Selection Decision before Final Evaluation. Test labels never enter Python; Host computes final metrics.
9. Extract `adaq:linear-model@1`, then reload it before published Forecast generation. Inspect ordered Input Slots, finite coefficients, intercept, numeric representation, Transformation identity, Forecast contract, and Adapter provenance.
10. Scan Project Archive, Artifact, staging, final Dataset, and Component inputs for pickle or executable object graphs; none may exist.
11. Generate the immutable Forecast Signal Dataset and Forecast Evaluation Report under existing M8 contracts.
12. Replay in a fresh process. Coefficients and Forecasts must remain inside the registered strict finite tolerance; identities, availability, order, and contracts remain exact.

Unsupported Qlib models, multiple targets, implicit Qlib processors, custom preprocessing, arbitrary serialization, generic ONNX export, or Local Qlib Paper claims must remain Research Only or fail explicit Adapter eligibility. They cannot inherit Ridge support by importability.

## 8. Guided tutorial behavior

Run Python Tutorial must:

1. Show the exact applicable Project Revisions, entry points, Locks, Runtime/Wheel downloads, disk need, licence, and trusted-code warning.
2. Permit one confirmation to record independent Trust Decisions; never create blanket or future trust.
3. Validate, prepare, and navigate mechanically while leaving the App responsive.
4. Stop after Factor evaluation until the User records Parameter Selection and Research Validated Promotion Decisions.
5. Stop after Model selection evidence until the User records its Parameter Selection Decision.
6. Run held-out Final Evaluation once for that Decision and truthfully label later feedback-driven work Overlapping.
7. In M12, finish at inspectable Factor and Model evidence and identify Strategy as an M13 continuation rather than a failed or hidden step.

Every displayed return or ranking must say Synthetic Demonstration and must not imply expected profitability. English and Chinese top-level and per-Project guides must describe the same buttons, paths, parameters, boundaries, expected structures, and troubleshooting.

## 9. Result validation, failure, and recovery

Every row below must retain a typed bounded diagnostic and publish no consumable partial result:

| Failure | Expected terminal behavior |
| --- | --- |
| Invalid Manifest, unsupported schema, entry-point mismatch | Invalid before preparation or Project import |
| Archive traversal, link, collision, undeclared or oversized entry | Import rejected before copying |
| Runtime or wheel hash/signature mismatch | Preparation Failed; no executable partial Environment |
| Untrusted Revision | Run blocked before import |
| Handshake/token mismatch | Attempt Failed before Project code |
| Oversized control, Arrow, artifact, checkpoint, or log | Typed limit failure; process stopped when required |
| Duplicate/reordered identity, invalid dtype/schema, NaN/infinity, invalid Decimal | Host validation failure; staging isolated |
| Missing required Factor/Model input | Typed Unavailable or failed input gate according to contract; no silent fill/drop |
| Python exception or child crash | Failed with bounded traceback and no authoritative partial result |
| Cancel or App restart | Terminal Cancelled or Failed/Interrupted only after isolation; Retry is new |
| Late result after cancellation/restart | Ignored; historical Attempt remains unchanged |
| Repeatability divergence | Result inspectable; Promotion, generation, and qualification blocked |
| Test-label access or overlapping selection | Access denied or Evidence State Overlapping; never Out-of-sample |
| SQLite/final Dataset write attempt | No path or authority is supplied; Host remains sole publisher |

Resource tests must exercise wall time, memory, thread, input row/column/cell, message, artifact, checkpoint, log, and process-count caps using measured platform policies. A Project request may lower but never increase Host limits.

## 10. Lab, Settings, localization, and accessibility

For Factor and Model Labs in M12:

| Operation | Expected result |
| --- | --- |
| Navigate directly to the Lab | Route shell paints immediately; no global blocking loader waits for Python or evidence. |
| Create/Open a Python Project | The Project appears in the owning Lab, not a generic Scripts page. |
| Prepare, Run, Cancel, Sync, Export | Pending/error/progress state belongs to the initiating control or Project/Attempt row. |
| Inspect Project | Clean/Dirty/Invalid, Missing/Preparing/Ready/Failed, Untrusted/Trusted, latest Attempt, hashes, and evidence links are visible. |
| Open Project Folder | The external editor/folder opens without an embedded editor, notebook, or terminal. |
| Inspect Settings | Runtime Profile, Wheelhouse/Environment disk use, and explicit inactive-cache removal are visible; no custom interpreter picker exists. |
| Use keyboard and screen reader | Actions, warnings, progress, logs, tables, dialogs, decision boundaries, focus restoration, and status announcements are usable. |
| Switch en-US/zh-CN | UI and docs localize immediately while IDs, hashes, schema codes, Decimal strings, and evidence identities remain unchanged. |

Test slow operations, rapid navigation, cancellation, stale responses, narrow windows, 200% scale, and each supported OS. Trust and destructive Reset dialogs require clear scope and focus handling.

## 11. Schema reset and cache retention

Create controlled incompatible metadata and verify:

- `PYTHON_RESEARCH_SCHEMA_VERSION` begins at `1.0.0`.
- Incompatible Python metadata blocks Python Research with explicit device-level Reset Python Research Evidence guidance.
- Reset stops Python research and removes Revision, Attempt, Trust, binding, and result metadata.
- User Working Copies and exported Project Archives remain untouched.
- Runtime, Wheelhouse, and Environment cache remains independently removable and is not confused with evidence reset.
- Incompatible Factor Candidate evidence uses the separate accepted Factor Research Reset for `1.0.0 → 1.1.0`.
- Neither reset performs migration, dual-read, silent deletion, or a full Local Data Reset.

Capture counts and hashes before and after Reset and prove another User's Working Copy source is not removed.

## 12. M13 Strategy extension gate

These rows are deferred during M12 and become mandatory for M13:

- `py-strategy-top-n-forecast` is independently valid, Apache-2.0, offline, and copied into Strategy Lab.
- `start(context)` creates one Segment-local Session; ordered `decide` calls are serial and never prefetch future batches.
- Required missing input records `Run Pause::MissingInput` before invocation.
- The finite Grid is `forecast-weight={0.5,0.7}`, `top-n={3,5}`, and `cash-reserve={0,0.1}` with defaults 0.7, 3, and 0.1.
- Portable operations are only weighted sum, deterministic Top-N, equal weight, and cash reserve.
- Equal scores use ascending Instrument ID.
- Every Universe member appears in the Long-only Target; nonnegative Decimal weights plus Cash Reserve equal exactly one.
- Host Risk, Execution, Backtest, and Portfolio State remain authoritative; Python emits no order.
- Strategy Repeatability and Golden Portfolio Targets are exact.
- The tutorial requires an explicit Strategy Parameter Selection Decision before its held-out Backtest.
- Short, leverage, custom eligibility, optimizers, stops, loops, callbacks, orders, and Qlib-native Backtest promotion remain excluded.

## 13. M14 generation extension gate

These rows are deferred during M12/M13 and become mandatory for M14:

- Portable Factor and Strategy Definitions and `adaq:linear-model@1` are the only three tutorial generation inputs.
- Fixed Rust SDK Generators receive canonical Definitions or Artifact data, never Python execution.
- Python source, Runtime, wheels, Environment, Lock, Dataset, and research results are absent from `.adaq`.
- Selected Factor/Strategy values become defaults; every finite allowed combination passes conformance and equivalence within Host caps.
- The Ridge Model Exporter produces one WASI Model Component; generic Qlib-to-WASM/ONNX and Local Qlib Paper remain unsupported.
- Generated Component Provenance binds Revision, Definition/Artifact, parameter schema, Decisions, Generator, SDK, ABI, toolchain, Build Attempt, and Equivalence Report.
- Build, conformance, numeric-boundary, resource, provenance, equivalence, trust, package validation, identity, and import gates all pass before Component Library entry.
- Failure retains evidence and never overwrites an existing Package identity/version.
- Exact Factor/Strategy behavior and tolerance-governed Ridge Forecast equivalence pass on all supported platforms.

## 14. Automated gates

M12.1 must add and document the repository-managed Python package/contract test entry point; do not replace that future command in evidence with a developer's system Python invocation. At the accepting revision, record the exact committed command and run at minimum:

```sh
(
  cd src-tauri
  cargo fmt --all --check
  cargo test -p adaq-python-research
  cargo test --workspace
  cargo check --workspace
)
pnpm exec jest --watchman=false --runInBand
pnpm run build
pnpm run lint
git diff --check
```

Also run the committed repository-managed commands added by the slices for:

- public SDK and private Runner unit/contract tests;
- deterministic Project Archive generation and hostile-archive fixtures;
- Runtime/Wheelhouse signature and Lock tests;
- Runner Protocol, cancellation, restart, resource, and redaction tests;
- Python Factor exact Golden and repeatability tests;
- Qlib Ridge Artifact/reload, withheld-label, tolerance, and Forecast Dataset tests;
- bilingual documentation path/parameter/expected-structure checks;
- retained diagnostic secret/path scan.

Every command must exit zero. Record exact test counts, ignored tests, warnings, fixture hashes, and any platform-specific limitation. A command not yet committed is an unmet Acceptance Criterion, not permission to invent local evidence.

## 15. Supported-platform CI

Required ongoing matrix:

| Trigger | macOS ARM64 | Windows x86_64 | Linux x86_64 |
| --- | --- | --- | --- |
| Pull request | Fast Manifest/Archive/SDK contracts | Fast Manifest/Archive/SDK contracts | Fast contracts plus complete offline Factor → Model tutorial |
| `main` | Runtime prepare, applicable full chain, Golden and failures | Same | Same |
| Release/manual | Runtime prepare, applicable full chain, Golden and failures | Same | Same |
| Accepting M12 slice | One recorded all-platform green run for added M12 capability | Required | Required |
| Accepting M13/M14 slice | One recorded all-platform green run for its extension | Required | Required |

The full failure matrix includes cancellation, untrusted Revision, Lock/hash failure, invalid output, restart recovery, and staging isolation. Record workflow URL, exact SHA, each job URL, conclusion, Runtime/Wheel hashes, and Fixture hash. A local run does not substitute for this evidence.

## 16. M12 slice acceptance matrix

| Slice | Required independent evidence | Explicit Out of Scope |
| --- | --- | --- |
| M12.1 Project/Archive/SDK | Rust core contracts, both Python package shapes, exact Manifest validation, safe Archive import/export, licences, source-visible examples, focused hostile-input tests | Runtime download, Python execution, Factor/Model results |
| M12.2 Runtime/Environment | Managed CPython 3.12, signed base Wheelhouse, Sync/Lock, atomic preparation, cache accounting/eviction, failure/retry tests | Research execution, custom interpreter, sdist builds |
| M12.3 Runner/lifecycle | Private process, handshake, IPC/staging, Trust, resources, queue, cancellation/restart/late-result/redaction evidence | Factor/Model semantic evaluation, process pool, strong sandbox claim |
| M12.4 Python Factor | Third Candidate source, schema/reset, exact Dataset, existing M11 evidence reuse, Repeatability, Factor Lab/example/docs | Qlib Model, Strategy, Component generation |
| M12.5 Qlib Ridge core | Host-fed Dataset Bridge, Train-only transformations, Ridge Adapter, withheld Test label, data-only Artifact/reload, unsupported-model gates | Model Lab completion, ONNX, Local Qlib Paper |
| M12.6 Model Lab | Grid/Trials, Selection/Final, Forecast Dataset/Evaluation, Repeatability tolerance, Model example/docs and responsive UI | Strategy execution, Component generation |
| M12.7 Acceptance | Guided Factor/Model tutorial, synthetic fixture, bilingual docs, Golden/failure matrix, all-platform evidence and final criterion mapping | M13 Strategy completion, M14 build/import, Marketplace |

Each child issue must map every Acceptance Criterion to focused implementation evidence, broad regression evidence, manual evidence, exact revision, supported-platform result, and remaining limitation before closure. Child completion never authorizes closing a parent issue unless the User explicitly requests it.

## 17. Final M12 acceptance condition

M12 is accepted only when:

- all seven dependency-ordered slices are complete with one traceable issue graph;
- every M12 row above passes and M13/M14 rows remain truthfully deferred;
- Factor and Model examples are executable, bilingual, offline after preparation, and independently inspectable;
- the synthetic Fixture, exact Golden evidence, Ridge tolerance, Trust, Promotion, Selection, and held-out boundaries are visible;
- no partial, divergent, overlapping, untrusted, unsupported, or failed result is presented as qualified;
- focused, workspace, frontend, documentation, secret/redaction, and three-platform gates pass at the reviewed revision;
- English completion evidence records commands, counts, SHA, CI links, limitations, and no secret material.

Planning approval or this document alone is not implementation or acceptance evidence.
