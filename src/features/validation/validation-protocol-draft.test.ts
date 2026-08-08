import {
	createValidationProtocolDraft,
	inspectValidationProtocolDraft,
	transitionValidationProtocolDraft,
	type ValidationSourceRun,
} from "./validation-protocol-draft";

const snapshot = (snapshotId: string) => ({
	snapshotId,
	src: "binance",
	code: snapshotId.toUpperCase(),
	interval: "1h",
	startTimeMs: 0,
	endTimeMs: 20 * 3_600_000,
	barCount: 20,
});

const sourceRun = (runId: string, snapshotId = runId): ValidationSourceRun => ({
	runId,
	snapshot: snapshot(snapshotId),
	bars: Array.from({ length: 20 }, (_, index) => ({
		openTimeMs: index * 3_600_000,
	})),
	provenance: {
		normalizedRequest: {
			snapshotId,
			runStartTimeMs: 0,
			runEndTimeMs: 20 * 3_600_000,
			strategyArchiveSha256: "strategy",
			strategyParameters: { period: "20" },
			factorInstances: [
				{
					alias: "trend",
					archiveSha256: "factor",
					parameters: [{ name: "length", value: "10" }],
				},
			],
			signalInstances: [],
			initialQuoteAllocation: "100",
			executionProfile: {
				makerFeeRate: "0.0008",
				takerFeeRate: "0.001",
				adverseSlippageRate: "0.0005",
				rebalanceThreshold: "0",
				priceIncrement: "0.1",
				quantityIncrement: "0.00000001",
				minimumQuantity: "0.00001",
				riskFreeRate: "0",
				fillPolicy: "taker",
			},
			seed: 7,
		},
	},
});

function ok<T>(result: { ok: true; value: T } | { ok: false; error: unknown }) {
	if (!result.ok) throw new Error(JSON.stringify(result.error));
	return result.value;
}

test("materializes a complete chronological holdout through one public seam", () => {
	let session = createValidationProtocolDraft();
	const load = ok(
		transitionValidationProtocolDraft(session, {
			type: "select-source",
			runId: "run-1",
		}),
	);
	expect(load.effect).toEqual({
		kind: "load-source",
		revision: 1,
		runId: "run-1",
	});
	session = load.session;
	session = ok(
		transitionValidationProtocolDraft(session, {
			type: "accept-source",
			revision: 1,
			run: sourceRun("run-1"),
		}),
	).session;
	session = ok(
		transitionValidationProtocolDraft(session, {
			type: "set-holdout-boundary",
			value: "1970-01-01T10:00Z",
		}),
	).session;
	const freeze = ok(
		transitionValidationProtocolDraft(session, {
			type: "request-freeze",
			userId: "alice",
		}),
	);
	expect(freeze.effect).toMatchObject({
		kind: "freeze",
		revision: session.draft.revision,
		request: {
			userId: "alice",
			methodVersion: "chronological-holdout@1",
			windows: [
				{
					snapshotId: "run-1",
					sampleOutStartTimeMs: 36_000_000,
				},
			],
			run: {
				userId: "alice",
				snapshotId: "run-1",
				factorInstances: [
					{
						alias: "trend",
						parameters: { length: "10" },
					},
				],
			},
		},
	});
});

test("switching methods resets only method-specific fields and keeps the source", () => {
	let session = createValidationProtocolDraft();
	session = ok(
		transitionValidationProtocolDraft(session, {
			type: "select-source",
			runId: "run-1",
		}),
	).session;
	session = ok(
		transitionValidationProtocolDraft(session, {
			type: "accept-source",
			revision: 1,
			run: sourceRun("run-1"),
		}),
	).session;
	session = ok(
		transitionValidationProtocolDraft(session, {
			type: "select-method",
			method: "walk-forward",
		}),
	).session;
	session = ok(
		transitionValidationProtocolDraft(session, {
			type: "set-walk-forward-field",
			field: "windowSizeBars",
			value: "5",
		}),
	).session;
	const switched = ok(
		transitionValidationProtocolDraft(session, {
			type: "select-method",
			method: "cross-market",
		}),
	).session;
	expect(switched.draft).toMatchObject({
		kind: "cross-market",
		contexts: [],
		source: { runId: "run-1" },
	});
	const noOp = ok(
		transitionValidationProtocolDraft(switched, {
			type: "select-method",
			method: "cross-market",
		}),
	).session;
	expect(noOp.draft.revision).toBe(switched.draft.revision);
});

test("ignores a stale Source response and clears holdout boundary on Source change", () => {
	let session = createValidationProtocolDraft();
	const first = ok(
		transitionValidationProtocolDraft(session, {
			type: "select-source",
			runId: "run-a",
		}),
	);
	session = first.session;
	const second = ok(
		transitionValidationProtocolDraft(session, {
			type: "select-source",
			runId: "run-b",
		}),
	);
	session = second.session;
	const stale = transitionValidationProtocolDraft(session, {
		type: "accept-source",
		revision: first.effect?.revision ?? -1,
		run: sourceRun("run-a"),
	});
	expect(stale).toMatchObject({ ok: true, value: { ignored: "stale-source" } });
	session = ok(
		transitionValidationProtocolDraft(session, {
			type: "accept-source",
			revision: second.effect?.revision ?? -1,
			run: sourceRun("run-b"),
		}),
	).session;
	session = ok(
		transitionValidationProtocolDraft(session, {
			type: "set-holdout-boundary",
			value: "1970-01-01T10:00Z",
		}),
	).session;
	session = ok(
		transitionValidationProtocolDraft(session, {
			type: "select-source",
			runId: "run-c",
		}),
	).session;
	expect(session.draft).toMatchObject({
		kind: "chronological-holdout",
		sampleOutStart: "",
	});
});

test("rejects stale cross-market overrides after the context is removed", () => {
	let session = createValidationProtocolDraft({ method: "cross-market" });
	session = ok(
		transitionValidationProtocolDraft(session, {
			type: "add-cross-market-context",
			snapshot: snapshot("btc"),
		}),
	).session;
	const load = ok(
		transitionValidationProtocolDraft(session, {
			type: "request-cross-market-override",
			snapshotId: "btc",
			runId: "override-btc",
		}),
	);
	session = ok(
		transitionValidationProtocolDraft(load.session, {
			type: "remove-cross-market-context",
			snapshotId: "btc",
		}),
	).session;
	const stale = transitionValidationProtocolDraft(session, {
		type: "accept-cross-market-override",
		revision: load.effect?.revision ?? -1,
		snapshotId: "btc",
		run: sourceRun("override-btc", "btc"),
	});
	expect(stale).toMatchObject({
		ok: true,
		value: { ignored: "stale-override" },
	});
});

test("clears pending overrides when the shared source changes", () => {
	let session = createValidationProtocolDraft({ method: "cross-market" });
	session = ok(
		transitionValidationProtocolDraft(session, {
			type: "add-cross-market-context",
			snapshot: snapshot("btc"),
		}),
	).session;
	const pendingOverride = ok(
		transitionValidationProtocolDraft(session, {
			type: "request-cross-market-override",
			snapshotId: "btc",
			runId: "override-btc",
		}),
	);
	const nextSource = ok(
		transitionValidationProtocolDraft(pendingOverride.session, {
			type: "select-source",
			runId: "run-2",
		}),
	).session;
	expect(nextSource.draft).toMatchObject({
		kind: "cross-market",
		contexts: [{ snapshot: { snapshotId: "btc" } }],
	});
	expect(nextSource.sourceLoad).toMatchObject({ runId: "run-2" });
	const stale = transitionValidationProtocolDraft(nextSource, {
		type: "accept-cross-market-override",
		revision: pendingOverride.effect?.revision ?? -1,
		snapshotId: "btc",
		run: sourceRun("override-btc", "btc"),
	});
	expect(stale).toMatchObject({
		ok: true,
		value: { ignored: "stale-override" },
	});
});

test("reports typed local errors and an advisory walk-forward preview", () => {
	let session = createValidationProtocolDraft({ method: "walk-forward" });
	const invalid = inspectValidationProtocolDraft(session);
	expect(invalid.errors[0]).toEqual({
		kind: "incomplete-draft",
		fields: ["source"],
	});
	session = ok(
		transitionValidationProtocolDraft(session, {
			type: "select-source",
			runId: "run-1",
		}),
	).session;
	session = ok(
		transitionValidationProtocolDraft(session, {
			type: "accept-source",
			revision: 1,
			run: sourceRun("run-1"),
		}),
	).session;
	session = ok(
		transitionValidationProtocolDraft(session, {
			type: "set-walk-forward-field",
			field: "windowSizeBars",
			value: "5",
		}),
	).session;
	session = ok(
		transitionValidationProtocolDraft(session, {
			type: "set-walk-forward-field",
			field: "stepSizeBars",
			value: "5",
		}),
	).session;
	session = ok(
		transitionValidationProtocolDraft(session, {
			type: "set-walk-forward-field",
			field: "minimumHistoryBars",
			value: "10",
		}),
	).session;
	const inspection = inspectValidationProtocolDraft(session, {
		sourceRunId: "run-1",
		bars: sourceRun("run-1").bars,
	});
	expect(inspection.errors).toEqual([]);
	expect(inspection.preview).toEqual({
		windows: [
			{ sampleOutStartTimeMs: 10 * 3_600_000, sampleOutEndTimeMs: 15 * 3_600_000 },
			{ sampleOutStartTimeMs: 15 * 3_600_000, sampleOutEndTimeMs: undefined },
		],
		partialFinalWindow: false,
	});
	const invalidValue = ok(
		transitionValidationProtocolDraft(session, {
			type: "set-walk-forward-field",
			field: "stepSizeBars",
			value: "0",
		}),
	).session;
	expect(inspectValidationProtocolDraft(invalidValue).errors).toContainEqual({
		kind: "invalid-value",
		field: "walkForward",
		reason: "not-positive-integer",
	});
});

test("does not accept a Freeze result after the Draft revision changes", () => {
	let session = createValidationProtocolDraft();
	session = ok(
		transitionValidationProtocolDraft(session, {
			type: "select-source",
			runId: "run-1",
		}),
	).session;
	session = ok(
		transitionValidationProtocolDraft(session, {
			type: "accept-source",
			revision: 1,
			run: sourceRun("run-1"),
		}),
	).session;
	session = ok(
		transitionValidationProtocolDraft(session, {
			type: "set-holdout-boundary",
			value: "1970-01-01T10:00Z",
		}),
	).session;
	const pending = ok(
		transitionValidationProtocolDraft(session, {
			type: "request-freeze",
			userId: "alice",
		}),
	);
	const edited = ok(
		transitionValidationProtocolDraft(pending.session, {
			type: "set-holdout-boundary",
			value: "1970-01-01T11:00Z",
		}),
	).session;
	const stale = transitionValidationProtocolDraft(edited, {
		type: "accept-freeze",
		revision: pending.effect?.revision ?? -1,
		protocolId: "protocol-old",
	});
	expect(stale).toMatchObject({ ok: true, value: { ignored: "stale-freeze" } });
});
