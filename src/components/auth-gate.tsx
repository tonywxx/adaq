import { getErrorMessage } from "@/lib/utils";
import { markStartup } from "@/lib/startup-timing";
import { isSupabaseConfigured, supabase } from "@/lib/supabase";
import type { Session } from "@supabase/supabase-js";
import { invoke } from "@tauri-apps/api/core";
import { lazy, Suspense, type ReactNode, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { LoaderCircleIcon } from "lucide-react";
import type { TFunction } from "i18next";
import { NavTitlebarTransparent } from "./nav-titlebar-transparent";

const AuthEntry = lazy(() => import("./auth-entry"));

export function AuthGate({
	children,
}: {
	children: (userId: string) => ReactNode;
}) {
	const { t } = useTranslation();
	const [session, setSession] = useState<Session | null>(null);
	const [loadingSession, setLoadingSession] = useState(true);
	const [hostAuthStatus, setHostAuthStatus] = useState<
		"idle" | "binding" | "bound" | "error"
	>("idle");
	const [hostAuthUserId, setHostAuthUserId] = useState<string>();
	const [hostAuthError, setHostAuthError] = useState<string>();

	useEffect(() => {
		if (!supabase) {
			markStartup("adaq:auth-session-ready");
			setLoadingSession(false);
			return;
		}

		supabase.auth.getSession().then(({ data }) => {
			markStartup("adaq:auth-session-ready");
			setSession(data.session);
			setLoadingSession(false);
		});

		const {
			data: { subscription },
		} = supabase.auth.onAuthStateChange((_event, nextSession) => {
			setSession(nextSession);
		});

		return () => subscription.unsubscribe();
	}, []);

	useEffect(() => {
		if (loadingSession) markStartup("adaq:auth-loading-visible");
	}, [loadingSession]);

	useEffect(() => {
		let disposed = false;
		if (!session) {
			setHostAuthStatus("idle");
			setHostAuthUserId(undefined);
			setHostAuthError(undefined);
			void invoke("auth_clear_session").catch(() => {});
			return () => {
				disposed = true;
			};
		}

		setHostAuthStatus("binding");
		setHostAuthUserId(undefined);
		setHostAuthError(undefined);
		void invoke<{ userId: string }>("auth_bind_session", {
			accessToken: session.access_token,
		})
			.then((context) => {
				if (disposed) return;
				if (context.userId !== session.user.id) {
					throw new Error("Host authentication user mismatch");
				}
				setHostAuthUserId(context.userId);
				setHostAuthStatus("bound");
				markStartup("adaq:host-auth-bound");
			})
			.catch((error) => {
				if (disposed) return;
				setHostAuthError(getErrorMessage(error));
				setHostAuthStatus("error");
			});

		return () => {
			disposed = true;
		};
	}, [session]);

	useEffect(() => {
		return () => {
			void invoke("auth_clear_session").catch(() => {});
		};
	}, []);

	if (!isSupabaseConfigured || !supabase) {
		return (
			<Suspense fallback={<AuthLoadingScreen t={t} />}>
				<AuthEntry mode="configured" />
			</Suspense>
		);
	}

	if (loadingSession) {
		return <AuthLoadingScreen t={t} />;
	}

	if (!session) {
		return (
			<Suspense fallback={<AuthLoadingScreen t={t} />}>
				<AuthEntry mode="sign-in" onSession={setSession} />
			</Suspense>
		);
	}

	if (hostAuthStatus !== "bound" || hostAuthUserId !== session.user.id) {
		return (
			<HostAuthLoading t={t} status={hostAuthStatus} error={hostAuthError} />
		);
	}

	if (!session.user.user_metadata.password_set_at) {
		return (
			<Suspense fallback={<AuthLoadingScreen t={t} />}>
				<AuthEntry mode="password" session={session} onSession={setSession} />
			</Suspense>
		);
	}

	return children(session.user.id);
}

function AuthLoadingScreen({ t }: { t: TFunction }) {
	return (
		<>
			<NavTitlebarTransparent />
			<main
				className="grid min-h-svh place-content-center justify-items-center gap-3 bg-background"
				role="status"
				aria-label={t("auth.initializingAria")}
			>
				<LoaderCircleIcon
					className="size-7 animate-spin text-primary"
					aria-hidden="true"
				/>
				<p className="font-semibold">AdaQ</p>
				<p className="text-sm text-muted-foreground">
					{t("auth.initializingWorkspace")}
				</p>
			</main>
		</>
	);
}

function HostAuthLoading({
	t,
	status,
	error,
}: {
	t: TFunction;
	status: "idle" | "binding" | "bound" | "error";
	error?: string;
}) {
	return (
		<>
			<NavTitlebarTransparent />
			<main
				className="grid min-h-svh place-content-center justify-items-center gap-3 bg-background"
				role={status === "error" ? "alert" : "status"}
				aria-live="polite"
			>
				{status === "error" ? (
					<>
						<p className="font-semibold">{t("auth.hostAuthenticationFailed")}</p>
						<p className="text-sm text-muted-foreground">
							{t("auth.hostAuthenticationFailedDescription")}
						</p>
						{error && (
							<p className="max-w-sm text-center text-xs text-destructive">{error}</p>
						)}
					</>
				) : (
					<>
						<LoaderCircleIcon
							className="size-7 animate-spin text-primary"
							aria-hidden="true"
						/>
						<p className="font-semibold">AdaQ</p>
						<p className="text-sm text-muted-foreground">
							{t("auth.initializingWorkspace")}
						</p>
					</>
				)}
			</main>
		</>
	);
}
