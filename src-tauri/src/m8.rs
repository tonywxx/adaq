use std::{
    collections::BTreeMap,
    fs,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use adaq_component_sdk::host::model_abi;
use adaq_component_tooling::{
    ComponentKind, FactorInstancePlanInput, RunLimits, WasmLoader, component_parameters,
    native_engine_identity, validate_and_freeze_feature_plan_with_factors_and_parameters,
};
use arrow_array::{ArrayRef, Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use parquet::arrow::ArrowWriter;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::Manager;

use crate::{
    m3::{M3State, validate_user},
    run_engine::{FactorRunRequest, MaterializedFeatureRow, materialize_feature_segment},
};

const DATASET_ENGINE: &str = "closed-bar@1";
const CHUNK_SIZE: usize = 256;
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

#[derive(Serialize)]
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
        let _watchlist = crate::watchlist::WatchlistDb::open(&root.join("adaq.db")).unwrap();
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
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }
}
