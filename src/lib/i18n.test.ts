/** @jest-environment jsdom */

import { readFileSync } from "node:fs";
import {
	changeInterfaceLocale,
	formatDateTime,
	formatDecimal,
	formatNumber,
	getInterfaceLocalePreference,
	i18n,
	INTERFACE_LOCALE_STORAGE_KEY,
	RESOURCE_LOCALES,
	resolveInterfaceLocale,
	resolveSystemLocale,
	resources,
	setInterfaceLocalePreference,
} from "@/lib/i18n";

function flatten(value: unknown, prefix = ""): string[] {
	if (typeof value === "string") return [prefix];
	if (!value || typeof value !== "object") return [];

	return Object.entries(value).flatMap(([key, child]) =>
		flatten(child, prefix ? `${prefix}.${key}` : key),
	);
}

function interpolationVariables(
	value: unknown,
	prefix = "",
	result: Record<string, string[]> = {},
) {
	if (typeof value === "string") {
		result[prefix] = [...value.matchAll(/\{\{\s*([^,}\s]+)/g)]
			.map((match) => match[1])
			.sort();
		return result;
	}
	if (!value || typeof value !== "object") return result;

	for (const [key, child] of Object.entries(value)) {
		interpolationVariables(child, prefix ? `${prefix}.${key}` : key, result);
	}
	return result;
}

beforeEach(async () => {
	localStorage.clear();
	await changeInterfaceLocale("en-US");
});

test("resolves Chinese system languages and keeps explicit choices fixed", () => {
	expect(resolveSystemLocale("zh-CN")).toBe("zh-CN");
	expect(resolveSystemLocale("zh-Hant-TW")).toBe("zh-CN");
	expect(resolveSystemLocale("en-GB")).toBe("en-US");
	expect(resolveInterfaceLocale("zh-CN", "en-US")).toBe("zh-CN");
	expect(resolveInterfaceLocale("en-US", "zh-CN")).toBe("en-US");
	expect(resolveInterfaceLocale("system", "zh-CN")).toBe("zh-CN");
});

test("bundles exactly the supported V1 resource locales", () => {
	expect(RESOURCE_LOCALES).toEqual(["en-US", "zh-CN"]);
	expect(Object.keys(resources)).toEqual(["en-US", "zh-CN"]);
});

test("persists only the device-local interface preference", () => {
	setInterfaceLocalePreference("zh-CN");

	expect(localStorage.getItem(INTERFACE_LOCALE_STORAGE_KEY)).toBe("zh-CN");
	expect(getInterfaceLocalePreference()).toBe("zh-CN");
	localStorage.setItem("adaq.user.profile", "unchanged");
	expect(localStorage.getItem(INTERFACE_LOCALE_STORAGE_KEY)).toBe("zh-CN");
});

test("keeps the preference across sign-out and research-data reset boundaries", () => {
	setInterfaceLocalePreference("zh-CN");
	localStorage.setItem("adaq.auth.session", "signed-in");
	const signOut = () => localStorage.removeItem("adaq.auth.session");
	signOut();
	localStorage.setItem("adaq.research-data.reset", "completed");
	const resetResearchData = () =>
		localStorage.removeItem("adaq.research-data.reset");
	resetResearchData();

	expect(getInterfaceLocalePreference()).toBe("zh-CN");
});

test("switches language immediately and updates document.lang", async () => {
	window.history.pushState({}, "", "/settings/general");
	const draft = document.createElement("input");
	draft.value = "unsaved draft";
	document.body.appendChild(draft);

	await changeInterfaceLocale("zh-CN");

	expect(i18n.resolvedLanguage).toBe("zh-CN");
	expect(document.documentElement.lang).toBe("zh-CN");
	expect(i18n.t("nav.dashboard")).toBe("仪表盘");
	expect(window.location.pathname).toBe("/settings/general");
	expect(draft.value).toBe("unsaved draft");

	await changeInterfaceLocale("en-US");
	expect(document.documentElement.lang).toBe("en-US");
	expect(i18n.t("nav.dashboard")).toBe("Dashboard");
	draft.remove();
});

test("changes the in-memory locale when device storage is unavailable", async () => {
	const unavailableStorage = {
		getItem: () => {
			throw new Error("storage unavailable");
		},
		setItem: () => {
			throw new Error("storage unavailable");
		},
	} as unknown as Storage;

	expect(getInterfaceLocalePreference(unavailableStorage)).toBe("system");
	await changeInterfaceLocale("zh-CN", unavailableStorage);

	expect(i18n.resolvedLanguage).toBe("zh-CN");
	expect(document.documentElement.lang).toBe("zh-CN");
});

test("keeps resource keys and interpolation variables in parity", () => {
	const english = resources["en-US"].translation;
	const chinese = resources["zh-CN"].translation;

	expect(flatten(chinese).sort()).toEqual(flatten(english).sort());
	expect(interpolationVariables(chinese)).toEqual(
		interpolationVariables(english),
	);
});

test("falls back to English for a missing active-locale key", async () => {
	i18n.addResource("en-US", "translation", "test.fallback", "English fallback");
	await changeInterfaceLocale("zh-CN");

	expect(i18n.t("test.fallback")).toBe("English fallback");
});

test("uses Intl for display formatting without changing exact decimal digits", async () => {
	await changeInterfaceLocale("en-US");
	const dateOptions: Intl.DateTimeFormatOptions = {
		dateStyle: "medium",
		timeZone: "UTC",
	};

	expect(formatNumber(1234567.5)).toBe(
		new Intl.NumberFormat("en-US").format(1234567.5),
	);
	expect(formatDateTime("2024-01-01T00:00:00Z", dateOptions)).toBe(
		new Intl.DateTimeFormat("en-US", dateOptions).format(
			new Date("2024-01-01T00:00:00Z"),
		),
	);
	expect(formatDecimal("12345678901234567890.123")).toBe(
		"12,345,678,901,234,567,890.123",
	);
	expect(
		formatDecimal("12345678901234567890.123456789", {
			maximumFractionDigits: 8,
		}),
	).toBe("12,345,678,901,234,567,890.12345679");
	const canonicalDecimal = "1583.200000000000000001";
	expect(formatDecimal(canonicalDecimal)).toBe("1,583.200000000000000001");
	expect(canonicalDecimal).toBe("1583.200000000000000001");
	const canonicalTimestamp = "2024-01-01T00:00:00Z";
	formatDateTime(canonicalTimestamp, dateOptions);
	expect(canonicalTimestamp).toBe("2024-01-01T00:00:00Z");
	expect(
		i18n.t("market.instrumentVenue", {
			baseAsset: "BTC",
			quoteAsset: "USDT",
		}),
	).toContain("OKX Spot");
});

test("initializes localization before the React render call", () => {
	const source = readFileSync(new URL("../main.tsx", import.meta.url), "utf8");
	const i18nSource = readFileSync(new URL("./i18n.ts", import.meta.url), "utf8");
	const htmlSource = readFileSync(
		new URL("../../index.html", import.meta.url),
		"utf8",
	);

	expect(source.indexOf('import "@/lib/i18n"')).toBeGreaterThanOrEqual(0);
	expect(source.indexOf('import "@/lib/i18n"')).toBeLessThan(
		source.indexOf("ReactDOM.createRoot"),
	);
	expect(i18nSource).toMatch(/initAsync: false/);
	expect(htmlSource.indexOf("adaq.interfaceLocale")).toBeLessThan(
		htmlSource.indexOf('src="/src/main.tsx"'),
	);
	expect(htmlSource).not.toMatch(/Initializing workspace/);
});
