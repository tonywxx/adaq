// Feature Workspace contract types. Shapes mirror the frozen camelCase
// serde contracts of the Feature lifecycle module (src-tauri/src/features)
// and the adaq-feature-engine crate.

export type FeatureScope = "pointwise" | "time-series" | "cross-sectional";

export type MarketField =
	| "open"
	| "high"
	| "low"
	| "close"
	| "base-volume"
	| "quote-volume";

// Internally tagged union; only the Indicator variant carries an extra id.
export type FeatureOperator = { kind: string; id?: string };

export type FeatureInput =
	| { kind: "market"; field: MarketField }
	| { kind: "node"; nodeId: string; definitionHash?: string }
	| { kind: "artifact"; artifactId: string };

export type FeatureNodeDraft = {
	id: string;
	operator: FeatureOperator;
	scope: FeatureScope;
	inputs: FeatureInput[];
	parameters: Record<string, unknown>;
	warmupBars: number;
};

export type FeatureOutputDraft = {
	name: string;
	nodeId: string;
};

export type DefinitionDraft = {
	definitionId: string;
	revision: number;
	scope: FeatureScope;
	nodes: FeatureNodeDraft[];
	outputs: FeatureOutputDraft[];
};

export type ValidationIssue = {
	code: string;
	path?: string | null;
};

export type DraftValidationView = {
	valid: boolean;
	issues: ValidationIssue[];
};

export type DefinitionView = {
	definitionId: string;
	revision: number;
	definitionHash: string;
	definitionJson: string;
	name: string;
	description: string;
	tags: string[];
	createdAtMs: number;
};

export type FeatureReference = {
	definitionHash: string;
	nodeId: string;
	outputName: string;
};

// FeatureEngineIdentity is camelCase with deny_unknown_fields on the wire.
// The backend always replaces it with the native identity, so the GUI
// submits empty strings.
export type FeatureEngineIdentity = {
	featureEngineVersion: string;
	featureEngineSourceSha256: string;
	featureEngineBuildId: string;
	operatorCatalogVersion: string;
	indicatorEngineVersion: string;
	indicatorCatalogVersion: string;
	taLibVersion: string;
	taSourceSha256: string;
	wrapperSha256: string;
	targetTriple: string;
	compilerAndFlagsSha256: string;
	engineBuildId: string;
};

// Frozen Definition document as stored in definitionJson: flattened content
// plus the immutable hash.
export type StoredDefinition = {
	definitionSchemaVersion: string;
	definitionId: string;
	revision: number;
	scope: FeatureScope;
	operatorCatalogVersion: string;
	nodes: FeatureNodeDraft[];
	outputs: FeatureOutputDraft[];
	definitionHash: string;
};

export type PlanFreezeView = {
	planHash: string;
	planJson: string;
};

export type FeaturePlanDraft = {
	definitions: StoredDefinition[];
	slots: unknown[];
	factors: unknown[];
	artifacts: FittedArtifactBinding[];
	consumerPackageSha256: string;
	consumerParameters: Array<{ name: string; value: unknown }>;
	consumerWarmupBars: number;
	engineIdentity: FeatureEngineIdentity;
};

export type FittedArtifactBinding = {
	artifactId: string;
	eligibleAtMs: number;
	fittedOutput: FeatureReference;
};

export type FeatureAttemptStatus =
	| "pending"
	| "running"
	| "completed"
	| "failed"
	| "cancelled";

export type FittingAttemptView = {
	attemptId: string;
	userId: string;
	protocolHash: string;
	planHash: string;
	status: FeatureAttemptStatus;
	sourceAttemptId?: string | null;
	artifactId?: string | null;
	failureCode?: string | null;
	diagnostic?: string | null;
	progressCompleted: number;
	progressTotal: number;
	createdAtMs: number;
	updatedAtMs: number;
};

export type ArtifactView = {
	artifactId: string;
	protocolHash: string;
	artifactJson: string;
	createdAtMs: number;
};

export type ObservationRange = {
	startTimeMs: number;
	endTimeMs: number;
};

// FittingAlgorithm variant fields keep snake_case: the enum has no
// rename_all_fields.
export type FittingAlgorithm =
	| { kind: "standardization" }
	| {
			kind: "winsorization";
			lower_quantile: number;
			upper_quantile: number;
			quantile_method_version: string;
	  };

export type TransformationFittingProtocolDraft = {
	inputFeature: FeatureReference;
	fittedNodeId: string;
	fittedOutput: FeatureReference;
	snapshotId: string;
	pointInTimeUniverseId: string;
	valuationCurrency: string;
	fittingScope: "pooled-universe" | "per-instrument";
	fittingWindow: ObservationRange;
	algorithm: FittingAlgorithm;
	minimumSamples: number;
	engineIdentity: FeatureEngineIdentity;
};

export type FeatureMaterializationRequest = {
	userId: string;
	featurePlanHash: string;
	snapshotId: string;
	pointInTimeUniverseId: string;
	valuationCurrency: string;
	observationRange: ObservationRange;
	parameters: Record<string, unknown>;
	artifactIds?: string[];
	seed: number;
};

export type MaterializationAttempt = {
	attemptId: string;
	userId: string;
	requestHash: string;
	status: FeatureAttemptStatus;
	sourceAttemptId?: string | null;
	datasetId?: string | null;
	failureCode?: string | null;
	diagnostic?: string | null;
	progressCompleted: number;
	progressTotal: number;
	createdAtMs: number;
	updatedAtMs: number;
};

export type FeatureObservationValue =
	| { state: "available"; value: number; availableAtMs: number }
	| { state: "unavailable"; reason: string };

export type FeatureObservation = {
	outputName: string;
	instrumentId: string;
	observationTimeMs: number;
	value: FeatureObservationValue;
	featureReference?: FeatureReference | null;
	crossSectionalCoverage?: unknown | null;
};

export type FeaturePreviewView = {
	observations: FeatureObservation[];
	eventCount: number;
	truncated: boolean;
};

export type FeatureDatasetOutputManifest = {
	outputName: string;
	valueColumn: string;
	availableAtColumn: string;
	stateColumn: string;
	reasonColumn: string;
};

export type FeatureDatasetManifest = {
	manifestSchemaVersion: string;
	request: FeatureMaterializationRequest;
	requestHash: string;
	planJson: unknown;
	artifactIds: string[];
	engineIdentity: FeatureEngineIdentity;
	reasonVersion: string;
	outputs: FeatureDatasetOutputManifest[];
	rowCount: number;
	contentSha256: string;
};

export type FeatureDatasetView = {
	datasetId: string;
	userId: string;
	requestHash: string;
	manifest: FeatureDatasetManifest;
	contentByteSize: number;
	createdAtMs: number;
};

export type FeatureDatasetCell =
	| { state: "available"; value: number; availableAtMs: number }
	| { state: "unavailable"; reason: string };

export type FeatureDatasetRow = {
	instrumentId: string;
	observationTimeMs: number;
	values: Record<string, FeatureDatasetCell>;
};

export type FeatureDatasetRowState = "available" | "unavailable";

export type FeatureDatasetFilter = {
	instrumentId?: string;
	startTimeMs?: number;
	endTimeMs?: number;
	outputName?: string;
	state?: FeatureDatasetRowState;
	limit: number;
};

export type FeatureDatasetPage = {
	rows: FeatureDatasetRow[];
	nextOffset?: number | null;
};

export type FeatureOutputSummary = {
	outputName: string;
	rowCount: number;
	availableCount: number;
	coverage: number;
	unavailableCounts: Record<string, number>;
	minimum?: number | null;
	maximum?: number | null;
	mean?: number | null;
	populationStandardDeviation?: number | null;
};

export type MarketDataSnapshotSummary = {
	snapshotId: string;
	code: string;
	interval: string;
	barCount: number;
};

export type UniverseSnapshotSummary = {
	snapshotId: string;
	venue: {
		id: string;
		kind: string;
		timeZone: string;
	};
	interval: string;
	startTimeMs: number;
	endTimeMs: number;
};
