import { Channel } from "@tauri-apps/api/core";
import type { TauriInvoke } from "@/lib/tauri-invoke";
import type { LibraryComponent } from "@/features/components/component-library";
import type {
	BacktestListRequest,
	BacktestRun,
	ChartDataRequest,
	ExecutionDataRequest,
	ExecutionPage,
	RunHistoryPage,
	Snapshot,
	SnapshotDownloadEvent,
	SnapshotDownloadRequest,
	SnapshotListRequest,
	SnapshotPage,
	StrategyAttempt,
	StrategyProject,
} from "./backtest-types";
import type {
	BacktestPreflight,
	BacktestRunRequest,
	SignalCandidate,
} from "./backtest-run-draft";

export function createBacktestAdapter(invoke: TauriInvoke) {
	return {
		listRuns(request: BacktestListRequest) {
			return invoke("backtest_list", { request }) as Promise<RunHistoryPage>;
		},
		listSnapshots(request: SnapshotListRequest) {
			return invoke("snapshot_list", { request }) as Promise<SnapshotPage>;
		},
		listComponents(userId: string) {
			return invoke("component_list", { request: { userId } }) as Promise<
				LibraryComponent[]
			>;
		},
		listCompatibleFactors(userId: string, strategyArchiveSha256: string) {
			return invoke("backtest_compatible_factors", {
				request: { userId, strategyArchiveSha256 },
			}) as Promise<Record<string, string[]>>;
		},
		listCompatibleSignals(
			userId: string,
			strategyArchiveSha256: string,
			snapshotId: string,
		) {
			return invoke("backtest_compatible_signals", {
				request: { userId, strategyArchiveSha256, snapshotId },
			}) as Promise<SignalCandidate[]>;
		},
		preflight(request: BacktestRunRequest) {
			return invoke("backtest_preflight", {
				request,
			}) as Promise<BacktestPreflight>;
		},
		downloadSnapshot(
			request: SnapshotDownloadRequest,
			onEvent: (event: SnapshotDownloadEvent) => void,
		) {
			const channel = new Channel<SnapshotDownloadEvent>();
			channel.onmessage = onEvent;
			return invoke("snapshot_download", {
				request,
				onEvent: channel,
			}) as Promise<Snapshot>;
		},
		cancelSnapshot(taskId: string) {
			return invoke("snapshot_cancel", { request: { taskId } });
		},
		run(request: BacktestRunRequest) {
			return invoke("backtest_run", { request }) as Promise<BacktestRun>;
		},
		executionData(request: ExecutionDataRequest) {
			return invoke("backtest_execution_data", {
				request,
			}) as Promise<ExecutionPage>;
		},
		chartData(request: ChartDataRequest) {
			return invoke("backtest_chart_data", { request }) as Promise<BacktestRun>;
		},
		getRun(userId: string, runId: string) {
			return invoke("backtest_get", {
				request: { userId, runId },
			}) as Promise<BacktestRun>;
		},
		listStrategyProjects() {
			return invoke("strategy_project_list", {}) as Promise<StrategyProject[]>;
		},
		saveStrategyProject(project: StrategyProject) {
			return invoke("strategy_project_save", { request: { project } });
		},
		startStrategyAttempt(projectId: string, window: "selection" | "final") {
			return invoke("strategy_attempt_start", {
				request: { projectId, window },
			}) as Promise<StrategyAttempt>;
		},
		completeStrategyAttempt(attemptId: string, runId: string) {
			return invoke("strategy_attempt_complete", {
				request: { attemptId, runId },
			}) as Promise<StrategyAttempt>;
		},
	};
}

export type BacktestAdapter = ReturnType<typeof createBacktestAdapter>;
