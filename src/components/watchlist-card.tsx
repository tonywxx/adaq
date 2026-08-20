import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
	Card,
	CardAction,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import {
	Pagination,
	PaginationContent,
	PaginationItem,
	PaginationNext,
	PaginationPrevious,
} from "@/components/ui/pagination";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "@/components/ui/select";
import {
	type BarInterval,
	type BarSeries,
	subtractBarIntervals,
	toMarketChartData,
} from "@/lib/market-chart-adapter";
import {
	addWatchlistInstrument,
	barKey,
	calculateChange,
	formatNumber,
	getErrorMessage,
	type InstrumentRef,
	instrumentKey,
	MINI_CHART_INTERVALS,
	removeWatchlistInstrument,
	type SpotInstrument,
	setActiveInstrument,
	setMiniChartInterval,
	useMarketSessionStore,
} from "@/lib/market-session";
import { formatNumber as formatLocaleNumber } from "@/lib/i18n";
import WChart from "@/w/lightweight-charts/WChart";
import { marketMatches } from "@/features/markets/market-workspaces";
import { useQuery } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { LoaderCircleIcon, PlusIcon, SearchIcon, XIcon } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useEffect, useMemo, useState } from "react";

const MINI_CHART_BARS = 60;
const WATCHLIST_PAGE_SIZE = 10;
type MiniChartInterval = (typeof MINI_CHART_INTERVALS)[number];

export function WatchlistCard() {
	const { t } = useTranslation();
	const ready = useMarketSessionStore((state) => state.ready);
	const loadError = useMarketSessionStore((state) => state.loadError);
	const allWatchlist = useMarketSessionStore((state) => state.watchlist);
	const watchlist = useMemo(
		() =>
			allWatchlist.filter((instrument) => marketMatches(instrument, "crypto")),
		[allWatchlist],
	);
	const watchlistLimit = useMarketSessionStore((state) => state.watchlistLimit);
	const activeInstrument = useMarketSessionStore(
		(state) => state.activeInstrument,
	);
	const interval = useMarketSessionStore((state) => state.miniChartInterval);
	const tickerStatus = useMarketSessionStore((state) => state.tickerStatus);
	const tickerError = useMarketSessionStore((state) => state.tickerError);
	const catalog = useMarketSessionStore((state) => state.catalog);
	const [adding, setAdding] = useState(false);
	const [query, setQuery] = useState("");
	const [mutationError, setMutationError] = useState<string>();
	const [watchlistPage, setWatchlistPage] = useState(1);
	// The watchlist's primary live value is the ticker. Mini-bars are an
	// independent stream and must not make fresh ticker prices look offline.
	const live = tickerStatus === "live";
	const pageCount = Math.max(
		1,
		Math.ceil(watchlist.length / WATCHLIST_PAGE_SIZE),
	);
	const currentPage = Math.min(watchlistPage, pageCount);
	const visibleWatchlist = watchlist.slice(
		(currentPage - 1) * WATCHLIST_PAGE_SIZE,
		currentPage * WATCHLIST_PAGE_SIZE,
	);
	useEffect(() => {
		const activeIndex = watchlist.findIndex(
			(item) => instrumentKey(item) === instrumentKey(activeInstrument),
		);
		if (activeIndex >= 0)
			setWatchlistPage(Math.floor(activeIndex / WATCHLIST_PAGE_SIZE) + 1);
	}, [activeInstrument, watchlist]);

	return (
		<Card className="min-w-0 rounded-md py-4 lg:sticky lg:top-6 lg:max-h-[calc(100svh-6rem)]">
			<CardHeader>
				<CardTitle className="text-xl font-semibold">
					{t("market.watchlist")}
				</CardTitle>
				<CardDescription>
					{t("market.watchlistSummary", {
						count: watchlist.length,
						limit: watchlistLimit || "—",
					})}
				</CardDescription>
				<CardAction>
					<Badge
						variant="outline"
						title={tickerError}
						className={
							live
								? "text-emerald-600 dark:text-emerald-400"
								: "text-amber-600 dark:text-amber-400"
						}
					>
						<span className="size-2 rounded-full bg-current" aria-hidden="true" />
						{live ? t("market.live") : t("market.reconnecting")}
					</Badge>
				</CardAction>
			</CardHeader>

			<CardContent className="flex min-h-0 flex-1 flex-col gap-3">
				<div className="flex items-center gap-2">
					<Select
						value={interval}
						onValueChange={(value) => {
							if (!MINI_CHART_INTERVALS.includes(value as MiniChartInterval)) {
								return;
							}
							setMutationError(undefined);
							void setMiniChartInterval(value as MiniChartInterval).catch((error) =>
								setMutationError(getErrorMessage(error)),
							);
						}}
					>
						<SelectTrigger
							className="w-24"
							size="sm"
							aria-label={t("market.miniChartInterval")}
						>
							<SelectValue>{interval}</SelectValue>
						</SelectTrigger>
						<SelectContent>
							{MINI_CHART_INTERVALS.map((value) => (
								<SelectItem key={value} value={value}>
									{value}
								</SelectItem>
							))}
						</SelectContent>
					</Select>
					<Button
						type="button"
						size="sm"
						variant="outline"
						className="ml-auto"
						disabled={!ready || watchlist.length >= watchlistLimit}
						onClick={() => setAdding((value) => !value)}
					>
						<PlusIcon />
						{t("market.add")}
					</Button>
				</div>

				{adding && (
					<InstrumentPicker
						query={query}
						onQueryChange={setQuery}
						onClose={() => {
							setAdding(false);
							setQuery("");
						}}
						onError={setMutationError}
					/>
				)}

				<div className="grid grid-cols-[minmax(0,1fr)_5.5rem_5rem_4.25rem_1.75rem] gap-2 border-b px-2 pb-2 text-xs text-muted-foreground">
					<span>{t("market.instrument")}</span>
					<span aria-hidden="true">{t("market.chart")}</span>
					<span className="text-right">{t("market.price")}</span>
					<span className="text-right">{t("market.change")}</span>
					<span className="sr-only">{t("market.actions")}</span>
				</div>

				<div className="min-h-0 space-y-2 overflow-y-auto pr-1">
					{!ready ? (
						<div className="flex h-32 items-center justify-center gap-2 text-sm text-muted-foreground">
							{loadError ? (
								loadError
							) : (
								<>
									<LoaderCircleIcon className="size-4 animate-spin" />
									{t("market.loadingWatchlist")}
								</>
							)}
						</div>
					) : watchlist.length === 0 ? (
						<div className="flex h-32 items-center justify-center text-sm text-muted-foreground">
							{t("market.emptyWatchlist")}
						</div>
					) : (
						visibleWatchlist.map((instrument) => (
							<WatchlistRow
								key={instrumentKey(instrument)}
								instrument={instrument}
								interval={interval}
								active={instrumentKey(instrument) === instrumentKey(activeInstrument)}
								available={
									!catalog[instrumentKey(instrument)] ||
									catalog[instrumentKey(instrument)]?.status === "live"
								}
								onError={setMutationError}
							/>
						))
					)}
				</div>
				{ready && watchlist.length > WATCHLIST_PAGE_SIZE && (
					<Pagination>
						<PaginationContent>
							<PaginationItem>
								<PaginationPrevious
									disabled={currentPage === 1}
									onClick={() => setWatchlistPage(currentPage - 1)}
								/>
							</PaginationItem>
							<PaginationItem>
								<span className="px-3 text-sm" aria-current="page">
									{t("market.pageOf", {
										current: currentPage,
										total: pageCount,
									})}
								</span>
							</PaginationItem>
							<PaginationItem>
								<PaginationNext
									disabled={currentPage === pageCount}
									onClick={() => setWatchlistPage(currentPage + 1)}
								/>
							</PaginationItem>
						</PaginationContent>
					</Pagination>
				)}

				{mutationError && (
					<div className="text-xs text-destructive" role="status">
						{mutationError}
					</div>
				)}
			</CardContent>
		</Card>
	);
}

function InstrumentPicker({
	query,
	onQueryChange,
	onClose,
	onError,
}: {
	query: string;
	onQueryChange: (value: string) => void;
	onClose: () => void;
	onError: (value?: string) => void;
}) {
	const { t } = useTranslation();
	const watchlist = useMarketSessionStore((state) => state.watchlist);
	const catalog = useMarketSessionStore((state) => state.catalog);
	const catalogLoaded = useMarketSessionStore((state) => state.catalogLoaded);
	const { data = [], isFetching } = useQuery({
		queryKey: ["okx-spot-instruments"],
		queryFn: () =>
			invoke<SpotInstrument[]>("market_list_spot_instruments", {
				request: { src: "okx" },
			}),
		enabled: !catalogLoaded,
		staleTime: 5 * 60_000,
	});
	const instruments = catalogLoaded ? Object.values(catalog) : data;
	const existing = useMemo(
		() => new Set(watchlist.map(instrumentKey)),
		[watchlist],
	);
	const matches = instruments
		.filter(
			(instrument) =>
				instrument.status === "live" &&
				!existing.has(instrumentKey(instrument)) &&
				(!query.trim() ||
					instrument.code.toLowerCase().includes(query.trim().toLowerCase())),
		)
		.slice(0, 8);

	return (
		<div className="rounded-md border bg-muted/20 p-2">
			<div className="relative">
				<SearchIcon className="-translate-y-1/2 pointer-events-none absolute top-1/2 left-2.5 size-4 text-muted-foreground" />
				<Input
					autoFocus
					type="search"
					value={query}
					className="h-8 pr-8 pl-8"
					placeholder={t("market.searchInstruments")}
					onChange={(event) => onQueryChange(event.target.value)}
				/>
				<Button
					type="button"
					size="icon-xs"
					variant="ghost"
					className="-translate-y-1/2 absolute top-1/2 right-1"
					aria-label={t("market.closeInstrumentSearch")}
					onClick={onClose}
				>
					<XIcon />
				</Button>
			</div>
			<div className="mt-2 max-h-48 overflow-y-auto">
				{isFetching && instruments.length === 0 ? (
					<div className="flex items-center gap-2 px-2 py-3 text-xs text-muted-foreground">
						<LoaderCircleIcon className="size-3.5 animate-spin" />
						{t("market.loadingInstruments")}
					</div>
				) : matches.length === 0 ? (
					<div className="px-2 py-3 text-xs text-muted-foreground">
						{t("market.noAvailableMatches")}
					</div>
				) : (
					matches.map((instrument) => (
						<button
							key={instrumentKey(instrument)}
							type="button"
							className="flex w-full items-center justify-between rounded px-2 py-2 text-left text-sm hover:bg-muted focus-visible:bg-muted focus-visible:outline-none"
							onClick={() => {
								onError(undefined);
								void addWatchlistInstrument(instrument)
									.then(onClose)
									.catch((error) => onError(getErrorMessage(error)));
							}}
						>
							<span className="font-medium">{instrument.code}</span>
							<span className="text-xs text-muted-foreground">OKX Spot</span>
						</button>
					))
				)}
			</div>
		</div>
	);
}

function WatchlistRow({
	instrument,
	interval,
	active,
	available,
	onError,
}: {
	instrument: InstrumentRef;
	interval: BarInterval;
	active: boolean;
	available: boolean;
	onError: (value?: string) => void;
}) {
	const { t } = useTranslation();
	const ticker = useMarketSessionStore(
		(state) => state.tickers[instrumentKey(instrument)],
	);
	const liveBar = useMarketSessionStore(
		(state) => state.liveBars[barKey(instrument, interval)],
	);
	const endTimeMs = Date.now();
	const { data, isPending } = useQuery({
		queryKey: ["watchlist-mini-bars", instrument.src, instrument.code, interval],
		queryFn: () =>
			invoke<BarSeries>("market_get_bar_series", {
				request: {
					...instrument,
					interval,
					startTimeMs: subtractBarIntervals(endTimeMs, interval, MINI_CHART_BARS),
					endTimeMs,
				},
			}),
		enabled: available,
		staleTime: 60_000,
	});
	const chartData = useMemo(() => {
		const bars = new Map((data?.bars ?? []).map((bar) => [bar.openTimeMs, bar]));
		if (liveBar) bars.set(liveBar.openTimeMs, liveBar);
		return toMarketChartData(
			[...bars.values()]
				.sort((left, right) => left.openTimeMs - right.openTimeMs)
				.slice(-MINI_CHART_BARS),
			[],
			interval,
		).map(({ time, close }) => ({ time, value: close }));
	}, [data, interval, liveBar]);
	const change = ticker ? calculateChange(ticker.last, ticker.open24h) : null;
	const changeLabel =
		change === null
			? "—"
			: `${formatLocaleNumber(change, {
					minimumFractionDigits: 2,
					maximumFractionDigits: 2,
					signDisplay: "always",
				})}%`;
	const color =
		change === null ? "#64748b" : change >= 0 ? "#10b981" : "#f43f5e";

	return (
		<div
			className={`flex min-w-0 items-center rounded-md border transition-colors ${
				active ? "border-primary/50 bg-primary/8" : "border-transparent bg-muted/40"
			} ${available ? "" : "opacity-60"}`}
		>
			<button
				type="button"
				disabled={!available}
				className="grid min-w-0 flex-1 grid-cols-[minmax(0,1fr)_5.5rem_5rem_4.25rem] items-center gap-2 rounded-l-md px-2 py-2 text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
				onClick={() => {
					onError(undefined);
					void setActiveInstrument(instrument).catch((error) =>
						onError(getErrorMessage(error)),
					);
				}}
			>
				<span className="min-w-0">
					<span className="block truncate font-medium">{instrument.code}</span>
					<span className="block text-xs text-muted-foreground">
						{available ? "OKX Spot" : t("market.unavailable")}
					</span>
				</span>
				<span className="h-12 min-w-0 overflow-hidden" aria-hidden="true">
					{!isPending && chartData.length > 0 ? (
						<WChart
							data={chartData}
							chartType="baseline"
							height={48}
							autoSize
							isMiniChart
							backgroundColor="transparent"
							lineColor={color}
							vertGridVisible={false}
							horzGridVisible={false}
							timeScaleBorderVisible={false}
							priceScaleBorderVisible={false}
							priceScalePosition="none"
							crosshairMode="hidden"
							showEma={false}
							timeScaleRightOffset={0}
						/>
					) : null}
				</span>
				<span className="truncate text-right font-semibold tabular-nums">
					{ticker ? formatNumber(ticker.last) : "—"}
				</span>
				<span
					className={`text-right font-semibold tabular-nums ${
						change === null
							? "text-muted-foreground"
							: change >= 0
								? "text-emerald-600 dark:text-emerald-400"
								: "text-rose-600 dark:text-rose-400"
					}`}
				>
					{changeLabel}
				</span>
			</button>
			<Button
				type="button"
				size="icon-xs"
				variant="ghost"
				className="mr-0.5 shrink-0 text-muted-foreground hover:text-destructive"
				aria-label={t("market.removeFromWatchlist", {
					instrument: instrument.code,
				})}
				onClick={() => {
					onError(undefined);
					void removeWatchlistInstrument(instrument).catch((error) =>
						onError(getErrorMessage(error)),
					);
				}}
			>
				<XIcon />
			</Button>
		</div>
	);
}
