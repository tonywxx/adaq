//! polymarket WebSocket 适配器(M4,ADR-0009)。
//!
//! 对齐 ccxt polymarket(仅实现 ccxt 实际支持的 watch_*):
//! - 市场频道 `wss://ws-subscriptions-clob.polymarket.com/ws/market`,
//!   订阅 `{"assets_ids": [tokenId], "type": "market"}`;
//!   `event_type`: `book`(快照重置)/ `price_change`(增量,size 0 删除)/
//!   `last_trade_price`(成交);
//! - 用户频道 `/ws/user`,订阅 `{"auth": {apiKey,secret,passphrase}, "markets": [], "type": "user"}`
//!   → `order` / `trade` 事件;
//! - watch_ticker = 合成(book 中价 + last_trade);watch_ohlcv/balance/positions
//!   与 ccxt 一致保持 NotSupported。

use std::collections::HashMap;
use std::sync::Mutex;

use serde_json::{Value, json};
use tokio::sync::watch;

use crate::error::{Error, ErrorKind, Result};
use crate::exchange::{Config, Exchange, Params, Realtime};
use crate::realtime::orderbook::{OrderBookStore, PriceChange};
use crate::realtime::watch::WatchContext;
use crate::realtime::ws::{WsSession, ws_err};
use crate::types::{Balances, OHLCV, Order, OrderBook, Position, Ticker, Trade};

const MARKET_WS: &str = "wss://ws-subscriptions-clob.polymarket.com/ws/market";
const USER_WS: &str = "wss://ws-subscriptions-clob.polymarket.com/ws/user";

/// polymarket WS 适配器。
pub struct PolymarketWs {
    rest: crate::adapters::Polymarket,
    pub_watch: WatchContext,
    books: std::sync::Arc<Mutex<HashMap<String, watch::Sender<OrderBook>>>>,
    book_stores: std::sync::Arc<Mutex<HashMap<String, OrderBookStore>>>,
    tickers: std::sync::Arc<Mutex<HashMap<String, watch::Sender<Ticker>>>>,
    trades: std::sync::Arc<Mutex<HashMap<String, watch::Sender<Vec<Trade>>>>>,
    user_sub_tx: Mutex<Option<tokio::sync::watch::Sender<Vec<String>>>>,
    orders: Mutex<Option<watch::Sender<Vec<Order>>>>,
    my_trades: Mutex<Option<watch::Sender<Vec<Trade>>>>,
}

impl PolymarketWs {
    pub fn new(config: Config) -> Result<Self> {
        let rest = crate::adapters::Polymarket::new(config)?;
        Ok(Self {
            rest,
            pub_watch: WatchContext::new(),
            books: std::sync::Arc::new(Mutex::new(HashMap::new())),
            book_stores: std::sync::Arc::new(Mutex::new(HashMap::new())),
            tickers: std::sync::Arc::new(Mutex::new(HashMap::new())),
            trades: std::sync::Arc::new(Mutex::new(HashMap::new())),
            user_sub_tx: Mutex::new(None),
            orders: Mutex::new(None),
            my_trades: Mutex::new(None),
        })
    }

    /// 解析 outcome symbol → token_id(懒加载 markets 缓存)。
    async fn token_id(&self, symbol: &str) -> Result<String> {
        self.rest.load_markets().await?;
        Ok(self.rest.resolve_outcome(symbol)?.token_id)
    }

    /// 启动市场频道连接(单例)。
    fn ensure_market(&self) -> watch::Receiver<bool> {
        self.pub_watch.ensure(|sub_tx| {
            let books = self.books.clone();
            let book_stores = self.book_stores.clone();
            let tickers = self.tickers.clone();
            let trades = self.trades.clone();
            let headers = reqwest::header::HeaderMap::new();
            WsSession::spawn(
                MARKET_WS.to_string(),
                headers,
                sub_tx.subscribe(),
                move |msg| dispatch_market(msg, &books, &book_stores, &tickers, &trades),
                None,
            )
        })
    }

    /// 订阅某 token(首次真正发送)。
    fn subscribe_token(&self, token_id: &str) -> Result<()> {
        self.pub_watch.subscribe(token_id, || {
            json!({"assets_ids": [token_id], "type": "market"}).to_string()
        })
    }

    /// 启动用户频道(单例),返回 (orders, my_trades)。
    async fn ensure_user(
        &self,
    ) -> Result<(watch::Receiver<Vec<Order>>, watch::Receiver<Vec<Trade>>)> {
        if let Some(rx) = self.user_sub_tx.lock().unwrap().clone() {
            let orders = self.orders.lock().unwrap().clone().unwrap().subscribe();
            let mt = self.my_trades.lock().unwrap().clone().unwrap().subscribe();
            let _ = rx;
            return Ok((orders, mt));
        }
        let api_key =
            self.rest.config().api_key.as_deref().ok_or_else(|| {
                Error::new(ErrorKind::Authentication, "polymarket apiKey required")
            })?;
        let secret =
            self.rest.config().secret.as_deref().ok_or_else(|| {
                Error::new(ErrorKind::Authentication, "polymarket secret required")
            })?;
        let passphrase =
            self.rest.config().password.as_deref().ok_or_else(|| {
                Error::new(ErrorKind::Authentication, "polymarket password required")
            })?;
        let mut orders_map = self.orders.lock().unwrap();
        if orders_map.is_none() {
            let (tx, _) = watch::channel(vec![]);
            *orders_map = Some(tx);
        }
        let orders_tx = self.orders.lock().unwrap().clone().unwrap();
        let mut mt_map = self.my_trades.lock().unwrap();
        if mt_map.is_none() {
            let (tx, _) = watch::channel(vec![]);
            *mt_map = Some(tx);
        }
        let mt_tx = self.my_trades.lock().unwrap().clone().unwrap();
        let (sub_tx, sub_rx) = tokio::sync::watch::channel(Vec::new());
        let headers = reqwest::header::HeaderMap::new();
        let _ = WsSession::spawn(
            USER_WS.to_string(),
            headers,
            sub_rx,
            move |msg| dispatch_user(msg, &orders_tx, &mt_tx),
            None,
        );
        let auth = json!({
            "auth": {
                "apiKey": api_key,
                "secret": secret,
                "passphrase": passphrase,
            },
            "markets": [],
            "type": "user",
        })
        .to_string();
        sub_tx.send_modify(|list| list.push(auth));
        *self.user_sub_tx.lock().unwrap() = Some(sub_tx);
        let orders = self.orders.lock().unwrap().clone().unwrap().subscribe();
        let mt = self.my_trades.lock().unwrap().clone().unwrap().subscribe();
        Ok((orders, mt))
    }
}

/// 市场频道消息分发。
fn dispatch_market(
    msg: Value,
    books: &Mutex<HashMap<String, watch::Sender<OrderBook>>>,
    book_stores: &Mutex<HashMap<String, OrderBookStore>>,
    tickers: &Mutex<HashMap<String, watch::Sender<Ticker>>>,
    trades: &Mutex<HashMap<String, watch::Sender<Vec<Trade>>>>,
) {
    let event_type = msg["event_type"].as_str().unwrap_or_default();
    let asset_id = msg["asset_id"].as_str().unwrap_or_default().to_string();
    match event_type {
        "book" => {
            // 全量快照 → 重置 store
            if let (Some(store), Some(tx)) = (
                book_stores.lock().unwrap().get_mut(&asset_id),
                books.lock().unwrap().get(&asset_id).cloned(),
            ) {
                let bids = collect_book_levels(msg.get("bids"));
                let asks = collect_book_levels(msg.get("asks"));
                store.reset(&bids, &asks);
                let ts = msg["timestamp"]
                    .as_str()
                    .and_then(|s| s.parse::<i64>().ok());
                let _ = tx.send(store.snapshot(&asset_id, ts, None, msg.clone()));
                // 同时更新合成 ticker
                if let Some(ttx) = tickers.lock().unwrap().get(&asset_id).cloned() {
                    let t = synthetic_ticker(&msg, &asset_id, store);
                    let _ = ttx.send(t);
                }
            }
        }
        "price_change" => {
            let mut stores = book_stores.lock().unwrap();
            let books = books.lock().unwrap();
            if let (Some(store), Some(tx)) =
                (stores.get_mut(&asset_id), books.get(&asset_id).cloned())
            {
                let changes: Vec<PriceChange> = msg["price_changes"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|c| {
                                Some(PriceChange {
                                    price: crate::httpcore::value_decimal(c.get("price")?)?,
                                    size: crate::httpcore::value_decimal(c.get("size")?)?,
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let side = msg["price_changes"]
                    .as_array()
                    .and_then(|a| a.first())
                    .and_then(|c| c["side"].as_str())
                    .unwrap_or("SELL");
                store.apply_polymarket(&changes, side);
                let _ = tx.send(store.snapshot(&asset_id, None, None, msg.clone()));
            }
        }
        "last_trade_price" => {
            if let Some(tx) = trades.lock().unwrap().get(&asset_id).cloned() {
                let _ = tx.send(vec![parse_trade(&msg, &asset_id)]);
            }
            // 更新 ticker last
            if let Some(ttx) = tickers.lock().unwrap().get(&asset_id).cloned() {
                let mut t = ttx.borrow().clone();
                t.last = msg.get("price").and_then(crate::httpcore::value_decimal);
                t.close = t.last;
                let _ = ttx.send(t);
            }
        }
        _ => {}
    }
}

fn collect_book_levels(v: Option<&Value>) -> Vec<(rust_decimal::Decimal, rust_decimal::Decimal)> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|l| {
                    Some((
                        crate::httpcore::value_decimal(l.get("price")?)?,
                        crate::httpcore::value_decimal(l.get("size")?)?,
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn synthetic_ticker(msg: &Value, asset_id: &str, store: &OrderBookStore) -> Ticker {
    let snap = store.snapshot(asset_id, None, None, msg.clone());
    let bid = snap.bids.first().and_then(|l| l.price);
    let ask = snap.asks.first().and_then(|l| l.price);
    let mid = match (bid, ask) {
        (Some(b), Some(a)) => Some((b + a) / rust_decimal::Decimal::from(2)),
        _ => None,
    };
    let last = msg
        .get("last_trade_price")
        .and_then(crate::httpcore::value_decimal)
        .or(mid);
    Ticker {
        symbol: asset_id.to_string(),
        timestamp: msg["timestamp"]
            .as_str()
            .and_then(|s| s.parse::<i64>().ok()),
        bid,
        ask,
        bid_volume: snap.bids.first().and_then(|l| l.amount),
        ask_volume: snap.asks.first().and_then(|l| l.amount),
        last,
        close: last,
        average: mid,
        info: msg.clone(),
        ..Ticker::default()
    }
}

fn parse_trade(msg: &Value, asset_id: &str) -> Trade {
    let price = msg.get("price").and_then(crate::httpcore::value_decimal);
    let amount = msg.get("size").and_then(crate::httpcore::value_decimal);
    Trade {
        id: msg["transaction_hash"].as_str().map(str::to_string),
        timestamp: msg["timestamp"]
            .as_str()
            .and_then(|s| s.parse::<i64>().ok()),
        symbol: Some(asset_id.to_string()),
        side: msg["side"].as_str().map(|s| s.to_lowercase()),
        price,
        amount,
        cost: match (price, amount) {
            (Some(p), Some(a)) => Some(p * a),
            _ => None,
        },
        info: msg.clone(),
        ..Trade::default()
    }
}

/// 用户频道消息分发(order / trade 事件)。
fn dispatch_user(
    msg: Value,
    orders: &watch::Sender<Vec<Order>>,
    my_trades: &watch::Sender<Vec<Trade>>,
) {
    match msg["event_type"].as_str().unwrap_or_default() {
        "order" => {
            let mut o = Order {
                id: msg["id"].as_str().map(str::to_string),
                status: msg["status"]
                    .as_str()
                    .map(|s| match s {
                        "live" => "open",
                        "matched" => "closed",
                        "cancelled" => "canceled",
                        other => other,
                    })
                    .map(str::to_string),
                side: msg["side"].as_str().map(|s| s.to_lowercase()),
                price: msg.get("price").and_then(crate::httpcore::value_decimal),
                amount: msg
                    .get("original_size")
                    .and_then(crate::httpcore::value_decimal),
                filled: msg
                    .get("size_matched")
                    .and_then(crate::httpcore::value_decimal),
                info: msg.clone(),
                ..Order::default()
            };
            if o.amount.is_some() && o.filled.is_some() {
                o.remaining = o.amount.zip(o.filled).map(|(a, f)| a - f);
            }
            let _ = orders.send(vec![o]);
        }
        "trade" => {
            let price = msg.get("price").and_then(crate::httpcore::value_decimal);
            let amount = msg.get("size").and_then(crate::httpcore::value_decimal);
            let t = Trade {
                id: msg["transaction_hash"].as_str().map(str::to_string),
                side: msg["side"].as_str().map(|s| s.to_lowercase()),
                price,
                amount,
                cost: match (price, amount) {
                    (Some(p), Some(a)) => Some(p * a),
                    _ => None,
                },
                info: msg.clone(),
                ..Trade::default()
            };
            let _ = my_trades.send(vec![t]);
        }
        _ => {}
    }
}

impl Realtime for PolymarketWs {
    async fn watch_ticker(&self, symbol: &str, _params: Params) -> Result<Ticker> {
        let token_id = self.token_id(symbol).await?;
        self.pub_watch
            .watch(&self.tickers, token_id, Ticker::default(), |k| async move {
                self.ensure_market();
                self.subscribe_token(&k)
            })
            .await
    }

    async fn watch_order_book(
        &self,
        symbol: &str,
        _limit: Option<i64>,
        _params: Params,
    ) -> Result<OrderBook> {
        let token_id = self.token_id(symbol).await?;
        if !self.book_stores.lock().unwrap().contains_key(&token_id) {
            self.book_stores
                .lock()
                .unwrap()
                .insert(token_id.clone(), OrderBookStore::new(0));
        }
        self.pub_watch
            .watch(
                &self.books,
                token_id,
                OrderBook::default(),
                |k| async move {
                    self.ensure_market();
                    self.subscribe_token(&k)
                },
            )
            .await
    }

    async fn watch_trades(
        &self,
        symbol: &str,
        _since: Option<i64>,
        _limit: Option<i64>,
        _params: Params,
    ) -> Result<Vec<Trade>> {
        let token_id = self.token_id(symbol).await?;
        self.pub_watch
            .watch(&self.trades, token_id, Vec::new(), |k| async move {
                self.ensure_market();
                self.subscribe_token(&k)
            })
            .await
    }

    async fn watch_ohlcv(
        &self,
        _symbol: &str,
        _timeframe: &str,
        _since: Option<i64>,
        _limit: Option<i64>,
        _params: Params,
    ) -> Result<Vec<OHLCV>> {
        Err(Error::not_supported("watch_ohlcv(polymarket)"))
    }

    async fn watch_balance(&self, _params: Params) -> Result<Balances> {
        Err(Error::not_supported("watch_balance(polymarket)"))
    }

    async fn watch_orders(
        &self,
        _symbol: Option<&str>,
        _since: Option<i64>,
        _limit: Option<i64>,
        _params: Params,
    ) -> Result<Vec<Order>> {
        let (orders, _) = self.ensure_user().await?;
        let mut rx = orders;
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
        let (_, my_trades) = self.ensure_user().await?;
        let mut rx = my_trades;
        rx.changed().await.map_err(ws_err)?;
        Ok(rx.borrow().clone())
    }

    async fn watch_positions(
        &self,
        _symbols: Option<&[&str]>,
        _params: Params,
    ) -> Result<Vec<Position>> {
        Err(Error::not_supported("watch_positions(polymarket)"))
    }
}
