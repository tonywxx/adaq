//! binanceus 现货适配器(ADR-0017,由 generated promote 为 curated)。
//!
//! 手写完整交易 API。签名与 binance 现货一致:HMAC-SHA256(`X-MBX-APIKEY`
//! 头 + `signature` 查询参数)。公开端点路径与 binance 现货相同(`/api/v3/...`),
//! base URL 为 `https://api.binance.us`。
//!
//! 参考 binance.rs(curated 模板)。base URL 与端点以官方文档为准。

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

pub const ID: &str = "binanceus";
const BASE_URL: &str = "https://api.binance.us/api/v3";
const RATE_LIMIT_MS: u64 = 50;

/// 常见计价币(用于无缓存时推导统一 symbol)。
const COMMON_QUOTES: &[&str] = &[
    "USDT", "USDC", "FDUSD", "TUSD", "BUSD", "DAI", "EUR", "USD", "BTC", "ETH", "BNB", "PAX",
    "USDS",
];

pub struct BinanceUs {
    config: Config,
    core: HttpCore,
}

impl BinanceUs {
    /// 已实现(非 NotSupported)的 REST 方法面(契约声明,ADR-0017)。
    pub const IMPLEMENTED: &'static [&'static str] = &[
        "fetch_time",
        "fetch_markets",
        "fetch_currencies",
        "fetch_ticker",
        "fetch_tickers",
        "fetch_bids_asks",
        "fetch_ohlcv",
        "fetch_order_book",
        "fetch_trades",
        "fetch_balance",
        "create_order",
        "cancel_order",
        "fetch_order",
        "fetch_open_orders",
        "fetch_orders",
        "fetch_my_trades",
    ];

    pub fn new(config: Config) -> Result<Self> {
        let core = HttpCore::new(&config, BASE_URL, RATE_LIMIT_MS, "binanceus")?;
        Ok(Self { config, core })
    }

    /// 统一 symbol → binanceus id(`BTC/USDT` → `BTCUSDT`)。
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
        self.core.public_get(path, params).await
    }

    /// 私有签名请求(binanceus:HMAC-SHA256,`X-MBX-APIKEY` 头)。
    async fn private_request(&self, method: &str, path: &str, params: &Params) -> Result<Value> {
        let api_key = crate::signing::require_api_key(&self.config, "binanceus")?;
        let secret = crate::signing::require_secret(&self.config, "binanceus")?;
        let mut p = params.clone();
        p.insert("timestamp".into(), json!(now_ms()));
        p.insert("recvWindow".into(), json!(5000));
        let qs = query_string(&p);
        let signature = crate::signing::hmac_sha256_hex(secret, &qs);
        let url = format!("{BASE_URL}{path}?{qs}&signature={signature}");
        let mut headers = HeaderMap::new();
        crate::signing::set_header(&mut headers, "X-MBX-APIKEY", api_key)?;
        self.core.request_url(method, &url, &headers, None).await
    }

    async fn fetch_markets_raw(&self) -> Result<Markets> {
        let resp = self.public_request("/exchangeInfo", &Params::new()).await?;
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
        let status = raw["status"].as_str().unwrap_or_default();
        let mut precision = Precision::default();
        let mut limits = crate::types::Limits::default();
        if let Some(filters) = raw["filters"].as_array() {
            for f in filters {
                match f["filterType"].as_str() {
                    Some("PRICE_FILTER") => {
                        precision.price = dec(f.get("tickSize"));
                        limits.price = Some(crate::types::Limit {
                            min: dec(f.get("minPrice")),
                            max: dec(f.get("maxPrice")),
                        });
                    }
                    Some("LOT_SIZE") => {
                        precision.amount = dec(f.get("stepSize"));
                        limits.amount = Some(crate::types::Limit {
                            min: dec(f.get("minQty")),
                            max: dec(f.get("maxQty")),
                        });
                    }
                    Some("MIN_NOTIONAL") => {
                        limits.cost = Some(crate::types::Limit {
                            min: dec(f.get("minNotional")),
                            max: None,
                        });
                    }
                    _ => {}
                }
            }
        }
        Market {
            id,
            symbol: self.market_symbol(raw["symbol"].as_str().unwrap_or_default()),
            base: raw["baseAsset"].as_str().map(String::from),
            quote: raw["quoteAsset"].as_str().map(String::from),
            base_id: raw["baseAsset"].as_str().map(String::from),
            quote_id: raw["quoteAsset"].as_str().map(String::from),
            active: Some(status == "TRADING"),
            market_type: Some(MarketType::Spot),
            spot: Some(true),
            precision,
            limits,
            info: raw.clone(),
            ..Market::default()
        }
    }

    pub fn parse_ticker(&self, raw: &Value) -> Ticker {
        let timestamp = raw["closeTime"].as_i64();
        let open_price = dec(raw.get("openPrice"));
        let close_price = dec(raw.get("lastPrice"));
        let average = match (open_price, close_price) {
            (Some(o), Some(c)) => {
                let sum = crate::precise::string_add(&o.to_string(), &c.to_string());
                crate::precise::string_div(&sum, "2").parse().ok()
            }
            _ => None,
        };
        Ticker {
            symbol: self.market_symbol(raw["symbol"].as_str().unwrap_or_default()),
            timestamp,
            datetime: timestamp.and_then(iso8601_ms),
            high: dec(raw.get("highPrice")),
            low: dec(raw.get("lowPrice")),
            bid: dec(raw.get("bidPrice")),
            ask: dec(raw.get("askPrice")),
            bid_volume: dec(raw.get("bidQty")),
            ask_volume: dec(raw.get("askQty")),
            vwap: dec(raw.get("weightedAvgPrice")),
            open: open_price,
            close: close_price,
            last: close_price,
            previous_close: dec(raw.get("prevClosePrice")),
            change: dec(raw.get("priceChange")),
            percentage: dec(raw.get("priceChangePercent")),
            average,
            quote_volume: dec(raw.get("quoteVolume")),
            base_volume: dec(raw.get("volume")),
            info: raw.clone(),
            ..Ticker::default()
        }
    }

    pub fn parse_ohlcv(&self, row: &Value) -> OHLCV {
        parse_ohlcv_standard(row)
    }

    pub fn parse_trade(&self, raw: &Value) -> Trade {
        let timestamp = raw["time"].as_i64();
        let buyer_maker = raw["isBuyerMaker"].as_bool().unwrap_or(false);
        Trade {
            id: raw["id"].as_i64().map(|v| v.to_string()),
            info: raw.clone(),
            timestamp,
            datetime: timestamp.and_then(iso8601_ms),
            symbol: Some(self.market_symbol(raw["symbol"].as_str().unwrap_or_default())),
            side: Some(if buyer_maker { "sell" } else { "buy" }.to_string()),
            taker_or_maker: Some("taker".to_string()),
            price: dec(raw.get("price")),
            amount: dec(raw.get("qty")),
            cost: dec(raw.get("quoteQty")),
            ..Trade::default()
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
            nonce: raw["lastUpdateId"].as_i64(),
            info: raw.clone(),
            ..OrderBook::default()
        }
    }

    pub fn parse_order(&self, raw: &Value) -> Order {
        let status = raw["status"].as_str().map(|s| match s {
            "NEW" => "open",
            "PARTIALLY_FILLED" => "open",
            "FILLED" => "closed",
            "CANCELED" => "canceled",
            "REJECTED" => "rejected",
            "EXPIRED" => "expired",
            other => other,
        });
        let qty = dec(raw.get("origQty"));
        let filled = dec(raw.get("executedQty"));
        let remaining = match (qty, filled) {
            (Some(q), Some(f)) => Some(q - f),
            _ => None,
        };
        Order {
            id: raw["orderId"].as_i64().map(|v| v.to_string()),
            client_order_id: raw["clientOrderId"].as_str().map(String::from),
            timestamp: raw["transactTime"].as_i64(),
            datetime: raw["transactTime"].as_i64().and_then(iso8601_ms),
            status: status.map(String::from),
            symbol: raw["symbol"].as_str().map(|s| self.market_symbol(s)),
            order_type: raw["type"].as_str().map(|s| s.to_lowercase()),
            side: raw["side"].as_str().map(|s| s.to_lowercase()),
            price: dec(raw.get("price")),
            average: dec(raw.get("avgPrice")),
            amount: qty,
            filled,
            remaining,
            info: raw.clone(),
            ..Order::default()
        }
    }
}

impl Exchange for BinanceUs {
    fn id(&self) -> &'static str {
        ID
    }
    fn config(&self) -> &Config {
        &self.config
    }

    async fn fetch_time(&self) -> Result<i64> {
        let resp = self.public_request("/time", &Params::new()).await?;
        resp["serverTime"]
            .as_i64()
            .ok_or_else(|| Error::new(ErrorKind::BadResponse, "missing serverTime"))
    }

    async fn fetch_markets(&self) -> Result<Vec<Market>> {
        self.core.load_markets(|| self.fetch_markets_raw()).await?;
        Ok(self.core.markets_snapshot().into_values().collect())
    }

    async fn fetch_currencies(&self) -> Result<crate::types::Currencies> {
        Ok(HashMap::new())
    }

    async fn fetch_ticker(&self, symbol: &str, _params: Params) -> Result<Ticker> {
        let id = self.symbol_id(symbol);
        let mut p = Params::new();
        p.insert("symbol".into(), json!(id));
        let resp = self.public_request("/ticker/24hr", &p).await?;
        Ok(self.parse_ticker(&resp))
    }

    async fn fetch_tickers(&self, symbols: Option<&[&str]>, _params: Params) -> Result<Tickers> {
        let resp = self.public_request("/ticker/24hr", &Params::new()).await?;
        let arr = resp
            .as_array()
            .ok_or_else(|| Error::new(ErrorKind::BadResponse, "tickers not array"))?;
        let mut out = Tickers::new();
        for raw in arr {
            let t = self.parse_ticker(raw);
            if let Some(wanted) = symbols {
                if !wanted.contains(&t.symbol.as_str()) {
                    continue;
                }
            }
            out.insert(t.symbol.clone(), t);
        }
        Ok(out)
    }

    async fn fetch_bids_asks(&self, symbols: Option<&[&str]>, _params: Params) -> Result<Tickers> {
        let resp = self
            .public_request("/ticker/bookTicker", &Params::new())
            .await?;
        let arr = resp
            .as_array()
            .ok_or_else(|| Error::new(ErrorKind::BadResponse, "bookTicker not array"))?;
        let mut out = Tickers::new();
        for raw in arr {
            let symbol = self.market_symbol(raw["symbol"].as_str().unwrap_or_default());
            if let Some(wanted) = symbols {
                if !wanted.contains(&symbol.as_str()) {
                    continue;
                }
            }
            out.insert(
                symbol.clone(),
                Ticker {
                    symbol,
                    bid: dec(raw.get("bidPrice")),
                    bid_volume: dec(raw.get("bidQty")),
                    ask: dec(raw.get("askPrice")),
                    ask_volume: dec(raw.get("askQty")),
                    info: raw.clone(),
                    ..Ticker::default()
                },
            );
        }
        Ok(out)
    }

    async fn fetch_ohlcv(
        &self,
        symbol: &str,
        timeframe: &str,
        since: Option<i64>,
        limit: Option<i64>,
        _params: Params,
    ) -> Result<Vec<OHLCV>> {
        let id = self.symbol_id(symbol);
        let mut p = Params::new();
        p.insert("symbol".into(), json!(id));
        p.insert("interval".into(), json!(timeframe));
        if let Some(s) = since {
            p.insert("startTime".into(), json!(s));
        }
        if let Some(l) = limit {
            p.insert("limit".into(), json!(l));
        }
        let resp = self.public_request("/klines", &p).await?;
        let arr = resp
            .as_array()
            .ok_or_else(|| Error::new(ErrorKind::BadResponse, "klines not array"))?;
        Ok(arr.iter().map(|r| self.parse_ohlcv(r)).collect())
    }

    async fn fetch_order_book(
        &self,
        symbol: &str,
        limit: Option<i64>,
        _params: Params,
    ) -> Result<OrderBook> {
        let id = self.symbol_id(symbol);
        let mut p = Params::new();
        p.insert("symbol".into(), json!(id));
        if let Some(l) = limit {
            p.insert("limit".into(), json!(l));
        }
        let resp = self.public_request("/depth", &p).await?;
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
        p.insert("symbol".into(), json!(id));
        if let Some(l) = limit {
            p.insert("limit".into(), json!(l));
        }
        let resp = self.public_request("/trades", &p).await?;
        let arr = resp
            .as_array()
            .ok_or_else(|| Error::new(ErrorKind::BadResponse, "trades not array"))?;
        Ok(arr.iter().map(|r| self.parse_trade(r)).collect())
    }

    async fn fetch_balance(&self, _params: Params) -> Result<Balances> {
        let resp = self
            .private_request("GET", "/account", &Params::new())
            .await?;
        let mut accounts = HashMap::new();
        if let Some(balances) = resp["balances"].as_array() {
            for b in balances {
                let asset = b["asset"].as_str().unwrap_or_default().to_string();
                let free = dec(b.get("free"));
                let used = dec(b.get("locked"));
                let total = match (free, used) {
                    (Some(f), Some(u)) => Some(f + u),
                    (Some(f), None) => Some(f),
                    (None, Some(u)) => Some(u),
                    _ => None,
                };
                accounts.insert(
                    asset,
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
        let id = self.symbol_id(symbol);
        let mut p = Params::new();
        p.insert("symbol".into(), json!(id));
        p.insert("side".into(), json!(side.to_uppercase()));
        p.insert("type".into(), json!(order_type.to_uppercase()));
        p.insert("quantity".into(), json!(amount));
        if order_type.eq_ignore_ascii_case("limit") {
            let px = price
                .ok_or_else(|| Error::new(ErrorKind::BadRequest, "limit order requires price"))?;
            p.insert("price".into(), json!(px));
            p.insert("timeInForce".into(), json!("GTC"));
        }
        let resp = self.private_request("POST", "/order", &p).await?;
        Ok(self.parse_order(&resp))
    }

    async fn cancel_order(&self, id: &str, symbol: &str, _params: Params) -> Result<Order> {
        let s = self.symbol_id(symbol);
        let mut p = Params::new();
        p.insert("symbol".into(), json!(s));
        p.insert("orderId".into(), json!(id.parse::<i64>().unwrap_or(0)));
        let resp = self.private_request("DELETE", "/order", &p).await?;
        Ok(self.parse_order(&resp))
    }

    async fn fetch_order(&self, id: &str, symbol: &str, _params: Params) -> Result<Order> {
        let s = self.symbol_id(symbol);
        let mut p = Params::new();
        p.insert("symbol".into(), json!(s));
        p.insert("orderId".into(), json!(id.parse::<i64>().unwrap_or(0)));
        let resp = self.private_request("GET", "/order", &p).await?;
        Ok(self.parse_order(&resp))
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
        let resp = self.private_request("GET", "/openOrders", &p).await?;
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
        let resp = self.private_request("GET", "/allOrders", &p).await?;
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
        let resp = self.private_request("GET", "/myTrades", &p).await?;
        let arr = resp.as_array().cloned().unwrap_or_default();
        Ok(arr.iter().map(|t| self.parse_trade(t)).collect())
    }
}
