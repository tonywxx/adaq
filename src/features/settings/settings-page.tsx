import { Avatar, AvatarFallback, AvatarImage } from "@/components/ui/avatar";
import { Button } from "@/components/ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Separator } from "@/components/ui/separator";
import {
	autoDownloadUpdates,
	LAST_APP_PATH_KEY,
	setAutoDownloadUpdates,
} from "@/lib/app-settings";
import {
	changeInterfaceLocale,
	formatNumber,
	getInterfaceLocalePreference,
	type InterfaceLocalePreference,
} from "@/lib/i18n";
import { checkStrongPassword } from "@/lib/password";
import { supabase } from "@/lib/supabase";
import { cn } from "@/lib/utils";
import { ConnectionsSettings } from "@/features/settings/connections-settings";
import { PythonRuntimeSettings } from "@/features/python-research/python-runtime-settings";
import type { User } from "@supabase/supabase-js";
import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import {
	Link,
	useNavigate,
	useParams,
	useRouter,
} from "@tanstack/react-router";
import {
	ArrowLeftIcon,
	CheckIcon,
	ChevronRightIcon,
	DatabaseIcon,
	KeyboardIcon,
	LaptopIcon,
	LogOutIcon,
	MoonIcon,
	PaletteIcon,
	PlugZapIcon,
	RefreshCwIcon,
	Settings2Icon,
	ShieldIcon,
	SunIcon,
	UserRoundIcon,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { useTheme } from "next-themes";
import {
	type FormEvent,
	type ReactNode,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { toast } from "sonner";

const sections = [
	{
		id: "general",
		labelKey: "settings.navigation.general",
		icon: Settings2Icon,
	},
	{
		id: "profile",
		labelKey: "settings.navigation.profile",
		icon: UserRoundIcon,
	},
	{
		id: "appearance",
		labelKey: "settings.navigation.appearance",
		icon: PaletteIcon,
	},
	{
		id: "keyboard-shortcuts",
		labelKey: "settings.navigation.keyboardShortcuts",
		icon: KeyboardIcon,
	},
	{ id: "account", labelKey: "settings.navigation.account", icon: ShieldIcon },
	{
		id: "connections",
		labelKey: "settings.navigation.connections",
		icon: PlugZapIcon,
	},
	{
		id: "data-storage",
		labelKey: "settings.navigation.dataStorage",
		icon: DatabaseIcon,
	},
] as const;

type Section = (typeof sections)[number]["id"];
type ResetKind =
	| "watchlist"
	| "components"
	| "marketData"
	| "all"
	| "factorResearch";

type LocalDataSummary = {
	dataDirectory: string;
	databaseBytes: number;
	componentBytes: number;
	marketDataBytes: number;
	watchlistCount: number;
	componentCount: number;
	snapshotCount: number;
	runCount: number;
	protocolCount: number;
	reportCount: number;
	generationAttemptCount: number;
	modelArtifactCount: number;
	signalDatasetCount: number;
	componentBlockingRunCount: number;
	marketDataBlockingRecordCount: number;
};

function isTauriRuntime() {
	return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export function SettingsPage() {
	const params = useParams({ strict: false });
	const navigate = useNavigate();
	const section = sections.some((item) => item.id === params.section)
		? (params.section as Section)
		: "general";

	useEffect(() => {
		if (params.section === section) return;
		void navigate({
			to: "/settings/$section",
			params: { section },
			replace: true,
		});
	}, [navigate, params.section, section]);

	return (
		<div className="flex min-h-full w-full bg-background">
			<SettingsNavigation section={section} />
			<main className="min-w-0 flex-1 px-6 py-8 lg:px-10">
				<div className="mx-auto w-full max-w-3xl">
					{section === "general" && <GeneralSettings />}
					{section === "profile" && <ProfileSettings />}
					{section === "appearance" && <AppearanceSettings />}
					{section === "keyboard-shortcuts" && <KeyboardSettings />}
					{section === "account" && <AccountSettings />}
					{section === "connections" && <ConnectionsSettings />}
					{section === "data-storage" && <DataStorageSettings />}
				</div>
			</main>
		</div>
	);
}

function SettingsNavigation({ section }: { section: Section }) {
	const router = useRouter();
	const { t } = useTranslation();
	const backToApp = () => {
		router.history.push(sessionStorage.getItem(LAST_APP_PATH_KEY) || "/");
	};

	return (
		<aside className="sticky top-0 h-[calc(100svh-3rem)] w-72 shrink-0 border-r bg-muted/25 px-3 py-5">
			<Button className="mb-5 justify-start" variant="ghost" onClick={backToApp}>
				<ArrowLeftIcon />
				{t("settings.navigation.backToApp")}
			</Button>
			<nav aria-label={t("settings.navigation.label")}>
				<ul className="grid gap-1">
					{sections.map((item) => {
						const Icon = item.icon;
						return (
							<li key={item.id}>
								<Link
									to="/settings/$section"
									params={{ section: item.id }}
									className={cn(
										"flex h-9 items-center gap-2 rounded-lg px-3 text-sm transition-colors hover:bg-muted",
										section === item.id && "bg-muted font-medium",
									)}
								>
									<Icon className="size-4" />
									<span className="truncate">{t(item.labelKey)}</span>
									{section === item.id ? (
										<ChevronRightIcon className="ml-auto size-3.5 text-muted-foreground" />
									) : null}
								</Link>
							</li>
						);
					})}
				</ul>
			</nav>
		</aside>
	);
}

function SettingsHeader({
	title,
	description,
}: {
	title: string;
	description: string;
}) {
	return (
		<header className="mb-6">
			<h2 className="text-2xl font-semibold tracking-tight">{title}</h2>
			<p className="mt-1 text-sm text-muted-foreground">{description}</p>
		</header>
	);
}

function GeneralSettings() {
	const { t } = useTranslation();
	const [autoDownload, setAutoDownload] = useState(autoDownloadUpdates);
	const [version, setVersion] = useState<string | null>(null);
	const [interfaceLocale, setInterfaceLocale] =
		useState<InterfaceLocalePreference>(getInterfaceLocalePreference);

	useEffect(() => {
		if (!isTauriRuntime()) {
			setVersion("Development");
			return;
		}
		void getVersion()
			.then(setVersion)
			.catch(() => setVersion("Unavailable"));
	}, []);

	const versionLabel =
		version === null
			? t("settings.general.updates.loading")
			: version === "Development"
				? t("settings.general.updates.development")
				: version === "Unavailable"
					? t("settings.general.updates.unavailable")
					: version;

	return (
		<>
			<SettingsHeader
				title={t("settings.general.title")}
				description={t("settings.general.description")}
			/>
			<Card>
				<CardHeader>
					<CardTitle>{t("settings.general.language.title")}</CardTitle>
					<CardDescription>
						{t("settings.general.language.description")}
					</CardDescription>
				</CardHeader>
				<CardContent>
					<div className="grid max-w-sm gap-2">
						<Label htmlFor="interface-locale">
							{t("settings.general.language.label")}
						</Label>
						<select
							id="interface-locale"
							value={interfaceLocale}
							onChange={(event) => {
								const preference = event.target.value as InterfaceLocalePreference;
								setInterfaceLocale(preference);
								void changeInterfaceLocale(preference);
							}}
							className="h-9 rounded-lg border border-input bg-background px-3 text-sm outline-none focus-visible:ring-3 focus-visible:ring-ring/50"
						>
							<option value="system">{t("settings.general.language.system")}</option>
							<option value="en-US">{t("settings.general.language.englishUS")}</option>
							<option value="zh-CN">
								{t("settings.general.language.simplifiedChinese")}
							</option>
						</select>
					</div>
				</CardContent>
			</Card>
			<br />
			<Card>
				<CardHeader>
					<CardTitle>{t("settings.general.updates.title")}</CardTitle>
					<CardDescription>
						{t("settings.general.updates.description")}
					</CardDescription>
				</CardHeader>
				<CardContent className="grid gap-4">
					<div className="flex items-center justify-between gap-6">
						<span>
							<span className="block font-medium">
								{t("settings.general.updates.autoDownload")}
							</span>
							<span className="block text-sm text-muted-foreground">
								{t("settings.general.updates.autoDownloadDescription")}
							</span>
						</span>
						<Checkbox
							aria-label={t("settings.general.updates.autoDownload")}
							checked={autoDownload}
							onCheckedChange={(checked) => {
								const enabled = checked === true;
								setAutoDownload(enabled);
								setAutoDownloadUpdates(enabled);
							}}
						/>
					</div>
					<Separator />
					<div className="flex items-center justify-between gap-4">
						<div>
							<p className="font-medium">
								{t("settings.general.updates.version", { version: versionLabel })}
							</p>
							<p className="text-sm text-muted-foreground">
								{t("settings.general.updates.versionDescription")}
							</p>
						</div>
						<Button
							variant="outline"
							disabled={!isTauriRuntime()}
							onClick={() => void emit("adaq-check-for-updates")}
						>
							<RefreshCwIcon />
							{t("settings.general.updates.check")}
						</Button>
					</div>
				</CardContent>
			</Card>
			<br />
			<br />
			<Card>
				<CardHeader>
					<CardTitle className="text-base">
						{t("settings.general.disclaimer.title")}
					</CardTitle>
				</CardHeader>
				<CardContent className="grid gap-4 space-y-3 text-sm leading-relaxed text-muted-foreground">
					<p>{t("settings.general.disclaimer.text")}</p>
				</CardContent>
			</Card>
		</>
	);
}

function useAuthUser() {
	const [user, setUser] = useState<User | null>(null);
	useEffect(() => {
		if (!supabase) return;
		void supabase.auth.getUser().then(({ data }) => setUser(data.user));
		const { data } = supabase.auth.onAuthStateChange((_event, session) =>
			setUser(session?.user ?? null),
		);
		return () => data.subscription.unsubscribe();
	}, []);
	return user;
}

function ProfileSettings() {
	const { t } = useTranslation();
	const user = useAuthUser();
	const initialName =
		user?.user_metadata.full_name ?? user?.user_metadata.name ?? "";
	const [displayName, setDisplayName] = useState("");
	const [saving, setSaving] = useState(false);

	useEffect(() => setDisplayName(initialName), [initialName]);

	async function saveProfile(event: FormEvent) {
		event.preventDefault();
		if (!supabase || !displayName.trim()) return;
		setSaving(true);
		const { error } = await supabase.auth.updateUser({
			data: { full_name: displayName.trim() },
		});
		setSaving(false);
		if (error) return toast.error(error.message);
		toast.success(t("settings.profile.saved"));
	}

	const avatar = user?.user_metadata.avatar_url ?? user?.user_metadata.picture;
	const initials = (displayName || user?.email || "A").slice(0, 2).toUpperCase();

	return (
		<>
			<SettingsHeader
				title={t("settings.profile.title")}
				description={t("settings.profile.description")}
			/>
			<Card>
				<CardContent>
					<form className="grid gap-5" onSubmit={saveProfile}>
						<div className="flex items-center gap-4">
							<Avatar size="lg">
								<AvatarImage
									src={avatar}
									alt={displayName || t("settings.profile.avatarAlt")}
								/>
								<AvatarFallback>{initials}</AvatarFallback>
							</Avatar>
							<p className="text-sm text-muted-foreground">
								{t("settings.profile.avatarDescription")}
							</p>
						</div>
						<div className="grid gap-2">
							<Label htmlFor="display-name">{t("settings.profile.displayName")}</Label>
							<Input
								id="display-name"
								value={displayName}
								onChange={(event) => setDisplayName(event.target.value)}
								maxLength={80}
								required
							/>
						</div>
						<Button className="w-fit" loading={saving} disabled={!displayName.trim()}>
							{t("settings.profile.save")}
						</Button>
					</form>
				</CardContent>
			</Card>
		</>
	);
}

function AppearanceSettings() {
	const { t } = useTranslation();
	const { setTheme, theme = "system" } = useTheme();
	const options = [
		{ id: "system", labelKey: "theme.system", icon: LaptopIcon },
		{ id: "light", labelKey: "theme.light", icon: SunIcon },
		{ id: "dark", labelKey: "theme.dark", icon: MoonIcon },
	];
	return (
		<>
			<SettingsHeader
				title={t("settings.appearance.title")}
				description={t("settings.appearance.description")}
			/>
			<Card>
				<CardHeader>
					<CardTitle>{t("settings.appearance.theme")}</CardTitle>
				</CardHeader>
				<CardContent className="grid grid-cols-3 gap-3">
					{options.map((option) => {
						const Icon = option.icon;
						return (
							<button
								type="button"
								key={option.id}
								className={cn(
									"relative grid justify-items-center gap-2 rounded-xl border p-5 text-sm hover:bg-muted",
									theme === option.id && "border-primary bg-muted",
								)}
								onClick={() => setTheme(option.id)}
							>
								<Icon className="size-6" />
								{t(option.labelKey)}
								{theme === option.id ? (
									<CheckIcon className="absolute right-2 top-2 size-4" />
								) : null}
							</button>
						);
					})}
				</CardContent>
			</Card>
		</>
	);
}

function KeyboardSettings() {
	const { t } = useTranslation();
	return (
		<>
			<SettingsHeader
				title={t("settings.keyboard.title")}
				description={t("settings.keyboard.description")}
			/>
			<Card>
				<CardContent className="flex items-center justify-between">
					<div>
						<p className="font-medium">{t("settings.keyboard.toggleSidebar")}</p>
						<p className="text-sm text-muted-foreground">
							{t("settings.keyboard.toggleSidebarDescription")}
						</p>
					</div>
					<kbd className="rounded-md border bg-muted px-2 py-1 font-mono text-xs">
						⌘/Ctrl + B
					</kbd>
				</CardContent>
			</Card>
			<br />
			<Card>
				<CardContent className="flex items-center justify-between">
					<div>
						<p className="font-medium">{t("settings.keyboard.reloadPage")}</p>
						<p className="text-sm text-muted-foreground">
							{t("settings.keyboard.reloadPageDescription")}
						</p>
					</div>
					<kbd className="rounded-md border bg-muted px-2 py-1 font-mono text-xs">
						⌘/Ctrl + R
					</kbd>
				</CardContent>
			</Card>
			<br />
			<Card>
				<CardContent className="flex items-center justify-between">
					<div>
						<p className="font-medium">{t("settings.keyboard.zoomIn")}</p>
						<p className="text-sm text-muted-foreground">
							{t("settings.keyboard.zoomInDescription")}
						</p>
					</div>
					<kbd className="rounded-md border bg-muted px-2 py-1 font-mono text-xs">
						⌘/Ctrl + +
					</kbd>
				</CardContent>
			</Card>
			<br />
			<Card>
				<CardContent className="flex items-center justify-between">
					<div>
						<p className="font-medium">{t("settings.keyboard.zoomOut")}</p>
						<p className="text-sm text-muted-foreground">
							{t("settings.keyboard.zoomOutDescription")}
						</p>
					</div>
					<kbd className="rounded-md border bg-muted px-2 py-1 font-mono text-xs">
						⌘/Ctrl + -
					</kbd>
				</CardContent>
			</Card>
			<br />
			<Card>
				<CardContent className="flex items-center justify-between">
					<div>
						<p className="font-medium">{t("settings.keyboard.resetZoom")}</p>
						<p className="text-sm text-muted-foreground">
							{t("settings.keyboard.resetZoomDescription")}
						</p>
					</div>
					<kbd className="rounded-md border bg-muted px-2 py-1 font-mono text-xs">
						⌘/Ctrl + 0
					</kbd>
				</CardContent>
			</Card>
		</>
	);
}

function AccountSettings() {
	const { t } = useTranslation();
	const user = useAuthUser();
	const [currentPassword, setCurrentPassword] = useState("");
	const [newPassword, setNewPassword] = useState("");
	const [confirmPassword, setConfirmPassword] = useState("");
	const [saving, setSaving] = useState(false);
	const passwordCheck = useMemo(
		() => checkStrongPassword(newPassword),
		[newPassword],
	);
	const matches = newPassword === confirmPassword && confirmPassword.length > 0;

	async function changePassword(event: FormEvent) {
		event.preventDefault();
		if (!supabase || !user?.email || !passwordCheck.ok || !matches) return;
		setSaving(true);
		const signIn = await supabase.auth.signInWithPassword({
			email: user.email,
			password: currentPassword,
		});
		if (signIn.error) {
			setSaving(false);
			return toast.error(t("settings.account.currentPasswordIncorrect"));
		}
		const { error } = await supabase.auth.updateUser({ password: newPassword });
		setSaving(false);
		if (error) return toast.error(error.message);
		setCurrentPassword("");
		setNewPassword("");
		setConfirmPassword("");
		toast.success(t("settings.account.passwordChanged"));
	}

	return (
		<>
			<SettingsHeader
				title={t("settings.account.title")}
				description={t("settings.account.description")}
			/>
			<div className="grid gap-5">
				<PythonRuntimeSettings userId={user?.id} />
				<Card>
					<CardHeader>
						<CardTitle>{t("settings.account.email")}</CardTitle>
						<CardDescription>
							{t("settings.account.emailDescription")}
						</CardDescription>
					</CardHeader>
					<CardContent>
						<Input value={user?.email ?? t("settings.account.loading")} readOnly />
					</CardContent>
				</Card>
				<Card>
					<CardHeader>
						<CardTitle>{t("settings.account.changePassword")}</CardTitle>
						<CardDescription>
							{t("settings.account.changePasswordDescription")}
						</CardDescription>
					</CardHeader>
					<CardContent>
						<form className="grid gap-4" onSubmit={changePassword}>
							<PasswordField
								id="current-password"
								label={t("settings.account.currentPassword")}
								value={currentPassword}
								onChange={setCurrentPassword}
								autoComplete="current-password"
							/>
							<PasswordField
								id="new-password"
								label={t("settings.account.newPassword")}
								value={newPassword}
								onChange={setNewPassword}
								autoComplete="new-password"
							/>
							<PasswordField
								id="confirm-new-password"
								label={t("settings.account.confirmPassword")}
								value={confirmPassword}
								onChange={setConfirmPassword}
								autoComplete="new-password"
							/>
							<ul className="grid gap-1 text-sm text-muted-foreground">
								{passwordCheck.items.map((item) => (
									<li
										key={item.key}
										className={item.met ? "text-emerald-600" : undefined}
									>
										{item.met ? "✓" : "•"} {t(`auth.passwordRequirements.${item.key}`)}
									</li>
								))}
								<li className={matches ? "text-emerald-600" : undefined}>
									{matches ? "✓" : "•"} {t("settings.account.passwordsMatch")}
								</li>
							</ul>
							<Button
								className="w-fit"
								loading={saving}
								disabled={!currentPassword || !passwordCheck.ok || !matches}
							>
								{t("settings.account.changePasswordAction")}
							</Button>
						</form>
					</CardContent>
				</Card>
				<Card>
					<CardContent className="flex items-center justify-between gap-4">
						<div>
							<p className="font-medium">{t("settings.account.logOut")}</p>
							<p className="text-sm text-muted-foreground">
								{t("settings.account.logOutDescription")}
							</p>
						</div>
						<Button
							variant="destructive"
							onClick={() => void supabase?.auth.signOut()}
						>
							<LogOutIcon />
							{t("settings.account.logOut")}
						</Button>
					</CardContent>
				</Card>
			</div>
		</>
	);
}

function PasswordField({
	id,
	label,
	value,
	onChange,
	autoComplete,
}: {
	id: string;
	label: string;
	value: string;
	onChange: (value: string) => void;
	autoComplete: string;
}) {
	return (
		<div className="grid gap-2">
			<Label htmlFor={id}>{label}</Label>
			<Input
				id={id}
				type="password"
				value={value}
				onChange={(event) => onChange(event.target.value)}
				autoComplete={autoComplete}
				required
			/>
		</div>
	);
}

function DataStorageSettings() {
	const { t } = useTranslation();
	const user = useAuthUser();
	const [summary, setSummary] = useState<LocalDataSummary | null>(null);
	const [error, setError] = useState("");

	useEffect(() => {
		if (!user || !isTauriRuntime()) return;
		void invoke<LocalDataSummary>("local_data_summary", {
			request: { userId: user.id },
		})
			.then(setSummary)
			.catch((reason) => setError(String(reason)));
	}, [user]);

	return (
		<>
			<SettingsHeader
				title={t("settings.dataStorage.title")}
				description={t("settings.dataStorage.description")}
			/>
			<div className="grid gap-5">
				<Card>
					<CardHeader>
						<CardTitle>{t("settings.dataStorage.localStorage")}</CardTitle>
						<CardDescription>
							{summary?.dataDirectory ?? (error || t("settings.dataStorage.loading"))}
						</CardDescription>
					</CardHeader>
					<CardContent className="grid gap-3">
						<StorageRow
							label={t("settings.dataStorage.database")}
							value={formatBytes(summary?.databaseBytes)}
						/>
						<StorageRow
							label={t("settings.dataStorage.componentPackages")}
							value={formatBytes(summary?.componentBytes)}
						/>
						<StorageRow
							label={t("settings.dataStorage.marketData")}
							value={formatBytes(summary?.marketDataBytes)}
						/>
						{/* <Button
							className="mt-2 w-fit"
							variant="outline"
							loading={openingDataDirectory}
							disabled={!summary || !isTauriRuntime() || openingDataDirectory}
							onClick={() => void openDataDirectory()}
						>
							<FolderOpenIcon />
							Open Data Folder
						</Button> */}
					</CardContent>
				</Card>
				<Card>
					<CardHeader>
						<CardTitle>{t("settings.dataStorage.resetLocalData")}</CardTitle>
						<CardDescription>
							{t("settings.dataStorage.resetDescription")}
						</CardDescription>
					</CardHeader>
					<CardContent className="grid gap-3">
						<ResetAction
							kind="watchlist"
							titleKey="settings.dataStorage.resetWatchlist"
							descriptionKey="settings.dataStorage.resetWatchlistDescription"
							summary={summary}
							userId={user?.id}
						/>
						<ResetAction
							kind="components"
							titleKey="settings.dataStorage.resetComponents"
							descriptionKey="settings.dataStorage.resetComponentsDescription"
							summary={summary}
							userId={user?.id}
						/>
						<ResetAction
							kind="marketData"
							titleKey="settings.dataStorage.resetMarketData"
							descriptionKey="settings.dataStorage.resetMarketDataDescription"
							summary={summary}
							userId={user?.id}
						/>
						<ResetAction
							kind="all"
							titleKey="settings.dataStorage.resetAll"
							descriptionKey="settings.dataStorage.resetAllDescription"
							summary={summary}
							userId={user?.id}
						/>
						<ResetAction
							kind="factorResearch"
							titleKey="settings.dataStorage.resetFactorResearch"
							descriptionKey="settings.dataStorage.resetFactorResearchDescription"
							summary={summary}
						/>
					</CardContent>
				</Card>
			</div>
		</>
	);
}

function StorageRow({ label, value }: { label: string; value: string }) {
	return (
		<div className="flex items-center justify-between border-b pb-3 last:border-0 last:pb-0">
			<span>{label}</span>
			<span className="font-mono text-sm text-muted-foreground">{value}</span>
		</div>
	);
}

function ResetAction({
	kind,
	titleKey,
	descriptionKey,
	summary,
	userId,
}: {
	kind: ResetKind;
	titleKey: string;
	descriptionKey: string;
	summary: LocalDataSummary | null;
	userId?: string;
}) {
	const { t } = useTranslation();
	const dialog = useRef<HTMLDialogElement>(null);
	const [confirmation, setConfirmation] = useState("");
	const [running, setRunning] = useState(false);
	const deviceWide = kind === "factorResearch";
	const requiredConfirmation = deviceWide ? "RESET FACTOR RESEARCH" : "RESET";
	const blocked =
		deviceWide
			? false
			: kind === "components"
			? (summary?.componentBlockingRunCount ?? 0) > 0
			: kind === "marketData"
				? (summary?.marketDataBlockingRecordCount ?? 0) > 0
				: false;

	async function reset() {
		if (
			(!deviceWide && !userId) ||
			blocked ||
			((kind === "all" || deviceWide) && confirmation !== requiredConfirmation)
		)
			return;
		setRunning(true);
		try {
			if (deviceWide) {
				await invoke("factor_research_device_reset");
			} else {
				await invoke("local_data_reset", { request: { userId, kind } });
			}
			toast.success(t("settings.dataStorage.completed", { title }));
			window.setTimeout(() => window.location.reload(), 500);
		} catch (reason) {
			setRunning(false);
			toast.error(String(reason));
		}
	}
	const title = t(titleKey);
	const description = t(descriptionKey);

	return (
		<>
			<div className="flex items-center justify-between gap-5 rounded-lg border p-4">
				<div>
					<p className="font-medium">{title}</p>
					<p className="text-sm text-muted-foreground">{description}</p>
				</div>
				<Button
					variant="destructive"
					loading={running}
					disabled={!summary || (!deviceWide && !userId)}
					onClick={() => dialog.current?.showModal()}
				>
					{t("settings.dataStorage.resetButton")}
				</Button>
			</div>
			<dialog
				ref={dialog}
				onCancel={(event) => {
					if (running) event.preventDefault();
				}}
				className="m-auto w-[min(32rem,calc(100%-2rem))] rounded-xl border bg-background p-0 text-foreground shadow-2xl backdrop:bg-black/45"
			>
				<div className="grid gap-4 p-6">
					<div>
						<h3 className="text-lg font-semibold">
							{t("settings.dataStorage.confirmTitle", { title })}
						</h3>
						<p className="mt-1 text-sm text-muted-foreground">
							{t(
								deviceWide
									? "settings.dataStorage.factorResearchConfirmDescription"
									: "settings.dataStorage.confirmDescription",
							)}
						</p>
					</div>
					<ResetDetails kind={kind} summary={summary} />
					{blocked ? (
						<p className="rounded-lg bg-destructive/10 p-3 text-sm text-destructive">
							{t("settings.dataStorage.blocked")}
						</p>
					) : null}
					{kind === "all" || deviceWide ? (
						<div className="grid gap-2">
							<Label htmlFor="reset-confirmation">
								{t(
									deviceWide
										? "settings.dataStorage.typeFactorResearchReset"
										: "settings.dataStorage.typeReset",
								)}
							</Label>
							<Input
								id="reset-confirmation"
								value={confirmation}
								onChange={(event) => setConfirmation(event.target.value)}
								autoComplete="off"
							/>
						</div>
					) : null}
					<div className="flex justify-end gap-2">
						<Button
							variant="outline"
							disabled={running}
							onClick={() => dialog.current?.close()}
						>
							{t("settings.dataStorage.cancel")}
						</Button>
						<Button
							variant="destructive"
							loading={running}
							disabled={
								blocked ||
								((kind === "all" || deviceWide) &&
									confirmation !== requiredConfirmation)
							}
							onClick={() => void reset()}
						>
							{title}
						</Button>
					</div>
				</div>
			</dialog>
		</>
	);
}

function ResetDetails({
	kind,
	summary,
}: {
	kind: ResetKind;
	summary: LocalDataSummary | null;
}) {
	const { t } = useTranslation();
	if (!summary) return null;
	const rows: ReactNode[] = [];
	if (kind === "watchlist" || kind === "all")
		rows.push(
			<li key="watchlist">
				{t("settings.dataStorage.watchlistItems", {
					count: summary.watchlistCount,
				})}
			</li>,
		);
	if (kind === "components" || kind === "all")
		rows.push(
			<li key="components">
				{t("settings.dataStorage.componentPackagesCount", {
					count: summary.componentCount,
				})}
			</li>,
		);
	if (kind === "marketData" || kind === "all")
		rows.push(
			<li key="snapshots">
				{t("settings.dataStorage.marketDataSnapshotsCount", {
					count: summary.snapshotCount,
				})}
			</li>,
		);
	if (kind === "all")
		rows.push(
			<li key="runs">
				{t("settings.dataStorage.backtestRuns", { count: summary.runCount })}
			</li>,
			<li key="protocols">
				{t("settings.dataStorage.validationProtocols", {
					count: summary.protocolCount,
				})}
			</li>,
			<li key="reports">
				{t("settings.dataStorage.validationReports", {
					count: summary.reportCount,
				})}
			</li>,
			<li key="attempts">
				{t("settings.dataStorage.generationAttempts", {
					count: summary.generationAttemptCount,
				})}
			</li>,
			<li key="artifacts">
				{t("settings.dataStorage.modelArtifacts", {
					count: summary.modelArtifactCount,
				})}
			</li>,
			<li key="datasets">
				{t("settings.dataStorage.signalDatasets", {
					count: summary.signalDatasetCount,
				})}
			</li>,
		);
	if (kind === "factorResearch")
		rows.push(<li key="factorResearch">{t("settings.dataStorage.factorResearchData")}</li>);
	return (
		<div className="rounded-lg border bg-muted/30 p-4 text-sm">
			<p className="mb-2 font-medium">{t("settings.dataStorage.dataToReset")}</p>
			<ul className="list-inside list-disc space-y-1 text-muted-foreground">
				{rows}
			</ul>
			<p className="mt-3">
				{t(
					kind === "factorResearch"
						? "settings.dataStorage.factorResearchPreserved"
						: "settings.dataStorage.preserved",
				)}
			</p>
		</div>
	);
}

function formatBytes(value?: number) {
	if (value === undefined) return "—";
	if (value < 1024) return `${formatNumber(value)} B`;
	if (value < 1024 ** 2)
		return `${formatNumber(value / 1024, { maximumFractionDigits: 1 })} KB`;
	if (value < 1024 ** 3)
		return `${formatNumber(value / 1024 ** 2, { maximumFractionDigits: 1 })} MB`;
	return `${formatNumber(value / 1024 ** 3, { maximumFractionDigits: 1 })} GB`;
}
