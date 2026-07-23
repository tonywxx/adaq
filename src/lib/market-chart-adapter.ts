export const BAR_INTERVALS = [
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

export type BarInterval = (typeof BAR_INTERVALS)[number];

export type OhlcvBar = {
	openTimeMs: number;
	open: string;
	high: string;
	low: string;
	close: string;
	baseVolume: string;
	quoteVolume: string;
};

export type BarGap = {
	startTimeMs: number;
	endTimeMs: number;
};

export type BarSeries = {
	src: string;
	code: string;
	interval: BarInterval;
	bars: OhlcvBar[];
	gaps: BarGap[];
};

type MarketChartDataItem = {
	time: number;
	open?: number;
	high?: number;
	low?: number;
	close?: number;
	volume?: number;
};

const FIXED_INTERVAL_MS: Partial<Record<BarInterval, number>> = {
	"1s": 1_000,
	"1m": 60_000,
	"3m": 3 * 60_000,
	"5m": 5 * 60_000,
	"15m": 15 * 60_000,
	"30m": 30 * 60_000,
	"1h": 60 * 60_000,
	"2h": 2 * 60 * 60_000,
	"4h": 4 * 60 * 60_000,
	"6h": 6 * 60 * 60_000,
	"12h": 12 * 60 * 60_000,
	"1d": 24 * 60 * 60_000,
	"2d": 2 * 24 * 60 * 60_000,
	"3d": 3 * 24 * 60 * 60_000,
	"5d": 5 * 24 * 60 * 60_000,
	"1w": 7 * 24 * 60 * 60_000,
};

export function toMarketChartData(
	bars: readonly OhlcvBar[],
	gaps: readonly BarGap[],
	interval: BarInterval,
) {
	const chartData: MarketChartDataItem[] = bars.map((bar) => ({
		time: bar.openTimeMs / 1000,
		open: Number(bar.open),
		high: Number(bar.high),
		low: Number(bar.low),
		close: Number(bar.close),
		volume: Number(bar.baseVolume),
	}));
	for (const gap of gaps) {
		for (
			let openTimeMs = gap.startTimeMs;
			openTimeMs < gap.endTimeMs;
			openTimeMs = nextOpenTimeMs(openTimeMs, interval)
		) {
			chartData.push({ time: openTimeMs / 1000 });
		}
	}
	return chartData.sort((left, right) => left.time - right.time);
}

function nextOpenTimeMs(openTimeMs: number, interval: BarInterval) {
	const fixed = FIXED_INTERVAL_MS[interval];
	if (fixed) return openTimeMs + fixed;

	const months = interval === "1mo" ? 1 : 3;
	const date = new Date(openTimeMs);
	return Date.UTC(date.getUTCFullYear(), date.getUTCMonth() + months, 1);
}
