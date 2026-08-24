import { AppSidebar } from "@/components/app-sidebar";
import { AppTitlebar } from "@/components/app-titlebar";
import { SidebarInset, SidebarProvider } from "@/components/ui/sidebar";
import { Toaster } from "@/components/ui/sonner";
import type { ReactNode } from "react";

export default function Home({
	children,
	showSidebar = true,
}: {
	children: ReactNode;
	showSidebar?: boolean;
}) {
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
			<AppTitlebar showSidebarTrigger={showSidebar} />
			<Toaster />

			{showSidebar ? (
				<AppSidebar className="top-14 h-[calc(100svh-3.5rem)]" variant="inset" />
			) : null}
			<SidebarInset
				className={`m-0! min-h-0 overflow-y-auto rounded-none! shadow-none! ${showSidebar ? "border-l border-border" : ""}`}
			>
				{children}
			</SidebarInset>
		</SidebarProvider>
	);
}
