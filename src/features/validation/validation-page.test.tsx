/** @jest-environment jsdom */
import "@/lib/i18n";
import { invoke } from "@tauri-apps/api/core";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { ValidationPage } from "./validation-page";

jest.mock("@tauri-apps/api/core", () => ({ invoke: jest.fn() }));
jest.mock("@tauri-apps/plugin-dialog", () => ({ save: jest.fn() }));
jest.mock("@tauri-apps/plugin-fs", () => ({ open: jest.fn() }));
jest.mock("@/lib/market-session", () => ({
	useMarketSessionStore: (selector: (state: { userId: string }) => unknown) =>
		selector({ userId: "user-1" }),
}));
jest.mock("@/lib/navigation-history", () => ({
	useHistoryTab: (_scope: string, fallback: string) => [fallback, jest.fn()],
}));

(
	globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }
).IS_REACT_ACT_ENVIRONMENT = true;

test("shows persisted validation return and drawdown ratios as percentages", async () => {
	jest.mocked(invoke).mockImplementation(async (command) => {
		if (command === "backtest_list")
			return { items: [], total: 0, page: 1, pageSize: 10 };
		if (command === "validation_report_list")
			return [
				{
					reportId: "report-1",
					protocolId: "protocol-1",
					methodVersion: "chronological-holdout@1",
					aggregationRuleVersion: "equal-window@1",
					windows: [],
					crossMarket: [],
					recommendedContexts: [],
					aggregate: {
						completedWindows: 1,
						failedWindows: 0,
						averageSampleOutReturn: "-0.5692635703982192131635550235",
						averageSampleInReturn: "-0.5432793797188964207830996981",
						worstSampleOutDrawdown: "-0.572123016259637889215505507",
						averageSampleOutSharpe: "0",
						totalFees: "11019.836858226090154404269916",
						totalTrades: 0,
					},
				},
			];
		return [];
	});
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	try {
		await act(async () => {
			root.render(<ValidationPage />);
		});
		expect(container.textContent).toContain("-56.93%");
		expect(container.textContent).toContain("-54.33%");
		expect(container.textContent).toContain("-57.21%");
	} finally {
		await act(async () => root.unmount());
		container.remove();
	}
});
