const AUTO_DOWNLOAD_UPDATES_KEY = "adaq.autoDownloadUpdates";
export const LAST_APP_PATH_KEY = "adaq.lastAppPath";

export function autoDownloadUpdates() {
	return localStorage.getItem(AUTO_DOWNLOAD_UPDATES_KEY) !== "false";
}

export function setAutoDownloadUpdates(enabled: boolean) {
	localStorage.setItem(AUTO_DOWNLOAD_UPDATES_KEY, String(enabled));
}
