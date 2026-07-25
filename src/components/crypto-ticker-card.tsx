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
import {
	calculateChange,
	formatNumber,
	instrumentKey,
	useMarketSessionStore,
} from "@/lib/market-session";

export function CryptoTickerCard() {
	const activeInstrument = useMarketSessionStore(
		(state) => state.activeInstrument,
	);
	const ticker = useMarketSessionStore(
		(state) => state.tickers[instrumentKey(activeInstrument)],
	);
	const status = useMarketSessionStore((state) => state.tickerStatus);
	const error = useMarketSessionStore((state) => state.streamError);
	const [baseAsset = activeInstrument.code, quoteAsset = ""] =
		activeInstrument.code.split("-");

	const change = ticker ? calculateChange(ticker.last, ticker.open24h) : null;

	return (
		<div className="*:data-[slot=card]:from-primary/5 *:data-[slot=card]:to-card dark:*:data-[slot=card]:bg-card *:data-[slot=card]:bg-linear-to-t *:data-[slot=card]:shadow-xs">
			<Card className="@container/card rounded-md py-4">
				<CardHeader>
					<CardDescription>
						{baseAsset} / {quoteAsset} · OKX Spot
					</CardDescription>
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
							{error ?? `Loading ${activeInstrument.code} ticker…`}
						</div>
					)}
				</CardContent>
				<CardFooter className="flex-col items-start gap-1.5 text-sm">
					<div className="flex flex-wrap items-center gap-x-2 font-medium">
						<span>{status === "live" ? "Live WebSocket" : "Reconnecting"}</span>
						{ticker && (
							<span className="text-muted-foreground">
								Updated {new Date(ticker.timestampMs).toLocaleTimeString()}
							</span>
						)}
					</div>
					{ticker && (
						<div className="text-muted-foreground">
							24h volume {formatNumber(ticker.baseVolume24h, 4)} {baseAsset} ·{" "}
							{formatNumber(ticker.quoteVolume24h, 2)} {quoteAsset}
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
