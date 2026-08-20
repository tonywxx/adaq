//! kraken WebSocket 适配器(Phase C,ADR-0009)。
//!
//! 对齐 kraken WS 协议(消息为**数组**格式):
//! - 公开流 `wss://ws.kraken.com`,订阅帧
//!   `{"event":"subscribe","pair":["XBT/USDT"],"subscription":{"name":"ticker"|"book"|"trade"|"ohlc"}}`;
//! - 消息 `[channelId, data, channelName, pair]`:ticker 的 `a[0]/b[0]/c[0]/h[0]/l[0]/o[0]/v[0]/p[0]`;
//!   book 快照 `{as, bs}` 与增量 `{a, b}`(`[price, vol, ts]`,vol=0 删除档位);
//!   trade 数组 `[[price, vol, ts, side, ...], ...]`;ohlc 数组 `[ts,o,h,l,c,vwap,vol,count]`;
//! - 私密流 `wss://ws-auth.kraken.com` 需 REST 取 token:
//!   POST /0/private/GetWebSocketsToken(复用 kraken 签名),订阅带 `"token"`;
//! - watch_positions 保持 NotSupported(kraken 无持仓 WS 频道)。

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

use reqwest::header::HeaderMap;
use serde_json::{Value, json};
use tokio::sync::watch;

use crate::error::{Error, ErrorKind, Result};
use crate::exchange::{Config, Params, Realtime};
use crate::httpcore::{collect_levels, iso8601, now_ms};
use crate::realtime::orderbook::{OrderBookStore, PriceChange};
use crate::realtime::watch::WatchContext;
use crate::realtime::ws::{ChannelMap, Conn, SubscriptionSet, WsSession, ws_err};
use crate::types::{Balances, OHLCV, Order, OrderBook, Position, Ticker, Trade};

const WS_PUBLIC: &str = "wss://ws.kraken.com";
const WS_PRIVATE: &str = "wss://ws-auth.kraken.com";

type TickerChannel = ChannelMap<String, Ticker>;
type BookChannel = ChannelMap<String, OrderBook>;
type BookStoreMap = Arc<Mutex<HashMap<String, OrderBookStore>>>;
type TradeChannel = ChannelMap<String, Vec<Trade>>;
type OhlcvChannel = ChannelMap<(String, String), Vec<OHLCV>>;

/// kraken WS 适配器。
pub struct KrakenWs {
    /// REST 适配器实例:WS 消息复用其 parse_* 解析(解析合一,ADR-0015),
    /// WS token 获取复用其 private_post(签名合一,ADR-0013 sign 接缝)。
    rest: std::sync::Arc<crate::adapters::Kraken>,
    pub_connected: WatchContext,
    priv_connected: Conn,
    tickers: TickerChannel,
    books: BookChannel,
    book_stores: BookStoreMap,
    trades: TradeChannel,
    ohlcvs: OhlcvChannel,
    balances: Mutex<Option<watch::Sender<Balances>>>,
    orders: Mutex<Option<watch::Sender<Vec<Order>>>>,
    my_trades: Mutex<Option<watch::Sender<Vec<Trade>>>>,
    priv_subs: Mutex<SubscriptionSet>,
    priv_sub_tx: Mutex<Option<watch::Sender<Vec<String>>>>,
}

impl KrakenWs {
    pub fn new(config: Config) -> Result<Self> {
        let rest = std::sync::Arc::new(crate::adapters::Kraken::new(config)?);
        Ok(Self {
            rest,
            pub_connected: WatchContext::new(),
            priv_connected: Conn::new(),
            tickers: Arc::new(Mutex::new(HashMap::new())),
            books: Arc::new(Mutex::new(HashMap::new())),
            book_stores: Arc::new(Mutex::new(HashMap::new())),
            trades: Arc::new(Mutex::new(HashMap::new())),
            ohlcvs: Arc::new(Mutex::new(HashMap::new())),
            balances: Mutex::new(None),
            orders: Mutex::new(None),
            my_trades: Mutex::new(None),
            priv_subs: Mutex::new(SubscriptionSet::new()),
            priv_sub_tx: Mutex::new(None),
        })
    }

    fn symbol_id(&self, symbol: &str) -> String {
        // kraken 用 XBT 表示 BTC;pair 用斜杠格式(XBT/USDT)
        symbol.replace("BTC/", "XBT/")
    }

    fn ensure_public(&self) -> watch::Receiver<bool> {
        self.pub_connected.ensure(|sub_tx| {
            let tickers = self.tickers.clone();
            let books = self.books.clone();
            let book_stores = self.book_stores.clone();
            let trades = self.trades.clone();
            let ohlcvs = self.ohlcvs.clone();
            let rest = self.rest.clone();
            let headers = HeaderMap::new();
            WsSession::spawn(
                WS_PUBLIC.to_string(),
                headers,
                sub_tx.subscribe(),
                move |msg| {
                    dispatch_public(msg, &tickers, &books, &book_stores, &trades, &ohlcvs, &rest)
                },
                None,
            )
        })
    }

    fn subscribe(&self, name: &str, pair: &str, interval: Option<&str>) -> Result<()> {
        self.pub_connected.subscribe(&format!("{name}:{pair}"), || {
            let mut sub = json!({"name": name});
            if let Some(iv) = interval {
                sub["interval"] = json!(iv);
            }
            json!({
                "event": "subscribe",
                "pair": [pair],
                "subscription": sub
            })
            .to_string()
        })
    }

    async fn subscribe_private(&self, name: &str, token: &str) -> Result<()> {
        let key = format!("priv:{name}");
        let first = self.priv_subs.lock().unwrap().register(&key);
        if !first {
            return Ok(());
        }
        let tx = self
            .priv_sub_tx
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| Error::new(ErrorKind::NetworkError, "private ws not started"))?;
        let frame = json!({
            "event": "subscribe",
            "subscription": {"name": name, "token": token}
        })
        .to_string();
        tx.send_modify(|list| list.push(frame));
        Ok(())
    }

    async fn fetch_ws_token(&self) -> Result<String> {
        // 复用 REST 适配器 private_post(签名+请求路径同构,sign 接缝),不再内联 SHA512。
        let resp = self
            .rest
            .private_post("/GetWebSocketsToken", &Params::new())
            .await?;
        resp["result"]["token"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| Error::new(ErrorKind::BadResponse, "no ws token"))
    }

    async fn ensure_private(&self) -> Result<()> {
        if self.priv_connected.is_connected() {
            return Ok(());
        }
        let token = self.fetch_ws_token().await?;
        let balances = self.balances.lock().unwrap().clone();
        let orders = self.orders.lock().unwrap().clone();
        let my_trades = self.my_trades.lock().unwrap().clone();
        let headers = HeaderMap::new();
        self.priv_connected.ensure(|| {
            let (sub_tx, sub_rx) = tokio::sync::watch::channel(Vec::new());
            *self.priv_sub_tx.lock().unwrap() = Some(sub_tx);
            let token_clone = token.clone();
            WsSession::spawn(
                WS_PRIVATE.to_string(),
                headers,
                sub_rx,
                move |msg| dispatch_private(msg, &balances, &orders, &my_trades, &token_clone),
                None,
            )
        });
        Ok(())
    }
}

fn dispatch_public(
    msg: Value,
    tickers: &TickerChannel,
    books: &BookChannel,
    book_stores: &BookStoreMap,
    trades: &TradeChannel,
    ohlcvs: &OhlcvChannel,
    rest: &std::sync::Arc<crate::adapters::Kraken>,
) {
    // kraken 公开消息为数组:[channelId, data, channelName, pair]
    let arr = match msg.as_array() {
        Some(a) => a,
        None => return, // 系统消息(event/heartbeat)
    };
    let channel = arr.get(2).and_then(Value::as_str).unwrap_or_default();
    let pair = arr.get(3).and_then(Value::as_str).unwrap_or_default();
    let data = arr.get(1);
    match channel {
        "ticker" => {
            if let (Some(d), Some(tx)) = (data, tickers.lock().unwrap().get(pair).cloned()) {
                // 复用 REST 解析(ADR-0015 解析合一),仅补 WS 实时时间戳。
                let mut t = rest.parse_ticker(d, pair);
                let ts = now_ms();
                t.timestamp = Some(ts);
                t.datetime = Some(iso8601(ts).unwrap_or_default());
                let _ = tx.send(t);
            }
        }
        channel if channel.starts_with("book") => {
            if let (Some(d), Some(tx)) = (data, books.lock().unwrap().get(pair).cloned()) {
                let store = book_stores.lock().unwrap().remove(pair);
                let is_snapshot = d.get("as").is_some() || d.get("bs").is_some();
                let mut store = store.unwrap_or_else(|| OrderBookStore::new(0));
                if is_snapshot {
                    store.reset(&collect_levels(d.get("bs")), &collect_levels(d.get("as")));
                } else {
                    // 增量:a/b 的 [price, vol, ts],vol=0 删除(复用 polymarket 语义)
                    store.apply_polymarket(&parse_changes(d.get("b")), "BUY");
                    store.apply_polymarket(&parse_changes(d.get("a")), "SELL");
                }
                let snap = store.snapshot(pair, None, None, d.clone());
                book_stores.lock().unwrap().insert(pair.to_string(), store);
                let _ = tx.send(snap);
            }
        }
        "trade" => {
            if let (Some(d), Some(tx)) = (data, trades.lock().unwrap().get(pair).cloned()) {
                let rows = d.as_array().cloned().unwrap_or_default();
                let parsed: Vec<Trade> = rows.iter().map(|r| rest.parse_trade(r, pair)).collect();
                let _ = tx.send(parsed);
            }
        }
        channel if channel.starts_with("ohlc") => {
            // kraken WS 频道名为 "ohlc-<interval>"(如 ohlc-5),interval 在频道名后缀。
            let tf = ohlc_interval_from_channel(channel);
            if let (Some(d), Some(tx)) = (
                data,
                ohlcvs.lock().unwrap().get(&(pair.to_string(), tf)).cloned(),
            ) {
                // WS 蜡烛比 REST 多 endtime(索引 1)且 time 为字符串,统一解析前先调整为 REST 形(ADR-0015 接缝)。
                let candle = strip_ohlc_endtime(d);
                let _ = tx.send(vec![rest.parse_ohlcv(&candle)]);
            }
        }
        _ => {}
    }
}

fn dispatch_private(
    msg: Value,
    balances: &Option<watch::Sender<Balances>>,
    orders: &Option<watch::Sender<Vec<Order>>>,
    my_trades: &Option<watch::Sender<Vec<Trade>>>,
    token: &str,
) {
    if let Some(data) = msg.get("channelName").and_then(Value::as_str) {
        let payload = msg.get("data").unwrap_or(&Value::Null);
        match data {
            "balance" => {
                if let Some(tx) = balances.as_ref() {
                    let mut out = Balances {
                        info: msg.clone(),
                        ..Balances::default()
                    };
                    if let Some(obj) = payload.as_object() {
                        for (code, bal) in obj {
                            out.accounts.insert(
                                code.clone(),
                                crate::types::Balance {
                                    free: crate::httpcore::value_decimal(bal),
                                    total: crate::httpcore::value_decimal(bal),
                                    ..crate::types::Balance::default()
                                },
                            );
                        }
                    }
                    let _ = tx.send(out);
                }
            }
            "openOrders" => {
                if let Some(tx) = orders.as_ref() {
                    let mut parsed = Vec::new();
                    if let Some(open) = payload.get("open").and_then(Value::as_object) {
                        for (id, o) in open {
                            let mut order = parse_order(o);
                            if order.id.is_none() {
                                order.id = Some(id.clone());
                            }
                            parsed.push(order);
                        }
                    }
                    let _ = tx.send(parsed);
                }
            }
            "ownTrades" => {
                if let Some(tx) = my_trades.as_ref() {
                    let mut parsed = Vec::new();
                    if let Some(obj) = payload.as_object() {
                        for (id, t) in obj {
                            let mut trade = parse_own_trade(t);
                            if trade.id.is_none() {
                                trade.id = Some(id.clone());
                            }
                            parsed.push(trade);
                        }
                    }
                    let _ = tx.send(parsed);
                }
            }
            _ => {}
        }
    }
    // token 后续订阅需等待认证完成;系统订阅在 watch_* 方法内带 token 发送
    let _ = token;
}

fn parse_order(raw: &Value) -> Order {
    let ts = raw["opentm"].as_f64().map(|f| f as i64 * 1000);
    // descr 在 WS 为字符串,经共享助手归一(与 REST 同一字段映射,ADR-0015)。
    let (symbol, side, order_type) =
        crate::adapters::Kraken::parse_order_descr(raw.get("descr").unwrap_or(&Value::Null));
    Order {
        id: raw["userref"].as_str().map(str::to_string),
        timestamp: ts,
        datetime: ts.and_then(iso8601),
        status: raw["status"]
            .as_str()
            .map(crate::adapters::Kraken::normalize_order_status)
            .map(str::to_string),
        symbol,
        order_type,
        side,
        price: raw["price"].as_str().and_then(|s| s.parse().ok()),
        amount: raw["vol"].as_str().and_then(|s| s.parse().ok()),
        filled: raw["vol_exec"].as_str().and_then(|s| s.parse().ok()),
        info: raw.clone(),
        ..Order::default()
    }
}

fn parse_own_trade(raw: &Value) -> Trade {
    let ts = raw["time"].as_f64().map(|f| f as i64 * 1000);
    Trade {
        timestamp: ts,
        datetime: ts.and_then(iso8601),
        side: raw["type"].as_str().map(str::to_string),
        price: raw["price"].as_str().and_then(|s| s.parse().ok()),
        amount: raw["vol"].as_str().and_then(|s| s.parse().ok()),
        cost: raw["cost"].as_str().and_then(|s| s.parse().ok()),
        fee: raw["fee"]
            .as_str()
            .and_then(|s| s.parse().ok())
            .map(|f| crate::types::Fee {
                cost: Some(f),
                ..crate::types::Fee::default()
            }),
        info: raw.clone(),
        ..Trade::default()
    }
}

/// kraken WS ohlc 频道名形如 "ohlc-5",从中提取 interval(无后缀时默认 "1")。
fn ohlc_interval_from_channel(channel: &str) -> String {
    channel
        .strip_prefix("ohlc-")
        .filter(|s| !s.is_empty())
        .unwrap_or("1")
        .to_string()
}

/// ccxt timeframe → kraken interval(分钟)。未知默认 "1"(与 kraken 默认一致)。
fn kraken_interval(timeframe: &str) -> &'static str {
    match timeframe {
        "1m" => "1",
        "5m" => "5",
        "15m" => "15",
        "30m" => "30",
        "1h" => "60",
        "4h" => "240",
        "1d" => "1440",
        _ => "1",
    }
}

/// WS 蜡烛数组比 REST 多 endtime(索引 1),且 time 为字符串;调整为 REST 形以复用 parse_ohlcv(ADR-0015)。
fn strip_ohlc_endtime(d: &Value) -> Value {
    if let Some(arr) = d.as_array() {
        if arr.len() >= 9 {
            let mut v = arr.clone();
            v.remove(1); // 去掉 endtime
            if let Some(Value::String(s)) = v.first() {
                if let Ok(n) = s.parse::<f64>() {
                    v[0] = json!(n);
                }
            }
            return Value::Array(v);
        }
    }
    d.clone()
}

fn parse_changes(v: Option<&Value>) -> Vec<PriceChange> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|row| {
                    let arr = row.as_array()?;
                    Some(PriceChange {
                        price: crate::httpcore::value_decimal(arr.first()?)?,
                        size: crate::httpcore::value_decimal(arr.get(1)?)?,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

impl Realtime for KrakenWs {
    async fn watch_ticker(&self, symbol: &str, _params: Params) -> Result<Ticker> {
        let pair = self.symbol_id(symbol);
        self.pub_connected
            .watch(&self.tickers, pair, Ticker::default(), |k| async move {
                self.ensure_public();
                self.subscribe("ticker", &k, None)
            })
            .await
    }

    async fn watch_order_book(
        &self,
        symbol: &str,
        _limit: Option<i64>,
        _params: Params,
    ) -> Result<OrderBook> {
        let pair = self.symbol_id(symbol);
        if !self.book_stores.lock().unwrap().contains_key(&pair) {
            self.book_stores
                .lock()
                .unwrap()
                .insert(pair.clone(), OrderBookStore::new(0));
        }
        self.pub_connected
            .watch(&self.books, pair, OrderBook::default(), |k| async move {
                self.ensure_public();
                self.subscribe("book", &k, None)
            })
            .await
    }

    async fn watch_trades(
        &self,
        symbol: &str,
        _since: Option<i64>,
        _limit: Option<i64>,
        _params: Params,
    ) -> Result<Vec<Trade>> {
        let pair = self.symbol_id(symbol);
        self.pub_connected
            .watch(&self.trades, pair, Vec::new(), |k| async move {
                self.ensure_public();
                self.subscribe("trade", &k, None)
            })
            .await
    }

    async fn watch_ohlcv(
        &self,
        symbol: &str,
        timeframe: &str,
        _since: Option<i64>,
        _limit: Option<i64>,
        _params: Params,
    ) -> Result<Vec<OHLCV>> {
        let pair = self.symbol_id(symbol);
        let iv = kraken_interval(timeframe);
        let key = (pair, iv.to_string());
        self.pub_connected
            .watch(&self.ohlcvs, key, Vec::new(), |k| async move {
                self.ensure_public();
                self.subscribe("ohlc", &k.0, Some(k.1.as_str()))
            })
            .await
    }

    async fn watch_balance(&self, _params: Params) -> Result<Balances> {
        self.ensure_private().await?;
        let token = self.fetch_ws_token().await?;
        self.subscribe_private("balance", &token).await?;
        let tx = {
            let mut b = self.balances.lock().unwrap();
            if b.is_none() {
                let (tx, _) = watch::channel(Balances::default());
                *b = Some(tx);
            }
            b.clone().unwrap()
        };
        let mut rx = tx.subscribe();
        rx.changed().await.map_err(ws_err)?;
        Ok(rx.borrow().clone())
    }

    async fn watch_orders(
        &self,
        _symbol: Option<&str>,
        _since: Option<i64>,
        _limit: Option<i64>,
        _params: Params,
    ) -> Result<Vec<Order>> {
        self.ensure_private().await?;
        let token = self.fetch_ws_token().await?;
        self.subscribe_private("openOrders", &token).await?;
        let tx = {
            let mut o = self.orders.lock().unwrap();
            if o.is_none() {
                let (tx, _) = watch::channel(vec![]);
                *o = Some(tx);
            }
            o.clone().unwrap()
        };
        let mut rx = tx.subscribe();
        rx.changed().await.map_err(ws_err)?;
        Ok(rx.borrow().clone())
    }

    async fn watch_my_trades(
        &self,
        _symbol: Option<&str>,
        _since: Option<i64>,
        _limit: Option<i64>,
        _params: Params,
    ) -> Result<Vec<Trade>> {
        self.ensure_private().await?;
        let token = self.fetch_ws_token().await?;
        self.subscribe_private("ownTrades", &token).await?;
        let tx = {
            let mut mt = self.my_trades.lock().unwrap();
            if mt.is_none() {
                let (tx, _) = watch::channel(vec![]);
                *mt = Some(tx);
            }
            mt.clone().unwrap()
        };
        let mut rx = tx.subscribe();
        rx.changed().await.map_err(ws_err)?;
        Ok(rx.borrow().clone())
    }

    async fn watch_positions(
        &self,
        _symbols: Option<&[&str]>,
        _params: Params,
    ) -> Result<Vec<Position>> {
        // kraken 无持仓 WS 频道(与 ccxt 一致)
        Err(Error::not_supported("watch_positions(kraken)"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channels() -> (
        TickerChannel,
        BookChannel,
        BookStoreMap,
        TradeChannel,
        OhlcvChannel,
    ) {
        (
            Arc::new(Mutex::new(HashMap::new())),
            Arc::new(Mutex::new(HashMap::new())),
            Arc::new(Mutex::new(HashMap::new())),
            Arc::new(Mutex::new(HashMap::new())),
            Arc::new(Mutex::new(HashMap::new())),
        )
    }

    fn rest() -> std::sync::Arc<crate::adapters::Kraken> {
        std::sync::Arc::new(crate::adapters::Kraken::new(Config::new()).unwrap())
    }

    #[test]
    fn replay_public_ticker_routes_to_channel() {
        let (tickers, books, stores, trades, ohlcvs) = channels();
        let (tx, rx) = watch::channel(Ticker::default());
        tickers.lock().unwrap().insert("XBT/USDT".into(), tx);
        // kraken WS ticker 内嵌数据对象(与 REST 同形):a/b/c/h/l/o/p/v 为数组
        let msg = json!([
            42,
            {
                "a": ["100.5"], "b": ["100.0"], "c": ["100.2"],
                "h": ["101.0", "101.0"], "l": ["99.0", "99.0"],
                "o": ["98.0"], "p": ["100.1", "100.1"], "v": ["10.0", "200.0"]
            },
            "ticker",
            "XBT/USDT"
        ]);
        dispatch_public(msg, &tickers, &books, &stores, &trades, &ohlcvs, &rest());
        let t = rx.borrow().clone();
        assert_eq!(t.symbol, "XBT/USDT");
        assert_eq!(t.last, Some("100.2".parse().unwrap()));
        assert_eq!(t.ask, Some("100.5".parse().unwrap()));
        assert_eq!(t.bid, Some("100.0".parse().unwrap()));
        assert_eq!(t.high, Some("101.0".parse().unwrap()));
        assert_eq!(t.open, Some("98.0".parse().unwrap()));
    }

    #[test]
    fn replay_public_trade_routes_to_channel() {
        let (tickers, books, stores, trades, ohlcvs) = channels();
        let (tx, rx) = watch::channel(vec![]);
        trades.lock().unwrap().insert("XBT/USDT".into(), tx);
        // kraken WS trade 行:[price, vol, ts(秒), side]
        let msg = json!([
            42,
            [["100.0", "2.0", 1.0, "b"], ["101.0", "1.0", 2.0, "s"]],
            "trade",
            "XBT/USDT"
        ]);
        dispatch_public(msg, &tickers, &books, &stores, &trades, &ohlcvs, &rest());
        let ts = rx.borrow().clone();
        assert_eq!(ts.len(), 2);
        assert_eq!(ts[0].side.as_deref(), Some("buy")); // b → buy
        assert_eq!(ts[0].price, Some("100.0".parse().unwrap()));
        assert!(ts[0].symbol.is_some()); // rest.parse_trade 补 symbol
        assert_eq!(ts[1].side.as_deref(), Some("sell")); // s → sell
    }

    #[test]
    fn replay_public_ohlcv_routes_to_channel() {
        let (tickers, books, stores, trades, ohlcvs) = channels();
        // 注册接收端,key 为 (pair, interval="5"),与 watch_ohlcv 用 kraken_interval("5m") 一致。
        let (tx, rx) = watch::channel(vec![]);
        ohlcvs
            .lock()
            .unwrap()
            .insert(("XBT/USD".into(), "5".into()), tx);
        // kraken WS ohlc 消息:[channelId, 蜡烛, "ohlc-5", pair]
        // 蜡烛含 endtime(索引 1)且 time 为字符串,统一解析前须调整为 REST 形。
        let msg = json!([
            42,
            [
                "1542057314.748456", // time
                "1542057360.435743", // endtime(须剥离)
                "3586.70000",        // open
                "3586.70000",        // high
                "3586.60000",        // low
                "3586.60000",        // close
                "3586.68894",        // vwap
                "0.03373000",        // volume
                2                    // count
            ],
            "ohlc-5",
            "XBT/USD"
        ]);
        dispatch_public(msg, &tickers, &books, &stores, &trades, &ohlcvs, &rest());
        let candles = rx.borrow().clone();
        assert_eq!(candles.len(), 1);
        let c = &candles[0];
        assert_eq!(c.open, Some("3586.70000".parse().unwrap()));
        assert_eq!(c.high, Some("3586.70000".parse().unwrap()));
        assert_eq!(c.low, Some("3586.60000".parse().unwrap()));
        assert_eq!(c.close, Some("3586.60000".parse().unwrap()));
        assert_eq!(c.volume, Some("0.03373000".parse().unwrap()));
        assert_eq!(c.timestamp, Some(1_542_057_314_748)); // 秒 → 毫秒
    }
}
