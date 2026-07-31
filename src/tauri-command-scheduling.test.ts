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
