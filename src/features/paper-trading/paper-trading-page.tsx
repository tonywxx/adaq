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
import {
	Pagination,
	PaginationContent,
	PaginationItem,
} from "@/components/ui/pagination";
import { afterPaint } from "@/features/factors/factor-workspace-data";
import { IdentifierDisplay } from "@/components/identifier-display";
import { formatDateTime, formatDecimal } from "@/lib/i18n";
import {
	keepPreviousData,
	useQuery,
	useQueryClient,
} from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

type PaperAccountView = {
	account: {
		account_id: string;
		market: "okx_spot";
		currency: string;
		cash: string;
		positions: Record<string, { quantity: string; sellable_quantity: string }>;
		observed_at_ms: number;
	};
	reservedCash: string;
	buyingPower: string;
	reconciliation: "reconciled" | "required" | "unknown";
	restartRequired: boolean;
	orders: Array<{
		order_id: string;
		instrument: string;
		side: string;
		quantity: string;
		filled_quantity: string;
		limit_price: string;
		status: string;
	}>;
	fills: Array<{
		fill_id: string;
		order_id: string;
		quantity: string;
		price: string;
		fee: string;
	}>;
	providerEvidence: Record<string, unknown>[];
	riskDecisions: Array<{
		approved: boolean;
		reason: string;
		requestedNotional: string;
		approvedNotional: string;
		decidedAtMs: number;
	}>;
};

type PaperAccountSummary = Pick<
	PaperAccountView,
	| "account"
	| "reservedCash"
	| "buyingPower"
	| "reconciliation"
	| "restartRequired"
> & { hasUncertain: boolean };
type EvidenceSection =
	| "positions"
	| "orders"
	| "fills"
	| "riskDecisions"
	| "providerEvidence";
type EvidencePage<T> = {
	items: T[];
	page: number;
	pageSize: number;
	total: number;
};

type PaperTradingWorkspaceView = {
	account: PaperAccountSummary | null;
	connection: {
		state: "connected" | "degraded" | "disconnected";
		evidence?: Record<string, unknown> | null;
	};
};

export function PaperTradingPage() {
	const { t } = useTranslation();
	const userId = useAuthenticatedUserId();
	const queryClient = useQueryClient();
	const paperAccountQueryKey = ["paper-account", userId] as const;
	const dialog = useRef<HTMLDialogElement>(null);
	const [reconciling, setReconciling] = useState(false);
	const [reconcileError, setReconcileError] = useState("");
	const account = useQuery({
		queryKey: paperAccountQueryKey,
		queryFn: async ({ signal }) => {
			await afterPaint();
			signal.throwIfAborted();
			return invoke<PaperTradingWorkspaceView>("paper_account_view");
		},
		refetchOnMount: "always",
		enabled: Boolean(userId),
		retry: false,
	});

	async function reconcile() {
		setReconciling(true);
		setReconcileError("");
		try {
			const next = await invoke<PaperAccountSummary>("paper_account_reconcile");
			queryClient.setQueryData<PaperTradingWorkspaceView>(
				paperAccountQueryKey,
				(current) =>
					current
						? { ...current, account: next, connection: { state: "connected" } }
						: current,
			);
			void queryClient.invalidateQueries({
				queryKey: ["paper-account-evidence", userId],
			});
			for (const key of [
				"operations-health",
				"operations-alerts",
				"operations-events",
				"operations-alert-history",
			]) {
				void queryClient.invalidateQueries({ queryKey: [key, userId] });
			}
			dialog.current?.close();
		} catch (reason) {
			setReconcileError(reconcileFailureMessage(t, reason));
		} finally {
			setReconciling(false);
		}
	}

	const workspace = account.data;
	const view = workspace?.account;
	const uncertain = view?.reconciliation === "unknown" || view?.hasUncertain;
	const refresh = () => {
		void account.refetch();
		void queryClient.invalidateQueries({
			queryKey: ["paper-account-evidence", userId],
		});
	};

	return (
		<div className="flex min-w-0 flex-1 flex-col gap-5 p-4 lg:p-6">
			<header className="flex flex-wrap items-start justify-between gap-3">
				<div>
					<p className="text-sm text-muted-foreground">
						{t("paperTrading.eyebrow")}
					</p>
					<h1 className="text-2xl font-semibold">{t("paperTrading.title")}</h1>
					<p className="text-sm text-muted-foreground">
						{t("paperTrading.description")}
					</p>
				</div>
				<Button
					variant="outline"
					onClick={refresh}
					disabled={account.isFetching || reconciling}
				>
					{t("paperTrading.refresh")}
				</Button>
				<Button onClick={() => dialog.current?.showModal()} disabled={reconciling}>
					{t("paperTrading.reconcile")}
				</Button>
			</header>

			{account.isPending ? (
				<p role="status" className="text-sm text-muted-foreground">
					{t("paperTrading.loading")}
				</p>
			) : null}
			{account.isError ? (
				<p
					role="alert"
					className="rounded-lg border border-destructive/50 p-3 text-sm text-destructive"
				>
					{t("paperTrading.unavailable")}
				</p>
			) : null}
			{reconcileError ? (
				<p
					role="alert"
					className="rounded-lg border border-destructive/50 p-3 text-sm text-destructive"
				>
					{reconcileError}
				</p>
			) : null}
			{workspace?.connection.state === "degraded" ? (
				<p
					role="alert"
					className="rounded-lg border border-amber-500/50 bg-amber-500/5 p-3 text-sm"
				>
					{t("paperTrading.connectionDegraded")}
				</p>
			) : null}
			{workspace?.connection.state === "disconnected" ? (
				<p
					role="alert"
					className="rounded-lg border border-amber-500/50 bg-amber-500/5 p-3 text-sm"
				>
					{t("paperTrading.connectionDisconnected")}
				</p>
			) : null}
			{view === null ? (
				<Card>
					<CardHeader>
						<CardTitle>{t("paperTrading.emptyTitle")}</CardTitle>
						<CardDescription>{t("paperTrading.emptyDescription")}</CardDescription>
					</CardHeader>
				</Card>
			) : null}
			{view ? (
				<>
					{view.reconciliation === "required" ? (
						<p
							role="alert"
							className="rounded-lg border border-amber-500/50 bg-amber-500/5 p-3 text-sm"
						>
							{t("paperTrading.reconciliationRequired")}
						</p>
					) : null}
					{view.restartRequired ? (
						<p
							role="alert"
							className="rounded-lg border border-amber-500/50 bg-amber-500/5 p-3 text-sm"
						>
							{t("paperTrading.restartRequired")}
						</p>
					) : null}
					{uncertain ? (
						<p
							role="alert"
							className="rounded-lg border border-amber-500/50 bg-amber-500/5 p-3 text-sm"
						>
							{t("paperTrading.uncertain")}
						</p>
					) : null}
					<div className="grid gap-4 lg:grid-cols-3">
						<Card>
							<CardHeader>
								<CardTitle>{t("paperTrading.account")}</CardTitle>
								<CardDescription>
									<IdentifierDisplay
										id={view.account.account_id}
										label={t("identifiers.account")}
									/>
								</CardDescription>
							</CardHeader>
							<CardContent className="grid gap-2 text-sm">
								<div className="flex justify-between gap-3">
									<span>{t("paperTrading.observed")}</span>
									<span>{formatDateTime(view.account.observed_at_ms)}</span>
								</div>
								<div className="flex justify-between gap-3">
									<span>{t("paperTrading.cash")}</span>
									<span>{formatDecimal(view.account.cash)}</span>
								</div>
								<div className="flex justify-between gap-3">
									<span>{t("paperTrading.reservedCash")}</span>
									<span>{formatDecimal(view.reservedCash)}</span>
								</div>
								<div className="flex justify-between gap-3">
									<span>{t("paperTrading.buyingPower")}</span>
									<span>{formatDecimal(view.buyingPower)}</span>
								</div>
								<div className="flex justify-between gap-3">
									<span>{t("paperTrading.provider")}</span>
									<Badge variant="outline">
										{t(`paperTrading.status.${view.reconciliation}`)}
									</Badge>
								</div>
								{workspace?.connection.evidence ? (
									<details>
										<summary>{t("paperTrading.connectionEvidence")}</summary>
										<pre className="whitespace-pre-wrap break-all text-xs">
											{JSON.stringify(workspace.connection.evidence)}
										</pre>
									</details>
								) : null}
							</CardContent>
						</Card>
						<EvidenceCard<{
							instrument: string;
							quantity: string;
							sellable_quantity: string;
						}>
							key={`${userId}:positions`}
							section="positions"
							title={t("paperTrading.positions")}
							empty={t("paperTrading.noPositions")}
							formatRow={(position) =>
								`${position.instrument}: ${formatDecimal(position.quantity)} / ${formatDecimal(position.sellable_quantity)}`
							}
						/>
						<EvidenceCard<PaperAccountView["riskDecisions"][number]>
							key={`${userId}:riskDecisions`}
							section="riskDecisions"
							title={t("paperTrading.riskDecision")}
							empty={t("paperTrading.noRiskDecision")}
							formatRow={(decision) =>
								`${decision.approved ? t("paperTrading.approved") : t("paperTrading.rejected")} · ${decision.reason} · ${formatDecimal(decision.approvedNotional)} / ${formatDecimal(decision.requestedNotional)}`
							}
						/>
					</div>
					<div className="grid gap-4 lg:grid-cols-3">
						<EvidenceCard<PaperAccountView["orders"][number]>
							key={`${userId}:orders`}
							section="orders"
							title={t("paperTrading.orders")}
							empty={t("paperTrading.noOrders")}
							formatRow={(order) =>
								`${order.order_id} · ${order.instrument} · ${t(`paperTrading.orderSide.${order.side.toLowerCase()}`, { defaultValue: order.side })} · ${formatDecimal(order.filled_quantity)} / ${formatDecimal(order.quantity)} · ${t(`paperTrading.orderStatus.${order.status}`, { defaultValue: order.status })}`
							}
						/>
						<EvidenceCard<PaperAccountView["fills"][number]>
							key={`${userId}:fills`}
							section="fills"
							title={t("paperTrading.fills")}
							empty={t("paperTrading.noFills")}
							formatRow={(fill) =>
								`${fill.order_id} · ${formatDecimal(fill.quantity)} @ ${formatDecimal(fill.price)} · ${t("paperTrading.fee")} ${formatDecimal(fill.fee)}`
							}
						/>
						<EvidenceCard<Record<string, unknown>>
							key={`${userId}:providerEvidence`}
							section="providerEvidence"
							title={t("paperTrading.providerEvidence")}
							empty={t("paperTrading.noProviderEvidence")}
							formatRow={(evidence) => JSON.stringify(evidence)}
						/>
					</div>
				</>
			) : null}

			<dialog
				ref={dialog}
				onCancel={(event) => reconciling && event.preventDefault()}
				className="m-auto w-[min(32rem,calc(100%-2rem))] rounded-xl border bg-background p-0 text-foreground shadow-2xl backdrop:bg-black/45"
			>
				<div className="grid gap-4 p-6">
					<div>
						<h2 className="text-lg font-semibold">
							{t("paperTrading.confirmTitle")}
						</h2>
						<p className="mt-1 text-sm text-muted-foreground">
							{t("paperTrading.confirmDescription")}
						</p>
					</div>
					<div className="flex justify-end gap-2">
						<Button
							variant="outline"
							disabled={reconciling}
							onClick={() => dialog.current?.close()}
						>
							{t("paperTrading.cancel")}
						</Button>
						<Button
							loading={reconciling}
							loadingText={t("paperTrading.reconciling")}
							onClick={() => void reconcile()}
						>
							{t("paperTrading.confirm")}
						</Button>
					</div>
				</div>
			</dialog>
		</div>
	);
}

function reconcileFailureMessage(t: (key: string) => string, reason: unknown) {
	if (typeof reason === "string") {
		try {
			const error = JSON.parse(reason) as { code?: string; message?: string };
			if (error.code) {
				const message = error.message?.trim();
				return `${t("paperTrading.reconcileFailed")} ${t(`paperTrading.errors.${error.code}`)}${message ? ` ${message}` : ""}`;
			}
		} catch {
			// The Host always sends a typed JSON error; retain a safe fallback for old Hosts.
		}
	}
	return t("paperTrading.reconcileFailed");
}

function EvidenceCard<T>({
	title,
	empty,
	section,
	formatRow,
}: {
	title: string;
	empty: string;
	section: EvidenceSection;
	formatRow: (row: T) => string;
}) {
	const { t } = useTranslation();
	const userId = useAuthenticatedUserId();
	const [page, setPage] = useState(1);
	const queryClient = useQueryClient();
	const retainedPage = useRef<EvidencePage<T> | undefined>(undefined);
	const evidence = useQuery({
		queryKey: ["paper-account-evidence", userId, section, page],
		queryFn: async ({ signal }) => {
			await afterPaint();
			signal.throwIfAborted();
			return invoke<EvidencePage<T>>("paper_account_evidence_page", {
				section,
				page,
			});
		},
		enabled: Boolean(userId),
		placeholderData: keepPreviousData,
		refetchOnMount: "always",
		retry: false,
	});
	useEffect(() => {
		if (evidence.data && !evidence.isPlaceholderData) {
			retainedPage.current = evidence.data;
		}
		if (
			evidence.data &&
			!evidence.isPlaceholderData &&
			evidence.data.page !== page
		) {
			queryClient.setQueryData(
				["paper-account-evidence", userId, section, evidence.data.page],
				evidence.data,
			);
			setPage(evidence.data.page);
		}
	}, [
		evidence.data,
		evidence.isPlaceholderData,
		page,
		queryClient,
		userId,
		section,
	]);
	const data = evidence.data ?? retainedPage.current;
	const current = data?.page ?? page;
	const totalPages = Math.max(
		1,
		Math.ceil((data?.total ?? 0) / (data?.pageSize ?? 10)),
	);
	const changePage = (next: number) => {
		if (next === page) void evidence.refetch();
		else setPage(next);
	};
	return (
		<Card data-evidence-section={section}>
			<CardHeader>
				<CardTitle>{title}</CardTitle>
			</CardHeader>
			<CardContent aria-busy={evidence.isFetching} className="space-y-3">
				{evidence.isFetching ? (
					<p role="status" className="text-sm text-muted-foreground">
						{t("paperTrading.loading")}
					</p>
				) : null}
				{evidence.isError ? (
					<p role="alert" className="text-sm text-destructive">
						{t("paperTrading.unavailable")}
					</p>
				) : null}
				{data?.items.length ? (
					<ul className="grid gap-2 text-sm">
						{data.items.map((item, index) => (
							<li className="rounded-md border p-2" key={`${current}:${index}`}>
								{formatRow(item)}
							</li>
						))}
					</ul>
				) : !evidence.isPending && !evidence.isError ? (
					<p className="text-sm text-muted-foreground">{empty}</p>
				) : null}
				{data ? (
					<Pagination
						aria-label={t("paperTrading.paginationLabel", { section: title })}
					>
						<PaginationContent>
							<PaginationItem>
								<Button
									variant="ghost"
									disabled={evidence.isFetching || current <= 1}
									onClick={() => changePage(current - 1)}
								>
									{t("paperTrading.previousPage")}
								</Button>
							</PaginationItem>
							<PaginationItem>
								<span role="status" className="px-2 text-sm">
									{t("paperTrading.pageOf", { current, total: totalPages })}
								</span>
							</PaginationItem>
							<PaginationItem>
								<Button
									variant="ghost"
									disabled={evidence.isFetching || current >= totalPages}
									onClick={() => changePage(current + 1)}
								>
									{t("paperTrading.nextPage")}
								</Button>
							</PaginationItem>
						</PaginationContent>
					</Pagination>
				) : null}
			</CardContent>
		</Card>
	);
}
