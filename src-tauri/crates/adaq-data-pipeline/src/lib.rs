//! Tauri-independent Source -> Canonical market-data publication.
//!
//! The pipeline deliberately performs only lossless canonicalization. Provider
//! records are retained before any quality decision, canonical rows are written
//! to immutable Parquet, and SQLite stores only the catalog, access, and
//! lifecycle metadata needed to find that evidence.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fmt::Write as _,
    fs::{self, File},
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    str::FromStr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use adaq_data_core::{
    BarGap, BarInterval, BarSeries, HistoricalBarRange, OhlcvBar,
    market::{
        BarIdentity, CalendarError, InstrumentId, PriceBasis, ScheduledClosure, SessionPhase,
        TradingCalendarSnapshot, VenueKind,
    },
    next_bar_open_time_ms,
};
use arrow_array::{Array, Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use chrono::{Datelike, Timelike};
use parquet::arrow::{ArrowWriter, arrow_reader::ParquetRecordBatchReaderBuilder};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

pub mod a_share;
pub mod okx;
pub mod us_equity;

pub const NORMALIZATION_CONTRACT_VERSION: &str = "lossless-v1";

const APPLIED_RULES: [&str; 6] = [
    "identity-mapping",
    "utc-and-interval-validation",
    "exact-decimal-parsing",
    "deterministic-ordering",
    "identical-duplicate-collapse",
    "financial-invariant-validation",
];

#[derive(Debug, Error)]
pub enum PipelineError {
    #[error("invalid pipeline request: {0}")]
    InvalidRequest(String),
    #[error("pipeline storage failed: {0}")]
    Storage(String),
    #[error("pipeline evidence was not found: {0}")]
    NotFound(String),
    #[error("pipeline attempt was cancelled after Source revision {source_id}")]
    Cancelled { source_id: String },
    #[error("pipeline publication failed after Source revision {source_id}: {message}")]
    PublicationFailed { source_id: String, message: String },
    #[error("market-data acquisition failed [{code}]: {message}")]
    Connector { code: String, message: String },
    #[error("{evidence_kind} evidence {evidence_id} is deletion-locked")]
    DeletionBlocked {
        evidence_kind: String,
        evidence_id: String,
        blockers: Vec<BlockingReference>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockingReference {
    pub consumer_kind: String,
    pub consumer_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCapabilitySnapshot {
    pub provider: String,
    pub captured_at_ms: i64,
    #[serde(default)]
    pub subscription_plan: Option<String>,
    #[serde(default)]
    pub feed: Option<String>,
    #[serde(default)]
    pub coverage: Option<String>,
    #[serde(default)]
    pub realtime: Option<bool>,
    #[serde(default)]
    pub venues: Vec<String>,
    #[serde(default)]
    pub record_types: Vec<String>,
    pub history_start_ms: Option<i64>,
    #[serde(default)]
    pub history_end_ms: Option<i64>,
    pub delayed: bool,
    #[serde(default)]
    pub delayed_known: bool,
    pub delay_ms: Option<u64>,
    pub rate_limit: Option<String>,
    #[serde(default)]
    pub rate_limit_known: bool,
    #[serde(default)]
    pub requests_per_minute: Option<u32>,
    #[serde(default)]
    pub stream_connection_limit: Option<u32>,
    pub streaming_symbol_limit: Option<u32>,
    #[serde(default)]
    pub unavailable_capabilities: Vec<String>,
    #[serde(default)]
    pub limitations: Vec<String>,
}

impl Default for ProviderCapabilitySnapshot {
    fn default() -> Self {
        Self {
            provider: "unknown".into(),
            captured_at_ms: 0,
            subscription_plan: None,
            feed: None,
            coverage: None,
            realtime: None,
            venues: Vec::new(),
            record_types: Vec::new(),
            history_start_ms: None,
            history_end_ms: None,
            delayed: false,
            delayed_known: false,
            delay_ms: None,
            rate_limit: None,
            rate_limit_known: false,
            requests_per_minute: None,
            stream_connection_limit: None,
            streaming_symbol_limit: None,
            unavailable_capabilities: Vec::new(),
            limitations: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AcquisitionDiagnostics {
    pub request_count: u32,
    pub retry_count: u32,
    #[serde(default)]
    pub response_statuses: Vec<u16>,
    #[serde(default)]
    pub notes: Vec<String>,
}

/// A provider-delivered record. Decimal fields remain strings at this trust
/// boundary so the Source file retains the exact spelling delivered upstream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceMarketRecord {
    pub provider_symbol: String,
    pub instrument: InstrumentId,
    pub interval: BarInterval,
    pub open_time_ms: i64,
    pub open: Option<String>,
    pub high: Option<String>,
    pub low: Option<String>,
    pub close: Option<String>,
    pub base_volume: Option<String>,
    pub quote_volume: Option<String>,
    #[serde(default)]
    pub raw_payload: Value,
}

impl SourceMarketRecord {
    pub fn from_bar(
        instrument: InstrumentId,
        interval: BarInterval,
        provider_symbol: impl Into<String>,
        bar: &OhlcvBar,
    ) -> Self {
        Self {
            provider_symbol: provider_symbol.into(),
            instrument,
            interval,
            open_time_ms: bar.open_time_ms,
            open: Some(bar.open.to_string()),
            high: Some(bar.high.to_string()),
            low: Some(bar.low.to_string()),
            close: Some(bar.close.to_string()),
            base_volume: Some(bar.base_volume.to_string()),
            quote_volume: Some(bar.quote_volume.to_string()),
            raw_payload: Value::Null,
        }
    }

    pub fn fingerprint(&self) -> String {
        digest(&canonical_json_bytes(self).expect("Source Market Record serializes"))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceAcquisition {
    pub provider: String,
    pub actual_upstream: Option<String>,
    pub connector: String,
    pub connector_version: String,
    pub request_parameters: Value,
    pub retrieved_at_ms: i64,
    #[serde(default)]
    pub response_sha256s: Vec<String>,
    #[serde(default)]
    pub acquisition_content_sha256: Option<String>,
    pub capability_snapshot: ProviderCapabilitySnapshot,
    pub acquisition_diagnostics: AcquisitionDiagnostics,
    #[serde(default)]
    pub price_basis: PriceBasis,
    pub records: Vec<SourceMarketRecord>,
}

impl Default for SourceAcquisition {
    fn default() -> Self {
        Self {
            provider: "fixture".into(),
            actual_upstream: None,
            connector: "fixture".into(),
            connector_version: "0.0.0".into(),
            request_parameters: Value::Object(Default::default()),
            retrieved_at_ms: 1,
            response_sha256s: Vec::new(),
            acquisition_content_sha256: None,
            capability_snapshot: ProviderCapabilitySnapshot::default(),
            acquisition_diagnostics: AcquisitionDiagnostics::default(),
            price_basis: PriceBasis::Unadjusted,
            records: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceIdentity {
    pub provider: String,
    pub actual_upstream: Option<String>,
    pub connector: String,
    pub connector_version: String,
    pub request_parameters: Value,
    pub retrieved_at_ms: i64,
    #[serde(default)]
    pub response_sha256s: Vec<String>,
    #[serde(default)]
    pub acquisition_content_sha256: Option<String>,
    pub payload_sha256: String,
    pub content_sha256: String,
    pub capability_snapshot: ProviderCapabilitySnapshot,
    pub acquisition_diagnostics: AcquisitionDiagnostics,
    #[serde(default)]
    pub price_basis: PriceBasis,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceMarketDataset {
    pub source_id: String,
    pub revision: u64,
    pub logical_key: String,
    pub identity: SourceIdentity,
    pub records: Vec<SourceMarketRecord>,
    pub evidence_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CalendarEvidence {
    UtcGrid {
        calendar_id: String,
        #[serde(default)]
        closures: Vec<ScheduledClosure>,
    },
    Venue {
        snapshot: TradingCalendarSnapshot,
    },
}

impl CalendarEvidence {
    fn validate_for(&self, instrument: &InstrumentId) -> Result<(), PipelineError> {
        match (instrument.venue.kind, self) {
            (VenueKind::CryptoSpot, Self::UtcGrid { calendar_id, .. })
                if !calendar_id.trim().is_empty() =>
            {
                Ok(())
            }
            (VenueKind::CryptoSpot, Self::Venue { .. }) => Err(PipelineError::InvalidRequest(
                "Crypto Spot canonicalization requires UTC-grid calendar evidence".into(),
            )),
            (VenueKind::ChinaAShareEquity | VenueKind::UsEquity, Self::Venue { snapshot })
                if snapshot.venue == instrument.venue =>
            {
                Ok(())
            }
            (VenueKind::ChinaAShareEquity | VenueKind::UsEquity, Self::Venue { .. }) => {
                Err(PipelineError::InvalidRequest(
                    "calendar Venue must match the canonical Instrument Venue".into(),
                ))
            }
            (_, Self::UtcGrid { .. }) => Err(PipelineError::InvalidRequest(
                "session-based equity canonicalization requires Venue calendar evidence".into(),
            )),
        }
    }

    fn is_expected_bar_time(
        &self,
        instrument: &InstrumentId,
        interval: BarInterval,
        open_time_ms: i64,
    ) -> Result<(), QuarantineReason> {
        match self {
            Self::UtcGrid { closures, .. } => {
                let aligned = interval_aligned(open_time_ms, interval);
                let closed = closures.iter().any(|closure| {
                    open_time_ms >= closure.start_ms && open_time_ms < closure.end_ms
                });
                if !aligned || closed {
                    Err(QuarantineReason::MisalignedTime {
                        details: if closed {
                            "UTC grid point is inside recorded scheduled closure".into()
                        } else {
                            "UTC instant is not aligned to the declared Bar Interval".into()
                        },
                    })
                } else {
                    Ok(())
                }
            }
            Self::Venue { snapshot } => {
                if snapshot.venue != instrument.venue {
                    return Err(QuarantineReason::UnsupportedIdentity {
                        details: "calendar Venue differs from Instrument Venue".into(),
                    });
                }
                validate_venue_bar_time(snapshot, interval, open_time_ms).map_err(|error| {
                    QuarantineReason::MisalignedTime {
                        details: error.to_string(),
                    }
                })
            }
        }
    }

    fn scheduled_non_trading(&self, open_time_ms: i64) -> Result<bool, PipelineError> {
        match self {
            Self::UtcGrid { closures, .. } => Ok(closures
                .iter()
                .any(|closure| open_time_ms >= closure.start_ms && open_time_ms < closure.end_ms)),
            Self::Venue { snapshot } => snapshot
                .is_scheduled_non_trading(open_time_ms)
                .map_err(|error| PipelineError::InvalidRequest(error.to_string())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalizationRequest {
    pub instrument: InstrumentId,
    pub interval: BarInterval,
    pub normalization_contract: String,
    pub calendar: CalendarEvidence,
    pub historical_range: Option<HistoricalBarRange>,
    #[serde(default)]
    pub price_basis: PriceBasis,
}

impl CanonicalizationRequest {
    pub fn new(
        instrument: InstrumentId,
        interval: BarInterval,
        calendar: CalendarEvidence,
    ) -> Result<Self, PipelineError> {
        let request = Self {
            instrument,
            interval,
            normalization_contract: NORMALIZATION_CONTRACT_VERSION.into(),
            calendar,
            historical_range: None,
            price_basis: PriceBasis::Unadjusted,
        };
        request.validate()?;
        Ok(request)
    }

    fn validate(&self) -> Result<(), PipelineError> {
        if self.normalization_contract.trim().is_empty() {
            return Err(PipelineError::InvalidRequest(
                "normalization contract must be non-empty".into(),
            ));
        }
        if let Some(range) = self.historical_range
            && range.start_time_ms >= range.end_time_ms
        {
            return Err(PipelineError::InvalidRequest(
                "historical range must be increasing".into(),
            ));
        }
        if matches!(
            self.instrument.venue.kind,
            VenueKind::ChinaAShareEquity | VenueKind::UsEquity
        ) && self.price_basis != PriceBasis::Unadjusted
        {
            return Err(PipelineError::InvalidRequest(
                "Canonical equity Bars require Unadjusted Price Basis".into(),
            ));
        }
        if self.instrument.venue.kind == VenueKind::ChinaAShareEquity
            && matches!(
                self.interval,
                BarInterval::OneMonth | BarInterval::ThreeMonths
            )
        {
            return Err(PipelineError::InvalidRequest(
                "A-share monthly Bar intervals are not supported by the pinned connector".into(),
            ));
        }
        self.calendar.validate_for(&self.instrument)
    }
}

impl Default for CanonicalizationRequest {
    fn default() -> Self {
        let instrument = InstrumentId::new(
            adaq_data_core::market::Venue::crypto_spot("okx").expect("valid default Venue"),
            "BTC-USDT",
        )
        .expect("valid default Instrument");
        Self {
            instrument,
            interval: BarInterval::OneMinute,
            normalization_contract: NORMALIZATION_CONTRACT_VERSION.into(),
            calendar: CalendarEvidence::UtcGrid {
                calendar_id: "utc-grid".into(),
                closures: Vec::new(),
            },
            historical_range: None,
            price_basis: PriceBasis::Unadjusted,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalRowEvidence {
    pub identity: BarIdentity,
    pub source_record_hashes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum QuarantineReason {
    MissingRequiredField { field: String },
    UnparsableExactValue { field: String },
    InvalidFinancialInvariant { details: String },
    MisalignedTime { details: String },
    ConflictingIdentityOrValue { details: String },
    UnsupportedIdentity { details: String },
    InvalidUtcInstant,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuarantinedMarketRecord {
    pub source_record_hash: String,
    pub record: SourceMarketRecord,
    pub reason: QuarantineReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WarningReason {
    ZeroVolume,
    WidePriceRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalWarning {
    pub source_record_hash: String,
    pub reason: WarningReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DataQualityState {
    Passed,
    Degraded,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Coverage {
    pub start_time_ms: Option<i64>,
    pub end_time_ms: Option<i64>,
    pub expected_record_count: usize,
    pub canonical_record_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum QualityReason {
    DuplicateCollapsed { count: usize },
    ConflictingValues { count: usize },
    QuarantinedRecords { count: usize },
    ExplicitGaps { count: usize },
    WarningRecords { count: usize },
    CapabilityLimitation { detail: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataQualityReport {
    pub report_id: String,
    pub source_id: String,
    pub canonical_id: Option<String>,
    pub state: DataQualityState,
    pub applied_rules: Vec<String>,
    pub coverage: Coverage,
    pub duplicate_count: usize,
    pub conflict_count: usize,
    pub quarantine_count: usize,
    pub gap_count: usize,
    pub warning_count: usize,
    pub capability_limitations: Vec<String>,
    pub reasons: Vec<QualityReason>,
    pub quarantined_records: Vec<QuarantinedMarketRecord>,
    pub warnings: Vec<CanonicalWarning>,
    pub gaps: Vec<BarGap>,
    pub evidence_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalMarketDataset {
    pub canonical_id: String,
    pub source_id: String,
    pub revision: u64,
    pub instrument: InstrumentId,
    pub interval: BarInterval,
    pub normalization_contract: String,
    pub calendar: CalendarEvidence,
    #[serde(default)]
    pub price_basis: PriceBasis,
    pub bars: Vec<OhlcvBar>,
    pub row_evidence: Vec<CanonicalRowEvidence>,
    pub gaps: Vec<BarGap>,
    pub quality_report_id: String,
    pub content_sha256: String,
    pub parquet_path: PathBuf,
}

impl CanonicalMarketDataset {
    /// Adapts the Canonical rows to the existing immutable Snapshot contract.
    /// Existing Backtest and Model consumers therefore keep one Snapshot path.
    pub fn to_bar_series(&self) -> BarSeries {
        BarSeries {
            src: self.instrument.venue.id.clone(),
            code: self.instrument.code.clone(),
            interval: self.interval,
            bars: self.bars.clone(),
            gaps: self.gaps.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelinePublication {
    pub attempt_id: Option<String>,
    pub source: SourceMarketDataset,
    pub canonical: Option<CanonicalMarketDataset>,
    pub quality: DataQualityReport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineDatasetSummary {
    pub source_id: String,
    pub canonical_id: Option<String>,
    pub revision: u64,
    pub state: DataQualityState,
    pub source_record_count: usize,
    pub canonical_record_count: usize,
    pub quarantined_record_count: usize,
    pub gap_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "event", content = "data")]
pub enum PipelineProgress {
    Started {
        attempt_id: String,
    },
    SourcePersisted {
        source_id: String,
        revision: u64,
    },
    Canonicalized {
        source_id: String,
        canonical_id: Option<String>,
        state: DataQualityState,
    },
    Published {
        source_id: String,
        canonical_id: String,
        report_id: String,
    },
    Cancelled {
        source_id: String,
    },
    Failed {
        source_id: String,
        stage: String,
    },
    Completed {
        source_id: String,
        canonical_id: Option<String>,
        report_id: String,
        state: DataQualityState,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineFailure {
    pub attempt_id: Option<String>,
    pub source_id: String,
    pub stage: String,
    pub message: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    pub(crate) fn is_same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Default)]
struct PublicationArtifacts {
    paths: Vec<(PathBuf, bool)>,
    committed: bool,
}

impl PublicationArtifacts {
    fn track(&mut self, path: &Path) {
        self.paths.push((path.to_owned(), path.is_file()));
    }

    fn commit_on_catalog_cutover(&mut self) {
        self.committed = true;
    }
}

impl Drop for PublicationArtifacts {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        for (path, existed) in &self.paths {
            if !existed && path.is_file() {
                let _ = fs::remove_file(path);
            }
        }
    }
}

struct SourceAccessGuard {
    pipeline: DataPipeline,
    user_id: String,
    source_id: String,
    keep_access: bool,
}

impl SourceAccessGuard {
    fn commit(&mut self) {
        self.keep_access = true;
    }
}

impl Drop for SourceAccessGuard {
    fn drop(&mut self) {
        if self.keep_access {
            let _ = self
                .pipeline
                .grant_source_for_user(&self.user_id, &self.source_id);
        } else {
            let _ = self
                .pipeline
                .delete_source_for_user(&self.user_id, &self.source_id);
        }
    }
}

pub struct UserOperationGuard {
    pipeline: DataPipeline,
    user_id: String,
    operation_id: String,
}

impl Drop for UserOperationGuard {
    fn drop(&mut self) {
        let mut became_idle = false;
        if let Ok(mut active) = self.pipeline.0.active_users.lock() {
            if let Some(operations) = active.get_mut(&self.user_id) {
                operations.remove(&self.operation_id);
                if operations.is_empty() {
                    active.remove(&self.user_id);
                    became_idle = true;
                }
            }
        }
        if became_idle {
            let _ = self.pipeline.release_timed_out_reset_if_idle(&self.user_id);
        }
    }
}

struct PipelineInner {
    root: PathBuf,
    database: Arc<Mutex<Connection>>,
    active: Mutex<HashMap<String, CancellationToken>>,
    attempt_users: Mutex<HashMap<String, String>>,
    active_users: Mutex<HashMap<String, HashMap<String, CancellationToken>>>,
    resetting: Mutex<HashSet<String>>,
    timed_out_resets: Mutex<HashSet<String>>,
    next_operation: std::sync::atomic::AtomicU64,
}

#[derive(Clone)]
pub struct DataPipeline(Arc<PipelineInner>);

impl DataPipeline {
    pub fn open(
        root: impl Into<PathBuf>,
        database: Arc<Mutex<Connection>>,
    ) -> Result<Self, PipelineError> {
        let root = root.into();
        for directory in ["sources", "canonical", "quality"] {
            fs::create_dir_all(root.join(directory)).map_err(storage)?;
        }
        let pipeline = Self(Arc::new(PipelineInner {
            root,
            database,
            active: Mutex::new(HashMap::new()),
            attempt_users: Mutex::new(HashMap::new()),
            active_users: Mutex::new(HashMap::new()),
            resetting: Mutex::new(HashSet::new()),
            timed_out_resets: Mutex::new(HashSet::new()),
            next_operation: std::sync::atomic::AtomicU64::new(1),
        }));
        pipeline.initialize_schema()?;
        Ok(pipeline)
    }

    pub fn open_with_connection(
        root: impl Into<PathBuf>,
        connection: Connection,
    ) -> Result<Self, PipelineError> {
        Self::open(root, Arc::new(Mutex::new(connection)))
    }

    pub(crate) fn root_dir(&self) -> &Path {
        &self.0.root
    }

    pub(crate) fn database(&self) -> Arc<Mutex<Connection>> {
        self.0.database.clone()
    }

    pub fn begin_user_reset(&self, user_id: &str) -> Result<(), PipelineError> {
        validate_user(user_id)?;
        let mut resetting = self.0.resetting.lock().map_err(lock_error)?;
        if !resetting.insert(user_id.to_owned()) {
            return Err(PipelineError::InvalidRequest(
                "pipeline reset is already in progress for this user".into(),
            ));
        }
        if let Ok(active) = self.0.active_users.lock() {
            if let Some(operations) = active.get(user_id) {
                for token in operations.values() {
                    token.cancel();
                }
            }
        }
        drop(resetting);
        let deadline = Instant::now() + Duration::from_secs(35);
        loop {
            let empty = self
                .0
                .active_users
                .lock()
                .map_err(lock_error)?
                .get(user_id)
                .is_none_or(HashMap::is_empty);
            if empty {
                return Ok(());
            }
            if Instant::now() >= deadline {
                let still_active = self
                    .0
                    .active_users
                    .lock()
                    .map_err(lock_error)?
                    .get(user_id)
                    .is_some_and(|operations| !operations.is_empty());
                if still_active {
                    self.0
                        .timed_out_resets
                        .lock()
                        .map_err(lock_error)?
                        .insert(user_id.to_owned());
                } else {
                    self.0.resetting.lock().map_err(lock_error)?.remove(user_id);
                }
                return Err(PipelineError::Storage(
                    "timed out waiting for pipeline operations during reset".into(),
                ));
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    pub fn finish_user_reset(&self, user_id: &str) -> Result<(), PipelineError> {
        validate_user(user_id)?;
        let active = self
            .0
            .active_users
            .lock()
            .map_err(lock_error)?
            .get(user_id)
            .is_some_and(|operations| !operations.is_empty());
        if active {
            return Err(PipelineError::Storage(
                "cannot finish pipeline reset while user operations are active".into(),
            ));
        }
        self.0.resetting.lock().map_err(lock_error)?.remove(user_id);
        self.0
            .timed_out_resets
            .lock()
            .map_err(lock_error)?
            .remove(user_id);
        Ok(())
    }

    fn release_timed_out_reset_if_idle(&self, user_id: &str) -> Result<(), PipelineError> {
        let idle = self
            .0
            .active_users
            .lock()
            .map_err(lock_error)?
            .get(user_id)
            .is_none_or(HashMap::is_empty);
        if !idle {
            return Ok(());
        }
        let timed_out = self
            .0
            .timed_out_resets
            .lock()
            .map_err(lock_error)?
            .remove(user_id);
        if timed_out {
            self.0.resetting.lock().map_err(lock_error)?.remove(user_id);
        }
        Ok(())
    }

    pub fn begin_user_operation(
        &self,
        user_id: &str,
        operation_id: String,
        cancellation: &CancellationToken,
    ) -> Result<UserOperationGuard, PipelineError> {
        validate_user(user_id)?;
        if operation_id.trim().is_empty() {
            return Err(PipelineError::InvalidRequest(
                "pipeline operation ID must be non-empty".into(),
            ));
        }
        let resetting = self.0.resetting.lock().map_err(lock_error)?;
        if resetting.contains(user_id) {
            return Err(PipelineError::InvalidRequest(
                "pipeline data is being reset for this user".into(),
            ));
        }
        let mut active_users = self.0.active_users.lock().map_err(lock_error)?;
        let operations = active_users.entry(user_id.to_owned()).or_default();
        if operations.contains_key(&operation_id) {
            return Err(PipelineError::InvalidRequest(
                "pipeline user operation is already in progress".into(),
            ));
        }
        operations.insert(operation_id.clone(), cancellation.clone());
        drop(resetting);
        Ok(UserOperationGuard {
            pipeline: self.clone(),
            user_id: user_id.to_owned(),
            operation_id,
        })
    }

    pub fn begin_attempt(
        &self,
        attempt_id: &str,
        user_id: &str,
    ) -> Result<CancellationToken, PipelineError> {
        validate_user(user_id)?;
        if attempt_id.trim().is_empty() {
            return Err(PipelineError::InvalidRequest(
                "pipeline attempt ID must be non-empty".into(),
            ));
        }
        let token = CancellationToken::new();
        let resetting = self.0.resetting.lock().map_err(lock_error)?;
        if resetting.contains(user_id) {
            return Err(PipelineError::InvalidRequest(
                "pipeline data is being reset for this user".into(),
            ));
        }
        let mut active = self.0.active.lock().map_err(lock_error)?;
        if active.contains_key(attempt_id) {
            return Err(PipelineError::InvalidRequest(
                "pipeline attempt is already in progress".into(),
            ));
        }
        let mut attempt_users = self.0.attempt_users.lock().map_err(lock_error)?;
        let mut active_users = self.0.active_users.lock().map_err(lock_error)?;
        let operations = active_users.entry(user_id.to_owned()).or_default();
        if operations.contains_key(attempt_id) {
            return Err(PipelineError::InvalidRequest(
                "pipeline user operation is already in progress".into(),
            ));
        }
        active.insert(attempt_id.to_owned(), token.clone());
        attempt_users.insert(attempt_id.to_owned(), user_id.to_owned());
        operations.insert(attempt_id.to_owned(), token.clone());
        Ok(token)
    }

    pub fn cancel(&self, attempt_id: &str) -> Result<(), PipelineError> {
        if let Some(token) = self.0.active.lock().map_err(lock_error)?.get(attempt_id) {
            token.cancel();
        }
        Ok(())
    }

    pub(crate) fn finish_attempt(&self, attempt_id: &str) -> Result<(), PipelineError> {
        self.0.active.lock().map_err(lock_error)?.remove(attempt_id);
        let user_id = self
            .0
            .attempt_users
            .lock()
            .map_err(lock_error)?
            .remove(attempt_id);
        if let Some(user_id) = user_id {
            let mut active_users = self.0.active_users.lock().map_err(lock_error)?;
            if let Some(operations) = active_users.get_mut(&user_id) {
                operations.remove(attempt_id);
                if operations.is_empty() {
                    active_users.remove(&user_id);
                }
            }
            drop(active_users);
            self.release_timed_out_reset_if_idle(&user_id)?;
        }
        Ok(())
    }

    pub fn publish_attempt(
        &self,
        attempt_id: &str,
        user_id: &str,
        acquisition: SourceAcquisition,
        request: CanonicalizationRequest,
        on_event: impl FnMut(PipelineProgress),
    ) -> Result<PipelinePublication, PipelineError> {
        validate_user(user_id)?;
        let token = self
            .0
            .active
            .lock()
            .map_err(lock_error)?
            .get(attempt_id)
            .cloned()
            .ok_or_else(|| PipelineError::NotFound("pipeline attempt".into()))?;
        let bound_user = self
            .0
            .attempt_users
            .lock()
            .map_err(lock_error)?
            .get(attempt_id)
            .cloned()
            .ok_or_else(|| PipelineError::NotFound("pipeline attempt".into()))?;
        if bound_user != user_id {
            return Err(PipelineError::InvalidRequest(
                "pipeline attempt belongs to a different user".into(),
            ));
        }
        let result = self.publish_internal(
            Some(attempt_id.to_owned()),
            user_id,
            acquisition,
            request,
            token,
            false,
            on_event,
        );
        self.finish_attempt(attempt_id)?;
        result
    }

    pub fn publish(
        &self,
        user_id: &str,
        acquisition: SourceAcquisition,
        request: CanonicalizationRequest,
        cancellation: CancellationToken,
        on_event: impl FnMut(PipelineProgress),
    ) -> Result<PipelinePublication, PipelineError> {
        self.publish_internal(
            None,
            user_id,
            acquisition,
            request,
            cancellation,
            false,
            on_event,
        )
    }

    pub fn publish_without_partial_source(
        &self,
        user_id: &str,
        acquisition: SourceAcquisition,
        request: CanonicalizationRequest,
        cancellation: CancellationToken,
        on_event: impl FnMut(PipelineProgress),
    ) -> Result<PipelinePublication, PipelineError> {
        self.publish_internal(
            None,
            user_id,
            acquisition,
            request,
            cancellation,
            true,
            on_event,
        )
    }

    pub fn list(&self, user_id: &str) -> Result<Vec<PipelineDatasetSummary>, PipelineError> {
        validate_user(user_id)?;
        let database = self.0.database.lock().map_err(lock_error)?;
        let mut statement = database
            .prepare(
                "SELECT s.source_json, c.canonical_json, q.report_json
                 FROM pipeline_sources s
                 JOIN pipeline_source_access sa USING(source_id)
                 LEFT JOIN pipeline_canonical_datasets c USING(source_id)
                 LEFT JOIN pipeline_quality_reports q USING(source_id)
                 WHERE sa.user_id = ?1
                 ORDER BY CAST(json_extract(s.source_json, '$.revision') AS INTEGER), s.source_id",
            )
            .map_err(storage)?;
        statement
            .query_map([user_id], |row| {
                let source: SourceCatalog = serde_json::from_str(&row.get::<_, String>(0)?)
                    .map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })?;
                let canonical = row
                    .get::<_, Option<String>>(1)?
                    .map(|json| serde_json::from_str::<CanonicalCatalog>(&json))
                    .transpose()
                    .map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            1,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })?;
                let quality = row
                    .get::<_, Option<String>>(2)?
                    .map(|json| serde_json::from_str::<QualityCatalog>(&json))
                    .transpose()
                    .map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            2,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })?;
                Ok(PipelineDatasetSummary {
                    source_id: source.source_id,
                    canonical_id: canonical.as_ref().map(|value| value.canonical_id.clone()),
                    revision: source.revision,
                    state: quality
                        .as_ref()
                        .map(|value| value.state.clone())
                        .unwrap_or(DataQualityState::Rejected),
                    source_record_count: source.record_count,
                    canonical_record_count: canonical.map_or(0, |value| value.bar_count),
                    quarantined_record_count: quality
                        .as_ref()
                        .map_or(0, |value| value.quarantine_count),
                    gap_count: quality.as_ref().map_or(0, |value| value.gap_count),
                })
            })
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)
    }

    pub fn source_for_user(
        &self,
        user_id: &str,
        source_id: &str,
    ) -> Result<SourceMarketDataset, PipelineError> {
        validate_user(user_id)?;
        let catalog = self.source_catalog_for_user(user_id, source_id)?;
        let records = read_json_lines(&catalog.evidence_path)?;
        let source = SourceMarketDataset {
            source_id: catalog.source_id,
            revision: catalog.revision,
            logical_key: catalog.logical_key,
            identity: catalog.identity,
            records,
            evidence_path: catalog.evidence_path,
        };
        if hash_file(&source.evidence_path)? != source.identity.content_sha256 {
            return Err(PipelineError::Storage(
                "Source evidence content hash does not match its catalog".into(),
            ));
        }
        Ok(source)
    }

    pub fn canonical_for_user(
        &self,
        user_id: &str,
        canonical_id: &str,
    ) -> Result<CanonicalMarketDataset, PipelineError> {
        validate_user(user_id)?;
        let catalog = self.canonical_catalog_for_user(user_id, canonical_id)?;
        let evidence_bytes = fs::read(&catalog.evidence_path).map_err(storage)?;
        if digest(&evidence_bytes) != catalog.evidence_sha256 {
            return Err(PipelineError::Storage(
                "Canonical row evidence hash does not match its catalog".into(),
            ));
        }
        let evidence: CanonicalEvidenceFile =
            serde_json::from_slice(&evidence_bytes).map_err(storage)?;
        if evidence.canonical_id != catalog.canonical_id
            || evidence.source_id != catalog.source_id
            || evidence.revision != catalog.revision
            || evidence.instrument != catalog.instrument
            || evidence.interval != catalog.interval
            || evidence.normalization_contract != catalog.normalization_contract
            || evidence.calendar != catalog.calendar
            || evidence.price_basis != catalog.price_basis
            || evidence.quality_report_id != catalog.quality_report_id
            || evidence.content_sha256 != catalog.content_sha256
            || evidence.parquet_path != catalog.parquet_path
        {
            return Err(PipelineError::Storage(
                "Canonical row evidence does not match its catalog".into(),
            ));
        }
        let bars = read_parquet(&catalog.parquet_path)?;
        if hash_file(&catalog.parquet_path)? != catalog.content_sha256 {
            return Err(PipelineError::Storage(
                "Canonical Parquet content hash does not match its catalog".into(),
            ));
        }
        Ok(CanonicalMarketDataset {
            canonical_id: catalog.canonical_id,
            source_id: catalog.source_id,
            revision: catalog.revision,
            instrument: catalog.instrument,
            interval: catalog.interval,
            normalization_contract: catalog.normalization_contract,
            calendar: catalog.calendar,
            price_basis: catalog.price_basis,
            bars,
            row_evidence: evidence.row_evidence,
            gaps: evidence.gaps,
            quality_report_id: evidence.quality_report_id,
            content_sha256: catalog.content_sha256,
            parquet_path: catalog.parquet_path,
        })
    }

    pub fn quality_for_user(
        &self,
        user_id: &str,
        report_id: &str,
    ) -> Result<DataQualityReport, PipelineError> {
        validate_user(user_id)?;
        let database = self.0.database.lock().map_err(lock_error)?;
        let quality_json: String = database
            .query_row(
                "SELECT q.report_json FROM pipeline_quality_reports q
                 JOIN pipeline_quality_access qa USING(report_id)
                 WHERE qa.user_id = ?1 AND q.report_id = ?2",
                params![user_id, report_id],
                |row| row.get(0),
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => {
                    PipelineError::NotFound("Data Quality Report".into())
                }
                error => storage(error),
            })?;
        drop(database);
        let catalog: QualityCatalog = serde_json::from_str(&quality_json).map_err(storage)?;
        let report_bytes = fs::read(catalog.evidence_path).map_err(storage)?;
        if digest(&report_bytes) != catalog.evidence_sha256 {
            return Err(PipelineError::Storage(
                "Data Quality Report hash does not match its catalog".into(),
            ));
        }
        serde_json::from_slice(&report_bytes).map_err(storage)
    }

    pub fn failures_for_user(&self, user_id: &str) -> Result<Vec<PipelineFailure>, PipelineError> {
        validate_user(user_id)?;
        let database = self.0.database.lock().map_err(lock_error)?;
        let mut statement = database
            .prepare(
                "SELECT attempt_id, source_id, stage, message, created_at_ms
                 FROM pipeline_failures
                 WHERE user_id = ?1
                 ORDER BY created_at_ms, rowid",
            )
            .map_err(storage)?;
        statement
            .query_map([user_id], |row| {
                Ok(PipelineFailure {
                    attempt_id: row.get(0)?,
                    source_id: row.get(1)?,
                    stage: row.get(2)?,
                    message: row.get(3)?,
                    created_at_ms: row.get(4)?,
                })
            })
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)
    }

    pub fn record_reference(
        &self,
        user_id: &str,
        evidence_kind: &str,
        evidence_id: &str,
        consumer_kind: &str,
        consumer_id: &str,
    ) -> Result<(), PipelineError> {
        validate_user(user_id)?;
        if evidence_id.trim().is_empty()
            || consumer_id.trim().is_empty()
            || !matches!(evidence_kind, "source" | "canonical" | "snapshot")
            || !matches!(
                consumer_kind,
                "dataset" | "run" | "report" | "deployment" | "snapshot"
            )
        {
            return Err(PipelineError::InvalidRequest(
                "invalid pipeline reference".into(),
            ));
        }
        let database = self.0.database.lock().map_err(lock_error)?;
        database
            .execute(
                "INSERT OR IGNORE INTO pipeline_references
                 (user_id, evidence_kind, evidence_id, consumer_kind, consumer_id)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    user_id,
                    evidence_kind,
                    evidence_id,
                    consumer_kind,
                    consumer_id
                ],
            )
            .map_err(storage)?;
        Ok(())
    }

    pub fn record_snapshot_reference(
        &self,
        user_id: &str,
        canonical_id: &str,
        snapshot_id: &str,
    ) -> Result<(), PipelineError> {
        self.record_reference(user_id, "canonical", canonical_id, "snapshot", snapshot_id)?;
        let database = self.0.database.lock().map_err(lock_error)?;
        database
            .execute(
                "INSERT OR IGNORE INTO pipeline_snapshot_links
                 (user_id, canonical_id, snapshot_id) VALUES (?1, ?2, ?3)",
                params![user_id, canonical_id, snapshot_id],
            )
            .map_err(storage)?;
        Ok(())
    }

    pub fn remove_snapshot_reference(
        &self,
        user_id: &str,
        canonical_id: &str,
        snapshot_id: &str,
    ) -> Result<(), PipelineError> {
        validate_user(user_id)?;
        let mut database = self.0.database.lock().map_err(lock_error)?;
        let transaction = database.transaction().map_err(storage)?;
        transaction
            .execute(
                "DELETE FROM pipeline_snapshot_links
                 WHERE user_id = ?1 AND canonical_id = ?2 AND snapshot_id = ?3",
                params![user_id, canonical_id, snapshot_id],
            )
            .map_err(storage)?;
        transaction
            .execute(
                "DELETE FROM pipeline_references
                 WHERE user_id = ?1 AND evidence_kind = 'canonical'
                   AND evidence_id = ?2 AND consumer_kind = 'snapshot'
                   AND consumer_id = ?3",
                params![user_id, canonical_id, snapshot_id],
            )
            .map_err(storage)?;
        transaction.commit().map_err(storage)
    }

    pub fn snapshot_deletion_blockers(
        &self,
        user_id: &str,
        snapshot_id: &str,
    ) -> Result<Vec<BlockingReference>, PipelineError> {
        validate_user(user_id)?;
        let database = self.0.database.lock().map_err(lock_error)?;
        let mut blockers = references_for(&database, user_id, "snapshot", snapshot_id)?;
        for (table, query, consumer_kind) in [
            (
                "backtest_runs",
                "SELECT run_id FROM backtest_runs
                 WHERE user_id = ?1
                 AND (json_extract(result_json, '$.snapshot.snapshotId') = ?2
                      OR json_extract(result_json, '$.provenance.normalizedRequest.snapshotId') = ?2)",
                "run",
            ),
            (
                "dataset_generation_attempts",
                "SELECT attempt_id FROM dataset_generation_attempts
                 WHERE user_id = ?1 AND json_extract(request_json, '$.snapshotId') = ?2",
                "dataset-generation-attempt",
            ),
            (
                "signal_dataset_content",
                "SELECT c.dataset_id FROM signal_dataset_content c
                 JOIN signal_dataset_access a USING(dataset_id)
                 WHERE a.user_id = ?1 AND json_extract(c.metadata_json, '$.snapshotId') = ?2",
                "dataset",
            ),
            (
                "validation_protocols",
                "SELECT DISTINCT p.protocol_id FROM validation_protocols p, json_tree(p.protocol_json) tree
                 WHERE p.user_id = ?1 AND tree.type = 'text' AND tree.value = ?2",
                "validation-protocol",
            ),
            (
                "validation_reports",
                "SELECT DISTINCT r.report_id FROM validation_reports r, json_tree(r.report_json) tree
                 WHERE r.user_id = ?1 AND tree.type = 'text' AND tree.value = ?2",
                "report",
            ),
            (
                "forecast_evaluation_content",
                "SELECT DISTINCT c.report_id FROM forecast_evaluation_content c
                 JOIN forecast_evaluation_access a USING(report_id), json_tree(c.report_json) tree
                 WHERE a.user_id = ?1 AND tree.type = 'text' AND tree.value = ?2",
                "report",
            ),
        ] {
            append_snapshot_table_blockers(
                &database,
                user_id,
                snapshot_id,
                table,
                query,
                consumer_kind,
                &mut blockers,
            )?;
        }
        blockers.sort_by(|left, right| {
            left.consumer_kind
                .cmp(&right.consumer_kind)
                .then_with(|| left.consumer_id.cmp(&right.consumer_id))
        });
        blockers.dedup();
        Ok(blockers)
    }

    pub fn snapshot_deletion_blockers_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<BlockingReference>, PipelineError> {
        validate_user(user_id)?;
        let database = self.0.database.lock().map_err(lock_error)?;
        let mut statement = database
            .prepare(
                "SELECT consumer_kind, consumer_id FROM pipeline_references
                 WHERE user_id = ?1 AND evidence_kind = 'snapshot'
                 ORDER BY consumer_kind, consumer_id",
            )
            .map_err(storage)?;
        statement
            .query_map([user_id], |row| {
                Ok(BlockingReference {
                    consumer_kind: row.get(0)?,
                    consumer_id: row.get(1)?,
                })
            })
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)
    }

    pub fn reset_paths_for_user(&self, user_id: &str) -> Result<Vec<PathBuf>, PipelineError> {
        validate_user(user_id)?;
        let database = self.0.database.lock().map_err(lock_error)?;
        self.reset_paths_for_user_with_connection(&database, user_id)
    }

    pub fn reset_paths_for_user_with_connection(
        &self,
        database: &Connection,
        user_id: &str,
    ) -> Result<Vec<PathBuf>, PipelineError> {
        validate_user(user_id)?;
        let queries = [
            (
                "SELECT s.source_json FROM pipeline_sources s
                 JOIN pipeline_source_access a USING(source_id)
                 WHERE a.user_id = ?1
                   AND NOT EXISTS (
                       SELECT 1 FROM pipeline_source_access other
                       WHERE other.source_id = s.source_id AND other.user_id <> ?1
                   )",
                0,
            ),
            (
                "SELECT c.canonical_json FROM pipeline_canonical_datasets c
                 JOIN pipeline_canonical_access a USING(canonical_id)
                 WHERE a.user_id = ?1
                   AND NOT EXISTS (
                       SELECT 1 FROM pipeline_canonical_access other
                       WHERE other.canonical_id = c.canonical_id AND other.user_id <> ?1
                   )",
                1,
            ),
            (
                "SELECT q.report_json FROM pipeline_quality_reports q
                 JOIN pipeline_quality_access a USING(report_id)
                 WHERE a.user_id = ?1
                   AND NOT EXISTS (
                       SELECT 1 FROM pipeline_quality_access other
                       WHERE other.report_id = q.report_id AND other.user_id <> ?1
                   )",
                2,
            ),
        ];
        let mut paths = Vec::new();
        for (sql, kind) in queries {
            let mut statement = database.prepare(sql).map_err(storage)?;
            let rows = statement
                .query_map([user_id], |row| row.get::<_, String>(0))
                .map_err(storage)?;
            for row in rows {
                let json = row.map_err(storage)?;
                match kind {
                    0 => paths.push(
                        serde_json::from_str::<SourceCatalog>(&json)
                            .map_err(storage)?
                            .evidence_path,
                    ),
                    1 => {
                        let catalog: CanonicalCatalog =
                            serde_json::from_str(&json).map_err(storage)?;
                        paths.push(catalog.parquet_path);
                        paths.push(catalog.evidence_path);
                    }
                    2 => paths.push(
                        serde_json::from_str::<QualityCatalog>(&json)
                            .map_err(storage)?
                            .evidence_path,
                    ),
                    _ => unreachable!(),
                }
            }
        }
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
            "pipeline_snapshot_links",
            "pipeline_references",
            "pipeline_failures",
            "pipeline_quality_access",
            "pipeline_canonical_access",
            "pipeline_source_access",
        ] {
            transaction
                .execute(
                    &format!("DELETE FROM {table} WHERE user_id = ?1"),
                    [user_id],
                )
                .map_err(storage)?;
        }
        transaction
            .execute(
                "DELETE FROM pipeline_quality_reports
                 WHERE NOT EXISTS (
                     SELECT 1 FROM pipeline_quality_access a
                     WHERE a.report_id = pipeline_quality_reports.report_id
                 )",
                [],
            )
            .map_err(storage)?;
        transaction
            .execute(
                "DELETE FROM pipeline_canonical_datasets
                 WHERE NOT EXISTS (
                     SELECT 1 FROM pipeline_canonical_access a
                     WHERE a.canonical_id = pipeline_canonical_datasets.canonical_id
                 )",
                [],
            )
            .map_err(storage)?;
        transaction
            .execute(
                "DELETE FROM pipeline_sources
                 WHERE NOT EXISTS (
                     SELECT 1 FROM pipeline_source_access a
                     WHERE a.source_id = pipeline_sources.source_id
                 )",
                [],
            )
            .map_err(storage)?;
        Ok(())
    }

    pub fn ensure_snapshot_deletable(
        &self,
        user_id: &str,
        snapshot_id: &str,
    ) -> Result<(), PipelineError> {
        let blockers = self.snapshot_deletion_blockers(user_id, snapshot_id)?;
        if blockers.is_empty() {
            Ok(())
        } else {
            Err(PipelineError::DeletionBlocked {
                evidence_kind: "Snapshot".into(),
                evidence_id: snapshot_id.into(),
                blockers,
            })
        }
    }

    pub fn delete_canonical_for_user(
        &self,
        user_id: &str,
        canonical_id: &str,
    ) -> Result<(), PipelineError> {
        validate_user(user_id)?;
        let blockers = {
            let database = self.0.database.lock().map_err(lock_error)?;
            references_for(&database, user_id, "canonical", canonical_id)?
        };
        if !blockers.is_empty() {
            return Err(PipelineError::DeletionBlocked {
                evidence_kind: "Canonical".into(),
                evidence_id: canonical_id.into(),
                blockers,
            });
        }
        let database = self.0.database.lock().map_err(lock_error)?;
        database
            .execute(
                "DELETE FROM pipeline_canonical_access
                 WHERE user_id = ?1 AND canonical_id = ?2",
                params![user_id, canonical_id],
            )
            .map_err(storage)?;
        Ok(())
    }

    pub fn delete_source_for_user(
        &self,
        user_id: &str,
        source_id: &str,
    ) -> Result<(), PipelineError> {
        validate_user(user_id)?;
        let database = self.0.database.lock().map_err(lock_error)?;
        let mut blockers = references_for(&database, user_id, "source", source_id)?;
        let mut statement = database
            .prepare(
                "SELECT c.canonical_id FROM pipeline_canonical_datasets c
                 JOIN pipeline_canonical_access ca USING(canonical_id)
                 WHERE ca.user_id = ?1 AND c.source_id = ?2",
            )
            .map_err(storage)?;
        let canonical_blockers = statement
            .query_map(params![user_id, source_id], |row| row.get::<_, String>(0))
            .map_err(storage)?
            .map(|id| {
                id.map(|consumer_id| BlockingReference {
                    consumer_kind: "canonical".into(),
                    consumer_id,
                })
                .map_err(storage)
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
        blockers.extend(canonical_blockers);
        if !blockers.is_empty() {
            return Err(PipelineError::DeletionBlocked {
                evidence_kind: "Source".into(),
                evidence_id: source_id.into(),
                blockers,
            });
        }
        database
            .execute(
                "DELETE FROM pipeline_source_access WHERE user_id = ?1 AND source_id = ?2",
                params![user_id, source_id],
            )
            .map_err(storage)?;
        Ok(())
    }

    fn grant_source_for_user(&self, user_id: &str, source_id: &str) -> Result<(), PipelineError> {
        validate_user(user_id)?;
        let database = self.0.database.lock().map_err(lock_error)?;
        database
            .execute(
                "INSERT OR IGNORE INTO pipeline_source_access(user_id, source_id)
                 VALUES (?1, ?2)",
                params![user_id, source_id],
            )
            .map_err(storage)?;
        Ok(())
    }

    fn publish_internal(
        &self,
        attempt_id: Option<String>,
        user_id: &str,
        acquisition: SourceAcquisition,
        request: CanonicalizationRequest,
        cancellation: CancellationToken,
        revoke_source_on_error: bool,
        mut on_event: impl FnMut(PipelineProgress),
    ) -> Result<PipelinePublication, PipelineError> {
        validate_user(user_id)?;
        request.validate()?;
        validate_acquisition(&acquisition)?;
        let operation_id = attempt_id
            .as_ref()
            .map(|attempt_id| format!("publication:{attempt_id}"))
            .unwrap_or_else(|| {
                format!(
                    "direct-{}",
                    self.0.next_operation.fetch_add(1, Ordering::Relaxed)
                )
            });
        let _user_operation = self.begin_user_operation(user_id, operation_id, &cancellation)?;
        if acquisition.price_basis != request.price_basis {
            return Err(PipelineError::InvalidRequest(
                "Source acquisition Price Basis differs from canonical request".into(),
            ));
        }
        let (source, access_inserted) = self.create_source(user_id, &acquisition, &request)?;
        let mut source_access = SourceAccessGuard {
            pipeline: self.clone(),
            user_id: user_id.to_owned(),
            source_id: source.source_id.clone(),
            keep_access: !revoke_source_on_error || !access_inserted,
        };
        emit(
            &mut on_event,
            PipelineProgress::Started {
                attempt_id: attempt_id.clone().unwrap_or_else(|| "direct".into()),
            },
        );
        emit(
            &mut on_event,
            PipelineProgress::SourcePersisted {
                source_id: source.source_id.clone(),
                revision: source.revision,
            },
        );
        if cancellation.is_cancelled() {
            emit(
                &mut on_event,
                PipelineProgress::Cancelled {
                    source_id: source.source_id.clone(),
                },
            );
            return Err(PipelineError::Cancelled {
                source_id: source.source_id,
            });
        }

        let output = match canonicalize_internal(&source, &request) {
            Ok(output) => output,
            Err(error) => {
                let stage = "canonicalization";
                let message = error.to_string();
                let failure_message = self.record_failure_message(
                    attempt_id.as_deref(),
                    user_id,
                    &source,
                    stage,
                    &message,
                );
                emit(
                    &mut on_event,
                    PipelineProgress::Failed {
                        source_id: source.source_id.clone(),
                        stage: stage.into(),
                    },
                );
                if failure_message == message {
                    return Err(error);
                }
                return Err(PipelineError::PublicationFailed {
                    source_id: source.source_id.clone(),
                    message: failure_message,
                });
            }
        };
        let canonical_id = if output.bars.is_empty() {
            None
        } else {
            Some(canonical_id(&source, &request, &output))
        };
        emit(
            &mut on_event,
            PipelineProgress::Canonicalized {
                source_id: source.source_id.clone(),
                canonical_id: canonical_id.clone(),
                state: quality_state(&source, &output),
            },
        );
        if cancellation.is_cancelled() {
            self.record_failure(
                attempt_id.as_deref(),
                user_id,
                &source,
                "canonicalization",
                "cancelled",
            )?;
            emit(
                &mut on_event,
                PipelineProgress::Cancelled {
                    source_id: source.source_id.clone(),
                },
            );
            return Err(PipelineError::Cancelled {
                source_id: source.source_id,
            });
        }

        let quality_path = self.0.root.join("quality").join(format!(
            "{}.json",
            report_id(&source, canonical_id.as_deref(), &output)
        ));
        let report_id = report_id(&source, canonical_id.as_deref(), &output);
        let mut quality = build_quality_report(
            &source,
            canonical_id.as_deref(),
            &output,
            quality_path.clone(),
        );
        let mut artifacts = PublicationArtifacts::default();
        let canonical = if let Some(canonical_id) = canonical_id {
            let parquet_path = self
                .0
                .root
                .join("canonical")
                .join(format!("{canonical_id}.parquet"));
            artifacts.track(&parquet_path);
            let content_sha256 =
                match write_parquet_atomic(&parquet_path, &output.bars, &cancellation) {
                    Ok(hash) => hash,
                    Err(PipelineError::Cancelled { .. }) => {
                        self.record_failure(
                            attempt_id.as_deref(),
                            user_id,
                            &source,
                            "canonical-publication",
                            "cancelled",
                        )?;
                        emit(
                            &mut on_event,
                            PipelineProgress::Cancelled {
                                source_id: source.source_id.clone(),
                            },
                        );
                        return Err(PipelineError::Cancelled {
                            source_id: source.source_id,
                        });
                    }
                    Err(error) => {
                        let message = error.to_string();
                        let message = self.record_failure_message(
                            attempt_id.as_deref(),
                            user_id,
                            &source,
                            "canonical-publication",
                            &message,
                        );
                        emit(
                            &mut on_event,
                            PipelineProgress::Failed {
                                source_id: source.source_id.clone(),
                                stage: "canonical-publication".into(),
                            },
                        );
                        return Err(PipelineError::PublicationFailed {
                            source_id: source.source_id.clone(),
                            message,
                        });
                    }
                };
            if cancellation.is_cancelled() {
                self.record_failure(
                    attempt_id.as_deref(),
                    user_id,
                    &source,
                    "canonical-publication",
                    "cancelled",
                )?;
                emit(
                    &mut on_event,
                    PipelineProgress::Cancelled {
                        source_id: source.source_id.clone(),
                    },
                );
                return Err(PipelineError::Cancelled {
                    source_id: source.source_id,
                });
            }
            let canonical = CanonicalMarketDataset {
                canonical_id: canonical_id.clone(),
                source_id: source.source_id.clone(),
                revision: source.revision,
                instrument: request.instrument.clone(),
                interval: request.interval,
                normalization_contract: request.normalization_contract.clone(),
                calendar: request.calendar.clone(),
                price_basis: request.price_basis,
                bars: output.bars.clone(),
                row_evidence: output.row_evidence.clone(),
                gaps: output.gaps.clone(),
                quality_report_id: report_id.clone(),
                content_sha256,
                parquet_path,
            };
            quality.canonical_id = Some(canonical_id.clone());
            Some(canonical)
        } else {
            None
        };
        let canonical_evidence = if let Some(canonical) = canonical.as_ref() {
            let evidence_path = self
                .0
                .root
                .join("canonical")
                .join(format!("{}.json", canonical.canonical_id));
            artifacts.track(&evidence_path);
            let evidence_bytes =
                match canonical_json_bytes(&CanonicalEvidenceFile::from_canonical(canonical)) {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        let message = error.to_string();
                        let message = self.record_failure_message(
                            attempt_id.as_deref(),
                            user_id,
                            &source,
                            "canonical-evidence-publication",
                            &message,
                        );
                        emit(
                            &mut on_event,
                            PipelineProgress::Failed {
                                source_id: source.source_id.clone(),
                                stage: "canonical-evidence-publication".into(),
                            },
                        );
                        return Err(PipelineError::PublicationFailed {
                            source_id: source.source_id.clone(),
                            message,
                        });
                    }
                };
            let evidence_sha256 = digest(&evidence_bytes);
            if let Err(error) = atomic_write(&evidence_path, &evidence_bytes) {
                let message = error.to_string();
                let message = self.record_failure_message(
                    attempt_id.as_deref(),
                    user_id,
                    &source,
                    "canonical-evidence-publication",
                    &message,
                );
                emit(
                    &mut on_event,
                    PipelineProgress::Failed {
                        source_id: source.source_id.clone(),
                        stage: "canonical-evidence-publication".into(),
                    },
                );
                return Err(PipelineError::PublicationFailed {
                    source_id: source.source_id.clone(),
                    message,
                });
            }
            Some((evidence_path, evidence_sha256))
        } else {
            None
        };
        let quality_bytes = match canonical_json_bytes(&quality) {
            Ok(bytes) => bytes,
            Err(error) => {
                let message = error.to_string();
                let message = self.record_failure_message(
                    attempt_id.as_deref(),
                    user_id,
                    &source,
                    "quality-publication",
                    &message,
                );
                emit(
                    &mut on_event,
                    PipelineProgress::Failed {
                        source_id: source.source_id.clone(),
                        stage: "quality-publication".into(),
                    },
                );
                return Err(PipelineError::PublicationFailed {
                    source_id: source.source_id.clone(),
                    message,
                });
            }
        };
        let quality_sha256 = digest(&quality_bytes);
        artifacts.track(&quality_path);
        if let Err(error) = atomic_write(&quality_path, &quality_bytes) {
            let message = error.to_string();
            let message = self.record_failure_message(
                attempt_id.as_deref(),
                user_id,
                &source,
                "quality-publication",
                &message,
            );
            emit(
                &mut on_event,
                PipelineProgress::Failed {
                    source_id: source.source_id.clone(),
                    stage: "quality-publication".into(),
                },
            );
            return Err(PipelineError::PublicationFailed {
                source_id: source.source_id.clone(),
                message,
            });
        }
        if cancellation.is_cancelled() {
            self.record_failure(
                attempt_id.as_deref(),
                user_id,
                &source,
                "quality-publication",
                "cancelled",
            )?;
            emit(
                &mut on_event,
                PipelineProgress::Cancelled {
                    source_id: source.source_id.clone(),
                },
            );
            return Err(PipelineError::Cancelled {
                source_id: source.source_id,
            });
        }
        if let Err(error) = self.commit_catalog(
            user_id,
            &source,
            canonical.as_ref(),
            canonical_evidence.as_ref(),
            &quality,
            &quality_sha256,
            &cancellation,
        ) {
            if matches!(&error, PipelineError::Cancelled { .. }) {
                self.record_failure(
                    attempt_id.as_deref(),
                    user_id,
                    &source,
                    "catalog-cutover",
                    "cancelled",
                )?;
                emit(
                    &mut on_event,
                    PipelineProgress::Cancelled {
                        source_id: source.source_id.clone(),
                    },
                );
                return Err(error);
            }
            let message = error.to_string();
            let message = self.record_failure_message(
                attempt_id.as_deref(),
                user_id,
                &source,
                "catalog-cutover",
                &message,
            );
            emit(
                &mut on_event,
                PipelineProgress::Failed {
                    source_id: source.source_id.clone(),
                    stage: "catalog-cutover".into(),
                },
            );
            return Err(PipelineError::PublicationFailed {
                source_id: source.source_id.clone(),
                message,
            });
        }
        artifacts.commit_on_catalog_cutover();
        source_access.commit();
        if let Some(canonical) = &canonical {
            emit(
                &mut on_event,
                PipelineProgress::Published {
                    source_id: source.source_id.clone(),
                    canonical_id: canonical.canonical_id.clone(),
                    report_id: report_id.clone(),
                },
            );
        }
        emit(
            &mut on_event,
            PipelineProgress::Completed {
                source_id: source.source_id.clone(),
                canonical_id: canonical.as_ref().map(|value| value.canonical_id.clone()),
                report_id,
                state: quality.state.clone(),
            },
        );
        Ok(PipelinePublication {
            attempt_id,
            source,
            canonical,
            quality,
        })
    }

    fn initialize_schema(&self) -> Result<(), PipelineError> {
        let database = self.0.database.lock().map_err(lock_error)?;
        database
            .execute_batch(
                "PRAGMA foreign_keys = ON;
                 CREATE TABLE IF NOT EXISTS pipeline_sources (
                    source_id TEXT PRIMARY KEY,
                    source_json TEXT NOT NULL,
                    created_at_ms INTEGER NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS pipeline_source_access (
                    user_id TEXT NOT NULL,
                    source_id TEXT NOT NULL,
                    PRIMARY KEY(user_id, source_id),
                    FOREIGN KEY(source_id) REFERENCES pipeline_sources(source_id)
                 );
                 CREATE TABLE IF NOT EXISTS pipeline_canonical_datasets (
                    canonical_id TEXT PRIMARY KEY,
                    source_id TEXT NOT NULL,
                    canonical_json TEXT NOT NULL,
                    FOREIGN KEY(source_id) REFERENCES pipeline_sources(source_id)
                 );
                 CREATE TABLE IF NOT EXISTS pipeline_canonical_access (
                    user_id TEXT NOT NULL,
                    canonical_id TEXT NOT NULL,
                    PRIMARY KEY(user_id, canonical_id),
                    FOREIGN KEY(canonical_id) REFERENCES pipeline_canonical_datasets(canonical_id)
                 );
                 CREATE TABLE IF NOT EXISTS pipeline_quality_reports (
                    report_id TEXT PRIMARY KEY,
                    source_id TEXT NOT NULL,
                    report_json TEXT NOT NULL,
                    FOREIGN KEY(source_id) REFERENCES pipeline_sources(source_id)
                 );
                 CREATE TABLE IF NOT EXISTS pipeline_quality_access (
                    user_id TEXT NOT NULL,
                    report_id TEXT NOT NULL,
                    PRIMARY KEY(user_id, report_id),
                    FOREIGN KEY(report_id) REFERENCES pipeline_quality_reports(report_id)
                 );
                 CREATE TABLE IF NOT EXISTS pipeline_snapshot_links (
                    user_id TEXT NOT NULL,
                    canonical_id TEXT NOT NULL,
                    snapshot_id TEXT NOT NULL,
                    PRIMARY KEY(user_id, canonical_id, snapshot_id)
                 );
                 CREATE TABLE IF NOT EXISTS pipeline_references (
                    user_id TEXT NOT NULL,
                    evidence_kind TEXT NOT NULL,
                    evidence_id TEXT NOT NULL,
                    consumer_kind TEXT NOT NULL,
                    consumer_id TEXT NOT NULL,
                    PRIMARY KEY(user_id, evidence_kind, evidence_id, consumer_kind, consumer_id)
                 );
                 CREATE TABLE IF NOT EXISTS pipeline_failures (
                    attempt_id TEXT,
                    user_id TEXT NOT NULL,
                    source_id TEXT NOT NULL,
                    stage TEXT NOT NULL,
                    message TEXT NOT NULL,
                    created_at_ms INTEGER NOT NULL
                 );",
            )
            .map_err(storage)
    }

    fn create_source(
        &self,
        user_id: &str,
        acquisition: &SourceAcquisition,
        request: &CanonicalizationRequest,
    ) -> Result<(SourceMarketDataset, bool), PipelineError> {
        let logical_key = digest(&canonical_json_bytes(&(
            &acquisition.provider,
            &acquisition.actual_upstream,
            &acquisition.connector,
            &acquisition.connector_version,
            &acquisition.request_parameters,
            acquisition.price_basis,
            &request.instrument,
            &request.interval,
        ))?);
        let payload_sha256 = payload_hash(&acquisition.records)?;
        let evidence_bytes = source_evidence_bytes(&acquisition.records)?;
        let content_sha256 = digest(&evidence_bytes);
        let identity_without_id = SourceIdentity {
            provider: acquisition.provider.clone(),
            actual_upstream: acquisition.actual_upstream.clone(),
            connector: acquisition.connector.clone(),
            connector_version: acquisition.connector_version.clone(),
            request_parameters: acquisition.request_parameters.clone(),
            retrieved_at_ms: acquisition.retrieved_at_ms,
            response_sha256s: acquisition.response_sha256s.clone(),
            acquisition_content_sha256: acquisition.acquisition_content_sha256.clone(),
            payload_sha256,
            content_sha256: content_sha256.clone(),
            capability_snapshot: acquisition.capability_snapshot.clone(),
            acquisition_diagnostics: acquisition.acquisition_diagnostics.clone(),
            price_basis: acquisition.price_basis,
        };
        let mut database = self.0.database.lock().map_err(lock_error)?;
        let transaction = database
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        let existing_json = transaction
            .query_row(
                "SELECT s.source_json FROM pipeline_sources s
                     JOIN pipeline_source_access a USING(source_id)
                     WHERE a.user_id = ?1
                       AND json_extract(s.source_json, '$.logicalKey') = ?2
                       AND json_extract(s.source_json, '$.identity.contentSha256') = ?3
                     ORDER BY CAST(json_extract(s.source_json, '$.revision') AS INTEGER) DESC
                     LIMIT 1",
                params![user_id, logical_key, content_sha256],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(storage)?;
        if let Some(existing_json) = existing_json {
            let catalog: SourceCatalog = serde_json::from_str(&existing_json).map_err(storage)?;
            if catalog.identity == identity_without_id {
                let evidence = fs::read(&catalog.evidence_path).map_err(storage)?;
                if digest(&evidence) != catalog.identity.content_sha256 {
                    return Err(PipelineError::Storage(
                        "Existing A-share Source evidence hash does not match its catalog".into(),
                    ));
                }
                let records = read_json_lines(&catalog.evidence_path)?;
                let had_access = transaction
                    .query_row(
                        "SELECT EXISTS(
                             SELECT 1 FROM pipeline_source_access
                             WHERE user_id = ?1 AND source_id = ?2
                         )",
                        params![user_id, catalog.source_id],
                        |row| row.get::<_, i64>(0),
                    )
                    .map_err(storage)?
                    != 0;
                transaction.commit().map_err(storage)?;
                return Ok((
                    SourceMarketDataset {
                        source_id: catalog.source_id,
                        revision: catalog.revision,
                        logical_key: catalog.logical_key,
                        identity: catalog.identity,
                        records,
                        evidence_path: catalog.evidence_path,
                    },
                    !had_access,
                ));
            }
        }
        let revision = {
            transaction
                .query_row(
                    "SELECT COALESCE(MAX(CAST(json_extract(source_json, '$.revision') AS INTEGER)), 0)
                     FROM pipeline_sources s JOIN pipeline_source_access a USING(source_id)
                     WHERE a.user_id = ?1 AND json_extract(source_json, '$.logicalKey') = ?2",
                    params![user_id, logical_key],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(storage)?
                .max(0) as u64
                + 1
        };
        let source_id = digest(&canonical_json_bytes(&(
            revision,
            &logical_key,
            &identity_without_id,
        ))?);
        let evidence_path = self
            .0
            .root
            .join("sources")
            .join(format!("{source_id}.jsonl"));
        atomic_write(&evidence_path, &evidence_bytes)?;
        let source = SourceMarketDataset {
            source_id,
            revision,
            logical_key,
            identity: identity_without_id,
            records: acquisition.records.clone(),
            evidence_path,
        };
        let catalog = SourceCatalog::from_source(&source);
        let catalog_json = serde_json::to_string(&catalog).map_err(storage)?;
        transaction
            .execute(
                "INSERT OR IGNORE INTO pipeline_sources(source_id, source_json, created_at_ms)
                 VALUES (?1, ?2, ?3)",
                params![source.source_id, catalog_json, acquisition.retrieved_at_ms],
            )
            .map_err(storage)?;
        transaction.commit().map_err(storage)?;
        Ok((source, true))
    }

    fn commit_catalog(
        &self,
        user_id: &str,
        source: &SourceMarketDataset,
        canonical: Option<&CanonicalMarketDataset>,
        canonical_evidence: Option<&(PathBuf, String)>,
        quality: &DataQualityReport,
        quality_sha256: &str,
        cancellation: &CancellationToken,
    ) -> Result<(), PipelineError> {
        let quality_json = serde_json::to_string(&QualityCatalog::from_report(
            quality,
            quality_sha256.to_owned(),
        ))
        .map_err(storage)?;
        let database = self.0.database.lock().map_err(lock_error)?;
        let transaction = database.unchecked_transaction().map_err(storage)?;
        transaction
            .execute(
                "INSERT OR IGNORE INTO pipeline_source_access(user_id, source_id)
                 VALUES (?1, ?2)",
                params![user_id, source.source_id],
            )
            .map_err(storage)?;
        if let Some(canonical) = canonical {
            let (evidence_path, evidence_sha256) = canonical_evidence.ok_or_else(|| {
                PipelineError::Storage("Canonical row evidence is missing".into())
            })?;
            let catalog_json = serde_json::to_string(&CanonicalCatalog::from_canonical(
                canonical,
                evidence_path.clone(),
                evidence_sha256.clone(),
            ))
            .map_err(storage)?;
            transaction
                .execute(
                    "INSERT OR IGNORE INTO pipeline_canonical_datasets
                     (canonical_id, source_id, canonical_json) VALUES (?1, ?2, ?3)",
                    params![canonical.canonical_id, source.source_id, catalog_json],
                )
                .map_err(storage)?;
            transaction
                .execute(
                    "INSERT OR IGNORE INTO pipeline_canonical_access(user_id, canonical_id)
                     VALUES (?1, ?2)",
                    params![user_id, canonical.canonical_id],
                )
                .map_err(storage)?;
        }
        transaction
            .execute(
                "INSERT OR IGNORE INTO pipeline_quality_reports
                 (report_id, source_id, report_json) VALUES (?1, ?2, ?3)",
                params![quality.report_id, source.source_id, quality_json],
            )
            .map_err(storage)?;
        transaction
            .execute(
                "INSERT OR IGNORE INTO pipeline_quality_access(user_id, report_id)
                 VALUES (?1, ?2)",
                params![user_id, quality.report_id],
            )
            .map_err(storage)?;
        if cancellation.is_cancelled() {
            // This is the publication cutover: once the final cooperative
            // check passes, SQLite commit wins over a cancellation arriving
            // during the atomic commit.
            return Err(PipelineError::Cancelled {
                source_id: source.source_id.clone(),
            });
        }
        transaction.commit().map_err(storage)
    }

    fn source_catalog_for_user(
        &self,
        user_id: &str,
        source_id: &str,
    ) -> Result<SourceCatalog, PipelineError> {
        let database = self.0.database.lock().map_err(lock_error)?;
        let json: String = database
            .query_row(
                "SELECT s.source_json FROM pipeline_sources s
                 JOIN pipeline_source_access a USING(source_id)
                 WHERE a.user_id = ?1 AND s.source_id = ?2",
                params![user_id, source_id],
                |row| row.get(0),
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => {
                    PipelineError::NotFound("Source Market Dataset".into())
                }
                error => storage(error),
            })?;
        serde_json::from_str(&json).map_err(storage)
    }

    fn canonical_catalog_for_user(
        &self,
        user_id: &str,
        canonical_id: &str,
    ) -> Result<CanonicalCatalog, PipelineError> {
        let database = self.0.database.lock().map_err(lock_error)?;
        let json: String = database
            .query_row(
                "SELECT c.canonical_json FROM pipeline_canonical_datasets c
                 JOIN pipeline_canonical_access a USING(canonical_id)
                 WHERE a.user_id = ?1 AND c.canonical_id = ?2",
                params![user_id, canonical_id],
                |row| row.get(0),
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => {
                    PipelineError::NotFound("Canonical Market Dataset".into())
                }
                error => storage(error),
            })?;
        serde_json::from_str(&json).map_err(storage)
    }

    fn record_failure_message(
        &self,
        attempt_id: Option<&str>,
        user_id: &str,
        source: &SourceMarketDataset,
        stage: &str,
        message: &str,
    ) -> String {
        match self.record_failure(attempt_id, user_id, source, stage, message) {
            Ok(()) => message.to_owned(),
            Err(error) => format!("{message}; failure evidence could not be retained: {error}"),
        }
    }

    fn record_failure(
        &self,
        attempt_id: Option<&str>,
        user_id: &str,
        source: &SourceMarketDataset,
        stage: &str,
        message: &str,
    ) -> Result<(), PipelineError> {
        let database = self.0.database.lock().map_err(lock_error)?;
        database
            .execute(
                "INSERT INTO pipeline_failures
                 (attempt_id, user_id, source_id, stage, message, created_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, CAST(strftime('%s','now') AS INTEGER) * 1000)",
                params![attempt_id, user_id, source.source_id, stage, message],
            )
            .map_err(storage)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SourceCatalog {
    source_id: String,
    revision: u64,
    logical_key: String,
    identity: SourceIdentity,
    record_count: usize,
    evidence_path: PathBuf,
}

impl SourceCatalog {
    fn from_source(source: &SourceMarketDataset) -> Self {
        Self {
            source_id: source.source_id.clone(),
            revision: source.revision,
            logical_key: source.logical_key.clone(),
            identity: source.identity.clone(),
            record_count: source.records.len(),
            evidence_path: source.evidence_path.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CanonicalCatalog {
    canonical_id: String,
    source_id: String,
    revision: u64,
    instrument: InstrumentId,
    interval: BarInterval,
    normalization_contract: String,
    calendar: CalendarEvidence,
    #[serde(default)]
    price_basis: PriceBasis,
    quality_report_id: String,
    content_sha256: String,
    parquet_path: PathBuf,
    evidence_path: PathBuf,
    evidence_sha256: String,
    bar_count: usize,
}

impl CanonicalCatalog {
    fn from_canonical(
        canonical: &CanonicalMarketDataset,
        evidence_path: PathBuf,
        evidence_sha256: String,
    ) -> Self {
        Self {
            canonical_id: canonical.canonical_id.clone(),
            source_id: canonical.source_id.clone(),
            revision: canonical.revision,
            instrument: canonical.instrument.clone(),
            interval: canonical.interval,
            normalization_contract: canonical.normalization_contract.clone(),
            calendar: canonical.calendar.clone(),
            price_basis: canonical.price_basis,
            quality_report_id: canonical.quality_report_id.clone(),
            content_sha256: canonical.content_sha256.clone(),
            parquet_path: canonical.parquet_path.clone(),
            evidence_path,
            evidence_sha256,
            bar_count: canonical.bars.len(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CanonicalEvidenceFile {
    canonical_id: String,
    source_id: String,
    revision: u64,
    instrument: InstrumentId,
    interval: BarInterval,
    normalization_contract: String,
    calendar: CalendarEvidence,
    #[serde(default)]
    price_basis: PriceBasis,
    row_evidence: Vec<CanonicalRowEvidence>,
    gaps: Vec<BarGap>,
    quality_report_id: String,
    content_sha256: String,
    parquet_path: PathBuf,
}

impl CanonicalEvidenceFile {
    fn from_canonical(canonical: &CanonicalMarketDataset) -> Self {
        Self {
            canonical_id: canonical.canonical_id.clone(),
            source_id: canonical.source_id.clone(),
            revision: canonical.revision,
            instrument: canonical.instrument.clone(),
            interval: canonical.interval,
            normalization_contract: canonical.normalization_contract.clone(),
            calendar: canonical.calendar.clone(),
            price_basis: canonical.price_basis,
            row_evidence: canonical.row_evidence.clone(),
            gaps: canonical.gaps.clone(),
            quality_report_id: canonical.quality_report_id.clone(),
            content_sha256: canonical.content_sha256.clone(),
            parquet_path: canonical.parquet_path.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QualityCatalog {
    report_id: String,
    state: DataQualityState,
    quarantine_count: usize,
    gap_count: usize,
    evidence_path: PathBuf,
    evidence_sha256: String,
}

impl QualityCatalog {
    fn from_report(report: &DataQualityReport, evidence_sha256: String) -> Self {
        Self {
            report_id: report.report_id.clone(),
            state: report.state.clone(),
            quarantine_count: report.quarantine_count,
            gap_count: report.gap_count,
            evidence_path: report.evidence_path.clone(),
            evidence_sha256,
        }
    }
}

fn emit(callback: &mut impl FnMut(PipelineProgress), event: PipelineProgress) {
    callback(event);
}

pub(crate) fn validate_user(user_id: &str) -> Result<(), PipelineError> {
    if user_id.trim().is_empty() || user_id.len() > 128 {
        Err(PipelineError::InvalidRequest("User ID is invalid".into()))
    } else {
        Ok(())
    }
}

fn validate_acquisition(acquisition: &SourceAcquisition) -> Result<(), PipelineError> {
    for (name, value) in [
        ("provider", acquisition.provider.as_str()),
        ("connector", acquisition.connector.as_str()),
        ("connector version", acquisition.connector_version.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(PipelineError::InvalidRequest(format!(
                "{name} must be non-empty"
            )));
        }
    }
    if acquisition.retrieved_at_ms < 0 {
        return Err(PipelineError::InvalidRequest(
            "retrieval time must be a UTC instant".into(),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct CanonicalizationOutput {
    bars: Vec<OhlcvBar>,
    row_evidence: Vec<CanonicalRowEvidence>,
    gaps: Vec<BarGap>,
    quarantined_records: Vec<QuarantinedMarketRecord>,
    warnings: Vec<CanonicalWarning>,
    duplicate_count: usize,
    conflict_count: usize,
    coverage: Coverage,
}

pub fn canonicalize(
    source: &SourceMarketDataset,
    request: &CanonicalizationRequest,
) -> Result<CanonicalizationPreview, PipelineError> {
    request.validate()?;
    let output = canonicalize_internal(source, request)?;
    Ok(CanonicalizationPreview::from_output(&output))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalizationPreview {
    pub bars: Vec<OhlcvBar>,
    pub row_evidence: Vec<CanonicalRowEvidence>,
    pub gaps: Vec<BarGap>,
    pub quarantined_records: Vec<QuarantinedMarketRecord>,
    pub warnings: Vec<CanonicalWarning>,
    pub duplicate_count: usize,
    pub conflict_count: usize,
    pub coverage: Coverage,
}

impl CanonicalizationPreview {
    fn from_output(output: &CanonicalizationOutput) -> Self {
        Self {
            bars: output.bars.clone(),
            row_evidence: output.row_evidence.clone(),
            gaps: output.gaps.clone(),
            quarantined_records: output.quarantined_records.clone(),
            warnings: output.warnings.clone(),
            duplicate_count: output.duplicate_count,
            conflict_count: output.conflict_count,
            coverage: output.coverage.clone(),
        }
    }
}

fn canonicalize_internal(
    source: &SourceMarketDataset,
    request: &CanonicalizationRequest,
) -> Result<CanonicalizationOutput, PipelineError> {
    let mut groups: BTreeMap<i64, Vec<ParsedRecord>> = BTreeMap::new();
    let mut quarantined_records = Vec::new();
    let mut warnings = Vec::new();
    for record in &source.records {
        let hash = record.fingerprint();
        match parse_record(record, request) {
            Ok(parsed) => {
                warnings.extend(
                    parsed
                        .warnings
                        .iter()
                        .cloned()
                        .map(|reason| CanonicalWarning {
                            source_record_hash: hash.clone(),
                            reason,
                        }),
                );
                groups
                    .entry(record.open_time_ms)
                    .or_default()
                    .push(ParsedRecord {
                        hash,
                        record: record.clone(),
                        bar: parsed.bar,
                    });
            }
            Err(reason) => quarantined_records.push(QuarantinedMarketRecord {
                source_record_hash: hash,
                record: record.clone(),
                reason,
            }),
        }
    }

    let mut bars = Vec::new();
    let mut row_evidence = Vec::new();
    let mut duplicate_count = 0;
    let mut conflict_count = 0;
    for (open_time_ms, mut records) in groups {
        records.sort_by(|left, right| left.hash.cmp(&right.hash));
        if records.windows(2).all(|pair| pair[0].bar == pair[1].bar) {
            duplicate_count += records.len().saturating_sub(1);
            let first = records.first().expect("group is non-empty");
            bars.push(first.bar.clone());
            row_evidence.push(CanonicalRowEvidence {
                identity: BarIdentity {
                    instrument: request.instrument.clone(),
                    interval: request.interval,
                    open_time_ms,
                },
                source_record_hashes: records.into_iter().map(|record| record.hash).collect(),
            });
        } else {
            conflict_count += records.len();
            for record in records {
                quarantined_records.push(QuarantinedMarketRecord {
                    source_record_hash: record.hash,
                    record: record.record,
                    reason: QuarantineReason::ConflictingIdentityOrValue {
                        details: "same Bar Identity has different financial values".into(),
                    },
                });
            }
        }
    }
    let gaps = detect_gaps(request, &bars, &quarantined_records)?;
    let coverage = coverage(request, &bars, &gaps);
    Ok(CanonicalizationOutput {
        bars,
        row_evidence,
        gaps,
        quarantined_records,
        warnings,
        duplicate_count,
        conflict_count,
        coverage,
    })
}

struct ParsedRecord {
    hash: String,
    record: SourceMarketRecord,
    bar: OhlcvBar,
}

struct ParsedBar {
    bar: OhlcvBar,
    warnings: Vec<WarningReason>,
}

fn parse_record(
    record: &SourceMarketRecord,
    request: &CanonicalizationRequest,
) -> Result<ParsedBar, QuarantineReason> {
    if record.instrument != request.instrument || record.interval != request.interval {
        return Err(QuarantineReason::UnsupportedIdentity {
            details: "record Instrument or Bar Interval differs from request".into(),
        });
    }
    if record.provider_symbol.trim().is_empty() {
        return Err(QuarantineReason::MissingRequiredField {
            field: "providerSymbol".into(),
        });
    }
    if adaq_data_core::market::Venue::local_time(&record.instrument.venue, record.open_time_ms)
        .is_err()
    {
        return Err(QuarantineReason::InvalidUtcInstant);
    }
    request.calendar.is_expected_bar_time(
        &request.instrument,
        request.interval,
        record.open_time_ms,
    )?;
    let parse = |field: &'static str, value: &Option<String>| {
        Decimal::from_str(value.as_deref().ok_or_else(|| {
            QuarantineReason::MissingRequiredField {
                field: field.into(),
            }
        })?)
        .map_err(|_| QuarantineReason::UnparsableExactValue {
            field: field.into(),
        })
    };
    let mut warnings = Vec::new();
    let quote_volume = parse("quoteVolume", &record.quote_volume)?;
    let bar = OhlcvBar {
        open_time_ms: record.open_time_ms,
        open: parse("open", &record.open)?,
        high: parse("high", &record.high)?,
        low: parse("low", &record.low)?,
        close: parse("close", &record.close)?,
        base_volume: parse("baseVolume", &record.base_volume)?,
        quote_volume,
    };
    if [
        &bar.open,
        &bar.high,
        &bar.low,
        &bar.close,
        &bar.base_volume,
        &bar.quote_volume,
    ]
    .iter()
    .any(|value| value.is_sign_negative())
    {
        return Err(QuarantineReason::InvalidFinancialInvariant {
            details: "OHLCV values must be non-negative".into(),
        });
    }
    if bar.high < bar.open.max(bar.close) || bar.low > bar.open.min(bar.close) {
        return Err(QuarantineReason::InvalidFinancialInvariant {
            details: "high and low do not contain open and close".into(),
        });
    }
    if bar.base_volume.is_zero() && bar.quote_volume.is_zero() {
        warnings.push(WarningReason::ZeroVolume);
    }
    if bar.low > Decimal::ZERO && bar.high / bar.low > Decimal::from(100u32) {
        warnings.push(WarningReason::WidePriceRange);
    }
    Ok(ParsedBar { bar, warnings })
}

fn validate_venue_bar_time(
    calendar: &TradingCalendarSnapshot,
    interval: BarInterval,
    open_time_ms: i64,
) -> Result<(), CalendarError> {
    let context = calendar.session_context_at(open_time_ms)?;
    if matches!(
        interval,
        BarInterval::OneDay
            | BarInterval::TwoDays
            | BarInterval::ThreeDays
            | BarInterval::FiveDays
            | BarInterval::OneWeek
            | BarInterval::OneMonth
            | BarInterval::ThreeMonths
    ) {
        if calendar.daily_boundary_open_ms(context.trading_date)? != open_time_ms {
            return Err(CalendarError::InvalidCalendar(
                "calendar Bar is not aligned to the Venue Trading Date boundary",
            ));
        }
        return Ok(());
    }
    if !matches!(
        context.phase,
        SessionPhase::Continuous | SessionPhase::Auction
    ) {
        return Err(CalendarError::InvalidCalendar(
            "bar open is outside a tradable session",
        ));
    }
    let window =
        calendar
            .session_window_containing(open_time_ms)?
            .ok_or(CalendarError::InvalidCalendar(
                "bar open has no session window",
            ))?;
    let step = interval_step_ms(interval).ok_or(CalendarError::InvalidCalendar(
        "calendar interval has no fixed step",
    ))?;
    if (open_time_ms - window.start_ms).rem_euclid(step) != 0 {
        return Err(CalendarError::InvalidCalendar(
            "bar open is not aligned to the session boundary",
        ));
    }
    Ok(())
}

fn detect_gaps(
    request: &CanonicalizationRequest,
    bars: &[OhlcvBar],
    quarantined_records: &[QuarantinedMarketRecord],
) -> Result<Vec<BarGap>, PipelineError> {
    let observed = bars
        .iter()
        .map(|bar| bar.open_time_ms)
        .chain(quarantined_records.iter().filter_map(|record| {
            let time = record.record.open_time_ms;
            (record.record.instrument == request.instrument
                && record.record.interval == request.interval
                && interval_aligned(time, request.interval)
                && chrono::DateTime::<chrono::Utc>::from_timestamp_millis(time).is_some()
                && request
                    .calendar
                    .is_expected_bar_time(&request.instrument, request.interval, time)
                    .is_ok())
            .then_some(time)
        }))
        .collect::<std::collections::BTreeSet<_>>();
    let mut gaps = Vec::new();
    let Some(start) = request
        .historical_range
        .map(|range| range.start_time_ms)
        .or_else(|| observed.first().copied())
    else {
        return Ok(gaps);
    };
    let Some(end) = request
        .historical_range
        .map(|range| range.end_time_ms)
        .or_else(|| {
            observed
                .last()
                .and_then(|time| next_expected_bar_open_time(request, *time).ok())
        })
    else {
        return Ok(gaps);
    };
    let venue_daily = is_daily_interval(request.interval)
        && matches!(&request.calendar, CalendarEvidence::Venue { .. });
    let Some(mut current) = (if venue_daily {
        let CalendarEvidence::Venue { snapshot } = &request.calendar else {
            unreachable!("venue_daily only matches venue calendars")
        };
        first_daily_open(snapshot, start, end)?
    } else {
        Some(start)
    }) else {
        return Ok(gaps);
    };
    let mut gap_start = None;
    let present = bars
        .iter()
        .map(|bar| bar.open_time_ms)
        .collect::<std::collections::BTreeSet<_>>();
    // ponytail: scan each expected interval; replace with venue calendar range
    // arithmetic if multi-year backfills make this measurable.
    while current < end {
        let next = next_expected_bar_open_time(request, current)
            .map_err(|error| PipelineError::InvalidRequest(error.to_string()))?;
        let missing = if venue_daily {
            !present.contains(&current)
        } else {
            !present.contains(&current) && !request.calendar.scheduled_non_trading(current)?
        };
        if missing {
            gap_start.get_or_insert(current);
        } else if let Some(start) = gap_start.take() {
            push_gap(&mut gaps, start, current);
        }
        current = next;
    }
    if let Some(start) = gap_start {
        push_gap(&mut gaps, start, current.min(end));
    }
    Ok(gaps)
}

fn push_gap(gaps: &mut Vec<BarGap>, start_time_ms: i64, end_time_ms: i64) {
    if start_time_ms >= end_time_ms {
        return;
    }
    if let Some(previous) = gaps.last_mut()
        && previous.end_time_ms == start_time_ms
    {
        previous.end_time_ms = end_time_ms;
    } else {
        gaps.push(BarGap {
            start_time_ms,
            end_time_ms,
        });
    }
}

fn coverage(request: &CanonicalizationRequest, bars: &[OhlcvBar], gaps: &[BarGap]) -> Coverage {
    let start_time_ms = request
        .historical_range
        .map(|range| range.start_time_ms)
        .or_else(|| bars.first().map(|bar| bar.open_time_ms));
    let end_time_ms = request
        .historical_range
        .map(|range| range.end_time_ms)
        .or_else(|| bars.last().map(|bar| bar.open_time_ms));
    Coverage {
        start_time_ms,
        end_time_ms,
        expected_record_count: bars.len()
            + gaps
                .iter()
                .map(|gap| gap_slots(*gap, request))
                .sum::<usize>(),
        canonical_record_count: bars.len(),
    }
}

fn gap_slots(gap: BarGap, request: &CanonicalizationRequest) -> usize {
    let mut count = 0;
    let mut current = gap.start_time_ms;
    while current < gap.end_time_ms {
        let Ok(next) = next_expected_bar_open_time(request, current) else {
            break;
        };
        count += 1;
        current = next;
    }
    count
}

fn is_daily_interval(interval: BarInterval) -> bool {
    matches!(
        interval,
        BarInterval::OneDay
            | BarInterval::TwoDays
            | BarInterval::ThreeDays
            | BarInterval::FiveDays
            | BarInterval::OneWeek
            | BarInterval::OneMonth
            | BarInterval::ThreeMonths
    )
}

fn daily_trading_day_step(interval: BarInterval) -> u32 {
    match interval {
        BarInterval::OneDay => 1,
        BarInterval::TwoDays => 2,
        BarInterval::ThreeDays => 3,
        BarInterval::FiveDays | BarInterval::OneWeek => 5,
        BarInterval::OneMonth => 21,
        BarInterval::ThreeMonths => 63,
        _ => 1,
    }
}

fn first_daily_open(
    calendar: &TradingCalendarSnapshot,
    start_time_ms: i64,
    end_time_ms: i64,
) -> Result<Option<i64>, PipelineError> {
    let mut date = calendar
        .trading_date_of(start_time_ms)
        .map_err(|error| PipelineError::InvalidRequest(error.to_string()))?;
    loop {
        if calendar
            .is_trading_day(date)
            .map_err(|error| PipelineError::InvalidRequest(error.to_string()))?
        {
            let open_time_ms = calendar
                .daily_boundary_open_ms(date)
                .map_err(|error| PipelineError::InvalidRequest(error.to_string()))?;
            if open_time_ms >= start_time_ms {
                return Ok((open_time_ms < end_time_ms).then_some(open_time_ms));
            }
        }
        date = calendar
            .next_trading_date(date)
            .map_err(|error| PipelineError::InvalidRequest(error.to_string()))?;
    }
}

fn next_expected_bar_open_time(
    request: &CanonicalizationRequest,
    current_time_ms: i64,
) -> Result<i64, PipelineError> {
    if !is_daily_interval(request.interval) {
        return next_bar_open_time_ms(current_time_ms, request.interval)
            .map_err(|error| PipelineError::InvalidRequest(error.to_string()));
    }
    let calendar = match &request.calendar {
        CalendarEvidence::Venue { snapshot } => snapshot,
        CalendarEvidence::UtcGrid { .. } => {
            return next_bar_open_time_ms(current_time_ms, request.interval)
                .map_err(|error| PipelineError::InvalidRequest(error.to_string()));
        }
    };
    let date = calendar
        .trading_date_of(current_time_ms)
        .map_err(|error| PipelineError::InvalidRequest(error.to_string()))?;
    let next_date = calendar
        .trading_date_offset(date, daily_trading_day_step(request.interval))
        .map_err(|error| PipelineError::InvalidRequest(error.to_string()))?;
    calendar
        .daily_boundary_open_ms(next_date)
        .map_err(|error| PipelineError::InvalidRequest(error.to_string()))
}

fn quality_state(
    source: &SourceMarketDataset,
    output: &CanonicalizationOutput,
) -> DataQualityState {
    if output.bars.is_empty() {
        DataQualityState::Rejected
    } else if output.duplicate_count > 0
        || output.conflict_count > 0
        || !output.quarantined_records.is_empty()
        || !output.gaps.is_empty()
        || !output.warnings.is_empty()
        || !source.identity.capability_snapshot.limitations.is_empty()
    {
        DataQualityState::Degraded
    } else {
        DataQualityState::Passed
    }
}

fn build_quality_report(
    source: &SourceMarketDataset,
    canonical_id: Option<&str>,
    output: &CanonicalizationOutput,
    evidence_path: PathBuf,
) -> DataQualityReport {
    let mut reasons = Vec::new();
    if output.duplicate_count > 0 {
        reasons.push(QualityReason::DuplicateCollapsed {
            count: output.duplicate_count,
        });
    }
    if output.conflict_count > 0 {
        reasons.push(QualityReason::ConflictingValues {
            count: output.conflict_count,
        });
    }
    if !output.quarantined_records.is_empty() {
        reasons.push(QualityReason::QuarantinedRecords {
            count: output.quarantined_records.len(),
        });
    }
    if !output.gaps.is_empty() {
        reasons.push(QualityReason::ExplicitGaps {
            count: output.gaps.len(),
        });
    }
    if !output.warnings.is_empty() {
        reasons.push(QualityReason::WarningRecords {
            count: output.warnings.len(),
        });
    }
    for limitation in &source.identity.capability_snapshot.limitations {
        reasons.push(QualityReason::CapabilityLimitation {
            detail: limitation.clone(),
        });
    }
    let state = if output.bars.is_empty() {
        DataQualityState::Rejected
    } else if !reasons.is_empty() {
        DataQualityState::Degraded
    } else {
        DataQualityState::Passed
    };
    DataQualityReport {
        report_id: report_id(source, canonical_id, output),
        source_id: source.source_id.clone(),
        canonical_id: canonical_id.map(str::to_owned),
        state,
        applied_rules: APPLIED_RULES.iter().map(|rule| (*rule).into()).collect(),
        coverage: output.coverage.clone(),
        duplicate_count: output.duplicate_count,
        conflict_count: output.conflict_count,
        quarantine_count: output.quarantined_records.len(),
        gap_count: output.gaps.len(),
        warning_count: output.warnings.len(),
        capability_limitations: source.identity.capability_snapshot.limitations.clone(),
        reasons,
        quarantined_records: output.quarantined_records.clone(),
        warnings: output.warnings.clone(),
        gaps: output.gaps.clone(),
        evidence_path,
    }
}

fn canonical_id(
    source: &SourceMarketDataset,
    request: &CanonicalizationRequest,
    output: &CanonicalizationOutput,
) -> String {
    digest(
        &canonical_json_bytes(&(
            &source.source_id,
            &request.instrument,
            request.interval,
            &request.normalization_contract,
            request.price_basis,
            &request.calendar,
            &output.bars,
            &output.row_evidence,
            &output.gaps,
        ))
        .expect("Canonical identity serializes"),
    )
}

fn report_id(
    source: &SourceMarketDataset,
    canonical_id: Option<&str>,
    output: &CanonicalizationOutput,
) -> String {
    digest(
        &canonical_json_bytes(&(
            &source.source_id,
            canonical_id,
            quality_state(source, output),
            output.duplicate_count,
            output.conflict_count,
            &output.quarantined_records,
            &output.warnings,
            &output.gaps,
        ))
        .expect("Data Quality Report identity serializes"),
    )
}

fn interval_step_ms(interval: BarInterval) -> Option<i64> {
    Some(match interval {
        BarInterval::OneSecond => 1_000,
        BarInterval::OneMinute => 60_000,
        BarInterval::ThreeMinutes => 180_000,
        BarInterval::FiveMinutes => 300_000,
        BarInterval::FifteenMinutes => 900_000,
        BarInterval::ThirtyMinutes => 1_800_000,
        BarInterval::OneHour => 3_600_000,
        BarInterval::TwoHours => 7_200_000,
        BarInterval::FourHours => 14_400_000,
        BarInterval::SixHours => 21_600_000,
        BarInterval::TwelveHours => 43_200_000,
        BarInterval::OneDay => 86_400_000,
        BarInterval::TwoDays => 172_800_000,
        BarInterval::ThreeDays => 259_200_000,
        BarInterval::FiveDays => 432_000_000,
        BarInterval::OneWeek => 604_800_000,
        BarInterval::OneMonth | BarInterval::ThreeMonths => return None,
    })
}

fn interval_aligned(open_time_ms: i64, interval: BarInterval) -> bool {
    if let Some(step) = interval_step_ms(interval) {
        open_time_ms.rem_euclid(step) == 0
    } else if let Some(utc) = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(open_time_ms) {
        utc.day() == 1
            && utc.hour() == 0
            && utc.minute() == 0
            && utc.second() == 0
            && utc.timestamp_subsec_millis() == 0
    } else {
        false
    }
}

fn payload_hash(records: &[SourceMarketRecord]) -> Result<String, PipelineError> {
    let mut values = records
        .iter()
        .map(|record| canonical_json_bytes(record))
        .collect::<Result<Vec<_>, _>>()?;
    values.sort();
    Ok(digest(&values.concat()))
}

fn source_evidence_bytes(records: &[SourceMarketRecord]) -> Result<Vec<u8>, PipelineError> {
    let mut bytes = Vec::new();
    for record in records {
        bytes.extend(canonical_json_bytes(record)?);
        bytes.push(b'\n');
    }
    Ok(bytes)
}

pub(crate) fn canonical_json_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, PipelineError> {
    let value = serde_json::to_value(value).map_err(storage)?;
    serde_json::to_vec(&sort_json(value)).map_err(storage)
}

fn sort_json(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(sort_json).collect()),
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .map(|(key, value)| (key, sort_json(value)))
                .collect(),
        ),
        value => value,
    }
}

pub(crate) fn digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let mut output = String::with_capacity(64);
    for byte in hasher.finalize() {
        write!(&mut output, "{byte:02x}").expect("writing hash cannot fail");
    }
    output
}

fn hash_file(path: &Path) -> Result<String, PipelineError> {
    let mut file = File::open(path).map_err(storage)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(storage)?;
    Ok(digest(&bytes))
}

pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), PipelineError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(storage)?;
    }
    if path.is_file() {
        if hash_file(path)? == digest(bytes) {
            return Ok(());
        }
        return Err(PipelineError::Storage(format!(
            "immutable evidence path already contains different content: {}",
            path.display()
        )));
    }
    let temporary = path.with_file_name(format!(
        ".{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("evidence")
    ));
    let result = (|| {
        let mut file = File::create(&temporary).map_err(storage)?;
        file.write_all(bytes).map_err(storage)?;
        file.sync_all().map_err(storage)?;
        fs::rename(&temporary, path).map_err(storage)?;
        if hash_file(path)? != digest(bytes) {
            return Err(PipelineError::Storage(
                "published evidence hash verification failed".into(),
            ));
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn write_parquet_atomic(
    path: &Path,
    bars: &[OhlcvBar],
    cancellation: &CancellationToken,
) -> Result<String, PipelineError> {
    if cancellation.is_cancelled() {
        return Err(PipelineError::Cancelled {
            source_id: "canonical-publication".into(),
        });
    }
    if path.is_file() {
        if read_parquet(path)? != bars {
            return Err(PipelineError::Storage(
                "immutable Canonical Parquet path already contains different rows".into(),
            ));
        }
        return hash_file(path);
    }
    let temporary = path.with_file_name(format!(
        ".{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("canonical.parquet")
    ));
    let result = (|| {
        let file = File::create(&temporary).map_err(storage)?;
        let sync_file = file.try_clone().map_err(storage)?;
        let batch = RecordBatch::try_new(
            snapshot_schema(),
            vec![
                Arc::new(Int64Array::from_iter_values(
                    bars.iter().map(|bar| bar.open_time_ms),
                )),
                string_column(bars, |bar| bar.open),
                string_column(bars, |bar| bar.high),
                string_column(bars, |bar| bar.low),
                string_column(bars, |bar| bar.close),
                string_column(bars, |bar| bar.base_volume),
                string_column(bars, |bar| bar.quote_volume),
            ],
        )
        .map_err(storage)?;
        let mut writer = ArrowWriter::try_new(file, batch.schema(), None).map_err(storage)?;
        writer.write(&batch).map_err(storage)?;
        writer.close().map_err(storage)?;
        sync_file.sync_all().map_err(storage)?;
        if cancellation.is_cancelled() {
            return Err(PipelineError::Cancelled {
                source_id: "canonical-publication".into(),
            });
        }
        fs::rename(&temporary, path).map_err(storage)?;
        hash_file(path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn snapshot_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("open_time_ms", DataType::Int64, false),
        Field::new("open", DataType::Utf8, false),
        Field::new("high", DataType::Utf8, false),
        Field::new("low", DataType::Utf8, false),
        Field::new("close", DataType::Utf8, false),
        Field::new("base_volume", DataType::Utf8, false),
        Field::new("quote_volume", DataType::Utf8, false),
    ]))
}

fn string_column(bars: &[OhlcvBar], value: impl Fn(&OhlcvBar) -> Decimal) -> Arc<StringArray> {
    Arc::new(StringArray::from_iter_values(
        bars.iter().map(|bar| value(bar).to_string()),
    ))
}

fn read_parquet(path: &Path) -> Result<Vec<OhlcvBar>, PipelineError> {
    let file = File::open(path).map_err(storage)?;
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(storage)?
        .with_batch_size(8192)
        .build()
        .map_err(storage)?;
    let mut bars = Vec::new();
    for batch in reader {
        let batch = batch.map_err(storage)?;
        let times = column::<Int64Array>(&batch, 0)?;
        let open = column::<StringArray>(&batch, 1)?;
        let high = column::<StringArray>(&batch, 2)?;
        let low = column::<StringArray>(&batch, 3)?;
        let close = column::<StringArray>(&batch, 4)?;
        let base_volume = column::<StringArray>(&batch, 5)?;
        let quote_volume = column::<StringArray>(&batch, 6)?;
        for index in 0..batch.num_rows() {
            bars.push(OhlcvBar {
                open_time_ms: times.value(index),
                open: Decimal::from_str(open.value(index)).map_err(storage)?,
                high: Decimal::from_str(high.value(index)).map_err(storage)?,
                low: Decimal::from_str(low.value(index)).map_err(storage)?,
                close: Decimal::from_str(close.value(index)).map_err(storage)?,
                base_volume: Decimal::from_str(base_volume.value(index)).map_err(storage)?,
                quote_volume: Decimal::from_str(quote_volume.value(index)).map_err(storage)?,
            });
        }
    }
    Ok(bars)
}

fn column<T: Array + 'static>(batch: &RecordBatch, index: usize) -> Result<&T, PipelineError> {
    batch
        .column(index)
        .as_any()
        .downcast_ref::<T>()
        .ok_or_else(|| PipelineError::Storage("Canonical Parquet schema is invalid".into()))
}

fn read_json_lines<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Vec<T>, PipelineError> {
    let file = File::open(path).map_err(storage)?;
    let mut values = Vec::new();
    for line in BufReader::new(file).lines() {
        let line = line.map_err(storage)?;
        if !line.trim().is_empty() {
            values.push(serde_json::from_str(&line).map_err(storage)?);
        }
    }
    Ok(values)
}

fn references_for(
    database: &Connection,
    user_id: &str,
    evidence_kind: &str,
    evidence_id: &str,
) -> Result<Vec<BlockingReference>, PipelineError> {
    let mut statement = database
        .prepare(
            "SELECT consumer_kind, consumer_id FROM pipeline_references
             WHERE user_id = ?1 AND evidence_kind = ?2 AND evidence_id = ?3
             ORDER BY consumer_kind, consumer_id",
        )
        .map_err(storage)?;
    statement
        .query_map(params![user_id, evidence_kind, evidence_id], |row| {
            Ok(BlockingReference {
                consumer_kind: row.get(0)?,
                consumer_id: row.get(1)?,
            })
        })
        .map_err(storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)
}

fn append_snapshot_table_blockers(
    database: &Connection,
    user_id: &str,
    snapshot_id: &str,
    table: &str,
    query: &str,
    consumer_kind: &str,
    blockers: &mut Vec<BlockingReference>,
) -> Result<(), PipelineError> {
    let table_exists: bool = database
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1
             )",
            [table],
            |row| row.get(0),
        )
        .map_err(storage)?;
    if !table_exists {
        return Ok(());
    }
    let mut statement = database.prepare(query).map_err(storage)?;
    let references = statement
        .query_map(params![user_id, snapshot_id], |row| {
            Ok(BlockingReference {
                consumer_kind: consumer_kind.into(),
                consumer_id: row.get(0)?,
            })
        })
        .map_err(storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)?;
    blockers.extend(references);
    Ok(())
}

fn storage(error: impl std::fmt::Display) -> PipelineError {
    PipelineError::Storage(error.to_string())
}

fn lock_error(error: impl std::fmt::Display) -> PipelineError {
    PipelineError::Storage(format!("pipeline database lock failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use adaq_data_core::market::Venue;
    use tempfile::tempdir;

    fn instrument() -> InstrumentId {
        InstrumentId::new(Venue::crypto_spot("okx").unwrap(), "BTC-USDT").unwrap()
    }

    fn bar(time: i64, close: &str) -> OhlcvBar {
        let value = Decimal::from_str(close).unwrap();
        OhlcvBar {
            open_time_ms: time,
            open: value,
            high: value,
            low: value,
            close: value,
            base_volume: Decimal::ONE,
            quote_volume: value,
        }
    }

    fn source(records: Vec<SourceMarketRecord>) -> SourceMarketDataset {
        let acquisition = SourceAcquisition {
            records,
            ..SourceAcquisition::default()
        };
        let bytes = source_evidence_bytes(&acquisition.records).unwrap();
        SourceMarketDataset {
            source_id: "source".into(),
            revision: 1,
            logical_key: "logical".into(),
            identity: SourceIdentity {
                provider: acquisition.provider,
                actual_upstream: acquisition.actual_upstream,
                connector: acquisition.connector,
                connector_version: acquisition.connector_version,
                request_parameters: acquisition.request_parameters,
                retrieved_at_ms: acquisition.retrieved_at_ms,
                response_sha256s: acquisition.response_sha256s,
                acquisition_content_sha256: acquisition.acquisition_content_sha256,
                payload_sha256: payload_hash(&acquisition.records).unwrap(),
                content_sha256: digest(&bytes),
                capability_snapshot: acquisition.capability_snapshot,
                acquisition_diagnostics: acquisition.acquisition_diagnostics,
                price_basis: acquisition.price_basis,
            },
            records: acquisition.records,
            evidence_path: PathBuf::from("source.jsonl"),
        }
    }

    fn request() -> CanonicalizationRequest {
        CanonicalizationRequest::new(
            instrument(),
            BarInterval::OneMinute,
            CalendarEvidence::UtcGrid {
                calendar_id: "utc-grid".into(),
                closures: Vec::new(),
            },
        )
        .unwrap()
    }

    fn acquisition(times: &[i64]) -> SourceAcquisition {
        let instrument = instrument();
        SourceAcquisition {
            records: times
                .iter()
                .map(|time| {
                    SourceMarketRecord::from_bar(
                        instrument.clone(),
                        BarInterval::OneMinute,
                        "BTC-USDT",
                        &bar(*time, "10"),
                    )
                })
                .collect(),
            ..SourceAcquisition::default()
        }
    }

    #[test]
    fn first_publish_is_content_addressed_and_round_trips_both_evidence_layers() {
        let directory = tempdir().unwrap();
        let database = Arc::new(Mutex::new(
            Connection::open(directory.path().join("pipeline.sqlite")).unwrap(),
        ));
        let pipeline = DataPipeline::open(directory.path(), database).unwrap();
        let publication = pipeline
            .publish(
                "alice",
                acquisition(&[0, 60_000]),
                request(),
                CancellationToken::new(),
                |_| {},
            )
            .unwrap();
        let canonical = publication.canonical.as_ref().unwrap();
        assert_eq!(publication.quality.state, DataQualityState::Passed);
        assert_eq!(
            publication
                .source
                .evidence_path
                .extension()
                .and_then(|value| value.to_str()),
            Some("jsonl")
        );
        assert_eq!(
            canonical
                .parquet_path
                .extension()
                .and_then(|value| value.to_str()),
            Some("parquet")
        );
        assert!(canonical.parquet_path.with_extension("json").is_file());
        assert_eq!(
            pipeline
                .source_for_user("alice", &publication.source.source_id)
                .unwrap(),
            publication.source
        );
        let loaded = pipeline
            .canonical_for_user("alice", &canonical.canonical_id)
            .unwrap();
        assert_eq!(loaded.bars, canonical.bars);
        assert_eq!(loaded.row_evidence, canonical.row_evidence);
        assert_eq!(pipeline.list("bob").unwrap(), Vec::new());
        assert!(
            pipeline
                .source_for_user("bob", &publication.source.source_id)
                .is_err()
        );
        assert!(
            pipeline
                .canonical_for_user("bob", &canonical.canonical_id)
                .is_err()
        );
        assert!(
            pipeline
                .quality_for_user("bob", &publication.quality.report_id)
                .is_err()
        );
    }

    #[test]
    fn reset_user_rows_removes_entitlements_without_deleting_shared_catalogs() {
        let directory = tempdir().unwrap();
        let pipeline = DataPipeline::open(
            directory.path(),
            Arc::new(Mutex::new(
                Connection::open(directory.path().join("pipeline.sqlite")).unwrap(),
            )),
        )
        .unwrap();
        let alice = pipeline
            .publish(
                "alice",
                acquisition(&[0, 60_000]),
                request(),
                CancellationToken::new(),
                |_| {},
            )
            .unwrap();
        let bob = pipeline
            .publish(
                "bob",
                acquisition(&[0, 60_000]),
                request(),
                CancellationToken::new(),
                |_| {},
            )
            .unwrap();
        assert_eq!(alice.source.source_id, bob.source.source_id);
        assert!(pipeline.reset_paths_for_user("alice").unwrap().is_empty());

        let database = pipeline.database();
        let mut database = database.lock().unwrap();
        let transaction = database.transaction().unwrap();
        pipeline.reset_user_rows(&transaction, "alice").unwrap();
        transaction.commit().unwrap();
        drop(database);

        assert!(pipeline.list("alice").unwrap().is_empty());
        assert_eq!(pipeline.list("bob").unwrap().len(), 1);
        assert!(alice.source.evidence_path.is_file());
        assert!(alice.canonical.unwrap().parquet_path.is_file());
    }

    #[test]
    fn identical_retry_reuses_the_published_source_revision() {
        let directory = tempdir().unwrap();
        let pipeline = DataPipeline::open(
            directory.path(),
            Arc::new(Mutex::new(
                Connection::open(directory.path().join("pipeline.sqlite")).unwrap(),
            )),
        )
        .unwrap();
        let first = pipeline
            .publish(
                "alice",
                acquisition(&[0]),
                request(),
                CancellationToken::new(),
                |_| {},
            )
            .unwrap();
        let retry = pipeline
            .publish(
                "alice",
                acquisition(&[0]),
                request(),
                CancellationToken::new(),
                |_| {},
            )
            .unwrap();
        assert_eq!(retry.source.source_id, first.source.source_id);
        assert_eq!(retry.source.revision, first.source.revision);
        assert_eq!(pipeline.list("alice").unwrap().len(), 1);

        let cancellation = CancellationToken::new();
        cancellation.cancel();
        assert!(matches!(
            pipeline.publish_without_partial_source(
                "alice",
                acquisition(&[0]),
                request(),
                cancellation,
                |_| {},
            ),
            Err(PipelineError::Cancelled { .. })
        ));
        assert_eq!(pipeline.list("alice").unwrap().len(), 1);
    }

    #[test]
    fn a_share_style_failure_revokes_source_access_before_catalog_cutover() {
        let directory = tempdir().unwrap();
        let pipeline = DataPipeline::open(
            directory.path(),
            Arc::new(Mutex::new(
                Connection::open(directory.path().join("pipeline.sqlite")).unwrap(),
            )),
        )
        .unwrap();
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        assert!(matches!(
            pipeline.publish_without_partial_source(
                "alice",
                acquisition(&[0]),
                request(),
                cancellation,
                |_| {},
            ),
            Err(PipelineError::Cancelled { .. })
        ));
        assert!(pipeline.list("alice").unwrap().is_empty());
    }

    #[test]
    fn canonicalization_collapses_identical_duplicates_and_preserves_gaps() {
        let mut acquisition = acquisition(&[0, 0, 120_000]);
        acquisition.records.push(SourceMarketRecord::from_bar(
            instrument(),
            BarInterval::OneMinute,
            "BTC-USDT",
            &bar(0, "10"),
        ));
        let source = source(acquisition.records);
        let mut request = request();
        request.historical_range = Some(HistoricalBarRange {
            start_time_ms: 0,
            end_time_ms: 180_000,
        });
        let preview = canonicalize(&source, &request).unwrap();
        assert_eq!(preview.bars.len(), 2);
        assert_eq!(preview.duplicate_count, 2);
        assert_eq!(
            preview.gaps,
            vec![BarGap {
                start_time_ms: 60_000,
                end_time_ms: 120_000
            }]
        );
    }

    #[test]
    fn empty_requested_history_is_an_explicit_gap() {
        let mut request = request();
        request.historical_range = Some(HistoricalBarRange {
            start_time_ms: 0,
            end_time_ms: 180_000,
        });
        let preview = canonicalize(&source(Vec::new()), &request).unwrap();
        assert_eq!(
            preview.gaps,
            vec![BarGap {
                start_time_ms: 0,
                end_time_ms: 180_000,
            }]
        );
    }

    #[test]
    fn conflicting_values_are_quarantined_instead_of_winning_by_input_order() {
        let mut records = vec![
            SourceMarketRecord::from_bar(
                instrument(),
                BarInterval::OneMinute,
                "BTC-USDT",
                &bar(0, "10"),
            ),
            SourceMarketRecord::from_bar(
                instrument(),
                BarInterval::OneMinute,
                "BTC-USDT",
                &bar(0, "11"),
            ),
            SourceMarketRecord::from_bar(
                instrument(),
                BarInterval::OneMinute,
                "BTC-USDT",
                &bar(60_000, "12"),
            ),
        ];
        records.reverse();
        let preview = canonicalize(&source(records), &request()).unwrap();
        assert_eq!(preview.bars.len(), 1);
        assert_eq!(preview.conflict_count, 2);
        assert!(matches!(
            preview.quarantined_records[0].reason,
            QuarantineReason::ConflictingIdentityOrValue { .. }
        ));
    }

    #[test]
    fn malformed_values_and_invariants_have_typed_quarantine_reasons() {
        let mut invalid = SourceMarketRecord::from_bar(
            instrument(),
            BarInterval::OneMinute,
            "BTC-USDT",
            &bar(0, "10"),
        );
        invalid.close = Some("not-a-decimal".into());
        let mut invariant = SourceMarketRecord::from_bar(
            instrument(),
            BarInterval::OneMinute,
            "BTC-USDT",
            &bar(60_000, "10"),
        );
        invariant.low = Some("11".into());
        let mut invalid_time = SourceMarketRecord::from_bar(
            instrument(),
            BarInterval::OneMinute,
            "BTC-USDT",
            &bar(120_000, "10"),
        );
        invalid_time.open_time_ms = i64::MAX;
        let preview =
            canonicalize(&source(vec![invalid, invariant, invalid_time]), &request()).unwrap();
        assert_eq!(preview.quarantined_records.len(), 3);
        assert!(
            preview.quarantined_records.iter().any(|record| matches!(
                record.reason,
                QuarantineReason::UnparsableExactValue { .. }
            ))
        );
        assert!(preview.quarantined_records.iter().any(|record| matches!(
            record.reason,
            QuarantineReason::InvalidFinancialInvariant { .. }
        )));
        assert!(
            preview
                .quarantined_records
                .iter()
                .any(|record| matches!(record.reason, QuarantineReason::InvalidUtcInstant))
        );
    }

    #[test]
    fn capability_limitations_and_unusual_valid_values_degrade_without_deletion() {
        let mut acquisition = acquisition(&[0]);
        acquisition
            .capability_snapshot
            .limitations
            .push("history is delayed".into());
        acquisition.records[0].base_volume = Some("0".into());
        acquisition.records[0].quote_volume = Some("0".into());
        let directory = tempdir().unwrap();
        let pipeline = DataPipeline::open(
            directory.path(),
            Arc::new(Mutex::new(
                Connection::open(directory.path().join("pipeline.sqlite")).unwrap(),
            )),
        )
        .unwrap();
        let publication = pipeline
            .publish(
                "alice",
                acquisition,
                request(),
                CancellationToken::new(),
                |_| {},
            )
            .unwrap();
        assert_eq!(publication.quality.state, DataQualityState::Degraded);
        assert_eq!(publication.canonical.unwrap().bars.len(), 1);
        assert_eq!(publication.quality.warning_count, 1);
        assert_eq!(
            publication.quality.capability_limitations,
            vec!["history is delayed"]
        );
    }

    #[test]
    fn rejected_quality_never_publishes_canonical_dataset() {
        let mut invalid = SourceMarketRecord::from_bar(
            instrument(),
            BarInterval::OneMinute,
            "BTC-USDT",
            &bar(0, "10"),
        );
        invalid.open = None;
        let directory = tempdir().unwrap();
        let pipeline = DataPipeline::open(
            directory.path(),
            Arc::new(Mutex::new(
                Connection::open(directory.path().join("pipeline.sqlite")).unwrap(),
            )),
        )
        .unwrap();
        let publication = pipeline
            .publish(
                "alice",
                SourceAcquisition {
                    records: vec![invalid],
                    ..SourceAcquisition::default()
                },
                request(),
                CancellationToken::new(),
                |_| {},
            )
            .unwrap();
        assert_eq!(publication.quality.state, DataQualityState::Rejected);
        assert!(publication.canonical.is_none());
        assert_eq!(pipeline.list("alice").unwrap()[0].canonical_id, None);
    }

    #[test]
    fn revisions_are_append_only_and_snapshot_references_lock_deletion() {
        let directory = tempdir().unwrap();
        let pipeline = DataPipeline::open(
            directory.path(),
            Arc::new(Mutex::new(
                Connection::open(directory.path().join("pipeline.sqlite")).unwrap(),
            )),
        )
        .unwrap();
        let first = pipeline
            .publish(
                "alice",
                acquisition(&[0]),
                request(),
                CancellationToken::new(),
                |_| {},
            )
            .unwrap();
        let mut second_acquisition = acquisition(&[0]);
        second_acquisition.retrieved_at_ms = 2;
        second_acquisition.records[0].close = Some("11".into());
        let second = pipeline
            .publish(
                "alice",
                second_acquisition,
                request(),
                CancellationToken::new(),
                |_| {},
            )
            .unwrap();
        assert_eq!(second.source.revision, first.source.revision + 1);
        assert_ne!(second.source.source_id, first.source.source_id);
        let first_canonical_id = first.canonical.as_ref().unwrap().canonical_id.clone();
        pipeline
            .record_snapshot_reference("alice", &first_canonical_id, "snapshot-1")
            .unwrap();
        pipeline
            .record_reference("alice", "snapshot", "snapshot-1", "run", "run-1")
            .unwrap();
        assert!(matches!(
            pipeline.ensure_snapshot_deletable("alice", "snapshot-1"),
            Err(PipelineError::DeletionBlocked { .. })
        ));
        let error = pipeline
            .delete_canonical_for_user("alice", &first_canonical_id)
            .unwrap_err();
        assert!(matches!(error, PipelineError::DeletionBlocked { .. }));
    }

    #[test]
    fn cancellation_retains_source_evidence_and_emits_typed_progress() {
        let directory = tempdir().unwrap();
        let pipeline = DataPipeline::open(
            directory.path(),
            Arc::new(Mutex::new(
                Connection::open(directory.path().join("pipeline.sqlite")).unwrap(),
            )),
        )
        .unwrap();
        let token = CancellationToken::new();
        token.cancel();
        let mut events = Vec::new();
        let error = pipeline
            .publish("alice", acquisition(&[0]), request(), token, |event| {
                events.push(event)
            })
            .unwrap_err();
        let source_id = match error {
            PipelineError::Cancelled { source_id } => source_id,
            other => panic!("unexpected error: {other:?}"),
        };
        assert!(
            events
                .iter()
                .any(|event| matches!(event, PipelineProgress::Cancelled { .. }))
        );
        assert!(pipeline.source_for_user("alice", &source_id).is_ok());
        assert_eq!(pipeline.failures_for_user("bob").unwrap(), Vec::new());
        assert_eq!(pipeline.failures_for_user("alice").unwrap(), Vec::new());
    }

    #[test]
    fn failed_parquet_publication_leaves_no_partial_file() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("canonical.parquet");
        fs::create_dir(&path).unwrap();
        let error =
            write_parquet_atomic(&path, &[bar(0, "10")], &CancellationToken::new()).unwrap_err();
        assert!(matches!(error, PipelineError::Storage(_)));
        assert!(path.is_dir());
        assert!(!directory.path().join(".canonical.parquet.tmp").exists());
    }

    #[test]
    fn canonical_identity_is_stable_when_source_records_are_reordered() {
        let first = source(acquisition(&[0, 60_000]).records);
        let mut reordered = acquisition(&[60_000, 0]).records;
        reordered.reverse();
        let second = source(reordered);
        let first_preview = canonicalize(&first, &request()).unwrap();
        let second_preview = canonicalize(&second, &request()).unwrap();
        assert_eq!(first_preview.bars, second_preview.bars);
        assert_eq!(first_preview.row_evidence, second_preview.row_evidence);
    }

    #[test]
    fn no_transformation_changes_canonical_values() {
        let mut record = SourceMarketRecord::from_bar(
            instrument(),
            BarInterval::OneMinute,
            "BTC-USDT",
            &bar(0, "10.123456789"),
        );
        record.base_volume = Some("0.000000001".into());
        let preview = canonicalize(&source(vec![record]), &request()).unwrap();
        assert_eq!(preview.bars[0].close.to_string(), "10.123456789");
        assert_eq!(preview.bars[0].base_volume.to_string(), "0.000000001");
    }

    #[test]
    fn canonical_rows_adapt_to_the_existing_snapshot_store_contract() {
        let directory = tempdir().unwrap();
        let pipeline = DataPipeline::open(
            directory.path().join("pipeline"),
            Arc::new(Mutex::new(
                Connection::open(directory.path().join("pipeline.sqlite")).unwrap(),
            )),
        )
        .unwrap();
        let publication = pipeline
            .publish(
                "alice",
                acquisition(&[0, 60_000]),
                request(),
                CancellationToken::new(),
                |_| {},
            )
            .unwrap();
        let canonical = publication.canonical.unwrap();
        let store =
            adaq_backtest_core::SnapshotStore::new(directory.path().join("snapshots")).unwrap();
        let snapshot = store.persist(&canonical.to_bar_series()).unwrap();
        assert_eq!(snapshot.bar_count, canonical.bars.len());
        assert_eq!(store.read(&snapshot).unwrap(), canonical.bars);
    }

    #[test]
    fn identity_mismatches_are_retained_as_quarantine_evidence() {
        let other_instrument = InstrumentId::new(
            adaq_data_core::market::Venue::crypto_spot("okx").unwrap(),
            "ETH-USDT",
        )
        .unwrap();
        let record = SourceMarketRecord::from_bar(
            other_instrument,
            BarInterval::OneMinute,
            "ETH-USDT",
            &bar(0, "10"),
        );
        let preview = canonicalize(&source(vec![record]), &request()).unwrap();
        assert!(matches!(
            preview.quarantined_records[0].reason,
            QuarantineReason::UnsupportedIdentity { .. }
        ));
    }

    #[test]
    fn venue_calendar_rejects_breaks_but_not_valid_session_bars() {
        let venue = adaq_data_core::market::Venue::china_a_share("sse").unwrap();
        let calendar = TradingCalendarSnapshot::new(
            "sse-2024",
            venue.clone(),
            0,
            2_000_000_000_000,
            vec![
                adaq_data_core::market::TradingSession {
                    phase: SessionPhase::Continuous,
                    start_local: chrono::NaiveTime::from_hms_opt(9, 30, 0).unwrap(),
                    end_local: chrono::NaiveTime::from_hms_opt(11, 30, 0).unwrap(),
                },
                adaq_data_core::market::TradingSession {
                    phase: SessionPhase::Break,
                    start_local: chrono::NaiveTime::from_hms_opt(11, 30, 0).unwrap(),
                    end_local: chrono::NaiveTime::from_hms_opt(13, 0, 0).unwrap(),
                },
                adaq_data_core::market::TradingSession {
                    phase: SessionPhase::Continuous,
                    start_local: chrono::NaiveTime::from_hms_opt(13, 0, 0).unwrap(),
                    end_local: chrono::NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
                },
            ],
            Vec::new(),
        )
        .unwrap();
        let instrument = InstrumentId::new(venue, "600000").unwrap();
        let mut request = CanonicalizationRequest::new(
            instrument.clone(),
            BarInterval::OneMinute,
            CalendarEvidence::Venue { snapshot: calendar },
        )
        .unwrap();
        let valid_time = 1_710_120_600_000i64;
        let break_time = valid_time + 7_200_000;
        let records = vec![
            SourceMarketRecord::from_bar(
                instrument.clone(),
                BarInterval::OneMinute,
                "600000",
                &bar(valid_time, "10"),
            ),
            SourceMarketRecord::from_bar(
                instrument,
                BarInterval::OneMinute,
                "600000",
                &bar(break_time, "10"),
            ),
        ];
        request.historical_range = None;
        let preview = canonicalize(&source(records), &request).unwrap();
        assert_eq!(preview.bars.len(), 1);
        assert_eq!(preview.quarantined_records.len(), 1);
    }
}
