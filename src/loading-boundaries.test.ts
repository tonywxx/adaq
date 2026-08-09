/// <reference types="node" />

import { readFileSync } from "node:fs";

const read = (path: string) =>
	readFileSync(new URL(path, import.meta.url), "utf8");

test("slow workspace reads expose loading feedback at their data boundaries", () => {
	const backtest = read("./features/backtest/backtest-page.tsx");
	const components = read("./features/components/components-page.tsx");
	const models = read("./features/models/models-page.tsx");
	const validation = read("./features/validation/validation-page.tsx");

	for (const source of [backtest, components, models, validation]) {
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
});
