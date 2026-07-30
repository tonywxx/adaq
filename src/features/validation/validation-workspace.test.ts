import {
	crossMarketGate,
	crossMarketProtocolFields,
	formatValidationError,
	holdoutGate,
	protocolDetails,
	protocolSummary,
	reportExportFilename,
	validationRunRequest,
	walkForwardGate,
	walkForwardProtocolFields,
	walkForwardPreview,
} from "./validation-workspace";

test("freezes an explicitly ordered unique cross-market protocol", () => {
	expect(
		crossMarketGate({ runId: "run", snapshotIds: ["btc", "eth"] }),
	).toBeUndefined();
	expect(
		crossMarketProtocolFields([
			{ snapshotId: "btc" },
			{ snapshotId: "eth", runOverride: { snapshotId: "eth" } },
		]),
	).toEqual({
		windows: [],
		crossMarket: {
			contexts: [
				{ snapshotId: "btc" },
				{ snapshotId: "eth", runOverride: { snapshotId: "eth" } },
			],
		},
		methodVersion: "cross-market@1",
	});
	expect(crossMarketGate({ runId: "run", snapshotIds: ["btc", "btc"] })).toMatch(
		/duplicate/i,
	);
	expect(crossMarketGate({ runId: "run", snapshotIds: ["btc"] })).toMatch(
		/two/i,
	);
});

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
	expect(
		protocolSummary({
			protocolId: "cross-identity",
			methodVersion: "cross-market@1",
			windows: [],
			crossMarket: { contexts: [{ snapshotId: "btc" }, { snapshotId: "eth" }] },
		}),
	).toBe("cross-market@1 · 2 market contexts · Protocol cross-identity");
});

test("previews deterministic complete walk-forward windows and rejects invalid history", () => {
	const bars = Array.from({ length: 20 }, (_, index) => ({
		openTimeMs: index * 3_600_000,
	}));
	const configuration = {
		windowSizeBars: 5,
		stepSizeBars: 5,
		minimumHistoryBars: 10,
	};
	expect(
		walkForwardGate({ runId: "run", barCount: bars.length, configuration }),
	).toBeUndefined();
	expect(walkForwardPreview(bars, configuration)).toEqual({
		windows: [
			{ sampleOutStartTimeMs: 10 * 3_600_000, sampleOutEndTimeMs: 15 * 3_600_000 },
			{ sampleOutStartTimeMs: 15 * 3_600_000, sampleOutEndTimeMs: undefined },
		],
		partialFinalWindow: false,
	});
	expect(
		walkForwardGate({
			runId: "run",
			barCount: bars.length,
			configuration: { ...configuration, minimumHistoryBars: bars.length },
		}),
	).toMatch(/more history/);
	expect(
		walkForwardPreview(bars, {
			...configuration,
			windowSizeBars: 6,
			stepSizeBars: 6,
		}),
	).toMatchObject({ partialFinalWindow: true });
	expect(
		walkForwardProtocolFields({ snapshotId: "snapshot", ...configuration }),
	).toEqual({
		windows: [],
		walkForward: { snapshotId: "snapshot", ...configuration },
		methodVersion: "walk-forward@1",
	});
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
