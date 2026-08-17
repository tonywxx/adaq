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
import { cn } from "@/lib/utils";
import { ShieldCheckIcon, ShieldXIcon } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import type {
	ConnectionsAdapter,
	ProfileView,
	Provider,
	ProviderCredentials,
} from "./connections-adapter";

type ConnectionError = { code: string; message: string };

export function ConnectionCard({
	provider,
	profile,
	disabled,
	userId,
	adapter,
	onChanged,
}: {
	provider: Provider;
	profile: ProfileView | null;
	disabled: boolean;
	userId?: string;
	adapter: ConnectionsAdapter;
	onChanged: () => void;
}) {
	const { t } = useTranslation();
	const [keyId, setKeyId] = useState("");
	const [secretKey, setSecretKey] = useState("");
	const [passphrase, setPassphrase] = useState("");
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<ConnectionError | null>(null);

	const isAlpaca = provider === "alpaca_paper";
	const titleKey = isAlpaca
		? "settings.connections.alpacaPaper"
		: "settings.connections.okxDemo";

	const localizedError = (connectionError: ConnectionError) => {
		const key = `settings.connections.errors.${connectionError.code}`;
		const localized = t(key);
		return localized === key ? connectionError.message : localized;
	};

	const run = async (action: () => Promise<unknown>, okKey: string) => {
		if (!userId) return;
		setBusy(true);
		setError(null);
		try {
			await action();
			toast.success(t(okKey));
			setKeyId("");
			setSecretKey("");
			setPassphrase("");
			onChanged();
		} catch (reason) {
			const parsed = parseConnectionError(reason);
			setError(parsed);
			toast.error(localizedError(parsed));
		} finally {
			setBusy(false);
		}
	};

	const save = () =>
		run(() => {
			if (!userId) return Promise.resolve();
			const credentials: ProviderCredentials = isAlpaca
				? { provider: "alpaca_paper", keyId, secretKey }
				: { provider: "okx_demo", apiKey: keyId, secretKey, passphrase };
			return adapter.saveProfile(userId, credentials);
		}, "settings.connections.saved");

	const test = () =>
		run(() => {
			if (!userId || !profile) return Promise.resolve();
			return adapter.testProfile(userId, profile.profileId);
		}, "settings.connections.tested");

	const remove = () =>
		run(() => {
			if (!userId || !profile) return Promise.resolve();
			return adapter.deleteProfile(userId, profile.profileId);
		}, "settings.connections.deleted");

	const canSubmit =
		!busy &&
		keyId.trim().length > 0 &&
		secretKey.trim().length > 0 &&
		(isAlpaca || passphrase.trim().length > 0);

	return (
		<Card>
			<CardHeader>
				<div className="flex items-center justify-between gap-4">
					<div>
						<CardTitle>{t(`${titleKey}.title`)}</CardTitle>
						<CardDescription>{t(`${titleKey}.description`)}</CardDescription>
					</div>
					{profile ? <StatusBadge status={profile.status} /> : null}
				</div>
			</CardHeader>
			<CardContent className="grid gap-4">
				{profile ? <ProfileSummary profile={profile} /> : null}
				{profile ? (
					<p className="text-xs text-muted-foreground">
						{t("settings.connections.savedSecretHint")}
					</p>
				) : null}
				<form
					className="grid gap-3"
					onSubmit={(event) => {
						event.preventDefault();
						void save();
					}}
				>
					<div className="grid gap-2">
						<Label htmlFor={`${provider}-key-id`}>
							{t(`${titleKey}.${isAlpaca ? "keyId" : "apiKey"}`)}
						</Label>
						<Input
							id={`${provider}-key-id`}
							value={keyId}
							onChange={(event) => setKeyId(event.target.value)}
							placeholder={
								profile ? t("settings.connections.keyPlaceholder") : undefined
							}
							autoComplete="off"
						/>
					</div>
					<div className="grid gap-2">
						<Label htmlFor={`${provider}-secret-key`}>
							{t(`${titleKey}.secretKey`)}
						</Label>
						<Input
							id={`${provider}-secret-key`}
							type="password"
							value={secretKey}
							onChange={(event) => setSecretKey(event.target.value)}
							placeholder={
								profile ? t("settings.connections.secretPlaceholder") : undefined
							}
							autoComplete="new-password"
						/>
					</div>
					{!isAlpaca ? (
						<div className="grid gap-2">
							<Label htmlFor={`${provider}-passphrase`}>
								{t("settings.connections.okxDemo.passphrase")}
							</Label>
							<Input
								id={`${provider}-passphrase`}
								type="password"
								value={passphrase}
								onChange={(event) => setPassphrase(event.target.value)}
								placeholder={
									profile ? t("settings.connections.secretPlaceholder") : undefined
								}
								autoComplete="new-password"
							/>
						</div>
					) : null}
					{error ? (
						<p className="text-sm text-destructive" role="alert">
							{localizedError(error)}
						</p>
					) : null}
					<div className="flex flex-wrap items-center gap-2">
						<Button type="submit" loading={busy} disabled={!canSubmit || disabled}>
							{profile
								? t("settings.connections.rotate")
								: t("settings.connections.save")}
						</Button>
						{profile ? (
							<>
								<Button
									type="button"
									variant="outline"
									loading={busy}
									disabled={disabled}
									onClick={() => void test()}
								>
									{t("settings.connections.test")}
								</Button>
								<Button
									type="button"
									variant="destructive"
									loading={busy}
									disabled={disabled}
									onClick={() => {
										if (window.confirm(t("settings.connections.deleteConfirm"))) {
											void remove();
										}
									}}
								>
									{t("settings.connections.delete")}
								</Button>
							</>
						) : null}
					</div>
				</form>
			</CardContent>
		</Card>
	);
}

function ProfileSummary({ profile }: { profile: ProfileView }) {
	const { t } = useTranslation();
	const evidence = profile.lastTestEvidence;
	const lastTest = profile.lastTestAtMs
		? new Date(profile.lastTestAtMs).toLocaleString()
		: t("settings.connections.neverTested");
	const capabilities = evidence?.capabilities ?? [];

	return (
		<div className="grid gap-2 rounded-lg border bg-muted/30 p-3 text-sm">
			<dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1">
				<dt className="text-muted-foreground">
					{t("settings.connections.environment")}
				</dt>
				<dd className="font-mono">{profile.environment}</dd>
				<dt className="text-muted-foreground">
					{t("settings.connections.maskedSuffix")}
				</dt>
				<dd className="font-mono">••••{profile.maskedKeySuffix}</dd>
				{profile.accountId ? (
					<>
						<dt className="text-muted-foreground">
							{t("settings.connections.accountId")}
						</dt>
						<dd className="font-mono">{profile.accountId}</dd>
					</>
				) : null}
				{profile.currency ? (
					<>
						<dt className="text-muted-foreground">
							{t("settings.connections.currency")}
						</dt>
						<dd>{profile.currency}</dd>
					</>
				) : null}
				<dt className="text-muted-foreground">
					{t("settings.connections.lastTest")}
				</dt>
				<dd>{lastTest}</dd>
				{capabilities.length > 0 ? (
					<>
						<dt className="text-muted-foreground">
							{t("settings.connections.capabilities")}
						</dt>
						<dd className="flex flex-wrap gap-1">
							{capabilities.map((capability) => (
								<span
									key={capability}
									className="rounded bg-background px-1.5 py-0.5 font-mono text-xs"
								>
									{capability}
								</span>
							))}
						</dd>
					</>
				) : null}
			</dl>
			{evidence?.redactedError ? (
				<p className="text-sm text-destructive">
					{(() => {
						const key = `settings.connections.errors.${evidence.errorCode ?? "unknown"}`;
						const localized = t(key);
						return localized === key ? evidence.redactedError : localized;
					})()}
				</p>
			) : null}
		</div>
	);
}

function StatusBadge({ status }: { status: ProfileView["status"] }) {
	const { t } = useTranslation();
	const usable = status === "usable";
	return (
		<span
			className={cn(
				"inline-flex items-center gap-1 rounded-full px-2.5 py-0.5 text-xs font-medium",
				usable
					? "bg-emerald-600/10 text-emerald-600"
					: "bg-destructive/10 text-destructive",
			)}
		>
			{usable ? (
				<ShieldCheckIcon className="size-3.5" />
			) : (
				<ShieldXIcon className="size-3.5" />
			)}
			{t(
				usable
					? "settings.connections.status.usable"
					: "settings.connections.status.unusable",
			)}
		</span>
	);
}

function parseConnectionError(reason: unknown): ConnectionError {
	if (typeof reason === "string") {
		try {
			const parsed = JSON.parse(reason) as ConnectionError;
			if (parsed && typeof parsed.code === "string") return parsed;
		} catch {
			// Not our typed error contract; fall through to the raw message.
		}
	}
	return { code: "unknown", message: String(reason) };
}
