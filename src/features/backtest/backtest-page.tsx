import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
	Workspace,
	type LibraryComponent,
} from "@/features/components/components-page";
import {
	BAR_INTERVALS,
	type BarInterval,
	type OhlcvBar,
} from "@/lib/market-chart-adapter";
import { instrumentKey, useMarketSessionStore } from "@/lib/market-session";
import { Channel, invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { BacktestChart } from "./backtest-chart";
import {
	snapshotError,
	snapshotRangeError,
	snapshotStatus,
	reuseSnapshot,
} from "./backtest-data";
import {
	defaultExecutionProfile,
	matchingFactors,
	runGate,
} from "./guided-backtest";
import { formatDecimal } from "./format-decimal";

type Snapshot = {
	snapshotId: string;
	src: string;
	code: string;
	interval: BarInterval;
	barCount: number;
	startTimeMs: number;
	endTimeMs: number;
	gaps: { startTimeMs: number; endTimeMs: number }[];
};
type Fill = {
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
type Order = {
	orderId: number;
	createdTimeMs: number;
	side: "buy" | "sell";
	quantity: string;
	limitPrice: string;
	policy: "maker" | "taker";
	status: { status: string; reason?: string } | string;
};
type EquityPoint = { openTimeMs: number; equity: string; drawdown: string };
type Metrics = Record<
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
type Provenance = {
	[key: string]: unknown;
};
export type BacktestRun = {
	runId: string;
	provenance?: Provenance;
	bars: OhlcvBar[];
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
type RunSummary = {
	runId: string;
	createdAt: string;
	code: string;
	interval: string;
	barCount: number;
	totalReturn: string;
};
type WalkForward = {
	snapshotId: string;
	windowSizeBars: number;
	stepSizeBars: number;
	minimumHistoryBars: number;
};
type ValidationProtocol = {
	protocolId: string;
	methodVersion: string;
	windows: {
		snapshotId: string;
		sampleOutStartTimeMs: number;
		sampleOutEndTimeMs?: number;
	}[];
	walkForward?: WalkForward;
	crossMarket?: { contexts: { snapshotId: string; runOverride?: unknown }[] };
};
type ValidationWindow = {
	sampleOutStartTimeMs: number;
	sampleOutEndTimeMs?: number;
	sampleOutMetrics?: {
		totalReturn: string;
		maxDrawdown: string;
		sharpe: string;
	};
	sampleOutPauses: unknown[];
	failure?: string;
};
type ValidationReport = {
	reportId: string;
	protocolId: string;
	methodVersion: string;
	walkForward?: WalkForward;
	crossMarket: {
		snapshot: Snapshot;
		runId?: string;
		metrics?: { totalReturn: string; maxDrawdown: string; sharpe: string };
		pauses: unknown[];
		failure?: string;
	}[];
	crossMarketEvidence?: { completedMarkets: number; totalReturnSpread: string };
	recommendedContexts: unknown[];
	windows: ValidationWindow[];
	aggregate: {
		completedWindows: number;
		failedWindows: number;
		averageSampleOutReturn: string;
		worstSampleOutDrawdown: string;
		averageSampleOutSharpe: string;
		totalFees: string;
		totalTrades: number;
	};
};
type ExecutionPage = {
	orders: Order[];
	fills: Fill[];
	totalOrders: number;
	totalFills: number;
};
type BacktestPreflight = {
	runId: string;
	reusesExistingRun: boolean;
	snapshot: Snapshot;
	normalizedRequest: Record<string, unknown>;
	indicatorPlan: Record<string, unknown>;
	componentLock: Array<Record<string, unknown>>;
};
const EXECUTION_PAGE_SIZE = 100;

export function BacktestPage() {
	const userId = useMarketSessionStore((state) => state.userId);
	const instrument = useMarketSessionStore((state) => state.activeInstrument);
	const watchlist = useMarketSessionStore((state) => state.watchlist);
	const [components, setComponents] = useState<LibraryComponent[]>([]);
	const [factorSelections, setFactorSelections] = useState<
		Record<string, string>
	>({});
	const [strategy, setStrategy] = useState("");
	const [factorParameters, setFactorParameters] = useState<
		Record<string, Record<string, string>>
	>({});
	const [strategyParameters, setStrategyParameters] = useState<
		Record<string, string>
	>({});
	const [stage, setStage] = useState<"data" | "strategy" | "execution" | "results">(
		"data",
	);
	const [initialQuoteAllocation, setInitialQuoteAllocation] = useState("10000");
	const [executionProfile, setExecutionProfile] = useState(defaultExecutionProfile);
	const [running, setRunning] = useState(false);
	const [compatibleFactors, setCompatibleFactors] = useState<Record<string, string[]>>({});
	const [preflight, setPreflight] = useState<BacktestPreflight>();
	const [start, setStart] = useState(() =>
		new Date(Date.now() - 30 * 864e5).toISOString().slice(0, 10),
	);
	const [end, setEnd] = useState(() => new Date().toISOString().slice(0, 10));
	const [selectedInstrumentKey, setSelectedInstrumentKey] = useState(() =>
		instrumentKey(instrument),
	);
	const [interval, setInterval] = useState<BarInterval>("1h");
	const [snapshot, setSnapshot] = useState<Snapshot>();
	const [snapshots, setSnapshots] = useState<Snapshot[]>([]);
	const [run, setRun] = useState<BacktestRun>();
	const [executionPage, setExecutionPage] = useState<ExecutionPage>();
	const [executionOffset, setExecutionOffset] = useState(0);
	const [history, setHistory] = useState<RunSummary[]>([]);
	const [message, setMessage] = useState("");
	const [runTechnicalError, setRunTechnicalError] = useState("");
	const [snapshotTechnicalError, setSnapshotTechnicalError] = useState("");
	const [downloadTaskId, setDownloadTaskId] = useState<string>();
	const [sampleOutStart, setSampleOutStart] = useState("");
	const [walkForwardWindowSize, setWalkForwardWindowSize] = useState("30");
	const [walkForwardStepSize, setWalkForwardStepSize] = useState("30");
	const [walkForwardMinimumHistory, setWalkForwardMinimumHistory] =
		useState("90");
	const [crossMarketSnapshotIds, setCrossMarketSnapshotIds] = useState("");
	const [protocols, setProtocols] = useState<ValidationProtocol[]>([]);
	const [reports, setReports] = useState<ValidationReport[]>([]);
	const [runningProtocolId, setRunningProtocolId] = useState<string>();
	const chartRequest = useRef(0);
	const chartRange = useRef("");
	const instruments = useMemo(
		() => [
			...new Map(
				[...watchlist, instrument].map((item) => [instrumentKey(item), item]),
			).values(),
		],
		[instrument, watchlist],
	);
	const selectedInstrument =
		instruments.find((item) => instrumentKey(item) === selectedInstrumentKey) ??
		instrument;
	const refreshHistory = async () => {
		if (userId)
			try {
				setHistory(await invoke("backtest_list", { request: { userId } }));
			} catch (error) {
				setMessage(String(error));
			}
	};
	const refreshValidation = async () => {
		if (!userId) return;
		setProtocols(
			await invoke("validation_protocol_list", { request: { userId } }),
		);
		setReports(await invoke("validation_report_list", { request: { userId } }));
	};
	useEffect(() => {
		if (userId) {
			void invoke<LibraryComponent[]>("component_list", { request: { userId } })
				.then((items) => {
					setComponents(items);
					if (items.some((item) => item.compatibilityError))
						setMessage(
							"Incompatible Components are hidden. Remove them from Component Library and import Manifest 1.0 packages.",
						);
				})
				.catch((error) => setMessage(String(error)));
			void invoke<RunSummary[]>("backtest_list", { request: { userId } })
				.then(setHistory)
				.catch((error) => setMessage(String(error)));
			void refreshValidation().catch((error) => setMessage(String(error)));
		}
	}, [userId]);
	useEffect(() => {
		if (!userId) return;
		void invoke<Snapshot[]>("snapshot_list", {
			request: { userId, ...selectedInstrument, interval },
		})
			.then(setSnapshots)
			.catch((error) => setSnapshotTechnicalError(snapshotError(error)));
	}, [interval, selectedInstrument, userId]);
	const factors = useMemo(
		() =>
			components.filter(
				(item) => item.kind === "factor" && !item.compatibilityError,
			),
		[components],
	);
	const strategies = useMemo(
		() =>
			components.filter(
				(item) => item.kind === "strategy" && !item.compatibilityError,
			),
		[components],
	);
	const selectedStrategy = strategies.find(
		(item) => item.archiveSha256 === strategy,
	);
	useEffect(() => {
		setCompatibleFactors({});
		setPreflight(undefined);
		if (!userId || !strategy) return;
		void invoke<Record<string, string[]>>("backtest_compatible_factors", {
			request: { userId, strategyArchiveSha256: strategy },
		})
			.then(setCompatibleFactors)
			.catch((error) => setMessage(String(error)));
	}, [strategy, userId]);
	const selectStage = async (next: "data" | "strategy" | "execution" | "results") => {
		if (next === "strategy" && !snapshot) {
			setMessage("Select a Market Data Snapshot before continuing.");
			return;
		}
		if (next === "execution") {
			const gate = runGate({
				snapshotId: snapshot?.snapshotId,
				strategy: selectedStrategy,
				dependencies: selectedStrategy?.dependencies ?? [],
				factorSelections,
				running,
			});
			if (gate) {
				setMessage(gate);
				return;
			}
			if (!snapshot) return;
			setStage("execution");
			setRunning(true);
			setRunTechnicalError("");
			try {
				setPreflight(
					await invoke<BacktestPreflight>("backtest_preflight", {
						request: buildRunRequest(snapshot.snapshotId),
					}),
				);
				setMessage("Authoritative inputs validated. Review before running.");
			} catch (error) {
				const details = String(error);
				setRunTechnicalError(details);
				setMessage(details);
			} finally {
				setRunning(false);
			}
			return;
		}
		if (next === "results" && !run) {
			setMessage("Run a Backtest before viewing Results.");
			return;
		}
		if (next === "data" || next === "strategy") setPreflight(undefined);
		setStage(next);
	};
	const prepare = async () => {
		if (!userId) return;
		const rangeError = snapshotRangeError(start, end);
		if (rangeError) {
			setSnapshotTechnicalError(rangeError);
			return;
		}
		const taskId = crypto.randomUUID();
		const onEvent = new Channel<{
			event: "progress" | "completed" | "cancelled";
			data?: { downloadedBars?: number };
		}>();
		onEvent.onmessage = (event) => {
			if (event.event === "progress")
				setMessage(snapshotStatus(event.event, event.data?.downloadedBars));
			if (event.event === "cancelled") setMessage(snapshotStatus(event.event));
		};
		setDownloadTaskId(taskId);
		setSnapshotTechnicalError("");
		setMessage("Downloading and freezing Closed Bars…");
		try {
			const value = await invoke<Snapshot>("snapshot_download", {
				request: {
					taskId,
					userId,
					...selectedInstrument,
					interval,
					startTimeMs: Date.parse(start),
					endTimeMs: Date.parse(end),
				},
				onEvent,
			});
			setSnapshot(value);
			setSnapshots((current) =>
				[
					...current.filter((item) => item.snapshotId !== value.snapshotId),
					value,
				].sort((left, right) => left.startTimeMs - right.startTimeMs),
			);
			setMessage(`${value.barCount} Bars frozen.`);
		} catch (error) {
			const technicalError = snapshotError(error);
			setSnapshotTechnicalError(technicalError);
			if (technicalError.includes("cancelled"))
				setMessage(snapshotStatus("cancelled"));
		} finally {
			setDownloadTaskId(undefined);
		}
	};
	const buildRunRequest = (snapshotId: string) => ({
		userId: userId!,
		snapshotId,
		factorInstances:
			selectedStrategy?.dependencies
				.map((dependency) => ({
					alias: dependency.alias,
					archiveSha256: factorSelections[dependency.alias],
					parameters: factorParameters[dependency.alias] ?? {},
				}))
				.filter((factor) => factor.archiveSha256) ?? [],
		strategyArchiveSha256: strategy,
		strategyParameters,
		initialQuoteAllocation,
		executionProfile,
	});
	const createValidation = async () => {
		if (!userId || !snapshot) return;
		try {
			const protocol = await invoke<ValidationProtocol>(
				"validation_protocol_create",
				{
					request: {
						userId,
						run: buildRunRequest(snapshot.snapshotId),
						windows: [
							{
								snapshotId: snapshot.snapshotId,
								sampleOutStartTimeMs: Date.parse(sampleOutStart),
							},
						],
						methodVersion: "chronological-holdout@1",
						aggregationRuleVersion: "equal-window@1",
					},
				},
			);
			setMessage(`Protocol ${protocol.protocolId.slice(0, 12)} frozen.`);
			await refreshValidation();
		} catch (error) {
			setMessage(String(error));
		}
	};
	const createWalkForwardValidation = async () => {
		if (!userId || !snapshot) return;
		try {
			const protocol = await invoke<ValidationProtocol>(
				"validation_protocol_create",
				{
					request: {
						userId,
						run: buildRunRequest(snapshot.snapshotId),
						windows: [],
						methodVersion: "walk-forward@1",
						aggregationRuleVersion: "equal-window@1",
						walkForward: {
							snapshotId: snapshot.snapshotId,
							windowSizeBars: Number(walkForwardWindowSize),
							stepSizeBars: Number(walkForwardStepSize),
							minimumHistoryBars: Number(walkForwardMinimumHistory),
						},
					},
				},
			);
			setMessage(
				`Walk-forward Protocol ${protocol.protocolId.slice(0, 12)} frozen.`,
			);
			await refreshValidation();
		} catch (error) {
			setMessage(String(error));
		}
	};
	const createCrossMarketValidation = async () => {
		if (!userId || !snapshot) return;
		const snapshotIds = [
			snapshot.snapshotId,
			...crossMarketSnapshotIds.split(/\s+/),
		].filter(Boolean);
		try {
			const protocol = await invoke<ValidationProtocol>(
				"validation_protocol_create",
				{
					request: {
						userId,
						run: buildRunRequest(snapshot.snapshotId),
						windows: [],
						methodVersion: "cross-market@1",
						aggregationRuleVersion: "equal-window@1",
						crossMarket: {
							contexts: snapshotIds.map((snapshotId) => ({ snapshotId })),
						},
					},
				},
			);
			setMessage(
				`Cross-market Protocol ${protocol.protocolId.slice(0, 12)} frozen.`,
			);
			await refreshValidation();
		} catch (error) {
			setMessage(String(error));
		}
	};
	const runValidation = async (protocolId: string) => {
		if (!userId) return;
		setRunningProtocolId(protocolId);
		setMessage("Running validation…");
		try {
			const report = await invoke<ValidationReport>("validation_report_run", {
				request: { userId, protocolId },
			});
			setMessage(`Validation Report ${report.reportId.slice(0, 12)} completed.`);
			await refreshValidation();
		} catch (error) {
			setMessage(String(error));
		} finally {
			setRunningProtocolId(undefined);
		}
	};
	const exportReport = async (reportId: string, format: "json" | "markdown") => {
		if (!userId) return;
		try {
			const content = await invoke<string>("validation_report_export", {
				request: { userId, protocolId: reportId },
				format,
			});
			const anchor = document.createElement("a");
			anchor.href = URL.createObjectURL(
				new Blob([content], {
					type: format === "json" ? "application/json" : "text/markdown",
				}),
			);
			anchor.download = `validation-report-${reportId}.${format === "json" ? "json" : "md"}`;
			anchor.click();
			URL.revokeObjectURL(anchor.href);
		} catch (error) {
			setMessage(String(error));
		}
	};
	const execute = async () => {
		if (!preflight) {
			await selectStage("execution");
			return;
		}
		const gate = runGate({
			snapshotId: snapshot?.snapshotId,
			strategy: selectedStrategy,
			dependencies: selectedStrategy?.dependencies ?? [],
			factorSelections,
			running,
		});
		if (gate) {
			setMessage(gate);
			return;
		}
		if (!userId || !snapshot || running) return;
		setRunning(true);
		setRunTechnicalError("");
		setMessage("Running deterministic Backtest…");
		try {
			const value = await invoke<BacktestRun>("backtest_run", {
				request: buildRunRequest(snapshot.snapshotId),
			});
			setRun(value);
			setExecutionOffset(0);
			setStage("results");
			setMessage(`Run ${value.runId.slice(0, 12)} completed.`);
			await refreshHistory();
		} catch (error) {
			const details = String(error);
			setMessage(details);
			setRunTechnicalError(details);
		} finally {
			setRunning(false);
		}
	};
	const runId = run?.runId;
	useEffect(() => {
		if (!userId || !runId) return;
		void invoke<ExecutionPage>("backtest_execution_data", {
			request: {
				userId,
				runId,
				offset: executionOffset,
				limit: EXECUTION_PAGE_SIZE,
			},
		})
			.then(setExecutionPage)
			.catch((error) => setMessage(String(error)));
	}, [executionOffset, runId, userId]);
	const loadChartRange = useCallback(
		async (startTimeMs: number, endTimeMs: number) => {
			if (!userId || !runId) return;
			const key = `${runId}:${startTimeMs}:${endTimeMs}`;
			if (chartRange.current === key) return;
			chartRange.current = key;
			const requestId = ++chartRequest.current;
			try {
				const view = await invoke<BacktestRun>("backtest_chart_data", {
					request: { userId, runId, startTimeMs, endTimeMs, maxPoints: 5000 },
				});
				if (requestId === chartRequest.current)
					setRun((current) =>
						current ? mergeRange(current, view, startTimeMs, endTimeMs) : view,
					);
			} catch (error) {
				setMessage(String(error));
			}
		},
		[runId, userId],
	);
	const setParameter = (alias: string, name: string, value: string) =>
		setFactorParameters((current) => ({
			...current,
			[alias]: { ...current[alias], [name]: value },
		}));
	return (
		<Workspace
			title="Backtest"
			description={`${selectedInstrument.code} · OKX Spot · Long Only`}
		>
			<nav aria-label="Backtest stages" className="mb-4 grid gap-2 sm:grid-cols-4">
				{(["data", "strategy", "execution", "results"] as const).map((item, index) => (
					<Button
						key={item}
						type="button"
						variant={stage === item ? "default" : "outline"}
						aria-current={stage === item ? "step" : undefined}
						onClick={() => void selectStage(item)}
					>
						{index + 1}. {item[0].toUpperCase() + item.slice(1)}
					</Button>
				))}
			</nav>
			<Card>
				<CardHeader>
					<CardTitle>Data and Strategy configuration</CardTitle>
				</CardHeader>
				<CardContent className="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
					{stage === "data" && (
						<>
					<Field label="Instrument" id="backtest-instrument">
						<select
							id="backtest-instrument"
							className="h-9 rounded-md border bg-background px-3"
							value={selectedInstrumentKey}
							onChange={(event) => {
								setSelectedInstrumentKey(event.target.value);
								setSnapshot(undefined);
							}}
						>
							{instruments.map((item) => (
								<option key={instrumentKey(item)} value={instrumentKey(item)}>
									{item.code} · {item.src.toUpperCase()}
								</option>
							))}
						</select>
					</Field>
					<Field label="Bar Interval" id="backtest-interval">
						<select
							id="backtest-interval"
							className="h-9 rounded-md border bg-background px-3"
							value={interval}
							onChange={(event) => {
								setInterval(event.target.value as BarInterval);
								setSnapshot(undefined);
							}}
						>
							{BAR_INTERVALS.map((value) => (
								<option key={value} value={value}>
									{value}
								</option>
							))}
						</select>
					</Field>
					<Field label="Start" id="backtest-start">
						<Input
							id="backtest-start"
							type="date"
							value={start}
							onChange={(e) => setStart(e.target.value)}
						/>
					</Field>
					<Field label="End" id="backtest-end">
						<Input
							id="backtest-end"
							type="date"
							value={end}
							onChange={(e) => setEnd(e.target.value)}
						/>
					</Field>
					<Field label="Sample-out starts">
						<Input
							type="date"
							value={sampleOutStart}
							onChange={(e) => setSampleOutStart(e.target.value)}
						/>
					</Field>
					<Field label="Walk-forward window (Bars)">
						<Input
							type="number"
							min="1"
							value={walkForwardWindowSize}
							onChange={(e) => setWalkForwardWindowSize(e.target.value)}
						/>
					</Field>
					<Field label="Walk-forward step (Bars)">
						<Input
							type="number"
							min="1"
							value={walkForwardStepSize}
							onChange={(e) => setWalkForwardStepSize(e.target.value)}
						/>
					</Field>
					<Field label="Walk-forward minimum history (Bars)">
						<Input
							type="number"
							min="1"
							value={walkForwardMinimumHistory}
							onChange={(e) => setWalkForwardMinimumHistory(e.target.value)}
						/>
					</Field>
					<Field label="Cross-market Snapshot IDs (one per line)">
						<textarea
							className="min-h-9 rounded-md border bg-background px-3 py-2"
							value={crossMarketSnapshotIds}
							onChange={(e) => setCrossMarketSnapshotIds(e.target.value)}
							placeholder="Freeze other market Snapshots first"
						/>
					</Field>
						</>
					)}
					{stage === "strategy" && (
						<>
					<Field label="Strategy">
						<select
							className="h-9 rounded-md border bg-background px-3"
							value={strategy}
							onChange={(e) => {
								setStrategy(e.target.value);
								setFactorSelections({});
								setFactorParameters({});
								setStrategyParameters({});
							}}
						>
							<option value="">Select</option>
							{strategies.map((item) => (
								<option key={item.archiveSha256} value={item.archiveSha256}>
									{item.name} v{item.version}
								</option>
							))}
						</select>
					</Field>
					{selectedStrategy?.dependencies.map((dependency) => (
						<Field key={dependency.alias} label={`Factor · ${dependency.alias}`}>
							<select
								className="h-9 rounded-md border bg-background px-3"
								value={factorSelections[dependency.alias] ?? ""}
								onChange={(event) =>
									setFactorSelections((current) => ({
										...current,
										[dependency.alias]: event.target.value,
									}))
								}
							>
								<option value="">Select {dependency.version}</option>
								{matchingFactors(
									dependency,
									factors,
									compatibleFactors[dependency.alias] ?? [],
								)
									.map((item) => (
										<option key={item.archiveSha256} value={item.archiveSha256}>
											{item.name} v{item.version}
										</option>
									))}
							</select>
						</Field>
					))}
					{selectedStrategy?.dependencies.flatMap((dependency) => {
						const component = factors.find(
							(item) => item.archiveSha256 === factorSelections[dependency.alias],
						);
						return (
							component?.parameters.map((parameter) => (
								<ParameterField
									key={`${dependency.alias}:${parameter.name}`}
									label={`${dependency.alias} · ${parameter.name}`}
									parameter={parameter}
									value={
										factorParameters[dependency.alias]?.[parameter.name] ??
										parameter.defaultValue
									}
									onChange={(value) =>
										setParameter(dependency.alias, parameter.name, value)
									}
								/>
							)) ?? []
						);
					})}
					{selectedStrategy?.parameters.map((parameter) => (
						<ParameterField
							key={parameter.name}
							label={`${selectedStrategy.name} · ${parameter.name}`}
							parameter={parameter}
							value={strategyParameters[parameter.name] ?? parameter.defaultValue}
							onChange={(value) =>
								setStrategyParameters((current) => ({
									...current,
									[parameter.name]: value,
								}))
							}
						/>
					))}
						</>
					)}
					{stage === "data" && (
						<>
					<div className="flex flex-wrap items-end gap-2">
						<Button disabled={Boolean(downloadTaskId)} onClick={() => void prepare()}>
							Prepare Snapshot
						</Button>
						{downloadTaskId && (
							<Button
								variant="outline"
								onClick={() =>
									void invoke("snapshot_cancel", { request: { taskId: downloadTaskId } })
								}
							>
								Cancel
							</Button>
						)}
						<Button disabled={!snapshot || !strategy} onClick={() => void selectStage("execution")}>
							Review execution
						</Button>
						<Button
							variant="outline"
							disabled={!snapshot || !strategy || !sampleOutStart}
							onClick={() => void createValidation()}
						>
							Freeze holdout
						</Button>
						<Button
							variant="outline"
							disabled={
								!snapshot ||
								!strategy ||
								!walkForwardWindowSize ||
								!walkForwardStepSize ||
								!walkForwardMinimumHistory
							}
							onClick={() => void createWalkForwardValidation()}
						>
							Freeze walk-forward
						</Button>
						<Button
							variant="outline"
							disabled={!snapshot || !strategy || !crossMarketSnapshotIds.trim()}
							onClick={() => void createCrossMarketValidation()}
						>
							Freeze cross-market
						</Button>
					</div>
					<p
						className="self-end text-sm text-muted-foreground"
						role="status"
						aria-live="polite"
					>
						{message}
					</p>
					<div className="md:col-span-2 lg:col-span-4">
						<p className="mb-2 text-sm font-medium">Reuse immutable evidence</p>
						{snapshots.length === 0 ? (
							<p className="text-sm text-muted-foreground">
								No matching Snapshots. Download and freeze new evidence.
							</p>
						) : (
							<div className="grid gap-2">
								{snapshots.map((item) => (
									<Button
										key={item.snapshotId}
										type="button"
										aria-pressed={snapshot?.snapshotId === item.snapshotId}
										variant={
											snapshot?.snapshotId === item.snapshotId ? "default" : "outline"
										}
										className="h-auto justify-start whitespace-normal p-3 text-left"
										onClick={() => {
											setSnapshot(reuseSnapshot(snapshots, item.snapshotId));
											setSnapshotTechnicalError("");
										}}
									>
										<span>
											{item.src.toUpperCase()} · {item.code} · {item.interval} ·{" "}
											{new Date(item.startTimeMs).toISOString().slice(0, 10)}–
											{new Date(item.endTimeMs).toISOString().slice(0, 10)} ·{" "}
											{item.barCount} Bars · {item.gaps.length} gaps
										</span>
										<span className="block break-all text-xs opacity-75">
											Snapshot {item.snapshotId}
										</span>
									</Button>
								))}
							</div>
						)}
						{snapshotTechnicalError && (
							<div
								className="mt-3 rounded-md border border-destructive p-3 text-sm"
								role="alert"
							>
								<p>Snapshot error</p>
								<pre className="mt-1 overflow-x-auto whitespace-pre-wrap text-xs">
									{snapshotTechnicalError}
								</pre>
							</div>
										)}
									</div>
						</>
					)}
				</CardContent>
			</Card>
			{stage === "execution" && snapshot && selectedStrategy && (
				<Card className="mt-4">
					<CardHeader>
						<CardTitle>Execution and pre-Run review</CardTitle>
					</CardHeader>
					<CardContent className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
						<Field label="Initial quote allocation" id="backtest-allocation">
							<Input
								id="backtest-allocation"
								type="text"
								inputMode="decimal"
								value={initialQuoteAllocation}
								onChange={(event) => {
									setInitialQuoteAllocation(event.target.value);
									setPreflight(undefined);
								}}
							/>
						</Field>
						{Object.entries(executionProfile)
							.filter(([name]) => name !== "fillPolicy")
							.map(([name, value]) => (
								<Field key={name} label={name.replace(/[A-Z]/g, (letter) => ` ${letter.toLowerCase()}`)}>
									<Input
										type="text"
										inputMode="decimal"
										value={value}
										onChange={(event) => {
											setExecutionProfile((current) => ({ ...current, [name]: event.target.value }));
											setPreflight(undefined);
										}}
									/>
								</Field>
							))}
						<Field label="Fill policy">
							<select
								className="h-9 rounded-md border bg-background px-3"
								value={executionProfile.fillPolicy}
								onChange={(event) => {
									setExecutionProfile((current) => ({
										...current,
										fillPolicy: event.target.value as "maker" | "taker",
									}));
									setPreflight(undefined);
								}}
							>
								<option value="taker">Taker</option>
								<option value="maker">Maker</option>
							</select>
						</Field>
						<div className="md:col-span-2 lg:col-span-3 rounded-md border p-3 text-sm">
							<p className="font-medium">Authoritative inputs</p>
							<pre className="mt-2 overflow-x-auto whitespace-pre-wrap text-xs">
								{JSON.stringify(preflight, null, 2)}
							</pre>
						</div>
						{runTechnicalError && (
							<div className="md:col-span-2 lg:col-span-3 rounded-md border border-destructive p-3 text-sm" role="alert">
								<p>Backtest error</p>
								<pre className="mt-1 overflow-x-auto whitespace-pre-wrap text-xs">{runTechnicalError}</pre>
							</div>
						)}
						<Button
							disabled={running}
							onClick={() => void (preflight ? execute() : selectStage("execution"))}
						>
							{running ? "Validating…" : preflight ? "Run Backtest" : "Validate inputs"}
						</Button>
						<p className="self-center text-sm text-muted-foreground" role="status" aria-live="polite">
							{message}
						</p>
					</CardContent>
				</Card>
			)}
			{stage === "results" && run && (
				<>
					<Card>
						<CardContent className="grid grid-cols-2 gap-4 py-4 md:grid-cols-4 lg:grid-cols-6">
							<Metric
								label="Total return"
								value={percent(run.result.metrics.totalReturn)}
							/>
							<Metric label="CAGR" value={percent(run.result.metrics.cagr)} />
							<Metric
								label="Max drawdown"
								value={percent(run.result.metrics.maxDrawdown)}
							/>
							<Metric
								label="Sharpe"
								value={formatDecimal(run.result.metrics.sharpe)}
							/>
							<Metric
								label="Sortino"
								value={formatDecimal(run.result.metrics.sortino)}
							/>
							<Metric
								label="Excess return"
								value={percent(run.result.metrics.excessReturn)}
							/>
							<Metric
								label="Final equity"
								value={formatDecimal(run.result.metrics.finalEquity)}
							/>
							<Metric
								label="Realized P&L"
								value={formatDecimal(run.result.metrics.realizedPnl)}
							/>
							<Metric
								label="Unrealized P&L"
								value={formatDecimal(run.result.metrics.unrealizedPnl)}
							/>
							<Metric label="Fees" value={formatDecimal(run.result.totalFees)} />
							<Metric label="Win rate" value={percent(run.result.metrics.winRate)} />
							<Metric label="Fills" value={String(run.result.metrics.fillCount)} />
						</CardContent>
					</Card>
					<Card>
						<CardHeader>
							<CardTitle>Replay provenance</CardTitle>
						</CardHeader>
						<CardContent className="space-y-1 break-all text-sm text-muted-foreground">
							{run.provenance ? (
								<pre className="overflow-x-auto whitespace-pre-wrap">
									{JSON.stringify(run.provenance, null, 2)}
								</pre>
							) : (
								<p>Legacy Run: complete replay provenance was not recorded.</p>
							)}
						</CardContent>
					</Card>
					<Card>
						<CardContent className="pt-4">
							<BacktestChart run={run} onVisibleRangeChange={loadChartRange} />
						</CardContent>
					</Card>
					<ExecutionTables
						page={executionPage}
						offset={executionOffset}
						onOffset={setExecutionOffset}
					/>
				</>
			)}
			<Card>
				<CardHeader>
					<CardTitle>Run history</CardTitle>
				</CardHeader>
				<CardContent className="space-y-2">
					{history.map((item) => (
						<div
							key={item.runId}
							className="flex items-center justify-between rounded-md border p-3 text-sm"
						>
							<button
								type="button"
								className="text-left"
								onClick={() =>
									userId &&
									void invoke<BacktestRun>("backtest_get", {
										request: { userId, runId: item.runId },
									}).then((value) => {
										setRun(value);
										setExecutionOffset(0);
									})
								}
							>
								<span className="font-medium">
									{item.code} · {item.interval}
								</span>
								<span className="ml-3 text-muted-foreground">
									{item.barCount} Bars · {percent(item.totalReturn)}
								</span>
							</button>
							<Button
								size="sm"
								variant="ghost"
								onClick={() =>
									userId &&
									void invoke("backtest_delete", {
										request: { userId, runId: item.runId },
									}).then(refreshHistory)
								}
							>
								Delete
							</Button>
						</div>
					))}
					{history.length === 0 && (
						<p className="text-sm text-muted-foreground">No persisted Runs.</p>
					)}
				</CardContent>
			</Card>
			<Card>
				<CardHeader>
					<CardTitle>Research validation</CardTitle>
				</CardHeader>
				<CardContent className="space-y-3 text-sm">
					{protocols.map((protocol) => (
						<div
							key={protocol.protocolId}
							className="flex items-center justify-between rounded-md border p-3"
						>
							<span className="font-mono">
								{protocol.protocolId.slice(0, 16)} · {protocol.methodVersion} ·{" "}
								{protocol.crossMarket?.contexts.length ?? protocol.windows.length}{" "}
								context
								{(protocol.crossMarket?.contexts.length ?? protocol.windows.length) ===
								1
									? ""
									: "s"}
							</span>
							<Button
								size="sm"
								disabled={runningProtocolId === protocol.protocolId}
								onClick={() => void runValidation(protocol.protocolId)}
							>
								{runningProtocolId === protocol.protocolId
									? "Running…"
									: "Run / resume"}
							</Button>
						</div>
					))}
					{reports.map((report) => (
						<div key={report.reportId} className="rounded-md border p-3">
							<div className="flex items-center justify-between gap-2">
								<span className="font-mono">{report.reportId.slice(0, 16)}</span>
								<div className="flex gap-2">
									<Button
										size="sm"
										variant="outline"
										onClick={() => void exportReport(report.reportId, "json")}
									>
										JSON
									</Button>
									<Button
										size="sm"
										variant="outline"
										onClick={() => void exportReport(report.reportId, "markdown")}
									>
										Markdown
									</Button>
								</div>
							</div>
							<p className="mt-2 text-muted-foreground">
								{report.methodVersion}
								{report.walkForward
									? ` · ${report.walkForward.windowSizeBars}-Bar windows, ${report.walkForward.stepSizeBars}-Bar step, ${report.walkForward.minimumHistoryBars}-Bar history`
									: ""}
							</p>
							<p className="mt-2 text-muted-foreground">
								{report.aggregate.completedWindows} complete ·{" "}
								{report.aggregate.failedWindows} failed · Out{" "}
								{percent(report.aggregate.averageSampleOutReturn)} · Drawdown{" "}
								{percent(report.aggregate.worstSampleOutDrawdown)} · Sharpe{" "}
								{formatDecimal(report.aggregate.averageSampleOutSharpe)} · Fees{" "}
								{formatDecimal(report.aggregate.totalFees)} · Trades{" "}
								{report.aggregate.totalTrades}
							</p>
							{report.crossMarketEvidence && (
								<p className="mt-2 text-muted-foreground">
									{report.crossMarketEvidence.completedMarkets} markets · Return spread{" "}
									{percent(report.crossMarketEvidence.totalReturnSpread)} ·{" "}
									{report.recommendedContexts.length} evidence-backed Recommended
									Contexts
								</p>
							)}
							<div className="mt-2 space-y-1 text-xs text-muted-foreground">
								{report.crossMarket.map((context) => (
									<p key={context.snapshot.snapshotId}>
										{context.snapshot.code} · {context.snapshot.interval} ·{" "}
										{context.failure
											? `Failed: ${context.failure}`
											: `Return ${percent(context.metrics?.totalReturn ?? "0")} · Drawdown ${percent(context.metrics?.maxDrawdown ?? "0")} · Sharpe ${formatDecimal(context.metrics?.sharpe ?? "0")} · ${context.pauses.length} pauses`}
									</p>
								))}
								{report.windows.map((window) => (
									<p
										key={`${window.sampleOutStartTimeMs}:${window.sampleOutEndTimeMs ?? "final"}`}
									>
										{new Date(window.sampleOutStartTimeMs).toLocaleString()}{" "}
										{window.sampleOutEndTimeMs
											? `– ${new Date(window.sampleOutEndTimeMs).toLocaleString()} `
											: "– final "}
										·{" "}
										{window.failure
											? `Failed: ${window.failure}`
											: `Return ${percent(window.sampleOutMetrics?.totalReturn ?? "0")} · Drawdown ${percent(window.sampleOutMetrics?.maxDrawdown ?? "0")} · Sharpe ${formatDecimal(window.sampleOutMetrics?.sharpe ?? "0")} · ${window.sampleOutPauses.length} pauses`}
									</p>
								))}
							</div>
							<details className="mt-2">
								<summary>
									Inspect immutable Report ({report.windows.length} windows)
								</summary>
								<pre className="mt-2 overflow-x-auto whitespace-pre-wrap text-xs">
									{JSON.stringify(report, null, 2)}
								</pre>
							</details>
						</div>
					))}
					{protocols.length === 0 && (
						<p className="text-muted-foreground">
							Freeze a Snapshot with a holdout boundary or walk-forward configuration
							to begin.
						</p>
					)}
				</CardContent>
			</Card>
		</Workspace>
	);
}

function Field({
	label,
	id,
	children,
}: {
	label: string;
	id?: string;
	children: React.ReactNode;
}) {
	return (
		<div className="grid gap-1">
			<Label htmlFor={id}>{label}</Label>
			{children}
		</div>
	);
}
function ParameterField({
	label,
	parameter,
	value,
	onChange,
}: {
	label: string;
	parameter: LibraryComponent["parameters"][number];
	value: string;
	onChange: (value: string) => void;
}) {
	if (parameter.allowedValues.length)
		return (
			<Field label={label}>
				<select
					className="h-9 rounded-md border bg-background px-3"
					value={value}
					onChange={(event) => onChange(event.target.value)}
				>
					{parameter.allowedValues.map((allowed) => (
						<option key={allowed}>{allowed}</option>
					))}
				</select>
			</Field>
		);
	if (parameter.parameterType === "boolean")
		return (
			<Field label={label}>
				<label className="flex h-9 items-center gap-2">
					<input
						type="checkbox"
						checked={value === "true"}
						onChange={(event) => onChange(String(event.target.checked))}
					/>
					Enabled
				</label>
			</Field>
		);
	return (
		<Field label={label}>
			<Input
				type={parameter.parameterType === "string" ? "text" : "number"}
				step="any"
				value={value}
				onChange={(event) => onChange(event.target.value)}
			/>
		</Field>
	);
}
function ExecutionTables({
	page,
	offset,
	onOffset,
}: {
	page?: ExecutionPage;
	offset: number;
	onOffset: (offset: number) => void;
}) {
	const total = Math.max(page?.totalOrders ?? 0, page?.totalFills ?? 0);
	return (
		<Card>
			<CardHeader className="flex-row items-center justify-between">
				<CardTitle>Orders and fills</CardTitle>
				<div className="flex items-center gap-2 text-xs text-muted-foreground">
					<span>
						{total
							? `${offset + 1}–${Math.min(offset + EXECUTION_PAGE_SIZE, total)} / ${total}`
							: "0"}
					</span>
					<Button
						size="sm"
						variant="outline"
						disabled={offset === 0}
						onClick={() => onOffset(Math.max(0, offset - EXECUTION_PAGE_SIZE))}
					>
						Previous
					</Button>
					<Button
						size="sm"
						variant="outline"
						disabled={offset + EXECUTION_PAGE_SIZE >= total}
						onClick={() => onOffset(offset + EXECUTION_PAGE_SIZE)}
					>
						Next
					</Button>
				</div>
			</CardHeader>
			<CardContent className="grid gap-6 overflow-auto xl:grid-cols-2 [&_td]:whitespace-nowrap [&_td]:py-2 [&_td]:pr-4 [&_th]:whitespace-nowrap [&_th]:pb-2 [&_th]:pr-4">
				<table className="w-full min-w-[640px] text-left text-xs">
					<thead>
						<tr>
							<th>Time</th>
							<th>Side</th>
							<th>Qty</th>
							<th>Limit</th>
							<th>Status</th>
						</tr>
					</thead>
					<tbody>
						{page?.orders.map((order) => (
							<tr key={order.orderId}>
								<td>{new Date(order.createdTimeMs).toLocaleString()}</td>
								<td>{order.side}</td>
								<td>{formatDecimal(order.quantity)}</td>
								<td>{formatDecimal(order.limitPrice)}</td>
								<td>
									{typeof order.status === "string" ? order.status : order.status.status}
								</td>
							</tr>
						))}
					</tbody>
				</table>
				<table className="w-full min-w-[760px] text-left text-xs">
					<thead>
						<tr>
							<th>Time</th>
							<th>Side</th>
							<th>Filled / Requested @ Price</th>
							<th>Role</th>
							<th>Fee / P&amp;L</th>
						</tr>
					</thead>
					<tbody>
						{page?.fills.map((fill) => (
							<tr key={`${fill.orderId}:${fill.openTimeMs}`}>
								<td>{new Date(fill.openTimeMs).toLocaleString()}</td>
								<td>{fill.side}</td>
								<td>
									{formatDecimal(fill.quantity)} /{" "}
									{formatDecimal(fill.requestedQuantity)} @ {formatDecimal(fill.price)}
								</td>
								<td>{fill.role}</td>
								<td>
									{formatDecimal(fill.fee)} / {formatDecimal(fill.realizedPnl)}
								</td>
							</tr>
						))}
					</tbody>
				</table>
			</CardContent>
		</Card>
	);
}
function mergeRange(
	current: BacktestRun,
	view: BacktestRun,
	start: number,
	end: number,
): BacktestRun {
	const replace = <T,>(
		existing: T[],
		incoming: T[],
		time: (value: T) => number,
	) =>
		[
			...existing.filter((value) => time(value) < start || time(value) >= end),
			...incoming,
		].sort((left, right) => time(left) - time(right));
	return {
		...current,
		bars: replace(current.bars, view.bars, (bar) => bar.openTimeMs),
		result: {
			...current.result,
			orders: replace(
				current.result.orders,
				view.result.orders,
				(order) => order.createdTimeMs,
			),
			fills: replace(
				current.result.fills,
				view.result.fills,
				(fill) => fill.openTimeMs,
			),
			equity: replace(
				current.result.equity,
				view.result.equity,
				(point) => point.openTimeMs,
			),
			benchmarkEquity: replace(
				current.result.benchmarkEquity,
				view.result.benchmarkEquity,
				(point) => point.openTimeMs,
			),
		},
	};
}
function Metric({ label, value }: { label: string; value: string }) {
	return (
		<div>
			<p className="text-xs text-muted-foreground">{label}</p>
			<p className="font-mono text-lg">{value}</p>
		</div>
	);
}
function percent(value: string) {
	const number = Number(value);
	return Number.isFinite(number) ? `${(number * 100).toFixed(2)}%` : value;
}
