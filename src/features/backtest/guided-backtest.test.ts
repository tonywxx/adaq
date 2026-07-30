import type { LibraryComponent } from "@/features/components/component-library";
import {
	copyRunConfiguration,
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
			component({ version: "2.0.0", archiveSha256: "c".repeat(64) }),
			component({ compatible: false, compatibilityError: "unsupported ABI" }),
		], ["a".repeat(64)]),
	).toHaveLength(1);
});

test("stage gates prevent incomplete and duplicate-ready submissions", () => {
	const strategy = component({ kind: "strategy", dependencies: [] });
	expect(
		runGate({
			strategy,
			dependencies: [],
			factorSelections: {},
		}),
	).toMatch(/Snapshot/);
	expect(
		runGate({
			snapshotId: "snapshot",
			strategy,
			dependencies: [],
			factorSelections: {},
		}),
	).toBeUndefined();
	expect(
		runGate({
			snapshotId: "snapshot",
			strategy,
			dependencies: [],
			factorSelections: {},
			running: true,
		}),
	).toMatch(/already running/);
	expect(defaultExecutionProfile.fillPolicy).toBe("taker");
});

test("copies frozen normalized settings into a new editable configuration", () => {
	expect(
		copyRunConfiguration({
			snapshotId: "snapshot",
			strategyArchiveSha256: "strategy",
			strategyParameters: { period: "20" },
			factorInstances: [
				{
					alias: "signal",
					archiveSha256: "factor",
					parameters: [{ name: "length", value: "10" }],
				},
			],
			initialQuoteAllocation: "10000.00",
			executionProfile: defaultExecutionProfile,
			seed: 7,
		}),
	).toEqual({
		snapshotId: "snapshot",
		strategy: "strategy",
		strategyParameters: { period: "20" },
		factorSelections: { signal: "factor" },
		factorParameters: { signal: { length: "10" } },
		initialQuoteAllocation: "10000.00",
		executionProfile: defaultExecutionProfile,
		seed: "7",
	});
});
