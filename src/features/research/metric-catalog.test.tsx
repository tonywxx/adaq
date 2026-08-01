/** @jest-environment jsdom */

import { readFileSync } from "node:fs";
import { act } from "react";
import { createRoot } from "react-dom/client";
import {
	getMetricDefinition,
	METRIC_CATALOG,
} from "@/features/research/metric-catalog";
import { MetricInfo } from "@/features/research/metric-info";

(
	globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }
).IS_REACT_ACT_ENVIRONMENT = true;

test("covers every rendered Forecast, Backtest, and Validation metric", () => {
	const sources = {
		forecast: readFileSync("src/features/models/models-page.tsx", "utf8"),
		backtest: readFileSync("src/features/backtest/backtest-page.tsx", "utf8"),
		validation: readFileSync(
			"src/features/validation/validation-page.tsx",
			"utf8",
		),
	};
	for (const source of Object.values(sources)) {
		const rendered = [...source.matchAll(/metricId="([^"]+)"/g)].map(
			(match) => match[1],
		);
		expect(rendered).not.toHaveLength(0);
		for (const id of rendered) expect(getMetricDefinition(id)).toBeDefined();
	}
});

test("fails loudly when a rendered metric has no definition", () => {
	expect(() => getMetricDefinition("missing.metric")).toThrow(
		"Missing Metric Definition: missing.metric",
	);
});

test("keeps representative research meanings precise and contextual", () => {
	const ic = getMetricDefinition("forecast.pearson-ic");
	const totalReturn = getMetricDefinition("strategy.total-return");

	expect(ic.id).toBe("forecast.pearson-ic");
	expect(ic.version).toMatch(/^1\./);
	expect(ic.meaning).toContain("single-Instrument time-series");
	expect(ic.caveat).toContain("not cross-sectional IC");
	expect(ic.undefinedState).toContain("constant");
	expect(totalReturn.meaning).toContain("Strategy");
	expect(totalReturn.caveat).not.toMatch(/universally good|universally bad/i);
	for (const definition of Object.values(METRIC_CATALOG)) {
		expect(definition).toEqual(
			expect.objectContaining({
				id: expect.any(String),
				version: expect.any(String),
				meaning: expect.any(String),
				formula: expect.any(String),
				direction: expect.any(String),
				caveat: expect.any(String),
				undefinedState: expect.any(String),
				documentationUrl: expect.stringContaining("research-metrics.md#"),
			}),
		);
	}
});

test.each(["pointer", "focus", "click"])(
	"opens the accessible definition by %s",
	async (mode) => {
		const container = document.createElement("div");
		document.body.append(container);
		const root = createRoot(container);
		await act(async () => {
			root.render(<MetricInfo metricId="strategy.sharpe" />);
		});
		const trigger = container.querySelector(
			'button[aria-label="Sharpe definition"]',
		) as HTMLButtonElement | null;
		if (!trigger) throw new Error(container.innerHTML);
		expect(trigger.className).toContain("focus-visible:ring-2");
		expect(trigger.className).toContain("border-b");
		expect(trigger.className).not.toContain("border-y");
		expect(trigger.className).toContain("border-dashed");
		expect(trigger.textContent).toBe("Sharpe");
		expect(trigger.textContent).not.toContain("ⓘ");

		await act(async () => {
			if (mode === "pointer") {
				trigger.dispatchEvent(new MouseEvent("mouseenter"));
			} else if (mode === "focus") {
				trigger.focus();
			} else {
				trigger.click();
			}
			await new Promise((resolve) => setTimeout(resolve, 0));
		});

		expect(document.body.textContent).toContain("Formula:");
		expect(document.body.textContent).toContain("Undefined:");
		expect(document.body.textContent).toContain("strategy.sharpe@1.0.0");
		const content = document.body.querySelector(
			'[data-slot="tooltip-content"]',
		) as HTMLElement | null;
		expect(content?.className).toContain("bg-popover");
		expect(content?.className).toContain("text-popover-foreground");
		expect(content?.parentElement?.className).toContain("z-50");

		await act(async () => root.unmount());
		container.remove();
	},
);

test("keeps English and Simplified Chinese references complete", () => {
	const references = [
		readFileSync("docs/reference/research-metrics.md", "utf8"),
		readFileSync("docs/reference/research-metrics.zh-CN.md", "utf8"),
	];
	for (const definition of Object.values(METRIC_CATALOG)) {
		for (const reference of references) {
			expect(reference).toContain(`${definition.id}@${definition.version}`);
		}
	}
});
