# V1 OKX Spot end-to-end manual acceptance

Status: ready for a human run; no current-head product-run result is claimed by this checklist.

## Record before starting

| Field | Observed value |
| --- | --- |
| Reviewed commit and package version | |
| macOS app executable hash and bundle ID | |
| Reviewer, platform, locale, UTC start/end | |
| OKX Demo profile and account ID (redacted) | |
| Source publication, Snapshot, Universe, research context IDs | |
| Feature Dataset, Factor Decision and Component IDs | |
| Model Artifact and qualified Component IDs | |
| Strategy Candidate, Backtest, Validation and Qualification IDs | |
| Bot, Bundle, Runtime Attempt and Experiment IDs | |
| Provider order, Fill and reconciliation evidence IDs | |
| Experiment Report, Feedback Snapshot/Report and Review Decision IDs | |

Use a package built from the reviewed commit. Verify the app bundle ID is `bid.adaq.desktop`. For local acceptance, build the debug app with `rtk pnpm tauri build --debug --features local-env-credentials --bundles app`; this credential path reads only the repository-root `.env` and does not access Keychain. Use only an OKX Demo connection. Never paste secrets into this record. Record failures and exact IDs as observed; do not reuse historical IDs as fresh evidence.

On 2026-09-28, a local `.env` based provider reconciliation succeeded without Keychain. A Desktop Stop and Flatten attempt on the old ETH Bot placed one Host-authorized OKX Demo market sell (local `order-3`, provider `3962337393260904448`). The provider returned two exact Fills totalling 11.035442 ETH at 2639.62 USDT. ADAQ initially rejected them because it applied the estimated bid as a sell limit; the local fix records them and resolves the matching provider uncertainty. After a fresh live reconciliation, the account was `reconciled`, execution was unblocked, and it held 1.2676717 ETH and 330.15288934 SOL. The market sell was an operator Flatten action, **not** a natural Strategy Target and therefore does not pass execution acceptance. The only saved Experiment Report predates these Fills.

The old ETH and SOL positions still block Experiment Launch's exact empty-account precondition. Stop any faulted Bot with Keep Position to release its instrument lease; handle the remaining Demo holdings using Host controls and exact provider evidence. If a quantity is below the configured minimum, record the unsellable dust and use an empty Demo account for the 48-hour experiment. Never edit the ledger or claim flatness by ignoring dust.

## Automated preflight recorded on 2026-09-28

| Check | Result |
| --- | --- |
| V1 Acceptance CI on committed `920515d` | macOS ARM64 and Windows x86_64 passed; predates the local reconciliation fix |
| Focused Host reconciliation test on the local working tree | passed; changed balance settles on the second fresh snapshot, while continuing change stays blocked |
| Paper Experiment Rust tests and Bot/Experiment/Feedback frontend tests | 13 Rust tests and 11 frontend tests passed |
| Rust check with `local-env-credentials`, Rust formatting, diff whitespace | passed |
| Actual local OKX Demo reconciliation using repository `.env` | passed; account persisted as `reconciled`; no Keychain or order submission |
| Desktop BTC Bot Start without a prior Reconcile click | passed at 2026-09-28 10:10 UTC on the local working tree; Attempt `3b67f2f8-5f4e-4140-a4b6-8e8900860823` recorded `account-reconciled`, Worker heartbeat and `warmup-started`, then the dev app restart faulted it; no warmup completion, natural Target or Fill claimed |
| Desktop BTC Bot Retry in en-US without a prior Reconcile click | passed after the final code edit; Attempt `e118e583-c338-4ca3-9c90-1d549d3be682` entered Running with fresh `account-reconciled`, heartbeat and decisions. At the 2026-09-28 observation cut there were 13 `no-target` decisions and 0 Bot orders; `warmup-progress` remained active. Leave this as partial execution evidence, not a completed strategy-to-Fill sample. |
| Host-authorized Demo Flatten and provider Fill retention | two market-sell Fills from the same provider order recorded; post-fix reconciliation `reconciled`, execution unblocked, ETH/SOL positions remain |
| Full Rust library test | stopped after prolonged silence; no pass claimed |
| Current-head Strategy-to-Bot-to-natural-Fill-to-Feedback run | not demonstrated; existing Fills and feedback are from different incomplete evidence windows |
| Model-based canonical chain and fixed 48-hour experiment | not demonstrated; local Dashboard showed 0 Model Artifacts, and the Demo account retained ETH/SOL positions that block exact-flat Experiment Launch |

This preflight checks the repaired startup gate and real Demo connectivity. It is not the human Desktop acceptance below.

## Canonical V1 chain

The V1 scope in ADR 0090 includes Model research. Keep the following evidence linked by exact Snapshot, Universe, context, Component and qualification identities. A separate model-free EMA experiment cannot substitute for this chain.

1. **Data Foundation:** acquire BTC-USDT, ETH-USDT and SOL-USDT OKX Spot data. Inspect Source provenance, validation, canonicalization, gaps and quarantine. Publish the accepted Snapshot and Point-in-Time Universe. Record the publication IDs and any rejected rows.
2. **Feature Engineering:** select that Snapshot and Universe, freeze the research context, materialize the Feature Dataset, and record its hash and time range.
3. **Factor:** evaluate the Factor on the same context, record the promotion decision, then qualify and inspect its Component package and exact hash. A raw Candidate is insufficient.
4. **Model:** train, evaluate and qualify a Model Artifact using accepted Factor output. Record the evaluation window, final report, Model Component identity and package hash. Keep failed or unknown evidence states visible.
5. **Strategy:** build a Candidate from those exact accepted Factor and Model inputs. Inspect its Backtest and Validation reports, then record the Gate 12 eligible Strategy Qualification and all bound upstream IDs. Negative returns are a valid observed result.
6. **Paper Trading:** open `/paper-trading` and inspect the Demo account, cash, positions, orders and Fills. Record the initial reconciliation state. After a restart, verify it is `Reconciliation Required`; this blocks risk until the Host obtains fresh provider evidence.
7. **Bots and Operations:** deploy that exact Strategy Qualification to the Demo account and start the supervised Bot. Start must automatically fetch OKX Demo reconciliation from `.env` credentials, retry once if the first snapshot changed, and proceed only when the account is reconciled and quiet. Do not click Reconcile first merely to make Start pass. Record the reconciliation evidence, Bundle and current Attempt IDs, Worker heartbeat, Decision, Target or No Target reason, Risk decision, provider acknowledgement, order and natural Fill evidence. Check `/operations` and the System Dashboard for the same identities and any critical condition. A Running state alone is insufficient.
8. **Post-order reconciliation and feedback:** Reconcile after any order or Fill. In `/paper-feedback`, create the Snapshot and four lens Reports only for the current Attempt and a valid observation horizon. Record realized sample counts, evidence states, limitations and a human Research Review Decision. A non-ready report must remain non-directional.

The canonical chain passes only when current-head Desktop evidence connects these stages, includes a provider-acknowledged Demo Fill and usable post-order reconciliation, and the declared feedback horizon is satisfied. If the selected strategy emits no Target, record the reason and mark execution acceptance incomplete; do not force a trade or weaken Risk.

## Supervised three-instrument EMA experiment

This is additional execution evidence for the model-free EMA Double-Cross strategy in [ema-double-cross-demo.md](ema-double-cross-demo.md). It does not satisfy the Model step above.

1. In Strategy Lab, qualify the EMA Double-Cross strategy separately for BTC-USDT, ETH-USDT and SOL-USDT from current accepted 15-minute Snapshots and a common Universe. Verify the three qualifications are Gate 12 eligible and require no continuation.
2. Inspect the OKX Demo account. Experiment Launch must automatically reconcile and recheck available funds before using the previously accepted equal allocations: 32,694.76 USDT per instrument, with a 32,367.81 USDT initial entry cap and 326.95 USDT reserve. If current funds or precision rules cannot support them, stop and record the shortfall. Do not click Reconcile first merely to make Launch pass.
3. On `/paper-experiment`, select the usable Demo profile, its account and the three exact qualifications. Enter one common UTC start and end exactly 48 hours apart. Record both timestamps and create the Experiment.
4. Launch and inspect all three Bot identities. Verify each current Attempt reaches `warmup-complete`, the Trade Stream is connected, and the Experiment advances through Preparing and Armed to Running before risk is permitted. Keep the supervised Desktop session available; a fault needs explicit operator recovery and new warmup.
5. During the window, record Decisions, Risk decisions, orders, provider acknowledgements, natural Fills, fees, positions, valuations, interruptions and Operations health. Zero trades or negative returns are valid observations. Do not extend the fixed window to seek a Fill or profit.
6. At the deadline, confirm Stop with Keep Position, check all three Bots' terminal states and reconcile the Demo account. Create the immutable comparison Report and Paper Feedback. Record each instrument's net equity return after fees, realized and unrealized PnL separately, and any open position. Record the human Research Review Decision after inspecting report completeness.

The experiment's execution acceptance requires at least one natural provider Fill, post-order reconciliation and feedback satisfying its stated horizon. If any are absent, keep the report but mark the experiment incomplete and diagnose the missing stage. Profitability is a measured strategy result, not a workflow pass condition.

## Final review

- Repeat the complete visible route and failure-state check in `en-US` and `zh-CN`; record semantic, layout or accessibility differences. Switching locale must not change Host evidence.
- Record the current-head V1 Acceptance CI result for macOS ARM64 and Windows x86_64, plus the packaged Desktop run actually performed. CI and a running process do not count as a Desktop product run.
- Record the reviewer decision as **Ready** only for the scope demonstrated by current evidence. Otherwise record **Not Ready** with the first missing stage and its exact diagnostic. Do not infer readiness from closed issues, green CI, or historical account balances.
