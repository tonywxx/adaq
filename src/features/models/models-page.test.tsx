/** @jest-environment jsdom */

import "@/lib/i18n";
import { act } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createRoot, type Root } from "react-dom/client";
import type { LibraryComponent } from "@/features/components/component-library";
import { useMarketSessionStore } from "@/lib/market-session";
import type { ModelsAdapter } from "./models-adapter";
import { ModelsPage } from "./models-page";

jest.mock("@tauri-apps/api/core", () => ({
	invoke: jest.fn().mockResolvedValue(null),
}));
jest.mock("@tauri-apps/plugin-fs", () => ({
	open: jest.fn(),
	readFile: jest.fn(),
}));
jest.mock("@tauri-apps/plugin-dialog", () => ({
	open: jest.fn(),
	save: jest.fn(),
}));

jest.mock("@/lib/market-session", () => {
	const state = { userId: null as string | null, ready: false };
	const useMarketSessionStore = Object.assign(
		(selector: (current: typeof state) => unknown) => selector(state),
		{
			setState: (next: Partial<typeof state>) => Object.assign(state, next),
			getState: () => ({
				...state,
				clear: () => Object.assign(state, { userId: null }),
			}),
		},
	);
	return { useMarketSessionStore };
});

jest.mock("@tanstack/react-router", () => ({
	Link: ({
		children,
		...props
	}: {
		children?: unknown;
		[key: string]: unknown;
	}) => require("react").createElement("a", props, children),
}));

jest.mock("@/lib/navigation-history", () => ({
	useHistoryTab: (_scope: string, fallback: string) => [fallback, jest.fn()],
}));

jest.mock("@/features/python-research/python-projects-panel", () => ({
	PythonProjectsPanel: () => null,
}));
jest.mock("@/features/python-research/python-model-lab-panel", () => ({
	PythonModelLabPanel: () => null,
}));
jest.mock("@/features/python-research/python-tutorial-panel", () => ({
	PythonTutorialPanel: () => null,
}));

(
	globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }
).IS_REACT_ACT_ENVIRONMENT = true;

type Deferred<T> = {
	promise: Promise<T>;
	reject: (reason?: unknown) => void;
};

function deferred<T>(): Deferred<T> {
	let reject!: (reason?: unknown) => void;
	const promise = new Promise<T>((_, rejectPromise) => {
		reject = rejectPromise;
	});
	return { promise, reject };
}

function makeAdapter(
	listComponents: ModelsAdapter["listComponents"],
): ModelsAdapter {
	return {
		listComponents,
		listSnapshots: async () => [],
		listAttempts: async () => [],
		listDatasets: async () => [],
		listEvaluations: async () => [],
		listCompatibleFactors: async () => ({}),
		startDatasetGeneration: async () => undefined,
		cancelDatasetGeneration: async () => undefined,
		importSignalDataset: async () => undefined,
		exportSignalDataset: async () => [],
		signalDatasetRows: async () => ({
			items: [],
			total: 0,
			page: 1,
			pageSize: 50,
		}),
		retryDatasetGeneration: async () => undefined,
		createEvaluation: async () => undefined,
		exportEvaluation: async () => "",
	} as unknown as ModelsAdapter;
}

async function settle() {
	await act(async () => {
		await Promise.resolve();
		await Promise.resolve();
		await Promise.resolve();
	});
}

async function unmount(root: Root, container: HTMLDivElement) {
	await act(async () => root.unmount());
	container.remove();
}

beforeEach(() => {
	useMarketSessionStore.setState({ userId: "user-1", ready: true });
	Object.defineProperty(globalThis, "requestAnimationFrame", {
		configurable: true,
		value: (callback: FrameRequestCallback) => {
			callback(0);
			return 0;
		},
	});
});

afterEach(() => {
	useMarketSessionStore.getState().clear();
	document.body.replaceChildren();
});

test("Models page keeps its shell while an injected transport reports an error", async () => {
	const pending = deferred<LibraryComponent[]>();
	const listComponents = jest.fn(() => pending.promise);
	const adapter = makeAdapter(listComponents);
	const container = document.createElement("div");
	const root = createRoot(container);
	document.body.append(container);
	const queryClient = new QueryClient({
		defaultOptions: { queries: { retry: false } },
	});

	await act(async () =>
		root.render(
			<QueryClientProvider client={queryClient}>
				<ModelsPage adapter={adapter} />
			</QueryClientProvider>,
		),
	);
	await settle();

	expect(listComponents).toHaveBeenCalledWith("user-1");
	expect(container.textContent).toContain("Models");

	await act(async () => pending.reject(new Error("component list failed")));
	await settle();
	expect(container.textContent).toContain("component list failed");

	await unmount(root, container);
});
