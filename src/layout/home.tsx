import { AppSidebar } from "@/components/app-sidebar";
import { AppTitlebar } from "@/components/app-titlebar";
import { CryptoKlineCard } from "@/components/crypto-kline-card";
import { CryptoTickerCard } from "@/components/crypto-ticker-card";
import { Button } from "@/components/ui/button";
import { SidebarInset, SidebarProvider } from "@/components/ui/sidebar";
import { Toaster } from "@/components/ui/sonner";
import { WatchlistCard } from "@/components/watchlist-card";
import { invoke } from "@tauri-apps/api/core";
import { useState } from "react";

export default function Home() {
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
							<FactorComponentName />
						</div>
					</div>
				</div>
			</SidebarInset>
		</SidebarProvider>
	);
}

function FactorComponentName() {
	const [schema, setSchema] = useState<FactorSchema>();
	const [error, setError] = useState<string>();
	const [loading, setLoading] = useState(false);

	const readName = async () => {
		setLoading(true);
		setError(undefined);

		try {
			setSchema(await invoke<FactorSchema>("get_factor_schema"));
		} catch (reason) {
			setError(String(reason));
		} finally {
			setLoading(false);
		}
	};

	return (
		<div className="flex items-center gap-3 px-4 lg:px-6">
			<Button type="button" onClick={readName} disabled={loading}>
				{loading ? "读取中…" : "读取 WASM 控件"}
			</Button>
			{schema && (
				<span className="text-sm font-medium">
					{schema.outputNames.join(", ")} (warmup: {schema.warmupBars})
				</span>
			)}
			{error && <span className="text-sm text-destructive">{error}</span>}
		</div>
	);
}

type FactorSchema = {
	outputNames: string[];
	warmupBars: number;
};
