import type { LibraryComponent } from "@/features/components/component-library";

export type EvaluationSignalContract = {
	name: string;
	predictionKind: { kind: string };
	forecastTarget: { kind: string; target?: string; valueType?: string };
	valueScale: { kind: string };
	horizonBars: number;
};

export type EvaluationMetricDefinition = {
	label: string;
	meaning: string;
	formula: string;
	direction: string;
	range: string;
	caveat: string;
	reference: string;
};

const FORECAST_METRICS_REFERENCE =
	"https://github.com/tonywxx/adaq/blob/main/docs/reference/forecast-evaluation-metrics.md";

export const EVALUATION_METRIC_DEFINITIONS = {
	mae: {
		label: "MAE",
		meaning: "Mean absolute error in Forecast Target-native units.",
		formula: "mean(|prediction - realized|)",
		direction: "Lower is better.",
		range: "[0, +∞)",
		caveat: "Scale depends on the Target; it is not Strategy profitability.",
		reference: `${FORECAST_METRICS_REFERENCE}#mae`,
	},
	rmse: {
		label: "RMSE",
		meaning: "Root mean squared error in Forecast Target-native units.",
		formula: "sqrt(mean((prediction - realized)²))",
		direction: "Lower is better; larger errors receive more weight.",
		range: "[0, +∞)",
		caveat: "Scale depends on the Target; it is not Strategy profitability.",
		reference: `${FORECAST_METRICS_REFERENCE}#rmse`,
	},
	meanBias: {
		label: "Mean bias",
		meaning: "Average signed prediction error.",
		formula: "mean(prediction - realized)",
		direction: "Closer to zero means less average signed bias.",
		range: "(-∞, +∞)",
		caveat:
			"Positive and negative errors can cancel; there is no universal quality threshold.",
		reference: `${FORECAST_METRICS_REFERENCE}#mean-bias`,
	},
	pearsonCorrelation: {
		label: "Pearson correlation",
		meaning:
			"Linear association between aligned predictions and realized labels.",
		formula: "cov(prediction, realized) / (σprediction × σrealized)",
		direction: "Interpret sign and magnitude in research context.",
		range: "[-1, 1]",
		caveat:
			"Undefined for insufficient or constant evidence; no universal quality threshold applies.",
		reference: `${FORECAST_METRICS_REFERENCE}#pearson-correlation`,
	},
	brierScore: {
		label: "Brier Score",
		meaning: "Mean squared error between probability and binary realized label.",
		formula: "mean((probability - label)²)",
		direction: "Lower is better.",
		range: "[0, 1]",
		caveat:
			"Interpret against class balance and calibration context; there is no universal quality threshold.",
		reference: `${FORECAST_METRICS_REFERENCE}#brier-score`,
	},
	logLoss: {
		label: "Log Loss",
		meaning: "Mean binary cross-entropy of probability forecasts.",
		formula: "-mean(label×ln(p) + (1-label)×ln(1-p))",
		direction: "Lower is better; confident errors receive a larger penalty.",
		range: "Approximately [0, 34.539] with p clipped to [1e-15, 1-1e-15].",
		caveat:
			"Interpret against class balance; there is no universal quality threshold.",
		reference: `${FORECAST_METRICS_REFERENCE}#log-loss`,
	},
	rocAuc: {
		label: "ROC AUC",
		meaning:
			"Probability that a positive label ranks above a negative label, with ties worth one half.",
		formula:
			"(concordant positive-negative pairs + 0.5×ties) / all positive-negative pairs",
		direction: "Higher means stronger ranking separation.",
		range: "[0, 1]",
		caveat:
			"Undefined unless both realized classes are present; there is no universal quality threshold.",
		reference: `${FORECAST_METRICS_REFERENCE}#roc-auc`,
	},
	calibration: {
		label: "Calibration",
		meaning:
			"Mean prediction versus observed positive frequency in ten fixed equal-width buckets.",
		formula: "For each bucket: mean(probability) compared with mean(label)",
		direction: "Closer agreement indicates better calibration.",
		range: "Both bucket means are in [0, 1].",
		caveat:
			"Empty buckets remain explicit and small bucket counts are weak evidence.",
		reference: `${FORECAST_METRICS_REFERENCE}#calibration`,
	},
} satisfies Record<string, EvaluationMetricDefinition>;

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

export function isCompatibleEvaluationSignal(output: EvaluationSignalContract) {
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

export function evaluationMetricKind(
	output: EvaluationSignalContract,
): "expected-value" | "probability" | "custom-binary" {
	if (output.forecastTarget.kind === "custom") return "custom-binary";
	return output.forecastTarget.target === "future-close-up"
		? "probability"
		: "expected-value";
}
