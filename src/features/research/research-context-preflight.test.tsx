/** @jest-environment jsdom */

import "@/lib/i18n";
import { invoke } from "@tauri-apps/api/core";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { createRoot } from "react-dom/client";
import {
	ResearchContextPreflight,
	type ResearchEvidenceProjection,
} from "./research-context-preflight";

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
				{
					datasetId: "feature-dataset-2",
					requestHash: "request-hash-2",
					manifest: {
						request: {
							featurePlanHash: "plan-hash-2",
							snapshotId: "snapshot-2",
							pointInTimeUniverseId: "universe-2",
							observationRange: { startTimeMs: 3, endTimeMs: 4 },
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

type Deferred<T> = {
	promise: Promise<T>;
	resolve: (value: T) => void;
};

function deferred<T>(): Deferred<T> {
	let resolve!: (value: T) => void;
	const promise = new Promise<T>((resolvePromise) => {
		resolve = resolvePromise;
	});
	return { promise, resolve };
}

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
		evidence: [],
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

test("retries a failed Feature Dataset query", async () => {
	const mockInvoke = invoke as jest.Mock;
	const originalImplementation = mockInvoke.getMockImplementation();
	let datasetListCalls = 0;
	mockInvoke.mockImplementation((command: string, args?: unknown) => {
		if (command === "feature_dataset_list") {
			datasetListCalls += 1;
			if (datasetListCalls === 1)
				return Promise.reject(new Error("temporary-feature-datasets"));
		}
		return originalImplementation?.(command, args);
	});

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

	expect(container.textContent).toContain("temporary-feature-datasets");
	const retry = Array.from(container.querySelectorAll("button")).find(
		(button) => button.textContent === "Retry",
	);
	expect(retry).toBeTruthy();
	await act(async () => {
		retry?.click();
		await Promise.resolve();
		await Promise.resolve();
		await new Promise((resolve) => setTimeout(resolve, 0));
	});

	expect(datasetListCalls).toBe(2);
	expect(container.querySelector("select")).toBeTruthy();

	await act(async () => root.unmount());
	mockInvoke.mockImplementation(originalImplementation);
});

test("does not let an older context handoff replace a newer selection", async () => {
	const first = deferred<ResearchEvidenceProjection>();
	const second = deferred<ResearchEvidenceProjection>();
	const mockInvoke = invoke as jest.Mock;
	const originalImplementation = mockInvoke.getMockImplementation();
	mockInvoke.mockImplementation(
		(command: string, args?: { featureDatasetId?: string }) => {
			if (command === "research_factor_context_establish") {
				return args?.featureDatasetId === "feature-dataset-1"
					? first.promise
					: second.promise;
			}
			return originalImplementation?.(command, args);
		},
	);

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

	const select = container.querySelector("select");
	expect(select).toBeTruthy();
	if (!select) return;
	select.value = "feature-dataset-1";
	await act(async () => {
		select.dispatchEvent(new Event("change", { bubbles: true }));
		await Promise.resolve();
	});
	select.value = "feature-dataset-2";
	await act(async () => {
		select.dispatchEvent(new Event("change", { bubbles: true }));
		await Promise.resolve();
	});

	await act(async () => {
		first.resolve({
			contextRevision: 2,
			contextHash: "first-context-hash",
			market: "crypto",
			venue: "okx",
			rangeStartMs: 1,
			rangeEndMs: 2,
			snapshotId: "snapshot-1",
			universeId: "universe-1",
			evidence: [],
			featureDataset: {
				datasetId: "feature-dataset-1",
				requestHash: "request-hash",
				featurePlanHash: "plan-hash",
				contentSha256: "content-sha",
				outputNames: ["return"],
			},
		});
		await Promise.resolve();
	});
	expect(
		queryClient.getQueryData(["research-evidence-context", "user-1"]),
	).toBeNull();

	await act(async () => {
		second.resolve({
			contextRevision: 3,
			contextHash: "second-context-hash",
			market: "crypto",
			venue: "okx",
			rangeStartMs: 3,
			rangeEndMs: 4,
			snapshotId: "snapshot-2",
			universeId: "universe-2",
			evidence: [],
			featureDataset: {
				datasetId: "feature-dataset-2",
				requestHash: "request-hash-2",
				featurePlanHash: "plan-hash-2",
				contentSha256: "content-sha-2",
				outputNames: ["return"],
			},
		});
		await Promise.resolve();
	});
	expect(
		queryClient.getQueryData(["research-evidence-context", "user-1"]),
	).toMatchObject({
		contextHash: "second-context-hash",
		featureDataset: { datasetId: "feature-dataset-2" },
	});

	await act(async () => root.unmount());
	mockInvoke.mockImplementation(originalImplementation);
});

test("does not let a stale context freeze replace a newer selection", async () => {
	const freeze = deferred<{
		operationId: string;
		contextRevision: number;
		contextHash: string;
		stage: "factors";
		snapshotId: string;
	}>();
	const mockInvoke = invoke as jest.Mock;
	const originalImplementation = mockInvoke.getMockImplementation();
	mockInvoke.mockImplementation(
		(command: string, args?: { featureDatasetId?: string }) => {
			if (command === "research_context_freeze") return freeze.promise;
			return originalImplementation?.(command, args);
		},
	);

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

	const select = container.querySelector("select");
	if (!select) return;
	select.value = "feature-dataset-1";
	await act(async () => {
		select.dispatchEvent(new Event("change", { bubbles: true }));
		await Promise.resolve();
		await Promise.resolve();
	});

	const button = container.querySelector("button");
	if (!button) return;
	await act(async () => {
		button.click();
		await Promise.resolve();
	});

	select.value = "feature-dataset-2";
	await act(async () => {
		select.dispatchEvent(new Event("change", { bubbles: true }));
		await Promise.resolve();
	});
	await act(async () => {
		freeze.resolve({
			contextRevision: 2,
			contextHash: "stale-freeze",
			stage: "factors",
			snapshotId: "snapshot-1",
			operationId: "factor-freeze",
		});
		await Promise.resolve();
	});

	expect(container.textContent).not.toContain("Frozen revision 2");

	await act(async () => root.unmount());
	mockInvoke.mockImplementation(originalImplementation);
});
