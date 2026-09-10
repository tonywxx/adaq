import { useTranslation } from "react-i18next";
import { identifierName } from "@/lib/identifier-display";
import { cn } from "@/lib/utils";

export function IdentifierDisplay({
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
