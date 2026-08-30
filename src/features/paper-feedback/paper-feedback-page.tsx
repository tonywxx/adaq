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
import { formatDateTime } from "@/lib/i18n";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "@tanstack/react-router";

const lenses = ["factor", "model", "strategy", "execution"] as const;
const actions = [
	"noChange",
	"pauseBot",
	"newFactorEvaluation",
	"newModelTraining",
	"newStrategyBacktest",
	"investigateOperations",
] as const;

type Lens = (typeof lenses)[number];
type Action = (typeof actions)[number];
type EvidenceState =
	| "notYetRealized"
	| "insufficientEvidence"
	| "ready"
	| "unknown"
	| "missing"
	| "incompatible"
	| "failed";

type BotView = {
	botId: string;
	currentAttemptId?: string | null;
	bundle: { identity: string; accountId: string };
	attempts: Array<{
		attemptId: string;
		state: string;
		createdAtMs: number;
		updatedAtMs: number;
	}>;
};

type Snapshot = {
	snapshotId: string;
	input: {
		bundleId: string;
		botId: string;
		attemptId: string;
		observationStartMs: number;
		observationEndMs: number;
		realizationCutoffMs: number;
		realizedObservations: number;
		requiredObservations: number;
	};
	evidenceState: EvidenceState;
	createdAtMs: number;
};

type Report = {
	reportId: string;
	input: {
		snapshotId: string;
		lens: Lens;
		metrics: Record<string, unknown>;
		comparableEvidenceId?: string | null;
	};
	evidenceState: EvidenceState;
	createdAtMs: number;
};

type ReviewDecision = {
	decisionId: string;
	input: {
		reportIds: string[];
		action: Action;
		rationale: string;
		decidedAtMs: number;
	};
};

type WorkspaceView = {
	snapshots: Snapshot[];
	reports: Report[];
	decisions: ReviewDecision[];
};

export function PaperFeedbackPage() {
	const { t } = useTranslation();
	const userId = useAuthenticatedUserId();
	const queryClient = useQueryClient();
	const [botId, setBotId] = useState("");
	const [attemptId, setAttemptId] = useState("");
	const [observationStart, setObservationStart] = useState("");
	const [observationEnd, setObservationEnd] = useState("");
	const [realizationCutoff, setRealizationCutoff] = useState("");
	const [requiredObservations, setRequiredObservations] = useState("20");
	const [feedback, setFeedback] = useState("");
	const [selectedReports, setSelectedReports] = useState<string[]>([]);
	const [action, setAction] = useState<Action>("noChange");
	const [rationale, setRationale] = useState("");

	const bots = useQuery({
		queryKey: ["paper-feedback-bots", userId],
		queryFn: () => invoke<BotView[]>("bot_list"),
		retry: false,
	});
	const workspace = useQuery({
		queryKey: ["paper-feedback", userId],
		queryFn: () => invoke<WorkspaceView>("paper_feedback_view"),
		retry: false,
	});

	const eligibleBots = useMemo(
		() =>
			(bots.data ?? []).filter(
				(bot) =>
					Boolean(bot.currentAttemptId) &&
					bot.attempts.some((attempt) => attempt.attemptId === bot.currentAttemptId),
			),
		[bots.data],
	);
	const selectedBot = eligibleBots.find((bot) => bot.botId === botId);
	const selectedAttempt = selectedBot?.attempts.find(
		(attempt) =>
			attempt.attemptId === (attemptId || selectedBot.currentAttemptId),
	);

	const refresh = async () => {
		await Promise.all([
			queryClient.invalidateQueries({ queryKey: ["paper-feedback", userId] }),
			queryClient.invalidateQueries({ queryKey: ["paper-feedback-bots", userId] }),
		]);
	};
	const snapshot = useMutation({
		mutationFn: () => {
			const start = parseDateInput(observationStart);
			const end = parseDateInput(observationEnd);
			const cutoff = parseDateInput(realizationCutoff);
			const required = Number(requiredObservations);
			if (
				!selectedBot ||
				!selectedAttempt ||
				start === undefined ||
				end === undefined ||
				cutoff === undefined ||
				!Number.isSafeInteger(required) ||
				required < 1
			) {
				throw new Error(
					"Select an eligible Bot, bounded dates, and a positive sample requirement.",
				);
			}
			return invoke<Snapshot>("paper_feedback_snapshot_create", {
				request: {
					botId: selectedBot.botId,
					bundleId: selectedBot.bundle.identity,
					attemptId: selectedAttempt.attemptId,
					observationStartMs: start,
					observationEndMs: end,
					realizationCutoffMs: cutoff,
					requiredObservations: required,
				},
			});
		},
		onSuccess: async () => {
			setFeedback(t("paperFeedback.snapshotCreated"));
			await refresh();
		},
		onError: (error) => setFeedback(String(error)),
	});

	const report = useMutation({
		mutationFn: (input: { snapshotId: string; lens: Lens }) =>
			invoke<Report>("paper_feedback_report_create", {
				request: { snapshotId: input.snapshotId, lens: input.lens },
			}),
		onSuccess: async () => {
			setFeedback("");
			await refresh();
		},
		onError: (error) => setFeedback(String(error)),
	});

	const decision = useMutation({
		mutationFn: () =>
			invoke<ReviewDecision>("paper_feedback_review_decide", {
				request: { reportIds: selectedReports, action, rationale },
			}),
		onSuccess: async () => {
			setFeedback(t("paperFeedback.decisionRecorded"));
			setSelectedReports([]);
			setRationale("");
			await refresh();
		},
		onError: (error) => setFeedback(String(error)),
	});

	function selectBot(nextBotId: string) {
		const nextBot = eligibleBots.find((bot) => bot.botId === nextBotId);
		const nextAttempt = nextBot?.attempts.find(
			(attempt) => attempt.attemptId === nextBot.currentAttemptId,
		);
		setBotId(nextBotId);
		setAttemptId(nextAttempt?.attemptId ?? "");
		if (nextAttempt) {
			const end = Math.min(nextAttempt.updatedAtMs, Date.now());
			setObservationStart(toDateInput(nextAttempt.createdAtMs));
			setObservationEnd(toDateInput(end));
			setRealizationCutoff(toDateInput(end));
		}
	}

	const data = workspace.data;
	const reports = data?.reports ?? [];
	const snapshots = data?.snapshots ?? [];
	const selectedReportSet = new Set(selectedReports);

	return (
		<div className="grid gap-4 p-4 md:p-6">
			<header>
				<p className="text-sm text-muted-foreground">
					{t("paperFeedback.eyebrow")}
				</p>
				<h1 className="text-2xl font-semibold tracking-tight">
					{t("paperFeedback.title")}
				</h1>
				<p className="text-sm text-muted-foreground">
					{t("paperFeedback.description")}
				</p>
			</header>

			<p className="rounded-lg border bg-muted/30 p-3 text-sm text-muted-foreground">
				{t("paperFeedback.hostBoundary")}
			</p>
			{feedback ? (
				<p
					role="alert"
					className="rounded-lg border border-destructive/50 p-3 text-sm text-destructive"
				>
					{feedback}
				</p>
			) : null}
			{bots.isPending || workspace.isPending ? (
				<p role="status" className="text-sm text-muted-foreground">
					{t("paperFeedback.loading")}
				</p>
			) : null}
			{bots.isError || workspace.isError ? (
				<p role="alert" className="text-sm text-destructive">
					{t("paperFeedback.unavailable")}
				</p>
			) : null}

			<Card>
				<CardHeader>
					<CardTitle>{t("paperFeedback.createSnapshot")}</CardTitle>
					<CardDescription>{t("paperFeedback.hostBoundary")}</CardDescription>
				</CardHeader>
				<CardContent className="grid gap-4 md:grid-cols-2">
					<div className="grid gap-2">
						<Label htmlFor="feedback-bot">{t("paperFeedback.selectBot")}</Label>
						<select
							id="feedback-bot"
							value={botId}
							onChange={(event) => selectBot(event.target.value)}
							className="h-9 rounded-md border bg-background px-3 text-sm"
						>
							<option value="">{t("paperFeedback.selectBot")}</option>
							{eligibleBots.map((bot) => (
								<option key={bot.botId} value={bot.botId}>
									{bot.botId} · {bot.bundle.identity}
								</option>
							))}
						</select>
						{!eligibleBots.length && !bots.isPending ? (
							<p className="text-xs text-muted-foreground">
								{t("paperFeedback.noBots")}{" "}
								<Link className="underline" to="/bots">
									{t("nav.bots")}
								</Link>
							</p>
						) : null}
					</div>
					<div className="grid gap-2">
						<Label htmlFor="feedback-attempt">
							{t("paperFeedback.selectAttempt")}
						</Label>
						<select
							id="feedback-attempt"
							value={attemptId}
							onChange={(event) => setAttemptId(event.target.value)}
							className="h-9 rounded-md border bg-background px-3 text-sm"
						>
							<option value="">{t("paperFeedback.selectAttempt")}</option>
							{selectedBot?.attempts
								.filter((attempt) => attempt.attemptId === selectedBot.currentAttemptId)
								.map((attempt) => (
									<option key={attempt.attemptId} value={attempt.attemptId}>
										{attempt.attemptId} · {attempt.state}
									</option>
								))}
						</select>
					</div>
					<DateField
						id="feedback-start"
						label={t("paperFeedback.observationStart")}
						value={observationStart}
						onChange={setObservationStart}
					/>
					<DateField
						id="feedback-end"
						label={t("paperFeedback.observationEnd")}
						value={observationEnd}
						onChange={setObservationEnd}
					/>
					<DateField
						id="feedback-cutoff"
						label={t("paperFeedback.realizationCutoff")}
						value={realizationCutoff}
						onChange={setRealizationCutoff}
					/>
					<div className="grid gap-2">
						<Label htmlFor="feedback-required">
							{t("paperFeedback.requiredObservations")}
						</Label>
						<input
							id="feedback-required"
							type="number"
							min="1"
							max="1000000"
							value={requiredObservations}
							onChange={(event) => setRequiredObservations(event.target.value)}
							className="h-9 rounded-md border bg-background px-3 text-sm"
						/>
					</div>
					<div className="flex items-end">
						<Button
							loading={snapshot.isPending}
							loadingText={t("paperFeedback.creatingSnapshot")}
							disabled={!selectedBot || !selectedAttempt}
							onClick={() => snapshot.mutate()}
						>
							{t("paperFeedback.createSnapshot")}
						</Button>
					</div>
				</CardContent>
			</Card>

			<section className="grid gap-4" aria-labelledby="feedback-snapshots-title">
				<div>
					<h2 id="feedback-snapshots-title" className="text-lg font-semibold">
						{t("paperFeedback.snapshots")}
					</h2>
				</div>
				{snapshots.length === 0 ? (
					<Card>
						<CardContent className="p-6 text-sm text-muted-foreground">
							{t("paperFeedback.noSnapshots")}
						</CardContent>
					</Card>
				) : (
					snapshots.map((item) => (
						<Card key={item.snapshotId}>
							<CardHeader className="flex flex-row items-start justify-between gap-3">
								<div>
									<CardTitle>{item.input.botId}</CardTitle>
									<CardDescription>
										{item.snapshotId} · {item.input.bundleId}
									</CardDescription>
								</div>
								<StateBadge state={item.evidenceState} t={t} />
							</CardHeader>
							<CardContent className="grid gap-2 text-sm">
								<div className="grid gap-1 text-muted-foreground sm:grid-cols-2">
									<span>
										{t("paperFeedback.attempt")}: {item.input.attemptId}
									</span>
									<span>
										{t("paperFeedback.created")}: {formatDateTime(item.createdAtMs)}
									</span>
									<span>
										{t("paperFeedback.observationStart")}:{" "}
										{formatDateTime(item.input.observationStartMs)}
									</span>
									<span>
										{t("paperFeedback.observationEnd")}:{" "}
										{formatDateTime(item.input.observationEndMs)}
									</span>
								</div>
								<div className="flex flex-wrap gap-2">
									{lenses.map((lens) => {
										const existing = reports.some(
											(reportItem) =>
												reportItem.input.snapshotId === item.snapshotId &&
												reportItem.input.lens === lens,
										);
										return (
											<Button
												key={lens}
												size="sm"
												variant="outline"
												disabled={existing || report.isPending}
												onClick={() => report.mutate({ snapshotId: item.snapshotId, lens })}
											>
												{existing
													? t(`paperFeedback.lenses.${lens}`)
													: `${t("paperFeedback.generate")} · ${t(`paperFeedback.lenses.${lens}`)}`}
											</Button>
										);
									})}
								</div>
							</CardContent>
						</Card>
					))
				)}
			</section>

			<Card>
				<CardHeader>
					<CardTitle>{t("paperFeedback.reports")}</CardTitle>
					<CardDescription>{t("paperFeedback.hostBoundary")}</CardDescription>
				</CardHeader>
				<CardContent className="grid gap-2">
					{reports.length === 0 ? (
						<p className="text-sm text-muted-foreground">
							{t("paperFeedback.noReports")}
						</p>
					) : (
						reports.map((item) => (
							<label
								className="flex items-start gap-3 rounded-md border p-3"
								key={item.reportId}
							>
								<input
									type="checkbox"
									checked={selectedReportSet.has(item.reportId)}
									onChange={(event) =>
										setSelectedReports((current) =>
											event.target.checked
												? [...current, item.reportId]
												: current.filter((id) => id !== item.reportId),
										)
									}
								/>
								<span className="grid gap-1 text-sm">
									<span className="flex flex-wrap items-center gap-2 font-medium">
										{t(`paperFeedback.lenses.${item.input.lens}`)}
										<StateBadge state={item.evidenceState} t={t} />
									</span>
									<span className="text-xs text-muted-foreground">
										{item.reportId} · {item.input.snapshotId}
									</span>
									{item.input.metrics.directionalConclusion === false ? (
										<span className="text-xs text-muted-foreground">
											{t("paperFeedback.directionalUnavailable")}
										</span>
									) : null}
								</span>
							</label>
						))
					)}
				</CardContent>
			</Card>

			<Card>
				<CardHeader>
					<CardTitle>{t("paperFeedback.review")}</CardTitle>
					<CardDescription>{t("paperFeedback.hostBoundary")}</CardDescription>
				</CardHeader>
				<CardContent className="grid gap-4">
					<div className="grid gap-2">
						<Label htmlFor="feedback-action">{t("paperFeedback.action")}</Label>
						<select
							id="feedback-action"
							value={action}
							onChange={(event) => setAction(event.target.value as Action)}
							className="h-9 rounded-md border bg-background px-3 text-sm"
						>
							{actions.map((value) => (
								<option key={value} value={value}>
									{t(`paperFeedback.actions.${value}`)}
								</option>
							))}
						</select>
					</div>
					<div className="grid gap-2">
						<Label htmlFor="feedback-rationale">{t("paperFeedback.rationale")}</Label>
						<textarea
							id="feedback-rationale"
							value={rationale}
							onChange={(event) => setRationale(event.target.value)}
							placeholder={t("paperFeedback.rationalePlaceholder")}
							rows={4}
							className="rounded-md border bg-background px-3 py-2 text-sm"
						/>
					</div>
					<Button
						loading={decision.isPending}
						loadingText={t("paperFeedback.submittingDecision")}
						disabled={!selectedReports.length || !rationale.trim()}
						onClick={() => decision.mutate()}
					>
						{t("paperFeedback.submitDecision")}
					</Button>
					{data?.decisions.length ? (
						<div className="grid gap-2 text-sm">
							{data.decisions.map((item) => (
								<div className="rounded-md border p-3" key={item.decisionId}>
									<div className="flex flex-wrap justify-between gap-2">
										<span className="font-medium">
											{t(`paperFeedback.actions.${item.input.action}`)}
										</span>
										<span className="text-xs text-muted-foreground">
											{formatDateTime(item.input.decidedAtMs)}
										</span>
									</div>
									<p className="mt-1 text-muted-foreground">{item.input.rationale}</p>
								</div>
							))}
						</div>
					) : (
						<p className="text-sm text-muted-foreground">
							{t("paperFeedback.noDecisions")}
						</p>
					)}
				</CardContent>
			</Card>
		</div>
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

function StateBadge({
	state,
	t,
}: {
	state: EvidenceState;
	t: (key: string, options?: Record<string, unknown>) => string;
}) {
	return (
		<Badge variant={state === "ready" ? "default" : "outline"}>
			{t(`paperFeedback.states.${state}`, { defaultValue: state })}
		</Badge>
	);
}

function parseDateInput(value: string) {
	const parsed = Date.parse(value);
	return Number.isFinite(parsed) ? parsed : undefined;
}

function toDateInput(value: number) {
	const date = new Date(value);
	const pad = (part: number) => String(part).padStart(2, "0");
	return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`;
}
