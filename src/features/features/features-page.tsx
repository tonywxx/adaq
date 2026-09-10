import { invoke } from "@tauri-apps/api/core";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useMarketSessionStore } from "@/lib/market-session";
import { SigmaIcon } from "lucide-react";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { ResearchContextPreflight } from "@/features/research/research-context-preflight";
import { createFeaturesAdapter } from "./features-adapter";
import { DefinitionsView } from "./definitions-view";
import { FittingView, MaterializationView } from "./features-attempts";
import { DatasetsView } from "./features-datasets";
import { FeaturesLoading } from "./features-shared";

// The route shell paints immediately; each owning control below manages its
// own loading, error, and empty state against the native Feature commands.
export function FeaturesPage() {
	const { t } = useTranslation();
	const userId = useMarketSessionStore((state) => state.userId);
	const adapter = useMemo(() => createFeaturesAdapter(invoke), []);

	return (
		<div className="flex min-w-0 flex-1 flex-col gap-5 p-4 lg:p-6">
			<div>
				<div className="flex items-center gap-2">
					<SigmaIcon className="size-5 text-primary" aria-hidden="true" />
					<h1 className="text-2xl font-semibold">{t("features.title")}</h1>
				</div>
				<p className="text-sm text-muted-foreground">{t("features.description")}</p>
			</div>

			{userId ? (
				<ResearchContextPreflight userId={userId} stage="features" />
			) : null}
			{!userId ? (
				<FeaturesLoading label={t("features.loading")} />
			) : (
				<Tabs defaultValue="definitions" className="min-w-0 gap-4">
					<TabsList
						aria-label={t("features.tabsLabel")}
						className="w-full flex-wrap justify-start gap-1 group-data-horizontal/tabs:h-auto"
					>
						<TabsTrigger value="definitions" className="flex-none">
							{t("features.tabs.definitions")}
						</TabsTrigger>
						<TabsTrigger value="fitting" className="flex-none">
							{t("features.tabs.fitting")}
						</TabsTrigger>
						<TabsTrigger value="materialization" className="flex-none">
							{t("features.tabs.materialization")}
						</TabsTrigger>
						<TabsTrigger value="datasets" className="flex-none">
							{t("features.tabs.datasets")}
						</TabsTrigger>
					</TabsList>
					<TabsContent value="definitions" className="min-w-0">
						<DefinitionsView userId={userId} adapter={adapter} />
					</TabsContent>
					<TabsContent value="fitting" className="min-w-0">
						<FittingView userId={userId} adapter={adapter} />
					</TabsContent>
					<TabsContent value="materialization" className="min-w-0">
						<MaterializationView userId={userId} adapter={adapter} />
					</TabsContent>
					<TabsContent value="datasets" className="min-w-0">
						<DatasetsView userId={userId} adapter={adapter} />
					</TabsContent>
				</Tabs>
			)}
		</div>
	);
}
