import { createModelsAdapter } from "./models-adapter";

test("Models adapter preserves semantic command payloads", async () => {
	const calls: Array<{ command: string; args?: Record<string, unknown> }> = [];
	const adapter = createModelsAdapter(async (command, args = {}) => {
		calls.push({ command, args });
		return undefined;
	});

	await adapter.listComponents("user-1");
	await adapter.listSnapshots("user-1");
	await adapter.listAttempts("user-1");
	await adapter.listDatasets("user-1");
	await adapter.listEvaluations("user-1");
	await adapter.listCompatibleFactors("user-1", "model-hash");
	await adapter.startDatasetGeneration({
		userId: "user-1",
		snapshotId: "snapshot-1",
		modelArchiveSha256: "model-hash",
		modelParameters: { window: "20" },
		factorInstances: [{ alias: "alpha", archiveSha256: "factor-hash" }],
		seed: 0,
	});
	await adapter.cancelDatasetGeneration("user-1", "attempt-1");
	await adapter.importSignalDataset("user-1", [1, 2, 3]);
	await adapter.exportSignalDataset("dataset-1", "user-1");
	await adapter.signalDatasetRows({
		datasetId: "dataset-1",
		userId: "user-1",
		page: 2,
	});
	await adapter.retryDatasetGeneration("attempt-1", "user-1");
	await adapter.createEvaluation({
		userId: "user-1",
		datasetId: "dataset-1",
		snapshotId: "snapshot-1",
		signalName: "signal",
		horizonBars: 4,
		evaluationStartTimeMs: 10,
		evaluationEndTimeMs: 20,
		stabilityWindowBars: 8,
	});
	await adapter.exportEvaluation({
		reportId: "report-1",
		userId: "user-1",
		format: "markdown",
	});

	expect(calls).toEqual([
		{ command: "component_list", args: { request: { userId: "user-1" } } },
		{
			command: "snapshot_list_readable",
			args: { request: { userId: "user-1" } },
		},
		{ command: "dataset_generation_list", args: { userId: "user-1" } },
		{ command: "signal_dataset_list", args: { userId: "user-1" } },
		{ command: "forecast_evaluation_list", args: { userId: "user-1" } },
		{
			command: "backtest_compatible_factors",
			args: {
				request: { userId: "user-1", strategyArchiveSha256: "model-hash" },
			},
		},
		{
			command: "research_context_freeze",
			args: {
				userId: "user-1",
				operationId: "model-dataset:snapshot-1:model-hash",
				stage: "models",
			},
		},
		{
			command: "dataset_generation_start",
			args: {
				request: {
					userId: "user-1",
					operationId: "model-dataset:snapshot-1:model-hash",
					snapshotId: "snapshot-1",
					modelArchiveSha256: "model-hash",
					modelParameters: { window: "20" },
					factorInstances: [{ alias: "alpha", archiveSha256: "factor-hash" }],
					seed: 0,
				},
			},
		},
		{
			command: "dataset_generation_cancel",
			args: { attemptId: "attempt-1", userId: "user-1" },
		},
		{
			command: "signal_dataset_import",
			args: { userId: "user-1", archive: [1, 2, 3] },
		},
		{
			command: "signal_dataset_export",
			args: { datasetId: "dataset-1", userId: "user-1" },
		},
		{
			command: "signal_dataset_rows",
			args: { datasetId: "dataset-1", userId: "user-1", page: 2 },
		},
		{
			command: "dataset_generation_retry",
			args: { attemptId: "attempt-1", userId: "user-1" },
		},
		{
			command: "research_context_freeze",
			args: {
				userId: "user-1",
				operationId: "model-evaluation:dataset-1:10:20",
				stage: "models",
			},
		},
		{
			command: "forecast_evaluation_create",
			args: {
				request: {
					userId: "user-1",
					operationId: "model-evaluation:dataset-1:10:20",
					datasetId: "dataset-1",
					snapshotId: "snapshot-1",
					signalName: "signal",
					horizonBars: 4,
					evaluationStartTimeMs: 10,
					evaluationEndTimeMs: 20,
					stabilityWindowBars: 8,
				},
			},
		},
		{
			command: "forecast_evaluation_export",
			args: { reportId: "report-1", userId: "user-1", format: "markdown" },
		},
	]);
});
