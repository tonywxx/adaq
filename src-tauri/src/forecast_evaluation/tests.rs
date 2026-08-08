#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset_generation::{Attempt, AttemptStatus, DatasetGenerationRequest};
    use crate::forecast_signal_dataset::{
        close_time, hash, import_signal_archive, pack_signal_archive, write_rows,
    };
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

    #[test]
    fn future_close_return_uses_exact_horizon_and_stops_at_gaps() {
        let bars = [(0, 1), (1, 2), (2, 4), (6, 8), (7, 16)]
            .into_iter()
            .map(|(minute, close)| OhlcvBar {
                open_time_ms: minute * 60_000,
                open: Decimal::from(close),
                high: Decimal::from(close),
                low: Decimal::from(close),
                close: Decimal::from(close),
                base_volume: Decimal::ONE,
                quote_volume: Decimal::ONE,
            })
            .collect::<Vec<_>>();
        let gaps = [BarGap {
            start_time_ms: 180_000,
            end_time_ms: 360_000,
        }];
        assert_eq!(
            realize_future_close_returns(&bars, &gaps, 2).unwrap(),
            [Some(3.0), None, None, None, None]
        );
        assert_eq!(
            realize_future_close_returns(&bars, &gaps, 0).unwrap_err(),
            "forecast-evaluation-horizon-must-be-positive"
        );
    }

    #[test]
    fn future_close_up_uses_strict_ties_exact_horizon_and_stops_at_gaps() {
        let bars = [(0, 1), (1, 1), (2, 2), (6, 1), (7, 3)]
            .into_iter()
            .map(|(minute, close)| OhlcvBar {
                open_time_ms: minute * 60_000,
                open: Decimal::from(close),
                high: Decimal::from(close),
                low: Decimal::from(close),
                close: Decimal::from(close),
                base_volume: Decimal::ONE,
                quote_volume: Decimal::ONE,
            })
            .collect::<Vec<_>>();
        let gaps = [BarGap {
            start_time_ms: 180_000,
            end_time_ms: 360_000,
        }];
        assert_eq!(
            realize_future_close_up(&bars, &gaps, 1).unwrap(),
            [Some(0.0), Some(1.0), None, Some(1.0), None]
        );
        assert_eq!(
            realize_future_close_up(&bars, &gaps, 0).unwrap_err(),
            "forecast-evaluation-horizon-must-be-positive"
        );
    }

    #[test]
    fn expected_value_metrics_preserve_unavailable_rows_and_edge_cases() {
        let metrics = expected_value_metrics(&[(1.0, 2.0), (3.0, 4.0)], 5, 2).unwrap();
        assert_eq!(metrics.aligned_count, 2);
        assert_eq!(metrics.coverage, 0.4);
        assert_eq!(metrics.missingness, 0.6);
        assert_eq!(metrics.mae, Some(1.0));
        assert_eq!(metrics.rmse, Some(1.0));
        assert_eq!(metrics.mean_bias, Some(-1.0));
        assert_eq!(metrics.pearson_correlation, Some(1.0));
        assert_eq!(metrics.prediction_distribution.unwrap().minimum, 1.0);
        assert_eq!(metrics.realized_distribution.unwrap().maximum, 4.0);
        assert_eq!(
            expected_value_metrics(&[(1.0, 2.0)], 1, 0)
                .unwrap()
                .pearson_correlation,
            None
        );
    }

    #[test]
    fn probability_metrics_cover_bounds_losses_auc_and_calibration() {
        let metrics =
            probability_metrics(&[(0.0, 0.0), (0.25, 0.0), (0.75, 1.0), (1.0, 1.0)], 5, 1).unwrap();
        assert_eq!(metrics.aligned_count, 4);
        assert_eq!(metrics.coverage, 0.8);
        assert!((metrics.missingness - 0.2).abs() < f64::EPSILON);
        assert_eq!(metrics.brier_score, Some(0.03125));
        assert!(metrics.log_loss.is_some_and(f64::is_finite));
        assert_eq!(metrics.roc_auc, Some(1.0));
        let buckets = metrics.calibration.as_ref().unwrap();
        assert_eq!(buckets.len(), 10);
        assert_eq!(buckets[0].count, 1);
        assert_eq!(buckets[2].mean_prediction, Some(0.25));
        assert_eq!(buckets[7].observed_frequency, Some(1.0));
        assert_eq!(buckets[9].count, 1);
        assert!(
            probability_metrics(&[(0.0, 0.0), (1.0, 1.0)], 2, 0)
                .unwrap()
                .log_loss
                .unwrap()
                < 1.1e-15
        );
        assert!(
            probability_metrics(&[(0.0, 1.0), (1.0, 0.0)], 2, 0)
                .unwrap()
                .log_loss
                .unwrap()
                > 34.0
        );
        assert_eq!(
            probability_metrics(&[(0.5, 0.0), (0.5, 1.0)], 2, 0)
                .unwrap()
                .roc_auc,
            Some(0.5)
        );

        assert_eq!(
            probability_metrics(&[(-0.01, 0.0)], 1, 0).unwrap_err(),
            "forecast-evaluation-probability-is-out-of-bounds"
        );
        assert_eq!(
            probability_metrics(&[(f64::NAN, 0.0)], 1, 0).unwrap_err(),
            "forecast-evaluation-probability-is-out-of-bounds"
        );
        let single_class = probability_metrics(&[(0.2, 1.0), (0.8, 1.0)], 2, 0).unwrap();
        assert_eq!(single_class.roc_auc, None);
        assert_eq!(
            single_class
                .undefined_metrics
                .get("rocAuc")
                .map(String::as_str),
            Some("requires-both-realized-classes")
        );
    }

    #[test]
    fn evaluation_evidence_uses_the_most_conservative_segment_state() {
        let out = classify_evidence_state(
            100,
            200,
            &[
                vec![Some((0, 99)), Some((0, 50)), Some((0, 75))],
                vec![Some((0, 120)), Some((0, 50)), Some((0, 75))],
            ],
        );
        assert_eq!(out.segment_states, ["out-of-sample", "overlapping"]);
        assert_eq!(out.summary, "overlapping");
        let unknown = classify_evidence_state(100, 200, &[vec![None, Some((0, 50)), None]]);
        assert_eq!(unknown.summary, "unknown");
    }

    #[test]
    fn score_metrics_cover_ties_constants_windows_icir_and_quantiles() {
        let pairs = [(0.1, 1.0), (0.1, 2.0), (0.5, 4.0), (0.9, 8.0), (1.0, 16.0)];
        let metrics = score_metrics(&pairs, 5, 0, &[Some(0.5), Some(1.0)]).unwrap();
        assert!(metrics.pearson_ic.is_some());
        assert!(metrics.spearman_rank_ic.is_some());
        assert_eq!(metrics.quantiles.as_ref().unwrap().len(), 5);
        assert_eq!(metrics.quantiles.as_ref().unwrap()[0].count, 2);
        assert!(metrics.window_icir.is_some());

        let constant = score_metrics(&[(1.0, 0.0), (1.0, 1.0)], 2, 0, &[None]).unwrap();
        assert_eq!(constant.pearson_ic, None);
        assert_eq!(constant.spearman_rank_ic, None);
        assert_eq!(
            constant
                .undefined_metrics
                .get("pearsonIc")
                .map(String::as_str),
            Some("requires-two-non-constant-series")
        );
        assert_eq!(
            constant
                .undefined_metrics
                .get("windowIcir")
                .map(String::as_str),
            Some("requires-two-non-constant-window-ics")
        );
        assert_eq!(
            constant
                .undefined_metrics
                .get("quantiles")
                .map(String::as_str),
            Some("requires-at-least-five-aligned-samples")
        );
        let constant_quantiles = score_metrics(
            &[(1.0, 0.0), (1.0, 1.0), (1.0, 2.0), (1.0, 3.0), (1.0, 4.0)],
            5,
            0,
            &[],
        )
        .unwrap();
        assert_eq!(
            constant_quantiles
                .undefined_metrics
                .get("quantiles")
                .map(String::as_str),
            Some("requires-non-constant-score-series")
        );
    }

    #[test]
    fn score_scale_requires_exact_causal_provenance() {
        let output = adaq_component_tooling::ModelOutput {
            name: "score".into(),
            prediction_kind: adaq_component_tooling::PredictionKind::Score,
            forecast_target: adaq_component_tooling::ForecastTarget::Builtin {
                target: adaq_component_tooling::BuiltinForecastTarget::FutureCloseReturn,
            },
            value_scale: adaq_component_tooling::ForecastValueScale::ZScore {
                method: "training-zscore-v1".into(),
            },
            horizon_bars: 1,
        };
        let proven = serde_json::json!({
            "provenance": {
                "scaleProvenance": {
                    "kind": "training-frozen",
                    "transformId": "training-zscore-v1",
                    "referenceDistributionId": "train-2025-v1",
                    "parameters": {"ddof": 0}
                }
            }
        });
        assert_eq!(score_scale_provenance(&output, &[proven]).unwrap().len(), 1);
        assert_eq!(
            score_scale_provenance(&output, &[serde_json::json!({"provenance": {}})]).unwrap_err(),
            "forecast-evaluation-score-scale-provenance-is-unproven"
        );
    }

    #[test]
    fn score_values_enforce_percentile_and_declared_custom_bounds() {
        let mut output = adaq_component_tooling::ModelOutput {
            name: "score".into(),
            prediction_kind: adaq_component_tooling::PredictionKind::Score,
            forecast_target: adaq_component_tooling::ForecastTarget::Builtin {
                target: adaq_component_tooling::BuiltinForecastTarget::FutureCloseReturn,
            },
            value_scale: adaq_component_tooling::ForecastValueScale::Percentile,
            horizon_bars: 1,
        };
        assert!(validate_prediction_scale(&output, 0.0).is_ok());
        assert!(validate_prediction_scale(&output, 1.0).is_ok());
        assert_eq!(
            validate_prediction_scale(&output, 1.01).unwrap_err(),
            "forecast-evaluation-percentile-is-out-of-bounds"
        );
        output.value_scale = adaq_component_tooling::ForecastValueScale::Custom {
            id: "bounded".into(),
            version: "1.0.0".parse().unwrap(),
            description: "Bounded custom scale".into(),
            minimum: Some(-1.0),
            maximum: Some(1.0),
        };
        assert_eq!(
            validate_prediction_scale(&output, -1.1).unwrap_err(),
            "forecast-evaluation-custom-scale-is-out-of-bounds"
        );
        assert_eq!(
            validate_prediction_scale(&output, f64::NAN).unwrap_err(),
            "forecast-evaluation-prediction-is-non-finite"
        );
    }

    #[test]
    fn forecast_evaluation_identity_reuses_exact_evidence_only() {
        let content = serde_json::json!({
            "datasetId": "dataset",
            "signalName": "return",
            "evaluationStartTimeMs": 1,
            "evaluationEndTimeMs": 2,
            "metricVersions": {"expectedValue": "expected-value@1"},
            "producerSegments": [{"modelArtifact": {"sha256": "a".repeat(64)}}],
            "componentLock": [{"alias": "model", "archiveSha256": "a".repeat(64)}],
            "unavailableRows": [{"predictionTimeMs": 2, "reason": "future-label-unavailable"}]
        });
        let first = forecast_evaluation_identity(&content).unwrap();
        assert_eq!(first, forecast_evaluation_identity(&content).unwrap());
        let mut changed = content.clone();
        changed["evaluationEndTimeMs"] = 3.into();
        assert_ne!(first, forecast_evaluation_identity(&changed).unwrap());
        let mut changed_reference = content;
        changed_reference["producerSegments"][0]["modelArtifact"]["sha256"] = "b".repeat(64).into();
        assert_ne!(
            first,
            forecast_evaluation_identity(&changed_reference).unwrap()
        );
        let mut changed_lock = changed_reference;
        changed_lock["componentLock"][0]["archiveSha256"] = "b".repeat(64).into();
        assert_ne!(first, forecast_evaluation_identity(&changed_lock).unwrap());
    }

    #[test]
    fn forecast_evaluation_accepts_proven_native_score_evidence() {
        let (root, state, request) = setup("valid", "evaluation-incompatible");
        let attempt = published_attempt(&state, &request);
        let dataset_id = attempt.dataset_id.unwrap();
        let report = evaluate_forecast(
            &state,
            &ForecastEvaluationRequest {
                user_id: "alice".into(),
                dataset_id,
                snapshot_id: request.snapshot_id.clone(),
                signal_name: "next-close-score".into(),
                horizon_bars: 1,
                evaluation_start_time_ms: 3_600_000,
                evaluation_end_time_ms: 9 * 3_600_000,
                stability_window_bars: 2,
            },
        )
        .unwrap();
        assert_eq!(report.signal_name, "next-close-score");
        assert_eq!(report.metric_versions["score"], "single-instrument-score@1");
        assert_eq!(report.scale_provenance.len(), 1);
        assert_eq!(
            report.metrics.undefined_metrics["pearsonIc"],
            "requires-two-non-constant-series"
        );
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn forecast_evaluation_is_immutable_user_scoped_and_exportable() {
        let (root, state, request) = setup("valid", "evaluation-report");
        let (snapshot, bars) = state
            .snapshot_for_user("alice", &request.snapshot_id)
            .unwrap();
        let records = bars
            .iter()
            .enumerate()
            .map(|(index, bar)| {
                Ok((
                    "okx:BTC-USDT".to_owned(),
                    close_time(snapshot.interval, bar.open_time_ms)?,
                    close_time(snapshot.interval, bar.open_time_ms)?,
                    Some(vec![
                        index as f64 / 10.0,
                        index as f64 / 10.0,
                        0.5,
                        index as f64 / 10.0,
                        index as f64 - 2.5,
                        index as f64,
                        index as f64,
                    ]),
                    None,
                ))
            })
            .collect::<Result<Vec<_>, String>>()
            .unwrap();
        let parquet_path = root.join("evaluation-source.parquet");
        write_rows(&parquet_path, &records).unwrap();
        let parquet = fs::read(&parquet_path).unwrap();
        let mut manifest: serde_json::Value = serde_json::from_slice(&external_manifest(
            &request.snapshot_id,
            &parquet,
            3_600_000,
            9 * 3_600_000,
        ))
        .unwrap();
        manifest["signalContract"]["outputs"] = serde_json::json!([
            {
                "name": "future-return",
                "predictionKind": {"kind": "expected-value"},
                "forecastTarget": {"kind": "builtin", "target": "future-close-return"},
                "valueScale": {"kind": "native"},
                "horizonBars": 1
            },
            {
                "name": "future-up",
                "predictionKind": {"kind": "probability"},
                "forecastTarget": {"kind": "builtin", "target": "future-close-up"},
                "valueScale": {"kind": "probability"},
                "horizonBars": 1
            },
            {
                "name": "custom-binary",
                "predictionKind": {"kind": "probability"},
                "forecastTarget": {
                    "kind": "custom",
                    "id": "custom-binary",
                    "version": "1.0.0",
                    "description": "Externally realized binary target",
                    "valueType": "binary"
                },
                "valueScale": {"kind": "probability"},
                "horizonBars": 1
            },
            {
                "name": "return-score",
                "predictionKind": {"kind": "score"},
                "forecastTarget": {"kind": "builtin", "target": "future-close-return"},
                "valueScale": {"kind": "percentile"},
                "horizonBars": 1
            },
            {
                "name": "up-score",
                "predictionKind": {"kind": "score"},
                "forecastTarget": {"kind": "builtin", "target": "future-close-up"},
                "valueScale": {"kind": "z-score", "method": "evaluation-zscore-v1"},
                "horizonBars": 1
            },
            {
                "name": "custom-score",
                "predictionKind": {"kind": "score"},
                "forecastTarget": {
                    "kind": "custom",
                    "id": "custom-continuous",
                    "version": "1.0.0",
                    "description": "Externally realized continuous target",
                    "valueType": "continuous"
                },
                "valueScale": {"kind": "custom", "id": "raw-score", "version": "1.0.0", "description": "Stable raw score", "minimum": null, "maximum": null},
                "horizonBars": 1
            },
            {
                "name": "custom-prediction",
                "predictionKind": {"kind": "custom", "id": "custom-prediction", "version": "1.0.0", "description": "Inspectable custom prediction"},
                "forecastTarget": {"kind": "builtin", "target": "future-close-return"},
                "valueScale": {"kind": "custom", "id": "raw-score", "version": "1.0.0", "description": "Stable raw score", "minimum": null, "maximum": null},
                "horizonBars": 1
            }
        ]);
        for field in ["trainingWindow", "fittingWindow", "normalizationWindow"] {
            manifest["producerSegments"][0]["provenance"][field] = "0..0".into();
        }
        manifest["producerSegments"][0]["provenance"]["scaleProvenance"] = serde_json::json!({
            "kind": "training-frozen",
            "transformId": "evaluation-zscore-v1",
            "referenceDistributionId": "evaluation-training-v1",
            "parameters": {"ddof": 0}
        });
        let manifest = serde_json::to_vec(&manifest).unwrap();
        let archive = pack_signal_archive(&manifest, &parquet).unwrap();
        let dataset = import_signal_archive(&state, "alice", &archive).unwrap();
        let request = ForecastEvaluationRequest {
            user_id: "alice".into(),
            dataset_id: dataset["datasetId"].as_str().unwrap().into(),
            snapshot_id: request.snapshot_id.clone(),
            signal_name: "future-return".into(),
            horizon_bars: 1,
            evaluation_start_time_ms: 3_600_000,
            evaluation_end_time_ms: 9 * 3_600_000,
            stability_window_bars: 2,
        };
        let mut mismatch = request.clone();
        mismatch.snapshot_id = "different-snapshot".into();
        assert_eq!(
            evaluate_forecast(&state, &mismatch).unwrap_err(),
            "forecast-evaluation-snapshot-mismatch"
        );
        mismatch = request.clone();
        mismatch.horizon_bars = 2;
        assert_eq!(
            evaluate_forecast(&state, &mismatch).unwrap_err(),
            "forecast-evaluation-horizon-mismatch"
        );
        let first = save_forecast_evaluation(&state, &request).unwrap();
        let replay = save_forecast_evaluation(&state, &request).unwrap();
        assert_eq!(first.report_id, replay.report_id);
        assert_eq!(first.metrics.aligned_count, 4);
        assert_eq!(first.metrics.unavailable_label_count, 2);
        assert_eq!(first.stability_windows.len(), 3);
        assert_eq!(first.stability_windows[1]["metrics"]["coverage"], 0.5);
        assert_eq!(first.evidence_state.summary, "out-of-sample");
        assert_eq!(first.unavailable_rows.len(), 2);
        assert_eq!(list_forecast_evaluations(&state, "alice").unwrap().len(), 1);
        assert!(list_forecast_evaluations(&state, "bob").unwrap().is_empty());
        assert!(
            export_forecast_evaluation(&state, "bob", &first.report_id, "json")
                .unwrap_err()
                .contains("not found")
        );
        let json = export_forecast_evaluation(&state, "alice", &first.report_id, "json").unwrap();
        assert!(json.contains("\"unavailableRows\""));
        assert!(json.contains("\"expectedValue\": \"expected-value@1\""));
        assert!(json.contains("\"componentLock\""));
        assert!(json.contains("\"featurePlanHash\""));
        let markdown =
            export_forecast_evaluation(&state, "alice", &first.report_id, "markdown").unwrap();
        assert!(markdown.contains("## Authoritative evidence"));
        assert!(markdown.contains("research-metrics.md"));
        assert!(markdown.contains(&first.report_id));
        let mut probability_request = request.clone();
        probability_request.signal_name = "future-up".into();
        let probability = save_forecast_evaluation(&state, &probability_request).unwrap();
        assert_ne!(probability.report_id, first.report_id);
        assert!(probability.metrics.brier_score.is_some());
        assert!(probability.metrics.log_loss.is_some());
        assert_eq!(probability.metrics.roc_auc, None);
        assert_eq!(probability.metrics.calibration.as_ref().unwrap().len(), 10);
        assert_eq!(
            probability
                .metric_versions
                .get("probability")
                .map(String::as_str),
            Some("binary-probability@1")
        );
        let probability_markdown =
            export_forecast_evaluation(&state, "alice", &probability.report_id, "markdown")
                .unwrap();
        assert!(probability_markdown.contains("## Probability metrics"));

        let mut score_request = request.clone();
        score_request.signal_name = "return-score".into();
        let score = save_forecast_evaluation(&state, &score_request).unwrap();
        assert!(score.metrics.pearson_ic.is_some());
        assert!(score.metrics.spearman_rank_ic.is_some());
        assert_eq!(score.metrics.quantiles.as_ref().unwrap().len(), 5);
        assert_eq!(score.scale_provenance.len(), 1);
        assert_eq!(score.metric_versions["score"], "single-instrument-score@1");
        let score_markdown =
            export_forecast_evaluation(&state, "alice", &score.report_id, "markdown").unwrap();
        assert!(score_markdown.contains("Single-Instrument time-series Score metrics"));

        score_request.signal_name = "up-score".into();
        let binary_score = save_forecast_evaluation(&state, &score_request).unwrap();
        assert_eq!(binary_score.metrics.pearson_ic, None);
        assert_eq!(
            binary_score
                .metrics
                .undefined_metrics
                .get("pearsonIc")
                .map(String::as_str),
            Some("requires-two-non-constant-series")
        );

        let mut custom_request = request.clone();
        custom_request.signal_name = "custom-binary".into();
        let custom = save_forecast_evaluation(&state, &custom_request).unwrap();
        assert!(custom.metrics.brier_score.is_none());
        assert!(custom.metrics.prediction_distribution.is_some());
        assert_eq!(
            custom
                .metrics
                .undefined_metrics
                .get("probabilityMetrics")
                .map(String::as_str),
            Some("requires-verifiable-realized-labels")
        );
        assert!(
            custom
                .unavailable_rows
                .iter()
                .all(|row| { row["reason"] == "target-specific-evaluator-unavailable" })
        );
        let mut custom_score_request = request.clone();
        custom_score_request.signal_name = "custom-score".into();
        let custom_score = save_forecast_evaluation(&state, &custom_score_request).unwrap();
        assert!(custom_score.metrics.prediction_distribution.is_some());
        assert!(custom_score.metrics.pearson_ic.is_none());
        let custom_score_markdown =
            export_forecast_evaluation(&state, "alice", &custom_score.report_id, "markdown")
                .unwrap();
        assert!(custom_score_markdown.contains("## Custom evidence"));
        assert!(!custom_score_markdown.contains("Score metrics"));
        custom_score_request.signal_name = "custom-prediction".into();
        let custom_prediction = save_forecast_evaluation(&state, &custom_score_request).unwrap();
        assert!(custom_prediction.metrics.prediction_distribution.is_some());
        assert!(custom_prediction.metrics.pearson_ic.is_none());
        assert_eq!(list_forecast_evaluations(&state, "alice").unwrap().len(), 7);

        let mut invalid_records = records.clone();
        invalid_records[0].3.as_mut().unwrap()[1] = 1.1;
        let invalid_path = root.join("invalid-probability.parquet");
        write_rows(&invalid_path, &invalid_records).unwrap();
        let invalid_parquet = fs::read(invalid_path).unwrap();
        let mut invalid_manifest: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
        invalid_manifest["parquetSha256"] = hash(&invalid_parquet).into();
        let invalid_archive = pack_signal_archive(
            &serde_json::to_vec(&invalid_manifest).unwrap(),
            &invalid_parquet,
        )
        .unwrap();
        let invalid_dataset = import_signal_archive(&state, "alice", &invalid_archive).unwrap();
        let mut invalid_request = probability_request.clone();
        invalid_request.dataset_id = invalid_dataset["datasetId"].as_str().unwrap().into();
        assert_eq!(
            evaluate_forecast(&state, &invalid_request).unwrap_err(),
            "forecast-evaluation-probability-is-out-of-bounds"
        );
        let mut changed = request.clone();
        changed.evaluation_end_time_ms = 8 * 3_600_000;
        assert_ne!(
            save_forecast_evaluation(&state, &changed)
                .unwrap()
                .report_id,
            first.report_id
        );
        let mut unavailable = request.clone();
        unavailable.evaluation_start_time_ms = 9 * 3_600_000;
        let unavailable = save_forecast_evaluation(&state, &unavailable).unwrap();
        assert_eq!(unavailable.metrics.aligned_count, 0);
        assert_eq!(unavailable.metrics.coverage, 0.0);
        assert_eq!(unavailable.metrics.missingness, 1.0);
        assert!(unavailable.metrics.prediction_distribution.is_none());
        assert!(unavailable.metrics.mae.is_none());
        assert_eq!(unavailable.unavailable_rows.len(), 1);
        let stored_path: String = state
            .database
            .lock()
            .unwrap()
            .query_row(
                "SELECT parquet_path FROM signal_dataset_content WHERE dataset_id = ?1",
                [&request.dataset_id],
                |row| row.get(0),
            )
            .unwrap();
        fs::write(stored_path, b"tampered").unwrap();
        assert_eq!(
            evaluate_forecast(&state, &request).unwrap_err(),
            "forecast-evaluation-dataset-content-hash-mismatch"
        );
        drop(state);
        fs::remove_dir_all(root).unwrap();
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

}
