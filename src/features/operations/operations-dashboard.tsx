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
import { useMarketSessionStore } from "@/lib/market-session";
import { invoke } from "@tauri-apps/api/core";
import { Link } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";

type Health = {
	entityId: string;
	dimension: string;
	state: "healthy" | "degraded" | "critical" | "unknown";
	required: boolean;
	observedAtMs: number;
	eventId: string;
};
type Alert = {
	alertId: string;
	entityId: string;
	dimension: string;
	condition: string;
	severity: "info" | "warning" | "critical";
	state: "active" | "acknowledged" | "resolved";
	safetyAction: string;
	lastEventId: string;
};

const healthTone: Record<Health["state"], string> = {
	healthy: "border-emerald-500/40 bg-emerald-500/5",
	degraded: "border-amber-500/40 bg-amber-500/5",
	critical: "border-destructive/50 bg-destructive/5",
	unknown: "bg-muted/40",
};

export function OperationsDashboard() {
	const { t } = useTranslation();
	const userId = useMarketSessionStore((state) => state.userId);
	const [filter, setFilter] = useState<"all" | Alert["state"]>("all");
	const health = useQuery({
		queryKey: ["operations-health", userId],
		queryFn: () => invoke<Health[]>("operations_health", { userId }),
		enabled: Boolean(userId),
		refetchInterval: 15_000,
	});
	const alerts = useQuery({
		queryKey: ["operations-alerts", userId],
		queryFn: () => invoke<Alert[]>("operations_alerts", { userId }),
		enabled: Boolean(userId),
		refetchInterval: 15_000,
	});
	const values = alerts.data ?? [];
	const critical = values.filter(
		(alert) => alert.severity === "critical" && alert.state !== "resolved",
	);
	const visible = values.filter(
		(alert) => filter === "all" || alert.state === filter,
	);
	const acknowledge = async (alert: Alert) => {
		if (!userId || alert.state !== "active") return;
		await invoke("operations_alert_transition", {
			userId,
			alertId: alert.alertId,
			state: "acknowledged",
			eventId: alert.lastEventId,
			occurredAtMs: Date.now(),
		});
		await alerts.refetch();
	};
	const label = (key: string) => t(key, { defaultValue: key.split(".").at(-1) });

	return (
		<div className="grid gap-4 p-4 md:p-6">
			<header>
				<h1 className="text-2xl font-semibold tracking-tight">
					{t("operations.title")}
				</h1>
				<p className="text-sm text-muted-foreground">
					{t("operations.description")}
				</p>
			</header>
			{critical.length ? (
				<div
					role="alert"
					className="rounded-lg border border-destructive bg-destructive/10 p-4 text-destructive"
				>
					<strong>{t("operations.criticalBanner")}</strong>
					<p className="mt-1 text-sm">
						{t("operations.criticalBannerDescription", { count: critical.length })}
					</p>
				</div>
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
						<CardAction>
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
										{alert.entityId} · {label(`operations.actions.${alert.safetyAction}`)}
									</p>
									{alert.state === "active" ? (
										<Button
											size="sm"
											variant="outline"
											className="mt-2"
											onClick={() => void acknowledge(alert)}
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
					{(["/factors", "/markets/crypto"] as const).map((to) => (
						<Link
							key={to}
							to={to}
							className="flex items-center justify-between rounded-md border px-3 py-2 hover:bg-muted"
						>
							<span>
								{to === "/factors"
									? t("nav.factorResearch")
									: to.replace("/markets/", "")}
							</span>
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
