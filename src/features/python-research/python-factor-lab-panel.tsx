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

const PROJECTS_CHANGED_EVENT = "adaq:python-projects-changed";

type FactorRun = {
	attemptId: string;
	candidateHash?: string;
	projectId: string;
	fixtureSha256: string;
	lookbacks: number[];
	defaultLookback: number;
	rowsPerTrial: number;
	availableRows: Record<string, number>;
	repeatability: Record<
		string,
		{ firstOutputSha256: string; replayOutputSha256: string; exact: boolean }
	>;
	familyId?: string;
	trialIds: string[];
	datasetIds: string[];
	reportHashes: string[];
	promotionPolicyHash?: string;
	promotionProtocolHash?: string;
	promotionDecisionHash?: string;
	selectedTrialId?: string;
	selectionHash?: string;
	promotionState?: "rejected" | "research-validated" | "component-eligible";
	synthetic: boolean;
	selectionRequired: boolean;
	promotionRequired: boolean;
};

type PromotionState = "rejected" | "research-validated" | "component-eligible";

type FactorSelection = {
	selectedTrialId: string;
	selectionHash: string;
	promotionProtocolHash: string;
};

export function PythonFactorLabPanel({ userId }: { userId: string }) {
	const { t } = useTranslation();
	const [project, setProject] = useState<Project>();
	const [environment, setEnvironment] = useState<Environment>();
	const [run, setRun] = useState<FactorRun>();
	const [selectedTrialId, setSelectedTrialId] = useState("");
	const [promotionState, setPromotionState] =
		useState<PromotionState>("research-validated");
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState("");

	const refreshProject = useCallback(async () => {
		if (!isTauriRuntime()) return;
		try {
			const projects = await invoke<Project[]>("project_list", { userId });
			const current = projects.find(
				(item) => item.projectId === "py-factor-cross-sectional-momentum",
			);
			setProject(current);
			setEnvironment(
				current
					? ((await invoke<Environment | null>("environment_for_project", {
							request: { userId, projectId: current.projectId },
						})) ?? undefined)
					: undefined,
			);
		} catch (reason) {
			setError(String(reason));
		}
	}, [userId]);

	useEffect(() => {
		void refreshProject();
	}, [refreshProject]);

	useEffect(() => {
		const refresh = () => void refreshProject();
		window.addEventListener(PROJECTS_CHANGED_EVENT, refresh);
		return () => window.removeEventListener(PROJECTS_CHANGED_EVENT, refresh);
	}, [refreshProject]);

	const execute = async () => {
		if (!project?.revisionSha256 || !environment) {
			setError(t("pythonResearch.factorLab.freezeAndPrepareRequired"));
			return;
		}
		setBusy(true);
		setError("");
		try {
			const nextRun = await invoke<FactorRun>("python_factor_demo", {
				request: {
					userId,
					projectId: project.projectId,
					projectRevisionSha256: project.revisionSha256,
					environmentSha256: environment.environmentSha256,
				},
			});
			setRun(nextRun);
			setSelectedTrialId(nextRun.trialIds[1] ?? nextRun.trialIds[0] ?? "");
		} catch (reason) {
			setError(String(reason));
		} finally {
			setBusy(false);
		}
	};

	const selectTrial = async () => {
		if (
			!run?.candidateHash ||
			!run.familyId ||
			!run.promotionPolicyHash ||
			!selectedTrialId
		)
			return;
		setBusy(true);
		setError("");
		try {
			const selection = await invoke<FactorSelection>(
				"python_factor_trial_select",
				{
					request: {
						userId,
						candidateHash: run.candidateHash,
						familyId: run.familyId,
						trialId: selectedTrialId,
						policyHash: run.promotionPolicyHash,
					},
				},
			);
			setRun((current) =>
				current
					? {
							...current,
							selectedTrialId: selection.selectedTrialId,
							selectionHash: selection.selectionHash,
							promotionProtocolHash: selection.promotionProtocolHash,
							selectionRequired: false,
						}
					: current,
			);
		} catch (reason) {
			setError(String(reason));
		} finally {
			setBusy(false);
		}
	};

	const promote = async () => {
		if (!run?.candidateHash || !run.selectedTrialId) return;
		setBusy(true);
		setError("");
		try {
			const promotion = await invoke<{
				decisionHash: string;
				state: FactorRun["promotionState"];
			}>("python_factor_promote", {
				request: {
					userId,
					candidateHash: run.candidateHash,
					trialId: run.selectedTrialId,
					state: promotionState,
				},
			});
			setRun((current) =>
				current
					? {
							...current,
							promotionDecisionHash: promotion.decisionHash,
							promotionState: promotion.state,
							promotionRequired: false,
						}
					: current,
			);
		} catch (reason) {
			setError(String(reason));
		} finally {
			setBusy(false);
		}
	};

	return (
		<Card className="mb-4">
			<CardHeader>
				<div className="flex flex-wrap items-start justify-between gap-3">
					<div>
						<CardTitle>{t("pythonResearch.factorLab.title")}</CardTitle>
						<CardDescription>
							{t("pythonResearch.factorLab.description")}
						</CardDescription>
					</div>
					<Button
						type="button"
						size="sm"
						onClick={() => void execute()}
						loading={busy}
						disabled={!isTauriRuntime() || !project?.revisionSha256 || !environment}
					>
						{t("pythonResearch.factorLab.run")}
					</Button>
				</div>
			</CardHeader>
			<CardContent className="grid gap-3 text-sm">
				{project && environment ? (
					<p className="break-all font-mono text-xs text-muted-foreground">
						Environment {environment.environmentSha256}
					</p>
				) : null}
				{error ? (
					<p className="text-destructive" role="alert">
						{error}
					</p>
				) : null}
				{run ? (
					<>
						<div className="flex flex-wrap items-center gap-2">
							<Badge variant="secondary">
								{t("pythonResearch.factorLab.synthetic")}
							</Badge>
							<code>{run.projectId}</code>
							<span className="text-muted-foreground">
								{run.rowsPerTrial} rows / trial
							</span>
						</div>
						<div className="grid gap-2 md:grid-cols-3">
							{run.lookbacks.map((lookback) => {
								const repeatability = run.repeatability[String(lookback)];
								return (
									<div key={lookback} className="grid gap-1 rounded-md border p-3">
										<span>
											lookback={lookback}
											{lookback === run.defaultLookback ? " · default" : ""}
										</span>
										<span className="text-muted-foreground">
											{run.availableRows[String(lookback)]} available
										</span>
										<Badge variant={repeatability?.exact ? "secondary" : "destructive"}>
											{repeatability?.exact
												? t("pythonResearch.factorLab.repeatable")
												: t("pythonResearch.factorLab.divergent")}
										</Badge>
									</div>
								);
							})}
						</div>
						<p className="break-all font-mono text-xs text-muted-foreground">
							Fixture {run.fixtureSha256}
						</p>
						{run.candidateHash ? (
							<p className="break-all font-mono text-xs text-muted-foreground">
								Candidate {run.candidateHash}
							</p>
						) : null}
						<p role="status">
							{run.selectionRequired
								? t("pythonResearch.factorLab.selectionRequired")
								: null}{" "}
							{run.promotionRequired
								? t("pythonResearch.factorLab.promotionRequired")
								: null}
						</p>
						{run.candidateHash && run.trialIds.length > 0 ? (
							<div className="flex flex-wrap items-center gap-2">
								<select
									className="h-9 rounded-md border bg-background px-2 text-sm"
									value={selectedTrialId}
									onChange={(event) => setSelectedTrialId(event.target.value)}
									disabled={busy || Boolean(run.selectedTrialId)}
								>
									{run.trialIds.map((trialId, index) => (
										<option key={trialId} value={trialId}>
											lookback={run.lookbacks[index] ?? index}
										</option>
									))}
								</select>
								<Button
									type="button"
									size="sm"
									onClick={() => void selectTrial()}
									loading={busy}
									disabled={
										busy ||
										Boolean(run.selectedTrialId) ||
										!selectedTrialId ||
										!run.familyId ||
										!run.promotionPolicyHash
									}
								>
									{t("pythonResearch.factorLab.recordSelection")}
								</Button>
							</div>
						) : null}
						{run.selectedTrialId && !run.promotionDecisionHash ? (
							<div className="flex flex-wrap items-center gap-2">
								<select
									className="h-9 rounded-md border bg-background px-2 text-sm"
									value={promotionState}
									onChange={(event) =>
										setPromotionState(event.target.value as PromotionState)
									}
								>
									<option value="research-validated">
										{t("pythonResearch.factorLab.researchValidated")}
									</option>
									<option value="rejected">
										{t("pythonResearch.factorLab.rejected")}
									</option>
									<option value="component-eligible">
										{t("pythonResearch.factorLab.componentEligible")}
									</option>
								</select>
								<Button
									type="button"
									size="sm"
									onClick={() => void promote()}
									loading={busy}
									disabled={busy}
								>
									{t("pythonResearch.factorLab.recordPromotion")}
								</Button>
							</div>
						) : null}
						{run.promotionDecisionHash ? (
							<p className="break-all font-mono text-xs text-muted-foreground">
								{t("pythonResearch.factorLab.promotionRecorded")}{" "}
								{run.promotionDecisionHash}
							</p>
						) : null}
					</>
				) : null}
			</CardContent>
		</Card>
	);
}
