import type {
	ArtifactView,
	DefinitionDraft,
	DefinitionView,
	DraftValidationView,
	FeatureDatasetFilter,
	FeatureDatasetPage,
	FeatureDatasetView,
	FeatureMaterializationRequest,
	FeatureObservation,
	FeatureOutputSummary,
	FeaturePlanDraft,
	FittingAttemptView,
	MarketDataSnapshotSummary,
	MaterializationAttempt,
	PlanFreezeView,
	TransformationFittingProtocolDraft,
	UniverseSnapshotSummary,
} from "./features-types";
import type { TauriInvoke } from "@/lib/tauri-invoke";

export type FeatureInvoke = TauriInvoke;

export type FeaturePreviewSelection = {
	snapshotId?: string;
	universeId?: string;
	valuationCurrency?: string;
	startTimeMs?: number;
	endTimeMs?: number;
	maxEvents?: number;
	artifactIds: string[];
};

// Thin typed adapter over the frozen Feature Tauri commands. The invoke
// transport is injected so helpers stay testable outside the Tauri runtime.
export function createFeaturesAdapter(invoke: FeatureInvoke) {
	const freezeContext = (userId: string, operationId: string) =>
		invoke("research_context_freeze", {
			userId,
			operationId,
			stage: "features",
		});

	return {
		async listDefinitions(userId: string) {
			return invoke("feature_definition_list", {
				request: { userId },
			}) as Promise<DefinitionView[]>;
		},
		async getDefinition(userId: string, definitionHash: string) {
			return invoke("feature_definition_get", {
				request: { userId, definitionHash },
			}) as Promise<DefinitionView>;
		},
		async validateDraft(userId: string, draft: DefinitionDraft) {
			return invoke("feature_definition_validate", {
				request: { userId, draft },
			}) as Promise<DraftValidationView>;
		},
		async publishDefinition(
			userId: string,
			draft: DefinitionDraft,
			presentation: { name: string; description: string; tags: string[] },
		) {
			return invoke("feature_definition_publish", {
				request: { userId, draft, ...presentation },
			}) as Promise<DefinitionView>;
		},
		async previewDraft(
			userId: string,
			draft: DefinitionDraft,
			selection: FeaturePreviewSelection,
		) {
			return invoke("feature_definition_preview", {
				request: {
					userId,
					draft,
					snapshotId: selection.snapshotId ?? null,
					universeId: selection.universeId ?? null,
					valuationCurrency: selection.valuationCurrency ?? null,
					startTimeMs: selection.startTimeMs ?? null,
					endTimeMs: selection.endTimeMs ?? null,
					maxEvents: selection.maxEvents ?? null,
					artifactIds: selection.artifactIds,
				},
			}) as Promise<{
				observations: FeatureObservation[];
				eventCount: number;
				truncated: boolean;
			}>;
		},
		async freezePlan(userId: string, plan: FeaturePlanDraft) {
			return invoke("feature_plan_freeze", {
				request: { userId, plan },
			}) as Promise<PlanFreezeView>;
		},
		async startFitting(
			userId: string,
			protocol: TransformationFittingProtocolDraft,
			plan: FeaturePlanDraft,
		) {
			const operationId = `feature-fitting:${crypto.randomUUID()}`;
			await freezeContext(userId, operationId);
			return invoke("feature_fitting_start", {
				request: { userId, operationId, protocol, plan },
			}) as Promise<FittingAttemptView>;
		},
		async listFittingAttempts(userId: string) {
			return invoke("feature_fitting_list", {
				request: { userId },
			}) as Promise<FittingAttemptView[]>;
		},
		async cancelFitting(userId: string, attemptId: string) {
			await invoke("feature_fitting_cancel", {
				request: { userId, attemptId },
			});
		},
		async retryFitting(userId: string, attemptId: string) {
			return invoke("feature_fitting_retry", {
				request: { userId, attemptId },
			}) as Promise<FittingAttemptView>;
		},
		async listArtifacts(userId: string) {
			return invoke("feature_artifact_list", {
				request: { userId },
			}) as Promise<ArtifactView[]>;
		},
		async deleteArtifact(userId: string, artifactId: string) {
			await invoke("feature_artifact_delete", {
				request: { userId, artifactId },
			});
		},
		async startMaterialization(
			userId: string,
			request: FeatureMaterializationRequest,
			plan: FeaturePlanDraft,
		) {
			const operationId = `feature-materialization:${crypto.randomUUID()}`;
			await freezeContext(userId, operationId);
			return invoke("feature_materialization_start", {
				request: { userId, operationId, request, plan },
			}) as Promise<MaterializationAttempt>;
		},
		async listMaterializationAttempts(userId: string) {
			return invoke("feature_materialization_list", {
				request: { userId },
			}) as Promise<MaterializationAttempt[]>;
		},
		async cancelMaterialization(userId: string, attemptId: string) {
			await invoke("feature_materialization_cancel", {
				request: { userId, attemptId },
			});
		},
		async retryMaterialization(userId: string, attemptId: string) {
			return invoke("feature_materialization_retry", {
				request: { userId, attemptId },
			}) as Promise<MaterializationAttempt>;
		},
		async listDatasets(userId: string) {
			return invoke("feature_dataset_list", {
				request: { userId },
			}) as Promise<FeatureDatasetView[]>;
		},
		async getDataset(userId: string, datasetId: string) {
			return invoke("feature_dataset_get", {
				request: { userId, datasetId },
			}) as Promise<FeatureDatasetView>;
		},
		async datasetSummary(userId: string, datasetId: string) {
			return invoke("feature_dataset_summary", {
				request: { userId, datasetId },
			}) as Promise<FeatureOutputSummary[]>;
		},
		async datasetRows(
			userId: string,
			datasetId: string,
			filter: FeatureDatasetFilter,
			offset: number,
		) {
			return invoke("feature_dataset_rows", {
				request: { userId, datasetId, filter, offset },
			}) as Promise<FeatureDatasetPage>;
		},
		async deleteDataset(userId: string, datasetId: string) {
			await invoke("feature_dataset_delete", {
				request: { userId, datasetId },
			});
		},
		async listSnapshots(userId: string) {
			return invoke("snapshot_list_readable", {
				request: { userId },
			}) as Promise<MarketDataSnapshotSummary[]>;
		},
		async listUniverseSnapshots(userId: string) {
			const page = (await invoke("snapshot_list_universe", {
				request: { userId, page: 1 },
			})) as { items: UniverseSnapshotSummary[] };
			return page.items;
		},
	};
}

export type FeaturesAdapter = ReturnType<typeof createFeaturesAdapter>;
