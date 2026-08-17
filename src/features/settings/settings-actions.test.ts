import { createSettingsActions } from "./settings-actions";

test("settings actions keep reset command payloads explicit", async () => {
	const calls: Array<{
		command: string;
		args?: Record<string, unknown>;
	}> = [];
	const actions = createSettingsActions(async (command, args) => {
		calls.push({ command, args });
	});

	await actions.resetLocalData("user-1", "components");
	await actions.resetFactorResearch();
	await actions.getLocalDataSummary("user-1");

	expect(calls).toEqual([
		{
			command: "local_data_reset",
			args: { request: { userId: "user-1", kind: "components" } },
		},
		{ command: "factor_research_device_reset" },
		{ command: "local_data_summary", args: { request: { userId: "user-1" } } },
	]);
});
