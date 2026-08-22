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
import { useState } from "react";

type ResearchStage = "features" | "factors" | "models";

type FrozenResearchEvidence = {
	operationId: string;
	contextRevision: number;
	contextHash: string;
	stage: ResearchStage;
	snapshotId: string;
	universeId?: string;
};

type ResearchEvidenceProjection = {
	contextRevision: number;
	contextHash: string;
	market: string;
	venue: string;
	snapshotId: string;
	universeId?: string;
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
	const query = useQuery({
		queryKey: ["research-evidence-context", userId],
		queryFn: () =>
			invoke<ResearchEvidenceProjection | null>("research_context_get", {
				userId,
			}),
		staleTime: 30_000,
	});
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
						<Badge variant={query.data ? "default" : "outline"}>
							{query.data ? t("researchContext.ready") : t("researchContext.blocked")}
						</Badge>
					)}
				</CardTitle>
				<CardDescription>{t("researchContext.description")}</CardDescription>
			</CardHeader>
			<CardContent>
				{query.error ? (
					<p className="text-sm text-destructive" role="alert">
						{String(query.error)}
					</p>
				) : query.data ? (
					<>
						<div className="grid gap-2 text-sm sm:grid-cols-3">
							<ContextValue
								label={t("researchContext.revision")}
								value={String(query.data.contextRevision)}
							/>
							<ContextValue
								label={t("researchContext.market")}
								value={`${query.data.market} · ${query.data.venue}`}
							/>
							<ContextValue
								label={t("researchContext.snapshot")}
								value={query.data.snapshotId}
							/>
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
						<span>{t("researchContext.empty")}</span>
						<Link
							to="/data-foundation"
							className="font-medium text-primary underline-offset-4 hover:underline"
						>
							{t("researchContext.openFoundation")}
						</Link>
					</div>
				)}
			</CardContent>
		</Card>
	);
}

function ContextValue({ label, value }: { label: string; value: string }) {
	return (
		<div className="rounded-md border p-3">
			<div className="text-xs text-muted-foreground">{label}</div>
			<code className="break-all text-sm">{value}</code>
		</div>
	);
}
