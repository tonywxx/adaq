import { getCurrentWindow } from "@tauri-apps/api/window";
import { ArrowLeftIcon, ArrowRightIcon } from "lucide-react";
import { AutoUpdateButton } from "@/components/ui/autoupdate-button";
import { DarkModeDropDownMenu } from "@/components/ui/dark-mode-dropdown-menu";
import { SidebarTrigger } from "@/components/ui/sidebar";
import { Button } from "./ui/button";

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

export function AppTitlebar() {
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
						className="size-5 bg-transparent text-sidebar-foreground/45 hover:bg-sidebar-accent hover:text-sidebar-foreground/70 [&_svg]:size-3.5"
						onPointerDown={(event) => event.stopPropagation()}
					/>
					<button
						type="button"
						aria-label="Back"
						title="Back"
						disabled
						className="flex size-4 items-center justify-center text-sidebar-foreground/35 disabled:cursor-default"
						onPointerDown={(event) => event.stopPropagation()}
					>
						<ArrowLeftIcon className="size-4 stroke-[1.6]" />
					</button>
					<button
						type="button"
						aria-label="Forward"
						title="Forward"
						disabled
						className="flex size-4 items-center justify-center text-sidebar-foreground/25 disabled:cursor-default"
						onPointerDown={(event) => event.stopPropagation()}
					>
						<ArrowRightIcon className="size-4 stroke-[1.6]" />
					</button>
				</div>
			</div>

			<div className="flex h-full min-w-0 flex-1 items-center rounded-tl-[12px] border-b border-sidebar-border px-2 bg-sidebar">
				<div className="flex items-center gap-3 bg-sidebar">
					{/* <img src="/adaq.svg" alt="AdaQ" className="size-8" /> */}
					<h1 className="min-w-0 flex-1 truncate text-[22px] font-semibold leading-none tracking-normal text-sidebar-foreground">
						{/* AdaQ */}
					</h1>
				</div>
				<div className="ml-auto flex items-center gap-2 bg-sidebar">
					<Button variant="outline" effect="shine">
						&nbsp;&nbsp;&nbsp;&nbsp;&nbsp; AdaQ&nbsp;&nbsp;&nbsp;&nbsp; &nbsp;
					</Button>
					<AutoUpdateButton />
					<DarkModeDropDownMenu />
					{/* <button
						type="button"
						aria-label="More"
						title="More"
						className="flex size-8 shrink-0 items-center justify-center rounded-md text-sidebar-foreground/35 hover:bg-sidebar-accent hover:text-sidebar-foreground/60"
						onClick={(event) => event.stopPropagation()}
						onPointerDown={(event) => event.stopPropagation()}
					>
						<MoreHorizontalIcon className="size-4" />
					</button> */}
				</div>
			</div>
		</div>
	);
}
