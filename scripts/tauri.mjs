import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";

const env = { ...process.env };

// Vite reads `.env` for the frontend, but the Rust build only sees real
// environment variables: `src-tauri/build.rs` forwards `VITE_SUPABASE_*` into
// the compile-time `ADAQ_SUPABASE_*` trust root that verifies a desktop
// session. Without this the app shows "Unable to verify this session".
const dotenv = readDotEnv(new URL("../.env", import.meta.url));
for (const [key, value] of Object.entries(dotenv)) {
	env[key] ??= value;
}

if (process.platform === "darwin") {
	const brewPrefix = spawnSync("brew", ["--prefix", "curl-impersonate"], {
		encoding: "utf8",
	});
	if (brewPrefix.status === 0) {
		const libDir = `${brewPrefix.stdout.trim()}/lib`;
		env.DYLD_LIBRARY_PATH = [libDir, env.DYLD_LIBRARY_PATH]
			.filter(Boolean)
			.join(":");
		env.RUSTFLAGS = [
			env.RUSTFLAGS,
			"-C link-arg=-Wl,-no_compact_unwind",
			`-C link-arg=-Wl,-rpath,${libDir}`,
		]
			.filter(Boolean)
			.join(" ");
	}
}

const command = process.platform === "win32" ? "tauri.cmd" : "tauri";
const result = spawnSync(command, process.argv.slice(2), {
	env,
	shell: process.platform === "win32",
	stdio: "inherit",
});

process.exit(result.status ?? 1);

function readDotEnv(url) {
	const values = {};
	let contents;
	try {
		contents = readFileSync(url, "utf8");
	} catch {
		return values;
	}
	for (const line of contents.split("\n")) {
		const trimmed = line.trim();
		if (!trimmed || trimmed.startsWith("#")) continue;
		const separator = trimmed.indexOf("=");
		if (separator <= 0) continue;
		const key = trimmed.slice(0, separator).trim();
		let value = trimmed.slice(separator + 1).trim();
		if (
			value.length > 1 &&
			((value.startsWith('"') && value.endsWith('"')) ||
				(value.startsWith("'") && value.endsWith("'")))
		) {
			value = value.slice(1, -1);
		}
		values[key] = value;
	}
	return values;
}