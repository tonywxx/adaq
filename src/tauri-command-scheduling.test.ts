/// <reference types="node" />

import { readFileSync } from "node:fs";

test("workspace list commands do not block the Tauri main thread", () => {
	const sources = [
		[
			readFileSync(
				new URL("../src-tauri/src/local_research.rs", import.meta.url),
				"utf8",
			),
			["component_list", "backtest_list"],
			"pub async fn",
		],
		[
			readFileSync(new URL("../src-tauri/src/lib.rs", import.meta.url), "utf8"),
			[
				"validation_protocol_list",
				"validation_report_list",
				"snapshot_list",
				"snapshot_list_readable",
			],
			"async fn",
		],
	] as const;

	for (const [source, commands, signature] of sources) {
		for (const command of commands) {
			expect(source).toMatch(
				new RegExp(`#\\[tauri::command\\]\\s+${signature} ${command}\\(`),
			);
		}
	}
});

test("Models list commands run blocking work off the Tauri main thread", () => {
	const sources = [
		[
			readFileSync(
				new URL("../src-tauri/src/local_research.rs", import.meta.url),
				"utf8",
			),
			["component_list"],
		],
		[
			readFileSync(new URL("../src-tauri/src/lib.rs", import.meta.url), "utf8"),
			["dataset_generation_list", "snapshot_list_readable"],
		],
	] as const;

	for (const [source, commands] of sources) {
		for (const command of commands) {
			const start = source.indexOf(`fn ${command}(`);
			const end = source.indexOf("\n#[tauri::command]", start);
			expect(source.slice(start, end)).toContain(
				"tauri::async_runtime::spawn_blocking",
			);
		}
	}
});
