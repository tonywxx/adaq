import { useState } from "react";
import {
	Tooltip,
	TooltipContent,
	TooltipTrigger,
} from "@/components/ui/tooltip";
import type { MetricId } from "@/features/research/metric-catalog";
import { getMetricDefinition } from "@/features/research/metric-catalog";
import { cn } from "@/lib/utils";

export function MetricInfo({ metricId }: { metricId: MetricId }) {
	const definition = getMetricDefinition(metricId);
	const [open, setOpen] = useState(false);
	return (
		<Tooltip open={open} onOpenChange={setOpen}>
			<TooltipTrigger
				delay={0}
				className="w-fit cursor-help border-b border-dashed border-current text-left text-muted-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
				aria-label={`${definition.label} definition`}
				onClick={() => setOpen((current) => !current)}
			>
				{definition.label}
			</TooltipTrigger>
			<TooltipContent
				align="start"
				sideOffset={4}
				className="block max-w-[min(24rem,calc(100vw-2rem))]"
			>
				<p>{definition.meaning}</p>
				<p className="mt-1">Formula: {definition.formula}</p>
				<p>Direction: {definition.direction}</p>
				{definition.range && <p>Range: {definition.range}</p>}
				<p>Caveat: {definition.caveat}</p>
				<p>Undefined: {definition.undefinedState}</p>
				<p className="mt-1 font-mono">
					{definition.id}@{definition.version}
				</p>
			</TooltipContent>
		</Tooltip>
	);
}

export function ResearchMetric({
	metricId,
	value,
	className,
	valueClassName = "font-medium",
}: {
	metricId: MetricId;
	value: string;
	className?: string;
	valueClassName?: string;
}) {
	return (
		<div className={className}>
			<MetricInfo metricId={metricId} />
			<p className={cn("break-all select-text", valueClassName)}>{value}</p>
		</div>
	);
}
