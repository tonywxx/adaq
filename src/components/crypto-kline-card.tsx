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
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "@/components/ui/select";
import {
	BAR_INTERVALS,
	type BarInterval,
	type BarSeries,
	type OhlcvBar,
	subtractBarIntervals,
	toMarketChartData,
} from "@/lib/market-chart-adapter";
import {
	barKey,
	getErrorMessage,
	instrumentKey,
	useMarketSessionStore,
} from "@/lib/market-session";
import WChart from "@/w/lightweight-charts/WChart";
import { useInfiniteQuery } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { LoaderCircleIcon, RotateCcwIcon } from "lucide-react";
import { useTheme } from "next-themes";
import { useTranslation } from "react-i18next";
import {
	type ReactNode,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";

const HISTORY_PAGE_BARS = 300;
const LEFT_EDGE_THRESHOLD_BARS = 5;

type BarRange = {
	startTimeMs: number;
	endTimeMs: number;
};

export function CryptoKlineCard() {
	const { t } = useTranslation();
	const { resolvedTheme } = useTheme();
	const activeInstrument = useMarketSessionStore(
		(state) => state.activeInstrument,
	);
	const connectionStatus = useMarketSessionStore((state) => state.barStatus);
	const streamError = useMarketSessionStore((state) => state.streamError);
	const setMainChartInterval = useMarketSessionStore(
		(state) => state.setMainChartInterval,
	);
	const [{ interval, endTimeMs }, setSelection] = useState<{
		interval: BarInterval;
		endTimeMs: number;
	}>(() => ({ interval: "15m", endTimeMs: Date.now() }));
	const liveBar = useMarketSessionStore(
		(state) => state.liveBars[barKey(activeInstrument, interval)],
	);
	const [crosshairBar, setCrosshairBar] = useState<OhlcvBar>();
	const [chartResetKey, setChartResetKey] = useState(0);

	useEffect(() => {
		if (!activeInstrument.src || !activeInstrument.code) return;
		setCrosshairBar(undefined);
		setSelection((current) => ({ ...current, endTimeMs: Date.now() }));
	}, [activeInstrument.code, activeInstrument.src]);

	const {
		data,
		error,
		fetchNextPage,
		hasNextPage,
		isError,
		isFetching,
		isFetchingNextPage,
		isPending,
		refetch,
	} = useInfiniteQuery({
		queryKey: [
			"market-bar-series",
			activeInstrument.src,
			activeInstrument.code,
			interval,
			endTimeMs,
		],
		initialPageParam: {
			startTimeMs: subtractBarIntervals(endTimeMs, interval, HISTORY_PAGE_BARS),
			endTimeMs,
		} satisfies BarRange,
		queryFn: ({ pageParam }) =>
			invoke<BarSeries>("market_get_bar_series", {
				request: { ...activeInstrument, interval, ...pageParam },
			}),
		getNextPageParam: (lastPage, _pages, lastPageParam) => {
			const earliest = lastPage.bars[0]?.openTimeMs;
			if (earliest === undefined || earliest >= lastPageParam.endTimeMs - 1) {
				return undefined;
			}
			return {
				startTimeMs: subtractBarIntervals(earliest, interval, HISTORY_PAGE_BARS),
				endTimeMs: earliest,
			};
		},
		staleTime: 60_000,
	});
	const bars = useMemo(() => {
		const byTime = new Map(
			(data?.pages.flatMap((page) => page.bars) ?? []).map((bar) => [
				bar.openTimeMs,
				bar,
			]),
		);
		if (liveBar) {
			byTime.set(liveBar.openTimeMs, liveBar);
		}
		return [...byTime.values()].sort(
			(left, right) => left.openTimeMs - right.openTimeMs,
		);
	}, [data, liveBar]);
	const gaps = useMemo(
		() => data?.pages.flatMap((page) => page.gaps) ?? [],
		[data],
	);
	const chartData = useMemo(
		() => toMarketChartData(bars, gaps, interval),
		[bars, gaps, interval],
	);
	const barsByTime = useMemo(
		() => new Map(bars.map((bar) => [bar.openTimeMs / 1_000, bar])),
		[bars],
	);
	const latestBar = bars[bars.length - 1];
	const detailBar = crosshairBar ?? latestBar;
	const historyRef = useRef({
		earliestTime: bars[0]?.openTimeMs,
		thresholdSeconds: 0,
		fetchNextPage,
		hasNextPage,
		isFetchingNextPage,
	});
	const earliestTime = bars[0]?.openTimeMs;
	historyRef.current = {
		earliestTime,
		thresholdSeconds:
			earliestTime === undefined
				? 0
				: (earliestTime -
						subtractBarIntervals(earliestTime, interval, LEFT_EDGE_THRESHOLD_BARS)) /
					1_000,
		fetchNextPage,
		hasNextPage,
		isFetchingNextPage,
	};
	const barsByTimeRef = useRef(barsByTime);
	barsByTimeRef.current = barsByTime;

	const handleVisibleRangeChange = useCallback((range: unknown) => {
		const from = getRangeStart(range);
		const state = historyRef.current;
		if (
			from === undefined ||
			state.earliestTime === undefined ||
			!state.hasNextPage ||
			state.isFetchingNextPage ||
			from > state.earliestTime / 1_000 + state.thresholdSeconds
		) {
			return;
		}
		state.isFetchingNextPage = true;
		void state.fetchNextPage();
	}, []);
	const handleCrosshairMove = useCallback((param: unknown) => {
		const time = getCrosshairTime(param);
		setCrosshairBar(
			time === undefined ? undefined : barsByTimeRef.current.get(time),
		);
	}, []);
	const handleIntervalChange = useCallback(
		(value: string) => {
			if (!BAR_INTERVALS.includes(value as BarInterval)) return;
			setCrosshairBar(undefined);
			setMainChartInterval(value as BarInterval);
			setSelection({ interval: value as BarInterval, endTimeMs: Date.now() });
		},
		[setMainChartInterval],
	);
	const [baseAsset = activeInstrument.code, quoteAsset = ""] =
		activeInstrument.code.split("-");
	const isDark = resolvedTheme === "dark";
	const chartColors = isDark
		? {
				background: "#292929",
				border: "#3c3c3c",
				grid: "rgba(255, 255, 255, 0.06)",
				text: "#a8a8a8",
				watermark: "rgba(255, 255, 255, 0.04)",
			}
		: {
				background: "#ffffff",
				border: "#d8d8d8",
				grid: "rgba(17, 17, 17, 0.06)",
				text: "#666666",
				watermark: "rgba(17, 17, 17, 0.04)",
			};

	return (
		<div className="*:data-[slot=card]:from-primary/5 *:data-[slot=card]:to-card dark:*:data-[slot=card]:bg-card *:data-[slot=card]:bg-linear-to-t *:data-[slot=card]:shadow-xs">
			<Card className="@container/card rounded-md py-4">
				<CardHeader>
					<CardDescription>
						{t("market.instrumentVenue", { baseAsset, quoteAsset })}{" "}
						<Badge
							variant="outline"
							title={streamError}
							aria-live="polite"
							className={
								connectionStatus === "live"
									? "text-emerald-600 dark:text-emerald-400"
									: "text-amber-600 dark:text-amber-400"
							}
						>
							<span className="size-2 rounded-full bg-current" aria-hidden="true" />
							{connectionStatusLabel(connectionStatus, t)}
						</Badge>
					</CardDescription>
					<CardTitle className="text-2xl font-semibold tabular-nums @[250px]/card:text-3xl">
						{detailBar ? `${detailBar.close} ${quoteAsset}` : "—"}
					</CardTitle>
					<CardAction className="flex items-center gap-2">
						<Select value={interval} onValueChange={handleIntervalChange}>
							<SelectTrigger size="sm" aria-label={t("market.barInterval")}>
								<SelectValue>{formatInterval(interval, t)}</SelectValue>
							</SelectTrigger>
							<SelectContent align="end">
								{BAR_INTERVALS.map((value) => (
									<SelectItem key={value} value={value}>
										{formatInterval(value, t)}
									</SelectItem>
								))}
							</SelectContent>
						</Select>
						<Button
							size="sm"
							variant="outline"
							onClick={() => setChartResetKey((value) => value + 1)}
						>
							<RotateCcwIcon />
							{t("market.reset")}
						</Button>
					</CardAction>
				</CardHeader>
				<CardContent className="px-2 sm:px-4">
					{isPending ? (
						<ChartMessage>
							<LoaderCircleIcon className="size-4 animate-spin" />
							{t("market.loadingBars")}
						</ChartMessage>
					) : isError ? (
						<ChartMessage>
							<span>{getErrorMessage(error)}</span>
							<Button
								size="sm"
								variant="outline"
								loading={isFetching}
								loadingText={t("market.retrying")}
								onClick={() => refetch()}
							>
								{t("market.retry")}
							</Button>
						</ChartMessage>
					) : chartData.length === 0 ? (
						<ChartMessage>{t("market.noClosedBars")}</ChartMessage>
					) : (
						<div className="relative">
							{isFetching && !isPending && (
								<Badge
									variant="secondary"
									className="absolute top-2 left-2 z-10"
									aria-live="polite"
								>
									<LoaderCircleIcon className="size-3 animate-spin" />
									{isFetchingNextPage
										? t("market.loadingHistory")
										: t("market.updating")}
								</Badge>
							)}
							<div
								role="img"
								aria-label={t("market.chartAriaLabel", {
									instrument: activeInstrument.code,
									interval: formatInterval(interval, t),
								})}
							>
								<WChart
									key={`${instrumentKey(activeInstrument)}-${interval}-${chartResetKey}`}
									data={chartData}
									chartType="candlestick"
									height={360}
									autoSize
									backgroundColor={chartColors.background}
									textColor={chartColors.text}
									fontFamily="Geist Variable, sans-serif"
									vertGridColor={chartColors.grid}
									horzGridColor={chartColors.grid}
									timeVisible
									timeSecondsVisible={interval === "1s"}
									timeScaleBorderColor={chartColors.border}
									timeScaleRightOffset={5}
									timeScaleBarSpacing={8}
									priceScaleBorderColor={chartColors.border}
									showVolume
									showEma
									emaPeriod1={10}
									emaPeriod2={20}
									emaColor1="#f59e0b"
									emaColor2="#8b5cf6"
									watermarkVisible
									watermarkText={`${baseAsset} / ${quoteAsset} · OKX`}
									watermarkColor={chartColors.watermark}
									onCrosshairMove={handleCrosshairMove}
									onVisibleRangeChange={handleVisibleRangeChange}
								/>
							</div>
						</div>
					)}
				</CardContent>
				<CardFooter className="w-full">
					{detailBar ? <BarDetails bar={detailBar} /> : t("market.publicMarketData")}
				</CardFooter>
			</Card>
		</div>
	);
}

function BarDetails({ bar }: { bar: OhlcvBar }) {
	const { t } = useTranslation();
	return (
		<dl className="grid w-full grid-cols-2 gap-x-5 gap-y-2 text-xs sm:grid-cols-4 lg:grid-cols-7">
			<BarField
				label={t("market.utc")}
				value={new Date(bar.openTimeMs)
					.toISOString()
					.replace("T", " ")
					.slice(0, 16)}
			/>
			<BarField label={t("market.open")} value={bar.open} />
			<BarField label={t("market.high")} value={bar.high} />
			<BarField label={t("market.low")} value={bar.low} />
			<BarField label={t("market.close")} value={bar.close} />
			<BarField label={t("market.baseVolume")} value={bar.baseVolume} />
			<BarField label={t("market.quoteVolume")} value={bar.quoteVolume} />
		</dl>
	);
}

function BarField({ label, value }: { label: string; value: string }) {
	return (
		<div className="min-w-0">
			<dt className="text-muted-foreground">{label}</dt>
			<dd className="truncate font-medium tabular-nums">{value}</dd>
		</div>
	);
}

function ChartMessage({ children }: { children: ReactNode }) {
	return (
		<div className="flex h-[360px] flex-col items-center justify-center gap-3 text-sm text-muted-foreground">
			{children}
		</div>
	);
}

function getRangeStart(range: unknown) {
	if (typeof range !== "object" || range === null || !("from" in range)) {
		return undefined;
	}
	return typeof range.from === "number" ? range.from : undefined;
}

function getCrosshairTime(param: unknown) {
	if (typeof param !== "object" || param === null || !("time" in param)) {
		return undefined;
	}
	return typeof param.time === "number" ? param.time : undefined;
}

function connectionStatusLabel(
	status: "connecting" | "live" | "reconnecting",
	t: (key: string) => string,
) {
	switch (status) {
		case "connecting":
			return t("market.connecting");
		case "live":
			return t("market.live");
		case "reconnecting":
			return t("market.reconnecting");
	}
}

function formatInterval(interval: BarInterval, t: (key: string) => string) {
	return `${interval} ${t("market.utc")}`;
}
