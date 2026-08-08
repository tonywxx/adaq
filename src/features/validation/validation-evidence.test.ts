import {
	formatValidationError,
	protocolDetails,
	protocolSummary,
	reportExportFilename,
} from "./validation-evidence";

test("keeps frozen Protocol identities and boundaries reviewable", () => {
	expect(
		protocolSummary({
			protocolId: "protocol-identity",
			methodVersion: "chronological-holdout@1",
			windows: [{ snapshotId: "snapshot-identity" }],
		}),
	).toBe("chronological-holdout@1 · 1 window · Protocol protocol-identity");
	expect(
		protocolSummary({
			protocolId: "cross-identity",
			methodVersion: "cross-market@1",
			windows: [],
			crossMarket: { contexts: [{ snapshotId: "btc" }, { snapshotId: "eth" }] },
		}),
	).toBe("cross-market@1 · 2 market contexts · Protocol cross-identity");
	expect(
		protocolDetails({
			aggregationRuleVersion: "equal-window@1",
			windows: [{ snapshotId: "snapshot", sampleOutStartTimeMs: 0 }],
		}),
	).toEqual([
		{
			snapshotId: "snapshot",
			boundary: "1970-01-01T00:00:00.000Z – final",
			aggregationRuleVersion: "equal-window@1",
		},
	]);
});

test("preserves native validation errors and export identities", () => {
	expect(
		formatValidationError("Validation sample-out window must be non-empty"),
	).toEqual({
		summary: "Validation could not freeze the Protocol.",
		details: "Validation sample-out window must be non-empty",
	});
	expect(reportExportFilename("report", "markdown")).toBe(
		"validation-report-report.md",
	);
});
