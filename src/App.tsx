import "./styles/globals.css";
import { ThemeProvider } from "@/components/theme-provider";
import { AuthGate } from "@/components/auth-gate";
import { lazy, Suspense } from "react";

const AuthenticatedApp = lazy(() => import("@/authenticated-app"));

function PostAuthLoading() {
	return <main className="min-h-svh bg-background" aria-busy="true" />;
}

function App() {
	return (
		<ThemeProvider attribute="class" defaultTheme="system" enableSystem>
			<AuthGate>
				{(userId) => (
					<Suspense fallback={<PostAuthLoading />}>
						<AuthenticatedApp userId={userId} />
					</Suspense>
				)}
			</AuthGate>
		</ThemeProvider>
	);
}

export default App;
