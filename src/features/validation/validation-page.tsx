import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { LoadingState } from "@/components/loading-state";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
	Pagination,
	PaginationContent,
	PaginationItem,
	PaginationNext,
	PaginationPrevious,
} from "@/components/ui/pagination";
import type { BacktestRun } from "@/features/backtest/backtest-page";
import { formatDecimal } from "@/features/backtest/format-decimal";
import type { LibraryComponent } from "@/features/components/component-library";
import { Workspace } from "@/features/components/components-page";
import { ResearchMetric } from "@/features/research/metric-info";
import { useMarketSessionStore } from "@/lib/market-session";
import { useHistoryTab } from "@/lib/navigation-history";
import { invoke } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import { open } from "@tauri-apps/plugin-fs";
import { useTranslation } from "react-i18next";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import {
	formatValidationError,
	protocolDetails,
	protocolSummary,
	reportExportFilename,
} from "./validation-evidence";
import {
	createValidationProtocolDraft,
	inspectValidationProtocolDraft,
	transitionValidationProtocolDraft,
	type DraftCommand,
	type DraftError,
	type ValidationContext,
	type ValidationDraftSession,
	type ValidationPreviewFacts,
	type ValidationSnapshot,
	type WalkForwardPreview,
} from "./validation-protocol-draft";

type RunSummary = {
	runId: string;
	createdAt: string;
	snapshotId: string;
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
	crossMarket?: {
		contexts: Array<{
			snapshotId: string;
			runOverride?: Record<string, unknown>;
		}>;
	};
};
type Snapshot = ValidationSnapshot;
type CrossMarketContext = ValidationContext;
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
	crossMarket: Array<{
		snapshot: Snapshot;
		run: Record<string, unknown>;
		runId?: string;
		metrics?: Metrics;
		pauses: Array<{ openTimeMs: number; reason: string }>;
		failure?: string;
	}>;
	crossMarketEvidence?: { completedMarkets: number; totalReturnSpread: string };
	recommendedContexts: Array<{
		supportingReportId: string;
		snapshot: Snapshot;
		run: Record<string, unknown>;
	}>;
};
type Metrics = { totalReturn: string; maxDrawdown: string; sharpe: string };

const waitForPaint = () =>
	new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
const RUN_HISTORY_PAGE_SIZE = 10;

export function ValidationPage() {
	const { t } = useTranslation();
	const userId = useMarketSessionStore((state) => state.userId);
	const [runs, setRuns] = useState<RunSummary[]>([]);
	const [runsPage, setRunsPage] = useState(1);
	const [runsTotal, setRunsTotal] = useState(0);
	const [components, setComponents] = useState<LibraryComponent[]>([]);
	const [source, setSource] = useState<BacktestRun>();
	const [draftSession, setDraftSession] = useState<ValidationDraftSession>(() =>
		createValidationProtocolDraft(),
	);
	const draftSessionRef = useRef(draftSession);
	const [snapshots, setSnapshots] = useState<Snapshot[]>([]);
	const [protocols, setProtocols] = useState<Protocol[]>([]);
	const [reports, setReports] = useState<Report[]>([]);
	const [selectedReportId, setSelectedReportId] = useState("");
	const [reportTab, setReportTab] = useHistoryTab(
		"validation-report",
		"summary",
		selectedReportId || undefined,
	);
	const [loadingRunId, setLoadingRunId] = useState<string>();
	const [runningProtocolId, setRunningProtocolId] = useState<string>();
	const [exportingReport, setExportingReport] = useState<string>();
	const [runsLoading, setRunsLoading] = useState(true);
	const [protocolsLoading, setProtocolsLoading] = useState(true);
	const [reportsLoading, setReportsLoading] = useState(true);
	const [snapshotsLoading, setSnapshotsLoading] = useState(true);
	const [feedback, setFeedback] = useState<{
		summary: string;
		details?: string;
	}>();
	const applyDraftCommand = useCallback(
		(command: DraftCommand, showError = true) => {
			const result = transitionValidationProtocolDraft(
				draftSessionRef.current,
				command,
			);
			if (!result.ok) {
				if (showError)
					setFeedback(
						formatDraftError(result.error, t("loading.exactValidationEvidence")),
					);
				return result;
			}
			draftSessionRef.current = result.value.session;
			setDraftSession(result.value.session);
			return result;
		},
		[t],
	);
	const refreshRuns = useCallback(
		async (page: number, isActive: () => boolean = () => true) => {
			if (!userId) return;
			setRunsLoading(true);
			try {
				const result = await invoke<RunHistoryPage>("backtest_list", {
					request: { userId, page },
				});
				if (!isActive()) return;
				setRuns(result.items);
				setRunsTotal(result.total);
			} catch (error) {
				if (isActive())
					setFeedback({
						summary: "Completed Backtest Runs could not load.",
						details: String(error),
					});
			} finally {
				if (isActive()) setRunsLoading(false);
			}
		},
		[userId],
	);

	const refresh = useCallback(async () => {
		if (!userId) return;
		setProtocolsLoading(true);
		setReportsLoading(true);
		setSnapshotsLoading(true);
		await Promise.all([
			invoke<LibraryComponent[]>("component_list", { request: { userId } }).then(
				setComponents,
			),
			invoke<Protocol[]>("validation_protocol_list", { request: { userId } })
				.then(setProtocols)
				.finally(() => setProtocolsLoading(false)),
			invoke<Report[]>("validation_report_list", { request: { userId } })
				.then((nextReports) => {
					setReports(nextReports);
					setSelectedReportId((current) =>
						nextReports.some((report) => report.reportId === current)
							? current
							: (nextReports[0]?.reportId ?? ""),
					);
				})
				.finally(() => setReportsLoading(false)),
			invoke<Snapshot[]>("snapshot_list_readable", { request: { userId } })
				.then(setSnapshots)
				.finally(() => setSnapshotsLoading(false)),
		]);
	}, [userId]);
	useEffect(() => {
		if (!userId) return;
		let active = true;
		void refreshRuns(runsPage, () => active);
		return () => {
			active = false;
		};
	}, [refreshRuns, runsPage, userId]);
	useEffect(() => {
		if (!userId) return;
		let active = true;
		setSource(undefined);
		const nextDraft = createValidationProtocolDraft();
		draftSessionRef.current = nextDraft;
		setDraftSession(nextDraft);
		setFeedback(undefined);
		void refresh().catch(
			(error) =>
				active &&
				setFeedback({
					summary: "Validation evidence could not load.",
					details: String(error),
				}),
		);
		return () => {
			active = false;
		};
	}, [refresh, userId]);
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
	const method = draftSession.draft.kind;
	const sourceMatchesDraft = Boolean(
		source && draftSession.draft.source?.runId === source.runId,
	);
	const previewFacts: ValidationPreviewFacts | undefined = sourceMatchesDraft
		? { sourceRunId: source?.runId ?? "", bars: source?.bars ?? [] }
		: undefined;
	const inspection = inspectValidationProtocolDraft(draftSession, previewFacts);
	const draftError = inspection.errors[0];
	const draftErrorMessage = draftError
		? formatDraftError(draftError, t("loading.exactValidationEvidence")).summary
		: undefined;
	const walkForwardPreview: WalkForwardPreview | undefined = inspection.preview;
	const walkForwardDraft =
		draftSession.draft.kind === "walk-forward" ? draftSession.draft : undefined;
	const walkForwardError =
		method === "walk-forward" && source ? draftErrorMessage : undefined;
	const crossMarketError =
		method === "cross-market" && source ? draftErrorMessage : undefined;
	const crossMarketContexts: readonly CrossMarketContext[] =
		method === "cross-market" ? draftSession.draft.contexts : [];
	const freezing = draftSession.freeze.status === "pending";
	const selectedSourceRunId =
		draftSession.sourceLoad.status === "pending"
			? draftSession.sourceLoad.runId
			: draftSession.draft.source?.runId;
	const loadOverride = async (snapshotId: string, runId: string) => {
		if (!userId) return;
		const requested = applyDraftCommand({
			type: "request-cross-market-override",
			snapshotId,
			runId,
		});
		if (!requested.ok || !requested.value.effect) return;
		if (requested.value.effect.kind !== "load-cross-market-override") return;
		const { revision } = requested.value.effect;
		try {
			const run = await invoke<BacktestRun>("backtest_get", {
				request: { userId, runId },
			});
			const accepted = applyDraftCommand({
				type: "accept-cross-market-override",
				revision,
				snapshotId,
				run,
			});
			if (!accepted.ok) {
				applyDraftCommand(
					{ type: "reject-cross-market-override", revision, snapshotId, runId },
					false,
				);
				return;
			}
			if (accepted.value.ignored) return;
		} catch (error) {
			const rejected = applyDraftCommand(
				{ type: "reject-cross-market-override", revision, snapshotId, runId },
				false,
			);
			if (rejected.ok && !rejected.value.ignored) {
				setFeedback({
					summary: "Override Run could not load.",
					details: String(error),
				});
			}
		}
	};
	const selectRun = async (runId: string) => {
		if (!userId) return;
		const selected = applyDraftCommand({ type: "select-source", runId });
		if (!selected.ok || !selected.value.effect) return;
		if (selected.value.effect.kind !== "load-source") return;
		const { revision } = selected.value.effect;
		setSource(undefined);
		setLoadingRunId(runId);
		setFeedback(undefined);
		try {
			const run = await invoke<BacktestRun>("backtest_get", {
				request: { userId, runId },
			});
			const accepted = applyDraftCommand({ type: "accept-source", revision, run });
			if (!accepted.ok) {
				applyDraftCommand({ type: "reject-source", revision, runId }, false);
			} else if (!accepted.value.ignored) {
				setSource(run);
			}
		} catch (error) {
			const rejected = applyDraftCommand(
				{ type: "reject-source", revision, runId },
				false,
			);
			if (rejected.ok && !rejected.value.ignored) {
				setFeedback({
					summary: "Backtest Run could not load.",
					details: String(error),
				});
			}
		} finally {
			setLoadingRunId((current) => (current === runId ? undefined : current));
		}
	};
	const freeze = async () => {
		if (freezing) return;
		if (!userId) return;
		const requested = applyDraftCommand({
			type: "request-freeze",
			userId,
			previewFacts,
		});
		if (!requested.ok || !requested.value.effect) return;
		if (requested.value.effect.kind !== "freeze") return;
		const { revision, request } = requested.value.effect;
		setFeedback({ summary: "Freezing Validation Protocol…" });
		await waitForPaint();
		try {
			const protocol = await invoke<Protocol>("validation_protocol_create", {
				request,
			});
			const accepted = applyDraftCommand(
				{ type: "accept-freeze", revision, protocolId: protocol.protocolId },
				false,
			);
			if (!accepted.ok || accepted.value.ignored) return;
			setFeedback({
				summary: `Protocol ${protocol.protocolId.slice(0, 16)} frozen and immutable.`,
			});
			await refresh();
		} catch (error) {
			const rejected = applyDraftCommand(
				{ type: "reject-freeze", revision },
				false,
			);
			if (rejected.ok && !rejected.value.ignored) {
				setFeedback(formatValidationError(error));
			}
		}
	};
	const run = async (protocolId: string) => {
		if (!userId || runningProtocolId) return;
		setRunningProtocolId(protocolId);
		setFeedback({ summary: "Running Validation Protocol…" });
		await waitForPaint();
		try {
			const report = await invoke<Report>("validation_report_run", {
				request: { userId, protocolId },
			});
			setFeedback({
				summary: `Validation Report ${report.reportId.slice(0, 16)} completed.`,
			});
			await Promise.all([refresh(), refreshRuns(runsPage)]);
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
		const exportKey = `${reportId}:${format}`;
		setExportingReport(exportKey);
		try {
			const content = await invoke<string>("validation_report_export", {
				request: { userId, protocolId: reportId },
				format,
			});
			const path = await save({
				defaultPath: reportExportFilename(reportId, format),
				filters: [
					{
						name: format === "json" ? "JSON" : "Markdown",
						extensions: [format === "json" ? "json" : "md"],
					},
				],
			});
			if (!path) return;
			const file = await open(path, { write: true, createNew: true });
			try {
				await file.write(new TextEncoder().encode(content));
			} finally {
				await file.close();
			}
			toast.success(`${format === "json" ? "JSON" : "Markdown"} report exported`, {
				description: path,
			});
		} catch (error) {
			setFeedback({ summary: "Report export failed.", details: String(error) });
		} finally {
			setExportingReport(undefined);
		}
	};
	return (
		<Workspace
			title="Validation"
			description="Immutable chronological holdout, walk-forward, and cross-market research evidence."
		>
			<Card>
				<CardHeader>
					<CardTitle>1. Choose validation method</CardTitle>
				</CardHeader>
				<CardContent className="space-y-3">
					<div
						className="grid gap-2 sm:grid-cols-3"
						role="radiogroup"
						aria-label="Validation method"
					>
						<label className="rounded-md border p-3 text-sm">
							<input
								type="radio"
								name="validation-method"
								checked={method === "chronological-holdout"}
								onChange={() =>
									applyDraftCommand({
										type: "select-method",
										method: "chronological-holdout",
									})
								}
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
								checked={method === "cross-market"}
								onChange={() =>
									applyDraftCommand({ type: "select-method", method: "cross-market" })
								}
							/>{" "}
							<span className="font-medium">Cross-market</span>
							<p className="mt-1 text-muted-foreground">
								Compare ordered immutable market contexts without implying future
								profitability.
							</p>
						</label>
						<label className="rounded-md border p-3 text-sm">
							<input
								type="radio"
								name="validation-method"
								checked={method === "walk-forward"}
								onChange={() =>
									applyDraftCommand({ type: "select-method", method: "walk-forward" })
								}
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
								: method === "walk-forward"
									? "walk-forward@1"
									: "cross-market@1"}
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
						{runsLoading ? (
							<LoadingState labelKey="loading.completedRuns" />
						) : (
							runs.map((item) => (
								<div
									key={item.runId}
									className="grid items-center gap-2 rounded-md border p-2 sm:grid-cols-[minmax(0,1fr)_auto]"
								>
									<Button
										type="button"
										variant={selectedSourceRunId === item.runId ? "default" : "outline"}
										className="h-auto justify-start whitespace-normal p-3 text-left"
										aria-pressed={selectedSourceRunId === item.runId}
										loading={loadingRunId === item.runId}
										loadingText={t("loading.loadingRun")}
										disabled={Boolean(loadingRunId)}
										onClick={() => void selectRun(item.runId)}
									>
										<span>
											{item.code} · {item.interval} · {item.barCount} Bars
										</span>
										<span className="block break-all font-mono text-xs opacity-75">
											Run {item.runId}
										</span>
									</Button>
									<ResearchMetric
										metricId="strategy.total-return"
										value={percent(item.totalReturn)}
									/>
								</div>
							))
						)}
						{!runsLoading && runs.length === 0 && (
							<p className="text-sm text-muted-foreground">
								No completed Backtest Runs. Create one in Backtest first.
							</p>
						)}
						{!runsLoading && runsTotal > RUN_HISTORY_PAGE_SIZE && (
							<Pagination>
								<PaginationContent>
									<PaginationItem>
										<PaginationPrevious
											disabled={runsPage === 1}
											onClick={() => setRunsPage((page) => page - 1)}
										/>
									</PaginationItem>
									<PaginationItem>
										<span className="px-3 text-sm" aria-current="page">
											Page {runsPage} of {Math.ceil(runsTotal / RUN_HISTORY_PAGE_SIZE)}
										</span>
									</PaginationItem>
									<PaginationItem>
										<PaginationNext
											disabled={runsPage >= Math.ceil(runsTotal / RUN_HISTORY_PAGE_SIZE)}
											onClick={() => setRunsPage((page) => page + 1)}
										/>
									</PaginationItem>
								</PaginationContent>
							</Pagination>
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
								value={
									draftSession.draft.kind === "chronological-holdout"
										? draftSession.draft.sampleOutStart
										: ""
								}
								onChange={(event) =>
									applyDraftCommand({
										type: "set-holdout-boundary",
										value: event.target.value,
									})
								}
							/>
						</label>
					) : method === "walk-forward" ? (
						<WalkForwardControls
							windowSizeBars={walkForwardDraft?.windowSizeBars ?? ""}
							stepSizeBars={walkForwardDraft?.stepSizeBars ?? ""}
							minimumHistoryBars={walkForwardDraft?.minimumHistoryBars ?? ""}
							onWindowSizeBarsChange={(value) =>
								applyDraftCommand({
									type: "set-walk-forward-field",
									field: "windowSizeBars",
									value,
								})
							}
							onStepSizeBarsChange={(value) =>
								applyDraftCommand({
									type: "set-walk-forward-field",
									field: "stepSizeBars",
									value,
								})
							}
							onMinimumHistoryBarsChange={(value) =>
								applyDraftCommand({
									type: "set-walk-forward-field",
									field: "minimumHistoryBars",
									value,
								})
							}
							error={source ? walkForwardError : undefined}
							preview={walkForwardPreview}
							gaps={source?.snapshot.gaps ?? []}
						/>
					) : (
						<CrossMarketControls
							snapshots={snapshots}
							contexts={crossMarketContexts}
							runs={runs}
							loading={snapshotsLoading}
							error={source ? crossMarketError : undefined}
							onChange={applyDraftCommand}
							onLoadOverride={loadOverride}
						/>
					)}
					<Button
						loading={freezing}
						loadingText={t("loading.freezing")}
						disabled={freezing || !sourceMatchesDraft || Boolean(draftError)}
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
					{protocolsLoading ? (
						<LoadingState labelKey="loading.protocols" />
					) : (
						protocols.map((protocol) => (
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
										{protocol.crossMarket?.contexts.map((context, index) => (
											<p key={context.snapshotId} className="mt-2">
												Market context {index + 1}:{" "}
												<code className="break-all">{context.snapshotId}</code>
												<br />
												Configuration: {context.runOverride ? "exact override" : "shared"}
											</p>
										))}
										<pre className="mt-2 overflow-x-auto whitespace-pre-wrap text-xs">
											{JSON.stringify(protocol.run, null, 2)}
										</pre>
									</details>
								</div>
								<Button
									loading={runningProtocolId === protocol.protocolId}
									loadingText={t("loading.running")}
									disabled={Boolean(runningProtocolId)}
									onClick={() => void run(protocol.protocolId)}
								>
									Run / resume
								</Button>
							</div>
						))
					)}
					{!protocolsLoading && protocols.length === 0 && (
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
					{reportsLoading ? (
						<LoadingState labelKey="loading.reports" />
					) : selectedReport ? (
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
							<Tabs
								key={selectedReport.reportId}
								value={reportTab}
								onValueChange={setReportTab}
							>
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
									exportingReport={exportingReport}
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
	preview?: WalkForwardPreview;
	gaps: ReadonlyArray<{ startTimeMs: number; endTimeMs: number }>;
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

function CrossMarketControls({
	snapshots,
	contexts,
	runs,
	loading,
	error,
	onChange,
	onLoadOverride,
}: {
	snapshots: Snapshot[];
	contexts: readonly CrossMarketContext[];
	runs: RunSummary[];
	loading: boolean;
	error?: string;
	onChange: (command: DraftCommand) => void;
	onLoadOverride: (snapshotId: string, runId: string) => Promise<void>;
}) {
	const selected = new Set(
		contexts.map((context) => context.snapshot.snapshotId),
	);
	return (
		<div className="space-y-3 rounded-md border p-3">
			<p className="text-sm font-medium">Ordered market contexts</p>
			<p className="text-sm text-muted-foreground" role="status">
				Select at least two readable Snapshots. The selected completed Run supplies
				the shared configuration; an exact Run on the same Snapshot may override it.
			</p>
			{loading && <LoadingState labelKey="loading.readableSnapshots" />}
			<div className="grid gap-2 sm:grid-cols-2">
				{!loading &&
					snapshots.map((snapshot) => (
						<Button
							key={snapshot.snapshotId}
							type="button"
							variant={selected.has(snapshot.snapshotId) ? "default" : "outline"}
							className="h-auto justify-start whitespace-normal p-3 text-left"
							aria-pressed={selected.has(snapshot.snapshotId)}
							disabled={selected.has(snapshot.snapshotId)}
							onClick={() => onChange({ type: "add-cross-market-context", snapshot })}
						>
							<span>
								{snapshot.code} · {snapshot.interval} · {snapshot.barCount} Bars
							</span>
							<span className="block break-all font-mono text-xs opacity-75">
								Snapshot {snapshot.snapshotId}
							</span>
						</Button>
					))}
			</div>
			{!loading && snapshots.length === 0 && (
				<p className="text-sm text-muted-foreground">
					No readable Snapshots are available.
				</p>
			)}
			<ol className="space-y-2" aria-label="Selected cross-market contexts">
				{contexts.map((context, index) => (
					<li
						key={context.snapshot.snapshotId}
						className="rounded-md border p-3 text-sm"
					>
						<p>
							{index + 1}. {context.snapshot.code} · {context.snapshot.interval} ·{" "}
							{context.snapshot.barCount} Bars
						</p>
						<code className="block break-all text-xs">
							{context.snapshot.snapshotId}
						</code>
						<div className="mt-2 flex flex-wrap gap-2">
							<Button
								type="button"
								variant="outline"
								size="sm"
								disabled={index === 0}
								onClick={() =>
									onChange({
										type: "move-cross-market-context",
										snapshotId: context.snapshot.snapshotId,
										direction: "earlier",
									})
								}
							>
								Move earlier
							</Button>
							<Button
								type="button"
								variant="outline"
								size="sm"
								disabled={index === contexts.length - 1}
								onClick={() =>
									onChange({
										type: "move-cross-market-context",
										snapshotId: context.snapshot.snapshotId,
										direction: "later",
									})
								}
							>
								Move later
							</Button>
							<Button
								type="button"
								variant="outline"
								size="sm"
								onClick={() =>
									onChange({
										type: "remove-cross-market-context",
										snapshotId: context.snapshot.snapshotId,
									})
								}
							>
								Remove
							</Button>
							<label className="text-xs">
								Override configuration
								<select
									className="ml-2 rounded border p-1"
									value={context.override?.runId ?? ""}
									onChange={(event) => {
										if (!event.target.value) {
											onChange({
												type: "clear-cross-market-override",
												snapshotId: context.snapshot.snapshotId,
											});
											return;
										}
										void onLoadOverride(context.snapshot.snapshotId, event.target.value);
									}}
								>
									<option value="">Shared selected Run</option>
									{runs
										.filter((run) => run.snapshotId === context.snapshot.snapshotId)
										.map((run) => (
											<option key={run.runId} value={run.runId}>
												{run.code} · {run.interval} · {run.runId.slice(0, 12)}
											</option>
										))}
								</select>
							</label>
						</div>
					</li>
				))}
			</ol>
			{error && (
				<pre className="overflow-x-auto whitespace-pre-wrap text-xs" role="alert">
					{error}
				</pre>
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
	gaps: ReadonlyArray<{ startTimeMs: number; endTimeMs: number }>,
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
	exportingReport,
}: {
	report: Report;
	protocol?: Protocol;
	onExport: (reportId: string, format: "json" | "markdown") => void;
	exportingReport?: string;
}) {
	const { t } = useTranslation();
	return (
		<>
			<TabsContent value="summary" className="space-y-3">
				<div className="grid gap-3 text-sm sm:grid-cols-2 lg:grid-cols-4">
					<ResearchMetric
						metricId="validation.completed"
						value={String(report.aggregate.completedWindows)}
						className="rounded-md border p-3"
					/>
					<ResearchMetric
						metricId="validation.failed"
						value={String(report.aggregate.failedWindows)}
						className="rounded-md border p-3"
					/>
					<ResearchMetric
						metricId="validation.average-sample-out-return"
						value={percent(report.aggregate.averageSampleOutReturn)}
						className="rounded-md border p-3"
					/>
					<ResearchMetric
						metricId="validation.average-sample-in-return"
						value={percent(report.aggregate.averageSampleInReturn)}
						className="rounded-md border p-3"
					/>
					<ResearchMetric
						metricId="validation.worst-sample-out-drawdown"
						value={percent(report.aggregate.worstSampleOutDrawdown)}
						className="rounded-md border p-3"
					/>
					<ResearchMetric
						metricId="validation.average-sample-out-sharpe"
						value={formatDecimal(report.aggregate.averageSampleOutSharpe)}
						className="rounded-md border p-3"
					/>
					<ResearchMetric
						metricId="validation.total-fees"
						value={formatDecimal(report.aggregate.totalFees)}
						className="rounded-md border p-3"
					/>
					<ResearchMetric
						metricId="validation.realized-trade-count"
						value={String(report.aggregate.totalTrades)}
						className="rounded-md border p-3"
					/>
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
				{report.crossMarketEvidence && (
					<div
						className="grid gap-3 rounded-md border p-3 text-sm sm:grid-cols-2"
						role="status"
					>
						<ResearchMetric
							metricId="validation.completed"
							value={String(report.crossMarketEvidence.completedMarkets)}
						/>
						<ResearchMetric
							metricId="validation.cross-market-return-spread"
							value={percent(report.crossMarketEvidence.totalReturnSpread)}
						/>
						<p className="text-muted-foreground sm:col-span-2">
							Historical cross-market evidence only; it is not a profitability
							guarantee.
						</p>
					</div>
				)}
				<div className="flex gap-2">
					<Button
						variant="outline"
						loading={exportingReport === `${report.reportId}:json`}
						loadingText={t("loading.exportingJson")}
						disabled={Boolean(exportingReport)}
						onClick={() => onExport(report.reportId, "json")}
					>
						Export JSON
					</Button>
					<Button
						variant="outline"
						loading={exportingReport === `${report.reportId}:markdown`}
						loadingText={t("loading.exportingMarkdown")}
						disabled={Boolean(exportingReport)}
						onClick={() => onExport(report.reportId, "markdown")}
					>
						Export Markdown
					</Button>
				</div>
			</TabsContent>
			<TabsContent value="evidence" className="space-y-2">
				{report.crossMarket.map((context) => (
					<div
						key={context.snapshot.snapshotId}
						className="rounded-md border p-3 text-sm"
					>
						<p>
							{context.snapshot.code} · {context.snapshot.interval} ·{" "}
							{context.snapshot.barCount} Bars
						</p>
						<code className="block break-all text-xs">
							Snapshot {context.snapshot.snapshotId}
						</code>
						{context.failure ? (
							<pre
								className="mt-2 overflow-x-auto whitespace-pre-wrap text-xs"
								role="alert"
							>
								Failure: {context.failure}
							</pre>
						) : (
							<StrategyEvidenceMetrics metrics={context.metrics} />
						)}
						<p className="mt-2">Run Pauses</p>
						{context.pauses.length ? (
							<ul className="list-disc pl-5 text-xs">
								{context.pauses.map((pause) => (
									<li key={`${pause.openTimeMs}:${pause.reason}`}>
										{new Date(pause.openTimeMs).toLocaleString()} · {pause.reason}
									</li>
								))}
							</ul>
						) : (
							<p className="text-muted-foreground">None</p>
						)}
						<p className="mt-2 text-xs">
							<a
								className="underline"
								href={context.runId ? `/backtest?runId=${context.runId}` : "/backtest"}
							>
								Run {context.runId ?? "not completed"}
							</a>
						</p>
					</div>
				))}
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
							<StrategyEvidenceMetrics metrics={window.sampleOutMetrics} />
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
					{report.recommendedContexts.length > 0 && (
						<div>
							<p className="mt-2">Recommended Contexts</p>
							<p className="text-xs text-muted-foreground">
								Historical evidence references, not best-market or future-profitability
								claims.
							</p>
							{report.recommendedContexts.map((context) => (
								<div key={context.snapshot.snapshotId} className="mt-2 text-xs">
									{context.snapshot.code} · {context.snapshot.interval}
									<code className="block break-all">
										Report {context.supportingReportId}
									</code>
									<pre className="overflow-x-auto whitespace-pre-wrap">
										{JSON.stringify(context.run, null, 2)}
									</pre>
								</div>
							))}
						</div>
					)}
				</div>
			</TabsContent>
		</>
	);
}

function StrategyEvidenceMetrics({ metrics }: { metrics?: Metrics }) {
	return (
		<div className="mt-2 grid gap-2 sm:grid-cols-3">
			<ResearchMetric
				metricId="strategy.total-return"
				value={percent(metrics?.totalReturn ?? "0")}
			/>
			<ResearchMetric
				metricId="strategy.max-drawdown"
				value={percent(metrics?.maxDrawdown ?? "0")}
			/>
			<ResearchMetric
				metricId="strategy.sharpe"
				value={formatDecimal(metrics?.sharpe ?? "0")}
			/>
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

function formatDraftError(error: DraftError, exactEvidenceLoading: string) {
	switch (error.kind) {
		case "incomplete-draft":
			return { summary: `Complete: ${error.fields.join(", ")}.` };
		case "invalid-value":
			if (error.reason === "not-a-date") {
				return { summary: "Choose a valid sample-out boundary." };
			}
			if (error.reason === "outside-source") {
				return {
					summary: "The sample-out boundary must be inside the source Snapshot.",
				};
			}
			if (error.reason === "not-enough-history") {
				return {
					summary: "Walk-forward history cannot produce a complete window.",
				};
			}
			return { summary: "Walk-forward window sizes must be positive integers." };
		case "incompatible-selection":
			return { summary: "Selected validation evidence is incompatible." };
		case "incomplete-provenance":
			return { summary: "This Backtest Run has incomplete provenance." };
		case "source-loading":
			return { summary: exactEvidenceLoading };
	}
}
