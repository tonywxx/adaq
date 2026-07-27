import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { useMarketSessionStore } from "@/lib/market-session";
import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";

export type LibraryComponent = {
	componentId: string;
	version: string;
	name: string;
	kind: "factor" | "strategy";
	archiveSha256: string;
	wasmSha256: string;
	parameters: Array<{
		name: string;
		parameterType: "decimal" | "integer" | "boolean" | "string";
		defaultValue: string;
		allowedValues: string[];
	}>;
	dependencies: Array<{ componentId: string; version: string; alias: string }>;
};

export function ComponentsPage() {
	const userId = useMarketSessionStore((state) => state.userId);
	const [items, setItems] = useState<LibraryComponent[]>([]);
	const [message, setMessage] = useState("");
	const refresh = async () => {
		if (!userId) return;
		try {
			setItems(await invoke("component_list", { request: { userId } }));
		} catch (error) {
			setMessage(String(error));
		}
	};
	useEffect(() => {
		if (!userId) return;
		void invoke<LibraryComponent[]>("component_list", { request: { userId } })
			.then(setItems)
			.catch((error) => setMessage(String(error)));
	}, [userId]);
	const importPackage = async (file?: File) => {
		if (!file || !userId) return;
		setMessage("Validating package…");
		try {
			await invoke("component_import", {
				request: {
					userId,
					bytes: Array.from(new Uint8Array(await file.arrayBuffer())),
				},
			});
			setMessage(`${file.name} imported.`);
			await refresh();
		} catch (error) {
			setMessage(String(error));
		}
	};
	const removePackage = async (archiveSha256: string) => {
		if (!userId) return;
		try {
			await invoke("component_delete", { request: { userId, archiveSha256 } });
			setMessage("Component removed from this User's Library.");
			await refresh();
		} catch (error) {
			setMessage(String(error));
		}
	};
	return (
		<Workspace
			title="Component Library"
			description="User-scoped, verified local Factor and Strategy packages."
		>
			<Card>
				<CardHeader>
					<CardTitle>Import .adaq</CardTitle>
					<CardDescription>
						Packages must contain manifest.json and component.wasm.
					</CardDescription>
				</CardHeader>
				<CardContent className="space-y-3">
					<input
						type="file"
						accept=".adaq,application/zip"
						onChange={(event) => void importPackage(event.target.files?.[0])}
					/>
					{message && (
						<p className="text-sm text-muted-foreground" aria-live="polite">
							{message}
						</p>
					)}
				</CardContent>
			</Card>
			<div className="grid gap-3 md:grid-cols-2">
				{items.map((item) => (
					<Card key={item.archiveSha256}>
						<CardHeader>
							<CardTitle>{item.name}</CardTitle>
							<CardDescription>
								{item.kind} · v{item.version}
							</CardDescription>
						</CardHeader>
						<CardContent className="space-y-3 font-mono text-xs text-muted-foreground">
							{item.componentId}
							<br />
							{item.wasmSha256}
							<div>
								<Button
									size="sm"
									variant="outline"
									onClick={() => void removePackage(item.archiveSha256)}
								>
									Delete
								</Button>
							</div>
						</CardContent>
					</Card>
				))}
				{items.length === 0 && (
					<p className="text-sm text-muted-foreground">
						No Components imported for this User.
					</p>
				)}
			</div>
		</Workspace>
	);
}

export function Workspace({
	title,
	description,
	children,
}: {
	title: string;
	description: string;
	children: React.ReactNode;
}) {
	return (
		<div className="flex flex-1 flex-col gap-5 p-4 lg:p-6">
			<div>
				<h1 className="text-2xl font-semibold">{title}</h1>
				<p className="text-sm text-muted-foreground">{description}</p>
			</div>
			{children}
		</div>
	);
}
