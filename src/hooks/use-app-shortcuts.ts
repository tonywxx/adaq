import { useEffect } from "react";

const MIN_ZOOM = 0.5;
const MAX_ZOOM = 2;
const ZOOM_STEP = 0.1;
const ZOOM_STORAGE_KEY = "adaq-zoom";

function getZoom() {
	const stored = Number.parseFloat(
		localStorage.getItem(ZOOM_STORAGE_KEY) ?? "",
	);
	if (!Number.isFinite(stored)) {
		return 1;
	}

	return Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, stored));
}

function setZoom(nextZoom: number) {
	const zoom = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, nextZoom));
	localStorage.setItem(ZOOM_STORAGE_KEY, String(zoom));
	document.documentElement.style.zoom = String(zoom);
}

export function useAppShortcuts() {
	useEffect(() => {
		setZoom(getZoom());

		const onKeyDown = (event: KeyboardEvent) => {
			if (!(event.metaKey || event.ctrlKey)) {
				return;
			}

			const key = event.key.toLowerCase();

			if (key === "r") {
				event.preventDefault();
				window.location.reload();
				return;
			}

			if (key === "=" || key === "+") {
				event.preventDefault();
				setZoom(getZoom() + ZOOM_STEP);
				return;
			}

			if (key === "-") {
				event.preventDefault();
				setZoom(getZoom() - ZOOM_STEP);
				return;
			}

			if (key === "0") {
				event.preventDefault();
				setZoom(1);
			}
		};

		window.addEventListener("keydown", onKeyDown);
		return () => window.removeEventListener("keydown", onKeyDown);
	}, []);
}
