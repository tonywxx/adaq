import "./styles/globals.css";
import { RouterProvider } from "@tanstack/react-router";
import { ThemeProvider } from "@/components/theme-provider";
import { router } from "@/router";

function App() {
	return (
		<ThemeProvider attribute="class" defaultTheme="system" enableSystem>
			<RouterProvider router={router} />
		</ThemeProvider>
	);
}

export default App;
