import { getCurrentWindow } from "@tauri-apps/api/window";
import { cn } from "@/lib/utils";

interface NavTitlebarTransparentProps
	extends React.HTMLAttributes<HTMLDivElement> {}

function isTauriRuntime() {
	return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

const appWindow = isTauriRuntime() ? getCurrentWindow() : null;

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

export function NavTitlebarTransparent({
	className,
	children,
	...props
}: NavTitlebarTransparentProps) {
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

	return (
		<div
			data-tauri-drag-region
			className={cn(
				"fixed top-0 left-0 right-0 z-50 flex h-12 w-full",
				className,
			)}
			style={{ WebkitAppRegion: "drag" } as React.CSSProperties}
			onPointerDown={(event) => {
				if (event.button === 0) {
					startDrag();
				}
			}}
			onDoubleClick={toggleMaximize}
			{...props}
		>
			{children}
		</div>
	);
}
