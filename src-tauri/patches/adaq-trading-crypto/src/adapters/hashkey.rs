//! hashkey 适配器(ADR-0017,由 generated promote 为 curated)。
//!
//! 手写完整交易 API。HashKey Exchange 签名:`HMAC-SHA256(secret,
//! method + timestamp + path + (query|body))`,头 `X-HK-APIKEY` /
//! `X-HK-TIMESTAMP` / `X-HK-SIGNATURE`(base `https://api.hashkey.com`)。
//! 端点路径以官方文档为准(此处采用 v1 命名,待核对)。

use std::collections::HashMap;

use reqwest::header::HeaderMap;
use serde_json::{Value, json};

use crate::error::{Error, ErrorKind, Result};
use crate::exchange::{Config, Exchange, Params};
use crate::httpcore::{
    HttpCore, dec, iso8601_ms, now_ms, parse_level, parse_ohlcv_standard, query_string,
};
use crate::types::{
    Balance, Balances, Market, MarketType, Markets, OHLCV, Order, OrderBook, Precision, Ticker,
    Tickers, Trade,
};

pub const ID: &str = "hashkey";
const BASE_URL: &str = "https://api.hashkey.com";
const RATE_LIMIT_MS: u64 = 50;

const COMMON_QUOTES: &[&str] = &[
    "USDT", "USDC", "FDUSD", "TUSD", "BUSD", "DAI", "EUR", "USD", "BTC", "ETH", "BNB", "PAX",
];

pub struct Hashkey {
    config: Config,
    core: HttpCore,
}

impl Hashkey {
    pub const IMPLEMENTED: &'static [&'static str] = &[
        "fetch_time",
        "fetch_markets",
        "fetch_ticker",
        "fetch_tickers",
        "fetch_order_book",
        "fetch_trades",
        "fetch_ohlcv",
        "fetch_balance",
        "create_order",
        "cancel_order",
        "fetch_order",
        "fetch_open_orders",
        "fetch_orders",
        "fetch_my_trades",
    ];

    pub fn new(config: Config) -> Result<Self> {
        let core = HttpCore::new(&config, BASE_URL, RATE_LIMIT_MS, "hashkey")?;
        Ok(Self { config, core })
    }

    fn symbol_id(&self, symbol: &str) -> String {
        if symbol.contains('/') {
            let (base, quote) = symbol.split_once('/').unwrap();
            format!("{base}{quote}")
        } else {
            symbol.to_string()
        }
    }

    fn market_symbol(&self, id: &str) -> String {
        if let Some(m) = self.core.markets_snapshot().values().find(|m| m.id == id) {
            return m.symbol.clone();
        }
        for q in COMMON_QUOTES {
            if let Some(base) = id.strip_suffix(q) {
                if !base.is_empty() {
                    return format!("{base}/{q}");
                }
            }
        }
        id.to_string()
    }

    async fn public_request(&self, path: &str, params: &Params) -> Result<Value> {
        let url = format!("{BASE_URL}{path}{}", query_string(params));
        self.core
            .request_url("GET", &url, &HeaderMap::new(), None)
            .await
    }

    async fn private_request(
        &self,
        method: &str,
        path: &str,
        params: &Params,
        body: Option<Value>,
    ) -> Result<Value> {
        let api_key = crate::signing::require_api_key(&self.config, "hashkey")?;
        let secret = crate::signing::require_secret(&self.config, "hashkey")?;
        let timestamp = now_ms().to_string();
        // GET:path 含 query;POST:path + body
        let (request_path, body_json): (String, Option<Value>) = if method == "GET" {
            (format!("{path}{}", query_string(params)), None)
        } else {
            let b = body.or_else(|| {
                if params.is_empty() {
                    None
                } else {
                    Some(json!(params))
                }
            });
            (path.to_string(), b)
        };
        let body_str = body_json
            .as_ref()
            .map(|v| v.to_string())
            .unwrap_or_default();
        let presign = format!("{method}{timestamp}{request_path}{body_str}");
        let signature = crate::signing::hmac_sha256_hex(secret, &presign);
        let mut headers = HeaderMap::new();
        crate::signing::set_header(&mut headers, "X-HK-APIKEY", api_key)?;
        crate::signing::set_header(&mut headers, "X-HK-TIMESTAMP", &timestamp)?;
        crate::signing::set_header(&mut headers, "X-HK-SIGNATURE", &signature)?;
        let url = format!("{BASE_URL}{request_path}");
        self.core
            .request_url(method, &url, &headers, body_json)
            .await
    }

    async fn fetch_markets_raw(&self) -> Result<Markets> {
        let resp = self
            .public_request("/api/v1/exchangeInfo", &Params::new())
            .await?;
        let arr = resp["symbols"].as_array().cloned().unwrap_or_default();
        let mut map = Markets::new();
        for raw in arr {
            let m = self.parse_market(&raw);
            map.insert(m.symbol.clone(), m);
        }
        Ok(map)
    }

    pub fn parse_market(&self, raw: &Value) -> Market {
        let id = raw["symbol"].as_str().unwrap_or_default().to_string();
        Market {
            id: id.clone(),
            symbol: self.market_symbol(&id),
            base: raw["baseAsset"].as_str().map(String::from),
            quote: raw["quoteAsset"].as_str().map(String::from),
            base_id: raw["baseAsset"].as_str().map(String::from),
            quote_id: raw["quoteAsset"].as_str().map(String::from),
            active: Some(
                raw["status"]
                    .as_str()
                    .map(|s| s == "TRADING" || s == "1")
                    .unwrap_or(true),
            ),
            market_type: Some(MarketType::Spot),
            spot: Some(true),
            precision: Precision {
                amount: raw["lotSize"].as_str().and_then(|s| s.parse().ok()),
                price: raw["tickSize"].as_str().and_then(|s| s.parse().ok()),
                cost: None,
            },
            limits: crate::types::Limits::default(),
            info: raw.clone(),
            ..Market::default()
        }
    }

    pub fn parse_ticker(&self, raw: &Value) -> Ticker {
        let ts = raw["timestamp"].as_i64().or_else(|| raw["time"].as_i64());
        Ticker {
            symbol: self.market_symbol(raw["symbol"].as_str().unwrap_or_default()),
            timestamp: ts,
            datetime: ts.and_then(iso8601_ms),
            high: dec(raw.get("highPrice")),
            low: dec(raw.get("lowPrice")),
            bid: dec(raw.get("bidPrice")),
            ask: dec(raw.get("askPrice")),
            open: dec(raw.get("openPrice")),
            close: dec(raw.get("lastPrice")),
            last: dec(raw.get("lastPrice")),
            base_volume: dec(raw.get("volume")),
            quote_volume: dec(raw.get("quoteVolume")),
            info: raw.clone(),
            ..Ticker::default()
        }
    }

    pub fn parse_ohlcv(&self, row: &Value) -> OHLCV {
        parse_ohlcv_standard(row)
    }

    pub fn parse_trade(&self, raw: &Value) -> Trade {
        let ts = raw["timestamp"].as_i64().or_else(|| raw["time"].as_i64());
        Trade {
            id: raw["id"]
                .as_i64()
                .or_else(|| raw["tradeId"].as_i64())
                .map(|v| v.to_string()),
            info: raw.clone(),
            timestamp: ts,
            datetime: ts.and_then(iso8601_ms),
            symbol: raw["symbol"].as_str().map(|s| self.market_symbol(s)),
            side: raw["side"].as_str().map(|s| s.to_lowercase()),
            price: dec(raw.get("price")),
            amount: dec(raw.get("quantity")).or_else(|| dec(raw.get("amount"))),
            ..Trade::default()
        }
    }

    pub fn parse_order_book(&self, raw: &Value, id: &str) -> OrderBook {
        OrderBook {
            symbol: self.market_symbol(id),
            bids: raw["bids"]
                .as_array()
                .map(|a| a.iter().map(parse_level).collect())
                .unwrap_or_default(),
            asks: raw["asks"]
                .as_array()
                .map(|a| a.iter().map(parse_level).collect())
                .unwrap_or_default(),
            nonce: raw["timestamp"].as_i64(),
            info: raw.clone(),
            ..OrderBook::default()
        }
    }

    pub fn parse_order(&self, raw: &Value) -> Order {
        let status = raw["status"].as_str().map(|s| match s {
            "NEW" | "PARTIALLY_FILLED" => "open",
            "FILLED" => "closed",
            "CANCELED" | "CANCELLED" => "canceled",
            "REJECTED" => "rejected",
            "EXPIRED" => "expired",
            other => other,
        });
        let qty = dec(raw.get("quantity")).or_else(|| dec(raw.get("origQty")));
        let filled = dec(raw.get("filledQuantity")).or_else(|| dec(raw.get("executedQty")));
        let remaining = match (qty, filled) {
            (Some(q), Some(f)) => Some(q - f),
            _ => None,
        };
        Order {
            id: raw["orderId"]
                .as_str()
                .or_else(|| raw["id"].as_str())
                .map(String::from),
            client_order_id: raw["clientOrderId"].as_str().map(String::from),
            timestamp: raw["createTime"]
                .as_i64()
                .or_else(|| raw["timestamp"].as_i64()),
            datetime: raw["createTime"]
                .as_i64()
                .or_else(|| raw["timestamp"].as_i64())
                .and_then(iso8601_ms),
            status: status.map(String::from),
            symbol: raw["symbol"].as_str().map(|s| self.market_symbol(s)),
            order_type: raw["type"].as_str().map(|s| s.to_lowercase()),
            side: raw["side"].as_str().map(|s| s.to_lowercase()),
            price: dec(raw.get("price")).or_else(|| dec(raw.get("avgPrice"))),
            average: dec(raw.get("avgPrice")),
            amount: qty,
            filled,
            remaining,
            info: raw.clone(),
            ..Order::default()
        }
    }
}

impl Exchange for Hashkey {
    fn id(&self) -> &'static str {
        ID
    }
    fn config(&self) -> &Config {
        &self.config
    }

    async fn fetch_time(&self) -> Result<i64> {
        Ok(now_ms())
    }

    async fn fetch_markets(&self) -> Result<Vec<Market>> {
        self.core.load_markets(|| self.fetch_markets_raw()).await?;
        Ok(self.core.markets_snapshot().into_values().collect())
    }

    async fn fetch_ticker(&self, symbol: &str, _params: Params) -> Result<Ticker> {
        let resp = self
            .public_request(
                &format!("/api/v1/tickers/{}", self.symbol_id(symbol)),
                &Params::new(),
            )
            .await?;
        Ok(self.parse_ticker(&resp))
    }

    async fn fetch_tickers(&self, symbols: Option<&[&str]>, _params: Params) -> Result<Tickers> {
        let resp = self
            .public_request("/api/v1/tickers", &Params::new())
            .await?;
        let arr = resp.as_array().cloned().unwrap_or_default();
        let mut out = Tickers::new();
        for raw in arr {
            let t = self.parse_ticker(&raw);
            if let Some(wanted) = symbols {
                if !wanted.contains(&t.symbol.as_str()) {
                    continue;
                }
            }
            out.insert(t.symbol.clone(), t);
        }
        Ok(out)
    }

    async fn fetch_order_book(
        &self,
        symbol: &str,
        limit: Option<i64>,
        _params: Params,
    ) -> Result<OrderBook> {
        let mut p = Params::new();
        if let Some(l) = limit {
            p.insert("limit".into(), json!(l));
        }
        let resp = self
            .public_request(&format!("/api/v1/depth/{}", self.symbol_id(symbol)), &p)
            .await?;
        Ok(self.parse_order_book(&resp, &self.symbol_id(symbol)))
    }

    async fn fetch_trades(
        &self,
        symbol: &str,
        _since: Option<i64>,
        limit: Option<i64>,
        _params: Params,
    ) -> Result<Vec<Trade>> {
        let mut p = Params::new();
        if let Some(l) = limit {
            p.insert("limit".into(), json!(l));
        }
        let resp = self
            .public_request(&format!("/api/v1/trades/{}", self.symbol_id(symbol)), &p)
            .await?;
        let arr = resp.as_array().cloned().unwrap_or_default();
        Ok(arr.iter().map(|t| self.parse_trade(t)).collect())
    }

    async fn fetch_ohlcv(
        &self,
        symbol: &str,
        timeframe: &str,
        _since: Option<i64>,
        limit: Option<i64>,
        _params: Params,
    ) -> Result<Vec<OHLCV>> {
        let mut p = Params::new();
        p.insert("period".into(), json!(timeframe));
        if let Some(l) = limit {
            p.insert("size".into(), json!(l));
        }
        let resp = self
            .public_request(&format!("/api/v1/klines/{}", self.symbol_id(symbol)), &p)
            .await?;
        let arr = resp.as_array().cloned().unwrap_or_default();
        Ok(arr.iter().map(|r| self.parse_ohlcv(r)).collect())
    }

    async fn fetch_balance(&self, _params: Params) -> Result<Balances> {
        let resp = self
            .private_request("GET", "/api/v1/account/balances", &Params::new(), None)
            .await?;
        let mut accounts = HashMap::new();
        let arr = resp
            .as_array()
            .or_else(|| resp.get("list").and_then(Value::as_array))
            .cloned()
            .unwrap_or_default();
        for b in arr {
            if let Some(asset) = b["asset"].as_str() {
                let free = dec(b.get("free")).or_else(|| dec(b.get("available")));
                let used = dec(b.get("locked")).or_else(|| dec(b.get("frozen")));
                let total = match (free, used) {
                    (Some(f), Some(u)) => Some(f + u),
                    (Some(f), None) => Some(f),
                    (None, Some(u)) => Some(u),
                    _ => None,
                };
                accounts.insert(
                    asset.to_string(),
                    Balance {
                        free,
                        used,
                        total,
                        ..Balance::default()
                    },
                );
            }
        }
        Ok(Balances {
            info: resp.clone(),
            accounts,
            ..Balances::default()
        })
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
        let mut body = serde_json::Map::new();
        body.insert("symbol".into(), json!(self.symbol_id(symbol)));
        body.insert("side".into(), json!(side.to_uppercase()));
        body.insert("type".into(), json!(order_type.to_uppercase()));
        body.insert("quantity".into(), json!(amount));
        if order_type.eq_ignore_ascii_case("limit") {
            body.insert(
                "price".into(),
                json!(price.ok_or_else(|| Error::new(
                    ErrorKind::BadRequest,
                    "limit order requires price"
                ))?),
            );
            body.insert("timeInForce".into(), json!("GTC"));
        }
        let resp = self
            .private_request(
                "POST",
                "/api/v1/orders",
                &Params::new(),
                Some(Value::Object(body)),
            )
            .await?;
        Ok(self.parse_order(&resp))
    }

    async fn cancel_order(&self, id: &str, symbol: &str, _params: Params) -> Result<Order> {
        let mut p = Params::new();
        p.insert("symbol".into(), json!(self.symbol_id(symbol)));
        let resp = self
            .private_request("DELETE", &format!("/api/v1/orders/{}", id), &p, None)
            .await?;
        let mut order = self.parse_order(&resp);
        if order.id.is_none() {
            order.id = Some(id.to_string());
        }
        Ok(order)
    }

    async fn fetch_order(&self, id: &str, symbol: &str, _params: Params) -> Result<Order> {
        let mut p = Params::new();
        p.insert("symbol".into(), json!(self.symbol_id(symbol)));
        p.insert("orderId".into(), json!(id));
        let resp = self
            .private_request("GET", "/api/v1/orders", &p, None)
            .await?;
        let arr = resp.as_array().cloned().unwrap_or_default();
        let raw = arr
            .into_iter()
            .find(|o| o["orderId"].as_str() == Some(id))
            .unwrap_or(resp);
        Ok(self.parse_order(&raw))
    }

    async fn fetch_open_orders(
        &self,
        symbol: Option<&str>,
        _since: Option<i64>,
        _limit: Option<i64>,
        _params: Params,
    ) -> Result<Vec<Order>> {
        let mut p = Params::new();
        if let Some(s) = symbol {
            p.insert("symbol".into(), json!(self.symbol_id(s)));
        }
        let resp = self
            .private_request("GET", "/api/v1/openOrders", &p, None)
            .await?;
        let arr = resp.as_array().cloned().unwrap_or_default();
        Ok(arr.iter().map(|o| self.parse_order(o)).collect())
    }

    async fn fetch_orders(
        &self,
        symbol: Option<&str>,
        _since: Option<i64>,
        _limit: Option<i64>,
        _params: Params,
    ) -> Result<Vec<Order>> {
        let mut p = Params::new();
        if let Some(s) = symbol {
            p.insert("symbol".into(), json!(self.symbol_id(s)));
        }
        let resp = self
            .private_request("GET", "/api/v1/orders", &p, None)
            .await?;
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
        if let Some(s) = symbol {
            p.insert("symbol".into(), json!(self.symbol_id(s)));
        }
        let resp = self
            .private_request("GET", "/api/v1/myTrades", &p, None)
            .await?;
        let arr = resp.as_array().cloned().unwrap_or_default();
        Ok(arr.iter().map(|t| self.parse_trade(t)).collect())
    }
}
