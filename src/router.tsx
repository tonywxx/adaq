import {
	createRootRoute,
	createRoute,
	createRouter,
	Outlet,
} from "@tanstack/react-router";
import { AuthGate } from "@/components/auth-gate";
import { PageLoadingSkeleton } from "@/components/page-loading-skeleton";
import Home, { Dashboard } from "@/layout/home";
import { lazy, Suspense } from "react";

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

function AppShell() {
	return (
		<AuthGate>
			<Home>
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
]);

export const router = createRouter({ routeTree });

declare module "@tanstack/react-router" {
	interface Register {
		router: typeof router;
	}
}
