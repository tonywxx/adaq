//! Yahoo Finance acquisition through `adaq-data-stock-us`.
//!
//! The public acquisition structs intentionally remain the existing ADAQ
//! shapes so the evidence pipeline does not need a second persistence format.

use std::{collections::HashSet, str::FromStr};

use adaq_data_stock_us::{Client, HistoryOptions, Interval};
use chrono::{DateTime, NaiveTime};
use rust_decimal::Decimal;
use serde_json::json;

use crate::{
    BarInterval, DataError, HistoricalBarRange, InstrumentStatus, MarketTrade, MarketTradeSide,
    TickerSnapshot,
    alpaca::{
        AlpacaBar, AlpacaBarsAcquisition, AlpacaCalendarAcquisition, AlpacaCapabilitySnapshot,
        AlpacaClient, AlpacaInstrument, AlpacaInstrumentMasterAcquisition, AlpacaMarketSnapshot,
        AlpacaRequestDiagnostics,
    },
    market::{InstrumentId, InstrumentSourceMapping, LocalTimeDisambiguation, Venue},
};

pub const STOCK_US_SRC: &str = "adaq-data-stock-us";
pub const STOCK_US_CONNECTOR_VERSION: &str = "adaq-data-core-stock-us-v1";

#[derive(Clone)]
pub struct StockUsClient {
    client: Client,
}

impl StockUsClient {
    pub fn new() -> Result<Self, DataError> {
        Ok(Self {
            client: Client::new().map_err(|value| upstream_error(value.to_string()))?,
        })
    }

    pub fn capability_snapshot(&self, captured_at_ms: i64) -> AlpacaCapabilitySnapshot {
        AlpacaCapabilitySnapshot::yahoo(captured_at_ms)
    }

    pub fn connector_version(&self) -> &'static str {
        STOCK_US_CONNECTOR_VERSION
    }

    pub async fn acquire_instrument_master(
        &self,
        retrieved_at_ms: i64,
    ) -> Result<AlpacaInstrumentMasterAcquisition, DataError> {
        validate_time(retrieved_at_ms)?;
        let lookup = self
            .client
            .lookup("US", 250, "equity")
            .await
            .map_err(|value| upstream_error(value.to_string()))?;
        let raw_response =
            serde_json::to_vec(&lookup).map_err(|value| upstream_error(value.to_string()))?;
        let mut instruments = Vec::new();
        let mut seen = HashSet::new();
        for row in lookup.results {
            let Some(symbol) = row.symbol.filter(|value| !value.trim().is_empty()) else {
                continue;
            };
            let symbol = symbol.trim().to_ascii_uppercase();
            let exchange = row
                .exchange
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| "yahoo".into())
                .trim()
                .to_ascii_lowercase();
            let venue = Venue::us_equity(exchange.clone())
                .map_err(|value| upstream_error(value.to_string()))?;
            let instrument = InstrumentId::new(venue, symbol.clone())
                .map_err(|value| upstream_error(value.to_string()))?;
            if !seen.insert(instrument.clone()) {
                continue;
            }
            instruments.push(AlpacaInstrument {
                mapping: InstrumentSourceMapping {
                    instrument: instrument.clone(),
                    provider: STOCK_US_SRC.into(),
                    provider_symbol: symbol.clone(),
                    connector_version: STOCK_US_CONNECTOR_VERSION.into(),
                    captured_at_ms: retrieved_at_ms,
                },
                instrument,
                provider_symbol: symbol,
                name: row.name.filter(|value| !value.trim().is_empty()),
                status: InstrumentStatus::Live,
                asset_class: "us_equity".into(),
                exchange,
                tradable: true,
                marginable: false,
                shortable: false,
                easy_to_borrow: false,
                fractionable: false,
                listing_time_ms: None,
                continuous_trading_time_ms: None,
                price_increment: None,
                quantity_increment: None,
                minimum_quantity: None,
            });
        }
        instruments.sort_by(|left, right| {
            left.instrument
                .venue
                .id
                .cmp(&right.instrument.venue.id)
                .then_with(|| left.instrument.code.cmp(&right.instrument.code))
        });
        if instruments.is_empty() {
            return Err(error("not_found", "Yahoo lookup returned no U.S. equities"));
        }
        let content_sha256 = sha256(
            &serde_json::to_vec(&instruments).map_err(|value| upstream_error(value.to_string()))?,
        );
        Ok(AlpacaInstrumentMasterAcquisition {
            provider: STOCK_US_SRC.into(),
            actual_upstream: "Yahoo Finance".into(),
            method: "GET /v1/finance/lookup".into(),
            connector_version: STOCK_US_CONNECTOR_VERSION.into(),
            request_parameters: json!({"query":"US","count":250,"type":"equity"}),
            retrieved_at_ms,
            response_sha256: sha256(&raw_response),
            content_sha256,
            raw_response,
            diagnostics: AlpacaRequestDiagnostics {
                request_count: 1,
                response_statuses: vec![200],
                notes: vec![
                    "Yahoo lookup is query-bounded and is not a complete historical exchange membership universe".into(),
                ],
                ..Default::default()
            },
            instruments,
            limitations: vec![
                "Yahoo Finance lookup returns a bounded query result; delisted and historical membership are not established".into(),
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
        {
            return Err(error(
                "invalid_request",
                "U.S. equity bar request is invalid",
            ));
        }
        if is_cancelled() {
            return Err(error("cancelled", "Yahoo acquisition was cancelled"));
        }
        let yahoo_interval = interval_for(interval)?;
        let start = DateTime::from_timestamp_millis(range.start_time_ms)
            .ok_or_else(|| error("invalid_request", "bar start time is invalid"))?;
        let end = DateTime::from_timestamp_millis(range.end_time_ms)
            .ok_or_else(|| error("invalid_request", "bar end time is invalid"))?;
        let options = HistoryOptions {
            interval: yahoo_interval,
            start: Some(start),
            end: Some(end),
            auto_adjust: false,
            back_adjust: false,
            actions: false,
            repair: false,
            ..Default::default()
        };
        let history = self
            .client
            .history(&instrument.code, &options)
            .await
            .map_err(|value| upstream_error(value.to_string()))?;
        let raw_response =
            serde_json::to_vec(&history).map_err(|value| upstream_error(value.to_string()))?;
        let mut bars = Vec::new();
        let mut invalid_bars = Vec::new();
        for raw in history.bars {
            if is_cancelled() {
                return Err(error("cancelled", "Yahoo acquisition was cancelled"));
            }
            let payload =
                serde_json::to_value(&raw).map_err(|value| upstream_error(value.to_string()))?;
            let open_time_ms = if interval == BarInterval::OneDay {
                let local = instrument
                    .venue
                    .local_time(raw.datetime.timestamp_millis())
                    .map_err(|value| upstream_error(value.to_string()))?;
                instrument
                    .venue
                    .resolve_local_time(
                        local
                            .date()
                            .and_time(NaiveTime::from_hms_opt(9, 30, 0).expect("valid session")),
                        LocalTimeDisambiguation::Reject,
                    )
                    .map_err(|value| upstream_error(value.to_string()))?
            } else {
                raw.datetime.timestamp_millis()
            };
            if open_time_ms < range.start_time_ms || open_time_ms >= range.end_time_ms {
                continue;
            }
            let open = decimal(raw.open, "open")?;
            let high = decimal(raw.high, "high")?;
            let low = decimal(raw.low, "low")?;
            let close = decimal(raw.close, "close")?;
            let volume = decimal(raw.volume, "volume")?;
            let quote_volume = match (close.as_deref(), volume.as_deref()) {
                (Some(close), Some(volume)) => Some(
                    (Decimal::from_str(close)
                        .map_err(|value| upstream_error(value.to_string()))?
                        * Decimal::from_str(volume)
                            .map_err(|value| upstream_error(value.to_string()))?)
                    .to_string(),
                ),
                _ => None,
            };
            let bar = AlpacaBar {
                instrument: instrument.clone(),
                provider_symbol: instrument.code.clone(),
                interval,
                open_time_ms,
                open,
                high,
                low,
                close,
                base_volume: volume,
                quote_volume,
                raw_payload: payload,
            };
            if bar.open.is_some()
                && bar.high.is_some()
                && bar.low.is_some()
                && bar.close.is_some()
                && bar.base_volume.is_some()
            {
                bars.push(bar);
            } else {
                invalid_bars.push(bar);
            }
        }
        let content_sha256 = sha256(
            &serde_json::to_vec(&(&bars, &invalid_bars))
                .map_err(|value| upstream_error(value.to_string()))?,
        );
        Ok(AlpacaBarsAcquisition {
            provider: STOCK_US_SRC.into(),
            actual_upstream: "Yahoo Finance".into(),
            method: "GET /v8/finance/chart/{symbol}".into(),
            connector_version: STOCK_US_CONNECTOR_VERSION.into(),
            request_parameters: json!({
                "symbol": instrument.code,
                "interval": yahoo_interval.as_str(),
                "startTimeMs": range.start_time_ms,
                "endTimeMs": range.end_time_ms,
                "autoAdjust": false,
            }),
            retrieved_at_ms,
            response_sha256s: vec![sha256(&raw_response)],
            content_sha256,
            raw_responses: vec![raw_response],
            diagnostics: AlpacaRequestDiagnostics {
                request_count: 1,
                response_statuses: vec![200],
                notes: vec![
                    "Yahoo history is unadjusted; quote volume is derived as close multiplied by base volume".into(),
                ],
                ..Default::default()
            },
            bars,
            invalid_bars,
            limitations: vec![
                "Yahoo chart history does not provide consolidated quote volume; ADAQ stores a derived close-times-volume value".into(),
            ],
        })
    }

    pub async fn acquire_calendar(
        &self,
        venue: Venue,
        range: HistoricalBarRange,
        retrieved_at_ms: i64,
    ) -> Result<AlpacaCalendarAcquisition, DataError> {
        let legacy = AlpacaClient::with_key_pair("unused", "unused");
        let mut acquisition = legacy
            .acquire_calendar(venue, range, retrieved_at_ms)
            .await?;
        acquisition.provider = STOCK_US_SRC.into();
        acquisition.actual_upstream = "ADAQ America/New_York session rules".into();
        acquisition.connector_version = STOCK_US_CONNECTOR_VERSION.into();
        acquisition.method = "versioned America/New_York regular-session calendar".into();
        Ok(acquisition)
    }

    pub async fn get_snapshot(
        &self,
        symbol: &str,
        retrieved_at_ms: i64,
    ) -> Result<AlpacaMarketSnapshot, DataError> {
        validate_time(retrieved_at_ms)?;
        let history = self
            .client
            .history(
                symbol,
                &HistoryOptions {
                    period: "5d".into(),
                    interval: Interval::Day1,
                    auto_adjust: false,
                    ..Default::default()
                },
            )
            .await
            .map_err(|value| upstream_error(value.to_string()))?;
        let raw_response =
            serde_json::to_vec(&history).map_err(|value| upstream_error(value.to_string()))?;
        let raw = history
            .bars
            .last()
            .ok_or_else(|| error("not_found", "Yahoo history returned no bars"))?;
        let last = required_decimal(raw.close, "close")?;
        let open = required_decimal(raw.open, "open")?;
        let high = required_decimal(raw.high, "high")?;
        let low = required_decimal(raw.low, "low")?;
        let volume = required_decimal(raw.volume, "volume")?;
        let quote_volume = &last * &volume;
        let timestamp_ms = raw.datetime.timestamp_millis();
        let ticker = TickerSnapshot {
            src: STOCK_US_SRC.into(),
            code: symbol.into(),
            last: last.clone(),
            last_quantity: Decimal::ZERO,
            ask_price: None,
            ask_quantity: None,
            bid_price: None,
            bid_quantity: None,
            open_24h: open,
            high_24h: high,
            low_24h: low,
            base_volume_24h: volume,
            quote_volume_24h: quote_volume,
            timestamp_ms,
        };
        let trade = MarketTrade {
            src: STOCK_US_SRC.into(),
            code: symbol.into(),
            trade_id: timestamp_ms.to_string(),
            price: last,
            quantity: Decimal::ZERO,
            side: MarketTradeSide::Unknown,
            timestamp_ms,
        };
        let quote = crate::alpaca::AlpacaQuote {
            src: STOCK_US_SRC.into(),
            code: symbol.into(),
            ask_price: None,
            ask_quantity: None,
            bid_price: None,
            bid_quantity: None,
            timestamp_ms,
            ask_exchange: None,
            bid_exchange: None,
            feed: "yahoo-history".into(),
        };
        Ok(AlpacaMarketSnapshot {
            ticker,
            trade,
            quote,
            feed: "yahoo-history".into(),
            retrieved_at_ms,
            response_sha256: sha256(&raw_response),
            raw_payload: serde_json::to_value(history)
                .map_err(|value| upstream_error(value.to_string()))?,
        })
    }
}

fn interval_for(interval: BarInterval) -> Result<Interval, DataError> {
    match interval {
        BarInterval::OneMinute => Ok(Interval::Min1),
        BarInterval::FiveMinutes => Ok(Interval::Min5),
        BarInterval::FifteenMinutes => Ok(Interval::Min15),
        BarInterval::ThirtyMinutes => Ok(Interval::Min30),
        BarInterval::OneHour => Ok(Interval::Hour1),
        BarInterval::OneDay => Ok(Interval::Day1),
        _ => Err(error(
            "unsupported_interval",
            format!("Yahoo history does not support ADAQ interval {interval:?}"),
        )),
    }
}

fn decimal(value: Option<f64>, field: &str) -> Result<Option<String>, DataError> {
    value
        .filter(|value| value.is_finite())
        .map(|value| {
            Decimal::from_f64_retain(value)
                .map(|value| value.to_string())
                .ok_or_else(|| error("invalid_decimal", format!("Yahoo {field} is invalid")))
        })
        .transpose()
}

fn required_decimal(value: Option<f64>, field: &str) -> Result<Decimal, DataError> {
    decimal(value, field)?
        .ok_or_else(|| error("invalid_decimal", format!("Yahoo {field} is missing")))
        .and_then(|value| {
            Decimal::from_str(&value).map_err(|value| upstream_error(value.to_string()))
        })
}

fn validate_time(value: i64) -> Result<(), DataError> {
    if value < 0 {
        Err(error(
            "invalid_request",
            "retrieval time must be non-negative",
        ))
    } else {
        Ok(())
    }
}

fn error(code: impl Into<String>, message: impl Into<String>) -> DataError {
    DataError::new(STOCK_US_SRC, code, message)
}

fn upstream_error(message: impl Into<String>) -> DataError {
    error("upstream", message)
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
