export type WorkflowModuleId = "factor" | "model" | "strategy" | "operations";
export type WorkflowCapability = "partial" | "planned";
export type WorkflowStepId = 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10;

export const workflowModules = [
	{ id: "factor", steps: "1–3", accent: "bg-cyan-600" },
	{ id: "model", steps: "4–6", accent: "bg-violet-600" },
	{ id: "strategy", steps: "7–8", accent: "bg-amber-600" },
	{ id: "operations", steps: "9–10", accent: "bg-rose-600" },
] as const;

export const workflowSteps: readonly {
	id: WorkflowStepId;
	module: WorkflowModuleId;
	capability: WorkflowCapability;
	milestone?: string;
	target?: "/components" | "/models" | "/backtest";
}[] = [
	{ id: 1, module: "factor", capability: "planned", milestone: "M11" },
	{ id: 2, module: "factor", capability: "planned", milestone: "M11" },
	{ id: 3, module: "factor", capability: "partial", target: "/components" },
	{ id: 4, module: "model", capability: "planned", milestone: "M12" },
	{ id: 5, module: "model", capability: "partial", target: "/models" },
	{ id: 6, module: "model", capability: "partial", target: "/models" },
	{ id: 7, module: "strategy", capability: "planned", milestone: "M13" },
	{ id: 8, module: "strategy", capability: "partial", target: "/backtest" },
	{
		id: 9,
		module: "operations",
		capability: "planned",
		milestone: "M15–M16",
	},
	{
		id: 10,
		module: "operations",
		capability: "planned",
		milestone: "M17–M18",
	},
];
