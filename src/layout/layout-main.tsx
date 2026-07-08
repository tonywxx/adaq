import { AppSidebar } from "@/components/app-sidebar";
import { AppTitlebar } from "@/components/app-titlebar";
import { ChartAreaInteractive } from "@/components/chart-area-interactive";
import { DataTable } from "@/components/data-table";
import { SectionCards } from "@/components/section-cards";
import {
	Card,
	CardContent,
	CardHeader,
	CardTitle,
} from "@/components/ui/card";
import { SidebarInset, SidebarProvider } from "@/components/ui/sidebar";
import { Toaster } from "@/components/ui/sonner";
import { useActiveInstStore } from "@/lib/active-inst-store";

// import data from "./data.json";

export default function LayoutMain() {
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
							<SectionCards />
							<ActiveInstrumentCard />
							<div className="px-4 lg:px-6">
								<ChartAreaInteractive />
							</div>
							<DataTable data={[]} />
						</div>
					</div>
				</div>
			</SidebarInset>
		</SidebarProvider>
	);
}

function ActiveInstrumentCard() {
	const activeInst = useActiveInstStore((state) => state.activeInst);

	return (
		<div className="px-4 lg:px-6">
			<Card>
				<CardHeader>
					<CardTitle>Active Instrument</CardTitle>
				</CardHeader>
				<CardContent>
					{activeInst ? (
						<dl className="grid gap-3 text-sm sm:grid-cols-2 lg:grid-cols-4">
							<InstrumentField label="Code" value={activeInst.code} />
							<InstrumentField label="Name" value={activeInst.name} />
							<InstrumentField label="Market" value={activeInst.market} />
							<InstrumentField label="Pinyin" value={activeInst.pinyin || "-"} />
						</dl>
					) : (
						<div className="text-sm text-muted-foreground">
							No instrument selected.
						</div>
					)}
				</CardContent>
			</Card>
		</div>
	);
}

function InstrumentField({ label, value }: { label: string; value: string }) {
	return (
		<div className="min-w-0">
			<dt className="text-xs text-muted-foreground">{label}</dt>
			<dd className="truncate font-medium">{value}</dd>
		</div>
	);
}
