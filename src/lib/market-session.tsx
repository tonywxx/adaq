import type { BarInterval, OhlcvBar } from "@/lib/market-chart-adapter";
import { formatDecimal as formatLocaleDecimal } from "@/lib/i18n";
import { Channel, invoke } from "@tauri-apps/api/core";
import { type ReactNode, useEffect, useMemo } from "react";
import { create } from "zustand";

export const DEFAULT_ACTIVE_INSTRUMENT = {
	src: "okx",
	code: "BTC-USDT",
	venue: { id: "okx", kind: "cryptoSpot", timeZone: "UTC" },
} as const;

export const MINI_CHART_INTERVALS = [
	"1m",
	"5m",
	"15m",
	"1h",
	"4h",
	"1d",
] as const satisfies readonly BarInterval[];

export type InstrumentRef = {
	src: string;
	code: string;
	venue?: Venue;
};

export type VenueKind = "cryptoSpot" | "chinaAShareEquity" | "usEquity";

export type Venue = {
	id: string;
	kind: VenueKind;
	timeZone: string;
};

export type WatchlistState = {
	items: InstrumentRef[];
	activeInstrument: InstrumentRef;
	miniChartInterval: BarInterval;
	limit: number;
};

export type TickerSnapshot = {
	src: string;
	code: string;
	last: string;
	lastQuantity: string;
	askPrice: string | null;
	askQuantity: string | null;
	bidPrice: string | null;
	bidQuantity: string | null;
	open24h: string;
	high24h: string;
	low24h: string;
	baseVolume24h: string;
	quoteVolume24h: string;
	timestampMs: number;
};

export type SpotInstrument = {
	src: string;
	code: string;
	baseAsset: string;
	quoteAsset: string;
	status: "live" | "suspended" | "preOpen" | "test" | "unknown";
};

type ConnectionStatus = "connecting" | "live" | "reconnecting";

type TickerStreamEvent =
	| { event: "connected" }
	| { event: "snapshot"; data: TickerSnapshot }
	| { event: "error"; data: { message: string } }
	| { event: "reconnecting"; data: { delayMs: number } }
	| { event: "closed" };

type BarStreamEvent =
	| { event: "connected" }
	| {
			event: "snapshot";
			data: {
				src: string;
				code: string;
				interval: BarInterval;
				bar: OhlcvBar;
				closed: boolean;
			};
	  }
	| { event: "error"; data: { message: string } }
	| { event: "reconnecting"; data: { delayMs: number } }
	| { event: "closed" };

type MarketSessionStore = {
	userId: string | null;
	ready: boolean;
	loadError?: string;
	watchlist: InstrumentRef[];
	watchlistLimit: number;
	activeInstrument: InstrumentRef;
	miniChartInterval: BarInterval;
	mainChartInterval: BarInterval;
	catalogLoaded: boolean;
	catalog: Record<string, SpotInstrument>;
	tickers: Record<string, TickerSnapshot>;
	liveBars: Record<string, OhlcvBar>;
	tickerStatus: ConnectionStatus;
	barStatus: ConnectionStatus;
	tickerError?: string;
	barError?: string;
	hydrate: (userId: string, state: WatchlistState) => void;
	failLoad: (userId: string, error: string) => void;
	clear: () => void;
	setMainChartInterval: (interval: BarInterval) => void;
	setCatalog: (instruments: SpotInstrument[]) => void;
	setTickerStatus: (status: ConnectionStatus, error?: string) => void;
	setBarStatus: (status: ConnectionStatus, error?: string) => void;
	updateTicker: (ticker: TickerSnapshot) => void;
	updateBar: (
		data: Extract<BarStreamEvent, { event: "snapshot" }>["data"],
	) => void;
};

const initialState = {
	userId: null,
	ready: false,
	loadError: undefined,
	watchlist: [],
	watchlistLimit: 0,
	activeInstrument: DEFAULT_ACTIVE_INSTRUMENT as InstrumentRef,
	miniChartInterval: "1m" as BarInterval,
	mainChartInterval: "15m" as BarInterval,
	catalogLoaded: false,
	catalog: {},
	tickers: {},
	liveBars: {},
	tickerStatus: "connecting" as ConnectionStatus,
	barStatus: "connecting" as ConnectionStatus,
	tickerError: undefined,
	barError: undefined,
};

export const useMarketSessionStore = create<MarketSessionStore>((set) => ({
	...initialState,
	hydrate: (userId, state) =>
		set({
			userId,
			ready: true,
			loadError: undefined,
			watchlist: state.items,
			watchlistLimit: state.limit,
			activeInstrument: state.activeInstrument,
			miniChartInterval: state.miniChartInterval,
		}),
	failLoad: (userId, loadError) => set({ userId, ready: false, loadError }),
	clear: () => set(initialState),
	setMainChartInterval: (mainChartInterval) => set({ mainChartInterval }),
	setCatalog: (instruments) =>
		set({
			catalogLoaded: true,
			catalog: Object.fromEntries(
				instruments.map((instrument) => [instrumentKey(instrument), instrument]),
			),
		}),
	setTickerStatus: (tickerStatus, tickerError) =>
		set((state) => ({
			tickerStatus,
			tickerError:
				tickerError ??
				(tickerStatus === "live" ? undefined : state.tickerError),
		})),
	setBarStatus: (barStatus, barError) =>
		set((state) => ({
			barStatus,
			barError:
				barError ?? (barStatus === "live" ? undefined : state.barError),
		})),
	updateTicker: (ticker) =>
		set((state) => {
			const key = instrumentKey(ticker);
			const current = state.tickers[key];
			if (current && current.timestampMs > ticker.timestampMs) return state;
			return {
				tickers: { ...state.tickers, [key]: ticker },
				tickerStatus: "live",
				tickerError: undefined,
			};
		}),
	updateBar: ({ src, code, interval, bar }) =>
		set((state) => ({
			liveBars: {
				...state.liveBars,
				[barKey({ src, code }, interval)]: bar,
			},
			barStatus: "live",
			barError: undefined,
		})),
}));

export function MarketSessionProvider({
	userId,
	children,
}: {
	userId: string;
	children: ReactNode;
}) {
	const ready = useMarketSessionStore((state) => state.ready);
	const watchlist = useMarketSessionStore((state) => state.watchlist);
	const activeInstrument = useMarketSessionStore(
		(state) => state.activeInstrument,
	);
	const miniChartInterval = useMarketSessionStore(
		(state) => state.miniChartInterval,
	);
	const mainChartInterval = useMarketSessionStore(
		(state) => state.mainChartInterval,
	);
	const catalogLoaded = useMarketSessionStore((state) => state.catalogLoaded);
	const catalog = useMarketSessionStore((state) => state.catalog);

	useEffect(() => {
		let disposed = false;
		void invoke<WatchlistState>("watchlist_get", {
			request: { userId },
		})
			.then((state) => {
				if (!disposed) useMarketSessionStore.getState().hydrate(userId, state);
			})
			.catch((error) => {
				if (!disposed) {
					useMarketSessionStore.getState().failLoad(userId, getErrorMessage(error));
				}
			});
		return () => {
			disposed = true;
			useMarketSessionStore.getState().clear();
		};
	}, [userId]);

	useEffect(() => {
		if (!ready) return;
		let disposed = false;
		void invoke<SpotInstrument[]>("market_list_spot_instruments", {
			request: { src: "okx" },
		})
			.then((instruments) => {
				if (disposed) return;
				useMarketSessionStore.getState().setCatalog(instruments);
				const active = useMarketSessionStore.getState().activeInstrument;
				const current = instruments.find(
					(instrument) => instrumentKey(instrument) === instrumentKey(active),
				);
				if (current?.status !== "live") {
					void setActiveInstrument(DEFAULT_ACTIVE_INSTRUMENT);
				}
			})
			.catch(() => {});
		return () => {
			disposed = true;
		};
	}, [ready]);

	const tickerCodes = useMemo(
		() =>
			[
				...new Set(
					[...watchlist, activeInstrument]
						.filter(
							(instrument) =>
								instrument.src === "okx" &&
								(!catalogLoaded ||
									catalog[instrumentKey(instrument)]?.status === "live"),
						)
						.map((instrument) => instrument.code),
				),
			].sort(),
		[activeInstrument, catalog, catalogLoaded, watchlist],
	);
	useEffect(() => {
		if (!ready || tickerCodes.length === 0) return;
		let disposed = false;
		const subscriptionId = crypto.randomUUID();
		const onEvent = new Channel<TickerStreamEvent>();
		const store = useMarketSessionStore.getState();
		store.setTickerStatus("connecting");
		onEvent.onmessage = (event) => {
			if (disposed) return;
			handleTickerEvent(event);
		};
		void invoke("market_subscribe_tickers", {
			request: { src: "okx", codes: tickerCodes, subscriptionId },
			onEvent,
		}).catch((error) => {
			if (!disposed) {
				useMarketSessionStore
					.getState()
					.setTickerStatus("reconnecting", getErrorMessage(error));
			}
		});
		return () => {
			disposed = true;
			void invoke("market_unsubscribe_ticker", {
				request: { subscriptionId },
			});
		};
	}, [ready, tickerCodes]);

	const barSubscriptions = useMemo(() => {
		const subscriptions = new Map<
			string,
			{ code: string; interval: BarInterval }
		>();
		for (const instrument of watchlist) {
			if (
				instrument.src !== "okx" ||
				(catalogLoaded && catalog[instrumentKey(instrument)]?.status !== "live")
			) {
				continue;
			}
			subscriptions.set(barKey(instrument, miniChartInterval), {
				code: instrument.code,
				interval: miniChartInterval,
			});
		}
		const activeCrypto =
			activeInstrument.src === "okx" &&
			(!activeInstrument.venue || activeInstrument.venue.kind === "cryptoSpot");
		if (
			activeCrypto &&
			(!catalogLoaded ||
				catalog[instrumentKey(activeInstrument)]?.status === "live")
		) {
			subscriptions.set(barKey(activeInstrument, mainChartInterval), {
				code: activeInstrument.code,
				interval: mainChartInterval,
			});
		}
		return [...subscriptions.values()].sort((left, right) =>
			`${left.code}:${left.interval}`.localeCompare(
				`${right.code}:${right.interval}`,
			),
		);
	}, [
		activeInstrument,
		catalog,
		catalogLoaded,
		mainChartInterval,
		miniChartInterval,
		watchlist,
	]);
	useEffect(() => {
		if (!ready || barSubscriptions.length === 0) return;
		let disposed = false;
		const subscriptionId = crypto.randomUUID();
		const onEvent = new Channel<BarStreamEvent>();
		useMarketSessionStore.getState().setBarStatus("connecting");
		onEvent.onmessage = (event) => {
			if (disposed) return;
			handleBarEvent(event);
		};
		void invoke("market_subscribe_bars", {
			request: { src: "okx", subscriptions: barSubscriptions, subscriptionId },
			onEvent,
		}).catch((error) => {
			if (!disposed) {
				useMarketSessionStore
					.getState()
					.setBarStatus("reconnecting", getErrorMessage(error));
			}
		});
		return () => {
			disposed = true;
			void invoke("market_unsubscribe_bar", {
				request: { subscriptionId },
			});
		};
	}, [barSubscriptions, ready]);

	return children;
}

export async function addWatchlistInstrument(instrument: InstrumentRef) {
	return mutateWatchlist("watchlist_add", { instrument });
}

export async function removeWatchlistInstrument(instrument: InstrumentRef) {
	return mutateWatchlist("watchlist_remove", { instrument });
}

export async function setActiveInstrument(instrument: InstrumentRef) {
	return mutateWatchlist("watchlist_set_active", { instrument });
}

export async function setMiniChartInterval(interval: BarInterval) {
	return mutateWatchlist("watchlist_set_interval", { interval });
}

async function mutateWatchlist(
	command: string,
	payload: Record<string, unknown>,
) {
	const { userId } = useMarketSessionStore.getState();
	if (!userId) throw new Error("market session is not ready");
	const state = await invoke<WatchlistState>(command, {
		request: { userId, ...payload },
	});
	useMarketSessionStore.getState().hydrate(userId, state);
	return state;
}

function handleTickerEvent(event: TickerStreamEvent) {
	const store = useMarketSessionStore.getState();
	switch (event.event) {
		case "connected":
			store.setTickerStatus("live");
			break;
		case "snapshot":
			store.updateTicker(event.data);
			break;
		case "error":
			store.setTickerStatus("reconnecting", event.data.message);
			break;
		case "reconnecting":
		case "closed":
			store.setTickerStatus("reconnecting");
			break;
	}
}

function handleBarEvent(event: BarStreamEvent) {
	const store = useMarketSessionStore.getState();
	switch (event.event) {
		case "connected":
			store.setBarStatus("live");
			break;
		case "snapshot":
			store.updateBar(event.data);
			break;
		case "error":
			store.setBarStatus("reconnecting", event.data.message);
			break;
		case "reconnecting":
		case "closed":
			store.setBarStatus("reconnecting");
			break;
	}
}

export function instrumentKey(instrument: InstrumentRef) {
	return `${instrument.venue?.id ?? instrument.src}:${instrument.code}`;
}

export function barKey(instrument: InstrumentRef, interval: BarInterval) {
	return `${instrumentKey(instrument)}:${interval}`;
}

export function calculateChange(last: string, open24h: string) {
	const lastValue = Number(last);
	const openValue = Number(open24h);
	return Number.isFinite(lastValue) &&
		Number.isFinite(openValue) &&
		openValue !== 0
		? ((lastValue - openValue) / openValue) * 100
		: null;
}

export function formatNumber(value: string | null, maximumFractionDigits = 8) {
	if (value === null) return "—";
	return formatLocaleDecimal(value, { maximumFractionDigits });
}

export function getErrorMessage(error: unknown) {
	if (
		typeof error === "object" &&
		error !== null &&
		"message" in error &&
		typeof error.message === "string"
	) {
		return error.message;
	}
	return String(error);
}
