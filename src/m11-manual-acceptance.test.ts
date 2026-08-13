import { readFileSync } from "node:fs";

const read = (path: string) => readFileSync(path, "utf8");

const checkpoints = [
	"scope",
	"contracts",
	"okx-journey",
	"a-share-journey",
	"us-equity-journey",
	"candidate-paths",
	"failure-recovery",
	"factor-gui",
	"boundary",
	"performance-baselines",
	"regressions",
	"automated-gates",
	"platform-evidence",
	"acceptance-matrix",
	"acceptance-record",
];

const guides = [
	"docs/m11-manual-acceptance.md",
	"docs/m11-manual-acceptance.zh-CN.md",
];

test("M11 manual acceptance guides expose the same executable contract", () => {
	const contents = guides.map(read);
	const commands = [
		"Node.js 24",
		"pnpm 11.20.0",
		"pnpm install --frozen-lockfile",
		"cargo fmt --all --check",
		"cargo test --workspace",
		"cargo check --workspace",
		"cargo test -p adaq-factor-research --test reference_fixtures",
		"cargo test -p adaq-factor-research --test metric_golden",
		"cargo test -p adaq-factor-research --release --test benchmarks -- --ignored --test-threads=1",
		"sh crates/adaq-factor-research/scripts/check_generated.sh",
		"pnpm exec jest --watchman=false --runInBand",
		"pnpm run build",
		"pnpm run lint",
		'gh workflow run "Indicator engine acceptance" --ref',
		"Get-FileHash -Algorithm SHA256",
		"sha256sum",
		"shasum -a 256",
		"factor-benchmark-baseline.json",
	];
	for (const guide of contents) {
		for (const checkpoint of checkpoints)
			expect(guide).toContain(`<!-- m11-acceptance:${checkpoint} -->`);
		for (const command of commands) expect(guide).toContain(command);
		for (const issue of [
			"#88",
			"#89",
			"#90",
			"#91",
			"#92",
			"#93",
			"#94",
			"#95",
			"#96",
		])
			expect(guide).toContain(issue);

		const ordered = [
			...guide.matchAll(/<!-- m11-acceptance:([a-z0-9-]+) -->/g),
		].map((match) => match[1]);
		expect(ordered).toEqual(checkpoints);
	}

	const englishMarkers = contents[0].match(
		/<!-- m11-acceptance:[a-z0-9-]+ -->/g,
	);
	const chineseMarkers = contents[1].match(
		/<!-- m11-acceptance:[a-z0-9-]+ -->/g,
	);
	expect(chineseMarkers).toEqual(englishMarkers);
});

test("both READMEs link to both accepted M11 guides", () => {
	for (const readme of [read("README.md"), read("README.zh-CN.md")]) {
		const row = readme.split("\n").find((line) => line.startsWith("| M11 |"));
		expect(row).toBeDefined();
		const acceptedMarker = row?.includes("Status: Accepted")
			? "Status: Accepted"
			: "状态：已接受";
		const acceptedAt = row?.indexOf(acceptedMarker) ?? -1;
		expect(acceptedAt).toBeGreaterThanOrEqual(0);
		for (const guide of guides)
			expect(row?.indexOf(guide) ?? -1).toBeGreaterThan(acceptedAt);
	}
});

test("M11 architecture and delivery docs remain bilingual and accepted", () => {
	const architecture = [
		read("docs/m11-factor-research.md"),
		read("docs/m11-factor-research.zh-CN.md"),
	];
	const sectionShape = (guide: string) =>
		[...guide.matchAll(/^(#{1,3})\s+/gm)].map((match) => match[1].length);

	expect(sectionShape(architecture[1])).toEqual(sectionShape(architecture[0]));
	for (const concept of [
		"adaq-factor-research",
		"Factor ABI v2",
		"Metric Catalog",
		"reset-required",
		"Parquet",
		"FIFO",
		"/factors",
		"#88",
		"#93",
		"M12",
	])
		for (const guide of architecture) expect(guide).toContain(concept);
	for (const guide of architecture) {
		expect(guide).toContain("M11.8");
		expect(guide).not.toContain("pending final #93");
		expect(guide).not.toContain("仍等待最终 #93");
	}
});

test("M11 navigation and GUI copy no longer describe delivered Factor workflows as planned", () => {
	const navigation = read("docs/workflow-navigation.md");
	const i18n = read("src/lib/i18n.ts");

	expect(navigation).toContain("Available · M11");
	expect(navigation).toContain("/factors");
	expect(i18n).toContain("Open the localized Factor Lab at /factors");
	expect(i18n).toContain("在 /factors 打开本地化因子实验室");
	expect(i18n).not.toContain(
		"Dedicated Factor Discovery workspace planned for M11",
	);
	expect(i18n).not.toContain(
		"Factor evaluation and promotion workflow planned for M11",
	);
	expect(i18n).not.toContain("专用因子发掘工作区计划在 M11 实现");
	expect(i18n).not.toContain("因子评估与晋级流程计划在 M11 实现");
});
