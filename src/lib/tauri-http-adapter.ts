import { fetch as tauriFetch } from "@tauri-apps/plugin-http";
import {
	type AxiosAdapter,
	AxiosError,
	AxiosHeaders,
	type AxiosRequestConfig,
	type AxiosResponse,
} from "axios";

export function isTauriRuntime() {
	return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export function createTauriHttpAdapter(): AxiosAdapter {
	return async (config) => {
		try {
			const response = await tauriFetch(buildUrl(config), {
				method: config.method?.toUpperCase() ?? "GET",
				headers: normalizeHeaders(config.headers),
				body: shouldSendBody(config) ? config.data : undefined,
				signal: config.signal as AbortSignal | undefined,
			});
			const data = await readResponse(response);
			const axiosResponse: AxiosResponse = {
				data,
				status: response.status,
				statusText: response.statusText,
				headers: headersToAxios(response.headers),
				config,
				request: undefined,
			};

			if (response.ok) {
				return axiosResponse;
			}

			throw new AxiosError(
				`Request failed with status code ${response.status}`,
				AxiosError.ERR_BAD_RESPONSE,
				config,
				undefined,
				axiosResponse,
			);
		} catch (error) {
			if (error instanceof AxiosError) {
				throw error;
			}

			throw new AxiosError(
				error instanceof Error ? error.message : "Tauri HTTP request failed",
				AxiosError.ERR_NETWORK,
				config,
			);
		}
	};
}

function buildUrl(config: AxiosRequestConfig) {
	const url = new URL(config.url ?? "", config.baseURL);

	if (config.params && typeof config.params === "object") {
		for (const [key, value] of Object.entries(
			config.params as Record<string, unknown>,
		)) {
			if (value !== undefined && value !== null) {
				url.searchParams.set(key, String(value));
			}
		}
	}

	return url.toString();
}

function normalizeHeaders(headers: AxiosRequestConfig["headers"]) {
	const normalized = AxiosHeaders.from(
		headers as Parameters<typeof AxiosHeaders.from>[0],
	).toJSON();
	const result: Record<string, string> = {};

	for (const [key, value] of Object.entries(normalized)) {
		if (value !== undefined && value !== null) {
			result[key] = String(value);
		}
	}

	return result;
}

function headersToAxios(headers: Headers) {
	const result: Record<string, string> = {};
	headers.forEach((value, key) => {
		result[key] = value;
	});
	return AxiosHeaders.from(result);
}

async function readResponse(response: Response) {
	const contentType = response.headers.get("content-type") ?? "";

	if (contentType.includes("json") || contentType.includes("javascript")) {
		return response.json();
	}

	return response.text();
}

function shouldSendBody(config: AxiosRequestConfig) {
	const method = config.method?.toUpperCase() ?? "GET";
	return method !== "GET" && method !== "HEAD" && config.data !== undefined;
}
