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
	if (protocol.crossMarket) {
		return `${protocol.methodVersion} · ${protocol.crossMarket.contexts.length} market context${protocol.crossMarket.contexts.length === 1 ? "" : "s"} · Protocol ${protocol.protocolId}`;
	}
	return `${protocol.methodVersion} · ${protocol.windows.length} window${protocol.windows.length === 1 ? "" : "s"} · Protocol ${protocol.protocolId}`;
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
