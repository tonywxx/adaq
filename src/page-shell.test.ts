/// <reference types="node" />

import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const read = (path: string) =>
	readFileSync(new URL(path, import.meta.url), "utf8");

// Workspace pages share one full-width shell so no page centres itself in a
// bounded column. data-foundation and components/markets are the reference
// implementations; pages that render through the shared Workspace/PageFrame
// shells inherit it.
const SHELL_TOKENS = [
	"flex",
	"min-w-0",
	"flex-1",
	"flex-col",
	"gap-5",
	"p-4",
	"lg:p-6",
];
const SHARED_SHELLS = [/<Workspace[\s>]/, /<PageFrame[\s>]/];

const pagesDirectory = fileURLToPath(new URL("./features/", import.meta.url));

function pageModules() {
	const modules: string[] = [];
	for (const directory of readdirSync(pagesDirectory, { withFileTypes: true })) {
		if (!directory.isDirectory()) continue;
		for (const file of readdirSync(join(pagesDirectory, directory.name))) {
			if (file.endsWith("-page.tsx") || file === "operations-dashboard.tsx") {
				modules.push(`./features/${directory.name}/${file}`);
			}
		}
	}
	return modules.sort();
}

function classNames(source: string) {
	return [...source.matchAll(/className=(?:"([^"]*)"|\{`([^`]*)`\})/g)].map(
		(match) => match[1] ?? match[2] ?? "",
	);
}

function tokens(value: string) {
	return new Set(value.split(/\s+/).filter(Boolean));
}

test("every workspace page renders inside the shared full-width shell", () => {
	const modules = pageModules();
	expect(modules.length).toBeGreaterThan(10);
	for (const modulePath of modules) {
		const source = read(modulePath);
		const rendersShell = classNames(source).some((value) => {
			const names = tokens(value);
			return SHELL_TOKENS.every((token) => names.has(token));
		});
		const inheritsShell = SHARED_SHELLS.some((pattern) => pattern.test(source));
		expect({ modulePath, shell: rendersShell || inheritsShell }).toEqual({
			modulePath,
			shell: true,
		});
	}
});

test("no workspace page centres its content in a bounded column", () => {
	for (const modulePath of pageModules()) {
		const centred = classNames(read(modulePath)).filter((value) => {
			const names = tokens(value);
			if (!names.has("mx-auto")) return false;
			return [...names].some((token) =>
				/^max-w-(?!full$|none$|fit$|min$|max$)/.test(token),
			);
		});
		expect({ modulePath, centred }).toEqual({ modulePath, centred: [] });
	}
});
