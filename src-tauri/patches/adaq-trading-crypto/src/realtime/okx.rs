//! okx WebSocket 适配器(Phase C,ADR-0009):核心 8 频道。
//!
//! 对齐 OKX v5 WS 协议:
//! - 公开流 `wss://ws.okx.com:8443/ws/v5/public`,私密流 `.../ws/v5/private`;
//! - 订阅帧 `{"op":"subscribe","args":[{"channel":"...","instId":"BTC-USDT"}]}`;
//! - tickers / trades / candle{tf} / books(增量,seqId 对账)为公开频道;
//!   account / orders / positions / fills 为私密频道(先 login);
//! - 私密登录:`sign = base64(HMAC-SHA256(secret, ts + "GET" + "/users/self/verify"))`;
//! - books:`action=snapshot` 初始化(记录 seqId),`action=update` 校验
//!   seqId 连续后合并增量(复用 OrderBookStore::apply_sequenced_delta 的序列对账)。
//!   Merges deltas after seqId is contiguous (reusing OrderBookStore::apply_sequenced_delta sequence reconciliation).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

use reqwest::header::HeaderMap;
use serde_json::{Value, json};
use tokio::sync::watch;

use crate::error::{Error, ErrorKind, Result};
use crate::exchange::{Config, Params, Realtime};
use crate::httpcore::{collect_levels, dec, iso8601, now_ms};
use crate::realtime::orderbook::OrderBookStore;
use crate::realtime::watch::WatchContext;
use crate::realtime::ws::{ChannelMap, Conn, SubscriptionSet, WsSession, ws_err};
use crate::types::{Balances, OHLCV, Order, OrderBook, Position, Ticker, Trade};

const WS_PUBLIC: &str = "wss://ws.okx.com:8443/ws/v5/public";
const WS_PRIVATE: &str = "wss://ws.okx.com:8443/ws/v5/private";

type TickerChannel = ChannelMap<String, Ticker>;
type BookChannel = ChannelMap<String, OrderBook>;
type BookStoreMap = Arc<Mutex<HashMap<String, OrderBookStore>>>;
type TradeChannel = ChannelMap<String, Vec<Trade>>;
type OhlcvChannel = ChannelMap<(String, String), Vec<OHLCV>>;

/// okx WS 适配器。
pub struct OkxWs {
    config: Config,
    /// REST 适配器实例:WS 消息复用其 parse_* 解析(解析合一,ADR-0013 方向)。
    rest: Arc<crate::adapters::Okx>,
    pub_connected: WatchContext,
    priv_connected: Conn,
    tickers: TickerChannel,
    books: BookChannel,
    book_stores: BookStoreMap,
    trades: TradeChannel,
    ohlcvs: OhlcvChannel,
    balances: Mutex<Option<watch::Sender<Balances>>>,
    orders: Mutex<Option<watch::Sender<Vec<Order>>>>,
    positions: Mutex<Option<watch::Sender<Vec<Position>>>>,
    my_trades: Mutex<Option<watch::Sender<Vec<Trade>>>>,
    priv_subs: Mutex<SubscriptionSet>,
    priv_sub_tx: Mutex<Option<watch::Sender<Vec<String>>>>,
}

impl OkxWs {
    pub fn new(config: Config) -> Result<Self> {
        let rest = Arc::new(crate::adapters::Okx::new(config.clone())?);
        Ok(Self {
            config,
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
            positions: Mutex::new(None),
            my_trades: Mutex::new(None),
            priv_subs: Mutex::new(SubscriptionSet::new()),
            priv_sub_tx: Mutex::new(None),
        })
    }

    fn symbol_id(&self, symbol: &str) -> String {
        symbol.replace('/', "-")
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

    fn subscribe(&self, channel: String, inst_id: String) -> Result<()> {
        self.pub_connected
            .subscribe(&format!("{channel}:{inst_id}"), || {
                json!({
                    "op": "subscribe",
                    "args": [{"channel": channel, "instId": inst_id}]
                })
                .to_string()
            })
    }

    async fn subscribe_private(&self, channel: &str) -> Result<()> {
        let first = self
            .priv_subs
            .lock()
            .unwrap()
            .register(&format!("priv:{channel}"));
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
            "op": "subscribe",
            "args": [{"channel": channel, "instType": "SPOT"}]
        })
        .to_string();
        tx.send_modify(|list| list.push(frame));
        Ok(())
    }

    async fn ensure_private(&self) -> Result<()> {
        if self.priv_connected.is_connected() {
            return Ok(());
        }
        // 构造登录帧与私密连接
        let api_key = self
            .config
            .api_key
            .as_deref()
            .ok_or_else(|| Error::new(ErrorKind::Authentication, "okx api_key required"))?;
        let passphrase = self
            .config
            .password
            .as_deref()
            .ok_or_else(|| Error::new(ErrorKind::Authentication, "okx password required"))?;
        let ts = now_ms().to_string();
        // 复用 REST 适配器 sign_str(签名合一,ADR-0013 sign 接缝;登录帧 = ts+GET+/users/self/verify+空 body)
        let sign = self.rest.sign_str(&ts, "GET", "/users/self/verify", "")?;
        let login_frame = json!({
            "op": "login",
            "args": [{"apiKey": api_key, "passphrase": passphrase, "timestamp": ts, "sign": sign}]
        })
        .to_string();
        let balances = self.balances.lock().unwrap().clone();
        let orders = self.orders.lock().unwrap().clone();
        let positions = self.positions.lock().unwrap().clone();
        let my_trades = self.my_trades.lock().unwrap().clone();
        let rest = self.rest.clone();
        let headers = HeaderMap::new();
        self.priv_connected.ensure(|| {
            let (sub_tx, sub_rx) = tokio::sync::watch::channel(vec![login_frame]);
            *self.priv_sub_tx.lock().unwrap() = Some(sub_tx);
            WsSession::spawn(
                WS_PRIVATE.to_string(),
                headers,
                sub_rx,
                move |msg| dispatch_private(msg, &balances, &orders, &positions, &my_trades, &rest),
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
    rest: &crate::adapters::Okx,
) {
    let channel = msg["arg"]["channel"].as_str().unwrap_or_default();
    let inst = msg["arg"]["instId"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let data = msg["data"].as_array().cloned().unwrap_or_default();
    if data.is_empty() {
        return;
    }
    match channel {
        "tickers" => {
            if let Some(tx) = tickers.lock().unwrap().get(&inst).cloned() {
                let _ = tx.send(rest.parse_ticker(&data[0]));
            }
        }
        "books" => {
            let mut stores = book_stores.lock().unwrap();
            let books = books.lock().unwrap();
            if let (Some(store), Some(tx)) = (stores.get_mut(&inst), books.get(&inst).cloned()) {
                let seq = data[0]["seqId"].as_u64().unwrap_or(0);
                let bids = collect_levels(data[0].get("bids"));
                let asks = collect_levels(data[0].get("asks"));
                match msg["action"].as_str() {
                    Some("snapshot") => {
                        store.reset(&bids, &asks);
                        store.last_update_id = Some(seq);
                        let _ = tx.send(store.snapshot(
                            &inst.replace('-', "/"),
                            data[0]["ts"].as_str().and_then(|s| s.parse::<i64>().ok()),
                            Some(seq as i64),
                            data[0].clone(),
                        ));
                    }
                    // okx `books` 增量用中性序列对账(复用共享 OrderBookStore::apply_sequenced_delta)
                    // okx `books` deltas use the neutral sequence reconciliation (reusing shared OrderBookStore::apply_sequenced_delta)
                    Some("update") if store.apply_sequenced_delta(seq, &bids, &asks) => {
                        let _ = tx.send(store.snapshot(
                            &inst.replace('-', "/"),
                            data[0]["ts"].as_str().and_then(|s| s.parse::<i64>().ok()),
                            Some(seq as i64),
                            data[0].clone(),
                        ));
                    }
                    _ => {}
                }
            }
        }
        "trades" => {
            if let Some(tx) = trades.lock().unwrap().get(&inst).cloned() {
                let parsed: Vec<Trade> = data.iter().map(|t| rest.parse_trade(t)).collect();
                let _ = tx.send(parsed);
            }
        }
        channel if channel.starts_with("candle") => {
            // candle1m → (inst, "1m")
            let tf = channel.trim_start_matches("candle").to_string();
            if let Some(tx) = ohlcvs
                .lock()
                .unwrap()
                .get(&(inst.clone(), tf.clone()))
                .cloned()
            {
                let _ = tx.send(vec![rest.parse_ohlcv(&data[0])]);
            }
        }
        _ => {}
    }
}

fn dispatch_private(
    msg: Value,
    balances: &Option<watch::Sender<Balances>>,
    orders: &Option<watch::Sender<Vec<Order>>>,
    positions: &Option<watch::Sender<Vec<Position>>>,
    my_trades: &Option<watch::Sender<Vec<Trade>>>,
    rest: &crate::adapters::Okx,
) {
    let channel = msg["arg"]["channel"].as_str().unwrap_or_default();
    let data = msg["data"].as_array().cloned().unwrap_or_default();
    if data.is_empty() {
        return;
    }
    match channel {
        "account" => {
            if let Some(tx) = balances.as_ref() {
                let mut out = Balances {
                    info: msg.clone(),
                    ..Balances::default()
                };
                if let Some(details) = data[0].get("details").and_then(Value::as_array) {
                    for d in details {
                        if let Some(code) = d["ccy"].as_str() {
                            let cash = dec(d.get("cashBal"));
                            let avail = dec(d.get("availBal"));
                            out.accounts.insert(
                                code.to_string(),
                                crate::types::Balance {
                                    free: avail.or(cash),
                                    total: cash,
                                    ..crate::types::Balance::default()
                                },
                            );
                        }
                    }
                }
                let _ = tx.send(out);
            }
        }
        "orders" => {
            if let Some(tx) = orders.as_ref() {
                let parsed: Vec<Order> = data.iter().map(|o| rest.parse_order(o)).collect();
                let _ = tx.send(parsed);
            }
        }
        "positions" => {
            if let Some(tx) = positions.as_ref() {
                let parsed: Vec<Position> = data
                    .iter()
                    .filter(|p| p["pos"].as_str().map(|s| s != "0").unwrap_or(false))
                    .map(|p| rest.parse_position(p))
                    .collect();
                let _ = tx.send(parsed);
            }
        }
        "fills" => {
            if let Some(tx) = my_trades.as_ref() {
                let parsed: Vec<Trade> = data.iter().map(parse_fill).collect();
                let _ = tx.send(parsed);
            }
        }
        _ => {}
    }
}

fn parse_fill(raw: &Value) -> Trade {
    let ts = raw["ts"].as_str().and_then(|s| s.parse::<i64>().ok());
    Trade {
        id: raw["tradeId"].as_str().map(str::to_string),
        order: raw["ordId"].as_str().map(str::to_string),
        timestamp: ts,
        datetime: ts.and_then(iso8601),
        symbol: raw["instId"].as_str().map(|s| s.replace('-', "/")),
        side: raw["side"].as_str().map(|s| s.to_lowercase()),
        price: dec(raw.get("px")),
        amount: dec(raw.get("sz")),
        cost: dec(raw.get("fillPnl")),
        info: raw.clone(),
        ..Trade::default()
    }
}

impl Realtime for OkxWs {
    async fn watch_ticker(&self, symbol: &str, _params: Params) -> Result<Ticker> {
        let inst = self.symbol_id(symbol);
        self.pub_connected
            .watch(&self.tickers, inst, Ticker::default(), |k| async move {
                self.ensure_public();
                self.subscribe("tickers".to_string(), k)
            })
            .await
    }

    async fn watch_order_book(
        &self,
        symbol: &str,
        _limit: Option<i64>,
        _params: Params,
    ) -> Result<OrderBook> {
        let inst = self.symbol_id(symbol);
        if !self.book_stores.lock().unwrap().contains_key(&inst) {
            self.book_stores
                .lock()
                .unwrap()
                .insert(inst.clone(), OrderBookStore::new(0));
        }
        self.pub_connected
            .watch(&self.books, inst, OrderBook::default(), |k| async move {
                self.ensure_public();
                self.subscribe("books".to_string(), k)
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
        let inst = self.symbol_id(symbol);
        self.pub_connected
            .watch(&self.trades, inst, Vec::new(), |k| async move {
                self.ensure_public();
                self.subscribe("trades".to_string(), k)
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
        let inst = self.symbol_id(symbol);
        let key = (inst, timeframe.to_string());
        let channel = format!("candle{timeframe}");
        self.pub_connected
            .watch(&self.ohlcvs, key, Vec::new(), |k| async move {
                self.ensure_public();
                self.subscribe(channel.clone(), k.0)
            })
            .await
    }

    async fn watch_balance(&self, _params: Params) -> Result<Balances> {
        self.ensure_private().await?;
        self.subscribe_private("account").await?;
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
        self.subscribe_private("orders").await?;
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
        self.subscribe_private("fills").await?;
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
        self.ensure_private().await?;
        self.subscribe_private("positions").await?;
        let tx = {
            let mut p = self.positions.lock().unwrap();
            if p.is_none() {
                let (tx, _) = watch::channel(vec![]);
                *p = Some(tx);
            }
            p.clone().unwrap()
        };
        let mut rx = tx.subscribe();
        rx.changed().await.map_err(ws_err)?;
        Ok(rx.borrow().clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 离线重放(候选 3 解析合一):WS tickers 消息经共享 REST parse 输出频道。
    #[tokio::test]
    async fn replay_ws_ticker_uses_shared_rest_parse() {
        let rest = Arc::new(crate::adapters::Okx::new(Config::new()).unwrap());
        let tickers: TickerChannel = Arc::new(Mutex::new(HashMap::new()));
        let books: BookChannel = Arc::new(Mutex::new(HashMap::new()));
        let stores: BookStoreMap = Arc::new(Mutex::new(HashMap::new()));
        let trades: TradeChannel = Arc::new(Mutex::new(HashMap::new()));
        let ohlcvs: OhlcvChannel = Arc::new(Mutex::new(HashMap::new()));
        let (tx, rx) = watch::channel(Ticker::default());
        tickers.lock().unwrap().insert("BTC-USDT".into(), tx);
        let msg = json!({
            "arg": {"channel": "tickers", "instId": "BTC-USDT"},
            "data": [{
                "instId": "BTC-USDT", "ts": "123", "last": "100",
                "bidPx": "99.9", "askPx": "100.1", "open24h": "95",
                "high24h": "101", "low24h": "94", "vol24h": "10", "volCcy24h": "1000"
            }]
        });
        dispatch_public(msg, &tickers, &books, &stores, &trades, &ohlcvs, &rest);
        let t = rx.borrow().clone();
        assert_eq!(t.symbol, "BTC/USDT");
        assert_eq!(t.last, Some("100".parse().unwrap()));
        assert_eq!(t.timestamp, Some(123));
    }
}
