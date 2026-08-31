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

type Health = {
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

type Alert = {
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

type Event = {
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
