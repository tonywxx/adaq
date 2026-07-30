import type { LibraryComponent } from "@/features/components/component-library";
import {
	defaultExecutionProfile,
	matchingFactors,
	runGate,
} from "./guided-backtest";

const component = (overrides: Partial<LibraryComponent>): LibraryComponent => ({
	componentId: "factor-id",
	version: "1.0.0",
	manifestSchemaVersion: "1.0.0",
	sdkVersion: "1.0.0",
	abiVersion: "1.0.0",
	name: "Factor",
	kind: "factor",
	archiveSha256: "a".repeat(64),
	wasmSha256: "b".repeat(64),
	parameters: [],
	featureSlots: [],
	outputNames: [],
	dependencies: [],
	warmupBars: 0,
	compatible: true,
	lockedByRunIds: [],
	...overrides,
});

test("shows only usable Factor Components for a required dependency", () => {
	const dependency = { componentId: "factor-id", version: "^1", alias: "signal" };
	expect(
		matchingFactors(dependency, [
			component({}),
			component({ componentId: "other" }),
			component({ compatible: false, compatibilityError: "unsupported ABI" }),
		]),
	).toHaveLength(1);
});

test("stage gates prevent incomplete and duplicate-ready submissions", () => {
	const strategy = component({ kind: "strategy", dependencies: [] });
	expect(
		runGate({
			strategy,
			dependencies: [],
			factorSelections: {},
			initialQuoteAllocation: "10000",
		}),
	).toMatch(/Snapshot/);
	expect(
		runGate({
			snapshotId: "snapshot",
			strategy,
			dependencies: [],
			factorSelections: {},
			initialQuoteAllocation: "10000.00",
		}),
	).toBeUndefined();
	expect(
		runGate({
			snapshotId: "snapshot",
			strategy,
			dependencies: [],
			factorSelections: {},
			initialQuoteAllocation: "10000.00",
			running: true,
		}),
	).toMatch(/already running/);
	expect(defaultExecutionProfile.fillPolicy).toBe("taker");
});
