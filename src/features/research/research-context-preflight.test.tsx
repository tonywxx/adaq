/** @jest-environment jsdom */

import "@/lib/i18n";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { ResearchContextPreflight } from "./research-context-preflight";

jest.mock("@tauri-apps/api/core", () => ({
	invoke: jest.fn(async (command: string) => {
		if (command === "research_context_get") {
			return {
				contextRevision: 1,
				contextHash: "context-hash",
				market: "crypto",
				venue: "okx",
				snapshotId: "snapshot-1",
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
