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
import { formatDateTime, formatNumber as formatLocaleNumber } from "@/lib/i18n";
import { useTranslation } from "react-i18next";

export function CryptoTickerCard() {
	const { t } = useTranslation();
	const activeInstrument = useMarketSessionStore(
		(state) => state.activeInstrument,
	);
	const ticker = useMarketSessionStore(
		(state) => state.tickers[instrumentKey(activeInstrument)],
	);
	const status = useMarketSessionStore((state) => state.tickerStatus);
	const error = useMarketSessionStore((state) => state.tickerError);
	const [baseAsset = activeInstrument.code, quoteAsset = ""] =
		activeInstrument.code.split("-");

	const change = ticker ? calculateChange(ticker.last, ticker.open24h) : null;

	return (
		<div className="*:data-[slot=card]:from-primary/5 *:data-[slot=card]:to-card dark:*:data-[slot=card]:bg-card *:data-[slot=card]:bg-linear-to-t *:data-[slot=card]:shadow-xs">
			<Card className="@container/card rounded-md py-4">
				<CardHeader>
					<CardDescription>
						{t("market.instrumentVenue", { baseAsset, quoteAsset })}
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
								: `${formatLocaleNumber(change, {
										minimumFractionDigits: 2,
										maximumFractionDigits: 2,
										signDisplay: "always",
									})}%`}
						</Badge>
					</CardAction>
				</CardHeader>
				<CardContent>
					{ticker ? (
						<dl className="grid grid-cols-2 gap-x-5 gap-y-3 text-sm sm:grid-cols-4">
							<TickerField
								label={t("market.bid")}
								value={formatNumber(ticker.bidPrice)}
							/>
							<TickerField
								label={t("market.ask")}
								value={formatNumber(ticker.askPrice)}
							/>
							<TickerField
								label={t("market.high24h")}
								value={formatNumber(ticker.high24h)}
							/>
							<TickerField
								label={t("market.low24h")}
								value={formatNumber(ticker.low24h)}
							/>
						</dl>
					) : (
						<div className="text-sm text-muted-foreground" aria-live="polite">
							{error ??
								t("market.loadingTicker", { instrument: activeInstrument.code })}
						</div>
					)}
				</CardContent>
				<CardFooter className="flex-col items-start gap-1.5 text-sm">
					<div className="flex flex-wrap items-center gap-x-2 font-medium">
						<span>
							{status === "live"
								? t("market.liveWebSocket")
								: t("market.reconnecting")}
						</span>
						{ticker && (
							<span className="text-muted-foreground">
								{t("market.updatedAt", {
									time: formatDateTime(ticker.timestampMs, { timeStyle: "medium" }),
								})}
							</span>
						)}
					</div>
					{ticker && (
						<div className="text-muted-foreground">
							{t("market.volume24h", {
								baseVolume: formatNumber(ticker.baseVolume24h, 4),
								baseAsset,
								quoteVolume: formatNumber(ticker.quoteVolume24h, 2),
								quoteAsset,
							})}
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
