//! WebSocket 连接基础设施(ADR-0009 / ADR-0014):连接建立(可选自定义头)、
//! 订阅命令通道、接收循环、心跳与重连退避。
//!
//! 仅编译于 `realtime` feature。每个交易所的 watch_* 适配器在后台 spawn 一个
//! [`WsSession`]:连接任务用 `tokio::select!` 同时处理收到的消息、调用方的订阅
//! 命令(mpsc)与可选心跳;消息解析后分发给 `watch` channel;watch_* 方法等待
//! [`tokio::sync::watch`] 的下一条更新。
//!
//! ADR-0014 收口:各适配器逐字重复的 `wait_first`/`ws_err`/频道别名/`ensure_public`
//! 单例模板集中于此([`wait_first`]/[`ws_err`]/[`ChannelMap`]/[`Conn`]);会话循环
//! 保持 socket 绑定,纯策略([`next_backoff`]/[`heartbeat_tick`])可单测;适配器的
//! dispatch 解析逻辑以离线重放测试覆盖(见各适配器 `#[cfg(test)]`)。

use std::collections::HashMap;
#[cfg(feature = "realtime")]
use std::collections::HashSet;
#[cfg(feature = "realtime")]
use std::sync::Arc;
#[cfg(feature = "realtime")]
use std::sync::Mutex;
#[cfg(feature = "realtime")]
use std::time::Duration;

#[cfg(feature = "realtime")]
use futures_util::{SinkExt, StreamExt};
use reqwest::header::HeaderMap;
use tokio::net::TcpStream;
#[cfg(feature = "realtime")]
use tokio::sync::watch;
#[cfg(feature = "realtime")]
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::error::{Error, ErrorKind, Result};

/// WS 连接类型。
type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// 建立 WS 连接(带自定义请求头,如 kalshi 认证头)。
pub async fn connect(url: &str, headers: &HeaderMap) -> Result<WsStream> {
    let mut req = url.into_client_request().map_err(|e| {
        Error::new(
            ErrorKind::NetworkError,
            format!("invalid ws url {url}: {e}"),
        )
    })?;
    for (k, v) in headers.iter() {
        req.headers_mut().insert(k.clone(), v.clone());
    }
    let (ws, _) = tokio_tungstenite::connect_async(req)
        .await
        .map_err(|e| Error::new(ErrorKind::NetworkError, format!("ws connect {url}: {e}")))?;
    Ok(ws)
}

/// 频道注册表:`Arc<Mutex<HashMap<K, watch::Sender<V>>>>`。
///
/// 原各适配器逐字重复的 `TickerChannel`/`BookChannel`/`TradeChannel`/`OhlcvChannel`
/// 别名统一为此泛型(ADR-0014)。
#[cfg(feature = "realtime")]
pub type ChannelMap<K, V> = Arc<Mutex<HashMap<K, watch::Sender<V>>>>;

/// 连接就绪信号缓存(原各适配器 `ensure_public` 单例模板,ADR-0014)。
#[cfg(feature = "realtime")]
#[derive(Debug, Default)]
pub struct Conn {
    inner: Mutex<Option<watch::Receiver<bool>>>,
}

#[cfg(feature = "realtime")]
impl Conn {
    pub fn new() -> Self {
        Self::default()
    }

    /// 已连接返回缓存信号;否则调用 `spawn` 建立连接并缓存。
    ///
    /// `spawn` 闭包只会在首次调用时执行一次(含其副作用,如初始化订阅发送端)。
    pub fn ensure(&self, spawn: impl FnOnce() -> watch::Receiver<bool>) -> watch::Receiver<bool> {
        let mut guard = self.inner.lock().unwrap();
        if let Some(rx) = guard.as_ref() {
            return rx.clone();
        }
        let rx = spawn();
        *guard = Some(rx.clone());
        rx
    }

    /// 是否已建立连接(不触发建立)。
    ///
    /// 用于"先做异步准备工作(如取 listenKey/认证),再同步 spawn"的流程:
    /// 避免把 async 工作包进 [`Conn::ensure`] 的同步闭包。
    pub fn is_connected(&self) -> bool {
        self.inner.lock().unwrap().is_some()
    }
}

/// 等待 watch channel 首个更新(原各适配器 `wait_first`,ADR-0014)。
#[cfg(feature = "realtime")]
pub async fn wait_first<T: Clone>(mut rx: watch::Receiver<T>) -> Result<T> {
    rx.changed().await.map_err(ws_err)?;
    Ok(rx.borrow().clone())
}

/// watch channel 关闭错误(原各适配器 `ws_err`,ADR-0014)。
#[cfg(feature = "realtime")]
pub fn ws_err(e: watch::error::RecvError) -> Error {
    Error::new(ErrorKind::NetworkError, format!("ws channel closed: {e}"))
}

/// 应用层心跳:间隔 + 心跳帧构造器(每交易所编码不同,
/// 如 okx `"ping"`、kraken `{"event":"heartbeat"}`)。
#[cfg(feature = "realtime")]
pub type Heartbeat = (Duration, Arc<dyn Fn() -> Message + Send + Sync>);

/// 重连退避:指数增长,封顶 30s,加亚秒抖动(ADR-0014)。
#[cfg(feature = "realtime")]
fn next_backoff(ms: u64) -> u64 {
    let base = ms.saturating_mul(2).min(30_000);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    (base + (nanos % 1000)).min(30_000)
}

/// 心跳 tick:有间隔则等待下一次,无心跳则永久挂起(select! 分支用)。
#[cfg(feature = "realtime")]
async fn heartbeat_tick(interval: &mut Option<tokio::time::Interval>) {
    match interval.as_mut() {
        Some(i) => {
            i.tick().await;
        }
        None => std::future::pending::<()>().await,
    }
}

/// 启动一个长连接会话。
///
/// - `sub_rx`:订阅命令通道,调用方通过它发送订阅 JSON 帧(如 binance `SUBSCRIBE`);
/// - `dispatch`:收到的 JSON 消息分发回调;
/// - `heartbeat`:应用层心跳(None 表示不发送),断线重连采用指数退避(ADR-0014);
/// - 返回连接就绪信号(watch)。
#[cfg(feature = "realtime")]
impl WsSession<'_> {
    pub fn spawn<F>(
        url: String,
        headers: HeaderMap,
        sub_rx: watch::Receiver<Vec<String>>,
        dispatch: F,
        heartbeat: Option<Heartbeat>,
    ) -> watch::Receiver<bool>
    where
        F: Fn(serde_json::Value) + Send + Sync + 'static,
    {
        let (tx, rx) = watch::channel(false);
        let dispatch = Arc::new(dispatch);
        tokio::spawn(async move {
            // 本地已发送订阅帧集合(重连后无需重发)
            let mut sent: HashSet<String> = HashSet::new();
            // 重连退避(ADR-0014):从 1s 起步,指数增长封顶 30s
            let mut backoff_ms: u64 = 1_000;
            loop {
                let ws = match connect(&url, &headers).await {
                    Ok(w) => w,
                    Err(e) => {
                        eprintln!(
                            "[realtime] connect {url} failed: {e}; retry in {}ms",
                            backoff_ms
                        );
                        tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                        backoff_ms = next_backoff(backoff_ms);
                        continue;
                    }
                };
                let _ = tx.send(true);
                let mut session = WsSession {
                    ws,
                    sub_rx: sub_rx.clone(),
                    dispatch: dispatch.clone(),
                    sent: &mut sent,
                    heartbeat: heartbeat.clone(),
                };
                if let Err(e) = session.run().await {
                    eprintln!("[realtime] session {url} ended: {e}; reconnecting");
                }
                let _ = tx.send(false);
                tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                backoff_ms = next_backoff(backoff_ms);
            }
        });
        rx
    }
} // impl WsSession

#[cfg(feature = "realtime")]
pub(crate) struct WsSession<'a> {
    ws: WsStream,
    sub_rx: watch::Receiver<Vec<String>>,
    dispatch: Arc<dyn Fn(serde_json::Value) + Send + Sync>,
    sent: &'a mut HashSet<String>,
    heartbeat: Option<Heartbeat>,
}

#[cfg(feature = "realtime")]
impl WsSession<'_> {
    async fn run(&mut self) -> Result<()> {
        let mut hb_interval = self
            .heartbeat
            .as_ref()
            .map(|(i, _)| tokio::time::interval(*i));
        loop {
            tokio::select! {
                msg = self.ws.next() => {
                    let msg = msg.ok_or_else(|| Error::new(ErrorKind::NetworkError, "ws stream ended"))?
                        .map_err(|e| Error::new(ErrorKind::NetworkError, format!("ws recv: {e}")))?;
                    match msg {
                        Message::Text(text) => {
                            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
                                (self.dispatch)(value);
                            }
                        }
                        Message::Ping(payload) => {
                            self.ws.send(Message::Pong(payload)).await.ok();
                        }
                        Message::Close(_) => {
                            return Err(Error::new(ErrorKind::NetworkError, "ws closed"));
                        }
                        _ => {}
                    }
                }
                _ = self.sub_rx.changed() => {
                    let frames = self.sub_rx.borrow().clone();
                    for frame in frames {
                        if self.sent.insert(frame.clone()) {
                            self.ws.send(Message::Text(frame.into())).await
                                .map_err(|e| Error::new(ErrorKind::NetworkError, format!("ws send sub: {e}")))?;
                        }
                    }
                }
                _ = heartbeat_tick(&mut hb_interval) => {
                    if let Some((_, frame)) = self.heartbeat.as_ref() {
                        let m = frame();
                        self.ws.send(m).await.map_err(|e| {
                            Error::new(ErrorKind::NetworkError, format!("ws send heartbeat: {e}"))
                        })?;
                    }
                }
            }
        }
    }
}

/// 订阅管理:已建立的订阅(避免重复订阅)。
#[derive(Debug, Default)]
pub struct SubscriptionSet {
    streams: HashMap<String, usize>,
}

impl SubscriptionSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册订阅,返回是否首次(需要真正发送订阅命令)。
    pub fn register(&mut self, key: &str) -> bool {
        let count = self.streams.entry(key.to_string()).or_insert(0);
        if *count == 0 {
            *count = 1;
            true
        } else {
            *count += 1;
            false
        }
    }
}

/// 取或创建指定键的 watch channel,并触发订阅(收口各 realtime 适配器
/// `watch_*` 里的 `if !map.contains_key(key) { insert watch::channel; }` 样板,
/// ADR-0014 续)。
///
/// 键 → channel 的映射由各适配器持有([`ChannelMap`]);订阅帧构造因交易所而异,
/// 交由 `subscribe` 闭包(通常直接转发到该适配器的 `subscribe(channel, key)`)。
/// 这样 `watch_*` 从「建 channel + 订阅 + 等首条更新」三段样板,收敛为数据驱动的一行。
#[cfg(feature = "realtime")]
pub async fn get_or_subscribe<K, V, F, Fut>(
    map: &ChannelMap<K, V>,
    key: K,
    default: V,
    subscribe: F,
) -> Result<watch::Receiver<V>>
where
    K: Eq + std::hash::Hash + Clone,
    F: FnOnce(K) -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    let rx = {
        let mut m = map.lock().unwrap();
        if !m.contains_key(&key) {
            let (tx, _) = watch::channel(default);
            m.insert(key.clone(), tx.clone());
        }
        m.get(&key).cloned().unwrap().subscribe()
    };
    subscribe(key).await?;
    Ok(rx)
}

#[cfg(all(test, feature = "realtime"))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn wait_first_returns_value() {
        let (tx, rx) = watch::channel(0i32);
        tx.send(42).ok();
        assert_eq!(wait_first(rx).await.unwrap(), 42);
    }

    #[tokio::test]
    async fn wait_first_errors_on_channel_close() {
        let (_tx, rx) = watch::channel(0i32);
        drop(_tx);
        let err = wait_first(rx).await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NetworkError);
    }

    #[test]
    fn conn_ensures_spawn_once() {
        let conn = Conn::new();
        let (tx, _) = watch::channel(false);
        let mut calls = 0;
        let rx = conn.ensure(|| {
            calls += 1;
            tx.subscribe()
        });
        let rx2 = conn.ensure(|| {
            calls += 1;
            watch::channel(false).1
        });
        assert_eq!(calls, 1);
        assert_eq!(*rx.borrow(), *rx2.borrow());
    }

    #[test]
    fn backoff_grows_and_caps() {
        let mut ms = 1_000;
        for _ in 0..6 {
            let next = next_backoff(ms);
            assert!(next >= ms, "backoff must not shrink");
            assert!(next <= 30_000, "backoff must cap at 30s");
            ms = next;
        }
        assert!(ms <= 30_000);
    }

    #[test]
    fn channel_map_insert_get() {
        let map: ChannelMap<String, i32> = ChannelMap::default();
        let (tx, _) = watch::channel(7);
        map.lock().unwrap().insert("BTC/USDT".into(), tx.clone());
        assert_eq!(*map.lock().unwrap().get("BTC/USDT").unwrap().borrow(), 7);
    }

    #[tokio::test]
    async fn heartbeat_tick_fires_with_interval() {
        let mut interval = Some(tokio::time::interval(Duration::from_millis(1)));
        tokio::time::timeout(Duration::from_millis(100), heartbeat_tick(&mut interval))
            .await
            .expect("heartbeat tick should fire");
    }

    #[tokio::test]
    async fn get_or_subscribe_returns_receiver_and_dedups_key() {
        let map: ChannelMap<String, i32> = ChannelMap::default();
        let rx = get_or_subscribe(&map, "BTC/USDT".to_string(), 0, |_k| async { Ok(()) })
            .await
            .unwrap();
        assert_eq!(*rx.borrow(), 0);
        // 同键复用同一 channel:get_or_subscribe 不再插入新 sender。
        let rx2 = get_or_subscribe(&map, "BTC/USDT".to_string(), 0, |_k| async { Ok(()) })
            .await
            .unwrap();
        assert_eq!(*rx.borrow(), *rx2.borrow());
        assert_eq!(map.lock().unwrap().len(), 1);
    }
}
