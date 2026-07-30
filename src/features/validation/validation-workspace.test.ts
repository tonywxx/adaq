import {
	formatValidationError,
	holdoutGate,
	protocolDetails,
	protocolSummary,
	reportExportFilename,
	validationRunRequest,
} from "./validation-workspace";

test("requires a frozen Run and a chronological sample-out boundary", () => {
	expect(holdoutGate()).toMatch(/Backtest Run/);
	expect(holdoutGate({ runId: "run" })).toMatch(/sample-out/);
	expect(
		holdoutGate({ runId: "run", sampleOutStartTimeMs: 1_700_000_000_000 }),
	).toBeUndefined();
});

test("rebuilds the frozen normalized Run as a valid Validation request", () => {
	expect(
		validationRunRequest("alice", {
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
			initialQuoteAllocation: "100",
			executionProfile: { fillPolicy: "taker" },
			seed: 7,
		}),
	).toMatchObject({
		userId: "alice",
		factorInstances: [{ parameters: { length: "10" } }],
	});
});

test("keeps exact technical validation errors copyable", () => {
	expect(
		formatValidationError("Validation sample-out window must be non-empty"),
	).toEqual({
		summary: "Validation could not freeze the Protocol.",
		details: "Validation sample-out window must be non-empty",
	});
});

test("summarizes frozen Protocol evidence without hiding identities", () => {
	expect(
		protocolSummary({
			protocolId: "protocol-identity",
			methodVersion: "chronological-holdout@1",
			windows: [{ snapshotId: "snapshot-identity" }],
		}),
	).toBe("chronological-holdout@1 · 1 window · Protocol protocol-identity");
});

test("keeps frozen boundaries and export identities reviewable", () => {
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
	expect(reportExportFilename("report", "markdown")).toBe(
		"validation-report-report.md",
	);
});
