/// <reference types="node" />

import { readFileSync } from "node:fs";

const read = (path: string) =>
	readFileSync(new URL(path, import.meta.url), "utf8");

test("slow workspace reads expose loading feedback at their data boundaries", () => {
	const backtest = read("./features/backtest/backtest-page.tsx");
	const components = read("./features/components/components-page.tsx");
	const models = read("./features/models/models-page.tsx");
	const validation = read("./features/validation/validation-page.tsx");
	const features = read("./features/features/features-page.tsx");
	const factors = read("./features/factors/factors-page.tsx");
	const workflow = read("./features/workflow/workflow-page.tsx");
	const app = read("./App.tsx");
	const main = read("./main.tsx");
	const authenticatedApp = read("./authenticated-app.tsx");
	const authGate = read("./components/auth-gate.tsx");
	const startupTiming = read("./lib/startup-timing.ts");
	const index = read("../index.html");

	for (const source of [
		backtest,
		components,
		models,
		validation,
		features,
		factors,
	]) {
		expect(source).not.toMatch(/if \(pageLoading\) return <PageLoadingSkeleton/);
	}
	expect(backtest).toMatch(/loading\.runHistory/);
	expect(backtest).toMatch(/loading\.snapshots/);
	expect(components).toMatch(/loading\.componentPackages/);
	expect(components).toMatch(
		/let active = true;[\s\S]*\.finally\(\(\) => \{[\s\S]*if \(active\) setPackagesLoading\(false\);[\s\S]*return \(\) => \{[\s\S]*active = false;/,
	);
	expect(models).toMatch(/loading\.modelPackages/);
	expect(models).toMatch(/loading\.marketDataSnapshots/);
	expect(models).toMatch(/loading\.generationAttempts/);
	expect(models).toMatch(
		/requestAnimationFrame\(\(\) =>\s*requestAnimationFrame\(\(\) => resolve\(\)\)/,
	);
	for (const refresh of [
		"refreshComponents",
		"refreshSnapshots",
		"refreshAttempts",
	]) {
		expect(models).toMatch(new RegExp(`const ${refresh} = useCallback`));
	}
	expect(validation).toMatch(/loading\.completedRuns/);
	expect(validation).toMatch(/loading\.protocols/);
	expect(validation).toMatch(/loading\.reports/);

	// The /features shell paints immediately; session wait shows bounded
	// aria-busy feedback, and each owning view manages its own loading state.
	expect(features).toMatch(/aria-busy="true"/);
	expect(features).toMatch(
		/<DefinitionsView userId=\{userId\} adapter=\{adapter\} \/>/,
	);
	expect(features).toMatch(
		/<FittingView userId=\{userId\} adapter=\{adapter\} \/>/,
	);
	expect(features).toMatch(
		/<MaterializationView userId=\{userId\} adapter=\{adapter\} \/>/,
	);
	expect(features).toMatch(
		/<DatasetsView userId=\{userId\} adapter=\{adapter\} \/>/,
	);
	expect(workflow).toMatch(
		/requestAnimationFrame\([\s\S]*?requestAnimationFrame\([\s\S]*?import\("@antv\/infographic"\)/,
	);
	expect(workflow).toContain('markStartup("adaq:help-visible")');
	expect(startupTiming).toContain(
		'console.info("[AdaQ startup timing]", report)',
	);
	expect(startupTiming).toContain('"adaq:webview-to-help"');
	expect(app).toMatch(
		/const AuthenticatedApp = lazy\(\(\) => import\("@\/authenticated-app"\)/,
	);
	expect(app).toMatch(/<AuthGate>/);
	expect(main).toContain('import "@/lib/i18n-core"');
	expect(authenticatedApp).toContain('import "@/lib/i18n"');
	expect(authGate).toMatch(
		/const AuthEntry = lazy\(\(\) => import\("\.\/auth-entry"\)\)/,
	);
	expect(authGate).not.toContain('from "@/components/ui/card"');
	expect(authGate).not.toContain("checkStrongPassword");
	expect(index).toContain('performance.mark("adaq:webview-start")');
	expect(main).toContain('markStartup("adaq:react-entry")');
	expect(authGate).toContain('markStartup("adaq:auth-loading-visible")');
	expect(workflow).toMatch(/id="workflow-steps"/);
	const definitions = read("./features/features/definitions-view.tsx");
	expect(definitions).toMatch(/useState<"validate" \| "publish" \| null>/);
	expect(definitions).toMatch(
		/<FeaturesLoading label=\{t\("features.loading"\)\} \/>/,
	);
});
