/// <reference types="node" />

import { readFileSync } from "node:fs";

const source = readFileSync(
	new URL("./factors-page.tsx", import.meta.url),
	"utf8",
);
const adapterSource = readFileSync(
	new URL("./factor-adapter.ts", import.meta.url),
	"utf8",
);

test("Factor Lab paints its shell immediately and guards cached reads from stale responses", () => {
	expect(source).toContain('data-route="factors"');
	expect(source).toMatch(/const afterPaint =/);
	expect(source).toMatch(/requestAnimationFrame\(\(\) => requestAnimationFrame/);
	expect(source).toMatch(/readFactorCache/);
	expect(source).toMatch(/writeFactorCache/);
	expect(source).toMatch(/const version = useRef\(0\)/);
	expect(source).toMatch(/if \(current !== version\.current\) return/);
	expect(source).not.toMatch(/if \(pageLoading\) return/);
});

test("Factor Lab exposes bounded workflows with local loading, recovery, and evidence boundaries", () => {
	for (const tab of [
		"families",
		"candidates",
		"datasets",
		"evaluations",
		"decisions",
	]) {
		expect(source).toContain(`TabsTrigger value="${tab}"`);
	}
	for (const operation of [
		"factor_candidate_publish",
		"factor_family_grid_register",
		"factor_materialization_start",
		"factor_evaluation_start",
		"factor_evaluation_protocol_freeze",
		"factor_materialization_protocol_freeze",
		"factor_metric_catalog",
		"factor_decision_library",
		"factor_attempt_cancel",
		"factor_attempt_retry",
	]) {
		expect(adapterSource).toContain(operation);
	}
	expect(source).toMatch(/aria-busy="true"/);
	expect(source).toMatch(/role="alert"/);
	expect(source).toMatch(/progress/);
	expect(source).toMatch(/notImported/);
	expect(source).toMatch(/MetricDefinition/);
	expect(source).toMatch(/metricObservation/);
	expect(source).toMatch(/textAt\(metric, "foldId"\)/);
	expect(source).toMatch(/freezeEvaluationProtocol/);
	expect(source).toMatch(/freezeMaterializationProtocol/);
	expect(adapterSource).toMatch(
		/instrumentId: instrumentId\.trim\(\) \|\| null/,
	);
	expect(source).not.toContain('kind="factor-family"');
	expect(source).toMatch(/eligibility\.gates\.map/);
});
