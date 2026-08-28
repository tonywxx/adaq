import { createFactorAdapter } from "./factor-adapter";

test("Factor adapter keeps User scope and immutable command payloads explicit", async () => {
	const calls: Array<{ command: string; args: Record<string, unknown> }> = [];
	const adapter = createFactorAdapter(async (command, args = {}) => {
		calls.push({ command, args });
		return {};
	});

	await adapter.publishCandidate(
		"user-1",
		{ candidateId: "candidate-1" },
		{ name: "demo" },
	);
	await adapter.buildCandidate(
		"user-1",
		{ candidateId: "candidate-1" },
		{ name: "demo" },
	);
	await adapter.registerGridFamily("user-1", {
		familyId: "family-1",
		candidateHash: "a".repeat(64),
		parameters: [],
	});
	await adapter.datasetRows("user-1", "dataset-1", 0, 50, "BTC");
	await adapter.freezeEvaluationProtocol("user-1", { protocolId: "draft-1" });
	await adapter.startMaterializationFromContext("user-1", "candidate-hash", 7);
	await adapter.startEvaluationFromContext(
		"user-1",
		"candidate-hash",
		"dataset-id",
		"momentum-score",
	);

	expect(calls).toEqual([
		{
			command: "factor_candidate_publish",
			args: {
				request: {
					userId: "user-1",
					draft: { candidateId: "candidate-1" },
					presentation: { name: "demo" },
				},
			},
		},
		{
			command: "factor_candidate_build",
			args: {
				request: {
					userId: "user-1",
					operationId: expect.stringMatching(/^factor-candidate-build:/),
					candidate: { candidateId: "candidate-1" },
					presentation: { name: "demo" },
					build: null,
				},
			},
		},
		{
			command: "factor_family_grid_register",
			args: {
				request: {
					userId: "user-1",
					familyId: "family-1",
					candidateHash: "a".repeat(64),
					parameters: [],
				},
			},
		},
		{
			command: "factor_dataset_rows",
			args: {
				request: {
					userId: "user-1",
					datasetId: "dataset-1",
					offset: 0,
					limit: 50,
					instrumentId: "BTC",
				},
			},
		},
		{
			command: "factor_evaluation_protocol_freeze",
			args: {
				request: {
					userId: "user-1",
					draft: { protocolId: "draft-1" },
				},
			},
		},
		{
			command: "factor_materialization_start_from_context",
			args: {
				request: {
					userId: "user-1",
					operationId: expect.any(String),
					candidateHash: "candidate-hash",
					seed: 7,
				},
			},
		},
		{
			command: "factor_evaluation_start_from_context",
			args: {
				request: {
					userId: "user-1",
					operationId: expect.stringMatching(/^factor-evaluation:/),
					candidateHash: "candidate-hash",
					datasetId: "dataset-id",
					outputName: "momentum-score",
					seed: 0,
				},
			},
		},
	]);
});
