/** @jest-environment jsdom */

import "@/lib/i18n";
import { AuthenticatedUserContext } from "@/authenticated-user";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { act, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { PaperFeedbackPage } from "./paper-feedback-page";
import { i18n } from "@/lib/i18n";

jest.mock("@tauri-apps/api/core", () => ({ invoke: jest.fn() }));
jest.mock("@tanstack/react-router", () => ({
	Link: ({ to, children }: { to: string; children: ReactNode }) => (
		<a href={to}>{children}</a>
	),
}));
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
			createdAtMs: Date.parse("2026-08-29T00:00:12Z"),
			updatedAtMs: Date.parse("2026-08-29T01:00:45Z"),
		},
	],
};

const snapshot = {
	snapshotId: "snapshot-a",
	existingLenses: [] as string[],
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
		await new Promise((resolve) => window.setTimeout(resolve, 120));
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
		if (command === "bot_page")
			return { items: [bot], page: 1, pageSize: 10, total: 1 };
		if (command === "paper_feedback_page") {
			const section = (args as { section: keyof typeof view }).section;
			return {
				items: view[section],
				page: 1,
				pageSize: 10,
				total: view[section].length,
			};
		}
		if (command === "paper_feedback_snapshot_create") {
			view = { ...view, snapshots: [snapshot] };
			return snapshot;
		}
		if (command === "paper_feedback_report_create") {
			view = {
				...view,
				reports: [report],
				snapshots: view.snapshots.map((item) => ({
					...item,
					existingLenses: ["factor"],
				})),
			};
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
	const snapshotRequest = mockInvoke.mock.calls.find(
		([command]) => command === "paper_feedback_snapshot_create",
	)?.[1] as { request: Record<string, number> };
	expect(snapshotRequest.request.observationStartMs).toBeGreaterThanOrEqual(
		bot.attempts[0].createdAtMs,
	);
	expect(snapshotRequest.request.observationEndMs).toBeLessThanOrEqual(
		bot.attempts[0].updatedAtMs,
	);
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

test("report pages retain selections and use complete Snapshot lens availability", async () => {
	const reports = Array.from({ length: 23 }, (_, index) => ({
		...report,
		reportId: `report-${index}`,
	}));
	mockInvoke.mockImplementation(async (command: string, args?: unknown) => {
		if (command === "bot_page")
			return { items: [], total: 0, page: 1, pageSize: 10 };
		if (command === "paper_feedback_page") {
			const request = args as { page: number; section: string };
			const page = request.page;
			const items =
				request.section === "reports"
					? reports
					: request.section === "snapshots"
						? [{ ...snapshot, existingLenses: ["factor"] }]
						: [];
			return {
				items: items.slice((page - 1) * 10, page * 10),
				total: items.length,
				page,
				pageSize: 10,
			};
		}
		if (command === "paper_feedback_review_decide")
			return { decisionId: "decision-a" };
		throw new Error(`unexpected command ${command}`);
	});
	const { container, root } = await mount();
	expect(container.querySelectorAll('input[type="checkbox"]')).toHaveLength(10);
	const factor = Array.from(container.querySelectorAll("button")).find(
		(button) => button.textContent === i18n.t("paperFeedback.lenses.factor"),
	);
	expect(factor?.disabled).toBe(true);
	await act(async () =>
		(
			container.querySelector('input[type="checkbox"]') as HTMLInputElement
		).click(),
	);
	const pagerLabel = i18n.t("paperTrading.paginationLabel", {
		section: i18n.t("paperFeedback.reports"),
	});
	const pager = Array.from(container.querySelectorAll("nav")).find(
		(nav) => nav.getAttribute("aria-label") === pagerLabel,
	)!;
	await act(async () =>
		(pager.querySelectorAll("button")[1] as HTMLButtonElement).click(),
	);
	await settle();
	expect(container.querySelectorAll('input[type="checkbox"]')).toHaveLength(10);
	await act(async () =>
		(
			container.querySelector('input[type="checkbox"]') as HTMLInputElement
		).click(),
	);
	await act(async () =>
		(pager.querySelectorAll("button")[0] as HTMLButtonElement).click(),
	);
	await settle();
	expect(
		(container.querySelector('input[type="checkbox"]') as HTMLInputElement)
			.checked,
	).toBe(true);
	const rationale = container.querySelector(
		"#feedback-rationale",
	) as HTMLTextAreaElement;
	await act(async () => {
		Object.getOwnPropertyDescriptor(
			HTMLTextAreaElement.prototype,
			"value",
		)!.set!.call(rationale, "Review both pages");
		rationale.dispatchEvent(new Event("input", { bubbles: true }));
	});
	await act(async () =>
		Array.from(container.querySelectorAll("button"))
			.find(
				(button) => button.textContent === i18n.t("paperFeedback.submitDecision"),
			)!
			.click(),
	);
	await settle();
	expect(mockInvoke).toHaveBeenCalledWith("paper_feedback_review_decide", {
		request: {
			reportIds: ["report-0", "report-10"],
			action: "noChange",
			rationale: "Review both pages",
		},
	});
	await unmount(root, container);
});
