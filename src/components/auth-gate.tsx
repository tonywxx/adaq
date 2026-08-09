import { Button } from "@/components/ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { MarketSessionProvider } from "@/lib/market-session";
import { checkStrongPassword } from "@/lib/password";
import { isSupabaseConfigured, supabase } from "@/lib/supabase";
import type { Session } from "@supabase/supabase-js";
import { KeyRound, LoaderCircleIcon, Mail, ShieldCheck } from "lucide-react";
import {
	type FormEvent,
	type ReactNode,
	useEffect,
	useMemo,
	useState,
} from "react";
import { useTranslation } from "react-i18next";
import { NavTitlebarTransparent } from "./nav-titlebar-transparent";
import type { TFunction } from "i18next";

type AuthStep = "email" | "password" | "otp";

export function AuthGate({ children }: { children: ReactNode }) {
	const { t } = useTranslation();
	const [session, setSession] = useState<Session | null>(null);
	const [loadingSession, setLoadingSession] = useState(true);

	useEffect(() => {
		if (!supabase) {
			setLoadingSession(false);
			return;
		}

		supabase.auth.getSession().then(({ data }) => {
			setSession(data.session);
			setLoadingSession(false);
		});

		const {
			data: { subscription },
		} = supabase.auth.onAuthStateChange((_event, nextSession) => {
			setSession(nextSession);
		});

		return () => subscription.unsubscribe();
	}, []);

	if (!isSupabaseConfigured || !supabase) {
		return (
			<>
				<NavTitlebarTransparent />
				<AuthShell
					title={t("auth.supabaseNotConfigured")}
					description={t("auth.supabaseNotConfiguredDescription")}
					icon="configured"
				/>
			</>
		);
	}

	if (loadingSession) {
		return (
			<>
				<NavTitlebarTransparent />
				<main
					className="grid min-h-svh place-content-center justify-items-center gap-3 bg-background"
					role="status"
					aria-label={t("auth.initializingAria")}
				>
					<LoaderCircleIcon
						className="size-7 animate-spin text-primary"
						aria-hidden="true"
					/>
					<p className="font-semibold">AdaQ</p>
					<p className="text-sm text-muted-foreground">
						{t("auth.initializingWorkspace")}
					</p>
				</main>
			</>
		);
	}

	if (!session) {
		return <EmailOtpForm onSession={setSession} />;
	}

	if (!session.user.user_metadata.password_set_at) {
		return <PasswordSetupForm session={session} onSession={setSession} />;
	}

	return (
		<MarketSessionProvider userId={session.user.id}>
			{children}
		</MarketSessionProvider>
	);
}

function EmailOtpForm({
	onSession,
}: {
	onSession: (session: Session) => void;
}) {
	const { t } = useTranslation();
	const [step, setStep] = useState<AuthStep>("email");
	const [email, setEmail] = useState("");
	const [password, setPassword] = useState("");
	const [token, setToken] = useState("");
	const [message, setMessage] = useState("");
	const [error, setError] = useState("");
	const [loading, setLoading] = useState(false);

	async function checkEmail(event: FormEvent<HTMLFormElement>) {
		event.preventDefault();
		if (!supabase) return;

		setLoading(true);
		setError("");
		setMessage("");

		const nextEmail = email.trim().toLowerCase();
		const account = await getAccountStatus(nextEmail);
		if (account.passwordSet) {
			setEmail(nextEmail);
			setLoading(false);
			setStep("password");
			return;
		}

		await sendOtp(nextEmail);
	}

	async function sendOtp(nextEmail = email.trim().toLowerCase()) {
		if (!supabase) return;

		setLoading(true);
		setError("");
		setMessage("");

		const { error: authError } = await supabase.auth.signInWithOtp({
			email: nextEmail,
			options: { shouldCreateUser: true },
		});

		setLoading(false);
		if (authError) {
			setError(authError.message);
			return;
		}

		setEmail(nextEmail);
		setStep("otp");
		setMessage(t("auth.checkEmail"));
	}

	async function signInWithPassword(event: FormEvent<HTMLFormElement>) {
		event.preventDefault();
		if (!supabase) return;

		setLoading(true);
		setError("");

		const { data, error: authError } = await supabase.auth.signInWithPassword({
			email,
			password,
		});

		setLoading(false);
		if (authError) {
			setError(authError.message);
			return;
		}
		if (!data.session) {
			setError(t("auth.passwordSignInNoSession"));
			return;
		}

		onSession(data.session);
	}

	async function verifyOtp(event: FormEvent<HTMLFormElement>) {
		event.preventDefault();
		if (!supabase) return;

		setLoading(true);
		setError("");

		const { data, error: authError } = await verifyEmailOtp(
			email,
			token.replace(/[\s-]/g, ""),
		);

		setLoading(false);
		if (authError) {
			setError(authError.message);
			return;
		}
		if (!data.session) {
			setError(t("auth.codeAcceptedNoSession"));
			return;
		}

		onSession(data.session);
	}

	return (
		<>
			<NavTitlebarTransparent />
			<AuthShell
				title={
					step === "password"
						? t("auth.signInWithPassword")
						: step === "otp"
							? t("auth.enterVerificationCode")
							: t("auth.signIn")
				}
				description={step === "email" ? t("auth.emailDescription") : email}
				icon={step === "password" ? "password" : "mail"}
			>
				<form
					className="grid gap-4"
					onSubmit={
						step === "email"
							? checkEmail
							: step === "password"
								? signInWithPassword
								: verifyOtp
					}
				>
					<div className="grid gap-2">
						<Label htmlFor="email">{t("auth.email")}</Label>
						<Input
							id="email"
							type="email"
							autoComplete="email"
							value={email}
							onChange={(event) => setEmail(event.target.value)}
							disabled={loading || step === "otp"}
							required
						/>
					</div>
					{step === "password" && (
						<div className="grid gap-2">
							<Label htmlFor="signin-password">{t("auth.password")}</Label>
							<Input
								id="signin-password"
								type="password"
								autoComplete="current-password"
								value={password}
								onChange={(event) => setPassword(event.target.value)}
								autoFocus
								required
							/>
						</div>
					)}
					{step === "otp" && (
						<div className="grid gap-2">
							<Label htmlFor="otp">{t("auth.code")}</Label>
							<Input
								id="otp"
								inputMode="numeric"
								autoComplete="one-time-code"
								value={token}
								onChange={(event) => setToken(event.target.value.trim())}
								minLength={8}
								maxLength={8}
								required
							/>
						</div>
					)}
					<AuthNotice message={message} error={error} />
					<Button type="submit" loading={loading}>
						{step === "email"
							? t("auth.continue")
							: step === "password"
								? t("auth.signIn")
								: t("auth.verifyCode")}
					</Button>
					{(step === "password" || step === "otp") && (
						<Button
							type="button"
							variant="ghost"
							onClick={() => {
								setPassword("");
								setToken("");
								setStep("email");
							}}
						>
							{t("auth.differentEmail")}
						</Button>
					)}
					{step === "password" && (
						<Button type="button" variant="ghost" onClick={() => sendOtp()}>
							{t("auth.emailCodeInstead")}
						</Button>
					)}
				</form>
			</AuthShell>
		</>
	);
}

async function getAccountStatus(email: string) {
	if (!supabase) return { exists: false, passwordSet: false };

	const { data, error } = await supabase.rpc("get_auth_account_status", {
		account_email: email,
	});

	if (error) return { exists: false, passwordSet: false };
	const status = Array.isArray(data) ? data[0] : data;

	return {
		exists: Boolean(status?.account_exists),
		passwordSet: Boolean(status?.password_set),
	};
}

async function verifyEmailOtp(email: string, token: string) {
	if (!supabase) {
		return {
			data: { session: null },
			error: new Error("Supabase is not configured."),
		};
	}

	for (const type of ["email", "magiclink", "signup"] as const) {
		const result = await supabase.auth.verifyOtp({
			email: email.trim().toLowerCase(),
			token,
			type,
		});
		if (!result.error || !isInvalidTokenError(result.error.message))
			return result;
	}

	return supabase.auth.verifyOtp({
		email: email.trim().toLowerCase(),
		token,
		type: "email",
	});
}

function isInvalidTokenError(message: string) {
	return message.toLowerCase().includes("token has expired or is invalid");
}

function PasswordSetupForm({
	session,
	onSession,
}: {
	session: Session;
	onSession: (session: Session) => void;
}) {
	const { t } = useTranslation();
	const [password, setPassword] = useState("");
	const [confirmPassword, setConfirmPassword] = useState("");
	const [error, setError] = useState("");
	const [loading, setLoading] = useState(false);
	const passwordCheck = useMemo(() => checkStrongPassword(password), [password]);
	const passwordsMatch =
		password === confirmPassword && confirmPassword.length > 0;

	async function createPassword(event: FormEvent<HTMLFormElement>) {
		event.preventDefault();
		if (!supabase || !passwordCheck.ok || !passwordsMatch) return;

		setLoading(true);
		setError("");

		const { data, error: authError } = await supabase.auth.updateUser({
			password,
			data: { password_set_at: new Date().toISOString() },
		});

		setLoading(false);
		if (authError) {
			setError(authError.message);
			return;
		}

		await markPasswordSet(data.user.id, data.user.email);
		onSession({
			...session,
			user: data.user,
		});
	}

	return (
		<>
			<NavTitlebarTransparent />
			<AuthShell
				title={t("auth.createStrongPassword")}
				description={session.user.email ?? t("auth.secureAccount")}
				icon="password"
			>
				<form className="grid gap-4" onSubmit={createPassword}>
					<div className="grid gap-2">
						<Label htmlFor="password">{t("auth.password")}</Label>
						<Input
							id="password"
							type="password"
							autoComplete="new-password"
							value={password}
							onChange={(event) => setPassword(event.target.value)}
							required
						/>
					</div>
					<div className="grid gap-2">
						<Label htmlFor="confirm-password">{t("auth.confirmPassword")}</Label>
						<Input
							id="confirm-password"
							type="password"
							autoComplete="new-password"
							value={confirmPassword}
							onChange={(event) => setConfirmPassword(event.target.value)}
							required
						/>
					</div>
					<ul className="grid gap-1 text-sm">
						{passwordCheck.items.map((item) => (
							<li
								className={item.met ? "text-emerald-600" : "text-muted-foreground"}
								key={item.key}
							>
								{item.met ? "✓" : "•"} {passwordRequirementLabel(t, item.key)}
							</li>
						))}
						<li
							className={passwordsMatch ? "text-emerald-600" : "text-muted-foreground"}
						>
							{passwordsMatch ? "✓" : "•"} {t("auth.passwordsMatch")}
						</li>
					</ul>
					<AuthNotice error={error} />
					<Button
						type="submit"
						loading={loading}
						disabled={!passwordCheck.ok || !passwordsMatch}
					>
						{t("auth.createPassword")}
					</Button>
				</form>
			</AuthShell>
		</>
	);
}

async function markPasswordSet(userId: string, email?: string) {
	if (!supabase || !email) return;

	await supabase.from("profiles").upsert({
		id: userId,
		email: email.trim().toLowerCase(),
		password_set_at: new Date().toISOString(),
	});
}

function AuthShell({
	title,
	description,
	icon,
	children,
}: {
	title: string;
	description: string;
	icon: "configured" | "mail" | "password";
	children?: ReactNode;
}) {
	return (
		<main className="flex min-h-svh items-center justify-center bg-background p-6">
			<Card className="w-full max-w-sm rounded-lg">
				<CardHeader>
					<div className="mb-2 flex size-9 items-center justify-center rounded-lg bg-primary text-primary-foreground">
						<AuthIcon kind={icon} />
					</div>
					<CardTitle>{title}</CardTitle>
					<CardDescription>{description}</CardDescription>
				</CardHeader>
				{children && <CardContent>{children}</CardContent>}
			</Card>
		</main>
	);
}

function AuthIcon({ kind }: { kind: "configured" | "mail" | "password" }) {
	if (kind === "password") return <KeyRound className="size-4" />;
	if (kind === "configured") return <ShieldCheck className="size-4" />;
	return <Mail className="size-4" />;
}

function passwordRequirementLabel(t: TFunction, key: string) {
	return t(`auth.passwordRequirements.${key}`);
}

function AuthNotice({ message, error }: { message?: string; error?: string }) {
	if (!message && !error) return null;

	return (
		<p
			className={
				error ? "text-sm text-destructive" : "text-sm text-muted-foreground"
			}
		>
			{error || message}
		</p>
	);
}
