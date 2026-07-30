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
	compatibleHashes: readonly string[],
) {
	return components.filter(
		(component) =>
			component.kind === "factor" &&
			component.compatible &&
			!component.compatibilityError &&
			component.componentId === dependency.componentId &&
			compatibleHashes.includes(component.archiveSha256),
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

export function runGate({
	snapshotId,
	strategy,
	dependencies,
	factorSelections,
	running = false,
}: {
	snapshotId?: string;
	strategy?: LibraryComponent;
	dependencies: readonly Dependency[];
	factorSelections: Record<string, string>;
	running?: boolean;
}) {
	if (running) return "A Backtest is already running.";
	if (!snapshotId) return "Select a Market Data Snapshot before continuing.";
	if (!strategy) return "Select a compatible Strategy Component before continuing.";
	if (dependencies.some((dependency) => !factorSelections[dependency.alias]))
		return "Select a matching Factor Component for every required dependency.";
	return undefined;
}
