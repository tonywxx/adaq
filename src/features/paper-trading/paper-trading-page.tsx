import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "@/components/ui/card";
import { formatDateTime, formatDecimal } from "@/lib/i18n";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { useRef, useState } from "react";
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
};

const paperAccountQueryKey = ["paper-account"] as const;

export function PaperTradingPage() {
	const { t } = useTranslation();
	const queryClient = useQueryClient();
	const dialog = useRef<HTMLDialogElement>(null);
	const [reconciling, setReconciling] = useState(false);
	const [reconcileError, setReconcileError] = useState("");
	const account = useQuery({
		queryKey: paperAccountQueryKey,
		queryFn: () => invoke<PaperAccountView | null>("paper_account_view"),
		retry: false,
	});

	async function reconcile() {
		setReconciling(true);
		setReconcileError("");
		try {
			const next = await invoke<PaperAccountView>("paper_account_reconcile");
			queryClient.setQueryData(paperAccountQueryKey, next);
			dialog.current?.close();
		} catch (reason) {
			setReconcileError(`${t("paperTrading.reconcileFailed")} ${String(reason)}`);
		} finally {
			setReconciling(false);
		}
	}

	const view = account.data;
	const uncertain = view?.providerEvidence.some(
		(evidence) => "Uncertain" in evidence,
	);

	return (
		<div className="grid gap-4 p-4 md:p-6">
			<header className="flex flex-wrap items-start justify-between gap-3">
				<div>
					<p className="text-sm text-muted-foreground">
						{t("paperTrading.eyebrow")}
					</p>
					<h1 className="text-2xl font-semibold tracking-tight">
						{t("paperTrading.title")}
					</h1>
					<p className="text-sm text-muted-foreground">
						{t("paperTrading.description")}
					</p>
				</div>
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
								<CardDescription>{view.account.account_id}</CardDescription>
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
									<span>OKX Demo</span>
									<Badge variant="outline">
										{t(`paperTrading.status.${view.reconciliation}`)}
									</Badge>
								</div>
							</CardContent>
						</Card>
						<EvidenceCard
							title={t("paperTrading.positions")}
							empty={t("paperTrading.noPositions")}
							rows={Object.entries(view.account.positions).map(
								([instrument, position]) =>
									`${instrument}: ${formatDecimal(position.quantity)} / ${formatDecimal(position.sellable_quantity)}`,
							)}
						/>
						<EvidenceCard
							title={t("paperTrading.riskDecision")}
							empty={t("paperTrading.noRiskDecision")}
							rows={[]}
						/>
					</div>
					<div className="grid gap-4 lg:grid-cols-3">
						<EvidenceCard
							title={t("paperTrading.orders")}
							empty={t("paperTrading.noOrders")}
							rows={view.orders.map(
								(order) =>
									`${order.instrument} · ${order.side} · ${formatDecimal(order.filled_quantity)} / ${formatDecimal(order.quantity)} · ${order.status}`,
							)}
						/>
						<EvidenceCard
							title={t("paperTrading.fills")}
							empty={t("paperTrading.noFills")}
							rows={view.fills.map(
								(fill) =>
									`${fill.order_id} · ${formatDecimal(fill.quantity)} @ ${formatDecimal(fill.price)} · fee ${formatDecimal(fill.fee)}`,
							)}
						/>
						<EvidenceCard
							title={t("paperTrading.providerEvidence")}
							empty={t("paperTrading.noProviderEvidence")}
							rows={view.providerEvidence.map((evidence) => JSON.stringify(evidence))}
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

function EvidenceCard({
	title,
	empty,
	rows,
}: {
	title: string;
	empty: string;
	rows: string[];
}) {
	return (
		<Card>
			<CardHeader>
				<CardTitle>{title}</CardTitle>
			</CardHeader>
			<CardContent>
				{rows.length ? (
					<ul className="grid gap-2 text-sm">
						{rows.map((row) => (
							<li className="rounded-md border p-2" key={row}>
								{row}
							</li>
						))}
					</ul>
				) : (
					<p className="text-sm text-muted-foreground">{empty}</p>
				)}
			</CardContent>
		</Card>
	);
}
