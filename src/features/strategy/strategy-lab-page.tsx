import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { useMarketSessionStore } from "@/lib/market-session";
import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

type StrategyScope = "single-instrument" | "portfolio";
type StrategyValue =
	| { type: "decimal"; value: string }
	| { type: "integer"; value: number };

type FactorInput = {
	decisionId: string;
	decisionHash: string;
	candidateHash: string;
	outputName: string;
	packageArchiveSha256: string;
	packageWasmSha256: string;
	componentId: string;
	componentVersion: string;
	featurePlanHash: string;
	contextHash: string;
	snapshotId: string;
	universeId: string;
	market: string;
	venue: string;
};

type ModelInput = {
	qualificationReportId: string;
	decisionId: string;
	finalEvaluationReportId: string;
	artifactSha256: string;
	transformationSha256: string;
	packageArchiveSha256: string;
	packageWasmSha256: string;
	componentId: string;
	componentVersion: string;
	modelProfile: string;
	exporterId: string;
	sdkVersion: string;
	abiVersion: string;
	runtimeIdentity: string;
	inputSlots: string[];
	outputName: string;
	targetId: string;
	targetHorizonBars: number;
	forecastContract: string;
};

type InputSlot = {
	alias: string;
	inputType: "factor-score" | "forecast-signal";
	binding: {
		kind: "factor" | "model";
		[key: string]: unknown;
	};
};

type Node = {
	nodeId: string;
	operation: string;
	inputAliases: string[];
	parameters: Record<string, StrategyValue>;
	outputAlias: string;
};

type Draft = {
	candidateId?: string;
	scope: StrategyScope;
	definition: {
		schemaVersion: "adaq:strategy-candidate@1";
		catalogVersion: "adaq:strategy-operations@1";
		inputSlots: InputSlot[];
		nodes: Node[];
		output:
			| { kind: "target-decision"; nodeId: string }
			| { kind: "portfolio-target"; nodeId: string };
	};
};

type Catalog = {
	factorInputs: FactorInput[];
	modelInputs: ModelInput[];
};

type Diagnostic = { code: string; path: string };
type Preflight = {
	attemptId: string;
	candidateId: string;
	nextRevision: number;
	status: "ready-to-create" | "rejected" | "published";
	diagnostics: Diagnostic[];
};
type Candidate = {
	candidateId: string;
	userId: string;
	scope: StrategyScope;
	state: "draft" | "frozen-revision";
	eligible: boolean;
	revisions: Array<{
		revision: {
			revision: number;
			scope: StrategyScope;
			definition: Draft["definition"];
			revisionHash: string;
			semanticContext: {
				featurePlanHash: string;
				researchContextHash: string;
				snapshotId: string;
				universeId: string;
				market: string;
				venue: string;
				inputEvidenceHashes: string[];
			};
			createdAtMs: number;
			createdByAttemptId: string;
		};
		eligible: boolean;
		staleReason?: string;
	}>;
	attempts: Array<{
		attemptId: string;
		status: "ready-to-create" | "rejected" | "published";
		diagnostics: Diagnostic[];
	}>;
};

type QualificationAttempt = {
	attemptId: string;
	candidateId: string;
	candidateRevision: number;
	status: "running" | "failed" | "ready-for-review";
	candidateRevisionHash: string;
	package?: Record<string, unknown>;
	context: Record<string, unknown>;
	backtestRunId?: string;
	validationProtocolId?: string;
	validationReportId?: string;
	diagnostics: Array<{ stage: string; code: string; message: string }>;
};

type Qualification = {
	qualificationId: string;
	attemptId: string;
	candidateId: string;
	candidateRevision: number;
	candidateRevisionHash: string;
	backtestRunId: string;
	validationProtocolId: string;
	validationReportId: string;
	gate12Eligible: boolean;
	gate12ContinuationRequired: boolean;
};

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

function factorSourceKey(input: FactorInput) {
	return `${input.decisionId}:${input.outputName}`;
}

function modelSourceKey(input: ModelInput) {
	return input.qualificationReportId;
}

function factorBinding(input: FactorInput) {
	return {
		kind: "factor" as const,
		decisionId: input.decisionId,
		decisionHash: input.decisionHash,
		candidateHash: input.candidateHash,
		outputName: input.outputName,
		packageArchiveSha256: input.packageArchiveSha256,
		packageWasmSha256: input.packageWasmSha256,
		componentId: input.componentId,
		componentVersion: input.componentVersion,
	};
}

function modelBinding(input: ModelInput) {
	return {
		kind: "model" as const,
		qualificationReportId: input.qualificationReportId,
		decisionId: input.decisionId,
		finalEvaluationReportId: input.finalEvaluationReportId,
		artifactSha256: input.artifactSha256,
		transformationSha256: input.transformationSha256,
		packageArchiveSha256: input.packageArchiveSha256,
		packageWasmSha256: input.packageWasmSha256,
		componentId: input.componentId,
		componentVersion: input.componentVersion,
		modelProfile: input.modelProfile,
		exporterId: input.exporterId,
		sdkVersion: input.sdkVersion,
		abiVersion: input.abiVersion,
		runtimeIdentity: input.runtimeIdentity,
		inputSlots: input.inputSlots,
		outputName: input.outputName,
		targetId: input.targetId,
		targetHorizonBars: input.targetHorizonBars,
		forecastContract: input.forecastContract,
	};
}

function buildDraft(
	scope: StrategyScope,
	factor: FactorInput | undefined,
	model: ModelInput | undefined,
	candidateId?: string,
): Draft | undefined {
	if (!factor || !model) return undefined;
	const inputSlots: InputSlot[] = [
		{
			alias: "factor-score",
			inputType: "factor-score",
			binding: factorBinding(factor),
		},
		{
			alias: "forecast-signal",
			inputType: "forecast-signal",
			binding: modelBinding(model),
		},
	];
	const combine: Node = {
		nodeId: "combine-score",
		operation: "weighted-sum",
		inputAliases: ["factor-score", "forecast-signal"],
		parameters: { "forecast-weight": { type: "decimal", value: "0.7" } },
		outputAlias: "combined-score",
	};
	if (scope === "single-instrument") {
		return {
			candidateId,
			scope,
			definition: {
				schemaVersion: "adaq:strategy-candidate@1",
				catalogVersion: "adaq:strategy-operations@1",
				inputSlots,
				nodes: [{ ...combine, outputAlias: "target-decision" }],
				output: { kind: "target-decision", nodeId: "combine-score" },
			},
		};
	}
	return {
		candidateId,
		scope,
		definition: {
			schemaVersion: "adaq:strategy-candidate@1",
			catalogVersion: "adaq:strategy-operations@1",
			inputSlots,
			nodes: [
				combine,
				{
					nodeId: "select-top",
					operation: "top-n",
					inputAliases: ["combined-score"],
					parameters: { "top-n": { type: "integer", value: 3 } },
					outputAlias: "selected-target",
				},
				{
					nodeId: "reserve-cash",
					operation: "cash-reserve",
					inputAliases: ["selected-target"],
					parameters: { "cash-reserve": { type: "decimal", value: "0.1" } },
					outputAlias: "portfolio-target",
				},
			],
			output: { kind: "portfolio-target", nodeId: "reserve-cash" },
		},
	};
}

function formatError(error: unknown) {
	return String(error);
}

export function StrategyLabPage() {
	const { t } = useTranslation();
	const userId = useMarketSessionStore((state) => state.userId);
	const [catalog, setCatalog] = useState<Catalog>();
	const [candidates, setCandidates] = useState<Candidate[]>([]);
	const [scope, setScope] = useState<StrategyScope>("portfolio");
	const [factorKey, setFactorKey] = useState("");
	const [modelKey, setModelKey] = useState("");
	const [draft, setDraft] = useState<Draft>();
	const [draftDirty, setDraftDirty] = useState(false);
	const [preflight, setPreflight] = useState<Preflight>();
	const [busy, setBusy] = useState<
		| "loading"
		| "preflight"
		| "create"
		| "retry"
		| "qualification"
		| "qualify"
		| ""
	>("");
	const [message, setMessage] = useState("");
	const [error, setError] = useState("");

	const factor = useMemo(
		() =>
			catalog?.factorInputs.find((input) => factorSourceKey(input) === factorKey),
		[catalog, factorKey],
	);
	const model = useMemo(
		() =>
			catalog?.modelInputs.find((input) => modelSourceKey(input) === modelKey),
		[catalog, modelKey],
	);

	const refresh = useCallback(async () => {
		if (!userId) return;
		setBusy("loading");
		setError("");
		await afterPaint();
		try {
			const [nextCatalog, nextCandidates] = await Promise.all([
				invoke("strategy_candidate_catalog") as Promise<Catalog>,
				invoke("strategy_candidate_list") as Promise<Candidate[]>,
			]);
			setCatalog(nextCatalog);
			setCandidates(nextCandidates);
			if (nextCatalog.factorInputs[0]) {
				setFactorKey(
					(current) => current || factorSourceKey(nextCatalog.factorInputs[0]),
				);
			}
			if (nextCatalog.modelInputs[0]) {
				setModelKey(
					(current) => current || modelSourceKey(nextCatalog.modelInputs[0]),
				);
			}
		} catch (requestError) {
			setError(formatError(requestError));
		} finally {
			setBusy("");
		}
	}, [userId]);

	useEffect(() => {
		let active = true;
		void refresh().catch(
			(requestError) => active && setError(formatError(requestError)),
		);
		return () => {
			active = false;
		};
	}, [refresh]);

	useEffect(() => {
		setDraft((current) => buildDraft(scope, factor, model, current?.candidateId));
		setPreflight(undefined);
	}, [factor, model, scope]);

	const updateParameter = (
		nodeIndex: number,
		name: string,
		value: StrategyValue,
	) => {
		setDraft((current) => {
			if (!current) return current;
			const nodes = current.definition.nodes.map((node, index) =>
				index === nodeIndex
					? {
							...node,
							parameters: { ...node.parameters, [name]: value },
						}
					: node,
			);
			return {
				...current,
				definition: { ...current.definition, nodes },
			};
		});
		setPreflight(undefined);
		setDraftDirty(true);
	};

	const runPreflight = async () => {
		if (!draft) return;
		setBusy("preflight");
		setError("");
		setMessage("");
		try {
			const result = (await invoke("strategy_candidate_preflight", {
				request: { draft },
			})) as Preflight;
			setPreflight(result);
			setDraftDirty(false);
			setMessage(
				result.status === "ready-to-create"
					? t("strategyLab.ready")
					: t("strategyLab.rejected"),
			);
		} catch (requestError) {
			setError(t("strategyLab.error", { error: formatError(requestError) }));
		} finally {
			setBusy("");
		}
	};

	const createRevision = async () => {
		if (preflight?.status !== "ready-to-create") return;
		setBusy("create");
		setError("");
		try {
			const candidate = (await invoke("strategy_candidate_create", {
				request: { attemptId: preflight.attemptId },
			})) as Candidate;
			setCandidates((current) => [
				...current.filter((item) => item.candidateId !== candidate.candidateId),
				candidate,
			]);
			setDraft((current) =>
				current ? { ...current, candidateId: candidate.candidateId } : current,
			);
			setPreflight(undefined);
			setDraftDirty(false);
			setMessage(t("strategyLab.published"));
		} catch (requestError) {
			setError(t("strategyLab.error", { error: formatError(requestError) }));
		} finally {
			setBusy("");
		}
	};

	const retryPreflight = async () => {
		if (preflight?.status !== "rejected") return;
		setBusy("retry");
		setError("");
		try {
			const result = (await invoke("strategy_candidate_retry", {
				request: { attemptId: preflight.attemptId },
			})) as Preflight;
			setPreflight(result);
			setDraftDirty(false);
			setMessage(
				result.status === "ready-to-create"
					? t("strategyLab.ready")
					: t("strategyLab.rejected"),
			);
			await refresh();
		} catch (requestError) {
			setError(t("strategyLab.error", { error: formatError(requestError) }));
		} finally {
			setBusy("");
		}
	};

	const runQualification = async (request: Record<string, unknown>) => {
		setBusy("qualification");
		setError("");
		setMessage("");
		try {
			const attempt = (await invoke("strategy_qualification_run", {
				request,
			})) as QualificationAttempt;
			setMessage(
				attempt.status === "ready-for-review"
					? t("strategyLab.qualification.ready")
					: t("strategyLab.qualification.failed"),
			);
			return attempt;
		} catch (requestError) {
			setError(
				t("strategyLab.qualification.error", { error: formatError(requestError) }),
			);
			return undefined;
		} finally {
			setBusy("");
		}
	};

	const qualifyAttempt = async (attemptId: string) => {
		setBusy("qualify");
		setError("");
		try {
			const qualification = (await invoke("strategy_qualification_qualify", {
				request: { attemptId },
			})) as Qualification;
			setMessage(t("strategyLab.qualification.qualified"));
			return qualification;
		} catch (requestError) {
			setError(
				t("strategyLab.qualification.error", { error: formatError(requestError) }),
			);
			return undefined;
		} finally {
			setBusy("");
		}
	};

	const readyToCreate = preflight?.status === "ready-to-create" && busy === "";
	const currentCandidate = candidates.find(
		(candidate) => candidate.candidateId === draft?.candidateId,
	);
	const lifecycle =
		busy === "preflight" || busy === "retry"
			? "validating"
			: preflight?.status === "rejected"
				? "rejected"
				: preflight?.status === "ready-to-create"
					? "ready"
					: draftDirty
						? "draft"
						: currentCandidate?.state === "frozen-revision"
							? currentCandidate.eligible
								? "published"
								: "stale"
							: "draft";

	return (
		<main className="mx-auto flex w-full max-w-7xl flex-col gap-6 p-4 md:p-8">
			<header className="space-y-2">
				<p className="font-mono text-xs uppercase tracking-[0.2em] text-muted-foreground">
					{t("strategyLab.eyebrow")}
				</p>
				<h1 className="text-3xl font-semibold tracking-tight">
					{t("strategyLab.title")}
				</h1>
				<p className="max-w-3xl text-muted-foreground">
					{t("strategyLab.description")}
				</p>
				<div className="flex items-center gap-2 text-sm" aria-live="polite">
					<span className="text-muted-foreground">{t("strategyLab.lifecycle")}</span>
					<Badge>{t(`strategyLab.status.${lifecycle}`)}</Badge>
				</div>
			</header>

			{message ? (
				<p
					className="rounded-md border border-primary/30 bg-primary/5 p-3 text-sm"
					aria-live="polite"
				>
					{message}
				</p>
			) : null}
			{error ? (
				<p
					className="rounded-md border border-destructive/40 bg-destructive/5 p-3 text-sm text-destructive"
					role="alert"
				>
					{error}
				</p>
			) : null}

			<Card>
				<CardHeader>
					<CardTitle>{t("strategyLab.orderedDefinition")}</CardTitle>
					<CardDescription>{t("strategyLab.noInputs")}</CardDescription>
				</CardHeader>
				<CardContent className="space-y-6">
					<div className="grid gap-4 md:grid-cols-3">
						<div className="space-y-2">
							<Label htmlFor="strategy-scope">{t("strategyLab.scope")}</Label>
							<select
								id="strategy-scope"
								className="h-9 w-full rounded-md border bg-background px-3 text-sm"
								value={scope}
								onChange={(event) => {
									setScope(event.target.value as StrategyScope);
									setDraftDirty(true);
									setDraft(
										buildDraft(
											event.target.value as StrategyScope,
											factor,
											model,
											undefined,
										),
									);
								}}
							>
								<option value="portfolio">{t("strategyLab.portfolio")}</option>
								<option value="single-instrument">
									{t("strategyLab.singleInstrument")}
								</option>
							</select>
						</div>
						<div className="space-y-2">
							<Label htmlFor="strategy-factor">{t("strategyLab.factorInput")}</Label>
							<select
								id="strategy-factor"
								className="h-9 w-full rounded-md border bg-background px-3 text-sm"
								value={factorKey}
								onChange={(event) => {
									setFactorKey(event.target.value);
									setDraftDirty(true);
								}}
								disabled={busy === "loading"}
							>
								<option value="">{t("strategyLab.selectFactor")}</option>
								{catalog?.factorInputs.map((input) => (
									<option key={factorSourceKey(input)} value={factorSourceKey(input)}>
										{input.outputName} · {input.decisionHash.slice(0, 12)}
									</option>
								))}
							</select>
						</div>
						<div className="space-y-2">
							<Label htmlFor="strategy-model">{t("strategyLab.modelInput")}</Label>
							<select
								id="strategy-model"
								className="h-9 w-full rounded-md border bg-background px-3 text-sm"
								value={modelKey}
								onChange={(event) => {
									setModelKey(event.target.value);
									setDraftDirty(true);
								}}
								disabled={busy === "loading"}
							>
								<option value="">{t("strategyLab.selectModel")}</option>
								{catalog?.modelInputs.map((input) => (
									<option key={modelSourceKey(input)} value={modelSourceKey(input)}>
										{input.qualificationReportId.slice(0, 12)} · {input.componentVersion}
									</option>
								))}
							</select>
						</div>
					</div>

					{draft ? (
						<>
							<div className="rounded-lg border bg-muted/20 p-4">
								<div className="mb-3 flex flex-wrap items-center justify-between gap-2">
									<div>
										<p className="font-medium">{t("strategyLab.nodes")}</p>
										<p className="text-sm text-muted-foreground">
											{draft.definition.output.kind === "portfolio-target"
												? t("strategyLab.portfolioTarget")
												: t("strategyLab.targetDecision")}
										</p>
									</div>
									<Badge variant="outline">{draft.scope}</Badge>
								</div>
								<ol className="space-y-3" aria-label={t("strategyLab.nodes")}>
									{draft.definition.nodes.map((node, index) => (
										<li
											key={node.nodeId}
											className="grid gap-3 rounded-md border bg-background p-3 md:grid-cols-[auto_1fr_1fr_1fr]"
										>
											<span className="font-mono text-sm text-muted-foreground">
												{index + 1}
											</span>
											<div>
												<p className="text-xs text-muted-foreground">ID</p>
												<code className="text-sm">{node.nodeId}</code>
											</div>
											<div>
												<p className="text-xs text-muted-foreground">
													{t("strategyLab.operation")}
												</p>
												<code className="text-sm">{node.operation}</code>
											</div>
											<div>
												<p className="text-xs text-muted-foreground">
													{t("strategyLab.inputs")}
												</p>
												<code className="text-sm">{node.inputAliases.join(", ")}</code>
											</div>
											<div className="md:col-start-2">
												<p className="text-xs text-muted-foreground">
													{t("strategyLab.parameters")}
												</p>
												{Object.entries(node.parameters).map(([name, value]) => (
													<label className="mt-1 flex items-center gap-2 text-sm" key={name}>
														<span className="font-mono text-xs">{name}</span>
														<select
															className="h-8 rounded-md border bg-background px-2 text-sm"
															value={String(value.value)}
															onChange={(event) =>
																updateParameter(
																	index,
																	name,
																	value.type === "integer"
																		? { type: "integer", value: Number(event.target.value) }
																		: { type: "decimal", value: event.target.value },
																)
															}
														>
															{(name === "forecast-weight"
																? ["0.5", "0.7"]
																: name === "top-n"
																	? ["3", "5"]
																	: ["0", "0.1"]
															).map((allowed) => (
																<option key={allowed} value={allowed}>
																	{allowed}
																</option>
															))}
														</select>
													</label>
												))}
											</div>
											<div className="md:col-start-3">
												<p className="text-xs text-muted-foreground">Output</p>
												<code className="text-sm">{node.outputAlias}</code>
											</div>
										</li>
									))}
								</ol>
							</div>
							<div className="grid gap-3 md:grid-cols-2">
								<SourceIdentity title={t("strategyLab.factorInput")} input={factor} />
								<SourceIdentity title={t("strategyLab.modelInput")} input={model} />
							</div>
							<div className="flex flex-wrap gap-3">
								<Button type="button" onClick={runPreflight} disabled={busy !== ""}>
									{busy === "preflight"
										? t("strategyLab.preflighting")
										: t("strategyLab.preflight")}
								</Button>
								<Button
									type="button"
									variant="secondary"
									onClick={createRevision}
									disabled={!readyToCreate}
								>
									{busy === "create"
										? t("strategyLab.creating")
										: t("strategyLab.create")}
								</Button>
								{preflight?.status === "rejected" ? (
									<Button
										type="button"
										variant="outline"
										onClick={retryPreflight}
										disabled={busy !== ""}
									>
										{busy === "retry"
											? t("strategyLab.preflighting")
											: t("strategyLab.retry")}
									</Button>
								) : null}
							</div>
							{preflight ? <PreflightStatus preflight={preflight} /> : null}
						</>
					) : (
						<p className="rounded-md border border-dashed p-4 text-sm text-muted-foreground">
							{t("strategyLab.noInputs")}
						</p>
					)}
				</CardContent>
			</Card>

			<QualificationPanel
				candidates={candidates}
				userId={userId}
				busy={busy}
				onRun={runQualification}
				onQualify={qualifyAttempt}
			/>

			<section aria-labelledby="strategy-history-heading">
				<div className="mb-3 flex items-center justify-between gap-3">
					<h2 id="strategy-history-heading" className="text-xl font-semibold">
						{t("strategyLab.history")}
					</h2>
					<Button
						type="button"
						variant="ghost"
						onClick={() => void refresh()}
						disabled={busy !== ""}
					>
						{t("researchContext.retry")}
					</Button>
				</div>
				{candidates.length === 0 ? (
					<p className="text-sm text-muted-foreground">
						{t("strategyLab.noCandidates")}
					</p>
				) : (
					<div className="grid gap-4">
						{candidates.map((candidate) => (
							<Card key={candidate.candidateId}>
								<CardHeader className="pb-3">
									<div className="flex flex-wrap items-center justify-between gap-2">
										<CardTitle className="text-base">
											{t("strategyLab.candidate", { id: candidate.candidateId })}
										</CardTitle>
										<Badge
											variant={
												candidate.state === "draft"
													? "outline"
													: candidate.eligible
														? "default"
														: "destructive"
											}
										>
											{candidate.state === "draft"
												? t("strategyLab.status.draft")
												: candidate.eligible
													? t("strategyLab.eligible")
													: t("strategyLab.stale")}
										</Badge>
									</div>
									<CardDescription>{candidate.scope}</CardDescription>
								</CardHeader>
								<CardContent className="space-y-3">
									<div className="grid gap-2">
										{candidate.revisions.map((item) => (
											<div
												className="rounded-md border p-3 text-sm"
												key={item.revision.revision}
											>
												<div className="flex flex-wrap justify-between gap-2">
													<span>
														{t("strategyLab.revision", { revision: item.revision.revision })}
													</span>
													<Badge variant={item.eligible ? "outline" : "destructive"}>
														{item.eligible
															? t("strategyLab.eligible")
															: t("strategyLab.stale")}
													</Badge>
												</div>
												<p className="mt-2 text-muted-foreground">
													{t("strategyLab.revisionHash")}
												</p>
												<code className="break-all text-xs">
													{item.revision.revisionHash}
												</code>
												<details className="mt-3 rounded-md border p-2">
													<summary className="cursor-pointer font-medium">
														{t("strategyLab.inspectRevision")}
													</summary>
													<div className="mt-2 space-y-2 text-xs">
														<p>
															{t("strategyLab.catalogVersion")}:{" "}
															{item.revision.definition.catalogVersion}
														</p>
														<p>
															{t("strategyLab.inputSlots")}:{" "}
															{item.revision.definition.inputSlots
																.map((slot) => `${slot.alias} (${slot.inputType})`)
																.join(", ")}
														</p>
														<p>
															{t("strategyLab.semanticContext")}:{" "}
															{item.revision.semanticContext.featurePlanHash} ·{" "}
															{item.revision.semanticContext.researchContextHash} ·{" "}
															{item.revision.semanticContext.snapshotId} ·{" "}
															{item.revision.semanticContext.universeId}
														</p>
														<pre className="max-h-64 overflow-auto whitespace-pre-wrap break-all rounded bg-muted p-2">
															{JSON.stringify(item.revision.definition, null, 2)}
														</pre>
													</div>
												</details>
												{item.staleReason ? (
													<p className="mt-2 text-xs text-destructive">
														{t("strategyLab.staleReason")} {item.staleReason}
													</p>
												) : null}
											</div>
										))}
									</div>
									{candidate.attempts.length > 0 ? (
										<div className="rounded-md border border-amber-500/30 bg-amber-500/5 p-3 text-sm">
											<p className="font-medium">{t("strategyLab.attempts")}</p>
											<ul aria-label={t("strategyLab.diagnostics")}>
												{candidate.attempts.map((attempt) => (
													<li className="mt-2" key={attempt.attemptId}>
														<span>
															{t(
																`strategyLab.status.${attempt.status === "ready-to-create" ? "ready" : attempt.status === "published" ? "published" : "rejected"}`,
															)}
														</span>{" "}
														<code className="text-xs">{attempt.attemptId}</code>
														{attempt.diagnostics.map((diagnostic) => (
															<p
																className="mt-1 font-mono text-xs"
																key={`${attempt.attemptId}-${diagnostic.code}-${diagnostic.path}`}
															>
																{t("strategyLab.hostRejected")} {diagnostic.code} ·{" "}
																{diagnostic.path}
															</p>
														))}
													</li>
												))}
											</ul>
										</div>
									) : null}
								</CardContent>
							</Card>
						))}
					</div>
				)}
			</section>
		</main>
	);
}

function SourceIdentity({
	title,
	input,
}: {
	title: string;
	input: FactorInput | ModelInput | undefined;
}) {
	return (
		<details className="rounded-md border p-3 text-sm">
			<summary className="cursor-pointer font-medium">{title}</summary>
			<pre className="mt-3 max-h-48 overflow-auto whitespace-pre-wrap break-all text-xs text-muted-foreground">
				{input ? JSON.stringify(input, null, 2) : "—"}
			</pre>
		</details>
	);
}

function PreflightStatus({ preflight }: { preflight: Preflight }) {
	const { t } = useTranslation();
	return (
		<div className="rounded-md border p-3 text-sm" aria-live="polite">
			<div className="flex flex-wrap items-center justify-between gap-2">
				<span>
					{preflight.status === "ready-to-create"
						? t("strategyLab.ready")
						: t("strategyLab.rejected")}
				</span>
				<code className="text-xs">
					{t("strategyLab.attempt", { id: preflight.attemptId })}
				</code>
			</div>
			<p className="mt-1 text-muted-foreground">
				{t("strategyLab.revision", { revision: preflight.nextRevision || "—" })}
			</p>
			{preflight.diagnostics.map((diagnostic) => (
				<p
					className="mt-2 font-mono text-xs text-destructive"
					key={`${diagnostic.code}-${diagnostic.path}`}
				>
					{t("strategyLab.hostRejected")} {diagnostic.code} · {diagnostic.path}
				</p>
			))}
		</div>
	);
}

function QualificationPanel({
	candidates,
	userId,
	busy,
	onRun,
	onQualify,
}: {
	candidates: Candidate[];
	userId: string | null;
	busy: string;
	onRun: (
		request: Record<string, unknown>,
	) => Promise<QualificationAttempt | undefined>;
	onQualify: (attemptId: string) => Promise<Qualification | undefined>;
}) {
	const { t } = useTranslation();
	const [candidateKey, setCandidateKey] = useState("");
	const [snapshotId, setSnapshotId] = useState("");
	const [universeSnapshotId, setUniverseSnapshotId] = useState("");
	const [selectionStart, setSelectionStart] = useState("");
	const [selectionEnd, setSelectionEnd] = useState("");
	const [finalStart, setFinalStart] = useState("");
	const [finalEnd, setFinalEnd] = useState("");
	const [signalDatasetIds, setSignalDatasetIds] = useState("");
	const [attempt, setAttempt] = useState<QualificationAttempt>();
	const [qualification, setQualification] = useState<Qualification>();
	const [reviewed, setReviewed] = useState(false);

	const options = useMemo(
		() =>
			candidates.flatMap((candidate) =>
				candidate.revisions
					.filter((item) => item.eligible)
					.map((item) => ({
						key: `${candidate.candidateId}:${item.revision.revision}`,
						candidate,
						item,
					})),
			),
		[candidates],
	);
	const selected = useMemo(
		() => options.find((option) => option.key === candidateKey),
		[candidateKey, options],
	);
	const modelSlots = useMemo(
		() =>
			selected?.item.revision.definition.inputSlots.filter(
				(slot) => slot.inputType === "forecast-signal",
			) ?? [],
		[selected],
	);

	useEffect(() => {
		if (!selected && options[0]) setCandidateKey(options[0].key);
	}, [options, selected]);

	useEffect(() => {
		if (!selected) return;
		const context = selected.item.revision.semanticContext;
		setSnapshotId(context.snapshotId);
		setUniverseSnapshotId("");
		setReviewed(false);
		setAttempt(undefined);
		setQualification(undefined);
	}, [selected]);

	const integer = (value: string) => {
		const number = Number(value);
		return Number.isSafeInteger(number) ? number : undefined;
	};
	const datasetIds = Array.from(
		new Set(
			signalDatasetIds
				.split(",")
				.map((value) => value.trim())
				.filter(Boolean),
		),
	);
	const isPortfolio = selected?.item.revision.scope === "portfolio";
	const canRun = Boolean(
		userId &&
			selected &&
			snapshotId.trim() &&
			universeSnapshotId.trim() &&
			integer(selectionStart) !== undefined &&
			integer(selectionEnd) !== undefined &&
			integer(finalStart) !== undefined &&
			integer(finalEnd) !== undefined &&
			(isPortfolio
				? datasetIds.length > 0 && modelSlots.length > 0
				: datasetIds.length === modelSlots.length),
	);

	const run = async () => {
		if (!selected || !canRun) return;
		const selectionStartMs = integer(selectionStart);
		const selectionEndMs = integer(selectionEnd);
		const finalStartMs = integer(finalStart);
		const finalEndMs = integer(finalEnd);
		if (
			selectionStartMs === undefined ||
			selectionEndMs === undefined ||
			finalStartMs === undefined ||
			finalEndMs === undefined
		)
			return;
		const nextAttempt = await onRun({
			userId,
			candidateId: selected.candidate.candidateId,
			candidateRevision: selected.item.revision.revision,
			snapshotId: snapshotId.trim(),
			universeSnapshotId: universeSnapshotId.trim(),
			selectionWindow: {
				startTimeMs: selectionStartMs,
				endTimeMs: selectionEndMs,
			},
			finalWindow: { startTimeMs: finalStartMs, endTimeMs: finalEndMs },
			signalInstances: isPortfolio
				? datasetIds.flatMap((datasetId) =>
						modelSlots.map((slot) => ({
							slot: slot.alias,
							datasetId,
							signalName: String(
								(slot.binding as { outputName?: string }).outputName ?? "",
							),
						})),
					)
				: modelSlots.map((slot, index) => ({
						slot: slot.alias,
						datasetId: datasetIds[index],
						signalName: String(
							(slot.binding as { outputName?: string }).outputName ?? "",
						),
					})),
			initialQuoteAllocation: "10000",
			executionProfile: {
				makerFeeRate: "0.0008",
				takerFeeRate: "0.001",
				adverseSlippageRate: "0",
				rebalanceThreshold: "0",
				priceIncrement: "0.0001",
				quantityIncrement: "0.0001",
				minimumQuantity: "0.0001",
				riskFreeRate: "0",
				fillPolicy: "taker",
			},
			riskPolicy: {
				policyId: "gate-11-default",
				maxInstrumentWeight: "1",
				maxTurnover: null,
			},
			seed: 0,
		});
		setAttempt(nextAttempt);
		setQualification(undefined);
		setReviewed(false);
	};

	const qualify = async () => {
		if (!attempt || !reviewed) return;
		setQualification(await onQualify(attempt.attemptId));
	};

	return (
		<Card>
			<CardHeader>
				<CardTitle>{t("strategyLab.qualification.title")}</CardTitle>
				<CardDescription>
					{t("strategyLab.qualification.description")}
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-4">
				<div className="grid gap-4 md:grid-cols-2">
					<div className="space-y-2 md:col-span-2">
						<Label htmlFor="qualification-candidate">
							{t("strategyLab.qualification.candidateRevision")}
						</Label>
						<select
							id="qualification-candidate"
							className="h-9 w-full rounded-md border bg-background px-3 text-sm"
							value={candidateKey}
							onChange={(event) => setCandidateKey(event.target.value)}
						>
							<option value="">{t("strategyLab.qualification.selectRevision")}</option>
							{options.map(({ key, candidate, item }) => (
								<option key={key} value={key}>
									{candidate.candidateId.slice(0, 12)} · {item.revision.revision} ·{" "}
									{candidate.scope}
								</option>
							))}
						</select>
					</div>
					<Field
						id="qualification-snapshot"
						label={t("strategyLab.qualification.snapshot")}
						value={snapshotId}
						onChange={setSnapshotId}
					/>
					<Field
						id="qualification-universe"
						label={t("strategyLab.qualification.universeSnapshot")}
						value={universeSnapshotId}
						onChange={setUniverseSnapshotId}
					/>
				</div>
				<div className="grid gap-4 md:grid-cols-4">
					<Field
						id="qualification-selection-start"
						label={t("strategyLab.qualification.selectionStart")}
						value={selectionStart}
						onChange={setSelectionStart}
						type="number"
					/>
					<Field
						id="qualification-selection-end"
						label={t("strategyLab.qualification.selectionEnd")}
						value={selectionEnd}
						onChange={setSelectionEnd}
						type="number"
					/>
					<Field
						id="qualification-final-start"
						label={t("strategyLab.qualification.finalStart")}
						value={finalStart}
						onChange={setFinalStart}
						type="number"
					/>
					<Field
						id="qualification-final-end"
						label={t("strategyLab.qualification.finalEnd")}
						value={finalEnd}
						onChange={setFinalEnd}
						type="number"
					/>
				</div>
				<Field
					id="qualification-signals"
					label={t("strategyLab.qualification.signalDatasets")}
					value={signalDatasetIds}
					onChange={setSignalDatasetIds}
					placeholder={t("strategyLab.qualification.signalDatasetsHint")}
				/>
				<div className="flex flex-wrap gap-3">
					<Button
						type="button"
						onClick={() => void run()}
						disabled={!canRun || busy !== ""}
					>
						{busy === "qualification"
							? t("strategyLab.qualification.running")
							: t("strategyLab.qualification.run")}
					</Button>
					{attempt?.status === "ready-for-review" ? (
						<label className="flex items-center gap-2 text-sm">
							<input
								type="checkbox"
								checked={reviewed}
								onChange={(event) => setReviewed(event.target.checked)}
							/>
							{t("strategyLab.qualification.reviewed")}
						</label>
					) : null}
					<Button
						type="button"
						variant="secondary"
						onClick={() => void qualify()}
						disabled={
							attempt?.status !== "ready-for-review" || !reviewed || busy !== ""
						}
					>
						{busy === "qualify"
							? t("strategyLab.qualification.qualifying")
							: t("strategyLab.qualification.qualify")}
					</Button>
				</div>
				{attempt ? (
					<div className="rounded-md border p-3 text-sm" aria-live="polite">
						<div className="flex flex-wrap justify-between gap-2">
							<span>{t("strategyLab.qualification.attemptStatus")}</span>
							<Badge
								variant={
									attempt.status === "ready-for-review" ? "default" : "destructive"
								}
							>
								{attempt.status}
							</Badge>
						</div>
						<code className="break-all text-xs">{attempt.attemptId}</code>
						{attempt.diagnostics.map((diagnostic) => (
							<p
								className="mt-2 font-mono text-xs text-destructive"
								key={`${diagnostic.stage}-${diagnostic.code}`}
							>
								{diagnostic.code}: {diagnostic.message}
							</p>
						))}
						{attempt.status === "ready-for-review" ? (
							<details open className="mt-3 rounded-md bg-muted/40 p-3">
								<summary className="cursor-pointer font-medium">
									{t("strategyLab.qualification.evidence")}
								</summary>
								<pre className="mt-2 max-h-96 overflow-auto whitespace-pre-wrap break-all text-xs">
									{JSON.stringify(attempt, null, 2)}
								</pre>
							</details>
						) : null}
					</div>
				) : null}
				{qualification ? (
					<div className="rounded-md border border-primary/30 bg-primary/5 p-3 text-sm">
						<p className="font-medium">{t("strategyLab.qualification.qualified")}</p>
						<code className="break-all text-xs">{qualification.qualificationId}</code>
						<p className="mt-2 text-muted-foreground">
							{qualification.backtestRunId} · {qualification.validationReportId}
						</p>
						<details className="mt-3 rounded-md bg-muted/40 p-3">
							<summary className="cursor-pointer font-medium">
								{t("strategyLab.qualification.evidence")}
							</summary>
							<pre className="mt-2 max-h-96 overflow-auto whitespace-pre-wrap break-all text-xs">
								{JSON.stringify(qualification, null, 2)}
							</pre>
						</details>
					</div>
				) : null}
			</CardContent>
		</Card>
	);
}

function Field({
	id,
	label,
	value,
	onChange,
	type = "text",
	placeholder,
}: {
	id: string;
	label: string;
	value: string;
	onChange: (value: string) => void;
	type?: "text" | "number";
	placeholder?: string;
}) {
	return (
		<div className="space-y-2">
			<Label htmlFor={id}>{label}</Label>
			<input
				id={id}
				className="h-9 w-full rounded-md border bg-background px-3 text-sm"
				type={type}
				value={value}
				placeholder={placeholder}
				onChange={(event) => onChange(event.target.value)}
			/>
		</div>
	);
}
