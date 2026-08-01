import type { LibraryComponent } from "@/features/components/component-library";
import {
	datasetGenerationRequest,
	datasetStatusSummary,
	EVALUATION_METRIC_DEFINITIONS,
	formatModelError,
	signalRowPageRequest,
	signalRowSummary,
	evaluationReportSummary,
	evaluationExportFilename,
	evaluationMetricKind,
	isCompatibleEvaluationSignal,
} from "./models-workspace";

const component = (value: Partial<LibraryComponent>): LibraryComponent => ({
	componentId: "model",
	version: "1.0.0",
	manifestSchemaVersion: "1.0.0",
	sdkVersion: "0.1.0",
	abiVersion: "1.0.0",
	name: "Model",
	kind: "model",
	archiveSha256: "model-hash",
	wasmSha256: "wasm-hash",
	parameters: [],
	featureSlots: [],
	outputNames: [],
	dependencies: [],
	warmupBars: 0,
	compatible: true,
	lockedByRunIds: [],
	...value,
});

test("maps paged external rows to exact copyable evidence", () => {
	expect(signalRowPageRequest("dataset", "user", 2)).toEqual({
		datasetId: "dataset",
		userId: "user",
		page: 2,
	});
	expect(
		signalRowSummary({
			predictionTimeMs: 10,
			availableAtMs: 20,
			status: "present",
			values: [0.25],
		}),
	).toBe("10 · available 20 · present · 0.25");
	expect(
		signalRowSummary({
			predictionTimeMs: 10,
			availableAtMs: 10,
			status: "unavailable",
			unavailableReason: "warmup",
		}),
	).toContain("unavailable · warmup");
});

test("freezes exact ordered Factor bindings for Dataset generation", () => {
	const model = component({
		dependencies: [{ componentId: "factor", version: "^1.0.0", alias: "alpha" }],
	});
	const factor = component({
		componentId: "factor",
		name: "Factor",
		kind: "factor",
		archiveSha256: "factor-hash",
	});
	expect(
		datasetGenerationRequest("user", "snapshot", model, [model, factor], {
			alpha: ["factor-hash"],
		}),
	).toEqual({
		userId: "user",
		snapshotId: "snapshot",
		modelArchiveSha256: "model-hash",
		modelParameters: {},
		factorInstances: [{ alias: "alpha", archiveSha256: "factor-hash" }],
		seed: 0,
	});
});

test("preserves exact missing binding and native error evidence", () => {
	const model = component({
		dependencies: [{ componentId: "factor", version: "^1.0.0", alias: "alpha" }],
	});
	expect(() =>
		datasetGenerationRequest("user", "snapshot", model, [model], { alpha: [] }),
	).toThrow("Required Factor alpha is not available.");
	expect(formatModelError("typed-evidence: row 7")).toBe(
		"typed-evidence: row 7",
	);
});

test("maps persisted Dataset statuses to readable inspection evidence", () => {
	expect(
		datasetStatusSummary({ "missing-input": 2, "model-warmup": 4, present: 8 }),
	).toBe("missing-input: 2, model-warmup: 4, present: 8");
});

test("maps Forecast Evaluation summaries and authoritative filenames", () => {
	expect(
		evaluationReportSummary({
			reportId: "report",
			evidenceState: { summary: "unknown" },
			metrics: { alignedCount: 8, coverage: 0.8, missingness: 0.2 },
		}),
	).toBe("8 aligned · 80.00% coverage · 20.00% missing · unknown · report");
	expect(evaluationExportFilename("report", "json")).toBe(
		"forecast-evaluation-report-report.json",
	);
	expect(evaluationExportFilename("report", "markdown")).toBe(
		"forecast-evaluation-report-report.md",
	);
});

test("maps reportable signals to Target-specific metric presentations", () => {
	const output = (
		predictionKind: string,
		target: string,
		valueScale: string,
	) => ({
		name: "signal",
		predictionKind: { kind: predictionKind },
		forecastTarget: { kind: "builtin", target },
		valueScale: { kind: valueScale },
		horizonBars: 1,
	});
	expect(
		isCompatibleEvaluationSignal(
			output("expected-value", "future-close-return", "native"),
		),
	).toBe(true);
	expect(
		isCompatibleEvaluationSignal(
			output("probability", "future-close-up", "probability"),
		),
	).toBe(true);
	expect(
		evaluationMetricKind(output("probability", "future-close-up", "probability")),
	).toBe("probability");
	expect(
		isCompatibleEvaluationSignal(
			output("probability", "future-close-return", "probability"),
		),
	).toBe(false);
	expect(
		isCompatibleEvaluationSignal(
			output("probability", "future-close-up", "native"),
		),
	).toBe(false);
	const custom = {
		...output("probability", "", "probability"),
		forecastTarget: { kind: "custom", valueType: "binary" },
	};
	expect(isCompatibleEvaluationSignal(custom)).toBe(true);
	expect(evaluationMetricKind(custom)).toBe("custom-binary");
	expect(EVALUATION_METRIC_DEFINITIONS.logLoss.range).toContain("34.539");
	expect(EVALUATION_METRIC_DEFINITIONS.rocAuc.caveat).toContain(
		"both realized classes",
	);
	expect(EVALUATION_METRIC_DEFINITIONS.calibration.caveat).toContain(
		"weak evidence",
	);
});
