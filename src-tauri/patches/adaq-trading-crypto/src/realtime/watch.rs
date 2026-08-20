//! watch 会话收口(ADR-0014 续,2026-08-18 架构评审候选 1)。
//!
//! 公开 watch 路径的「连接单例 + 订阅去重 + 建/取频道 + 等首条」收进 [`WatchContext`]
//! 一个深模块。适配器只填三个接缝:
//! - **spawn 配方**:[`WatchContext::ensure`] 的闭包(URL/头/消息分发);
//! - **订阅帧构造**:[`WatchContext::subscribe`] / [`WatchContext::watch`] 的帧闭包;
//! - **key 解析**:调用方先算好频道键(如 kalshi 的 market_ticker 需 REST 解析)。
//!
//! 设计要点:
//! - context 持有 `Conn`(单飞连接)、`SubscriptionSet`(去重)、`sub_tx`(帧发送端);
//! - [`WatchContext::ensure`] 由 context 先建 `(sub_tx, sub_rx)`,把发送端交给配方,
//!   配方只负责 `WsSession::spawn` 并返回就绪信号——发送端生命周期收进 context,
//!   订阅路径不再有「ws not started」时序缝隙;
//! - 可失败步骤(认证头、REST key 解析)留在调用方,配方保持无失败;
//! - 订单簿的初始化时序(如 binance REST 快照)留在适配器,组合本模块助手。

#[cfg(feature = "realtime")]
use std::sync::Mutex;

#[cfg(feature = "realtime")]
use tokio::sync::watch;

#[cfg(feature = "realtime")]
use crate::error::{Error, ErrorKind, Result};
#[cfg(feature = "realtime")]
use crate::realtime::ws::{ChannelMap, Conn, SubscriptionSet, get_or_subscribe, wait_first};

/// watch 会话上下文:连接单例 + 订阅去重 + 帧发送端。
#[cfg(feature = "realtime")]
#[derive(Debug, Default)]
pub struct WatchContext {
    conn: Conn,
    subs: Mutex<SubscriptionSet>,
    sub_tx: Mutex<Option<watch::Sender<Vec<String>>>>,
}

#[cfg(feature = "realtime")]
impl WatchContext {
    /// 构造上下文。
    pub fn new() -> Self {
        Self::default()
    }

    /// 连接单例:context 先建帧发送端交给配方 spawn,配方只跑一次。
    ///
    /// 配方为无失败闭包(可失败步骤如认证头/凭据解析由调用方在之前完成),
    /// 返回就绪信号;重复调用返回缓存的同一信号,不重建连接。
    pub fn ensure(
        &self,
        recipe: impl FnOnce(watch::Sender<Vec<String>>) -> watch::Receiver<bool>,
    ) -> watch::Receiver<bool> {
        self.conn.ensure(|| {
            let (sub_tx, _) = watch::channel(Vec::new());
            let ready = recipe(sub_tx.clone());
            *self.sub_tx.lock().unwrap() = Some(sub_tx);
            ready
        })
    }

    /// 是否已建立连接(不触发建立)。
    pub fn is_connected(&self) -> bool {
        self.conn.is_connected()
    }

    /// 订阅去重 + 帧发送:key 首次注册时构造帧并入队,重复订阅为 no-op。
    pub fn subscribe(&self, key: &str, frame: impl FnOnce() -> String) -> Result<()> {
        let first = self.subs.lock().unwrap().register(key);
        if !first {
            return Ok(());
        }
        let tx = self
            .sub_tx
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| Error::new(ErrorKind::NetworkError, "ws not started"))?;
        tx.send_modify(|list| list.push(frame()));
        Ok(())
    }

    /// 建/取频道 + 订阅 + 等首条更新,一步到位(公开 watch_* 的统一入口)。
    ///
    /// `subscribe` 闭包由适配器提供,通常先 `ensure` 再构造订阅帧。
    pub async fn watch<K, V, F, Fut>(
        &self,
        map: &ChannelMap<K, V>,
        key: K,
        default: V,
        subscribe: F,
    ) -> Result<V>
    where
        K: Eq + std::hash::Hash + Clone,
        V: Clone,
        F: FnOnce(K) -> Fut,
        Fut: std::future::Future<Output = Result<()>>,
    {
        let rx = get_or_subscribe(map, key, default, subscribe).await?;
        wait_first(rx).await
    }

    /// 帧发送端(私密流与公共流共用同一连接时使用,如 kalshi)。
    pub fn sender(&self) -> Option<watch::Sender<Vec<String>>> {
        self.sub_tx.lock().unwrap().clone()
    }
}

#[cfg(all(test, feature = "realtime"))]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn ensure_runs_recipe_once() {
        let ctx = WatchContext::new();
        let calls = Arc::new(Mutex::new(0));
        let c1 = calls.clone();
        let rx = ctx.ensure(move |_tx| {
            *c1.lock().unwrap() += 1;
            watch::channel(true).1
        });
        let c2 = calls.clone();
        let rx2 = ctx.ensure(move |_tx| {
            *c2.lock().unwrap() += 1;
            watch::channel(true).1
        });
        assert_eq!(*calls.lock().unwrap(), 1, "配方只应执行一次");
        assert!(*rx.borrow());
        assert!(*rx2.borrow());
    }

    #[test]
    fn is_connected_flips_after_ensure() {
        let ctx = WatchContext::new();
        assert!(!ctx.is_connected());
        ctx.ensure(|_tx| watch::channel(true).1);
        assert!(ctx.is_connected());
    }

    #[test]
    fn subscribe_before_ensure_errors() {
        let ctx = WatchContext::new();
        let err = ctx.subscribe("k", || "F".to_string()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NetworkError);
    }

    #[test]
    fn subscribe_dedups_and_pushes_once() {
        let ctx = WatchContext::new();
        ctx.ensure(|_tx| watch::channel(true).1);
        let rx = ctx.sender().unwrap().subscribe();
        let calls = Arc::new(Mutex::new(0));
        let c1 = calls.clone();
        ctx.subscribe("k1", move || {
            *c1.lock().unwrap() += 1;
            "F1".to_string()
        })
        .unwrap();
        let c2 = calls.clone();
        ctx.subscribe("k1", move || {
            *c2.lock().unwrap() += 1;
            "F1".to_string()
        })
        .unwrap();
        ctx.subscribe("k2", || "F2".to_string()).unwrap();
        assert_eq!(*calls.lock().unwrap(), 1, "重复 key 不应再构造帧");
        assert_eq!(
            rx.borrow().clone(),
            vec!["F1".to_string(), "F2".to_string()]
        );
    }

    #[tokio::test]
    async fn watch_returns_first_update() {
        let ctx = WatchContext::new();
        let map: ChannelMap<String, i32> = ChannelMap::default();
        let (tx, _) = watch::channel(0);
        map.lock()
            .unwrap()
            .insert("BTC/USDT".to_string(), tx.clone());
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            tx.send(42).ok();
        });
        let v = ctx
            .watch(&map, "BTC/USDT".to_string(), 0, |_| async { Ok(()) })
            .await
            .unwrap();
        assert_eq!(v, 42);
    }

    #[tokio::test]
    async fn watch_propagates_subscribe_error() {
        // 订阅闭包失败(如认证/连接准备失败)时,错误原样上抛,不等首条。
        // (频道关闭的 wait_first 错误路径已由 ws.rs wait_first_errors_on_channel_close 覆盖。)
        let ctx = WatchContext::new();
        let map: ChannelMap<String, i32> = ChannelMap::default();
        let err = ctx
            .watch(&map, "K".to_string(), 0, |_| async {
                Err(Error::new(ErrorKind::Authentication, "sub failed"))
            })
            .await
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Authentication);
    }

    #[test]
    fn sender_available_after_ensure() {
        let ctx = WatchContext::new();
        assert!(ctx.sender().is_none());
        ctx.ensure(|_tx| watch::channel(true).1);
        assert!(ctx.sender().is_some());
    }
}
