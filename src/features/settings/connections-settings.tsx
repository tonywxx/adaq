import { Card, CardContent } from "@/components/ui/card";
import { supabase } from "@/lib/supabase";
import type { User } from "@supabase/supabase-js";
import { invoke } from "@tauri-apps/api/core";
import { LoaderCircleIcon } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { ConnectionCard } from "./connection-card";
import {
	createConnectionsAdapter,
	type ProfileView,
	type Provider,
} from "./connections-adapter";

function isTauriRuntime() {
	return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

function useAuthUser() {
	const [user, setUser] = useState<User | null>(null);
	useEffect(() => {
		if (!supabase) return;
		void supabase.auth.getUser().then(({ data }) => setUser(data.user));
		const { data } = supabase.auth.onAuthStateChange((_event, session) =>
			setUser(session?.user ?? null),
		);
		return () => data.subscription.unsubscribe();
	}, []);
	return user;
}

export function ConnectionsSettings() {
	const { t } = useTranslation();
	const user = useAuthUser();
	const adapter = useMemo(() => createConnectionsAdapter(invoke), []);
	const [profiles, setProfiles] = useState<ProfileView[]>([]);
	const [loaded, setLoaded] = useState(false);

	const refresh = useCallback(async () => {
		if (!user) return;
		const list = await adapter.listProfiles(user.id);
		setProfiles(list);
	}, [adapter, user]);

	useEffect(() => {
		if (!isTauriRuntime()) return;
		void refresh()
			.catch(() => setProfiles([]))
			.finally(() => setLoaded(true));
	}, [refresh]);

	const profileFor = (provider: Provider) =>
		profiles.find((profile) => profile.provider === provider) ?? null;

	return (
		<>
			<div>
				<h1 className="text-xl font-semibold">{t("settings.connections.title")}</h1>
				<p className="text-sm text-muted-foreground">
					{t("settings.connections.description")}
				</p>
				<p className="mt-2 rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs text-amber-900 dark:text-amber-100">
					{t("settings.connections.credentialEntryHint")}
				</p>
			</div>
			<div className="mt-5 grid gap-5">
				{!isTauriRuntime() ? (
					<Card>
						<CardContent className="pt-6 text-sm text-muted-foreground">
							{t("settings.connections.requiresDesktop")}
						</CardContent>
					</Card>
				) : !loaded ? (
					<Card>
						<CardContent className="flex items-center gap-2 pt-6 text-sm text-muted-foreground">
							<LoaderCircleIcon className="size-4 animate-spin" />
							{t("settings.connections.loading")}
						</CardContent>
					</Card>
				) : (
					<ConnectionCard
						provider="okx_demo"
						profile={profileFor("okx_demo")}
						disabled={!user}
						userId={user?.id}
						adapter={adapter}
						onChanged={() => void refresh()}
					/>
				)}
			</div>
		</>
	);
}
