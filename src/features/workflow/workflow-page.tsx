import { Link, useNavigate } from "@tanstack/react-router";
import type { Infographic as InfographicInstance } from "@antv/infographic";
import {
	ArrowDownLeftIcon,
	ArrowRightIcon,
	BookOpenIcon,
	BoxesIcon,
	DatabaseIcon,
	RouteIcon,
} from "lucide-react";
import { useTheme } from "next-themes";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Badge } from "@/components/ui/badge";
import { Button, buttonVariants } from "@/components/ui/button";
import {
	Card,
	CardAction,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "@/components/ui/card";
import {
	Sheet,
	SheetContent,
	SheetDescription,
	SheetFooter,
	SheetHeader,
	SheetTitle,
} from "@/components/ui/sheet";
import { Skeleton } from "@/components/ui/skeleton";
import { useIsMobile } from "@/hooks/use-mobile";
import { workflowModules, workflowSteps } from "./workflow";

export function WorkflowHomePage() {
	// M15 introduces the first authoritative Bot Runtime records. No current
	// source can establish Operational Responsibility, so home resolves to Help.
	return <WorkflowGuidePage />;
}

export function WorkflowGuidePage({
	selectedStepId,
}: {
	selectedStepId?: number;
}) {
	const { t } = useTranslation();
	const navigate = useNavigate();
	const isMobile = useIsMobile();
	const selectedStep = workflowSteps.find((step) => step.id === selectedStepId);
	const selectStep = useCallback(
		(stepId: number) => {
			void navigate({
				to: "/help/workflow/$step",
				params: { step: String(stepId) },
			});
		},
		[navigate],
	);

	return (
		<main className="flex min-w-0 flex-1 flex-col gap-6 p-4 lg:p-6">
			<section className="relative overflow-hidden rounded-2xl border bg-card p-5 shadow-xs lg:p-7">
				<div
					className="absolute inset-y-0 left-0 w-1 bg-primary"
					aria-hidden="true"
				/>
				<div className="max-w-3xl">
					<p className="mb-3 font-mono text-xs font-medium tracking-[0.18em] text-muted-foreground uppercase">
						{t("workflow.eyebrow")}
					</p>
					<h1 className="text-3xl font-semibold tracking-tight lg:text-4xl">
						{t("workflow.title")}
					</h1>
					<p className="mt-3 max-w-2xl text-sm leading-6 text-muted-foreground lg:text-base">
						{t("workflow.description")}
					</p>
					<div className="mt-5 flex flex-wrap gap-2">
						<Link to="/markets" className={buttonVariants()}>
							{t("workflow.reviewFoundations")}
							<ArrowRightIcon aria-hidden="true" />
						</Link>
						<a
							href="#workflow-steps"
							className={buttonVariants({ variant: "outline" })}
						>
							{t("workflow.browseSteps")}
						</a>
					</div>
				</div>
			</section>

			<section aria-labelledby="workflow-foundations-title">
				<div className="mb-3 flex items-end justify-between gap-4">
					<div>
						<h2 id="workflow-foundations-title" className="text-lg font-semibold">
							{t("workflow.foundations.title")}
						</h2>
						<p className="text-sm text-muted-foreground">
							{t("workflow.foundations.description")}
						</p>
					</div>
					<Badge variant="outline">{t("workflow.capability.available")}</Badge>
				</div>
				<div className="grid gap-3 md:grid-cols-2">
					<FoundationCard
						icon={<DatabaseIcon aria-hidden="true" />}
						title={t("workflow.foundations.data.title")}
						description={t("workflow.foundations.data.description")}
						to="/markets"
					/>
					<FoundationCard
						icon={<BoxesIcon aria-hidden="true" />}
						title={t("workflow.foundations.features.title")}
						description={t("workflow.foundations.features.description")}
						to="/features"
					/>
				</div>
			</section>

			<section aria-labelledby="workflow-map-title">
				<div className="mb-1">
					<h2 id="workflow-map-title" className="text-lg font-semibold">
						{t("workflow.map.title")}
					</h2>
					<p className="text-sm text-muted-foreground">
						{t("workflow.map.description")}
					</p>
				</div>
				<WorkflowInfographic onSelectStep={selectStep} />
				<div className="mt-3 flex flex-wrap items-center gap-2 rounded-lg border border-dashed px-3 py-2 text-xs text-muted-foreground">
					<ArrowDownLeftIcon className="size-4 text-primary" aria-hidden="true" />
					<span>{t("workflow.map.feedback")}</span>
				</div>
			</section>

			<section id="workflow-steps" aria-labelledby="workflow-steps-title">
				<div className="mb-3">
					<h2 id="workflow-steps-title" className="text-lg font-semibold">
						{t("workflow.stepsTitle")}
					</h2>
					<p className="text-sm text-muted-foreground">
						{t("workflow.stepsDescription")}
					</p>
				</div>
				<div className="grid gap-4 xl:grid-cols-2">
					{workflowModules.map((module) => (
						<section
							key={module.id}
							className="overflow-hidden rounded-xl border bg-card"
							aria-labelledby={`workflow-module-${module.id}`}
						>
							<div className="flex items-center gap-3 border-b px-4 py-3">
								<span className={`h-8 w-1 rounded-full ${module.accent}`} />
								<div className="min-w-0 flex-1">
									<h3 id={`workflow-module-${module.id}`} className="font-medium">
										{t(`workflow.modules.${module.id}.title`)}
									</h3>
									<p className="text-xs text-muted-foreground">
										{t("workflow.stepRange", { range: module.steps })}
									</p>
								</div>
							</div>
							<ol className="divide-y">
								{workflowSteps
									.filter((step) => step.module === module.id)
									.map((step) => (
										<li key={step.id}>
											<Link
												to="/help/workflow/$step"
												params={{ step: String(step.id) }}
												className="group flex w-full items-start gap-3 px-4 py-3 text-left transition-colors hover:bg-muted/60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset"
												aria-haspopup="dialog"
											>
												<span className="grid size-7 shrink-0 place-items-center rounded-full border bg-background font-mono text-xs font-semibold">
													{step.id}
												</span>
												<span className="min-w-0 flex-1">
													<span className="flex flex-wrap items-center gap-2">
														<span className="font-medium">
															{t(`workflow.steps.${step.id}.title`)}
														</span>
														<CapabilityBadge step={step} />
													</span>
													<span className="mt-1 block text-xs leading-5 text-muted-foreground">
														{t(`workflow.steps.${step.id}.summary`)}
													</span>
												</span>
												<ArrowRightIcon
													className="mt-1 size-4 shrink-0 text-muted-foreground transition-transform group-hover:translate-x-0.5"
													aria-hidden="true"
												/>
											</Link>
										</li>
									))}
							</ol>
						</section>
					))}
				</div>
			</section>

			<section
				className="grid gap-3 lg:grid-cols-2"
				aria-label={t("workflow.evidence.title")}
			>
				<EvidenceBoundary
					title={t("workflow.evidence.recentTitle")}
					description={t("workflow.evidence.recentDescription")}
				/>
				<EvidenceBoundary
					title={t("workflow.evidence.blockersTitle")}
					description={t("workflow.evidence.blockersDescription")}
				/>
			</section>

			<Sheet
				open={Boolean(selectedStep)}
				onOpenChange={(open) => {
					if (!open) void navigate({ to: "/help/workflow" });
				}}
			>
				{selectedStep ? (
					<SheetContent
						side={isMobile ? "bottom" : "right"}
						className="max-h-[88svh] overflow-y-auto sm:max-w-md"
					>
						<SheetHeader className="border-b pr-12">
							<div className="mb-2 flex items-center gap-2">
								<span className="grid size-8 place-items-center rounded-full bg-primary font-mono text-xs font-semibold text-primary-foreground">
									{selectedStep.id}
								</span>
								<CapabilityBadge step={selectedStep} />
							</div>
							<SheetTitle>{t(`workflow.steps.${selectedStep.id}.title`)}</SheetTitle>
							<SheetDescription>
								{t(`workflow.steps.${selectedStep.id}.summary`)}
							</SheetDescription>
						</SheetHeader>
						<div className="grid gap-5 px-4">
							<DetailField
								label={t("workflow.detail.output")}
								value={t(`workflow.steps.${selectedStep.id}.output`)}
							/>
							<DetailField
								label={t("workflow.detail.requirement")}
								value={t(`workflow.steps.${selectedStep.id}.requirement`)}
							/>
							<DetailField
								label={t("workflow.detail.currentEntry")}
								value={
									selectedStep.target
										? t(`workflow.steps.${selectedStep.id}.entry`)
										: t("workflow.detail.plannedEntry")
								}
							/>
						</div>
						<SheetFooter>
							{selectedStep.target ? (
								<Link to={selectedStep.target} className={buttonVariants()}>
									{t("workflow.openWorkspace")}
									<ArrowRightIcon aria-hidden="true" />
								</Link>
							) : (
								<Button disabled>{t("workflow.notImplemented")}</Button>
							)}
						</SheetFooter>
					</SheetContent>
				) : null}
			</Sheet>
		</main>
	);
}

function FoundationCard({
	icon,
	title,
	description,
	to,
}: {
	icon: React.ReactNode;
	title: string;
	description: string;
	to: "/markets" | "/features";
}) {
	const { t } = useTranslation();
	return (
		<Card size="sm">
			<CardHeader>
				<div className="mb-2 grid size-8 place-items-center rounded-lg bg-primary/10 text-primary [&>svg]:size-4">
					{icon}
				</div>
				<CardTitle>{title}</CardTitle>
				<CardDescription>{description}</CardDescription>
				<CardAction>
					<Badge variant="outline">{t("workflow.capability.available")}</Badge>
				</CardAction>
			</CardHeader>
			<CardContent>
				<Link
					to={to}
					className={buttonVariants({ size: "sm", variant: "outline" })}
				>
					{t("workflow.openWorkspace")}
					<ArrowRightIcon aria-hidden="true" />
				</Link>
			</CardContent>
		</Card>
	);
}

function WorkflowInfographic({
	onSelectStep,
}: {
	onSelectStep: (stepId: number) => void;
}) {
	const { t } = useTranslation();
	const { resolvedTheme } = useTheme();
	const isMobile = useIsMobile();
	const containerRef = useRef<HTMLDivElement>(null);
	const [state, setState] = useState<"loading" | "ready" | "error">("loading");
	const data = useMemo(
		() => ({
			lists: workflowSteps.map((step) => ({
				label: String(step.id).padStart(2, "0"),
				desc: t(`workflow.steps.${step.id}.shortTitle`),
			})),
		}),
		[t],
	);

	useEffect(() => {
		let active = true;
		let infographic: InfographicInstance | undefined;
		let secondFrame = 0;
		const container = containerRef.current;
		const handleClick = (event: Event) => {
			const target = event.target;
			if (!(target instanceof Element)) return;
			const index = Number.parseInt(
				target.closest("[data-indexes]")?.getAttribute("data-indexes") ?? "",
				10,
			);
			const step = workflowSteps[index];
			if (step) onSelectStep(step.id);
		};
		container?.addEventListener("click", handleClick);
		setState("loading");
		const firstFrame = requestAnimationFrame(() => {
			secondFrame = requestAnimationFrame(() => {
				void import("@antv/infographic")
					.then(({ Infographic }) => {
						if (!active || !containerRef.current) return;
						infographic = new Infographic({
							container: containerRef.current,
							width: "100%",
							height: "100%",
							template: "list-grid-horizontal-icon-arrow",
							design: {
								structure: {
									type: "list-grid",
									columns: isMobile ? 2 : 5,
									gap: 0,
									zigzag: true,
								},
							},
							data,
							theme: resolvedTheme === "dark" ? "dark" : "default",
							themeConfig: {
								colorPrimary: "#2563eb",
								palette: [
									"#0891b2",
									"#0891b2",
									"#0891b2",
									"#7c3aed",
									"#7c3aed",
									"#7c3aed",
									"#d97706",
									"#d97706",
									"#e11d48",
									"#e11d48",
								],
							},
							padding: 24,
							editable: false,
						});
						infographic.render();
						setState("ready");
					})
					.catch(() => {
						if (active) setState("error");
					});
			});
		});

		return () => {
			active = false;
			cancelAnimationFrame(firstFrame);
			cancelAnimationFrame(secondFrame);
			container?.removeEventListener("click", handleClick);
			infographic?.destroy();
		};
	}, [data, isMobile, onSelectStep, resolvedTheme]);

	return (
		<div
			className={`relative overflow-hidden rounded-xl border bg-card ${isMobile ? "min-h-[920px]" : "min-h-[430px]"}`}
		>
			{state === "loading" ? (
				<div className="absolute inset-0 grid gap-3 p-6" aria-live="polite">
					<span className="sr-only">{t("workflow.map.loading")}</span>
					<Skeleton className="h-8 w-64" />
					<div className="grid grid-cols-2 gap-3 md:grid-cols-3">
						{workflowSteps.slice(0, 9).map((step) => (
							<Skeleton key={step.id} className="h-28" />
						))}
					</div>
				</div>
			) : null}
			<div
				ref={containerRef}
				className="absolute inset-0 [&_[data-indexes]]:cursor-pointer"
				aria-hidden="true"
			/>
			{state === "error" ? (
				<div className="absolute inset-0 grid place-content-center gap-2 p-6 text-center">
					<RouteIcon
						className="mx-auto size-6 text-muted-foreground"
						aria-hidden="true"
					/>
					<p className="font-medium">{t("workflow.map.unavailable")}</p>
					<p className="max-w-md text-sm text-muted-foreground">
						{t("workflow.map.unavailableDescription")}
					</p>
				</div>
			) : null}
		</div>
	);
}

function CapabilityBadge({ step }: { step: (typeof workflowSteps)[number] }) {
	const { t } = useTranslation();
	return (
		<Badge variant={step.capability === "planned" ? "outline" : "secondary"}>
			{step.capability === "planned"
				? t("workflow.capability.planned", { milestone: step.milestone })
				: t(`workflow.capability.${step.capability}`)}
		</Badge>
	);
}

function DetailField({ label, value }: { label: string; value: string }) {
	return (
		<div>
			<p className="mb-1 text-xs font-medium tracking-wide text-muted-foreground uppercase">
				{label}
			</p>
			<p className="leading-6">{value}</p>
		</div>
	);
}

function EvidenceBoundary({
	title,
	description,
}: {
	title: string;
	description: string;
}) {
	const { t } = useTranslation();
	return (
		<Card size="sm" className="border-dashed">
			<CardHeader>
				<div className="mb-2 grid size-8 place-items-center rounded-lg bg-muted text-muted-foreground">
					<BookOpenIcon className="size-4" aria-hidden="true" />
				</div>
				<CardTitle>{title}</CardTitle>
				<CardDescription>{description}</CardDescription>
				<CardAction>
					<Badge variant="outline">
						{t("workflow.capability.planned", { milestone: "V1" })}
					</Badge>
				</CardAction>
			</CardHeader>
		</Card>
	);
}
