import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "@/components/ui/card";
import { invoke } from "@tauri-apps/api/core";
import { Link } from "@tanstack/react-router";
import { CircleAlertIcon, LoaderCircleIcon } from "lucide-react";
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { useEffect, useState } from "react";
import type { FeatureDatasetView } from "@/features/features/features-types";

type ResearchStage = "features" | "factors" | "models";

type FrozenResearchEvidence = {
	operationId: string;
	contextRevision: number;
	contextHash: string;
	stage: ResearchStage;
	snapshotId: string;
	universeId?: string;
};

type FeatureDatasetBinding = {
	datasetId: string;
	requestHash: string;
	featurePlanHash: string;
	contentSha256: string;
	outputNames: string[];
};

type ResearchEvidenceProjection = {
	contextRevision: number;
	contextHash: string;
	market: string;
	venue: string;
	rangeStartMs: number;
	rangeEndMs: number;
	snapshotId: string;
	universeId?: string;
	featureDataset?: FeatureDatasetBinding;
};

export function ResearchContextPreflight({
	userId,
	stage,
}: {
	userId: string;
	stage: ResearchStage;
}) {
	const { t } = useTranslation();
	const [frozen, setFrozen] = useState<FrozenResearchEvidence>();
	const [freezing, setFreezing] = useState(false);
	const [freezeError, setFreezeError] = useState<string>();
	const [selectedFeatureDatasetId, setSelectedFeatureDatasetId] = useState("");
	const [factorContext, setFactorContext] =
		useState<ResearchEvidenceProjection>();
	const [handoffError, setHandoffError] = useState<string>();
	const [handingOff, setHandingOff] = useState(false);
	const isFactorStage = stage === "factors";
	const query = useQuery({
		queryKey: ["research-evidence-context", userId],
		queryFn: () =>
			invoke<ResearchEvidenceProjection | null>("research_context_get", {
				userId,
			}),
		staleTime: 30_000,
	});
	const featureDatasetsQuery = useQuery({
		queryKey: ["factor-feature-datasets", userId],
		queryFn: async () =>
			(await invoke<FeatureDatasetView[] | null>("feature_dataset_list", {
				request: { userId },
			})) ?? [],
		enabled: isFactorStage,
		staleTime: 30_000,
	});
	const context = factorContext ?? query.data;
	const contextReady = Boolean(
		context && (!isFactorStage || context.featureDataset),
	);

	useEffect(() => {
		if (!userId.trim() || !stage) return;
		setFactorContext(undefined);
		setSelectedFeatureDatasetId("");
		setHandoffError(undefined);
	}, [userId, stage]);

	useEffect(() => {
		const datasetId = query.data?.featureDataset?.datasetId;
		if (datasetId) setSelectedFeatureDatasetId(datasetId);
	}, [query.data?.featureDataset?.datasetId]);

	const establishFactorContext = async (featureDatasetId: string) => {
		setSelectedFeatureDatasetId(featureDatasetId);
		setHandoffError(undefined);
		if (!featureDatasetId) return;
		setHandingOff(true);
		try {
			const result = await invoke<ResearchEvidenceProjection>(
				"research_factor_context_establish",
				{ featureDatasetId },
			);
			setFactorContext(result);
		} catch (error) {
			setHandoffError(localizedFactorContextError(error, t));
		} finally {
			setHandingOff(false);
		}
	};
	const freeze = async () => {
		setFreezing(true);
		setFreezeError(undefined);
		try {
			const result = await invoke<FrozenResearchEvidence>(
				"research_context_freeze",
				{
					userId,
					operationId: `${stage}-${crypto.randomUUID()}`,
					stage,
				},
			);
			setFrozen(result);
		} catch (error) {
			setFreezeError(String(error));
		} finally {
			setFreezing(false);
		}
	};

	return (
		<Card>
			<CardHeader>
				<CardTitle className="flex items-center gap-2">
					{t("researchContext.title")}
					{query.isPending ? (
						<LoaderCircleIcon
							className="size-4 animate-spin"
							aria-label={t("researchContext.loading")}
						/>
					) : (
						<Badge variant={contextReady ? "default" : "outline"}>
							{contextReady
								? t("researchContext.ready")
								: t("researchContext.blocked")}
						</Badge>
					)}
				</CardTitle>
				<CardDescription>{t("researchContext.description")}</CardDescription>
			</CardHeader>
			<CardContent>
				{isFactorStage ? (
					<div className="mb-4 grid gap-2">
						<label htmlFor="factor-feature-dataset" className="text-sm font-medium">
							{t("researchContext.selectFeatureDataset")}
						</label>
						{featureDatasetsQuery.isPending ? (
							<span className="text-sm text-muted-foreground">
								{t("researchContext.featureDatasetLoading")}
							</span>
						) : featureDatasetsQuery.error ? (
							<p className="text-sm text-destructive" role="alert">
								{localizedFactorContextError(featureDatasetsQuery.error, t)}
							</p>
						) : (
							<select
								id="factor-feature-dataset"
								className="w-full rounded-md border bg-transparent px-2 py-1.5 text-sm"
								value={selectedFeatureDatasetId}
								disabled={handingOff}
								onChange={(event) => void establishFactorContext(event.target.value)}
							>
								<option value="">
									{t("researchContext.selectFeatureDatasetPlaceholder")}
								</option>
								{featureDatasetsQuery.data?.map((dataset) => (
									<option key={dataset.datasetId} value={dataset.datasetId}>
										{dataset.datasetId} · {dataset.manifest.request.featurePlanHash}
									</option>
								))}
							</select>
						)}
						{!featureDatasetsQuery.isPending &&
						!featureDatasetsQuery.error &&
						featureDatasetsQuery.data?.length === 0 ? (
							<div className="flex flex-wrap items-center gap-3 text-sm text-muted-foreground">
								<span>{t("researchContext.featureDatasetEmpty")}</span>
								<Link
									to="/features"
									className="font-medium text-primary underline-offset-4 hover:underline"
								>
									{t("researchContext.openFeatures")}
								</Link>
							</div>
						) : null}
						{handoffError ? (
							<p className="text-sm text-destructive" role="alert">
								{handoffError}
							</p>
						) : null}
						{handingOff ? (
							<span className="text-xs text-muted-foreground" role="status">
								{t("researchContext.handoffLoading")}
							</span>
						) : null}
					</div>
				) : null}
				{query.error ? (
					<p className="text-sm text-destructive" role="alert">
						{String(query.error)}
					</p>
				) : contextReady && context ? (
					<>
						<div className="grid gap-2 text-sm sm:grid-cols-3">
							<ContextValue
								label={t("researchContext.revision")}
								value={String(context.contextRevision)}
							/>
							<ContextValue
								label={t("researchContext.market")}
								value={`${context.market} · ${context.venue}`}
							/>
							<ContextValue
								label={t("researchContext.snapshot")}
								value={context.snapshotId}
							/>
							{Number.isFinite(context.rangeStartMs) &&
							Number.isFinite(context.rangeEndMs) ? (
								<ContextValue
									label={t("researchContext.observationRange")}
									value={`${context.rangeStartMs} → ${context.rangeEndMs}`}
								/>
							) : null}
							{context.featureDataset ? (
								<>
									<ContextValue
										label={t("researchContext.featureDataset")}
										value={context.featureDataset.datasetId}
									/>
									<ContextValue
										label={t("researchContext.featurePlan")}
										value={context.featureDataset.featurePlanHash}
									/>
									<ContextValue
										label={t("researchContext.universe")}
										value={context.universeId ?? "—"}
									/>
								</>
							) : null}
						</div>
						<div className="mt-3 flex flex-wrap items-center gap-3">
							<Button
								type="button"
								size="sm"
								loading={freezing}
								onClick={() => void freeze()}
							>
								{t("researchContext.freeze", { stage })}
							</Button>
							{frozen ? (
								<span className="text-xs text-muted-foreground">
									{t("researchContext.frozen", { revision: frozen.contextRevision })}
								</span>
							) : null}
							{freezeError ? (
								<span className="text-xs text-destructive" role="alert">
									{freezeError}
								</span>
							) : null}
						</div>
					</>
				) : (
					<div className="flex flex-wrap items-center gap-3 text-sm text-muted-foreground">
						<CircleAlertIcon className="size-4 text-amber-600" aria-hidden="true" />
						<span>
							{isFactorStage
								? t("researchContext.factorContextBlocked")
								: t("researchContext.empty")}
						</span>
						<Link
							to={isFactorStage ? "/features" : "/data-foundation"}
							className="font-medium text-primary underline-offset-4 hover:underline"
						>
							{isFactorStage
								? t("researchContext.openFeatures")
								: t("researchContext.openFoundation")}
						</Link>
					</div>
				)}
			</CardContent>
		</Card>
	);
}

function localizedFactorContextError(
	error: unknown,
	t: (key: string, options?: { defaultValue?: string }) => string,
) {
	const raw = String(error);
	const code = raw.replace(/^Error:\s*/, "").split(":")[0];
	return t(`researchContext.reasons.${code}`, { defaultValue: raw });
}

function ContextValue({ label, value }: { label: string; value: string }) {
	return (
		<div className="rounded-md border p-3">
			<div className="text-xs text-muted-foreground">{label}</div>
			<code className="break-all text-sm">{value}</code>
		</div>
	);
}
