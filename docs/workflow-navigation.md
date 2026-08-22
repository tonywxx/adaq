# Workflow Navigation

## Purpose

The authenticated application shell presents ADAQ as one research-to-paper workflow. A user opening the app should see the next honest action, understand which capabilities exist today, and reach every step without learning the underlying milestone structure.

For V1, this workflow is scoped to OKX Spot data and OKX Demo Paper execution under ADR 0090 and the [V1 Completion Recovery Map](./v1-completion-recovery-map.md). Retained A-share and U.S. equity routes are Post-V1 and must not appear as supported V1 steps or readiness claims.

## Home selection

- `/` shows the Workflow Guide when the user has no Operational Responsibility.
- `/` shows the Operations Dashboard when authoritative runtime evidence establishes Operational Responsibility.
- `/help/workflow` always opens the Workflow Guide.
- `/operations` always opens the Operations Dashboard.
- Until M15 introduces authoritative Bot Runtime records, the home condition resolves to the Workflow Guide. The UI must not invent a running Bot or simulated status.

Operational Responsibility is defined in `CONTEXT.md` and ADR-0059. It includes a non-terminal or faulted Bot Runtime Attempt, reconciliation work, retained Paper positions or non-terminal Paper orders, and active Warning or Critical alerts.

## Information architecture

The sidebar and guide use the same ordered structure:

1. Foundations
   - Markets & Data
   - Feature Engineering
2. Factor Research
   - 1 Discover Factors
   - 2 Evaluate & Promote Factors
   - 3 Qualify, Package & Import Factor
3. Model Research
   - 4 Train Model
   - 5 Evaluate Model
   - 6 Qualify Model Deployment
4. Strategy Validation
   - 7 Build Strategy
   - 8 Backtest, Validate & Qualify Strategy
5. Paper Operations
   - Operations Dashboard
   - 9 Prepare Paper Account & Deploy Bot
   - 10 Monitor, Diagnose & Review
6. Component Library
7. Help, Settings, GitHub, and WeChat

Existing workspace URLs remain stable. Planned steps open an inspectable guide detail instead of a dead route; implemented and partial steps expose a direct action to the existing workspace.

## Step contract

| Step | Capability | Primary output | Current entry |
| --- | --- | --- | --- |
| 1 Discover Factors | Available · M11 | Factor Candidate | `/factors` |
| 2 Evaluate & Promote Factors | Available · M11 | Factor Promotion Decision | `/factors` |
| 3 Qualify, Package & Import Factor | Partial | Qualified Factor Package + Component Meta | Component Library |
| 4 Train Model | Available · M12 | Validated Model Artifact | Models |
| 5 Evaluate Model | Partial | Forecast Evaluation Report | Models |
| 6 Qualify Model Deployment | Partial | Model Runtime Qualification Report | Models |
| 7 Build Strategy | Planned · M13 | Strategy Candidate | Guide detail |
| 8 Backtest, Validate & Qualify Strategy | Partial | Validation Report + Qualified Strategy Package | Backtest |
| 9 Prepare Paper Account & Deploy Bot | Planned · M15–M16 | Bot Deployment Bundle + first Running Attempt | Guide detail |
| 10 Monitor, Diagnose & Review | Planned · M17–M18 | Events, alerts, paper feedback, and a Research Review Decision | Guide detail |

Step 6 preserves both deployment paths: Portable models are packaged and imported; Local Qlib Paper models receive runtime qualification without a portable package. Step 10 reports operational health rather than a permanent completion state, and review creates new research attempts rather than mutating a running bundle.

## Status language

Capability state describes product availability:

- Available
- Partial
- Planned · Mxx

Workflow state describes user evidence only when authoritative data exists:

- Not started
- In progress
- Needs review
- Blocked
- Complete

Step 10 instead uses Healthy, Degraded, Critical, or Unknown. A Partial capability may show existing evidence, but must not imply that a future step is complete. No global workflow project or progress record is introduced for this navigation change.

## Product readiness visibility

Product readiness is recorded as an immutable `Readiness Assertion` for a declared Capability, Journey, Market/Data Context, Supported Platform, and Interface Locale scope. It is not a global green flag and does not create a runtime record or grant permission. The Guide exposes the assertion's declared scope, known limitations, blockers, evidence entry points, and next honest action beside the existing Capability State and user-scoped Step State. A Ready assertion means only `Ready for declared scope`; any in-scope change requires a new reviewed assertion.

The complete acceptance journey must not contain a hidden manual edit of SQLite, Parquet, configuration, Python output, or an external script. Cancellation, failure, restart, recovery, evidence inspection, localization, keyboard/assistive access, and supported-platform behavior are part of the same gate rather than optional polish. The final V1 acceptance set covers the OKX Crypto Paper journey and the declared missing-data, provider-disconnect, clock-skew, Worker-crash, uncertain-order, credential-rotation, and restart-reconciliation failures.

## Workflow Guide

The page paints its semantic content immediately. Its order is:

1. Title, short orientation, and one recommended primary action.
2. Compact Foundations cards.
3. A full-width `@antv/infographic` overview of the four modules and ten steps.
4. A keyboard-accessible semantic step list that remains available if the graphic is loading or unavailable.
5. Step details in a right sheet on desktop and bottom sheet on mobile.
6. Honest Recent Work and Active Blockers empty states until reliable evidence is available.

The infographic is presentation, not the only navigation surface. Selecting a module or step opens details; navigation occurs through an explicit Open or Continue action. AntV is loaded after the first paint so it never blocks the shell or semantic guide.

## Visual and responsive behavior

- Preserve the existing Geist, shadcn/Base UI, titlebar, sidebar, and color tokens.
- Use a restrained research-workbench layout with four accessible module accents.
- Desktop presents modules left to right; compact layouts use a vertical semantic sequence.
- The guide explicitly shows the feedback path from monitoring to Factor, Model, and Strategy research.
- Group expansion is session-local. The active workflow module opens automatically; users may open or close other modules.
- Every state uses text as well as color. Focus, labels, headings, and tap targets remain keyboard and screen-reader accessible.

## Recommendation order

The home action follows this priority once reliable evidence is connected:

1. Missing readable Snapshot: open Markets & Data.
2. Missing completed Feature Dataset: open Feature Engineering.
3. Blocked or Needs Review work: open that evidence.
4. One recent incomplete lineage: continue it.
5. Multiple incomplete lineages: require Choose Work.
6. No research evidence: start at step 1.

Failed history does not permanently replace the next useful action. When multiple lineages exist, the UI never silently selects one.

## Delivery boundary

This navigation slice adds no Bot persistence, Tauri command, database table, workflow project record, or synthetic status. M15–M18 will supply the runtime evidence that activates Operations home and enriches steps 9–10.
