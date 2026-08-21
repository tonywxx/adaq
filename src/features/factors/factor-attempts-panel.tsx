import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "@/components/ui/card";
import { ResearchContextEvidence } from "@/features/research/research-context-evidence";
import { formatDateTime, formatNumber } from "@/lib/i18n";
import {
	formatFactorError,
	isTerminalFactorAttempt,
	shortFactorHash,
} from "./factor-data";
import type { FactorAdapter } from "./factor-adapter";
import type { FactorAttemptView } from "./factor-types";
import { useFactorPage } from "./factor-workspace-data";
import { EmptyState, Feedback, LoadingState } from "./factor-workspace-support";

export function AttemptsPanel({
	userId,
	adapter,
	kind,
	refreshKey = 0,
}: {
	userId: string;
	adapter: FactorAdapter;
	kind: string;
	refreshKey?: number;
}) {
	const { t } = useTranslation();
	const attempts = useFactorPage(
		userId,
		`attempts:${kind}`,
		adapter.listAttempts,
	);
	const [feedback, setFeedback] = useState(undefined as string | undefined);
	const [actionKey, setActionKey] = useState(undefined as string | undefined);
	useEffect(() => {
		if (refreshKey > 0) void attempts.load();
	}, [attempts.load, refreshKey]);
	useEffect(() => {
		if (
			!attempts.data?.items.some(
				(item) => item.kind === kind && !isTerminalFactorAttempt(item.status),
			)
		)
			return;
		const timer = window.setInterval(
			() => void attempts.load(attempts.data?.page ?? 1),
			3_000,
		);
		return () => window.clearInterval(timer);
	}, [attempts.data, attempts.load, kind]);
	const visible =
		attempts.data?.items.filter((item) => item.kind === kind) ?? [];
	const action = async (
		attempt: FactorAttemptView,
		type: "cancel" | "retry",
	) => {
		const key = `${attempt.attemptId}:${type}`;
		setActionKey(key);
		setFeedback(undefined);
		try {
			if (type === "cancel")
				await adapter.cancelAttempt(userId, attempt.attemptId);
			else await adapter.retryAttempt(userId, attempt.attemptId);
			await attempts.load(attempts.data?.page ?? 1);
		} catch (error) {
			setFeedback(formatFactorError(error));
		} finally {
			setActionKey(undefined);
		}
	};
	return (
		<Card>
			<CardHeader>
				<CardTitle>{t("factors.attempts.heading")}</CardTitle>
				<CardDescription>{t("factors.attempts.description")}</CardDescription>
			</CardHeader>
			<CardContent className="space-y-3">
				{feedback && <Feedback message={feedback} />}
				{attempts.loading && !attempts.data ? (
					<LoadingState label={t("factors.loading")} />
				) : visible.length === 0 ? (
					<EmptyState message={t("factors.attempts.empty")} />
				) : (
					visible.map((attempt) => (
						<div key={attempt.attemptId} className="rounded-md border p-3 text-sm">
							<ResearchContextEvidence userId={userId} attemptId={attempt.attemptId} />
							<div className="flex flex-wrap items-center gap-2">
								<span className="font-mono text-xs">
									{shortFactorHash(attempt.attemptId, 12)}
								</span>
								<Badge
									variant={attempt.status === "completed" ? "secondary" : "outline"}
								>
									{t(`factors.status.${attempt.status}`)}
								</Badge>
								<span className="text-xs text-muted-foreground">
									{formatNumber(attempt.completedUnits)} /{" "}
									{formatNumber(attempt.progressTotal)}
								</span>
								<span className="text-xs text-muted-foreground">
									{formatDateTime(attempt.createdAtMs, {
										dateStyle: "medium",
										timeStyle: "short",
									})}
								</span>
								<div className="ml-auto flex gap-2">
									{attempt.status === "pending" || attempt.status === "running" ? (
										<Button
											type="button"
											size="sm"
											variant="outline"
											loading={actionKey === `${attempt.attemptId}:cancel`}
											onClick={() => void action(attempt, "cancel")}
										>
											{t("factors.attempts.cancel")}
										</Button>
									) : null}
									{attempt.status === "failed" || attempt.status === "cancelled" ? (
										<Button
											type="button"
											size="sm"
											variant="outline"
											loading={actionKey === `${attempt.attemptId}:retry`}
											onClick={() => void action(attempt, "retry")}
										>
											{t("factors.attempts.retry")}
										</Button>
									) : null}
								</div>
							</div>
							{attempt.progressTotal > 0 ? (
								<progress
									className="mt-3 h-2 w-full"
									value={attempt.completedUnits}
									max={attempt.progressTotal}
									aria-label={t("factors.attempts.progress")}
								/>
							) : null}
							{attempt.diagnostic ? (
								<p className="mt-2 break-words text-xs text-destructive">
									{attempt.diagnostic}
								</p>
							) : null}
						</div>
					))
				)}
			</CardContent>
		</Card>
	);
}
