import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
	Workspace,
	type LibraryComponent,
} from "@/features/components/components-page";
import type { OhlcvBar } from "@/lib/market-chart-adapter";
import { useMarketSessionStore } from "@/lib/market-session";
import { Channel, invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { BacktestChart } from "./backtest-chart";
import { formatDecimal } from "./format-decimal";

type Snapshot = {
	snapshotId: string;
	barCount: number;
	startTimeMs: number;
	endTimeMs: number;
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
type ExecutionPage = {
	orders: Order[];
	fills: Fill[];
	totalOrders: number;
	totalFills: number;
};
const EXECUTION_PAGE_SIZE = 100;

export function BacktestPage() {
	const userId = useMarketSessionStore((state) => state.userId);
	const instrument = useMarketSessionStore((state) => state.activeInstrument);
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
	const [start, setStart] = useState(() =>
		new Date(Date.now() - 30 * 864e5).toISOString().slice(0, 10),
	);
	const [end, setEnd] = useState(() => new Date().toISOString().slice(0, 10));
	const [snapshot, setSnapshot] = useState<Snapshot>();
	const [run, setRun] = useState<BacktestRun>();
	const [executionPage, setExecutionPage] = useState<ExecutionPage>();
	const [executionOffset, setExecutionOffset] = useState(0);
	const [history, setHistory] = useState<RunSummary[]>([]);
	const [message, setMessage] = useState("");
	const [downloadTaskId, setDownloadTaskId] = useState<string>();
	const chartRequest = useRef(0);
	const chartRange = useRef("");
	const refreshHistory = async () => {
		if (userId)
			try {
				setHistory(await invoke("backtest_list", { request: { userId } }));
			} catch (error) {
				setMessage(String(error));
			}
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
		}
	}, [userId]);
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
	const prepare = async () => {
		const taskId = crypto.randomUUID();
		const onEvent = new Channel<{
			event: "progress" | "completed" | "cancelled";
			data?: { downloadedBars?: number };
		}>();
		onEvent.onmessage = (event) => {
			if (event.event === "progress")
				setMessage(`Downloaded ${event.data?.downloadedBars ?? 0} Closed Bars…`);
			if (event.event === "cancelled") setMessage("Download cancelled.");
		};
		setDownloadTaskId(taskId);
		setMessage("Downloading and freezing Closed Bars…");
		try {
			const value = await invoke<Snapshot>("snapshot_download", {
				request: {
					taskId,
					...instrument,
					interval: "1h",
					startTimeMs: Date.parse(start),
					endTimeMs: Date.parse(end),
				},
				onEvent,
			});
			setSnapshot(value);
			setMessage(`${value.barCount} Bars frozen.`);
		} catch (error) {
			if (!String(error).includes("cancelled")) setMessage(String(error));
		} finally {
			setDownloadTaskId(undefined);
		}
	};
	const execute = async () => {
		if (!userId || !snapshot) return;
		setMessage("Running deterministic Backtest…");
		try {
			const value = await invoke<BacktestRun>("backtest_run", {
				request: {
					userId,
					snapshotId: snapshot.snapshotId,
					factorInstances:
						selectedStrategy?.dependencies
							.map((dependency) => ({
								alias: dependency.alias,
								archiveSha256: factorSelections[dependency.alias],
								parameters: factorParameters[dependency.alias] ?? {},
							}))
							.filter((instance) => instance.archiveSha256) ?? [],
					strategyArchiveSha256: strategy,
					strategyParameters,
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
						fillPolicy: "taker",
					},
				},
			});
			setRun(value);
			setExecutionOffset(0);
			setMessage(`Run ${value.runId.slice(0, 12)} completed.`);
			await refreshHistory();
		} catch (error) {
			setMessage(String(error));
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
			description={`${instrument.code} · OKX Spot · Long Only`}
		>
			<Card>
				<CardHeader>
					<CardTitle>Run configuration</CardTitle>
				</CardHeader>
				<CardContent className="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
					<Field label="Start">
						<Input
							type="date"
							value={start}
							onChange={(e) => setStart(e.target.value)}
						/>
					</Field>
					<Field label="End">
						<Input type="date" value={end} onChange={(e) => setEnd(e.target.value)} />
					</Field>
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
								{factors
									.filter((item) => item.componentId === dependency.componentId)
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
					<div className="flex items-end gap-2">
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
						<Button
							disabled={
								!snapshot ||
								!strategy ||
								Boolean(
									selectedStrategy?.dependencies.some(
										(dependency) => !factorSelections[dependency.alias],
									),
								)
							}
							onClick={() => void execute()}
						>
							Run
						</Button>
					</div>
					<p className="self-end text-sm text-muted-foreground" aria-live="polite">
						{message}
					</p>
				</CardContent>
			</Card>
			{run && (
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
		</Workspace>
	);
}

function Field({
	label,
	children,
}: {
	label: string;
	children: React.ReactNode;
}) {
	return (
		<div className="grid gap-1">
			<Label>{label}</Label>
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
