import { readFileSync } from "node:fs";

const read = (path: string) => readFileSync(path, "utf8");

test("M9 manual acceptance guides expose the same executable contract", () => {
	const guides = [
		read("docs/m9-manual-acceptance.md"),
		read("docs/m9-manual-acceptance.zh-CN.md"),
	];
	const checkpoints = [
		"scope",
		"localization",
		"connections",
		"crypto",
		"a-shares",
		"us-equities",
		"quality-snapshot",
		"markets",
		"regressions",
		"automated-gates",
		"acceptance-matrix",
		"acceptance-record",
	];
	const commands = [
		"Node.js 24",
		"pnpm 11.20.0",
		"cargo fmt --all --check",
		"cargo test --workspace",
		"cargo check --workspace",
		"pnpm exec jest --watchman=false --runInBand",
		"pnpm run build",
		"connection_test_never_requests_an_order_endpoint",
		"Get-FileHash -Algorithm SHA256",
		"sha256sum",
		"shasum -a 256",
	];

	for (const guide of guides) {
		for (const checkpoint of checkpoints) {
			expect(guide).toContain(`<!-- m9-acceptance:${checkpoint} -->`);
		}
		for (const command of commands) expect(guide).toContain(command);
		for (const issue of [
			"#66",
			"#67",
			"#68",
			"#69",
			"#70",
			"#71",
			"#72",
			"#73",
			"#74",
			"#75",
			"#76",
		])
			expect(guide).toContain(issue);
	}
});

test("both READMEs link to both M9 acceptance guides", () => {
	for (const readme of [read("README.md"), read("README.zh-CN.md")]) {
		expect(readme).toContain("docs/m9-manual-acceptance.md");
		expect(readme).toContain("docs/m9-manual-acceptance.zh-CN.md");
	}
});
