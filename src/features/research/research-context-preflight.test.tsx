/** @jest-environment jsdom */

import "@/lib/i18n";
import { invoke } from "@tauri-apps/api/core";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { ResearchContextPreflight } from "./research-context-preflight";

jest.mock("@tauri-apps/api/core", () => ({
	invoke: jest.fn(async (command: string, args?: { userId?: string }) => {
		if (command === "research_context_get") {
			if (args?.userId === "no-context-user") return null;
			return {
				contextRevision: 1,
				contextHash: "context-hash",
				market: "crypto",
				venue: "okx",
				rangeStartMs: 1,
				rangeEndMs: 2,
				snapshotId: "snapshot-1",
			};
		}
		if (command === "feature_dataset_list") {
			return [
				{
					datasetId: "feature-dataset-1",
					requestHash: "request-hash",
					manifest: {
						request: {
							featurePlanHash: "plan-hash",
							snapshotId: "snapshot-1",
							pointInTimeUniverseId: "universe-1",
							observationRange: { startTimeMs: 1, endTimeMs: 2 },
						},
						rowCount: 10,
					},
				},
			];
		}
		if (command === "research_factor_context_establish") {
			return {
				contextRevision: 2,
				contextHash: "factor-context-hash",
				market: "crypto",
				venue: "okx",
				rangeStartMs: 1,
				rangeEndMs: 2,
				snapshotId: "snapshot-1",
				universeId: "universe-1",
				featureDataset: {
					datasetId: "feature-dataset-1",
					requestHash: "request-hash",
					featurePlanHash: "plan-hash",
					contentSha256: "content-sha",
					outputNames: ["return"],
				},
			};
		}
		throw new Error("stale-context");
	}),
}));
jest.mock("@tanstack/react-router", () => ({
	Link: ({ children }: { children?: unknown }) => children,
}));

(
	globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }
).IS_REACT_ACT_ENVIRONMENT = true;

test("shows a fail-closed context freeze error", async () => {
	const container = document.createElement("div");
	const root = createRoot(container);
	const queryClient = new QueryClient({
		defaultOptions: { queries: { retry: false } },
	});
	queryClient.setQueryData(["research-evidence-context", "user-1"], {
		contextRevision: 1,
		contextHash: "context-hash",
		market: "crypto",
		venue: "okx",
		snapshotId: "snapshot-1",
	});

	await act(async () => {
		root.render(
			<QueryClientProvider client={queryClient}>
				<ResearchContextPreflight userId="user-1" stage="models" />
			</QueryClientProvider>,
		);
		await Promise.resolve();
	});

	const button = container.querySelector("button");
	expect(button).toBeTruthy();
	await act(async () => {
		button?.click();
		await Promise.resolve();
	});

	expect(container.textContent).toContain("Error: stale-context");
	await act(async () => root.unmount());
});

test("keeps Factors blocked when no context is established", async () => {
	const container = document.createElement("div");
	const root = createRoot(container);
	const queryClient = new QueryClient({
		defaultOptions: { queries: { retry: false } },
	});

	await act(async () => {
		root.render(
			<QueryClientProvider client={queryClient}>
				<ResearchContextPreflight userId="no-context-user" stage="factors" />
			</QueryClientProvider>,
		);
	});
	await act(async () => {
		await Promise.resolve();
		await Promise.resolve();
		await new Promise((resolve) => setTimeout(resolve, 0));
	});

	expect(container.textContent).toContain("Blocked");
	expect(container.textContent).toContain("Open Features");
	await act(async () => root.unmount());
});

test("lets Factors select a Feature Dataset and shows the Host-resolved binding", async () => {
	const container = document.createElement("div");
	const root = createRoot(container);
	const queryClient = new QueryClient({
		defaultOptions: { queries: { retry: false } },
	});

	await act(async () => {
		root.render(
			<QueryClientProvider client={queryClient}>
				<ResearchContextPreflight userId="user-1" stage="factors" />
			</QueryClientProvider>,
		);
	});
	await act(async () => {
		await Promise.resolve();
		await Promise.resolve();
		await new Promise((resolve) => setTimeout(resolve, 0));
	});
	expect(container.textContent).toContain("Blocked");
	expect(container.textContent).toContain("Open Features");

	const select = container.querySelector("select");
	expect(select).toBeTruthy();
	if (!select) return;
	select.value = "feature-dataset-1";
	await act(async () => {
		select.dispatchEvent(new Event("change", { bubbles: true }));
		await Promise.resolve();
	});
	await act(async () => {
		await Promise.resolve();
		await Promise.resolve();
		await new Promise((resolve) => setTimeout(resolve, 0));
	});

	expect(invoke).toHaveBeenCalledWith("research_factor_context_establish", {
		featureDatasetId: "feature-dataset-1",
	});
	expect(container.textContent).toContain("feature-dataset-1");
	expect(container.textContent).toContain("plan-hash");
	expect(container.textContent).toContain("snapshot-1");
	expect(container.textContent).toContain("1 → 2");
	expect(
		queryClient.getQueryData(["research-evidence-context", "user-1"]),
	).toMatchObject({
		featureDataset: { datasetId: "feature-dataset-1" },
	});

	await act(async () => root.unmount());
});
