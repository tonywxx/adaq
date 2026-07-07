import Axios, {
	type AxiosAdapter,
	type AxiosError,
	type AxiosInstance,
	type AxiosProgressEvent,
	type AxiosRequestConfig,
	type AxiosResponse,
	type CustomParamsSerializer,
	type InternalAxiosRequestConfig,
	type Method,
} from "axios";
import { stringify } from "qs";

// ------------------------------------------------------------------
// Base configuration
// ------------------------------------------------------------------

const defaultConfig: AxiosRequestConfig = {
	timeout: 16000,
	headers: {
		"Content-Type": "application/json;charset=utf-8",
	},
	paramsSerializer: {
		serialize: stringify as unknown as CustomParamsSerializer,
	},
};

// Shape every backend response is expected to share
export interface BaseResponse {
	code: number;
	message?: string;
}

// Strip fields that would collide with BaseResponse
type OmitBaseResponse<T> = Omit<T, keyof BaseResponse>;

// Final response type: BaseResponse fields always win
export type ResponseData<T = any> = BaseResponse & OmitBaseResponse<T>;

// Optional validator for a response payload
export type ResponseValidator<T = any> = (data: ResponseData<T>) => boolean;

// ------------------------------------------------------------------
// Retry configuration
// ------------------------------------------------------------------

export interface RetryConfig {
	/** Number of retry attempts after the initial call (default: 0) */
	retries?: number;
	/** Base delay between retries, in ms (default: 1000) */
	retryDelay?: number;
	/** If true, delay grows as retryDelay * 2^attempt instead of staying fixed */
	useExponentialBackoff?: boolean;
	/** Decide whether a given error is worth retrying */
	retryCondition?: (error: AxiosError) => boolean;
	/** Fired right before each retry attempt — useful for logging/telemetry */
	onRetry?: (error: AxiosError, attempt: number) => void;
}

// ------------------------------------------------------------------
// Interceptor overrides
// ------------------------------------------------------------------

interface InterceptorsConfig {
	requestInterceptor?: (
		config: InternalAxiosRequestConfig,
	) => InternalAxiosRequestConfig;
	requestErrorInterceptor?: (error: AxiosError) => Promise<any>;
	responseInterceptor?: (response: AxiosResponse<ResponseData<any>>) => any;
	responseErrorInterceptor?: (error: AxiosError) => Promise<any>;
}

// ------------------------------------------------------------------
// Client-wide options
// ------------------------------------------------------------------

export interface HttpClientOptions {
	/** Cancel an in-flight request that shares the same key when a new one fires (default: true) */
	dedupeRequests?: boolean;
	/** Log requests/responses/errors to the console (default: false) */
	enableLogging?: boolean;
	/** Use Tauri's native HTTP adapter in desktop runtime; plain Axios otherwise */
	transport?: "axios" | "auto" | "tauri";
	/** Extra HTTP status -> message entries, merged over the built-in defaults */
	errorMessages?: Record<number, string>;
	/** Fired whenever the number of in-flight requests changes — wire this to a global spinner */
	onPendingChange?: (pendingCount: number) => void;
	/**
	 * If set, every response is checked against this "success" business code
	 * (e.g. `code === 0`). Responses that don't match are rejected with an Error
	 * built from `message`, even though the HTTP status itself was 2xx.
	 */
	successCode?: number;
}

type RequestKey = string | symbol;

type RequestExtraConfig = {
	requestKey?: RequestKey;
	retry?: RetryConfig;
	/** Skip de-duplication/auto-cancel for this single call (e.g. polling) */
	allowConcurrent?: boolean;
	/** Skip the default error handling/logging for this single call */
	silent?: boolean;
	/** Override the client-wide successCode check for this single call */
	successCode?: number | false;
};

const DEFAULT_ERROR_MESSAGES: Record<number, string> = {
	400: "Bad request",
	401: "Unauthorized, please sign in again",
	403: "Forbidden",
	404: "The requested resource was not found",
	408: "Request timed out",
	429: "Too many requests, please slow down",
	500: "Internal server error",
	502: "Bad gateway",
	503: "Service unavailable",
	504: "Gateway timeout",
};

/**
 * Enhanced HTTP client built on top of Axios.
 *
 * Features:
 * - Configurable request/response interceptors
 * - Request de-duplication / cancellation by key, with an opt-out per call
 * - Retry support with fixed or exponential backoff, plus an onRetry hook
 * - Pending-request counter for driving a global loading indicator
 * - Optional request/response logging
 * - Upload/download helpers with progress callbacks
 * - Optional business-status-code validation (separate from HTTP status)
 * - Multiple independent instances via `createHttpClient`
 */
class HttpClient {
	private instance: AxiosInstance;
	private requestInterceptorId?: number;
	private responseInterceptorId?: number;
	private abortControllers: Map<RequestKey, AbortController> = new Map();
	private pendingCount = 0;
	private options: Required<
		Pick<HttpClientOptions, "dedupeRequests" | "enableLogging">
	> & {
		errorMessages: Record<number, string>;
		onPendingChange?: HttpClientOptions["onPendingChange"];
		successCode?: number;
	};

	/**
	 * @param customConfig Custom Axios config, merged over the defaults
	 * @param interceptors Custom interceptor overrides
	 * @param options Client-wide behavior options
	 */
	constructor(
		customConfig?: AxiosRequestConfig,
		interceptors?: InterceptorsConfig,
		options?: HttpClientOptions,
	) {
		this.instance = Axios.create({
			...defaultConfig,
			...customConfig,
			...resolveTransportConfig(customConfig, options),
		});
		this.options = {
			dedupeRequests: options?.dedupeRequests ?? true,
			enableLogging: options?.enableLogging ?? false,
			errorMessages: { ...DEFAULT_ERROR_MESSAGES, ...options?.errorMessages },
			onPendingChange: options?.onPendingChange,
			successCode: options?.successCode,
		};
		this.initInterceptors(interceptors);
	}

	/** Set up both interceptors */
	private initInterceptors(interceptors?: InterceptorsConfig): void {
		this.initRequestInterceptor(
			interceptors?.requestInterceptor,
			interceptors?.requestErrorInterceptor,
		);
		this.initResponseInterceptor(
			interceptors?.responseInterceptor,
			interceptors?.responseErrorInterceptor,
		);
	}

	/** Set up the request interceptor */
	private initRequestInterceptor(
		customInterceptor?: InterceptorsConfig["requestInterceptor"],
		customErrorInterceptor?: InterceptorsConfig["requestErrorInterceptor"],
	): void {
		const defaultInterceptor = (
			config: InternalAxiosRequestConfig,
		): InternalAxiosRequestConfig => {
			/* Default request-side business logic goes here */
			// Example: attach an auth token
			// const token = localStorage.getItem("token");
			// if (token) config.headers.Authorization = `Bearer ${token}`;

			// Example: bust the cache on GET requests
			// if (config.method?.toUpperCase() === "GET") {
			//   config.params = { ...config.params, _t: Date.now() };
			// }

			if (this.options.enableLogging) {
				console.log(
					`[HttpClient] → ${config.method?.toUpperCase()} ${config.url}`,
					{
						params: config.params,
						data: config.data,
					},
				);
			}

			return config;
		};

		const defaultErrorInterceptor = (error: AxiosError): Promise<any> => {
			if (this.options.enableLogging) {
				console.error("[HttpClient] Request setup error:", error.message);
			}
			return Promise.reject(error);
		};

		this.requestInterceptorId = this.instance.interceptors.request.use(
			customInterceptor || defaultInterceptor,
			customErrorInterceptor || defaultErrorInterceptor,
		);
	}

	/** Set up the response interceptor */
	private initResponseInterceptor(
		customInterceptor?: InterceptorsConfig["responseInterceptor"],
		customErrorInterceptor?: InterceptorsConfig["responseErrorInterceptor"],
	): void {
		const defaultInterceptor = (response: AxiosResponse<ResponseData<any>>) => {
			const requestKey = this.getRequestKey(response.config);
			if (requestKey) this.abortControllers.delete(requestKey);

			if (this.options.enableLogging) {
				console.log(
					`[HttpClient] ← ${response.config.method?.toUpperCase()} ${response.config.url}`,
					response.data,
				);
			}

			// Optional business-status-code check (separate from HTTP status).
			// A 200 OK can still represent an application-level failure.
			const perCallSuccessCode = (
				response.config as AxiosRequestConfig & RequestExtraConfig
			).successCode;
			const effectiveSuccessCode =
				perCallSuccessCode === false
					? undefined
					: (perCallSuccessCode ?? this.options.successCode);

			if (
				effectiveSuccessCode !== undefined &&
				response.data &&
				response.data.code !== effectiveSuccessCode
			) {
				return Promise.reject(
					new Error(
						response.data.message ||
							`Business error (code: ${response.data.code})`,
					),
				);
			}

			/* Default response-side business logic goes here */
			return response.data;
		};

		const defaultErrorInterceptor = (error: AxiosError): Promise<any> => {
			if (error.config) {
				const requestKey = this.getRequestKey(error.config);
				if (requestKey) this.abortControllers.delete(requestKey);
			}

			const silent = (
				error.config as (AxiosRequestConfig & RequestExtraConfig) | undefined
			)?.silent;

			if (Axios.isCancel(error)) {
				if (!silent && this.options.enableLogging)
					console.warn("[HttpClient] Request cancelled:", error.message);
				return Promise.reject(new Error("Request was cancelled"));
			}

			if (!error.response) {
				if (
					error.code === "ECONNABORTED" ||
					error.message?.includes("timeout")
				) {
					return Promise.reject(
						new Error("Request timed out, please try again"),
					);
				}
				return Promise.reject(
					new Error("Network error, please check your connection"),
				);
			}

			const status = error.response.status;
			const message =
				this.options.errorMessages[status] ||
				`Request failed (status: ${status})`;

			if (!silent && this.options.enableLogging) {
				console.error(`[HttpClient] ${status} ${error.config?.url}:`, message);
			}

			return Promise.reject(new Error(message));
		};

		this.responseInterceptorId = this.instance.interceptors.response.use(
			customInterceptor || defaultInterceptor,
			customErrorInterceptor || defaultErrorInterceptor,
		);
	}

	/** Build a unique key for a request from its method + url */
	private getRequestKey(config: AxiosRequestConfig): RequestKey | undefined {
		if (!config.url) return undefined;
		return `${config.method?.toUpperCase()}-${config.url}`;
	}

	/** Attach an AbortController, cancelling any earlier request under the same key */
	private setupCancelController(
		config: AxiosRequestConfig,
		requestKey?: RequestKey,
	): AxiosRequestConfig {
		const key = requestKey || this.getRequestKey(config);
		if (!key) return config;

		this.cancelRequest(key);

		const controller = new AbortController();
		this.abortControllers.set(key, controller);

		return { ...config, signal: controller.signal };
	}

	/** Update the pending-request counter and notify listeners */
	private adjustPendingCount(delta: number): void {
		this.pendingCount = Math.max(0, this.pendingCount + delta);
		this.options.onPendingChange?.(this.pendingCount);
	}

	/** Remove the request interceptor */
	public removeRequestInterceptor(): void {
		if (this.requestInterceptorId !== undefined) {
			this.instance.interceptors.request.eject(this.requestInterceptorId);
			this.requestInterceptorId = undefined;
		}
	}

	/** Remove the response interceptor */
	public removeResponseInterceptor(): void {
		if (this.responseInterceptorId !== undefined) {
			this.instance.interceptors.response.eject(this.responseInterceptorId);
			this.responseInterceptorId = undefined;
		}
	}

	/** Replace the request interceptor at runtime */
	public setRequestInterceptor(
		customInterceptor?: InterceptorsConfig["requestInterceptor"],
		customErrorInterceptor?: InterceptorsConfig["requestErrorInterceptor"],
	): void {
		this.removeRequestInterceptor();
		this.initRequestInterceptor(customInterceptor, customErrorInterceptor);
	}

	/** Replace the response interceptor at runtime */
	public setResponseInterceptor(
		customInterceptor?: InterceptorsConfig["responseInterceptor"],
		customErrorInterceptor?: InterceptorsConfig["responseErrorInterceptor"],
	): void {
		this.removeResponseInterceptor();
		this.initResponseInterceptor(customInterceptor, customErrorInterceptor);
	}

	/** Access the underlying Axios instance directly if needed */
	public getInstance(): AxiosInstance {
		return this.instance;
	}

	/** Number of requests currently in flight */
	public getPendingCount(): number {
		return this.pendingCount;
	}

	/**
	 * Cancel a single request by key.
	 * @returns whether a matching in-flight request was found and cancelled
	 */
	public cancelRequest(key: RequestKey, message?: string): boolean {
		const controller = this.abortControllers.get(key);
		if (controller) {
			controller.abort(message || `Cancelled request: ${String(key)}`);
			this.abortControllers.delete(key);
			return true;
		}
		return false;
	}

	/** Cancel every in-flight request */
	public cancelAllRequests(message?: string): void {
		this.abortControllers.forEach((controller, key) => {
			controller.abort(message || `Cancelled all requests: ${String(key)}`);
		});
		this.abortControllers.clear();
	}

	/** Whether an error is an Axios cancellation error */
	public static isCancel(error: unknown): boolean {
		return Axios.isCancel(error);
	}

	private sleep(ms: number): Promise<void> {
		return new Promise((resolve) => setTimeout(resolve, ms));
	}

	/**
	 * Generic request method used by get/post/put/delete/patch.
	 */
	public async request<T = any>(
		method: Method,
		url: string,
		config?: AxiosRequestConfig & RequestExtraConfig,
	): Promise<ResponseData<T>> {
		const {
			requestKey,
			retry,
			allowConcurrent,
			silent,
			successCode,
			...restConfig
		} = config || {};

		const defaultRetryCondition = (error: AxiosError) => {
			// By default only retry on network errors or 5xx server errors
			return (
				!error.response ||
				(error.response.status >= 500 && error.response.status < 600)
			);
		};

		const retryConfig: Required<Omit<RetryConfig, "onRetry">> &
			Pick<RetryConfig, "onRetry"> = {
			retries: 0,
			retryDelay: 1000,
			useExponentialBackoff: false,
			retryCondition: defaultRetryCondition,
			onRetry: undefined,
			...retry,
		};

		const key =
			requestKey || this.getRequestKey({ ...restConfig, method, url });
		let lastError: any;

		this.adjustPendingCount(1);
		try {
			for (let attempt = 0; attempt <= retryConfig.retries; attempt++) {
				try {
					if (attempt > 0 && key) {
						this.abortControllers.delete(key);
					}

					const baseConfig: AxiosRequestConfig & RequestExtraConfig = {
						...restConfig,
						method,
						url,
						silent,
						successCode,
					};
					const requestConfig = allowConcurrent
						? baseConfig
						: this.setupCancelController(baseConfig, requestKey);

					return await this.instance.request<ResponseData<T>, ResponseData<T>>(
						requestConfig,
					);
				} catch (error) {
					lastError = error;

					if (
						attempt === retryConfig.retries ||
						!retryConfig.retryCondition(error as AxiosError) ||
						HttpClient.isCancel(error)
					) {
						break;
					}

					retryConfig.onRetry?.(error as AxiosError, attempt + 1);

					const delay = retryConfig.useExponentialBackoff
						? retryConfig.retryDelay * 2 ** attempt
						: retryConfig.retryDelay;
					if (delay > 0) await this.sleep(delay);
				}
			}

			return Promise.reject(lastError);
		} finally {
			this.adjustPendingCount(-1);
		}
	}

	public get<T = any>(
		url: string,
		config?: AxiosRequestConfig & RequestExtraConfig,
	): Promise<ResponseData<T>> {
		return this.request<T>("get", url, config);
	}

	public post<T = any>(
		url: string,
		data?: any,
		config?: AxiosRequestConfig & RequestExtraConfig,
	): Promise<ResponseData<T>> {
		return this.request<T>("post", url, { ...config, data });
	}

	public put<T = any>(
		url: string,
		data?: any,
		config?: AxiosRequestConfig & RequestExtraConfig,
	): Promise<ResponseData<T>> {
		return this.request<T>("put", url, { ...config, data });
	}

	public delete<T = any>(
		url: string,
		config?: AxiosRequestConfig & RequestExtraConfig,
	): Promise<ResponseData<T>> {
		return this.request<T>("delete", url, config);
	}

	public patch<T = any>(
		url: string,
		data?: any,
		config?: AxiosRequestConfig & RequestExtraConfig,
	): Promise<ResponseData<T>> {
		return this.request<T>("patch", url, { ...config, data });
	}

	/**
	 * Upload one or more files as multipart/form-data.
	 * @param url Endpoint URL
	 * @param files A single File/Blob, or a map of field name -> File/Blob
	 * @param extraFields Additional plain fields to send alongside the files
	 * @param onProgress Upload progress callback (0-100)
	 */
	public upload<T = any>(
		url: string,
		files: File | Blob | Record<string, File | Blob>,
		extraFields?: Record<string, string | number | boolean>,
		onProgress?: (percent: number) => void,
		config?: AxiosRequestConfig & RequestExtraConfig,
	): Promise<ResponseData<T>> {
		const formData = new FormData();

		if (files instanceof Blob) {
			formData.append("file", files);
		} else {
			Object.entries(files).forEach(([field, file]) =>
				formData.append(field, file),
			);
		}

		if (extraFields) {
			Object.entries(extraFields).forEach(([key, value]) =>
				formData.append(key, String(value)),
			);
		}

		return this.request<T>("post", url, {
			...config,
			data: formData,
			headers: { ...config?.headers, "Content-Type": "multipart/form-data" },
			onUploadProgress: (event: AxiosProgressEvent) => {
				if (onProgress && event.total) {
					onProgress(Math.round((event.loaded * 100) / event.total));
				}
			},
		});
	}

	/**
	 * Download a file as a Blob and optionally trigger a browser save-as.
	 * @param url Endpoint URL
	 * @param filename If provided, triggers a browser download with this filename
	 * @param onProgress Download progress callback (0-100)
	 */
	public async download(
		url: string,
		filename?: string,
		onProgress?: (percent: number) => void,
		config?: AxiosRequestConfig & RequestExtraConfig,
	): Promise<Blob> {
		const response = await this.instance.request<Blob>({
			...config,
			url,
			method: config?.method || "get",
			responseType: "blob",
			onDownloadProgress: (event: AxiosProgressEvent) => {
				if (onProgress && event.total) {
					onProgress(Math.round((event.loaded * 100) / event.total));
				}
			},
		});

		// The response interceptor normally unwraps `response.data`; for blobs we
		// bypass that by reading the raw axios response, so cast accordingly.
		const blob = response as unknown as Blob;

		if (filename && typeof document !== "undefined") {
			const blobUrl = URL.createObjectURL(blob);
			const link = document.createElement("a");
			link.href = blobUrl;
			link.download = filename;
			document.body.appendChild(link);
			link.click();
			link.remove();
			URL.revokeObjectURL(blobUrl);
		}

		return blob;
	}
}

/** Convenience factory for spinning up an independently-configured instance */
export function createHttpClient(
	customConfig?: AxiosRequestConfig,
	interceptors?: InterceptorsConfig,
	options?: HttpClientOptions,
): HttpClient {
	return new HttpClient(customConfig, interceptors, options);
}

// Default, ready-to-use instance
export const http = new HttpClient();

export default HttpClient;

export function isTauriRuntime() {
	return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

function resolveTransportConfig(
	config?: AxiosRequestConfig,
	options?: HttpClientOptions,
): AxiosRequestConfig {
	if (config?.adapter) {
		return {};
	}

	const transport = options?.transport ?? "axios";
	if (transport === "axios" || (transport === "auto" && !isTauriRuntime())) {
		return {};
	}

	return { adapter: createLazyTauriHttpAdapter() };
}

function createLazyTauriHttpAdapter(): AxiosAdapter {
	return async (config) => {
		const { createTauriHttpAdapter } = await import("@/lib/tauri-http-adapter");
		return createTauriHttpAdapter()(config);
	};
}
