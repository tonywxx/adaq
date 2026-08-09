import {
	createRootRoute,
	createRoute,
	createRouter,
	Navigate,
	Outlet,
	useRouterState,
} from "@tanstack/react-router";
import { AuthGate } from "@/components/auth-gate";
import { PageLoadingSkeleton } from "@/components/page-loading-skeleton";
import { ModelsPage } from "@/features/models/models-page";
import {
	CryptoMarketPage,
	MarketWorkspacePage,
	MarketsOverview,
	OperationsDashboard,
} from "@/features/markets/markets-page";
import Home from "@/layout/home";
import { lazy, Suspense, useEffect } from "react";
import { LAST_APP_PATH_KEY } from "@/lib/app-settings";

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
	component: OperationsDashboard,
});

const marketsRoute = createRoute({
	getParentRoute: () => rootRoute,
	path: "/markets",
	component: MarketsOverview,
});

const cryptoMarketRoute = createRoute({
	getParentRoute: () => rootRoute,
	path: "/markets/crypto",
	component: CryptoMarketPage,
});

const ashareMarketRoute = createRoute({
	getParentRoute: () => rootRoute,
	path: "/markets/a-shares",
	component: () => <MarketWorkspacePage market="a-shares" />,
});

const usEquitiesMarketRoute = createRoute({
	getParentRoute: () => rootRoute,
	path: "/markets/us-equities",
	component: () => <MarketWorkspacePage market="us-equities" />,
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
const modelsRoute = createRoute({
	getParentRoute: () => rootRoute,
	path: "/models",
	component: ModelsPage,
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

function AppShell() {
	const href = useRouterState({ select: (state) => state.location.href });
	const isSettings = href.startsWith("/settings");

	useEffect(() => {
		if (!isSettings) sessionStorage.setItem(LAST_APP_PATH_KEY, href);
	}, [href, isSettings]);

	return (
		<AuthGate>
			<Home showSidebar={!isSettings}>
				<Outlet />
			</Home>
		</AuthGate>
	);
}

const routeTree = rootRoute.addChildren([
	appRoute,
	marketsRoute,
	cryptoMarketRoute,
	ashareMarketRoute,
	usEquitiesMarketRoute,
	backtestRoute,
	componentsRoute,
	validationRoute,
	modelsRoute,
	settingsIndexRoute,
	settingsRoute,
]);

export const router = createRouter({ routeTree });

declare module "@tanstack/react-router" {
	interface Register {
		router: typeof router;
	}
}
