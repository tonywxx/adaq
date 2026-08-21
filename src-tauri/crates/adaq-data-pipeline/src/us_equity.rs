//! Alpaca Basic U.S. equity Source -> Canonical publication and evidence.
//!
//! This module owns persistence and user scoping. Credentials never enter it;
//! the Host supplies an already-authenticated client for each operation.

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use adaq_data_core::{
    BarInterval, HistoricalBarRange, InstrumentStatus, TickerSnapshot,
    alpaca::{
        AlpacaBarsAcquisition, AlpacaCalendarAcquisition, AlpacaCapabilitySnapshot, AlpacaClient,
        AlpacaInstrument, AlpacaInstrumentMasterAcquisition, AlpacaMarketSnapshot,
    },
    market::{InstrumentId, PriceBasis, TradingCalendarSnapshot, VenueKind},
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    AcquisitionDiagnostics, CalendarEvidence, CancellationToken, CanonicalizationRequest,
    DataPipeline, DataQualityState, PipelineError, PipelinePublication, ProviderCapabilitySnapshot,
    SourceAcquisition, SourceMarketRecord, canonical_json_bytes, digest, validate_user,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsEquityUniverseEvidenceState {
    Observed,
    Reconstructed,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsEquityInstrumentMasterSnapshot {
    pub snapshot_id: String,
    pub effective_at_ms: i64,
    pub provider: String,
    pub actual_upstream: String,
    pub method: String,
    pub connector_version: String,
    pub request_parameters: Value,
    pub response_sha256: String,
    pub content_sha256: String,
    pub diagnostics: adaq_data_core::alpaca::AlpacaRequestDiagnostics,
    pub capability_snapshot: ProviderCapabilitySnapshot,
    pub evidence_state: UsEquityUniverseEvidenceState,
    pub instruments: Vec<AlpacaInstrument>,
    #[serde(default)]
    pub limitations: Vec<String>,
    pub evidence_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsEquityInstrumentMasterSnapshotDto {
    pub snapshot_id: String,
    pub effective_at_ms: i64,
    pub provider: String,
    pub actual_upstream: String,
    pub method: String,
    pub connector_version: String,
    pub request_parameters: Value,
    pub response_sha256: String,
    pub content_sha256: String,
    pub diagnostics: adaq_data_core::alpaca::AlpacaRequestDiagnostics,
    pub capability_snapshot: ProviderCapabilitySnapshot,
    pub evidence_state: UsEquityUniverseEvidenceState,
    pub instruments: Vec<AlpacaInstrument>,
    #[serde(default)]
    pub limitations: Vec<String>,
}

impl UsEquityInstrumentMasterSnapshot {
    pub fn gui_dto(&self) -> UsEquityInstrumentMasterSnapshotDto {
        UsEquityInstrumentMasterSnapshotDto {
            snapshot_id: self.snapshot_id.clone(),
            effective_at_ms: self.effective_at_ms,
            provider: self.provider.clone(),
            actual_upstream: self.actual_upstream.clone(),
            method: self.method.clone(),
            connector_version: self.connector_version.clone(),
            request_parameters: self.request_parameters.clone(),
            response_sha256: self.response_sha256.clone(),
            content_sha256: self.content_sha256.clone(),
            diagnostics: self.diagnostics.clone(),
            capability_snapshot: self.capability_snapshot.clone(),
            evidence_state: self.evidence_state,
            instruments: self.instruments.clone(),
            limitations: self.limitations.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsEquityPointInTimeUniverse {
    pub universe_id: String,
    pub as_of_ms: i64,
    pub snapshot_id: Option<String>,
    pub evidence_state: UsEquityUniverseEvidenceState,
    #[serde(default)]
    pub evidence_reasons: Vec<String>,
    pub coverage_start_ms: Option<i64>,
    pub coverage_end_ms: Option<i64>,
    pub instruments: Vec<AlpacaInstrument>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsEquityCalendarSnapshot {
    pub snapshot: TradingCalendarSnapshot,
    pub provider: String,
    pub actual_upstream: String,
    pub method: String,
    pub connector_version: String,
    pub request_parameters: Value,
    pub retrieved_at_ms: i64,
    pub response_sha256: String,
    pub content_sha256: String,
    pub diagnostics: adaq_data_core::alpaca::AlpacaRequestDiagnostics,
    pub capability_snapshot: ProviderCapabilitySnapshot,
    #[serde(default)]
    pub limitations: Vec<String>,
    pub evidence_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsEquityCalendarSnapshotDto {
    pub snapshot: TradingCalendarSnapshot,
    pub provider: String,
    pub actual_upstream: String,
    pub method: String,
    pub connector_version: String,
    pub request_parameters: Value,
    pub retrieved_at_ms: i64,
    pub response_sha256: String,
    pub content_sha256: String,
    pub diagnostics: adaq_data_core::alpaca::AlpacaRequestDiagnostics,
    pub capability_snapshot: ProviderCapabilitySnapshot,
    #[serde(default)]
    pub limitations: Vec<String>,
}

impl UsEquityCalendarSnapshot {
    pub fn gui_dto(&self) -> UsEquityCalendarSnapshotDto {
        UsEquityCalendarSnapshotDto {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsEquityBackfillState {
    Running,
    Completed,
    Degraded,
    Rejected,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsEquityBackfillRequest {
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
struct UsEquityBackfillCheckpoint {
    task_id: String,
    user_id: String,
    request: UsEquityBackfillRequest,
    state: UsEquityBackfillState,
    source_id: Option<String>,
    canonical_id: Option<String>,
    revision: Option<u64>,
    completed_through_ms: Option<i64>,
    acquisition_path: Option<PathBuf>,
    acquisition_sha256: Option<String>,
    failure_response_sha256: Option<String>,
    last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "event", content = "data")]
pub enum UsEquityBackfillEvent {
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
pub struct UsEquityMarketWorkspaceDto {
    pub instrument: InstrumentId,
    pub provider: String,
    pub actual_upstream: Option<String>,
    pub connector: String,
    pub connector_version: String,
    pub retrieved_at_ms: i64,
    pub freshness_ms: Option<i64>,
    pub feed: Option<String>,
    pub capability_snapshot: ProviderCapabilitySnapshot,
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

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsEquityMarketSnapshotDto {
    pub provider: String,
    pub instrument: InstrumentId,
    pub feed: String,
    pub retrieved_at_ms: i64,
    pub freshness_ms: Option<i64>,
    pub response_sha256: String,
    pub ticker: TickerSnapshot,
    pub trade: adaq_data_core::MarketTrade,
    pub quote: adaq_data_core::alpaca::AlpacaQuote,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsEquityAcquisitionStatus {
    pub task_id: String,
    pub user_id: String,
    pub state: UsEquityBackfillState,
    pub source_id: Option<String>,
    pub canonical_id: Option<String>,
    pub revision: Option<u64>,
    pub completed_through_ms: Option<i64>,
    pub last_error: Option<String>,
}

#[derive(Clone)]
pub struct UsEquityDataPath {
    pipeline: DataPipeline,
    active: Arc<Mutex<HashMap<String, CancellationToken>>>,
    resetting: Arc<Mutex<HashSet<String>>>,
}

struct ActivityGuard {
    pipeline: DataPipeline,
    active: Arc<Mutex<HashMap<String, CancellationToken>>>,
    key: String,
}

impl Drop for ActivityGuard {
    fn drop(&mut self) {
        if let Ok(mut active) = self.active.lock() {
            active.remove(&self.key);
        }
        let _ = self.pipeline.finish_attempt(&self.key);
    }
}

impl UsEquityDataPath {
    pub fn open(pipeline: DataPipeline) -> Result<Self, PipelineError> {
        for directory in [
            "us-equity",
            "us-equity/instrument-master",
            "us-equity/calendars",
            "us-equity/raw",
            "us-equity/checkpoints",
        ] {
            fs::create_dir_all(pipeline.root_dir().join(directory)).map_err(storage)?;
        }
        let path = Self {
            pipeline,
            active: Arc::new(Mutex::new(HashMap::new())),
            resetting: Arc::new(Mutex::new(HashSet::new())),
        };
        path.initialize_schema()?;
        Ok(path)
    }

    pub fn begin_acquisition(
        &self,
        user_id: &str,
        operation_id: &str,
    ) -> Result<CancellationToken, PipelineError> {
        validate_user(user_id)?;
        self.ensure_user_available(user_id)?;
        if operation_id.trim().is_empty() {
            return Err(PipelineError::InvalidRequest(
                "Alpaca acquisition operation ID must be non-empty".into(),
            ));
        }
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
        let key = acquisition_key(user_id, operation_id);
        self.pipeline.cancel(&key, user_id)?;
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
        let key = acquisition_key(user_id, operation_id);
        self.active.lock().map_err(lock_error)?.remove(&key);
        self.pipeline.finish_attempt(&key)
    }

    pub fn begin_backfill(
        &self,
        user_id: &str,
        task_id: &str,
    ) -> Result<CancellationToken, PipelineError> {
        validate_user(user_id)?;
        self.ensure_user_available(user_id)?;
        let key = backfill_key(user_id, task_id);
        let token = self.pipeline.begin_attempt(&key, user_id)?;
        self.active
            .lock()
            .map_err(lock_error)?
            .insert(key, token.clone());
        Ok(token)
    }

    pub fn cancel_backfill(&self, user_id: &str, task_id: &str) -> Result<(), PipelineError> {
        let key = backfill_key(user_id, task_id);
        self.pipeline.cancel(&key, user_id)?;
        if let Some(token) = self.active.lock().map_err(lock_error)?.get(&key) {
            token.cancel();
        }
        Ok(())
    }

    pub fn finish_backfill(&self, user_id: &str, task_id: &str) -> Result<(), PipelineError> {
        let key = backfill_key(user_id, task_id);
        self.active.lock().map_err(lock_error)?.remove(&key);
        self.pipeline.finish_attempt(&key)
    }

    pub fn cancel_user_operations(&self, user_id: &str) -> Result<(), PipelineError> {
        validate_user(user_id)?;
        let prefix = format!("us-equity:{}:{user_id}", user_id.len());
        let deadline = Instant::now() + Duration::from_secs(35);
        loop {
            let tokens = self
                .active
                .lock()
                .map_err(lock_error)?
                .iter()
                .filter(|(key, _)| key.starts_with(&prefix))
                .map(|(key, token)| (key.clone(), token.clone()))
                .collect::<Vec<_>>();
            if tokens.is_empty() {
                return Ok(());
            }
            for (key, token) in tokens {
                token.cancel();
                self.pipeline.cancel(&key, user_id)?;
            }
            if Instant::now() >= deadline {
                return Err(PipelineError::Storage(
                    "Timed out waiting for Alpaca operations to stop".into(),
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
                "Alpaca U.S. equity reset is already in progress for this user".into(),
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
                "Alpaca U.S. equity data is being reset for this user".into(),
            ));
        }
        Ok(())
    }

    fn connector_error(&self, error: adaq_data_core::DataError) -> PipelineError {
        let response_sha256 = error
            .response_sha256
            .clone()
            .or_else(|| error.raw_response.as_ref().map(|bytes| digest(bytes)));
        if let (Some(hash), Some(bytes)) =
            (response_sha256.as_deref(), error.raw_response.as_deref())
            && self.retain_raw_response(hash, bytes).is_err()
        {
            return PipelineError::Storage(
                "Alpaca connector error response could not be retained".into(),
            );
        }
        let mut pipeline_error = connector_error(error);
        if let (Some(hash), PipelineError::Connector { message, .. }) =
            (response_sha256, &mut pipeline_error)
        {
            *message = format!("{message}; rawResponseSha256={hash}");
        }
        pipeline_error
    }

    pub async fn acquire_instrument_master(
        &self,
        user_id: &str,
        client: &AlpacaClient,
        cancellation: &CancellationToken,
        retrieved_at_ms: i64,
    ) -> Result<UsEquityInstrumentMasterSnapshot, PipelineError> {
        validate_user(user_id)?;
        self.ensure_user_available(user_id)?;
        let acquisition = client
            .acquire_instrument_master(retrieved_at_ms)
            .await
            .map_err(|error| self.connector_error(error))?;
        if cancellation.is_cancelled() {
            return Err(PipelineError::Cancelled {
                source_id: "alpaca-instrument-master".into(),
            });
        }
        self.record_instrument_master(user_id, client, acquisition)
    }

    pub fn record_instrument_master(
        &self,
        user_id: &str,
        client: &AlpacaClient,
        acquisition: AlpacaInstrumentMasterAcquisition,
    ) -> Result<UsEquityInstrumentMasterSnapshot, PipelineError> {
        validate_user(user_id)?;
        self.ensure_user_available(user_id)?;
        if acquisition.provider != "alpaca"
            || acquisition.actual_upstream.trim().is_empty()
            || acquisition.method.trim().is_empty()
            || acquisition.connector_version.trim().is_empty()
            || acquisition.retrieved_at_ms < 0
            || acquisition.response_sha256.trim().is_empty()
            || acquisition.raw_response.is_empty()
        {
            return Err(PipelineError::InvalidRequest(
                "Alpaca Instrument Master provenance is incomplete".into(),
            ));
        }
        self.retain_raw_response(&acquisition.response_sha256, &acquisition.raw_response)?;
        let content_sha256 =
            digest(&serde_json::to_vec(&acquisition.instruments).map_err(storage)?);
        if acquisition.content_sha256 != content_sha256 {
            return Err(PipelineError::InvalidRequest(
                "Alpaca Instrument Master content hash does not match retained instruments".into(),
            ));
        }
        validate_instruments(&acquisition.instruments)?;
        let capability_snapshot = capability_snapshot(
            &client.capability_snapshot(acquisition.retrieved_at_ms),
            &acquisition.limitations,
            acquisition.retrieved_at_ms,
            &["instrument-master"],
            &acquisition.instruments,
        );
        let snapshot_id = digest(&canonical_json_bytes(&(
            &acquisition.provider,
            &acquisition.actual_upstream,
            &acquisition.method,
            &acquisition.connector_version,
            &acquisition.request_parameters,
            acquisition.retrieved_at_ms,
            &acquisition.response_sha256,
            &content_sha256,
            &capability_snapshot,
            &acquisition.instruments,
            &acquisition.limitations,
        ))?);
        let evidence_path = self
            .pipeline
            .root_dir()
            .join("us-equity/instrument-master")
            .join(format!("{snapshot_id}.json"));
        let snapshot = UsEquityInstrumentMasterSnapshot {
            snapshot_id: snapshot_id.clone(),
            effective_at_ms: acquisition.retrieved_at_ms,
            provider: acquisition.provider,
            actual_upstream: acquisition.actual_upstream,
            method: acquisition.method,
            connector_version: acquisition.connector_version,
            request_parameters: acquisition.request_parameters,
            response_sha256: acquisition.response_sha256,
            content_sha256,
            diagnostics: acquisition.diagnostics,
            capability_snapshot,
            evidence_state: UsEquityUniverseEvidenceState::Observed,
            instruments: acquisition.instruments,
            limitations: acquisition.limitations,
            evidence_path,
        };
        self.persist_snapshot(
            user_id,
            "us_equity_instrument_master_snapshots",
            &snapshot.snapshot_id,
            snapshot.effective_at_ms,
            &snapshot.evidence_path,
            &snapshot,
        )?;
        Ok(snapshot)
    }

    pub fn list_instrument_master_snapshots(
        &self,
        user_id: &str,
    ) -> Result<Vec<UsEquityInstrumentMasterSnapshot>, PipelineError> {
        validate_user(user_id)?;
        let jsons = self.read_jsons(
            "SELECT snapshot_json FROM us_equity_instrument_master_snapshots
             WHERE user_id = ?1 ORDER BY retrieved_at_ms, snapshot_id",
            user_id,
        )?;
        jsons
            .into_iter()
            .map(|json| {
                let snapshot: UsEquityInstrumentMasterSnapshot =
                    serde_json::from_str(&json).map_err(storage)?;
                verify_json_evidence(
                    &snapshot.evidence_path,
                    &snapshot,
                    "Alpaca Instrument Master",
                )?;
                self.verify_raw_response(&snapshot.response_sha256)?;
                Ok(snapshot)
            })
            .collect()
    }

    pub fn point_in_time_membership(
        &self,
        user_id: &str,
        observation_time_ms: i64,
    ) -> Result<UsEquityPointInTimeUniverse, PipelineError> {
        if observation_time_ms < 0 {
            return Err(PipelineError::InvalidRequest(
                "U.S. equity universe observation time must be non-negative".into(),
            ));
        }
        let snapshots = self.list_instrument_master_snapshots(user_id)?;
        let snapshot = snapshots
            .iter()
            .filter(|snapshot| snapshot.effective_at_ms <= observation_time_ms)
            .max_by_key(|snapshot| (snapshot.effective_at_ms, snapshot.snapshot_id.clone()))
            .cloned();
        let Some(snapshot) = snapshot else {
            return Ok(UsEquityPointInTimeUniverse {
                universe_id: digest(&canonical_json_bytes(&(observation_time_ms, "unknown"))?),
                as_of_ms: observation_time_ms,
                snapshot_id: None,
                evidence_state: UsEquityUniverseEvidenceState::Unknown,
                evidence_reasons: vec!["instrument-master-unavailable-at-as-of".into()],
                coverage_start_ms: None,
                coverage_end_ms: None,
                instruments: Vec::new(),
            });
        };
        let evidence_state = if snapshot.effective_at_ms == observation_time_ms {
            UsEquityUniverseEvidenceState::Observed
        } else {
            UsEquityUniverseEvidenceState::Reconstructed
        };
        let coverage_end_ms = snapshots
            .iter()
            .filter(|candidate| candidate.effective_at_ms > snapshot.effective_at_ms)
            .map(|candidate| candidate.effective_at_ms)
            .min();
        let instruments = snapshot
            .instruments
            .into_iter()
            .filter(|instrument| instrument.status == InstrumentStatus::Live)
            .collect::<Vec<_>>();
        Ok(UsEquityPointInTimeUniverse {
            universe_id: digest(&canonical_json_bytes(&(
                observation_time_ms,
                &snapshot.snapshot_id,
                &evidence_state,
                &instruments,
            ))?),
            as_of_ms: observation_time_ms,
            snapshot_id: Some(snapshot.snapshot_id),
            evidence_state,
            evidence_reasons: match evidence_state {
                UsEquityUniverseEvidenceState::Observed => {
                    vec!["instrument-master-observed-at-as-of".into()]
                }
                UsEquityUniverseEvidenceState::Reconstructed => {
                    vec!["instrument-master-reconstructed-from-prior-observation".into()]
                }
                UsEquityUniverseEvidenceState::Unknown => unreachable!(),
            },
            coverage_start_ms: Some(snapshot.effective_at_ms),
            coverage_end_ms,
            instruments,
        })
    }

    pub async fn acquire_calendar(
        &self,
        user_id: &str,
        client: &AlpacaClient,
        venue: adaq_data_core::market::Venue,
        range: HistoricalBarRange,
        cancellation: &CancellationToken,
        retrieved_at_ms: i64,
    ) -> Result<UsEquityCalendarSnapshot, PipelineError> {
        validate_user(user_id)?;
        self.ensure_user_available(user_id)?;
        let acquisition = client
            .acquire_calendar(venue, range, retrieved_at_ms)
            .await
            .map_err(|error| self.connector_error(error))?;
        if cancellation.is_cancelled() {
            return Err(PipelineError::Cancelled {
                source_id: "alpaca-calendar".into(),
            });
        }
        self.record_calendar(user_id, client, acquisition)
    }

    pub fn record_calendar(
        &self,
        user_id: &str,
        client: &AlpacaClient,
        acquisition: AlpacaCalendarAcquisition,
    ) -> Result<UsEquityCalendarSnapshot, PipelineError> {
        validate_user(user_id)?;
        self.ensure_user_available(user_id)?;
        if acquisition.provider != "alpaca"
            || acquisition.retrieved_at_ms < 0
            || acquisition.response_sha256.trim().is_empty()
            || acquisition.raw_response.is_empty()
            || acquisition.snapshot.venue.kind != VenueKind::UsEquity
            || acquisition.snapshot.venue.time_zone != "America/New_York"
        {
            return Err(PipelineError::InvalidRequest(
                "Alpaca U.S. equity calendar provenance is incomplete".into(),
            ));
        }
        self.retain_raw_response(&acquisition.response_sha256, &acquisition.raw_response)?;
        let content_sha256 = digest(&acquisition.raw_response);
        let parsed_sha256 = digest(&serde_json::to_vec(&acquisition.snapshot).map_err(storage)?);
        if acquisition.content_sha256 != content_sha256 || parsed_sha256 != content_sha256 {
            return Err(PipelineError::InvalidRequest(
                "Alpaca calendar content hash does not match retained snapshot".into(),
            ));
        }
        let capability_snapshot = capability_snapshot(
            &client.capability_snapshot(acquisition.retrieved_at_ms),
            &acquisition.limitations,
            acquisition.retrieved_at_ms,
            &["trading-calendar"],
            &[],
        );
        let snapshot_id = acquisition.snapshot.snapshot_id.clone();
        let evidence_path = self
            .pipeline
            .root_dir()
            .join("us-equity/calendars")
            .join(format!("{snapshot_id}.json"));
        let snapshot = UsEquityCalendarSnapshot {
            snapshot: acquisition.snapshot,
            provider: acquisition.provider,
            actual_upstream: acquisition.actual_upstream,
            method: acquisition.method,
            connector_version: acquisition.connector_version,
            request_parameters: acquisition.request_parameters,
            retrieved_at_ms: acquisition.retrieved_at_ms,
            response_sha256: acquisition.response_sha256,
            content_sha256,
            diagnostics: acquisition.diagnostics,
            capability_snapshot,
            limitations: acquisition.limitations,
            evidence_path,
        };
        self.persist_snapshot(
            user_id,
            "us_equity_calendar_snapshots",
            &snapshot.snapshot.snapshot_id,
            snapshot.retrieved_at_ms,
            &snapshot.evidence_path,
            &snapshot,
        )?;
        Ok(snapshot)
    }

    pub async fn backfill<F>(
        &self,
        request: UsEquityBackfillRequest,
        client: &AlpacaClient,
        cancellation: CancellationToken,
        mut on_event: F,
    ) -> Result<Option<PipelinePublication>, PipelineError>
    where
        F: FnMut(UsEquityBackfillEvent),
    {
        validate_backfill_request(&request)?;
        self.ensure_user_available(&request.user_id)?;
        let key = backfill_key(&request.user_id, &request.task_id);
        let guard = ActivityGuard {
            pipeline: self.pipeline.clone(),
            active: self.active.clone(),
            key: key.clone(),
        };
        let checkpoint = self.read_checkpoint(&request.user_id, &request.task_id)?;
        if let Some(checkpoint) = checkpoint.as_ref() {
            if checkpoint.request != request {
                return Err(PipelineError::InvalidRequest(
                    "Alpaca task ID is already bound to a different request".into(),
                ));
            }
            if checkpoint.state == UsEquityBackfillState::Completed
                && self.checkpoint_publication_is_intact(checkpoint)
            {
                let source_id = checkpoint.source_id.clone().ok_or_else(|| {
                    PipelineError::Storage("completed Alpaca checkpoint has no Source ID".into())
                })?;
                on_event(UsEquityBackfillEvent::AlreadyCompleted {
                    task_id: request.task_id.clone(),
                    source_id,
                });
                drop(guard);
                return Ok(None);
            }
        }
        if cancellation.is_cancelled() {
            self.mark_checkpoint(
                &request,
                UsEquityBackfillState::Cancelled,
                None,
                None,
                Some("cancelled"),
            )?;
            on_event(UsEquityBackfillEvent::Cancelled {
                task_id: request.task_id,
            });
            drop(guard);
            return Err(PipelineError::Cancelled { source_id: key });
        }
        on_event(UsEquityBackfillEvent::Started {
            task_id: request.task_id.clone(),
            instrument: request.instrument.clone(),
        });
        let resume = self.load_checkpoint_acquisition(checkpoint.as_ref())?;
        self.write_checkpoint(&UsEquityBackfillCheckpoint {
            task_id: request.task_id.clone(),
            user_id: request.user_id.clone(),
            request: request.clone(),
            state: UsEquityBackfillState::Running,
            source_id: None,
            canonical_id: None,
            revision: None,
            completed_through_ms: None,
            acquisition_path: checkpoint.and_then(|value| value.acquisition_path),
            acquisition_sha256: None,
            failure_response_sha256: None,
            last_error: None,
        })?;
        on_event(UsEquityBackfillEvent::AcquisitionStarted {
            instrument: request.instrument.clone(),
            interval: request.interval,
        });
        let acquisition = if let Some(acquisition) = resume {
            acquisition
        } else {
            let result = client
                .acquire_bars(
                    request.instrument.clone(),
                    request.interval,
                    HistoricalBarRange {
                        start_time_ms: request.start_time_ms,
                        end_time_ms: request.end_time_ms,
                    },
                    now_ms(),
                    || cancellation.is_cancelled(),
                )
                .await;
            match result {
                Ok(value) => value,
                Err(error) => {
                    let response_hash = error
                        .response_sha256
                        .clone()
                        .or_else(|| error.raw_response.as_ref().map(|bytes| digest(bytes)));
                    if let Some(bytes) = error.raw_response.as_deref()
                        && let Some(hash) = response_hash.as_deref()
                    {
                        self.retain_raw_response(hash, bytes)?;
                    }
                    let message = error.message.clone();
                    let state = if cancellation.is_cancelled() || error.code == "cancelled" {
                        UsEquityBackfillState::Cancelled
                    } else {
                        UsEquityBackfillState::Failed
                    };
                    self.mark_checkpoint(
                        &request,
                        state.clone(),
                        None,
                        response_hash,
                        Some(&message),
                    )?;
                    if state == UsEquityBackfillState::Cancelled {
                        on_event(UsEquityBackfillEvent::Cancelled {
                            task_id: request.task_id,
                        });
                        drop(guard);
                        return Err(PipelineError::Cancelled { source_id: key });
                    }
                    on_event(UsEquityBackfillEvent::Failed {
                        task_id: request.task_id,
                        message: message.clone(),
                    });
                    drop(guard);
                    return Err(connector_error(error));
                }
            }
        };
        if let Err(error) = self.retain_raw_responses(&acquisition) {
            self.mark_checkpoint(
                &request,
                UsEquityBackfillState::Failed,
                None,
                None,
                Some(&error.to_string()),
            )?;
            on_event(UsEquityBackfillEvent::Failed {
                task_id: request.task_id,
                message: error.to_string(),
            });
            drop(guard);
            return Err(error);
        }
        let acquisition_path = self.acquisition_path(&request.user_id, &request.task_id);
        let acquisition_bytes = serde_json::to_vec(&acquisition).map_err(storage)?;
        atomic_write(&acquisition_path, &acquisition_bytes)?;
        self.mark_checkpoint_path(
            &request,
            Some(acquisition_path.clone()),
            Some(digest(&acquisition_bytes)),
        )?;
        let source_acquisition = source_acquisition(&acquisition)?;
        let mut canonicalization = CanonicalizationRequest::new(
            request.instrument.clone(),
            request.interval,
            CalendarEvidence::Venue {
                snapshot: request.calendar.clone(),
            },
        )?;
        canonicalization.historical_range = Some(HistoricalBarRange {
            start_time_ms: request.start_time_ms,
            end_time_ms: request.end_time_ms,
        });
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
                    UsEquityBackfillState::Cancelled
                } else {
                    UsEquityBackfillState::Failed
                };
                self.mark_checkpoint(
                    &request,
                    state.clone(),
                    None,
                    None,
                    Some(&error.to_string()),
                )?;
                if state == UsEquityBackfillState::Cancelled {
                    on_event(UsEquityBackfillEvent::Cancelled {
                        task_id: request.task_id,
                    });
                } else {
                    on_event(UsEquityBackfillEvent::Failed {
                        task_id: request.task_id,
                        message: error.to_string(),
                    });
                }
                drop(guard);
                return Err(error);
            }
        };
        let state = match publication.quality.state {
            DataQualityState::Passed => UsEquityBackfillState::Completed,
            DataQualityState::Degraded => UsEquityBackfillState::Degraded,
            DataQualityState::Rejected => UsEquityBackfillState::Rejected,
        };
        self.write_checkpoint(&UsEquityBackfillCheckpoint {
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
            acquisition_sha256: Some(acquisition.content_sha256),
            failure_response_sha256: None,
            last_error: None,
        })?;
        let _ = fs::remove_file(acquisition_path);
        on_event(UsEquityBackfillEvent::Published {
            instrument: request.instrument,
            source_id: publication.source.source_id.clone(),
            canonical_id: publication
                .canonical
                .as_ref()
                .map(|value| value.canonical_id.clone()),
            revision: publication.source.revision,
            state: publication.quality.state.clone(),
        });
        drop(guard);
        Ok(Some(publication))
    }

    pub async fn snapshot(
        &self,
        client: &AlpacaClient,
        instrument: InstrumentId,
        retrieved_at_ms: i64,
    ) -> Result<UsEquityMarketSnapshotDto, PipelineError> {
        if instrument.venue.kind != VenueKind::UsEquity {
            return Err(PipelineError::InvalidRequest(
                "Alpaca snapshot Instrument must be U.S. equity".into(),
            ));
        }
        let snapshot = client
            .get_snapshot(&instrument.code, retrieved_at_ms)
            .await
            .map_err(|error| self.connector_error(error))?;
        Ok(snapshot_dto(snapshot, instrument, now_ms()))
    }

    pub fn acquisition_status(
        &self,
        user_id: &str,
        task_id: &str,
    ) -> Result<Option<UsEquityAcquisitionStatus>, PipelineError> {
        Ok(self
            .read_checkpoint(user_id, task_id)?
            .map(|checkpoint| UsEquityAcquisitionStatus {
                task_id: checkpoint.task_id,
                user_id: checkpoint.user_id,
                state: checkpoint.state,
                source_id: checkpoint.source_id,
                canonical_id: checkpoint.canonical_id,
                revision: checkpoint.revision,
                completed_through_ms: checkpoint.completed_through_ms,
                last_error: checkpoint.last_error,
            }))
    }

    pub fn workspace_dto_for_user(
        &self,
        user_id: &str,
        source_id: &str,
        now_ms: i64,
    ) -> Result<UsEquityMarketWorkspaceDto, PipelineError> {
        validate_user(user_id)?;
        let source = self.pipeline.source_for_user(user_id, source_id)?;
        let report_id: String = self
            .pipeline
            .database()
            .lock()
            .map_err(lock_error)?
            .query_row(
                "SELECT q.report_id FROM pipeline_quality_reports q
                 JOIN pipeline_quality_access qa ON qa.report_id = q.report_id
                 WHERE qa.user_id = ?1 AND q.source_id = ?2
                 ORDER BY q.report_id DESC LIMIT 1",
                params![user_id, source_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(storage)?
            .ok_or_else(|| PipelineError::NotFound("Alpaca Data Quality Report".into()))?;
        let quality = self.pipeline.quality_for_user(user_id, &report_id)?;
        let canonical = quality
            .canonical_id
            .as_deref()
            .map(|id| self.pipeline.canonical_for_user(user_id, id))
            .transpose()?;
        let instrument = canonical
            .as_ref()
            .map(|value| value.instrument.clone())
            .or_else(|| source.records.first().map(|value| value.instrument.clone()))
            .ok_or_else(|| PipelineError::NotFound("Alpaca workspace Instrument".into()))?;
        let calendar_id = canonical
            .as_ref()
            .map(|value| match &value.calendar {
                CalendarEvidence::Venue { snapshot } => snapshot.snapshot_id.clone(),
                CalendarEvidence::UtcGrid { calendar_id, .. } => calendar_id.clone(),
            })
            .unwrap_or_else(|| "unknown".into());
        Ok(UsEquityMarketWorkspaceDto {
            instrument,
            provider: source.identity.provider.clone(),
            actual_upstream: source.identity.actual_upstream.clone(),
            connector: source.identity.connector.clone(),
            connector_version: source.identity.connector_version.clone(),
            retrieved_at_ms: source.identity.retrieved_at_ms,
            freshness_ms: (now_ms >= source.identity.retrieved_at_ms)
                .then_some(now_ms - source.identity.retrieved_at_ms),
            feed: source.identity.capability_snapshot.feed.clone(),
            capability_snapshot: source.identity.capability_snapshot.clone(),
            price_basis: source.identity.price_basis,
            calendar_id,
            quality: quality.state.clone(),
            source_id: source.source_id,
            canonical_id: quality.canonical_id,
            revision: source.revision,
            coverage_start_ms: quality.coverage.start_time_ms,
            coverage_end_ms: quality.coverage.end_time_ms,
            gap_count: quality.gap_count,
            limitations: quality.capability_limitations,
        })
    }

    pub fn reset_paths_for_user(&self, user_id: &str) -> Result<Vec<PathBuf>, PipelineError> {
        validate_user(user_id)?;
        let database = self.pipeline.database();
        let guard = database.lock().map_err(lock_error)?;
        self.reset_paths_for_user_with_connection(&guard, user_id)
    }

    pub fn reset_paths_for_user_with_connection(
        &self,
        database: &Connection,
        user_id: &str,
    ) -> Result<Vec<PathBuf>, PipelineError> {
        validate_user(user_id)?;
        let mut paths = Vec::new();
        for (table, json_column, field) in [
            (
                "us_equity_instrument_master_snapshots",
                "snapshot_json",
                "evidencePath",
            ),
            (
                "us_equity_calendar_snapshots",
                "snapshot_json",
                "evidencePath",
            ),
            (
                "us_equity_backfill_checkpoints",
                "checkpoint_json",
                "acquisitionPath",
            ),
        ] {
            let sql = format!(
                "SELECT current.{json_column} FROM {table} current
                 WHERE current.user_id = ?1 AND NOT EXISTS (
                   SELECT 1 FROM {table} other
                   WHERE other.user_id <> ?1
                     AND json_extract(other.{json_column}, '$.{field}')
                         = json_extract(current.{json_column}, '$.{field}')
                 )"
            );
            let mut statement = database.prepare(&sql).map_err(storage)?;
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
        let raw = self.raw_paths_for_user(database, user_id)?;
        paths.extend(raw);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }

    pub fn reset_user_rows(
        &self,
        transaction: &Transaction<'_>,
        user_id: &str,
    ) -> Result<(), PipelineError> {
        validate_user(user_id)?;
        for table in [
            "us_equity_instrument_master_snapshots",
            "us_equity_calendar_snapshots",
            "us_equity_backfill_checkpoints",
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

    fn initialize_schema(&self) -> Result<(), PipelineError> {
        self.pipeline
            .database()
            .lock()
            .map_err(lock_error)?
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS us_equity_instrument_master_snapshots (
                    user_id TEXT NOT NULL,
                    snapshot_id TEXT NOT NULL,
                    retrieved_at_ms INTEGER NOT NULL,
                    snapshot_json TEXT NOT NULL,
                    PRIMARY KEY(user_id, snapshot_id)
                 );
                 CREATE TABLE IF NOT EXISTS us_equity_calendar_snapshots (
                    user_id TEXT NOT NULL,
                    snapshot_id TEXT NOT NULL,
                    retrieved_at_ms INTEGER NOT NULL,
                    snapshot_json TEXT NOT NULL,
                    PRIMARY KEY(user_id, snapshot_id)
                 );
                 CREATE TABLE IF NOT EXISTS us_equity_backfill_checkpoints (
                    user_id TEXT NOT NULL,
                    task_id TEXT NOT NULL,
                    checkpoint_json TEXT NOT NULL,
                    PRIMARY KEY(user_id, task_id)
                 );",
            )
            .map_err(storage)
    }

    fn persist_snapshot<T: Serialize>(
        &self,
        user_id: &str,
        table: &str,
        snapshot_id: &str,
        retrieved_at_ms: i64,
        path: &Path,
        snapshot: &T,
    ) -> Result<(), PipelineError> {
        let bytes = canonical_json_bytes(snapshot)?;
        atomic_write(path, &bytes)?;
        let json = serde_json::to_string(snapshot).map_err(storage)?;
        self.pipeline
            .database()
            .lock()
            .map_err(lock_error)?
            .execute(
                &format!(
                    "INSERT OR IGNORE INTO {table}
                     (user_id, snapshot_id, retrieved_at_ms, snapshot_json)
                     VALUES (?1, ?2, ?3, ?4)"
                ),
                params![user_id, snapshot_id, retrieved_at_ms, json],
            )
            .map_err(storage)?;
        Ok(())
    }

    fn read_jsons(&self, sql: &str, user_id: &str) -> Result<Vec<String>, PipelineError> {
        let database = self.pipeline.database();
        let guard = database.lock().map_err(lock_error)?;
        let mut statement = guard.prepare(sql).map_err(storage)?;
        statement
            .query_map([user_id], |row| row.get::<_, String>(0))
            .map_err(storage)?
            .map(|row| row.map_err(storage))
            .collect()
    }

    fn retain_raw_responses(
        &self,
        acquisition: &AlpacaBarsAcquisition,
    ) -> Result<(), PipelineError> {
        if acquisition.response_sha256s.len() != acquisition.raw_responses.len()
            || acquisition.response_sha256s.is_empty()
        {
            return Err(PipelineError::InvalidRequest(
                "Alpaca raw responses must match non-empty provenance hashes".into(),
            ));
        }
        for (hash, bytes) in acquisition
            .response_sha256s
            .iter()
            .zip(&acquisition.raw_responses)
        {
            self.retain_raw_response(hash, bytes)?;
        }
        Ok(())
    }

    fn retain_raw_response(&self, hash: &str, bytes: &[u8]) -> Result<(), PipelineError> {
        if hash.trim().is_empty() || bytes.is_empty() || digest(bytes) != hash {
            return Err(PipelineError::InvalidRequest(
                "Alpaca raw response hash does not match retained bytes".into(),
            ));
        }
        atomic_write(&self.raw_response_path(hash), bytes)
    }

    fn verify_raw_response(&self, hash: &str) -> Result<(), PipelineError> {
        let path = self.raw_response_path(hash);
        if !path.is_file() || digest(&fs::read(&path).map_err(storage)?) != hash {
            return Err(PipelineError::Storage(
                "Alpaca retained raw response evidence is missing or corrupt".into(),
            ));
        }
        Ok(())
    }

    fn raw_response_path(&self, hash: &str) -> PathBuf {
        self.pipeline
            .root_dir()
            .join("us-equity/raw")
            .join(format!("{hash}.bin"))
    }

    fn acquisition_path(&self, user_id: &str, task_id: &str) -> PathBuf {
        self.pipeline
            .root_dir()
            .join("us-equity/checkpoints")
            .join(format!(
                "{}.json",
                digest(format!("{user_id}\0{task_id}").as_bytes())
            ))
    }

    fn read_checkpoint(
        &self,
        user_id: &str,
        task_id: &str,
    ) -> Result<Option<UsEquityBackfillCheckpoint>, PipelineError> {
        validate_user(user_id)?;
        let json = self
            .pipeline
            .database()
            .lock()
            .map_err(lock_error)?
            .query_row(
                "SELECT checkpoint_json FROM us_equity_backfill_checkpoints
                 WHERE user_id = ?1 AND task_id = ?2",
                params![user_id, task_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(storage)?;
        json.map(|value| serde_json::from_str(&value).map_err(storage))
            .transpose()
    }

    fn load_checkpoint_acquisition(
        &self,
        checkpoint: Option<&UsEquityBackfillCheckpoint>,
    ) -> Result<Option<AlpacaBarsAcquisition>, PipelineError> {
        let Some(checkpoint) = checkpoint else {
            return Ok(None);
        };
        let (Some(path), Some(expected_hash)) = (
            checkpoint.acquisition_path.as_ref(),
            checkpoint.acquisition_sha256.as_deref(),
        ) else {
            return Ok(None);
        };
        let bytes = fs::read(path).map_err(storage)?;
        if digest(&bytes) != expected_hash {
            return Err(PipelineError::Storage(
                "Alpaca checkpoint acquisition is missing or corrupt".into(),
            ));
        }
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|error| storage(format!("invalid Alpaca checkpoint acquisition: {error}")))
    }

    fn write_checkpoint(
        &self,
        checkpoint: &UsEquityBackfillCheckpoint,
    ) -> Result<(), PipelineError> {
        let json = serde_json::to_string(checkpoint).map_err(storage)?;
        self.pipeline
            .database()
            .lock()
            .map_err(lock_error)?
            .execute(
                "INSERT INTO us_equity_backfill_checkpoints
                 (user_id, task_id, checkpoint_json) VALUES (?1, ?2, ?3)
                 ON CONFLICT(user_id, task_id) DO UPDATE SET checkpoint_json = excluded.checkpoint_json",
                params![checkpoint.user_id, checkpoint.task_id, json],
            )
            .map_err(storage)?;
        Ok(())
    }

    fn update_checkpoint<F>(
        &self,
        user_id: &str,
        task_id: &str,
        update: F,
    ) -> Result<(), PipelineError>
    where
        F: FnOnce(&mut UsEquityBackfillCheckpoint),
    {
        let mut checkpoint = self
            .read_checkpoint(user_id, task_id)?
            .ok_or_else(|| PipelineError::NotFound("Alpaca backfill checkpoint".into()))?;
        update(&mut checkpoint);
        self.write_checkpoint(&checkpoint)
    }

    fn mark_checkpoint(
        &self,
        request: &UsEquityBackfillRequest,
        state: UsEquityBackfillState,
        source_id: Option<String>,
        failure_response_sha256: Option<String>,
        last_error: Option<&str>,
    ) -> Result<(), PipelineError> {
        if self
            .read_checkpoint(&request.user_id, &request.task_id)?
            .is_none()
        {
            self.write_checkpoint(&UsEquityBackfillCheckpoint {
                task_id: request.task_id.clone(),
                user_id: request.user_id.clone(),
                request: request.clone(),
                state: state.clone(),
                source_id,
                canonical_id: None,
                revision: None,
                completed_through_ms: None,
                acquisition_path: None,
                acquisition_sha256: None,
                failure_response_sha256,
                last_error: last_error.map(str::to_owned),
            })
        } else {
            self.update_checkpoint(&request.user_id, &request.task_id, |checkpoint| {
                checkpoint.state = state;
                if source_id.is_some() {
                    checkpoint.source_id = source_id;
                }
                checkpoint.failure_response_sha256 = failure_response_sha256;
                checkpoint.last_error = last_error.map(str::to_owned);
            })
        }
    }

    fn mark_checkpoint_path(
        &self,
        request: &UsEquityBackfillRequest,
        acquisition_path: Option<PathBuf>,
        acquisition_sha256: Option<String>,
    ) -> Result<(), PipelineError> {
        self.update_checkpoint(&request.user_id, &request.task_id, |checkpoint| {
            checkpoint.acquisition_path = acquisition_path;
            checkpoint.acquisition_sha256 = acquisition_sha256;
        })
    }

    fn checkpoint_publication_is_intact(&self, checkpoint: &UsEquityBackfillCheckpoint) -> bool {
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
        let Ok(canonical) = self
            .pipeline
            .canonical_for_user(&checkpoint.user_id, canonical_id)
        else {
            return false;
        };
        source.revision == revision
            && canonical.source_id == source.source_id
            && canonical.revision == revision
            && self
                .pipeline
                .quality_for_user(&checkpoint.user_id, &canonical.quality_report_id)
                .is_ok_and(|quality| {
                    matches!(
                        (&checkpoint.state, quality.state),
                        (UsEquityBackfillState::Completed, DataQualityState::Passed)
                            | (UsEquityBackfillState::Degraded, DataQualityState::Degraded)
                    )
                })
    }

    fn raw_paths_for_user(
        &self,
        database: &Connection,
        user_id: &str,
    ) -> Result<Vec<PathBuf>, PipelineError> {
        let queries = [
            "SELECT a.user_id, s.source_json FROM pipeline_sources s JOIN pipeline_source_access a USING(source_id)",
            "SELECT user_id, snapshot_json FROM us_equity_instrument_master_snapshots",
            "SELECT user_id, snapshot_json FROM us_equity_calendar_snapshots",
            "SELECT user_id, checkpoint_json FROM us_equity_backfill_checkpoints",
        ];
        let mut current = HashSet::new();
        let mut other = HashSet::new();
        for sql in queries {
            let mut statement = database.prepare(sql).map_err(storage)?;
            let rows = statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(storage)?;
            for row in rows {
                let (row_user, json) = row.map_err(storage)?;
                let value: Value = serde_json::from_str(&json).map_err(storage)?;
                let hashes = if row_user == user_id {
                    &mut current
                } else {
                    &mut other
                };
                collect_hashes(&value, hashes);
                if let Some(path) = value
                    .get("acquisitionPath")
                    .and_then(Value::as_str)
                    .map(PathBuf::from)
                    .filter(|path| path.is_file())
                {
                    let acquisition: Value =
                        serde_json::from_slice(&fs::read(path).map_err(storage)?)
                            .map_err(storage)?;
                    collect_hashes(&acquisition, hashes);
                }
            }
        }
        Ok(current
            .difference(&other)
            .map(|hash| self.raw_response_path(hash))
            .filter(|path| path.is_file())
            .collect())
    }
}

fn validate_instruments(instruments: &[AlpacaInstrument]) -> Result<(), PipelineError> {
    let mut identities = HashSet::new();
    let mut provider_symbols = HashSet::new();
    for instrument in instruments {
        if instrument.instrument.venue.kind != VenueKind::UsEquity
            || instrument.instrument.venue.time_zone != "America/New_York"
            || instrument.mapping.instrument != instrument.instrument
            || instrument.mapping.provider != "alpaca"
            || instrument.mapping.provider_symbol != instrument.provider_symbol
            || !identities.insert(instrument.instrument.clone())
            || !provider_symbols.insert(instrument.provider_symbol.clone())
        {
            return Err(PipelineError::InvalidRequest(
                "Alpaca Instrument Master identity mapping is invalid".into(),
            ));
        }
    }
    Ok(())
}

fn capability_snapshot(
    capability: &AlpacaCapabilitySnapshot,
    limitations: &[String],
    captured_at_ms: i64,
    record_types: &[&str],
    instruments: &[AlpacaInstrument],
) -> ProviderCapabilitySnapshot {
    let mut all_limitations = limitations.to_vec();
    all_limitations.push("Basic U.S. equity feed is IEX-only; no consolidated/full-market realtime claim is permitted".into());
    all_limitations.push(
        "Canonical Bars are Unadjusted; corporate actions are not merged or used for gap repair"
            .into(),
    );
    ProviderCapabilitySnapshot {
        provider: "alpaca".into(),
        captured_at_ms,
        subscription_plan: Some(capability.subscription_plan.clone()),
        feed: Some(capability.feed.clone()),
        coverage: Some(capability.coverage.clone()),
        realtime: Some(capability.realtime),
        venues: if instruments.is_empty() {
            vec!["us-equity".into()]
        } else {
            instruments
                .iter()
                .map(|value| value.instrument.venue.id.clone())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect()
        },
        record_types: record_types.iter().map(|value| (*value).into()).collect(),
        history_start_ms: capability.history_start_ms,
        history_end_ms: capability.historical_latest_cutoff_ms,
        delayed: capability.delayed,
        delayed_known: true,
        delay_ms: capability.delay_ms,
        rate_limit: Some("200 historical requests per minute".into()),
        rate_limit_known: true,
        requests_per_minute: Some(capability.historical_calls_per_minute),
        stream_connection_limit: Some(capability.stream_connection_limit as u32),
        streaming_symbol_limit: Some(capability.stream_symbol_limit as u32),
        unavailable_capabilities: capability.unavailable_capabilities.clone(),
        limitations: all_limitations,
    }
}

fn source_acquisition(
    acquisition: &AlpacaBarsAcquisition,
) -> Result<SourceAcquisition, PipelineError> {
    let content_sha256 = digest(
        &serde_json::to_vec(&(&acquisition.bars, &acquisition.invalid_bars)).map_err(storage)?,
    );
    if acquisition.content_sha256 != content_sha256 {
        return Err(PipelineError::InvalidRequest(
            "Alpaca Bars content hash does not match retained rows".into(),
        ));
    }
    let capability = AlpacaCapabilitySnapshot::basic(acquisition.retrieved_at_ms);
    Ok(SourceAcquisition {
        provider: acquisition.provider.clone(),
        actual_upstream: Some(acquisition.actual_upstream.clone()),
        connector: "adaq-alpaca-market-data".into(),
        connector_version: acquisition.connector_version.clone(),
        request_parameters: acquisition.request_parameters.clone(),
        retrieved_at_ms: acquisition.retrieved_at_ms,
        response_sha256s: acquisition.response_sha256s.clone(),
        acquisition_content_sha256: Some(acquisition.content_sha256.clone()),
        capability_snapshot: capability_snapshot(
            &capability,
            &acquisition.limitations,
            acquisition.retrieved_at_ms,
            &["unadjusted-bars"],
            &[],
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
            .map(|bar| SourceMarketRecord {
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
            })
            .collect(),
    })
}

fn snapshot_dto(
    snapshot: AlpacaMarketSnapshot,
    instrument: InstrumentId,
    now_ms: i64,
) -> UsEquityMarketSnapshotDto {
    UsEquityMarketSnapshotDto {
        provider: "alpaca".into(),
        instrument,
        feed: snapshot.feed,
        retrieved_at_ms: snapshot.retrieved_at_ms,
        freshness_ms: (now_ms >= snapshot.retrieved_at_ms)
            .then_some(now_ms - snapshot.retrieved_at_ms),
        response_sha256: snapshot.response_sha256,
        ticker: snapshot.ticker,
        trade: snapshot.trade,
        quote: snapshot.quote,
    }
}

fn collect_hashes(value: &Value, hashes: &mut HashSet<String>) {
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                if matches!(key.as_str(), "responseSha256" | "failureResponseSha256") {
                    if let Some(hash) = value.as_str().filter(|hash| !hash.trim().is_empty()) {
                        hashes.insert(hash.to_owned());
                    }
                } else if key == "responseSha256s" {
                    if let Value::Array(values) = value {
                        hashes.extend(
                            values
                                .iter()
                                .filter_map(Value::as_str)
                                .filter(|hash| !hash.trim().is_empty())
                                .map(str::to_owned),
                        );
                    }
                }
                collect_hashes(value, hashes);
            }
        }
        Value::Array(values) => values
            .iter()
            .for_each(|value| collect_hashes(value, hashes)),
        _ => {}
    }
}

fn verify_json_evidence<T: Serialize>(
    path: &Path,
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

fn connector_error(error: adaq_data_core::DataError) -> PipelineError {
    PipelineError::Connector {
        code: error.code,
        message: error.message,
    }
}

fn acquisition_key(user_id: &str, operation_id: &str) -> String {
    format!(
        "us-equity:{}:{user_id}:acquisition:{}:{operation_id}",
        user_id.len(),
        operation_id.len()
    )
}

fn backfill_key(user_id: &str, task_id: &str) -> String {
    format!(
        "us-equity:{}:{user_id}:backfill:{}:{task_id}",
        user_id.len(),
        task_id.len()
    )
}

fn validate_backfill_request(request: &UsEquityBackfillRequest) -> Result<(), PipelineError> {
    validate_user(&request.user_id)?;
    if request.task_id.trim().is_empty()
        || request.start_time_ms >= request.end_time_ms
        || request.instrument.venue.kind != VenueKind::UsEquity
        || request.instrument.venue.time_zone != "America/New_York"
        || request.calendar.venue != request.instrument.venue
        || request.calendar.venue.time_zone != "America/New_York"
        || request.calendar.effective_from_ms > request.start_time_ms
        || request.calendar.effective_to_ms < request.end_time_ms
        || request.end_time_ms.saturating_sub(request.start_time_ms) > 20 * 366 * 86_400_000
    {
        return Err(PipelineError::InvalidRequest(
            "Alpaca U.S. equity backfill request is invalid".into(),
        ));
    }
    Ok(())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|value| i64::try_from(value.as_millis()).ok())
        .unwrap_or_default()
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), PipelineError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(storage)?;
    }
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, bytes).map_err(storage)?;
    fs::rename(temporary, path).map_err(storage)
}

fn storage(error: impl std::fmt::Display) -> PipelineError {
    PipelineError::Storage(error.to_string())
}

fn lock_error(error: impl std::fmt::Display) -> PipelineError {
    PipelineError::Storage(format!("Alpaca pipeline database lock failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use adaq_data_core::{alpaca::AlpacaCredentials, market::Venue};
    use rusqlite::Connection;
    use tempfile::tempdir;

    #[test]
    fn capability_snapshot_is_explicitly_iex_only() {
        let capability = capability_snapshot(
            &AlpacaCapabilitySnapshot::basic(1_700_000_000_000),
            &[],
            1_700_000_000_000,
            &["bars"],
            &[],
        );
        assert_eq!(capability.feed.as_deref(), Some("iex"));
        assert_eq!(capability.requests_per_minute, Some(200));
        assert_eq!(capability.stream_connection_limit, Some(1));
        assert!(
            capability
                .unavailable_capabilities
                .iter()
                .any(|value| value == "consolidated-us-equity-realtime")
        );
        let credentials = AlpacaCredentials::new("key", "secret");
        let debug = format!("{:?}", capability);
        assert!(!debug.contains("secret"));
        drop(credentials);
    }

    #[test]
    fn user_scoped_calendar_evidence_survives_round_trip() {
        let directory = tempdir().unwrap();
        let database = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let pipeline = DataPipeline::open(directory.path(), database).unwrap();
        let path = UsEquityDataPath::open(pipeline).unwrap();
        let venue = Venue::us_equity("nasdaq").unwrap();
        let calendar = adaq_data_core::alpaca::AlpacaClient::with_key_pair("k", "s");
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let result = runtime.block_on(calendar.acquire_calendar(
            venue,
            HistoricalBarRange {
                start_time_ms: 1_704_067_200_000,
                end_time_ms: 1_704_240_000_000,
            },
            1_704_067_200_000,
        ));
        let acquisition = result.unwrap();
        let snapshot = path
            .record_calendar("user-a", &calendar, acquisition)
            .unwrap();
        assert_eq!(snapshot.snapshot.venue.time_zone, "America/New_York");
        assert_eq!(
            path.list_instrument_master_snapshots("user-a")
                .unwrap()
                .len(),
            0
        );
        assert_eq!(snapshot.capability_snapshot.feed.as_deref(), Some("iex"));
    }

    #[test]
    fn alpaca_source_reaches_canonical_quality_with_exact_decimals() {
        let directory = tempdir().unwrap();
        let database = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let pipeline = DataPipeline::open(directory.path(), database).unwrap();
        let venue = Venue::us_equity("nasdaq").unwrap();
        let client = AlpacaClient::with_key_pair("key", "secret");
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let calendar = runtime
            .block_on(client.acquire_calendar(
                venue.clone(),
                HistoricalBarRange {
                    start_time_ms: 1_704_163_200_000,
                    end_time_ms: 1_704_249_600_000,
                },
                1_704_163_200_000,
            ))
            .unwrap()
            .snapshot;
        let instrument = InstrumentId::new(venue.clone(), "AAPL").unwrap();
        let open_time_ms = venue
            .resolve_local_time(
                chrono::NaiveDate::from_ymd_opt(2024, 1, 2)
                    .unwrap()
                    .and_hms_opt(9, 30, 0)
                    .unwrap(),
                adaq_data_core::market::LocalTimeDisambiguation::Reject,
            )
            .unwrap();
        let bar = adaq_data_core::alpaca::AlpacaBar {
            instrument: instrument.clone(),
            provider_symbol: "AAPL".into(),
            interval: BarInterval::OneMinute,
            open_time_ms,
            open: Some("1.2300".into()),
            high: Some("1.2400".into()),
            low: Some("1.2200".into()),
            close: Some("1.2350".into()),
            base_volume: Some("2".into()),
            quote_volume: Some("2.4600".into()),
            raw_payload: serde_json::json!({"o":"1.2300","c":"1.2350"}),
        };
        let invalid = Vec::new();
        let raw_response = br#"{"bars":{"AAPL":[]}}"#.to_vec();
        let acquisition = AlpacaBarsAcquisition {
            provider: "alpaca".into(),
            actual_upstream: "Alpaca Market Data API".into(),
            method: "GET /v2/stocks/{symbol}/bars".into(),
            connector_version: client.connector_version().into(),
            request_parameters: serde_json::json!({"symbol":"AAPL","feed":"iex"}),
            retrieved_at_ms: 1_704_163_200_000,
            response_sha256s: vec![digest(&raw_response)],
            content_sha256: digest(&serde_json::to_vec(&(&vec![bar.clone()], &invalid)).unwrap()),
            raw_responses: vec![raw_response],
            diagnostics: Default::default(),
            bars: vec![bar],
            invalid_bars: invalid,
            limitations: vec!["IEX-only fixture".into()],
        };
        let mut canonicalization = CanonicalizationRequest::new(
            instrument,
            BarInterval::OneMinute,
            CalendarEvidence::Venue { snapshot: calendar },
        )
        .unwrap();
        canonicalization.historical_range = Some(HistoricalBarRange {
            start_time_ms: open_time_ms,
            end_time_ms: open_time_ms + 60_000,
        });
        let publication = pipeline
            .publish(
                "user-a",
                source_acquisition(&acquisition).unwrap(),
                canonicalization,
                CancellationToken::new(),
                |_| {},
            )
            .unwrap();
        assert_eq!(
            publication
                .source
                .identity
                .capability_snapshot
                .feed
                .as_deref(),
            Some("iex")
        );
        assert_eq!(publication.canonical.unwrap().bars.len(), 1);
        assert_ne!(publication.quality.state, DataQualityState::Rejected);
    }
}
