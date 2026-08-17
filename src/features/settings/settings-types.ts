export type LocalDataSummary = {
	dataDirectory: string;
	databaseBytes: number;
	componentBytes: number;
	marketDataBytes: number;
	watchlistCount: number;
	componentCount: number;
	snapshotCount: number;
	runCount: number;
	protocolCount: number;
	reportCount: number;
	generationAttemptCount: number;
	modelArtifactCount: number;
	signalDatasetCount: number;
	componentBlockingRunCount: number;
	marketDataBlockingRecordCount: number;
};
