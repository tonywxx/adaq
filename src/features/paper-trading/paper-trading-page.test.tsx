/** @jest-environment jsdom */

import "@/lib/i18n";
import { AuthenticatedUserContext } from "@/authenticated-user";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { PaperTradingPage } from "./paper-trading-page";

jest.mock("@tauri-apps/api/core", () => ({ invoke: jest.fn() }));
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

async function mount() {
	const container = document.createElement("div");
	const root = createRoot(container);
	document.body.append(container);
	await act(async () => {
		root.render(
			<AuthenticatedUserContext.Provider value="alice">
				<QueryClientProvider client={new QueryClient()}>
					<PaperTradingPage />
				</QueryClientProvider>
			</AuthenticatedUserContext.Provider>,
		);
	});
	await settle();
	return { container, root };
}

async function unmount(root: Root, container: HTMLDivElement) {
	await act(async () => root.unmount());
	container.remove();
}

beforeEach(() => {
	mockInvoke.mockImplementation(async (command: string) => {
		if (command === "paper_account_view") return retainedWorkspace;
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
	mockInvoke.mockReset();
	document.body.replaceChildren();
});

test("keeps retained evidence visible until a confirmed reconcile succeeds", async () => {
	const { container, root } = await mount();

	expect(container.textContent).toContain("Paper Trading Workspace");
	expect(container.textContent).toContain("okx-demo-account");
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
	mockInvoke.mockImplementation(async (command: string) => {
		if (command === "paper_account_view") return retainedWorkspace;
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

	expect(container.textContent).toContain("okx-demo-account");
	expect(container.textContent).toContain(
		"Reconcile cannot start because the OKX Demo connection is unavailable.",
	);
	expect(container.textContent).toContain("Retained evidence has not changed.");
	await unmount(root, container);
});
