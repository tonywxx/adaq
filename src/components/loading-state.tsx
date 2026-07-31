import { LoaderCircleIcon } from "lucide-react";
import { cn } from "@/lib/utils";

export function LoadingState({
	label,
	className,
}: {
	label: string;
	className?: string;
}) {
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
			{label}
		</div>
	);
}
