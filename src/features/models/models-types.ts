export type EvaluationSignalContract = {
	name: string;
	predictionKind: { kind: string };
	forecastTarget: { kind: string; target?: string; valueType?: string };
	valueScale: { kind: string; minimum?: number; maximum?: number };
	horizonBars: number;
};

export type Snapshot = {
	snapshotId: string;
	code: string;
	interval: string;
	barCount: number;
};

export type Dataset = {
	datasetId: string;
	snapshotId: string;
	code: string;
	interval: string;
	predictionSource: string;
	rowCount: number;
	unavailableCount: number;
	statusCounts: Record<string, number>;
	modelArtifact?: { sha256: string; provenance: Record<string, string> };
	modelOutputs: Array<Record<string, unknown>>;
	modelParameters: Record<string, Record<string, unknown>>;
	sourceWarmupBars: number;
	modelWarmupBars: number;
	modelArchiveSha256: string;
	trust: string;
	componentLock: Array<{ alias: string; archiveSha256: string }>;
	featurePlanHash: string;
	featurePlanJson: string;
	seed: number;
	engineIdentity: Record<string, string>;
	producerSegments: Array<Record<string, unknown>>;
	continuousBarSegments: number;
	barGapRule: string;
	parquetSha256: string;
	archiveManifestJson?: string;
	externalProducerSegments?: Array<Record<string, unknown>>;
};

export type Attempt = {
	attemptId: string;
	datasetId?: string;
	status: "pending" | "running" | "completed" | "failed" | "cancelled";
	diagnosticEvidence?: string;
	progressCompleted: number;
	progressTotal: number;
};

export type RowPage = {
	items: Array<{
		predictionTimeMs: number;
		availableAtMs: number;
		status: string;
		values?: number[];
		unavailableReason?: string;
	}>;
	total: number;
	page: number;
	pageSize: number;
};

export type EvaluationReport = {
	reportId: string;
	datasetId: string;
	snapshotId: string;
	signalName: string;
	signalContract: EvaluationSignalContract;
	evaluationStartTimeMs: number;
	evaluationEndTimeMs: number;
	stabilityWindowBars: number;
	metrics: {
		evaluationRowCount: number;
		alignedCount: number;
		unavailablePredictionCount: number;
		unavailableLabelCount: number;
		coverage: number;
		missingness: number;
		predictionDistribution?: Record<string, number>;
		realizedDistribution?: Record<string, number>;
		mae?: number;
		rmse?: number;
		meanBias?: number;
		pearsonCorrelation?: number;
		brierScore?: number;
		logLoss?: number;
		rocAuc?: number;
		calibration?: Array<Record<string, unknown>>;
		pearsonIc?: number;
		spearmanRankIc?: number;
		windowIcir?: number;
		quantiles?: Array<Record<string, unknown>>;
		undefinedMetrics?: Record<string, string>;
	};
	stabilityWindows: Array<Record<string, unknown>>;
	evidenceState: { summary: string; segmentStates: string[] };
	unavailableRows: Array<Record<string, unknown>>;
	producerSegments: Array<Record<string, unknown>>;
	scaleProvenance?: Array<Record<string, unknown>>;
	trustState: string;
	metricVersions: Record<string, string>;
	engineIdentity: Record<string, string>;
	schemaIdentity: string;
	datasetParquetSha256: string;
	componentLock: Array<{ alias: string; archiveSha256: string }>;
	featurePlanHash: string;
};

export type DatasetGenerationRequest = {
	userId: string;
	snapshotId: string;
	modelArchiveSha256: string;
	modelParameters: Record<string, string>;
	factorInstances: Array<{ alias: string; archiveSha256: string }>;
	seed: number;
};

export type SignalDatasetRowsRequest = {
	datasetId: string;
	userId: string;
	page: number;
};

export type ForecastEvaluationRequest = {
	userId: string;
	datasetId: string;
	snapshotId: string;
	signalName: string;
	horizonBars: number;
	evaluationStartTimeMs: number;
	evaluationEndTimeMs: number;
	stabilityWindowBars: number;
};

export type EvaluationExportFormat = "json" | "markdown";

export type EvaluationExportRequest = {
	reportId: string;
	userId: string;
	format: EvaluationExportFormat;
};
