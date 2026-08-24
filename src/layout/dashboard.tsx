import { CryptoKlineCard } from "@/components/crypto-kline-card";
import { CryptoTickerCard } from "@/components/crypto-ticker-card";
import { Skeleton } from "@/components/ui/skeleton";
import { WatchlistCard } from "@/components/watchlist-card";
import { useMarketSessionStore } from "@/lib/market-session";
import { useTranslation } from "react-i18next";

export function Dashboard() {
	const ready = useMarketSessionStore((state) => state.ready);
	if (!ready) return <DashboardLoadingSkeleton />;
	return (
		<div className="flex flex-1 flex-col">
			<div className="@container/main flex flex-1 flex-col gap-2">
				<div className="flex flex-col gap-4 py-4 md:gap-6 md:py-6">
					<div className="grid min-w-0 gap-4 px-4 lg:grid-cols-[minmax(360px,420px)_minmax(0,1fr)] lg:px-6">
						<WatchlistCard />
						<div className="flex min-w-0 flex-col gap-4">
							<CryptoTickerCard />
							<CryptoKlineCard />
						</div>
					</div>
				</div>
			</div>
		</div>
	);
}

function DashboardLoadingSkeleton() {
	const { t } = useTranslation();

	return (
		<div
			className="grid min-w-0 gap-4 px-4 py-4 lg:grid-cols-[minmax(360px,420px)_minmax(0,1fr)] lg:py-6"
			aria-busy="true"
			aria-label={t("loading.page")}
			role="status"
		>
			<Skeleton className="h-[32rem] w-full" />
			<div className="flex min-w-0 flex-col gap-4">
				<Skeleton className="h-48 w-full" />
				<Skeleton className="h-[30rem] w-full" />
			</div>
		</div>
	);
}
