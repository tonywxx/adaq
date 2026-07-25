import { Badge } from "@/components/ui/badge";
import {
	Card,
	CardAction,
	CardContent,
	CardDescription,
	CardFooter,
	CardHeader,
	CardTitle,
} from "@/components/ui/card";
import { Channel, invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";

const REQUEST = {
	src: "okx",
	code: "BTC-USDT",
} as const;

type TickerSnapshot = {
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

type DataError = {
	src: string;
	code: string;
	message: string;
};

type TickerStreamEvent =
	| { event: "connected" }
	| { event: "snapshot"; data: TickerSnapshot }
	| { event: "error"; data: DataError }
	| { event: "reconnecting"; data: { delayMs: number } }
	| { event: "closed" };

type ConnectionStatus = "rest" | "live" | "reconnecting";

export function CryptoTickerCard() {
	const [ticker, setTicker] = useState<TickerSnapshot>();
	const [status, setStatus] = useState<ConnectionStatus>("rest");
	const [error, setError] = useState<string>();

	useEffect(() => {
		let disposed = false;
		const subscriptionId = crypto.randomUUID();
		const onEvent = new Channel<TickerStreamEvent>();
		const updateTicker = (snapshot: TickerSnapshot) => {
			setTicker((current) =>
				!current || snapshot.timestampMs >= current.timestampMs
					? snapshot
					: current,
			);
		};

		onEvent.onmessage = (event) => {
			if (disposed) return;
			switch (event.event) {
				case "connected":
					setStatus("live");
					setError(undefined);
					break;
				case "snapshot":
					updateTicker(event.data);
					setStatus("live");
					setError(undefined);
					break;
				case "error":
					setStatus("reconnecting");
					setError(event.data.message);
					break;
				case "reconnecting":
					setStatus("reconnecting");
					break;
				case "closed":
					setStatus("rest");
					break;
			}
		};

		void invoke<TickerSnapshot>("market_get_ticker", { request: REQUEST })
			.then((snapshot) => {
				if (!disposed) updateTicker(snapshot);
			})
			.catch((reason) => {
				if (!disposed) setError(getErrorMessage(reason));
			});
		void invoke("market_subscribe_ticker", {
			request: { ...REQUEST, subscriptionId },
			onEvent,
		}).catch((reason) => {
			if (!disposed) {
				setStatus("rest");
				setError(getErrorMessage(reason));
			}
		});

		return () => {
			disposed = true;
			void invoke("market_unsubscribe_ticker", {
				request: { subscriptionId },
			});
		};
	}, []);

	const change = ticker ? calculateChange(ticker.last, ticker.open24h) : null;

	return (
		<div className="*:data-[slot=card]:from-primary/5 *:data-[slot=card]:to-card dark:*:data-[slot=card]:bg-card *:data-[slot=card]:bg-linear-to-t *:data-[slot=card]:shadow-xs">
			<Card className="@container/card rounded-md py-4">
				<CardHeader>
					<CardDescription>BTC / USDT · OKX Spot</CardDescription>
					<CardTitle className="text-2xl font-semibold tabular-nums @[250px]/card:text-3xl">
						{ticker ? `${formatNumber(ticker.last)} USDT` : "—"}
					</CardTitle>
					<CardAction>
						<Badge
							variant="outline"
							className={
								change === null
									? undefined
									: change >= 0
										? "text-emerald-600 dark:text-emerald-400"
										: "text-red-600 dark:text-red-400"
							}
						>
							{change === null
								? "—"
								: `${change >= 0 ? "+" : ""}${change.toFixed(2)}%`}
						</Badge>
					</CardAction>
				</CardHeader>
				<CardContent>
					{ticker ? (
						<dl className="grid grid-cols-2 gap-x-5 gap-y-3 text-sm sm:grid-cols-4">
							<TickerField label="Bid" value={formatNumber(ticker.bidPrice)} />
							<TickerField label="Ask" value={formatNumber(ticker.askPrice)} />
							<TickerField label="24h High" value={formatNumber(ticker.high24h)} />
							<TickerField label="24h Low" value={formatNumber(ticker.low24h)} />
						</dl>
					) : (
						<div className="text-sm text-muted-foreground" aria-live="polite">
							{error ?? "Loading OKX ticker…"}
						</div>
					)}
				</CardContent>
				<CardFooter className="flex-col items-start gap-1.5 text-sm">
					<div className="flex flex-wrap items-center gap-x-2 font-medium">
						<span>{statusLabel(status)}</span>
						{ticker && (
							<span className="text-muted-foreground">
								Updated {new Date(ticker.timestampMs).toLocaleTimeString()}
							</span>
						)}
					</div>
					{ticker && (
						<div className="text-muted-foreground">
							24h volume {formatNumber(ticker.baseVolume24h, 4)} BTC ·{" "}
							{formatNumber(ticker.quoteVolume24h, 2)} USDT
						</div>
					)}
					{error && ticker && (
						<div className="text-destructive" role="status">
							{error}
						</div>
					)}
				</CardFooter>
			</Card>
		</div>
	);
}

function TickerField({ label, value }: { label: string; value: string }) {
	return (
		<div>
			<dt className="text-xs text-muted-foreground">{label}</dt>
			<dd className="font-medium tabular-nums">{value}</dd>
		</div>
	);
}

function calculateChange(last: string, open24h: string) {
	const lastValue = Number(last);
	const openValue = Number(open24h);
	return Number.isFinite(lastValue) &&
		Number.isFinite(openValue) &&
		openValue !== 0
		? ((lastValue - openValue) / openValue) * 100
		: null;
}

function formatNumber(value: string | null, maximumFractionDigits = 8) {
	if (value === null) return "—";
	const number = Number(value);
	return Number.isFinite(number)
		? number.toLocaleString(undefined, { maximumFractionDigits })
		: value;
}

function statusLabel(status: ConnectionStatus) {
	if (status === "live") return "Live";
	if (status === "reconnecting") return "Reconnecting…";
	return "REST";
}

function getErrorMessage(error: unknown) {
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
