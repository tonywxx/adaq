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
	type BarSeries,
	type OhlcvBar,
	toMarketChartData,
} from "@/lib/market-chart-adapter";
import WChart from "@/w/lightweight-charts/WChart";
import { useInfiniteQuery } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { LoaderCircleIcon } from "lucide-react";
import { useTheme } from "next-themes";
import { type ReactNode, useCallback, useMemo, useRef, useState } from "react";

const REQUEST = {
	src: "okx",
	code: "BTC-USDT",
	interval: "1d",
} as const;
const DAY_MS = 86_400_000;
const HISTORY_PAGE_MS = 120 * DAY_MS;
const LEFT_EDGE_THRESHOLD_SECONDS = (5 * DAY_MS) / 1_000;

type BarRange = {
	startTimeMs: number;
	endTimeMs: number;
};

export function CryptoKlineCard() {
	const { resolvedTheme } = useTheme();
	const [initialEndTimeMs] = useState(Date.now);
	const [crosshairBar, setCrosshairBar] = useState<OhlcvBar>();
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
		queryKey: ["market-bar-series", REQUEST],
		initialPageParam: {
			startTimeMs: initialEndTimeMs - HISTORY_PAGE_MS,
			endTimeMs: initialEndTimeMs,
		} satisfies BarRange,
		queryFn: ({ pageParam }) =>
			invoke<BarSeries>("market_get_bar_series", {
				request: { ...REQUEST, ...pageParam },
			}),
		getNextPageParam: (lastPage, _pages, lastPageParam) => {
			const earliest = lastPage.bars[0]?.openTimeMs;
			if (earliest === undefined || earliest >= lastPageParam.endTimeMs - 1) {
				return undefined;
			}
			return {
				startTimeMs: Math.max(0, earliest - HISTORY_PAGE_MS),
				endTimeMs: earliest + 1,
			};
		},
		staleTime: 60_000,
	});
	const bars = useMemo(
		() =>
			(data?.pages.flatMap((page) => page.bars) ?? []).sort(
				(left, right) => left.openTimeMs - right.openTimeMs,
			),
		[data],
	);
	const gaps = useMemo(
		() => data?.pages.flatMap((page) => page.gaps) ?? [],
		[data],
	);
	const chartData = useMemo(
		() => toMarketChartData(bars, gaps, REQUEST.interval),
		[bars, gaps],
	);
	const barsByTime = useMemo(
		() => new Map(bars.map((bar) => [bar.openTimeMs / 1_000, bar])),
		[bars],
	);
	const latestBar = bars[bars.length - 1];
	const detailBar = crosshairBar ?? latestBar;
	const historyRef = useRef({
		earliestTime: bars[0]?.openTimeMs,
		fetchNextPage,
		hasNextPage,
		isFetchingNextPage,
	});
	historyRef.current = {
		earliestTime: bars[0]?.openTimeMs,
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
			from > state.earliestTime / 1_000 + LEFT_EDGE_THRESHOLD_SECONDS
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
		<Card className="@container/card rounded-none bg-linear-to-t from-primary/5 to-card py-4 shadow-xs dark:bg-card">
			<CardHeader>
				<CardDescription>BTC / USDT · OKX Spot</CardDescription>
				<CardTitle className="text-2xl font-semibold tabular-nums @[250px]/card:text-3xl">
					{detailBar ? `${detailBar.close} USDT` : "—"}
				</CardTitle>
				<CardAction>
					<Badge variant="outline">
						{isFetchingNextPage
							? "Loading history…"
							: isFetching && !isPending
								? "Updating…"
								: "1D UTC"}
					</Badge>
				</CardAction>
			</CardHeader>
			<CardContent className="px-2 sm:px-4">
				{isPending ? (
					<ChartMessage>
						<LoaderCircleIcon className="size-4 animate-spin" />
						Loading OKX bars…
					</ChartMessage>
				) : isError ? (
					<ChartMessage>
						<span>{getErrorMessage(error)}</span>
						<Button size="sm" variant="outline" onClick={() => refetch()}>
							Retry
						</Button>
					</ChartMessage>
				) : chartData.length === 0 ? (
					<ChartMessage>No closed bars returned by OKX.</ChartMessage>
				) : (
					<div role="img" aria-label="BTC USDT daily candlestick chart from OKX">
						<WChart
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
							timeScaleBorderColor={chartColors.border}
							priceScaleBorderColor={chartColors.border}
							showVolume
							showEma
							emaPeriod1={10}
							emaPeriod2={20}
							emaColor1="#f59e0b"
							emaColor2="#8b5cf6"
							watermarkVisible
							watermarkText="BTC / USDT · OKX"
							watermarkColor={chartColors.watermark}
							onCrosshairMove={handleCrosshairMove}
							onVisibleRangeChange={handleVisibleRangeChange}
						/>
					</div>
				)}
			</CardContent>
			<CardFooter className="w-full">
				{detailBar ? <BarDetails bar={detailBar} /> : "Public market data from OKX"}
			</CardFooter>
		</Card>
	);
}

function BarDetails({ bar }: { bar: OhlcvBar }) {
	return (
		<dl className="grid w-full grid-cols-2 gap-x-5 gap-y-2 text-xs sm:grid-cols-4 lg:grid-cols-7">
			<BarField
				label="UTC"
				value={new Date(bar.openTimeMs)
					.toISOString()
					.replace("T", " ")
					.slice(0, 16)}
			/>
			<BarField label="Open" value={bar.open} />
			<BarField label="High" value={bar.high} />
			<BarField label="Low" value={bar.low} />
			<BarField label="Close" value={bar.close} />
			<BarField label="Base volume" value={bar.baseVolume} />
			<BarField label="Quote volume" value={bar.quoteVolume} />
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

function getErrorMessage(error: unknown) {
	if (
		typeof error === "object" &&
		error !== null &&
		"message" in error &&
		typeof error.message === "string"
	) {
		return error.message;
	}
	return String(error);
}
