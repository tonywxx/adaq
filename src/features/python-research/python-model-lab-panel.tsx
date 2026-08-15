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
import { useCallback, useEffect, useState } from "react";
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
	selectionMetric?: number;
	evidenceState: string;
};

type Experiment = {
	experimentId: string;
	trials: Trial[];
};

type Decision = {
	decisionId: string;
	selectedTrialId: string;
	selectedAlpha: number;
};

type Report = {
	reportId: string;
	meanSquaredError: number;
	meanAbsoluteError: number;
};

type ResearchAttempt = {
	attemptId: string;
	projectId: string;
	status: "pending" | "running" | "completed" | "failed" | "cancelled";
	queueSequence: number;
	failureCode?: string;
	diagnostic?: string;
	progressCompleted?: number;
	progressTotal?: number;
};

const PROJECTS_CHANGED_EVENT = "adaq:python-projects-changed";

const afterPaint = () =>
	new Promise<void>((resolve) => {
		if (typeof requestAnimationFrame === "undefined") return resolve();
		requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
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

	const refreshProjects = useCallback(async () => {
		if (!isTauriRuntime()) return;
		try {
			const [current, decisionPage] = await Promise.all([
				invoke<Project[]>("project_list", { userId }),
				invoke<FactorDecisionPage>("factor_decision_library", {
					request: { userId, page: 1, pageSize: 50 },
				}),
			]);
			setProjects(current);
			setFactorDecisions(
				decisionPage.items.filter((item) => item.decision.state !== "rejected"),
			);
			const modelProject = current.find(
				(project) => project.projectId === "py-model-qlib-ridge-return",
			);
			setProjectId(modelProject?.projectId ?? "");
			if (modelProject) {
				setEnvironment(
					(await invoke<Environment | null>("environment_for_project", {
						request: { userId, projectId: modelProject.projectId },
					})) ?? undefined,
				);
			} else {
				setEnvironment(undefined);
			}
		} catch (reason) {
			setError(String(reason));
		}
	}, [userId]);

	const refreshAttempts = useCallback(async () => {
		if (!isTauriRuntime()) return;
		try {
			setAttempts(await invoke<ResearchAttempt[]>("attempt_list", { userId }));
		} catch (reason) {
			setError(String(reason));
		}
	}, [userId]);

	useEffect(() => {
		void refreshProjects();
	}, [refreshProjects]);

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
		await afterPaint();
		try {
			setRun(
				await invoke<ModelRun>("model_demo_run", {
					request: {
						userId,
						projectId,
						projectRevisionSha256: project.revisionSha256,
						environmentSha256: environment.environmentSha256,
						factorDecisionHash,
						alpha: 1,
					},
				}),
			);
		} catch (reason) {
			setError(String(reason));
		} finally {
			setBusy("");
		}
	};

	const registerGrid = async () => {
		if (!run) return;
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
			setError(t("pythonResearch.modelLab.freezeRequired"));
			return;
		}
		setBusy("register");
		setError("");
		await afterPaint();
		try {
			setExperiment(
				await invoke<Experiment>("model_experiment_register", {
					request: {
						userId,
						projectRevisionSha256: revision,
						environmentSha256: environment.environmentSha256,
						inputEvidenceSha256: run.inputEvidenceSha256,
						factorDecisionHash,
						seed: run.seed,
					},
				}),
			);
		} catch (reason) {
			setError(String(reason));
		} finally {
			setBusy("");
		}
	};

	const completeTrial = async (trial: Trial) => {
		if (!experiment) return;
		setBusy(trial.trialId);
		setError("");
		await afterPaint();
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
			setRun(trialRun);
			setExperiment(
				await invoke<Experiment>("model_trial_complete", {
					request: {
						userId,
						experimentId: experiment.experimentId,
						trialId: trial.trialId,
						attemptId: trialRun.attemptId,
						selectionMetric,
					},
				}),
			);
		} catch (reason) {
			setError(String(reason));
		} finally {
			setBusy("");
		}
	};

	const selectTrial = async (trial: Trial) => {
		if (!experiment || trial.status !== "completed") return;
		setBusy(`select:${trial.trialId}`);
		setError("");
		try {
			setDecision(
				await invoke<Decision>("model_selection_record", {
					request: {
						userId,
						experimentId: experiment.experimentId,
						trialId: trial.trialId,
					},
				}),
			);
		} catch (reason) {
			setError(String(reason));
		} finally {
			setBusy("");
		}
	};

	const evaluateFinal = async () => {
		if (!decision) return;
		setBusy("final");
		setError("");
		try {
			setReport(
				await invoke<Report>("model_final_evaluate", {
					request: { userId, decisionId: decision.decisionId },
				}),
			);
		} catch (reason) {
			setError(String(reason));
		} finally {
			setBusy("");
		}
	};

	const updateAttempt = async (
		attemptId: string,
		action: "cancel" | "retry",
	) => {
		setBusy(`${attemptId}:${action}`);
		setError("");
		await afterPaint();
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
				setRun(
					await invoke<ModelRun>("model_demo_run", {
						request: {
							userId,
							projectId,
							projectRevisionSha256: project.revisionSha256,
							environmentSha256: environment.environmentSha256,
							factorDecisionHash,
							alpha: run?.alpha ?? 1,
						},
					}),
				);
			} else {
				await invoke("attempt_cancel", { request: { userId, attemptId } });
			}
			await refreshAttempts();
		} catch (reason) {
			setError(String(reason));
		} finally {
			setBusy("");
		}
	};

	const modelAttempts = attempts.filter(
		(attempt) => attempt.projectId === "py-model-qlib-ridge-return",
	);

	return (
		<Card className="mt-4">
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
						onChange={(event) => setFactorDecisionHash(event.target.value)}
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
								<Badge variant="outline">{attempt.status}</Badge>
								<span className="text-muted-foreground">#{attempt.queueSequence}</span>
								{attempt.progressTotal ? (
									<span className="text-muted-foreground">
										{attempt.progressCompleted ?? 0}/{attempt.progressTotal}
									</span>
								) : null}
								<div className="ml-auto flex gap-2">
									{attempt.status === "pending" || attempt.status === "running" ? (
										<Button
											type="button"
											size="sm"
											variant="outline"
											onClick={() => void updateAttempt(attempt.attemptId, "cancel")}
											loading={busy === `${attempt.attemptId}:cancel`}
										>
											{t("pythonResearch.projects.cancel")}
										</Button>
									) : null}
									{attempt.status === "failed" || attempt.status === "cancelled" ? (
										<Button
											type="button"
											size="sm"
											variant="outline"
											onClick={() => void updateAttempt(attempt.attemptId, "retry")}
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
						{experiment.trials.map((trial) => (
							<div
								key={trial.trialId}
								className="flex flex-wrap items-center gap-2 border-t pt-2 first:border-0 first:pt-0"
							>
								<span className="min-w-12">α={trial.alpha}</span>
								<Badge variant="outline">{trial.status}</Badge>
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
									disabled={trial.status !== "completed"}
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
						· {report.reportId}
					</p>
				) : null}
			</CardContent>
		</Card>
	);
}
