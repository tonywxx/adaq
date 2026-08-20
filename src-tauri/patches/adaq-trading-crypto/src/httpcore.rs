//! HttpCore 深模块(ADR-0013):交易所无关的 HTTP 骨架、市集缓存与 safe 提取。
//!
//! 每个交易所适配器持有 [`HttpCore`],只填四个接缝(describe 路径/参数、
//! sign 签名、handle_errors 错误码、字段映射 parse 覆写),不再各自实现
//! 请求骨架、市集缓存与数值解析(原 13 份 `query_string`/`public_get`/
//! `load_markets`/`value_decimal`/`parse_level` 副本的集中处)。

use std::future::Future;
use std::sync::Mutex;

use reqwest::header::HeaderMap;
use serde_json::Value;

use crate::client::Client;
use crate::error::{Error, ErrorContext, ErrorKind, Result};
use crate::exchange::{Config, Params};
#[cfg(test)]
use crate::transport::Transport;
use crate::types::{Level, Markets, OHLCV};

/// 交易所无关核心:HTTP 客户端 + 市集缓存。
pub struct HttpCore {
    client: Client,
    base_url: String,
    markets: Mutex<Option<Markets>>,
    /// 交易所 id(如 `binance`),供 `handle_errors` 接缝在错误上下文标注来源。
    exchange: &'static str,
}

impl HttpCore {
    /// 构造核心。
    ///
    /// `base_url` 为交易所 REST 根地址;`default_rate_limit_ms` 为该交易所
    /// 默认限速,`config.rate_limit_ms` 显式设置时优先;`exchange` 为交易所
    /// id,供 `handle_errors` 接缝标注错误来源。
    pub fn new(
        config: &Config,
        base_url: &str,
        default_rate_limit_ms: u64,
        exchange: &'static str,
    ) -> Result<Self> {
        let rate_limit_ms = if config.rate_limit_ms > 0 {
            config.rate_limit_ms
        } else {
            default_rate_limit_ms
        };
        let client = Client::new(
            config.timeout_ms,
            config.max_retries,
            config.proxy.as_deref(),
            rate_limit_ms,
            config.enable_rate_limit,
        )?;
        Ok(Self {
            client,
            base_url: base_url.to_string(),
            markets: Mutex::new(None),
            exchange,
        })
    }

    /// 发起请求。`path` 为端点路径,GET 可含 `?query`(与签名 requestPath 一致)。
    pub async fn request(
        &self,
        method: &str,
        path: &str,
        headers: &HeaderMap,
        body: Option<Value>,
    ) -> Result<Value> {
        self.request_url(method, &format!("{}{}", self.base_url, path), headers, body)
            .await
    }

    /// 用完整 URL 发起请求(多 base 交易所,如 polymarket 的
    /// data-api/gamma-api/clob,不走 `base_url` 前缀)。
    pub async fn request_url(
        &self,
        method: &str,
        url: &str,
        headers: &HeaderMap,
        body: Option<Value>,
    ) -> Result<Value> {
        let value = self.client.request(method, url, headers, body).await?;
        // handle_errors 接缝(ADR-0013 四缝之一):HTTP 200 + 合法 JSON 仍可能
        // 是交易所业务错误体(如 OKX `{"code":"51000"}`、Kraken
        // `{"error":[...]}`),在此统一识别为分类错误,而非交给解析层静默失败。
        if let Some(err) = extract_exchange_error(&value, self.exchange) {
            return Err(err);
        }
        Ok(value)
    }

    /// 公共 GET(自动拼 query)。
    pub async fn public_get(&self, path: &str, params: &Params) -> Result<Value> {
        let url = format!("{}{}", path, query_string(params));
        self.request("GET", &url, &HeaderMap::new(), None).await
    }

    /// 市集缓存:未加载时调用 `fetch` 拉取并缓存,已加载直接返回。
    ///
    /// `fetch` 由适配器提供(市集解析是字段映射接缝的一部分),核心只负责
    /// 缓存读写——原各适配器 `markets: Mutex<Option<Markets>>` + 缓存判断
    /// 的副本在此收口。
    pub async fn load_markets<F, Fut>(&self, fetch: F) -> Result<()>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Markets>>,
    {
        if self.markets.lock().unwrap().is_some() {
            return Ok(());
        }
        let markets = fetch().await?;
        *self.markets.lock().unwrap() = Some(markets);
        Ok(())
    }

    /// 当前市集缓存快照(未加载时为空)。
    pub fn markets_snapshot(&self) -> Markets {
        self.markets.lock().unwrap().clone().unwrap_or_default()
    }

    /// 运行时替换传输(离线测试桩注入,候选 6)。仅测试构建可用。
    #[cfg(test)]
    pub(crate) fn set_transport(&mut self, transport: Box<dyn Transport + Send + Sync>) {
        self.client.set_transport(transport);
    }
}

/// 兼容字符串/数字的 Decimal 解析(原各适配器 `value_decimal` 副本)。
pub fn value_decimal(v: &Value) -> Option<rust_decimal::Decimal> {
    match v {
        Value::String(s) => s.parse().ok(),
        Value::Number(n) => n.to_string().parse().ok(),
        _ => None,
    }
}

/// `Option<&Value>` → Decimal(原各适配器 `dec` 副本)。
pub fn dec(v: Option<&Value>) -> Option<rust_decimal::Decimal> {
    v.and_then(value_decimal)
}

/// 客户端 `limit` 截断原语(ADR-0013 `handle_errors` 之外的请求后处理接缝)。
///
/// 各 curated 适配器原本各自内联手写截断(`truncate` 保留头 /
/// `split_off` 保留尾),语义与方向不一。收口到此一处:`tail=false`
/// 保留前 `limit` 项,`tail=true` 保留后 `limit` 项;`limit >= len` 或
/// `None` 时原样返回(与既有行为一致,不补零)。
///
/// 仅 `gemini` / `hyperliquid` / `kraken` 三个 curated 适配器当前调用本原语,
/// 故按这些 feature 门控,避免默认 feature 集(binance/okx/kalshi/polymarket)
/// 下出现未使用告警。新增调用方时把对应 feature 并入门控即可。
#[cfg(any(feature = "gemini", feature = "hyperliquid", feature = "kraken"))]
pub(crate) fn apply_client_limit<T>(items: &mut Vec<T>, limit: Option<usize>, tail: bool) {
    if let Some(l) = limit {
        if l < items.len() {
            if tail {
                *items = items.split_off(items.len() - l);
            } else {
                items.truncate(l);
            }
        }
    }
}

/// `[price, amount, ...]` → Level(原各适配器 `parse_level` 副本)。
pub fn parse_level(raw: &Value) -> Level {
    let arr = raw.as_array();
    Level {
        price: arr.and_then(|a| a.first()).and_then(value_decimal),
        amount: arr.and_then(|a| a.get(1)).and_then(value_decimal),
    }
}

/// 规范 ccxt OHLCV 数组 `[timestamp, open, high, low, close, volume]` → [`OHLCV`]。
///
/// 覆盖大多数交易所的数组形 K 线;仅索引顺序或时间戳缩放不同的适配器可继续
/// 用 `value_decimal` 自行拼装(如 coinbase 秒→毫秒、kraken 量在第 6 位、
/// bybit 7 元素、htx 对象形),那些属真实交易所差异,不强塞本构造器。
pub fn parse_ohlcv_standard(row: &Value) -> OHLCV {
    let a = row.as_array();
    OHLCV {
        timestamp: a.and_then(|x| x.first()).and_then(|v| v.as_i64()),
        open: a.and_then(|x| x.get(1)).and_then(value_decimal),
        high: a.and_then(|x| x.get(2)).and_then(value_decimal),
        low: a.and_then(|x| x.get(3)).and_then(value_decimal),
        close: a.and_then(|x| x.get(4)).and_then(value_decimal),
        volume: a.and_then(|x| x.get(5)).and_then(value_decimal),
    }
}

/// Params → `?k=v&...`(原各适配器 `query_string` 副本)。
pub fn query_string(params: &Params) -> String {
    if params.is_empty() {
        return String::new();
    }
    let pairs: Vec<String> = params
        .iter()
        .map(|(k, v)| {
            let val = match v {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                other => other.to_string(),
            };
            format!("{}={}", pct_encode(k), pct_encode(&val))
        })
        .collect();
    format!("?{}", pairs.join("&"))
}

/// RFC 3986 百分号编码(原各适配器 `pct_encode` 副本)。
pub fn pct_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// 当前 UTC 毫秒精度时间戳(签名头用,原 `iso8601_now` 副本)。
pub fn iso8601_now() -> String {
    chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

/// 毫秒时间戳 → RFC3339(原 `iso8601` 副本)。
pub fn iso8601(ms: i64) -> Option<String> {
    chrono::DateTime::from_timestamp_millis(ms).map(|d| d.to_rfc3339())
}

/// 毫秒时间戳 → RFC3339(毫秒精度,`...Z` 后缀;对齐 ccxt 输出)。
///
/// 适配器原本各自定义 `fn iso8601(ms)` 使用 `to_rfc3339_opts(Millis, true)`,
/// 与 [`iso8601`] 的 `to_rfc3339()`(AutoSi,`+00:00`) 精度不同;此处提供单一
/// 毫秒精度真源,统一那些副本而不改变任何适配器的输出格式。
pub fn iso8601_ms(ms: i64) -> Option<String> {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

/// 当前 UTC 毫秒时间戳(签名/认证用;统一为 `i64`,与 [`iso8601`] 对齐)。
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 兼容字符串/整数的 `i64` 提取(原 `to_i64` 副本)。
pub fn to_i64(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

/// 按候选键顺序提取首个可解析的 Decimal(safe 提取;原 `pick_decimal` 副本)。
pub fn pick_decimal(v: &Value, keys: &[&str]) -> Option<rust_decimal::Decimal> {
    for k in keys {
        if let Some(d) = v.get(k).and_then(value_decimal) {
            return Some(d);
        }
    }
    None
}

/// 按候选键顺序提取首个字符串(原 `pick_str` 副本)。
pub fn pick_str<'a>(v: &'a Value, keys: &[&str]) -> Option<&'a str> {
    for k in keys {
        if let Some(s) = v.get(k).and_then(|x| x.as_str()) {
            return Some(s);
        }
    }
    None
}

/// 按候选键顺序提取首个 `i64`(原 `pick_i64` 副本)。
pub fn pick_i64(v: &Value, keys: &[&str]) -> Option<i64> {
    for k in keys {
        if let Some(n) = v.get(k).and_then(to_i64) {
            return Some(n);
        }
    }
    None
}

/// 从对象按 `key` 取出数组字段(收口原各适配器 `.get(key).and_then(Value::as_array)` 副本)。
///
/// 交易所响应把数组放在不同信封字段下:OKX/Bitget/Kucoin 用 `data`,
/// Bybit 用 `result`/`list`,Kraken 用 `result`。本函数把「按字段取数组」
/// 这一 safe 提取动作集中到核心,调用方只需关心字段名;其余链式转换
/// (`.and_then(|a| a.first())` / `.cloned().unwrap_or_default()`)保持原位。
pub fn array_at<'a>(v: &'a Value, key: &str) -> Option<&'a Vec<Value>> {
    v.get(key).and_then(Value::as_array)
}

/// 取 `data` 信封数组(OKX/Bitget/Kucoin/Kraken 等 `{"code":"0","data":[...]}` 形)。
///
/// [`array_at`] 的 `data` 字段特化,收口原各适配器 `resp.get("data").and_then(Value::as_array)` 副本。
pub fn data_array(v: &Value) -> Option<&Vec<Value>> {
    array_at(v, "data")
}

/// 表驱动把统一 `timeframe`(如 `"1m"`/`"1h"`)映射到交易所原生粒度字符串。
///
/// 取代各适配器 `match timeframe { "1m" => "1m", "1h" => "1H", other => Err(...) }` 副本
/// (属 ADR-0013 describe 面)。`table` 形如 `&[("1m","1m"),("1h","1H")]`;未命中返回
/// [`ErrorKind::BadRequest`](unsupported timeframe)。仅适用于原生粒度是单个字符串的交易所
/// (coinbase 等返回 `(粒度, 秒数)` 元组的属真实差异,仍走各自 match)。
pub fn resolve_timeframe<'a>(table: &[(&str, &'a str)], timeframe: &str) -> Result<&'a str> {
    table
        .iter()
        .find(|(u, _)| *u == timeframe)
        .map(|(_, n)| *n)
        .ok_or_else(|| {
            Error::new(
                ErrorKind::BadRequest,
                format!("unsupported timeframe {timeframe}"),
            )
        })
}

/// `[price, amount, ...]` 行数组 → `(price, amount)` 元组(逐行跳过无效行)。
///
/// realtime 订单簿快照/增量共用(原 5 份 `collect_levels` 副本);与
/// [`parse_level`](Level 形状)互补。
pub fn collect_levels(v: Option<&Value>) -> Vec<(rust_decimal::Decimal, rust_decimal::Decimal)> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|row| {
                    let arr = row.as_array()?;
                    Some((value_decimal(arr.first()?)?, value_decimal(arr.get(1)?)?))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// f64 兜底(转译适配器专用,与手写集字符串精确解析语义不同)。
///
/// `serde_json` 无引号小数解析为 f64 时,`value_decimal` 走 `n.to_string()`
/// 已能还原十进制表示;此函数仅兜底二进制表示无法避免的误差场景,属
/// best-effort,不承诺与手写集精度一致(原 `dec_f64` 副本)。
pub fn dec_f64(f: f64) -> Option<rust_decimal::Decimal> {
    rust_decimal::Decimal::from_f64_retain(f)
}

/// `handle_errors` 接缝的通用实现(ADR-0013 四缝之一)。
///
/// 每次成功响应(HTTP 200 + 合法 JSON)后由 [`HttpCore::request_url`] 调用,
/// 识别 ccxt 基类常见的错误信封形状。识别到的错误经 [`classify_error_code`]
/// 按 `(exchange, code)` 升级为细粒度 [`ErrorKind`](如 `InsufficientFunds` /
/// `InvalidOrder` / `BadSymbol` / `RateLimitExceeded`),查表未命中则回退
/// [`ErrorKind::Exchange`];业务错误码同时写入 `context.http_error_code` 供适配
/// 器做更细分类。无法识别(合法成功响应 / 无错误信封 / 非对象体)返回 `None`,
/// 交还解析层。
///
/// 识别保守:仅对**明确**错误信号判定为错误,绝不误伤成功响应——
/// 这是与解析层"静默失败"相比的核心改进。覆盖形状:
/// - Kraken `{"error":["EQuery:..."]}`(非空数组)
/// - Bybit v5 `{"retCode":<非零>,"retMsg":"..."}`
/// - HTX/Huobi `{"status":"error","err-code":"...","err-msg":"..."}`
/// - 通用 `{"success":false,"msg":"..."}`
/// - 通用 `{"code":<非成功>,"msg":"..."}`(数字 0/200 或字符串
///   `"0"`/`"200"`/`"200000"`/`"00000"` 视为成功;如 binance 负码、
///   okx `"51000"`、kucoin `"400001"`、bitget 非 `"00000"`)
pub fn extract_exchange_error(body: &Value, exchange: &str) -> Option<Error> {
    let obj = body.as_object()?;

    // Kraken:{ "error": ["EQuery:Unknown asset"] } —— 非空 error 数组即错误
    if let Some(Value::Array(errs)) = obj.get("error") {
        if let Some(first) = errs
            .iter()
            .filter_map(|v| v.as_str())
            .find(|s| !s.is_empty())
        {
            // kraken 错误按消息前缀细分归类(收口到此处,ADR-0013 handle_errors 接缝)。
            if exchange == "kraken" {
                let mut ctx = ErrorContext::new();
                ctx = ctx.exchange("kraken".to_string());
                return Some(Error::with_context(
                    classify_kraken_error(first),
                    first.to_string(),
                    ctx,
                ));
            }
            return Some(error_with(exchange, first, code_str(obj.get("code"))));
        }
    }

    // Bybit v5:{ "retCode": 0, "retMsg": "OK", "result": {...} }
    if let Some(rc) = obj.get("retCode").and_then(to_i64) {
        if rc != 0 {
            let msg = pick_str(body, &["retMsg", "ret_msg"])
                .filter(|m| !m.is_empty())
                .unwrap_or("bybit error");
            return Some(error_with(
                exchange,
                &format!("{rc}: {msg}"),
                Some(rc.to_string()),
            ));
        }
    }

    // HTX/Huobi:{ "status": "error", "err-code": "...", "err-msg": "..." }
    if let Some(Value::String(st)) = obj.get("status") {
        if st == "error" {
            let msg =
                pick_str(body, &["err-msg", "err_msg", "message", "msg"]).unwrap_or("htx error");
            return Some(error_with(
                exchange,
                msg,
                obj.get("err-code")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
            ));
        }
    }

    // 通用:{ "success": false, ... } 或 { "code": <非成功>, "msg"/"message": "..." }
    if let Some(Value::Bool(false)) = obj.get("success") {
        let msg = pick_str(body, &["msg", "message"]).unwrap_or("request failed");
        return Some(error_with(exchange, msg, code_str(obj.get("code"))));
    }

    if let Some(code_val) = obj.get("code") {
        let is_success = match code_val {
            Value::Number(n) => n.as_i64().map(|c| c == 0 || c == 200).unwrap_or(false),
            Value::String(s) => matches!(s.as_str(), "0" | "200" | "200000" | "00000"),
            _ => false, // 非数字/字符串的 code 视为错误(保守)
        };
        if !is_success {
            let msg = pick_str(body, &["msg", "message"]).unwrap_or("exchange error");
            return Some(error_with(exchange, msg, code_str(Some(code_val))));
        }
    }

    None
}

/// 各交易所「错误码 → 细粒度 `ErrorKind`」映射表(候选 ⑤ / ADR-0013 已知遗留 ①)。
///
/// `extract_exchange_error` 识别到错误信封后,用 `(exchange, code)` 查此表:
/// 命中则升级为对应的细粒度 `ErrorKind`(影响重试性与调用方分支判断),未命中
/// 回退 [`ErrorKind::Exchange`](保持原有粗粒度行为)。仅收录高置信度、长期稳定
/// 的业务码;新增交易所/码在此扩展,无需改动识别逻辑。
static ERROR_CODE_MAP: &[(&str, &str, ErrorKind)] = &[
    // binance 负码(见 ccxt binance 文档,长期稳定)
    ("binance", "-1121", ErrorKind::BadSymbol),
    ("binance", "-2019", ErrorKind::InsufficientFunds),
    ("binance", "-2015", ErrorKind::Authentication),
    ("binance", "-1003", ErrorKind::RateLimitExceeded),
    ("binance", "-2010", ErrorKind::InvalidOrder),
    // bybit v5 retCode
    ("bybit", "10001", ErrorKind::InvalidOrder),
    ("bybit", "10003", ErrorKind::Authentication),
    ("bybit", "10005", ErrorKind::PermissionDenied),
    ("bybit", "10006", ErrorKind::RateLimitExceeded),
    // htx / huobi err-code
    ("htx", "invalid-symbol", ErrorKind::BadSymbol),
];

/// 把 `(exchange, code)` 解析为细粒度 `ErrorKind`;查表未命中或 `code` 为空
/// 回退 [`ErrorKind::Exchange`]。
fn classify_error_code(exchange: &str, code: &str) -> ErrorKind {
    if code.is_empty() {
        return ErrorKind::Exchange;
    }
    for (ex, c, kind) in ERROR_CODE_MAP {
        if *ex == exchange && *c == code {
            return *kind;
        }
    }
    ErrorKind::Exchange
}

/// Kraken 业务错误按消息前缀归类(无数字码,只有形如
/// `EAPI:Invalid key` / `EOrder:...` 的前缀串)。原 curated 适配器曾在
/// `ok_result` 内做此归类,但因 `HttpCore` 先拦截错误体而不可达;现收口到
/// `handle_errors` 接缝一处(ADR-0013)。
fn classify_kraken_error(msg: &str) -> ErrorKind {
    if msg.contains("EAPI:Invalid key") || msg.contains("EGeneral:Permission denied") {
        ErrorKind::Authentication
    } else if msg.contains("EOrder") || msg.contains("EAPI:Rate limit") {
        ErrorKind::RateLimitExceeded
    } else {
        ErrorKind::Exchange
    }
}
fn code_str(v: Option<&Value>) -> Option<String> {
    match v {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    }
}

/// 构造带交易所来源与错误码的分类错误(错误码经 [`classify_error_code`]
/// 升级为细粒度 [`ErrorKind`])。
fn error_with(exchange: &str, msg: &str, code: Option<String>) -> Error {
    let kind = classify_error_code(exchange, code.as_deref().unwrap_or(""));
    let mut ctx = ErrorContext::new();
    if !exchange.is_empty() {
        ctx = ctx.exchange(exchange.to_string());
    }
    if let Some(c) = code {
        ctx = ctx.http_error_code(c);
    }
    Error::with_context(kind, msg.to_string(), ctx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ---- handle_errors 接缝:extract_exchange_error ----

    #[test]
    fn kraken_auth_error_maps_to_authentication() {
        let body = json!({"error": ["EAPI:Invalid key"]});
        let err = extract_exchange_error(&body, "kraken").expect("should detect error");
        assert_eq!(err.kind, ErrorKind::Authentication);
        assert_eq!(err.context.exchange.as_deref(), Some("kraken"));
    }

    #[test]
    fn kraken_rate_limit_maps_to_rate_limit_exceeded() {
        let body = json!({"error": ["EAPI:Rate limit exceeded"]});
        let err = extract_exchange_error(&body, "kraken").expect("should detect error");
        assert_eq!(err.kind, ErrorKind::RateLimitExceeded);
    }

    #[test]
    fn kraken_unknown_error_maps_to_exchange() {
        let body = json!({"error": ["EGeneral:Unknown asset"]});
        let err = extract_exchange_error(&body, "kraken").expect("should detect error");
        assert_eq!(err.kind, ErrorKind::Exchange);
    }

    #[test]
    fn kraken_error_classification_is_exchange_scoped() {
        // 非 kraken 的 error 数组不触发 kraken 前缀归类,回退通用 Exchange。
        let body = json!({"error": ["EAPI:Invalid key"]});
        let err = extract_exchange_error(&body, "binance").expect("should detect error");
        assert_eq!(err.kind, ErrorKind::Exchange);
    }

    #[test]
    fn okx_error_body_maps_to_exchange_error() {
        let body = json!({"code":"51000","msg":"Order parameter invalid","data":null});
        let err = extract_exchange_error(&body, "okx").expect("should detect error");
        assert_eq!(err.kind, ErrorKind::Exchange);
        assert_eq!(err.context.http_error_code.as_deref(), Some("51000"));
        assert_eq!(err.context.exchange.as_deref(), Some("okx"));
    }

    #[test]
    fn okx_success_body_passes_through() {
        let body = json!({"code":"0","msg":"","data":[{"instId":"BTC-USDT"}]});
        assert!(extract_exchange_error(&body, "okx").is_none());
    }

    #[test]
    fn binance_negative_code_is_error() {
        let body = json!({"code":-1000,"msg":"An unknown error occurred"});
        let err = extract_exchange_error(&body, "binance").expect("should detect error");
        assert_eq!(err.kind, ErrorKind::Exchange);
        assert_eq!(err.context.http_error_code.as_deref(), Some("-1000"));
    }

    #[test]
    fn binance_success_order_passes_through() {
        // 成功订单响应通常无 code 字段
        let body = json!({"symbol":"BTCUSDT","orderId":123,"status":"NEW"});
        assert!(extract_exchange_error(&body, "binance").is_none());
    }

    #[test]
    fn kraken_error_array_is_error() {
        // `EOrder` 前缀按收口后的归类映射到 RateLimitExceeded(原适配器死代码即如此)。
        let body = json!({"error":["EOrder:Insufficient funds"]});
        let err = extract_exchange_error(&body, "kraken").expect("should detect error");
        assert_eq!(err.kind, ErrorKind::RateLimitExceeded);
        assert!(err.message.contains("Insufficient funds"));
    }

    #[test]
    fn kraken_empty_error_array_passes_through() {
        let body = json!({"error":[],"result":{"last":50000}});
        assert!(extract_exchange_error(&body, "kraken").is_none());
    }

    #[test]
    fn bybit_retcode_nonzero_is_error() {
        let body = json!({"retCode":10001,"retMsg":"param error","result":null});
        let err = extract_exchange_error(&body, "bybit").expect("should detect error");
        assert_eq!(err.context.http_error_code.as_deref(), Some("10001"));
        assert_eq!(err.kind, ErrorKind::InvalidOrder);
    }

    #[test]
    fn bybit_success_passes_through() {
        let body = json!({"retCode":0,"retMsg":"OK","result":{"price":"1"}});
        assert!(extract_exchange_error(&body, "bybit").is_none());
    }

    // ---- 客户端 limit 截断原语:apply_client_limit ----

    #[cfg(any(feature = "gemini", feature = "hyperliquid", feature = "kraken"))]
    #[test]
    fn apply_client_limit_none_keeps_all() {
        let mut v = vec![1, 2, 3];
        apply_client_limit(&mut v, None, false);
        assert_eq!(v, vec![1, 2, 3]);
    }

    #[cfg(any(feature = "gemini", feature = "hyperliquid", feature = "kraken"))]
    #[test]
    fn apply_client_limit_head_keeps_first() {
        let mut v = vec![1, 2, 3, 4, 5];
        apply_client_limit(&mut v, Some(2), false);
        assert_eq!(v, vec![1, 2]);
    }

    #[cfg(any(feature = "gemini", feature = "hyperliquid", feature = "kraken"))]
    #[test]
    fn apply_client_limit_tail_keeps_last() {
        let mut v = vec![1, 2, 3, 4, 5];
        apply_client_limit(&mut v, Some(2), true);
        assert_eq!(v, vec![4, 5]);
    }

    #[cfg(any(feature = "gemini", feature = "hyperliquid", feature = "kraken"))]
    #[test]
    fn apply_client_limit_ge_len_noop() {
        let mut v = vec![1, 2, 3];
        apply_client_limit(&mut v, Some(5), false);
        assert_eq!(v, vec![1, 2, 3]);
    }

    #[test]
    fn bitget_success_code_00000_passes_through() {
        let body = json!({"code":"00000","msg":"success","data":{"price":"1"}});
        assert!(extract_exchange_error(&body, "bitget").is_none());
    }

    #[test]
    fn kucoin_success_code_200000_passes_through() {
        let body = json!({"code":"200000","data":{"price":"1"}});
        assert!(extract_exchange_error(&body, "kucoin").is_none());
    }

    #[test]
    fn htx_status_error_is_error() {
        let body = json!({"status":"error","err-code":"invalid-symbol","err-msg":"invalid symbol"});
        let err = extract_exchange_error(&body, "htx").expect("should detect error");
        assert_eq!(
            err.context.http_error_code.as_deref(),
            Some("invalid-symbol")
        );
        assert_eq!(err.kind, ErrorKind::BadSymbol);
    }

    #[test]
    fn htx_status_ok_passes_through() {
        let body = json!({"status":"ok","data":{"price":"1"}});
        assert!(extract_exchange_error(&body, "htx").is_none());
    }

    #[test]
    fn raw_array_body_passes_through() {
        // 部分端点直接返回数组(无错误信封)
        let body = json!([{"symbol":"BTC-USDT"}]);
        assert!(extract_exchange_error(&body, "okx").is_none());
    }

    // ---- handle_errors 接缝:error_code_map 细粒度分类(候选 ⑤) ----

    #[test]
    fn binance_known_code_maps_to_fine_grained_kind() {
        // 仅收录高置信度长期稳定码;未命中表则回退 Exchange。
        let cases = [
            ("-1121", ErrorKind::BadSymbol),
            ("-2019", ErrorKind::InsufficientFunds),
            ("-2015", ErrorKind::Authentication),
            ("-1003", ErrorKind::RateLimitExceeded),
            ("-2010", ErrorKind::InvalidOrder),
        ];
        for (code, want) in cases {
            let body = json!({"code": code.parse::<i64>().unwrap(), "msg": "x"});
            let err = extract_exchange_error(&body, "binance").expect("detected");
            assert_eq!(err.kind, want, "code {code}");
            assert_eq!(err.context.http_error_code.as_deref(), Some(code));
        }
    }

    #[test]
    fn unmapped_code_falls_back_to_exchange() {
        // okx 51000 等业务码未入表,保持原有粗粒度行为。
        let body = json!({"code":"51000","msg":"param invalid"});
        let err = extract_exchange_error(&body, "okx").expect("detected");
        assert_eq!(err.kind, ErrorKind::Exchange);
        // 业务码仍写入 context 供适配器覆写。
        assert_eq!(err.context.http_error_code.as_deref(), Some("51000"));
    }

    #[test]
    fn value_decimal_parses_string_and_number() {
        assert_eq!(
            value_decimal(&json!("0.0015")),
            Some("0.0015".parse().unwrap())
        );
        assert_eq!(value_decimal(&json!(100)), Some("100".parse().unwrap()));
        assert_eq!(value_decimal(&Value::Null), None);
    }

    #[test]
    fn query_string_encodes_pairs() {
        let mut p = Params::new();
        p.insert("instType".into(), json!("SPOT"));
        p.insert("bar".into(), json!("1H"));
        let qs = query_string(&p);
        assert!(qs.starts_with('?'));
        assert!(qs.contains("instType=SPOT"));
        assert!(qs.contains("bar=1H"));
    }

    #[test]
    fn parse_ohlcv_standard_reads_canonical_array() {
        let row = json!([1499040000000i64, "0.1", "0.2", "0.05", "0.15", "10.5"]);
        let o = parse_ohlcv_standard(&row);
        assert_eq!(o.timestamp, Some(1499040000000));
        assert_eq!(o.open, Some("0.1".parse().unwrap()));
        assert_eq!(o.high, Some("0.2".parse().unwrap()));
        assert_eq!(o.low, Some("0.05".parse().unwrap()));
        assert_eq!(o.close, Some("0.15".parse().unwrap()));
        assert_eq!(o.volume, Some("10.5".parse().unwrap()));
    }

    #[test]
    fn iso8601_ms_emits_millisecond_z() {
        assert_eq!(
            iso8601_ms(1499040000000),
            Some("2017-07-03T00:00:00.000Z".to_string())
        );
    }

    #[test]
    fn query_string_empty_when_no_params() {
        assert_eq!(query_string(&Params::new()), "");
    }

    #[test]
    fn parse_level_takes_first_two_elements() {
        let l = parse_level(&json!(["100", "1", "0", "2"]));
        assert_eq!(l.price, Some("100".parse().unwrap()));
        assert_eq!(l.amount, Some("1".parse().unwrap()));
    }

    #[test]
    fn pct_encode_reserves_unreserved_chars() {
        assert_eq!(pct_encode("BTC/USDT"), "BTC%2FUSDT");
        assert_eq!(pct_encode("a-b_c.d~"), "a-b_c.d~");
    }

    // ---- safe 提取:array_at / data_array / resolve_timeframe(候选 ②) ----

    #[test]
    fn array_at_reads_named_field_as_array() {
        let v = json!({"data":[{"a":1},{"b":2}],"result":[3]});
        assert_eq!(array_at(&v, "data").map(Vec::len), Some(2));
        assert_eq!(array_at(&v, "result").map(Vec::len), Some(1));
        // 字段缺失或非数组 → None(不 panic)。
        assert!(array_at(&v, "missing").is_none());
        assert!(array_at(&json!({"data":"not array"}), "data").is_none());
    }

    #[test]
    fn data_array_is_array_at_data() {
        let v = json!({"code":"0","data":[{"instId":"BTC-USDT"}]});
        assert_eq!(data_array(&v).map(Vec::len), Some(1));
        assert!(data_array(&json!({"code":"0"})).is_none());
    }

    #[test]
    fn resolve_timeframe_maps_unified_to_native() {
        let tf: &[(&str, &str)] = &[("1m", "1m"), ("1h", "1H"), ("1d", "1D")];
        assert_eq!(resolve_timeframe(tf, "1h").unwrap(), "1H");
        assert_eq!(resolve_timeframe(tf, "1m").unwrap(), "1m");
        // 表驱动未命中 → BadRequest(unsupported timeframe)。
        assert_eq!(
            resolve_timeframe(tf, "2w").err().map(|e| e.kind),
            Some(ErrorKind::BadRequest)
        );
    }
}
