//! OKX Spot acquisition on top of the immutable Source -> Canonical pipeline.
//!
//! This module owns only market-data evidence. Account credentials, balances,
//! orders, and fills never enter this boundary.

use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use adaq_data_core::{
    BarAcquisition, BarInterval, BarSeries, BarSnapshot, InstrumentMasterAcquisition,
    InstrumentStatus, Level2StreamEvent, MarketTrade, OhlcvBar, OkxClient, SpotInstrument,
    TickerStreamEvent, TradeStreamEvent,
    market::{InstrumentId, Venue},
    next_bar_open_time_ms,
};
use rusqlite::{Connection, OptionalExtension, params};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    AcquisitionDiagnostics, CalendarEvidence, CancellationToken, CanonicalizationRequest,
    DataPipeline, DataQualityState, PipelineError, PipelinePublication, ProviderCapabilitySnapshot,
    SourceAcquisition, SourceMarketRecord, canonical_json_bytes, digest, validate_user,
};

const OKX_BAR_INTERVAL: BarInterval = BarInterval::OneMinute;
const DAY_MS: i64 = 86_400_000;
const BAR_OVERLAP_COUNT: i64 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UniverseEvidenceState {
    Observed,
    Reconstructed,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstrumentMasterSnapshot {
    pub snapshot_id: String,
    pub retrieved_at_ms: i64,
    pub response_sha256: String,
    pub connector_version: String,
    pub instruments: Vec<SpotInstrument>,
    pub content_sha256: String,
    pub evidence_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PointInTimeInstrumentUniverse {
    pub universe_id: String,
    pub as_of_ms: i64,
    pub snapshot_id: Option<String>,
    pub evidence_state: UniverseEvidenceState,
    pub instruments: Vec<SpotInstrument>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OkxAcquisitionState {
    Pending,
    Running,
    Completed,
    Degraded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OkxAcquisitionStatus {
    pub instrument: InstrumentId,
    pub interval: BarInterval,
    pub start_time_ms: Option<i64>,
    pub end_time_ms: Option<i64>,
    pub universe_snapshot_id: Option<String>,
    pub state: OkxAcquisitionState,
    pub pages: u64,
    pub next_cursor_ms: Option<i64>,
    pub latest_confirmed_open_time_ms: Option<i64>,
    pub coverage_start_ms: Option<i64>,
    pub coverage_end_ms: Option<i64>,
    pub gap_count: usize,
    pub revision: Option<u64>,
    pub source_id: Option<String>,
    pub retry_count: u32,
    pub backoff_ms: u64,
    pub last_error_code: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OkxBackfillRequest {
    pub task_id: String,
    pub user_id: String,
    pub start_time_ms: i64,
    pub end_time_ms: i64,
    pub interval: BarInterval,
    #[serde(default = "default_gap_retries")]
    pub max_gap_retries: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "event", content = "data")]
pub enum OkxBackfillEvent {
    UniverseLoaded {
        snapshot_id: String,
        instrument_count: usize,
    },
    InstrumentStarted {
        instrument: InstrumentId,
    },
    Page {
        instrument: InstrumentId,
        downloaded_records: usize,
        next_cursor_ms: i64,
    },
    Published {
        instrument: InstrumentId,
        source_id: String,
        canonical_id: Option<String>,
        revision: u64,
        state: DataQualityState,
    },
    InstrumentCompleted {
        instrument: InstrumentId,
        state: OkxAcquisitionState,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OkxTradeRetentionPolicy {
    pub max_age_ms: i64,
    pub max_records_per_instrument: u64,
}

impl Default for OkxTradeRetentionPolicy {
    fn default() -> Self {
        Self {
            max_age_ms: 7 * DAY_MS,
            max_records_per_instrument: 100_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OkxStreamHealth {
    pub stream_kind: String,
    pub status: String,
    pub last_event_at_ms: Option<i64>,
    pub reconnect_count: u32,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub retained_trade_count: u64,
    pub trade_retention_max_age_ms: i64,
    pub trade_retention_max_records: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BackfillCheckpoint {
    #[serde(default)]
    start_time_ms: Option<i64>,
    #[serde(default)]
    end_time_ms: Option<i64>,
    #[serde(default)]
    universe_snapshot_id: Option<String>,
    state: OkxAcquisitionState,
    pages: u64,
    next_cursor_ms: Option<i64>,
    latest_confirmed_open_time_ms: Option<i64>,
    coverage_start_ms: Option<i64>,
    coverage_end_ms: Option<i64>,
    gap_count: usize,
    revision: Option<u64>,
    source_id: Option<String>,
    retry_count: u32,
    backoff_ms: u64,
    #[serde(default)]
    last_error_code: Option<String>,
    last_error: Option<String>,
    #[serde(default)]
    partial_records: Vec<SourceMarketRecord>,
}

impl Default for BackfillCheckpoint {
    fn default() -> Self {
        Self {
            start_time_ms: None,
            end_time_ms: None,
            universe_snapshot_id: None,
            state: OkxAcquisitionState::Pending,
            pages: 0,
            next_cursor_ms: None,
            latest_confirmed_open_time_ms: None,
            coverage_start_ms: None,
            coverage_end_ms: None,
            gap_count: 0,
            revision: None,
            source_id: None,
            retry_count: 0,
            backoff_ms: 0,
            last_error_code: None,
            last_error: None,
            partial_records: Vec::new(),
        }
    }
}

#[derive(Clone)]
pub struct OkxSpotDataPath {
    pipeline: DataPipeline,
    client: OkxClient,
    trade_retention: OkxTradeRetentionPolicy,
    active_backfills: Arc<Mutex<HashMap<String, (String, CancellationToken)>>>,
}

impl OkxSpotDataPath {
    pub fn open(pipeline: DataPipeline, client: OkxClient) -> Result<Self, PipelineError> {
        Self::open_with_trade_retention(pipeline, client, OkxTradeRetentionPolicy::default())
    }

    pub fn open_with_trade_retention(
        pipeline: DataPipeline,
        client: OkxClient,
        trade_retention: OkxTradeRetentionPolicy,
    ) -> Result<Self, PipelineError> {
        if trade_retention.max_age_ms <= 0 || trade_retention.max_records_per_instrument == 0 {
            return Err(PipelineError::InvalidRequest(
                "OKX Trade retention policy must be positive".into(),
            ));
        }
        fs::create_dir_all(pipeline.0.root.join("instrument-master"))
            .map_err(|error| PipelineError::Storage(error.to_string()))?;
        let path = Self {
            pipeline,
            client,
            trade_retention,
            active_backfills: Arc::new(Mutex::new(HashMap::new())),
        };
        path.initialize_schema()?;
        Ok(path)
    }

    pub fn client(&self) -> &OkxClient {
        &self.client
    }

    pub fn begin_backfill(
        &self,
        task_id: &str,
        user_id: &str,
    ) -> Result<CancellationToken, PipelineError> {
        validate_user(user_id)?;
        if task_id.trim().is_empty() {
            return Err(PipelineError::InvalidRequest(
                "OKX backfill task ID must be non-empty".into(),
            ));
        }
        let token = CancellationToken::new();
        let mut active = self
            .active_backfills
            .lock()
            .map_err(|_| PipelineError::Storage("OKX backfill state lock failed".into()))?;
        if active.contains_key(task_id) {
            return Err(PipelineError::InvalidRequest(
                "OKX backfill task is already in progress".into(),
            ));
        }
        active.insert(task_id.into(), (user_id.into(), token.clone()));
        Ok(token)
    }

    pub fn cancel_backfill(&self, task_id: &str, user_id: &str) -> Result<(), PipelineError> {
        validate_user(user_id)?;
        if let Some((owner, token)) = self
            .active_backfills
            .lock()
            .map_err(|_| PipelineError::Storage("OKX backfill state lock failed".into()))?
            .get(task_id)
        {
            if owner != user_id {
                return Err(PipelineError::NotFound("OKX backfill task".into()));
            }
            token.cancel();
        }
        Ok(())
    }

    pub fn finish_backfill(&self, task_id: &str) -> Result<(), PipelineError> {
        self.active_backfills
            .lock()
            .map_err(|_| PipelineError::Storage("OKX backfill state lock failed".into()))?
            .remove(task_id);
        Ok(())
    }

    pub async fn acquire_instrument_master(
        &self,
        user_id: &str,
    ) -> Result<InstrumentMasterSnapshot, PipelineError> {
        validate_user(user_id)?;
        let acquisition = self
            .client
            .list_spot_instrument_master()
            .await
            .map_err(connector_error)?;
        self.record_instrument_master(user_id, acquisition)
    }

    pub fn record_instrument_master(
        &self,
        user_id: &str,
        acquisition: InstrumentMasterAcquisition,
    ) -> Result<InstrumentMasterSnapshot, PipelineError> {
        validate_user(user_id)?;
        self.persist_master_if_due(user_id, acquisition)
    }

    pub fn list_instrument_master_snapshots(
        &self,
        user_id: &str,
    ) -> Result<Vec<InstrumentMasterSnapshot>, PipelineError> {
        validate_user(user_id)?;
        let database = self.database()?;
        let mut statement = database
            .prepare(
                "SELECT s.snapshot_json
                 FROM okx_instrument_master_snapshots s
                 JOIN okx_instrument_master_access a USING(snapshot_id)
                 WHERE a.user_id = ?1
                 ORDER BY s.retrieved_at_ms, s.snapshot_id",
            )
            .map_err(storage)?;
        let rows = statement
            .query_map([user_id], |row| row.get::<_, String>(0))
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
        rows.into_iter()
            .map(|json| self.load_snapshot(&json))
            .collect()
    }

    pub fn point_in_time_universe(
        &self,
        user_id: &str,
        as_of_ms: i64,
    ) -> Result<PointInTimeInstrumentUniverse, PipelineError> {
        validate_user(user_id)?;
        if as_of_ms < 0 {
            return Err(PipelineError::InvalidRequest(
                "Universe observation time must be non-negative".into(),
            ));
        }
        let snapshot = self.latest_snapshot_before(user_id, as_of_ms)?;
        let Some(snapshot) = snapshot else {
            return Ok(PointInTimeInstrumentUniverse {
                universe_id: digest(&canonical_json_bytes(&(as_of_ms, "unknown"))?),
                as_of_ms,
                snapshot_id: None,
                evidence_state: UniverseEvidenceState::Unknown,
                instruments: Vec::new(),
            });
        };
        let instruments = snapshot
            .instruments
            .into_iter()
            .filter(|instrument| instrument.status == InstrumentStatus::Live)
            .collect::<Vec<_>>();
        let universe_id = digest(&canonical_json_bytes(&(
            as_of_ms,
            &snapshot.snapshot_id,
            &instruments,
        ))?);
        Ok(PointInTimeInstrumentUniverse {
            universe_id,
            as_of_ms,
            snapshot_id: Some(snapshot.snapshot_id),
            evidence_state: UniverseEvidenceState::Observed,
            instruments,
        })
    }

    pub fn acquisition_status(
        &self,
        user_id: &str,
        instrument: &InstrumentId,
        interval: BarInterval,
    ) -> Result<Option<OkxAcquisitionStatus>, PipelineError> {
        validate_user(user_id)?;
        let checkpoint = self.read_checkpoint(user_id, instrument, interval)?;
        Ok(checkpoint.map(|checkpoint| status_from_checkpoint(instrument, interval, checkpoint)))
    }

    pub fn acquisition_statuses(
        &self,
        user_id: &str,
    ) -> Result<Vec<OkxAcquisitionStatus>, PipelineError> {
        validate_user(user_id)?;
        let database = self.database()?;
        let mut statement = database
            .prepare(
                "SELECT instrument_json, interval, status_json FROM okx_backfill_checkpoints
                 WHERE user_id = ?1 ORDER BY instrument_code, interval",
            )
            .map_err(storage)?;
        let rows = statement
            .query_map([user_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
        rows.into_iter()
            .map(|(instrument_json, interval_json, status_json)| {
                let checkpoint: BackfillCheckpoint =
                    serde_json::from_str(&status_json).map_err(storage)?;
                let instrument: InstrumentId =
                    serde_json::from_str(&instrument_json).map_err(storage)?;
                let interval: BarInterval =
                    serde_json::from_str(&interval_json).map_err(storage)?;
                Ok(status_from_checkpoint(&instrument, interval, checkpoint))
            })
            .collect()
    }

    pub async fn backfill(
        &self,
        request: &OkxBackfillRequest,
        cancellation: CancellationToken,
        mut on_event: impl FnMut(OkxBackfillEvent),
    ) -> Result<Vec<PipelinePublication>, PipelineError> {
        validate_backfill_request(request)?;
        let universe = self.point_in_time_universe(&request.user_id, request.end_time_ms)?;
        let Some(snapshot_id) = universe.snapshot_id.clone() else {
            return Err(PipelineError::NotFound(
                "OKX Instrument Master evidence for the requested observation time".into(),
            ));
        };
        on_event(OkxBackfillEvent::UniverseLoaded {
            snapshot_id: snapshot_id.clone(),
            instrument_count: universe.instruments.len(),
        });

        let mut publications = Vec::new();
        for instrument in universe.instruments {
            if cancellation.is_cancelled() {
                break;
            }
            let instrument_id = InstrumentId::new(
                Venue::crypto_spot("okx")
                    .map_err(|error| PipelineError::InvalidRequest(error.to_string()))?,
                instrument.code.clone(),
            )
            .map_err(|error| PipelineError::InvalidRequest(error.to_string()))?;
            on_event(OkxBackfillEvent::InstrumentStarted {
                instrument: instrument_id.clone(),
            });
            match self
                .backfill_instrument(
                    request,
                    &snapshot_id,
                    instrument_id.clone(),
                    cancellation.clone(),
                    &mut on_event,
                )
                .await
            {
                Ok(Some(publication)) => {
                    on_event(OkxBackfillEvent::Published {
                        instrument: instrument_id.clone(),
                        source_id: publication.source.source_id.clone(),
                        canonical_id: publication
                            .canonical
                            .as_ref()
                            .map(|canonical| canonical.canonical_id.clone()),
                        revision: publication.source.revision,
                        state: publication.quality.state.clone(),
                    });
                    publications.push(publication);
                }
                Ok(None) => {}
                Err(error) => {
                    let mut checkpoint = self
                        .read_checkpoint(&request.user_id, &instrument_id, request.interval)?
                        .unwrap_or_default();
                    checkpoint.state = if cancellation.is_cancelled() {
                        OkxAcquisitionState::Cancelled
                    } else {
                        OkxAcquisitionState::Failed
                    };
                    checkpoint.last_error_code = match &error {
                        PipelineError::Connector { code, .. } => Some(code.clone()),
                        _ => None,
                    };
                    checkpoint.last_error = Some(error.to_string());
                    self.write_checkpoint(
                        &request.user_id,
                        &instrument_id,
                        request.interval,
                        &checkpoint,
                    )?;
                }
            }
        }
        Ok(publications)
    }

    pub async fn reconcile_closed_bar(
        &self,
        user_id: &str,
        websocket: &BarSnapshot,
    ) -> Result<PipelinePublication, PipelineError> {
        validate_user(user_id)?;
        if websocket.src != "okx" || !websocket.closed {
            return Err(PipelineError::InvalidRequest(
                "only confirmed OKX Closed Bars can be reconciled".into(),
            ));
        }
        let end_time_ms = next_bar_open_time_ms(websocket.bar.open_time_ms, websocket.interval)
            .map_err(|error| PipelineError::Connector {
                code: "invalid_timestamp".into(),
                message: error.to_string(),
            })?;
        let rest = self
            .client
            .get_bar_series_range_with_evidence(
                &websocket.code,
                websocket.interval,
                adaq_data_core::HistoricalBarRange {
                    start_time_ms: websocket.bar.open_time_ms,
                    end_time_ms,
                },
                |_, _| true,
            )
            .await
            .map_err(connector_error)?;
        let rest_bar = rest
            .series
            .bars
            .iter()
            .find(|bar| bar.open_time_ms == websocket.bar.open_time_ms)
            .cloned()
            .ok_or_else(|| PipelineError::NotFound("REST Closed Bar for reconciliation".into()))?;
        let rest_payload = rest
            .series
            .bars
            .iter()
            .position(|bar| bar.open_time_ms == websocket.bar.open_time_ms)
            .and_then(|index| rest.raw_payloads.get(index))
            .cloned()
            .unwrap_or_default();
        let instrument = InstrumentId::new(
            Venue::crypto_spot("okx")
                .map_err(|error| PipelineError::InvalidRequest(error.to_string()))?,
            websocket.code.clone(),
        )
        .map_err(|error| PipelineError::InvalidRequest(error.to_string()))?;
        let checkpoint = self
            .read_checkpoint(user_id, &instrument, websocket.interval)?
            .unwrap_or_default();
        let prior = checkpoint
            .source_id
            .as_deref()
            .map(|source_id| self.pipeline.source_for_user(user_id, source_id))
            .transpose()?;
        let mut records = prior
            .as_ref()
            .map(|source| source.records.clone())
            .unwrap_or_default();
        let mut record = SourceMarketRecord::from_bar(
            instrument.clone(),
            websocket.interval,
            websocket.code.clone(),
            &rest_bar,
        );
        record.raw_payload = json!({
            "rest": rest_payload,
            "websocket": websocket.bar,
            "websocketClosed": true,
        });
        replace_bar_record(&mut records, record);
        records.retain(|record| {
            record.open_time_ms
                >= prior
                    .as_ref()
                    .and_then(|source| {
                        source
                            .records
                            .iter()
                            .map(|record| record.open_time_ms)
                            .min()
                    })
                    .unwrap_or(websocket.bar.open_time_ms)
                && record.open_time_ms
                    < prior
                        .as_ref()
                        .and_then(|source| {
                            source
                                .records
                                .iter()
                                .map(|record| record.open_time_ms)
                                .max()
                        })
                        .and_then(|time| next_bar_open_time_ms(time, websocket.interval).ok())
                        .unwrap_or(end_time_ms)
                        .max(end_time_ms)
        });
        let range_start = records
            .iter()
            .map(|record| record.open_time_ms)
            .min()
            .unwrap_or(websocket.bar.open_time_ms);
        let range_end = records
            .iter()
            .map(|record| record.open_time_ms)
            .max()
            .and_then(|time| next_bar_open_time_ms(time, websocket.interval).ok())
            .unwrap_or(end_time_ms);
        let acquisition = SourceAcquisition {
            provider: "okx".into(),
            actual_upstream: Some("OKX public history-candles REST".into()),
            connector: adaq_data_core::OKX_CONNECTOR_VERSION.into(),
            connector_version: adaq_data_core::OKX_CONNECTOR_VERSION.into(),
            request_parameters: prior
                .as_ref()
                .map(|source| source.identity.request_parameters.clone())
                .unwrap_or(request_parameters(
                    &instrument,
                    websocket.interval,
                    self.latest_snapshot_before(user_id, i64::MAX)?
                        .as_ref()
                        .map(|snapshot| snapshot.snapshot_id.as_str()),
                )),
            retrieved_at_ms: rest.retrieved_at_ms,
            capability_snapshot: capability_snapshot(rest.retrieved_at_ms, None),
            acquisition_diagnostics: AcquisitionDiagnostics {
                request_count: rest.diagnostics.request_count,
                retry_count: rest.diagnostics.retry_count,
                response_statuses: rest.diagnostics.response_statuses,
                notes: vec![format!(
                    "REST/WebSocket reconciliation at {}: {}",
                    websocket.bar.open_time_ms,
                    if rest_bar == websocket.bar {
                        "identical"
                    } else {
                        "REST value selected and WebSocket evidence retained in Source payload"
                    }
                )],
            },
            records,
        };
        let mut canonicalization = CanonicalizationRequest::new(
            instrument.clone(),
            websocket.interval,
            CalendarEvidence::UtcGrid {
                calendar_id: "okx-utc-grid".into(),
                closures: Vec::new(),
            },
        )?;
        canonicalization.historical_range = Some(adaq_data_core::HistoricalBarRange {
            start_time_ms: range_start,
            end_time_ms: range_end,
        });
        let publication = self.pipeline.publish(
            user_id,
            acquisition,
            canonicalization,
            CancellationToken::new(),
            |_| {},
        )?;
        let mut checkpoint = checkpoint;
        checkpoint.state = if publication.quality.state == DataQualityState::Passed {
            OkxAcquisitionState::Completed
        } else {
            OkxAcquisitionState::Degraded
        };
        checkpoint.source_id = Some(publication.source.source_id.clone());
        checkpoint.revision = Some(publication.source.revision);
        checkpoint.gap_count = publication.quality.gap_count;
        checkpoint.last_error_code = None;
        checkpoint.last_error = None;
        self.write_checkpoint(user_id, &instrument, websocket.interval, &checkpoint)?;
        Ok(publication)
    }

    pub fn retain_trade(&self, user_id: &str, trade: &MarketTrade) -> Result<u64, PipelineError> {
        validate_user(user_id)?;
        if trade.src != "okx" || trade.code.trim().is_empty() || trade.trade_id.trim().is_empty() {
            return Err(PipelineError::InvalidRequest(
                "OKX Trade identity is invalid".into(),
            ));
        }
        let trade_json = serde_json::to_string(trade).map_err(storage)?;
        let database = self.database()?;
        database
            .execute(
                "INSERT OR REPLACE INTO okx_market_trades
                 (user_id, instrument_code, trade_id, timestamp_ms, trade_json)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    user_id,
                    trade.code,
                    trade.trade_id,
                    trade.timestamp_ms,
                    trade_json
                ],
            )
            .map_err(storage)?;
        drop(database);
        self.prune_trade_retention(user_id, now_ms())?;
        let database = self.database()?;
        database
            .query_row(
                "SELECT COUNT(*) FROM okx_market_trades
                 WHERE user_id = ?1 AND instrument_code = ?2",
                params![user_id, trade.code],
                |row| row.get::<_, i64>(0),
            )
            .map(|count| count.max(0) as u64)
            .map_err(storage)
    }

    fn prune_trade_retention(
        &self,
        user_id: &str,
        reference_time_ms: i64,
    ) -> Result<(), PipelineError> {
        let cutoff = reference_time_ms.saturating_sub(self.trade_retention.max_age_ms);
        let database = self.database()?;
        database
            .execute(
                "DELETE FROM okx_market_trades
                 WHERE user_id = ?1 AND timestamp_ms < ?2",
                params![user_id, cutoff],
            )
            .map_err(storage)?;
        let instruments = database
            .prepare(
                "SELECT DISTINCT instrument_code FROM okx_market_trades
                 WHERE user_id = ?1",
            )
            .map_err(storage)?
            .query_map([user_id], |row| row.get::<_, String>(0))
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
        for instrument_code in instruments {
            let stale_ids = database
                .prepare(
                    "SELECT trade_id FROM okx_market_trades
                     WHERE user_id = ?1 AND instrument_code = ?2
                     ORDER BY timestamp_ms DESC, trade_id DESC
                     LIMIT -1 OFFSET ?3",
                )
                .map_err(storage)?
                .query_map(
                    params![
                        user_id,
                        instrument_code,
                        i64::try_from(self.trade_retention.max_records_per_instrument)
                            .unwrap_or(i64::MAX)
                    ],
                    |row| row.get::<_, String>(0),
                )
                .map_err(storage)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(storage)?;
            for trade_id in stale_ids {
                database
                    .execute(
                        "DELETE FROM okx_market_trades
                         WHERE user_id = ?1 AND instrument_code = ?2 AND trade_id = ?3",
                        params![user_id, instrument_code, trade_id],
                    )
                    .map_err(storage)?;
            }
        }
        Ok(())
    }

    pub fn retained_trade_count(
        &self,
        user_id: &str,
        instrument_code: &str,
    ) -> Result<u64, PipelineError> {
        validate_user(user_id)?;
        self.prune_trade_retention(user_id, now_ms())?;
        let database = self.database()?;
        database
            .query_row(
                "SELECT COUNT(*) FROM okx_market_trades
                 WHERE user_id = ?1 AND instrument_code = ?2",
                params![user_id, instrument_code],
                |row| row.get::<_, i64>(0),
            )
            .map(|count| count.max(0) as u64)
            .map_err(storage)
    }

    pub async fn stream_tickers<F>(
        &self,
        user_id: &str,
        codes: &[String],
        mut on_event: F,
    ) -> Result<(), adaq_data_core::DataError>
    where
        F: FnMut(TickerStreamEvent) -> bool,
    {
        self.set_stream_health(user_id, "ticker", "connecting", None, None)
            .map_err(data_error)?;
        let stream_error = Arc::new(Mutex::new(None));
        let result = self
            .client
            .stream_tickers(codes, |event| {
                if let Err(error) = self.observe_stream_event(user_id, "ticker", &event) {
                    store_stream_error(&stream_error, error);
                    return false;
                }
                on_event(event)
            })
            .await;
        take_stream_error(&stream_error).map_or(result, |error| Err(data_error(error)))
    }

    pub async fn stream_trades<F>(
        &self,
        user_id: &str,
        codes: &[String],
        mut on_event: F,
    ) -> Result<(), adaq_data_core::DataError>
    where
        F: FnMut(TradeStreamEvent) -> bool,
    {
        self.set_stream_health(user_id, "trade", "connecting", None, None)
            .map_err(data_error)?;
        let stream_error = Arc::new(Mutex::new(None));
        let result = self
            .client
            .stream_trades(codes, |event| {
                if let TradeStreamEvent::Snapshot(trade) = &event {
                    if let Err(error) = self.retain_trade(user_id, trade) {
                        store_stream_error(&stream_error, error);
                        return false;
                    }
                }
                if let Err(error) = self.observe_stream_event(user_id, "trade", &event) {
                    store_stream_error(&stream_error, error);
                    return false;
                }
                on_event(event)
            })
            .await;
        take_stream_error(&stream_error).map_or(result, |error| {
            Err(adaq_data_core::DataError::new(
                "okx",
                "storage",
                error.to_string(),
            ))
        })
    }

    pub async fn stream_level2<F>(
        &self,
        user_id: &str,
        codes: &[String],
        mut on_event: F,
    ) -> Result<(), adaq_data_core::DataError>
    where
        F: FnMut(Level2StreamEvent) -> bool,
    {
        self.set_stream_health(user_id, "level2", "connecting", None, None)
            .map_err(data_error)?;
        let stream_error = Arc::new(Mutex::new(None));
        let result = self
            .client
            .stream_order_books(codes, |event| {
                if let Err(error) = self.observe_stream_event(user_id, "level2", &event) {
                    store_stream_error(&stream_error, error);
                    return false;
                }
                on_event(event)
            })
            .await;
        take_stream_error(&stream_error).map_or(result, |error| Err(data_error(error)))
    }

    pub fn stream_health(&self, user_id: &str) -> Result<Vec<OkxStreamHealth>, PipelineError> {
        validate_user(user_id)?;
        self.prune_trade_retention(user_id, now_ms())?;
        let database = self.database()?;
        let mut statement = database
            .prepare(
                "SELECT health_json FROM okx_stream_health
                 WHERE user_id = ?1 ORDER BY stream_kind",
            )
            .map_err(storage)?;
        statement
            .query_map([user_id], |row| row.get::<_, String>(0))
            .map_err(storage)?
            .map(|row| {
                row.map_err(storage)
                    .and_then(|json| serde_json::from_str(&json).map_err(storage))
            })
            .collect()
    }

    async fn backfill_instrument(
        &self,
        request: &OkxBackfillRequest,
        universe_snapshot_id: &str,
        instrument: InstrumentId,
        cancellation: CancellationToken,
        on_event: &mut impl FnMut(OkxBackfillEvent),
    ) -> Result<Option<PipelinePublication>, PipelineError> {
        let mut checkpoint = self
            .read_checkpoint(&request.user_id, &instrument, request.interval)?
            .unwrap_or_default();
        let prior = checkpoint
            .source_id
            .as_deref()
            .map(|source_id| self.pipeline.source_for_user(&request.user_id, source_id))
            .transpose()?;
        let checkpoint_matches_request = checkpoint.start_time_ms == Some(request.start_time_ms)
            && checkpoint.end_time_ms == Some(request.end_time_ms)
            && checkpoint.universe_snapshot_id.as_deref() == Some(universe_snapshot_id);
        if !checkpoint_matches_request
            && (checkpoint.start_time_ms.is_some()
                || checkpoint.end_time_ms.is_some()
                || checkpoint.universe_snapshot_id.is_some())
        {
            checkpoint = BackfillCheckpoint {
                source_id: checkpoint.source_id,
                revision: checkpoint.revision,
                ..BackfillCheckpoint::default()
            };
        }
        checkpoint.start_time_ms = Some(request.start_time_ms);
        checkpoint.end_time_ms = Some(request.end_time_ms);
        checkpoint.universe_snapshot_id = Some(universe_snapshot_id.to_owned());
        let previous_max = prior.as_ref().and_then(|source| {
            source
                .records
                .iter()
                .map(|record| record.open_time_ms)
                .max()
        });
        let fetch_start = checkpoint
            .next_cursor_ms
            .map(|_| request.start_time_ms)
            .unwrap_or_else(|| {
                previous_max
                    .map(|max| {
                        request
                            .start_time_ms
                            .max(max.saturating_sub(BAR_OVERLAP_COUNT * 60_000))
                    })
                    .unwrap_or(request.start_time_ms)
            });
        let fetch_end = checkpoint.next_cursor_ms.unwrap_or(request.end_time_ms);
        let publish_checkpoint = fetch_end <= fetch_start && !checkpoint.partial_records.is_empty();
        if fetch_end <= fetch_start && !publish_checkpoint {
            checkpoint.state = OkxAcquisitionState::Completed;
            checkpoint.next_cursor_ms = None;
            self.write_checkpoint(&request.user_id, &instrument, request.interval, &checkpoint)?;
            on_event(OkxBackfillEvent::InstrumentCompleted {
                instrument,
                state: checkpoint.state,
            });
            return Ok(None);
        }

        checkpoint.state = OkxAcquisitionState::Running;
        checkpoint.last_error_code = None;
        checkpoint.last_error = None;
        self.write_checkpoint(&request.user_id, &instrument, request.interval, &checkpoint)?;
        let mut fetched = publish_checkpoint
            .then(|| {
                checkpoint_acquisition(&instrument, request.interval, &checkpoint.partial_records)
            })
            .transpose()?;
        if fetched.is_none() {
            for gap_retry in 0..=request.max_gap_retries {
                let mut checkpoint_error = None;
                let result = self
                    .client
                    .get_bar_series_range_with_pages_and_payloads(
                        &instrument.code,
                        request.interval,
                        adaq_data_core::HistoricalBarRange {
                            start_time_ms: fetch_start,
                            end_time_ms: fetch_end,
                        },
                        |page, payloads, downloaded, oldest| {
                            for (bar, payload) in page.iter().zip(payloads) {
                                let mut record = SourceMarketRecord::from_bar(
                                    instrument.clone(),
                                    request.interval,
                                    instrument.code.clone(),
                                    bar,
                                );
                                record.raw_payload = payload.clone();
                                replace_bar_record(&mut checkpoint.partial_records, record);
                            }
                            checkpoint.pages += 1;
                            checkpoint.next_cursor_ms = Some(oldest);
                            checkpoint.latest_confirmed_open_time_ms = checkpoint
                                .partial_records
                                .iter()
                                .map(|record| record.open_time_ms)
                                .max();
                            if let Err(error) = self.write_checkpoint(
                                &request.user_id,
                                &instrument,
                                request.interval,
                                &checkpoint,
                            ) {
                                checkpoint_error = Some(error.to_string());
                                return false;
                            }
                            on_event(OkxBackfillEvent::Page {
                                instrument: instrument.clone(),
                                downloaded_records: downloaded,
                                next_cursor_ms: oldest,
                            });
                            !cancellation.is_cancelled()
                        },
                    )
                    .await;
                if let Some(error) = checkpoint_error {
                    return Err(PipelineError::Storage(error));
                }
                match result {
                    Ok(acquisition) => {
                        checkpoint.retry_count += acquisition.diagnostics.retry_count;
                        checkpoint.next_cursor_ms = None;
                        fetched = Some(acquisition);
                        if fetched
                            .as_ref()
                            .is_none_or(|value| value.series.gaps.is_empty())
                        {
                            break;
                        }
                        checkpoint.gap_count =
                            fetched.as_ref().map_or(0, |value| value.series.gaps.len());
                        checkpoint.retry_count += u32::from(gap_retry > 0);
                    }
                    Err(error) if error.code == "cancelled" => {
                        checkpoint.state = OkxAcquisitionState::Cancelled;
                        self.write_checkpoint(
                            &request.user_id,
                            &instrument,
                            request.interval,
                            &checkpoint,
                        )?;
                        on_event(OkxBackfillEvent::InstrumentCompleted {
                            instrument,
                            state: checkpoint.state,
                        });
                        return Ok(None);
                    }
                    Err(error) => {
                        checkpoint.state = OkxAcquisitionState::Failed;
                        checkpoint.last_error_code = Some(error.code.clone());
                        checkpoint.last_error = Some(error.to_string());
                        self.write_checkpoint(
                            &request.user_id,
                            &instrument,
                            request.interval,
                            &checkpoint,
                        )?;
                        return Err(connector_error(error));
                    }
                }
            }
        }

        let Some(acquisition) = fetched else {
            checkpoint.state = OkxAcquisitionState::Completed;
            self.write_checkpoint(&request.user_id, &instrument, request.interval, &checkpoint)?;
            return Ok(None);
        };
        let mut records = prior
            .as_ref()
            .map(|source| source.records.clone())
            .unwrap_or_default();
        for record in checkpoint.partial_records.clone() {
            replace_bar_record(&mut records, record);
        }
        for bar in &acquisition.series.bars {
            replace_bar_record(
                &mut records,
                SourceMarketRecord::from_bar(
                    instrument.clone(),
                    request.interval,
                    instrument.code.clone(),
                    bar,
                ),
            );
        }
        records.retain(|record| {
            record.open_time_ms >= request.start_time_ms
                && record.open_time_ms < request.end_time_ms
        });
        records.sort_by_key(|record| record.open_time_ms);
        if prior
            .as_ref()
            .is_some_and(|source| source.records == records)
            && acquisition.series.gaps.is_empty()
        {
            checkpoint.state = OkxAcquisitionState::Completed;
            checkpoint.next_cursor_ms = None;
            checkpoint.partial_records.clear();
            self.write_checkpoint(&request.user_id, &instrument, request.interval, &checkpoint)?;
            on_event(OkxBackfillEvent::InstrumentCompleted {
                instrument,
                state: checkpoint.state,
            });
            return Ok(None);
        }
        let retrieved_at_ms = acquisition.retrieved_at_ms;
        checkpoint.backoff_ms = acquisition.diagnostics.backoff_ms;
        let mut notes = acquisition
            .response_sha256s
            .iter()
            .map(|hash| format!("response-sha256:{hash}"))
            .collect::<Vec<_>>();
        if !acquisition.series.gaps.is_empty() {
            notes.push(format!(
                "{} explicit Bar Gaps remain after {} bounded retries",
                acquisition.series.gaps.len(),
                request.max_gap_retries
            ));
        }
        if acquisition.diagnostics.backoff_ms > 0 {
            notes.push(format!(
                "maximum REST backoff-ms:{}",
                acquisition.diagnostics.backoff_ms
            ));
        }
        let source_acquisition = SourceAcquisition {
            provider: "okx".into(),
            actual_upstream: Some("OKX public history-candles REST".into()),
            connector: adaq_data_core::OKX_CONNECTOR_VERSION.into(),
            connector_version: adaq_data_core::OKX_CONNECTOR_VERSION.into(),
            request_parameters: request_parameters(
                &instrument,
                request.interval,
                Some(universe_snapshot_id),
            ),
            retrieved_at_ms,
            capability_snapshot: capability_snapshot(
                retrieved_at_ms,
                (!acquisition.series.gaps.is_empty()).then_some("provider gap"),
            ),
            acquisition_diagnostics: AcquisitionDiagnostics {
                request_count: acquisition.diagnostics.request_count,
                retry_count: acquisition.diagnostics.retry_count,
                response_statuses: acquisition.diagnostics.response_statuses,
                notes,
            },
            records,
        };
        let mut canonicalization = CanonicalizationRequest::new(
            instrument.clone(),
            request.interval,
            CalendarEvidence::UtcGrid {
                calendar_id: "okx-utc-grid".into(),
                closures: Vec::new(),
            },
        )?;
        canonicalization.historical_range = Some(adaq_data_core::HistoricalBarRange {
            start_time_ms: request.start_time_ms,
            end_time_ms: request.end_time_ms,
        });
        let publication = self.pipeline.publish(
            &request.user_id,
            source_acquisition,
            canonicalization,
            cancellation,
            |_| {},
        )?;
        checkpoint.state = if publication.quality.state == DataQualityState::Passed {
            OkxAcquisitionState::Completed
        } else {
            OkxAcquisitionState::Degraded
        };
        checkpoint.next_cursor_ms = None;
        checkpoint.latest_confirmed_open_time_ms = publication
            .source
            .records
            .iter()
            .map(|record| record.open_time_ms)
            .max();
        checkpoint.coverage_start_ms = publication
            .source
            .records
            .iter()
            .map(|record| record.open_time_ms)
            .min();
        checkpoint.coverage_end_ms = checkpoint
            .latest_confirmed_open_time_ms
            .and_then(|time| next_bar_open_time_ms(time, request.interval).ok());
        checkpoint.gap_count = publication.quality.gap_count;
        checkpoint.revision = Some(publication.source.revision);
        checkpoint.source_id = Some(publication.source.source_id.clone());
        checkpoint.partial_records.clear();
        checkpoint.last_error_code = None;
        checkpoint.last_error = None;
        self.write_checkpoint(&request.user_id, &instrument, request.interval, &checkpoint)?;
        on_event(OkxBackfillEvent::InstrumentCompleted {
            instrument,
            state: checkpoint.state,
        });
        Ok(Some(publication))
    }

    fn persist_master_if_due(
        &self,
        user_id: &str,
        acquisition: InstrumentMasterAcquisition,
    ) -> Result<InstrumentMasterSnapshot, PipelineError> {
        let previous = self.latest_snapshot_before(user_id, i64::MAX)?;
        let should_persist = previous.as_ref().is_none_or(|previous| {
            previous.retrieved_at_ms.div_euclid(DAY_MS)
                != acquisition.retrieved_at_ms.div_euclid(DAY_MS)
                || previous.instruments != acquisition.instruments
        });
        if !should_persist {
            let snapshot = previous.expect("previous Instrument Master snapshot exists");
            self.grant_master_access(user_id, &snapshot.snapshot_id)?;
            return Ok(snapshot);
        }
        let snapshot_id = digest(&canonical_json_bytes(&(
            acquisition.retrieved_at_ms,
            &acquisition.response_sha256,
            &acquisition.connector_version,
            &acquisition.instruments,
        ))?);
        let evidence_path = self
            .pipeline
            .0
            .root
            .join("instrument-master")
            .join(format!("{snapshot_id}.json"));
        let mut snapshot = InstrumentMasterSnapshot {
            snapshot_id,
            retrieved_at_ms: acquisition.retrieved_at_ms,
            response_sha256: acquisition.response_sha256,
            connector_version: acquisition.connector_version,
            instruments: acquisition.instruments,
            content_sha256: String::new(),
            evidence_path,
        };
        let evidence_bytes = master_evidence_bytes(&snapshot)?;
        snapshot.content_sha256 = digest(&evidence_bytes);
        super::atomic_write(&snapshot.evidence_path, &evidence_bytes)?;
        let snapshot_json = serde_json::to_string(&snapshot).map_err(storage)?;
        let database = self.database()?;
        database
            .execute(
                "INSERT OR IGNORE INTO okx_instrument_master_snapshots
                 (snapshot_id, retrieved_at_ms, snapshot_json)
                 VALUES (?1, ?2, ?3)",
                params![
                    snapshot.snapshot_id,
                    snapshot.retrieved_at_ms,
                    snapshot_json
                ],
            )
            .map_err(storage)?;
        database
            .execute(
                "INSERT OR IGNORE INTO okx_instrument_master_access
                 (user_id, snapshot_id) VALUES (?1, ?2)",
                params![user_id, snapshot.snapshot_id],
            )
            .map_err(storage)?;
        Ok(snapshot)
    }

    fn grant_master_access(&self, user_id: &str, snapshot_id: &str) -> Result<(), PipelineError> {
        let database = self.database()?;
        database
            .execute(
                "INSERT OR IGNORE INTO okx_instrument_master_access
                 (user_id, snapshot_id) VALUES (?1, ?2)",
                params![user_id, snapshot_id],
            )
            .map_err(storage)?;
        Ok(())
    }

    fn latest_snapshot_before(
        &self,
        user_id: &str,
        as_of_ms: i64,
    ) -> Result<Option<InstrumentMasterSnapshot>, PipelineError> {
        let database = self.database()?;
        let json = database
            .query_row(
                "SELECT s.snapshot_json
                 FROM okx_instrument_master_snapshots s
                 JOIN okx_instrument_master_access a USING(snapshot_id)
                 WHERE a.user_id = ?1 AND s.retrieved_at_ms <= ?2
                 ORDER BY s.retrieved_at_ms DESC, s.snapshot_id DESC LIMIT 1",
                params![user_id, as_of_ms],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(storage)?;
        json.map(|json| self.load_snapshot(&json)).transpose()
    }

    fn load_snapshot(&self, json: &str) -> Result<InstrumentMasterSnapshot, PipelineError> {
        let snapshot: InstrumentMasterSnapshot = serde_json::from_str(json).map_err(storage)?;
        let bytes = fs::read(&snapshot.evidence_path).map_err(storage)?;
        if digest(&bytes) != snapshot.content_sha256 {
            return Err(PipelineError::Storage(
                "Instrument Master evidence hash does not match its catalog".into(),
            ));
        }
        Ok(snapshot)
    }

    fn read_checkpoint(
        &self,
        user_id: &str,
        instrument: &InstrumentId,
        interval: BarInterval,
    ) -> Result<Option<BackfillCheckpoint>, PipelineError> {
        let database = self.database()?;
        let interval_json = serde_json::to_string(&interval).map_err(storage)?;
        database
            .query_row(
                "SELECT status_json FROM okx_backfill_checkpoints
                 WHERE user_id = ?1 AND instrument_code = ?2 AND interval = ?3",
                params![user_id, instrument.code, interval_json],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(storage)?
            .map(|json| serde_json::from_str(&json).map_err(storage))
            .transpose()
    }

    fn write_checkpoint(
        &self,
        user_id: &str,
        instrument: &InstrumentId,
        interval: BarInterval,
        checkpoint: &BackfillCheckpoint,
    ) -> Result<(), PipelineError> {
        let interval_json = serde_json::to_string(&interval).map_err(storage)?;
        let status_json = serde_json::to_string(checkpoint).map_err(storage)?;
        let database = self.database()?;
        database
            .execute(
                "INSERT INTO okx_backfill_checkpoints
                 (user_id, instrument_code, interval, instrument_json, status_json)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(user_id, instrument_code, interval) DO UPDATE SET
                   instrument_json = excluded.instrument_json,
                   status_json = excluded.status_json",
                params![
                    user_id,
                    instrument.code,
                    interval_json,
                    serde_json::to_string(instrument).map_err(storage)?,
                    status_json
                ],
            )
            .map_err(storage)?;
        Ok(())
    }

    fn database(&self) -> Result<std::sync::MutexGuard<'_, Connection>, PipelineError> {
        self.pipeline
            .0
            .database
            .lock()
            .map_err(|_| PipelineError::Storage("OKX pipeline database lock failed".into()))
    }

    fn initialize_schema(&self) -> Result<(), PipelineError> {
        let database = self.database()?;
        database
            .execute_batch(
                "PRAGMA foreign_keys = ON;
                 CREATE TABLE IF NOT EXISTS okx_instrument_master_snapshots (
                    snapshot_id TEXT PRIMARY KEY,
                    retrieved_at_ms INTEGER NOT NULL,
                    snapshot_json TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS okx_instrument_master_access (
                    user_id TEXT NOT NULL,
                    snapshot_id TEXT NOT NULL,
                    PRIMARY KEY(user_id, snapshot_id),
                    FOREIGN KEY(snapshot_id) REFERENCES okx_instrument_master_snapshots(snapshot_id)
                 );
                 CREATE TABLE IF NOT EXISTS okx_backfill_checkpoints (
                    user_id TEXT NOT NULL,
                    instrument_code TEXT NOT NULL,
                    interval TEXT NOT NULL,
                    instrument_json TEXT NOT NULL,
                    status_json TEXT NOT NULL,
                    PRIMARY KEY(user_id, instrument_code, interval)
                 );
                 CREATE TABLE IF NOT EXISTS okx_market_trades (
                    user_id TEXT NOT NULL,
                    instrument_code TEXT NOT NULL,
                    trade_id TEXT NOT NULL,
                    timestamp_ms INTEGER NOT NULL,
                    trade_json TEXT NOT NULL,
                    PRIMARY KEY(user_id, instrument_code, trade_id)
                 );
                 CREATE TABLE IF NOT EXISTS okx_stream_health (
                    user_id TEXT NOT NULL,
                    stream_kind TEXT NOT NULL,
                    health_json TEXT NOT NULL,
                    PRIMARY KEY(user_id, stream_kind)
                 );",
            )
            .map_err(storage)
    }

    fn set_stream_health(
        &self,
        user_id: &str,
        stream_kind: &str,
        status: &str,
        error_code: Option<String>,
        error_message: Option<String>,
    ) -> Result<(), PipelineError> {
        validate_user(user_id)?;
        let current = self
            .stream_health(user_id)?
            .into_iter()
            .find(|health| health.stream_kind == stream_kind)
            .unwrap_or_else(|| OkxStreamHealth {
                stream_kind: stream_kind.into(),
                status: "unknown".into(),
                last_event_at_ms: None,
                reconnect_count: 0,
                error_code: None,
                error_message: None,
                retained_trade_count: 0,
                trade_retention_max_age_ms: self.trade_retention.max_age_ms,
                trade_retention_max_records: self.trade_retention.max_records_per_instrument,
            });
        let health = OkxStreamHealth {
            stream_kind: stream_kind.into(),
            status: status.into(),
            last_event_at_ms: if status == "connecting" {
                current.last_event_at_ms
            } else {
                Some(now_ms())
            },
            reconnect_count: current.reconnect_count + u32::from(status == "reconnecting"),
            error_code,
            error_message,
            retained_trade_count: if stream_kind == "trade" {
                let database = self.database()?;
                database
                    .query_row(
                        "SELECT COUNT(*) FROM okx_market_trades WHERE user_id = ?1",
                        [user_id],
                        |row| row.get::<_, i64>(0),
                    )
                    .map(|count| count.max(0) as u64)
                    .map_err(storage)?
            } else {
                current.retained_trade_count
            },
            trade_retention_max_age_ms: self.trade_retention.max_age_ms,
            trade_retention_max_records: self.trade_retention.max_records_per_instrument,
        };
        let health_json = serde_json::to_string(&health).map_err(storage)?;
        let database = self.database()?;
        database
            .execute(
                "INSERT INTO okx_stream_health(user_id, stream_kind, health_json)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(user_id, stream_kind) DO UPDATE SET health_json = excluded.health_json",
                params![user_id, stream_kind, health_json],
            )
            .map_err(storage)?;
        Ok(())
    }

    fn observe_stream_event<E: Serialize>(
        &self,
        user_id: &str,
        stream_kind: &str,
        event: &E,
    ) -> Result<(), PipelineError> {
        let value = serde_json::to_value(event).map_err(storage)?;
        let event_name = value.get("event").and_then(|value| value.as_str());
        if event_name == Some("snapshot") {
            return Ok(());
        }
        let data = value.get("data");
        let error = data
            .and_then(|data| data.get("code"))
            .and_then(|value| value.as_str());
        let message = data
            .and_then(|data| data.get("message"))
            .and_then(|value| value.as_str());
        let status = match event_name {
            Some("connected") => "live",
            Some("reconnecting") => "reconnecting",
            Some("error") => "degraded",
            Some("closed") => "closed",
            _ => "live",
        };
        self.set_stream_health(
            user_id,
            stream_kind,
            status,
            error.map(str::to_owned),
            message.map(str::to_owned),
        )
    }
}

fn store_stream_error(slot: &Arc<Mutex<Option<PipelineError>>>, error: PipelineError) {
    if let Ok(mut stored) = slot.lock() {
        *stored = Some(error);
    }
}

fn take_stream_error(slot: &Arc<Mutex<Option<PipelineError>>>) -> Option<PipelineError> {
    slot.lock().ok().and_then(|mut stored| stored.take())
}

fn validate_backfill_request(request: &OkxBackfillRequest) -> Result<(), PipelineError> {
    validate_user(&request.user_id)?;
    if request.task_id.trim().is_empty()
        || request.start_time_ms < 0
        || request.start_time_ms >= request.end_time_ms
        || request.interval != OKX_BAR_INTERVAL
    {
        return Err(PipelineError::InvalidRequest(
            "OKX backfill requires a non-empty task, increasing UTC range, and 1m interval".into(),
        ));
    }
    Ok(())
}

fn default_gap_retries() -> u8 {
    2
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(0)
}

fn storage(error: impl std::fmt::Display) -> PipelineError {
    PipelineError::Storage(error.to_string())
}

fn connector_error(error: adaq_data_core::DataError) -> PipelineError {
    PipelineError::Connector {
        code: error.code,
        message: error.message,
    }
}

fn data_error(error: PipelineError) -> adaq_data_core::DataError {
    adaq_data_core::DataError::new("okx", "storage", error.to_string())
}

fn status_from_checkpoint(
    instrument: &InstrumentId,
    interval: BarInterval,
    checkpoint: BackfillCheckpoint,
) -> OkxAcquisitionStatus {
    OkxAcquisitionStatus {
        instrument: instrument.clone(),
        interval,
        start_time_ms: checkpoint.start_time_ms,
        end_time_ms: checkpoint.end_time_ms,
        universe_snapshot_id: checkpoint.universe_snapshot_id,
        state: checkpoint.state,
        pages: checkpoint.pages,
        next_cursor_ms: checkpoint.next_cursor_ms,
        latest_confirmed_open_time_ms: checkpoint.latest_confirmed_open_time_ms,
        coverage_start_ms: checkpoint.coverage_start_ms,
        coverage_end_ms: checkpoint.coverage_end_ms,
        gap_count: checkpoint.gap_count,
        revision: checkpoint.revision,
        source_id: checkpoint.source_id,
        retry_count: checkpoint.retry_count,
        backoff_ms: checkpoint.backoff_ms,
        last_error_code: checkpoint.last_error_code,
        last_error: checkpoint.last_error,
    }
}

fn checkpoint_acquisition(
    instrument: &InstrumentId,
    interval: BarInterval,
    records: &[SourceMarketRecord],
) -> Result<BarAcquisition, PipelineError> {
    let decimal = |value: &Option<String>, field: &str| {
        value
            .as_deref()
            .ok_or_else(|| PipelineError::InvalidRequest(format!("checkpoint {field} is missing")))
            .and_then(|value| {
                Decimal::from_str_exact(value).map_err(|error| {
                    PipelineError::InvalidRequest(format!("checkpoint {field} is invalid: {error}"))
                })
            })
    };
    let bars = records
        .iter()
        .map(|record| {
            Ok(OhlcvBar {
                open_time_ms: record.open_time_ms,
                open: decimal(&record.open, "open")?,
                high: decimal(&record.high, "high")?,
                low: decimal(&record.low, "low")?,
                close: decimal(&record.close, "close")?,
                base_volume: decimal(&record.base_volume, "base volume")?,
                quote_volume: decimal(&record.quote_volume, "quote volume")?,
            })
        })
        .collect::<Result<Vec<_>, PipelineError>>()?;
    Ok(BarAcquisition {
        series: BarSeries {
            src: "okx".into(),
            code: instrument.code.clone(),
            interval,
            bars,
            gaps: Vec::new(),
        },
        retrieved_at_ms: now_ms(),
        response_sha256s: Vec::new(),
        diagnostics: Default::default(),
        raw_payloads: records
            .iter()
            .map(|record| record.raw_payload.clone())
            .collect(),
    })
}

fn master_evidence_bytes(snapshot: &InstrumentMasterSnapshot) -> Result<Vec<u8>, PipelineError> {
    canonical_json_bytes(&(
        &snapshot.snapshot_id,
        snapshot.retrieved_at_ms,
        &snapshot.response_sha256,
        &snapshot.connector_version,
        &snapshot.instruments,
    ))
}

fn request_parameters(
    instrument: &InstrumentId,
    interval: BarInterval,
    universe_snapshot_id: Option<&str>,
) -> serde_json::Value {
    json!({
        "kind": "okx-spot-closed-bars",
        "instrument": instrument,
        "interval": interval,
        "universeSnapshotId": universe_snapshot_id,
    })
}

fn capability_snapshot(
    retrieved_at_ms: i64,
    limitation: Option<&str>,
) -> ProviderCapabilitySnapshot {
    ProviderCapabilitySnapshot {
        provider: "okx".into(),
        captured_at_ms: retrieved_at_ms,
        venues: vec!["okx".into()],
        record_types: vec!["instrument-master".into(), "closed-bar@1m".into()],
        history_start_ms: None,
        delayed: false,
        delay_ms: None,
        rate_limit: Some("bounded REST retries with client rate gate".into()),
        streaming_symbol_limit: Some(adaq_data_core::OKX_MAX_STREAM_SYMBOLS as u32),
        limitations: limitation
            .map(|value| vec![value.into()])
            .unwrap_or_default(),
    }
}

fn replace_bar_record(records: &mut Vec<SourceMarketRecord>, record: SourceMarketRecord) {
    if let Some(existing) = records.iter_mut().find(|existing| {
        existing.instrument == record.instrument
            && existing.interval == record.interval
            && existing.open_time_ms == record.open_time_ms
    }) {
        if !record.raw_payload.is_null() || existing.raw_payload.is_null() {
            *existing = record;
        }
    } else {
        records.push(record);
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{Arc, Mutex, mpsc},
        thread,
    };

    use adaq_data_core::{
        BarInterval, BarSnapshot, InstrumentMasterAcquisition, InstrumentStatus, MarketTrade,
        MarketTradeSide, OhlcvBar, OkxClient, OkxRequestDiagnostics, SpotInstrument,
    };
    use rusqlite::Connection;
    use tempfile::tempdir;

    use super::*;

    fn spot(code: &str, status: InstrumentStatus) -> SpotInstrument {
        SpotInstrument {
            src: "okx".into(),
            code: code.into(),
            base_asset: code.split('-').next().unwrap_or(code).into(),
            quote_asset: "USDT".into(),
            status,
            listing_time_ms: Some(1),
            continuous_trading_time_ms: Some(1),
            price_increment: "0.1".parse().unwrap(),
            quantity_increment: "0.0001".parse().unwrap(),
            minimum_quantity: "0.0001".parse().unwrap(),
        }
    }

    fn master(retrieved_at_ms: i64, response_sha256: &str) -> InstrumentMasterAcquisition {
        InstrumentMasterAcquisition {
            retrieved_at_ms,
            response_sha256: response_sha256.into(),
            connector_version: adaq_data_core::OKX_CONNECTOR_VERSION.into(),
            diagnostics: OkxRequestDiagnostics::default(),
            instruments: vec![spot("BTC-USDT", InstrumentStatus::Live)],
        }
    }

    fn master_with_status(
        retrieved_at_ms: i64,
        response_sha256: &str,
        status: InstrumentStatus,
    ) -> InstrumentMasterAcquisition {
        let mut acquisition = master(retrieved_at_ms, response_sha256);
        acquisition.instruments[0].status = status;
        acquisition
    }

    fn bar_row(open_time_ms: i64, close: &str) -> String {
        serde_json::json!({
            "code": "0",
            "msg": "",
            "data": [[
                open_time_ms.to_string(), "1", "2", "0.5", close, "1", "1", "1", "1"
            ]]
        })
        .to_string()
    }

    fn bar_page(start_time_ms: i64, count: usize, close: &str) -> String {
        let data = (0..count)
            .map(|index| {
                serde_json::json!([
                    (start_time_ms + index as i64 * 60_000).to_string(),
                    "1",
                    "2",
                    "0.5",
                    close,
                    "1",
                    "1",
                    "1",
                    "1"
                ])
            })
            .collect::<Vec<_>>();
        serde_json::json!({"code": "0", "msg": "", "data": data}).to_string()
    }

    fn serving_json(bodies: Vec<String>) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            for body in bodies {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0; 4096];
                let size = stream.read(&mut request).unwrap();
                sender
                    .send(
                        String::from_utf8_lossy(&request[..size])
                            .lines()
                            .next()
                            .unwrap()
                            .into(),
                    )
                    .unwrap();
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .unwrap();
            }
        });
        (format!("http://{address}"), receiver)
    }

    fn data_path(
        bodies: Vec<String>,
    ) -> (tempfile::TempDir, OkxSpotDataPath, mpsc::Receiver<String>) {
        let root = tempdir().unwrap();
        let database = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let pipeline = DataPipeline::open(root.path().join("pipeline"), database).unwrap();
        let (base_url, requests) = serving_json(bodies);
        let client = OkxClient::new_with_policy(
            base_url,
            adaq_data_core::OkxRequestPolicy {
                max_attempts: 1,
                min_delay_ms: 0,
                retry_delay_ms: 0,
                max_retry_delay_ms: 0,
            },
        );
        (
            root,
            OkxSpotDataPath::open(pipeline, client).unwrap(),
            requests,
        )
    }

    #[test]
    fn instrument_master_snapshots_are_frozen_daily_and_universe_never_uses_current_listings() {
        let (_root, path, _requests) = data_path(Vec::new());
        let first = path
            .record_instrument_master("alice", master(1, "one"))
            .unwrap();
        let same_day = path
            .record_instrument_master("alice", master(2, "two"))
            .unwrap();
        assert_eq!(first.snapshot_id, same_day.snapshot_id);

        let status_changed = path
            .record_instrument_master(
                "alice",
                master_with_status(3, "status", InstrumentStatus::Suspended),
            )
            .unwrap();
        assert_ne!(first.snapshot_id, status_changed.snapshot_id);

        let next_day = path
            .record_instrument_master(
                "alice",
                master_with_status(DAY_MS + 1, "three", InstrumentStatus::Suspended),
            )
            .unwrap();
        assert_ne!(status_changed.snapshot_id, next_day.snapshot_id);
        assert_eq!(
            path.list_instrument_master_snapshots("alice")
                .unwrap()
                .len(),
            3
        );

        let historical = path.point_in_time_universe("alice", 2).unwrap();
        assert_eq!(historical.evidence_state, UniverseEvidenceState::Observed);
        assert_eq!(historical.instruments[0].code, "BTC-USDT");
        assert!(
            path.point_in_time_universe("alice", 4)
                .unwrap()
                .instruments
                .is_empty()
        );
        assert_eq!(
            path.point_in_time_universe("alice", 0)
                .unwrap()
                .evidence_state,
            UniverseEvidenceState::Unknown
        );
    }

    #[tokio::test]
    async fn backfill_resumes_from_page_checkpoint_and_publishes_append_only_revisions() {
        let (_root, path, requests) = data_path(vec![bar_row(0, "1.5"), bar_row(120_000, "2.5")]);
        path.record_instrument_master("alice", master(1, "master"))
            .unwrap();
        let cancellation = CancellationToken::new();
        let first = path
            .backfill(
                &OkxBackfillRequest {
                    task_id: "first".into(),
                    user_id: "alice".into(),
                    start_time_ms: 0,
                    end_time_ms: 120_000,
                    interval: BarInterval::OneMinute,
                    max_gap_retries: 0,
                },
                cancellation.clone(),
                |_| {},
            )
            .await
            .unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].source.revision, 1);
        assert!(first[0].source.records[0].raw_payload.is_array());

        let second = path
            .backfill(
                &OkxBackfillRequest {
                    task_id: "second".into(),
                    user_id: "alice".into(),
                    start_time_ms: 0,
                    end_time_ms: 180_000,
                    interval: BarInterval::OneMinute,
                    max_gap_retries: 0,
                },
                cancellation,
                |_| {},
            )
            .await
            .unwrap();
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].source.revision, 2);
        assert_eq!(second[0].source.records.len(), 2);
        assert_ne!(first[0].source.source_id, second[0].source.source_id);
        let status = path.acquisition_statuses("alice").unwrap();
        assert_eq!(status[0].revision, Some(2));
        assert_eq!(
            requests.recv().unwrap().split('?').next().unwrap(),
            "GET /api/v5/market/history-candles"
        );
        assert_eq!(
            requests.recv().unwrap().split('?').next().unwrap(),
            "GET /api/v5/market/history-candles"
        );
    }

    #[tokio::test]
    async fn backfill_restart_resumes_persisted_page_without_duplicate_records() {
        let root = tempdir().unwrap();
        let database_path = root.path().join("adaq.db");
        let (base_url, requests) =
            serving_json(vec![bar_page(60_000, 100, "1.5"), bar_row(0, "1.5")]);
        let policy = adaq_data_core::OkxRequestPolicy {
            max_attempts: 1,
            min_delay_ms: 0,
            retry_delay_ms: 0,
            max_retry_delay_ms: 0,
        };
        let pipeline = DataPipeline::open(
            root.path().join("pipeline"),
            Arc::new(Mutex::new(Connection::open(&database_path).unwrap())),
        )
        .unwrap();
        let path = OkxSpotDataPath::open(
            pipeline,
            OkxClient::new_with_policy(base_url.clone(), policy),
        )
        .unwrap();
        path.record_instrument_master("alice", master(1, "master"))
            .unwrap();
        let request = OkxBackfillRequest {
            task_id: "restartable".into(),
            user_id: "alice".into(),
            start_time_ms: 0,
            end_time_ms: 6_060_000,
            interval: BarInterval::OneMinute,
            max_gap_retries: 0,
        };
        let cancellation = CancellationToken::new();
        let cancellation_for_event = cancellation.clone();
        assert!(
            path.backfill(&request, cancellation, |event| {
                if matches!(event, OkxBackfillEvent::Page { .. }) {
                    cancellation_for_event.cancel();
                }
            })
            .await
            .unwrap()
            .is_empty()
        );
        assert_eq!(
            path.acquisition_statuses("alice").unwrap()[0].state,
            OkxAcquisitionState::Cancelled
        );
        drop(path);

        let pipeline = DataPipeline::open(
            root.path().join("pipeline"),
            Arc::new(Mutex::new(Connection::open(&database_path).unwrap())),
        )
        .unwrap();
        let path =
            OkxSpotDataPath::open(pipeline, OkxClient::new_with_policy(base_url, policy)).unwrap();
        let publications = path
            .backfill(&request, CancellationToken::new(), |_| {})
            .await
            .unwrap();
        assert_eq!(publications.len(), 1);
        assert_eq!(publications[0].source.records.len(), 101);
        assert!(
            publications[0]
                .source
                .records
                .windows(2)
                .all(|records| records[0].open_time_ms < records[1].open_time_ms)
        );
        assert_eq!(publications[0].source.revision, 1);
        assert!(requests.recv().is_ok());
        assert!(requests.recv().is_ok());
    }

    #[tokio::test]
    async fn backfill_status_retains_provider_error_codes() {
        let (_root, path, _requests) = data_path(vec![
            serde_json::json!({
                "code": "50011",
                "msg": "Too many requests",
                "data": []
            })
            .to_string(),
        ]);
        path.record_instrument_master("alice", master(1, "master"))
            .unwrap();
        let publications = path
            .backfill(
                &OkxBackfillRequest {
                    task_id: "rate-limited".into(),
                    user_id: "alice".into(),
                    start_time_ms: 0,
                    end_time_ms: 60_000,
                    interval: BarInterval::OneMinute,
                    max_gap_retries: 0,
                },
                CancellationToken::new(),
                |_| {},
            )
            .await
            .unwrap();
        assert!(publications.is_empty());
        let status = path.acquisition_statuses("alice").unwrap();
        assert_eq!(status[0].state, OkxAcquisitionState::Failed);
        assert_eq!(status[0].last_error_code.as_deref(), Some("50011"));
    }

    #[tokio::test]
    async fn websocket_reconciliation_publishes_new_revision_and_retains_prior_evidence() {
        let (_root, path, _requests) = data_path(vec![bar_row(0, "1.5"), bar_row(0, "2.5")]);
        path.record_instrument_master("alice", master(1, "master"))
            .unwrap();
        let first = path
            .backfill(
                &OkxBackfillRequest {
                    task_id: "first".into(),
                    user_id: "alice".into(),
                    start_time_ms: 0,
                    end_time_ms: 60_000,
                    interval: BarInterval::OneMinute,
                    max_gap_retries: 0,
                },
                CancellationToken::new(),
                |_| {},
            )
            .await
            .unwrap()
            .pop()
            .unwrap();
        let websocket = BarSnapshot {
            src: "okx".into(),
            code: "BTC-USDT".into(),
            interval: BarInterval::OneMinute,
            bar: OhlcvBar {
                open_time_ms: 0,
                open: "1".parse().unwrap(),
                high: "2".parse().unwrap(),
                low: "0.5".parse().unwrap(),
                close: "2".parse().unwrap(),
                base_volume: "1".parse().unwrap(),
                quote_volume: "1".parse().unwrap(),
            },
            closed: true,
        };
        let second = path
            .reconcile_closed_bar("alice", &websocket)
            .await
            .unwrap();
        assert_eq!(second.source.revision, 2);
        assert_ne!(first.source.source_id, second.source.source_id);
        assert_eq!(second.source.records[0].open.as_deref(), Some("1"));
        assert_eq!(second.source.records[0].close.as_deref(), Some("2.5"));
        assert!(
            second.source.records[0]
                .raw_payload
                .get("websocket")
                .is_some()
        );
        assert!(
            path.pipeline
                .source_for_user("alice", &first.source.source_id)
                .is_ok()
        );
    }

    #[test]
    fn trade_retention_is_bounded_and_level_two_has_no_persistent_store() {
        let (_root, path, _requests) = data_path(Vec::new());
        let path = OkxSpotDataPath::open_with_trade_retention(
            path.pipeline.clone(),
            path.client.clone(),
            OkxTradeRetentionPolicy {
                max_age_ms: 10,
                max_records_per_instrument: 1,
            },
        )
        .unwrap();
        let trade = |id: &str, timestamp_ms| MarketTrade {
            src: "okx".into(),
            code: "BTC-USDT".into(),
            trade_id: id.into(),
            price: "1".parse().unwrap(),
            quantity: "0.1".parse().unwrap(),
            side: MarketTradeSide::Buy,
            timestamp_ms,
        };
        let now = now_ms();
        path.retain_trade("alice", &trade("old", now.saturating_sub(20)))
            .unwrap();
        path.retain_trade("alice", &trade("new", now)).unwrap();
        assert_eq!(path.retained_trade_count("alice", "BTC-USDT").unwrap(), 1);
        assert!(path.stream_health("alice").unwrap().is_empty());
    }
}
