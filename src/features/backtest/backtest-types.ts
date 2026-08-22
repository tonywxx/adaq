import type { OhlcvBar, BarInterval } from "@/lib/market-chart-adapter";
import type { NormalizedRunConfiguration } from "./backtest-run-draft";

export type Snapshot = {
	snapshotId: string;
	src: string;
	code: string;
	interval: BarInterval;
	barCount: number;
	startTimeMs: number;
	endTimeMs: number;
	gaps: { startTimeMs: number; endTimeMs: number }[];
};

export type SnapshotPage = {
	items: Snapshot[];
	total: number;
	page: number;
	pageSize: number;
};

export type Fill = {
	orderId: number;
	openTimeMs: number;
	side: "buy" | "sell";
	price: string;
	quantity: string;
	requestedQuantity: string;
	fee: string;
	realizedPnl: string;
	role: "maker" | "taker";
};

export type Order = {
	orderId: number;
	createdTimeMs: number;
	side: "buy" | "sell";
	quantity: string;
	limitPrice: string;
	policy: "maker" | "taker";
	status: { status: string; reason?: string } | string;
};

export type EquityPoint = {
	openTimeMs: number;
	equity: string;
	drawdown: string;
};

export type Metrics = Record<
	| "initialEquity"
	| "finalEquity"
	| "totalReturn"
	| "cagr"
	| "annualizedVolatility"
	| "sharpe"
	| "sortino"
	| "maxDrawdown"
	| "calmar"
	| "realizedPnl"
	| "unrealizedPnl"
	| "totalFees"
	| "turnover"
	| "winRate"
	| "profitFactor"
	| "averageWin"
	| "averageLoss"
	| "exposureTime"
	| "benchmarkReturn"
	| "excessReturn",
	string
> & { fillCount: number; realizedTradeCount: number };

export type Provenance = {
	normalizedRequest: NormalizedRunConfiguration;
	featurePlanJson: string;
	featurePlanHash: string;
	componentLock: Array<{
		componentId: string;
		version: string;
		archiveSha256: string;
		wasmSha256: string;
	}>;
	datasetLock: Array<{
		slot: string;
		datasetId: string;
		signalName: string;
		evidenceState: string;
	}>;
	architecture: "signal-driven" | "composed" | "hybrid";
	indicatorEngineBuildIdentity: Record<string, string>;
	backtestEngineVersion: string;
	seed: number;
};

export type BacktestRun = {
	runId: string;
	snapshot: Snapshot;
	provenance?: Provenance;
	bars: OhlcvBar[];
	decisions: Array<{ openTimeMs: number; targetExposure: string }>;
	pauses: Array<{ openTimeMs: number; reason: string }>;
	result: {
		orders: Order[];
		fills: Fill[];
		equity: EquityPoint[];
		benchmarkEquity: EquityPoint[];
		metrics: Metrics;
		totalFees: string;
		finalCash: string;
		finalBaseQuantity: string;
	};
};

export type RunSummary = {
	runId: string;
	createdAt: string;
	code: string;
	interval: string;
	barCount: number;
	totalReturn: string;
};

export type RunHistoryPage = {
	items: RunSummary[];
	total: number;
	page: number;
	pageSize: number;
};

export type ExecutionPage = {
	orders: Order[];
	fills: Fill[];
	totalOrders: number;
	totalFills: number;
};

export type BacktestListRequest = {
	userId: string;
	src: string;
	code: string;
	page: number;
};

export type SnapshotListRequest = {
	userId: string;
	src: string;
	code: string;
	interval: BarInterval;
	page: number;
};

export type SnapshotDownloadRequest = {
	taskId: string;
	userId: string;
	src: string;
	code: string;
	interval: BarInterval;
	startTimeMs: number;
	endTimeMs: number;
};

export type SnapshotDownloadEvent = {
	event: "progress" | "completed" | "cancelled";
	data?: { downloadedBars?: number };
};

export type ExecutionDataRequest = {
	userId: string;
	runId: string;
	offset: number;
	limit: number;
};

export type ChartDataRequest = {
	userId: string;
	runId: string;
	startTimeMs: number;
	endTimeMs: number;
	maxPoints: number;
};

export type StrategyScope = "single-instrument" | "portfolio";
export type EvaluationWindow = "selection" | "final";
export type StrategyProject = {
	strategyId: string;
	userId: string;
	revision: number;
	strategyArchiveSha256: string;
	scope: StrategyScope;
	contextHash: string;
	contextStartTimeMs: number;
	contextEndTimeMs: number;
	selectionWindow: { startTimeMs: number; endTimeMs: number };
	finalWindow: { startTimeMs: number; endTimeMs: number };
	bindings: Array<{ slot: string; evidenceId: string; lineageHash: string }>;
	parameters: Record<string, string>;
};

export type StrategyAttempt = {
	attemptId: string;
	projectId: string;
	projectRevision: number;
	contextHash: string;
	window: EvaluationWindow;
	status: "pending" | "running" | "completed" | "failed" | "cancelled";
	failure?: string;
	evidence?: {
		attemptId: string;
		projectRevision: number;
		contextHash: string;
		window: EvaluationWindow;
		runIds: string[];
		provenance: Record<string, string>;
	};
};

export type UniverseSnapshot = {
	snapshotId: string;
	interval: BarInterval;
	startTimeMs: number;
	endTimeMs: number;
	universe: {
		universeId: string;
		evidenceState: string;
		instruments: Array<{ code: string }>;
	};
};

export type PortfolioBacktest = {
	runId: string;
	reusedExistingRun: boolean;
	evidence: { finalEquity: string; totalCosts: string; turnover: string };
};
