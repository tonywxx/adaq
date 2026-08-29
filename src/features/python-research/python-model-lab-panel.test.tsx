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
	factorDecisionHash?: string;
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
		factorDecisionHash,
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

function makeExperimentWithCompletedTrial(trialId: string): ExperimentFixture {
	const experiment = makeExperiment(false);
	const trial = experiment.trials.find((item) => item.trialId === trialId);
	if (!trial) throw new Error(`unknown trial ${trialId}`);
	const index = experiment.trials.indexOf(trial);
	experiment.trials[index] = {
		...trial,
		status: "completed",
		attemptIds: [`attempt-complete-${index}`],
		successfulAttemptId: `attempt-complete-${index}`,
		candidateArtifactSha256: `candidate-complete-${index}`,
		selectionMetric: index + 1,
		repeatabilityState: "verified",
	};
	return experiment;
}

let experimentResponse = makeExperiment(false);
let persistedExperiments: ExperimentFixture[] = [];
let persistedDecision: Record<string, unknown> | null = null;
let persistedReport: Record<string, unknown> | null = null;
let finalEvaluationResponse: Record<string, unknown> | null = null;
let finalEvaluationError: string | undefined;
let attemptsResponse: Array<Record<string, unknown>> = [];
let completionResponse: ExperimentFixture | undefined;
let partialCompletionResponses = false;
let delayedTrialCompletion:
	| {
			promise: Promise<ExperimentFixture>;
			resolve: (experiment: ExperimentFixture) => void;
	  }
	| undefined;

const invokeMock = jest.requireMock("@tauri-apps/api/core").invoke as jest.Mock;

invokeMock.mockImplementation(
	async (
		command: string,
		args?: {
			request?: {
				alpha?: number;
				trialId?: string;
				factorDecisionHash?: string;
			};
		},
	) => {
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
				return attemptsResponse;
			case "model_lab_state": {
				const requestedFactorDecisionHash =
					args?.request?.factorDecisionHash;
				const matchingExperiments = requestedFactorDecisionHash
					? persistedExperiments.filter(
							(item) =>
								item.factorDecisionHash === requestedFactorDecisionHash,
					  )
					: persistedExperiments;
				return {
					experiments: persistedExperiments,
					experiment:
						matchingExperiments.length === 1 ? matchingExperiments[0] : null,
					decision: persistedDecision,
					report: persistedReport,
					finalEvaluation: finalEvaluationResponse,
				};
			}
			case "model_demo_run": {
				const alpha = args?.request?.alpha ?? 1;
				return {
					...demoRun,
					alpha,
					attemptId: alpha === 1 ? demoRun.attemptId : `attempt-alpha-${alpha}`,
				};
			}
			case "model_experiment_register":
				return experimentResponse;
			case "model_trial_retry":
				return experimentResponse;
			case "model_trial_complete":
				if (args?.request?.trialId === "trial-0" && delayedTrialCompletion) {
					return delayedTrialCompletion.promise;
				}
				if (completionResponse) return completionResponse;
				if (partialCompletionResponses && args?.request?.trialId) {
					return makeExperimentWithCompletedTrial(args.request.trialId);
				}
				return experimentResponse;
			case "model_selection_record": {
				const nextDecision = {
					decisionId: "decision-sha",
					selectedTrialId: "trial-1",
					selectedAlpha: 1,
					bindingSha256,
					projectRevisionSha256: revision,
					environmentSha256,
					inputEvidenceSha256,
					seed: 7,
					selectionMetricsSha256: "selection-metrics-sha",
					candidateArtifactSha256: "candidate-artifact-1",
					evidenceState: "unknown",
				};
				persistedDecision = nextDecision;
				return nextDecision;
			}
			case "model_final_evaluate":
				if (finalEvaluationError) throw finalEvaluationError;
				persistedReport = {
					reportId: "report-sha",
					decisionId: "decision-sha",
					forecastSha256: "forecast-final-sha",
					targetSha256: "target-sha",
					meanSquaredError: 0.1,
					meanAbsoluteError: 0.2,
					evidenceState: "out-of-sample",
					artifactSha256: "candidate-artifact-1",
					forecastDatasetSha256: "forecast-dataset-sha",
				};
				finalEvaluationResponse = {
					decisionId: "decision-sha",
					status: "completed",
					attemptId: "final-attempt-retry",
					stagedDatasetSha256: "forecast-dataset-sha",
					reportId: "report-sha",
					createdAtMs: 1,
					updatedAtMs: 2,
				};
				return persistedReport;
			default:
				return null;
		}
	},
);

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
	persistedExperiments = [];
	persistedDecision = null;
	persistedReport = null;
	finalEvaluationResponse = null;
	finalEvaluationError = undefined;
	attemptsResponse = [];
	completionResponse = undefined;
	partialCompletionResponses = false;
	delayedTrialCompletion = undefined;
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

test("reloads persisted trials with recovery status and source Attempt identity", async () => {
	persistedExperiments = [makeExperiment(false)];
	attemptsResponse = [
		{
			attemptId: "attempt-interrupted",
			projectId,
			revisionSha256: revision,
			environmentSha256,
			status: "interrupted",
			sourceAttemptId: "attempt-original",
			queueSequence: 4,
			failureCode: "research-interrupted",
			diagnostic: "Runner was not terminal at application restart",
			execution: { parameters: { alpha: "0.1" } },
		},
	];
	const { container, root } = mount();
	await act(async () => {
		root.render(<PythonModelLabPanel userId={userId} />);
	});
	await settle();

	expect(container.textContent).toContain("Experiment experiment-sha");
	expect(container.textContent).toContain("Interrupted");
	expect(container.textContent).toContain("Source Attempt attempt-original");
	expect(container.textContent).toContain(
		"No downstream Forecast Signal Dataset is published for this Trial.",
	);
	expect(container.querySelector('[data-status="interrupted"]')).not.toBeNull();

	await act(async () => root.unmount());
});

test("keeps retry visible after a failed Attempt is retained on the Trial", async () => {
	const experiment = makeExperiment(false);
	experiment.trials[0] = {
		...experiment.trials[0],
		status: "failed",
		attemptIds: ["attempt-failed"],
		diagnostics: ["model-host-save-failed"],
	};
	persistedExperiments = [experiment];
	attemptsResponse = [
		{
			attemptId: "attempt-failed",
			projectId,
			revisionSha256: revision,
			environmentSha256,
			status: "failed",
			failureCode: "model-host-save-failed",
			diagnostic: "bounded host diagnostic",
			execution: { parameters: { alpha: "0.1" } },
		},
	];
	const { container, root } = mount();
	await act(async () => {
		root.render(<PythonModelLabPanel userId={userId} />);
	});
	await settle();

	expect(container.textContent).toContain("Retry this Trial");
	const retainButton = [...container.querySelectorAll("button")].find(
		(button) => button.textContent === "Retain failure",
	);
	expect(retainButton?.hasAttribute("disabled")).toBe(true);

	await act(async () => root.unmount());
});

test("ignores an older Trial completion after Retry starts", async () => {
	let resolveOldCompletion: (experiment: ExperimentFixture) => void = () => {};
	delayedTrialCompletion = {
		promise: new Promise((resolve) => {
			resolveOldCompletion = resolve;
		}),
		resolve: (experiment) => resolveOldCompletion(experiment),
	};
	attemptsResponse = [
		{
			attemptId: "attempt-failed",
			projectId,
			revisionSha256: revision,
			environmentSha256,
			status: "failed",
			execution: { parameters: { alpha: "0.1" } },
		},
	];
	completionResponse = makeExperimentWithCompletedTrial("trial-0");
	const { container, root } = mount();
	await prepareGrid(root, container);

	const completeButton = [...container.querySelectorAll("button")].find(
		(button) => button.textContent === "Complete trial",
	);
	if (!completeButton)
		throw new Error("initial completion button did not render");
	await act(async () => {
		completeButton.click();
		await Promise.resolve();
		await Promise.resolve();
	});

	delayedTrialCompletion = undefined;
	const retryButton = [...container.querySelectorAll("button")].find(
		(button) => button.textContent === "Retry this Trial",
	);
	if (!retryButton) throw new Error("retry button did not render");
	await act(async () => {
		retryButton.click();
		await Promise.resolve();
		await Promise.resolve();
	});
	await settle();

	const oldCompletion = makeExperimentWithCompletedTrial("trial-0");
	oldCompletion.trials[0].candidateArtifactSha256 = "candidate-old";
	resolveOldCompletion(oldCompletion);
	await settle();

	expect(container.textContent).toContain(
		"Candidate Model Artifact candidate-complete-0",
	);
	expect(container.textContent).not.toContain(
		"Candidate Model Artifact candidate-old",
	);

	await act(async () => root.unmount());
});

test("retries the same Trial from its failed Attempt and binds the new Attempt", async () => {
	attemptsResponse = [
		{
			attemptId: "attempt-interrupted",
			projectId,
			revisionSha256: revision,
			environmentSha256,
			status: "interrupted",
			sourceAttemptId: "attempt-original",
			queueSequence: 4,
			failureCode: "research-interrupted",
			diagnostic: "Runner was not terminal at application restart",
			execution: { parameters: { alpha: "0.1" } },
		},
	];
	completionResponse = makeExperiment(true);
	const { container, root } = mount();
	await prepareGrid(root, container);

	const retryButton = [...container.querySelectorAll("button")].find(
		(button) => button.textContent === "Retry this Trial",
	);
	if (!retryButton) throw new Error("same-Trial retry button did not render");
	await act(async () => {
		retryButton.click();
		await Promise.resolve();
		await Promise.resolve();
	});
	await settle();

	const retryCall = invokeMock.mock.calls.find(
		([command]) => command === "model_trial_retry",
	);
	expect(retryCall?.[1]).toEqual({
		request: {
			userId,
			experimentId: "experiment-sha",
			trialId: "trial-0",
			attemptId: "attempt-interrupted",
		},
	});
	const modelRuns = invokeMock.mock.calls.filter(
		([command]) => command === "model_demo_run",
	);
	expect(modelRuns.at(-1)?.[1]).toEqual({
		request: expect.objectContaining({
			alpha: 0.1,
			retryAttemptId: "attempt-interrupted",
		}),
	});
	expect(container.textContent).toContain(
		"Candidate Model Artifact candidate-artifact-0",
	);

	await act(async () => root.unmount());
});

test("keeps concurrent Trial completions from overwriting each other", async () => {
	partialCompletionResponses = true;
	const { container, root } = mount();
	await prepareGrid(root, container);

	const completeButtons = [...container.querySelectorAll("button")].filter(
		(button) => button.textContent === "Complete trial",
	);
	expect(completeButtons).toHaveLength(3);
	await act(async () => {
		completeButtons[0].click();
		completeButtons[1].click();
		await Promise.resolve();
		await Promise.resolve();
	});
	await settle();

	expect(
		invokeMock.mock.calls.filter(
			([command]) => command === "model_trial_complete",
		),
	).toHaveLength(2);
	expect(container.textContent).toContain(
		"Candidate Model Artifact candidate-complete-0",
	);
	expect(container.textContent).toContain(
		"Candidate Model Artifact candidate-complete-1",
	);

	await act(async () => root.unmount());
});

test("restores the persisted Decision, Report, and recoverable Final Evaluation state", async () => {
	persistedExperiments = [makeExperiment(true)];
	persistedDecision = {
		decisionId: "decision-sha",
		selectedTrialId: "trial-1",
		selectedAlpha: 1,
		bindingSha256,
		projectRevisionSha256: revision,
		environmentSha256,
		inputEvidenceSha256,
		seed: 7,
		selectionMetricsSha256: "selection-metrics-sha",
		candidateArtifactSha256: "candidate-artifact-1",
		evidenceState: "unknown",
	};
	persistedReport = {
		reportId: "report-sha",
		decisionId: "decision-sha",
		forecastSha256: "forecast-final-sha",
		targetSha256: "target-sha",
		meanSquaredError: 0.1,
		meanAbsoluteError: 0.2,
		evidenceState: "out-of-sample",
		artifactSha256: "candidate-artifact-1",
		forecastDatasetSha256: "forecast-dataset-sha",
	};
	finalEvaluationResponse = {
		decisionId: "decision-sha",
		status: "persistence-failed",
		attemptId: "final-attempt",
		stagedDatasetSha256: "forecast-dataset-sha",
		failureCode: "model-final-evaluation-persistence-failed",
		diagnostic: "report persistence failed",
		createdAtMs: 1,
		updatedAtMs: 2,
	};
	const { container, root } = mount();
	await act(async () => {
		root.render(<PythonModelLabPanel userId={userId} />);
	});
	await settle();

	expect(container.textContent).toContain("User selected α=1");
	expect(container.textContent).toContain("Final MSE 0.1 · MAE 0.2");
	expect(container.textContent).toContain("Persistence failed");
	expect(container.textContent).toContain("Staged Forecast Dataset forecast-dataset-sha");
	expect(container.textContent).toContain("Retry final evaluation");
	expect(container.querySelector('[data-status="persistence-failed"]')).not.toBeNull();

	await act(async () => root.unmount());
});

test("retries Final Evaluation with the same Decision identity", async () => {
	persistedExperiments = [makeExperiment(true)];
	persistedDecision = {
		decisionId: "decision-sha",
		selectedTrialId: "trial-1",
		selectedAlpha: 1,
		bindingSha256,
		projectRevisionSha256: revision,
		environmentSha256,
		inputEvidenceSha256,
		seed: 7,
		selectionMetricsSha256: "selection-metrics-sha",
		candidateArtifactSha256: "candidate-artifact-1",
		evidenceState: "unknown",
	};
	finalEvaluationResponse = {
		decisionId: "decision-sha",
		status: "failed",
		failureCode: "model-final-evaluation-failed",
		createdAtMs: 1,
		updatedAtMs: 2,
	};
	const { container, root } = mount();
	await act(async () => {
		root.render(<PythonModelLabPanel userId={userId} />);
	});
	await settle();

	const retryButton = [...container.querySelectorAll("button")].find(
		(button) => button.textContent === "Retry final evaluation",
	);
	if (!retryButton) throw new Error("Final Evaluation retry button did not render");
	await act(async () => {
		retryButton.click();
		await Promise.resolve();
		await Promise.resolve();
	});
	await settle();

	const finalCall = invokeMock.mock.calls.find(
		([command]) => command === "model_final_evaluate",
	);
	expect(finalCall?.[1]).toEqual({
		request: { userId, decisionId: "decision-sha" },
	});
	expect(container.textContent).toContain("Completed");
	expect(container.textContent).toContain("Final MSE 0.1 · MAE 0.2");
	expect(container.querySelector('[data-status="completed"]')).not.toBeNull();

	await act(async () => root.unmount());
});

test("renders persisted Model Research status and controls in simplified Chinese", async () => {
	i18n.changeLanguage("zh-CN");
	persistedExperiments = [makeExperiment(true)];
	const { container, root } = mount();
	await act(async () => {
		root.render(<PythonModelLabPanel userId={userId} />);
	});
	await settle();

	expect(container.textContent).toContain("Host-fed Qlib Ridge 模型");
	expect(container.textContent).toContain("已完成");
	expect(container.textContent).toContain(
		"此 Trial 不会发布下游 Forecast Signal Dataset。",
	);
	expect(
		[...container.querySelectorAll("button")].filter(
			(button) => button.textContent === "记录选择",
		),
	).toHaveLength(3);

	await act(async () => root.unmount());
});
