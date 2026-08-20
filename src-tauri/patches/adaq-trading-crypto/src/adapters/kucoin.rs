//! kucoin 现货适配器(Phase B,ADR-0005)。
//!
//! 对齐 ccxt kucoin(v1/v2)语义:
//! - 公共面 `GET /api/v1/market/...`,symbol id 为 `BTC-USDT`(分隔符 `-`);
//! - ticker(/market/stats):`last/buy/sell/changeRate(小数×100)/high/low/vol/volValue/averagePrice/time`;
//! - candles `[ts秒, open, close, high, low, volume, turnover]`(注意 close 在 high 前),
//!   ts ≤10 位 → ×1000;
//! - histories `{tradeId, price, size, side, time}`(time 为纳秒,ccxt 原样保留);
//! - 订单簿 level2_20:`{time, sequence, bids/asks}`;
//! - 私密面:`base64(HMAC-SHA256(ts+method+path+body))`,
//!   `KC-API-KEY/SIGN/TIMESTAMP/PASSPHRASE` 头;余额 `available`→free,`holds`→used;
//! - rateLimit 50ms。
//!
//! 参考实现基于 ccxt(MIT)适配器解析逻辑,见 `NOTICE`。

use reqwest::header::HeaderMap;
use serde_json::{Value, json};

use crate::error::{Error, ErrorKind, Result};
use crate::exchange::{Config, Exchange, Params};
use crate::httpcore::{HttpCore, dec, iso8601, now_ms, parse_level, query_string, value_decimal};
use crate::types::{
    Balance, Balances, Level, Market, MarketType, Markets, OHLCV, OrderBook, Precision, Ticker,
    Tickers, Trade,
};

pub const ID: &str = "kucoin";
const BASE_URL: &str = "https://api.kucoin.com";
const RATE_LIMIT_MS: u64 = 50;

/// kucoin 现货适配器。
pub struct Kucoin {
    config: Config,
    core: HttpCore,
}

impl Kucoin {
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
        let core = HttpCore::new(&config, BASE_URL, RATE_LIMIT_MS, "kucoin")?;
        Ok(Self { config, core })
    }

    // ================= 内部 HTTP =================

    /// 私密 GET(v1 签名)。
    async fn private_get(&self, path: &str, params: &Params) -> Result<Value> {
        let api_key = crate::signing::require_api_key(&self.config, "kucoin")?;
        let secret = crate::signing::require_secret(&self.config, "kucoin")?;
        let passphrase = crate::signing::require_passphrase(&self.config, "kucoin")?;
        let timestamp = now_ms().to_string();
        let qs = query_string(params);
        let auth = format!("{timestamp}GET{path}{qs}");
        let signature = crate::signing::hmac_sha256_b64(secret, &auth);
        let mut headers = HeaderMap::new();
        crate::signing::set_header(&mut headers, "KC-API-KEY", api_key)?;
        crate::signing::set_header(&mut headers, "KC-API-SIGN", &signature)?;
        crate::signing::set_header(&mut headers, "KC-API-TIMESTAMP", &timestamp)?;
        crate::signing::set_header(&mut headers, "KC-API-PASSPHRASE", passphrase)?;
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
            .public_get("/api/v2/symbols", &Params::new())
            .await?;
        let arr = resp
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| Error::new(ErrorKind::BadResponse, "symbols not array"))?;
        let mut map = Markets::new();
        for raw in arr {
            let m = self.parse_market(raw);
            map.insert(m.symbol.clone(), m);
        }
        Ok(map)
    }

    /// 统一 symbol → id(`BTC/USDT` → `BTC-USDT`)。
    fn symbol_id(&self, symbol: &str) -> String {
        symbol.replace('/', "-")
    }

    /// id → 统一 symbol(`BTC-USDT` → `BTC/USDT`):先查缓存,否则按 `-` 拆分。
    pub fn market_symbol(&self, id: &str) -> String {
        if let Some(m) = self.core.markets_snapshot().values().find(|m| m.id == id) {
            return m.symbol.clone();
        }
        match id.split_once('-') {
            Some((b, q)) if !b.is_empty() && !q.is_empty() => {
                format!("{}/{}", b.to_uppercase(), q.to_uppercase())
            }
            _ => id.to_string(),
        }
    }

    // ================= parse(公开,供差分测试) =================

    pub fn parse_market(&self, raw: &Value) -> Market {
        let id = raw["symbol"].as_str().unwrap_or_default();
        let (base, quote) = match id.split_once('-') {
            Some((b, q)) => (b.to_string(), q.to_string()),
            None => (String::new(), String::new()),
        };
        Market {
            id: id.to_string(),
            symbol: format!("{}/{}", base.to_uppercase(), quote.to_uppercase()),
            base: Some(base.to_uppercase()),
            quote: Some(quote.to_uppercase()),
            base_id: Some(base),
            quote_id: Some(quote),
            active: Some(raw["enableTrading"].as_bool().unwrap_or(false)),
            market_type: Some(MarketType::Spot),
            spot: Some(true),
            // TICK_SIZE:increment 字符串即最小档位(price 用 priceIncrement,回退 quoteIncrement)
            precision: Precision {
                price: dec(raw.get("priceIncrement")).or_else(|| dec(raw.get("quoteIncrement"))),
                amount: dec(raw.get("baseIncrement")),
                cost: None,
            },
            info: raw.clone(),
            ..Market::default()
        }
    }

    pub fn parse_ticker(&self, raw: &Value) -> Ticker {
        let id = raw["symbol"].as_str().unwrap_or_default();
        let ts = raw["time"].as_i64();
        let close = dec(raw.get("last"));
        // percentage:changeRate 为小数(如 -0.0037)→ ×100
        let percentage = raw
            .get("changeRate")
            .and_then(value_decimal)
            .map(|c| c * rust_decimal::Decimal::new(100, 0));
        Ticker {
            symbol: self.market_symbol(id),
            timestamp: ts,
            datetime: ts.and_then(iso8601),
            high: dec(raw.get("high")),
            low: dec(raw.get("low")),
            bid: dec(raw.get("buy")).or_else(|| dec(raw.get("bestBid"))),
            ask: dec(raw.get("sell")).or_else(|| dec(raw.get("bestAsk"))),
            bid_volume: dec(raw.get("bestBidSize")),
            ask_volume: dec(raw.get("bestAskSize")),
            open: dec(raw.get("open")),
            close,
            last: close,
            change: dec(raw.get("changePrice")),
            percentage,
            average: dec(raw.get("averagePrice")),
            base_volume: dec(raw.get("vol")),
            quote_volume: dec(raw.get("volValue")),
            info: raw.clone(),
            ..Ticker::default()
        }
    }

    pub fn parse_ohlcv(&self, row: &Value) -> OHLCV {
        let arr = row.as_array();
        // [ts秒, open, close, high, low, volume, turnover];ts ≤10 位 → ×1000
        let ts = arr
            .and_then(|a| a.first())
            .and_then(value_str)
            .and_then(|s| s.parse::<i64>().ok())
            .map(|t| if t <= 10_000_000_000 { t * 1000 } else { t });
        OHLCV {
            timestamp: ts,
            open: arr.and_then(|a| a.get(1)).and_then(value_decimal),
            close: arr.and_then(|a| a.get(2)).and_then(value_decimal),
            high: arr.and_then(|a| a.get(3)).and_then(value_decimal),
            low: arr.and_then(|a| a.get(4)).and_then(value_decimal),
            volume: arr.and_then(|a| a.get(5)).and_then(value_decimal),
        }
    }

    pub fn parse_trade(&self, raw: &Value) -> Trade {
        // time 为纳秒 → ccxt 统一 /1e6 转毫秒
        let timestamp = raw["time"].as_i64().map(|t| t / 1_000_000);
        let price = dec(raw.get("price"));
        let amount = dec(raw.get("size"));
        let id = match raw.get("tradeId") {
            Some(Value::String(s)) => Some(s.clone()),
            Some(Value::Number(n)) => Some(n.to_string()),
            _ => None,
        };
        Trade {
            id,
            timestamp,
            datetime: timestamp.and_then(iso8601),
            symbol: raw["symbol"].as_str().map(|s| self.market_symbol(s)),
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
        let data = raw.get("data").unwrap_or(raw);
        let mut bids: Vec<Level> = data
            .get("bids")
            .and_then(Value::as_array)
            .map(|a| a.iter().map(parse_level).collect())
            .unwrap_or_default();
        let mut asks: Vec<Level> = data
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
            timestamp: data["time"].as_i64(),
            nonce: data["sequence"].as_i64(),
            info: data.clone(),
            ..OrderBook::default()
        }
    }
}

// ================= Exchange trait =================

impl Exchange for Kucoin {
    fn id(&self) -> &'static str {
        ID
    }
    fn config(&self) -> &Config {
        &self.config
    }

    async fn fetch_time(&self) -> Result<i64> {
        let resp = self
            .core
            .public_get("/api/v1/timestamp", &Params::new())
            .await?;
        resp["data"]
            .as_i64()
            .ok_or_else(|| Error::new(ErrorKind::BadResponse, "missing timestamp"))
    }

    async fn fetch_markets(&self) -> Result<Vec<Market>> {
        self.load_markets().await?;
        Ok(self.core.markets_snapshot().into_values().collect())
    }

    async fn fetch_ticker(&self, symbol: &str, _params: Params) -> Result<Ticker> {
        let p = params1("symbol", &self.symbol_id(symbol));
        let resp = self.core.public_get("/api/v1/market/stats", &p).await?;
        let raw = resp
            .get("data")
            .ok_or_else(|| Error::new(ErrorKind::BadResponse, "missing data"))?;
        Ok(self.parse_ticker(raw))
    }

    async fn fetch_tickers(&self, _symbols: Option<&[&str]>, _params: Params) -> Result<Tickers> {
        let resp = self
            .core
            .public_get("/api/v1/market/allTickers", &Params::new())
            .await?;
        let arr = resp
            .get("data")
            .and_then(|d| d.get("ticker"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
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
        _limit: Option<i64>,
        _params: Params,
    ) -> Result<Vec<OHLCV>> {
        let mut p = Params::new();
        p.insert("symbol".into(), json!(self.symbol_id(symbol)));
        p.insert("type".into(), json!(timeframe));
        let resp = self.core.public_get("/api/v1/market/candles", &p).await?;
        let arr = resp
            .get("data")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(arr.iter().map(|r| self.parse_ohlcv(r)).collect())
    }

    async fn fetch_order_book(
        &self,
        symbol: &str,
        _limit: Option<i64>,
        _params: Params,
    ) -> Result<OrderBook> {
        let p = params1("symbol", &self.symbol_id(symbol));
        // level2_20:20 档快照(公共端点限制)
        let resp = self
            .core
            .public_get("/api/v1/market/orderbook/level2_20", &p)
            .await?;
        Ok(self.parse_order_book(&resp, symbol))
    }

    async fn fetch_trades(
        &self,
        symbol: &str,
        _since: Option<i64>,
        _limit: Option<i64>,
        _params: Params,
    ) -> Result<Vec<Trade>> {
        let p = params1("symbol", &self.symbol_id(symbol));
        let resp = self.core.public_get("/api/v1/market/histories", &p).await?;
        let arr = resp
            .get("data")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(arr.iter().map(|t| self.parse_trade(t)).collect())
    }

    async fn fetch_balance(&self, _params: Params) -> Result<Balances> {
        let resp = self.private_get("/api/v1/accounts", &Params::new()).await?;
        let mut out = Balances {
            info: resp.clone(),
            ..Balances::default()
        };
        if let Some(arr) = resp.get("data").and_then(Value::as_array) {
            for a in arr {
                let acc_type = a["type"].as_str().unwrap_or_default();
                if acc_type != "trade" && acc_type != "main" {
                    continue;
                }
                if let Some(code) = a["currency"].as_str() {
                    let free = dec(a.get("available"));
                    let holds = dec(a.get("holds"));
                    let balance = dec(a.get("balance"));
                    let entry = out
                        .accounts
                        .entry(code.to_string())
                        .or_insert_with(|| Balance {
                            ..Balance::default()
                        });
                    if acc_type == "trade" {
                        entry.free = free;
                        entry.used = holds;
                        entry.total = balance;
                    } else if entry.free.is_none() {
                        // main 账户仅在 trade 账户缺失时兜底
                        entry.free = free;
                        entry.used = holds;
                        entry.total = balance;
                    }
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

fn value_str(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbol_id_converts() {
        let ex = Kucoin::new(Config::new()).unwrap();
        assert_eq!(ex.symbol_id("BTC/USDT"), "BTC-USDT");
        assert_eq!(ex.market_symbol("BTC-USDT"), "BTC/USDT");
    }

    #[test]
    fn parse_market_spot() {
        let ex = Kucoin::new(Config::new()).unwrap();
        let raw = json!({
            "symbol": "BTC-USDT",
            "baseCurrency": "BTC",
            "quoteCurrency": "USDT",
            "baseIncrement": "0.000001",
            "quoteIncrement": "0.01",
            "enableTrading": true
        });
        let m = ex.parse_market(&raw);
        assert_eq!(m.symbol, "BTC/USDT");
        assert_eq!(m.active, Some(true));
        assert_eq!(m.precision.price, Some("0.01".parse().unwrap()));
        assert_eq!(m.precision.amount, Some("0.000001".parse().unwrap()));
    }

    #[test]
    fn parse_ticker_percentage_scaled() {
        let ex = Kucoin::new(Config::new()).unwrap();
        let raw = json!({
            "symbol": "BTC-USDT",
            "last": "63820.2",
            "buy": "63820.1",
            "sell": "63820.2",
            "changeRate": "-0.0037",
            "changePrice": "-241.4",
            "high": "64494.3",
            "low": "63237.4",
            "vol": "1665.13448497935070901736",
            "volValue": "106243224.35525068102968015498",
            "averagePrice": "64046.97496029",
            "time": 1786523071399_i64
        });
        let t = ex.parse_ticker(&raw);
        assert_eq!(t.symbol, "BTC/USDT");
        assert_eq!(t.last, Some("63820.2".parse().unwrap()));
        assert_eq!(t.bid, Some("63820.1".parse().unwrap()));
        assert_eq!(t.percentage, Some("-0.37".parse().unwrap()));
    }

    #[test]
    fn parse_ohlcv_close_before_high() {
        let ex = Kucoin::new(Config::new()).unwrap();
        let row = json!([
            "1786523040",
            "63820.2",
            "63820.2",
            "63820.3",
            "63818.7",
            "0.27859153",
            "17779.658162723"
        ]);
        let c = ex.parse_ohlcv(&row);
        assert_eq!(c.timestamp, Some(1786523040000));
        assert_eq!(c.close, Some("63820.2".parse().unwrap()));
        assert_eq!(c.high, Some("63820.3".parse().unwrap()));
        assert_eq!(c.low, Some("63818.7".parse().unwrap()));
        assert_eq!(c.volume, Some("0.27859153".parse().unwrap()));
    }

    #[test]
    fn parse_trade_nanosecond_time() {
        let ex = Kucoin::new(Config::new()).unwrap();
        let raw = json!({
            "sequence": "23896030947983360",
            "tradeId": "23896030947983360",
            "price": "63820.2",
            "size": "0.00023516",
            "side": "sell",
            "time": 1786523048787000000_u64
        });
        let t = ex.parse_trade(&raw);
        assert_eq!(t.id.as_deref(), Some("23896030947983360"));
        assert_eq!(t.side.as_deref(), Some("sell"));
        assert_eq!(t.timestamp, Some(1786523048787));
    }
}
