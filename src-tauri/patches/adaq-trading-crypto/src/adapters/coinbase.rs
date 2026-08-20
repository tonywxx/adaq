//! coinbase 现货适配器(Phase B,ADR-0005)。
//!
//! 对齐 ccxt coinbase(v3 Advanced Trade API)语义:
//! - 公共面 `GET /api/v3/brokerage/market/...`,symbol id 为 `BTC-USDT`(分隔符 `-`);
//! - 订单簿 `pricebook.{bids,asks}` 为 `[{price, size}]` 对象;
//! - OHLCV candles 为 `[{start, open, high, low, close, volume}]`,`start` 为**秒**;
//! - trades 复用 ticker 端点(`products/{id}/ticker` 响应含 `trades`);
//! - 私密面:签名 `hex(HMAC-SHA256(secret, timestamp + method + path))`(v3 GET
//!   不含 query/payload,POST 含 JSON body),`CB-ACCESS-KEY/SIGN/TIMESTAMP` 头;
//! - fetch_time 走 v2 `GET /v2/time`(`data.epoch` 秒)。
//!
//! 参考实现基于 ccxt(MIT)适配器解析逻辑,见 `NOTICE`。

use reqwest::header::HeaderMap;
use serde_json::{Value, json};

use crate::error::{Error, ErrorKind, Result};
use crate::exchange::{Config, Exchange, Params};
use crate::httpcore::{HttpCore, dec, query_string};
use crate::types::{
    Balance, Balances, Level, Market, MarketType, Markets, OHLCV, OrderBook, Precision, Ticker,
    Tickers, Trade,
};

pub const ID: &str = "coinbase";
const BASE_URL: &str = "https://api.coinbase.com";
const RATE_LIMIT_MS: u64 = 334; // ccxt rateLimit ~= 3 req/s

/// coinbase 现货适配器。
pub struct Coinbase {
    config: Config,
    core: HttpCore,
}

impl Coinbase {
    /// 已实现(非 NotSupported)的 REST 方法面(契约声明,Phase B)。
    pub const IMPLEMENTED: &'static [&'static str] = &[
        "fetch_time",
        "fetch_markets",
        "fetch_ticker",
        "fetch_tickers",
        "fetch_bids_asks",
        "fetch_ohlcv",
        "fetch_order_book",
        "fetch_trades",
        "fetch_balance",
    ];

    pub fn new(config: Config) -> Result<Self> {
        let core = HttpCore::new(&config, BASE_URL, RATE_LIMIT_MS, "coinbase")?;
        Ok(Self { config, core })
    }

    // ================= 内部 HTTP =================

    /// 公共 GET(v3,path 以 `/` 开头,如 `/brokerage/market/products`)。
    async fn public_get(&self, path: &str, params: &Params) -> Result<Value> {
        let url = format!("{BASE_URL}/api/v3{path}{}", query_string(params));
        let headers = HeaderMap::new();
        self.core.request_url("GET", &url, &headers, None).await
    }

    /// 私密 GET(v3 签名)。
    async fn private_get(&self, path: &str, params: &Params) -> Result<Value> {
        let api_key = crate::signing::require_api_key(&self.config, "coinbase")?;
        let secret = crate::signing::require_secret(&self.config, "coinbase")?;
        let timestamp = now_secs().to_string();
        // v3:GET 签名只覆盖 `timestamp + method + path`(不含 query)
        let auth = format!("{timestamp}GET/api/v3{path}");
        let signature = crate::signing::hmac_sha256_hex(secret, &auth);
        let mut headers = HeaderMap::new();
        crate::signing::set_header(&mut headers, "CB-ACCESS-KEY", api_key)?;
        crate::signing::set_header(&mut headers, "CB-ACCESS-SIGN", &signature)?;
        crate::signing::set_header(&mut headers, "CB-ACCESS-TIMESTAMP", &timestamp)?;
        crate::signing::set_header(&mut headers, "Content-Type", "application/json")?;
        let url = format!("{BASE_URL}/api/v3{path}{}", query_string(params));
        self.core.request_url("GET", &url, &headers, None).await
    }

    // ================= markets 缓存 =================

    async fn load_markets(&self) -> Result<()> {
        self.core.load_markets(|| self.fetch_markets_raw()).await
    }

    async fn fetch_markets_raw(&self) -> Result<Markets> {
        let resp = self
            .public_get("/brokerage/market/products", &Params::new())
            .await?;
        let arr = resp
            .get("products")
            .and_then(Value::as_array)
            .ok_or_else(|| Error::new(ErrorKind::BadResponse, "products not array"))?;
        let mut map = Markets::new();
        for raw in arr {
            let m = self.parse_market(raw);
            map.insert(m.symbol.clone(), m);
        }
        Ok(map)
    }

    /// 统一 symbol → product_id(`BTC/USDT` → `BTC-USDT`)。
    fn symbol_id(&self, symbol: &str) -> String {
        symbol.replace('/', "-")
    }

    fn market_symbol(&self, id: &str) -> String {
        id.replace('-', "/")
    }

    // ================= parse(公开,供差分测试) =================

    pub fn parse_market(&self, raw: &Value) -> Market {
        let product_id = raw["product_id"].as_str().unwrap_or_default();
        // 对齐 ccxt parse_spot_market:active = !trading_disabled
        let active = !raw["trading_disabled"].as_bool().unwrap_or(false);
        Market {
            id: product_id.to_string(),
            symbol: self.market_symbol(product_id),
            base: raw["base_currency_id"].as_str().map(String::from),
            quote: raw["quote_currency_id"].as_str().map(String::from),
            base_id: raw["base_currency_id"].as_str().map(String::from),
            quote_id: raw["quote_currency_id"].as_str().map(String::from),
            active: Some(active),
            market_type: Some(MarketType::Spot),
            spot: Some(true),
            precision: Precision {
                price: dec(raw.get("price_increment")).or_else(|| dec(raw.get("quote_increment"))),
                amount: dec(raw.get("base_increment")),
                cost: None,
            },
            limits: crate::types::Limits {
                amount: Some(crate::types::Limit {
                    min: dec(raw.get("base_min_size")),
                    max: dec(raw.get("base_max_size")),
                }),
                cost: Some(crate::types::Limit {
                    min: dec(raw.get("quote_min_size")),
                    max: dec(raw.get("quote_max_size")),
                }),
                ..crate::types::Limits::default()
            },
            info: raw.clone(),
            ..Market::default()
        }
    }

    pub fn parse_ticker(&self, raw: &Value) -> Ticker {
        let product_id = raw["product_id"].as_str().unwrap_or_default();
        let datetime = raw["time"].as_str().map(str::to_string);
        let timestamp = datetime.as_deref().and_then(parse_iso8601);
        let last = dec(raw.get("price"));
        let bid = dec(raw.get("bid")).or_else(|| {
            raw.get("bids")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(|b| dec(b.get("price")))
        });
        let ask = dec(raw.get("ask")).or_else(|| {
            raw.get("asks")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(|b| dec(b.get("price")))
        });
        Ticker {
            symbol: self.market_symbol(product_id),
            timestamp,
            datetime,
            bid,
            ask,
            last,
            close: last,
            percentage: dec(raw.get("price_percentage_change_24h")),
            base_volume: dec(raw.get("volume_24h")),
            quote_volume: dec(raw.get("approximate_quote_24h_volume")),
            info: raw.clone(),
            ..Ticker::default()
        }
    }

    pub fn parse_ohlcv(&self, row: &Value) -> OHLCV {
        OHLCV {
            // start 为秒 → 毫秒
            timestamp: row["start"]
                .as_str()
                .and_then(|s| s.parse::<i64>().ok())
                .map(|s| s * 1000),
            open: dec(row.get("open")),
            high: dec(row.get("high")),
            low: dec(row.get("low")),
            close: dec(row.get("close")),
            volume: dec(row.get("volume")),
        }
    }

    pub fn parse_trade(&self, raw: &Value) -> Trade {
        let datetime = raw["time"].as_str().map(str::to_string);
        let timestamp = datetime.as_deref().and_then(parse_iso8601);
        let price = dec(raw.get("price"));
        let amount = dec(raw.get("size"));
        Trade {
            id: raw["trade_id"].as_str().map(str::to_string),
            info: raw.clone(),
            timestamp,
            datetime,
            symbol: raw["product_id"].as_str().map(|s| s.replace('-', "/")),
            side: raw["side"].as_str().map(|s| s.to_lowercase()),
            price,
            amount,
            cost: match (price, amount) {
                (Some(p), Some(a)) => Some(p * a),
                _ => None,
            },
            ..Trade::default()
        }
    }

    pub fn parse_order_book(&self, raw: &Value, symbol: &str) -> OrderBook {
        let book = raw.get("pricebook").unwrap_or(raw);
        let mut bids: Vec<Level> = book
            .get("bids")
            .and_then(Value::as_array)
            .map(|a| a.iter().map(parse_level).collect())
            .unwrap_or_default();
        let mut asks: Vec<Level> = book
            .get("asks")
            .and_then(Value::as_array)
            .map(|a| a.iter().map(parse_level).collect())
            .unwrap_or_default();
        bids.sort_by_key(|l| std::cmp::Reverse(l.price));
        asks.sort_by_key(|l| l.price);
        let datetime = book["time"].as_str().map(str::to_string);
        OrderBook {
            symbol: symbol.to_string(),
            bids,
            asks,
            timestamp: datetime.as_deref().and_then(parse_iso8601),
            info: book.clone(),
            ..OrderBook::default()
        }
    }
}

// ================= Exchange trait =================

impl Exchange for Coinbase {
    fn id(&self) -> &'static str {
        ID
    }
    fn config(&self) -> &Config {
        &self.config
    }

    async fn fetch_time(&self) -> Result<i64> {
        // v2 `/v2/time` → data.epoch(秒)
        let url = format!("{BASE_URL}/v2/time");
        let headers = HeaderMap::new();
        let resp = self.core.request_url("GET", &url, &headers, None).await?;
        let epoch = resp
            .get("data")
            .and_then(|d| d.get("epoch"))
            .and_then(Value::as_i64)
            .ok_or_else(|| Error::new(ErrorKind::BadResponse, "missing data.epoch"))?;
        Ok(epoch * 1000)
    }

    async fn fetch_markets(&self) -> Result<Vec<Market>> {
        self.load_markets().await?;
        Ok(self.core.markets_snapshot().into_values().collect())
    }

    async fn fetch_ticker(&self, symbol: &str, _params: Params) -> Result<Ticker> {
        // 对齐 ccxt fetch_ticker_v3:ticker 由 trades[0] 推导,bid/ask 取 best_bid/best_ask
        let mut p = Params::new();
        p.insert("limit".into(), json!(1));
        let id = self.symbol_id(symbol);
        let resp = self
            .public_get(&format!("/brokerage/market/products/{id}/ticker"), &p)
            .await?;
        let first = resp
            .get("trades")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .cloned()
            .unwrap_or_default();
        let mut t = self.parse_ticker(&first);
        t.bid = dec(resp.get("best_bid")).or(t.bid);
        t.ask = dec(resp.get("best_ask")).or(t.ask);
        Ok(t)
    }

    async fn fetch_tickers(&self, symbols: Option<&[&str]>, _params: Params) -> Result<Tickers> {
        let resp = self
            .public_get("/brokerage/market/products", &Params::new())
            .await?;
        let arr = resp
            .get("products")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
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

    async fn fetch_bids_asks(&self, symbols: Option<&[&str]>, _params: Params) -> Result<Tickers> {
        // 公共 best_bid_ask 需私密端点;退化为逐 symbol 的 ticker(仅 bid/ask)
        let mut out = Tickers::new();
        let wanted: Vec<String> = match symbols {
            Some(s) => s.iter().map(|s| s.to_string()).collect(),
            None => {
                self.load_markets().await?;
                self.core.markets_snapshot().into_keys().take(100).collect()
            }
        };
        for sym in wanted {
            let resp = self
                .public_get(
                    &format!("/brokerage/market/products/{}/ticker", self.symbol_id(&sym)),
                    &Params::new(),
                )
                .await?;
            let t = self.parse_ticker(&resp);
            out.insert(
                sym,
                Ticker {
                    bid: t.bid,
                    ask: t.ask,
                    symbol: t.symbol,
                    info: t.info,
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
        // coinbase granularity 为字符串枚举;必须带 start/end(秒)
        let (granularity, seconds) = match timeframe {
            "1m" => ("ONE_MINUTE", 60i64),
            "5m" => ("FIVE_MINUTE", 300),
            "15m" => ("FIFTEEN_MINUTE", 900),
            "30m" => ("THIRTY_MINUTE", 1800),
            "1h" => ("ONE_HOUR", 3600),
            "2h" => ("TWO_HOUR", 7200),
            "6h" => ("SIX_HOUR", 21600),
            "1d" => ("ONE_DAY", 86400),
            other => {
                return Err(Error::new(
                    ErrorKind::BadRequest,
                    format!("unsupported timeframe {other}"),
                ));
            }
        };
        let n = limit.unwrap_or(300).min(300);
        let end = since
            .map(|s| s / 1000)
            .unwrap_or_else(now_secs as fn() -> i64);
        let start = since.map(|s| s / 1000).unwrap_or_else(|| end - n * seconds);
        let mut p = Params::new();
        p.insert("granularity".into(), json!(granularity));
        p.insert("start".into(), json!(start));
        p.insert("end".into(), json!(end));
        let id = self.symbol_id(symbol);
        let resp = self
            .public_get(&format!("/brokerage/market/products/{id}/candles"), &p)
            .await?;
        let arr = resp
            .get("candles")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(arr.iter().map(|r| self.parse_ohlcv(r)).collect())
    }

    async fn fetch_order_book(
        &self,
        symbol: &str,
        limit: Option<i64>,
        _params: Params,
    ) -> Result<OrderBook> {
        let mut p = Params::new();
        p.insert("product_id".into(), json!(self.symbol_id(symbol)));
        if let Some(l) = limit {
            p.insert("limit".into(), json!(l.min(100)));
        }
        let resp = self
            .public_get("/brokerage/market/product_book", &p)
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
        // trades 复用 ticker 端点(响应含 trades 列表)
        let mut p = Params::new();
        if let Some(l) = limit {
            p.insert("limit".into(), json!(l.min(1000)));
        }
        let id = self.symbol_id(symbol);
        let resp = self
            .public_get(&format!("/brokerage/market/products/{id}/ticker"), &p)
            .await?;
        let arr = resp
            .get("trades")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(arr.iter().map(|t| self.parse_trade(t)).collect())
    }

    async fn fetch_balance(&self, _params: Params) -> Result<Balances> {
        let resp = self
            .private_get("/brokerage/accounts", &Params::new())
            .await?;
        let mut out = Balances {
            info: resp.clone(),
            ..Balances::default()
        };
        if let Some(accounts) = resp.get("accounts").and_then(Value::as_array) {
            for a in accounts {
                let code = a["currency"].as_str().unwrap_or_default().to_string();
                if code.is_empty() {
                    continue;
                }
                let free = dec(a.get("available_balance").and_then(|b| b.get("value")));
                let used = dec(a.get("hold").and_then(|b| b.get("value")));
                let total = match (free, used) {
                    (Some(f), Some(u)) => Some(f + u),
                    (Some(f), None) => Some(f),
                    (None, Some(u)) => Some(u),
                    _ => None,
                };
                out.accounts.insert(
                    code,
                    Balance {
                        free,
                        used,
                        total,
                        ..Balance::default()
                    },
                );
            }
        }
        Ok(out)
    }
}

// ================= 静态助手 =================

/// `{price, size}` 对象 → Level。
fn parse_level(raw: &Value) -> Level {
    Level {
        price: dec(raw.get("price")),
        amount: dec(raw.get("size")),
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn parse_iso8601(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.timestamp_millis())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbol_id_converts_separator() {
        let ex = Coinbase::new(Config::new()).unwrap();
        assert_eq!(ex.symbol_id("BTC/USDT"), "BTC-USDT");
        assert_eq!(ex.market_symbol("BTC-USDT"), "BTC/USDT");
    }

    #[test]
    fn parse_market_spot() {
        let ex = Coinbase::new(Config::new()).unwrap();
        let raw = json!({
            "product_id": "BTC-USDT",
            "product_type": "SPOT",
            "base_currency_id": "BTC",
            "quote_currency_id": "USDT",
            "base_increment": "0.00000001",
            "quote_increment": "0.01",
            "price_increment": "0.01",
            "status": "online",
            "trading_disabled": false
        });
        let m = ex.parse_market(&raw);
        assert_eq!(m.symbol, "BTC/USDT");
        assert_eq!(m.base.as_deref(), Some("BTC"));
        assert_eq!(m.quote.as_deref(), Some("USDT"));
        assert_eq!(m.active, Some(true));
        assert_eq!(m.precision.price, Some("0.01".parse().unwrap()));
    }

    #[test]
    fn parse_ticker_fields() {
        let ex = Coinbase::new(Config::new()).unwrap();
        let raw = json!({
            "product_id": "BTC-USDT",
            "price": "100",
            "size": "0.1",
            "time": "2023-01-13T20:35:41.865970Z",
            "side": "BUY",
            "bid": "99.9",
            "ask": "100.1"
        });
        let t = ex.parse_ticker(&raw);
        assert_eq!(t.symbol, "BTC/USDT");
        assert_eq!(t.last, Some("100".parse().unwrap()));
        assert_eq!(t.bid, Some("99.9".parse().unwrap()));
        assert_eq!(t.ask, Some("100.1".parse().unwrap()));
        assert_eq!(t.timestamp, Some(1673642141865));
    }

    #[test]
    fn parse_ohlcv_seconds_to_ms() {
        let ex = Coinbase::new(Config::new()).unwrap();
        let row = json!({
            "start": "1673637300",
            "open": "100",
            "high": "101",
            "low": "99",
            "close": "100.5",
            "volume": "10"
        });
        let c = ex.parse_ohlcv(&row);
        assert_eq!(c.timestamp, Some(1673637300000));
        assert_eq!(c.close, Some("100.5".parse().unwrap()));
        assert_eq!(c.volume, Some("10".parse().unwrap()));
    }

    #[test]
    fn parse_order_book_sorted() {
        let ex = Coinbase::new(Config::new()).unwrap();
        let raw = json!({
            "pricebook": {
                "product_id": "BTC-USDT",
                "bids": [{"price": "99", "size": "2"}, {"price": "100", "size": "1"}],
                "asks": [{"price": "101", "size": "3"}, {"price": "102", "size": "4"}]
            }
        });
        let book = ex.parse_order_book(&raw, "BTC/USDT");
        assert_eq!(book.bids[0].price, Some("100".parse().unwrap()));
        assert_eq!(book.bids[1].price, Some("99".parse().unwrap()));
        assert_eq!(book.asks[0].price, Some("101".parse().unwrap()));
        assert_eq!(book.asks[1].amount, Some("4".parse().unwrap()));
    }

    #[test]
    fn parse_trade_fields() {
        let ex = Coinbase::new(Config::new()).unwrap();
        let raw = json!({
            "trade_id": "10209805",
            "product_id": "BTC-USDT",
            "price": "100",
            "size": "0.5",
            "time": "2023-01-13T20:35:41.865970Z",
            "side": "BUY"
        });
        let t = ex.parse_trade(&raw);
        assert_eq!(t.id.as_deref(), Some("10209805"));
        assert_eq!(t.side.as_deref(), Some("buy"));
        assert_eq!(t.price, Some("100".parse().unwrap()));
        assert_eq!(t.amount, Some("0.5".parse().unwrap()));
        assert_eq!(t.cost, Some("50".parse().unwrap()));
    }
}
