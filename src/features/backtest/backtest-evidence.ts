import {
	BAR_INTERVALS,
	nextOpenTimeMs,
	type BarInterval,
} from "@/lib/market-chart-adapter";

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
