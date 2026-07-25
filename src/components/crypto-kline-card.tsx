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
import WChart from "@/w/lightweight-charts/WChart";
import { useInfiniteQuery } from "@tanstack/react-query";
import { Channel, invoke } from "@tauri-apps/api/core";
import { LoaderCircleIcon } from "lucide-react";
import { useTheme } from "next-themes";
import {
	type ReactNode,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";

const BASE_REQUEST = {
	src: "okx",
	code: "BTC-USDT",
} as const;
const HISTORY_PAGE_BARS = 300;
const LEFT_EDGE_THRESHOLD_BARS = 5;

type BarRange = {
	startTimeMs: number;
	endTimeMs: number;
};

type BarStreamEvent =
	| { event: "connected" }
	| { event: "snapshot"; data: { bar: OhlcvBar; closed: boolean } }
	| { event: "error"; data: { message: string } }
	| { event: "reconnecting"; data: { delayMs: number } }
	| { event: "closed" };

type ConnectionStatus = "connecting" | "live" | "reconnecting";

export function CryptoKlineCard() {
	const { resolvedTheme } = useTheme();
	const [{ interval, endTimeMs }, setSelection] = useState<{
		interval: BarInterval;
		endTimeMs: number;
	}>(() => ({ interval: "15m", endTimeMs: Date.now() }));
	const [liveBar, setLiveBar] = useState<{
		interval: BarInterval;
		bar: OhlcvBar;
	}>();
	const [connectionStatus, setConnectionStatus] =
		useState<ConnectionStatus>("connecting");
	const [streamError, setStreamError] = useState<string>();
	const [crosshairBar, setCrosshairBar] = useState<OhlcvBar>();

	useEffect(() => {
		let disposed = false;
		const subscriptionId = crypto.randomUUID();
		const onEvent = new Channel<BarStreamEvent>();

		setConnectionStatus("connecting");
		setStreamError(undefined);
		onEvent.onmessage = (event) => {
			if (disposed) return;
			switch (event.event) {
				case "connected":
					setConnectionStatus("live");
					setStreamError(undefined);
					break;
				case "snapshot":
					setLiveBar({ interval, bar: event.data.bar });
					setConnectionStatus("live");
					setStreamError(undefined);
					break;
				case "error":
					setConnectionStatus("reconnecting");
					setStreamError(event.data.message);
					break;
				case "reconnecting":
					setConnectionStatus("reconnecting");
					break;
				case "closed":
					setConnectionStatus("reconnecting");
					break;
			}
		};

		void invoke("market_subscribe_bar", {
			request: { ...BASE_REQUEST, interval, subscriptionId },
			onEvent,
		}).catch((reason) => {
			if (!disposed) {
				setConnectionStatus("reconnecting");
				setStreamError(getErrorMessage(reason));
			}
		});

		return () => {
			disposed = true;
			void invoke("market_unsubscribe_bar", {
				request: { subscriptionId },
			});
		};
	}, [interval]);

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
			BASE_REQUEST.src,
			BASE_REQUEST.code,
			interval,
			endTimeMs,
		],
		initialPageParam: {
			startTimeMs: subtractBarIntervals(
				endTimeMs,
				interval,
				HISTORY_PAGE_BARS,
			),
			endTimeMs,
		} satisfies BarRange,
		queryFn: ({ pageParam }) =>
			invoke<BarSeries>("market_get_bar_series", {
				request: { ...BASE_REQUEST, interval, ...pageParam },
			}),
		getNextPageParam: (lastPage, _pages, lastPageParam) => {
			const earliest = lastPage.bars[0]?.openTimeMs;
			if (earliest === undefined || earliest >= lastPageParam.endTimeMs - 1) {
				return undefined;
			}
			return {
				startTimeMs: subtractBarIntervals(
					earliest,
					interval,
					HISTORY_PAGE_BARS,
				),
				endTimeMs: earliest,
			};
		},
		staleTime: 60_000,
	});
	const bars = useMemo(
		() => {
			const byTime = new Map(
				(data?.pages.flatMap((page) => page.bars) ?? []).map((bar) => [
					bar.openTimeMs,
					bar,
				]),
			);
			if (liveBar?.interval === interval) {
				byTime.set(liveBar.bar.openTimeMs, liveBar.bar);
			}
			return [...byTime.values()].sort(
				(left, right) => left.openTimeMs - right.openTimeMs,
			);
		},
		[data, interval, liveBar],
	);
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
						subtractBarIntervals(
							earliestTime,
							interval,
							LEFT_EDGE_THRESHOLD_BARS,
						)) /
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
	const handleIntervalChange = useCallback((value: string) => {
		if (!BAR_INTERVALS.includes(value as BarInterval)) return;
		setCrosshairBar(undefined);
		setLiveBar(undefined);
		setConnectionStatus("connecting");
		setSelection({ interval: value as BarInterval, endTimeMs: Date.now() });
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
		<div className="*:data-[slot=card]:from-primary/5 *:data-[slot=card]:to-card dark:*:data-[slot=card]:bg-card *:data-[slot=card]:bg-linear-to-t *:data-[slot=card]:shadow-xs">
			<Card className="@container/card rounded-md py-4">
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
					{detailBar ? (
						<BarDetails bar={detailBar} />
					) : (
						"Public market data from OKX"
					)}
				</CardFooter>
			</Card>
		</div>
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
