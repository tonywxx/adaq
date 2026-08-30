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
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";

type Qualification = {
	qualificationId: string;
	candidateId: string;
	candidateRevision: number;
	candidateRevisionHash: string;
	gate12Eligible: boolean;
	gate12ContinuationRequired: boolean;
};

type ConnectionProfile = {
	profileId: string;
	provider: string;
	accountId?: string | null;
	status: string;
};

type BotView = {
	botId: string;
	state: string;
	currentAttemptId?: string | null;
	bundle: {
		identity: string;
		qualificationId: string;
		candidateId: string;
		candidateRevision: number;
		accountId: string;
		connectionProfileId: string;
		schedule:
			| { type: "closed-bar"; instrumentId: string; interval: string }
			| {
					type: "scheduled-cross-section";
					universeId: string;
					instruments: string[];
			  };
	};
	attempts: Array<{
		attemptId: string;
		state: string;
		reconciliationRequired: boolean;
		unmanagedPositions: string[];
		events: Array<{
			from: string;
			to: string;
			actor: string;
			reason: string;
		}>;
		evidence: Array<{
			kind: string;
			code: string;
			detail: string;
			relatedId?: string | null;
			observedAtMs: number;
		}>;
		decisions: Array<{
			requestId: string;
			decisionId: string;
			outcome: string;
			targetHash?: string | null;
			observedAtMs: number;
		}>;
		orders: Array<{
			operationId: string;
			decisionId?: string | null;
			status: string;
			providerOrderId?: string | null;
			observedAtMs: number;
		}>;
	}>;
	control: {
		canStart: boolean;
		canRetry: boolean;
		canPause: boolean;
		canResume: boolean;
		canStop: boolean;
		canFlatten: boolean;
	};
};

type ScheduleKind = "closed-bar" | "scheduled-cross-section";

const commandId = () => globalThis.crypto.randomUUID();

export function BotsPage() {
	const { t } = useTranslation();
	const userId = useAuthenticatedUserId();
	const queryClient = useQueryClient();
	const flattenDialog = useRef<HTMLDialogElement>(null);
	const [qualificationId, setQualificationId] = useState("");
	const [profileId, setProfileId] = useState("");
	const [accountId, setAccountId] = useState("");
	const [scheduleKind, setScheduleKind] = useState<ScheduleKind>("closed-bar");
	const [instrumentId, setInstrumentId] = useState("");
	const [interval, setInterval] = useState("1m");
	const [universeId, setUniverseId] = useState("");
	const [instruments, setInstruments] = useState("");
	const [feedback, setFeedback] = useState("");
	const [pendingCommand, setPendingCommand] = useState("");
	const [flattenBotId, setFlattenBotId] = useState("");

	const bots = useQuery({
		queryKey: ["bots", userId],
		queryFn: () => invoke<BotView[]>("bot_list"),
		retry: false,
	});
	const qualifications = useQuery({
		queryKey: ["bot-qualifications", userId],
		queryFn: () =>
			invoke<Qualification[]>("strategy_qualification_list", {
				request: { userId },
			}),
		retry: false,
	});
	const profiles = useQuery({
		queryKey: ["bot-profiles", userId],
		queryFn: () =>
			invoke<ConnectionProfile[]>("connection_profile_list", {
				request: { userId },
			}),
		retry: false,
	});

	const deploy = useMutation({
		mutationFn: () =>
			invoke<BotView>("bot_deploy", {
				request: {
					qualificationId,
					profileId,
					accountId,
					schedule:
						scheduleKind === "closed-bar"
							? { type: "closed-bar", instrumentId, interval }
							: {
									type: "scheduled-cross-section",
									universeId,
									instruments: instruments
										.split(",")
										.map((value) => value.trim())
										.filter(Boolean),
								},
				},
			}),
		onSuccess: async () => {
			setFeedback(t("bots.deployed"));
			await queryClient.invalidateQueries({ queryKey: ["bots", userId] });
		},
		onError: (error) => setFeedback(String(error)),
	});

	const selectedProfile = profiles.data?.find(
		(profile) => profile.profileId === profileId,
	);
	const eligibleQualifications =
		qualifications.data?.filter(
			(qualification) =>
				qualification.gate12Eligible && !qualification.gate12ContinuationRequired,
		) ?? [];

	async function control(
		botId: string,
		command: "bot_start" | "bot_retry" | "bot_pause" | "bot_resume",
	) {
		setPendingCommand(`${command}:${botId}`);
		setFeedback("");
		try {
			await invoke<BotView>(command, {
				request: { botId, commandId: commandId() },
			});
			await queryClient.invalidateQueries({ queryKey: ["bots", userId] });
		} catch (error) {
			setFeedback(String(error));
		} finally {
			setPendingCommand("");
		}
	}

	async function stop(botId: string, policy: "keep-position" | "flatten") {
		setPendingCommand(`stop:${botId}`);
		setFeedback("");
		try {
			await invoke<BotView>("bot_stop", {
				request: {
					botId,
					commandId: commandId(),
					policy,
					confirmFlatten: policy === "flatten",
				},
			});
			await queryClient.invalidateQueries({ queryKey: ["bots", userId] });
			flattenDialog.current?.close();
		} catch (error) {
			setFeedback(String(error));
		} finally {
			setPendingCommand("");
		}
	}

	return (
		<div className="grid gap-4 p-4 md:p-6">
			<header>
				<p className="text-sm text-muted-foreground">{t("bots.eyebrow")}</p>
				<h1 className="text-2xl font-semibold tracking-tight">{t("bots.title")}</h1>
				<p className="text-sm text-muted-foreground">{t("bots.description")}</p>
			</header>

			{feedback ? (
				<p
					role="alert"
					className="rounded-lg border border-destructive/50 p-3 text-sm text-destructive"
				>
					{feedback}
				</p>
			) : null}

			<Card>
				<CardHeader>
					<CardTitle>{t("bots.deployTitle")}</CardTitle>
					<CardDescription>{t("bots.deployDescription")}</CardDescription>
				</CardHeader>
				<CardContent className="grid gap-4 md:grid-cols-2">
					<div className="grid gap-2">
						<Label htmlFor="bot-qualification">{t("bots.qualification")}</Label>
						<select
							id="bot-qualification"
							value={qualificationId}
							onChange={(event) => setQualificationId(event.target.value)}
							className="h-9 rounded-md border bg-background px-3 text-sm"
						>
							<option value="">{t("bots.selectQualification")}</option>
							{eligibleQualifications.map((qualification) => (
								<option
									key={qualification.qualificationId}
									value={qualification.qualificationId}
								>
									{qualification.qualificationId} · {qualification.candidateId} r
									{qualification.candidateRevision}
								</option>
							))}
						</select>
						<p className="text-xs text-muted-foreground">
							{t("bots.exactQualificationHint")}
						</p>
					</div>
					<div className="grid gap-2">
						<Label htmlFor="bot-profile">{t("bots.connection")}</Label>
						<select
							id="bot-profile"
							value={profileId}
							onChange={(event) => {
								setProfileId(event.target.value);
								setAccountId(
									profiles.data?.find(
										(profile) => profile.profileId === event.target.value,
									)?.accountId ?? "",
								);
							}}
							className="h-9 rounded-md border bg-background px-3 text-sm"
						>
							<option value="">{t("bots.selectConnection")}</option>
							{(profiles.data ?? [])
								.filter(
									(profile) =>
										profile.provider === "okx_demo" &&
										profile.status === "usable" &&
										profile.accountId,
								)
								.map((profile) => (
									<option key={profile.profileId} value={profile.profileId}>
										{profile.profileId} · {profile.accountId}
									</option>
								))}
						</select>
						<p className="text-xs text-muted-foreground">{t("bots.okxDemoOnly")}</p>
					</div>
					<div className="grid gap-2">
						<Label htmlFor="bot-schedule">{t("bots.schedule")}</Label>
						<select
							id="bot-schedule"
							value={scheduleKind}
							onChange={(event) => setScheduleKind(event.target.value as ScheduleKind)}
							className="h-9 rounded-md border bg-background px-3 text-sm"
						>
							<option value="closed-bar">{t("bots.closedBar")}</option>
							<option value="scheduled-cross-section">
								{t("bots.scheduledCrossSection")}
							</option>
						</select>
					</div>
					{scheduleKind === "closed-bar" ? (
						<>
							<div className="grid gap-2">
								<Label htmlFor="bot-instrument">{t("bots.instrument")}</Label>
								<input
									id="bot-instrument"
									value={instrumentId}
									onChange={(event) => setInstrumentId(event.target.value)}
									placeholder={t("bots.instrumentHint")}
									className="h-9 rounded-md border bg-background px-3 text-sm"
								/>
							</div>
							<div className="grid gap-2">
								<Label htmlFor="bot-interval">{t("bots.interval")}</Label>
								<input
									id="bot-interval"
									value={interval}
									onChange={(event) => setInterval(event.target.value)}
									className="h-9 rounded-md border bg-background px-3 text-sm"
								/>
							</div>
						</>
					) : (
						<>
							<div className="grid gap-2">
								<Label htmlFor="bot-universe">{t("bots.universe")}</Label>
								<input
									id="bot-universe"
									value={universeId}
									onChange={(event) => setUniverseId(event.target.value)}
									className="h-9 rounded-md border bg-background px-3 text-sm"
								/>
							</div>
							<div className="grid gap-2">
								<Label htmlFor="bot-instruments">{t("bots.instruments")}</Label>
								<input
									id="bot-instruments"
									value={instruments}
									onChange={(event) => setInstruments(event.target.value)}
									placeholder={t("bots.instrumentsHint")}
									className="h-9 rounded-md border bg-background px-3 text-sm"
								/>
							</div>
						</>
					)}
					<div className="flex items-end">
						<Button
							loading={deploy.isPending}
							loadingText={t("bots.deploying")}
							disabled={
								!qualificationId || !profileId || !accountId || !selectedProfile
							}
							onClick={() => deploy.mutate()}
						>
							{t("bots.deploy")}
						</Button>
					</div>
				</CardContent>
			</Card>

			<section className="grid gap-4" aria-label={t("bots.listTitle")}>
				<div>
					<h2 className="text-lg font-semibold">{t("bots.listTitle")}</h2>
					<p className="text-sm text-muted-foreground">
						{t("bots.listDescription")}
					</p>
				</div>
				{bots.isPending ? <p role="status">{t("bots.loading")}</p> : null}
				{bots.isError ? <p role="alert">{t("bots.unavailable")}</p> : null}
				{bots.data?.length === 0 ? (
					<Card>
						<CardContent className="p-6 text-sm text-muted-foreground">
							{t("bots.empty")}
						</CardContent>
					</Card>
				) : null}
				{bots.data?.map((bot) => {
					const attempt = bot.attempts.find(
						(item) => item.attemptId === bot.currentAttemptId,
					);
					const busy = pendingCommand.endsWith(`:${bot.botId}`);
					return (
						<Card key={bot.botId}>
							<CardHeader className="flex flex-row items-start justify-between gap-3">
								<div>
									<CardTitle>{bot.botId}</CardTitle>
									<CardDescription>
										{t("bots.bundle")}: {bot.bundle.identity}
									</CardDescription>
								</div>
								<Badge variant={bot.state === "running" ? "default" : "outline"}>
									{t(`bots.states.${bot.state}`, { defaultValue: bot.state })}
								</Badge>
							</CardHeader>
							<CardContent className="grid gap-4 text-sm">
								<div className="grid gap-1 text-muted-foreground">
									<div>
										{t("bots.qualification")}:{" "}
										<span className="text-foreground">{bot.bundle.qualificationId}</span>
									</div>
									<div>
										{t("bots.candidate")}:{" "}
										<span className="text-foreground">
											{bot.bundle.candidateId} r{bot.bundle.candidateRevision}
										</span>
									</div>
									<div>
										{t("bots.account")}:{" "}
										<span className="text-foreground">{bot.bundle.accountId}</span>
									</div>
									<div>
										{t("bots.attempt")}:{" "}
										<span className="text-foreground">
											{attempt?.attemptId ?? t("bots.none")}
										</span>
									</div>
								</div>
								{attempt?.reconciliationRequired ? (
									<p
										role="alert"
										className="rounded-md border border-amber-500/50 bg-amber-500/5 p-2"
									>
										{t("bots.reconciliationRequired")}
									</p>
								) : null}
								{attempt?.unmanagedPositions.length ? (
									<p className="rounded-md border border-amber-500/50 bg-amber-500/5 p-2">
										{t("bots.unmanaged")} · {attempt.unmanagedPositions.join(", ")}
									</p>
								) : null}
								{attempt?.evidence.slice(-3).map((item) => (
									<p
										key={`${item.code}-${item.observedAtMs}`}
										className="text-xs text-muted-foreground"
									>
										{item.code}: {item.detail}
									</p>
								))}
								<details className="rounded-md border p-3">
									<summary className="cursor-pointer font-medium">
										{t("bots.audit")}
									</summary>
									<div className="mt-3 grid gap-3 text-xs">
										<div>
											<p className="font-medium">{t("bots.lifecycleEvents")}</p>
											{attempt?.events.slice(-8).map((event) => (
												<p
													key={`${event.from}-${event.to}-${event.actor}-${event.reason}`}
													className="text-muted-foreground"
												>
													{event.from} → {event.to} · {event.actor} · {event.reason}
												</p>
											))}
										</div>
										<div>
											<p className="font-medium">{t("bots.decisions")}</p>
											{attempt?.decisions.slice(-8).map((decision) => (
												<p key={decision.decisionId} className="text-muted-foreground">
													{decision.outcome} · {decision.decisionId}
													{decision.targetHash
														? ` · ${decision.targetHash.slice(0, 12)}`
														: ""}
												</p>
											))}
										</div>
										<div>
											<p className="font-medium">{t("bots.orders")}</p>
											{attempt?.orders.slice(-8).map((order) => (
												<p
													key={`${order.operationId}-${order.observedAtMs}`}
													className="text-muted-foreground"
												>
													{order.status} · {order.operationId}
													{order.providerOrderId ? ` · ${order.providerOrderId}` : ""}
												</p>
											))}
										</div>
										<div>
											<p className="font-medium">{t("bots.evidence")}</p>
											{attempt?.evidence.slice(-8).map((item) => (
												<p
													key={`${item.code}-${item.observedAtMs}`}
													className="text-muted-foreground"
												>
													{item.kind}/{item.code} · {item.detail}
													{item.relatedId ? ` · ${item.relatedId}` : ""}
												</p>
											))}
										</div>
									</div>
								</details>
								<div className="flex flex-wrap gap-2">
									{bot.control.canStart ? (
										<Button
											size="sm"
											disabled={busy}
											onClick={() => void control(bot.botId, "bot_start")}
										>
											{t("bots.start")}
										</Button>
									) : null}
									{bot.control.canRetry ? (
										<Button
											size="sm"
											disabled={busy}
											onClick={() => void control(bot.botId, "bot_retry")}
										>
											{t("bots.retry")}
										</Button>
									) : null}
									{bot.control.canPause ? (
										<Button
											size="sm"
											variant="outline"
											disabled={busy}
											onClick={() => void control(bot.botId, "bot_pause")}
										>
											{t("bots.pause")}
										</Button>
									) : null}
									{bot.control.canResume ? (
										<Button
											size="sm"
											disabled={busy}
											onClick={() => void control(bot.botId, "bot_resume")}
										>
											{t("bots.resume")}
										</Button>
									) : null}
									{bot.control.canStop ? (
										<Button
											size="sm"
											variant="outline"
											disabled={busy}
											onClick={() => void stop(bot.botId, "keep-position")}
										>
											{t("bots.stopKeep")}
										</Button>
									) : null}
									{bot.control.canFlatten ? (
										<Button
											size="sm"
											variant="destructive"
											disabled={busy}
											onClick={() => {
												setFlattenBotId(bot.botId);
												flattenDialog.current?.showModal();
											}}
										>
											{t("bots.stopFlatten")}
										</Button>
									) : null}
								</div>
							</CardContent>
						</Card>
					);
				})}
			</section>

			<dialog
				ref={flattenDialog}
				className="m-auto w-[min(32rem,calc(100%-2rem))] rounded-xl border bg-background p-0 text-foreground shadow-2xl backdrop:bg-black/45"
			>
				<div className="grid gap-4 p-6">
					<div>
						<h2 className="text-lg font-semibold">{t("bots.flattenConfirmTitle")}</h2>
						<p className="mt-1 text-sm text-muted-foreground">
							{t("bots.flattenConfirmDescription")}
						</p>
					</div>
					<div className="flex justify-end gap-2">
						<Button
							variant="outline"
							disabled={Boolean(pendingCommand)}
							onClick={() => flattenDialog.current?.close()}
						>
							{t("bots.cancel")}
						</Button>
						<Button
							variant="destructive"
							loading={pendingCommand === `stop:${flattenBotId}`}
							loadingText={t("bots.flattening")}
							onClick={() => void stop(flattenBotId, "flatten")}
						>
							{t("bots.confirmFlatten")}
						</Button>
					</div>
				</div>
			</dialog>
		</div>
	);
}
