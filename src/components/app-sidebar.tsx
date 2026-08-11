import { NavMain } from "@/components/nav-main";
import { NavSecondary } from "@/components/nav-secondary";
import { NavUser } from "@/components/nav-user";
import {
	Sidebar,
	SidebarContent,
	SidebarFooter,
	SidebarGroup,
	SidebarGroupContent,
	SidebarGroupLabel,
	SidebarHeader,
	SidebarMenu,
	SidebarMenuButton,
	SidebarMenuItem,
	SidebarMenuSub,
	SidebarMenuSubButton,
	SidebarMenuSubItem,
} from "@/components/ui/sidebar";
import { Link, useLocation } from "@tanstack/react-router";
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
	SigmaIcon,
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
		{
			titleKey: "nav.features",
			url: "/features",
			icon: <SigmaIcon />,
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
	const location = useLocation();
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
				<SidebarGroup>
					<SidebarGroupLabel>{t("nav.markets")}</SidebarGroupLabel>
					<SidebarGroupContent>
						<SidebarMenu>
							<SidebarMenuItem>
								<SidebarMenuButton
									asChild
									isActive={location.pathname.startsWith("/markets")}
									tooltip={t("nav.markets")}
								>
									<Link to="/markets">
										<CandlestickChart aria-hidden="true" />
										<span>{t("nav.markets")}</span>
									</Link>
								</SidebarMenuButton>
								<SidebarMenuSub>
									<SidebarMenuSubItem>
										<SidebarMenuSubButton
											asChild
											isActive={location.pathname === "/markets"}
										>
											<Link to="/markets">{t("nav.marketsOverview")}</Link>
										</SidebarMenuSubButton>
									</SidebarMenuSubItem>
									<SidebarMenuSubItem>
										<SidebarMenuSubButton
											asChild
											isActive={location.pathname === "/markets/crypto"}
										>
											<Link to="/markets/crypto">{t("nav.crypto")}</Link>
										</SidebarMenuSubButton>
									</SidebarMenuSubItem>
									<SidebarMenuSubItem>
										<SidebarMenuSubButton
											asChild
											isActive={location.pathname === "/markets/a-shares"}
										>
											<Link to="/markets/a-shares">{t("nav.aShares")}</Link>
										</SidebarMenuSubButton>
									</SidebarMenuSubItem>
									<SidebarMenuSubItem>
										<SidebarMenuSubButton
											asChild
											isActive={location.pathname === "/markets/us-equities"}
										>
											<Link to="/markets/us-equities">{t("nav.usEquities")}</Link>
										</SidebarMenuSubButton>
									</SidebarMenuSubItem>
								</SidebarMenuSub>
							</SidebarMenuItem>
						</SidebarMenu>
					</SidebarGroupContent>
				</SidebarGroup>
				{/* <NavDocuments items={data.documents} /> */}
				<NavSecondary items={navSecondary} className="mt-auto" />
			</SidebarContent>
			<SidebarFooter>
				<NavUser />
			</SidebarFooter>
		</Sidebar>
	);
}
