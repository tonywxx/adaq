import {
	DATASET_PAGE_SIZE,
	attemptProgressFraction,
	buildDatasetFilter,
	camelReason,
	clearSessionCache,
	createEmptyDraft,
	datasetPageOffset,
	defaultNodeId,
	emptyPlanDraft,
	isTerminalAttemptStatus,
	moveNode,
	parseParameterValue,
	parseStoredDefinition,
	readSessionCache,
	removeNode,
	updateNode,
	writeSessionCache,
} from "./features-data";

test("creates a draft with one ordered node and one bound output", () => {
	const draft = createEmptyDraft();

	expect(draft.nodes).toHaveLength(1);
	expect(draft.outputs).toEqual([{ name: "output-1", nodeId: "node-1" }]);
	expect(draft.revision).toBe(1);
	expect(draft.definitionId).toMatch(
		/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i,
	);
	expect(defaultNodeId(draft)).toBe("node-2");
});

test("keeps node order keyboard-operable without dropping entries", () => {
	let draft = createEmptyDraft();
	draft = {
		...draft,
		nodes: [
			...draft.nodes,
			{ ...draft.nodes[0], id: "node-2" },
			{ ...draft.nodes[0], id: "node-3" },
		],
	};

	const moved = moveNode(draft, 0, 1);
	expect(moved.nodes.map((node) => node.id)).toEqual([
		"node-2",
		"node-1",
		"node-3",
	]);
	// Boundary moves are no-ops, never reorders out of range.
	expect(moveNode(draft, 0, -1)).toBe(draft);
	expect(moveNode(draft, 2, 1)).toBe(draft);
	expect(moveNode(draft, 5, -1)).toBe(draft);
});

test("removing a node also removes the outputs bound to it", () => {
	let draft = createEmptyDraft();
	draft = {
		...draft,
		nodes: [...draft.nodes, { ...draft.nodes[0], id: "node-2" }],
		outputs: [...draft.outputs, { name: "output-2", nodeId: "node-2" }],
	};

	const withoutFirst = removeNode(draft, 0);
	expect(withoutFirst.nodes.map((node) => node.id)).toEqual(["node-2"]);
	expect(withoutFirst.outputs).toEqual([{ name: "output-2", nodeId: "node-2" }]);
	expect(
		updateNode(withoutFirst, 0, { warmupBars: 7 }).nodes[0].warmupBars,
	).toBe(7);
});

test("parses parameter values as JSON with a plain-string fallback", () => {
	expect(parseParameterValue("14")).toBe(14);
	expect(parseParameterValue('{"a":1}')).toEqual({ a: 1 });
	expect(parseParameterValue("RSI")).toBe("RSI");
	expect(parseParameterValue("  ")).toBeNull();
});

test("reads stored frozen Definition documents and rejects malformed JSON", () => {
	const stored = parseStoredDefinition(
		JSON.stringify({ definitionId: "d", revision: 2, nodes: [] }),
	);
	expect(stored?.revision).toBe(2);
	expect(parseStoredDefinition("{not-json")).toBeNull();
	expect(parseStoredDefinition(JSON.stringify({ revision: 1 }))).toBeNull();
});

test("builds Plan drafts with the empty engine identity for native replacement", () => {
	const plan = emptyPlanDraft([]);
	expect(plan.engineIdentity.featureEngineVersion).toBe("");
	expect(plan.slots).toEqual([]);
	expect(plan.artifacts).toEqual([]);
});

test("dataset filters are bounded to 50 rows and ignore blank fields", () => {
	const filter = buildDatasetFilter({
		instrumentId: "  okx:BTC-USDT  ",
		outputName: "",
		state: "nonsense",
	});
	expect(filter).toEqual({
		instrumentId: "okx:BTC-USDT",
		limit: DATASET_PAGE_SIZE,
	});
	expect(DATASET_PAGE_SIZE).toBe(50);
	expect(
		buildDatasetFilter({
			instrumentId: "",
			startTimeMs: 1,
			endTimeMs: 2,
			outputName: "return",
			state: "unavailable",
		}),
	).toEqual({
		startTimeMs: 1,
		endTimeMs: 2,
		outputName: "return",
		state: "unavailable",
		limit: 50,
	});
	expect(datasetPageOffset(1)).toBe(0);
	expect(datasetPageOffset(3)).toBe(100);
	expect(datasetPageOffset(0)).toBe(0);
});

test("attempt progress stays bounded and terminal statuses stop polling", () => {
	expect(attemptProgressFraction(0, 0)).toBe(0);
	expect(attemptProgressFraction(5, 10)).toBe(0.5);
	expect(attemptProgressFraction(12, 10)).toBe(1);
	expect(isTerminalAttemptStatus("pending")).toBe(false);
	expect(isTerminalAttemptStatus("running")).toBe(false);
	expect(isTerminalAttemptStatus("completed")).toBe(true);
	expect(isTerminalAttemptStatus("failed")).toBe(true);
	expect(isTerminalAttemptStatus("cancelled")).toBe(true);
});

test("maps typed unavailability reasons to localization keys", () => {
	expect(camelReason("bar-gap")).toBe("barGap");
	expect(camelReason("missing-market-input")).toBe("missingMarketInput");
	expect(camelReason("warmup")).toBe("warmup");
});

test("session caches are User-scoped and never shared across Users", () => {
	clearSessionCache();
	writeSessionCache("alice", "definitions", [1]);
	writeSessionCache("bob", "definitions", [2]);
	writeSessionCache(null, "definitions", [3]);

	const read = (user: string | null, resource: string) =>
		readSessionCache(user, resource) as number[] | undefined;

	expect(read("alice", "definitions")).toEqual([1]);
	expect(read("bob", "fitting")).toBeUndefined();
	expect(read(null, "definitions")).toBeUndefined();
	writeSessionCache("alice", "definitions", [4]);
	expect(read("alice", "definitions")).toEqual([4]);
	clearSessionCache();
	expect(read("alice", "definitions")).toBeUndefined();
});
