//! gate.io 现货适配器(Phase B,ADR-0005)。
//!
//! 对齐 ccxt gate(v4)语义:
//! - 公共面 `GET /api/v4/spot/...`,symbol id 为 `BTC_USDT`(分隔符 `_`);
//! - ticker 字段 `last/latest_bid/lowest_ask/high_24h/low_24h/change_percentage`,`t` 可选;
//! - candlesticks 行 `[ts秒, quote_volume, close, high, low, open, base_volume, ...]`
//!   → 映射 ts*1000 / open=5 / high=3 / low=4 / close=2 / volume=6;
//! - trades 字段 `id/create_time_ms/currency_pair/side/amount/price`;
//! - 私密面:签名 `hex(HMAC-SHA512(secret, METHOD\npath\nquery\nbodyHash\ntimestamp))`,
//!   `KEY/Timestamp/SIGN` 头;余额 `available`→free,`locked`→used;
//! - rateLimit 20ms。
//!
//! 参考实现基于 ccxt(MIT)适配器解析逻辑,见 `NOTICE`。

use reqwest::header::HeaderMap;
use serde_json::{Value, json};

use crate::error::{Error, ErrorKind, Result};
use crate::exchange::{Config, Exchange, Params};
use crate::httpcore::{HttpCore, dec, iso8601, parse_level, query_string, value_decimal};
use crate::types::{
    Balance, Balances, Level, Market, MarketType, Markets, OHLCV, OrderBook, Precision, Ticker,
    Tickers, Trade,
};

pub const ID: &str = "gate";
const BASE_URL: &str = "https://api.gateio.ws/api/v4";
const RATE_LIMIT_MS: u64 = 20;

/// gate.io 现货适配器。
pub struct Gate {
    config: Config,
    core: HttpCore,
}

impl Gate {
    /// 已实现(非 NotSupported)的 REST 方法面(契约声明,Phase B)。
    pub const IMPLEMENTED: &'static [&'static str] = &[
        "fetch_time",
        "fetch_markets",
        "fetch_ticker",
        "fetch_tickers",
        "fetch_ohlcv",
        "fetch_order_book",
        "fetch_trades",
        "fetch_balance",
    ];

    pub fn new(config: Config) -> Result<Self> {
        let core = HttpCore::new(&config, BASE_URL, RATE_LIMIT_MS, "gate")?;
        Ok(Self { config, core })
    }

    // ================= 内部 HTTP =================

    /// 私密 GET(v4 签名,payload = METHOD\npath\nquery\nbodyHash\ntimestamp)。
    async fn private_get(&self, path: &str, params: &Params) -> Result<Value> {
        let api_key = crate::signing::require_api_key(&self.config, "gate")?;
        let secret = crate::signing::require_secret(&self.config, "gate")?;
        let qs = query_string(params);
        let timestamp = now_secs().to_string();
        let signature_path = format!("/api/v4{path}");
        let payload = format!(
            "GET\n{signature_path}\n{}\n\n{timestamp}",
            qs.trim_start_matches('?')
        );
        let signature = crate::signing::hmac_sha512_hex(secret, &payload);
        let mut headers = HeaderMap::new();
        crate::signing::set_header(&mut headers, "KEY", api_key)?;
        crate::signing::set_header(&mut headers, "Timestamp", &timestamp)?;
        crate::signing::set_header(&mut headers, "SIGN", &signature)?;
        crate::signing::set_header(&mut headers, "Content-Type", "application/json")?;
        let url = format!("{path}{qs}");
        self.core.request("GET", &url, &headers, None).await
    }

    // ================= markets 缓存 =================

    async fn load_markets(&self) -> Result<()> {
        self.core.load_markets(|| self.fetch_markets_raw()).await
    }

    async fn fetch_markets_raw(&self) -> Result<Markets> {
        let resp = self
            .core
            .public_get("/spot/currency_pairs", &Params::new())
            .await?;
        let arr = resp
            .as_array()
            .ok_or_else(|| Error::new(ErrorKind::BadResponse, "currency_pairs not array"))?;
        let mut map = Markets::new();
        for raw in arr {
            let m = self.parse_market(raw);
            map.insert(m.symbol.clone(), m);
        }
        Ok(map)
    }

    /// 统一 symbol → id(`BTC/USDT` → `BTC_USDT`)。
    fn symbol_id(&self, symbol: &str) -> String {
        symbol.replace('/', "_")
    }

    /// id → 统一 symbol(`BTC_USDT` → `BTC/USDT`):先查缓存,否则按 `_` 拆分。
    fn market_symbol(&self, id: &str) -> String {
        if let Some(m) = self.core.markets_snapshot().values().find(|m| m.id == id) {
            return m.symbol.clone();
        }
        match id.split_once('_') {
            Some((b, q)) if !b.is_empty() && !q.is_empty() => {
                format!("{}/{}", b.to_uppercase(), q.to_uppercase())
            }
            _ => id.to_string(),
        }
    }

    // ================= parse(公开,供差分测试) =================

    pub fn parse_market(&self, raw: &Value) -> Market {
        let id = raw["id"].as_str().unwrap_or_default();
        let (base_id, quote_id) = match id.split_once('_') {
            Some((b, q)) => (b.to_string(), q.to_string()),
            None => (String::new(), String::new()),
        };
        let base = base_id.to_uppercase();
        let quote = quote_id.to_uppercase();
        let active = raw["trade_status"].as_str() == Some("tradable");
        // gate precision 为整数(数字或字符串),如 6 → 10^-6
        let precision = |scale: Option<&Value>| {
            scale
                .and_then(|v| match v {
                    Value::String(s) => s.parse::<u32>().ok(),
                    Value::Number(n) => n.as_u64().and_then(|n| u32::try_from(n).ok()),
                    _ => None,
                })
                .map(|n| rust_decimal::Decimal::new(1, n))
        };
        Market {
            id: id.to_string(),
            symbol: format!("{base}/{quote}"),
            base: Some(base),
            quote: Some(quote),
            base_id: Some(base_id),
            quote_id: Some(quote_id),
            active: Some(active),
            market_type: Some(MarketType::Spot),
            spot: Some(true),
            taker: raw["fee"]
                .as_str()
                .and_then(|s| s.parse().ok())
                .map(|f: rust_decimal::Decimal| f / rust_decimal::Decimal::new(100, 0)),
            maker: raw["maker_fee_rate"]
                .as_str()
                .or_else(|| raw["fee"].as_str())
                .and_then(|s| s.parse().ok())
                .map(|f: rust_decimal::Decimal| f / rust_decimal::Decimal::new(100, 0)),
            precision: Precision {
                amount: precision(raw.get("amount_precision")),
                price: precision(raw.get("precision")),
                cost: None,
            },
            limits: crate::types::Limits {
                amount: Some(crate::types::Limit {
                    min: dec(raw.get("min_base_amount")),
                    max: None,
                }),
                cost: Some(crate::types::Limit {
                    min: dec(raw.get("min_quote_amount")),
                    max: None,
                }),
                ..crate::types::Limits::default()
            },
            info: raw.clone(),
            ..Market::default()
        }
    }

    pub fn parse_ticker(&self, raw: &Value) -> Ticker {
        let id = raw["currency_pair"].as_str().unwrap_or_default();
        let ts = raw["t"].as_i64();
        let close = dec(raw.get("last"));
        Ticker {
            symbol: self.market_symbol(id),
            timestamp: ts,
            datetime: ts.and_then(iso8601),
            high: dec(raw.get("high_24h")),
            low: dec(raw.get("low_24h")),
            bid: dec(raw.get("highest_bid")),
            ask: dec(raw.get("lowest_ask")),
            close,
            last: close,
            percentage: dec(raw.get("change_percentage")),
            base_volume: dec(raw.get("base_volume")),
            quote_volume: dec(raw.get("quote_volume")),
            info: raw.clone(),
            ..Ticker::default()
        }
    }

    pub fn parse_ohlcv(&self, row: &Value) -> OHLCV {
        let arr = row.as_array();
        let ts = arr
            .and_then(|a| a.first())
            .and_then(|v| match v {
                Value::String(s) => s.parse::<i64>().ok(),
                Value::Number(n) => n.as_i64(),
                _ => None,
            })
            .map(|s| s * 1000);
        OHLCV {
            timestamp: ts,
            open: arr.and_then(|a| a.get(5)).and_then(value_decimal),
            high: arr.and_then(|a| a.get(3)).and_then(value_decimal),
            low: arr.and_then(|a| a.get(4)).and_then(value_decimal),
            close: arr.and_then(|a| a.get(2)).and_then(value_decimal),
            volume: arr.and_then(|a| a.get(6)).and_then(value_decimal),
        }
    }

    pub fn parse_trade(&self, raw: &Value) -> Trade {
        // create_time_ms 为字符串(如 "1626342738331.497000")→ 取前 13 位
        let timestamp = raw
            .get("create_time_ms")
            .and_then(Value::as_str)
            .and_then(|s| {
                let s = if s.contains('.') {
                    &s[..s.find('.').unwrap_or(s.len())]
                } else {
                    s
                };
                s.get(0..13.min(s.len()))
                    .and_then(|t| t.parse::<i64>().ok())
            })
            .or_else(|| {
                raw.get("create_time")
                    .and_then(Value::as_str)
                    .and_then(|s| s.parse::<i64>().ok())
                    .map(|s| s * 1000)
            });
        let price = dec(raw.get("price"));
        let amount = dec(raw.get("amount"));
        Trade {
            id: raw["id"].as_str().map(str::to_string),
            timestamp,
            datetime: timestamp.and_then(iso8601),
            symbol: raw["currency_pair"].as_str().map(|s| self.market_symbol(s)),
            side: raw["side"].as_str().map(str::to_string),
            price,
            amount,
            cost: match (price, amount) {
                (Some(p), Some(a)) => Some(p * a),
                _ => None,
            },
            info: raw.clone(),
            ..Trade::default()
        }
    }

    pub fn parse_order_book(&self, raw: &Value, symbol: &str) -> OrderBook {
        let mut bids: Vec<Level> = raw
            .get("bids")
            .and_then(Value::as_array)
            .map(|a| a.iter().map(parse_level).collect())
            .unwrap_or_default();
        let mut asks: Vec<Level> = raw
            .get("asks")
            .and_then(Value::as_array)
            .map(|a| a.iter().map(parse_level).collect())
            .unwrap_or_default();
        bids.sort_by_key(|l| std::cmp::Reverse(l.price));
        asks.sort_by_key(|l| l.price);
        OrderBook {
            symbol: symbol.to_string(),
            bids,
            asks,
            nonce: raw["id"].as_i64(),
            info: raw.clone(),
            ..OrderBook::default()
        }
    }
}

// ================= Exchange trait =================

impl Exchange for Gate {
    fn id(&self) -> &'static str {
        ID
    }
    fn config(&self) -> &Config {
        &self.config
    }

    async fn fetch_time(&self) -> Result<i64> {
        let resp = self.core.public_get("/spot/time", &Params::new()).await?;
        resp["server_time"]
            .as_i64()
            .ok_or_else(|| Error::new(ErrorKind::BadResponse, "missing server_time"))
    }

    async fn fetch_markets(&self) -> Result<Vec<Market>> {
        self.load_markets().await?;
        Ok(self.core.markets_snapshot().into_values().collect())
    }

    async fn fetch_ticker(&self, symbol: &str, _params: Params) -> Result<Ticker> {
        let p = params1("currency_pair", &self.symbol_id(symbol));
        let resp = self.core.public_get("/spot/tickers", &p).await?;
        let raw = resp
            .as_array()
            .and_then(|a| a.first())
            .ok_or_else(|| Error::new(ErrorKind::BadResponse, "ticker empty"))?;
        Ok(self.parse_ticker(raw))
    }

    async fn fetch_tickers(&self, _symbols: Option<&[&str]>, _params: Params) -> Result<Tickers> {
        let resp = self
            .core
            .public_get("/spot/tickers", &Params::new())
            .await?;
        let arr = resp.as_array().cloned().unwrap_or_default();
        let mut out = Tickers::new();
        for raw in arr {
            let t = self.parse_ticker(&raw);
            out.insert(t.symbol.clone(), t);
        }
        Ok(out)
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
        p.insert("currency_pair".into(), json!(self.symbol_id(symbol)));
        p.insert("interval".into(), json!(timeframe));
        if let Some(l) = limit {
            p.insert("limit".into(), json!(l.min(1000)));
        }
        let resp = self.core.public_get("/spot/candlesticks", &p).await?;
        let arr = resp.as_array().cloned().unwrap_or_default();
        Ok(arr.iter().map(|r| self.parse_ohlcv(r)).collect())
    }

    async fn fetch_order_book(
        &self,
        symbol: &str,
        limit: Option<i64>,
        _params: Params,
    ) -> Result<OrderBook> {
        let mut p = Params::new();
        p.insert("currency_pair".into(), json!(self.symbol_id(symbol)));
        if let Some(l) = limit {
            p.insert("limit".into(), json!(l.min(100)));
        }
        let resp = self.core.public_get("/spot/order_book", &p).await?;
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
        p.insert("currency_pair".into(), json!(self.symbol_id(symbol)));
        if let Some(l) = limit {
            p.insert("limit".into(), json!(l.min(1000)));
        }
        let resp = self.core.public_get("/spot/trades", &p).await?;
        let arr = resp.as_array().cloned().unwrap_or_default();
        Ok(arr.iter().map(|t| self.parse_trade(t)).collect())
    }

    async fn fetch_balance(&self, _params: Params) -> Result<Balances> {
        let resp = self.private_get("/spot/accounts", &Params::new()).await?;
        let mut out = Balances {
            info: resp.clone(),
            ..Balances::default()
        };
        if let Some(arr) = resp.as_array() {
            for a in arr {
                if let Some(code) = a["currency"].as_str() {
                    let free = dec(a.get("available"));
                    let locked = dec(a.get("locked"));
                    out.accounts.insert(
                        code.to_string(),
                        Balance {
                            free,
                            used: locked,
                            total: match (free, locked) {
                                (Some(f), Some(l)) => Some(f + l),
                                (Some(f), None) => Some(f),
                                (None, Some(l)) => Some(l),
                                _ => None,
                            },
                            ..Balance::default()
                        },
                    );
                }
            }
        }
        Ok(out)
    }
}

// ================= 静态助手 =================

fn params1(k: &str, v: &str) -> Params {
    let mut p = Params::new();
    p.insert(k.into(), json!(v));
    p
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbol_id_converts() {
        let ex = Gate::new(Config::new()).unwrap();
        assert_eq!(ex.symbol_id("BTC/USDT"), "BTC_USDT");
        assert_eq!(ex.market_symbol("BTC_USDT"), "BTC/USDT");
    }

    #[test]
    fn parse_market_spot() {
        let ex = Gate::new(Config::new()).unwrap();
        let raw = json!({
            "id": "BTC_USDT",
            "base": "BTC",
            "quote": "USDT",
            "fee": "0.2",
            "amount_precision": "6",
            "precision": "2",
            "trade_status": "tradable",
            "min_base_amount": "0.000001"
        });
        let m = ex.parse_market(&raw);
        assert_eq!(m.symbol, "BTC/USDT");
        assert_eq!(m.active, Some(true));
        assert_eq!(m.precision.price, Some("0.01".parse().unwrap()));
        assert_eq!(m.precision.amount, Some("0.000001".parse().unwrap()));
        assert_eq!(m.taker, Some("0.002".parse().unwrap()));
    }

    #[test]
    fn parse_ticker_fields() {
        let ex = Gate::new(Config::new()).unwrap();
        let raw = json!({
            "currency_pair": "BTC_USDT",
            "last": "63742.1",
            "highest_bid": "63742",
            "lowest_ask": "63742.1",
            "high_24h": "64495.9",
            "low_24h": "63234.7",
            "base_volume": "4692.001758",
            "quote_volume": "299496732.1057458",
            "change_percentage": "-0.52"
        });
        let t = ex.parse_ticker(&raw);
        assert_eq!(t.symbol, "BTC/USDT");
        assert_eq!(t.last, Some("63742.1".parse().unwrap()));
        assert_eq!(t.bid, Some("63742".parse().unwrap()));
        assert_eq!(t.percentage, Some("-0.52".parse().unwrap()));
    }

    #[test]
    fn parse_ohlcv_gate_format() {
        let ex = Gate::new(Config::new()).unwrap();
        let row = json!([
            "1786521120",
            "82814.41",
            "63725.9",
            "63725.9",
            "63715.9",
            "63715.9",
            "1.29961400",
            "true"
        ]);
        let c = ex.parse_ohlcv(&row);
        assert_eq!(c.timestamp, Some(1786521120000));
        assert_eq!(c.open, Some("63715.9".parse().unwrap()));
        assert_eq!(c.high, Some("63725.9".parse().unwrap()));
        assert_eq!(c.low, Some("63715.9".parse().unwrap()));
        assert_eq!(c.close, Some("63725.9".parse().unwrap()));
        assert_eq!(c.volume, Some("1.29961400".parse().unwrap()));
    }

    #[test]
    fn parse_trade_fields() {
        let ex = Gate::new(Config::new()).unwrap();
        let raw = json!({
            "id": "1334253759",
            "create_time_ms": "1626342738331.497000",
            "currency_pair": "BTC_USDT",
            "side": "sell",
            "amount": "0.0022",
            "price": "32452.16"
        });
        let t = ex.parse_trade(&raw);
        assert_eq!(t.id.as_deref(), Some("1334253759"));
        assert_eq!(t.timestamp, Some(1626342738331));
        assert_eq!(t.side.as_deref(), Some("sell"));
        assert_eq!(t.cost, Some("71.394752".parse().unwrap()));
    }
}
