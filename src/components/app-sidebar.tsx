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
	CandlestickChart,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import type * as React from "react";

const data = {
	navMain: [
		{
			titleKey: "nav.dashboard",
			url: "/",
			icon: <LayoutDashboardIcon />,
		},
		{
			titleKey: "nav.components",
			url: "/components",
			icon: <ListIcon />,
		},
		{
			titleKey: "nav.models",
			url: "/models",
			icon: <CommandIcon />,
		},
		{
			titleKey: "nav.backtest",
			url: "/backtest",
			icon: <ChartBarIcon />,
		},
		{
			titleKey: "nav.validation",
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
			titleKey: "nav.settings",
			url: "/settings/general",
			icon: <Settings2Icon />,
		},
		{
			titleKey: "nav.github",
			url: "https://github.com/tonywxx/adaq",
			icon: <GitCompareArrows />,
		},
		{
			titleKey: "nav.wechat",
			url: "https://mp.weixin.qq.com/s/fHFDyntJ7PRwrsRJqfLnaA", // https://weixin.qq.com/r/mp/TC_Pl63E2vClren093pe
			icon: <QrCodeIcon />,
		},
	],
};

export function AppSidebar({ ...props }: React.ComponentProps<typeof Sidebar>) {
	const { t } = useTranslation();
	const navMain = data.navMain.map((item) => ({
		...item,
		title: t(item.titleKey),
	}));
	const navSecondary = data.navSecondary.map((item) => ({
		...item,
		title: t(item.titleKey),
	}));

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
								<CandlestickChart className="size-5!" />
								<span className="text-base font-semibold">AdaQ</span>
							</Link>
						</SidebarMenuButton>
					</SidebarMenuItem>
				</SidebarMenu>
			</SidebarHeader>
			<SidebarContent>
				<NavMain items={navMain} />
				{/* <NavDocuments items={data.documents} /> */}
				<NavSecondary items={navSecondary} className="mt-auto" />
			</SidebarContent>
			<SidebarFooter>
				<NavUser />
			</SidebarFooter>
		</Sidebar>
	);
}
