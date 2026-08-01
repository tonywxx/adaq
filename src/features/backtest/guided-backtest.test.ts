import type { LibraryComponent } from "@/features/components/component-library";
import {
	copyRunConfiguration,
	decisionSignalEvidence,
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
	expect(
		matchingFactors(
			[
				component({}),
				component({ componentId: "other", archiveSha256: "d".repeat(64) }),
				component({ version: "2.0.0", archiveSha256: "c".repeat(64) }),
				component({
					archiveSha256: "e".repeat(64),
					compatible: false,
					compatibilityError: "unsupported ABI",
				}),
			],
			["a".repeat(64)],
		),
	).toHaveLength(1);
});

test("uses the host-compatible hashes as the Factor selection authority", () => {
	const factor = component({
		componentId: "stale-client-id",
		compatible: false,
		compatibilityError: "stale client metadata",
	});
	expect(matchingFactors([factor], [factor.archiveSha256])).toEqual([factor]);
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
			signalSlots: [{ name: "forecast" }],
			signalSelections: {},
		}),
	).toMatch(/Dataset Signal/);
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
			runStartTimeMs: 1,
			runEndTimeMs: 2,
			strategyArchiveSha256: "strategy",
			strategyParameters: { period: "20" },
			factorInstances: [
				{
					alias: "signal",
					archiveSha256: "factor",
					parameters: [{ name: "length", value: "10" }],
				},
			],
			signalInstances: [
				{ slot: "forecast", datasetId: "dataset", signalName: "up" },
			],
			initialQuoteAllocation: "10000.00",
			executionProfile: defaultExecutionProfile,
			seed: 7,
		}),
	).toEqual({
		snapshotId: "snapshot",
		runStartTimeMs: 1,
		runEndTimeMs: 2,
		strategy: "strategy",
		strategyParameters: { period: "20" },
		factorSelections: { signal: "factor" },
		factorParameters: { signal: { length: "10" } },
		signalSelections: { forecast: "dataset:up" },
		initialQuoteAllocation: "10000.00",
		executionProfile: defaultExecutionProfile,
		seed: "7",
	});
});

test("maps each decision to its exact Dataset and Producer Segment evidence", () => {
	expect(
		decisionSignalEvidence(
			JSON.stringify({
				slots: [
					{
						name: "forecast",
						source: {
							kind: "signal",
							dataset_id: "dataset",
							signal_name: "up",
							evidence_state: "unknown",
							bar_interval: "1m",
							producer_segments: [
								{
									startPredictionTimeMs: 60_010,
									endPredictionTimeMs: 60_020,
									modelArtifact: { sha256: "artifact" },
								},
							],
						},
					},
				],
			}),
			15,
		),
	).toContain('"modelArtifact":{"sha256":"artifact"}');
});
