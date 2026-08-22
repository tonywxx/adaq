//! okx 现货适配器(M5,ADR-0005)。
//!
//! 对齐 ccxt okx(v5)语义:
//! - symbol 分隔符 `-`(`BTC-USDT`),统一 symbol `BTC/USDT`;
//! - 公共面 `GET /api/v5/{market,public}/...`,价格/数量均为字符串;
//! - 订单簿 `bids/asks` 为 `[price, amount, ...]` 4 元素数组,ccxt 排序 bids 降序/asks 升序;
//! - OHLCV 原始 9 元素数组,取 `[ts, o, h, l, c, vol]`;
//! - 私密面:签名 `base64(HMAC-SHA256(secret, timestamp + method + requestPath + body/query))`,
//!   `OK-ACCESS-KEY/SIGN/TIMESTAMP/PASSPHRASE` 头;GET 的 requestPath 含 query;
//! - rateLimit 110ms。
//!
//! 参考实现基于 ccxt(MIT)适配器解析逻辑,见 `NOTICE`。

use reqwest::header::HeaderMap;
use serde_json::{Value, json};

use crate::error::{Error, ErrorKind, Result};
use crate::exchange::{Config, Exchange, Params};
use crate::httpcore::{
    HttpCore, data_array, dec, iso8601, iso8601_now, parse_level, query_string, resolve_timeframe,
    value_decimal,
};
use crate::types::{
    Balance, Balances, Level, Market, MarketType, Markets, OHLCV, Order, OrderBook, Position,
    Precision, Ticker, Tickers, Trade,
};

pub const ID: &str = "okx";
const BASE_URL: &str = "https://www.okx.com/api/v5";
const RATE_LIMIT_MS: u64 = 110;

/// okx 现货适配器(姊妹所 okxus / myokx 复用,见候选 ③ 参数化)。
///
/// `id` 为字段而非常量:姊妹所只是换 id 与根地址,实现(签名、解析、市集)
/// 完全复用,不再各持一份 ~570 行克隆。base_url 由 [`HttpCore`] 内部持有。
pub struct Okx {
    config: Config,
    core: HttpCore,
    id: &'static str,
}

impl Okx {
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

    /// okx 主站(默认 id `okx`)。
    pub fn new(config: Config) -> Result<Self> {
        Self::with_endpoints(config, ID, BASE_URL, RATE_LIMIT_MS)
    }

    /// 姊妹所构造:换 `id` 与 `base_url`,实现(签名/解析/市集)复用。
    ///
    /// `id` 同时用于凭据查找(`signing::require_secret`)与错误来源标注,
    /// 因此 okxus / myokx 用各自 id 即可命中对应密钥,无需重抄签名代码。
    pub fn with_endpoints(
        config: Config,
        id: &'static str,
        base_url: &str,
        rate_limit_ms: u64,
    ) -> Result<Self> {
        let core = HttpCore::new(&config, base_url, rate_limit_ms, id)?;
        Ok(Self { config, core, id })
    }

    // ================= 内部 HTTP =================

    /// 签名原语(ADR-0013 sign 接缝):`timestamp+method+path+body` → base64 HMAC-SHA256。
    ///
    /// REST 私密请求与 realtime WS 登录帧共用(登录帧 = `ts+GET+/users/self/verify+""`)。
    pub fn sign_str(
        &self,
        timestamp: &str,
        method: &str,
        path: &str,
        body: &str,
    ) -> Result<String> {
        let secret = crate::signing::require_secret(&self.config, self.id)?;
        let auth = format!("{timestamp}{method}{path}{body}");
        Ok(crate::signing::hmac_sha256_b64(secret, &auth))
    }

    /// 私密请求(OKX v5 签名)。
    async fn private_request(
        &self,
        method: &str,
        path: &str,
        params: &Params,
        body: Option<Value>,
    ) -> Result<Value> {
        let api_key = crate::signing::require_api_key(&self.config, "okx")?;
        let passphrase = crate::signing::require_passphrase(&self.config, "okx")?;
        let timestamp = iso8601_now();
        // GET:requestPath 含 ?query;POST:拼接 JSON body
        let (request_path, body_json) = if method == "GET" {
            let qs = query_string(params);
            (format!("{path}{qs}"), None)
        } else {
            let body_json = body.or_else(|| {
                if params.is_empty() {
                    None
                } else {
                    Some(json!(params))
                }
            });
            (path.to_string(), body_json)
        };
        let body_str = body_json
            .as_ref()
            .map(|v| v.to_string())
            .unwrap_or_default();
        let signature = self.sign_str(&timestamp, method, &request_path, &body_str)?;
        let mut headers = HeaderMap::new();
        crate::signing::set_header(&mut headers, "OK-ACCESS-KEY", api_key)?;
        crate::signing::set_header(&mut headers, "OK-ACCESS-SIGN", &signature)?;
        crate::signing::set_header(&mut headers, "OK-ACCESS-TIMESTAMP", &timestamp)?;
        crate::signing::set_header(&mut headers, "OK-ACCESS-PASSPHRASE", passphrase)?;
        if self.config.sandbox {
            headers.insert(
                "x-simulated-trading",
                "1".parse().expect("static header is valid"),
            );
        }
        self.core
            .request(method, &request_path, &headers, body_json)
            .await
    }

    // ================= markets 缓存 =================

    /// 拉取并解析市集(字段映射接缝;缓存由核心 `HttpCore::load_markets` 负责)。
    async fn fetch_markets_raw(&self) -> Result<Markets> {
        let mut p = Params::new();
        p.insert("instType".into(), json!("SPOT"));
        let resp = self.core.public_get("/public/instruments", &p).await?;
        let arr = data_array(&resp)
            .ok_or_else(|| Error::new(ErrorKind::BadResponse, "instruments not array"))?;
        let mut map = Markets::new();
        for raw in arr {
            let m = self.parse_market(raw);
            map.insert(m.symbol.clone(), m);
        }
        Ok(map)
    }

    /// 统一 symbol → 交易所 instId(BTC/USDT → BTC-USDT)。
    fn symbol_id(&self, symbol: &str) -> String {
        symbol.replace('/', "-")
    }

    // ================= parse(公开,供差分测试) =================

    pub fn parse_market(&self, raw: &Value) -> Market {
        let inst_id = raw["instId"].as_str().unwrap_or_default();
        let (base, quote) = match inst_id.split_once('-') {
            Some((base, quote)) => (base, quote),
            None => (inst_id, ""),
        };
        let active = raw["state"].as_str() == Some("live");
        Market {
            id: inst_id.to_string(),
            symbol: format!("{base}/{quote}"),
            base: Some(base.to_string()),
            quote: Some(quote.to_string()),
            base_id: Some(base.to_string()),
            quote_id: Some(quote.to_string()),
            active: Some(active),
            market_type: Some(MarketType::Spot),
            spot: Some(true),
            taker: Some("0.0015".parse().unwrap_or_default()),
            maker: Some("0.0010".parse().unwrap_or_default()),
            precision: Precision {
                amount: raw["lotSz"].as_str().and_then(|s| s.parse().ok()),
                price: raw["tickSz"].as_str().and_then(|s| s.parse().ok()),
                cost: None,
            },
            limits: crate::types::Limits::default(),
            info: raw.clone(),
            ..Market::default()
        }
    }

    pub fn parse_ticker(&self, raw: &Value) -> Ticker {
        let inst_id = raw["instId"].as_str().unwrap_or_default();
        let ts = raw["ts"].as_str().and_then(|s| s.parse::<i64>().ok());
        Ticker {
            symbol: inst_id.replace('-', "/"),
            timestamp: ts,
            datetime: ts.and_then(iso8601),
            open: dec(raw.get("open24h")),
            high: dec(raw.get("high24h")),
            low: dec(raw.get("low24h")),
            close: dec(raw.get("last")),
            last: dec(raw.get("last")),
            bid: dec(raw.get("bidPx")),
            ask: dec(raw.get("askPx")),
            base_volume: dec(raw.get("vol24h")),
            quote_volume: dec(raw.get("volCcy24h")),
            info: raw.clone(),
            ..Ticker::default()
        }
    }

    pub fn parse_order_book(&self, raw: &Value, symbol: &str) -> OrderBook {
        let data = data_array(raw)
            .and_then(|a| a.first())
            .cloned();
        let book = data.as_ref().unwrap_or(raw);
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
        OrderBook {
            symbol: symbol.to_string(),
            bids,
            asks,
            timestamp: book["ts"].as_str().and_then(|s| s.parse::<i64>().ok()),
            nonce: None,
            info: book.clone(),
            ..OrderBook::default()
        }
    }

    pub fn parse_trade(&self, raw: &Value) -> Trade {
        let ts = raw["ts"].as_str().and_then(|s| s.parse::<i64>().ok());
        let inst_id = raw["instId"].as_str().unwrap_or_default();
        Trade {
            id: raw["tradeId"].as_str().map(str::to_string),
            timestamp: ts,
            datetime: ts.and_then(iso8601),
            symbol: Some(inst_id.replace('-', "/")),
            side: raw["side"].as_str().map(|s| s.to_lowercase()),
            price: dec(raw.get("px")),
            amount: dec(raw.get("sz")),
            info: raw.clone(),
            ..Trade::default()
        }
    }

    pub fn parse_ohlcv(&self, row: &Value) -> OHLCV {
        let arr = row.as_array();
        OHLCV {
            timestamp: arr
                .and_then(|a| a.first())
                .and_then(Value::as_str)
                .and_then(|s| s.parse::<i64>().ok()),
            open: arr.and_then(|a| a.get(1)).and_then(value_decimal),
            high: arr.and_then(|a| a.get(2)).and_then(value_decimal),
            low: arr.and_then(|a| a.get(3)).and_then(value_decimal),
            close: arr.and_then(|a| a.get(4)).and_then(value_decimal),
            volume: arr.and_then(|a| a.get(5)).and_then(value_decimal),
        }
    }

    /// 解析订单(私密面 orders)。
    pub fn parse_order(&self, raw: &Value) -> Order {
        let status = raw["state"]
            .as_str()
            .map(|s| match s {
                "live" => "open",
                "partially_filled" => "open",
                "filled" => "closed",
                "canceled" => "canceled",
                other => other,
            })
            .map(str::to_string);
        let ts = raw["cTime"].as_str().and_then(|s| s.parse::<i64>().ok());
        Order {
            id: raw["ordId"].as_str().map(str::to_string),
            client_order_id: raw["clOrdId"].as_str().map(str::to_string),
            timestamp: ts,
            datetime: ts.and_then(iso8601),
            status,
            symbol: raw["instId"].as_str().map(|s| s.replace('-', "/")),
            order_type: raw["ordType"].as_str().map(|s| s.to_lowercase()),
            side: raw["side"].as_str().map(|s| s.to_lowercase()),
            price: dec(raw.get("px")),
            average: dec(raw.get("avgPx")),
            amount: dec(raw.get("sz")),
            filled: dec(raw.get("accFillSz")),
            remaining: dec(raw.get("sz")).and_then(|sz| dec(raw.get("accFillSz")).map(|f| sz - f)),
            info: raw.clone(),
            ..Order::default()
        }
    }

    /// 解析仓位(公开,供差分测试与 WS 复用)。
    pub fn parse_position(&self, raw: &Value) -> Position {
        Position {
            symbol: raw["instId"].as_str().map(|s| s.replace('-', "/")),
            id: raw["posId"].as_str().map(str::to_string),
            contracts: dec(raw.get("pos")),
            entry_price: dec(raw.get("avgPx")),
            unrealized_pnl: dec(raw.get("upl")),
            notional: dec(raw.get("notionalUsd")),
            info: raw.clone(),
            ..Position::default()
        }
    }
}

// ================= Exchange trait =================

impl Exchange for Okx {
    fn id(&self) -> &'static str {
        self.id
    }
    fn config(&self) -> &Config {
        &self.config
    }

    async fn fetch_markets(&self) -> Result<Vec<Market>> {
        self.core.load_markets(|| self.fetch_markets_raw()).await?;
        Ok(self.core.markets_snapshot().into_values().collect())
    }

    async fn fetch_ticker(&self, symbol: &str, _params: Params) -> Result<Ticker> {
        let p = params1("instId", &self.symbol_id(symbol));
        let resp = self.core.public_get("/market/ticker", &p).await?;
        let raw = data_array(&resp)
            .and_then(|a| a.first())
            .ok_or_else(|| Error::new(ErrorKind::BadResponse, "ticker empty"))?;
        Ok(self.parse_ticker(raw))
    }

    async fn fetch_tickers(&self, _symbols: Option<&[&str]>, _params: Params) -> Result<Tickers> {
        let mut p = Params::new();
        p.insert("instType".into(), json!("SPOT"));
        let resp = self.core.public_get("/market/tickers", &p).await?;
        let arr = data_array(&resp)
            .cloned()
            .unwrap_or_default();
        let mut out = Tickers::new();
        for raw in arr {
            let t = self.parse_ticker(&raw);
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
        p.insert("instId".into(), json!(self.symbol_id(symbol)));
        p.insert("sz".into(), json!(limit.unwrap_or(100).min(400)));
        let resp = self.core.public_get("/market/books", &p).await?;
        Ok(self.parse_order_book(&resp, &symbol.replace('-', "/")))
    }

    async fn fetch_trades(
        &self,
        symbol: &str,
        _since: Option<i64>,
        limit: Option<i64>,
        _params: Params,
    ) -> Result<Vec<Trade>> {
        let mut p = Params::new();
        p.insert("instId".into(), json!(self.symbol_id(symbol)));
        p.insert("limit".into(), json!(limit.unwrap_or(100).min(500)));
        let resp = self.core.public_get("/market/trades", &p).await?;
        let arr = data_array(&resp)
            .cloned()
            .unwrap_or_default();
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
        let bar = resolve_timeframe(
            &[("1m", "1m"), ("5m", "5m"), ("15m", "15m"), ("1h", "1H"), ("4h", "4H"), ("1d", "1D")],
            timeframe,
        )?;
        let mut p = Params::new();
        p.insert("instId".into(), json!(self.symbol_id(symbol)));
        p.insert("bar".into(), json!(bar));
        p.insert("limit".into(), json!(limit.unwrap_or(100).min(300)));
        let resp = self.core.public_get("/market/candles", &p).await?;
        let arr = data_array(&resp)
            .cloned()
            .unwrap_or_default();
        Ok(arr.iter().map(|r| self.parse_ohlcv(r)).collect())
    }

    async fn fetch_balance(&self, _params: Params) -> Result<Balances> {
        let resp = self
            .private_request("GET", "/account/balance", &Params::new(), None)
            .await?;
        let mut out = Balances {
            info: resp.clone(),
            ..Balances::default()
        };
        if let Some(data) = data_array(&resp)
            .and_then(|a| a.first())
        {
            if let Some(details) = data.get("details").and_then(Value::as_array) {
                for d in details {
                    if let Some(code) = d["ccy"].as_str() {
                        let cash = dec(d.get("cashBal"));
                        let avail = dec(d.get("availBal"));
                        out.accounts.insert(
                            code.to_string(),
                            Balance {
                                free: avail.or(cash),
                                total: cash,
                                ..Balance::default()
                            },
                        );
                    }
                }
            }
        }
        Ok(out)
    }

    async fn fetch_positions(
        &self,
        _symbols: Option<&[&str]>,
        _params: Params,
    ) -> Result<Vec<Position>> {
        let resp = self
            .private_request("GET", "/account/positions", &Params::new(), None)
            .await?;
        let arr = data_array(&resp)
            .cloned()
            .unwrap_or_default();
        Ok(arr
            .iter()
            .filter(|p| p["pos"].as_str().map(|s| s != "0").unwrap_or(false))
            .map(|p| self.parse_position(p))
            .collect())
    }

    async fn fetch_orders(
        &self,
        symbol: Option<&str>,
        _since: Option<i64>,
        _limit: Option<i64>,
        _params: Params,
    ) -> Result<Vec<Order>> {
        let mut p = Params::new();
        p.insert("instType".into(), json!("SPOT"));
        if let Some(s) = symbol {
            p.insert("instId".into(), json!(self.symbol_id(s)));
        }
        let resp = self
            .private_request("GET", "/trade/orders-history", &p, None)
            .await?;
        let arr = data_array(&resp)
            .cloned()
            .unwrap_or_default();
        Ok(arr.iter().map(|o| self.parse_order(o)).collect())
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
            p.insert("instId".into(), json!(self.symbol_id(s)));
        }
        let resp = self
            .private_request("GET", "/trade/orders-pending", &p, None)
            .await?;
        let arr = data_array(&resp)
            .cloned()
            .unwrap_or_default();
        Ok(arr.iter().map(|o| self.parse_order(o)).collect())
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
        let ord_type = match order_type {
            "limit" => "limit",
            "market" => "market",
            other => {
                return Err(Error::new(
                    ErrorKind::BadRequest,
                    format!("unsupported ordType {other}"),
                ));
            }
        };
        let mut body = serde_json::Map::new();
        body.insert("instId".into(), json!(self.symbol_id(symbol)));
        body.insert("tdMode".into(), json!("cash"));
        body.insert("side".into(), json!(side));
        body.insert("ordType".into(), json!(ord_type));
        body.insert("sz".into(), json!(amount));
        if let Some(px) = price {
            body.insert("px".into(), json!(px));
        }
        let resp = self
            .private_request(
                "POST",
                "/trade/order",
                &Params::new(),
                Some(Value::Object(body)),
            )
            .await?;
        let data = data_array(&resp)
            .and_then(|a| a.first())
            .cloned()
            .ok_or_else(|| Error::new(ErrorKind::BadResponse, "order empty"))?;
        let mut order = self.parse_order(&data);
        if order.id.is_none() {
            order.id = data["ordId"].as_str().map(str::to_string);
        }
        Ok(order)
    }

    async fn cancel_order(&self, id: &str, symbol: &str, _params: Params) -> Result<Order> {
        let body = json!({
            "instId": self.symbol_id(symbol),
            "ordId": id,
        });
        let resp = self
            .private_request("POST", "/trade/cancel-order", &Params::new(), Some(body))
            .await?;
        let data = data_array(&resp)
            .and_then(|a| a.first())
            .cloned()
            .ok_or_else(|| Error::new(ErrorKind::BadResponse, "cancel empty"))?;
        let mut order = Order {
            id: data["ordId"].as_str().map(str::to_string),
            status: Some("canceled".into()),
            info: data,
            ..Order::default()
        };
        if order.id.is_none() {
            order.id = Some(id.to_string());
        }
        Ok(order)
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
    fn symbol_id_converts_separator() {
        let ex = Okx::new(Config::new()).unwrap();
        assert_eq!(ex.symbol_id("BTC/USDT"), "BTC-USDT");
    }

    #[test]
    fn parse_market_spot() {
        let ex = Okx::new(Config::new()).unwrap();
        let raw = json!({
            "instId": "BTC-USDT",
            "state": "live",
            "lotSz": "0.0001",
            "tickSz": "0.1"
        });
        let m = ex.parse_market(&raw);
        assert_eq!(m.symbol, "BTC/USDT");
        assert_eq!(m.base.as_deref(), Some("BTC"));
        assert_eq!(m.quote.as_deref(), Some("USDT"));
        assert_eq!(m.market_type, Some(MarketType::Spot));
        assert_eq!(m.active, Some(true));
    }

    #[test]
    fn parse_order_book_sorted() {
        let ex = Okx::new(Config::new()).unwrap();
        let raw = json!({
            "data": [{
                "ts": "123",
                "bids": [["100", "1", "0", "2"], ["99", "2", "0", "1"]],
                "asks": [["101", "3", "0", "1"], ["102", "4", "0", "2"]]
            }]
        });
        let book = ex.parse_order_book(&raw, "BTC/USDT");
        assert_eq!(book.bids[0].price, Some("100".parse().unwrap()));
        assert_eq!(book.bids[1].price, Some("99".parse().unwrap()));
        assert_eq!(book.asks[0].price, Some("101".parse().unwrap()));
        assert_eq!(book.asks[1].price, Some("102".parse().unwrap()));
        assert_eq!(book.bids[0].amount, Some("1".parse().unwrap()));
    }

    #[test]
    fn parse_ohlcv_takes_first_six() {
        let ex = Okx::new(Config::new()).unwrap();
        let row = json!([
            "1234567890123",
            "100",
            "101",
            "99",
            "100.5",
            "10",
            "1000",
            "10000",
            "1"
        ]);
        let c = ex.parse_ohlcv(&row);
        assert_eq!(c.timestamp, Some(1234567890123));
        assert_eq!(c.open, Some("100".parse().unwrap()));
        assert_eq!(c.close, Some("100.5".parse().unwrap()));
        assert_eq!(c.volume, Some("10".parse().unwrap()));
    }

    #[test]
    fn parse_ticker_fields() {
        let ex = Okx::new(Config::new()).unwrap();
        let raw = json!({
            "instId": "BTC-USDT",
            "ts": "123",
            "last": "100",
            "bidPx": "99.9",
            "askPx": "100.1",
            "open24h": "95",
            "high24h": "101",
            "low24h": "94",
            "vol24h": "10",
            "volCcy24h": "1000"
        });
        let t = ex.parse_ticker(&raw);
        assert_eq!(t.symbol, "BTC/USDT");
        assert_eq!(t.last, Some("100".parse().unwrap()));
        assert_eq!(t.bid, Some("99.9".parse().unwrap()));
        assert_eq!(t.ask, Some("100.1".parse().unwrap()));
        assert_eq!(t.base_volume, Some("10".parse().unwrap()));
    }
}
