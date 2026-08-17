/** @jest-environment jsdom */

import "@/lib/i18n";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { useMarketSessionStore } from "@/lib/market-session";
import type { BacktestAdapter } from "./backtest-adapter";
import { BacktestPage } from "./backtest-page";

jest.mock("@/lib/market-session", () => {
	const state = {
		userId: null as string | null,
		activeInstrument: { src: "okx", code: "BTC-USDT" },
		watchlist: [] as Array<{ src: string; code: string }>,
	};
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
	return {
		useMarketSessionStore,
		instrumentKey: (item: { src: string; code: string }) =>
			`${item.src}:${item.code}`,
	};
});

jest.mock("@/lib/navigation-history", () => ({
	useHistoryTab: (_scope: string, fallback: string) => [fallback, jest.fn()],
}));

jest.mock("./backtest-chart", () => ({
	BacktestChart: () => null,
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
	listComponents: BacktestAdapter["listComponents"],
): BacktestAdapter {
	return {
		listRuns: async () => ({ items: [], total: 0, page: 1, pageSize: 10 }),
		listSnapshots: async () => ({ items: [], total: 0, page: 1, pageSize: 10 }),
		listComponents,
		listCompatibleFactors: async () => ({}),
		listCompatibleSignals: async () => [],
		preflight: async () => undefined,
		downloadSnapshot: async () => undefined,
		cancelSnapshot: async () => undefined,
		run: async () => undefined,
		executionData: async () => ({
			orders: [],
			fills: [],
			totalOrders: 0,
			totalFills: 0,
		}),
		chartData: async () => undefined,
		getRun: async () => undefined,
	} as unknown as BacktestAdapter;
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
	useMarketSessionStore.setState({ userId: "user-1" });
});

afterEach(() => {
	useMarketSessionStore.getState().clear();
	document.body.replaceChildren();
});

test("Backtest page keeps draft UI while an injected component load fails", async () => {
	const pending = deferred<never>();
	const listComponents = jest.fn(() => pending.promise);
	const adapter = makeAdapter(listComponents);
	const container = document.createElement("div");
	const root = createRoot(container);
	document.body.append(container);

	await act(async () => root.render(<BacktestPage adapter={adapter} />));
	await settle();

	expect(listComponents).toHaveBeenCalledWith("user-1");
	expect(container.textContent).toContain("Data and Strategy configuration");

	await act(async () => pending.reject(new Error("component load failed")));
	await settle();
	expect(container.textContent).toContain("component load failed");

	await unmount(root, container);
});
