import { IdentifierDisplay } from "@/components/identifier-display";
import { Badge } from "@/components/ui/badge";
import { invoke } from "@tauri-apps/api/core";
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { localizedFactorContextError } from "./research-context-preflight";

type FrozenResearchEvidence = {
	operationId: string;
	contextRevision: number;
	contextHash: string;
	stage: "features" | "factors" | "models";
	snapshotId: string;
	universeId?: string;
	featureDataset?: {
		datasetId: string;
		featurePlanHash: string;
	};
};

export function ResearchContextEvidence({
	userId,
	attemptId,
}: {
	userId: string;
	attemptId: string;
}) {
	const { t } = useTranslation();
	const query = useQuery({
		queryKey: ["research-context-attempt", userId, attemptId],
		queryFn: () =>
			invoke<FrozenResearchEvidence | null>("research_context_for_attempt", {
				userId,
				attemptId,
			}),
		staleTime: 30_000,
	});

	if (query.isLoading) return null;
	if (query.error) {
		return (
			<span className="text-xs text-destructive" role="alert">
				{localizedFactorContextError(query.error, t)}
			</span>
		);
	}
	if (!query.data) {
		return (
			<Badge variant="outline">{t("researchContext.evidenceMissing")}</Badge>
		);
	}
	return (
		<div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
			<Badge variant="secondary">{t("researchContext.evidenceBound")}</Badge>
			<span>
				{t("researchContext.revision")} {query.data.contextRevision} ·{" "}
				{t(`researchContext.stages.${query.data.stage}`)}
			</span>
			<span className="break-all">
				{t("researchContext.snapshot")}:{" "}
				<IdentifierDisplay id={query.data.snapshotId} label={t("identifiers.snapshot")} />
			</span>
			{query.data.universeId && (
				<span className="break-all">
					{t("researchContext.universe")}:{" "}
					<IdentifierDisplay id={query.data.universeId} label={t("identifiers.universe")} />
				</span>
			)}
			{query.data.featureDataset && (
				<span className="break-all">
					{t("researchContext.featureDataset")}:{" "}
					<IdentifierDisplay
						id={query.data.featureDataset.datasetId}
						label={t("identifiers.featureDataset")}
					/> · {t("researchContext.featurePlan")}:{" "}
					<IdentifierDisplay
						id={query.data.featureDataset.featurePlanHash}
						label={t("identifiers.featurePlan")}
					/>
				</span>
			)}
			<IdentifierDisplay
				id={query.data.operationId}
				label={t("identifiers.operation")}
			/>
		</div>
	);
}
