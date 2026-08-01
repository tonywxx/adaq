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
import Home, { Dashboard } from "@/layout/home";
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
const ModelsPage = lazy(() =>
	import("@/features/models/models-page").then((module) => ({
		default: module.ModelsPage,
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
	component: Dashboard,
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
	component: () => (
		<Suspense fallback={<PageLoadingSkeleton />}>
			<ModelsPage />
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
