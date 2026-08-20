//! lighter 适配器(ADR-0017,由 generated promote 为 curated)。
//!
//! Lighter 是 zk-rollup 永续/现货 DEX,公开行情走 REST `https://mainnet.zklighter.elliot.ai`。
//! 本 curated 适配器实现**完整公开行情面**(markets / ticker / order book / OHLCV)。
//!
//! **下单与私密读被故意留作 `NotSupported`**,原因:
//! Lighter 的订单签名是 zk 原生方案——**Schnorr over Goldilocks(ECgFp5)曲线 +
//! Poseidon2 哈希**(40 字节私钥,非 secp256k1 / 非 EIP-712),签名依赖官方原生
//! `lighter-signer` 共享库(libsecp256k1 不适用于此)。ccxt 自身也把签名委托给该原生库。
//! 在纯 Rust 中复现需移植整套 `poseidon_crypto` 密码学,超出本适配器范围。待引入
//! 原生 signer 绑定或移植该密码学后,再补全 `create_order` / `cancel_order` /
//! `fetch_balance` / `fetch_positions` 等私密面(照搬 polymarket 的 EIP-712 方案是**错误**的)。

use reqwest::header::HeaderMap;
use serde_json::{Value, json};

use crate::error::{Error, ErrorKind, Result};
use crate::exchange::{Config, Exchange, Params};
use crate::httpcore::{HttpCore, dec, query_string};
use crate::types::{Market, MarketType, Markets, OHLCV, OrderBook, Precision, Ticker, Tickers};

pub const ID: &str = "lighter";
const BASE_URL: &str = "https://mainnet.zklighter.elliot.ai";
const RATE_LIMIT_MS: u64 = 50;

pub struct Lighter {
    config: Config,
    core: HttpCore,
}

impl Lighter {
    /// 已实现(非 NotSupported)的 REST 方法面(契约声明,ADR-0017)。
    /// 仅公开行情面;下单/私密读见文件头注释,当前为 `NotSupported`。
    pub const IMPLEMENTED: &'static [&'static str] = &[
        "fetch_markets",
        "fetch_ticker",
        "fetch_tickers",
        "fetch_order_book",
        "fetch_ohlcv",
    ];

    pub fn new(config: Config) -> Result<Self> {
        let core = HttpCore::new(&config, BASE_URL, RATE_LIMIT_MS, "lighter")?;
        Ok(Self { config, core })
    }

    async fn public_request(&self, path: &str, params: &Params) -> Result<Value> {
        let url = format!("{BASE_URL}{path}{}", query_string(params));
        self.core
            .request_url("GET", &url, &HeaderMap::new(), None)
            .await
    }

    /// 由整数 market_id 反查统一 symbol(命中缓存优先)。
    fn symbol_of(&self, market_id: &str) -> String {
        if let Some(m) = self
            .core
            .markets_snapshot()
            .values()
            .find(|m| m.id == market_id)
        {
            return m.symbol.clone();
        }
        market_id.to_string()
    }

    async fn fetch_markets_raw(&self) -> Result<Markets> {
        let resp = self
            .public_request("/orderBookDetails", &Params::new())
            .await?;
        let mut map = Markets::new();
        let perps = resp.get("order_book_details").and_then(Value::as_array);
        let spots = resp
            .get("spot_order_book_details")
            .and_then(Value::as_array);
        for group in [perps, spots].into_iter().flatten() {
            for raw in group {
                let market_id = raw["market_id"].as_i64().map(|v| v.to_string());
                let market_id = match market_id {
                    Some(id) => id,
                    None => continue,
                };
                let raw_symbol = raw["symbol"].as_str().unwrap_or_default();
                let is_spot = raw["market_type"].as_str() == Some("spot");
                let (symbol, base, quote, mtype, spot, swap) = if is_spot {
                    let (b, q) = match raw_symbol.split_once('/') {
                        Some((b, q)) => (b.to_string(), q.to_string()),
                        None => (raw_symbol.to_string(), "USDC".to_string()),
                    };
                    (raw_symbol.to_string(), b, q, MarketType::Spot, true, false)
                } else {
                    // perp 以 USDC 结算,统一 symbol 写作 {symbol}/USDC
                    (
                        format!("{raw_symbol}/USDC"),
                        raw_symbol.to_string(),
                        "USDC".to_string(),
                        MarketType::Swap,
                        false,
                        true,
                    )
                };
                let price_dec = raw["price_decimals"].as_u64().unwrap_or(0);
                let size_dec = raw["size_decimals"].as_u64().unwrap_or(0);
                let tick = |d: u64| -> Option<rust_decimal::Decimal> {
                    if d == 0 {
                        return None;
                    }
                    let denom = 10u64.checked_pow(d as u32)?;
                    rust_decimal::Decimal::from(1u8).checked_div(rust_decimal::Decimal::from(denom))
                };
                map.insert(
                    symbol.clone(),
                    Market {
                        id: market_id,
                        symbol,
                        base: Some(base),
                        quote: Some(quote),
                        base_id: raw["base_asset_id"].as_i64().map(|v| v.to_string()),
                        quote_id: raw["quote_asset_id"].as_i64().map(|v| v.to_string()),
                        active: Some(raw["status"].as_str() == Some("active")),
                        market_type: Some(mtype),
                        spot: Some(spot),
                        swap: Some(swap),
                        settle: if swap { Some("USDC".to_string()) } else { None },
                        linear: if swap { Some(true) } else { None },
                        contract_size: if swap {
                            Some(rust_decimal::Decimal::ONE)
                        } else {
                            None
                        },
                        taker: raw["taker_fee"].as_str().and_then(|s| s.parse().ok()),
                        maker: raw["maker_fee"].as_str().and_then(|s| s.parse().ok()),
                        precision: Precision {
                            price: tick(price_dec),
                            amount: tick(size_dec),
                            cost: None,
                        },
                        info: raw.clone(),
                        ..Market::default()
                    },
                );
            }
        }
        Ok(map)
    }

    pub fn parse_ticker(&self, raw: &Value) -> Ticker {
        Ticker {
            symbol: self.symbol_of(
                raw["market_id"]
                    .as_i64()
                    .map(|v| v.to_string())
                    .as_deref()
                    .unwrap_or(""),
            ),
            last: dec(raw.get("last_trade_price")),
            high: dec(raw.get("daily_price_high")),
            low: dec(raw.get("daily_price_low")),
            change: dec(raw.get("daily_price_change")),
            base_volume: dec(raw.get("daily_base_token_volume")),
            quote_volume: dec(raw.get("daily_quote_token_volume")),
            info: raw.clone(),
            ..Ticker::default()
        }
    }

    pub fn parse_order_book(&self, raw: &Value, market_id: &str) -> OrderBook {
        let bids = raw["bids"]
            .as_array()
            .map(|a| a.iter().map(parse_book_level).collect())
            .unwrap_or_default();
        let asks = raw["asks"]
            .as_array()
            .map(|a| a.iter().map(parse_book_level).collect())
            .unwrap_or_default();
        OrderBook {
            symbol: self.symbol_of(market_id),
            bids,
            asks,
            info: raw.clone(),
            ..OrderBook::default()
        }
    }

    pub fn parse_ohlcv(&self, raw: &Value) -> OHLCV {
        OHLCV {
            timestamp: raw["t"].as_i64(),
            open: dec(raw.get("o")),
            high: dec(raw.get("h")),
            low: dec(raw.get("l")),
            close: dec(raw.get("c")),
            volume: dec(raw.get("v")),
        }
    }
}

impl Exchange for Lighter {
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
        self.core.load_markets(|| self.fetch_markets_raw()).await?;
        let id = self
            .core
            .markets_snapshot()
            .get(symbol)
            .map(|m| m.id.clone())
            .ok_or_else(|| Error::new(ErrorKind::BadSymbol, format!("unknown symbol {symbol}")))?;
        let mut p = Params::new();
        p.insert("market_id".into(), json!(id));
        let resp = self.public_request("/orderBookDetails", &p).await?;
        let first = resp
            .get("order_book_details")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .or_else(|| {
                resp.get("spot_order_book_details")
                    .and_then(Value::as_array)
                    .and_then(|a| a.first())
            })
            .ok_or_else(|| Error::new(ErrorKind::BadResponse, "ticker empty"))?;
        Ok(self.parse_ticker(first))
    }

    async fn fetch_tickers(&self, _symbols: Option<&[&str]>, _params: Params) -> Result<Tickers> {
        self.core.load_markets(|| self.fetch_markets_raw()).await?;
        let mut out = Tickers::new();
        for m in self.core.markets_snapshot().values() {
            let mut p = Params::new();
            p.insert("market_id".into(), json!(m.id.clone()));
            if let Ok(resp) = self.public_request("/orderBookDetails", &p).await {
                if let Some(first) = resp
                    .get("order_book_details")
                    .and_then(Value::as_array)
                    .and_then(|a| a.first())
                    .or_else(|| {
                        resp.get("spot_order_book_details")
                            .and_then(Value::as_array)
                            .and_then(|a| a.first())
                    })
                {
                    out.insert(m.symbol.clone(), self.parse_ticker(first));
                }
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
        self.core.load_markets(|| self.fetch_markets_raw()).await?;
        let id = self
            .core
            .markets_snapshot()
            .get(symbol)
            .map(|m| m.id.clone())
            .ok_or_else(|| Error::new(ErrorKind::BadSymbol, format!("unknown symbol {symbol}")))?;
        let mut p = Params::new();
        p.insert("market_id".into(), json!(id));
        p.insert("limit".into(), json!(limit.unwrap_or(100).min(100)));
        let resp = self.public_request("/orderBookOrders", &p).await?;
        Ok(self.parse_order_book(&resp, &id))
    }

    async fn fetch_ohlcv(
        &self,
        symbol: &str,
        timeframe: &str,
        since: Option<i64>,
        limit: Option<i64>,
        _params: Params,
    ) -> Result<Vec<OHLCV>> {
        self.core.load_markets(|| self.fetch_markets_raw()).await?;
        let id = self
            .core
            .markets_snapshot()
            .get(symbol)
            .map(|m| m.id.clone())
            .ok_or_else(|| Error::new(ErrorKind::BadSymbol, format!("unknown symbol {symbol}")))?;
        let tf = match timeframe {
            "1m" => "1m",
            "5m" => "5m",
            "15m" => "15m",
            "30m" => "30m",
            "1h" => "1h",
            "4h" => "4h",
            "12h" => "12h",
            "1d" => "1d",
            "1w" => "1w",
            other => {
                return Err(Error::new(
                    ErrorKind::BadRequest,
                    format!("unsupported timeframe {other}"),
                ));
            }
        };
        let now = chrono::Utc::now().timestamp_millis();
        let step = tf_ms(timeframe);
        let (start, end) = match since {
            Some(s) => (s, s + step * limit.unwrap_or(100)),
            None => {
                let l = limit.unwrap_or(100);
                (now - step * l, now)
            }
        };
        let mut p = Params::new();
        p.insert("market_id".into(), json!(id));
        p.insert("resolution".into(), json!(tf));
        p.insert("start_timestamp".into(), json!(start));
        p.insert("end_timestamp".into(), json!(end));
        p.insert("count_back".into(), json!(0));
        let resp = self.public_request("/candles", &p).await?;
        let arr = resp
            .get("c")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(arr.iter().map(|c| self.parse_ohlcv(c)).collect())
    }
}

// ---------- 工具 ----------

/// 解析 Lighter 簿档位(`{price, remaining_base_amount, ...}` → `Level`)。
fn parse_book_level(raw: &Value) -> crate::types::Level {
    crate::types::Level {
        price: dec(raw.get("price")),
        amount: dec(raw.get("remaining_base_amount")),
    }
}
fn tf_ms(timeframe: &str) -> i64 {
    match timeframe {
        "1m" => 60_000i64,
        "5m" => 300_000,
        "15m" => 900_000,
        "30m" => 1_800_000,
        "1h" => 3_600_000,
        "4h" => 14_400_000,
        "12h" => 43_200_000,
        "1d" => 86_400_000,
        "1w" => 604_800_000,
        _ => 3_600_000,
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tf_ms_known() {
        assert_eq!(tf_ms("1m"), 60_000);
        assert_eq!(tf_ms("1h"), 3_600_000);
        assert_eq!(tf_ms("1d"), 86_400_000);
    }

    #[test]
    fn parse_ohlcv_fields() {
        let ex = Lighter::new(Config::new()).unwrap();
        let raw = json!({"t": 123, "o": "1", "h": "2", "l": "0.5", "c": "1.5", "v": "10"});
        let c = ex.parse_ohlcv(&raw);
        assert_eq!(c.timestamp, Some(123));
        assert_eq!(c.open, Some("1".parse().unwrap()));
        assert_eq!(c.close, Some("1.5".parse().unwrap()));
        assert_eq!(c.volume, Some("10".parse().unwrap()));
    }

    #[test]
    fn parse_order_book_levels() {
        let ex = Lighter::new(Config::new()).unwrap();
        let raw = json!({"bids": [{"price": "100", "remaining_base_amount": "1.5"}], "asks": [{"price": "101", "remaining_base_amount": "2"}]});
        let book = ex.parse_order_book(&raw, "0");
        assert_eq!(book.bids.len(), 1);
        assert_eq!(book.bids[0].price, Some("100".parse().unwrap()));
        assert_eq!(book.bids[0].amount, Some("1.5".parse().unwrap()));
        assert_eq!(book.asks[0].price, Some("101".parse().unwrap()));
    }
}
