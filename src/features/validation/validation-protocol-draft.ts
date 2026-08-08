import type { NormalizedRunConfiguration } from "@/features/backtest/backtest-run-draft";

export type ValidationMethod =
	| "chronological-holdout"
	| "walk-forward"
	| "cross-market";

export type ValidationSnapshot = Readonly<{
	snapshotId: string;
	src: string;
	code: string;
	interval: string;
	startTimeMs: number;
	endTimeMs: number;
	barCount: number;
	gaps?: readonly { startTimeMs: number; endTimeMs: number }[];
}>;

export type ValidationSourceRun = Readonly<{
	runId: string;
	snapshot: ValidationSnapshot;
	bars: readonly { openTimeMs: number }[];
	provenance?: Readonly<{
		normalizedRequest: NormalizedRunConfiguration;
	}>;
}>;

export type ValidationRunRequest = Readonly<{
	userId: string;
	snapshotId: string;
	runStartTimeMs?: number;
	runEndTimeMs?: number;
	factorInstances: Array<{
		alias: string;
		archiveSha256: string;
		parameters: Record<string, string>;
	}>;
	signalInstances: Array<{
		slot: string;
		datasetId: string;
		signalName: string;
	}>;
	strategyArchiveSha256: string;
	strategyParameters: Record<string, string>;
	initialQuoteAllocation: string;
	executionProfile: NormalizedRunConfiguration["executionProfile"];
	seed: number;
}>;

export type ValidationProtocolCreateRequest = Readonly<{
	userId: string;
	run: ValidationRunRequest;
	windows: Array<{
		snapshotId: string;
		sampleOutStartTimeMs: number;
		sampleOutEndTimeMs?: number;
	}>;
	walkForward?: {
		snapshotId: string;
		windowSizeBars: number;
		stepSizeBars: number;
		minimumHistoryBars: number;
	};
	crossMarket?: {
		contexts: Array<{
			snapshotId: string;
			runOverride?: ValidationRunRequest;
		}>;
	};
	methodVersion: `${ValidationMethod}@${number}` | string;
	aggregationRuleVersion: string;
}>;

export type WalkForwardConfiguration = Readonly<{
	windowSizeBars: string;
	stepSizeBars: string;
	minimumHistoryBars: string;
}>;

export type WalkForwardPreview = Readonly<{
	windows: Array<{
		sampleOutStartTimeMs: number;
		sampleOutEndTimeMs?: number;
	}>;
	partialFinalWindow: boolean;
}>;

type SourceBinding = Readonly<{
	runId: string;
	snapshot: ValidationSnapshot;
	normalizedRequest: NormalizedRunConfiguration;
}>;

export type ValidationOverride = Readonly<
	| { status: "pending"; runId: string }
	| {
			status: "ready";
			runId: string;
			normalizedRequest: NormalizedRunConfiguration;
	  }
>;

export type ValidationContext = Readonly<{
	snapshot: ValidationSnapshot;
	override?: ValidationOverride;
}>;

export type ValidationProtocolDraft = Readonly<
	| {
			kind: "chronological-holdout";
			revision: number;
			source?: SourceBinding;
			sampleOutStart: string;
	  }
	| {
			kind: "walk-forward";
			revision: number;
			source?: SourceBinding;
			windowSizeBars: string;
			stepSizeBars: string;
			minimumHistoryBars: string;
	  }
	| {
			kind: "cross-market";
			revision: number;
			source?: SourceBinding;
			contexts: readonly ValidationContext[];
	  }
>;

type FreezeState = Readonly<
	| { status: "absent" }
	| { status: "pending"; revision: number }
	| { status: "succeeded"; revision: number; protocolId: string }
>;

type SourceLoadState = Readonly<
	{ status: "idle" } | { status: "pending"; revision: number; runId: string }
>;

export type ValidationDraftSession = Readonly<{
	draft: ValidationProtocolDraft;
	freeze: FreezeState;
	sourceLoad: SourceLoadState;
}>;

export type DraftField =
	| "source"
	| "sampleOutStart"
	| "walkForward"
	| "crossMarket";
export type DraftError =
	| { kind: "incomplete-draft"; fields: readonly DraftField[] }
	| {
			kind: "invalid-value";
			field: DraftField;
			reason:
				| "not-a-date"
				| "outside-source"
				| "not-positive-integer"
				| "not-enough-history";
	  }
	| { kind: "incompatible-selection"; field: DraftField }
	| { kind: "incomplete-provenance" }
	| { kind: "source-loading" };

export type ValidationPreviewFacts = Readonly<{
	sourceRunId: string;
	bars: readonly { openTimeMs: number }[];
}>;

export type ValidationDraftInspection = Readonly<{
	errors: readonly DraftError[];
	preview?: WalkForwardPreview;
}>;

export type DraftCommand =
	| { type: "select-method"; method: ValidationMethod }
	| { type: "select-source"; runId: string }
	| { type: "accept-source"; revision: number; run: ValidationSourceRun }
	| { type: "reject-source"; revision: number; runId: string }
	| { type: "set-holdout-boundary"; value: string }
	| {
			type: "set-walk-forward-field";
			field: keyof WalkForwardConfiguration;
			value: string;
	  }
	| { type: "add-cross-market-context"; snapshot: ValidationSnapshot }
	| { type: "remove-cross-market-context"; snapshotId: string }
	| {
			type: "move-cross-market-context";
			snapshotId: string;
			direction: "earlier" | "later";
	  }
	| { type: "clear-cross-market-override"; snapshotId: string }
	| {
			type: "request-cross-market-override";
			snapshotId: string;
			runId: string;
	  }
	| {
			type: "accept-cross-market-override";
			revision: number;
			snapshotId: string;
			run: ValidationSourceRun;
	  }
	| {
			type: "reject-cross-market-override";
			revision: number;
			snapshotId: string;
			runId: string;
	  }
	| {
			type: "request-freeze";
			userId: string;
			previewFacts?: ValidationPreviewFacts;
	  }
	| { type: "accept-freeze"; revision: number; protocolId: string }
	| { type: "reject-freeze"; revision: number };

export type DraftEffect =
	| Readonly<{ kind: "load-source"; revision: number; runId: string }>
	| Readonly<{
			kind: "load-cross-market-override";
			revision: number;
			snapshotId: string;
			runId: string;
	  }>
	| Readonly<{
			kind: "freeze";
			revision: number;
			request: ValidationProtocolCreateRequest;
	  }>;

export type DraftTransition = Readonly<{
	session: ValidationDraftSession;
	effect?: DraftEffect;
	ignored?: "stale-source" | "stale-override" | "stale-freeze";
}>;

export type DraftResult<T> =
	| Readonly<{ ok: true; value: T }>
	| Readonly<{ ok: false; error: DraftError }>;

const EMPTY_FREEZE: FreezeState = { status: "absent" };
const IDLE_SOURCE_LOAD: SourceLoadState = { status: "idle" };

export function createValidationProtocolDraft(input?: {
	method?: ValidationMethod;
}): ValidationDraftSession {
	return {
		draft: createMethodDraft(input?.method ?? "chronological-holdout", 0),
		freeze: EMPTY_FREEZE,
		sourceLoad: IDLE_SOURCE_LOAD,
	};
}

export function transitionValidationProtocolDraft(
	session: ValidationDraftSession,
	command: DraftCommand,
): DraftResult<DraftTransition> {
	switch (command.type) {
		case "select-method":
			return selectMethod(session, command.method);
		case "select-source":
			return selectSource(session, command.runId);
		case "accept-source":
			return acceptSource(session, command);
		case "reject-source":
			return rejectSource(session, command);
		case "set-holdout-boundary":
			return setHoldoutBoundary(session, command.value);
		case "set-walk-forward-field":
			return setWalkForwardField(session, command.field, command.value);
		case "add-cross-market-context":
			return addCrossMarketContext(session, command.snapshot);
		case "remove-cross-market-context":
			return removeCrossMarketContext(session, command.snapshotId);
		case "move-cross-market-context":
			return moveCrossMarketContext(
				session,
				command.snapshotId,
				command.direction,
			);
		case "clear-cross-market-override":
			return clearCrossMarketOverride(session, command.snapshotId);
		case "request-cross-market-override":
			return requestCrossMarketOverride(
				session,
				command.snapshotId,
				command.runId,
			);
		case "accept-cross-market-override":
			return acceptCrossMarketOverride(session, command);
		case "reject-cross-market-override":
			return rejectCrossMarketOverride(session, command);
		case "request-freeze":
			return requestFreeze(session, command.userId, command.previewFacts);
		case "accept-freeze":
			return acceptFreeze(session, command.revision, command.protocolId);
		case "reject-freeze":
			return rejectFreeze(session, command.revision);
	}
}

export function inspectValidationProtocolDraft(
	session: ValidationDraftSession,
	previewFacts?: ValidationPreviewFacts,
): ValidationDraftInspection {
	const { draft } = session;
	if (!draft.source) {
		return {
			errors: [
				session.sourceLoad.status === "pending"
					? { kind: "source-loading" }
					: { kind: "incomplete-draft", fields: ["source"] },
			],
		};
	}
	if (draft.kind === "chronological-holdout") {
		const timestamp = Date.parse(draft.sampleOutStart);
		if (!draft.sampleOutStart || Number.isNaN(timestamp)) {
			return {
				errors: [
					{
						kind: "invalid-value",
						field: "sampleOutStart",
						reason: "not-a-date",
					},
				],
			};
		}
		if (
			timestamp <= draft.source.snapshot.startTimeMs ||
			timestamp >= draft.source.snapshot.endTimeMs
		) {
			return {
				errors: [
					{
						kind: "invalid-value",
						field: "sampleOutStart",
						reason: "outside-source",
					},
				],
			};
		}
		return { errors: [] };
	}
	if (draft.kind === "walk-forward") {
		const configuration = parseWalkForwardConfiguration(draft);
		if (!configuration) {
			return {
				errors: [
					{
						kind: "invalid-value",
						field: "walkForward",
						reason: "not-positive-integer",
					},
				],
			};
		}
		if (
			previewFacts &&
			(previewFacts.sourceRunId !== draft.source.runId ||
				previewFacts.bars.length <
					configuration.minimumHistoryBars + configuration.windowSizeBars)
		) {
			return {
				errors: [
					{
						kind: "invalid-value",
						field: "walkForward",
						reason: "not-enough-history",
					},
				],
			};
		}
		return {
			errors: [],
			preview: previewFacts
				? walkForwardPreview(previewFacts.bars, configuration)
				: undefined,
		};
	}
	const ids = draft.contexts.map((context) => context.snapshot.snapshotId);
	if (draft.contexts.length < 2 || new Set(ids).size !== ids.length) {
		return { errors: [{ kind: "incomplete-draft", fields: ["crossMarket"] }] };
	}
	if (draft.contexts.some((context) => context.override?.status === "pending")) {
		return { errors: [{ kind: "source-loading" }] };
	}
	if (
		draft.contexts.some(
			(context) =>
				context.snapshot.interval !== draft.contexts[0].snapshot.interval,
		)
	) {
		return {
			errors: [{ kind: "incompatible-selection", field: "crossMarket" }],
		};
	}
	return { errors: [] };
}

function createMethodDraft(
	method: ValidationMethod,
	revision: number,
	source?: SourceBinding,
): ValidationProtocolDraft {
	if (method === "walk-forward") {
		return {
			kind: method,
			revision,
			source,
			windowSizeBars: "100",
			stepSizeBars: "100",
			minimumHistoryBars: "500",
		};
	}
	if (method === "cross-market") {
		return { kind: method, revision, source, contexts: [] };
	}
	return { kind: method, revision, source, sampleOutStart: "" };
}

function transition(draft: ValidationProtocolDraft): DraftTransition {
	return {
		session: {
			draft,
			freeze: EMPTY_FREEZE,
			sourceLoad: IDLE_SOURCE_LOAD,
		},
	};
}

function withChangedDraft(
	session: ValidationDraftSession,
	draft: ValidationProtocolDraft,
): DraftTransition {
	return transition({ ...draft, revision: session.draft.revision + 1 });
}

function selectMethod(
	session: ValidationDraftSession,
	method: ValidationMethod,
): DraftResult<DraftTransition> {
	if (session.draft.kind === method) return { ok: true, value: { session } };
	return {
		ok: true,
		value: withChangedDraft(
			session,
			createMethodDraft(method, session.draft.revision, session.draft.source),
		),
	};
}

function selectSource(
	session: ValidationDraftSession,
	runId: string,
): DraftResult<DraftTransition> {
	if (
		(session.sourceLoad.status === "pending" &&
			session.sourceLoad.runId === runId) ||
		(session.draft.source?.runId === runId &&
			session.sourceLoad.status === "idle")
	) {
		return { ok: true, value: { session } };
	}
	let draft: ValidationProtocolDraft = {
		...session.draft,
		source: undefined,
	};
	if (draft.kind === "chronological-holdout") {
		draft = { ...draft, sampleOutStart: "" };
	} else if (draft.kind === "cross-market") {
		draft = {
			...draft,
			contexts: draft.contexts.map((context) =>
				context.override?.status === "pending"
					? { snapshot: context.snapshot }
					: context,
			),
		};
	}
	const revision = session.draft.revision + 1;
	draft = { ...draft, revision };
	return {
		ok: true,
		value: {
			session: {
				draft,
				freeze: EMPTY_FREEZE,
				sourceLoad: { status: "pending", revision, runId },
			},
			effect: { kind: "load-source", revision, runId },
		},
	};
}

function acceptSource(
	session: ValidationDraftSession,
	command: Extract<DraftCommand, { type: "accept-source" }>,
): DraftResult<DraftTransition> {
	if (
		session.sourceLoad.status !== "pending" ||
		session.sourceLoad.revision !== command.revision ||
		session.sourceLoad.runId !== command.run.runId
	) {
		return { ok: true, value: { session, ignored: "stale-source" } };
	}
	if (!command.run.provenance) {
		return { ok: false, error: { kind: "incomplete-provenance" } };
	}
	if (
		command.run.provenance.normalizedRequest.snapshotId !==
		command.run.snapshot.snapshotId
	) {
		return {
			ok: false,
			error: { kind: "incompatible-selection", field: "source" },
		};
	}
	const source: SourceBinding = {
		runId: command.run.runId,
		snapshot: command.run.snapshot,
		normalizedRequest: cloneNormalizedRequest(
			command.run.provenance.normalizedRequest,
		),
	};
	return {
		ok: true,
		value: {
			session: {
				draft: { ...session.draft, source },
				freeze: EMPTY_FREEZE,
				sourceLoad: IDLE_SOURCE_LOAD,
			},
		},
	};
}

function rejectSource(
	session: ValidationDraftSession,
	command: Extract<DraftCommand, { type: "reject-source" }>,
): DraftResult<DraftTransition> {
	if (
		session.sourceLoad.status !== "pending" ||
		session.sourceLoad.revision !== command.revision ||
		session.sourceLoad.runId !== command.runId
	) {
		return { ok: true, value: { session, ignored: "stale-source" } };
	}
	return {
		ok: true,
		value: { session: { ...session, sourceLoad: IDLE_SOURCE_LOAD } },
	};
}

function setHoldoutBoundary(
	session: ValidationDraftSession,
	value: string,
): DraftResult<DraftTransition> {
	if (session.draft.kind !== "chronological-holdout") {
		return {
			ok: false,
			error: { kind: "incompatible-selection", field: "sampleOutStart" },
		};
	}
	if (session.draft.sampleOutStart === value)
		return { ok: true, value: { session } };
	return {
		ok: true,
		value: withChangedDraft(session, { ...session.draft, sampleOutStart: value }),
	};
}

function setWalkForwardField(
	session: ValidationDraftSession,
	field: keyof WalkForwardConfiguration,
	value: string,
): DraftResult<DraftTransition> {
	if (session.draft.kind !== "walk-forward") {
		return {
			ok: false,
			error: { kind: "incompatible-selection", field: "walkForward" },
		};
	}
	if (session.draft[field] === value) return { ok: true, value: { session } };
	return {
		ok: true,
		value: withChangedDraft(session, { ...session.draft, [field]: value }),
	};
}

function addCrossMarketContext(
	session: ValidationDraftSession,
	snapshot: ValidationSnapshot,
): DraftResult<DraftTransition> {
	if (session.draft.kind !== "cross-market") {
		return {
			ok: false,
			error: { kind: "incompatible-selection", field: "crossMarket" },
		};
	}
	if (
		session.draft.contexts.some(
			(context) => context.snapshot.snapshotId === snapshot.snapshotId,
		)
	) {
		return {
			ok: false,
			error: { kind: "incompatible-selection", field: "crossMarket" },
		};
	}
	return {
		ok: true,
		value: withChangedDraft(session, {
			...session.draft,
			contexts: [...session.draft.contexts, { snapshot }],
		}),
	};
}

function removeCrossMarketContext(
	session: ValidationDraftSession,
	snapshotId: string,
): DraftResult<DraftTransition> {
	if (session.draft.kind !== "cross-market") {
		return {
			ok: false,
			error: { kind: "incompatible-selection", field: "crossMarket" },
		};
	}
	const contexts = session.draft.contexts.filter(
		(context) => context.snapshot.snapshotId !== snapshotId,
	);
	if (contexts.length === session.draft.contexts.length)
		return { ok: true, value: { session } };
	return {
		ok: true,
		value: withChangedDraft(session, { ...session.draft, contexts }),
	};
}

function moveCrossMarketContext(
	session: ValidationDraftSession,
	snapshotId: string,
	direction: "earlier" | "later",
): DraftResult<DraftTransition> {
	if (session.draft.kind !== "cross-market") {
		return {
			ok: false,
			error: { kind: "incompatible-selection", field: "crossMarket" },
		};
	}
	const index = session.draft.contexts.findIndex(
		(context) => context.snapshot.snapshotId === snapshotId,
	);
	const nextIndex = direction === "earlier" ? index - 1 : index + 1;
	if (index < 0 || nextIndex < 0 || nextIndex >= session.draft.contexts.length) {
		return { ok: true, value: { session } };
	}
	const contexts = [...session.draft.contexts];
	[contexts[index], contexts[nextIndex]] = [
		contexts[nextIndex],
		contexts[index],
	];
	return {
		ok: true,
		value: withChangedDraft(session, { ...session.draft, contexts }),
	};
}

function clearCrossMarketOverride(
	session: ValidationDraftSession,
	snapshotId: string,
): DraftResult<DraftTransition> {
	if (session.draft.kind !== "cross-market") {
		return {
			ok: false,
			error: { kind: "incompatible-selection", field: "crossMarket" },
		};
	}
	const context = session.draft.contexts.find(
		(item) => item.snapshot.snapshotId === snapshotId,
	);
	if (!context?.override) return { ok: true, value: { session } };
	return {
		ok: true,
		value: withChangedDraft(session, {
			...session.draft,
			contexts: session.draft.contexts.map((item) =>
				item.snapshot.snapshotId === snapshotId
					? { snapshot: item.snapshot }
					: item,
			),
		}),
	};
}

function requestCrossMarketOverride(
	session: ValidationDraftSession,
	snapshotId: string,
	runId: string,
): DraftResult<DraftTransition> {
	if (session.draft.kind !== "cross-market") {
		return {
			ok: false,
			error: { kind: "incompatible-selection", field: "crossMarket" },
		};
	}
	const context = session.draft.contexts.find(
		(item) => item.snapshot.snapshotId === snapshotId,
	);
	if (!context)
		return {
			ok: false,
			error: { kind: "incompatible-selection", field: "crossMarket" },
		};
	if (
		context.override?.runId === runId &&
		(context.override.status === "ready" || context.override.status === "pending")
	) {
		return { ok: true, value: { session } };
	}
	const revision = session.draft.revision + 1;
	const draft: ValidationProtocolDraft = {
		...session.draft,
		revision,
		contexts: session.draft.contexts.map((item) =>
			item.snapshot.snapshotId === snapshotId
				? { snapshot: item.snapshot, override: { status: "pending", runId } }
				: item,
		),
	};
	return {
		ok: true,
		value: {
			session: {
				draft,
				freeze: EMPTY_FREEZE,
				sourceLoad: IDLE_SOURCE_LOAD,
			},
			effect: {
				kind: "load-cross-market-override",
				revision,
				snapshotId,
				runId,
			},
		},
	};
}

function acceptCrossMarketOverride(
	session: ValidationDraftSession,
	command: Extract<DraftCommand, { type: "accept-cross-market-override" }>,
): DraftResult<DraftTransition> {
	if (
		session.draft.revision !== command.revision ||
		session.draft.kind !== "cross-market"
	) {
		return { ok: true, value: { session, ignored: "stale-override" } };
	}
	const context = session.draft.contexts.find(
		(item) => item.snapshot.snapshotId === command.snapshotId,
	);
	if (
		context?.override?.status !== "pending" ||
		context.override?.runId !== command.run.runId
	) {
		return { ok: true, value: { session, ignored: "stale-override" } };
	}
	if (!command.run.provenance) {
		return { ok: false, error: { kind: "incomplete-provenance" } };
	}
	if (command.run.snapshot.snapshotId !== command.snapshotId) {
		return {
			ok: false,
			error: { kind: "incompatible-selection", field: "crossMarket" },
		};
	}
	const normalizedRequest = command.run.provenance.normalizedRequest;
	if (normalizedRequest.snapshotId !== command.snapshotId) {
		return {
			ok: false,
			error: { kind: "incompatible-selection", field: "crossMarket" },
		};
	}
	return {
		ok: true,
		value: {
			session: {
				draft: {
					...session.draft,
					contexts: session.draft.contexts.map((item) =>
						item.snapshot.snapshotId === command.snapshotId
							? {
									snapshot: item.snapshot,
									override: {
										status: "ready",
										runId: command.run.runId,
										normalizedRequest: cloneNormalizedRequest(normalizedRequest),
									},
								}
							: item,
					),
				},
				freeze: EMPTY_FREEZE,
				sourceLoad: IDLE_SOURCE_LOAD,
			},
		},
	};
}

function rejectCrossMarketOverride(
	session: ValidationDraftSession,
	command: Extract<DraftCommand, { type: "reject-cross-market-override" }>,
): DraftResult<DraftTransition> {
	if (
		session.draft.revision !== command.revision ||
		session.draft.kind !== "cross-market"
	) {
		return { ok: true, value: { session, ignored: "stale-override" } };
	}
	const context = session.draft.contexts.find(
		(item) => item.snapshot.snapshotId === command.snapshotId,
	);
	if (
		context?.override?.status !== "pending" ||
		context.override?.runId !== command.runId
	) {
		return { ok: true, value: { session, ignored: "stale-override" } };
	}
	return {
		ok: true,
		value: {
			session: {
				draft: {
					...session.draft,
					contexts: session.draft.contexts.map((item) =>
						item.snapshot.snapshotId === command.snapshotId
							? { snapshot: item.snapshot }
							: item,
					),
				},
				freeze: EMPTY_FREEZE,
				sourceLoad: IDLE_SOURCE_LOAD,
			},
		},
	};
}

function requestFreeze(
	session: ValidationDraftSession,
	userId: string,
	previewFacts?: ValidationPreviewFacts,
): DraftResult<DraftTransition> {
	if (
		session.freeze.status === "pending" ||
		session.freeze.status === "succeeded"
	) {
		return { ok: true, value: { session } };
	}
	const inspection = inspectValidationProtocolDraft(session, previewFacts);
	if (inspection.errors.length > 0)
		return { ok: false, error: inspection.errors[0] };
	if (!session.draft.source) {
		return { ok: false, error: { kind: "incomplete-draft", fields: ["source"] } };
	}
	const request = materializeProtocolRequest(session.draft, userId);
	return {
		ok: true,
		value: {
			session: {
				...session,
				freeze: { status: "pending", revision: session.draft.revision },
			},
			effect: { kind: "freeze", revision: session.draft.revision, request },
		},
	};
}

function acceptFreeze(
	session: ValidationDraftSession,
	revision: number,
	protocolId: string,
): DraftResult<DraftTransition> {
	if (
		session.freeze.status !== "pending" ||
		session.freeze.revision !== revision
	) {
		return { ok: true, value: { session, ignored: "stale-freeze" } };
	}
	return {
		ok: true,
		value: {
			session: {
				...session,
				freeze: { status: "succeeded", revision, protocolId },
			},
		},
	};
}

function rejectFreeze(
	session: ValidationDraftSession,
	revision: number,
): DraftResult<DraftTransition> {
	if (
		session.freeze.status !== "pending" ||
		session.freeze.revision !== revision
	) {
		return { ok: true, value: { session, ignored: "stale-freeze" } };
	}
	return { ok: true, value: { session: { ...session, freeze: EMPTY_FREEZE } } };
}

function parseWalkForwardConfiguration(
	draft: Extract<ValidationProtocolDraft, { kind: "walk-forward" }>,
) {
	const values = [
		draft.windowSizeBars,
		draft.stepSizeBars,
		draft.minimumHistoryBars,
	].map((value) => Number(value));
	if (!values.every((value) => Number.isInteger(value) && value > 0)) return;
	if (values[1] < values[0]) return;
	return {
		windowSizeBars: values[0],
		stepSizeBars: values[1],
		minimumHistoryBars: values[2],
	};
}

function walkForwardPreview(
	bars: readonly { openTimeMs: number }[],
	configuration: {
		windowSizeBars: number;
		stepSizeBars: number;
		minimumHistoryBars: number;
	},
): WalkForwardPreview {
	const windows = [];
	for (
		let start = configuration.minimumHistoryBars;
		start + configuration.windowSizeBars <= bars.length;
		start += configuration.stepSizeBars
	) {
		windows.push({
			sampleOutStartTimeMs: bars[start].openTimeMs,
			sampleOutEndTimeMs: bars[start + configuration.windowSizeBars]?.openTimeMs,
		});
	}
	const nextStart =
		configuration.minimumHistoryBars +
		windows.length * configuration.stepSizeBars;
	return {
		windows,
		partialFinalWindow:
			nextStart < bars.length &&
			nextStart + configuration.windowSizeBars > bars.length,
	};
}

function materializeProtocolRequest(
	draft: ValidationProtocolDraft,
	userId: string,
): ValidationProtocolCreateRequest {
	if (!draft.source) throw new Error("Validation source is incomplete");
	const run = materializeRunRequest(userId, draft.source.normalizedRequest);
	if (draft.kind === "chronological-holdout") {
		return {
			userId,
			run,
			windows: [
				{
					snapshotId: draft.source.snapshot.snapshotId,
					sampleOutStartTimeMs: Date.parse(draft.sampleOutStart),
				},
			],
			methodVersion: "chronological-holdout@1",
			aggregationRuleVersion: "equal-window@1",
		};
	}
	if (draft.kind === "walk-forward") {
		const configuration = parseWalkForwardConfiguration(draft);
		if (!configuration) throw new Error("Walk-forward configuration is invalid");
		return {
			userId,
			run,
			windows: [],
			walkForward: {
				snapshotId: draft.source.snapshot.snapshotId,
				...configuration,
			},
			methodVersion: "walk-forward@1",
			aggregationRuleVersion: "equal-window@1",
		};
	}
	return {
		userId,
		run,
		windows: [],
		crossMarket: {
			contexts: draft.contexts.map((context) => ({
				snapshotId: context.snapshot.snapshotId,
				...(context.override?.status === "ready"
					? {
							runOverride: materializeRunRequest(
								userId,
								context.override.normalizedRequest,
							),
						}
					: {}),
			})),
		},
		methodVersion: "cross-market@1",
		aggregationRuleVersion: "equal-window@1",
	};
}

function materializeRunRequest(
	userId: string,
	configuration: NormalizedRunConfiguration,
): ValidationRunRequest {
	return {
		userId,
		snapshotId: configuration.snapshotId,
		runStartTimeMs: configuration.runStartTimeMs,
		runEndTimeMs: configuration.runEndTimeMs,
		strategyArchiveSha256: configuration.strategyArchiveSha256,
		strategyParameters: { ...configuration.strategyParameters },
		factorInstances: configuration.factorInstances.map((factor) => ({
			alias: factor.alias,
			archiveSha256: factor.archiveSha256,
			parameters: Object.fromEntries(
				factor.parameters.map((parameter) => [parameter.name, parameter.value]),
			),
		})),
		signalInstances: configuration.signalInstances.map((signal) => ({
			...signal,
		})),
		initialQuoteAllocation: configuration.initialQuoteAllocation,
		executionProfile: { ...configuration.executionProfile },
		seed: configuration.seed,
	};
}

function cloneNormalizedRequest(
	configuration: NormalizedRunConfiguration,
): NormalizedRunConfiguration {
	return {
		...configuration,
		strategyParameters: { ...configuration.strategyParameters },
		factorInstances: configuration.factorInstances.map((factor) => ({
			...factor,
			parameters: factor.parameters.map((parameter) => ({ ...parameter })),
		})),
		signalInstances: configuration.signalInstances.map((signal) => ({
			...signal,
		})),
		executionProfile: { ...configuration.executionProfile },
	};
}
