import { invoke } from "@tauri-apps/api/core";
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
	factorJsonArray,
	factorString,
	formatFactorError,
	isGridWithinLimit,
	parseFactorJson,
	shortFactorHash,
} from "./factor-data";
import type {
	FactorCandidateView,
	FactorDatasetRow,
	FactorDatasetView,
	FactorJson,
	FactorLineageView,
	FactorMetricCatalogView,
	FactorReportView,
	M12Eligibility,
} from "./factor-types";
import { PythonProjectsPanel } from "@/features/python-research/python-projects-panel";
import { PythonFactorLabPanel } from "@/features/python-research/python-factor-lab-panel";
import { CandidatesWorkspace } from "./candidates-workspace";
import { AttemptsPanel } from "./factor-attempts-panel";
import { Detail, Field, JsonEditor, TextField } from "./factor-form-fields";
import { useFactorPage } from "./factor-workspace-data";
import {
	commaSeparated,
	commaSeparatedNumbers,
	EmptyState,
	ErrorState,
	EvidenceJson,
	Feedback,
	jsonText,
	lines,
	localizedFactorCode,
	localizedFactorReason,
	mergeFactorFields,
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

	const inspect = async (familyId: string, trialId: string) => {
		setLineageLoading(familyId);
		setFeedback(undefined);
		try {
			const details = await adapter.getLineage(userId, trialId);
			setLineage((current) => ({ ...current, [familyId]: details }));
		} catch (error) {
			setFeedback(formatFactorError(error));
		} finally {
			setLineageLoading(undefined);
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
					{families.error && !families.data ? (
						<ErrorState
							message={families.error}
							onRetry={() => void families.load()}
							retryLabel={t("factors.retry")}
						/>
					) : null}
					{families.loading && !families.data ? (
						<LoadingState label={t("factors.loading")} />
					) : null}
					{families.data && families.data.items.length === 0 ? (
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
			setFeedback(formatFactorError(error));
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
						{candidates.error}
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
	const inspect = async (item: FactorDatasetView) => {
		const id = textAt(item.manifest, "datasetId");
		lastFocus.current = document.activeElement as HTMLElement | null;
		setLoadingAction(id);
		setFeedback(undefined);
		try {
			setSelected(await adapter.getDataset(userId, id));
		} catch (error) {
			setFeedback(formatFactorError(error));
		} finally {
			setLoadingAction(undefined);
		}
	};
	const remove = async (item: FactorDatasetView) => {
		const id = textAt(item.manifest, "datasetId");
		setDeletingId(id);
		setFeedback(undefined);
		try {
			await adapter.deleteDataset(userId, id);
			setSelected(undefined);
			setFeedback(t("factors.datasets.deleted"));
			await datasets.load();
		} catch (error) {
			setFeedback(formatFactorError(error));
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
					{datasets.error && !datasets.data ? (
						<ErrorState
							message={datasets.error}
							onRetry={() => void datasets.load()}
							retryLabel={t("factors.retry")}
						/>
					) : null}
					{datasets.loading && !datasets.data ? (
						<LoadingState label={t("factors.loading")} />
					) : null}
					{datasets.data && datasets.data.items.length === 0 ? (
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
					onClose={() => setSelected(undefined)}
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
				setError(formatFactorError(loadError));
			} finally {
				if (version === requestVersion.current) setLoading(false);
			}
		},
		[adapter, datasetId, userId],
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
					{reports.error && !reports.data ? (
						<ErrorState
							message={reports.error}
							onRetry={() => void reports.load()}
							retryLabel={t("factors.retry")}
						/>
					) : null}
					{reports.loading && !reports.data ? (
						<LoadingState label={t("factors.loading")} />
					) : null}
					{reports.data && reports.data.items.length === 0 ? (
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
															const reportHash = textAt(item.report, "reportHash");
															lastFocus.current = document.activeElement as HTMLElement | null;
															setLoadingReportId(reportHash);
															try {
																setSelected(await adapter.getReport(userId, reportHash));
															} catch (error) {
																setFeedback(formatFactorError(error));
															} finally {
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
					onClose={() => setSelected(undefined)}
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
	const [protocol, setProtocol] = useState("{}");
	const [marketSeries, setMarketSeries] = useState("[]");
	const [featureEvidence, setFeatureEvidence] = useState("");
	const [factorDatasetId, setFactorDatasetId] = useState("");
	const [featureDatasetId, setFeatureDatasetId] = useState("");
	const [featurePlanHash, setFeaturePlanHash] = useState("");
	const [snapshotId, setSnapshotId] = useState("");
	const [universeId, setUniverseId] = useState("");
	const [pointInTimeUniverse, setPointInTimeUniverse] = useState("");
	const [outputName, setOutputName] = useState("");
	const [scope, setScope] = useState("");
	const [target, setTarget] = useState("");
	const [horizonBars, setHorizonBars] = useState("");
	const [orientation, setOrientation] = useState("");
	const [purgeBars, setPurgeBars] = useState("");
	const [embargoBars, setEmbargoBars] = useState("");
	const [lenses, setLenses] = useState("");
	const [nuisanceFeatureNames, setNuisanceFeatureNames] = useState("");
	const [familyId, setFamilyId] = useState("");
	const [trialId, setTrialId] = useState("");
	const [seed, setSeed] = useState("");
	const [busy, setBusy] = useState(false);
	const [feedback, setFeedback] = useState<string>();
	const [frozenHash, setFrozenHash] = useState<string>();
	const factorContextQuery = useResearchEvidenceContext(userId);
	const factorContext = factorContextQuery.data;
	const featureBinding = factorContext?.featureDataset;

	useEffect(() => {
		if (!featureBinding || !factorContext) return;
		setFeatureDatasetId(featureBinding.datasetId);
		setFeaturePlanHash(featureBinding.featurePlanHash);
		setSnapshotId(factorContext.snapshotId);
		setUniverseId(factorContext.universeId ?? "");
	}, [factorContext, featureBinding]);

	const start = async () => {
		setBusy(true);
		setFeedback(undefined);
		try {
			const factorProtocol = applyFactorContext(
				mergeFactorFields(protocol, t("factors.evaluations.protocol"), {
					factorDatasetId,
					featureDatasetId,
					featurePlanHash,
					marketDataSnapshotId: snapshotId,
					pointInTimeUniverseId: universeId,
					pointInTimeUniverse: commaSeparated(pointInTimeUniverse),
					outputName,
					scope,
					target,
					horizonBars: commaSeparatedNumbers(horizonBars),
					orientation,
					purgeBars: optionalNumber(purgeBars),
					embargoBars: optionalNumber(embargoBars),
					lenses: commaSeparated(lenses),
					nuisanceFeatureNames: commaSeparated(nuisanceFeatureNames),
					familyId,
					trialId,
					seed: optionalNumber(seed),
				}),
				factorContext,
			);
			const frozen = await adapter.freezeEvaluationProtocol(
				userId,
				factorProtocol,
			);
			setFrozenHash(textAt(frozen, "protocolHash"));
			await adapter.startEvaluation(
				userId,
				frozen,
				factorJsonArray(JSON.parse(marketSeries)),
				featureEvidence.trim()
					? parseFactorJson(
							featureEvidence,
							t("factors.evaluations.featureEvidence"),
						)
					: undefined,
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
					<Field
						label={t("factors.evaluations.dataset")}
						value={factorDatasetId}
						onChange={setFactorDatasetId}
						mono
					/>
					<Field
						label={t("factors.datasets.featureDataset")}
						value={featureDatasetId}
						onChange={setFeatureDatasetId}
						disabled={Boolean(featureBinding)}
						mono
					/>
					<Field
						label={t("factors.candidates.featurePlanHash")}
						value={featurePlanHash}
						onChange={setFeaturePlanHash}
						disabled={Boolean(featureBinding)}
						mono
					/>
					<Field
						label={t("factors.datasets.snapshot")}
						value={snapshotId}
						onChange={setSnapshotId}
						disabled={Boolean(featureBinding)}
						mono
					/>
					<Field
						label={t("factors.datasets.universe")}
						value={universeId}
						onChange={setUniverseId}
						disabled={Boolean(featureBinding)}
						mono
					/>
					<Field
						label={t("factors.evaluations.output")}
						value={outputName}
						onChange={setOutputName}
						mono
					/>
					<Field
						label={t("factors.evaluations.target")}
						value={target}
						onChange={setTarget}
						placeholder="future-close-return"
					/>
					<Field
						label={t("factors.common.scope")}
						value={scope}
						onChange={setScope}
						placeholder="pooled"
					/>
					<Field
						label={t("factors.evaluations.orientation")}
						value={orientation}
						onChange={setOrientation}
						placeholder="positive"
					/>
					<Field
						label={t("factors.evaluations.horizons")}
						value={horizonBars}
						onChange={setHorizonBars}
						placeholder="1, 5, 20"
					/>
					<Field
						label={t("factors.evaluations.purgeEmbargo")}
						value={purgeBars}
						onChange={setPurgeBars}
						type="number"
						placeholder="purge bars"
					/>
					<Field
						label={t("factors.evaluations.embargoBars")}
						value={embargoBars}
						onChange={setEmbargoBars}
						type="number"
						placeholder="embargo bars"
					/>
					<Field
						label={t("factors.evaluations.lenses")}
						value={lenses}
						onChange={setLenses}
						placeholder="temporal, economic"
					/>
					<Field
						label={t("factors.evaluations.pointInTimeUniverse")}
						value={pointInTimeUniverse}
						onChange={setPointInTimeUniverse}
						placeholder="VENUE:SYMBOL, ..."
						mono
					/>
					<Field
						label={t("factors.evaluations.nuisance")}
						value={nuisanceFeatureNames}
						onChange={setNuisanceFeatureNames}
						placeholder="feature-a, feature-b"
					/>
					<Field
						label={t("factors.families.familyId")}
						value={familyId}
						onChange={setFamilyId}
						mono
					/>
					<Field
						label={t("factors.families.trialId")}
						value={trialId}
						onChange={setTrialId}
						mono
					/>
					<Field
						label={t("features.materialization.seed")}
						value={seed}
						onChange={setSeed}
						type="number"
					/>
				</div>
				<TextField
					label={t("factors.evaluations.protocol")}
					value={protocol}
					onChange={setProtocol}
					hint={t("factors.evaluations.protocolHint")}
				/>
				{frozenHash ? (
					<Detail
						label={t("factors.evaluations.frozenProtocolHash")}
						value={frozenHash}
						mono
					/>
				) : null}
				<TextField
					label={t("factors.evaluations.marketSeries")}
					value={marketSeries}
					onChange={setMarketSeries}
					hint={t("factors.evaluations.marketSeriesHint")}
				/>
				<TextField
					label={t("factors.evaluations.featureEvidence")}
					value={featureEvidence}
					onChange={setFeatureEvidence}
					hint={t("factors.evaluations.featureEvidenceHint")}
				/>
				<div className="flex flex-wrap items-center gap-3">
					<Button
						type="button"
						loading={busy}
						loadingText={t("factors.common.queueing")}
						onClick={() => void start()}
					>
						{t("factors.evaluations.start")}
					</Button>
					<span className="text-xs text-muted-foreground">
						{t("factors.evaluations.noRandomSplit")}
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

function DecisionsWorkspace({
	userId,
	adapter,
}: {
	userId: string;
	adapter: FactorAdapter;
}) {
	const { t } = useTranslation();
	const policies = useFactorPage(userId, "policies", adapter.listPolicies);
	const decisions = useFactorPage(userId, "decisions", adapter.listDecisions);
	const libraryPage = useFactorPage(
		userId,
		"decision-library",
		adapter.listDecisionLibrary,
	);
	const [policy, setPolicy] = useState("{}");
	const [decision, setDecision] = useState("{}");
	const [protocol, setProtocol] = useState("{}");
	const [component, setComponent] = useState("{}");
	const [eligibilityProtocol, setEligibilityProtocol] = useState("{}");
	const [feedback, setFeedback] = useState<string>();
	const [feedbackTone, setFeedbackTone] = useState<"error" | "success">("error");
	const [eligibility, setEligibility] = useState<M12Eligibility>();
	const [policyBusy, setPolicyBusy] = useState(false);
	const [decisionBusy, setDecisionBusy] = useState(false);
	const [eligibilityBusy, setEligibilityBusy] = useState(false);
	const savePolicy = async () => {
		setPolicyBusy(true);
		setFeedbackTone("error");
		setFeedback(undefined);
		try {
			await adapter.savePolicy(
				userId,
				parseFactorJson(policy, t("factors.decisions.policy")),
			);
			setFeedbackTone("success");
			setFeedback(t("factors.decisions.policySaved"));
			await policies.load();
		} catch (error) {
			setFeedback(formatFactorError(error));
		} finally {
			setPolicyBusy(false);
		}
	};
	const saveDecision = async () => {
		setDecisionBusy(true);
		setFeedbackTone("error");
		setFeedback(undefined);
		try {
			await adapter.saveDecision(
				userId,
				parseFactorJson(decision, t("factors.decisions.decision")),
				parseFactorJson(protocol, t("factors.decisions.protocol")),
				parseFactorJson(component, t("factors.decisions.component")),
			);
			setFeedbackTone("success");
			setFeedback(t("factors.decisions.decisionSaved"));
			await decisions.load();
			await libraryPage.load();
		} catch (error) {
			setFeedback(formatFactorError(error));
		} finally {
			setDecisionBusy(false);
		}
	};
	const checkEligibility = async () => {
		setEligibilityBusy(true);
		setFeedbackTone("success");
		setFeedback(undefined);
		try {
			const result = await adapter.m12Eligibility(
				userId,
				parseFactorJson(eligibilityProtocol, t("factors.decisions.protocol")),
			);
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
			setFeedback(formatFactorError(error));
		} finally {
			setEligibilityBusy(false);
		}
	};
	const library = libraryPage.data?.items ?? [];
	return (
		<div className="space-y-5">
			<Card>
				<CardHeader>
					<CardTitle className="flex items-center gap-2">
						<GavelIcon className="size-4" aria-hidden="true" />
						{t("factors.decisions.heading")}
					</CardTitle>
					<CardDescription>{t("factors.decisions.description")}</CardDescription>
				</CardHeader>
				<CardContent className="space-y-4">
					<div className="grid gap-4 xl:grid-cols-2">
						<JsonEditor
							label={t("factors.decisions.policyEditor")}
							value={policy}
							onChange={setPolicy}
							hint={t("factors.decisions.policyHint")}
						/>
						<JsonEditor
							label={t("factors.decisions.decisionEditor")}
							value={decision}
							onChange={setDecision}
							hint={t("factors.decisions.decisionHint")}
						/>
						<JsonEditor
							label={t("factors.decisions.protocolEditor")}
							value={protocol}
							onChange={setProtocol}
							hint={t("factors.decisions.protocolHint")}
						/>
						<JsonEditor
							label={t("factors.decisions.componentEditor")}
							value={component}
							onChange={setComponent}
							hint={t("factors.decisions.componentHint")}
						/>
					</div>
					<div className="flex flex-wrap gap-3">
						<Button
							type="button"
							loading={policyBusy}
							onClick={() => void savePolicy()}
						>
							{t("factors.decisions.savePolicy")}
						</Button>
						<Button
							type="button"
							variant="outline"
							loading={decisionBusy}
							onClick={() => void saveDecision()}
						>
							{t("factors.decisions.recordDecision")}
						</Button>
					</div>
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
					<JsonEditor
						label={t("factors.decisions.protocolEditor")}
						value={eligibilityProtocol}
						onChange={setEligibilityProtocol}
						hint={t("factors.decisions.eligibilityHint")}
					/>
					<Button
						type="button"
						variant="outline"
						loading={eligibilityBusy}
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
					{libraryPage.error && !libraryPage.data ? (
						<ErrorState
							message={libraryPage.error}
							onRetry={() => void libraryPage.load()}
							retryLabel={t("factors.retry")}
						/>
					) : null}
					{library.length ? (
						<div className="max-w-full overflow-x-auto">
							<table className="w-full min-w-[720px] text-sm">
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
					) : (
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
					{decisions.error && !decisions.data ? (
						<ErrorState
							message={decisions.error}
							onRetry={() => void decisions.load()}
							retryLabel={t("factors.retry")}
						/>
					) : null}
					{decisions.loading && !decisions.data ? (
						<LoadingState label={t("factors.loading")} />
					) : null}
					{decisions.data && decisions.data.items.length === 0 ? (
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
					{policies.error && !policies.data ? (
						<ErrorState
							message={policies.error}
							onRetry={() => void policies.load()}
							retryLabel={t("factors.retry")}
						/>
					) : null}
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
					) : policies.loading ? (
						<LoadingState label={t("factors.loading")} />
					) : (
						<EmptyState message={t("factors.decisions.noPolicies")} />
					)}
				</CardContent>
			</Card>
		</div>
	);
}
