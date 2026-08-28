import { ChevronDownIcon, RefreshCwIcon } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { formatNumber } from "@/lib/i18n";
import {
	FACTOR_PAGE_SIZE,
	factorPageCount,
	factorString,
	formatFactorJson,
	formatFactorError,
	parseFactorJson,
} from "./factor-data";
import type { FactorAttemptStatus, FactorJson } from "./factor-types";

const FACTOR_STATUS_GLYPH: Record<FactorAttemptStatus, string> = {
	pending: "◷",
	running: "▸",
	completed: "✓",
	failed: "✕",
	cancelled: "⊘",
	interrupted: "⚠",
	stale: "⌁",
};

export function FactorAttemptStatusBadge({
	status,
}: {
	status: FactorAttemptStatus;
}) {
	const { t } = useTranslation();
	const variant =
		status === "completed"
			? "default"
			: status === "failed" ||
					status === "cancelled" ||
					status === "interrupted" ||
					status === "stale"
				? "secondary"
				: "outline";
	return (
		<Badge variant={variant} className="gap-1 font-normal">
			<span aria-hidden="true">{FACTOR_STATUS_GLYPH[status]}</span>
			{t(`factors.status.${status}`)}
		</Badge>
	);
}

export function newUuid() {
	if (globalThis.crypto?.randomUUID) return globalThis.crypto.randomUUID();
	const bytes = new Uint8Array(16);
	if (globalThis.crypto?.getRandomValues) {
		globalThis.crypto.getRandomValues(bytes);
	} else {
		for (let index = 0; index < bytes.length; index += 1) {
			bytes[index] = Math.floor(Math.random() * 256);
		}
	}
	bytes[6] = (bytes[6] & 0x0f) | 0x40;
	bytes[8] = (bytes[8] & 0x3f) | 0x80;
	const hex = Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0"));
	return (
		hex.slice(0, 4).join("") +
		"-" +
		hex.slice(4, 6).join("") +
		"-" +
		hex.slice(6, 8).join("") +
		"-" +
		hex.slice(8, 10).join("") +
		"-" +
		hex.slice(10).join("")
	);
}

export function valueAt(value: unknown, path: string): unknown {
	return path.split(".").reduce((current: unknown, part) => {
		if (!current || typeof current !== "object" || Array.isArray(current))
			return undefined;
		return (current as FactorJson)[part];
	}, value);
}

export function textAt(value: unknown, path: string, fallback = "—") {
	return factorString(valueAt(value, path), fallback);
}

export function localizedFactorCode(
	code: string,
	t: (key: string, options?: Record<string, unknown>) => string,
) {
	return t(`factors.codes.${code}`, { defaultValue: code });
}

export function localizedFactorAttemptCode(
	code: string,
	t: (key: string, options?: Record<string, unknown>) => string,
) {
	return code.startsWith("factor-context-")
		? t(`researchContext.reasons.${code}`, { defaultValue: code })
		: localizedFactorCode(code, t);
}

export function localizedFactorError(
	error: unknown,
	t: (key: string, options?: Record<string, unknown>) => string,
) {
	const raw = formatFactorError(error);
	const diagnostic = raw.replace(/^Error:\s*/, "");
	const prefix = diagnostic.split(":")[0];
	if (prefix.startsWith("factor-context-")) {
		return t(`researchContext.reasons.${prefix}`, { defaultValue: raw });
	}
	if (
		diagnostic.startsWith("Feature Slot ") ||
		diagnostic.startsWith("Feature 槽位")
	) {
		return diagnostic;
	}
	if (
		prefix === "cancelled" ||
		prefix === "factor-component-build-failed" ||
		prefix === "factor-component-qualification-failed" ||
		prefix === "research-interrupted" ||
		prefix === "reset-required"
	) {
		return localizedFactorCode(prefix, t);
	}

	// ponytail: map legacy string diagnostics until every invoke boundary returns typed errors.
	const lower = diagnostic.toLowerCase();
	const code =
		lower.includes("hash mismatch") ||
		lower.includes("collision") ||
		lower.includes("integrity") ||
		lower.includes("corrupt")
			? "factor-corruption-detected"
			: lower.includes("cannot be published") ||
					lower.includes("publication") ||
					lower.includes("staging")
				? "factor-publication-failed"
				: lower.includes("not found") ||
						lower.includes("not available") ||
						lower.includes("not configured") ||
						lower.includes("missing") ||
						lower.includes("unavailable")
					? "factor-missing-input"
					: lower.includes("resource") ||
							lower.includes("too large") ||
							lower.includes("timed out") ||
							lower.includes("timeout") ||
							lower.includes("memory") ||
							lower.includes("thread") ||
							lower.includes("limit")
						? "factor-resource-failed"
						: lower.includes("does not match") ||
								lower.includes("differs from") ||
								lower.includes("not bound") ||
								lower.includes("incompatible") ||
								lower.includes("not present") ||
								lower.includes("requires")
							? "factor-compatibility-failed"
							: lower.includes("invalid") ||
									lower.includes("validate") ||
									lower.includes("must be") ||
									lower.includes("empty")
								? "factor-validation-failed"
								: undefined;
	return localizedFactorCode(code ?? "factor-research-failed", t);
}

export function localizedFactorReason(
	reason: string,
	t: (key: string, options?: Record<string, unknown>) => string,
) {
	if (
		reason === "completed output lacks a current frozen promotion evidence set"
	) {
		return t("factors.decisions.currentEvidenceMissing");
	}
	return localizedFactorCode(reason, t);
}

export function jsonText(value: unknown) {
	return formatFactorJson(value) ?? "null";
}

export function lines(value: string) {
	return value
		.split(/\r?\n/)
		.map((item) => item.trim())
		.filter(Boolean);
}

export function commaSeparated(value: string) {
	const items = value
		.split(",")
		.map((item) => item.trim())
		.filter(Boolean);
	return items.length > 0 ? items : undefined;
}

export function commaSeparatedNumbers(value: string) {
	const items = commaSeparated(value);
	if (!items) return undefined;
	const numbers = items.map(Number);
	return numbers.every(Number.isFinite) ? numbers : undefined;
}

export function optionalNumber(value: string) {
	if (!value.trim()) return undefined;
	const number = Number(value);
	return Number.isFinite(number) ? number : undefined;
}

export function mergeFactorFields(
	raw: string,
	label: string,
	fields: Record<string, unknown>,
) {
	const draft = { ...parseFactorJson(raw, label) };
	for (const [key, value] of Object.entries(fields)) {
		if (value !== undefined && value !== "") draft[key] = value;
	}
	return draft;
}

export function Feedback({
	message,
	tone = "error",
}: {
	message?: string;
	tone?: "error" | "success";
}) {
	if (!message) return null;
	return (
		<p
			role={tone === "error" ? "alert" : "status"}
			aria-live="polite"
			className={
				tone === "error" ? "text-sm text-destructive" : "text-sm text-emerald-600"
			}
		>
			{message}
		</p>
	);
}

export function LoadingState({ label }: { label: string }) {
	return (
		<div
			className="py-8 text-center text-sm text-muted-foreground"
			aria-busy="true"
			role="status"
		>
			{label}
		</div>
	);
}

export function EmptyState({ message }: { message: string }) {
	return (
		<p className="py-8 text-center text-sm text-muted-foreground">{message}</p>
	);
}

export function ErrorState({
	message,
	onRetry,
	retryLabel,
	loading = false,
}: {
	message: string;
	onRetry: () => void;
	retryLabel: string;
	loading?: boolean;
}) {
	return (
		<div
			className="space-y-3 rounded-lg border border-destructive/30 bg-destructive/5 p-4"
			role="alert"
		>
			<p className="text-sm text-destructive">{message}</p>
			<Button
				type="button"
				variant="outline"
				size="sm"
				loading={loading}
				onClick={onRetry}
			>
				<RefreshCwIcon aria-hidden="true" />
				{retryLabel}
			</Button>
		</div>
	);
}

export function EvidenceJson({
	label,
	value,
}: {
	label: string;
	value: unknown;
}) {
	return (
		<details className="rounded-md border bg-muted/20 p-3">
			<summary className="flex cursor-pointer list-none items-center gap-2 text-sm font-medium [&::-webkit-details-marker]:hidden">
				<ChevronDownIcon
					className="size-4 transition-transform details-open:rotate-180"
					aria-hidden="true"
				/>
				{label}
			</summary>
			<pre className="mt-3 max-h-96 overflow-auto whitespace-pre-wrap break-words font-mono text-xs text-muted-foreground">
				{jsonText(value)}
			</pre>
		</details>
	);
}

export function PageControls({
	page,
	total,
	pageSize,
	onPage,
}: {
	page: number;
	total: number;
	pageSize: number;
	onPage: (page: number) => void;
}) {
	const { t } = useTranslation();
	const pages = factorPageCount(total, pageSize || FACTOR_PAGE_SIZE);
	return (
		<div className="flex flex-wrap items-center justify-between gap-2 border-t pt-3 text-xs text-muted-foreground">
			<span>
				{page} / {pages} · {formatNumber(total)}
			</span>
			<div className="flex gap-2">
				<Button
					type="button"
					size="sm"
					variant="outline"
					disabled={page <= 1}
					onClick={() => onPage(page - 1)}
				>
					{t("factors.pagination.previous")}
				</Button>
				<Button
					type="button"
					size="sm"
					variant="outline"
					disabled={page >= pages}
					onClick={() => onPage(page + 1)}
				>
					{t("factors.pagination.next")}
				</Button>
			</div>
		</div>
	);
}
