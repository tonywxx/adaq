use std::{
    collections::BTreeMap,
    fs,
    io::{Cursor, Read, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use adaq_backtest_core::MarketDataSnapshot;
use adaq_component_sdk::host::model_abi;
use adaq_component_tooling::{
    ComponentKind, FactorInstancePlanInput, RunLimits, WasmLoader, component_parameters,
    native_engine_identity, validate_and_freeze_feature_plan_with_factors_and_parameters,
};
use arrow_array::{Array, ArrayRef, Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use parquet::arrow::ArrowWriter;
use rusqlite::{OptionalExtension, params};
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::Manager;
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

use crate::{
    m3::{M3State, validate_user},
    run_engine::{FactorRunRequest, MaterializedFeatureRow, materialize_feature_segment},
};

const DATASET_ENGINE: &str = "closed-bar@1";
const CHUNK_SIZE: usize = 256;
const SIGNAL_ARCHIVE_SCHEMA_VERSION: u32 = 1;
const MAX_SIGNAL_ARCHIVE_BYTES: usize = 64 * 1024 * 1024;
const MAX_SIGNAL_MANIFEST_BYTES: usize = 1024 * 1024;
static NEXT_ATTEMPT: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatasetGenerationRequest {
    pub user_id: String,
    pub snapshot_id: String,
    pub model_archive_sha256: String,
    #[serde(default)]
    pub model_parameters: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub factor_instances: Vec<DatasetFactorInstance>,
    #[serde(default)]
    pub seed: u64,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatasetFactorInstance {
    pub alias: String,
    pub archive_sha256: String,
    #[serde(default)]
    pub parameters: std::collections::HashMap<String, String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatasetGenerationAttempt {
    attempt_id: String,
    dataset_id: Option<String>,
    status: String,
    diagnostic_evidence: Option<String>,
    progress_completed: i64,
    progress_total: i64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SignalDataset {
    dataset_id: String,
    snapshot_id: String,
    src: String,
    code: String,
    interval: String,
    prediction_source: String,
    model_artifact: Option<adaq_component_tooling::ModelArtifact>,
    model_outputs: Vec<adaq_component_tooling::ModelOutput>,
    model_parameters: BTreeMap<String, adaq_component_tooling::ComponentParameterValue>,
    source_warmup_bars: u32,
    model_warmup_bars: u32,
    model_archive_sha256: String,
    trust: String,
    component_lock: Vec<ComponentLockEntry>,
    feature_plan_json: String,
    feature_plan_hash: String,
    seed: u64,
    engine_identity: adaq_component_tooling::EngineIdentity,
    producer_segments: Vec<ModelProducerSegment>,
    continuous_bar_segments: usize,
    bar_gap_rule: String,
    row_count: usize,
    unavailable_count: usize,
    status_counts: BTreeMap<String, usize>,
    parquet_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    archive_manifest_json: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    external_producer_segments: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExternalSignalManifest {
    schema_version: u32,
    snapshot_id: String,
    src: String,
    code: String,
    interval: String,
    parquet_sha256: String,
    signal_contract: serde_json::Value,
    producer_segments: Vec<ExternalProducerSegment>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExternalProducerSegment {
    start_prediction_time_ms: i64,
    end_prediction_time_ms: i64,
    model_artifact: serde_json::Value,
    inference_configuration: serde_json::Value,
    availability_policy: serde_json::Value,
    provenance: serde_json::Value,
    #[serde(default)]
    signal_contract: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelProducerSegment {
    start_prediction_time_ms: Option<i64>,
    end_prediction_time_ms: Option<i64>,
    model_archive_sha256: String,
    model_artifact: Option<adaq_component_tooling::ModelArtifact>,
    model_parameters: BTreeMap<String, adaq_component_tooling::ComponentParameterValue>,
    seed: u64,
    trust: String,
    engine_identity: adaq_component_tooling::EngineIdentity,
    feature_plan_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ComponentLockEntry {
    alias: String,
    archive_sha256: String,
}

#[derive(Debug)]
struct PendingDataset {
    metadata: SignalDataset,
    temporary_path: std::path::PathBuf,
    final_path: std::path::PathBuf,
}

struct PreparedAttempt {
    attempt: DatasetGenerationAttempt,
    should_start: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExternalRow {
    instrument_id: String,
    prediction_time_ms: i64,
    available_at_ms: i64,
    status: String,
    values: Option<Vec<f64>>,
    unavailable_reason: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct BacktestSignalRow {
    pub prediction_time_ms: i64,
    pub available_at_ms: i64,
    pub values: Option<Vec<f64>>,
}

#[derive(Debug, Clone)]
pub(crate) struct BacktestSignalDataset {
    pub dataset_id: String,
    pub snapshot_id: String,
    pub src: String,
    pub code: String,
    pub interval: String,
    pub outputs: Vec<adaq_component_tooling::ModelOutput>,
    pub producer_segments: Vec<serde_json::Value>,
    pub artifact_provenance: serde_json::Value,
    pub evidence_state: String,
    pub component_lock: Vec<serde_json::Value>,
    pub rows: Vec<BacktestSignalRow>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForecastEvaluationRequest {
    pub user_id: String,
    pub dataset_id: String,
    pub snapshot_id: String,
    pub signal_name: String,
    pub horizon_bars: u32,
    pub evaluation_start_time_ms: i64,
    pub evaluation_end_time_ms: i64,
    pub stability_window_bars: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DistributionMetrics {
    count: usize,
    minimum: f64,
    maximum: f64,
    mean: f64,
    standard_deviation: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CalibrationBucket {
    bucket_index: usize,
    lower_bound: f64,
    upper_bound: f64,
    count: usize,
    mean_prediction: Option<f64>,
    observed_frequency: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScoreQuantile {
    quantile: usize,
    count: usize,
    minimum_prediction: Option<f64>,
    maximum_prediction: Option<f64>,
    mean_prediction: Option<f64>,
    mean_realized_target: Option<f64>,
    realized_target_distribution: Option<DistributionMetrics>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum ScaleProvenance {
    TrainingFrozen {
        transform_id: String,
        reference_distribution_id: String,
        parameters: BTreeMap<String, serde_json::Value>,
    },
    PastOnlyRolling {
        transform_id: String,
        parameters: BTreeMap<String, serde_json::Value>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ForecastMetrics {
    evaluation_row_count: usize,
    aligned_count: usize,
    unavailable_prediction_count: usize,
    unavailable_label_count: usize,
    coverage: f64,
    missingness: f64,
    prediction_distribution: Option<DistributionMetrics>,
    realized_distribution: Option<DistributionMetrics>,
    mae: Option<f64>,
    rmse: Option<f64>,
    mean_bias: Option<f64>,
    pearson_correlation: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    brier_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    log_loss: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    roc_auc: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    calibration: Option<Vec<CalibrationBucket>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pearson_ic: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    spearman_rank_ic: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    window_icir: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    quantiles: Option<Vec<ScoreQuantile>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    undefined_metrics: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EvaluationEvidenceState {
    summary: String,
    segment_states: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForecastEvaluationReport {
    report_id: String,
    dataset_id: String,
    snapshot_id: String,
    signal_name: String,
    signal_contract: adaq_component_tooling::ModelOutput,
    evaluation_start_time_ms: i64,
    evaluation_end_time_ms: i64,
    stability_window_bars: usize,
    metrics: ForecastMetrics,
    stability_windows: Vec<serde_json::Value>,
    evidence_state: EvaluationEvidenceState,
    unavailable_rows: Vec<serde_json::Value>,
    producer_segments: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    scale_provenance: Vec<serde_json::Value>,
    trust_state: String,
    metric_versions: BTreeMap<String, String>,
    engine_identity: adaq_component_tooling::EngineIdentity,
    schema_identity: String,
    dataset_parquet_sha256: String,
    #[serde(default)]
    component_lock: Vec<ComponentLockEntry>,
    #[serde(default)]
    feature_plan_hash: String,
}

const SIGNAL_ROW_PAGE_SIZE: usize = 10;

fn unpack_signal_archive(bytes: &[u8]) -> Result<(Vec<u8>, Vec<u8>), String> {
    if bytes.is_empty() || bytes.len() > MAX_SIGNAL_ARCHIVE_BYTES {
        return Err("signal-archive-size-is-invalid".into());
    }
    let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(string)?;
    if archive.len() != 2 {
        return Err("signal-archive-must-contain-only-manifest-json-and-signals-parquet".into());
    }
    let mut names = (0..archive.len())
        .map(|index| {
            archive
                .by_index(index)
                .map(|file| file.name().to_owned())
                .map_err(string)
        })
        .collect::<Result<Vec<_>, _>>()?;
    names.sort();
    if names != ["manifest.json", "signals.parquet"] {
        return Err("signal-archive-layout-is-invalid".into());
    }
    let mut read = |name: &str, maximum: usize| -> Result<Vec<u8>, String> {
        let mut file = archive.by_name(name).map_err(string)?;
        if file.size() > maximum as u64 {
            return Err(format!("{name}-is-too-large"));
        }
        let mut content = Vec::with_capacity(file.size() as usize);
        file.read_to_end(&mut content).map_err(string)?;
        if content.len() > maximum {
            return Err(format!("{name}-is-too-large"));
        }
        Ok(content)
    };
    Ok((
        read("manifest.json", MAX_SIGNAL_MANIFEST_BYTES)?,
        read("signals.parquet", MAX_SIGNAL_ARCHIVE_BYTES)?,
    ))
}

fn pack_signal_archive(manifest: &[u8], parquet: &[u8]) -> Result<Vec<u8>, String> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644);
    writer
        .start_file("manifest.json", options)
        .map_err(string)?;
    writer.write_all(manifest).map_err(string)?;
    writer
        .start_file("signals.parquet", options)
        .map_err(string)?;
    writer.write_all(parquet).map_err(string)?;
    Ok(writer.finish().map_err(string)?.into_inner())
}

fn validate_external_manifest(manifest: &ExternalSignalManifest) -> Result<(), String> {
    if manifest.snapshot_id.is_empty()
        || manifest.src.is_empty()
        || manifest.code.is_empty()
        || manifest.interval.is_empty()
        || !is_sha256(&manifest.parquet_sha256)
        || !manifest.signal_contract.is_object()
        || manifest.producer_segments.is_empty()
    {
        return Err("invalid-signal-manifest-contract".into());
    }
    let outputs = manifest
        .signal_contract
        .get("outputs")
        .cloned()
        .ok_or("signal-contract-must-declare-outputs")
        .and_then(|value| {
            serde_json::from_value::<Vec<adaq_component_tooling::ModelOutput>>(value)
                .map_err(|_| "invalid-signal-contract".into())
        })?;
    if !(1..=64).contains(&outputs.len()) {
        return Err("invalid-signal-contract".into());
    }
    adaq_component_tooling::validate_model_outputs(&outputs)
        .map_err(|error| format!("invalid-signal-contract: {error}"))?;
    let mut previous_end = None;
    for segment in &manifest.producer_segments {
        if segment.start_prediction_time_ms > segment.end_prediction_time_ms
            || previous_end.is_some_and(|end| segment.start_prediction_time_ms <= end)
            || !segment.model_artifact.get("sha256").is_some_and(|value| {
                value
                    .as_str()
                    .is_some_and(|value| value == "unknown" || is_sha256(value))
            })
            || !segment
                .inference_configuration
                .as_object()
                .is_some_and(|value| !value.is_empty())
            || !segment
                .availability_policy
                .get("kind")
                .is_some_and(|value| value.is_string())
            || !segment.provenance.is_object()
        {
            return Err("invalid-or-overlapping-producer-segments".into());
        }
        if let Some(contract) = &segment.signal_contract {
            if contract != &manifest.signal_contract {
                return Err("producer-segments-must-share-one-signal-contract".into());
            }
        }
        match segment.availability_policy["kind"].as_str() {
            Some("closed-bar@1") => {}
            Some("delayed@1")
                if segment
                    .availability_policy
                    .get("delayMs")
                    .is_some_and(|value| value.as_i64().is_some_and(|value| value >= 0)) => {}
            _ => return Err("invalid-availability-policy".into()),
        }
        for field in [
            "sourceRevision",
            "weightHash",
            "tokenizerHash",
            "normalizerHash",
            "featureProcessorHash",
            "architecture",
            "frameworkRuntime",
            "adapterVersion",
            "licence",
            "source",
            "trainingWindow",
            "fittingWindow",
            "validationWindow",
            "normalizationWindow",
        ] {
            if !segment
                .provenance
                .get(field)
                .is_some_and(|value| value.as_str().is_some_and(|value| !value.trim().is_empty()))
            {
                return Err(format!("producer-provenance-missing-{field}"));
            }
        }
        previous_end = Some(segment.end_prediction_time_ms);
    }
    let segments = manifest
        .producer_segments
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
        .map_err(string)?;
    for output in &outputs {
        score_scale_provenance(output, &segments)
            .map_err(|_| "external-score-scale-provenance-is-unproven".to_owned())?;
    }
    Ok(())
}

fn read_external_rows(parquet: &[u8]) -> Result<Vec<ExternalRow>, String> {
    let builder = parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::try_new(
        bytes::Bytes::copy_from_slice(parquet),
    )
    .map_err(|_| "invalid-signals-parquet".to_owned())?;
    let names = builder
        .schema()
        .fields()
        .iter()
        .map(|field| (field.name().as_str(), field.data_type()))
        .collect::<Vec<_>>();
    let expected = [
        ("instrument_id", DataType::Utf8),
        ("prediction_time_ms", DataType::Int64),
        ("available_at_ms", DataType::Int64),
        ("status", DataType::Utf8),
        ("forecast_json", DataType::Utf8),
        ("unavailable_reason", DataType::Utf8),
    ];
    if names.len() != expected.len()
        || names.iter().zip(expected.iter()).any(
            |((name, kind), (expected_name, expected_kind))| {
                *name != *expected_name || *kind != expected_kind
            },
        )
    {
        return Err("signal-parquet-schema-mismatch".into());
    }
    let batches = builder.build().map_err(string)?;
    let mut rows = Vec::new();
    for batch in batches {
        let batch = batch.map_err(string)?;
        let instrument = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .ok_or("signal-parquet-schema-mismatch")?;
        let prediction = batch
            .column(1)
            .as_any()
            .downcast_ref::<Int64Array>()
            .ok_or("signal-parquet-schema-mismatch")?;
        let available = batch
            .column(2)
            .as_any()
            .downcast_ref::<Int64Array>()
            .ok_or("signal-parquet-schema-mismatch")?;
        let status = batch
            .column(3)
            .as_any()
            .downcast_ref::<StringArray>()
            .ok_or("signal-parquet-schema-mismatch")?;
        let forecast = batch
            .column(4)
            .as_any()
            .downcast_ref::<StringArray>()
            .ok_or("signal-parquet-schema-mismatch")?;
        let reason = batch
            .column(5)
            .as_any()
            .downcast_ref::<StringArray>()
            .ok_or("signal-parquet-schema-mismatch")?;
        for index in 0..batch.num_rows() {
            let values = if forecast.is_null(index) {
                None
            } else {
                Some(
                    serde_json::from_str::<Vec<f64>>(forecast.value(index))
                        .map_err(|_| "invalid-signal-forecast-json")?,
                )
            };
            rows.push(ExternalRow {
                instrument_id: instrument.value(index).into(),
                prediction_time_ms: prediction.value(index),
                available_at_ms: available.value(index),
                status: status.value(index).into(),
                values,
                unavailable_reason: (!reason.is_null(index)).then(|| reason.value(index).into()),
            });
        }
    }
    Ok(rows)
}

fn validate_external_rows(
    rows: &[ExternalRow],
    manifest: &ExternalSignalManifest,
    snapshot: &MarketDataSnapshot,
    bars: &[adaq_data_core::OhlcvBar],
) -> Result<(), String> {
    let output_count = manifest.signal_contract["outputs"]
        .as_array()
        .ok_or("invalid-signal-contract")?
        .len();
    if rows.len() != bars.len() {
        return Err("signal-rows-do-not-exactly-align-with-snapshot".into());
    }
    let instrument = format!("{}:{}", manifest.src, manifest.code);
    for (index, row) in rows.iter().enumerate() {
        let expected_time = close_time(snapshot.interval, bars[index].open_time_ms)?;
        if row.instrument_id != instrument
            || row.prediction_time_ms != expected_time
            || row.available_at_ms < row.prediction_time_ms
            || index > 0 && rows[index - 1].prediction_time_ms >= row.prediction_time_ms
        {
            return Err("signal-row-identity-or-availability-is-invalid".into());
        }
        match (&row.status[..], &row.values, &row.unavailable_reason) {
            ("present", Some(values), None)
                if values.len() == output_count && values.iter().all(|value| value.is_finite()) =>
            {
                if manifest
                    .producer_segments
                    .iter()
                    .filter(|segment| {
                        row.prediction_time_ms >= segment.start_prediction_time_ms
                            && row.prediction_time_ms <= segment.end_prediction_time_ms
                    })
                    .count()
                    != 1
                {
                    return Err(
                        "present-signal-row-must-resolve-to-exactly-one-producer-segment".into(),
                    );
                }
                let policy = manifest
                    .producer_segments
                    .iter()
                    .find(|segment| {
                        row.prediction_time_ms >= segment.start_prediction_time_ms
                            && row.prediction_time_ms <= segment.end_prediction_time_ms
                    })
                    .expect("validated above");
                let minimum = if policy.availability_policy["kind"] == "closed-bar@1" {
                    row.prediction_time_ms
                } else {
                    row.prediction_time_ms
                        .checked_add(
                            policy.availability_policy["delayMs"]
                                .as_i64()
                                .expect("validated policy"),
                        )
                        .ok_or("signal-availability-overflow")?
                };
                if row.available_at_ms != minimum {
                    return Err("signal-row-violates-availability-policy".into());
                }
            }
            ("unavailable", None, Some(_)) => {}
            _ => return Err("signal-row-status-contract-is-invalid".into()),
        }
    }
    Ok(())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[tauri::command]
pub fn dataset_generation_start(
    request: DatasetGenerationRequest,
    app: tauri::AppHandle,
    state: tauri::State<'_, M3State>,
) -> Result<DatasetGenerationAttempt, String> {
    start_generation(request, app, &state)
}

fn start_generation(
    request: DatasetGenerationRequest,
    app: tauri::AppHandle,
    state: &M3State,
) -> Result<DatasetGenerationAttempt, String> {
    validate_user(&request.user_id)?;
    let database = state.database.lock().map_err(string)?;
    let prepared = prepare_attempt(&database, &request)?;
    if !prepared.should_start {
        return Ok(prepared.attempt);
    }
    let attempt_id = prepared.attempt.attempt_id.clone();
    drop(database);
    let cancelled = Arc::new(AtomicBool::new(false));
    state
        .generation_attempts
        .lock()
        .map_err(string)?
        .insert(attempt_id.clone(), cancelled.clone());
    let task_id = attempt_id.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<M3State>();
        let started = state
            .database
            .lock()
            .map_err(string)
            .and_then(|database| {
                database
                    .execute(
                        "UPDATE dataset_generation_attempts SET status = 'running' WHERE attempt_id = ?1 AND status = 'pending'",
                        [&task_id],
                    )
                    .map(|changed| changed == 1)
                    .map_err(string)
            })
            .unwrap_or(false);
        if !started {
            if let Ok(mut attempts) = state.generation_attempts.lock() {
                attempts.remove(&task_id);
            }
            return;
        }
        let result = generate(&request, &state, &cancelled, &task_id);
        let final_result = (|| -> Result<(), String> {
            if cancelled.load(Ordering::Relaxed) {
                state.database.lock().map_err(string)?.execute(
                    "UPDATE dataset_generation_attempts SET status = 'cancelled' WHERE attempt_id = ?1 AND status IN ('pending', 'running')",
                    [&task_id],
                ).map_err(string)?;
                return Ok(());
            }
            match result {
                Ok(dataset) => {
                    publish_dataset(&state, &request.user_id, &task_id, &cancelled, dataset)
                }
                Err(error) => {
                    record_failure(&state, &task_id, &error)?;
                    Ok(())
                }
            }
        })();
        if let Err(error) = final_result {
            let _ = record_failure(&state, &task_id, &error);
            eprintln!("Dataset Generation Attempt {task_id} finalization failed: {error}");
        }
        if let Ok(mut attempts) = state.generation_attempts.lock() {
            attempts.remove(&task_id);
        }
    });
    Ok(prepared.attempt)
}

#[tauri::command]
pub fn dataset_generation_retry(
    attempt_id: String,
    user_id: String,
    app: tauri::AppHandle,
    state: tauri::State<'_, M3State>,
) -> Result<DatasetGenerationAttempt, String> {
    validate_user(&user_id)?;
    let request_json = state
        .database
        .lock()
        .map_err(string)?
        .query_row(
            "SELECT request_json FROM dataset_generation_attempts WHERE attempt_id = ?1 AND user_id = ?2 AND status IN ('failed', 'cancelled')",
            params![attempt_id, user_id],
            |row| row.get::<_, String>(0),
        )
        .map_err(|_| "Dataset Generation Attempt cannot be retried".to_owned())?;
    let request = serde_json::from_str(&request_json).map_err(string)?;
    start_generation(request, app, &state)
}

#[tauri::command]
pub async fn dataset_generation_list(
    user_id: String,
    app: tauri::AppHandle,
) -> Result<Vec<DatasetGenerationAttempt>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        validate_user(&user_id)?;
        let state = app.state::<M3State>();
        let database = state.database.lock().map_err(string)?;
        database.prepare("SELECT attempt_id, dataset_id, status, diagnostic_json, progress_completed, progress_total FROM dataset_generation_attempts WHERE user_id = ?1 ORDER BY created_at DESC")
            .map_err(string)?.query_map([user_id], |row| Ok(DatasetGenerationAttempt { attempt_id: row.get(0)?, dataset_id: row.get(1)?, status: row.get(2)?, diagnostic_evidence: row.get(3)?, progress_completed: row.get(4)?, progress_total: row.get(5)? }))
            .map_err(string)?.collect::<Result<Vec<_>, _>>().map_err(string)
    })
    .await
    .map_err(string)?
}

#[tauri::command]
pub fn dataset_generation_cancel(
    attempt_id: String,
    user_id: String,
    state: tauri::State<'_, M3State>,
) -> Result<(), String> {
    validate_user(&user_id)?;
    let database = state.database.lock().map_err(string)?;
    if !mark_cancelled(&database, &attempt_id, &user_id)? {
        return Err("Dataset Generation Attempt cannot be cancelled".into());
    }
    drop(database);
    if let Some(cancelled) = state
        .generation_attempts
        .lock()
        .map_err(string)?
        .get(&attempt_id)
    {
        cancelled.store(true, Ordering::Relaxed);
    }
    Ok(())
}

fn prepare_attempt(
    database: &rusqlite::Connection,
    request: &DatasetGenerationRequest,
) -> Result<PreparedAttempt, String> {
    let request_hash = hash(&canonical_request(request)?);
    if let Some((attempt_id, dataset_id, status, diagnostic, completed, total)) = database.query_row(
        "SELECT attempt_id, dataset_id, status, diagnostic_json, progress_completed, progress_total FROM dataset_generation_attempts WHERE request_hash = ?1 AND user_id = ?2 AND status IN ('pending', 'running', 'completed') ORDER BY created_at DESC LIMIT 1",
        params![request_hash, request.user_id],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, String>(2)?, row.get::<_, Option<String>>(3)?, row.get::<_, i64>(4)?, row.get::<_, i64>(5)?)),
    ).optional().map_err(string)? {
        return Ok(PreparedAttempt {
            attempt: DatasetGenerationAttempt { attempt_id, dataset_id, status, diagnostic_evidence: diagnostic, progress_completed: completed, progress_total: total },
            should_start: false,
        });
    }
    let attempt_id = new_attempt_id(&request_hash);
    database.execute(
        "INSERT INTO dataset_generation_attempts(attempt_id, request_hash, user_id, status, request_json) VALUES (?1, ?2, ?3, 'pending', ?4)",
        params![attempt_id, request_hash, request.user_id, serde_json::to_string(request).map_err(string)?],
    ).map_err(string)?;
    Ok(PreparedAttempt {
        attempt: DatasetGenerationAttempt {
            attempt_id,
            dataset_id: None,
            status: "pending".into(),
            diagnostic_evidence: None,
            progress_completed: 0,
            progress_total: 0,
        },
        should_start: true,
    })
}

fn mark_cancelled(
    database: &rusqlite::Connection,
    attempt_id: &str,
    user_id: &str,
) -> Result<bool, String> {
    database
        .execute(
            "UPDATE dataset_generation_attempts SET status = 'cancelled' WHERE attempt_id = ?1 AND user_id = ?2 AND status IN ('pending', 'running')",
            params![attempt_id, user_id],
        )
        .map(|changed| changed == 1)
        .map_err(string)
}

#[tauri::command]
pub fn signal_dataset_list(
    user_id: String,
    state: tauri::State<'_, M3State>,
) -> Result<Vec<serde_json::Value>, String> {
    validate_user(&user_id)?;
    let database = state.database.lock().map_err(string)?;
    database
        .prepare("SELECT c.metadata_json FROM signal_dataset_content c JOIN signal_dataset_access a USING(dataset_id) WHERE a.user_id = ?1 ORDER BY c.dataset_id")
        .map_err(string)?
        .query_map([user_id], |row| row.get::<_, String>(0))
        .map_err(string)?
        .map(|row| serde_json::from_str(&row.map_err(string)?).map_err(string))
        .collect()
}

#[tauri::command]
pub fn signal_dataset_get(
    dataset_id: String,
    user_id: String,
    state: tauri::State<'_, M3State>,
) -> Result<serde_json::Value, String> {
    validate_user(&user_id)?;
    let database = state.database.lock().map_err(string)?;
    let json: String = database
        .query_row(
            "SELECT c.metadata_json FROM signal_dataset_content c JOIN signal_dataset_access a USING(dataset_id) WHERE c.dataset_id = ?1 AND a.user_id = ?2",
            params![dataset_id, user_id],
            |row| row.get(0),
        )
        .map_err(|_| "Forecast Signal Dataset is not available to this User".to_owned())?;
    serde_json::from_str(&json).map_err(string)
}

#[tauri::command]
pub fn signal_dataset_rows(
    dataset_id: String,
    user_id: String,
    page: usize,
    state: tauri::State<'_, M3State>,
) -> Result<serde_json::Value, String> {
    signal_rows_page(&state, &user_id, &dataset_id, page)
}

fn signal_rows_page(
    state: &M3State,
    user_id: &str,
    dataset_id: &str,
    page: usize,
) -> Result<serde_json::Value, String> {
    validate_user(user_id)?;
    if page == 0 {
        return Err("Signal row page must be positive".into());
    }
    let path: String = state.database.lock().map_err(string)?.query_row(
        "SELECT c.parquet_path FROM signal_dataset_content c JOIN signal_dataset_access a USING(dataset_id) WHERE c.dataset_id = ?1 AND a.user_id = ?2",
        params![dataset_id, user_id], |row| row.get(0),
    ).map_err(|_| "Forecast Signal Dataset is not available to this User".to_owned())?;
    let rows = read_external_rows(&fs::read(path).map_err(string)?)?;
    let total = rows.len();
    let start = (page - 1).saturating_mul(SIGNAL_ROW_PAGE_SIZE).min(total);
    Ok(
        serde_json::json!({ "items": rows.into_iter().skip(start).take(SIGNAL_ROW_PAGE_SIZE).collect::<Vec<_>>(), "total": total, "page": page, "pageSize": SIGNAL_ROW_PAGE_SIZE }),
    )
}

#[tauri::command]
pub fn signal_dataset_import(
    user_id: String,
    archive: Vec<u8>,
    state: tauri::State<'_, M3State>,
) -> Result<serde_json::Value, String> {
    import_signal_archive(&state, &user_id, &archive)
}

#[tauri::command]
pub fn signal_dataset_export(
    dataset_id: String,
    user_id: String,
    state: tauri::State<'_, M3State>,
) -> Result<Vec<u8>, String> {
    export_signal_archive(&state, &user_id, &dataset_id)
}

fn export_signal_archive(
    state: &M3State,
    user_id: &str,
    dataset_id: &str,
) -> Result<Vec<u8>, String> {
    validate_user(&user_id)?;
    let database = state.database.lock().map_err(string)?;
    let (metadata_json, parquet_path): (String, String) = database.query_row(
        "SELECT c.metadata_json, c.parquet_path FROM signal_dataset_content c JOIN signal_dataset_access a USING(dataset_id) WHERE c.dataset_id = ?1 AND a.user_id = ?2",
        params![dataset_id, user_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).map_err(|_| "Forecast Signal Dataset is not available to this User".to_owned())?;
    drop(database);
    let metadata: SignalDataset = serde_json::from_str(&metadata_json).map_err(string)?;
    let manifest = metadata
        .archive_manifest_json
        .ok_or("Only externally generated Signal Datasets can be exported as .adaq-signals")?;
    let parquet = fs::read(parquet_path).map_err(string)?;
    if hash(&parquet) != metadata.parquet_sha256 {
        return Err("stored-signal-parquet-hash-mismatch".into());
    }
    pack_signal_archive(manifest.as_bytes(), &parquet)
}

fn import_signal_archive(
    state: &M3State,
    user_id: &str,
    archive_bytes: &[u8],
) -> Result<serde_json::Value, String> {
    validate_user(user_id)?;
    let (manifest_json, parquet) = unpack_signal_archive(archive_bytes)?;
    let manifest: ExternalSignalManifest = serde_json::from_slice(&manifest_json)
        .map_err(|error| format!("invalid-signal-manifest: {error}"))?;
    if manifest.schema_version != SIGNAL_ARCHIVE_SCHEMA_VERSION {
        return Err("unsupported-signal-archive-schema-version".into());
    }
    if hash(&parquet) != manifest.parquet_sha256 {
        return Err("signal-parquet-hash-mismatch".into());
    }
    validate_external_manifest(&manifest)?;
    let (snapshot, bars) = state.snapshot_for_user(user_id, &manifest.snapshot_id)?;
    if snapshot.src != manifest.src
        || snapshot.code != manifest.code
        || snapshot.interval.as_str() != manifest.interval
    {
        return Err("signal-snapshot-instrument-venue-or-interval-mismatch".into());
    }
    let rows = read_external_rows(&parquet)?;
    validate_external_rows(&rows, &manifest, &snapshot, &bars)?;
    let dataset_id = hash(&[manifest_json.as_slice(), parquet.as_slice()].concat());
    let directory = state.root.join("signal-datasets");
    fs::create_dir_all(&directory).map_err(string)?;
    let temporary_path = directory.join(format!(".{dataset_id}.import.tmp"));
    fs::write(&temporary_path, &parquet).map_err(string)?;
    let final_path = directory.join(format!("{dataset_id}.parquet"));
    let unavailable_count = rows.iter().filter(|row| row.values.is_none()).count();
    let status_counts = rows.iter().fold(BTreeMap::new(), |mut counts, row| {
        *counts.entry(row.status.clone()).or_insert(0) += 1;
        counts
    });
    let external_producer_segments = serde_json::to_value(&manifest.producer_segments)
        .map_err(string)?
        .as_array()
        .cloned();
    let metadata = SignalDataset {
        dataset_id: dataset_id.clone(),
        snapshot_id: manifest.snapshot_id,
        src: manifest.src,
        code: manifest.code,
        interval: manifest.interval,
        prediction_source: "external-import@1".into(),
        model_artifact: None,
        model_outputs: vec![],
        model_parameters: BTreeMap::new(),
        source_warmup_bars: 0,
        model_warmup_bars: 0,
        model_archive_sha256: "external".into(),
        trust: "externally-generated".into(),
        component_lock: vec![],
        feature_plan_json: "{}".into(),
        feature_plan_hash: "external".into(),
        seed: 0,
        engine_identity: native_engine_identity().map_err(string)?,
        producer_segments: vec![],
        continuous_bar_segments: 0,
        bar_gap_rule: "external-evidence@1".into(),
        row_count: rows.len(),
        unavailable_count,
        status_counts,
        parquet_sha256: manifest.parquet_sha256,
        archive_manifest_json: Some(String::from_utf8(manifest_json).map_err(string)?),
        external_producer_segments,
    };
    let metadata_json = serde_json::to_string(&metadata).map_err(string)?;
    let mut database = state.database.lock().map_err(string)?;
    let transaction = database.transaction().map_err(string)?;
    let mut created_final = false;
    let result = (|| -> Result<(), String> {
        if final_path.exists() {
            if hash(&fs::read(&final_path).map_err(string)?) != metadata.parquet_sha256 {
                return Err("existing-dataset-content-hash-mismatch".into());
            }
            fs::remove_file(&temporary_path).map_err(string)?;
        } else {
            fs::rename(&temporary_path, &final_path).map_err(string)?;
            created_final = true;
        }
        transaction.execute("INSERT OR IGNORE INTO signal_dataset_content(dataset_id, metadata_json, parquet_path) VALUES (?1, ?2, ?3)", params![dataset_id, metadata_json, final_path.to_string_lossy()]).map_err(string)?;
        transaction
            .execute(
                "INSERT OR IGNORE INTO signal_dataset_access(user_id, dataset_id) VALUES (?1, ?2)",
                params![user_id, metadata.dataset_id],
            )
            .map_err(string)?;
        transaction.commit().map_err(string)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary_path);
        if created_final {
            let _ = fs::remove_file(&final_path);
        }
    }
    result?;
    serde_json::to_value(metadata).map_err(string)
}

fn publish_dataset(
    state: &M3State,
    user_id: &str,
    attempt_id: &str,
    cancelled: &AtomicBool,
    pending: PendingDataset,
) -> Result<(), String> {
    let mut database = state.database.lock().map_err(string)?;
    let transaction = database.transaction().map_err(string)?;
    if cancelled.load(Ordering::Relaxed)
        || transaction
            .execute(
                "UPDATE dataset_generation_attempts SET status = 'completed', dataset_id = ?2 WHERE attempt_id = ?1 AND status = 'running'",
                params![attempt_id, pending.metadata.dataset_id],
            )
            .map_err(string)?
            == 0
    {
        transaction.commit().map_err(string)?;
        let _ = fs::remove_file(&pending.temporary_path);
        return Ok(());
    }

    let metadata_json = serde_json::to_string(&pending.metadata).map_err(string)?;
    let mut created_final = false;
    let publication = (|| -> Result<(), String> {
        if pending.final_path.is_file() {
            if hash(&fs::read(&pending.final_path).map_err(string)?)
                != pending.metadata.parquet_sha256
            {
                return Err("existing-dataset-content-hash-mismatch".into());
            }
            fs::remove_file(&pending.temporary_path).map_err(string)?;
        } else {
            fs::rename(&pending.temporary_path, &pending.final_path).map_err(string)?;
            created_final = true;
        }
        transaction.execute(
            "INSERT OR IGNORE INTO signal_dataset_content(dataset_id, metadata_json, parquet_path) VALUES (?1, ?2, ?3)",
            params![pending.metadata.dataset_id, metadata_json, pending.final_path.to_string_lossy()],
        ).map_err(string)?;
        transaction
            .execute(
                "INSERT OR IGNORE INTO signal_dataset_access(user_id, dataset_id) VALUES (?1, ?2)",
                params![user_id, pending.metadata.dataset_id],
            )
            .map_err(string)?;
        transaction.commit().map_err(string)
    })();
    if publication.is_err() {
        let _ = fs::remove_file(&pending.temporary_path);
        if created_final {
            let _ = fs::remove_file(&pending.final_path);
        }
    }
    publication
}

fn record_failure(state: &M3State, attempt_id: &str, error: &str) -> Result<(), String> {
    let evidence = error.chars().take(8_192).collect::<String>();
    state
        .database
        .lock()
        .map_err(string)?
        .execute(
            "UPDATE dataset_generation_attempts SET status = 'failed', diagnostic_json = ?2 WHERE attempt_id = ?1 AND status = 'running'",
            params![attempt_id, evidence],
        )
        .map_err(string)?;
    Ok(())
}

fn generate(
    request: &DatasetGenerationRequest,
    state: &M3State,
    cancelled: &AtomicBool,
    attempt_id: &str,
) -> Result<PendingDataset, String> {
    if cancelled.load(Ordering::Relaxed) {
        return Err("Dataset Generation Attempt cancelled".into());
    }
    let model = state.package_for_user(&request.user_id, &request.model_archive_sha256)?;
    if model.manifest.kind != ComponentKind::Model {
        return Err("Dataset generation requires a Model Component".into());
    }
    let parameters = component_parameters(&model.manifest, Some(&request.model_parameters))?;
    let named_model_parameters = model
        .manifest
        .parameters
        .iter()
        .zip(parameters.iter().cloned())
        .map(|(definition, value)| (definition.name.clone(), value))
        .collect::<BTreeMap<_, _>>();
    let model_warmup_bars = model.manifest.warmup_bars;
    let factor_packages = request
        .factor_instances
        .iter()
        .map(|factor| {
            let package = state.package_for_user(&request.user_id, &factor.archive_sha256)?;
            if package.manifest.kind != ComponentKind::Factor {
                return Err("Model-to-Model dependencies are not supported".into());
            }
            let parameters = component_parameters(&package.manifest, Some(&factor.parameters))?;
            Ok((factor, package, parameters))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let factor_inputs = factor_packages
        .iter()
        .map(|(factor, package, parameters)| FactorInstancePlanInput {
            alias: &factor.alias,
            manifest: &package.manifest,
            parameters: parameters.clone(),
        })
        .collect::<Vec<_>>();
    let identity = native_engine_identity().map_err(string)?;
    let plan = validate_and_freeze_feature_plan_with_factors_and_parameters(
        &model.manifest,
        &model.archive_sha256,
        &identity,
        &factor_inputs,
        &request
            .model_parameters
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    )
    .map_err(|error| format!("Feature Plan validation failed: {:?}", error.issues))?;
    let factor_paths = factor_packages
        .iter()
        .map(|(factor, package, _)| Ok((factor.alias.as_str(), state.runtime_component(package)?)))
        .collect::<Result<Vec<_>, String>>()?;
    let factor_runs = factor_paths
        .iter()
        .map(|(alias, path)| {
            Ok(FactorRunRequest {
                alias,
                path: path.to_str().ok_or("Factor runtime path is invalid")?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let (snapshot, bars) = state.snapshot_for_user(&request.user_id, &request.snapshot_id)?;
    state
        .database
        .lock()
        .map_err(string)?
        .execute(
            "UPDATE dataset_generation_attempts SET progress_total = ?2 WHERE attempt_id = ?1",
            params![
                attempt_id,
                i64::try_from(bars.len()).map_err(|_| "Dataset row count is too large")?
            ],
        )
        .map_err(string)?;
    let slots = plan
        .slot_names()
        .map(|name| model_abi::exports::adaq::model::api::FeatureSlot { name: name.into() })
        .collect::<Vec<_>>();
    let loader = WasmLoader::with_limits(RunLimits::default());
    let mut boundaries = snapshot
        .gaps
        .iter()
        .filter_map(|gap| {
            bars.iter()
                .position(|bar| bar.open_time_ms >= gap.start_time_ms)
        })
        .collect::<Vec<_>>();
    boundaries.push(bars.len());
    boundaries.dedup();
    let instrument_id = format!("{}:{}", snapshot.src, snapshot.code);
    let mut rows = vec![None; bars.len()];
    let mut output = vec![None; bars.len()];
    let mut model_warmup = vec![false; bars.len()];
    let mut unavailable_reasons = vec![None; bars.len()];
    let mut start = 0;
    for &end in &boundaries {
        if cancelled.load(Ordering::Relaxed) {
            return Err("Dataset Generation Attempt cancelled".into());
        }
        let segment = &bars[start..end];
        if segment.is_empty() {
            continue;
        }
        let features =
            materialize_feature_segment(&plan, &factor_runs, segment, RunLimits::default())
                .map_err(|error| format!("Feature materialization failed: {error:?}"))?;
        let mut present = Vec::new();
        for (offset, feature) in features.into_iter().enumerate() {
            let index = start + offset;
            match feature {
                MaterializedFeatureRow::Warmup => {
                    unavailable_reasons[index] = Some("warmup".into())
                }
                MaterializedFeatureRow::MissingInput { slot, source } => {
                    unavailable_reasons[index] = Some(format!("missing-input:{slot}:{source}"));
                }
                MaterializedFeatureRow::Present(values) => {
                    let row = model_abi::exports::adaq::model::api::PredictionRow {
                        instrument_id: instrument_id.clone(),
                        prediction_time_ms: close_time(
                            snapshot.interval,
                            bars[index].open_time_ms,
                        )?,
                        values,
                    };
                    rows[index] = Some(row.clone());
                    present.push((index, row));
                }
            }
        }
        loader.load_model_bytes(&model.wasm, slots.clone(), &parameters, request.seed)?;
        let mut model_input_count = 0usize;
        for chunk in present.chunks(CHUNK_SIZE) {
            if cancelled.load(Ordering::Relaxed) {
                return Err("Dataset Generation Attempt cancelled".into());
            }
            let forecasts =
                loader.process_model(chunk.iter().map(|(_, row)| row.clone()).collect())?;
            if forecasts.len() != chunk.len() {
                return Err("invalid-model-forecast-count".into());
            }
            for ((index, _), forecast) in chunk.iter().zip(forecasts) {
                model_warmup[*index] = model_input_count < model_warmup_bars as usize;
                model_input_count += 1;
                output[*index] = forecast;
            }
            if let Some((index, _)) = chunk.last() {
                state.database.lock().map_err(string)?.execute(
                    "UPDATE dataset_generation_attempts SET progress_completed = ?2 WHERE attempt_id = ?1",
                    params![attempt_id, i64::try_from(index + 1).map_err(|_| "Dataset progress is too large")?],
                ).map_err(string)?;
            }
        }
        start = end;
        state.database.lock().map_err(string)?.execute(
            "UPDATE dataset_generation_attempts SET progress_completed = ?2 WHERE attempt_id = ?1",
            params![attempt_id, i64::try_from(end).map_err(|_| "Dataset progress is too large")?],
        ).map_err(string)?;
    }
    let records = output
        .into_iter()
        .enumerate()
        .map(|(index, forecast)| {
            let prediction_time_ms = close_time(snapshot.interval, bars[index].open_time_ms)?;
            let (values, unavailable_reason) = match (rows[index].as_ref(), forecast) {
                (None, _) => (None, unavailable_reasons[index].clone()),
                (Some(row), Some(value))
                    if value.instrument_id != row.instrument_id
                        || value.prediction_time_ms != prediction_time_ms
                        || value.values.len() != model.manifest.model_outputs.len()
                        || value.values.iter().any(|value| !value.is_finite()) =>
                {
                    return Err(
                        "invalid-model-forecast: malformed or non-finite present output".into(),
                    );
                }
                (Some(_), _) if model_warmup[index] => (None, Some("model-warmup".into())),
                (Some(_), None) => (None, Some("model-unavailable".into())),
                (Some(_), Some(value)) => (Some(value.values), None),
            };
            Ok((
                instrument_id.clone(),
                prediction_time_ms,
                prediction_time_ms,
                values,
                unavailable_reason,
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    if cancelled.load(Ordering::Relaxed) {
        return Err("Dataset Generation Attempt cancelled".into());
    }
    let directory = state.root.join("signal-datasets");
    fs::create_dir_all(&directory).map_err(string)?;
    let temporary_path = directory.join(format!(".{attempt_id}.parquet.tmp"));
    if let Err(error) = write_rows(&temporary_path, &records) {
        let _ = fs::remove_file(&temporary_path);
        return Err(error);
    }
    let parquet_sha256 = match fs::read(&temporary_path) {
        Ok(bytes) => hash(&bytes),
        Err(error) => {
            let _ = fs::remove_file(&temporary_path);
            return Err(string(error));
        }
    };
    let mut component_lock = request
        .factor_instances
        .iter()
        .map(|factor| ComponentLockEntry {
            alias: factor.alias.clone(),
            archive_sha256: factor.archive_sha256.clone(),
        })
        .collect::<Vec<_>>();
    component_lock.sort_by(|left, right| left.alias.cmp(&right.alias));
    component_lock.insert(
        0,
        ComponentLockEntry {
            alias: "model".into(),
            archive_sha256: request.model_archive_sha256.clone(),
        },
    );
    let dataset_id = dataset_identity(
        &snapshot.snapshot_id,
        plan.plan_hash(),
        request.seed,
        &identity,
        &component_lock,
        &parquet_sha256,
    )
    .map_err(|error| {
        let _ = fs::remove_file(&temporary_path);
        error
    })?;
    if cancelled.load(Ordering::Relaxed) {
        let _ = fs::remove_file(&temporary_path);
        return Err("Dataset Generation Attempt cancelled".into());
    }
    let final_path = directory.join(format!("{dataset_id}.parquet"));
    let unavailable_count = records
        .iter()
        .filter(|(_, _, _, values, _)| values.is_none())
        .count();
    let status_counts = records.iter().fold(BTreeMap::new(), |mut counts, row| {
        let status = match (&row.3, row.4.as_deref()) {
            (Some(_), _) => "present",
            (_, Some(reason)) if reason.starts_with("missing-input:") => "missing-input",
            (_, Some(reason)) => reason,
            _ => "unavailable",
        };
        *counts.entry(status.to_owned()).or_insert(0) += 1;
        counts
    });
    let producer_segments = vec![ModelProducerSegment {
        start_prediction_time_ms: records.first().map(|row| row.1),
        end_prediction_time_ms: records.last().map(|row| row.1),
        model_archive_sha256: model.archive_sha256.clone(),
        model_artifact: model.manifest.model_artifact.clone(),
        model_parameters: named_model_parameters.clone(),
        seed: request.seed,
        trust: "verified-package".into(),
        engine_identity: identity.clone(),
        feature_plan_hash: plan.plan_hash().into(),
    }];
    let metadata = SignalDataset {
        dataset_id,
        snapshot_id: snapshot.snapshot_id,
        src: snapshot.src,
        code: snapshot.code,
        interval: snapshot.interval.as_str().into(),
        prediction_source: DATASET_ENGINE.into(),
        model_artifact: model.manifest.model_artifact,
        model_outputs: model.manifest.model_outputs,
        model_parameters: named_model_parameters,
        source_warmup_bars: plan.effective_warmup_bars(),
        model_warmup_bars,
        model_archive_sha256: model.archive_sha256,
        trust: "verified-package".into(),
        component_lock,
        feature_plan_json: String::from_utf8(plan.to_json()).map_err(string)?,
        feature_plan_hash: plan.plan_hash().into(),
        seed: request.seed,
        engine_identity: identity,
        producer_segments,
        continuous_bar_segments: boundaries.len(),
        bar_gap_rule: "recreate-state-at-each-continuous-bar-segment@1".into(),
        row_count: records.len(),
        unavailable_count,
        status_counts,
        parquet_sha256,
        archive_manifest_json: None,
        external_producer_segments: None,
    };
    Ok(PendingDataset {
        metadata,
        temporary_path,
        final_path,
    })
}

fn close_time(interval: adaq_data_core::BarInterval, open: i64) -> Result<i64, String> {
    adaq_data_core::next_bar_open_time_ms(open, interval).map_err(string)
}

fn write_rows(
    path: &std::path::Path,
    rows: &[(String, i64, i64, Option<Vec<f64>>, Option<String>)],
) -> Result<(), String> {
    let schema = std::sync::Arc::new(Schema::new(vec![
        Field::new("instrument_id", DataType::Utf8, false),
        Field::new("prediction_time_ms", DataType::Int64, false),
        Field::new("available_at_ms", DataType::Int64, false),
        Field::new("status", DataType::Utf8, false),
        Field::new("forecast_json", DataType::Utf8, true),
        Field::new("unavailable_reason", DataType::Utf8, true),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            std::sync::Arc::new(StringArray::from_iter_values(
                rows.iter().map(|row| row.0.as_str()),
            )) as ArrayRef,
            std::sync::Arc::new(Int64Array::from_iter_values(rows.iter().map(|row| row.1))),
            std::sync::Arc::new(Int64Array::from_iter_values(rows.iter().map(|row| row.2))),
            std::sync::Arc::new(StringArray::from_iter_values(rows.iter().map(|row| {
                if row.3.is_some() {
                    "present"
                } else {
                    "unavailable"
                }
            }))),
            std::sync::Arc::new(StringArray::from(
                rows.iter()
                    .map(|row| {
                        row.3.as_ref().map(|value| {
                            serde_json::to_string(value).expect("finite forecast serializes")
                        })
                    })
                    .collect::<Vec<_>>(),
            )),
            std::sync::Arc::new(StringArray::from(
                rows.iter().map(|row| row.4.as_deref()).collect::<Vec<_>>(),
            )),
        ],
    )
    .map_err(string)?;
    let file = fs::File::create(path).map_err(string)?;
    let mut writer = ArrowWriter::try_new(file, schema, None).map_err(string)?;
    writer.write(&batch).map_err(string)?;
    writer.close().map_err(string)?;
    Ok(())
}

fn realize_future_close_target(
    bars: &[adaq_data_core::OhlcvBar],
    gaps: &[adaq_data_core::BarGap],
    horizon_bars: u32,
    realize: impl Fn(f64, f64) -> f64,
) -> Result<Vec<Option<f64>>, String> {
    if horizon_bars == 0 {
        return Err("forecast-evaluation-horizon-must-be-positive".into());
    }
    let horizon = usize::try_from(horizon_bars).map_err(string)?;
    Ok(bars
        .iter()
        .enumerate()
        .map(|(index, origin)| {
            let future = bars.get(index.checked_add(horizon)?)?;
            if gaps.iter().any(|gap| {
                gap.start_time_ms > origin.open_time_ms && gap.end_time_ms <= future.open_time_ms
            }) {
                return None;
            }
            let origin = origin.close.to_f64()?;
            let future = future.close.to_f64()?;
            (origin.is_finite() && future.is_finite() && origin > 0.0 && future > 0.0)
                .then(|| realize(origin, future))
        })
        .collect())
}

fn realize_future_close_returns(
    bars: &[adaq_data_core::OhlcvBar],
    gaps: &[adaq_data_core::BarGap],
    horizon_bars: u32,
) -> Result<Vec<Option<f64>>, String> {
    realize_future_close_target(bars, gaps, horizon_bars, |origin, future| {
        future / origin - 1.0
    })
}

fn realize_future_close_up(
    bars: &[adaq_data_core::OhlcvBar],
    gaps: &[adaq_data_core::BarGap],
    horizon_bars: u32,
) -> Result<Vec<Option<f64>>, String> {
    realize_future_close_target(bars, gaps, horizon_bars, |origin, future| {
        f64::from(future > origin)
    })
}

fn distribution(values: &[f64]) -> Result<DistributionMetrics, String> {
    if values.is_empty() || values.iter().any(|value| !value.is_finite()) {
        return Err("forecast-evaluation-has-no-aligned-finite-rows".into());
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let variance = values
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / values.len() as f64;
    Ok(DistributionMetrics {
        count: values.len(),
        minimum: values.iter().copied().fold(f64::INFINITY, f64::min),
        maximum: values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        mean,
        standard_deviation: variance.sqrt(),
    })
}

fn pearson(pairs: &[(f64, f64)]) -> Option<f64> {
    if pairs.len() < 2 {
        return None;
    }
    let prediction_mean = pairs.iter().map(|pair| pair.0).sum::<f64>() / pairs.len() as f64;
    let realized_mean = pairs.iter().map(|pair| pair.1).sum::<f64>() / pairs.len() as f64;
    let numerator = pairs
        .iter()
        .map(|pair| (pair.0 - prediction_mean) * (pair.1 - realized_mean))
        .sum::<f64>();
    let prediction_sum = pairs
        .iter()
        .map(|pair| (pair.0 - prediction_mean).powi(2))
        .sum::<f64>();
    let realized_sum = pairs
        .iter()
        .map(|pair| (pair.1 - realized_mean).powi(2))
        .sum::<f64>();
    let denominator = (prediction_sum * realized_sum).sqrt();
    (denominator > 0.0).then_some(numerator / denominator)
}

fn expected_value_metrics(
    pairs: &[(f64, f64)],
    evaluation_row_count: usize,
    unavailable_prediction_count: usize,
) -> Result<ForecastMetrics, String> {
    if evaluation_row_count == 0
        || pairs.len() > evaluation_row_count
        || unavailable_prediction_count > evaluation_row_count
        || pairs
            .iter()
            .any(|pair| !pair.0.is_finite() || !pair.1.is_finite())
    {
        return Err("forecast-evaluation-metric-input-is-invalid".into());
    }
    let predictions = pairs.iter().map(|pair| pair.0).collect::<Vec<_>>();
    let realized = pairs.iter().map(|pair| pair.1).collect::<Vec<_>>();
    let errors = pairs.iter().map(|pair| pair.0 - pair.1).collect::<Vec<_>>();
    let coverage = pairs.len() as f64 / evaluation_row_count as f64;
    let errors = (!pairs.is_empty()).then(|| {
        let count = pairs.len() as f64;
        (
            errors.iter().map(|value| value.abs()).sum::<f64>() / count,
            (errors.iter().map(|value| value.powi(2)).sum::<f64>() / count).sqrt(),
            errors.iter().sum::<f64>() / count,
        )
    });
    Ok(ForecastMetrics {
        evaluation_row_count,
        aligned_count: pairs.len(),
        unavailable_prediction_count,
        unavailable_label_count: evaluation_row_count
            .saturating_sub(pairs.len() + unavailable_prediction_count),
        coverage,
        missingness: 1.0 - coverage,
        prediction_distribution: (!predictions.is_empty())
            .then(|| distribution(&predictions))
            .transpose()?,
        realized_distribution: (!realized.is_empty())
            .then(|| distribution(&realized))
            .transpose()?,
        mae: errors.map(|metrics| metrics.0),
        rmse: errors.map(|metrics| metrics.1),
        mean_bias: errors.map(|metrics| metrics.2),
        pearson_correlation: pearson(pairs),
        brier_score: None,
        log_loss: None,
        roc_auc: None,
        calibration: None,
        pearson_ic: None,
        spearman_rank_ic: None,
        window_icir: None,
        quantiles: None,
        undefined_metrics: BTreeMap::new(),
    })
}

fn average_ranks(values: &[f64]) -> Vec<f64> {
    let mut order = (0..values.len()).collect::<Vec<_>>();
    order.sort_by(|left, right| values[*left].total_cmp(&values[*right]));
    let mut ranks = vec![0.0; values.len()];
    let mut start = 0;
    while start < order.len() {
        let mut end = start + 1;
        while end < order.len() && values[order[start]] == values[order[end]] {
            end += 1;
        }
        let rank = (start + 1 + end) as f64 / 2.0;
        for index in &order[start..end] {
            ranks[*index] = rank;
        }
        start = end;
    }
    ranks
}

fn score_quantiles(pairs: &[(f64, f64)]) -> Result<Vec<ScoreQuantile>, String> {
    let mut ranked = pairs.to_vec();
    ranked.sort_by(|left, right| left.0.total_cmp(&right.0).then(left.1.total_cmp(&right.1)));
    let mut buckets = vec![Vec::new(); 5];
    let mut start = 0;
    while start < ranked.len() {
        let mut end = start + 1;
        while end < ranked.len() && ranked[start].0 == ranked[end].0 {
            end += 1;
        }
        let bucket = (start * 5 / ranked.len()).min(4);
        buckets[bucket].extend_from_slice(&ranked[start..end]);
        start = end;
    }
    buckets
        .into_iter()
        .enumerate()
        .map(|(index, values)| {
            let predictions = values.iter().map(|pair| pair.0).collect::<Vec<_>>();
            let realized = values.iter().map(|pair| pair.1).collect::<Vec<_>>();
            let count = values.len();
            Ok(ScoreQuantile {
                quantile: index + 1,
                count,
                minimum_prediction: predictions.first().copied(),
                maximum_prediction: predictions.last().copied(),
                mean_prediction: (count > 0)
                    .then(|| predictions.iter().sum::<f64>() / count as f64),
                mean_realized_target: (count > 0)
                    .then(|| realized.iter().sum::<f64>() / count as f64),
                realized_target_distribution: (count > 0)
                    .then(|| distribution(&realized))
                    .transpose()?,
            })
        })
        .collect()
}

fn score_metrics(
    pairs: &[(f64, f64)],
    evaluation_row_count: usize,
    unavailable_prediction_count: usize,
    window_ics: &[Option<f64>],
) -> Result<ForecastMetrics, String> {
    let mut metrics =
        expected_value_metrics(pairs, evaluation_row_count, unavailable_prediction_count)?;
    metrics.mae = None;
    metrics.rmse = None;
    metrics.mean_bias = None;
    metrics.pearson_correlation = None;
    metrics.pearson_ic = pearson(pairs);
    let prediction_ranks = average_ranks(&pairs.iter().map(|pair| pair.0).collect::<Vec<_>>());
    let target_ranks = average_ranks(&pairs.iter().map(|pair| pair.1).collect::<Vec<_>>());
    let rank_pairs = prediction_ranks
        .into_iter()
        .zip(target_ranks)
        .collect::<Vec<_>>();
    metrics.spearman_rank_ic = pearson(&rank_pairs);
    metrics.quantiles = Some(score_quantiles(pairs)?);
    if pairs.len() < 5 {
        metrics.undefined_metrics.insert(
            "quantiles".into(),
            "requires-at-least-five-aligned-samples".into(),
        );
    } else if pairs
        .first()
        .is_some_and(|first| pairs.iter().all(|pair| pair.0 == first.0))
    {
        metrics.undefined_metrics.insert(
            "quantiles".into(),
            "requires-non-constant-score-series".into(),
        );
    }
    if metrics.pearson_ic.is_none() {
        metrics.undefined_metrics.insert(
            "pearsonIc".into(),
            "requires-two-non-constant-series".into(),
        );
    }
    if metrics.spearman_rank_ic.is_none() {
        metrics.undefined_metrics.insert(
            "spearmanRankIc".into(),
            "requires-two-non-constant-series".into(),
        );
    }
    if !window_ics.is_empty() {
        let values = window_ics.iter().flatten().copied().collect::<Vec<_>>();
        if values.len() >= 2 {
            let mean = values.iter().sum::<f64>() / values.len() as f64;
            let deviation = (values
                .iter()
                .map(|value| (value - mean).powi(2))
                .sum::<f64>()
                / values.len() as f64)
                .sqrt();
            if deviation > 0.0 {
                metrics.window_icir = Some(mean / deviation);
            }
        }
        if metrics.window_icir.is_none() {
            metrics.undefined_metrics.insert(
                "windowIcir".into(),
                "requires-two-non-constant-window-ics".into(),
            );
        }
    }
    Ok(metrics)
}

fn probability_metrics(
    pairs: &[(f64, f64)],
    evaluation_row_count: usize,
    unavailable_prediction_count: usize,
) -> Result<ForecastMetrics, String> {
    if pairs.iter().any(|(prediction, realized)| {
        !prediction.is_finite()
            || !(0.0..=1.0).contains(prediction)
            || !matches!(*realized, 0.0 | 1.0)
    }) {
        return Err("forecast-evaluation-probability-is-out-of-bounds".into());
    }
    let mut metrics =
        expected_value_metrics(pairs, evaluation_row_count, unavailable_prediction_count)?;
    metrics.mae = None;
    metrics.rmse = None;
    metrics.mean_bias = None;
    metrics.pearson_correlation = None;
    if pairs.is_empty() {
        metrics.undefined_metrics.insert(
            "probabilityMetrics".into(),
            "requires-verifiable-realized-labels".into(),
        );
        return Ok(metrics);
    }
    let count = pairs.len() as f64;
    metrics.brier_score = Some(
        pairs
            .iter()
            .map(|(prediction, realized)| (prediction - realized).powi(2))
            .sum::<f64>()
            / count,
    );
    const LOG_LOSS_EPSILON: f64 = 1e-15;
    metrics.log_loss = Some(
        -pairs
            .iter()
            .map(|(prediction, realized)| {
                let prediction = prediction.clamp(LOG_LOSS_EPSILON, 1.0 - LOG_LOSS_EPSILON);
                realized * prediction.ln() + (1.0 - realized) * (1.0 - prediction).ln()
            })
            .sum::<f64>()
            / count,
    );
    let positives = pairs.iter().filter(|pair| pair.1 == 1.0).count();
    let negatives = pairs.len() - positives;
    if positives == 0 || negatives == 0 {
        metrics
            .undefined_metrics
            .insert("rocAuc".into(), "requires-both-realized-classes".into());
    } else {
        let mut ranked = pairs.to_vec();
        ranked.sort_by(|left, right| left.0.total_cmp(&right.0));
        let mut positive_rank_sum = 0.0;
        let mut start = 0;
        while start < ranked.len() {
            let mut end = start + 1;
            while end < ranked.len() && ranked[start].0 == ranked[end].0 {
                end += 1;
            }
            let average_rank = (start + 1 + end) as f64 / 2.0;
            positive_rank_sum += ranked[start..end]
                .iter()
                .filter(|pair| pair.1 == 1.0)
                .count() as f64
                * average_rank;
            start = end;
        }
        metrics.roc_auc = Some(
            (positive_rank_sum - (positives * (positives + 1) / 2) as f64)
                / (positives * negatives) as f64,
        );
    }
    metrics.calibration = Some(
        (0..10)
            .map(|bucket_index| {
                let values = pairs
                    .iter()
                    .filter(|(prediction, _)| {
                        ((*prediction * 10.0).floor() as usize).min(9) == bucket_index
                    })
                    .collect::<Vec<_>>();
                let bucket_count = values.len();
                CalibrationBucket {
                    bucket_index,
                    lower_bound: bucket_index as f64 / 10.0,
                    upper_bound: (bucket_index + 1) as f64 / 10.0,
                    count: bucket_count,
                    mean_prediction: (bucket_count > 0).then(|| {
                        values.iter().map(|pair| pair.0).sum::<f64>() / bucket_count as f64
                    }),
                    observed_frequency: (bucket_count > 0).then(|| {
                        values.iter().map(|pair| pair.1).sum::<f64>() / bucket_count as f64
                    }),
                }
            })
            .collect(),
    );
    Ok(metrics)
}

fn classify_evidence_state(
    evaluation_start_time_ms: i64,
    evaluation_end_time_ms: i64,
    segment_windows: &[Vec<Option<(i64, i64)>>],
) -> EvaluationEvidenceState {
    let segment_states = segment_windows
        .iter()
        .map(|windows| {
            if windows.len() != 3 || windows.iter().any(Option::is_none) {
                "unknown"
            } else if windows.iter().flatten().any(|(start, end)| {
                *start <= evaluation_end_time_ms && *end >= evaluation_start_time_ms
            }) {
                "overlapping"
            } else {
                "out-of-sample"
            }
            .to_owned()
        })
        .collect::<Vec<_>>();
    EvaluationEvidenceState {
        summary: conservative_evidence_state(&segment_states).into(),
        segment_states,
    }
}

fn conservative_evidence_state(states: &[String]) -> &'static str {
    if states.is_empty() {
        "unknown"
    } else if states.iter().any(|state| state == "overlapping") {
        "overlapping"
    } else if states.iter().any(|state| state == "unknown") {
        "unknown"
    } else {
        "out-of-sample"
    }
}

fn parse_provenance_window(value: Option<&serde_json::Value>) -> Option<(i64, i64)> {
    let value = value?.as_str()?.trim();
    if value.eq_ignore_ascii_case("unknown") || value.is_empty() {
        return None;
    }
    if let Ok(object) = serde_json::from_str::<serde_json::Value>(value) {
        let start = object.get("startTimeMs")?.as_i64()?;
        let end = object.get("endTimeMs")?.as_i64()?;
        return (start <= end).then_some((start, end));
    }
    let (start, end) = value.split_once("..")?;
    let range = (start.parse().ok()?, end.parse().ok()?);
    (range.0 <= range.1).then_some(range)
}

fn forecast_evaluation_identity(content: &serde_json::Value) -> Result<String, String> {
    serde_json::to_vec(content)
        .map(|canonical| hash(&canonical))
        .map_err(string)
}

fn dataset_outputs(
    dataset: &SignalDataset,
) -> Result<Vec<adaq_component_tooling::ModelOutput>, String> {
    if !dataset.model_outputs.is_empty() {
        return Ok(dataset.model_outputs.clone());
    }
    let manifest = dataset
        .archive_manifest_json
        .as_deref()
        .ok_or("forecast-evaluation-dataset-has-no-signal-contract")?;
    let manifest: ExternalSignalManifest = serde_json::from_str(manifest).map_err(string)?;
    serde_json::from_value(
        manifest
            .signal_contract
            .get("outputs")
            .cloned()
            .ok_or("forecast-evaluation-dataset-has-no-signal-contract")?,
    )
    .map_err(string)
}

pub(crate) fn backtest_signal_datasets(
    state: &M3State,
    user_id: &str,
    include_rows: bool,
    dataset_ids: Option<&[String]>,
) -> Result<Vec<BacktestSignalDataset>, String> {
    validate_user(user_id)?;
    let database = state.database.lock().map_err(string)?;
    let stored = if let Some(dataset_ids) = dataset_ids {
        let mut statement = database
            .prepare(
                "SELECT c.metadata_json, c.parquet_path FROM signal_dataset_content c JOIN signal_dataset_access a USING(dataset_id) WHERE a.user_id = ?1 AND c.dataset_id = ?2",
            )
            .map_err(string)?;
        dataset_ids
            .iter()
            .map(|dataset_id| {
                statement
                    .query_row(params![user_id, dataset_id], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })
                    .optional()
                    .map_err(string)
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect()
    } else {
        let mut statement = database
            .prepare(
                "SELECT c.metadata_json, c.parquet_path FROM signal_dataset_content c JOIN signal_dataset_access a USING(dataset_id) WHERE a.user_id = ?1 ORDER BY c.dataset_id",
            )
            .map_err(string)?;
        statement
            .query_map([user_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(string)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(string)?
    };
    drop(database);
    stored
        .into_iter()
        .map(|(metadata_json, path)| {
            let dataset: SignalDataset = serde_json::from_str(&metadata_json).map_err(string)?;
            let outputs = dataset_outputs(&dataset)?;
            let producer_segments = producer_segment_values(&dataset)?;
            let start = producer_segments
                .iter()
                .filter_map(|segment| segment.get("startPredictionTimeMs")?.as_i64())
                .min()
                .unwrap_or(i64::MIN);
            let end = producer_segments
                .iter()
                .filter_map(|segment| segment.get("endPredictionTimeMs")?.as_i64())
                .max()
                .unwrap_or(i64::MAX);
            let evidence_state = segment_evidence(&producer_segments, start, end).summary;
            let artifact_provenance = dataset
                .model_artifact
                .as_ref()
                .map(|artifact| serde_json::to_value(artifact).map_err(string))
                .transpose()?
                .unwrap_or_else(|| {
                    serde_json::json!({
                        "producerSegments": producer_segments,
                        "predictionSource": dataset.prediction_source,
                        "trust": dataset.trust,
                    })
                });
            let component_lock = dataset
                .component_lock
                .iter()
                .map(|entry| serde_json::to_value(entry).map_err(string))
                .collect::<Result<Vec<_>, _>>()?;
            let rows = if include_rows {
                let parquet = fs::read(path).map_err(string)?;
                if hash(&parquet) != dataset.parquet_sha256 {
                    return Err("stored-signal-parquet-hash-mismatch".into());
                }
                let rows = read_external_rows(&parquet)?;
                if rows.iter().any(|row| {
                    row.values.as_ref().is_some_and(|values| {
                        values.len() != outputs.len()
                            || values.iter().zip(&outputs).any(|(value, output)| {
                                validate_prediction_scale(output, *value).is_err()
                            })
                    })
                }) {
                    return Err("signal-dataset-has-invalid-present-values".into());
                }
                rows.into_iter()
                    .map(|row| BacktestSignalRow {
                        prediction_time_ms: row.prediction_time_ms,
                        available_at_ms: row.available_at_ms,
                        values: row.values,
                    })
                    .collect::<Vec<_>>()
            } else {
                vec![]
            };
            Ok(BacktestSignalDataset {
                dataset_id: dataset.dataset_id,
                snapshot_id: dataset.snapshot_id,
                src: dataset.src,
                code: dataset.code,
                interval: dataset.interval,
                outputs,
                producer_segments,
                artifact_provenance,
                evidence_state,
                component_lock,
                rows,
            })
        })
        .collect()
}

fn producer_segment_values(dataset: &SignalDataset) -> Result<Vec<serde_json::Value>, String> {
    if let Some(segments) = &dataset.external_producer_segments {
        return Ok(segments.clone());
    }
    dataset
        .producer_segments
        .iter()
        .map(|segment| serde_json::to_value(segment).map_err(string))
        .collect()
}

fn segment_evidence(
    segments: &[serde_json::Value],
    evaluation_start_time_ms: i64,
    evaluation_end_time_ms: i64,
) -> EvaluationEvidenceState {
    let segment_states = segments
        .iter()
        .filter_map(|segment| {
            let start = segment
                .get("startPredictionTimeMs")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(evaluation_start_time_ms);
            let end = segment
                .get("endPredictionTimeMs")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(evaluation_end_time_ms);
            if start > evaluation_end_time_ms || end < evaluation_start_time_ms {
                return None;
            }
            let provenance = segment
                .get("provenance")
                .or_else(|| segment.get("modelArtifact")?.get("provenance"));
            let windows = ["trainingWindow", "fittingWindow", "normalizationWindow"]
                .iter()
                .map(|field| parse_provenance_window(provenance?.get(field)))
                .collect::<Vec<_>>();
            Some(
                classify_evidence_state(
                    start.max(evaluation_start_time_ms),
                    end.min(evaluation_end_time_ms),
                    &[windows],
                )
                .summary,
            )
        })
        .collect::<Vec<_>>();
    EvaluationEvidenceState {
        summary: conservative_evidence_state(&segment_states).into(),
        segment_states,
    }
}

#[derive(Clone, Copy)]
enum ForecastEvaluator {
    ExpectedValue,
    Probability,
    Score,
    Custom,
}

fn custom_metrics(
    predictions: &[f64],
    evaluation_row_count: usize,
    unavailable_prediction_count: usize,
) -> Result<ForecastMetrics, String> {
    let mut metrics = probability_metrics(&[], evaluation_row_count, unavailable_prediction_count)?;
    metrics.prediction_distribution = (!predictions.is_empty())
        .then(|| distribution(predictions))
        .transpose()?;
    Ok(metrics)
}

fn segment_provenance(segment: &serde_json::Value) -> Option<&serde_json::Value> {
    segment
        .get("provenance")
        .or_else(|| segment.get("modelArtifact")?.get("provenance"))
}

fn score_scale_provenance(
    signal: &adaq_component_tooling::ModelOutput,
    segments: &[serde_json::Value],
) -> Result<Vec<serde_json::Value>, String> {
    if !matches!(
        signal.prediction_kind,
        adaq_component_tooling::PredictionKind::Score
    ) || matches!(
        signal.value_scale,
        adaq_component_tooling::ForecastValueScale::Custom { .. }
    ) {
        return Ok(Vec::new());
    }
    let expected_transform = match &signal.value_scale {
        adaq_component_tooling::ForecastValueScale::ZScore { method } => Some(method.as_str()),
        adaq_component_tooling::ForecastValueScale::Percentile => None,
        _ => return Err("forecast-evaluation-score-scale-is-invalid".into()),
    };
    segments
        .iter()
        .map(|segment| {
            let raw = segment_provenance(segment)
                .and_then(|value| value.get("scaleProvenance"))
                .ok_or("forecast-evaluation-score-scale-provenance-is-unproven")?;
            let value = match raw.as_str() {
                Some(raw) => serde_json::from_str(raw).map_err(|_| {
                    "forecast-evaluation-score-scale-provenance-is-unproven".to_owned()
                })?,
                None => raw.clone(),
            };
            let value = value.get(&signal.name).cloned().unwrap_or(value);
            let provenance: ScaleProvenance = serde_json::from_value(value)
                .map_err(|_| "forecast-evaluation-score-scale-provenance-is-unproven".to_owned())?;
            let (transform_id, parameters, causal) = match &provenance {
                ScaleProvenance::TrainingFrozen {
                    transform_id,
                    reference_distribution_id,
                    parameters,
                } => (
                    transform_id,
                    parameters,
                    !reference_distribution_id.trim().is_empty()
                        && reference_distribution_id != "unknown",
                ),
                ScaleProvenance::PastOnlyRolling {
                    transform_id,
                    parameters,
                } => (
                    transform_id,
                    parameters,
                    parameters
                        .get("windowBars")
                        .and_then(serde_json::Value::as_u64)
                        .is_some_and(|window| window > 0),
                ),
            };
            if transform_id.trim().is_empty()
                || transform_id == "unknown"
                || parameters.is_empty()
                || !expected_transform.is_none_or(|expected| transform_id == expected)
                || !causal
            {
                return Err("forecast-evaluation-score-scale-provenance-is-unproven".into());
            }
            serde_json::to_value(provenance).map_err(string)
        })
        .collect()
}

fn validate_prediction_scale(
    signal: &adaq_component_tooling::ModelOutput,
    prediction: f64,
) -> Result<(), String> {
    if !prediction.is_finite() {
        return Err("forecast-evaluation-prediction-is-non-finite".into());
    }
    let valid = match signal.value_scale {
        adaq_component_tooling::ForecastValueScale::Probability
        | adaq_component_tooling::ForecastValueScale::Percentile => {
            (0.0..=1.0).contains(&prediction)
        }
        adaq_component_tooling::ForecastValueScale::Custom {
            minimum, maximum, ..
        } => {
            minimum.is_none_or(|minimum| prediction >= minimum)
                && maximum.is_none_or(|maximum| prediction <= maximum)
        }
        _ => true,
    };
    valid.then_some(()).ok_or_else(|| {
        match signal.value_scale {
            adaq_component_tooling::ForecastValueScale::Probability => {
                "forecast-evaluation-probability-is-out-of-bounds"
            }
            adaq_component_tooling::ForecastValueScale::Percentile => {
                "forecast-evaluation-percentile-is-out-of-bounds"
            }
            _ => "forecast-evaluation-custom-scale-is-out-of-bounds",
        }
        .into()
    })
}

fn evaluate_forecast(
    state: &M3State,
    request: &ForecastEvaluationRequest,
) -> Result<ForecastEvaluationReport, String> {
    validate_user(&request.user_id)?;
    if request.evaluation_start_time_ms > request.evaluation_end_time_ms
        || request.stability_window_bars == 0
    {
        return Err("forecast-evaluation-window-is-invalid".into());
    }
    let (metadata_json, parquet_path): (String, String) = state
        .database
        .lock()
        .map_err(string)?
        .query_row(
            "SELECT c.metadata_json, c.parquet_path FROM signal_dataset_content c JOIN signal_dataset_access a USING(dataset_id) WHERE c.dataset_id = ?1 AND a.user_id = ?2",
            params![request.dataset_id, request.user_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| "forecast-evaluation-dataset-is-not-available-to-user".to_owned())?;
    let dataset: SignalDataset = serde_json::from_str(&metadata_json).map_err(string)?;
    if request.snapshot_id != dataset.snapshot_id {
        return Err("forecast-evaluation-snapshot-mismatch".into());
    }
    let outputs = dataset_outputs(&dataset)?;
    let (signal_index, signal) = outputs
        .iter()
        .enumerate()
        .find(|(_, output)| output.name == request.signal_name)
        .ok_or("forecast-evaluation-signal-was-not-found")?;
    let evaluator = match (
        &signal.prediction_kind,
        &signal.forecast_target,
        &signal.value_scale,
    ) {
        (
            adaq_component_tooling::PredictionKind::ExpectedValue,
            adaq_component_tooling::ForecastTarget::Builtin {
                target: adaq_component_tooling::BuiltinForecastTarget::FutureCloseReturn,
            },
            adaq_component_tooling::ForecastValueScale::Native,
        ) => ForecastEvaluator::ExpectedValue,
        (
            adaq_component_tooling::PredictionKind::Probability,
            adaq_component_tooling::ForecastTarget::Builtin {
                target: adaq_component_tooling::BuiltinForecastTarget::FutureCloseUp,
            },
            adaq_component_tooling::ForecastValueScale::Probability,
        ) => ForecastEvaluator::Probability,
        (
            adaq_component_tooling::PredictionKind::Score,
            adaq_component_tooling::ForecastTarget::Builtin { .. },
            adaq_component_tooling::ForecastValueScale::Percentile
            | adaq_component_tooling::ForecastValueScale::ZScore { .. }
            | adaq_component_tooling::ForecastValueScale::Custom { .. },
        ) => ForecastEvaluator::Score,
        (adaq_component_tooling::PredictionKind::Custom { .. }, _, _)
        | (_, adaq_component_tooling::ForecastTarget::Custom { .. }, _) => {
            ForecastEvaluator::Custom
        }
        _ => {
            return Err("forecast-evaluation-signal-contract-is-incompatible".into());
        }
    };
    if signal.horizon_bars == 0 {
        return Err("forecast-evaluation-signal-contract-is-incompatible".into());
    }
    if request.horizon_bars != signal.horizon_bars {
        return Err("forecast-evaluation-horizon-mismatch".into());
    }
    let (snapshot, bars) = state.snapshot_for_user(&request.user_id, &dataset.snapshot_id)?;
    if snapshot.snapshot_id != dataset.snapshot_id
        || snapshot.src != dataset.src
        || snapshot.code != dataset.code
        || snapshot.interval.as_str() != dataset.interval
    {
        return Err("forecast-evaluation-dataset-snapshot-mismatch".into());
    }
    let parquet = fs::read(parquet_path).map_err(string)?;
    if hash(&parquet) != dataset.parquet_sha256 {
        return Err("forecast-evaluation-dataset-content-hash-mismatch".into());
    }
    let rows = read_external_rows(&parquet)?;
    if rows.len() != bars.len() {
        return Err("forecast-evaluation-dataset-row-count-mismatch".into());
    }
    for row in &rows {
        let Some(values) = &row.values else { continue };
        if values.len() != outputs.len() {
            return Err("forecast-evaluation-dataset-signal-contract-mismatch".into());
        }
        let prediction = values[signal_index];
        validate_prediction_scale(signal, prediction)?;
    }
    let gaps = snapshot
        .gaps
        .iter()
        .map(|gap| adaq_data_core::BarGap {
            start_time_ms: gap.start_time_ms,
            end_time_ms: gap.end_time_ms,
        })
        .collect::<Vec<_>>();
    let labels = match evaluator {
        ForecastEvaluator::ExpectedValue => {
            realize_future_close_returns(&bars, &gaps, signal.horizon_bars)?
        }
        ForecastEvaluator::Probability => {
            realize_future_close_up(&bars, &gaps, signal.horizon_bars)?
        }
        ForecastEvaluator::Score => match signal.forecast_target {
            adaq_component_tooling::ForecastTarget::Builtin {
                target: adaq_component_tooling::BuiltinForecastTarget::FutureCloseReturn,
            } => realize_future_close_returns(&bars, &gaps, signal.horizon_bars)?,
            adaq_component_tooling::ForecastTarget::Builtin {
                target: adaq_component_tooling::BuiltinForecastTarget::FutureCloseUp,
            } => realize_future_close_up(&bars, &gaps, signal.horizon_bars)?,
            _ => unreachable!("custom targets use the common evaluator"),
        },
        ForecastEvaluator::Custom => vec![None; bars.len()],
    };
    let selected = rows
        .iter()
        .enumerate()
        .filter(|(_, row)| {
            row.prediction_time_ms >= request.evaluation_start_time_ms
                && row.prediction_time_ms <= request.evaluation_end_time_ms
        })
        .collect::<Vec<_>>();
    if selected.is_empty() {
        return Err("forecast-evaluation-window-has-no-dataset-rows".into());
    }
    let mut aligned = Vec::new();
    let mut available_predictions = Vec::new();
    let mut unavailable_rows = Vec::new();
    let mut unavailable_prediction_count = 0usize;
    for (index, row) in selected.iter().copied() {
        match (
            row.values
                .as_ref()
                .and_then(|values| values.get(signal_index)),
            labels[index],
        ) {
            (Some(prediction), Some(realized)) if prediction.is_finite() => {
                aligned.push((*prediction, realized));
                available_predictions.push(*prediction);
            }
            (Some(prediction), None) if prediction.is_finite() => {
                available_predictions.push(*prediction);
                unavailable_rows.push(serde_json::json!({
                    "predictionTimeMs": row.prediction_time_ms,
                    "reason": match evaluator {
                        ForecastEvaluator::ExpectedValue => "future-close-return-unavailable",
                        ForecastEvaluator::Probability => "future-close-up-unavailable",
                        ForecastEvaluator::Score => "score-target-unavailable",
                        ForecastEvaluator::Custom => "target-specific-evaluator-unavailable",
                    }
                }));
            }
            (None, _) => {
                unavailable_prediction_count += 1;
                unavailable_rows.push(serde_json::json!({
                    "predictionTimeMs": row.prediction_time_ms,
                    "reason": row.unavailable_reason.as_deref().unwrap_or("prediction-unavailable")
                }));
            }
            _ => return Err("forecast-evaluation-prediction-is-non-finite".into()),
        }
    }
    let stability_windows = selected
        .chunks(request.stability_window_bars)
        .map(|window| {
            let pairs = window
                .iter()
                .filter_map(|(index, row)| {
                    row.values
                        .as_ref()
                        .and_then(|values| values.get(signal_index))
                        .zip(labels[*index])
                        .map(|(prediction, realized)| (*prediction, realized))
                })
                .collect::<Vec<_>>();
            let unavailable_predictions = window
                .iter()
                .filter(|(_, row)| row.values.is_none())
                .count();
            let predictions = window
                .iter()
                .filter_map(|(_, row)| {
                    row.values
                        .as_ref()
                        .and_then(|values| values.get(signal_index))
                        .copied()
                })
                .collect::<Vec<_>>();
            let metrics = match evaluator {
                ForecastEvaluator::ExpectedValue => {
                    expected_value_metrics(&pairs, window.len(), unavailable_predictions)?
                }
                ForecastEvaluator::Probability => {
                    probability_metrics(&pairs, window.len(), unavailable_predictions)?
                }
                ForecastEvaluator::Score => {
                    score_metrics(&pairs, window.len(), unavailable_predictions, &[])?
                }
                ForecastEvaluator::Custom => {
                    custom_metrics(&predictions, window.len(), unavailable_predictions)?
                }
            };
            Ok(serde_json::json!({
                "startPredictionTimeMs": window.first().expect("window is non-empty").1.prediction_time_ms,
                "endPredictionTimeMs": window.last().expect("window is non-empty").1.prediction_time_ms,
                "metrics": metrics,
            }))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let producer_segments = producer_segment_values(&dataset)?;
    let scale_provenance = score_scale_provenance(signal, &producer_segments)?;
    let metrics = match evaluator {
        ForecastEvaluator::ExpectedValue => {
            expected_value_metrics(&aligned, selected.len(), unavailable_prediction_count)?
        }
        ForecastEvaluator::Probability => {
            probability_metrics(&aligned, selected.len(), unavailable_prediction_count)?
        }
        ForecastEvaluator::Score => {
            let window_ics = stability_windows
                .iter()
                .map(|window| window["metrics"]["pearsonIc"].as_f64())
                .collect::<Vec<_>>();
            score_metrics(
                &aligned,
                selected.len(),
                unavailable_prediction_count,
                &window_ics,
            )?
        }
        ForecastEvaluator::Custom => custom_metrics(
            &available_predictions,
            selected.len(),
            unavailable_prediction_count,
        )?,
    };
    let evidence_state = segment_evidence(
        &producer_segments,
        request.evaluation_start_time_ms,
        request.evaluation_end_time_ms,
    );
    let mut metric_versions: BTreeMap<String, String> = BTreeMap::from([
        ("coverage".into(), "coverage@1".into()),
        ("distribution".into(), "distribution@1".into()),
        (
            "timeWindowStability".into(),
            "non-overlapping-windows@1".into(),
        ),
    ]);
    match evaluator {
        ForecastEvaluator::ExpectedValue => {
            metric_versions.insert("expectedValue".into(), "expected-value@1".into());
        }
        ForecastEvaluator::Probability => {
            metric_versions.insert("probability".into(), "binary-probability@1".into());
            metric_versions.insert("calibration".into(), "equal-width-10-buckets@1".into());
        }
        ForecastEvaluator::Score => {
            metric_versions.insert("score".into(), "single-instrument-score@1".into());
            metric_versions.insert("quantiles".into(), "tie-preserving-five-quantiles@1".into());
            metric_versions.insert("windowIcir".into(), "non-overlapping-window-icir@1".into());
        }
        ForecastEvaluator::Custom => {}
    }
    let content = serde_json::json!({
        "datasetId": dataset.dataset_id,
        "snapshotId": dataset.snapshot_id,
        "signalName": request.signal_name,
        "signalContract": signal,
        "evaluationStartTimeMs": request.evaluation_start_time_ms,
        "evaluationEndTimeMs": request.evaluation_end_time_ms,
        "stabilityWindowBars": request.stability_window_bars,
        "metrics": metrics,
        "stabilityWindows": stability_windows,
        "evidenceState": evidence_state,
        "unavailableRows": unavailable_rows,
        "producerSegments": producer_segments,
        "scaleProvenance": scale_provenance,
        "trustState": dataset.trust,
        "metricVersions": metric_versions,
        "engineIdentity": dataset.engine_identity,
        "schemaIdentity": "forecast-evaluation-report@2",
        "datasetParquetSha256": dataset.parquet_sha256,
        "componentLock": dataset.component_lock,
        "featurePlanHash": dataset.feature_plan_hash,
    });
    let report_id = forecast_evaluation_identity(&content)?;
    let mut report = content;
    report
        .as_object_mut()
        .expect("evaluation content is an object")
        .insert("reportId".into(), report_id.into());
    serde_json::from_value(report).map_err(string)
}

fn save_forecast_evaluation(
    state: &M3State,
    request: &ForecastEvaluationRequest,
) -> Result<ForecastEvaluationReport, String> {
    let report = evaluate_forecast(state, request)?;
    let report_json = serde_json::to_string(&report).map_err(string)?;
    let mut database = state.database.lock().map_err(string)?;
    let transaction = database.transaction().map_err(string)?;
    transaction
        .execute(
            "INSERT OR IGNORE INTO forecast_evaluation_content(report_id, report_json) VALUES (?1, ?2)",
            params![report.report_id, report_json],
        )
        .map_err(string)?;
    transaction
        .execute(
            "INSERT OR IGNORE INTO forecast_evaluation_access(user_id, report_id) VALUES (?1, ?2)",
            params![request.user_id, report.report_id],
        )
        .map_err(string)?;
    transaction.commit().map_err(string)?;
    Ok(report)
}

#[tauri::command]
pub async fn forecast_evaluation_create(
    request: ForecastEvaluationRequest,
    state: tauri::State<'_, M3State>,
) -> Result<ForecastEvaluationReport, String> {
    save_forecast_evaluation(&state, &request)
}

#[tauri::command]
pub async fn forecast_evaluation_list(
    user_id: String,
    state: tauri::State<'_, M3State>,
) -> Result<Vec<ForecastEvaluationReport>, String> {
    list_forecast_evaluations(&state, &user_id)
}

fn list_forecast_evaluations(
    state: &M3State,
    user_id: &str,
) -> Result<Vec<ForecastEvaluationReport>, String> {
    validate_user(&user_id)?;
    state
        .database
        .lock()
        .map_err(string)?
        .prepare("SELECT c.report_json FROM forecast_evaluation_content c JOIN forecast_evaluation_access a USING(report_id) WHERE a.user_id = ?1 ORDER BY c.report_id")
        .map_err(string)?
        .query_map([user_id], |row| row.get::<_, String>(0))
        .map_err(string)?
        .map(|row| serde_json::from_str(&row.map_err(string)?).map_err(string))
        .collect()
}

#[tauri::command]
pub fn forecast_evaluation_export(
    report_id: String,
    user_id: String,
    format: String,
    state: tauri::State<'_, M3State>,
) -> Result<String, String> {
    export_forecast_evaluation(&state, &user_id, &report_id, &format)
}

fn export_forecast_evaluation(
    state: &M3State,
    user_id: &str,
    report_id: &str,
    format: &str,
) -> Result<String, String> {
    validate_user(&user_id)?;
    let report_json: String = state
        .database
        .lock()
        .map_err(string)?
        .query_row(
            "SELECT c.report_json FROM forecast_evaluation_content c JOIN forecast_evaluation_access a USING(report_id) WHERE c.report_id = ?1 AND a.user_id = ?2",
            params![report_id, user_id],
            |row| row.get(0),
        )
        .map_err(|_| "Forecast Evaluation Report was not found".to_owned())?;
    let report: ForecastEvaluationReport = serde_json::from_str(&report_json).map_err(string)?;
    match format {
        "json" => serde_json::to_string_pretty(&report).map_err(string),
        "markdown" => Ok(forecast_evaluation_markdown(&report)),
        _ => Err("Forecast Evaluation export format is invalid".into()),
    }
}

fn forecast_evaluation_markdown(report: &ForecastEvaluationReport) -> String {
    let custom_evidence = matches!(
        report.signal_contract.prediction_kind,
        adaq_component_tooling::PredictionKind::Custom { .. }
    ) || matches!(
        report.signal_contract.forecast_target,
        adaq_component_tooling::ForecastTarget::Custom { .. }
    );
    let specialized_metrics = if custom_evidence {
        "## Custom evidence\n\nCommon coverage, distribution, stability, and provenance are retained. No specialized evaluator is claimed.\n".into()
    } else {
        match report.signal_contract.prediction_kind {
        adaq_component_tooling::PredictionKind::ExpectedValue => format!(
            "## Expected Value metrics\n\n- MAE: {}\n- RMSE: {}\n- Mean bias: {}\n- Pearson correlation: {}\n",
            report
                .metrics
                .mae
                .map_or_else(|| "unavailable".into(), |value| value.to_string()),
            report
                .metrics
                .rmse
                .map_or_else(|| "unavailable".into(), |value| value.to_string()),
            report
                .metrics
                .mean_bias
                .map_or_else(|| "unavailable".into(), |value| value.to_string()),
            report
                .metrics
                .pearson_correlation
                .map_or_else(|| "unavailable".into(), |value| value.to_string()),
        ),
        adaq_component_tooling::PredictionKind::Probability => format!(
            "## Probability metrics\n\n- Brier Score: {}\n- Log Loss: {}\n- ROC AUC: {}\n- Calibration: {}\n- Undefined metrics: {}\n",
            report
                .metrics
                .brier_score
                .map_or_else(|| "unavailable".into(), |value| value.to_string()),
            report
                .metrics
                .log_loss
                .map_or_else(|| "unavailable".into(), |value| value.to_string()),
            report
                .metrics
                .roc_auc
                .map_or_else(|| "unavailable".into(), |value| value.to_string()),
            serde_json::to_string(&report.metrics.calibration).expect("calibration serializes"),
            serde_json::to_string(&report.metrics.undefined_metrics)
                .expect("diagnostics serialize"),
        ),
        adaq_component_tooling::PredictionKind::Score => format!(
            "## Single-Instrument time-series Score metrics\n\n- Pearson IC: {}\n- Spearman Rank IC: {}\n- Window ICIR: {}\n- Five-quantile realized Target evidence: {}\n\nThese are not cross-sectional IC, Strategy profitability, turnover, or a universal investment-quality score.\n",
            report
                .metrics
                .pearson_ic
                .map_or_else(|| "unavailable".into(), |value| value.to_string()),
            report
                .metrics
                .spearman_rank_ic
                .map_or_else(|| "unavailable".into(), |value| value.to_string()),
            report
                .metrics
                .window_icir
                .map_or_else(|| "unavailable".into(), |value| value.to_string()),
            report.metrics.undefined_metrics.get("quantiles").cloned().unwrap_or_else(||
                serde_json::to_string(&report.metrics.quantiles).expect("quantiles serialize")
            ),
        ),
        adaq_component_tooling::PredictionKind::Custom { .. } =>
            "## Custom evidence\n\nCommon coverage, distribution, stability, and provenance are retained. No specialized evaluator is claimed.\n".into(),
        }
    };
    format!(
        "# Forecast Evaluation Report\n\n- Report ID: `{}`\n- Dataset ID: `{}`\n- Snapshot ID: `{}`\n- Signal: `{}`\n- Evaluation window: `{}` to `{}`\n- Evidence state: `{}`\n- Trust state: `{}`\n- Schema: `{}`\n\n[Metric definitions](https://github.com/tonywxx/adaq/blob/main/docs/reference/research-metrics.md)\n\n## Common metrics\n\n- Coverage: {}\n- Missingness: {}\n\n{}\n## Authoritative evidence\n\n```json\n{}\n```\n",
        report.report_id,
        report.dataset_id,
        report.snapshot_id,
        report.signal_name,
        report.evaluation_start_time_ms,
        report.evaluation_end_time_ms,
        report.evidence_state.summary,
        report.trust_state,
        report.schema_identity,
        report.metrics.coverage,
        report.metrics.missingness,
        specialized_metrics,
        serde_json::to_string_pretty(report).expect("report serializes"),
    )
}

fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn new_attempt_id(request_hash: &str) -> String {
    let nonce = NEXT_ATTEMPT.fetch_add(1, Ordering::Relaxed);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    hash(format!("{request_hash}:{now}:{nonce}").as_bytes())
}

fn dataset_identity(
    snapshot_id: &str,
    feature_plan_hash: &str,
    seed: u64,
    identity: &adaq_component_tooling::EngineIdentity,
    component_lock: &[ComponentLockEntry],
    parquet_sha256: &str,
) -> Result<String, String> {
    serde_json::to_vec(&(
        snapshot_id,
        feature_plan_hash,
        seed,
        identity,
        DATASET_ENGINE,
        "verified-package",
        "recreate-state-at-each-continuous-bar-segment@1",
        1usize,
        component_lock,
        parquet_sha256,
    ))
    .map(|canonical| hash(&canonical))
    .map_err(string)
}

fn canonical_request(request: &DatasetGenerationRequest) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&(
        &request.user_id,
        &request.snapshot_id,
        &request.model_archive_sha256,
        request.model_parameters.iter().collect::<BTreeMap<_, _>>(),
        request
            .factor_instances
            .iter()
            .map(|factor| {
                (
                    &factor.alias,
                    &factor.archive_sha256,
                    factor.parameters.iter().collect::<BTreeMap<_, _>>(),
                )
            })
            .collect::<Vec<_>>(),
        request.seed,
    ))
    .map_err(string)
}
fn string(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use adaq_component_tooling::{ComponentManifest, ComponentPackage, pack_component};
    use adaq_data_core::{BarGap, BarInterval, BarSeries, OhlcvBar};
    use arrow_array::Array;
    use rust_decimal::Decimal;
    use std::collections::HashMap;

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

    fn setup(mode: &str, name: &str) -> (std::path::PathBuf, M3State, DatasetGenerationRequest) {
        let root = root(name);
        let state = M3State::open(&root).unwrap();
        let package = model_package();
        let model_archive_sha256 = ComponentPackage::read(&package).unwrap().archive_sha256;
        state.import_component("alice", &package).unwrap();
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
            .persist_snapshot(&BarSeries {
                src: "okx".into(),
                code: "BTC-USDT".into(),
                interval: BarInterval::OneHour,
                bars,
                gaps: vec![BarGap {
                    start_time_ms: 3 * 3_600_000,
                    end_time_ms: 6 * 3_600_000,
                }],
            })
            .unwrap();
        state
            .database
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO market_data_snapshot_access(user_id, snapshot_id) VALUES ('alice', ?1)",
                [&snapshot.snapshot_id],
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

    fn running_attempt(state: &M3State, request: &DatasetGenerationRequest) -> String {
        let database = state.database.lock().unwrap();
        let prepared = prepare_attempt(&database, request).unwrap();
        assert!(prepared.should_start);
        database
            .execute(
                "UPDATE dataset_generation_attempts SET status = 'running' WHERE attempt_id = ?1",
                [&prepared.attempt.attempt_id],
            )
            .unwrap();
        prepared.attempt.attempt_id
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
        let attempt = running_attempt(&state, &request);
        let pending = generate(&request, &state, &AtomicBool::new(false), &attempt).unwrap();
        let dataset_id = pending.metadata.dataset_id.clone();
        publish_dataset(&state, "alice", &attempt, &AtomicBool::new(false), pending).unwrap();
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
    #[test]
    fn dataset_identity_is_content_addressed() {
        let identity = native_engine_identity().unwrap();
        let lock = |factor: &str| {
            vec![
                ComponentLockEntry {
                    alias: "model".into(),
                    archive_sha256: "a".repeat(64),
                },
                ComponentLockEntry {
                    alias: "factor".into(),
                    archive_sha256: factor.repeat(64),
                },
            ]
        };
        let first =
            dataset_identity("snapshot", "plan", 7, &identity, &lock("b"), "parquet").unwrap();
        assert_eq!(
            first,
            dataset_identity("snapshot", "plan", 7, &identity, &lock("b"), "parquet").unwrap(),
        );
        assert_ne!(
            first,
            dataset_identity("snapshot", "plan", 7, &identity, &lock("c"), "parquet").unwrap(),
        );
    }

    #[test]
    fn request_identity_is_order_independent_and_user_scoped() {
        let request = |user: &str, parameters: HashMap<String, String>| DatasetGenerationRequest {
            user_id: user.into(),
            snapshot_id: "snapshot".into(),
            model_archive_sha256: "model".into(),
            model_parameters: parameters,
            factor_instances: vec![],
            seed: 7,
        };
        let first = request(
            "user-1",
            HashMap::from([("b".into(), "2".into()), ("a".into(), "1".into())]),
        );
        let reordered = request(
            "user-1",
            HashMap::from([("a".into(), "1".into()), ("b".into(), "2".into())]),
        );
        let other_user = request("user-2", reordered.model_parameters.clone());
        assert_eq!(
            canonical_request(&first).unwrap(),
            canonical_request(&reordered).unwrap()
        );
        assert_ne!(
            canonical_request(&first).unwrap(),
            canonical_request(&other_user).unwrap()
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

    #[test]
    fn cancellation_stops_before_component_or_snapshot_access() {
        let root = std::env::temp_dir().join(format!(
            "adaq-m8-{}-{}-cancel",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        let state = M3State::open(&root).unwrap();
        let request = DatasetGenerationRequest {
            user_id: "user".into(),
            snapshot_id: "missing".into(),
            model_archive_sha256: "missing".into(),
            model_parameters: HashMap::new(),
            factor_instances: vec![],
            seed: 0,
        };
        let cancelled = AtomicBool::new(true);
        assert_eq!(
            generate(&request, &state, &cancelled, "attempt").unwrap_err(),
            "Dataset Generation Attempt cancelled",
        );
        drop(state);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn duplicate_start_is_suppressed_and_retry_retains_terminal_evidence() {
        let (root, state, request) = setup("valid", "duplicate");
        let database = state.database.lock().unwrap();
        let first = prepare_attempt(&database, &request).unwrap();
        let duplicate = prepare_attempt(&database, &request).unwrap();
        assert!(first.should_start);
        assert!(!duplicate.should_start);
        assert_eq!(first.attempt.attempt_id, duplicate.attempt.attempt_id);
        database
            .execute(
                "UPDATE dataset_generation_attempts SET status = 'failed', diagnostic_json = 'retained' WHERE attempt_id = ?1",
                [&first.attempt.attempt_id],
            )
            .unwrap();
        let retry = prepare_attempt(&database, &request).unwrap();
        assert!(retry.should_start);
        assert_ne!(first.attempt.attempt_id, retry.attempt.attempt_id);
        assert_eq!(
            database
                .query_row(
                    "SELECT diagnostic_json FROM dataset_generation_attempts WHERE attempt_id = ?1",
                    [&first.attempt.attempt_id],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "retained",
        );
        drop(database);
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cancellation_authorization_is_user_scoped() {
        let (root, state, request) = setup("valid", "cancel-scope");
        let attempt_id = running_attempt(&state, &request);
        let database = state.database.lock().unwrap();
        assert!(!mark_cancelled(&database, &attempt_id, "bob").unwrap());
        assert_eq!(
            database
                .query_row(
                    "SELECT status FROM dataset_generation_attempts WHERE attempt_id = ?1",
                    [&attempt_id],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "running",
        );
        assert!(mark_cancelled(&database, &attempt_id, "alice").unwrap());
        drop(database);
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn real_model_generation_restarts_state_and_warmup_at_gaps() {
        let (root, state, request) = setup("valid", "real-model");
        let attempt_id = running_attempt(&state, &request);
        let cancelled = AtomicBool::new(false);
        let pending = generate(&request, &state, &cancelled, &attempt_id).unwrap();
        assert_eq!(pending.metadata.source_warmup_bars, 0);
        assert_eq!(pending.metadata.model_warmup_bars, 2);
        assert_eq!(pending.metadata.continuous_bar_segments, 2);
        assert_eq!(pending.metadata.producer_segments.len(), 1);
        assert_eq!(
            pending.metadata.producer_segments[0].start_prediction_time_ms,
            Some(3_600_000),
        );
        assert_eq!(
            pending.metadata.producer_segments[0].end_prediction_time_ms,
            Some(9 * 3_600_000),
        );
        assert_eq!(
            pending.metadata.producer_segments[0].feature_plan_hash,
            pending.metadata.feature_plan_hash,
        );
        assert_eq!(pending.metadata.status_counts["model-warmup"], 4);
        assert_eq!(pending.metadata.status_counts["present"], 2);
        assert!(
            pending
                .metadata
                .feature_plan_json
                .contains("consumerParameters")
        );
        let batches = parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::try_new(
            fs::File::open(&pending.temporary_path).unwrap(),
        )
        .unwrap()
        .build()
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
        let forecasts = batches[0]
            .column(4)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(forecasts.value(2), "[5.0]");
        assert_eq!(forecasts.value(5), "[8.0]");
        let dataset_id = pending.metadata.dataset_id.clone();
        let final_path = pending.final_path.clone();
        publish_dataset(&state, "alice", &attempt_id, &cancelled, pending).unwrap();
        assert!(final_path.is_file());
        let database = state.database.lock().unwrap();
        assert_eq!(
            database
                .query_row(
                    "SELECT COUNT(*) FROM signal_dataset_access WHERE user_id = 'alice'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1,
        );
        assert_eq!(
            database
                .query_row(
                    "SELECT COUNT(*) FROM signal_dataset_access WHERE user_id = 'bob'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            0,
        );
        database
            .execute(
                "INSERT INTO component_access(user_id, archive_sha256) VALUES ('bob', ?1)",
                [&request.model_archive_sha256],
            )
            .unwrap();
        database
            .execute(
                "INSERT INTO market_data_snapshot_access(user_id, snapshot_id) VALUES ('bob', ?1)",
                [&request.snapshot_id],
            )
            .unwrap();
        drop(database);
        let mut bob_request = request.clone();
        bob_request.user_id = "bob".into();
        let bob_attempt = running_attempt(&state, &bob_request);
        let bob_pending =
            generate(&bob_request, &state, &AtomicBool::new(false), &bob_attempt).unwrap();
        assert_eq!(bob_pending.metadata.dataset_id, dataset_id);
        publish_dataset(
            &state,
            "bob",
            &bob_attempt,
            &AtomicBool::new(false),
            bob_pending,
        )
        .unwrap();
        let database = state.database.lock().unwrap();
        assert_eq!(
            database
                .query_row("SELECT COUNT(*) FROM signal_dataset_content", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            1,
        );
        assert_eq!(
            database
                .query_row("SELECT COUNT(*) FROM signal_dataset_access", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            2,
        );
        drop(database);
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn malformed_model_failure_is_bounded_and_publishes_nothing() {
        let (root, state, request) = setup("non-finite", "failure");
        let attempt_id = running_attempt(&state, &request);
        let error = generate(&request, &state, &AtomicBool::new(false), &attempt_id).unwrap_err();
        assert!(error.starts_with("invalid-model-forecast:"));
        record_failure(
            &state,
            &attempt_id,
            &format!("{error}{}", "x".repeat(9_000)),
        )
        .unwrap();
        let database = state.database.lock().unwrap();
        let (status, evidence): (String, String) = database
            .query_row(
                "SELECT status, diagnostic_json FROM dataset_generation_attempts WHERE attempt_id = ?1",
                [&attempt_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "failed");
        assert_eq!(evidence.chars().count(), 8_192);
        assert_eq!(
            database
                .query_row("SELECT COUNT(*) FROM signal_dataset_content", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            0,
        );
        drop(database);
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn future_row_revision_is_rejected() {
        let (root, state, request) = setup("wrong-time", "future-revision");
        let attempt_id = running_attempt(&state, &request);
        assert!(
            generate(&request, &state, &AtomicBool::new(false), &attempt_id,)
                .unwrap_err()
                .starts_with("invalid-model-forecast:"),
        );
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reset_all_cancels_pending_publication() {
        let (root, state, request) = setup("valid", "reset-race");
        let watchlist = crate::watchlist::WatchlistDb::open(&root.join("adaq.db")).unwrap();
        let attempt_id = running_attempt(&state, &request);
        let cancelled = Arc::new(AtomicBool::new(false));
        state
            .generation_attempts
            .lock()
            .unwrap()
            .insert(attempt_id.clone(), cancelled.clone());
        let pending = generate(&request, &state, &cancelled, &attempt_id).unwrap();
        let temporary_path = pending.temporary_path.clone();
        let final_path = pending.final_path.clone();
        state
            .reset_local_data("alice", crate::m3::LocalDataResetKind::All)
            .unwrap();
        assert!(cancelled.load(Ordering::Relaxed));
        publish_dataset(&state, "alice", &attempt_id, &cancelled, pending).unwrap();
        assert!(!temporary_path.exists());
        assert!(!final_path.exists());
        assert_eq!(
            state
                .database
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM signal_dataset_access", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            0,
        );
        drop(watchlist);
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

    #[test]
    fn external_signal_archive_is_validated_published_and_round_trips() {
        let (root, state, request) = setup("valid", "external-archive");
        let attempt = running_attempt(&state, &request);
        let pending = generate(&request, &state, &AtomicBool::new(false), &attempt).unwrap();
        let parquet = fs::read(&pending.temporary_path).unwrap();
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
                .delete_component("alice", &request.model_archive_sha256)
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
        let attempt = running_attempt(&state, &request);
        let pending = generate(&request, &state, &AtomicBool::new(false), &attempt).unwrap();
        let parquet = fs::read(&pending.temporary_path).unwrap();
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
        let attempt = running_attempt(&state, &request);
        let pending = generate(&request, &state, &AtomicBool::new(false), &attempt).unwrap();
        let parquet = fs::read(&pending.temporary_path).unwrap();
        let manifest = external_manifest(&request.snapshot_id, &parquet, 3_600_000, 9 * 3_600_000);
        let archive = pack_signal_archive(&manifest, &parquet).unwrap();
        let dataset_id = hash(&[manifest.as_slice(), parquet.as_slice()].concat());
        state.database.lock().unwrap().execute_batch("CREATE TRIGGER reject_external_access BEFORE INSERT ON signal_dataset_access BEGIN SELECT RAISE(ABORT, 'forced publication failure'); END;").unwrap();
        assert!(
            import_signal_archive(&state, "alice", &archive)
                .unwrap_err()
                .contains("forced publication failure")
        );
        assert_eq!(
            state
                .database
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM signal_dataset_content", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
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
        let attempt = running_attempt(&state, &request);
        let pending = generate(&request, &state, &AtomicBool::new(false), &attempt).unwrap();
        publish_dataset(&state, "alice", &attempt, &AtomicBool::new(false), pending).unwrap();
        assert!(
            state
                .delete_component("alice", &request.model_archive_sha256)
                .unwrap_err()
                .contains("immutable Signal Dataset")
        );
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn kronos_fixture_reaches_import_evaluation_and_dataset_first_backtest() {
        let root = root("kronos-external-path");
        let state = M3State::open(&root).unwrap();
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
            .persist_snapshot(&BarSeries {
                src: "okx".into(),
                code: "BTC-USDT".into(),
                interval: BarInterval::OneHour,
                bars: bars.clone(),
                gaps: vec![],
            })
            .unwrap();
        state
            .database
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO market_data_snapshot_access(user_id, snapshot_id) VALUES ('alice', ?1)",
                [&snapshot.snapshot_id],
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
        state.import_component("alice", &strategy).unwrap();
        let run = crate::m3::execute_backtest(
            crate::m3::BacktestRunRequest {
                user_id: "alice".into(),
                snapshot_id: snapshot.snapshot_id,
                run_start_time_ms: None,
                run_end_time_ms: None,
                factor_instances: vec![],
                signal_instances: vec![crate::m3::SignalInstanceRequest {
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
                seed: 0,
            },
            &state,
        )
        .unwrap();
        let run_provenance = run.provenance.unwrap();
        assert_eq!(run_provenance.dataset_lock.len(), 1);
        assert_eq!(run_provenance.dataset_lock[0].evidence_state, "unknown");
        assert_eq!(format!("{:?}", run_provenance.architecture), "SignalDriven");
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }
}
