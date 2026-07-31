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
