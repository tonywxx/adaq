/** @jest-environment jsdom */

import "@/lib/i18n";
import { AuthenticatedUserContext } from "@/authenticated-user";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { act } from "react";
import { createRoot } from "react-dom/client";
import type { ReactNode } from "react";
import {
	OperationsDashboard,
	SystemDashboard,
	type SystemDashboardProjection,
} from "./operations-dashboard";
import { WorkflowHomePage } from "@/features/workflow/workflow-page";

jest.mock("@tauri-apps/api/core", () => ({ invoke: jest.fn() }));
jest.mock("@tanstack/react-router", () => ({
	Link: ({ to, children }: { to: string; children: ReactNode }) => (
		<a href={to}>{children}</a>
	),
}));
const mockInvoke = invoke as jest.MockedFunction<typeof invoke>;
let historyCalls = 0;

(
	globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }
).IS_REACT_ACT_ENVIRONMENT = true;

const criticalAlert = {
	alertId: "alert-critical",
	entityId: "bot-a",
	dimension: "worker",
	condition: "worker_fault",
	policyId: "adaq:operations-policy@1",
	severity: "critical",
	state: "active",
	safetyAction: "faultAndReconcile",
	firstEventId: "event-first",
	firstObservedAtMs: 1,
	occurrenceCount: 2,
	lastObservedAtMs: 2,
	lastEventId: "event-last",
	evidenceId: "bot-a",
	diagnostic: "worker fault",
};

const warningAlert = {
	...criticalAlert,
	alertId: "alert-warning",
	entityId: "paper-risk",
	dimension: "riskOms",
	condition: "latency_excursion",
	severity: "warning",
	safetyAction: "none",
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
					<OperationsDashboard />
				</QueryClientProvider>
			</AuthenticatedUserContext.Provider>,
		);
	});
	await settle();
	return { container, root };
}

beforeEach(() => {
	historyCalls = 0;
	mockInvoke.mockImplementation(async (command: string, args?: unknown) => {
		if (command === "operations_health")
			return [
				{
					entityId: "bot-a",
					dimension: "worker",
					state: "critical",
					required: true,
					observedAtMs: 2,
					eventId: "event-last",
					condition: "worker_fault",
				},
			];
		if (command === "operations_alerts") return [criticalAlert, warningAlert];
		if (command === "operations_events") return [];
		if (command === "operations_probe") return null;
		if (command === "operations_alert_history") {
			historyCalls += 1;
			const active = {
				lifecycleId: "lifecycle-active",
				state: "active",
				eventId: "event-first",
				occurredAtMs: 1,
				actor: "host",
			};
			const acknowledged = {
				lifecycleId: "lifecycle-acknowledged",
				state: "acknowledged",
				eventId: "event-acknowledged",
				occurredAtMs: 2,
				actor: "alice",
			};
			return historyCalls === 1 ? [active] : [active, acknowledged];
		}
		if (command === "operations_alert_acknowledge") return undefined;
		if (command === "operations_freeze_all") return { alertId: "alert-critical" };
		throw new Error(`unexpected command ${command} ${JSON.stringify(args)}`);
	});
});

afterEach(() => {
	mockInvoke.mockReset();
	document.body.replaceChildren();
});

test("filters retained alerts and routes controls through authenticated Host commands", async () => {
	const { container, root } = await mount();
	expect(container.textContent).toContain("worker_fault");
	expect(container.textContent).toContain("latency_excursion");

	const severity = container.querySelectorAll("select")[1] as HTMLSelectElement;
	await act(async () => {
		severity.value = "critical";
		severity.dispatchEvent(new Event("change", { bubbles: true }));
	});
	await settle();
	expect(container.textContent).toContain("worker_fault");
	expect(container.textContent).not.toContain("latency_excursion");

	await act(async () => {
		Array.from(container.querySelectorAll("button"))
			.find((button) => button.textContent === "Acknowledge")
			?.click();
	});
	await settle();
	expect(mockInvoke).toHaveBeenCalledWith("operations_alert_acknowledge", {
		alertId: "alert-critical",
	});

	const confirm = jest.spyOn(window, "confirm").mockReturnValue(true);
	await act(async () => {
		Array.from(container.querySelectorAll("button"))
			.find((button) => button.textContent === "Freeze all")
			?.click();
	});
	await settle();
	expect(confirm).toHaveBeenCalled();
	expect(mockInvoke).toHaveBeenCalledWith("operations_freeze_all");
	confirm.mockRestore();

	await act(async () => root.unmount());
	container.remove();
});

test("refreshes expanded lifecycle history after acknowledgement", async () => {
	const { container, root } = await mount();
	const details = container.querySelector("details") as HTMLDetailsElement;
	details.open = true;
	await act(async () => {
		details.dispatchEvent(new Event("toggle", { bubbles: true }));
	});
	await settle();
	expect(container.textContent).toContain("Active · host · event-first");

	await act(async () => {
		Array.from(container.querySelectorAll("button"))
			.find((button) => button.textContent === "Acknowledge")
			?.click();
	});
	await settle();

	expect(historyCalls).toBeGreaterThan(1);
	expect(container.textContent).toContain(
		"Acknowledged · alice · event-acknowledged",
	);

	await act(async () => root.unmount());
	container.remove();
});

const responsibleProjection: SystemDashboardProjection = {
	operationalResponsibility: true,
	updatedAtMs: Date.now(),
	unavailable: [],
	health: [
		{
			entityId: "bot-a",
			dimension: "worker",
			state: "healthy",
			required: true,
			observedAtMs: Date.now(),
			eventId: "event-1",
			condition: "heartbeat_ok",
		},
	],
	alerts: [],
	events: [],
	bots: [
		{
			botId: "bot-a",
			state: "running",
			currentAttemptId: "attempt-a",
			currentAttemptState: "running",
			attemptCount: 1,
			decisionCount: 2,
			orderCount: 1,
			unmanagedPositionCount: 0,
			reconciliationRequired: false,
		},
	],
	paperAccount: {
		accountId: "okx-demo",
		market: "okx_spot",
		currency: "usdt",
		cash: "1000",
		reservedCash: "10",
		buyingPower: "990",
		positionCount: 1,
		orderCount: 1,
		fillCount: 1,
		reconciliation: "reconciled",
		restartRequired: false,
		observedAtMs: Date.now(),
	},
	research: {
		watchlistCount: 1,
		snapshotCount: 1,
		componentCount: 1,
		modelArtifactCount: 1,
		signalDatasetCount: 1,
		generationAttemptCount: 1,
		backtestRunCount: 1,
		validationProtocolCount: 1,
		validationReportCount: 1,
		feedbackSnapshotCount: 1,
		feedbackReportCount: 1,
		reviewDecisionCount: 1,
	},
};

test("keeps unavailable Alerts explicit instead of reporting zero unresolved Critical conditions", async () => {
	const container = document.createElement("div");
	const root = createRoot(container);
	document.body.append(container);
	await act(async () =>
		root.render(
			<SystemDashboard
				projection={{
					...responsibleProjection,
					alerts: [],
					unavailable: ["alerts"],
				}}
			/>,
		),
	);
	await settle();

	expect(container.textContent).toContain("Notification center");
	expect(container.textContent).toContain(
		"This summary is temporarily unavailable.",
	);
	expect(container.textContent).not.toContain("0 unresolved Critical condition");

	await act(async () => root.unmount());
	container.remove();
});

test("uses the System Dashboard at the root for an operationally responsible User", async () => {
	mockInvoke.mockImplementation(async (command: string) => {
		if (command === "system_dashboard") return responsibleProjection;
		throw new Error(`unexpected command ${command}`);
	});
	const container = document.createElement("div");
	const root = createRoot(container);
	document.body.append(container);
	await act(async () => {
		root.render(
			<AuthenticatedUserContext.Provider value="alice">
				<QueryClientProvider client={new QueryClient()}>
					<WorkflowHomePage />
				</QueryClientProvider>
			</AuthenticatedUserContext.Provider>,
		);
	});
	await settle();

	expect(container.textContent).toContain("System Dashboard");
	expect(container.textContent).not.toContain("Workflow Guide");

	await act(async () => root.unmount());
	container.remove();
});

test("renders the authorized global projection without cross-currency totals", async () => {
	const container = document.createElement("div");
	const root = createRoot(container);
	document.body.append(container);
	await act(async () =>
		root.render(<SystemDashboard projection={responsibleProjection} />),
	);
	await settle();

	expect(container.textContent).toContain("System Dashboard");
	expect(container.textContent).toContain("USDT");
	expect(container.textContent).toContain("Orders / Fills");
	expect(container.textContent).not.toContain("USD 1990");
	for (const path of [
		"/operations",
		"/bots",
		"/paper-trading",
		"/markets",
		"/factors",
		"/validation",
		"/components",
	]) {
		expect(container.querySelector(`a[href="${path}"]`)).not.toBeNull();
	}

	await act(async () => root.unmount());
	container.remove();
});
