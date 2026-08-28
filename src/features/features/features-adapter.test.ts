import type {
	FeatureMaterializationRequest,
	FeaturePlanDraft,
} from "./features-types";
import { createFeaturesAdapter } from "./features-adapter";

test("Feature materialization sends the native payload at the command boundary", async () => {
	const calls: Array<{ command: string; args: Record<string, unknown> }> = [];
	const adapter = createFeaturesAdapter(async (command, args = {}) => {
		calls.push({ command, args });
		return {};
	});
	const request = {} as FeatureMaterializationRequest;
	const plan = {} as FeaturePlanDraft;

	await adapter.startMaterialization("user-1", request, plan);

	expect(calls).toHaveLength(2);
	expect(calls[0]).toEqual({
		command: "research_context_freeze",
		args: {
			userId: "user-1",
			operationId: expect.any(String),
			stage: "features",
		},
	});
	expect(calls[1]).toEqual({
		command: "feature_materialization_start",
		args: {
			userId: "user-1",
			operationId: expect.any(String),
			request,
			plan,
		},
	});
});
