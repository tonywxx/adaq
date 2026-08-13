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
	| "cancelled";

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
