import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { formatNumber } from "@/lib/i18n";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { FeaturesAdapter } from "./features-adapter";
import {
	DATASET_PAGE_SIZE,
	buildDatasetFilter,
	camelReason,
	datasetPageOffset,
	formatFeatureError,
	parseTimestampInput,
	readSessionCache,
	writeSessionCache,
} from "./features-data";
import {
	FeaturesEmpty,
	FeaturesError,
	FeaturesLoading,
	formatUtc,
} from "./features-shared";
import type {
	FeatureDatasetView,
	FeatureDatasetRow,
	FeatureOutputSummary,
} from "./features-types";

export function DatasetsView({
	userId,
	adapter,
}: {
	userId: string;
	adapter: FeaturesAdapter;
}) {
	const { t } = useTranslation();
	const [datasets, setDatasets] = useState<FeatureDatasetView[]>();
	const [loadError, setLoadError] = useState<string>();
	const [selectedId, setSelectedId] = useState<string>();
	const [deleteError, setDeleteError] = useState<string>();

	const load = useCallback(async () => {
		try {
			const items = await adapter.listDatasets(userId);
			writeSessionCache(userId, "datasets", items);
			setDatasets(items);
			setLoadError(undefined);
		} catch (error) {
			setLoadError(formatFeatureError(error));
		}
	}, [adapter, userId]);

	useEffect(() => {
		setDatasets(
			readSessionCache(userId, "datasets") as FeatureDatasetView[] | undefined,
		);
		load();
	}, [userId, load]);

	const selected = datasets?.find((item) => item.datasetId === selectedId);

	const remove = async (datasetId: string) => {
		setDeleteError(undefined);
		try {
			await adapter.deleteDataset(userId, datasetId);
			if (selectedId === datasetId) setSelectedId(undefined);
			await load();
		} catch (error) {
			// Reference locks surface as typed native errors.
			setDeleteError(formatFeatureError(error));
		}
	};

	return (
		<div className="space-y-6">
			<Card>
				<CardHeader>
					<CardTitle>{t("features.datasets.heading")}</CardTitle>
				</CardHeader>
				<CardContent>
					<div aria-live="polite">
						{deleteError && (
							<p role="alert" className="mb-3 text-sm text-destructive">
								{deleteError.includes("feature-dataset-referenced")
									? t("features.datasets.locked")
									: deleteError}
							</p>
						)}
					</div>
					{loadError && datasets === undefined ? (
						<FeaturesError
							message={loadError}
							onRetry={load}
							retryLabel={t("features.retryLoad")}
						/>
					) : datasets === undefined ? (
						<FeaturesLoading label={t("features.loading")} />
					) : datasets.length === 0 ? (
						<FeaturesEmpty message={t("features.datasets.empty")} />
					) : (
						<div className="max-w-full overflow-x-auto">
							<table className="w-full text-sm">
								<thead>
									<tr className="border-b text-left text-muted-foreground">
										<th className="py-2 pr-4 font-medium">{t("features.datasets.id")}</th>
										<th className="py-2 pr-4 font-medium">
											{t("features.datasets.rows")}
										</th>
										<th className="py-2 pr-4 font-medium">
											{t("features.datasets.size")}
										</th>
										<th className="py-2 pr-4 font-medium">
											{t("features.datasets.created")}
										</th>
										<th className="py-2 font-medium" />
									</tr>
								</thead>
								<tbody>
									{datasets.map((dataset) => (
										<tr key={dataset.datasetId} className="border-b">
											<td className="py-2 pr-4 font-mono text-xs">
												{dataset.datasetId.slice(0, 16)}…
											</td>
											<td className="py-2 pr-4">
												{formatNumber(dataset.manifest.rowCount)}
											</td>
											<td className="py-2 pr-4">
												{formatNumber(dataset.contentByteSize)} B
											</td>
											<td className="py-2 pr-4 whitespace-nowrap">
												{formatUtc(dataset.createdAtMs)} UTC
											</td>
											<td className="py-2 text-right">
												<div className="flex justify-end gap-1">
													<Button
														type="button"
														variant="outline"
														size="sm"
														onClick={() => setSelectedId(dataset.datasetId)}
													>
														{t("features.datasets.open")}
													</Button>
													<Button
														type="button"
														variant="ghost"
														size="sm"
														onClick={() => remove(dataset.datasetId)}
													>
														{t("features.datasets.delete")}
													</Button>
												</div>
											</td>
										</tr>
									))}
								</tbody>
							</table>
						</div>
					)}
				</CardContent>
			</Card>

			{selected && (
				<DatasetInspector
					userId={userId}
					adapter={adapter}
					dataset={selected}
					onClose={() => setSelectedId(undefined)}
				/>
			)}
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
	adapter: FeaturesAdapter;
	dataset: FeatureDatasetView;
	onClose: () => void;
}) {
	const { t } = useTranslation();
	const [summary, setSummary] = useState<FeatureOutputSummary[]>();
	const [summaryError, setSummaryError] = useState<string>();
	const [rows, setRows] = useState<FeatureDatasetRow[]>();
	const [rowsError, setRowsError] = useState<string>();
	const [page, setPage] = useState(1);
	const [hasNext, setHasNext] = useState(false);
	const [filterForm, setFilterForm] = useState({
		instrumentId: "",
		startTime: "",
		endTime: "",
		outputName: "",
		state: "",
	});
	const [appliedFilter, setAppliedFilter] = useState(filterForm);
	const [rowsLoading, setRowsLoading] = useState(false);

	useEffect(() => {
		setSummary(undefined);
		setSummaryError(undefined);
		adapter
			.datasetSummary(userId, dataset.datasetId)
			.then(setSummary)
			.catch((error) => setSummaryError(formatFeatureError(error)));
	}, [adapter, userId, dataset.datasetId]);

	const loadRows = useCallback(
		async (filterValues: typeof appliedFilter, nextPage: number) => {
			setRowsLoading(true);
			setRowsError(undefined);
			try {
				const result = await adapter.datasetRows(
					userId,
					dataset.datasetId,
					buildDatasetFilter({
						instrumentId: filterValues.instrumentId,
						startTimeMs: parseTimestampInput(filterValues.startTime),
						endTimeMs: parseTimestampInput(filterValues.endTime),
						outputName: filterValues.outputName,
						state: filterValues.state,
					}),
					datasetPageOffset(nextPage),
				);
				setRows(result.rows);
				setHasNext(result.nextOffset !== null && result.nextOffset !== undefined);
			} catch (error) {
				setRowsError(formatFeatureError(error));
			} finally {
				setRowsLoading(false);
			}
		},
		[adapter, userId, dataset.datasetId],
	);

	useEffect(() => {
		loadRows(appliedFilter, page);
	}, [loadRows, appliedFilter, page]);

	const manifest = dataset.manifest;
	const outputNames = manifest.outputs.map((output) => output.outputName);
	const visibleOutputs = appliedFilter.outputName
		? [appliedFilter.outputName]
		: outputNames;

	return (
		<Card>
			<CardHeader className="flex-row items-center justify-between space-y-0">
				<CardTitle className="font-mono text-base">{dataset.datasetId}</CardTitle>
				<Button
					type="button"
					variant="ghost"
					size="sm"
					aria-label={t("features.datasets.close")}
					onClick={onClose}
				>
					✕
				</Button>
			</CardHeader>
			<CardContent className="space-y-6">
				<section aria-label={t("features.datasets.manifest.heading")}>
					<h3 className="mb-2 text-sm font-semibold">
						{t("features.datasets.manifest.heading")}
					</h3>
					<dl className="grid gap-x-8 gap-y-1 text-xs sm:grid-cols-2">
						<div>
							<dt className="inline font-medium">
								{t("features.datasets.manifest.requestHash")}:{" "}
							</dt>
							<dd className="inline break-all font-mono">{manifest.requestHash}</dd>
						</div>
						<div>
							<dt className="inline font-medium">
								{t("features.datasets.manifest.snapshot")}:{" "}
							</dt>
							<dd className="inline font-mono">{manifest.request.snapshotId}</dd>
						</div>
						<div>
							<dt className="inline font-medium">
								{t("features.datasets.manifest.universe")}:{" "}
							</dt>
							<dd className="inline font-mono">
								{manifest.request.pointInTimeUniverseId}
							</dd>
						</div>
						<div>
							<dt className="inline font-medium">
								{t("features.datasets.manifest.range")}:{" "}
							</dt>
							<dd className="inline">
								{formatUtc(manifest.request.observationRange.startTimeMs)} →{" "}
								{formatUtc(manifest.request.observationRange.endTimeMs)} UTC
							</dd>
						</div>
						<div>
							<dt className="inline font-medium">
								{t("features.datasets.manifest.rowCount")}:{" "}
							</dt>
							<dd className="inline">{formatNumber(manifest.rowCount)}</dd>
						</div>
						<div>
							<dt className="inline font-medium">
								{t("features.datasets.manifest.reasonVersion")}:{" "}
							</dt>
							<dd className="inline font-mono">{manifest.reasonVersion}</dd>
						</div>
						<div className="sm:col-span-2">
							<dt className="inline font-medium">
								{t("features.datasets.manifest.contentSha256")}:{" "}
							</dt>
							<dd className="inline break-all font-mono">{manifest.contentSha256}</dd>
						</div>
					</dl>
				</section>

				<section aria-label={t("features.datasets.summary.heading")}>
					<h3 className="mb-2 text-sm font-semibold">
						{t("features.datasets.summary.heading")}
					</h3>
					<p className="mb-2 text-xs text-muted-foreground">
						{t("features.datasets.summary.disclaimer")}
					</p>
					{summaryError ? (
						<FeaturesError message={summaryError} />
					) : summary === undefined ? (
						<FeaturesLoading label={t("features.loading")} />
					) : (
						<div className="max-w-full overflow-x-auto">
							<table className="w-full text-xs">
								<thead>
									<tr className="border-b text-left text-muted-foreground">
										<th className="py-1.5 pr-3 font-medium">
											{t("features.datasets.summary.output")}
										</th>
										<th className="py-1.5 pr-3 font-medium">
											{t("features.datasets.summary.coverage")}
										</th>
										<th className="py-1.5 pr-3 font-medium">
											{t("features.datasets.summary.available")}
										</th>
										<th className="py-1.5 pr-3 font-medium">
											{t("features.datasets.summary.min")}
										</th>
										<th className="py-1.5 pr-3 font-medium">
											{t("features.datasets.summary.max")}
										</th>
										<th className="py-1.5 pr-3 font-medium">
											{t("features.datasets.summary.mean")}
										</th>
										<th className="py-1.5 pr-3 font-medium">
											{t("features.datasets.summary.std")}
										</th>
										<th className="py-1.5 font-medium">
											{t("features.datasets.summary.reasons")}
										</th>
									</tr>
								</thead>
								<tbody>
									{summary.map((output) => (
										<tr key={output.outputName} className="border-b">
											<td className="py-1.5 pr-3 font-mono">{output.outputName}</td>
											<td className="py-1.5 pr-3">
												{formatNumber(output.coverage * 100, {
													maximumFractionDigits: 1,
												})}
												%
											</td>
											<td className="py-1.5 pr-3">
												{formatNumber(output.availableCount)} /{" "}
												{formatNumber(output.rowCount)}
											</td>
											<td className="py-1.5 pr-3 font-mono">{output.minimum ?? "—"}</td>
											<td className="py-1.5 pr-3 font-mono">{output.maximum ?? "—"}</td>
											<td className="py-1.5 pr-3 font-mono">
												{output.mean !== null && output.mean !== undefined
													? output.mean.toFixed(6)
													: "—"}
											</td>
											<td className="py-1.5 pr-3 font-mono">
												{output.populationStandardDeviation !== null &&
												output.populationStandardDeviation !== undefined
													? output.populationStandardDeviation.toFixed(6)
													: "—"}
											</td>
											<td className="py-1.5 font-mono">
												{Object.entries(output.unavailableCounts)
													.map(([reason, count]) => `${reason}:${count}`)
													.join(" ") || "—"}
											</td>
										</tr>
									))}
								</tbody>
							</table>
						</div>
					)}
				</section>

				<section aria-label={t("features.datasets.table.heading")}>
					<h3 className="mb-2 text-sm font-semibold">
						{t("features.datasets.table.heading")}
					</h3>
					<div className="mb-3 grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
						<div>
							<Label htmlFor="rows-filter-instrument">
								{t("features.datasets.table.filterInstrument")}
							</Label>
							<Input
								id="rows-filter-instrument"
								className="mt-1"
								value={filterForm.instrumentId}
								onChange={(event) =>
									setFilterForm({
										...filterForm,
										instrumentId: event.target.value,
									})
								}
							/>
						</div>
						<div>
							<Label htmlFor="rows-filter-start">
								{t("features.datasets.table.filterStart")}
							</Label>
							<Input
								id="rows-filter-start"
								className="mt-1"
								type="datetime-local"
								value={filterForm.startTime}
								onChange={(event) =>
									setFilterForm({ ...filterForm, startTime: event.target.value })
								}
							/>
						</div>
						<div>
							<Label htmlFor="rows-filter-end">
								{t("features.datasets.table.filterEnd")}
							</Label>
							<Input
								id="rows-filter-end"
								className="mt-1"
								type="datetime-local"
								value={filterForm.endTime}
								onChange={(event) =>
									setFilterForm({ ...filterForm, endTime: event.target.value })
								}
							/>
						</div>
						<div>
							<Label htmlFor="rows-filter-output">
								{t("features.datasets.table.filterOutput")}
							</Label>
							<select
								id="rows-filter-output"
								className="mt-1 w-full rounded-md border bg-transparent px-2 py-1.5 text-sm"
								value={filterForm.outputName}
								onChange={(event) =>
									setFilterForm({
										...filterForm,
										outputName: event.target.value,
									})
								}
							>
								<option value="">{t("features.datasets.table.stateAll")}</option>
								{outputNames.map((name) => (
									<option key={name} value={name}>
										{name}
									</option>
								))}
							</select>
						</div>
						<div>
							<Label htmlFor="rows-filter-state">
								{t("features.datasets.table.filterState")}
							</Label>
							<select
								id="rows-filter-state"
								className="mt-1 w-full rounded-md border bg-transparent px-2 py-1.5 text-sm"
								value={filterForm.state}
								onChange={(event) =>
									setFilterForm({ ...filterForm, state: event.target.value })
								}
							>
								<option value="">{t("features.datasets.table.stateAll")}</option>
								<option value="available">
									{t("features.datasets.table.stateAvailable")}
								</option>
								<option value="unavailable">
									{t("features.datasets.table.stateUnavailable")}
								</option>
							</select>
						</div>
						<div className="flex items-end">
							<Button
								type="button"
								variant="outline"
								size="sm"
								onClick={() => {
									setAppliedFilter(filterForm);
									setPage(1);
								}}
							>
								{t("features.datasets.table.apply")}
							</Button>
						</div>
					</div>

					{rowsError ? (
						<FeaturesError message={rowsError} />
					) : rowsLoading || rows === undefined ? (
						<FeaturesLoading label={t("features.loading")} />
					) : rows.length === 0 ? (
						<FeaturesEmpty message={t("features.datasets.table.empty")} />
					) : (
						<>
							<div className="max-w-full overflow-x-auto">
								<table className="w-full text-xs">
									<thead>
										<tr className="border-b text-left text-muted-foreground">
											<th className="py-1.5 pr-3 font-medium">
												{t("features.datasets.table.instrument")}
											</th>
											<th className="py-1.5 pr-3 font-medium">
												{t("features.datasets.table.time")}
											</th>
											{visibleOutputs.map((name) => (
												<th key={name} className="py-1.5 pr-3 font-medium font-mono">
													{name}
												</th>
											))}
										</tr>
									</thead>
									<tbody>
										{rows.map((row, index) => (
											<tr
												key={`${row.instrumentId}-${row.observationTimeMs}-${index}`}
												className="border-b"
											>
												<td className="py-1.5 pr-3">{row.instrumentId}</td>
												<td className="py-1.5 pr-3 whitespace-nowrap">
													{formatUtc(row.observationTimeMs)}
												</td>
												{visibleOutputs.map((name) => {
													const cell = row.values[name];
													return (
														<td key={name} className="py-1.5 pr-3 font-mono">
															{!cell
																? "—"
																: cell.state === "available"
																	? cell.value
																	: t(`features.unavailability.${camelReason(cell.reason)}`, {
																			defaultValue: cell.reason,
																		})}
														</td>
													);
												})}
											</tr>
										))}
									</tbody>
								</table>
							</div>
							<div className="mt-3 flex items-center gap-2 text-sm">
								<Button
									type="button"
									variant="outline"
									size="sm"
									disabled={page <= 1 || rowsLoading}
									onClick={() => setPage(page - 1)}
								>
									{t("features.datasets.table.previous")}
								</Button>
								<span aria-live="polite">
									{t("features.datasets.table.page", {
										page: formatNumber(page),
										size: formatNumber(DATASET_PAGE_SIZE),
									})}
								</span>
								<Button
									type="button"
									variant="outline"
									size="sm"
									disabled={!hasNext || rowsLoading}
									onClick={() => setPage(page + 1)}
								>
									{t("features.datasets.table.next")}
								</Button>
							</div>
						</>
					)}
				</section>
			</CardContent>
		</Card>
	);
}
