import { Badge } from "@/components/ui/badge";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { getErrorMessage, useMarketSessionStore } from "@/lib/market-session";
import { Link } from "@tanstack/react-router";
import { Channel, invoke } from "@tauri-apps/api/core";
import {
	ArrowRightIcon,
	CheckCircle2Icon,
	CircleAlertIcon,
	DatabaseIcon,
	LoaderCircleIcon,
} from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useQuery } from "@tanstack/react-query";

type PipelineDatasetSummary = {
	sourceId: string;
	canonicalId?: string;
	revision: number;
	state: "unassessed" | "passed" | "degraded" | "rejected";
	sourceRecordCount: number;
	canonicalRecordCount: number;
	quarantinedRecordCount: number;
	gapCount: number;
};

type ResearchEvidenceProjection = {
	contextRevision: number;
	contextHash: string;
	market: string;
	venue: string;
	rangeStartMs: number;
	rangeEndMs: number;
	snapshotId: string;
	universeId?: string;
};

type SnapshotOption = {
	snapshotId: string;
	code: string;
	interval: string;
	barCount: number;
};

type UniverseOption = {
	snapshotId: string;
	startTimeMs: number;
	endTimeMs: number;
	contentSha256: string;
};

type QualityView = {
	reportId: string;
	state: "passed" | "degraded" | "rejected";
	canonicalId?: string;
	duplicateCount: number;
	conflictCount: number;
	quarantineCount: number;
	gapCount: number;
	warningCount: number;
	reasons: Array<{ code: string; message: string }>;
};

type FoundationAcquisitionView = {
	operationId: string;
	market: string;
	venue: string;
	state: "running" | "completed" | "cancelled" | "failed";
	revision?: number;
	error?: string;
	startedAtMs: number;
	finishedAtMs?: number;
};

type OkxAcquisitionStatus = {
	instrument: { venue: { id: string }; code: string };
	interval: string;
	state:
		| "pending"
		| "running"
		| "completed"
		| "degraded"
		| "failed"
		| "cancelled";
	pages: number;
	gapCount: number;
	revision?: number;
	retryCount: number;
	lastError?: string;
};

type InstrumentMasterSnapshot = {
	snapshotId: string;
	retrievedAtMs: number;
	connectorVersion: string;
	instruments: Array<{
		code: string;
		baseAsset: string;
		quoteAsset: string;
		status: string;
		listingTimeMs?: number;
		priceIncrement: string;
		quantityIncrement: string;
		minimumQuantity: string;
	}>;
	quoteVolume24hUsdt?: Record<string, string>;
	ignoreUntradable?: boolean;
	minimumQuoteVolume24h?: string;
};

const shortId = (value: string) =>
	value.length > 8 ? `${value.slice(0, 3)}...${value.slice(-3)}` : value;

const humanizeNumber = (value: string | number | undefined) => {
	const number = Number(value);
	if (!Number.isFinite(number)) return "—";
	return new Intl.NumberFormat(undefined, {
		notation: Math.abs(number) >= 1_000 ? "compact" : "standard",
		maximumFractionDigits: Math.abs(number) < 1 ? 8 : 2,
	}).format(number);
};

type OkxBackfillEvent = {
	event:
		| "universeLoaded"
		| "instrumentStarted"
		| "page"
		| "published"
		| "sourceRetained"
		| "instrumentCompleted";
	data?: {
		downloadedRecords?: number;
		instrumentCount?: number;
		instrument?: { code?: string };
	};
};

type OkxBackfillProgress = {
	instrumentCount: number;
	completedInstruments: number;
	currentInstrument?: string;
	downloadedRecords: number;
	startedAtMs: number;
};

type BackfillDraft = {
	rangeStart: string;
	rangeEnd: string;
	interval: OkxInterval;
	scope: "selected" | "all";
	instrumentCodes: string[];
	startedAtMs: number;
};

const OKX_INTERVALS = [
	"1s",
	"1m",
	"3m",
	"5m",
	"15m",
	"30m",
	"1h",
	"2h",
	"4h",
	"6h",
	"12h",
	"1d",
	"2d",
	"3d",
	"5d",
	"1w",
	"1mo",
	"3mo",
] as const;
type OkxInterval = (typeof OKX_INTERVALS)[number];

type FoundationMarket = {
	id: "crypto";
	titleKey: string;
	descriptionKey: string;
	acquireButtonKey: string;
	workspace: "/markets/crypto";
	acquireCommand: string;
	cancelCommand?: string;
};

const MARKET_VENUES = {
	crypto: ["okx"],
} as const;

const markets: FoundationMarket[] = [
	{
		id: "crypto",
		titleKey: "dataFoundation.okxInstrumentMasterTitle",
		descriptionKey: "dataFoundation.okxInstrumentMasterDescription",
		acquireButtonKey: "dataFoundation.okxInstrumentMasterStart",
		workspace: "/markets/crypto",
		acquireCommand: "okx_instrument_master_acquire",
		cancelCommand: "okx_instrument_master_cancel",
	},
];

const PAGE_SIZE = 5;

export function DataFoundationPage() {
	const { t } = useTranslation();
	const userId = useMarketSessionStore((state) => state.userId);
	const [activeOperation, setActiveOperation] = useState<string>();
	const [backfillTaskId, setBackfillTaskId] = useState<string>();
	const [backfillProgress, setBackfillProgress] = useState<string>();
	const [backfillStats, setBackfillStats] = useState<OkxBackfillProgress>();
	const [backfillScope, setBackfillScope] = useState<"selected" | "all">(
		"selected",
	);
	const [instrumentCodes, setInstrumentCodes] = useState("BTC-USDT");
	const [backfillInterval, setBackfillInterval] = useState<OkxInterval>("1m");
	const [savedBackfill, setSavedBackfill] = useState<BackfillDraft>();
	const [clockMs, setClockMs] = useState(() => Date.now());
	const [error, setError] = useState<string>();
	const [contextMarket, setContextMarket] =
		useState<FoundationMarket["id"]>("crypto");
	const [contextVenue, setContextVenue] = useState("okx");
	const [snapshotId, setSnapshotId] = useState("");
	const [ignoreUntradable, setIgnoreUntradable] = useState(true);
	const [minimumQuoteVolume24h, setMinimumQuoteVolume24h] = useState("1000000");
	const [universeId, setUniverseId] = useState("");
	const [selectedSourceId, setSelectedSourceId] = useState<string>();
	const [publishingId, setPublishingId] = useState<string>();
	const [publicationPage, setPublicationPage] = useState(1);
	const [acquisitionPage, setAcquisitionPage] = useState(1);
	const [rangeStart, setRangeStart] = useState(() =>
		new Date(Date.now() - 30 * 86_400_000).toISOString().slice(0, 10),
	);
	const [rangeEnd, setRangeEnd] = useState(() =>
		new Date().toISOString().slice(0, 10),
	);
	useEffect(() => {
		const raw = localStorage.getItem("adaq.okx-backfill-draft");
		if (!raw) return;
		try {
			const parsed = JSON.parse(raw) as Partial<BackfillDraft>;
			setSavedBackfill({
				rangeStart: parsed.rangeStart ?? "",
				rangeEnd: parsed.rangeEnd ?? "",
				interval: parsed.interval ?? "1m",
				scope: parsed.scope ?? "selected",
				instrumentCodes: parsed.instrumentCodes ?? ["BTC-USDT"],
				startedAtMs: parsed.startedAtMs ?? Date.now(),
			});
		} catch {
			localStorage.removeItem("adaq.okx-backfill-draft");
		}
	}, []);
	useEffect(() => {
		if (!backfillTaskId) return;
		const timer = window.setInterval(() => setClockMs(Date.now()), 1000);
		return () => window.clearInterval(timer);
	}, [backfillTaskId]);
	const pipelineQuery = useQuery({
		queryKey: ["data-foundation-pipeline", userId],
		queryFn: () =>
			invoke<PipelineDatasetSummary[]>("market_data_pipeline_list", { userId }),
		enabled: Boolean(userId),
		staleTime: 30_000,
	});
	const snapshotsQuery = useQuery({
		queryKey: ["research-context-snapshots", userId],
		queryFn: () =>
			invoke<SnapshotOption[]>("snapshot_list_readable", {
				request: { userId },
			}),
		enabled: Boolean(userId),
		staleTime: 30_000,
	});
	const universeQuery = useQuery({
		queryKey: ["research-context-universes", userId],
		queryFn: async () => {
			const page = await invoke<{ items: UniverseOption[] }>(
				"snapshot_list_universe",
				{
					request: { userId, page: 1 },
				},
			);
			return page.items;
		},
		enabled: Boolean(userId),
		staleTime: 30_000,
	});
	const contextQuery = useQuery({
		queryKey: ["research-evidence-context", userId],
		queryFn: () =>
			invoke<ResearchEvidenceProjection | null>("research_context_get", {
				userId,
			}),
		enabled: Boolean(userId),
		staleTime: 30_000,
	});
	const foundationHistoryQuery = useQuery({
		queryKey: ["data-foundation-operation-history", userId],
		queryFn: () =>
			invoke<FoundationAcquisitionView[]>("foundation_acquisition_history", {
				userId,
			}),
		enabled: Boolean(userId),
		staleTime: 10_000,
	});
	const instrumentMasterQuery = useQuery({
		queryKey: ["okx-instrument-master", userId],
		queryFn: () =>
			invoke<InstrumentMasterSnapshot[]>("okx_instrument_master_list", {
				request: { userId },
			}),
		enabled: Boolean(userId),
		staleTime: 30_000,
	});
	const acquisitionQuery = useQuery({
		queryKey: ["data-foundation-acquisitions", userId],
		queryFn: () =>
			invoke<OkxAcquisitionStatus[]>("okx_acquisition_status", {
				request: { userId },
			}),
		enabled: Boolean(userId),
		staleTime: 30_000,
	});
	const qualityQuery = useQuery({
		queryKey: ["data-foundation-quality", userId, selectedSourceId],
		queryFn: () => {
			if (!selectedSourceId) throw new Error("Source evidence is not selected");
			return invoke<QualityView>("market_data_pipeline_quality", {
				request: { userId, evidenceId: selectedSourceId },
			});
		},
		enabled: Boolean(userId && selectedSourceId),
		staleTime: 30_000,
	});

	const publish = async (
		dataset: PipelineDatasetSummary,
		allowDegraded = false,
	) => {
		if (!userId || !dataset.canonicalId) return;
		setError(undefined);
		setPublishingId(dataset.canonicalId);
		try {
			await invoke("market_data_pipeline_publish_snapshot", {
				request: {
					userId,
					canonicalId: dataset.canonicalId,
					allowDegraded,
				},
			});
			await snapshotsQuery.refetch();
		} catch (cause) {
			setError(getErrorMessage(cause));
		} finally {
			setPublishingId(undefined);
		}
	};

	const runOkxBackfill = async (draft?: BackfillDraft) => {
		if (!userId || backfillTaskId) return;
		const nextDraft = draft ?? {
			rangeStart,
			rangeEnd,
			interval: backfillInterval,
			scope: backfillScope,
			instrumentCodes: instrumentCodes
				.split(",")
				.map((code) => code.trim().toUpperCase())
				.filter(Boolean),
			startedAtMs: Date.now(),
		};
		if (nextDraft.scope === "selected" && !nextDraft.instrumentCodes.length) {
			setError(
				"Select at least one OKX instrument, or explicitly choose all instruments.",
			);
			return;
		}
		const startTimeMs = Date.parse(`${nextDraft.rangeStart}T00:00:00Z`);
		const endTimeMs = Date.parse(`${nextDraft.rangeEnd}T00:00:00Z`) + 86_400_000;
		if (
			!Number.isFinite(startTimeMs) ||
			!Number.isFinite(endTimeMs) ||
			startTimeMs >= endTimeMs
		) {
			setError("OKX backfill requires an increasing date range");
			return;
		}
		const taskId = `crypto-foundation-${crypto.randomUUID()}`;
		localStorage.setItem("adaq.okx-backfill-draft", JSON.stringify(nextDraft));
		const onEvent = new Channel<OkxBackfillEvent>();
		onEvent.onmessage = (event) => {
			setBackfillProgress(event.event);
			setBackfillStats((current) => {
				const next = current ?? {
					instrumentCount: 0,
					completedInstruments: 0,
					downloadedRecords: 0,
					startedAtMs: nextDraft.startedAtMs,
				};
				return {
					...next,
					instrumentCount: event.data?.instrumentCount ?? next.instrumentCount,
					downloadedRecords:
						next.downloadedRecords + (event.data?.downloadedRecords ?? 0),
					completedInstruments:
						event.event === "instrumentCompleted"
							? next.completedInstruments + 1
							: next.completedInstruments,
					currentInstrument: event.data?.instrument?.code ?? next.currentInstrument,
				};
			});
		};
		setError(undefined);
		setBackfillTaskId(taskId);
		setBackfillStats({
			instrumentCount: 0,
			completedInstruments: 0,
			downloadedRecords: 0,
			startedAtMs: nextDraft.startedAtMs,
		});
		let completed = false;
		try {
			await invoke("okx_backfill_source", {
				request: {
					userId,
					taskId,
					startTimeMs,
					endTimeMs,
					interval: nextDraft.interval,
					instrumentCodes:
						nextDraft.scope === "all" ? [] : nextDraft.instrumentCodes,
				},
				onEvent,
			});
			await Promise.all([
				pipelineQuery.refetch(),
				acquisitionQuery.refetch(),
				foundationHistoryQuery.refetch(),
				instrumentMasterQuery.refetch(),
			]);
			completed = true;
		} catch (cause) {
			setError(getErrorMessage(cause));
		} finally {
			setBackfillTaskId(undefined);
			setBackfillProgress(undefined);
			setBackfillStats(undefined);
			if (completed) {
				localStorage.removeItem("adaq.okx-backfill-draft");
				setSavedBackfill(undefined);
			} else {
				setSavedBackfill(nextDraft);
			}
		}
	};

	const establishContext = async () => {
		if (!userId || !snapshotId) return;
		setError(undefined);
		try {
			await invoke("research_context_establish", {
				draft: {
					userId,
					market: contextMarket,
					venue: contextVenue,
					rangeStartMs: Date.parse(`${rangeStart}T00:00:00Z`),
					rangeEndMs: Date.parse(`${rangeEnd}T00:00:00Z`) + 86_400_000,
					snapshotId,
					universeId: universeId || null,
					evidence: [
						{
							id: snapshotId,
							lineageHash: snapshotId,
							userId,
							market: contextMarket,
							venue: contextVenue,
							snapshotId,
							universeId: universeId || null,
							featureId: null,
							factorId: null,
							modelId: null,
							grade: "provider-graded",
							accessible: true,
							complete: true,
							fresh: true,
						},
					],
				},
				stage: "features",
			});
			await contextQuery.refetch();
		} catch (cause) {
			setError(getErrorMessage(cause));
		}
	};

	const acquire = async (market: FoundationMarket) => {
		if (!userId) return;
		const latest = instrumentMasterQuery.data?.at(-1);
		if (market.id === "crypto" && latest) {
			const retrieved = new Date(latest.retrievedAtMs).toLocaleString(undefined, {
				second: "2-digit",
			});
			if (
				!window.confirm(`OKX 交易品种目录已于 ${retrieved} 获取。是否重新获取？`)
			) {
				return;
			}
		}
		const operationId = `${market.id}-foundation-${crypto.randomUUID()}`;
		setError(undefined);
		setActiveOperation(operationId);
		try {
			await invoke(market.acquireCommand, {
				request: {
					userId,
					operationId,
					ignoreUntradable,
					minimumQuoteVolume24h,
				},
			});
			await instrumentMasterQuery.refetch();
			await Promise.all([
				pipelineQuery.refetch(),
				acquisitionQuery.refetch(),
				foundationHistoryQuery.refetch(),
			]);
		} catch (cause) {
			setError(getErrorMessage(cause));
		} finally {
			setActiveOperation(undefined);
		}
	};

	const cancel = async (market: FoundationMarket) => {
		if (!userId || !activeOperation || !market.cancelCommand) return;
		try {
			await invoke(market.cancelCommand, {
				request: { userId, operationId: activeOperation },
			});
			await Promise.all([
				acquisitionQuery.refetch(),
				foundationHistoryQuery.refetch(),
			]);
		} catch (cause) {
			setError(getErrorMessage(cause));
		}
	};

	const cancelOkxBackfill = async () => {
		if (!userId || !backfillTaskId) return;
		try {
			await invoke("okx_backfill_cancel", {
				request: { userId, taskId: backfillTaskId },
			});
		} catch (cause) {
			setError(getErrorMessage(cause));
		}
	};

	const retry = async (operation: OkxAcquisitionStatus) => {
		if (operation.instrument.venue.id !== "okx") return;
		await acquire(markets[0]);
	};
	const elapsedMs = backfillStats
		? Math.max(
				0,
				(backfillTaskId ? clockMs : backfillStats.startedAtMs) -
					backfillStats.startedAtMs,
			)
		: 0;
	const elapsedLabel = `${Math.floor(elapsedMs / 60_000)}m ${Math.floor((elapsedMs % 60_000) / 1_000)}s`;
	const etaMs =
		backfillStats && backfillStats.completedInstruments > 0
			? (elapsedMs / backfillStats.completedInstruments) *
				Math.max(
					0,
					backfillStats.instrumentCount - backfillStats.completedInstruments,
				)
			: undefined;
	const etaLabel = etaMs == null ? "—" : `${Math.ceil(etaMs / 60_000)}m`;

	const publicationItems = pipelineQuery.data
		? [...pipelineQuery.data].reverse()
		: [];
	const publicationPageCount = Math.max(
		1,
		Math.ceil(publicationItems.length / PAGE_SIZE),
	);
	const publicationSafePage = Math.min(publicationPage, publicationPageCount);
	const publicationSlice = publicationItems.slice(
		(publicationSafePage - 1) * PAGE_SIZE,
		publicationSafePage * PAGE_SIZE,
	);

	const acquisitionItems = acquisitionQuery.data
		? [...acquisitionQuery.data].reverse()
		: [];
	const acquisitionPageCount = Math.max(
		1,
		Math.ceil(acquisitionItems.length / PAGE_SIZE),
	);
	const acquisitionSafePage = Math.min(acquisitionPage, acquisitionPageCount);
	const acquisitionSlice = acquisitionItems.slice(
		(acquisitionSafePage - 1) * PAGE_SIZE,
		acquisitionSafePage * PAGE_SIZE,
	);

	return (
		<div className="flex min-w-0 flex-1 flex-col gap-5 p-4 lg:p-6">
			<div>
				<div className="flex items-center gap-2">
					<DatabaseIcon className="size-5 text-primary" aria-hidden="true" />
					<h1 className="text-2xl font-semibold">{t("dataFoundation.title")}</h1>
				</div>
				<p className="text-sm text-muted-foreground">
					{t("dataFoundation.description")}
				</p>
			</div>
			{error ? (
				<p className="text-sm text-destructive" role="alert">
					{error}
				</p>
			) : null}
			<Card>
				<CardHeader>
					<CardTitle>{t("dataFoundation.readinessTitle")}</CardTitle>
					<CardDescription>
						{t("dataFoundation.readinessDescription")}
					</CardDescription>
				</CardHeader>
				<CardContent className="grid gap-3 sm:grid-cols-3">
					<ReadinessStat
						label={t("dataFoundation.sourceEvidence")}
						value={pipelineQuery.data?.length ?? 0}
						loading={pipelineQuery.isPending}
					/>
					<ReadinessStat
						label={t("dataFoundation.canonicalEvidence")}
						value={
							pipelineQuery.data?.filter((item) => Boolean(item.canonicalId)).length ??
							0
						}
						loading={pipelineQuery.isPending}
					/>
					<ReadinessStat
						label={t("dataFoundation.degradedOrRejected")}
						value={
							pipelineQuery.data?.filter(
								(item) => item.state === "degraded" || item.state === "rejected",
							).length ?? 0
						}
						loading={pipelineQuery.isPending}
					/>
				</CardContent>
			</Card>
			<div className="grid gap-4 lg:grid-cols-3">
				{markets.map((market) => {
					const active = activeOperation?.startsWith(`${market.id}-`) ?? false;
					return (
						<Card key={market.id}>
							<CardHeader>
								<div className="flex items-start justify-between gap-3">
									<div>
										<CardTitle>{t(market.titleKey)}</CardTitle>
										<CardDescription>{t(market.descriptionKey)}</CardDescription>
									</div>
									<Badge variant={active ? "default" : "outline"}>
										{active
											? t("dataFoundation.running")
											: t("dataFoundation.readyToAcquire")}
									</Badge>
								</div>
							</CardHeader>
							<CardContent className="flex flex-wrap gap-2">
								<div className="w-full grid gap-3 rounded-md border p-3 text-sm sm:grid-cols-2">
									<label className="flex items-center gap-2">
										<input
											type="checkbox"
											checked={ignoreUntradable}
											onChange={(event) => setIgnoreUntradable(event.target.checked)}
										/>
										{t("dataFoundation.okxIgnoreUntradable")}
									</label>
									<label className="grid gap-1">
										<span>{t("dataFoundation.okxMinimumQuoteVolume")}</span>
										<input
											className="h-9 rounded-md border bg-background px-2"
											inputMode="decimal"
											value={minimumQuoteVolume24h}
											onChange={(event) => setMinimumQuoteVolume24h(event.target.value)}
										/>
									</label>
								</div>
								<button
									type="button"
									className="inline-flex h-9 items-center justify-center rounded-md bg-primary px-3 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:pointer-events-none disabled:opacity-50"
									onClick={() => void acquire(market)}
									disabled={Boolean(activeOperation)}
								>
									{t(market.acquireButtonKey)}
								</button>
								{active && market.cancelCommand ? (
									<button
										type="button"
										className="inline-flex h-9 items-center justify-center rounded-md border px-3 text-sm font-medium hover:bg-muted"
										onClick={() => void cancel(market)}
									>
										{t("dataFoundation.cancelAcquisition")}
									</button>
								) : null}
								<Link
									to={market.workspace}
									className="inline-flex h-9 items-center gap-2 rounded-md border px-3 text-sm font-medium hover:bg-muted"
								>
									{t("dataFoundation.openMarketWorkspace")}
									<ArrowRightIcon className="size-4" aria-hidden="true" />
								</Link>
							</CardContent>
						</Card>
					);
				})}
			</div>
			<Card>
				<CardHeader>
					<CardTitle>
						{t("dataFoundation.okxInstrumentMasterEvidenceTitle")}
					</CardTitle>
					<CardDescription>
						{t("dataFoundation.okxInstrumentMasterEvidenceDescription")}
					</CardDescription>
				</CardHeader>
				<CardContent>
					{instrumentMasterQuery.isPending ? (
						<p className="text-sm text-muted-foreground" role="status">
							{t("dataFoundation.loadingHistory")}
						</p>
					) : instrumentMasterQuery.data?.length ? (
						(() => {
							const latest = instrumentMasterQuery.data.at(-1);
							if (!latest) return null;
							return (
								<div className="grid gap-3 text-sm">
									<p className="text-xs text-muted-foreground">
										{t("dataFoundation.okxInstrumentMasterTableDescription")}
									</p>
									<div className="grid gap-1 sm:grid-cols-3">
										<span>
											<strong className="text-primary">
												{humanizeNumber(latest.instruments.length)}
											</strong>{" "}
											{t("dataFoundation.okxInstrumentMasterCountLabel")}
										</span>
										<span>
											{t("dataFoundation.okxInstrumentMasterRetrieved", {
												date: new Date(latest.retrievedAtMs).toLocaleString(),
											})}
										</span>
										<span>
											{t("dataFoundation.okxInstrumentMasterSnapshot", {
												id: shortId(latest.snapshotId),
											})}
										</span>
									</div>
									<div className="max-h-72 overflow-auto rounded-md border">
										<table className="w-full text-left text-xs">
											<thead className="sticky top-0 bg-muted">
												<tr>
													<th className="p-2">{t("dataFoundation.okxColumnCode")}</th>
													<th className="p-2">{t("dataFoundation.okxColumnAssets")}</th>
													<th className="p-2">{t("dataFoundation.okxColumnVolume")}</th>
													<th className="p-2">{t("dataFoundation.okxColumnStatus")}</th>
													<th className="p-2">{t("dataFoundation.okxColumnMinimum")}</th>
												</tr>
											</thead>
											<tbody>
												{latest.instruments.map((instrument) => (
													<tr key={instrument.code} className="border-t">
														<td className="p-2 font-medium">{instrument.code}</td>
														<td className="p-2">
															{instrument.baseAsset}/{instrument.quoteAsset}
														</td>
														<td className="p-2">
															{humanizeNumber(latest.quoteVolume24hUsdt?.[instrument.code])}
														</td>
														<td className="p-2">{instrument.status}</td>
														<td className="p-2">
															{humanizeNumber(instrument.minimumQuantity)}
														</td>
													</tr>
												))}
											</tbody>
										</table>
									</div>
									<p className="text-xs text-muted-foreground">
										{t("dataFoundation.okxInstrumentMasterUse")}
									</p>
									<details className="rounded-md border p-3">
										<summary className="cursor-pointer text-sm font-medium">
											{t("dataFoundation.okxInstrumentMasterHistory", {
												count: instrumentMasterQuery.data.length,
											})}
										</summary>
										<div className="mt-3 max-h-48 overflow-auto rounded-md border">
											<table className="w-full text-left text-xs">
												<thead className="bg-muted">
													<tr>
														<th className="p-2">
															{t("dataFoundation.okxInstrumentMasterSnapshotHeader")}
														</th>
														<th className="p-2">
															{t("dataFoundation.okxInstrumentMasterRetrievedHeader")}
														</th>
														<th className="p-2">
															{t("dataFoundation.okxInstrumentMasterCountLabel")}
														</th>
														<th className="p-2">
															{t("dataFoundation.okxInstrumentMasterFilterHeader")}
														</th>
													</tr>
												</thead>
												<tbody>
													{[...instrumentMasterQuery.data].reverse().map((snapshot) => (
														<tr key={snapshot.snapshotId} className="border-t">
															<td className="p-2 font-mono">{shortId(snapshot.snapshotId)}</td>
															<td className="p-2">
																{new Date(snapshot.retrievedAtMs).toLocaleString()}
															</td>
															<td className="p-2">
																{humanizeNumber(snapshot.instruments.length)}
															</td>
															<td className="p-2">
																{snapshot.ignoreUntradable ? "live · " : "all · "}
																{humanizeNumber(snapshot.minimumQuoteVolume24h)}
															</td>
														</tr>
													))}
												</tbody>
											</table>
										</div>
									</details>
								</div>
							);
						})()
					) : (
						<p className="text-sm text-muted-foreground">
							{t("dataFoundation.okxInstrumentMasterEmpty")}
						</p>
					)}
				</CardContent>
			</Card>
			<Card>
				<CardHeader>
					<CardTitle>{t("dataFoundation.selectContextTitle")}</CardTitle>
					<CardDescription>
						{t("dataFoundation.selectContextDescription")}
					</CardDescription>
				</CardHeader>
				<CardContent className="grid gap-3 md:grid-cols-2 lg:grid-cols-4">
					<label className="grid gap-1 text-sm">
						<span>{t("dataFoundation.contextMarket")}</span>
						<select
							className="h-9 w-full min-w-0 rounded-md border bg-background px-3"
							value={contextMarket}
							onChange={(event) => {
								const next = event.target.value as FoundationMarket["id"];
								setContextMarket(next);
								setContextVenue(MARKET_VENUES[next][0]);
							}}
						>
							{markets.map((market) => (
								<option key={market.id} value={market.id}>
									{t(market.titleKey)}
								</option>
							))}
						</select>
					</label>
					<label className="grid gap-1 text-sm">
						<span>{t("dataFoundation.contextVenue")}</span>
						<select
							className="h-9 w-full min-w-0 rounded-md border bg-background px-3"
							value={contextVenue}
							onChange={(event) => setContextVenue(event.target.value)}
						>
							{MARKET_VENUES[contextMarket].map((venue) => (
								<option key={venue}>{venue}</option>
							))}
						</select>
					</label>
					<label className="grid gap-1 text-sm">
						<span>{t("dataFoundation.contextSnapshot")}</span>
						<select
							className="h-9 w-full min-w-0 rounded-md border bg-background px-3"
							value={snapshotId}
							onChange={(event) => setSnapshotId(event.target.value)}
						>
							<option value="">{t("dataFoundation.selectSnapshot")}</option>
							{snapshotsQuery.data?.map((snapshot) => (
								<option key={snapshot.snapshotId} value={snapshot.snapshotId}>
									{snapshot.code} · {snapshot.interval} · {snapshot.barCount}
								</option>
							))}
						</select>
					</label>
					<label className="grid gap-1 text-sm">
						<span>{t("dataFoundation.contextUniverse")}</span>
						<select
							className="h-9 w-full min-w-0 rounded-md border bg-background px-3"
							value={universeId}
							onChange={(event) => setUniverseId(event.target.value)}
						>
							<option value="">{t("dataFoundation.selectUniverse")}</option>
							{universeQuery.data?.map((universe) => (
								<option key={universe.snapshotId} value={universe.snapshotId}>
									{universe.snapshotId} · {universe.contentSha256.slice(0, 8)}
								</option>
							))}
						</select>
					</label>
					<label className="grid gap-1 text-sm" htmlFor="context-range-start">
						<span>{t("dataFoundation.rangeStart")}</span>
						<Input
							id="context-range-start"
							type="date"
							value={rangeStart}
							onChange={(event) => setRangeStart(event.target.value)}
						/>
					</label>
					<label className="grid gap-1 text-sm" htmlFor="context-range-end">
						<span>{t("dataFoundation.rangeEnd")}</span>
						<Input
							id="context-range-end"
							type="date"
							value={rangeEnd}
							onChange={(event) => setRangeEnd(event.target.value)}
						/>
					</label>
					<button
						type="button"
						className="h-9 rounded-md bg-primary px-3 text-sm font-medium text-primary-foreground disabled:opacity-50 md:col-span-2 lg:col-span-2 lg:justify-self-start"
						disabled={!snapshotId || snapshotsQuery.isPending}
						onClick={() => void establishContext()}
					>
						{t("dataFoundation.establishContext")}
					</button>
					{snapshotsQuery.error || universeQuery.error ? (
						<p className="text-sm text-destructive" role="alert">
							{getErrorMessage(snapshotsQuery.error ?? universeQuery.error)}
						</p>
					) : null}
				</CardContent>
			</Card>
			<Card>
				<CardHeader>
					<CardTitle>{t("dataFoundation.publicationTitle")}</CardTitle>
					<CardDescription>
						{t("dataFoundation.publicationDescription")}
					</CardDescription>
				</CardHeader>
				<CardContent className="grid gap-3">
					<div className="grid gap-3 rounded-md border p-3 text-sm sm:grid-cols-3">
						<label className="grid gap-1">
							<span>{t("dataFoundation.backfillScope")}</span>
							<select
								className="h-9 rounded-md border bg-background px-3"
								value={backfillScope}
								disabled={Boolean(backfillTaskId)}
								onChange={(event) =>
									setBackfillScope(event.target.value as "selected" | "all")
								}
							>
								<option value="selected">{t("dataFoundation.backfillSelected")}</option>
								<option value="all">{t("dataFoundation.backfillAll")}</option>
							</select>
						</label>
						<label
							className="grid gap-1 sm:col-span-2"
							htmlFor="okx-backfill-instruments"
						>
							<span>{t("dataFoundation.backfillInstruments")}</span>
							<Input
								id="okx-backfill-instruments"
								value={instrumentCodes}
								disabled={backfillScope === "all" || Boolean(backfillTaskId)}
								placeholder="BTC-USDT, ETH-USDT"
								onChange={(event) => setInstrumentCodes(event.target.value)}
							/>
						</label>
						<label className="grid gap-1">
							<span>{t("dataFoundation.backfillInterval")}</span>
							<select
								className="h-9 rounded-md border bg-background px-3"
								value={backfillInterval}
								disabled={Boolean(backfillTaskId)}
								onChange={(event) =>
									setBackfillInterval(event.target.value as OkxInterval)
								}
							>
								{OKX_INTERVALS.map((interval) => (
									<option key={interval} value={interval}>
										{interval}
									</option>
								))}
							</select>
						</label>
					</div>
					<div className="flex flex-wrap items-center gap-3 rounded-md border p-3">
						<div className="min-w-0 flex-1">
							<strong className="text-sm">
								{t("dataFoundation.okxBackfillTitle")}
							</strong>
							<p className="text-xs text-muted-foreground">
								{t("dataFoundation.okxBackfillDescription")}
							</p>
							{backfillProgress ? (
								<div className="grid gap-1" role="status">
									<p className="text-xs text-muted-foreground">
										{backfillProgress}
										{backfillStats?.currentInstrument
											? ` · ${backfillStats.currentInstrument}`
											: ""}
									</p>
									{backfillStats?.instrumentCount ? (
										<>
											<progress
												className="h-2 w-full"
												max={backfillStats.instrumentCount}
												value={backfillStats.completedInstruments}
												aria-label="OKX backfill progress"
											/>
											<span className="text-xs text-muted-foreground">
												{backfillStats.completedInstruments}/{backfillStats.instrumentCount}{" "}
												instruments
											</span>
										</>
									) : null}
								</div>
							) : null}
						</div>
						<button
							type="button"
							className="rounded-md bg-primary px-3 py-2 text-xs font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
							disabled={Boolean(activeOperation || backfillTaskId)}
							onClick={() => void runOkxBackfill()}
						>
							{backfillTaskId
								? t("dataFoundation.okxBackfillRunning")
								: t("dataFoundation.okxBackfillStart")}
						</button>
						{backfillTaskId ? (
							<button
								type="button"
								className="rounded-md border px-3 py-2 text-xs font-medium hover:bg-muted"
								onClick={() => void cancelOkxBackfill()}
							>
								{t("dataFoundation.okxBackfillCancel")}
							</button>
						) : null}
						{savedBackfill && !backfillTaskId ? (
							<button
								type="button"
								className="rounded-md border px-3 py-2 text-xs font-medium hover:bg-muted"
								onClick={() => void runOkxBackfill(savedBackfill)}
							>
								{t("dataFoundation.backfillResume")}
							</button>
						) : null}
					</div>
					{backfillTaskId && backfillStats ? (
						<div className="grid gap-1 rounded-md border p-3 text-xs text-muted-foreground">
							<div className="flex justify-between gap-2">
								<span>{t("dataFoundation.backfillElapsed")}</span>
								<span>{elapsedLabel}</span>
							</div>
							<div className="flex justify-between gap-2">
								<span>{t("dataFoundation.backfillEta")}</span>
								<span>{etaLabel}</span>
							</div>
							<div className="flex justify-between gap-2">
								<span>{t("dataFoundation.backfillRecords")}</span>
								<span>{backfillStats.downloadedRecords}</span>
							</div>
						</div>
					) : null}
					{pipelineQuery.data?.length ? (
						<>
							{publicationSlice.map((dataset) => (
								<div
									key={dataset.sourceId}
									className="grid gap-2 rounded-md border p-3"
								>
									<button
										type="button"
										className={`grid gap-1 text-left text-sm ${selectedSourceId === dataset.sourceId ? "text-primary" : ""}`}
										onClick={() => setSelectedSourceId(dataset.sourceId)}
									>
										<span className="font-medium">{dataset.sourceId}</span>
										<span className="text-muted-foreground">
											{t("dataFoundation.publicationCounts", {
												source: dataset.sourceRecordCount,
												canonical: dataset.canonicalRecordCount,
												quarantined: dataset.quarantinedRecordCount,
												gaps: dataset.gapCount,
											})}
										</span>
									</button>
									<button
										type="button"
										className="justify-self-start rounded-md border px-2 py-1 text-xs hover:bg-muted disabled:opacity-50"
										disabled={!dataset.canonicalId || Boolean(publishingId)}
										onClick={() => void publish(dataset, dataset.state === "degraded")}
									>
										{publishingId === dataset.canonicalId
											? t("dataFoundation.publishing")
											: dataset.state === "degraded"
												? t("dataFoundation.acceptDegradedPublish")
												: t("dataFoundation.publishSnapshot")}
									</button>
								</div>
							))}
							<Pagination
								page={publicationSafePage}
								pageCount={publicationPageCount}
								onPageChange={setPublicationPage}
							/>
						</>
					) : (
						<p className="text-sm text-muted-foreground">
							{t("dataFoundation.emptyPublication")}
						</p>
					)}
					{qualityQuery.data ? (
						<div className="rounded-md border p-3 text-sm">
							<div className="flex flex-wrap items-center justify-between gap-2">
								<strong>{t("dataFoundation.qualityDetail")}</strong>
								<Badge
									variant={qualityQuery.data.state === "passed" ? "default" : "outline"}
								>
									{t(`dataFoundation.states.${qualityQuery.data.state}`)}
								</Badge>
							</div>
							<p className="mt-2 text-muted-foreground">
								{t("dataFoundation.qualityCounts", qualityQuery.data)}
							</p>
							{qualityQuery.data.state !== "passed" ? (
								<div className="mt-2 text-amber-700 dark:text-amber-300" role="status">
									<p>{t("dataFoundation.downstreamBlocked")}</p>
									{qualityQuery.data.reasons.length ? (
										<ul className="mt-1 list-disc pl-5">
											{qualityQuery.data.reasons.map((reason) => (
												<li key={reason.code}>
													{reason.code}: {reason.message}
												</li>
											))}
										</ul>
									) : null}
								</div>
							) : null}
						</div>
					) : null}
				</CardContent>
			</Card>
			<Card>
				<CardHeader>
					<CardTitle>{t("dataFoundation.contextTitle")}</CardTitle>
					<CardDescription>{t("dataFoundation.contextDescription")}</CardDescription>
				</CardHeader>
				<CardContent>
					{contextQuery.data ? (
						<div className="grid gap-2 text-sm sm:grid-cols-3">
							<ReadinessStat
								label={t("dataFoundation.contextState")}
								value={contextQuery.data.contextRevision}
								loading={false}
							/>
							<ContextField
								label={t("dataFoundation.contextMarket")}
								value={`${contextQuery.data.market} · ${contextQuery.data.venue}`}
							/>
							<ContextField
								label={t("dataFoundation.contextSnapshot")}
								value={contextQuery.data.snapshotId}
							/>
						</div>
					) : (
						<p className="text-sm text-muted-foreground">
							{t("dataFoundation.contextEmpty")}
						</p>
					)}
				</CardContent>
			</Card>
			<Card>
				<CardHeader>
					<CardTitle>{t("dataFoundation.operationHistoryTitle")}</CardTitle>
					<CardDescription>
						{t("dataFoundation.operationHistoryDescription")}
					</CardDescription>
				</CardHeader>
				<CardContent>
					{acquisitionQuery.isPending ? (
						<p className="text-sm text-muted-foreground" role="status">
							{t("dataFoundation.loadingHistory")}
						</p>
					) : acquisitionQuery.data?.length ? (
						<div className="grid gap-3">
							<strong className="text-sm">
								{t("dataFoundation.executionStatus")}
							</strong>
							<div className="grid gap-2">
								{acquisitionSlice.map((operation) => (
									<div
										key={`${operation.instrument.venue.id}:${operation.instrument.code}:${operation.interval}`}
										className="flex flex-wrap items-center justify-between gap-3 rounded-md border p-3 text-sm"
									>
										<div>
											<strong>{operation.instrument.code}</strong>
											<span className="ml-2 text-muted-foreground">
												{operation.interval} ·{" "}
												{t("dataFoundation.pages", { count: operation.pages })} ·{" "}
												{t("dataFoundation.revision", {
													revision: operation.revision ?? "—",
												})}{" "}
												· {t("dataFoundation.retries", { count: operation.retryCount })}
											</span>
											{operation.lastError ? (
												<p className="mt-1 text-destructive">{operation.lastError}</p>
											) : null}
										</div>
										<div className="flex items-center gap-2">
											<Badge
												variant={operation.state === "completed" ? "default" : "outline"}
											>
												{t(`dataFoundation.states.${operation.state}`)}
											</Badge>
											{operation.state === "failed" || operation.state === "cancelled" ? (
												<button
													type="button"
													className="rounded-md border px-2 py-1 text-xs hover:bg-muted disabled:opacity-50"
													onClick={() => void retry(operation)}
													disabled={Boolean(activeOperation)}
												>
													{t("dataFoundation.retryAcquisition")}
												</button>
											) : null}
										</div>
									</div>
								))}
							</div>
							<Pagination
								page={acquisitionSafePage}
								pageCount={acquisitionPageCount}
								onPageChange={setAcquisitionPage}
							/>
						</div>
					) : (
						<p className="text-sm text-muted-foreground">
							{t("dataFoundation.emptyHistory")}
						</p>
					)}
					<AcquisitionOperationHistory
						loading={foundationHistoryQuery.isPending}
						operations={foundationHistoryQuery.data ?? []}
						onCancel={async (operation) => {
							if (
								!userId ||
								operation.market !== "crypto" ||
								operation.venue !== "okx"
							)
								return;
							try {
								await invoke("okx_backfill_cancel", {
									request: { userId, taskId: operation.operationId },
								});
								await Promise.all([
									acquisitionQuery.refetch(),
									foundationHistoryQuery.refetch(),
								]);
							} catch (cause) {
								setError(getErrorMessage(cause));
							}
						}}
						onRetry={(operation) => {
							const market = markets.find((item) => item.id === operation.market);
							if (market) void acquire(market);
						}}
					/>
				</CardContent>
			</Card>
			{pipelineQuery.data?.some(
				(item) => item.state === "degraded" || item.state === "rejected",
			) ? (
				<p className="text-sm text-amber-700 dark:text-amber-300" role="status">
					<CircleAlertIcon className="mr-1 inline size-4" aria-hidden="true" />
					{t("dataFoundation.qualityWarning")}
				</p>
			) : null}
		</div>
	);
}

function AcquisitionOperationHistory({
	loading,
	operations,
	onCancel,
	onRetry,
}: {
	loading: boolean;
	operations: FoundationAcquisitionView[];
	onCancel: (operation: FoundationAcquisitionView) => void;
	onRetry: (operation: FoundationAcquisitionView) => void;
}) {
	const { t } = useTranslation();
	const [page, setPage] = useState(1);
	const orderedOperations = [...operations].reverse();
	const pageCount = Math.max(1, Math.ceil(orderedOperations.length / PAGE_SIZE));
	const safePage = Math.min(page, pageCount);
	const slice = orderedOperations.slice(
		(safePage - 1) * PAGE_SIZE,
		safePage * PAGE_SIZE,
	);
	return (
		<div className="mt-4 grid gap-2 border-t pt-4">
			<strong className="text-sm">{t("dataFoundation.operationLedger")}</strong>
			{loading ? (
				<p className="text-sm text-muted-foreground" role="status">
					{t("dataFoundation.loadingHistory")}
				</p>
			) : operations.length ? (
				<>
					{slice.map((operation) => (
						<div
							key={operation.operationId}
							className="flex flex-wrap items-center justify-between gap-2 rounded-md border p-3 text-sm"
						>
							<div>
								<strong>{operation.market}</strong>
								<span className="ml-2 text-muted-foreground">{operation.venue}</span>
								<p className="text-xs text-muted-foreground">{operation.operationId}</p>
								{operation.error ? (
									<p className="text-destructive">{operation.error}</p>
								) : null}
							</div>
							<div className="flex items-center gap-2">
								<Badge
									variant={operation.state === "completed" ? "default" : "outline"}
								>
									{t(`dataFoundation.states.${operation.state}`)}
								</Badge>
								{operation.state === "failed" || operation.state === "cancelled" ? (
									<button
										type="button"
										className="rounded-md border px-2 py-1 text-xs hover:bg-muted"
										onClick={() => onRetry(operation)}
									>
										{t("dataFoundation.retryAcquisition")}
									</button>
								) : null}
								{operation.state === "running" &&
								operation.market === "crypto" &&
								operation.venue === "okx" ? (
									<button
										type="button"
										className="rounded-md border px-2 py-1 text-xs hover:bg-muted"
										onClick={() => onCancel(operation)}
									>
										{t("dataFoundation.okxBackfillCancel")}
									</button>
								) : null}
							</div>
						</div>
					))}
					<Pagination page={safePage} pageCount={pageCount} onPageChange={setPage} />
				</>
			) : (
				<p className="text-sm text-muted-foreground">
					{t("dataFoundation.emptyHistory")}
				</p>
			)}
		</div>
	);
}

function ContextField({ label, value }: { label: string; value: string }) {
	return (
		<div className="rounded-md border p-3">
			<div className="text-xs text-muted-foreground">{label}</div>
			<code className="block break-all text-sm">{value}</code>
		</div>
	);
}

function Pagination({
	page,
	pageCount,
	onPageChange,
}: {
	page: number;
	pageCount: number;
	onPageChange: (next: number) => void;
}) {
	const { t } = useTranslation();
	if (pageCount <= 1) return null;
	return (
		<div className="flex items-center justify-between gap-2 pt-2">
			<span className="text-xs text-muted-foreground">
				{t("dataFoundation.paginationPage", { current: page, total: pageCount })}
			</span>
			<div className="flex items-center gap-2">
				<button
					type="button"
					className="rounded-md border px-2 py-1 text-xs hover:bg-muted disabled:opacity-50"
					disabled={page <= 1}
					onClick={() => onPageChange(page - 1)}
				>
					{t("dataFoundation.paginationPrev")}
				</button>
				<button
					type="button"
					className="rounded-md border px-2 py-1 text-xs hover:bg-muted disabled:opacity-50"
					disabled={page >= pageCount}
					onClick={() => onPageChange(page + 1)}
				>
					{t("dataFoundation.paginationNext")}
				</button>
			</div>
		</div>
	);
}

function ReadinessStat({
	label,
	value,
	loading,
}: {
	label: string;
	value: number;
	loading: boolean;
}) {
	return (
		<div className="rounded-md border p-3">
			<div className="flex items-center justify-between gap-2 text-sm text-muted-foreground">
				<span>{label}</span>
				{loading ? (
					<LoaderCircleIcon className="size-4 animate-spin" aria-label={label} />
				) : (
					<CheckCircle2Icon className="size-4" aria-hidden="true" />
				)}
			</div>
			<strong className="text-2xl">{loading ? "…" : value}</strong>
		</div>
	);
}
