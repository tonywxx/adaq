//! gemini 适配器(ADR-0017,由 generated promote 为 curated)。
//!
//! 手写完整交易 API。Gemini 签名:`HMAC-SHA384(secret, base64(payload))`,
//! 头 `X-GEMINI-APIKEY` / `X-GEMINI-PAYLOAD`(base64 JSON)/ `X-GEMINI-SIGNATURE`
//! (hex)。公开面 base `https://api.gemini.com`,交易对无 `/`(如 `BTCUSD`,
//! 统一 symbol 写作 `BTC/USD`)。端点与签名以官方文档为准。

use std::collections::HashMap;

use base64::Engine;
use reqwest::header::HeaderMap;
use serde_json::{Value, json};

use crate::error::{Error, ErrorKind, Result};
use crate::exchange::{Config, Exchange, Params};
use crate::httpcore::{HttpCore, apply_client_limit, dec, iso8601_ms, now_ms, parse_level, query_string};
use crate::types::{
    Balance, Balances, Market, MarketType, Markets, OHLCV, Order, OrderBook, Precision, Ticker,
    Tickers, Trade,
};

pub const ID: &str = "gemini";
const BASE_URL: &str = "https://api.gemini.com";
const RATE_LIMIT_MS: u64 = 100;

/// 常见计价币(Gemini 后缀,用于推导统一 symbol)。
const COMMON_QUOTES: &[&str] = &[
    "USD", "BTC", "ETH", "GBP", "SGD", "EUR", "CAD", "AUD", "NZD", "HKD", "PAX", "DAI",
];

pub struct Gemini {
    config: Config,
    core: HttpCore,
}

impl Gemini {
    /// 已实现(非 NotSupported)的 REST 方法面(契约声明,ADR-0017)。
    pub const IMPLEMENTED: &'static [&'static str] = &[
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
        let core = HttpCore::new(&config, BASE_URL, RATE_LIMIT_MS, "gemini")?;
        Ok(Self { config, core })
    }

    /// 统一 symbol → gemini id(`BTC/USD` → `BTCUSD`)。
    fn symbol_id(&self, symbol: &str) -> String {
        symbol.replace('/', "")
    }

    /// gemini id → 统一 symbol(`BTCUSD` → `BTC/USD`),否则原样返回。
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

    /// 私有签名请求(gemini:HMAC-SHA384 over base64(payload),`X-GEMINI-*` 头)。
    async fn private_request(&self, method: &str, path: &str, payload: Value) -> Result<Value> {
        let api_key = crate::signing::require_api_key(&self.config, "gemini")?;
        let secret = crate::signing::require_secret(&self.config, "gemini")?;
        let mut body = payload;
        if !body.is_object() {
            body = json!({});
        }
        // nonce 必须单调递增,用毫秒时间戳
        let mut obj = body.as_object().cloned().unwrap_or_default();
        obj.insert("request".to_string(), json!(path));
        obj.insert("nonce".to_string(), json!(now_ms()));
        body = Value::Object(obj);
        let b64 = base64::engine::general_purpose::STANDARD.encode(body.to_string());
        let signature = crate::signing::hmac_sha384_hex(secret, &b64);
        let mut headers = HeaderMap::new();
        crate::signing::set_header(&mut headers, "X-GEMINI-APIKEY", api_key)?;
        crate::signing::set_header(&mut headers, "X-GEMINI-PAYLOAD", &b64)?;
        crate::signing::set_header(&mut headers, "X-GEMINI-SIGNATURE", &signature)?;
        let url = format!("{BASE_URL}{path}");
        self.core.request_url(method, &url, &headers, None).await
    }

    async fn fetch_markets_raw(&self) -> Result<Markets> {
        let symbols = self
            .public_request("/v1/symbols", &Params::new())
            .await?
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mut map = Markets::new();
        for raw in symbols {
            let id = raw.as_str().unwrap_or_default().to_string();
            if id.is_empty() {
                continue;
            }
            // 逐个拉取详情(精度/最小下单量),失败则退化为仅 symbol
            let mut market = Market {
                id: id.clone(),
                symbol: self.market_symbol(&id),
                active: Some(true),
                market_type: Some(MarketType::Spot),
                spot: Some(true),
                ..Market::default()
            };
            if let Ok(detail) = self
                .public_request(&format!("/v1/symbols/details/{id}"), &Params::new())
                .await
            {
                market.base = detail["base_currency"].as_str().map(String::from);
                market.quote = detail["quote_currency"].as_str().map(String::from);
                market.base_id = market.base.clone();
                market.quote_id = market.quote.clone();
                market.precision = Precision {
                    price: detail["tick_size"].as_str().and_then(|s| s.parse().ok()),
                    amount: detail["min_order_size"]
                        .as_str()
                        .and_then(|s| s.parse().ok()),
                    cost: None,
                };
                market.taker = detail["taker_fee"].as_str().and_then(|s| s.parse().ok());
                market.maker = detail["maker_fee"].as_str().and_then(|s| s.parse().ok());
                market.info = detail;
            }
            map.insert(market.symbol.clone(), market);
        }
        Ok(map)
    }

    pub fn parse_ticker(&self, raw: &Value) -> Ticker {
        let ts = raw["volume"].get("timestamp").and_then(Value::as_i64);
        Ticker {
            symbol: self.market_symbol(raw["symbol"].as_str().unwrap_or_default()),
            timestamp: ts,
            datetime: ts.and_then(iso8601_ms),
            high: dec(raw.get("high")),
            low: dec(raw.get("low")),
            bid: dec(raw.get("bid")),
            ask: dec(raw.get("ask")),
            open: dec(raw.get("open")),
            close: dec(raw.get("close")),
            last: dec(raw.get("last")),
            base_volume: dec(raw.get("volume")).or_else(|| {
                raw.get("volume")
                    .and_then(|v| v.get("USD"))
                    .and_then(Value::as_str)
                    .and_then(|s| s.parse().ok())
            }),
            info: raw.clone(),
            ..Ticker::default()
        }
    }

    pub fn parse_trade(&self, raw: &Value) -> Trade {
        let timestamp = raw["timestampms"].as_i64();
        let side = raw["type"].as_str().map(|s| s.to_lowercase());
        Trade {
            id: raw["tid"].as_i64().map(|v| v.to_string()),
            info: raw.clone(),
            timestamp,
            datetime: timestamp.and_then(iso8601_ms),
            symbol: Some(self.market_symbol(raw["symbol"].as_str().unwrap_or_default())),
            side,
            taker_or_maker: Some("taker".to_string()),
            price: dec(raw.get("price")),
            amount: dec(raw.get("amount")),
            ..Trade::default()
        }
    }

    pub fn parse_ohlcv(&self, row: &Value) -> OHLCV {
        // Gemini v2 candles: [timestamp, low, high, open, close, volume, quoteVolume]
        let arr = row.as_array();
        OHLCV {
            timestamp: arr.and_then(|a| a.first()).and_then(Value::as_i64),
            open: arr.and_then(|a| a.get(3)).and_then(|v| dec(Some(v))),
            high: arr.and_then(|a| a.get(2)).and_then(|v| dec(Some(v))),
            low: arr.and_then(|a| a.get(1)).and_then(|v| dec(Some(v))),
            close: arr.and_then(|a| a.get(4)).and_then(|v| dec(Some(v))),
            volume: arr.and_then(|a| a.get(5)).and_then(|v| dec(Some(v))),
        }
    }

    pub fn parse_order_book(&self, raw: &Value, id: &str) -> OrderBook {
        let bids = raw["bids"]
            .as_array()
            .map(|a| a.iter().map(parse_level).collect())
            .unwrap_or_default();
        let asks = raw["asks"]
            .as_array()
            .map(|a| a.iter().map(parse_level).collect())
            .unwrap_or_default();
        OrderBook {
            symbol: self.market_symbol(id),
            bids,
            asks,
            info: raw.clone(),
            ..OrderBook::default()
        }
    }

    pub fn parse_order(&self, raw: &Value) -> Order {
        let status = raw["is_live"].as_bool().map(|live| {
            if live {
                "open"
            } else if raw["is_cancelled"].as_bool().unwrap_or(false) {
                "canceled"
            } else {
                "closed"
            }
        });
        let qty = dec(raw.get("original_amount"));
        let filled = dec(raw.get("executed_amount"));
        Order {
            id: raw["order_id"].as_i64().map(|v| v.to_string()),
            client_order_id: raw["client_order_id"].as_str().map(String::from),
            timestamp: raw["timestampms"].as_i64(),
            datetime: raw["timestampms"].as_i64().and_then(iso8601_ms),
            status: status.map(String::from),
            symbol: raw["symbol"].as_str().map(|s| self.market_symbol(s)),
            order_type: Some("limit".to_string()),
            side: raw["side"].as_str().map(|s| s.to_lowercase()),
            price: dec(raw.get("price")),
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
}

impl Exchange for Gemini {
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
        let id = self.symbol_id(symbol);
        let resp = self
            .public_request(&format!("/v2/ticker/{id}"), &Params::new())
            .await?;
        Ok(self.parse_ticker(&resp))
    }

    async fn fetch_tickers(&self, symbols: Option<&[&str]>, _params: Params) -> Result<Tickers> {
        // gemini 无批量 ticker,逐一对活跃 symbol 拉取
        self.core.load_markets(|| self.fetch_markets_raw()).await?;
        let wanted: Option<Vec<String>> =
            symbols.map(|s| s.iter().map(|x| x.to_string()).collect());
        let mut out = Tickers::new();
        for m in self.core.markets_snapshot().values() {
            if let Some(w) = &wanted {
                if !w.contains(&m.symbol) {
                    continue;
                }
            }
            if let Ok(t) = self
                .public_request(&format!("/v2/ticker/{}", m.id), &Params::new())
                .await
            {
                out.insert(m.symbol.clone(), self.parse_ticker(&t));
            }
        }
        Ok(out)
    }

    async fn fetch_order_book(
        &self,
        symbol: &str,
        limit: Option<i64>,
        _params: Params,
    ) -> Result<OrderBook> {
        let id = self.symbol_id(symbol);
        let mut p = Params::new();
        if let Some(l) = limit {
            p.insert("limit".into(), json!(l.min(100)));
        }
        let resp = self.public_request(&format!("/v1/book/{id}"), &p).await?;
        Ok(self.parse_order_book(&resp, &id))
    }

    async fn fetch_trades(
        &self,
        symbol: &str,
        _since: Option<i64>,
        limit: Option<i64>,
        _params: Params,
    ) -> Result<Vec<Trade>> {
        let id = self.symbol_id(symbol);
        let mut p = Params::new();
        if let Some(l) = limit {
            p.insert("limit".into(), json!(l.min(500)));
        }
        let resp = self.public_request(&format!("/v1/trades/{id}"), &p).await?;
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
        let id = self.symbol_id(symbol);
        let tf = match timeframe {
            "1m" => "1m",
            "5m" => "5m",
            "15m" => "15m",
            "30m" => "30m",
            "1h" => "1h",
            "6h" => "6h",
            "1d" => "1d",
            other => {
                return Err(Error::new(
                    ErrorKind::BadRequest,
                    format!("unsupported timeframe {other}"),
                ));
            }
        };
        let path = format!("/v2/candles/{id}/{tf}");
        let resp = self.public_request(&path, &Params::new()).await?;
        let arr = resp.as_array().cloned().unwrap_or_default();
        let mut out: Vec<OHLCV> = arr.iter().map(|r| self.parse_ohlcv(r)).collect();
        apply_client_limit(&mut out, limit.map(|l| l as usize), false);
        Ok(out)
    }

    async fn fetch_balance(&self, _params: Params) -> Result<Balances> {
        let resp = self
            .private_request("POST", "/v1/balances", json!({}))
            .await?;
        let mut accounts = HashMap::new();
        if let Some(arr) = resp.as_array() {
            for b in arr {
                let cur = b["currency"].as_str().unwrap_or_default().to_string();
                let free = dec(b.get("available"));
                let total = dec(b.get("amount"));
                accounts.insert(
                    cur,
                    Balance {
                        free,
                        used: match (total, free) {
                            (Some(t), Some(f)) => Some(t - f),
                            _ => None,
                        },
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
        let id = self.symbol_id(symbol);
        let mut body = json!({
            "symbol": id,
            "amount": amount,
            "side": side.to_lowercase(),
            "type": if order_type.eq_ignore_ascii_case("market") { "market" } else { "exchange limit" },
        });
        if let Some(px) = price {
            body["price"] = json!(px);
        }
        let resp = self.private_request("POST", "/v1/order/new", body).await?;
        Ok(self.parse_order(&resp))
    }

    async fn cancel_order(&self, id: &str, symbol: &str, _params: Params) -> Result<Order> {
        let body = json!({
            "order_id": id.parse::<i64>().unwrap_or(0),
            "symbol": self.symbol_id(symbol),
        });
        let resp = self
            .private_request("POST", "/v1/order/cancel", body)
            .await?;
        Ok(self.parse_order(&resp))
    }

    async fn fetch_order(&self, id: &str, symbol: &str, _params: Params) -> Result<Order> {
        let body = json!({
            "order_id": id.parse::<i64>().unwrap_or(0),
            "symbol": self.symbol_id(symbol),
        });
        let resp = self
            .private_request("POST", "/v1/order/status", body)
            .await?;
        Ok(self.parse_order(&resp))
    }

    async fn fetch_open_orders(
        &self,
        symbol: Option<&str>,
        _since: Option<i64>,
        _limit: Option<i64>,
        _params: Params,
    ) -> Result<Vec<Order>> {
        let mut body = json!({});
        if let Some(s) = symbol {
            body["symbol"] = json!(self.symbol_id(s));
        }
        let resp = self.private_request("POST", "/v1/orders", body).await?;
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
        // gemini 无独立历史订单端点;复用未成交(活跃)订单列表
        self.fetch_open_orders(symbol, None, None, Params::new())
            .await
    }

    async fn fetch_my_trades(
        &self,
        symbol: Option<&str>,
        _since: Option<i64>,
        limit: Option<i64>,
        _params: Params,
    ) -> Result<Vec<Trade>> {
        let mut p = Params::new();
        if let Some(s) = symbol {
            p.insert("symbol".into(), json!(self.symbol_id(s)));
        }
        if let Some(l) = limit {
            p.insert("limit".into(), json!(l.min(500)));
        }
        let resp = self
            .private_request(
                "POST",
                "/v1/mytrades",
                Value::Object(p.into_iter().collect()),
            )
            .await?;
        let arr = resp.as_array().cloned().unwrap_or_default();
        Ok(arr.iter().map(|t| self.parse_trade(t)).collect())
    }
}

// ---------- 工具 ----------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbol_id_strips_slash() {
        let ex = Gemini::new(Config::new()).unwrap();
        assert_eq!(ex.symbol_id("BTC/USD"), "BTCUSD");
        assert_eq!(ex.market_symbol("BTCUSD"), "BTC/USD");
    }

    #[test]
    fn parse_ohlcv_gemini_order() {
        let ex = Gemini::new(Config::new()).unwrap();
        let row = json!([
            1600866900000i64,
            "0.03385",
            "0.033895",
            "0.03389",
            "0.03388",
            "0.115",
            "0.0039"
        ]);
        let c = ex.parse_ohlcv(&row);
        assert_eq!(c.timestamp, Some(1600866900000i64));
        assert_eq!(c.open, Some("0.03389".parse().unwrap()));
        assert_eq!(c.high, Some("0.033895".parse().unwrap()));
        assert_eq!(c.low, Some("0.03385".parse().unwrap()));
        assert_eq!(c.close, Some("0.03388".parse().unwrap()));
        assert_eq!(c.volume, Some("0.115".parse().unwrap()));
    }
}
