import type {
	DefinitionDraft,
	FeatureDatasetFilter,
	FeatureEngineIdentity,
	FeatureInput,
	FeatureNodeDraft,
	FeaturePlanDraft,
	FeatureScope,
	MarketField,
	StoredDefinition,
} from "./features-types";

// The backend always replaces the engine identity with the native one
// before freezing; the GUI submits the empty shape.
export const EMPTY_ENGINE_IDENTITY: FeatureEngineIdentity = {
	featureEngineVersion: "",
	featureEngineSourceSha256: "",
	featureEngineBuildId: "",
	operatorCatalogVersion: "",
	indicatorEngineVersion: "",
	indicatorCatalogVersion: "",
	taLibVersion: "",
	taSourceSha256: "",
	wrapperSha256: "",
	targetTriple: "",
	compilerAndFlagsSha256: "",
	engineBuildId: "",
};

export const MARKET_FIELDS: MarketField[] = [
	"open",
	"high",
	"low",
	"close",
	"base-volume",
	"quote-volume",
];

export const FEATURE_SCOPES: FeatureScope[] = [
	"pointwise",
	"time-series",
	"cross-sectional",
];

// The frozen initial operator catalog; operator ids are stable contract
// identifiers and are displayed verbatim in both locales.
export const OPERATOR_KINDS: string[] = [
	"checked-arithmetic",
	"indicator",
	"backward-simple-return",
	"backward-log-return",
	"rolling-mean",
	"rolling-population-standard-deviation",
	"rolling-minimum",
	"rolling-maximum",
	"realized-volatility",
	"quote-volume",
	"rolling-quote-volume",
	"zero-volume",
	"amihud-illiquidity",
	"trading-day-of-week",
	"trading-month",
	"minutes-from-session-open",
	"minutes-to-session-close",
	"session-progress",
	"one-hot",
	"sine",
	"cosine",
	"cross-sectional-rank",
	"cross-sectional-percentile",
	"cross-sectional-z-score",
	"causal-split-adjustment",
	"dividend-total-return",
	"standardization",
	"winsorization",
];

export const UNAVAILABILITY_REASONS: string[] = [
	"warmup",
	"bar-gap",
	"missing-market-input",
	"missing-dependency",
	"unknown-universe",
	"insufficient-coverage",
	"undefined-arithmetic",
	"artifact-missing-instrument",
	"corporate-action-unavailable",
];

export const DATASET_PAGE_SIZE = 50;

export function operatorLabel(kind: string, id?: string): string {
	return kind === "indicator" && id ? `indicator:${id}` : kind;
}

export function defaultNodeId(draft: DefinitionDraft): string {
	let index = draft.nodes.length + 1;
	let id = `node-${index}`;
	const taken = new Set(draft.nodes.map((node) => node.id));
	while (taken.has(id)) {
		index += 1;
		id = `node-${index}`;
	}
	return id;
}

export function createEmptyNode(
	id: string,
	scope: FeatureScope,
): FeatureNodeDraft {
	return {
		id,
		operator: { kind: "backward-simple-return" },
		scope,
		inputs: [{ kind: "market", field: "close" }],
		parameters: {},
		warmupBars: 1,
	};
}

export function createEmptyDraft(): DefinitionDraft {
	const scope: FeatureScope = "time-series";
	const node = createEmptyNode("node-1", scope);
	return {
		definitionId: crypto.randomUUID(),
		revision: 1,
		scope,
		nodes: [node],
		outputs: [{ name: "output-1", nodeId: node.id }],
	};
}

export function moveNode(
	draft: DefinitionDraft,
	index: number,
	direction: -1 | 1,
): DefinitionDraft {
	const target = index + direction;
	if (index < 0 || index >= draft.nodes.length || target < 0) return draft;
	if (target >= draft.nodes.length) return draft;
	const nodes = [...draft.nodes];
	const [node] = nodes.splice(index, 1);
	nodes.splice(target, 0, node);
	return { ...draft, nodes };
}

export function removeNode(
	draft: DefinitionDraft,
	index: number,
): DefinitionDraft {
	const node = draft.nodes[index];
	if (!node) return draft;
	return {
		...draft,
		nodes: draft.nodes.filter((_, position) => position !== index),
		outputs: draft.outputs.filter((output) => output.nodeId !== node.id),
	};
}

export function updateNode(
	draft: DefinitionDraft,
	index: number,
	patch: Partial<FeatureNodeDraft>,
): DefinitionDraft {
	const nodes = draft.nodes.map((node, position) =>
		position === index ? { ...node, ...patch } : node,
	);
	return { ...draft, nodes };
}

export function addMarketInput(
	draft: DefinitionDraft,
	index: number,
	field: MarketField,
): DefinitionDraft {
	const node = draft.nodes[index];
	if (!node) return draft;
	const input: FeatureInput = { kind: "market", field };
	return updateNode(draft, index, { inputs: [...node.inputs, input] });
}

export function removeInput(
	draft: DefinitionDraft,
	nodeIndex: number,
	inputIndex: number,
): DefinitionDraft {
	const node = draft.nodes[nodeIndex];
	if (!node) return draft;
	return updateNode(draft, nodeIndex, {
		inputs: node.inputs.filter((_, position) => position !== inputIndex),
	});
}

export function parseParameterValue(text: string): unknown {
	const trimmed = text.trim();
	if (trimmed === "") return null;
	try {
		return JSON.parse(trimmed);
	} catch {
		return trimmed;
	}
}

export function parseStoredDefinition(
	definitionJson: string,
): StoredDefinition | null {
	try {
		const parsed = JSON.parse(definitionJson) as StoredDefinition;
		if (!parsed || !Array.isArray(parsed.nodes)) return null;
		return parsed;
	} catch {
		return null;
	}
}

export function emptyPlanDraft(
	definitions: StoredDefinition[],
): FeaturePlanDraft {
	return {
		definitions,
		slots: [],
		factors: [],
		artifacts: [],
		consumerPackageSha256: "",
		consumerParameters: [],
		consumerWarmupBars: 0,
		engineIdentity: EMPTY_ENGINE_IDENTITY,
	};
}

export function isTerminalAttemptStatus(status: string): boolean {
	return status === "completed" || status === "failed" || status === "cancelled";
}

export function attemptProgressFraction(
	completed: number,
	total: number,
): number {
	if (total <= 0) return 0;
	return Math.min(1, Math.max(0, completed / total));
}

export function datasetPageOffset(page: number): number {
	return Math.max(0, (page - 1) * DATASET_PAGE_SIZE);
}

export function buildDatasetFilter(form: {
	instrumentId: string;
	startTimeMs?: number;
	endTimeMs?: number;
	outputName: string;
	state: string;
}): FeatureDatasetFilter {
	const filter: FeatureDatasetFilter = { limit: DATASET_PAGE_SIZE };
	if (form.instrumentId.trim()) filter.instrumentId = form.instrumentId.trim();
	if (form.startTimeMs !== undefined) filter.startTimeMs = form.startTimeMs;
	if (form.endTimeMs !== undefined) filter.endTimeMs = form.endTimeMs;
	if (form.outputName) filter.outputName = form.outputName;
	if (form.state === "available" || form.state === "unavailable")
		filter.state = form.state;
	return filter;
}

// datetime-local inputs are device-local; the contract stores UTC epoch ms.
export function parseTimestampInput(value: string): number | undefined {
	if (!value) return undefined;
	const parsed = new Date(value).getTime();
	return Number.isFinite(parsed) ? parsed : undefined;
}

export function formatFeatureError(error: unknown): string {
	if (typeof error === "string") return error;
	if (error instanceof Error) return error.message;
	try {
		return JSON.stringify(error);
	} catch {
		return String(error);
	}
}

// kebab-case unavailability reasons map to camelCase i18n keys.
export function camelReason(reason: string): string {
	return reason.replace(/-([a-z])/g, (_, letter: string) =>
		letter.toUpperCase(),
	);
}

// ---- User-scoped current-session list caches ----
// The cache renders re-entries instantly and is invalidated by User; a
// background refresh always runs and only replaces the cache on success.

const sessionCaches: Map<string, unknown> = new Map();

function sessionCacheKey(userId: string, resource: string): string {
	return `${userId}:${resource}`;
}

export function readSessionCache(
	userId: string | null,
	resource: string,
): unknown {
	if (!userId) return undefined;
	return sessionCaches.get(sessionCacheKey(userId, resource));
}

export function writeSessionCache(
	userId: string | null,
	resource: string,
	value: unknown,
): void {
	if (!userId) return;
	sessionCaches.set(sessionCacheKey(userId, resource), value);
}

export function clearSessionCache(): void {
	sessionCaches.clear();
}
