//! bybit 现货适配器(M5,ADR-0005)。
//!
//! 对齐 ccxt bybit(v5)语义:
//! - `base/quote` 来自 `baseCoin/quoteCoin` 显式字段;统一 symbol `BTC/USDT`;
//! - 订单簿 `result.{b,a}` 为 `[price, size]` 字符串对,ccxt 排序 bids 降序/asks 升序;
//! - trades 为对象(`execId/price/size/side/time`);OHLCV 为 7 字符串数组,取 `[0..5]`;
//! - 私密面:`X-BAPI-SIGN = hex(HMAC-SHA256(secret, ts + apiKey + recvWindow + payload))`,
//!   payload:GET = query string,POST = JSON body;recvWindow 默认 5000;
//! - 现货 fetch_orders 在 ccxt 为 NotSupported(history 仅衍生品)→ 未声明;
//! - rateLimit 20ms。
//!
//! 参考实现基于 ccxt(MIT)适配器解析逻辑,见 `NOTICE`。

use reqwest::header::HeaderMap;
use serde_json::{Value, json};

use crate::error::{Error, ErrorKind, Result};
use crate::exchange::{Config, Exchange, Params};
use crate::httpcore::{HttpCore, dec, iso8601, now_ms, parse_level, query_string, value_decimal};
use crate::types::{
    Balance, Balances, Level, Market, MarketType, Markets, OHLCV, Order, OrderBook, Position,
    Precision, Ticker, Tickers, Trade,
};

pub const ID: &str = "bybit";
const BASE_URL: &str = "https://api.bybit.com/v5";
const RATE_LIMIT_MS: u64 = 20;
const RECV_WINDOW: &str = "5000";

/// bybit 现货适配器。
pub struct Bybit {
    config: Config,
    core: HttpCore,
}

impl Bybit {
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
        "fetch_open_orders",
        "create_order",
        "cancel_order",
    ];

    pub fn new(config: Config) -> Result<Self> {
        let core = HttpCore::new(&config, BASE_URL, RATE_LIMIT_MS, "bybit")?;
        Ok(Self { config, core })
    }

    // ================= 内部 HTTP =================

    /// 私密请求(bybit v5 签名)。
    async fn private_request(
        &self,
        method: &str,
        path: &str,
        params: &Params,
        body: Option<Value>,
    ) -> Result<Value> {
        let api_key = crate::signing::require_api_key(&self.config, "bybit")?;
        let secret = crate::signing::require_secret(&self.config, "bybit")?;
        let timestamp = now_ms().to_string();
        // payload:GET = query string(不带 ?),POST = JSON body
        let (payload, body_json, req_path) = if method == "GET" {
            let qs = query_string(params).trim_start_matches('?').to_string();
            (qs.clone(), None, format!("{path}{}", query_string(params)))
        } else {
            let b = body.or_else(|| {
                if params.is_empty() {
                    None
                } else {
                    Some(json!(params))
                }
            });
            let s = b.clone().map(|v| v.to_string()).unwrap_or_default();
            (s.clone(), b, path.to_string())
        };
        let auth = format!("{timestamp}{api_key}{RECV_WINDOW}{payload}");
        let signature = crate::signing::hmac_sha256_hex(secret, &auth);
        let mut headers = HeaderMap::new();
        crate::signing::set_header(&mut headers, "X-BAPI-API-KEY", api_key)?;
        crate::signing::set_header(&mut headers, "X-BAPI-TIMESTAMP", &timestamp)?;
        crate::signing::set_header(&mut headers, "X-BAPI-RECV-WINDOW", RECV_WINDOW)?;
        crate::signing::set_header(&mut headers, "X-BAPI-SIGN", &signature)?;
        self.core
            .request(method, &req_path, &headers, body_json)
            .await
    }

    // ================= markets 缓存 =================

    async fn load_markets(&self) -> Result<()> {
        self.core.load_markets(|| self.fetch_markets_raw()).await
    }

    async fn fetch_markets_raw(&self) -> Result<Markets> {
        let mut p = Params::new();
        p.insert("category".into(), json!("spot"));
        let resp = self.core.public_get("/market/instruments-info", &p).await?;
        let arr = resp
            .get("result")
            .and_then(|r| r.get("list"))
            .and_then(Value::as_array)
            .ok_or_else(|| Error::new(ErrorKind::BadResponse, "instruments not array"))?;
        let mut map = Markets::new();
        for raw in arr {
            let m = self.parse_market(raw);
            map.insert(m.symbol.clone(), m);
        }
        Ok(map)
    }

    fn symbol_id(&self, symbol: &str) -> String {
        symbol.replace('/', "")
    }

    // ================= parse(公开,供差分测试) =================

    pub fn parse_market(&self, raw: &Value) -> Market {
        let id = raw["symbol"].as_str().unwrap_or_default();
        let base = raw["baseCoin"].as_str().unwrap_or_default();
        let quote = raw["quoteCoin"].as_str().unwrap_or_default();
        let active = raw["status"].as_str() == Some("Trading");
        Market {
            id: id.to_string(),
            symbol: format!("{base}/{quote}"),
            base: Some(base.to_string()),
            quote: Some(quote.to_string()),
            base_id: Some(base.to_string()),
            quote_id: Some(quote.to_string()),
            active: Some(active),
            market_type: Some(MarketType::Spot),
            spot: Some(true),
            taker: Some("0.001".parse().unwrap_or_default()),
            maker: Some("0.001".parse().unwrap_or_default()),
            precision: Precision {
                amount: raw
                    .get("lotSizeFilter")
                    .and_then(|f| f.get("basePrecision"))
                    .and_then(Value::as_str)
                    .and_then(|s| s.parse().ok()),
                price: raw
                    .get("priceFilter")
                    .and_then(|f| f.get("tickSize"))
                    .and_then(Value::as_str)
                    .and_then(|s| s.parse().ok()),
                cost: None,
            },
            limits: crate::types::Limits::default(),
            info: raw.clone(),
            ..Market::default()
        }
    }

    pub fn parse_ticker(&self, raw: &Value) -> Ticker {
        let id = raw["symbol"].as_str().unwrap_or_default();
        Ticker {
            symbol: id.replace('/', "").replace("USDT", "/USDT"),
            // WS 推送用 timestamp/ts(REST 用 time,解析合一 fallback;WS 为数字,REST 为字符串)
            timestamp: raw
                .get("time")
                .or_else(|| raw.get("timestamp"))
                .or_else(|| raw.get("ts"))
                .and_then(ts_value),
            open: dec(raw.get("prevPrice24h")),
            high: dec(raw.get("highPrice24h").or_else(|| raw.get("high24h"))),
            low: dec(raw.get("lowPrice24h").or_else(|| raw.get("low24h"))),
            close: dec(raw.get("lastPrice")),
            last: dec(raw.get("lastPrice")),
            bid: dec(raw.get("bid1Price")),
            ask: dec(raw.get("ask1Price")),
            base_volume: dec(raw.get("volume24h")),
            quote_volume: dec(raw.get("turnover24h")),
            percentage: raw
                .get("price24hPcnt")
                .and_then(value_decimal)
                .map(|p| p * rust_decimal::Decimal::from(100)),
            info: raw.clone(),
            ..Ticker::default()
        }
    }

    pub fn parse_order_book(&self, raw: &Value, symbol: &str) -> OrderBook {
        let result = raw.get("result").unwrap_or(raw);
        let mut bids: Vec<Level> = result
            .get("b")
            .and_then(Value::as_array)
            .map(|a| a.iter().map(parse_level).collect())
            .unwrap_or_default();
        let mut asks: Vec<Level> = result
            .get("a")
            .and_then(Value::as_array)
            .map(|a| a.iter().map(parse_level).collect())
            .unwrap_or_default();
        bids.sort_by_key(|l| std::cmp::Reverse(l.price));
        asks.sort_by_key(|l| l.price);
        OrderBook {
            symbol: symbol.to_string(),
            bids,
            asks,
            timestamp: result["ts"].as_str().and_then(|s| s.parse::<i64>().ok()),
            nonce: result["u"].as_str().and_then(|s| s.parse::<i64>().ok()),
            info: result.clone(),
            ..OrderBook::default()
        }
    }

    pub fn parse_trade(&self, raw: &Value) -> Trade {
        let ts = raw["time"].as_str().and_then(|s| s.parse::<i64>().ok());
        let id = raw["symbol"].as_str().unwrap_or_default().to_string();
        Trade {
            id: raw["execId"].as_str().map(str::to_string),
            timestamp: ts,
            datetime: ts.and_then(iso8601),
            symbol: Some(format_symbol(&id)),
            side: raw["side"].as_str().map(|s| s.to_lowercase()),
            price: dec(raw.get("price")),
            amount: dec(raw.get("size")),
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

    /// 解析订单(私密面)。
    pub fn parse_order(&self, raw: &Value) -> Order {
        let status = raw["orderStatus"]
            .as_str()
            .map(|s| match s {
                "New" | "PartiallyFilled" | "PartiallyFilledCanceled" => "open",
                "Filled" => "closed",
                "Cancelled" => "canceled",
                "Rejected" => "rejected",
                other => other,
            })
            .map(str::to_string);
        // WS 推送用 timestamp(REST 用 createdTime,解析合一 fallback;WS 为数字,REST 为字符串)
        let ts = raw
            .get("createdTime")
            .or_else(|| raw.get("timestamp"))
            .and_then(ts_value);
        Order {
            id: raw["orderId"].as_str().map(str::to_string),
            client_order_id: raw["orderLinkId"].as_str().map(str::to_string),
            timestamp: ts,
            datetime: ts.and_then(iso8601),
            status,
            symbol: raw["symbol"].as_str().map(|s| s.replace("USDT", "/USDT")),
            order_type: raw["orderType"].as_str().map(|s| s.to_lowercase()),
            side: raw["side"].as_str().map(|s| s.to_lowercase()),
            price: dec(raw.get("price")),
            average: dec(raw.get("avgPrice")),
            amount: dec(raw.get("qty")),
            filled: dec(raw.get("cumExecQty")),
            info: raw.clone(),
            ..Order::default()
        }
    }
}

// ================= Exchange trait =================

impl Exchange for Bybit {
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
        let mut p = Params::new();
        p.insert("category".into(), json!("spot"));
        p.insert("symbol".into(), json!(self.symbol_id(symbol)));
        let resp = self.core.public_get("/market/tickers", &p).await?;
        let raw = resp
            .get("result")
            .and_then(|r| r.get("list"))
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .ok_or_else(|| Error::new(ErrorKind::BadResponse, "ticker empty"))?;
        Ok(self.parse_ticker(raw))
    }

    async fn fetch_tickers(&self, _symbols: Option<&[&str]>, _params: Params) -> Result<Tickers> {
        let mut p = Params::new();
        p.insert("category".into(), json!("spot"));
        let resp = self.core.public_get("/market/tickers", &p).await?;
        let arr = resp
            .get("result")
            .and_then(|r| r.get("list"))
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

    async fn fetch_order_book(
        &self,
        symbol: &str,
        limit: Option<i64>,
        _params: Params,
    ) -> Result<OrderBook> {
        let mut p = Params::new();
        p.insert("category".into(), json!("spot"));
        p.insert("symbol".into(), json!(self.symbol_id(symbol)));
        p.insert("limit".into(), json!(limit.unwrap_or(50).clamp(1, 50)));
        let resp = self.core.public_get("/market/orderbook", &p).await?;
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
        p.insert("category".into(), json!("spot"));
        p.insert("symbol".into(), json!(self.symbol_id(symbol)));
        p.insert("limit".into(), json!(limit.unwrap_or(60).clamp(1, 60)));
        let resp = self.core.public_get("/market/recent-trade", &p).await?;
        let arr = resp
            .get("result")
            .and_then(|r| r.get("list"))
            .and_then(Value::as_array)
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
        let interval = match timeframe {
            "1m" => "1",
            "5m" => "5",
            "15m" => "15",
            "1h" => "60",
            "4h" => "240",
            "1d" => "D",
            other => {
                return Err(Error::new(
                    ErrorKind::BadRequest,
                    format!("unsupported timeframe {other}"),
                ));
            }
        };
        let mut p = Params::new();
        p.insert("category".into(), json!("spot"));
        p.insert("symbol".into(), json!(self.symbol_id(symbol)));
        p.insert("interval".into(), json!(interval));
        p.insert("limit".into(), json!(limit.unwrap_or(200).min(1000)));
        let resp = self.core.public_get("/market/kline", &p).await?;
        let arr = resp
            .get("result")
            .and_then(|r| r.get("list"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(arr.iter().map(|r| self.parse_ohlcv(r)).collect())
    }

    async fn fetch_balance(&self, _params: Params) -> Result<Balances> {
        let mut p = Params::new();
        p.insert("accountType".into(), json!("SPOT"));
        let resp = self
            .private_request("GET", "/account/wallet-balance", &p, None)
            .await?;
        let mut out = Balances {
            info: resp.clone(),
            ..Balances::default()
        };
        if let Some(list) = resp
            .get("result")
            .and_then(|r| r.get("list"))
            .and_then(Value::as_array)
        {
            for acc in list {
                if let Some(coins) = acc.get("coin").and_then(Value::as_array) {
                    for c in coins {
                        if let Some(code) = c["coin"].as_str() {
                            let total = dec(c.get("walletBalance"));
                            let free = dec(c.get("availableToWithdraw"));
                            out.accounts.insert(
                                code.to_string(),
                                Balance {
                                    free,
                                    total,
                                    used: match (total, free) {
                                        (Some(t), Some(f)) => Some(t - f),
                                        _ => None,
                                    },
                                    ..Balance::default()
                                },
                            );
                        }
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
        let mut p = Params::new();
        p.insert("category".into(), json!("spot"));
        let resp = self
            .private_request("GET", "/position/list", &p, None)
            .await?;
        let arr = resp
            .get("result")
            .and_then(|r| r.get("list"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(arr
            .iter()
            .filter(|x| x["size"].as_str().map(|s| s != "0").unwrap_or(false))
            .map(|x| Position {
                symbol: x["symbol"].as_str().map(|s| s.replace("USDT", "/USDT")),
                id: x["positionIdx"].as_str().map(str::to_string),
                contracts: dec(x.get("size")),
                entry_price: dec(x.get("avgPrice")),
                unrealized_pnl: dec(x.get("unrealisedPnl")),
                info: x.clone(),
                ..Position::default()
            })
            .collect())
    }

    async fn fetch_open_orders(
        &self,
        symbol: Option<&str>,
        _since: Option<i64>,
        _limit: Option<i64>,
        _params: Params,
    ) -> Result<Vec<Order>> {
        let mut p = Params::new();
        p.insert("category".into(), json!("spot"));
        if let Some(s) = symbol {
            p.insert("symbol".into(), json!(self.symbol_id(s)));
        }
        let resp = self
            .private_request("GET", "/order/realtime", &p, None)
            .await?;
        let arr = resp
            .get("result")
            .and_then(|r| r.get("list"))
            .and_then(Value::as_array)
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
        let order_type_cap = match order_type {
            "limit" => "Limit",
            "market" => "Market",
            other => {
                return Err(Error::new(
                    ErrorKind::BadRequest,
                    format!("unsupported orderType {other}"),
                ));
            }
        };
        let side_cap = match side {
            "buy" => "Buy",
            "sell" => "Sell",
            other => {
                return Err(Error::new(
                    ErrorKind::BadRequest,
                    format!("unsupported side {other}"),
                ));
            }
        };
        let mut body = serde_json::Map::new();
        body.insert("category".into(), json!("spot"));
        body.insert("symbol".into(), json!(self.symbol_id(symbol)));
        body.insert("side".into(), json!(side_cap));
        body.insert("orderType".into(), json!(order_type_cap));
        body.insert("qty".into(), json!(amount));
        if let Some(px) = price {
            body.insert("price".into(), json!(px));
            body.insert("timeInForce".into(), json!("GTC"));
        }
        let resp = self
            .private_request(
                "POST",
                "/order/create",
                &Params::new(),
                Some(Value::Object(body)),
            )
            .await?;
        let result = resp.get("result").cloned().unwrap_or(Value::Null);
        let mut order = self.parse_order(&result);
        if order.id.is_none() {
            order.id = result["orderId"].as_str().map(str::to_string);
        }
        Ok(order)
    }

    async fn cancel_order(&self, id: &str, symbol: &str, _params: Params) -> Result<Order> {
        let body = json!({
            "category": "spot",
            "symbol": self.symbol_id(symbol),
            "orderId": id,
        });
        let resp = self
            .private_request("POST", "/order/cancel", &Params::new(), Some(body))
            .await?;
        let result = resp.get("result").cloned().unwrap_or(Value::Null);
        let mut order = Order {
            id: result["orderId"].as_str().map(str::to_string),
            status: Some("canceled".into()),
            info: result,
            ..Order::default()
        };
        if order.id.is_none() {
            order.id = Some(id.to_string());
        }
        Ok(order)
    }
}

// ================= 静态助手 =================

/// BTCUSDT → BTC/USDT(报价币固定 USDT 拆分;非 USDT 对原样)。
fn format_symbol(id: &str) -> String {
    id.replace("USDT", "/USDT")
}

/// 时间戳取值:兼容字符串(如 REST `"time"`)与数字(如 WS `timestamp`)。
fn ts_value(v: &Value) -> Option<i64> {
    match v {
        Value::String(s) => s.parse().ok(),
        Value::Number(n) => n.as_i64(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_market_spot() {
        let ex = Bybit::new(Config::new()).unwrap();
        let raw = json!({
            "symbol": "BTCUSDT",
            "baseCoin": "BTC",
            "quoteCoin": "USDT",
            "status": "Trading",
            "priceFilter": {"tickSize": "0.01"},
            "lotSizeFilter": {"basePrecision": "0.0001"}
        });
        let m = ex.parse_market(&raw);
        assert_eq!(m.symbol, "BTC/USDT");
        assert_eq!(m.base.as_deref(), Some("BTC"));
        assert_eq!(m.quote.as_deref(), Some("USDT"));
        assert_eq!(m.market_type, Some(MarketType::Spot));
        assert_eq!(m.precision.price, Some("0.01".parse().unwrap()));
    }

    #[test]
    fn parse_order_book_sorted() {
        let ex = Bybit::new(Config::new()).unwrap();
        let raw = json!({
            "result": {
                "b": [["100", "1"], ["99", "2"]],
                "a": [["101", "3"], ["102", "4"]],
                "ts": "123",
                "u": "456"
            }
        });
        let book = ex.parse_order_book(&raw, "BTC/USDT");
        assert_eq!(book.bids[0].price, Some("100".parse().unwrap()));
        assert_eq!(book.asks[1].price, Some("102".parse().unwrap()));
        assert_eq!(book.nonce, Some(456));
    }

    #[test]
    fn parse_ohlcv_seven_elements() {
        let ex = Bybit::new(Config::new()).unwrap();
        let row = json!(["1234567890", "100", "101", "99", "100.5", "10", "1000"]);
        let c = ex.parse_ohlcv(&row);
        assert_eq!(c.timestamp, Some(1234567890));
        assert_eq!(c.close, Some("100.5".parse().unwrap()));
        assert_eq!(c.volume, Some("10".parse().unwrap()));
    }

    #[test]
    fn parse_trade_object() {
        let ex = Bybit::new(Config::new()).unwrap();
        let raw = json!({
            "execId": "e1",
            "symbol": "BTCUSDT",
            "price": "100",
            "size": "0.5",
            "side": "Buy",
            "time": "123"
        });
        let t = ex.parse_trade(&raw);
        assert_eq!(t.id.as_deref(), Some("e1"));
        assert_eq!(t.side.as_deref(), Some("buy"));
        assert_eq!(t.price, Some("100".parse().unwrap()));
    }
}
