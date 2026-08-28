import type {
	FactorAttemptView,
	FactorCandidateView,
	FactorComponentCandidateView,
	FactorDatasetRowsPage,
	FactorDatasetView,
	FactorDecisionView,
	FactorFamilyView,
	FactorJson,
	FactorLineageView,
	FactorMetricCatalogView,
	FactorPage,
	FactorPolicyView,
	FactorReportView,
	M12Eligibility,
} from "./factor-types";
import type { TauriInvoke } from "@/lib/tauri-invoke";

export type FactorInvoke = TauriInvoke;

export function createFactorAdapter(invoke: FactorInvoke) {
	const freezeContext = (userId: string, operationId: string) =>
		invoke("research_context_freeze", {
			userId,
			operationId,
			stage: "factors",
		});
	const page = <T>(
		command: string,
		userId: string,
		pageNumber: number,
		extra: Record<string, unknown> = {},
	) =>
		invoke(command, {
			request: { userId, page: pageNumber, pageSize: 50, ...extra },
		}) as Promise<FactorPage<T>>;

	return {
		listCandidates: (userId: string, pageNumber: number) =>
			page<FactorCandidateView>("factor_candidate_list", userId, pageNumber),
		getCandidate: (userId: string, evidenceId: string) =>
			invoke("factor_candidate_get", {
				request: { userId, evidenceId },
			}) as Promise<FactorCandidateView>,
		publishCandidate: (
			userId: string,
			draft: FactorJson,
			presentation: FactorJson,
		) =>
			invoke("factor_candidate_publish", {
				request: { userId, draft, presentation },
			}) as Promise<FactorCandidateView>,
		buildCandidate: (
			userId: string,
			candidate: FactorJson,
			presentation: FactorJson,
			build?: FactorJson,
		) => {
			const operationId = `factor-candidate-build:${crypto.randomUUID()}`;
			return invoke("factor_candidate_build", {
				request: {
					userId,
					operationId,
					candidate,
					presentation,
					build: build ?? null,
				},
			}) as Promise<FactorAttemptView>;
		},
		prepareComponent: (userId: string, decisionId: string, outputName: string) =>
			invoke("factor_component_prepare", {
				request: { userId, decisionId, outputName },
			}) as Promise<FactorAttemptView>,
		getComponentCandidate: (userId: string, attemptId: string) =>
			invoke("factor_component_candidate_get", {
				request: { userId, attemptId },
			}) as Promise<FactorComponentCandidateView>,
		listFamilies: (userId: string, pageNumber: number) =>
			page<FactorFamilyView>("factor_family_list", userId, pageNumber),
		getFamily: (userId: string, evidenceId: string) =>
			invoke("factor_family_get", {
				request: { userId, evidenceId },
			}) as Promise<FactorFamilyView>,
		getLineage: (userId: string, evidenceId: string) =>
			invoke("factor_lineage_get", {
				request: { userId, evidenceId },
			}) as Promise<FactorLineageView>,
		registerFamily: (
			userId: string,
			registration: FactorJson,
			trials: FactorJson[] = [],
		) =>
			invoke("factor_family_register", {
				request: { userId, registration, trials },
			}) as Promise<FactorFamilyView>,
		registerGridFamily: (userId: string, draft: FactorJson) =>
			invoke("factor_family_grid_register", {
				request: { userId, ...draft },
			}) as Promise<FactorAttemptView>,
		updateTrial: (userId: string, trial: FactorJson) =>
			invoke("factor_trial_update", { request: { userId, trial } }),
		listAttempts: (userId: string, pageNumber: number, kind?: string) =>
			page<FactorAttemptView>("factor_attempt_list", userId, pageNumber, {
				kind,
			}),
		getAttempt: (userId: string, attemptId: string) =>
			invoke("factor_attempt_get", {
				request: { userId, attemptId },
			}) as Promise<FactorAttemptView>,
		cancelAttempt: (userId: string, attemptId: string) =>
			invoke("factor_attempt_cancel", { request: { userId, attemptId } }),
		retryAttempt: (userId: string, attemptId: string) =>
			invoke("factor_attempt_retry", {
				request: { userId, attemptId },
			}) as Promise<FactorAttemptView>,
		retryComponentAttempt: (userId: string, attemptId: string) =>
			invoke("factor_component_retry", {
				request: { userId, attemptId },
			}) as Promise<FactorAttemptView>,
		listDatasets: (userId: string, pageNumber: number) =>
			page<FactorDatasetView>("factor_dataset_list", userId, pageNumber),
		getDataset: (userId: string, evidenceId: string) =>
			invoke("factor_dataset_get", {
				request: { userId, evidenceId },
			}) as Promise<FactorDatasetView>,
		datasetRows: (
			userId: string,
			datasetId: string,
			offset: number,
			limit = 50,
			instrumentId = "",
		) =>
			invoke("factor_dataset_rows", {
				request: {
					userId,
					datasetId,
					offset,
					limit,
					instrumentId: instrumentId.trim() || null,
				},
			}) as Promise<FactorDatasetRowsPage>,
		deleteDataset: (userId: string, evidenceId: string) =>
			invoke("factor_dataset_delete", {
				request: { userId, evidenceId },
			}),
		startMaterialization: (
			userId: string,
			protocol: FactorJson,
			dataset?: FactorJson,
		) => {
			const operationId = `factor-materialization:${crypto.randomUUID()}`;
			return freezeContext(userId, operationId).then(
				() =>
					invoke("factor_materialization_start", {
						request: {
							userId,
							operationId,
							protocol,
							dataset: dataset ?? null,
						},
					}) as Promise<FactorAttemptView>,
			);
		},
		startMaterializationFromContext: (
			userId: string,
			candidateHash: string,
			seed = 0,
		) => {
			const operationId = `factor-materialization:${crypto.randomUUID()}`;
			return invoke("factor_materialization_start_from_context", {
				request: { userId, operationId, candidateHash, seed },
			}) as Promise<FactorAttemptView>;
		},
		freezeMaterializationProtocol: (userId: string, draft: FactorJson) =>
			invoke("factor_materialization_protocol_freeze", {
				request: { userId, draft },
			}) as Promise<FactorJson>,
		listReports: (userId: string, pageNumber: number) =>
			page<FactorReportView>("factor_report_list", userId, pageNumber),
		getReport: (userId: string, evidenceId: string) =>
			invoke("factor_report_get", {
				request: { userId, evidenceId },
			}) as Promise<FactorReportView>,
		startEvaluation: (
			userId: string,
			protocol: FactorJson,
			marketSeries: FactorJson[],
			featureEvidence?: FactorJson,
			dataset?: FactorJson,
		) => {
			const operationId = `factor-evaluation:${crypto.randomUUID()}`;
			return freezeContext(userId, operationId).then(
				() =>
					invoke("factor_evaluation_start", {
						request: {
							userId,
							operationId,
							protocol,
							dataset: dataset ?? null,
							marketSeries,
							featureEvidence: featureEvidence ?? null,
						},
					}) as Promise<FactorAttemptView>,
			);
		},
		startEvaluationFromContext: (
			userId: string,
			candidateHash: string,
			datasetId: string,
			outputName: string,
			seed = 0,
		) => {
			const operationId = `factor-evaluation:${crypto.randomUUID()}`;
			return invoke("factor_evaluation_start_from_context", {
				request: {
					userId,
					operationId,
					candidateHash,
					datasetId,
					outputName,
					seed,
				},
			}) as Promise<FactorAttemptView>;
		},
		freezeEvaluationProtocol: (userId: string, draft: FactorJson) =>
			invoke("factor_evaluation_protocol_freeze", {
				request: { userId, draft },
			}) as Promise<FactorJson>,
		listPolicies: (userId: string, pageNumber: number) =>
			page<FactorPolicyView>("factor_policy_list", userId, pageNumber),
		savePolicy: (userId: string, policy: FactorJson) =>
			invoke("factor_policy_save", {
				request: { userId, policy },
			}) as Promise<FactorPolicyView>,
		freezePromotionProtocol: (userId: string, selection: FactorJson) =>
			invoke("factor_promotion_protocol_freeze", {
				request: { userId, ...selection },
			}) as Promise<FactorJson>,
		recordDecision: (
			userId: string,
			state: string,
			promotionProtocol: FactorJson,
			component: FactorJson = {},
			supersedes: string | null = null,
		) =>
			invoke("factor_decision_record", {
				request: { userId, state, promotionProtocol, component, supersedes },
			}) as Promise<FactorDecisionView>,
		listDecisions: (userId: string, pageNumber: number) =>
			page<FactorDecisionView>("factor_decision_list", userId, pageNumber),
		listDecisionLibrary: (userId: string, pageNumber: number) =>
			page<FactorDecisionView>("factor_decision_library", userId, pageNumber),
		saveDecision: (
			userId: string,
			decision: FactorJson,
			promotionProtocol: FactorJson,
			component: FactorJson = {},
		) =>
			invoke("factor_decision_save", {
				request: { userId, decision, promotionProtocol, component },
			}) as Promise<FactorDecisionView>,
		m12Eligibility: (userId: string, promotionProtocol: FactorJson) =>
			invoke("factor_m12_eligibility", {
				request: { userId, promotionProtocol },
			}) as Promise<M12Eligibility>,
		metricCatalog: () =>
			invoke("factor_metric_catalog", {}) as Promise<FactorMetricCatalogView>,
	};
}

export type FactorAdapter = ReturnType<typeof createFactorAdapter>;
