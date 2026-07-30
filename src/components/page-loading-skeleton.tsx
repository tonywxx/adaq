import { Card, CardContent, CardHeader } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";

export function PageLoadingSkeleton() {
	return (
		<div
			className="flex min-w-0 flex-1 flex-col gap-5 p-4 lg:p-6"
			aria-busy="true"
		>
			<div className="space-y-2">
				<Skeleton className="h-8 w-36" />
				<Skeleton className="h-4 w-72 max-w-full" />
			</div>
			<Card>
				<CardHeader className="space-y-2">
					<Skeleton className="h-5 w-48" />
					<Skeleton className="h-4 w-full max-w-md" />
				</CardHeader>
				<CardContent className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
					{["first", "second", "third"].map((key) => (
						<Skeleton key={key} className="h-20 w-full" />
					))}
				</CardContent>
			</Card>
			<Card>
				<CardContent className="space-y-3 p-6">
					<Skeleton className="h-5 w-56" />
					<Skeleton className="h-4 w-full" />
					<Skeleton className="h-4 w-4/5" />
				</CardContent>
			</Card>
		</div>
	);
}
