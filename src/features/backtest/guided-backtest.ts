import type { LibraryComponent } from "@/features/components/component-library";

export const defaultExecutionProfile = {
	makerFeeRate: "0.0008",
	takerFeeRate: "0.001",
	adverseSlippageRate: "0.0005",
	rebalanceThreshold: "0",
	priceIncrement: "0.1",
	quantityIncrement: "0.00000001",
	minimumQuantity: "0.00001",
	riskFreeRate: "0",
	fillPolicy: "taker" as "maker" | "taker",
};

type Dependency = LibraryComponent["dependencies"][number];

export function matchingFactors(
	dependency: Dependency,
	components: readonly LibraryComponent[],
) {
	return components.filter(
		(component) =>
			component.kind === "factor" &&
			component.compatible &&
			!component.compatibilityError &&
			component.componentId === dependency.componentId &&
			versionMatches(component.version, dependency.version),
	);
}

export function parameterValues(
	component: LibraryComponent,
	overrides: Record<string, string>,
) {
	return component.parameters.map((parameter) => ({
		name: parameter.name,
		value: overrides[parameter.name] ?? parameter.defaultValue,
	}));
}

function versionMatches(version: string, requirement: string) {
	const current = version.split(".").map(Number);
	const required = requirement.match(/(?:\^|~|>=|>|=)?\s*(\d+)(?:\.(\d+))?(?:\.(\d+))?/);
	if (!required || current.some((part) => !Number.isInteger(part))) return false;
	const target = [Number(required[1]), Number(required[2] ?? 0), Number(required[3] ?? 0)];
	const comparison = current.findIndex((part, index) => part !== target[index]);
	const atLeast = comparison < 0 || current[comparison] > target[comparison];
	if (requirement.startsWith("^"))
		return atLeast && (target[0] ? current[0] === target[0] : target[1] ? current[1] === target[1] : current[2] === target[2]);
	if (requirement.startsWith("~")) return atLeast && current[0] === target[0] && current[1] === target[1];
	return current.every((part, index) => part === target[index]);
}

export function runGate({
	snapshotId,
	strategy,
	dependencies,
	factorSelections,
	initialQuoteAllocation,
	executionValues = [],
	running = false,
}: {
	snapshotId?: string;
	strategy?: LibraryComponent;
	dependencies: readonly Dependency[];
	factorSelections: Record<string, string>;
	initialQuoteAllocation: string;
	executionValues?: string[];
	running?: boolean;
}) {
	if (running) return "A Backtest is already running.";
	if (!snapshotId) return "Select a Market Data Snapshot before continuing.";
	if (!strategy) return "Select a compatible Strategy Component before continuing.";
	if (dependencies.some((dependency) => !factorSelections[dependency.alias]))
		return "Select a matching Factor Component for every required dependency.";
	if (!/^(?:0|[1-9]\d*)(?:\.\d+)?$/.test(initialQuoteAllocation))
		return "Initial quote allocation must be an exact non-negative decimal.";
	if (executionValues.some((value) => !/^(?:0|[1-9]\d*)(?:\.\d+)?$/.test(value)))
		return "Execution Profile values must be exact non-negative decimals.";
	return undefined;
}
