//! 统一错误体系(ADR-0007)。
//!
//! 设计:单一 `Error` 类型,内部 `kind` 为镜像 ccxt 异常树
//! (`ts/src/base/errors.ts`)的封闭枚举,作为差分测试与适配器错误映射的
//! 比对面;`context` 携带诊断信息;`source` 保留底层错误链。
//! `is_retryable()` 由 kind 推导,与传输层重试循环同源,不会漂移。

use std::fmt;

/// 错误分类:镜像 ccxt 异常树(扁平化)。
///
/// 类别即"行为分支"的比对面:调用方按 `kind` 决定恢复策略,差分测试
/// 按 `kind` 比对错误映射是否正确。类别之下不再细分变体字段——
/// 所有诊断细节统一放 [`ErrorContext`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorKind {
    // ---- ExchangeError 之下(可恢复的交易类错误)----
    /// 通用交易所错误(ccxt `ExchangeError`)。
    Exchange,
    /// 认证失败(API key/secret 无效等)。
    Authentication,
    /// 权限不足。
    PermissionDenied,
    /// 账户未启用对应功能。
    AccountNotEnabled,
    /// 账户被冻结。
    AccountSuspended,
    /// 缺少必需参数。
    ArgumentsRequired,
    /// 请求非法。
    BadRequest,
    /// 交易对符号不合法/不存在。
    BadSymbol,
    /// 操作被交易所拒绝。
    OperationRejected,
    /// 操作无变化(如重复设置相同杠杆)。
    NoChange,
    /// 保证金模式已是目标模式。
    MarginModeAlreadySet,
    /// 市集已收盘。
    MarketClosed,
    /// 需要人工介入。
    ManualInteractionNeeded,
    /// 地域受限。
    RestrictedLocation,
    /// 资金不足。
    InsufficientFunds,
    /// 地址无效。
    InvalidAddress,
    /// 地址待确认。
    AddressPending,
    /// 订单参数非法。
    InvalidOrder,
    /// 订单不存在。
    OrderNotFound,
    /// 订单未在本地缓存。
    OrderNotCached,
    /// 订单立即可成交(与请求矛盾)。
    OrderImmediatelyFillable,
    /// 订单无法成交。
    OrderNotFillable,
    /// 订单 ID 重复。
    DuplicateOrderId,
    /// 合约不可用。
    ContractUnavailable,
    /// 该交易所/功能不支持(统一接口默认返回值)。
    NotSupported,
    /// 代理设置无效。
    InvalidProxySettings,
    /// 用户主动关闭交易所连接。
    ExchangeClosedByUser,

    // ---- OperationFailed 之下 ----
    /// 操作失败(通用)。
    OperationFailed,
    /// 网络错误(可重试族)。
    NetworkError,
    /// 触发反 DDoS 保护。
    DDoSProtection,
    /// 触发限速。
    RateLimitExceeded,
    /// 交易所不可用(5xx 等)。
    ExchangeNotAvailable,
    /// 交易所维护中。
    OnMaintenance,
    /// nonce 无效(时钟偏差)。
    InvalidNonce,
    /// 校验和不匹配。
    ChecksumError,
    /// 请求超时。
    RequestTimeout,
    /// 响应异常(格式/契约不符)。
    BadResponse,
    /// 空响应。
    NullResponse,
    /// 取消订单请求尚在处理。
    CancelPending,
    /// 订阅/退订失败(WS)。
    UnsubscribeError,

    // ---- 兜底 ----
    /// ccxt 根异常 `BaseError`。
    Base,
    /// 无法归类(映射失败时的兜底)。
    Unknown,
}

impl ErrorKind {
    /// 该类别是否可安全重试(与传输层重试循环共用此判定,单一来源)。
    pub fn is_retryable(self) -> bool {
        matches!(
            self,
            ErrorKind::OperationFailed
                | ErrorKind::NetworkError
                | ErrorKind::DDoSProtection
                | ErrorKind::RateLimitExceeded
                | ErrorKind::ExchangeNotAvailable
                | ErrorKind::OnMaintenance
                | ErrorKind::InvalidNonce
                | ErrorKind::RequestTimeout
        )
    }

    /// 该类别是否属于认证/权限族。
    pub fn is_authentication(self) -> bool {
        matches!(
            self,
            ErrorKind::Authentication | ErrorKind::PermissionDenied | ErrorKind::AccountSuspended
        )
    }

    /// 该类别是否属于网络/传输族。
    pub fn is_network(self) -> bool {
        self.is_retryable()
    }

    /// 人类可读的类别名(ccxt 异常类名)。
    pub fn as_str(self) -> &'static str {
        use ErrorKind::*;
        match self {
            Exchange => "ExchangeError",
            Authentication => "AuthenticationError",
            PermissionDenied => "PermissionDenied",
            AccountNotEnabled => "AccountNotEnabled",
            AccountSuspended => "AccountSuspended",
            ArgumentsRequired => "ArgumentsRequired",
            BadRequest => "BadRequest",
            BadSymbol => "BadSymbol",
            OperationRejected => "OperationRejected",
            NoChange => "NoChange",
            MarginModeAlreadySet => "MarginModeAlreadySet",
            MarketClosed => "MarketClosed",
            ManualInteractionNeeded => "ManualInteractionNeeded",
            RestrictedLocation => "RestrictedLocation",
            InsufficientFunds => "InsufficientFunds",
            InvalidAddress => "InvalidAddress",
            AddressPending => "AddressPending",
            InvalidOrder => "InvalidOrder",
            OrderNotFound => "OrderNotFound",
            OrderNotCached => "OrderNotCached",
            OrderImmediatelyFillable => "OrderImmediatelyFillable",
            OrderNotFillable => "OrderNotFillable",
            DuplicateOrderId => "DuplicateOrderId",
            ContractUnavailable => "ContractUnavailable",
            NotSupported => "NotSupported",
            InvalidProxySettings => "InvalidProxySettings",
            ExchangeClosedByUser => "ExchangeClosedByUser",
            OperationFailed => "OperationFailed",
            NetworkError => "NetworkError",
            DDoSProtection => "DDoSProtection",
            RateLimitExceeded => "RateLimitExceeded",
            ExchangeNotAvailable => "ExchangeNotAvailable",
            OnMaintenance => "OnMaintenance",
            InvalidNonce => "InvalidNonce",
            ChecksumError => "ChecksumError",
            RequestTimeout => "RequestTimeout",
            BadResponse => "BadResponse",
            NullResponse => "NullResponse",
            CancelPending => "CancelPending",
            UnsubscribeError => "UnsubscribeError",
            Base => "BaseError",
            Unknown => "UnknownError",
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 错误诊断上下文:与 `kind` 分离,所有类别共用。
#[derive(Debug, Clone, Default)]
pub struct ErrorContext {
    /// 交易所 id,如 `binance`。
    pub exchange: Option<String>,
    /// 出错方法名,如 `fetch_order`。
    pub method: Option<&'static str>,
    /// 请求 URL(可选)。
    pub url: Option<String>,
    /// HTTP 状态码。
    pub http_status: Option<u16>,
    /// 交易所返回的业务错误码。
    pub http_error_code: Option<String>,
    /// 原始响应体(截断)。
    pub raw_body: Option<String>,
    /// 已重试次数。
    pub retries: u32,
}

impl ErrorContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn exchange(mut self, exchange: impl Into<String>) -> Self {
        self.exchange = Some(exchange.into());
        self
    }

    pub fn method(mut self, method: &'static str) -> Self {
        self.method = Some(method);
        self
    }

    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }

    pub fn http_status(mut self, status: u16) -> Self {
        self.http_status = Some(status);
        self
    }

    pub fn http_error_code(mut self, code: impl Into<String>) -> Self {
        self.http_error_code = Some(code.into());
        self
    }

    pub fn raw_body(mut self, body: impl Into<String>) -> Self {
        self.raw_body = Some(body.into());
        self
    }

    pub fn retries(mut self, n: u32) -> Self {
        self.retries = n;
        self
    }
}

/// 统一错误类型(ADR-0007):全库唯一的 `Result<T, Error>`。
#[derive(Debug)]
pub struct Error {
    pub kind: ErrorKind,
    pub message: String,
    /// Box 化以控制 `Error` 尺寸(避免 clippy::result_large_err 触发)。
    pub context: Box<ErrorContext>,
    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl Error {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            context: Box::new(ErrorContext::new()),
            source: None,
        }
    }

    pub fn with_context(
        kind: ErrorKind,
        message: impl Into<String>,
        context: ErrorContext,
    ) -> Self {
        Self {
            kind,
            message: message.into(),
            context: Box::new(context),
            source: None,
        }
    }

    /// 统一接口默认返回值:该方法未实现/不支持。
    pub fn not_supported(method: &'static str) -> Self {
        Self::new(
            ErrorKind::NotSupported,
            format!("method {method} is not supported by this exchange"),
        )
        .with_method(method)
    }

    /// 附加一个已完成的错误分类(适配器内 throw 类错误的便捷入口)。
    pub fn from_exchange(kind: ErrorKind, context: ErrorContext) -> Self {
        Self::with_context(kind, kind.as_str(), context)
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    pub fn is_retryable(&self) -> bool {
        self.kind.is_retryable()
    }

    pub fn with_method(mut self, method: &'static str) -> Self {
        self.context.method = Some(method);
        self
    }

    pub fn with_context_ctx(mut self, context: ErrorContext) -> Self {
        self.context = Box::new(context);
        self
    }

    pub fn with_source(mut self, source: impl std::error::Error + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(source));
        self
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind.as_str(), self.message)?;
        if let Some(exchange) = &self.context.exchange {
            write!(f, " [exchange={exchange}]")?;
        }
        if let Some(method) = self.context.method {
            write!(f, " [method={method}]")?;
        }
        if let Some(status) = self.context.http_status {
            write!(f, " [http={status}]")?;
        }
        if let Some(code) = &self.context.http_error_code {
            write!(f, " [code={code}]")?;
        }
        Ok(())
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_deref()
            .map(|e| e as &(dyn std::error::Error + 'static))
    }
}

impl From<reqwest::Error> for Error {
    fn from(err: reqwest::Error) -> Self {
        let kind = if err.is_timeout() {
            ErrorKind::RequestTimeout
        } else if err.is_connect() || err.is_body() || err.is_decode() {
            // 传输层失败视为网络族
            ErrorKind::NetworkError
        } else if let Some(status) = err.status() {
            match status.as_u16() {
                401 => ErrorKind::Authentication,
                403 => ErrorKind::PermissionDenied,
                429 => ErrorKind::RateLimitExceeded,
                400..=499 => ErrorKind::BadRequest,
                500..=599 => ErrorKind::ExchangeNotAvailable,
                _ => ErrorKind::NetworkError,
            }
        } else {
            ErrorKind::NetworkError
        };
        let mut error = Error::new(kind, err.to_string());
        if let Some(status) = err.status() {
            error.context.http_status = Some(status.as_u16());
        }
        error.source = Some(Box::new(err));
        error
    }
}

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Error::new(
            ErrorKind::BadResponse,
            format!("invalid JSON response: {err}"),
        )
        .with_source(err)
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Error::new(ErrorKind::NetworkError, err.to_string()).with_source(err)
    }
}

/// 便捷别名:全库统一使用的 Result。
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retryable_classification() {
        assert!(ErrorKind::RateLimitExceeded.is_retryable());
        assert!(ErrorKind::ExchangeNotAvailable.is_retryable());
        assert!(ErrorKind::RequestTimeout.is_retryable());
        assert!(!ErrorKind::InsufficientFunds.is_retryable());
        assert!(!ErrorKind::OrderNotFound.is_retryable());
        assert!(!ErrorKind::BadSymbol.is_retryable());
    }

    #[test]
    fn not_supported_carries_method() {
        let err = Error::not_supported("fetch_ticker");
        assert_eq!(err.kind, ErrorKind::NotSupported);
        assert_eq!(err.context.method, Some("fetch_ticker"));
        assert!(!err.is_retryable());
    }

    #[test]
    fn display_includes_kind_and_context() {
        let err = Error::new(ErrorKind::InsufficientFunds, "balance too low")
            .with_method("create_order")
            .with_context_ctx(ErrorContext::new().exchange("binance").http_status(400));
        let text = err.to_string();
        assert!(text.contains("InsufficientFunds"));
        assert!(text.contains("balance too low"));
        assert!(text.contains("binance"));
        assert!(text.contains("http=400"));
    }
}
