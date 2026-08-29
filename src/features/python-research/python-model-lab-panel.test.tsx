/** @jest-environment jsdom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { i18n } from "@/lib/i18n";
import "@/lib/i18n";
import { PythonModelLabPanel } from "./python-model-lab-panel";

jest.mock("@tauri-apps/api/core", () => ({
	invoke: jest.fn(),
}));

(
	globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }
).IS_REACT_ACT_ENVIRONMENT = true;

type TrialFixture = {
	trialId: string;
	alpha: number;
	status: string;
	attemptIds: string[];
	successfulAttemptId?: string;
	candidateArtifactSha256?: string;
	selectionMetric?: number;
	evidenceState: "out-of-sample" | "overlapping" | "unknown";
	repeatabilityState: "unverified" | "verified" | "divergent";
	diagnostics: string[];
};

type ExperimentFixture = {
	experimentId: string;
	projectRevisionSha256: string;
	environmentSha256: string;
	inputEvidenceSha256: string;
	bindingSha256: string;
	seed: number;
	lineageEvidenceState: "out-of-sample" | "overlapping" | "unknown";
	trials: TrialFixture[];
};

const userId = "user-a";
const projectId = "py-model-qlib-ridge-return";
const revision = "revision-sha";
const environmentSha256 = "environment-sha";
const inputEvidenceSha256 = "input-sha";
const bindingSha256 = "binding-sha";
const factorDecisionHash = "factor-decision-sha";

const demoRun = {
	attemptId: "attempt-demo",
	adapterId: "adaq-python-ridge@1",
	alpha: 1,
	projectRevisionSha256: revision,
	environmentSha256,
	inputEvidenceSha256,
	bindingSha256,
	seed: 7,
	fixtureSha256: "fixture-sha",
	artifactSha256: "artifact-demo",
	transformationSha256: "transformation-sha",
	forecastSha256: "forecast-sha",
	trainRows: 10,
	selectionRows: 4,
	selectionMetric: 0.25,
	finalRows: 5,
	testLabelsWithheld: true,
	repeatabilityVerified: true,
	repeatabilityTolerance: 1e-9,
	repeatabilityState: "verified" as const,
	evidenceState: "unknown" as const,
	diagnostics: [],
	resourcePolicy: { cpu: 1 },
	inputSlots: ["close"],
	targetId: "future-close-return",
	targetHorizonBars: 5,
	forecastContract: "adaq:forecast@1",
	artifactSchema: "adaq:linear-model-artifact@1",
	numericRepresentation: "f64",
	factorDecisionHash,
	factorPromotionProtocolHash: "protocol-sha",
	factorDatasetId: "factor-dataset-sha",
	featureDatasetId: "feature-dataset-sha",
	featurePlanHash: "feature-plan-sha",
	snapshotId: "snapshot-sha",
	universeId: "universe-sha",
	factorLookback: 20,
	windows: {
		trainStart: 1,
		trainEnd: 10,
		selectionStart: 11,
		selectionEnd: 14,
		finalStart: 15,
		finalEnd: 19,
	},
};

function makeExperiment(completed: boolean): ExperimentFixture {
	return {
		experimentId: "experiment-sha",
		projectRevisionSha256: revision,
		environmentSha256,
		inputEvidenceSha256,
		bindingSha256,
		seed: 7,
		lineageEvidenceState: "unknown",
		trials: [0.1, 1, 10].map((alpha, index) => ({
			trialId: `trial-${index}`,
			alpha,
			status: completed ? "completed" : "registered",
			attemptIds: completed ? [`attempt-${index}`] : [],
			successfulAttemptId: completed ? `attempt-${index}` : undefined,
			candidateArtifactSha256: completed
				? `candidate-artifact-${index}`
				: undefined,
			selectionMetric: completed ? index + 1 : undefined,
			evidenceState: "unknown",
			repeatabilityState: completed ? "verified" : "unverified",
			diagnostics: [],
		})),
	};
}

let experimentResponse = makeExperiment(false);

const invokeMock = jest.requireMock("@tauri-apps/api/core").invoke as jest.Mock;

invokeMock.mockImplementation(async (command: string) => {
	switch (command) {
		case "project_list":
			return [{ projectId, state: "clean", revisionSha256: revision }];
		case "factor_decision_library":
			return {
				items: [
					{
						decision: {
							decisionHash: factorDecisionHash,
							candidateHash: "factor-candidate-sha",
							state: "research-validated",
						},
						promotionProtocolHash: "protocol-sha",
					},
				],
			};
		case "environment_for_project":
			return { environmentSha256 };
		case "attempt_list":
			return [];
		case "model_demo_run":
			return demoRun;
		case "model_experiment_register":
			return experimentResponse;
		case "model_selection_record":
			return {
				decisionId: "decision-sha",
				selectedTrialId: "trial-1",
				selectedAlpha: 1,
				bindingSha256,
				projectRevisionSha256: revision,
				environmentSha256,
				inputEvidenceSha256,
				seed: 7,
				candidateArtifactSha256: "candidate-artifact-1",
				evidenceState: "unknown",
			};
		default:
			return null;
	}
});

function mount() {
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	return { container, root };
}

async function settle() {
	await act(async () => {
		await Promise.resolve();
		await Promise.resolve();
		await Promise.resolve();
	});
}

async function prepareGrid(root: Root, container: HTMLDivElement) {
	await act(async () => {
		root.render(<PythonModelLabPanel userId={userId} />);
	});
	await settle();

	const factorDecision = container.querySelector<HTMLSelectElement>(
		"#python-model-factor-decision",
	);
	if (!factorDecision)
		throw new Error("factor decision selector did not render");
	await act(async () => {
		factorDecision.value = factorDecisionHash;
		factorDecision.dispatchEvent(new Event("change", { bubbles: true }));
	});
	await settle();

	const runButton = [...container.querySelectorAll("button")].find(
		(button) => button.textContent === "Run α=1 demo",
	);
	if (!runButton) throw new Error("run button did not render");
	await act(async () => {
		runButton.click();
		await Promise.resolve();
		await Promise.resolve();
	});
	await settle();

	const registerButton = [...container.querySelectorAll("button")].find(
		(button) => button.textContent === "Register α grid",
	);
	if (!registerButton) throw new Error("register button did not render");
	await act(async () => {
		registerButton.click();
		await Promise.resolve();
		await Promise.resolve();
	});
	await settle();
}

beforeEach(() => {
	i18n.changeLanguage("en-US");
	experimentResponse = makeExperiment(false);
	invokeMock.mockClear();
	Object.defineProperty(window, "__TAURI_INTERNALS__", {
		configurable: true,
		value: {},
	});
	Object.defineProperty(globalThis, "requestAnimationFrame", {
		configurable: true,
		value: (callback: FrameRequestCallback) => {
			callback(0);
			return 0;
		},
	});
});

afterEach(() => {
	document.body.replaceChildren();
	delete (window as Window & { __TAURI_INTERNALS__?: unknown })
		.__TAURI_INTERNALS__;
});

test("keeps selection blocked until every trial has its candidate identity", async () => {
	const { container, root } = mount();
	await prepareGrid(root, container);

	expect(container.textContent).toContain(
		"Selection requires all three alpha trials to be completed, repeatable, and bound to candidate artifacts.",
	);
	expect(container.textContent).toContain("Successful Attempt —");
	expect(container.textContent).toContain("Candidate Model Artifact —");
	expect(container.textContent).not.toContain("Forecast dataset");

	await act(async () => root.unmount());
});

test("renders every successful attempt and candidate artifact, then binds selection", async () => {
	experimentResponse = makeExperiment(true);
	const { container, root } = mount();
	await prepareGrid(root, container);

	for (let index = 0; index < 3; index += 1) {
		expect(container.textContent).toContain(
			`Successful Attempt attempt-${index}`,
		);
		expect(container.textContent).toContain(
			`Candidate Model Artifact candidate-artifact-${index}`,
		);
	}

	const selectButtons = [...container.querySelectorAll("button")].filter(
		(button) => button.textContent === "Record selection",
	);
	expect(selectButtons).toHaveLength(3);
	await act(async () => {
		selectButtons[1].click();
		await Promise.resolve();
		await Promise.resolve();
	});
	await settle();

	expect(container.textContent).toContain(
		"Candidate Artifact candidate-artifact-1",
	);
	const selectionCall = invokeMock.mock.calls.find(
		([command]) => command === "model_selection_record",
	);
	expect(selectionCall?.[1]).toEqual({
		request: {
			userId,
			experimentId: "experiment-sha",
			trialId: "trial-1",
		},
	});

	await act(async () => root.unmount());
});
