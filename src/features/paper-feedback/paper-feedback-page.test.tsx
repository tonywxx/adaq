/** @jest-environment jsdom */

import "@/lib/i18n";
import { AuthenticatedUserContext } from "@/authenticated-user";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { PaperFeedbackPage } from "./paper-feedback-page";

jest.mock("@tauri-apps/api/core", () => ({ invoke: jest.fn() }));
const mockInvoke = invoke as jest.MockedFunction<typeof invoke>;

(
	globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }
).IS_REACT_ACT_ENVIRONMENT = true;

const bot = {
	botId: "bot-a",
	currentAttemptId: "attempt-a",
	bundle: { identity: "bundle-a", accountId: "account-a" },
	attempts: [
		{
			attemptId: "attempt-a",
			state: "running",
			createdAtMs: Date.parse("2026-08-29T00:00:00Z"),
			updatedAtMs: Date.parse("2026-08-29T01:00:00Z"),
		},
	],
};

const snapshot = {
	snapshotId: "snapshot-a",
	input: {
		bundleId: "bundle-a",
		botId: "bot-a",
		attemptId: "attempt-a",
		observationStartMs: Date.parse("2026-08-29T00:00:00Z"),
		observationEndMs: Date.parse("2026-08-29T01:00:00Z"),
		realizationCutoffMs: Date.parse("2026-08-29T01:00:00Z"),
		realizedObservations: 0,
		requiredObservations: 20,
	},
	evidenceState: "notYetRealized",
	createdAtMs: Date.parse("2026-08-29T01:00:00Z"),
};

const report = {
	reportId: "report-a",
	input: {
		snapshotId: "snapshot-a",
		lens: "factor",
		metrics: {
			lensMetrics: {
				realizedFactorSamples: 2,
				factorOutputsAvailable: true,
				outputMetrics: {
					score: { samples: 2, coverage: 1, ic: 0.5, rankIc: 0.4 },
				},
			},
			evidenceReasons: ["target-horizon-not-matured"],
		},
		comparableEvidenceId: null,
	},
	evidenceState: "notYetRealized",
	createdAtMs: Date.parse("2026-08-29T01:00:00Z"),
};

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
					<PaperFeedbackPage />
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
	let view = { snapshots: [], reports: [], decisions: [] } as {
		snapshots: (typeof snapshot)[];
		reports: (typeof report)[];
		decisions: unknown[];
	};
	mockInvoke.mockImplementation(async (command: string, args?: unknown) => {
		if (command === "bot_list") return [bot];
		if (command === "paper_feedback_view") return view;
		if (command === "paper_feedback_snapshot_create") {
			view = { ...view, snapshots: [snapshot] };
			return snapshot;
		}
		if (command === "paper_feedback_report_create") {
			view = { ...view, reports: [report] };
			return report;
		}
		if (command === "paper_feedback_review_decide")
			return { decisionId: "decision-a" };
		throw new Error(`unexpected command ${command} ${JSON.stringify(args)}`);
	});
});

afterEach(() => {
	mockInvoke.mockReset();
	document.body.replaceChildren();
});

test("creates a Host-bound snapshot and exposes all four review lenses", async () => {
	const { container, root } = await mount();
	const botSelect = container.querySelector(
		"#feedback-bot",
	) as HTMLSelectElement;
	await act(async () => {
		botSelect.value = "bot-a";
		botSelect.dispatchEvent(new Event("change", { bubbles: true }));
	});
	await settle();

	await act(async () => {
		Array.from(container.querySelectorAll("button"))
			.find((button) => button.textContent === "Create immutable Snapshot")
			?.click();
	});
	await settle();

	expect(mockInvoke).toHaveBeenCalledWith("paper_feedback_snapshot_create", {
		request: {
			botId: "bot-a",
			bundleId: "bundle-a",
			attemptId: "attempt-a",
			observationStartMs: expect.any(Number),
			observationEndMs: expect.any(Number),
			realizationCutoffMs: expect.any(Number),
			requiredObservations: 20,
		},
	});
	for (const lens of ["Factor", "Model", "Strategy", "Execution"]) {
		expect(container.textContent).toContain(`Generate Report · ${lens}`);
	}

	await act(async () => {
		Array.from(container.querySelectorAll("button"))
			.find((button) => button.textContent === "Generate Report · Factor")
			?.click();
	});
	await settle();
	expect(mockInvoke).toHaveBeenCalledWith("paper_feedback_report_create", {
		request: { snapshotId: "snapshot-a", lens: "factor" },
	});
	expect(container.textContent).toContain(
		"score: samples=2 · coverage=1 · ic=0.5000 · rankIc=0.4000",
	);
	expect(container.textContent).toContain("target-horizon-not-matured");
	await unmount(root, container);
});
