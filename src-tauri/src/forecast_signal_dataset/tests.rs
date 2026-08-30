#[cfg(test)]
mod tests {
    use super::*;
    use crate::backtest::{BacktestRunRequest, SignalInstanceRequest};
    use crate::dataset_generation::{Attempt, AttemptStatus, DatasetGenerationRequest};
    use crate::forecast_evaluation::{ForecastEvaluationRequest, save_forecast_evaluation};
    use adaq_component_tooling::{ComponentManifest, ComponentPackage, pack_component};
    use adaq_data_core::{BarGap, BarInterval, BarSeries, OhlcvBar};
    use rust_decimal::Decimal;
    use std::{
        collections::HashMap,
        time::{Duration, Instant},
    };

    fn root(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "adaq-m8-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ))
    }

    fn model_package() -> Vec<u8> {
        let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/model/target/wasm32-unknown-unknown/debug/m8_model_fixture.wasm");
        assert!(
            fixture.is_file(),
            "build the model fixture with cargo component build"
        );
        let wasm = fs::read(fixture).unwrap();
        let mut manifest: ComponentManifest =
            serde_json::from_str(include_str!("../fixtures/model/manifest.json")).unwrap();
        let wasm_sha256 = hash(&wasm);
        manifest.wasm_sha256 = wasm_sha256.clone();
        manifest.model_artifact.as_mut().unwrap().sha256 = wasm_sha256;
        pack_component(manifest, &wasm).unwrap()
    }

    fn setup(
        mode: &str,
        name: &str,
    ) -> (
        std::path::PathBuf,
        Arc<LocalResearchState>,
        DatasetGenerationRequest,
    ) {
        let root = root(name);
        let state = LocalResearchState::open(&root).unwrap();
        let package = model_package();
        let model_archive_sha256 = ComponentPackage::read(&package).unwrap().archive_sha256;
        state.components.import("alice", &package).unwrap();
        let bars = [0, 1, 2, 6, 7, 8]
            .into_iter()
            .enumerate()
            .map(|(index, hour)| {
                let value = Decimal::from(i64::try_from(index + 1).unwrap());
                OhlcvBar {
                    open_time_ms: hour * 3_600_000,
                    open: value,
                    high: value,
                    low: value,
                    close: value,
                    base_volume: Decimal::ONE,
                    quote_volume: value,
                }
            })
            .collect();
        let snapshot = state
            .persist_snapshot_for_user(
                "alice",
                &BarSeries {
                    src: "okx".into(),
                    code: "BTC-USDT".into(),
                    interval: BarInterval::OneHour,
                    bars,
                    gaps: vec![BarGap {
                        start_time_ms: 3 * 3_600_000,
                        end_time_ms: 6 * 3_600_000,
                    }],
                },
            )
            .unwrap();
        (
            root,
            state,
            DatasetGenerationRequest {
                user_id: "alice".into(),
                snapshot_id: snapshot.snapshot_id,
                model_archive_sha256,
                model_parameters: HashMap::from([("mode".into(), mode.into())]),
                factor_instances: vec![],
                seed: 7,
            },
        )
    }

    /// Starts generation through the lifecycle interface and waits for the
    /// published Completed Attempt.
    fn published_attempt(
        state: &LocalResearchState,
        request: &DatasetGenerationRequest,
    ) -> Attempt {
        let attempt = state.generation.start(request.clone()).unwrap();
        wait_for_attempt(
            state,
            &request.user_id,
            &attempt.attempt_id,
            AttemptStatus::Completed,
        )
    }

    fn wait_for_attempt(
        state: &LocalResearchState,
        user_id: &str,
        attempt_id: &str,
        expected: AttemptStatus,
    ) -> Attempt {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let attempt = state
                .generation
                .list(user_id)
                .unwrap()
                .into_iter()
                .find(|attempt| attempt.attempt_id == attempt_id)
                .unwrap();
            if attempt.status == expected {
                return attempt;
            }
            assert!(
                !matches!(
                    attempt.status,
                    AttemptStatus::Completed | AttemptStatus::Failed | AttemptStatus::Cancelled
                ),
                "Attempt {attempt_id} reached {:?} before {expected:?}",
                attempt.status
            );
            assert!(
                Instant::now() < deadline,
                "Attempt {attempt_id} did not reach {expected:?}: {:?}",
                attempt.status
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn dataset_parquet(state: &LocalResearchState, dataset_id: &str) -> Vec<u8> {
        let path: String = state
            .database
            .lock()
            .unwrap()
            .query_row(
                "SELECT parquet_path FROM signal_dataset_content WHERE dataset_id = ?1",
                [dataset_id],
                |row| row.get(0),
            )
            .unwrap();
        fs::read(path).unwrap()
    }

    #[test]
    fn closed_bar_assigns_source_close_boundary() {
        assert_eq!(
            close_time(adaq_data_core::BarInterval::OneMinute, 1_000),
            Ok(61_000)
        );
    }

    #[test]
    fn calendar_closed_bar_assigns_the_next_calendar_boundary() {
        assert_eq!(
            close_time(adaq_data_core::BarInterval::OneMonth, 1_704_067_200_000),
            Ok(1_706_745_600_000),
        );
    }

    #[test]
    fn parquet_publication_is_atomic_and_preserves_unavailable_evidence() {
        let path =
            std::env::temp_dir().join(format!("adaq-m8-{}-rows.parquet", std::process::id()));
        let rows = vec![
            (
                "okx:BTC-USDT".into(),
                60_000,
                60_000,
                None,
                Some("warmup".into()),
            ),
            (
                "okx:BTC-USDT".into(),
                120_000,
                120_000,
                Some(vec![0.25]),
                None,
            ),
        ];
        write_rows(&path, &rows).unwrap();
        assert!(path.is_file());
        assert!(!path.with_extension("parquet.tmp").exists());
        let builder = parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::try_new(
            std::fs::File::open(&path).unwrap(),
        )
        .unwrap();
        assert_eq!(
            builder
                .schema()
                .fields()
                .iter()
                .map(|field| field.name().as_str())
                .collect::<Vec<_>>(),
            [
                "instrument_id",
                "prediction_time_ms",
                "available_at_ms",
                "status",
                "forecast_json",
                "unavailable_reason"
            ]
        );
        std::fs::remove_file(path).unwrap();
    }

    fn external_manifest(snapshot_id: &str, parquet: &[u8], start: i64, end: i64) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1,
            "snapshotId": snapshot_id,
            "src": "okx",
            "code": "BTC-USDT",
            "interval": "1h",
            "parquetSha256": hash(parquet),
            "signalContract": { "outputs": [{ "name": "qlib-score", "predictionKind": { "kind": "score" }, "forecastTarget": { "kind": "builtin", "target": "future-close-return" }, "valueScale": { "kind": "custom", "id": "qlib-score", "version": "1.0.0", "description": "External Qlib score", "minimum": null, "maximum": null }, "horizonBars": 1 }] },
            "producerSegments": [{
                "startPredictionTimeMs": start,
                "endPredictionTimeMs": end,
                "modelArtifact": { "sha256": "a".repeat(64) },
                "inferenceConfiguration": { "batchSize": 256 },
                "availabilityPolicy": { "kind": "closed-bar@1" },
                "provenance": {
                    "sourceRevision": "unknown", "weightHash": "unknown", "tokenizerHash": "unknown", "normalizerHash": "unknown", "featureProcessorHash": "unknown", "architecture": "unknown", "frameworkRuntime": "unknown", "adapterVersion": "unknown", "licence": "unknown", "source": "unknown", "trainingWindow": "unknown", "fittingWindow": "unknown", "validationWindow": "unknown", "normalizationWindow": "unknown"
                }
            }]
        })).unwrap()
    }

    #[test]
    fn external_signal_archive_is_validated_published_and_round_trips() {
        let (root, state, request) = setup("valid", "external-archive");
        let attempt = published_attempt(&state, &request);
        let parquet = dataset_parquet(&state, attempt.dataset_id.as_deref().unwrap());
        let mut manifest_value: serde_json::Value = serde_json::from_slice(&external_manifest(
            &request.snapshot_id,
            &parquet,
            3_600_000,
            9 * 3_600_000,
        ))
        .unwrap();
        manifest_value["producerSegments"][0]["modelArtifact"]["sha256"] =
            request.model_archive_sha256.clone().into();
        let manifest = serde_json::to_vec(&manifest_value).unwrap();
        let archive = pack_signal_archive(&manifest, &parquet).unwrap();
        let imported = import_signal_archive(&state, "alice", &archive).unwrap();
        assert_eq!(imported["trust"], "externally-generated");
        assert_eq!(imported["predictionSource"], "external-import@1");
        let (stored_manifest, stored_parquet) = unpack_signal_archive(
            &export_signal_archive(&state, "alice", imported["datasetId"].as_str().unwrap())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(stored_manifest, manifest);
        assert_eq!(stored_parquet, parquet);
        let page =
            signal_rows_page(&state, "alice", imported["datasetId"].as_str().unwrap(), 1).unwrap();
        assert_eq!(page["total"], 6);
        assert_eq!(page["items"][0]["availableAtMs"], 3_600_000);
        assert_eq!(
            signal_rows_page(&state, "alice", imported["datasetId"].as_str().unwrap(), 2).unwrap()
                ["items"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
        assert_eq!(
            signal_rows_page(&state, "alice", imported["datasetId"].as_str().unwrap(), 0)
                .unwrap_err(),
            "Signal row page must be positive"
        );
        assert!(
            signal_rows_page(&state, "bob", imported["datasetId"].as_str().unwrap(), 1)
                .unwrap_err()
                .contains("not available")
        );
        assert!(
            state
                .components
                .delete("alice", &request.model_archive_sha256)
                .unwrap_err()
                .contains("immutable Signal Dataset")
        );
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn external_signal_archive_rejects_hashes_layout_and_segment_gaps() {
        let malformed = {
            let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
            writer
                .start_file("manifest.json", SimpleFileOptions::default())
                .unwrap();
            writer.write_all(b"{}").unwrap();
            writer
                .start_file("../signals.parquet", SimpleFileOptions::default())
                .unwrap();
            writer.write_all(b"x").unwrap();
            writer.finish().unwrap().into_inner()
        };
        assert_eq!(
            unpack_signal_archive(&malformed).unwrap_err(),
            "signal-archive-layout-is-invalid"
        );
        let manifest: ExternalSignalManifest =
            serde_json::from_slice(&external_manifest("snapshot", b"parquet", 10, 9)).unwrap();
        assert_eq!(
            validate_external_manifest(&manifest).unwrap_err(),
            "invalid-or-overlapping-producer-segments"
        );
        let mut value: serde_json::Value =
            serde_json::from_slice(&external_manifest("snapshot", b"parquet", 1, 2)).unwrap();
        let duplicate = value["producerSegments"][0].clone();
        value["producerSegments"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        let manifest: ExternalSignalManifest = serde_json::from_value(value).unwrap();
        assert_eq!(
            validate_external_manifest(&manifest).unwrap_err(),
            "invalid-or-overlapping-producer-segments"
        );
        let oversized = vec![0; MAX_SIGNAL_ARCHIVE_BYTES + 1];
        assert_eq!(
            unpack_signal_archive(&oversized).unwrap_err(),
            "signal-archive-size-is-invalid"
        );
        let archive =
            pack_signal_archive(&external_manifest("snapshot", b"wrong", 1, 2), b"parquet")
                .unwrap();
        let (manifest, parquet) = unpack_signal_archive(&archive).unwrap();
        let manifest: ExternalSignalManifest = serde_json::from_slice(&manifest).unwrap();
        assert_ne!(hash(&parquet), manifest.parquet_sha256);
    }

    #[test]
    fn external_rows_reject_schema_order_and_availability_violations() {
        let (root, state, request) = setup("valid", "external-rejections");
        let attempt = published_attempt(&state, &request);
        let parquet = dataset_parquet(&state, attempt.dataset_id.as_deref().unwrap());
        assert_eq!(
            read_external_rows(b"not parquet").unwrap_err(),
            "invalid-signals-parquet"
        );
        let wrong_schema = root.join("wrong-schema.parquet");
        let schema = std::sync::Arc::new(Schema::new(vec![Field::new(
            "wrong",
            DataType::Utf8,
            false,
        )]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![std::sync::Arc::new(StringArray::from_iter_values(["x"]))],
        )
        .unwrap();
        let mut writer =
            ArrowWriter::try_new(fs::File::create(&wrong_schema).unwrap(), schema, None).unwrap();
        writer.write(&batch).unwrap();
        writer.close().unwrap();
        assert_eq!(
            read_external_rows(&fs::read(&wrong_schema).unwrap()).unwrap_err(),
            "signal-parquet-schema-mismatch"
        );
        let manifest: ExternalSignalManifest = serde_json::from_slice(&external_manifest(
            &request.snapshot_id,
            &parquet,
            3_600_000,
            9 * 3_600_000,
        ))
        .unwrap();
        let mut rows = read_external_rows(&parquet).unwrap();
        rows[1].prediction_time_ms = rows[0].prediction_time_ms;
        let (snapshot, bars) = state
            .snapshot_for_user("alice", &request.snapshot_id)
            .unwrap();
        assert_eq!(
            validate_external_rows(&rows, &manifest, &snapshot, &bars).unwrap_err(),
            "signal-row-identity-or-availability-is-invalid"
        );
        let mut rows = read_external_rows(&parquet).unwrap();
        rows[2].available_at_ms += 1;
        assert_eq!(
            validate_external_rows(&rows, &manifest, &snapshot, &bars).unwrap_err(),
            "signal-row-violates-availability-policy"
        );
        let mut rows = read_external_rows(&parquet).unwrap();
        rows[2].values = Some(vec![0.1, 0.2]);
        assert_eq!(
            validate_external_rows(&rows, &manifest, &snapshot, &bars).unwrap_err(),
            "signal-row-status-contract-is-invalid"
        );
        let mut contract: serde_json::Value = serde_json::from_slice(&external_manifest(
            &request.snapshot_id,
            &parquet,
            3_600_000,
            9 * 3_600_000,
        ))
        .unwrap();
        contract["signalContract"]["outputs"][0]["valueScale"] =
            serde_json::json!({ "kind": "percentile" });
        assert_eq!(
            validate_external_manifest(&serde_json::from_value(contract.clone()).unwrap())
                .unwrap_err(),
            "external-score-scale-provenance-is-unproven"
        );
        contract["producerSegments"][0]["provenance"]["scaleProvenance"] = serde_json::json!({
            "kind": "past-only-rolling",
            "transformId": "rolling-percentile-v1",
            "parameters": {"windowBars": 252, "minimumBars": 60}
        });
        assert!(validate_external_manifest(&serde_json::from_value(contract).unwrap()).is_ok());
        let mut contract: serde_json::Value = serde_json::from_slice(&external_manifest(
            &request.snapshot_id,
            &parquet,
            3_600_000,
            9 * 3_600_000,
        ))
        .unwrap();
        contract["signalContract"]["outputs"][0]["valueScale"] = serde_json::json!({ "kind": "custom", "id": "", "version": "1.0.0", "description": "", "minimum": 3.0, "maximum": 2.0 });
        assert!(
            validate_external_manifest(&serde_json::from_value(contract).unwrap())
                .unwrap_err()
                .starts_with("invalid-signal-contract:")
        );
        let mut manifest: ExternalSignalManifest = serde_json::from_slice(&external_manifest(
            &request.snapshot_id,
            &parquet,
            3_600_000,
            3_600_000,
        ))
        .unwrap();
        manifest.producer_segments[0].end_prediction_time_ms = 3_600_000;
        assert_eq!(
            validate_external_rows(
                &read_external_rows(&parquet).unwrap(),
                &manifest,
                &snapshot,
                &bars
            )
            .unwrap_err(),
            "present-signal-row-must-resolve-to-exactly-one-producer-segment"
        );
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_external_import_is_atomic() {
        let (root, state, request) = setup("valid", "external-atomic-failure");
        let attempt = published_attempt(&state, &request);
        let parquet = dataset_parquet(&state, attempt.dataset_id.as_deref().unwrap());
        let manifest = external_manifest(&request.snapshot_id, &parquet, 3_600_000, 9 * 3_600_000);
        let archive = pack_signal_archive(&manifest, &parquet).unwrap();
        let dataset_id = hash(&[manifest.as_slice(), parquet.as_slice()].concat());
        let count_before: i64 = state
            .database
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM signal_dataset_content", [], |row| {
                row.get(0)
            })
            .unwrap();
        state.database.lock().unwrap().execute_batch("CREATE TRIGGER reject_external_access BEFORE INSERT ON signal_dataset_access BEGIN SELECT RAISE(ABORT, 'forced publication failure'); END;").unwrap();
        assert!(
            import_signal_archive(&state, "alice", &archive)
                .unwrap_err()
                .contains("forced publication failure")
        );
        let count_after: i64 = state
            .database
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM signal_dataset_content", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count_after, count_before);
        assert!(
            !state
                .root
                .join("signal-datasets")
                .join(format!("{dataset_id}.parquet"))
                .exists()
        );
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn datasets_lock_their_component_artifacts() {
        let (root, state, request) = setup("valid", "dataset-lock");
        published_attempt(&state, &request);
        assert!(
            state
                .components
                .delete("alice", &request.model_archive_sha256)
                .unwrap_err()
                .contains("immutable Signal Dataset")
        );
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn kronos_fixture_reaches_import_evaluation_and_dataset_first_backtest() {
        let root = root("kronos-external-path");
        let state = LocalResearchState::open(&root).unwrap();
        let fixture_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../examples/external-models/kronos/fixtures");
        let fixture: serde_json::Value =
            serde_json::from_slice(&fs::read(fixture_root.join("snapshot.json")).unwrap()).unwrap();
        let bars = fixture["bars"]
            .as_array()
            .unwrap()
            .iter()
            .map(|bar| OhlcvBar {
                open_time_ms: bar["openTimeMs"].as_i64().unwrap(),
                open: bar["open"].as_str().unwrap().parse().unwrap(),
                high: bar["high"].as_str().unwrap().parse().unwrap(),
                low: bar["low"].as_str().unwrap().parse().unwrap(),
                close: bar["close"].as_str().unwrap().parse().unwrap(),
                base_volume: bar["baseVolume"].as_str().unwrap().parse().unwrap(),
                quote_volume: bar["quoteVolume"].as_str().unwrap().parse().unwrap(),
            })
            .collect::<Vec<_>>();
        let snapshot = state
            .persist_snapshot_for_user(
                "alice",
                &BarSeries {
                    src: "okx".into(),
                    code: "BTC-USDT".into(),
                    interval: BarInterval::OneHour,
                    bars: bars.clone(),
                    gaps: vec![],
                },
            )
            .unwrap();

        assert_eq!(fixture["snapshotId"], snapshot.snapshot_id);
        let archive = fs::read(fixture_root.join("kronos-fixture.adaq-signals")).unwrap();
        let (manifest, parquet) = unpack_signal_archive(&archive).unwrap();
        let manifest: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
        assert_eq!(
            manifest["producerSegments"][0]["inferenceConfiguration"]["seed"],
            7
        );
        assert_eq!(
            manifest["producerSegments"][0]["provenance"]["externallyGenerated"],
            true
        );
        let fixture_rows = read_external_rows(&parquet).unwrap();
        assert_eq!(
            fixture_rows[0].unavailable_reason.as_deref(),
            Some("warmup")
        );
        assert_eq!(fixture_rows[1].values, Some(vec![0.01, 0.02]));
        let dataset = import_signal_archive(&state, "alice", &archive).unwrap();
        assert_eq!(dataset["trust"], "externally-generated");
        let backtest_dataset = backtest_signal_datasets(
            &state,
            "alice",
            true,
            Some(&[dataset["datasetId"].as_str().unwrap().into()]),
        )
        .unwrap()
        .pop()
        .unwrap();
        assert!(is_sha256(&backtest_dataset.dataset_id));
        assert_eq!(
            backtest_dataset.outputs[0].name,
            "expected-close-return-1-bar"
        );
        assert_eq!(backtest_dataset.producer_segments.len(), 1);

        let evaluation = save_forecast_evaluation(
            &state,
            &ForecastEvaluationRequest {
                user_id: "alice".into(),
                dataset_id: dataset["datasetId"].as_str().unwrap().into(),
                snapshot_id: snapshot.snapshot_id.clone(),
                signal_name: "expected-close-return-1-bar".into(),
                horizon_bars: 1,
                evaluation_start_time_ms: 3_600_000,
                evaluation_end_time_ms: 14_400_000,
                stability_window_bars: 2,
            },
        )
        .unwrap();
        assert_eq!(evaluation.evidence_state.summary, "unknown");
        assert_eq!(evaluation.trust_state, "externally-generated");

        let wasm_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
            "fixtures/external-strategy/target/wasm32-unknown-unknown/debug/m5_external_strategy_fixture.wasm",
        );
        let wasm = fs::read(wasm_path).unwrap();
        let mut strategy: ComponentManifest = serde_json::from_value(serde_json::json!({
            "manifestSchemaVersion":"1.0.0",
            "componentId":"47474747-4747-4747-8747-474747474747",
            "version":"1.0.0",
            "name":"Kronos Fixture Signal Strategy",
            "kind":"strategy",
            "sdkVersion":"0.1.0",
            "abiVersion":"1.0.0",
            "featureSlots":[{"name":"close-change","source":{"kind":"signal","predictionKind":{"kind":"expected-value"},"forecastTarget":{"kind":"builtin","target":"future-close-return"},"valueScale":{"kind":"native"},"horizonBars":1}}]
        }))
        .unwrap();
        strategy.wasm_sha256 = hash(&wasm);
        let strategy = pack_component(strategy, &wasm).unwrap();
        let strategy_archive_sha256 = ComponentPackage::read(&strategy).unwrap().archive_sha256;
        state.components.import("alice", &strategy).unwrap();
        let run = state
            .backtests
            .run(crate::backtest::BacktestRunRequest {
                user_id: "alice".into(),
                snapshot_id: snapshot.snapshot_id,
                run_start_time_ms: None,
                run_end_time_ms: None,
                factor_instances: vec![],
                signal_instances: vec![crate::backtest::SignalInstanceRequest {
                    slot: "close-change".into(),
                    dataset_id: dataset["datasetId"].as_str().unwrap().into(),
                    signal_name: "expected-close-return-1-bar".into(),
                }],
                strategy_archive_sha256,
                strategy_parameters: HashMap::new(),
                initial_quote_allocation: 10_000.into(),
                execution_profile: adaq_backtest_core::ExecutionProfile {
                    maker_fee_rate: Decimal::ZERO,
                    taker_fee_rate: Decimal::ZERO,
                    adverse_slippage_rate: Decimal::ZERO,
                    rebalance_threshold: Decimal::ZERO,
                    price_increment: Decimal::ONE,
                    quantity_increment: Decimal::new(1, 4),
                    minimum_quantity: Decimal::new(1, 4),
                    risk_free_rate: Decimal::ZERO,
                    fill_policy: adaq_backtest_core::FillPolicy::Taker,
                },
                strategy_binding: None,
                risk_policy: None,
                seed: 0,
            })
            .unwrap();
        let run_provenance = run.provenance.unwrap();
        assert_eq!(run_provenance.dataset_lock.len(), 1);
        assert_eq!(run_provenance.dataset_lock[0].evidence_state, "unknown");
        assert_eq!(format!("{:?}", run_provenance.architecture), "SignalDriven");
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }

}
