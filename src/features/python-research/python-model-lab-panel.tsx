import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "@/components/ui/card";
import { isTauriRuntime } from "@/lib/http";
import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

type Project = {
	projectId: string;
	state: "clean" | "dirty" | "invalid";
	revisionSha256?: string;
};

type Environment = {
	environmentSha256: string;
};

type FactorDecision = {
	decision: {
		decisionHash: string;
		candidateHash: string;
		state: "rejected" | "research-validated" | "component-eligible";
	};
	promotionProtocolHash: string;
};

type FactorDecisionPage = {
	items: FactorDecision[];
};

type ModelRun = {
	attemptId: string;
	adapterId: string;
	alpha: number;
	projectRevisionSha256: string;
	environmentSha256: string;
	inputEvidenceSha256: string;
	bindingSha256: string;
	seed: number;
	fixtureSha256: string;
	artifactSha256: string;
	transformationSha256: string;
	forecastSha256: string;
	trainRows: number;
	selectionRows: number;
	selectionMetric?: number;
	finalRows: number;
	testLabelsWithheld: boolean;
	repeatabilityVerified: boolean;
	repeatabilityTolerance: number;
	repeatabilityState: "unverified" | "verified" | "divergent";
	evidenceState: "out-of-sample" | "overlapping" | "unknown";
	diagnostics: string[];
	resourcePolicy: Record<string, number | string>;
	inputSlots: string[];
	targetId: string;
	targetHorizonBars: number;
	forecastContract: string;
	artifactSchema: string;
	numericRepresentation: string;
	factorDecisionHash: string;
	factorPromotionProtocolHash: string;
	factorDatasetId: string;
	featureDatasetId: string;
	featurePlanHash: string;
	snapshotId: string;
	universeId: string;
	factorLookback: number;
	windows: {
		trainStart: number;
		trainEnd: number;
		selectionStart: number;
		selectionEnd: number;
		finalStart: number;
		finalEnd: number;
	};
};

type Trial = {
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

type Experiment = {
	experimentId: string;
	projectRevisionSha256: string;
	environmentSha256: string;
	inputEvidenceSha256: string;
	bindingSha256: string;
	seed: number;
	factorDecisionHash?: string;
	parentDecisionId?: string;
	lineageEvidenceState: "out-of-sample" | "overlapping" | "unknown";
	trials: Trial[];
};

type Decision = {
	decisionId: string;
	selectedTrialId: string;
	selectedAlpha: number;
	bindingSha256: string;
	projectRevisionSha256: string;
	environmentSha256: string;
	inputEvidenceSha256: string;
	seed: number;
	candidateArtifactSha256: string;
	evidenceState: "out-of-sample" | "overlapping" | "unknown";
};

type Report = {
	reportId: string;
	decisionId: string;
	forecastSha256: string;
	meanSquaredError: number;
	meanAbsoluteError: number;
	evidenceState: "out-of-sample" | "overlapping" | "unknown";
	artifactSha256: string;
	forecastDatasetSha256: string;
};

type ResearchAttempt = {
	attemptId: string;
	projectId: string;
	revisionSha256: string;
	environmentSha256: string;
	status:
		| "pending"
		| "running"
		| "completed"
		| "failed"
		| "cancelled"
		| "interrupted"
		| "stale";
	sourceAttemptId?: string;
	cancelRequested?: boolean;
	queueSequence: number;
	failureCode?: string;
	diagnostic?: string;
	progressCompleted?: number;
	progressTotal?: number;
	execution?: {
		parameters?: Record<string, string>;
	};
};

type RequestToken = {
	key: string;
	version: number;
	userEpoch: number;
};

const RETRYABLE_ATTEMPT_STATUSES = new Set([
	"failed",
	"cancelled",
	"interrupted",
	"stale",
]);

const isRetryableAttempt = (attempt: ResearchAttempt) =>
	RETRYABLE_ATTEMPT_STATUSES.has(attempt.status) ||
	(attempt.status === "completed" && Boolean(attempt.failureCode));

function mergeExperiment(
	current: Experiment | undefined,
	next: Experiment,
	focusTrialId?: string,
): Experiment {
	if (!current || current.experimentId !== next.experimentId || !focusTrialId) {
		return next;
	}
	const incoming = next.trials.find((trial) => trial.trialId === focusTrialId);
	if (!incoming) return current;
	return {
		...next,
		trials: current.trials.map((trial) =>
			trial.trialId === focusTrialId ? incoming : trial,
		),
	};
}

const PROJECTS_CHANGED_EVENT = "adaq:python-projects-changed";

const afterPaint = () =>
	new Promise<void>((resolve) => {
		if (
			typeof requestAnimationFrame === "undefined" ||
			document.visibilityState === "hidden"
		)
			return resolve();
		requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
		window.setTimeout(resolve, 100);
	});

export function PythonModelLabPanel({ userId }: { userId: string }) {
	const { t } = useTranslation();
	const [projects, setProjects] = useState<Project[]>([]);
	const [projectId, setProjectId] = useState("");
	const [environment, setEnvironment] = useState<Environment>();
	const [factorDecisions, setFactorDecisions] = useState<FactorDecision[]>([]);
	const [factorDecisionHash, setFactorDecisionHash] = useState("");
	const [run, setRun] = useState<ModelRun>();
	const [experiment, setExperiment] = useState<Experiment>();
	const [decision, setDecision] = useState<Decision>();
	const [report, setReport] = useState<Report>();
	const [attempts, setAttempts] = useState<ResearchAttempt[]>([]);
	const [busy, setBusy] = useState("");
	const [error, setError] = useState("");
	const requestVersions = useRef(new Map<string, number>());
	const userEpoch = useRef(0);
	const activeUserId = useRef(userId);
	activeUserId.current = userId;
	const beginRequest = useCallback((key = "workspace"): RequestToken => {
		const version = (requestVersions.current.get(key) ?? 0) + 1;
		requestVersions.current.set(key, version);
		return { key, version, userEpoch: userEpoch.current };
	}, []);
	const isCurrentRequest = useCallback(
		(token: RequestToken) =>
			token.userEpoch === userEpoch.current &&
			requestVersions.current.get(token.key) === token.version,
		[],
	);

	useEffect(() => {
		activeUserId.current = userId;
		userEpoch.current += 1;
		requestVersions.current.clear();
		setBusy("");
		setRun(undefined);
		setExperiment(undefined);
		setDecision(undefined);
		setReport(undefined);
		return () => {
			userEpoch.current += 1;
			requestVersions.current.clear();
		};
	}, [userId]);

	const refreshProjects = useCallback(async () => {
		if (!isTauriRuntime()) return;
		const token = beginRequest("projects");
		try {
			const [current, decisionPage] = await Promise.all([
				invoke<Project[]>("project_list", { userId }),
				invoke<FactorDecisionPage>("factor_decision_library", {
					request: { userId, page: 1, pageSize: 50 },
				}),
			]);
			if (!isCurrentRequest(token) || activeUserId.current !== userId) return;
			setProjects(current);
			setFactorDecisions(
				decisionPage.items.filter((item) => item.decision.state !== "rejected"),
			);
			const modelProject = current.find(
				(project) => project.projectId === "py-model-qlib-ridge-return",
			);
			setProjectId(modelProject?.projectId ?? "");
			if (modelProject) {
				const nextEnvironment = await invoke<Environment | null>(
					"environment_for_project",
					{
						request: { userId, projectId: modelProject.projectId },
					},
				);
				if (!isCurrentRequest(token) || activeUserId.current !== userId) return;
				setEnvironment(nextEnvironment ?? undefined);
			} else {
				setEnvironment(undefined);
			}
		} catch (reason) {
			if (isCurrentRequest(token) && activeUserId.current === userId) {
				setError(String(reason));
			}
		}
	}, [beginRequest, isCurrentRequest, userId]);

	const refreshExperiments = useCallback(
		async (selectedFactorDecisionHash = factorDecisionHash) => {
			if (!isTauriRuntime()) return;
			const token = beginRequest("experiments");
			try {
				const nextExperiments = await invoke<Experiment[]>(
					"model_experiment_list",
					{
						userId,
					},
				);
				if (!isCurrentRequest(token) || activeUserId.current !== userId) return;
				const matchingExperiments = selectedFactorDecisionHash
					? nextExperiments.filter(
							(item) => item.factorDecisionHash === selectedFactorDecisionHash,
						)
					: nextExperiments;
				const nextExperiment =
					matchingExperiments.length === 1 ? matchingExperiments[0] : undefined;
				setExperiment(nextExperiment);
				if (!nextExperiment) return;
				if (nextExperiment.factorDecisionHash) {
					setFactorDecisionHash(nextExperiment.factorDecisionHash);
				}
			} catch (reason) {
				if (isCurrentRequest(token) && activeUserId.current === userId) {
					setError(String(reason));
				}
			}
		},
		[beginRequest, factorDecisionHash, isCurrentRequest, userId],
	);

	const refreshAttempts = useCallback(async () => {
		if (!isTauriRuntime()) return;
		const token = beginRequest("attempts");
		try {
			const nextAttempts = await invoke<ResearchAttempt[]>("attempt_list", {
				userId,
			});
			if (isCurrentRequest(token) && activeUserId.current === userId) {
				setAttempts(nextAttempts);
			}
		} catch (reason) {
			if (isCurrentRequest(token) && activeUserId.current === userId) {
				setError(String(reason));
			}
		}
	}, [beginRequest, isCurrentRequest, userId]);

	useEffect(() => {
		void refreshProjects();
		void refreshExperiments();
	}, [refreshExperiments, refreshProjects]);

	useEffect(() => {
		void refreshAttempts();
		if (!isTauriRuntime()) return;
		const timer = window.setInterval(() => void refreshAttempts(), 2000);
		return () => window.clearInterval(timer);
	}, [refreshAttempts]);

	useEffect(() => {
		const refresh = () => void refreshProjects();
		window.addEventListener(PROJECTS_CHANGED_EVENT, refresh);
		return () => window.removeEventListener(PROJECTS_CHANGED_EVENT, refresh);
	}, [refreshProjects]);

	const runDefault = async () => {
		const version = beginRequest("workspace");
		const project = projects.find((item) => item.projectId === projectId);
		if (!project?.revisionSha256 || !environment) {
			setError(t("pythonResearch.modelLab.freezeAndPrepareRequired"));
			return;
		}
		if (!factorDecisionHash) {
			setError(t("pythonResearch.modelLab.factorDecisionRequired"));
			return;
		}
		setBusy("run");
		setError("");
		setExperiment(undefined);
		setDecision(undefined);
		setReport(undefined);
		setRun(undefined);
		await afterPaint();
		if (!isCurrentRequest(version)) return;
		try {
			const nextRun = await invoke<ModelRun>("model_demo_run", {
				request: {
					userId,
					projectId,
					projectRevisionSha256: project.revisionSha256,
					environmentSha256: environment.environmentSha256,
					factorDecisionHash,
					alpha: 1,
				},
			});
			if (isCurrentRequest(version)) setRun(nextRun);
		} catch (reason) {
			if (isCurrentRequest(version)) setError(String(reason));
		} finally {
			if (isCurrentRequest(version)) setBusy("");
		}
	};

	const registerGrid = async () => {
		const version = beginRequest("workspace");
		if (!run) return;
		setBusy("register");
		setError("");
		await afterPaint();
		if (!isCurrentRequest(version)) return;
		try {
			const currentProjects = isTauriRuntime()
				? await invoke<Project[]>("project_list", { userId })
				: projects;
			setProjects(currentProjects);
			const revision = currentProjects.find(
				(project) =>
					project.projectId === projectId &&
					project.state === "clean" &&
					project.revisionSha256,
			)?.revisionSha256;
			if (!revision || !environment) {
				throw new Error(t("pythonResearch.modelLab.freezeRequired"));
			}
			if (
				run.projectRevisionSha256 !== revision ||
				run.environmentSha256 !== environment.environmentSha256 ||
				run.factorDecisionHash !== factorDecisionHash
			) {
				throw new Error(t("pythonResearch.modelLab.bindingChanged"));
			}
			const nextExperiment = await invoke<Experiment>(
				"model_experiment_register",
				{
					request: {
						userId,
						attemptId: run.attemptId,
						projectRevisionSha256: revision,
						environmentSha256: environment.environmentSha256,
						inputEvidenceSha256: run.inputEvidenceSha256,
						factorDecisionHash,
						seed: run.seed,
					},
				},
			);
			if (isCurrentRequest(version)) setExperiment(nextExperiment);
		} catch (reason) {
			if (isCurrentRequest(version)) setError(String(reason));
		} finally {
			if (isCurrentRequest(version)) setBusy("");
		}
	};

	const completeTrial = async (trial: Trial) => {
		const version = beginRequest(`trial:${trial.trialId}`);
		if (!experiment) return;
		const experimentId = experiment.experimentId;
		setBusy(trial.trialId);
		setError("");
		await afterPaint();
		if (!isCurrentRequest(version)) return;
		try {
			const project = projects.find(
				(item) =>
					item.projectId === projectId &&
					item.state === "clean" &&
					item.revisionSha256,
			);
			if (!project?.revisionSha256 || !environment || !factorDecisionHash) {
				throw new Error(t("pythonResearch.modelLab.freezeAndPrepareRequired"));
			}
			const trialRun = await invoke<ModelRun>("model_demo_run", {
				request: {
					userId,
					projectId,
					projectRevisionSha256: project.revisionSha256,
					environmentSha256: environment.environmentSha256,
					factorDecisionHash,
					alpha: trial.alpha,
				},
			});
			const selectionMetric = trialRun.selectionMetric;
			if (selectionMetric === undefined) {
				throw new Error(t("pythonResearch.modelLab.metricUnavailable"));
			}
			if (!isCurrentRequest(version)) return;
			const nextExperiment = await invoke<Experiment>("model_trial_complete", {
				request: {
					userId,
					experimentId,
					trialId: trial.trialId,
					attemptId: trialRun.attemptId,
					selectionMetric,
				},
			});
			if (isCurrentRequest(version)) {
				setRun(trialRun);
				setExperiment((current) =>
					mergeExperiment(current, nextExperiment, trial.trialId),
				);
			}
		} catch (reason) {
			if (isCurrentRequest(version)) setError(String(reason));
		} finally {
			if (isCurrentRequest(version)) setBusy("");
		}
	};

	const selectionReady =
		experiment?.trials.length === 3 &&
		experiment.trials.every(
			(trial) =>
				trial.status === "completed" &&
				trial.repeatabilityState === "verified" &&
				Boolean(trial.successfulAttemptId && trial.candidateArtifactSha256) &&
				(trial.evidenceState === "unknown" ||
					trial.evidenceState === "overlapping"),
		);

	const selectTrial = async (trial: Trial) => {
		const version = beginRequest("selection");
		if (!experiment || !selectionReady || trial.status !== "completed") return;
		setBusy(`select:${trial.trialId}`);
		setError("");
		await afterPaint();
		if (!isCurrentRequest(version)) return;
		try {
			const nextDecision = await invoke<Decision>("model_selection_record", {
				request: {
					userId,
					experimentId: experiment.experimentId,
					trialId: trial.trialId,
				},
			});
			if (isCurrentRequest(version)) setDecision(nextDecision);
		} catch (reason) {
			if (isCurrentRequest(version)) setError(String(reason));
		} finally {
			if (isCurrentRequest(version)) setBusy("");
		}
	};

	const evaluateFinal = async () => {
		const version = beginRequest("final");
		if (!decision) return;
		setBusy("final");
		setError("");
		await afterPaint();
		if (!isCurrentRequest(version)) return;
		try {
			const nextReport = await invoke<Report>("model_final_evaluate", {
				request: { userId, decisionId: decision.decisionId },
			});
			if (isCurrentRequest(version)) setReport(nextReport);
		} catch (reason) {
			if (isCurrentRequest(version)) setError(String(reason));
		} finally {
			if (isCurrentRequest(version)) setBusy("");
		}
	};

	const retryTrial = async (trial: Trial, sourceAttemptId: string) => {
		const version = beginRequest(`trial:${trial.trialId}`);
		if (!experiment) return;
		const experimentId = experiment.experimentId;
		const retryFactorDecisionHash =
			experiment.factorDecisionHash || factorDecisionHash;
		setBusy(`retry:${sourceAttemptId}`);
		setError("");
		await afterPaint();
		if (!isCurrentRequest(version)) return;
		try {
			const resetExperiment = await invoke<Experiment>("model_trial_retry", {
				request: {
					userId,
					experimentId,
					trialId: trial.trialId,
					attemptId: sourceAttemptId,
				},
			});
			if (!isCurrentRequest(version)) return;
			setExperiment((current) =>
				mergeExperiment(current, resetExperiment, trial.trialId),
			);
			const project = projects.find(
				(item) =>
					item.projectId === projectId &&
					item.state === "clean" &&
					item.revisionSha256 === resetExperiment.projectRevisionSha256,
			);
			if (!project?.revisionSha256 || !environment || !retryFactorDecisionHash) {
				throw new Error(t("pythonResearch.modelLab.freezeAndPrepareRequired"));
			}
			const retryRun = await invoke<ModelRun>("model_demo_run", {
				request: {
					userId,
					projectId,
					projectRevisionSha256: project.revisionSha256,
					environmentSha256: environment.environmentSha256,
					factorDecisionHash: retryFactorDecisionHash,
					alpha: trial.alpha,
					retryAttemptId: sourceAttemptId,
				},
			});
			const selectionMetric = retryRun.selectionMetric;
			if (selectionMetric === undefined) {
				throw new Error(t("pythonResearch.modelLab.metricUnavailable"));
			}
			if (!isCurrentRequest(version)) return;
			const completedExperiment = await invoke<Experiment>(
				"model_trial_complete",
				{
					request: {
						userId,
						experimentId,
						trialId: trial.trialId,
						attemptId: retryRun.attemptId,
						selectionMetric,
					},
				},
			);
			if (isCurrentRequest(version)) {
				setRun(retryRun);
				setExperiment((current) =>
					mergeExperiment(current, completedExperiment, trial.trialId),
				);
			}
			await refreshAttempts();
		} catch (reason) {
			if (isCurrentRequest(version)) setError(String(reason));
		} finally {
			if (isCurrentRequest(version)) setBusy("");
		}
	};

	const updateAttempt = async (
		attempt: ResearchAttempt,
		action: "cancel" | "retry",
	) => {
		const version = beginRequest(`attempt:${attempt.attemptId}`);
		setBusy(`${attempt.attemptId}:${action}`);
		setError("");
		await afterPaint();
		if (!isCurrentRequest(version)) return;
		try {
			if (action === "retry") {
				const project = projects.find(
					(item) =>
						item.projectId === projectId &&
						item.state === "clean" &&
						item.revisionSha256,
				);
				if (!project?.revisionSha256 || !environment || !factorDecisionHash) {
					throw new Error(t("pythonResearch.modelLab.freezeAndPrepareRequired"));
				}
				const nextRun = await invoke<ModelRun>("model_demo_run", {
					request: {
						userId,
						projectId,
						projectRevisionSha256: project.revisionSha256,
						environmentSha256: environment.environmentSha256,
						factorDecisionHash,
						alpha: Number(attempt.execution?.parameters?.alpha) || run?.alpha || 1,
						retryAttemptId: attempt.attemptId,
					},
				});
				if (isCurrentRequest(version)) {
					setRun(nextRun);
				}
			} else {
				await invoke("attempt_cancel", {
					request: { userId, attemptId: attempt.attemptId },
				});
			}
			await refreshAttempts();
		} catch (reason) {
			if (isCurrentRequest(version)) setError(String(reason));
		} finally {
			if (isCurrentRequest(version)) setBusy("");
		}
	};

	const modelAttempts = attempts.filter(
		(attempt) => attempt.projectId === "py-model-qlib-ridge-return",
	);

	const retainFailure = async (trial: Trial, attemptId: string) => {
		const version = beginRequest(`trial:${trial.trialId}`);
		if (!experiment) return;
		const experimentId = experiment.experimentId;
		setBusy(`fail:${attemptId}`);
		setError("");
		await afterPaint();
		if (!isCurrentRequest(version)) return;
		try {
			const nextExperiment = await invoke<Experiment>("model_trial_fail", {
				request: {
					userId,
					experimentId,
					trialId: trial.trialId,
					attemptId,
				},
			});
			if (isCurrentRequest(version)) {
				setExperiment((current) =>
					mergeExperiment(current, nextExperiment, trial.trialId),
				);
			}
		} catch (reason) {
			if (isCurrentRequest(version)) setError(String(reason));
		} finally {
			if (isCurrentRequest(version)) setBusy("");
		}
	};

	const failureAttemptsForTrial = (trial: Trial) =>
		modelAttempts.filter(
			(attempt) =>
				isRetryableAttempt(attempt) &&
				attempt.revisionSha256 === experiment?.projectRevisionSha256 &&
				attempt.environmentSha256 === experiment?.environmentSha256 &&
				Number(attempt.execution?.parameters?.alpha) === trial.alpha,
		);

	return (
		<Card className="mt-4" aria-busy={Boolean(busy)}>
			<CardHeader>
				<div className="flex flex-wrap items-start justify-between gap-3">
					<div>
						<CardTitle>{t("pythonResearch.modelLab.title")}</CardTitle>
						<CardDescription>
							{t("pythonResearch.modelLab.description")}
						</CardDescription>
					</div>
					<div className="flex flex-wrap gap-2">
						<Button
							type="button"
							size="sm"
							onClick={() => void runDefault()}
							loading={busy === "run"}
							disabled={!projectId || !environment || !factorDecisionHash}
						>
							{t("pythonResearch.modelLab.runDefault")}
						</Button>
						<Button
							type="button"
							size="sm"
							variant="outline"
							onClick={() => void registerGrid()}
							disabled={
								!run ||
								!factorDecisionHash ||
								!projectId ||
								!environment ||
								!projects.some(
									(project) =>
										project.projectId === projectId &&
										project.state === "clean" &&
										project.revisionSha256,
								)
							}
							loading={busy === "register"}
						>
							{t("pythonResearch.modelLab.registerGrid")}
						</Button>
					</div>
				</div>
			</CardHeader>
			<CardContent className="grid gap-3 text-sm">
				<div className="grid gap-2 rounded-md border p-3">
					<label htmlFor="python-model-factor-decision">
						{t("pythonResearch.modelLab.factorDecision")}
					</label>
					<select
						id="python-model-factor-decision"
						className="h-9 rounded-md border bg-background px-2 text-sm"
						value={factorDecisionHash}
						onChange={(event) => {
							beginRequest();
							setFactorDecisionHash(event.target.value);
							setRun(undefined);
							setExperiment(undefined);
							setDecision(undefined);
							setReport(undefined);
						}}
					>
						<option value="">
							{t("pythonResearch.modelLab.chooseFactorDecision")}
						</option>
						{factorDecisions.map((item) => (
							<option
								key={item.decision.decisionHash}
								value={item.decision.decisionHash}
							>
								{item.decision.state} · {item.decision.candidateHash}
							</option>
						))}
					</select>
					{!factorDecisions.length ? (
						<p className="text-muted-foreground">
							{t("pythonResearch.modelLab.factorDecisionRequired")}
						</p>
					) : null}
				</div>
				{error ? (
					<p className="text-destructive" role="alert">
						{error}
					</p>
				) : null}
				<div className="grid gap-2 rounded-md border p-3">
					<p className="font-medium">{t("pythonResearch.modelLab.attempts")}</p>
					{modelAttempts.length ? (
						modelAttempts.map((attempt) => (
							<div
								key={attempt.attemptId}
								className="flex flex-wrap items-center gap-2 border-t pt-2 first:border-0 first:pt-0"
							>
								<code className="break-all text-xs">{attempt.attemptId}</code>
								<Badge
									variant="outline"
									data-status={attempt.status}
									title={attempt.status}
								>
									{t(`pythonResearch.modelLab.attemptStatus.${attempt.status}`, {
										defaultValue: attempt.status,
									})}
								</Badge>
								<span className="text-muted-foreground">#{attempt.queueSequence}</span>
								{attempt.sourceAttemptId ? (
									<span className="break-all text-muted-foreground">
										{t("pythonResearch.modelLab.sourceAttempt", {
											attempt: attempt.sourceAttemptId,
										})}
									</span>
								) : null}
								{attempt.progressTotal ? (
									<span className="text-muted-foreground">
										{attempt.progressCompleted ?? 0}/{attempt.progressTotal}
									</span>
								) : null}
								{attempt.diagnostic || attempt.failureCode ? (
									<p className="basis-full text-destructive" role="alert">
										{t("pythonResearch.modelLab.diagnostics", {
											value: attempt.diagnostic ?? attempt.failureCode,
										})}
									</p>
								) : null}
								<div className="ml-auto flex gap-2">
									{attempt.status === "pending" || attempt.status === "running" ? (
										<Button
											type="button"
											size="sm"
											variant="outline"
											onClick={() => void updateAttempt(attempt, "cancel")}
											loading={busy === `${attempt.attemptId}:cancel`}
										>
											{t("pythonResearch.projects.cancel")}
										</Button>
									) : null}
									{isRetryableAttempt(attempt) &&
									!experiment?.trials.some(
										(trial) =>
											Number(attempt.execution?.parameters?.alpha) === trial.alpha,
									) ? (
										<Button
											type="button"
											size="sm"
											variant="outline"
											onClick={() => void updateAttempt(attempt, "retry")}
											loading={busy === `${attempt.attemptId}:retry`}
										>
											{t("pythonResearch.projects.retry")}
										</Button>
									) : null}
								</div>
							</div>
						))
					) : (
						<p className="text-xs text-muted-foreground">
							{t("pythonResearch.modelLab.noAttempts")}
						</p>
					)}
				</div>
				{run ? (
					<div className="grid gap-1 rounded-md border p-3">
						<div className="flex flex-wrap items-center gap-2">
							<Badge variant="secondary">
								{t("pythonResearch.modelLab.synthetic")}
							</Badge>
							<span>
								{run.adapterId} · α={run.alpha}
							</span>
							<span className="text-muted-foreground">
								{run.trainRows} / {run.selectionRows} / {run.finalRows} rows
							</span>
						</div>
						<p className="text-xs text-muted-foreground">
							{t("pythonResearch.modelLab.windows", {
								train: `${run.windows.trainStart}–${run.windows.trainEnd}`,
								selection: `${run.windows.selectionStart}–${run.windows.selectionEnd}`,
								final: `${run.windows.finalStart}–${run.windows.finalEnd}`,
							})}
						</p>
						<p className="break-all font-mono text-xs text-muted-foreground">
							Artifact {run.artifactSha256} · Forecast {run.forecastSha256}
						</p>
						<p className="break-all font-mono text-xs text-muted-foreground">
							{t("pythonResearch.modelLab.binding", {
								binding: run.bindingSha256,
							})}
						</p>
						<p className="break-all font-mono text-xs text-muted-foreground">
							{t("pythonResearch.modelLab.contract", {
								target: `${run.targetId} / ${run.targetHorizonBars} bars`,
								slots: run.inputSlots.join(", "),
								transformation: run.transformationSha256,
								contract: run.forecastContract,
								schema: run.artifactSchema,
								numeric: run.numericRepresentation,
							})}
						</p>
						<p className="break-all font-mono text-xs text-muted-foreground">
							{t("pythonResearch.modelLab.provenance", {
								revision: run.projectRevisionSha256,
								environment: run.environmentSha256,
								snapshot: run.snapshotId,
								universe: run.universeId,
								seed: run.seed,
								resourcePolicy: JSON.stringify(run.resourcePolicy),
							})}
						</p>
						<p role="status">
							{run.testLabelsWithheld
								? t("pythonResearch.modelLab.labelsWithheld")
								: t("pythonResearch.modelLab.labelsExposed")}
						</p>
						<p role="status">
							{run.repeatabilityVerified
								? `${t("pythonResearch.modelLab.repeatable")} (${run.repeatabilityTolerance})`
								: t("pythonResearch.modelLab.repeatabilityFailed")}
						</p>
						<p role="status">
							{t("pythonResearch.modelLab.evidence", {
								state: run.evidenceState,
							})}{" "}
							·{" "}
							{t("pythonResearch.modelLab.repeatabilityState", {
								state: run.repeatabilityState,
							})}
						</p>
						{run.diagnostics.map((diagnostic) => (
							<p key={diagnostic} className="text-destructive" role="alert">
								{t("pythonResearch.modelLab.diagnostics", { value: diagnostic })}
							</p>
						))}
						<p className="break-all font-mono text-xs text-muted-foreground">
							Factor {run.factorDecisionHash} · Dataset {run.factorDatasetId} · Feature{" "}
							{run.featureDatasetId} · lookback {run.factorLookback}
						</p>
					</div>
				) : null}
				{experiment ? (
					<div className="grid gap-2 rounded-md border p-3">
						<p className="break-all font-mono text-xs">
							Experiment {experiment.experimentId}
						</p>
						<p className="break-all font-mono text-xs text-muted-foreground">
							{t("pythonResearch.modelLab.lineage", {
								state: experiment.lineageEvidenceState,
								parent: experiment.parentDecisionId ?? "none",
							})}
						</p>
						<p className="break-all font-mono text-xs text-muted-foreground">
							{t("pythonResearch.modelLab.binding", {
								binding: experiment.bindingSha256,
							})}
						</p>
						{!selectionReady ? (
							<p className="text-muted-foreground" role="status">
								{t("pythonResearch.modelLab.selectionBlocked")}
							</p>
						) : null}
						{experiment.trials.map((trial) => (
							<div
								key={trial.trialId}
								className="flex flex-wrap items-center gap-2 border-t pt-2 first:border-0 first:pt-0"
							>
								<span className="min-w-12">α={trial.alpha}</span>
								<Badge
									variant="outline"
									data-status={trial.status}
									title={trial.status}
								>
									{t(`pythonResearch.modelLab.trialStatus.${trial.status}`, {
										defaultValue: trial.status,
									})}
								</Badge>
								<span className="text-muted-foreground">
									{trial.attemptIds.length} attempt(s)
								</span>
								{trial.selectionMetric !== undefined ? (
									<span className="text-muted-foreground">
										{t("pythonResearch.modelLab.selectionMetric", {
											metric: trial.selectionMetric,
										})}
									</span>
								) : null}
								<p className="basis-full break-all font-mono text-xs text-muted-foreground">
									{t("pythonResearch.modelLab.trial", { trial: trial.trialId })} ·{" "}
									{t("pythonResearch.modelLab.evidence", { state: trial.evidenceState })}{" "}
									·{" "}
									{t("pythonResearch.modelLab.repeatabilityState", {
										state: trial.repeatabilityState,
									})}
								</p>
								<p className="basis-full break-all font-mono text-xs text-muted-foreground">
									{t("pythonResearch.modelLab.successfulAttempt", {
										attempt: trial.successfulAttemptId ?? "—",
									})}
								</p>
								<p className="basis-full break-all font-mono text-xs text-muted-foreground">
									{t("pythonResearch.modelLab.candidateArtifact", {
										artifact: trial.candidateArtifactSha256 ?? "—",
									})}
								</p>
								<p className="basis-full text-muted-foreground" role="status">
									{t("pythonResearch.modelLab.noDownstreamDataset")}
								</p>
								{trial.attemptIds.map((attemptId) => (
									<code key={attemptId} className="basis-full break-all text-xs">
										Attempt {attemptId}
									</code>
								))}
								{trial.diagnostics.map((diagnostic) => (
									<p
										key={diagnostic}
										className="basis-full text-destructive"
										role="alert"
									>
										{t("pythonResearch.modelLab.diagnostics", { value: diagnostic })}
									</p>
								))}
								{failureAttemptsForTrial(trial).map((attempt) => (
									<div
										key={`failure:${attempt.attemptId}`}
										className="flex flex-wrap gap-2"
									>
										<Button
											type="button"
											size="sm"
											variant="outline"
											disabled={trial.status !== "registered"}
											loading={busy === `fail:${attempt.attemptId}`}
											onClick={() => void retainFailure(trial, attempt.attemptId)}
										>
											{t("pythonResearch.modelLab.retainFailure")}
										</Button>
										<Button
											type="button"
											size="sm"
											variant="outline"
											disabled={trial.status === "completed"}
											loading={busy === `retry:${attempt.attemptId}`}
											onClick={() => void retryTrial(trial, attempt.attemptId)}
										>
											{t("pythonResearch.modelLab.retryTrial")}
										</Button>
									</div>
								))}
								<Button
									type="button"
									size="sm"
									variant="outline"
									disabled={trial.status !== "registered"}
									loading={busy === trial.trialId}
									onClick={() => void completeTrial(trial)}
								>
									{t("pythonResearch.modelLab.completeTrial")}
								</Button>
								<Button
									type="button"
									size="sm"
									variant="outline"
									disabled={!selectionReady || trial.status !== "completed"}
									loading={busy === `select:${trial.trialId}`}
									onClick={() => void selectTrial(trial)}
								>
									{t("pythonResearch.modelLab.select")}
								</Button>
							</div>
						))}
					</div>
				) : null}
				{decision ? (
					<div className="flex flex-wrap items-center gap-2 rounded-md border p-3">
						<span>
							{t("pythonResearch.modelLab.decision", {
								alpha: decision.selectedAlpha,
							})}
						</span>
						<code className="break-all text-xs">{decision.decisionId}</code>
						<p className="basis-full break-all font-mono text-xs text-muted-foreground">
							{t("pythonResearch.modelLab.decisionIdentity", {
								binding: decision.bindingSha256,
								revision: decision.projectRevisionSha256,
								environment: decision.environmentSha256,
								input: decision.inputEvidenceSha256,
								seed: decision.seed,
								artifact: decision.candidateArtifactSha256,
							})}
						</p>
						<p className="basis-full text-muted-foreground">
							{t("pythonResearch.modelLab.evidence", {
								state: decision.evidenceState,
							})}
						</p>
						<Button
							type="button"
							size="sm"
							onClick={() => void evaluateFinal()}
							loading={busy === "final"}
						>
							{t("pythonResearch.modelLab.finalEvaluate")}
						</Button>
					</div>
				) : null}
				{report ? (
					<p className="break-all" role="status">
						{t("pythonResearch.modelLab.report", {
							mse: report.meanSquaredError,
							mae: report.meanAbsoluteError,
						})}{" "}
						· {report.reportId} ·{" "}
						{t("pythonResearch.modelLab.reportEvidence", {
							state: report.evidenceState,
							artifact: report.artifactSha256,
							forecast: report.forecastSha256,
							dataset: report.forecastDatasetSha256,
						})}
					</p>
				) : null}
			</CardContent>
		</Card>
	);
}
