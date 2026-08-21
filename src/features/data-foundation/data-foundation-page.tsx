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
import { invoke } from "@tauri-apps/api/core";
import {
	ArrowRightIcon,
	CheckCircle2Icon,
	CircleAlertIcon,
	DatabaseIcon,
	LoaderCircleIcon,
} from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useQuery } from "@tanstack/react-query";

type PipelineDatasetSummary = {
	sourceId: string;
	canonicalId?: string;
	revision: number;
	state: "passed" | "degraded" | "rejected";
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

type FoundationMarket = {
	id: "crypto" | "a-shares" | "us-equities";
	titleKey: string;
	descriptionKey: string;
	workspace: "/markets/crypto" | "/markets/a-shares" | "/markets/us-equities";
	acquireCommand: string;
	cancelCommand?: string;
};

const MARKET_VENUES = {
	crypto: ["okx"],
	"a-shares": ["sse", "szse"],
	"us-equities": ["alpaca"],
} as const;

const markets: FoundationMarket[] = [
	{
		id: "crypto",
		titleKey: "markets.crypto.title",
		descriptionKey: "markets.crypto.description",
		workspace: "/markets/crypto",
		acquireCommand: "okx_instrument_master_acquire",
	},
	{
		id: "a-shares",
		titleKey: "markets.aShares.title",
		descriptionKey: "markets.aShares.description",
		workspace: "/markets/a-shares",
		acquireCommand: "ashare_instrument_master_acquire",
		cancelCommand: "ashare_acquisition_cancel",
	},
	{
		id: "us-equities",
		titleKey: "markets.usEquities.title",
		descriptionKey: "markets.usEquities.description",
		workspace: "/markets/us-equities",
		acquireCommand: "alpaca_instrument_master_acquire",
		cancelCommand: "alpaca_acquisition_cancel",
	},
];

export function DataFoundationPage() {
	const { t } = useTranslation();
	const userId = useMarketSessionStore((state) => state.userId);
	const [activeOperation, setActiveOperation] = useState<string>();
	const [error, setError] = useState<string>();
	const [contextMarket, setContextMarket] =
		useState<FoundationMarket["id"]>("crypto");
	const [contextVenue, setContextVenue] = useState("okx");
	const [snapshotId, setSnapshotId] = useState("");
	const [universeId, setUniverseId] = useState("");
	const [rangeStart, setRangeStart] = useState(() =>
		new Date(Date.now() - 30 * 86_400_000).toISOString().slice(0, 10),
	);
	const [rangeEnd, setRangeEnd] = useState(() =>
		new Date().toISOString().slice(0, 10),
	);
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
					request: { userId, page: 0 },
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
	const acquisitionQuery = useQuery({
		queryKey: ["data-foundation-acquisitions", userId],
		queryFn: () =>
			invoke<OkxAcquisitionStatus[]>("okx_acquisition_status", {
				request: { userId },
			}),
		enabled: Boolean(userId),
		staleTime: 30_000,
	});

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
		const operationId = `${market.id}-foundation-${crypto.randomUUID()}`;
		setError(undefined);
		setActiveOperation(operationId);
		try {
			await invoke(market.acquireCommand, {
				request: { userId, operationId },
			});
			await Promise.all([pipelineQuery.refetch(), acquisitionQuery.refetch()]);
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
		} catch (cause) {
			setError(getErrorMessage(cause));
		}
	};

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
							pipelineQuery.data?.filter((item) => item.state !== "passed").length ?? 0
						}
						loading={pipelineQuery.isPending}
					/>
				</CardContent>
			</Card>
			<Card>
				<CardHeader>
					<CardTitle>{t("dataFoundation.selectContextTitle")}</CardTitle>
					<CardDescription>
						{t("dataFoundation.selectContextDescription")}
					</CardDescription>
				</CardHeader>
				<CardContent className="grid gap-3 md:grid-cols-2 lg:grid-cols-6">
					<label className="grid gap-1 text-sm">
						<span>{t("dataFoundation.contextMarket")}</span>
						<select
							className="h-9 rounded-md border bg-background px-3"
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
							className="h-9 rounded-md border bg-background px-3"
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
							className="h-9 rounded-md border bg-background px-3"
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
							className="h-9 rounded-md border bg-background px-3"
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
						className="h-9 rounded-md bg-primary px-3 text-sm font-medium text-primary-foreground disabled:opacity-50 md:col-span-2 lg:col-span-5 lg:justify-self-start"
						disabled={!snapshotId || snapshotsQuery.isPending}
						onClick={() => void establishContext()}
					>
						{t("dataFoundation.establishContext")}
					</button>
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
						<div className="grid gap-2">
							{acquisitionQuery.data.slice(0, 8).map((operation) => (
								<div
									key={`${operation.instrument.venue.id}:${operation.instrument.code}:${operation.interval}`}
									className="flex flex-wrap items-center justify-between gap-3 rounded-md border p-3 text-sm"
								>
									<div>
										<strong>{operation.instrument.code}</strong>
										<span className="ml-2 text-muted-foreground">
											{operation.interval} ·{" "}
											{t("dataFoundation.pages", { count: operation.pages })}
										</span>
									</div>
									<Badge
										variant={operation.state === "completed" ? "default" : "outline"}
									>
										{t(`dataFoundation.states.${operation.state}`)}
									</Badge>
								</div>
							))}
						</div>
					) : (
						<p className="text-sm text-muted-foreground">
							{t("dataFoundation.emptyHistory")}
						</p>
					)}
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
								<button
									type="button"
									className="inline-flex h-9 items-center justify-center rounded-md bg-primary px-3 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:pointer-events-none disabled:opacity-50"
									onClick={() => void acquire(market)}
									disabled={Boolean(activeOperation)}
								>
									{t("dataFoundation.startAcquisition")}
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
			{pipelineQuery.data?.some((item) => item.state !== "passed") ? (
				<p className="text-sm text-amber-700 dark:text-amber-300" role="status">
					<CircleAlertIcon className="mr-1 inline size-4" aria-hidden="true" />
					{t("dataFoundation.qualityWarning")}
				</p>
			) : null}
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
