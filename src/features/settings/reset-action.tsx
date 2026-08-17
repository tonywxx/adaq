import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useTranslation } from "react-i18next";
import { type ReactNode, useId, useRef, useState } from "react";
import { toast } from "sonner";
import type { ResetKind, SettingsActions } from "./settings-actions";
import type { LocalDataSummary } from "./settings-types";
export type { LocalDataSummary } from "./settings-types";

export function ResetAction({
	kind,
	titleKey,
	descriptionKey,
	summary,
	userId,
	actions,
}: {
	kind: ResetKind;
	titleKey: string;
	descriptionKey: string;
	summary: LocalDataSummary | null;
	userId?: string;
	actions: SettingsActions;
}) {
	const { t } = useTranslation();
	const dialog = useRef<HTMLDialogElement>(null);
	const confirmationId = useId();
	const [confirmation, setConfirmation] = useState("");
	const [running, setRunning] = useState(false);
	const deviceWide = kind === "factorResearch";
	const requiredConfirmation = deviceWide ? "RESET FACTOR RESEARCH" : "RESET";
	const blocked = deviceWide
		? false
		: kind === "components"
			? (summary?.componentBlockingRunCount ?? 0) > 0
			: kind === "marketData"
				? (summary?.marketDataBlockingRecordCount ?? 0) > 0
				: false;

	async function reset() {
		if (
			(!deviceWide && !userId) ||
			blocked ||
			((kind === "all" || deviceWide) && confirmation !== requiredConfirmation)
		)
			return;
		setRunning(true);
		try {
			if (deviceWide) {
				await actions.resetFactorResearch();
			} else {
				if (!userId) return;
				await actions.resetLocalData(userId, kind);
			}
			toast.success(t("settings.dataStorage.completed", { title }));
			window.setTimeout(() => window.location.reload(), 500);
		} catch (reason) {
			setRunning(false);
			toast.error(String(reason));
		}
	}
	const title = t(titleKey);
	const description = t(descriptionKey);

	return (
		<>
			<div className="flex items-center justify-between gap-5 rounded-lg border p-4">
				<div>
					<p className="font-medium">{title}</p>
					<p className="text-sm text-muted-foreground">{description}</p>
				</div>
				<Button
					variant="destructive"
					loading={running}
					disabled={!summary || (!deviceWide && !userId)}
					onClick={() => dialog.current?.showModal()}
				>
					{t("settings.dataStorage.resetButton")}
				</Button>
			</div>
			<dialog
				ref={dialog}
				onCancel={(event) => {
					if (running) event.preventDefault();
				}}
				className="m-auto w-[min(32rem,calc(100%-2rem))] rounded-xl border bg-background p-0 text-foreground shadow-2xl backdrop:bg-black/45"
			>
				<div className="grid gap-4 p-6">
					<div>
						<h3 className="text-lg font-semibold">
							{t("settings.dataStorage.confirmTitle", { title })}
						</h3>
						<p className="mt-1 text-sm text-muted-foreground">
							{t(
								deviceWide
									? "settings.dataStorage.factorResearchConfirmDescription"
									: "settings.dataStorage.confirmDescription",
							)}
						</p>
					</div>
					<ResetDetails kind={kind} summary={summary} />
					{blocked ? (
						<p className="rounded-lg bg-destructive/10 p-3 text-sm text-destructive">
							{t("settings.dataStorage.blocked")}
						</p>
					) : null}
					{kind === "all" || deviceWide ? (
						<div className="grid gap-2">
							<Label htmlFor={confirmationId}>
								{t(
									deviceWide
										? "settings.dataStorage.typeFactorResearchReset"
										: "settings.dataStorage.typeReset",
								)}
							</Label>
							<Input
								id={confirmationId}
								value={confirmation}
								onChange={(event) => setConfirmation(event.target.value)}
								autoComplete="off"
							/>
						</div>
					) : null}
					<div className="flex justify-end gap-2">
						<Button
							variant="outline"
							disabled={running}
							onClick={() => dialog.current?.close()}
						>
							{t("settings.dataStorage.cancel")}
						</Button>
						<Button
							variant="destructive"
							loading={running}
							disabled={
								blocked ||
								((kind === "all" || deviceWide) &&
									confirmation !== requiredConfirmation)
							}
							onClick={() => void reset()}
						>
							{title}
						</Button>
					</div>
				</div>
			</dialog>
		</>
	);
}

function ResetDetails({
	kind,
	summary,
}: {
	kind: ResetKind;
	summary: LocalDataSummary | null;
}) {
	const { t } = useTranslation();
	if (!summary) return null;
	const rows: ReactNode[] = [];
	if (kind === "watchlist" || kind === "all")
		rows.push(
			<li key="watchlist">
				{t("settings.dataStorage.watchlistItems", {
					count: summary.watchlistCount,
				})}
			</li>,
		);
	if (kind === "components" || kind === "all")
		rows.push(
			<li key="components">
				{t("settings.dataStorage.componentPackagesCount", {
					count: summary.componentCount,
				})}
			</li>,
		);
	if (kind === "marketData" || kind === "all")
		rows.push(
			<li key="snapshots">
				{t("settings.dataStorage.marketDataSnapshotsCount", {
					count: summary.snapshotCount,
				})}
			</li>,
		);
	if (kind === "all")
		rows.push(
			<li key="runs">
				{t("settings.dataStorage.backtestRuns", { count: summary.runCount })}
			</li>,
			<li key="protocols">
				{t("settings.dataStorage.validationProtocols", {
					count: summary.protocolCount,
				})}
			</li>,
			<li key="reports">
				{t("settings.dataStorage.validationReports", {
					count: summary.reportCount,
				})}
			</li>,
			<li key="attempts">
				{t("settings.dataStorage.generationAttempts", {
					count: summary.generationAttemptCount,
				})}
			</li>,
			<li key="artifacts">
				{t("settings.dataStorage.modelArtifacts", {
					count: summary.modelArtifactCount,
				})}
			</li>,
			<li key="datasets">
				{t("settings.dataStorage.signalDatasets", {
					count: summary.signalDatasetCount,
				})}
			</li>,
		);
	if (kind === "factorResearch")
		rows.push(
			<li key="factorResearch">{t("settings.dataStorage.factorResearchData")}</li>,
		);
	return (
		<div className="rounded-lg border bg-muted/30 p-4 text-sm">
			<p className="mb-2 font-medium">{t("settings.dataStorage.dataToReset")}</p>
			<ul className="list-inside list-disc space-y-1 text-muted-foreground">
				{rows}
			</ul>
			<p className="mt-3">
				{t(
					kind === "factorResearch"
						? "settings.dataStorage.factorResearchPreserved"
						: "settings.dataStorage.preserved",
				)}
			</p>
		</div>
	);
}
