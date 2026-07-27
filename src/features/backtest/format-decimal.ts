export function formatDecimal(value: string) {
	const [integer, fraction] = value.split(".");
	if (fraction === undefined) return value;
	const trimmed = fraction.replace(/0+$/, "");
	if (trimmed) return `${integer}.${trimmed}`;
	return integer === "-0" ? "0" : integer;
}
