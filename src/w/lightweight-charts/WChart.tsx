import { DOWN_COLOR, UP_COLOR } from "@/config/trade-theme";
import { getDecimalLength } from "@/w/number/getDecimalLength";
import { getDecimalMinMove } from "@/w/number/getDecimalMinMove";
import {
	AreaSeries,
	BarSeries,
	BaselineSeries,
	CandlestickSeries,
	ColorType,
	CrosshairMode,
	createChart,
	type HistogramData,
	HistogramSeries,
	type IChartApi,
	type ISeriesApi,
	LastPriceAnimationMode,
	type LineData,
	LineSeries,
	LineStyle,
	LineType,
	type LineWidth,
	PriceScaleMode,
	type Time,
	type WhitespaceData,
} from "lightweight-charts";
import type React from "react";
import { useEffect, useRef } from "react";

type ChartType =
	| "candlestick"
	| "line"
	| "area"
	| "bar"
	| "histogram"
	| "baseline";

interface ChartDataItem {
	time: number;
	open?: number;
	high?: number;
	low?: number;
	close?: number;
	value?: number;
	volume?: number;
}

interface WChartProps {
	data?: ChartDataItem[];
	chartType?: ChartType;
	width?: number;
	height?: number;
	autoSize?: boolean;
	isMiniChart?: boolean;
	backgroundColor?: string;
	textColor?: string;
	fontSize?: number;
	fontFamily?: string;
	vertGridVisible?: boolean;
	horzGridVisible?: boolean;
	vertGridColor?: string;
	horzGridColor?: string;
	timeVisible?: boolean;
	timeSecondsVisible?: boolean;
	timeScaleBorderVisible?: boolean;
	timeScaleBorderColor?: string;
	timeScaleRightOffset?: number;
	priceScaleMode?: "normal" | "logarithmic" | "percentage" | "indexedTo100";
	autoScale?: boolean;
	priceScaleBorderVisible?: boolean;
	priceScaleBorderColor?: string;
	priceScalePosition?: "left" | "right" | "none";
	lastPriceAnimationMode?: LastPriceAnimationMode;
	crosshairMode?: "normal" | "magnet" | "hidden";
	crosshairVertColor?: string;
	crosshairHorzColor?: string;
	crosshairVertStyle?: 0 | 1 | 2 | 3;
	crosshairHorzStyle?: 0 | 1 | 2 | 3;
	upColor?: string;
	downColor?: string;
	borderVisible?: boolean;
	borderUpColor?: string;
	borderDownColor?: string;
	wickUpColor?: string;
	wickDownColor?: string;
	lineColor?: string;
	lineWidth?: number;
	lineStyle?: 0 | 1 | 2 | 3;
	lineType?: 0 | 1 | 2;
	topColor?: string;
	bottomColor?: string;
	watermarkText?: string;
	watermarkColor?: string;
	watermarkFontSize?: number;
	watermarkVisible?: boolean;
	onClick?: (param: unknown) => void;
	onCrosshairMove?: (param: unknown) => void;
	onVisibleRangeChange?: (range: unknown) => void;
	baselinePrice?: number;
	showHighPriceLine?: boolean;
	highPriceLineColor?: string;
	showLowPriceLine?: boolean;
	lowPriceLineColor?: string;
	showAvgPriceLine?: boolean;
	avgPriceLineColor?: string;
	priceLineStyle?: LineStyle;
	highPriceLineStyle?: LineStyle;
	lowPriceLineStyle?: LineStyle;
	avgPriceLineStyle?: LineStyle;
	showVolume?: boolean;
	showVolumeLabel?: boolean;
	volumeUpColor?: string;
	volumeDownColor?: string;
	showEma?: boolean;
	emaPeriod1?: number;
	emaPeriod2?: number;
	emaColor1?: string;
	emaColor2?: string;
	emaLineWidth?: number;
}

type SeriesApi = ISeriesApi<any>;

const WChart: React.FC<WChartProps> = ({
	data = [],
	chartType = "candlestick",
	width = 800,
	height = 400,
	autoSize = true,
	isMiniChart = false,
	backgroundColor = "#ffffff",
	textColor = "#677489",
	fontSize = 12,
	fontFamily = "Arial, sans-serif",
	vertGridVisible = true,
	horzGridVisible = true,
	vertGridColor = "rgba(197, 203, 206, 0.5)",
	horzGridColor = "rgba(197, 203, 206, 0.5)",
	timeVisible = false,
	timeSecondsVisible = false,
	timeScaleBorderVisible = true,
	timeScaleBorderColor = "#2B2B43",
	timeScaleRightOffset = 6,
	priceScaleMode = "normal",
	autoScale = true,
	priceScaleBorderVisible = true,
	priceScaleBorderColor = "#2B2B43",
	priceScalePosition = "right",
	lastPriceAnimationMode = LastPriceAnimationMode.OnDataUpdate,
	crosshairMode = "normal",
	crosshairVertColor = "#758696",
	crosshairHorzColor = "#758696",
	crosshairVertStyle = 3,
	crosshairHorzStyle = 3,
	upColor = "#26a69a",
	downColor = "#ef5350",
	borderVisible = false,
	borderUpColor = "#4A4A4A",
	borderDownColor = "#4A4A4A",
	wickUpColor = "#26a69a",
	wickDownColor = "#ef5350",
	lineColor = "#2962FF",
	lineWidth = 2,
	lineStyle = LineStyle.Solid,
	lineType = LineType.Simple,
	topColor = "rgba(41, 98, 255, 0.28)",
	bottomColor = "rgba(41, 98, 255, 0.05)",
	watermarkText = "",
	watermarkColor = "rgba(0, 0, 0, 0.1)",
	watermarkFontSize = 48,
	watermarkVisible = false,
	onClick,
	onCrosshairMove,
	onVisibleRangeChange,
	showHighPriceLine = false,
	highPriceLineColor = "#2962FF",
	showLowPriceLine = false,
	lowPriceLineColor = "#758696",
	showAvgPriceLine = false,
	avgPriceLineColor = "#EAB308",
	priceLineStyle = LineStyle.Dotted,
	highPriceLineStyle = LineStyle.Solid,
	lowPriceLineStyle = LineStyle.Solid,
	avgPriceLineStyle = LineStyle.Solid,
	showVolume = false,
	showVolumeLabel = false,
	volumeUpColor = UP_COLOR,
	volumeDownColor = DOWN_COLOR,
	showEma = true,
	emaPeriod1 = 10,
	emaPeriod2 = 20,
	emaColor1 = "#FF8C00",
	emaColor2 = "#8A2BE2",
	emaLineWidth = 1,
}) => {
	const containerRef = useRef<HTMLDivElement>(null);
	const chartRef = useRef<IChartApi | null>(null);
	const seriesRef = useRef<SeriesApi | null>(null);
	const volumeSeriesRef = useRef<ISeriesApi<"Histogram"> | null>(null);
	const highPriceLineRef = useRef<ReturnType<
		SeriesApi["createPriceLine"]
	> | null>(null);
	const lowPriceLineRef = useRef<ReturnType<
		SeriesApi["createPriceLine"]
	> | null>(null);
	const avgPriceLineRef = useRef<ReturnType<
		SeriesApi["createPriceLine"]
	> | null>(null);
	const prevDataRef = useRef<ChartDataItem[]>([]);
	const resizeObserverRef = useRef<ResizeObserver | null>(null);
	const fitContentTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
	const ema1SeriesRef = useRef<ISeriesApi<"Line"> | null>(null);
	const ema2SeriesRef = useRef<ISeriesApi<"Line"> | null>(null);

	useEffect(() => {
		const container = containerRef.current;
		if (!container) return;
		const safeData = normalizeChartData(data);

		const chartOptions: any = {
			width: autoSize ? container.clientWidth : width,
			height,
			layout: {
				attributionLogo: true,
				background: {
					type: ColorType.Solid,
					color: backgroundColor,
				},
				textColor,
				fontSize,
				fontFamily,
			},
			grid: {
				vertLines: { visible: vertGridVisible, color: vertGridColor },
				horzLines: { visible: horzGridVisible, color: horzGridColor },
			},
			timeScale: {
				visible: timeVisible,
				secondsVisible: timeSecondsVisible,
				borderVisible: timeScaleBorderVisible,
				borderColor: timeScaleBorderColor,
				timeVisible: timeVisible,
			},
			rightPriceScale: {
				visible: priceScalePosition === "right",
				borderVisible: priceScaleBorderVisible,
				borderColor: priceScaleBorderColor,
				mode: getModeValue(priceScaleMode),
				autoScale,
			},
			leftPriceScale: {
				visible: priceScalePosition === "left",
				borderVisible: priceScaleBorderVisible,
				borderColor: priceScaleBorderColor,
			},
			crosshair: {
				mode: getCrosshairMode(crosshairMode),
				vertLine: {
					color: crosshairVertColor,
					style: crosshairVertStyle as LineStyle,
				},
				horzLine: {
					color: crosshairHorzColor,
					style: crosshairHorzStyle as LineStyle,
				},
			},
		};

		if (watermarkVisible && watermarkText) {
			chartOptions.watermark = {
				visible: true,
				fontSize: watermarkFontSize,
				color: watermarkColor,
				text: watermarkText,
			};
		}

		const chart = createChart(container, chartOptions);
		chartRef.current = chart;

		const series = createSeries(chart, chartType, {
			upColor,
			downColor,
			borderVisible,
			borderUpColor,
			borderDownColor,
			wickUpColor,
			wickDownColor,
			lineColor,
			lineWidth: isMiniChart ? 1 : lineWidth,
			lineStyle: lineStyle as LineStyle,
			lineType: lineType as LineType,
			lastPriceAnimationMode,
			topColor,
			bottomColor,
			priceLineStyle,
			data: safeData,
		});
		seriesRef.current = series;

		let volumeSeries: ISeriesApi<"Histogram"> | null = null;
		if (showVolume) {
			volumeSeries = chart.addSeries(HistogramSeries, {
				color: "#26a69a",
				priceFormat: { type: "volume", precision: 0 },
				lastValueVisible: showVolumeLabel,
				priceScaleId: "volume",
			});
			volumeSeries
				.priceScale()
				.applyOptions({ scaleMargins: { top: 0.8, bottom: 0 } });
			volumeSeriesRef.current = volumeSeries;
		}

		const shouldShowEma = showEma && !isMiniChart && chartType === "candlestick";
		let ema1: ISeriesApi<"Line"> | null = null;
		let ema2: ISeriesApi<"Line"> | null = null;
		if (shouldShowEma) {
			ema1 = chart.addSeries(LineSeries, {
				color: emaColor1,
				lineWidth: emaLineWidth as LineWidth,
				priceLineVisible: false,
				lastValueVisible: true,
				title: `E${emaPeriod1}`,
			});
			ema2 = chart.addSeries(LineSeries, {
				color: emaColor2,
				lineWidth: emaLineWidth as LineWidth,
				priceLineVisible: false,
				lastValueVisible: true,
				title: `E${emaPeriod2}`,
			});
			ema1SeriesRef.current = ema1;
			ema2SeriesRef.current = ema2;
		}

		if (safeData.length > 0) {
			series.setData(safeData as any);
			if (volumeSeries) {
				volumeSeries.setData(safeData.map(toVolumeData));
			}
			if (shouldShowEma && ema1 && ema2) {
				const ema1Data = calcEMA(safeData, emaPeriod1);
				const ema2Data = calcEMA(safeData, emaPeriod2);
				if (ema1Data.length > 0) ema1.setData(ema1Data);
				if (ema2Data.length > 0) ema2.setData(ema2Data);
			}
		}

		updatePriceLines(
			series,
			safeData,
			{
				showHighPriceLine,
				highPriceLineColor,
				showLowPriceLine,
				lowPriceLineColor,
				showAvgPriceLine,
				avgPriceLineColor,
				highPriceLineStyle,
				lowPriceLineStyle,
				avgPriceLineStyle,
				isMiniChart,
			},
			{
				highPriceLineRef,
				lowPriceLineRef,
				avgPriceLineRef,
			},
		);

		if (onClick) chart.subscribeClick(onClick);
		if (onCrosshairMove) chart.subscribeCrosshairMove(onCrosshairMove);
		if (onVisibleRangeChange)
			chart.timeScale().subscribeVisibleTimeRangeChange(onVisibleRangeChange);

		if (autoSize) {
			resizeObserverRef.current = new ResizeObserver((entries) => {
				const { width: w, height: h } = entries[0].contentRect;
				if (w > 0 && h > 0) {
					chart.applyOptions({ width: w, height: h });
				}
			});
			resizeObserverRef.current.observe(container);
		}

		prevDataRef.current = safeData;

		chart
			.timeScale()
			.applyOptions({ fixLeftEdge: true, rightOffset: timeScaleRightOffset });
		fitDataWithRightOffset(safeData.length);
		chart.timeScale().fitContent();

		return () => {
			if (resizeObserverRef.current) {
				resizeObserverRef.current.disconnect();
				resizeObserverRef.current = null;
			}
			if (fitContentTimerRef.current) {
				clearTimeout(fitContentTimerRef.current);
				fitContentTimerRef.current = null;
			}
			chart.remove();
			chartRef.current = null;
			seriesRef.current = null;
			volumeSeriesRef.current = null;
			ema1SeriesRef.current = null;
			ema2SeriesRef.current = null;
			highPriceLineRef.current = null;
			lowPriceLineRef.current = null;
			avgPriceLineRef.current = null;
			prevDataRef.current = [];
		};
	}, [
		chartType,
		height,
		autoSize,
		backgroundColor,
		textColor,
		fontSize,
		fontFamily,
		vertGridVisible,
		horzGridVisible,
		vertGridColor,
		horzGridColor,
		timeVisible,
		timeSecondsVisible,
		timeScaleBorderVisible,
		timeScaleBorderColor,
		timeScaleRightOffset,
		priceScaleMode,
		autoScale,
		priceScaleBorderVisible,
		priceScaleBorderColor,
		priceScalePosition,
		lastPriceAnimationMode,
		crosshairMode,
		crosshairVertColor,
		crosshairHorzColor,
		crosshairVertStyle,
		crosshairHorzStyle,
		upColor,
		downColor,
		borderVisible,
		borderUpColor,
		borderDownColor,
		wickUpColor,
		wickDownColor,
		lineColor,
		lineStyle,
		lineType,
		topColor,
		bottomColor,
		watermarkText,
		watermarkColor,
		watermarkFontSize,
		watermarkVisible,
		showHighPriceLine,
		highPriceLineColor,
		showLowPriceLine,
		lowPriceLineColor,
		showAvgPriceLine,
		avgPriceLineColor,
		showVolume,
		showVolumeLabel,
		volumeUpColor,
		volumeDownColor,
		priceLineStyle,
		highPriceLineStyle,
		lowPriceLineStyle,
		avgPriceLineStyle,
		showEma,
		emaPeriod1,
		emaPeriod2,
		emaColor1,
		emaColor2,
		emaLineWidth,
	]);

	useEffect(() => {
		const series = seriesRef.current;
		if (!series) return;

		const safeData = normalizeChartData(data);
		const prevData = prevDataRef.current;
		const previousVisibleRange = chartRef.current
			?.timeScale()
			.getVisibleLogicalRange();
		const prependedCount =
			prevData.length > 0 &&
			safeData[safeData.length - 1]?.time === prevData[prevData.length - 1]?.time
				? safeData.findIndex((item) => item.time === prevData[0]?.time)
				: 0;

		if (safeData.length === 0) {
			try {
				series.setData([]);
				volumeSeriesRef.current?.setData([]);
				ema1SeriesRef.current?.setData([]);
				ema2SeriesRef.current?.setData([]);
				updatePriceLines(
					series,
					[],
					{
						showHighPriceLine,
						highPriceLineColor,
						showLowPriceLine,
						lowPriceLineColor,
						showAvgPriceLine,
						avgPriceLineColor,
						highPriceLineStyle,
						lowPriceLineStyle,
						avgPriceLineStyle,
						isMiniChart,
					},
					{
						highPriceLineRef,
						lowPriceLineRef,
						avgPriceLineRef,
					},
				);
			} catch (error) {
				console.error("[WChart] Failed to clear series data", error);
			}
			prevDataRef.current = [];
			return;
		}

		const isNewData =
			prevData.length === 0 ||
			safeData.length !== prevData.length ||
			safeData[0]?.time !== prevData[0]?.time ||
			safeData[safeData.length - 1]?.time !== prevData[prevData.length - 1]?.time;

		try {
			if (isNewData) {
				series.setData(safeData as any);
			} else {
				series.update(safeData[safeData.length - 1] as any);
			}
		} catch (error) {
			console.error("[WChart] Failed to update series; resetting data", error);
			try {
				series.setData(safeData as any);
			} catch (resetError) {
				console.error("[WChart] Failed to reset series data", resetError);
				return;
			}
		}
		prevDataRef.current = safeData;

		series.applyOptions({
			priceLineStyle,
			priceFormat: {
				precision: getDecimalLength(safeData[0].open ?? 0),
				minMove: getDecimalMinMove(safeData[0].open ?? 0),
			},
		});

		if (chartType === "baseline") {
			const total = safeData.reduce(
				(acc, item) => acc + (item.close ?? item.value ?? 0),
				0,
			);
			series.applyOptions({
				baseValue: { type: "price" as const, price: total / safeData.length },
			});
		}

		if (volumeSeriesRef.current && showVolume) {
			const volData = safeData.map(toVolumeData);
			if (isNewData) {
				volumeSeriesRef.current.setData(volData);
			} else {
				volumeSeriesRef.current.update(volData[volData.length - 1]);
			}
		}

		const shouldShowEma = showEma && !isMiniChart && chartType === "candlestick";
		if (shouldShowEma && ema1SeriesRef.current && ema2SeriesRef.current) {
			const ema1Data = calcEMA(safeData, emaPeriod1);
			const ema2Data = calcEMA(safeData, emaPeriod2);
			if (isNewData) {
				if (ema1Data.length > 0) ema1SeriesRef.current.setData(ema1Data);
				if (ema2Data.length > 0) ema2SeriesRef.current.setData(ema2Data);
			} else {
				if (ema1Data.length > 0)
					ema1SeriesRef.current.update(ema1Data[ema1Data.length - 1]);
				if (ema2Data.length > 0)
					ema2SeriesRef.current.update(ema2Data[ema2Data.length - 1]);
			}
		}

		updatePriceLines(
			series,
			safeData,
			{
				showHighPriceLine,
				highPriceLineColor,
				showLowPriceLine,
				lowPriceLineColor,
				showAvgPriceLine,
				avgPriceLineColor,
				highPriceLineStyle,
				lowPriceLineStyle,
				avgPriceLineStyle,
				isMiniChart,
			},
			{
				highPriceLineRef,
				lowPriceLineRef,
				avgPriceLineRef,
			},
		);

		if (prependedCount > 0 && previousVisibleRange && chartRef.current) {
			chartRef.current.timeScale().setVisibleLogicalRange({
				from: previousVisibleRange.from + prependedCount,
				to: previousVisibleRange.to + prependedCount,
			});
		} else {
			debouncedFitContent(safeData.length);
		}
	}, [
		data,
		chartType,
		showVolume,
		volumeUpColor,
		volumeDownColor,
		showHighPriceLine,
		highPriceLineColor,
		showLowPriceLine,
		lowPriceLineColor,
		showAvgPriceLine,
		avgPriceLineColor,
		priceLineStyle,
		highPriceLineStyle,
		lowPriceLineStyle,
		avgPriceLineStyle,
		showEma,
		emaPeriod1,
		emaPeriod2,
		timeScaleRightOffset,
	]);

	function debouncedFitContent(dataLength: number) {
		if (fitContentTimerRef.current) clearTimeout(fitContentTimerRef.current);
		fitContentTimerRef.current = setTimeout(() => {
			fitDataWithRightOffset(dataLength);
			fitContentTimerRef.current = null;
		}, 200);
	}

	function fitDataWithRightOffset(dataLength: number) {
		const chart = chartRef.current;
		if (!chart) return;

		if (dataLength <= 0) {
			chart.timeScale().fitContent();
			return;
		}

		chart.timeScale().setVisibleLogicalRange({
			from: 0,
			to: dataLength - 1 + Math.max(timeScaleRightOffset, 0),
		});
	}

	return (
		<div
			ref={containerRef}
			style={{
				width: autoSize ? "100%" : width,
				height,
				position: "relative",
			}}
		/>
	);
};

function createSeries(chart: IChartApi, type: ChartType, opts: any): SeriesApi {
	switch (type) {
		case "candlestick":
			return chart.addSeries(CandlestickSeries, {
				title: ">",
				priceLineStyle: opts.priceLineStyle,
				upColor: opts.upColor,
				downColor: opts.downColor,
				borderVisible: opts.borderVisible,
				borderUpColor: opts.borderUpColor || opts.upColor,
				borderDownColor: opts.borderDownColor || opts.downColor,
				wickUpColor: opts.wickUpColor || opts.upColor,
				wickDownColor: opts.wickDownColor || opts.downColor,
			});
		case "line":
			return chart.addSeries(LineSeries, {
				title: ">",
				color: opts.lineColor,
				lineStyle: opts.lineStyle as LineStyle,
				lastPriceAnimation: opts.lastPriceAnimationMode,
				lineWidth: opts.lineWidth as LineWidth,
				lineType: opts.lineType as LineType,
			});
		case "area":
			return chart.addSeries(AreaSeries, {
				title: ">",
				lineColor: opts.lineColor,
				lineStyle: opts.lineStyle as LineStyle,
				lastPriceAnimation: opts.lastPriceAnimationMode,
				lineWidth: opts.lineWidth as LineWidth,
				topColor: opts.topColor || opts.lineColor,
				bottomColor: opts.bottomColor || opts.lineColor,
			});
		case "bar":
			return chart.addSeries(BarSeries, {
				upColor: opts.upColor,
				downColor: opts.downColor,
			});
		case "histogram":
			return chart.addSeries(HistogramSeries, { color: opts.lineColor });
		case "baseline": {
			const bopts: any = {
				lineStyle: opts.lineStyle as LineStyle,
				lineWidth: opts.lineWidth as LineWidth,
				lastPriceAnimation: opts.lastPriceAnimationMode,
				topLineColor: opts.upColor,
				bottomLineColor: opts.downColor,
				topFillColor1: `${opts.upColor}33`,
				topFillColor2: `${opts.upColor}11`,
				bottomFillColor1: `${opts.downColor}33`,
				bottomFillColor2: `${opts.downColor}11`,
			};
			if (opts.data.length > 0) {
				const total = opts.data.reduce(
					(acc: number, item: any) => acc + (item.close ?? item.value ?? 0),
					0,
				);
				bopts.baseValue = {
					type: "price" as const,
					price: total / opts.data.length,
				};
			}
			return chart.addSeries(BaselineSeries, bopts);
		}
	}
}

function toVolumeData(
	item: ChartDataItem,
): (HistogramData & { color: string }) | WhitespaceData {
	const close = item.close ?? item.value;
	const open = item.open ?? item.value;
	if (close === undefined || open === undefined) {
		return { time: item.time as Time };
	}
	return {
		time: item.time as Time,
		value: item.volume ?? 0,
		color: close >= open ? "#26a69a" : "#ef5350",
	};
}

function normalizeChartData(data: ChartDataItem[]): ChartDataItem[] {
	const byTime = new Map<number, ChartDataItem>();

	for (const item of data) {
		if (!Number.isFinite(item.time)) continue;
		const value = item.close ?? item.value;
		const isWhitespace =
			item.open === undefined &&
			item.high === undefined &&
			item.low === undefined &&
			item.close === undefined &&
			item.value === undefined;
		if (!isWhitespace && !Number.isFinite(value)) continue;

		byTime.set(item.time, item);
	}

	return Array.from(byTime.values()).sort((a, b) => a.time - b.time);
}

function calcStats(data: ChartDataItem[]) {
	let max = -Infinity,
		min = Infinity,
		total = 0,
		count = 0;
	for (const item of data) {
		if (item.close === undefined && item.value === undefined) continue;
		const h = item.high ?? item.value ?? -Infinity;
		const l = item.low ?? item.value ?? Infinity;
		if (h > max) max = h;
		if (l < min) min = l;
		total += item.close ?? item.value ?? 0;
		count++;
	}
	return {
		maxPrice: max,
		minPrice: min,
		avgPrice: count > 0 ? total / count : Number.NaN,
	};
}

function updatePriceLines(
	series: SeriesApi,
	data: ChartDataItem[],
	opts: {
		showHighPriceLine: boolean;
		highPriceLineColor: string;
		showLowPriceLine: boolean;
		lowPriceLineColor: string;
		showAvgPriceLine: boolean;
		avgPriceLineColor: string;
		highPriceLineStyle: LineStyle;
		lowPriceLineStyle: LineStyle;
		avgPriceLineStyle: LineStyle;
		isMiniChart: boolean;
	},
	refs: {
		highPriceLineRef: React.MutableRefObject<any>;
		lowPriceLineRef: React.MutableRefObject<any>;
		avgPriceLineRef: React.MutableRefObject<any>;
	},
) {
	const { maxPrice, minPrice, avgPrice } = calcStats(data);

	const upsertLine = (
		ref: React.MutableRefObject<any>,
		show: boolean,
		price: number,
		color: string,
		title: string,
		style: LineStyle,
	) => {
		if (show && Number.isFinite(price)) {
			if (ref.current) {
				try {
					ref.current.applyOptions({ price, color, title, lineStyle: style });
				} catch {}
			} else {
				ref.current = series.createPriceLine({
					price,
					color,
					lineWidth: 1,
					lineStyle: style,
					axisLabelVisible: true,
					title,
				});
			}
		} else if (ref.current) {
			try {
				series.removePriceLine(ref.current);
			} catch {}
			ref.current = null;
		}
	};

	upsertLine(
		refs.highPriceLineRef,
		opts.showHighPriceLine,
		maxPrice,
		opts.highPriceLineColor,
		"H",
		opts.highPriceLineStyle,
	);
	upsertLine(
		refs.lowPriceLineRef,
		opts.showLowPriceLine,
		minPrice,
		opts.lowPriceLineColor,
		"L",
		opts.lowPriceLineStyle,
	);
	upsertLine(
		refs.avgPriceLineRef,
		opts.showAvgPriceLine,
		avgPrice,
		opts.avgPriceLineColor,
		opts.isMiniChart ? "" : "A",
		opts.avgPriceLineStyle,
	);
}

function getModeValue(mode: string): PriceScaleMode {
	const modes: Record<string, PriceScaleMode> = {
		normal: PriceScaleMode.Normal,
		logarithmic: PriceScaleMode.Logarithmic,
		percentage: PriceScaleMode.Percentage,
		indexedTo100: PriceScaleMode.IndexedTo100,
	};
	return modes[mode] ?? PriceScaleMode.Normal;
}

function getCrosshairMode(mode: string): CrosshairMode {
	const modes: Record<string, CrosshairMode> = {
		normal: CrosshairMode.Normal,
		magnet: CrosshairMode.Magnet,
		hidden: CrosshairMode.Hidden,
	};
	return modes[mode] ?? CrosshairMode.Normal;
}

function calcEMA(data: ChartDataItem[], period: number): LineData[] {
	const priceData = data.filter(
		(item) => item.close !== undefined || item.value !== undefined,
	);
	if (priceData.length < period) return [];
	const k = 2 / (period + 1);
	const prices = priceData.map((d) => d.close ?? d.value ?? 0);
	let ema = prices.slice(0, period).reduce((a, b) => a + b, 0) / period;
	const result: LineData[] = [
		{ time: priceData[period - 1].time as Time, value: ema },
	];
	for (let i = period; i < prices.length; i++) {
		ema = (prices[i] - ema) * k + ema;
		result.push({ time: priceData[i].time as Time, value: ema });
	}
	return result;
}

export default WChart;
