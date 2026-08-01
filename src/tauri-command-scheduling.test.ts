/// <reference types="node" />

import { readFileSync } from "node:fs";

test("workspace list commands do not block the Tauri main thread", () => {
	const source = readFileSync(
		new URL("../src-tauri/src/m3.rs", import.meta.url),
		"utf8",
	);

	for (const command of [
		"component_list",
		"snapshot_list",
		"snapshot_list_readable",
		"backtest_list",
		"validation_protocol_list",
		"validation_report_list",
	]) {
		expect(source).toMatch(
			new RegExp(`#\\[tauri::command\\]\\s+pub async fn ${command}\\(`),
		);
	}
});

test("Models list commands run blocking work off the Tauri main thread", () => {
	const sources = [
		[
			readFileSync(
				new URL("../src-tauri/src/m3.rs", import.meta.url),
				"utf8",
			),
			["component_list", "snapshot_list_readable"],
		],
		[
			readFileSync(
				new URL("../src-tauri/src/m8.rs", import.meta.url),
				"utf8",
			),
			["dataset_generation_list"],
		],
	] as const;

	for (const [source, commands] of sources) {
		for (const command of commands) {
			const start = source.indexOf(`pub async fn ${command}(`);
			const end = source.indexOf("\n#[tauri::command]", start);
			expect(source.slice(start, end)).toContain(
				"tauri::async_runtime::spawn_blocking",
			);
		}
	}
});
