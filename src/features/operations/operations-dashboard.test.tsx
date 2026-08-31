/** @jest-environment jsdom */

import "@/lib/i18n";
import { AuthenticatedUserContext } from "@/authenticated-user";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { act } from "react";
import { createRoot } from "react-dom/client";
import type { ReactNode } from "react";
import { OperationsDashboard } from "./operations-dashboard";

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
		if (command === "operations_alert_history") return [];
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
