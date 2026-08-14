import { invoke } from "@tauri-apps/api/core";
import { openPath } from "@tauri-apps/plugin-opener";
import { readFile, writeFile } from "@tauri-apps/plugin-fs";
import { open as chooseFile, save } from "@tauri-apps/plugin-dialog";
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
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

type WorkingCopy = {
	projectId: string;
	path: string;
	state: "clean" | "dirty" | "invalid";
	revisionSha256?: string;
	issues: Array<{ code: string; message: string }>;
};

type ProjectRevision = {
	revisionSha256: string;
};

type ResearchAttempt = {
	attemptId: string;
	projectId: string;
	status: string;
	queueSequence: number;
	failureCode?: string;
	diagnostic?: string;
	log?: string;
	stagedResultSha256?: string;
};

type EnvironmentRecord = {
	environmentSha256: string;
};

type Props = {
	userId: string;
	kind: "factor" | "model";
};

const SDK_ARTIFACT_SHA256 =
	"f7d25a1e4dd57e8a2d845d117bc95973e177042bc514af02290fc7563bd6abfd";
const PROJECTS_CHANGED_EVENT = "adaq:python-projects-changed";

function afterPaint() {
	return new Promise<void>((resolve) => {
		if (typeof requestAnimationFrame === "undefined") return resolve();
		requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
	});
}

export function PythonProjectsPanel({ userId, kind }: Props) {
	const { t } = useTranslation();
	const [projects, setProjects] = useState<WorkingCopy[]>([]);
	const [loading, setLoading] = useState(true);
	const [busy, setBusy] = useState("");
	const [error, setError] = useState("");
	const [revisions, setRevisions] = useState<Record<string, string>>({});
	const [trusted, setTrusted] = useState<Record<string, boolean>>({});
	const [attempts, setAttempts] = useState<ResearchAttempt[]>([]);
	const [environmentSha256, setEnvironmentSha256] = useState<
		Record<string, string>
	>({});

	const refreshAttempts = useCallback(async () => {
		if (!isTauriRuntime()) return;
		try {
			setAttempts(await invoke<ResearchAttempt[]>("attempt_list", { userId }));
		} catch (reason) {
			setError(String(reason));
		}
	}, [userId]);

	const refresh = useCallback(
		async (notify = false) => {
			if (!isTauriRuntime()) {
				setLoading(false);
				return;
			}
			setLoading(true);
			setError("");
			await afterPaint();
			try {
				const [nextProjects, nextAttempts] = await Promise.all([
					invoke<WorkingCopy[]>("project_list", { userId }),
					invoke<ResearchAttempt[]>("attempt_list", { userId }),
				]);
				setProjects(nextProjects);
				setAttempts(nextAttempts);
				if (notify) window.dispatchEvent(new Event(PROJECTS_CHANGED_EVENT));
			} catch (reason) {
				setError(String(reason));
			} finally {
				setLoading(false);
			}
		},
		[userId],
	);

	useEffect(() => {
		let active = true;
		void refresh().finally(() => {
			if (!active) return;
		});
		return () => {
			active = false;
		};
	}, [refresh]);

	useEffect(() => {
		if (!isTauriRuntime()) return;
		const timer = window.setInterval(() => void refreshAttempts(), 2000);
		return () => window.clearInterval(timer);
	}, [refreshAttempts]);

	const create = async () => {
		const key = `${kind}:create`;
		setBusy(key);
		setError("");
		await afterPaint();
		try {
			await invoke("project_create", {
				request: { userId, example: kind },
			});
			await refresh(true);
		} catch (reason) {
			setError(String(reason));
		} finally {
			setBusy("");
		}
	};

	const validate = async (projectId: string) => {
		setBusy(`${projectId}:validate`);
		setError("");
		await afterPaint();
		try {
			await invoke("project_validate", {
				request: { userId, projectId },
			});
			await refresh(true);
		} catch (reason) {
			setError(String(reason));
		} finally {
			setBusy("");
		}
	};

	const openProject = (path: string) => {
		if (isTauriRuntime())
			void openPath(path).catch((reason) => setError(String(reason)));
	};

	const exportProject = async (project: WorkingCopy) => {
		setBusy(`${project.projectId}:export`);
		setError("");
		await afterPaint();
		try {
			const result = await invoke<{
				projectId: string;
				revisionSha256: string;
				bytes: number[];
			}>("project_export", {
				request: {
					userId,
					projectId: project.projectId,
					sdkArtifactSha256: SDK_ARTIFACT_SHA256,
					runtimeArtifactSha256: null,
				},
			});
			const path = await save({
				defaultPath: `${project.projectId}.adaq-python.zip`,
			});
			if (path) await writeFile(path, new Uint8Array(result.bytes));
			setRevisions((current) => ({
				...current,
				[project.projectId]: result.revisionSha256,
			}));
		} catch (reason) {
			setError(String(reason));
		} finally {
			setBusy("");
		}
	};

	const freeze = async (project: WorkingCopy) => {
		setBusy(`${project.projectId}:freeze`);
		setError("");
		await afterPaint();
		try {
			const revision = await invoke<ProjectRevision>("project_freeze", {
				request: {
					userId,
					projectId: project.projectId,
					sdkArtifactSha256: SDK_ARTIFACT_SHA256,
					runtimeArtifactSha256: null,
				},
			});
			setRevisions((current) => ({
				...current,
				[project.projectId]: revision.revisionSha256,
			}));
			await refresh(true);
		} catch (reason) {
			setError(String(reason));
		} finally {
			setBusy("");
		}
	};

	const trust = async (project: WorkingCopy) => {
		const revision = revisions[project.projectId];
		if (!revision) return;
		setBusy(`${project.projectId}:trust`);
		setError("");
		await afterPaint();
		try {
			await invoke("trust_revision", {
				request: { userId, projectId: project.projectId, revisionSha256: revision },
			});
			setTrusted((current) => ({ ...current, [project.projectId]: true }));
		} catch (reason) {
			setError(String(reason));
		} finally {
			setBusy("");
		}
	};

	const startAttempt = async (project: WorkingCopy) => {
		const revision = revisions[project.projectId];
		const environment = environmentSha256[project.projectId];
		if (!revision || !trusted[project.projectId] || !environment) return;
		setBusy(`${project.projectId}:start`);
		setError("");
		await afterPaint();
		try {
			await invoke("attempt_start", {
				request: {
					userId,
					projectId: project.projectId,
					revisionSha256: revision,
					environmentSha256: environment,
				},
			});
			await refresh(true);
		} catch (reason) {
			setError(String(reason));
		} finally {
			setBusy("");
		}
	};

	const prepareEnvironment = async (project: WorkingCopy) => {
		setBusy(`${project.projectId}:environment`);
		setError("");
		await afterPaint();
		try {
			const result = await invoke<EnvironmentRecord>(
				"environment_prepare_managed",
				{
					request: { userId, projectId: project.projectId },
				},
			);
			setEnvironmentSha256((current) => ({
				...current,
				[project.projectId]: result.environmentSha256,
			}));
			setRevisions((current) => {
				const next = { ...current };
				delete next[project.projectId];
				return next;
			});
			setTrusted((current) => {
				const next = { ...current };
				delete next[project.projectId];
				return next;
			});
			await refresh(true);
		} catch (reason) {
			setError(String(reason));
		} finally {
			setBusy("");
		}
	};

	const updateAttempt = async (
		attemptId: string,
		action: "cancel" | "retry",
	) => {
		setBusy(`${attemptId}:${action}`);
		setError("");
		await afterPaint();
		try {
			await invoke(action === "cancel" ? "attempt_cancel" : "attempt_retry", {
				request: { userId, attemptId },
			});
			await refresh();
		} catch (reason) {
			setError(String(reason));
		} finally {
			setBusy("");
		}
	};

	const importProject = async () => {
		const path = await chooseFile({ multiple: false, directory: false });
		if (!path || Array.isArray(path)) return;
		setBusy("import");
		setError("");
		await afterPaint();
		try {
			await invoke("project_import", {
				request: { userId, bytes: Array.from(await readFile(path)) },
			});
			await refresh(true);
		} catch (reason) {
			setError(String(reason));
		} finally {
			setBusy("");
		}
	};

	const visible = projects.filter((project) =>
		project.projectId.startsWith(`py-${kind}-`),
	);
	const visibleAttempts = attempts.filter((attempt) =>
		attempt.projectId.startsWith(`py-${kind}-`),
	);
	return (
		<Card>
			<CardHeader>
				<div className="flex flex-wrap items-start justify-between gap-3">
					<div>
						<CardTitle>{t("pythonResearch.projects.title")}</CardTitle>
						<CardDescription>
							{t("pythonResearch.projects.description")}
						</CardDescription>
					</div>
					<div className="flex flex-wrap gap-2">
						<Button
							type="button"
							size="sm"
							onClick={() => void create()}
							loading={busy === `${kind}:create`}
						>
							{t(`pythonResearch.projects.create.${kind}`)}
						</Button>
						<Button
							type="button"
							size="sm"
							variant="outline"
							onClick={() => void importProject()}
							disabled={!isTauriRuntime() || busy === "import"}
						>
							{t("pythonResearch.projects.import")}
						</Button>
					</div>
				</div>
			</CardHeader>
			<CardContent className="grid gap-3">
				<div className="grid gap-2 rounded-md border p-3 text-sm">
					<p>{t("pythonResearch.projects.environment")}</p>
					<p className="text-xs text-muted-foreground">
						{t("pythonResearch.projects.environmentHint")}
					</p>
				</div>
				<p className="text-xs text-muted-foreground">
					{t("pythonResearch.projects.staticNote")}
				</p>
				{error ? (
					<p className="text-sm text-destructive" role="alert">
						{error}
					</p>
				) : null}
				{loading ? (
					<p className="text-sm text-muted-foreground" role="status">
						{t("pythonResearch.projects.loading")}
					</p>
				) : null}
				{!loading && visible.length === 0 ? (
					<p className="text-sm text-muted-foreground">
						{t("pythonResearch.projects.empty")}
					</p>
				) : null}
				{visible.map((project) => (
					<div
						key={project.projectId}
						className="grid gap-2 rounded-md border p-3 text-sm"
					>
						<div className="flex flex-wrap items-center gap-2">
							<code className="break-all">{project.projectId}</code>
							<Badge variant={project.state === "clean" ? "secondary" : "outline"}>
								{t(`pythonResearch.projects.state.${project.state}`)}
							</Badge>
							<div className="ml-auto flex flex-wrap gap-2">
								<Button
									type="button"
									size="sm"
									variant="outline"
									onClick={() => openProject(project.path)}
								>
									{t("pythonResearch.projects.open")}
								</Button>
								<Button
									type="button"
									size="sm"
									variant="outline"
									onClick={() => void validate(project.projectId)}
									loading={busy === `${project.projectId}:validate`}
								>
									{t("pythonResearch.projects.validate")}
								</Button>
								<Button
									type="button"
									size="sm"
									variant="outline"
									onClick={() => void freeze(project)}
									disabled={project.state !== "clean"}
									loading={busy === `${project.projectId}:freeze`}
								>
									{t("pythonResearch.projects.freeze")}
								</Button>
								<Button
									type="button"
									size="sm"
									variant="outline"
									onClick={() => void exportProject(project)}
									disabled={project.state !== "clean"}
									loading={busy === `${project.projectId}:export`}
								>
									{t("pythonResearch.projects.export")}
								</Button>
								<Button
									type="button"
									size="sm"
									variant="outline"
									onClick={() => void trust(project)}
									disabled={!revisions[project.projectId]}
									loading={busy === `${project.projectId}:trust`}
								>
									{trusted[project.projectId]
										? t("pythonResearch.projects.trusted")
										: t("pythonResearch.projects.trust")}
								</Button>
								<Button
									type="button"
									size="sm"
									variant="outline"
									onClick={() => void prepareEnvironment(project)}
									disabled={project.state !== "clean"}
									loading={busy === `${project.projectId}:environment`}
								>
									{t("pythonResearch.projects.prepareEnvironment")}
								</Button>
								<Button
									type="button"
									size="sm"
									variant="outline"
									onClick={() => void startAttempt(project)}
									disabled={
										!trusted[project.projectId] || !environmentSha256[project.projectId]
									}
									loading={busy === `${project.projectId}:start`}
								>
									{t("pythonResearch.projects.start")}
								</Button>
							</div>
						</div>
						<p className="break-all text-xs text-muted-foreground">{project.path}</p>
						{revisions[project.projectId] ? (
							<p className="break-all font-mono text-xs text-muted-foreground">
								{t("pythonResearch.projects.revision")}: {revisions[project.projectId]}
							</p>
						) : null}
						{environmentSha256[project.projectId] ? (
							<p className="break-all font-mono text-xs text-muted-foreground">
								{t("pythonResearch.projects.environmentReady")}:{" "}
								{environmentSha256[project.projectId]}
							</p>
						) : null}
						{project.issues.length ? (
							<p className="text-xs text-destructive">{project.issues[0]?.message}</p>
						) : null}
					</div>
				))}
				{visibleAttempts.map((attempt) => (
					<div
						key={attempt.attemptId}
						className="grid gap-1 rounded-md border p-3 text-sm"
					>
						<div className="flex flex-wrap items-center gap-2">
							<code className="break-all">{attempt.attemptId}</code>
							<Badge variant="outline">{attempt.status}</Badge>
							<span className="text-muted-foreground">#{attempt.queueSequence}</span>
							<div className="ml-auto flex gap-2">
								{(attempt.status === "pending" || attempt.status === "running") && (
									<Button
										type="button"
										size="sm"
										variant="outline"
										onClick={() => void updateAttempt(attempt.attemptId, "cancel")}
										loading={busy === `${attempt.attemptId}:cancel`}
									>
										{t("pythonResearch.projects.cancel")}
									</Button>
								)}
								{(attempt.status === "failed" || attempt.status === "cancelled") && (
									<Button
										type="button"
										size="sm"
										variant="outline"
										onClick={() => void updateAttempt(attempt.attemptId, "retry")}
										loading={busy === `${attempt.attemptId}:retry`}
									>
										{t("pythonResearch.projects.retry")}
									</Button>
								)}
							</div>
						</div>
						{attempt.failureCode || attempt.diagnostic || attempt.log ? (
							<p className="break-all text-xs text-muted-foreground">
								{attempt.failureCode ?? ""} {attempt.diagnostic ?? attempt.log ?? ""}
							</p>
						) : null}
						{attempt.stagedResultSha256 ? (
							<p className="break-all font-mono text-xs text-muted-foreground">
								{t("pythonResearch.projects.result")}: {attempt.stagedResultSha256}
							</p>
						) : null}
					</div>
				))}
			</CardContent>
		</Card>
	);
}
