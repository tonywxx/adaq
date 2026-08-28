import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { LockKeyholeIcon, SigmaIcon } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "@/components/ui/card";
import type { FactorAdapter } from "./factor-adapter";
import {
	parseFactorJson,
	parseFactorJsonArray,
	shortFactorHash,
} from "./factor-data";
import { AttemptsPanel } from "./factor-attempts-panel";
import { Detail, Field, TextField } from "./factor-form-fields";
import {
	EmptyState,
	ErrorState,
	EvidenceJson,
	Feedback,
	LoadingState,
	localizedFactorError,
	PageControls,
	lines,
	newUuid,
	textAt,
	valueAt,
} from "./factor-workspace-support";
import { useFactorPage } from "./factor-workspace-data";
import type { ResearchEvidenceProjection } from "@/features/research/research-context-preflight";

const FEATURE_OPERATOR_CATALOG_VERSION = "adaq-feature-operator-catalog@1.0.0";

export function CandidatesWorkspace({
	userId,
	adapter,
	context,
	contextLoading = false,
	contextError,
}: {
	userId: string;
	adapter: FactorAdapter;
	context?: ResearchEvidenceProjection | null;
	contextLoading?: boolean;
	contextError?: unknown;
}) {
	const { t } = useTranslation();
	const candidates = useFactorPage(userId, "candidates", adapter.listCandidates);
	const featureBinding = context?.featureDataset;
	const contextReady = Boolean(
		context?.universeId && featureBinding?.outputNames.length,
	);
	const [feedback, setFeedback] = useState(undefined as string | undefined);
	const [busy, setBusy] = useState(false);
	const [attemptRefresh, setAttemptRefresh] = useState(0);
	const [draft, setDraft] = useState({
		candidateId: newUuid(),
		revision: "1",
		scope: "time-series",
		slots: "feature-1",
		outputs: "factor-value",
		parameters: "[]",
		name: "",
		description: "",
		tags: "",
	});

	useEffect(() => {
		if (!featureBinding?.outputNames.length) return;
		setDraft((current) => {
			const slots = lines(current.slots).filter((slot) =>
				featureBinding.outputNames.includes(slot),
			);
			const nextSlots = slots.length ? slots : [featureBinding.outputNames[0]];
			const nextValue = nextSlots.join("\n");
			return current.slots === nextValue
				? current
				: { ...current, slots: nextValue };
		});
	}, [featureBinding]);

	const publish = async () => {
		setBusy(true);
		setFeedback(undefined);
		try {
			if (!contextReady || !featureBinding)
				throw new Error(t("factors.candidates.contextRequired"));
			const slots = lines(draft.slots).map((name) => ({ name }));
			const outputNames = lines(draft.outputs).map((name) => ({ name }));
			if (
				!draft.name.trim() ||
				slots.length === 0 ||
				outputNames.length === 0 ||
				slots.length !== outputNames.length
			)
				throw new Error(t("factors.candidates.invalidDraft"));
			const missingSlot = slots.find(
				(slot) => !featureBinding.outputNames.includes(slot.name),
			);
			if (missingSlot)
				throw new Error(
					t("factors.candidates.missingFeatureOutput", {
						name: missingSlot.name,
					}),
				);
			const parameters = parseFactorJsonArray(
				draft.parameters,
				t("factors.candidates.parameters"),
			);
			await adapter.publishCandidate(
				userId,
				{
					candidateId: draft.candidateId,
					revision: Number(draft.revision),
					scope: draft.scope,
					featureSlots: slots,
					parameters,
					outputs: outputNames,
					source: {
						kind: "declarative",
						definition: {
							featurePlanHash: featureBinding.featurePlanHash,
							operatorCatalogVersion: FEATURE_OPERATOR_CATALOG_VERSION,
							outputs: outputNames.map((output, index) => ({
								outputName: output.name,
								featureSlot: slots[index].name,
							})),
						},
					},
				},
				{
					name: draft.name.trim(),
					description: draft.description,
					tags: lines(draft.tags.replaceAll(",", "\n")),
				},
			);
			setFeedback(t("factors.candidates.published"));
			setDraft((current) => ({
				...current,
				candidateId: newUuid(),
				revision: String(Number(current.revision) + 1),
			}));
			await candidates.load();
		} catch (error) {
			setFeedback(localizedFactorError(error, t));
		} finally {
			setBusy(false);
		}
	};

	return (
		<div className="space-y-5">
			<Card>
				<CardHeader>
					<CardTitle>{t("factors.candidates.heading")}</CardTitle>
					<CardDescription>{t("factors.candidates.description")}</CardDescription>
				</CardHeader>
				<CardContent className="space-y-4">
					<div className="rounded-lg border bg-muted/20 p-3">
						{contextLoading ? (
							<p role="status" className="text-sm text-muted-foreground">
								{t("factors.candidates.contextLoading")}
							</p>
						) : contextError ? (
							<p role="alert" className="text-sm text-destructive">
								{localizedFactorError(contextError, t)}
							</p>
						) : !contextReady || !context || !featureBinding ? (
							<p role="status" className="text-sm text-muted-foreground">
								{t("factors.candidates.contextRequired")}
							</p>
						) : (
							<>
								<p className="mb-3 text-sm font-medium">
									{t("factors.candidates.context")}
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
										label={t("factors.candidates.featureDataset")}
										value={featureBinding.datasetId}
										mono
									/>
									<Detail
										label={t("factors.candidates.featurePlanHash")}
										value={featureBinding.featurePlanHash}
										mono
									/>
									<Detail
										label={t("factors.candidates.snapshot")}
										value={context.snapshotId}
										mono
									/>
									<Detail
										label={t("factors.candidates.universe")}
										value={context.universeId ?? "—"}
										mono
									/>
									<Detail
										label={t("factors.candidates.range")}
										value={`${String(context.rangeStartMs)} → ${String(context.rangeEndMs)}`}
										mono
									/>
									<Detail
										label={t("factors.candidates.operatorCatalog")}
										value={FEATURE_OPERATOR_CATALOG_VERSION}
										mono
									/>
								</dl>
								<p className="mt-3 text-xs text-muted-foreground">
									<span className="font-medium">
										{t("factors.candidates.availableOutputs")}:
									</span>{" "}
									{featureBinding.outputNames.join(", ")}
								</p>
							</>
						)}
					</div>
					<fieldset disabled={!contextReady || busy} className="space-y-4">
						<div className="grid gap-3 md:grid-cols-2">
							<Field
								label={t("factors.candidates.name")}
								value={draft.name}
								onChange={(value) =>
									setDraft((current) => ({ ...current, name: value }))
								}
							/>
							<Field
								label={t("factors.candidates.candidateId")}
								value={draft.candidateId}
								onChange={(value) =>
									setDraft((current) => ({ ...current, candidateId: value }))
								}
								mono
							/>
							<Field
								label={t("factors.candidates.revision")}
								value={draft.revision}
								onChange={(value) =>
									setDraft((current) => ({ ...current, revision: value }))
								}
								type="number"
							/>
							<div className="grid gap-1.5">
								<label htmlFor="factor-candidate-scope" className="text-sm font-medium">
									{t("factors.candidates.scope")}
								</label>
								<select
									id="factor-candidate-scope"
									className="h-9 rounded-md border bg-background px-3 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
									value={draft.scope}
									onChange={(event) =>
										setDraft((current) => ({
											...current,
											scope: event.target.value,
										}))
									}
								>
									<option value="time-series">
										{t("factors.candidates.scopeTimeSeries")}
									</option>
									<option value="cross-sectional">
										{t("factors.candidates.scopeCrossSectional")}
									</option>
								</select>
							</div>
						</div>
						<div className="grid gap-3 md:grid-cols-2">
							<TextField
								label={t("factors.candidates.featureSlots")}
								value={draft.slots}
								onChange={(value) =>
									setDraft((current) => ({ ...current, slots: value }))
								}
								hint={t("factors.candidates.onePerLine")}
							/>
							<TextField
								label={t("factors.candidates.outputs")}
								value={draft.outputs}
								onChange={(value) =>
									setDraft((current) => ({ ...current, outputs: value }))
								}
								hint={t("factors.candidates.onePerLine")}
							/>
							<TextField
								label={t("factors.candidates.parameters")}
								value={draft.parameters}
								onChange={(value) =>
									setDraft((current) => ({ ...current, parameters: value }))
								}
								hint={t("factors.candidates.parametersHint")}
							/>
							<TextField
								label={t("factors.candidates.tags")}
								value={draft.tags}
								onChange={(value) =>
									setDraft((current) => ({ ...current, tags: value }))
								}
								hint={t("factors.candidates.tagsHint")}
							/>
						</div>
						<TextField
							label={t("factors.candidates.descriptionField")}
							value={draft.description}
							onChange={(value) =>
								setDraft((current) => ({ ...current, description: value }))
							}
						/>
						<div className="flex flex-wrap items-center gap-3">
							<Button
								type="button"
								disabled={!contextReady}
								loading={busy}
								loadingText={t("factors.common.saving")}
								onClick={() => void publish()}
							>
								<SigmaIcon aria-hidden="true" />
								{t("factors.candidates.publish")}
							</Button>
							<span className="text-xs text-muted-foreground">
								{t("factors.candidates.draftNote")}
							</span>
						</div>
					</fieldset>
					<Feedback
						message={feedback}
						tone={
							feedback === t("factors.candidates.published") ? "success" : "error"
						}
					/>
				</CardContent>
			</Card>
			<CustomBuildWorkspace
				userId={userId}
				adapter={adapter}
				onQueued={() => setAttemptRefresh((current) => current + 1)}
			/>
			<AttemptsPanel
				userId={userId}
				adapter={adapter}
				kind="candidate-build"
				refreshKey={attemptRefresh}
			/>
			<Card>
				<CardHeader>
					<CardTitle>{t("factors.candidates.heading")}</CardTitle>
					<CardDescription>
						{t("factors.candidates.listDescription")}
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
					{candidates.loading && !candidates.data ? (
						<LoadingState label={t("factors.loading")} />
					) : null}
					{candidates.data &&
					!candidates.error &&
					candidates.data.items.length === 0 ? (
						<EmptyState message={t("factors.candidates.empty")} />
					) : null}
					{candidates.data && candidates.data.items.length > 0 ? (
						<>
							<div className="max-w-full overflow-x-auto">
								<table className="w-full min-w-[900px] text-sm">
									<caption className="sr-only">
										{t("factors.candidates.heading")}
									</caption>
									<thead>
										<tr className="border-b text-left text-muted-foreground">
											<th scope="col" className="py-2 pr-4">
												{t("factors.common.identity")}
											</th>
											<th scope="col" className="py-2 pr-4">
												{t("factors.common.scope")}
											</th>
											<th scope="col" className="py-2 pr-4">
												{t("factors.candidates.source")}
											</th>
											<th scope="col" className="py-2 pr-4">
												{t("factors.candidates.predecessor")}
											</th>
											<th scope="col" className="py-2 pr-4">
												{t("factors.candidates.lock")}
											</th>
											<th scope="col" className="py-2">
												{t("factors.common.rawEvidence")}
											</th>
										</tr>
									</thead>
									<tbody>
										{candidates.data.items.map((item) => (
											<tr
												key={textAt(item.candidate, "candidateHash")}
												className="border-b align-top"
											>
												<td className="py-3 pr-4">
													<div className="font-mono text-xs">
														{shortFactorHash(valueAt(item.candidate, "candidateHash"))}
													</div>
													<div className="text-xs text-muted-foreground">
														r{textAt(item.candidate, "revision")}
													</div>
												</td>
												<td className="py-3 pr-4">{textAt(item.candidate, "scope")}</td>
												<td className="py-3 pr-4">
													{textAt(item.candidate, "source.kind")}
												</td>
												<td className="py-3 pr-4">
													{item.predecessor ? (
														<>
															<div className="font-mono text-xs">
																{item.predecessor.featureDataset.datasetId}
															</div>
															<div className="font-mono text-xs text-muted-foreground">
																r{item.predecessor.contextRevision} ·{" "}
																{shortFactorHash(item.predecessor.contextHash)}
															</div>
														</>
													) : (
														"—"
													)}
												</td>
												<td className="py-3 pr-4">
													{item.lockedBy.length ? (
														<Badge variant="outline">
															<LockKeyholeIcon aria-hidden="true" />
															{t("factors.common.locked")}
														</Badge>
													) : (
														<Badge variant="secondary">{t("factors.common.unlocked")}</Badge>
													)}
												</td>
												<td className="py-3">
													<EvidenceJson
														label={item.presentation.name || t("factors.candidates.details")}
														value={item}
													/>
												</td>
											</tr>
										))}
									</tbody>
								</table>
							</div>
							<PageControls
								page={candidates.data.page}
								total={candidates.data.total}
								pageSize={candidates.data.pageSize}
								onPage={(page) => void candidates.load(page)}
							/>
						</>
					) : null}
				</CardContent>
			</Card>
		</div>
	);
}

function CustomBuildWorkspace({
	userId,
	adapter,
	onQueued,
}: {
	userId: string;
	adapter: FactorAdapter;
	onQueued: () => void;
}) {
	const { t } = useTranslation();
	const [candidate, setCandidate] = useState("");
	const [name, setName] = useState("");
	const [projectRoot, setProjectRoot] = useState("");
	const [sourceSha256, setSourceSha256] = useState("");
	const [sdkVersion, setSdkVersion] = useState("");
	const [fuel, setFuel] = useState("1000000");
	const [memory, setMemory] = useState("67108864");
	const [busy, setBusy] = useState(false);
	const [feedback, setFeedback] = useState(undefined as string | undefined);
	const build = async () => {
		setBusy(true);
		setFeedback(undefined);
		try {
			await adapter.buildCandidate(
				userId,
				parseFactorJson(candidate, t("factors.candidates.template")),
				{ name, description: "", tags: [] },
				{
					projectRoot,
					sourceSha256,
					sdkVersion,
					toolchain: "stable",
					target: "wasm32-unknown-unknown",
					resourcePolicy: { fuelPerCall: Number(fuel), memoryBytes: Number(memory) },
				},
			);
			setFeedback(t("factors.candidates.buildQueued"));
			onQueued();
		} catch (error) {
			setFeedback(localizedFactorError(error, t));
		} finally {
			setBusy(false);
		}
	};
	return (
		<Card>
			<CardHeader>
				<CardTitle>{t("factors.candidates.customHeading")}</CardTitle>
				<CardDescription>
					{t("factors.candidates.customDescription")}
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-4">
				<TextField
					label={t("factors.candidates.customTemplate")}
					value={candidate}
					onChange={setCandidate}
					hint={t("factors.candidates.customTemplateHint")}
				/>
				<div className="grid gap-3 md:grid-cols-2">
					<Field
						label={t("factors.candidates.name")}
						value={name}
						onChange={setName}
					/>
					<Field
						label={t("factors.candidates.projectRoot")}
						value={projectRoot}
						onChange={setProjectRoot}
						mono
					/>
					<Field
						label={t("factors.candidates.sourceHash")}
						value={sourceSha256}
						onChange={setSourceSha256}
						mono
					/>
					<Field
						label={t("factors.candidates.sdkVersion")}
						value={sdkVersion}
						onChange={setSdkVersion}
					/>
					<Field
						label={t("factors.candidates.fuel")}
						value={fuel}
						onChange={setFuel}
						type="number"
					/>
					<Field
						label={t("factors.candidates.memory")}
						value={memory}
						onChange={setMemory}
						type="number"
					/>
				</div>
				<div className="flex flex-wrap items-center gap-3">
					<Button
						type="button"
						loading={busy}
						loadingText={t("factors.common.queueing")}
						onClick={() => void build()}
					>
						{t("factors.candidates.build")}
					</Button>
					<span className="text-xs text-muted-foreground">
						{t("factors.candidates.notImported")}
					</span>
				</div>
				<Feedback
					message={feedback}
					tone={
						feedback === t("factors.candidates.buildQueued") ? "success" : "error"
					}
				/>
			</CardContent>
		</Card>
	);
}
