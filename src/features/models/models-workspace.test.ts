import type { LibraryComponent } from "@/features/components/component-library";
import {
	datasetGenerationRequest,
	datasetStatusSummary,
	formatModelError,
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
