import {
	AreaSeries,
	CandlestickSeries,
	ColorType,
	HistogramSeries,
	LineSeries,
	createChart,
	createSeriesMarkers,
} from "lightweight-charts";
import { useEffect, useRef, useState } from "react";
import type { BacktestRun } from "./backtest-page";
import { formatDecimal } from "./format-decimal";

export function BacktestChart({
	run,
	onVisibleRangeChange,
}: {
	run: BacktestRun;
	onVisibleRangeChange?: (startTimeMs: number, endTimeMs: number) => void;
}) {
	const container = useRef<HTMLDivElement>(null);
	const [hoveredExecution, setHoveredExecution] = useState<{
		openTimeMs: number;
		fills: BacktestRun["result"]["fills"];
	}>();
	const visibleRange = useRef<{ from: number; to: number } | undefined>(
		undefined,
	);
	useEffect(() => {
		if (!container.current) return;
		const chart = createChart(container.current, {
			autoSize: true,
			height: 680,
			layout: {
				background: { type: ColorType.Solid, color: "transparent" },
				textColor: "#888",
				attributionLogo: true,
				panes: { enableResize: true },
			},
			grid: {
				vertLines: { color: "rgba(128,128,128,.08)" },
				horzLines: { color: "rgba(128,128,128,.08)" },
			},
			timeScale: { timeVisible: true },
		});
		const candles = chart.addSeries(
			CandlestickSeries,
			{
				upColor: "#16a34a",
				downColor: "#dc2626",
				wickUpColor: "#16a34a",
				wickDownColor: "#dc2626",
				borderVisible: false,
			},
			0,
		);
		candles.setData(
			run.bars.map((bar) => ({
				time: (bar.openTimeMs / 1000) as never,
				open: Number(bar.open),
				high: Number(bar.high),
				low: Number(bar.low),
				close: Number(bar.close),
			})),
		);
		const volume = chart.addSeries(
			HistogramSeries,
			{ priceFormat: { type: "volume" }, priceScaleId: "volume" },
			0,
		);
		volume.priceScale().applyOptions({ scaleMargins: { top: 0.82, bottom: 0 } });
		volume.setData(
			run.bars.map((bar) => ({
				time: (bar.openTimeMs / 1000) as never,
				value: Number(bar.baseVolume),
				color:
					Number(bar.close) >= Number(bar.open)
						? "rgba(22,163,74,.35)"
						: "rgba(220,38,38,.35)",
			})),
		);
		const equity = chart.addSeries(
			LineSeries,
			{ color: "#2563eb", lineWidth: 2, title: "Strategy Equity" },
			1,
		);
		equity.setData(
			run.result.equity.map((point) => ({
				time: (point.openTimeMs / 1000) as never,
				value: Number(point.equity),
			})),
		);
		const benchmark = chart.addSeries(
			LineSeries,
			{ color: "#a1a1aa", lineWidth: 1, title: "Buy & Hold" },
			1,
		);
		benchmark.setData(
			run.result.benchmarkEquity.map((point) => ({
				time: (point.openTimeMs / 1000) as never,
				value: Number(point.equity),
			})),
		);
		const drawdown = chart.addSeries(
			AreaSeries,
			{
				lineColor: "#dc2626",
				topColor: "rgba(220,38,38,.35)",
				bottomColor: "rgba(220,38,38,.03)",
				title: "Drawdown",
			},
			2,
		);
		drawdown.setData(
			run.result.equity.map((point) => ({
				time: (point.openTimeMs / 1000) as never,
				value: Number(point.drawdown) * 100,
			})),
		);
		createSeriesMarkers(
			candles,
			run.result.fills.map((fill) => ({
				time: (fill.openTimeMs / 1000) as never,
				position:
					fill.side === "buy" ? ("belowBar" as const) : ("aboveBar" as const),
				shape: fill.side === "buy" ? ("arrowUp" as const) : ("arrowDown" as const),
				color: fill.side === "buy" ? "#16a34a" : "#dc2626",
				text: fill.side === "buy" ? "B" : "S",
			})),
		);
		const fillsByTime = new Map<number, BacktestRun["result"]["fills"]>();
		for (const fill of run.result.fills) {
			const fills = fillsByTime.get(fill.openTimeMs) ?? [];
			fills.push(fill);
			fillsByTime.set(fill.openTimeMs, fills);
		}
		chart.subscribeCrosshairMove(({ time }) => {
			if (typeof time !== "number") {
				setHoveredExecution(undefined);
				return;
			}
			const openTimeMs = Number(time) * 1000;
			const fills = fillsByTime.get(openTimeMs);
			setHoveredExecution(fills?.length ? { openTimeMs, fills } : undefined);
		});
		chart.panes()[0]?.setHeight(390);
		chart.panes()[1]?.setHeight(160);
		chart.panes()[2]?.setHeight(110);
		let rangeTimer: ReturnType<typeof setTimeout> | undefined;
		chart.timeScale().subscribeVisibleTimeRangeChange((range) => {
			if (!range || typeof range.from !== "number" || typeof range.to !== "number")
				return;
			clearTimeout(rangeTimer);
			const from = Number(range.from);
			const to = Number(range.to);
			visibleRange.current = { from, to };
			rangeTimer = setTimeout(
				() =>
					onVisibleRangeChange?.(Math.floor(from * 1000), Math.ceil(to * 1000) + 1),
				150,
			);
		});
		if (visibleRange.current)
			chart.timeScale().setVisibleRange(visibleRange.current as never);
		else chart.timeScale().fitContent();
		return () => {
			clearTimeout(rangeTimer);
			chart.remove();
		};
	}, [run, onVisibleRangeChange]);
	return (
		<div className="relative">
			<div
				ref={container}
				role="img"
				aria-label="Backtest candlesticks, fills, equity, and drawdown"
				className="w-full"
			/>
			{hoveredExecution && (
				<div className="pointer-events-none absolute right-3 top-3 z-10 max-w-[calc(100%-1.5rem)] rounded-md border bg-background/95 px-3 py-2 text-xs shadow-sm backdrop-blur">
					<p className="mb-1 text-muted-foreground">
						{new Date(hoveredExecution.openTimeMs).toLocaleString()}
					</p>
					{hoveredExecution.fills.map((fill) => (
						<div
							key={`${fill.orderId}:${fill.openTimeMs}`}
							className="flex flex-wrap items-center gap-x-3 gap-y-1 font-mono"
						>
							<span
								className={fill.side === "buy" ? "text-green-600" : "text-red-600"}
							>
								{fill.side === "buy" ? "BUY" : "SELL"}
							</span>
							<span>Price {formatDecimal(fill.price)}</span>
							<span>Qty {formatDecimal(fill.quantity)}</span>
							<span className="text-muted-foreground">{fill.role}</span>
						</div>
					))}
				</div>
			)}
		</div>
	);
}
