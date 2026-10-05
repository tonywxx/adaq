/** @jest-environment jsdom */

import "@/lib/i18n";
import { AuthenticatedUserContext } from "@/authenticated-user";
import { i18n, resources } from "@/lib/i18n";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import type { ReactNode } from "react";
import { BotsPage, botDisplayName } from "./bots-page";

jest.mock("@tauri-apps/api/core", () => ({ invoke: jest.fn() }));
jest.mock("@tanstack/react-router", () => ({
	Link: ({ to, children }: { to: string; children: ReactNode }) => (
		<a href={to}>{children}</a>
	),
}));

const mockInvoke = invoke as jest.MockedFunction<typeof invoke>;

test("Bot loading paints its shell and allows navigation while every read is pending", async () => {
	mockInvoke.mockImplementation(() => new Promise(() => {}));
	const queryClient = new QueryClient({
		defaultOptions: { queries: { retry: false } },
	});
	const container = document.createElement("div");
	const root = createRoot(container);
	function Journey() {
		const [showBots, setShowBots] = useState(true);
		return (
			<>
				<button onClick={() => setShowBots(false)}>Leave page</button>
				{showBots ? <BotsPage /> : <p>Other page</p>}
			</>
		);
	}
	await act(async () =>
		root.render(
			<AuthenticatedUserContext.Provider value="alice">
				<QueryClientProvider client={queryClient}>
					<Journey />
				</QueryClientProvider>
			</AuthenticatedUserContext.Provider>,
		),
	);
	expect(container.querySelector("h1")).not.toBeNull();
	await settle();
	expect(mockInvoke).toHaveBeenCalledWith("bot_page", { page: 1 });
	await act(async () => container.querySelector("button")!.click());
	expect(container.textContent).toContain("Other page");
	expect(container.querySelector("h1")).toBeNull();
	await act(async () => root.unmount());
	queryClient.clear();
	mockInvoke.mockReset();
});

(
	globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }
).IS_REACT_ACT_ENVIRONMENT = true;

async function settle() {
	await act(async () => {
		await Promise.resolve();
		await Promise.resolve();
		await new Promise((resolve) => window.setTimeout(resolve, 120));
	});
}

test("Bot cards identify the scheduled market before their opaque ID", () => {
	expect(
		botDisplayName({ type: "ema-double-cross", instrumentId: "okx:ETH-USDT" }),
	).toBe("EMA Double-Cross · ETH-USDT");
	expect(
		botDisplayName({
			type: "scheduled-cross-section",
			universeId: "universe",
			instruments: ["okx:BTC-USDT", "okx:ETH-USDT"],
		}),
	).toBe("Cross-Section · BTC-USDT, ETH-USDT");
});

test("refreshes Bot runtime evidence every 15 seconds", async () => {
	mockInvoke.mockImplementation(async (command: string) => {
		if (
			command === "bot_page" ||
			command === "strategy_qualification_list" ||
			command === "connection_profile_list"
		)
			return command === "bot_page"
				? { items: [], page: 1, pageSize: 10, total: 0 }
				: [];
		throw new Error(`unexpected command: ${command}`);
	});
	const queryClient = new QueryClient({
		defaultOptions: { queries: { retry: false } },
	});
	const container = document.createElement("div");
	const root = createRoot(container);
	document.body.append(container);
	await act(async () => {
		root.render(
			<AuthenticatedUserContext.Provider value="alice">
				<QueryClientProvider client={queryClient}>
					<BotsPage />
				</QueryClientProvider>
			</AuthenticatedUserContext.Provider>,
		);
	});
	await settle();

	const observer = queryClient
		.getQueryCache()
		.find({ queryKey: ["bots", "alice"], exact: false })?.observers[0];
	expect(observer?.options.refetchInterval).toBe(15_000);

	await act(async () => root.unmount());
	queryClient.clear();
	container.remove();
	mockInvoke.mockReset();
});

test("Bot reconciliation status explains that Start and Retry reconcile automatically", async () => {
	const previousLanguage = i18n.language;
	await act(async () => {
		await i18n.changeLanguage("en-US");
	});
	mockInvoke.mockImplementation(async (command: string) => {
		if (command === "bot_page")
			return {
				page: 1,
				pageSize: 10,
				total: 1,
				items: [
					{
						botId: "bot-1",
						state: "faulted",
						currentAttemptId: "attempt-1",
						bundle: {
							identity: "bundle-1",
							qualificationId: "qualification-1",
							candidateId: "candidate-1",
							candidateRevision: 1,
							accountId: "demo-account",
							connectionProfileId: "demo-profile",
							schedule: { type: "ema-double-cross", instrumentId: "okx:BTC-USDT" },
						},
						attempts: [
							{
								attemptId: "attempt-1",
								state: "faulted",
								reconciliationRequired: true,
								unmanagedPositions: [],
								evidence: [],
								events: [],
								decisions: [
									{
										requestId: "request-1",
										decisionId: "decision-1",
										outcome: "no-target",
										noTargetReason: "missing-input",
										noTargetDetail: "one or more feature values are missing",
										observedAtMs: 1,
									},
								],
								orders: [],
							},
						],
						control: {
							canStart: false,
							canRetry: true,
							canPause: false,
							canResume: false,
							canStop: false,
							canFlatten: false,
						},
					},
				],
			};
		if (
			command === "strategy_qualification_list" ||
			command === "connection_profile_list"
		)
			return [];
		throw new Error(`unexpected command: ${command}`);
	});
	const queryClient = new QueryClient({
		defaultOptions: { queries: { retry: false } },
	});
	const container = document.createElement("div");
	const root = createRoot(container);
	document.body.append(container);
	await act(async () => {
		root.render(
			<AuthenticatedUserContext.Provider value="alice">
				<QueryClientProvider client={queryClient}>
					<BotsPage />
				</QueryClientProvider>
			</AuthenticatedUserContext.Provider>,
		);
	});
	await settle();

	const englishReconciliationStatus =
		container.querySelector('[role="status"]')?.textContent;
	expect(englishReconciliationStatus).toBe(
		resources["en-US"].translation.bots.reconciliationRequired,
	);
	expect(englishReconciliationStatus).toContain("automatically reconciles");
	expect(englishReconciliationStatus).toContain(
		"no manual Reconcile is needed first",
	);
	expect(container.textContent).not.toContain(
		"no-target · missing-input: one or more feature values are missing",
	);
	expect(container.querySelector('[role="alert"]')).toBeNull();
	await act(async () => {
		await i18n.changeLanguage("zh-CN");
	});
	const chineseReconciliationStatus =
		container.querySelector('[role="status"]')?.textContent;
	expect(chineseReconciliationStatus).toBe(
		resources["zh-CN"].translation.bots.reconciliationRequired,
	);
	expect(chineseReconciliationStatus).toContain("自动对账");
	expect(chineseReconciliationStatus).toContain("无需先手动对账");

	await act(async () => root.unmount());
	queryClient.clear();
	container.remove();
	await i18n.changeLanguage(previousLanguage);
	mockInvoke.mockReset();
});
