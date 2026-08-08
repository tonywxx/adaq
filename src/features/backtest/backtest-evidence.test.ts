import { decisionSignalEvidence } from "./backtest-evidence";

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
