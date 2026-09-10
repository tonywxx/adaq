import { useTranslation } from "react-i18next";
import { CheckIcon, CopyIcon } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import {
	Tooltip,
	TooltipContent,
	TooltipTrigger,
} from "@/components/ui/tooltip";
import {
	abbreviateIdentifier,
	identifierName,
	identifierOptionalName,
} from "@/lib/identifier-display";
import { cn } from "@/lib/utils";

function copyToClipboard(value: string) {
	void navigator.clipboard?.writeText(value);
}

// The one way this app shows a hash or long opaque identifier: readable name (when
// known) plus the value abbreviated to "abc…xyz". Hovering reveals the complete
// value and clicking copies it. `label` stays optional because most rows already
// name the value next to it, and repeating the name there would read twice.
export function IdentifierDisplay({
	id,
	label,
	name,
	className,
}: {
	id: string;
	label?: string;
	name?: string | null;
	className?: string;
}) {
	const { t } = useTranslation();
	const [copied, setCopied] = useState(false);
	const timeoutRef = useRef<number | undefined>(undefined);
	useEffect(() => () => window.clearTimeout(timeoutRef.current), []);
	const visibleName =
		identifierOptionalName(name, id) ?? identifierOptionalName(label, id);
	return (
		<Tooltip>
			<TooltipTrigger
				className={cn(
					"inline-flex min-w-0 max-w-full cursor-copy items-baseline gap-1.5 rounded-sm text-left font-sans outline-none focus-visible:ring-2 focus-visible:ring-ring",
					className,
				)}
				aria-label={`${visibleName ? `${visibleName} — ` : ""}${t("identifiers.copyHint")}`}
				onClick={() => {
					copyToClipboard(id);
					setCopied(true);
					window.clearTimeout(timeoutRef.current);
					timeoutRef.current = window.setTimeout(() => setCopied(false), 1500);
				}}
			>
				{visibleName ? (
					<span className="truncate font-medium">{visibleName}</span>
				) : null}
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
				<span className="text-muted-foreground">
					{visibleName ?? identifierName(id, t("identifiers.fullId"))}
				</span>
				<code className="mt-1 block break-all font-mono">{id}</code>
			</TooltipContent>
		</Tooltip>
	);
}
