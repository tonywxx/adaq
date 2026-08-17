/** @jest-environment jsdom */

import "@/lib/i18n";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import type { ComponentProps } from "react";
import { toast } from "sonner";
import { ResetAction, type LocalDataSummary } from "./reset-action";
import type { SettingsActions } from "./settings-actions";

jest.mock("sonner", () => ({
	toast: { error: jest.fn(), success: jest.fn() },
}));

(
	globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }
).IS_REACT_ACT_ENVIRONMENT = true;

const summary: LocalDataSummary = {
	dataDirectory: "/tmp/adaq",
	databaseBytes: 10,
	componentBytes: 20,
	marketDataBytes: 30,
	watchlistCount: 1,
	componentCount: 2,
	snapshotCount: 3,
	runCount: 4,
	protocolCount: 5,
	reportCount: 6,
	generationAttemptCount: 7,
	modelArtifactCount: 8,
	signalDatasetCount: 9,
	componentBlockingRunCount: 0,
	marketDataBlockingRecordCount: 0,
};

function makeActions(
	overrides: Partial<SettingsActions> = {},
): SettingsActions {
	return {
		resetLocalData: jest.fn().mockResolvedValue(undefined),
		resetFactorResearch: jest.fn().mockResolvedValue(undefined),
		getLocalDataSummary: jest.fn().mockResolvedValue(summary),
		...overrides,
	};
}

async function mount(
	actions: SettingsActions,
	props: Partial<ComponentProps<typeof ResetAction>> = {},
) {
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	await act(async () => {
		root.render(
			<ResetAction
				kind="watchlist"
				titleKey="settings.dataStorage.resetWatchlist"
				descriptionKey="settings.dataStorage.resetWatchlistDescription"
				summary={summary}
				userId="user-1"
				actions={actions}
				{...props}
			/>,
		);
	});
	return { container, root };
}

async function settle() {
	await act(async () => {
		await Promise.resolve();
		await Promise.resolve();
	});
}

async function unmount(root: Root, container: HTMLDivElement) {
	await act(async () => root.unmount());
	container.remove();
}

function setInputValue(input: HTMLInputElement, value: string) {
	const setter = Object.getOwnPropertyDescriptor(
		HTMLInputElement.prototype,
		"value",
	)?.set;
	if (!setter) throw new Error("input value setter not found");
	setter.call(input, value);
	input.dispatchEvent(new Event("input", { bubbles: true }));
}

function confirmButton(container: HTMLDivElement) {
	return Array.from(container.querySelectorAll("button")).at(-1);
}

beforeEach(() => {
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
	document.body.replaceChildren();
	jest.restoreAllMocks();
});

test("blocks component reset when an active run references components", async () => {
	const actions = makeActions();
	const mounted = await mount(actions, {
		kind: "components",
		summary: { ...summary, componentBlockingRunCount: 1 },
	});
	await settle();

	const confirm = confirmButton(mounted.container);
	if (!confirm) throw new Error("component reset button not found");
	confirm.click();
	await settle();

	expect(actions.resetLocalData).not.toHaveBeenCalled();
	await unmount(mounted.root, mounted.container);
});

test("requires exact confirmation before resetting all local data", async () => {
	const resetLocalData = jest.fn().mockResolvedValue(undefined);
	const actions = makeActions({ resetLocalData });
	const reload = jest
		.spyOn(window, "setTimeout")
		.mockReturnValue(0 as unknown as ReturnType<typeof window.setTimeout>);
	const mounted = await mount(actions, { kind: "all" });
	await settle();

	const input = mounted.container.querySelector("input");
	if (!input) throw new Error("confirmation input not found");
	const confirm = confirmButton(mounted.container);
	if (!confirm) throw new Error("all reset button not found");
	if (!(input instanceof HTMLInputElement)) throw new Error("not an input");

	await act(async () => {
		setInputValue(input, "WRONG");
	});
	expect((confirm as HTMLButtonElement).disabled).toBe(true);

	await act(async () => {
		setInputValue(input, "RESET");
	});
	expect((confirm as HTMLButtonElement).disabled).toBe(false);
	await act(async () => confirm.click());
	await settle();

	expect(resetLocalData).toHaveBeenCalledWith("user-1", "all");
	expect(reload).toHaveBeenCalledWith(expect.any(Function), 500);
	await unmount(mounted.root, mounted.container);
});

test("reports reset failures and clears running state", async () => {
	const resetFactorResearch = jest
		.fn()
		.mockRejectedValue(new Error("reset failed"));
	const actions = makeActions({ resetFactorResearch });
	const mounted = await mount(actions, { kind: "factorResearch" });
	await settle();

	const input = mounted.container.querySelector("input");
	if (!(input instanceof HTMLInputElement))
		throw new Error("confirmation input not found");
	await act(async () => {
		setInputValue(input, "RESET FACTOR RESEARCH");
	});
	const confirm = confirmButton(mounted.container);
	if (!confirm) throw new Error("factor reset button not found");
	await act(async () => confirm.click());
	await settle();

	expect(resetFactorResearch).toHaveBeenCalled();
	expect((confirm as HTMLButtonElement).disabled).toBe(false);
	expect(toast.error).toHaveBeenCalledWith("Error: reset failed");
	await unmount(mounted.root, mounted.container);
});
