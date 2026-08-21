import { readFileSync } from "node:fs";

const read = (path: string) => readFileSync(path, "utf8");

test("M13 entry-gate records keep the automated and product-run matrix aligned", () => {
	const guides = [
		read("docs/m13-entry-gate-acceptance.md"),
		read("docs/m13-entry-gate-acceptance.zh-CN.md"),
	];
	for (const guide of guides) {
		for (const marker of [
			"cargo test --workspace",
			"cargo check --workspace",
			"pnpm exec jest --watchman=false --runInBand",
			"pnpm run build",
			"Features",
			"Features → Factors → Models",
			"Provider",
		]) {
			expect(guide).toContain(marker);
		}
	}
});
