export function holdoutGate(input?: {
	runId?: string;
	sampleOutStartTimeMs?: number;
}) {
	if (!input?.runId) return "Select a completed Backtest Run first.";
	if (!input.sampleOutStartTimeMs)
		return "Choose a chronological sample-out boundary.";
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
}) {
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
