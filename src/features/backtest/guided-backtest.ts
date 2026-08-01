import type { LibraryComponent } from "@/features/components/component-library";
import {
	BAR_INTERVALS,
	nextOpenTimeMs,
	type BarInterval,
} from "@/lib/market-chart-adapter";

export const defaultExecutionProfile = {
	makerFeeRate: "0.0008",
	takerFeeRate: "0.001",
	adverseSlippageRate: "0.0005",
	rebalanceThreshold: "0",
	priceIncrement: "0.1",
	quantityIncrement: "0.00000001",
	minimumQuantity: "0.00001",
	riskFreeRate: "0",
	fillPolicy: "taker" as "maker" | "taker",
};

export type NormalizedRunConfiguration = {
	snapshotId: string;
	runStartTimeMs?: number;
	runEndTimeMs?: number;
	strategyArchiveSha256: string;
	strategyParameters: Record<string, string>;
	factorInstances: Array<{
		alias: string;
		archiveSha256: string;
		parameters: Array<{ name: string; value: string }>;
	}>;
	signalInstances: Array<{
		slot: string;
		datasetId: string;
		signalName: string;
	}>;
	initialQuoteAllocation: string;
	executionProfile: typeof defaultExecutionProfile;
	seed: number;
};

export function copyRunConfiguration(
	configuration: NormalizedRunConfiguration,
) {
	return {
		snapshotId: configuration.snapshotId,
		runStartTimeMs: configuration.runStartTimeMs,
		runEndTimeMs: configuration.runEndTimeMs,
		strategy: configuration.strategyArchiveSha256,
		strategyParameters: configuration.strategyParameters,
		factorSelections: Object.fromEntries(
			configuration.factorInstances.map((factor) => [
				factor.alias,
				factor.archiveSha256,
			]),
		),
		factorParameters: Object.fromEntries(
			configuration.factorInstances.map((factor) => [
				factor.alias,
				Object.fromEntries(
					factor.parameters.map((parameter) => [parameter.name, parameter.value]),
				),
			]),
		),
		signalSelections: Object.fromEntries(
			configuration.signalInstances.map((signal) => [
				signal.slot,
				`${signal.datasetId}:${signal.signalName}`,
			]),
		),
		initialQuoteAllocation: configuration.initialQuoteAllocation,
		executionProfile: configuration.executionProfile,
		seed: String(configuration.seed),
	};
}

type Dependency = LibraryComponent["dependencies"][number];

export function matchingFactors(
	components: readonly LibraryComponent[],
	compatibleHashes: readonly string[],
) {
	return components.filter(
		(component) =>
			component.kind === "factor" &&
			compatibleHashes.includes(component.archiveSha256),
	);
}

export function runGate({
	snapshotId,
	strategy,
	dependencies,
	factorSelections,
	signalSlots = [],
	signalSelections = {},
	running = false,
}: {
	snapshotId?: string;
	strategy?: LibraryComponent;
	dependencies: readonly Dependency[];
	factorSelections: Record<string, string>;
	signalSlots?: readonly { name: string }[];
	signalSelections?: Record<string, string>;
	running?: boolean;
}) {
	if (running) return "A Backtest is already running.";
	if (!snapshotId) return "Select a Market Data Snapshot before continuing.";
	if (!strategy)
		return "Select a compatible Strategy Component before continuing.";
	if (dependencies.some((dependency) => !factorSelections[dependency.alias]))
		return "Select a matching Factor Component for every required dependency.";
	if (signalSlots.some((slot) => !signalSelections[slot.name]))
		return "Select a compatible Dataset Signal for every Forecast Signal Slot.";
	return undefined;
}

export function decisionSignalEvidence(
	featurePlanJson: string,
	decisionTimeMs: number,
) {
	try {
		const plan = JSON.parse(featurePlanJson) as {
			slots?: Array<{
				name?: string;
				source?: {
					kind?: string;
					dataset_id?: string;
					signal_name?: string;
					evidence_state?: string;
					bar_interval?: string;
					artifact_provenance?: unknown;
					component_lock?: unknown;
					producer_segments?: Array<
						Record<string, unknown> & {
							startPredictionTimeMs?: number | null;
							endPredictionTimeMs?: number | null;
						}
					>;
				};
			}>;
		};
		const evidence = (plan.slots ?? []).flatMap((slot) => {
			const source = slot.source;
			if (source?.kind !== "signal") return [];
			const predictionTimeMs = BAR_INTERVALS.includes(
				source.bar_interval as BarInterval,
			)
				? nextOpenTimeMs(decisionTimeMs, source.bar_interval as BarInterval)
				: decisionTimeMs;
			const producerSegment = source.producer_segments?.find(
				(segment) =>
					(segment.startPredictionTimeMs ?? Number.MIN_SAFE_INTEGER) <=
						predictionTimeMs &&
					(segment.endPredictionTimeMs ?? Number.MAX_SAFE_INTEGER) >=
						predictionTimeMs,
			);
			return [
				{
					slot: slot.name,
					datasetId: source.dataset_id,
					signalName: source.signal_name,
					evidenceState: source.evidence_state,
					producerSegment,
					artifactProvenance: source.artifact_provenance,
					componentLock: source.component_lock,
				},
			];
		});
		return evidence.length
			? JSON.stringify(evidence)
			: "Composed inputs are recorded in the frozen Feature Plan.";
	} catch {
		return "Signal evidence is unavailable because the frozen Feature Plan is invalid.";
	}
}
