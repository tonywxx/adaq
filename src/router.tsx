import {
	createRootRoute,
	createRoute,
	createRouter,
	Outlet,
} from "@tanstack/react-router";
import { AuthGate } from "@/components/auth-gate";
import Home from "@/layout/home";

const rootRoute = createRootRoute({
	component: Outlet,
});

const appRoute = createRoute({
	getParentRoute: () => rootRoute,
	path: "/",
	component: AppRoute,
});

function AppRoute() {
	return (
		<AuthGate>
			<Home />
		</AuthGate>
	);
}

const routeTree = rootRoute.addChildren([appRoute]);

export const router = createRouter({ routeTree });

declare module "@tanstack/react-router" {
	interface Register {
		router: typeof router;
	}
}
