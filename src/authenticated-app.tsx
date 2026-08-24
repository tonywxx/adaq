import { AuthenticatedUserContext } from "@/authenticated-user";
import "@/lib/i18n";
import { useAppShortcuts } from "@/hooks/use-app-shortcuts";
import { router } from "@/router";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";

const queryClient = new QueryClient();

export default function AuthenticatedApp({ userId }: { userId: string }) {
	useAppShortcuts();

	return (
		<AuthenticatedUserContext.Provider value={userId}>
			<QueryClientProvider client={queryClient}>
				<RouterProvider router={router} />
			</QueryClientProvider>
		</AuthenticatedUserContext.Provider>
	);
}
