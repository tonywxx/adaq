import type { LibraryComponent } from "@/features/components/component-library";
import {
	createBacktestDraft,
	defaultExecutionProfile,
	transitionBacktestDraft,
	type BacktestPreflight,
	type BacktestDraftSession,
	type DraftCommand,
	type SignalCandidate,
} from "./backtest-run-draft";

const input = {
	kind: "empty" as const,
	selectedInstrumentKey: "okx:BTC-USDT",
	interval: "1h" as const,
	start: "2026-01-01",
	end: "2026-01-31",
	defaultExecutionProfile,
	defaultInitialQuoteAllocation: "10000",
	defaultSeed: "0",
};

const component = (overrides: Partial<LibraryComponent>): LibraryComponent => ({
	componentId: "component-id",
	version: "1.0.0",
	manifestSchemaVersion: "1.0.0",
	sdkVersion: "1.0.0",
	abiVersion: "1.0.0",
	name: "Component",
	kind: "factor",
	archiveSha256: "a".repeat(64),
	wasmSha256: "b".repeat(64),
	parameters: [],
	featureSlots: [],
	outputNames: [],
	dependencies: [],
	warmupBars: 0,
	compatible: true,
	lockedByRunIds: [],
	...overrides,
});

const strategy = component({
	kind: "strategy",
	archiveSha256: "s".repeat(64),
	dependencies: [{ componentId: "factor", version: "1.0.0", alias: "trend" }],
	featureSlots: [{ name: "forecast", source: { kind: "signal" } }],
});

const factorA = component({ archiveSha256: "f".repeat(64) });
const factorB = component({ archiveSha256: "g".repeat(64) });
const candidate: SignalCandidate = {
	slot: "forecast",
	datasetId: "dataset",
	signalName: "up",
	evidenceState: "verified",
};

function transition(session: BacktestDraftSession, command: DraftCommand) {
	const result = transitionBacktestDraft(session, command);
	if (!result.ok) throw new Error(result.error.kind);
	return result.value;
}

test("creates one editable Draft and keeps no-op edits at the same revision", () => {
	const created = createBacktestDraft(input);
	if (!created.ok) throw new Error(created.error.kind);
	const session = created.value;

	const result = transitionBacktestDraft(session, {
		type: "set-seed",
		value: "0",
	});

	expect(result).toEqual({ ok: true, value: { session } });
	expect(session.draft.revision).toBe(0);
});

test("applies dependency-aware retention and clears Factor parameters on package change", () => {
	const created = createBacktestDraft(input);
	if (!created.ok) throw new Error(created.error.kind);
	let session = created.value;
	session = transition(session, {
		type: "select-snapshot",
		snapshot: { snapshotId: "snapshot", startTimeMs: 1, endTimeMs: 100 },
	}).session;
	session = transition(session, { type: "select-strategy", strategy }).session;
	session = transition(session, {
		type: "select-factor",
		alias: "trend",
		archiveSha256: factorA.archiveSha256,
		compatibleHashes: [factorA.archiveSha256, factorB.archiveSha256],
	}).session;
	session = transition(session, {
		type: "set-factor-parameter",
		alias: "trend",
		name: "length",
		value: "20",
	}).session;
	session = transition(session, {
		type: "select-signal",
		slot: "forecast",
		candidate,
		compatibleCandidates: [candidate],
	}).session;
	session = transition(session, {
		type: "set-run-window",
		value: { startTimeMs: 10, endTimeMs: 90 },
	}).session;

	const changedFactor = transition(session, {
		type: "select-factor",
		alias: "trend",
		archiveSha256: factorB.archiveSha256,
		compatibleHashes: [factorA.archiveSha256, factorB.archiveSha256],
	}).session;
	expect(changedFactor.draft.factorSelections.trend).toBe(factorB.archiveSha256);
	expect(changedFactor.draft.factorParameters.trend).toBeUndefined();
	expect(changedFactor.draft.signalSelections.forecast).toEqual({
		datasetId: "dataset",
		signalName: "up",
	});
	expect(changedFactor.draft.runWindow).toEqual({
		startTimeMs: 10,
		endTimeMs: 90,
	});

	const changedSnapshot = transition(changedFactor, {
		type: "select-snapshot",
		snapshot: { snapshotId: "other", startTimeMs: 2, endTimeMs: 100 },
	}).session;
	expect(changedSnapshot.draft.runWindow).toBeUndefined();
	expect(changedSnapshot.draft.signalSelections).toEqual({});
	expect(changedSnapshot.draft.strategy?.archiveSha256).toBe(
		strategy.archiveSha256,
	);
	expect(changedSnapshot.draft.factorSelections.trend).toBe(
		factorB.archiveSha256,
	);

	const changedStrategy = transition(changedSnapshot, {
		type: "select-strategy",
		strategy: component({ kind: "strategy", archiveSha256: "t".repeat(64) }),
	}).session;
	expect(changedStrategy.draft.strategyParameters).toEqual({});
	expect(changedStrategy.draft.factorSelections).toEqual({});
	expect(changedStrategy.draft.signalSelections).toEqual({});
});

test("reconciliation removes pruned Factor parameters with the selection", () => {
	const created = createBacktestDraft(input);
	if (!created.ok) throw new Error(created.error.kind);
	let session = created.value;
	session = transition(session, {
		type: "select-strategy",
		strategy,
	}).session;
	session = transition(session, {
		type: "select-factor",
		alias: "trend",
		archiveSha256: factorA.archiveSha256,
		compatibleHashes: [factorA.archiveSha256],
	}).session;
	session = transition(session, {
		type: "set-factor-parameter",
		alias: "trend",
		name: "length",
		value: "20",
	}).session;

	const reconciled = transitionBacktestDraft(session, {
		type: "reconcile-factor-compatibility",
		strategyArchiveSha256: strategy.archiveSha256,
		compatibleHashes: {},
	});
	if (!reconciled.ok) throw new Error(reconciled.error.kind);
	expect(reconciled.value.session.draft.factorSelections).toEqual({});
	expect(reconciled.value.session.draft.factorParameters).toEqual({});
});

test("binds preflight and run effects to the current Draft revision", () => {
	const created = createBacktestDraft(input);
	if (!created.ok) throw new Error(created.error.kind);
	let session = created.value;
	session = transition(session, {
		type: "select-snapshot",
		snapshot: { snapshotId: "snapshot", startTimeMs: 1, endTimeMs: 100 },
	}).session;
	session = transition(session, { type: "select-strategy", strategy }).session;
	session = transition(session, {
		type: "select-factor",
		alias: "trend",
		archiveSha256: factorA.archiveSha256,
		compatibleHashes: [factorA.archiveSha256],
	}).session;
	session = transition(session, {
		type: "select-signal",
		slot: "forecast",
		candidate,
		compatibleCandidates: [candidate],
	}).session;

	const pending = transition(session, {
		type: "enter-stage",
		stage: "execution",
		userId: "user",
	});
	expect(pending.effect?.kind).toBe("preflight");
	if (pending.effect?.kind !== "preflight")
		throw new Error("missing preflight effect");
	expect(pending.effect.request.userId).toBe("user");
	session = pending.session;

	const stale = transitionBacktestDraft(session, {
		type: "accept-preflight",
		revision: pending.effect.revision - 1,
		preflight: {} as BacktestPreflight,
	});
	if (!stale.ok) throw new Error(stale.error.kind);
	expect(stale.value.ignored).toBe("stale-preflight");

	const preflight: BacktestPreflight = {
		runId: "run",
		reusesExistingRun: false,
		snapshot: { snapshotId: "snapshot", startTimeMs: 1, endTimeMs: 100 },
		normalizedRequest: {},
		featurePlan: {},
		componentLock: [],
		datasetLock: [],
		architecture: "composed",
	};
	session = transition(session, {
		type: "accept-preflight",
		revision: pending.effect.revision,
		preflight,
	}).session;
	const ready = transition(session, {
		type: "enter-stage",
		stage: "execution",
		userId: "user",
	});
	expect(ready.effect).toBeUndefined();
	const run = transition(session, { type: "request-run", userId: "user" });
	expect(run.effect?.kind).toBe("run");

	const edited = transition(session, { type: "set-seed", value: "1" }).session;
	const blocked = transitionBacktestDraft(edited, {
		type: "request-run",
		userId: "user",
	});
	if (blocked.ok) throw new Error("stale preflight was accepted");
	expect(blocked.error.kind).toBe("preflight-required");
});

test("restores a complete Draft from immutable provenance and rejects incomplete provenance", () => {
	const restored = createBacktestDraft({
		kind: "from-run-provenance",
		selectedInstrumentKey: "okx:BTC-USDT",
		interval: "1h",
		start: "2026-01-01",
		end: "2026-01-31",
		snapshot: { snapshotId: "snapshot", startTimeMs: 1, endTimeMs: 100 },
		strategy,
		normalizedRequest: {
			snapshotId: "snapshot",
			runStartTimeMs: 10,
			runEndTimeMs: 90,
			strategyArchiveSha256: strategy.archiveSha256,
			strategyParameters: { period: "20" },
			factorInstances: [
				{
					alias: "trend",
					archiveSha256: factorA.archiveSha256,
					parameters: [{ name: "length", value: "10" }],
				},
			],
			signalInstances: [
				{ slot: "forecast", datasetId: "dataset", signalName: "up" },
			],
			initialQuoteAllocation: "10000",
			executionProfile: defaultExecutionProfile,
			seed: 7,
		},
	});
	if (!restored.ok) throw new Error(restored.error.kind);
	expect(restored.value.stage).toBe("strategy");
	expect(restored.value.draft.revision).toBe(0);
	expect(restored.value.draft.factorParameters.trend).toEqual({ length: "10" });
	expect(restored.value.draft.signalSelections.forecast).toEqual({
		datasetId: "dataset",
		signalName: "up",
	});

	const incomplete = createBacktestDraft({
		kind: "from-run-provenance",
		selectedInstrumentKey: "okx:BTC-USDT",
		interval: "1h",
		start: "2026-01-01",
		end: "2026-01-31",
		snapshot: { snapshotId: "snapshot", startTimeMs: 1, endTimeMs: 100 },
		strategy,
		normalizedRequest: { strategyArchiveSha256: "wrong" } as never,
	});
	if (incomplete.ok) throw new Error("incomplete provenance was accepted");
	expect(incomplete.error.kind).toBe("incomplete-provenance");

	const missingDependency = createBacktestDraft({
		kind: "from-run-provenance",
		selectedInstrumentKey: "okx:BTC-USDT",
		interval: "1h",
		start: "2026-01-01",
		end: "2026-01-31",
		snapshot: { snapshotId: "snapshot", startTimeMs: 1, endTimeMs: 100 },
		strategy,
		normalizedRequest: {
			snapshotId: "snapshot",
			runStartTimeMs: 10,
			runEndTimeMs: 90,
			strategyArchiveSha256: strategy.archiveSha256,
			strategyParameters: { period: "20" },
			factorInstances: [],
			signalInstances: [],
			initialQuoteAllocation: "10000",
			executionProfile: defaultExecutionProfile,
			seed: 7,
		} as never,
	});
	if (missingDependency.ok) throw new Error("missing dependency was accepted");
	expect(missingDependency.error.kind).toBe("incomplete-provenance");
});
