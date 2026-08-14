import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { LoadingState } from "@/components/loading-state";
import type { LibraryComponent } from "@/features/components/component-library";
import { ResearchMetric } from "@/features/research/metric-info";
import { useMarketSessionStore } from "@/lib/market-session";
import { useHistoryTab } from "@/lib/navigation-history";
import { invoke } from "@tauri-apps/api/core";
import { open as chooseFile, save } from "@tauri-apps/plugin-dialog";
import { open, readFile } from "@tauri-apps/plugin-fs";
import { useTranslation } from "react-i18next";
import { PythonProjectsPanel } from "@/features/python-research/python-projects-panel";
import { PythonModelLabPanel } from "@/features/python-research/python-model-lab-panel";
import { useCallback, useEffect, useState } from "react";
import {
	datasetGenerationRequest,
	datasetStatusSummary,
	evaluationMetricKind,
	evaluationExportFilename,
	evaluationReportSummary,
	formatModelError,
	isCompatibleEvaluationSignal,
	signalRowPageRequest,
	signalRowSummary,
	type EvaluationSignalContract,
} from "./models-workspace";

type Snapshot = {
	snapshotId: string;
	code: string;
	interval: string;
	barCount: number;
};
type Dataset = {
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
type Attempt = {
	attemptId: string;
	datasetId?: string;
	status: "pending" | "running" | "completed" | "failed" | "cancelled";
	diagnosticEvidence?: string;
	progressCompleted: number;
	progressTotal: number;
};
type RowPage = {
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
type ModelOutput = EvaluationSignalContract;
type EvaluationReport = {
	reportId: string;
	datasetId: string;
	snapshotId: string;
	signalName: string;
	signalContract: ModelOutput;
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

const metricValue = (value?: number) =>
	value == null ? "Unavailable" : String(value);

const datasetOutputs = (dataset?: Dataset): ModelOutput[] => {
	if (!dataset) return [];
	if (dataset.modelOutputs.length) return dataset.modelOutputs as ModelOutput[];
	if (!dataset.archiveManifestJson) return [];
	return (JSON.parse(dataset.archiveManifestJson).signalContract?.outputs ??
		[]) as ModelOutput[];
};

const datasetEvaluationBounds = (dataset?: Dataset) => {
	const segments =
		dataset?.externalProducerSegments ?? dataset?.producerSegments ?? [];
	return {
		start: Number(segments[0]?.startPredictionTimeMs ?? 0),
		end: Number(segments.at(-1)?.endPredictionTimeMs ?? 0),
	};
};

const afterPaint = () =>
	new Promise<void>((resolve) =>
		requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
	);

export function ModelsPage() {
	const { t } = useTranslation();
	const userId = useMarketSessionStore((state) => state.userId);
	const [models, setModels] = useState<LibraryComponent[]>([]);
	const [components, setComponents] = useState<LibraryComponent[]>([]);
	const [snapshots, setSnapshots] = useState<Snapshot[]>([]);
	const [datasets, setDatasets] = useState<Dataset[]>([]);
	const [evaluationReports, setEvaluationReports] = useState<EvaluationReport[]>(
		[],
	);
	const [datasetRows, setDatasetRows] = useState<Record<string, RowPage>>({});
	const [rowsLoading, setRowsLoading] = useState("");
	const [componentsLoading, setComponentsLoading] = useState(true);
	const [snapshotsLoading, setSnapshotsLoading] = useState(true);
	const [attemptsLoading, setAttemptsLoading] = useState(true);
	const [datasetsLoading, setDatasetsLoading] = useState(false);
	const [evaluationsLoading, setEvaluationsLoading] = useState(false);
	const [attempts, setAttempts] = useState<Attempt[]>([]);
	const [compatibleFactors, setCompatibleFactors] = useState<
		Record<string, string[]>
	>({});
	const [model, setModel] = useState("");
	const [snapshot, setSnapshot] = useState("");
	const [modelParameters, setModelParameters] = useState<Record<string, string>>(
		{},
	);
	const [busy, setBusy] = useState(false);
	const [evidence, setEvidence] = useState("");
	const [activeAttempt, setActiveAttempt] = useState("");
	const [evaluationDataset, setEvaluationDataset] = useState("");
	const [evaluationSignal, setEvaluationSignal] = useState("");
	const [evaluationStart, setEvaluationStart] = useState(0);
	const [evaluationEnd, setEvaluationEnd] = useState(0);
	const [stabilityWindowBars, setStabilityWindowBars] = useState(20);
	const [tab, setTab] = useHistoryTab("models", "create");
	const refreshComponents = useCallback(
		async (isActive: () => boolean = () => true) => {
			if (!userId) return;
			setComponentsLoading(true);
			await afterPaint();
			if (!isActive()) return;
			try {
				const items = await invoke<LibraryComponent[]>("component_list", {
					request: { userId },
				});
				if (!isActive()) return;
				setModels(items.filter((item) => item.kind === "model"));
				setComponents(items);
				setModel(
					(current) =>
						current ||
						items.find((item) => item.kind === "model")?.archiveSha256 ||
						"",
				);
			} finally {
				if (isActive()) setComponentsLoading(false);
			}
		},
		[userId],
	);
	const refreshSnapshots = useCallback(
		async (isActive: () => boolean = () => true) => {
			if (!userId) return;
			setSnapshotsLoading(true);
			await afterPaint();
			if (!isActive()) return;
			try {
				const items = await invoke<Snapshot[]>("snapshot_list_readable", {
					request: { userId },
				});
				if (!isActive()) return;
				setSnapshots(items);
				setSnapshot((current) => current || items[0]?.snapshotId || "");
			} finally {
				if (isActive()) setSnapshotsLoading(false);
			}
		},
		[userId],
	);
	const refreshAttempts = useCallback(
		async (isActive: () => boolean = () => true) => {
			if (!userId) return;
			setAttemptsLoading(true);
			await afterPaint();
			if (!isActive()) return;
			try {
				const items = await invoke<Attempt[]>("dataset_generation_list", {
					userId,
				});
				if (isActive()) setAttempts(items);
			} finally {
				if (isActive()) setAttemptsLoading(false);
			}
		},
		[userId],
	);
	const refreshDatasets = useCallback(
		async (isActive: () => boolean = () => true) => {
			if (!userId) return;
			setDatasetsLoading(true);
			await afterPaint();
			if (!isActive()) return;
			try {
				const items = await invoke<Dataset[]>("signal_dataset_list", { userId });
				if (isActive()) setDatasets(items);
			} finally {
				if (isActive()) setDatasetsLoading(false);
			}
		},
		[userId],
	);
	const refreshEvaluations = useCallback(
		async (isActive: () => boolean = () => true) => {
			if (!userId) return;
			setEvaluationsLoading(true);
			await afterPaint();
			if (!isActive()) return;
			try {
				const items = await invoke<EvaluationReport[]>("forecast_evaluation_list", {
					userId,
				});
				if (isActive()) setEvaluationReports(items);
			} finally {
				if (isActive()) setEvaluationsLoading(false);
			}
		},
		[userId],
	);
	const refresh = useCallback(
		(isActive: () => boolean = () => true) =>
			Promise.all([
				refreshComponents(isActive),
				refreshSnapshots(isActive),
				refreshAttempts(isActive),
			]),
		[refreshAttempts, refreshComponents, refreshSnapshots],
	);
	useEffect(() => {
		let active = true;
		void refreshComponents(() => active).catch(
			(error) => active && setEvidence(formatModelError(error)),
		);
		return () => {
			active = false;
		};
	}, [refreshComponents]);
	useEffect(() => {
		let active = true;
		void refreshSnapshots(() => active).catch(
			(error) => active && setEvidence(formatModelError(error)),
		);
		return () => {
			active = false;
		};
	}, [refreshSnapshots]);
	useEffect(() => {
		let active = true;
		void refreshAttempts(() => active).catch(
			(error) => active && setEvidence(formatModelError(error)),
		);
		return () => {
			active = false;
		};
	}, [refreshAttempts]);
	useEffect(() => {
		if (tab !== "datasets" && tab !== "evaluations") return;
		let active = true;
		void Promise.all([
			refreshDatasets(() => active),
			tab === "evaluations" ? refreshEvaluations(() => active) : Promise.resolve(),
		]).catch((error) => active && setEvidence(formatModelError(error)));
		return () => {
			active = false;
		};
	}, [refreshDatasets, refreshEvaluations, tab]);
	useEffect(() => {
		if (!datasets.length || evaluationDataset) return;
		const dataset = datasets.find((item) =>
			datasetOutputs(item).some(isCompatibleEvaluationSignal),
		);
		if (!dataset) return;
		const bounds = datasetEvaluationBounds(dataset);
		const signal = datasetOutputs(dataset).find(isCompatibleEvaluationSignal);
		setEvaluationDataset(dataset.datasetId);
		setEvaluationSignal(signal?.name ?? "");
		setEvaluationStart(bounds.start);
		setEvaluationEnd(bounds.end);
	}, [datasets, evaluationDataset]);
	useEffect(() => {
		setCompatibleFactors({});
		if (!userId || !model) return;
		void invoke<Record<string, string[]>>("backtest_compatible_factors", {
			request: { userId, strategyArchiveSha256: model },
		})
			.then(setCompatibleFactors)
			.catch((error) => setEvidence(formatModelError(error)));
	}, [model, userId]);
	const trackAttempt = async (result: Attempt) => {
		if (!userId) return;
		setActiveAttempt(result.attemptId);
		let attempt = result;
		while (attempt.status === "pending" || attempt.status === "running") {
			await new Promise((resolve) => window.setTimeout(resolve, 250));
			const attempts = await invoke<Attempt[]>("dataset_generation_list", {
				userId,
			});
			attempt =
				attempts.find((item) => item.attemptId === result.attemptId) ?? attempt;
			setEvidence(
				`${attempt.status} · ${attempt.progressCompleted}/${attempt.progressTotal || "?"} rows`,
			);
		}
		setEvidence(
			attempt.diagnosticEvidence ||
				attempt.datasetId ||
				`Dataset generation ${attempt.status}.`,
		);
		await Promise.all([refresh(), refreshDatasets()]);
	};
	const generate = async () => {
		if (!userId || !model || !snapshot || busy) return;
		setBusy(true);
		setEvidence("");
		await new Promise(requestAnimationFrame);
		try {
			const selected = models.find((item) => item.archiveSha256 === model);
			if (!selected) throw new Error("Select a Model Package.");
			const request = datasetGenerationRequest(
				userId,
				snapshot,
				selected,
				components,
				compatibleFactors,
				{
					...Object.fromEntries(
						selected.parameters.map((parameter) => [
							parameter.name,
							parameter.defaultValue,
						]),
					),
					...modelParameters,
				},
			);
			const result = await invoke<Attempt>("dataset_generation_start", {
				request,
			});
			await trackAttempt(result);
		} catch (error) {
			setEvidence(formatModelError(error));
		} finally {
			setBusy(false);
			setActiveAttempt("");
		}
	};
	const cancel = async () => {
		if (!userId || !activeAttempt) return;
		try {
			await invoke("dataset_generation_cancel", {
				attemptId: activeAttempt,
				userId,
			});
		} catch (error) {
			setEvidence(formatModelError(error));
		}
	};
	const importDataset = async () => {
		if (!userId || busy) return;
		const path = await chooseFile({
			multiple: false,
			filters: [{ name: "AdaQ Signals", extensions: ["adaq-signals"] }],
		});
		if (!path || Array.isArray(path)) return;
		setBusy(true);
		setEvidence("");
		await afterPaint();
		try {
			await invoke("signal_dataset_import", {
				userId,
				archive: Array.from(await readFile(path)),
			});
			setEvidence("External Signal Dataset imported.");
			await refreshDatasets();
		} catch (error) {
			setEvidence(formatModelError(error));
		} finally {
			setBusy(false);
		}
	};
	const exportDataset = async (datasetId: string) => {
		if (!userId || busy) return;
		setBusy(true);
		await afterPaint();
		try {
			const archive = await invoke<number[]>("signal_dataset_export", {
				datasetId,
				userId,
			});
			const path = await save({
				defaultPath: `${datasetId}.adaq-signals`,
				filters: [{ name: "AdaQ Signals", extensions: ["adaq-signals"] }],
			});
			if (!path) {
				setEvidence("Signal Dataset export cancelled.");
				return;
			}
			const file = await open(path, { write: true, createNew: true });
			try {
				await file.write(new Uint8Array(archive));
			} finally {
				await file.close();
			}
			setEvidence(`Signal Dataset exported to ${path}`);
		} catch (error) {
			setEvidence(formatModelError(error));
		} finally {
			setBusy(false);
		}
	};
	const inspectRows = async (datasetId: string, page = 1) => {
		if (!userId || rowsLoading) return;
		setRowsLoading(datasetId);
		await afterPaint();
		try {
			const result = await invoke<RowPage>(
				"signal_dataset_rows",
				signalRowPageRequest(datasetId, userId, page),
			);
			setDatasetRows((current) => ({ ...current, [datasetId]: result }));
		} catch (error) {
			setEvidence(formatModelError(error));
		} finally {
			setRowsLoading("");
		}
	};
	const retry = async (attemptId: string) => {
		if (!userId || busy) return;
		setBusy(true);
		setEvidence("");
		await new Promise(requestAnimationFrame);
		try {
			const result = await invoke<Attempt>("dataset_generation_retry", {
				attemptId,
				userId,
			});
			await trackAttempt(result);
		} catch (error) {
			setEvidence(formatModelError(error));
		} finally {
			setBusy(false);
		}
	};
	const createEvaluation = async () => {
		if (!userId || !evaluationDataset || !evaluationSignal || busy) return;
		setBusy(true);
		setEvidence("Evaluating aligned predictions and realized labels…");
		await afterPaint();
		try {
			const dataset = datasets.find(
				(item) => item.datasetId === evaluationDataset,
			);
			const signal = datasetOutputs(dataset).find(
				(output) => output.name === evaluationSignal,
			);
			if (!dataset || !signal)
				throw new Error("Select compatible evaluation evidence.");
			const report = await invoke<EvaluationReport>("forecast_evaluation_create", {
				request: {
					userId,
					datasetId: evaluationDataset,
					snapshotId: dataset.snapshotId,
					signalName: evaluationSignal,
					horizonBars: signal.horizonBars,
					evaluationStartTimeMs: evaluationStart,
					evaluationEndTimeMs: evaluationEnd,
					stabilityWindowBars,
				},
			});
			setEvidence(`Forecast Evaluation Report ${report.reportId} created.`);
			await refreshEvaluations();
		} catch (error) {
			setEvidence(formatModelError(error));
		} finally {
			setBusy(false);
		}
	};
	const exportEvaluation = async (
		reportId: string,
		format: "json" | "markdown",
	) => {
		if (!userId || busy) return;
		setBusy(true);
		setEvidence(`Preparing authoritative ${format} export…`);
		await afterPaint();
		try {
			const content = await invoke<string>("forecast_evaluation_export", {
				reportId,
				userId,
				format,
			});
			const path = await save({
				defaultPath: evaluationExportFilename(reportId, format),
				filters: [
					{
						name: format === "json" ? "JSON" : "Markdown",
						extensions: [format === "json" ? "json" : "md"],
					},
				],
			});
			if (!path) {
				setEvidence("Forecast Evaluation export cancelled.");
				return;
			}
			const file = await open(path, { write: true, createNew: true });
			try {
				await file.write(new TextEncoder().encode(content));
			} finally {
				await file.close();
			}
			setEvidence(`Forecast Evaluation Report exported to ${path}`);
		} catch (error) {
			setEvidence(formatModelError(error));
		} finally {
			setBusy(false);
		}
	};
	return (
		<main className="mx-auto w-full max-w-6xl p-4 sm:p-6" aria-busy={busy}>
			<header className="mb-6">
				<h1 className="text-2xl font-semibold">Models</h1>
				<p className="text-sm text-muted-foreground">
					Generate immutable forecast evidence from a verified Model Package.
				</p>
			</header>
			{userId ? <PythonProjectsPanel userId={userId} kind="model" /> : null}
			{userId ? <PythonModelLabPanel userId={userId} /> : null}
			<Tabs value={tab} onValueChange={setTab}>
				<TabsList className="max-w-full overflow-x-auto [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
					<TabsTrigger value="create">Create Dataset</TabsTrigger>
					<TabsTrigger value="datasets">Signal Datasets</TabsTrigger>
					<TabsTrigger value="evaluations">Evaluation Reports</TabsTrigger>
				</TabsList>
				<TabsContent value="create">
					<Card>
						<CardHeader>
							<CardTitle>Native Dataset Generation</CardTitle>
						</CardHeader>
						<CardContent className="grid gap-4">
							{componentsLoading ? (
								<LoadingState labelKey="loading.modelPackages" />
							) : (
								<>
									<label className="grid gap-1 text-sm">
										Model Package
										<select
											className="rounded border bg-background p-2"
											value={model}
											onChange={(event) => setModel(event.target.value)}
										>
											{models.map((item) => (
												<option key={item.archiveSha256} value={item.archiveSha256}>
													{item.name} — {item.archiveSha256}
												</option>
											))}
										</select>
									</label>
									{models
										.find((item) => item.archiveSha256 === model)
										?.parameters.map((parameter) => (
											<label key={parameter.name} className="grid gap-1 text-sm">
												{parameter.name}
												<input
													className="rounded border bg-background p-2"
													value={modelParameters[parameter.name] ?? parameter.defaultValue}
													onChange={(event) =>
														setModelParameters((current) => ({
															...current,
															[parameter.name]: event.target.value,
														}))
													}
												/>
											</label>
										))}
								</>
							)}
							{snapshotsLoading ? (
								<LoadingState labelKey="loading.marketDataSnapshots" />
							) : (
								<label className="grid gap-1 text-sm">
									Market Data Snapshot
									<select
										className="rounded border bg-background p-2"
										value={snapshot}
										onChange={(event) => setSnapshot(event.target.value)}
									>
										{snapshots.map((item) => (
											<option key={item.snapshotId} value={item.snapshotId}>
												{item.code} {item.interval} — {item.snapshotId}
											</option>
										))}
									</select>
								</label>
							)}
							<div className="flex gap-2">
								<Button
									className="w-fit"
									loading={busy}
									disabled={
										componentsLoading || snapshotsLoading || !model || !snapshot || busy
									}
									onClick={() => void generate()}
								>
									Create Dataset
								</Button>
								{activeAttempt && (
									<Button variant="outline" onClick={() => void cancel()}>
										Cancel
									</Button>
								)}
							</div>
							{evidence && (
								<pre
									className="max-h-40 overflow-auto rounded bg-muted p-3 text-xs whitespace-pre-wrap"
									aria-live="polite"
								>
									{evidence}
								</pre>
							)}
							<div className="grid gap-2">
								<p className="text-sm font-medium">Generation Attempts</p>
								{attemptsLoading ? (
									<LoadingState labelKey="loading.generationAttempts" />
								) : attempts.length ? (
									attempts.map((attempt) => (
										<div
											key={attempt.attemptId}
											className="grid gap-2 rounded border p-2 text-xs"
										>
											<div className="flex items-center justify-between gap-3">
												<span className="break-all select-text">
													{attempt.status} · {attempt.progressCompleted}/
													{attempt.progressTotal || "?"} · {attempt.attemptId}
												</span>
												{(attempt.status === "failed" ||
													attempt.status === "cancelled") && (
													<Button
														size="sm"
														variant="outline"
														disabled={busy}
														onClick={() => void retry(attempt.attemptId)}
													>
														Retry
													</Button>
												)}
											</div>
											{attempt.diagnosticEvidence && (
												<pre className="max-h-32 overflow-auto whitespace-pre-wrap select-text">
													{attempt.diagnosticEvidence}
												</pre>
											)}
										</div>
									))
								) : (
									<p className="text-sm text-muted-foreground">
										No Generation Attempts yet.
									</p>
								)}
							</div>
						</CardContent>
					</Card>
				</TabsContent>
				<TabsContent value="datasets">
					<Card>
						<CardHeader>
							<div className="flex items-center justify-between gap-3">
								<CardTitle>Signal Datasets</CardTitle>
								<Button
									variant="outline"
									loading={busy}
									disabled={busy}
									onClick={() => void importDataset()}
								>
									Import .adaq-signals
								</Button>
							</div>
						</CardHeader>
						<CardContent className="grid gap-3">
							{datasetsLoading ? (
								<LoadingState labelKey="loading.signalDatasets" />
							) : datasets.length ? (
								datasets.map((item) => (
									<article
										key={item.datasetId}
										className="grid gap-2 rounded border p-3"
										aria-busy={rowsLoading === item.datasetId}
									>
										<p className="font-medium">
											{item.code} {item.interval} · {item.rowCount} rows
										</p>
										<dl className="grid gap-1 break-all text-xs text-muted-foreground">
											<div>
												<dt className="inline font-medium text-foreground">Coverage: </dt>
												<dd className="inline">
													{item.rowCount - item.unavailableCount} present,{" "}
													{item.unavailableCount} unavailable
												</dd>
											</div>
											{item.archiveManifestJson && (
												<Button
													size="sm"
													variant="outline"
													disabled={busy}
													onClick={() => void exportDataset(item.datasetId)}
												>
													Export .adaq-signals
												</Button>
											)}
											<div>
												<dt className="inline font-medium text-foreground">Statuses: </dt>
												<dd className="inline select-text">
													{datasetStatusSummary(item.statusCounts)}
												</dd>
											</div>
											<div>
												<dt className="inline font-medium text-foreground">
													Model Artifact:{" "}
												</dt>
												<dd className="inline select-text">
													{item.modelArtifact?.sha256 ?? "Unavailable"}
												</dd>
											</div>
											<div>
												<dt className="inline font-medium text-foreground">
													Producer Segments:{" "}
												</dt>
												<dd className="inline">
													{item.externalProducerSegments?.length ??
														item.producerSegments.length}{" "}
													· {item.continuousBarSegments} continuous · {item.barGapRule}
												</dd>
											</div>
											<div>
												<dt className="inline font-medium text-foreground">Snapshot: </dt>
												<dd className="inline select-text">{item.snapshotId}</dd>
											</div>
											<div>
												<dt className="inline font-medium text-foreground">
													Feature Plan:{" "}
												</dt>
												<dd className="inline select-text">{item.featurePlanHash}</dd>
											</div>
											<div>
												<dt className="inline font-medium text-foreground">
													Seed / Trust:{" "}
												</dt>
												<dd className="inline">
													{item.seed} · {item.trust}
												</dd>
											</div>
											<div>
												<dt className="inline font-medium text-foreground">
													Dataset / Parquet:{" "}
												</dt>
												<dd className="inline select-text">
													{item.datasetId} · {item.parquetSha256}
												</dd>
											</div>
											<div>
												<dt className="inline font-medium text-foreground">
													Component Lock:{" "}
												</dt>
												<dd className="inline select-text">
													{item.componentLock
														.map((entry) => `${entry.alias}: ${entry.archiveSha256}`)
														.join(", ")}
												</dd>
											</div>
											<details>
												<summary className="cursor-pointer font-medium text-foreground">
													Provenance
												</summary>
												<pre className="mt-2 max-h-48 overflow-auto whitespace-pre-wrap select-text">
													{JSON.stringify(
														{
															modelArtifact: item.modelArtifact?.provenance,
															modelOutputs: item.modelOutputs,
															modelParameters: item.modelParameters,
															sourceWarmupBars: item.sourceWarmupBars,
															modelWarmupBars: item.modelWarmupBars,
															producerSegments: item.producerSegments,
															externalProducerSegments: item.externalProducerSegments,
															predictionSource: item.predictionSource,
															engineIdentity: item.engineIdentity,
															featurePlan: JSON.parse(item.featurePlanJson),
															archiveManifest:
																item.archiveManifestJson &&
																JSON.parse(item.archiveManifestJson),
														},
														null,
														2,
													)}
												</pre>
											</details>
											<details
												onToggle={(event) =>
													event.currentTarget.open &&
													!datasetRows[item.datasetId] &&
													void inspectRows(item.datasetId)
												}
											>
												<summary className="cursor-pointer font-medium text-foreground">
													Rows
												</summary>
												{rowsLoading === item.datasetId && (
													<p aria-live="polite">{t("loading.signalRows")}</p>
												)}
												{datasetRows[item.datasetId] && (
													<div className="mt-2 grid gap-2">
														{datasetRows[item.datasetId].items.map((row) => (
															<code
																key={`${row.predictionTimeMs}:${row.status}`}
																className="select-text whitespace-pre-wrap"
															>
																{signalRowSummary(row)}
															</code>
														))}
														<div className="flex gap-2">
															<Button
																size="sm"
																variant="outline"
																disabled={
																	Boolean(rowsLoading) || datasetRows[item.datasetId].page === 1
																}
																onClick={() =>
																	void inspectRows(
																		item.datasetId,
																		datasetRows[item.datasetId].page - 1,
																	)
																}
															>
																Previous
															</Button>
															<Button
																size="sm"
																variant="outline"
																disabled={
																	Boolean(rowsLoading) ||
																	datasetRows[item.datasetId].page *
																		datasetRows[item.datasetId].pageSize >=
																		datasetRows[item.datasetId].total
																}
																onClick={() =>
																	void inspectRows(
																		item.datasetId,
																		datasetRows[item.datasetId].page + 1,
																	)
																}
															>
																Next
															</Button>
														</div>
													</div>
												)}
											</details>
										</dl>
									</article>
								))
							) : (
								<p className="text-sm text-muted-foreground">
									No Forecast Signal Datasets yet.
								</p>
							)}
						</CardContent>
					</Card>
				</TabsContent>
				<TabsContent value="evaluations">
					<div className="grid gap-4 lg:grid-cols-[minmax(18rem,24rem)_1fr]">
						<Card>
							<CardHeader>
								<CardTitle>Create Forecast Evaluation</CardTitle>
							</CardHeader>
							<CardContent className="grid gap-3">
								<label className="grid gap-1 text-sm">
									Signal Dataset
									<select
										className="min-w-0 rounded border bg-background p-2"
										value={evaluationDataset}
										onChange={(event) => {
											const dataset = datasets.find(
												(item) => item.datasetId === event.target.value,
											);
											const outputs = datasetOutputs(dataset).filter(
												isCompatibleEvaluationSignal,
											);
											const bounds = datasetEvaluationBounds(dataset);
											setEvaluationDataset(event.target.value);
											setEvaluationSignal(outputs[0]?.name ?? "");
											setEvaluationStart(bounds.start);
											setEvaluationEnd(bounds.end);
										}}
									>
										<option value="">Select compatible evidence</option>
										{datasets
											.filter((item) =>
												datasetOutputs(item).some(isCompatibleEvaluationSignal),
											)
											.map((item) => (
												<option key={item.datasetId} value={item.datasetId}>
													{item.code} {item.interval} — {item.datasetId}
												</option>
											))}
									</select>
								</label>
								<label className="grid gap-1 text-sm">
									Forecast Signal
									<select
										className="rounded border bg-background p-2"
										value={evaluationSignal}
										onChange={(event) => setEvaluationSignal(event.target.value)}
									>
										{datasetOutputs(
											datasets.find((item) => item.datasetId === evaluationDataset),
										)
											.filter(isCompatibleEvaluationSignal)
											.map((output) => (
												<option key={output.name} value={output.name}>
													{output.name} · horizon {output.horizonBars}
												</option>
											))}
									</select>
								</label>
								<label className="grid gap-1 text-sm">
									Evaluation start (ms)
									<input
										type="number"
										className="rounded border bg-background p-2"
										value={evaluationStart}
										onChange={(event) => setEvaluationStart(event.target.valueAsNumber)}
									/>
								</label>
								<label className="grid gap-1 text-sm">
									Evaluation end (ms)
									<input
										type="number"
										className="rounded border bg-background p-2"
										value={evaluationEnd}
										onChange={(event) => setEvaluationEnd(event.target.valueAsNumber)}
									/>
								</label>
								<label className="grid gap-1 text-sm">
									Stability window (Bars)
									<input
										type="number"
										min={1}
										className="rounded border bg-background p-2"
										value={stabilityWindowBars}
										onChange={(event) =>
											setStabilityWindowBars(event.target.valueAsNumber)
										}
									/>
								</label>
								<Button
									loading={busy}
									loadingText={t("loading.evaluating")}
									disabled={
										busy ||
										!evaluationDataset ||
										!evaluationSignal ||
										!Number.isFinite(evaluationStart) ||
										!Number.isFinite(evaluationEnd) ||
										evaluationStart > evaluationEnd ||
										!Number.isInteger(stabilityWindowBars) ||
										stabilityWindowBars < 1
									}
									onClick={() => void createEvaluation()}
								>
									Create Report
								</Button>
								{evidence && (
									<pre
										className="max-h-40 overflow-auto rounded bg-muted p-3 text-xs whitespace-pre-wrap"
										aria-live="polite"
									>
										{evidence}
									</pre>
								)}
								<p className="text-xs text-muted-foreground">
									Single-Instrument time-series evidence only. These statistics are not
									cross-sectional IC, Strategy profitability, fees, turnover, a universal
									investment-quality score, or a trading recommendation.
								</p>
							</CardContent>
						</Card>
						<div className="grid min-w-0 gap-3">
							{evaluationsLoading ? (
								<LoadingState labelKey="loading.forecastEvaluationReports" />
							) : evaluationReports.length ? (
								evaluationReports.map((report) => (
									<Card key={report.reportId}>
										<CardHeader>
											<CardTitle className="break-all text-base">
												{evaluationReportSummary(report)}
											</CardTitle>
										</CardHeader>
										<CardContent className="grid gap-3 text-sm">
											{report.evidenceState.summary !== "out-of-sample" && (
												<p
													className="rounded border border-amber-500/50 bg-amber-500/10 p-2"
													role="alert"
												>
													{report.evidenceState.summary === "overlapping"
														? "Known training, fitting, or normalization evidence overlaps this evaluation window."
														: "Complete training, fitting, or normalization boundaries are unavailable; this report is not proven out-of-sample."}
												</p>
											)}
											<div className="grid gap-2 sm:grid-cols-2 xl:grid-cols-4">
												<ResearchMetric
													metricId="forecast.aligned-count"
													value={String(report.metrics.alignedCount)}
													className="rounded border p-3"
												/>
												<ResearchMetric
													metricId="forecast.coverage"
													value={`${(report.metrics.coverage * 100).toFixed(2)}%`}
													className="rounded border p-3"
												/>
												<ResearchMetric
													metricId="forecast.missingness"
													value={`${(report.metrics.missingness * 100).toFixed(2)}%`}
													className="rounded border p-3"
												/>
												{evaluationMetricKind(report.signalContract) === "probability" ? (
													<>
														<ResearchMetric
															metricId="forecast.brier-score"
															value={metricValue(report.metrics.brierScore)}
															className="rounded border p-3"
														/>
														<ResearchMetric
															metricId="forecast.log-loss"
															value={metricValue(report.metrics.logLoss)}
															className="rounded border p-3"
														/>
														<ResearchMetric
															metricId="forecast.roc-auc"
															value={metricValue(report.metrics.rocAuc)}
															className="rounded border p-3"
														/>
														<ResearchMetric
															metricId="forecast.calibration"
															value={
																report.metrics.calibration
																	? `${report.metrics.calibration.filter((bucket) => bucket.count).length} populated buckets`
																	: "Unavailable"
															}
															className="rounded border p-3"
														/>
													</>
												) : evaluationMetricKind(report.signalContract) === "score" ? (
													<>
														<ResearchMetric
															metricId="forecast.pearson-ic"
															value={metricValue(report.metrics.pearsonIc)}
															className="rounded border p-3"
														/>
														<ResearchMetric
															metricId="forecast.spearman-rank-ic"
															value={metricValue(report.metrics.spearmanRankIc)}
															className="rounded border p-3"
														/>
														<ResearchMetric
															metricId="forecast.window-icir"
															value={metricValue(report.metrics.windowIcir)}
															className="rounded border p-3"
														/>
														<ResearchMetric
															metricId="forecast.five-quantiles"
															value={
																report.metrics.undefinedMetrics?.quantiles ??
																"Five-quantile realized Target evidence"
															}
															className="rounded border p-3"
														/>
													</>
												) : evaluationMetricKind(report.signalContract) === "custom" ? (
													<p className="col-span-full rounded border p-3" role="status">
														Custom Prediction Kind or Custom Target recorded. Common coverage,
														distribution, stability, and provenance remain inspectable; no
														specialized evaluator is invented. Evidence:{" "}
														<code>
															{report.metrics.undefinedMetrics?.probabilityMetrics ??
																"requires-verifiable-realized-labels"}
														</code>
													</p>
												) : (
													<>
														<ResearchMetric
															metricId="forecast.mae"
															value={metricValue(report.metrics.mae)}
															className="rounded border p-3"
														/>
														<ResearchMetric
															metricId="forecast.rmse"
															value={metricValue(report.metrics.rmse)}
															className="rounded border p-3"
														/>
														<ResearchMetric
															metricId="forecast.mean-bias"
															value={metricValue(report.metrics.meanBias)}
															className="rounded border p-3"
														/>
														<ResearchMetric
															metricId="forecast.pearson-correlation"
															value={
																report.metrics.pearsonCorrelation == null
																	? "Unavailable"
																	: metricValue(report.metrics.pearsonCorrelation)
															}
															className="rounded border p-3"
														/>
													</>
												)}
											</div>
											<details>
												<summary className="cursor-pointer font-medium">Evidence</summary>
												<pre className="mt-2 max-h-64 overflow-auto whitespace-pre-wrap select-text">
													{JSON.stringify(
														{
															metrics: report.metrics,
															stabilityWindows: report.stabilityWindows,
															unavailableRows: report.unavailableRows,
														},
														null,
														2,
													)}
												</pre>
											</details>
											<details>
												<summary className="cursor-pointer font-medium">Provenance</summary>
												<pre className="mt-2 max-h-64 overflow-auto whitespace-pre-wrap select-text">
													{JSON.stringify(
														{
															reportId: report.reportId,
															datasetId: report.datasetId,
															snapshotId: report.snapshotId,
															signalContract: report.signalContract,
															producerSegments: report.producerSegments,
															scaleProvenance: report.scaleProvenance,
															evidenceState: report.evidenceState,
															trustState: report.trustState,
															metricVersions: report.metricVersions,
															engineIdentity: report.engineIdentity,
															schemaIdentity: report.schemaIdentity,
															datasetParquetSha256: report.datasetParquetSha256,
															componentLock: report.componentLock,
															featurePlanHash: report.featurePlanHash,
														},
														null,
														2,
													)}
												</pre>
											</details>
											<div className="flex flex-wrap gap-2">
												<Button
													variant="outline"
													disabled={busy}
													onClick={() => void exportEvaluation(report.reportId, "json")}
												>
													Export JSON
												</Button>
												<Button
													variant="outline"
													disabled={busy}
													onClick={() => void exportEvaluation(report.reportId, "markdown")}
												>
													Export Markdown
												</Button>
											</div>
										</CardContent>
									</Card>
								))
							) : (
								<p className="rounded border p-4 text-sm text-muted-foreground">
									No Forecast Evaluation Reports yet. Choose compatible immutable Score,
									Expected Value, Probability, or Custom evidence to create one.
								</p>
							)}
						</div>
					</div>
				</TabsContent>
			</Tabs>
		</main>
	);
}
