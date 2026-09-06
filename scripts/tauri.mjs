import { spawnSync } from "node:child_process";

const env = { ...process.env };

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
