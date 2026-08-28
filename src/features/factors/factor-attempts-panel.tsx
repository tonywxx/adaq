import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
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
import { isTerminalFactorAttempt, shortFactorHash } from "./factor-data";
import type { FactorAdapter } from "./factor-adapter";
import type { FactorAttemptView } from "./factor-types";
import { useFactorPage } from "./factor-workspace-data";
import {
	EmptyState,
	FactorAttemptStatusBadge,
	Feedback,
	LoadingState,
	localizedFactorAttemptCode,
	localizedFactorError,
	PageControls,
} from "./factor-workspace-support";

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
	const listAttempts = useCallback(
		(requestedUserId: string, page: number) =>
			adapter.listAttempts(requestedUserId, page, kind),
		[adapter.listAttempts, kind],
	);
	const attempts = useFactorPage(userId, `attempts:${kind}`, listAttempts);
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
	const diagnosticMessage = (attempt: FactorAttemptView) => {
		if (attempt.failureCode === "cancelled")
			return t("factors.attempts.cancelledDiagnostic");
		if (attempt.failureCode === "research-interrupted")
			return t("factors.attempts.recoveredDiagnostic");
		if (attempt.diagnostic === "Factor research Attempt cancellation requested")
			return t("factors.attempts.cancellationRequestedDiagnostic");
		return attempt.diagnostic;
	};
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
			setFeedback(localizedFactorError(error, t));
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
				{attempts.error && (
					<Feedback message={localizedFactorError(attempts.error, t)} />
				)}
				{attempts.loading && !attempts.data ? (
					<LoadingState label={t("factors.loading")} />
				) : attempts.error && !attempts.data ? null : visible.length === 0 ? (
					<EmptyState message={t("factors.attempts.empty")} />
				) : (
					<>
						{visible.map((attempt) => {
							const diagnostic = diagnosticMessage(attempt);
							return (
								<fieldset
									key={attempt.attemptId}
									className="rounded-md border p-3 text-sm"
									aria-busy={!isTerminalFactorAttempt(attempt.status)}
									aria-label={t("factors.attempts.attemptLabel", {
										id: shortFactorHash(attempt.attemptId, 12),
									})}
								>
									<ResearchContextEvidence
										userId={userId}
										attemptId={attempt.attemptId}
									/>
									<div className="flex flex-wrap items-center gap-2">
										<span className="font-mono text-xs">
											{shortFactorHash(attempt.attemptId, 12)}
										</span>
										<span aria-live="polite" aria-atomic="true">
											<FactorAttemptStatusBadge status={attempt.status} />
										</span>
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
									{diagnostic || attempt.failureCode ? (
										<p
											className="mt-2 break-words text-xs text-destructive"
											role="alert"
											aria-live="polite"
										>
											{attempt.failureCode ? (
												<span className="font-medium">
													{localizedFactorAttemptCode(attempt.failureCode, t)} ({" "}
													<code>{attempt.failureCode}</code>):{" "}
												</span>
											) : null}
											{diagnostic}
										</p>
									) : null}
									<dl className="mt-3 grid gap-x-6 gap-y-1 text-xs text-muted-foreground sm:grid-cols-2">
										<div>
											<dt className="inline font-medium">
												{t("factors.attempts.requestHash")}:{" "}
											</dt>
											<dd className="inline font-mono" title={attempt.requestHash}>
												{shortFactorHash(attempt.requestHash, 16)}
											</dd>
										</div>
										{attempt.sourceAttemptId ? (
											<div>
												<dt className="inline font-medium">
													{t("factors.attempts.sourceAttempt")}:{" "}
												</dt>
												<dd className="inline font-mono" title={attempt.sourceAttemptId}>
													{shortFactorHash(attempt.sourceAttemptId, 16)}
												</dd>
											</div>
										) : null}
										{attempt.resultId ? (
											<div>
												<dt className="inline font-medium">
													{t("factors.attempts.result")}:{" "}
												</dt>
												<dd className="inline font-mono" title={attempt.resultId}>
													{shortFactorHash(attempt.resultId, 16)}
												</dd>
											</div>
										) : null}
									</dl>
								</fieldset>
							);
						})}
						{attempts.data && attempts.data.total > attempts.data.pageSize ? (
							<PageControls
								page={attempts.data.page}
								total={attempts.data.total}
								pageSize={attempts.data.pageSize}
								onPage={(page) => void attempts.load(page)}
							/>
						) : null}
					</>
				)}
			</CardContent>
		</Card>
	);
}
