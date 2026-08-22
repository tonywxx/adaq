/// <reference types="node" />

import { readFileSync } from "node:fs";
import { workflowModules, workflowSteps } from "@/features/workflow/workflow";

test("workspace navigation keeps one authenticated shell mounted", () => {
	const source = readFileSync(new URL("./router.tsx", import.meta.url), "utf8");

	expect(source.match(/<AuthGate>/g)).toHaveLength(1);
	expect(source).toMatch(
		/const rootRoute = createRootRoute\(\{\s*component: AppShell/,
	);
	expect(source).toMatch(
		/<AuthGate>\s*<Home showSidebar=\{!isSettings\}>\s*<Outlet \/>\s*<\/Home>\s*<\/AuthGate>/,
	);
	expect(source).toMatch(/path: "\/settings\/\$section"/);
});

	test("routes adaptive home, Help, Operations, and the supported OKX workspace", () => {
	const routerSource = readFileSync(
		new URL("./router.tsx", import.meta.url),
		"utf8",
	);
	const sidebarSource = readFileSync(
		new URL("./components/app-sidebar.tsx", import.meta.url),
		"utf8",
	);
	const marketsSource = readFileSync(
		new URL("./features/markets/markets-page.tsx", import.meta.url),
		"utf8",
	);

	expect(routerSource).toMatch(/path: "\/",\s*component: WorkflowHomePage/);
	expect(routerSource).toMatch(
		/path: "\/operations",\s*component: OperationsDashboard/,
	);
	expect(routerSource).toContain('path: "/help/workflow"');
	expect(routerSource).toContain('path: "/help/workflow/$step"');
	expect(routerSource).toMatch(/const FactorsPage = lazy\(/);
	expect(routerSource).toMatch(
		/path: "\/factors"[\s\S]*?component: \(\) => \([\s\S]*?<FactorsPage \/>/,
	);
	expect(routerSource).toContain('titleKey: "factors.title"');
	for (const path of [
		"/markets",
		"/markets/crypto",
	]) {
		expect(routerSource).toContain(`path: "${path}"`);
	}
	expect(routerSource).not.toMatch(/path: "\/",\s*component: Dashboard/);
	expect(sidebarSource).toMatch(/t\("nav\.marketsData"\)/);
	expect(sidebarSource).toMatch(/to="\/factors"/);
	expect(sidebarSource).toMatch(/t\("nav\.factorResearch"\)/);
	expect(sidebarSource).toMatch(/startsWith\("\/markets"\)/);
	expect(sidebarSource).toContain('url: "/help/workflow"');
	expect(marketsSource).toMatch(/staleTime: 5 \* 60_000/);
	expect(marketsSource).toMatch(/gcTime: 30 \* 60_000/);
	expect(marketsSource).toMatch(/aria-busy=/);
	expect(marketsSource).toMatch(/role="alert"/);
	expect(marketsSource).toMatch(/gapsUnknown/);
});

test("keeps the ten-step workflow ordered and mapped to four modules", () => {
	expect(workflowSteps.map((step) => step.id)).toEqual([
		1, 2, 3, 4, 5, 6, 7, 8, 9, 10,
	]);
	expect(
		workflowModules.map((module) => [
			module.id,
			workflowSteps
				.filter((step) => step.module === module.id)
				.map((step) => step.id),
		]),
	).toEqual([
		["factor", [1, 2, 3]],
		["model", [4, 5, 6]],
		["strategy", [7, 8]],
		["operations", [9, 10]],
	]);
	for (const step of workflowSteps) {
		expect(step.capability === "partial" || step.milestone).toBeTruthy();
	}
	expect(workflowSteps.slice(0, 2)).toEqual([
		{ id: 1, module: "factor", capability: "partial", target: "/factors" },
		{ id: 2, module: "factor", capability: "partial", target: "/factors" },
	]);
});

test("Models switches immediately and keeps loading inside its controls", () => {
	const routerSource = readFileSync(
		new URL("./router.tsx", import.meta.url),
		"utf8",
	);
	const pageSource = readFileSync(
		new URL("./features/models/models-page.tsx", import.meta.url),
		"utf8",
	);
	const adapterSource = readFileSync(
		new URL("./features/models/models-adapter.ts", import.meta.url),
		"utf8",
	);

	expect(routerSource).toMatch(/const ModelsPage = lazy\(/);
	expect(routerSource).not.toMatch(
		/import \{ ModelsPage \} from "@\/features\/models\/models-page"/,
	);
	expect(routerSource).toMatch(
		/path: "\/models"[\s\S]*?component: \(\) => \([\s\S]*?<ModelsPage \/>/,
	);
	expect(pageSource).toMatch(
		/if \(tab !== "datasets" && tab !== "evaluations"\) return;[\s\S]*?refreshDatasets/,
	);
	expect(pageSource).toMatch(/loading\.modelPackages/);
	expect(pageSource).toMatch(/loading\.marketDataSnapshots/);
	expect(pageSource).toMatch(/loading\.generationAttempts/);
	expect(pageSource).toMatch(
		/setSnapshotsLoading\(true\);[\s\S]*?await afterPaint\(\);[\s\S]*?adapter\.listSnapshots/,
	);
	expect(adapterSource).toContain('"snapshot_list_readable"');
	expect(pageSource).not.toMatch(/Loading Models workspace/);
});

test("Forecast Evaluation presentation keeps partial evidence and native exports inspectable", () => {
	const source = readFileSync(
		new URL("./features/models/models-page.tsx", import.meta.url),
		"utf8",
	);
	const metricSource = readFileSync(
		new URL("./features/research/metric-info.tsx", import.meta.url),
		"utf8",
	);
	const tooltipSource = readFileSync(
		new URL("./components/ui/tooltip.tsx", import.meta.url),
		"utf8",
	);

	expect(source).toMatch(/TabsTrigger value="evaluations">Evaluation Reports/);
	expect(source).toMatch(/No Forecast Evaluation Reports yet/);
	expect(source).toMatch(/not proven out-of-sample/);
	expect(source).toMatch(/metrics: report\.metrics,[\s\S]*?unavailableRows/);
	expect(source).toMatch(/aria-live="polite"/);
	expect(source).toMatch(/max-w-full overflow-x-auto/);
	expect(metricSource).toMatch(/<Tooltip open=\{open\}/);
	expect(metricSource).toMatch(/border-b border-dashed/);
	expect(metricSource).not.toMatch(/border-y/);
	expect(metricSource).toMatch(/align="start"/);
	expect(metricSource).not.toMatch(/ⓘ/);
	expect(source).toMatch(/open\(path, \{ write: true, createNew: true \}\)/);
	expect(metricSource).toMatch(/Formula: \{definition\.formula\}/);
	expect(metricSource).toMatch(/TooltipContent/);
	expect(tooltipSource).not.toMatch(/TooltipPrimitive\.Arrow/);
	expect(source).toMatch(/Custom Prediction Kind or Custom Target recorded/);
	expect(source).toMatch(/Single-Instrument time-series evidence/);
	expect(source).toMatch(/Five-quantile realized Target evidence/);
	expect(source).toMatch(/undefinedMetrics\?\.quantiles/);
});
