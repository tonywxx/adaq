import { readSessionCache, writeSessionCache } from "@/lib/session-cache";
import type { FactorJson } from "./factor-types";

export const FACTOR_PAGE_SIZE = 50;
export const MAX_GRID_TRIALS = 256;

export function factorPageCount(total: number, pageSize = FACTOR_PAGE_SIZE) {
	return Math.max(1, Math.ceil(total / pageSize));
}

export function finiteGridTrialCount(cardinalities: number[]): number | null {
	if (
		cardinalities.length === 0 ||
		cardinalities.some((value) => !Number.isSafeInteger(value) || value <= 0)
	)
		return null;
	return cardinalities.reduce((total, value) => total * value, 1);
}

export function isGridWithinLimit(cardinalities: number[]) {
	const count = finiteGridTrialCount(cardinalities);
	return count !== null && count <= MAX_GRID_TRIALS;
}

export function isTerminalFactorAttempt(status: string) {
	return (
		status === "completed" ||
		status === "failed" ||
		status === "cancelled" ||
		status === "interrupted" ||
		status === "stale"
	);
}

export function factorHash(value: unknown) {
	return typeof value === "string" && /^[0-9a-f]{64}$/.test(value);
}

export function shortFactorHash(value: unknown, length = 16) {
	if (typeof value !== "string") return "—";
	return value.length > length ? `${value.slice(0, length)}…` : value;
}

export function factorString(value: unknown, fallback = "—") {
	return typeof value === "string" || typeof value === "number"
		? String(value)
		: fallback;
}

export function factorJson(value: unknown): FactorJson {
	if (!value || typeof value !== "object" || Array.isArray(value)) {
		throw new Error("Factor evidence must be a JSON object.");
	}
	return value as FactorJson;
}

export function factorJsonArray(value: unknown): FactorJson[] {
	if (!Array.isArray(value))
		throw new Error("Factor evidence must be a JSON array.");
	return value.map((item) => factorJson(item));
}

export function parseFactorJson(text: string, label: string): FactorJson {
	try {
		return factorJson(JSON.parse(text));
	} catch (error) {
		throw new Error(
			`${label}: ${error instanceof Error ? error.message : "invalid JSON"}`,
		);
	}
}

export function parseFactorJsonArray(
	text: string,
	label: string,
): FactorJson[] {
	try {
		return factorJsonArray(JSON.parse(text));
	} catch (error) {
		throw new Error(
			`${label}: ${error instanceof Error ? error.message : "invalid JSON"}`,
		);
	}
}

export function formatFactorJson(value: unknown) {
	return JSON.stringify(value, null, 2);
}

export function formatFactorError(error: unknown) {
	if (typeof error === "string") return error;
	if (error instanceof Error) return error.message;
	try {
		return JSON.stringify(error);
	} catch {
		return String(error);
	}
}

export function readFactorCache<T>(userId: string | null, resource: string) {
	return readSessionCache(userId, `factors:${resource}`) as T | undefined;
}

export function writeFactorCache(
	userId: string | null,
	resource: string,
	value: unknown,
) {
	writeSessionCache(userId, `factors:${resource}`, value);
}
