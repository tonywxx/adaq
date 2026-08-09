import type { InstrumentRef, Venue, VenueKind } from "@/lib/market-session";

export type MarketId = "crypto" | "a-shares" | "us-equities";

export type InstrumentStatus =
	| "live"
	| "suspended"
	| "preOpen"
	| "test"
	| "unknown";

export type InstrumentId = {
	venue: Venue;
	code: string;
};

export type SourceMapping = {
	provider: string;
	providerSymbol: string;
	connectorVersion: string;
	capturedAtMs: number;
};

export type ProviderCapabilitySnapshot = {
	provider: string;
	capturedAtMs: number;
	subscriptionPlan?: string;
	feed?: string;
	coverage?: string;
	realtime?: boolean;
	venues: string[];
	recordTypes: string[];
	historyStartMs?: number;
	historyEndMs?: number;
	delayed: boolean;
	delayedKnown: boolean;
	delayMs?: number;
	rateLimit?: string;
	rateLimitKnown: boolean;
	requestsPerMinute?: number;
	streamConnectionLimit?: number;
	streamingSymbolLimit?: number;
	unavailableCapabilities: string[];
	limitations: string[];
};

export type AshareInstrument = {
	instrument: InstrumentId;
	providerSymbol: string;
	name?: string;
	status: InstrumentStatus;
	listingTimeMs?: number;
	continuousTradingTimeMs?: number;
	currentPrice?: string;
	currentBaseVolume?: string;
	currentQuoteVolume?: string;
	currentObservedAtMs?: number;
	mapping: SourceMapping;
};

export type AlpacaInstrument = {
	instrument: InstrumentId;
	providerSymbol: string;
	name?: string;
	status: InstrumentStatus;
	assetClass: string;
	exchange: string;
	tradable: boolean;
	marginable: boolean;
	shortable: boolean;
	easyToBorrow: boolean;
	fractionable: boolean;
	listingTimeMs?: number;
	continuousTradingTimeMs?: number;
	priceIncrement?: string;
	quantityIncrement?: string;
	minimumQuantity?: string;
	mapping: SourceMapping;
};

export type InstrumentMasterSnapshot<T> = {
	snapshotId: string;
	effectiveAtMs: number;
	provider: string;
	actualUpstream: string;
	method: string;
	connectorVersion: string;
	retrievedAtMs?: number;
	capabilitySnapshot: ProviderCapabilitySnapshot;
	evidenceState: "observed" | "reconstructed" | "unknown";
	instruments: T[];
	limitations: string[];
};

export type MarketInstrument = {
	key: string;
	market: MarketId;
	instrument: InstrumentId;
	ref: InstrumentRef;
	provider: string;
	providerSymbol: string;
	name?: string;
	status: InstrumentStatus;
	observedAtMs?: number;
	last?: string;
	baseVolume?: string;
	quoteVolume?: string;
	exchange?: string;
	capability?: ProviderCapabilitySnapshot;
	limitations: string[];
};

export type TradingSession = {
	phase: SessionPhase;
	startLocal: string;
	endLocal: string;
};

export type DayEvidence = {
	date: { year: number; month: number; day: number };
	dayKind:
		| "tradingDay"
		| "holiday"
		| "weekend"
		| "specialClosure"
		| "unavailable";
	sessionOverride?: TradingSession[];
};

export type TradingCalendarSnapshot = {
	snapshotId: string;
	venue: Venue;
	effectiveFromMs: number;
	effectiveToMs: number;
	defaultSessions: TradingSession[];
	days: DayEvidence[];
};

export type CalendarDto = {
	snapshot: TradingCalendarSnapshot;
	provider: string;
	actualUpstream: string;
	method: string;
	retrievedAtMs: number;
	limitations: string[];
};

export type SessionPhase =
	| "preOpen"
	| "auction"
	| "continuous"
	| "break"
	| "extendedHours"
	| "closed"
	| "unknown";

export type SessionState = {
	phase: SessionPhase;
	tradingDate?: string;
	timeZone: string;
	reason?: string;
};

export function marketForVenueKind(kind: VenueKind): MarketId {
	switch (kind) {
		case "cryptoSpot":
			return "crypto";
		case "chinaAShareEquity":
			return "a-shares";
		case "usEquity":
			return "us-equities";
	}
}

export function marketMatches(instrument: InstrumentRef, market: MarketId) {
	if (instrument.venue)
		return marketForVenueKind(instrument.venue.kind) === market;
	return (
		(market === "crypto" && instrument.src === "okx") ||
		(market === "a-shares" && instrument.src === "akshare-rs") ||
		(market === "us-equities" && instrument.src === "alpaca")
	);
}

export function instrumentKey(instrument: InstrumentId) {
	return `${instrument.venue.id}:${instrument.code}`;
}

export function toWatchlistRef(
	instrument: InstrumentId,
	provider: string,
): InstrumentRef {
	return { src: provider, code: instrument.code, venue: instrument.venue };
}

export function normalizeAshareSnapshot(
	snapshot: InstrumentMasterSnapshot<AshareInstrument>,
): MarketInstrument[] {
	return snapshot.instruments.map((value) => ({
		key: instrumentKey(value.instrument),
		market: "a-shares",
		instrument: value.instrument,
		ref: toWatchlistRef(value.instrument, snapshot.provider),
		provider: snapshot.provider,
		providerSymbol: value.providerSymbol,
		name: value.name,
		status: value.status,
		observedAtMs: value.currentObservedAtMs ?? snapshot.effectiveAtMs,
		last: value.currentPrice,
		baseVolume: value.currentBaseVolume,
		quoteVolume: value.currentQuoteVolume,
		limitations: snapshot.limitations,
		capability: snapshot.capabilitySnapshot,
	}));
}

export function normalizeUsEquitySnapshot(
	snapshot: InstrumentMasterSnapshot<AlpacaInstrument>,
): MarketInstrument[] {
	return snapshot.instruments.map((value) => ({
		key: instrumentKey(value.instrument),
		market: "us-equities",
		instrument: value.instrument,
		ref: toWatchlistRef(value.instrument, snapshot.provider),
		provider: snapshot.provider,
		providerSymbol: value.providerSymbol,
		name: value.name,
		status: value.status,
		exchange: value.exchange,
		limitations: snapshot.limitations,
		capability: snapshot.capabilitySnapshot,
	}));
}

export function resolveSession(
	calendar: TradingCalendarSnapshot | undefined,
	nowMs: number,
	venue: Venue,
): SessionState {
	const timeZone = venue.timeZone;
	if (!calendar) {
		return {
			phase: "unknown",
			timeZone,
			reason: "Trading Calendar evidence is unavailable",
		};
	}
	if (
		calendar.venue.id !== venue.id ||
		calendar.venue.kind !== venue.kind ||
		!between(nowMs, calendar.effectiveFromMs, calendar.effectiveToMs)
	) {
		return {
			phase: "unknown",
			timeZone,
			reason: "The selected calendar does not cover the current Venue instant",
		};
	}

	const local = localParts(nowMs, timeZone);
	const tradingDate = `${local.year}-${String(local.month).padStart(2, "0")}-${String(local.day).padStart(2, "0")}`;
	const day = calendar.days.find(
		(value) =>
			value.date.year === local.year &&
			value.date.month === local.month &&
			value.date.day === local.day,
	);
	if (day && day.dayKind !== "tradingDay") {
		return { phase: "closed", tradingDate, timeZone };
	}
	if ([0, 6].includes(local.weekday)) {
		return { phase: "closed", tradingDate, timeZone };
	}

	const sessions = day?.sessionOverride ?? calendar.defaultSessions;
	const minutes = local.hour * 60 + local.minute;
	const active = sessions.find((session) => {
		const start = parseLocalTime(session.startLocal);
		const end = parseLocalTime(session.endLocal);
		return minutes >= start && minutes < end;
	});
	if (active) return { phase: active.phase, tradingDate, timeZone };
	return { phase: "closed", tradingDate, timeZone };
}

function between(value: number, start: number, end: number) {
	return value >= start && value < end;
}

function parseLocalTime(value: string) {
	const [hour = "0", minute = "0"] = value.split(":");
	return Number(hour) * 60 + Number(minute);
}

function localParts(value: number, timeZone: string) {
	const parts = new Intl.DateTimeFormat("en-US", {
		timeZone,
		year: "numeric",
		month: "numeric",
		day: "numeric",
		hour: "numeric",
		minute: "numeric",
		hourCycle: "h23",
		weekday: "short",
	}).formatToParts(new Date(value));
	const get = (type: Intl.DateTimeFormatPartTypes) =>
		Number(parts.find((part) => part.type === type)?.value ?? 0);
	const weekday = parts.find((part) => part.type === "weekday")?.value;
	return {
		year: get("year"),
		month: get("month"),
		day: get("day"),
		hour: get("hour"),
		minute: get("minute"),
		weekday: weekday === "Sun" ? 0 : weekday === "Sat" ? 6 : 1,
	};
}
