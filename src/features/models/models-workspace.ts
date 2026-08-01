import type { LibraryComponent } from "@/features/components/component-library";

export function datasetGenerationRequest(
	userId: string,
	snapshotId: string,
	model: LibraryComponent,
	components: readonly LibraryComponent[],
	compatibleFactors: Readonly<Record<string, readonly string[]>>,
	modelParameters: Record<string, string> = Object.fromEntries(
		model.parameters.map((parameter) => [parameter.name, parameter.defaultValue]),
	),
) {
	return {
		userId,
		snapshotId,
		modelArchiveSha256: model.archiveSha256,
		modelParameters,
		factorInstances: model.dependencies.map((dependency) => {
			const factor = components.find(
				(item) =>
					item.kind === "factor" &&
					compatibleFactors[dependency.alias]?.includes(item.archiveSha256),
			);
			if (!factor)
				throw new Error(`Required Factor ${dependency.alias} is not available.`);
			return { alias: dependency.alias, archiveSha256: factor.archiveSha256 };
		}),
		seed: 0,
	};
}

export function formatModelError(error: unknown) {
	return error instanceof Error ? error.message : String(error);
}

export function datasetStatusSummary(statusCounts: Record<string, number>) {
	return Object.entries(statusCounts)
		.map(([status, count]) => `${status}: ${count}`)
		.join(", ");
}

export const signalRowPageRequest = (
	datasetId: string,
	userId: string,
	page: number,
) => ({ datasetId, userId, page });

export function signalRowSummary(row: {
	predictionTimeMs: number;
	availableAtMs: number;
	status: string;
	values?: number[];
	unavailableReason?: string;
}) {
	return `${row.predictionTimeMs} · available ${row.availableAtMs} · ${row.status} · ${row.values?.join(", ") ?? row.unavailableReason ?? "unavailable"}`;
}

export function evaluationReportSummary(report: {
	reportId: string;
	evidenceState: { summary: string };
	metrics: { alignedCount: number; coverage: number; missingness: number };
}) {
	return `${report.metrics.alignedCount} aligned · ${(report.metrics.coverage * 100).toFixed(2)}% coverage · ${(report.metrics.missingness * 100).toFixed(2)}% missing · ${report.evidenceState.summary} · ${report.reportId}`;
}

export function evaluationExportFilename(
	reportId: string,
	format: "json" | "markdown",
) {
	return `forecast-evaluation-report-${reportId}.${format === "json" ? "json" : "md"}`;
}

export function isCompatibleEvaluationSignal(output: {
	predictionKind: { kind: string };
	forecastTarget: { kind: string; target?: string; valueType?: string };
	valueScale: { kind: string };
	horizonBars: number;
}) {
	if (!Number.isInteger(output.horizonBars) || output.horizonBars < 1)
		return false;
	if (
		output.predictionKind.kind === "expected-value" &&
		output.forecastTarget.target === "future-close-return"
	)
		return output.valueScale.kind === "native";
	return (
		output.predictionKind.kind === "probability" &&
		output.valueScale.kind === "probability" &&
		(output.forecastTarget.target === "future-close-up" ||
			(output.forecastTarget.kind === "custom" &&
				output.forecastTarget.valueType === "binary"))
	);
}
