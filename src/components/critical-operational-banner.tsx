import { useAuthenticatedUserId } from "@/authenticated-user";
import { invoke } from "@tauri-apps/api/core";
import { Link } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";

type Alert = {
	alertId: string;
	condition: string;
	entityId: string;
	severity: "info" | "warning" | "critical";
	state: "active" | "acknowledged" | "resolved";
};

export function CriticalOperationalBanner() {
	const { t } = useTranslation();
	const userId = useAuthenticatedUserId();
	const alerts = useQuery({
		queryKey: ["operations-alerts", userId],
		queryFn: () => invoke<Alert[]>("operations_alerts"),
		enabled: Boolean(userId),
		refetchInterval: 15_000,
	});
	const critical = (alerts.data ?? []).filter(
		(alert) => alert.severity === "critical" && alert.state !== "resolved",
	);

	if (!critical.length) return null;

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
				<Link
					to="/operations"
					className="rounded-md border border-destructive/40 px-3 py-1 text-sm font-medium hover:bg-destructive/10"
				>
					{t("operations.openDashboard")}
				</Link>
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
