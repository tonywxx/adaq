export function snapshotRangeError(start: string, end: string) {
	const startTimeMs = Date.parse(start);
	const endTimeMs = Date.parse(end);
	if (
		!Number.isFinite(startTimeMs) ||
		!Number.isFinite(endTimeMs) ||
		startTimeMs >= endTimeMs
	)
		return "Snapshot time range is invalid";
	return undefined;
}

export function snapshotStatus(
	event: "progress" | "completed" | "cancelled",
	downloadedBars?: number,
) {
	if (event === "progress")
		return `Downloaded ${downloadedBars ?? 0} Closed Bars…`;
	if (event === "cancelled") return "Download cancelled.";
	return "Snapshot download completed.";
}

export function snapshotError(error: unknown) {
	return String(error);
}

export function reuseSnapshot<T extends { snapshotId: string }>(
	snapshots: readonly T[],
	snapshotId: string,
) {
	return snapshots.find((snapshot) => snapshot.snapshotId === snapshotId);
}

export function provenanceMessage(hasProvenance: boolean) {
	return hasProvenance
		? undefined
		: "Legacy Run: complete provenance is unavailable, so this evidence remains readable but cannot be copied safely.";
}
