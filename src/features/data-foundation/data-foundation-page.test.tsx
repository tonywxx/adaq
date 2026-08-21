/** @jest-environment jsdom */

import "@/lib/i18n";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { i18n } from "@/lib/i18n";
import { DataFoundationPage } from "./data-foundation-page";

jest.mock("@tauri-apps/api/core", () => ({
	invoke: jest.fn(async (command: string) => {
		if (command === "market_data_pipeline_list") {
			return [
				{
					sourceId: "source-1",
					canonicalId: "canonical-1",
					revision: 1,
					state: "passed",
					sourceRecordCount: 10,
					canonicalRecordCount: 10,
					quarantinedRecordCount: 0,
					gapCount: 0,
				},
			];
		}
		if (command === "snapshot_list_readable") {
			return [
				{
					snapshotId: "snapshot-1",
					code: "BTC-USDT",
					interval: "1m",
					barCount: 10,
				},
			];
		}
		if (command === "snapshot_list_universe") return { items: [] };
		if (command === "research_context_get") return null;
		if (command === "foundation_acquisition_history") {
			return [
				{
					operationId: "crypto-foundation-1",
					market: "crypto",
					venue: "okx",
					state: "completed",
					revision: 1,
					startedAtMs: 1,
					finishedAtMs: 2,
				},
			];
		}
		if (
			command === "okx_acquisition_status" ||
			command === "ashare_instrument_master_list" ||
			command === "alpaca_instrument_master_list"
		) {
			return [];
		}
		if (command === "market_data_pipeline_quality") {
			return {
				reportId: "report-1",
				state: "passed",
				duplicateCount: 0,
				conflictCount: 0,
				quarantineCount: 0,
				gapCount: 0,
				warningCount: 0,
				reasons: [],
			};
		}
		return null;
	}),
}));
jest.mock("@tanstack/react-router", () => ({
	Link: ({
		children,
		...props
	}: {
		children?: unknown;
		[key: string]: unknown;
	}) => require("react").createElement("a", props, children),
}));
jest.mock("@/lib/market-session", () => ({
	useMarketSessionStore: (selector: (state: { userId: string }) => unknown) =>
		selector({ userId: "user-1" }),
	getErrorMessage: (error: unknown) => String(error),
}));

const mockInvoke = jest.requireMock("@tauri-apps/api/core").invoke as jest.Mock;
(
	globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }
).IS_REACT_ACT_ENVIRONMENT = true;

afterEach(() => {
	document.body.replaceChildren();
	mockInvoke.mockClear();
});

test("renders localized evidence and persisted operation history", async () => {
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	const queryClient = new QueryClient({
		defaultOptions: { queries: { retry: false } },
	});

	await act(async () => {
		root.render(
			<QueryClientProvider client={queryClient}>
				<DataFoundationPage />
			</QueryClientProvider>,
		);
		await Promise.resolve();
		await Promise.resolve();
		await new Promise((resolve) => setTimeout(resolve, 50));
	});

	expect(container.textContent).toContain(i18n.t("dataFoundation.title"));
	expect(container.textContent).toContain(
		i18n.t("dataFoundation.operationLedger"),
	);
	expect(mockInvoke).toHaveBeenCalledWith("foundation_acquisition_history", {
		userId: "user-1",
	});

	await act(async () => root.unmount());
});
