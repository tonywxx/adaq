import { invoke } from "@tauri-apps/api/core";
import type { LibraryComponent } from "@/features/components/component-library";
import {
	DatabaseIcon,
	GitBranchIcon,
	GavelIcon,
	LockKeyholeIcon,
	PlusIcon,
	SigmaIcon,
	Trash2Icon,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "@tanstack/react-router";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
	Breadcrumb,
	BreadcrumbItem,
	BreadcrumbLink,
	BreadcrumbList,
	BreadcrumbPage,
	BreadcrumbSeparator,
} from "@/components/ui/breadcrumb";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
	ResearchContextPreflight,
	localizedFactorContextError,
	useResearchEvidenceContext,
	type ResearchEvidenceProjection,
} from "@/features/research/research-context-preflight";
import { formatDateTime, formatNumber } from "@/lib/i18n";
import { useMarketSessionStore } from "@/lib/market-session";
import { createFactorAdapter, type FactorAdapter } from "./factor-adapter";
import {
	finiteGridTrialCount,
	factorHash,
	factorString,
	isTerminalFactorAttempt,
	isGridWithinLimit,
	shortFactorHash,
} from "./factor-data";
import type {
	FactorComponentCandidateView,
	FactorComponentQualificationView,
	FactorCandidateView,
	FactorAttemptView,
	FactorDatasetRow,
	FactorDatasetView,
	FactorDecisionView,
	FactorJson,
	FactorLineageView,
	FactorMetricCatalogView,
	FactorPolicyView,
	FactorReportView,
	M12Eligibility,
} from "./factor-types";
import { PythonProjectsPanel } from "@/features/python-research/python-projects-panel";
import { PythonFactorLabPanel } from "@/features/python-research/python-factor-lab-panel";
import { CandidatesWorkspace } from "./candidates-workspace";
import { AttemptsPanel } from "./factor-attempts-panel";
import { Detail, Field } from "./factor-form-fields";
import { useFactorPage } from "./factor-workspace-data";
import {
	EmptyState,
	ErrorState,
	EvidenceJson,
	FactorAttemptStatusBadge,
	Feedback,
	jsonText,
	lines,
	localizedFactorCode,
	localizedFactorAttemptCode,
	localizedFactorError,
	localizedFactorReason,
	newUuid,
	optionalNumber,
	PageControls,
	LoadingState,
	textAt,
	valueAt,
} from "./factor-workspace-support";

type FactorTab =
	| "families"
	| "candidates"
	| "datasets"
	| "evaluations"
	| "decisions";

function useFactorAdapter(providedAdapter?: FactorAdapter) {
	return useMemo(
		() => providedAdapter ?? createFactorAdapter(invoke),
		[providedAdapter],
	);
}

export function applyFactorContext(
	draft: FactorJson,
	context?: ResearchEvidenceProjection | null,
) {
	if (!context?.featureDataset || !context.universeId) return draft;
	const marketContext =
		draft.marketContext &&
		typeof draft.marketContext === "object" &&
		!Array.isArray(draft.marketContext)
			? (draft.marketContext as FactorJson)
			: {};
	return {
		...draft,
		observationRange: {
			startTimeMs: context.rangeStartMs,
			endTimeMs: context.rangeEndMs,
		},
		marketContext: {
			...marketContext,
			assetClass: context.market,
			venue: context.venue,
			pointInTimeUniverseId: context.universeId,
		},
	};
}

export function factorCandidatesForContext(
	candidates: FactorCandidateView[],
	context?: ResearchEvidenceProjection | null,
) {
	const featureDataset = context?.featureDataset;
	if (!context?.universeId || !featureDataset) return [];
	return candidates.filter((candidate) => {
		const predecessor = candidate.predecessor;
		const predecessorDataset = predecessor?.featureDataset;
		return Boolean(
			predecessor &&
				predecessor.contextRevision === context.contextRevision &&
				predecessor.contextHash === context.contextHash &&
				predecessor.market === context.market &&
				predecessor.venue === context.venue &&
				predecessor.rangeStartMs === context.rangeStartMs &&
				predecessor.rangeEndMs === context.rangeEndMs &&
				predecessor.snapshotId === context.snapshotId &&
				predecessor.universeId === context.universeId &&
				predecessorDataset?.datasetId === featureDataset.datasetId &&
				predecessorDataset.featurePlanHash === featureDataset.featurePlanHash &&
				predecessorDataset.requestHash === featureDataset.requestHash &&
				predecessorDataset.contentSha256 === featureDataset.contentSha256,
		);
	});
}

export function FactorsPage({
	adapter: providedAdapter,
}: {
	adapter?: FactorAdapter;
} = {}) {
	const { t } = useTranslation();
	const userId = useMarketSessionStore((state) => state.userId);
	const adapter = useFactorAdapter(providedAdapter);
	const [tab, setTab] = useState<FactorTab>("families");
	const factorContextQuery = useResearchEvidenceContext(userId ?? "");

	useEffect(() => {
		const previousTitle = document.title;
		document.title = `${t("factors.title")} · AdaQ`;
		return () => {
			document.title = previousTitle;
		};
	}, [t]);

	return (
		<main
			className="mx-auto min-w-0 max-w-7xl flex-1 space-y-5 p-4 md:p-6"
			data-route="factors"
		>
			<Breadcrumb aria-label={t("factors.breadcrumb")}>
				<BreadcrumbList>
					<BreadcrumbItem>
						<BreadcrumbLink asChild>
							<Link to="/">{t("nav.home")}</Link>
						</BreadcrumbLink>
					</BreadcrumbItem>
					<BreadcrumbSeparator />
					<BreadcrumbItem>
						<BreadcrumbPage>{t("factors.title")}</BreadcrumbPage>
					</BreadcrumbItem>
				</BreadcrumbList>
			</Breadcrumb>

			<header className="space-y-2">
				<div className="flex flex-wrap items-center gap-2">
					<SigmaIcon className="size-5 text-primary" aria-hidden="true" />
					<p className="font-mono text-xs tracking-[0.18em] text-muted-foreground uppercase">
						M11 · {t("factors.eyebrow")}
					</p>
				</div>
				<h1 className="text-2xl font-semibold tracking-tight md:text-3xl">
					{t("factors.title")}
				</h1>
				<p className="max-w-4xl text-sm leading-6 text-muted-foreground">
					{t("factors.description")}
				</p>
				<div className="rounded-lg border border-amber-500/30 bg-amber-500/5 p-3 text-sm text-muted-foreground">
					{t("factors.historicalEvidenceNote")}
				</div>
			</header>

			{userId ? (
				<ResearchContextPreflight userId={userId} stage="factors" />
			) : null}
			{userId ? <PythonProjectsPanel userId={userId} kind="factor" /> : null}
			{userId ? <PythonFactorLabPanel userId={userId} /> : null}

			{!userId ? (
				<LoadingState label={t("factors.loading")} />
			) : (
				<Tabs value={tab} onValueChange={(value) => setTab(value as FactorTab)}>
					<TabsList className="flex h-auto w-full flex-wrap justify-start gap-1">
						<TabsTrigger value="families">{t("factors.tabs.families")}</TabsTrigger>
						<TabsTrigger value="candidates">
							{t("factors.tabs.candidates")}
						</TabsTrigger>
						<TabsTrigger value="datasets">{t("factors.tabs.datasets")}</TabsTrigger>
						<TabsTrigger value="evaluations">
							{t("factors.tabs.evaluations")}
						</TabsTrigger>
						<TabsTrigger value="decisions">{t("factors.tabs.decisions")}</TabsTrigger>
					</TabsList>
					<TabsContent value="families" className="mt-4">
						<FamiliesWorkspace key={userId} userId={userId} adapter={adapter} />
					</TabsContent>
					<TabsContent value="candidates" className="mt-4">
						<CandidatesWorkspace
							key={userId}
							userId={userId}
							adapter={adapter}
							context={factorContextQuery.data}
							contextLoading={factorContextQuery.isPending}
							contextError={factorContextQuery.error}
						/>
					</TabsContent>
					<TabsContent value="datasets" className="mt-4">
						<DatasetsWorkspace
							key={userId}
							userId={userId}
							adapter={adapter}
							context={factorContextQuery.data}
							contextLoading={factorContextQuery.isPending}
							contextError={factorContextQuery.error}
						/>
					</TabsContent>
					<TabsContent value="evaluations" className="mt-4">
						<EvaluationsWorkspace key={userId} userId={userId} adapter={adapter} />
					</TabsContent>
					<TabsContent value="decisions" className="mt-4">
						<DecisionsWorkspace key={userId} userId={userId} adapter={adapter} />
					</TabsContent>
				</Tabs>
			)}
		</main>
	);
}

function FamiliesWorkspace({
	userId,
	adapter,
}: {
	userId: string;
	adapter: FactorAdapter;
}) {
	const { t } = useTranslation();
	const families = useFactorPage(userId, "families", adapter.listFamilies);
	const [lineage, setLineage] = useState<Record<string, FactorLineageView>>({});
	const [lineageLoading, setLineageLoading] = useState<string>();
	const [feedback, setFeedback] = useState<string>();
	const [attemptRefresh, setAttemptRefresh] = useState(0);
	const lineageRequest = useRef(0);

	const inspect = async (familyId: string, trialId: string) => {
		const request = ++lineageRequest.current;
		setLineageLoading(familyId);
		setFeedback(undefined);
		try {
			const details = await adapter.getLineage(userId, trialId);
			if (request !== lineageRequest.current) return;
			setLineage((current) => ({ ...current, [familyId]: details }));
		} catch (error) {
			if (request === lineageRequest.current)
				setFeedback(localizedFactorError(error, t));
		} finally {
			if (request === lineageRequest.current) setLineageLoading(undefined);
		}
	};

	return (
		<div className="space-y-5">
			<Card>
				<CardHeader>
					<CardTitle className="flex items-center gap-2">
						<GitBranchIcon className="size-4" aria-hidden="true" />
						{t("factors.families.heading")}
					</CardTitle>
					<CardDescription>{t("factors.families.description")}</CardDescription>
				</CardHeader>
				<CardContent className="space-y-4">
					{feedback && <Feedback message={feedback} />}
					{families.error ? (
						<ErrorState
							message={localizedFactorError(families.error, t)}
							onRetry={() => void families.load()}
							retryLabel={t("factors.retry")}
							loading={families.loading}
						/>
					) : null}
					{families.loading && !families.data ? (
						<LoadingState label={t("factors.loading")} />
					) : null}
					{families.data && !families.error && families.data.items.length === 0 ? (
						<EmptyState message={t("factors.families.empty")} />
					) : null}
					{families.data && families.data.items.length > 0 ? (
						<div className="space-y-3">
							<div className="max-w-full overflow-x-auto">
								<table className="w-full min-w-[720px] text-sm">
									<caption className="sr-only">{t("factors.families.heading")}</caption>
									<thead>
										<tr className="border-b text-left text-muted-foreground">
											<th scope="col" className="py-2 pr-4">
												{t("factors.common.identity")}
											</th>
											<th scope="col" className="py-2 pr-4">
												{t("factors.common.candidate")}
											</th>
											<th scope="col" className="py-2 pr-4">
												{t("factors.families.trials")}
											</th>
											<th scope="col" className="py-2 pr-4">
												{t("factors.families.lineage")}
											</th>
											<th scope="col" className="py-2 text-right">
												{t("factors.common.actions")}
											</th>
										</tr>
									</thead>
									<tbody>
										{families.data.items.map((item) => {
											const id = textAt(item.family, "familyId");
											const registeredTrialIds = valueAt(
												item.family,
												"registeredTrialIds",
											) as unknown[] | undefined;
											const trialId =
												Array.isArray(registeredTrialIds) && registeredTrialIds[0]
													? factorString(registeredTrialIds[0])
													: id;
											return (
												<tr key={id} className="border-b align-top">
													<td className="py-3 pr-4 font-mono text-xs">{id}</td>
													<td className="py-3 pr-4 font-mono text-xs">
														{shortFactorHash(valueAt(item.family, "rootCandidateHash"))}
													</td>
													<td className="py-3 pr-4">{formatNumber(item.trialCount)}</td>
													<td className="py-3 pr-4 font-mono text-xs">
														{shortFactorHash(item.lineageHash)}
													</td>
													<td className="py-3 text-right">
														<Button
															type="button"
															variant="outline"
															size="sm"
															loading={lineageLoading === id}
															onClick={() => void inspect(id, trialId)}
														>
															{t("factors.families.inspectLineage")}
														</Button>
													</td>
												</tr>
											);
										})}
									</tbody>
								</table>
							</div>
							{families.data.items.map((item) => {
								const details = lineage[textAt(item.family, "familyId")];
								if (!details) return null;
								return (
									<LineageDetails key={textAt(item.family, "familyId")} view={details} />
								);
							})}
							<PageControls
								page={families.data.page}
								total={families.data.total}
								pageSize={families.data.pageSize}
								onPage={(page) => void families.load(page)}
							/>
						</div>
					) : null}
				</CardContent>
			</Card>
			<GridSetup
				userId={userId}
				adapter={adapter}
				onCreated={() => {
					setAttemptRefresh((current) => current + 1);
					void families.load();
				}}
			/>
			<AttemptsPanel
				userId={userId}
				adapter={adapter}
				kind="factor-family-grid"
				refreshKey={attemptRefresh}
			/>
		</div>
	);
}

function LineageDetails({ view }: { view: FactorLineageView }) {
	const { t } = useTranslation();
	const registration = view.registrations[0];
	const protocol = view.protocols[0];
	const relatedFamilies = Array.isArray(valueAt(view.lineage, "familyIds"))
		? (valueAt(view.lineage, "familyIds") as unknown[])
				.map((value) => factorString(value))
				.join(", ")
		: "—";
	return (
		<div className="space-y-3 rounded-lg border bg-muted/10 p-4">
			<div className="flex flex-wrap items-center gap-2">
				<Badge variant="outline">{t("factors.families.lineage")}</Badge>
				<span className="font-mono text-xs">
					{textAt(view.lineage, "lineageHash")}
				</span>
			</div>
			<div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
				<Detail
					label={t("factors.families.target")}
					value={textAt(registration, "target")}
				/>
				<Detail
					label={t("factors.families.context")}
					value={[
						textAt(registration, "marketContext.venue"),
						textAt(registration, "marketContext.barInterval"),
					].join(" · ")}
				/>
				<Detail
					label={t("factors.families.relatedFamilies")}
					value={relatedFamilies}
					mono
				/>
				<Detail
					label={t("factors.families.holmPopulation")}
					value={formatNumber(view.trials.length)}
				/>
			</div>
			<div className="grid gap-3 lg:grid-cols-2">
				<EvidenceJson
					label={t("factors.families.searchSpace")}
					value={{
						parameterSetHashes: view.registrations.map((item) =>
							textAt(item, "parameterSetHash"),
						),
						horizons: valueAt(protocol, "horizonBars"),
						folds: valueAt(protocol, "windows"),
					}}
				/>
				<EvidenceJson
					label={t("factors.families.protocolEvidence")}
					value={view.protocols}
				/>
			</div>
			<div className="max-w-full overflow-x-auto">
				<table className="w-full min-w-[720px] text-xs">
					<thead>
						<tr className="border-b text-left text-muted-foreground">
							<th scope="col" className="py-2 pr-3">
								{t("factors.families.trial")}
							</th>
							<th scope="col" className="py-2 pr-3">
								{t("factors.families.status")}
							</th>
							<th scope="col" className="py-2 pr-3">
								{t("factors.families.report")}
							</th>
							<th scope="col" className="py-2 pr-3">
								{t("factors.families.adjusted")}
							</th>
							<th scope="col" className="py-2">
								{t("factors.families.diagnostic")}
							</th>
						</tr>
					</thead>
					<tbody>
						{view.trials.map((trial) => (
							<tr key={textAt(trial, "trialId")} className="border-b">
								<td className="py-2 pr-3 font-mono">
									{shortFactorHash(textAt(trial, "trialId"), 12)}
								</td>
								<td className="py-2 pr-3">
									<Badge
										variant={
											textAt(trial, "status") === "completed" ? "secondary" : "outline"
										}
									>
										{localizedFactorCode(textAt(trial, "status"), t)}
									</Badge>
								</td>
								<td className="py-2 pr-3 font-mono">
									{shortFactorHash(valueAt(trial, "reportHash"))}
								</td>
								<td className="py-2 pr-3 font-mono">
									{
										metricObservation(
											(valueAt(trial, "holmAdjusted") ?? {}) as FactorJson,
										).value
									}
									<span className="ml-2 text-muted-foreground">
										{localizedFactorReason(
											metricObservation(
												(valueAt(trial, "holmAdjusted") ?? {}) as FactorJson,
											).reason,
											t,
										)}
									</span>
								</td>
								<td className="py-2 text-muted-foreground">
									{textAt(trial, "diagnostic")}
								</td>
							</tr>
						))}
					</tbody>
				</table>
			</div>
			<EvidenceJson label={t("factors.common.rawEvidence")} value={view} />
		</div>
	);
}

function GridSetup({
	userId,
	adapter,
	onCreated,
}: {
	userId: string;
	adapter: FactorAdapter;
	onCreated: () => void;
}) {
	const { t } = useTranslation();
	const [candidateHash, setCandidateHash] = useState("");
	const [baseProtocolHash, setBaseProtocolHash] = useState("");
	const [familyId, setFamilyId] = useState<string>(newUuid());
	const [parentFamilyId, setParentFamilyId] = useState("");
	const [parameterRows, setParameterRows] = useState([
		{ id: newUuid(), name: "lookback", values: "5, 10" },
	]);
	const [context, setContext] = useState({
		venue: "",
		assetClass: "",
		barInterval: "1d",
		priceBasis: "unadjusted",
		valuationCurrency: "",
		universe: "",
	});
	const [range, setRange] = useState({ start: "", end: "" });
	const [busy, setBusy] = useState(false);
	const [feedback, setFeedback] = useState<string>();
	const cardinalities = parameterRows.map(
		(row) => lines(row.values.replaceAll(",", "\n")).length,
	);
	const trialCount = finiteGridTrialCount(cardinalities);

	const register = async () => {
		setBusy(true);
		setFeedback(undefined);
		try {
			if (
				!factorHash(candidateHash) ||
				!factorHash(baseProtocolHash) ||
				!range.start ||
				!range.end ||
				trialCount === null ||
				!isGridWithinLimit(cardinalities)
			)
				throw new Error(t("factors.families.gridInvalid"));
			await adapter.registerGridFamily(userId, {
				familyId,
				candidateHash,
				parentFamilyId: parentFamilyId || null,
				parameters: parameterRows.map((row) => ({
					name: row.name.trim(),
					values: lines(row.values.replaceAll(",", "\n")).map((value) => ({
						text: value,
					})),
				})),
				target: "future-close-return",
				marketContext: {
					venue: context.venue,
					assetClass: context.assetClass,
					barInterval: context.barInterval,
					priceBasis: context.priceBasis,
					valuationCurrency: context.valuationCurrency,
					pointInTimeUniverseId: context.universe,
				},
				pointInTimeUniverseId: context.universe,
				observationRange: {
					startTimeMs: Number(range.start),
					endTimeMs: Number(range.end),
				},
				baseProtocolHash,
				derivationHash: null,
			});
			setFeedback(t("factors.families.gridQueued"));
			setFamilyId(newUuid());
			onCreated();
		} catch (error) {
			setFeedback(localizedFactorError(error, t));
		} finally {
			setBusy(false);
		}
	};

	return (
		<Card>
			<CardHeader>
				<CardTitle className="flex items-center gap-2">
					<PlusIcon className="size-4" aria-hidden="true" />
					{t("factors.families.gridHeading")}
				</CardTitle>
				<CardDescription>{t("factors.families.gridDescription")}</CardDescription>
			</CardHeader>
			<CardContent className="space-y-4">
				<div className="grid gap-3 md:grid-cols-2">
					<Field
						label={t("factors.common.candidateHash")}
						value={candidateHash}
						onChange={setCandidateHash}
						placeholder="64-char SHA-256"
						mono
					/>
					<Field
						label={t("factors.families.baseProtocol")}
						value={baseProtocolHash}
						onChange={setBaseProtocolHash}
						placeholder="64-char SHA-256"
						mono
					/>
					<Field
						label={t("factors.families.familyId")}
						value={familyId}
						onChange={setFamilyId}
						mono
					/>
					<Field
						label={t("factors.families.parentFamily")}
						value={parentFamilyId}
						onChange={setParentFamilyId}
						placeholder={t("factors.common.optional")}
						mono
					/>
				</div>
				<div className="space-y-2">
					<div className="flex items-center justify-between">
						<Label>{t("factors.families.searchSpace")}</Label>
						<span className="text-xs text-muted-foreground">
							{trialCount === null
								? t("factors.families.gridIncomplete")
								: t("factors.families.trialCount", { count: trialCount })}
						</span>
					</div>
					{parameterRows.map((row, index) => (
						<div className="flex flex-wrap gap-2" key={row.id}>
							<Input
								aria-label={t("factors.families.parameterName")}
								value={row.name}
								onChange={(event) =>
									setParameterRows((current) =>
										current.map((item, position) =>
											position === index ? { ...item, name: event.target.value } : item,
										),
									)
								}
								placeholder="lower-kebab-name"
								className="max-w-56"
							/>
							<Input
								aria-label={t("factors.families.parameterValues")}
								value={row.values}
								onChange={(event) =>
									setParameterRows((current) =>
										current.map((item, position) =>
											position === index ? { ...item, values: event.target.value } : item,
										),
									)
								}
								placeholder="5, 10"
								className="min-w-56 flex-1"
							/>
							{parameterRows.length > 1 ? (
								<Button
									type="button"
									variant="ghost"
									size="icon-sm"
									aria-label={t("factors.families.removeParameter")}
									onClick={() =>
										setParameterRows((current) =>
											current.filter((_, position) => position !== index),
										)
									}
								>
									<Trash2Icon aria-hidden="true" />
								</Button>
							) : null}
						</div>
					))}
					<Button
						type="button"
						variant="outline"
						size="sm"
						onClick={() =>
							setParameterRows((current) => [
								...current,
								{ id: newUuid(), name: "parameter", values: "" },
							])
						}
					>
						<PlusIcon aria-hidden="true" />
						{t("factors.families.addParameter")}
					</Button>
				</div>
				<div className="grid gap-3 md:grid-cols-3">
					<Field
						label={t("factors.families.venue")}
						value={context.venue}
						onChange={(value) =>
							setContext((current) => ({ ...current, venue: value }))
						}
					/>
					<Field
						label={t("factors.families.assetClass")}
						value={context.assetClass}
						onChange={(value) =>
							setContext((current) => ({ ...current, assetClass: value }))
						}
					/>
					<Field
						label={t("factors.families.interval")}
						value={context.barInterval}
						onChange={(value) =>
							setContext((current) => ({ ...current, barInterval: value }))
						}
					/>
					<Field
						label={t("factors.families.priceBasis")}
						value={context.priceBasis}
						onChange={(value) =>
							setContext((current) => ({ ...current, priceBasis: value }))
						}
					/>
					<Field
						label={t("factors.families.currency")}
						value={context.valuationCurrency}
						onChange={(value) =>
							setContext((current) => ({ ...current, valuationCurrency: value }))
						}
					/>
					<Field
						label={t("factors.families.universe")}
						value={context.universe}
						onChange={(value) =>
							setContext((current) => ({ ...current, universe: value }))
						}
						mono
					/>
				</div>
				<div className="grid gap-3 md:grid-cols-2">
					<Field
						label={t("factors.families.startMs")}
						value={range.start}
						onChange={(value) =>
							setRange((current) => ({ ...current, start: value }))
						}
						type="number"
					/>
					<Field
						label={t("factors.families.endMs")}
						value={range.end}
						onChange={(value) => setRange((current) => ({ ...current, end: value }))}
						type="number"
					/>
				</div>
				<div className="flex flex-wrap items-center gap-3">
					<Button
						type="button"
						loading={busy}
						loadingText={t("factors.common.saving")}
						onClick={() => void register()}
						disabled={trialCount === null || !isGridWithinLimit(cardinalities)}
					>
						{t("factors.families.registerGrid")}
					</Button>
					<span className="text-xs text-muted-foreground">
						{t("factors.families.gridLimit", { limit: 256 })}
					</span>
				</div>
				<Feedback
					message={feedback}
					tone={feedback === t("factors.families.gridQueued") ? "success" : "error"}
				/>
			</CardContent>
		</Card>
	);
}

function MaterializationStart({
	userId,
	adapter,
	onStarted,
	context,
	contextLoading = false,
	contextError,
}: {
	userId: string;
	adapter: FactorAdapter;
	onStarted: () => void;
	context?: ResearchEvidenceProjection | null;
	contextLoading?: boolean;
	contextError?: unknown;
}) {
	const { t } = useTranslation();
	const [candidateHash, setCandidateHash] = useState("");
	const [seed, setSeed] = useState("");
	const [busy, setBusy] = useState(false);
	const [feedback, setFeedback] = useState<string>();
	const candidates = useFactorPage(userId, "candidates", adapter.listCandidates);
	const compatibleCandidates = useMemo(
		() => factorCandidatesForContext(candidates.data?.items ?? [], context),
		[candidates.data?.items, context],
	);
	const contextReady = Boolean(
		!contextLoading &&
			!contextError &&
			context?.featureDataset &&
			context.universeId,
	);

	useEffect(() => {
		if (!compatibleCandidates.length) {
			setCandidateHash("");
			return;
		}
		setCandidateHash((current) =>
			compatibleCandidates.some(
				(candidate) => textAt(candidate.candidate, "candidateHash") === current,
			)
				? current
				: textAt(compatibleCandidates[0].candidate, "candidateHash", ""),
		);
	}, [compatibleCandidates]);

	const start = async () => {
		setBusy(true);
		setFeedback(undefined);
		try {
			if (!contextReady) {
				throw new Error(t("factors.datasets.materializationContextRequired"));
			}
			if (!candidateHash) {
				throw new Error(t("factors.datasets.materializationCandidateRequired"));
			}
			await adapter.startMaterializationFromContext(
				userId,
				candidateHash,
				optionalNumber(seed) ?? 0,
			);
			setFeedback(t("factors.datasets.materializationStarted"));
			onStarted();
		} catch (error) {
			setFeedback(localizedFactorContextError(error, t));
		} finally {
			setBusy(false);
		}
	};
	return (
		<Card>
			<CardHeader>
				<CardTitle>{t("factors.datasets.materializationHeading")}</CardTitle>
				<CardDescription>
					{t("factors.datasets.materializationDescription")}
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-4">
				<div className="rounded-lg border bg-muted/20 p-3">
					{contextLoading ? (
						<p role="status" className="text-sm text-muted-foreground">
							{t("factors.datasets.materializationContextLoading")}
						</p>
					) : contextError ? (
						<p role="alert" className="text-sm text-destructive">
							{localizedFactorContextError(contextError, t)}
						</p>
					) : !contextReady || !context?.featureDataset ? (
						<p role="status" className="text-sm text-muted-foreground">
							{t("factors.datasets.materializationContextRequired")}
						</p>
					) : (
						<>
							<p className="mb-3 text-sm font-medium">
								{t("factors.datasets.materializationContext")}
							</p>
							<dl className="grid gap-3 text-sm sm:grid-cols-2 lg:grid-cols-3">
								<Detail
									label={t("factors.candidates.contextRevision")}
									value={String(context.contextRevision)}
								/>
								<Detail
									label={t("factors.candidates.contextHash")}
									value={context.contextHash}
									mono
								/>
								<Detail
									label={t("factors.datasets.featureDataset")}
									value={context.featureDataset.datasetId}
									mono
								/>
								<Detail
									label={t("factors.candidates.featurePlanHash")}
									value={context.featureDataset.featurePlanHash}
									mono
								/>
								<Detail
									label={t("factors.datasets.snapshot")}
									value={context.snapshotId}
									mono
								/>
								<Detail
									label={t("factors.datasets.universe")}
									value={context.universeId ?? "—"}
									mono
								/>
								<Detail
									label={t("factors.candidates.range")}
									value={`${context.rangeStartMs} → ${context.rangeEndMs}`}
									mono
								/>
							</dl>
						</>
					)}
				</div>
				<fieldset disabled={!contextReady || busy} className="space-y-4">
					<div className="grid gap-3 md:grid-cols-2">
						<div className="grid gap-1.5">
							<Label htmlFor="factor-materialization-candidate">
								{t("factors.datasets.materializationCandidate")}
							</Label>
							<select
								id="factor-materialization-candidate"
								className="h-9 rounded-md border bg-background px-3 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
								value={candidateHash}
								onChange={(event) => setCandidateHash(event.target.value)}
							>
								<option value="">
									{candidates.loading
										? t("factors.datasets.materializationCandidateLoading")
										: t("factors.datasets.materializationCandidatePlaceholder")}
								</option>
								{compatibleCandidates.map((candidate) => {
									const hash = textAt(candidate.candidate, "candidateHash", "");
									return (
										<option key={hash} value={hash}>
											{candidate.presentation.name} · {shortFactorHash(hash)} ·{" "}
											{textAt(candidate.candidate, "scope")}
										</option>
									);
								})}
							</select>
						</div>
						<Field
							label={t("features.materialization.seed")}
							value={seed}
							onChange={setSeed}
							type="number"
						/>
					</div>
				</fieldset>
				{contextReady && candidates.error ? (
					<p role="alert" className="text-sm text-destructive">
						{localizedFactorError(candidates.error, t)}
					</p>
				) : null}
				{contextReady &&
				!candidates.loading &&
				candidates.data &&
				compatibleCandidates.length === 0 ? (
					<p role="status" className="text-sm text-muted-foreground">
						{t("factors.datasets.materializationCandidateEmpty")}
					</p>
				) : null}
				<div className="flex flex-wrap items-center gap-3">
					<Button
						type="button"
						loading={busy}
						loadingText={t("factors.common.queueing")}
						disabled={!contextReady || !candidateHash || busy}
						onClick={() => void start()}
					>
						{t("factors.datasets.materializationStart")}
					</Button>
					<Feedback
						message={feedback}
						tone={
							feedback === t("factors.datasets.materializationStarted")
								? "success"
								: "error"
						}
					/>
				</div>
			</CardContent>
		</Card>
	);
}

function DatasetsWorkspace({
	userId,
	adapter,
	context,
	contextLoading,
	contextError,
}: {
	userId: string;
	adapter: FactorAdapter;
	context?: ResearchEvidenceProjection | null;
	contextLoading?: boolean;
	contextError?: unknown;
}) {
	const { t } = useTranslation();
	const datasets = useFactorPage(userId, "datasets", adapter.listDatasets);
	const [selected, setSelected] = useState<FactorDatasetView>();
	const [feedback, setFeedback] = useState<string>();
	const [attemptRefresh, setAttemptRefresh] = useState(0);
	const [loadingAction, setLoadingAction] = useState<string>();
	const [deletingId, setDeletingId] = useState<string>();
	const lastFocus = useRef<HTMLElement | null>(null);
	const inspectRequest = useRef(0);
	const inspect = async (item: FactorDatasetView) => {
		const request = ++inspectRequest.current;
		const id = textAt(item.manifest, "datasetId");
		lastFocus.current = document.activeElement as HTMLElement | null;
		setLoadingAction(id);
		setFeedback(undefined);
		try {
			const details = await adapter.getDataset(userId, id);
			if (request === inspectRequest.current) setSelected(details);
		} catch (error) {
			if (request === inspectRequest.current)
				setFeedback(localizedFactorError(error, t));
		} finally {
			if (request === inspectRequest.current) setLoadingAction(undefined);
		}
	};
	const remove = async (item: FactorDatasetView) => {
		const request = ++inspectRequest.current;
		const id = textAt(item.manifest, "datasetId");
		setDeletingId(id);
		setFeedback(undefined);
		try {
			await adapter.deleteDataset(userId, id);
			if (request === inspectRequest.current) {
				setSelected(undefined);
				setFeedback(t("factors.datasets.deleted"));
			}
			await datasets.load();
		} catch (error) {
			if (request === inspectRequest.current)
				setFeedback(localizedFactorError(error, t));
		} finally {
			setDeletingId(undefined);
		}
	};
	useEffect(() => {
		if (!selected) lastFocus.current?.focus();
	}, [selected]);
	return (
		<div className="space-y-5">
			<MaterializationStart
				userId={userId}
				adapter={adapter}
				context={context}
				contextLoading={contextLoading}
				contextError={contextError}
				onStarted={() => setAttemptRefresh((current) => current + 1)}
			/>
			<Card>
				<CardHeader>
					<CardTitle className="flex items-center gap-2">
						<DatabaseIcon className="size-4" aria-hidden="true" />
						{t("factors.datasets.heading")}
					</CardTitle>
					<CardDescription>{t("factors.datasets.description")}</CardDescription>
				</CardHeader>
				<CardContent className="space-y-4">
					{feedback && (
						<Feedback
							message={feedback}
							tone={feedback === t("factors.datasets.deleted") ? "success" : "error"}
						/>
					)}
					{datasets.error ? (
						<ErrorState
							message={localizedFactorError(datasets.error, t)}
							onRetry={() => void datasets.load()}
							retryLabel={t("factors.retry")}
							loading={datasets.loading}
						/>
					) : null}
					{datasets.loading && !datasets.data ? (
						<LoadingState label={t("factors.loading")} />
					) : null}
					{datasets.data && !datasets.error && datasets.data.items.length === 0 ? (
						<EmptyState message={t("factors.datasets.empty")} />
					) : null}
					{datasets.data && datasets.data.items.length > 0 ? (
						<>
							<div className="max-w-full overflow-x-auto">
								<table className="w-full min-w-[820px] text-sm">
									<caption className="sr-only">{t("factors.datasets.heading")}</caption>
									<thead>
										<tr className="border-b text-left text-muted-foreground">
											<th scope="col" className="py-2 pr-4">
												{t("factors.datasets.identity")}
											</th>
											<th scope="col" className="py-2 pr-4">
												{t("factors.datasets.candidate")}
											</th>
											<th scope="col" className="py-2 pr-4">
												{t("factors.datasets.context")}
											</th>
											<th scope="col" className="py-2 pr-4">
												{t("factors.datasets.rows")}
											</th>
											<th scope="col" className="py-2 pr-4">
												{t("factors.datasets.lock")}
											</th>
											<th scope="col" className="py-2 text-right">
												{t("factors.common.actions")}
											</th>
										</tr>
									</thead>
									<tbody>
										{datasets.data.items.map((item) => {
											const id = textAt(item.manifest, "datasetId");
											const locked = item.lockedBy.length > 0;
											return (
												<tr key={id} className="border-b align-top">
													<td className="py-3 pr-4 font-mono text-xs">
														{shortFactorHash(id)}
													</td>
													<td className="py-3 pr-4 font-mono text-xs">
														{shortFactorHash(valueAt(item.manifest, "candidateHash"))}
													</td>
													<td className="py-3 pr-4">
														{textAt(item.manifest, "marketContext.venue")} ·{" "}
														{textAt(item.manifest, "marketContext.barInterval")}
													</td>
													<td className="py-3 pr-4">
														{formatNumber(
															Number(valueAt(item.manifest, "observationCount") ?? 0),
														)}
													</td>
													<td className="py-3 pr-4">
														{locked ? (
															<Badge variant="outline">
																<LockKeyholeIcon aria-hidden="true" />
																{t("factors.common.locked")}
															</Badge>
														) : (
															<Badge variant="secondary">{t("factors.common.unlocked")}</Badge>
														)}
														{locked ? (
															<p className="mt-1 max-w-56 break-words text-xs text-muted-foreground">
																{t("factors.datasets.lockReferences", {
																	references: item.lockedBy.join(", "),
																})}
															</p>
														) : null}
													</td>
													<td className="py-3 text-right">
														<div className="flex justify-end gap-2">
															<Button
																type="button"
																variant="outline"
																size="sm"
																loading={loadingAction === id}
																loadingText={t("factors.loading")}
																onClick={() => void inspect(item)}
															>
																{t("factors.datasets.inspect")}
															</Button>
															<Button
																type="button"
																variant="destructive"
																size="sm"
																loading={deletingId === id}
																loadingText={t("factors.loading")}
																disabled={locked || Boolean(deletingId)}
																aria-label={
																	locked
																		? t("factors.datasets.locked")
																		: t("factors.datasets.delete")
																}
																onClick={() => void remove(item)}
															>
																<Trash2Icon aria-hidden="true" />
																{t("factors.datasets.delete")}
															</Button>
														</div>
													</td>
												</tr>
											);
										})}
									</tbody>
								</table>
							</div>
							<PageControls
								page={datasets.data.page}
								total={datasets.data.total}
								pageSize={datasets.data.pageSize}
								onPage={(page) => void datasets.load(page)}
							/>
						</>
					) : null}
				</CardContent>
			</Card>
			{selected ? (
				<DatasetInspector
					userId={userId}
					adapter={adapter}
					dataset={selected}
					onClose={() => {
						inspectRequest.current += 1;
						setSelected(undefined);
					}}
				/>
			) : null}
			<AttemptsPanel
				userId={userId}
				adapter={adapter}
				kind="factor-materialization"
				refreshKey={attemptRefresh}
			/>
		</div>
	);
}

function DatasetInspector({
	userId,
	adapter,
	dataset,
	onClose,
}: {
	userId: string;
	adapter: FactorAdapter;
	dataset: FactorDatasetView;
	onClose: () => void;
}) {
	const { t } = useTranslation();
	const datasetId = textAt(dataset.manifest, "datasetId");
	const [rows, setRows] = useState<FactorDatasetRow[]>([]);
	const [offset, setOffset] = useState(0);
	const [nextOffset, setNextOffset] = useState<number | null>();
	const [total, setTotal] = useState(0);
	const [loading, setLoading] = useState(true);
	const [error, setError] = useState<string>();
	const [instrument, setInstrument] = useState("");
	const requestVersion = useRef(0);
	const loadRows = useCallback(
		async (next = 0, filter = "") => {
			const version = ++requestVersion.current;
			setLoading(true);
			setError(undefined);
			try {
				const page = await adapter.datasetRows(userId, datasetId, next, 50, filter);
				if (version !== requestVersion.current) return;
				setRows(page.rows);
				setOffset(page.offset);
				setNextOffset(page.nextOffset ?? null);
				setTotal(page.total);
			} catch (loadError) {
				if (version !== requestVersion.current) return;
				setError(localizedFactorError(loadError, t));
			} finally {
				if (version === requestVersion.current) setLoading(false);
			}
		},
		[adapter, datasetId, t, userId],
	);
	useEffect(() => {
		void loadRows();
		return () => {
			requestVersion.current += 1;
		};
	}, [loadRows]);
	return (
		<Card>
			<CardHeader className="flex-row items-start justify-between space-y-0">
				<div>
					<CardTitle>{t("factors.datasets.inspector")}</CardTitle>
					<CardDescription className="font-mono">{datasetId}</CardDescription>
				</div>
				<Button type="button" variant="outline" size="sm" onClick={onClose}>
					{t("factors.datasets.close")}
				</Button>
			</CardHeader>
			<CardContent className="space-y-4">
				<div className="grid gap-3 md:grid-cols-3">
					<Detail
						label={t("factors.datasets.candidate")}
						value={textAt(dataset.manifest, "candidateHash")}
						mono
					/>
					<Detail
						label={t("factors.datasets.featureDataset")}
						value={textAt(dataset.manifest, "featureDatasetId")}
						mono
					/>
					<Detail
						label={t("factors.datasets.snapshot")}
						value={textAt(dataset.manifest, "marketDataSnapshotId")}
						mono
					/>
					<Detail
						label={t("factors.datasets.universe")}
						value={textAt(dataset.manifest, "pointInTimeUniverseId")}
						mono
					/>
					<Detail
						label={t("factors.datasets.range")}
						value={`${textAt(dataset.manifest, "observationRange.startTimeMs")} → ${textAt(dataset.manifest, "observationRange.endTimeMs")}`}
						mono
					/>
					<Detail
						label={t("factors.datasets.engine")}
						value={textAt(dataset.manifest, "engineIdentity.engineId")}
						mono
					/>
					<Detail
						label={t("factors.datasets.payload")}
						value={shortFactorHash(valueAt(dataset.manifest, "payloadSha256"))}
						mono
					/>
					<Detail
						label={t("factors.datasets.size")}
						value={formatNumber(dataset.byteSize)}
					/>
					<Detail
						label={t("factors.datasets.created")}
						value={formatDateTime(dataset.createdAtMs, {
							dateStyle: "medium",
							timeStyle: "short",
						})}
					/>
				</div>
				<EvidenceJson
					label={t("factors.datasets.manifest")}
					value={dataset.manifest}
				/>
				<div className="flex flex-wrap items-end gap-3">
					<div className="grid gap-1.5">
						<Label htmlFor="factor-row-instrument">
							{t("factors.datasets.filterInstrument")}
						</Label>
						<Input
							id="factor-row-instrument"
							value={instrument}
							onChange={(event) => setInstrument(event.target.value)}
							placeholder="venue:instrument"
						/>
					</div>
					<Button
						type="button"
						variant="outline"
						loading={loading}
						loadingText={t("factors.loadingRows")}
						onClick={() => void loadRows(0, instrument)}
					>
						{t("factors.datasets.applyFilter")}
					</Button>
				</div>
				{error ? <Feedback message={error} /> : null}
				{loading ? (
					<LoadingState label={t("factors.loadingRows")} />
				) : (
					<>
						<div className="max-w-full overflow-x-auto">
							<table className="w-full min-w-[760px] text-sm">
								<caption className="sr-only">{t("factors.datasets.rows")}</caption>
								<thead>
									<tr className="border-b text-left text-muted-foreground">
										<th scope="col" className="py-2 pr-4">
											{t("factors.datasets.instrument")}
										</th>
										<th scope="col" className="py-2 pr-4">
											{t("factors.datasets.observationTime")}
										</th>
										<th scope="col" className="py-2">
											{t("factors.datasets.values")}
										</th>
									</tr>
								</thead>
								<tbody>
									{rows.map((row) => (
										<tr key={JSON.stringify(row)} className="border-b">
											<td className="py-2 pr-4 font-mono text-xs">
												{row.instrumentId ?? "—"}
											</td>
											<td className="py-2 pr-4">
												{row.observationTimeMs
													? formatDateTime(row.observationTimeMs, {
															dateStyle: "medium",
															timeStyle: "short",
															timeZone: "UTC",
														})
													: "—"}
											</td>
											<td className="py-2">
												<pre className="max-w-xl whitespace-pre-wrap break-words font-mono text-xs">
													{jsonText(row.values ?? row)}
												</pre>
											</td>
										</tr>
									))}
								</tbody>
							</table>
						</div>
						<div className="flex flex-wrap items-center justify-between gap-2 text-xs text-muted-foreground">
							<span>
								{formatNumber(offset)}–{formatNumber(offset + rows.length)} /{" "}
								{formatNumber(total)}
							</span>
							<div className="flex gap-2">
								<Button
									type="button"
									size="sm"
									variant="outline"
									disabled={offset === 0}
									onClick={() => void loadRows(Math.max(0, offset - 50), instrument)}
								>
									{t("factors.datasets.previous")}
								</Button>
								<Button
									type="button"
									size="sm"
									variant="outline"
									disabled={nextOffset == null}
									onClick={() => void loadRows(nextOffset ?? offset, instrument)}
								>
									{t("factors.datasets.next")}
								</Button>
							</div>
						</div>
					</>
				)}
			</CardContent>
		</Card>
	);
}

function EvaluationsWorkspace({
	userId,
	adapter,
}: {
	userId: string;
	adapter: FactorAdapter;
}) {
	const { t } = useTranslation();
	const reports = useFactorPage(userId, "reports", adapter.listReports);
	const [selected, setSelected] = useState<FactorReportView>();
	const [metricDefinitions, setMetricDefinitions] = useState<FactorJson[]>();
	const [feedback, setFeedback] = useState<string>();
	const [attemptRefresh, setAttemptRefresh] = useState(0);
	const [loadingReportId, setLoadingReportId] = useState<string>();
	const lastFocus = useRef<HTMLElement | null>(null);
	const inspectRequest = useRef(0);
	useEffect(() => {
		if (!selected) lastFocus.current?.focus();
	}, [selected]);
	useEffect(() => {
		let active = true;
		void adapter
			.metricCatalog()
			.then((catalog: FactorMetricCatalogView) => {
				if (active) setMetricDefinitions(catalog.definitions);
			})
			.catch(() => undefined);
		return () => {
			active = false;
		};
	}, [adapter]);
	return (
		<div className="space-y-5">
			<EvaluationStart
				userId={userId}
				adapter={adapter}
				onStarted={() => {
					setFeedback(t("factors.evaluations.started"));
					setAttemptRefresh((current) => current + 1);
				}}
			/>
			<AttemptsPanel
				userId={userId}
				adapter={adapter}
				kind="factor-evaluation"
				refreshKey={attemptRefresh}
			/>
			<Card>
				<CardHeader>
					<CardTitle>{t("factors.evaluations.heading")}</CardTitle>
					<CardDescription>{t("factors.evaluations.description")}</CardDescription>
				</CardHeader>
				<CardContent className="space-y-4">
					<Feedback message={feedback} tone="success" />
					{reports.error ? (
						<ErrorState
							message={localizedFactorError(reports.error, t)}
							onRetry={() => void reports.load()}
							retryLabel={t("factors.retry")}
							loading={reports.loading}
						/>
					) : null}
					{reports.loading && !reports.data ? (
						<LoadingState label={t("factors.loading")} />
					) : null}
					{reports.data && !reports.error && reports.data.items.length === 0 ? (
						<EmptyState message={t("factors.evaluations.empty")} />
					) : null}
					{reports.data && reports.data.items.length > 0 ? (
						<>
							<div className="max-w-full overflow-x-auto">
								<table className="w-full min-w-[820px] text-sm">
									<caption className="sr-only">
										{t("factors.evaluations.heading")}
									</caption>
									<thead>
										<tr className="border-b text-left text-muted-foreground">
											<th scope="col" className="py-2 pr-4">
												{t("factors.evaluations.report")}
											</th>
											<th scope="col" className="py-2 pr-4">
												{t("factors.evaluations.output")}
											</th>
											<th scope="col" className="py-2 pr-4">
												{t("factors.evaluations.state")}
											</th>
											<th scope="col" className="py-2 pr-4">
												{t("factors.evaluations.target")}
											</th>
											<th scope="col" className="py-2 text-right">
												{t("factors.common.actions")}
											</th>
										</tr>
									</thead>
									<tbody>
										{reports.data.items.map((item) => (
											<tr key={textAt(item.report, "reportHash")} className="border-b">
												<td className="py-3 pr-4 font-mono text-xs">
													{shortFactorHash(valueAt(item.report, "reportHash"))}
												</td>
												<td className="py-3 pr-4">{textAt(item.report, "outputName")}</td>
												<td className="py-3 pr-4">
													<Badge
														variant={
															textAt(item.report, "evidenceState") === "out-of-sample"
																? "secondary"
																: "outline"
														}
													>
														{localizedFactorCode(textAt(item.report, "evidenceState"), t)}
													</Badge>
												</td>
												<td className="py-3 pr-4">{textAt(item.report, "target")}</td>
												<td className="py-3 text-right">
													<Button
														type="button"
														size="sm"
														variant="outline"
														loading={loadingReportId === textAt(item.report, "reportHash")}
														loadingText={t("factors.loading")}
														onClick={async () => {
															const request = ++inspectRequest.current;
															const reportHash = textAt(item.report, "reportHash");
															lastFocus.current = document.activeElement as HTMLElement | null;
															setLoadingReportId(reportHash);
															try {
																const details = await adapter.getReport(userId, reportHash);
																if (request === inspectRequest.current) setSelected(details);
															} catch (error) {
																if (request === inspectRequest.current)
																	setFeedback(localizedFactorError(error, t));
															} finally {
																if (request === inspectRequest.current)
																	setLoadingReportId(undefined);
															}
														}}
													>
														{t("factors.evaluations.inspect")}
													</Button>
												</td>
											</tr>
										))}
									</tbody>
								</table>
							</div>
							<PageControls
								page={reports.data.page}
								total={reports.data.total}
								pageSize={reports.data.pageSize}
								onPage={(page) => void reports.load(page)}
							/>
						</>
					) : null}
				</CardContent>
			</Card>
			{selected ? (
				<ReportInspector
					report={selected}
					onClose={() => {
						inspectRequest.current += 1;
						setSelected(undefined);
					}}
					metricDefinitions={metricDefinitions}
				/>
			) : null}
		</div>
	);
}

function EvaluationStart({
	userId,
	adapter,
	onStarted,
}: {
	userId: string;
	adapter: FactorAdapter;
	onStarted: () => void;
}) {
	const { t } = useTranslation();
	const candidates = useFactorPage(userId, "candidates", adapter.listCandidates);
	const datasets = useFactorPage(userId, "datasets", adapter.listDatasets);
	const factorContextQuery = useResearchEvidenceContext(userId);
	const factorContext = factorContextQuery.data;
	const compatibleCandidates = useMemo(
		() => factorCandidatesForContext(candidates.data?.items ?? [], factorContext),
		[candidates.data?.items, factorContext],
	);
	const compatibleDatasets = useMemo(() => {
		const featureDataset = factorContext?.featureDataset;
		const universeId = factorContext?.universeId;
		if (!featureDataset || !universeId) return [];
		return (datasets.data?.items ?? []).filter((dataset) => {
			return (
				textAt(dataset.manifest, "featureDatasetId") === featureDataset.datasetId &&
				textAt(dataset.manifest, "featurePlanHash") ===
					featureDataset.featurePlanHash &&
				textAt(dataset.manifest, "marketDataSnapshotId") ===
					factorContext.snapshotId &&
				textAt(dataset.manifest, "pointInTimeUniverseId") === universeId
			);
		});
	}, [datasets.data?.items, factorContext]);
	const [candidateHash, setCandidateHash] = useState("");
	const [datasetId, setDatasetId] = useState("");
	const [outputName, setOutputName] = useState("");
	const [candidatePage, setCandidatePage] = useState(1);
	const [datasetPage, setDatasetPage] = useState(1);
	const [busy, setBusy] = useState(false);
	const [feedback, setFeedback] = useState<string>();
	const selectedCandidate = compatibleCandidates.find(
		(candidate) => textAt(candidate.candidate, "candidateHash") === candidateHash,
	);
	const selectedDataset = compatibleDatasets.find(
		(dataset) =>
			textAt(dataset.manifest, "datasetId") === datasetId &&
			textAt(dataset.manifest, "candidateHash") === candidateHash,
	);
	const candidateDatasets = useMemo(
		() =>
			compatibleDatasets.filter(
				(dataset) => textAt(dataset.manifest, "candidateHash") === candidateHash,
			),
		[compatibleDatasets, candidateHash],
	);
	const outputNames = useMemo(() => {
		const outputs = valueAt(selectedDataset?.manifest, "outputNames");
		return Array.isArray(outputs)
			? outputs.filter((output): output is string => typeof output === "string")
			: [];
	}, [selectedDataset?.manifest]);

	useEffect(() => {
		if (candidates.data && candidates.data.page !== candidatePage) {
			void candidates.load(candidatePage);
		}
	}, [candidatePage, candidates.data, candidates.load]);

	useEffect(() => {
		if (datasets.data && datasets.data.page !== datasetPage) {
			void datasets.load(datasetPage);
		}
	}, [datasetPage, datasets.data, datasets.load]);

	useEffect(() => {
		setCandidateHash((current) =>
			compatibleCandidates.some(
				(candidate) => textAt(candidate.candidate, "candidateHash") === current,
			)
				? current
				: textAt(compatibleCandidates[0]?.candidate, "candidateHash", ""),
		);
	}, [compatibleCandidates]);

	useEffect(() => {
		setDatasetId((current) =>
			candidateDatasets.some(
				(dataset) => textAt(dataset.manifest, "datasetId") === current,
			)
				? current
				: textAt(candidateDatasets[0]?.manifest, "datasetId", ""),
		);
	}, [candidateDatasets]);

	useEffect(() => {
		setOutputName((current) =>
			outputNames.includes(current) ? current : (outputNames[0] ?? ""),
		);
	}, [outputNames]);

	const start = async () => {
		setBusy(true);
		setFeedback(undefined);
		try {
			if (!factorContext?.featureDataset || !factorContext.universeId) {
				throw new Error(t("factors.evaluations.contextRequired"));
			}
			if (!candidateHash || !datasetId || !outputName) {
				throw new Error(t("factors.evaluations.selectionRequired"));
			}
			await adapter.startEvaluationFromContext(
				userId,
				candidateHash,
				datasetId,
				outputName,
			);
			setFeedback(t("factors.evaluations.started"));
			onStarted();
		} catch (error) {
			setFeedback(localizedFactorContextError(error, t));
		} finally {
			setBusy(false);
		}
	};
	return (
		<Card>
			<CardHeader>
				<CardTitle>{t("factors.evaluations.startHeading")}</CardTitle>
				<CardDescription>
					{t("factors.evaluations.startDescription")}
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-4">
				<div className="grid gap-3 md:grid-cols-2">
					<div className="grid gap-1.5">
						<Label htmlFor="factor-evaluation-candidate">
							{t("factors.evaluations.candidate")}
						</Label>
						<select
							id="factor-evaluation-candidate"
							className="h-9 rounded-md border bg-background px-3 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
							value={candidateHash}
							disabled={busy}
							onChange={(event) => setCandidateHash(event.target.value)}
						>
							<option value="">
								{candidates.loading
									? t("factors.evaluations.loadingEvidence")
									: t("factors.evaluations.selectCandidate")}
							</option>
							{compatibleCandidates.map((candidate) => {
								const hash = textAt(candidate.candidate, "candidateHash", "");
								return (
									<option key={hash} value={hash}>
										{candidate.presentation.name} · {shortFactorHash(hash)}
									</option>
								);
							})}
						</select>
					</div>
					<div className="grid gap-1.5">
						<Label htmlFor="factor-evaluation-dataset">
							{t("factors.evaluations.dataset")}
						</Label>
						<select
							id="factor-evaluation-dataset"
							className="h-9 rounded-md border bg-background px-3 font-mono text-xs outline-none focus-visible:ring-2 focus-visible:ring-ring"
							value={datasetId}
							disabled={busy || !candidateHash}
							onChange={(event) => setDatasetId(event.target.value)}
						>
							<option value="">{t("factors.evaluations.selectDataset")}</option>
							{candidateDatasets.map((dataset) => {
								const id = textAt(dataset.manifest, "datasetId", "");
								return (
									<option key={id} value={id}>
										{shortFactorHash(id)} ·{" "}
										{formatNumber(
											Number(valueAt(dataset.manifest, "observationCount") ?? 0),
										)}{" "}
										rows
									</option>
								);
							})}
						</select>
					</div>
					<div className="grid gap-1.5">
						<Label htmlFor="factor-evaluation-output">
							{t("factors.evaluations.output")}
						</Label>
						<select
							id="factor-evaluation-output"
							className="h-9 rounded-md border bg-background px-3 font-mono text-xs outline-none focus-visible:ring-2 focus-visible:ring-ring"
							value={outputName}
							disabled={busy || outputNames.length === 0}
							onChange={(event) => setOutputName(event.target.value)}
						>
							<option value="">{t("factors.evaluations.selectOutput")}</option>
							{outputNames.map((output) => (
								<option key={output} value={output}>
									{output}
								</option>
							))}
						</select>
					</div>
				</div>
				{candidates.data && candidates.data.total > candidates.data.pageSize ? (
					<PageControls
						page={candidates.data.page}
						total={candidates.data.total}
						pageSize={candidates.data.pageSize}
						onPage={setCandidatePage}
					/>
				) : null}
				{datasets.data && datasets.data.total > datasets.data.pageSize ? (
					<PageControls
						page={datasets.data.page}
						total={datasets.data.total}
						pageSize={datasets.data.pageSize}
						onPage={setDatasetPage}
					/>
				) : null}
				{selectedCandidate && selectedDataset ? (
					<div className="space-y-3 rounded-lg border bg-muted/20 p-3">
						<p className="text-sm font-medium">
							{t("factors.evaluations.boundaryHeading")}
						</p>
						<dl className="grid gap-3 text-sm sm:grid-cols-2 lg:grid-cols-3">
							<Detail
								label={t("factors.candidates.candidateHash")}
								value={textAt(selectedCandidate.candidate, "candidateHash")}
								mono
							/>
							<Detail
								label={t("factors.evaluations.dataset")}
								value={textAt(selectedDataset.manifest, "datasetId")}
								mono
							/>
							<Detail
								label={t("factors.datasets.featureDataset")}
								value={textAt(selectedDataset.manifest, "featureDatasetId")}
								mono
							/>
							<Detail
								label={t("factors.candidates.featurePlanHash")}
								value={textAt(selectedDataset.manifest, "featurePlanHash")}
								mono
							/>
							<Detail
								label={t("factors.datasets.snapshot")}
								value={textAt(selectedDataset.manifest, "marketDataSnapshotId")}
								mono
							/>
							<Detail
								label={t("factors.datasets.universe")}
								value={textAt(selectedDataset.manifest, "pointInTimeUniverseId")}
								mono
							/>
						</dl>
						<EvidenceJson
							label={t("factors.evaluations.candidateEvidence")}
							value={selectedCandidate.predecessor}
						/>
						<EvidenceJson
							label={t("factors.evaluations.datasetManifest")}
							value={selectedDataset.manifest}
						/>
					</div>
				) : null}
				<div className="rounded-lg border bg-muted/20 p-3">
					<p className="mb-3 text-sm font-medium">
						{t("factors.evaluations.protocolBoundary")}
					</p>
					<dl className="grid gap-3 text-sm sm:grid-cols-2 lg:grid-cols-4">
						<Detail
							label={t("factors.evaluations.target")}
							value="future-close-return"
						/>
						<Detail label={t("factors.evaluations.horizons")} value="1" mono />
						<Detail label={t("factors.evaluations.purgeEmbargo")} value="0 / 0" />
						<Detail
							label={t("factors.evaluations.lenses")}
							value={
								textAt(selectedDataset?.manifest, "scope") === "cross-sectional"
									? "cross-sectional, economic"
									: "temporal, economic"
							}
						/>
						<Detail
							label={t("factors.evaluations.trialIdentity")}
							value={t("factors.evaluations.hostResolved")}
						/>
						<Detail
							label={t("factors.evaluations.scope")}
							value={textAt(selectedDataset?.manifest, "scope")}
						/>
					</dl>
				</div>
				{factorContextQuery.error ? (
					<p role="alert" className="text-sm text-destructive">
						{localizedFactorContextError(factorContextQuery.error, t)}
					</p>
				) : null}
				{candidates.error || datasets.error ? (
					<p role="alert" className="text-sm text-destructive">
						{localizedFactorError(candidates.error ?? datasets.error, t)}
					</p>
				) : null}
				<div className="flex flex-wrap items-center gap-3">
					<Button
						type="button"
						loading={busy}
						loadingText={t("factors.common.queueing")}
						disabled={busy || !candidateHash || !datasetId || !outputName}
						onClick={() => void start()}
					>
						{t("factors.evaluations.start")}
					</Button>
					<span className="text-xs text-muted-foreground">
						{t("factors.evaluations.hostOwnsEvidence")}
					</span>
				</div>
				<Feedback
					message={feedback}
					tone={feedback === t("factors.evaluations.started") ? "success" : "error"}
				/>
			</CardContent>
		</Card>
	);
}

function ReportInspector({
	report,
	onClose,
	metricDefinitions,
}: {
	report: FactorReportView;
	onClose: () => void;
	metricDefinitions?: FactorJson[];
}) {
	const { t } = useTranslation();
	const metrics = Array.isArray(report.report.metrics)
		? report.report.metrics
		: [];
	return (
		<Card>
			<CardHeader className="flex-row items-start justify-between space-y-0">
				<div>
					<CardTitle>{t("factors.evaluations.inspector")}</CardTitle>
					<CardDescription className="font-mono">
						{textAt(report.report, "reportHash")}
					</CardDescription>
				</div>
				<Button type="button" variant="outline" size="sm" onClick={onClose}>
					{t("factors.datasets.close")}
				</Button>
			</CardHeader>
			<CardContent className="space-y-4">
				<div className="grid gap-3 md:grid-cols-4">
					<Detail
						label={t("factors.evaluations.state")}
						value={textAt(report.report, "evidenceState")}
					/>
					<Detail
						label={t("factors.evaluations.output")}
						value={textAt(report.report, "outputName")}
						mono
					/>
					<Detail
						label={t("factors.evaluations.dataset")}
						value={shortFactorHash(valueAt(report.report, "factorDatasetId"))}
						mono
					/>
					<Detail
						label={t("factors.evaluations.protocolHash")}
						value={shortFactorHash(valueAt(report.report, "protocolHash"))}
						mono
					/>
					<Detail
						label={t("factors.evaluations.targetUnavailable")}
						value={formatNumber(
							Array.isArray(valueAt(report.report, "targetUnavailable"))
								? (valueAt(report.report, "targetUnavailable") as unknown[]).length
								: 0,
						)}
					/>
					<Detail
						label={t("factors.evaluations.regimeEvidence")}
						value={formatNumber(
							Array.isArray(valueAt(report.report, "regimeEvidence"))
								? (valueAt(report.report, "regimeEvidence") as unknown[]).length
								: 0,
						)}
					/>
				</div>
				{report.protocol ? (
					<div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
						<Detail
							label={t("factors.evaluations.orientation")}
							value={textAt(report.protocol, "orientation")}
						/>
						<Detail
							label={t("factors.evaluations.horizons")}
							value={jsonText(valueAt(report.protocol, "horizonBars"))}
						/>
						<Detail
							label={t("factors.evaluations.folds")}
							value={jsonText(valueAt(report.protocol, "windows"))}
						/>
						<Detail
							label={t("factors.evaluations.purgeEmbargo")}
							value={`${textAt(report.protocol, "purgeBars")} / ${textAt(report.protocol, "embargoBars")}`}
						/>
						<Detail
							label={t("factors.evaluations.lenses")}
							value={jsonText(valueAt(report.protocol, "lenses"))}
						/>
						<Detail
							label={t("factors.evaluations.nuisance")}
							value={jsonText(valueAt(report.protocol, "nuisanceFeatureNames"))}
						/>
						<Detail
							label={t("factors.evaluations.regime")}
							value={jsonText(valueAt(report.protocol, "regime"))}
						/>
						<Detail
							label={t("factors.evaluations.economic")}
							value={jsonText(valueAt(report.protocol, "economic"))}
						/>
					</div>
				) : null}
				<div className="rounded-lg border p-3 text-sm text-muted-foreground">
					{t("factors.evaluations.guaranteeNote")}
				</div>
				<MetricTable metrics={metrics} metricDefinitions={metricDefinitions} />
				{report.protocol ? (
					<EvidenceJson
						label={t("factors.evaluations.frozenProtocol")}
						value={report.protocol}
					/>
				) : null}
				<EvidenceJson label={t("factors.common.rawEvidence")} value={report} />
			</CardContent>
		</Card>
	);
}

function MetricTable({
	metrics,
	metricDefinitions,
}: {
	metrics: FactorJson[];
	metricDefinitions?: FactorJson[];
}) {
	const { t } = useTranslation();
	return (
		<div className="max-w-full overflow-x-auto">
			<table className="w-full min-w-[1120px] text-sm">
				<caption className="sr-only">{t("factors.evaluations.metrics")}</caption>
				<thead>
					<tr className="border-b text-left text-muted-foreground">
						<th scope="col" className="py-2 pr-4">
							{t("factors.evaluations.metric")}
						</th>
						<th scope="col" className="py-2 pr-4">
							{t("factors.evaluations.lens")}
						</th>
						<th scope="col" className="py-2 pr-4">
							{t("factors.evaluations.output")}
						</th>
						<th scope="col" className="py-2 pr-4">
							{t("factors.evaluations.variant")}
						</th>
						<th scope="col" className="py-2 pr-4">
							{t("factors.evaluations.horizon")}
						</th>
						<th scope="col" className="py-2 pr-4">
							{t("factors.evaluations.fold")}
						</th>
						<th scope="col" className="py-2 pr-4">
							{t("factors.evaluations.value")}
						</th>
						<th scope="col" className="py-2">
							{t("factors.evaluations.samples")}
						</th>
					</tr>
				</thead>
				<tbody>
					{metrics.map((metric) => (
						<tr key={JSON.stringify(metric)} className="border-b">
							<td className="py-2 pr-4">
								<span className="font-mono text-xs">{textAt(metric, "metric")}</span>
								<MetricDefinition metric={metric} definitions={metricDefinitions} />
							</td>
							<td className="py-2 pr-4">{textAt(metric, "lens")}</td>
							<td className="py-2 pr-4">{textAt(metric, "outputName")}</td>
							<td className="py-2 pr-4">{textAt(metric, "variant")}</td>
							<td className="py-2 pr-4">{textAt(metric, "horizonBars")}</td>
							<td className="py-2 pr-4">{textAt(metric, "foldId")}</td>
							<td className="py-2 pr-4 font-mono">
								{metricObservation(metric).value}
								<span className="ml-2 text-xs text-muted-foreground">
									{localizedFactorCode(metricObservation(metric).reason, t)}
								</span>
							</td>
							<td className="py-2">{metricObservation(metric).sampleCount}</td>
						</tr>
					))}
				</tbody>
			</table>
		</div>
	);
}

function metricObservation(metric: FactorJson) {
	const observation = valueAt(metric, "observation");
	const available = valueAt(observation, "available");
	const unavailable = valueAt(observation, "unavailable");
	if (available && typeof available === "object") {
		return {
			value: factorString(valueAt(available, "value"), "unavailable"),
			reason: "",
			sampleCount: factorString(valueAt(available, "sampleCount")),
		};
	}
	if (unavailable && typeof unavailable === "object") {
		return {
			value: "unavailable",
			reason: factorString(valueAt(unavailable, "reason")),
			sampleCount: factorString(valueAt(unavailable, "sampleCount")),
		};
	}
	return {
		value: textAt(
			metric,
			"observation.value",
			textAt(metric, "value", "unavailable"),
		),
		reason: textAt(metric, "observation.reason", ""),
		sampleCount: textAt(
			metric,
			"observation.sampleCount",
			textAt(metric, "sampleCount"),
		),
	};
}

function MetricDefinition({
	metric,
	definitions,
}: {
	metric: FactorJson;
	definitions?: FactorJson[];
}) {
	const { t } = useTranslation();
	const definition = definitions?.find(
		(item) => textAt(item, "id") === textAt(metric, "metric"),
	);
	return (
		<details className="mt-1 text-xs text-muted-foreground">
			<summary className="cursor-pointer">
				{t("factors.evaluations.definition")}
			</summary>
			<pre className="mt-1 max-w-[32rem] whitespace-pre-wrap break-words font-mono">
				{definition
					? jsonText(definition)
					: t("factors.evaluations.catalogUnavailable")}
			</pre>
		</details>
	);
}

type FactorPromotionDecisionState =
	| "rejected"
	| "research-validated"
	| "component-eligible";

type Gate6QualificationStage = "build" | "qualification";

type Gate6QualificationPreflight = {
	decision: FactorDecisionView;
	candidate: FactorCandidateView;
	dataset: FactorDatasetView;
	report: FactorReportView;
	reports: FactorReportView[];
	policy: FactorPolicyView;
};

type Gate6QualificationOperation = {
	stage: Gate6QualificationStage;
	attempt: FactorAttemptView;
	decisionId: string;
	outputName: string;
	preflight: Gate6QualificationPreflight;
	candidate?: FactorComponentCandidateView;
	qualification?: FactorComponentQualificationView;
};

function Gate6QualificationWorkspace({
	userId,
	adapter,
	decisions,
	decisionLoading,
	decisionError,
	candidates,
	datasets,
	reports,
	policies,
}: {
	userId: string;
	adapter: FactorAdapter;
	decisions: FactorDecisionView[];
	decisionLoading: boolean;
	decisionError?: string;
	candidates: FactorCandidateView[];
	datasets: FactorDatasetView[];
	reports: FactorReportView[];
	policies: FactorPolicyView[];
}) {
	const { t } = useTranslation();
	const eligibleDecisions = decisions.filter(
		(item) =>
			textAt(item.decision, "state") === "component-eligible" &&
			Boolean(textAt(item.decision, "decisionId", "")),
	);
	const [selectedDecisionId, setSelectedDecisionId] = useState("");
	const [selectedOutputName, setSelectedOutputName] = useState("");
	const [operation, setOperation] = useState<Gate6QualificationOperation>();
	const [operationError, setOperationError] = useState<string>();
	const [starting, setStarting] = useState(false);
	const [cancelling, setCancelling] = useState(false);
	const [retrying, setRetrying] = useState(false);
	const [libraryChecked, setLibraryChecked] = useState(false);
	const [libraryRecord, setLibraryRecord] = useState<LibraryComponent>();
	const operationVersion = useRef(0);
	const hasSeenEligibleDecision = useRef(false);

	useEffect(() => {
		if (operation) return;
		if (!eligibleDecisions.length) {
			setSelectedDecisionId("");
			setSelectedOutputName("");
			return;
		}
		setSelectedDecisionId((current) => {
			if (
				current &&
				eligibleDecisions.some(
					(item) => textAt(item.decision, "decisionId", "") === current,
				)
			)
				return current;
			if (current || hasSeenEligibleDecision.current) return "";
			hasSeenEligibleDecision.current = true;
			return textAt(eligibleDecisions[0].decision, "decisionId", "");
		});
	}, [eligibleDecisions, operation]);

	const selectedDecision = eligibleDecisions.find(
		(item) => textAt(item.decision, "decisionId", "") === selectedDecisionId,
	);
	const operationPreflight = operation?.preflight;
	const decisionForDisplay = operationPreflight?.decision ?? selectedDecision;
	const candidateHash = textAt(
		decisionForDisplay?.decision,
		"candidateHash",
		"",
	);
	const selectedCandidate =
		operationPreflight?.candidate ??
		candidates.find(
			(item) => textAt(item.candidate, "candidateHash", "") === candidateHash,
		);
	const decisionOptions =
		operationPreflight &&
		!eligibleDecisions.some(
			(item) =>
				textAt(item.decision, "decisionId", "") ===
				textAt(operationPreflight.decision.decision, "decisionId", ""),
		)
			? [...eligibleDecisions, operationPreflight.decision]
			: eligibleDecisions;
	const outputOptions = Array.from(
		new Set(
			[
				...eligibleDecisions
					.filter(
						(item) => textAt(item.decision, "candidateHash", "") === candidateHash,
					)
					.map((item) => textAt(item.decision, "outputName", "")),
				operationPreflight
					? textAt(operationPreflight.decision.decision, "outputName", "")
					: "",
			].filter(Boolean),
		),
	);
	const hasGate6Selection =
		eligibleDecisions.length > 0 || Boolean(operationPreflight);
	useEffect(() => {
		if (operation) return;
		setSelectedOutputName(textAt(decisionForDisplay?.decision, "outputName", ""));
	}, [decisionForDisplay, operation]);

	const reportHashes = Array.isArray(
		valueAt(decisionForDisplay?.decision, "reportHashes"),
	)
		? (valueAt(decisionForDisplay?.decision, "reportHashes") as unknown[]).filter(
				(value): value is string => typeof value === "string",
			)
		: [];
	const resolvedReports = reportHashes
		.map((hash) =>
			reports.find((item) => textAt(item.report, "reportHash", "") === hash),
		)
		.filter((item): item is FactorReportView => Boolean(item));
	const selectedReports = operationPreflight?.reports ?? resolvedReports;
	const selectedReport = operationPreflight?.report ?? selectedReports[0];
	const factorDatasetId = textAt(selectedReport?.report, "factorDatasetId", "");
	const selectedDataset =
		operationPreflight?.dataset ??
		datasets.find(
			(item) => textAt(item.manifest, "datasetId", "") === factorDatasetId,
		);
	const policyHash = textAt(decisionForDisplay?.decision, "policyHash", "");
	const selectedPolicy =
		operationPreflight?.policy ??
		policies.find((item) => textAt(item.policy, "policyHash", "") === policyHash);
	const predecessor = selectedCandidate?.predecessor;
	const decisionOutput = textAt(decisionForDisplay?.decision, "outputName", "");
	const candidateOutputs = Array.isArray(
		valueAt(selectedCandidate?.candidate, "outputs"),
	)
		? (valueAt(selectedCandidate?.candidate, "outputs") as unknown[])
				.map((output) =>
					typeof output === "string" ? output : textAt(output, "name", ""),
				)
				.filter(Boolean)
		: [];
	const outputAvailable = candidateOutputs.includes(selectedOutputName);
	const reportsMatchContext =
		reportHashes.length > 0 &&
		selectedReports.length === reportHashes.length &&
		selectedReports.every(
			(item) =>
				textAt(item.report, "factorDatasetId", "") === factorDatasetId &&
				textAt(item.report, "outputName", "") === selectedOutputName,
		);
	const evidenceMatchesContext = Boolean(
		selectedReport &&
			reportsMatchContext &&
			selectedDataset &&
			textAt(selectedReport.report, "outputName", "") === selectedOutputName &&
			textAt(selectedDataset.manifest, "featureDatasetId", "") ===
				textAt(predecessor?.featureDataset, "datasetId", "") &&
			textAt(selectedDataset.manifest, "featurePlanHash", "") ===
				textAt(predecessor?.featureDataset, "featurePlanHash", "") &&
			textAt(selectedDataset.manifest, "marketDataSnapshotId", "") ===
				textAt(predecessor, "snapshotId", "") &&
			textAt(selectedDataset.manifest, "pointInTimeUniverseId", "") ===
				textAt(predecessor, "universeId", ""),
	);
	const preflightReady = Boolean(
		decisionForDisplay &&
			selectedDecisionId &&
			selectedOutputName &&
			selectedOutputName === decisionOutput &&
			outputAvailable &&
			selectedCandidate &&
			predecessor?.userId === userId &&
			selectedDataset &&
			selectedReport &&
			selectedReports.length === reportHashes.length &&
			selectedPolicy &&
			predecessor.featureDataset &&
			predecessor.universeId &&
			evidenceMatchesContext,
	);
	const preflightReason = !decisionForDisplay
		? t("factors.gate6.selectDecision")
		: !selectedCandidate
			? t("factors.gate6.candidateUnavailable")
			: !selectedReport ||
					!selectedDataset ||
					!selectedPolicy ||
					!evidenceMatchesContext
				? t("factors.gate6.evidenceUnavailable")
				: !predecessor?.featureDataset || !predecessor.universeId
					? t("factors.gate6.contextUnavailable")
					: !outputAvailable || !preflightReady
						? t("factors.gate6.outputMismatch")
						: "";
	const selectClassName =
		"h-9 w-full rounded-md border bg-background px-3 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50";

	const resetOperation = () => {
		operationVersion.current += 1;
		setOperation(undefined);
		setOperationError(undefined);
		setStarting(false);
		setCancelling(false);
		setRetrying(false);
		setLibraryChecked(false);
		setLibraryRecord(undefined);
	};
	const selectDecision = (decisionId: string) => {
		resetOperation();
		setSelectedDecisionId(decisionId);
	};
	const selectOutput = (outputName: string) => {
		const next = eligibleDecisions.find(
			(item) =>
				textAt(item.decision, "candidateHash", "") === candidateHash &&
				textAt(item.decision, "outputName", "") === outputName,
		);
		if (!next) return;
		resetOperation();
		setSelectedOutputName(outputName);
		setSelectedDecisionId(textAt(next.decision, "decisionId", ""));
	};

	const isCurrentOperation = (version: number) =>
		version === operationVersion.current;
	const waitForAttemptPoll = () =>
		new Promise<void>((resolve) => window.setTimeout(resolve, 1_000));
	const setAttempt = (version: number, attempt: FactorAttemptView) => {
		if (!isCurrentOperation(version)) return;
		setOperation((current) => (current ? { ...current, attempt } : current));
	};

	const inspectQualification = async (
		version: number,
		attempt: FactorAttemptView,
		candidate?: FactorComponentCandidateView,
	) => {
		let details: FactorComponentQualificationView | undefined;
		try {
			details = await adapter.getComponentQualification(userId, attempt.attemptId);
			if (isCurrentOperation(version)) {
				setOperation((current) =>
					current
						? {
								...current,
								attempt: details?.attempt ?? attempt,
								qualification: details,
							}
						: current,
				);
			}
		} catch (error) {
			if (isCurrentOperation(version))
				setOperationError(localizedFactorError(error, t));
		}
		if (!isCurrentOperation(version)) return;
		try {
			const components = await adapter.listComponents(userId);
			if (!isCurrentOperation(version)) return;
			const packageSha256 =
				details?.packageSha256 ?? candidate?.packageSha256 ?? "";
			setLibraryRecord(
				details?.published === true
					? components.find((item) => item.archiveSha256 === packageSha256)
					: undefined,
			);
			setLibraryChecked(true);
		} catch (error) {
			if (isCurrentOperation(version))
				setOperationError(localizedFactorError(error, t));
		}
	};

	const followAttempt = async (
		version: number,
		stage: Gate6QualificationStage,
		initial: FactorAttemptView,
		candidate?: FactorComponentCandidateView,
	): Promise<void> => {
		let attempt = initial;
		while (!isTerminalFactorAttempt(attempt.status)) {
			if (!isCurrentOperation(version)) return;
			attempt = await adapter.getAttempt(userId, attempt.attemptId);
			setAttempt(version, attempt);
			if (isTerminalFactorAttempt(attempt.status)) break;
			await waitForAttemptPoll();
		}
		if (!isCurrentOperation(version)) return;
		if (stage === "build") {
			if (attempt.status !== "completed") return;
			const built = await adapter.getComponentCandidate(userId, attempt.attemptId);
			if (!isCurrentOperation(version)) return;
			setOperation((current) =>
				current ? { ...current, candidate: built } : current,
			);
			const qualificationAttempt = await adapter.prepareComponentQualification(
				userId,
				attempt.attemptId,
			);
			if (!isCurrentOperation(version)) return;
			setOperation((current) =>
				current
					? { ...current, stage: "qualification", attempt: qualificationAttempt }
					: current,
			);
			return followAttempt(version, "qualification", qualificationAttempt, built);
		}
		await inspectQualification(version, attempt, candidate);
	};

	const start = async () => {
		if (
			!preflightReady ||
			!selectedDecisionId ||
			!selectedOutputName ||
			!decisionForDisplay ||
			!selectedCandidate ||
			!selectedDataset ||
			!selectedReport ||
			!selectedPolicy
		) {
			setOperationError(preflightReason);
			return;
		}
		const version = ++operationVersion.current;
		setStarting(true);
		setOperationError(undefined);
		setLibraryChecked(false);
		setLibraryRecord(undefined);
		try {
			const attempt = await adapter.prepareComponent(
				userId,
				selectedDecisionId,
				selectedOutputName,
			);
			if (!isCurrentOperation(version)) return;
			setOperation({
				stage: "build",
				attempt,
				decisionId: selectedDecisionId,
				outputName: selectedOutputName,
				preflight: {
					decision: decisionForDisplay,
					candidate: selectedCandidate,
					dataset: selectedDataset,
					report: selectedReport,
					reports: selectedReports,
					policy: selectedPolicy,
				},
			});
			await followAttempt(version, "build", attempt);
		} catch (error) {
			if (isCurrentOperation(version))
				setOperationError(localizedFactorError(error, t));
		} finally {
			if (isCurrentOperation(version)) setStarting(false);
		}
	};

	const cancel = async () => {
		if (!operation) return;
		setCancelling(true);
		setOperationError(undefined);
		try {
			await adapter.cancelAttempt(userId, operation.attempt.attemptId);
		} catch (error) {
			setOperationError(localizedFactorError(error, t));
		} finally {
			setCancelling(false);
		}
	};

	const retry = async () => {
		if (!operation || operation.attempt.status === "completed") return;
		const version = ++operationVersion.current;
		const previous = operation;
		setRetrying(true);
		setOperationError(undefined);
		setLibraryChecked(false);
		setLibraryRecord(undefined);
		try {
			const attempt = await adapter.retryComponentAttempt(
				userId,
				previous.attempt.attemptId,
			);
			if (!isCurrentOperation(version)) return;
			setOperation({ ...previous, attempt, qualification: undefined });
			await followAttempt(version, previous.stage, attempt, previous.candidate);
		} catch (error) {
			if (isCurrentOperation(version))
				setOperationError(localizedFactorError(error, t));
		} finally {
			if (isCurrentOperation(version)) setRetrying(false);
		}
	};

	const attemptIsTerminal = operation
		? isTerminalFactorAttempt(operation.attempt.status)
		: false;
	const operationActive = Boolean(operation && !attemptIsTerminal);
	const source = valueAt(selectedCandidate?.candidate, "source");
	const sourceBuild = valueAt(source, "build");
	const sourceIdentity = source
		? {
				kind: textAt(source, "kind"),
				definition: valueAt(source, "definition"),
				build: sourceBuild,
			}
		: undefined;
	const operationDiagnostic = operation?.attempt.diagnostic?.trim();
	const operationDiagnosticLabel = operation?.attempt.failureCode
		? localizedFactorAttemptCode(operation.attempt.failureCode, t)
		: operationDiagnostic
			? localizedFactorError(operationDiagnostic, t)
			: "";

	return (
		<Card>
			<CardHeader>
				<CardTitle>{t("factors.gate6.heading")}</CardTitle>
				<CardDescription>{t("factors.gate6.description")}</CardDescription>
			</CardHeader>
			<CardContent className="space-y-4">
				{decisionError ? (
					<p className="text-sm text-destructive" role="alert">
						{localizedFactorError(decisionError, t)}
					</p>
				) : null}
				{decisionLoading && !decisions.length ? (
					<LoadingState label={t("factors.loading")} />
				) : null}
				{!decisionLoading &&
				!decisionError &&
				!eligibleDecisions.length &&
				!operationPreflight ? (
					<EmptyState message={t("factors.gate6.emptyDecisions")} />
				) : null}
				{hasGate6Selection ? (
					<>
						<div className="grid gap-3 md:grid-cols-2">
							<div className="grid gap-1.5">
								<Label htmlFor="factor-gate6-decision">
									{t("factors.gate6.decisionSelection")}
								</Label>
								<select
									id="factor-gate6-decision"
									className={selectClassName}
									value={selectedDecisionId}
									disabled={operationActive || starting}
									onChange={(event) => selectDecision(event.target.value)}
								>
									<option value="">{t("factors.gate6.selectDecision")}</option>
									{decisionOptions.map((item) => {
										const decisionId = textAt(item.decision, "decisionId", "");
										const candidateHash = textAt(item.decision, "candidateHash", "");
										const outputName = textAt(item.decision, "outputName", "");
										const candidate =
											candidates.find(
												(candidateItem) =>
													textAt(candidateItem.candidate, "candidateHash", "") ===
													candidateHash,
											) ??
											(operationPreflight?.candidate &&
											textAt(
												operationPreflight.candidate.candidate,
												"candidateHash",
												"",
											) === candidateHash
												? operationPreflight.candidate
												: undefined);
										return (
											<option key={decisionId} value={decisionId}>
												{`${textAt(candidate?.presentation, "name", t("factors.common.candidate"))} · ${outputName}`}
											</option>
										);
									})}
								</select>
							</div>
							<div className="grid gap-1.5">
								<Label htmlFor="factor-gate6-output">
									{t("factors.gate6.outputSelection")}
								</Label>
								<select
									id="factor-gate6-output"
									className={`${selectClassName} font-mono text-xs`}
									value={selectedOutputName}
									disabled={operationActive || starting || !candidateHash}
									onChange={(event) => selectOutput(event.target.value)}
								>
									<option value="">{t("factors.gate6.selectOutput")}</option>
									{outputOptions.map((outputName) => (
										<option key={outputName} value={outputName}>
											{outputName}
										</option>
									))}
								</select>
							</div>
						</div>
						<div
							className={`rounded-md border p-3 text-sm ${preflightReady ? "border-emerald-500/40 bg-emerald-500/5" : "border-amber-500/40 bg-amber-500/5"}`}
							role="status"
							aria-live="polite"
						>
							<p className="font-medium">
								{preflightReady ? t("factors.gate6.ready") : t("factors.gate6.blocked")}
							</p>
							{!preflightReady ? (
								<p className="mt-1 text-muted-foreground">{preflightReason}</p>
							) : null}
						</div>
						{decisionForDisplay ? (
							<div className="rounded-md border bg-muted/20 p-3">
								<p className="text-sm font-medium">
									{t("factors.gate6.preflightHeading")}
								</p>
								<dl className="mt-3 grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
									<Detail
										label={t("factors.gate6.decision")}
										value={selectedDecisionId || "—"}
										mono
									/>
									<Detail
										label={t("factors.gate6.output")}
										value={selectedOutputName || "—"}
										mono
									/>
									<Detail
										label={t("factors.gate6.candidate")}
										value={candidateHash || "—"}
										mono
									/>
									<Detail
										label={t("factors.gate6.policy")}
										value={policyHash || "—"}
										mono
									/>
									<Detail
										label={t("factors.gate6.reports")}
										value={reportHashes.join(", ") || "—"}
										mono
									/>
									<Detail
										label={t("factors.gate6.factorDataset")}
										value={factorDatasetId || "—"}
										mono
									/>
									<Detail
										label={t("factors.gate6.featureDataset")}
										value={textAt(predecessor?.featureDataset, "datasetId", "—")}
										mono
									/>
									<Detail
										label={t("factors.gate6.featurePlan")}
										value={textAt(predecessor?.featureDataset, "featurePlanHash", "—")}
										mono
									/>
									<Detail
										label={t("factors.gate6.marketSnapshot")}
										value={textAt(predecessor, "snapshotId", "—")}
										mono
									/>
									<Detail
										label={t("factors.gate6.universe")}
										value={textAt(predecessor, "universeId", "—")}
										mono
									/>
									<Detail
										label={t("factors.gate6.context")}
										value={`${textAt(predecessor, "market", "—")} · ${textAt(predecessor, "venue", "—")} · ${textAt(predecessor, "contextHash", "—")}`}
										mono
									/>
									<Detail
										label={t("factors.gate6.scope")}
										value={textAt(selectedCandidate?.candidate, "scope", "—")}
										mono
									/>
									<Detail
										label={t("factors.gate6.range")}
										value={`${textAt(predecessor, "rangeStartMs", "—")} → ${textAt(predecessor, "rangeEndMs", "—")}`}
										mono
									/>
									<Detail
										label={t("factors.gate6.parameters")}
										value={jsonText(valueAt(selectedCandidate?.candidate, "parameters"))}
									/>
									<Detail
										label={t("factors.gate6.sourceBuild")}
										value={jsonText(sourceIdentity)}
									/>
								</dl>
								<EvidenceJson
									label={t("factors.gate6.evidence")}
									value={{
										decision: decisionForDisplay,
										candidate: selectedCandidate?.candidate,
										dataset: selectedDataset?.manifest,
										report: selectedReport?.report,
										reports: selectedReports.map((item) => item.report),
										policy: selectedPolicy?.policy,
										context: predecessor,
									}}
								/>
							</div>
						) : null}
						<div className="flex flex-wrap items-center gap-3">
							<Button
								type="button"
								loading={starting}
								disabled={!preflightReady || starting || Boolean(operation)}
								onClick={() => void start()}
							>
								{t("factors.gate6.start")}
							</Button>
							<Link
								to="/components"
								className="text-sm text-primary underline-offset-4 hover:underline"
							>
								{t("factors.gate6.genericImport")}
							</Link>
						</div>
					</>
				) : null}
				{!eligibleDecisions.length && !operationPreflight ? (
					<Link
						to="/components"
						className="text-sm text-primary underline-offset-4 hover:underline"
					>
						{t("factors.gate6.genericImport")}
					</Link>
				) : null}
				{operation ? (
					<Card className="border-primary/30">
						<CardHeader>
							<CardTitle>{t("factors.gate6.operationHeading")}</CardTitle>
							<CardDescription>
								{t("factors.gate6.operationDescription")}
							</CardDescription>
						</CardHeader>
						<CardContent className="space-y-3">
							<div
								className="flex flex-wrap items-center gap-2"
								role="status"
								aria-live="polite"
							>
								<FactorAttemptStatusBadge status={operation.attempt.status} />
								<span className="text-sm text-muted-foreground">
									{operation.stage === "build"
										? t("factors.gate6.buildStage")
										: t("factors.gate6.qualificationStage")}
								</span>
							</div>
							<dl className="grid gap-3 text-xs sm:grid-cols-3">
								<Detail
									label={t("factors.gate6.attempt")}
									value={operation.attempt.attemptId}
									mono
								/>
								<Detail
									label={t("factors.gate6.requestHash")}
									value={operation.attempt.requestHash}
									mono
								/>
								<Detail
									label={t("factors.gate6.result")}
									value={operation.attempt.resultId ?? "—"}
									mono
								/>
							</dl>
							{operation.attempt.progressTotal > 0 ? (
								<progress
									className="h-2 w-full"
									value={operation.attempt.completedUnits}
									max={operation.attempt.progressTotal}
									aria-label={t("factors.gate6.progress")}
								/>
							) : null}
							{operation.attempt.failureCode || operation.attempt.diagnostic ? (
								<div className="space-y-1 text-sm text-destructive" role="alert">
									<p className="break-words">
										<span className="font-medium">
											{operationDiagnosticLabel}
											{operation.attempt.failureCode ? (
												<> ({operation.attempt.failureCode})</>
											) : null}
										</span>
									</p>
									{operationDiagnostic ? (
										<details className="text-muted-foreground">
											<summary className="cursor-pointer">
												{t("factors.gate6.technicalDiagnostic")}
											</summary>
											<code className="mt-1 block max-h-32 overflow-auto whitespace-pre-wrap break-words">
												{operationDiagnostic}
											</code>
										</details>
									) : null}
								</div>
							) : null}
							{operation.candidate ? (
								<div className="rounded-md border bg-muted/20 p-3">
									<p className="text-sm font-medium">
										{t("factors.gate6.candidatePackage")}
									</p>
									<dl className="mt-2 grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
										<Detail
											label={t("factors.gate6.package")}
											value={operation.candidate.packageSha256}
											mono
										/>
										<Detail
											label={t("factors.gate6.component")}
											value={textAt(operation.candidate.manifest, "componentId", "—")}
											mono
										/>
										<Detail
											label={t("factors.gate6.version")}
											value={textAt(operation.candidate.manifest, "version", "—")}
										/>
										<Detail
											label={t("factors.gate6.wasm")}
											value={textAt(operation.candidate.manifest, "wasmSha256", "—")}
											mono
										/>
									</dl>
									<EvidenceJson
										label={t("factors.gate6.candidateBinding")}
										value={operation.candidate.binding}
									/>
								</div>
							) : null}
							{operation.qualification ? (
								<div className="space-y-3 rounded-md border bg-muted/20 p-3">
									<dl className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
										<Detail
											label={t("factors.gate6.qualificationResult")}
											value={
												operation.qualification.published
													? t("factors.gate6.published")
													: t("factors.gate6.notPublished")
											}
										/>
										<Detail
											label={t("factors.gate6.package")}
											value={operation.qualification.packageSha256 ?? "—"}
											mono
										/>
										<Detail
											label={t("factors.gate6.provenance")}
											value={
												operation.qualification.provenance
													? t("factors.gate6.available")
													: t("factors.gate6.unavailable")
											}
										/>
										<Detail
											label={t("factors.gate6.equivalence")}
											value={
												operation.qualification.equivalence
													? t("factors.gate6.available")
													: t("factors.gate6.unavailable")
											}
										/>
									</dl>
									<EvidenceJson
										label={t("factors.gate6.qualificationEvidence")}
										value={{
											qualification: operation.qualification.qualification,
											provenance: operation.qualification.provenance,
											equivalence: operation.qualification.equivalence,
										}}
									/>
								</div>
							) : null}
							{operation.stage === "qualification" && attemptIsTerminal ? (
								<div className="rounded-md border p-3">
									<dl className="grid gap-3 sm:grid-cols-2">
										<Detail
											label={t("factors.gate6.libraryRecord")}
											value={
												libraryRecord
													? `${libraryRecord.name} · v${libraryRecord.version}`
													: t("factors.gate6.notPublished")
											}
										/>
										<Detail
											label={t("factors.gate6.entitlement")}
											value={
												libraryRecord
													? t("factors.gate6.entitlementGranted")
													: t("factors.gate6.entitlementMissing")
											}
										/>
									</dl>
									{libraryRecord ? (
										<EvidenceJson
											label={t("factors.gate6.libraryEvidence")}
											value={libraryRecord}
										/>
									) : null}
									{libraryChecked && libraryRecord ? (
										<Link
											to="/components"
											className="mt-3 inline-block text-sm text-primary underline-offset-4 hover:underline"
										>
											{t("factors.gate6.inspectLibrary")}
										</Link>
									) : null}
								</div>
							) : null}
							<div className="flex flex-wrap gap-3">
								{!attemptIsTerminal ? (
									<Button
										type="button"
										variant="outline"
										loading={cancelling}
										onClick={() => void cancel()}
									>
										{t("factors.gate6.cancel")}
									</Button>
								) : null}
								{attemptIsTerminal && operation.attempt.status !== "completed" ? (
									<Button
										type="button"
										variant="outline"
										loading={retrying}
										onClick={() => void retry()}
									>
										{operation.attempt.status === "interrupted" ||
										operation.attempt.status === "stale"
											? t("factors.gate6.restart")
											: t("factors.gate6.retry")}
									</Button>
								) : null}
							</div>
						</CardContent>
					</Card>
				) : null}
				<Feedback message={operationError} />
			</CardContent>
		</Card>
	);
}

function DecisionsWorkspace({
	userId,
	adapter,
}: {
	userId: string;
	adapter: FactorAdapter;
}) {
	const { t } = useTranslation();
	const candidates = useFactorPage(
		userId,
		"decision-candidates",
		adapter.listCandidates,
	);
	const datasets = useFactorPage(
		userId,
		"decision-datasets",
		adapter.listDatasets,
	);
	const reports = useFactorPage(userId, "decision-reports", adapter.listReports);
	const policies = useFactorPage(userId, "policies", adapter.listPolicies);
	const decisions = useFactorPage(userId, "decisions", adapter.listDecisions);
	const libraryPage = useFactorPage(
		userId,
		"decision-library",
		adapter.listDecisionLibrary,
	);
	const gate6Candidates = useFactorPage(
		userId,
		"gate6-candidates",
		adapter.listCandidates,
		{ allPages: true },
	);
	const gate6Datasets = useFactorPage(
		userId,
		"gate6-datasets",
		adapter.listDatasets,
		{ allPages: true },
	);
	const gate6Reports = useFactorPage(
		userId,
		"gate6-reports",
		adapter.listReports,
		{ allPages: true },
	);
	const gate6Policies = useFactorPage(
		userId,
		"gate6-policies",
		adapter.listPolicies,
		{ allPages: true },
	);
	const gate6Decisions = useFactorPage(
		userId,
		"gate6-decisions",
		adapter.listDecisionLibrary,
		{ allPages: true },
	);
	const [candidateHash, setCandidateHash] = useState("");
	const [datasetId, setDatasetId] = useState("");
	const [outputName, setOutputName] = useState("");
	const [reportHash, setReportHash] = useState("");
	const [policyHash, setPolicyHash] = useState("");
	const [decisionState, setDecisionState] =
		useState<FactorPromotionDecisionState>("rejected");
	const [supersedes, setSupersedes] = useState("");
	const [component, setComponent] = useState({
		deterministicExecution: false,
		completeSourceProvenance: false,
		abiV2Expressible: false,
		buildable: false,
	});
	const [frozenProtocol, setFrozenProtocol] = useState<FactorJson>();
	const [lineage, setLineage] = useState<FactorLineageView>();
	const [lineageLoading, setLineageLoading] = useState(false);
	const [lineageError, setLineageError] = useState<unknown>();
	const [lineageRetry, setLineageRetry] = useState(0);
	const [feedback, setFeedback] = useState<string>();
	const [feedbackTone, setFeedbackTone] = useState<"error" | "success">("error");
	const [eligibility, setEligibility] = useState<M12Eligibility>();
	const [protocolBusy, setProtocolBusy] = useState(false);
	const [decisionBusy, setDecisionBusy] = useState(false);
	const [eligibilityBusy, setEligibilityBusy] = useState(false);
	const candidateItems = candidates.data?.items ?? [];
	const datasetItems = (datasets.data?.items ?? []).filter(
		(item) => textAt(item.manifest, "candidateHash") === candidateHash,
	);
	const selectedCandidate = candidateItems.find(
		(item) => textAt(item.candidate, "candidateHash") === candidateHash,
	);
	const selectedDataset = datasetItems.find(
		(item) => textAt(item.manifest, "datasetId") === datasetId,
	);
	const outputNames = Array.isArray(
		valueAt(selectedDataset?.manifest, "outputNames"),
	)
		? (valueAt(selectedDataset?.manifest, "outputNames") as unknown[]).filter(
				(value): value is string => typeof value === "string",
			)
		: [];
	const reportItems = (reports.data?.items ?? []).filter(
		(item) =>
			textAt(item.report, "factorDatasetId") === datasetId &&
			textAt(item.report, "outputName") === outputName &&
			item.protocol,
	);
	const selectedReport = reportItems.find(
		(item) => textAt(item.report, "reportHash") === reportHash,
	);
	const selectedProtocol = selectedReport?.protocol;
	const familyId = textAt(selectedProtocol, "familyId", "");
	const trialId = textAt(selectedProtocol, "trialId", "");
	const selectedPolicy = (policies.data?.items ?? []).find(
		(item) => textAt(item.policy, "policyHash") === policyHash,
	);
	const matchingDecisions = (decisions.data?.items ?? []).filter(
		(item) =>
			textAt(item.decision, "candidateHash") === candidateHash &&
			textAt(item.decision, "outputName") === outputName,
	);
	const supersededDecisionIds = new Set(
		matchingDecisions
			.map((item) => textAt(item.decision, "supersedes", ""))
			.filter(Boolean),
	);
	const currentDecision = matchingDecisions.find(
		(item) => !supersededDecisionIds.has(textAt(item.decision, "decisionId", "")),
	);
	const selectClassName =
		"h-9 w-full rounded-md border bg-background px-3 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50";
	const clearFrozenSelection = () => {
		setFrozenProtocol(undefined);
		setEligibility(undefined);
	};
	useEffect(() => {
		setSupersedes(
			currentDecision ? textAt(currentDecision.decision, "decisionId", "") : "",
		);
	}, [currentDecision]);
	// biome-ignore lint/correctness/useExhaustiveDependencies: lineageRetry is the explicit retry trigger for this effect.
	useEffect(() => {
		if (!trialId) {
			setLineage(undefined);
			setLineageError(undefined);
			return;
		}
		let active = true;
		setLineage(undefined);
		setLineageError(undefined);
		setLineageLoading(true);
		void adapter
			.getLineage(userId, trialId)
			.then((result) => {
				if (active) setLineage(result);
			})
			.catch((error) => {
				if (active) {
					setLineage(undefined);
					setLineageError(error);
				}
			})
			.finally(() => {
				if (active) setLineageLoading(false);
			});
		return () => {
			active = false;
		};
	}, [adapter, lineageRetry, trialId, userId]);
	const freezeProtocol = async () => {
		if (
			!candidateHash ||
			!datasetId ||
			!outputName ||
			!reportHash ||
			!familyId ||
			!trialId ||
			!policyHash
		) {
			setFeedbackTone("error");
			setFeedback(t("factors.decisions.selectionRequired"));
			return;
		}
		if (!lineage) {
			setFeedbackTone("error");
			setFeedback(t("factors.decisions.lineageRequired"));
			return;
		}
		setProtocolBusy(true);
		setFeedbackTone("error");
		setFeedback(undefined);
		try {
			const result = await adapter.freezePromotionProtocol(userId, {
				candidateHash,
				datasetId,
				outputName,
				familyId,
				trialId,
				reportHashes: [reportHash],
				policyHash,
			});
			setFrozenProtocol(result);
			setFeedbackTone("success");
			setFeedback(t("factors.decisions.protocolFrozen"));
		} catch (error) {
			setFeedback(localizedFactorError(error, t));
		} finally {
			setProtocolBusy(false);
		}
	};
	const recordDecision = async () => {
		if (!frozenProtocol) {
			setFeedbackTone("error");
			setFeedback(t("factors.decisions.freezeRequired"));
			return;
		}
		setDecisionBusy(true);
		setFeedbackTone("error");
		setFeedback(undefined);
		try {
			await adapter.recordDecision(
				userId,
				decisionState,
				frozenProtocol,
				component,
				supersedes || null,
			);
			setFeedbackTone("success");
			setFeedback(t("factors.decisions.decisionSaved"));
			await decisions.load();
			await libraryPage.load();
		} catch (error) {
			setFeedback(localizedFactorError(error, t));
		} finally {
			setDecisionBusy(false);
		}
	};
	const checkEligibility = async () => {
		setEligibilityBusy(true);
		setFeedbackTone("success");
		setFeedback(undefined);
		if (!frozenProtocol) {
			setEligibilityBusy(false);
			setFeedbackTone("error");
			setFeedback(t("factors.decisions.freezeRequired"));
			return;
		}
		try {
			const result = await adapter.m12Eligibility(userId, frozenProtocol);
			setEligibility(result);
			setFeedback(
				result.eligible
					? t("factors.decisions.eligible")
					: `${t("factors.decisions.ineligible")}: ${
							result.reason
								? localizedFactorReason(result.reason, t)
								: t("factors.decisions.noReason")
						}`,
			);
		} catch (error) {
			setFeedbackTone("error");
			setFeedback(localizedFactorError(error, t));
		} finally {
			setEligibilityBusy(false);
		}
	};
	const library = libraryPage.data?.items ?? [];
	const gate6Error =
		gate6Decisions.error ??
		gate6Candidates.error ??
		gate6Datasets.error ??
		gate6Reports.error ??
		gate6Policies.error;
	return (
		<div className="space-y-5">
			<Gate6QualificationWorkspace
				userId={userId}
				adapter={adapter}
				decisions={gate6Decisions.data?.items ?? []}
				decisionLoading={
					gate6Decisions.loading ||
					gate6Candidates.loading ||
					gate6Datasets.loading ||
					gate6Reports.loading ||
					gate6Policies.loading
				}
				decisionError={gate6Error}
				candidates={gate6Candidates.data?.items ?? []}
				datasets={gate6Datasets.data?.items ?? []}
				reports={gate6Reports.data?.items ?? []}
				policies={gate6Policies.data?.items ?? []}
			/>
			<Card>
				<CardHeader>
					<CardTitle className="flex items-center gap-2">
						<GavelIcon className="size-4" aria-hidden="true" />
						{t("factors.decisions.selectionHeading")}
					</CardTitle>
					<CardDescription>
						{t("factors.decisions.selectionDescription")}
					</CardDescription>
				</CardHeader>
				<CardContent className="space-y-4">
					{candidates.error ? (
						<ErrorState
							message={localizedFactorError(candidates.error, t)}
							onRetry={() => void candidates.load()}
							retryLabel={t("factors.retry")}
							loading={candidates.loading}
						/>
					) : null}
					{datasets.error ? (
						<ErrorState
							message={localizedFactorError(datasets.error, t)}
							onRetry={() => void datasets.load()}
							retryLabel={t("factors.retry")}
							loading={datasets.loading}
						/>
					) : null}
					{reports.error ? (
						<ErrorState
							message={localizedFactorError(reports.error, t)}
							onRetry={() => void reports.load()}
							retryLabel={t("factors.retry")}
							loading={reports.loading}
						/>
					) : null}
					{policies.error ? (
						<ErrorState
							message={localizedFactorError(policies.error, t)}
							onRetry={() => void policies.load()}
							retryLabel={t("factors.retry")}
							loading={policies.loading}
						/>
					) : null}
					<div className="grid gap-3 md:grid-cols-2">
						<div className="grid gap-1.5">
							<Label htmlFor="factor-decision-candidate">
								{t("factors.decisions.candidateSelection")}
							</Label>
							<select
								id="factor-decision-candidate"
								className={selectClassName}
								value={candidateHash}
								onChange={(event) => {
									setCandidateHash(event.target.value);
									setDatasetId("");
									setOutputName("");
									setReportHash("");
									clearFrozenSelection();
								}}
							>
								<option value="">{t("factors.decisions.selectEvidence")}</option>
								{candidateItems.map((item) => {
									const hash = textAt(item.candidate, "candidateHash", "");
									return (
										<option key={hash} value={hash}>
											{`${textAt(item.presentation, "name", t("factors.common.candidate"))} · ${shortFactorHash(hash)}`}
										</option>
									);
								})}
							</select>
							{!candidates.error &&
							!candidates.loading &&
							candidateItems.length === 0 ? (
								<p className="text-xs text-muted-foreground">
									{t("factors.decisions.noCandidates")}
								</p>
							) : null}
						</div>
						<div className="grid gap-1.5">
							<Label htmlFor="factor-decision-dataset">
								{t("factors.decisions.datasetSelection")}
							</Label>
							<select
								id="factor-decision-dataset"
								className={`${selectClassName} font-mono text-xs`}
								value={datasetId}
								disabled={!candidateHash}
								onChange={(event) => {
									setDatasetId(event.target.value);
									setOutputName("");
									setReportHash("");
									clearFrozenSelection();
								}}
							>
								<option value="">{t("factors.decisions.selectEvidence")}</option>
								{datasetItems.map((item) => {
									const id = textAt(item.manifest, "datasetId", "");
									return (
										<option key={id} value={id}>
											{id}
										</option>
									);
								})}
							</select>
							{candidateHash &&
							!datasets.error &&
							!datasets.loading &&
							datasetItems.length === 0 ? (
								<p className="text-xs text-muted-foreground">
									{t("factors.decisions.noDatasets")}
								</p>
							) : null}
						</div>
						<div className="grid gap-1.5">
							<Label htmlFor="factor-decision-output">
								{t("factors.decisions.outputSelection")}
							</Label>
							<select
								id="factor-decision-output"
								className={`${selectClassName} font-mono text-xs`}
								value={outputName}
								disabled={!datasetId || outputNames.length === 0}
								onChange={(event) => {
									setOutputName(event.target.value);
									setReportHash("");
									clearFrozenSelection();
								}}
							>
								<option value="">{t("factors.decisions.selectEvidence")}</option>
								{outputNames.map((name) => (
									<option key={name} value={name}>
										{name}
									</option>
								))}
							</select>
						</div>
						<div className="grid gap-1.5">
							<Label htmlFor="factor-decision-report">
								{t("factors.decisions.reportSelection")}
							</Label>
							<select
								id="factor-decision-report"
								className={`${selectClassName} font-mono text-xs`}
								value={reportHash}
								disabled={!outputName}
								onChange={(event) => {
									setReportHash(event.target.value);
									clearFrozenSelection();
								}}
							>
								<option value="">{t("factors.decisions.selectEvidence")}</option>
								{reportItems.map((item) => {
									const hash = textAt(item.report, "reportHash", "");
									return (
										<option key={hash} value={hash}>
											{`${shortFactorHash(hash)} · ${localizedFactorCode(textAt(item.report, "evidenceState", "unknown"), t)}`}
										</option>
									);
								})}
							</select>
							{outputName &&
							!reports.error &&
							!reports.loading &&
							reportItems.length === 0 ? (
								<p className="text-xs text-muted-foreground">
									{t("factors.decisions.noReports")}
								</p>
							) : null}
						</div>
						<div className="grid gap-1.5">
							<Label htmlFor="factor-decision-policy">
								{t("factors.decisions.policySelection")}
							</Label>
							<select
								id="factor-decision-policy"
								className={`${selectClassName} font-mono text-xs`}
								value={policyHash}
								onChange={(event) => {
									setPolicyHash(event.target.value);
									clearFrozenSelection();
								}}
							>
								<option value="">{t("factors.decisions.selectEvidence")}</option>
								{(policies.data?.items ?? []).map((item) => {
									const hash = textAt(item.policy, "policyHash", "");
									return (
										<option key={hash} value={hash}>
											{`r${textAt(item.policy, "revision")} · ${shortFactorHash(hash)}`}
										</option>
									);
								})}
							</select>
							{!policies.error &&
							!policies.loading &&
							policies.data?.items.length === 0 ? (
								<p className="text-xs text-muted-foreground">
									{t("factors.decisions.noPolicies")}
								</p>
							) : null}
						</div>
					</div>
					{selectedProtocol ? (
						<div className="rounded-md border bg-muted/20 p-3">
							<p className="text-sm font-medium">
								{t("factors.decisions.selectedEvidence")}
							</p>
							<dl className="mt-3 grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
								<Detail
									label={t("factors.decisions.candidateSelection")}
									value={candidateHash}
									mono
								/>
								<Detail
									label={t("factors.decisions.datasetSelection")}
									value={datasetId}
									mono
								/>
								<Detail
									label={t("factors.decisions.reportSelection")}
									value={reportHash}
									mono
								/>
								<Detail
									label={t("factors.decisions.policySelection")}
									value={policyHash}
									mono
								/>
								<Detail label={t("factors.decisions.family")} value={familyId} mono />
								<Detail label={t("factors.decisions.trial")} value={trialId} mono />
								<Detail
									label={t("factors.decisions.evidenceState")}
									value={localizedFactorCode(
										textAt(selectedReport?.report, "evidenceState", "unknown"),
										t,
									)}
								/>
								<Detail
									label={t("factors.decisions.lineageHash")}
									value={textAt(lineage?.lineage, "lineageHash", t("factors.loading"))}
									mono
								/>
							</dl>
							{lineageLoading ? (
								<p className="mt-3 text-xs text-muted-foreground">
									{t("factors.decisions.lineageLoading")}
								</p>
							) : lineageError ? (
								<div className="mt-3">
									<ErrorState
										message={localizedFactorError(lineageError, t)}
										onRetry={() => setLineageRetry((current) => current + 1)}
										retryLabel={t("factors.retry")}
										loading={lineageLoading}
									/>
								</div>
							) : lineage ? (
								<div className="mt-3 space-y-2">
									<p className="text-xs text-muted-foreground">
										{t("factors.decisions.lineageTrials", {
											count: lineage.trials.length,
										})}
									</p>
									<ul className="grid gap-2 sm:grid-cols-2">
										{lineage.trials.map((trial) => (
											<li
												key={textAt(trial, "trialId")}
												className="flex items-center justify-between rounded-md border px-3 py-2 text-xs"
											>
												<span className="font-mono">
													{shortFactorHash(textAt(trial, "trialId"))}
												</span>
												<Badge variant="outline">
													{localizedFactorCode(textAt(trial, "status"), t)}
												</Badge>
											</li>
										))}
									</ul>
									<EvidenceJson
										label={t("factors.common.rawEvidence")}
										value={{
											candidate: selectedCandidate?.candidate,
											dataset: selectedDataset?.manifest,
											report: selectedReport?.report,
											policy: selectedPolicy?.policy,
											lineage,
										}}
									/>
								</div>
							) : null}
						</div>
					) : null}
					<div className="grid gap-3 md:grid-cols-2">
						<div className="grid gap-1.5">
							<Label htmlFor="factor-decision-state">
								{t("factors.decisions.stateSelection")}
							</Label>
							<select
								id="factor-decision-state"
								className={selectClassName}
								value={decisionState}
								onChange={(event) => {
									setDecisionState(event.target.value as FactorPromotionDecisionState);
									clearFrozenSelection();
								}}
							>
								{["rejected", "research-validated", "component-eligible"].map(
									(state) => (
										<option key={state} value={state}>
											{localizedFactorCode(state, t)}
										</option>
									),
								)}
							</select>
						</div>
						<div className="grid gap-1.5">
							<Label htmlFor="factor-decision-supersedes">
								{t("factors.decisions.supersedePrevious")}
							</Label>
							<select
								id="factor-decision-supersedes"
								className={`${selectClassName} font-mono text-xs`}
								value={supersedes}
								onChange={(event) => {
									setSupersedes(event.target.value);
									clearFrozenSelection();
								}}
							>
								<option value="">{t("factors.decisions.noPriorDecision")}</option>
								{matchingDecisions.map((item) => {
									const id = textAt(item.decision, "decisionId", "");
									return (
										<option key={id} value={id}>
											{shortFactorHash(id)}
										</option>
									);
								})}
							</select>
						</div>
					</div>
					<fieldset className="rounded-md border p-3">
						<legend className="px-1 text-sm font-medium">
							{t("factors.decisions.componentGates")}
						</legend>
						<div className="grid gap-2 sm:grid-cols-2">
							{(
								[
									["completeSourceProvenance", "complete-source-provenance"],
									["deterministicExecution", "deterministic-execution"],
									["abiV2Expressible", "abi-v2-expressible"],
									["buildable", "buildable"],
								] as const
							).map(([key, label]) => (
								<label key={key} className="flex items-center gap-2 text-sm">
									<input
										type="checkbox"
										checked={component[key]}
										onChange={(event) =>
											setComponent((current) => ({
												...current,
												[key]: event.target.checked,
											}))
										}
									/>
									{t(`factors.codes.${label}`)}
								</label>
							))}
						</div>
					</fieldset>
					<div className="flex flex-wrap gap-3">
						<Button
							type="button"
							loading={protocolBusy}
							disabled={
								!candidateHash ||
								!datasetId ||
								!outputName ||
								!reportHash ||
								!policyHash ||
								lineageLoading ||
								!lineage ||
								Boolean(lineageError)
							}
							onClick={() => void freezeProtocol()}
						>
							{t("factors.decisions.freezeProtocol")}
						</Button>
						<Button
							type="button"
							variant="outline"
							loading={decisionBusy}
							disabled={!frozenProtocol}
							onClick={() => void recordDecision()}
						>
							{t("factors.decisions.recordDecision")}
						</Button>
					</div>
					{frozenProtocol ? (
						<div className="rounded-md border border-emerald-500/40 bg-emerald-500/5 p-3 text-sm">
							<p className="font-medium">{t("factors.decisions.protocolFrozen")}</p>
							<p className="mt-1 break-all font-mono text-xs text-muted-foreground">
								{t("factors.decisions.protocol")}:{" "}
								{textAt(frozenProtocol, "protocolHash")}
							</p>
						</div>
					) : null}
					<Feedback message={feedback} tone={feedbackTone} />
				</CardContent>
			</Card>
			<Card>
				<CardHeader>
					<CardTitle>{t("factors.decisions.eligibilityHeading")}</CardTitle>
					<CardDescription>
						{t("factors.decisions.eligibilityDescription")}
					</CardDescription>
				</CardHeader>
				<CardContent className="space-y-3">
					<p className="text-sm text-muted-foreground">
						{t("factors.decisions.eligibilityHint")}
					</p>
					<Button
						type="button"
						variant="outline"
						loading={eligibilityBusy}
						disabled={!frozenProtocol}
						onClick={() => void checkEligibility()}
					>
						{t("factors.decisions.checkEligibility")}
					</Button>
					{eligibility ? (
						<div className="space-y-2" aria-live="polite">
							<p className="text-sm font-medium">
								{eligibility.eligible
									? t("factors.decisions.eligible")
									: t("factors.decisions.ineligible")}
							</p>
							<ul
								className="grid gap-2 sm:grid-cols-2"
								aria-label={t("factors.decisions.gates")}
							>
								{eligibility.gates.map((gate) => (
									<li
										key={gate.gate}
										className="flex items-center justify-between rounded-md border px-3 py-2 text-sm"
									>
										<span className="font-mono text-xs">
											{localizedFactorCode(gate.gate, t)}
										</span>
										<Badge variant={gate.passed ? "secondary" : "destructive"}>
											{gate.passed
												? t("factors.decisions.passed")
												: t("factors.decisions.failed")}
										</Badge>
									</li>
								))}
							</ul>
						</div>
					) : null}
				</CardContent>
			</Card>
			<Card>
				<CardHeader>
					<CardTitle>{t("factors.decisions.libraryHeading")}</CardTitle>
					<CardDescription>
						{t("factors.decisions.libraryDescription")}
					</CardDescription>
				</CardHeader>
				<CardContent>
					{libraryPage.loading && !libraryPage.data ? (
						<LoadingState label={t("factors.loading")} />
					) : null}
					{libraryPage.error ? (
						<ErrorState
							message={localizedFactorError(libraryPage.error, t)}
							onRetry={() => void libraryPage.load()}
							retryLabel={t("factors.retry")}
							loading={libraryPage.loading}
						/>
					) : null}
					{library.length ? (
						<div className="max-w-full overflow-x-auto">
							<table className="w-full min-w-[860px] text-sm">
								<caption className="sr-only">
									{t("factors.decisions.libraryHeading")}
								</caption>
								<thead>
									<tr className="border-b text-left text-muted-foreground">
										<th scope="col" className="py-2 pr-4">
											{t("factors.common.candidate")}
										</th>
										<th scope="col" className="py-2 pr-4">
											{t("factors.decisions.output")}
										</th>
										<th scope="col" className="py-2 pr-4">
											{t("factors.decisions.state")}
										</th>
										<th scope="col" className="py-2">
											{t("factors.decisions.reports")}
										</th>
										<th scope="col" className="py-2">
											{t("factors.common.rawEvidence")}
										</th>
									</tr>
								</thead>
								<tbody>
									{library.map((item) => (
										<tr key={textAt(item.decision, "decisionHash")} className="border-b">
											<td className="py-2 pr-4 font-mono text-xs">
												{shortFactorHash(valueAt(item.decision, "candidateHash"))}
											</td>
											<td className="py-2 pr-4">{textAt(item.decision, "outputName")}</td>
											<td className="py-2 pr-4">
												<Badge variant="secondary">
													{localizedFactorCode(textAt(item.decision, "state"), t)}
												</Badge>
											</td>
											<td className="py-2 font-mono text-xs">
												{Array.isArray(valueAt(item.decision, "reportHashes"))
													? `${(valueAt(item.decision, "reportHashes") as unknown[]).length}`
													: "—"}
											</td>
											<td className="py-2">
												<EvidenceJson
													label={t("factors.common.rawEvidence")}
													value={item}
												/>
											</td>
										</tr>
									))}
								</tbody>
							</table>
							<PageControls
								page={libraryPage.data?.page ?? 1}
								total={libraryPage.data?.total ?? 0}
								pageSize={libraryPage.data?.pageSize ?? 50}
								onPage={(page) => void libraryPage.load(page)}
							/>
						</div>
					) : libraryPage.error ? null : (
						<EmptyState message={t("factors.decisions.libraryEmpty")} />
					)}
				</CardContent>
			</Card>
			<Card>
				<CardHeader>
					<CardTitle>{t("factors.decisions.historyHeading")}</CardTitle>
					<CardDescription>
						{t("factors.decisions.historyDescription")}
					</CardDescription>
				</CardHeader>
				<CardContent className="space-y-4">
					{decisions.error ? (
						<ErrorState
							message={localizedFactorError(decisions.error, t)}
							onRetry={() => void decisions.load()}
							retryLabel={t("factors.retry")}
							loading={decisions.loading}
						/>
					) : null}
					{decisions.loading && !decisions.data ? (
						<LoadingState label={t("factors.loading")} />
					) : null}
					{decisions.data &&
					!decisions.error &&
					decisions.data.items.length === 0 ? (
						<EmptyState message={t("factors.decisions.empty")} />
					) : null}
					{decisions.data && decisions.data.items.length > 0 ? (
						<>
							<div className="max-w-full overflow-x-auto">
								<table className="w-full min-w-[760px] text-sm">
									<caption className="sr-only">
										{t("factors.decisions.historyHeading")}
									</caption>
									<thead>
										<tr className="border-b text-left text-muted-foreground">
											<th scope="col" className="py-2 pr-4">
												{t("factors.decisions.decision")}
											</th>
											<th scope="col" className="py-2 pr-4">
												{t("factors.common.candidate")}
											</th>
											<th scope="col" className="py-2 pr-4">
												{t("factors.decisions.state")}
											</th>
											<th scope="col" className="py-2 pr-4">
												{t("factors.decisions.output")}
											</th>
											<th scope="col" className="py-2 pr-4">
												{t("factors.decisions.reports")}
											</th>
											<th scope="col" className="py-2 pr-4">
												{t("factors.decisions.gates")}
											</th>
											<th scope="col" className="py-2 pr-4">
												{t("factors.decisions.policy")}
											</th>
											<th scope="col" className="py-2 pr-4">
												{t("factors.decisions.protocol")}
											</th>
											<th scope="col" className="py-2 pr-4">
												{t("factors.decisions.supersedes")}
											</th>
											<th scope="col" className="py-2">
												{t("factors.decisions.lock")}
											</th>
										</tr>
									</thead>
									<tbody>
										{decisions.data.items.map((item) => (
											<tr key={textAt(item.decision, "decisionHash")} className="border-b">
												<td className="py-2 pr-4 font-mono text-xs">
													{shortFactorHash(valueAt(item.decision, "decisionHash"))}
												</td>
												<td className="py-2 pr-4 font-mono text-xs">
													{shortFactorHash(valueAt(item.decision, "candidateHash"))}
												</td>
												<td className="py-2 pr-4">
													{localizedFactorCode(textAt(item.decision, "state"), t)}
												</td>
												<td className="py-2 pr-4">{textAt(item.decision, "outputName")}</td>
												<td className="py-2 pr-4 font-mono text-xs">
													{Array.isArray(valueAt(item.decision, "reportHashes"))
														? (valueAt(item.decision, "reportHashes") as unknown[]).length
														: 0}
												</td>
												<td className="py-2 pr-4 font-mono text-xs">
													{item.eligibilityGates.filter((gate) => gate.passed).length}/
													{item.eligibilityGates.length}
												</td>
												<td className="py-2 pr-4 font-mono text-xs">
													{shortFactorHash(valueAt(item.decision, "policyHash"))}
												</td>
												<td className="py-2 pr-4 font-mono text-xs">
													{shortFactorHash(item.promotionProtocolHash)}
												</td>
												<td className="py-2 pr-4 font-mono text-xs">
													{shortFactorHash(valueAt(item.decision, "supersedes"))}
												</td>
												<td className="py-2">
													<EvidenceJson
														label={t("factors.common.rawEvidence")}
														value={item}
													/>
												</td>
											</tr>
										))}
									</tbody>
								</table>
							</div>
							<PageControls
								page={decisions.data.page}
								total={decisions.data.total}
								pageSize={decisions.data.pageSize}
								onPage={(page) => void decisions.load(page)}
							/>
						</>
					) : null}
				</CardContent>
			</Card>
			<Card>
				<CardHeader>
					<CardTitle>{t("factors.decisions.policyHistory")}</CardTitle>
				</CardHeader>
				<CardContent>
					{policies.data?.items.length ? (
						policies.data.items.map((item) => (
							<div
								className="border-b py-3 last:border-0"
								key={textAt(item.policy, "policyHash")}
							>
								<div className="flex flex-wrap items-center gap-2">
									<Badge variant="outline">r{textAt(item.policy, "revision")}</Badge>
									<span className="font-mono text-xs">
										{shortFactorHash(valueAt(item.policy, "policyHash"))}
									</span>
								</div>
								<div className="mt-3 grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
									<Detail
										label={t("factors.codes.required-lenses")}
										value={jsonText(valueAt(item.policy, "requiredLenses"))}
									/>
									<Detail
										label={t("factors.codes.minimum-coverage")}
										value={textAt(item.policy, "minimumCoverage")}
									/>
									<Detail
										label={t("factors.codes.minimum-samples")}
										value={textAt(item.policy, "minimumSamples")}
									/>
									<Detail
										label={t("factors.codes.holm-adjusted-significance")}
										value={textAt(item.policy, "maximumHolmPValue")}
									/>
									<Detail
										label={t("factors.codes.subperiod-sign-consistency")}
										value={textAt(item.policy, "requireSubperiodSignConsistency")}
									/>
									<Detail
										label={t("factors.codes.cost-aware-outcome")}
										value={textAt(item.policy, "requireCostAwareEconomic")}
									/>
								</div>
								<EvidenceJson
									label={t("factors.decisions.policyDetails")}
									value={item.policy}
								/>
							</div>
						))
					) : policies.error ? null : policies.loading ? (
						<LoadingState label={t("factors.loading")} />
					) : (
						<EmptyState message={t("factors.decisions.noPolicies")} />
					)}
				</CardContent>
			</Card>
		</div>
	);
}
