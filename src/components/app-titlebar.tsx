import { getVersion } from "@tauri-apps/api/app";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";
import {
	ArrowLeftIcon,
	ArrowRightIcon,
	DownloadIcon,
	MoreHorizontalIcon,
	RefreshCwIcon,
} from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";

import { SidebarTrigger } from "@/components/ui/sidebar";

const AUTO_DOWNLOAD_DELAY_MS = 5000;
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

const appWindow = isTauriRuntime() ? getCurrentWindow() : null;

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
			return version ? `Update to ${version}` : "Update";
		case "downloading":
			return progress.percent === null
				? "Downloading"
				: `Downloading ${progress.percent}%`;
		case "ready":
			return "Relaunch";
		case "error":
			return "Retry update";
		default:
			return null;
	}
}

export function AppTitlebar() {
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
		if (!isTauriRuntime()) {
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

	const startDrag = () => {
		if (!appWindow) {
			return;
		}

		void runWindowAction(() => appWindow.startDragging());
	};

	const toggleMaximize = () => {
		if (!appWindow) {
			return;
		}

		void runWindowAction(async () => {
			if (await appWindow.isMaximized()) {
				await appWindow.unmaximize();
				return;
			}

			await appWindow.maximize();
		});
	};

	const updateButtonLabel = getUpdateButtonLabel(
		updateStatus,
		updateVersion,
		downloadProgress,
	);
	const isDownloadingUpdate = updateStatus === "downloading";
	const isRelaunchReady = updateStatus === "ready";

	const handleUpdateButtonClick = () => {
		if (isRelaunchReady) {
			void runWindowAction(() => relaunch());
			return;
		}

		void startUpdateDownload();
	};

	return (
		<div
			data-tauri-drag-region
			className="fixed inset-x-0 top-0 z-50 flex h-12 select-none bg-sidebar"
			onDoubleClick={toggleMaximize}
			onPointerDown={(event) => {
				if (event.button === 0) {
					startDrag();
				}
			}}
		>
			<div className="flex h-full shrink-0 items-center gap-3 overflow-hidden bg-sidebar pl-22 pr-4 transition-[width,border-color] duration-200 ease-linear w-(--titlebar-sidebar-collapsed-width) border-b">
				<div className="flex items-center gap-3 bg-sidebar">
					<SidebarTrigger
						className="size-5 bg-transparent text-black/45 hover:bg-black/5 hover:text-black/60 [&_svg]:size-3.5"
						onPointerDown={(event) => event.stopPropagation()}
					/>
					<button
						type="button"
						aria-label="Back"
						title="Back"
						disabled
						className="flex size-4 items-center justify-center text-black/35 disabled:cursor-default"
						onPointerDown={(event) => event.stopPropagation()}
					>
						<ArrowLeftIcon className="size-4 stroke-[1.6]" />
					</button>
					<button
						type="button"
						aria-label="Forward"
						title="Forward"
						disabled
						className="flex size-4 items-center justify-center text-black/25 disabled:cursor-default"
						onPointerDown={(event) => event.stopPropagation()}
					>
						<ArrowRightIcon className="size-4 stroke-[1.6]" />
					</button>
				</div>
			</div>

			<div className="flex h-full min-w-0 flex-1 items-center rounded-tl-[12px] border-b border-black/10 px-2 bg-sidebar">
				<div className="flex items-center gap-3 bg-sidebar">
					{/* <img src="/adaq.svg" alt="AdaQ" className="size-8" /> */}
					<h1 className="min-w-0 flex-1 truncate text-[22px] font-semibold leading-none tracking-normal text-[#242629]">
						AdaQ
					</h1>
				</div>
				<div className="ml-auto flex items-center gap-2 bg-sidebar">
					{updateButtonLabel ? (
						<button
							type="button"
							aria-label={updateButtonLabel}
							title={updateButtonLabel}
							disabled={isDownloadingUpdate}
							className="relative flex h-8 max-w-[190px] shrink-0 items-center justify-center overflow-hidden rounded-md border border-black/10 bg-white/70 px-2.5 text-xs font-medium text-[#242629] shadow-sm transition-colors hover:bg-white disabled:cursor-default disabled:opacity-85"
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
								{isRelaunchReady ? (
									<RefreshCwIcon className="size-3.5 shrink-0" />
								) : (
									<DownloadIcon className="size-3.5 shrink-0" />
								)}
								<span className="truncate">{updateButtonLabel}</span>
							</span>
						</button>
					) : null}
					<button
						type="button"
						aria-label="More"
						title="More"
						className="flex size-8 shrink-0 items-center justify-center rounded-md text-black/35 hover:bg-black/5 hover:text-black/55"
						onClick={(event) => event.stopPropagation()}
						onPointerDown={(event) => event.stopPropagation()}
					>
						<MoreHorizontalIcon className="size-4" />
					</button>
				</div>
			</div>
		</div>
	);
}
