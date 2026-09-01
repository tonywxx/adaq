import {
	createRootRoute,
	createRoute,
	createRouter,
	Navigate,
	Outlet,
	useRouterState,
} from "@tanstack/react-router";
import { useAuthenticatedUserId } from "@/authenticated-user";
import { CriticalOperationalBanner } from "@/components/critical-operational-banner";
import { PageLoadingSkeleton } from "@/components/page-loading-skeleton";
import { WorkspaceReadyBoundary } from "@/components/workspace-ready-boundary";
import {
	WorkflowGuidePage,
	WorkflowHomePage,
} from "@/features/workflow/workflow-page";
import Home from "@/layout/home";
import { lazy, Suspense, useEffect } from "react";
import { LAST_APP_PATH_KEY } from "@/lib/app-settings";

const DataFoundationPage = lazy(() =>
	import("@/features/data-foundation/data-foundation-page").then((module) => ({
		default: module.DataFoundationPage,
	})),
);
const OperationsDashboardPage = lazy(() =>
	import("@/features/operations/operations-dashboard").then((module) => ({
		default: module.OperationsDashboard,
	})),
);
const PaperTradingPage = lazy(() =>
	import("@/features/paper-trading/paper-trading-page").then((module) => ({
		default: module.PaperTradingPage,
	})),
);
const BotsPage = lazy(() =>
	import("@/features/bots/bots-page").then((module) => ({
		default: module.BotsPage,
	})),
);
const PaperFeedbackPage = lazy(() =>
	import("@/features/paper-feedback/paper-feedback-page").then((module) => ({
		default: module.PaperFeedbackPage,
	})),
);
const MarketsOverviewPage = lazy(() =>
	import("@/features/markets/markets-page").then((module) => ({
		default: module.MarketsOverview,
	})),
);
const CryptoMarketPage = lazy(() =>
	import("@/features/markets/markets-page").then((module) => ({
		default: module.CryptoMarketPage,
	})),
);
const MarketSessionBoundary = lazy(() =>
	import("@/components/market-session-boundary").then((module) => ({
		default: module.MarketSessionBoundary,
	})),
);

const BacktestPage = lazy(() =>
	import("@/features/backtest/backtest-page").then((module) => ({
		default: module.BacktestPage,
	})),
);
const ComponentsPage = lazy(() =>
	import("@/features/components/components-page").then((module) => ({
		default: module.ComponentsPage,
	})),
);
const ValidationPage = lazy(() =>
	import("@/features/validation/validation-page").then((module) => ({
		default: module.ValidationPage,
	})),
);
const FeaturesPage = lazy(() =>
	import("@/features/features/features-page").then((module) => ({
		default: module.FeaturesPage,
	})),
);
const ModelsPage = lazy(() =>
	import("@/features/models/models-page").then((module) => ({
		default: module.ModelsPage,
	})),
);
const FactorsPage = lazy(() =>
	import("@/features/factors/factors-page").then((module) => ({
		default: module.FactorsPage,
	})),
);
const StrategyLabPage = lazy(() =>
	import("@/features/strategy/strategy-lab-page").then((module) => ({
		default: module.StrategyLabPage,
	})),
);
const SettingsPage = lazy(() =>
	import("@/features/settings/settings-page").then((module) => ({
		default: module.SettingsPage,
	})),
);

const rootRoute = createRootRoute({
	component: AppShell,
});

const appRoute = createRoute({
	getParentRoute: () => rootRoute,
	path: "/",
	component: WorkflowHomePage,
});

const operationsRoute = createRoute({
	getParentRoute: () => rootRoute,
	path: "/operations",
	component: () => (
		<Suspense fallback={<PageLoadingSkeleton />}>
			<OperationsDashboardPage />
		</Suspense>
	),
});

const paperTradingRoute = createRoute({
	getParentRoute: () => rootRoute,
	path: "/paper-trading",
	component: () => (
		<Suspense fallback={<PageLoadingSkeleton />}>
			<PaperTradingPage />
		</Suspense>
	),
});

const botsRoute = createRoute({
	getParentRoute: () => rootRoute,
	path: "/bots",
	component: () => (
		<Suspense fallback={<PageLoadingSkeleton />}>
			<BotsPage />
		</Suspense>
	),
});

const paperFeedbackRoute = createRoute({
	getParentRoute: () => rootRoute,
	path: "/paper-feedback",
	component: () => (
		<Suspense fallback={<PageLoadingSkeleton />}>
			<PaperFeedbackPage />
		</Suspense>
	),
});

const workflowGuideRoute = createRoute({
	getParentRoute: () => rootRoute,
	path: "/help/workflow",
	component: WorkflowGuidePage,
});

const workflowStepRoute = createRoute({
	getParentRoute: () => rootRoute,
	path: "/help/workflow/$step",
	component: WorkflowStepGuidePage,
});

const dataFoundationRoute = createRoute({
	getParentRoute: () => rootRoute,
	path: "/data-foundation",
	component: () => (
		<Suspense fallback={<PageLoadingSkeleton />}>
			<DataFoundationPage />
		</Suspense>
	),
});

const marketsRoute = createRoute({
	getParentRoute: () => rootRoute,
	path: "/markets",
	component: () => (
		<Suspense fallback={<PageLoadingSkeleton />}>
			<MarketsOverviewPage />
		</Suspense>
	),
});

const cryptoMarketRoute = createRoute({
	getParentRoute: () => rootRoute,
	path: "/markets/crypto",
	component: () => (
		<Suspense fallback={<PageLoadingSkeleton />}>
			<CryptoMarketPage />
		</Suspense>
	),
});

const backtestRoute = createRoute({
	getParentRoute: () => rootRoute,
	path: "/backtest",
	component: () => (
		<Suspense fallback={<PageLoadingSkeleton />}>
			<BacktestPage />
		</Suspense>
	),
});
const componentsRoute = createRoute({
	getParentRoute: () => rootRoute,
	path: "/components",
	component: () => (
		<Suspense fallback={<PageLoadingSkeleton />}>
			<ComponentsPage />
		</Suspense>
	),
});
const validationRoute = createRoute({
	getParentRoute: () => rootRoute,
	path: "/validation",
	component: () => (
		<Suspense fallback={<PageLoadingSkeleton />}>
			<ValidationPage />
		</Suspense>
	),
});
const featuresRoute = createRoute({
	getParentRoute: () => rootRoute,
	path: "/features",
	component: () => (
		<Suspense fallback={<PageLoadingSkeleton />}>
			<FeaturesPage />
		</Suspense>
	),
});
const modelsRoute = createRoute({
	getParentRoute: () => rootRoute,
	path: "/models",
	component: () => (
		<Suspense fallback={<PageLoadingSkeleton />}>
			<ModelsPage />
		</Suspense>
	),
});
const factorsRoute = createRoute({
	getParentRoute: () => rootRoute,
	path: "/factors",
	staticData: {
		titleKey: "factors.title",
		breadcrumbKey: "factors.breadcrumb",
	},
	component: () => (
		<Suspense fallback={<PageLoadingSkeleton />}>
			<FactorsPage />
		</Suspense>
	),
});
const strategiesRoute = createRoute({
	getParentRoute: () => rootRoute,
	path: "/strategies",
	staticData: {
		titleKey: "strategyLab.title",
		breadcrumbKey: "strategyLab.breadcrumb",
	},
	component: () => (
		<Suspense fallback={<PageLoadingSkeleton />}>
			<StrategyLabPage />
		</Suspense>
	),
});
const settingsIndexRoute = createRoute({
	getParentRoute: () => rootRoute,
	path: "/settings",
	component: () => (
		<Navigate to="/settings/$section" params={{ section: "general" }} replace />
	),
});
const settingsRoute = createRoute({
	getParentRoute: () => rootRoute,
	path: "/settings/$section",
	component: () => (
		<Suspense fallback={<PageLoadingSkeleton />}>
			<SettingsPage />
		</Suspense>
	),
});

const MARKET_SESSION_PATHS = [
	"/data-foundation",
	"/operations",
	"/markets",
	"/backtest",
	"/components",
	"/validation",
	"/features",
	"/models",
	"/factors",
	"/strategies",
] as const;

function usesMarketSession(pathname: string) {
	return MARKET_SESSION_PATHS.some(
		(path) => pathname === path || pathname.startsWith(`${path}/`),
	);
}

function AppShell() {
	const userId = useAuthenticatedUserId();
	const href = useRouterState({ select: (state) => state.location.href });
	const pathname = useRouterState({
		select: (state) => state.location.pathname,
	});
	const isSettings = pathname.startsWith("/settings");
	const isHelp = pathname.startsWith("/help/workflow");
	const needsMarketSession = usesMarketSession(pathname);
	const isCryptoMarket =
		pathname === "/markets/crypto" || pathname.startsWith("/markets/crypto/");

	useEffect(() => {
		if (!isSettings) sessionStorage.setItem(LAST_APP_PATH_KEY, href);
	}, [href, isSettings]);

	const content = needsMarketSession ? (
		<Suspense fallback={<PageLoadingSkeleton />}>
			<MarketSessionBoundary userId={userId} realtime={isCryptoMarket}>
				<Outlet />
			</MarketSessionBoundary>
		</Suspense>
	) : (
		<Outlet />
	);

	return (
		<Home showSidebar={!isSettings}>
			{isHelp ? (
				content
			) : (
				<WorkspaceReadyBoundary>
					<CriticalOperationalBanner />
					{content}
				</WorkspaceReadyBoundary>
			)}
		</Home>
	);
}

function WorkflowStepGuidePage() {
	const { step } = workflowStepRoute.useParams();
	return <WorkflowGuidePage selectedStepId={Number(step)} />;
}

const routeTree = rootRoute.addChildren([
	appRoute,
	operationsRoute,
	paperTradingRoute,
	botsRoute,
	paperFeedbackRoute,
	workflowGuideRoute,
	workflowStepRoute,
	dataFoundationRoute,
	marketsRoute,
	cryptoMarketRoute,
	backtestRoute,
	componentsRoute,
	validationRoute,
	featuresRoute,
	modelsRoute,
	factorsRoute,
	strategiesRoute,
	settingsIndexRoute,
	settingsRoute,
]);

export const router = createRouter({ routeTree });

declare module "@tanstack/react-router" {
	interface Register {
		router: typeof router;
	}
}
