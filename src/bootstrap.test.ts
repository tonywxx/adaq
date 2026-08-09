/// <reference types="node" />

import { readFileSync } from "node:fs";

test("initial HTML paints a loading spinner before React starts", () => {
	const source = readFileSync(new URL("../index.html", import.meta.url), "utf8");
	const spinner = source.indexOf('id="app-bootstrap"');
	const appScript = source.indexOf('src="/src/main.tsx"');

	expect(spinner).toBeGreaterThan(-1);
	expect(spinner).toBeLessThan(appScript);
	expect(source).toMatch(/role="status"/);
	expect(source).toMatch(/aria-label="AdaQ"/);
});
