import i18next from "i18next";
import { initReactI18next } from "react-i18next";

export const RESOURCE_LOCALES = ["en-US", "zh-CN"] as const;
export type ResourceLocale = (typeof RESOURCE_LOCALES)[number];
export type InterfaceLocalePreference = "system" | ResourceLocale;

export const INTERFACE_LOCALE_STORAGE_KEY = "adaq.interfaceLocale";

const resources = {
	"en-US": {
		translation: {
			auth: {
				supabaseNotConfigured: "Supabase is not configured",
				supabaseNotConfiguredDescription:
					"Set VITE_SUPABASE_URL and VITE_SUPABASE_PUBLISHABLE_KEY, then restart the app.",
				hostAuthenticationFailed: "Unable to verify this session",
				hostAuthenticationFailedDescription:
					"The desktop host could not verify your session. Sign in again or restart the app.",
				initializingAria: "AdaQ is initializing",
				initializingWorkspace: "Initializing workspace…",
				signIn: "Sign in",
				signInWithPassword: "Sign in with password",
				enterVerificationCode: "Enter verification code",
				createStrongPassword: "Create a strong password",
				secureAccount: "Secure your account before continuing.",
				checkEmail: "Check your email for the 8-digit code.",
				passwordSignInNoSession:
					"Password sign-in succeeded, but no session was returned.",
				codeAcceptedNoSession:
					"The code was accepted, but no session was returned.",
				continue: "Continue",
				verifyCode: "Verify code",
				differentEmail: "Use a different email",
				emailCodeInstead: "Sign in with email code instead",
				emailDescription:
					"Please input your email address. For new accounts, an email with a OTP code will be sent to your email address. Existing accounts continue with password.",
				email: "Email",
				password: "Password",
				code: "Code",
				confirmPassword: "Confirm password",
				createPassword: "Create password",
				passwordsMatch: "Passwords match",
				passwordRequirements: {
					length: "At least 8 characters",
					lowercase: "Lowercase letter",
					uppercase: "Uppercase letter",
					digit: "Digit",
					symbol: "Symbol",
				},
			},
		},
	},
	"zh-CN": {
		translation: {
			auth: {
				supabaseNotConfigured: "Supabase 尚未配置",
				supabaseNotConfiguredDescription:
					"设置 VITE_SUPABASE_URL 和 VITE_SUPABASE_PUBLISHABLE_KEY，然后重启应用。",
				hostAuthenticationFailed: "无法验证当前会话",
				hostAuthenticationFailedDescription:
					"桌面 Host 无法验证您的会话。请重新登录或重启应用。",
				initializingAria: "AdaQ 正在初始化",
				initializingWorkspace: "正在初始化工作区…",
				signIn: "登录",
				signInWithPassword: "使用密码登录",
				enterVerificationCode: "输入验证码",
				createStrongPassword: "创建强密码",
				secureAccount: "请先保护您的账户。",
				checkEmail: "请查看您的邮箱，获取 8 位验证码。",
				passwordSignInNoSession: "密码登录成功，但未返回会话。",
				codeAcceptedNoSession: "验证码已接受，但未返回会话。",
				continue: "继续",
				verifyCode: "验证验证码",
				differentEmail: "使用其他邮箱",
				emailCodeInstead: "改用邮箱验证码登录",
				emailDescription:
					"请输入您的邮箱地址。新账户会收到包含验证码的邮件，已有账户将继续使用密码登录。",
				email: "邮箱",
				password: "密码",
				code: "验证码",
				confirmPassword: "确认密码",
				createPassword: "创建密码",
				passwordsMatch: "密码匹配",
				passwordRequirements: {
					length: "至少 8 个字符",
					lowercase: "小写字母",
					uppercase: "大写字母",
					digit: "数字",
					symbol: "符号",
				},
			},
		},
	},
} as const;

function localStorageOrNull() {
	if (typeof window === "undefined") return null;
	try {
		return window.localStorage;
	} catch {
		return null;
	}
}

function isInterfaceLocalePreference(
	value: string | null,
): value is InterfaceLocalePreference {
	return (
		value === "system" || RESOURCE_LOCALES.includes(value as ResourceLocale)
	);
}

export function getSystemLanguage() {
	if (typeof navigator === "undefined") return "en-US";
	return navigator.languages?.[0] ?? navigator.language ?? "en-US";
}

export function resolveSystemLocale(
	language = getSystemLanguage(),
): ResourceLocale {
	return /^zh(?:-|$)/i.test(language.trim()) ? "zh-CN" : "en-US";
}

export function resolveInterfaceLocale(
	preference: InterfaceLocalePreference,
	systemLanguage = getSystemLanguage(),
): ResourceLocale {
	return preference === "system"
		? resolveSystemLocale(systemLanguage)
		: preference;
}

export function getInterfaceLocalePreference(
	storage: Storage | null = localStorageOrNull(),
): InterfaceLocalePreference {
	try {
		const stored = storage?.getItem(INTERFACE_LOCALE_STORAGE_KEY) ?? null;
		return isInterfaceLocalePreference(stored) ? stored : "system";
	} catch {
		return "system";
	}
}

export const i18n = i18next;
const initialLocale = resolveInterfaceLocale(getInterfaceLocalePreference());

function updateDocumentLanguage(locale: string) {
	if (typeof document !== "undefined") {
		document.documentElement.lang = /^zh(?:-|$)/i.test(locale)
			? "zh-CN"
			: "en-US";
	}
}

updateDocumentLanguage(initialLocale);
i18n.on("languageChanged", updateDocumentLanguage);

if (!i18n.isInitialized) {
	void i18n.use(initReactI18next).init({
		resources,
		lng: initialLocale,
		fallbackLng: "en-US",
		supportedLngs: [...RESOURCE_LOCALES],
		load: "currentOnly",
		defaultNS: "translation",
		initAsync: false,
		returnEmptyString: false,
		interpolation: { escapeValue: false },
	});
}
