//! A-share Source -> Canonical publication, immutable reference data, and
//! restart-safe acquisition state.

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::PathBuf,
    str::FromStr,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use adaq_data_core::{
    BarInterval, HistoricalBarRange,
    a_share::{
        AshareBar, AshareBarsAcquisition, AshareCalendarAcquisition, AshareClient,
        AshareCorporateAction, AshareCorporateActionAcquisition, AshareInstrument,
        AshareInstrumentMasterAcquisition, normalize_provider_instrument,
    },
    market::{InstrumentId, PriceBasis, SessionPhase, TradingCalendarSnapshot, VenueKind},
};
use chrono::{NaiveDate, NaiveTime};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    AcquisitionDiagnostics, CalendarEvidence, CancellationToken, CanonicalizationRequest,
    DataPipeline, DataQualityState, PipelineError, PipelinePublication, ProviderCapabilitySnapshot,
    SourceAcquisition, SourceMarketRecord, canonical_json_bytes, digest, validate_user,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UniverseEvidenceState {
    Observed,
    Reconstructed,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AshareInstrumentMasterSnapshot {
    pub snapshot_id: String,
    pub effective_at_ms: i64,
    pub provider: String,
    pub actual_upstream: String,
    pub method: String,
    pub connector_version: String,
    pub request_parameters: Value,
    pub response_sha256: String,
    pub parsed_response_sha256: String,
    pub content_sha256: String,
    pub diagnostics: adaq_data_core::a_share::AshareRequestDiagnostics,
    pub capability_snapshot: ProviderCapabilitySnapshot,
    pub evidence_state: UniverseEvidenceState,
    pub instruments: Vec<AshareInstrument>,
    #[serde(default)]
    pub limitations: Vec<String>,
    pub evidence_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AshareInstrumentMasterSnapshotDto {
    pub snapshot_id: String,
    pub effective_at_ms: i64,
    pub provider: String,
    pub actual_upstream: String,
    pub method: String,
    pub connector_version: String,
    pub request_parameters: Value,
    pub response_sha256: String,
    pub parsed_response_sha256: String,
    pub content_sha256: String,
    pub diagnostics: adaq_data_core::a_share::AshareRequestDiagnostics,
    pub capability_snapshot: ProviderCapabilitySnapshot,
    pub evidence_state: UniverseEvidenceState,
    pub instruments: Vec<AshareInstrument>,
    #[serde(default)]
    pub limitations: Vec<String>,
}

impl AshareInstrumentMasterSnapshot {
    pub fn gui_dto(&self) -> AshareInstrumentMasterSnapshotDto {
        AshareInstrumentMasterSnapshotDto {
            snapshot_id: self.snapshot_id.clone(),
            effective_at_ms: self.effective_at_ms,
            provider: self.provider.clone(),
            actual_upstream: self.actual_upstream.clone(),
            method: self.method.clone(),
            connector_version: self.connector_version.clone(),
            request_parameters: self.request_parameters.clone(),
            response_sha256: self.response_sha256.clone(),
            parsed_response_sha256: self.parsed_response_sha256.clone(),
            content_sha256: self.content_sha256.clone(),
            diagnostics: self.diagnostics.clone(),
            capability_snapshot: self.capability_snapshot.clone(),
            evidence_state: self.evidence_state.clone(),
            instruments: self.instruments.clone(),
            limitations: self.limitations.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AsharePointInTimeUniverse {
    pub universe_id: String,
    pub as_of_ms: i64,
    pub snapshot_id: Option<String>,
    pub evidence_state: UniverseEvidenceState,
    pub instruments: Vec<AshareInstrument>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AshareCalendarSnapshot {
    pub snapshot: TradingCalendarSnapshot,
    pub provider: String,
    pub actual_upstream: String,
    pub method: String,
    pub connector_version: String,
    pub request_parameters: Value,
    pub retrieved_at_ms: i64,
    pub response_sha256: String,
    pub content_sha256: String,
    pub diagnostics: adaq_data_core::a_share::AshareRequestDiagnostics,
    pub capability_snapshot: ProviderCapabilitySnapshot,
    #[serde(default)]
    pub limitations: Vec<String>,
    pub evidence_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AshareCalendarSnapshotDto {
    pub snapshot: TradingCalendarSnapshot,
    pub provider: String,
    pub actual_upstream: String,
    pub method: String,
    pub connector_version: String,
    pub request_parameters: Value,
    pub retrieved_at_ms: i64,
    pub response_sha256: String,
    pub content_sha256: String,
    pub diagnostics: adaq_data_core::a_share::AshareRequestDiagnostics,
    pub capability_snapshot: ProviderCapabilitySnapshot,
    #[serde(default)]
    pub limitations: Vec<String>,
}

impl AshareCalendarSnapshot {
    pub fn gui_dto(&self) -> AshareCalendarSnapshotDto {
        AshareCalendarSnapshotDto {
            snapshot: self.snapshot.clone(),
            provider: self.provider.clone(),
            actual_upstream: self.actual_upstream.clone(),
            method: self.method.clone(),
            connector_version: self.connector_version.clone(),
            request_parameters: self.request_parameters.clone(),
            retrieved_at_ms: self.retrieved_at_ms,
            response_sha256: self.response_sha256.clone(),
            content_sha256: self.content_sha256.clone(),
            diagnostics: self.diagnostics.clone(),
            capability_snapshot: self.capability_snapshot.clone(),
            limitations: self.limitations.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AshareCorporateActionDataset {
    pub dataset_id: String,
    pub revision: u64,
    pub logical_key: String,
    pub instrument: InstrumentId,
    pub provider: String,
    pub actual_upstream: String,
    pub method: String,
    pub connector_version: String,
    pub request_parameters: Value,
    pub retrieved_at_ms: i64,
    pub response_sha256: String,
    pub content_sha256: String,
    pub diagnostics: adaq_data_core::a_share::AshareRequestDiagnostics,
    pub capability_snapshot: ProviderCapabilitySnapshot,
    pub quality: DataQualityState,
    pub records: Vec<AshareCorporateAction>,
    #[serde(default)]
    pub quarantined_records: Vec<AshareCorporateAction>,
    #[serde(default)]
    pub limitations: Vec<String>,
    pub evidence_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AshareCorporateActionDto {
    pub instrument: InstrumentId,
    pub provider_symbol: String,
    pub kind: adaq_data_core::a_share::AshareCorporateActionKind,
    pub effective_at_ms: Option<i64>,
    pub announced_at_ms: Option<i64>,
    pub available_at_ms: i64,
    pub cash_per_share: Option<String>,
    pub shares_per_share: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AshareCorporateActionDatasetDto {
    pub dataset_id: String,
    pub revision: u64,
    pub instrument: InstrumentId,
    pub provider: String,
    pub actual_upstream: String,
    pub method: String,
    pub connector_version: String,
    pub request_parameters: Value,
    pub retrieved_at_ms: i64,
    pub response_sha256: String,
    pub content_sha256: String,
    pub diagnostics: adaq_data_core::a_share::AshareRequestDiagnostics,
    pub capability_snapshot: ProviderCapabilitySnapshot,
    pub quality: DataQualityState,
    pub quarantine_count: usize,
    pub records: Vec<AshareCorporateActionDto>,
    #[serde(default)]
    pub limitations: Vec<String>,
}

impl AshareCorporateActionDataset {
    pub fn gui_dto(&self) -> AshareCorporateActionDatasetDto {
        AshareCorporateActionDatasetDto {
            dataset_id: self.dataset_id.clone(),
            revision: self.revision,
            instrument: self.instrument.clone(),
            provider: self.provider.clone(),
            actual_upstream: self.actual_upstream.clone(),
            method: self.method.clone(),
            connector_version: self.connector_version.clone(),
            request_parameters: self.request_parameters.clone(),
            retrieved_at_ms: self.retrieved_at_ms,
            response_sha256: self.response_sha256.clone(),
            content_sha256: self.content_sha256.clone(),
            diagnostics: self.diagnostics.clone(),
            capability_snapshot: self.capability_snapshot.clone(),
            quality: self.quality.clone(),
            quarantine_count: self.quarantined_records.len(),
            records: self
                .records
                .iter()
                .map(|record| AshareCorporateActionDto {
                    instrument: record.instrument.clone(),
                    provider_symbol: record.provider_symbol.clone(),
                    kind: record.kind,
                    effective_at_ms: record.effective_at_ms,
                    announced_at_ms: record.announced_at_ms,
                    available_at_ms: record.available_at_ms,
                    cash_per_share: record.cash_per_share.clone(),
                    shares_per_share: record.shares_per_share.clone(),
                })
                .collect(),
            limitations: self.limitations.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AshareBackfillState {
    Running,
    Completed,
    Degraded,
    Rejected,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AshareBackfillRequest {
    pub task_id: String,
    pub user_id: String,
    pub instrument: InstrumentId,
    pub interval: BarInterval,
    pub start_time_ms: i64,
    pub end_time_ms: i64,
    pub calendar: TradingCalendarSnapshot,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AshareBackfillCheckpoint {
    task_id: String,
    user_id: String,
    request: AshareBackfillRequest,
    state: AshareBackfillState,
    source_id: Option<String>,
    canonical_id: Option<String>,
    revision: Option<u64>,
    #[serde(default)]
    completed_through_ms: Option<i64>,
    #[serde(default)]
    acquisition_path: Option<PathBuf>,
    #[serde(default)]
    acquisition_sha256: Option<String>,
    #[serde(default)]
    failure_response_sha256: Option<String>,
    #[serde(default)]
    failure_response_path: Option<PathBuf>,
    last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "event", content = "data")]
pub enum AshareBackfillEvent {
    Started {
        task_id: String,
        instrument: InstrumentId,
    },
    AcquisitionStarted {
        instrument: InstrumentId,
        interval: BarInterval,
    },
    Published {
        instrument: InstrumentId,
        source_id: String,
        canonical_id: Option<String>,
        revision: u64,
        state: DataQualityState,
    },
    AlreadyCompleted {
        task_id: String,
        source_id: String,
    },
    Cancelled {
        task_id: String,
    },
    Failed {
        task_id: String,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AshareMarketWorkspaceDto {
    pub instrument: InstrumentId,
    pub provider: String,
    pub actual_upstream: Option<String>,
    pub connector: String,
    pub connector_version: String,
    pub retrieved_at_ms: i64,
    pub freshness_ms: Option<i64>,
    pub price_basis: PriceBasis,
    pub calendar_id: String,
    pub quality: DataQualityState,
    pub source_id: String,
    pub canonical_id: Option<String>,
    pub revision: u64,
    pub coverage_start_ms: Option<i64>,
    pub coverage_end_ms: Option<i64>,
    pub gap_count: usize,
    #[serde(default)]
    pub limitations: Vec<String>,
}

#[derive(Clone)]
pub struct AshareDataPath {
    pipeline: DataPipeline,
    client: AshareClient,
    active: Arc<Mutex<HashMap<String, CancellationToken>>>,
    resetting: Arc<Mutex<HashSet<String>>>,
}

struct BackfillActivityGuard {
    pipeline: DataPipeline,
    active: Arc<Mutex<HashMap<String, CancellationToken>>>,
    key: String,
}

impl Drop for BackfillActivityGuard {
    fn drop(&mut self) {
        if let Ok(mut active) = self.active.lock() {
            active.remove(&self.key);
        }
        let _ = self.pipeline.finish_attempt(&self.key);
    }
}

impl AshareDataPath {
    pub fn open(pipeline: DataPipeline, client: AshareClient) -> Result<Self, PipelineError> {
        for directory in [
            "a-share",
            "a-share/instrument-master",
            "a-share/calendars",
            "a-share/corporate-actions",
            "a-share/raw",
            "a-share/checkpoints",
        ] {
            fs::create_dir_all(pipeline.root_dir().join(directory)).map_err(storage)?;
        }
        let path = Self {
            pipeline,
            client,
            active: Arc::new(Mutex::new(HashMap::new())),
            resetting: Arc::new(Mutex::new(HashSet::new())),
        };
        path.initialize_schema()?;
        Ok(path)
    }

    pub fn client(&self) -> &AshareClient {
        &self.client
    }

    pub fn cancel_user_operations(&self, user_id: &str) -> Result<(), PipelineError> {
        validate_user(user_id)?;
        let prefixes = [
            format!("ashare-backfill:{}:{}", user_id.len(), user_id),
            format!("ashare-acquisition:{}:{}", user_id.len(), user_id),
        ];
        let deadline = Instant::now() + Duration::from_secs(35);
        loop {
            let active = self.active.lock().map_err(lock_error)?;
            let tokens = active
                .iter()
                .filter(|(key, _)| prefixes.iter().any(|prefix| key.starts_with(prefix)))
                .map(|(key, token)| (key.clone(), token.clone()))
                .collect::<Vec<_>>();
            drop(active);
            if tokens.is_empty() {
                return Ok(());
            }
            for (key, token) in tokens {
                token.cancel();
                self.pipeline.cancel(&key)?;
            }
            if Instant::now() >= deadline {
                return Err(PipelineError::Storage(
                    "Timed out waiting for A-share operations to stop".into(),
                ));
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    pub fn begin_user_reset(&self, user_id: &str) -> Result<(), PipelineError> {
        validate_user(user_id)?;
        if !self
            .resetting
            .lock()
            .map_err(lock_error)?
            .insert(user_id.to_owned())
        {
            return Err(PipelineError::InvalidRequest(
                "A-share reset is already in progress for this user".into(),
            ));
        }
        if let Err(error) = self.cancel_user_operations(user_id) {
            self.resetting.lock().map_err(lock_error)?.remove(user_id);
            return Err(error);
        }
        Ok(())
    }

    pub fn finish_user_reset(&self, user_id: &str) -> Result<(), PipelineError> {
        validate_user(user_id)?;
        self.resetting.lock().map_err(lock_error)?.remove(user_id);
        Ok(())
    }

    fn ensure_user_available(&self, user_id: &str) -> Result<(), PipelineError> {
        if self.resetting.lock().map_err(lock_error)?.contains(user_id) {
            return Err(PipelineError::InvalidRequest(
                "A-share data is being reset for this user".into(),
            ));
        }
        Ok(())
    }

    fn connector_error(&self, error: adaq_data_core::DataError) -> PipelineError {
        let response_sha256 = error
            .response_sha256
            .clone()
            .or_else(|| error.raw_response.as_ref().map(|bytes| digest(bytes)));
        if let Err(storage_error) = self.retain_connector_error_response(&error) {
            return storage_error;
        }
        let mut pipeline_error = connector_error(error);
        if let (Some(response_sha256), PipelineError::Connector { message, .. }) =
            (response_sha256, &mut pipeline_error)
        {
            *message = format!("{message}; rawResponseSha256={response_sha256}");
        }
        pipeline_error
    }

    fn retain_connector_error_response(
        &self,
        error: &adaq_data_core::DataError,
    ) -> Result<(), PipelineError> {
        let Some(raw_response) = error.raw_response.as_deref() else {
            return Ok(());
        };
        let actual_hash = digest(raw_response);
        if error
            .response_sha256
            .as_deref()
            .is_some_and(|expected_hash| expected_hash != actual_hash)
        {
            return Err(PipelineError::InvalidRequest(
                "A-share connector raw response hash does not match retained bytes".into(),
            ));
        }
        self.retain_raw_response(&actual_hash, raw_response)
    }

    pub fn reset_paths_for_user(&self, user_id: &str) -> Result<Vec<PathBuf>, PipelineError> {
        validate_user(user_id)?;
        let database = self.pipeline.database();
        let database = database.lock().map_err(lock_error)?;
        self.reset_paths_for_user_with_connection(&database, user_id)
    }

    pub fn reset_paths_for_user_with_connection(
        &self,
        database: &Connection,
        user_id: &str,
    ) -> Result<Vec<PathBuf>, PipelineError> {
        validate_user(user_id)?;
        let sources = [
            (
                "SELECT current.snapshot_json
                 FROM ashare_instrument_master_snapshots current
                 WHERE current.user_id = ?1
                   AND NOT EXISTS (
                       SELECT 1 FROM ashare_instrument_master_snapshots other
                       WHERE other.user_id <> ?1
                         AND json_extract(other.snapshot_json, '$.evidencePath')
                             = json_extract(current.snapshot_json, '$.evidencePath')
                   )",
                "evidencePath",
            ),
            (
                "SELECT current.snapshot_json
                 FROM ashare_calendar_snapshots current
                 WHERE current.user_id = ?1
                   AND NOT EXISTS (
                       SELECT 1 FROM ashare_calendar_snapshots other
                       WHERE other.user_id <> ?1
                         AND json_extract(other.snapshot_json, '$.evidencePath')
                             = json_extract(current.snapshot_json, '$.evidencePath')
                   )",
                "evidencePath",
            ),
            (
                "SELECT current.dataset_json
                 FROM ashare_corporate_actions current
                 WHERE current.user_id = ?1
                   AND NOT EXISTS (
                       SELECT 1 FROM ashare_corporate_actions other
                       WHERE other.user_id <> ?1
                         AND json_extract(other.dataset_json, '$.evidencePath')
                             = json_extract(current.dataset_json, '$.evidencePath')
                   )",
                "evidencePath",
            ),
            (
                "SELECT current.checkpoint_json
                 FROM ashare_backfill_checkpoints current
                 WHERE current.user_id = ?1
                   AND NOT EXISTS (
                       SELECT 1 FROM ashare_backfill_checkpoints other
                       WHERE other.user_id <> ?1
                         AND json_extract(other.checkpoint_json, '$.acquisitionPath')
                             = json_extract(current.checkpoint_json, '$.acquisitionPath')
                   )",
                "acquisitionPath",
            ),
        ];
        let mut paths = Vec::new();
        for (sql, field) in sources {
            let mut statement = database.prepare(sql).map_err(storage)?;
            let rows = statement
                .query_map([user_id], |row| row.get::<_, String>(0))
                .map_err(storage)?;
            for row in rows {
                let value: Value = serde_json::from_str(&row.map_err(storage)?).map_err(storage)?;
                if let Some(path) = value.get(field).and_then(Value::as_str) {
                    paths.push(PathBuf::from(path));
                }
            }
        }
        paths.extend(self.reset_raw_response_paths_for_user(database, user_id)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }

    fn reset_raw_response_paths_for_user(
        &self,
        database: &Connection,
        user_id: &str,
    ) -> Result<Vec<PathBuf>, PipelineError> {
        let queries = [
            "SELECT a.user_id, s.source_json
             FROM pipeline_sources s
             JOIN pipeline_source_access a USING(source_id)",
            "SELECT user_id, snapshot_json FROM ashare_instrument_master_snapshots",
            "SELECT user_id, snapshot_json FROM ashare_calendar_snapshots",
            "SELECT user_id, dataset_json FROM ashare_corporate_actions",
            "SELECT user_id, checkpoint_json FROM ashare_backfill_checkpoints",
        ];
        let mut current_hashes = HashSet::new();
        let mut other_hashes = HashSet::new();
        for sql in queries {
            let mut statement = database.prepare(sql).map_err(storage)?;
            let rows = statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(storage)?;
            for row in rows {
                let (row_user_id, json) = row.map_err(storage)?;
                let value: Value = serde_json::from_str(&json).map_err(storage)?;
                let hashes = if row_user_id == user_id {
                    &mut current_hashes
                } else {
                    &mut other_hashes
                };
                collect_raw_response_hashes(&value, hashes);
                if let Some(path) = value
                    .get("acquisitionPath")
                    .and_then(Value::as_str)
                    .map(PathBuf::from)
                    .filter(|path| path.is_file())
                {
                    let acquisition: Value =
                        serde_json::from_slice(&fs::read(path).map_err(storage)?)
                            .map_err(storage)?;
                    collect_raw_response_hashes(&acquisition, hashes);
                }
            }
        }
        Ok(current_hashes
            .difference(&other_hashes)
            .map(|hash| self.raw_response_path(hash))
            .filter(|path| path.is_file())
            .collect())
    }

    pub fn reset_user_rows(
        &self,
        transaction: &Transaction<'_>,
        user_id: &str,
    ) -> Result<(), PipelineError> {
        validate_user(user_id)?;
        for table in [
            "ashare_instrument_master_snapshots",
            "ashare_calendar_snapshots",
            "ashare_corporate_actions",
            "ashare_backfill_checkpoints",
        ] {
            transaction
                .execute(
                    &format!("DELETE FROM {table} WHERE user_id = ?1"),
                    [user_id],
                )
                .map_err(storage)?;
        }
        Ok(())
    }

    fn retain_raw_response(
        &self,
        response_sha256: &str,
        bytes: &[u8],
    ) -> Result<(), PipelineError> {
        if response_sha256.trim().is_empty() || bytes.is_empty() {
            return Err(PipelineError::InvalidRequest(
                "A-share raw response evidence must be non-empty".into(),
            ));
        }
        if digest(bytes) != response_sha256 {
            return Err(PipelineError::InvalidRequest(
                "A-share raw response hash does not match provenance".into(),
            ));
        }
        let path = self.raw_response_path(response_sha256);
        super::atomic_write(&path, bytes)
    }

    fn retain_raw_responses(
        &self,
        response_sha256s: &[String],
        raw_responses: &[Vec<u8>],
    ) -> Result<(), PipelineError> {
        if response_sha256s.is_empty() || response_sha256s.len() != raw_responses.len() {
            return Err(PipelineError::InvalidRequest(
                "A-share raw responses must match non-empty provenance hashes".into(),
            ));
        }
        for (response_sha256, bytes) in response_sha256s.iter().zip(raw_responses) {
            self.retain_raw_response(response_sha256, bytes)?;
        }
        Ok(())
    }

    fn verify_raw_response(&self, response_sha256: &str) -> Result<(), PipelineError> {
        let path = self.raw_response_path(response_sha256);
        if !path.is_file() || digest(&fs::read(&path).map_err(storage)?) != response_sha256 {
            return Err(PipelineError::Storage(
                "A-share retained raw response evidence is missing or corrupt".into(),
            ));
        }
        Ok(())
    }

    fn raw_response_path(&self, response_sha256: &str) -> PathBuf {
        self.pipeline
            .root_dir()
            .join("a-share/raw")
            .join(format!("{response_sha256}.bin"))
    }

    pub fn begin_backfill(
        &self,
        user_id: &str,
        task_id: &str,
    ) -> Result<CancellationToken, PipelineError> {
        validate_user(user_id)?;
        self.ensure_user_available(user_id)?;
        let key = active_key(user_id, task_id);
        let token = self.pipeline.begin_attempt(&key, user_id)?;
        self.active
            .lock()
            .map_err(lock_error)?
            .insert(key, token.clone());
        Ok(token)
    }

    pub fn cancel_backfill(&self, user_id: &str, task_id: &str) -> Result<(), PipelineError> {
        validate_user(user_id)?;
        let key = active_key(user_id, task_id);
        self.pipeline.cancel(&key)?;
        if let Some(token) = self.active.lock().map_err(lock_error)?.get(&key) {
            token.cancel();
        }
        Ok(())
    }

    pub fn finish_backfill(&self, user_id: &str, task_id: &str) -> Result<(), PipelineError> {
        validate_user(user_id)?;
        let key = active_key(user_id, task_id);
        self.active.lock().map_err(lock_error)?.remove(&key);
        self.pipeline.finish_attempt(&key)
    }

    pub fn begin_acquisition(
        &self,
        user_id: &str,
        operation_id: &str,
    ) -> Result<CancellationToken, PipelineError> {
        validate_user(user_id)?;
        self.ensure_user_available(user_id)?;
        validate_operation_id(operation_id)?;
        let key = acquisition_key(user_id, operation_id);
        let token = self.pipeline.begin_attempt(&key, user_id)?;
        self.active
            .lock()
            .map_err(lock_error)?
            .insert(key, token.clone());
        Ok(token)
    }

    pub fn cancel_acquisition(
        &self,
        user_id: &str,
        operation_id: &str,
    ) -> Result<(), PipelineError> {
        validate_user(user_id)?;
        validate_operation_id(operation_id)?;
        let key = acquisition_key(user_id, operation_id);
        self.pipeline.cancel(&key)?;
        if let Some(token) = self.active.lock().map_err(lock_error)?.get(&key) {
            token.cancel();
        }
        Ok(())
    }

    pub fn finish_acquisition(
        &self,
        user_id: &str,
        operation_id: &str,
    ) -> Result<(), PipelineError> {
        validate_user(user_id)?;
        validate_operation_id(operation_id)?;
        let key = acquisition_key(user_id, operation_id);
        self.active.lock().map_err(lock_error)?.remove(&key);
        self.pipeline.finish_attempt(&key)
    }

    pub async fn acquire_instrument_master(
        &self,
        user_id: &str,
    ) -> Result<AshareInstrumentMasterSnapshot, PipelineError> {
        self.acquire_instrument_master_with_cancel(user_id, || false)
            .await
    }

    pub async fn acquire_instrument_master_with_cancel<F>(
        &self,
        user_id: &str,
        is_cancelled: F,
    ) -> Result<AshareInstrumentMasterSnapshot, PipelineError>
    where
        F: Fn() -> bool,
    {
        validate_user(user_id)?;
        self.ensure_user_available(user_id)?;
        let acquisition = self
            .client
            .acquire_instrument_master_at_with_cancel(now_ms(), &is_cancelled)
            .await
            .map_err(|error| self.connector_error(error))?;
        self.ensure_user_available(user_id)?;
        self.record_instrument_master(user_id, acquisition)
    }

    pub fn record_instrument_master(
        &self,
        user_id: &str,
        acquisition: AshareInstrumentMasterAcquisition,
    ) -> Result<AshareInstrumentMasterSnapshot, PipelineError> {
        validate_user(user_id)?;
        self.ensure_user_available(user_id)?;
        validate_master(&acquisition)?;
        let raw_response = acquisition.raw_response.as_deref().ok_or_else(|| {
            PipelineError::InvalidRequest(
                "A-share Instrument Master raw response evidence is required".into(),
            )
        })?;
        let parsed_response = acquisition.parsed_response.as_deref().ok_or_else(|| {
            PipelineError::InvalidRequest(
                "A-share Instrument Master parsed response evidence is required".into(),
            )
        })?;
        self.retain_raw_response(&acquisition.response_sha256, raw_response)?;
        if digest(parsed_response) != acquisition.parsed_response_sha256 {
            return Err(PipelineError::InvalidRequest(
                "A-share Instrument Master parsed response hash does not match retained evidence"
                    .into(),
            ));
        }
        self.retain_raw_response(&acquisition.parsed_response_sha256, parsed_response)?;
        let provider = acquisition.provider.clone();
        let content_bytes = serde_json::to_vec(&acquisition.instruments).map_err(storage)?;
        let input_content_sha256 = digest(&content_bytes);
        if acquisition.content_sha256 != input_content_sha256 {
            return Err(PipelineError::InvalidRequest(
                "A-share Instrument Master content hash does not match retained instruments".into(),
            ));
        }
        let content_sha256 = input_content_sha256;
        let capability_snapshot = capability_snapshot(
            &provider,
            &acquisition.instruments,
            &acquisition.limitations,
            acquisition.retrieved_at_ms,
            &["ordinary-equity-instrument-master"],
        );
        let snapshot_id = digest(&canonical_json_bytes(&(
            &acquisition.provider,
            &acquisition.actual_upstream,
            &acquisition.method,
            &acquisition.connector_version,
            &acquisition.request_parameters,
            acquisition.retrieved_at_ms,
            &acquisition.response_sha256,
            &acquisition.parsed_response_sha256,
            &content_sha256,
            &acquisition.diagnostics,
            &capability_snapshot,
            &acquisition.limitations,
            &acquisition.instruments,
        ))?);
        let evidence_path = self
            .pipeline
            .root_dir()
            .join("a-share/instrument-master")
            .join(format!("{snapshot_id}.json"));
        let snapshot = AshareInstrumentMasterSnapshot {
            snapshot_id: snapshot_id.clone(),
            effective_at_ms: acquisition.retrieved_at_ms,
            provider: provider.clone(),
            actual_upstream: acquisition.actual_upstream,
            method: acquisition.method,
            connector_version: acquisition.connector_version,
            request_parameters: acquisition.request_parameters,
            response_sha256: acquisition.response_sha256,
            parsed_response_sha256: acquisition.parsed_response_sha256,
            content_sha256,
            diagnostics: acquisition.diagnostics,
            capability_snapshot,
            evidence_state: UniverseEvidenceState::Observed,
            instruments: acquisition.instruments,
            limitations: acquisition.limitations,
            evidence_path,
        };
        let bytes = canonical_json_bytes(&snapshot)?;
        super::atomic_write(&snapshot.evidence_path, &bytes)?;
        let json = serde_json::to_string(&snapshot).map_err(storage)?;
        let database = self.pipeline.database();
        database
            .lock()
            .map_err(lock_error)?
            .execute(
                "INSERT OR IGNORE INTO ashare_instrument_master_snapshots
             (user_id, snapshot_id, retrieved_at_ms, snapshot_json)
             VALUES (?1, ?2, ?3, ?4)",
                params![
                    user_id,
                    snapshot.snapshot_id,
                    snapshot.effective_at_ms,
                    json
                ],
            )
            .map_err(storage)?;
        Ok(snapshot)
    }

    pub fn list_instrument_master_snapshots(
        &self,
        user_id: &str,
    ) -> Result<Vec<AshareInstrumentMasterSnapshot>, PipelineError> {
        validate_user(user_id)?;
        let database = self.pipeline.database();
        let database_guard = database.lock().map_err(lock_error)?;
        let mut statement = database_guard
            .prepare(
                "SELECT snapshot_json FROM ashare_instrument_master_snapshots
                 WHERE user_id = ?1 ORDER BY retrieved_at_ms, snapshot_id",
            )
            .map_err(storage)?;
        let snapshots = statement
            .query_map([user_id], |row| row.get::<_, String>(0))
            .map_err(storage)?
            .map(|row| {
                let json = row.map_err(storage)?;
                serde_json::from_str(&json).map_err(storage)
            })
            .collect::<Result<Vec<AshareInstrumentMasterSnapshot>, PipelineError>>()?;
        drop(statement);
        for snapshot in &snapshots {
            verify_json_evidence(
                &snapshot.evidence_path,
                snapshot,
                "A-share Instrument Master",
            )?;
            self.verify_raw_response(&snapshot.response_sha256)?;
            self.verify_raw_response(&snapshot.parsed_response_sha256)?;
        }
        Ok(snapshots)
    }

    pub fn point_in_time_universe(
        &self,
        user_id: &str,
        observation_time_ms: i64,
    ) -> Result<AshareInstrumentMasterSnapshot, PipelineError> {
        validate_user(user_id)?;
        let database = self.pipeline.database();
        let json: String = database
            .lock()
            .map_err(lock_error)?
            .query_row(
                "SELECT snapshot_json FROM ashare_instrument_master_snapshots
                 WHERE user_id = ?1 AND retrieved_at_ms <= ?2
                 ORDER BY retrieved_at_ms DESC, snapshot_id DESC LIMIT 1",
                params![user_id, observation_time_ms],
                |row| row.get(0),
            )
            .optional()
            .map_err(storage)?
            .ok_or_else(|| {
                PipelineError::NotFound(
                    "A-share Instrument Master evidence for the requested observation time".into(),
                )
            })?;
        let snapshot: AshareInstrumentMasterSnapshot =
            serde_json::from_str(&json).map_err(storage)?;
        verify_json_evidence(
            &snapshot.evidence_path,
            &snapshot,
            "A-share Instrument Master",
        )?;
        self.verify_raw_response(&snapshot.response_sha256)?;
        self.verify_raw_response(&snapshot.parsed_response_sha256)?;
        Ok(snapshot)
    }

    pub fn point_in_time_membership(
        &self,
        user_id: &str,
        observation_time_ms: i64,
    ) -> Result<AsharePointInTimeUniverse, PipelineError> {
        validate_user(user_id)?;
        if observation_time_ms < 0 {
            return Err(PipelineError::InvalidRequest(
                "A-share universe observation time must be non-negative".into(),
            ));
        }
        let snapshot = self
            .list_instrument_master_snapshots(user_id)?
            .into_iter()
            .filter(|snapshot| snapshot.effective_at_ms <= observation_time_ms)
            .max_by_key(|snapshot| (snapshot.effective_at_ms, snapshot.snapshot_id.clone()));
        let Some(snapshot) = snapshot else {
            return Ok(AsharePointInTimeUniverse {
                universe_id: digest(&canonical_json_bytes(&(observation_time_ms, "unknown"))?),
                as_of_ms: observation_time_ms,
                snapshot_id: None,
                evidence_state: UniverseEvidenceState::Unknown,
                instruments: Vec::new(),
            });
        };
        let evidence_state = if snapshot.effective_at_ms == observation_time_ms {
            UniverseEvidenceState::Observed
        } else {
            UniverseEvidenceState::Reconstructed
        };
        let instruments = snapshot
            .instruments
            .into_iter()
            .filter(|instrument| instrument.status == adaq_data_core::InstrumentStatus::Live)
            .collect::<Vec<_>>();
        let universe_id = digest(&canonical_json_bytes(&(
            observation_time_ms,
            &snapshot.snapshot_id,
            &evidence_state,
            &instruments,
        ))?);
        Ok(AsharePointInTimeUniverse {
            universe_id,
            as_of_ms: observation_time_ms,
            snapshot_id: Some(snapshot.snapshot_id),
            evidence_state,
            instruments,
        })
    }

    fn retained_master_instrument(
        &self,
        user_id: &str,
        observation_time_ms: i64,
        instrument: &InstrumentId,
    ) -> Result<bool, PipelineError> {
        let snapshots = self.list_instrument_master_snapshots(user_id)?;
        let Some(snapshot) = snapshots
            .iter()
            .filter(|snapshot| snapshot.effective_at_ms <= observation_time_ms)
            .max_by_key(|snapshot| (snapshot.effective_at_ms, snapshot.snapshot_id.clone()))
        else {
            return Ok(false);
        };
        let Some(master_instrument) = snapshot
            .instruments
            .iter()
            .find(|value| value.instrument == *instrument)
        else {
            return Err(PipelineError::NotFound(
                "A-share Instrument mapping is absent from retained master evidence".into(),
            ));
        };
        if snapshot.effective_at_ms == observation_time_ms
            && master_instrument.status != adaq_data_core::InstrumentStatus::Live
        {
            return Err(PipelineError::NotFound(
                "A-share Instrument is not live in the point-in-time master".into(),
            ));
        }
        Ok(master_instrument.status == adaq_data_core::InstrumentStatus::Live)
    }

    fn checkpoint_publication_is_intact(&self, checkpoint: &AshareBackfillCheckpoint) -> bool {
        if checkpoint.acquisition_path.is_some()
            && self
                .load_checkpoint_acquisition(checkpoint)
                .ok()
                .flatten()
                .is_none()
        {
            return false;
        }
        let (Some(source_id), Some(canonical_id), Some(revision)) = (
            checkpoint.source_id.as_deref(),
            checkpoint.canonical_id.as_deref(),
            checkpoint.revision,
        ) else {
            return false;
        };
        if checkpoint.completed_through_ms != Some(checkpoint.request.end_time_ms) {
            return false;
        }
        let Ok(source) = self
            .pipeline
            .source_for_user(&checkpoint.user_id, source_id)
        else {
            return false;
        };
        if source.revision != revision {
            return false;
        }
        let Ok(canonical) = self
            .pipeline
            .canonical_for_user(&checkpoint.user_id, canonical_id)
        else {
            return false;
        };
        if canonical.source_id != source.source_id || canonical.revision != revision {
            return false;
        }
        let Ok(quality) = self
            .pipeline
            .quality_for_user(&checkpoint.user_id, &canonical.quality_report_id)
        else {
            return false;
        };
        matches!(
            (&checkpoint.state, quality.state),
            (AshareBackfillState::Completed, DataQualityState::Passed)
                | (AshareBackfillState::Degraded, DataQualityState::Degraded)
        )
    }

    pub async fn acquire_calendar(
        &self,
        user_id: &str,
        range: HistoricalBarRange,
    ) -> Result<Vec<AshareCalendarSnapshot>, PipelineError> {
        self.acquire_calendar_with_cancel(user_id, range, || false)
            .await
    }

    pub async fn acquire_calendar_with_cancel<F>(
        &self,
        user_id: &str,
        range: HistoricalBarRange,
        is_cancelled: F,
    ) -> Result<Vec<AshareCalendarSnapshot>, PipelineError>
    where
        F: Fn() -> bool,
    {
        validate_user(user_id)?;
        self.ensure_user_available(user_id)?;
        let acquisition = self
            .client
            .acquire_calendar_with_cancel(range, now_ms(), &is_cancelled)
            .await
            .map_err(|error| self.connector_error(error))?;
        self.ensure_user_available(user_id)?;
        self.record_calendar(user_id, acquisition)
    }

    pub fn record_calendar(
        &self,
        user_id: &str,
        acquisition: AshareCalendarAcquisition,
    ) -> Result<Vec<AshareCalendarSnapshot>, PipelineError> {
        validate_user(user_id)?;
        self.ensure_user_available(user_id)?;
        validate_calendar(&acquisition)?;
        let raw_response = acquisition.raw_response.as_deref().ok_or_else(|| {
            PipelineError::InvalidRequest(
                "A-share calendar raw response evidence is required".into(),
            )
        })?;
        self.retain_raw_response(&acquisition.response_sha256, raw_response)?;
        let content_sha256 = digest(&serde_json::to_vec(&acquisition.snapshots).map_err(storage)?);
        if acquisition.content_sha256 != content_sha256 {
            return Err(PipelineError::InvalidRequest(
                "A-share calendar content hash does not match retained snapshots".into(),
            ));
        }
        let capability_snapshot = ProviderCapabilitySnapshot {
            provider: acquisition.provider.clone(),
            captured_at_ms: acquisition.retrieved_at_ms,
            venues: acquisition
                .snapshots
                .iter()
                .map(|value| value.venue.id.clone())
                .collect(),
            record_types: vec!["trading-calendar".into(), "session".into()],
            history_start_ms: acquisition
                .snapshots
                .iter()
                .map(|value| value.effective_from_ms)
                .min(),
            delayed: false,
            delayed_known: false,
            delay_ms: None,
            rate_limit: None,
            rate_limit_known: false,
            streaming_symbol_limit: None,
            limitations: acquisition.limitations.clone(),
        };
        let mut snapshots = Vec::with_capacity(acquisition.snapshots.len());
        for snapshot in acquisition.snapshots {
            let snapshot_id = snapshot.snapshot_id.clone();
            let evidence_path = self
                .pipeline
                .root_dir()
                .join("a-share/calendars")
                .join(format!("{snapshot_id}.json"));
            let record = AshareCalendarSnapshot {
                snapshot,
                provider: acquisition.provider.clone(),
                actual_upstream: acquisition.actual_upstream.clone(),
                method: acquisition.method.clone(),
                connector_version: acquisition.connector_version.clone(),
                request_parameters: acquisition.request_parameters.clone(),
                retrieved_at_ms: acquisition.retrieved_at_ms,
                response_sha256: acquisition.response_sha256.clone(),
                content_sha256: acquisition.content_sha256.clone(),
                diagnostics: acquisition.diagnostics.clone(),
                capability_snapshot: capability_snapshot.clone(),
                limitations: acquisition.limitations.clone(),
                evidence_path,
            };
            snapshots.push(record);
        }
        let evidence = snapshots
            .iter()
            .map(|record| {
                Ok::<_, PipelineError>((
                    record.evidence_path.clone(),
                    canonical_json_bytes(record)?,
                    serde_json::to_string(record).map_err(storage)?,
                ))
            })
            .collect::<Result<Vec<_>, PipelineError>>()?;
        let new_paths = evidence
            .iter()
            .filter(|(path, _, _)| !path.is_file())
            .map(|(path, _, _)| path.clone())
            .collect::<Vec<_>>();
        let database = self.pipeline.database();
        let mut database = database.lock().map_err(lock_error)?;
        let result = (|| {
            // Hold the database mutex through the immutable file check and
            // catalog insert so two local acquisitions cannot race on one
            // snapshot identity.
            for (path, bytes, _) in &evidence {
                if path.is_file() && fs::read(path).map_err(storage)? != *bytes {
                    return Err(PipelineError::InvalidRequest(
                        "A-share calendar evidence identity collides with different content".into(),
                    ));
                }
            }
            for (record, (_, _, json)) in snapshots.iter().zip(evidence.iter()) {
                let existing = database
                    .query_row(
                        "SELECT snapshot_json FROM ashare_calendar_snapshots
                         WHERE user_id = ?1 AND snapshot_id = ?2",
                        params![user_id, record.snapshot.snapshot_id],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()
                    .map_err(storage)?;
                if existing.is_some_and(|existing| existing != *json) {
                    return Err(PipelineError::InvalidRequest(
                        "A-share calendar snapshot identity is immutable".into(),
                    ));
                }
            }
            for (path, bytes, _) in &evidence {
                super::atomic_write(path, bytes)?;
            }
            let transaction = database
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(storage)?;
            for (record, (_, _, json)) in snapshots.iter().zip(evidence.iter()) {
                transaction
                    .execute(
                        "INSERT OR IGNORE INTO ashare_calendar_snapshots
                         (user_id, snapshot_id, retrieved_at_ms, snapshot_json)
                         VALUES (?1, ?2, ?3, ?4)",
                        params![
                            user_id,
                            record.snapshot.snapshot_id,
                            record.retrieved_at_ms,
                            json
                        ],
                    )
                    .map_err(storage)?;
            }
            transaction.commit().map_err(storage)
        })();
        if let Err(error) = result {
            for path in new_paths {
                let _ = fs::remove_file(path);
            }
            return Err(error);
        }
        Ok(snapshots)
    }

    pub async fn acquire_corporate_actions(
        &self,
        user_id: &str,
        instrument: InstrumentId,
    ) -> Result<AshareCorporateActionDataset, PipelineError> {
        self.acquire_corporate_actions_with_cancel(user_id, instrument, || false)
            .await
    }

    pub async fn acquire_corporate_actions_with_cancel<F>(
        &self,
        user_id: &str,
        instrument: InstrumentId,
        is_cancelled: F,
    ) -> Result<AshareCorporateActionDataset, PipelineError>
    where
        F: Fn() -> bool,
    {
        validate_user(user_id)?;
        self.ensure_user_available(user_id)?;
        let acquisition = self
            .client
            .acquire_corporate_actions_with_cancel(instrument, now_ms(), &is_cancelled)
            .await
            .map_err(|error| self.connector_error(error))?;
        self.ensure_user_available(user_id)?;
        self.record_corporate_actions(user_id, acquisition)
    }

    pub fn record_corporate_actions(
        &self,
        user_id: &str,
        mut acquisition: AshareCorporateActionAcquisition,
    ) -> Result<AshareCorporateActionDataset, PipelineError> {
        validate_user(user_id)?;
        self.ensure_user_available(user_id)?;
        validate_corporate_actions(&acquisition)?;
        let raw_response = acquisition.raw_response.as_deref().ok_or_else(|| {
            PipelineError::InvalidRequest(
                "A-share corporate-action raw response evidence is required".into(),
            )
        })?;
        self.retain_raw_response(&acquisition.response_sha256, raw_response)?;
        let content_bytes =
            serde_json::to_vec(&(&acquisition.records, &acquisition.invalid_records))
                .map_err(storage)?;
        let content_sha256 = digest(&content_bytes);
        if acquisition.content_sha256 != content_sha256 {
            return Err(PipelineError::InvalidRequest(
                "A-share corporate-action content hash does not match retained records".into(),
            ));
        }
        let mut valid_records = Vec::with_capacity(acquisition.records.len());
        let mut quarantined_records = acquisition.invalid_records;
        for record in acquisition.records {
            if corporate_action_record_is_valid(&record) {
                valid_records.push(record);
            } else {
                quarantined_records.push(record);
            }
        }
        acquisition.records = valid_records;
        acquisition.invalid_records = quarantined_records;
        let content_sha256 = digest(
            &serde_json::to_vec(&(&acquisition.records, &acquisition.invalid_records))
                .map_err(storage)?,
        );
        let logical_key = digest(&canonical_json_bytes(&(
            &acquisition.instrument,
            &acquisition.provider,
            &acquisition.actual_upstream,
            &acquisition.method,
            &acquisition.connector_version,
            &acquisition.request_parameters,
            &acquisition.diagnostics,
            &acquisition.limitations,
            acquisition
                .records
                .first()
                .or_else(|| acquisition.invalid_records.first())
                .map(|value| &value.instrument),
        ))?);
        let database = self.pipeline.database();
        let mut database_guard = database.lock().map_err(lock_error)?;
        let transaction = database_guard
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        let revision: u64 = transaction
            .query_row(
                "SELECT COALESCE(MAX(revision), 0) FROM ashare_corporate_actions
                 WHERE user_id = ?1 AND logical_key = ?2",
                params![user_id, logical_key],
                |row| row.get::<_, i64>(0),
            )
            .map_err(storage)?
            .max(0) as u64
            + 1;
        let dataset_id = digest(&canonical_json_bytes(&(
            revision,
            &logical_key,
            &acquisition.response_sha256,
            &content_sha256,
        ))?);
        let evidence_path = self
            .pipeline
            .root_dir()
            .join("a-share/corporate-actions")
            .join(format!("{dataset_id}.json"));
        let quality = if acquisition.records.is_empty() {
            DataQualityState::Rejected
        } else if !acquisition.invalid_records.is_empty() || !acquisition.limitations.is_empty() {
            DataQualityState::Degraded
        } else {
            DataQualityState::Passed
        };
        let dataset = AshareCorporateActionDataset {
            dataset_id: dataset_id.clone(),
            revision,
            logical_key,
            instrument: acquisition.instrument.clone(),
            provider: acquisition.provider.clone(),
            actual_upstream: acquisition.actual_upstream,
            method: acquisition.method,
            connector_version: acquisition.connector_version,
            request_parameters: acquisition.request_parameters,
            retrieved_at_ms: acquisition.retrieved_at_ms,
            response_sha256: acquisition.response_sha256,
            content_sha256,
            diagnostics: acquisition.diagnostics,
            capability_snapshot: capability_snapshot(
                &acquisition.provider,
                &[],
                &acquisition.limitations,
                acquisition.retrieved_at_ms,
                &["corporate-actions"],
            ),
            quality,
            records: acquisition.records,
            quarantined_records: acquisition.invalid_records,
            limitations: acquisition.limitations,
            evidence_path,
        };
        let bytes = canonical_json_bytes(&dataset)?;
        super::atomic_write(&dataset.evidence_path, &bytes)?;
        let json = serde_json::to_string(&dataset).map_err(storage)?;
        transaction
            .execute(
                "INSERT INTO ashare_corporate_actions
                 (user_id, dataset_id, logical_key, revision, dataset_json)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    user_id,
                    dataset.dataset_id,
                    dataset.logical_key,
                    dataset.revision as i64,
                    json
                ],
            )
            .map_err(storage)?;
        transaction.commit().map_err(storage)?;
        Ok(dataset)
    }

    pub async fn backfill(
        &self,
        request: &AshareBackfillRequest,
        cancellation: CancellationToken,
        mut on_event: impl FnMut(AshareBackfillEvent),
    ) -> Result<Option<PipelinePublication>, PipelineError> {
        let _activity_guard = BackfillActivityGuard {
            pipeline: self.pipeline.clone(),
            active: self.active.clone(),
            key: active_key(&request.user_id, &request.task_id),
        };
        validate_backfill_request(request)?;
        self.ensure_user_available(&request.user_id)?;
        let checkpoint = self.read_checkpoint(&request.user_id, &request.task_id)?;
        if let Some(checkpoint) = checkpoint.as_ref() {
            if checkpoint.request != *request {
                return Err(PipelineError::InvalidRequest(
                    "A-share task ID is already bound to a different request".into(),
                ));
            }
            match checkpoint.state {
                AshareBackfillState::Completed | AshareBackfillState::Degraded => {
                    if self.checkpoint_publication_is_intact(checkpoint) {
                        if let Some(source_id) = checkpoint.source_id.clone() {
                            on_event(AshareBackfillEvent::AlreadyCompleted {
                                task_id: request.task_id.clone(),
                                source_id,
                            });
                        }
                        self.finish_backfill(&request.user_id, &request.task_id)?;
                        return Ok(None);
                    }
                }
                AshareBackfillState::Running => {
                    let active = self.active.lock().map_err(lock_error)?;
                    if active
                        .get(&active_key(&request.user_id, &request.task_id))
                        .is_some_and(|active_token| !active_token.is_same(&cancellation))
                    {
                        return Err(PipelineError::InvalidRequest(
                            "A-share backfill task is already running".into(),
                        ));
                    }
                }
                AshareBackfillState::Rejected
                | AshareBackfillState::Cancelled
                | AshareBackfillState::Failed => {}
            }
        }
        let resume_acquisition = checkpoint
            .as_ref()
            .map(|checkpoint| self.load_checkpoint_acquisition(checkpoint))
            .transpose()?
            .flatten();
        let acquisition_path = resume_acquisition.as_ref().and_then(|_| {
            checkpoint
                .as_ref()
                .and_then(|checkpoint| checkpoint.acquisition_path.clone())
        });
        if cancellation.is_cancelled() {
            self.mark_checkpoint_state_with_path(
                request,
                AshareBackfillState::Cancelled,
                Some("cancelled"),
                acquisition_path.clone(),
            )?;
            on_event(AshareBackfillEvent::Cancelled {
                task_id: request.task_id.clone(),
            });
            self.finish_backfill(&request.user_id, &request.task_id)?;
            return Err(PipelineError::Cancelled {
                source_id: request.task_id.clone(),
            });
        }
        self.require_recorded_calendar(&request.user_id, &request.calendar)?;
        let membership_observed = self.retained_master_instrument(
            &request.user_id,
            request.start_time_ms,
            &request.instrument,
        )?;
        if resume_acquisition.is_none() {
            self.write_checkpoint(&AshareBackfillCheckpoint {
                task_id: request.task_id.clone(),
                user_id: request.user_id.clone(),
                request: request.clone(),
                state: AshareBackfillState::Running,
                source_id: None,
                canonical_id: None,
                revision: None,
                completed_through_ms: None,
                acquisition_path: None,
                acquisition_sha256: None,
                failure_response_sha256: None,
                failure_response_path: None,
                last_error: None,
            })?;
        }
        on_event(AshareBackfillEvent::Started {
            task_id: request.task_id.clone(),
            instrument: request.instrument.clone(),
        });
        if cancellation.is_cancelled() {
            self.mark_checkpoint_state(request, AshareBackfillState::Cancelled, Some("cancelled"))?;
            on_event(AshareBackfillEvent::Cancelled {
                task_id: request.task_id.clone(),
            });
            self.finish_backfill(&request.user_id, &request.task_id)?;
            return Err(PipelineError::Cancelled {
                source_id: request.task_id.clone(),
            });
        }
        on_event(AshareBackfillEvent::AcquisitionStarted {
            instrument: request.instrument.clone(),
            interval: request.interval,
        });
        let mut acquisition = if let Some(acquisition) = resume_acquisition {
            acquisition
        } else {
            let connector_cancellation = cancellation.clone();
            match self
                .client
                .acquire_bars_with_cancel(
                    request.instrument.clone(),
                    request.interval,
                    HistoricalBarRange {
                        start_time_ms: request.start_time_ms,
                        end_time_ms: request.end_time_ms,
                    },
                    now_ms(),
                    move || connector_cancellation.is_cancelled(),
                )
                .await
            {
                Ok(value) => value,
                Err(error) => {
                    let cancelled = cancellation.is_cancelled() || error.code == "cancelled";
                    let response_sha256 = error
                        .response_sha256
                        .clone()
                        .or_else(|| error.raw_response.as_ref().map(|bytes| digest(bytes)));
                    let message = error.to_string();
                    self.retain_connector_error_response(&error)?;
                    let pipeline_error = self.connector_error(error);
                    if cancelled {
                        self.mark_connector_failure(
                            request,
                            AshareBackfillState::Cancelled,
                            &message,
                            response_sha256.clone(),
                        )?;
                        on_event(AshareBackfillEvent::Cancelled {
                            task_id: request.task_id.clone(),
                        });
                        self.finish_backfill(&request.user_id, &request.task_id)?;
                        return Err(PipelineError::Cancelled {
                            source_id: request.task_id.clone(),
                        });
                    }
                    self.mark_connector_failure(
                        request,
                        AshareBackfillState::Failed,
                        &message,
                        response_sha256,
                    )?;
                    on_event(AshareBackfillEvent::Failed {
                        task_id: request.task_id.clone(),
                        message,
                    });
                    self.finish_backfill(&request.user_id, &request.task_id)?;
                    return Err(pipeline_error);
                }
            }
        };
        if !membership_observed
            && !acquisition
                .limitations
                .iter()
                .any(|value| value.contains("Point-in-Time membership is unknown"))
        {
            acquisition.limitations.push(
                "Point-in-Time membership is unknown; no retained master snapshot proves membership at the requested observation"
                    .into(),
            );
        }
        if cancellation.is_cancelled() {
            self.mark_checkpoint_state_with_path(
                request,
                AshareBackfillState::Cancelled,
                Some("cancelled"),
                acquisition_path.clone(),
            )?;
            on_event(AshareBackfillEvent::Cancelled {
                task_id: request.task_id.clone(),
            });
            self.finish_backfill(&request.user_id, &request.task_id)?;
            return Err(PipelineError::Cancelled {
                source_id: request.task_id.clone(),
            });
        }
        if let Err(error) =
            self.retain_raw_responses(&acquisition.response_sha256s, &acquisition.raw_responses)
        {
            let state = self.record_backfill_failure(
                request,
                &error,
                &cancellation,
                acquisition_path.clone(),
            )?;
            if state == AshareBackfillState::Cancelled {
                on_event(AshareBackfillEvent::Cancelled {
                    task_id: request.task_id.clone(),
                });
            } else {
                on_event(AshareBackfillEvent::Failed {
                    task_id: request.task_id.clone(),
                    message: error.to_string(),
                });
            }
            self.finish_backfill(&request.user_id, &request.task_id)?;
            return Err(error);
        }
        let acquisition_path = if let Some(path) = acquisition_path {
            Some(path)
        } else {
            let path = self.checkpoint_acquisition_path(&request.user_id, &request.task_id);
            match self.persist_checkpoint_acquisition(request, &acquisition) {
                Ok(path) => Some(path),
                Err(error) => {
                    let state = self.record_backfill_failure(
                        request,
                        &error,
                        &cancellation,
                        path.is_file().then_some(path),
                    )?;
                    if state == AshareBackfillState::Cancelled {
                        on_event(AshareBackfillEvent::Cancelled {
                            task_id: request.task_id.clone(),
                        });
                    } else {
                        on_event(AshareBackfillEvent::Failed {
                            task_id: request.task_id.clone(),
                            message: error.to_string(),
                        });
                    }
                    self.finish_backfill(&request.user_id, &request.task_id)?;
                    return Err(error);
                }
            }
        };
        let source_acquisition = match source_acquisition(&acquisition) {
            Ok(value) => value,
            Err(error) => {
                let state = self.record_backfill_failure(
                    request,
                    &error,
                    &cancellation,
                    acquisition_path.clone(),
                )?;
                if state == AshareBackfillState::Cancelled {
                    on_event(AshareBackfillEvent::Cancelled {
                        task_id: request.task_id.clone(),
                    });
                } else {
                    on_event(AshareBackfillEvent::Failed {
                        task_id: request.task_id.clone(),
                        message: error.to_string(),
                    });
                }
                self.finish_backfill(&request.user_id, &request.task_id)?;
                return Err(error);
            }
        };
        let mut canonicalization = match CanonicalizationRequest::new(
            request.instrument.clone(),
            request.interval,
            CalendarEvidence::Venue {
                snapshot: request.calendar.clone(),
            },
        ) {
            Ok(value) => value,
            Err(error) => {
                let state = self.record_backfill_failure(
                    request,
                    &error,
                    &cancellation,
                    acquisition_path.clone(),
                )?;
                if state == AshareBackfillState::Cancelled {
                    on_event(AshareBackfillEvent::Cancelled {
                        task_id: request.task_id.clone(),
                    });
                } else {
                    on_event(AshareBackfillEvent::Failed {
                        task_id: request.task_id.clone(),
                        message: error.to_string(),
                    });
                }
                self.finish_backfill(&request.user_id, &request.task_id)?;
                return Err(error);
            }
        };
        canonicalization.historical_range = Some(HistoricalBarRange {
            start_time_ms: request.start_time_ms,
            end_time_ms: request.end_time_ms,
        });
        self.ensure_user_available(&request.user_id)?;
        let publication = match self.pipeline.publish_without_partial_source(
            &request.user_id,
            source_acquisition,
            canonicalization,
            cancellation,
            |_| {},
        ) {
            Ok(value) => value,
            Err(error) => {
                let state = if matches!(error, PipelineError::Cancelled { .. }) {
                    AshareBackfillState::Cancelled
                } else {
                    AshareBackfillState::Failed
                };
                self.mark_checkpoint_state_with_path(
                    request,
                    state.clone(),
                    Some(&error.to_string()),
                    acquisition_path.clone(),
                )?;
                if state == AshareBackfillState::Cancelled {
                    on_event(AshareBackfillEvent::Cancelled {
                        task_id: request.task_id.clone(),
                    });
                } else {
                    on_event(AshareBackfillEvent::Failed {
                        task_id: request.task_id.clone(),
                        message: error.to_string(),
                    });
                }
                self.finish_backfill(&request.user_id, &request.task_id)?;
                return Err(error);
            }
        };
        let state = match publication.quality.state {
            DataQualityState::Passed => AshareBackfillState::Completed,
            DataQualityState::Degraded => AshareBackfillState::Degraded,
            DataQualityState::Rejected => AshareBackfillState::Rejected,
        };
        self.write_checkpoint(&AshareBackfillCheckpoint {
            task_id: request.task_id.clone(),
            user_id: request.user_id.clone(),
            request: request.clone(),
            state,
            source_id: Some(publication.source.source_id.clone()),
            canonical_id: publication
                .canonical
                .as_ref()
                .map(|value| value.canonical_id.clone()),
            revision: Some(publication.source.revision),
            completed_through_ms: Some(request.end_time_ms),
            acquisition_path: None,
            acquisition_sha256: None,
            failure_response_sha256: None,
            failure_response_path: None,
            last_error: None,
        })?;
        if let Some(path) = acquisition_path {
            let _ = fs::remove_file(path);
        }
        on_event(AshareBackfillEvent::Published {
            instrument: request.instrument.clone(),
            source_id: publication.source.source_id.clone(),
            canonical_id: publication
                .canonical
                .as_ref()
                .map(|value| value.canonical_id.clone()),
            revision: publication.source.revision,
            state: publication.quality.state.clone(),
        });
        self.finish_backfill(&request.user_id, &request.task_id)?;
        Ok(Some(publication))
    }

    pub fn workspace_dto(
        &self,
        publication: &PipelinePublication,
        now_ms: i64,
    ) -> Result<AshareMarketWorkspaceDto, PipelineError> {
        let source = &publication.source;
        let quality = &publication.quality;
        let instrument = publication
            .canonical
            .as_ref()
            .map(|value| value.instrument.clone())
            .or_else(|| source.records.first().map(|value| value.instrument.clone()))
            .ok_or_else(|| {
                PipelineError::NotFound(
                    "A-share workspace DTO has no retained instrument identity".into(),
                )
            })?;
        Ok(AshareMarketWorkspaceDto {
            instrument,
            provider: source.identity.provider.clone(),
            actual_upstream: source.identity.actual_upstream.clone(),
            connector: source.identity.connector.clone(),
            connector_version: source.identity.connector_version.clone(),
            retrieved_at_ms: source.identity.retrieved_at_ms,
            freshness_ms: (now_ms >= source.identity.retrieved_at_ms)
                .then_some(now_ms - source.identity.retrieved_at_ms),
            price_basis: source.identity.price_basis,
            calendar_id: publication
                .canonical
                .as_ref()
                .map(|value| match &value.calendar {
                    CalendarEvidence::Venue { snapshot } => snapshot.snapshot_id.clone(),
                    CalendarEvidence::UtcGrid { calendar_id, .. } => calendar_id.clone(),
                })
                .unwrap_or_else(|| "unknown".into()),
            quality: quality.state.clone(),
            source_id: source.source_id.clone(),
            canonical_id: publication
                .canonical
                .as_ref()
                .map(|value| value.canonical_id.clone()),
            revision: source.revision,
            coverage_start_ms: quality.coverage.start_time_ms,
            coverage_end_ms: quality.coverage.end_time_ms,
            gap_count: quality.gap_count,
            limitations: quality.capability_limitations.clone(),
        })
    }

    pub fn workspace_dto_for_user(
        &self,
        user_id: &str,
        source_id: &str,
        now_ms: i64,
    ) -> Result<AshareMarketWorkspaceDto, PipelineError> {
        validate_user(user_id)?;
        let source = self.pipeline.source_for_user(user_id, source_id)?;
        let (report_id, canonical_id): (String, Option<String>) = self
            .pipeline
            .database()
            .lock()
            .map_err(lock_error)?
            .query_row(
                "SELECT q.report_id, c.canonical_id
                 FROM pipeline_quality_reports q
                 JOIN pipeline_quality_access qa ON qa.report_id = q.report_id
                 LEFT JOIN pipeline_canonical_datasets c ON c.source_id = q.source_id
                 WHERE qa.user_id = ?1 AND q.source_id = ?2
                 ORDER BY q.report_id DESC LIMIT 1",
                params![user_id, source_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(storage)?
            .ok_or_else(|| PipelineError::NotFound("A-share Data Quality Report".into()))?;
        let quality = self.pipeline.quality_for_user(user_id, &report_id)?;
        let canonical = canonical_id
            .as_deref()
            .map(|id| self.pipeline.canonical_for_user(user_id, id))
            .transpose()?;
        self.workspace_dto(
            &PipelinePublication {
                attempt_id: None,
                source,
                canonical,
                quality,
            },
            now_ms,
        )
    }

    fn initialize_schema(&self) -> Result<(), PipelineError> {
        let database = self.pipeline.database();
        database
            .lock()
            .map_err(lock_error)?
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS ashare_instrument_master_snapshots (
                    user_id TEXT NOT NULL,
                    snapshot_id TEXT NOT NULL,
                    retrieved_at_ms INTEGER NOT NULL,
                    snapshot_json TEXT NOT NULL,
                    PRIMARY KEY(user_id, snapshot_id)
                 );
                 CREATE TABLE IF NOT EXISTS ashare_calendar_snapshots (
                    user_id TEXT NOT NULL,
                    snapshot_id TEXT NOT NULL,
                    retrieved_at_ms INTEGER NOT NULL,
                    snapshot_json TEXT NOT NULL,
                    PRIMARY KEY(user_id, snapshot_id)
                 );
                 CREATE TABLE IF NOT EXISTS ashare_corporate_actions (
                    user_id TEXT NOT NULL,
                    dataset_id TEXT NOT NULL,
                    logical_key TEXT NOT NULL,
                    revision INTEGER NOT NULL,
                    dataset_json TEXT NOT NULL,
                    PRIMARY KEY(user_id, dataset_id)
                 );
                 CREATE TABLE IF NOT EXISTS ashare_backfill_checkpoints (
                    user_id TEXT NOT NULL,
                    task_id TEXT NOT NULL,
                    checkpoint_json TEXT NOT NULL,
                    PRIMARY KEY(user_id, task_id)
                 );
                 CREATE UNIQUE INDEX IF NOT EXISTS ashare_corporate_actions_revision
                 ON ashare_corporate_actions(user_id, logical_key, revision);",
            )
            .map_err(storage)
    }

    fn read_checkpoint(
        &self,
        user_id: &str,
        task_id: &str,
    ) -> Result<Option<AshareBackfillCheckpoint>, PipelineError> {
        let database = self.pipeline.database();
        let json = database
            .lock()
            .map_err(lock_error)?
            .query_row(
                "SELECT checkpoint_json FROM ashare_backfill_checkpoints
                 WHERE user_id = ?1 AND task_id = ?2",
                params![user_id, task_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(storage)?;
        json.map(|value| serde_json::from_str(&value).map_err(storage))
            .transpose()
    }

    fn write_checkpoint(&self, checkpoint: &AshareBackfillCheckpoint) -> Result<(), PipelineError> {
        let json = serde_json::to_string(checkpoint).map_err(storage)?;
        let database = self.pipeline.database();
        database
            .lock()
            .map_err(lock_error)?
            .execute(
                "INSERT OR REPLACE INTO ashare_backfill_checkpoints
                 (user_id, task_id, checkpoint_json) VALUES (?1, ?2, ?3)",
                params![checkpoint.user_id, checkpoint.task_id, json],
            )
            .map_err(storage)?;
        Ok(())
    }

    fn checkpoint_acquisition_path(&self, user_id: &str, task_id: &str) -> PathBuf {
        let key = digest(format!("{user_id}:{task_id}").as_bytes());
        self.pipeline
            .root_dir()
            .join("a-share/checkpoints")
            .join(format!("{key}.acquisition.json"))
    }

    fn load_checkpoint_acquisition(
        &self,
        checkpoint: &AshareBackfillCheckpoint,
    ) -> Result<Option<AshareBarsAcquisition>, PipelineError> {
        let Some(path) = checkpoint.acquisition_path.as_ref() else {
            return Ok(None);
        };
        if checkpoint.user_id != checkpoint.request.user_id
            || checkpoint.task_id != checkpoint.request.task_id
            || path != &self.checkpoint_acquisition_path(&checkpoint.user_id, &checkpoint.task_id)
        {
            return Err(PipelineError::Storage(
                "A-share checkpoint acquisition identity or path is invalid".into(),
            ));
        }
        if !path.is_file() {
            return Ok(None);
        }
        let Some(expected_sha256) = checkpoint.acquisition_sha256.as_deref() else {
            return Ok(None);
        };
        let bytes = fs::read(path).map_err(storage)?;
        if digest(&bytes) != expected_sha256 {
            return Err(PipelineError::Storage(
                "A-share checkpoint acquisition evidence hash does not match".into(),
            ));
        }
        serde_json::from_slice(&bytes).map(Some).map_err(storage)
    }

    fn persist_checkpoint_acquisition(
        &self,
        request: &AshareBackfillRequest,
        acquisition: &AshareBarsAcquisition,
    ) -> Result<PathBuf, PipelineError> {
        let path = self.checkpoint_acquisition_path(&request.user_id, &request.task_id);
        let bytes = canonical_json_bytes(acquisition)?;
        super::atomic_write(&path, &bytes)?;
        self.write_checkpoint(&AshareBackfillCheckpoint {
            task_id: request.task_id.clone(),
            user_id: request.user_id.clone(),
            request: request.clone(),
            state: AshareBackfillState::Running,
            source_id: None,
            canonical_id: None,
            revision: None,
            completed_through_ms: Some(request.end_time_ms),
            acquisition_path: Some(path.clone()),
            acquisition_sha256: Some(digest(&bytes)),
            failure_response_sha256: None,
            failure_response_path: None,
            last_error: None,
        })?;
        Ok(path)
    }

    fn mark_checkpoint_state(
        &self,
        request: &AshareBackfillRequest,
        state: AshareBackfillState,
        error: Option<&str>,
    ) -> Result<(), PipelineError> {
        self.mark_checkpoint_state_with_path(request, state, error, None)
    }

    fn mark_checkpoint_state_with_path(
        &self,
        request: &AshareBackfillRequest,
        state: AshareBackfillState,
        error: Option<&str>,
        acquisition_path: Option<PathBuf>,
    ) -> Result<(), PipelineError> {
        let acquisition_sha256 = match acquisition_path.as_ref() {
            Some(path) if path.is_file() => Some(digest(&fs::read(path).map_err(storage)?)),
            _ => None,
        };
        self.write_checkpoint(&AshareBackfillCheckpoint {
            task_id: request.task_id.clone(),
            user_id: request.user_id.clone(),
            request: request.clone(),
            state,
            source_id: None,
            canonical_id: None,
            revision: None,
            completed_through_ms: None,
            acquisition_path,
            acquisition_sha256,
            failure_response_sha256: None,
            failure_response_path: None,
            last_error: error.map(str::to_owned),
        })
    }

    fn mark_connector_failure(
        &self,
        request: &AshareBackfillRequest,
        state: AshareBackfillState,
        error: &str,
        response_sha256: Option<String>,
    ) -> Result<(), PipelineError> {
        let response_sha256 = response_sha256.filter(|hash| self.raw_response_path(hash).is_file());
        let response_path = response_sha256
            .as_deref()
            .map(|hash| self.raw_response_path(hash));
        self.write_checkpoint(&AshareBackfillCheckpoint {
            task_id: request.task_id.clone(),
            user_id: request.user_id.clone(),
            request: request.clone(),
            state,
            source_id: None,
            canonical_id: None,
            revision: None,
            completed_through_ms: None,
            acquisition_path: None,
            acquisition_sha256: None,
            failure_response_sha256: response_sha256,
            failure_response_path: response_path,
            last_error: Some(error.to_owned()),
        })
    }

    fn record_backfill_failure(
        &self,
        request: &AshareBackfillRequest,
        error: &PipelineError,
        cancellation: &CancellationToken,
        acquisition_path: Option<PathBuf>,
    ) -> Result<AshareBackfillState, PipelineError> {
        let state =
            if cancellation.is_cancelled() || matches!(error, PipelineError::Cancelled { .. }) {
                AshareBackfillState::Cancelled
            } else {
                AshareBackfillState::Failed
            };
        self.mark_checkpoint_state_with_path(
            request,
            state.clone(),
            Some(&error.to_string()),
            acquisition_path,
        )?;
        Ok(state)
    }

    fn require_recorded_calendar(
        &self,
        user_id: &str,
        calendar: &TradingCalendarSnapshot,
    ) -> Result<(), PipelineError> {
        let database = self.pipeline.database();
        let json = database
            .lock()
            .map_err(lock_error)?
            .query_row(
                "SELECT snapshot_json FROM ashare_calendar_snapshots
                 WHERE user_id = ?1
                   AND json_extract(snapshot_json, '$.snapshot.snapshotId') = ?2
                 LIMIT 1",
                params![user_id, calendar.snapshot_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(storage)?
            .ok_or_else(|| {
                PipelineError::NotFound(
                    "A-share backfill requires a user-owned recorded calendar snapshot".into(),
                )
            })?;
        let recorded: AshareCalendarSnapshot = serde_json::from_str(&json).map_err(storage)?;
        if recorded.snapshot != *calendar {
            return Err(PipelineError::InvalidRequest(
                "A-share backfill calendar does not match retained calendar evidence".into(),
            ));
        }
        if !recorded.evidence_path.is_file() {
            return Err(PipelineError::Storage(
                "A-share retained calendar evidence file is missing".into(),
            ));
        }
        let evidence_bytes = fs::read(&recorded.evidence_path).map_err(storage)?;
        if evidence_bytes != canonical_json_bytes(&recorded)? {
            return Err(PipelineError::Storage(
                "A-share retained calendar evidence hash does not match its catalog".into(),
            ));
        }
        let raw_path = self
            .pipeline
            .root_dir()
            .join("a-share/raw")
            .join(format!("{}.bin", recorded.response_sha256));
        if !raw_path.is_file()
            || digest(&fs::read(&raw_path).map_err(storage)?) != recorded.response_sha256
        {
            return Err(PipelineError::Storage(
                "A-share retained calendar raw evidence is missing or corrupt".into(),
            ));
        }
        Ok(())
    }
}

fn validate_master(acquisition: &AshareInstrumentMasterAcquisition) -> Result<(), PipelineError> {
    if acquisition.provider.trim().is_empty()
        || acquisition.actual_upstream.trim().is_empty()
        || acquisition.method.trim().is_empty()
        || acquisition.connector_version.trim().is_empty()
        || acquisition.retrieved_at_ms < 0
        || acquisition.parsed_response_sha256.trim().is_empty()
    {
        return Err(PipelineError::InvalidRequest(
            "A-share Instrument Master provenance is incomplete".into(),
        ));
    }
    let mut ids = HashMap::new();
    for instrument in &acquisition.instruments {
        if instrument.instrument.venue.kind != VenueKind::ChinaAShareEquity
            || instrument.mapping.instrument != instrument.instrument
            || instrument.mapping.provider_symbol != instrument.provider_symbol
        {
            return Err(PipelineError::InvalidRequest(
                "A-share Instrument Master identity mapping is invalid".into(),
            ));
        }
        let (venue, code) =
            normalize_provider_instrument(&instrument.provider_symbol, &instrument.instrument.code)
                .map_err(|error| PipelineError::InvalidRequest(error.to_string()))?;
        if venue != instrument.instrument.venue || code != instrument.instrument.code {
            return Err(PipelineError::InvalidRequest(
                "A-share Instrument Master provider mapping conflicts with Instrument".into(),
            ));
        }
        if let Some(previous) = ids.insert(
            instrument.instrument.clone(),
            instrument.provider_symbol.clone(),
        ) && previous != instrument.provider_symbol
        {
            return Err(PipelineError::InvalidRequest(
                "A-share Instrument Master contains an ambiguous mapping".into(),
            ));
        }
    }
    Ok(())
}

fn validate_calendar(acquisition: &AshareCalendarAcquisition) -> Result<(), PipelineError> {
    if acquisition.provider.trim().is_empty()
        || acquisition.actual_upstream.trim().is_empty()
        || acquisition.method.trim().is_empty()
        || acquisition.connector_version.trim().is_empty()
        || acquisition.retrieved_at_ms < 0
        || acquisition.response_sha256.trim().is_empty()
        || acquisition.content_sha256.trim().is_empty()
        || acquisition.snapshots.is_empty()
    {
        return Err(PipelineError::InvalidRequest(
            "A-share calendar provenance or retained range is incomplete".into(),
        ));
    }
    let start = acquisition
        .request_parameters
        .get("startTimeMs")
        .and_then(Value::as_i64)
        .ok_or_else(|| PipelineError::InvalidRequest("A-share calendar start is missing".into()))?;
    let end = acquisition
        .request_parameters
        .get("endTimeMs")
        .and_then(Value::as_i64)
        .ok_or_else(|| PipelineError::InvalidRequest("A-share calendar end is missing".into()))?;
    if start >= end || end.saturating_sub(start) > 20 * 366 * 86_400_000 {
        return Err(PipelineError::InvalidRequest(
            "A-share calendar range is invalid or exceeds the bounded acquisition window".into(),
        ));
    }
    let mut venues = std::collections::BTreeSet::new();
    for snapshot in &acquisition.snapshots {
        if snapshot.venue.kind != VenueKind::ChinaAShareEquity
            || snapshot.venue.time_zone != "Asia/Shanghai"
            || snapshot.effective_from_ms != start
            || snapshot.effective_to_ms != end
            || !is_a_share_session_contract(&snapshot.default_sessions)
            || !venues.insert(snapshot.venue.id.clone())
        {
            return Err(PipelineError::InvalidRequest(
                "A-share calendar venue or effective range is invalid".into(),
            ));
        }
    }
    if venues.len() != 2 || !venues.contains("sse") || !venues.contains("szse") {
        return Err(PipelineError::InvalidRequest(
            "A-share calendar must retain both SSE and SZSE snapshots".into(),
        ));
    }
    Ok(())
}

fn is_a_share_session_contract(sessions: &[adaq_data_core::market::TradingSession]) -> bool {
    sessions
        == [
            adaq_data_core::market::TradingSession {
                phase: SessionPhase::Auction,
                start_local: NaiveTime::from_hms_opt(9, 15, 0).expect("valid session"),
                end_local: NaiveTime::from_hms_opt(9, 25, 0).expect("valid session"),
            },
            adaq_data_core::market::TradingSession {
                phase: SessionPhase::PreOpen,
                start_local: NaiveTime::from_hms_opt(9, 25, 0).expect("valid session"),
                end_local: NaiveTime::from_hms_opt(9, 30, 0).expect("valid session"),
            },
            adaq_data_core::market::TradingSession {
                phase: SessionPhase::Continuous,
                start_local: NaiveTime::from_hms_opt(9, 30, 0).expect("valid session"),
                end_local: NaiveTime::from_hms_opt(11, 30, 0).expect("valid session"),
            },
            adaq_data_core::market::TradingSession {
                phase: SessionPhase::Break,
                start_local: NaiveTime::from_hms_opt(11, 30, 0).expect("valid session"),
                end_local: NaiveTime::from_hms_opt(13, 0, 0).expect("valid session"),
            },
            adaq_data_core::market::TradingSession {
                phase: SessionPhase::Continuous,
                start_local: NaiveTime::from_hms_opt(13, 0, 0).expect("valid session"),
                end_local: NaiveTime::from_hms_opt(15, 0, 0).expect("valid session"),
            },
        ]
}

fn validate_backfill_request(request: &AshareBackfillRequest) -> Result<(), PipelineError> {
    validate_user(&request.user_id)?;
    if request.task_id.trim().is_empty()
        || request.start_time_ms >= request.end_time_ms
        || request.instrument.venue.kind != VenueKind::ChinaAShareEquity
        || request.calendar.venue != request.instrument.venue
        || request.calendar.venue.time_zone != "Asia/Shanghai"
    {
        return Err(PipelineError::InvalidRequest(
            "A-share backfill request is invalid".into(),
        ));
    }
    if request.end_time_ms.saturating_sub(request.start_time_ms) > 20 * 366 * 86_400_000 {
        return Err(PipelineError::InvalidRequest(
            "A-share backfill range exceeds the bounded acquisition window".into(),
        ));
    }
    if request.calendar.effective_from_ms > request.start_time_ms
        || request.calendar.effective_to_ms < request.end_time_ms
    {
        return Err(PipelineError::InvalidRequest(
            "A-share calendar evidence does not cover the requested range".into(),
        ));
    }
    Ok(())
}

fn active_key(user_id: &str, task_id: &str) -> String {
    format!(
        "ashare-backfill:{}:{user_id}{}:{task_id}",
        user_id.len(),
        task_id.len()
    )
}

fn acquisition_key(user_id: &str, operation_id: &str) -> String {
    format!(
        "ashare-acquisition:{}:{user_id}{}:{operation_id}",
        user_id.len(),
        operation_id.len()
    )
}

fn validate_operation_id(operation_id: &str) -> Result<(), PipelineError> {
    if operation_id.trim().is_empty() {
        return Err(PipelineError::InvalidRequest(
            "A-share acquisition operation ID must be non-empty".into(),
        ));
    }
    Ok(())
}

fn validate_corporate_actions(
    acquisition: &AshareCorporateActionAcquisition,
) -> Result<(), PipelineError> {
    if acquisition.provider.trim().is_empty()
        || acquisition.actual_upstream.trim().is_empty()
        || acquisition.method.trim().is_empty()
        || acquisition.connector_version.trim().is_empty()
        || acquisition.retrieved_at_ms < 0
    {
        return Err(PipelineError::InvalidRequest(
            "A-share corporate-action provenance is incomplete".into(),
        ));
    }
    if acquisition.instrument.venue.kind != VenueKind::ChinaAShareEquity {
        return Err(PipelineError::InvalidRequest(
            "A-share corporate-action Instrument is not an A-share equity".into(),
        ));
    }
    if acquisition
        .request_parameters
        .get("symbol")
        .and_then(Value::as_str)
        .is_some_and(|symbol| symbol.trim() != acquisition.instrument.code)
    {
        return Err(PipelineError::InvalidRequest(
            "A-share corporate-action request symbol does not match the acquisition Instrument"
                .into(),
        ));
    }
    let mut instrument = None;
    for record in acquisition
        .records
        .iter()
        .chain(acquisition.invalid_records.iter())
    {
        if record.instrument != acquisition.instrument {
            return Err(PipelineError::InvalidRequest(
                "A-share corporate-action record Instrument does not match the acquisition".into(),
            ));
        }
        let (venue, code) =
            normalize_provider_instrument(&record.provider_symbol, &record.instrument.code)
                .map_err(|error| PipelineError::InvalidRequest(error.to_string()))?;
        if venue != record.instrument.venue || code != record.instrument.code {
            return Err(PipelineError::InvalidRequest(
                "A-share corporate-action provider mapping conflicts with Instrument".into(),
            ));
        }
        if instrument.get_or_insert_with(|| record.instrument.clone()) != &record.instrument {
            return Err(PipelineError::InvalidRequest(
                "A-share corporate-action acquisition mixes Instruments".into(),
            ));
        }
    }
    Ok(())
}

fn corporate_action_record_is_valid(
    record: &adaq_data_core::a_share::AshareCorporateAction,
) -> bool {
    if record.available_at_ms < 0
        || record.effective_at_ms.is_some_and(|value| value < 0)
        || record.announced_at_ms.is_some_and(|value| value < 0)
    {
        return false;
    }
    if !raw_action_date_is_valid(&record.raw_payload, "NOTICE_DATE")
        || !raw_action_date_is_valid(&record.raw_payload, "PLAN_NOTICE_DATE")
        || !raw_action_date_is_valid(&record.raw_payload, "EX_DIVIDEND_DATE")
        || !raw_action_date_is_valid(&record.raw_payload, "EQUITY_RECORD_DATE")
    {
        return false;
    }
    if record
        .raw_payload
        .get("SECURITY_CODE")
        .filter(|value| !value.is_null())
        .and_then(Value::as_str)
        .is_some_and(|value| value.trim() != record.instrument.code)
    {
        return false;
    }
    if record
        .cash_per_share
        .as_deref()
        .is_some_and(|value| Decimal::from_str(value).is_err())
        || record
            .shares_per_share
            .as_deref()
            .is_some_and(|value| Decimal::from_str(value).is_err())
    {
        return false;
    }
    match record.kind {
        adaq_data_core::a_share::AshareCorporateActionKind::CashDividend => {
            record.cash_per_share.is_some()
        }
        adaq_data_core::a_share::AshareCorporateActionKind::ShareDistribution => {
            record.shares_per_share.is_some()
        }
        adaq_data_core::a_share::AshareCorporateActionKind::CashAndShareDistribution => {
            record.cash_per_share.is_some() && record.shares_per_share.is_some()
        }
        adaq_data_core::a_share::AshareCorporateActionKind::Unknown => false,
    }
}

fn raw_action_date_is_valid(payload: &Value, field: &str) -> bool {
    payload
        .get(field)
        .filter(|value| !value.is_null())
        .is_none_or(|value| {
            value
                .as_str()
                .is_some_and(|value| parse_provider_date(value.trim()))
        })
}

fn parse_provider_date(value: &str) -> bool {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").is_ok()
        || NaiveDate::parse_from_str(value, "%Y%m%d").is_ok()
}

fn source_acquisition(
    acquisition: &AshareBarsAcquisition,
) -> Result<SourceAcquisition, PipelineError> {
    let content_sha256 = digest(
        &serde_json::to_vec(&(&acquisition.bars, &acquisition.invalid_bars)).map_err(storage)?,
    );
    if acquisition.content_sha256 != content_sha256 {
        return Err(PipelineError::InvalidRequest(
            "A-share Bars content hash does not match retained rows".into(),
        ));
    }
    if acquisition
        .bars
        .iter()
        .chain(acquisition.invalid_bars.iter())
        .any(|bar| bar.price_basis != PriceBasis::Unadjusted)
    {
        return Err(PipelineError::InvalidRequest(
            "A-share Source Bars must use Unadjusted Price Basis".into(),
        ));
    }
    Ok(SourceAcquisition {
        provider: acquisition.provider.clone(),
        actual_upstream: Some(acquisition.actual_upstream.clone()),
        connector: "adaq-ashare".into(),
        connector_version: acquisition.connector_version.clone(),
        request_parameters: acquisition.request_parameters.clone(),
        retrieved_at_ms: acquisition.retrieved_at_ms,
        response_sha256s: acquisition.response_sha256s.clone(),
        acquisition_content_sha256: Some(acquisition.content_sha256.clone()),
        capability_snapshot: capability_snapshot(
            &acquisition.provider,
            &[],
            &acquisition.limitations,
            acquisition.retrieved_at_ms,
            &["unadjusted-bars"],
        ),
        acquisition_diagnostics: AcquisitionDiagnostics {
            request_count: acquisition.diagnostics.request_count,
            retry_count: acquisition.diagnostics.retry_count,
            response_statuses: acquisition.diagnostics.response_statuses.clone(),
            notes: acquisition.diagnostics.notes.clone(),
        },
        price_basis: PriceBasis::Unadjusted,
        records: acquisition
            .bars
            .iter()
            .chain(acquisition.invalid_bars.iter())
            .map(source_record)
            .collect(),
    })
}

fn source_record(bar: &AshareBar) -> SourceMarketRecord {
    SourceMarketRecord {
        provider_symbol: bar.provider_symbol.clone(),
        instrument: bar.instrument.clone(),
        interval: bar.interval,
        open_time_ms: bar.open_time_ms,
        open: bar.open.clone(),
        high: bar.high.clone(),
        low: bar.low.clone(),
        close: bar.close.clone(),
        base_volume: bar.base_volume.clone(),
        quote_volume: bar.quote_volume.clone(),
        raw_payload: bar.raw_payload.clone(),
    }
}

fn collect_raw_response_hashes(value: &Value, hashes: &mut HashSet<String>) {
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                if matches!(
                    key.as_str(),
                    "responseSha256" | "parsedResponseSha256" | "failureResponseSha256"
                ) {
                    if let Some(hash) = value.as_str().filter(|hash| !hash.trim().is_empty()) {
                        hashes.insert(hash.to_owned());
                    }
                } else if key == "responseSha256s"
                    && let Value::Array(values) = value
                {
                    hashes.extend(
                        values
                            .iter()
                            .filter_map(Value::as_str)
                            .filter(|hash| !hash.trim().is_empty())
                            .map(str::to_owned),
                    );
                }
                collect_raw_response_hashes(value, hashes);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_raw_response_hashes(value, hashes);
            }
        }
        _ => {}
    }
}

fn capability_snapshot(
    provider: &str,
    instruments: &[AshareInstrument],
    limitations: &[String],
    captured_at_ms: i64,
    record_types: &[&str],
) -> ProviderCapabilitySnapshot {
    ProviderCapabilitySnapshot {
        provider: provider.into(),
        captured_at_ms,
        venues: if instruments.is_empty() {
            vec!["sse".into(), "szse".into()]
        } else {
            instruments
                .iter()
                .map(|value| value.instrument.venue.id.clone())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect()
        },
        record_types: record_types.iter().map(|value| (*value).into()).collect(),
        history_start_ms: None,
        delayed: false,
        delayed_known: false,
        delay_ms: None,
        rate_limit: None,
        rate_limit_known: false,
        streaming_symbol_limit: None,
        limitations: limitations.to_vec(),
    }
}

fn connector_error(error: adaq_data_core::DataError) -> PipelineError {
    PipelineError::Connector {
        code: error.code,
        message: error.message,
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|value| i64::try_from(value.as_millis()).ok())
        .unwrap_or(0)
}

fn verify_json_evidence<T: Serialize>(
    path: &PathBuf,
    value: &T,
    evidence_kind: &str,
) -> Result<(), PipelineError> {
    let bytes = fs::read(path).map_err(storage)?;
    if bytes != canonical_json_bytes(value)? {
        return Err(PipelineError::Storage(format!(
            "{evidence_kind} evidence hash does not match its catalog"
        )));
    }
    Ok(())
}

fn storage(error: impl std::fmt::Display) -> PipelineError {
    PipelineError::Storage(error.to_string())
}

fn lock_error(error: impl std::fmt::Display) -> PipelineError {
    PipelineError::Storage(format!("A-share pipeline database lock failed: {error}"))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        io::{Read, Write},
        net::TcpListener,
        sync::{Arc, Mutex, mpsc},
        thread::{self, JoinHandle},
        time::Duration,
    };

    use adaq_data_core::a_share::{
        AshareBar, AshareBarsAcquisition, AshareCalendarAcquisition, AshareCorporateAction,
        AshareCorporateActionAcquisition, AshareInstrument, AshareInstrumentMasterAcquisition,
        AshareRequestDiagnostics,
    };
    use adaq_data_core::market::{
        DayEvidence, InstrumentId, LocalTimeDisambiguation, PriceBasis, SessionPhase,
        TradingCalendarSnapshot, TradingSession, Venue,
    };
    use adaq_data_core::{BarInterval, HistoricalBarRange};
    use rusqlite::Connection;
    use tempfile::tempdir;

    use super::{
        AshareBackfillCheckpoint, AshareBackfillEvent, AshareBackfillRequest, AshareBackfillState,
        AshareDataPath, source_acquisition,
    };
    use crate::{
        CalendarEvidence, CancellationToken, CanonicalizationRequest, DataPipeline,
        DataQualityState, PipelineError, SourceAcquisition,
    };
    use adaq_data_core::a_share::ASHARE_CONNECTOR_VERSION;

    fn instrument() -> InstrumentId {
        InstrumentId::new(Venue::china_a_share("sse").unwrap(), "600000").unwrap()
    }

    fn raw_evidence(label: &str) -> (String, Vec<u8>) {
        let bytes = label.as_bytes().to_vec();
        (super::digest(&bytes), bytes)
    }

    fn calendar() -> TradingCalendarSnapshot {
        let venue = Venue::china_a_share("sse").unwrap();
        let date = adaq_data_core::market::TradingDate::new(2024, 1, 2).unwrap();
        let start = venue
            .resolve_local_time(
                date.to_naive_date().unwrap().and_hms_opt(0, 0, 0).unwrap(),
                LocalTimeDisambiguation::Reject,
            )
            .unwrap();
        let end = venue
            .resolve_local_time(
                date.to_naive_date()
                    .unwrap()
                    .succ_opt()
                    .unwrap()
                    .and_hms_opt(0, 0, 0)
                    .unwrap(),
                LocalTimeDisambiguation::Reject,
            )
            .unwrap();
        TradingCalendarSnapshot::new(
            "fixture-ashare-calendar",
            venue,
            start,
            end,
            vec![
                TradingSession {
                    phase: SessionPhase::Auction,
                    start_local: chrono::NaiveTime::from_hms_opt(9, 15, 0).unwrap(),
                    end_local: chrono::NaiveTime::from_hms_opt(9, 25, 0).unwrap(),
                },
                TradingSession {
                    phase: SessionPhase::PreOpen,
                    start_local: chrono::NaiveTime::from_hms_opt(9, 25, 0).unwrap(),
                    end_local: chrono::NaiveTime::from_hms_opt(9, 30, 0).unwrap(),
                },
                TradingSession {
                    phase: SessionPhase::Continuous,
                    start_local: chrono::NaiveTime::from_hms_opt(9, 30, 0).unwrap(),
                    end_local: chrono::NaiveTime::from_hms_opt(11, 30, 0).unwrap(),
                },
                TradingSession {
                    phase: SessionPhase::Break,
                    start_local: chrono::NaiveTime::from_hms_opt(11, 30, 0).unwrap(),
                    end_local: chrono::NaiveTime::from_hms_opt(13, 0, 0).unwrap(),
                },
                TradingSession {
                    phase: SessionPhase::Continuous,
                    start_local: chrono::NaiveTime::from_hms_opt(13, 0, 0).unwrap(),
                    end_local: chrono::NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
                },
            ],
            vec![DayEvidence::trading_day(date)],
        )
        .unwrap()
    }

    fn record_test_calendar(
        path: &AshareDataPath,
        user_id: &str,
        snapshot: &TradingCalendarSnapshot,
    ) {
        record_test_calendar_with_limitations(path, user_id, snapshot, Vec::new()).unwrap();
    }

    fn record_test_calendar_with_limitations(
        path: &AshareDataPath,
        user_id: &str,
        snapshot: &TradingCalendarSnapshot,
        limitations: Vec<String>,
    ) -> Result<Vec<super::AshareCalendarSnapshot>, PipelineError> {
        let szse = TradingCalendarSnapshot::new(
            format!("{}-szse", snapshot.snapshot_id),
            Venue::china_a_share("szse").unwrap(),
            snapshot.effective_from_ms,
            snapshot.effective_to_ms,
            snapshot.default_sessions.clone(),
            snapshot.days.clone(),
        )
        .unwrap();
        let raw = format!("calendar:{}", snapshot.snapshot_id).into_bytes();
        let content_sha256 =
            super::digest(&serde_json::to_vec(&vec![snapshot.clone(), szse.clone()]).unwrap());
        path.record_calendar(
            user_id,
            AshareCalendarAcquisition {
                provider: "akshare-rs".into(),
                actual_upstream: "Sina Finance".into(),
                method: "fixture klc_td_sh".into(),
                connector_version: ASHARE_CONNECTOR_VERSION.into(),
                request_parameters: serde_json::json!({
                    "startTimeMs": snapshot.effective_from_ms,
                    "endTimeMs": snapshot.effective_to_ms,
                }),
                retrieved_at_ms: snapshot.effective_from_ms,
                response_sha256: super::digest(&raw),
                content_sha256,
                raw_response: Some(raw),
                diagnostics: AshareRequestDiagnostics::default(),
                snapshots: vec![snapshot.clone(), szse],
                limitations,
            },
        )
    }

    #[test]
    fn scoped_activity_keys_do_not_collide_on_delimiters() {
        assert_ne!(
            super::active_key("alice:desk", "task"),
            super::active_key("alice", "desk:task")
        );
        assert_ne!(
            super::acquisition_key("alice:desk", "op"),
            super::acquisition_key("alice", "desk:op")
        );
    }

    #[test]
    fn user_reset_cancels_and_waits_for_scoped_acquisition_workers() {
        let root = tempdir().unwrap();
        let pipeline = DataPipeline::open(
            root.path(),
            Arc::new(Mutex::new(Connection::open_in_memory().unwrap())),
        )
        .unwrap();
        let path =
            AshareDataPath::open(pipeline, adaq_data_core::a_share::AshareClient::new()).unwrap();
        path.begin_acquisition("alice", "reset-test").unwrap();
        let finishing_path = path.clone();
        let (finished, received) = mpsc::channel();
        let worker = thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            finishing_path
                .finish_acquisition("alice", "reset-test")
                .unwrap();
            finished.send(()).unwrap();
        });
        path.cancel_user_operations("alice").unwrap();
        assert!(received.recv_timeout(Duration::from_millis(5)).is_ok());
        worker.join().unwrap();
    }

    #[test]
    fn shared_calendar_evidence_is_not_selected_for_one_user_reset() {
        let root = tempdir().unwrap();
        let pipeline = DataPipeline::open(
            root.path(),
            Arc::new(Mutex::new(Connection::open_in_memory().unwrap())),
        )
        .unwrap();
        let path =
            AshareDataPath::open(pipeline, adaq_data_core::a_share::AshareClient::new()).unwrap();
        let snapshot = calendar();
        record_test_calendar(&path, "alice", &snapshot);
        assert!(matches!(
            record_test_calendar_with_limitations(
                &path,
                "alice",
                &snapshot,
                vec!["changed acquisition".into()]
            ),
            Err(PipelineError::InvalidRequest(_))
        ));
        record_test_calendar(&path, "bob", &snapshot);
        assert!(path.reset_paths_for_user("alice").unwrap().is_empty());
    }

    #[test]
    fn compact_provider_action_dates_are_validated_without_quarantine() {
        assert!(super::parse_provider_date("20240102"));
        assert!(super::parse_provider_date("2024-01-02"));
        assert!(!super::parse_provider_date("2024/01/02"));
    }

    #[test]
    fn empty_corporate_action_evidence_keeps_instrument_identity() {
        let root = tempdir().unwrap();
        let pipeline = DataPipeline::open(
            root.path(),
            Arc::new(Mutex::new(Connection::open_in_memory().unwrap())),
        )
        .unwrap();
        let path =
            AshareDataPath::open(pipeline, adaq_data_core::a_share::AshareClient::new()).unwrap();
        let empty_records: Vec<AshareCorporateAction> = Vec::new();
        let content_sha256 =
            super::digest(&serde_json::to_vec(&(&empty_records, &empty_records)).unwrap());
        let (response_sha256, raw_response) = raw_evidence("empty-actions-sse");
        let sse = path
            .record_corporate_actions(
                "alice",
                AshareCorporateActionAcquisition {
                    instrument: instrument(),
                    provider: "akshare-rs".into(),
                    actual_upstream: "Eastmoney".into(),
                    method: "fixture empty actions".into(),
                    connector_version: ASHARE_CONNECTOR_VERSION.into(),
                    request_parameters: serde_json::json!({"symbol":"600000"}),
                    retrieved_at_ms: 1,
                    response_sha256,
                    content_sha256,
                    raw_response: Some(raw_response),
                    diagnostics: AshareRequestDiagnostics::default(),
                    records: empty_records.clone(),
                    invalid_records: Vec::new(),
                    limitations: vec!["provider returned no rows".into()],
                },
            )
            .unwrap();
        assert_eq!(sse.instrument, instrument());
        assert_eq!(sse.quality, DataQualityState::Rejected);
    }

    #[test]
    fn publishes_unadjusted_source_canonical_quality_and_separate_actions() {
        let root = tempdir().unwrap();
        let pipeline = DataPipeline::open(
            root.path(),
            Arc::new(Mutex::new(Connection::open_in_memory().unwrap())),
        )
        .unwrap();
        let path = AshareDataPath::open(
            pipeline.clone(),
            adaq_data_core::a_share::AshareClient::new(),
        )
        .unwrap();
        let calendar_snapshot = calendar();
        let mut fixture_bars: Vec<AshareBar> =
            serde_json::from_str(include_str!("../../../fixtures/a-share/bars.json")).unwrap();
        let bar = fixture_bars.pop().unwrap();
        let open_time_ms = bar.open_time_ms;
        let (bar_response_sha256, bar_raw) = raw_evidence("bars");
        let bar_raw_for_reset = bar_raw.clone();
        let mut acquisition = AshareBarsAcquisition {
            provider: "akshare-rs".into(),
            actual_upstream: "Eastmoney".into(),
            method: "fixture stock_zh_a_daily".into(),
            connector_version: ASHARE_CONNECTOR_VERSION.into(),
            request_parameters: serde_json::json!({"adjust":""}),
            retrieved_at_ms: open_time_ms + 86_400_000,
            response_sha256s: vec![bar_response_sha256.clone()],
            content_sha256: String::new(),
            raw_responses: vec![bar_raw],
            diagnostics: AshareRequestDiagnostics::default(),
            bars: vec![bar],
            invalid_bars: Vec::new(),
            limitations: Vec::new(),
        };
        acquisition.content_sha256 = super::digest(
            &serde_json::to_vec(&(&acquisition.bars, &acquisition.invalid_bars)).unwrap(),
        );
        let mut request = CanonicalizationRequest::new(
            instrument(),
            BarInterval::OneDay,
            CalendarEvidence::Venue {
                snapshot: calendar_snapshot,
            },
        )
        .unwrap();
        request.historical_range = Some(HistoricalBarRange {
            start_time_ms: open_time_ms,
            end_time_ms: open_time_ms + 86_400_000,
        });
        let publication = pipeline
            .publish(
                "alice",
                source_acquisition(&acquisition).unwrap(),
                request,
                CancellationToken::new(),
                |_| {},
            )
            .unwrap();
        assert_eq!(publication.quality.state, DataQualityState::Passed);
        assert_eq!(
            publication.source.identity.response_sha256s,
            vec![bar_response_sha256.clone()]
        );
        assert_eq!(
            publication.canonical.as_ref().unwrap().price_basis,
            PriceBasis::Unadjusted
        );
        assert_eq!(
            publication.canonical.as_ref().unwrap().bars[0]
                .close
                .to_string(),
            "10.25"
        );

        let action_records: Vec<AshareCorporateAction> = serde_json::from_str(include_str!(
            "../../../fixtures/a-share/corporate-actions.json"
        ))
        .unwrap();
        let action_content_sha256 = super::digest(
            &serde_json::to_vec(&(&action_records, &Vec::<AshareCorporateAction>::new())).unwrap(),
        );
        let (action_response_sha256, action_raw_response) = raw_evidence("corporate-actions");
        let actions = path
            .record_corporate_actions(
                "alice",
                AshareCorporateActionAcquisition {
                    instrument: instrument(),
                    provider: "akshare-rs".into(),
                    actual_upstream: "Eastmoney".into(),
                    method: "fixture stock_fhps_detail_em".into(),
                    connector_version: ASHARE_CONNECTOR_VERSION.into(),
                    request_parameters: serde_json::json!({"symbol":"600000"}),
                    retrieved_at_ms: open_time_ms,
                    response_sha256: action_response_sha256,
                    content_sha256: action_content_sha256,
                    raw_response: Some(action_raw_response),
                    diagnostics: AshareRequestDiagnostics::default(),
                    records: action_records,
                    invalid_records: Vec::new(),
                    limitations: Vec::new(),
                },
            )
            .unwrap();
        assert_eq!(actions.records.len(), 1);
        assert_eq!(actions.quality, DataQualityState::Passed);
        assert!(actions.evidence_path.is_file());
        let dto = actions.gui_dto();
        let dto_json = serde_json::to_string(&dto).unwrap();
        assert!(!dto_json.contains("rawPayload"));
        assert!(!dto_json.contains("evidencePath"));
        assert!(
            path.list_instrument_master_snapshots("alice")
                .unwrap()
                .is_empty()
        );

        let master_instruments: Vec<AshareInstrument> = serde_json::from_str(include_str!(
            "../../../fixtures/a-share/instrument-master.json"
        ))
        .unwrap();
        let master_content_sha256 =
            super::digest(&serde_json::to_vec(&master_instruments).unwrap());
        let master_parsed_response_sha256 =
            super::digest(&serde_json::to_vec(&master_instruments).unwrap());
        let (master_response_sha256, master_raw_response) = raw_evidence("instrument-master");
        let master = path
            .record_instrument_master(
                "alice",
                AshareInstrumentMasterAcquisition {
                    provider: "akshare-rs".into(),
                    actual_upstream: "Sina Finance".into(),
                    method: "fixture stock_zh_a_spot".into(),
                    connector_version: ASHARE_CONNECTOR_VERSION.into(),
                    request_parameters: serde_json::json!({"node":"hs_a"}),
                    retrieved_at_ms: open_time_ms,
                    response_sha256: master_response_sha256,
                    parsed_response_sha256: master_parsed_response_sha256,
                    parsed_response: Some(serde_json::to_vec(&master_instruments).unwrap()),
                    content_sha256: master_content_sha256,
                    raw_response: Some(master_raw_response),
                    diagnostics: AshareRequestDiagnostics::default(),
                    instruments: master_instruments,
                    limitations: Vec::new(),
                },
            )
            .unwrap();
        assert_eq!(
            master.evidence_state,
            super::UniverseEvidenceState::Observed
        );
        assert_eq!(master.request_parameters["node"], "hs_a");
        assert_eq!(
            path.point_in_time_universe("alice", open_time_ms)
                .unwrap()
                .instruments
                .len(),
            1
        );
        let universe = path
            .point_in_time_membership("alice", open_time_ms)
            .unwrap();
        assert_eq!(
            universe.evidence_state,
            super::UniverseEvidenceState::Observed
        );
        assert_eq!(universe.instruments.len(), 1);
        assert_eq!(
            path.point_in_time_membership("alice", open_time_ms - 1)
                .unwrap()
                .evidence_state,
            super::UniverseEvidenceState::Unknown
        );
        assert_eq!(
            path.point_in_time_membership("alice", open_time_ms + 1)
                .unwrap()
                .evidence_state,
            super::UniverseEvidenceState::Reconstructed
        );
        path.retain_raw_response(&bar_response_sha256, &bar_raw_for_reset)
            .unwrap();
        let reset_paths = path.reset_paths_for_user("alice").unwrap();
        assert!(reset_paths.contains(&path.raw_response_path(&bar_response_sha256)));
    }

    #[test]
    fn adjusted_equity_basis_is_rejected_before_publication() {
        let root = tempdir().unwrap();
        let pipeline = DataPipeline::open(
            root.path(),
            Arc::new(Mutex::new(Connection::open_in_memory().unwrap())),
        )
        .unwrap();
        let mut request = CanonicalizationRequest::new(
            instrument(),
            BarInterval::OneDay,
            CalendarEvidence::Venue {
                snapshot: calendar(),
            },
        )
        .unwrap();
        request.price_basis = PriceBasis::ForwardAdjusted;
        let mut acquisition = SourceAcquisition::default();
        acquisition.price_basis = PriceBasis::ForwardAdjusted;
        let error = pipeline
            .publish(
                "alice",
                acquisition,
                request,
                CancellationToken::new(),
                |_| {},
            )
            .unwrap_err();
        assert!(error.to_string().contains("Unadjusted Price Basis"));
    }

    #[test]
    fn daily_a_share_gaps_use_the_local_continuous_open() {
        let root = tempdir().unwrap();
        let pipeline = DataPipeline::open(
            root.path(),
            Arc::new(Mutex::new(Connection::open_in_memory().unwrap())),
        )
        .unwrap();
        let calendar = calendar();
        let trading_date = calendar
            .trading_date_of(calendar.effective_from_ms)
            .unwrap();
        let expected_open = calendar.daily_boundary_open_ms(trading_date).unwrap();
        let mut request = CanonicalizationRequest::new(
            instrument(),
            BarInterval::OneDay,
            CalendarEvidence::Venue {
                snapshot: calendar.clone(),
            },
        )
        .unwrap();
        request.historical_range = Some(HistoricalBarRange {
            start_time_ms: calendar.effective_from_ms,
            end_time_ms: calendar.effective_to_ms,
        });
        let publication = pipeline
            .publish(
                "alice",
                SourceAcquisition::default(),
                request,
                CancellationToken::new(),
                |_| {},
            )
            .unwrap();
        assert_eq!(publication.quality.gaps.len(), 1);
        assert_eq!(publication.quality.gaps[0].start_time_ms, expected_open);
        assert_eq!(publication.quality.coverage.expected_record_count, 1);
    }

    #[test]
    fn malformed_a_share_rows_are_quarantined_and_quality_is_degraded() {
        let root = tempdir().unwrap();
        let pipeline = DataPipeline::open(
            root.path(),
            Arc::new(Mutex::new(Connection::open_in_memory().unwrap())),
        )
        .unwrap();
        let mut fixture_bars: Vec<AshareBar> =
            serde_json::from_str(include_str!("../../../fixtures/a-share/bars.json")).unwrap();
        let valid = fixture_bars.pop().unwrap();
        let mut invalid = valid.clone();
        invalid.close = None;
        let (bar_response_sha256, bar_raw) = raw_evidence("malformed-bars");
        let mut acquisition = AshareBarsAcquisition {
            provider: "akshare-rs".into(),
            actual_upstream: "Eastmoney".into(),
            method: "fixture stock_zh_a_daily".into(),
            connector_version: ASHARE_CONNECTOR_VERSION.into(),
            request_parameters: serde_json::json!({"adjust":""}),
            retrieved_at_ms: valid.open_time_ms + 86_400_000,
            response_sha256s: vec![bar_response_sha256],
            content_sha256: String::new(),
            raw_responses: vec![bar_raw],
            diagnostics: AshareRequestDiagnostics::default(),
            bars: vec![valid.clone(), invalid],
            invalid_bars: Vec::new(),
            limitations: Vec::new(),
        };
        acquisition.content_sha256 = super::digest(
            &serde_json::to_vec(&(&acquisition.bars, &acquisition.invalid_bars)).unwrap(),
        );
        let mut request = CanonicalizationRequest::new(
            instrument(),
            BarInterval::OneDay,
            CalendarEvidence::Venue {
                snapshot: calendar(),
            },
        )
        .unwrap();
        request.historical_range = Some(HistoricalBarRange {
            start_time_ms: valid.open_time_ms,
            end_time_ms: valid.open_time_ms + 86_400_000,
        });
        let publication = pipeline
            .publish(
                "alice",
                source_acquisition(&acquisition).unwrap(),
                request,
                CancellationToken::new(),
                |_| {},
            )
            .unwrap();
        assert_eq!(publication.quality.state, DataQualityState::Degraded);
        assert_eq!(publication.quality.quarantine_count, 1);
        assert!(publication.canonical.is_some());
        assert!(
            publication.quality.quarantined_records[0]
                .record
                .raw_payload
                .is_object()
        );
    }

    #[tokio::test]
    async fn stale_completed_checkpoint_is_not_reused() {
        let root = tempdir().unwrap();
        let pipeline = DataPipeline::open(
            root.path(),
            Arc::new(Mutex::new(Connection::open_in_memory().unwrap())),
        )
        .unwrap();
        let path =
            AshareDataPath::open(pipeline, adaq_data_core::a_share::AshareClient::new()).unwrap();
        let calendar_snapshot = calendar();
        record_test_calendar(&path, "alice", &calendar_snapshot);
        let start = calendar_snapshot.effective_from_ms;
        let end = calendar_snapshot.effective_to_ms;
        let request = AshareBackfillRequest {
            task_id: "task".into(),
            user_id: "alice".into(),
            instrument: instrument(),
            interval: BarInterval::OneDay,
            start_time_ms: start,
            end_time_ms: end,
            calendar: calendar_snapshot,
        };
        path.write_checkpoint(&AshareBackfillCheckpoint {
            task_id: request.task_id.clone(),
            user_id: request.user_id.clone(),
            request: request.clone(),
            state: AshareBackfillState::Completed,
            source_id: Some("source".into()),
            canonical_id: Some("canonical".into()),
            revision: Some(1),
            completed_through_ms: Some(end),
            acquisition_path: None,
            acquisition_sha256: None,
            failure_response_sha256: None,
            failure_response_path: None,
            last_error: None,
        })
        .unwrap();
        let mut events = Vec::new();
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let error = path
            .backfill(&request, cancellation, |event| events.push(event))
            .await
            .unwrap_err();
        assert!(matches!(error, PipelineError::Cancelled { .. }));
        assert!(matches!(
            events[0],
            super::AshareBackfillEvent::Cancelled { .. }
        ));
    }

    #[tokio::test]
    async fn local_mock_backfill_round_trips_source_quality_actions_and_restart() {
        let (base_url, server) = serve_mock(vec![
            (
                "getHQNodeStockCount",
                include_str!("../../../fixtures/a-share/upstream/spot-count.txt"),
                200,
            ),
            (
                "getHQNodeData",
                include_str!("../../../fixtures/a-share/upstream/spot.json"),
                200,
            ),
            (
                "getHQNodeData",
                include_str!("../../../fixtures/a-share/upstream/spot.json"),
                200,
            ),
            (
                "klc_td_sh.txt",
                include_str!("../../../fixtures/a-share/upstream/trade-dates.txt"),
                200,
            ),
            (
                "stock/kline/get",
                include_str!("../../../fixtures/a-share/upstream/daily-kline.json"),
                200,
            ),
            (
                "data/v1/get",
                include_str!("../../../fixtures/a-share/upstream/corporate-actions.json"),
                200,
            ),
        ]);
        let root = tempdir().unwrap();
        let database_path = root.path().join("pipeline.sqlite");
        let pipeline = DataPipeline::open(
            root.path(),
            Arc::new(Mutex::new(Connection::open(&database_path).unwrap())),
        )
        .unwrap();
        let client = adaq_data_core::a_share::AshareClient::with_mock(base_url);
        let path = AshareDataPath::open(pipeline, client.clone()).unwrap();
        let start_time_ms = 1_704_159_000_000;
        let end_time_ms = start_time_ms + 86_400_000;
        let range = HistoricalBarRange {
            start_time_ms,
            end_time_ms,
        };
        let master = client
            .acquire_instrument_master_at(start_time_ms)
            .await
            .unwrap();
        path.record_instrument_master("alice", master).unwrap();
        let calendar = path
            .acquire_calendar("alice", range)
            .await
            .unwrap()
            .into_iter()
            .find(|value| value.snapshot.venue.id == "sse")
            .unwrap();
        let request = AshareBackfillRequest {
            task_id: "mock-e2e".into(),
            user_id: "alice".into(),
            instrument: instrument(),
            interval: BarInterval::OneDay,
            start_time_ms,
            end_time_ms,
            calendar: calendar.snapshot.clone(),
        };
        let publication = path
            .backfill(&request, CancellationToken::new(), |_| {})
            .await
            .unwrap()
            .unwrap();
        assert!(publication.canonical.is_some());
        assert!(matches!(
            publication.quality.state,
            DataQualityState::Passed | DataQualityState::Degraded
        ));
        let actions = path
            .acquire_corporate_actions("alice", instrument())
            .await
            .unwrap();
        assert_eq!(actions.records.len(), 1);
        assert!(matches!(
            actions.quality,
            DataQualityState::Passed | DataQualityState::Degraded
        ));
        let source_id = publication.source.source_id.clone();
        server.join().unwrap();

        let reopened_pipeline = DataPipeline::open(
            root.path(),
            Arc::new(Mutex::new(Connection::open(&database_path).unwrap())),
        )
        .unwrap();
        let reopened_path = AshareDataPath::open(
            reopened_pipeline,
            adaq_data_core::a_share::AshareClient::new(),
        )
        .unwrap();
        let workspace = reopened_path
            .workspace_dto_for_user("alice", &source_id, end_time_ms)
            .unwrap();
        assert_eq!(workspace.source_id, source_id);
        assert_eq!(workspace.price_basis, PriceBasis::Unadjusted);
        assert!(
            reopened_path
                .backfill(&request, CancellationToken::new(), |_| {})
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn empty_history_is_rejected_but_remains_retryable() {
        let (base_url, server) = serve_mock(vec![
            (
                "getHQNodeStockCount",
                include_str!("../../../fixtures/a-share/upstream/spot-count.txt"),
                200,
            ),
            (
                "getHQNodeData",
                include_str!("../../../fixtures/a-share/upstream/spot.json"),
                200,
            ),
            (
                "getHQNodeData",
                include_str!("../../../fixtures/a-share/upstream/spot.json"),
                200,
            ),
            (
                "klc_td_sh.txt",
                include_str!("../../../fixtures/a-share/upstream/trade-dates.txt"),
                200,
            ),
            (
                "stock/kline/get",
                include_str!("../../../fixtures/a-share/upstream/daily-kline-empty.json"),
                200,
            ),
            (
                "stock/kline/get",
                include_str!("../../../fixtures/a-share/upstream/daily-kline-empty.json"),
                200,
            ),
        ]);
        let root = tempdir().unwrap();
        let database_path = root.path().join("pipeline.sqlite");
        let pipeline = DataPipeline::open(
            root.path(),
            Arc::new(Mutex::new(Connection::open(&database_path).unwrap())),
        )
        .unwrap();
        let client = adaq_data_core::a_share::AshareClient::with_mock(base_url);
        let path = AshareDataPath::open(pipeline, client.clone()).unwrap();
        let start_time_ms = 1_704_159_000_000;
        let end_time_ms = start_time_ms + 86_400_000;
        let range = HistoricalBarRange {
            start_time_ms,
            end_time_ms,
        };
        path.record_instrument_master(
            "alice",
            client
                .acquire_instrument_master_at(start_time_ms)
                .await
                .unwrap(),
        )
        .unwrap();
        let calendar = path
            .acquire_calendar("alice", range)
            .await
            .unwrap()
            .into_iter()
            .find(|value| value.snapshot.venue.id == "sse")
            .unwrap()
            .snapshot;
        let request = AshareBackfillRequest {
            task_id: "empty-history-retry".into(),
            user_id: "alice".into(),
            instrument: instrument(),
            interval: BarInterval::OneDay,
            start_time_ms,
            end_time_ms,
            calendar,
        };
        let first = path
            .backfill(&request, CancellationToken::new(), |_| {})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.quality.state, DataQualityState::Rejected);
        assert_eq!(
            path.read_checkpoint("alice", "empty-history-retry")
                .unwrap()
                .unwrap()
                .state,
            AshareBackfillState::Rejected
        );
        let mut events = Vec::new();
        let second = path
            .backfill(&request, CancellationToken::new(), |event| {
                events.push(event)
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(second.quality.state, DataQualityState::Rejected);
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, AshareBackfillEvent::AlreadyCompleted { .. }))
        );
        server.join().unwrap();
    }

    #[test]
    fn acquired_bars_are_persisted_for_restart_resume() {
        let root = tempdir().unwrap();
        let pipeline = DataPipeline::open(
            root.path(),
            Arc::new(Mutex::new(Connection::open_in_memory().unwrap())),
        )
        .unwrap();
        let path =
            AshareDataPath::open(pipeline, adaq_data_core::a_share::AshareClient::new()).unwrap();
        let raw = b"wire response";
        let raw_hash = super::digest(raw);
        path.retain_raw_response(&raw_hash, raw).unwrap();
        assert_eq!(
            fs::read(
                root.path()
                    .join("a-share/raw")
                    .join(format!("{raw_hash}.bin"))
            )
            .unwrap(),
            raw
        );
        let calendar = calendar();
        let request = AshareBackfillRequest {
            task_id: "resume-task".into(),
            user_id: "alice".into(),
            instrument: instrument(),
            interval: BarInterval::OneDay,
            start_time_ms: calendar.effective_from_ms,
            end_time_ms: calendar.effective_to_ms,
            calendar,
        };
        let mut bars: Vec<AshareBar> =
            serde_json::from_str(include_str!("../../../fixtures/a-share/bars.json")).unwrap();
        let bar = bars.pop().unwrap();
        let mut acquisition = AshareBarsAcquisition {
            provider: "akshare-rs".into(),
            actual_upstream: "Eastmoney".into(),
            method: "fixture stock_zh_a_daily".into(),
            connector_version: ASHARE_CONNECTOR_VERSION.into(),
            request_parameters: serde_json::json!({"adjust":""}),
            retrieved_at_ms: request.end_time_ms,
            response_sha256s: vec![raw_evidence("resume-bars").0],
            content_sha256: String::new(),
            raw_responses: vec![raw_evidence("resume-bars").1],
            diagnostics: AshareRequestDiagnostics::default(),
            bars: vec![bar],
            invalid_bars: Vec::new(),
            limitations: Vec::new(),
        };
        acquisition.content_sha256 = super::digest(
            &serde_json::to_vec(&(&acquisition.bars, &acquisition.invalid_bars)).unwrap(),
        );
        let path_name = path
            .persist_checkpoint_acquisition(&request, &acquisition)
            .unwrap();
        let checkpoint = path
            .read_checkpoint("alice", "resume-task")
            .unwrap()
            .unwrap();
        let resumed = path.load_checkpoint_acquisition(&checkpoint).unwrap();
        assert_eq!(resumed, Some(acquisition));
        assert!(path_name.is_file());
        assert_eq!(checkpoint.completed_through_ms, Some(request.end_time_ms));
    }

    #[tokio::test]
    async fn cancelled_backfill_retains_checkpoint_without_network_or_publication() {
        let root = tempdir().unwrap();
        let pipeline = DataPipeline::open(
            root.path(),
            Arc::new(Mutex::new(Connection::open_in_memory().unwrap())),
        )
        .unwrap();
        let path =
            AshareDataPath::open(pipeline, adaq_data_core::a_share::AshareClient::new()).unwrap();
        let calendar_snapshot = calendar();
        let request = AshareBackfillRequest {
            task_id: "cancelled-task".into(),
            user_id: "alice".into(),
            instrument: instrument(),
            interval: BarInterval::OneDay,
            start_time_ms: calendar_snapshot.effective_from_ms,
            end_time_ms: calendar_snapshot.effective_to_ms,
            calendar: calendar_snapshot,
        };
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let mut events = Vec::new();
        let error = path
            .backfill(&request, cancellation, |event| events.push(event))
            .await
            .unwrap_err();
        assert!(matches!(error, PipelineError::Cancelled { .. }));
        assert!(matches!(
            events[0],
            super::AshareBackfillEvent::Cancelled { .. }
        ));
        assert_eq!(
            path.read_checkpoint("alice", "cancelled-task")
                .unwrap()
                .unwrap()
                .state,
            AshareBackfillState::Cancelled
        );
    }

    #[tokio::test]
    async fn stale_running_checkpoint_can_resume_after_restart() {
        let root = tempdir().unwrap();
        let pipeline = DataPipeline::open(
            root.path(),
            Arc::new(Mutex::new(Connection::open_in_memory().unwrap())),
        )
        .unwrap();
        let path =
            AshareDataPath::open(pipeline, adaq_data_core::a_share::AshareClient::new()).unwrap();
        let calendar_snapshot = calendar();
        let request = AshareBackfillRequest {
            task_id: "restart-task".into(),
            user_id: "alice".into(),
            instrument: instrument(),
            interval: BarInterval::OneDay,
            start_time_ms: calendar_snapshot.effective_from_ms,
            end_time_ms: calendar_snapshot.effective_to_ms,
            calendar: calendar_snapshot,
        };
        path.write_checkpoint(&AshareBackfillCheckpoint {
            task_id: request.task_id.clone(),
            user_id: request.user_id.clone(),
            request: request.clone(),
            state: AshareBackfillState::Running,
            source_id: None,
            canonical_id: None,
            revision: None,
            completed_through_ms: None,
            acquisition_path: None,
            acquisition_sha256: None,
            failure_response_sha256: None,
            failure_response_path: None,
            last_error: None,
        })
        .unwrap();
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let error = path
            .backfill(&request, cancellation, |_| {})
            .await
            .unwrap_err();
        assert!(matches!(error, PipelineError::Cancelled { .. }));
        assert_eq!(
            path.read_checkpoint("alice", "restart-task")
                .unwrap()
                .unwrap()
                .state,
            AshareBackfillState::Cancelled
        );
    }

    fn serve_mock(responses: Vec<(&'static str, &'static str, u16)>) -> (String, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            for (path, body, status) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0; 8192];
                let size = stream.read(&mut request).unwrap();
                let request = String::from_utf8_lossy(&request[..size]);
                assert!(
                    request.contains(path),
                    "request {request:?} did not contain {path:?}"
                );
                write!(
                    stream,
                    "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .unwrap();
            }
        });
        (format!("http://{address}"), server)
    }
}
