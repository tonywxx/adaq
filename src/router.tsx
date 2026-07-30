import {
	createRootRoute,
	createRoute,
	createRouter,
	Outlet,
} from "@tanstack/react-router";
import { AuthGate } from "@/components/auth-gate";
import Home from "@/layout/home";
import { BacktestPage } from "@/features/backtest/backtest-page";
import { ComponentsPage } from "@/features/components/components-page";
import { ValidationPage } from "@/features/validation/validation-page";

const rootRoute = createRootRoute({
	component: Outlet,
});

const appRoute = createRoute({
	getParentRoute: () => rootRoute,
	path: "/",
	component: AppRoute,
});

const backtestRoute = createRoute({
	getParentRoute: () => rootRoute,
	path: "/backtest",
	component: () => (
		<AuthenticatedPage>
			<BacktestPage />
		</AuthenticatedPage>
	),
});
const componentsRoute = createRoute({
	getParentRoute: () => rootRoute,
	path: "/components",
	component: () => (
		<AuthenticatedPage>
			<ComponentsPage />
		</AuthenticatedPage>
	),
});
const validationRoute = createRoute({
	getParentRoute: () => rootRoute,
	path: "/validation",
	component: () => (
		<AuthenticatedPage>
			<ValidationPage />
		</AuthenticatedPage>
	),
});

function AppRoute() {
	return (
		<AuthGate>
			<Home />
		</AuthGate>
	);
}

function AuthenticatedPage({ children }: { children: React.ReactNode }) {
	return (
		<AuthGate>
			<Home>{children}</Home>
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
