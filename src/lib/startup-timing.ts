type StartupMark =
	| "adaq:webview-start"
	| "adaq:react-entry"
	| "adaq:auth-loading-visible"
	| "adaq:auth-session-ready"
	| "adaq:host-auth-bound"
	| "adaq:help-visible";

export type StartupTimingReport = {
	webviewToReactEntryMs: number | null;
	reactEntryToAuthLoadingMs: number | null;
	authLoadingToAuthSessionMs: number | null;
	webviewToAuthSessionMs: number | null;
	authSessionToHostAuthMs: number | null;
	hostAuthToHelpMs: number | null;
	webviewToHelpMs: number | null;
};

declare global {
	interface Window {
		__ADAQ_STARTUP_TIMING__?: StartupTimingReport;
	}
}

function elapsed(start: StartupMark, end: StartupMark) {
	const startEntries = performance.getEntriesByName(start, "mark");
	const endEntries = performance.getEntriesByName(end, "mark");
	const startEntry = startEntries[0];
	const endEntry = endEntries[0];
	if (!startEntry || !endEntry || endEntry.startTime < startEntry.startTime) {
		return null;
	}
	return Math.round(endEntry.startTime - startEntry.startTime);
}

function reportStartupTiming() {
	if (typeof window === "undefined" || window.__ADAQ_STARTUP_TIMING__) return;

	const report: StartupTimingReport = {
		webviewToReactEntryMs: elapsed("adaq:webview-start", "adaq:react-entry"),
		reactEntryToAuthLoadingMs: elapsed(
			"adaq:react-entry",
			"adaq:auth-loading-visible",
		),
		authLoadingToAuthSessionMs: elapsed(
			"adaq:auth-loading-visible",
			"adaq:auth-session-ready",
		),
		webviewToAuthSessionMs: elapsed(
			"adaq:webview-start",
			"adaq:auth-session-ready",
		),
		authSessionToHostAuthMs: elapsed(
			"adaq:auth-session-ready",
			"adaq:host-auth-bound",
		),
		hostAuthToHelpMs: elapsed("adaq:host-auth-bound", "adaq:help-visible"),
		webviewToHelpMs: elapsed("adaq:webview-start", "adaq:help-visible"),
	};

	window.__ADAQ_STARTUP_TIMING__ = report;
	for (const [name, start, end] of [
		["adaq:webview-to-react-entry", "adaq:webview-start", "adaq:react-entry"],
		[
			"adaq:react-entry-to-auth-loading",
			"adaq:react-entry",
			"adaq:auth-loading-visible",
		],
		[
			"adaq:auth-loading-to-auth-session",
			"adaq:auth-loading-visible",
			"adaq:auth-session-ready",
		],
		[
			"adaq:webview-to-auth-session",
			"adaq:webview-start",
			"adaq:auth-session-ready",
		],
		[
			"adaq:auth-session-to-host-auth",
			"adaq:auth-session-ready",
			"adaq:host-auth-bound",
		],
		["adaq:host-auth-to-help", "adaq:host-auth-bound", "adaq:help-visible"],
		["adaq:webview-to-help", "adaq:webview-start", "adaq:help-visible"],
	] as const) {
		if (elapsed(start, end) !== null) performance.measure(name, start, end);
	}
	console.info("[AdaQ startup timing]", report);
}

export function markStartup(name: StartupMark) {
	if (typeof performance === "undefined") return;
	if (performance.getEntriesByName(name, "mark").length > 0) return;

	performance.mark(name);
	if (name === "adaq:help-visible") reportStartupTiming();
}
