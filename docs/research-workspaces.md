# Research Workspaces

The Components, Backtest, and Validation workspaces turn the research loop into guided, auditable desktop workflows over immutable local evidence, without changing the established Component ABI, Indicator Engine, Backtest, or Validation domain rules.

## Workspace contracts

- Three research entries: Components, Backtest, and Validation. Market data remains on the Markets workspaces.
- Component Library uses a list-and-detail layout. The list shows name, kind, version, compatibility, and Run-lock status; details show parameters, Feature Slots, Factor dependencies, Warmup, ABI/SDK/Manifest versions, and exact hashes. Import uses the native file picker. Deletion requires confirmation and explains references that prevent removal.
- Component source stays outside the desktop app and uses the existing `adaq-component new -> build -> verify` workflow. The app starts by importing a verified `.adaq` package.
- Backtest uses four stages on one page: Data, Strategy, Execution, and Results.
- The Data stage lists existing Snapshots matching the selected Instrument and Bar Interval with their range, Bar count, source, and ID. Users may reuse one or download and freeze a new Snapshot with progress and cancellation.
- Backtest Results uses four tabs: Overview for metrics and equity, benchmark, and drawdown charts; Decisions for Target Decisions and Run Pauses; Execution for paged simulated orders, fills, and fees; and Provenance for the Snapshot, Packages, parameters, Indicator Plan, Execution Profile, engine identities, versions, and seed. Human-readable names lead, while exact IDs and hashes remain visible and copyable.
- Historical Backtest Runs remain read-only. `Use as new configuration` copies a Run's settings into the current ephemeral form, and any changed execution creates a new immutable Run. Persistent presets, parameter sweeps, and multi-Run comparison are outside the current scope.
- Validation uses one guided flow: choose a method, configure contexts, freeze a Protocol, run or resume it, then inspect or export its Report. Chronological holdout, walk-forward, and cross-market are presented as three method choices, and users select named artifacts instead of copying internal IDs.
- Validation Reports use three tabs: Summary for method-level aggregate evidence; Evidence for each window or market, failures, Run Pauses, and linked Backtest Runs; and Provenance for the Protocol, Runs, Packages, Plans, Snapshots, configurations, aggregation rules, and versions. JSON and Markdown exports are top-level actions. Recommended Contexts remain historical evidence and never claim a best or profitable future configuration.
- No background queue in the research workspaces: Backtest and Validation operations expose an explicit busy state and prevent duplicate submission. A frozen Protocol remains available after failure or restart; Resume reuses completed immutable Runs. Pause, queueing, and parallel scheduling are outside the current scope.
- Errors appear beside the stage that produced them with a concise actionable explanation. Expandable technical details preserve the exact typed error code, cause, and related Slot, Component, or Run identity and provide copy-to-clipboard. Unknown errors are not rewritten as guesses, and partial Validation failures remain visible in Report evidence.
- Minimal query-oriented Tauri APIs, such as listing reusable Snapshots, back the UI. No Paper Trading, Supervised Live Trading, or Marketplace capability is exposed from these workspaces.
- No AI-generated Factor or Strategy source, automated parameter or strategy search, AI interpretation of Validation Reports, model-provider configuration, or research-data upload. Results are Validation-ranked Candidates, never claims of a best Factor or Strategy.
- Target desktop windows only: the workflow remains usable at 1024 px wide and is optimized for 1280 px and above. Tabs, forms, and lists are keyboard accessible with visible focus; state and chart meaning do not rely on color alone; dynamic progress and errors expose accessible status text.
- The completed Tauri/Supabase authentication flow is unchanged. The guides document environment configuration and sign-in without storing real credentials; existing-account password sign-in is the primary path and first-time email OTP plus password setup is supplementary.

## Verification

The canonical manual verification path (macOS ARM64; Windows uses PowerShell with `.\` paths) walks, in order:

1. **Prerequisites and sign-in** — toolchain plus CLI install (`rustup toolchain install stable`, `rustup target add --toolchain stable wasm32-unknown-unknown`, `cargo install cargo-component --locked`, `cargo install --force --path src-tauri/crates/adaq-component-tooling`), `VITE_SUPABASE_URL` / `VITE_SUPABASE_PUBLISHABLE_KEY` supplied outside version control, `pnpm tauri dev`, and password sign-in.
2. **Author and verify empty Components** — `adaq-component new factor ...` / `adaq-component new strategy ...` from empty projects (the committed [examples](../examples/components/README.md) are references, not substitutes), point the generated `Cargo.toml` at the local SDK path, implement the Factor and Strategy, complete both Manifests, then `adaq-component build` and `adaq-component verify dist/*.adaq`, recording the archive hashes.
3. **Import and audit Components** — import both packages, review identities, compatibility, dependencies, and hashes in the Component Library.
4. **Freeze data and execute a Backtest** — select Instrument/interval, reuse or freeze a Snapshot, bind the Factor dependency, run, and inspect all four Results tabs; verify `Use as new configuration` leaves the Run immutable.
5. **Validate and export reports** — freeze `chronological-holdout@1`, `walk-forward@1`, and `cross-market@1` Protocols, inspect Summary/Evidence/Provenance, and export JSON and Markdown.

Automated gates:

```sh
(
  cd src-tauri
  cargo test --workspace
  cargo check --workspace
)
pnpm test
pnpm run build
```

On failure at any step, capture the exact command, complete output, and affected identities (Snapshot, Run, Protocol, Report, package hashes) with credentials, OTPs, tokens, and Supabase values redacted. A completed Report is historical evidence, not a best Strategy, a profitability claim, Paper Trading, or Live Trading.
