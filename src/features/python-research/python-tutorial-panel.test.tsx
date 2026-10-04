/** @jest-environment jsdom */

import "@/lib/i18n";
import { invoke } from "@tauri-apps/api/core";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { PythonProjectsPanel } from "./python-projects-panel";
import { PythonTutorialPanel } from "./python-tutorial-panel";

jest.mock("@tauri-apps/api/core", () => ({ invoke: jest.fn() }));
jest.mock("@tauri-apps/plugin-opener", () => ({ openPath: jest.fn() }));
jest.mock("@tauri-apps/plugin-fs", () => ({
	readFile: jest.fn(),
	writeFile: jest.fn(),
}));
jest.mock("@tauri-apps/plugin-dialog", () => ({
	open: jest.fn(),
	save: jest.fn(),
}));
jest.mock("@/lib/http", () => ({ isTauriRuntime: () => true }));
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

test("preparing a tutorial project refreshes the shared card before Trust review", async () => {
	let revisionSha256 = "revision-old";
	let prepared = false;
	const projectId = "py-model-qlib-ridge-return";
	const invokeMock = invoke as jest.Mock;
	invokeMock.mockImplementation(
		async (command: string, args?: { request?: { revisionSha256?: string } }) => {
			switch (command) {
				case "project_list":
					return [
						{ projectId, revisionSha256, path: "/model", state: "clean", issues: [] },
					];
				case "runtime_profile":
					return { status: "ready", wheelhouseStatus: "ready" };
				case "attempt_list":
					return [];
				case "project_validate":
				case "environment_sync_managed":
				case "trust_revision":
					return {};
				case "project_freeze":
					revisionSha256 = "revision-new";
					return { revisionSha256 };
				case "environment_prepare_managed":
					prepared = true;
					return { environmentSha256: "environment-new" };
				case "environment_for_project":
					return prepared ? { environmentSha256: "environment-new" } : null;
				case "attempt_preview":
					return {
						projectId,
						revisionSha256: args?.request?.revisionSha256,
						entryPoint: "project:create_project",
						sourceFiles: {},
						lock: {
							lockSha256: "lock",
							runtimeArtifactSha256: "runtime",
							wheelhouseIdentity: "wheelhouse",
							platform: "macos-aarch64",
							wheels: [],
						},
						environmentSha256: "environment-new",
						runtime: {
							profile: "adaq-python@1",
							version: "3.12.13",
							platform: "macos-aarch64",
							artifactSha256: "runtime",
							source: "https://example.invalid/python",
							signature: "signature",
						},
						sdkArtifactSha256: "sdk",
						trustedCodeWarning: "Trusted code will execute",
						inputBindings: {},
						normalizedParameters: { alpha: "1" },
						resourcePolicy: {},
						seed: 0,
					};
				default:
					throw new Error(`Unexpected command: ${command}`);
			}
		},
	);
	Object.defineProperty(globalThis, "requestAnimationFrame", {
		configurable: true,
		value: (callback: FrameRequestCallback) => {
			callback(0);
			return 0;
		},
	});
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	const settle = () =>
		act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});
	try {
		await act(async () =>
			root.render(
				<>
					<PythonTutorialPanel userId="user-1" />
					<div data-testid="cards">
						<PythonProjectsPanel userId="user-1" kind="model" />
					</div>
				</>,
			),
		);
		await settle();
		const cards = container.querySelector<HTMLDivElement>(
			'[data-testid="cards"]',
		);
		expect(cards?.textContent).toContain("revision-old");
		const prepare = [...container.querySelectorAll("button")].find(
			(button) => button.textContent === "Validate, sync, and prepare",
		);
		expect(prepare).toBeDefined();
		await act(async () => prepare?.click());
		await settle();
		expect(cards?.textContent).toContain("revision-new");
		expect(cards?.textContent).not.toContain("revision-old");
		invokeMock.mockClear();
		const review = [...(cards?.querySelectorAll("button") ?? [])].find(
			(button) => button.textContent === "Review Model Trust",
		);
		expect(review).toBeDefined();
		await act(async () => review?.click());
		await settle();
		expect(invokeMock).toHaveBeenCalledWith("attempt_preview", {
			request: {
				userId: "user-1",
				projectId,
				revisionSha256: "revision-new",
				seed: 0,
			},
		});
		expect(invokeMock).not.toHaveBeenCalledWith(
			"trust_revision",
			expect.anything(),
		);
		expect(invokeMock).not.toHaveBeenCalledWith(
			"attempt_start",
			expect.anything(),
		);
		const confirm = [...container.querySelectorAll("button")].find(
			(button) => button.textContent === "Trust Revision",
		);
		expect(confirm).toBeDefined();
		await act(async () => confirm?.click());
		await settle();
		expect(invokeMock).toHaveBeenCalledWith("trust_revision", {
			request: { userId: "user-1", projectId, revisionSha256: "revision-new" },
		});
		expect(invokeMock).not.toHaveBeenCalledWith(
			"attempt_start",
			expect.anything(),
		);
	} finally {
		await act(async () => root.unmount());
		container.remove();
	}
});
