import { useEffect, useMemo, useState } from "react";

// 动态价格组件 - 类似 TradingView 的价格变化效果
interface AnimatedPriceProps {
	currentPrice: number;
	prevPrice: number;
	decimalLength: number;
}

export const AnimatedPrice: React.FC<AnimatedPriceProps> = ({
	currentPrice,
	prevPrice,
	decimalLength,
}) => {
	// 格式化价格字符串 (加入千位分隔符)
	const formatPrice = (p: number) =>
		p.toLocaleString("en-US", {
			minimumFractionDigits: decimalLength,
			maximumFractionDigits: decimalLength,
		});

	const currentStr = formatPrice(currentPrice);
	const prevStr = prevPrice > 0 ? formatPrice(prevPrice) : currentStr;

	// 计算涨跌状态和颜色
	const isUp = currentPrice > prevPrice && prevPrice > 0;
	const isDown = currentPrice < prevPrice && prevPrice > 0;
	const colorClass = isUp ? "text-emerald-400" : isDown ? "text-rose-400" : "";

	const [highlightAll, setHighlightAll] = useState(false);

	useEffect(() => {
		if (isUp || isDown) {
			setHighlightAll(true);
			const timer = setTimeout(() => {
				setHighlightAll(false);
			}, 300);
			return () => clearTimeout(timer);
		}
	}, [currentPrice, prevPrice, isUp, isDown]);

	// 使用 useMemo 优化字符串比较
	const { unchangedPart, changedPart } = useMemo(() => {
		// 找到第一个不同的字符位置
		let diffIndex = -1;
		const minLen = Math.min(currentStr.length, prevStr.length);

		for (let i = 0; i < minLen; i++) {
			if (currentStr[i] !== prevStr[i]) {
				diffIndex = i;
				break;
			}
		}

		// 如果前面都相同，但长度不同，则从长度差异处开始
		if (diffIndex === -1 && currentStr.length !== prevStr.length) {
			diffIndex = minLen;
		}

		const unchanged =
			diffIndex === -1 ? currentStr : currentStr.slice(0, diffIndex);
		const changed = diffIndex === -1 ? "" : currentStr.slice(diffIndex);

		return {
			unchangedPart: unchanged,
			changedPart: changed,
			firstDiffIndex: diffIndex,
		};
	}, [currentStr, prevStr]);

	// 如果价格为 0，不显示
	if (currentPrice === 0) {
		return null;
	}

	// 如果没有变化部分，整个价格正常显示
	if (changedPart === "") {
		return <span className="text-4xl">{currentStr}</span>; // 修复：添加分号
	}

	return (
		<span className="inline-flex text-4xl">
			<span
				className={`${highlightAll ? colorClass : ""} transition-colors duration-200`}
			>
				{unchangedPart}
			</span>
			<span
				// 修复：修正类名拼接错误 ($colorClass 改为 ${colorClass}，并添加空格)
				className={`${colorClass} font-bold transition-colors duration-200`}
			>
				{changedPart}
			</span>
		</span>
	);
};
