import { getVersion } from "@tauri-apps/api/app";
import { listen } from "@tauri-apps/api/event";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { DownloadIcon, RefreshCwIcon } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";

const AUTO_UPDATE_DEBUG = false;
const AUTO_DOWNLOAD_DELAY_MS = AUTO_UPDATE_DEBUG ? 50000 : 5000;
const CHECK_FOR_UPDATES_EVENT = "adaq-check-for-updates";

type UpdateStatus =
	| "idle"
	| "checking"
	| "available"
	| "downloading"
	| "ready"
	| "error";

type DownloadProgress = {
	downloaded: number;
	contentLength: number | null;
	percent: number | null;
};

const initialDownloadProgress: DownloadProgress = {
	downloaded: 0,
	contentLength: null,
	percent: null,
};

function isTauriRuntime() {
	return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

async function alertLatestVersion() {
	try {
		const appVersion = await getVersion();
		toast.success(`You are using the latest version v${appVersion}.`);
	} catch (error) {
		console.error("Failed to read app version", error);
		toast.success("You are using the latest version.");
	}
}

function alertUpdateCheckFailed() {
	toast.error("Unable to check for updates. Please try again later.");
}

async function runWindowAction(action: () => Promise<void>) {
	if (!isTauriRuntime()) {
		return;
	}

	try {
		await action();
	} catch (error) {
		console.error("Window action failed", error);
	}
}

function getProgressPercent(downloaded: number, contentLength: number | null) {
	if (!contentLength || contentLength <= 0) {
		return null;
	}

	return Math.min(100, Math.round((downloaded / contentLength) * 100));
}

function getUpdateButtonLabel(
	status: UpdateStatus,
	version: string | null,
	progress: DownloadProgress,
) {
	switch (status) {
		case "available":
			return version ? `Update to v${version}` : "Update";
		case "downloading":
			return progress.percent === null
				? "Downloading"
				: `${progress.percent}% Downloading...`;
		case "ready":
			return "Restart to update";
		case "error":
			return "Retry update";
		default:
			return null;
	}
}

export function AutoUpdateButton() {
	const [updateStatus, setUpdateStatus] = useState<UpdateStatus>("idle");
	const [updateVersion, setUpdateVersion] = useState<string | null>(null);
	const [downloadProgress, setDownloadProgress] = useState<DownloadProgress>(
		initialDownloadProgress,
	);
	const pendingUpdateRef = useRef<Update | null>(null);
	const downloadStartedRef = useRef(false);
	const autoDownloadTimerRef = useRef<number | null>(null);
	const updateCheckRequestRef = useRef(0);

	const clearAutoDownloadTimer = useCallback(() => {
		if (autoDownloadTimerRef.current === null) {
			return;
		}

		window.clearTimeout(autoDownloadTimerRef.current);
		autoDownloadTimerRef.current = null;
	}, []);

	const startUpdateDownload = useCallback(async () => {
		if (!isTauriRuntime() || downloadStartedRef.current) {
			return;
		}

		const pendingUpdate = pendingUpdateRef.current;

		if (!pendingUpdate) {
			return;
		}

		clearAutoDownloadTimer();
		downloadStartedRef.current = true;
		setUpdateStatus("downloading");
		setDownloadProgress(initialDownloadProgress);

		let downloaded = 0;
		let contentLength: number | null = null;

		try {
			await pendingUpdate.downloadAndInstall((event) => {
				switch (event.event) {
					case "Started":
						contentLength = event.data.contentLength ?? null;
						setDownloadProgress({
							downloaded: 0,
							contentLength,
							percent: getProgressPercent(0, contentLength),
						});
						break;
					case "Progress":
						downloaded += event.data.chunkLength;
						setDownloadProgress({
							downloaded,
							contentLength,
							percent: getProgressPercent(downloaded, contentLength),
						});
						break;
					case "Finished":
						setDownloadProgress({
							downloaded: contentLength ?? downloaded,
							contentLength,
							percent: contentLength ? 100 : null,
						});
						break;
					default:
						break;
				}
			});

			pendingUpdateRef.current = null;
			setUpdateStatus("ready");
		} catch (error) {
			console.error("Update download failed", error);
			downloadStartedRef.current = false;
			setUpdateStatus("error");
		}
	}, [clearAutoDownloadTimer]);

	const checkForUpdates = useCallback(
		async ({
			autoDownload,
			notifyNoUpdate,
			notifyError,
		}: {
			autoDownload: boolean;
			notifyNoUpdate: boolean;
			notifyError: boolean;
		}) => {
			if (!isTauriRuntime()) {
				return;
			}

			const requestId = updateCheckRequestRef.current + 1;
			updateCheckRequestRef.current = requestId;

			clearAutoDownloadTimer();
			downloadStartedRef.current = false;
			pendingUpdateRef.current = null;
			setUpdateVersion(null);
			setDownloadProgress(initialDownloadProgress);
			setUpdateStatus("checking");

			try {
				const update = await check({ timeout: 30000 });

				if (updateCheckRequestRef.current !== requestId) {
					return;
				}

				if (!update) {
					setUpdateStatus("idle");

					if (notifyNoUpdate) {
						void alertLatestVersion();
					}

					return;
				}

				pendingUpdateRef.current = update;
				setUpdateVersion(update.version);
				setUpdateStatus("available");

				if (autoDownload) {
					autoDownloadTimerRef.current = window.setTimeout(() => {
						void startUpdateDownload();
					}, AUTO_DOWNLOAD_DELAY_MS);
				}
			} catch (error) {
				if (updateCheckRequestRef.current !== requestId) {
					return;
				}

				console.error("Update check failed", error);
				setUpdateStatus("idle");

				if (notifyError) {
					alertUpdateCheckFailed();
				}
			}
		},
		[clearAutoDownloadTimer, startUpdateDownload],
	);

	useEffect(() => {
		if (!isTauriRuntime() || AUTO_UPDATE_DEBUG) {
			return;
		}

		void checkForUpdates({
			autoDownload: true,
			notifyNoUpdate: false,
			notifyError: false,
		});

		return () => {
			updateCheckRequestRef.current += 1;
			clearAutoDownloadTimer();
		};
	}, [checkForUpdates, clearAutoDownloadTimer]);

	useEffect(() => {
		if (!isTauriRuntime()) {
			return;
		}

		let disposed = false;
		let unlisten: (() => void) | null = null;

		listen(CHECK_FOR_UPDATES_EVENT, () => {
			void checkForUpdates({
				autoDownload: true,
				notifyNoUpdate: true,
				notifyError: true,
			});
		})
			.then((cleanup) => {
				if (disposed) {
					cleanup();
					return;
				}

				unlisten = cleanup;
			})
			.catch((error) => {
				console.error("Failed to listen for update menu event", error);
			});

		return () => {
			disposed = true;
			unlisten?.();
		};
	}, [checkForUpdates]);

	const updateButtonLabel = getUpdateButtonLabel(
		updateStatus,
		updateVersion,
		downloadProgress,
	);
	const isDownloadingUpdate = updateStatus === "downloading";
	const isRestartReady = updateStatus === "ready";

	const handleUpdateButtonClick = () => {
		if (isRestartReady) {
			void runWindowAction(() => relaunch());
			return;
		}

		if (updateStatus === "idle") {
			void checkForUpdates({
				autoDownload: false,
				notifyNoUpdate: true,
				notifyError: true,
			});
			return;
		}

		void startUpdateDownload();
	};

	if (!updateButtonLabel && !AUTO_UPDATE_DEBUG) {
		return null;
	}

	return (
		<button
			type="button"
			aria-label={updateButtonLabel ?? "Check for Updates"}
			title={updateButtonLabel ?? "Check for Updates"}
			disabled={isDownloadingUpdate}
			className="relative flex h-8 max-w-[190px] shrink-0 items-center justify-center overflow-hidden rounded-md border border-sidebar-border bg-background/70 px-2.5 text-xs font-medium text-foreground shadow-sm transition-colors hover:bg-background disabled:cursor-default disabled:opacity-85"
			onClick={(event) => {
				event.stopPropagation();
				handleUpdateButtonClick();
			}}
			onPointerDown={(event) => event.stopPropagation()}
		>
			{isDownloadingUpdate && downloadProgress.percent !== null ? (
				<span
					aria-hidden="true"
					className="absolute inset-y-0 left-0 bg-black/10 transition-[width]"
					style={{ width: `${downloadProgress.percent}%` }}
				/>
			) : null}
			<span className="relative z-10 flex min-w-0 items-center gap-1.5">
				{isRestartReady ? (
					<RefreshCwIcon className="size-3.5 shrink-0" />
				) : (
					<DownloadIcon className="size-3.5 shrink-0" />
				)}
				<span className="truncate">
					{updateButtonLabel ?? "Check for Updates"}
				</span>
			</span>
		</button>
	);
}
