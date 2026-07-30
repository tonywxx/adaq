import { AppSidebar } from "@/components/app-sidebar";
import { AppTitlebar } from "@/components/app-titlebar";
import { CryptoKlineCard } from "@/components/crypto-kline-card";
import { CryptoTickerCard } from "@/components/crypto-ticker-card";
import { SidebarInset, SidebarProvider } from "@/components/ui/sidebar";
import { Skeleton } from "@/components/ui/skeleton";
import { Toaster } from "@/components/ui/sonner";
import { WatchlistCard } from "@/components/watchlist-card";
import { useMarketSessionStore } from "@/lib/market-session";
import type { ReactNode } from "react";

export default function Home({ children }: { children?: ReactNode }) {
	return (
		<SidebarProvider
			className="h-svh overflow-hidden bg-sidebar pt-(--header-height)"
			style={
				{
					"--sidebar-width": "calc(var(--spacing) * 72)",
					"--titlebar-sidebar-collapsed-width": "calc(var(--spacing) * 45)",
					"--header-height": "calc(var(--spacing) * 12)",
				} as React.CSSProperties
			}
		>
			<AppTitlebar />
			<Toaster />

			<AppSidebar className="top-14 h-[calc(100svh-3.5rem)]" variant="inset" />
			<SidebarInset className="m-0! rounded-none! min-h-0 overflow-y-auto border-l border-border shadow-none!">
				{children ?? <Dashboard />}
			</SidebarInset>
		</SidebarProvider>
	);
}

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
	return (
		<div
			className="grid min-w-0 gap-4 px-4 py-4 lg:grid-cols-[minmax(360px,420px)_minmax(0,1fr)] lg:px-6 lg:py-6"
			aria-busy="true"
		>
			<Skeleton className="h-[32rem] w-full" />
			<div className="flex min-w-0 flex-col gap-4">
				<Skeleton className="h-48 w-full" />
				<Skeleton className="h-[30rem] w-full" />
			</div>
		</div>
	);
}
