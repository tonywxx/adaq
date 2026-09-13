/** @jest-environment jsdom */

import { i18n } from "@/lib/i18n";
import { AuthenticatedUserContext } from "@/authenticated-user";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import type { ReactNode } from "react";
import { PaperExperimentPage } from "@/features/paper-experiment/paper-experiment-page";

jest.mock("@tauri-apps/api/core", () => ({
	Channel: class {},
	invoke: jest.fn(),
}));
jest.mock("@tanstack/react-router", () => ({
	Link: ({ to, children }: { to: string; children: ReactNode }) => (
		<a href={to}>{children}</a>
	),
}));
const mockInvoke = invoke as jest.MockedFunction<typeof invoke>;

(
	globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }
).IS_REACT_ACT_ENVIRONMENT = true;

const instruments = ["BTC-USDT", "ETH-USDT", "SOL-USDT"] as const;
const qualifications = instruments.map((_, index) => ({
	qualificationId: `qualification-${index + 1}`,
	candidateId: "ema-double-cross-v1",
	gate12Eligible: true,
	gate12ContinuationRequired: false,
}));
const profiles = [
	{
		profileId: "profile-1",
		provider: "okx_demo",
		accountId: "account-1",
		status: "usable",
	},
];

type MutableView = {
	experiment: {
		experimentId: string;
		profileId: string;
		accountId: string;
		observationStartMs: number;
		observationEndMs: number;
		allocationTotalUsdt: string;
		unallocatedRemainderUsdt: string;
		state: string;
		limitations: string[];
		instruments: Array<{
			instrument: (typeof instruments)[number];
			qualificationId: string;
			botId: string | null;
			allocationUsdt: string;
			entryNotionalCapUsdt: string;
			reservedCashUsdt: string;
		}>;
		feedbackSnapshotIds: string[];
		feedbackReportIds: string[];
	};
	bots: Array<{
		botId: string;
		state: string;
		currentAttemptId: string | null;
		control: {
			canStart: boolean;
			canRetry: boolean;
			canPause: boolean;
			canResume: boolean;
		};
		attempts: Array<{
			attemptId: string;
			unmanagedPositions: string[];
			reconciliationRequired: boolean;
			evidence: Array<{ code: string }>;
		}>;
	}>;
	valuations: Array<{
		instrument: (typeof instruments)[number];
		observedAtMs: number;
	}>;
	account: unknown;
	report: unknown;
};

function draftView(): MutableView {
	return {
		experiment: {
			experimentId: "experiment-1",
			profileId: "profile-1",
			accountId: "account-1",
			observationStartMs: Date.parse("2026-09-12T00:00:00Z"),
			observationEndMs: Date.parse("2026-09-12T00:02:00Z"),
			allocationTotalUsdt: "98084.28",
			unallocatedRemainderUsdt: "0.00951395709",
			state: "draft",
			limitations: [],
			instruments: instruments.map((instrument, index) => ({
				instrument,
				qualificationId: qualifications[index].qualificationId,
				botId: null,
				allocationUsdt: "32694.76",
				entryNotionalCapUsdt: "32367.81",
				reservedCashUsdt: "326.95",
			})),
			feedbackSnapshotIds: [],
			feedbackReportIds: [],
		},
		bots: [],
		valuations: [],
		account: null,
		report: null,
	};
}

function completedView() {
	const view = draftView();
	view.experiment.state = "completed";
	view.experiment.instruments = view.experiment.instruments.map(
		(binding, index) => ({ ...binding, botId: `bot-${index + 1}` }),
	);
	view.experiment.feedbackSnapshotIds = ["snapshot-1"];
	view.experiment.feedbackReportIds = ["feedback-report-1"];
	view.bots = instruments.map((instrument, index) => ({
		botId: `bot-${index + 1}`,
		state: "running",
		currentAttemptId: `attempt-${index + 1}`,
		control: {
			canStart: false,
			canRetry: false,
			canPause: true,
			canResume: false,
		},
		attempts: [
			{
				attemptId: `attempt-${index + 1}`,
				unmanagedPositions: index === 0 ? [instrument] : [],
				reconciliationRequired: false,
				evidence: [{ code: "warmup-complete" }],
			},
		],
	}));
	view.valuations = instruments.flatMap((instrument) => [
		{ instrument, observedAtMs: view.experiment.observationStartMs },
		{ instrument, observedAtMs: view.experiment.observationEndMs },
	]);
	view.account = { reconciliation: "fresh", account: { cash: "97103.43" } };
	view.report = {
		reportId: "report-1",
		evidenceState: "ready",
		rankingEligible: true,
		valuationCadenceMs: 30_000,
		rankingBlockedReason: null,
		rankedInstruments: ["ETH-USDT", "BTC-USDT", "SOL-USDT"],
		bestInstrument: "ETH-USDT",
		commonEndValuationAtMs: view.experiment.observationEndMs,
		limitations: [],
		instruments: instruments.map((instrument, index) => ({
			instrument,
			botId: `bot-${index + 1}`,
			endingCashUsdt: "32367.81",
			endingPositionQuantity: "0",
			endingPositionValueUsdt: "0",
			realizedPnlUsdt: "0",
			unrealizedPnlUsdt: "0",
			feesUsdt: "0.12",
			netEquityReturn: "0.0123",
			maxDrawdown: "0.005",
			completedTrades: 2,
			valuationCount: 2,
			exposureTimeMs: 60_000,
			interruptions: 1,
			evidenceState: "ready",
			limitations: [],
		})),
	};
	return view;
}

async function settle() {
	await act(async () => {
		await Promise.resolve();
		await Promise.resolve();
		await new Promise((resolve) => window.setTimeout(resolve, 0));
	});
}

async function mount() {
	const container = document.createElement("div");
	const root = createRoot(container);
	document.body.append(container);
	await act(async () => {
		root.render(
			<AuthenticatedUserContext.Provider value="alice">
				<QueryClientProvider client={new QueryClient()}>
					<PaperExperimentPage />
				</QueryClientProvider>
			</AuthenticatedUserContext.Provider>,
		);
	});
	await settle();
	return { container, root };
}

async function changeValue(
	element: HTMLInputElement | HTMLSelectElement,
	value: string,
) {
	await act(async () => {
		const setter = Object.getOwnPropertyDescriptor(
			Object.getPrototypeOf(element),
			"value",
		)?.set;
		setter?.call(element, value);
		element.dispatchEvent(new Event("change", { bubbles: true }));
	});
	await settle();
}

async function unmount(root: Root, container: HTMLDivElement) {
	await act(async () => root.unmount());
	container.remove();
}

let view: ReturnType<typeof draftView> | null;

beforeEach(() => {
	view = null;
	mockInvoke.mockImplementation(async (command: string, args?: unknown) => {
		if (command === "paper_experiment_view") return view;
		if (command === "strategy_qualification_list") return qualifications;
		if (command === "connection_profile_list") return profiles;
		if (
			command === "market_subscribe_trades" ||
			command === "market_unsubscribe_trades"
		)
			return undefined;
		if (command === "paper_experiment_create") {
			view = draftView();
			return view;
		}
		if (command === "paper_experiment_stop") {
			if (view) {
				view.experiment.state = "incomplete";
				view.experiment.limitations = ["shutdown unresolved"];
			}
			throw new Error("shutdown unresolved");
		}
		if (command.startsWith("bot_")) return view;
		throw new Error(`unexpected command ${command} ${JSON.stringify(args)}`);
	});
});

afterEach(() => {
	mockInvoke.mockReset();
	document.body.replaceChildren();
});

test("creates a bounded UTC experiment with all three qualified instruments", async () => {
	const { container, root } = await mount();

	await changeValue(
		container.querySelector("#experiment-profile") as HTMLSelectElement,
		"profile-1",
	);
	expect(
		(container.querySelector("#experiment-profile") as HTMLSelectElement).value,
	).toBe("profile-1");
	for (const [index, instrument] of instruments.entries()) {
		await changeValue(
			container.querySelector(`#experiment-${instrument}`) as HTMLSelectElement,
			qualifications[index].qualificationId,
		);
		expect(
			(container.querySelector(`#experiment-${instrument}`) as HTMLSelectElement)
				.value,
		).toBe(qualifications[index].qualificationId);
	}
	await changeValue(
		container.querySelector("#experiment-start") as HTMLInputElement,
		"2026-09-12T00:00",
	);
	await changeValue(
		container.querySelector("#experiment-end") as HTMLInputElement,
		"2026-09-12T00:02",
	);
	expect(
		(container.querySelector("#experiment-start") as HTMLInputElement).value,
	).toBe("2026-09-12T00:00");
	expect(
		(container.querySelector("#experiment-end") as HTMLInputElement).value,
	).toBe("2026-09-12T00:02");
	await act(async () => {
		Array.from(container.querySelectorAll("button"))
			.find((button) => button.textContent === "Create experiment")
			?.click();
	});
	await settle();

	const createCall = mockInvoke.mock.calls.find(
		([command]) => command === "paper_experiment_create",
	);
	expect(createCall?.[1]).toEqual({
		request: {
			profileId: "profile-1",
			accountId: "account-1",
			observationStartMs: Date.parse("2026-09-12T00:00:00Z"),
			observationEndMs: Date.parse("2026-09-12T00:02:00Z"),
			bindings: instruments.map((instrument, index) => ({
				instrument,
				qualificationId: qualifications[index].qualificationId,
			})),
		},
	});
	await unmount(root, container);
});

test("renders report completeness and retained feedback evidence", async () => {
	view = completedView();
	const { container, root } = await mount();

	for (const text of [
		"Net equity return",
		"Runtime attempts",
		"Exposure time",
		"Interruptions",
		"Retained valuations",
		"Common end valuation",
		"Best observed instrument",
		"ETH-USDT → BTC-USDT → SOL-USDT",
		"Valuation cadence",
		"snapshot-1",
		"feedback-report-1",
		"Unmanaged positions: BTC-USDT",
		"Warmup: complete",
	]) {
		expect(container.textContent).toContain(text);
	}
	await unmount(root, container);
});

test("subscribes to instrument trades while the experiment is preparing", async () => {
	view = completedView();
	view.experiment.state = "preparing";
	const { container, root } = await mount();

	expect(mockInvoke).toHaveBeenCalledWith("market_subscribe_trades", {
		request: {
			src: "okx",
			codes: [...instruments],
			subscriptionId: expect.any(String),
		},
		onEvent: expect.anything(),
	});
	await unmount(root, container);
});

test("keeps per-instrument supervision available during warmup", async () => {
	view = completedView();
	view.experiment.state = "preparing";
	view.report = null;
	const { container, root } = await mount();

	await act(async () => {
		Array.from(container.querySelectorAll("button"))
			.find((button) => button.textContent === "Pause")
			?.click();
	});
	await settle();

	expect(mockInvoke).toHaveBeenCalledWith("bot_pause", {
		request: { botId: "bot-1", commandId: expect.any(String) },
	});
	await unmount(root, container);
});

test("does not display stale warmup evidence after a Worker resume", async () => {
	view = completedView();
	view.experiment.state = "preparing";
	view.report = null;
	view.bots[0].attempts[0].evidence = [
		{ code: "warmup-complete" },
		{ code: "worker-restarted-for-resume" },
	];
	const { container, root } = await mount();

	expect(container.textContent).toContain("Warmup: pending");
	await unmount(root, container);
});

test("routes per-instrument pause through the existing Host control", async () => {
	view = completedView();
	view.experiment.state = "running";
	view.report = null;
	const { container, root } = await mount();

	await act(async () => {
		Array.from(container.querySelectorAll("button"))
			.find((button) => button.textContent === "Pause")
			?.click();
	});
	await settle();

	expect(mockInvoke).toHaveBeenCalledWith("bot_pause", {
		request: { botId: "bot-1", commandId: expect.any(String) },
	});
	await unmount(root, container);
});

test("refreshes persisted incomplete state after an unresolved stop", async () => {
	view = completedView();
	view.experiment.state = "preparing";
	view.report = null;
	const { container, root } = await mount();

	await act(async () => {
		Array.from(container.querySelectorAll("button"))
			.find((button) => button.textContent?.startsWith("Stop"))
			?.click();
	});
	await settle();

	expect(container.textContent).toContain("shutdown unresolved");
	await unmount(root, container);
});

test("renders the experiment setup in Simplified Chinese", async () => {
	const previousLocale = i18n.resolvedLanguage ?? "en-US";
	await act(async () => {
		await i18n.changeLanguage("zh-CN");
	});
	const { container, root } = await mount();

	expect(container.textContent).toContain("三标的模拟实验");
	expect(container.textContent).toContain("配置实验");
	expect(container.textContent).toContain("观测开始（UTC）");

	await unmount(root, container);
	await act(async () => {
		await i18n.changeLanguage(previousLocale);
	});
});
