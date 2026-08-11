import { Badge } from "@/components/ui/badge";
import { useTranslation } from "react-i18next";
import type { FeatureAttemptStatus } from "./features-types";

// Attempt states pair text with a shape so state never relies on color alone.
const STATUS_GLYPH: Record<FeatureAttemptStatus, string> = {
	pending: "◷",
	running: "▸",
	completed: "✓",
	failed: "✕",
	cancelled: "⊘",
};

export function AttemptStatusBadge({
	status,
}: {
	status: FeatureAttemptStatus;
}) {
	const { t } = useTranslation();
	const variant =
		status === "completed"
			? "default"
			: status === "failed" || status === "cancelled"
				? "secondary"
				: "outline";
	return (
		<Badge variant={variant} className="gap-1 font-normal">
			<span aria-hidden="true">{STATUS_GLYPH[status]}</span>
			{t(`features.status.${status}`)}
		</Badge>
	);
}

export function FeaturesLoading({ label }: { label: string }) {
	return (
		<p
			aria-busy="true"
			className="py-8 text-center text-sm text-muted-foreground"
		>
			{label}
		</p>
	);
}

export function FeaturesError({
	message,
	onRetry,
	retryLabel,
}: {
	message: string;
	onRetry?: () => void;
	retryLabel?: string;
}) {
	return (
		<div
			role="alert"
			className="rounded-md border border-destructive/40 bg-destructive/5 p-4 text-sm"
		>
			<p className="break-words whitespace-pre-wrap">{message}</p>
			{onRetry && (
				<button
					type="button"
					onClick={onRetry}
					className="mt-2 rounded border px-2 py-1 text-xs underline underline-offset-2"
				>
					{retryLabel}
				</button>
			)}
		</div>
	);
}

export function FeaturesEmpty({ message }: { message: string }) {
	return (
		<p className="py-8 text-center text-sm text-muted-foreground">{message}</p>
	);
}

export function formatUtc(ms: number): string {
	return new Date(ms).toISOString().replace("T", " ").slice(0, 19);
}
