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
import { workflowModules, workflowSteps } from "@/features/workflow/workflow";
import { Link, useLocation } from "@tanstack/react-router";
import {
	CandlestickChart,
	BotIcon,
	ChartBarIcon,
	DatabaseIcon,
	ChevronDownIcon,
	CircleHelpIcon,
	CommandIcon,
	FileTextIcon,
	GitCompareArrows,
	LayoutDashboardIcon,
	ListIcon,
	QrCodeIcon,
	Settings2Icon,
	SigmaIcon,
} from "lucide-react";
import {
	useEffect,
	useState,
	type ComponentProps,
	type ReactNode,
} from "react";
import { useTranslation } from "react-i18next";

const moduleIcons: Record<(typeof workflowModules)[number]["id"], ReactNode> = {
	factor: <SigmaIcon aria-hidden="true" />,
	model: <CommandIcon aria-hidden="true" />,
	strategy: <FileTextIcon aria-hidden="true" />,
	operations: <ChartBarIcon aria-hidden="true" />,
};

const secondaryItems = [
	{
		titleKey: "nav.help",
		url: "/help/workflow",
		icon: <CircleHelpIcon />,
	},
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
		url: "https://mp.weixin.qq.com/s/fHFDyntJ7PRwrsRJqfLnaA",
		icon: <QrCodeIcon />,
	},
];

export function AppSidebar({ ...props }: ComponentProps<typeof Sidebar>) {
	const { t } = useTranslation();
	const location = useLocation();
	const activeStep = workflowSteps.find(
		(step) => location.pathname === `/help/workflow/${step.id}`,
	);
	const activeModule =
		location.pathname === "/operations" ||
		location.pathname === "/paper-trading" ||
		location.pathname === "/bots"
			? "operations"
			: location.pathname === "/factors"
				? "factor"
				: location.pathname === "/strategies"
					? "strategy"
					: activeStep?.module;
	const [openModules, setOpenModules] = useState<Set<string>>(
		() => new Set([activeModule ?? "factor"]),
	);

	useEffect(() => {
		if (!activeModule) return;
		setOpenModules((current) => {
			if (current.has(activeModule)) return current;
			return new Set([...current, activeModule]);
		});
	}, [activeModule]);

	const navSecondary = secondaryItems.map((item) => ({
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

				{workflowModules.map((module) => {
					const open = openModules.has(module.id);
					return (
						<SidebarGroup key={module.id}>
							<SidebarGroupLabel asChild>
								<button
									type="button"
									className="w-full justify-between"
									aria-expanded={open}
									aria-controls={`sidebar-workflow-${module.id}`}
									onClick={() =>
										setOpenModules((current) => {
											const next = new Set(current);
											if (open) next.delete(module.id);
											else next.add(module.id);
											return next;
										})
									}
								>
									<span>{t(`workflow.modules.${module.id}.title`)}</span>
									<span className="flex items-center gap-1 font-mono text-[10px]">
										{module.steps}
										<ChevronDownIcon
											className={`transition-transform ${open ? "rotate-180" : ""}`}
											aria-hidden="true"
										/>
									</span>
								</button>
							</SidebarGroupLabel>
							{open ? (
								<SidebarGroupContent id={`sidebar-workflow-${module.id}`}>
									<SidebarMenu>
										{module.id === "factor" ? (
											<SidebarLink
												to="/factors"
												label={t("nav.factorResearch")}
												icon={<SigmaIcon aria-hidden="true" />}
												active={location.pathname === "/factors"}
											/>
										) : null}
										{module.id === "operations" ? (
											<>
												<SidebarLink
													to="/operations"
													label={t("nav.operationsDashboard")}
													icon={moduleIcons[module.id]}
													active={location.pathname === "/operations"}
												/>
												<SidebarLink
													to="/paper-trading"
													label={t("nav.paperTrading")}
													icon={moduleIcons[module.id]}
													active={location.pathname === "/paper-trading"}
												/>
												<SidebarLink
													to="/bots"
													label={t("nav.bots")}
													icon={<BotIcon aria-hidden="true" />}
													active={location.pathname === "/bots"}
												/>
											</>
										) : null}
										{module.id === "strategy" ? (
											<SidebarLink
												to="/strategies"
												label={t("nav.strategyLab")}
												icon={<FileTextIcon aria-hidden="true" />}
												active={location.pathname === "/strategies"}
											/>
										) : null}
										<SidebarMenuItem>
											<div className="flex h-8 items-center gap-2 rounded-md px-2 text-sm text-sidebar-foreground/80 [&>svg]:size-4">
												{moduleIcons[module.id]}
												<span>{t("nav.workflowSteps")}</span>
											</div>
											<SidebarMenuSub>
												{workflowSteps
													.filter((step) => step.module === module.id)
													.map((step) => (
														<SidebarMenuSubItem key={step.id}>
															<SidebarMenuSubButton
																asChild
																isActive={location.pathname === `/help/workflow/${step.id}`}
															>
																<Link
																	to="/help/workflow/$step"
																	params={{ step: String(step.id) }}
																>
																	<span className="font-mono text-[10px] text-muted-foreground">
																		{step.id}
																	</span>
																	<span>{t(`workflow.steps.${step.id}.shortTitle`)}</span>
																</Link>
															</SidebarMenuSubButton>
														</SidebarMenuSubItem>
													))}
											</SidebarMenuSub>
										</SidebarMenuItem>
									</SidebarMenu>
								</SidebarGroupContent>
							) : null}
						</SidebarGroup>
					);
				})}

				<SidebarGroup>
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
		| "/strategies"
		| "/operations"
		| "/paper-trading"
		| "/bots"
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
