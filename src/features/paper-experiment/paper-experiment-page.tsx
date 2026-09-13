import { useAuthenticatedUserId } from "@/authenticated-user";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { formatDateTime, formatDecimal } from "@/lib/i18n";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { Channel } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "@tanstack/react-router";

const instruments = ["BTC-USDT", "ETH-USDT", "SOL-USDT"] as const;
type Instrument = (typeof instruments)[number];
type ExperimentState =
	| "draft"
	| "preparing"
	| "armed"
	| "running"
	| "stopping"
	| "completed"
	| "incomplete";
type EvidenceState =
	| "notYetRealized"
	| "insufficientEvidence"
	| "ready"
	| "unknown"
	| "missing";

type Qualification = {
	qualificationId: string;
	candidateId: string;
	gate12Eligible: boolean;
	gate12ContinuationRequired: boolean;
};

type ConnectionProfile = {
	profileId: string;
	provider: string;
	accountId?: string | null;
	status: string;
};

type ExperimentView = {
	experiment: {
		experimentId: string;
		profileId: string;
		accountId: string;
		observationStartMs: number;
		observationEndMs: number;
		allocationTotalUsdt: string;
		unallocatedRemainderUsdt: string;
		state: ExperimentState;
		limitations: string[];
		instruments: Array<{
			instrument: Instrument;
			qualificationId: string;
			botId?: string | null;
			allocationUsdt: string;
			entryNotionalCapUsdt: string;
			reservedCashUsdt: string;
		}>;
		feedbackSnapshotIds: string[];
		feedbackReportIds: string[];
	};
	bots: Array<{
		botId: string;
		state: string;
		currentAttemptId?: string | null;
		control: {
			canStart: boolean;
			canRetry: boolean;
			canPause: boolean;
			canResume: boolean;
		};
			attempts: Array<{
				attemptId: string;
				unmanagedPositions: string[];
				reconciliationRequired: boolean;
				evidence: Array<{ code: string }>;
			}>;
	}>;
	valuations: Array<{
		instrument: Instrument;
		observedAtMs: number;
	}>;
	account?: {
		reconciliation: string;
		account: { cash: string };
	} | null;
	report?: {
		reportId: string;
		evidenceState: EvidenceState;
		rankingEligible: boolean;
		valuationCadenceMs: number;
		rankingBlockedReason?: string | null;
		rankedInstruments: string[];
		bestInstrument?: string | null;
		commonEndValuationAtMs?: number | null;
		limitations: string[];
		instruments: Array<{
			instrument: Instrument;
			botId: string;
			endingCashUsdt: string;
			endingPositionQuantity: string;
			endingPositionValueUsdt?: string | null;
			realizedPnlUsdt: string;
			unrealizedPnlUsdt?: string | null;
			feesUsdt: string;
			netEquityReturn?: string | null;
			maxDrawdown?: string | null;
			completedTrades: number;
			valuationCount: number;
			exposureTimeMs: number;
			interruptions: number;
			evidenceState: EvidenceState;
			limitations: string[];
		}>;
	} | null;
};

type TradeStreamEvent = { event?: string };

function utcValue(value: string) {
	if (!value) return undefined;
	const timestamp = Date.parse(
		value.length === 16 ? `${value}:00Z` : `${value}Z`,
	);
	return Number.isSafeInteger(timestamp) ? timestamp : undefined;
}

function commandId() {
	return `paper-experiment-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

function formatDurationMs(value: number) {
	if (value < 1_000) return `${value} ms`;
	const seconds = Math.floor(value / 1_000);
	if (seconds < 60) return `${seconds}s`;
	const minutes = Math.floor(seconds / 60);
	return `${minutes}m ${seconds % 60}s`;
}

function formatOptionalDecimal(value?: string | null) {
	return value == null ? "—" : formatDecimal(value);
}

export function PaperExperimentPage() {
	const { t } = useTranslation();
	const userId = useAuthenticatedUserId();
	const queryClient = useQueryClient();
	const queryKey = useMemo(
		() => ["paper-experiment", userId] as const,
		[userId],
	);
	const [profileId, setProfileId] = useState("");
	const [accountId, setAccountId] = useState("");
	const [start, setStart] = useState("");
	const [end, setEnd] = useState("");
	const [selection, setSelection] = useState<Record<Instrument, string>>({
		"BTC-USDT": "",
		"ETH-USDT": "",
		"SOL-USDT": "",
	});
	const [feedback, setFeedback] = useState("");
	const [pendingAction, setPendingAction] = useState("");
	const [pendingBotCommand, setPendingBotCommand] = useState("");
	const [streamStatus, setStreamStatus] = useState<
		"idle" | "connected" | "error"
	>("idle");

	const experiment = useQuery({
		queryKey,
		queryFn: () =>
			invoke<ExperimentView | null>("paper_experiment_view", { request: {} }),
		retry: false,
	});
	const qualifications = useQuery({
		queryKey: ["paper-experiment-qualifications", userId],
		queryFn: () =>
			invoke<Qualification[]>("strategy_qualification_list", {
				request: { userId },
			}),
		retry: false,
	});
	const profiles = useQuery({
		queryKey: ["paper-experiment-profiles", userId],
		queryFn: () =>
			invoke<ConnectionProfile[]>("connection_profile_list", {
				request: { userId },
			}),
		retry: false,
	});

	const eligibleQualifications = useMemo(
		() =>
			(qualifications.data ?? []).filter(
				(qualification) =>
					qualification.candidateId === "ema-double-cross-v1" &&
					qualification.gate12Eligible &&
					!qualification.gate12ContinuationRequired,
			),
		[qualifications.data],
	);
	const current = experiment.data;
	const experimentId = current?.experiment.experimentId;
	const experimentIsActive =
		current?.experiment.state === "preparing" ||
		current?.experiment.state === "armed" ||
		current?.experiment.state === "running";

	const setView = useCallback(
		(next: ExperimentView) => {
			queryClient.setQueryData<ExperimentView | null>(queryKey, next);
		},
		[queryClient, queryKey],
	);

	const create = useMutation({
		mutationFn: () => {
			const observationStartMs = utcValue(start);
			const observationEndMs = utcValue(end);
			if (
				!profileId ||
				!accountId ||
				observationStartMs === undefined ||
				observationEndMs === undefined ||
				observationEndMs <= observationStartMs ||
				instruments.some((instrument) => !selection[instrument])
			) {
				throw new Error(t("paperExperiment.invalidConfiguration"));
			}
			return invoke<ExperimentView>("paper_experiment_create", {
				request: {
					profileId,
					accountId,
					observationStartMs,
					observationEndMs,
					bindings: instruments.map((instrument) => ({
						instrument,
						qualificationId: selection[instrument],
					})),
				},
			});
		},
		onSuccess: (next) => {
			setView(next);
			setFeedback("");
		},
		onError: (error) => setFeedback(String(error)),
	});

	async function runAction(
		name:
			| "paper_experiment_launch"
			| "paper_experiment_refresh"
			| "paper_experiment_stop"
			| "paper_experiment_report_create"
			| "paper_experiment_feedback_create",
	) {
		if (!experimentId || pendingAction) return;
		setPendingAction(name);
		setFeedback("");
		try {
			const next = await invoke<ExperimentView | ExperimentView["report"]>(name, {
				request: { experimentId },
			});
			if (name === "paper_experiment_report_create") {
				await queryClient.invalidateQueries({ queryKey });
			} else if (name === "paper_experiment_feedback_create") {
				setView(next as ExperimentView);
			} else {
				setView(next as ExperimentView);
			}
		} catch (error) {
			setFeedback(String(error));
			await queryClient.invalidateQueries({ queryKey });
		} finally {
			setPendingAction("");
		}
	}

	async function controlBot(
		botId: string,
		command: "bot_start" | "bot_retry" | "bot_pause" | "bot_resume",
	) {
		setPendingBotCommand(`${command}:${botId}`);
		setFeedback("");
		try {
			await invoke(command, {
				request: { botId, commandId: commandId() },
			});
			await queryClient.invalidateQueries({ queryKey });
		} catch (error) {
			setFeedback(String(error));
		} finally {
			setPendingBotCommand("");
		}
	}

	useEffect(() => {
		if (!experimentId || !experimentIsActive) {
			setStreamStatus("idle");
			return;
		}
		let disposed = false;
		const subscriptionId = globalThis.crypto?.randomUUID?.() ?? commandId();
		const channel = new Channel<TradeStreamEvent>();
		channel.onmessage = (event) => {
			if (disposed) return;
			if (event.event === "connected") setStreamStatus("connected");
			if (event.event === "error" || event.event === "closed")
				setStreamStatus("error");
		};
		void invoke("market_subscribe_trades", {
			request: { src: "okx", codes: [...instruments], subscriptionId },
			onEvent: channel,
		}).catch(() => {
			if (!disposed) setStreamStatus("error");
		});
		return () => {
			disposed = true;
			void invoke("market_unsubscribe_trades", { request: { subscriptionId } });
		};
	}, [experimentIsActive, experimentId]);

	useEffect(() => {
		if (!experimentId || !experimentIsActive) return;
		const timer = window.setInterval(() => {
			void invoke<ExperimentView>("paper_experiment_refresh", {
				request: { experimentId },
				})
					.then(setView)
					.catch((error) => setFeedback(String(error)));
		}, 30_000);
		return () => window.clearInterval(timer);
	}, [experimentIsActive, experimentId, setView]);

	if (experiment.isPending) {
		return (
			<p className="p-6 text-sm text-muted-foreground">
				{t("paperExperiment.loading")}
			</p>
		);
	}
	if (experiment.isError) {
		return (
			<p className="p-6 text-sm text-destructive">
				{t("paperExperiment.unavailable")}
			</p>
		);
	}

	return (
		<div className="flex min-w-0 flex-1 flex-col gap-5 p-4 lg:p-6">
			<header>
				<p className="text-sm text-muted-foreground">
					{t("paperExperiment.eyebrow")}
				</p>
				<h1 className="text-2xl font-semibold">{t("paperExperiment.title")}</h1>
				<p className="text-sm text-muted-foreground">
					{t("paperExperiment.description")}
				</p>
			</header>
			{feedback ? (
				<p
					role="alert"
					className="rounded-lg border border-destructive/50 p-3 text-sm text-destructive"
				>
					{feedback}
				</p>
			) : null}
			{!current ? (
				<Card>
					<CardHeader>
						<CardTitle>{t("paperExperiment.createTitle")}</CardTitle>
						<CardDescription>
							{t("paperExperiment.createDescription")}
						</CardDescription>
					</CardHeader>
					<CardContent className="grid gap-4 md:grid-cols-2">
						<div className="grid gap-2">
							<Label htmlFor="experiment-profile">
								{t("paperExperiment.connection")}
							</Label>
							<select
								id="experiment-profile"
								value={profileId}
								onChange={(event) => {
									const next = profiles.data?.find(
										(profile) => profile.profileId === event.target.value,
									);
									setProfileId(event.target.value);
									setAccountId(next?.accountId ?? "");
								}}
								className="h-9 rounded-md border bg-background px-3 text-sm"
							>
								<option value="">{t("paperExperiment.selectConnection")}</option>
								{(profiles.data ?? [])
									.filter(
										(profile) =>
											profile.provider === "okx_demo" && profile.status === "usable",
									)
									.map((profile) => (
										<option key={profile.profileId} value={profile.profileId}>
											{profile.profileId} · {profile.accountId}
										</option>
									))}
							</select>
						</div>
						<div className="grid gap-2">
							<Label>{t("paperExperiment.account")}</Label>
							<p className="h-9 rounded-md border bg-muted/30 px-3 py-2 text-sm">
								{accountId || "—"}
							</p>
						</div>
						{instruments.map((instrument) => (
							<div className="grid gap-2" key={instrument}>
								<Label htmlFor={`experiment-${instrument}`}>
									{instrument} · {t("paperExperiment.qualification")}
								</Label>
								<select
									id={`experiment-${instrument}`}
									value={selection[instrument]}
									onChange={(event) =>
										setSelection((current) => ({
											...current,
											[instrument]: event.target.value,
										}))
									}
									className="h-9 rounded-md border bg-background px-3 text-sm"
								>
									<option value="">{t("paperExperiment.selectQualification")}</option>
									{eligibleQualifications.map((qualification) => (
										<option
											key={qualification.qualificationId}
											value={qualification.qualificationId}
										>
											{qualification.qualificationId}
										</option>
									))}
								</select>
							</div>
						))}
						<DateField
							id="experiment-start"
							label={t("paperExperiment.observationStart")}
							value={start}
							onChange={setStart}
						/>
						<DateField
							id="experiment-end"
							label={t("paperExperiment.observationEnd")}
							value={end}
							onChange={setEnd}
						/>
						<div className="md:col-span-2 flex justify-end">
							<Button
								loading={create.isPending}
								loadingText={t("paperExperiment.creating")}
								onClick={() => void create.mutateAsync()}
							>
								{t("paperExperiment.create")}
							</Button>
						</div>
					</CardContent>
				</Card>
			) : (
				<ExperimentEvidence
					current={current}
					streamStatus={streamStatus}
				onAction={runAction}
				pendingAction={pendingAction}
				onBotControl={controlBot}
					pendingBotCommand={pendingBotCommand}
					t={t}
				/>
			)}
		</div>
	);
}

function ExperimentEvidence({
	current,
	streamStatus,
	onAction,
	pendingAction,
	onBotControl,
	pendingBotCommand,
	t,
}: {
	current: ExperimentView;
	streamStatus: "idle" | "connected" | "error";
	pendingAction: string;
	onAction: (
		name:
			| "paper_experiment_launch"
			| "paper_experiment_refresh"
			| "paper_experiment_stop"
			| "paper_experiment_report_create"
			| "paper_experiment_feedback_create",
	) => Promise<void>;
	onBotControl: (
		botId: string,
		command: "bot_start" | "bot_retry" | "bot_pause" | "bot_resume",
	) => Promise<void>;
	pendingBotCommand: string;
	t: (key: string) => string;
}) {
	const { experiment, report, valuations, account, bots } = current;
	const canLaunch = experiment.state === "draft";
	const canStop =
		experiment.state === "preparing" ||
		experiment.state === "armed" ||
		experiment.state === "running" ||
		experiment.state === "stopping";
	const canControlBots =
		experiment.state === "preparing" ||
		experiment.state === "armed" ||
		experiment.state === "running";
	return (
		<>
			<Card>
				<CardHeader>
					<div className="flex flex-wrap items-start justify-between gap-3">
						<div>
							<CardTitle>{experiment.experimentId}</CardTitle>
							<CardDescription>
								{t("paperExperiment.window")}:{" "}
								{formatDateTime(experiment.observationStartMs, {
									timeZone: "UTC",
									timeZoneName: "short",
								})}{" "}
								→{" "}
								{formatDateTime(experiment.observationEndMs, {
									timeZone: "UTC",
									timeZoneName: "short",
								})}
							</CardDescription>
						</div>
						<Badge
							variant={experiment.state === "incomplete" ? "destructive" : "outline"}
						>
							{t(`paperExperiment.states.${experiment.state}`)}
						</Badge>
					</div>
				</CardHeader>
				<CardContent className="grid gap-3 text-sm md:grid-cols-5">
					<span>
						{t("paperExperiment.allocation")}:{" "}
						{formatDecimal(experiment.allocationTotalUsdt)} USDT
					</span>
					<span>
						{t("paperExperiment.unallocated")}:{" "}
						{formatDecimal(experiment.unallocatedRemainderUsdt)} USDT
					</span>
					<span>
						{t("paperExperiment.account")}: {experiment.accountId}
					</span>
					<span>
						{t("paperExperiment.valuations")}: {valuations.length}
					</span>
					<span>
						{t("paperExperiment.reconciliation")}: {account?.reconciliation ?? "—"}
					</span>
				</CardContent>
			</Card>
			{experiment.limitations.length ? (
				<Card className="border-amber-500/50">
					<CardContent className="grid gap-2 pt-6 text-sm">
						{experiment.limitations.map((limitation) => (
							<p className="text-muted-foreground" key={limitation}>
								{limitation}
							</p>
						))}
					</CardContent>
				</Card>
			) : null}
			<div className="flex flex-wrap gap-2">
				{canLaunch ? (
					<Button
						loading={pendingAction === "paper_experiment_launch"}
						loadingText={t("paperExperiment.launching")}
						disabled={pendingAction !== ""}
						onClick={() => void onAction("paper_experiment_launch")}
					>
						{t("paperExperiment.launch")}
					</Button>
				) : null}
				{experiment.state === "running" ? (
					<Button
						variant="outline"
						loading={pendingAction === "paper_experiment_refresh"}
						loadingText={t("paperExperiment.refreshing")}
						disabled={pendingAction !== ""}
						onClick={() => void onAction("paper_experiment_refresh")}
					>
						{t("paperExperiment.refresh")}
					</Button>
				) : null}
				{canStop ? (
					<Button
						variant="outline"
						loading={pendingAction === "paper_experiment_stop"}
						loadingText={t("paperExperiment.stopping")}
						disabled={pendingAction !== ""}
						onClick={() => void onAction("paper_experiment_stop")}
					>
						{t("paperExperiment.stop")}
					</Button>
				) : null}
				{experiment.state === "completed" || experiment.state === "incomplete" ? (
					<Button
						variant="outline"
						loading={pendingAction === "paper_experiment_report_create"}
						loadingText={t("paperExperiment.reporting")}
						disabled={pendingAction !== ""}
						onClick={() => void onAction("paper_experiment_report_create")}
					>
						{t("paperExperiment.report")}
					</Button>
				) : null}
				{report ? (
					<Button
						variant="outline"
						loading={pendingAction === "paper_experiment_feedback_create"}
						loadingText={t("paperExperiment.feedbacking")}
						disabled={pendingAction !== ""}
						onClick={() => void onAction("paper_experiment_feedback_create")}
					>
						{t("paperExperiment.feedback")}
					</Button>
				) : null}
				{streamStatus === "connected" ? (
					<Badge variant="secondary">{t("paperExperiment.streamConnected")}</Badge>
				) : null}
				{streamStatus === "error" ? (
					<Badge variant="destructive">
						{t("paperExperiment.streamUnavailable")}
					</Badge>
				) : null}
			</div>
			<div className="grid gap-4 lg:grid-cols-3">
				{experiment.instruments.map((binding) => {
					const instrumentReport = report?.instruments.find(
						(item) => item.instrument === binding.instrument,
					);
					const instrumentBot = bots.find((bot) => bot.botId === binding.botId);
					const instrumentAttempt = instrumentBot?.attempts.find(
						(attempt) => attempt.attemptId === instrumentBot.currentAttemptId,
					);
					const latestWarmupReset =
						instrumentAttempt?.evidence.reduce(
							(latest, evidence, index) =>
								evidence.code === "warmup-started" ||
								evidence.code === "worker-restarted-for-resume"
									? index
									: latest,
							-1,
						) ?? -1;
					const warmupComplete =
						instrumentAttempt?.evidence.some(
							(evidence, index) =>
								evidence.code === "warmup-complete" &&
								index > latestWarmupReset,
						) ?? false;
					return (
						<Card key={binding.instrument}>
							<CardHeader>
								<CardTitle>{binding.instrument}</CardTitle>
								<CardDescription>
									{t("paperExperiment.bots")}: {instrumentBot?.state ?? "—"}
								</CardDescription>
							</CardHeader>
							<CardContent className="grid gap-2 text-sm">
								<div>
									{t("paperExperiment.allocation")}:{" "}
									{formatDecimal(binding.allocationUsdt)} USDT
								</div>
								<div>
									{t("paperExperiment.entryCap")}: {" "}
									{formatDecimal(binding.entryNotionalCapUsdt)} USDT · {" "}
									{t("paperExperiment.reservedCash")}: {" "}
									{formatDecimal(binding.reservedCashUsdt)} USDT
								</div>
								<div>
									{t("paperExperiment.qualification")}: {binding.qualificationId}
								</div>
								<div>
									{t("paperExperiment.warmup")}: {warmupComplete
										? t("paperExperiment.warmupComplete")
										: t("paperExperiment.warmupPending")}
								</div>
								{instrumentBot ? (
									<div>
										{t("paperExperiment.runtimeAttempts")}:{" "}
										{instrumentBot.attempts.length}
									</div>
								) : null}
								{instrumentAttempt?.reconciliationRequired ? (
									<p className="text-amber-600">
										{t("paperExperiment.reconciliationRequired")}
									</p>
								) : null}
								{instrumentAttempt?.unmanagedPositions.length ? (
									<p className="text-amber-600">
										{t("paperExperiment.unmanagedPositions")}:{" "}
										{instrumentAttempt.unmanagedPositions.join(", ")}
									</p>
								) : null}
								{canControlBots && instrumentBot ? (
									<div className="flex flex-wrap gap-2">
										{instrumentBot.control.canStart ? (
											<Button
												size="sm"
												disabled={pendingBotCommand !== ""}
												onClick={() => void onBotControl(instrumentBot.botId, "bot_start")}
											>
												{t("bots.start")}
											</Button>
										) : null}
										{instrumentBot.control.canRetry ? (
											<Button
												size="sm"
												disabled={pendingBotCommand !== ""}
												onClick={() => void onBotControl(instrumentBot.botId, "bot_retry")}
											>
												{t("bots.retry")}
											</Button>
										) : null}
										{instrumentBot.control.canPause ? (
											<Button
												size="sm"
												variant="outline"
												disabled={pendingBotCommand !== ""}
												onClick={() => void onBotControl(instrumentBot.botId, "bot_pause")}
											>
												{t("bots.pause")}
											</Button>
										) : null}
										{instrumentBot.control.canResume ? (
											<Button
												size="sm"
												variant="outline"
												disabled={pendingBotCommand !== ""}
												onClick={() => void onBotControl(instrumentBot.botId, "bot_resume")}
											>
												{t("bots.resume")}
											</Button>
										) : null}
									</div>
								) : null}
								{instrumentReport ? (
									<>
										<div>
											{t("paperExperiment.cash")}:{" "}
											{formatDecimal(instrumentReport.endingCashUsdt)}
										</div>
										<div>
											{t("paperExperiment.position")}:{" "}
											{formatDecimal(instrumentReport.endingPositionQuantity)} /{" "}
										{formatOptionalDecimal(instrumentReport.endingPositionValueUsdt)}
										</div>
										<div>
											{t("paperExperiment.pnl")}:{" "}
											{formatDecimal(instrumentReport.realizedPnlUsdt)} /{" "}
										{formatOptionalDecimal(instrumentReport.unrealizedPnlUsdt)}
										</div>
										<div>
											{t("paperExperiment.fees")}:{" "}
											{formatDecimal(instrumentReport.feesUsdt)}
										</div>
										<div>
											{t("paperExperiment.trades")}: {instrumentReport.completedTrades}
										</div>
										<div>
											{t("paperExperiment.equityReturn")}:{" "}
										{formatOptionalDecimal(instrumentReport.netEquityReturn)}
										</div>
										<div>
											{t("paperExperiment.exposure")}:{" "}
											{formatDurationMs(instrumentReport.exposureTimeMs)}
										</div>
										<div>
											{t("paperExperiment.interruptions")}:{" "}
											{instrumentReport.interruptions}
										</div>
										<div>
											{t("paperExperiment.valuationCount")}:{" "}
											{instrumentReport.valuationCount}
										</div>
										<div>
											{t("paperExperiment.maxDrawdown")}:{" "}
										{formatOptionalDecimal(instrumentReport.maxDrawdown)}
										</div>
										<Badge variant="outline">
											{t(`paperExperiment.states.${instrumentReport.evidenceState}`)}
										</Badge>
									</>
								) : (
									<p className="text-muted-foreground">
										{t("paperExperiment.valuations")}:{" "}
										{
											valuations.filter(
												(valuation) => valuation.instrument === binding.instrument,
											).length
										}
									</p>
								)}
							</CardContent>
						</Card>
					);
				})}
			</div>
			{report ? (
				<Card>
					<CardHeader>
						<CardTitle>{t("paperExperiment.evidence")}</CardTitle>
						<CardDescription>
							{report.reportId} · {t(`paperExperiment.states.${report.evidenceState}`)}
						</CardDescription>
					</CardHeader>
					<CardContent className="grid gap-2 text-sm">
						<div>
							{report.rankingEligible
								? t("paperExperiment.rankingEligible")
								: t("paperExperiment.rankingBlocked")}
						</div>
						{report.rankingBlockedReason ? (
							<p className="text-muted-foreground">{report.rankingBlockedReason}</p>
						) : null}
						{report.rankingEligible && report.bestInstrument ? (
							<p>
								{t("paperExperiment.bestInstrument")}: {report.bestInstrument}
							</p>
						) : null}
						{report.rankedInstruments.length ? (
							<p>
								{t("paperExperiment.rankingOrder")}: {report.rankedInstruments.join(" → ")}
							</p>
						) : null}
						{report.commonEndValuationAtMs ? (
							<p>
								{t("paperExperiment.commonEndValuation")}:{" "}
								{formatDateTime(report.commonEndValuationAtMs, {
									timeZone: "UTC",
									timeZoneName: "short",
								})}
							</p>
						) : null}
						<p>
							{t("paperExperiment.valuationCadence")}:{" "}
							{formatDurationMs(report.valuationCadenceMs)}
						</p>
						{report.limitations.map((limitation) => (
							<p className="text-muted-foreground" key={limitation}>
								{limitation}
							</p>
						))}
						{experiment.feedbackSnapshotIds.length ||
						experiment.feedbackReportIds.length ? (
							<div className="grid gap-2">
								{experiment.feedbackSnapshotIds.length ? (
									<p>
										{t("paperExperiment.feedbackSnapshots")}:{" "}
										{experiment.feedbackSnapshotIds.join(", ")}
									</p>
								) : null}
								{experiment.feedbackReportIds.length ? (
									<p>
										{t("paperExperiment.feedbackIds")}:{" "}
										{experiment.feedbackReportIds.join(", ")}
									</p>
								) : null}
							</div>
						) : (
							<Link className="underline" to="/paper-feedback">
								{t("nav.paperFeedback")}
							</Link>
						)}
					</CardContent>
				</Card>
			) : null}
		</>
	);
}

function DateField({
	id,
	label,
	value,
	onChange,
}: {
	id: string;
	label: string;
	value: string;
	onChange: (value: string) => void;
}) {
	return (
		<div className="grid gap-2">
			<Label htmlFor={id}>{label}</Label>
			<input
				id={id}
				type="datetime-local"
				value={value}
				onChange={(event) => onChange(event.target.value)}
				className="h-9 rounded-md border bg-background px-3 text-sm"
			/>
		</div>
	);
}
