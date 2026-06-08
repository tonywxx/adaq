import "./styles/globals.css";
import LayoutMain from "./layout/layout-main";
import { ThemeProvider } from "@/components/theme-provider";

function App() {
	return (
		<ThemeProvider attribute="class" defaultTheme="system" enableSystem>
			<LayoutMain />
		</ThemeProvider>
	);
}

export default App;
