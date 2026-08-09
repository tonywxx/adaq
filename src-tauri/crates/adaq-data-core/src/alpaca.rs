//! Tauri-independent Alpaca Basic U.S. equity market-data acquisition.
//!
//! Credentials are accepted only as an in-memory value supplied by the Host.
//! This module never serializes, logs, or includes them in acquisition
//! provenance. All provider payloads are retained as raw JSON evidence while
//! numeric fields cross the connector boundary as exact decimal strings.

use std::{
    collections::BTreeSet,
    str::FromStr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use chrono::{DateTime, Datelike, Duration as ChronoDuration, NaiveDate, NaiveTime, Utc};
use futures_util::{SinkExt, StreamExt};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::{
    BarInterval, BarSnapshot, DataError, HistoricalBarRange, InstrumentStatus, MarketTrade,
    MarketTradeSide, TickerSnapshot,
    market::{
        DayEvidence, DayKind, InstrumentId, InstrumentSourceMapping, LocalTimeDisambiguation,
        ScheduledClosure, ScheduledClosureKind, SessionPhase, TradingCalendarSnapshot, TradingDate,
        TradingSession, Venue,
    },
};

pub const ALPACA_SRC: &str = "alpaca";
pub const ALPACA_CONNECTOR_VERSION: &str = "adaq-data-core-alpaca-v1";
pub const ALPACA_MARKET_DATA_ENDPOINT: &str = "https://data.alpaca.markets";
pub const ALPACA_STREAM_ENDPOINT: &str = "wss://stream.data.alpaca.markets/v2/iex";
pub const ALPACA_BASIC_HISTORY_START_YEAR: i32 = 2016;
pub const ALPACA_BASIC_HISTORICAL_CALLS_PER_MINUTE: u32 = 200;
pub const ALPACA_BASIC_STREAM_CONNECTION_LIMIT: usize = 1;
pub const ALPACA_BASIC_STREAM_SYMBOL_LIMIT: usize = 30;

const ALPACA_HISTORY_LATEST_DELAY_MS: i64 = 15 * 60 * 1_000;
const ALPACA_MAX_RETRY_SECONDS: u64 = 15;
const ALPACA_HEARTBEAT_SECONDS: u64 = 25;
const ALPACA_MAX_PAGE_SIZE: u32 = 10_000;
const ALPACA_MAX_PAGES: u32 = 1_000;

/// The only value that crosses the Host-to-connector credential boundary.
/// Deliberately has no `Debug`, `Serialize`, or public field accessors.
#[derive(Clone)]
pub struct AlpacaCredentials {
    key_id: String,
    secret_key: String,
}

impl AlpacaCredentials {
    pub fn new(key_id: impl Into<String>, secret_key: impl Into<String>) -> Self {
        Self {
            key_id: key_id.into(),
            secret_key: secret_key.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlpacaRequestPolicy {
    pub max_attempts: u8,
    pub min_delay_ms: u64,
    pub retry_delay_ms: u64,
    pub max_retry_delay_ms: u64,
}

impl Default for AlpacaRequestPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            // Basic permits 200 historical calls/minute. This gate is shared
            // by cloned clients so a single Host profile cannot burst past it.
            min_delay_ms: 300,
            retry_delay_ms: 250,
            max_retry_delay_ms: 5_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AlpacaRequestDiagnostics {
    pub request_count: u32,
    pub retry_count: u32,
    pub page_count: u32,
    #[serde(default)]
    pub response_statuses: Vec<u16>,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlpacaInstrument {
    pub instrument: InstrumentId,
    pub provider_symbol: String,
    pub name: Option<String>,
    pub status: InstrumentStatus,
    pub asset_class: String,
    pub exchange: String,
    pub tradable: bool,
    pub marginable: bool,
    pub shortable: bool,
    pub easy_to_borrow: bool,
    pub fractionable: bool,
    pub listing_time_ms: Option<i64>,
    pub continuous_trading_time_ms: Option<i64>,
    pub price_increment: Option<String>,
    pub quantity_increment: Option<String>,
    pub minimum_quantity: Option<String>,
    pub mapping: InstrumentSourceMapping,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlpacaInstrumentMasterAcquisition {
    pub provider: String,
    pub actual_upstream: String,
    pub method: String,
    pub connector_version: String,
    pub request_parameters: Value,
    pub retrieved_at_ms: i64,
    pub response_sha256: String,
    pub content_sha256: String,
    #[serde(default)]
    pub raw_response: Vec<u8>,
    pub diagnostics: AlpacaRequestDiagnostics,
    pub instruments: Vec<AlpacaInstrument>,
    #[serde(default)]
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlpacaBar {
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
    pub raw_payload: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlpacaBarsAcquisition {
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
    pub diagnostics: AlpacaRequestDiagnostics,
    pub bars: Vec<AlpacaBar>,
    #[serde(default)]
    pub invalid_bars: Vec<AlpacaBar>,
    #[serde(default)]
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlpacaQuote {
    pub src: String,
    pub code: String,
    #[serde(with = "rust_decimal::serde::str_option")]
    pub ask_price: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::str_option")]
    pub ask_quantity: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::str_option")]
    pub bid_price: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::str_option")]
    pub bid_quantity: Option<Decimal>,
    pub timestamp_ms: i64,
    pub ask_exchange: Option<String>,
    pub bid_exchange: Option<String>,
    pub feed: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlpacaMarketSnapshot {
    pub ticker: TickerSnapshot,
    pub trade: MarketTrade,
    pub quote: AlpacaQuote,
    pub feed: String,
    pub retrieved_at_ms: i64,
    pub response_sha256: String,
    pub raw_payload: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlpacaCalendarAcquisition {
    pub provider: String,
    pub actual_upstream: String,
    pub method: String,
    pub connector_version: String,
    pub request_parameters: Value,
    pub retrieved_at_ms: i64,
    pub response_sha256: String,
    pub content_sha256: String,
    #[serde(default)]
    pub raw_response: Vec<u8>,
    pub diagnostics: AlpacaRequestDiagnostics,
    pub snapshot: TradingCalendarSnapshot,
    #[serde(default)]
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlpacaStreamSubscription {
    pub symbols: Vec<String>,
    #[serde(default)]
    pub trades: bool,
    #[serde(default)]
    pub quotes: bool,
    #[serde(default)]
    pub bars: bool,
}

impl AlpacaStreamSubscription {
    pub fn trades(symbols: Vec<String>) -> Self {
        Self {
            symbols,
            trades: true,
            quotes: false,
            bars: false,
        }
    }

    pub fn quotes(symbols: Vec<String>) -> Self {
        Self {
            symbols,
            trades: false,
            quotes: true,
            bars: false,
        }
    }

    pub fn all(symbols: Vec<String>) -> Self {
        Self {
            symbols,
            trades: true,
            quotes: true,
            bars: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "event", content = "data")]
pub enum AlpacaStreamEvent {
    Connected,
    Authenticated,
    Subscribed,
    Trade(MarketTrade),
    Quote(AlpacaQuote),
    Bar(BarSnapshot),
    Error(DataError),
    Reconnecting { delay_ms: u64 },
    Closed,
}

#[derive(Clone)]
pub struct AlpacaClient {
    http: reqwest::Client,
    credentials: AlpacaCredentials,
    base_url: String,
    stream_url: String,
    policy: AlpacaRequestPolicy,
    next_request_at: Arc<Mutex<Instant>>,
}

impl AlpacaClient {
    pub fn new(credentials: AlpacaCredentials) -> Self {
        Self::with_urls_and_policy(
            credentials,
            ALPACA_MARKET_DATA_ENDPOINT,
            ALPACA_STREAM_ENDPOINT,
            AlpacaRequestPolicy::default(),
        )
    }

    pub fn with_key_pair(key_id: impl Into<String>, secret_key: impl Into<String>) -> Self {
        Self::new(AlpacaCredentials::new(key_id, secret_key))
    }

    /// Builds the production client with a Host-owned rate gate shared by
    /// operations that resolve the same local connection profile.
    pub fn with_rate_gate(
        credentials: AlpacaCredentials,
        next_request_at: Arc<Mutex<Instant>>,
    ) -> Self {
        Self {
            http: reqwest::Client::new(),
            credentials,
            base_url: ALPACA_MARKET_DATA_ENDPOINT.into(),
            stream_url: ALPACA_STREAM_ENDPOINT.into(),
            policy: AlpacaRequestPolicy::default(),
            next_request_at,
        }
    }

    pub fn with_urls_and_policy(
        credentials: AlpacaCredentials,
        base_url: impl Into<String>,
        stream_url: impl Into<String>,
        policy: AlpacaRequestPolicy,
    ) -> Self {
        Self {
            http: reqwest::Client::new(),
            credentials,
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            stream_url: stream_url.into(),
            policy,
            next_request_at: Arc::new(Mutex::new(Instant::now())),
        }
    }

    pub fn connector_version(&self) -> &'static str {
        ALPACA_CONNECTOR_VERSION
    }

    pub fn capability_snapshot(&self, captured_at_ms: i64) -> AlpacaCapabilitySnapshot {
        AlpacaCapabilitySnapshot::basic(captured_at_ms)
    }

    pub async fn acquire_instrument_master(
        &self,
        retrieved_at_ms: i64,
    ) -> Result<AlpacaInstrumentMasterAcquisition, DataError> {
        if retrieved_at_ms < 0 {
            return Err(error(
                "invalid_request",
                "retrieval time must be non-negative",
            ));
        }
        let response = self
            .get_bytes(
                "/v2/assets",
                &[
                    ("status".into(), "active".into()),
                    ("asset_class".into(), "us_equity".into()),
                ],
            )
            .await?;
        let values: Vec<RawAsset> = serde_json::from_slice(&response.bytes).map_err(|value| {
            with_raw(
                error("invalid_response", value.to_string()),
                &response.bytes,
            )
        })?;
        let mut instruments = values
            .into_iter()
            .map(|asset| self.normalize_asset(asset, retrieved_at_ms))
            .collect::<Result<Vec<_>, _>>()?;
        instruments.sort_by(|left, right| {
            left.instrument
                .venue
                .id
                .cmp(&right.instrument.venue.id)
                .then_with(|| left.instrument.code.cmp(&right.instrument.code))
        });
        let parsed_bytes = canonical_json_bytes(&instruments)?;
        Ok(AlpacaInstrumentMasterAcquisition {
            provider: ALPACA_SRC.into(),
            actual_upstream: "Alpaca Market Data API".into(),
            method: "GET /v2/assets".into(),
            connector_version: ALPACA_CONNECTOR_VERSION.into(),
            request_parameters: json!({"status":"active","assetClass":"us_equity"}),
            retrieved_at_ms,
            response_sha256: sha256(&response.bytes),
            content_sha256: sha256(&parsed_bytes),
            raw_response: response.bytes,
            diagnostics: response.diagnostics,
            instruments,
            limitations: vec![
                "The Basic plan exposes the available active U.S. stock and ETF universe; historical membership and delisted assets are not included by this endpoint".into(),
            ],
        })
    }

    pub async fn acquire_bars(
        &self,
        instrument: InstrumentId,
        interval: BarInterval,
        range: HistoricalBarRange,
        retrieved_at_ms: i64,
        is_cancelled: impl Fn() -> bool,
    ) -> Result<AlpacaBarsAcquisition, DataError> {
        if instrument.venue.kind != crate::market::VenueKind::UsEquity
            || range.start_time_ms >= range.end_time_ms
            || retrieved_at_ms < 0
        {
            return Err(error(
                "invalid_request",
                "U.S. equity bar request is invalid",
            ));
        }
        let timeframe = alpaca_timeframe(interval)?;
        let symbol = instrument.code.clone();
        let mut cursor: Option<String> = None;
        let mut bars = Vec::new();
        let mut invalid_bars = Vec::new();
        let mut raw_responses = Vec::new();
        let mut response_sha256s = Vec::new();
        let mut diagnostics = AlpacaRequestDiagnostics::default();
        let mut limitations = vec![
            "Basic historical equity access is limited to data no newer than the latest 15 minutes".into(),
            "Equity feed is IEX-only; volume and quote-volume do not represent consolidated U.S. market volume".into(),
        ];

        loop {
            if is_cancelled() {
                return Err(error("cancelled", "Alpaca acquisition was cancelled"));
            }
            if diagnostics.page_count >= ALPACA_MAX_PAGES {
                return Err(error(
                    "pagination_limit",
                    "Alpaca pagination exceeded the bounded page limit",
                ));
            }
            let mut query = vec![
                ("timeframe".into(), timeframe.into()),
                ("start".into(), format_rfc3339(range.start_time_ms)?),
                ("end".into(), format_rfc3339(range.end_time_ms)?),
                ("limit".into(), ALPACA_MAX_PAGE_SIZE.to_string()),
                ("sort".into(), "asc".into()),
                ("feed".into(), "iex".into()),
            ];
            if let Some(page_token) = cursor.as_ref() {
                query.push(("page_token".into(), page_token.clone()));
            }
            let response = self
                .get_bytes(&format!("/v2/stocks/{symbol}/bars"), &query)
                .await?;
            let payload: Value = serde_json::from_slice(&response.bytes).map_err(|value| {
                with_raw(
                    error("invalid_response", value.to_string()),
                    &response.bytes,
                )
            })?;
            let page = payload
                .get("bars")
                .and_then(|value| value.get(&symbol))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for raw in page {
                match self.parse_bar(&instrument, interval, &raw) {
                    Ok(bar) => bars.push(bar),
                    Err(reason) => {
                        diagnostics
                            .notes
                            .push(format!("bar retained for quarantine: {reason}"));
                        invalid_bars.push(self.invalid_bar(&instrument, interval, &raw));
                    }
                }
            }
            diagnostics.request_count += response.diagnostics.request_count;
            diagnostics.retry_count += response.diagnostics.retry_count;
            diagnostics.page_count += 1;
            diagnostics
                .response_statuses
                .extend(response.diagnostics.response_statuses);
            diagnostics.notes.extend(response.diagnostics.notes);
            response_sha256s.push(sha256(&response.bytes));
            raw_responses.push(response.bytes);
            let next = payload
                .get("next_page_token")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_owned);
            if next.is_none() {
                break;
            }
            if next == cursor {
                return Err(error(
                    "invalid_response",
                    "Alpaca pagination did not advance",
                ));
            }
            cursor = next;
        }

        bars.sort_by_key(|bar| bar.open_time_ms);
        invalid_bars.sort_by_key(|bar| bar.open_time_ms);
        let content_bytes = canonical_json_bytes(&(&bars, &invalid_bars))?;
        if bars.is_empty() {
            limitations.push("The provider returned no bars for the requested range".into());
        }
        Ok(AlpacaBarsAcquisition {
            provider: ALPACA_SRC.into(),
            actual_upstream: "Alpaca Market Data API".into(),
            method: "GET /v2/stocks/{symbol}/bars".into(),
            connector_version: ALPACA_CONNECTOR_VERSION.into(),
            request_parameters: json!({
                "symbol": symbol,
                "timeframe": timeframe,
                "startTimeMs": range.start_time_ms,
                "endTimeMs": range.end_time_ms,
                "feed": "iex",
                "limit": ALPACA_MAX_PAGE_SIZE,
            }),
            retrieved_at_ms,
            response_sha256s,
            content_sha256: sha256(&content_bytes),
            raw_responses,
            diagnostics,
            bars,
            invalid_bars,
            limitations,
        })
    }

    pub async fn get_snapshot(
        &self,
        symbol: &str,
        retrieved_at_ms: i64,
    ) -> Result<AlpacaMarketSnapshot, DataError> {
        validate_symbol(symbol)?;
        if retrieved_at_ms < 0 {
            return Err(error(
                "invalid_request",
                "retrieval time must be non-negative",
            ));
        }
        let response = self
            .get_bytes(
                &format!("/v2/stocks/{symbol}/snapshot"),
                &[("feed".into(), "iex".into())],
            )
            .await?;
        let payload: Value = serde_json::from_slice(&response.bytes).map_err(|value| {
            with_raw(
                error("invalid_response", value.to_string()),
                &response.bytes,
            )
        })?;
        let trade = parse_latest_trade(symbol, payload.get("latestTrade"))
            .map_err(|error| with_raw(error, &response.bytes))?;
        let quote = parse_latest_quote(symbol, payload.get("latestQuote"), "iex")
            .map_err(|error| with_raw(error, &response.bytes))?;
        let daily = payload
            .get("dailyBar")
            .ok_or_else(|| error("invalid_response", "Alpaca snapshot has no daily bar"))
            .map_err(|error| with_raw(error, &response.bytes))?;
        let open_24h = decimal_value(daily.get("o"), "daily open")
            .map_err(|error| with_raw(error, &response.bytes))?;
        let high_24h = decimal_value(daily.get("h"), "daily high")
            .map_err(|error| with_raw(error, &response.bytes))?;
        let low_24h = decimal_value(daily.get("l"), "daily low")
            .map_err(|error| with_raw(error, &response.bytes))?;
        let base_volume_24h = decimal_value(daily.get("v"), "daily volume")
            .map_err(|error| with_raw(error, &response.bytes))?;
        let quote_volume_24h = decimal_value(daily.get("vw"), "daily vwap")
            .map_err(|error| with_raw(error, &response.bytes))?
            * base_volume_24h;
        let ticker = TickerSnapshot {
            src: ALPACA_SRC.into(),
            code: symbol.into(),
            last: trade.price,
            last_quantity: trade.quantity,
            ask_price: quote.ask_price,
            ask_quantity: quote.ask_quantity,
            bid_price: quote.bid_price,
            bid_quantity: quote.bid_quantity,
            open_24h,
            high_24h,
            low_24h,
            base_volume_24h,
            quote_volume_24h,
            timestamp_ms: trade.timestamp_ms.max(quote.timestamp_ms),
        };
        Ok(AlpacaMarketSnapshot {
            ticker,
            trade,
            quote,
            feed: "iex".into(),
            retrieved_at_ms,
            response_sha256: sha256(&response.bytes),
            raw_payload: payload,
        })
    }

    pub async fn acquire_calendar(
        &self,
        venue: Venue,
        range: HistoricalBarRange,
        retrieved_at_ms: i64,
    ) -> Result<AlpacaCalendarAcquisition, DataError> {
        if venue.kind != crate::market::VenueKind::UsEquity
            || range.start_time_ms >= range.end_time_ms
            || retrieved_at_ms < 0
        {
            return Err(error(
                "invalid_request",
                "U.S. equity calendar request is invalid",
            ));
        }
        let snapshot = build_us_calendar(&venue, range)?;
        let raw_response = canonical_json_bytes(&snapshot)?;
        let response_sha256 = sha256(&raw_response);
        Ok(AlpacaCalendarAcquisition {
            provider: ALPACA_SRC.into(),
            actual_upstream: "Alpaca U.S. Equity session rules".into(),
            method: "versioned America/New_York regular-session calendar".into(),
            connector_version: ALPACA_CONNECTOR_VERSION.into(),
            request_parameters: json!({
                "venue": venue.id,
                "startTimeMs": range.start_time_ms,
                "endTimeMs": range.end_time_ms,
            }),
            retrieved_at_ms,
            response_sha256: response_sha256.clone(),
            content_sha256: response_sha256,
            raw_response,
            diagnostics: AlpacaRequestDiagnostics {
                notes: vec![
                    "Calendar uses America/New_York IANA rules; UTC instants are stored for every session boundary".into(),
                ],
                ..Default::default()
            },
            snapshot,
            limitations: vec![
                "Provider calendar evidence covers regular U.S. equity sessions; security-specific halts and IPO dates remain separate market evidence".into(),
            ],
        })
    }

    pub async fn stream<F>(
        &self,
        subscription: AlpacaStreamSubscription,
        mut on_event: F,
    ) -> Result<(), DataError>
    where
        F: FnMut(AlpacaStreamEvent) -> bool,
    {
        validate_subscription(&subscription)?;
        let mut retry_seconds = 1;
        loop {
            let received_data = Arc::new(Mutex::new(false));
            let received_data_for_callback = received_data.clone();
            let result = self
                .stream_once(&subscription, |event| {
                    if matches!(
                        event,
                        AlpacaStreamEvent::Trade(_)
                            | AlpacaStreamEvent::Quote(_)
                            | AlpacaStreamEvent::Bar(_)
                    ) {
                        if let Ok(mut value) = received_data_for_callback.lock() {
                            *value = true;
                        }
                    }
                    on_event(event)
                })
                .await;
            let had_data = received_data.lock().map(|value| *value).unwrap_or(false);
            if had_data {
                retry_seconds = 1;
            }
            let error_value = match result {
                Ok(()) => error("connection_closed", "Alpaca market-data WebSocket closed"),
                Err(value) => value,
            };
            if !on_event(AlpacaStreamEvent::Error(error_value)) {
                break;
            }
            let delay_ms = retry_seconds * 1_000;
            if !on_event(AlpacaStreamEvent::Reconnecting { delay_ms }) {
                break;
            }
            tokio::time::sleep(Duration::from_secs(retry_seconds)).await;
            retry_seconds = (retry_seconds * 2).min(ALPACA_MAX_RETRY_SECONDS);
        }
        let _ = on_event(AlpacaStreamEvent::Closed);
        Ok(())
    }

    async fn stream_once<F>(
        &self,
        subscription: &AlpacaStreamSubscription,
        mut on_event: F,
    ) -> Result<(), DataError>
    where
        F: FnMut(AlpacaStreamEvent) -> bool,
    {
        let (mut socket, _) = connect_async(&self.stream_url)
            .await
            .map_err(|value| error("transport", value.to_string()))?;
        if !on_event(AlpacaStreamEvent::Connected) {
            return Ok(());
        }
        socket
            .send(Message::Text(
                json!({
                    "action": "auth",
                    "key": self.credentials.key_id,
                    "secret": self.credentials.secret_key,
                })
                .to_string()
                .into(),
            ))
            .await
            .map_err(|value| error("transport", value.to_string()))?;

        let mut authenticated = false;
        let mut subscribed = false;
        let heartbeat = Duration::from_secs(ALPACA_HEARTBEAT_SECONDS);
        let mut awaiting_pong = false;
        loop {
            let message = match tokio::time::timeout(heartbeat, socket.next()).await {
                Ok(Some(Ok(message))) => message,
                Ok(Some(Err(value))) => return Err(error("transport", value.to_string())),
                Ok(None) => return Err(error("connection_closed", "Alpaca stream closed")),
                Err(_) if awaiting_pong => {
                    return Err(error(
                        "heartbeat_timeout",
                        "Alpaca stream heartbeat timed out",
                    ));
                }
                Err(_) => {
                    socket
                        .send(Message::Text("ping".into()))
                        .await
                        .map_err(|value| error("transport", value.to_string()))?;
                    awaiting_pong = true;
                    continue;
                }
            };
            match message {
                Message::Text(text) if text == "pong" => awaiting_pong = false,
                Message::Text(text) => {
                    awaiting_pong = false;
                    let values: Vec<Value> = serde_json::from_str(&text)
                        .map_err(|value| error("invalid_response", value.to_string()))?;
                    for value in values {
                        let kind = value.get("T").and_then(Value::as_str).unwrap_or_default();
                        match kind {
                            "success"
                                if value.get("msg").and_then(Value::as_str)
                                    == Some("connected") => {}
                            "success"
                                if value.get("msg").and_then(Value::as_str)
                                    == Some("authenticated") =>
                            {
                                authenticated = true;
                                if !on_event(AlpacaStreamEvent::Authenticated) {
                                    return Ok(());
                                }
                                if !subscribed {
                                    socket
                                        .send(Message::Text(
                                            subscription_message(subscription).to_string().into(),
                                        ))
                                        .await
                                        .map_err(|value| error("transport", value.to_string()))?;
                                    subscribed = true;
                                }
                            }
                            "subscription" => {
                                if !on_event(AlpacaStreamEvent::Subscribed) {
                                    return Ok(());
                                }
                            }
                            "error" => {
                                return Err(error(
                                    value
                                        .get("code")
                                        .and_then(Value::as_i64)
                                        .unwrap_or_default()
                                        .to_string(),
                                    value
                                        .get("msg")
                                        .and_then(Value::as_str)
                                        .unwrap_or("Alpaca stream error"),
                                ));
                            }
                            "t" if authenticated => {
                                if !on_event(AlpacaStreamEvent::Trade(parse_stream_trade(&value)?))
                                {
                                    return Ok(());
                                }
                            }
                            "q" if authenticated => {
                                if !on_event(AlpacaStreamEvent::Quote(parse_stream_quote(
                                    &value, "iex",
                                )?)) {
                                    return Ok(());
                                }
                            }
                            "b" | "u" if authenticated => {
                                if !on_event(AlpacaStreamEvent::Bar(parse_stream_bar(
                                    &value,
                                    kind == "b",
                                )?)) {
                                    return Ok(());
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Message::Ping(payload) => {
                    socket
                        .send(Message::Pong(payload))
                        .await
                        .map_err(|value| error("transport", value.to_string()))?;
                }
                Message::Pong(_) => awaiting_pong = false,
                Message::Close(_) => {
                    return Err(error("connection_closed", "Alpaca stream closed"));
                }
                _ => {}
            }
        }
    }

    async fn get_bytes(
        &self,
        path: &str,
        query: &[(String, String)],
    ) -> Result<AlpacaHttpResponse, DataError> {
        if self.credentials.key_id.trim().is_empty()
            || self.credentials.secret_key.trim().is_empty()
        {
            return Err(error("invalid_credentials", "Alpaca credentials are empty"));
        }
        let max_attempts = self.policy.max_attempts.max(1);
        let mut retry_delay_ms = self.policy.retry_delay_ms;
        let mut diagnostics = AlpacaRequestDiagnostics::default();
        for attempt in 0..max_attempts {
            diagnostics.request_count += 1;
            self.wait_for_rate_limit().await?;
            let response = match self
                .http
                .get(format!("{}{}", self.base_url, path))
                .query(query)
                .header("APCA-API-KEY-ID", &self.credentials.key_id)
                .header("APCA-API-SECRET-KEY", &self.credentials.secret_key)
                .send()
                .await
            {
                Ok(response) => response,
                Err(_) if attempt + 1 < max_attempts => {
                    diagnostics.retry_count += 1;
                    tokio::time::sleep(Duration::from_millis(retry_delay_ms)).await;
                    retry_delay_ms =
                        next_retry_delay(retry_delay_ms, self.policy.max_retry_delay_ms);
                    continue;
                }
                Err(value) => return Err(error("transport", value.to_string())),
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
                .map_err(|value| error("transport", value.to_string()))?
                .to_vec();
            if retryable && attempt + 1 < max_attempts {
                diagnostics.retry_count += 1;
                let delay = retry_after_ms.unwrap_or(retry_delay_ms);
                tokio::time::sleep(Duration::from_millis(delay)).await;
                retry_delay_ms = next_retry_delay(retry_delay_ms, self.policy.max_retry_delay_ms);
                continue;
            }
            if !status.is_success() {
                return Err(with_raw(
                    error(
                        if status == reqwest::StatusCode::UNAUTHORIZED
                            || status == reqwest::StatusCode::FORBIDDEN
                        {
                            "auth_failed"
                        } else if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                            "rate_limited"
                        } else {
                            "http_status"
                        },
                        format!("Alpaca returned HTTP {status}"),
                    ),
                    &bytes,
                ));
            }
            return Ok(AlpacaHttpResponse { bytes, diagnostics });
        }
        unreachable!("Alpaca request attempts is always at least one")
    }

    async fn wait_for_rate_limit(&self) -> Result<(), DataError> {
        let now = Instant::now();
        let wait = {
            let mut next = self
                .next_request_at
                .lock()
                .map_err(|_| error("internal", "Alpaca rate-limit gate is poisoned"))?;
            let scheduled = (*next).max(now);
            let wait = scheduled.saturating_duration_since(now);
            *next = scheduled
                .checked_add(Duration::from_millis(self.policy.min_delay_ms))
                .unwrap_or(scheduled);
            wait
        };
        if wait > Duration::ZERO {
            tokio::time::sleep(wait).await;
        }
        Ok(())
    }

    fn normalize_asset(
        &self,
        asset: RawAsset,
        captured_at_ms: i64,
    ) -> Result<AlpacaInstrument, DataError> {
        validate_symbol(&asset.symbol)?;
        if !asset.class.eq_ignore_ascii_case("us_equity") {
            return Err(error(
                "invalid_instrument",
                "Alpaca asset is not a U.S. equity",
            ));
        }
        let exchange = asset.exchange.trim().to_ascii_lowercase();
        if exchange.is_empty() {
            return Err(error("invalid_instrument", "Alpaca asset has no exchange"));
        }
        let venue = Venue::us_equity(exchange.clone())
            .map_err(|value| error("invalid_venue", value.to_string()))?;
        let instrument = InstrumentId::new(venue, asset.symbol.clone())
            .map_err(|value| error("invalid_instrument", value.to_string()))?;
        let status = if asset.status.eq_ignore_ascii_case("active") && asset.tradable {
            InstrumentStatus::Live
        } else if asset.status.eq_ignore_ascii_case("active") {
            InstrumentStatus::Suspended
        } else {
            InstrumentStatus::Unknown
        };
        Ok(AlpacaInstrument {
            mapping: InstrumentSourceMapping {
                instrument: instrument.clone(),
                provider: ALPACA_SRC.into(),
                provider_symbol: asset.symbol.clone(),
                connector_version: ALPACA_CONNECTOR_VERSION.into(),
                captured_at_ms,
            },
            instrument,
            provider_symbol: asset.symbol,
            name: asset.name.filter(|value| !value.trim().is_empty()),
            status,
            asset_class: asset.class,
            exchange,
            tradable: asset.tradable,
            marginable: asset.marginable,
            shortable: asset.shortable,
            easy_to_borrow: asset.easy_to_borrow,
            fractionable: asset.fractionable,
            listing_time_ms: None,
            continuous_trading_time_ms: None,
            price_increment: asset.price_increment,
            quantity_increment: asset.quantity_increment,
            minimum_quantity: asset.minimum_quantity,
        })
    }

    fn parse_bar(
        &self,
        instrument: &InstrumentId,
        interval: BarInterval,
        raw: &Value,
    ) -> Result<AlpacaBar, String> {
        let timestamp = timestamp_ms(raw.get("t")).map_err(|value| value.message)?;
        let open_time_ms = if is_daily_interval(interval) {
            let local_date = instrument
                .venue
                .local_time(timestamp)
                .map_err(|value| value.to_string())?
                .date();
            instrument
                .venue
                .resolve_local_time(
                    local_date.and_hms_opt(9, 30, 0).ok_or_else(|| {
                        "Alpaca daily bar session-open time is invalid".to_owned()
                    })?,
                    LocalTimeDisambiguation::Reject,
                )
                .map_err(|value| value.to_string())?
        } else {
            timestamp
        };
        let open = value_string(raw.get("o"));
        let high = value_string(raw.get("h"));
        let low = value_string(raw.get("l"));
        let close = value_string(raw.get("c"));
        let base_volume = value_string(raw.get("v"));
        let vwap = value_string(raw.get("vw"));
        if [
            open.as_ref(),
            high.as_ref(),
            low.as_ref(),
            close.as_ref(),
            base_volume.as_ref(),
            vwap.as_ref(),
        ]
        .iter()
        .any(Option::is_none)
        {
            return Err("Alpaca bar is missing an OHLCV/VWAP field".into());
        }
        let quote_volume = match (vwap.as_deref(), base_volume.as_deref()) {
            (Some(vwap), Some(volume)) => {
                let vwap = Decimal::from_str(vwap).map_err(|value| value.to_string())?;
                let volume = Decimal::from_str(volume).map_err(|value| value.to_string())?;
                Some((vwap * volume).to_string())
            }
            _ => None,
        };
        Ok(AlpacaBar {
            instrument: instrument.clone(),
            provider_symbol: instrument.code.clone(),
            interval,
            open_time_ms,
            open,
            high,
            low,
            close,
            base_volume,
            quote_volume,
            raw_payload: raw.clone(),
        })
    }

    fn invalid_bar(
        &self,
        instrument: &InstrumentId,
        interval: BarInterval,
        raw: &Value,
    ) -> AlpacaBar {
        AlpacaBar {
            instrument: instrument.clone(),
            provider_symbol: instrument.code.clone(),
            interval,
            open_time_ms: timestamp_ms(raw.get("t")).unwrap_or_default(),
            open: value_string(raw.get("o")),
            high: value_string(raw.get("h")),
            low: value_string(raw.get("l")),
            close: value_string(raw.get("c")),
            base_volume: value_string(raw.get("v")),
            quote_volume: value_string(raw.get("vw")),
            raw_payload: raw.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlpacaCapabilitySnapshot {
    pub subscription_plan: String,
    pub feed: String,
    pub coverage: String,
    pub realtime: bool,
    pub delayed: bool,
    pub delay_ms: Option<u64>,
    pub history_start_ms: Option<i64>,
    pub historical_latest_cutoff_ms: Option<i64>,
    pub historical_calls_per_minute: u32,
    pub stream_connection_limit: usize,
    pub stream_symbol_limit: usize,
    pub record_types: Vec<String>,
    pub unavailable_capabilities: Vec<String>,
    pub captured_at_ms: i64,
}

impl AlpacaCapabilitySnapshot {
    pub fn basic(captured_at_ms: i64) -> Self {
        let cutoff = captured_at_ms.checked_sub(ALPACA_HISTORY_LATEST_DELAY_MS);
        Self {
            subscription_plan: "Basic".into(),
            feed: "iex".into(),
            coverage: "iex-only".into(),
            realtime: true,
            delayed: false,
            delay_ms: None,
            history_start_ms: NaiveDate::from_ymd_opt(ALPACA_BASIC_HISTORY_START_YEAR, 1, 1)
                .and_then(|date| date.and_hms_opt(0, 0, 0))
                .map(|value| value.and_utc().timestamp_millis()),
            historical_latest_cutoff_ms: cutoff,
            historical_calls_per_minute: ALPACA_BASIC_HISTORICAL_CALLS_PER_MINUTE,
            stream_connection_limit: ALPACA_BASIC_STREAM_CONNECTION_LIMIT,
            stream_symbol_limit: ALPACA_BASIC_STREAM_SYMBOL_LIMIT,
            record_types: vec![
                "instrument-master".into(),
                "bars".into(),
                "ticker".into(),
                "trade".into(),
                "quote".into(),
            ],
            unavailable_capabilities: vec![
                "consolidated-us-equity-realtime".into(),
                "full-market-volume".into(),
                "historical-bars-newer-than-15-minutes".into(),
                "unadjusted-corporate-actions".into(),
            ],
            captured_at_ms,
        }
    }
}

struct AlpacaHttpResponse {
    bytes: Vec<u8>,
    diagnostics: AlpacaRequestDiagnostics,
}

#[derive(Debug, Deserialize)]
struct RawAsset {
    symbol: String,
    name: Option<String>,
    status: String,
    class: String,
    exchange: String,
    #[serde(default)]
    tradable: bool,
    #[serde(default)]
    marginable: bool,
    #[serde(default)]
    shortable: bool,
    #[serde(default)]
    easy_to_borrow: bool,
    #[serde(default)]
    fractionable: bool,
    #[serde(default)]
    price_increment: Option<String>,
    #[serde(default)]
    quantity_increment: Option<String>,
    #[serde(default)]
    minimum_quantity: Option<String>,
}

fn parse_latest_trade(symbol: &str, value: Option<&Value>) -> Result<MarketTrade, DataError> {
    let value =
        value.ok_or_else(|| error("invalid_response", "Alpaca snapshot has no latest trade"))?;
    Ok(MarketTrade {
        src: ALPACA_SRC.into(),
        code: symbol.into(),
        trade_id: value_string(value.get("i"))
            .ok_or_else(|| error("invalid_response", "trade id missing"))?,
        price: decimal_value(value.get("p"), "trade price")?,
        quantity: decimal_value(value.get("s"), "trade size")?,
        side: MarketTradeSide::Unknown,
        timestamp_ms: timestamp_ms(value.get("t"))?,
    })
}

fn parse_latest_quote(
    symbol: &str,
    value: Option<&Value>,
    feed: &str,
) -> Result<AlpacaQuote, DataError> {
    let value =
        value.ok_or_else(|| error("invalid_response", "Alpaca snapshot has no latest quote"))?;
    Ok(AlpacaQuote {
        src: ALPACA_SRC.into(),
        code: symbol.into(),
        ask_price: optional_decimal(value.get("ap"), "ask price")?,
        ask_quantity: optional_decimal(value.get("as"), "ask size")?,
        bid_price: optional_decimal(value.get("bp"), "bid price")?,
        bid_quantity: optional_decimal(value.get("bs"), "bid size")?,
        timestamp_ms: timestamp_ms(value.get("t"))?,
        ask_exchange: value_string(value.get("ax")),
        bid_exchange: value_string(value.get("bx")),
        feed: feed.into(),
    })
}

fn parse_stream_trade(value: &Value) -> Result<MarketTrade, DataError> {
    let symbol = value_string(value.get("S"))
        .ok_or_else(|| error("invalid_response", "trade symbol missing"))?;
    Ok(MarketTrade {
        src: ALPACA_SRC.into(),
        code: symbol,
        trade_id: value_string(value.get("i"))
            .ok_or_else(|| error("invalid_response", "trade id missing"))?,
        price: decimal_value(value.get("p"), "trade price")?,
        quantity: decimal_value(value.get("s"), "trade size")?,
        side: MarketTradeSide::Unknown,
        timestamp_ms: timestamp_ms(value.get("t"))?,
    })
}

fn parse_stream_quote(value: &Value, feed: &str) -> Result<AlpacaQuote, DataError> {
    let symbol = value_string(value.get("S"))
        .ok_or_else(|| error("invalid_response", "quote symbol missing"))?;
    Ok(AlpacaQuote {
        src: ALPACA_SRC.into(),
        code: symbol,
        ask_price: optional_decimal(value.get("ap"), "ask price")?,
        ask_quantity: optional_decimal(value.get("as"), "ask size")?,
        bid_price: optional_decimal(value.get("bp"), "bid price")?,
        bid_quantity: optional_decimal(value.get("bs"), "bid size")?,
        timestamp_ms: timestamp_ms(value.get("t"))?,
        ask_exchange: value_string(value.get("ax")),
        bid_exchange: value_string(value.get("bx")),
        feed: feed.into(),
    })
}

fn parse_stream_bar(value: &Value, closed: bool) -> Result<BarSnapshot, DataError> {
    let symbol = value_string(value.get("S"))
        .ok_or_else(|| error("invalid_response", "bar symbol missing"))?;
    let open_time_ms = timestamp_ms(value.get("t"))?;
    let parse = |field: &str| decimal_value(value.get(field), field);
    Ok(BarSnapshot {
        src: ALPACA_SRC.into(),
        code: symbol,
        interval: BarInterval::OneMinute,
        bar: crate::OhlcvBar {
            open_time_ms,
            open: parse("o")?,
            high: parse("h")?,
            low: parse("l")?,
            close: parse("c")?,
            base_volume: parse("v")?,
            quote_volume: parse("vw")? * parse("v")?,
        },
        closed,
    })
}

fn subscription_message(subscription: &AlpacaStreamSubscription) -> Value {
    let symbols = &subscription.symbols;
    let mut value = json!({"action":"subscribe"});
    if subscription.trades {
        value["trades"] = json!(symbols);
    }
    if subscription.quotes {
        value["quotes"] = json!(symbols);
    }
    if subscription.bars {
        value["bars"] = json!(symbols);
    }
    value
}

fn validate_subscription(subscription: &AlpacaStreamSubscription) -> Result<(), DataError> {
    if subscription.symbols.is_empty()
        || subscription.symbols.len() > ALPACA_BASIC_STREAM_SYMBOL_LIMIT
    {
        return Err(error(
            "stream_limit",
            format!(
                "Alpaca Basic permits at most {ALPACA_BASIC_STREAM_SYMBOL_LIMIT} symbols per connection"
            ),
        ));
    }
    if !subscription.trades && !subscription.quotes && !subscription.bars {
        return Err(error(
            "invalid_request",
            "Alpaca stream has no subscribed channels",
        ));
    }
    let mut symbols = BTreeSet::new();
    for symbol in &subscription.symbols {
        validate_symbol(symbol)?;
        if !symbols.insert(symbol) {
            return Err(error(
                "invalid_request",
                "Alpaca stream symbols must be unique",
            ));
        }
    }
    Ok(())
}

fn validate_symbol(symbol: &str) -> Result<(), DataError> {
    if symbol.trim().is_empty()
        || symbol.len() > 16
        || !symbol
            .chars()
            .all(|value| value.is_ascii_alphanumeric() || value == '.' || value == '-')
    {
        return Err(error("invalid_request", "Alpaca symbol is invalid"));
    }
    Ok(())
}

fn alpaca_timeframe(interval: BarInterval) -> Result<&'static str, DataError> {
    match interval {
        BarInterval::OneMinute => Ok("1Min"),
        BarInterval::FiveMinutes => Ok("5Min"),
        BarInterval::FifteenMinutes => Ok("15Min"),
        BarInterval::OneHour => Ok("1Hour"),
        BarInterval::OneDay => Ok("1Day"),
        _ => Err(error(
            "unsupported_interval",
            "Alpaca Basic supports 1m, 5m, 15m, 1h, and 1d bars in this connector",
        )),
    }
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

fn decimal_value(value: Option<&Value>, field: &str) -> Result<Decimal, DataError> {
    let text = value_string(value)
        .ok_or_else(|| error("invalid_decimal", format!("Alpaca {field} is missing")))?;
    Decimal::from_str(&text).map_err(|value| {
        error(
            "invalid_decimal",
            format!("Alpaca {field} is invalid: {value}"),
        )
    })
}

fn optional_decimal(value: Option<&Value>, field: &str) -> Result<Option<Decimal>, DataError> {
    value
        .map(|value| decimal_value(Some(value), field))
        .transpose()
}

fn value_string(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn timestamp_ms(value: Option<&Value>) -> Result<i64, DataError> {
    let value = value_string(value)
        .ok_or_else(|| error("invalid_response", "Alpaca timestamp is missing"))?;
    if let Ok(value) = value.parse::<i64>() {
        return Ok(value);
    }
    DateTime::parse_from_rfc3339(&value)
        .map(|value| value.timestamp_millis())
        .map_err(|value| {
            error(
                "invalid_response",
                format!("Alpaca timestamp is invalid: {value}"),
            )
        })
}

fn format_rfc3339(value: i64) -> Result<String, DataError> {
    DateTime::<Utc>::from_timestamp_millis(value)
        .map(|value| value.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .ok_or_else(|| error("invalid_request", "Alpaca time is outside the UTC range"))
}

fn next_retry_delay(current_ms: u64, max_ms: u64) -> u64 {
    current_ms.saturating_mul(2).min(max_ms.max(current_ms))
}

fn error(code: impl Into<String>, message: impl Into<String>) -> DataError {
    DataError::new(ALPACA_SRC, code, message)
}

fn with_raw(mut value: DataError, bytes: &[u8]) -> DataError {
    value.response_sha256 = Some(sha256(bytes));
    value.raw_response = Some(bytes.to_vec());
    value
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

fn canonical_json_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, DataError> {
    serde_json::to_vec(value).map_err(|value| error("serialization", value.to_string()))
}

fn build_us_calendar(
    venue: &Venue,
    range: HistoricalBarRange,
) -> Result<TradingCalendarSnapshot, DataError> {
    let start_date = venue
        .local_time(range.start_time_ms)
        .map_err(|value| error("invalid_calendar", value.to_string()))?
        .date();
    let end_date = venue
        .local_time(range.end_time_ms.saturating_sub(1))
        .map_err(|value| error("invalid_calendar", value.to_string()))?
        .date();
    let sessions = vec![TradingSession {
        phase: SessionPhase::Continuous,
        start_local: NaiveTime::from_hms_opt(9, 30, 0).expect("valid U.S. session"),
        end_local: NaiveTime::from_hms_opt(16, 0, 0).expect("valid U.S. session"),
    }];
    let mut days = Vec::new();
    let mut date = start_date;
    while date <= end_date {
        let trading_date = TradingDate::from_naive_date(date);
        let day = if matches!(date.weekday(), chrono::Weekday::Sat | chrono::Weekday::Sun) {
            DayEvidence::closed(trading_date, DayKind::Weekend)
        } else if let Some(kind) = us_holiday(date) {
            DayEvidence::closed(trading_date, kind)
        } else if is_us_early_close(date) {
            let close = ScheduledClosure {
                kind: ScheduledClosureKind::EarlyClose,
                start_ms: venue
                    .resolve_local_time(
                        date.and_time(NaiveTime::from_hms_opt(13, 0, 0).expect("valid time")),
                        LocalTimeDisambiguation::Reject,
                    )
                    .map_err(|value| error("invalid_calendar", value.to_string()))?,
                end_ms: venue
                    .resolve_local_time(
                        date.and_time(NaiveTime::from_hms_opt(16, 0, 0).expect("valid time")),
                        LocalTimeDisambiguation::Reject,
                    )
                    .map_err(|value| error("invalid_calendar", value.to_string()))?,
                reason: Some("U.S. equity early close".into()),
            };
            DayEvidence {
                date: trading_date,
                day_kind: DayKind::TradingDay,
                session_override: Some(vec![TradingSession {
                    phase: SessionPhase::Continuous,
                    start_local: NaiveTime::from_hms_opt(9, 30, 0).expect("valid session"),
                    end_local: NaiveTime::from_hms_opt(13, 0, 0).expect("valid session"),
                }]),
                closures: vec![close],
            }
        } else {
            DayEvidence::trading_day(trading_date)
        };
        days.push(day);
        date = date
            .succ_opt()
            .ok_or_else(|| error("invalid_calendar", "calendar date overflow"))?;
    }
    let bytes = canonical_json_bytes(&(venue, range, &sessions, &days))?;
    TradingCalendarSnapshot::new(
        format!("alpaca-us-calendar-{}", sha256(&bytes)),
        venue.clone(),
        range.start_time_ms,
        range.end_time_ms,
        sessions,
        days,
    )
    .map_err(|value| error("invalid_calendar", value.to_string()))
}

fn us_holiday(date: NaiveDate) -> Option<DayKind> {
    let year = date.year();
    let observed = |month: u32, day: u32| {
        let date = NaiveDate::from_ymd_opt(year, month, day)?;
        Some(if date.weekday() == chrono::Weekday::Sat {
            date - ChronoDuration::days(1)
        } else if date.weekday() == chrono::Weekday::Sun {
            date + ChronoDuration::days(1)
        } else {
            date
        })
    };
    if observed(1, 1) == Some(date)
        || observed(6, 19) == Some(date)
        || observed(7, 4) == Some(date)
        || observed(12, 25) == Some(date)
    {
        return Some(DayKind::Holiday);
    }
    if Some(date) == nth_weekday(year, 1, chrono::Weekday::Mon, 3)
        || Some(date) == nth_weekday(year, 2, chrono::Weekday::Mon, 3)
        || Some(date) == last_weekday(year, 5, chrono::Weekday::Mon)
        || Some(date) == nth_weekday(year, 9, chrono::Weekday::Mon, 1)
        || Some(date) == nth_weekday(year, 11, chrono::Weekday::Thu, 4)
    {
        return Some(DayKind::Holiday);
    }
    if Some(date) == good_friday(year) {
        return Some(DayKind::Holiday);
    }
    None
}

fn is_us_early_close(date: NaiveDate) -> bool {
    let year = date.year();
    let independence_eve = NaiveDate::from_ymd_opt(year, 7, 3);
    let thanksgiving = nth_weekday(year, 11, chrono::Weekday::Thu, 4);
    let thanksgiving_friday = thanksgiving.map(|value| value + ChronoDuration::days(1));
    let christmas_eve = NaiveDate::from_ymd_opt(year, 12, 24);
    Some(date) == independence_eve
        || Some(date) == thanksgiving_friday
        || Some(date) == christmas_eve
}

fn nth_weekday(year: i32, month: u32, weekday: chrono::Weekday, ordinal: u32) -> Option<NaiveDate> {
    let first = NaiveDate::from_ymd_opt(year, month, 1)?;
    let offset = (weekday.num_days_from_monday() + 7 - first.weekday().num_days_from_monday()) % 7;
    first.checked_add_days(chrono::Days::new(u64::from(offset + 7 * (ordinal - 1))))
}

fn last_weekday(year: i32, month: u32, weekday: chrono::Weekday) -> Option<NaiveDate> {
    let next = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)?
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)?
    };
    let mut date = next - ChronoDuration::days(1);
    while date.weekday() != weekday {
        date -= ChronoDuration::days(1);
    }
    Some(date)
}

fn good_friday(year: i32) -> Option<NaiveDate> {
    // Anonymous Gregorian computus; the exchange holiday is two days before Easter.
    let a = year % 19;
    let b = year / 100;
    let c = year % 100;
    let d = b / 4;
    let e = b % 4;
    let f = (b + 8) / 25;
    let g = (b - f + 1) / 3;
    let h = (19 * a + b - d - g + 15) % 30;
    let i = c / 4;
    let k = c % 4;
    let l = (32 + 2 * e + 2 * i - h - k) % 7;
    let m = (a + 11 * h + 22 * l) / 451;
    let month = (h + l - 7 * m + 114) / 31;
    let day = (h + l - 7 * m + 114) % 31 + 1;
    NaiveDate::from_ymd_opt(year, month as u32, day as u32)
        .map(|date| date - ChronoDuration::days(2))
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    use super::*;
    use chrono::Timelike;

    #[test]
    fn basic_capability_snapshot_is_explicit_and_credential_free() {
        let capability = AlpacaCapabilitySnapshot::basic(1_700_000_000_000);
        assert_eq!(capability.subscription_plan, "Basic");
        assert_eq!(capability.feed, "iex");
        assert_eq!(capability.coverage, "iex-only");
        assert_eq!(capability.historical_calls_per_minute, 200);
        assert_eq!(capability.stream_symbol_limit, 30);
        assert!(
            capability
                .unavailable_capabilities
                .contains(&"consolidated-us-equity-realtime".into())
        );
        let serialized = serde_json::to_string(&capability).unwrap();
        assert!(!serialized.contains("secret"));
    }

    #[test]
    fn us_calendar_handles_dst_holiday_and_early_close() {
        let venue = Venue::us_equity("nasdaq").unwrap();
        let start = venue
            .resolve_local_time(
                NaiveDate::from_ymd_opt(2024, 7, 1)
                    .unwrap()
                    .and_hms_opt(0, 0, 0)
                    .unwrap(),
                LocalTimeDisambiguation::Reject,
            )
            .unwrap();
        let end = venue
            .resolve_local_time(
                NaiveDate::from_ymd_opt(2024, 11, 5)
                    .unwrap()
                    .and_hms_opt(0, 0, 0)
                    .unwrap(),
                LocalTimeDisambiguation::Reject,
            )
            .unwrap();
        let calendar = build_us_calendar(
            &venue,
            HistoricalBarRange {
                start_time_ms: start,
                end_time_ms: end,
            },
        )
        .unwrap();
        assert_eq!(calendar.venue.time_zone, "America/New_York");
        assert_eq!(
            calendar
                .day(TradingDate::new(2024, 7, 4).unwrap())
                .unwrap()
                .day_kind,
            DayKind::Holiday
        );
        let early = calendar.day(TradingDate::new(2024, 7, 3).unwrap()).unwrap();
        assert_eq!(
            early.session_override.as_ref().unwrap()[0].end_local,
            NaiveTime::from_hms_opt(13, 0, 0).unwrap()
        );
        let november_open = venue
            .resolve_local_time(
                NaiveDate::from_ymd_opt(2024, 11, 4)
                    .unwrap()
                    .and_hms_opt(9, 30, 0)
                    .unwrap(),
                LocalTimeDisambiguation::Reject,
            )
            .unwrap();
        assert_eq!(venue.local_time(november_open).unwrap().hour(), 9);
    }

    #[test]
    fn subscription_enforces_basic_symbol_limit() {
        let symbols = (0..31).map(|value| format!("S{value}")).collect();
        let result = validate_subscription(&AlpacaStreamSubscription::trades(symbols));
        assert_eq!(result.unwrap_err().code, "stream_limit");
    }

    #[test]
    fn daily_bars_anchor_to_the_new_york_session_open() {
        let venue = Venue::us_equity("nasdaq").unwrap();
        let instrument = InstrumentId::new(venue.clone(), "AAPL").unwrap();
        let client = AlpacaClient::with_key_pair("key", "secret");
        let bar = client
            .parse_bar(
                &instrument,
                BarInterval::OneDay,
                &json!({
                    "t": "2024-01-08T05:00:00Z",
                    "o": "1.00",
                    "h": "1.10",
                    "l": "0.90",
                    "c": "1.05",
                    "v": "10",
                    "vw": "1.02"
                }),
            )
            .unwrap();
        assert_eq!(
            bar.open_time_ms,
            venue
                .resolve_local_time(
                    NaiveDate::from_ymd_opt(2024, 1, 8)
                        .unwrap()
                        .and_hms_opt(9, 30, 0)
                        .unwrap(),
                    LocalTimeDisambiguation::Reject,
                )
                .unwrap()
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn local_mock_covers_auth_pagination_exact_values_and_snapshot() {
        let responses = vec![
            (
                200,
                r#"[{"symbol":"AAPL","name":"Apple Inc.","status":"active","class":"us_equity","exchange":"NASDAQ","tradable":true,"marginable":true,"shortable":true,"easy_to_borrow":true,"fractionable":true}]"#,
            ),
            (
                200,
                r#"{"bars":{"AAPL":[{"t":"2024-01-02T14:30:00Z","o":"1.2300","h":"1.2400","l":"1.2200","c":"1.2350","v":"2","vw":"1.2300","n":1}]},"next_page_token":"page-2"}"#,
            ),
            (
                200,
                r#"{"bars":{"AAPL":[{"t":"2024-01-02T14:31:00Z","o":"1.2350","h":"1.2500","l":"1.2300","c":"1.2450","v":"3","vw":"1.2400","n":1}]}}"#,
            ),
            (
                200,
                r#"{"latestTrade":{"i":"trade-1","p":"1.2450","s":"4","t":"2024-01-02T14:31:30Z"},"latestQuote":{"ax":"Q","ap":"1.2500","as":"5","bx":"B","bp":"1.2400","bs":"6","t":"2024-01-02T14:31:29Z"},"dailyBar":{"o":"1.2000","h":"1.2600","l":"1.1900","c":"1.2450","v":"10","vw":"1.2300","n":2}}"#,
            ),
        ];
        let (base_url, server) = serve_http(responses);
        let client = AlpacaClient::with_urls_and_policy(
            AlpacaCredentials::new("key", "secret"),
            base_url,
            "ws://127.0.0.1:1/stream",
            AlpacaRequestPolicy {
                max_attempts: 1,
                min_delay_ms: 0,
                retry_delay_ms: 0,
                max_retry_delay_ms: 0,
            },
        );
        let master = client
            .acquire_instrument_master(1_704_160_000_000)
            .await
            .unwrap();
        assert_eq!(master.instruments[0].instrument.code, "AAPL");
        assert_eq!(master.instruments[0].mapping.provider_symbol, "AAPL");
        let instrument = master.instruments[0].instrument.clone();
        let bars = client
            .acquire_bars(
                instrument,
                BarInterval::OneMinute,
                HistoricalBarRange {
                    start_time_ms: 1_704_160_000_000,
                    end_time_ms: 1_704_160_100_000,
                },
                1_704_160_000_000,
                || false,
            )
            .await
            .unwrap();
        assert_eq!(bars.bars.len(), 2);
        assert_eq!(bars.bars[0].open.as_deref(), Some("1.2300"));
        assert_eq!(bars.diagnostics.page_count, 2);
        let snapshot = client
            .get_snapshot("AAPL", 1_704_160_000_000)
            .await
            .unwrap();
        assert_eq!(snapshot.trade.price.to_string(), "1.2450");
        assert_eq!(snapshot.quote.ask_price.unwrap().to_string(), "1.2500");
        assert!(serde_json::to_string(&snapshot).unwrap().contains("1.2500"));
        server.join().unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn local_mock_retries_basic_rate_limit() {
        let (base_url, server) = serve_http(vec![
            (429, r#"{"message":"rate limited"}"#),
            (
                200,
                r#"[{"symbol":"AAPL","name":"Apple Inc.","status":"active","class":"us_equity","exchange":"NASDAQ","tradable":true}]"#,
            ),
        ]);
        let client = AlpacaClient::with_urls_and_policy(
            AlpacaCredentials::new("key", "secret"),
            base_url,
            "ws://127.0.0.1:1/stream",
            AlpacaRequestPolicy {
                max_attempts: 2,
                min_delay_ms: 0,
                retry_delay_ms: 0,
                max_retry_delay_ms: 0,
            },
        );
        let acquisition = client
            .acquire_instrument_master(1_704_160_000_000)
            .await
            .unwrap();
        assert_eq!(acquisition.diagnostics.retry_count, 1);
        assert_eq!(acquisition.diagnostics.response_statuses, [429, 200]);
        server.join().unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn local_mock_stream_reconnects_and_preserves_updates() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
                let Some(Ok(Message::Text(auth))) = socket.next().await else {
                    panic!("stream auth was not received")
                };
                assert!(auth.contains("\"key\":\"key\""));
                assert!(auth.contains("\"secret\":\"secret\""));
                socket
                    .send(Message::Text(
                        r#"[{"T":"success","msg":"connected"}]"#.into(),
                    ))
                    .await
                    .unwrap();
                socket
                    .send(Message::Text(
                        r#"[{"T":"success","msg":"authenticated"}]"#.into(),
                    ))
                    .await
                    .unwrap();
                let Some(Ok(Message::Text(_subscribe))) = socket.next().await else {
                    panic!("stream subscription was not received")
                };
                for message in [
                    r#"[{"T":"subscription","trades":["AAPL"]}]"#,
                    r#"[{"T":"t","S":"AAPL","i":"trade-1","p":"1.2450","s":"4","t":"2024-01-02T14:31:30Z"}]"#,
                    r#"[{"T":"u","S":"AAPL","o":"1.2300","h":"1.2500","l":"1.2200","c":"1.2460","v":"3","vw":"1.2400","n":2,"t":"2024-01-02T14:31:00Z"}]"#,
                ] {
                    socket.send(Message::Text(message.into())).await.unwrap();
                }
                socket.send(Message::Close(None)).await.unwrap();
                drop(socket);
            }
        });
        let errors = Arc::new(AtomicUsize::new(0));
        let trades = Arc::new(AtomicUsize::new(0));
        let errors_for_callback = errors.clone();
        let trades_for_callback = trades.clone();
        let client = AlpacaClient::with_urls_and_policy(
            AlpacaCredentials::new("key", "secret"),
            "http://127.0.0.1:1",
            format!("ws://{address}/stream"),
            AlpacaRequestPolicy::default(),
        );
        client
            .stream(
                AlpacaStreamSubscription::trades(vec!["AAPL".into()]),
                move |event| match event {
                    AlpacaStreamEvent::Error(_) => {
                        errors_for_callback.fetch_add(1, Ordering::Relaxed) + 1 < 2
                    }
                    AlpacaStreamEvent::Trade(_) => {
                        trades_for_callback.fetch_add(1, Ordering::Relaxed);
                        true
                    }
                    _ => true,
                },
            )
            .await
            .unwrap();
        server.await.unwrap();
        assert_eq!(errors.load(Ordering::Relaxed), 2);
        assert_eq!(trades.load(Ordering::Relaxed), 2);
    }

    fn serve_http(responses: Vec<(u16, &'static str)>) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                let mut buffer = [0u8; 1024];
                loop {
                    let count = stream.read(&mut buffer).unwrap();
                    request.extend_from_slice(&buffer[..count]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") || count == 0 {
                        break;
                    }
                }
                let request = String::from_utf8_lossy(&request).to_ascii_lowercase();
                assert!(request.contains("apca-api-key-id: key"));
                assert!(request.contains("apca-api-secret-key: secret"));
                let status_text = if status == 200 {
                    "OK"
                } else {
                    "Too Many Requests"
                };
                let response = format!(
                    "HTTP/1.1 {status} {status_text}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
            }
        });
        (format!("http://{address}"), handle)
    }
}
