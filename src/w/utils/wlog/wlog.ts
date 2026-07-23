/**
 * wlog.ts - Advanced TypeScript Logging Utility by TONy.W
 *
 * Features:
 * - wlog() as the default smart formatter
 * - Level-specific methods: info/warn/error/debug/trace/success
 * - Precise caller location tracking
 * - True singleton pattern for optimal performance
 * - Beautiful colored console output with %c styling
 */

// ============================================================================
// Type Definitions
// ============================================================================

export type ConsoleMethods =
	| "log"
	| "debug"
	| "info"
	| "warn"
	| "error"
	| "table"
	| "clear"
	| "time"
	| "timeEnd"
	| "count"
	| "assert";

export enum LogLevel {
	OFF = 0,
	ERROR = 1,
	WARN = 2,
	INFO = 3,
	DEBUG = 4,
	TRACE = 5,
	ALL = 6,
}

export interface LogColorScheme {
	label: string;
	background: string;
	text: string;
	border: string;
}

export interface WLogConfig {
	logSwitch: boolean;
	level: LogLevel;
	showTime: boolean;
	showCaller: boolean;
	prefix: string;
	colors: Record<string, LogColorScheme>;
	maxDepth: number;
	prettyJSON: boolean;
}

export interface WLogFn {
	// Default smart logging
	(label: string, data: unknown): void;
	(data: unknown): void;
	(...args: unknown[]): void;

	// Level methods
	info: (...args: unknown[]) => void;
	warn: (...args: unknown[]) => void;
	error: (...args: unknown[]) => void;
	debug: (...args: unknown[]) => void;
	trace: (...args: unknown[]) => void;
	success: (...args: unknown[]) => void;

	// Utility methods
	smart: (label: string, data: unknown) => void;
	table: (label: string, data: unknown[]) => void;
	divider: (char?: string, length?: number) => void;
	section: (title: string) => void;
	time: (label: string) => void;
	timeEnd: (label: string) => void;
	count: (label?: string) => void;
	countReset: (label?: string) => void;
	clear: () => void;
	assert: (condition: boolean, ...args: unknown[]) => void;
	json: (label: string, data: unknown, indent?: number) => void;

	// Config methods
	config: (config: Partial<WLogConfig>) => void;
	getConfig: () => Readonly<WLogConfig>;
	reset: () => void;
	setLevel: (level: LogLevel | keyof typeof LogLevel) => void;
	setSwitch: (enabled: boolean) => void;
	setPrefix: (prefix: string) => void;
}

// ============================================================================
// Default Configuration
// ============================================================================

const DEFAULT_COLORS: Record<string, LogColorScheme> = {
	info: {
		label: "INFO",
		background: "#e3f2fd",
		text: "#1976d2",
		border: "#2196f3",
	},
	warn: {
		label: "WARN",
		background: "#fff8e1",
		text: "#f57c00",
		border: "#ffb300",
	},
	error: {
		label: "ERR ",
		background: "#ffebee",
		text: "#d32f2f",
		border: "#f44336",
	},
	debug: {
		label: "DBG ",
		background: "#e8f5e9",
		text: "#388e3c",
		border: "#4caf50",
	},
	trace: {
		label: "TRC ",
		background: "#f3e5f5",
		text: "#7b1fa2",
		border: "#9c27b0",
	},
	success: {
		label: "SUCC",
		background: "#e8f5e9",
		text: "#388e3c",
		border: "#4caf50",
	},
	smart: {
		label: "LOG ",
		background: "#f0f4f8",
		text: "#5c7cfa",
		border: "#748ffc",
	},
};

const isDevMode = (() => {
	try {
		return (Function("return import.meta.env")() as { DEV?: boolean })?.DEV !== false;
	} catch {
		return (
			typeof (globalThis as { process?: { env?: { NODE_ENV?: string } } }).process === "undefined" ||
			(globalThis as { process?: { env?: { NODE_ENV?: string } } }).process?.env?.NODE_ENV !== "production"
		);
	}
})();

const DEFAULT_CONFIG: WLogConfig = {
	logSwitch: true,
	level: isDevMode ? LogLevel.ALL : LogLevel.ERROR,
	showTime: true,
	showCaller: true,
	prefix: "",
	colors: DEFAULT_COLORS,
	maxDepth: 5,
	prettyJSON: true,
};

// ============================================================================
// Singleton Config (Module-level)
// ============================================================================

const config: WLogConfig = { ...DEFAULT_CONFIG };

// ============================================================================
// Internal Utilities
// ============================================================================

function formatTime(): string {
	return new Date().toLocaleTimeString("en-US", {
		hour12: false,
		hour: "2-digit",
		minute: "2-digit",
		second: "2-digit",
	});
}

/**
 * Robust caller location extraction (works across Chrome, Firefox, Safari, Edge)
 */
function getCallerLocation(): string {
	const stack = new Error().stack?.split("\n") ?? [];
	for (let i = 1; i < stack.length; i++) {
		const line = stack[i];
		if (line.includes("wlog.ts") || line.includes("wlog.js")) continue;

		// Try multiple stack formats
		const regexes = [
			/\(([^)]+):(\d+):(\d+)\)/, // Chrome/Edge
			/@([^@(]+):(\d+):(\d+)/, // Firefox/Safari
			/at\s+(.+):(\d+):(\d+)/, // General
		];

		for (const regex of regexes) {
			const match = line.match(regex);
			if (match) {
				const [, file, lineNum, column] = match;
				const fileName = file.split(/[/\\]/).pop()?.split("?")[0] || file;
				return `${fileName}:${lineNum}:${column}`;
			}
		}
	}
	return "";
}

/**
 * Lightweight deep clone with depth limit
 */
function deepClone<T>(obj: T, depth = 0, maxDepth = 5): T {
	if (depth > maxDepth) return obj as T;
	if (obj === null || typeof obj !== "object") return obj;

	if (Array.isArray(obj)) {
		return obj.map((item) => deepClone(item, depth + 1, maxDepth)) as T;
	}

	const cloned: Record<string, unknown> = {};
	for (const key in obj) {
		if (Object.hasOwn(obj, key)) {
			cloned[key] = deepClone(
				(obj as Record<string, unknown>)[key],
				depth + 1,
				maxDepth,
			);
		}
	}
	return cloned as T;
}

// ============================================================================
// Common Styling Helpers
// ============================================================================

function getBaseStyles() {
	return {
		time: "color: #999; font-size: 10px; font-family: monospace;",
		label: "font-size: 10px; font-weight: bold;",
		caller: "color: #999; font-style: italic; font-size: 10px;",
	};
}

// ============================================================================
// Smart Formatter (Core)
// ============================================================================

function smartFormat(label: string, data: unknown): void {
	if (!config.logSwitch) return;

	const { time, label: labelStyle, caller: callerStyle } = getBaseStyles();
	const caller = config.showCaller ? getCallerLocation() : "";
	const prefix = config.prefix ? `[${config.prefix}] ` : "";
	const timeStr = config.showTime ? formatTime() : "";
	const colorScheme = config.colors.smart;

	const valueStyle = `
    color: ${colorScheme.text};
    background: ${colorScheme.background};
    border: 1px solid ${colorScheme.border};
    padding: 1px 6px;
    border-radius: 3px;
    font-weight: bold;
    font-size: 10px;
  `;

	// Special primitives
	if (data === null || data === undefined) {
		console.log(
			`%c${prefix}${label}%c${caller} %c ${data} `,
			"color: #f57c00; font-weight: bold;",
			callerStyle,
			"background: #fff8e1; color: #f57c00; border: 1px solid #ffb300; padding: 2px 4px; border-radius: 3px;",
		);
		return;
	}

	const type = typeof data;

	if (type === "string") {
		console.log(
			config.showTime
				? `%c ${timeStr} %c ${label} %c ${prefix}${data} %c ${caller}`
				: `%c ${label} %c ${prefix}${data} %c ${caller}`,
			time,
			labelStyle,
			valueStyle,
			callerStyle,
		);
		return;
	}

	if (
		type === "number" ||
		type === "bigint" ||
		type === "boolean" ||
		type === "symbol"
	) {
		const color =
			type === "boolean"
				? data
					? "#388e3c"
					: "#d32f2f"
				: type === "bigint"
					? "#9c27b0"
					: "#d32f2f";

		console.log(
			config.showTime
				? `%c ${timeStr} %c ${label} %c ${prefix}${String(data)}${type === "bigint" ? "n" : ""} %c ${caller}`
				: `%c ${label} %c ${prefix}${String(data)}${type === "bigint" ? "n" : ""} %c ${caller}`,
			time,
			labelStyle,
			`color: ${color}; font-weight: bold; font-family: monospace;`,
			callerStyle,
		);
		return;
	}

	// Complex types
	if (Array.isArray(data)) {
		const header = config.showTime
			? `%c ${timeStr} %c ${label} %c ${prefix}[${data.length} items] %c ${caller}`
			: `%c ${label} %c ${prefix}[${data.length} items] %c ${caller}`;

		console.groupCollapsed(
			header,
			...(config.showTime
				? [time, labelStyle, "color:#666", callerStyle]
				: [labelStyle, "color:#666", callerStyle]),
		);
		console.table(data);
		console.groupEnd();
		return;
	}

	if (data instanceof Date) {
		console.log(
			config.showTime
				? `%c ${timeStr} %c ${label} %c ${prefix}${data.toISOString()} %c ${caller}`
				: `%c ${label} %c ${prefix}${data.toISOString()} %c ${caller}`,
			time,
			labelStyle,
			"color: #1976d2;",
			callerStyle,
		);
		return;
	}

	if (data instanceof Error) {
		const header = config.showTime
			? `%c ${timeStr} %c ${label} %c ${prefix} %c ${caller}`
			: `%c ${label} %c ${prefix} %c ${caller}`;

		console.groupCollapsed(
			header,
			...(config.showTime
				? [time, labelStyle, valueStyle, callerStyle]
				: [labelStyle, valueStyle, callerStyle]),
		);
		console.log("Message:", data.message);
		console.log("Stack:", data.stack);
		console.groupEnd();
		return;
	}

	if (data instanceof RegExp || data instanceof Promise) {
		console.log(
			config.showTime
				? `%c ${timeStr} %c ${label} %c ${prefix}[${data.constructor.name}] %c ${caller}`
				: `%c ${label} %c ${prefix}[${data.constructor.name}] %c ${caller}`,
			time,
			labelStyle,
			"background: #e3f2fd; color: #1976d2; padding: 2px 4px; border-radius: 3px;",
			callerStyle,
		);
		return;
	}

	// Objects
	const obj = data as Record<string, unknown>;
	const keys = Object.keys(obj);

	const header = config.showTime
		? `%c ${timeStr} %c ${label} %c ${prefix}[${keys.length} keys] %c ${caller}`
		: `%c ${label} %c ${prefix}[${keys.length} keys] %c ${caller}`;

	const styles = config.showTime
		? [time, labelStyle, "color:#666; font-size:10px;", callerStyle]
		: [labelStyle, "color:#666; font-size:10px;", callerStyle];

	console.groupCollapsed(header, ...styles);

	if (keys.length === 0) {
		console.log("{} (empty object)");
	} else {
		const isFlat = keys.every((k) => {
			const v = obj[k];
			return typeof v !== "object" || v === null || Array.isArray(v);
		});

		if (isFlat && keys.length <= 10) {
			console.table(obj);
		} else {
			console.dir(deepClone(obj, 0, config.maxDepth), { depth: config.maxDepth });
		}
	}

	console.groupEnd();
}

// ============================================================================
// Level-based Logging
// ============================================================================

function levelLog(
	level: keyof typeof config.colors,
	message: unknown,
	...args: unknown[]
): void {
	if (
		!config.logSwitch ||
		config.level < LogLevel[level.toUpperCase() as keyof typeof LogLevel]
	) {
		return;
	}

	const colorScheme = config.colors[level];
	const caller = config.showCaller ? getCallerLocation() : "";
	const timeStr = config.showTime ? formatTime() : "";
	const prefix = config.prefix ? `[${config.prefix}] ` : "";

	const labelStyle = `
    color: ${colorScheme.text};
    background: ${colorScheme.background};
    border: 1px solid ${colorScheme.border};
    padding: 2px 6px;
    border-radius: 3px;
    font-weight: bold;
    font-size: 11px;
  `;

	const { time, caller: callerStyle } = getBaseStyles();

	const consoleMethod =
		level === "error"
			? console.error
			: level === "warn"
				? console.warn
				: level === "info"
					? console.info
					: console.log;

	const logMessage = String(message);

	if (config.showTime) {
		consoleMethod(
			`%c ${timeStr} %c ${colorScheme.label} %c ${prefix}${logMessage} %c ${caller}`,
			time,
			labelStyle,
			"color: #333;",
			callerStyle,
			...args,
		);
	} else {
		consoleMethod(
			`%c ${colorScheme.label} %c ${prefix}${logMessage} %c ${caller}`,
			labelStyle,
			"color: #333;",
			callerStyle,
			...args,
		);
	}
}

// ============================================================================
// Main wlog Function
// ============================================================================

function wlogFn(...args: unknown[]): void {
	if (!config.logSwitch) return;

	// wlog(label, data)
	if (args.length === 2 && typeof args[0] === "string") {
		smartFormat(args[0] as string, args[1]);
		return;
	}

	// wlog(data) - single argument
	if (args.length === 1) {
		const data = args[0];
		if (typeof data === "string") {
			smartFormat("value", `"${data}"`);
		} else {
			smartFormat("data", data);
		}
		return;
	}

	// Fallback for multiple arguments
	const caller = config.showCaller ? getCallerLocation() : "";
	const timeStr = config.showTime ? formatTime() : "";
	const prefix = config.prefix ? `[${config.prefix}] ` : "";

	if (config.showTime) {
		console.log(
			`%c ${timeStr} %c ${prefix}%c ${caller}`,
			"color:#999;",
			"",
			"color:#999; font-style:italic;",
			...args,
		);
	} else {
		console.log(
			`%c ${prefix}%c ${caller}`,
			"",
			"color:#999; font-style:italic;",
			...args,
		);
	}
}

// ============================================================================
// Attach Methods
// ============================================================================

// Level methods
wlogFn.info = (msg: unknown, ...args: unknown[]) =>
	levelLog("info", msg, ...args);
wlogFn.warn = (msg: unknown, ...args: unknown[]) =>
	levelLog("warn", msg, ...args);
wlogFn.error = (msg: unknown, ...args: unknown[]) =>
	levelLog("error", msg, ...args);
wlogFn.debug = (msg: unknown, ...args: unknown[]) =>
	levelLog("debug", msg, ...args);
wlogFn.trace = (msg: unknown, ...args: unknown[]) =>
	levelLog("trace", msg, ...args);
wlogFn.success = (msg: unknown, ...args: unknown[]) =>
	levelLog("success", msg, ...args);

// Utilities
wlogFn.smart = smartFormat;

wlogFn.table = (label: string, data: unknown[]) => {
	if (!config.logSwitch) return;
	const caller = config.showCaller ? getCallerLocation() : "";
	console.groupCollapsed(
		`%c ${label} %c ${caller}`,
		`color: ${config.colors.info.text}; font-weight: bold;`,
		"color: #999;",
	);
	console.table(data);
	console.groupEnd();
};

wlogFn.divider = (char = "─", length = 60) => {
	if (config.logSwitch) console.log(char.repeat(length));
};

wlogFn.section = (title: string) => {
	if (!config.logSwitch) return;
	wlogFn.divider("═");
	console.log(` ${title.toUpperCase()} `);
	wlogFn.divider("═");
};

wlogFn.time = (label?: string) => config.logSwitch && console.time(label);
wlogFn.timeEnd = (label?: string) => config.logSwitch && console.timeEnd(label);
wlogFn.count = (label?: string) => config.logSwitch && console.count(label);
wlogFn.countReset = (label?: string) => config.logSwitch && console.countReset(label);
wlogFn.clear = () => config.logSwitch && console.clear();
wlogFn.assert = (condition?: boolean, ...args: unknown[]) =>
	config.logSwitch && console.assert(condition, ...args);

wlogFn.json = (label: string, data: unknown, indent = 2) => {
	if (!config.logSwitch) return;
	const caller = config.showCaller ? getCallerLocation() : "";
	console.log(
		`%c${label}%c ${caller}`,
		"color:#1976d2; font-weight:bold;",
		"color:#999;",
	);
	console.log(JSON.stringify(data, null, indent));
};

// Config methods
wlogFn.config = (newConfig: Partial<WLogConfig>) =>
	Object.assign(config, newConfig);
wlogFn.getConfig = () => Object.freeze({ ...config });
wlogFn.reset = () => Object.assign(config, DEFAULT_CONFIG);

wlogFn.setLevel = (level: LogLevel | keyof typeof LogLevel) => {
	config.level = typeof level === "string" ? LogLevel[level] : level;
};

wlogFn.setSwitch = (enabled: boolean) => {
	config.logSwitch = enabled;
};
wlogFn.setPrefix = (prefix: string) => {
	config.prefix = prefix;
};

// ============================================================================
// Export Singleton
// ============================================================================

export const wlog = wlogFn as WLogFn;
export default wlog;
