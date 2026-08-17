/** @jest-environment jsdom */

import "@/lib/i18n";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { clearSessionCache } from "@/lib/session-cache";
import { i18n } from "@/lib/i18n";
import { useMarketSessionStore } from "@/lib/market-session";
import { writeFactorCache } from "./factor-data";
import { FactorsPage } from "./factors-page";
import type { FactorAdapter } from "./factor-adapter";
import type { FactorFamilyView, FactorPage } from "./factor-types";

jest.mock("@/lib/market-session", () => {
	const state = { userId: null as string | null, ready: false };
	const useMarketSessionStore = Object.assign(
		(selector: (current: typeof state) => unknown) => selector(state),
		{
			setState: (next: Partial<typeof state>) => Object.assign(state, next),
			getState: () => ({
				...state,
				clear: () => Object.assign(state, { userId: null, ready: false }),
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

(
	globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }
).IS_REACT_ACT_ENVIRONMENT = true;

type Deferred<T> = {
	promise: Promise<T>;
	resolve: (value: T) => void;
	reject: (reason?: unknown) => void;
};

function deferred<T>(): Deferred<T> {
	let resolve!: (value: T) => void;
	let reject!: (reason?: unknown) => void;
	const promise = new Promise<T>((resolvePromise, rejectPromise) => {
		resolve = resolvePromise;
		reject = rejectPromise;
	});
	return { promise, resolve, reject };
}

function page<T>(items: T[]): FactorPage<T> {
	return { items, page: 1, pageSize: 50, total: items.length };
}

function family(name: string): FactorFamilyView {
	return {
		family: {
			familyId: name,
			rootCandidateHash: `${name}-candidate`,
			registeredTrialIds: [],
		},
		trialCount: 0,
		lineageHash: `${name}-lineage`,
	};
}

function makeAdapter(
	overrides: Partial<Pick<FactorAdapter, "listFamilies" | "listAttempts">> = {},
): FactorAdapter {
	return {
		listFamilies: async () => page([] as FactorFamilyView[]),
		listAttempts: async () => page([]),
		...overrides,
	} as unknown as FactorAdapter;
}

function mount(adapter: FactorAdapter) {
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	return { container, root, adapter };
}

async function unmount(root: Root, container: HTMLDivElement) {
	await act(async () => root.unmount());
	container.remove();
}

async function settle() {
	await act(async () => {
		await Promise.resolve();
		await Promise.resolve();
		await Promise.resolve();
	});
}

beforeEach(() => {
	clearSessionCache();
	useMarketSessionStore.setState({ userId: "user-1", ready: true });
	Object.defineProperty(globalThis, "requestAnimationFrame", {
		configurable: true,
		value: (callback: FrameRequestCallback) => {
			callback(0);
			return 0;
		},
	});
	Object.defineProperty(globalThis, "cancelAnimationFrame", {
		configurable: true,
		value: () => undefined,
	});
});

afterEach(() => {
	useMarketSessionStore.getState().clear();
	document.body.replaceChildren();
	clearSessionCache();
});

test("renders the Factor Lab shell and surfaces family loading errors", async () => {
	const pending: Deferred<FactorPage<FactorFamilyView>> = deferred();
	const listFamilies = jest.fn(() => pending.promise);
	const mounted = mount(makeAdapter({ listFamilies }));

	await act(async () => {
		mounted.root.render(<FactorsPage adapter={mounted.adapter} />);
	});
	await settle();

	expect(
		mounted.container.querySelector('[data-route="factors"]'),
	).not.toBeNull();
	for (const tab of [
		"factors.tabs.families",
		"factors.tabs.candidates",
		"factors.tabs.datasets",
		"factors.tabs.evaluations",
		"factors.tabs.decisions",
	]) {
		expect(mounted.container.textContent).toContain(i18n.t(tab));
	}
	expect(mounted.container.querySelector('[aria-busy="true"]')).not.toBeNull();

	await act(async () => {
		pending.reject(new Error("family list failed"));
	});
	await settle();
	expect(
		mounted.container.querySelector('[role="alert"]')?.textContent,
	).toContain("family list failed");

	await unmount(mounted.root, mounted.container);
});

test("shows cached families before replacing them with a refresh", async () => {
	const pending: Deferred<FactorPage<FactorFamilyView>> = deferred();
	writeFactorCache("user-1", "families", page([family("cached-family")]));
	const mounted = mount(
		makeAdapter({ listFamilies: jest.fn(() => pending.promise) }),
	);

	await act(async () => {
		mounted.root.render(<FactorsPage adapter={mounted.adapter} />);
	});
	await settle();

	expect(mounted.container.textContent).toContain("cached-family");

	await act(async () => {
		pending.resolve(page([family("fresh-family")]));
	});
	await settle();

	expect(mounted.container.textContent).toContain("fresh-family");
	expect(mounted.container.textContent).not.toContain("cached-family");

	await unmount(mounted.root, mounted.container);
});
