//! 传输层 trait 与实现:reqwest 封装(超时/代理/重试)对齐 ccxt `fetch2` 语义,
//! 外加 [`MockTransport`] 供离线请求/签名测试(候选 6:ADR-0013 第五缝提案)。
//!
//! 重试循环仅对 [`ErrorKind::is_retryable`] 的错误生效,退避为
//! `base_backoff × attempt`;非 2xx 响应按状态码归类为业务错误,
//! 具体错误码映射由适配器的 `handle_errors` 阶段负责(后续里程碑)。
//!
//! [`Transport`] 抽为 trait 后,`Client` 持 `Box<dyn Transport + Send + Sync>`,
//! 使请求/签名路径可离线注入内存 [`MockTransport`],不再依赖网络。

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reqwest::header::{CONTENT_TYPE, HeaderMap};
use serde_json::Value;

use crate::error::{Error, ErrorContext, ErrorKind, Result};

/// HTTP 传输抽象(候选 6 注入缝)。
///
/// 真实实现为 [`ReqwestTransport`];离线测试用 [`MockTransport`] 在内存中返回
/// 预制响应并记录请求,使 `parse_*` 之上的请求/签名路径可离线断言。
///
/// 方法返回 `Box<dyn Future + Send>`,使 trait 可用于 `dyn Transport`
/// (稳定版 Rust 的原生 `async fn` 尚不 dyn 兼容,故手动装箱未来)。
pub trait Transport: Send + Sync {
    /// 发起请求并解析 JSON 响应,失败时按策略重试。
    fn fetch_json<'a>(
        &'a self,
        method: &'a str,
        url: &'a str,
        headers: &'a HeaderMap,
        body: Option<Value>,
    ) -> Pin<Box<dyn Future<Output = Result<Value>> + Send + 'a>>;
}

/// 真实 reqwest 传输封装。
pub struct ReqwestTransport {
    client: reqwest::Client,
    max_retries: u32,
    base_backoff: Duration,
}

impl ReqwestTransport {
    /// 构造传输层。
    pub fn new(timeout_ms: u64, max_retries: u32, proxy: Option<&str>) -> Result<Self> {
        let mut builder = reqwest::Client::builder()
            .timeout(Duration::from_millis(timeout_ms))
            .user_agent(concat!("adaq-trading-crypto/", env!("CARGO_PKG_VERSION")));
        if let Some(proxy_url) = proxy {
            builder = builder.proxy(reqwest::Proxy::all(proxy_url).map_err(Error::from)?);
        }
        Ok(Self {
            client: builder.build().map_err(Error::from)?,
            max_retries,
            base_backoff: Duration::from_millis(1_000),
        })
    }

    async fn fetch_once(
        &self,
        method: &str,
        url: &str,
        headers: &HeaderMap,
        body: Option<Value>,
    ) -> Result<Value> {
        let reqwest_method = match method {
            "GET" => reqwest::Method::GET,
            "POST" => reqwest::Method::POST,
            "PUT" => reqwest::Method::PUT,
            "DELETE" => reqwest::Method::DELETE,
            "PATCH" => reqwest::Method::PATCH,
            _ => {
                return Err(Error::new(
                    ErrorKind::BadRequest,
                    format!("unsupported HTTP method {method}"),
                ));
            }
        };

        let mut request = self
            .client
            .request(reqwest_method, url)
            .headers(headers.clone());
        if let Some(json) = body {
            request = request.header(CONTENT_TYPE, "application/json").json(&json);
        }

        let response = request.send().await.map_err(Error::from)?;
        let status = response.status();
        let status_u16 = status.as_u16();
        let body_text = response.text().await.map_err(Error::from)?;

        if !status.is_success() {
            let kind = match status_u16 {
                401 => ErrorKind::Authentication,
                403 => ErrorKind::PermissionDenied,
                429 => ErrorKind::RateLimitExceeded,
                400..=499 => ErrorKind::BadRequest,
                _ => ErrorKind::ExchangeNotAvailable,
            };
            let context = ErrorContext::new()
                .url(url)
                .http_status(status_u16)
                .raw_body(truncate(&body_text, 512));
            return Err(Error::with_context(
                kind,
                format!("HTTP {status_u16}: {}", truncate(&body_text, 256)),
                context,
            ));
        }

        if body_text.trim().is_empty() {
            return Err(Error::new(ErrorKind::NullResponse, "empty response body")
                .with_context_ctx(ErrorContext::new().url(url).http_status(status_u16)));
        }

        serde_json::from_str(&body_text).map_err(|parse_err| {
            Error::new(
                ErrorKind::BadResponse,
                format!("invalid JSON response: {parse_err}"),
            )
            .with_context_ctx(
                ErrorContext::new()
                    .url(url)
                    .http_status(status_u16)
                    .raw_body(truncate(&body_text, 512)),
            )
            .with_source(parse_err)
        })
    }
}

impl Transport for ReqwestTransport {
    fn fetch_json<'a>(
        &'a self,
        method: &'a str,
        url: &'a str,
        headers: &'a HeaderMap,
        body: Option<Value>,
    ) -> Pin<Box<dyn Future<Output = Result<Value>> + Send + 'a>> {
        Box::pin(async move {
            let mut attempt: u32 = 0;
            loop {
                match self.fetch_once(method, url, headers, body.clone()).await {
                    Ok(value) => return Ok(value),
                    Err(mut err) if err.is_retryable() && attempt < self.max_retries => {
                        attempt += 1;
                        err.context.retries = attempt;
                        tokio::time::sleep(self.base_backoff * attempt).await;
                    }
                    Err(err) => return Err(err),
                }
            }
        })
    }
}

/// 内存传输桩:返回预制响应并记录最近一次请求(候选 6 离线测试用)。
///
/// 返回 `(MockTransport, Arc<Mutex<Option<RecordedRequest>>>)`,测试保留 Arc
/// 句柄即可在请求发起后读取被捕获的 method/url/body,无需 downcast `dyn Transport`。
pub struct MockTransport {
    response: Value,
    recorded: Arc<Mutex<Option<RecordedRequest>>>,
}

/// `MockTransport` 捕获的最近一次请求。
#[derive(Debug, Clone)]
pub struct RecordedRequest {
    pub method: String,
    pub url: String,
    pub body: Option<Value>,
}

impl MockTransport {
    /// 构造桩,返回桩本身与共享的记录句柄。
    pub fn new(response: Value) -> (Self, Arc<Mutex<Option<RecordedRequest>>>) {
        let recorded = Arc::new(Mutex::new(None));
        (
            Self {
                response,
                recorded: recorded.clone(),
            },
            recorded,
        )
    }
}

impl Transport for MockTransport {
    fn fetch_json<'a>(
        &'a self,
        method: &'a str,
        url: &'a str,
        _headers: &'a HeaderMap,
        body: Option<Value>,
    ) -> Pin<Box<dyn Future<Output = Result<Value>> + Send + 'a>> {
        Box::pin(async move {
            *self.recorded.lock().unwrap() = Some(RecordedRequest {
                method: method.to_string(),
                url: url.to_string(),
                body: body.clone(),
            });
            Ok(self.response.clone())
        })
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    // 回退到合法的字符边界,避免 UTF-8 切半
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &s[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_limits_length() {
        assert_eq!(truncate("abc", 5), "abc");
        let t = truncate(&"x".repeat(100), 50);
        assert!(t.len() <= 53);
        assert!(t.ends_with("..."));
    }
}
