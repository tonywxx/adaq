import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { LoadingState } from "@/components/loading-state";
import { Label } from "@/components/ui/label";
import {
	Pagination,
	PaginationContent,
	PaginationItem,
	PaginationNext,
	PaginationPrevious,
} from "@/components/ui/pagination";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
	Workspace,
	type LibraryComponent,
} from "@/features/components/components-page";
import { MetricInfo, ResearchMetric } from "@/features/research/metric-info";
import {
	BAR_INTERVALS,
	type BarInterval,
	type OhlcvBar,
} from "@/lib/market-chart-adapter";
import { instrumentKey, useMarketSessionStore } from "@/lib/market-session";
import { useHistoryTab } from "@/lib/navigation-history";
import { Channel, invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { BacktestChart } from "./backtest-chart";
import {
	snapshotError,
	provenanceMessage,
	snapshotRangeError,
	snapshotStatus,
	reuseSnapshot,
} from "./backtest-data";
import {
	copyRunConfiguration,
	decisionSignalEvidence,
	defaultExecutionProfile,
	matchingFactors,
	type NormalizedRunConfiguration,
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
type SnapshotPage = {
	items: Snapshot[];
	total: number;
	page: number;
	pageSize: number;
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
type RunSummary = {
	runId: string;
	createdAt: string;
	code: string;
	interval: string;
	barCount: number;
	totalReturn: string;
};
type RunHistoryPage = {
	items: RunSummary[];
	total: number;
	page: number;
	pageSize: number;
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
	featurePlan: Record<string, unknown>;
	componentLock: Array<Record<string, unknown>>;
};
type SignalCandidate = {
	slot: string;
	datasetId: string;
	signalName: string;
	evidenceState: string;
};
const EXECUTION_PAGE_SIZE = 100;
const RUN_HISTORY_PAGE_SIZE = 10;
const SNAPSHOT_PAGE_SIZE = 10;

export function BacktestPage() {
	const userId = useMarketSessionStore((state) => state.userId);
	const instrument = useMarketSessionStore((state) => state.activeInstrument);
	const watchlist = useMarketSessionStore((state) => state.watchlist);
	const [components, setComponents] = useState<LibraryComponent[]>([]);
	const [factorSelections, setFactorSelections] = useState<
		Record<string, string>
	>({});
	const [signalSelections, setSignalSelections] = useState<
		Record<string, string>
	>({});
	const [compatibleSignals, setCompatibleSignals] = useState<SignalCandidate[]>(
		[],
	);
	const [strategy, setStrategy] = useState("");
	const [factorParameters, setFactorParameters] = useState<
		Record<string, Record<string, string>>
	>({});
	const [strategyParameters, setStrategyParameters] = useState<
		Record<string, string>
	>({});
	const [stage, setStage] = useState<
		"data" | "strategy" | "execution" | "results"
	>("data");
	const [initialQuoteAllocation, setInitialQuoteAllocation] = useState("10000");
	const [executionProfile, setExecutionProfile] = useState(
		defaultExecutionProfile,
	);
	const [seed, setSeed] = useState("0");
	const [running, setRunning] = useState(false);
	const [compatibleFactors, setCompatibleFactors] = useState<
		Record<string, string[]>
	>({});
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
	const [runWindow, setRunWindow] = useState<{
		startTimeMs: number;
		endTimeMs: number;
	}>();
	const [snapshots, setSnapshots] = useState<Snapshot[]>([]);
	const [snapshotsPage, setSnapshotsPage] = useState(1);
	const [snapshotsTotal, setSnapshotsTotal] = useState(0);
	const [run, setRun] = useState<BacktestRun>();
	const [resultTab, setResultTab] = useHistoryTab(
		"backtest-results",
		"overview",
		run?.runId,
	);
	const [executionPage, setExecutionPage] = useState<ExecutionPage>();
	const [executionOffset, setExecutionOffset] = useState(0);
	const [history, setHistory] = useState<RunSummary[]>([]);
	const [historyPage, setHistoryPage] = useState(1);
	const [historyTotal, setHistoryTotal] = useState(0);
	const [message, setMessage] = useState("");
	const [runTechnicalError, setRunTechnicalError] = useState("");
	const [snapshotTechnicalError, setSnapshotTechnicalError] = useState("");
	const [downloadTaskId, setDownloadTaskId] = useState<string>();
	const [componentsLoading, setComponentsLoading] = useState(true);
	const [historyLoading, setHistoryLoading] = useState(true);
	const [snapshotsLoading, setSnapshotsLoading] = useState(true);
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
	const refreshHistory = useCallback(
		async (page: number, isActive: () => boolean = () => true) => {
			if (!userId) return;
			setHistoryLoading(true);
			try {
				const result = await invoke<RunHistoryPage>("backtest_list", {
					request: {
						userId,
						src: selectedInstrument.src,
						code: selectedInstrument.code,
						page,
					},
				});
				if (!isActive()) return;
				setHistory(result.items);
				setHistoryTotal(result.total);
			} catch (error) {
				if (isActive()) setMessage(String(error));
			} finally {
				if (isActive()) setHistoryLoading(false);
			}
		},
		[selectedInstrument.code, selectedInstrument.src, userId],
	);
	const refreshSnapshots = useCallback(
		async (page: number, isActive: () => boolean = () => true) => {
			if (!userId) return;
			setSnapshotsLoading(true);
			try {
				const result = await invoke<SnapshotPage>("snapshot_list", {
					request: {
						userId,
						...selectedInstrument,
						interval,
						page,
					},
				});
				if (!isActive()) return;
				setSnapshots(result.items);
				setSnapshotsTotal(result.total);
			} catch (error) {
				if (isActive()) setSnapshotTechnicalError(snapshotError(error));
			} finally {
				if (isActive()) setSnapshotsLoading(false);
			}
		},
		[interval, selectedInstrument, userId],
	);
	useEffect(() => {
		if (!userId) return;
		let active = true;
		setComponentsLoading(true);
		void invoke<LibraryComponent[]>("component_list", { request: { userId } })
			.then((items) => {
				if (!active) return;
				setComponents(items);
				if (items.some((item) => item.compatibilityError))
					setMessage(
						"Incompatible Components are hidden. Remove them from Component Library and import Manifest 1.0 packages.",
					);
			})
			.catch((error) => active && setMessage(String(error)))
			.finally(() => active && setComponentsLoading(false));
		return () => {
			active = false;
		};
	}, [userId]);
	useEffect(() => {
		if (!userId) return;
		let active = true;
		void refreshHistory(historyPage, () => active);
		return () => {
			active = false;
		};
	}, [historyPage, refreshHistory, userId]);
	useEffect(() => {
		if (!userId) return;
		let active = true;
		void refreshSnapshots(snapshotsPage, () => active);
		return () => {
			active = false;
		};
	}, [refreshSnapshots, snapshotsPage, userId]);
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
	const signalSlots =
		selectedStrategy?.featureSlots.filter(
			(slot) => slot.source.kind === "signal",
		) ?? [];
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
	useEffect(() => {
		setCompatibleSignals([]);
		setPreflight(undefined);
		if (!userId || !strategy || !snapshot) return;
		void invoke<SignalCandidate[]>("backtest_compatible_signals", {
			request: {
				userId,
				strategyArchiveSha256: strategy,
				snapshotId: snapshot.snapshotId,
			},
		})
			.then(setCompatibleSignals)
			.catch((error) => setMessage(String(error)));
	}, [snapshot, strategy, userId]);
	const selectStage = async (
		next: "data" | "strategy" | "execution" | "results",
	) => {
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
				signalSlots,
				signalSelections,
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
			setRunWindow(undefined);
			setSignalSelections({});
			void refreshSnapshots(snapshotsPage);
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
		userId: userId ?? "",
		snapshotId,
		runStartTimeMs: runWindow?.startTimeMs ?? snapshot?.startTimeMs,
		runEndTimeMs: runWindow?.endTimeMs ?? snapshot?.endTimeMs,
		factorInstances:
			selectedStrategy?.dependencies
				.map((dependency) => ({
					alias: dependency.alias,
					archiveSha256: factorSelections[dependency.alias],
					parameters: factorParameters[dependency.alias] ?? {},
				}))
				.filter((factor) => factor.archiveSha256) ?? [],
		signalInstances: Object.entries(signalSelections).map(([slot, selection]) => {
			const [datasetId, signalName] = selection.split(":", 2);
			return { slot, datasetId, signalName };
		}),
		strategyArchiveSha256: strategy,
		strategyParameters,
		initialQuoteAllocation,
		executionProfile,
		seed: Number(seed),
	});
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
			signalSlots,
			signalSelections,
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
			if (historyPage === 1) void refreshHistory(1);
			else setHistoryPage(1);
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
		let current = true;
		setExecutionPage(undefined);
		void invoke<ExecutionPage>("backtest_execution_data", {
			request: {
				userId,
				runId,
				offset: executionOffset,
				limit: EXECUTION_PAGE_SIZE,
			},
		})
			.then((page) => {
				if (!current) return;
				setExecutionPage(page);
				setRunTechnicalError("");
			})
			.catch((error) => {
				if (!current) return;
				const details = String(error);
				setMessage(details);
				setRunTechnicalError(details);
			});
		return () => {
			current = false;
		};
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
	const useRunAsNewConfiguration = (source: BacktestRun) => {
		if (!source.provenance) {
			setMessage(
				"This legacy Run has incomplete provenance and cannot be copied safely.",
			);
			return;
		}
		const configuration = copyRunConfiguration(
			source.provenance.normalizedRequest,
		);
		setSnapshot(source.snapshot);
		setRunWindow({
			startTimeMs: configuration.runStartTimeMs ?? source.snapshot.startTimeMs,
			endTimeMs: configuration.runEndTimeMs ?? source.snapshot.endTimeMs,
		});
		setStrategy(configuration.strategy);
		setStrategyParameters(configuration.strategyParameters);
		setFactorSelections(configuration.factorSelections);
		setFactorParameters(configuration.factorParameters);
		setSignalSelections(configuration.signalSelections);
		setInitialQuoteAllocation(configuration.initialQuoteAllocation);
		setExecutionProfile(configuration.executionProfile);
		setSeed(configuration.seed);
		setPreflight(undefined);
		setStage("strategy");
		setMessage(
			`Run ${source.runId.slice(0, 12)} copied into a new editable configuration.`,
		);
	};
	return (
		<Workspace
			title="Backtest"
			description={`${selectedInstrument.code} · OKX Spot · Long Only`}
		>
			<nav aria-label="Backtest stages" className="mb-4 grid gap-2 sm:grid-cols-4">
				{(["data", "strategy", "execution", "results"] as const).map(
					(item, index) => (
						<Button
							key={item}
							type="button"
							variant={stage === item ? "default" : "outline"}
							aria-current={stage === item ? "step" : undefined}
							onClick={() => void selectStage(item)}
						>
							{index + 1}. {item[0].toUpperCase() + item.slice(1)}
						</Button>
					),
				)}
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
										setHistoryPage(1);
										setSnapshotsPage(1);
										setSnapshot(undefined);
										setRunWindow(undefined);
										setSignalSelections({});
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
										setSnapshotsPage(1);
										setSnapshot(undefined);
										setRunWindow(undefined);
										setSignalSelections({});
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
						</>
					)}
					{stage === "strategy" && componentsLoading && (
						<LoadingState
							label="Loading Strategy Components…"
							className="md:col-span-2 lg:col-span-4"
						/>
					)}
					{stage === "strategy" && !componentsLoading && (
						<>
							<Field label="Strategy">
								<select
									className="h-9 rounded-md border bg-background px-3"
									value={strategy}
									onChange={(e) => {
										setStrategy(e.target.value);
										setFactorSelections({});
										setFactorParameters({});
										setSignalSelections({});
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
											factors,
											compatibleFactors[dependency.alias] ?? [],
										).map((item) => (
											<option key={item.archiveSha256} value={item.archiveSha256}>
												{item.name} v{item.version}
											</option>
										))}
									</select>
								</Field>
							))}
							{signalSlots.map((slot) => (
								<Field key={slot.name} label={`Signal · ${slot.name}`}>
									<select
										className="h-9 rounded-md border bg-background px-3"
										value={signalSelections[slot.name] ?? ""}
										onChange={(event) =>
											setSignalSelections((current) => ({
												...current,
												[slot.name]: event.target.value,
											}))
										}
									>
										<option value="">Select compatible Dataset Signal</option>
										{compatibleSignals
											.filter((candidate) => candidate.slot === slot.name)
											.map((candidate) => (
												<option
													key={`${candidate.datasetId}:${candidate.signalName}`}
													value={`${candidate.datasetId}:${candidate.signalName}`}
												>
													{candidate.signalName} · {candidate.datasetId.slice(0, 12)} ·{" "}
													{candidate.evidenceState}
												</option>
											))}
									</select>
								</Field>
							))}
							{selectedStrategy?.architecture && (
								<p className="self-end text-sm text-muted-foreground">
									Architecture · {selectedStrategy.architecture}
								</p>
							)}
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
								<Button
									loading={Boolean(downloadTaskId)}
									loadingText="Preparing Snapshot…"
									onClick={() => void prepare()}
								>
									Prepare Snapshot
								</Button>
								{downloadTaskId && (
									<Button
										variant="outline"
										onClick={() =>
											void invoke("snapshot_cancel", {
												request: { taskId: downloadTaskId },
											})
										}
									>
										Cancel
									</Button>
								)}
								<Button
									disabled={!snapshot || !strategy}
									onClick={() => void selectStage("execution")}
								>
									Review execution
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
								{snapshotsLoading ? (
									<LoadingState label="Loading Snapshots…" />
								) : snapshots.length === 0 ? (
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
													setRunWindow(undefined);
													setSignalSelections({});
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
								{!snapshotsLoading && snapshotsTotal > SNAPSHOT_PAGE_SIZE && (
									<Pagination className="mt-3">
										<PaginationContent>
											<PaginationItem>
												<PaginationPrevious
													disabled={snapshotsPage === 1}
													onClick={() => setSnapshotsPage((page) => page - 1)}
												/>
											</PaginationItem>
											<PaginationItem>
												<span className="px-3 text-sm" aria-current="page">
													Page {snapshotsPage} of{" "}
													{Math.ceil(snapshotsTotal / SNAPSHOT_PAGE_SIZE)}
												</span>
											</PaginationItem>
											<PaginationItem>
												<PaginationNext
													disabled={
														snapshotsPage >= Math.ceil(snapshotsTotal / SNAPSHOT_PAGE_SIZE)
													}
													onClick={() => setSnapshotsPage((page) => page + 1)}
												/>
											</PaginationItem>
										</PaginationContent>
									</Pagination>
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
						<Field label="Run start · UTC" id="backtest-run-start">
							<Input
								id="backtest-run-start"
								type="datetime-local"
								value={utcInput(runWindow?.startTimeMs ?? snapshot.startTimeMs)}
								onChange={(event) => {
									const startTimeMs = parseUtcInput(event.target.value);
									if (!Number.isFinite(startTimeMs)) return;
									setRunWindow((current) => ({
										startTimeMs,
										endTimeMs: current?.endTimeMs ?? snapshot.endTimeMs,
									}));
									setPreflight(undefined);
								}}
							/>
						</Field>
						<Field label="Run end · UTC" id="backtest-run-end">
							<Input
								id="backtest-run-end"
								type="datetime-local"
								value={utcInput(runWindow?.endTimeMs ?? snapshot.endTimeMs)}
								onChange={(event) => {
									const endTimeMs = parseUtcInput(event.target.value);
									if (!Number.isFinite(endTimeMs)) return;
									setRunWindow((current) => ({
										startTimeMs: current?.startTimeMs ?? snapshot.startTimeMs,
										endTimeMs,
									}));
									setPreflight(undefined);
								}}
							/>
						</Field>
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
						<Field label="Seed" id="backtest-seed">
							<Input
								id="backtest-seed"
								type="number"
								min="0"
								step="1"
								value={seed}
								onChange={(event) => {
									setSeed(event.target.value);
									setPreflight(undefined);
								}}
							/>
						</Field>
						{Object.entries(executionProfile)
							.filter(([name]) => name !== "fillPolicy")
							.map(([name, value]) => (
								<Field
									key={name}
									label={name.replace(/[A-Z]/g, (letter) => ` ${letter.toLowerCase()}`)}
								>
									<Input
										type="text"
										inputMode="decimal"
										value={value}
										onChange={(event) => {
											setExecutionProfile((current) => ({
												...current,
												[name]: event.target.value,
											}));
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
							<div
								className="md:col-span-2 lg:col-span-3 rounded-md border border-destructive p-3 text-sm"
								role="alert"
							>
								<p>Backtest error</p>
								<pre className="mt-1 overflow-x-auto whitespace-pre-wrap text-xs">
									{runTechnicalError}
								</pre>
							</div>
						)}
						<Button
							loading={running}
							onClick={() => void (preflight ? execute() : selectStage("execution"))}
						>
							{running
								? "Validating…"
								: preflight
									? "Run Backtest"
									: "Validate inputs"}
						</Button>
						<p
							className="self-center text-sm text-muted-foreground"
							role="status"
							aria-live="polite"
						>
							{message}
						</p>
					</CardContent>
				</Card>
			)}
			{stage === "results" && run && (
				<Tabs key={run.runId} value={resultTab} onValueChange={setResultTab}>
					<TabsList
						aria-label="Backtest Run results"
						className="w-full justify-start overflow-x-auto"
					>
						<TabsTrigger value="overview">Overview</TabsTrigger>
						<TabsTrigger value="decisions">Decisions</TabsTrigger>
						<TabsTrigger value="execution">Execution</TabsTrigger>
						<TabsTrigger value="provenance">Provenance</TabsTrigger>
					</TabsList>
					{runTechnicalError && (
						<p
							className="rounded-md border border-destructive p-3 text-sm"
							role="alert"
						>
							Results error: {runTechnicalError}
						</p>
					)}
					<TabsContent value="overview" className="space-y-4">
						<Card>
							<CardContent className="grid grid-cols-2 gap-4 py-4 md:grid-cols-4 lg:grid-cols-6">
								<ResearchMetric
									metricId="strategy.total-return"
									value={percent(run.result.metrics.totalReturn)}
									valueClassName="font-mono text-lg"
								/>
								<ResearchMetric
									metricId="strategy.cagr"
									value={percent(run.result.metrics.cagr)}
									valueClassName="font-mono text-lg"
								/>
								<ResearchMetric
									metricId="strategy.max-drawdown"
									value={percent(run.result.metrics.maxDrawdown)}
									valueClassName="font-mono text-lg"
								/>
								<ResearchMetric
									metricId="strategy.sharpe"
									value={formatDecimal(run.result.metrics.sharpe)}
									valueClassName="font-mono text-lg"
								/>
								<ResearchMetric
									metricId="strategy.sortino"
									value={formatDecimal(run.result.metrics.sortino)}
									valueClassName="font-mono text-lg"
								/>
								<ResearchMetric
									metricId="strategy.excess-return"
									value={percent(run.result.metrics.excessReturn)}
									valueClassName="font-mono text-lg"
								/>
								<ResearchMetric
									metricId="strategy.final-equity"
									value={formatDecimal(run.result.metrics.finalEquity)}
									valueClassName="font-mono text-lg"
								/>
								<ResearchMetric
									metricId="strategy.realized-pnl"
									value={formatDecimal(run.result.metrics.realizedPnl)}
									valueClassName="font-mono text-lg"
								/>
								<ResearchMetric
									metricId="strategy.unrealized-pnl"
									value={formatDecimal(run.result.metrics.unrealizedPnl)}
									valueClassName="font-mono text-lg"
								/>
								<ResearchMetric
									metricId="strategy.total-fees"
									value={formatDecimal(run.result.totalFees)}
									valueClassName="font-mono text-lg"
								/>
								<ResearchMetric
									metricId="strategy.win-rate"
									value={percent(run.result.metrics.winRate)}
									valueClassName="font-mono text-lg"
								/>
								<ResearchMetric
									metricId="strategy.fill-count"
									value={String(run.result.metrics.fillCount)}
									valueClassName="font-mono text-lg"
								/>
							</CardContent>
						</Card>
						<Card>
							<CardContent className="pt-4">
								<BacktestChart run={run} onVisibleRangeChange={loadChartRange} />
							</CardContent>
						</Card>
						<p className="text-sm text-muted-foreground">
							Strategy equity is the solid blue line; benchmark is the thin gray line;
							drawdown is the labeled lower area.
						</p>
					</TabsContent>
					<TabsContent value="decisions">
						<DecisionTable run={run} />
					</TabsContent>
					<TabsContent value="execution">
						<ExecutionTables
							page={executionPage}
							offset={executionOffset}
							onOffset={setExecutionOffset}
							error={runTechnicalError}
						/>
					</TabsContent>
					<TabsContent value="provenance">
						<ProvenanceView run={run} onUseAsNew={useRunAsNewConfiguration} />
					</TabsContent>
				</Tabs>
			)}
			<Card>
				<CardHeader>
					<CardTitle>Run history · {selectedInstrument.code}</CardTitle>
				</CardHeader>
				<CardContent className="flex flex-col gap-2">
					{historyLoading && <LoadingState label="Loading Run History…" />}
					{!historyLoading &&
						history.map((item) => (
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
										})
											.then((value) => {
												setRun(value);
												setExecutionOffset(0);
												setRunTechnicalError("");
												setStage("results");
											})
											.catch((error) => {
												const details = String(error);
												setMessage(details);
												setRunTechnicalError(details);
											})
									}
								>
									<span className="font-medium">
										{item.code} · {item.interval}
									</span>
									<span className="ml-3 text-muted-foreground">
										{item.barCount} Bars
									</span>
								</button>
								<ResearchMetric
									metricId="strategy.total-return"
									value={percent(item.totalReturn)}
									className="ml-3"
									valueClassName="text-sm font-medium"
								/>
							</div>
						))}
					{!historyLoading && history.length === 0 && (
						<p className="text-sm text-muted-foreground">
							No persisted Runs for {selectedInstrument.code}.
						</p>
					)}
					{!historyLoading && historyTotal > RUN_HISTORY_PAGE_SIZE && (
						<Pagination>
							<PaginationContent>
								<PaginationItem>
									<PaginationPrevious
										disabled={historyPage === 1}
										onClick={() => setHistoryPage((page) => page - 1)}
									/>
								</PaginationItem>
								<PaginationItem>
									<span className="px-3 text-sm" aria-current="page">
										Page {historyPage} of{" "}
										{Math.ceil(historyTotal / RUN_HISTORY_PAGE_SIZE)}
									</span>
								</PaginationItem>
								<PaginationItem>
									<PaginationNext
										disabled={
											historyPage >= Math.ceil(historyTotal / RUN_HISTORY_PAGE_SIZE)
										}
										onClick={() => setHistoryPage((page) => page + 1)}
									/>
								</PaginationItem>
							</PaginationContent>
						</Pagination>
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
function utcInput(timeMs: number) {
	return new Date(timeMs).toISOString().slice(0, 16);
}
function parseUtcInput(value: string) {
	return Date.parse(`${value}:00Z`);
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
function DecisionTable({ run }: { run: BacktestRun }) {
	const entries = [
		...run.decisions.map((decision) => ({
			...decision,
			type: "Target Decision" as const,
			description: `${
				decision.targetExposure === "0"
					? "Flat target exposure (not a Run Pause)"
					: `Target exposure ${formatDecimal(decision.targetExposure)}`
			} · ${
				run.provenance
					? decisionSignalEvidence(
							run.provenance.featurePlanJson,
							decision.openTimeMs,
						)
					: "Legacy Run signal evidence is unavailable."
			}`,
		})),
		...run.pauses.map((pause) => ({
			...pause,
			type: "Run Pause" as const,
			description: pauseDescription(pause.reason),
		})),
	].sort((left, right) => left.openTimeMs - right.openTimeMs);
	return (
		<Card>
			<CardHeader>
				<CardTitle>Target Decisions and Run Pauses</CardTitle>
			</CardHeader>
			<CardContent className="overflow-auto">
				{entries.length ? (
					<table className="w-full min-w-[680px] text-left text-sm [&_td]:border-t [&_td]:py-2 [&_td]:pr-4 [&_th]:pb-2 [&_th]:pr-4">
						<thead>
							<tr>
								<th>Time</th>
								<th>Record</th>
								<th>Evidence</th>
							</tr>
						</thead>
						<tbody>
							{entries.map((entry) => (
								<tr key={`${entry.type}:${entry.openTimeMs}:${entry.description}`}>
									<td>{new Date(entry.openTimeMs).toLocaleString()}</td>
									<td>{entry.type}</td>
									<td>{entry.description}</td>
								</tr>
							))}
						</tbody>
					</table>
				) : (
					<p className="text-muted-foreground">
						No Target Decisions or Run Pauses were recorded.
					</p>
				)}
			</CardContent>
		</Card>
	);
}

function pauseDescription(reason: string) {
	if (reason === "warmup") return "Warmup — no Target Decision was invoked.";
	if (reason.startsWith("missing-input:")) {
		const [, slot, ...sourceParts] = reason.split(":");
		const source = sourceParts.join(":");
		return `Missing Input${slot ? ` — Slot ${slot}` : ""}${source ? ` from ${source}` : ""}; no Target Decision was invoked.`;
	}
	return `Run Pause — ${reason}; no Target Decision was invoked.`;
}

function ProvenanceView({
	run,
	onUseAsNew,
}: {
	run: BacktestRun;
	onUseAsNew: (run: BacktestRun) => void;
}) {
	const provenance = run.provenance;
	const legacyMessage = provenanceMessage(Boolean(provenance));
	return (
		<Card>
			<CardHeader className="flex-row items-center justify-between gap-3">
				<CardTitle>Immutable Run provenance</CardTitle>
				<Button size="sm" disabled={!provenance} onClick={() => onUseAsNew(run)}>
					Use as new configuration
				</Button>
			</CardHeader>
			<CardContent className="space-y-4 text-sm">
				{!provenance ? (
					<p role="alert">{legacyMessage}</p>
				) : (
					<>
						<Evidence label="Run ID" value={run.runId} />
						<Evidence label="Snapshot ID" value={run.snapshot.snapshotId} />
						<Evidence label="Feature Plan hash" value={provenance.featurePlanHash} />
						<Evidence label="Architecture" value={provenance.architecture} />
						<Evidence label="Seed" value={String(provenance.seed)} />
						<Evidence
							label="Strategy Package"
							value={provenance.normalizedRequest.strategyArchiveSha256}
						/>
						<Evidence
							label="Strategy parameters"
							value={JSON.stringify(
								provenance.normalizedRequest.strategyParameters,
								null,
								2,
							)}
						/>
						<Evidence
							label="Factor instances"
							value={JSON.stringify(
								provenance.normalizedRequest.factorInstances,
								null,
								2,
							)}
						/>
						<Evidence
							label="Dataset Signals"
							value={JSON.stringify(
								provenance.normalizedRequest.signalInstances,
								null,
								2,
							)}
						/>
						<Evidence
							label="Dataset Locks and Evidence State"
							value={JSON.stringify(provenance.datasetLock, null, 2)}
						/>
						<Evidence
							label="Initial quote allocation"
							value={provenance.normalizedRequest.initialQuoteAllocation}
						/>
						<Evidence
							label="Execution Profile"
							value={JSON.stringify(
								provenance.normalizedRequest.executionProfile,
								null,
								2,
							)}
						/>
						<Evidence
							label="Component Packages"
							value={JSON.stringify(provenance.componentLock, null, 2)}
						/>
						<Evidence label="Feature Plan" value={provenance.featurePlanJson} />
						<Evidence
							label="Indicator Engine identity"
							value={JSON.stringify(provenance.indicatorEngineBuildIdentity, null, 2)}
						/>
						<Evidence
							label="Backtest engine version"
							value={provenance.backtestEngineVersion}
						/>
					</>
				)}
			</CardContent>
		</Card>
	);
}

function Evidence({ label, value }: { label: string; value: string }) {
	return (
		<div className="rounded-md border p-3">
			<div className="mb-2 flex flex-wrap items-center justify-between gap-2">
				<p className="font-medium">{label}</p>
				<Button
					size="xs"
					variant="outline"
					onClick={() => void navigator.clipboard.writeText(value)}
				>
					Copy {label}
				</Button>
			</div>
			<pre className="max-h-48 overflow-auto whitespace-pre-wrap break-all rounded-md bg-muted p-2 text-xs">
				{value}
			</pre>
		</div>
	);
}
function ExecutionTables({
	page,
	offset,
	onOffset,
	error,
}: {
	page?: ExecutionPage;
	offset: number;
	onOffset: (offset: number) => void;
	error?: string;
}) {
	const total = Math.max(page?.totalOrders ?? 0, page?.totalFills ?? 0);
	return (
		<Card>
			<CardHeader className="flex-row items-center justify-between">
				<CardTitle>Orders and fills</CardTitle>
				<div className="flex items-center gap-2 text-xs text-muted-foreground">
					<ResearchMetric
						metricId="execution.order-count"
						value={String(page?.totalOrders ?? 0)}
						valueClassName="font-medium"
					/>
					<ResearchMetric
						metricId="strategy.fill-count"
						value={String(page?.totalFills ?? 0)}
						valueClassName="font-medium"
					/>
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
				{error ? (
					<p className="xl:col-span-2" role="alert">
						Execution query failed: {error}
					</p>
				) : !page ? (
					<p className="xl:col-span-2 text-muted-foreground" role="status">
						Loading paged execution evidence…
					</p>
				) : total === 0 ? (
					<p className="xl:col-span-2 text-muted-foreground">
						No simulated Orders or Fills were recorded.
					</p>
				) : null}
				<table className="w-full min-w-[640px] text-left text-xs">
					<thead>
						<tr>
							<th>Time</th>
							<th>Side</th>
							<th>
								<MetricInfo metricId="execution.order-quantity" />
							</th>
							<th>
								<MetricInfo metricId="execution.limit-price" />
							</th>
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
						{page && page.orders.length === 0 && total > 0 && (
							<tr>
								<td colSpan={5}>No Orders on this page.</td>
							</tr>
						)}
					</tbody>
				</table>
				<table className="w-full min-w-[760px] text-left text-xs">
					<thead>
						<tr>
							<th>Time</th>
							<th>Side</th>
							<th>
								<div className="grid gap-1">
									<MetricInfo metricId="execution.fill-quantity" />
									<MetricInfo metricId="execution.requested-quantity" />
									<MetricInfo metricId="execution.fill-price" />
								</div>
							</th>
							<th>Role</th>
							<th>
								<div className="grid gap-1">
									<MetricInfo metricId="execution.fill-fee" />
									<MetricInfo metricId="execution.fill-realized-pnl" />
								</div>
							</th>
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
						{page && page.fills.length === 0 && total > 0 && (
							<tr>
								<td colSpan={5}>No Fills on this page.</td>
							</tr>
						)}
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
	const existingBars = current.bars.filter(
		(bar) => bar.openTimeMs >= start && bar.openTimeMs < end,
	);
	if (
		existingBars.length === view.bars.length &&
		existingBars.every(
			(bar, index) => bar.openTimeMs === view.bars[index]?.openTimeMs,
		)
	)
		return current;
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
function percent(value: string) {
	const number = Number(value);
	return Number.isFinite(number) ? `${(number * 100).toFixed(2)}%` : value;
}
