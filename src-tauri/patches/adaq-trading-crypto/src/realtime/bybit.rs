//! bybit WebSocket 适配器(Phase C,ADR-0009):核心 8 频道。
//!
//! 对齐 bybit v5 WS 协议:
//! - 公开流 `wss://stream.bybit.com/v5/public/spot`,私密流 `.../v5/private`;
//! - 订阅帧 `{"op":"subscribe","args":["ticker.BTCUSDT","orderbook.50.BTCUSDT",...]}`;
//! - orderbook.50:首条 `type=snapshot`(记录 seq),后续 `type=delta` 按 seq 对账合并;
//! - 私密 auth:`{"op":"auth","args":[key, ts, recvWindow, hex(hmac(secret, ts+key+recvWindow))]}`;
//! - 私密频道:wallet(余额)/ order(订单)/ position(仓位)/ execution(成交)。

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

const WS_PUBLIC: &str = "wss://stream.bybit.com/v5/public/spot";
const WS_PRIVATE: &str = "wss://stream.bybit.com/v5/private";

type TickerChannel = ChannelMap<String, Ticker>;
type BookChannel = ChannelMap<String, OrderBook>;
type BookStoreMap = Arc<Mutex<HashMap<String, OrderBookStore>>>;
type TradeChannel = ChannelMap<String, Vec<Trade>>;
type OhlcvChannel = ChannelMap<(String, String), Vec<OHLCV>>;

/// bybit WS 适配器。
pub struct BybitWs {
    config: Config,
    /// REST 适配器实例:WS 消息复用其 parse_* 解析(解析合一,ADR-0013 方向)。
    rest: Arc<crate::adapters::Bybit>,
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

impl BybitWs {
    pub fn new(config: Config) -> Result<Self> {
        let rest = Arc::new(crate::adapters::Bybit::new(config.clone())?);
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
        symbol.replace('/', "")
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

    fn subscribe(&self, arg: &str) -> Result<()> {
        self.pub_connected.subscribe(arg, || {
            json!({"op": "subscribe", "args": [arg]}).to_string()
        })
    }

    async fn subscribe_private(&self, channel: &str) -> Result<()> {
        let key = format!("priv:{channel}");
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
        let frame = json!({"op": "subscribe", "args": [channel]}).to_string();
        tx.send_modify(|list| list.push(frame));
        Ok(())
    }

    async fn ensure_private(&self) -> Result<()> {
        if self.priv_connected.is_connected() {
            return Ok(());
        }
        let api_key = self
            .config
            .api_key
            .as_deref()
            .ok_or_else(|| Error::new(ErrorKind::Authentication, "bybit api_key required"))?;
        let secret = self
            .config
            .secret
            .as_deref()
            .ok_or_else(|| Error::new(ErrorKind::Authentication, "bybit secret required"))?;
        let ts = now_ms().to_string();
        let recv_window = "20000";
        let auth_str = format!("{ts}{api_key}{recv_window}");
        // 复用签名深模块(签名合一,ADR-0013 sign 接缝)
        let signature = crate::signing::hmac_sha256_hex(secret, &auth_str);
        let auth_frame = json!({
            "op": "auth",
            "args": [api_key, ts, recv_window, signature]
        })
        .to_string();
        let balances = self.balances.lock().unwrap().clone();
        let orders = self.orders.lock().unwrap().clone();
        let positions = self.positions.lock().unwrap().clone();
        let my_trades = self.my_trades.lock().unwrap().clone();
        let rest = self.rest.clone();
        let headers = HeaderMap::new();
        self.priv_connected.ensure(|| {
            let (sub_tx, sub_rx) = tokio::sync::watch::channel(vec![auth_frame]);
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
    rest: &crate::adapters::Bybit,
) {
    let topic = msg["topic"].as_str().unwrap_or_default().to_string();
    let data = msg["data"].clone();
    if topic.starts_with("tickers.") {
        if let Some(tx) = tickers
            .lock()
            .unwrap()
            .get(topic.trim_start_matches("tickers."))
            .cloned()
        {
            let _ = tx.send(rest.parse_ticker(&data));
        }
    } else if topic.starts_with("orderbook.") {
        let sym = topic.rsplit('.').next().unwrap_or_default().to_string();
        let mut stores = book_stores.lock().unwrap();
        let books = books.lock().unwrap();
        if let (Some(store), Some(tx)) = (stores.get_mut(&sym), books.get(&sym).cloned()) {
            let seq = data["seq"].as_u64().unwrap_or(0);
            let bids = collect_levels(data.get("b"));
            let asks = collect_levels(data.get("a"));
            match msg["type"].as_str() {
                Some("snapshot") => {
                    store.reset(&bids, &asks);
                    store.last_update_id = Some(seq);
                    let _ = tx.send(store.snapshot(
                        &sym,
                        data["cts"].as_i64(),
                        Some(seq as i64),
                        data.clone(),
                    ));
                }
                // bybit `orderbook` 增量用中性序列对账(复用共享 OrderBookStore::apply_sequenced_delta)
                // bybit `orderbook` deltas use the neutral sequence reconciliation (reusing shared OrderBookStore::apply_sequenced_delta)
                Some("delta") if store.apply_sequenced_delta(seq, &bids, &asks) => {
                    let _ = tx.send(store.snapshot(
                        &sym,
                        data["cts"].as_i64(),
                        Some(seq as i64),
                        data.clone(),
                    ));
                }
                _ => {}
            }
        }
    } else if topic.starts_with("publicTrade.") {
        let sym = topic.trim_start_matches("publicTrade.").to_string();
        if let Some(tx) = trades.lock().unwrap().get(&sym).cloned() {
            let arr = data.as_array().cloned().unwrap_or_default();
            let parsed: Vec<Trade> = arr.iter().map(parse_trade).collect();
            let _ = tx.send(parsed);
        }
    } else if topic.starts_with("kline.") {
        // kline.{interval}.{symbol}
        let parts: Vec<&str> = topic.split('.').collect();
        if parts.len() >= 3 {
            let tf = parts[1].to_string();
            let sym = parts[2..].join(".");
            if let Some(tx) = ohlcvs.lock().unwrap().get(&(sym, tf)).cloned() {
                let arr = data.as_array().cloned().unwrap_or_default();
                let parsed: Vec<OHLCV> = arr.iter().map(parse_candle).collect();
                let _ = tx.send(parsed);
            }
        }
    }
}

fn dispatch_private(
    msg: Value,
    balances: &Option<watch::Sender<Balances>>,
    orders: &Option<watch::Sender<Vec<Order>>>,
    positions: &Option<watch::Sender<Vec<Position>>>,
    my_trades: &Option<watch::Sender<Vec<Trade>>>,
    rest: &crate::adapters::Bybit,
) {
    let topic = msg["topic"].as_str().unwrap_or_default();
    let data = msg["data"].clone();
    match topic {
        "wallet" => {
            if let Some(tx) = balances.as_ref() {
                let mut out = Balances {
                    info: msg.clone(),
                    ..Balances::default()
                };
                if let Some(coins) = data
                    .as_array()
                    .and_then(|a| a.first())
                    .and_then(|d| d.get("coin"))
                    .and_then(Value::as_array)
                {
                    for c in coins {
                        if let Some(code) = c["coin"].as_str() {
                            out.accounts.insert(
                                code.to_string(),
                                crate::types::Balance {
                                    free: dec(c.get("availableToWithdraw")),
                                    total: dec(c.get("walletBalance")),
                                    ..crate::types::Balance::default()
                                },
                            );
                        }
                    }
                }
                let _ = tx.send(out);
            }
        }
        "order" => {
            if let Some(tx) = orders.as_ref() {
                let arr = data.as_array().cloned().unwrap_or_default();
                let parsed: Vec<Order> = arr.iter().map(|o| rest.parse_order(o)).collect();
                let _ = tx.send(parsed);
            }
        }
        "position" => {
            if let Some(tx) = positions.as_ref() {
                let arr = data.as_array().cloned().unwrap_or_default();
                let parsed: Vec<Position> = arr
                    .iter()
                    .filter(|p| p["size"].as_str().map(|s| s != "0").unwrap_or(false))
                    .map(parse_position)
                    .collect();
                let _ = tx.send(parsed);
            }
        }
        "execution" => {
            if let Some(tx) = my_trades.as_ref() {
                let arr = data.as_array().cloned().unwrap_or_default();
                let parsed: Vec<Trade> = arr.iter().map(parse_fill).collect();
                let _ = tx.send(parsed);
            }
        }
        _ => {}
    }
}

fn parse_trade(raw: &Value) -> Trade {
    let ts = raw["T"].as_i64();
    Trade {
        id: raw["i"].as_str().map(str::to_string),
        timestamp: ts,
        datetime: ts.and_then(iso8601),
        symbol: raw["s"].as_str().map(str::to_string),
        side: raw["S"].as_str().map(|s| s.to_lowercase()),
        price: dec(raw.get("p")),
        amount: dec(raw.get("v")),
        info: raw.clone(),
        ..Trade::default()
    }
}

fn parse_candle(raw: &Value) -> OHLCV {
    OHLCV {
        timestamp: raw["start"].as_i64(),
        open: dec(raw.get("open")),
        high: dec(raw.get("high")),
        low: dec(raw.get("low")),
        close: dec(raw.get("close")),
        volume: dec(raw.get("volume")),
    }
}

fn parse_position(raw: &Value) -> Position {
    Position {
        symbol: raw["symbol"].as_str().map(str::to_string),
        id: raw["positionIdx"].as_str().map(str::to_string),
        contracts: dec(raw.get("size")),
        entry_price: dec(raw.get("avgPrice")),
        unrealized_pnl: dec(raw.get("unrealisedPnl")),
        notional: dec(raw.get("positionValue")),
        info: raw.clone(),
        ..Position::default()
    }
}

fn parse_fill(raw: &Value) -> Trade {
    let ts = raw["execTime"].as_i64();
    Trade {
        id: raw["execId"].as_str().map(str::to_string),
        order: raw["orderId"].as_str().map(str::to_string),
        timestamp: ts,
        datetime: ts.and_then(iso8601),
        symbol: raw["symbol"].as_str().map(str::to_string),
        side: raw["side"].as_str().map(|s| s.to_lowercase()),
        price: dec(raw.get("execPrice")),
        amount: dec(raw.get("execQty")),
        cost: dec(raw.get("execFee")),
        info: raw.clone(),
        ..Trade::default()
    }
}

impl Realtime for BybitWs {
    async fn watch_ticker(&self, symbol: &str, _params: Params) -> Result<Ticker> {
        let sym = self.symbol_id(symbol);
        // bybit 现货 ticker 频道为复数形式 tickers.{symbol}
        self.pub_connected
            .watch(&self.tickers, sym, Ticker::default(), |k| async move {
                self.ensure_public();
                self.subscribe(&format!("tickers.{k}"))
            })
            .await
    }

    async fn watch_order_book(
        &self,
        symbol: &str,
        _limit: Option<i64>,
        _params: Params,
    ) -> Result<OrderBook> {
        let sym = self.symbol_id(symbol);
        if !self.book_stores.lock().unwrap().contains_key(&sym) {
            self.book_stores
                .lock()
                .unwrap()
                .insert(sym.clone(), OrderBookStore::new(50));
        }
        self.pub_connected
            .watch(&self.books, sym, OrderBook::default(), |k| async move {
                self.ensure_public();
                self.subscribe(&format!("orderbook.50.{k}"))
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
        let sym = self.symbol_id(symbol);
        // bybit 现货成交频道为 publicTrade.{symbol}
        self.pub_connected
            .watch(&self.trades, sym, Vec::new(), |k| async move {
                self.ensure_public();
                self.subscribe(&format!("publicTrade.{k}"))
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
        let sym = self.symbol_id(symbol);
        let key = (sym, timeframe.to_string());
        self.pub_connected
            .watch(&self.ohlcvs, key, Vec::new(), |k| async move {
                self.ensure_public();
                self.subscribe(&format!("kline.{}.{}", k.1, k.0))
            })
            .await
    }

    async fn watch_balance(&self, _params: Params) -> Result<Balances> {
        self.ensure_private().await?;
        self.subscribe_private("wallet").await?;
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
        self.subscribe_private("order").await?;
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
        self.subscribe_private("execution").await?;
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
        self.subscribe_private("position").await?;
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

    fn rest() -> Arc<crate::adapters::Bybit> {
        Arc::new(crate::adapters::Bybit::new(Config::new()).unwrap())
    }

    /// 离线重放(候选 3 解析合一):WS ticker 数字时间戳经共享 REST parse。
    #[tokio::test]
    async fn replay_ws_ticker_numeric_ts_shared_parse() {
        let rest = rest();
        let tickers: TickerChannel = Arc::new(Mutex::new(HashMap::new()));
        let books: BookChannel = Arc::new(Mutex::new(HashMap::new()));
        let stores: BookStoreMap = Arc::new(Mutex::new(HashMap::new()));
        let trades: TradeChannel = Arc::new(Mutex::new(HashMap::new()));
        let ohlcvs: OhlcvChannel = Arc::new(Mutex::new(HashMap::new()));
        let (tx, rx) = watch::channel(Ticker::default());
        tickers.lock().unwrap().insert("BTCUSDT".into(), tx);
        let msg = json!({
            "topic": "tickers.BTCUSDT",
            "data": {
                "symbol": "BTCUSDT", "timestamp": 123, "high24h": "101", "low24h": "94",
                "bid1Price": "99.9", "ask1Price": "100.1", "lastPrice": "100",
                "volume24h": "10", "turnover24h": "1000"
            }
        });
        dispatch_public(msg, &tickers, &books, &stores, &trades, &ohlcvs, &rest);
        let t = rx.borrow().clone();
        assert_eq!(t.symbol, "BTC/USDT"); // REST 统一 symbol 格式
        assert_eq!(t.timestamp, Some(123)); // 数字时间戳 fallback
        assert_eq!(t.last, Some("100".parse().unwrap()));
    }

    /// 离线重放(候选 3):WS order PartiallyFilled + 数字时间戳经共享 REST parse。
    #[tokio::test]
    async fn replay_ws_order_partially_filled_shared_parse() {
        let rest = rest();
        let (tx_o, rx_o) = watch::channel(vec![]);
        let orders = Some(tx_o);
        let msg = json!({
            "topic": "order",
            "data": [{
                "orderId": "o1", "orderStatus": "PartiallyFilled", "timestamp": 123,
                "symbol": "BTCUSDT", "orderType": "Limit", "side": "Buy",
                "price": "100", "qty": "1", "cumExecQty": "0.5"
            }]
        });
        dispatch_private(msg, &None, &orders, &None, &None, &rest);
        let os = rx_o.borrow().clone();
        assert_eq!(os.len(), 1);
        assert_eq!(os[0].status.as_deref(), Some("open")); // PartiallyFilled→open
        assert_eq!(os[0].timestamp, Some(123)); // 数字时间戳 fallback
    }
}
