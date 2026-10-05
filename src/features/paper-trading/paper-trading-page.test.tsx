/** @jest-environment jsdom */

import { i18n } from "@/lib/i18n";
import { AuthenticatedUserContext } from "@/authenticated-user";
import { abbreviateIdentifier } from "@/lib/identifier-display";
import {
	QueryClient,
	QueryClientProvider,
	QueryObserver,
} from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { PaperTradingPage } from "./paper-trading-page";
import { afterPaint } from "@/features/factors/factor-workspace-data";

jest.mock("@tauri-apps/api/core", () => ({ invoke: jest.fn() }));
jest.mock("@/features/factors/factor-workspace-data", () => ({
	afterPaint: jest.fn(),
}));
const mockInvoke = invoke as jest.MockedFunction<typeof invoke>;

(
	globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }
).IS_REACT_ACT_ENVIRONMENT = true;

const retainedView = {
	account: {
		account_id: "okx-demo-account",
		market: "okx_spot",
		currency: "usdt",
		cash: "1000",
		positions: { "BTC-USDT": { quantity: "0.1", sellable_quantity: "0.1" } },
		observed_at_ms: 1_700_000_000_000,
	},
	reservedCash: "50",
	buyingPower: "950",
	reconciliation: "required",
	restartRequired: true,
	hasUncertain: false,
	orders: [],
	fills: [],
	providerEvidence: [],
	riskDecisions: [
		{
			approved: true,
			reason: "approved",
			requestedNotional: "100",
			approvedNotional: "100",
			decidedAtMs: 1_700_000_000_000,
		},
	],
};

const retainedWorkspace = {
	account: retainedView,
	connection: { state: "degraded", evidence: null },
};

function retainedEvidence(args: unknown) {
	const { section, page } = args as { section: string; page: number };
	const items =
		section === "positions"
			? Object.entries(retainedView.account.positions).map(
					([instrument, position]) => ({ instrument, ...position }),
				)
			: section === "riskDecisions"
				? retainedView.riskDecisions
				: [];
	return { items, page, pageSize: 10, total: items.length };
}

async function settle() {
	await act(async () => {
		await Promise.resolve();
		await Promise.resolve();
		await Promise.resolve();
		await new Promise((resolve) => window.setTimeout(resolve, 0));
	});
}

function button(container: HTMLElement, label: string) {
	return Array.from(container.querySelectorAll("button")).find(
		(candidate) => candidate.textContent === label,
	);
}

async function mount(client = new QueryClient()) {
	const container = document.createElement("div");
	const root = createRoot(container);
	document.body.append(container);
	await act(async () => {
		root.render(
			<AuthenticatedUserContext.Provider value="alice">
				<QueryClientProvider client={client}>
					<PaperTradingPage />
				</QueryClientProvider>
			</AuthenticatedUserContext.Provider>,
		);
	});
	await settle();
	await settle();
	return { container, root };
}

async function unmount(root: Root, container: HTMLDivElement) {
	await act(async () => root.unmount());
	container.remove();
}

beforeEach(() => {
	jest.mocked(afterPaint).mockResolvedValue();
	mockInvoke.mockImplementation(async (command: string, args) => {
		if (command === "paper_account_view") return retainedWorkspace;
		if (command === "paper_account_evidence_page") return retainedEvidence(args);
		if (command === "paper_account_reconcile") return retainedView;
		throw new Error(`unexpected command: ${command}`);
	});
	Object.defineProperty(HTMLDialogElement.prototype, "showModal", {
		configurable: true,
		value: function (this: HTMLDialogElement) {
			this.open = true;
		},
	});
	Object.defineProperty(HTMLDialogElement.prototype, "close", {
		configurable: true,
		value: function (this: HTMLDialogElement) {
			this.open = false;
		},
	});
});

afterEach(() => {
	jest.mocked(afterPaint).mockReset();
	mockInvoke.mockReset();
	document.body.replaceChildren();
});

test("keeps retained evidence visible until a confirmed reconcile succeeds", async () => {
	const { container, root } = await mount();

	expect(container.textContent).toContain("Paper Trading Workspace");
	expect(container.textContent).toContain(
		abbreviateIdentifier("okx-demo-account"),
	);
	expect(container.textContent).toContain("approved");
	expect(container.textContent).toContain("Retained connection is degraded.");
	expect(mockInvoke).toHaveBeenCalledWith("paper_account_view");
	expect(mockInvoke).not.toHaveBeenCalledWith("paper_account_reconcile");

	await act(async () => button(container, "Reconcile")?.click());
	expect(mockInvoke).not.toHaveBeenCalledWith("paper_account_reconcile");
	await act(async () => button(container, "Confirm Reconcile")?.click());
	await settle();

	expect(mockInvoke).toHaveBeenCalledWith("paper_account_reconcile");
	await unmount(root, container);
});

test("keeps the retained view when Reconcile fails", async () => {
	mockInvoke.mockImplementation(async (command: string, args) => {
		if (command === "paper_account_view") return retainedWorkspace;
		if (command === "paper_account_evidence_page") return retainedEvidence(args);
		if (command === "paper_account_reconcile")
			throw JSON.stringify({
				code: "connectionUnavailable",
				message: "The OKX Demo connection is unavailable.",
			});
		throw new Error(`unexpected command: ${command}`);
	});
	const { container, root } = await mount();

	await act(async () => button(container, "Reconcile")?.click());
	await act(async () => button(container, "Confirm Reconcile")?.click());
	await settle();

	expect(container.textContent).toContain(
		abbreviateIdentifier("okx-demo-account"),
	);
	expect(container.textContent).toContain(
		"Reconcile cannot start because the OKX Demo connection is unavailable.",
	);
	expect(container.textContent).toContain("Retained evidence has not changed.");
	await unmount(root, container);
});

test("refreshes retained operational alerts after a successful reconcile", async () => {
	const client = new QueryClient();
	const key = ["operations-alerts", "alice"];
	client.setQueryData(key, [{ state: "active" }]);
	client.setQueryData(["operations-alerts", "bob"], [{ state: "active" }]);
	const observer = new QueryObserver(client, {
		queryKey: key,
		staleTime: Infinity,
		queryFn: async () => [{ state: "resolved" }],
	});
	const unsubscribe = observer.subscribe(() => {});
	const { container, root } = await mount(client);
	expect(client.getQueryData(key)).toEqual([{ state: "active" }]);

	await act(async () => button(container, "Reconcile")?.click());
	expect(client.getQueryData(key)).toEqual([{ state: "active" }]);
	await act(async () => button(container, "Confirm Reconcile")?.click());
	await settle();

	expect(client.getQueryData(key)).toEqual([{ state: "resolved" }]);
	expect(client.getQueryData(["operations-alerts", "bob"])).toEqual([
		{ state: "active" },
	]);
	unsubscribe();
	await unmount(root, container);
});

test("clarifies that Bot Start and Retry reconcile without a prior manual click", async () => {
	const previousLanguage = i18n.language;
	await act(async () => {
		await i18n.changeLanguage("en-US");
	});
	const { container, root } = await mount();
	const englishCopy = container.textContent ?? "";
	expect(englishCopy).toContain(
		"Bot Start/Retry automatically reconciles before enabling risk",
	);
	expect(englishCopy).toContain("no manual Reconcile is needed first.");
	expect(englishCopy).toContain(i18n.t("paperTrading.restartRequired"));

	await act(async () => {
		await i18n.changeLanguage("zh-CN");
	});
	expect(container.textContent).toContain(
		"启动或重试 Bot 时，Host 会先自动对账 OKX Demo 账户再启用风险",
	);
	expect(container.textContent).toContain("无需先手动点击“对账”。");
	expect(container.textContent).toContain(
		i18n.t("paperTrading.restartRequired"),
	);

	await unmount(root, container);
	await i18n.changeLanguage(previousLanguage);
});

function installLargeWorkspace(total = () => 23) {
	mockInvoke.mockImplementation(async (command: string, args) => {
		if (command === "paper_account_view") return retainedWorkspace;
		if (command === "paper_account_evidence_page") {
			const { section, page: requestedPage } = args as {
				section: string;
				page: number;
			};
			const count = total();
			const page = Math.min(requestedPage, Math.max(1, Math.ceil(count / 10)));
			return {
				page,
				pageSize: 10,
				total: count,
				items: Array.from(
					{ length: Math.max(0, Math.min(10, count - (page - 1) * 10)) },
					(_, offset) => {
						const id = (page - 1) * 10 + offset;
						return {
							instrument: `COIN-${id}-USDT`,
							quantity: "1",
							sellable_quantity: "1",
							order_id: `order-${id}`,
							side: "Buy",
							filled_quantity: "0",
							status: "accepted",
							fill_id: `fill-${id}`,
							price: "1",
							fee: "0",
							approved: true,
							reason: `risk-${id}`,
							requestedNotional: "1",
							approvedNotional: "1",
							decidedAtMs: id,
							evidence: `${section}-${id}`,
						};
					},
				),
			};
		}
		throw new Error(`unexpected command: ${command}`);
	});
}

function evidenceCard(container: HTMLElement, section: string) {
	const card = container.querySelector<HTMLElement>(
		`[data-evidence-section="${section}"]`,
	);
	if (!card) throw new Error(`missing ${section} card`);
	return card;
}

test("pages all five evidence cards independently with ten rows and localized controls", async () => {
	installLargeWorkspace();
	const { container, root } = await mount();
	const sections = [
		"positions",
		"orders",
		"fills",
		"riskDecisions",
		"providerEvidence",
	];
	for (const section of sections) {
		const card = evidenceCard(container, section);
		expect(card.querySelectorAll("ul.grid > li")).toHaveLength(10);
		expect(card.textContent).toContain("Page 1 of 3");
	}
	const positions = evidenceCard(container, "positions");
	const initialCalls = mockInvoke.mock.calls.length;
	await act(async () => button(positions, "Next")?.click());
	await settle();
	expect(mockInvoke.mock.calls.slice(initialCalls)).toEqual([
		["paper_account_evidence_page", { section: "positions", page: 2 }],
	]);
	expect(positions.textContent).toContain("COIN-10-USDT");
	expect(positions.textContent).not.toContain("COIN-0-USDT");
	for (const section of sections.slice(1)) {
		expect(evidenceCard(container, section).textContent).toContain("Page 1 of 3");
	}
	await act(async () => button(positions, "Previous")?.click());
	await settle();
	expect(positions.textContent).toContain("COIN-0-USDT");
	const previousLanguage = i18n.language;
	await act(async () => {
		await i18n.changeLanguage("zh-CN");
	});
	expect(button(positions, "下一页")).toBeDefined();
	expect(positions.querySelector("nav")?.getAttribute("aria-label")).toContain(
		"分页",
	);
	await unmount(root, container);
	await i18n.changeLanguage(previousLanguage);
});

test("refresh retains the current page and clamps it when evidence shrinks", async () => {
	let count = 23;
	installLargeWorkspace(() => count);
	const { container, root } = await mount();
	const positions = evidenceCard(container, "positions");
	await act(async () => button(positions, "Next")?.click());
	await settle();
	await act(async () => button(container, "Refresh")?.click());
	await settle();
	expect(positions.textContent).toContain("Page 2 of 3");
	count = 7;
	await act(async () => button(container, "Refresh")?.click());
	await settle();
	expect(positions.textContent).toContain("Page 1 of 1");
	expect(positions.querySelectorAll("ul.grid > li")).toHaveLength(7);
	expect(button(positions, "Previous")?.disabled).toBe(true);
	expect(button(positions, "Next")?.disabled).toBe(true);
	await unmount(root, container);
});

test("keeps existing evidence and other controls usable while a page is slow or fails", async () => {
	installLargeWorkspace();
	const read = mockInvoke.getMockImplementation();
	let failPage: (error: Error) => void = () => {};
	mockInvoke.mockImplementation(async (command, args) => {
		if (
			command === "paper_account_evidence_page" &&
			(args as { section: string; page: number }).section === "positions" &&
			(args as { page: number }).page === 2
		) {
			return new Promise((_resolve, reject) => {
				failPage = reject;
			});
		}
		return read?.(command, args);
	});
	const { container, root } = await mount();
	const positions = evidenceCard(container, "positions");
	await act(async () => button(positions, "Next")?.click());
	await settle();
	expect(positions.textContent).toContain("COIN-0-USDT");
	expect(positions.querySelector('[aria-busy="true"]')).not.toBeNull();
	expect(button(evidenceCard(container, "orders"), "Next")?.disabled).toBe(
		false,
	);
	await act(async () => button(container, "Reconcile")?.click());
	expect(container.querySelector("dialog")?.open).toBe(true);
	expect(mockInvoke).not.toHaveBeenCalledWith("paper_account_reconcile");
	await act(async () => failPage(new Error("read failed")));
	await settle();
	expect(positions.textContent).toContain("COIN-0-USDT");
	expect(positions.querySelector('[role="alert"]')).not.toBeNull();
	await unmount(root, container);
});

test("paints the route before entry IPC and keeps off-page uncertainty visible", async () => {
	let finishPaint: () => void = () => {};
	jest.mocked(afterPaint).mockReturnValueOnce(
		new Promise((resolve) => {
			finishPaint = resolve;
		}),
	);
	mockInvoke.mockImplementation(async (command, args) => {
		if (command === "paper_account_view")
			return {
				...retainedWorkspace,
				account: { ...retainedView, hasUncertain: true },
			};
		if (command === "paper_account_evidence_page") return retainedEvidence(args);
		throw new Error(`unexpected command: ${command}`);
	});
	const { container, root } = await mount();
	expect(container.textContent).toContain("Paper Trading Workspace");
	expect(mockInvoke).not.toHaveBeenCalled();
	await act(async () => finishPaint());
	await settle();
	expect(container.textContent).toContain(i18n.t("paperTrading.uncertain"));
	await unmount(root, container);
});
