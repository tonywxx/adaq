import {
	deleteComponentPackage,
	formatComponentError,
	importComponentPackage,
	isComponentPackageImported,
	type LibraryComponent,
} from "./component-library";

const component = (lockedByRunIds: string[] = []): LibraryComponent => ({
	componentId: "11111111-1111-4111-8111-111111111111",
	version: "1.2.3",
	manifestSchemaVersion: "1.0.0",
	sdkVersion: "0.8.4",
	abiVersion: "1.0.0",
	name: "Momentum Strategy",
	kind: "strategy",
	archiveSha256: "a".repeat(64),
	wasmSha256: "b".repeat(64),
	parameters: [],
	featureSlots: [],
	outputNames: [],
	dependencies: [],
	warmupBars: 0,
	compatible: true,
	lockedByRunIds,
});

test("successful import refreshes the user-scoped Component Library", async () => {
	const invoke = jest.fn().mockResolvedValue(component());
	const refresh = jest.fn().mockResolvedValue([component()]);

	const imported = await importComponentPackage(
		"alice",
		[0, 1, 2],
		invoke,
		refresh,
	);

	expect(invoke).toHaveBeenCalledWith("component_import", {
		request: { userId: "alice", bytes: [0, 1, 2] },
	});
	expect(refresh).toHaveBeenCalledTimes(1);
	expect(imported.name).toBe("Momentum Strategy");
});

test("existing packages are detected by archive hash before import", async () => {
	const invoke = jest.fn().mockResolvedValue(true);

	await expect(
		isComponentPackageImported("alice", "a".repeat(64), invoke),
	).resolves.toBe(true);
	expect(invoke).toHaveBeenCalledWith("component_is_imported", {
		request: { userId: "alice", archiveSha256: "a".repeat(64) },
	});
});

test("deletion is confirmed and locked Components never reach the command", async () => {
	const invoke = jest.fn().mockResolvedValue(undefined);
	const refresh = jest.fn().mockResolvedValue([]);
	const confirmDelete = jest.fn().mockReturnValue(true);

	await expect(
		deleteComponentPackage(
			"alice",
			component(["run-123"]),
			invoke,
			refresh,
			confirmDelete,
		),
	).resolves.toBe("locked");
	expect(confirmDelete).not.toHaveBeenCalled();
	expect(invoke).not.toHaveBeenCalled();

	confirmDelete.mockReturnValueOnce(false);
	await expect(
		deleteComponentPackage("alice", component(), invoke, refresh, confirmDelete),
	).resolves.toBe("cancelled");
	expect(invoke).not.toHaveBeenCalled();

	confirmDelete.mockReturnValueOnce(true);
	await expect(
		deleteComponentPackage("alice", component(), invoke, refresh, confirmDelete),
	).resolves.toBe("deleted");
	expect(invoke).toHaveBeenCalledWith("component_delete", {
		request: { userId: "alice", archiveSha256: "a".repeat(64) },
	});
	expect(refresh).toHaveBeenCalledTimes(1);
});

test("error formatting preserves exact evidence and labels unknown failures", () => {
	const typed = {
		code: "manifest-hash-mismatch",
		cause: "expected abc, received def",
		componentId: "11111111-1111-4111-8111-111111111111",
	};
	const validation = formatComponentError(typed, "import");
	expect(validation.summary).toBe("Package validation failed.");
	expect(validation.details).toBe(JSON.stringify(typed, null, 2));

	const unknown = formatComponentError("socket disappeared", "load");
	expect(unknown.summary).toBe("The Component Library could not be loaded.");
	expect(unknown.details).toBe("socket disappeared");
});
