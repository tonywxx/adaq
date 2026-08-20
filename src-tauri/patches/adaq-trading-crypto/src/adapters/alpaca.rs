//! alpaca 适配器(ADR-0017,由 generated promote 为 curated)。
//!
//! 手写完整交易 API(仅 crypto 通道)。alpaca 双 base:
//! 交易 `https://api.alpaca.markets`(`/v2/orders`、`/v2/account`、`/v2/positions`);
//! 行情 `https://data.alpaca.markets`(`/v2/crypto/...` 与 `/v1beta3/crypto/...` L2 簿)。
//! 认证:`APCA-API-KEY-ID` + `APCA-API-SECRET-KEY` 头(无签名)。交易对符号带 `/`
//! (如 `BTC/USD`)。端点以官方文档为准(此处采用 v2 data / v2 trading,待核对)。

use std::collections::HashMap;

use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::{Value, json};

use crate::error::{Error, ErrorKind, Result};
use crate::exchange::{Config, Exchange, Params};
use crate::httpcore::{HttpCore, dec, iso8601_ms, now_ms, parse_level, query_string};
use crate::types::{
    Balance, Balances, Market, MarketType, Markets, OHLCV, Order, OrderBook, Position, Precision,
    Ticker, Tickers, Trade,
};

pub const ID: &str = "alpaca";
const TRADING_URL: &str = "https://api.alpaca.markets";
const DATA_URL: &str = "https://data.alpaca.markets";
const RATE_LIMIT_MS: u64 = 100;

pub struct Alpaca {
    config: Config,
    core: HttpCore,
}

impl Alpaca {
    /// 已实现(非 NotSupported)的 REST 方法面(契约声明,ADR-0017)。
    pub const IMPLEMENTED: &'static [&'static str] = &[
        "fetch_markets",
        "fetch_ticker",
        "fetch_tickers",
        "fetch_order_book",
        "fetch_trades",
        "fetch_ohlcv",
        "fetch_balance",
        "fetch_positions",
        "create_order",
        "cancel_order",
        "fetch_order",
        "fetch_open_orders",
        "fetch_orders",
        "fetch_my_trades",
    ];

    pub fn new(config: Config) -> Result<Self> {
        let core = HttpCore::new(&config, TRADING_URL, RATE_LIMIT_MS, "alpaca")?;
        Ok(Self { config, core })
    }

    /// 注入 alpaca 认证头(`APCA-API-KEY-ID` / `APCA-API-SECRET-KEY`)。
    fn auth_headers(&self) -> Result<HeaderMap> {
        let api_key = self
            .config
            .api_key
            .as_deref()
            .ok_or_else(|| Error::new(ErrorKind::Authentication, "alpaca api_key required"))?;
        let secret = self
            .config
            .secret
            .as_deref()
            .ok_or_else(|| Error::new(ErrorKind::Authentication, "alpaca secret required"))?;
        let mut h = HeaderMap::new();
        h.insert(
            "APCA-API-KEY-ID",
            HeaderValue::from_str(api_key)
                .map_err(|e| Error::new(ErrorKind::BadRequest, format!("apikey: {e}")))?,
        );
        h.insert(
            "APCA-API-SECRET-KEY",
            HeaderValue::from_str(secret)
                .map_err(|e| Error::new(ErrorKind::BadRequest, format!("secret: {e}")))?,
        );
        Ok(h)
    }

    async fn trading_get(&self, path: &str, params: &Params) -> Result<Value> {
        let h = self.auth_headers()?;
        let url = format!("{TRADING_URL}{path}{}", query_string(params));
        self.core.request_url("GET", &url, &h, None).await
    }

    async fn trading_post(&self, path: &str, body: Value) -> Result<Value> {
        let h = self.auth_headers()?;
        let url = format!("{TRADING_URL}{path}");
        self.core.request_url("POST", &url, &h, Some(body)).await
    }

    async fn trading_delete(&self, path: &str) -> Result<Value> {
        let h = self.auth_headers()?;
        let url = format!("{TRADING_URL}{path}");
        self.core.request_url("DELETE", &url, &h, None).await
    }

    async fn data_get(&self, path: &str, params: &Params) -> Result<Value> {
        let h = self.auth_headers()?;
        let url = format!("{DATA_URL}{path}{}", query_string(params));
        self.core.request_url("GET", &url, &h, None).await
    }

    async fn fetch_markets_raw(&self) -> Result<Markets> {
        let mut p = Params::new();
        p.insert("asset_class".into(), json!("crypto"));
        p.insert("status".into(), json!("active"));
        let resp = self.trading_get("/v2/assets", &p).await?;
        let arr = resp.as_array().cloned().unwrap_or_default();
        let mut map = Markets::new();
        for raw in arr {
            let symbol = raw["symbol"].as_str().unwrap_or_default().to_string();
            if symbol.is_empty() {
                continue;
            }
            let (base, quote) = match symbol.split_once('/') {
                Some((b, q)) => (b.to_string(), q.to_string()),
                None => (symbol.clone(), "USD".to_string()),
            };
            map.insert(
                symbol.clone(),
                Market {
                    id: symbol.clone(),
                    symbol,
                    base: Some(base),
                    quote: Some(quote),
                    base_id: raw["id"].as_str().map(String::from),
                    quote_id: Some("USD".to_string()),
                    active: raw["tradable"].as_bool().or(Some(true)),
                    market_type: Some(MarketType::Spot),
                    spot: Some(true),
                    precision: Precision {
                        amount: raw["min_order_size"].as_str().and_then(|s| s.parse().ok()),
                        price: raw["min_price_increment"]
                            .as_str()
                            .and_then(|s| s.parse().ok()),
                        cost: None,
                    },
                    taker: raw["margin_initial_ratio"]
                        .as_str()
                        .and_then(|s| s.parse().ok()),
                    info: raw,
                    ..Market::default()
                },
            );
        }
        Ok(map)
    }

    pub fn parse_ticker(&self, snap: &Value, symbol: &str) -> Ticker {
        let trade = snap.get("latestTrade");
        let quote = snap.get("latestQuote");
        let ts = trade
            .and_then(|t| t.get("t"))
            .and_then(Value::as_str)
            .and_then(parse_rfc3339_ms);
        Ticker {
            symbol: symbol.to_string(),
            timestamp: ts,
            datetime: ts.and_then(iso8601_ms),
            high: snap
                .get("dailyBar")
                .and_then(|b| b.get("h"))
                .and_then(|v| dec(Some(v))),
            low: snap
                .get("dailyBar")
                .and_then(|b| b.get("l"))
                .and_then(|v| dec(Some(v))),
            bid: quote.and_then(|q| q.get("bp")).and_then(|v| dec(Some(v))),
            ask: quote.and_then(|q| q.get("ap")).and_then(|v| dec(Some(v))),
            last: trade.and_then(|t| t.get("p")).and_then(|v| dec(Some(v))),
            close: trade.and_then(|t| t.get("p")).and_then(|v| dec(Some(v))),
            base_volume: snap
                .get("dailyBar")
                .and_then(|b| b.get("v"))
                .and_then(|v| dec(Some(v))),
            info: snap.clone(),
            ..Ticker::default()
        }
    }

    pub fn parse_trade(&self, raw: &Value, symbol: &str) -> Trade {
        let ts = raw
            .get("t")
            .and_then(Value::as_str)
            .and_then(parse_rfc3339_ms);
        Trade {
            id: raw["i"].as_u64().map(|v| v.to_string()),
            info: raw.clone(),
            timestamp: ts,
            datetime: ts.and_then(iso8601_ms),
            symbol: Some(symbol.to_string()),
            side: raw["t"].as_str().map(|s| s.to_lowercase()),
            taker_or_maker: Some("taker".to_string()),
            price: dec(raw.get("p")),
            amount: dec(raw.get("s")),
            ..Trade::default()
        }
    }

    pub fn parse_ohlcv(&self, raw: &Value) -> OHLCV {
        let ts = raw
            .get("t")
            .and_then(Value::as_str)
            .and_then(parse_rfc3339_ms);
        OHLCV {
            timestamp: ts,
            open: dec(raw.get("o")),
            high: dec(raw.get("h")),
            low: dec(raw.get("l")),
            close: dec(raw.get("c")),
            volume: dec(raw.get("v")),
        }
    }

    pub fn parse_order_book(&self, raw: &Value, symbol: &str) -> OrderBook {
        let bids = raw["bids"]
            .as_array()
            .map(|a| a.iter().map(parse_book_level).collect())
            .unwrap_or_default();
        let asks = raw["asks"]
            .as_array()
            .map(|a| a.iter().map(parse_book_level).collect())
            .unwrap_or_default();
        let ts = raw
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_rfc3339_ms);
        OrderBook {
            symbol: symbol.to_string(),
            bids,
            asks,
            timestamp: ts,
            datetime: ts.and_then(iso8601_ms),
            info: raw.clone(),
            ..OrderBook::default()
        }
    }

    pub fn parse_order(&self, raw: &Value) -> Order {
        let status = raw["status"].as_str().map(|s| match s {
            "new" | "accepted" | "pending_new" | "partially_filled" => "open",
            "filled" => "closed",
            "canceled" | "cancelled" | "expired" | "rejected" | "done_for_day" => "canceled",
            other => other,
        });
        let qty = dec(raw.get("qty")).or_else(|| dec(raw.get("filled_qty")));
        let filled = dec(raw.get("filled_qty"));
        let ts = raw
            .get("created_at")
            .and_then(Value::as_str)
            .and_then(parse_rfc3339_ms);
        Order {
            id: raw["id"].as_str().map(String::from),
            client_order_id: raw["client_order_id"].as_str().map(String::from),
            timestamp: ts,
            datetime: ts.and_then(iso8601_ms),
            status: status.map(String::from),
            symbol: raw["symbol"].as_str().map(String::from),
            order_type: raw["type"].as_str().map(|s| s.to_lowercase()),
            side: raw["side"].as_str().map(|s| s.to_lowercase()),
            price: dec(raw.get("limit_price")).or_else(|| dec(raw.get("filled_avg_price"))),
            average: dec(raw.get("filled_avg_price")),
            amount: qty,
            filled,
            remaining: match (qty, filled) {
                (Some(q), Some(f)) => Some(q - f),
                _ => None,
            },
            info: raw.clone(),
            ..Order::default()
        }
    }

    pub fn parse_position(&self, raw: &Value) -> Position {
        Position {
            symbol: raw["symbol"].as_str().map(String::from),
            id: raw["symbol"].as_str().map(String::from),
            contracts: dec(raw.get("qty")),
            entry_price: dec(raw.get("avg_entry_price")),
            notional: dec(raw.get("market_value")),
            unrealized_pnl: dec(raw.get("unrealized_pl")),
            info: raw.clone(),
            ..Position::default()
        }
    }
}

impl Exchange for Alpaca {
    fn id(&self) -> &'static str {
        ID
    }
    fn config(&self) -> &Config {
        &self.config
    }

    async fn fetch_markets(&self) -> Result<Vec<Market>> {
        self.core.load_markets(|| self.fetch_markets_raw()).await?;
        Ok(self.core.markets_snapshot().into_values().collect())
    }

    async fn fetch_ticker(&self, symbol: &str, _params: Params) -> Result<Ticker> {
        let resp = self
            .data_get(&format!("/v2/crypto/snapshots/{symbol}"), &Params::new())
            .await?;
        Ok(self.parse_ticker(&resp, symbol))
    }

    async fn fetch_tickers(&self, symbols: Option<&[&str]>, _params: Params) -> Result<Tickers> {
        let syms = match symbols {
            Some(s) => s.join(","),
            None => "*".to_string(),
        };
        let mut p = Params::new();
        p.insert("symbols".into(), json!(syms));
        let resp = self.data_get("/v2/crypto/snapshots", &p).await?;
        // 批量快照返回 { "BTC/USD": {...}, ... }
        let mut out = Tickers::new();
        if let Some(obj) = resp.as_object() {
            for (sym, snap) in obj {
                out.insert(sym.clone(), self.parse_ticker(snap, sym));
            }
        }
        Ok(out)
    }

    async fn fetch_order_book(
        &self,
        symbol: &str,
        _limit: Option<i64>,
        _params: Params,
    ) -> Result<OrderBook> {
        // L2 簿仅在 v1beta3 提供
        let resp = self
            .data_get(
                &format!("/v1beta3/crypto/{symbol}/orderbook/latest"),
                &Params::new(),
            )
            .await?;
        Ok(self.parse_order_book(&resp, symbol))
    }

    async fn fetch_trades(
        &self,
        symbol: &str,
        _since: Option<i64>,
        limit: Option<i64>,
        _params: Params,
    ) -> Result<Vec<Trade>> {
        let mut p = Params::new();
        p.insert("symbols".into(), json!(symbol));
        if let Some(l) = limit {
            p.insert("limit".into(), json!(l.min(1000)));
        }
        let resp = self.data_get("/v2/crypto/trades", &p).await?;
        // 返回 { "trades": { "BTC/USD": [ {...} ] } }
        let arr = resp
            .get("trades")
            .and_then(|t| t.get(symbol))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(arr.iter().map(|t| self.parse_trade(t, symbol)).collect())
    }

    async fn fetch_ohlcv(
        &self,
        symbol: &str,
        timeframe: &str,
        _since: Option<i64>,
        limit: Option<i64>,
        _params: Params,
    ) -> Result<Vec<OHLCV>> {
        let tf = match timeframe {
            "1m" => "1Min",
            "5m" => "5Min",
            "15m" => "15Min",
            "30m" => "30Min",
            "1h" => "1Hour",
            "4h" => "4Hour",
            "1d" => "1Day",
            other => {
                return Err(Error::new(
                    ErrorKind::BadRequest,
                    format!("unsupported timeframe {other}"),
                ));
            }
        };
        let mut p = Params::new();
        p.insert("symbols".into(), json!(symbol));
        p.insert("timeframe".into(), json!(tf));
        if let Some(l) = limit {
            p.insert("limit".into(), json!(l.min(1000)));
        }
        let resp = self.data_get("/v2/crypto/bars", &p).await?;
        let arr = resp
            .get("bars")
            .and_then(|b| b.get(symbol))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(arr.iter().map(|b| self.parse_ohlcv(b)).collect())
    }

    async fn fetch_balance(&self, _params: Params) -> Result<Balances> {
        let resp = self.trading_get("/v2/account", &Params::new()).await?;
        let mut accounts = HashMap::new();
        let currency = resp["currency"].as_str().unwrap_or("USD").to_string();
        let equity = dec(resp.get("equity"));
        let cash = dec(resp.get("cash"));
        accounts.insert(
            currency,
            Balance {
                free: cash,
                used: match (equity, cash) {
                    (Some(e), Some(c)) => Some(e - c),
                    _ => None,
                },
                total: equity,
                ..Balance::default()
            },
        );
        Ok(Balances {
            info: resp.clone(),
            accounts,
            ..Balances::default()
        })
    }

    async fn fetch_positions(
        &self,
        _symbols: Option<&[&str]>,
        _params: Params,
    ) -> Result<Vec<Position>> {
        let resp = self.trading_get("/v2/positions", &Params::new()).await?;
        let arr = resp.as_array().cloned().unwrap_or_default();
        Ok(arr.iter().map(|p| self.parse_position(p)).collect())
    }

    async fn create_order(
        &self,
        symbol: &str,
        order_type: &str,
        side: &str,
        amount: &str,
        price: Option<&str>,
        _params: Params,
    ) -> Result<Order> {
        let tif = if order_type.eq_ignore_ascii_case("market") {
            "ioc"
        } else {
            "gtc"
        };
        let mut body = json!({
            "symbol": symbol,
            "qty": amount,
            "side": side.to_lowercase(),
            "type": order_type.to_lowercase(),
            "time_in_force": tif,
        });
        if let Some(px) = price {
            body["limit_price"] = json!(px);
        }
        let resp = self.trading_post("/v2/orders", body).await?;
        Ok(self.parse_order(&resp))
    }

    async fn cancel_order(&self, id: &str, _symbol: &str, _params: Params) -> Result<Order> {
        let resp = self.trading_delete(&format!("/v2/orders/{id}")).await?;
        Ok(self.parse_order(&resp))
    }

    async fn fetch_order(&self, id: &str, _symbol: &str, _params: Params) -> Result<Order> {
        let resp = self
            .trading_get(&format!("/v2/orders/{id}"), &Params::new())
            .await?;
        Ok(self.parse_order(&resp))
    }

    async fn fetch_open_orders(
        &self,
        _symbol: Option<&str>,
        _since: Option<i64>,
        _limit: Option<i64>,
        _params: Params,
    ) -> Result<Vec<Order>> {
        let mut p = Params::new();
        p.insert("status".into(), json!("open"));
        let resp = self.trading_get("/v2/orders", &p).await?;
        let arr = resp.as_array().cloned().unwrap_or_default();
        Ok(arr.iter().map(|o| self.parse_order(o)).collect())
    }

    async fn fetch_orders(
        &self,
        _symbol: Option<&str>,
        _since: Option<i64>,
        _limit: Option<i64>,
        _params: Params,
    ) -> Result<Vec<Order>> {
        let mut p = Params::new();
        p.insert("status".into(), json!("closed"));
        p.insert("limit".into(), json!(100));
        let resp = self.trading_get("/v2/orders", &p).await?;
        let arr = resp.as_array().cloned().unwrap_or_default();
        Ok(arr.iter().map(|o| self.parse_order(o)).collect())
    }

    async fn fetch_my_trades(
        &self,
        symbol: Option<&str>,
        _since: Option<i64>,
        _limit: Option<i64>,
        _params: Params,
    ) -> Result<Vec<Trade>> {
        let mut p = Params::new();
        p.insert("activity_types".into(), json!("FILL"));
        let resp = self.trading_get("/v2/account/activities", &p).await?;
        let arr = resp.as_array().cloned().unwrap_or_default();
        Ok(arr
            .iter()
            .map(|a| Trade {
                id: a["id"].as_str().map(String::from),
                info: a.clone(),
                timestamp: a.get("transaction_time").and_then(Value::as_i64),
                datetime: a
                    .get("transaction_time")
                    .and_then(Value::as_i64)
                    .and_then(iso8601_ms),
                symbol: a["symbol"].as_str().map(String::from),
                side: a["side"].as_str().map(|s| s.to_lowercase()),
                taker_or_maker: Some("taker".to_string()),
                price: dec(a.get("price")),
                amount: dec(a.get("qty")),
                cost: dec(a.get("price")).and_then(|p| dec(a.get("qty")).map(|q| p * q)),
                ..Trade::default()
            })
            .filter(|t| match symbol {
                Some(s) => t.symbol.as_deref() == Some(s),
                None => true,
            })
            .collect())
    }
}

// ---------- 工具 ----------

/// 解析 alpaca RFC3339 时间戳(`2021-01-01T00:00:00.000Z`)→毫秒。
fn parse_rfc3339_ms(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&chrono::Utc).timestamp_millis())
}

/// 解析 L2 簿档位(`{p, s, x}` → `Level`)。
fn parse_book_level(raw: &Value) -> crate::types::Level {
    crate::types::Level {
        price: dec(raw.get("p")),
        amount: dec(raw.get("s")),
    }
}

#[allow(dead_code)]
fn _unused(_l: crate::types::Level) {
    let _ = parse_level;
    let _ = now_ms;
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_ms_parses() {
        assert_eq!(
            parse_rfc3339_ms("2021-01-01T00:00:00Z"),
            Some(1609459200000i64)
        );
    }

    #[test]
    fn parse_ohlcv_bars() {
        let ex = Alpaca::new(Config::new()).unwrap();
        let raw = json!({"t": "2021-01-01T00:00:00Z", "o": "1", "h": "2", "l": "0.5", "c": "1.5", "v": "10"});
        let c = ex.parse_ohlcv(&raw);
        assert_eq!(c.open, Some("1".parse().unwrap()));
        assert_eq!(c.close, Some("1.5".parse().unwrap()));
        assert!(c.timestamp.is_some());
    }
}
