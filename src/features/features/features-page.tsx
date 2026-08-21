import { invoke } from "@tauri-apps/api/core";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useMarketSessionStore } from "@/lib/market-session";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { ResearchContextPreflight } from "@/features/research/research-context-preflight";
import { createFeaturesAdapter } from "./features-adapter";
import { DefinitionsView } from "./definitions-view";
import { FittingView, MaterializationView } from "./features-attempts";
import { DatasetsView } from "./features-datasets";

// The route shell paints immediately; each owning control below manages its
// own loading, error, and empty state against the native Feature commands.
export function FeaturesPage() {
	const { t } = useTranslation();
	const userId = useMarketSessionStore((state) => state.userId);
	const adapter = useMemo(() => createFeaturesAdapter(invoke), []);

	return (
		<div className="mx-auto max-w-6xl space-y-6 p-4 md:p-6">
			<header>
				<h1 className="text-2xl font-semibold tracking-tight">
					{t("features.title")}
				</h1>
				<p className="mt-1 text-sm text-muted-foreground">
					{t("features.description")}
				</p>
			</header>

			{userId ? (
				<ResearchContextPreflight userId={userId} stage="features" />
			) : null}
			{!userId ? (
				<p
					aria-busy="true"
					className="py-8 text-center text-sm text-muted-foreground"
				>
					{t("features.loading")}
				</p>
			) : (
				<Tabs defaultValue="definitions">
					<TabsList className="flex-wrap">
						<TabsTrigger value="definitions">
							{t("features.tabs.definitions")}
						</TabsTrigger>
						<TabsTrigger value="fitting">{t("features.tabs.fitting")}</TabsTrigger>
						<TabsTrigger value="materialization">
							{t("features.tabs.materialization")}
						</TabsTrigger>
						<TabsTrigger value="datasets">{t("features.tabs.datasets")}</TabsTrigger>
					</TabsList>
					<TabsContent value="definitions" className="mt-4">
						<DefinitionsView userId={userId} adapter={adapter} />
					</TabsContent>
					<TabsContent value="fitting" className="mt-4">
						<FittingView userId={userId} adapter={adapter} />
					</TabsContent>
					<TabsContent value="materialization" className="mt-4">
						<MaterializationView userId={userId} adapter={adapter} />
					</TabsContent>
					<TabsContent value="datasets" className="mt-4">
						<DatasetsView userId={userId} adapter={adapter} />
					</TabsContent>
				</Tabs>
			)}
		</div>
	);
}
