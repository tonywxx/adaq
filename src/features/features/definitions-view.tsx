import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { formatNumber } from "@/lib/i18n";
import { readSessionCache, writeSessionCache } from "@/lib/session-cache";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { DefinitionEditor } from "./definition-editor";
import type { FeaturesAdapter } from "./features-adapter";
import {
	camelReason,
	createEmptyDraft,
	formatFeatureError,
	parseStoredDefinition,
	parseTimestampInput,
} from "./features-data";
import {
	FeaturesEmpty,
	FeaturesError,
	FeaturesLoading,
	formatUtc,
} from "./features-shared";
import type {
	ArtifactView,
	DefinitionDraft,
	DefinitionView,
	DraftValidationView,
	FeatureObservation,
	MarketDataSnapshotSummary,
	UniverseSnapshotSummary,
} from "./features-types";

type EditorState = {
	draft: DefinitionDraft;
	name: string;
	description: string;
	tagsText: string;
};

type PreviewState = {
	running: boolean;
	error?: string;
	observations?: FeatureObservation[];
	eventCount?: number;
	truncated?: boolean;
};

export function DefinitionsView({
	userId,
	adapter,
}: {
	userId: string;
	adapter: FeaturesAdapter;
}) {
	const { t } = useTranslation();
	const [definitions, setDefinitions] = useState<DefinitionView[]>();
	const [loadError, setLoadError] = useState<string>();
	const [editor, setEditor] = useState<EditorState | null>(null);
	const [validation, setValidation] = useState<DraftValidationView | null>(null);
	const [busy, setBusy] = useState<"validate" | "publish" | null>(null);
	const [feedback, setFeedback] = useState<{
		kind: "ok" | "error";
		text: string;
	}>();
	const [snapshots, setSnapshots] = useState<MarketDataSnapshotSummary[]>([]);
	const [universes, setUniverses] = useState<UniverseSnapshotSummary[]>([]);
	const [artifacts, setArtifacts] = useState<ArtifactView[]>([]);
	const [preview, setPreview] = useState<PreviewState>({ running: false });
	const [previewSelection, setPreviewSelection] = useState({
		snapshotId: "",
		universeId: "",
		startTime: "",
		endTime: "",
		maxEvents: "100",
		artifactIds: [] as string[],
	});

	const loadDefinitions = useCallback(async () => {
		try {
			const items = await adapter.listDefinitions(userId);
			writeSessionCache(userId, "definitions", items);
			setDefinitions(items);
			setLoadError(undefined);
		} catch (error) {
			setLoadError(formatFeatureError(error));
		}
	}, [adapter, userId]);

	useEffect(() => {
		// Current-session cache paints re-entry instantly; the fetch below is
		// the background refresh.
		setDefinitions(
			readSessionCache(userId, "definitions") as DefinitionView[] | undefined,
		);
		loadDefinitions();
		Promise.all([
			adapter.listSnapshots(userId).catch(() => []),
			adapter.listUniverseSnapshots(userId).catch(() => []),
			adapter.listArtifacts(userId).catch(() => []),
		]).then(([snapshotItems, universeItems, artifactItems]) => {
			setSnapshots(snapshotItems);
			setUniverses(universeItems);
			setArtifacts(artifactItems);
		});
	}, [adapter, userId, loadDefinitions]);

	const openNewDraft = () => {
		setEditor({
			draft: createEmptyDraft(),
			name: "",
			description: "",
			tagsText: "",
		});
		setValidation(null);
		setFeedback(undefined);
	};

	const openPublished = (definition: DefinitionView) => {
		const stored = parseStoredDefinition(definition.definitionJson);
		if (!stored) {
			setFeedback({ kind: "error", text: t("features.definitions.error") });
			return;
		}
		setEditor({
			draft: {
				definitionId: stored.definitionId,
				revision: stored.revision + 1,
				scope: stored.scope,
				nodes: stored.nodes,
				outputs: stored.outputs,
			},
			name: definition.name,
			description: definition.description,
			tagsText: definition.tags.join(", "),
		});
		setValidation(null);
		setFeedback(undefined);
	};

	const validate = async () => {
		if (!editor) return;
		setBusy("validate");
		setFeedback(undefined);
		try {
			const result = await adapter.validateDraft(userId, editor.draft);
			setValidation(result);
		} catch (error) {
			setFeedback({ kind: "error", text: formatFeatureError(error) });
		} finally {
			setBusy(null);
		}
	};

	const publish = async () => {
		if (!editor) return;
		setBusy("publish");
		setFeedback(undefined);
		try {
			const published = await adapter.publishDefinition(userId, editor.draft, {
				name: editor.name,
				description: editor.description,
				tags: editor.tagsText
					.split(",")
					.map((tag) => tag.trim())
					.filter(Boolean),
			});
			setFeedback({
				kind: "ok",
				text: t("features.definitions.published", {
					hash: published.definitionHash.slice(0, 12),
				}),
			});
			setEditor(null);
			setValidation(null);
			await loadDefinitions();
		} catch (error) {
			setFeedback({ kind: "error", text: formatFeatureError(error) });
		} finally {
			setBusy(null);
		}
	};

	const runPreview = async () => {
		if (!editor) return;
		setPreview({ running: true });
		try {
			const result = await adapter.previewDraft(userId, editor.draft, {
				snapshotId: previewSelection.snapshotId || undefined,
				universeId: previewSelection.universeId || undefined,
				startTimeMs: parseTimestampInput(previewSelection.startTime),
				endTimeMs: parseTimestampInput(previewSelection.endTime),
				maxEvents: Number.parseInt(previewSelection.maxEvents, 10) || undefined,
				artifactIds: previewSelection.artifactIds,
			});
			setPreview({
				running: false,
				observations: result.observations,
				eventCount: result.eventCount,
				truncated: result.truncated,
			});
		} catch (error) {
			setPreview({ running: false, error: formatFeatureError(error) });
		}
	};

	return (
		<div className="space-y-6">
			<Card>
				<CardHeader className="flex-row items-center justify-between space-y-0">
					<CardTitle>{t("features.definitions.heading")}</CardTitle>
					<Button type="button" size="sm" onClick={openNewDraft}>
						{t("features.definitions.new")}
					</Button>
				</CardHeader>
				<CardContent>
					{loadError && definitions === undefined ? (
						<FeaturesError
							message={loadError}
							onRetry={loadDefinitions}
							retryLabel={t("features.retryLoad")}
						/>
					) : definitions === undefined ? (
						<FeaturesLoading label={t("features.loading")} />
					) : definitions.length === 0 ? (
						<FeaturesEmpty message={t("features.definitions.empty")} />
					) : (
						<div className="max-w-full overflow-x-auto">
							<table className="w-full text-sm">
								<thead>
									<tr className="border-b text-left text-muted-foreground">
										<th className="py-2 pr-4 font-medium">{t("features.form.name")}</th>
										<th className="py-2 pr-4 font-medium">
											{t("features.definitions.revision")}
										</th>
										<th className="py-2 pr-4 font-medium">
											{t("features.definitions.hash")}
										</th>
										<th className="py-2 pr-4 font-medium">
											{t("features.definitions.created")}
										</th>
										<th className="py-2 font-medium" />
									</tr>
								</thead>
								<tbody>
									{definitions.map((definition) => (
										<tr key={definition.definitionHash} className="border-b">
											<td className="py-2 pr-4">
												{definition.name || definition.definitionId}
											</td>
											<td className="py-2 pr-4">r{definition.revision}</td>
											<td className="py-2 pr-4 font-mono text-xs">
												{definition.definitionHash.slice(0, 16)}…
											</td>
											<td className="py-2 pr-4 whitespace-nowrap">
												{formatUtc(definition.createdAtMs)} UTC
											</td>
											<td className="py-2 text-right">
												<Button
													type="button"
													variant="outline"
													size="sm"
													onClick={() => openPublished(definition)}
												>
													{t("features.definitions.edit")}
												</Button>
											</td>
										</tr>
									))}
								</tbody>
							</table>
						</div>
					)}
				</CardContent>
			</Card>

			<div aria-live="polite">
				{feedback && (
					<p
						role={feedback.kind === "error" ? "alert" : undefined}
						className={
							feedback.kind === "error"
								? "text-sm text-destructive"
								: "text-sm text-emerald-600 dark:text-emerald-400"
						}
					>
						{feedback.text}
					</p>
				)}
			</div>

			{editor && (
				<Card>
					<CardHeader>
						<CardTitle className="text-base">
							{t("features.definitions.editor.draftHint")} ·{" "}
							<span className="font-mono text-sm">
								{editor.draft.definitionId} · r{editor.draft.revision}
							</span>
						</CardTitle>
					</CardHeader>
					<CardContent className="space-y-6">
						<div className="grid gap-3 sm:grid-cols-3">
							<div>
								<Label htmlFor="definition-name">{t("features.form.name")}</Label>
								<Input
									id="definition-name"
									className="mt-1"
									value={editor.name}
									onChange={(event) =>
										setEditor({ ...editor, name: event.target.value })
									}
								/>
							</div>
							<div>
								<Label htmlFor="definition-description">
									{t("features.form.description")}
								</Label>
								<Input
									id="definition-description"
									className="mt-1"
									value={editor.description}
									onChange={(event) =>
										setEditor({
											...editor,
											description: event.target.value,
										})
									}
								/>
							</div>
							<div>
								<Label htmlFor="definition-tags">{t("features.form.tags")}</Label>
								<Input
									id="definition-tags"
									className="mt-1"
									value={editor.tagsText}
									onChange={(event) =>
										setEditor({ ...editor, tagsText: event.target.value })
									}
								/>
							</div>
						</div>

						<DefinitionEditor
							draft={editor.draft}
							onChange={(draft) => setEditor({ ...editor, draft })}
						/>

						<div className="flex flex-wrap gap-2">
							<Button
								type="button"
								variant="outline"
								disabled={busy !== null}
								onClick={validate}
							>
								{t("features.definitions.validate")}
							</Button>
							<Button type="button" disabled={busy !== null} onClick={publish}>
								{t("features.definitions.publish")}
							</Button>
							<Button
								type="button"
								variant="ghost"
								onClick={() => {
									setEditor(null);
									setValidation(null);
								}}
							>
								{t("features.definitions.editor.cancel")}
							</Button>
						</div>

						{validation && (
							<div
								role="status"
								className={
									validation.valid
										? "rounded-md border border-emerald-500/40 bg-emerald-500/5 p-3 text-sm"
										: "rounded-md border border-destructive/40 bg-destructive/5 p-3 text-sm"
								}
							>
								{validation.valid ? (
									<p>{t("features.definitions.validationOk")}</p>
								) : (
									<>
										<p className="font-medium">
											{t("features.definitions.validationFailed")}
										</p>
										<ul className="mt-1 list-inside list-disc font-mono text-xs">
											{validation.issues.map((issue) => (
												<li key={`${issue.code}-${issue.path ?? "root"}`}>
													{issue.code}
													{issue.path ? ` → ${issue.path}` : ""}
												</li>
											))}
										</ul>
									</>
								)}
							</div>
						)}
					</CardContent>
				</Card>
			)}

			{editor && (
				<Card>
					<CardHeader>
						<CardTitle className="text-base">
							{t("features.preview.heading")}
						</CardTitle>
						<p className="text-xs text-muted-foreground">
							{t("features.preview.hint")}
						</p>
					</CardHeader>
					<CardContent className="space-y-4">
						<div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
							<div>
								<Label htmlFor="preview-snapshot">{t("features.form.snapshot")}</Label>
								<select
									id="preview-snapshot"
									className="mt-1 w-full rounded-md border bg-transparent px-2 py-1.5 text-sm"
									value={previewSelection.snapshotId}
									onChange={(event) =>
										setPreviewSelection({
											...previewSelection,
											snapshotId: event.target.value,
										})
									}
								>
									<option value="">{t("features.form.none")}</option>
									{snapshots.map((snapshot) => (
										<option key={snapshot.snapshotId} value={snapshot.snapshotId}>
											{snapshot.code} {snapshot.interval} ·{" "}
											{snapshot.snapshotId.slice(0, 8)}
										</option>
									))}
								</select>
							</div>
							<div>
								<Label htmlFor="preview-universe">{t("features.form.universe")}</Label>
								<select
									id="preview-universe"
									className="mt-1 w-full rounded-md border bg-transparent px-2 py-1.5 text-sm"
									value={previewSelection.universeId}
									onChange={(event) =>
										setPreviewSelection({
											...previewSelection,
											universeId: event.target.value,
										})
									}
								>
									<option value="">{t("features.form.none")}</option>
									{universes.map((universe) => (
										<option key={universe.snapshotId} value={universe.snapshotId}>
											{universe.venue} {universe.interval} ·{" "}
											{universe.snapshotId.slice(0, 8)}
										</option>
									))}
								</select>
							</div>
							<div>
								<Label htmlFor="preview-max-events">
									{t("features.preview.maxEvents")}
								</Label>
								<Input
									id="preview-max-events"
									className="mt-1"
									type="number"
									min={1}
									max={500}
									value={previewSelection.maxEvents}
									onChange={(event) =>
										setPreviewSelection({
											...previewSelection,
											maxEvents: event.target.value,
										})
									}
								/>
							</div>
							<div>
								<Label htmlFor="preview-start">{t("features.form.startTime")}</Label>
								<Input
									id="preview-start"
									className="mt-1"
									type="datetime-local"
									value={previewSelection.startTime}
									onChange={(event) =>
										setPreviewSelection({
											...previewSelection,
											startTime: event.target.value,
										})
									}
								/>
							</div>
							<div>
								<Label htmlFor="preview-end">{t("features.form.endTime")}</Label>
								<Input
									id="preview-end"
									className="mt-1"
									type="datetime-local"
									value={previewSelection.endTime}
									onChange={(event) =>
										setPreviewSelection({
											...previewSelection,
											endTime: event.target.value,
										})
									}
								/>
							</div>
						</div>

						{artifacts.length > 0 && (
							<fieldset>
								<legend className="mb-1 text-sm font-medium">
									{t("features.preview.artifacts")}
								</legend>
								<div className="flex flex-wrap gap-3">
									{artifacts.map((artifact) => (
										<label
											key={artifact.artifactId}
											className="flex items-center gap-1.5 text-xs"
										>
											<input
												type="checkbox"
												checked={previewSelection.artifactIds.includes(artifact.artifactId)}
												onChange={(event) =>
													setPreviewSelection({
														...previewSelection,
														artifactIds: event.target.checked
															? [...previewSelection.artifactIds, artifact.artifactId]
															: previewSelection.artifactIds.filter(
																	(id) => id !== artifact.artifactId,
																),
													})
												}
											/>
											<span className="font-mono">
												{artifact.artifactId.slice(0, 12)}…
											</span>
										</label>
									))}
								</div>
							</fieldset>
						)}

						<Button
							type="button"
							variant="outline"
							disabled={preview.running}
							onClick={runPreview}
						>
							{t("features.preview.run")}
						</Button>

						{preview.running && <FeaturesLoading label={t("features.loading")} />}
						{preview.error && <FeaturesError message={preview.error} />}
						{preview.observations && (
							<div className="space-y-2">
								<p className="text-xs text-muted-foreground">
									{t("features.preview.eventCount", {
										count: formatNumber(preview.eventCount ?? 0),
									})}
									{preview.truncated ? ` · ${t("features.preview.truncated")}` : ""}
								</p>
								{preview.observations.length === 0 ? (
									<FeaturesEmpty message={t("features.preview.empty")} />
								) : (
									<div className="max-w-full overflow-x-auto">
										<table className="w-full text-xs">
											<thead>
												<tr className="border-b text-left text-muted-foreground">
													<th className="py-1.5 pr-3 font-medium">
														{t("features.preview.outputName")}
													</th>
													<th className="py-1.5 pr-3 font-medium">
														{t("features.preview.instrument")}
													</th>
													<th className="py-1.5 pr-3 font-medium">
														{t("features.preview.time")}
													</th>
													<th className="py-1.5 pr-3 font-medium">
														{t("features.preview.state")}
													</th>
													<th className="py-1.5 pr-3 font-medium">
														{t("features.preview.value")}
													</th>
												</tr>
											</thead>
											<tbody>
												{preview.observations.map((observation) => (
													<tr
														key={`${observation.outputName}-${observation.instrumentId}-${observation.observationTimeMs}`}
														className="border-b"
													>
														<td className="py-1.5 pr-3 font-mono">
															{observation.outputName}
														</td>
														<td className="py-1.5 pr-3">{observation.instrumentId}</td>
														<td className="py-1.5 pr-3 whitespace-nowrap">
															{formatUtc(observation.observationTimeMs)}
														</td>
														{observation.value.state === "available" ? (
															<>
																<td className="py-1.5 pr-3">
																	{t("features.datasets.table.stateAvailable")}
																</td>
																<td className="py-1.5 pr-3 font-mono">
																	{observation.value.value}
																</td>
															</>
														) : (
															<>
																<td className="py-1.5 pr-3">
																	{t("features.datasets.table.stateUnavailable")}
																</td>
																<td className="py-1.5 pr-3 font-mono">
																	{t(
																		`features.unavailability.${camelReason(observation.value.reason)}`,
																		{ defaultValue: observation.value.reason },
																	)}
																</td>
															</>
														)}
													</tr>
												))}
											</tbody>
										</table>
									</div>
								)}
							</div>
						)}
					</CardContent>
				</Card>
			)}
		</div>
	);
}
