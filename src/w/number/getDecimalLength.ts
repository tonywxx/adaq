/**
 * 计算数字的小数位数（兼容浮点数精度问题和科学计数法）
 * @param num 要计算的数字（整数返回 0，NaN/Infinity 返回 -1）
 */
export function getDecimalLength(num: number): number {
	// 处理非数字、无穷大的情况
	if (Number.isNaN(num) || !Number.isFinite(num)) return -1;

	// 转为字符串，处理科学计数法（如 1.23e-4 → 0.000123，1.23e4 → 12300）
	const numStr = num.toString();
	if (numStr.includes("e") || numStr.includes("E")) {
		// 拆分科学计数法（基数 + 指数）
		const [base, exponentStr] = numStr.split(/[eE]/);
		const exponent = parseInt(exponentStr, 10);

		// 处理指数：正数→整数（无小数），负数→补0后计算小数位
		if (exponent >= 0) {
			return 0; // 如 1.23e4 = 12300，小数位 0
		} else {
			// 基数去掉小数点（如 1.23 → 123），小数位 = 基数长度 + 负指数的绝对值
			const baseWithoutDot = base.replace(".", "");
			return baseWithoutDot.length + Math.abs(exponent);
		}
	}

	// 非科学计数法：直接分割整数和小数部分
	if (!numStr.includes(".")) {
		return 0; // 整数，小数位 0
	}

	// 分割后取小数部分，统计长度（处理末尾多余的0？看需求，这里保留原始长度）
	const [, decimalPart] = numStr.split(".");
	return decimalPart.length;
}
