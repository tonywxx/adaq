//! Tauri-independent China A-share acquisition through the pinned akshare
//! (akshare-rs) client.

use std::{
    collections::HashMap,
    future::Future,
    str::FromStr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use akshare::AkShareClient;
use chrono::{Datelike, NaiveDate, NaiveTime};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::market::{
    DayEvidence, DayKind, InstrumentId, InstrumentSourceMapping, LocalTimeDisambiguation,
    PriceBasis, ScheduledClosure, ScheduledClosureKind, SessionPhase, TradingCalendarSnapshot,
    TradingDate, TradingSession, Venue, VenueKind,
};
use crate::{BarInterval, DataError, HistoricalBarRange, InstrumentStatus};

pub const ASHARE_SRC: &str = "akshare-rs";
pub const ASHARE_CONNECTOR_VERSION: &str = "adaq-data-core-akshare-v1";
pub const ASHARE_RAW_WIRE_ADAPTER_VERSION: &str = "adaq-data-core-raw-wire-v1";

const SINA_UPSTREAM: &str = "Sina Finance";
const EASTMONEY_UPSTREAM: &str = "Eastmoney";
const MAX_CORPORATE_ACTION_PAGES: u64 = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AshareRequestPolicy {
    pub max_attempts: u8,
    pub timeout_ms: u64,
    pub retry_delay_ms: u64,
}

impl Default for AshareRequestPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            timeout_ms: 30_000,
            retry_delay_ms: 250,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AshareRequestDiagnostics {
    pub request_count: u32,
    pub retry_count: u32,
    #[serde(default)]
    pub response_statuses: Vec<u16>,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AshareInstrument {
    pub instrument: InstrumentId,
    pub provider_symbol: String,
    pub name: Option<String>,
    pub status: InstrumentStatus,
    pub listing_time_ms: Option<i64>,
    pub continuous_trading_time_ms: Option<i64>,
    #[serde(default)]
    pub current_price: Option<String>,
    #[serde(default)]
    pub current_base_volume: Option<String>,
    #[serde(default)]
    pub current_quote_volume: Option<String>,
    #[serde(default)]
    pub current_observed_at_ms: Option<i64>,
    pub mapping: InstrumentSourceMapping,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AshareInstrumentMasterAcquisition {
    pub provider: String,
    pub actual_upstream: String,
    pub method: String,
    pub connector_version: String,
    pub request_parameters: Value,
    pub retrieved_at_ms: i64,
    pub response_sha256: String,
    /// Hash of the parsed akshare-rs response, retained separately from the
    /// exact raw-wire response because the two adapters have different trust
    /// boundaries.
    pub parsed_response_sha256: String,
    #[serde(skip)]
    pub parsed_response: Option<Vec<u8>>,
    pub content_sha256: String,
    #[serde(default)]
    pub raw_response: Option<Vec<u8>>,
    pub diagnostics: AshareRequestDiagnostics,
    pub instruments: Vec<AshareInstrument>,
    #[serde(default)]
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AshareBar {
    pub instrument: InstrumentId,
    pub provider_symbol: String,
    pub interval: BarInterval,
    pub open_time_ms: i64,
    pub open: Option<String>,
    pub high: Option<String>,
    pub low: Option<String>,
    pub close: Option<String>,
    pub base_volume: Option<String>,
    pub quote_volume: Option<String>,
    pub price_basis: PriceBasis,
    pub raw_payload: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AshareBarsAcquisition {
    pub provider: String,
    pub actual_upstream: String,
    pub method: String,
    pub connector_version: String,
    pub request_parameters: Value,
    pub retrieved_at_ms: i64,
    pub response_sha256s: Vec<String>,
    pub content_sha256: String,
    #[serde(default)]
    pub raw_responses: Vec<Vec<u8>>,
    pub diagnostics: AshareRequestDiagnostics,
    pub bars: Vec<AshareBar>,
    #[serde(default)]
    pub invalid_bars: Vec<AshareBar>,
    #[serde(default)]
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AshareCalendarAcquisition {
    pub provider: String,
    pub actual_upstream: String,
    pub method: String,
    pub connector_version: String,
    pub request_parameters: Value,
    pub retrieved_at_ms: i64,
    pub response_sha256: String,
    pub content_sha256: String,
    #[serde(default)]
    pub raw_response: Option<Vec<u8>>,
    pub diagnostics: AshareRequestDiagnostics,
    pub snapshots: Vec<TradingCalendarSnapshot>,
    #[serde(default)]
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AshareCorporateActionKind {
    CashDividend,
    ShareDistribution,
    CashAndShareDistribution,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AshareCorporateAction {
    pub instrument: InstrumentId,
    pub provider_symbol: String,
    pub kind: AshareCorporateActionKind,
    pub effective_at_ms: Option<i64>,
    pub announced_at_ms: Option<i64>,
    pub available_at_ms: i64,
    pub cash_per_share: Option<String>,
    pub shares_per_share: Option<String>,
    pub raw_payload: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AshareCorporateActionAcquisition {
    pub instrument: InstrumentId,
    pub provider: String,
    pub actual_upstream: String,
    pub method: String,
    pub connector_version: String,
    pub request_parameters: Value,
    pub retrieved_at_ms: i64,
    pub response_sha256: String,
    pub content_sha256: String,
    #[serde(default)]
    pub raw_response: Option<Vec<u8>>,
    pub diagnostics: AshareRequestDiagnostics,
    pub records: Vec<AshareCorporateAction>,
    #[serde(default)]
    pub invalid_records: Vec<AshareCorporateAction>,
    #[serde(default)]
    pub limitations: Vec<String>,
}

#[derive(Clone)]
pub struct AshareClient {
    client: AkShareClient,
    raw_http: reqwest::Client,
    mock_uri: Option<String>,
    policy: AshareRequestPolicy,
}

#[derive(Debug, Clone)]
struct RawResponse {
    status: u16,
    bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
struct RawDailyCandle {
    date: String,
    open: Option<String>,
    high: Option<String>,
    low: Option<String>,
    close: Option<String>,
    volume: Option<String>,
    amount: Option<String>,
    raw_payload: Value,
}

#[derive(Debug, Clone)]
struct RawMinuteCandle {
    datetime: Option<String>,
    open: Option<String>,
    high: Option<String>,
    low: Option<String>,
    close: Option<String>,
    volume: Option<String>,
    amount: Option<String>,
    raw_payload: Value,
}

#[derive(Debug, Clone, Deserialize)]
struct RawSpotQuote {
    symbol: String,
    #[serde(default)]
    trade: Option<String>,
    #[serde(default)]
    volume: Option<String>,
    #[serde(default)]
    amount: Option<String>,
}

impl Default for AshareClient {
    fn default() -> Self {
        Self::new()
    }
}

impl AshareClient {
    pub fn new() -> Self {
        Self::with_policy(AshareRequestPolicy::default())
    }

    pub fn with_mock(mock_uri: impl Into<String>) -> Self {
        let mock_uri = mock_uri.into();
        Self {
            client: AkShareClient::with_mock(mock_uri.clone()),
            raw_http: raw_http_client(AshareRequestPolicy::default()),
            mock_uri: Some(mock_uri),
            policy: AshareRequestPolicy::default(),
        }
    }

    pub fn with_policy(policy: AshareRequestPolicy) -> Self {
        let client = AkShareClient::builder()
            .timeout(Duration::from_millis(policy.timeout_ms.max(1)))
            .build();
        Self {
            client,
            raw_http: raw_http_client(policy),
            mock_uri: None,
            policy,
        }
    }

    pub fn with_mock_and_policy(mock_uri: impl Into<String>, policy: AshareRequestPolicy) -> Self {
        let mock_uri = mock_uri.into();
        Self {
            client: AkShareClient::with_mock(mock_uri.clone()),
            raw_http: raw_http_client(policy),
            mock_uri: Some(mock_uri),
            policy,
        }
    }

    pub fn connector_version(&self) -> &'static str {
        ASHARE_CONNECTOR_VERSION
    }

    pub async fn acquire_instrument_master(
        &self,
    ) -> Result<AshareInstrumentMasterAcquisition, DataError> {
        self.acquire_instrument_master_at_with_cancel(now_ms(), || false)
            .await
    }

    pub async fn acquire_instrument_master_at(
        &self,
        retrieved_at_ms: i64,
    ) -> Result<AshareInstrumentMasterAcquisition, DataError> {
        self.acquire_instrument_master_at_with_cancel(retrieved_at_ms, || false)
            .await
    }

    pub async fn acquire_instrument_master_at_with_cancel<F>(
        &self,
        retrieved_at_ms: i64,
        is_cancelled: F,
    ) -> Result<AshareInstrumentMasterAcquisition, DataError>
    where
        F: Fn() -> bool,
    {
        if retrieved_at_ms < 0 {
            return Err(error(
                "invalid_request",
                "retrieval time must be non-negative",
            ));
        }
        let (quotes, mut diagnostics) = self
            .retry_with_cancel(|| self.client.stock_zh_a_spot(), &is_cancelled)
            .await?;
        let parsed_response = serde_json::to_vec(&quotes)
            .map_err(|value| error("serialization", value.to_string()))?;
        let parsed_response_sha256 = sha256(&parsed_response);
        diagnostics.notes.push(
            "akshare-rs 0.1.14 does not expose transport statuses for parsed requests; raw-wire statuses are recorded separately"
                .into(),
        );
        let (raw_response, raw_diagnostics) = self
            .retry_raw(|| self.fetch_spot_raw(), &is_cancelled)
            .await?;
        diagnostics.request_count += raw_diagnostics.request_count;
        diagnostics.retry_count += raw_diagnostics.retry_count;
        diagnostics
            .response_statuses
            .extend(raw_diagnostics.response_statuses);
        diagnostics.notes.extend(raw_diagnostics.notes);
        let raw_quotes = parse_spot_response(&raw_response.bytes)
            .map_err(|error| with_raw_evidence(error, &raw_response.bytes))?
            .into_iter()
            .map(|quote| (quote.symbol.to_ascii_lowercase(), quote))
            .collect::<HashMap<_, _>>();
        let mut instruments = Vec::with_capacity(quotes.len());
        let mut seen = HashMap::new();
        for quote in &quotes {
            if is_cancelled() {
                return Err(error("cancelled", "A-share acquisition was cancelled"));
            }
            let (venue, code) = normalize_provider_instrument(&quote.symbol, &quote.code)?;
            let instrument = InstrumentId::new(venue, code.clone())
                .map_err(|value| error("invalid_instrument", value.to_string()))?;
            let provider_symbol = quote.symbol.trim().to_owned();
            if let Some(previous) = seen.insert(instrument.clone(), provider_symbol.clone())
                && previous != provider_symbol
            {
                return Err(error(
                    "ambiguous_mapping",
                    format!(
                        "provider symbols {previous} and {provider_symbol} map to the same Instrument"
                    ),
                ));
            }
            let current = raw_quotes.get(&provider_symbol.to_ascii_lowercase());
            let current_price = current.and_then(|value| {
                exact_spot_decimal(value.trade.as_deref(), "trade", &mut diagnostics)
            });
            let current_base_volume = current.and_then(|value| {
                exact_spot_decimal(value.volume.as_deref(), "volume", &mut diagnostics)
            });
            let current_quote_volume = current.and_then(|value| {
                exact_spot_decimal(value.amount.as_deref(), "amount", &mut diagnostics)
            });
            let status = match current_price
                .as_deref()
                .and_then(|value| Decimal::from_str(value).ok())
            {
                Some(value) if value > Decimal::ZERO => InstrumentStatus::Live,
                Some(_) => InstrumentStatus::Suspended,
                None => InstrumentStatus::Unknown,
            };
            instruments.push(AshareInstrument {
                mapping: InstrumentSourceMapping {
                    instrument: instrument.clone(),
                    provider: ASHARE_SRC.into(),
                    provider_symbol: provider_symbol.clone(),
                    connector_version: raw_wire_connector_version(),
                    captured_at_ms: retrieved_at_ms,
                },
                instrument,
                provider_symbol,
                name: non_empty(quote.name.clone()),
                status,
                listing_time_ms: None,
                continuous_trading_time_ms: None,
                current_price,
                current_base_volume,
                current_quote_volume,
                current_observed_at_ms: current.map(|_| retrieved_at_ms),
            });
        }
        instruments.sort_by(|left, right| {
            left.instrument
                .venue
                .id
                .cmp(&right.instrument.venue.id)
                .then_with(|| left.instrument.code.cmp(&right.instrument.code))
        });
        diagnostics.notes.push(
            "Instrument Status is inferred from the exact raw current observation: positive price is Live, zero is Suspended, and missing price is Unknown"
                .into(),
        );
        let response_sha256 = sha256(&raw_response.bytes);
        diagnostics.notes.push(
            "akshare-rs exposes parsed DTOs for identity; raw Sina spot bytes are retained for exact current observations"
                .into(),
        );
        if raw_quotes.len() < quotes.len() {
            diagnostics.notes.push(format!(
                "{} spot rows were not present in the exact raw observation response",
                quotes.len() - raw_quotes.len()
            ));
        }
        Ok(AshareInstrumentMasterAcquisition {
            provider: ASHARE_SRC.into(),
            actual_upstream: SINA_UPSTREAM.into(),
            method: "stock_zh_a_spot (akshare-rs identity) + raw-wire Market_Center.getHQNodeData (Sina hs_a current values)".into(),
            connector_version: raw_wire_connector_version(),
            request_parameters: serde_json::json!({ "node": "hs_a", "adjust": "" }),
            retrieved_at_ms,
            response_sha256: response_sha256.clone(),
            parsed_response_sha256,
            parsed_response: Some(parsed_response),
            content_sha256: sha256(&serde_json::to_vec(&instruments).map_err(|value| {
                error("serialization", value.to_string())
            })?),
            raw_response: Some(raw_response.bytes),
            diagnostics,
            instruments,
            limitations: vec![
                "Sina spot status is current evidence; historical listing and suspension times are unavailable from this method"
                    .into(),
                "akshare-rs 0.1.14 caps this Sina spot method at its first five pages; completeness is therefore capability-limited"
                    .into(),
            ],
        })
    }

    pub async fn acquire_bars(
        &self,
        instrument: InstrumentId,
        interval: BarInterval,
        range: HistoricalBarRange,
        retrieved_at_ms: i64,
    ) -> Result<AshareBarsAcquisition, DataError> {
        self.acquire_bars_with_cancel(instrument, interval, range, retrieved_at_ms, || false)
            .await
    }

    pub async fn acquire_bars_with_cancel<F>(
        &self,
        instrument: InstrumentId,
        interval: BarInterval,
        range: HistoricalBarRange,
        retrieved_at_ms: i64,
        is_cancelled: F,
    ) -> Result<AshareBarsAcquisition, DataError>
    where
        F: Fn() -> bool,
    {
        if range.start_time_ms >= range.end_time_ms || retrieved_at_ms < 0 {
            return Err(error(
                "invalid_request",
                "bar range or retrieval time is invalid",
            ));
        }
        let provider_symbol = provider_symbol_for(&instrument)?;
        let (
            bars,
            invalid_bars,
            mut diagnostics,
            actual_upstream,
            method,
            request_parameters,
            hashes,
            raw_responses,
        ) = match interval {
            BarInterval::OneDay => {
                let start = instrument
                    .venue
                    .local_time(range.start_time_ms)
                    .map_err(|value| error("invalid_request", value.to_string()))?
                    .date();
                let end = instrument
                    .venue
                    .local_time(range.end_time_ms)
                    .map_err(|value| error("invalid_request", value.to_string()))?
                    .date();
                let start_text = start.format("%Y%m%d").to_string();
                let end_text = end.format("%Y%m%d").to_string();
                let (response, mut diagnostics) = self
                    .retry_raw(
                        || self.fetch_daily_raw(&instrument, &start_text, &end_text),
                        &is_cancelled,
                    )
                    .await?;
                let candles = parse_daily_response(&response.bytes)
                    .map_err(|error| with_raw_evidence(error, &response.bytes))?;
                let hash = sha256(&response.bytes);
                let mut bars = Vec::new();
                let mut invalid_bars = Vec::new();
                for candle in &candles {
                    if is_cancelled() {
                        return Err(error("cancelled", "A-share acquisition was cancelled"));
                    }
                    match daily_bar(
                        &instrument,
                        &provider_symbol,
                        candle,
                        range,
                        retrieved_at_ms,
                    ) {
                        Ok(bar) => bars.push(bar),
                        Err(value) if value.code == "outside_range" => {}
                        Err(value) => {
                            diagnostics.notes.push(format!(
                                "daily provider row retained for quarantine: {}",
                                value.code
                            ));
                            invalid_bars.push(invalid_daily_bar(
                                &instrument,
                                &provider_symbol,
                                candle,
                            ));
                        }
                    }
                }
                (
                    bars,
                    invalid_bars,
                    diagnostics,
                    EASTMONEY_UPSTREAM,
                    "raw-wire stock_zh_a_daily (Eastmoney kline, adjust=unadjusted)",
                    serde_json::json!({
                        "symbol": instrument.code,
                        "startDate": start_text,
                        "endDate": end_text,
                        "adjust": ""
                    }),
                    vec![hash],
                    vec![response.bytes],
                )
            }
            BarInterval::OneMinute
            | BarInterval::FiveMinutes
            | BarInterval::FifteenMinutes
            | BarInterval::ThirtyMinutes
            | BarInterval::OneHour => {
                let period = match interval {
                    BarInterval::OneMinute => "1",
                    BarInterval::FiveMinutes => "5",
                    BarInterval::FifteenMinutes => "15",
                    BarInterval::ThirtyMinutes => "30",
                    BarInterval::OneHour => "60",
                    _ => unreachable!(),
                };
                let (response, mut diagnostics) = self
                    .retry_raw(
                        || self.fetch_minute_raw(&provider_symbol, period),
                        &is_cancelled,
                    )
                    .await?;
                let candles = parse_minute_response(&response.bytes)
                    .map_err(|error| with_raw_evidence(error, &response.bytes))?;
                let hash = sha256(&response.bytes);
                let mut bars = Vec::new();
                let mut invalid_bars = Vec::new();
                for candle in &candles {
                    if is_cancelled() {
                        return Err(error("cancelled", "A-share acquisition was cancelled"));
                    }
                    match minute_bar(
                        &instrument,
                        &provider_symbol,
                        interval,
                        candle,
                        range,
                        retrieved_at_ms,
                    ) {
                        Ok(bar) => bars.push(bar),
                        Err(value) if value.code == "outside_range" => {}
                        Err(value) => {
                            diagnostics.notes.push(format!(
                                "minute provider row retained for quarantine: {}",
                                value.code
                            ));
                            invalid_bars.push(invalid_minute_bar(
                                &instrument,
                                &provider_symbol,
                                interval,
                                candle,
                            ));
                        }
                    }
                }
                (
                    bars,
                    invalid_bars,
                    diagnostics,
                    SINA_UPSTREAM,
                    "raw-wire stock_zh_a_minute (Sina KLineData, unadjusted)",
                    serde_json::json!({
                        "symbol": provider_symbol,
                        "period": period,
                        "adjust": ""
                    }),
                    vec![hash],
                    vec![response.bytes],
                )
            }
            _ => {
                return Err(error(
                    "unsupported_interval",
                    format!(
                        "A-share akshare acquisition does not support {}",
                        interval.as_str()
                    ),
                ));
            }
        };
        diagnostics
            .notes
            .push("canonical price basis is unadjusted".into());
        diagnostics.notes.push(format!(
            "exact raw bytes captured by {ASHARE_RAW_WIRE_ADAPTER_VERSION}; akshare-rs 0.1.14 DTOs use floating-point fields for these methods"
        ));
        if bars.is_empty() && invalid_bars.is_empty() {
            diagnostics.notes.push(
                "provider returned no rows for the requested range; retained raw evidence does not establish history availability"
                    .into(),
            );
        }
        let content_sha256 = sha256(
            &serde_json::to_vec(&(&bars, &invalid_bars))
                .map_err(|value| error("serialization", value.to_string()))?,
        );
        let mut limitations = if matches!(
            interval,
            BarInterval::OneMinute
                | BarInterval::FiveMinutes
                | BarInterval::FifteenMinutes
                | BarInterval::ThirtyMinutes
                | BarInterval::OneHour
        ) {
            vec!["Sina minute history is limited to the provider response window".into()]
        } else {
            Vec::new()
        };
        if bars.is_empty() && invalid_bars.is_empty() {
            limitations
                .push("No provider rows were returned; history availability is unconfirmed".into());
        }
        Ok(AshareBarsAcquisition {
            provider: ASHARE_SRC.into(),
            actual_upstream: actual_upstream.into(),
            method: method.into(),
            connector_version: raw_wire_connector_version(),
            request_parameters,
            retrieved_at_ms,
            response_sha256s: hashes,
            content_sha256,
            raw_responses,
            diagnostics,
            bars,
            invalid_bars,
            limitations,
        })
    }

    pub async fn acquire_calendar(
        &self,
        range: HistoricalBarRange,
        retrieved_at_ms: i64,
    ) -> Result<AshareCalendarAcquisition, DataError> {
        self.acquire_calendar_with_cancel(range, retrieved_at_ms, || false)
            .await
    }

    pub async fn acquire_calendar_with_cancel<F>(
        &self,
        range: HistoricalBarRange,
        retrieved_at_ms: i64,
        is_cancelled: F,
    ) -> Result<AshareCalendarAcquisition, DataError>
    where
        F: Fn() -> bool,
    {
        if range.start_time_ms < 0
            || range.end_time_ms < 0
            || range.start_time_ms >= range.end_time_ms
            || range.end_time_ms.saturating_sub(range.start_time_ms) > 20 * 366 * 86_400_000
            || retrieved_at_ms < 0
        {
            return Err(error(
                "invalid_request",
                "calendar range exceeds the bounded acquisition window or retrieval time is invalid",
            ));
        }
        let (response, mut diagnostics) = self
            .retry_raw(|| self.fetch_calendar_raw(), &is_cancelled)
            .await?;
        let date_strings = parse_trade_date_response(&response.bytes)
            .map_err(|error| with_raw_evidence(error, &response.bytes))?;
        let invalid_date_count = date_strings
            .iter()
            .filter(|value| parse_date(value).is_err())
            .count();
        let open_dates = date_strings
            .iter()
            .filter_map(|value| parse_date(value).ok())
            .collect::<std::collections::BTreeSet<_>>();
        let sse = Venue::china_a_share("sse")
            .map_err(|value| error("invalid_venue", value.to_string()))?;
        let start_date = sse
            .local_time(range.start_time_ms)
            .map_err(|value| error("invalid_request", value.to_string()))?
            .date();
        let end_date = sse
            .local_time(range.end_time_ms)
            .map_err(|value| error("invalid_request", value.to_string()))?
            .date();
        let sessions = vec![
            TradingSession {
                phase: SessionPhase::Auction,
                start_local: NaiveTime::from_hms_opt(9, 15, 0).expect("valid session"),
                end_local: NaiveTime::from_hms_opt(9, 25, 0).expect("valid session"),
            },
            TradingSession {
                phase: SessionPhase::PreOpen,
                start_local: NaiveTime::from_hms_opt(9, 25, 0).expect("valid session"),
                end_local: NaiveTime::from_hms_opt(9, 30, 0).expect("valid session"),
            },
            TradingSession {
                phase: SessionPhase::Continuous,
                start_local: NaiveTime::from_hms_opt(9, 30, 0).expect("valid session"),
                end_local: NaiveTime::from_hms_opt(11, 30, 0).expect("valid session"),
            },
            TradingSession {
                phase: SessionPhase::Break,
                start_local: NaiveTime::from_hms_opt(11, 30, 0).expect("valid session"),
                end_local: NaiveTime::from_hms_opt(13, 0, 0).expect("valid session"),
            },
            TradingSession {
                phase: SessionPhase::Continuous,
                start_local: NaiveTime::from_hms_opt(13, 0, 0).expect("valid session"),
                end_local: NaiveTime::from_hms_opt(15, 0, 0).expect("valid session"),
            },
        ];
        let coverage_start = open_dates.iter().next().copied();
        let coverage_end = open_dates.iter().next_back().copied();
        let mut unavailable_day_count = 0;
        let mut days = Vec::new();
        let mut date = start_date;
        while date <= end_date {
            if is_cancelled() {
                return Err(error("cancelled", "A-share acquisition was cancelled"));
            }
            let trading_date = TradingDate::from_naive_date(date);
            let is_weekend = matches!(date.weekday(), chrono::Weekday::Sat | chrono::Weekday::Sun);
            let day_kind = if is_weekend {
                DayKind::Weekend
            } else if coverage_start.is_none_or(|start| date < start)
                || coverage_end.is_none_or(|end| date > end)
            {
                unavailable_day_count += 1;
                DayKind::Unavailable
            } else if open_dates.contains(&date) {
                DayKind::TradingDay
            } else {
                DayKind::Holiday
            };
            let closures = if day_kind == DayKind::TradingDay {
                vec![ScheduledClosure {
                    kind: ScheduledClosureKind::Maintenance,
                    start_ms: sse
                        .resolve_local_time(
                            date.and_time(NaiveTime::from_hms_opt(11, 30, 0).expect("valid time")),
                            LocalTimeDisambiguation::Reject,
                        )
                        .map_err(|value| error("invalid_calendar", value.to_string()))?,
                    end_ms: sse
                        .resolve_local_time(
                            date.and_time(NaiveTime::from_hms_opt(13, 0, 0).expect("valid time")),
                            LocalTimeDisambiguation::Reject,
                        )
                        .map_err(|value| error("invalid_calendar", value.to_string()))?,
                    reason: Some("scheduled A-share midday break".into()),
                }]
            } else {
                Vec::new()
            };
            days.push(DayEvidence {
                date: trading_date,
                day_kind,
                session_override: None,
                closures,
            });
            date = date
                .succ_opt()
                .ok_or_else(|| error("invalid_calendar", "calendar date overflow"))?;
        }
        let mut snapshots = Vec::with_capacity(2);
        for venue_id in ["sse", "szse"] {
            let venue = Venue::china_a_share(venue_id)
                .map_err(|value| error("invalid_venue", value.to_string()))?;
            let snapshot_bytes = serde_json::to_vec(&(venue_id, range, &days, &sessions))
                .map_err(|value| error("serialization", value.to_string()))?;
            snapshots.push(
                TradingCalendarSnapshot::new(
                    format!("ashare-calendar-{}", sha256(&snapshot_bytes)),
                    venue,
                    range.start_time_ms,
                    range.end_time_ms,
                    sessions.clone(),
                    days.clone(),
                )
                .map_err(|value| error("invalid_calendar", value.to_string()))?,
            );
        }
        if invalid_date_count > 0 {
            diagnostics.notes.push(format!(
                "{invalid_date_count} provider trade-date rows could not be parsed and were not used"
            ));
        }
        if unavailable_day_count > 0 {
            diagnostics.notes.push(format!(
                "{unavailable_day_count} requested dates are outside the provider trade-date coverage"
            ));
        }
        diagnostics.notes.push(
            "Sina trade-date history provides shared open-date evidence; exchange-specific auction and early-close detail is unavailable"
                .into(),
        );
        let content_bytes = serde_json::to_vec(&snapshots)
            .map_err(|value| error("serialization", value.to_string()))?;
        let mut limitations = vec![
            "Provider trade-date history does not expose every exchange-specific early close or ad-hoc closure"
                .into(),
        ];
        if unavailable_day_count > 0 {
            limitations.push(format!(
                "{unavailable_day_count} dates are outside provider trade-date coverage and are marked Unavailable"
            ));
        }
        if invalid_date_count > 0 {
            limitations.push(format!(
                "{invalid_date_count} provider trade-date rows were not parseable"
            ));
        }
        Ok(AshareCalendarAcquisition {
            provider: ASHARE_SRC.into(),
            actual_upstream: SINA_UPSTREAM.into(),
            method: "raw-wire tool_trade_date_hist (Sina klc_td_sh)".into(),
            connector_version: raw_wire_connector_version(),
            request_parameters: serde_json::json!({
                "startTimeMs": range.start_time_ms,
                "endTimeMs": range.end_time_ms
            }),
            retrieved_at_ms,
            response_sha256: sha256(&response.bytes),
            content_sha256: sha256(&content_bytes),
            raw_response: Some(response.bytes),
            diagnostics,
            snapshots,
            limitations,
        })
    }

    pub async fn acquire_corporate_actions(
        &self,
        instrument: InstrumentId,
        retrieved_at_ms: i64,
    ) -> Result<AshareCorporateActionAcquisition, DataError> {
        self.acquire_corporate_actions_with_cancel(instrument, retrieved_at_ms, || false)
            .await
    }

    pub async fn acquire_corporate_actions_with_cancel<F>(
        &self,
        instrument: InstrumentId,
        retrieved_at_ms: i64,
        is_cancelled: F,
    ) -> Result<AshareCorporateActionAcquisition, DataError>
    where
        F: Fn() -> bool,
    {
        if retrieved_at_ms < 0 {
            return Err(error(
                "invalid_request",
                "retrieval time must be non-negative",
            ));
        }
        let provider_symbol = provider_symbol_for(&instrument)?;
        let (response, mut diagnostics) = self
            .retry_raw(
                || self.fetch_corporate_actions_raw(&instrument.code, 1),
                &is_cancelled,
            )
            .await?;
        let (mut dividends, page_count) = parse_corporate_actions_response(&response.bytes)
            .map_err(|error| with_raw_evidence(error, &response.bytes))?;
        let retained_page_count = page_count.min(MAX_CORPORATE_ACTION_PAGES);
        let mut raw_responses = vec![response.bytes];
        if page_count > 1 {
            diagnostics.notes.push(format!(
                "corporate-action response exposes {page_count} pages; the connector will request the bounded first {retained_page_count} pages"
            ));
        }
        for page_number in 2..=retained_page_count {
            if is_cancelled() {
                return Err(error("cancelled", "A-share acquisition was cancelled"));
            }
            let (page, page_diagnostics) = self
                .retry_raw(
                    || self.fetch_corporate_actions_raw(&instrument.code, page_number),
                    &is_cancelled,
                )
                .await?;
            diagnostics.request_count += page_diagnostics.request_count;
            diagnostics.retry_count += page_diagnostics.retry_count;
            diagnostics
                .response_statuses
                .extend(page_diagnostics.response_statuses);
            diagnostics.notes.extend(page_diagnostics.notes);
            let (page_dividends, declared_page_count) =
                parse_corporate_actions_response(&page.bytes)
                    .map_err(|error| with_raw_evidence(error, &page.bytes))?;
            if declared_page_count != page_count {
                diagnostics.notes.push(format!(
                    "corporate-action page {page_number} reported {declared_page_count} pages instead of {page_count}"
                ));
            }
            dividends.extend(page_dividends);
            raw_responses.push(page.bytes);
        }
        let mut records = Vec::new();
        let mut invalid_records = Vec::new();
        for dividend in dividends {
            if is_cancelled() {
                return Err(error("cancelled", "A-share acquisition was cancelled"));
            }
            let provider_code = action_security_code(&dividend);
            let announcement = action_date_text(&dividend, "NOTICE_DATE")
                .or_else(|| action_date_text(&dividend, "PLAN_NOTICE_DATE"));
            let effective = action_date_text(&dividend, "EX_DIVIDEND_DATE");
            if effective.is_none() {
                diagnostics.notes.push(
                    "corporate action effective date is unavailable without EX_DIVIDEND_DATE"
                        .into(),
                );
            }
            let announced_at_ms = action_date_ms(
                &instrument,
                announcement.as_deref(),
                &mut diagnostics,
                "announcement",
            );
            let effective_at_ms = action_date_ms(
                &instrument,
                effective.as_deref(),
                &mut diagnostics,
                "effective",
            );
            if provider_code.as_deref() != Some(instrument.code.as_str()) {
                diagnostics.notes.push(format!(
                    "corporate action row quarantined: SECURITY_CODE {:?} does not match {}",
                    provider_code, instrument.code
                ));
                invalid_records.push(quarantined_corporate_action(
                    &instrument,
                    &provider_symbol,
                    retrieved_at_ms,
                    effective_at_ms,
                    announced_at_ms,
                    dividend,
                ));
                continue;
            }
            if (announcement.is_some() && announced_at_ms.is_none())
                || (effective.is_some() && effective_at_ms.is_none())
            {
                diagnostics
                    .notes
                    .push("corporate action row quarantined: invalid date".into());
                invalid_records.push(quarantined_corporate_action(
                    &instrument,
                    &provider_symbol,
                    retrieved_at_ms,
                    effective_at_ms,
                    announced_at_ms,
                    dividend,
                ));
                continue;
            }
            let cash = raw_decimal_value(&dividend, "PRETAX_BONUS_RMB");
            let shares = sum_raw_decimals(&["BONUS_IT_RATIO", "TRANSFER_IT_RATIO"], &dividend);
            let (cash, shares) = match (cash, shares) {
                (Ok(cash), Ok(shares)) => (cash, shares),
                (Err(error), _) | (_, Err(error)) => {
                    diagnostics.notes.push(format!(
                        "corporate action row quarantined: {}",
                        error.message
                    ));
                    invalid_records.push(quarantined_corporate_action(
                        &instrument,
                        &provider_symbol,
                        retrieved_at_ms,
                        effective_at_ms,
                        announced_at_ms,
                        dividend,
                    ));
                    continue;
                }
            };
            let kind = match (cash.is_some(), shares.is_some()) {
                (true, false) => AshareCorporateActionKind::CashDividend,
                (false, true) => AshareCorporateActionKind::ShareDistribution,
                (true, true) => AshareCorporateActionKind::CashAndShareDistribution,
                (false, false) => AshareCorporateActionKind::Unknown,
            };
            records.push(AshareCorporateAction {
                instrument: instrument.clone(),
                provider_symbol: provider_symbol.clone(),
                kind,
                effective_at_ms,
                announced_at_ms,
                available_at_ms: retrieved_at_ms,
                cash_per_share: cash,
                shares_per_share: shares,
                raw_payload: dividend,
            });
        }
        diagnostics
            .notes
            .push("corporate actions are retained separately from Bars".into());
        let invalid_date_count = diagnostics
            .notes
            .iter()
            .filter(|note| {
                note.starts_with("corporate action ") && note.contains("date is invalid")
            })
            .count();
        let content_bytes = serde_json::to_vec(&(&records, &invalid_records))
            .map_err(|value| error("serialization", value.to_string()))?;
        let raw_response = raw_responses
            .into_iter()
            .enumerate()
            .flat_map(|(index, bytes)| {
                std::iter::once(index.to_string().into_bytes())
                    .chain(std::iter::once(b"\n".to_vec()))
                    .chain(std::iter::once(bytes))
                    .chain(std::iter::once(b"\n".to_vec()))
            })
            .flatten()
            .collect::<Vec<_>>();
        Ok(AshareCorporateActionAcquisition {
            instrument: instrument.clone(),
            provider: ASHARE_SRC.into(),
            actual_upstream: EASTMONEY_UPSTREAM.into(),
            method: "raw-wire stock_fhps_detail_em (Eastmoney RPT_SHAREBONUS_DET)".into(),
            connector_version: raw_wire_connector_version(),
            request_parameters: serde_json::json!({
                "symbol": instrument.code,
                "pageNumber": 1,
                "pageSize": 500,
                "pagesRequested": retained_page_count
            }),
            retrieved_at_ms,
            response_sha256: sha256(&raw_response),
            content_sha256: sha256(&content_bytes),
            raw_response: Some(raw_response),
            diagnostics,
            records,
            invalid_records,
            limitations: {
                let mut limitations = vec![
                    "Provider exposes announcement, record, and ex-dividend dates but not a complete effective-time taxonomy"
                        .into(),
                ];
                if page_count > MAX_CORPORATE_ACTION_PAGES {
                    limitations.push(format!(
                        "Provider returned {page_count} corporate-action pages; this bounded connector retains only the first {MAX_CORPORATE_ACTION_PAGES} pages"
                    ));
                }
                if invalid_date_count > 0 {
                    limitations.push(format!(
                        "{invalid_date_count} corporate-action date fields were invalid and retained as Unknown"
                    ));
                }
                limitations
            },
        })
    }

    async fn fetch_daily_raw(
        &self,
        instrument: &InstrumentId,
        start_date: &str,
        end_date: &str,
    ) -> Result<RawResponse, DataError> {
        let query = vec![
            ("secid".into(), eastmoney_secid(instrument)?),
            ("ut".into(), "fa5fd1943c7b386f172d6893dbfba10b".into()),
            ("klt".into(), "101".into()),
            ("fqt".into(), "0".into()),
            ("lmt".into(), "1000000".into()),
            ("beg".into(), start_date.into()),
            ("end".into(), end_date.into()),
            ("fields1".into(), "f1,f2,f3,f4,f5,f6".into()),
            (
                "fields2".into(),
                "f51,f52,f53,f54,f55,f56,f57,f58,f59,f60,f61".into(),
            ),
        ];
        self.raw_get(
            "https://push2his.eastmoney.com/api/qt/stock/kline/get",
            &query,
        )
        .await
    }

    async fn fetch_spot_raw(&self) -> Result<RawResponse, DataError> {
        let query = vec![
            ("page".into(), "1".into()),
            ("num".into(), "10000".into()),
            ("sort".into(), "symbol".into()),
            ("asc".into(), "1".into()),
            ("node".into(), "hs_a".into()),
        ];
        self.raw_get(
            "https://vip.stock.finance.sina.com.cn/quotes_service/api/json_v2.php/Market_Center.getHQNodeData",
            &query,
        )
        .await
    }

    async fn fetch_calendar_raw(&self) -> Result<RawResponse, DataError> {
        self.raw_get(
            "https://finance.sina.com.cn/realstock/company/klc_td_sh.txt",
            &[],
        )
        .await
    }

    async fn fetch_minute_raw(
        &self,
        provider_symbol: &str,
        period: &str,
    ) -> Result<RawResponse, DataError> {
        let query = vec![
            ("symbol".into(), provider_symbol.into()),
            ("scale".into(), period.into()),
            ("datalen".into(), "1970".into()),
        ];
        self.raw_get(
            "https://quotes.sina.cn/cn/api/jsonp_v2.php/=/CN_MarketDataService.getKLineData",
            &query,
        )
        .await
    }

    async fn fetch_corporate_actions_raw(
        &self,
        code: &str,
        page_number: u64,
    ) -> Result<RawResponse, DataError> {
        let query = vec![
            ("reportName".into(), "RPT_SHAREBONUS_DET".into()),
            ("columns".into(), "ALL".into()),
            ("filter".into(), format!("(SECURITY_CODE=\"{code}\")")),
            ("pageNumber".into(), page_number.to_string()),
            ("pageSize".into(), "500".into()),
            ("sortTypes".into(), "-1".into()),
            ("sortColumns".into(), "REPORT_DATE".into()),
            ("source".into(), "WEB".into()),
            ("client".into(), "WEB".into()),
        ];
        self.raw_get(
            "https://datacenter-web.eastmoney.com/api/data/v1/get",
            &query,
        )
        .await
    }

    async fn raw_get(
        &self,
        url: &str,
        query: &[(String, String)],
    ) -> Result<RawResponse, DataError> {
        let response = self
            .raw_http
            .get(self.raw_url(url))
            .query(query)
            .send()
            .await
            .map_err(|value| error("upstream", value.to_string()))?;
        let status = response.status().as_u16();
        let bytes = response
            .bytes()
            .await
            .map_err(|value| error("upstream", value.to_string()))?
            .to_vec();
        Ok(RawResponse { status, bytes })
    }

    fn raw_url(&self, url: &str) -> String {
        let Some(mock_uri) = &self.mock_uri else {
            return url.into();
        };
        let path = url
            .split_once("//")
            .and_then(|(_, rest)| rest.find('/').map(|index| &rest[index..]))
            .unwrap_or("/");
        format!("{}{}", mock_uri.trim_end_matches('/'), path)
    }

    async fn retry_raw<F, Fut>(
        &self,
        mut operation: F,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<(RawResponse, AshareRequestDiagnostics), DataError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<RawResponse, DataError>>,
    {
        let attempts = self.policy.max_attempts.max(1);
        let mut diagnostics = AshareRequestDiagnostics::default();
        let mut last_error = None;
        let mut last_response = None;
        for attempt in 0..attempts {
            diagnostics.request_count += 1;
            match cancellable_timeout(
                operation(),
                Duration::from_millis(self.policy.timeout_ms.max(1)),
                is_cancelled,
            )
            .await?
            {
                Some(Ok(response)) if (200..300).contains(&response.status) => {
                    diagnostics.response_statuses.push(response.status);
                    return Ok((response, diagnostics));
                }
                Some(Ok(response)) => {
                    diagnostics.response_statuses.push(response.status);
                    last_error = Some(format!(
                        "A-share upstream returned HTTP {}",
                        response.status
                    ));
                    last_response = Some(response);
                }
                Some(Err(value)) => {
                    diagnostics
                        .notes
                        .push(format!("transport error: {}", value.message));
                    last_error = Some(value.message)
                }
                None => {
                    diagnostics.notes.push("request timed out".into());
                    last_error = Some("request timed out".into())
                }
            }
            if attempt + 1 < attempts {
                diagnostics.retry_count += 1;
                sleep_or_cancel(
                    Duration::from_millis(self.policy.retry_delay_ms),
                    is_cancelled,
                )
                .await?;
            }
        }
        let mut error = retry_failure(last_error, &diagnostics);
        if let Some(response) = last_response {
            error = with_raw_evidence(error, &response.bytes);
        }
        Err(error)
    }

    async fn retry_with_cancel<T, F, Fut>(
        &self,
        mut operation: F,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<(T, AshareRequestDiagnostics), DataError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, akshare::Error>>,
    {
        let attempts = self.policy.max_attempts.max(1);
        let mut diagnostics = AshareRequestDiagnostics::default();
        let mut last_error = None;
        for attempt in 0..attempts {
            diagnostics.request_count += 1;
            match cancellable_timeout(
                operation(),
                Duration::from_millis(self.policy.timeout_ms.max(1)),
                is_cancelled,
            )
            .await?
            {
                Some(Ok(value)) => return Ok((value, diagnostics)),
                Some(Err(value)) => {
                    diagnostics.notes.push(format!("transport error: {value}"));
                    last_error = Some(value.to_string())
                }
                None => {
                    diagnostics.notes.push("request timed out".into());
                    last_error = Some("request timed out".into())
                }
            }
            if attempt + 1 < attempts {
                diagnostics.retry_count += 1;
                sleep_or_cancel(
                    Duration::from_millis(self.policy.retry_delay_ms),
                    is_cancelled,
                )
                .await?;
            }
        }
        Err(retry_failure(last_error, &diagnostics))
    }
}

fn retry_failure(last_error: Option<String>, diagnostics: &AshareRequestDiagnostics) -> DataError {
    error(
        "upstream",
        format!(
            "{}; attempts={}; retries={}; responseStatuses={:?}",
            last_error.unwrap_or_else(|| "upstream request failed".into()),
            diagnostics.request_count,
            diagnostics.retry_count,
            diagnostics.response_statuses,
        ),
    )
}

async fn cancellable_timeout<T, Fut>(
    future: Fut,
    duration: Duration,
    is_cancelled: &dyn Fn() -> bool,
) -> Result<Option<T>, DataError>
where
    Fut: Future<Output = T>,
{
    tokio::pin!(future);
    let deadline = tokio::time::sleep(duration);
    tokio::pin!(deadline);
    loop {
        if is_cancelled() {
            return Err(error("cancelled", "A-share acquisition was cancelled"));
        }
        tokio::select! {
            value = &mut future => return Ok(Some(value)),
            _ = &mut deadline => return Ok(None),
            _ = tokio::time::sleep(Duration::from_millis(25)) => {}
        }
    }
}

async fn sleep_or_cancel(
    duration: Duration,
    is_cancelled: &dyn Fn() -> bool,
) -> Result<(), DataError> {
    let deadline = tokio::time::sleep(duration);
    tokio::pin!(deadline);
    loop {
        if is_cancelled() {
            return Err(error("cancelled", "A-share acquisition was cancelled"));
        }
        tokio::select! {
            _ = &mut deadline => return Ok(()),
            _ = tokio::time::sleep(Duration::from_millis(25)) => {}
        }
    }
}

fn raw_wire_connector_version() -> String {
    format!("{ASHARE_CONNECTOR_VERSION}+{ASHARE_RAW_WIRE_ADAPTER_VERSION}")
}

pub fn normalize_provider_instrument(
    provider_symbol: &str,
    provider_code: &str,
) -> Result<(Venue, String), DataError> {
    let code = provider_code.trim();
    if code.len() != 6 || !code.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(error(
            "invalid_instrument",
            format!("A-share code must be six digits: {code}"),
        ));
    }
    let expected_venue = venue_for_code(code)?;
    let symbol = provider_symbol.trim().to_ascii_lowercase();
    if symbol.is_empty() {
        return Err(error("invalid_instrument", "provider symbol is empty"));
    }
    if let Some(prefix) = symbol.strip_suffix(code) {
        if !matches!(prefix, "sh" | "sz") {
            return Err(error(
                "ambiguous_mapping",
                format!("provider symbol {provider_symbol} has no supported exchange prefix"),
            ));
        }
        let expected_prefix = if expected_venue.id == "sse" {
            "sh"
        } else {
            "sz"
        };
        if prefix != expected_prefix {
            return Err(error(
                "ambiguous_mapping",
                format!("provider symbol {provider_symbol} conflicts with code {code}"),
            ));
        }
    } else if symbol != code {
        return Err(error(
            "ambiguous_mapping",
            format!("provider symbol {provider_symbol} does not identify code {code}"),
        ));
    }
    Ok((expected_venue, code.into()))
}

fn provider_symbol_for(instrument: &InstrumentId) -> Result<String, DataError> {
    if instrument.venue.kind != VenueKind::ChinaAShareEquity {
        return Err(error(
            "unsupported_instrument",
            "A-share connector requires a China A-share Venue",
        ));
    }
    let prefix = match instrument.venue.id.as_str() {
        "sse" => "sh",
        "szse" => "sz",
        venue => {
            return Err(error(
                "unsupported_venue",
                format!("unsupported China A-share Venue {venue}"),
            ));
        }
    };
    let (venue, code) =
        normalize_provider_instrument(&format!("{prefix}{}", instrument.code), &instrument.code)?;
    if venue != instrument.venue {
        return Err(error(
            "ambiguous_mapping",
            "Instrument Venue does not match provider code",
        ));
    }
    Ok(format!("{prefix}{code}"))
}

fn venue_for_code(code: &str) -> Result<Venue, DataError> {
    let sse = code.starts_with("600")
        || code.starts_with("601")
        || code.starts_with("603")
        || code.starts_with("605")
        || code.starts_with("688")
        || code.starts_with("689");
    let szse = code.starts_with("000")
        || code.starts_with("001")
        || code.starts_with("002")
        || code.starts_with("003")
        || code.starts_with("300")
        || code.starts_with("301");
    match (sse, szse) {
        (true, false) => {
            Venue::china_a_share("sse").map_err(|value| error("invalid_venue", value.to_string()))
        }
        (false, true) => {
            Venue::china_a_share("szse").map_err(|value| error("invalid_venue", value.to_string()))
        }
        _ => Err(error(
            "ambiguous_mapping",
            format!("A-share code {code} has no unique SSE/SZSE mapping"),
        )),
    }
}

fn daily_bar(
    instrument: &InstrumentId,
    provider_symbol: &str,
    candle: &RawDailyCandle,
    range: HistoricalBarRange,
    retrieved_at_ms: i64,
) -> Result<AshareBar, DataError> {
    let date = parse_date(&candle.date)?;
    let open_time_ms = instrument
        .venue
        .resolve_local_time(
            date.and_time(NaiveTime::from_hms_opt(9, 30, 0).expect("valid session")),
            LocalTimeDisambiguation::Reject,
        )
        .map_err(|value| error("invalid_timestamp", value.to_string()))?;
    if open_time_ms < range.start_time_ms
        || open_time_ms >= range.end_time_ms
        || !daily_closed(instrument, date, retrieved_at_ms)?
    {
        return Err(error(
            "outside_range",
            "provider bar is outside requested closed range",
        ));
    }
    Ok(AshareBar {
        instrument: instrument.clone(),
        provider_symbol: provider_symbol.into(),
        interval: BarInterval::OneDay,
        open_time_ms,
        open: Some(required_decimal("open", candle.open.as_deref())?),
        high: Some(required_decimal("high", candle.high.as_deref())?),
        low: Some(required_decimal("low", candle.low.as_deref())?),
        close: Some(required_decimal("close", candle.close.as_deref())?),
        base_volume: Some(required_decimal("volume", candle.volume.as_deref())?),
        quote_volume: optional_decimal("amount", candle.amount.as_deref())?,
        price_basis: PriceBasis::Unadjusted,
        raw_payload: candle.raw_payload.clone(),
    })
}

fn invalid_daily_bar(
    instrument: &InstrumentId,
    provider_symbol: &str,
    candle: &RawDailyCandle,
) -> AshareBar {
    let open_time_ms = parse_date(&candle.date)
        .ok()
        .and_then(|date| {
            instrument
                .venue
                .resolve_local_time(
                    date.and_time(NaiveTime::from_hms_opt(9, 30, 0).expect("valid session")),
                    LocalTimeDisambiguation::Reject,
                )
                .ok()
        })
        .unwrap_or_default();
    AshareBar {
        instrument: instrument.clone(),
        provider_symbol: provider_symbol.into(),
        interval: BarInterval::OneDay,
        open_time_ms,
        open: None,
        high: None,
        low: None,
        close: None,
        base_volume: None,
        quote_volume: None,
        price_basis: PriceBasis::Unadjusted,
        raw_payload: candle.raw_payload.clone(),
    }
}

fn minute_bar(
    instrument: &InstrumentId,
    provider_symbol: &str,
    interval: BarInterval,
    candle: &RawMinuteCandle,
    range: HistoricalBarRange,
    retrieved_at_ms: i64,
) -> Result<AshareBar, DataError> {
    let datetime = candle
        .datetime
        .as_deref()
        .ok_or_else(|| error("invalid_timestamp", "minute provider row has no datetime"))?;
    let local = chrono::NaiveDateTime::parse_from_str(datetime, "%Y-%m-%d %H:%M:%S")
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(datetime, "%Y-%m-%d %H:%M"))
        .map_err(|value| error("invalid_timestamp", value.to_string()))?;
    let open_time_ms = instrument
        .venue
        .resolve_local_time(local, LocalTimeDisambiguation::Reject)
        .map_err(|value| error("invalid_timestamp", value.to_string()))?;
    let step_ms = interval_ms(interval)
        .ok_or_else(|| error("unsupported_interval", "interval is not fixed"))?;
    if open_time_ms < range.start_time_ms
        || open_time_ms >= range.end_time_ms
        || open_time_ms.saturating_add(step_ms) > retrieved_at_ms
    {
        return Err(error(
            "outside_range",
            "provider bar is outside requested closed range",
        ));
    }
    let open = required_decimal("open", candle.open.as_deref())?;
    let high = required_decimal("high", candle.high.as_deref())?;
    let low = required_decimal("low", candle.low.as_deref())?;
    let close = required_decimal("close", candle.close.as_deref())?;
    let base_volume = required_decimal("volume", candle.volume.as_deref())?;
    let quote_volume = required_decimal("amount", candle.amount.as_deref())?;
    Ok(AshareBar {
        instrument: instrument.clone(),
        provider_symbol: provider_symbol.into(),
        interval,
        open_time_ms,
        open: Some(open.to_string()),
        high: Some(high.to_string()),
        low: Some(low.to_string()),
        close: Some(close.to_string()),
        base_volume: Some(base_volume.to_string()),
        quote_volume: Some(quote_volume),
        price_basis: PriceBasis::Unadjusted,
        raw_payload: candle.raw_payload.clone(),
    })
}

fn invalid_minute_bar(
    instrument: &InstrumentId,
    provider_symbol: &str,
    interval: BarInterval,
    candle: &RawMinuteCandle,
) -> AshareBar {
    let open_time_ms = candle
        .datetime
        .as_deref()
        .and_then(|datetime| {
            chrono::NaiveDateTime::parse_from_str(datetime, "%Y-%m-%d %H:%M:%S")
                .or_else(|_| chrono::NaiveDateTime::parse_from_str(datetime, "%Y-%m-%d %H:%M"))
                .ok()
        })
        .and_then(|local| {
            instrument
                .venue
                .resolve_local_time(local, LocalTimeDisambiguation::Reject)
                .ok()
        })
        .unwrap_or_default();
    AshareBar {
        instrument: instrument.clone(),
        provider_symbol: provider_symbol.into(),
        interval,
        open_time_ms,
        open: None,
        high: None,
        low: None,
        close: None,
        base_volume: None,
        quote_volume: None,
        price_basis: PriceBasis::Unadjusted,
        raw_payload: candle.raw_payload.clone(),
    }
}

fn daily_closed(
    instrument: &InstrumentId,
    date: NaiveDate,
    retrieved_at_ms: i64,
) -> Result<bool, DataError> {
    let local = instrument
        .venue
        .local_time(retrieved_at_ms)
        .map_err(|value| error("invalid_timestamp", value.to_string()))?;
    Ok(local.date() > date
        || (local.date() == date
            && local.time() >= NaiveTime::from_hms_opt(15, 0, 0).expect("valid session")))
}

fn parse_date(value: &str) -> Result<NaiveDate, DataError> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .or_else(|_| NaiveDate::parse_from_str(value, "%Y%m%d"))
        .map_err(|value| error("invalid_date", value.to_string()))
}

fn date_to_midnight_ms(instrument: &InstrumentId, value: &str) -> Result<i64, DataError> {
    let date = parse_date(value)?;
    instrument
        .venue
        .resolve_local_time(
            date.and_time(NaiveTime::from_hms_opt(0, 0, 0).expect("valid time")),
            LocalTimeDisambiguation::Reject,
        )
        .map_err(|value| error("invalid_timestamp", value.to_string()))
}

fn eastmoney_secid(instrument: &InstrumentId) -> Result<String, DataError> {
    let market = match instrument.venue.id.as_str() {
        "sse" => "1",
        "szse" => "0",
        venue => {
            return Err(error(
                "unsupported_venue",
                format!("unsupported Eastmoney A-share Venue {venue}"),
            ));
        }
    };
    Ok(format!("{market}.{}", instrument.code))
}

fn parse_daily_response(response: &[u8]) -> Result<Vec<RawDailyCandle>, DataError> {
    let payload: Value = serde_json::from_slice(response)
        .map_err(|value| error("decode", format!("daily response JSON is invalid: {value}")))?;
    reject_adjusted_payload(&payload)?;
    let rows = payload
        .pointer("/data/klines")
        .and_then(Value::as_array)
        .ok_or_else(|| error("decode", "daily response has no data.klines array"))?;
    Ok(rows
        .iter()
        .map(|row| {
            let line = row.as_str().unwrap_or_default();
            let parts = line.split(',').collect::<Vec<_>>();
            RawDailyCandle {
                date: parts.first().copied().unwrap_or_default().into(),
                open: parts.get(1).map(|value| (*value).into()),
                close: parts.get(2).map(|value| (*value).into()),
                high: parts.get(3).map(|value| (*value).into()),
                low: parts.get(4).map(|value| (*value).into()),
                volume: parts.get(5).map(|value| (*value).into()),
                amount: parts.get(6).map(|value| (*value).into()),
                raw_payload: row.clone(),
            }
        })
        .collect())
}

fn parse_spot_response(response: &[u8]) -> Result<Vec<RawSpotQuote>, DataError> {
    let quotes: Vec<RawSpotQuote> = serde_json::from_slice(response)
        .map_err(|value| error("decode", format!("spot response JSON is invalid: {value}")))?;
    if quotes.is_empty() {
        return Err(error("not_found", "spot response has no rows"));
    }
    Ok(quotes)
}

fn parse_trade_date_response(response: &[u8]) -> Result<Vec<String>, DataError> {
    let text = std::str::from_utf8(response).map_err(|value| {
        error(
            "decode",
            format!("trade-date response is not UTF-8: {value}"),
        )
    })?;
    let dates = text
        .split_once('=')
        .and_then(|(_, rest)| rest.split_once(';'))
        .map(|(value, _)| value.trim_matches('"'))
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if dates.is_empty() {
        return Err(error("not_found", "trade-date response has no rows"));
    }
    Ok(dates)
}

fn parse_minute_response(response: &[u8]) -> Result<Vec<RawMinuteCandle>, DataError> {
    let text = std::str::from_utf8(response)
        .map_err(|value| error("decode", format!("minute response is not UTF-8: {value}")))?;
    let json_start = text
        .find("=(")
        .ok_or_else(|| error("decode", "minute response is missing JSONP prefix"))?
        + 2;
    let json_end = text
        .rfind(");")
        .ok_or_else(|| error("decode", "minute response is missing JSONP suffix"))?;
    let rows: Vec<Value> = serde_json::from_str(&text[json_start..json_end]).map_err(|value| {
        error(
            "decode",
            format!("minute response JSON is invalid: {value}"),
        )
    })?;
    for row in &rows {
        reject_adjusted_payload(row)?;
    }
    Ok(rows
        .into_iter()
        .map(|row| RawMinuteCandle {
            datetime: row.get("day").and_then(value_text),
            open: row.get("open").and_then(value_text),
            high: row.get("high").and_then(value_text),
            low: row.get("low").and_then(value_text),
            close: row.get("close").and_then(value_text),
            volume: row.get("volume").and_then(value_text),
            amount: row.get("amount").and_then(value_text),
            raw_payload: row,
        })
        .collect())
}

fn reject_adjusted_payload(value: &Value) -> Result<(), DataError> {
    match value {
        Value::Object(fields) => {
            for (field, value) in fields {
                let field = field.to_ascii_lowercase();
                if matches!(
                    field.as_str(),
                    "adjust" | "adjusttype" | "adjustment" | "fqt" | "pricebasis"
                ) && !is_unadjusted_marker(value)
                {
                    return Err(error(
                        "adjusted_data",
                        format!("provider payload marks {field} as adjusted"),
                    ));
                }
                reject_adjusted_payload(value)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                reject_adjusted_payload(value)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn is_unadjusted_marker(value: &Value) -> bool {
    value.is_null()
        || value.as_str().is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "" | "0" | "none" | "unadjusted"
            )
        })
        || value.as_i64() == Some(0)
}

fn parse_corporate_actions_response(response: &[u8]) -> Result<(Vec<Value>, u64), DataError> {
    let payload: Value = serde_json::from_slice(response).map_err(|value| {
        error(
            "decode",
            format!("corporate-action response JSON is invalid: {value}"),
        )
    })?;
    let rows = payload
        .pointer("/result/data")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            error(
                "decode",
                "corporate-action response has no result.data array",
            )
        })?;
    let pages = payload
        .pointer("/result/pages")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .max(1);
    Ok((rows.clone(), pages))
}

fn action_security_code(row: &Value) -> Option<String> {
    row.get("SECURITY_CODE")
        .filter(|value| !value.is_null())
        .and_then(value_text)
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn action_date_text(row: &Value, field: &str) -> Option<String> {
    row.get(field)
        .filter(|value| !value.is_null())
        .and_then(value_text)
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn quarantined_corporate_action(
    instrument: &InstrumentId,
    provider_symbol: &str,
    retrieved_at_ms: i64,
    effective_at_ms: Option<i64>,
    announced_at_ms: Option<i64>,
    raw_payload: Value,
) -> AshareCorporateAction {
    AshareCorporateAction {
        instrument: instrument.clone(),
        provider_symbol: provider_symbol.into(),
        kind: AshareCorporateActionKind::Unknown,
        effective_at_ms,
        announced_at_ms,
        available_at_ms: retrieved_at_ms,
        cash_per_share: None,
        shares_per_share: None,
        raw_payload,
    }
}

fn raw_decimal_value(row: &Value, field: &str) -> Result<Option<String>, DataError> {
    let Some(value) = row.get(field).filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let text = value_text(value).ok_or_else(|| {
        error(
            "invalid_decimal",
            format!("corporate-action field {field} is invalid"),
        )
    })?;
    optional_decimal(field, Some(&text))
}

fn exact_spot_decimal(
    value: Option<&str>,
    field: &str,
    diagnostics: &mut AshareRequestDiagnostics,
) -> Option<String> {
    match optional_decimal(field, value) {
        Ok(value) => value,
        Err(error) => {
            diagnostics.notes.push(format!(
                "current spot {field} value was quarantined: {}",
                error.message
            ));
            None
        }
    }
}

fn sum_raw_decimals(fields: &[&str], row: &Value) -> Result<Option<String>, DataError> {
    let mut total = Decimal::ZERO;
    let mut present = false;
    for field in fields {
        if let Some(value) = raw_decimal_value(row, field)? {
            total += Decimal::from_str(&value).map_err(|error| {
                DataError::new(ASHARE_SRC, "invalid_decimal", error.to_string())
            })?;
            present = true;
        }
    }
    Ok((present && total != Decimal::ZERO).then(|| total.to_string()))
}

fn value_text(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.as_number().map(ToString::to_string))
}

fn action_date_ms(
    instrument: &InstrumentId,
    value: Option<&str>,
    diagnostics: &mut AshareRequestDiagnostics,
    field: &str,
) -> Option<i64> {
    let value = value?;
    match date_to_midnight_ms(instrument, value) {
        Ok(timestamp) => Some(timestamp),
        Err(error) => {
            diagnostics.notes.push(format!(
                "corporate action {field} date is invalid: {value} ({})",
                error.message
            ));
            None
        }
    }
}

fn interval_ms(interval: BarInterval) -> Option<i64> {
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

fn required_decimal(field: &str, value: Option<&str>) -> Result<String, DataError> {
    let value = value
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            error(
                "invalid_decimal",
                format!("provider field {field} is missing"),
            )
        })?;
    Decimal::from_str(value).map_err(|error| {
        DataError::new(
            ASHARE_SRC,
            "invalid_decimal",
            format!("provider field {field} is not an exact Decimal: {error}"),
        )
    })?;
    Ok(value.into())
}

fn optional_decimal(field: &str, value: Option<&str>) -> Result<Option<String>, DataError> {
    value
        .map(|value| required_decimal(field, Some(value)))
        .transpose()
}

fn non_empty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

fn sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn error(code: impl Into<String>, message: impl Into<String>) -> DataError {
    DataError::new(ASHARE_SRC, code, message)
}

fn with_raw_evidence(mut error: DataError, response: &[u8]) -> DataError {
    error.response_sha256 = Some(sha256(response));
    error.raw_response = Some(response.to_vec());
    error
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|value| i64::try_from(value.as_millis()).ok())
        .unwrap_or(0)
}

fn raw_http_client(policy: AshareRequestPolicy) -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_millis(policy.timeout_ms.max(1)))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread::{self, JoinHandle},
    };

    use super::{
        ASHARE_RAW_WIRE_ADAPTER_VERSION, AshareClient, AshareCorporateActionKind,
        AshareRequestPolicy, normalize_provider_instrument, sha256,
    };
    use crate::market::{InstrumentId, LocalTimeDisambiguation, PriceBasis, SessionPhase, Venue};
    use crate::{BarInterval, HistoricalBarRange, InstrumentStatus};

    #[test]
    fn provider_symbol_maps_to_one_venue_and_rejects_ambiguous_codes() {
        let (venue, code) = normalize_provider_instrument("sh600000", "600000").unwrap();
        assert_eq!(venue.id, "sse");
        assert_eq!(code, "600000");

        let error = normalize_provider_instrument("sz600000", "600000").unwrap_err();
        assert_eq!(error.code, "ambiguous_mapping");
    }

    #[test]
    fn ordinary_a_share_code_mappings_cover_both_exchanges() {
        assert_eq!(
            normalize_provider_instrument("sz000001", "000001")
                .unwrap()
                .0
                .id,
            "szse"
        );
        assert_eq!(
            normalize_provider_instrument("sz300750", "300750")
                .unwrap()
                .0
                .id,
            "szse"
        );
        assert_eq!(
            normalize_provider_instrument("sh688001", "688001")
                .unwrap()
                .0
                .id,
            "sse"
        );
        assert!(normalize_provider_instrument("bj830001", "830001").is_err());
    }

    #[test]
    fn adjusted_provider_payloads_are_rejected() {
        let adjusted: serde_json::Value = serde_json::from_str(include_str!(
            "../../../fixtures/a-share/upstream/daily-kline-adjusted.json"
        ))
        .unwrap();
        let error = super::reject_adjusted_payload(&adjusted).unwrap_err();
        assert_eq!(error.code, "adjusted_data");
        super::reject_adjusted_payload(&serde_json::json!({
            "adjust": "",
            "fqt": 0
        }))
        .unwrap();
    }

    #[tokio::test]
    async fn local_mock_covers_provenance_status_calendar_bars_actions_and_retry() {
        let (base_url, server) = serve_mock(vec![
            ("getHQNodeStockCount", "throttled", 503),
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
                "stock/kline/get",
                include_str!("../../../fixtures/a-share/upstream/daily-kline.json"),
                200,
            ),
            (
                "KLineData",
                include_str!("../../../fixtures/a-share/upstream/minute.jsonp"),
                200,
            ),
            (
                "klc_td_sh.txt",
                include_str!("../../../fixtures/a-share/upstream/trade-dates.txt"),
                200,
            ),
            (
                "data/v1/get",
                include_str!("../../../fixtures/a-share/upstream/corporate-actions.json"),
                200,
            ),
        ]);
        let client = AshareClient::with_mock_and_policy(
            base_url,
            AshareRequestPolicy {
                max_attempts: 2,
                timeout_ms: 1_000,
                retry_delay_ms: 1,
            },
        );
        let retrieved_at_ms = 1_704_211_200_000;
        let master = client
            .acquire_instrument_master_at(retrieved_at_ms)
            .await
            .unwrap();
        assert_eq!(master.actual_upstream, "Sina Finance");
        assert_eq!(
            master.method,
            "stock_zh_a_spot (akshare-rs identity) + raw-wire Market_Center.getHQNodeData (Sina hs_a current values)"
        );
        assert_eq!(master.diagnostics.request_count, 3);
        assert_eq!(master.diagnostics.retry_count, 1);
        assert_eq!(master.diagnostics.response_statuses, vec![200]);
        assert_eq!(master.instruments.len(), 2);
        assert_eq!(master.instruments[1].status, InstrumentStatus::Suspended);
        assert_eq!(
            master.instruments[0].current_price.as_deref(),
            Some("10.25")
        );
        assert!(!master.response_sha256.is_empty());

        let range = HistoricalBarRange {
            start_time_ms: 1_704_124_800_000,
            end_time_ms: retrieved_at_ms,
        };
        let sse_instrument =
            InstrumentId::new(Venue::china_a_share("sse").unwrap(), "600000").unwrap();
        let daily = client
            .acquire_bars(
                sse_instrument.clone(),
                BarInterval::OneDay,
                range,
                retrieved_at_ms,
            )
            .await
            .unwrap();
        assert_eq!(daily.actual_upstream, "Eastmoney");
        assert_eq!(daily.bars[0].open.as_deref(), Some("10.00"));
        assert_eq!(daily.bars[0].close.as_deref(), Some("10.25"));
        assert_eq!(daily.bars[0].quote_volume.as_deref(), Some("1025.00"));
        assert_eq!(daily.bars[0].price_basis, PriceBasis::Unadjusted);
        assert_eq!(
            daily.response_sha256s[0],
            sha256(include_bytes!(
                "../../../fixtures/a-share/upstream/daily-kline.json"
            ))
        );
        assert!(daily.invalid_bars.is_empty());

        let minute = client
            .acquire_bars(
                sse_instrument.clone(),
                BarInterval::OneMinute,
                range,
                retrieved_at_ms,
            )
            .await
            .unwrap();
        assert_eq!(minute.actual_upstream, "Sina Finance");
        assert_eq!(minute.bars[0].quote_volume.as_deref(), Some("51.25"));
        assert_eq!(minute.bars[0].open.as_deref(), Some("10.20"));

        let calendar = client
            .acquire_calendar(range, retrieved_at_ms)
            .await
            .unwrap();
        assert_eq!(calendar.snapshots.len(), 2);
        assert_eq!(
            calendar.response_sha256,
            sha256(include_bytes!(
                "../../../fixtures/a-share/upstream/trade-dates.txt"
            ))
        );
        assert!(
            calendar
                .connector_version
                .contains(ASHARE_RAW_WIRE_ADAPTER_VERSION)
        );
        assert!(
            calendar.snapshots[0]
                .is_scheduled_non_trading(1_704_168_000_000)
                .unwrap()
        );
        let venue = &calendar.snapshots[0].venue;
        let auction = venue
            .resolve_local_time(
                chrono::NaiveDate::from_ymd_opt(2024, 1, 2)
                    .unwrap()
                    .and_hms_opt(9, 20, 0)
                    .unwrap(),
                LocalTimeDisambiguation::Reject,
            )
            .unwrap();
        assert_eq!(
            calendar.snapshots[0].session_phase_at(auction).unwrap(),
            SessionPhase::Auction
        );
        let break_time = venue
            .resolve_local_time(
                chrono::NaiveDate::from_ymd_opt(2024, 1, 2)
                    .unwrap()
                    .and_hms_opt(11, 45, 0)
                    .unwrap(),
                LocalTimeDisambiguation::Reject,
            )
            .unwrap();
        assert_eq!(
            calendar.snapshots[0].session_phase_at(break_time).unwrap(),
            SessionPhase::Break
        );

        let actions = client
            .acquire_corporate_actions(sse_instrument, retrieved_at_ms)
            .await
            .unwrap();
        assert_eq!(actions.actual_upstream, "Eastmoney");
        assert_eq!(
            actions.records[0].kind,
            AshareCorporateActionKind::CashDividend
        );
        assert_eq!(actions.records[0].cash_per_share.as_deref(), Some("0.25"));
        assert_eq!(actions.records[0].shares_per_share, None);
        assert!(actions.invalid_records.is_empty());
        assert!(actions.records[0].effective_at_ms.is_some());
        assert!(actions.records[0].announced_at_ms.is_some());
        assert_eq!(actions.records[0].available_at_ms, retrieved_at_ms);

        server.join().unwrap();
    }

    #[tokio::test]
    async fn raw_wire_rows_are_retained_when_exact_decimal_parsing_fails() {
        let (base_url, server) = serve_mock(vec![(
            "stock/kline/get",
            include_str!("../../../fixtures/a-share/upstream/daily-kline-invalid.json"),
            200,
        )]);
        let client = AshareClient::with_mock(base_url);
        let instrument = InstrumentId::new(Venue::china_a_share("sse").unwrap(), "600000").unwrap();
        let acquisition = client
            .acquire_bars(
                instrument,
                BarInterval::OneDay,
                HistoricalBarRange {
                    start_time_ms: 1_704_124_800_000,
                    end_time_ms: 1_704_384_000_000,
                },
                1_704_384_000_000,
            )
            .await
            .unwrap();
        assert_eq!(acquisition.bars.len(), 1);
        assert_eq!(acquisition.invalid_bars.len(), 1);
        assert_eq!(
            acquisition.invalid_bars[0].raw_payload.as_str(),
            Some("2024-01-03,not-a-decimal,10.30,10.40,10.10,100,1030.00")
        );
        server.join().unwrap();
    }

    #[tokio::test]
    async fn empty_history_retains_raw_evidence_as_unconfirmed_availability() {
        let (base_url, server) = serve_mock(vec![(
            "stock/kline/get",
            include_str!("../../../fixtures/a-share/upstream/daily-kline-empty.json"),
            200,
        )]);
        let client = AshareClient::with_mock(base_url);
        let instrument = InstrumentId::new(Venue::china_a_share("sse").unwrap(), "600000").unwrap();
        let acquisition = client
            .acquire_bars(
                instrument,
                BarInterval::OneDay,
                HistoricalBarRange {
                    start_time_ms: 1_704_124_800_000,
                    end_time_ms: 1_704_384_000_000,
                },
                1_704_384_000_000,
            )
            .await
            .unwrap();
        assert!(acquisition.bars.is_empty());
        assert!(acquisition.invalid_bars.is_empty());
        assert_eq!(acquisition.raw_responses.len(), 1);
        assert!(
            acquisition
                .limitations
                .iter()
                .any(|value| value.contains("availability is unconfirmed"))
        );
        server.join().unwrap();
    }

    #[tokio::test]
    async fn corporate_action_rows_are_quarantined_when_exact_decimal_parsing_fails() {
        let (base_url, server) = serve_mock(vec![(
            "data/v1/get",
            include_str!("../../../fixtures/a-share/upstream/corporate-actions-invalid.json"),
            200,
        )]);
        let client = AshareClient::with_mock(base_url);
        let instrument = InstrumentId::new(Venue::china_a_share("sse").unwrap(), "600000").unwrap();
        let acquisition = client
            .acquire_corporate_actions(instrument, 1_704_384_000_000)
            .await
            .unwrap();
        assert!(acquisition.records.is_empty());
        assert_eq!(acquisition.invalid_records.len(), 1);
        assert!(acquisition.invalid_records[0].raw_payload.is_object());
        server.join().unwrap();
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
