import { Badge } from "@/components/ui/badge";
import {
	Card,
	CardAction,
	CardContent,
	CardDescription,
	CardFooter,
	CardHeader,
	CardTitle,
} from "@/components/ui/card";

export function TempWCard() {
	return (
		<Card className="@container/card rounded-none py-4 from-primary/5 to-card dark:bg-card bg-linear-to-t shadow-xs">
			<CardHeader>
				<CardDescription>Total Revenue</CardDescription>
				<CardTitle className="text-2xl font-semibold tabular-nums @[250px]/card:text-3xl">
					$1,250.00
				</CardTitle>
				<CardAction>
					<Badge variant="outline">+12.5%</Badge>
				</CardAction>
			</CardHeader>
			<CardContent>
				<p>111</p>
			</CardContent>
			<CardFooter className="flex-col items-start gap-1.5 text-sm">
				<div className="line-clamp-1 flex gap-2 font-medium">
					Trending up this month
				</div>
				<div className="text-muted-foreground">Visitors for the last 6 months</div>
			</CardFooter>
		</Card>
	);
}
