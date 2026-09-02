import { useAuthenticatedUserId } from "@/authenticated-user";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
	Card,
	CardAction,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "@/components/ui/card";
import { invoke } from "@tauri-apps/api/core";
import { Link } from "@tanstack/react-router";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { formatDateTime, formatDecimal, formatNumber } from "@/lib/i18n";

export type Health = {
	entityId: string;
	dimension: string;
	state: "healthy" | "degraded" | "critical" | "unknown";
	required: boolean;
	observedAtMs: number;
	eventId: string;
	condition: string;
	evidenceId?: string;
	diagnostic?: string;
};

export type Alert = {
	alertId: string;
	entityId: string;
	dimension: string;
	condition: string;
	policyId: string;
	severity: "info" | "warning" | "critical";
	state: "active" | "acknowledged" | "resolved";
	safetyAction: string;
	firstEventId: string;
	firstCriticalEventId?: string;
	firstObservedAtMs: number;
	occurrenceCount: number;
	lastObservedAtMs: number;
	lastEventId: string;
	evidenceId?: string;
	correlationId?: string;
	diagnostic?: string;
};

export type Event = {
	eventId: string;
	entityId: string;
	dimension: string;
	kind: string;
	observedAtMs: number;
	diagnostic?: string;
};

type Lifecycle = {
	lifecycleId: string;
	state: Alert["state"];
	eventId: string;
	occurredAtMs: number;
	actor: string;
};

export type SystemDashboardProjection = {
	operationalResponsibility: boolean;
	updatedAtMs: number;
	unavailable: string[];
	health: Health[];
	alerts: Alert[];
	events: Event[];
	bots: Array<{
		botId: string;
		state: string;
		currentAttemptId?: string;
		currentAttemptState?: string;
		attemptCount: number;
		decisionCount: number;
		orderCount: number;
		unmanagedPositionCount: number;
		reconciliationRequired: boolean;
		lastDecisionTimeMs?: number;
		updatedAtMs?: number;
	}>;
	paperAccount: {
		accountId: string;
		market: string;
		currency: string;
		cash: string;
		reservedCash: string;
		buyingPower: string;
		positionCount: number;
		orderCount: number;
		fillCount: number;
		reconciliation: string;
		restartRequired: boolean;
		observedAtMs: number;
	} | null;
	research: {
		watchlistCount: number;
		snapshotCount: number;
		componentCount: number;
		modelArtifactCount: number;
		signalDatasetCount: number;
		generationAttemptCount: number;
		backtestRunCount: number;
		validationProtocolCount: number;
		validationReportCount: number;
		feedbackSnapshotCount: number;
		feedbackReportCount: number;
		reviewDecisionCount: number;
	};
};

const healthTone: Record<Health["state"], string> = {
	healthy: "border-emerald-500/40 bg-emerald-500/5",
	degraded: "border-amber-500/40 bg-amber-500/5",
	critical: "border-destructive/50 bg-destructive/5",
	unknown: "bg-muted/40",
};

const evidenceLinks = [
	{ to: "/factors", labelKey: "nav.factorResearch" },
	{ to: "/markets/crypto", labelKey: "operations.marketEvidence" },
	{ to: "/paper-feedback", labelKey: "nav.paperFeedback" },
] as const;

const evidencePathByDimension: Record<string, string> = {
	marketData: "/markets/crypto",
	worker: "/bots",
	featureModelStrategy: "/features",
	paperAccount: "/paper-trading",
	riskOms: "/paper-trading",
	executionAdapter: "/paper-trading",
	localSystem: "/operations",
	researchFeedback: "/paper-feedback",
};

export function OperationsDashboard() {
	const { t } = useTranslation();
	const userId = useAuthenticatedUserId();
	const queryClient = useQueryClient();
	const [filter, setFilter] = useState<"all" | Alert["state"]>("all");
	const [severityFilter, setSeverityFilter] = useState<
		"all" | Alert["severity"]
	>("all");
	const [dimensionFilter, setDimensionFilter] = useState("all");
	const [entityFilter, setEntityFilter] = useState("");
	const [historyAlertId, setHistoryAlertId] = useState<string | null>(null);
	const health = useQuery({
		queryKey: ["operations-health", userId],
		queryFn: () => invoke<Health[]>("operations_health"),
		enabled: Boolean(userId),
		refetchInterval: 15_000,
	});
	const alerts = useQuery({
		queryKey: ["operations-alerts", userId],
		queryFn: () => invoke<Alert[]>("operations_alerts"),
		enabled: Boolean(userId),
		refetchInterval: 15_000,
	});
	const events = useQuery({
		queryKey: ["operations-events", userId],
		queryFn: () => invoke<Event[]>("operations_events", { limit: 64 }),
		enabled: Boolean(userId),
		refetchInterval: 15_000,
	});
	const probe = useQuery({
		queryKey: ["operations-probe", userId],
		queryFn: () => invoke("operations_probe"),
		enabled: Boolean(userId),
		refetchInterval: 30_000,
	});
	const history = useQuery({
		queryKey: ["operations-alert-history", userId, historyAlertId],
		queryFn: () =>
			invoke<Lifecycle[]>("operations_alert_history", { alertId: historyAlertId }),
		enabled: Boolean(historyAlertId),
	});
	const acknowledge = useMutation({
		mutationFn: (alertId: string) =>
			invoke("operations_alert_acknowledge", { alertId }),
		onSuccess: () => {
			void queryClient.invalidateQueries({
				queryKey: ["operations-alerts", userId],
			});
			void queryClient.invalidateQueries({
				queryKey: ["operations-events", userId],
			});
			void queryClient.invalidateQueries({
				queryKey: ["operations-alert-history", userId],
			});
		},
	});
	const freezeAll = useMutation({
		mutationFn: () => invoke("operations_freeze_all"),
		onSuccess: () => {
			void queryClient.invalidateQueries({
				queryKey: ["operations-health", userId],
			});
			void queryClient.invalidateQueries({
				queryKey: ["operations-alerts", userId],
			});
			void queryClient.invalidateQueries({
				queryKey: ["operations-events", userId],
			});
			void queryClient.invalidateQueries({
				queryKey: ["operations-alert-history", userId],
			});
		},
	});
	const recoverHost = useMutation({
		mutationFn: () => invoke("operations_recover_host"),
		onSuccess: () => {
			void queryClient.invalidateQueries({
				queryKey: ["operations-health", userId],
			});
			void queryClient.invalidateQueries({
				queryKey: ["operations-alerts", userId],
			});
			void queryClient.invalidateQueries({
				queryKey: ["operations-events", userId],
			});
			void queryClient.invalidateQueries({
				queryKey: ["operations-alert-history", userId],
			});
		},
	});
	useEffect(() => {
		if (probe.dataUpdatedAt === 0) return;
		void queryClient.invalidateQueries({
			queryKey: ["operations-health", userId],
		});
		void queryClient.invalidateQueries({
			queryKey: ["operations-alerts", userId],
		});
		void queryClient.invalidateQueries({
			queryKey: ["operations-events", userId],
		});
		void queryClient.invalidateQueries({
			queryKey: ["operations-alert-history", userId],
		});
	}, [probe.dataUpdatedAt, queryClient, userId]);
	const values = alerts.data ?? [];
	const visible = values.filter(
		(alert) =>
			(filter === "all" || alert.state === filter) &&
			(severityFilter === "all" || alert.severity === severityFilter) &&
			(dimensionFilter === "all" || alert.dimension === dimensionFilter) &&
			(!entityFilter ||
				alert.entityId.toLowerCase().includes(entityFilter.toLowerCase())),
	);
	const label = (key: string) => t(key, { defaultValue: key.split(".").at(-1) });
	const dimensions = [
		"marketData",
		"worker",
		"featureModelStrategy",
		"paperAccount",
		"riskOms",
		"executionAdapter",
		"localSystem",
		"researchFeedback",
	] as const;
	const requestFreezeAll = () => {
		if (window.confirm(t("operations.freezeConfirm"))) freezeAll.mutate();
	};

	return (
		<div className="grid gap-4 p-4 md:p-6">
			<header className="flex flex-wrap items-start justify-between gap-3">
				<div>
					<h1 className="text-2xl font-semibold tracking-tight">
						{t("operations.title")}
					</h1>
					<p className="text-sm text-muted-foreground">
						{t("operations.description")}
					</p>
				</div>
				<Button
					variant="destructive"
					disabled={freezeAll.isPending}
					onClick={requestFreezeAll}
				>
					{freezeAll.isPending
						? t("operations.freezeStarted")
						: t("operations.freezeAll")}
				</Button>
				{values.some(
					(alert) =>
						alert.safetyAction === "freezeAll" && alert.state !== "resolved",
				) ? (
					<Button
						variant="outline"
						disabled={recoverHost.isPending}
						onClick={() => recoverHost.mutate()}
					>
						{recoverHost.isPending
							? t("operations.recoverStarted")
							: t("operations.recoverHost")}
					</Button>
				) : null}
			</header>
			{freezeAll.isError ? (
				<p role="alert" className="text-sm text-destructive">
					{t("operations.freezeFailed")}
				</p>
			) : freezeAll.isSuccess ? (
				<p role="status" className="text-sm text-muted-foreground">
					{t("operations.freezeCompleted")}
				</p>
			) : null}
			{recoverHost.isError ? (
				<p role="alert" className="text-sm text-destructive">
					{t("operations.recoverFailed")}
				</p>
			) : recoverHost.isSuccess ? (
				<p role="status" className="text-sm text-muted-foreground">
					{t("operations.recoverCompleted")}
				</p>
			) : null}
			{probe.isError ? (
				<p role="status" className="text-xs text-muted-foreground">
					{t("operations.probeUnavailable")}
				</p>
			) : null}
			<div className="grid gap-4 lg:grid-cols-2">
				<Card>
					<CardHeader>
						<CardTitle>{t("operations.healthTitle")}</CardTitle>
						<CardDescription>{t("operations.healthDescription")}</CardDescription>
					</CardHeader>
					<CardContent>
						{health.isPending ? (
							<p role="status" className="text-sm text-muted-foreground">
								{t("operations.loading")}
							</p>
						) : health.isError ? (
							<p role="alert" className="text-sm text-destructive">
								{t("operations.loadError")}
							</p>
						) : health.data?.length ? (
							<div className="grid gap-2 sm:grid-cols-2">
								{health.data.map((item) => (
									<div
										className={`rounded-md border p-3 ${healthTone[item.state]}`}
										key={`${item.entityId}-${item.dimension}`}
									>
										<div className="flex items-center justify-between gap-2">
											<span className="font-medium">
												{label(`operations.dimensions.${item.dimension}`)}
											</span>
											<Badge variant="outline">
												{label(`operations.states.${item.state}`)}
											</Badge>
										</div>
										<p className="mt-1 text-xs text-muted-foreground">
											{item.entityId}
											{item.required ? ` · ${t("operations.required")}` : ""}
										</p>
										<p className="mt-1 text-xs text-muted-foreground">{item.condition}</p>
									</div>
								))}
							</div>
						) : (
							<p className="text-sm text-muted-foreground">
								{t("operations.noHealth")}
							</p>
						)}
					</CardContent>
				</Card>
				<Card>
					<CardHeader>
						<CardTitle>{t("operations.alertsTitle")}</CardTitle>
						<CardDescription>{t("operations.alertsDescription")}</CardDescription>
						<CardAction className="flex flex-wrap justify-end gap-2">
							<input
								aria-label={t("operations.entityFilter")}
								className="w-32 rounded-md border bg-background px-2 py-1 text-sm"
								placeholder={t("operations.entityFilter")}
								value={entityFilter}
								onChange={(event) => setEntityFilter(event.target.value)}
							/>
							<select
								aria-label={t("operations.alertFilter")}
								className="rounded-md border bg-background px-2 py-1 text-sm"
								value={filter}
								onChange={(event) => setFilter(event.target.value as typeof filter)}
							>
								<option value="all">{t("operations.allAlerts")}</option>
								<option value="active">{label("operations.states.active")}</option>
								<option value="acknowledged">
									{label("operations.states.acknowledged")}
								</option>
								<option value="resolved">{label("operations.states.resolved")}</option>
							</select>
							<select
								aria-label={t("operations.severityFilter")}
								className="rounded-md border bg-background px-2 py-1 text-sm"
								value={severityFilter}
								onChange={(event) =>
									setSeverityFilter(event.target.value as typeof severityFilter)
								}
							>
								<option value="all">{t("operations.allSeverities")}</option>
								<option value="info">{label("operations.severities.info")}</option>
								<option value="warning">
									{label("operations.severities.warning")}
								</option>
								<option value="critical">
									{label("operations.severities.critical")}
								</option>
							</select>
							<select
								aria-label={t("operations.dimensionFilter")}
								className="rounded-md border bg-background px-2 py-1 text-sm"
								value={dimensionFilter}
								onChange={(event) => setDimensionFilter(event.target.value)}
							>
								<option value="all">{t("operations.allDimensions")}</option>
								{dimensions.map((dimension) => (
									<option key={dimension} value={dimension}>
										{label(`operations.dimensions.${dimension}`)}
									</option>
								))}
							</select>
						</CardAction>
					</CardHeader>
					<CardContent className="space-y-2">
						{alerts.isPending ? (
							<p role="status" className="text-sm text-muted-foreground">
								{t("operations.loading")}
							</p>
						) : alerts.isError ? (
							<p role="alert" className="text-sm text-destructive">
								{t("operations.loadError")}
							</p>
						) : visible.length ? (
							visible.map((alert) => (
								<div className="rounded-md border p-3" key={alert.alertId}>
									<div className="flex flex-wrap items-center justify-between gap-2">
										<span className="font-medium">{alert.condition}</span>
										<div className="flex gap-2">
											<Badge
												variant={alert.severity === "critical" ? "destructive" : "outline"}
											>
												{label(`operations.severities.${alert.severity}`)}
											</Badge>
											<Badge variant="outline">
												{label(`operations.states.${alert.state}`)}
											</Badge>
										</div>
									</div>
									<p className="mt-1 text-xs text-muted-foreground">
										{alert.entityId} · {label(`operations.actions.${alert.safetyAction}`)}{" "}
										· {t("operations.occurrences", { count: alert.occurrenceCount })}
									</p>
									<details
										className="mt-2 text-xs"
										onToggle={(event) => {
											if (event.currentTarget.open) setHistoryAlertId(alert.alertId);
										}}
									>
										<summary className="cursor-pointer font-medium">
											{t("operations.evidenceDetails")}
										</summary>
										<dl className="mt-2 grid gap-1 text-muted-foreground">
											<div>
												<dt className="inline font-medium">
													{t("operations.evidenceId")}:{" "}
												</dt>
												<dd className="inline">
													{alert.evidenceId ?? t("operations.notAvailable")}
												</dd>
											</div>
											<div>
												<dt className="inline font-medium">{t("operations.policyId")}: </dt>
												<dd className="inline">{alert.policyId}</dd>
											</div>
											<div>
												<dt className="inline font-medium">
													{t("operations.correlationId")}:{" "}
												</dt>
												<dd className="inline">
													{alert.correlationId ?? t("operations.notAvailable")}
												</dd>
											</div>
											<div>
												<dt className="inline font-medium">
													{t("operations.firstEvent")}:{" "}
												</dt>
												<dd className="inline">{alert.firstEventId}</dd>
											</div>
											<div>
												<dt className="inline font-medium">
													{t("operations.firstCriticalEvent")}:{" "}
												</dt>
												<dd className="inline">
													{alert.firstCriticalEventId ?? t("operations.notAvailable")}
												</dd>
											</div>
											<div>
												<dt className="inline font-medium">
													{t("operations.lastEvent")}:{" "}
												</dt>
												<dd className="inline">{alert.lastEventId}</dd>
											</div>
											<div>
												<dt className="inline font-medium">
													{t("operations.diagnostic")}:{" "}
												</dt>
												<dd className="inline">
													{alert.diagnostic ?? t("operations.noDiagnostic")}
												</dd>
											</div>
										</dl>
										<Link
											to={evidencePathByDimension[alert.dimension] ?? "/operations"}
											className="mt-2 inline-flex rounded-md border px-2 py-1 font-medium hover:bg-muted"
										>
											{t("operations.openEvidence")}
										</Link>
										{historyAlertId === alert.alertId && history.data?.length ? (
											<div className="mt-2 border-t pt-2">
												<p className="font-medium">{t("operations.lifecycleHistory")}</p>
												<ul className="mt-1 grid gap-1">
													{history.data.map((item) => (
														<li key={item.lifecycleId}>
															{label(`operations.states.${item.state}`)} · {item.actor} ·{" "}
															{item.eventId}
														</li>
													))}
												</ul>
											</div>
										) : null}
									</details>
									{alert.state === "active" ? (
										<Button
											size="sm"
											variant="outline"
											className="mt-2"
											disabled={acknowledge.isPending}
											onClick={() => acknowledge.mutate(alert.alertId)}
										>
											{t("operations.acknowledge")}
										</Button>
									) : null}
								</div>
							))
						) : (
							<p className="text-sm text-muted-foreground">
								{t("operations.noAlerts")}
							</p>
						)}
					</CardContent>
				</Card>
			</div>
			<Card>
				<CardHeader>
					<CardTitle>{t("operations.evidenceTitle")}</CardTitle>
					<CardDescription>{t("operations.evidenceDescription")}</CardDescription>
				</CardHeader>
				<CardContent className="grid gap-2 text-sm sm:grid-cols-4">
					{events.data?.slice(0, 8).map((event) => (
						<div className="rounded-md border px-3 py-2" key={event.eventId}>
							<div className="flex items-center justify-between gap-2">
								<span className="font-medium">{event.kind}</span>
								<Badge variant="outline">
									{label(`operations.dimensions.${event.dimension}`)}
								</Badge>
							</div>
							<p className="mt-1 text-xs text-muted-foreground">{event.entityId}</p>
							{event.diagnostic ? (
								<p className="mt-1 text-xs text-muted-foreground">{event.diagnostic}</p>
							) : null}
						</div>
					))}
					{evidenceLinks.map((link) => (
						<Link
							key={link.to}
							to={link.to}
							className="flex items-center justify-between rounded-md border px-3 py-2 hover:bg-muted"
						>
							<span>{t(link.labelKey)}</span>
							<span aria-hidden="true">→</span>
						</Link>
					))}
				</CardContent>
			</Card>
			<p className="text-xs text-muted-foreground">
				{t("operations.authorityBoundary")}
			</p>
		</div>
	);
}

export function SystemDashboard({
	projection,
}: {
	projection: SystemDashboardProjection;
}) {
	const { t } = useTranslation();
	const unavailable = (section: string) =>
		projection.unavailable.includes(section);
	const stateLabel = (group: string, value: string) =>
		t(`operations.${group}.${value}`, { defaultValue: value });
	const unresolvedAlerts = projection.alerts.filter(
		(alert) => alert.state !== "resolved",
	);
	const stale = (observedAtMs: number) => Date.now() - observedAtMs > 5 * 60_000;
	const researchItems = [
		["watchlist", projection.research.watchlistCount],
		["snapshots", projection.research.snapshotCount],
		["components", projection.research.componentCount],
		["modelArtifacts", projection.research.modelArtifactCount],
		["signalDatasets", projection.research.signalDatasetCount],
		["generationAttempts", projection.research.generationAttemptCount],
		["backtestRuns", projection.research.backtestRunCount],
		["validationProtocols", projection.research.validationProtocolCount],
		["validationReports", projection.research.validationReportCount],
		["feedbackSnapshots", projection.research.feedbackSnapshotCount],
		["feedbackReports", projection.research.feedbackReportCount],
		["reviewDecisions", projection.research.reviewDecisionCount],
	] as const;

	return (
		<main
			className="grid min-w-0 gap-4 p-4 md:p-6"
			aria-labelledby="system-dashboard-title"
		>
			<header className="flex flex-wrap items-start justify-between gap-3">
				<div>
					<p className="text-sm text-muted-foreground">
						{t("operations.systemDashboardEyebrow")}
					</p>
					<h1
						id="system-dashboard-title"
						className="text-2xl font-semibold tracking-tight"
					>
						{t("operations.systemDashboardTitle")}
					</h1>
					<p className="text-sm text-muted-foreground">
						{t("operations.systemDashboardDescription")}
					</p>
				</div>
				<Link
					to="/operations"
					className="rounded-md border px-3 py-2 text-sm font-medium hover:bg-muted"
				>
					{t("operations.openOperations")}
				</Link>
			</header>

			<div className="grid gap-4 lg:grid-cols-2">
				<Card>
					<CardHeader>
						<h2 className="text-base font-medium">
							{t("operations.systemStatusTitle")}
						</h2>
						<CardDescription>
							{t("operations.systemStatusDescription")}
						</CardDescription>
					</CardHeader>
					<CardContent className="space-y-3">
						{unavailable("health") ? (
							<ProjectionNotice
								message={t("operations.unavailableSection")}
								role="alert"
							/>
						) : projection.health.length ? (
							<div className="grid gap-2 sm:grid-cols-2">
								{projection.health.slice(0, 8).map((item) => (
									<div
										className={`rounded-md border p-3 ${healthTone[item.state]}`}
										key={`${item.entityId}-${item.dimension}`}
									>
										<div className="flex items-center justify-between gap-2">
											<span className="font-medium">
												{stateLabel("dimensions", item.dimension)}
											</span>
											<Badge variant="outline">{stateLabel("states", item.state)}</Badge>
										</div>
										<p className="mt-1 text-xs text-muted-foreground">
											{item.entityId}
											{item.required ? ` · ${t("operations.required")}` : ""}
										</p>
									</div>
								))}
							</div>
						) : (
							<ProjectionNotice message={t("operations.noHealth")} />
						)}
						<div className="flex flex-wrap items-center justify-between gap-2 border-t pt-3 text-sm">
							{unavailable("alerts") ? (
								<ProjectionNotice
									message={t("operations.unavailableSection")}
									role="alert"
								/>
							) : (
								<span>
									{t("operations.unresolvedCritical", {
										count: unresolvedAlerts.filter(
											(alert) => alert.severity === "critical",
										).length,
									})}
								</span>
							)}
							<Link
								to="/operations"
								className="font-medium text-primary hover:underline"
							>
								{t("operations.openEvidence")}
							</Link>
						</div>
					</CardContent>
				</Card>

				<Card>
					<CardHeader>
						<h2 className="text-base font-medium">{t("operations.alertsTitle")}</h2>
						<CardDescription>{t("operations.alertsDescription")}</CardDescription>
					</CardHeader>
					<CardContent className="space-y-2">
						{unavailable("alerts") ? (
							<ProjectionNotice
								message={t("operations.unavailableSection")}
								role="alert"
							/>
						) : projection.alerts.length ? (
							projection.alerts.slice(0, 6).map((alert) => (
								<div className="rounded-md border p-3" key={alert.alertId}>
									<div className="flex flex-wrap items-center justify-between gap-2">
										<span className="font-medium">{alert.condition}</span>
										<Badge
											variant={alert.severity === "critical" ? "destructive" : "outline"}
										>
											{stateLabel("severities", alert.severity)}
										</Badge>
									</div>
									<p className="mt-1 text-xs text-muted-foreground">
										{alert.entityId} · {stateLabel("alertStates", alert.state)}
									</p>
								</div>
							))
						) : (
							<ProjectionNotice message={t("operations.noSystemAlerts")} />
						)}
						<Link
							to="/operations"
							className="inline-flex rounded-md border px-2 py-1 text-sm font-medium hover:bg-muted"
						>
							{t("operations.openOperations")}
						</Link>
					</CardContent>
				</Card>

				<Card>
					<CardHeader>
						<h2 className="text-base font-medium">
							{t("operations.botSummaryTitle")}
						</h2>
						<CardDescription>{t("operations.botSummaryDescription")}</CardDescription>
					</CardHeader>
					<CardContent className="space-y-2">
						{unavailable("bots") ? (
							<ProjectionNotice
								message={t("operations.unavailableSection")}
								role="alert"
							/>
						) : projection.bots.length ? (
							projection.bots.map((bot) => (
								<div className="rounded-md border p-3" key={bot.botId}>
									<div className="flex flex-wrap items-center justify-between gap-2">
										<span className="font-medium">{bot.botId}</span>
										<Badge variant={bot.state === "faulted" ? "destructive" : "outline"}>
											{stateLabel("lifecycleStates", bot.state)}
										</Badge>
									</div>
									<p className="mt-1 text-xs text-muted-foreground">
										{t("operations.attemptsAndDecisions", {
											attempts: bot.attemptCount,
											decisions: bot.decisionCount,
										})}
									</p>
									{bot.reconciliationRequired ? (
										<p role="alert" className="mt-1 text-xs text-destructive">
											{t("operations.reconciliationRequired")}
										</p>
									) : null}
								</div>
							))
						) : (
							<ProjectionNotice message={t("operations.noBots")} />
						)}
						<Link
							to="/bots"
							className="inline-flex rounded-md border px-2 py-1 text-sm font-medium hover:bg-muted"
						>
							{t("operations.openBots")}
						</Link>
					</CardContent>
				</Card>

				<Card>
					<CardHeader>
						<h2 className="text-base font-medium">
							{t("operations.paperSummaryTitle")}
						</h2>
						<CardDescription>
							{t("operations.paperSummaryDescription")}
						</CardDescription>
					</CardHeader>
					<CardContent>
						{unavailable("paperAccount") ? (
							<ProjectionNotice
								message={t("operations.unavailableSection")}
								role="alert"
							/>
						) : projection.paperAccount ? (
							<div className="space-y-2 text-sm">
								<div className="flex flex-wrap items-center justify-between gap-2">
									<span className="font-medium">
										{projection.paperAccount.accountId}
									</span>
									<Badge variant="outline">
										{stateLabel("paperStates", projection.paperAccount.reconciliation)}
									</Badge>
								</div>
								<div className="grid gap-1 sm:grid-cols-2">
									<DashboardMetric
										label={t("operations.currency")}
										value={stateLabel("currencies", projection.paperAccount.currency)}
									/>
									<DashboardMetric
										label={t("operations.cash")}
										value={`${stateLabel("currencies", projection.paperAccount.currency)} ${formatDecimal(projection.paperAccount.cash)}`}
									/>
									<DashboardMetric
										label={t("operations.buyingPower")}
										value={formatDecimal(projection.paperAccount.buyingPower)}
									/>
									<DashboardMetric
										label={t("operations.reservedCash")}
										value={formatDecimal(projection.paperAccount.reservedCash)}
									/>
									<DashboardMetric
										label={t("operations.positions")}
										value={formatNumber(projection.paperAccount.positionCount)}
									/>
									<DashboardMetric
										label={t("operations.ordersAndFills")}
										value={`${formatNumber(projection.paperAccount.orderCount)} / ${formatNumber(projection.paperAccount.fillCount)}`}
									/>
									<DashboardMetric
										label={t("paperTrading.observed")}
										value={formatDateTime(projection.paperAccount.observedAtMs)}
									/>
								</div>
								{projection.paperAccount.restartRequired ? (
									<p role="alert" className="text-xs text-destructive">
										{t("operations.restartRequired")}
									</p>
								) : null}
								{stale(projection.paperAccount.observedAtMs) ? (
									<p
										role="status"
										className="text-xs text-amber-700 dark:text-amber-300"
									>
										{t("operations.staleProjection")}
									</p>
								) : null}
							</div>
						) : (
							<ProjectionNotice message={t("operations.noPaperAccount")} />
						)}
						<Link
							to="/paper-trading"
							className="mt-3 inline-flex rounded-md border px-2 py-1 text-sm font-medium hover:bg-muted"
						>
							{t("operations.openPaperTrading")}
						</Link>
					</CardContent>
				</Card>

				<Card>
					<CardHeader>
						<h2 className="text-base font-medium">
							{t("operations.researchSummaryTitle")}
						</h2>
						<CardDescription>
							{t("operations.researchSummaryDescription")}
						</CardDescription>
					</CardHeader>
					<CardContent>
						{unavailable("research") ? (
							<ProjectionNotice
								message={t("operations.unavailableSection")}
								role="alert"
							/>
						) : (
							<div className="grid gap-2 sm:grid-cols-2">
								{researchItems.map(([key, value]) => (
									<div
										className="flex items-center justify-between gap-2 rounded-md border px-3 py-2 text-sm"
										key={key}
									>
										<span>{t(`operations.research.${key}`)}</span>
										<span className="font-mono font-medium">{formatNumber(value)}</span>
									</div>
								))}
							</div>
						)}
						{unavailable("paperFeedback") ? (
							<p role="alert" className="mt-2 text-sm text-muted-foreground">
								{t("operations.unavailableSection")}
							</p>
						) : null}
						<div className="mt-3 flex flex-wrap gap-2">
							<Link
								to="/strategies"
								className="rounded-md border px-2 py-1 text-sm hover:bg-muted"
							>
								{t("operations.openResearch")}
							</Link>
							<Link
								to="/backtest"
								className="rounded-md border px-2 py-1 text-sm hover:bg-muted"
							>
								{t("operations.openBacktest")}
							</Link>
							<Link
								to="/paper-feedback"
								className="rounded-md border px-2 py-1 text-sm hover:bg-muted"
							>
								{t("nav.paperFeedback")}
							</Link>
						</div>
					</CardContent>
				</Card>
			</div>

			<Card>
				<CardHeader>
					<h2 className="text-base font-medium">{t("operations.evidenceTitle")}</h2>
					<CardDescription>
						{t("operations.systemEvidenceDescription")}
					</CardDescription>
				</CardHeader>
				<CardContent className="grid gap-2 text-sm sm:grid-cols-3">
					{unavailable("events") ? (
						<ProjectionNotice
							message={t("operations.unavailableSection")}
							role="alert"
						/>
					) : projection.events.length ? (
						projection.events.map((event) => (
							<div className="rounded-md border px-3 py-2" key={event.eventId}>
								<div className="flex items-center justify-between gap-2">
									<span className="font-medium">{event.kind}</span>
									<Badge variant="outline">
										{stateLabel("dimensions", event.dimension)}
									</Badge>
								</div>
								<p className="mt-1 text-xs text-muted-foreground">{event.entityId}</p>
							</div>
						))
					) : (
						<ProjectionNotice message={t("operations.noEvents")} />
					)}
					<Link
						to="/operations"
						className="flex items-center justify-between rounded-md border px-3 py-2 font-medium hover:bg-muted"
					>
						<span>{t("operations.openOperations")}</span>
						<span aria-hidden="true">→</span>
					</Link>
				</CardContent>
			</Card>

			<Card>
				<CardHeader>
					<h2 className="text-base font-medium">
						{t("operations.workspaceLinksTitle")}
					</h2>
					<CardDescription>
						{t("operations.workspaceLinksDescription")}
					</CardDescription>
				</CardHeader>
				<CardContent className="grid gap-2 sm:grid-cols-3 lg:grid-cols-4">
					{(
						[
							["/markets", "nav.marketsData"],
							["/features", "nav.featureEngineering"],
							["/factors", "nav.factorResearch"],
							["/models", "nav.models"],
							["/strategies", "nav.strategyLab"],
							["/backtest", "operations.openBacktest"],
							["/validation", "nav.validation"],
							["/components", "nav.components"],
						] as const
					).map(([to, labelKey]) => (
						<Link
							key={to}
							to={to}
							className="rounded-md border px-3 py-2 text-sm font-medium hover:bg-muted"
						>
							{t(labelKey)}
						</Link>
					))}
				</CardContent>
			</Card>
			<p className="text-xs text-muted-foreground">
				{t("operations.authorityBoundary")}
			</p>
		</main>
	);
}

export function SystemDashboardLoading() {
	const { t } = useTranslation();
	return (
		<main
			className="grid min-w-0 gap-4 p-4 md:p-6"
			aria-busy="true"
			aria-label={t("operations.systemDashboardTitle")}
		>
			<h1 className="text-2xl font-semibold tracking-tight">
				{t("operations.systemDashboardTitle")}
			</h1>
			<p role="status" className="text-sm text-muted-foreground">
				{t("operations.loading")}
			</p>
		</main>
	);
}

export function SystemDashboardUnavailable() {
	const { t } = useTranslation();
	return (
		<main
			className="grid min-w-0 gap-4 p-4 md:p-6"
			aria-labelledby="system-dashboard-unavailable-title"
		>
			<Card>
				<CardHeader>
					<h1
						id="system-dashboard-unavailable-title"
						className="text-2xl font-semibold tracking-tight"
					>
						{t("operations.systemDashboardTitle")}
					</h1>
					<CardDescription>
						{t("operations.systemDashboardUnavailable")}
					</CardDescription>
				</CardHeader>
				<CardContent>
					<p role="alert" className="text-sm text-destructive">
						{t("operations.loadError")}
					</p>
					<Link
						to="/operations"
						className="mt-3 inline-flex rounded-md border px-3 py-2 text-sm font-medium hover:bg-muted"
					>
						{t("operations.openOperations")}
					</Link>
				</CardContent>
			</Card>
		</main>
	);
}

function ProjectionNotice({
	message,
	role = "status",
}: {
	message: string;
	role?: "status" | "alert";
}) {
	return (
		<p role={role} className="text-sm text-muted-foreground">
			{message}
		</p>
	);
}

function DashboardMetric({ label, value }: { label: string; value: string }) {
	return (
		<div className="flex items-center justify-between gap-2 rounded-md border px-3 py-2">
			<span className="text-muted-foreground">{label}</span>
			<span className="font-mono font-medium">{value}</span>
		</div>
	);
}
