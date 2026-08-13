import {
	finiteGridTrialCount,
	factorPageCount,
	isGridWithinLimit,
	parseFactorJson,
	shortFactorHash,
} from "./factor-data";

test("bounds explicit Grid Search before it reaches the native API", () => {
	expect(finiteGridTrialCount([2, 3, 4])).toBe(24);
	expect(isGridWithinLimit([16, 16])).toBe(true);
	expect(finiteGridTrialCount([17, 16])).toBe(272);
	expect(isGridWithinLimit([17, 16])).toBe(false);
	expect(finiteGridTrialCount([])).toBeNull();
	expect(finiteGridTrialCount([0, 2])).toBeNull();
});

test("keeps pagination and exact evidence identifiers deterministic", () => {
	expect(factorPageCount(0)).toBe(1);
	expect(factorPageCount(101)).toBe(3);
	expect(shortFactorHash("a".repeat(64))).toBe(`${"a".repeat(16)}…`);
	expect(parseFactorJson('{"schemaVersion":"1.0.0"}', "draft")).toEqual({
		schemaVersion: "1.0.0",
	});
	expect(() => parseFactorJson("[]", "draft")).toThrow("draft");
});
