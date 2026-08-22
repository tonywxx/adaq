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
import { DataFoundationPage } from "@/features/data-foundation/data-foundation-page";
import {
	CryptoMarketPage,
	MarketsOverview,
	OperationsDashboard,
} from "@/features/markets/markets-page";
import {
	WorkflowGuidePage,
	WorkflowHomePage,
} from "@/features/workflow/workflow-page";
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
	component: OperationsDashboard,
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
	component: DataFoundationPage,
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

function WorkflowStepGuidePage() {
	const { step } = workflowStepRoute.useParams();
	return <WorkflowGuidePage selectedStepId={Number(step)} />;
}

const routeTree = rootRoute.addChildren([
	appRoute,
	operationsRoute,
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
	settingsIndexRoute,
	settingsRoute,
]);

export const router = createRouter({ routeTree });

declare module "@tanstack/react-router" {
	interface Register {
		router: typeof router;
	}
}
