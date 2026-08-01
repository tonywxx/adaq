/// <reference types="node" />

import { readFileSync } from "node:fs";

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

test("Models switches immediately and keeps loading inside its controls", () => {
	const routerSource = readFileSync(
		new URL("./router.tsx", import.meta.url),
		"utf8",
	);
	const pageSource = readFileSync(
		new URL("./features/models/models-page.tsx", import.meta.url),
		"utf8",
	);

	expect(routerSource).toMatch(
		/import \{ ModelsPage \} from "@\/features\/models\/models-page"/,
	);
	expect(routerSource).toMatch(/path: "\/models",\s*component: ModelsPage/);
	expect(routerSource).not.toMatch(/const ModelsPage = lazy/);
	expect(pageSource).toMatch(
		/if \(tab !== "datasets" && tab !== "evaluations"\) return;[\s\S]*?refreshDatasets/,
	);
	expect(pageSource).toMatch(/Loading Model Packages/);
	expect(pageSource).toMatch(/Loading Market Data Snapshots/);
	expect(pageSource).toMatch(/Loading Generation Attempts/);
	expect(pageSource).toMatch(
		/setSnapshotsLoading\(true\);[\s\S]*?await afterPaint\(\);[\s\S]*?snapshot_list_readable/,
	);
	expect(pageSource).not.toMatch(/Loading Models workspace/);
});
