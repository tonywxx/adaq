import type {
	FeatureDatasetBinding,
	ResearchEvidenceBinding,
	ResearchEvidenceProjection,
} from "@/features/research/research-context-preflight";

export type FactorJson = Record<string, unknown>;

export type FactorMetricCatalogView = {
	schemaVersion: string;
	catalogVersion: string;
	definitions: FactorJson[];
	catalogHash: string;
};

export type FactorGate = {
	gate: string;
	passed: boolean;
};

export type FactorAttemptStatus =
	| "pending"
	| "running"
	| "completed"
	| "failed"
	| "cancelled"
	| "interrupted"
	| "stale";

export type FactorFailureCode =
	| "cancelled"
	| "factor-component-build-failed"
	| "factor-component-qualification-failed"
	| "candidate-build-failed"
	| "factor-compatibility-failed"
	| "factor-corruption-detected"
	| "factor-evaluation-failed"
	| "factor-family-grid-failed"
	| "factor-materialization-failed"
	| "factor-missing-input"
	| "factor-publication-failed"
	| "factor-research-failed"
	| "factor-resource-failed"
	| "factor-validation-failed"
	| "research-interrupted"
	| "reset-required"
	| `factor-context-${string}`;

export type FactorAttemptView = {
	attemptId: string;
	userId: string;
	kind: string;
	requestHash: string;
	status: FactorAttemptStatus;
	sourceAttemptId?: string | null;
	resultId?: string | null;
	completedUnits: number;
	progressTotal: number;
	diagnostic?: string | null;
	failureCode?: FactorFailureCode | null;
	createdAtMs: number;
	updatedAtMs: number;
};

export type FactorPage<T> = {
	items: T[];
	page: number;
	pageSize: number;
	total: number;
};

export type FactorCandidateView = {
	candidate: FactorJson;
	presentation: {
		name: string;
		description?: string;
		tags?: string[];
	};
	lockedBy: string[];
	createdAtMs: number;
	predecessor?: FactorCandidatePredecessor | null;
};

export type FactorComponentCandidateView = {
	attemptId: string;
	userId: string;
	packageSha256: string;
	manifest: FactorJson;
	binding: FactorJson;
};

export type FactorComponentQualificationView = {
	attempt: FactorAttemptView;
	candidateAttemptId?: string | null;
	packageSha256?: string | null;
	binding?: FactorJson | null;
	qualification?: FactorJson | null;
	provenance?: FactorJson | null;
	equivalence?: FactorJson | null;
	published: boolean;
	evidenceCreatedAtMs?: number | null;
};

export type FactorCandidatePredecessor = ResearchEvidenceProjection & {
	userId: string;
	featureDataset: FeatureDatasetBinding;
	evidence: ResearchEvidenceBinding[];
};

export type FactorDatasetView = {
	manifest: FactorJson;
	byteSize: number;
	lockedBy: string[];
	createdAtMs: number;
};

export type FactorDatasetRow = FactorJson & {
	instrumentId?: string;
	observationTimeMs?: number;
	values?: FactorJson;
};

export type FactorDatasetRowsPage = {
	rows: FactorDatasetRow[];
	offset: number;
	limit: number;
	nextOffset?: number | null;
	total: number;
};

export type FactorFamilyView = {
	family: FactorJson;
	trialCount: number;
	lineageHash: string;
};

export type FactorReportView = {
	report: FactorJson;
	protocol?: FactorJson | null;
	lockedBy: string[];
	createdAtMs: number;
};

export type FactorPolicyView = {
	policy: FactorJson;
	createdAtMs: number;
};

export type FactorDecisionView = {
	decision: FactorJson;
	promotionProtocolHash: string;
	eligibilityGates: FactorGate[];
	createdAtMs: number;
};

export type FactorLineageView = {
	lineage: FactorJson;
	trials: FactorJson[];
	registrations: FactorJson[];
	protocols: FactorJson[];
};

export type M12Eligibility = {
	eligible: boolean;
	reason?: string | null;
	gates: FactorGate[];
};
