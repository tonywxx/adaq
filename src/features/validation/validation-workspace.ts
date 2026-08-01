export function holdoutGate(input?: {
	runId?: string;
	sampleOutStartTimeMs?: number;
}) {
	if (!input?.runId) return "Select a completed Backtest Run first.";
	if (!input.sampleOutStartTimeMs)
		return "Choose a chronological sample-out boundary.";
}

export function crossMarketGate(input?: {
	runId?: string;
	snapshotIds?: readonly string[];
}) {
	if (!input?.runId)
		return "Select a completed Backtest Run for the shared configuration first.";
	if (!input.snapshotIds || input.snapshotIds.length < 2)
		return "Cross-market validation requires at least two market contexts.";
	if (new Set(input.snapshotIds).size !== input.snapshotIds.length)
		return "Cross-market validation contains a duplicate Snapshot.";
}

export function crossMarketProtocolFields(
	contexts: Array<{ snapshotId: string; runOverride?: unknown }>,
) {
	return {
		windows: [],
		crossMarket: { contexts },
		methodVersion: "cross-market@1",
	};
}

export type WalkForwardConfiguration = {
	windowSizeBars: number;
	stepSizeBars: number;
	minimumHistoryBars: number;
};

export type WalkForwardPreview = {
	windows: Array<{
		sampleOutStartTimeMs: number;
		sampleOutEndTimeMs?: number;
	}>;
	partialFinalWindow: boolean;
};

export function walkForwardGate(input?: {
	runId?: string;
	barCount?: number;
	configuration?: WalkForwardConfiguration;
}) {
	if (!input?.runId) return "Select a completed Backtest Run first.";
	const configuration = input.configuration;
	if (
		!configuration ||
		![
			configuration.windowSizeBars,
			configuration.stepSizeBars,
			configuration.minimumHistoryBars,
		].every((value) => Number.isInteger(value) && value > 0)
	)
		return "Walk-forward window sizes must be positive";
	if (configuration.stepSizeBars < configuration.windowSizeBars)
		return "Walk-forward step must not overlap sample-out windows";
	if (!input.barCount || configuration.minimumHistoryBars >= input.barCount)
		return "Walk-forward requires more history than the minimum.";
	if (
		configuration.minimumHistoryBars + configuration.windowSizeBars >
		input.barCount
	)
		return "Walk-forward history cannot produce a complete window.";
}

export function walkForwardProtocolFields(
	walkForward: WalkForwardConfiguration & { snapshotId: string },
) {
	return {
		windows: [],
		walkForward,
		methodVersion: "walk-forward@1",
	};
}

export function walkForwardPreview(
	bars: readonly { openTimeMs: number }[],
	configuration: WalkForwardConfiguration,
): WalkForwardPreview {
	const windows = [];
	for (
		let start = configuration.minimumHistoryBars;
		start + configuration.windowSizeBars <= bars.length;
		start += configuration.stepSizeBars
	) {
		windows.push({
			sampleOutStartTimeMs: bars[start].openTimeMs,
			sampleOutEndTimeMs: bars[start + configuration.windowSizeBars]?.openTimeMs,
		});
	}
	const nextStart =
		configuration.minimumHistoryBars +
		windows.length * configuration.stepSizeBars;
	return {
		windows,
		partialFinalWindow:
			nextStart < bars.length &&
			nextStart + configuration.windowSizeBars > bars.length,
	};
}

export function formatValidationError(error: unknown) {
	return {
		summary: "Validation could not freeze the Protocol.",
		details: String(error),
	};
}

export function protocolSummary(protocol: {
	protocolId: string;
	methodVersion: string;
	windows: readonly unknown[];
	crossMarket?: { contexts: readonly unknown[] };
}) {
	if (protocol.crossMarket)
		return `${protocol.methodVersion} · ${protocol.crossMarket.contexts.length} market context${protocol.crossMarket.contexts.length === 1 ? "" : "s"} · Protocol ${protocol.protocolId}`;
	return `${protocol.methodVersion} · ${protocol.windows.length} window${protocol.windows.length === 1 ? "" : "s"} · Protocol ${protocol.protocolId}`;
}

export function validationRunRequest(
	userId: string,
	configuration: {
		snapshotId: string;
		strategyArchiveSha256: string;
		strategyParameters: Record<string, string>;
		factorInstances: Array<{
			alias: string;
			archiveSha256: string;
			parameters: Array<{ name: string; value: string }>;
		}>;
		signalInstances?: Array<{
			slot: string;
			datasetId: string;
			signalName: string;
		}>;
		initialQuoteAllocation: string;
		executionProfile: unknown;
		seed: number;
	},
) {
	return {
		...configuration,
		userId,
		factorInstances: configuration.factorInstances.map((factor) => ({
			...factor,
			parameters: Object.fromEntries(
				factor.parameters.map((parameter) => [parameter.name, parameter.value]),
			),
		})),
	};
}

export function protocolDetails(protocol: {
	windows: Array<{
		snapshotId: string;
		sampleOutStartTimeMs: number;
		sampleOutEndTimeMs?: number;
	}>;
	aggregationRuleVersion: string;
}) {
	return protocol.windows.map((window) => ({
		snapshotId: window.snapshotId,
		boundary: `${new Date(window.sampleOutStartTimeMs).toISOString()} – ${window.sampleOutEndTimeMs ? new Date(window.sampleOutEndTimeMs).toISOString() : "final"}`,
		aggregationRuleVersion: protocol.aggregationRuleVersion,
	}));
}

export function reportExportFilename(
	reportId: string,
	format: "json" | "markdown",
) {
	return `validation-report-${reportId}.${format === "json" ? "json" : "md"}`;
}
