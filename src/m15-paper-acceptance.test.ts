import { readFileSync } from "node:fs";

const guides = [
	"docs/m15-paper-trading-manual-acceptance.md",
	"docs/m15-paper-trading-manual-acceptance.zh-CN.md",
];

test("M15 acceptance remains bilingual and OKX Demo-only", () => {
	const contents = guides.map((guide) => readFileSync(guide, "utf8"));
	const markers = contents.map((content) =>
		[...content.matchAll(/<!-- m15-acceptance:([a-z-]+) -->/g)].map(
			(match) => match[1],
		),
	);
	expect(markers[1]).toEqual(markers[0]);
	for (const content of contents) {
		expect(content).toContain("OKX Demo");
		expect(content).toContain("paper_account_reconcile");
		expect(content).toContain("paper_order_submit");
		expect(content).toContain("paper_order_cancel");
		expect(content).toContain("paper_order_sync");
		expect(content).toContain("en-US");
		expect(content).toContain("zh-CN");
		expect(content).not.toContain("Real Trading");
	}
});
