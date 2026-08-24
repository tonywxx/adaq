import { Badge } from "@/components/ui/badge";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { formatDateTime } from "@/lib/i18n";
import { getErrorMessage, useMarketSessionStore } from "@/lib/market-session";
import { Link } from "@tanstack/react-router";
import { Channel, invoke } from "@tauri-apps/api/core";
import {
	ArrowRightIcon,
	CheckCircle2Icon,
	DatabaseIcon,
	LoaderCircleIcon,
} from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { useQuery } from "@tanstack/react-query";
import { toast } from "sonner";

type PipelineDatasetSummary = {
	sourceId: string;
	source: SourceEvidenceSummary;
	canonicalId?: string;
	qualityReportId?: string;
	revision: number;
	state: "unassessed" | "passed" | "degraded" | "rejected";
	sourceRecordCount: number;
	canonicalRecordCount: number;
	quarantinedRecordCount: number;
	gapCount: number;
};

type GateTwoRequest = {
	sourceIds: string[];
	startTimeMs: number;
	endTimeMs: number;
	interval: OkxInterval;
	instrumentCodes: string[];
};

type GateTwoPublicationView = {
	publications: Array<{ sourceId: string; canonicalId?: string }>;
	universeSnapshotId: string;
};

type SourceEvidenceSummary = {
	logicalKey: string;
	provider: string;
	actualUpstream?: string;
	connector: string;
	connectorVersion: string;
	requestParameters: unknown;
	retrievedAtMs: number;
	responseSha256s: string[];
	acquisitionContentSha256?: string;
	payloadSha256: string;
	contentSha256: string;
	capabilitySnapshot: unknown;
	instrument?: { venue: { id: string }; code: string };
	interval?: string;
	requestedStartTimeMs?: number;
	requestedEndTimeMs?: number;
	receivedStartTimeMs?: number;
	receivedEndTimeMs?: number;
	requestCount: number;
	retryCount: number;
	responseStatuses: number[];
	notes: string[];
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
	operationId?: string;
	instrument: { venue: { id: string }; code: string };
	interval: string;
	startTimeMs?: number;
	endTimeMs?: number;
	coverageStartMs?: number;
	coverageEndMs?: number;
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
	sourceId?: string;
	nextCursorMs?: number;
	retryCount: number;
	lastErrorCode?: string;
	lastError?: string;
	provider: string;
	actualUpstream: string;
	connector: string;
	connectorVersion: string;
	requestParameters: unknown;
	capabilitySnapshot: unknown;
	updatedAtMs?: number;
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

const formatTimestamp = (value: number | undefined) =>
	value == null ? "—" : formatDateTime(value, { timeZone: "UTC" });

type ProvenanceItem =
	| {
			kind: "dataset";
			sourceId: string;
			revision: number;
			source: SourceEvidenceSummary;
			operation?: OkxAcquisitionStatus;
	  }
	| { kind: "operation"; operation: OkxAcquisitionStatus };

function SourceProvenancePanel({
	datasets,
	operations,
	t,
}: {
	datasets: PipelineDatasetSummary[];
	operations: OkxAcquisitionStatus[];
	t: (key: string, options?: Record<string, unknown>) => string;
}) {
	const unresolvedOperations = operations.filter(
		(operation) => !operation.sourceId,
	);
	const [page, setPage] = useState(1);
	if (!datasets.length && !unresolvedOperations.length) {
		return (
			<p className="text-sm text-muted-foreground">
				{t("dataFoundation.sourceProvenanceEmpty")}
			</p>
		);
	}
	const items: ProvenanceItem[] = [
		...datasets.map((dataset) => ({
			kind: "dataset" as const,
			sourceId: dataset.sourceId,
			revision: dataset.revision,
			source: dataset.source,
			operation: operations.find((item) => item.sourceId === dataset.sourceId),
		})),
		...unresolvedOperations.map((operation) => ({
			kind: "operation" as const,
			operation,
		})),
	];
	const PROVENANCE_PAGE_SIZE = 6;
	const pageCount = Math.max(1, Math.ceil(items.length / PROVENANCE_PAGE_SIZE));
	const safePage = Math.min(page, pageCount);
	const slice = items.slice(
		(safePage - 1) * PROVENANCE_PAGE_SIZE,
		safePage * PROVENANCE_PAGE_SIZE,
	);
	return (
		<div className="grid gap-3">
			{slice.map((item) =>
				item.kind === "dataset" ? (
					<details key={item.sourceId} className="rounded-md border p-3">
						<summary className="cursor-pointer text-sm font-medium">
							{item.source.provider} · {item.source.instrument?.code ?? "—"} ·{" "}
							{item.source.interval ?? "—"}
						</summary>
						<div className="mt-3 grid gap-3 text-xs sm:grid-cols-2">
							<ContextField
								label={t("dataFoundation.sourceProvider")}
								value={`${item.source.provider} · ${item.source.actualUpstream ?? "—"}`}
							/>
							<ContextField
								label={t("dataFoundation.sourceCapability")}
								value={JSON.stringify(item.source.capabilitySnapshot)}
							/>
							<ContextField
								label={t("dataFoundation.sourceRequest")}
								value={JSON.stringify(item.source.requestParameters)}
							/>
							<ContextField
								label={t("dataFoundation.sourceInstrument")}
								value={`${item.source.instrument?.venue.id ?? "—"} · ${item.source.instrument?.code ?? "—"} · ${item.source.interval ?? "—"}`}
							/>
							<ContextField
								label={t("dataFoundation.sourceRequestedRange")}
								value={`${formatTimestamp(item.source.requestedStartTimeMs)} — ${formatTimestamp(item.source.requestedEndTimeMs)}`}
							/>
							<ContextField
								label={t("dataFoundation.sourceReceivedRange")}
								value={`${formatTimestamp(item.source.receivedStartTimeMs)} — ${formatTimestamp(item.source.receivedEndTimeMs)}`}
							/>
							<ContextField
								label={t("dataFoundation.sourceRevision")}
								value={`${item.revision} · ${item.sourceId}`}
							/>
							<ContextField
								label={t("dataFoundation.sourceLogicalKey")}
								value={item.source.logicalKey}
							/>
							<ContextField
								label={t("dataFoundation.sourceHashes")}
								value={[
									item.source.contentSha256,
									item.source.payloadSha256,
									item.source.acquisitionContentSha256,
									...item.source.responseSha256s,
								]
									.filter(Boolean)
									.join("\n")}
							/>
							<ContextField
								label={t("dataFoundation.sourceRetrieved")}
								value={formatTimestamp(item.source.retrievedAtMs)}
							/>
							<ContextField
								label={t("dataFoundation.sourceContinuation")}
								value={t("dataFoundation.sourceRequests", {
									count: item.source.requestCount,
									retries: item.operation?.retryCount ?? item.source.retryCount,
									statuses: item.source.responseStatuses.join(", ") || "—",
									pages: item.operation?.pages ?? 0,
									cursor: formatTimestamp(item.operation?.nextCursorMs),
								})}
							/>
							{item.source.notes.length ? (
								<ContextField
									label={t("dataFoundation.sourceNotes")}
									value={item.source.notes.join("\n")}
								/>
							) : null}
							{item.operation?.lastError ? (
								<ContextField
									label={t("dataFoundation.sourceNotes")}
									value={`${item.operation.lastErrorCode ?? t("dataFoundation.unknownErrorCode")}: ${item.operation.lastError}`}
								/>
							) : null}
						</div>
					</details>
				) : (
					<details
						key={
							item.operation.operationId ??
							`${item.operation.instrument.code}-${item.operation.state}`
						}
						className="rounded-md border p-3"
					>
						<summary className="cursor-pointer text-sm font-medium">
							{item.operation.provider} · {item.operation.instrument.code} ·{" "}
							{item.operation.interval}
						</summary>
						<div className="mt-3 grid gap-3 text-xs sm:grid-cols-2">
							<ContextField
								label={t("dataFoundation.sourceProvider")}
								value={`${item.operation.provider} · ${item.operation.actualUpstream}`}
							/>
							<ContextField
								label={t("dataFoundation.sourceCapability")}
								value={JSON.stringify(item.operation.capabilitySnapshot)}
							/>
							<ContextField
								label={t("dataFoundation.sourceRequest")}
								value={JSON.stringify(item.operation.requestParameters)}
							/>
							<ContextField
								label={t("dataFoundation.sourceInstrument")}
								value={`${item.operation.instrument.venue.id} · ${item.operation.instrument.code} · ${item.operation.interval}`}
							/>
							<ContextField
								label={t("dataFoundation.sourceRequestedRange")}
								value={`${formatTimestamp(item.operation.startTimeMs)} — ${formatTimestamp(item.operation.endTimeMs)}`}
							/>
							<ContextField
								label={t("dataFoundation.sourceReceivedRange")}
								value={`${formatTimestamp(item.operation.coverageStartMs)} — ${formatTimestamp(item.operation.coverageEndMs)}`}
							/>
							<ContextField
								label={t("dataFoundation.sourceRevision")}
								value={`${item.operation.revision ?? "—"} · ${item.operation.sourceId ?? "—"}`}
							/>
							<ContextField label={t("dataFoundation.sourceHashes")} value="—" />
							<ContextField
								label={t("dataFoundation.sourceRetrieved")}
								value={formatTimestamp(item.operation.updatedAtMs)}
							/>
							<ContextField
								label={t("dataFoundation.sourceContinuation")}
								value={t("dataFoundation.sourceRequests", {
									count: "—",
									retries: item.operation.retryCount,
									statuses: "—",
									pages: item.operation.pages,
									cursor: formatTimestamp(item.operation.nextCursorMs),
								})}
							/>
							{item.operation.lastError ? (
								<ContextField
									label={t("dataFoundation.sourceNotes")}
									value={`${item.operation.lastErrorCode ?? t("dataFoundation.unknownErrorCode")}: ${item.operation.lastError}`}
								/>
							) : null}
						</div>
					</details>
				),
			)}
			<Pagination page={safePage} pageCount={pageCount} onPageChange={setPage} />
		</div>
	);
}

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
	scope: "selected" | "watchlist" | "all";
	instrumentCodes: string[];
	instrumentMasterSnapshotId?: string;
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

function StepTitle({ step, children }: { step: number; children: ReactNode }) {
	return (
		<CardTitle className="flex items-center gap-2">
			<span className="inline-flex size-7 items-center justify-center rounded-full bg-primary text-sm font-semibold text-primary-foreground">
				{step}
			</span>
			{children}
		</CardTitle>
	);
}

function InstrumentEvidencePanel({
	snapshots,
	pending,
	t,
}: {
	snapshots?: InstrumentMasterSnapshot[];
	pending: boolean;
	t: (key: string, options?: Record<string, unknown>) => string;
}) {
	const [selectedSnapshotId, setSelectedSnapshotId] = useState<string>();
	if (pending)
		return (
			<p className="text-sm text-muted-foreground">
				{t("dataFoundation.loadingHistory")}
			</p>
		);
	const latest =
		snapshots?.find((snapshot) => snapshot.snapshotId === selectedSnapshotId) ??
		snapshots?.at(-1);
	if (!latest)
		return (
			<p className="text-sm text-muted-foreground">
				{t("dataFoundation.okxInstrumentMasterEmpty")}
			</p>
		);
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
						date: formatDateTime(latest.retrievedAtMs),
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
								<td className="p-2">{humanizeNumber(instrument.minimumQuantity)}</td>
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
						count: snapshots?.length ?? 0,
					})}
				</summary>
				<div className="mt-3 max-h-48 overflow-auto">
					<table className="w-full text-left text-xs">
						<thead className="sticky top-0 bg-muted">
							<tr>
								<th className="p-2">
									{t("dataFoundation.okxInstrumentMasterSnapshotHeader")}
								</th>
								<th className="p-2">
									{t("dataFoundation.okxInstrumentMasterRetrievedHeader")}
								</th>
								<th className="p-2">
									{t("dataFoundation.okxInstrumentMasterCountHeader")}
								</th>
							</tr>
						</thead>
						<tbody>
							{[...(snapshots ?? [])].reverse().map((snapshot) => (
								<tr
									key={snapshot.snapshotId}
									className={`cursor-pointer border-t hover:bg-muted/50 ${snapshot.snapshotId === latest.snapshotId ? "bg-primary/10 font-medium ring-1 ring-inset ring-primary/30" : ""}`}
									aria-current={snapshot.snapshotId === latest.snapshotId}
									onClick={() => setSelectedSnapshotId(snapshot.snapshotId)}
									onKeyDown={(event) => {
										if (event.key === "Enter" || event.key === " ") {
											event.preventDefault();
											setSelectedSnapshotId(snapshot.snapshotId);
										}
									}}
									tabIndex={0}
								>
									<td className="p-2 font-mono">{shortId(snapshot.snapshotId)}</td>
									<td className="p-2">{formatDateTime(snapshot.retrievedAtMs)}</td>
									<td className="p-2">{humanizeNumber(snapshot.instruments.length)}</td>
								</tr>
							))}
						</tbody>
					</table>
				</div>
			</details>
		</div>
	);
}

const PAGE_SIZE = 5;

export function DataFoundationPage() {
	const { t } = useTranslation();
	const userId = useMarketSessionStore((state) => state.userId);
	const [activeOperation, setActiveOperation] = useState<string>();
	const [backfillTaskId, setBackfillTaskId] = useState<string>();
	const [backfillProgress, setBackfillProgress] = useState<string>();
	const [backfillStats, setBackfillStats] = useState<OkxBackfillProgress>();
	const [backfillScope, setBackfillScope] = useState<
		"selected" | "watchlist" | "all"
	>("watchlist");
	const [instrumentCodes, setInstrumentCodes] = useState("BTC-USDT, ETH-USDT");
	const watchlist = useMarketSessionStore((state) => state.watchlist);
	const watchlistCodes = (watchlist ?? [])
		.filter((instrument) => instrument.src === "okx")
		.map((instrument) => instrument.code);
	const [backfillInterval, setBackfillInterval] = useState<OkxInterval>("1h");
	const [selectedBackfillSnapshotId, setSelectedBackfillSnapshotId] =
		useState<string>();
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
	const [publishingSourceId, setPublishingSourceId] = useState<string>();
	const [gateTwoRequest, setGateTwoRequest] = useState<GateTwoRequest>();
	const [publishingGateTwo, setPublishingGateTwo] = useState(false);
	const [publicationPage, setPublicationPage] = useState(1);
	const [acquisitionPage, setAcquisitionPage] = useState(1);
	const [rangeStart, setRangeStart] = useState(() =>
		new Date(Date.now() - 30 * 86_400_000).toISOString().slice(0, 10),
	);
	const [rangeEnd, setRangeEnd] = useState(() =>
		new Date(Date.now() - 86_400_000).toISOString().slice(0, 10),
	);
	const latestClosedDate = new Date(clockMs - 86_400_000)
		.toISOString()
		.slice(0, 10);
	useEffect(() => {
		const raw = localStorage.getItem("adaq.okx-backfill-draft");
		if (!raw) return;
		try {
			const parsed = JSON.parse(raw) as Partial<BackfillDraft>;
			setSavedBackfill({
				rangeStart: parsed.rangeStart ?? "",
				rangeEnd: parsed.rangeEnd ?? "",
				interval: parsed.interval ?? "1h",
				scope: parsed.scope ?? "watchlist",
				instrumentCodes: parsed.instrumentCodes ?? ["BTC-USDT", "ETH-USDT"],
				instrumentMasterSnapshotId: parsed.instrumentMasterSnapshotId,
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
	useEffect(() => {
		if (!error) return;
		toast.error(error);
		setError(undefined);
	}, [error]);
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
	useEffect(() => {
		const queryError = snapshotsQuery.error ?? universeQuery.error;
		if (queryError) toast.error(getErrorMessage(queryError));
	}, [snapshotsQuery.error, universeQuery.error]);
	const selectedDataset = pipelineQuery.data?.find(
		(dataset) => dataset.sourceId === selectedSourceId,
	);
	const qualityQuery = useQuery({
		queryKey: [
			"data-foundation-quality",
			userId,
			selectedDataset?.qualityReportId,
		],
		queryFn: () => {
			if (!selectedDataset?.qualityReportId)
				throw new Error("Data Quality Report is not published");
			return invoke<QualityView>("market_data_pipeline_quality", {
				request: { userId, evidenceId: selectedDataset.qualityReportId },
			});
		},
		enabled: Boolean(userId && selectedDataset?.qualityReportId),
		staleTime: 30_000,
	});
	const backfillSnapshots = [...(instrumentMasterQuery.data ?? [])]
		.filter(
			(snapshot) =>
				snapshot.retrievedAtMs <= Date.parse(`${rangeEnd}T00:00:00Z`) + 86_400_000,
		)
		.reverse()
		.slice(0, 10);
	const selectedBackfillSnapshot =
		backfillSnapshots.find(
			(snapshot) => snapshot.snapshotId === selectedBackfillSnapshotId,
		) ?? backfillSnapshots[0];

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

	const publishSource = async (dataset: PipelineDatasetSummary) => {
		const source = dataset.source;
		if (
			!userId ||
			!source.instrument ||
			!source.interval ||
			source.requestedStartTimeMs == null ||
			source.requestedEndTimeMs == null
		) {
			setError("Source evidence is missing its canonicalization request identity");
			return;
		}
		setError(undefined);
		setPublishingSourceId(dataset.sourceId);
		try {
			const onEvent = new Channel<OkxBackfillEvent>();
			await invoke("okx_publish_sources", {
				request: {
					userId,
					taskId: `crypto-gate-two-${crypto.randomUUID()}`,
					sourceIds: [dataset.sourceId],
					startTimeMs: source.requestedStartTimeMs,
					endTimeMs: source.requestedEndTimeMs,
					interval: source.interval,
					instrumentCodes: [source.instrument.code],
				},
				onEvent,
			});
			setSelectedSourceId(dataset.sourceId);
			setGateTwoRequest({
				sourceIds: [dataset.sourceId],
				startTimeMs: source.requestedStartTimeMs,
				endTimeMs: source.requestedEndTimeMs,
				interval: source.interval as OkxInterval,
				instrumentCodes: [source.instrument.code],
			});
			await pipelineQuery.refetch();
		} catch (cause) {
			setError(getErrorMessage(cause));
		} finally {
			setPublishingSourceId(undefined);
		}
	};

	const publishGateTwo = async () => {
		if (!userId || !gateTwoRequest) return;
		setError(undefined);
		setPublishingGateTwo(true);
		try {
			const onEvent = new Channel<OkxBackfillEvent>();
			const result = await invoke<GateTwoPublicationView>("okx_publish_gate_two", {
				request: {
					userId,
					taskId: `crypto-gate-two-${crypto.randomUUID()}`,
					...gateTwoRequest,
				},
				onEvent,
			});
			setUniverseId(result.universeSnapshotId);
			toast.success(
				t("dataFoundation.gateTwoPublished", {
					id: result.universeSnapshotId,
				}),
			);
			await Promise.all([
				pipelineQuery.refetch(),
				snapshotsQuery.refetch(),
				universeQuery.refetch(),
				foundationHistoryQuery.refetch(),
			]);
		} catch (cause) {
			setError(getErrorMessage(cause));
		} finally {
			setPublishingGateTwo(false);
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
			instrumentMasterSnapshotId: selectedBackfillSnapshot?.snapshotId,
			startedAtMs: Date.now(),
		};
		const selectedSnapshot = instrumentMasterQuery.data?.find(
			(snapshot) => snapshot.snapshotId === nextDraft.instrumentMasterSnapshotId,
		);
		const requestedCodes =
			nextDraft.scope === "watchlist"
				? watchlistCodes
				: nextDraft.scope === "all"
					? (selectedSnapshot?.instruments
							.filter((instrument) => instrument.status === "live")
							.map((instrument) => instrument.code) ?? [])
					: nextDraft.instrumentCodes;
		if (nextDraft.scope !== "all" && !requestedCodes.length) {
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
			const sources = await invoke<Array<{ sourceId: string }> | null>(
				"okx_backfill_source",
				{
					request: {
						userId,
						taskId,
						startTimeMs,
						endTimeMs,
						interval: nextDraft.interval,
						instrumentCodes: requestedCodes,
						universeSnapshotId: nextDraft.instrumentMasterSnapshotId,
					},
					onEvent,
				},
			);
			if (sources?.length) {
				setSelectedSourceId(sources[0].sourceId);
				setGateTwoRequest({
					sourceIds: sources.map((source) => source.sourceId),
					startTimeMs,
					endTimeMs,
					interval: nextDraft.interval,
					instrumentCodes: requestedCodes,
				});
			}
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
			const retrieved = formatDateTime(latest.retrievedAtMs, {
				second: "2-digit",
			});
			if (
				!window.confirm(
					t("dataFoundation.okxInstrumentMasterRefreshConfirm", {
						date: retrieved,
					}),
				)
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

	const retryOkxBackfill = async (operationId?: string) => {
		if (!userId || !operationId || backfillTaskId) return;
		const retryOperationId = `crypto-foundation-${crypto.randomUUID()}`;
		setError(undefined);
		setBackfillTaskId(retryOperationId);
		try {
			await invoke("okx_backfill_retry", {
				request: { userId, operationId, retryOperationId },
				onEvent: new Channel<OkxBackfillEvent>(),
			});
			await Promise.all([
				pipelineQuery.refetch(),
				acquisitionQuery.refetch(),
				foundationHistoryQuery.refetch(),
			]);
		} catch (cause) {
			setError(getErrorMessage(cause));
		} finally {
			setBackfillTaskId(undefined);
		}
	};
	const retry = async (operation: OkxAcquisitionStatus) => {
		if (operation.instrument.venue.id === "okx") {
			await retryOkxBackfill(operation.operationId);
		}
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

	const publicationItems = pipelineQuery.data ?? [];
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
			<Card>
				<CardHeader>
					<CardTitle>{t("dataFoundation.sourceProvenanceTitle")}</CardTitle>
					<CardDescription>
						{t("dataFoundation.sourceProvenanceDescription")}
					</CardDescription>
				</CardHeader>
				<CardContent>
					{pipelineQuery.isPending ? (
						<p className="text-sm text-muted-foreground" role="status">
							{t("dataFoundation.loadingHistory")}
						</p>
					) : (
						<SourceProvenancePanel
							datasets={pipelineQuery.data ?? []}
							operations={acquisitionQuery.data ?? []}
							t={t}
						/>
					)}
				</CardContent>
			</Card>
			<div className="grid gap-4 lg:grid-cols-3">
				{markets.map((market) => {
					const active = activeOperation?.startsWith(`${market.id}-`) ?? false;
					return (
						<Card key={market.id} className="h-fit lg:col-span-3">
							<CardHeader>
								<div className="flex items-start justify-between gap-3">
									<div>
										<StepTitle step={1}>{t(market.titleKey)}</StepTitle>
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
								<div className="w-full border-t pt-4">
									<p className="mb-3 text-sm font-medium">
										{t("dataFoundation.okxInstrumentMasterEvidenceTitle")}
									</p>
									<InstrumentEvidencePanel
										snapshots={instrumentMasterQuery.data}
										pending={instrumentMasterQuery.isPending}
										t={t}
									/>
								</div>
							</CardContent>
						</Card>
					);
				})}
			</div>
			<section className="hidden">
				<div className="pb-4">
					<CardTitle>
						{t("dataFoundation.okxInstrumentMasterEvidenceTitle")}
					</CardTitle>
					<CardDescription>
						{t("dataFoundation.okxInstrumentMasterEvidenceDescription")}
					</CardDescription>
				</div>
				<div>
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
												date: formatDateTime(latest.retrievedAtMs),
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
															<td className="p-2">{formatDateTime(snapshot.retrievedAtMs)}</td>
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
				</div>
			</section>
			<Card className="order-2">
				<CardHeader>
					<StepTitle step={3}>{t("dataFoundation.selectContextTitle")}</StepTitle>
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
					<button
						type="button"
						className="h-9 rounded-md bg-primary px-3 text-sm font-medium text-primary-foreground disabled:opacity-50 md:col-span-2 lg:col-span-2 lg:justify-self-start"
						disabled={!snapshotId || snapshotsQuery.isPending}
						onClick={() => void establishContext()}
					>
						{t("dataFoundation.establishContext")}
					</button>
				</CardContent>
			</Card>
			<Card className="order-1">
				<CardHeader>
					<StepTitle step={2}>{t("dataFoundation.publicationTitle")}</StepTitle>
					<CardDescription>
						{t("dataFoundation.publicationDescription")}
					</CardDescription>
				</CardHeader>
				<CardContent className="grid gap-3">
					<div className="grid gap-3 rounded-md border p-3 text-sm sm:grid-cols-2 lg:grid-cols-4">
						<label className="grid gap-1">
							<span>{t("dataFoundation.backfillScope")}</span>
							<select
								className="h-9 rounded-md border bg-background px-3"
								value={backfillScope}
								disabled={Boolean(backfillTaskId)}
								onChange={(event) =>
									setBackfillScope(
										event.target.value as "selected" | "watchlist" | "all",
									)
								}
							>
								<option value="selected">{t("dataFoundation.backfillSelected")}</option>
								<option value="watchlist">
									{t("dataFoundation.backfillWatchlist")}
								</option>
								<option value="all">{t("dataFoundation.backfillAll")}</option>
							</select>
						</label>
						<label className="grid gap-1 text-right">
							<span>{t("dataFoundation.backfillInterval")}</span>
							<select
								className="ml-auto h-9 w-1/2 rounded-md border bg-background px-3"
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
						<label className="grid gap-1" htmlFor="backfill-range-start">
							<span>{t("dataFoundation.rangeStart")}</span>
							<Input
								id="backfill-range-start"
								type="date"
								value={rangeStart}
								disabled={Boolean(backfillTaskId)}
								onChange={(event) => setRangeStart(event.target.value)}
							/>
						</label>
						<label className="grid gap-1" htmlFor="backfill-range-end">
							<span>{t("dataFoundation.rangeEnd")}</span>
							<Input
								id="backfill-range-end"
								type="date"
								value={rangeEnd}
								max={latestClosedDate}
								disabled={Boolean(backfillTaskId)}
								onChange={(event) => setRangeEnd(event.target.value)}
							/>
						</label>
						<label
							className="grid gap-1 sm:col-span-2 lg:col-span-4"
							htmlFor="okx-backfill-instruments"
						>
							<span>{t("dataFoundation.backfillCustomInstruments")}</span>
							<Input
								id="okx-backfill-instruments"
								value={instrumentCodes}
								disabled={backfillScope !== "selected" || Boolean(backfillTaskId)}
								placeholder="BTC-USDT, ETH-USDT"
								onChange={(event) => setInstrumentCodes(event.target.value)}
							/>
							<p className="text-xs text-muted-foreground">
								{t("dataFoundation.backfillCustomInstrumentsHint")}
							</p>
						</label>
						{backfillScope === "watchlist" ? (
							<div className="rounded-md border p-2 text-xs text-muted-foreground sm:col-span-2 lg:col-span-4">
								<strong className="text-foreground">
									{t("dataFoundation.backfillWatchlistContents")}
								</strong>
								<p>
									{watchlistCodes.length
										? watchlistCodes.join(", ")
										: t("dataFoundation.backfillWatchlistEmpty")}
								</p>
							</div>
						) : null}
						{backfillScope === "all" ? (
							<label className="grid gap-1 sm:col-span-2 lg:col-span-4">
								<span>{t("dataFoundation.backfillAllSnapshot")}</span>
								<select
									className="h-9 rounded-md border bg-background px-3"
									value={selectedBackfillSnapshot?.snapshotId ?? ""}
									disabled={Boolean(backfillTaskId) || !backfillSnapshots.length}
									onChange={(event) => setSelectedBackfillSnapshotId(event.target.value)}
								>
									{backfillSnapshots.length ? (
										backfillSnapshots.map((snapshot) => (
											<option key={snapshot.snapshotId} value={snapshot.snapshotId}>
												{shortId(snapshot.snapshotId)} -{" "}
												{formatDateTime(snapshot.retrievedAtMs)} -{" "}
												{humanizeNumber(snapshot.instruments.length)}
											</option>
										))
									) : (
										<option value="">
											{t("dataFoundation.okxInstrumentMasterEmpty")}
										</option>
									)}
								</select>
							</label>
						) : null}
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
						<button
							type="button"
							className="rounded-md border px-3 py-2 text-xs font-medium hover:bg-muted disabled:opacity-50"
							disabled={Boolean(
								activeOperation ||
									backfillTaskId ||
									publishingGateTwo ||
									!gateTwoRequest,
							)}
							onClick={() => void publishGateTwo()}
						>
							{publishingGateTwo
								? t("dataFoundation.publishingGateTwo")
								: t("dataFoundation.publishGateTwo")}
						</button>
						{universeId ? (
							<p className="w-full text-xs text-muted-foreground" role="status">
								{t("dataFoundation.gateTwoPublished", { id: universeId })}
							</p>
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
					<div className="mt-4 border-t pt-3">
						<strong className="text-base">
							{t("dataFoundation.backfillHistoryTitle")}
						</strong>
					</div>
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
										<span className="font-medium" title={dataset.sourceId}>
											{shortId(dataset.sourceId)}
										</span>
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
										disabled={Boolean(
											dataset.qualityReportId || publishingSourceId || backfillTaskId,
										)}
										onClick={() => void publishSource(dataset)}
									>
										{publishingSourceId === dataset.sourceId
											? t("dataFoundation.assessingQuality")
											: t("dataFoundation.assessQuality")}
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
					<div className="mt-4 border-t pt-4">
						<p className="mb-3 text-base font-medium">
							{t("dataFoundation.executionStatus")}
						</p>
						{acquisitionQuery.isPending ? (
							<p className="text-sm text-muted-foreground" role="status">
								{t("dataFoundation.loadingHistory")}
							</p>
						) : acquisitionQuery.data?.length ? (
							<div className="grid gap-3 text-sm">
								<div className="grid gap-2">
									{acquisitionSlice.map((operation) => (
										<div
											key={`${operation.instrument.venue.id}:${operation.instrument.code}:${operation.interval}`}
											className="flex flex-wrap items-center justify-between gap-3 rounded-md border p-3"
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
							onRetry={(operation) => void retryOkxBackfill(operation.operationId)}
						/>
					</div>
				</CardContent>
			</Card>
			<Card className="order-3">
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
			<strong className="text-sm">
				{t("dataFoundation.operationHistoryTitle")}
			</strong>
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
