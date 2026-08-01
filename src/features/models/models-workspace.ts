import type { LibraryComponent } from "@/features/components/component-library";

export function datasetGenerationRequest(
	userId: string,
	snapshotId: string,
	model: LibraryComponent,
	components: readonly LibraryComponent[],
	compatibleFactors: Readonly<Record<string, readonly string[]>>,
	modelParameters: Record<string, string> = Object.fromEntries(
		model.parameters.map((parameter) => [parameter.name, parameter.defaultValue]),
	),
) {
	return {
		userId,
		snapshotId,
		modelArchiveSha256: model.archiveSha256,
		modelParameters,
		factorInstances: model.dependencies.map((dependency) => {
			const factor = components.find(
				(item) =>
					item.kind === "factor" &&
					compatibleFactors[dependency.alias]?.includes(item.archiveSha256),
			);
			if (!factor)
				throw new Error(`Required Factor ${dependency.alias} is not available.`);
			return { alias: dependency.alias, archiveSha256: factor.archiveSha256 };
		}),
		seed: 0,
	};
}

export function formatModelError(error: unknown) {
	return error instanceof Error ? error.message : String(error);
}

export function datasetStatusSummary(statusCounts: Record<string, number>) {
	return Object.entries(statusCounts)
		.map(([status, count]) => `${status}: ${count}`)
		.join(", ");
}
