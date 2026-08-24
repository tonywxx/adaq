import { markStartup } from "./startup-timing";

beforeEach(() => {
	Object.defineProperty(globalThis, "window", {
		configurable: true,
		value: {},
	});
	performance.clearMarks();
	performance.clearMeasures();
	delete window.__ADAQ_STARTUP_TIMING__;
});

test("reports one startup sequence even when marks are repeated", () => {
	const info = jest.spyOn(console, "info").mockImplementation(() => {});

	markStartup("adaq:webview-start");
	markStartup("adaq:webview-start");
	markStartup("adaq:react-entry");
	markStartup("adaq:auth-loading-visible");
	markStartup("adaq:auth-session-ready");
	markStartup("adaq:host-auth-bound");
	markStartup("adaq:help-visible");

	expect(window.__ADAQ_STARTUP_TIMING__).toEqual({
		webviewToReactEntryMs: expect.any(Number),
		reactEntryToAuthLoadingMs: expect.any(Number),
		authLoadingToAuthSessionMs: expect.any(Number),
		webviewToAuthSessionMs: expect.any(Number),
		authSessionToHostAuthMs: expect.any(Number),
		hostAuthToHelpMs: expect.any(Number),
		webviewToHelpMs: expect.any(Number),
	});
	expect(
		performance.getEntriesByName("adaq:webview-to-help", "measure"),
	).toHaveLength(1);
	expect(info).toHaveBeenCalledTimes(1);

	markStartup("adaq:help-visible");
	expect(info).toHaveBeenCalledTimes(1);
});
