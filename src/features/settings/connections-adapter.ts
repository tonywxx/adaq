export type Provider = "alpaca_paper" | "okx_demo";
export type ProfileStatus = "usable" | "unusable";

export type ConnectionEvidence = {
	outcome: "success" | "failure";
	errorCode: string | null;
	redactedError: string | null;
	accountId: string | null;
	currency: string | null;
	accountStatus: string | null;
	serverTimeMs: number | null;
	clockSkewSeconds: number | null;
	capabilities: string[];
	requestedPaths: string[];
	checkedAtMs: number;
};

export type ProfileView = {
	profileId: string;
	provider: Provider;
	environment: string;
	maskedKeySuffix: string;
	accountId: string | null;
	currency: string | null;
	status: ProfileStatus;
	lastTestAtMs: number | null;
	lastTestEvidence: ConnectionEvidence | null;
	createdAtMs: number;
	updatedAtMs: number;
};

export type ProviderCredentials =
	| { provider: "alpaca_paper"; keyId: string; secretKey: string }
	| {
			provider: "okx_demo";
			apiKey: string;
			secretKey: string;
			passphrase: string;
	  };

import type { TauriInvoke } from "@/lib/tauri-invoke";

export type ConnectionsInvoke = TauriInvoke;

export function createConnectionsAdapter(invoke: ConnectionsInvoke) {
	return {
		listProfiles(userId: string) {
			return invoke("connection_profile_list", {
				request: { userId },
			}) as Promise<ProfileView[]>;
		},
		saveProfile(userId: string, credentials: ProviderCredentials) {
			return invoke("connection_profile_save", {
				request: { userId, credentials },
			}) as Promise<ProfileView>;
		},
		testProfile(userId: string, profileId: string) {
			return invoke("connection_profile_test", {
				request: { userId, profileId },
			}) as Promise<ProfileView>;
		},
		deleteProfile(userId: string, profileId: string) {
			return invoke("connection_profile_delete", {
				request: { userId, profileId },
			});
		},
	};
}

export type ConnectionsAdapter = ReturnType<typeof createConnectionsAdapter>;
