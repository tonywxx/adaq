import {
	reuseSnapshot,
	provenanceMessage,
	snapshotError,
	snapshotRangeError,
	snapshotStatus,
} from "./backtest-data";

test("reports invalid Snapshot ranges and exact technical errors", () => {
	expect(snapshotRangeError("2026-07-30", "2026-07-30")).toBe(
		"Snapshot time range is invalid",
	);
	expect(snapshotRangeError("2026-07-31", "2026-07-30")).toBe(
		"Snapshot time range is invalid",
	);
	expect(snapshotError("provider: rate limit")).toBe("provider: rate limit");
});

test("formats Snapshot download progress and cancellation state", () => {
	expect(snapshotStatus("progress", 42)).toBe("Downloaded 42 Closed Bars…");
	expect(snapshotStatus("cancelled")).toBe("Download cancelled.");
});

test("reuses the selected immutable Snapshot", () => {
	const first = { snapshotId: "first" };
	const second = { snapshotId: "second" };
	expect(reuseSnapshot([first, second], "second")).toBe(second);
});

test("keeps incomplete legacy provenance explicit", () => {
	expect(provenanceMessage(false)).toMatch(/Legacy Run/);
	expect(provenanceMessage(true)).toBeUndefined();
});
