import { readFileSync } from "node:fs";

const read = (path: string) => readFileSync(path, "utf8");

test("M8 manual acceptance guides expose the same executable contract", () => {
	const english = read("docs/m8-manual-acceptance.md");
	const chinese = read("docs/m8-manual-acceptance.zh-CN.md");
	const checkpoints = [
		"prerequisites",
		"components",
		"native-dataset",
		"external-dataset",
		"evaluation",
		"backtests",
		"negative-paths",
		"regressions",
		"automated-gates",
		"acceptance-record",
	];
	const commands = [
		"Node.js 24",
		"pnpm@11.18.0",
		"adaq-component new model",
		"adaq-component new strategy",
		"--template composed",
		"adaq-component build",
		"adaq-component verify",
		"python -m unittest test_adapter.py",
		"kronos_fixture_reaches_import_evaluation_and_dataset_first_backtest",
		"cargo test --workspace",
		"cargo check --workspace",
		"pnpm exec jest --watchman=false --runInBand",
		"pnpm run build",
	];

	for (const guide of [english, chinese]) {
		for (const checkpoint of checkpoints) {
			expect(guide).toContain(`<!-- m8-acceptance:${checkpoint} -->`);
		}
		for (const command of commands) expect(guide).toContain(command);
		expect(guide).toContain("Get-FileHash -Algorithm SHA256");
		expect(guide).toContain("sha256sum");
	}
});

test("both READMEs link to both M8 acceptance guides", () => {
	for (const readme of [read("README.md"), read("README.zh-CN.md")]) {
		expect(readme).toContain("docs/m8-manual-acceptance.md");
		expect(readme).toContain("docs/m8-manual-acceptance.zh-CN.md");
	}
});
