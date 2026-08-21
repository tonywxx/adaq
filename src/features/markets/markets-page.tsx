import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
	Card,
	CardAction,
	CardContent,
	CardDescription,
	CardFooter,
	CardHeader,
	CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Dashboard } from "@/layout/home";
import {
	addWatchlistInstrument,
	DEFAULT_ACTIVE_INSTRUMENT,
	getErrorMessage,
	instrumentKey as sessionInstrumentKey,
	removeWatchlistInstrument,
	setActiveInstrument,
	useMarketSessionStore,
} from "@/lib/market-session";
import { formatDateTime, formatDecimal } from "@/lib/i18n";
import { toMarketChartData } from "@/lib/market-chart-adapter";
import WChart from "@/w/lightweight-charts/WChart";
import { Link } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import {
	ArrowRightIcon,
	CircleAlertIcon,
	Clock3Icon,
	LoaderCircleIcon,
	PlusIcon,
	SearchIcon,
	XIcon,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { ReactNode, RefObject } from "react";
import { useTranslation } from "react-i18next";
import {
	type AlpacaInstrument,
	type AshareInstrument,
	type CalendarDto,
	type InstrumentMasterSnapshot,
	type MarketId,
	type MarketInstrument,
	type ProviderCapabilitySnapshot,
	marketMatches,
	normalizeAshareSnapshot,
	normalizeUsEquitySnapshot,
	resolveSession,
} from "@/features/markets/market-workspaces";

type CatalogResult = {
	snapshot: InstrumentMasterSnapshot<AshareInstrument | AlpacaInstrument>;
	instruments: MarketInstrument[];
};

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

type MarketTickerSnapshot = {
	src: string;
	code: string;
	last: string;
	lastQuantity: string;
	askPrice: string | null;
	askQuantity: string | null;
	bidPrice: string | null;
	bidQuantity: string | null;
	open24h: string;
	high24h: string;
	low24h: string;
	baseVolume24h: string;
	quoteVolume24h: string;
	timestampMs: number;
};

type MarketSnapshot = {
	provider: string;
	instrument: { venue: MarketInstrument["instrument"]["venue"]; code: string };
	feed: string;
	retrievedAtMs: number;
	freshnessMs?: number;
	ticker: MarketTickerSnapshot;
	quote: {
		askPrice: string | null;
		askQuantity: string | null;
		bidPrice: string | null;
		bidQuantity: string | null;
		timestampMs: number;
		feed: string;
	};
};

type WorkspaceBars = {
	instrument: MarketInstrument["instrument"];
	provider: string;
	actualUpstream: string;
	method: string;
	connectorVersion: string;
	retrievedAtMs: number;
	freshnessMs?: number;
	priceBasis: "unadjusted" | "forwardAdjusted" | "backwardAdjusted" | "unknown";
	quality: "unknown";
	bars: Array<{
		openTimeMs: number;
		open: string;
		high: string;
		low: string;
		close: string;
		baseVolume: string;
		quoteVolume: string;
	}>;
	gaps?: Array<{ startTimeMs: number; endTimeMs: number }>;
	limitations: string[];
};

const STALE_AFTER_MS = 5 * 60_000;

const MARKET_CONFIG = {
	"a-shares": {
		title: "markets.aShares.title",
		description: "markets.aShares.description",
		provider: "akshare-rs",
		listCommand: "ashare_instrument_master_list",
		acquireCommand: "ashare_instrument_master_acquire",
		cancelCommand: "ashare_acquisition_cancel",
	},
	"us-equities": {
		title: "markets.usEquities.title",
		description: "markets.usEquities.description",
		provider: "alpaca",
		listCommand: "alpaca_instrument_master_list",
		acquireCommand: "alpaca_instrument_master_acquire",
		cancelCommand: "alpaca_acquisition_cancel",
	},
} as const;

export function OperationsDashboard() {
	const { t } = useTranslation();
	return (
		<PageFrame
			title={t("operations.title")}
			description={t("operations.description")}
		>
			<Card>
				<CardHeader>
					<CardTitle>{t("operations.paperOperations")}</CardTitle>
					<CardDescription>
						{t("operations.paperOperationsDescription")}
					</CardDescription>
					<CardAction>
						<Badge variant="outline">{t("operations.planned")}</Badge>
					</CardAction>
				</CardHeader>
			</Card>
			<Card>
				<CardHeader>
					<CardTitle>{t("operations.marketSessions")}</CardTitle>
					<CardDescription>
						{t("operations.marketSessionsDescription")}
					</CardDescription>
				</CardHeader>
				<CardContent className="grid gap-2 sm:grid-cols-4">
					{(
						[
							["/factors", t("nav.factorResearch")],
							["/markets/crypto", t("markets.crypto.title")],
							["/markets/a-shares", t("markets.aShares.title")],
							["/markets/us-equities", t("markets.usEquities.title")],
						] as const
					).map(([to, label]) => (
						<Link
							key={to}
							to={to}
							className="flex items-center justify-between rounded-md border px-3 py-2 text-sm hover:bg-muted"
						>
							<span>{label}</span>
							<ArrowRightIcon className="size-4" aria-hidden="true" />
						</Link>
					))}
				</CardContent>
			</Card>
		</PageFrame>
	);
}

export function MarketsOverview() {
	const { t } = useTranslation();
	const catalog = useMarketSessionStore((state) => state.watchlist);
	return (
		<PageFrame
			title={t("markets.overview.title")}
			description={t("markets.overview.description")}
		>
			<div className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_minmax(360px,420px)]">
				<div className="grid gap-4 sm:grid-cols-3">
					<MarketEntry market="crypto" />
					<MarketEntry market="a-shares" />
					<MarketEntry market="us-equities" />
				</div>
				<UnifiedWatchlistCard market="all" catalog={[]} />
			</div>
			{catalog.length === 0 ? null : (
				<p className="text-xs text-muted-foreground">
					{t("markets.overview.watchlistHint")}
				</p>
			)}
		</PageFrame>
	);
}

export function CryptoMarketPage() {
	const { t } = useTranslation();
	const watchlist = useMarketSessionStore((state) => state.watchlist);
	const activeInstrument = useMarketSessionStore(
		(state) => state.activeInstrument,
	);
	const tickerStatus = useMarketSessionStore((state) => state.tickerStatus);
	const barStatus = useMarketSessionStore((state) => state.barStatus);
	useEffect(() => {
		if (marketMatches(activeInstrument, "crypto")) return;
		const next = watchlist.find((value) => marketMatches(value, "crypto"));
		void setActiveInstrument(next ?? DEFAULT_ACTIVE_INSTRUMENT).catch(() => {});
	}, [activeInstrument, watchlist]);
	return (
		<PageFrame
			title={t("markets.crypto.title")}
			description={t("markets.crypto.description")}
			trailing={
				<Badge variant="outline">
					{tickerStatus === "live" && barStatus === "live"
						? t("markets.live")
						: t("markets.pending")}
				</Badge>
			}
		>
			<Dashboard />
			<Card>
				<CardHeader>
					<CardTitle>{t("markets.evidence.title")}</CardTitle>
					<CardDescription>
						{t("markets.evidence.cryptoDescription")}
					</CardDescription>
				</CardHeader>
				<CardContent className="grid gap-3 text-sm sm:grid-cols-3">
					<EvidenceField label={t("markets.evidence.provider")} value="OKX Spot" />
					<EvidenceField
						label={t("markets.evidence.session")}
						value={t("markets.evidence.utcContinuous")}
					/>
					<EvidenceField
						label={t("markets.evidence.quality")}
						value={t("markets.evidence.liveObservation")}
					/>
				</CardContent>
			</Card>
		</PageFrame>
	);
}

export function MarketWorkspacePage({
	market,
}: {
	market: Exclude<MarketId, "crypto">;
}) {
	const { t } = useTranslation();
	const userId = useMarketSessionStore((state) => state.userId);
	const activeInstrument = useMarketSessionStore(
		(state) => state.activeInstrument,
	);
	const [search, setSearch] = useState("");
	const [foundationRequested, setFoundationRequested] = useState(false);
	const [activeOperationId, setActiveOperationId] = useState<string>();
	const [acquisitionError, setAcquisitionError] = useState<string>();
	const instrumentSearchRef = useRef<HTMLInputElement>(null);
	const catalogQuery = useMarketCatalog(market, userId);
	const instruments = useMemo(() => {
		const all = catalogQuery.data?.instruments ?? [];
		const needle = search.trim().toLowerCase();
		return needle
			? all.filter(
					(value) =>
						value.instrument.code.toLowerCase().includes(needle) ||
						value.providerSymbol.toLowerCase().includes(needle) ||
						value.name?.toLowerCase().includes(needle),
				)
			: all;
	}, [catalogQuery.data?.instruments, search]);
	const selected = useMemo(() => {
		const activeKey = activeInstrument
			? `${activeInstrument.venue?.id ?? activeInstrument.src}:${activeInstrument.code}`
			: "";
		return instruments.find((value) => value.key === activeKey) ?? instruments[0];
	}, [activeInstrument, instruments]);
	useEffect(() => {
		if (!selected) return;
		const activeKey = activeInstrument
			? `${activeInstrument.venue?.id ?? activeInstrument.src}:${activeInstrument.code}`
			: "";
		if (activeKey === selected.key) return;
		void setActiveInstrument(selected.ref).catch(() => {});
	}, [activeInstrument, selected]);
	const calendarQuery = useMarketCalendar(
		market,
		userId,
		selected,
		foundationRequested,
	);
	const pipelineQuery = useQuery({
		queryKey: ["market-pipeline-summary", userId],
		queryFn: () =>
			invoke<PipelineDatasetSummary[]>("market_data_pipeline_list", { userId }),
		enabled: Boolean(userId),
		staleTime: 60_000,
	});
	const snapshotQuery = useQuery({
		queryKey: ["market-us-snapshot", userId, selected?.key],
		queryFn: () =>
			invoke<MarketSnapshot>("alpaca_snapshot", {
				request: { userId, instrument: selected?.instrument },
			}),
		enabled: Boolean(
			userId && selected && foundationRequested && market === "us-equities",
		),
		staleTime: 30_000,
	});
	const barsQuery = useQuery({
		queryKey: ["market-workspace-bars", userId, selected?.key],
		queryFn: () => {
			const endTimeMs = Date.now();
			return invoke<WorkspaceBars>("market_workspace_get_bars", {
				request: {
					userId,
					instrument: selected?.instrument,
					interval: "1d",
					startTimeMs: endTimeMs - 180 * 86_400_000,
					endTimeMs,
				},
			});
		},
		enabled: Boolean(userId && selected && foundationRequested),
		staleTime: 60_000,
	});
	const config = MARKET_CONFIG[market];
	const startFoundationAcquisition = async () => {
		if (!userId) return;
		setAcquisitionError(undefined);
		const operationId = `${market}-instrument-master-${crypto.randomUUID()}`;
		setActiveOperationId(operationId);
		try {
			await invoke(config.acquireCommand, {
				request: { userId, operationId },
			});
			setFoundationRequested(true);
			await catalogQuery.refetch();
		} catch (error) {
			setAcquisitionError(getErrorMessage(error));
		} finally {
			setActiveOperationId(undefined);
		}
	};
	const cancelFoundationAcquisition = async () => {
		if (!userId || !activeOperationId) return;
		try {
			await invoke(config.cancelCommand, {
				request: { userId, operationId: activeOperationId },
			});
		} catch (error) {
			setAcquisitionError(getErrorMessage(error));
		}
	};

	return (
		<PageFrame
			title={t(config.title)}
			description={t(config.description)}
			trailing={
				<div className="flex items-center gap-2">
					{catalogQuery.isFetching ? (
						<Badge variant="outline" aria-live="polite">
							<LoaderCircleIcon className="animate-spin" aria-hidden="true" />
							{t("markets.refreshing")}
						</Badge>
					) : null}
					<Button
						type="button"
						onClick={() => void startFoundationAcquisition()}
						disabled={catalogQuery.isFetching || Boolean(activeOperationId)}
					>
						{t("markets.foundation.startAcquisition")}
					</Button>
					{activeOperationId ? (
						<Button
							type="button"
							variant="outline"
							onClick={() => void cancelFoundationAcquisition()}
						>
							{t("markets.foundation.cancelAcquisition")}
						</Button>
					) : null}
				</div>
			}
		>
			<div className="grid min-w-0 gap-4 lg:grid-cols-[minmax(300px,360px)_minmax(0,1fr)]">
				<UnifiedWatchlistCard
					market={market}
					catalog={catalogQuery.data?.instruments ?? []}
					onAdd={() => instrumentSearchRef.current?.focus()}
				/>
				<main className="grid min-w-0 gap-4" aria-busy={catalogQuery.isPending}>
					{acquisitionError ? (
						<p className="text-sm text-destructive" role="alert">
							{acquisitionError}
						</p>
					) : null}
					<InstrumentSearchCard
						market={market}
						instruments={instruments}
						allInstruments={catalogQuery.data?.instruments ?? []}
						search={search}
						onSearch={setSearch}
						inputRef={instrumentSearchRef}
						loading={catalogQuery.isPending}
						error={catalogQuery.error}
					/>
					<MarketTickerCard
						market={market}
						instrument={selected}
						snapshot={snapshotQuery.data}
						loading={snapshotQuery.isPending}
						error={snapshotQuery.error}
					/>
					<MarketBarsCard
						instrument={selected}
						bars={barsQuery.data}
						loading={barsQuery.isPending}
						error={barsQuery.error}
					/>
					<MarketEvidenceCard
						market={market}
						instrument={selected}
						catalog={catalogQuery.data}
						calendar={calendarQuery.data}
						calendarLoading={calendarQuery.isPending}
						pipeline={pipelineQuery.data ?? []}
						snapshot={snapshotQuery.data}
						bars={barsQuery.data}
					/>
				</main>
			</div>
		</PageFrame>
	);
}

function PageFrame({
	title,
	description,
	trailing,
	children,
}: {
	title: string;
	description: string;
	trailing?: ReactNode;
	children: ReactNode;
}) {
	return (
		<div className="flex min-w-0 flex-1 flex-col gap-5 p-4 lg:p-6">
			<div className="flex items-start justify-between gap-4">
				<div>
					<h1 className="text-2xl font-semibold tracking-tight">{title}</h1>
					<p className="text-sm text-muted-foreground">{description}</p>
				</div>
				{trailing}
			</div>
			{children}
		</div>
	);
}

function MarketEntry({ market }: { market: MarketId }) {
	const { t } = useTranslation();
	const route = market === "crypto" ? "/markets/crypto" : `/markets/${market}`;
	const title =
		market === "crypto"
			? t("markets.crypto.title")
			: market === "a-shares"
				? t("markets.aShares.title")
				: t("markets.usEquities.title");
	const overview =
		market === "crypto"
			? t("markets.crypto.overview")
			: market === "a-shares"
				? t("markets.aShares.overview")
				: t("markets.usEquities.overview");
	return (
		<Card className="flex flex-col">
			<CardHeader>
				<CardTitle className="text-base">{title}</CardTitle>
				<CardDescription>{overview}</CardDescription>
			</CardHeader>
			<CardFooter className="mt-auto">
				<Link
					to={route}
					className="inline-flex items-center gap-2 text-sm text-primary hover:underline"
				>
					{t("markets.openWorkspace")}
					<ArrowRightIcon className="size-4" aria-hidden="true" />
				</Link>
			</CardFooter>
		</Card>
	);
}

function UnifiedWatchlistCard({
	market,
	catalog,
	onAdd,
}: {
	market: MarketId | "all";
	catalog: MarketInstrument[];
	onAdd?: () => void;
}) {
	const { t } = useTranslation();
	const watchlist = useMarketSessionStore((state) => state.watchlist);
	const ready = useMarketSessionStore((state) => state.ready);
	const limit = useMarketSessionStore((state) => state.watchlistLimit);
	const active = useMarketSessionStore((state) => state.activeInstrument);
	const [error, setError] = useState<string>();
	const items = watchlist.filter(
		(value) => market === "all" || marketMatches(value, market),
	);
	const details = useMemo(
		() => new Map(catalog.map((value) => [value.key, value])),
		[catalog],
	);

	return (
		<Card className="min-w-0">
			<CardHeader>
				<CardTitle>{t("market.watchlist")}</CardTitle>
				<CardDescription>
					{t("markets.watchlistSummary", {
						count: items.length,
						limit: limit || "—",
					})}
				</CardDescription>
				{onAdd ? (
					<CardAction>
						<Button
							type="button"
							size="sm"
							variant="outline"
							disabled={!ready || items.length >= limit}
							onClick={onAdd}
						>
							<PlusIcon aria-hidden="true" />
							{t("market.add")}
						</Button>
					</CardAction>
				) : null}
			</CardHeader>
			<CardContent className="grid gap-2">
				{!ready ? (
					<div
						className="flex items-center gap-2 py-6 text-sm text-muted-foreground"
						role="status"
					>
						<LoaderCircleIcon className="size-4 animate-spin" aria-hidden="true" />
						{t("market.loadingWatchlist")}
					</div>
				) : items.length === 0 ? (
					<div className="rounded-md border border-dashed px-3 py-6 text-center text-sm text-muted-foreground">
						{t("markets.emptyWatchlist")}
					</div>
				) : (
					items.map((item) => {
						const detail = details.get(sessionInstrumentKey(item));
						const activeItem =
							sessionInstrumentKey(item) === sessionInstrumentKey(active);
						return (
							<div
								key={sessionInstrumentKey(item)}
								className={`flex items-center gap-2 rounded-md border px-2 py-2 ${activeItem ? "border-primary/50 bg-primary/5" : ""}`}
							>
								<button
									type="button"
									className="min-w-0 flex-1 text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
									onClick={() => {
										setError(undefined);
										void setActiveInstrument(item).catch((value) =>
											setError(getErrorMessage(value)),
										);
									}}
								>
									<span className="block truncate font-medium">{item.code}</span>
									<span className="block truncate text-xs text-muted-foreground">
										{detail?.name ?? item.venue?.id ?? item.src}
									</span>
								</button>
								<Button
									type="button"
									size="icon-xs"
									variant="ghost"
									aria-label={t("market.removeFromWatchlist", { instrument: item.code })}
									onClick={() => {
										setError(undefined);
										void removeWatchlistInstrument(item).catch((value) =>
											setError(getErrorMessage(value)),
										);
									}}
								>
									<XIcon aria-hidden="true" />
								</Button>
							</div>
						);
					})
				)}
				{error ? (
					<p className="text-xs text-destructive" role="status">
						{error}
					</p>
				) : null}
			</CardContent>
		</Card>
	);
}

function InstrumentSearchCard({
	market,
	instruments,
	allInstruments,
	search,
	onSearch,
	inputRef,
	loading,
	error,
}: {
	market: Exclude<MarketId, "crypto">;
	instruments: MarketInstrument[];
	allInstruments: MarketInstrument[];
	search: string;
	onSearch: (value: string) => void;
	inputRef: RefObject<HTMLInputElement | null>;
	loading: boolean;
	error: unknown;
}) {
	const { t } = useTranslation();
	const watchlist = useMarketSessionStore((state) => state.watchlist);
	const existing = useMemo(
		() => new Set(watchlist.map(sessionInstrumentKey)),
		[watchlist],
	);
	const results = instruments
		.filter((value) => !existing.has(value.key))
		.slice(0, 8);
	const config = MARKET_CONFIG[market];
	const [adding, setAdding] = useState<string>();
	const [mutationError, setMutationError] = useState<string>();

	return (
		<Card>
			<CardHeader>
				<CardTitle>{t("markets.instrumentSearch.title")}</CardTitle>
				<CardDescription>
					{t("markets.instrumentSearch.description")}
				</CardDescription>
			</CardHeader>
			<CardContent className="grid gap-3">
				<div className="relative">
					<SearchIcon
						className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground"
						aria-hidden="true"
					/>
					<Input
						value={search}
						type="search"
						aria-label={t("markets.instrumentSearch.label")}
						placeholder={t("markets.instrumentSearch.placeholder")}
						className="pl-9"
						ref={inputRef}
						onChange={(event) => onSearch(event.target.value)}
					/>
				</div>
				{loading ? (
					<div
						className="flex items-center gap-2 py-3 text-sm text-muted-foreground"
						role="status"
					>
						<LoaderCircleIcon className="size-4 animate-spin" aria-hidden="true" />
						{t("markets.instrumentSearch.loading")}
					</div>
				) : error ? (
					<div
						className="flex items-start gap-2 rounded-md border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive"
						role="alert"
					>
						<CircleAlertIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
						<span>{getErrorMessage(error)}</span>
					</div>
				) : results.length === 0 ? (
					<p className="py-3 text-sm text-muted-foreground">
						{allInstruments.length === 0
							? t("markets.instrumentSearch.empty")
							: t("markets.instrumentSearch.noMatches")}
					</p>
				) : (
					<div className="grid gap-1">
						{results.map((instrument) => (
							<div
								key={instrument.key}
								className="flex items-center gap-3 rounded-md border px-3 py-2"
							>
								<button
									type="button"
									className="min-w-0 flex-1 text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
									onClick={() =>
										void setActiveInstrument(instrument.ref).catch((value) =>
											setMutationError(getErrorMessage(value)),
										)
									}
								>
									<span className="block truncate font-medium">
										{instrument.instrument.code}
									</span>
									<span className="block truncate text-xs text-muted-foreground">
										{instrument.providerSymbol}
										{instrument.name ? ` · ${instrument.name}` : ""} ·{" "}
										{instrument.instrument.venue.id}
									</span>
								</button>
								<Button
									type="button"
									size="sm"
									variant="outline"
									disabled={adding === instrument.key}
									onClick={() => {
										setMutationError(undefined);
										setAdding(instrument.key);
										void addWatchlistInstrument(instrument.ref)
											.then(() => setAdding(undefined))
											.catch((value) => {
												setAdding(undefined);
												setMutationError(getErrorMessage(value));
											});
									}}
								>
									{adding === instrument.key ? (
										<LoaderCircleIcon className="animate-spin" aria-hidden="true" />
									) : (
										<PlusIcon aria-hidden="true" />
									)}
									{t("market.add")}
								</Button>
							</div>
						))}
					</div>
				)}
				{mutationError ? (
					<p className="text-xs text-destructive" role="status">
						{mutationError}
					</p>
				) : null}
				{allInstruments.length > results.length ? (
					<p className="text-xs text-muted-foreground">
						{t("markets.instrumentSearch.resultLimit", { provider: config.provider })}
					</p>
				) : null}
			</CardContent>
		</Card>
	);
}

function MarketTickerCard({
	market,
	instrument,
	snapshot,
	loading,
	error,
}: {
	market: Exclude<MarketId, "crypto">;
	instrument?: MarketInstrument;
	snapshot?: MarketSnapshot;
	loading: boolean;
	error: unknown;
}) {
	const { t } = useTranslation();
	const isAshare = market === "a-shares";
	const ticker = snapshot?.ticker;
	const last = instrument?.last ?? ticker?.last;
	const bid = isAshare ? null : (snapshot?.quote.bidPrice ?? ticker?.bidPrice);
	const ask = isAshare ? null : (snapshot?.quote.askPrice ?? ticker?.askPrice);
	const observedAt = instrument?.observedAtMs ?? snapshot?.retrievedAtMs;
	return (
		<Card>
			<CardHeader>
				<CardTitle>{t("markets.ticker.title")}</CardTitle>
				<CardDescription>
					{instrument
						? `${instrument.instrument.code} · ${instrument.instrument.venue.id}`
						: t("markets.ticker.noInstrument")}
				</CardDescription>
			</CardHeader>
			<CardContent>
				{loading ? (
					<div
						className="flex items-center gap-2 py-6 text-sm text-muted-foreground"
						role="status"
					>
						<LoaderCircleIcon className="size-4 animate-spin" aria-hidden="true" />
						{t("markets.ticker.loading")}
					</div>
				) : error ? (
					<p
						className="rounded-md border border-dashed px-3 py-4 text-sm text-muted-foreground"
						role="status"
					>
						{getErrorMessage(error)}
					</p>
				) : !instrument ? (
					<p className="py-4 text-sm text-muted-foreground">
						{t("markets.ticker.noInstrument")}
					</p>
				) : (
					<dl className="grid grid-cols-2 gap-4 text-sm sm:grid-cols-4">
						<EvidenceField
							label={t("markets.ticker.last")}
							value={last ? formatDecimal(last) : t("markets.unavailable")}
						/>
						<EvidenceField
							label={t("markets.ticker.bid")}
							value={bid ? formatDecimal(bid) : t("markets.unavailable")}
						/>
						<EvidenceField
							label={t("markets.ticker.ask")}
							value={ask ? formatDecimal(ask) : t("markets.unavailable")}
						/>
						<EvidenceField
							label={t("markets.ticker.volume")}
							value={
								instrument.baseVolume
									? formatDecimal(instrument.baseVolume)
									: ticker
										? formatDecimal(ticker.baseVolume24h)
										: t("markets.unavailable")
							}
						/>
					</dl>
				)}
			</CardContent>
			{observedAt ? (
				<CardFooter className="flex-wrap gap-x-2 gap-y-1 text-xs text-muted-foreground">
					<Clock3Icon className="size-3.5" aria-hidden="true" />
					{t("markets.ticker.observedAt", {
						time: formatDateTime(observedAt, {
							dateStyle: "medium",
							timeStyle: "medium",
						}),
					})}
					{snapshot?.feed ? ` · ${snapshot.feed}` : null}
				</CardFooter>
			) : null}
		</Card>
	);
}

function MarketBarsCard({
	instrument,
	bars,
	loading,
	error,
}: {
	instrument?: MarketInstrument;
	bars?: WorkspaceBars;
	loading: boolean;
	error: unknown;
}) {
	const { t } = useTranslation();
	const chartData = useMemo(
		() => toMarketChartData(bars?.bars ?? [], bars?.gaps ?? [], "1d"),
		[bars],
	);
	return (
		<Card>
			<CardHeader>
				<CardTitle>{t("markets.bars.title")}</CardTitle>
				<CardDescription>{t("markets.bars.description")}</CardDescription>
			</CardHeader>
			<CardContent>
				{loading ? (
					<div
						className="flex h-64 items-center justify-center gap-2 text-sm text-muted-foreground"
						role="status"
					>
						<LoaderCircleIcon className="size-4 animate-spin" aria-hidden="true" />
						{t("markets.bars.loading")}
					</div>
				) : error ? (
					<div
						className="flex items-start gap-2 rounded-md border border-dashed p-4 text-sm text-muted-foreground"
						role="status"
					>
						<CircleAlertIcon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
						{getErrorMessage(error)}
					</div>
				) : !instrument || chartData.length === 0 ? (
					<p className="py-6 text-sm text-muted-foreground">
						{t("markets.bars.empty")}
					</p>
				) : (
					<div
						role="img"
						aria-label={t("markets.bars.ariaLabel", {
							instrument: instrument.instrument.code,
						})}
					>
						<WChart
							data={chartData}
							chartType="candlestick"
							height={300}
							autoSize
							showVolume
							showEma={false}
							watermarkVisible
							watermarkText={`${instrument.instrument.code} · ${bars?.provider ?? ""}`}
						/>
					</div>
				)}
			</CardContent>
			{bars ? (
				<CardFooter className="flex-wrap gap-2 text-xs text-muted-foreground">
					<Badge variant="outline">{t("markets.bars.qualityUnknown")}</Badge>
					<span>
						{bars.gaps
							? t("markets.bars.gaps", { count: bars.gaps.length })
							: t("markets.bars.gapsUnknown")}
					</span>
				</CardFooter>
			) : null}
		</Card>
	);
}

function MarketEvidenceCard({
	market,
	instrument,
	catalog,
	calendar,
	calendarLoading,
	pipeline,
	snapshot,
	bars,
}: {
	market: Exclude<MarketId, "crypto">;
	instrument?: MarketInstrument;
	catalog?: CatalogResult;
	calendar?: CalendarDto;
	calendarLoading: boolean;
	pipeline: PipelineDatasetSummary[];
	snapshot?: MarketSnapshot;
	bars?: WorkspaceBars;
}) {
	const { t } = useTranslation();
	const observedAt =
		instrument?.observedAtMs ??
		snapshot?.retrievedAtMs ??
		bars?.retrievedAtMs ??
		catalog?.snapshot.retrievedAtMs;
	const freshnessMs =
		snapshot?.freshnessMs ??
		bars?.freshnessMs ??
		(observedAt === undefined ? undefined : Math.max(0, Date.now() - observedAt));
	const freshnessSeconds =
		freshnessMs === undefined ? undefined : Math.round(freshnessMs / 1000);
	const stale = freshnessMs !== undefined && freshnessMs > STALE_AFTER_MS;
	const latestRevision = pipeline.reduce(
		(maximum, value) => Math.max(maximum, value.revision),
		0,
	);
	const session = resolveSession(
		calendar?.snapshot,
		Date.now(),
		instrument?.instrument.venue ?? {
			id: market === "a-shares" ? "sse" : "nasdaq",
			kind: market === "a-shares" ? "chinaAShareEquity" : "usEquity",
			timeZone: market === "a-shares" ? "Asia/Shanghai" : "America/New_York",
		},
	);
	const qualityCounts = useMemo(
		() => ({
			degraded: pipeline.filter((value) => value.state === "degraded").length,
			rejected: pipeline.filter((value) => value.state === "rejected").length,
		}),
		[pipeline],
	);
	const capability = catalog?.snapshot.capabilitySnapshot;
	return (
		<Card>
			<CardHeader>
				<CardTitle>{t("markets.evidence.title")}</CardTitle>
				<CardDescription>{t("markets.evidence.description")}</CardDescription>
			</CardHeader>
			<CardContent className="grid gap-5">
				<div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-5">
					<EvidenceField
						label={t("markets.evidence.provider")}
						value={
							catalog?.snapshot.provider ?? bars?.provider ?? t("markets.unavailable")
						}
					/>
					<EvidenceField
						label={t("markets.evidence.upstream")}
						value={
							catalog?.snapshot.actualUpstream ??
							bars?.actualUpstream ??
							t("markets.unavailable")
						}
					/>
					<EvidenceField
						label={t("markets.evidence.connector")}
						value={
							catalog?.snapshot.connectorVersion ??
							bars?.connectorVersion ??
							t("markets.unavailable")
						}
					/>
					<EvidenceField
						label={t("markets.evidence.method")}
						value={
							catalog?.snapshot.method ?? bars?.method ?? t("markets.unavailable")
						}
					/>
					<EvidenceField
						label={t("markets.evidence.calendar")}
						value={calendar?.snapshot.snapshotId ?? t("markets.unavailable")}
					/>
					<EvidenceField
						label={t("markets.evidence.revision")}
						value={latestRevision ? String(latestRevision) : t("markets.unknown")}
					/>
				</div>
				<div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
					<EvidenceField
						label={t("markets.evidence.session")}
						value={
							calendarLoading
								? t("markets.loading")
								: session.phase === "unknown"
									? t("markets.unknown")
									: t(`markets.session.${session.phase}`)
						}
					/>
					<EvidenceField
						label={t("markets.evidence.tradingDate")}
						value={session.tradingDate ?? t("markets.unknown")}
					/>
					<EvidenceField
						label={t("markets.evidence.timeZone")}
						value={session.timeZone}
					/>
					<EvidenceField
						label={t("markets.evidence.priceBasis")}
						value={bars?.priceBasis ?? t("markets.unknown")}
					/>
				</div>
				<div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
					<EvidenceField
						label={t("markets.evidence.quality")}
						value={t("markets.evidence.qualityUnknown")}
					/>
					<EvidenceField
						label={t("markets.evidence.degradedPublications")}
						value={String(qualityCounts.degraded)}
					/>
					<EvidenceField
						label={t("markets.evidence.rejectedPublications")}
						value={String(qualityCounts.rejected)}
					/>
					<EvidenceField
						label={t("markets.evidence.coverage")}
						value={capability?.coverage ?? t("markets.unknown")}
					/>
					<EvidenceField
						label={t("markets.evidence.observedBars")}
						value={bars ? String(bars.bars.length) : t("markets.unknown")}
					/>
				</div>
				<EvidenceField
					label={t("markets.evidence.freshness")}
					value={
						freshnessSeconds === undefined
							? t("markets.unknown")
							: stale
								? t("markets.evidence.stale", { seconds: freshnessSeconds })
								: t("markets.evidence.fresh", { seconds: freshnessSeconds })
					}
				/>
				<CapabilitySummary
					capability={capability}
					limitations={[
						...(catalog?.snapshot.limitations ?? []),
						...(bars?.limitations ?? []),
					]}
				/>
			</CardContent>
			<CardFooter className="flex flex-wrap gap-3">
				<Link
					to="/models"
					className="inline-flex items-center gap-2 text-sm text-primary hover:underline"
				>
					{t("markets.evidence.openResearch")}
					{instrument ? `: ${instrument.instrument.code}` : ""}
					<ArrowRightIcon className="size-4" aria-hidden="true" />
				</Link>
				<Link
					to="/backtest"
					className="inline-flex items-center gap-2 text-sm text-primary hover:underline"
				>
					{t("markets.evidence.openBacktest")}
					{instrument ? `: ${instrument.instrument.code}` : ""}
					<ArrowRightIcon className="size-4" aria-hidden="true" />
				</Link>
			</CardFooter>
		</Card>
	);
}

function CapabilitySummary({
	capability,
	limitations,
}: {
	capability?: ProviderCapabilitySnapshot;
	limitations: string[];
}) {
	const { t } = useTranslation();
	return (
		<div className="rounded-md border bg-muted/20 p-3 text-sm">
			<div className="mb-2 flex items-center gap-2 font-medium">
				{t("markets.evidence.capabilities")}
				{capability?.feed ? (
					<Badge variant="outline">{capability.feed}</Badge>
				) : null}
			</div>
			<p className="text-muted-foreground">
				{capability?.delayedKnown
					? capability.delayed
						? t("markets.evidence.delayed", { delay: capability.delayMs ?? "—" })
						: t("markets.evidence.notDelayed")
					: t("markets.evidence.delayUnknown")}
			</p>
			{limitations.length > 0 ? (
				<ul className="mt-2 list-disc space-y-1 pl-5 text-xs text-muted-foreground">
					{[...new Set(limitations)].slice(0, 5).map((value) => (
						<li key={value}>{value}</li>
					))}
				</ul>
			) : null}
		</div>
	);
}

function EvidenceField({ label, value }: { label: string; value: string }) {
	return (
		<div className="min-w-0">
			<dt className="text-xs text-muted-foreground">{label}</dt>
			<dd className="truncate font-medium tabular-nums">{value}</dd>
		</div>
	);
}

function useMarketCatalog(
	market: Exclude<MarketId, "crypto">,
	userId: string | null,
) {
	const config = MARKET_CONFIG[market];
	return useQuery({
		queryKey: ["market-instrument-master", userId, market],
		queryFn: async (): Promise<CatalogResult> => {
			if (!userId) throw new Error("Market workspace is not authenticated");
			const request = { request: { userId } };
			const snapshots = await invoke<
				InstrumentMasterSnapshot<AshareInstrument | AlpacaInstrument>[]
			>(config.listCommand, request);
			const snapshot = [...snapshots].sort(
				(left, right) => right.effectiveAtMs - left.effectiveAtMs,
			)[0];
			if (!snapshot) throw new Error("Instrument Master evidence is empty");
			const instruments =
				market === "a-shares"
					? normalizeAshareSnapshot(
							snapshot as InstrumentMasterSnapshot<AshareInstrument>,
						)
					: normalizeUsEquitySnapshot(
							snapshot as InstrumentMasterSnapshot<AlpacaInstrument>,
						);
			return { snapshot, instruments };
		},
		enabled: Boolean(userId),
		staleTime: 5 * 60_000,
		gcTime: 30 * 60_000,
	});
}

function useMarketCalendar(
	market: Exclude<MarketId, "crypto">,
	userId: string | null,
	instrument?: MarketInstrument,
	enabled = true,
) {
	return useQuery({
		queryKey: [
			"market-calendar",
			userId,
			market,
			instrument?.instrument.venue.id,
		],
		queryFn: async (): Promise<CalendarDto | undefined> => {
			if (!userId || !instrument) return undefined;
			const now = Date.now();
			const request = {
				userId,
				startTimeMs: now - 120 * 86_400_000,
				endTimeMs: now + 120 * 86_400_000,
				operationId: `${market}-calendar-${instrument.instrument.venue.id}`,
			};
			if (market === "a-shares") {
				const values = await invoke<CalendarDto[]>("ashare_calendar_acquire", {
					request,
				});
				return (
					values.find(
						(value) => value.snapshot.venue.id === instrument.instrument.venue.id,
					) ?? values[0]
				);
			}
			return invoke<CalendarDto>("alpaca_calendar_acquire", {
				request: { ...request, venue: instrument.instrument.venue },
			});
		},
		enabled: Boolean(userId && instrument && enabled),
		staleTime: 5 * 60_000,
		gcTime: 30 * 60_000,
	});
}
