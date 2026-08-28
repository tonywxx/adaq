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
import { AttemptsPanel } from "./factor-attempts-panel";
import { localizedFactorError } from "./factor-workspace-support";
import type { FactorAdapter } from "./factor-adapter";
import type {
	FactorComponentCandidateView,
	FactorComponentQualificationView,
	FactorCandidateView,
	FactorAttemptView,
	FactorDatasetView,
	FactorDecisionView,
	FactorFamilyView,
	FactorLineageView,
	FactorPage,
	FactorPolicyView,
	FactorReportView,
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
	jest.useRealTimers();
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
	).toContain(i18n.t("factors.codes.factor-research-failed"));

	await unmount(mounted.root, mounted.container);
});

test("keeps failed Attempt identity, recovery code, and retry feedback accessible", async () => {
	const attempt: FactorAttemptView = {
		attemptId: "attempt-1234567890",
		userId: "user-1",
		kind: "factor-materialization",
		requestHash: "r".repeat(64),
		status: "failed",
		sourceAttemptId: "source-1234567890",
		resultId: null,
		completedUnits: 3,
		progressTotal: 10,
		failureCode: "research-interrupted",
		diagnostic: "research-interrupted: research run stopped before publication",
		createdAtMs: 1,
		updatedAtMs: 2,
	};
	const mounted = mount(
		makeAdapter({ listAttempts: async () => page([attempt]) }),
	);

	await act(async () => {
		mounted.root.render(
			<QueryClientProvider client={mounted.queryClient}>
				<AttemptsPanel
					userId="user-1"
					adapter={mounted.adapter}
					kind="factor-materialization"
				/>
			</QueryClientProvider>,
		);
	});
	await settle();

	const group = mounted.container.querySelector("fieldset");
	expect(group).not.toBeNull();
	expect(group?.getAttribute("aria-busy")).toBe("false");
	expect(mounted.container.textContent).toContain(
		i18n.t("factors.attempts.recoveredDiagnostic"),
	);
	expect(mounted.container.textContent).toContain("research-interrupted");
	expect(mounted.container.textContent).toContain("source-123456789…");
	expect(mounted.container.querySelector('[role="alert"]')).not.toBeNull();
	expect(mounted.container.querySelector("button")?.textContent).toContain(
		i18n.t("factors.attempts.retry"),
	);

	await unmount(mounted.root, mounted.container);
});

test("localizes transient Factor failure categories", async () => {
	const translate = (key: string, options?: Record<string, unknown>) =>
		i18n.t(key, options);
	const previousLocale = i18n.language;
	await i18n.changeLanguage("zh-CN");
	expect(
		localizedFactorError(
			"Factor Candidate predecessor identity is invalid",
			translate,
		),
	).toBe(i18n.t("factors.codes.factor-validation-failed"));
	expect(
		localizedFactorError("dataset publication cannot be published", translate),
	).toBe(i18n.t("factors.codes.factor-publication-failed"));
	await i18n.changeLanguage(previousLocale);
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

test("starts evaluation from Host-owned Candidate and Dataset selections", async () => {
	const candidateHash = "c".repeat(64);
	const context = {
		contextRevision: 2,
		contextHash: "h".repeat(64),
		market: "crypto",
		venue: "okx",
		rangeStartMs: 1,
		rangeEndMs: 20,
		snapshotId: "snapshot-1",
		universeId: "universe-1",
		evidence: [],
		featureDataset: {
			datasetId: "feature-1",
			requestHash: "r".repeat(64),
			featurePlanHash: "f".repeat(64),
			contentSha256: "d".repeat(64),
			outputNames: ["momentum"],
		},
	};
	const candidate: FactorCandidateView = {
		candidate: {
			candidateHash,
			scope: "time-series",
			outputs: [{ name: "momentum" }],
		},
		presentation: { name: "Momentum" },
		lockedBy: [],
		createdAtMs: 1,
		predecessor: {
			...context,
			userId: "user-1",
			evidence: [],
			featureDataset: context.featureDataset,
		},
	};
	const dataset: FactorDatasetView = {
		manifest: {
			datasetId: "dataset-1",
			candidateHash,
			scope: "time-series",
			featureDatasetId: "feature-1",
			featurePlanHash: "f".repeat(64),
			marketDataSnapshotId: "snapshot-1",
			pointInTimeUniverseId: "universe-1",
			outputNames: ["momentum"],
			observationCount: 10,
		},
		byteSize: 128,
		lockedBy: [],
		createdAtMs: 1,
	};
	const startEvaluationFromContext = jest.fn(
		async () => ({}) as FactorAttemptView,
	);
	const adapter = {
		...makeAdapter(),
		listCandidates: async () => page([candidate]),
		listDatasets: async () => page([dataset]),
		listReports: async () => page([]),
		metricCatalog: async () => ({ definitions: [] }),
		startEvaluationFromContext,
	} as unknown as FactorAdapter;
	const invokeMock = jest.requireMock("@tauri-apps/api/core")
		.invoke as jest.Mock;
	invokeMock.mockImplementation(async (command: string) =>
		command === "research_context_get" ? context : null,
	);
	const mounted = mount(adapter);

	await act(async () => {
		mounted.root.render(
			<QueryClientProvider client={mounted.queryClient}>
				<FactorsPage adapter={mounted.adapter} />
			</QueryClientProvider>,
		);
	});
	await settle();

	const evaluationsTab = Array.from(
		mounted.container.querySelectorAll('[role="tab"]'),
	).find((tab) => tab.textContent === i18n.t("factors.tabs.evaluations"));
	await act(async () => {
		(evaluationsTab as HTMLElement).click();
	});
	await settle();

	expect(mounted.container.textContent).toContain(candidateHash);
	expect(mounted.container.textContent).toContain("dataset-1");
	const startButton = Array.from(
		mounted.container.querySelectorAll("button"),
	).find((button) => button.textContent === i18n.t("factors.evaluations.start"));
	expect(startButton).toBeTruthy();
	await act(async () => {
		(startButton as HTMLElement).click();
	});
	await settle();

	expect(startEvaluationFromContext).toHaveBeenCalledWith(
		"user-1",
		candidateHash,
		"dataset-1",
		"momentum",
	);
	expect(mounted.container.textContent).toContain(
		i18n.t("factors.evaluations.hostOwnsEvidence"),
	);

	await unmount(mounted.root, mounted.container);
});

test("records a Decision only after structured evidence is frozen", async () => {
	const candidateHash = "c".repeat(64);
	const reportHash = "r".repeat(64);
	const policyHash = "p".repeat(64);
	const familyId = "11111111-1111-4111-8111-111111111111";
	const trialId = "22222222-2222-4222-8222-222222222222";
	const candidate: FactorCandidateView = {
		candidate: { candidateHash, outputs: [{ name: "momentum" }] },
		presentation: { name: "Momentum" },
		lockedBy: [],
		createdAtMs: 1,
	};
	const dataset: FactorDatasetView = {
		manifest: {
			datasetId: "dataset-1",
			candidateHash,
			outputNames: ["momentum"],
		},
		byteSize: 1,
		lockedBy: [],
		createdAtMs: 1,
	};
	const report: FactorReportView = {
		report: {
			reportHash,
			factorDatasetId: "dataset-1",
			outputName: "momentum",
			evidenceState: "out-of-sample",
		},
		protocol: {
			familyId,
			trialId,
			factorDatasetId: "dataset-1",
			outputName: "momentum",
		},
		lockedBy: [],
		createdAtMs: 1,
	};
	const policy: FactorPolicyView = {
		policy: { policyHash, revision: 1 },
		createdAtMs: 1,
	};
	const freezePromotionProtocol = jest.fn(async () => ({
		protocolHash: "t".repeat(64),
	}));
	const recordDecision = jest.fn(async () => ({}));
	const lineage = deferred<FactorLineageView>();
	const adapter = {
		...makeAdapter(),
		listCandidates: async () => page([candidate]),
		listDatasets: async () => page([dataset]),
		listReports: async () => page([report]),
		listPolicies: async () => page([policy]),
		listDecisions: async () => page([]),
		listDecisionLibrary: async () => page([]),
		getLineage: () => lineage.promise,
		freezePromotionProtocol,
		recordDecision,
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

	const decisionsTab = Array.from(
		mounted.container.querySelectorAll('[role="tab"]'),
	).find((tab) => tab.textContent === i18n.t("factors.tabs.decisions"));
	await act(async () => {
		(decisionsTab as HTMLElement).click();
	});
	await settle();

	const select = (id: string, value: string) => {
		const element = mounted.container.querySelector(id) as HTMLSelectElement;
		element.value = value;
		element.dispatchEvent(new Event("change", { bubbles: true }));
	};
	await act(async () => select("#factor-decision-candidate", candidateHash));
	await act(async () => select("#factor-decision-dataset", "dataset-1"));
	await act(async () => select("#factor-decision-output", "momentum"));
	await act(async () => select("#factor-decision-report", reportHash));
	await act(async () => select("#factor-decision-policy", policyHash));
	await settle();
	const evidenceDetails = Array.from(
		mounted.container.querySelectorAll("dt"),
	).map((label) => label.parentElement?.textContent);
	expect(evidenceDetails).toEqual(
		expect.arrayContaining([
			`${i18n.t("factors.decisions.candidateSelection")}${candidateHash}`,
			`${i18n.t("factors.decisions.datasetSelection")}dataset-1`,
			`${i18n.t("factors.decisions.reportSelection")}${reportHash}`,
			`${i18n.t("factors.decisions.policySelection")}${policyHash}`,
		]),
	);

	const freezeButton = Array.from(
		mounted.container.querySelectorAll("button"),
	).find(
		(button) => button.textContent === i18n.t("factors.decisions.freezeProtocol"),
	);
	expect((freezeButton as HTMLButtonElement).disabled).toBe(true);
	await act(async () => {
		lineage.resolve({
			lineage: { lineageHash: "l".repeat(64) },
			trials: [{ trialId, status: "completed" }],
			registrations: [],
			protocols: [],
		});
	});
	await settle();
	expect((freezeButton as HTMLButtonElement).disabled).toBe(false);
	await act(async () => (freezeButton as HTMLElement).click());
	await settle();
	const recordButton = Array.from(
		mounted.container.querySelectorAll("button"),
	).find(
		(button) => button.textContent === i18n.t("factors.decisions.recordDecision"),
	);
	await act(async () => (recordButton as HTMLElement).click());
	await settle();

	expect(freezePromotionProtocol).toHaveBeenCalledWith("user-1", {
		candidateHash,
		datasetId: "dataset-1",
		outputName: "momentum",
		familyId,
		trialId,
		reportHashes: [reportHash],
		policyHash,
	});
	expect(recordDecision).toHaveBeenCalledWith(
		"user-1",
		"rejected",
		{ protocolHash: "t".repeat(64) },
		expect.objectContaining({
			deterministicExecution: false,
			completeSourceProvenance: false,
			abiV2Expressible: false,
			buildable: false,
		}),
		null,
	);

	await unmount(mounted.root, mounted.container);
});

test("runs Gate 6 from a current Component Eligible Decision to Library inspection", async () => {
	const candidateHash = "c".repeat(64);
	const decisionId = "11111111-1111-4111-8111-111111111111";
	const reportHash = "r".repeat(64);
	const secondReportHash = "s".repeat(64);
	const policyHash = "p".repeat(64);
	const featurePlanHash = "f".repeat(64);
	const packageHash = "a".repeat(64);
	const wasmHash = "w".repeat(64);
	const candidate: FactorCandidateView = {
		candidate: {
			candidateHash,
			candidateId: decisionId,
			revision: 2,
			scope: "time-series",
			parameters: [{ name: "window", defaultValue: "20" }],
			outputs: [{ name: "momentum" }],
			source: {
				kind: "declarative",
				definition: { featurePlanHash },
			},
		},
		presentation: { name: "Momentum" },
		lockedBy: [],
		createdAtMs: 1,
		predecessor: {
			userId: "user-1",
			contextRevision: 3,
			contextHash: "h".repeat(64),
			market: "crypto",
			venue: "okx",
			rangeStartMs: 10,
			rangeEndMs: 20,
			snapshotId: "snapshot-1",
			universeId: "universe-1",
			evidence: [],
			featureDataset: {
				datasetId: "feature-1",
				requestHash: "q".repeat(64),
				featurePlanHash,
				contentSha256: "d".repeat(64),
				outputNames: ["close"],
			},
		},
	};
	const dataset: FactorDatasetView = {
		manifest: {
			datasetId: "factor-1",
			protocolHash: "protocol".repeat(8),
			candidateHash,
			featureDatasetId: "feature-1",
			featurePlanHash,
			marketDataSnapshotId: "snapshot-1",
			pointInTimeUniverseId: "universe-1",
			observationRange: { startTimeMs: 10, endTimeMs: 20 },
			outputNames: ["momentum"],
		},
		byteSize: 128,
		lockedBy: [],
		createdAtMs: 1,
	};
	const report: FactorReportView = {
		report: {
			reportHash,
			factorDatasetId: "factor-1",
			outputName: "momentum",
			evidenceState: "out-of-sample",
		},
		protocol: {
			familyId: "22222222-2222-4222-8222-222222222222",
			trialId: "33333333-3333-4333-8333-333333333333",
		},
		lockedBy: [],
		createdAtMs: 1,
	};
	const secondReport: FactorReportView = {
		...report,
		report: { ...report.report, reportHash: secondReportHash },
	};
	const policy: FactorPolicyView = {
		policy: { policyHash, revision: 4 },
		createdAtMs: 1,
	};
	const decision: FactorDecisionView = {
		decision: {
			decisionId,
			candidateHash,
			outputName: "momentum",
			state: "component-eligible",
			reportHashes: [reportHash, secondReportHash],
			policyHash,
			evidenceState: "out-of-sample",
		},
		promotionProtocolHash: "protocol".repeat(8),
		eligibilityGates: [{ gate: "complete-lineage", passed: true }],
		createdAtMs: 1,
	};
	const buildAttempt: FactorAttemptView = {
		attemptId: "44444444-4444-4444-8444-444444444444",
		userId: "user-1",
		kind: "factor-component-build",
		requestHash: "b".repeat(64),
		status: "completed",
		resultId: packageHash,
		completedUnits: 1,
		progressTotal: 1,
		diagnostic: null,
		failureCode: null,
		createdAtMs: 1,
		updatedAtMs: 1,
	};
	const qualificationAttempt: FactorAttemptView = {
		...buildAttempt,
		attemptId: "55555555-5555-4555-8555-555555555555",
		kind: "factor-component-qualification",
		requestHash: "q".repeat(64),
	};
	const failedQualificationAttempt: FactorAttemptView = {
		...qualificationAttempt,
		status: "failed",
		resultId: null,
		failureCode: "factor-component-qualification-failed",
		diagnostic: "factor-component-qualification-failed: equivalence failed",
	};
	const componentCandidate: FactorComponentCandidateView = {
		attemptId: buildAttempt.attemptId,
		userId: "user-1",
		packageSha256: packageHash,
		manifest: {
			name: "Momentum Factor",
			componentId: decisionId,
			version: "1.0.0",
			kind: "factor",
			wasmSha256: wasmHash,
			outputNames: ["momentum"],
		},
		binding: { candidate: candidate.candidate },
	};
	const qualification: FactorComponentQualificationView = {
		attempt: qualificationAttempt,
		candidateAttemptId: buildAttempt.attemptId,
		packageSha256: packageHash,
		binding: { candidate: candidate.candidate },
		qualification: {
			qualified: true,
			componentId: decisionId,
			version: "1.0.0",
			evidence: [],
		},
		provenance: {
			sourceSha256: "s".repeat(64),
			sdkVersion: "0.9.2",
			abiVersion: "1.0.0",
			toolchain: "stable",
			compiler: "rustc",
			target: "wasm32-unknown-unknown",
			packageSha256: packageHash,
		},
		equivalence: {
			comparisonContract: "bit-identical",
			inputIdentitySha256: "i".repeat(64),
			frozenOutputSha256: "o".repeat(64),
			cases: [{ passed: true }],
		},
		published: true,
		evidenceCreatedAtMs: 2,
	};
	const failedQualification: FactorComponentQualificationView = {
		...qualification,
		attempt: failedQualificationAttempt,
		qualification: { qualified: false, reason: "equivalence failed" },
		provenance: null,
		equivalence: null,
		published: false,
	};
	const libraryComponent = {
		componentId: decisionId,
		version: "1.0.0",
		manifestSchemaVersion: "1.0.0",
		sdkVersion: "0.9.2",
		abiVersion: "1.0.0",
		name: "Momentum Factor",
		kind: "factor" as const,
		archiveSha256: packageHash,
		wasmSha256: wasmHash,
		parameters: [],
		featureSlots: [],
		outputNames: ["momentum"],
		dependencies: [],
		warmupBars: 20,
		compatible: true,
		lockedByRunIds: [],
	};
	const pendingBuildAttempt: FactorAttemptView = {
		...buildAttempt,
		status: "running",
		resultId: null,
		completedUnits: 0,
	};
	const cancelledBuildAttempt: FactorAttemptView = {
		...pendingBuildAttempt,
		status: "cancelled",
		failureCode: "cancelled",
		diagnostic: "cancelled: user requested",
	};
	const prepareComponent = jest.fn(async () => pendingBuildAttempt);
	const prepareComponentQualification = jest.fn(
		async () => failedQualificationAttempt,
	);
	const buildPoll = deferred<FactorAttemptView>();
	const getAttempt = jest.fn(() => buildPoll.promise);
	const cancelAttempt = jest.fn(async () => undefined);
	const getComponentQualification = jest
		.fn()
		.mockResolvedValueOnce(failedQualification)
		.mockResolvedValue(qualification);
	const retryComponentAttempt = jest
		.fn()
		.mockResolvedValueOnce(buildAttempt)
		.mockResolvedValueOnce(qualificationAttempt);
	const listComponents = jest
		.fn()
		.mockResolvedValueOnce([libraryComponent])
		.mockResolvedValue([libraryComponent]);
	const adapter = {
		...makeAdapter(),
		listCandidates: async () => page([candidate]),
		listDatasets: async () => page([dataset]),
		listReports: async () => page([report, secondReport]),
		listPolicies: async () => page([policy]),
		listDecisions: async () => page([decision]),
		listDecisionLibrary: async () => page([decision]),
		prepareComponent,
		getAttempt,
		cancelAttempt,
		getComponentCandidate: async () => componentCandidate,
		prepareComponentQualification,
		getComponentQualification,
		retryComponentAttempt,
		listComponents,
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
	const decisionsTab = Array.from(
		mounted.container.querySelectorAll('[role="tab"]'),
	).find((tab) => tab.textContent === i18n.t("factors.tabs.decisions"));
	await act(async () => (decisionsTab as HTMLElement).click());
	await settle();

	expect(mounted.container.textContent).toContain(i18n.t("factors.gate6.ready"));
	expect(mounted.container.textContent).toContain("feature-1");
	expect(mounted.container.textContent).toContain("snapshot-1");
	expect(mounted.container.textContent).toContain("universe-1");
	expect(mounted.container.textContent).toContain(secondReportHash);
	expect(
		mounted.container.querySelector('label[for="factor-gate6-decision"]'),
	).not.toBeNull();
	expect(
		mounted.container.querySelector('label[for="factor-gate6-output"]'),
	).not.toBeNull();
	const startButton = Array.from(
		mounted.container.querySelectorAll("button"),
	).find((button) => button.textContent === i18n.t("factors.gate6.start"));
	expect(startButton).toBeTruthy();
	await act(async () => (startButton as HTMLElement).click());
	await settle();
	expect(mounted.container.querySelector("progress")).not.toBeNull();
	const cancelButton = Array.from(
		mounted.container.querySelectorAll("button"),
	).find((button) => button.textContent === i18n.t("factors.gate6.cancel"));
	expect(cancelButton).toBeTruthy();
	await act(async () => (cancelButton as HTMLElement).click());
	expect(cancelAttempt).toHaveBeenCalledWith("user-1", buildAttempt.attemptId);
	await act(async () => buildPoll.resolve(cancelledBuildAttempt));
	await settle();

	expect(prepareComponent).toHaveBeenCalledWith(
		"user-1",
		decisionId,
		"momentum",
	);
	expect(getAttempt).toHaveBeenCalledWith("user-1", buildAttempt.attemptId);
	expect(mounted.container.textContent).toContain("cancelled");
	expect(listComponents).not.toHaveBeenCalled();
	const cancelledRetryButton = Array.from(
		mounted.container.querySelectorAll("button"),
	).find((button) => button.textContent === i18n.t("factors.gate6.retry"));
	expect(cancelledRetryButton).toBeTruthy();
	await act(async () => (cancelledRetryButton as HTMLElement).click());
	await settle();
	expect(prepareComponentQualification).toHaveBeenCalledWith(
		"user-1",
		buildAttempt.attemptId,
	);
	expect(mounted.container.textContent).toContain(
		"factor-component-qualification-failed",
	);
	expect(mounted.container.textContent).toContain(
		i18n.t("factors.gate6.notPublished"),
	);
	expect(mounted.container.textContent).not.toContain(
		i18n.t("factors.gate6.entitlementGranted"),
	);
	const retryButton = Array.from(
		mounted.container.querySelectorAll("button"),
	).find((button) => button.textContent === i18n.t("factors.gate6.retry"));
	expect(retryButton).toBeTruthy();
	await act(async () => (retryButton as HTMLElement).click());
	await settle();
	expect(retryComponentAttempt).toHaveBeenCalledWith(
		"user-1",
		failedQualificationAttempt.attemptId,
	);
	expect(mounted.container.textContent).toContain(packageHash);
	expect(mounted.container.textContent).toContain("sourceSha256");
	expect(mounted.container.textContent).toContain("inputIdentitySha256");
	expect(mounted.container.textContent).toContain("Momentum Factor");
	expect(mounted.container.textContent).toContain(
		i18n.t("factors.gate6.entitlementGranted"),
	);
	expect(
		mounted.container.querySelector('a[to="/components"], a[href="/components"]'),
	).not.toBeNull();
	const previousLocale = i18n.language;
	await act(async () => i18n.changeLanguage("zh-CN"));
	expect(mounted.container.textContent).toContain(
		"Gate 6 · 资格认定 Factor Decision",
	);
	expect(mounted.container.textContent).toContain(packageHash);
	await act(async () => i18n.changeLanguage(previousLocale));

	await unmount(mounted.root, mounted.container);
});

test("blocks Gate 6 when a Component Eligible Decision has no User-scoped Candidate", async () => {
	const decision: FactorDecisionView = {
		decision: {
			decisionId: "66666666-6666-4666-8666-666666666666",
			candidateHash: "c".repeat(64),
			outputName: "momentum",
			state: "component-eligible",
		},
		promotionProtocolHash: "p".repeat(64),
		eligibilityGates: [],
		createdAtMs: 1,
	};
	const secondDecision: FactorDecisionView = {
		...decision,
		decision: {
			...decision.decision,
			decisionId: "77777777-7777-4777-8777-777777777777",
			candidateHash: "d".repeat(64),
			outputName: "reversal",
		},
	};
	const listDecisionLibrary = jest.fn(
		async (_userId: string, pageNumber: number) =>
			pageNumber === 1
				? { items: [decision], page: 1, pageSize: 1, total: 2 }
				: { items: [secondDecision], page: 2, pageSize: 1, total: 2 },
	);
	const adapter = {
		...makeAdapter(),
		listCandidates: async () => page([]),
		listDatasets: async () => page([]),
		listReports: async () => page([]),
		listPolicies: async () => page([]),
		listDecisions: async () => page([]),
		listDecisionLibrary,
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
	const decisionsTab = Array.from(
		mounted.container.querySelectorAll('[role="tab"]'),
	).find((tab) => tab.textContent === i18n.t("factors.tabs.decisions"));
	await act(async () => (decisionsTab as HTMLElement).click());
	await settle();

	expect(mounted.container.textContent).toContain(
		i18n.t("factors.gate6.blocked"),
	);
	expect(
		mounted.container.querySelectorAll("#factor-gate6-decision option"),
	).toHaveLength(3);
	const startButton = Array.from(
		mounted.container.querySelectorAll("button"),
	).find((button) => button.textContent === i18n.t("factors.gate6.start"));
	expect(startButton).toBeTruthy();
	expect((startButton as HTMLButtonElement).disabled).toBe(true);

	await unmount(mounted.root, mounted.container);
});
