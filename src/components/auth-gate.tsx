import type { Session } from "@supabase/supabase-js";
import { KeyRound, Mail, ShieldCheck } from "lucide-react";
import {
	type FormEvent,
	type ReactNode,
	useEffect,
	useMemo,
	useState,
} from "react";
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
import { checkStrongPassword } from "@/lib/password";
import { isSupabaseConfigured, supabase } from "@/lib/supabase";
import { NavTitlebarTransparent } from "./nav-titlebar-transparent";

type AuthStep = "email" | "password" | "otp";

export function AuthGate({ children }: { children: ReactNode }) {
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
					title="Supabase is not configured"
					description="Set VITE_SUPABASE_URL and VITE_SUPABASE_PUBLISHABLE_KEY, then restart the app."
				/>
			</>
		);
	}

	if (loadingSession) {
		return (
			<>
				<NavTitlebarTransparent />
				<AuthShell
					title="Checking session"
					description="Loading your secure workspace."
				/>
			</>
		);
	}

	if (!session) {
		return <EmailOtpForm onSession={setSession} />;
	}

	if (!session.user.user_metadata.password_set_at) {
		return <PasswordSetupForm session={session} onSession={setSession} />;
	}

	return children;
}

function EmailOtpForm({
	onSession,
}: {
	onSession: (session: Session) => void;
}) {
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
		setMessage("Check your email for the 8-digit code.");
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
			setError("Password sign-in succeeded, but no session was returned.");
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
			setError("The code was accepted, but no session was returned.");
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
						? "Sign in with password"
						: step === "otp"
							? "Enter verification code"
							: "Sign in"
				}
				description={
					step === "email"
						? "Please input your email address. For new accounts, an email with a OTP code will be sent to your email address. Existing accounts continue with password."
						: email
				}
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
						<Label htmlFor="email">Email</Label>
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
							<Label htmlFor="signin-password">Password</Label>
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
							<Label htmlFor="otp">Code</Label>
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
							? "Continue"
							: step === "password"
								? "Sign in"
								: "Verify code"}
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
							Use a different email
						</Button>
					)}
					{step === "password" && (
						<Button type="button" variant="ghost" onClick={() => sendOtp()}>
							Sign in with email code instead
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
	const [password, setPassword] = useState("");
	const [confirmPassword, setConfirmPassword] = useState("");
	const [error, setError] = useState("");
	const [loading, setLoading] = useState(false);
	const passwordCheck = useMemo(
		() => checkStrongPassword(password),
		[password],
	);
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
				title="Create a strong password"
				description={
					session.user.email ?? "Secure your account before continuing."
				}
			>
				<form className="grid gap-4" onSubmit={createPassword}>
					<div className="grid gap-2">
						<Label htmlFor="password">Password</Label>
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
						<Label htmlFor="confirm-password">Confirm password</Label>
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
								className={
									item.met ? "text-emerald-600" : "text-muted-foreground"
								}
								key={item.label}
							>
								{item.met ? "✓" : "•"} {item.label}
							</li>
						))}
						<li
							className={
								passwordsMatch ? "text-emerald-600" : "text-muted-foreground"
							}
						>
							{passwordsMatch ? "✓" : "•"} Passwords match
						</li>
					</ul>
					<AuthNotice error={error} />
					<Button
						type="submit"
						loading={loading}
						disabled={!passwordCheck.ok || !passwordsMatch}
					>
						Create password
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
	children,
}: {
	title: string;
	description: string;
	children?: ReactNode;
}) {
	return (
		<main className="flex min-h-svh items-center justify-center bg-background p-6">
			<Card className="w-full max-w-sm rounded-lg">
				<CardHeader>
					<div className="mb-2 flex size-9 items-center justify-center rounded-lg bg-primary text-primary-foreground">
						<AuthIcon title={title} />
					</div>
					<CardTitle>{title}</CardTitle>
					<CardDescription>{description}</CardDescription>
				</CardHeader>
				{children && <CardContent>{children}</CardContent>}
			</Card>
		</main>
	);
}

function AuthIcon({ title }: { title: string }) {
	if (title.includes("password")) return <KeyRound className="size-4" />;
	if (title.includes("configured")) return <ShieldCheck className="size-4" />;
	return <Mail className="size-4" />;
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
