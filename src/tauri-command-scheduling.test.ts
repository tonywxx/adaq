/// <reference types="node" />

import { readFileSync } from "node:fs";

test("workspace list commands do not block the Tauri main thread", () => {
	const sources = [
		[
			readFileSync(new URL("../src-tauri/src/lib.rs", import.meta.url), "utf8"),
			[
				"component_list",
				"component_page",
				"backtest_list",
				"validation_protocol_list",
				"validation_report_list",
				"snapshot_list",
				"snapshot_list_readable",
				"feature_definition_list",
				"feature_definition_get",
				"feature_definition_preview",
				"feature_fitting_list",
				"feature_fitting_get",
				"feature_artifact_list",
				"feature_artifact_get",
				"feature_materialization_list",
				"feature_materialization_get",
				"feature_dataset_list",
				"feature_dataset_get",
				"feature_dataset_summary",
				"feature_dataset_rows",
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
			readFileSync(new URL("../src-tauri/src/lib.rs", import.meta.url), "utf8"),
			[
				"component_list",
				"dataset_generation_list",
				"snapshot_list_readable",
				"feature_definition_list",
				"feature_definition_preview",
				"feature_fitting_list",
				"feature_materialization_list",
				"feature_dataset_rows",
			],
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

test("Feature Attempt start commands enqueue instead of evaluating inline", () => {
	const lib = readFileSync(
		new URL("../src-tauri/src/lib.rs", import.meta.url),
		"utf8",
	);
	// Start and Retry return promptly: they persist a Pending Attempt and
	// notify the runner. The command body must delegate to the lifecycle
	// module and never evaluate Feature Observations itself.
	for (const [command, delegate] of [
		["feature_fitting_start", "start_fitting"],
		["feature_materialization_start", "start_materialization"],
		["feature_fitting_retry", "retry_fitting_attempt"],
		["feature_materialization_retry", "retry_materialization_attempt"],
	] as const) {
		const start = lib.indexOf(`fn ${command}(`);
		expect(start).toBeGreaterThan(-1);
		const end = lib.indexOf("\n#[tauri::command]", start);
		const body = lib.slice(start, end);
		expect(body).toContain(delegate);
		expect(body).not.toContain("evaluate_batch");
		expect(body).not.toContain(".observe(");
	}
	// Heavy Feature work runs on the dedicated FIFO runner thread, not on
	// any Tauri command thread.
	const features = readFileSync(
		new URL("../src-tauri/src/features/mod.rs", import.meta.url),
		"utf8",
	);
	expect(features).toContain('.name("adaq-feature-runner".into())');
	expect(features).toContain("runner::run_worker(inner)");
});
