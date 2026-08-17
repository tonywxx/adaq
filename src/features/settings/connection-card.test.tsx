/** @jest-environment jsdom */

import "@/lib/i18n";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import type { ComponentProps } from "react";
import { i18n } from "@/lib/i18n";
import { ConnectionCard } from "./connection-card";
import type { ConnectionsAdapter, ProfileView } from "./connections-adapter";

(
	globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }
).IS_REACT_ACT_ENVIRONMENT = true;

const profile: ProfileView = {
	profileId: "profile-1",
	provider: "alpaca_paper",
	environment: "paper",
	maskedKeySuffix: "1234",
	accountId: "account-1",
	currency: "USD",
	status: "usable",
	lastTestAtMs: null,
	lastTestEvidence: null,
	createdAtMs: 1,
	updatedAtMs: 1,
};

function makeAdapter(
	overrides: Partial<ConnectionsAdapter> = {},
): ConnectionsAdapter {
	return {
		listProfiles: jest.fn().mockResolvedValue([]),
		saveProfile: jest.fn().mockResolvedValue(profile),
		testProfile: jest.fn().mockResolvedValue(profile),
		deleteProfile: jest.fn().mockResolvedValue(undefined),
		...overrides,
	};
}

async function mount(
	adapter: ConnectionsAdapter,
	props: Partial<ComponentProps<typeof ConnectionCard>> = {},
) {
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	await act(async () => {
		root.render(
			<ConnectionCard
				provider="alpaca_paper"
				profile={null}
				disabled={false}
				userId="user-1"
				adapter={adapter}
				onChanged={jest.fn()}
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

function buttonWithText(container: HTMLDivElement, text: string) {
	return Array.from(container.querySelectorAll("button")).find((button) =>
		button.textContent?.includes(text),
	);
}

afterEach(() => {
	document.body.replaceChildren();
	jest.restoreAllMocks();
});

test("saves credentials through the adapter and clears the form", async () => {
	const saveProfile = jest.fn().mockResolvedValue(profile);
	const onChanged = jest.fn();
	const adapter = makeAdapter({ saveProfile });
	const mounted = await mount(adapter, { onChanged });
	const inputs = mounted.container.querySelectorAll("input");
	if (inputs.length < 2) throw new Error("credential inputs not found");

	await act(async () => {
		setInputValue(inputs[0], "demo-key");
		setInputValue(inputs[1], "demo-secret");
	});
	const save = buttonWithText(
		mounted.container,
		i18n.t("settings.connections.save"),
	);
	if (!save) throw new Error("save button not found");
	await act(async () => save.click());
	await settle();

	expect(saveProfile).toHaveBeenCalledWith("user-1", {
		provider: "alpaca_paper",
		keyId: "demo-key",
		secretKey: "demo-secret",
	});
	expect(onChanged).toHaveBeenCalled();
	expect((inputs[0] as HTMLInputElement).value).toBe("");
	expect((inputs[1] as HTMLInputElement).value).toBe("");
	await unmount(mounted.root, mounted.container);
});

test("requires the OKX passphrase before saving", async () => {
	const saveProfile = jest.fn().mockResolvedValue(profile);
	const adapter = makeAdapter({ saveProfile });
	const mounted = await mount(adapter, { provider: "okx_demo" });
	const inputs = mounted.container.querySelectorAll("input");
	if (inputs.length < 3) throw new Error("OKX credential inputs not found");

	await act(async () => {
		setInputValue(inputs[0], "demo-api-key");
		setInputValue(inputs[1], "demo-secret");
	});
	const save = buttonWithText(
		mounted.container,
		i18n.t("settings.connections.save"),
	);
	if (!save) throw new Error("save button not found");
	expect((save as HTMLButtonElement).disabled).toBe(true);

	await act(async () => {
		setInputValue(inputs[2], "demo-passphrase");
	});
	expect((save as HTMLButtonElement).disabled).toBe(false);

	await act(async () => save.click());
	await settle();
	expect(saveProfile).toHaveBeenCalledWith("user-1", {
		provider: "okx_demo",
		apiKey: "demo-api-key",
		secretKey: "demo-secret",
		passphrase: "demo-passphrase",
	});
	await unmount(mounted.root, mounted.container);
});

test("requires confirmation for delete and does not expose profile secrets", async () => {
	const deleteProfile = jest.fn().mockResolvedValue(undefined);
	const adapter = makeAdapter({ deleteProfile });
	const mounted = await mount(adapter, { profile });
	await settle();

	expect(mounted.container.textContent).toContain("1234");
	expect(mounted.container.textContent).not.toContain("demo-secret");
	const confirm = jest.spyOn(window, "confirm").mockReturnValue(false);
	const remove = buttonWithText(
		mounted.container,
		i18n.t("settings.connections.delete"),
	);
	if (!remove) throw new Error("delete button not found");
	await act(async () => remove.click());

	expect(confirm).toHaveBeenCalled();
	expect(deleteProfile).not.toHaveBeenCalled();
	await unmount(mounted.root, mounted.container);
});

test("renders a typed adapter failure without leaking the credential", async () => {
	const saveProfile = jest
		.fn()
		.mockRejectedValue(
			JSON.stringify({ code: "auth_failed", message: "redacted failure" }),
		);
	const adapter = makeAdapter({ saveProfile });
	const mounted = await mount(adapter);
	const inputs = mounted.container.querySelectorAll("input");
	if (inputs.length < 2) throw new Error("credential inputs not found");

	await act(async () => {
		setInputValue(inputs[0], "demo-key");
		setInputValue(inputs[1], "demo-secret");
	});
	const save = buttonWithText(
		mounted.container,
		i18n.t("settings.connections.save"),
	);
	if (!save) throw new Error("save button not found");
	await act(async () => save.click());
	await settle();

	const alert = mounted.container.querySelector('[role="alert"]');
	expect(alert).not.toBeNull();
	expect(alert?.textContent).not.toContain("demo-secret");
	await unmount(mounted.root, mounted.container);
});
