import { useState } from "react";
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
	formatFactorError,
	parseFactorJson,
	parseFactorJsonArray,
	shortFactorHash,
} from "./factor-data";
import { AttemptsPanel } from "./factor-attempts-panel";
import { Field, TextField } from "./factor-form-fields";
import {
	EmptyState,
	ErrorState,
	EvidenceJson,
	Feedback,
	LoadingState,
	PageControls,
	lines,
	newUuid,
	textAt,
	valueAt,
} from "./factor-workspace-support";
import { useFactorPage } from "./factor-workspace-data";

export function CandidatesWorkspace({
	userId,
	adapter,
}: {
	userId: string;
	adapter: FactorAdapter;
}) {
	const { t } = useTranslation();
	const candidates = useFactorPage(userId, "candidates", adapter.listCandidates);
	const [feedback, setFeedback] = useState(undefined as string | undefined);
	const [busy, setBusy] = useState(false);
	const [attemptRefresh, setAttemptRefresh] = useState(0);
	const [draft, setDraft] = useState({
		candidateId: newUuid(),
		revision: "1",
		scope: "time-series",
		featurePlanHash: "",
		operatorCatalogVersion: "adaq-feature-operator-catalog@1.0.0",
		slots: "feature-1",
		outputs: "factor-value",
		parameters: "[]",
		name: "",
		description: "",
		tags: "",
	});

	const publish = async () => {
		setBusy(true);
		setFeedback(undefined);
		try {
			const slots = lines(draft.slots).map((name) => ({ name }));
			const outputNames = lines(draft.outputs).map((name) => ({ name }));
			if (
				!draft.name.trim() ||
				slots.length === 0 ||
				outputNames.length === 0 ||
				slots.length !== outputNames.length
			)
				throw new Error(t("factors.candidates.invalidDraft"));
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
							featurePlanHash: draft.featurePlanHash,
							operatorCatalogVersion: draft.operatorCatalogVersion,
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
			setFeedback(formatFactorError(error));
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
						<Field
							label={t("factors.candidates.scope")}
							value={draft.scope}
							onChange={(value) =>
								setDraft((current) => ({ ...current, scope: value }))
							}
						/>
						<Field
							label={t("factors.candidates.featurePlanHash")}
							value={draft.featurePlanHash}
							onChange={(value) =>
								setDraft((current) => ({ ...current, featurePlanHash: value }))
							}
							mono
						/>
						<Field
							label={t("factors.candidates.operatorCatalog")}
							value={draft.operatorCatalogVersion}
							onChange={(value) =>
								setDraft((current) => ({ ...current, operatorCatalogVersion: value }))
							}
							mono
						/>
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
					{candidates.error && !candidates.data ? (
						<ErrorState
							message={candidates.error}
							onRetry={() => void candidates.load()}
							retryLabel={t("factors.retry")}
						/>
					) : null}
					{candidates.loading && !candidates.data ? (
						<LoadingState label={t("factors.loading")} />
					) : null}
					{candidates.data && candidates.data.items.length === 0 ? (
						<EmptyState message={t("factors.candidates.empty")} />
					) : null}
					{candidates.data && candidates.data.items.length > 0 ? (
						<>
							<div className="max-w-full overflow-x-auto">
								<table className="w-full min-w-[760px] text-sm">
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
			setFeedback(formatFactorError(error));
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
