//! bitget 现货适配器(Phase B,ADR-0005)。
//!
//! 对齐 ccxt bitget(v2)语义:
//! - 公共面 `GET /api/v2/spot/{public,market}/...`,symbol id 为 `BTCUSDT`;
//! - 订单簿 `data.{asks,bids}` 为 `[price, size]` 数组,ts 字符串毫秒;
//! - trades(spot/market/fills)字段 `tradeId/price/size/side/ts`;
//! - OHLCV 为 `[ts, o, h, l, c, baseVol, quoteVol]` 数组,取 index 5 为 volume;
//! - 私密面:签名 `base64(HMAC-SHA256(secret, timestamp + method + path + body/query))`,
//!   `ACCESS-KEY/SIGN/TIMESTAMP/PASSPHRASE` 头;GET 的 query 按 key 排序后入签名;
//! - 余额(spot/account/assets):`available`→free,`frozen + locked`→used;
//! - rateLimit 20ms。
//!
//! 参考实现基于 ccxt(MIT)适配器解析逻辑,见 `NOTICE`。

use reqwest::header::HeaderMap;
use serde_json::{Value, json};

use crate::error::{Error, ErrorKind, Result};
use crate::exchange::{Config, Exchange, Params};
use crate::httpcore::{HttpCore, dec, iso8601, now_ms, parse_level, pct_encode, value_decimal};
use crate::types::{
    Balance, Balances, Level, Market, MarketType, Markets, OHLCV, OrderBook, Precision, Ticker,
    Tickers, Trade,
};

pub const ID: &str = "bitget";
const BASE_URL: &str = "https://api.bitget.com/api/v2";
const RATE_LIMIT_MS: u64 = 20;

/// 常见计价币(用于无缓存时推导统一 symbol)。
const COMMON_QUOTES: &[&str] = &[
    "USDT", "USDC", "FDUSD", "TUSD", "BUSD", "DAI", "EUR", "USD", "BTC", "ETH",
];

/// bitget 现货适配器。
pub struct Bitget {
    config: Config,
    core: HttpCore,
}

impl Bitget {
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
        let core = HttpCore::new(&config, BASE_URL, RATE_LIMIT_MS, "bitget")?;
        Ok(Self { config, core })
    }

    // ================= 内部 HTTP =================

    /// 私密 GET(v2 签名,query 按 key 排序入签名)。
    async fn private_get(&self, path: &str, params: &Params) -> Result<Value> {
        let api_key = crate::signing::require_api_key(&self.config, "bitget")?;
        let secret = crate::signing::require_secret(&self.config, "bitget")?;
        let passphrase = crate::signing::require_passphrase(&self.config, "bitget")?;
        let timestamp = now_ms().to_string();
        let qs = sorted_query_string(params);
        let auth = format!("{timestamp}GET{path}{qs}");
        let signature = crate::signing::hmac_sha256_b64(secret, &auth);
        let mut headers = HeaderMap::new();
        crate::signing::set_header(&mut headers, "ACCESS-KEY", api_key)?;
        crate::signing::set_header(&mut headers, "ACCESS-SIGN", &signature)?;
        crate::signing::set_header(&mut headers, "ACCESS-TIMESTAMP", &timestamp)?;
        crate::signing::set_header(&mut headers, "ACCESS-PASSPHRASE", passphrase)?;
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
            .public_get("/spot/public/symbols", &Params::new())
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

    /// 统一 symbol → id(`BTC/USDT` → `BTCUSDT`)。
    fn symbol_id(&self, symbol: &str) -> String {
        symbol.replace('/', "")
    }

    /// id → 统一 symbol(`BTCUSDT` → `BTC/USDT`):先查缓存,否则按常见计价币后缀剥离。
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
        // 对齐 ccxt safe_currency_code:code 大写(commonCurrencies 特例暂不处理)
        let base = raw["baseCoin"].as_str().unwrap_or_default().to_uppercase();
        let quote = raw["quoteCoin"].as_str().unwrap_or_default().to_uppercase();
        let status = raw["status"].as_str().unwrap_or_default();
        let precision = |scale: Option<&Value>| {
            scale
                .and_then(Value::as_str)
                .and_then(|s| s.parse::<u32>().ok())
                .map(|n| rust_decimal::Decimal::new(1, n))
        };
        Market {
            id: id.to_string(),
            symbol: format!("{base}/{quote}"),
            base: Some(base.clone()),
            quote: Some(quote.clone()),
            base_id: Some(base),
            quote_id: Some(quote),
            active: Some(status == "online" || status == "normal"),
            market_type: Some(MarketType::Spot),
            spot: Some(true),
            taker: raw["takerFeeRate"].as_str().and_then(|s| s.parse().ok()),
            maker: raw["makerFeeRate"].as_str().and_then(|s| s.parse().ok()),
            precision: Precision {
                price: precision(raw.get("pricePrecision")),
                amount: precision(raw.get("quantityPrecision")),
                cost: None,
            },
            limits: crate::types::Limits {
                amount: Some(crate::types::Limit {
                    min: dec(raw.get("minTradeAmount")),
                    max: dec(raw.get("maxTradeAmount")),
                }),
                cost: raw["quoteCoin"].as_str().and_then(|q| {
                    (q == "USDT").then(|| crate::types::Limit {
                        min: dec(raw.get("minTradeUSDT")),
                        max: None,
                    })
                }),
                ..crate::types::Limits::default()
            },
            info: raw.clone(),
            ..Market::default()
        }
    }

    pub fn parse_ticker(&self, raw: &Value) -> Ticker {
        let id = raw["symbol"].as_str().unwrap_or_default();
        let ts = raw["ts"].as_str().and_then(|s| s.parse::<i64>().ok());
        let close = dec(raw.get("lastPr"));
        // percentage:change24h 为小数(如 0.00321)→ *100
        let percentage = raw
            .get("change24h")
            .and_then(value_decimal)
            .map(|c| c * rust_decimal::Decimal::new(100, 0));
        Ticker {
            symbol: self.market_symbol(id),
            timestamp: ts,
            datetime: ts.and_then(iso8601),
            open: dec(raw.get("open")),
            high: dec(raw.get("high24h")),
            low: dec(raw.get("low24h")),
            bid: dec(raw.get("bidPr")),
            ask: dec(raw.get("askPr")),
            bid_volume: dec(raw.get("bidSz")),
            ask_volume: dec(raw.get("askSz")),
            close,
            last: close,
            percentage,
            base_volume: dec(raw.get("baseVolume")),
            quote_volume: dec(raw.get("quoteVolume")),
            info: raw.clone(),
            ..Ticker::default()
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
            timestamp: data["ts"].as_str().and_then(|s| s.parse::<i64>().ok()),
            info: data.clone(),
            ..OrderBook::default()
        }
    }

    pub fn parse_trade(&self, raw: &Value) -> Trade {
        let ts = raw["ts"].as_str().and_then(|s| s.parse::<i64>().ok());
        Trade {
            id: raw["tradeId"].as_str().map(str::to_string),
            timestamp: ts,
            datetime: ts.and_then(iso8601),
            symbol: raw["symbol"].as_str().map(|s| self.market_symbol(s)),
            side: raw["side"].as_str().map(|s| s.to_lowercase()),
            price: dec(raw.get("price")),
            amount: dec(raw.get("size")),
            info: raw.clone(),
            ..Trade::default()
        }
    }

    pub fn parse_ohlcv(&self, row: &Value) -> OHLCV {
        let arr = row.as_array();
        // ts 兼容 number/string(ccxt safe_integer 语义)
        let timestamp = arr.and_then(|a| a.first()).and_then(|v| match v {
            Value::Number(n) => n.as_i64(),
            Value::String(s) => s.parse().ok(),
            _ => None,
        });
        OHLCV {
            timestamp,
            open: arr.and_then(|a| a.get(1)).and_then(value_decimal),
            high: arr.and_then(|a| a.get(2)).and_then(value_decimal),
            low: arr.and_then(|a| a.get(3)).and_then(value_decimal),
            close: arr.and_then(|a| a.get(4)).and_then(value_decimal),
            volume: arr.and_then(|a| a.get(5)).and_then(value_decimal),
        }
    }
}

// ================= Exchange trait =================

impl Exchange for Bitget {
    fn id(&self) -> &'static str {
        ID
    }
    fn config(&self) -> &Config {
        &self.config
    }

    async fn fetch_time(&self) -> Result<i64> {
        let resp = self.core.public_get("/public/time", &Params::new()).await?;
        resp["data"]
            .get("serverTime")
            .and_then(Value::as_i64)
            .ok_or_else(|| Error::new(ErrorKind::BadResponse, "missing serverTime"))
    }

    async fn fetch_markets(&self) -> Result<Vec<Market>> {
        self.load_markets().await?;
        Ok(self.core.markets_snapshot().into_values().collect())
    }

    async fn fetch_ticker(&self, symbol: &str, _params: Params) -> Result<Ticker> {
        let p = params1("symbol", &self.symbol_id(symbol));
        let resp = self.core.public_get("/spot/market/tickers", &p).await?;
        let raw = resp
            .get("data")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .ok_or_else(|| Error::new(ErrorKind::BadResponse, "ticker empty"))?;
        Ok(self.parse_ticker(raw))
    }

    async fn fetch_tickers(&self, _symbols: Option<&[&str]>, _params: Params) -> Result<Tickers> {
        let resp = self
            .core
            .public_get("/spot/market/tickers", &Params::new())
            .await?;
        let arr = resp
            .get("data")
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
        limit: Option<i64>,
        _params: Params,
    ) -> Result<Vec<OHLCV>> {
        // spot granularity 映射(对齐 ccxt bitget spot timeframes)
        let granularity = match timeframe {
            "1m" => "1min",
            "3m" => "3min",
            "5m" => "5min",
            "15m" => "15min",
            "30m" => "30min",
            "1h" => "1h",
            "4h" => "4h",
            "6h" => "6Hutc",
            "12h" => "12Hutc",
            "1d" => "1Dutc",
            other => {
                return Err(Error::new(
                    ErrorKind::BadRequest,
                    format!("unsupported timeframe {other}"),
                ));
            }
        };
        let mut p = Params::new();
        p.insert("symbol".into(), json!(self.symbol_id(symbol)));
        p.insert("granularity".into(), json!(granularity));
        p.insert("limit".into(), json!(limit.unwrap_or(100).min(1000)));
        let resp = self.core.public_get("/spot/market/candles", &p).await?;
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
        limit: Option<i64>,
        _params: Params,
    ) -> Result<OrderBook> {
        let mut p = Params::new();
        p.insert("symbol".into(), json!(self.symbol_id(symbol)));
        if let Some(l) = limit {
            p.insert("limit".into(), json!(l.min(200)));
        }
        let resp = self.core.public_get("/spot/market/orderbook", &p).await?;
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
        p.insert("symbol".into(), json!(self.symbol_id(symbol)));
        if let Some(l) = limit {
            p.insert("limit".into(), json!(l.min(500)));
        }
        let resp = self.core.public_get("/spot/market/fills", &p).await?;
        let arr = resp
            .get("data")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(arr.iter().map(|t| self.parse_trade(t)).collect())
    }

    async fn fetch_balance(&self, _params: Params) -> Result<Balances> {
        let resp = self
            .private_get("/spot/account/assets", &Params::new())
            .await?;
        let mut out = Balances {
            info: resp.clone(),
            ..Balances::default()
        };
        if let Some(arr) = resp.get("data").and_then(Value::as_array) {
            for a in arr {
                if let Some(code) = a["coin"].as_str() {
                    let free = dec(a.get("available"));
                    let frozen = dec(a.get("frozen"));
                    let locked = dec(a.get("locked"));
                    let used = match (frozen, locked) {
                        (Some(f), Some(l)) => Some(f + l),
                        (Some(f), None) => Some(f),
                        (None, Some(l)) => Some(l),
                        _ => None,
                    };
                    out.accounts.insert(
                        code.to_string(),
                        Balance {
                            free,
                            used,
                            total: match (free, used) {
                                (Some(f), Some(u)) => Some(f + u),
                                (Some(f), None) => Some(f),
                                (None, Some(u)) => Some(u),
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

/// 签名用 query:按 key 排序(bitget 要求)。
fn sorted_query_string(params: &Params) -> String {
    if params.is_empty() {
        return String::new();
    }
    let mut keys: Vec<&String> = params.keys().collect();
    keys.sort();
    let pairs: Vec<String> = keys
        .iter()
        .map(|k| {
            let val = match &params[*k] {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                other => other.to_string(),
            };
            format!("{}={}", pct_encode(k), pct_encode(&val))
        })
        .collect();
    format!("?{}", pairs.join("&"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbol_id_converts() {
        let ex = Bitget::new(Config::new()).unwrap();
        assert_eq!(ex.symbol_id("BTC/USDT"), "BTCUSDT");
    }

    #[test]
    fn parse_market_spot() {
        let ex = Bitget::new(Config::new()).unwrap();
        let raw = json!({
            "symbol": "BTCUSDT",
            "baseCoin": "BTC",
            "quoteCoin": "USDT",
            "status": "online",
            "pricePrecision": "5",
            "quantityPrecision": "4",
            "minTradeUSDT": "1"
        });
        let m = ex.parse_market(&raw);
        assert_eq!(m.symbol, "BTC/USDT");
        assert_eq!(m.active, Some(true));
        assert_eq!(m.precision.price, Some("0.00001".parse().unwrap()));
        assert_eq!(m.precision.amount, Some("0.0001".parse().unwrap()));
    }

    #[test]
    fn parse_ticker_fields() {
        let ex = Bitget::new(Config::new()).unwrap();
        let raw = json!({
            "symbol": "BTCUSDT",
            "ts": "1700532903261",
            "open": "37202.46",
            "high24h": "37744.75",
            "low24h": "36666",
            "lastPr": "37583.69",
            "bidPr": "37583.68",
            "askPr": "37583.69",
            "bidSz": "0.0007",
            "askSz": "0.0829",
            "baseVolume": "13907.0386",
            "quoteVolume": "519127705.303",
            "change24h": "0.00321"
        });
        let t = ex.parse_ticker(&raw);
        assert_eq!(t.symbol, "BTC/USDT");
        assert_eq!(t.last, Some("37583.69".parse().unwrap()));
        assert_eq!(t.bid, Some("37583.68".parse().unwrap()));
        assert_eq!(t.percentage, Some("0.321".parse().unwrap()));
    }

    #[test]
    fn parse_order_book_sorted() {
        let ex = Bitget::new(Config::new()).unwrap();
        let raw = json!({
            "data": {
                "ts": "123",
                "asks": [["101", "3"], ["102", "4"]],
                "bids": [["99", "2"], ["100", "1"]]
            }
        });
        let book = ex.parse_order_book(&raw, "BTC/USDT");
        assert_eq!(book.bids[0].price, Some("100".parse().unwrap()));
        assert_eq!(book.asks[0].price, Some("101".parse().unwrap()));
    }

    #[test]
    fn parse_ohlcv_takes_index_five() {
        let ex = Bitget::new(Config::new()).unwrap();
        let row = json!([1700000000000_i64, "100", "101", "99", "100.5", "10", "1000"]);
        let c = ex.parse_ohlcv(&row);
        assert_eq!(c.timestamp, Some(1700000000000));
        assert_eq!(c.volume, Some("10".parse().unwrap()));
    }

    #[test]
    fn parse_trade_fields() {
        let ex = Bitget::new(Config::new()).unwrap();
        let raw = json!({
            "tradeId": "1075199767891652609",
            "price": "29376.5",
            "size": "6.035",
            "side": "Buy",
            "ts": "1692073521000",
            "symbol": "BTCUSDT"
        });
        let t = ex.parse_trade(&raw);
        assert_eq!(t.id.as_deref(), Some("1075199767891652609"));
        assert_eq!(t.side.as_deref(), Some("buy"));
        assert_eq!(t.price, Some("29376.5".parse().unwrap()));
        assert_eq!(t.amount, Some("6.035".parse().unwrap()));
    }
}
