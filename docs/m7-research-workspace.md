# M7 Research Workspace

Status: Approved design; implementation has not started.

## Goal

Turn the M1-M6 research loop into a usable desktop workflow without changing the established Component ABI, Indicator Engine, Backtest, or Validation domain rules.

## Confirmed scope

- Keep three research entries: Components, Backtest, and Validation. Market data remains on the home page.
- Component Library uses a list-and-detail layout. The list shows name, kind, version, compatibility, and Run-lock status; details show parameters, Feature Slots, Factor dependencies, Warmup, ABI/SDK/Manifest versions, and exact hashes. Import uses the native file picker. Deletion requires confirmation and explains references that prevent removal.
- Component source stays outside the desktop app and uses the existing `adaq-component new -> build -> verify` workflow. The app starts by importing a verified `.adaq` package.
- Backtest uses four stages on one page: Data, Strategy, Execution, and Results.
- The Data stage lists existing Snapshots matching the selected Instrument and Bar Interval with their range, Bar count, source, and ID. Users may reuse one or download and freeze a new Snapshot with progress and cancellation. M7 does not add Snapshot deletion or garbage collection.
- Backtest Results uses four tabs: Overview for metrics and equity, benchmark, and drawdown charts; Decisions for Target Decisions and Run Pauses; Execution for paged simulated orders, fills, and fees; and Provenance for the Snapshot, Packages, parameters, Indicator Plan, Execution Profile, engine identities, versions, and seed. Human-readable names lead, while exact IDs and hashes remain visible and copyable.
- Historical Backtest Runs remain read-only. `Use as new configuration` copies a Run's settings into the current ephemeral form, and any changed execution creates a new immutable Run. Persistent presets, parameter sweeps, and multi-Run comparison are outside M7.
- Validation uses one guided flow: choose a method, configure contexts, freeze a Protocol, run or resume it, then inspect or export its Report. Chronological holdout, walk-forward, and cross-market are presented as three method choices, and users select named artifacts instead of copying internal IDs.
- Validation Reports use three tabs: Summary for method-level aggregate evidence; Evidence for each window or market, failures, Run Pauses, and linked Backtest Runs; and Provenance for the Protocol, Runs, Packages, Plans, Snapshots, configurations, aggregation rules, and versions. JSON and Markdown exports are top-level actions. Recommended Contexts remain historical evidence and never claim a best or profitable future configuration.
- M7 does not add a background queue. Backtest and Validation operations expose an explicit busy state and prevent duplicate submission. A frozen Protocol remains available after failure or restart; Resume reuses completed immutable Runs. Pause, queueing, and parallel scheduling are outside M7.
- Errors appear beside the stage that produced them with a concise actionable explanation. Expandable technical details preserve the exact typed error code, cause, and related Slot, Component, or Run identity and provide copy-to-clipboard. Unknown errors are not rewritten as guesses, and partial Validation failures remain visible in Report evidence.
- M7 may add minimal query-oriented Tauri APIs required by the UI, such as listing reusable Snapshots. It does not add Paper Trading, Supervised Live Trading, or Marketplace capabilities.
- M7 does not add AI-generated Factor or Strategy source, automated parameter or strategy search, AI interpretation of Validation Reports, model-provider configuration, or research-data upload. A later Candidate Discovery milestone may generate Components and run them through the frozen research-validation workflow, subject to separate privacy and evidence-integrity decisions. Results are Validation-ranked Candidates, never claims of a best Factor or Strategy.
- Continue using `lightweight-charts` and `recharts` for market and quantitative charts and React/CSS for workflow navigation and provenance lists.
- Do not add `@antv/infographic` in M7. Reconsider it when ADAQ generates research reports or needs flow, relationship, or narrative infographics.
- Keep the M7 desktop UI in English and do not introduce partial application internationalization. README files, Component tutorials, and the final acceptance guide remain equivalent in English and Simplified Chinese.
- Keep the completed Tauri/Supabase authentication flow unchanged. The acceptance guide documents environment configuration and sign-in without storing real credentials; existing-account password sign-in is the primary path and first-time email OTP plus password setup is supplementary.
- Target desktop windows only: the workflow remains usable at 1024 px wide and is optimized for 1280 px and above. Tabs, forms, and lists are keyboard accessible with visible focus; state and chart meaning do not rely on color alone; dynamic progress and errors expose accessible status text.

## Required acceptance guide

After M7, provide a complete English and Simplified Chinese manual verification guide covering Factor and Strategy authoring from empty CLI-generated projects, `build`, `verify`, app import, market-data preparation, Backtest configuration and execution, result and provenance inspection, research validation, and report export. Committed examples are references and recovery aids, not substitutes for manually authoring the primary acceptance Components.

The implementation must pass focused Rust tests for new queries, user scoping, and reference state; focused Jest tests for independently testable workflow gates, artifact selection, and error formatting; `cargo test --workspace`; `cargo check --workspace`; `pnpm test`; and `pnpm run build`. M7 does not add a browser end-to-end test dependency.

At handoff, the agent walks the user through the manual guide one operation at a time, stating the exact action, expected result, and evidence to return when a step fails.

The canonical manual run targets macOS ARM64. The guide notes Windows command differences, while Windows execution remains covered by automated build and test gates rather than requiring the user to repeat the full manual flow on two systems.

## Delivery order

1. **M7.1 Artifact Queries and Workspace Shell**: add readable Snapshot, Run, Protocol, and Report queries, the three research entries, and shared workflow state.
2. **M7.2 Component Library Productization**: deliver list and detail views, compatibility and Run-lock status, import, and safe deletion.
3. **M7.3 Guided Backtest Workspace**: deliver the four-stage workflow, historical Run reuse, and four Results tabs.
4. **M7.4 Guided Validation Workspace**: deliver the three validation methods, resumable execution, three Report tabs, and exports.
5. **M7.5 Bilingual Manual Acceptance**: update both README files, author the complete from-empty-project manual guide in English and Simplified Chinese, and execute the final verification gates.
