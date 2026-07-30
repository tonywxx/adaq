import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import type { BacktestRun } from "@/features/backtest/backtest-page";
import { formatDecimal } from "@/features/backtest/format-decimal";
import type { LibraryComponent } from "@/features/components/component-library";
import { Workspace } from "@/features/components/components-page";
import { useMarketSessionStore } from "@/lib/market-session";
import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useState } from "react";
import {
	formatValidationError,
	holdoutGate,
	protocolDetails,
	protocolSummary,
	reportExportFilename,
	validationRunRequest,
	walkForwardGate,
	walkForwardProtocolFields,
	walkForwardPreview as previewWalkForward,
} from "./validation-workspace";

type RunSummary = {
	runId: string;
	createdAt: string;
	code: string;
	interval: string;
	barCount: number;
	totalReturn: string;
};
type Protocol = {
	protocolId: string;
	methodVersion: string;
	aggregationRuleVersion: string;
	run: Record<string, unknown>;
	windows: Array<{
		snapshotId: string;
		sampleOutStartTimeMs: number;
		sampleOutEndTimeMs?: number;
	}>;
	walkForward?: {
		snapshotId: string;
		windowSizeBars: number;
		stepSizeBars: number;
		minimumHistoryBars: number;
	};
};
type Report = {
	reportId: string;
	protocolId: string;
	methodVersion: string;
	aggregationRuleVersion: string;
	windows: Array<{
		sampleOutStartTimeMs: number;
		sampleOutEndTimeMs?: number;
		sampleInSnapshotId: string;
		sampleOutSnapshotId: string;
		sampleInRunId?: string;
		sampleOutRunId?: string;
		sampleInMetrics?: Metrics;
		sampleOutMetrics?: Metrics;
		sampleInPauses: Array<{ openTimeMs: number; reason: string }>;
		sampleOutPauses: Array<{ openTimeMs: number; reason: string }>;
		failure?: string;
	}>;
	aggregate: {
		completedWindows: number;
		failedWindows: number;
		averageSampleInReturn: string;
		averageSampleOutReturn: string;
		worstSampleOutDrawdown: string;
		averageSampleOutSharpe: string;
		totalFees: string;
		totalTrades: number;
	};
	walkForward?: Protocol["walkForward"];
};
type Metrics = { totalReturn: string; maxDrawdown: string; sharpe: string };

export function ValidationPage() {
	const userId = useMarketSessionStore((state) => state.userId);
	const [runs, setRuns] = useState<RunSummary[]>([]);
	const [components, setComponents] = useState<LibraryComponent[]>([]);
	const [source, setSource] = useState<BacktestRun>();
	const [method, setMethod] = useState<"chronological-holdout" | "walk-forward">(
		"chronological-holdout",
	);
	const [sampleOutStart, setSampleOutStart] = useState("");
	const [windowSizeBars, setWindowSizeBars] = useState("100");
	const [stepSizeBars, setStepSizeBars] = useState("100");
	const [minimumHistoryBars, setMinimumHistoryBars] = useState("500");
	const [protocols, setProtocols] = useState<Protocol[]>([]);
	const [reports, setReports] = useState<Report[]>([]);
	const [selectedReportId, setSelectedReportId] = useState("");
	const [runningProtocolId, setRunningProtocolId] = useState<string>();
	const [feedback, setFeedback] = useState<{
		summary: string;
		details?: string;
	}>();

	const refresh = useCallback(async () => {
		if (!userId) return;
		const [nextRuns, nextComponents, nextProtocols, nextReports] =
			await Promise.all([
				invoke<RunSummary[]>("backtest_list", { request: { userId } }),
				invoke<LibraryComponent[]>("component_list", { request: { userId } }),
				invoke<Protocol[]>("validation_protocol_list", { request: { userId } }),
				invoke<Report[]>("validation_report_list", { request: { userId } }),
			]);
		setRuns(nextRuns);
		setComponents(nextComponents);
		setProtocols(nextProtocols);
		setReports(nextReports);
		setSelectedReportId((current) =>
			nextReports.some((report) => report.reportId === current)
				? current
				: (nextReports[0]?.reportId ?? ""),
		);
	}, [userId]);
	useEffect(() => {
		setSource(undefined);
		setSampleOutStart("");
		setFeedback(undefined);
		void refresh().catch((error) =>
			setFeedback({
				summary: "Validation evidence could not load.",
				details: String(error),
			}),
		);
	}, [refresh]);
	const labels = useMemo(
		() =>
			new Map(
				components.map((component) => [
					component.archiveSha256,
					`${component.name} v${component.version}`,
				]),
			),
		[components],
	);
	const selectedReport =
		reports.find((report) => report.reportId === selectedReportId) ?? reports[0];
	const walkForward = {
		snapshotId: source?.snapshot.snapshotId ?? "",
		windowSizeBars: Number(windowSizeBars),
		stepSizeBars: Number(stepSizeBars),
		minimumHistoryBars: Number(minimumHistoryBars),
	};
	const walkForwardError = walkForwardGate({
		runId: source?.runId,
		barCount: source?.bars.length,
		configuration: walkForward,
	});
	const walkForwardPreview =
		!walkForwardError && source
			? previewWalkForward(source.bars, walkForward)
			: undefined;
	const selectRun = async (runId: string) => {
		if (!userId) return;
		setFeedback(undefined);
		try {
			setSource(
				await invoke<BacktestRun>("backtest_get", { request: { userId, runId } }),
			);
		} catch (error) {
			setFeedback({
				summary: "Backtest Run could not load.",
				details: String(error),
			});
		}
	};
	const freeze = async () => {
		const boundary = Date.parse(sampleOutStart);
		const gate =
			method === "chronological-holdout"
				? holdoutGate({
						runId: source?.runId,
						sampleOutStartTimeMs: boundary,
					})
				: walkForwardError;
		if (gate || !userId || !source?.provenance) {
			setFeedback({
				summary: gate ?? "This Backtest Run has incomplete provenance.",
			});
			return;
		}
		try {
			const protocol = await invoke<Protocol>("validation_protocol_create", {
				request: {
					userId,
					run: validationRunRequest(userId, source.provenance.normalizedRequest),
					...(method === "chronological-holdout"
						? {
								windows: [
									{
										snapshotId: source.snapshot.snapshotId,
										sampleOutStartTimeMs: boundary,
									},
								],
								methodVersion: "chronological-holdout@1",
							}
						: walkForwardProtocolFields(walkForward)),
					aggregationRuleVersion: "equal-window@1",
				},
			});
			setFeedback({
				summary: `Protocol ${protocol.protocolId.slice(0, 16)} frozen and immutable.`,
			});
			await refresh();
		} catch (error) {
			setFeedback(formatValidationError(error));
		}
	};
	const run = async (protocolId: string) => {
		if (!userId || runningProtocolId) return;
		setRunningProtocolId(protocolId);
		setFeedback({ summary: "Running Validation Protocol…" });
		try {
			const report = await invoke<Report>("validation_report_run", {
				request: { userId, protocolId },
			});
			setFeedback({
				summary: `Validation Report ${report.reportId.slice(0, 16)} completed.`,
			});
			await refresh();
		} catch (error) {
			setFeedback({
				summary: "Validation could not run or resume.",
				details: String(error),
			});
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
			const url = URL.createObjectURL(
				new Blob([content], {
					type: format === "json" ? "application/json" : "text/markdown",
				}),
			);
			const anchor = document.createElement("a");
			anchor.href = url;
			anchor.download = reportExportFilename(reportId, format);
			anchor.click();
			URL.revokeObjectURL(url);
		} catch (error) {
			setFeedback({ summary: "Report export failed.", details: String(error) });
		}
	};
	return (
		<Workspace
			title="Validation"
			description="Immutable chronological holdout and walk-forward research evidence."
		>
			<Card>
				<CardHeader>
					<CardTitle>1. Choose validation method</CardTitle>
				</CardHeader>
				<CardContent className="space-y-3">
					<div
						className="grid gap-2 sm:grid-cols-2"
						role="radiogroup"
						aria-label="Validation method"
					>
						<label className="rounded-md border p-3 text-sm">
							<input
								type="radio"
								name="validation-method"
								checked={method === "chronological-holdout"}
								onChange={() => setMethod("chronological-holdout")}
							/>{" "}
							<span className="font-medium">Chronological holdout</span>
							<p className="mt-1 text-muted-foreground">
								Train before one boundary, then evaluate later evidence.
							</p>
						</label>
						<label className="rounded-md border p-3 text-sm">
							<input
								type="radio"
								name="validation-method"
								checked={method === "walk-forward"}
								onChange={() => setMethod("walk-forward")}
							/>{" "}
							<span className="font-medium">Walk-forward</span>
							<p className="mt-1 text-muted-foreground">
								Repeat ordered train and complete sample-out windows on one frozen
								Snapshot.
							</p>
						</label>
					</div>
					<div className="rounded-md border p-3 text-sm">
						Method version:{" "}
						<code>
							{method === "chronological-holdout"
								? "chronological-holdout@1"
								: "walk-forward@1"}
						</code>
						<br />
						Aggregation rule: <code>equal-window@1</code>
					</div>
				</CardContent>
			</Card>
			<Card className="mt-4">
				<CardHeader>
					<CardTitle>2. Configure immutable evidence</CardTitle>
				</CardHeader>
				<CardContent className="space-y-4">
					<div className="grid gap-2">
						<p className="text-sm font-medium">Completed Backtest Run</p>
						{runs.map((item) => (
							<Button
								key={item.runId}
								type="button"
								variant={source?.runId === item.runId ? "default" : "outline"}
								className="h-auto justify-start whitespace-normal p-3 text-left"
								aria-pressed={source?.runId === item.runId}
								onClick={() => void selectRun(item.runId)}
							>
								<span>
									{item.code} · {item.interval} · {item.barCount} Bars · return{" "}
									{percent(item.totalReturn)}
								</span>
								<span className="block break-all font-mono text-xs opacity-75">
									Run {item.runId}
								</span>
							</Button>
						))}
						{runs.length === 0 && (
							<p className="text-sm text-muted-foreground">
								No completed Backtest Runs. Create one in Backtest first.
							</p>
						)}
					</div>
					{source && <ProtocolContext run={source} labels={labels} />}
					{method === "chronological-holdout" ? (
						<label
							className="grid max-w-sm gap-2 text-sm font-medium"
							htmlFor="sample-out-start"
						>
							Sample-out starts
							<Input
								id="sample-out-start"
								type="datetime-local"
								value={sampleOutStart}
								onChange={(event) => setSampleOutStart(event.target.value)}
							/>
						</label>
					) : (
						<WalkForwardControls
							windowSizeBars={windowSizeBars}
							stepSizeBars={stepSizeBars}
							minimumHistoryBars={minimumHistoryBars}
							onWindowSizeBarsChange={setWindowSizeBars}
							onStepSizeBarsChange={setStepSizeBars}
							onMinimumHistoryBarsChange={setMinimumHistoryBars}
							error={source ? walkForwardError : undefined}
							preview={walkForwardPreview}
							gaps={source?.snapshot.gaps ?? []}
						/>
					)}
					<Button
						disabled={
							!source ||
							(method === "chronological-holdout" && !sampleOutStart) ||
							(method === "walk-forward" && Boolean(walkForwardError))
						}
						onClick={() => void freeze()}
					>
						Freeze Validation Protocol
					</Button>
					{feedback && <Feedback feedback={feedback} />}
				</CardContent>
			</Card>
			<Card className="mt-4">
				<CardHeader>
					<CardTitle>3. Run or resume frozen Protocols</CardTitle>
				</CardHeader>
				<CardContent className="space-y-3">
					{protocols.map((protocol) => (
						<div
							key={protocol.protocolId}
							className="flex flex-wrap items-center justify-between gap-3 rounded-md border p-3 text-sm"
						>
							<div>
								<p>{protocolSummary(protocol)}</p>
								<code className="break-all text-xs">{protocol.protocolId}</code>
								<details className="mt-2">
									<summary>Review immutable Protocol</summary>
									{protocolDetails(protocol).map((window) => (
										<p key={`${window.snapshotId}:${window.boundary}`} className="mt-2">
											Snapshot <code className="break-all">{window.snapshotId}</code>
											<br />
											Sample-out boundary: {window.boundary}
											<br />
											Aggregation: <code>{window.aggregationRuleVersion}</code>
										</p>
									))}
									<pre className="mt-2 overflow-x-auto whitespace-pre-wrap text-xs">
										{JSON.stringify(protocol.run, null, 2)}
									</pre>
								</details>
							</div>
							<Button
								disabled={Boolean(runningProtocolId)}
								onClick={() => void run(protocol.protocolId)}
							>
								{runningProtocolId === protocol.protocolId
									? "Running…"
									: "Run / resume"}
							</Button>
						</div>
					))}
					{protocols.length === 0 && (
						<p className="text-sm text-muted-foreground">
							Freeze a Protocol to run it. Completed immutable Backtest Runs are
							reused.
						</p>
					)}
				</CardContent>
			</Card>
			<Card className="mt-4">
				<CardHeader>
					<CardTitle>4. Validation Reports</CardTitle>
				</CardHeader>
				<CardContent>
					{selectedReport ? (
						<>
							<div className="mb-3 flex flex-wrap gap-2">
								{reports.map((report) => (
									<Button
										key={report.reportId}
										size="sm"
										variant={
											report.reportId === selectedReport.reportId ? "default" : "outline"
										}
										onClick={() => setSelectedReportId(report.reportId)}
									>
										Report {report.reportId.slice(0, 12)}
									</Button>
								))}
							</div>
							<Tabs key={selectedReport.reportId} defaultValue="summary">
								<TabsList
									aria-label="Validation Report views"
									className="w-full justify-start overflow-x-auto"
								>
									<TabsTrigger value="summary">Summary</TabsTrigger>
									<TabsTrigger value="evidence">Evidence</TabsTrigger>
									<TabsTrigger value="provenance">Provenance</TabsTrigger>
								</TabsList>
								<ReportViews
									report={selectedReport}
									protocol={protocols.find(
										(item) => item.protocolId === selectedReport.protocolId,
									)}
									onExport={exportReport}
								/>
							</Tabs>
						</>
					) : (
						<p className="text-sm text-muted-foreground">
							No Validation Reports yet.
						</p>
					)}
				</CardContent>
			</Card>
		</Workspace>
	);
}

function WalkForwardControls({
	windowSizeBars,
	stepSizeBars,
	minimumHistoryBars,
	onWindowSizeBarsChange,
	onStepSizeBarsChange,
	onMinimumHistoryBarsChange,
	error,
	preview,
	gaps,
}: {
	windowSizeBars: string;
	stepSizeBars: string;
	minimumHistoryBars: string;
	onWindowSizeBarsChange: (value: string) => void;
	onStepSizeBarsChange: (value: string) => void;
	onMinimumHistoryBarsChange: (value: string) => void;
	error?: string;
	preview?: ReturnType<typeof previewWalkForward>;
	gaps: Array<{ startTimeMs: number; endTimeMs: number }>;
}) {
	return (
		<div className="space-y-3 rounded-md border p-3">
			<div className="grid gap-3 sm:grid-cols-3">
				<NumberControl
					id="sample-out-window"
					label="Sample-out window (Bars)"
					value={windowSizeBars}
					onChange={onWindowSizeBarsChange}
				/>
				<NumberControl
					id="walk-forward-step"
					label="Step (Bars)"
					value={stepSizeBars}
					onChange={onStepSizeBarsChange}
				/>
				<NumberControl
					id="walk-forward-minimum-history"
					label="Minimum history (Bars)"
					value={minimumHistoryBars}
					onChange={onMinimumHistoryBarsChange}
				/>
			</div>
			{error ? (
				<pre className="overflow-x-auto whitespace-pre-wrap text-xs" role="alert">
					{error}
				</pre>
			) : preview ? (
				<div className="space-y-1 text-sm" role="status">
					<p>
						{preview.windows.length} deterministic complete window
						{preview.windows.length === 1 ? "" : "s"} will freeze in chronological
						order.
					</p>
					<ol className="list-decimal pl-5 text-xs">
						{preview.windows.map((window) => {
							const gapCount = gapCountForWindow(window, gaps);
							return (
								<li
									key={`${window.sampleOutStartTimeMs}:${window.sampleOutEndTimeMs ?? "final"}`}
								>
									{new Date(window.sampleOutStartTimeMs).toLocaleString()} –{" "}
									{window.sampleOutEndTimeMs
										? new Date(window.sampleOutEndTimeMs).toLocaleString()
										: "final"}
									{gapCount > 0 && (
										<>
											{" "}
											· {gapCount} Bar Gap{gapCount === 1 ? "" : "s"} in frozen evidence
										</>
									)}
								</li>
							);
						})}
					</ol>
					{preview.partialFinalWindow && (
						<p>The partial final window is excluded; only complete windows freeze.</p>
					)}
				</div>
			) : null}
			{gaps.length > 0 && (
				<details className="text-sm">
					<summary>
						{gaps.length} Bar Gap{gaps.length === 1 ? "" : "s"} remain visible in Run
						evidence
					</summary>
					<ul className="mt-2 list-disc pl-5 text-xs">
						{gaps.map((gap) => (
							<li key={`${gap.startTimeMs}:${gap.endTimeMs}`}>
								{new Date(gap.startTimeMs).toLocaleString()} –{" "}
								{new Date(gap.endTimeMs).toLocaleString()}
							</li>
						))}
					</ul>
				</details>
			)}
		</div>
	);
}

function NumberControl({
	id,
	label,
	value,
	onChange,
}: {
	id: string;
	label: string;
	value: string;
	onChange: (value: string) => void;
}) {
	return (
		<label className="grid gap-2 text-sm font-medium" htmlFor={id}>
			{label}
			<Input
				id={id}
				type="number"
				min="1"
				step="1"
				inputMode="numeric"
				value={value}
				onChange={(event) => onChange(event.target.value)}
			/>
		</label>
	);
}

function gapCountForWindow(
	window: { sampleOutEndTimeMs?: number },
	gaps: Array<{ startTimeMs: number; endTimeMs: number }>,
) {
	const end = window.sampleOutEndTimeMs ?? Number.MAX_SAFE_INTEGER;
	return gaps.filter((gap) => gap.startTimeMs < end).length;
}

function ProtocolContext({
	run,
	labels,
}: {
	run: BacktestRun;
	labels: Map<string, string>;
}) {
	const provenance = run.provenance;
	if (!provenance)
		return (
			<p className="rounded-md border p-3 text-sm" role="alert">
				This legacy Run has incomplete provenance and cannot freeze a Protocol
				safely.
			</p>
		);
	const config = provenance.normalizedRequest;
	return (
		<div className="rounded-md border p-3 text-sm">
			<p className="font-medium">Frozen source context</p>
			<p>
				Snapshot: {run.snapshot.code} · {run.snapshot.interval} ·{" "}
				{run.snapshot.barCount} Bars
			</p>
			<code className="block break-all text-xs">{run.snapshot.snapshotId}</code>
			<p className="mt-2">
				Strategy: {labels.get(config.strategyArchiveSha256) ?? "Unknown package"}
			</p>
			<code className="block break-all text-xs">
				{config.strategyArchiveSha256}
			</code>
			{config.factorInstances.map((factor) => (
				<p key={factor.alias} className="mt-2">
					Factor {factor.alias}:{" "}
					{labels.get(factor.archiveSha256) ?? "Unknown package"}
					<code className="block break-all text-xs">{factor.archiveSha256}</code>
				</p>
			))}
			<p className="mt-2">
				Backtest Run <code className="break-all">{run.runId}</code>
			</p>
		</div>
	);
}

function ReportViews({
	report,
	protocol,
	onExport,
}: {
	report: Report;
	protocol?: Protocol;
	onExport: (reportId: string, format: "json" | "markdown") => void;
}) {
	return (
		<>
			<TabsContent value="summary" className="space-y-3">
				<div className="grid gap-3 text-sm sm:grid-cols-2 lg:grid-cols-4">
					<Metric
						label="Completed"
						value={String(report.aggregate.completedWindows)}
					/>
					<Metric label="Failed" value={String(report.aggregate.failedWindows)} />
					<Metric
						label="Average sample-out return"
						value={percent(report.aggregate.averageSampleOutReturn)}
					/>
					<Metric
						label="Average sample-in return"
						value={percent(report.aggregate.averageSampleInReturn)}
					/>
					<Metric
						label="Worst drawdown"
						value={percent(report.aggregate.worstSampleOutDrawdown)}
					/>
					<Metric
						label="Average Sharpe"
						value={formatDecimal(report.aggregate.averageSampleOutSharpe)}
					/>
					<Metric
						label="Total fees"
						value={formatDecimal(report.aggregate.totalFees)}
					/>
					<Metric label="Trades" value={String(report.aggregate.totalTrades)} />
				</div>
				{report.walkForward && (
					<p className="rounded-md border p-3 text-sm">
						Walk-forward consistency: {report.windows.length} ordered window
						{report.windows.length === 1 ? "" : "s"} · sample-out{" "}
						{report.walkForward.windowSizeBars} Bars · step{" "}
						{report.walkForward.stepSizeBars} Bars · minimum history{" "}
						{report.walkForward.minimumHistoryBars} Bars
					</p>
				)}
				<div className="flex gap-2">
					<Button
						variant="outline"
						onClick={() => onExport(report.reportId, "json")}
					>
						Export JSON
					</Button>
					<Button
						variant="outline"
						onClick={() => onExport(report.reportId, "markdown")}
					>
						Export Markdown
					</Button>
				</div>
			</TabsContent>
			<TabsContent value="evidence" className="space-y-2">
				{report.windows.map((window) => (
					<div
						key={`${window.sampleOutStartTimeMs}:${window.sampleOutEndTimeMs ?? "final"}`}
						className="rounded-md border p-3 text-sm"
					>
						<p>
							Sample-out: {new Date(window.sampleOutStartTimeMs).toLocaleString()} –{" "}
							{window.sampleOutEndTimeMs
								? new Date(window.sampleOutEndTimeMs).toLocaleString()
								: "final"}
						</p>
						{window.failure ? (
							<pre
								className="mt-2 overflow-x-auto whitespace-pre-wrap text-xs"
								role="alert"
							>
								Failure: {window.failure}
							</pre>
						) : (
							<p className="mt-2">
								Return {percent(window.sampleOutMetrics?.totalReturn ?? "0")} · drawdown{" "}
								{percent(window.sampleOutMetrics?.maxDrawdown ?? "0")} · Sharpe{" "}
								{formatDecimal(window.sampleOutMetrics?.sharpe ?? "0")}
							</p>
						)}
						<p className="mt-2">Run Pauses</p>
						{[...window.sampleInPauses, ...window.sampleOutPauses].length ? (
							<ul className="list-disc pl-5 text-xs">
								{[...window.sampleInPauses, ...window.sampleOutPauses].map((pause) => (
									<li key={`${pause.openTimeMs}:${pause.reason}`}>
										{new Date(pause.openTimeMs).toLocaleString()} · {pause.reason}
									</li>
								))}
							</ul>
						) : (
							<p className="text-muted-foreground">None</p>
						)}
						<p className="mt-2 text-xs">
							Sample-in Snapshot{" "}
							<code className="break-all">{window.sampleInSnapshotId}</code>
							<br />
							Sample-out Snapshot{" "}
							<code className="break-all">{window.sampleOutSnapshotId}</code>
							<br />
							<a className="underline" href="/backtest">
								Sample-in Run {window.sampleInRunId ?? "not completed"}
							</a>
							<br />
							<a className="underline" href="/backtest">
								Sample-out Run {window.sampleOutRunId ?? "not completed"}
							</a>
						</p>
					</div>
				))}
			</TabsContent>
			<TabsContent value="provenance">
				<div className="space-y-2 rounded-md border p-3 text-sm">
					<p>
						Method: <code>{report.methodVersion}</code> · aggregation:{" "}
						<code>{report.aggregationRuleVersion}</code>
					</p>
					<p>Protocol</p>
					<code className="block break-all text-xs">{report.protocolId}</code>
					{protocol ? (
						<pre className="overflow-x-auto whitespace-pre-wrap text-xs">
							{JSON.stringify(protocol, null, 2)}
						</pre>
					) : (
						<p className="text-muted-foreground">
							The Protocol is unavailable in this user-scoped list.
						</p>
					)}
					<p>Authoritative Report identity</p>
					<code className="block break-all text-xs">{report.reportId}</code>
				</div>
			</TabsContent>
		</>
	);
}

function Metric({ label, value }: { label: string; value: string }) {
	return (
		<div className="rounded-md border p-3">
			<p className="text-muted-foreground">{label}</p>
			<p className="font-medium">{value}</p>
		</div>
	);
}
function Feedback({
	feedback,
}: {
	feedback: { summary: string; details?: string };
}) {
	return (
		<div
			className="rounded-md border p-3 text-sm"
			role={feedback.details ? "alert" : "status"}
			aria-live="polite"
		>
			<p>{feedback.summary}</p>
			{feedback.details && (
				<pre className="mt-2 overflow-x-auto whitespace-pre-wrap text-xs">
					{feedback.details}
				</pre>
			)}
		</div>
	);
}
function percent(value: string) {
	return `${formatDecimal(value)}%`;
}
