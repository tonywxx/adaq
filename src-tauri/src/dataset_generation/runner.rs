use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use adaq_component_sdk::host::model_abi;
use adaq_component_tooling::{
    ComponentKind, FactorInstancePlanInput, RunLimits, WasmLoader, component_parameters,
    native_engine_identity, validate_and_freeze_feature_plan_with_factors_and_parameters,
};
use rusqlite::params;

use super::{
    Attempt, DatasetGenerationRequest, Diagnostic, GenerationInner, UserResetBlock,
    store::AttemptStore, string,
};
use crate::{
    forecast_signal_dataset::{
        ComponentLockEntry, ModelProducerSegment, SignalDataset, close_time, hash, write_rows,
    },
    run_engine::{FactorRunRequest, MaterializedFeatureRow, materialize_feature_segment},
    user::validate_user,
};

const DATASET_ENGINE: &str = "closed-bar@1";
const CHUNK_SIZE: usize = 256;
static NEXT_ATTEMPT: AtomicU64 = AtomicU64::new(0);

/// Temporary Dataset output owned by one Attempt: generation writes the
/// Parquet to `temporary_path` and publication decides whether it becomes
/// the immutable final Signal Dataset.
#[derive(Debug)]
pub(crate) struct PendingDataset {
    pub(crate) metadata: SignalDataset,
    pub(crate) temporary_path: PathBuf,
    pub(crate) final_path: PathBuf,
}

pub(super) struct StartedGeneration {
    pub(super) attempt: Attempt,
    pub(super) cancelled: Option<Arc<AtomicBool>>,
    pub(super) request: DatasetGenerationRequest,
}

pub(super) fn start(
    inner: &GenerationInner,
    request: &DatasetGenerationRequest,
) -> Result<StartedGeneration, String> {
    validate_user(&request.user_id)?;
    let database = inner.source.database()?;
    if inner
        .reset_blocks
        .lock()
        .map_err(string)?
        .contains(&request.user_id)
    {
        return Err("Dataset Generation is blocked while Reset All is in progress".into());
    }
    let prepared = prepare_attempt(&database, request)?;
    if !prepared.should_start {
        return Ok(StartedGeneration {
            attempt: prepared.attempt,
            cancelled: None,
            request: request.clone(),
        });
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    inner
        .attempts
        .lock()
        .map_err(string)?
        .insert(prepared.attempt.attempt_id.clone(), cancelled.clone());
    Ok(StartedGeneration {
        attempt: prepared.attempt,
        cancelled: Some(cancelled),
        request: request.clone(),
    })
}

pub(super) fn retry(
    inner: &GenerationInner,
    attempt_id: &str,
    user_id: &str,
) -> Result<StartedGeneration, String> {
    validate_user(user_id)?;
    let database = inner.source.database()?;
    if inner.reset_blocks.lock().map_err(string)?.contains(user_id) {
        return Err("Dataset Generation is blocked while Reset All is in progress".into());
    }
    let (prepared, request) = AttemptStore::new(&database).prepare_retry(
        attempt_id,
        user_id,
        new_attempt_id,
        |request_json| {
            let request: DatasetGenerationRequest =
                serde_json::from_str(request_json).map_err(string)?;
            (request.user_id == user_id)
                .then_some(request)
                .ok_or_else(|| "Dataset Generation Attempt cannot be retried".into())
        },
    )?;
    let cancelled = Arc::new(AtomicBool::new(false));
    inner
        .attempts
        .lock()
        .map_err(string)?
        .insert(prepared.attempt.attempt_id.clone(), cancelled.clone());
    Ok(StartedGeneration {
        attempt: prepared.attempt,
        cancelled: Some(cancelled),
        request,
    })
}

/// Blocks new Start/Retry for one User, cancels active Attempts, and
/// waits for all to exit without holding the SQLite mutex.
pub(super) fn stop_all_for_user<'a>(
    inner: &'a GenerationInner,
    user_id: &str,
) -> Result<UserResetBlock<'a>, String> {
    validate_user(user_id)?;
    let database = inner.source.database()?;
    inner
        .reset_blocks
        .lock()
        .map_err(string)?
        .insert(user_id.to_string());
    let block = UserResetBlock {
        inner,
        user_id: user_id.to_string(),
    };
    let attempt_ids = AttemptStore::new(&database).active_ids_for_user(user_id)?;
    drop(database);
    {
        let attempts = inner.attempts.lock().map_err(string)?;
        for attempt_id in &attempt_ids {
            if let Some(cancelled) = attempts.get(attempt_id) {
                cancelled.store(true, Ordering::Relaxed);
            }
        }
    }
    let timeout = *inner.reset_wait_timeout.lock().map_err(string)?;
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = {
            let attempts = inner.attempts.lock().map_err(string)?;
            attempt_ids
                .iter()
                .filter(|attempt_id| attempts.contains_key(*attempt_id))
                .count()
        };
        if remaining == 0 {
            break;
        }
        if Instant::now() >= deadline {
            return Err(
                "Reset All could not stop Dataset Generation within the allowed time".into(),
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(block)
}

pub(super) fn run_started(
    inner: &GenerationInner,
    request: &DatasetGenerationRequest,
    cancelled: &Arc<AtomicBool>,
    attempt_id: &str,
) -> Result<(), String> {
    let database = inner.source.database()?;
    let attempts = AttemptStore::new(&database);
    let started = attempts.mark_running(attempt_id)?;
    if started {
        if attempts.reuse_completed_dataset(attempt_id)? {
            Ok(())
        } else {
            drop(database);
            run_attempt(inner, request, cancelled, attempt_id)
        }
    } else {
        Ok(())
    }
}

fn run_attempt(
    inner: &GenerationInner,
    request: &DatasetGenerationRequest,
    cancelled: &AtomicBool,
    attempt_id: &str,
) -> Result<(), String> {
    match generate(inner, request, cancelled, attempt_id) {
        Ok(dataset) => {
            match publish_dataset(inner, &request.user_id, attempt_id, cancelled, dataset) {
                Ok(PublicationResult::Cancelled) => {
                    let database = inner.source.database()?;
                    AttemptStore::new(&database)
                        .mark_cancelled_after_exit(attempt_id, &request.user_id)?;
                    Ok(())
                }
                Ok(PublicationResult::Published) => Ok(()),
                Err(error) => record_publication_failure(inner, attempt_id, &error),
            }
        }
        Err(error) => {
            if cancelled.load(Ordering::Relaxed) {
                let database = inner.source.database()?;
                AttemptStore::new(&database)
                    .mark_cancelled_after_exit(attempt_id, &request.user_id)?;
                Ok(())
            } else {
                record_failure(inner, attempt_id, &error)
            }
        }
    }
}

#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum LifecycleCheckpoint {
    AfterGeneration,
    BeforePublication,
    AfterPublicationCutover,
}

/// Private controllable runner seam: stops at named lifecycle checkpoints so
/// cancellation, publication-cutover, and Reset All races are deterministic
/// in tests. Not part of the module interface.
#[cfg(test)]
pub(super) fn run_attempt_with_lifecycle_checkpoint(
    inner: &GenerationInner,
    request: &DatasetGenerationRequest,
    cancelled: &AtomicBool,
    attempt_id: &str,
    mut checkpoint: impl FnMut(LifecycleCheckpoint),
) -> Result<(), String> {
    match generate(inner, request, cancelled, attempt_id) {
        Ok(dataset) => {
            checkpoint(LifecycleCheckpoint::AfterGeneration);
            checkpoint(LifecycleCheckpoint::BeforePublication);
            match publish_dataset(inner, &request.user_id, attempt_id, cancelled, dataset) {
                Ok(PublicationResult::Cancelled) => {
                    let database = inner.source.database()?;
                    AttemptStore::new(&database)
                        .mark_cancelled_after_exit(attempt_id, &request.user_id)?;
                    Ok(())
                }
                Ok(PublicationResult::Published) => {
                    checkpoint(LifecycleCheckpoint::AfterPublicationCutover);
                    Ok(())
                }
                Err(error) => record_publication_failure(inner, attempt_id, &error),
            }
        }
        Err(error) => {
            checkpoint(LifecycleCheckpoint::AfterGeneration);
            if cancelled.load(Ordering::Relaxed) {
                let database = inner.source.database()?;
                AttemptStore::new(&database)
                    .mark_cancelled_after_exit(attempt_id, &request.user_id)?;
                Ok(())
            } else {
                record_failure(inner, attempt_id, &error)
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PublicationResult {
    Cancelled,
    Published,
}

/// Publication finalization: cancellation wins before atomic publication
/// begins; publication wins after it begins. The Attempt reaches Completed
/// inside the same transaction that stores the Signal Dataset.
fn publish_dataset(
    inner: &GenerationInner,
    user_id: &str,
    attempt_id: &str,
    cancelled: &AtomicBool,
    pending: PendingDataset,
) -> Result<PublicationResult, String> {
    let mut database = inner.source.database()?;
    let transaction = database.transaction().map_err(string)?;
    if cancelled.load(Ordering::Relaxed) {
        transaction.commit().map_err(string)?;
        let _ = fs::remove_file(&pending.temporary_path);
        return Ok(PublicationResult::Cancelled);
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
        if !AttemptStore::new(&transaction)
            .mark_completed(attempt_id, &pending.metadata.dataset_id)?
        {
            return Err("Dataset Generation Attempt cannot be published".into());
        }
        transaction.commit().map_err(string)
    })();
    if publication.is_err() {
        let _ = fs::remove_file(&pending.temporary_path);
        if created_final {
            let _ = fs::remove_file(&pending.final_path);
        }
    }
    publication.map(|()| PublicationResult::Published)
}

pub(super) fn record_failure(
    inner: &GenerationInner,
    attempt_id: &str,
    error: &str,
) -> Result<(), String> {
    let database = inner.source.database()?;
    AttemptStore::new(&database).record_failure(attempt_id, Diagnostic::generation_failed(error))
}

pub(super) fn record_publication_failure(
    inner: &GenerationInner,
    attempt_id: &str,
    error: &str,
) -> Result<(), String> {
    let database = inner.source.database()?;
    AttemptStore::new(&database).record_failure(attempt_id, Diagnostic::publication_failed(error))
}

fn prepare_attempt(
    database: &rusqlite::Connection,
    request: &DatasetGenerationRequest,
) -> Result<super::store::PreparedAttempt, String> {
    let request_hash = hash(&canonical_request(request)?);
    AttemptStore::new(database).prepare(
        &request_hash,
        &request.user_id,
        &serde_json::to_string(request).map_err(string)?,
        || new_attempt_id(&request_hash),
    )
}

fn generate(
    inner: &GenerationInner,
    request: &DatasetGenerationRequest,
    cancelled: &AtomicBool,
    attempt_id: &str,
) -> Result<PendingDataset, String> {
    if cancelled.load(Ordering::Relaxed) {
        return Err("Dataset Generation Attempt cancelled".into());
    }
    let model = inner
        .source
        .package_for_user(&request.user_id, &request.model_archive_sha256)?;
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
            let package = inner
                .source
                .package_for_user(&request.user_id, &factor.archive_sha256)?;
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
        .map(|(factor, package, _)| {
            Ok((
                factor.alias.as_str(),
                inner.source.runtime_component(package)?,
            ))
        })
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
    let (snapshot, bars) = inner
        .source
        .snapshot_for_user(&request.user_id, &request.snapshot_id)?;
    let total = i64::try_from(bars.len()).map_err(|_| "Dataset row count is too large")?;
    let database = inner.source.database()?;
    AttemptStore::new(&database).set_progress_total(attempt_id, total)?;
    drop(database);
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
                let completed =
                    i64::try_from(index + 1).map_err(|_| "Dataset progress is too large")?;
                let database = inner.source.database()?;
                AttemptStore::new(&database).set_progress_completed(attempt_id, completed)?;
            }
        }
        start = end;
        let completed = i64::try_from(end).map_err(|_| "Dataset progress is too large")?;
        let database = inner.source.database()?;
        AttemptStore::new(&database).set_progress_completed(attempt_id, completed)?;
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
    let directory = inner.source.dataset_directory()?;
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
    let mut factors = request
        .factor_instances
        .iter()
        .map(|factor| {
            (
                &factor.alias,
                &factor.archive_sha256,
                factor.parameters.iter().collect::<BTreeMap<_, _>>(),
            )
        })
        .collect::<Vec<_>>();
    factors.sort();
    serde_json::to_vec(&(
        &request.user_id,
        &request.snapshot_id,
        &request.model_archive_sha256,
        request.model_parameters.iter().collect::<BTreeMap<_, _>>(),
        factors,
        request.seed,
    ))
    .map_err(string)
}

#[cfg(test)]
mod tests {
    use super::super::tests::{seed_running_attempt, setup};
    use super::*;
    use crate::dataset_generation::AttemptStatus;
    use arrow_array::Array;
    use std::collections::HashMap;

    #[test]
    fn cancellation_stops_before_component_or_snapshot_access() {
        let (root, state, request) = setup("valid", "runner-cancel-early");
        let cancelled = AtomicBool::new(true);
        assert_eq!(
            generate(&state.generation.0, &request, &cancelled, "attempt").unwrap_err(),
            "Dataset Generation Attempt cancelled",
        );
        drop(state);
        std::fs::remove_dir_all(root).unwrap();
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
    fn publication_cutover_wins_over_a_late_cancellation() {
        let (root, state, request) = setup("valid", "runner-publication-cutover");
        let (attempt_id, cancelled) = seed_running_attempt(&state, "alice", "cutover");
        let mut late_cancellation = None;
        run_attempt_with_lifecycle_checkpoint(
            &state.generation.0,
            &request,
            &cancelled,
            &attempt_id,
            |checkpoint| {
                if checkpoint == LifecycleCheckpoint::AfterPublicationCutover {
                    late_cancellation =
                        Some(state.generation.cancel(&attempt_id, "alice").unwrap_err());
                }
            },
        )
        .unwrap();
        let attempt = state.generation.list("alice").unwrap().remove(0);
        assert_eq!(attempt.status, AttemptStatus::Completed);
        assert!(attempt.dataset_id.is_some());
        assert_eq!(
            late_cancellation.as_deref(),
            Some("Dataset Generation Attempt cannot be cancelled")
        );
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn publication_failure_retains_no_dataset_and_records_its_cause() {
        let (root, state, request) = setup("valid", "runner-publication-failure");
        let (attempt_id, cancelled) = seed_running_attempt(&state, "alice", "pub-failure");
        state.database.lock().unwrap().execute_batch("CREATE TRIGGER reject_signal_dataset_access BEFORE INSERT ON signal_dataset_access BEGIN SELECT RAISE(ABORT, 'forced publication failure'); END;").unwrap();
        run_attempt_with_lifecycle_checkpoint(
            &state.generation.0,
            &request,
            &cancelled,
            &attempt_id,
            |_| {},
        )
        .unwrap();
        let attempt = state.generation.list("alice").unwrap().remove(0);
        assert_eq!(attempt.status, AttemptStatus::Failed);
        assert!(attempt.dataset_id.is_none());
        assert!(
            attempt
                .diagnostic_evidence
                .unwrap()
                .contains("publication-failed: forced publication failure")
        );
        assert!(
            fs::read_dir(state.root.join("signal-datasets"))
                .unwrap()
                .next()
                .is_none()
        );
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn generation_restarts_state_and_warmup_at_gaps() {
        let (root, state, request) = setup("valid", "runner-real-model");
        let (attempt_id, cancelled) = seed_running_attempt(&state, "alice", "real-model");
        let pending = generate(&state.generation.0, &request, &cancelled, &attempt_id).unwrap();
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
            .downcast_ref::<arrow_array::StringArray>()
            .unwrap();
        assert_eq!(forecasts.value(2), "[5.0]");
        assert_eq!(forecasts.value(5), "[8.0]");
        let dataset_id = pending.metadata.dataset_id.clone();
        let final_path = pending.final_path.clone();
        publish_dataset(
            &state.generation.0,
            "alice",
            &attempt_id,
            &cancelled,
            pending,
        )
        .unwrap();
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
        drop(database);
        state
            .grant_snapshot_for_user("bob", &request.snapshot_id)
            .unwrap();
        let mut bob_request = request.clone();
        bob_request.user_id = "bob".into();
        let (bob_attempt_id, bob_cancelled) = seed_running_attempt(&state, "bob", "real-model-bob");
        let bob_pending = generate(
            &state.generation.0,
            &bob_request,
            &bob_cancelled,
            &bob_attempt_id,
        )
        .unwrap();
        assert_eq!(bob_pending.metadata.dataset_id, dataset_id);
        publish_dataset(
            &state.generation.0,
            "bob",
            &bob_attempt_id,
            &bob_cancelled,
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
        let (root, state, request) = setup("non-finite", "runner-failure");
        let (attempt_id, cancelled) = seed_running_attempt(&state, "alice", "bounded-failure");
        let error = generate(&state.generation.0, &request, &cancelled, &attempt_id).unwrap_err();
        assert!(error.starts_with("invalid-model-forecast:"));
        record_failure(
            &state.generation.0,
            &attempt_id,
            &format!("{error}{}", "x".repeat(9_000)),
        )
        .unwrap();
        let attempt = state.generation.list("alice").unwrap().remove(0);
        assert_eq!(attempt.status, AttemptStatus::Failed);
        assert_eq!(
            attempt
                .diagnostic_evidence
                .as_ref()
                .unwrap()
                .chars()
                .count(),
            8_192
        );
        let content: i64 = state
            .database
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM signal_dataset_content", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(content, 0);
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn future_row_revision_is_rejected() {
        let (root, state, request) = setup("wrong-time", "runner-future-revision");
        let (attempt_id, cancelled) = seed_running_attempt(&state, "alice", "future-revision");
        assert!(
            generate(&state.generation.0, &request, &cancelled, &attempt_id)
                .unwrap_err()
                .starts_with("invalid-model-forecast:"),
        );
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }
}
