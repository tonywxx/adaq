import { createConnectionsAdapter } from "./connections-adapter";

test("connection adapter keeps profile command payloads explicit", async () => {
	const calls: Array<{
		command: string;
		args?: Record<string, unknown>;
	}> = [];
	const adapter = createConnectionsAdapter(async (command, args) => {
		calls.push({ command, args });
		return {};
	});

	await adapter.listProfiles("user-1");
	await adapter.saveProfile("user-1", {
		provider: "alpaca_paper",
		keyId: "demo-key",
		secretKey: "demo-secret",
	});
	await adapter.saveProfile("user-1", {
		provider: "okx_demo",
		apiKey: "demo-api-key",
		secretKey: "demo-secret",
		passphrase: "demo-passphrase",
	});
	await adapter.testProfile("user-1", "profile-1");
	await adapter.deleteProfile("user-1", "profile-1");

	expect(calls).toEqual([
		{
			command: "connection_profile_list",
			args: { request: { userId: "user-1" } },
		},
		{
			command: "connection_profile_save",
			args: {
				request: {
					userId: "user-1",
					credentials: {
						provider: "alpaca_paper",
						keyId: "demo-key",
						secretKey: "demo-secret",
					},
				},
			},
		},
		{
			command: "connection_profile_save",
			args: {
				request: {
					userId: "user-1",
					credentials: {
						provider: "okx_demo",
						apiKey: "demo-api-key",
						secretKey: "demo-secret",
						passphrase: "demo-passphrase",
					},
				},
			},
		},
		{
			command: "connection_profile_test",
			args: { request: { userId: "user-1", profileId: "profile-1" } },
		},
		{
			command: "connection_profile_delete",
			args: { request: { userId: "user-1", profileId: "profile-1" } },
		},
	]);
});
