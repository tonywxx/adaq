use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use tokio_tungstenite::{connect_async, tungstenite::Message};

const OKX_SRC: &str = "okx";
const OKX_BASE_URL: &str = "https://www.okx.com";
const OKX_WS_URL: &str = "wss://ws.okx.com:8443/ws/v5/public";
const OKX_BUSINESS_WS_URL: &str = "wss://ws.okx.com:8443/ws/v5/business";
const OKX_WS_HEARTBEAT_SECONDS: u64 = 25;
const OKX_WS_MAX_RETRY_SECONDS: u64 = 15;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BarSeries {
    pub src: String,
    pub code: String,
    pub interval: BarInterval,
    pub bars: Vec<OhlcvBar>,
    pub gaps: Vec<BarGap>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BarSnapshot {
    pub bar: OhlcvBar,
    pub closed: bool,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum InstrumentStatus {
    Live,
    Suspended,
    PreOpen,
    Test,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("{message}")]
pub struct DataError {
    pub src: String,
    pub code: String,
    pub message: String,
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
        }
    }

    fn okx(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(OKX_SRC, code, message)
    }
}

#[derive(Clone)]
pub struct OkxClient {
    http: reqwest::Client,
    base_url: String,
    ws_url: String,
    business_ws_url: String,
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

    fn new_with_all_urls(
        base_url: impl Into<String>,
        ws_url: impl Into<String>,
        business_ws_url: impl Into<String>,
    ) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            ws_url: ws_url.into(),
            business_ws_url: business_ws_url.into(),
        }
    }

    pub async fn get_ticker(&self, code: &str) -> Result<TickerSnapshot, DataError> {
        validate_ticker_code(code)?;
        let response = self
            .http
            .get(format!("{}/api/v5/market/ticker", self.base_url))
            .query(&[("instId", code)])
            .send()
            .await
            .map_err(|error| DataError::okx("transport", error.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            return Err(DataError::okx(
                "http_status",
                format!("OKX returned HTTP {status}"),
            ));
        }

        let payload: OkxEnvelope<Vec<OkxTicker>> = response
            .json()
            .await
            .map_err(|error| DataError::okx("invalid_response", error.to_string()))?;
        if payload.code != "0" {
            return Err(DataError::okx(payload.code, payload.msg));
        }
        let ticker = payload
            .data
            .into_iter()
            .next()
            .ok_or_else(|| DataError::okx("invalid_response", "OKX ticker data is empty"))?;
        normalize_okx_ticker(ticker, code)
    }

    pub async fn stream_ticker<F>(&self, code: &str, mut on_event: F) -> Result<(), DataError>
    where
        F: FnMut(TickerStreamEvent) -> bool,
    {
        validate_ticker_code(code)?;
        let mut retry_seconds = 1;

        loop {
            let mut received_snapshot = false;
            let result = self
                .stream_ticker_once(code, |event| {
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

    async fn stream_ticker_once<F>(&self, code: &str, mut on_event: F) -> Result<(), DataError>
    where
        F: FnMut(TickerStreamEvent) -> bool,
    {
        let (mut socket, _) = connect_async(&self.ws_url)
            .await
            .map_err(|error| DataError::okx("transport", error.to_string()))?;
        socket
            .send(Message::Text(
                serde_json::json!({
                    "op": "subscribe",
                    "args": [{ "channel": "tickers", "instId": code }]
                })
                .to_string()
                .into(),
            ))
            .await
            .map_err(|error| DataError::okx("transport", error.to_string()))?;

        let heartbeat = Duration::from_secs(OKX_WS_HEARTBEAT_SECONDS);
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
                        "OKX ticker WebSocket closed",
                    ));
                }
                Err(_) if awaiting_pong => {
                    return Err(DataError::okx(
                        "heartbeat_timeout",
                        "OKX ticker WebSocket did not answer ping",
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
                Message::Text(text) if text == "pong" => awaiting_pong = false,
                Message::Text(text) => {
                    awaiting_pong = false;
                    if let Some(snapshot) = parse_okx_ticker_message(&text, code)? {
                        if !announced_connected {
                            if !on_event(TickerStreamEvent::Connected) {
                                return Ok(());
                            }
                            announced_connected = true;
                        }
                        if !on_event(TickerStreamEvent::Snapshot(snapshot)) {
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
                        "OKX ticker WebSocket closed",
                    ));
                }
                _ => {}
            }
        }
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
        validate_ticker_code(code)?;
        let mut retry_seconds = 1;

        loop {
            let mut received_snapshot = false;
            let result = self
                .stream_bar_once(code, interval, |event| {
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

    async fn stream_bar_once<F>(
        &self,
        code: &str,
        interval: BarInterval,
        mut on_event: F,
    ) -> Result<(), DataError>
    where
        F: FnMut(BarStreamEvent) -> bool,
    {
        let channel = format!("candle{}", interval.okx_bar());
        let (mut socket, _) = connect_async(&self.business_ws_url)
            .await
            .map_err(|error| DataError::okx("transport", error.to_string()))?;
        socket
            .send(Message::Text(
                serde_json::json!({
                    "op": "subscribe",
                    "args": [{ "channel": channel, "instId": code }]
                })
                .to_string()
                .into(),
            ))
            .await
            .map_err(|error| DataError::okx("transport", error.to_string()))?;

        let heartbeat = Duration::from_secs(OKX_WS_HEARTBEAT_SECONDS);
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
                Message::Text(text) if text == "pong" => awaiting_pong = false,
                Message::Text(text) => {
                    awaiting_pong = false;
                    let snapshots = parse_okx_bar_message(&text, code, &channel)?;
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
        if code.trim().is_empty() || range.start_time_ms >= range.end_time_ms {
            return Err(DataError::okx(
                "invalid_request",
                "code must be non-empty and bar range must be increasing",
            ));
        }

        let mut cursor = range.end_time_ms;
        let mut bars = Vec::new();
        loop {
            let page = self
                .fetch_bar_page(code, interval, Some(cursor), 100)
                .await?;
            let Some(oldest_open_time_ms) = page.oldest_open_time_ms else {
                break;
            };
            bars.extend(page.bars);
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

        let bars = bars
            .into_iter()
            .filter(|bar| {
                bar.open_time_ms >= range.start_time_ms && bar.open_time_ms < range.end_time_ms
            })
            .collect();

        build_bar_series(code, interval, bars)
    }

    async fn fetch_bar_page(
        &self,
        code: &str,
        interval: BarInterval,
        after_time_ms: Option<i64>,
        limit: u16,
    ) -> Result<OkxBarPage, DataError> {
        let mut query = vec![
            ("instId", code.to_owned()),
            ("bar", interval.okx_bar().to_owned()),
        ];
        if let Some(after_time_ms) = after_time_ms {
            query.push(("after", after_time_ms.to_string()));
        }
        query.push(("limit", limit.to_string()));

        let response = self
            .http
            .get(format!("{}/api/v5/market/history-candles", self.base_url))
            .query(&query)
            .send()
            .await
            .map_err(|error| DataError::okx("transport", error.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            return Err(DataError::okx(
                "http_status",
                format!("OKX returned HTTP {status}"),
            ));
        }

        let payload: OkxEnvelope<Vec<Vec<String>>> = response
            .json()
            .await
            .map_err(|error| DataError::okx("invalid_response", error.to_string()))?;
        if payload.code != "0" {
            return Err(DataError::okx(payload.code, payload.msg));
        }

        let row_count = payload.data.len();
        let mut oldest_open_time_ms = None;
        let mut bars = Vec::with_capacity(row_count);
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
            if let Some(bar) = parse_okx_bar(values)? {
                bars.push(bar);
            }
        }
        Ok(OkxBarPage {
            bars,
            row_count,
            oldest_open_time_ms,
        })
    }

    pub async fn list_spot_instruments(&self) -> Result<Vec<SpotInstrument>, DataError> {
        let response = self
            .http
            .get(format!("{}/api/v5/public/instruments", self.base_url))
            .query(&[("instType", "SPOT")])
            .send()
            .await
            .map_err(|error| DataError::okx("transport", error.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            return Err(DataError::okx(
                "http_status",
                format!("OKX returned HTTP {status}"),
            ));
        }

        let payload: OkxEnvelope<Vec<OkxSpotInstrument>> = response
            .json()
            .await
            .map_err(|error| DataError::okx("invalid_response", error.to_string()))?;
        if payload.code != "0" {
            return Err(DataError::okx(payload.code, payload.msg));
        }

        let mut instruments = payload
            .data
            .into_iter()
            .map(SpotInstrument::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        instruments.sort_unstable_by(|left, right| left.code.cmp(&right.code));
        Ok(instruments)
    }
}

struct OkxBarPage {
    bars: Vec<OhlcvBar>,
    row_count: usize,
    oldest_open_time_ms: Option<i64>,
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

fn next_bar_open_time_ms(open_time_ms: i64, interval: BarInterval) -> Result<i64, DataError> {
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

#[derive(Deserialize)]
struct OkxWsEnvelope {
    event: Option<String>,
    code: Option<String>,
    msg: Option<String>,
    #[serde(default)]
    data: Vec<OkxTicker>,
}

#[derive(Deserialize)]
struct OkxBarWsEnvelope {
    event: Option<String>,
    code: Option<String>,
    msg: Option<String>,
    arg: Option<OkxWsArg>,
    #[serde(default)]
    data: Vec<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OkxWsArg {
    channel: String,
    inst_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OkxTicker {
    inst_type: String,
    inst_id: String,
    last: String,
    last_sz: String,
    ask_px: String,
    ask_sz: String,
    bid_px: String,
    bid_sz: String,
    open_24h: String,
    high_24h: String,
    low_24h: String,
    vol_ccy_24h: String,
    vol_24h: String,
    ts: String,
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

fn parse_okx_ticker_message(
    message: &str,
    expected_code: &str,
) -> Result<Option<TickerSnapshot>, DataError> {
    let payload: OkxWsEnvelope = serde_json::from_str(message)
        .map_err(|error| DataError::okx("invalid_response", error.to_string()))?;
    if payload.event.as_deref() == Some("error") {
        return Err(DataError::okx(
            payload.code.unwrap_or_else(|| "websocket_error".to_owned()),
            payload
                .msg
                .unwrap_or_else(|| "OKX rejected ticker subscription".to_owned()),
        ));
    }
    payload
        .data
        .into_iter()
        .next()
        .map(|ticker| normalize_okx_ticker(ticker, expected_code))
        .transpose()
}

fn normalize_okx_ticker(
    value: OkxTicker,
    expected_code: &str,
) -> Result<TickerSnapshot, DataError> {
    if value.inst_type != "SPOT" {
        return Err(DataError::okx(
            "unsupported_instrument",
            format!("expected OKX SPOT ticker, received {}", value.inst_type),
        ));
    }
    if value.inst_id != expected_code {
        return Err(DataError::okx(
            "invalid_response",
            format!(
                "expected OKX ticker for {expected_code}, received {}",
                value.inst_id
            ),
        ));
    }

    let decimal = |raw: &str, field: &str| {
        Decimal::from_str_exact(raw).map_err(|error| {
            DataError::okx("invalid_decimal", format!("invalid OKX {field}: {error}"))
        })
    };
    let optional_decimal = |raw: &str, field: &str| {
        if raw.is_empty() {
            Ok(None)
        } else {
            decimal(raw, field).map(Some)
        }
    };

    Ok(TickerSnapshot {
        src: OKX_SRC.to_owned(),
        code: value.inst_id,
        last: decimal(&value.last, "last price")?,
        last_quantity: decimal(&value.last_sz, "last quantity")?,
        ask_price: optional_decimal(&value.ask_px, "ask price")?,
        ask_quantity: optional_decimal(&value.ask_sz, "ask quantity")?,
        bid_price: optional_decimal(&value.bid_px, "bid price")?,
        bid_quantity: optional_decimal(&value.bid_sz, "bid quantity")?,
        open_24h: decimal(&value.open_24h, "24-hour open")?,
        high_24h: decimal(&value.high_24h, "24-hour high")?,
        low_24h: decimal(&value.low_24h, "24-hour low")?,
        base_volume_24h: decimal(&value.vol_24h, "24-hour base volume")?,
        quote_volume_24h: decimal(&value.vol_ccy_24h, "24-hour quote volume")?,
        timestamp_ms: value.ts.parse().map_err(|error| {
            DataError::okx(
                "invalid_response",
                format!("invalid OKX ticker timestamp: {error}"),
            )
        })?,
    })
}

fn parse_okx_bar_message(
    message: &str,
    expected_code: &str,
    expected_channel: &str,
) -> Result<Vec<BarSnapshot>, DataError> {
    let payload: OkxBarWsEnvelope = serde_json::from_str(message)
        .map_err(|error| DataError::okx("invalid_response", error.to_string()))?;
    if payload.event.as_deref() == Some("error") {
        return Err(DataError::okx(
            payload.code.unwrap_or_else(|| "websocket_error".to_owned()),
            payload
                .msg
                .unwrap_or_else(|| "OKX rejected bar subscription".to_owned()),
        ));
    }
    if let Some(arg) = payload.arg
        && (arg.inst_id != expected_code || arg.channel != expected_channel)
    {
        return Err(DataError::okx(
            "invalid_response",
            format!(
                "expected OKX {expected_channel} bars for {expected_code}, received {} for {}",
                arg.channel, arg.inst_id
            ),
        ));
    }
    payload
        .data
        .into_iter()
        .map(parse_okx_bar_snapshot)
        .collect()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OkxSpotInstrument {
    inst_id: String,
    base_ccy: String,
    quote_ccy: String,
    state: String,
    list_time: String,
    cont_td_sw_time: String,
    tick_sz: String,
    lot_sz: String,
    min_sz: String,
}

impl TryFrom<OkxSpotInstrument> for SpotInstrument {
    type Error = DataError;

    fn try_from(value: OkxSpotInstrument) -> Result<Self, Self::Error> {
        let decimal = |raw: &str, field: &str| {
            Decimal::from_str_exact(raw).map_err(|error| {
                DataError::okx("invalid_decimal", format!("invalid OKX {field}: {error}"))
            })
        };
        let timestamp = |raw: &str, field: &str| {
            if raw.is_empty() {
                Ok(None)
            } else {
                raw.parse().map(Some).map_err(|error| {
                    DataError::okx("invalid_response", format!("invalid OKX {field}: {error}"))
                })
            }
        };

        Ok(Self {
            src: OKX_SRC.to_owned(),
            code: value.inst_id,
            base_asset: value.base_ccy,
            quote_asset: value.quote_ccy,
            status: match value.state.as_str() {
                "live" => InstrumentStatus::Live,
                "suspend" => InstrumentStatus::Suspended,
                "preopen" => InstrumentStatus::PreOpen,
                "test" => InstrumentStatus::Test,
                _ => InstrumentStatus::Unknown,
            },
            listing_time_ms: timestamp(&value.list_time, "listing time")?,
            continuous_trading_time_ms: timestamp(
                &value.cont_td_sw_time,
                "continuous trading time",
            )?,
            price_increment: decimal(&value.tick_sz, "price increment")?,
            quantity_increment: decimal(&value.lot_sz, "quantity increment")?,
            minimum_quantity: decimal(&value.min_sz, "minimum quantity")?,
        })
    }
}

fn parse_okx_bar(values: Vec<String>) -> Result<Option<OhlcvBar>, DataError> {
    let snapshot = parse_okx_bar_snapshot(values)?;
    Ok(snapshot.closed.then_some(snapshot.bar))
}

fn parse_okx_bar_snapshot(values: Vec<String>) -> Result<BarSnapshot, DataError> {
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

    Ok(BarSnapshot {
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

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::mpsc,
        thread,
    };

    use futures_util::{SinkExt, StreamExt};
    use tokio::sync::oneshot;
    use tokio_tungstenite::{accept_async, tungstenite::Message};

    use super::{
        BarInterval, BarStreamEvent, HistoricalBarRange, OkxClient, TickerStreamEvent,
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

    #[tokio::test]
    async fn okx_client_subscribes_and_streams_normalized_spot_ticker() {
        let (ws_url, subscription) = serve_ticker_ws().await;
        let client = OkxClient::new_with_urls("http://unused", ws_url);
        let mut snapshots = Vec::new();

        client
            .stream_ticker_once("BTC-USDT", |event| match event {
                TickerStreamEvent::Snapshot(snapshot) => {
                    snapshots.push(snapshot);
                    false
                }
                _ => true,
            })
            .await
            .unwrap();

        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&subscription.await.unwrap()).unwrap(),
            serde_json::json!({
                "op": "subscribe",
                "args": [{ "channel": "tickers", "instId": "BTC-USDT" }]
            })
        );
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].last.to_string(), "67433.25");
        assert_eq!(snapshots[0].ask_price.unwrap().to_string(), "67433.30");
    }

    #[tokio::test]
    async fn okx_client_subscribes_and_streams_open_bars() {
        let (ws_url, subscription) = serve_bar_ws().await;
        let client = OkxClient::new_with_all_urls("http://unused", "ws://unused", ws_url);
        let mut snapshots = Vec::new();

        client
            .stream_bar_once(
                "BTC-USDT",
                BarInterval::FifteenMinutes,
                |event| match event {
                    BarStreamEvent::Snapshot(snapshot) => {
                        snapshots.push(snapshot);
                        false
                    }
                    _ => true,
                },
            )
            .await
            .unwrap();

        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&subscription.await.unwrap()).unwrap(),
            serde_json::json!({
                "op": "subscribe",
                "args": [{ "channel": "candle15m", "instId": "BTC-USDT" }]
            })
        );
        assert_eq!(snapshots.len(), 1);
        assert!(!snapshots[0].closed);
        assert_eq!(snapshots[0].bar.close.to_string(), "67433.25");
        assert_eq!(snapshots[0].bar.quote_volume.to_string(), "123456.78");
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
        const BASE: i64 = 1_700_000_000_000;
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

    async fn serve_ticker_ws() -> (String, oneshot::Receiver<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, receiver) = oneshot::channel();

        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let subscription = socket.next().await.unwrap().unwrap().into_text().unwrap();
            sender.send(subscription.to_string()).unwrap();
            socket
                .send(Message::Text(
                    r#"{
                        "arg": {"channel": "tickers", "instId": "BTC-USDT"},
                        "data": [{
                            "instType": "SPOT",
                            "instId": "BTC-USDT",
                            "last": "67433.25",
                            "lastSz": "0.001",
                            "askPx": "67433.30",
                            "askSz": "1.2",
                            "bidPx": "67433.20",
                            "bidSz": "0.9",
                            "open24h": "66100",
                            "high24h": "68000",
                            "low24h": "65500",
                            "volCcy24h": "123456789.12",
                            "vol24h": "1842.5",
                            "ts": "1720000000456"
                        }]
                    }"#
                    .into(),
                ))
                .await
                .unwrap();
        });

        (format!("ws://{address}"), receiver)
    }

    async fn serve_bar_ws() -> (String, oneshot::Receiver<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, receiver) = oneshot::channel();

        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let subscription = socket.next().await.unwrap().unwrap().into_text().unwrap();
            sender.send(subscription.to_string()).unwrap();
            socket
                .send(Message::Text(
                    r#"{
                        "arg": {"channel": "candle15m", "instId": "BTC-USDT"},
                        "data": [[
                            "1720000000000",
                            "67000.10",
                            "67500.20",
                            "66900.30",
                            "67433.25",
                            "1.25",
                            "84000",
                            "123456.78",
                            "0"
                        ]]
                    }"#
                    .into(),
                ))
                .await
                .unwrap();
        });

        (format!("ws://{address}"), receiver)
    }
}
