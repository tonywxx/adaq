jest.mock("@tauri-apps/api/core", () => ({
	Channel: class {
		onmessage?: (event: unknown) => void;
	},
}));

import { createBacktestAdapter } from "./backtest-adapter";

test("Backtest adapter preserves request ownership and progress channels", async () => {
	const calls: Array<{ command: string; args?: Record<string, unknown> }> = [];
	const adapter = createBacktestAdapter(async (command, args = {}) => {
		calls.push({ command, args });
		return undefined;
	});
	const runRequest = {
		userId: "user-1",
		snapshotId: "snapshot-1",
		strategyArchiveSha256: "strategy-hash",
		strategyParameters: {},
		factorInstances: [],
		signalInstances: [],
		initialQuoteAllocation: "10000",
		executionProfile: {
			makerFeeRate: "0.0008",
			takerFeeRate: "0.001",
			adverseSlippageRate: "0.0005",
			rebalanceThreshold: "0",
			priceIncrement: "0.1",
			quantityIncrement: "0.00000001",
			minimumQuantity: "0.00001",
			riskFreeRate: "0",
			fillPolicy: "taker" as const,
		},
		seed: 0,
	};

	await adapter.listRuns({
		userId: "user-1",
		src: "okx",
		code: "BTC-USDT",
		page: 2,
	});
	await adapter.listSnapshots({
		userId: "user-1",
		src: "okx",
		code: "BTC-USDT",
		interval: "1h",
		page: 1,
	});
	await adapter.listComponents("user-1");
	await adapter.listCompatibleFactors("user-1", "strategy-hash");
	await adapter.listCompatibleSignals("user-1", "strategy-hash", "snapshot-1");
	await adapter.preflight(runRequest);
	let progress: unknown;
	await adapter.downloadSnapshot(
		{
			taskId: "task-1",
			userId: "user-1",
			src: "okx",
			code: "BTC-USDT",
			interval: "1h",
			startTimeMs: 10,
			endTimeMs: 20,
		},
		(event) => {
			progress = event;
		},
	);
	const channel = calls[6].args?.onEvent as {
		onmessage?: (event: unknown) => void;
	};
	channel.onmessage?.({ event: "progress", data: { downloadedBars: 3 } });
	await adapter.cancelSnapshot("task-1");
	await adapter.run(runRequest);
	await adapter.executionData({
		userId: "user-1",
		runId: "run-1",
		offset: 100,
		limit: 100,
	});
	await adapter.chartData({
		userId: "user-1",
		runId: "run-1",
		startTimeMs: 10,
		endTimeMs: 20,
		maxPoints: 5000,
	});
	await adapter.getRun("user-1", "run-1");

	expect(progress).toEqual({ event: "progress", data: { downloadedBars: 3 } });
	expect(calls.slice(0, 6)).toEqual([
		{
			command: "backtest_list",
			args: {
				request: { userId: "user-1", src: "okx", code: "BTC-USDT", page: 2 },
			},
		},
		{
			command: "snapshot_list",
			args: {
				request: {
					userId: "user-1",
					src: "okx",
					code: "BTC-USDT",
					interval: "1h",
					page: 1,
				},
			},
		},
		{ command: "component_list", args: { request: { userId: "user-1" } } },
		{
			command: "backtest_compatible_factors",
			args: {
				request: { userId: "user-1", strategyArchiveSha256: "strategy-hash" },
			},
		},
		{
			command: "backtest_compatible_signals",
			args: {
				request: {
					userId: "user-1",
					strategyArchiveSha256: "strategy-hash",
					snapshotId: "snapshot-1",
				},
			},
		},
		{ command: "backtest_preflight", args: { request: runRequest } },
	]);
	expect(calls[6].command).toBe("snapshot_download");
	expect(calls[6].args).toMatchObject({
		request: {
			taskId: "task-1",
			userId: "user-1",
			src: "okx",
			code: "BTC-USDT",
			interval: "1h",
			startTimeMs: 10,
			endTimeMs: 20,
		},
	});
	expect(calls.slice(7)).toEqual([
		{ command: "snapshot_cancel", args: { request: { taskId: "task-1" } } },
		{ command: "backtest_run", args: { request: runRequest } },
		{
			command: "backtest_execution_data",
			args: {
				request: { userId: "user-1", runId: "run-1", offset: 100, limit: 100 },
			},
		},
		{
			command: "backtest_chart_data",
			args: {
				request: {
					userId: "user-1",
					runId: "run-1",
					startTimeMs: 10,
					endTimeMs: 20,
					maxPoints: 5000,
				},
			},
		},
		{
			command: "backtest_get",
			args: { request: { userId: "user-1", runId: "run-1" } },
		},
	]);
});
