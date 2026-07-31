import { historyTabValue } from "./navigation-history";

test("restores a tab only for its original result or report", () => {
	const state = {
		__adaqTab: { owner: "run-1", scope: "backtest-results", value: "decisions" },
	};

	expect(historyTabValue(state, "backtest-results", "overview", "run-1")).toBe(
		"decisions",
	);
	expect(historyTabValue(state, "backtest-results", "overview", "run-2")).toBe(
		"overview",
	);
});
