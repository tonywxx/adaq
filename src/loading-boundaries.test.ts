/// <reference types="node" />

import { readFileSync } from "node:fs";

const read = (path: string) =>
	readFileSync(new URL(path, import.meta.url), "utf8");

test("slow workspace reads expose loading feedback at their data boundaries", () => {
	const backtest = read("./features/backtest/backtest-page.tsx");
	const components = read("./features/components/components-page.tsx");
	const validation = read("./features/validation/validation-page.tsx");

	for (const source of [backtest, components, validation]) {
		expect(source).not.toMatch(/if \(pageLoading\) return <PageLoadingSkeleton/);
	}
	expect(backtest).toMatch(/Loading Run History…/);
	expect(backtest).toMatch(/Loading Snapshots…/);
	expect(components).toMatch(/Loading Component Packages…/);
	expect(components).toMatch(
		/let active = true;[\s\S]*\.finally\(\(\) => \{[\s\S]*if \(active\) setPackagesLoading\(false\);[\s\S]*return \(\) => \{[\s\S]*active = false;/,
	);
	expect(validation).toMatch(/Loading Completed Runs…/);
	expect(validation).toMatch(/Loading Protocols…/);
	expect(validation).toMatch(/Loading Reports…/);
});
