//! 适配器共用的 HTTP 客户端:Transport(超时/代理/重试)+ 限速。
//!
//! 每个交易所适配器持有一个 [`Client`],在统一方法实现中调用
//! [`Client::request`] 完成带限速、带重试的 HTTP 往返。
//!
//! `transport` 持 `Box<dyn Transport + Send + Sync>`,使请求/签名路径可离线
//! 注入 [`MockTransport`](候选 6)。生产路径默认用真实 [`ReqwestTransport`]。

use reqwest::header::HeaderMap;
use serde_json::Value;

use crate::error::Result;
use crate::throttle::{ThrottleMode, Throttler};
use crate::transport::{ReqwestTransport, Transport};

/// 适配器 HTTP 客户端。
pub struct Client {
    transport: Box<dyn Transport + Send + Sync>,
    throttler: Throttler,
    enable_rate_limit: bool,
}

/// 由 `rate_limit_ms` 构造令牌桶(与 `Client::new` / 注入构造共用)。
fn build_throttler(rate_limit_ms: u64) -> Throttler {
    // 令牌桶:容量=1 秒突发量,补速=每秒可发请求数
    let refill_rate = if rate_limit_ms > 0 {
        1000.0 / rate_limit_ms as f64
    } else {
        // 未配置限速:构造一个几乎不限的桶,仅当 enable 时才被调用
        1e6
    };
    Throttler::new(ThrottleMode::LeakyBucket {
        capacity: refill_rate,
        refill_rate,
    })
}

impl Client {
    /// 构造客户端(默认真实 reqwest 传输)。
    ///
    /// `rate_limit_ms` 为单次请求最小间隔(毫秒),如 binance 为 50;
    /// 仅在 `enable_rate_limit` 时启用限速。
    pub fn new(
        timeout_ms: u64,
        max_retries: u32,
        proxy: Option<&str>,
        rate_limit_ms: u64,
        enable_rate_limit: bool,
    ) -> Result<Self> {
        let transport = ReqwestTransport::new(timeout_ms, max_retries, proxy)?;
        Ok(Self {
            transport: Box::new(transport),
            throttler: build_throttler(rate_limit_ms),
            enable_rate_limit,
        })
    }

    /// 运行时替换传输(离线测试桩注入,候选 6)。
    #[cfg(test)]
    pub(crate) fn set_transport(&mut self, transport: Box<dyn Transport + Send + Sync>) {
        self.transport = transport;
    }

    /// 请求 JSON 响应(带限速与重试)。
    pub async fn request(
        &self,
        method: &str,
        url: &str,
        headers: &HeaderMap,
        body: Option<Value>,
    ) -> Result<Value> {
        if self.enable_rate_limit {
            self.throttler.throttle(1).await;
        }
        self.transport.fetch_json(method, url, headers, body).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::MockTransport;
    use serde_json::json;

    // 证明候选 6 注入缝:客户端可离线换成内存桩,`request` 返回预制响应
    // 并记录发出的 URL,不必触网。
    #[tokio::test]
    async fn request_uses_injected_mock_transport() {
        let mut client = Client::new(1000, 0, None, 0, false).unwrap();
        let (mock, recorded) = MockTransport::new(json!({ "ok": true }));
        client.set_transport(Box::new(mock));
        let headers = HeaderMap::new();
        let resp = client
            .request(
                "GET",
                "https://api.mock.com/v1/klines?symbol=BTCUSDT",
                &headers,
                None,
            )
            .await
            .unwrap();
        assert_eq!(resp, json!({ "ok": true }));
        let url = recorded.lock().unwrap().as_ref().unwrap().url.clone();
        assert!(url.contains("symbol=BTCUSDT"), "应记录发出的 URL,url={url}");
    }
}
