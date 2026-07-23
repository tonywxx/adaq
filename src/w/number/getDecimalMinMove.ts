/**
 * 根据数字获取最小精度单位（minMove）
 * 例：1.12345 → 0.00001，0.15416 → 0.00001，123 → 1，1.23e-4 → 0.000001
 * @param num 输入数字（NaN/Infinity 返回 NaN）
 */
export function getDecimalMinMove(num: number): number {
	// 处理非数字、无穷大的情况
	if (Number.isNaN(num) || !Number.isFinite(num)) return NaN;

	// 第一步：计算小数位数（复用之前的核心逻辑，优化后内嵌）
	let decimalLength = 0;
	const numStr = num.toString();

	// 处理科学计数法（如 1.23e-4 → 0.000123，1.23e4 → 12300）
	if (numStr.includes("e") || numStr.includes("E")) {
		const [base, exponentStr] = numStr.split(/[eE]/);
		const exponent = parseInt(exponentStr, 10);
		const baseWithoutDot = base.replace(".", "");
		// 小数位数 = 基数长度（去小数点） + 负指数绝对值（正指数则小数位为0）
		decimalLength =
			exponent >= 0 ? 0 : baseWithoutDot.length + Math.abs(exponent);
	} else {
		// 非科学计数法：分割整数和小数部分
		if (numStr.includes(".")) {
			decimalLength = numStr.split(".")[1].length;
		}
	}

	// 第二步：计算 minMove = 10^(-小数位数)
	// 用 Math.pow 确保精度（小数位数≤20时无精度丢失，满足绝大多数场景）
	return 10 ** -decimalLength;
}
