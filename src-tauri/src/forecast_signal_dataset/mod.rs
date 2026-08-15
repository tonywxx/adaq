//! Forecast Signal Dataset management: import, archive, and query of external
//! and backtest-generated forecast signal evidence.
//!
//! Domain terms: Forecast Signal Dataset, Forecast Signal Archive, Model
//! Producer Segment, Externally Generated Signal Dataset, Component Lock.
//! See CONTEXT.md.

use std::{
    collections::BTreeMap,
    fs,
    io::{Cursor, Read, Write},
    sync::Arc,
};

use adaq_backtest_core::MarketDataSnapshot;
use adaq_component_tooling::{
    BuiltinForecastTarget, ComponentParameterValue, ForecastTarget, ForecastValueScale,
    ModelArtifact, ModelOutput, PredictionKind, native_engine_identity,
};
use arrow_array::{Array, ArrayRef, Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use parquet::arrow::ArrowWriter;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

use crate::forecast_evaluation::{
    score_scale_provenance, segment_evidence, validate_prediction_scale,
};
use crate::local_research::LocalResearchState;
use crate::user::validate_user;

const SIGNAL_ARCHIVE_SCHEMA_VERSION: u32 = 1;
const MAX_SIGNAL_ARCHIVE_BYTES: usize = 64 * 1024 * 1024;
const MAX_SIGNAL_MANIFEST_BYTES: usize = 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SignalDataset {
    pub(crate) dataset_id: String,
    pub(crate) snapshot_id: String,
    pub(crate) src: String,
    pub(crate) code: String,
    pub(crate) interval: String,
    pub(crate) prediction_source: String,
    pub(crate) model_artifact: Option<adaq_component_tooling::ModelArtifact>,
    pub(crate) model_outputs: Vec<adaq_component_tooling::ModelOutput>,
    pub(crate) model_parameters: BTreeMap<String, adaq_component_tooling::ComponentParameterValue>,
    pub(crate) source_warmup_bars: u32,
    pub(crate) model_warmup_bars: u32,
    pub(crate) model_archive_sha256: String,
    pub(crate) trust: String,
    pub(crate) component_lock: Vec<ComponentLockEntry>,
    pub(crate) feature_plan_json: String,
    pub(crate) feature_plan_hash: String,
    pub(crate) seed: u64,
    pub(crate) engine_identity: adaq_component_tooling::EngineIdentity,
    pub(crate) producer_segments: Vec<ModelProducerSegment>,
    pub(crate) continuous_bar_segments: usize,
    pub(crate) bar_gap_rule: String,
    pub(crate) row_count: usize,
    pub(crate) unavailable_count: usize,
    pub(crate) status_counts: BTreeMap<String, usize>,
    pub(crate) parquet_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) archive_manifest_json: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) external_producer_segments: Option<Vec<serde_json::Value>>,
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
pub(crate) struct ModelProducerSegment {
    pub(crate) start_prediction_time_ms: Option<i64>,
    pub(crate) end_prediction_time_ms: Option<i64>,
    pub(crate) model_archive_sha256: String,
    pub(crate) model_artifact: Option<adaq_component_tooling::ModelArtifact>,
    pub(crate) model_parameters: BTreeMap<String, adaq_component_tooling::ComponentParameterValue>,
    pub(crate) seed: u64,
    pub(crate) trust: String,
    pub(crate) engine_identity: adaq_component_tooling::EngineIdentity,
    pub(crate) feature_plan_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ComponentLockEntry {
    pub(crate) alias: String,
    pub(crate) archive_sha256: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExternalRow {
    instrument_id: String,
    pub(crate) prediction_time_ms: i64,
    available_at_ms: i64,
    status: String,
    pub(crate) values: Option<Vec<f64>>,
    pub(crate) unavailable_reason: Option<String>,
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

pub(crate) fn read_external_rows(parquet: &[u8]) -> Result<Vec<ExternalRow>, String> {
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
pub fn signal_dataset_list(
    user_id: String,
    state: tauri::State<'_, Arc<LocalResearchState>>,
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
    state: tauri::State<'_, Arc<LocalResearchState>>,
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
    state: tauri::State<'_, Arc<LocalResearchState>>,
) -> Result<serde_json::Value, String> {
    signal_rows_page(&state, &user_id, &dataset_id, page)
}

fn signal_rows_page(
    state: &LocalResearchState,
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
    state: tauri::State<'_, Arc<LocalResearchState>>,
) -> Result<serde_json::Value, String> {
    import_signal_archive(&state, &user_id, &archive)
}

#[tauri::command]
pub fn signal_dataset_export(
    dataset_id: String,
    user_id: String,
    state: tauri::State<'_, Arc<LocalResearchState>>,
) -> Result<Vec<u8>, String> {
    export_signal_archive(&state, &user_id, &dataset_id)
}

fn export_signal_archive(
    state: &LocalResearchState,
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
    state: &LocalResearchState,
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

pub(crate) fn close_time(interval: adaq_data_core::BarInterval, open: i64) -> Result<i64, String> {
    adaq_data_core::next_bar_open_time_ms(open, interval).map_err(string)
}

pub(crate) fn write_rows(
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

pub(crate) fn publish_python_model_signal_dataset(
    state: &LocalResearchState,
    user_id: &str,
    dataset_id: &str,
    snapshot_id: &str,
    feature_plan_hash: &str,
    factor_dataset_id: &str,
    feature_dataset_id: &str,
    artifact_sha256: &str,
    artifact_provenance: &BTreeMap<String, String>,
    adapter_id: &str,
    alpha: f64,
    seed: u64,
    forecast_contract: &str,
    rows: &[adaq_python_research::model::ForecastRow],
) -> Result<serde_json::Value, String> {
    validate_user(user_id)?;
    if dataset_id.is_empty()
        || snapshot_id.is_empty()
        || feature_plan_hash.is_empty()
        || factor_dataset_id.is_empty()
        || feature_dataset_id.is_empty()
        || artifact_sha256.is_empty()
        || adapter_id.is_empty()
        || !alpha.is_finite()
        || forecast_contract.is_empty()
        || rows.windows(2).any(|window| {
            (window[0].datetime, window[0].instrument.as_str())
                >= (window[1].datetime, window[1].instrument.as_str())
        })
        || rows.iter().any(|row| {
            row.instrument.trim().is_empty()
                || row.value.is_some_and(|value| !value.is_finite())
                || row.value.is_some() == row.unavailable_reason.is_some()
        })
    {
        return Err("python-model-signal-dataset-contract-invalid".into());
    }
    let (snapshot, bars) = state.snapshot_for_user(user_id, snapshot_id)?;
    let snapshot_instrument = format!("{}:{}", snapshot.src, snapshot.code);
    let forecasts_by_identity = rows
        .iter()
        .map(|row| ((row.datetime, row.instrument.clone()), row))
        .collect::<BTreeMap<_, _>>();
    let published_rows = bars
        .iter()
        .map(|bar| {
            let prediction_time = close_time(snapshot.interval, bar.open_time_ms)?;
            let source = forecasts_by_identity
                .get(&(bar.open_time_ms, snapshot.code.clone()))
                .or_else(|| {
                    forecasts_by_identity.get(&(bar.open_time_ms, snapshot_instrument.clone()))
                });
            let (values, unavailable_reason) = match source {
                Some(row) => (
                    row.value.map(|value| vec![value]),
                    row.unavailable_reason.clone(),
                ),
                None => (None, Some("model-window-outside-final".into())),
            };
            Ok::<_, String>((
                snapshot_instrument.clone(),
                prediction_time,
                prediction_time,
                values,
                unavailable_reason,
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let identity = native_engine_identity().map_err(string)?;
    let model_output = ModelOutput {
        name: "forecast".into(),
        prediction_kind: PredictionKind::ExpectedValue,
        forecast_target: ForecastTarget::Builtin {
            target: BuiltinForecastTarget::FutureCloseReturn,
        },
        value_scale: ForecastValueScale::Native,
        horizon_bars: 5,
    };
    let model_parameters = BTreeMap::from([(
        "alpha".into(),
        ComponentParameterValue::Decimal(alpha.to_string()),
    )]);
    let model_artifact = ModelArtifact {
        sha256: artifact_sha256.into(),
        provenance: artifact_provenance.clone(),
    };
    let producer_segments = vec![ModelProducerSegment {
        start_prediction_time_ms: published_rows.first().map(|row| row.1),
        end_prediction_time_ms: published_rows.last().map(|row| row.1),
        model_archive_sha256: artifact_sha256.into(),
        model_artifact: Some(model_artifact.clone()),
        model_parameters: model_parameters.clone(),
        seed,
        trust: "managed-python-model@1".into(),
        engine_identity: identity.clone(),
        feature_plan_hash: feature_plan_hash.into(),
    }];
    let feature_plan_json = serde_json::json!({
        "featurePlanHash": feature_plan_hash,
        "factorDatasetId": factor_dataset_id,
        "featureDatasetId": feature_dataset_id,
        "adapterId": adapter_id,
        "forecastContract": forecast_contract,
    })
    .to_string();
    let temporary_path = state
        .root
        .join("signal-datasets")
        .join(format!(".{dataset_id}.model.tmp"));
    let directory = state.root.join("signal-datasets");
    fs::create_dir_all(&directory).map_err(string)?;
    write_rows(&temporary_path, &published_rows)?;
    let parquet = fs::read(&temporary_path).map_err(string)?;
    let parquet_sha256 = hash(&parquet);
    let final_path = directory.join(format!("{dataset_id}.parquet"));
    let unavailable_count = published_rows.iter().filter(|row| row.3.is_none()).count();
    let status_counts = published_rows
        .iter()
        .fold(BTreeMap::new(), |mut counts, row| {
            let status = row
                .3
                .is_some()
                .then_some("present")
                .unwrap_or_else(|| row.4.as_deref().unwrap_or("unavailable"));
            *counts.entry(status.to_owned()).or_insert(0) += 1;
            counts
        });
    let metadata = SignalDataset {
        dataset_id: dataset_id.into(),
        snapshot_id: snapshot.snapshot_id,
        src: snapshot.src,
        code: snapshot.code,
        interval: snapshot.interval.as_str().into(),
        prediction_source: "adaq-python-model@1".into(),
        model_artifact: Some(model_artifact),
        model_outputs: vec![model_output],
        model_parameters,
        source_warmup_bars: 0,
        model_warmup_bars: 5,
        model_archive_sha256: artifact_sha256.into(),
        trust: "managed-python-model@1".into(),
        component_lock: vec![],
        feature_plan_json,
        feature_plan_hash: feature_plan_hash.into(),
        seed,
        engine_identity: identity,
        producer_segments,
        continuous_bar_segments: 1,
        bar_gap_rule: "model-forecast@1".into(),
        row_count: published_rows.len(),
        unavailable_count,
        status_counts,
        parquet_sha256: parquet_sha256.clone(),
        archive_manifest_json: None,
        external_producer_segments: None,
    };
    let metadata_json = serde_json::to_string(&metadata).map_err(string)?;
    let mut database = state.database.lock().map_err(string)?;
    let transaction = database.transaction().map_err(string)?;
    let mut created_final = false;
    let result = (|| -> Result<(), String> {
        if final_path.exists() {
            if hash(&fs::read(&final_path).map_err(string)?) != parquet_sha256 {
                return Err("existing-dataset-content-hash-mismatch".into());
            }
            fs::remove_file(&temporary_path).map_err(string)?;
        } else {
            fs::rename(&temporary_path, &final_path).map_err(string)?;
            created_final = true;
        }
        transaction
            .execute(
                "INSERT OR IGNORE INTO signal_dataset_content(dataset_id, metadata_json, parquet_path) VALUES (?1, ?2, ?3)",
                params![dataset_id, metadata_json, final_path.to_string_lossy()],
            )
            .map_err(string)?;
        transaction
            .execute(
                "INSERT OR IGNORE INTO signal_dataset_access(user_id, dataset_id) VALUES (?1, ?2)",
                params![user_id, dataset_id],
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

pub(crate) fn dataset_outputs(
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
    state: &LocalResearchState,
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

pub(crate) fn producer_segment_values(
    dataset: &SignalDataset,
) -> Result<Vec<serde_json::Value>, String> {
    if let Some(segments) = &dataset.external_producer_segments {
        return Ok(segments.clone());
    }
    dataset
        .producer_segments
        .iter()
        .map(|segment| serde_json::to_value(segment).map_err(string))
        .collect()
}

pub(crate) fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(crate) fn string(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod python_model_tests {
    use super::*;
    use adaq_data_core::{BarInterval, BarSeries, OhlcvBar};
    use rust_decimal::Decimal;

    #[test]
    fn python_model_forecasts_publish_through_the_m8_signal_contract() {
        let root = std::env::temp_dir().join(format!(
            "adaq-python-model-signal-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let state = LocalResearchState::open(&root).unwrap();
        let snapshot = state
            .persist_snapshot_for_user(
                "alice",
                &BarSeries {
                    src: "okx".into(),
                    code: "BTC-USDT".into(),
                    interval: BarInterval::OneHour,
                    bars: vec![
                        OhlcvBar {
                            open_time_ms: 0,
                            open: Decimal::ONE,
                            high: Decimal::ONE,
                            low: Decimal::ONE,
                            close: Decimal::ONE,
                            base_volume: Decimal::ONE,
                            quote_volume: Decimal::ONE,
                        },
                        OhlcvBar {
                            open_time_ms: 3_600_000,
                            open: Decimal::ONE,
                            high: Decimal::ONE,
                            low: Decimal::ONE,
                            close: Decimal::ONE,
                            base_volume: Decimal::ONE,
                            quote_volume: Decimal::ONE,
                        },
                    ],
                    gaps: vec![],
                },
            )
            .unwrap();
        let rows = vec![
            adaq_python_research::model::ForecastRow {
                datetime: 0,
                instrument: "BTC-USDT".into(),
                value: Some(0.25),
                unavailable_reason: None,
            },
            adaq_python_research::model::ForecastRow {
                datetime: 3_600_000,
                instrument: "BTC-USDT".into(),
                value: None,
                unavailable_reason: Some("target-window-boundary".into()),
            },
        ];
        let metadata = publish_python_model_signal_dataset(
            &state,
            "alice",
            "python-model-forecast",
            &snapshot.snapshot_id,
            "feature-plan",
            "factor-dataset",
            "feature-dataset",
            &"b".repeat(64),
            &BTreeMap::from([(String::from("resourcePolicy"), String::from("policy"))]),
            "qlib-linear-ridge@1",
            1.0,
            7,
            "forecast:continuous-future-close-return:native@1",
            &rows,
        )
        .unwrap();
        assert_eq!(metadata["predictionSource"], "adaq-python-model@1");
        assert_eq!(metadata["rowCount"], 2);
        assert_eq!(metadata["unavailableCount"], 1);
        let page = signal_rows_page(&state, "alice", "python-model-forecast", 1).unwrap();
        assert_eq!(page["total"], 2);
        assert_eq!(page["items"][0]["availableAtMs"], 3_600_000);
        assert_eq!(page["items"][1]["status"], "unavailable");
        assert_eq!(
            page["items"][1]["unavailableReason"],
            "target-window-boundary"
        );
        assert!(
            signal_rows_page(&state, "bob", "python-model-forecast", 1)
                .unwrap_err()
                .contains("not available")
        );
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }
}
