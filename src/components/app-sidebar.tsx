import { NavMain } from "@/components/nav-main";
import { NavSecondary } from "@/components/nav-secondary";
import { NavUser } from "@/components/nav-user";
import {
	Sidebar,
	SidebarContent,
	SidebarFooter,
	SidebarHeader,
	SidebarMenu,
	SidebarMenuButton,
	SidebarMenuItem,
} from "@/components/ui/sidebar";
import { Link } from "@tanstack/react-router";
import {
	CameraIcon,
	ChartBarIcon,
	CommandIcon,
	FileTextIcon,
	QrCodeIcon,
	LayoutDashboardIcon,
	ListIcon,
	Settings2Icon,
	GitCompareArrows,
} from "lucide-react";
import type * as React from "react";

const data = {
	navMain: [
		{
			title: "Dashboard",
			url: "/",
			icon: <LayoutDashboardIcon />,
		},
		{
			title: "Components",
			url: "/components",
			icon: <ListIcon />,
		},
		{
			title: "Models",
			url: "/models",
			icon: <CommandIcon />,
		},
		{
			title: "Backtest",
			url: "/backtest",
			icon: <ChartBarIcon />,
		},
		{
			title: "Validation",
			url: "/validation",
			icon: <FileTextIcon />,
		},
	],
	navClouds: [
		{
			title: "Capture",
			icon: <CameraIcon />,
			isActive: true,
			url: "#",
			items: [
				{
					title: "Active Proposals",
					url: "#",
				},
				{
					title: "Archived",
					url: "#",
				},
			],
		},
		{
			title: "Proposal",
			icon: <FileTextIcon />,
			url: "#",
			items: [
				{
					title: "Active Proposals",
					url: "#",
				},
				{
					title: "Archived",
					url: "#",
				},
			],
		},
		{
			title: "Prompts",
			icon: <FileTextIcon />,
			url: "#",
			items: [
				{
					title: "Active Proposals",
					url: "#",
				},
				{
					title: "Archived",
					url: "#",
				},
			],
		},
	],
	navSecondary: [
		{
			title: "Settings",
			url: "/settings/general",
			icon: <Settings2Icon />,
		},
		{
			title: "GitHub",
			url: "https://github.com/tonywxx/adaq",
			icon: <GitCompareArrows />,
		},
		{
			title: "WeChat",
			url: "https://mp.weixin.qq.com/s/fHFDyntJ7PRwrsRJqfLnaA", // https://weixin.qq.com/r/mp/TC_Pl63E2vClren093pe
			icon: <QrCodeIcon />,
		},
	],
};

export function AppSidebar({ ...props }: React.ComponentProps<typeof Sidebar>) {
	return (
		<Sidebar collapsible="offcanvas" {...props}>
			<SidebarHeader>
				<SidebarMenu>
					<SidebarMenuItem>
						<SidebarMenuButton
							asChild
							className="data-[slot=sidebar-menu-button]:p-1.5!"
						>
							<Link to="/">
								<CommandIcon className="size-5!" />
								<span className="text-base font-semibold">AdaQ</span>
							</Link>
						</SidebarMenuButton>
					</SidebarMenuItem>
				</SidebarMenu>
			</SidebarHeader>
			<SidebarContent>
				<NavMain items={data.navMain} />
				{/* <NavDocuments items={data.documents} /> */}
				<NavSecondary items={data.navSecondary} className="mt-auto" />
			</SidebarContent>
			<SidebarFooter>
				<NavUser />
			</SidebarFooter>
		</Sidebar>
	);
}
