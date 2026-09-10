import { useTranslation } from "react-i18next";
import { CheckIcon, CopyIcon } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import {
	Tooltip,
	TooltipContent,
	TooltipTrigger,
} from "@/components/ui/tooltip";
import { abbreviateIdentifier, identifierName } from "@/lib/identifier-display";
import { cn } from "@/lib/utils";

function copyToClipboard(value: string) {
	void navigator.clipboard?.writeText(value);
}

// "compact" keeps the row to one line and moves the complete identity into the tooltip,
// copying the raw value on click. "stacked" shows the name and the full id as two lines.
export function IdentifierDisplay({
	id,
	label,
	name,
	className,
	variant = "stacked",
}: {
	id: string;
	label: string;
	name?: string | null;
	className?: string;
	variant?: "stacked" | "compact";
}) {
	if (variant === "compact") {
		return (
			<CompactIdentifier
				id={id}
				name={identifierName(id, label, name)}
				className={className}
			/>
		);
	}
	return (
		<StackedIdentifier id={id} label={label} name={name} className={className} />
	);
}

function StackedIdentifier({
	id,
	label,
	name,
	className,
}: {
	id: string;
	label: string;
	name?: string | null;
	className?: string;
}) {
	const { t } = useTranslation();
	return (
		<span
			className={cn(
				"inline-flex min-w-0 max-w-full flex-col text-left font-sans",
				className,
			)}
		>
			<span className="font-medium">{identifierName(id, label, name)}</span>
			<code className="select-text break-all text-xs font-normal text-muted-foreground">
				{t("identifiers.fullId")}: {id}
			</code>
		</span>
	);
}

function CompactIdentifier({
	id,
	name,
	className,
}: {
	id: string;
	name: string;
	className?: string;
}) {
	const { t } = useTranslation();
	const [copied, setCopied] = useState(false);
	const timeoutRef = useRef<number | undefined>(undefined);
	useEffect(() => () => window.clearTimeout(timeoutRef.current), []);
	return (
		<Tooltip>
			<TooltipTrigger
				className={cn(
					"inline-flex min-w-0 max-w-full cursor-copy items-baseline gap-1.5 rounded-sm text-left font-sans outline-none focus-visible:ring-2 focus-visible:ring-ring",
					className,
				)}
				aria-label={t("identifiers.copyHint")}
				onClick={() => {
					copyToClipboard(id);
					setCopied(true);
					window.clearTimeout(timeoutRef.current);
					timeoutRef.current = window.setTimeout(() => setCopied(false), 1500);
				}}
			>
				<span className="truncate font-medium">{name}</span>
				<code className="font-mono text-xs font-normal text-muted-foreground">
					{abbreviateIdentifier(id)}
				</code>
				{copied ? (
					<CheckIcon className="size-3.5 shrink-0 text-primary" aria-hidden="true" />
				) : (
					<CopyIcon
						className="size-3.5 shrink-0 text-muted-foreground"
						aria-hidden="true"
					/>
				)}
			</TooltipTrigger>
			<TooltipContent
				sideOffset={4}
				className="block max-w-[min(28rem,calc(100vw-2rem))]"
			>
				<span className="text-muted-foreground">{t("identifiers.fullId")}</span>
				<code className="mt-1 block break-all font-mono">{id}</code>
			</TooltipContent>
		</Tooltip>
	);
}
