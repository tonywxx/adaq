//! mexc 现货适配器(Phase B,ADR-0005)。
//!
//! 对齐 ccxt mexc(v3)语义:
//! - 公共面 `GET /api/v3/...`(与 binance v3 同构):exchangeInfo / ticker/24hr /
//!   klines / depth / trades;symbol id 为 `BTCUSDT`;
//! - ticker `priceChangePercent` 为小数(如 -0.0038)→ percentage ×100;
//! - market `active = status == '1' && isSpotTradingAllowed`;
//!   precision = 10^-baseAssetPrecision / 10^-quoteAssetPrecision;
//! - 私密面:binance 式 `timestamp+recvWindow` query + HMAC-SHA256 签名,
//!   `X-MEXC-APIKEY` 头;余额 `free/locked`;
//! - rateLimit 20ms。
//!
//! 参考实现基于 ccxt(MIT)适配器解析逻辑,见 `NOTICE`。

use reqwest::header::HeaderMap;
use serde_json::{Value, json};

use crate::error::{Error, ErrorKind, Result};
use crate::exchange::{Config, Exchange, Params};
use crate::httpcore::{
    HttpCore, dec, iso8601, now_ms, parse_level, parse_ohlcv_standard, query_string, value_decimal,
};
use crate::types::{
    Balance, Balances, Market, MarketType, Markets, OHLCV, OrderBook, Precision, Ticker, Tickers,
    Trade,
};

pub const ID: &str = "mexc";
const BASE_URL: &str = "https://api.mexc.com/api/v3";
const RATE_LIMIT_MS: u64 = 20;

/// 常见计价币(用于无缓存时推导统一 symbol)。
const COMMON_QUOTES: &[&str] = &[
    "USDT", "USDC", "FDUSD", "TUSD", "BUSD", "DAI", "EUR", "USD", "BTC", "ETH",
];

/// mexc 现货适配器。
pub struct Mexc {
    config: Config,
    core: HttpCore,
}

impl Mexc {
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
        let core = HttpCore::new(&config, BASE_URL, RATE_LIMIT_MS, "mexc")?;
        Ok(Self { config, core })
    }

    // ================= 内部 HTTP =================

    async fn public_get(&self, path: &str, params: &Params) -> Result<Value> {
        self.core.public_get(path, params).await
    }

    /// 私密 GET(binance 式签名)。
    async fn private_get(&self, path: &str, params: &Params) -> Result<Value> {
        let api_key = crate::signing::require_api_key(&self.config, "mexc")?;
        let secret = crate::signing::require_secret(&self.config, "mexc")?;
        let mut p = params.clone();
        p.insert("timestamp".into(), json!(now_ms()));
        p.insert("recvWindow".into(), json!(5000));
        let qs = query_string(&p);
        let signature = crate::signing::hmac_sha256_hex(secret, qs.trim_start_matches('?'));
        let url = format!("{BASE_URL}{path}{qs}&signature={signature}");
        let mut headers = HeaderMap::new();
        crate::signing::set_header(&mut headers, "X-MEXC-APIKEY", api_key)?;
        self.core.request_url("GET", &url, &headers, None).await
    }

    // ================= markets 缓存 =================

    async fn load_markets(&self) -> Result<()> {
        self.core.load_markets(|| self.fetch_markets_raw()).await
    }

    async fn fetch_markets_raw(&self) -> Result<Markets> {
        let resp = self.public_get("/exchangeInfo", &Params::new()).await?;
        let arr = resp
            .get("symbols")
            .and_then(Value::as_array)
            .ok_or_else(|| Error::new(ErrorKind::BadResponse, "symbols not array"))?;
        let mut map = Markets::new();
        for raw in arr {
            let m = self.parse_market(raw);
            map.insert(m.symbol.clone(), m);
        }
        Ok(map)
    }

    /// 统一 symbol → id(`BTC/USDT` → `BTCUSDT`)。
    fn symbol_id(&self, symbol: &str) -> String {
        symbol.replace('/', "")
    }

    /// id → 统一 symbol:先查缓存,否则按常见计价币后缀剥离。
    fn market_symbol(&self, id: &str) -> String {
        if let Some(m) = self.core.markets_snapshot().values().find(|m| m.id == id) {
            return m.symbol.clone();
        }
        for q in COMMON_QUOTES {
            if let Some(base) = id.strip_suffix(q) {
                if !base.is_empty() {
                    return format!("{}/{}", base.to_uppercase(), q);
                }
            }
        }
        id.to_string()
    }

    // ================= parse(公开,供差分测试) =================

    pub fn parse_market(&self, raw: &Value) -> Market {
        let id = raw["symbol"].as_str().unwrap_or_default();
        let base = raw["baseAsset"].as_str().unwrap_or_default();
        let quote = raw["quoteAsset"].as_str().unwrap_or_default();
        let active = raw["status"].as_str() == Some("1")
            && raw["isSpotTradingAllowed"].as_bool().unwrap_or(false);
        let precision = |scale: Option<&Value>| {
            scale
                .and_then(|v| match v {
                    Value::Number(n) => n.as_u64().and_then(|n| u32::try_from(n).ok()),
                    Value::String(s) => s.parse::<u32>().ok(),
                    _ => None,
                })
                .map(|n| rust_decimal::Decimal::new(1, n))
        };
        Market {
            id: id.to_string(),
            symbol: format!("{}/{}", base.to_uppercase(), quote.to_uppercase()),
            base: Some(base.to_uppercase()),
            quote: Some(quote.to_uppercase()),
            base_id: Some(base.to_string()),
            quote_id: Some(quote.to_string()),
            active: Some(active),
            market_type: Some(MarketType::Spot),
            spot: Some(true),
            taker: dec(raw.get("takerCommission")),
            maker: dec(raw.get("makerCommission")),
            precision: Precision {
                amount: precision(raw.get("baseAssetPrecision")),
                price: precision(raw.get("quoteAssetPrecision")),
                cost: None,
            },
            limits: crate::types::Limits {
                amount: Some(crate::types::Limit {
                    min: dec(raw.get("baseSizePrecision")),
                    max: None,
                }),
                cost: Some(crate::types::Limit {
                    min: dec(raw.get("quoteAmountPrecision")),
                    max: dec(raw.get("maxQuoteAmount")),
                }),
                ..crate::types::Limits::default()
            },
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
        // priceChangePercent 为小数(如 -0.0038)→ ×100
        let percentage = raw
            .get("priceChangePercent")
            .and_then(value_decimal)
            .map(|c| c * rust_decimal::Decimal::new(100, 0));
        Ticker {
            symbol: self.market_symbol(raw["symbol"].as_str().unwrap_or_default()),
            timestamp,
            datetime: timestamp.and_then(iso8601),
            high: dec(raw.get("highPrice")),
            low: dec(raw.get("lowPrice")),
            bid: dec(raw.get("bidPrice")),
            ask: dec(raw.get("askPrice")),
            bid_volume: dec(raw.get("bidQty")),
            ask_volume: dec(raw.get("askQty")),
            open: open_price,
            close: close_price,
            last: close_price,
            previous_close: dec(raw.get("prevClosePrice")),
            change: dec(raw.get("priceChange")),
            percentage,
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
        let price = dec(raw.get("price"));
        let amount = dec(raw.get("qty"));
        Trade {
            id: raw["id"].as_str().map(str::to_string),
            info: raw.clone(),
            timestamp,
            datetime: timestamp.and_then(iso8601),
            symbol: raw["symbol"].as_str().map(|s| self.market_symbol(s)),
            side: Some(if buyer_maker { "sell" } else { "buy" }.to_string()),
            taker_or_maker: Some("taker".to_string()),
            price,
            amount,
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
}

// ================= Exchange trait =================

impl Exchange for Mexc {
    fn id(&self) -> &'static str {
        ID
    }
    fn config(&self) -> &Config {
        &self.config
    }

    async fn fetch_time(&self) -> Result<i64> {
        let resp = self.public_get("/time", &Params::new()).await?;
        resp["serverTime"]
            .as_i64()
            .ok_or_else(|| Error::new(ErrorKind::BadResponse, "missing serverTime"))
    }

    async fn fetch_markets(&self) -> Result<Vec<Market>> {
        self.load_markets().await?;
        Ok(self.core.markets_snapshot().into_values().collect())
    }

    async fn fetch_ticker(&self, symbol: &str, _params: Params) -> Result<Ticker> {
        let p = params1("symbol", &self.symbol_id(symbol));
        let resp = self.public_get("/ticker/24hr", &p).await?;
        Ok(self.parse_ticker(&resp))
    }

    async fn fetch_tickers(&self, symbols: Option<&[&str]>, _params: Params) -> Result<Tickers> {
        let resp = self.public_get("/ticker/24hr", &Params::new()).await?;
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

    async fn fetch_bids_asks(&self, symbols: Option<&[&str]>, _params: Params) -> Result<Tickers> {
        let resp = self
            .public_get("/ticker/bookTicker", &Params::new())
            .await?;
        let arr = resp.as_array().cloned().unwrap_or_default();
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
        _since: Option<i64>,
        limit: Option<i64>,
        _params: Params,
    ) -> Result<Vec<OHLCV>> {
        let mut p = Params::new();
        p.insert("symbol".into(), json!(self.symbol_id(symbol)));
        p.insert("interval".into(), json!(timeframe));
        if let Some(l) = limit {
            p.insert("limit".into(), json!(l));
        }
        let resp = self.public_get("/klines", &p).await?;
        let arr = resp.as_array().cloned().unwrap_or_default();
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
            p.insert("limit".into(), json!(l.min(5000)));
        }
        let resp = self.public_get("/depth", &p).await?;
        Ok(self.parse_order_book(&resp, &id))
    }

    async fn fetch_trades(
        &self,
        symbol: &str,
        _since: Option<i64>,
        limit: Option<i64>,
        _params: Params,
    ) -> Result<Vec<Trade>> {
        let mut p = Params::new();
        p.insert("symbol".into(), json!(self.symbol_id(symbol)));
        if let Some(l) = limit {
            p.insert("limit".into(), json!(l.min(1000)));
        }
        let resp = self.public_get("/trades", &p).await?;
        let arr = resp.as_array().cloned().unwrap_or_default();
        Ok(arr.iter().map(|t| self.parse_trade(t)).collect())
    }

    async fn fetch_balance(&self, _params: Params) -> Result<Balances> {
        let resp = self.private_get("/account", &Params::new()).await?;
        let mut out = Balances {
            info: resp.clone(),
            ..Balances::default()
        };
        if let Some(balances) = resp["balances"].as_array() {
            for b in balances {
                if let Some(asset) = b["asset"].as_str() {
                    let free = dec(b.get("free"));
                    let locked = dec(b.get("locked"));
                    out.accounts.insert(
                        asset.to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbol_id_converts() {
        let ex = Mexc::new(Config::new()).unwrap();
        assert_eq!(ex.symbol_id("BTC/USDT"), "BTCUSDT");
        assert_eq!(ex.market_symbol("BTCUSDT"), "BTC/USDT");
    }

    #[test]
    fn parse_market_spot() {
        let ex = Mexc::new(Config::new()).unwrap();
        let raw = json!({
            "symbol": "BTCUSDT",
            "status": "1",
            "baseAsset": "BTC",
            "quoteAsset": "USDT",
            "baseAssetPrecision": 8,
            "quoteAssetPrecision": 2,
            "baseSizePrecision": "0.000001",
            "quoteAmountPrecision": "1",
            "isSpotTradingAllowed": true,
            "takerCommission": "0.0005",
            "makerCommission": "0"
        });
        let m = ex.parse_market(&raw);
        assert_eq!(m.symbol, "BTC/USDT");
        assert_eq!(m.active, Some(true));
        assert_eq!(m.precision.price, Some("0.01".parse().unwrap()));
        assert_eq!(m.precision.amount, Some("0.00000001".parse().unwrap()));
        assert_eq!(m.taker, Some("0.0005".parse().unwrap()));
    }

    #[test]
    fn parse_ticker_percentage_scaled() {
        let ex = Mexc::new(Config::new()).unwrap();
        let raw = json!({
            "symbol": "BTCUSDT",
            "lastPrice": "63851.62",
            "bidPrice": "63850.25",
            "askPrice": "63850.26",
            "openPrice": "64096.74",
            "highPrice": "64493.94",
            "lowPrice": "63250",
            "volume": "6700.85654289",
            "quoteVolume": "427877905.7",
            "priceChange": "-245.12",
            "priceChangePercent": "-0.0038",
            "prevClosePrice": "64096.74",
            "closeTime": 1786521806087_i64
        });
        let t = ex.parse_ticker(&raw);
        assert_eq!(t.symbol, "BTC/USDT");
        assert_eq!(t.last, Some("63851.62".parse().unwrap()));
        assert_eq!(t.percentage, Some("-0.38".parse().unwrap()));
        assert_eq!(t.timestamp, Some(1786521806087_i64));
    }

    #[test]
    fn parse_trade_fields() {
        let ex = Mexc::new(Config::new()).unwrap();
        let raw = json!({
            "price": "63850.52",
            "qty": "0.001459",
            "quoteQty": "93.15790868",
            "time": 1786521796722_i64,
            "isBuyerMaker": false,
            "isBestMatch": true
        });
        let t = ex.parse_trade(&raw);
        assert_eq!(t.side.as_deref(), Some("buy"));
        assert_eq!(t.timestamp, Some(1786521796722_i64));
        assert_eq!(t.cost, Some("93.15790868".parse().unwrap()));
    }

    #[test]
    fn parse_ohlcv_standard_shape() {
        let ex = Mexc::new(Config::new()).unwrap();
        let row = json!([
            1786521720000_i64,
            "63801.35",
            "63821.01",
            "63801.35",
            "63821.01",
            "2.11931179",
            1786521780000_i64,
            "135235.6"
        ]);
        let c = ex.parse_ohlcv(&row);
        assert_eq!(c.timestamp, Some(1786521720000_i64));
        assert_eq!(c.volume, Some("2.11931179".parse().unwrap()));
    }
}
