import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState, type ReactNode } from "react";
import { PageLoadingSkeleton } from "@/components/page-loading-skeleton";

export function WorkspaceReadyBoundary({ children }: { children: ReactNode }) {
	const [ready, setReady] = useState(false);
	const [error, setError] = useState<string | null>(null);

	useEffect(() => {
		let active = true;
		if (!("__TAURI_INTERNALS__" in window)) {
			setReady(true);
			return () => {
				active = false;
			};
		}

		void invoke("workspace_ready")
			.then(() => {
				if (active) setReady(true);
			})
			.catch((reason: unknown) => {
				if (active)
					setError(reason instanceof Error ? reason.message : String(reason));
			});
		return () => {
			active = false;
		};
	}, []);

	if (error) {
		return (
			<main
				className="grid min-h-full place-content-center gap-2 p-6"
				role="alert"
			>
				<p className="font-semibold">Workspace initialization failed</p>
				<p className="text-sm text-muted-foreground">{error}</p>
			</main>
		);
	}

	return ready ? children : <PageLoadingSkeleton />;
}
