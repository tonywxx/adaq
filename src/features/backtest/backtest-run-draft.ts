import type { LibraryComponent } from "@/features/components/component-library";
import type { BarInterval } from "@/lib/market-chart-adapter";

export type ExecutionProfile = {
	makerFeeRate: string;
	takerFeeRate: string;
	adverseSlippageRate: string;
	rebalanceThreshold: string;
	priceIncrement: string;
	quantityIncrement: string;
	minimumQuantity: string;
	riskFreeRate: string;
	fillPolicy: "maker" | "taker";
};

export const defaultExecutionProfile: ExecutionProfile = {
	makerFeeRate: "0.0008",
	takerFeeRate: "0.001",
	adverseSlippageRate: "0.0005",
	rebalanceThreshold: "0",
	priceIncrement: "0.1",
	quantityIncrement: "0.00000001",
	minimumQuantity: "0.00001",
	riskFreeRate: "0",
	fillPolicy: "taker",
};

export type NormalizedRunConfiguration = {
	snapshotId: string;
	runStartTimeMs?: number;
	runEndTimeMs?: number;
	strategyArchiveSha256: string;
	strategyParameters: Record<string, string>;
	factorInstances: Array<{
		alias: string;
		archiveSha256: string;
		parameters: Array<{ name: string; value: string }>;
	}>;
	signalInstances: Array<{
		slot: string;
		datasetId: string;
		signalName: string;
	}>;
	initialQuoteAllocation: string;
	executionProfile: ExecutionProfile;
	seed: number;
};

export type SnapshotBinding = Readonly<{
	snapshotId: string;
	startTimeMs: number;
	endTimeMs: number;
}>;

export type RunWindow = Readonly<{
	startTimeMs: number;
	endTimeMs: number;
}>;

export type SignalCandidate = Readonly<{
	slot: string;
	datasetId: string;
	signalName: string;
	evidenceState: string;
}>;

export type BacktestPreflight = Readonly<{
	runId: string;
	reusesExistingRun: boolean;
	snapshot: SnapshotBinding;
	normalizedRequest: Record<string, unknown>;
	featurePlan: Record<string, unknown>;
	componentLock: Array<Record<string, unknown>>;
	datasetLock: Array<Record<string, unknown>>;
	architecture: "signal-driven" | "composed" | "hybrid";
}>;

export type BacktestRunRequest = Readonly<{
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
	executionProfile: ExecutionProfile;
	seed: number;
}>;

export type DraftStage = "data" | "strategy" | "execution";
export type ExecutionProfileField = Exclude<
	keyof ExecutionProfile,
	"fillPolicy"
>;
export type DraftField =
	| "snapshot"
	| "strategy"
	| "factor"
	| "signal"
	| "runWindow"
	| "initialQuoteAllocation"
	| "executionProfile"
	| "seed";
export type DraftErrorReason =
	| "required"
	| "invalid-number"
	| "not-positive"
	| "not-ordered";

export type DraftError =
	| { kind: "incomplete-draft"; fields: readonly DraftField[] }
	| { kind: "invalid-value"; field: DraftField; reason: DraftErrorReason }
	| { kind: "incompatible-selection"; field: DraftField }
	| { kind: "incomplete-provenance" }
	| { kind: "preflight-required" };

export type BacktestRunDraft = Readonly<{
	revision: number;
	selectedInstrumentKey: string;
	interval: BarInterval;
	start: string;
	end: string;
	snapshot?: SnapshotBinding;
	runWindow?: RunWindow;
	strategy?: LibraryComponent;
	strategyParameters: Readonly<Record<string, string>>;
	factorSelections: Readonly<Record<string, string>>;
	factorParameters: Readonly<Record<string, Readonly<Record<string, string>>>>;
	signalSelections: Readonly<
		Record<
			string,
			Readonly<{
				datasetId: string;
				signalName: string;
			}>
		>
	>;
	initialQuoteAllocation: string;
	executionProfile: ExecutionProfile;
	seed: string;
}>;

type PreflightState =
	| Readonly<{ status: "absent" }>
	| Readonly<{ status: "pending"; revision: number }>
	| Readonly<{
			status: "ready";
			revision: number;
			value: BacktestPreflight;
	  }>;

export type BacktestDraftSession = Readonly<{
	draft: BacktestRunDraft;
	stage: DraftStage;
	preflight: PreflightState;
}>;

export type EmptyDraftInput = Readonly<{
	kind: "empty";
	selectedInstrumentKey: string;
	interval: BarInterval;
	start: string;
	end: string;
	defaultExecutionProfile: ExecutionProfile;
	defaultInitialQuoteAllocation: string;
	defaultSeed: string;
}>;

export type ProvenanceDraftInput = Readonly<{
	kind: "from-run-provenance";
	selectedInstrumentKey: string;
	interval: BarInterval;
	start: string;
	end: string;
	snapshot: SnapshotBinding;
	strategy: LibraryComponent;
	normalizedRequest: NormalizedRunConfiguration;
}>;

export type CreateDraftInput = EmptyDraftInput | ProvenanceDraftInput;

export type DraftCommand =
	| { type: "select-instrument"; selectedInstrumentKey: string }
	| { type: "set-interval"; interval: BarInterval }
	| { type: "set-date-range"; start: string; end: string }
	| { type: "select-snapshot"; snapshot?: SnapshotBinding }
	| { type: "set-run-window"; value?: RunWindow }
	| { type: "select-strategy"; strategy?: LibraryComponent }
	| { type: "set-strategy-parameter"; name: string; value: string }
	| {
			type: "select-factor";
			alias: string;
			archiveSha256?: string;
			compatibleHashes: readonly string[];
	  }
	| { type: "set-factor-parameter"; alias: string; name: string; value: string }
	| {
			type: "select-signal";
			slot: string;
			candidate?: SignalCandidate;
			compatibleCandidates: readonly SignalCandidate[];
	  }
	| {
			type: "set-allocation";
			value: string;
	  }
	| {
			type: "set-execution-profile-field";
			field: ExecutionProfileField;
			value: string;
	  }
	| { type: "set-fill-policy"; value: ExecutionProfile["fillPolicy"] }
	| { type: "set-seed"; value: string }
	| {
			type: "reconcile-factor-compatibility";
			strategyArchiveSha256: string;
			compatibleHashes: Readonly<Record<string, readonly string[]>>;
	  }
	| {
			type: "reconcile-signal-compatibility";
			strategyArchiveSha256: string;
			snapshotId: string;
			compatibleCandidates: readonly SignalCandidate[];
	  }
	| { type: "enter-stage"; stage: DraftStage; userId: string }
	| {
			type: "accept-preflight";
			revision: number;
			preflight: BacktestPreflight;
	  }
	| { type: "reject-preflight"; revision: number }
	| { type: "request-run"; userId: string };

export type DraftEffect =
	| Readonly<{
			kind: "preflight";
			revision: number;
			request: BacktestRunRequest;
	  }>
	| Readonly<{
			kind: "run";
			revision: number;
			request: BacktestRunRequest;
	  }>;

export type DraftTransition = Readonly<{
	session: BacktestDraftSession;
	effect?: DraftEffect;
	ignored?: "stale-compatibility" | "stale-preflight";
}>;

export type DraftResult<T> =
	| Readonly<{ ok: true; value: T }>
	| Readonly<{ ok: false; error: DraftError }>;

function absentPreflight(): PreflightState {
	return { status: "absent" };
}

function emptyDraft(input: EmptyDraftInput): BacktestRunDraft {
	return {
		revision: 0,
		selectedInstrumentKey: input.selectedInstrumentKey,
		interval: input.interval,
		start: input.start,
		end: input.end,
		strategyParameters: {},
		factorSelections: {},
		factorParameters: {},
		signalSelections: {},
		initialQuoteAllocation: input.defaultInitialQuoteAllocation,
		executionProfile: { ...input.defaultExecutionProfile },
		seed: input.defaultSeed,
	};
}

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null;
}

function hasUniqueStringField(items: readonly unknown[], field: string) {
	const values = items.map((item) =>
		isRecord(item) && typeof item[field] === "string" ? item[field] : "",
	);
	return values.every(Boolean) && new Set(values).size === values.length;
}

function isCompleteProvenance(
	value: unknown,
	snapshotId: string,
	strategyArchiveSha256: string,
): value is NormalizedRunConfiguration {
	if (!isRecord(value)) return false;
	if (
		value.snapshotId !== snapshotId ||
		value.strategyArchiveSha256 !== strategyArchiveSha256 ||
		typeof value.initialQuoteAllocation !== "string" ||
		!Number.isSafeInteger(value.seed) ||
		!isRecord(value.strategyParameters) ||
		!isRecord(value.executionProfile) ||
		!Array.isArray(value.factorInstances) ||
		!Array.isArray(value.signalInstances)
	)
		return false;
	if (
		typeof value.runStartTimeMs !== "number" ||
		typeof value.runEndTimeMs !== "number" ||
		!Number.isFinite(value.runStartTimeMs) ||
		!Number.isFinite(value.runEndTimeMs) ||
		value.runStartTimeMs >= value.runEndTimeMs
	)
		return false;
	if (
		!Object.values(value.strategyParameters).every(
			(item) => typeof item === "string",
		)
	)
		return false;
	const executionProfile = value.executionProfile;
	if (!isRecord(executionProfile)) return false;
	if (
		!Object.keys(defaultExecutionProfile).every(
			(field) => typeof executionProfile[field] === "string",
		) ||
		(executionProfile.fillPolicy !== "maker" &&
			executionProfile.fillPolicy !== "taker") ||
		!Object.values(executionProfile).every((item) => typeof item === "string")
	)
		return false;
	if (
		!hasUniqueStringField(value.factorInstances, "alias") ||
		!value.factorInstances.every(
			(item) =>
				isRecord(item) &&
				typeof item.alias === "string" &&
				item.alias.length > 0 &&
				typeof item.archiveSha256 === "string" &&
				item.archiveSha256.length > 0 &&
				Array.isArray(item.parameters) &&
				hasUniqueStringField(item.parameters, "name") &&
				item.parameters.every(
					(parameter) =>
						isRecord(parameter) &&
						typeof parameter.name === "string" &&
						typeof parameter.value === "string",
				),
		)
	)
		return false;
	if (
		!hasUniqueStringField(value.signalInstances, "slot") ||
		!value.signalInstances.every(
			(item) =>
				isRecord(item) &&
				typeof item.datasetId === "string" &&
				item.datasetId.length > 0 &&
				typeof item.signalName === "string" &&
				item.signalName.length > 0,
		)
	)
		return false;
	return true;
}

function fromProvenance(
	input: ProvenanceDraftInput,
): DraftResult<BacktestDraftSession> {
	const { normalizedRequest } = input;
	if (
		!isCompleteProvenance(
			normalizedRequest,
			input.snapshot.snapshotId,
			input.strategy.archiveSha256,
		)
	)
		return { ok: false, error: { kind: "incomplete-provenance" } };
	const factorAliases = new Set(
		input.strategy.dependencies.map((dependency) => dependency.alias),
	);
	if (
		normalizedRequest.factorInstances.length !== factorAliases.size ||
		normalizedRequest.factorInstances.some(
			(factor) => !factorAliases.has(factor.alias),
		)
	)
		return { ok: false, error: { kind: "incomplete-provenance" } };
	const signalSlots = new Set(
		input.strategy.featureSlots
			.filter((slot) => slot.source.kind === "signal")
			.map((slot) => slot.name),
	);
	if (
		normalizedRequest.signalInstances.length !== signalSlots.size ||
		normalizedRequest.signalInstances.some(
			(signal) => !signalSlots.has(signal.slot),
		)
	)
		return { ok: false, error: { kind: "incomplete-provenance" } };
	const factorSelections = Object.fromEntries(
		normalizedRequest.factorInstances.map((factor) => [
			factor.alias,
			factor.archiveSha256,
		]),
	);
	const factorParameters = Object.fromEntries(
		normalizedRequest.factorInstances.map((factor) => [
			factor.alias,
			Object.fromEntries(
				factor.parameters.map((parameter) => [parameter.name, parameter.value]),
			),
		]),
	);
	const signalSelections = Object.fromEntries(
		normalizedRequest.signalInstances.map((signal) => [
			signal.slot,
			{
				datasetId: signal.datasetId,
				signalName: signal.signalName,
			},
		]),
	);
	return {
		ok: true,
		value: {
			draft: {
				revision: 0,
				selectedInstrumentKey: input.selectedInstrumentKey,
				interval: input.interval,
				start: input.start,
				end: input.end,
				snapshot: input.snapshot,
				runWindow:
					normalizedRequest.runStartTimeMs == null ||
					normalizedRequest.runEndTimeMs == null
						? undefined
						: {
								startTimeMs: normalizedRequest.runStartTimeMs,
								endTimeMs: normalizedRequest.runEndTimeMs,
							},
				strategy: input.strategy,
				strategyParameters: { ...normalizedRequest.strategyParameters },
				factorSelections,
				factorParameters,
				signalSelections,
				initialQuoteAllocation: normalizedRequest.initialQuoteAllocation,
				executionProfile: { ...normalizedRequest.executionProfile },
				seed: String(normalizedRequest.seed),
			},
			stage: "strategy",
			preflight: absentPreflight(),
		},
	};
}

export function createBacktestDraft(
	input: CreateDraftInput,
): DraftResult<BacktestDraftSession> {
	if (input.kind === "from-run-provenance") return fromProvenance(input);
	return {
		ok: true,
		value: {
			draft: emptyDraft(input),
			stage: "data",
			preflight: absentPreflight(),
		},
	};
}

function sameSerializedValue(left: unknown, right: unknown) {
	return JSON.stringify(left) === JSON.stringify(right);
}

function comparableConfig(draft: BacktestRunDraft) {
	return {
		selectedInstrumentKey: draft.selectedInstrumentKey,
		interval: draft.interval,
		start: draft.start,
		end: draft.end,
		snapshot: draft.snapshot,
		runWindow: draft.runWindow,
		strategy: draft.strategy?.archiveSha256,
		strategyParameters: draft.strategyParameters,
		factorSelections: draft.factorSelections,
		factorParameters: draft.factorParameters,
		signalSelections: draft.signalSelections,
		initialQuoteAllocation: draft.initialQuoteAllocation,
		executionProfile: draft.executionProfile,
		seed: draft.seed,
	};
}

function sameConfig(left: BacktestRunDraft, right: BacktestRunDraft) {
	return sameSerializedValue(comparableConfig(left), comparableConfig(right));
}

function withConfig(
	session: BacktestDraftSession,
	next: BacktestRunDraft,
): BacktestDraftSession {
	if (sameConfig(session.draft, next)) return session;
	return {
		...session,
		draft: { ...next, revision: session.draft.revision + 1 },
		preflight: absentPreflight(),
	};
}

function draftError(
	kind: DraftError["kind"],
	field?: DraftField,
): DraftResult<never> {
	if (kind === "incomplete-draft")
		return { ok: false, error: { kind, fields: field ? [field] : [] } };
	if (kind === "invalid-value")
		return {
			ok: false,
			error: { kind, field: field ?? "snapshot", reason: "required" },
		};
	if (kind === "incompatible-selection")
		return { ok: false, error: { kind, field: field ?? "snapshot" } };
	if (kind === "incomplete-provenance") return { ok: false, error: { kind } };
	return { ok: false, error: { kind } };
}

function selectSnapshot(
	session: BacktestDraftSession,
	snapshot?: SnapshotBinding,
) {
	return withConfig(session, {
		...session.draft,
		snapshot,
		runWindow: undefined,
		signalSelections: {},
	});
}

function selectStrategy(
	session: BacktestDraftSession,
	strategy?: LibraryComponent,
) {
	return withConfig(session, {
		...session.draft,
		strategy,
		strategyParameters: {},
		factorSelections: {},
		factorParameters: {},
		signalSelections: {},
	});
}

function validateDraft(
	draft: BacktestRunDraft,
): DraftResult<BacktestRunRequest> {
	const fields: DraftField[] = [];
	if (!draft.snapshot) fields.push("snapshot");
	if (!draft.strategy) fields.push("strategy");
	if (draft.strategy)
		for (const dependency of draft.strategy.dependencies)
			if (!draft.factorSelections[dependency.alias]) fields.push("factor");
	const signalSlots =
		draft.strategy?.featureSlots.filter(
			(slot) => slot.source.kind === "signal",
		) ?? [];
	for (const slot of signalSlots)
		if (!draft.signalSelections[slot.name]) fields.push("signal");
	if (fields.length)
		return {
			ok: false,
			error: { kind: "incomplete-draft", fields: [...new Set(fields)] },
		};
	if (!draft.snapshot || !draft.strategy)
		return draftError("incomplete-draft", "snapshot");
	const start = draft.runWindow?.startTimeMs ?? draft.snapshot.startTimeMs;
	const end = draft.runWindow?.endTimeMs ?? draft.snapshot.endTimeMs;
	if (!Number.isFinite(start) || !Number.isFinite(end))
		return {
			ok: false,
			error: {
				kind: "invalid-value",
				field: "runWindow",
				reason: "invalid-number",
			},
		};
	if (start >= end)
		return {
			ok: false,
			error: { kind: "invalid-value", field: "runWindow", reason: "not-ordered" },
		};
	if (!draft.initialQuoteAllocation.trim())
		return {
			ok: false,
			error: {
				kind: "invalid-value",
				field: "initialQuoteAllocation",
				reason: "required",
			},
		};
	const allocation = Number(draft.initialQuoteAllocation);
	if (!Number.isFinite(allocation))
		return {
			ok: false,
			error: {
				kind: "invalid-value",
				field: "initialQuoteAllocation",
				reason: "invalid-number",
			},
		};
	if (allocation <= 0)
		return {
			ok: false,
			error: {
				kind: "invalid-value",
				field: "initialQuoteAllocation",
				reason: "not-positive",
			},
		};
	const seed = Number(draft.seed);
	if (!Number.isSafeInteger(seed) || seed < 0)
		return {
			ok: false,
			error: { kind: "invalid-value", field: "seed", reason: "invalid-number" },
		};
	return {
		ok: true,
		value: {
			userId: "",
			snapshotId: draft.snapshot.snapshotId,
			runStartTimeMs: draft.runWindow?.startTimeMs,
			runEndTimeMs: draft.runWindow?.endTimeMs,
			factorInstances: draft.strategy.dependencies.map((dependency) => ({
				alias: dependency.alias,
				archiveSha256: draft.factorSelections[dependency.alias],
				parameters: { ...(draft.factorParameters[dependency.alias] ?? {}) },
			})),
			signalInstances: Object.entries(draft.signalSelections).map(
				([slot, signal]) => ({
					slot,
					datasetId: signal.datasetId,
					signalName: signal.signalName,
				}),
			),
			strategyArchiveSha256: draft.strategy.archiveSha256,
			strategyParameters: { ...draft.strategyParameters },
			initialQuoteAllocation: draft.initialQuoteAllocation,
			executionProfile: { ...draft.executionProfile },
			seed,
		},
	};
}

function withUserId(
	request: BacktestRunRequest,
	userId: string,
): BacktestRunRequest {
	return { ...request, userId };
}

function transitionConfig(
	session: BacktestDraftSession,
	next: BacktestRunDraft,
): DraftResult<DraftTransition> {
	return { ok: true, value: { session: withConfig(session, next) } };
}

export function transitionBacktestDraft(
	session: BacktestDraftSession,
	command: DraftCommand,
): DraftResult<DraftTransition> {
	const { draft } = session;
	switch (command.type) {
		case "select-instrument":
			return transitionConfig(session, {
				...draft,
				selectedInstrumentKey: command.selectedInstrumentKey,
				snapshot: undefined,
				runWindow: undefined,
				signalSelections: {},
			});
		case "set-interval":
			return transitionConfig(session, {
				...draft,
				interval: command.interval,
				snapshot: undefined,
				runWindow: undefined,
				signalSelections: {},
			});
		case "set-date-range":
			return transitionConfig(session, {
				...draft,
				start: command.start,
				end: command.end,
				runWindow: undefined,
				signalSelections: {},
			});
		case "select-snapshot":
			return transitionConfig(
				session,
				selectSnapshot(session, command.snapshot).draft,
			);
		case "set-run-window":
			return transitionConfig(session, { ...draft, runWindow: command.value });
		case "select-strategy":
			return transitionConfig(
				session,
				selectStrategy(session, command.strategy).draft,
			);
		case "set-strategy-parameter":
			return transitionConfig(session, {
				...draft,
				strategyParameters: {
					...draft.strategyParameters,
					[command.name]: command.value,
				},
			});
		case "select-factor": {
			if (
				command.archiveSha256 &&
				!command.compatibleHashes.includes(command.archiveSha256)
			)
				return draftError("incompatible-selection", "factor");
			const factorSelections = { ...draft.factorSelections };
			const factorParameters = { ...draft.factorParameters };
			if (command.archiveSha256) {
				if (factorSelections[command.alias] !== command.archiveSha256)
					delete factorParameters[command.alias];
				factorSelections[command.alias] = command.archiveSha256;
			} else {
				delete factorSelections[command.alias];
				delete factorParameters[command.alias];
			}
			return transitionConfig(session, {
				...draft,
				factorSelections,
				factorParameters,
			});
		}
		case "set-factor-parameter":
			return transitionConfig(session, {
				...draft,
				factorParameters: {
					...draft.factorParameters,
					[command.alias]: {
						...draft.factorParameters[command.alias],
						[command.name]: command.value,
					},
				},
			});
		case "select-signal": {
			if (
				(command.candidate && command.candidate.slot !== command.slot) ||
				(command.candidate &&
					!command.compatibleCandidates.some(
						(candidate) =>
							candidate.slot === command.candidate?.slot &&
							candidate.datasetId === command.candidate?.datasetId &&
							candidate.signalName === command.candidate?.signalName,
					))
			)
				return draftError("incompatible-selection", "signal");
			const signalSelections = { ...draft.signalSelections };
			if (command.candidate)
				signalSelections[command.slot] = {
					datasetId: command.candidate.datasetId,
					signalName: command.candidate.signalName,
				};
			else delete signalSelections[command.slot];
			return transitionConfig(session, { ...draft, signalSelections });
		}
		case "set-allocation":
			return transitionConfig(session, {
				...draft,
				initialQuoteAllocation: command.value,
			});
		case "set-execution-profile-field":
			return transitionConfig(session, {
				...draft,
				executionProfile: {
					...draft.executionProfile,
					[command.field]: command.value,
				},
			});
		case "set-fill-policy":
			return transitionConfig(session, {
				...draft,
				executionProfile: { ...draft.executionProfile, fillPolicy: command.value },
			});
		case "set-seed":
			return transitionConfig(session, { ...draft, seed: command.value });
		case "reconcile-factor-compatibility": {
			if (draft.strategy?.archiveSha256 !== command.strategyArchiveSha256)
				return { ok: true, value: { session, ignored: "stale-compatibility" } };
			const factorSelections = Object.fromEntries(
				Object.entries(draft.factorSelections).filter(([alias, hash]) =>
					command.compatibleHashes[alias]?.includes(hash),
				),
			);
			const factorParameters = Object.fromEntries(
				Object.entries(draft.factorParameters).filter(
					([alias]) => factorSelections[alias] !== undefined,
				),
			);
			return transitionConfig(session, {
				...draft,
				factorSelections,
				factorParameters,
			});
		}
		case "reconcile-signal-compatibility": {
			if (
				draft.strategy?.archiveSha256 !== command.strategyArchiveSha256 ||
				draft.snapshot?.snapshotId !== command.snapshotId
			)
				return { ok: true, value: { session, ignored: "stale-compatibility" } };
			const compatible = new Set(
				command.compatibleCandidates.map(
					(candidate) =>
						`${candidate.slot}:${candidate.datasetId}:${candidate.signalName}`,
				),
			);
			const signalSelections = Object.fromEntries(
				Object.entries(draft.signalSelections).filter(([slot, signal]) =>
					compatible.has(`${slot}:${signal.datasetId}:${signal.signalName}`),
				),
			);
			return transitionConfig(session, { ...draft, signalSelections });
		}
		case "enter-stage": {
			if (command.stage === "strategy" && !draft.snapshot)
				return draftError("incomplete-draft", "snapshot");
			if (command.stage !== "execution")
				return {
					ok: true,
					value: { session: { ...session, stage: command.stage } },
				};
			const requestResult = validateDraft(draft);
			if (!requestResult.ok) return requestResult;
			const request = withUserId(requestResult.value, command.userId);
			if (
				session.preflight.status === "ready" &&
				session.preflight.revision === draft.revision
			)
				return { ok: true, value: { session: { ...session, stage: "execution" } } };
			const nextSession: BacktestDraftSession = {
				...session,
				stage: "execution",
				preflight: { status: "pending", revision: draft.revision },
			};
			return {
				ok: true,
				value: {
					session: nextSession,
					effect: { kind: "preflight", revision: draft.revision, request },
				},
			};
		}
		case "accept-preflight":
			if (
				session.preflight.status !== "pending" ||
				session.preflight.revision !== command.revision ||
				command.revision !== draft.revision
			)
				return { ok: true, value: { session, ignored: "stale-preflight" } };
			return {
				ok: true,
				value: {
					session: {
						...session,
						preflight: {
							status: "ready",
							revision: command.revision,
							value: command.preflight,
						},
					},
				},
			};
		case "reject-preflight":
			if (
				session.preflight.status !== "pending" ||
				session.preflight.revision !== command.revision ||
				command.revision !== draft.revision
			)
				return { ok: true, value: { session, ignored: "stale-preflight" } };
			return {
				ok: true,
				value: { session: { ...session, preflight: absentPreflight() } },
			};
		case "request-run": {
			if (
				session.preflight.status !== "ready" ||
				session.preflight.revision !== draft.revision
			)
				return draftError("preflight-required");
			const requestResult = validateDraft(draft);
			if (!requestResult.ok) return requestResult;
			return {
				ok: true,
				value: {
					session,
					effect: {
						kind: "run",
						revision: draft.revision,
						request: withUserId(requestResult.value, command.userId),
					},
				},
			};
		}
		default: {
			const unreachable: never = command;
			return unreachable;
		}
	}
}
