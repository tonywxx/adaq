export type TauriInvoke = (
	command: string,
	args?: Record<string, unknown>,
) => Promise<unknown>;
