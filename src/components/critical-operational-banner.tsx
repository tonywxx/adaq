import { useAuthenticatedUserId } from "@/authenticated-user";
import { invoke } from "@tauri-apps/api/core";
import { Link } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";

type Alert = {
	alertId: string;
	condition: string;
	entityId: string;
	severity: "info" | "warning" | "critical";
	state: "active" | "acknowledged" | "resolved";
	lastObservedAtMs: number;
};

export function CriticalOperationalBanner() {
	const { t } = useTranslation();
	const userId = useAuthenticatedUserId();
	const [dismissedKey, setDismissedKey] = useState<string>();
	const alerts = useQuery({
		queryKey: ["operations-alerts", userId],
		queryFn: () => invoke<Alert[]>("operations_alerts"),
		enabled: Boolean(userId),
		refetchInterval: 15_000,
	});
	const critical = (alerts.data ?? []).filter(
		(alert) => alert.severity === "critical" && alert.state !== "resolved",
	);
	const criticalKey = critical
		.map((alert) => `${alert.alertId}:${alert.lastObservedAtMs}`)
		.sort()
		.join("|");

	if (!critical.length || criticalKey === dismissedKey) return null;

	return (
		<div
			role="alert"
			aria-live="assertive"
			className="sticky top-0 z-20 border-b border-destructive bg-destructive/10 px-4 py-3 text-destructive md:px-6"
		>
			<div className="flex flex-wrap items-center justify-between gap-2">
				<div>
					<strong>{t("operations.criticalBanner")}</strong>
					<p className="text-sm">
						{t("operations.criticalBannerDescription", { count: critical.length })}
					</p>
				</div>
				<div className="flex items-center gap-2">
					<Link
						to="/operations"
						className="rounded-md border border-destructive/40 px-3 py-1 text-sm font-medium hover:bg-destructive/10"
					>
						{t("operations.openDashboard")}
					</Link>
					<button
						type="button"
						className="rounded-md px-3 py-1 text-sm font-medium hover:bg-destructive/10"
						onClick={() => setDismissedKey(criticalKey)}
					>
						{t("operations.dismissBanner")}
					</button>
				</div>
			</div>
			<p className="mt-2 text-xs">
				{critical
					.slice(0, 3)
					.map((alert) => `${alert.entityId} · ${alert.condition}`)
					.join(" · ")}
			</p>
		</div>
	);
}
