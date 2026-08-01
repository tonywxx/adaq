import { useId, useRef, useState } from "react";
import type { MetricId } from "@/features/research/metric-catalog";
import { getMetricDefinition } from "@/features/research/metric-catalog";
import { cn } from "@/lib/utils";

export function MetricInfo({ metricId }: { metricId: MetricId }) {
	const definition = getMetricDefinition(metricId);
	const contentId = useId();
	const root = useRef(null as HTMLFieldSetElement | null);
	const [open, setOpen] = useState(false);
	const [pinned, setPinned] = useState(false);
	return (
		<fieldset
			ref={root}
			className="grid min-w-0 max-w-full gap-1 border-0 p-0 text-muted-foreground"
			onMouseEnter={() => setOpen(true)}
			onMouseLeave={() => {
				if (!pinned && !root.current?.contains(document.activeElement)) {
					setOpen(false);
				}
			}}
			onFocus={() => setOpen(true)}
			onBlur={(event) => {
				if (!pinned && !root.current?.contains(event.relatedTarget)) setOpen(false);
			}}
		>
			<div className="flex min-w-0 items-center gap-1">
				<span>{definition.label}</span>
				<button
					type="button"
					className="shrink-0 cursor-help rounded-sm px-1 underline decoration-dotted outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
					aria-label={`${definition.label} definition`}
					aria-expanded={open}
					aria-controls={contentId}
					onClick={() => {
						setPinned((current) => {
							setOpen(!current);
							return !current;
						});
					}}
					onKeyDown={(event) => {
						if (event.key === "Escape") {
							setPinned(false);
							setOpen(false);
						}
					}}
				>
					ⓘ
				</button>
			</div>
			{open && (
				<section
					id={contentId}
					aria-label={`${definition.label} definition`}
					className="max-w-full rounded-md border bg-popover p-3 text-xs text-popover-foreground shadow-md"
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
					<a
						className="mt-1 inline-block rounded-sm underline outline-none focus-visible:ring-2 focus-visible:ring-ring"
						href={definition.documentationUrl}
						target="_blank"
						rel="noreferrer"
					>
						Reference documentation
					</a>
				</section>
			)}
		</fieldset>
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
