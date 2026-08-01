export type LibraryComponent = {
	componentId: string;
	version: string;
	manifestSchemaVersion: string;
	sdkVersion: string;
	abiVersion: string;
	name: string;
	kind: "factor" | "strategy" | "model";
	archiveSha256: string;
	wasmSha256: string;
	parameters: Array<{
		name: string;
		parameterType: "decimal" | "integer" | "boolean" | "string";
		defaultValue: string;
		allowedValues: string[];
	}>;
	featureSlots: Array<{
		name: string;
		source: Record<string, unknown> & { kind: string };
	}>;
	outputNames: string[];
	dependencies: Array<{ componentId: string; version: string; alias: string }>;
	warmupBars: number;
	modelScope?: "single-instrument";
	modelOutputs?: Array<{
		name: string;
		predictionKind: Record<string, unknown> & { kind: string };
		forecastTarget: Record<string, unknown> & { kind: string };
		valueScale: Record<string, unknown> & { kind: string };
		horizonBars: number;
	}>;
	modelArtifact?: { sha256: string; provenance: Record<string, string> };
	compatible: boolean;
	compatibilityError?: string;
	lockedByRunIds: string[];
};

type ComponentInvoke = (
	command: string,
	args: Record<string, unknown>,
) => Promise<unknown>;

export async function importComponentPackage(
	userId: string,
	bytes: number[],
	invoke: ComponentInvoke,
	refresh: () => Promise<LibraryComponent[]>,
) {
	const imported = (await invoke("component_import", {
		request: { userId, bytes },
	})) as LibraryComponent;
	await refresh();
	return imported;
}

export async function deleteComponentPackage(
	userId: string,
	component: LibraryComponent,
	invoke: ComponentInvoke,
	refresh: () => Promise<LibraryComponent[]>,
	confirmDelete: (message: string) => boolean,
) {
	if (component.lockedByRunIds.length) return "locked" as const;
	if (
		!confirmDelete(
			`Delete ${component.name} v${component.version} from this User's Component Library?`,
		)
	)
		return "cancelled" as const;
	await invoke("component_delete", {
		request: { userId, archiveSha256: component.archiveSha256 },
	});
	await refresh();
	return "deleted" as const;
}

export function formatComponentError(
	error: unknown,
	action: "import" | "delete" | "load",
) {
	let details: string;
	if (typeof error === "string") details = error;
	else if (error instanceof Error) details = error.message;
	else {
		try {
			details = JSON.stringify(error, null, 2);
		} catch {
			details = String(error);
		}
	}
	const locked = details.includes("locked by Backtest Run");
	return {
		summary: locked
			? "This Component is locked by an immutable Backtest Run and cannot be removed."
			: action === "import"
				? "Package validation failed."
				: action === "load"
					? "The Component Library could not be loaded."
					: "The Component could not be removed.",
		details,
	};
}
