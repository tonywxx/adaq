import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "@/components/ui/card";
import { LoadingState } from "@/components/loading-state";
import {
	Pagination,
	PaginationContent,
	PaginationItem,
	PaginationNext,
	PaginationPrevious,
} from "@/components/ui/pagination";
import { useMarketSessionStore } from "@/lib/market-session";
import { invoke } from "@tauri-apps/api/core";
import { LoaderCircleIcon } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import {
	archiveSha256,
	deleteComponentPackage,
	formatComponentError,
	importComponentPackage,
	isComponentPackageImported,
	type LibraryComponent,
} from "./component-library";

export type { LibraryComponent } from "./component-library";

type Feedback = {
	tone: "success" | "error";
	summary: string;
	details?: string;
};
type ComponentPage = {
	items: LibraryComponent[];
	total: number;
	page: number;
	pageSize: number;
};

const COMPONENT_PAGE_SIZE = 10;

export function ComponentsPage() {
	const userId = useMarketSessionStore((state) => state.userId);
	const [items, setItems] = useState<LibraryComponent[]>([]);
	const [packagesPage, setPackagesPage] = useState(1);
	const [packagesTotal, setPackagesTotal] = useState(0);
	const [selectedHash, setSelectedHash] = useState("");
	const [importing, setImporting] = useState(false);
	const [deleting, setDeleting] = useState(false);
	const [importFeedback, setImportFeedback] = useState<Feedback>();
	const [deleteFeedback, setDeleteFeedback] = useState<Feedback>();
	const [loadFeedback, setLoadFeedback] = useState<Feedback>();
	const [packagesLoading, setPackagesLoading] = useState(true);
	const activeUserId = useRef(userId);
	const requestVersion = useRef(0);
	activeUserId.current = userId;
	const refresh = useCallback(
		async (page = packagesPage) => {
			if (!userId) return [];
			const requestedUserId = userId;
			const version = ++requestVersion.current;
			let result: ComponentPage;
			try {
				result = await invoke("component_page", {
					request: { userId: requestedUserId, page },
				});
			} catch (error) {
				if (
					version === requestVersion.current &&
					activeUserId.current === requestedUserId
				)
					throw error;
				return [];
			}
			const components = result.items;
			if (
				version !== requestVersion.current ||
				activeUserId.current !== requestedUserId
			)
				return [];
			setItems(components);
			setPackagesTotal(result.total);
			setSelectedHash((current) =>
				components.some((item) => item.archiveSha256 === current)
					? current
					: (components[0]?.archiveSha256 ?? ""),
			);
			setLoadFeedback(undefined);
			return components;
		},
		[packagesPage, userId],
	);

	useEffect(() => {
		requestVersion.current += 1;
		setItems([]);
		setPackagesPage(1);
		setPackagesTotal(0);
		setSelectedHash("");
		setImporting(false);
		setDeleting(false);
		setImportFeedback(undefined);
		setDeleteFeedback(undefined);
		setLoadFeedback(undefined);
		if (!userId) {
			setPackagesLoading(false);
			return;
		}
		let active = true;
		setPackagesLoading(true);
		void refresh()
			.catch((error) => {
				if (active)
					setLoadFeedback({
						tone: "error",
						...formatComponentError(error, "load"),
					});
			})
			.finally(() => {
				if (active) setPackagesLoading(false);
			});
		return () => {
			active = false;
		};
	}, [refresh, userId]);

	const importPackage = async (file?: File) => {
		if (!file || !userId) return;
		setImporting(true);
		setImportFeedback(undefined);
		try {
			const bytes = new Uint8Array(await file.arrayBuffer());
			const hash = await archiveSha256(bytes);
			if (
				await isComponentPackageImported(userId, hash, (command, args) =>
					invoke(command, args),
				)
			) {
				if (activeUserId.current !== userId) return;
				setSelectedHash(hash);
				setImportFeedback({
					tone: "success",
					summary: `${file.name} is already imported.`,
				});
				return;
			}
			const imported = await importComponentPackage(
				userId,
				Array.from(bytes),
				(command, args) => invoke(command, args),
				() => refresh(1),
			);
			if (activeUserId.current !== userId) return;
			setPackagesPage(1);
			setSelectedHash(imported.archiveSha256);
			setImportFeedback({
				tone: "success",
				summary: `${file.name} imported as ${imported.name} v${imported.version}.`,
				details: `Archive SHA-256: ${imported.archiveSha256}\nWASM SHA-256: ${imported.wasmSha256}`,
			});
		} catch (error) {
			setImportFeedback({
				tone: "error",
				...formatComponentError(error, "import"),
			});
		} finally {
			setImporting(false);
		}
	};

	const removePackage = async (component: LibraryComponent) => {
		if (!userId) return;
		setDeleting(true);
		setDeleteFeedback(undefined);
		try {
			const result = await deleteComponentPackage(
				userId,
				component,
				(command, args) => invoke(command, args),
				refresh,
				window.confirm,
			);
			if (activeUserId.current !== userId) return;
			if (result === "deleted") {
				setDeleteFeedback({
					tone: "success",
					summary: `${component.name} v${component.version} was removed from this User's Component Library.`,
				});
			} else if (result === "locked") {
				setDeleteFeedback({
					tone: "error",
					summary:
						"This Component is locked by an immutable Backtest Run and cannot be removed.",
					details: component.lockedByRunIds.join("\n"),
				});
			}
		} catch (error) {
			setDeleteFeedback({
				tone: "error",
				...formatComponentError(error, "delete"),
			});
		} finally {
			setDeleting(false);
		}
	};

	const selected = items.find((item) => item.archiveSha256 === selectedHash);

	return (
		<Workspace
			title="Component Library"
			description="Audit and manage this User's verified local Factor, Strategy, and Model packages."
		>
			<Card>
				<CardHeader>
					<CardTitle>Import Component Package</CardTitle>
					<CardDescription>
						Choose an immutable .adaq package containing manifest.json and
						component.wasm.
					</CardDescription>
				</CardHeader>
				<CardContent className="space-y-3">
					<label className="block text-sm font-medium" htmlFor="component-package">
						.adaq package
					</label>
					<input
						id="component-package"
						type="file"
						accept=".adaq"
						disabled={importing}
						className="block w-full rounded-md border bg-background p-2 text-sm file:mr-3 file:rounded-md file:border-0 file:bg-secondary file:px-3 file:py-1 file:text-foreground focus-visible:outline-none focus-visible:ring-3 focus-visible:ring-ring/50"
						onChange={(event) => void importPackage(event.target.files?.[0])}
					/>
					{importing && (
						<p className="flex items-center gap-2 text-sm" role="status">
							<LoaderCircleIcon
								className="size-4 animate-spin"
								aria-hidden="true"
							/>
							Validating package…
						</p>
					)}
					{importFeedback && <ActionFeedback feedback={importFeedback} />}
				</CardContent>
			</Card>

			{loadFeedback && <ActionFeedback feedback={loadFeedback} />}
			<div className="grid min-w-0 gap-4 lg:grid-cols-[minmax(16rem,22rem)_minmax(0,1fr)]">
				<Card className="min-w-0">
					<CardHeader>
						<CardTitle>Packages</CardTitle>
						<CardDescription>{packagesTotal} available to this User</CardDescription>
					</CardHeader>
					<CardContent className="flex flex-col gap-3">
						{packagesLoading ? (
							<LoadingState label="Loading Component Packages…" />
						) : items.length ? (
							<ul className="flex flex-col gap-2" aria-label="Component packages">
								{items.map((item) => (
									<li key={item.archiveSha256}>
										<button
											type="button"
											aria-current={selectedHash === item.archiveSha256}
											className="w-full rounded-lg border p-3 text-left focus-visible:outline-none focus-visible:ring-3 focus-visible:ring-ring/50 aria-current:border-ring aria-current:bg-muted"
											onClick={() => {
												setSelectedHash(item.archiveSha256);
												setDeleteFeedback(undefined);
											}}
										>
											<span className="block font-medium">{item.name}</span>
											<span className="text-xs text-muted-foreground">
												{item.kind} · v{item.version}
											</span>
											<span className="mt-2 flex flex-wrap gap-1">
												<CompatibilityBadge component={item} />
												<LockBadge component={item} />
											</span>
										</button>
									</li>
								))}
							</ul>
						) : (
							<div className="rounded-lg border border-dashed p-6 text-center">
								<p className="font-medium">No Component Packages</p>
								<p className="mt-1 text-sm text-muted-foreground">
									Import a verified .adaq package to begin.
								</p>
							</div>
						)}
						{!packagesLoading && packagesTotal > COMPONENT_PAGE_SIZE && (
							<Pagination>
								<PaginationContent>
									<PaginationItem>
										<PaginationPrevious
											disabled={packagesPage === 1}
											onClick={() => setPackagesPage((page) => page - 1)}
										/>
									</PaginationItem>
									<PaginationItem>
										<span className="px-3 text-sm" aria-current="page">
											Page {packagesPage} of{" "}
											{Math.ceil(packagesTotal / COMPONENT_PAGE_SIZE)}
										</span>
									</PaginationItem>
									<PaginationItem>
										<PaginationNext
											disabled={
												packagesPage >= Math.ceil(packagesTotal / COMPONENT_PAGE_SIZE)
											}
											onClick={() => setPackagesPage((page) => page + 1)}
										/>
									</PaginationItem>
								</PaginationContent>
							</Pagination>
						)}
					</CardContent>
				</Card>

				{selected ? (
					<ComponentDetail
						component={selected}
						deleting={deleting}
						feedback={deleteFeedback}
						onDelete={removePackage}
					/>
				) : (
					<Card className="min-w-0">
						<CardContent className="p-6 text-sm text-muted-foreground">
							Select a Component Package to inspect its exact contract.
						</CardContent>
					</Card>
				)}
			</div>
		</Workspace>
	);
}

function ComponentDetail({
	component,
	deleting,
	feedback,
	onDelete,
}: {
	component: LibraryComponent;
	deleting: boolean;
	feedback?: Feedback;
	onDelete: (component: LibraryComponent) => Promise<void>;
}) {
	const locked = component.lockedByRunIds.length > 0;
	return (
		<Card className="min-w-0">
			<CardHeader>
				<div className="flex flex-wrap items-start justify-between gap-3">
					<div>
						<CardTitle>{component.name}</CardTitle>
						<CardDescription>
							{component.kind} · v{component.version}
						</CardDescription>
					</div>
					<div className="flex flex-wrap gap-1">
						<CompatibilityBadge component={component} />
						<LockBadge component={component} />
					</div>
				</div>
			</CardHeader>
			<CardContent className="space-y-6">
				{component.architecture && (
					<DetailSection title="Architecture">
						<p className="text-sm">{component.architecture}</p>
						<p className="text-sm text-muted-foreground">
							Derived from authoritative Feature Slot sources.
						</p>
					</DetailSection>
				)}
				{!component.compatible && (
					<div
						className="rounded-lg border border-destructive/40 bg-destructive/10 p-3"
						role="alert"
					>
						<p className="font-medium">Incompatible Component Package</p>
						<p className="mt-1 text-sm">
							This package cannot be executed by this host.
						</p>
						<TechnicalDetails
							details={
								component.compatibilityError ??
								"Compatibility validation failed without details."
							}
						/>
					</div>
				)}

				<DetailSection title="Identity and versions">
					<dl className="grid gap-3 sm:grid-cols-2">
						<Detail label="Component ID" value={component.componentId} mono />
						<Detail label="Version" value={component.version} />
						<Detail
							label="Manifest schema"
							value={component.manifestSchemaVersion || "Unavailable"}
						/>
						<Detail label="ABI" value={component.abiVersion || "Unavailable"} />
						<Detail label="SDK" value={component.sdkVersion || "Unavailable"} />
						<Detail label="Warmup" value={`${component.warmupBars} Closed Bars`} />
					</dl>
					{component.modelScope && (
						<p className="mt-3 text-sm text-muted-foreground">
							Model scope: Single Instrument
						</p>
					)}
				</DetailSection>

				<DetailSection title="Exact hashes">
					<div className="space-y-3">
						<HashValue label="Archive SHA-256" value={component.archiveSha256} />
						<HashValue label="WASM SHA-256" value={component.wasmSha256} />
					</div>
				</DetailSection>

				<DetailSection title="Parameters">
					{component.parameters.length ? (
						<ul className="space-y-2">
							{component.parameters.map((parameter) => (
								<li className="rounded-md border p-3 text-sm" key={parameter.name}>
									<p className="font-medium">{parameter.name}</p>
									<p className="text-muted-foreground">
										{parameter.parameterType} · default {parameter.defaultValue}
										{parameter.allowedValues.length
											? ` · allowed ${parameter.allowedValues.join(", ")}`
											: ""}
									</p>
								</li>
							))}
						</ul>
					) : (
						<EmptyContract label="No parameters declared." />
					)}
				</DetailSection>

				<DetailSection title="Feature Slots">
					{component.featureSlots.length ? (
						<ol className="space-y-2">
							{component.featureSlots.map((slot) => (
								<li className="rounded-md border p-3" key={slot.name}>
									<p className="text-sm font-medium">{slot.name}</p>
									<pre className="mt-2 overflow-x-auto whitespace-pre-wrap break-words text-xs text-muted-foreground">
										{JSON.stringify(slot.source, null, 2)}
									</pre>
								</li>
							))}
						</ol>
					) : (
						<EmptyContract label="No Feature Slots declared." />
					)}
				</DetailSection>

				<DetailSection title="Factor dependencies">
					{component.dependencies.length ? (
						<ul className="space-y-2">
							{component.dependencies.map((dependency) => (
								<li className="rounded-md border p-3 text-sm" key={dependency.alias}>
									<p className="font-medium">{dependency.alias}</p>
									<p className="break-all font-mono text-xs text-muted-foreground">
										{dependency.componentId} · {dependency.version}
									</p>
								</li>
							))}
						</ul>
					) : (
						<EmptyContract label="No external Factor dependencies declared." />
					)}
				</DetailSection>

				<DetailSection title="Outputs">
					{component.modelOutputs?.length ? (
						<ul className="space-y-2">
							{component.modelOutputs.map((output) => (
								<li className="rounded-md border p-3 text-sm" key={output.name}>
									<p className="font-medium">{output.name}</p>
									<p className="text-muted-foreground">
										{readableModelKind(output.predictionKind.kind)} · {output.horizonBars}{" "}
										Bar horizon
									</p>
									<p className="text-muted-foreground">
										Target:{" "}
										{readableModelKind(
											String(
												output.forecastTarget.target ??
													output.forecastTarget.id ??
													output.forecastTarget.kind,
											),
										)}{" "}
										· Scale: {readableModelKind(String(output.valueScale.kind))}
									</p>
									<pre className="mt-2 overflow-x-auto whitespace-pre-wrap break-words text-xs text-muted-foreground">
										{JSON.stringify(
											{
												predictionKind: output.predictionKind,
												target: output.forecastTarget,
												scale: output.valueScale,
											},
											null,
											2,
										)}
									</pre>
								</li>
							))}
						</ul>
					) : (
						<p className="text-sm text-muted-foreground">
							{component.outputNames.length
								? component.outputNames.join(", ")
								: "No named outputs declared."}
						</p>
					)}
				</DetailSection>

				{component.modelArtifact && (
					<DetailSection title="Embedded Model Artifact">
						<HashValue
							label="Artifact SHA-256"
							value={component.modelArtifact.sha256}
						/>
						<pre className="mt-3 overflow-x-auto whitespace-pre-wrap break-words text-xs text-muted-foreground">
							{JSON.stringify(component.modelArtifact.provenance, null, 2)}
						</pre>
					</DetailSection>
				)}

				<DetailSection title="Run-lock state">
					{locked ? (
						<div className="space-y-2">
							<p className="text-sm font-medium">
								Locked — historical evidence references this exact package.
							</p>
							<ul className="space-y-1 font-mono text-xs">
								{component.lockedByRunIds.map((runId) => (
									<li className="break-all" key={runId}>
										Backtest Run {runId}
									</li>
								))}
							</ul>
						</div>
					) : (
						<p className="text-sm text-muted-foreground">
							Unlocked — no historical Backtest Run for this User references this
							package.
						</p>
					)}
				</DetailSection>

				<div className="space-y-3 border-t pt-4">
					<Button
						variant="destructive"
						disabled={locked}
						loading={deleting}
						loadingText="Removing…"
						onClick={() => void onDelete(component)}
					>
						Delete Component Package
					</Button>
					{locked && (
						<p className="text-sm text-muted-foreground">
							Deletion is unavailable because the Backtest Run reference above must
							remain reproducible.
						</p>
					)}
					{feedback && <ActionFeedback feedback={feedback} />}
				</div>
			</CardContent>
		</Card>
	);
}

function readableModelKind(value: string) {
	return value
		.replace(/-/g, " ")
		.replace(/\b\w/g, (letter) => letter.toUpperCase());
}

function CompatibilityBadge({ component }: { component: LibraryComponent }) {
	return (
		<Badge variant={component.compatible ? "secondary" : "destructive"}>
			{component.compatible ? "Compatible" : "Incompatible"}
		</Badge>
	);
}

function LockBadge({ component }: { component: LibraryComponent }) {
	return (
		<Badge variant={component.lockedByRunIds.length ? "outline" : "secondary"}>
			{component.lockedByRunIds.length ? "Locked by Run" : "Unlocked"}
		</Badge>
	);
}

function DetailSection({
	title,
	children,
}: {
	title: string;
	children: React.ReactNode;
}) {
	return (
		<section className="space-y-2">
			<h2 className="text-sm font-semibold">{title}</h2>
			{children}
		</section>
	);
}

function Detail({
	label,
	value,
	mono = false,
}: {
	label: string;
	value: string;
	mono?: boolean;
}) {
	return (
		<div className="min-w-0">
			<dt className="text-xs text-muted-foreground">{label}</dt>
			<dd className={mono ? "break-all font-mono text-xs" : "text-sm"}>{value}</dd>
		</div>
	);
}

function HashValue({ label, value }: { label: string; value: string }) {
	return (
		<div className="min-w-0 rounded-md border p-3">
			<p className="text-xs text-muted-foreground">{label}</p>
			<code className="block break-all text-xs">{value}</code>
			<Button
				className="mt-2"
				size="xs"
				variant="outline"
				onClick={() => void navigator.clipboard.writeText(value)}
			>
				Copy {label}
			</Button>
		</div>
	);
}

function EmptyContract({ label }: { label: string }) {
	return <p className="text-sm text-muted-foreground">{label}</p>;
}

function ActionFeedback({ feedback }: { feedback: Feedback }) {
	return (
		<div
			className={`rounded-lg border p-3 text-sm ${feedback.tone === "error" ? "border-destructive/40 bg-destructive/10" : "bg-muted"}`}
			role={feedback.tone === "error" ? "alert" : "status"}
		>
			<p className="font-medium">{feedback.summary}</p>
			{feedback.details && <TechnicalDetails details={feedback.details} />}
		</div>
	);
}

function TechnicalDetails({ details }: { details: string }) {
	return (
		<details className="mt-2">
			<summary className="cursor-pointer font-medium">Technical details</summary>
			<Button
				className="my-2"
				size="xs"
				variant="outline"
				onClick={() => void navigator.clipboard.writeText(details)}
			>
				Copy technical details
			</Button>
			<pre className="max-h-48 overflow-auto whitespace-pre-wrap break-all rounded-md bg-background p-2 text-xs">
				{details}
			</pre>
		</details>
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
		<div className="flex min-w-0 flex-1 flex-col gap-5 p-4 lg:p-6">
			<div>
				<h1 className="text-2xl font-semibold">{title}</h1>
				<p className="text-sm text-muted-foreground">{description}</p>
			</div>
			{children}
		</div>
	);
}
