// 格式化价格，最大保留小数点后16位
export const formatCoinPrice = (price: number) => {
	return price.toLocaleString("en-US", {
		minimumFractionDigits: 1,
		maximumFractionDigits: 16,
	});
};
