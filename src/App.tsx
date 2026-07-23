import "./styles/globals.css";
import { ThemeProvider } from "@/components/theme-provider";
import { router } from "@/router";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";

const queryClient = new QueryClient();

function App() {
	return (
		<QueryClientProvider client={queryClient}>
			<ThemeProvider attribute="class" defaultTheme="system" enableSystem>
				<RouterProvider router={router} />
			</ThemeProvider>
		</QueryClientProvider>
	);
}

export default App;
