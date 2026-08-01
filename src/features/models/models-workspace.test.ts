import type { LibraryComponent } from "@/features/components/component-library";
import {
	datasetGenerationRequest,
	datasetStatusSummary,
	formatModelError,
	signalRowPageRequest,
	signalRowSummary,
	evaluationRequest,
	evaluationReportSummary,
	evaluationExportFilename,
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

test("maps Forecast Evaluation requests, summaries, and authoritative filenames", () => {
	expect(
		evaluationRequest("user", "dataset", "snapshot", "return", 1, 10, 20, 5),
	).toEqual({
		userId: "user",
		datasetId: "dataset",
		snapshotId: "snapshot",
		signalName: "return",
		horizonBars: 1,
		evaluationStartTimeMs: 10,
		evaluationEndTimeMs: 20,
		stabilityWindowBars: 5,
	});
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
