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
} from "@/components/ui/sidebar";
import { Link, useLocation } from "@tanstack/react-router";
import {
	CandlestickChart,
	BotIcon,
	ChartBarIcon,
	CircleHelpIcon,
	DatabaseIcon,
	FileTextIcon,
	GitCompareArrows,
	LayoutDashboardIcon,
	ListIcon,
	QrCodeIcon,
	Settings2Icon,
	SigmaIcon,
} from "lucide-react";
import type { ComponentProps, ReactNode } from "react";
import { useTranslation } from "react-i18next";

const secondaryItems = [
	{
		titleKey: "nav.settings",
		url: "/settings/general",
		icon: <Settings2Icon />,
	},
	{
		titleKey: "nav.help",
		url: "/help/workflow",
		icon: <CircleHelpIcon />,
	},
	{
		titleKey: "nav.github",
		url: "https://github.com/tonywxx/adaq",
		icon: <GitCompareArrows />,
	},
	{
		titleKey: "nav.wechat",
		url: "https://mp.weixin.qq.com/s/fHFDyntJ7PRwrsRJqfLnaA",
		icon: <QrCodeIcon />,
	},
];

export function AppSidebar({ ...props }: ComponentProps<typeof Sidebar>) {
	const { t } = useTranslation();
	const location = useLocation();

	const navSecondary = secondaryItems.map((item) => ({
		...item,
		title: t(item.titleKey),
		active:
			item.url.startsWith("/") &&
			location.pathname.startsWith(
				item.url === "/settings/general" ? "/settings" : item.url,
			),
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
								<CandlestickChart className="size-5!" aria-hidden="true" />
								<span className="text-base font-semibold">AdaQ</span>
							</Link>
						</SidebarMenuButton>
					</SidebarMenuItem>
				</SidebarMenu>
			</SidebarHeader>
			<SidebarContent>
				<SidebarGroup>
					<SidebarGroupContent>
						<SidebarMenu>
							<SidebarLink
								to="/"
								label={t("nav.home")}
								icon={<LayoutDashboardIcon aria-hidden="true" />}
								active={location.pathname === "/"}
							/>
						</SidebarMenu>
					</SidebarGroupContent>
				</SidebarGroup>

				<SidebarGroup>
					<SidebarGroupLabel>{t("nav.foundations")}</SidebarGroupLabel>
					<SidebarGroupContent>
						<SidebarMenu>
							<SidebarLink
								to="/data-foundation"
								label={t("nav.dataFoundation")}
								icon={<DatabaseIcon aria-hidden="true" />}
								active={location.pathname === "/data-foundation"}
							/>
							<SidebarLink
								to="/markets"
								label={t("nav.marketsData")}
								icon={<CandlestickChart aria-hidden="true" />}
								active={location.pathname.startsWith("/markets")}
							/>
							<SidebarLink
								to="/features"
								label={t("nav.featureEngineering")}
								icon={<SigmaIcon aria-hidden="true" />}
								active={location.pathname === "/features"}
							/>
						</SidebarMenu>
					</SidebarGroupContent>
				</SidebarGroup>

				<SidebarGroup>
					<SidebarGroupLabel>{t("nav.research")}</SidebarGroupLabel>
					<SidebarGroupContent>
						<SidebarMenu>
							<SidebarLink
								to="/factors"
								label={t("nav.factorResearch")}
								icon={<SigmaIcon aria-hidden="true" />}
								active={location.pathname === "/factors"}
							/>
							<SidebarLink
								to="/models"
								label={t("nav.modelResearch")}
								icon={<FileTextIcon aria-hidden="true" />}
								active={location.pathname === "/models"}
							/>
						</SidebarMenu>
					</SidebarGroupContent>
				</SidebarGroup>

				<SidebarGroup>
					<SidebarGroupLabel>{t("nav.simulationValidation")}</SidebarGroupLabel>
					<SidebarGroupContent>
						<SidebarMenu>
							<SidebarLink
								to="/strategies"
								label={t("nav.strategyLab")}
								icon={<FileTextIcon aria-hidden="true" />}
								active={location.pathname === "/strategies"}
							/>
							<SidebarLink
								to="/backtest"
								label={t("nav.backtest")}
								icon={<ChartBarIcon aria-hidden="true" />}
								active={location.pathname === "/backtest"}
							/>
							<SidebarLink
								to="/validation"
								label={t("nav.validation")}
								icon={<FileTextIcon aria-hidden="true" />}
								active={location.pathname === "/validation"}
							/>
						</SidebarMenu>
					</SidebarGroupContent>
				</SidebarGroup>

				<SidebarGroup>
					<SidebarGroupLabel>{t("nav.library")}</SidebarGroupLabel>
					<SidebarGroupContent>
						<SidebarMenu>
							<SidebarLink
								to="/components"
								label={t("nav.componentLibrary")}
								icon={<ListIcon aria-hidden="true" />}
								active={location.pathname === "/components"}
							/>
						</SidebarMenu>
					</SidebarGroupContent>
				</SidebarGroup>

				<SidebarGroup>
					<SidebarGroupLabel>{t("nav.paperOperations")}</SidebarGroupLabel>
					<SidebarGroupContent>
						<SidebarMenu>
							<SidebarLink
								to="/operations"
								label={t("nav.operationsDashboard")}
								icon={<ChartBarIcon aria-hidden="true" />}
								active={location.pathname === "/operations"}
							/>
							<SidebarLink
								to="/paper-trading"
								label={t("nav.paperTrading")}
								icon={<ChartBarIcon aria-hidden="true" />}
								active={location.pathname === "/paper-trading"}
							/>
							<SidebarLink
								to="/bots"
								label={t("nav.bots")}
								icon={<BotIcon aria-hidden="true" />}
								active={location.pathname === "/bots"}
							/>
							<SidebarLink
								to="/paper-feedback"
								label={t("nav.paperFeedback")}
								icon={<FileTextIcon aria-hidden="true" />}
								active={location.pathname === "/paper-feedback"}
							/>
						</SidebarMenu>
					</SidebarGroupContent>
				</SidebarGroup>

				<NavSecondary items={navSecondary} className="mt-auto" />
			</SidebarContent>
			<SidebarFooter>
				<NavUser />
			</SidebarFooter>
		</Sidebar>
	);
}

function SidebarLink({
	to,
	label,
	icon,
	active,
}: {
	to:
		| "/"
		| "/markets"
		| "/data-foundation"
		| "/features"
		| "/factors"
		| "/models"
		| "/strategies"
		| "/backtest"
		| "/validation"
		| "/operations"
		| "/paper-trading"
		| "/bots"
		| "/paper-feedback"
		| "/components";
	label: string;
	icon: ReactNode;
	active: boolean;
}) {
	return (
		<SidebarMenuItem>
			<SidebarMenuButton asChild isActive={active} tooltip={label}>
				<Link to={to}>
					{icon}
					<span>{label}</span>
				</Link>
			</SidebarMenuButton>
		</SidebarMenuItem>
	);
}
