import { LoaderCircleIcon } from "lucide-react";
import { useTranslation } from "react-i18next";
import { cn } from "@/lib/utils";

export function LoadingState({
	labelKey,
	className,
}: {
	labelKey: string;
	className?: string;
}) {
	const { t } = useTranslation();
	const translatedLabel = t(labelKey, {
		defaultValue: t("loading.page"),
	});

	return (
		<div
			className={cn(
				"flex min-h-24 items-center justify-center gap-2 text-sm text-muted-foreground",
				className,
			)}
			role="status"
			aria-live="polite"
		>
			<LoaderCircleIcon className="size-4 animate-spin" aria-hidden="true" />
			{translatedLabel}
		</div>
	);
}
