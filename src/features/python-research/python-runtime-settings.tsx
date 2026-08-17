import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "@/components/ui/card";
import { isTauriRuntime } from "@/lib/http";
import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

type RuntimeProfile = {
	profile: string;
	platform?: string;
	status: string;
	preparationStatus?: string;
	preparationAttemptId?: string;
	preparationDiagnostic?: string;
	preparationCompletedBytes?: number;
	preparationTotalBytes?: number;
	expectedVersion: string;
	source: string;
	artifactSha256?: string;
	downloadBytes?: number;
	installedBytes?: number;
	license?: string;
	wheelhouseIdentity?: string;
	wheelhouseStatus: string;
	wheelhouseWheelCount: number;
	runtimeCacheBytes: number;
	wheelhouseDiskBytes: number;
	environmentCacheBytes: number;
	environmentCount: number;
};

export function PythonRuntimeSettings({ userId }: { userId?: string }) {
	const { t } = useTranslation();
	const [profile, setProfile] = useState<RuntimeProfile>();
	const [busy, setBusy] = useState(false);
	const [preparationTaskId, setPreparationTaskId] = useState<string>();
	const [error, setError] = useState("");
	const [removed, setRemoved] = useState(0);
	const refreshVersion = useRef(0);

	const refresh = useCallback(async () => {
		if (!isTauriRuntime()) return;
		const version = ++refreshVersion.current;
		try {
			const next = await invoke<RuntimeProfile>("runtime_profile", {
				request: { userId },
			});
			if (version !== refreshVersion.current) return;
			setProfile(next);
		} catch (reason) {
			if (version !== refreshVersion.current) return;
			setError(String(reason));
		}
	}, [userId]);

	useEffect(() => {
		void refresh();
	}, [refresh]);

	useEffect(() => {
		if (!preparationTaskId) return;
		const timer = window.setInterval(() => void refresh(), 500);
		return () => window.clearInterval(timer);
	}, [preparationTaskId, refresh]);

	const evict = async () => {
		setBusy(true);
		setError("");
		try {
			const result = await invoke<{
				runtimes: string[];
				wheelhouses: string[];
				environments: string[];
			}>("cache_evict", {
				request: {
					activeRuntimeArtifacts: profile?.artifactSha256
						? [profile.artifactSha256]
						: [],
					activeWheelhouses: profile?.wheelhouseIdentity
						? [profile.wheelhouseIdentity]
						: [],
					activeEnvironments: [],
				},
			});
			setRemoved(
				result.runtimes.length +
					result.wheelhouses.length +
					result.environments.length,
			);
			await refresh();
		} catch (reason) {
			setError(String(reason));
		} finally {
			setBusy(false);
		}
	};

	const prepare = async () => {
		if (!userId) return;
		setBusy(true);
		setError("");
		setProfile((current) =>
			current
				? { ...current, status: "preparing", preparationStatus: "preparing" }
				: current,
		);
		const taskId = crypto.randomUUID();
		setPreparationTaskId(taskId);
		try {
			const sourceAttemptId =
				profile?.preparationAttemptId &&
				(profile.status === "failed" || profile.status === "cancelled")
					? profile.preparationAttemptId
					: undefined;
			await invoke("runtime_prepare_managed", {
				request: {
					userId,
					taskId,
					...(sourceAttemptId ? { sourceAttemptId } : {}),
				},
			});
			await refresh();
		} catch (reason) {
			setError(String(reason));
		} finally {
			setPreparationTaskId(undefined);
			setBusy(false);
		}
	};

	const cancelPreparation = async () => {
		if (!preparationTaskId) return;
		try {
			await invoke("runtime_prepare_cancel", {
				request: { taskId: preparationTaskId },
			});
		} catch (reason) {
			setError(String(reason));
		}
	};

	return (
		<Card>
			<CardHeader>
				<div className="flex flex-wrap items-start justify-between gap-3">
					<div>
						<CardTitle>{t("pythonResearch.runtime.title")}</CardTitle>
						<CardDescription>
							{t("pythonResearch.runtime.description")}
						</CardDescription>
					</div>
					<div className="flex flex-wrap gap-2">
						{preparationTaskId ? (
							<Button
								type="button"
								size="sm"
								variant="outline"
								onClick={() => void cancelPreparation()}
							>
								{t("pythonResearch.runtime.cancel")}
							</Button>
						) : null}
						<Button
							type="button"
							size="sm"
							onClick={() => void prepare()}
							loading={busy}
							disabled={!isTauriRuntime() || !userId || profile?.status === "ready"}
						>
							{profile?.status === "failed" || profile?.status === "cancelled"
								? t("pythonResearch.runtime.retry")
								: t("pythonResearch.runtime.prepare")}
						</Button>
						<Button
							type="button"
							variant="outline"
							size="sm"
							onClick={() => void evict()}
							loading={busy}
							disabled={!isTauriRuntime()}
						>
							{t("pythonResearch.runtime.evict")}
						</Button>
					</div>
				</div>
			</CardHeader>
			<CardContent className="grid gap-2 text-sm">
				{error ? (
					<p className="text-destructive" role="alert">
						{error}
					</p>
				) : null}
				{profile ? (
					<>
						<div className="flex flex-wrap items-center gap-2" role="status">
							<code>{profile.profile}</code>
							<Badge variant="outline">{profile.status}</Badge>
							<Badge variant="outline">{profile.wheelhouseStatus}</Badge>
							<span className="text-muted-foreground">
								{profile.platform ?? t("pythonResearch.runtime.platformUnknown")}
							</span>
						</div>
						<p>
							{t("pythonResearch.runtime.version", {
								version: profile.expectedVersion,
							})}
						</p>
						<p className="text-muted-foreground">{profile.source}</p>
						{profile.license ? <p>{profile.license}</p> : null}
						{profile.downloadBytes ? (
							<p>
								{t("pythonResearch.runtime.download", {
									bytes: profile.downloadBytes,
								})}
							</p>
						) : null}
						{profile.installedBytes ? (
							<p>
								{t("pythonResearch.runtime.installed", {
									bytes: profile.installedBytes,
								})}
							</p>
						) : null}
						{profile.artifactSha256 ? (
							<p className="break-all font-mono text-xs">{profile.artifactSha256}</p>
						) : null}
						{profile.preparationAttemptId ? (
							<p className="break-all font-mono text-xs">
								{profile.preparationStatus}: {profile.preparationAttemptId}
							</p>
						) : null}
						{profile.preparationDiagnostic ? (
							<p className="text-destructive">{profile.preparationDiagnostic}</p>
						) : null}
						{profile.preparationTotalBytes ? (
							<div className="grid gap-1">
								<label htmlFor="python-runtime-progress">
									{t("pythonResearch.runtime.progress")}
								</label>
								<progress
									id="python-runtime-progress"
									max={profile.preparationTotalBytes}
									value={profile.preparationCompletedBytes ?? 0}
								/>
								<span className="text-muted-foreground">
									{profile.preparationCompletedBytes ?? 0} /{" "}
									{profile.preparationTotalBytes} bytes
								</span>
							</div>
						) : null}
						{profile.wheelhouseIdentity ? (
							<p className="break-all font-mono text-xs">
								{t("pythonResearch.runtime.wheelhouse", {
									count: profile.wheelhouseWheelCount,
								})}
								: {profile.wheelhouseIdentity}
							</p>
						) : null}
						<p className="text-muted-foreground">
							{t("pythonResearch.runtime.cacheUse", {
								runtime: profile.runtimeCacheBytes,
								wheelhouse: profile.wheelhouseDiskBytes,
								environments: profile.environmentCacheBytes,
							})}
						</p>
						<p className="text-muted-foreground">
							{t("pythonResearch.runtime.environments", {
								count: profile.environmentCount,
							})}
						</p>
					</>
				) : (
					<p role="status">{t("pythonResearch.runtime.loading")}</p>
				)}
				{removed ? (
					<p role="status">
						{t("pythonResearch.runtime.removed", { count: removed })}
					</p>
				) : null}
			</CardContent>
		</Card>
	);
}
