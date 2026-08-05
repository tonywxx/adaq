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
import { checkStrongPassword } from "@/lib/password";
import { supabase } from "@/lib/supabase";
import { cn } from "@/lib/utils";
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
	RefreshCwIcon,
	Settings2Icon,
	ShieldIcon,
	SunIcon,
	UserRoundIcon,
} from "lucide-react";
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
	{ id: "general", label: "General", icon: Settings2Icon },
	{ id: "profile", label: "Profile", icon: UserRoundIcon },
	{ id: "appearance", label: "Appearance", icon: PaletteIcon },
	{ id: "keyboard-shortcuts", label: "Keyboard Shortcuts", icon: KeyboardIcon },
	{ id: "account", label: "Account", icon: ShieldIcon },
	{ id: "data-storage", label: "Data & Storage", icon: DatabaseIcon },
] as const;

type Section = (typeof sections)[number]["id"];
type ResetKind = "watchlist" | "components" | "marketData" | "all";

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
					{section === "data-storage" && <DataStorageSettings />}
				</div>
			</main>
		</div>
	);
}

function SettingsNavigation({ section }: { section: Section }) {
	const router = useRouter();
	const backToApp = () => {
		router.history.push(sessionStorage.getItem(LAST_APP_PATH_KEY) || "/");
	};

	return (
		<aside className="sticky top-0 h-[calc(100svh-3rem)] w-72 shrink-0 border-r bg-muted/25 px-3 py-5">
			<Button className="mb-5 justify-start" variant="ghost" onClick={backToApp}>
				<ArrowLeftIcon />
				Back to App
			</Button>
			<nav aria-label="Settings">
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
									<span className="truncate">{item.label}</span>
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
	const [autoDownload, setAutoDownload] = useState(autoDownloadUpdates);
	const [version, setVersion] = useState("Loading…");

	useEffect(() => {
		if (!isTauriRuntime()) {
			setVersion("Development");
			return;
		}
		void getVersion()
			.then(setVersion)
			.catch(() => setVersion("Unavailable"));
	}, []);

	return (
		<>
			<SettingsHeader
				title="General"
				description="Application updates and version information."
			/>
			<Card>
				<CardHeader>
					<CardTitle>Software Updates</CardTitle>
					<CardDescription>
						Keep ADAQ current with signed application releases.
					</CardDescription>
				</CardHeader>
				<CardContent className="grid gap-4">
					<div className="flex items-center justify-between gap-6">
						<span>
							<span className="block font-medium">Automatically download updates</span>
							<span className="block text-sm text-muted-foreground">
								Check at startup and download an available update.
							</span>
						</span>
						<Checkbox
							aria-label="Automatically download updates"
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
							<p className="font-medium">Version {version}</p>
							<p className="text-sm text-muted-foreground">
								Installed application version.
							</p>
						</div>
						<Button
							variant="outline"
							disabled={!isTauriRuntime()}
							onClick={() => void emit("adaq-check-for-updates")}
						>
							<RefreshCwIcon />
							Check for updates
						</Button>
					</div>
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
		toast.success("Profile saved.");
	}

	const avatar = user?.user_metadata.avatar_url ?? user?.user_metadata.picture;
	const initials = (displayName || user?.email || "A").slice(0, 2).toUpperCase();

	return (
		<>
			<SettingsHeader
				title="Profile"
				description="Your presentation identity in ADAQ."
			/>
			<Card>
				<CardContent>
					<form className="grid gap-5" onSubmit={saveProfile}>
						<div className="flex items-center gap-4">
							<Avatar size="lg">
								<AvatarImage src={avatar} alt={displayName || "Profile"} />
								<AvatarFallback>{initials}</AvatarFallback>
							</Avatar>
							<p className="text-sm text-muted-foreground">
								Your connected account avatar is used when available.
							</p>
						</div>
						<div className="grid gap-2">
							<Label htmlFor="display-name">Display name</Label>
							<Input
								id="display-name"
								value={displayName}
								onChange={(event) => setDisplayName(event.target.value)}
								maxLength={80}
								required
							/>
						</div>
						<Button className="w-fit" loading={saving} disabled={!displayName.trim()}>
							Save profile
						</Button>
					</form>
				</CardContent>
			</Card>
		</>
	);
}

function AppearanceSettings() {
	const { setTheme, theme = "system" } = useTheme();
	const options = [
		{ id: "system", label: "System", icon: LaptopIcon },
		{ id: "light", label: "Light", icon: SunIcon },
		{ id: "dark", label: "Dark", icon: MoonIcon },
	];
	return (
		<>
			<SettingsHeader
				title="Appearance"
				description="Choose how ADAQ looks on this device."
			/>
			<Card>
				<CardHeader>
					<CardTitle>Theme</CardTitle>
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
								{option.label}
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
	return (
		<>
			<SettingsHeader
				title="Keyboard Shortcuts"
				description="Available application shortcuts."
			/>
			<Card>
				<CardContent className="flex items-center justify-between">
					<div>
						<p className="font-medium">Toggle Sidebar</p>
						<p className="text-sm text-muted-foreground">
							Show or hide the workspace sidebar.
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
						<p className="font-medium">Reload Page</p>
						<p className="text-sm text-muted-foreground">
							Reload the current application window.
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
						<p className="font-medium">Zoom In</p>
						<p className="text-sm text-muted-foreground">
							Increase the interface zoom level.
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
						<p className="font-medium">Zoom Out</p>
						<p className="text-sm text-muted-foreground">
							Decrease the interface zoom level.
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
						<p className="font-medium">Reset Zoom</p>
						<p className="text-sm text-muted-foreground">
							Restore the default interface zoom level.
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
			return toast.error("Current password is incorrect.");
		}
		const { error } = await supabase.auth.updateUser({ password: newPassword });
		setSaving(false);
		if (error) return toast.error(error.message);
		setCurrentPassword("");
		setNewPassword("");
		setConfirmPassword("");
		toast.success("Password changed.");
	}

	return (
		<>
			<SettingsHeader
				title="Account"
				description="Authentication details and session actions."
			/>
			<div className="grid gap-5">
				<Card>
					<CardHeader>
						<CardTitle>Email</CardTitle>
						<CardDescription>
							Your account email cannot be changed here.
						</CardDescription>
					</CardHeader>
					<CardContent>
						<Input value={user?.email ?? "Loading…"} readOnly />
					</CardContent>
				</Card>
				<Card>
					<CardHeader>
						<CardTitle>Change password</CardTitle>
						<CardDescription>
							Confirm your current password before choosing a new one.
						</CardDescription>
					</CardHeader>
					<CardContent>
						<form className="grid gap-4" onSubmit={changePassword}>
							<PasswordField
								id="current-password"
								label="Current password"
								value={currentPassword}
								onChange={setCurrentPassword}
								autoComplete="current-password"
							/>
							<PasswordField
								id="new-password"
								label="New password"
								value={newPassword}
								onChange={setNewPassword}
								autoComplete="new-password"
							/>
							<PasswordField
								id="confirm-new-password"
								label="Confirm new password"
								value={confirmPassword}
								onChange={setConfirmPassword}
								autoComplete="new-password"
							/>
							<ul className="grid gap-1 text-sm text-muted-foreground">
								{passwordCheck.items.map((item) => (
									<li
										key={item.label}
										className={item.met ? "text-emerald-600" : undefined}
									>
										{item.met ? "✓" : "•"} {item.label}
									</li>
								))}
								<li className={matches ? "text-emerald-600" : undefined}>
									{matches ? "✓" : "•"} Passwords match
								</li>
							</ul>
							<Button
								className="w-fit"
								loading={saving}
								disabled={!currentPassword || !passwordCheck.ok || !matches}
							>
								Change password
							</Button>
						</form>
					</CardContent>
				</Card>
				<Card>
					<CardContent className="flex items-center justify-between gap-4">
						<div>
							<p className="font-medium">Log out</p>
							<p className="text-sm text-muted-foreground">
								End the current ADAQ session.
							</p>
						</div>
						<Button
							variant="destructive"
							onClick={() => void supabase?.auth.signOut()}
						>
							<LogOutIcon />
							Log out
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
				title="Data & Storage"
				description="Inspect and reset local research data for this User."
			/>
			<div className="grid gap-5">
				<Card>
					<CardHeader>
						<CardTitle>Local storage</CardTitle>
						<CardDescription>
							{summary?.dataDirectory ?? (error || "Loading local data…")}
						</CardDescription>
					</CardHeader>
					<CardContent className="grid gap-3">
						<StorageRow
							label="Database"
							value={formatBytes(summary?.databaseBytes)}
						/>
						<StorageRow
							label="Component Packages"
							value={formatBytes(summary?.componentBytes)}
						/>
						<StorageRow
							label="Market Data"
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
						<CardTitle>Reset local data</CardTitle>
						<CardDescription>
							These actions cannot be undone. Account and interface preferences are
							preserved.
						</CardDescription>
					</CardHeader>
					<CardContent className="grid gap-3">
						<ResetAction
							kind="watchlist"
							title="Reset Watchlist"
							description="Restore BTC-USDT, ETH-USDT, and SOL-USDT."
							summary={summary}
							userId={user?.id}
						/>
						<ResetAction
							kind="components"
							title="Reset Component Packages"
							description="Remove local Component Package access and unreferenced files."
							summary={summary}
							userId={user?.id}
						/>
						<ResetAction
							kind="marketData"
							title="Reset Market Data"
							description="Remove local Market Data Snapshot access and unreferenced Parquet files."
							summary={summary}
							userId={user?.id}
						/>
						<ResetAction
							kind="all"
							title="Reset All Local Research Data"
							description="Remove this User's Watchlist, Components, Model Artifacts, Market Data, Generation Attempts, Signal Datasets, Runs, Protocols, and Reports."
							summary={summary}
							userId={user?.id}
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
	title,
	description,
	summary,
	userId,
}: {
	kind: ResetKind;
	title: string;
	description: string;
	summary: LocalDataSummary | null;
	userId?: string;
}) {
	const dialog = useRef<HTMLDialogElement>(null);
	const [confirmation, setConfirmation] = useState("");
	const [running, setRunning] = useState(false);
	const blocked =
		kind === "components"
			? (summary?.componentBlockingRunCount ?? 0) > 0
			: kind === "marketData"
				? (summary?.marketDataBlockingRecordCount ?? 0) > 0
				: false;

	async function reset() {
		if (!userId || blocked || (kind === "all" && confirmation !== "RESET"))
			return;
		setRunning(true);
		try {
			await invoke("local_data_reset", { request: { userId, kind } });
			toast.success(`${title} completed.`);
			window.setTimeout(() => window.location.reload(), 500);
		} catch (reason) {
			setRunning(false);
			toast.error(String(reason));
		}
	}

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
					disabled={!summary || !userId}
					onClick={() => dialog.current?.showModal()}
				>
					Reset
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
						<h3 className="text-lg font-semibold">Confirm {title}</h3>
						<p className="mt-1 text-sm text-muted-foreground">
							This action affects only the current User and cannot be undone.
						</p>
					</div>
					<ResetDetails kind={kind} summary={summary} />
					{blocked ? (
						<p className="rounded-lg bg-destructive/10 p-3 text-sm text-destructive">
							This reset is blocked by immutable research records. Use Reset All to
							remove the complete dependency chain.
						</p>
					) : null}
					{kind === "all" ? (
						<div className="grid gap-2">
							<Label htmlFor="reset-confirmation">Type RESET to continue</Label>
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
							Cancel
						</Button>
						<Button
							variant="destructive"
							loading={running}
							disabled={blocked || (kind === "all" && confirmation !== "RESET")}
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
	if (!summary) return null;
	const rows: ReactNode[] = [];
	if (kind === "watchlist" || kind === "all")
		rows.push(<li key="watchlist">Watchlist items: {summary.watchlistCount}</li>);
	if (kind === "components" || kind === "all")
		rows.push(
			<li key="components">Component Packages: {summary.componentCount}</li>,
		);
	if (kind === "marketData" || kind === "all")
		rows.push(
			<li key="snapshots">Market Data Snapshots: {summary.snapshotCount}</li>,
		);
	if (kind === "all")
		rows.push(
			<li key="runs">Backtest Runs: {summary.runCount}</li>,
			<li key="protocols">Validation Protocols: {summary.protocolCount}</li>,
			<li key="reports">Validation Reports: {summary.reportCount}</li>,
			<li key="attempts">
				Generation Attempts: {summary.generationAttemptCount}
			</li>,
			<li key="artifacts">
				Model Artifact registrations: {summary.modelArtifactCount}
			</li>,
			<li key="datasets">
				Forecast Signal Datasets: {summary.signalDatasetCount}
			</li>,
		);
	return (
		<div className="rounded-lg border bg-muted/30 p-4 text-sm">
			<p className="mb-2 font-medium">Data to reset</p>
			<ul className="list-inside list-disc space-y-1 text-muted-foreground">
				{rows}
			</ul>
			<p className="mt-3">
				Preserved: login, Account, Profile, theme, and update preference.
			</p>
		</div>
	);
}

function formatBytes(value?: number) {
	if (value === undefined) return "—";
	if (value < 1024) return `${value} B`;
	if (value < 1024 ** 2) return `${(value / 1024).toFixed(1)} KB`;
	if (value < 1024 ** 3) return `${(value / 1024 ** 2).toFixed(1)} MB`;
	return `${(value / 1024 ** 3).toFixed(1)} GB`;
}
