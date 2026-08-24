import "@/lib/i18n-core";
import { markStartup } from "@/lib/startup-timing";
import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";

markStartup("adaq:webview-start");
markStartup("adaq:react-entry");

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
	<React.StrictMode>
		<App />
	</React.StrictMode>,
);
