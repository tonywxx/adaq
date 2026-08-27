/** @jest-environment jsdom */

import "@/lib/i18n";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { clearSessionCache } from "@/lib/session-cache";
import { i18n } from "@/lib/i18n";
import { useMarketSessionStore } from "@/lib/market-session";
import { writeFactorCache } from "./factor-data";
import {
	applyFactorContext,
	factorCandidatesForContext,
	FactorsPage,
} from "./factors-page";
import type { FactorAdapter } from "./factor-adapter";
import type {
	FactorCandidateView,
	FactorDatasetView,
	FactorFamilyView,
	FactorPage,
} from "./factor-types";

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

jest.mock("@tauri-apps/api/core", () => ({
	invoke: jest.fn().mockResolvedValue(null),
}));
jest.mock("@tauri-apps/plugin-fs", () => ({
	readFile: jest.fn(),
	writeFile: jest.fn(),
}));
jest.mock("@tauri-apps/plugin-dialog", () => ({
	open: jest.fn(),
	save: jest.fn(),
}));
jest.mock("@tauri-apps/plugin-opener", () => ({ openPath: jest.fn() }));
jest.mock("@/features/python-research/python-projects-panel", () => ({
	PythonProjectsPanel: () => null,
}));
jest.mock("@/features/python-research/python-factor-lab-panel", () => ({
	PythonFactorLabPanel: () => null,
}));
jest.mock("@/features/python-research/python-tutorial-panel", () => ({
	PythonTutorialPanel: () => null,
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
	const queryClient = new QueryClient({
		defaultOptions: { queries: { retry: false } },
	});
	return { container, root, adapter, queryClient };
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
		mounted.root.render(
			<QueryClientProvider client={mounted.queryClient}>
				<FactorsPage adapter={mounted.adapter} />
			</QueryClientProvider>,
		);
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

test("applies the selected context to Factor protocol identity and range", () => {
	const draft = applyFactorContext(
		{ marketContext: { barInterval: "1h", priceBasis: "unadjusted" } },
		{
			contextRevision: 2,
			contextHash: "context-hash",
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
		},
	);

	expect(draft).toMatchObject({
		observationRange: { startTimeMs: 1, endTimeMs: 2 },
		marketContext: {
			assetClass: "crypto",
			venue: "okx",
			pointInTimeUniverseId: "universe-1",
			barInterval: "1h",
			priceBasis: "unadjusted",
		},
	});
});

test("only offers Candidates with the exact handed-off Factor context", () => {
	const context = {
		contextRevision: 2,
		contextHash: "c".repeat(64),
		market: "crypto",
		venue: "okx",
		rangeStartMs: 1,
		rangeEndMs: 2,
		snapshotId: "snapshot-1",
		universeId: "universe-1",
		evidence: [],
		featureDataset: {
			datasetId: "feature-dataset-1",
			requestHash: "r".repeat(64),
			featurePlanHash: "p".repeat(64),
			contentSha256: "d".repeat(64),
			outputNames: ["return"],
		},
	};
	const candidate = {
		candidate: { candidateHash: "candidate-1", scope: "time-series" },
		presentation: { name: "Exact candidate" },
		lockedBy: [],
		createdAtMs: 1,
		predecessor: {
			...context,
			userId: "user-1",
			evidence: [],
			featureDataset: context.featureDataset,
		},
	} satisfies FactorCandidateView;
	const stale = {
		...candidate,
		candidate: { candidateHash: "candidate-2", scope: "time-series" },
		predecessor: { ...candidate.predecessor, contextHash: "s".repeat(64) },
	} satisfies FactorCandidateView;

	expect(factorCandidatesForContext([candidate, stale], context)).toEqual([
		candidate,
	]);
	expect(factorCandidatesForContext([candidate], null)).toEqual([]);
});

test("shows cached families before replacing them with a refresh", async () => {
	const pending: Deferred<FactorPage<FactorFamilyView>> = deferred();
	writeFactorCache("user-1", "families", page([family("cached-family")]));
	const mounted = mount(
		makeAdapter({ listFamilies: jest.fn(() => pending.promise) }),
	);

	await act(async () => {
		mounted.root.render(
			<QueryClientProvider client={mounted.queryClient}>
				<FactorsPage adapter={mounted.adapter} />
			</QueryClientProvider>,
		);
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

test("blocks raw materialization without context and inspects completed Dataset rows", async () => {
	const dataset: FactorDatasetView = {
		manifest: {
			datasetId: "dataset-1",
			candidateHash: "candidate-1",
			featureDatasetId: "feature-1",
			marketDataSnapshotId: "snapshot-1",
			pointInTimeUniverseId: "universe-1",
			observationRange: { startTimeMs: 1, endTimeMs: 2 },
			engineIdentity: { engineId: "adaq-native-factor" },
		},
		byteSize: 128,
		lockedBy: [],
		createdAtMs: 1,
	};
	const adapter = {
		...makeAdapter(),
		listCandidates: async () => page([]),
		listDatasets: async () => page([dataset]),
		getDataset: async () => dataset,
		datasetRows: async () => ({
			rows: [
				{
					instrumentId: "okx:BTC-USDT",
					observationTimeMs: 1,
					values: { score: { state: "available", value: 1 } },
				},
			],
			offset: 0,
			limit: 50,
			nextOffset: null,
			total: 1,
		}),
	} as unknown as FactorAdapter;
	const mounted = mount(adapter);

	await act(async () => {
		mounted.root.render(
			<QueryClientProvider client={mounted.queryClient}>
				<FactorsPage adapter={mounted.adapter} />
			</QueryClientProvider>,
		);
	});
	await settle();

	const datasetsTab = Array.from(
		mounted.container.querySelectorAll('[role="tab"]'),
	).find((tab) => tab.textContent === i18n.t("factors.tabs.datasets"));
	expect(datasetsTab).toBeTruthy();
	await act(async () => {
		(datasetsTab as HTMLElement).click();
	});
	await settle();

	expect(mounted.container.textContent).toContain(
		i18n.t("factors.datasets.materializationContextRequired"),
	);
	const startButton = Array.from(
		mounted.container.querySelectorAll("button"),
	).find(
		(button) =>
			button.textContent === i18n.t("factors.datasets.materializationStart"),
	);
	expect(startButton?.hasAttribute("disabled")).toBe(true);

	const inspectButton = Array.from(
		mounted.container.querySelectorAll("button"),
	).find((button) => button.textContent === i18n.t("factors.datasets.inspect"));
	expect(inspectButton).toBeTruthy();
	await act(async () => {
		(inspectButton as HTMLElement).click();
	});
	await settle();
	expect(mounted.container.textContent).toContain("dataset-1");
	expect(mounted.container.textContent).toContain("okx:BTC-USDT");

	await unmount(mounted.root, mounted.container);
});
