import type { TauriInvoke } from "@/lib/tauri-invoke";
import type { LocalDataSummary } from "./settings-types";

export type LocalResetKind = "watchlist" | "components" | "marketData" | "all";
export type ResetKind = LocalResetKind | "factorResearch";

export type SettingsInvoke = TauriInvoke;

export function createSettingsActions(invoke: SettingsInvoke) {
	return {
		resetLocalData(userId: string, kind: LocalResetKind) {
			return invoke("local_data_reset", { request: { userId, kind } });
		},
		resetFactorResearch() {
			return invoke("factor_research_device_reset");
		},
		getLocalDataSummary(userId: string) {
			return invoke("local_data_summary", {
				request: { userId },
			}) as Promise<LocalDataSummary>;
		},
	};
}

export type SettingsActions = ReturnType<typeof createSettingsActions>;
