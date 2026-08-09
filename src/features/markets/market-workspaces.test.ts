import {
	instrumentKey,
	marketMatches,
	normalizeAshareSnapshot,
	resolveSession,
	type AshareInstrument,
	type InstrumentMasterSnapshot,
} from "@/features/markets/market-workspaces";
import type { Venue } from "@/lib/market-session";

const sse: Venue = {
	id: "sse",
	kind: "chinaAShareEquity",
	timeZone: "Asia/Shanghai",
};

test("uses Venue plus native code as the stable identity", () => {
	const sameCodeAtSzse = {
		venue: { ...sse, id: "szse" },
		code: "600000",
	};

	expect(instrumentKey({ venue: sse, code: "600000" })).not.toBe(
		instrumentKey(sameCodeAtSzse),
	);
	expect(
		marketMatches({ src: "akshare-rs", code: "600000", venue: sse }, "a-shares"),
	).toBe(true);
});

test("normalizes provider records without inventing unavailable quotes", () => {
	const snapshot: InstrumentMasterSnapshot<AshareInstrument> = {
		snapshotId: "snapshot-1",
		effectiveAtMs: 1_700_000_000_000,
		provider: "akshare-rs",
		actualUpstream: "eastmoney",
		method: "fixture",
		connectorVersion: "test",
		capabilitySnapshot: {
			provider: "akshare-rs",
			capturedAtMs: 1_700_000_000_000,
			venues: ["sse"],
			recordTypes: ["instrument-master"],
			delayed: true,
			delayedKnown: true,
			rateLimitKnown: false,
			unavailableCapabilities: ["bid-ask"],
			limitations: ["fixture data"],
		},
		evidenceState: "observed",
		instruments: [
			{
				instrument: { venue: sse, code: "600000" },
				providerSymbol: "600000.SS",
				name: "Example",
				status: "live",
				currentPrice: "10.00",
				mapping: {
					provider: "akshare-rs",
					providerSymbol: "600000.SS",
					connectorVersion: "test",
					capturedAtMs: 1_700_000_000_000,
				},
			},
		],
		limitations: ["fixture data"],
	};

	const [instrument] = normalizeAshareSnapshot(snapshot);
	expect(instrument.ref).toEqual({
		src: "akshare-rs",
		code: "600000",
		venue: sse,
	});
	expect(instrument.last).toBe("10.00");
	expect(instrument.baseVolume).toBeUndefined();
	expect(instrument.quoteVolume).toBeUndefined();
});

test("derives the Venue-local session and refuses uncovered calendar time", () => {
	const calendar = {
		snapshotId: "calendar-1",
		venue: sse,
		effectiveFromMs: Date.parse("2023-01-01T00:00:00Z"),
		effectiveToMs: Date.parse("2025-01-01T00:00:00Z"),
		defaultSessions: [
			{ phase: "continuous" as const, startLocal: "09:30", endLocal: "11:30" },
		],
		days: [],
	};

	expect(
		resolveSession(calendar, Date.parse("2024-01-02T02:00:00Z"), sse),
	).toEqual({
		phase: "continuous",
		tradingDate: "2024-01-02",
		timeZone: "Asia/Shanghai",
	});
	expect(resolveSession(undefined, Date.now(), sse).phase).toBe("unknown");
	expect(
		resolveSession(calendar, Date.parse("2024-01-06T02:00:00Z"), sse).phase,
	).toBe("closed");
});
