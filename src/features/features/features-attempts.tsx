import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { formatNumber } from "@/lib/i18n";
import { readSessionCache, writeSessionCache } from "@/lib/session-cache";
import { ResearchContextEvidence } from "@/features/research/research-context-evidence";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { FeaturesAdapter } from "./features-adapter";
import {
	EMPTY_ENGINE_IDENTITY,
	attemptProgressFraction,
	emptyPlanDraft,
	formatFeatureError,
	isTerminalAttemptStatus,
	parseStoredDefinition,
	parseTimestampInput,
} from "./features-data";
import {
	AttemptStatusBadge,
	FeaturesEmpty,
	FeaturesError,
	FeaturesLoading,
	formatUtc,
} from "./features-shared";
import type {
	DefinitionView,
	FittingAttemptView,
	MarketDataSnapshotSummary,
	MaterializationAttempt,
	StoredDefinition,
	UniverseSnapshotSummary,
} from "./features-types";

const POLL_INTERVAL_MS = 3_000;

type EvidenceSelection = {
	definitionHash: string;
	nodeId: string;
	outputName: string;
	snapshotId: string;
	universeId: string;
};

function useEvidenceOptions(userId: string, adapter: FeaturesAdapter) {
	const [definitions, setDefinitions] = useState<DefinitionView[]>();
	const [snapshots, setSnapshots] = useState<MarketDataSnapshotSummary[]>([]);
	const [universes, setUniverses] = useState<UniverseSnapshotSummary[]>([]);
	const [error, setError] = useState<string>();

	const load = useCallback(async () => {
		try {
			const items = await adapter.listDefinitions(userId);
			writeSessionCache(userId, "definitions", items);
			setDefinitions(items);
			setError(undefined);
		} catch (loadError) {
			setError(formatFeatureError(loadError));
		}
	}, [adapter, userId]);

	useEffect(() => {
		setDefinitions(
			readSessionCache(userId, "definitions") as DefinitionView[] | undefined,
		);
		load();
		Promise.all([
			adapter.listSnapshots(userId).catch(() => []),
			adapter.listUniverseSnapshots(userId).catch(() => []),
		]).then(([snapshotItems, universeItems]) => {
			setSnapshots(snapshotItems);
			setUniverses(universeItems);
		});
	}, [adapter, userId, load]);

	return { definitions, snapshots, universes, error, reload: load };
}

function storedFor(
	definitions: DefinitionView[] | undefined,
	definitionHash: string,
): StoredDefinition | undefined {
	const view = definitions?.find(
		(item) => item.definitionHash === definitionHash,
	);
	return view
		? (parseStoredDefinition(view.definitionJson) ?? undefined)
		: undefined;
}

export function FittingView({
	userId,
	adapter,
}: {
	userId: string;
	adapter: FeaturesAdapter;
}) {
	const { t } = useTranslation();
	const options = useEvidenceOptions(userId, adapter);
	const [attempts, setAttempts] = useState<FittingAttemptView[]>();
	const [loadError, setLoadError] = useState<string>();
	const [actionFeedback, setActionFeedback] = useState<{
		kind: "ok" | "error";
		text: string;
	}>();

	const [form, setForm] = useState({
		definitionHash: "",
		nodeId: "",
		outputName: "",
		fittedOutputName: "fitted-output",
		snapshotId: "",
		universeId: "",
		windowStart: "",
		windowEnd: "",
		scope: "pooled-universe" as "pooled-universe" | "per-instrument",
		algorithm: "standardization" as "standardization" | "winsorization",
		lowerQuantile: "0.05",
		upperQuantile: "0.95",
		minimumSamples: "30",
	});
	const [starting, setStarting] = useState(false);

	const load = useCallback(async () => {
		try {
			const items = await adapter.listFittingAttempts(userId);
			writeSessionCache(userId, "fitting", items);
			setAttempts(items);
			setLoadError(undefined);
		} catch (error) {
			setLoadError(formatFeatureError(error));
		}
	}, [adapter, userId]);

	useEffect(() => {
		setAttempts(
			readSessionCache(userId, "fitting") as FittingAttemptView[] | undefined,
		);
		load();
	}, [userId, load]);

	useEffect(() => {
		if (!attempts?.some((attempt) => !isTerminalAttemptStatus(attempt.status)))
			return;
		const timer = setInterval(load, POLL_INTERVAL_MS);
		return () => clearInterval(timer);
	}, [attempts, load]);

	const stored = storedFor(options.definitions, form.definitionHash);

	const start = async () => {
		setStarting(true);
		setActionFeedback(undefined);
		try {
			const definitionView = options.definitions?.find(
				(item) => item.definitionHash === form.definitionHash,
			);
			if (!definitionView || !stored) {
				throw new Error(t("features.fitting.startInvalid"));
			}
			const windowStart = parseTimestampInput(form.windowStart);
			const windowEnd = parseTimestampInput(form.windowEnd);
			if (windowStart === undefined || windowEnd === undefined) {
				throw new Error(t("features.fitting.startInvalid"));
			}
			const reference = {
				definitionHash: definitionView.definitionHash,
				nodeId: form.nodeId,
				outputName: form.outputName,
			};
			await adapter.startFitting(
				userId,
				{
					inputFeature: reference,
					fittedNodeId: form.nodeId,
					fittedOutput: {
						definitionHash: definitionView.definitionHash,
						nodeId: form.nodeId,
						outputName: form.fittedOutputName,
					},
					snapshotId: form.snapshotId,
					pointInTimeUniverseId: form.universeId,
					fittingScope: form.scope,
					fittingWindow: { startTimeMs: windowStart, endTimeMs: windowEnd },
					algorithm:
						form.algorithm === "standardization"
							? { kind: "standardization" }
							: {
									kind: "winsorization",
									lower_quantile: Number.parseFloat(form.lowerQuantile),
									upper_quantile: Number.parseFloat(form.upperQuantile),
									quantile_method_version: "nearest-rank@1.0.0",
								},
					minimumSamples: Number.parseInt(form.minimumSamples, 10) || 0,
					engineIdentity: EMPTY_ENGINE_IDENTITY,
				},
				emptyPlanDraft([stored]),
			);
			setActionFeedback({ kind: "ok", text: t("features.fitting.started") });
			await load();
		} catch (error) {
			setActionFeedback({ kind: "error", text: formatFeatureError(error) });
		} finally {
			setStarting(false);
		}
	};

	const cancel = async (attemptId: string) => {
		setActionFeedback(undefined);
		try {
			await adapter.cancelFitting(userId, attemptId);
			await load();
		} catch (error) {
			setActionFeedback({ kind: "error", text: formatFeatureError(error) });
		}
	};

	const retry = async (attemptId: string) => {
		setActionFeedback(undefined);
		try {
			await adapter.retryFitting(userId, attemptId);
			await load();
		} catch (error) {
			setActionFeedback({ kind: "error", text: formatFeatureError(error) });
		}
	};

	return (
		<div className="space-y-6">
			<Card>
				<CardHeader>
					<CardTitle>{t("features.fitting.heading")}</CardTitle>
					{attempts?.[0] ? (
						<ResearchContextEvidence
							userId={userId}
							attemptId={attempts[0].attemptId}
						/>
					) : null}
				</CardHeader>
				<CardContent>
					{loadError && attempts === undefined ? (
						<FeaturesError
							message={loadError}
							onRetry={load}
							retryLabel={t("features.retryLoad")}
						/>
					) : attempts === undefined ? (
						<FeaturesLoading label={t("features.loading")} />
					) : attempts.length === 0 ? (
						<FeaturesEmpty message={t("features.fitting.empty")} />
					) : (
						<ul className="space-y-3">
							{attempts.map((attempt) => (
								<li key={attempt.attemptId} className="rounded-md border p-3 text-sm">
									<div className="flex flex-wrap items-center gap-2">
										<span className="font-mono text-xs">
											{attempt.attemptId.slice(0, 16)}…
										</span>
										<AttemptStatusBadge status={attempt.status} />
										<span className="text-xs text-muted-foreground">
											{formatNumber(attempt.progressCompleted)} /{" "}
											{formatNumber(attempt.progressTotal)}
										</span>
										<span className="text-xs text-muted-foreground">
											{formatUtc(attempt.createdAtMs)} UTC
										</span>
										<div className="ml-auto flex gap-1">
											{(attempt.status === "pending" || attempt.status === "running") && (
												<Button
													type="button"
													variant="outline"
													size="sm"
													onClick={() => cancel(attempt.attemptId)}
												>
													{t("features.fitting.cancel")}
												</Button>
											)}
											{(attempt.status === "failed" || attempt.status === "cancelled") && (
												<Button
													type="button"
													variant="outline"
													size="sm"
													onClick={() => retry(attempt.attemptId)}
												>
													{t("features.fitting.retry")}
												</Button>
											)}
										</div>
									</div>
									<div
										role="progressbar"
										aria-valuemin={0}
										aria-valuemax={100}
										aria-valuenow={Math.round(
											attemptProgressFraction(
												attempt.progressCompleted,
												attempt.progressTotal,
											) * 100,
										)}
										className="mt-2 h-1.5 w-full overflow-hidden rounded bg-muted"
									>
										<div
											className="h-full bg-foreground/60"
											style={{
												width: `${attemptProgressFraction(attempt.progressCompleted, attempt.progressTotal) * 100}%`,
											}}
										/>
									</div>
									<dl className="mt-2 grid gap-x-6 gap-y-1 text-xs text-muted-foreground sm:grid-cols-2">
										<div>
											<dt className="inline font-medium">
												{t("features.fitting.protocolHash")}:{" "}
											</dt>
											<dd className="inline font-mono">
												{attempt.protocolHash.slice(0, 16)}…
											</dd>
										</div>
										<div>
											<dt className="inline font-medium">
												{t("features.fitting.planHash")}:{" "}
											</dt>
											<dd className="inline font-mono">
												{attempt.planHash.slice(0, 16)}…
											</dd>
										</div>
										{attempt.sourceAttemptId && (
											<div>
												<dt className="inline font-medium">
													{t("features.fitting.sourceAttempt")}:{" "}
												</dt>
												<dd className="inline font-mono">
													{attempt.sourceAttemptId.slice(0, 16)}…
												</dd>
											</div>
										)}
										{attempt.artifactId && (
											<div>
												<dt className="inline font-medium">
													{t("features.fitting.artifact")}:{" "}
												</dt>
												<dd className="inline font-mono">
													{attempt.artifactId.slice(0, 16)}…
												</dd>
											</div>
										)}
									</dl>
									{attempt.diagnostic && (
										<pre className="mt-2 max-w-full overflow-x-auto whitespace-pre-wrap rounded bg-muted p-2 text-xs">
											{attempt.failureCode ? `${attempt.failureCode}: ` : ""}
											{attempt.diagnostic}
										</pre>
									)}
								</li>
							))}
						</ul>
					)}
				</CardContent>
			</Card>

			<Card>
				<CardHeader>
					<CardTitle className="text-base">
						{t("features.fitting.startHeading")}
					</CardTitle>
				</CardHeader>
				<CardContent className="space-y-4">
					<div aria-live="polite">
						{actionFeedback && (
							<p
								role={actionFeedback.kind === "error" ? "alert" : undefined}
								className={
									actionFeedback.kind === "error"
										? "text-sm text-destructive"
										: "text-sm text-emerald-600 dark:text-emerald-400"
								}
							>
								{actionFeedback.text}
							</p>
						)}
					</div>
					<div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
						<EvidenceSelectors
							prefix="fitting"
							options={options}
							selection={{
								definitionHash: form.definitionHash,
								nodeId: form.nodeId,
								outputName: form.outputName,
								snapshotId: form.snapshotId,
								universeId: form.universeId,
							}}
							onChange={(selection) => setForm({ ...form, ...selection })}
						/>
						<div>
							<Label htmlFor="fitting-output-name">
								{t("features.fitting.fittedOutputName")}
							</Label>
							<Input
								id="fitting-output-name"
								className="mt-1"
								value={form.fittedOutputName}
								onChange={(event) =>
									setForm({ ...form, fittedOutputName: event.target.value })
								}
							/>
						</div>
						<div>
							<Label htmlFor="fitting-window-start">
								{t("features.form.startTime")}
							</Label>
							<Input
								id="fitting-window-start"
								className="mt-1"
								type="datetime-local"
								value={form.windowStart}
								onChange={(event) =>
									setForm({ ...form, windowStart: event.target.value })
								}
							/>
						</div>
						<div>
							<Label htmlFor="fitting-window-end">{t("features.form.endTime")}</Label>
							<Input
								id="fitting-window-end"
								className="mt-1"
								type="datetime-local"
								value={form.windowEnd}
								onChange={(event) =>
									setForm({ ...form, windowEnd: event.target.value })
								}
							/>
						</div>
						<div>
							<Label htmlFor="fitting-scope">{t("features.fitting.scope")}</Label>
							<select
								id="fitting-scope"
								className="mt-1 w-full rounded-md border bg-transparent px-2 py-1.5 text-sm"
								value={form.scope}
								onChange={(event) =>
									setForm({
										...form,
										scope: event.target.value as "pooled-universe" | "per-instrument",
									})
								}
							>
								<option value="pooled-universe">
									{t("features.fitting.scopePooled")}
								</option>
								<option value="per-instrument">
									{t("features.fitting.scopePerInstrument")}
								</option>
							</select>
						</div>
						<div>
							<Label htmlFor="fitting-algorithm">
								{t("features.fitting.algorithm")}
							</Label>
							<select
								id="fitting-algorithm"
								className="mt-1 w-full rounded-md border bg-transparent px-2 py-1.5 text-sm"
								value={form.algorithm}
								onChange={(event) =>
									setForm({
										...form,
										algorithm: event.target.value as "standardization" | "winsorization",
									})
								}
							>
								<option value="standardization">
									{t("features.fitting.standardization")}
								</option>
								<option value="winsorization">
									{t("features.fitting.winsorization")}
								</option>
							</select>
						</div>
						{form.algorithm === "winsorization" && (
							<>
								<div>
									<Label htmlFor="fitting-lower-quantile">
										{t("features.fitting.lowerQuantile")}
									</Label>
									<Input
										id="fitting-lower-quantile"
										className="mt-1"
										type="number"
										step="0.01"
										value={form.lowerQuantile}
										onChange={(event) =>
											setForm({ ...form, lowerQuantile: event.target.value })
										}
									/>
								</div>
								<div>
									<Label htmlFor="fitting-upper-quantile">
										{t("features.fitting.upperQuantile")}
									</Label>
									<Input
										id="fitting-upper-quantile"
										className="mt-1"
										type="number"
										step="0.01"
										value={form.upperQuantile}
										onChange={(event) =>
											setForm({ ...form, upperQuantile: event.target.value })
										}
									/>
								</div>
							</>
						)}
						<div>
							<Label htmlFor="fitting-min-samples">
								{t("features.fitting.minimumSamples")}
							</Label>
							<Input
								id="fitting-min-samples"
								className="mt-1"
								type="number"
								min={0}
								value={form.minimumSamples}
								onChange={(event) =>
									setForm({ ...form, minimumSamples: event.target.value })
								}
							/>
						</div>
					</div>
					<Button
						type="button"
						disabled={starting || !stored || !form.snapshotId}
						onClick={start}
					>
						{t("features.fitting.start")}
					</Button>
				</CardContent>
			</Card>
		</div>
	);
}

export function MaterializationView({
	userId,
	adapter,
}: {
	userId: string;
	adapter: FeaturesAdapter;
}) {
	const { t } = useTranslation();
	const options = useEvidenceOptions(userId, adapter);
	const [attempts, setAttempts] = useState<MaterializationAttempt[]>();
	const [loadError, setLoadError] = useState<string>();
	const [actionFeedback, setActionFeedback] = useState<{
		kind: "ok" | "error";
		text: string;
	}>();
	const [form, setForm] = useState({
		definitionHash: "",
		nodeId: "",
		outputName: "",
		snapshotId: "",
		universeId: "",
		rangeStart: "",
		rangeEnd: "",
		seed: "1",
	});
	const [starting, setStarting] = useState(false);

	const load = useCallback(async () => {
		try {
			const items = await adapter.listMaterializationAttempts(userId);
			writeSessionCache(userId, "materialization", items);
			setAttempts(items);
			setLoadError(undefined);
		} catch (error) {
			setLoadError(formatFeatureError(error));
		}
	}, [adapter, userId]);

	useEffect(() => {
		setAttempts(
			readSessionCache(userId, "materialization") as
				| MaterializationAttempt[]
				| undefined,
		);
		load();
	}, [userId, load]);

	useEffect(() => {
		if (!attempts?.some((attempt) => !isTerminalAttemptStatus(attempt.status)))
			return;
		const timer = setInterval(load, POLL_INTERVAL_MS);
		return () => clearInterval(timer);
	}, [attempts, load]);

	const stored = storedFor(options.definitions, form.definitionHash);

	const start = async () => {
		setStarting(true);
		setActionFeedback(undefined);
		try {
			if (!stored) throw new Error(t("features.materialization.startInvalid"));
			const startTimeMs = parseTimestampInput(form.rangeStart);
			const endTimeMs = parseTimestampInput(form.rangeEnd);
			if (startTimeMs === undefined || endTimeMs === undefined) {
				throw new Error(t("features.materialization.startInvalid"));
			}
			const plan = emptyPlanDraft([stored]);
			// The Plan hash is an immutable evidence identity; only the native
			// engine can compute it, so the GUI asks the module to freeze first.
			const frozen = await adapter.freezePlan(userId, plan);
			await adapter.startMaterialization(
				userId,
				{
					userId,
					featurePlanHash: frozen.planHash,
					snapshotId: form.snapshotId,
					pointInTimeUniverseId: form.universeId,
					observationRange: { startTimeMs, endTimeMs },
					parameters: {},
					seed: Number.parseInt(form.seed, 10) || 0,
				},
				plan,
			);
			setActionFeedback({
				kind: "ok",
				text: t("features.materialization.started"),
			});
			await load();
		} catch (error) {
			setActionFeedback({ kind: "error", text: formatFeatureError(error) });
		} finally {
			setStarting(false);
		}
	};

	const cancel = async (attemptId: string) => {
		setActionFeedback(undefined);
		try {
			await adapter.cancelMaterialization(userId, attemptId);
			await load();
		} catch (error) {
			setActionFeedback({ kind: "error", text: formatFeatureError(error) });
		}
	};

	const retry = async (attemptId: string) => {
		setActionFeedback(undefined);
		try {
			await adapter.retryMaterialization(userId, attemptId);
			await load();
		} catch (error) {
			setActionFeedback({ kind: "error", text: formatFeatureError(error) });
		}
	};

	return (
		<div className="space-y-6">
			<Card>
				<CardHeader>
					<CardTitle>{t("features.materialization.heading")}</CardTitle>
					{attempts?.[0] ? (
						<ResearchContextEvidence
							userId={userId}
							attemptId={attempts[0].attemptId}
						/>
					) : null}
				</CardHeader>
				<CardContent>
					{loadError && attempts === undefined ? (
						<FeaturesError
							message={loadError}
							onRetry={load}
							retryLabel={t("features.retryLoad")}
						/>
					) : attempts === undefined ? (
						<FeaturesLoading label={t("features.loading")} />
					) : attempts.length === 0 ? (
						<FeaturesEmpty message={t("features.materialization.empty")} />
					) : (
						<ul className="space-y-3">
							{attempts.map((attempt) => (
								<li key={attempt.attemptId} className="rounded-md border p-3 text-sm">
									<div className="flex flex-wrap items-center gap-2">
										<span className="font-mono text-xs">
											{attempt.attemptId.slice(0, 16)}…
										</span>
										<AttemptStatusBadge status={attempt.status} />
										<span className="text-xs text-muted-foreground">
											{formatNumber(attempt.progressCompleted)} /{" "}
											{formatNumber(attempt.progressTotal)}
										</span>
										<span className="text-xs text-muted-foreground">
											{formatUtc(attempt.createdAtMs)} UTC
										</span>
										<div className="ml-auto flex gap-1">
											{(attempt.status === "pending" || attempt.status === "running") && (
												<Button
													type="button"
													variant="outline"
													size="sm"
													onClick={() => cancel(attempt.attemptId)}
												>
													{t("features.materialization.cancel")}
												</Button>
											)}
											{(attempt.status === "failed" || attempt.status === "cancelled") && (
												<Button
													type="button"
													variant="outline"
													size="sm"
													onClick={() => retry(attempt.attemptId)}
												>
													{t("features.materialization.retry")}
												</Button>
											)}
										</div>
									</div>
									<div
										role="progressbar"
										aria-valuemin={0}
										aria-valuemax={100}
										aria-valuenow={Math.round(
											attemptProgressFraction(
												attempt.progressCompleted,
												attempt.progressTotal,
											) * 100,
										)}
										className="mt-2 h-1.5 w-full overflow-hidden rounded bg-muted"
									>
										<div
											className="h-full bg-foreground/60"
											style={{
												width: `${attemptProgressFraction(attempt.progressCompleted, attempt.progressTotal) * 100}%`,
											}}
										/>
									</div>
									<dl className="mt-2 grid gap-x-6 gap-y-1 text-xs text-muted-foreground sm:grid-cols-2">
										<div>
											<dt className="inline font-medium">
												{t("features.materialization.requestHash")}:{" "}
											</dt>
											<dd className="inline font-mono">
												{attempt.requestHash.slice(0, 16)}…
											</dd>
										</div>
										{attempt.sourceAttemptId && (
											<div>
												<dt className="inline font-medium">
													{t("features.materialization.sourceAttempt")}:{" "}
												</dt>
												<dd className="inline font-mono">
													{attempt.sourceAttemptId.slice(0, 16)}…
												</dd>
											</div>
										)}
										{attempt.datasetId && (
											<div>
												<dt className="inline font-medium">
													{t("features.materialization.dataset")}:{" "}
												</dt>
												<dd className="inline font-mono">
													{attempt.datasetId.slice(0, 16)}…
												</dd>
											</div>
										)}
									</dl>
									{attempt.diagnostic && (
										<pre className="mt-2 max-w-full overflow-x-auto whitespace-pre-wrap rounded bg-muted p-2 text-xs">
											{attempt.failureCode ? `${attempt.failureCode}: ` : ""}
											{attempt.diagnostic}
										</pre>
									)}
								</li>
							))}
						</ul>
					)}
				</CardContent>
			</Card>

			<Card>
				<CardHeader>
					<CardTitle className="text-base">
						{t("features.materialization.startHeading")}
					</CardTitle>
					<p className="text-xs text-muted-foreground">
						{t("features.materialization.crossSectionalBlocked")}
					</p>
				</CardHeader>
				<CardContent className="space-y-4">
					<div aria-live="polite">
						{actionFeedback && (
							<p
								role={actionFeedback.kind === "error" ? "alert" : undefined}
								className={
									actionFeedback.kind === "error"
										? "text-sm text-destructive"
										: "text-sm text-emerald-600 dark:text-emerald-400"
								}
							>
								{actionFeedback.text}
							</p>
						)}
					</div>
					<div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
						<EvidenceSelectors
							prefix="materialization"
							options={options}
							selection={{
								definitionHash: form.definitionHash,
								nodeId: form.nodeId,
								outputName: form.outputName,
								snapshotId: form.snapshotId,
								universeId: form.universeId,
							}}
							onChange={(selection) => setForm({ ...form, ...selection })}
						/>
						<div>
							<Label htmlFor="materialization-range-start">
								{t("features.form.startTime")}
							</Label>
							<Input
								id="materialization-range-start"
								className="mt-1"
								type="datetime-local"
								value={form.rangeStart}
								onChange={(event) =>
									setForm({ ...form, rangeStart: event.target.value })
								}
							/>
						</div>
						<div>
							<Label htmlFor="materialization-range-end">
								{t("features.form.endTime")}
							</Label>
							<Input
								id="materialization-range-end"
								className="mt-1"
								type="datetime-local"
								value={form.rangeEnd}
								onChange={(event) => setForm({ ...form, rangeEnd: event.target.value })}
							/>
						</div>
						<div>
							<Label htmlFor="materialization-seed">
								{t("features.materialization.seed")}
							</Label>
							<Input
								id="materialization-seed"
								className="mt-1"
								type="number"
								min={0}
								value={form.seed}
								onChange={(event) => setForm({ ...form, seed: event.target.value })}
							/>
						</div>
					</div>
					<Button
						type="button"
						disabled={starting || !stored || !form.snapshotId}
						onClick={start}
					>
						{t("features.materialization.start")}
					</Button>
				</CardContent>
			</Card>
		</div>
	);
}

function EvidenceSelectors({
	prefix,
	options,
	selection,
	onChange,
}: {
	prefix: string;
	options: ReturnType<typeof useEvidenceOptions>;
	selection: EvidenceSelection;
	onChange: (selection: Partial<EvidenceSelection>) => void;
}) {
	const { t } = useTranslation();
	const stored = storedFor(options.definitions, selection.definitionHash);
	return (
		<>
			<div>
				<Label htmlFor={`${prefix}-definition`}>
					{t("features.form.definition")}
				</Label>
				<select
					id={`${prefix}-definition`}
					className="mt-1 w-full rounded-md border bg-transparent px-2 py-1.5 text-sm"
					value={selection.definitionHash}
					onChange={(event) =>
						onChange({
							definitionHash: event.target.value,
							nodeId: "",
							outputName: "",
						})
					}
				>
					<option value="">{t("features.form.none")}</option>
					{(options.definitions ?? []).map((definition) => (
						<option key={definition.definitionHash} value={definition.definitionHash}>
							{definition.name || definition.definitionId} · r{definition.revision}
						</option>
					))}
				</select>
			</div>
			<div>
				<Label htmlFor={`${prefix}-node`}>{t("features.form.node")}</Label>
				<select
					id={`${prefix}-node`}
					className="mt-1 w-full rounded-md border bg-transparent px-2 py-1.5 text-sm"
					value={selection.nodeId}
					onChange={(event) =>
						onChange({ nodeId: event.target.value, outputName: "" })
					}
				>
					<option value="">{t("features.form.none")}</option>
					{(stored?.nodes ?? []).map((node) => (
						<option key={node.id} value={node.id}>
							{node.id}
						</option>
					))}
				</select>
			</div>
			<div>
				<Label htmlFor={`${prefix}-output`}>{t("features.form.output")}</Label>
				<select
					id={`${prefix}-output`}
					className="mt-1 w-full rounded-md border bg-transparent px-2 py-1.5 text-sm"
					value={selection.outputName}
					onChange={(event) => onChange({ outputName: event.target.value })}
				>
					<option value="">{t("features.form.none")}</option>
					{(stored?.outputs ?? [])
						.filter(
							(output) => !selection.nodeId || output.nodeId === selection.nodeId,
						)
						.map((output) => (
							<option key={output.name} value={output.name}>
								{output.name}
							</option>
						))}
				</select>
			</div>
			<div>
				<Label htmlFor={`${prefix}-snapshot`}>{t("features.form.snapshot")}</Label>
				<select
					id={`${prefix}-snapshot`}
					className="mt-1 w-full rounded-md border bg-transparent px-2 py-1.5 text-sm"
					value={selection.snapshotId}
					onChange={(event) => onChange({ snapshotId: event.target.value })}
				>
					<option value="">{t("features.form.none")}</option>
					{options.snapshots.map((snapshot) => (
						<option key={snapshot.snapshotId} value={snapshot.snapshotId}>
							{snapshot.code} {snapshot.interval} · {snapshot.snapshotId.slice(0, 8)}
						</option>
					))}
				</select>
			</div>
			<div>
				<Label htmlFor={`${prefix}-universe`}>{t("features.form.universe")}</Label>
				<select
					id={`${prefix}-universe`}
					className="mt-1 w-full rounded-md border bg-transparent px-2 py-1.5 text-sm"
					value={selection.universeId}
					onChange={(event) => onChange({ universeId: event.target.value })}
				>
					<option value="">{t("features.form.none")}</option>
					{options.universes.map((universe) => (
						<option key={universe.snapshotId} value={universe.snapshotId}>
							{universe.venue} {universe.interval} · {universe.snapshotId.slice(0, 8)}
						</option>
					))}
				</select>
			</div>
		</>
	);
}
