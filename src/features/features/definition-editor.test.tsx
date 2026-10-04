/** @jest-environment jsdom */

import "@/lib/i18n";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { DefinitionEditor } from "./definition-editor";
import type { DefinitionDraft } from "./features-types";

(
	globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }
).IS_REACT_ACT_ENVIRONMENT = true;

test("allows a time-series node to feed a cross-sectional output", async () => {
	const draft: DefinitionDraft = {
		definitionId: "definition-1",
		revision: 1,
		scope: "cross-sectional",
		nodes: [
			{
				id: "node-1",
				operator: { kind: "backward-simple-return" },
				scope: "cross-sectional",
				inputs: [{ kind: "market", field: "close" }],
				parameters: { period: 20 },
				warmupBars: 20,
			},
			{
				id: "node-2",
				operator: { kind: "cross-sectional-percentile" },
				scope: "cross-sectional",
				inputs: [{ kind: "node", nodeId: "node-1" }],
				parameters: {},
				warmupBars: 1,
			},
		],
		outputs: [{ name: "momentum-score", nodeId: "node-2" }],
	};
	const container = document.createElement("div");
	document.body.append(container);
	const root: Root = createRoot(container);
	let updated = draft;

	await act(async () => {
		root.render(
			<DefinitionEditor draft={draft} onChange={(next) => (updated = next)} />,
		);
	});

	const scope = container.querySelector<HTMLSelectElement>("#node-scope-node-1");
	if (!scope) throw new Error("node scope selector was not rendered");
	await act(async () => {
		scope.value = "time-series";
		scope.dispatchEvent(new Event("change", { bubbles: true }));
	});

	expect(updated.scope).toBe("cross-sectional");
	expect(updated.nodes.map((node) => node.scope)).toEqual([
		"time-series",
		"cross-sectional",
	]);

	await act(async () => root.unmount());
	container.remove();
});
