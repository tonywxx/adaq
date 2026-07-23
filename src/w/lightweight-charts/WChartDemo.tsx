import { LineStyle } from "lightweight-charts";
import { useState } from "react";
import WChart from "@/w/coms/lightweight-charts/WChart";

// ========== 示例使用 Demo ==========
const WChartDemo = () => {
	const [chartType, setChartType] = useState<"candlestick" | "line">(
		"candlestick",
	);
	const [theme, setTheme] = useState<"light" | "dark">("light");

	// 生成示例数据
	const generateSampleData = () => {
		const data = [];
		const basePrice = 100;
		let currentPrice = basePrice;
		const now = new Date();
		now.setHours(0, 0, 0, 0);

		for (let i = 0; i < 100; i++) {
			const date = new Date(now);
			date.setDate(date.getDate() - (100 - i));

			const change = (Math.random() - 0.5) * 5;
			const open = currentPrice;
			const close = currentPrice + change;
			const high = Math.max(open, close) + Math.random() * 2;
			const low = Math.min(open, close) - Math.random() * 2;

			currentPrice = close;

			data.push({
				time: date.toISOString().split("T")[0],
				open,
				high,
				low,
				close,
				value: close, // 用于折线图
			} as never);
		}

		return data;
	};

	const [data] = useState(generateSampleData());

	const themeConfig = {
		light: {
			backgroundColor: "#ffffff",
			textColor: "#191919",
			gridColor: "rgba(197, 203, 206, 0.5)",
		},
		dark: {
			backgroundColor: "#1e222d",
			textColor: "#d1d4dc",
			gridColor: "rgba(42, 46, 57, 0.5)",
		},
	};

	const currentTheme = themeConfig[theme];

	return (
		<div
			style={{
				padding: "20px",
				backgroundColor: currentTheme.backgroundColor,
				minHeight: "100vh",
			}}
		>
			<div
				style={{
					marginBottom: "20px",
					display: "flex",
					gap: "10px",
					flexWrap: "wrap",
				}}
			>
				<button
					onClick={() => setChartType("candlestick")}
					style={{
						padding: "8px 16px",
						backgroundColor: chartType === "candlestick" ? "#2962FF" : "#ddd",
						color: chartType === "candlestick" ? "white" : "black",
						border: "none",
						borderRadius: "4px",
						cursor: "pointer",
						fontSize: "14px",
					}}
				>
					K线图
				</button>
				<button
					onClick={() => setChartType("line")}
					style={{
						padding: "8px 16px",
						backgroundColor: chartType === "line" ? "#2962FF" : "#ddd",
						color: chartType === "line" ? "white" : "black",
						border: "none",
						borderRadius: "4px",
						cursor: "pointer",
						fontSize: "14px",
					}}
				>
					折线图
				</button>
				<button
					onClick={() => setTheme(theme === "light" ? "dark" : "light")}
					style={{
						padding: "8px 16px",
						backgroundColor: "#4caf50",
						color: "white",
						border: "none",
						borderRadius: "4px",
						cursor: "pointer",
						fontSize: "14px",
					}}
				>
					切换主题 ({theme === "light" ? "浅色" : "深色"})
				</button>
			</div>

			<h2 style={{ color: currentTheme.textColor, marginBottom: "16px" }}>
				WChart - TradingView 风格股票图表
			</h2>

			<WChart
				data={data}
				chartType={chartType}
				height={500}
				autoSize={true}
				backgroundColor={currentTheme.backgroundColor}
				textColor={currentTheme.textColor}
				vertGridColor={currentTheme.gridColor}
				horzGridColor={currentTheme.gridColor}
				upColor="#26a69a"
				downColor="#ef5350"
				lineColor="#2962FF"
				lineWidth={2}
				showVolume={true}
				volumeUpColor="#26a69a"
				volumeDownColor="#ef5350"
				showHighPriceLine={true}
				highPriceLineColor="#2962FF"
				showLowPriceLine={true}
				lowPriceLineColor="#ccc"
				showAvgPriceLine={true}
				avgPriceLineColor="#EAB308"
				priceLineStyle={LineStyle.Solid}
				crosshairMode="magnet"
				watermarkVisible={true}
				watermarkText="DEMO"
				watermarkColor={
					theme === "dark" ? "rgba(255, 255, 255, 0.05)" : "rgba(0, 0, 0, 0.05)"
				}
				onCrosshairMove={(param) => {
					if (param.time) {
						console.log("Crosshair moved:", param);
					}
				}}
			/>

			<div
				style={{
					marginTop: "20px",
					padding: "16px",
					backgroundColor: theme === "dark" ? "#2a2e39" : "#f5f5f5",
					borderRadius: "8px",
					color: currentTheme.textColor,
				}}
			>
				<h3>使用说明</h3>
				<ul style={{ lineHeight: "1.8" }}>
					<li>支持 K 线图和折线图切换</li>
					<li>鼠标滚轮缩放时间轴</li>
					<li>拖拽移动图表</li>
					<li>双击重置缩放</li>
					<li>悬停显示详细数据</li>
					<li>支持深色/浅色主题切换</li>
				</ul>
			</div>
		</div>
	);
};

export default WChartDemo;
