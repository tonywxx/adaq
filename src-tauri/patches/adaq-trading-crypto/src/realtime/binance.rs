//! binance WebSocket 适配器(M4,ADR-0009):核心 8 频道。
//!
//! 对齐 ccxt.pro binance:
//! - 公开流 `wss://stream.binance.com:9443/ws`,`SUBSCRIBE` 帧;
//! - ticker `<sym>@miniTicker` / trades `<sym>@trade` / klines `<sym>@kline_<tf>`;
//! - 订单簿 `<sym>@depth@100ms`(diff)+ REST `/depth?limit=1000` 快照对账
//!   (u 序列,U/u 丢弃规则,ADR-0011 增量引擎);
//! - 私密流:POST `/api/v3/userDataStream` 取 listenKey → `/ws/<listenKey>`,
//!   `outboundAccountPosition` → 余额、`executionReport` → 订单/成交;
//! - 现货无持仓流,watch_positions 保持 NotSupported(与 ccxt 现货一致)。

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

use reqwest::header::HeaderMap;
use serde_json::{Value, json};
use tokio::sync::watch;

use crate::adapters::binance::Binance;
use crate::error::{Error, ErrorKind, Result};
use crate::exchange::{Config, Exchange, Params, Realtime};
use crate::httpcore::{collect_levels, dec, iso8601};
use crate::realtime::orderbook::OrderBookStore;
use crate::realtime::watch::WatchContext;
use crate::realtime::ws::{ChannelMap, Conn, WsSession, ws_err};
use crate::types::{Balances, OHLCV, Order, OrderBook, Position, Ticker, Trade};

const WS_BASE: &str = "wss://stream.binance.com:9443/ws";

/// symbol + timeframe → OHLCV 频道。
type OhlcvChannel = ChannelMap<(String, String), Vec<OHLCV>>;
type TickerChannel = ChannelMap<String, Ticker>;
type BookChannel = ChannelMap<String, OrderBook>;
type BookStoreMap = Arc<Mutex<HashMap<String, OrderBookStore>>>;
type TradeChannel = ChannelMap<String, Vec<Trade>>;

/// binance WS 适配器。
pub struct BinanceWs {
    /// REST 适配器实例(ADR-0015:持有 REST 实例,复用其 parse_* 与快照/认证)。
    rest: Arc<Binance>,
    /// 公开流连接就绪信号(懒启动,WatchContext 收口)。
    pub_connected: WatchContext,
    tickers: TickerChannel,
    books: BookChannel,
    book_stores: BookStoreMap,
    trades: TradeChannel,
    ohlcvs: OhlcvChannel,
    // 私密流
    user_connected: Conn,
    balances: Mutex<Option<watch::Sender<Balances>>>,
    orders: Mutex<Option<watch::Sender<Vec<Order>>>>,
    my_trades: Mutex<Option<watch::Sender<Vec<Trade>>>>,
}

impl BinanceWs {
    /// 构造 WS 适配器(复用 REST 客户端做快照/认证)。
    pub fn new(config: Config) -> Result<Self> {
        let rest = Arc::new(Binance::new(config)?);
        Ok(Self {
            rest,
            pub_connected: WatchContext::new(),
            tickers: Arc::new(Mutex::new(HashMap::new())),
            books: Arc::new(Mutex::new(HashMap::new())),
            book_stores: Arc::new(Mutex::new(HashMap::new())),
            trades: Arc::new(Mutex::new(HashMap::new())),
            ohlcvs: Arc::new(Mutex::new(HashMap::new())),
            user_connected: Conn::new(),
            balances: Mutex::new(None),
            orders: Mutex::new(None),
            my_trades: Mutex::new(None),
        })
    }

    /// 懒启动公开流连接(单例),返回连接就绪信号。
    fn ensure_public(&self) -> watch::Receiver<bool> {
        self.pub_connected.ensure(|sub_tx| {
            let tickers = self.tickers.clone();
            let books = self.books.clone();
            let book_stores = self.book_stores.clone();
            let trades = self.trades.clone();
            let ohlcvs = self.ohlcvs.clone();
            let rest = self.rest.clone();
            WsSession::spawn(
                WS_BASE.to_string(),
                HeaderMap::new(),
                sub_tx.subscribe(),
                move |msg| {
                    dispatch_public(msg, &tickers, &books, &book_stores, &trades, &ohlcvs, &rest)
                },
                None,
            )
        })
    }

    /// 订阅(仅首次真正发送 SUBSCRIBE 帧)。
    fn subscribe_public(&self, stream: &str) -> Result<()> {
        self.pub_connected.subscribe(stream, || {
            json!({"method": "SUBSCRIBE", "params": [stream], "id": 1}).to_string()
        })
    }

    /// REST 快照初始化订单簿(symbol_id 需小写)。
    async fn ensure_book(
        &self,
        symbol: &str,
        limit: Option<i64>,
    ) -> Result<watch::Receiver<OrderBook>> {
        // 幂等:已存在直接返回
        if let Some(tx) = self.books.lock().unwrap().get(symbol).cloned() {
            return Ok(tx.subscribe());
        }
        // REST 快照委托 Binance 适配器(ADR-0015:持有 REST 实例,复用其快照)
        let ob = self
            .rest
            .fetch_order_book(symbol, limit, Params::new())
            .await?;
        let last_update_id = ob.nonce.unwrap_or(0);
        let bids: Vec<(rust_decimal::Decimal, rust_decimal::Decimal)> = ob
            .bids
            .iter()
            .filter_map(|l| Some((l.price?, l.amount?)))
            .collect();
        let asks: Vec<(rust_decimal::Decimal, rust_decimal::Decimal)> = ob
            .asks
            .iter()
            .filter_map(|l| Some((l.price?, l.amount?)))
            .collect();
        let mut store = OrderBookStore::new(limit.unwrap_or(1000) as usize);
        store.reset(&bids, &asks);
        store.last_update_id = Some(last_update_id as u64);
        let (tx, _) =
            watch::channel(store.snapshot(symbol, None, Some(last_update_id), ob.info.clone()));
        self.book_stores
            .lock()
            .unwrap()
            .insert(symbol.to_string(), store);
        self.books
            .lock()
            .unwrap()
            .insert(symbol.to_string(), tx.clone());
        Ok(tx.subscribe())
    }
}

/// 公开流消息分发(binance 事件格式)。
///
/// 公开行情解析委托 Binance REST 适配器的 `parse_*`(ADR-0015):
/// WS 短键先归一化为 REST 形状,再复用 `Binance::parse_ticker/trade/ohlcv`,
/// 删除实时侧重复的公开行情 parse。用户数据(executionReport)形状差异大,
/// 仍由本地 `parse_exec_*` 处理(与 okx/bybit/kraken 等保持一致)。
fn dispatch_public(
    msg: Value,
    tickers: &TickerChannel,
    books: &BookChannel,
    book_stores: &BookStoreMap,
    trades: &TradeChannel,
    ohlcvs: &OhlcvChannel,
    rest: &Arc<Binance>,
) {
    let event = msg["e"].as_str().unwrap_or_default();
    match event {
        "24hrMiniTicker" | "24hrTicker" => {
            let sym = msg["s"].as_str().unwrap_or_default().to_string();
            if let Some(tx) = tickers.lock().unwrap().get(&sym).cloned() {
                // WS miniTicker 短键 → REST 24hr 形状
                let shape = json!({
                    "symbol": sym,
                    "openPrice": msg["o"], "lastPrice": msg["c"],
                    "highPrice": msg["h"], "lowPrice": msg["l"],
                    "volume": msg["v"], "quoteVolume": msg["q"],
                    "closeTime": msg["E"],
                });
                let _ = tx.send(rest.parse_ticker(&shape));
            }
        }
        "depthUpdate" => {
            let sym = msg["s"].as_str().unwrap_or_default().to_string();
            let u = msg["u"].as_u64().unwrap_or(0);
            let mut stores = book_stores.lock().unwrap();
            let books = books.lock().unwrap();
            if let (Some(store), Some(tx)) = (stores.get_mut(&sym), books.get(&sym).cloned()) {
                let bids = collect_levels(msg.get("b"));
                let asks = collect_levels(msg.get("a"));
                // binance `@depth` 增量用中性序列对账(复用共享 OrderBookStore::apply_sequenced_delta)
                if store.apply_sequenced_delta(u, &bids, &asks) {
                    let _ = tx.send(store.snapshot(
                        &sym,
                        msg["E"].as_i64(),
                        Some(u as i64),
                        msg.clone(),
                    ));
                }
            }
        }
        "trade" => {
            let sym = msg["s"].as_str().unwrap_or_default().to_string();
            if let Some(tx) = trades.lock().unwrap().get(&sym).cloned() {
                // WS trade 短键 → REST trade 形状(id 兼容字符串/数字)
                let id = msg["t"]
                    .as_i64()
                    .or_else(|| msg["t"].as_str().and_then(|s| s.parse::<i64>().ok()));
                let shape = json!({
                    "id": id, "time": msg["T"],
                    "price": msg["p"], "qty": msg["q"],
                    "isBuyerMaker": msg["m"], "symbol": sym,
                });
                let _ = tx.send(vec![rest.parse_trade(&shape)]);
            }
        }
        "kline" => {
            let k = msg.get("k");
            let sym = msg["s"].as_str().unwrap_or_default().to_string();
            let tf = k
                .and_then(|k| k["i"].as_str())
                .unwrap_or_default()
                .to_string();
            if let Some(tx) = ohlcvs
                .lock()
                .unwrap()
                .get(&(sym.clone(), tf.clone()))
                .cloned()
            {
                if let Some(k) = k {
                    // WS kline 对象 → REST klines 行数组
                    let row = json!([k["t"], k["o"], k["h"], k["l"], k["c"], k["v"]]);
                    let _ = tx.send(vec![rest.parse_ohlcv(&row)]);
                }
            }
        }
        _ => {}
    }
}

/// 私密流(listenKey)消息分发。
fn dispatch_user(
    msg: Value,
    balances: &Option<watch::Sender<Balances>>,
    orders: &Option<watch::Sender<Vec<Order>>>,
    my_trades: &Option<watch::Sender<Vec<Trade>>>,
) {
    match msg["e"].as_str().unwrap_or_default() {
        "outboundAccountPosition" => {
            if let Some(tx) = balances.as_ref() {
                let mut out = Balances {
                    info: msg.clone(),
                    ..Balances::default()
                };
                for b in msg["B"].as_array().unwrap_or(&Vec::new()) {
                    if let Some(code) = b["a"].as_str() {
                        let free = dec(b.get("f"));
                        let locked = dec(b.get("l"));
                        out.accounts.insert(
                            code.to_string(),
                            crate::types::Balance {
                                free,
                                used: locked,
                                total: match (free, locked) {
                                    (Some(f), Some(l)) => Some(f + l),
                                    other => other.0,
                                },
                                ..crate::types::Balance::default()
                            },
                        );
                    }
                }
                let _ = tx.send(out);
            }
        }
        "executionReport" => {
            // 订单状态更新
            if let Some(tx) = orders.as_ref() {
                let order = parse_exec_order(&msg);
                let _ = tx.send(vec![order]);
            }
            // 成交部分(仅当有成交价)
            if msg["l"].is_string() || msg["l"].is_number() {
                if let Some(tx) = my_trades.as_ref() {
                    let t = parse_exec_trade(&msg);
                    let _ = tx.send(vec![t]);
                }
            }
        }
        _ => {}
    }
}

fn parse_exec_order(msg: &Value) -> Order {
    let ts = msg["E"].as_i64();
    Order {
        id: msg["i"].as_str().map(str::to_string),
        client_order_id: msg["c"].as_str().map(str::to_string),
        timestamp: ts,
        datetime: ts.and_then(iso8601),
        status: msg["X"]
            .as_str()
            .map(|s| match s {
                "NEW" => "open",
                "PARTIALLY_FILLED" => "open",
                "FILLED" => "closed",
                "CANCELED" | "EXPIRED" => "canceled",
                "REJECTED" => "rejected",
                other => other,
            })
            .map(str::to_string),
        symbol: msg["s"].as_str().map(|s| s.replace('/', "")),
        order_type: msg["o"].as_str().map(|s| s.to_lowercase()),
        side: msg["S"].as_str().map(|s| s.to_lowercase()),
        price: dec(msg.get("p")),
        average: dec(msg.get("L")),
        amount: dec(msg.get("q")),
        filled: dec(msg.get("z")),
        remaining: dec(msg.get("q")).and_then(|q| dec(msg.get("z")).map(|z| q - z)),
        info: msg.clone(),
        ..Order::default()
    }
}

fn parse_exec_trade(msg: &Value) -> Trade {
    let ts = msg["T"].as_i64();
    Trade {
        id: msg["t"].as_str().map(str::to_string),
        timestamp: ts,
        datetime: ts.and_then(iso8601),
        symbol: msg["s"].as_str().map(str::to_string),
        side: msg["S"].as_str().map(|s| s.to_lowercase()),
        price: dec(msg.get("L")),
        amount: dec(msg.get("l")),
        cost: dec(msg.get("l")).and_then(|l| dec(msg.get("L")).map(|p| l * p)),
        info: msg.clone(),
        ..Trade::default()
    }
}

impl Realtime for BinanceWs {
    async fn watch_ticker(&self, symbol: &str, _params: Params) -> Result<Ticker> {
        let sym = symbol.to_uppercase().replace('/', "");
        self.pub_connected
            .watch(&self.tickers, sym, Ticker::default(), |k| async move {
                self.ensure_public();
                self.subscribe_public(&format!("{}@miniTicker", k.to_lowercase()))
            })
            .await
    }

    async fn watch_order_book(
        &self,
        symbol: &str,
        limit: Option<i64>,
        _params: Params,
    ) -> Result<OrderBook> {
        let sym = symbol.to_uppercase().replace('/', "");
        self.ensure_public();
        self.subscribe_public(&format!("{}@depth@100ms", sym.to_lowercase()))?;
        let rx = self.ensure_book(&sym, limit).await?;
        // REST 快照已在 ensure_book 中发出,直接返回当前状态(后续调用为增量最新值)
        Ok(rx.borrow().clone())
    }

    async fn watch_trades(
        &self,
        symbol: &str,
        _since: Option<i64>,
        _limit: Option<i64>,
        _params: Params,
    ) -> Result<Vec<Trade>> {
        let sym = symbol.to_uppercase().replace('/', "");
        self.pub_connected
            .watch(&self.trades, sym, Vec::new(), |k| async move {
                self.ensure_public();
                self.subscribe_public(&format!("{}@trade", k.to_lowercase()))
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
        let sym = symbol.to_uppercase().replace('/', "");
        let key = (sym, timeframe.to_string());
        self.pub_connected
            .watch(&self.ohlcvs, key, Vec::new(), |k| async move {
                self.ensure_public();
                self.subscribe_public(&format!("{}@kline_{}", k.0.to_lowercase(), k.1))
            })
            .await
    }

    async fn watch_balance(&self, _params: Params) -> Result<Balances> {
        let (balance, _, _) = self.ensure_user().await?;
        let mut rx =
            balance.ok_or_else(|| Error::new(ErrorKind::NetworkError, "no balance channel"))?;
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
        let (_, orders, _) = self.ensure_user().await?;
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
        let (_, _, my_trades) = self.ensure_user().await?;
        let mut rx = my_trades;
        rx.changed().await.map_err(ws_err)?;
        Ok(rx.borrow().clone())
    }

    async fn watch_positions(
        &self,
        _symbols: Option<&[&str]>,
        _params: Params,
    ) -> Result<Vec<Position>> {
        // 现货无持仓流(与 ccxt 一致);futures 面后续增量
        Err(Error::not_supported("watch_positions(spot)"))
    }
}

impl BinanceWs {
    /// 懒启动私密流(listenKey),返回 (balance, orders, my_trades) 三个 channel。
    async fn ensure_user(
        &self,
    ) -> Result<(
        Option<watch::Receiver<Balances>>,
        watch::Receiver<Vec<Order>>,
        watch::Receiver<Vec<Trade>>,
    )> {
        if self.user_connected.is_connected() {
            let balance = self.balances.lock().unwrap().clone().map(|t| t.subscribe());
            let orders = self.orders.lock().unwrap().clone().unwrap().subscribe();
            let my_trades = self.my_trades.lock().unwrap().clone().unwrap().subscribe();
            return Ok((balance, orders, my_trades));
        }
        // 获取 listenKey(委托 Binance REST 适配器,ADR-0015)
        let listen_key = self.rest.fetch_listen_key().await?;
        let url = format!("{WS_BASE}/{listen_key}");
        // 用户流以 listenKey 作为 URL 路径,无需额外认证头
        let headers = HeaderMap::new();
        // 初始 channel
        let mut channels = self.balances.lock().unwrap();
        if channels.is_none() {
            let (tx, _) = watch::channel(Balances::default());
            *channels = Some(tx);
        }
        let balances = self.balances.lock().unwrap().clone();
        let mut orders = self.orders.lock().unwrap();
        if orders.is_none() {
            let (tx, _) = watch::channel(vec![]);
            *orders = Some(tx);
        }
        let orders_tx = self.orders.lock().unwrap().clone();
        let mut mt = self.my_trades.lock().unwrap();
        if mt.is_none() {
            let (tx, _) = watch::channel(vec![]);
            *mt = Some(tx);
        }
        let mt_tx = self.my_trades.lock().unwrap().clone();
        let (_, sub_rx) = tokio::sync::watch::channel(Vec::new());
        self.user_connected.ensure(|| {
            WsSession::spawn(
                url,
                headers,
                sub_rx,
                move |msg| dispatch_user(msg, &balances, &orders_tx, &mt_tx),
                None,
            )
        });
        let balance = self.balances.lock().unwrap().clone().map(|t| t.subscribe());
        let orders = self.orders.lock().unwrap().clone().unwrap().subscribe();
        let my_trades = self.my_trades.lock().unwrap().clone().unwrap().subscribe();
        Ok((balance, orders, my_trades))
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
        Arc<Binance>,
    ) {
        (
            Arc::new(Mutex::new(HashMap::new())),
            Arc::new(Mutex::new(HashMap::new())),
            Arc::new(Mutex::new(HashMap::new())),
            Arc::new(Mutex::new(HashMap::new())),
            Arc::new(Mutex::new(HashMap::new())),
            Arc::new(Binance::new(Config::new()).unwrap()),
        )
    }

    /// 离线重放(ADR-0014):合成 binance WS 消息喂 dispatch,验证频道输出。
    #[tokio::test]
    async fn replay_mini_ticker_routes_to_channel() {
        let (tickers, books, stores, trades, ohlcvs, rest) = channels();
        let (tx, rx) = watch::channel(Ticker::default());
        tickers.lock().unwrap().insert("BTCUSDT".into(), tx);
        let msg = json!({
            "e": "24hrMiniTicker", "s": "BTCUSDT", "E": 123,
            "o": "95", "h": "101", "l": "94", "c": "100", "v": "10", "q": "1000"
        });
        dispatch_public(msg, &tickers, &books, &stores, &trades, &ohlcvs, &rest);
        let t = rx.borrow().clone();
        assert_eq!(t.symbol, "BTC/USDT");
        assert_eq!(t.last, Some("100".parse().unwrap()));
        assert_eq!(t.base_volume, Some("10".parse().unwrap()));
    }

    #[tokio::test]
    async fn replay_depth_update_applies_delta() {
        let (tickers, books, stores, trades, ohlcvs, rest) = channels();
        let mut store = OrderBookStore::new(0);
        store.reset(&[("100".parse().unwrap(), "1".parse().unwrap())], &[]);
        store.last_update_id = Some(1);
        stores.lock().unwrap().insert("BTCUSDT".into(), store);
        let (tx, rx) = watch::channel(OrderBook::default());
        books.lock().unwrap().insert("BTCUSDT".into(), tx);
        let msg = json!({
            "e": "depthUpdate", "s": "BTCUSDT", "u": 2, "E": 123,
            "b": [["100", "2"]], "a": [["101", "3"]]
        });
        dispatch_public(msg, &tickers, &books, &stores, &trades, &ohlcvs, &rest);
        let book = rx.borrow().clone();
        assert_eq!(book.bids[0].amount, Some("2".parse().unwrap()));
        assert_eq!(book.asks.len(), 1);
        assert_eq!(book.asks[0].price, Some("101".parse().unwrap()));
    }

    #[tokio::test]
    async fn replay_trade_routes_to_channel() {
        let (tickers, books, stores, trades, ohlcvs, rest) = channels();
        let (tx, rx) = watch::channel(vec![]);
        trades.lock().unwrap().insert("BTCUSDT".into(), tx);
        let msg = json!({"e": "trade", "s": "BTCUSDT", "T": 1, "t": "9", "m": true, "p": "100", "q": "2"});
        dispatch_public(msg, &tickers, &books, &stores, &trades, &ohlcvs, &rest);
        let ts = rx.borrow().clone();
        assert_eq!(ts.len(), 1);
        assert_eq!(ts[0].side.as_deref(), Some("sell")); // m=true → maker sell
        assert_eq!(ts[0].price, Some("100".parse().unwrap()));
    }

    #[tokio::test]
    async fn replay_kline_routes_to_channel() {
        let (tickers, books, stores, trades, ohlcvs, rest) = channels();
        let (tx, rx) = watch::channel(vec![]);
        ohlcvs
            .lock()
            .unwrap()
            .insert(("BTCUSDT".into(), "1m".into()), tx);
        let msg = json!({
            "e": "kline", "s": "BTCUSDT",
            "k": {"t": 1, "i": "1m", "o": "100", "h": "101", "l": "99", "c": "100.5", "v": "3"}
        });
        dispatch_public(msg, &tickers, &books, &stores, &trades, &ohlcvs, &rest);
        let cs = rx.borrow().clone();
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0].close, Some("100.5".parse().unwrap()));
        assert_eq!(cs[0].volume, Some("3".parse().unwrap()));
    }

    #[tokio::test]
    async fn replay_user_execution_report_updates_order() {
        let (tx_o, rx_o) = watch::channel(vec![]);
        let orders = Some(tx_o);
        let msg = json!({
            "e": "executionReport", "E": 1, "s": "BTCUSDT", "i": "123",
            "X": "FILLED", "o": "LIMIT", "S": "BUY", "p": "100", "L": "99", "q": "1", "z": "1"
        });
        dispatch_user(msg, &None, &orders, &None);
        let os = rx_o.borrow().clone();
        assert_eq!(os.len(), 1);
        assert_eq!(os[0].status.as_deref(), Some("closed"));
        assert_eq!(os[0].id.as_deref(), Some("123"));
    }

    #[tokio::test]
    async fn replay_user_balance_position() {
        let (tx_b, rx_b) = watch::channel(Balances::default());
        let balances = Some(tx_b);
        let msg =
            json!({"e": "outboundAccountPosition", "B": [{"a": "BTC", "f": "1.5", "l": "0.5"}]});
        dispatch_user(msg, &balances, &None, &None);
        let b = rx_b.borrow().clone();
        let acct = b.accounts.get("BTC").unwrap();
        assert_eq!(acct.free, Some("1.5".parse().unwrap()));
        assert_eq!(acct.used, Some("0.5".parse().unwrap()));
        assert_eq!(acct.total, Some("2.0".parse().unwrap()));
    }
}
