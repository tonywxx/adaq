import { LaptopIcon, MoonIcon, SunIcon } from "lucide-react";
import { useTheme } from "next-themes";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuRadioGroup,
	DropdownMenuRadioItem,
	DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { useTranslation } from "react-i18next";

type ThemeMode = "light" | "dark" | "system";

const themeOptions: {
	value: ThemeMode;
	labelKey: "theme.light" | "theme.dark" | "theme.system";
	icon: typeof SunIcon;
}[] = [
	{ value: "light", labelKey: "theme.light", icon: SunIcon },
	{ value: "dark", labelKey: "theme.dark", icon: MoonIcon },
	{ value: "system", labelKey: "theme.system", icon: LaptopIcon },
];

export function DarkModeDropDownMenu() {
	const { setTheme, theme = "system" } = useTheme();
	const { t } = useTranslation();

	const activeTheme = themeOptions.some((option) => option.value === theme)
		? (theme as ThemeMode)
		: "system";
	const ActiveThemeIcon =
		themeOptions.find((option) => option.value === activeTheme)?.icon ??
		LaptopIcon;

	return (
		<DropdownMenu>
			<DropdownMenuTrigger asChild>
				<button
					type="button"
					aria-label={t("titlebar.theme", {
						theme: t(
							themeOptions.find((option) => option.value === activeTheme)?.labelKey ??
								"theme.system",
						),
					})}
					title={t("titlebar.themeLabel")}
					className="flex size-8 shrink-0 items-center justify-center rounded-md text-sidebar-foreground/55 transition-colors hover:bg-sidebar-accent hover:text-sidebar-foreground"
					onClick={(event) => event.stopPropagation()}
					onPointerDown={(event) => event.stopPropagation()}
				>
					<ActiveThemeIcon className="size-4" />
				</button>
			</DropdownMenuTrigger>
			<DropdownMenuContent
				align="end"
				className="w-36"
				onPointerDown={(event) => event.stopPropagation()}
				onClick={(event) => event.stopPropagation()}
			>
				<DropdownMenuRadioGroup
					value={activeTheme}
					onValueChange={(value) => setTheme(value as ThemeMode)}
				>
					{themeOptions.map((option) => {
						const Icon = option.icon;

						return (
							<DropdownMenuRadioItem
								key={option.value}
								value={option.value}
								className="gap-2"
							>
								<Icon className="size-4" />
								<span>{t(option.labelKey)}</span>
							</DropdownMenuRadioItem>
						);
					})}
				</DropdownMenuRadioGroup>
			</DropdownMenuContent>
		</DropdownMenu>
	);
}
