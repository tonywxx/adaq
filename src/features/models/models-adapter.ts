import type { TauriInvoke } from "@/lib/tauri-invoke";
import type { LibraryComponent } from "@/features/components/component-library";
import type {
	Attempt,
	Dataset,
	DatasetGenerationRequest,
	EvaluationExportRequest,
	EvaluationReport,
	ForecastEvaluationRequest,
	RowPage,
	SignalDatasetRowsRequest,
	Snapshot,
} from "./models-types";

export function createModelsAdapter(invoke: TauriInvoke) {
	return {
		listComponents(userId: string) {
			return invoke("component_list", { request: { userId } }) as Promise<
				LibraryComponent[]
			>;
		},
		listSnapshots(userId: string) {
			return invoke("snapshot_list_readable", { request: { userId } }) as Promise<
				Snapshot[]
			>;
		},
		listAttempts(userId: string) {
			return invoke("dataset_generation_list", { userId }) as Promise<Attempt[]>;
		},
		listDatasets(userId: string) {
			return invoke("signal_dataset_list", { userId }) as Promise<Dataset[]>;
		},
		listEvaluations(userId: string) {
			return invoke("forecast_evaluation_list", { userId }) as Promise<
				EvaluationReport[]
			>;
		},
		listCompatibleFactors(userId: string, strategyArchiveSha256: string) {
			return invoke("backtest_compatible_factors", {
				request: { userId, strategyArchiveSha256 },
			}) as Promise<Record<string, string[]>>;
		},
		startDatasetGeneration(request: DatasetGenerationRequest) {
			return invoke("dataset_generation_start", { request }) as Promise<Attempt>;
		},
		cancelDatasetGeneration(userId: string, attemptId: string) {
			return invoke("dataset_generation_cancel", { attemptId, userId });
		},
		importSignalDataset(userId: string, archive: number[]) {
			return invoke("signal_dataset_import", { userId, archive });
		},
		exportSignalDataset(datasetId: string, userId: string) {
			return invoke("signal_dataset_export", { datasetId, userId }) as Promise<
				number[]
			>;
		},
		signalDatasetRows(request: SignalDatasetRowsRequest) {
			return invoke("signal_dataset_rows", request) as Promise<RowPage>;
		},
		retryDatasetGeneration(attemptId: string, userId: string) {
			return invoke("dataset_generation_retry", {
				attemptId,
				userId,
			}) as Promise<Attempt>;
		},
		createEvaluation(request: ForecastEvaluationRequest) {
			return invoke("forecast_evaluation_create", {
				request,
			}) as Promise<EvaluationReport>;
		},
		exportEvaluation(request: EvaluationExportRequest) {
			return invoke("forecast_evaluation_export", request) as Promise<string>;
		},
	};
}

export type ModelsAdapter = ReturnType<typeof createModelsAdapter>;
