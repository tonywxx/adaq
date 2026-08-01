import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import type { LibraryComponent } from "@/features/components/component-library";
import { useMarketSessionStore } from "@/lib/market-session";
import { useHistoryTab } from "@/lib/navigation-history";
import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import {
	datasetGenerationRequest,
	datasetStatusSummary,
	formatModelError,
} from "./models-workspace";

type Snapshot = {
	snapshotId: string;
	code: string;
	interval: string;
	barCount: number;
};
type Dataset = {
	datasetId: string;
	snapshotId: string;
	code: string;
	interval: string;
	predictionSource: string;
	rowCount: number;
	unavailableCount: number;
	statusCounts: Record<string, number>;
	modelArtifact?: { sha256: string; provenance: Record<string, string> };
	modelOutputs: Array<Record<string, unknown>>;
	modelParameters: Record<string, Record<string, unknown>>;
	sourceWarmupBars: number;
	modelWarmupBars: number;
	modelArchiveSha256: string;
	trust: string;
	componentLock: Array<{ alias: string; archiveSha256: string }>;
	featurePlanHash: string;
	featurePlanJson: string;
	seed: number;
	engineIdentity: Record<string, string>;
	producerSegments: Array<Record<string, unknown>>;
	continuousBarSegments: number;
	barGapRule: string;
	parquetSha256: string;
};
type Attempt = {
	attemptId: string;
	datasetId?: string;
	status: "pending" | "running" | "completed" | "failed" | "cancelled";
	diagnosticEvidence?: string;
	progressCompleted: number;
	progressTotal: number;
};

export function ModelsPage() {
	const userId = useMarketSessionStore((state) => state.userId);
	const [models, setModels] = useState<LibraryComponent[]>([]);
	const [components, setComponents] = useState<LibraryComponent[]>([]);
	const [snapshots, setSnapshots] = useState<Snapshot[]>([]);
	const [datasets, setDatasets] = useState<Dataset[]>([]);
	const [attempts, setAttempts] = useState<Attempt[]>([]);
	const [compatibleFactors, setCompatibleFactors] = useState<
		Record<string, string[]>
	>({});
	const [model, setModel] = useState("");
	const [snapshot, setSnapshot] = useState("");
	const [modelParameters, setModelParameters] = useState<Record<string, string>>(
		{},
	);
	const [busy, setBusy] = useState(false);
	const [evidence, setEvidence] = useState("");
	const [activeAttempt, setActiveAttempt] = useState("");
	const [tab, setTab] = useHistoryTab("models", "create");
	const refresh = useCallback(async () => {
		if (!userId) return;
		const [components, readable, generated, generationAttempts] =
			await Promise.all([
				invoke<LibraryComponent[]>("component_list", { request: { userId } }),
				invoke<Snapshot[]>("snapshot_list_readable", { request: { userId } }),
				invoke<Dataset[]>("signal_dataset_list", { userId }),
				invoke<Attempt[]>("dataset_generation_list", { userId }),
			]);
		setModels(components.filter((item) => item.kind === "model"));
		setComponents(components);
		setSnapshots(readable);
		setDatasets(generated);
		setAttempts(generationAttempts);
		setModel(
			(current) =>
				current ||
				components.find((item) => item.kind === "model")?.archiveSha256 ||
				"",
		);
		setSnapshot((current) => current || readable[0]?.snapshotId || "");
	}, [userId]);
	useEffect(() => {
		void refresh().catch((error) => setEvidence(String(error)));
	}, [refresh]);
	useEffect(() => {
		setCompatibleFactors({});
		if (!userId || !model) return;
		void invoke<Record<string, string[]>>("backtest_compatible_factors", {
			request: { userId, strategyArchiveSha256: model },
		})
			.then(setCompatibleFactors)
			.catch((error) => setEvidence(formatModelError(error)));
	}, [model, userId]);
	const trackAttempt = async (result: Attempt) => {
		if (!userId) return;
		setActiveAttempt(result.attemptId);
		let attempt = result;
		while (attempt.status === "pending" || attempt.status === "running") {
			await new Promise((resolve) => window.setTimeout(resolve, 250));
			const attempts = await invoke<Attempt[]>("dataset_generation_list", {
				userId,
			});
			attempt =
				attempts.find((item) => item.attemptId === result.attemptId) ?? attempt;
			setEvidence(
				`${attempt.status} · ${attempt.progressCompleted}/${attempt.progressTotal || "?"} rows`,
			);
		}
		setEvidence(
			attempt.diagnosticEvidence ||
				attempt.datasetId ||
				`Dataset generation ${attempt.status}.`,
		);
		await refresh();
	};
	const generate = async () => {
		if (!userId || !model || !snapshot || busy) return;
		setBusy(true);
		setEvidence("");
		await new Promise(requestAnimationFrame);
		try {
			const selected = models.find((item) => item.archiveSha256 === model);
			if (!selected) throw new Error("Select a Model Package.");
			const request = datasetGenerationRequest(
				userId,
				snapshot,
				selected,
				components,
				compatibleFactors,
				{
					...Object.fromEntries(
						selected.parameters.map((parameter) => [
							parameter.name,
							parameter.defaultValue,
						]),
					),
					...modelParameters,
				},
			);
			const result = await invoke<Attempt>("dataset_generation_start", {
				request,
			});
			await trackAttempt(result);
		} catch (error) {
			setEvidence(formatModelError(error));
		} finally {
			setBusy(false);
			setActiveAttempt("");
		}
	};
	const cancel = async () => {
		if (!userId || !activeAttempt) return;
		try {
			await invoke("dataset_generation_cancel", {
				attemptId: activeAttempt,
				userId,
			});
		} catch (error) {
			setEvidence(formatModelError(error));
		}
	};
	const retry = async (attemptId: string) => {
		if (!userId || busy) return;
		setBusy(true);
		setEvidence("");
		await new Promise(requestAnimationFrame);
		try {
			const result = await invoke<Attempt>("dataset_generation_retry", {
				attemptId,
				userId,
			});
			await trackAttempt(result);
		} catch (error) {
			setEvidence(formatModelError(error));
		} finally {
			setBusy(false);
		}
	};
	return (
		<main className="mx-auto w-full max-w-6xl p-4 sm:p-6" aria-busy={busy}>
			<header className="mb-6">
				<h1 className="text-2xl font-semibold">Models</h1>
				<p className="text-sm text-muted-foreground">
					Generate immutable forecast evidence from a verified Model Package.
				</p>
			</header>
			<Tabs value={tab} onValueChange={setTab}>
				<TabsList>
					<TabsTrigger value="create">Create Dataset</TabsTrigger>
					<TabsTrigger value="datasets">Signal Datasets</TabsTrigger>
				</TabsList>
				<TabsContent value="create">
					<Card>
						<CardHeader>
							<CardTitle>Native Dataset Generation</CardTitle>
						</CardHeader>
						<CardContent className="grid gap-4">
							<label className="grid gap-1 text-sm">
								Model Package
								<select
									className="rounded border bg-background p-2"
									value={model}
									onChange={(event) => setModel(event.target.value)}
								>
									{models.map((item) => (
										<option key={item.archiveSha256} value={item.archiveSha256}>
											{item.name} — {item.archiveSha256}
										</option>
									))}
								</select>
							</label>
							{models
								.find((item) => item.archiveSha256 === model)
								?.parameters.map((parameter) => (
									<label key={parameter.name} className="grid gap-1 text-sm">
										{parameter.name}
										<input
											className="rounded border bg-background p-2"
											value={modelParameters[parameter.name] ?? parameter.defaultValue}
											onChange={(event) =>
												setModelParameters((current) => ({
													...current,
													[parameter.name]: event.target.value,
												}))
											}
										/>
									</label>
								))}
							<label className="grid gap-1 text-sm">
								Market Data Snapshot
								<select
									className="rounded border bg-background p-2"
									value={snapshot}
									onChange={(event) => setSnapshot(event.target.value)}
								>
									{snapshots.map((item) => (
										<option key={item.snapshotId} value={item.snapshotId}>
											{item.code} {item.interval} — {item.snapshotId}
										</option>
									))}
								</select>
							</label>
							<div className="flex gap-2">
								<Button
									className="w-fit"
									loading={busy}
									disabled={!model || !snapshot || busy}
									onClick={() => void generate()}
								>
									Create Dataset
								</Button>
								{activeAttempt && (
									<Button variant="outline" onClick={() => void cancel()}>
										Cancel
									</Button>
								)}
							</div>
							{evidence && (
								<pre
									className="max-h-40 overflow-auto rounded bg-muted p-3 text-xs whitespace-pre-wrap"
									aria-live="polite"
								>
									{evidence}
								</pre>
							)}
							<div className="grid gap-2">
								<p className="text-sm font-medium">Generation Attempts</p>
								{attempts.map((attempt) => (
									<div
										key={attempt.attemptId}
										className="grid gap-2 rounded border p-2 text-xs"
									>
										<div className="flex items-center justify-between gap-3">
											<span className="break-all select-text">
												{attempt.status} · {attempt.progressCompleted}/
												{attempt.progressTotal || "?"} · {attempt.attemptId}
											</span>
											{(attempt.status === "failed" || attempt.status === "cancelled") && (
												<Button
													size="sm"
													variant="outline"
													disabled={busy}
													onClick={() => void retry(attempt.attemptId)}
												>
													Retry
												</Button>
											)}
										</div>
										{attempt.diagnosticEvidence && (
											<pre className="max-h-32 overflow-auto whitespace-pre-wrap select-text">
												{attempt.diagnosticEvidence}
											</pre>
										)}
									</div>
								))}
							</div>
						</CardContent>
					</Card>
				</TabsContent>
				<TabsContent value="datasets">
					<Card>
						<CardHeader>
							<CardTitle>Signal Datasets</CardTitle>
						</CardHeader>
						<CardContent className="grid gap-3">
							{datasets.length ? (
								datasets.map((item) => (
									<article
										key={item.datasetId}
										className="grid gap-2 rounded border p-3"
									>
										<p className="font-medium">
											{item.code} {item.interval} · {item.rowCount} rows
										</p>
										<dl className="grid gap-1 break-all text-xs text-muted-foreground">
											<div>
												<dt className="inline font-medium text-foreground">Coverage: </dt>
												<dd className="inline">
													{item.rowCount - item.unavailableCount} present,{" "}
													{item.unavailableCount} unavailable
												</dd>
											</div>
											<div>
												<dt className="inline font-medium text-foreground">Statuses: </dt>
												<dd className="inline select-text">
													{datasetStatusSummary(item.statusCounts)}
												</dd>
											</div>
											<div>
												<dt className="inline font-medium text-foreground">
													Model Artifact:{" "}
												</dt>
												<dd className="inline select-text">
													{item.modelArtifact?.sha256 ?? "Unavailable"}
												</dd>
											</div>
											<div>
												<dt className="inline font-medium text-foreground">
													Producer Segments:{" "}
												</dt>
												<dd className="inline">
													{item.producerSegments.length} · {item.continuousBarSegments}{" "}
													continuous · {item.barGapRule}
												</dd>
											</div>
											<div>
												<dt className="inline font-medium text-foreground">Snapshot: </dt>
												<dd className="inline select-text">{item.snapshotId}</dd>
											</div>
											<div>
												<dt className="inline font-medium text-foreground">
													Feature Plan:{" "}
												</dt>
												<dd className="inline select-text">{item.featurePlanHash}</dd>
											</div>
											<div>
												<dt className="inline font-medium text-foreground">
													Seed / Trust:{" "}
												</dt>
												<dd className="inline">
													{item.seed} · {item.trust}
												</dd>
											</div>
											<div>
												<dt className="inline font-medium text-foreground">
													Dataset / Parquet:{" "}
												</dt>
												<dd className="inline select-text">
													{item.datasetId} · {item.parquetSha256}
												</dd>
											</div>
											<div>
												<dt className="inline font-medium text-foreground">
													Component Lock:{" "}
												</dt>
												<dd className="inline select-text">
													{item.componentLock
														.map((entry) => `${entry.alias}: ${entry.archiveSha256}`)
														.join(", ")}
												</dd>
											</div>
											<details>
												<summary className="cursor-pointer font-medium text-foreground">
													Provenance
												</summary>
												<pre className="mt-2 max-h-48 overflow-auto whitespace-pre-wrap select-text">
													{JSON.stringify(
														{
															modelArtifact: item.modelArtifact?.provenance,
															modelOutputs: item.modelOutputs,
															modelParameters: item.modelParameters,
															sourceWarmupBars: item.sourceWarmupBars,
															modelWarmupBars: item.modelWarmupBars,
															producerSegments: item.producerSegments,
															predictionSource: item.predictionSource,
															engineIdentity: item.engineIdentity,
															featurePlan: JSON.parse(item.featurePlanJson),
														},
														null,
														2,
													)}
												</pre>
											</details>
										</dl>
									</article>
								))
							) : (
								<p className="text-sm text-muted-foreground">
									No Forecast Signal Datasets yet.
								</p>
							)}
						</CardContent>
					</Card>
				</TabsContent>
			</Tabs>
		</main>
	);
}
