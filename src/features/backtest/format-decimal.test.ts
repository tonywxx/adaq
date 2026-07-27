import { formatDecimal } from "./format-decimal";

test("removes only insignificant decimal zeros", () => {
	expect(formatDecimal("6.3100113600000000000000000000")).toBe("6.31001136");
	expect(formatDecimal("1583.20")).toBe("1583.2");
	expect(formatDecimal("100")).toBe("100");
	expect(formatDecimal("1.0200")).toBe("1.02");
	expect(formatDecimal("-0.000")).toBe("0");
});
