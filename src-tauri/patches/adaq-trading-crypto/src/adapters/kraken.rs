//! kraken 现货适配器(M5,ADR-0005)。
//!
//! 对齐 ccxt kraken 语义:
//! - 响应信封 `{"error": [], "result": {...}}`;pair id 如 `XBTUSD`(AssetPairs 的 key);
//! - 统一 symbol `BTC/USD`(XXBT→BTC、ZUSD→USD 等 code 映射);
//! - OHLCV `[t,o,h,l,c,vwap,volume,count]` → 取 index 6 为 volume;
//! - 私密面全部 POST,body 为 urlencoded(`nonce=...&params`),签名:
//!   `base64(HMAC-SHA512(b64decode(secret), path + sha256(nonce + body)))`,
//!   `API-Key` / `API-Sign` 头;
//! - rateLimit 1000ms;taker 0.0026 / maker 0.0016。
//!
//! 参考实现基于 ccxt(MIT)适配器解析逻辑,见 `NOTICE`。

use std::collections::HashMap;
use std::sync::Mutex;

use base64::Engine;
use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::error::{Error, ErrorKind, Result};
use crate::exchange::{Config, Exchange, Params};
use crate::httpcore::{
    HttpCore, apply_client_limit, iso8601, now_ms, parse_level, pct_encode, query_string,
    value_decimal,
};
use crate::types::{
    Balance, Balances, Level, Market, MarketType, Markets, OHLCV, Order, OrderBook, Position,
    Precision, Ticker, Tickers, Trade,
};

pub const ID: &str = "kraken";
const BASE_URL: &str = "https://api.kraken.com";
const RATE_LIMIT_MS: u64 = 1000;

/// kraken 现货适配器。
pub struct Kraken {
    config: Config,
    core: HttpCore,
    /// 统一 symbol → kraken pair id(altname)。
    pair_ids: Mutex<Option<HashMap<String, String>>>,
}

impl Kraken {
    /// 已实现(非 NotSupported)的 REST 方法面(契约声明,M5)。
    pub const IMPLEMENTED: &'static [&'static str] = &[
        "fetch_markets",
        "fetch_ticker",
        "fetch_tickers",
        "fetch_order_book",
        "fetch_trades",
        "fetch_ohlcv",
        "fetch_balance",
        "fetch_positions",
        "fetch_orders",
        "fetch_open_orders",
        "create_order",
        "cancel_order",
    ];

    pub fn new(config: Config) -> Result<Self> {
        let core = HttpCore::new(&config, BASE_URL, RATE_LIMIT_MS, "kraken")?;
        Ok(Self {
            config,
            core,
            pair_ids: Mutex::new(None),
        })
    }

    // ================= 内部 HTTP =================

    async fn public_get(&self, path: &str, params: &Params) -> Result<Value> {
        let url = format!("{BASE_URL}/0/public{path}{}", query_string(params));
        let headers = HeaderMap::new();
        let resp = self.core.request_url("GET", &url, &headers, None).await?;
        ok_result(resp, path)
    }

    /// 私密请求(kraken 全部 POST,urlencoded body + HMAC-SHA512 签名)。
    ///
    /// `pub`:realtime WS token 获取等场景复用同一签名+请求路径(sign 接缝)。
    pub async fn private_post(&self, path: &str, params: &Params) -> Result<Value> {
        let api_key = self
            .config
            .api_key
            .as_deref()
            .ok_or_else(|| Error::new(ErrorKind::Authentication, "kraken api_key required"))?;
        let secret = self
            .config
            .secret
            .as_deref()
            .ok_or_else(|| Error::new(ErrorKind::Authentication, "kraken secret required"))?;
        let nonce = now_ms().to_string();
        let mut body_pairs = vec![format!("nonce={nonce}")];
        for (k, v) in params {
            let val = match v {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                other => other.to_string(),
            };
            body_pairs.push(format!("{k}={}", pct_encode(&val)));
        }
        let body = body_pairs.join("&");
        // auth = nonce + body → sha256
        let auth = format!("{nonce}{body}");
        let mut hasher = Sha256::new();
        hasher.update(auth.as_bytes());
        let auth_hash = hasher.finalize();
        // binhash = path_bytes + sha256_binary
        let path = format!("/0/private{path}");
        let mut binhash = path.as_bytes().to_vec();
        binhash.extend_from_slice(&auth_hash);
        // signature = base64(HMAC-SHA512(b64decode(secret), binhash))
        let secret_bytes = base64::engine::general_purpose::STANDARD
            .decode(secret)
            .map_err(|e| Error::new(ErrorKind::Authentication, format!("invalid secret: {e}")))?;
        let signature = crate::signing::hmac_sha512_b64_bytes(&secret_bytes, &binhash);
        let mut headers = HeaderMap::new();
        headers.insert("API-Key", HeaderValue::from_str(api_key).unwrap());
        headers.insert("API-Sign", HeaderValue::from_str(&signature).unwrap());
        headers.insert(
            "Content-Type",
            HeaderValue::from_static("application/x-www-form-urlencoded"),
        );
        let url = format!("{BASE_URL}/0/private{path}");
        let resp = self
            .core
            .request_url("POST", &url, &headers, Some(Value::String(body)))
            .await?;
        ok_result(resp, &path)
    }

    // ================= markets 缓存 =================

    async fn load_markets(&self) -> Result<()> {
        self.core.load_markets(|| self.fetch_markets_raw()).await
    }

    async fn fetch_markets_raw(&self) -> Result<Markets> {
        let resp = self.public_get("/AssetPairs", &Params::new()).await?;
        let result = resp.as_object().cloned().unwrap_or_default();
        let mut map = Markets::new();
        let mut pair_ids = HashMap::new();
        for (id, raw) in result {
            if !raw.is_object() {
                continue;
            }
            let m = self.parse_market(&raw, &id);
            let altname = raw["altname"].as_str().unwrap_or(&id).to_string();
            pair_ids.insert(m.symbol.clone(), altname.clone());
            pair_ids.insert(id.clone(), altname.clone());
            map.insert(m.symbol.clone(), m);
        }
        *self.pair_ids.lock().unwrap() = Some(pair_ids);
        Ok(map)
    }

    /// 从统一 symbol 查 kraken pair id(altname)。
    fn pair_id_cached(&self, symbol: &str) -> Option<String> {
        self.pair_ids.lock().unwrap().as_ref()?.get(symbol).cloned()
    }

    // ================= parse(公开,供差分测试) =================

    pub fn parse_market(&self, raw: &Value, id: &str) -> Market {
        let altname = raw["altname"].as_str().unwrap_or(id);
        let base_raw = raw["base"].as_str().unwrap_or_default();
        let quote_raw = raw["quote"].as_str().unwrap_or_default();
        let base = code(base_raw);
        let quote = code(quote_raw);
        let active = raw["status"]
            .as_str()
            .map(|s| s != "delisted")
            .unwrap_or(true);
        Market {
            id: altname.to_string(),
            symbol: format!("{base}/{quote}"),
            base: Some(base),
            quote: Some(quote),
            base_id: Some(base_raw.to_string()),
            quote_id: Some(quote_raw.to_string()),
            active: Some(active),
            market_type: Some(MarketType::Spot),
            spot: Some(true),
            taker: Some("0.0026".parse().unwrap_or_default()),
            maker: Some("0.0016".parse().unwrap_or_default()),
            precision: Precision {
                amount: raw["lot_decimals"]
                    .as_u64()
                    .map(|d| rust_decimal::Decimal::new(1, d as u32)),
                price: raw["pair_decimals"]
                    .as_u64()
                    .map(|d| rust_decimal::Decimal::new(1, d as u32)),
                cost: None,
            },
            limits: crate::types::Limits::default(),
            info: raw.clone(),
            ..Market::default()
        }
    }

/// kraken ticker 字段的数组/标量兼容抽取:数组取 `idx`(仅 1 个元素时取 0,
/// 兼容 WS 单元素形),标量直接取值。REST 与 WS 共用同一解析(ADR-0015)。
fn ticker_arr_num(v: Option<&Value>, idx: usize) -> Option<rust_decimal::Decimal> {
    match v {
        Some(Value::Array(a)) => a.get(if a.len() > 1 { idx } else { 0 }).and_then(value_decimal),
        Some(s) => value_decimal(s),
        None => None,
    }
}

/// 从订单 `descr` 抽取 (symbol, side, order_type)。REST 中 `descr` 为对象,
/// WS 中为空格分隔字符串 —— 两种形状在此收口(ADR-0015 解析合一)。
pub(crate) fn parse_order_descr(
    descr: &Value,
) -> (Option<String>, Option<String>, Option<String>) {
    match descr {
        Value::Object(_) => (
            descr.get("pair").and_then(Value::as_str).map(str::to_string),
            descr.get("type").and_then(Value::as_str).map(str::to_string),
            descr.get("ordertype").and_then(Value::as_str).map(str::to_string),
        ),
        Value::String(s) => {
            let p: Vec<&str> = s.split(' ').collect();
            (
                p.first().map(|s| s.to_string()),
                p.get(1).map(|s| s.to_string()),
                p.get(2).map(|s| s.to_string()),
            )
        }
        _ => (None, None, None),
    }
}

/// 统一订单状态归一化(合并 REST 与 WS 的状态词)。
pub(crate) fn normalize_order_status(s: &str) -> &str {
    match s {
        "open" | "pending" | "partially_filled" => "open",
        "closed" | "filled" => "closed",
        "canceled" | "cancelled" => "canceled",
        "expired" => "expired",
        other => other,
    }
}

    pub fn parse_ticker(&self, raw: &Value, pair_id: &str) -> Ticker {
        let a = raw.get("a").and_then(Value::as_array);
        let b = raw.get("b").and_then(Value::as_array);
        let c = raw.get("c").and_then(Value::as_array);
        let symbol = self
            .symbol_from_pair(pair_id)
            .unwrap_or_else(|| pair_id.to_string());
        let ask = a.and_then(|a| a.first()).and_then(value_decimal);
        let bid = b.and_then(|b| b.first()).and_then(value_decimal);
        let last = c.and_then(|c| c.first()).and_then(value_decimal);
        let base_volume = Self::ticker_arr_num(raw.get("v"), 1);
        let vwap = Self::ticker_arr_num(raw.get("p"), 1);
        let quote_volume = match (base_volume, vwap) {
            (Some(bv), Some(w)) => Some(bv * w),
            _ => None,
        };
        Ticker {
            symbol,
            open: Self::ticker_arr_num(raw.get("o"), 1),
            high: Self::ticker_arr_num(raw.get("h"), 1),
            low: Self::ticker_arr_num(raw.get("l"), 1),
            close: last,
            last,
            bid,
            ask,
            vwap,
            base_volume,
            quote_volume,
            info: raw.clone(),
            ..Ticker::default()
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
            timestamp: None,
            nonce: None,
            info: raw.clone(),
            ..OrderBook::default()
        }
    }

    pub fn parse_trade(&self, raw: &Value, pair_id: &str) -> Trade {
        let arr = raw.as_array();
        let ts = arr
            .and_then(|a| a.get(2))
            .and_then(Value::as_f64)
            .map(|f| (f * 1000.0) as i64);
        let side = arr
            .and_then(|a| a.get(3))
            .and_then(Value::as_str)
            .map(|s| if s == "b" { "buy" } else { "sell" })
            .map(str::to_string);
        Trade {
            timestamp: ts,
            datetime: ts.and_then(iso8601),
            symbol: Some(
                self.symbol_from_pair(pair_id)
                    .unwrap_or_else(|| pair_id.to_string()),
            ),
            side,
            price: arr.and_then(|a| a.first()).and_then(value_decimal),
            amount: arr.and_then(|a| a.get(1)).and_then(value_decimal),
            info: raw.clone(),
            ..Trade::default()
        }
    }

    pub fn parse_ohlcv(&self, row: &Value) -> OHLCV {
        let arr = row.as_array();
        OHLCV {
            // kraken 时间为秒,ccxt 统一毫秒
            timestamp: arr
                .and_then(|a| a.first())
                .and_then(Value::as_f64)
                .map(|f| (f * 1000.0) as i64),
            open: arr.and_then(|a| a.get(1)).and_then(value_decimal),
            high: arr.and_then(|a| a.get(2)).and_then(value_decimal),
            low: arr.and_then(|a| a.get(3)).and_then(value_decimal),
            close: arr.and_then(|a| a.get(4)).and_then(value_decimal),
            // kraken:index 6 为 volume(5 是 vwap)
            volume: arr.and_then(|a| a.get(6)).and_then(value_decimal),
        }
    }

    /// 解析订单(私密面订单对象)。
    pub fn parse_order(&self, raw: &Value) -> Order {
        let ts = raw["opentm"].as_f64().map(|f| (f * 1000.0) as i64);
        let (symbol, side, order_type) =
            Self::parse_order_descr(raw.get("descr").unwrap_or(&Value::Null));
        Order {
            id: raw["txid"].as_str().map(str::to_string),
            timestamp: ts,
            datetime: ts.and_then(iso8601),
            status: raw["status"]
                .as_str()
                .map(Self::normalize_order_status)
                .map(str::to_string),
            symbol,
            order_type,
            side,
            price: raw["descr"].get("price").and_then(value_decimal),
            amount: raw.get("vol").and_then(value_decimal),
            filled: raw.get("vol_exec").and_then(value_decimal),
            info: raw.clone(),
            ..Order::default()
        }
    }

    /// 从 kraken pair id(altname)→ 统一 symbol(通过 markets 缓存;离线回退简单映射)。
    fn symbol_from_pair(&self, pair_id: &str) -> Option<String> {
        for (symbol, m) in self.core.markets_snapshot().iter() {
            if m.id == pair_id {
                return Some(symbol.clone());
            }
        }
        Some(pair_id.to_string())
    }
}

// ================= Exchange trait =================

impl Exchange for Kraken {
    fn id(&self) -> &'static str {
        ID
    }
    fn config(&self) -> &Config {
        &self.config
    }

    async fn fetch_markets(&self) -> Result<Vec<Market>> {
        self.load_markets().await?;
        Ok(self.core.markets_snapshot().into_values().collect())
    }

    async fn fetch_ticker(&self, symbol: &str, _params: Params) -> Result<Ticker> {
        self.load_markets().await?;
        let pair = self
            .pair_id_cached(symbol)
            .ok_or_else(|| Error::new(ErrorKind::BadSymbol, format!("unknown pair {symbol}")))?;
        let p = params1("pair", &pair);
        let resp = self.public_get("/Ticker", &p).await?;
        let raw = resp
            .get(&pair)
            .ok_or_else(|| Error::new(ErrorKind::BadResponse, format!("no ticker for {pair}")))?;
        Ok(self.parse_ticker(raw, &pair))
    }

    async fn fetch_tickers(&self, _symbols: Option<&[&str]>, _params: Params) -> Result<Tickers> {
        self.load_markets().await?;
        // kraken 要求 pair 参数;用全部缓存 pair 的逗号列表
        let pairs: Vec<String> = self
            .pair_ids
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_default()
            .values()
            .cloned()
            .collect();
        let p = params1("pair", &pairs.join(","));
        let resp = self.public_get("/Ticker", &p).await?;
        let mut out = Tickers::new();
        for (pair, raw) in resp.as_object().unwrap_or(&serde_json::Map::new()) {
            if raw.is_string() {
                continue;
            }
            let t = self.parse_ticker(raw, pair);
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
        self.load_markets().await?;
        let pair = self
            .pair_id_cached(symbol)
            .ok_or_else(|| Error::new(ErrorKind::BadSymbol, format!("unknown pair {symbol}")))?;
        let mut p = Params::new();
        p.insert("pair".into(), json!(pair));
        if let Some(c) = limit {
            p.insert("count".into(), json!(c.min(500)));
        }
        let resp = self.public_get("/Depth", &p).await?;
        let raw = resp
            .get(&pair)
            .ok_or_else(|| Error::new(ErrorKind::BadResponse, "no depth"))?;
        Ok(self.parse_order_book(raw, symbol))
    }

    async fn fetch_trades(
        &self,
        symbol: &str,
        since: Option<i64>,
        limit: Option<i64>,
        _params: Params,
    ) -> Result<Vec<Trade>> {
        self.load_markets().await?;
        let pair = self
            .pair_id_cached(symbol)
            .ok_or_else(|| Error::new(ErrorKind::BadSymbol, format!("unknown pair {symbol}")))?;
        let mut p = Params::new();
        p.insert("pair".into(), json!(pair));
        if let Some(s) = since {
            p.insert("since".into(), json!(s / 1000));
        }
        if let Some(c) = limit {
            p.insert("count".into(), json!(c.min(1000)));
        }
        let resp = self.public_get("/Trades", &p).await?;
        let arr = resp
            .get(&pair)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(arr.iter().map(|t| self.parse_trade(t, &pair)).collect())
    }

    async fn fetch_ohlcv(
        &self,
        symbol: &str,
        timeframe: &str,
        since: Option<i64>,
        limit: Option<i64>,
        _params: Params,
    ) -> Result<Vec<OHLCV>> {
        self.load_markets().await?;
        let pair = self
            .pair_id_cached(symbol)
            .ok_or_else(|| Error::new(ErrorKind::BadSymbol, format!("unknown pair {symbol}")))?;
        let interval = match timeframe {
            "1m" => "1",
            "5m" => "5",
            "15m" => "15",
            "30m" => "30",
            "1h" => "60",
            "4h" => "240",
            "1d" => "1440",
            other => {
                return Err(Error::new(
                    ErrorKind::BadRequest,
                    format!("unsupported timeframe {other}"),
                ));
            }
        };
        let mut p = Params::new();
        p.insert("pair".into(), json!(pair));
        p.insert("interval".into(), json!(interval));
        if let Some(s) = since {
            p.insert("since".into(), json!(s / 1000));
        }
        let resp = self.public_get("/OHLC", &p).await?;
        let arr = resp
            .get(&pair)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut out: Vec<OHLCV> = arr.iter().map(|r| self.parse_ohlcv(r)).collect();
        apply_client_limit(&mut out, limit.map(|l| l as usize), false);
        Ok(out)
    }

    async fn fetch_balance(&self, _params: Params) -> Result<Balances> {
        let resp = self.private_post("/BalanceEx", &Params::new()).await?;
        let mut out = Balances {
            info: resp.clone(),
            ..Balances::default()
        };
        for (currency, raw) in resp.as_object().unwrap_or(&serde_json::Map::new()) {
            if let Some(total) = raw.get("balance").and_then(value_decimal) {
                let hold = raw.get("hold_trade").and_then(value_decimal);
                out.accounts.insert(
                    code(currency).to_string(),
                    Balance {
                        free: match (total, hold) {
                            (t, Some(h)) => Some(t - h),
                            (t, None) => Some(t),
                        },
                        used: hold,
                        total: Some(total),
                        ..Balance::default()
                    },
                );
            }
        }
        Ok(out)
    }

    async fn fetch_positions(
        &self,
        _symbols: Option<&[&str]>,
        _params: Params,
    ) -> Result<Vec<Position>> {
        // 现货无持仓;OpenPositions 仅保证金账户返回数据
        let resp = self.private_post("/OpenPositions", &Params::new()).await?;
        let mut out = Vec::new();
        if let Some(obj) = resp.as_object() {
            for (txid, raw) in obj {
                if !raw.is_object() {
                    continue;
                }
                out.push(Position {
                    id: Some(txid.clone()),
                    symbol: raw["pair"].as_str().map(str::to_string),
                    contracts: raw.get("vol").and_then(value_decimal),
                    entry_price: raw.get("avg_entry_price").and_then(value_decimal),
                    unrealized_pnl: raw.get("net").and_then(value_decimal),
                    info: raw.clone(),
                    ..Position::default()
                });
            }
        }
        Ok(out)
    }

    async fn fetch_orders(
        &self,
        _symbol: Option<&str>,
        _since: Option<i64>,
        _limit: Option<i64>,
        _params: Params,
    ) -> Result<Vec<Order>> {
        // ccxt 无统一 fetch_orders(kraken 用 ClosedOrders);此处映射历史订单
        let resp = self.private_post("/ClosedOrders", &Params::new()).await?;
        let mut orders = Vec::new();
        if let Some(closed) = resp.get("closed").and_then(Value::as_object) {
            for (txid, raw) in closed {
                let mut o = self.parse_order(raw);
                o.id = Some(txid.clone());
                orders.push(o);
            }
        }
        Ok(orders)
    }

    async fn fetch_open_orders(
        &self,
        _symbol: Option<&str>,
        _since: Option<i64>,
        _limit: Option<i64>,
        _params: Params,
    ) -> Result<Vec<Order>> {
        let resp = self.private_post("/OpenOrders", &Params::new()).await?;
        let mut orders = Vec::new();
        if let Some(open) = resp.get("open").and_then(Value::as_object) {
            for (txid, raw) in open {
                let mut o = self.parse_order(raw);
                o.id = Some(txid.clone());
                orders.push(o);
            }
        }
        Ok(orders)
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
        self.load_markets().await?;
        let pair = self
            .pair_id_cached(symbol)
            .ok_or_else(|| Error::new(ErrorKind::BadSymbol, format!("unknown pair {symbol}")))?;
        let ordertype = match order_type {
            "limit" => "limit",
            "market" => "market",
            other => {
                return Err(Error::new(
                    ErrorKind::BadRequest,
                    format!("unsupported ordertype {other}"),
                ));
            }
        };
        let mut p = Params::new();
        p.insert("pair".into(), json!(pair));
        p.insert("type".into(), json!(side));
        p.insert("ordertype".into(), json!(ordertype));
        p.insert("volume".into(), json!(amount));
        if let Some(px) = price {
            p.insert("price".into(), json!(px));
        }
        let resp = self.private_post("/AddOrder", &p).await?;
        let txid = resp
            .get("txid")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(Value::as_str)
            .map(str::to_string);
        let descr = resp.get("descr").cloned().unwrap_or(Value::Null);
        let mut order = Order {
            id: txid,
            symbol: Some(symbol.to_string()),
            order_type: Some(order_type.to_string()),
            side: Some(side.to_string()),
            price: price.and_then(|s| s.parse().ok()),
            amount: amount.parse().ok(),
            status: Some("open".into()),
            info: resp,
            ..Order::default()
        };
        if let Some(order_desc) = descr.get("order").and_then(Value::as_str) {
            order.info = serde_json::json!({ "descr": order_desc });
        }
        Ok(order)
    }

    async fn cancel_order(&self, id: &str, _symbol: &str, _params: Params) -> Result<Order> {
        let mut p = Params::new();
        p.insert("txid".into(), json!(id));
        let resp = self.private_post("/CancelOrder", &p).await?;
        Ok(Order {
            id: Some(id.to_string()),
            status: Some("canceled".into()),
            info: resp,
            ..Order::default()
        })
    }
}

// ================= 静态助手 =================

/// kraken 币种 code → 统一 code(ccxt safe_currency_code)。
pub fn code(raw: &str) -> String {
    match raw {
        "XXBT" | "XBT" => "BTC".to_string(),
        "XETH" => "ETH".to_string(),
        "XLTC" => "LTC".to_string(),
        "XXRP" => "XRP".to_string(),
        "XXLM" => "XLM".to_string(),
        "XDOGE" => "DOGE".to_string(),
        "XETC" => "ETC".to_string(),
        "ZUSD" => "USD".to_string(),
        "ZEUR" => "EUR".to_string(),
        "ZJPY" => "JPY".to_string(),
        "ZCAD" => "CAD".to_string(),
        "ZGBP" => "GBP".to_string(),
        "ZCHF" => "CHF".to_string(),
        "ZAUD" => "AUD".to_string(),
        other => other.to_string(),
    }
}

fn params1(k: &str, v: &str) -> Params {
    let mut p = Params::new();
    p.insert(k.into(), json!(v));
    p
}

/// 校验响应信封 `{error: [], result: {...}}`,返回 `result`。
///
/// 错误体由 `HttpCore` 的 `handle_errors` 接缝(`extract_exchange_error`)
/// 统一识别并细分归类,此处只抽取成功时的 `result`(不可达的错误分支已收口)。
fn ok_result(resp: Value, path: &str) -> Result<Value> {
    resp.get("result").cloned().ok_or_else(|| {
        Error::new(
            ErrorKind::BadResponse,
            format!("kraken {path}: missing result"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn currency_code_mapping() {
        assert_eq!(code("XXBT"), "BTC");
        assert_eq!(code("ZUSD"), "USD");
        assert_eq!(code("USDT"), "USDT");
    }

    #[test]
    fn parse_market_pairs() {
        let ex = Kraken::new(Config::new()).unwrap();
        let raw = json!({
            "altname": "XBTUSD",
            "wsname": "XBT/USD",
            "base": "XXBT",
            "quote": "ZUSD",
            "status": "online",
            "lot_decimals": 8,
            "pair_decimals": 1
        });
        let m = ex.parse_market(&raw, "XBTUSD");
        assert_eq!(m.id, "XBTUSD");
        assert_eq!(m.symbol, "BTC/USD");
        assert_eq!(m.base.as_deref(), Some("BTC"));
        assert_eq!(m.quote.as_deref(), Some("USD"));
        assert_eq!(m.active, Some(true));
    }

    #[test]
    fn parse_ticker_fields() {
        let ex = Kraken::new(Config::new()).unwrap();
        let raw = json!({
            "a": ["100.1", "2", "2.000"],
            "b": ["99.9", "3", "3.000"],
            "c": ["100", "0.5"],
            "v": ["100", "200"],
            "h": ["101", "102"],
            "l": ["98", "97"],
            "o": "95",
            "p": ["100.2", "100.3"]
        });
        let t = ex.parse_ticker(&raw, "XBTUSD");
        assert_eq!(t.ask, Some("100.1".parse().unwrap()));
        assert_eq!(t.bid, Some("99.9".parse().unwrap()));
        assert_eq!(t.last, Some("100".parse().unwrap()));
        assert_eq!(t.base_volume, Some("200".parse().unwrap()));
        assert_eq!(t.quote_volume, Some("20060".parse().unwrap()));
    }

    #[test]
    fn parse_trade_array() {
        let ex = Kraken::new(Config::new()).unwrap();
        let raw = json!(["100", "0.5", 1700000000.0, "b", "l", ""]);
        let t = ex.parse_trade(&raw, "XBTUSD");
        assert_eq!(t.side.as_deref(), Some("buy"));
        assert_eq!(t.price, Some("100".parse().unwrap()));
        assert_eq!(t.amount, Some("0.5".parse().unwrap()));
        assert!(t.timestamp.is_some());
    }

    #[test]
    fn parse_ohlcv_volume_index6() {
        let ex = Kraken::new(Config::new()).unwrap();
        let row = json!([1700000000, "100", "101", "99", "100.5", "100.2", "10", 3]);
        let c = ex.parse_ohlcv(&row);
        assert_eq!(c.timestamp, Some(1700000000000));
        assert_eq!(c.close, Some("100.5".parse().unwrap()));
        assert_eq!(c.volume, Some("10".parse().unwrap()));
    }
}
