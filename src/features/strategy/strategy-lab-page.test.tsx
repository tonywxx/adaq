/** @jest-environment jsdom */

import "@/lib/i18n";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { useMarketSessionStore } from "@/lib/market-session";
import { invoke } from "@tauri-apps/api/core";
import { StrategyLabPage } from "./strategy-lab-page";

jest.mock("@tauri-apps/api/core", () => ({ invoke: jest.fn() }));
const mockInvoke = invoke as jest.MockedFunction<typeof invoke>;
jest.mock("@/lib/market-session", () => {
	const state = { userId: null as string | null };
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

(
	globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }
).IS_REACT_ACT_ENVIRONMENT = true;

const hash = (character: string) => character.repeat(64);

const catalog = {
	factorInputs: [
		{
			decisionId: "00000000-0000-4000-8000-000000000001",
			decisionHash: hash("1"),
			candidateHash: hash("2"),
			outputName: "momentum-score",
			packageArchiveSha256: hash("3"),
			packageWasmSha256: hash("4"),
			componentId: "00000000-0000-4000-8000-000000000002",
			componentVersion: "1.0.0",
			featurePlanHash: hash("5"),
			contextHash: hash("6"),
			snapshotId: "snapshot-1",
			universeId: "universe-1",
			market: "crypto",
			venue: "okx",
		},
	],
	modelInputs: [
		{
			qualificationReportId: hash("7"),
			decisionId: hash("8"),
			finalEvaluationReportId: hash("9"),
			artifactSha256: hash("a"),
			transformationSha256: hash("b"),
			packageArchiveSha256: hash("c"),
			packageWasmSha256: hash("d"),
			componentId: "00000000-0000-4000-8000-000000000003",
			componentVersion: "1.0.0",
			modelProfile: "adaq:wasi-model@1",
			exporterId: "adaq:exporter@1",
			sdkVersion: "2.0.0",
			abiVersion: "2.0.0",
			runtimeIdentity: "runtime-1",
			inputSlots: ["momentum-score"],
			outputName: "forecast",
			targetId: "future-close-return",
			targetHorizonBars: 5,
			forecastContract: "forecast:continuous-future-close-return:native@1",
		},
	],
};

const candidate = {
	candidateId: "00000000-0000-4000-8000-000000000004",
	userId: "user-1",
	scope: "portfolio",
	state: "frozen-revision",
	eligible: true,
	revisions: [
		{
			revision: {
				revision: 1,
				scope: "portfolio",
				definition: {
					schemaVersion: "adaq:strategy-candidate@1",
					catalogVersion: "adaq:strategy-operations@1",
					inputSlots: [],
					nodes: [],
					output: { kind: "portfolio-target", nodeId: "reserve-cash" },
				},
				revisionHash: hash("f"),
				semanticContext: {
					featurePlanHash: hash("5"),
					researchContextHash: hash("6"),
					snapshotId: "snapshot-1",
					universeId: "universe-1",
					market: "crypto",
					venue: "okx",
					inputEvidenceHashes: [hash("e")],
				},
				createdAtMs: 1,
				createdByAttemptId: "attempt-1",
			},
			eligible: true,
		},
	],
	attempts: [],
};

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
	mockInvoke.mockImplementation(async (command: string) => {
		if (command === "strategy_candidate_catalog")
			return { ...catalog, operations: [] };
		if (command === "strategy_candidate_list") return [];
		if (command === "strategy_candidate_preflight")
			return {
				attemptId: "attempt-1",
				candidateId: candidate.candidateId,
				nextRevision: 1,
				status: "ready-to-create",
				diagnostics: [],
			};
		if (command === "strategy_candidate_create") return candidate;
		throw new Error(`unexpected command: ${command}`);
	});
	useMarketSessionStore.setState({ userId: "user-1" });
	Object.defineProperty(globalThis, "requestAnimationFrame", {
		configurable: true,
		value: (callback: FrameRequestCallback) => {
			callback(0);
			return 0;
		},
	});
});

afterEach(() => {
	mockInvoke.mockReset();
	useMarketSessionStore.getState().clear();
	document.body.replaceChildren();
});

test("builds a bounded definition and requires Host preflight before Create", async () => {
	const container = document.createElement("div");
	const root = createRoot(container);
	document.body.append(container);

	await act(async () => root.render(<StrategyLabPage />));
	await settle();

	expect(container.textContent).toContain("Strategy Candidate Lab");
	expect(container.textContent).toContain("cash-reserve");
	expect(container.textContent).toContain("Draft");
	const buttons = Array.from(container.querySelectorAll("button"));
	const preflight = buttons.find((button) =>
		button.textContent?.includes("Run Host preflight"),
	);
	expect(preflight).toBeDefined();
	const create = buttons.find((button) =>
		button.textContent?.includes("Create immutable Revision"),
	);
	expect(create?.disabled).toBe(true);

	await act(async () => preflight?.click());
	await settle();
	expect(mockInvoke).toHaveBeenCalledWith(
		"strategy_candidate_preflight",
		expect.objectContaining({ request: expect.any(Object) }),
	);
	expect(container.textContent).toContain("Ready to create");
	expect(create?.disabled).toBe(false);

	await act(async () => create?.click());
	await settle();
	expect(container.textContent).toContain("Revision published.");
	expect(container.textContent).toContain("Published");
	expect(mockInvoke).toHaveBeenCalledWith("strategy_candidate_create", {
		request: { attemptId: "attempt-1" },
	});

	await unmount(root, container);
});

test("keeps rejected Host diagnostics localized and retryable", async () => {
	let preflightCalls = 0;
	mockInvoke.mockImplementation(async (command: string) => {
		if (command === "strategy_candidate_catalog")
			return { ...catalog, operations: [] };
		if (command === "strategy_candidate_list") return [];
		if (command === "strategy_candidate_preflight") {
			preflightCalls += 1;
			return {
				attemptId: `attempt-${preflightCalls}`,
				candidateId: candidate.candidateId,
				nextRevision: 0,
				status: "rejected",
				diagnostics: [
					{ code: "strategy-factor-input-not-accepted", path: "definition" },
				],
			};
		}
		if (command === "strategy_candidate_retry")
			return {
				attemptId: "attempt-2",
				candidateId: candidate.candidateId,
				nextRevision: 0,
				status: "rejected",
				diagnostics: [
					{ code: "strategy-factor-input-not-accepted", path: "definition" },
				],
			};
		throw new Error(`unexpected command: ${command}`);
	});
	const container = document.createElement("div");
	const root = createRoot(container);
	document.body.append(container);

	await act(async () => root.render(<StrategyLabPage />));
	await settle();
	const preflight = Array.from(container.querySelectorAll("button")).find(
		(button) => button.textContent?.includes("Run Host preflight"),
	);
	await act(async () => preflight?.click());
	await settle();

	expect(container.textContent).toContain("Preflight rejected");
	expect(container.textContent).toContain("Host rejected this input:");
	expect(
		Array.from(container.querySelectorAll("button")).find((button) =>
			button.textContent?.includes("Retry same frozen Attempt"),
		),
	).toBeDefined();
	const create = Array.from(container.querySelectorAll("button")).find(
		(button) => button.textContent?.includes("Create immutable Revision"),
	);
	expect(create?.disabled).toBe(true);

	const retry = Array.from(container.querySelectorAll("button")).find((button) =>
		button.textContent?.includes("Retry same frozen Attempt"),
	);
	await act(async () => retry?.click());
	await settle();
	expect(mockInvoke).toHaveBeenCalledWith("strategy_candidate_retry", {
		request: { attemptId: "attempt-1" },
	});

	await unmount(root, container);
});
