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
import { Link } from "@tanstack/react-router";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

const SDK_ARTIFACT_SHA256 =
	"54cb0dd8f1b2f911a30099f1c7ffdc3798cd3d18e7a331b6708b437f6fa28ed7";
const PROJECTS_CHANGED_EVENT = "adaq:python-projects-changed";

const TUTORIAL_PROJECTS = [
	{
		id: "py-factor-cross-sectional-momentum",
		example: "factor",
		kind: "factor",
		parameter: "lookback={5,20,60}",
		license: "Apache-2.0",
	},
	{
		id: "py-model-qlib-ridge-return",
		example: "model",
		kind: "model",
		parameter: "alpha={0.1,1,10}",
		license: "Apache-2.0",
	},
	{
		id: "py-strategy-top-n-forecast",
		example: "strategy",
		kind: "strategy",
		parameter: "M13 continuation",
		license: "Apache-2.0",
	},
] as const;

const EXECUTABLE_PROJECTS = TUTORIAL_PROJECTS.filter(
	(project) => project.kind !== "strategy",
);

type WorkingCopy = {
	projectId: string;
	state: "clean" | "dirty" | "invalid";
	path: string;
	issues: Array<{ code: string; message: string }>;
	revisionSha256?: string;
};

type RuntimeProfile = {
	status: string;
	platform?: string;
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

type AttemptPreview = {
	projectId: string;
	revisionSha256: string;
	entryPoint: string;
	sourceFiles: Record<string, string>;
	lock: {
		lockSha256: string;
		runtimeArtifactSha256: string;
		wheelhouseIdentity: string;
		platform: string;
		wheels: Array<{
			fileName: string;
			package: string;
			version: string;
			sha256: string;
			size: number;
		}>;
	};
	environmentSha256: string;
	runtime: {
		profile: string;
		version: string;
		platform: string;
		artifactSha256: string;
		source: string;
		signature: string;
	};
	sdkArtifactSha256: string;
	inputBindings: Record<string, string>;
	normalizedParameters: Record<string, string>;
	seed: number;
	resourcePolicy: Record<string, number | string>;
	trustDecision?: { decisionId: string };
	trustedCodeWarning: string;
};

const FIXTURE = {
	id: "python-tutorial-a-share@1",
	instruments: 12,
	sessions: 180,
	instrumentSha256:
		"a6963ebf7e0481749a1db2db22ef2f23bc5fee6d39d5afe258ca27c3c17fdaca",
	calendarSha256:
		"2e423b9b46a4af56729da0fee4298ed47cdaee70b6e0bc4e4e8f5fb03cd978a9",
	barsSha256: "fd4dc3bcccb554ad29ca08e89c35c220dafcb546db4df436009612f795a2bb4e",
	contentSha256:
		"6d44423e009d2251d442f388f1621242fc4dac1e0eb5d9b774fc62ecd135d848",
};

const WINDOWS = [
	["Train", "1–100"],
	["Purge", "101–105"],
	["Selection Validation", "106–140"],
	["Embargo", "141–145"],
	["Final Evaluation", "146–180"],
] as const;

const afterPaint = () =>
	new Promise<void>((resolve) => {
		if (
			typeof requestAnimationFrame === "undefined" ||
			document.visibilityState === "hidden"
		)
			return resolve();
		requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
		window.setTimeout(resolve, 100);
	});

function bytes(value?: number) {
	return value === undefined ? "—" : `${value.toLocaleString()} bytes`;
}

export function PythonTutorialPanel({ userId }: { userId: string }) {
	const { t } = useTranslation();
	const [projects, setProjects] = useState<WorkingCopy[]>([]);
	const [runtime, setRuntime] = useState<RuntimeProfile>();
	const [previews, setPreviews] = useState<Record<string, AttemptPreview>>({});
	const [loading, setLoading] = useState(true);
	const [busy, setBusy] = useState("");
	const [error, setError] = useState("");
	const [trustOpen, setTrustOpen] = useState(false);
	const refreshVersion = useRef(0);
	const trustTrigger = useRef<HTMLButtonElement>(null);
	const trustCancel = useRef<HTMLButtonElement>(null);
	const trustWasOpen = useRef(false);
	const trustDialog = useRef<HTMLDivElement>(null);

	const refresh = useCallback(async () => {
		if (!isTauriRuntime()) {
			setLoading(false);
			return;
		}
		const version = ++refreshVersion.current;
		setLoading(true);
		setError("");
		await afterPaint();
		try {
			const [nextProjects, nextRuntime] = await Promise.all([
				invoke<WorkingCopy[]>("project_list", { userId }),
				invoke<RuntimeProfile>("runtime_profile", { request: { userId } }),
			]);
			if (version !== refreshVersion.current) return;
			setProjects(nextProjects);
			setRuntime(nextRuntime);
			const nextPreviews: Record<string, AttemptPreview> = {};
			await Promise.all(
				EXECUTABLE_PROJECTS.map(async ({ id }) => {
					const project = nextProjects.find((item) => item.projectId === id);
					if (!project?.revisionSha256) return;
					const environment = await invoke<{ environmentSha256: string } | null>(
						"environment_for_project",
						{ request: { userId, projectId: id } },
					);
					if (!environment) return;
					try {
						nextPreviews[id] = await invoke<AttemptPreview>("attempt_preview", {
							request: {
								userId,
								projectId: id,
								revisionSha256: project.revisionSha256,
								seed: 0,
							},
						});
					} catch {
						// A missing Runtime or invalid Environment is shown by the owning card.
					}
				}),
			);
			if (version === refreshVersion.current) setPreviews(nextPreviews);
		} catch (reason) {
			if (version === refreshVersion.current) setError(String(reason));
		} finally {
			if (version === refreshVersion.current) setLoading(false);
		}
	}, [userId]);

	useEffect(() => {
		void refresh();
		const notify = () => void refresh();
		window.addEventListener(PROJECTS_CHANGED_EVENT, notify);
		return () => {
			refreshVersion.current += 1;
			window.removeEventListener(PROJECTS_CHANGED_EVENT, notify);
		};
	}, [refresh]);

	useEffect(() => {
		if (!trustOpen) {
			if (trustWasOpen.current) trustTrigger.current?.focus();
			trustWasOpen.current = false;
			return;
		}
		trustWasOpen.current = true;
		trustCancel.current?.focus();
		const onKeyDown = (event: KeyboardEvent) => {
			if (event.key === "Escape") setTrustOpen(false);
			if (event.key !== "Tab") return;
			const focusable = trustDialog.current?.querySelectorAll<HTMLElement>(
				"button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled])",
			);
			if (!focusable?.length) return;
			const first = focusable[0];
			const last = focusable[focusable.length - 1];
			if (event.shiftKey && document.activeElement === first) {
				event.preventDefault();
				last.focus();
			} else if (!event.shiftKey && document.activeElement === last) {
				event.preventDefault();
				first.focus();
			}
		};
		document.addEventListener("keydown", onKeyDown);
		return () => document.removeEventListener("keydown", onKeyDown);
	}, [trustOpen]);

	const createProject = async (example: string) => {
		setBusy(`create:${example}`);
		setError("");
		await afterPaint();
		try {
			await invoke("project_create", { request: { userId, example } });
			await refresh();
		} catch (reason) {
			setError(String(reason));
		} finally {
			setBusy("");
		}
	};

	const prepareProject = async (projectId: string) => {
		setBusy(`prepare:${projectId}`);
		setError("");
		await afterPaint();
		try {
			let currentProjects = await invoke<WorkingCopy[]>("project_list", {
				userId,
			});
			let project = currentProjects.find((item) => item.projectId === projectId);
			if (!project) throw new Error(t("pythonResearch.tutorial.projectRequired"));
			await invoke("project_validate", { request: { userId, projectId } });
			if (runtime?.status !== "ready") {
				await invoke("runtime_prepare_managed", { request: { userId } });
			}
			await invoke("environment_sync_managed", {
				request: { userId, projectId },
			});
			await invoke("project_validate", { request: { userId, projectId } });
			currentProjects = await invoke<WorkingCopy[]>("project_list", { userId });
			project = currentProjects.find((item) => item.projectId === projectId);
			if (project?.state !== "clean") {
				throw new Error(t("pythonResearch.tutorial.projectInvalid"));
			}
			await invoke("project_freeze", {
				request: {
					userId,
					projectId,
					sdkArtifactSha256: SDK_ARTIFACT_SHA256,
					runtimeArtifactSha256: null,
				},
			});
			await invoke("environment_prepare_managed", {
				request: { userId, projectId },
			});
			await refresh();
		} catch (reason) {
			setError(String(reason));
		} finally {
			setBusy("");
		}
	};

	const runTutorial = async () => {
		setBusy("tutorial");
		setError("");
		await afterPaint();
		try {
			for (const project of EXECUTABLE_PROJECTS) {
				const currentProjects = await invoke<WorkingCopy[]>("project_list", {
					userId,
				});
				if (!currentProjects.some((item) => item.projectId === project.id)) {
					await invoke("project_create", {
						request: { userId, example: project.example },
					});
				}
				await prepareProjectWithoutState(project.id);
			}
			await refresh();
		} catch (reason) {
			setError(String(reason));
		} finally {
			setBusy("");
		}
	};

	const prepareProjectWithoutState = async (projectId: string) => {
		await invoke("project_validate", { request: { userId, projectId } });
		if (runtime?.status !== "ready") {
			await invoke("runtime_prepare_managed", { request: { userId } });
		}
		await invoke("environment_sync_managed", { request: { userId, projectId } });
		await invoke("project_validate", { request: { userId, projectId } });
		const currentProjects = await invoke<WorkingCopy[]>("project_list", {
			userId,
		});
		const project = currentProjects.find((item) => item.projectId === projectId);
		if (project?.state !== "clean") {
			throw new Error(t("pythonResearch.tutorial.projectInvalid"));
		}
		await invoke("project_freeze", {
			request: {
				userId,
				projectId,
				sdkArtifactSha256: SDK_ARTIFACT_SHA256,
				runtimeArtifactSha256: null,
			},
		});
		await invoke("environment_prepare_managed", {
			request: { userId, projectId },
		});
	};

	const openTrustReview = () => {
		if (EXECUTABLE_PROJECTS.some(({ id }) => !previews[id])) {
			setError(t("pythonResearch.tutorial.prepareRequired"));
			return;
		}
		setTrustOpen(true);
	};

	const trustTutorialProjects = async () => {
		setBusy("trust");
		setError("");
		await afterPaint();
		try {
			for (const { id } of EXECUTABLE_PROJECTS) {
				const preview = previews[id];
				if (!preview) throw new Error(t("pythonResearch.tutorial.prepareRequired"));
				if (!preview.trustDecision) {
					await invoke("trust_revision", {
						request: {
							userId,
							projectId: id,
							revisionSha256: preview.revisionSha256,
						},
					});
				}
			}
			setTrustOpen(false);
			await refresh();
		} catch (reason) {
			setError(String(reason));
		} finally {
			setBusy("");
		}
	};

	const projectById = (projectId: string) =>
		projects.find((project) => project.projectId === projectId);

	return (
		<>
			<Card className="mb-4" aria-busy={Boolean(busy)}>
				<CardHeader>
					<div className="flex flex-wrap items-start justify-between gap-3">
						<div>
							<CardTitle>{t("pythonResearch.tutorial.title")}</CardTitle>
							<CardDescription>
								{t("pythonResearch.tutorial.description")}
							</CardDescription>
						</div>
						<div className="flex flex-wrap gap-2">
							<Button
								type="button"
								size="sm"
								onClick={() => void runTutorial()}
								loading={busy === "tutorial"}
							>
								{t("pythonResearch.tutorial.run")}
							</Button>
							<Button
								type="button"
								size="sm"
								variant="outline"
								onClick={() => void refresh()}
								loading={loading}
							>
								{t("pythonResearch.tutorial.refresh")}
							</Button>
						</div>
					</div>
				</CardHeader>
				<CardContent className="grid gap-4 text-sm">
					<div className="grid gap-2 rounded-md border p-3">
						<div className="flex flex-wrap items-center gap-2">
							<Badge variant="secondary">
								{t("pythonResearch.tutorial.synthetic")}
							</Badge>
							<code>{FIXTURE.id}</code>
						</div>
						<p>{t("pythonResearch.tutorial.fixtureShape", FIXTURE)}</p>
						<p className="break-all font-mono text-xs text-muted-foreground">
							{t("pythonResearch.tutorial.fixtureHashes", FIXTURE)}
						</p>
					</div>

					<div className="grid gap-2 md:grid-cols-5">
						{WINDOWS.map(([name, range]) => (
							<div key={name} className="rounded-md border p-2">
								<p className="font-medium">{name}</p>
								<p className="font-mono text-xs text-muted-foreground">{range}</p>
							</div>
						))}
					</div>
					<p className="text-xs text-muted-foreground">
						{t("pythonResearch.tutorial.boundary")}
					</p>

					{runtime ? (
						<div className="grid gap-1 rounded-md border p-3">
							<p className="font-medium">
								{t("pythonResearch.tutorial.runtime")} · {runtime.status}
							</p>
							<p>
								{runtime.expectedVersion} · {runtime.platform ?? "—"} · {runtime.source}
							</p>
							<p className="break-all font-mono text-xs text-muted-foreground">
								{runtime.artifactSha256 ?? "—"} · {bytes(runtime.downloadBytes)}{" "}
								download · {bytes(runtime.installedBytes)} installed ·{" "}
								{runtime.license ?? "—"}
							</p>
							<p className="break-all font-mono text-xs text-muted-foreground">
								{runtime.wheelhouseIdentity ?? "—"} · {runtime.wheelhouseStatus} ·{" "}
								{runtime.wheelhouseWheelCount} wheels ·{" "}
								{bytes(runtime.wheelhouseDiskBytes)} wheelhouse ·{" "}
								{bytes(runtime.environmentCacheBytes)} environments ·{" "}
								{bytes(runtime.runtimeCacheBytes)} runtime cache
							</p>
						</div>
					) : null}

					{error ? (
						<p className="text-destructive" role="alert">
							{error}
						</p>
					) : null}
					{loading ? (
						<p className="text-muted-foreground" role="status">
							{t("pythonResearch.tutorial.loading")}
						</p>
					) : null}

					<div className="grid gap-2">
						{TUTORIAL_PROJECTS.map((definition) => {
							const project = projectById(definition.id);
							const preview = previews[definition.id];
							const deferred = definition.kind === "strategy";
							return (
								<div key={definition.id} className="grid gap-2 rounded-md border p-3">
									<div className="flex flex-wrap items-center gap-2">
										<code className="break-all">{definition.id}</code>
										<Badge variant={deferred ? "outline" : "secondary"}>
											{deferred
												? t("pythonResearch.tutorial.deferred")
												: (project?.state ?? t("pythonResearch.tutorial.notCreated"))}
										</Badge>
										<span className="text-muted-foreground">{definition.parameter}</span>
										<span className="text-muted-foreground">{definition.license}</span>
									</div>
									{deferred ? (
										<p className="text-muted-foreground">
											{t("pythonResearch.tutorial.strategyDeferred")}
										</p>
									) : (
										<div className="flex flex-wrap gap-2">
											{!project ? (
												<Button
													type="button"
													size="sm"
													onClick={() => void createProject(definition.example)}
													loading={busy === `create:${definition.example}`}
												>
													{t("pythonResearch.tutorial.create")}
												</Button>
											) : null}
											{project ? (
												<Button
													type="button"
													size="sm"
													variant="outline"
													onClick={() => void prepareProject(definition.id)}
													loading={busy === `prepare:${definition.id}`}
												>
													{t("pythonResearch.tutorial.prepare")}
												</Button>
											) : null}
											{preview ? (
												<Badge variant="secondary">
													{preview.trustDecision
														? t("pythonResearch.tutorial.trusted")
														: t("pythonResearch.tutorial.untrusted")}
												</Badge>
											) : null}
										</div>
									)}
									{preview ? (
										<details>
											<summary className="cursor-pointer">
												{t("pythonResearch.tutorial.inspect")}
											</summary>
											<div className="mt-2 grid gap-1 break-all font-mono text-xs text-muted-foreground">
												<p>
													{t("pythonResearch.tutorial.revision")}: {preview.revisionSha256}
												</p>
												<p>
													{t("pythonResearch.tutorial.entryPoint")}: {preview.entryPoint}
												</p>
												<p>
													{t("pythonResearch.tutorial.sdk")}: {preview.sdkArtifactSha256}
												</p>
												<p>
													{t("pythonResearch.tutorial.environment")}:{" "}
													{preview.environmentSha256}
												</p>
												<p>
													{t("pythonResearch.tutorial.lock")}: {JSON.stringify(preview.lock)}
												</p>
												<p>
													{t("pythonResearch.tutorial.runtimeIdentity")}:{" "}
													{JSON.stringify(preview.runtime)}
												</p>
												<p>
													{t("pythonResearch.tutorial.wheelhouse")}:{" "}
													{preview.lock.wheelhouseIdentity}
												</p>
												<p>
													{t("pythonResearch.tutorial.parameters")}:{" "}
													{JSON.stringify(preview.normalizedParameters)}
												</p>
												<p>
													{t("pythonResearch.tutorial.inputs")}:{" "}
													{JSON.stringify(preview.inputBindings)}
												</p>
												<p>
													{t("pythonResearch.tutorial.seed")}: {preview.seed}
												</p>
												<p>
													{t("pythonResearch.tutorial.resourcePolicy")}:{" "}
													{JSON.stringify(preview.resourcePolicy)}
												</p>
												<p>{preview.trustedCodeWarning}</p>
												<details>
													<summary className="cursor-pointer">
														{t("pythonResearch.tutorial.sourceFiles")}
													</summary>
													{Object.entries(preview.sourceFiles).map(([path, hash]) => (
														<p key={path}>
															{path}: {hash}
														</p>
													))}
												</details>
											</div>
										</details>
									) : null}
								</div>
							);
						})}
					</div>

					<div className="flex flex-wrap items-center gap-2 rounded-md border p-3">
						<Button
							ref={trustTrigger}
							type="button"
							size="sm"
							onClick={openTrustReview}
							disabled={EXECUTABLE_PROJECTS.some(({ id }) => !previews[id])}
						>
							{t("pythonResearch.tutorial.reviewTrust")}
						</Button>
						<span className="text-xs text-muted-foreground">
							{t("pythonResearch.tutorial.trustHint")}
						</span>
					</div>

					<div className="grid gap-2 rounded-md border p-3">
						<p className="font-medium">{t("pythonResearch.tutorial.nextSteps")}</p>
						<ol className="grid gap-2 pl-5 text-muted-foreground [list-style:decimal]">
							<li>
								<Link className="underline" to="/factors">
									{t("pythonResearch.tutorial.factorStep")}
								</Link>
							</li>
							<li>
								<Link className="underline" to="/models">
									{t("pythonResearch.tutorial.modelStep")}
								</Link>
							</li>
							<li>{t("pythonResearch.tutorial.decisionStep")}</li>
						</ol>
					</div>
				</CardContent>
			</Card>

			{trustOpen ? (
				<div className="fixed inset-0 z-50 grid place-items-center bg-black/50 p-4">
					<div
						ref={trustDialog}
						role="dialog"
						aria-modal="true"
						aria-labelledby="python-tutorial-trust-title"
						aria-describedby="python-tutorial-trust-description"
						className="grid w-full max-w-xl gap-4 rounded-lg border bg-background p-6 shadow-lg"
					>
						<div>
							<h2 id="python-tutorial-trust-title" className="font-semibold">
								{t("pythonResearch.tutorial.trustTitle")}
							</h2>
							<p
								id="python-tutorial-trust-description"
								className="mt-1 text-sm text-muted-foreground"
							>
								{t("pythonResearch.tutorial.trustDescription")}
							</p>
						</div>
						<div className="grid gap-2 text-xs">
							{EXECUTABLE_PROJECTS.map(({ id }) => (
								<p key={id} className="break-all font-mono">
									{id}: {previews[id]?.revisionSha256}
								</p>
							))}
						</div>
						<div className="flex justify-end gap-2">
							<Button
								ref={trustCancel}
								type="button"
								variant="outline"
								onClick={() => setTrustOpen(false)}
							>
								{t("pythonResearch.tutorial.cancel")}
							</Button>
							<Button
								type="button"
								onClick={() => void trustTutorialProjects()}
								loading={busy === "trust"}
							>
								{t("pythonResearch.tutorial.trustExact")}
							</Button>
						</div>
					</div>
				</div>
			) : null}
		</>
	);
}
