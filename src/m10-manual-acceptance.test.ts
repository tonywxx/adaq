import { readFileSync } from "node:fs";

const read = (path: string) => readFileSync(path, "utf8");

test("M10 manual acceptance guides expose the same executable contract", () => {
	const guides = [
		read("docs/m10-manual-acceptance.md"),
		read("docs/m10-manual-acceptance.zh-CN.md"),
	];
	const checkpoints = [
		"scope",
		"definitions",
		"fitting",
		"materialization",
		"datasets",
		"okx-journey",
		"a-share-journey",
		"us-equity-journey",
		"semantics",
		"isolation",
		"features-gui",
		"performance-baselines",
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
		"pnpm run lint",
		"git diff --check",
		'gh workflow run "Indicator engine acceptance" --ref',
		"cargo test -p adaq-feature-engine --release --test benchmarks -- --ignored --test-threads=1",
		"feature-benchmark-baseline.json",
		"Get-FileHash -Algorithm SHA256",
		"sha256sum",
		"shasum -a 256",
	];

	for (const guide of guides) {
		for (const checkpoint of checkpoints) {
			expect(guide).toContain(`<!-- m10-acceptance:${checkpoint} -->`);
		}
		for (const command of commands) expect(guide).toContain(command);
		for (const issue of [
			"#77",
			"#78",
			"#79",
			"#80",
			"#81",
			"#82",
			"#83",
			"#84",
			"#85",
			"#86",
			"#87",
		])
			expect(guide).toContain(issue);

		const ordered = [
			...guide.matchAll(/<!-- m10-acceptance:([a-z0-9-]+) -->/g),
		].map((match) => match[1]);
		expect(ordered).toEqual(checkpoints);
	}
});

test("both READMEs link to both M10 acceptance guides", () => {
	for (const readme of [read("README.md"), read("README.zh-CN.md")]) {
		const m10Row = readme.split("\n").find((line) => line.startsWith("| M10 |"));
		expect(m10Row).toBeDefined();
		const acceptedMarker = m10Row?.includes("Status: Accepted")
			? "Status: Accepted"
			: "状态：已接受";
		const acceptedAt = m10Row?.indexOf(acceptedMarker) ?? -1;
		expect(acceptedAt).toBeGreaterThanOrEqual(0);
		for (const guide of [
			"docs/m10-manual-acceptance.md",
			"docs/m10-manual-acceptance.zh-CN.md",
		]) {
			expect(m10Row?.indexOf(guide) ?? -1).toBeGreaterThan(acceptedAt);
		}
	}
});

test("M10 architecture guides retain matching ordered sections", () => {
	const guides = [
		read("docs/m10-feature-engineering.md"),
		read("docs/m10-feature-engineering.zh-CN.md"),
	];
	const sectionShape = (guide: string) =>
		[...guide.matchAll(/^(#{1,3})\s+/gm)].map((match) => match[1].length);

	expect(sectionShape(guides[1])).toEqual(sectionShape(guides[0]));
	for (const concept of [
		"adaq-feature-engine",
		'planSchemaVersion: "2.0.0"',
		"Available At",
		"Point-in-Time",
		"Unavailable",
		"Parquet",
		"FIFO",
		"/features",
		"#77",
		"#87",
		"M11",
	]) {
		for (const guide of guides) expect(guide).toContain(concept);
	}
});
