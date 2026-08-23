use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use adaq_trading_crypto::realtime::OkxWs;
use adaq_trading_crypto::{Config, Exchange, Realtime};
use futures_util::{SinkExt, StreamExt};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use tokio_tungstenite::tungstenite::Message;

#[cfg(feature = "deferred-equity")]
pub mod a_share;
pub mod alpaca;
pub mod market;
#[cfg(feature = "deferred-equity")]
pub mod stock_us;

pub(crate) const OKX_SRC: &str = "okx";
pub const OKX_CONNECTOR_VERSION: &str = "adaq-data-core-okx-v1";

/// Process-wide unified crypto engine used by the offline mapping tests.
/// `adaq_trading_crypto::adapters::Okx::new` performs no I/O, so a single
/// shared handle is safe and cheap.
#[allow(dead_code)]
fn engine() -> &'static adaq_trading_crypto::adapters::Okx {
    static ENGINE: std::sync::OnceLock<adaq_trading_crypto::adapters::Okx> =
        std::sync::OnceLock::new();
    ENGINE.get_or_init(|| {
        adaq_trading_crypto::adapters::Okx::new(adaq_trading_crypto::Config::new())
            .expect("adaq-trading-crypto Okx adapter builds with default config")
    })
}
const OKX_BASE_URL: &str = "https://www.okx.com";
const OKX_WS_URL: &str = "wss://ws.okx.com:8443/ws/v5/public";
const OKX_BUSINESS_WS_URL: &str = "wss://ws.okx.com:8443/ws/v5/business";
const OKX_WS_MAX_RETRY_SECONDS: u64 = 15;
// Rebuild the crate websocket when a subscription stops producing updates.
// `adaq-trading-crypto` reconnects its socket internally, but a reconnect can
// retain the old sent-subscription set and leave a channel quiet forever.
const OKX_STREAM_UPDATE_TIMEOUT_SECONDS: u64 = 30;
pub const OKX_MAX_STREAM_SYMBOLS: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OkxRequestPolicy {
    pub max_attempts: u8,
    pub min_delay_ms: u64,
    pub retry_delay_ms: u64,
    pub max_retry_delay_ms: u64,
}

impl Default for OkxRequestPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            min_delay_ms: 100,
            retry_delay_ms: 250,
            max_retry_delay_ms: 5_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct OkxRequestDiagnostics {
    pub request_count: u32,
    pub retry_count: u32,
    pub backoff_ms: u64,
    #[serde(default)]
    pub response_statuses: Vec<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BarInterval {
    #[serde(rename = "1s")]
    OneSecond,
    #[serde(rename = "1m")]
    OneMinute,
    #[serde(rename = "3m")]
    ThreeMinutes,
    #[serde(rename = "5m")]
    FiveMinutes,
    #[serde(rename = "15m")]
    FifteenMinutes,
    #[serde(rename = "30m")]
    ThirtyMinutes,
    #[serde(rename = "1h")]
    OneHour,
    #[serde(rename = "2h")]
    TwoHours,
    #[serde(rename = "4h")]
    FourHours,
    #[serde(rename = "6h")]
    SixHours,
    #[serde(rename = "12h")]
    TwelveHours,
    #[serde(rename = "1d")]
    OneDay,
    #[serde(rename = "2d")]
    TwoDays,
    #[serde(rename = "3d")]
    ThreeDays,
    #[serde(rename = "5d")]
    FiveDays,
    #[serde(rename = "1w")]
    OneWeek,
    #[serde(rename = "1mo")]
    OneMonth,
    #[serde(rename = "3mo")]
    ThreeMonths,
}

impl BarInterval {
    pub const ALL: [Self; 18] = [
        Self::OneSecond,
        Self::OneMinute,
        Self::ThreeMinutes,
        Self::FiveMinutes,
        Self::FifteenMinutes,
        Self::ThirtyMinutes,
        Self::OneHour,
        Self::TwoHours,
        Self::FourHours,
        Self::SixHours,
        Self::TwelveHours,
        Self::OneDay,
        Self::TwoDays,
        Self::ThreeDays,
        Self::FiveDays,
        Self::OneWeek,
        Self::OneMonth,
        Self::ThreeMonths,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OneSecond => "1s",
            Self::OneMinute => "1m",
            Self::ThreeMinutes => "3m",
            Self::FiveMinutes => "5m",
            Self::FifteenMinutes => "15m",
            Self::ThirtyMinutes => "30m",
            Self::OneHour => "1h",
            Self::TwoHours => "2h",
            Self::FourHours => "4h",
            Self::SixHours => "6h",
            Self::TwelveHours => "12h",
            Self::OneDay => "1d",
            Self::TwoDays => "2d",
            Self::ThreeDays => "3d",
            Self::FiveDays => "5d",
            Self::OneWeek => "1w",
            Self::OneMonth => "1mo",
            Self::ThreeMonths => "3mo",
        }
    }

    pub const fn okx_bar(self) -> &'static str {
        match self {
            Self::OneSecond => "1s",
            Self::OneMinute => "1m",
            Self::ThreeMinutes => "3m",
            Self::FiveMinutes => "5m",
            Self::FifteenMinutes => "15m",
            Self::ThirtyMinutes => "30m",
            Self::OneHour => "1H",
            Self::TwoHours => "2H",
            Self::FourHours => "4H",
            Self::SixHours => "6Hutc",
            Self::TwelveHours => "12Hutc",
            Self::OneDay => "1Dutc",
            Self::TwoDays => "2Dutc",
            Self::ThreeDays => "3Dutc",
            Self::FiveDays => "5Dutc",
            Self::OneWeek => "1Wutc",
            Self::OneMonth => "1Mutc",
            Self::ThreeMonths => "3Mutc",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OhlcvBar {
    pub open_time_ms: i64,
    #[serde(with = "rust_decimal::serde::str")]
    pub open: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub high: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub low: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub close: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub base_volume: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub quote_volume: Decimal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BarSeries {
    pub src: String,
    pub code: String,
    pub interval: BarInterval,
    pub bars: Vec<OhlcvBar>,
    pub gaps: Vec<BarGap>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BarAcquisition {
    pub series: BarSeries,
    pub retrieved_at_ms: i64,
    pub response_sha256s: Vec<String>,
    pub diagnostics: OkxRequestDiagnostics,
    /// Raw OKX candle rows aligned with `series.bars` by index.
    pub raw_payloads: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BarSnapshot {
    pub src: String,
    pub code: String,
    pub interval: BarInterval,
    pub bar: OhlcvBar,
    pub closed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BarSubscription {
    pub code: String,
    pub interval: BarInterval,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "event",
    content = "data"
)]
pub enum BarStreamEvent {
    Connected,
    Snapshot(BarSnapshot),
    Error(DataError),
    Reconnecting { delay_ms: u64 },
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BarGap {
    pub start_time_ms: i64,
    pub end_time_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoricalBarRange {
    pub start_time_ms: i64,
    pub end_time_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InstrumentStatus {
    Live,
    Suspended,
    PreOpen,
    Test,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpotInstrument {
    pub src: String,
    pub code: String,
    pub base_asset: String,
    pub quote_asset: String,
    pub status: InstrumentStatus,
    pub listing_time_ms: Option<i64>,
    pub continuous_trading_time_ms: Option<i64>,
    #[serde(with = "rust_decimal::serde::str")]
    pub price_increment: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub quantity_increment: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub minimum_quantity: Decimal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstrumentMasterAcquisition {
    pub retrieved_at_ms: i64,
    pub response_sha256: String,
    pub connector_version: String,
    pub diagnostics: OkxRequestDiagnostics,
    pub instruments: Vec<SpotInstrument>,
    #[serde(default)]
    pub quote_volume_24h_usdt: BTreeMap<String, Decimal>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TickerSnapshot {
    pub src: String,
    pub code: String,
    #[serde(with = "rust_decimal::serde::str")]
    pub last: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub last_quantity: Decimal,
    #[serde(with = "rust_decimal::serde::str_option")]
    pub ask_price: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::str_option")]
    pub ask_quantity: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::str_option")]
    pub bid_price: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::str_option")]
    pub bid_quantity: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::str")]
    pub open_24h: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub high_24h: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub low_24h: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub base_volume_24h: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub quote_volume_24h: Decimal,
    pub timestamp_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "event",
    content = "data"
)]
pub enum TickerStreamEvent {
    Connected,
    Snapshot(TickerSnapshot),
    Error(DataError),
    Reconnecting { delay_ms: u64 },
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MarketTradeSide {
    Buy,
    Sell,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketTrade {
    pub src: String,
    pub code: String,
    pub trade_id: String,
    #[serde(with = "rust_decimal::serde::str")]
    pub price: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub quantity: Decimal,
    pub side: MarketTradeSide,
    pub timestamp_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderBookLevel {
    #[serde(with = "rust_decimal::serde::str")]
    pub price: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub quantity: Decimal,
    pub order_count: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Level2Snapshot {
    pub src: String,
    pub code: String,
    pub asks: Vec<OrderBookLevel>,
    pub bids: Vec<OrderBookLevel>,
    pub timestamp_ms: i64,
    pub checksum: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "event",
    content = "data"
)]
pub enum TradeStreamEvent {
    Connected,
    Snapshot(MarketTrade),
    Error(DataError),
    Reconnecting { delay_ms: u64 },
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "event",
    content = "data"
)]
pub enum Level2StreamEvent {
    Connected,
    Snapshot(Level2Snapshot),
    Error(DataError),
    Reconnecting { delay_ms: u64 },
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("{message}")]
pub struct DataError {
    pub src: String,
    pub code: String,
    pub message: String,
    #[serde(skip)]
    pub raw_response: Option<Vec<u8>>,
    #[serde(skip)]
    pub response_sha256: Option<String>,
}

impl DataError {
    pub fn new(
        src: impl Into<String>,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            src: src.into(),
            code: code.into(),
            message: message.into(),
            raw_response: None,
            response_sha256: None,
        }
    }

    fn okx(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(OKX_SRC, code, message)
    }
}

#[derive(Clone)]
pub struct OkxClient {
    /// Underlying unified crypto client from `adaq-trading-crypto`. This is the
    /// real data source for REST market data (ticker, instruments, candles):
    /// `OkxClient` delegates to its `Exchange` trait implementation instead of
    /// hand-rolling OKX REST calls.
    engine: Arc<adaq_trading_crypto::adapters::Okx>,
    http: reqwest::Client,
    base_url: String,
    /// Retained for API compatibility. Live WebSocket streams are served by
    /// `adaq-trading-crypto`'s `OkxWs` (`Realtime` trait), which owns its own
    /// connection URLs, so these are no longer read after the crate took over
    /// the streaming layer.
    #[allow(dead_code)]
    ws_url: String,
    #[allow(dead_code)]
    business_ws_url: String,
    policy: OkxRequestPolicy,
    next_request_at: Arc<Mutex<Instant>>,
}

impl Default for OkxClient {
    fn default() -> Self {
        Self::new_with_all_urls(OKX_BASE_URL, OKX_WS_URL, OKX_BUSINESS_WS_URL)
    }
}

impl OkxClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self::new_with_urls(base_url, OKX_WS_URL)
    }

    pub fn new_with_urls(base_url: impl Into<String>, ws_url: impl Into<String>) -> Self {
        Self::new_with_all_urls(base_url, ws_url, OKX_BUSINESS_WS_URL)
    }

    pub fn new_with_policy(base_url: impl Into<String>, policy: OkxRequestPolicy) -> Self {
        Self::new_with_all_urls_and_policy(base_url, OKX_WS_URL, OKX_BUSINESS_WS_URL, policy)
    }

    pub fn new_with_urls_and_policy(
        base_url: impl Into<String>,
        ws_url: impl Into<String>,
        business_ws_url: impl Into<String>,
        policy: OkxRequestPolicy,
    ) -> Self {
        Self::new_with_all_urls_and_policy(base_url, ws_url, business_ws_url, policy)
    }

    fn new_with_all_urls(
        base_url: impl Into<String>,
        ws_url: impl Into<String>,
        business_ws_url: impl Into<String>,
    ) -> Self {
        Self::new_with_all_urls_and_policy(
            base_url,
            ws_url,
            business_ws_url,
            OkxRequestPolicy::default(),
        )
    }

    fn new_with_all_urls_and_policy(
        base_url: impl Into<String>,
        ws_url: impl Into<String>,
        business_ws_url: impl Into<String>,
        policy: OkxRequestPolicy,
    ) -> Self {
        let base_url = base_url.into().trim_end_matches('/').to_owned();
        let mut config = adaq_trading_crypto::Config::new();
        config.max_retries = u32::from(policy.max_attempts.saturating_sub(1));
        config.enable_rate_limit = false;
        let engine_base_url = format!("{base_url}/api/v5");
        let engine = adaq_trading_crypto::adapters::Okx::with_endpoints(
            config,
            OKX_SRC,
            &engine_base_url,
            0,
        )
        .expect("adaq-trading-crypto Okx adapter builds with configured endpoint");
        Self {
            engine: Arc::new(engine),
            http: reqwest::Client::new(),
            base_url,
            ws_url: ws_url.into(),
            business_ws_url: business_ws_url.into(),
            policy,
            next_request_at: Arc::new(Mutex::new(Instant::now())),
        }
    }

    /// Borrow the underlying unified crypto client. Exposed for callers that
    /// want to reach the full `adaq-trading-crypto` surface (e.g. other
    /// exchanges) through the same handle ADAQ already holds.
    pub fn engine(&self) -> &adaq_trading_crypto::adapters::Okx {
        &self.engine
    }

    async fn get_envelope<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(String, String)],
    ) -> Result<(OkxEnvelope<T>, OkxHttpResponse), DataError> {
        let response = self.get_bytes(path, query).await?;
        let payload = serde_json::from_slice(&response.bytes)
            .map_err(|error| DataError::okx("invalid_response", error.to_string()))?;
        Ok((payload, response))
    }

    async fn get_bytes(
        &self,
        path: &str,
        query: &[(String, String)],
    ) -> Result<OkxHttpResponse, DataError> {
        let max_attempts = self.policy.max_attempts.max(1);
        let mut retry_delay_ms = self.policy.retry_delay_ms;
        let mut diagnostics = OkxRequestDiagnostics::default();

        for attempt in 0..max_attempts {
            diagnostics.request_count += 1;
            self.wait_for_rate_limit().await?;
            let response = match self
                .http
                .get(format!("{}{}", self.base_url, path))
                .query(query)
                .send()
                .await
            {
                Ok(response) => response,
                Err(_error) if attempt + 1 < max_attempts => {
                    diagnostics.retry_count += 1;
                    diagnostics.backoff_ms = diagnostics.backoff_ms.max(retry_delay_ms);
                    tokio::time::sleep(Duration::from_millis(retry_delay_ms)).await;
                    retry_delay_ms =
                        next_retry_delay(retry_delay_ms, self.policy.max_retry_delay_ms);
                    continue;
                }
                Err(error) => {
                    return Err(DataError::okx("transport", error.to_string()));
                }
            };
            let status = response.status();
            diagnostics.response_statuses.push(status.as_u16());
            let retryable =
                status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error();
            let retry_after_ms = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok())
                .map(|seconds| seconds.saturating_mul(1_000));
            let bytes = response
                .bytes()
                .await
                .map_err(|error| DataError::okx("transport", error.to_string()))?;
            if retryable && attempt + 1 < max_attempts {
                diagnostics.retry_count += 1;
                let delay_ms = retry_after_ms.unwrap_or(retry_delay_ms);
                diagnostics.backoff_ms = diagnostics.backoff_ms.max(delay_ms);
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                retry_delay_ms = next_retry_delay(retry_delay_ms, self.policy.max_retry_delay_ms);
                continue;
            }
            if !status.is_success() {
                return Err(DataError::okx(
                    "http_status",
                    format!("OKX returned HTTP {status}"),
                ));
            }
            return Ok(OkxHttpResponse {
                bytes: bytes.to_vec(),
                diagnostics,
            });
        }

        unreachable!("OKX request attempts is always at least one")
    }

    async fn wait_for_rate_limit(&self) -> Result<(), DataError> {
        let now = Instant::now();
        let (wait, next) = {
            let mut next_request_at = self
                .next_request_at
                .lock()
                .map_err(|_| DataError::okx("internal", "OKX rate-limit gate is poisoned"))?;
            let next = (*next_request_at).max(now);
            let wait = next.saturating_duration_since(now);
            let scheduled = next
                .checked_add(Duration::from_millis(self.policy.min_delay_ms))
                .unwrap_or(next);
            *next_request_at = scheduled;
            (wait, scheduled)
        };
        if wait > Duration::ZERO {
            tokio::time::sleep(wait).await;
        }
        let _ = next;
        Ok(())
    }

    pub async fn get_ticker(&self, code: &str) -> Result<TickerSnapshot, DataError> {
        validate_ticker_code(code)?;
        let ticker = self
            .engine
            .fetch_ticker(&dash_to_slash(code), Default::default())
            .await
            .map_err(map_crate_error)?;
        map_crate_ticker(&ticker, code)
    }

    pub async fn stream_ticker<F>(&self, code: &str, mut on_event: F) -> Result<(), DataError>
    where
        F: FnMut(TickerStreamEvent) -> bool,
    {
        self.stream_tickers(&[code.to_owned()], |event| on_event(event))
            .await
    }

    pub async fn stream_tickers<F>(
        &self,
        codes: &[String],
        mut on_event: F,
    ) -> Result<(), DataError>
    where
        F: FnMut(TickerStreamEvent) -> bool,
    {
        validate_ticker_codes(codes)?;
        let mut retry_seconds = 1;

        loop {
            let mut received_snapshot = false;
            let result = self
                .stream_tickers_once(codes, |event| {
                    if matches!(&event, TickerStreamEvent::Snapshot(_)) {
                        received_snapshot = true;
                    }
                    on_event(event)
                })
                .await;
            if received_snapshot {
                retry_seconds = 1;
            }
            let error = match result {
                Ok(()) => DataError::okx("connection_closed", "OKX ticker WebSocket closed"),
                Err(error) => error,
            };
            if !on_event(TickerStreamEvent::Error(error)) {
                break;
            }

            let delay_ms = retry_seconds * 1_000;
            if !on_event(TickerStreamEvent::Reconnecting { delay_ms }) {
                break;
            }
            tokio::time::sleep(Duration::from_secs(retry_seconds)).await;
            retry_seconds = (retry_seconds * 2).min(OKX_WS_MAX_RETRY_SECONDS);
        }

        Ok(())
    }

    /// Drives a unified realtime stream through `adaq-trading-crypto`'s `OkxWs`
    /// adapter. One task is spawned per `entries` key; each task loops on the
    /// crate's `watch_*` method (which blocks until the next update arrives)
    /// and pushes the translated events into a shared channel. The coordinator
    /// emits a single `Connected` event before the first snapshot and honors
    /// the caller's `on_event` return value (`false` stops the stream).
    async fn drive_symbol_stream<K, E, F>(
        &self,
        entries: &[K],
        mut on_event: F,
        fetch: impl Fn(Arc<OkxWs>, K) -> WatchTask<E> + Clone + Send + Sync + 'static,
    ) -> Result<(), DataError>
    where
        K: Clone + Send + Sync + 'static,
        E: StreamConnected + Send + 'static,
        F: FnMut(E) -> bool,
    {
        let ws = Arc::new(OkxWs::new(Config::new()).map_err(map_crate_error)?);
        let (tx, mut rx) =
            tokio::sync::mpsc::channel::<Result<Vec<E>, DataError>>(entries.len().max(1) * 8);
        let mut handles = Vec::with_capacity(entries.len());
        for entry in entries {
            let ws = ws.clone();
            let tx = tx.clone();
            let fetch = fetch.clone();
            let entry = entry.clone();
            handles.push(tokio::spawn(async move {
                loop {
                    let item = match tokio::time::timeout(
                        Duration::from_secs(OKX_STREAM_UPDATE_TIMEOUT_SECONDS),
                        fetch(ws.clone(), entry.clone()),
                    )
                    .await
                    {
                        Ok(item) => item,
                        Err(_) => Err(DataError::okx(
                            "stale_connection",
                            format!(
                                "OKX realtime subscription produced no update for {} seconds",
                                OKX_STREAM_UPDATE_TIMEOUT_SECONDS
                            ),
                        )),
                    };
                    let failed = item.is_err();
                    if tx.send(item).await.is_err() || failed {
                        return;
                    }
                }
            }));
        }

        let mut connected = false;
        while let Some(item) = rx.recv().await {
            match item {
                Ok(events) => {
                    for event in events {
                        if !connected {
                            connected = true;
                            if !on_event(E::connected()) {
                                for handle in &handles {
                                    handle.abort();
                                }
                                return Ok(());
                            }
                        }
                        if !on_event(event) {
                            for handle in &handles {
                                handle.abort();
                            }
                            return Ok(());
                        }
                    }
                }
                Err(err) => {
                    for handle in &handles {
                        handle.abort();
                    }
                    return Err(err);
                }
            }
        }
        for handle in &handles {
            handle.abort();
        }
        Ok(())
    }

    async fn stream_tickers_once<F>(&self, codes: &[String], on_event: F) -> Result<(), DataError>
    where
        F: FnMut(TickerStreamEvent) -> bool,
    {
        self.drive_symbol_stream(codes, on_event, |ws, code| {
            Box::pin(async move {
                let ticker = ws
                    .watch_ticker(&code, Default::default())
                    .await
                    .map_err(map_crate_error)?;
                Ok(vec![TickerStreamEvent::Snapshot(map_crate_ticker(
                    &ticker, &code,
                )?)])
            })
        })
        .await
    }

    pub async fn stream_trades<F>(&self, codes: &[String], mut on_event: F) -> Result<(), DataError>
    where
        F: FnMut(TradeStreamEvent) -> bool,
    {
        validate_ticker_codes(codes)?;
        let mut retry_seconds = 1;

        loop {
            let mut received_snapshot = false;
            let result = self
                .stream_trades_once_inner(codes, |event| {
                    if matches!(&event, TradeStreamEvent::Snapshot(_)) {
                        received_snapshot = true;
                    }
                    on_event(event)
                })
                .await;
            if received_snapshot {
                retry_seconds = 1;
            }
            let error = match result {
                Ok(()) => DataError::okx("connection_closed", "OKX trade WebSocket closed"),
                Err(error) => error,
            };
            if !on_event(TradeStreamEvent::Error(error)) {
                break;
            }
            let delay_ms = retry_seconds * 1_000;
            if !on_event(TradeStreamEvent::Reconnecting { delay_ms }) {
                break;
            }
            tokio::time::sleep(Duration::from_secs(retry_seconds)).await;
            retry_seconds = (retry_seconds * 2).min(OKX_WS_MAX_RETRY_SECONDS);
        }

        Ok(())
    }

    async fn stream_trades_once_inner<F>(
        &self,
        codes: &[String],
        on_event: F,
    ) -> Result<(), DataError>
    where
        F: FnMut(TradeStreamEvent) -> bool,
    {
        self.drive_symbol_stream(codes, on_event, |ws, code| {
            Box::pin(async move {
                let trades = ws
                    .watch_trades(&code, None, None, Default::default())
                    .await
                    .map_err(map_crate_error)?;
                trades
                    .iter()
                    .map(|trade| {
                        Ok::<_, DataError>(TradeStreamEvent::Snapshot(map_crate_trade(
                            trade, &code,
                        )?))
                    })
                    .collect::<Result<Vec<_>, DataError>>()
            })
        })
        .await
    }

    pub async fn stream_order_books<F>(
        &self,
        codes: &[String],
        mut on_event: F,
    ) -> Result<(), DataError>
    where
        F: FnMut(Level2StreamEvent) -> bool,
    {
        validate_ticker_codes(codes)?;
        let mut retry_seconds = 1;

        loop {
            let mut received_snapshot = false;
            let result = self
                .stream_order_books_once_inner(codes, |event| {
                    if matches!(&event, Level2StreamEvent::Snapshot(_)) {
                        received_snapshot = true;
                    }
                    on_event(event)
                })
                .await;
            if received_snapshot {
                retry_seconds = 1;
            }
            let error = match result {
                Ok(()) => DataError::okx("connection_closed", "OKX Level 2 WebSocket closed"),
                Err(error) => error,
            };
            if !on_event(Level2StreamEvent::Error(error)) {
                break;
            }
            let delay_ms = retry_seconds * 1_000;
            if !on_event(Level2StreamEvent::Reconnecting { delay_ms }) {
                break;
            }
            tokio::time::sleep(Duration::from_secs(retry_seconds)).await;
            retry_seconds = (retry_seconds * 2).min(OKX_WS_MAX_RETRY_SECONDS);
        }

        Ok(())
    }

    async fn stream_order_books_once_inner<F>(
        &self,
        codes: &[String],
        on_event: F,
    ) -> Result<(), DataError>
    where
        F: FnMut(Level2StreamEvent) -> bool,
    {
        self.drive_symbol_stream(codes, on_event, |ws, code| {
            Box::pin(async move {
                let book = ws
                    .watch_order_book(&code, None, Default::default())
                    .await
                    .map_err(map_crate_error)?;
                Ok(vec![Level2StreamEvent::Snapshot(map_crate_order_book(
                    &book, &code,
                )?)])
            })
        })
        .await
    }

    pub async fn stream_bar<F>(
        &self,
        code: &str,
        interval: BarInterval,
        mut on_event: F,
    ) -> Result<(), DataError>
    where
        F: FnMut(BarStreamEvent) -> bool,
    {
        self.stream_bars(
            &[BarSubscription {
                code: code.to_owned(),
                interval,
            }],
            |event| on_event(event),
        )
        .await
    }

    pub async fn stream_bars<F>(
        &self,
        subscriptions: &[BarSubscription],
        mut on_event: F,
    ) -> Result<(), DataError>
    where
        F: FnMut(BarStreamEvent) -> bool,
    {
        validate_bar_subscriptions(subscriptions)?;
        let mut retry_seconds = 1;

        loop {
            let mut received_snapshot = false;
            let result = self
                .stream_bars_once(subscriptions, |event| {
                    if matches!(&event, BarStreamEvent::Snapshot(_)) {
                        received_snapshot = true;
                    }
                    on_event(event)
                })
                .await;
            if received_snapshot {
                retry_seconds = 1;
            }
            let error = match result {
                Ok(()) => DataError::okx("connection_closed", "OKX bar WebSocket closed"),
                Err(error) => error,
            };
            if !on_event(BarStreamEvent::Error(error)) {
                break;
            }

            let delay_ms = retry_seconds * 1_000;
            if !on_event(BarStreamEvent::Reconnecting { delay_ms }) {
                break;
            }
            tokio::time::sleep(Duration::from_secs(retry_seconds)).await;
            retry_seconds = (retry_seconds * 2).min(OKX_WS_MAX_RETRY_SECONDS);
        }

        Ok(())
    }

    async fn stream_bars_once<F>(
        &self,
        subscriptions: &[BarSubscription],
        mut on_event: F,
    ) -> Result<(), DataError>
    where
        F: FnMut(BarStreamEvent) -> bool,
    {
        validate_bar_subscriptions(subscriptions)?;
        let mut socket = adaq_trading_crypto::realtime::ws::connect(
            &self.business_ws_url,
            &reqwest::header::HeaderMap::new(),
        )
        .await
        .map_err(map_crate_error)?;
        let args = subscriptions
            .iter()
            .map(|subscription| {
                serde_json::json!({
                    "channel": format!("candle{}", subscription.interval.okx_bar()),
                    "instId": subscription.code,
                })
            })
            .collect::<Vec<_>>();
        socket
            .send(Message::Text(
                serde_json::json!({ "op": "subscribe", "args": args })
                    .to_string()
                    .into(),
            ))
            .await
            .map_err(|error| DataError::okx("transport", error.to_string()))?;

        let heartbeat = Duration::from_secs(25);
        let mut awaiting_pong = false;
        let mut announced_connected = false;
        loop {
            let message = match tokio::time::timeout(heartbeat, socket.next()).await {
                Ok(Some(Ok(message))) => message,
                Ok(Some(Err(error))) => {
                    return Err(DataError::okx("transport", error.to_string()));
                }
                Ok(None) => {
                    return Err(DataError::okx(
                        "connection_closed",
                        "OKX bar WebSocket closed",
                    ));
                }
                Err(_) if awaiting_pong => {
                    return Err(DataError::okx(
                        "heartbeat_timeout",
                        "OKX bar WebSocket did not answer ping",
                    ));
                }
                Err(_) => {
                    socket
                        .send(Message::Text("ping".into()))
                        .await
                        .map_err(|error| DataError::okx("transport", error.to_string()))?;
                    awaiting_pong = true;
                    continue;
                }
            };

            match message {
                Message::Text(text) if text.as_str() == "pong" => awaiting_pong = false,
                Message::Text(text) => {
                    awaiting_pong = false;
                    let snapshots =
                        parse_realtime_bar_message(&self.engine, text.as_str(), subscriptions)?;
                    if !snapshots.is_empty() && !announced_connected {
                        if !on_event(BarStreamEvent::Connected) {
                            return Ok(());
                        }
                        announced_connected = true;
                    }
                    for snapshot in snapshots {
                        if !on_event(BarStreamEvent::Snapshot(snapshot)) {
                            return Ok(());
                        }
                    }
                }
                Message::Ping(payload) => {
                    socket
                        .send(Message::Pong(payload))
                        .await
                        .map_err(|error| DataError::okx("transport", error.to_string()))?;
                }
                Message::Pong(_) => awaiting_pong = false,
                Message::Close(_) => {
                    return Err(DataError::okx(
                        "connection_closed",
                        "OKX bar WebSocket closed",
                    ));
                }
                _ => {}
            }
        }
    }

    pub async fn get_bar_series(
        &self,
        code: &str,
        interval: BarInterval,
        limit: u16,
    ) -> Result<BarSeries, DataError> {
        if code.trim().is_empty() || !(1..=100).contains(&limit) {
            return Err(DataError::okx(
                "invalid_request",
                "code must be non-empty and limit must be between 1 and 100",
            ));
        }

        let page = self.fetch_bar_page(code, interval, None, limit).await?;
        build_bar_series(code, interval, page.bars)
    }

    pub async fn get_bar_series_range(
        &self,
        code: &str,
        interval: BarInterval,
        range: HistoricalBarRange,
    ) -> Result<BarSeries, DataError> {
        self.get_bar_series_range_with_progress(code, interval, range, |_, _| true)
            .await
    }

    pub async fn get_bar_series_range_with_progress(
        &self,
        code: &str,
        interval: BarInterval,
        range: HistoricalBarRange,
        mut on_progress: impl FnMut(usize, i64) -> bool,
    ) -> Result<BarSeries, DataError> {
        Ok(self
            .get_bar_series_range_with_evidence(code, interval, range, |downloaded, oldest| {
                on_progress(downloaded, oldest)
            })
            .await?
            .series)
    }

    pub async fn get_bar_series_range_with_evidence(
        &self,
        code: &str,
        interval: BarInterval,
        range: HistoricalBarRange,
        mut on_progress: impl FnMut(usize, i64) -> bool,
    ) -> Result<BarAcquisition, DataError> {
        self.get_bar_series_range_with_pages(code, interval, range, |_, downloaded, oldest| {
            on_progress(downloaded, oldest)
        })
        .await
    }

    pub async fn get_bar_series_range_with_pages(
        &self,
        code: &str,
        interval: BarInterval,
        range: HistoricalBarRange,
        mut on_page: impl FnMut(&[OhlcvBar], usize, i64) -> bool,
    ) -> Result<BarAcquisition, DataError> {
        self.get_bar_series_range_with_pages_and_payloads(
            code,
            interval,
            range,
            |bars, _, downloaded, oldest| on_page(bars, downloaded, oldest),
        )
        .await
    }

    pub async fn get_bar_series_range_with_pages_and_payloads(
        &self,
        code: &str,
        interval: BarInterval,
        range: HistoricalBarRange,
        mut on_page: impl FnMut(&[OhlcvBar], &[serde_json::Value], usize, i64) -> bool,
    ) -> Result<BarAcquisition, DataError> {
        if code.trim().is_empty() || range.start_time_ms >= range.end_time_ms {
            return Err(DataError::okx(
                "invalid_request",
                "code must be non-empty and bar range must be increasing",
            ));
        }

        let mut cursor = range.end_time_ms;
        let mut bars = Vec::new();
        let mut raw_payloads = Vec::new();
        let mut response_sha256s = Vec::new();
        let mut diagnostics = OkxRequestDiagnostics::default();
        loop {
            let page = self
                .fetch_bar_page(code, interval, Some(cursor), 100)
                .await?;
            let Some(oldest_open_time_ms) = page.oldest_open_time_ms else {
                break;
            };
            let page_bars = page.bars;
            let page_payloads = page.raw_payloads;
            bars.extend(page_bars.iter().cloned());
            raw_payloads.extend(page_payloads.iter().cloned());
            response_sha256s.push(page.response_sha256);
            diagnostics.request_count += page.diagnostics.request_count;
            diagnostics.retry_count += page.diagnostics.retry_count;
            diagnostics.backoff_ms = diagnostics.backoff_ms.max(page.diagnostics.backoff_ms);
            diagnostics
                .response_statuses
                .extend(page.diagnostics.response_statuses);
            if !on_page(&page_bars, &page_payloads, bars.len(), oldest_open_time_ms) {
                return Err(DataError::okx(
                    "cancelled",
                    "market data download cancelled",
                ));
            }
            if oldest_open_time_ms <= range.start_time_ms || page.row_count < 100 {
                break;
            }
            if oldest_open_time_ms >= cursor {
                return Err(DataError::okx(
                    "invalid_response",
                    "OKX bar pagination did not advance",
                ));
            }
            cursor = oldest_open_time_ms;
        }

        let mut raw_payloads_by_time = bars
            .iter()
            .zip(raw_payloads.iter())
            .map(|(bar, payload)| (bar.open_time_ms, payload.clone()))
            .collect::<std::collections::HashMap<_, _>>();
        let bars = bars
            .into_iter()
            .filter(|bar| {
                bar.open_time_ms >= range.start_time_ms && bar.open_time_ms < range.end_time_ms
            })
            .collect::<Vec<_>>();
        let series = build_bar_series(code, interval, bars)?;
        let raw_payloads = series
            .bars
            .iter()
            .map(|bar| {
                raw_payloads_by_time
                    .remove(&bar.open_time_ms)
                    .unwrap_or_default()
            })
            .collect();

        Ok(BarAcquisition {
            series,
            retrieved_at_ms: now_ms(),
            response_sha256s,
            diagnostics,
            raw_payloads,
        })
    }

    async fn fetch_bar_page(
        &self,
        code: &str,
        interval: BarInterval,
        after_time_ms: Option<i64>,
        limit: u16,
    ) -> Result<OkxBarPage, DataError> {
        let mut query = vec![
            ("instId".to_owned(), code.to_owned()),
            ("bar".to_owned(), interval.okx_bar().to_owned()),
        ];
        if let Some(after_time_ms) = after_time_ms {
            query.push(("after".to_owned(), after_time_ms.to_string()));
        }
        query.push(("limit".to_owned(), limit.to_string()));

        let (payload, response) = self
            .get_envelope::<Vec<Vec<String>>>("/api/v5/market/history-candles", &query)
            .await?;
        if payload.code != "0" {
            return Err(DataError::okx(payload.code, payload.msg));
        }

        let row_count = payload.data.len();
        let mut oldest_open_time_ms = None;
        let mut bars = Vec::with_capacity(row_count);
        let mut raw_payloads = Vec::with_capacity(row_count);
        for values in payload.data {
            let open_time_ms = values
                .first()
                .ok_or_else(|| DataError::okx("invalid_response", "OKX bar is empty"))?
                .parse()
                .map_err(|error| {
                    DataError::okx(
                        "invalid_response",
                        format!("invalid OKX timestamp: {error}"),
                    )
                })?;
            oldest_open_time_ms = Some(
                oldest_open_time_ms.map_or(open_time_ms, |oldest: i64| oldest.min(open_time_ms)),
            );
            let raw_payload = serde_json::Value::Array(
                values
                    .iter()
                    .cloned()
                    .map(serde_json::Value::String)
                    .collect(),
            );
            if let Some(bar) = parse_okx_bar(values)? {
                bars.push(bar);
                raw_payloads.push(raw_payload);
            }
        }
        Ok(OkxBarPage {
            bars,
            raw_payloads,
            row_count,
            oldest_open_time_ms,
            response_sha256: sha256_hex(&response.bytes),
            diagnostics: response.diagnostics,
        })
    }

    pub async fn list_spot_instruments(&self) -> Result<Vec<SpotInstrument>, DataError> {
        let markets = self.engine.fetch_markets().await.map_err(map_crate_error)?;
        let mut instruments = markets
            .iter()
            .filter(|market| market.spot == Some(true))
            .map(map_crate_market)
            .collect::<Result<Vec<_>, _>>()?;
        instruments.sort_unstable_by(|left, right| left.code.cmp(&right.code));
        Ok(instruments)
    }

    pub async fn list_spot_instrument_master(
        &self,
    ) -> Result<InstrumentMasterAcquisition, DataError> {
        self.list_spot_instrument_master_at(now_ms()).await
    }

    pub async fn list_spot_instrument_master_at(
        &self,
        retrieved_at_ms: i64,
    ) -> Result<InstrumentMasterAcquisition, DataError> {
        if retrieved_at_ms < 0 {
            return Err(DataError::okx(
                "invalid_request",
                "instrument master retrieval time must be non-negative",
            ));
        }
        // `Okx::fetch_markets` is the single source for the request and OKX
        // response parsing. The adapter itself sends `instType=SPOT`.
        let markets = self.engine.fetch_markets().await.map_err(map_crate_error)?;
        let response_sha256 = serde_json::to_vec(&markets)
            .map(|bytes| sha256_hex(&bytes))
            .map_err(|error| DataError::okx("serialization", error.to_string()))?;
        let mut instruments = markets
            .iter()
            .filter(|market| market.spot == Some(true))
            .map(map_crate_market)
            .collect::<Result<Vec<_>, _>>()?;
        instruments.sort_unstable_by(|left, right| left.code.cmp(&right.code));
        let tickers = self
            .engine
            .fetch_tickers(None, Default::default())
            .await
            .map_err(map_crate_error)?;
        let quote_volume_24h_usdt = instruments
            .iter()
            .filter_map(|instrument| {
                let symbol = dash_to_slash(&instrument.code);
                let ticker = tickers
                    .get(&symbol)
                    .or_else(|| tickers.get(&instrument.code))?;
                let quote_volume = ticker.quote_volume?;
                let volume_usdt = if instrument.quote_asset == "USDT"
                    || instrument.quote_asset == "USDC"
                    || instrument.quote_asset == "USD"
                {
                    quote_volume
                } else {
                    let conversion = format!("{}/USDT", instrument.quote_asset);
                    let conversion_ticker = tickers.get(&conversion)?;
                    quote_volume * conversion_ticker.last?
                };
                Some((instrument.code.clone(), volume_usdt))
            })
            .collect();
        Ok(InstrumentMasterAcquisition {
            retrieved_at_ms,
            response_sha256,
            connector_version: OKX_CONNECTOR_VERSION.into(),
            diagnostics: OkxRequestDiagnostics {
                request_count: 2,
                response_statuses: vec![200],
                ..Default::default()
            },
            instruments,
            quote_volume_24h_usdt,
        })
    }
}

struct OkxBarPage {
    bars: Vec<OhlcvBar>,
    raw_payloads: Vec<serde_json::Value>,
    row_count: usize,
    oldest_open_time_ms: Option<i64>,
    response_sha256: String,
    diagnostics: OkxRequestDiagnostics,
}

struct OkxHttpResponse {
    bytes: Vec<u8>,
    diagnostics: OkxRequestDiagnostics,
}

fn next_retry_delay(current_ms: u64, max_ms: u64) -> u64 {
    current_ms.saturating_mul(2).min(max_ms.max(current_ms))
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(0)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn normalize_bars(mut bars: Vec<OhlcvBar>) -> Result<Vec<OhlcvBar>, DataError> {
    bars.sort_unstable_by_key(|bar| bar.open_time_ms);
    let mut normalized: Vec<OhlcvBar> = Vec::with_capacity(bars.len());
    for bar in bars {
        match normalized.last() {
            Some(previous) if previous == &bar => continue,
            Some(previous) if previous.open_time_ms == bar.open_time_ms => {
                return Err(DataError::okx(
                    "conflicting_bar",
                    format!("conflicting OKX bars at {}", bar.open_time_ms),
                ));
            }
            _ => normalized.push(bar),
        }
    }
    Ok(normalized)
}

fn build_bar_series(
    code: &str,
    interval: BarInterval,
    bars: Vec<OhlcvBar>,
) -> Result<BarSeries, DataError> {
    if bars
        .iter()
        .any(|bar| !bar_time_aligned(interval, bar.open_time_ms))
    {
        return Err(DataError::okx(
            "invalid_timestamp",
            "OKX bar timestamp is not aligned to the requested interval",
        ));
    }
    let bars = normalize_bars(bars)?;
    let gaps = detect_bar_gaps(interval, &bars)?;
    Ok(BarSeries {
        src: OKX_SRC.to_owned(),
        code: code.to_owned(),
        interval,
        bars,
        gaps,
    })
}

fn bar_time_aligned(interval: BarInterval, open_time_ms: i64) -> bool {
    let fixed_ms = match interval {
        BarInterval::OneSecond => Some(1_000),
        BarInterval::OneMinute => Some(60_000),
        BarInterval::ThreeMinutes => Some(3 * 60_000),
        BarInterval::FiveMinutes => Some(5 * 60_000),
        BarInterval::FifteenMinutes => Some(15 * 60_000),
        BarInterval::ThirtyMinutes => Some(30 * 60_000),
        BarInterval::OneHour => Some(60 * 60_000),
        BarInterval::TwoHours => Some(2 * 60 * 60_000),
        BarInterval::FourHours => Some(4 * 60 * 60_000),
        BarInterval::SixHours => Some(6 * 60 * 60_000),
        BarInterval::TwelveHours => Some(12 * 60 * 60_000),
        BarInterval::OneDay => Some(24 * 60 * 60_000),
        BarInterval::TwoDays => Some(2 * 24 * 60 * 60_000),
        BarInterval::ThreeDays => Some(3 * 24 * 60 * 60_000),
        BarInterval::FiveDays => Some(5 * 24 * 60 * 60_000),
        BarInterval::OneWeek => Some(7 * 24 * 60 * 60_000),
        BarInterval::OneMonth | BarInterval::ThreeMonths => None,
    };
    if let Some(fixed_ms) = fixed_ms {
        return open_time_ms.rem_euclid(fixed_ms) == 0;
    }
    let Ok(datetime) = time::OffsetDateTime::from_unix_timestamp(open_time_ms.div_euclid(1_000))
    else {
        return false;
    };
    datetime.day() == 1
        && datetime.hour() == 0
        && datetime.minute() == 0
        && datetime.second() == 0
        && datetime.nanosecond() == 0
}

fn detect_bar_gaps(interval: BarInterval, bars: &[OhlcvBar]) -> Result<Vec<BarGap>, DataError> {
    let mut gaps = Vec::new();
    for adjacent in bars.windows(2) {
        let expected = next_bar_open_time_ms(adjacent[0].open_time_ms, interval)?;
        if expected < adjacent[1].open_time_ms {
            gaps.push(BarGap {
                start_time_ms: expected,
                end_time_ms: adjacent[1].open_time_ms,
            });
        }
    }
    Ok(gaps)
}

pub fn next_bar_open_time_ms(open_time_ms: i64, interval: BarInterval) -> Result<i64, DataError> {
    let fixed_seconds = match interval {
        BarInterval::OneSecond => Some(1),
        BarInterval::OneMinute => Some(60),
        BarInterval::ThreeMinutes => Some(3 * 60),
        BarInterval::FiveMinutes => Some(5 * 60),
        BarInterval::FifteenMinutes => Some(15 * 60),
        BarInterval::ThirtyMinutes => Some(30 * 60),
        BarInterval::OneHour => Some(60 * 60),
        BarInterval::TwoHours => Some(2 * 60 * 60),
        BarInterval::FourHours => Some(4 * 60 * 60),
        BarInterval::SixHours => Some(6 * 60 * 60),
        BarInterval::TwelveHours => Some(12 * 60 * 60),
        BarInterval::OneDay => Some(24 * 60 * 60),
        BarInterval::TwoDays => Some(2 * 24 * 60 * 60),
        BarInterval::ThreeDays => Some(3 * 24 * 60 * 60),
        BarInterval::FiveDays => Some(5 * 24 * 60 * 60),
        BarInterval::OneWeek => Some(7 * 24 * 60 * 60),
        BarInterval::OneMonth | BarInterval::ThreeMonths => None,
    };
    if let Some(seconds) = fixed_seconds {
        return open_time_ms
            .checked_add(seconds * 1_000)
            .ok_or_else(|| DataError::okx("invalid_response", "bar open time exceeds i64 range"));
    }

    let datetime = time::OffsetDateTime::from_unix_timestamp(open_time_ms.div_euclid(1_000))
        .map_err(|error| DataError::okx("invalid_response", error.to_string()))?;
    let months = match interval {
        BarInterval::OneMonth => 1,
        BarInterval::ThreeMonths => 3,
        _ => unreachable!(),
    };
    let month_index = datetime.year() * 12 + i32::from(u8::from(datetime.month())) - 1 + months;
    let year = month_index.div_euclid(12);
    let month = time::Month::try_from((month_index.rem_euclid(12) + 1) as u8)
        .map_err(|error| DataError::okx("invalid_response", error.to_string()))?;
    let next = time::Date::from_calendar_date(year, month, 1)
        .map_err(|error| DataError::okx("invalid_response", error.to_string()))?
        .midnight()
        .assume_utc()
        .unix_timestamp();
    next.checked_mul(1_000)
        .ok_or_else(|| DataError::okx("invalid_response", "bar open time exceeds i64 range"))
}

#[derive(Deserialize)]
struct OkxEnvelope<T> {
    code: String,
    msg: String,
    data: T,
}

fn validate_ticker_code(code: &str) -> Result<(), DataError> {
    if code.trim().is_empty() {
        Err(DataError::okx(
            "invalid_request",
            "ticker code must be non-empty",
        ))
    } else {
        Ok(())
    }
}

fn validate_ticker_codes(codes: &[String]) -> Result<(), DataError> {
    if codes.is_empty() {
        return Err(DataError::okx(
            "invalid_request",
            "at least one ticker code is required",
        ));
    }
    if codes.len() > OKX_MAX_STREAM_SYMBOLS {
        return Err(DataError::okx(
            "stream_limit",
            format!("OKX public streams support at most {OKX_MAX_STREAM_SYMBOLS} symbols"),
        ));
    }
    for code in codes {
        validate_ticker_code(code)?;
    }
    Ok(())
}

fn validate_bar_subscriptions(subscriptions: &[BarSubscription]) -> Result<(), DataError> {
    if subscriptions.is_empty() {
        return Err(DataError::okx(
            "invalid_request",
            "at least one bar subscription is required",
        ));
    }
    if subscriptions.len() > OKX_MAX_STREAM_SYMBOLS {
        return Err(DataError::okx(
            "stream_limit",
            format!("OKX public streams support at most {OKX_MAX_STREAM_SYMBOLS} subscriptions"),
        ));
    }
    for subscription in subscriptions {
        validate_ticker_code(&subscription.code)?;
    }
    Ok(())
}

fn parse_okx_bar(values: Vec<String>) -> Result<Option<OhlcvBar>, DataError> {
    let snapshot = parse_okx_bar_snapshot(values)?;
    Ok(snapshot.closed.then_some(snapshot.bar))
}

struct ParsedBarSnapshot {
    bar: OhlcvBar,
    closed: bool,
}

fn parse_okx_bar_snapshot(values: Vec<String>) -> Result<ParsedBarSnapshot, DataError> {
    if values.len() < 9 {
        return Err(DataError::okx(
            "invalid_response",
            "OKX bar has fewer than 9 fields",
        ));
    }
    let closed = match values[8].as_str() {
        "0" => false,
        "1" => true,
        value => {
            return Err(DataError::okx(
                "invalid_response",
                format!("invalid OKX bar confirmation value: {value}"),
            ));
        }
    };

    let parse_decimal = |index: usize, field: &str| {
        Decimal::from_str_exact(&values[index]).map_err(|error| {
            DataError::okx("invalid_decimal", format!("invalid OKX {field}: {error}"))
        })
    };

    Ok(ParsedBarSnapshot {
        bar: OhlcvBar {
            open_time_ms: values[0].parse().map_err(|error| {
                DataError::okx(
                    "invalid_response",
                    format!("invalid OKX timestamp: {error}"),
                )
            })?,
            open: parse_decimal(1, "open")?,
            high: parse_decimal(2, "high")?,
            low: parse_decimal(3, "low")?,
            close: parse_decimal(4, "close")?,
            base_volume: parse_decimal(5, "base volume")?,
            quote_volume: parse_decimal(7, "quote volume")?,
        },
        closed,
    })
}

// ===================== adaq-trading-crypto mapping layer =====================
//
// These helpers translate the ccxt-aligned types returned by
// `adaq_trading_crypto::adapters::Okx` into ADAQ's existing public data
// contracts. The crate uses the slash symbol form (`BTC/USDT`); ADAQ's
// external contract uses the dash form (`BTC-USDT`), so output types always
// carry the dash form.

fn dash_to_slash(code: &str) -> String {
    code.replace('-', "/")
}

#[allow(dead_code)]
fn slash_to_dash(code: &str) -> String {
    code.replace('/', "-")
}

fn map_crate_ticker(
    ticker: &adaq_trading_crypto::types::Ticker,
    code: &str,
) -> Result<TickerSnapshot, DataError> {
    let decimal = |value: Option<Decimal>, field: &str| -> Result<Decimal, DataError> {
        value.ok_or_else(|| {
            DataError::okx(
                "invalid_response",
                format!("adaq-trading-crypto ticker missing {field} for {code}"),
            )
        })
    };
    let zero = Decimal::ZERO;
    Ok(TickerSnapshot {
        src: OKX_SRC.to_owned(),
        code: code.to_owned(),
        last: decimal(ticker.last, "last")?,
        // ccxt ticker carries no last-trade size; preserve the field as zero.
        last_quantity: zero,
        ask_price: ticker.ask,
        ask_quantity: ticker.ask_volume,
        bid_price: ticker.bid,
        bid_quantity: ticker.bid_volume,
        open_24h: decimal(ticker.open, "open24h")?,
        high_24h: decimal(ticker.high, "high24h")?,
        low_24h: decimal(ticker.low, "low24h")?,
        base_volume_24h: decimal(ticker.base_volume, "base volume")?,
        quote_volume_24h: decimal(ticker.quote_volume, "quote volume")?,
        timestamp_ms: ticker.timestamp.unwrap_or_else(now_ms),
    })
}

fn map_crate_trade(
    trade: &adaq_trading_crypto::types::Trade,
    code: &str,
) -> Result<MarketTrade, DataError> {
    let side = match trade.side.as_deref() {
        Some("buy") => MarketTradeSide::Buy,
        Some("sell") => MarketTradeSide::Sell,
        _ => MarketTradeSide::Unknown,
    };
    Ok(MarketTrade {
        src: OKX_SRC.to_owned(),
        code: code.to_owned(),
        trade_id: trade.id.clone().unwrap_or_default(),
        price: trade.price.unwrap_or(Decimal::ZERO),
        quantity: trade.amount.unwrap_or(Decimal::ZERO),
        side,
        timestamp_ms: trade.timestamp.unwrap_or_else(now_ms),
    })
}

fn map_crate_order_book(
    book: &adaq_trading_crypto::types::OrderBook,
    code: &str,
) -> Result<Level2Snapshot, DataError> {
    let map_levels =
        |levels: &[adaq_trading_crypto::types::Level]| -> Result<Vec<OrderBookLevel>, DataError> {
            levels
                .iter()
                .map(|level| {
                    Ok(OrderBookLevel {
                        price: level.price.unwrap_or(Decimal::ZERO),
                        quantity: level.amount.unwrap_or(Decimal::ZERO),
                        order_count: None,
                    })
                })
                .collect()
        };
    Ok(Level2Snapshot {
        src: OKX_SRC.to_owned(),
        code: code.to_owned(),
        asks: map_levels(&book.asks)?,
        bids: map_levels(&book.bids)?,
        timestamp_ms: book.timestamp.unwrap_or_else(now_ms),
        checksum: book.nonce,
    })
}

fn map_crate_market(
    market: &adaq_trading_crypto::types::Market,
) -> Result<SpotInstrument, DataError> {
    let status = match market.active {
        Some(true) => InstrumentStatus::Live,
        Some(false) => InstrumentStatus::Suspended,
        None => InstrumentStatus::Unknown,
    };
    let decimal = |value: Option<Decimal>, field: &str| -> Result<Decimal, DataError> {
        value.ok_or_else(|| {
            DataError::okx(
                "invalid_response",
                format!(
                    "adaq-trading-crypto market missing {field} for {}",
                    market.id
                ),
            )
        })
    };
    Ok(SpotInstrument {
        src: OKX_SRC.to_owned(),
        code: market.id.clone(),
        base_asset: market.base.clone().unwrap_or_default(),
        quote_asset: market.quote.clone().unwrap_or_default(),
        status,
        listing_time_ms: None,
        continuous_trading_time_ms: None,
        price_increment: decimal(market.precision.price, "price precision")?,
        quantity_increment: decimal(market.precision.amount, "amount precision")?,
        minimum_quantity: decimal(market.precision.amount, "minimum quantity")?,
    })
}

// ===================== realtime bridge (adaq-trading-crypto) =====================
//
// Ticker/trade/order-book streams use the crate's `OkxWs` watch API. K-lines
// preserve ADAQ's original single-session, heartbeat, and reconnect lifecycle,
// using the crate's public websocket connector and OKX parser for the wire path.

fn map_crate_error(err: adaq_trading_crypto::Error) -> DataError {
    DataError::new(OKX_SRC, err.kind().as_str().to_owned(), err.to_string())
}

/// Event enums that carry a `Connected` variant. The stream drivers emit a
/// single connection-established event before the first snapshot.
trait StreamConnected {
    fn connected() -> Self;
}

impl StreamConnected for TickerStreamEvent {
    fn connected() -> Self {
        TickerStreamEvent::Connected
    }
}
impl StreamConnected for TradeStreamEvent {
    fn connected() -> Self {
        TradeStreamEvent::Connected
    }
}
impl StreamConnected for Level2StreamEvent {
    fn connected() -> Self {
        Level2StreamEvent::Connected
    }
}
impl StreamConnected for BarStreamEvent {
    fn connected() -> Self {
        BarStreamEvent::Connected
    }
}

fn parse_realtime_bar_message(
    engine: &adaq_trading_crypto::adapters::Okx,
    text: &str,
    subscriptions: &[BarSubscription],
) -> Result<Vec<BarSnapshot>, DataError> {
    let message = serde_json::from_str::<serde_json::Value>(text)
        .map_err(|error| DataError::okx("invalid_response", error.to_string()))?;
    let channel = message["arg"]["channel"].as_str().unwrap_or_default();
    let code = message["arg"]["instId"].as_str().unwrap_or_default();
    let Some(subscription) = subscriptions.iter().find(|subscription| {
        subscription.code == code && channel == format!("candle{}", subscription.interval.okx_bar())
    }) else {
        return Ok(Vec::new());
    };
    let Some(rows) = message["data"].as_array() else {
        return Ok(Vec::new());
    };
    rows.iter()
        .map(|row| {
            let ohlcv = engine.parse_ohlcv(row);
            let mut snapshot = map_crate_ohlcv(&ohlcv, code, subscription.interval)?;
            snapshot.closed = row
                .as_array()
                .and_then(|values| values.get(8))
                .and_then(serde_json::Value::as_str)
                .map(|value| value == "1")
                .ok_or_else(|| {
                    DataError::okx("invalid_response", "OKX bar missing confirm flag")
                })?;
            Ok(snapshot)
        })
        .collect()
}

/// Maps a crate `OHLCV` into ADAQ's `BarSnapshot`. The crate's unified `OHLCV`
/// carries no `closed` flag and only exposes base volume, so:
/// - `closed` is derived: a bar is closed once its next open time is in the past;
/// - quote volume is approximated as `close * base_volume` (OKX exposes it as a
///   separate field that the unified `OHLCV` drops — see crate-improvement notes).
fn map_crate_ohlcv(
    ohlcv: &adaq_trading_crypto::types::OHLCV,
    code: &str,
    interval: BarInterval,
) -> Result<BarSnapshot, DataError> {
    let open_time_ms = ohlcv
        .timestamp
        .ok_or_else(|| DataError::okx("invalid_response", "OKX candle missing timestamp"))?;
    let close = ohlcv.close.unwrap_or(Decimal::ZERO);
    let base_volume = ohlcv.volume.unwrap_or(Decimal::ZERO);
    let quote_volume = close * base_volume;
    Ok(BarSnapshot {
        src: OKX_SRC.to_owned(),
        code: code.to_owned(),
        interval,
        bar: OhlcvBar {
            open_time_ms,
            open: ohlcv.open.unwrap_or(Decimal::ZERO),
            high: ohlcv.high.unwrap_or(Decimal::ZERO),
            low: ohlcv.low.unwrap_or(Decimal::ZERO),
            close,
            base_volume,
            quote_volume,
        },
        closed: bar_is_closed(open_time_ms, interval),
    })
}

fn bar_is_closed(open_time_ms: i64, interval: BarInterval) -> bool {
    match next_bar_open_time_ms(open_time_ms, interval) {
        Ok(next_open_ms) => next_open_ms <= now_ms(),
        Err(_) => false,
    }
}

/// Boxed future returned by a `Realtime` watch closure.
type WatchTask<E> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<E>, DataError>> + Send>>;

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::mpsc,
        thread,
    };

    use rust_decimal::Decimal;

    use super::{
        BarInterval, BarStreamEvent, BarSubscription, HistoricalBarRange, Level2StreamEvent,
        OkxClient, OkxRequestPolicy, TickerStreamEvent, TradeStreamEvent,
    };

    #[test]
    fn all_intervals_map_to_okx_history_bars() {
        let actual = BarInterval::ALL.map(|interval| (interval.as_str(), interval.okx_bar()));

        assert_eq!(
            actual,
            [
                ("1s", "1s"),
                ("1m", "1m"),
                ("3m", "3m"),
                ("5m", "5m"),
                ("15m", "15m"),
                ("30m", "30m"),
                ("1h", "1H"),
                ("2h", "2H"),
                ("4h", "4H"),
                ("6h", "6Hutc"),
                ("12h", "12Hutc"),
                ("1d", "1Dutc"),
                ("2d", "2Dutc"),
                ("3d", "3Dutc"),
                ("5d", "5Dutc"),
                ("1w", "1Wutc"),
                ("1mo", "1Mutc"),
                ("3mo", "3Mutc"),
            ]
        );
    }

    #[test]
    fn ticker_stream_events_match_the_frontend_channel_schema() {
        assert_eq!(
            serde_json::to_value(TickerStreamEvent::Reconnecting { delay_ms: 1_000 }).unwrap(),
            serde_json::json!({
                "event": "reconnecting",
                "data": { "delayMs": 1_000 }
            })
        );
        assert_eq!(
            serde_json::to_value(TickerStreamEvent::Connected).unwrap(),
            serde_json::json!({ "event": "connected" })
        );
    }

    #[test]
    fn bar_stream_events_match_the_frontend_channel_schema() {
        assert_eq!(
            serde_json::to_value(BarStreamEvent::Reconnecting { delay_ms: 1_000 }).unwrap(),
            serde_json::json!({
                "event": "reconnecting",
                "data": { "delayMs": 1_000 }
            })
        );
        assert_eq!(
            serde_json::to_value(BarStreamEvent::Connected).unwrap(),
            serde_json::json!({ "event": "connected" })
        );
    }

    #[tokio::test]
    async fn okx_client_returns_normalized_spot_ticker() {
        let (base_url, request_line) = serve_json(
            r#"{
                "code": "0",
                "msg": "",
                "data": [{
                    "instType": "SPOT",
                    "instId": "BTC-USDT",
                    "last": "67432.10",
                    "lastSz": "0.002",
                    "askPx": "67432.20",
                    "askSz": "1.5",
                    "bidPx": "67432.10",
                    "bidSz": "0.8",
                    "open24h": "66100",
                    "high24h": "68000",
                    "low24h": "65500",
                    "volCcy24h": "123456789.12",
                    "vol24h": "1842.5",
                    "ts": "1720000000123"
                }]
            }"#,
        );

        let ticker = OkxClient::new(base_url)
            .get_ticker("BTC-USDT")
            .await
            .unwrap();

        assert_eq!(ticker.code, "BTC-USDT");
        assert_eq!(ticker.last.to_string(), "67432.10");
        assert_eq!(ticker.base_volume_24h.to_string(), "1842.5");
        assert_eq!(
            serde_json::to_value(&ticker).unwrap()["quoteVolume24h"],
            "123456789.12"
        );
        assert_eq!(
            request_line.recv().unwrap(),
            "GET /api/v5/market/ticker?instId=BTC-USDT HTTP/1.1"
        );
    }

    // Live integration test: streams real OKX ticker data through the
    // `adaq-trading-crypto` `OkxWs` adapter. Requires network; run with
    // `cargo test -- --ignored`. Ignored by default so the offline suite passes.
    #[tokio::test]
    #[ignore]
    async fn okx_client_subscribes_and_streams_normalized_spot_ticker() {
        let client = OkxClient::new("https://www.okx.com");
        let (tx, rx) = tokio::sync::oneshot::channel();
        let mut tx = Some(tx);
        let handle = tokio::spawn(async move {
            let _ = client.stream_ticker("BTC-USDT", |event| match event {
                TickerStreamEvent::Connected => true,
                TickerStreamEvent::Snapshot(snapshot) => {
                    if let Some(tx) = tx.take() {
                        let _ = tx.send(snapshot);
                    }
                    false
                }
                _ => true,
            });
        });
        let snapshot = tokio::time::timeout(std::time::Duration::from_secs(20), rx)
            .await
            .expect("timed out waiting for a live OKX ticker snapshot")
            .unwrap();
        handle.abort();
        assert_eq!(snapshot.code, "BTC-USDT");
        assert!(snapshot.last > Decimal::ZERO);
    }

    // Live integration test: see note on the ticker variant above.
    #[tokio::test]
    #[ignore]
    async fn okx_client_multiplexes_tickers_over_one_subscription() {
        let client = OkxClient::new("https://www.okx.com");
        let (tx, rx) = tokio::sync::oneshot::channel();
        let mut tx = Some(tx);
        let handle = tokio::spawn(async move {
            let _ =
                client.stream_tickers(&["BTC-USDT".to_owned(), "ETH-USDT".to_owned()], |event| {
                    match event {
                        TickerStreamEvent::Snapshot(snapshot) => {
                            if let Some(tx) = tx.take() {
                                let _ = tx.send(snapshot);
                            }
                            false
                        }
                        _ => true,
                    }
                });
        });
        let snapshot = tokio::time::timeout(std::time::Duration::from_secs(20), rx)
            .await
            .expect("timed out waiting for a live OKX ticker snapshot")
            .unwrap();
        handle.abort();
        assert!(["BTC-USDT", "ETH-USDT"].contains(&snapshot.code.as_str()));
        assert!(snapshot.last > Decimal::ZERO);
    }

    // Live integration test: see note on the ticker variant above.
    #[tokio::test]
    #[ignore]
    async fn okx_client_streams_normalized_market_trades_over_one_connection() {
        let client = OkxClient::new("https://www.okx.com");
        let (tx, rx) = tokio::sync::oneshot::channel();
        let mut tx = Some(tx);
        let handle = tokio::spawn(async move {
            let _ = client.stream_trades(&["BTC-USDT".to_owned()], |event| match event {
                TradeStreamEvent::Snapshot(trade) => {
                    if let Some(tx) = tx.take() {
                        let _ = tx.send(trade);
                    }
                    false
                }
                _ => true,
            });
        });
        let trade = tokio::time::timeout(std::time::Duration::from_secs(20), rx)
            .await
            .expect("timed out waiting for a live OKX trade")
            .unwrap();
        handle.abort();
        assert_eq!(trade.code, "BTC-USDT");
        assert!(trade.price > Decimal::ZERO);
    }

    // Live integration test: see note on the ticker variant above.
    #[tokio::test]
    #[ignore]
    async fn okx_client_streams_level_two_snapshots_without_float_values() {
        let client = OkxClient::new("https://www.okx.com");
        let (tx, rx) = tokio::sync::oneshot::channel();
        let mut tx = Some(tx);
        let handle = tokio::spawn(async move {
            let _ = client.stream_order_books(&["BTC-USDT".to_owned()], |event| match event {
                Level2StreamEvent::Snapshot(book) => {
                    if let Some(tx) = tx.take() {
                        let _ = tx.send(book);
                    }
                    false
                }
                _ => true,
            });
        });
        let book = tokio::time::timeout(std::time::Duration::from_secs(20), rx)
            .await
            .expect("timed out waiting for a live OKX order book")
            .unwrap();
        handle.abort();
        assert_eq!(book.code, "BTC-USDT");
        assert!(!book.asks.is_empty());
        assert!(!book.bids.is_empty());
        assert!(book.asks[0].price > Decimal::ZERO);
    }

    // Live integration test: see note on the ticker variant above.
    #[tokio::test]
    #[ignore]
    async fn okx_client_subscribes_and_streams_open_bars() {
        let client = OkxClient::new("https://www.okx.com");
        let (tx, rx) = tokio::sync::oneshot::channel();
        let mut tx = Some(tx);
        let handle = tokio::spawn(async move {
            let _ = client.stream_bar(
                "BTC-USDT",
                BarInterval::FifteenMinutes,
                |event| match event {
                    BarStreamEvent::Snapshot(snapshot) => {
                        if let Some(tx) = tx.take() {
                            let _ = tx.send(snapshot);
                        }
                        false
                    }
                    _ => true,
                },
            );
        });
        let snapshot = tokio::time::timeout(std::time::Duration::from_secs(20), rx)
            .await
            .expect("timed out waiting for a live OKX bar")
            .unwrap();
        handle.abort();
        assert_eq!(snapshot.code, "BTC-USDT");
        assert_eq!(snapshot.interval, BarInterval::FifteenMinutes);
        assert!(snapshot.bar.close > Decimal::ZERO);
    }

    // Live integration test: see note on the ticker variant above.
    #[tokio::test]
    #[ignore]
    async fn okx_client_multiplexes_bars_over_one_subscription() {
        let client = OkxClient::new("https://www.okx.com");
        let (tx, rx) = tokio::sync::oneshot::channel();
        let mut tx = Some(tx);
        let handle = tokio::spawn(async move {
            let _ = client.stream_bars(
                &[
                    BarSubscription {
                        code: "BTC-USDT".to_owned(),
                        interval: BarInterval::FifteenMinutes,
                    },
                    BarSubscription {
                        code: "ETH-USDT".to_owned(),
                        interval: BarInterval::OneHour,
                    },
                ],
                |event| match event {
                    BarStreamEvent::Snapshot(snapshot) => {
                        if let Some(tx) = tx.take() {
                            let _ = tx.send(snapshot);
                        }
                        false
                    }
                    _ => true,
                },
            );
        });
        let snapshot = tokio::time::timeout(std::time::Duration::from_secs(20), rx)
            .await
            .expect("timed out waiting for a live OKX bar")
            .unwrap();
        handle.abort();
        assert!(["BTC-USDT", "ETH-USDT"].contains(&snapshot.code.as_str()));
        assert!(snapshot.bar.close > Decimal::ZERO);
    }

    #[tokio::test]
    async fn okx_client_returns_closed_bars_in_ascending_order_with_decimal_strings() {
        let (base_url, request_line) = serve_json(
            r#"{
                "code": "0",
                "msg": "",
                "data": [
                    ["1704240000000", "43000.1", "43100.2", "42900.3", "43050.4", "1.25", "53813", "53813.00000001", "0"],
                    ["1704153600000", "42000.00000001", "42500.2", "41900.3", "42400.4", "2.50000000", "106001", "106001.00000001", "1"],
                    ["1704067200000", "41000.00000001", "42100.2", "40900.3", "42000.4", "3.75000000", "157501", "157501.00000001", "1"]
                ]
            }"#,
        );
        let client = OkxClient::new(base_url);

        let series = client
            .get_bar_series("BTC-USDT", BarInterval::OneDay, 100)
            .await
            .unwrap();

        assert_eq!(series.src, "okx");
        assert_eq!(series.code, "BTC-USDT");
        assert_eq!(series.bars.len(), 2);
        assert_eq!(series.bars[0].open_time_ms, 1_704_067_200_000);
        assert_eq!(series.bars[0].open.to_string(), "41000.00000001");
        assert_eq!(series.bars[1].base_volume.to_string(), "2.50000000");
        assert_eq!(
            serde_json::to_value(&series).unwrap()["bars"][0]["open"],
            "41000.00000001"
        );
        assert_eq!(
            request_line.recv().unwrap(),
            "GET /api/v5/market/history-candles?instId=BTC-USDT&bar=1Dutc&limit=100 HTTP/1.1"
        );
    }

    #[tokio::test]
    async fn okx_client_retries_rate_limits_and_reports_backoff_diagnostics() {
        let body = r#"{
            "code": "0",
            "msg": "",
            "data": [["1704067200000", "1", "2", "0.5", "1.5", "1", "1.5", "1.5", "1"]]
        }"#;
        let (base_url, _requests) = serve_status_pages(vec![
            (429, r#"{"code":"50011","msg":"rate limit"}"#.into()),
            (200, body.into()),
        ]);
        let acquisition = OkxClient::new_with_policy(
            base_url,
            OkxRequestPolicy {
                max_attempts: 2,
                min_delay_ms: 0,
                retry_delay_ms: 0,
                max_retry_delay_ms: 0,
            },
        )
        .get_bar_series_range_with_evidence(
            "BTC-USDT",
            BarInterval::OneDay,
            HistoricalBarRange {
                start_time_ms: 1_704_067_200_000,
                end_time_ms: 1_704_153_600_000,
            },
            |_, _| true,
        )
        .await
        .unwrap();

        assert_eq!(acquisition.diagnostics.request_count, 2);
        assert_eq!(acquisition.diagnostics.retry_count, 1);
        assert_eq!(acquisition.diagnostics.response_statuses, vec![429, 200]);
    }

    #[tokio::test]
    async fn okx_client_rejects_malformed_provider_bars() {
        let (base_url, _request_line) = serve_json(
            r#"{
                "code": "0",
                "msg": "",
                "data": [["1704067200000", "1", "2", "0.5", "1.5", "3", "4", "5"]]
            }"#,
        );

        let error = OkxClient::new(base_url)
            .get_bar_series("BTC-USDT", BarInterval::OneDay, 100)
            .await
            .unwrap_err();

        assert_eq!(error.src, "okx");
        assert_eq!(error.code, "invalid_response");
    }

    #[tokio::test]
    async fn okx_client_rejects_misaligned_provider_bar_timestamps() {
        let (base_url, _request_line) = serve_json(
            r#"{
                "code": "0",
                "msg": "",
                "data": [["1704067200123", "1", "2", "0.5", "1.5", "3", "4", "5", "1"]]
            }"#,
        );

        let error = OkxClient::new(base_url)
            .get_bar_series("BTC-USDT", BarInterval::OneMinute, 100)
            .await
            .unwrap_err();

        assert_eq!(error.code, "invalid_timestamp");
    }

    #[tokio::test]
    async fn okx_client_returns_normalized_spot_instruments() {
        let (base_url, request_line) = serve_json(
            r#"{
                "code": "0",
                "msg": "",
                "data": [{
                    "instId": "BTC-USDT",
                    "baseCcy": "BTC",
                    "quoteCcy": "USDT",
                    "state": "live",
                    "listTime": "1539828749000",
                    "contTdSwTime": "",
                    "tickSz": "0.1",
                    "lotSz": "0.00000001",
                    "minSz": "0.00001"
                }]
            }"#,
        );

        let instruments = OkxClient::new(base_url)
            .list_spot_instruments()
            .await
            .unwrap();

        assert_eq!(instruments.len(), 1);
        assert_eq!(instruments[0].src, "okx");
        assert_eq!(instruments[0].code, "BTC-USDT");
        assert_eq!(instruments[0].price_increment.to_string(), "0.1");
        assert!(!instruments[0].code.is_empty());
        assert_eq!(
            serde_json::to_value(&instruments[0]).unwrap()["quantityIncrement"],
            "0.00000001"
        );
        assert_eq!(
            request_line.recv().unwrap(),
            "GET /api/v5/public/instruments?instType=SPOT HTTP/1.1"
        );
    }

    #[tokio::test]
    async fn okx_client_rejects_conflicting_bars_with_the_same_identity() {
        let (base_url, _request_line) = serve_json(
            r#"{
                "code": "0",
                "msg": "",
                "data": [
                    ["1704067200000", "41000", "42100", "40900", "42000", "3.75", "157500", "157500", "1"],
                    ["1704067200000", "41000", "42100", "40900", "41999", "3.75", "157500", "157500", "1"]
                ]
            }"#,
        );

        let error = OkxClient::new(base_url)
            .get_bar_series("BTC-USDT", BarInterval::OneDay, 100)
            .await
            .unwrap_err();

        assert_eq!(error.code, "conflicting_bar");
    }

    #[tokio::test]
    async fn okx_client_returns_only_closed_bars_inside_the_requested_range() {
        let (base_url, request_line) = serve_json(
            r#"{
                "code": "0",
                "msg": "",
                "data": [
                    ["1704326400000", "4", "5", "3", "4.5", "1", "4.5", "4.5", "1"],
                    ["1704240000000", "3", "4", "2", "3.5", "1", "3.5", "3.5", "0"],
                    ["1704153600000", "2", "3", "1", "2.5", "1", "2.5", "2.5", "1"],
                    ["1704067200000", "1", "2", "0.5", "1.5", "1", "1.5", "1.5", "1"],
                    ["1703980800000", "0.5", "1", "0.1", "0.8", "1", "0.8", "0.8", "1"]
                ]
            }"#,
        );
        let range = HistoricalBarRange {
            start_time_ms: 1_704_067_200_000,
            end_time_ms: 1_704_326_400_000,
        };

        let series = OkxClient::new(base_url)
            .get_bar_series_range("BTC-USDT", BarInterval::OneDay, range)
            .await
            .unwrap();

        assert_eq!(
            series
                .bars
                .iter()
                .map(|bar| bar.open_time_ms)
                .collect::<Vec<_>>(),
            vec![1_704_067_200_000, 1_704_153_600_000]
        );
        assert_eq!(
            request_line.recv().unwrap(),
            "GET /api/v5/market/history-candles?instId=BTC-USDT&bar=1Dutc&after=1704326400000&limit=100 HTTP/1.1"
        );
    }

    #[tokio::test]
    async fn okx_client_paginates_until_the_requested_range_is_complete() {
        const BASE: i64 = 1_699_920_000_000;
        const DAY: i64 = 86_400_000;
        let row = |index: i64| {
            serde_json::json!([
                (BASE + index * DAY).to_string(),
                "1",
                "2",
                "0.5",
                "1.5",
                "1",
                "1.5",
                "1.5",
                "1"
            ])
        };
        let first_page = serde_json::json!({
            "code": "0",
            "msg": "",
            "data": (101..=200).rev().map(row).collect::<Vec<_>>()
        })
        .to_string();
        let second_page = serde_json::json!({
            "code": "0",
            "msg": "",
            "data": [row(100)]
        })
        .to_string();
        let (base_url, _request_lines) = serve_json_pages(vec![first_page, second_page]);

        let series = OkxClient::new(base_url)
            .get_bar_series_range(
                "BTC-USDT",
                BarInterval::OneDay,
                HistoricalBarRange {
                    start_time_ms: BASE + 100 * DAY,
                    end_time_ms: BASE + 201 * DAY,
                },
            )
            .await
            .unwrap();

        assert_eq!(series.bars.len(), 101);
        assert_eq!(series.bars[0].open_time_ms, BASE + 100 * DAY);
        assert_eq!(series.bars[100].open_time_ms, BASE + 200 * DAY);
    }

    #[tokio::test]
    async fn okx_client_reports_contiguous_bar_gaps() {
        let (base_url, _request_line) = serve_json(
            r#"{
                "code": "0",
                "msg": "",
                "data": [
                    ["1704240000000", "3", "4", "2", "3.5", "1", "3.5", "3.5", "1"],
                    ["1704067200000", "1", "2", "0.5", "1.5", "1", "1.5", "1.5", "1"]
                ]
            }"#,
        );

        let series = OkxClient::new(base_url)
            .get_bar_series_range(
                "BTC-USDT",
                BarInterval::OneDay,
                HistoricalBarRange {
                    start_time_ms: 1_704_067_200_000,
                    end_time_ms: 1_704_326_400_000,
                },
            )
            .await
            .unwrap();

        assert_eq!(series.gaps.len(), 1);
        assert_eq!(series.gaps[0].start_time_ms, 1_704_153_600_000);
        assert_eq!(series.gaps[0].end_time_ms, 1_704_240_000_000);
    }

    #[tokio::test]
    async fn okx_range_download_can_be_cancelled_after_progress() {
        let (base_url, _request_line) = serve_json(
            r#"{"code":"0","msg":"","data":[["1704067200000","1","2","0.5","1.5","1","1.5","1.5","1"]]}"#,
        );
        let error = OkxClient::new(base_url)
            .get_bar_series_range_with_progress(
                "BTC-USDT",
                BarInterval::OneDay,
                HistoricalBarRange {
                    start_time_ms: 1_704_067_200_000,
                    end_time_ms: 1_704_153_600_000,
                },
                |downloaded, _| {
                    assert_eq!(downloaded, 1);
                    false
                },
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, "cancelled");
    }

    fn serve_json(body: &str) -> (String, mpsc::Receiver<String>) {
        serve_json_pages(vec![body.to_owned()])
    }

    fn serve_json_pages(bodies: Vec<String>) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, receiver) = mpsc::channel();

        thread::spawn(move || {
            for body in bodies {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0; 4096];
                let size = stream.read(&mut request).unwrap();
                let request = String::from_utf8_lossy(&request[..size]);
                sender
                    .send(request.lines().next().unwrap().to_owned())
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

    fn serve_status_pages(responses: Vec<(u16, String)>) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, receiver) = mpsc::channel();

        thread::spawn(move || {
            for (status, body) in responses {
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
                    "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .unwrap();
            }
        });

        (format!("http://{address}"), receiver)
    }

    // ===================== adaq-trading-crypto mapping tests =====================
    //
    // These exercise ADAQ's translation layer against the crate's public
    // `parse_*` functions using fixture JSON, so the mapping is verified
    // without any network access.

    #[test]
    fn symbol_translation_round_trips() {
        assert_eq!(super::dash_to_slash("BTC-USDT"), "BTC/USDT");
        assert_eq!(super::slash_to_dash("BTC/USDT"), "BTC-USDT");
        assert_eq!(super::dash_to_slash("ETH-BTC"), "ETH/BTC");
    }

    #[test]
    fn ticker_mapping_preserves_dash_code_and_fields() {
        let raw = serde_json::json!({
            "instId": "BTC-USDT",
            "last": "67000.5",
            "bidPx": "66999.0",
            "askPx": "67001.0",
            "open24h": "66000.0",
            "high24h": "68000.0",
            "low24h": "65500.0",
            "vol24h": "1234.5",
            "volCcy24h": "83000000.0",
            "ts": "1719999900000"
        });
        let ticker = super::engine().parse_ticker(&raw);
        let snapshot = super::map_crate_ticker(&ticker, "BTC-USDT").unwrap();

        assert_eq!(snapshot.code, "BTC-USDT");
        assert_eq!(snapshot.src, "okx");
        assert_eq!(snapshot.last, Decimal::from_str_exact("67000.5").unwrap());
        assert_eq!(
            snapshot.bid_price,
            Some(Decimal::from_str_exact("66999.0").unwrap())
        );
        assert_eq!(
            snapshot.ask_price,
            Some(Decimal::from_str_exact("67001.0").unwrap())
        );
        // The crate's `parse_ticker` does not populate per-side sizes, so these
        // map to None (recommendation: add `bidSz`/`askSz` -> bid/ask volume).
        assert_eq!(snapshot.bid_quantity, None);
        assert_eq!(snapshot.ask_quantity, None);
        assert_eq!(
            snapshot.high_24h,
            Decimal::from_str_exact("68000.0").unwrap()
        );
        assert_eq!(
            snapshot.low_24h,
            Decimal::from_str_exact("65500.0").unwrap()
        );
        assert_eq!(
            snapshot.base_volume_24h,
            Decimal::from_str_exact("1234.5").unwrap()
        );
        assert_eq!(
            snapshot.quote_volume_24h,
            Decimal::from_str_exact("83000000.0").unwrap()
        );
        assert_eq!(snapshot.timestamp_ms, 1_719_999_900_000);
    }

    #[test]
    fn trade_mapping_uses_dash_code_and_side() {
        let raw = serde_json::json!({
            "instId": "BTC-USDT",
            "tradeId": "12345",
            "px": "67000.5",
            "sz": "0.25",
            "side": "buy",
            "ts": "1719999900000"
        });
        let trade = super::engine().parse_trade(&raw);
        let mapped = super::map_crate_trade(&trade, "BTC-USDT").unwrap();

        assert_eq!(mapped.code, "BTC-USDT");
        assert_eq!(mapped.trade_id, "12345");
        assert_eq!(mapped.price, Decimal::from_str_exact("67000.5").unwrap());
        assert_eq!(mapped.quantity, Decimal::from_str_exact("0.25").unwrap());
        assert_eq!(mapped.side, super::MarketTradeSide::Buy);
        assert_eq!(mapped.timestamp_ms, 1_719_999_900_000);
    }

    #[test]
    fn order_book_mapping_uses_dash_code_and_levels() {
        let raw = serde_json::json!({
            "asks": [["67001.0", "1.2"], ["67002.0", "3.4"]],
            "bids": [["66999.0", "0.5"]],
            "ts": "1719999900000"
        });
        let book = super::engine().parse_order_book(&raw, "BTC/USDT");
        let mapped = super::map_crate_order_book(&book, "BTC-USDT").unwrap();

        assert_eq!(mapped.code, "BTC-USDT");
        assert_eq!(mapped.asks.len(), 2);
        assert_eq!(mapped.bids.len(), 1);
        assert_eq!(
            mapped.asks[0].price,
            Decimal::from_str_exact("67001.0").unwrap()
        );
        assert_eq!(
            mapped.asks[0].quantity,
            Decimal::from_str_exact("1.2").unwrap()
        );
        assert_eq!(
            mapped.bids[0].price,
            Decimal::from_str_exact("66999.0").unwrap()
        );
        assert_eq!(mapped.timestamp_ms, 1_719_999_900_000);
    }

    #[test]
    fn bar_mapping_derives_closed_flag_and_quote_volume() {
        // The crate's unified `OHLCV` carries no `closed` flag and only exposes
        // base volume, so `map_crate_ohlcv` derives both from the open time and
        // approximates quote volume as `close * base_volume`.
        let ohlcv =
            serde_json::from_value::<adaq_trading_crypto::types::OHLCV>(serde_json::json!({
                "timestamp": 1_719_999_900_000i64,
                "open": "67000.10",
                "high": "67500.20",
                "low": "66900.30",
                "close": "67433.25",
                "volume": "1.25"
            }))
            .unwrap();
        let bar =
            super::map_crate_ohlcv(&ohlcv, "BTC-USDT", super::BarInterval::FifteenMinutes).unwrap();

        assert_eq!(bar.code, "BTC-USDT");
        assert_eq!(bar.interval, super::BarInterval::FifteenMinutes);
        assert_eq!(bar.bar.open_time_ms, 1_719_999_900_000);
        assert_eq!(bar.bar.close, Decimal::from_str_exact("67433.25").unwrap());
        assert_eq!(
            bar.bar.base_volume,
            Decimal::from_str_exact("1.25").unwrap()
        );
        // quote_volume = close * base_volume = 67433.25 * 1.25 = 84291.5625
        assert_eq!(
            bar.bar.quote_volume,
            Decimal::from_str_exact("84291.5625").unwrap()
        );
        // A 15m bar whose open time is in the past is considered closed.
        assert!(bar.closed);
    }

    #[test]
    fn realtime_bar_parser_preserves_confirmed_flag_and_crate_values() {
        let subscriptions = [super::BarSubscription {
            code: "BTC-USDT".into(),
            interval: super::BarInterval::FifteenMinutes,
        }];
        let text = serde_json::json!({
            "arg": {"channel": "candle15m", "instId": "BTC-USDT"},
            "data": [[
                "1719999900000", "67000.10", "67500.20", "66900.30",
                "67433.25", "1.25", "84291.5625", "84291.5625", "1"
            ]]
        })
        .to_string();
        let snapshots =
            super::parse_realtime_bar_message(super::engine(), &text, &subscriptions).unwrap();

        assert_eq!(snapshots.len(), 1);
        assert!(snapshots[0].closed);
        assert_eq!(snapshots[0].code, "BTC-USDT");
        assert_eq!(
            snapshots[0].bar.close,
            Decimal::from_str_exact("67433.25").unwrap()
        );
    }

    #[test]
    fn market_mapping_produces_spot_instrument() {
        let market =
            serde_json::from_value::<adaq_trading_crypto::types::Market>(serde_json::json!({
                "id": "BTC-USDT",
                "base": "BTC",
                "quote": "USDT",
                "active": true,
                "spot": true,
                "precision": { "amount": "0.0001", "price": "0.1" }
            }))
            .unwrap();
        let instrument = super::map_crate_market(&market).unwrap();

        assert_eq!(instrument.code, "BTC-USDT");
        assert_eq!(instrument.base_asset, "BTC");
        assert_eq!(instrument.quote_asset, "USDT");
        assert_eq!(instrument.status, super::InstrumentStatus::Live);
        assert_eq!(
            instrument.price_increment,
            Decimal::from_str_exact("0.1").unwrap()
        );
        assert_eq!(
            instrument.quantity_increment,
            Decimal::from_str_exact("0.0001").unwrap()
        );
    }

    #[test]
    fn okx_client_default_constructs_without_network() {
        // `Okx::new(Config::new())` performs no I/O; this proves the unified
        // client is wired into the façade constructor.
        let _client = super::OkxClient::default();
    }
}
