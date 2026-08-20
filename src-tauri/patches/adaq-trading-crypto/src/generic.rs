//! 转译生成的通用交易所引擎(ADR-0005 / 0013 / 0016)。
//!
//! `scripts/gen_adapters.py` 从 vendored ccxt 4.5.73 的 `describe()` 抽取每个
//! 交易所的元数据,生成 [`ApiSpec`](一个 `&'static` 数据描述),本引擎据此以
//! **描述驱动**方式实现 ccxt 公共 REST 方法面:通过 `api` 端点表路由 +
//! 共享 `parse_*`(best-effort)。无法通用实现的方法继承 trait 默认
//! `NotSupported`(与 ccxt 自身行为一致)。
//!
//! 签名采用 ccxt 基类默认 HMAC-SHA256(`apiKey`/`sign`/`timestamp` 头);
//! 无法通用覆盖的认证方案(如 RSA-PSS / JWT / ECDSA / 自定义头)由各交易所
//! 精选手写适配器覆盖(binance/okx/.../kalshi/polymarket)。
//!
//! 转译适配器是 ADR-0005 的 best-effort 批量补齐手段;精确性与差分测试由
//! 精选手写集保证。所有响应均保留 `info` 原始字段,用户始终可取底层数据。

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Value, json};

use crate::error::{Error, ErrorKind, Result};
use crate::exchange::{Config, Exchange, Params};
use crate::generic_parse::*;
use crate::httpcore::{HttpCore, now_ms, pick_i64, query_string};
use crate::types::{
    Balances, Currencies, Market, Markets, OHLCV, Order, OrderBook, Ticker, Tickers, Trade,
};

/// 市集类别(CEX / DEX / 预测市场),由转译器从 ccxt `dex` 标志 + 预测市场名单判定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarketKind {
    Cex,
    Dex,
    Prediction,
}

impl MarketKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            MarketKind::Cex => "cex",
            MarketKind::Dex => "dex",
            MarketKind::Prediction => "prediction",
        }
    }
}

/// 单个 REST 端点(已从 ccxt `api` 表扁平化,并解析出所属基址)。
#[derive(Debug, Clone, Copy)]
pub struct Endpoint {
    /// 该端点基址(已按 `urls.api` 解析,如 `https://api.binance.com/api/v3`)。
    pub base: &'static str,
    /// HTTP 动词(GET/POST/PUT/DELETE,大写)。
    pub verb: &'static str,
    /// ccxt 端点键(如 `ticker/price`、`order`、`depth`)。
    pub key: &'static str,
    /// 路径(相对基址,如 `ticker/price`)。
    pub path: &'static str,
    /// 是否需认证(私人端点)。
    pub auth: bool,
}

/// 转译生成的交易所描述(纯 `&'static` 数据,由脚本从 ccxt `describe()` 生成)。
#[derive(Debug, Clone, Copy)]
pub struct ApiSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub version: &'static str,
    pub rate_limit_ms: u64,
    /// `has` 能力名列表(ccxt 中为 `true` 的能力,如 `fetchTicker`)。
    pub has: &'static [&'static str],
    /// 扁平化后的 REST 端点表。
    pub endpoints: &'static [Endpoint],
    /// 吃单/挂单费率(来自 `fees.trading`,缺失为 0)。
    pub taker: f64,
    pub maker: f64,
    /// 支持的 K 线周期(来自 `timeframes` 的键)。
    pub timeframes: &'static [&'static str],
    pub kind: MarketKind,
}

/// 签名方案(ADR-0013 第四缝 `sign` 的结构化表达)。
///
/// 转译生成的适配器默认使用 ccxt 基类方案 `HmacSha256Default`
/// (`HMAC-SHA256(method+url+body)`,`apiKey`/`sign`/`timestamp` 头)。
/// 需要非默认签名的交易所(如 OKX/Binance/Bybit 的定制 HMAC 或 RSA-PSS)
/// 应写为手写适配器并覆写 `sign_headers`,或未来扩展此枚举按 `ApiSpec` 选择。
/// 生成层只 emit `HmacSha256Default`,故默认即“恰好能用”的显式化,而非隐式硬编码。
///
/// Signature scheme — the structured expression of ADR-0013's 4th seam `sign`.
/// Generated adapters default to the ccxt base `HmacSha256Default`. Exchanges
/// needing a custom scheme should be hand-written (overriding `sign_headers`)
/// or extend this enum selected per `ApiSpec` in the future. The transpiler
/// only emits `HmacSha256Default`, so the default is an explicit choice rather
/// than an implicit hard-code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignScheme {
    /// ccxt 基类默认:`HMAC-SHA256(method + url + body)`,`apiKey`/`sign`/`timestamp` 头。
    /// ccxt base default: HMAC-SHA256(method + url + body) with apiKey/sign/timestamp headers.
    HmacSha256Default,
}

/// 描述驱动的通用交易所适配器。
pub struct GenericExchange {
    config: Config,
    spec: &'static ApiSpec,
    core: HttpCore,
    sign_scheme: SignScheme,
}

impl GenericExchange {
    pub fn new(config: Config, spec: &'static ApiSpec, sign_scheme: SignScheme) -> Result<Self> {
        // 取任一端点基址作为 HttpCore 主基址(仅用于限速/缓存语境)。
        let primary = spec
            .endpoints
            .first()
            .map(|e| e.base)
            .unwrap_or("https://api.example.com");
        let core = HttpCore::new(&config, primary, spec.rate_limit_ms, spec.id)?;
        Ok(Self {
            config,
            spec,
            core,
            sign_scheme,
        })
    }

    fn id(&self) -> &'static str {
        self.spec.id
    }

    fn config(&self) -> &Config {
        &self.config
    }

    /// 运行时替换传输(离线测试桩注入,候选 6)。仅测试构建可用。
    #[cfg(test)]
    pub(crate) fn set_transport(
        &mut self,
        transport: Box<dyn crate::transport::Transport + Send + Sync>,
    ) {
        self.core.set_transport(transport);
    }

    // ---- 路由 ----

    /// 在端点表中按候选键(从具体到宽泛)查找首个匹配端点。
    ///
    /// `auth` 过滤公私端点;`prefer_verb` 在动词冲突时优先(如 `order` 的
    /// GET/POST/DELETE 同键)。匹配要求候选键作为“词”出现(前后非字母数字),
    /// 避免 `orders` 误匹配 `openOrders`。
    fn find_first(
        &self,
        candidates: &[&str],
        auth: bool,
        prefer_verb: Option<&str>,
    ) -> Option<&Endpoint> {
        let mut fallback: Option<&Endpoint> = None;
        for c in candidates {
            for ep in self.spec.endpoints {
                if ep.auth != auth {
                    continue;
                }
                let k = ep.key.to_lowercase().replace(['/', '_', '-', ' '], "");
                if key_matches(&k, c) {
                    if let Some(v) = prefer_verb {
                        if ep.verb.eq_ignore_ascii_case(v) {
                            return Some(ep);
                        }
                        if fallback.is_none() {
                            fallback = Some(ep);
                        }
                    } else if fallback.is_none() {
                        fallback = Some(ep);
                    }
                }
            }
            if fallback.is_some() {
                return fallback;
            }
        }
        fallback
    }

    // ---- 请求 ----

    async fn request_ep(&self, ep: &Endpoint, params: &Params) -> Result<Value> {
        let path = fill_path(ep.path, params);
        let base_url = join_url(ep.base, &path);
        let (method, target, body) = if ep.verb.eq_ignore_ascii_case("GET") {
            let qs = query_string(params);
            (ep.verb.to_string(), format!("{base_url}{qs}"), None)
        } else {
            (
                ep.verb.to_string(),
                base_url,
                Some(Value::Object(params.clone())),
            )
        };
        let mut headers = HeaderMap::new();
        if ep.auth {
            headers = self.sign_headers(&method, &target, body.as_ref())?;
        }
        self.core
            .request_url(&method, &target, &headers, body)
            .await
    }

    /// 按 `sign_scheme` 选择签名算法并写入认证头(ADR-0013 第四缝 `sign`)。
    ///
    /// Select the signing algorithm per `sign_scheme` and write auth headers
    /// (ADR-0013 seam #4 `sign`).
    fn sign_headers(&self, method: &str, url: &str, body: Option<&Value>) -> Result<HeaderMap> {
        match self.sign_scheme {
            // 当前唯一实现:ccxt 基类默认 HMAC-SHA256 方案。
            // Only implementation today: the ccxt base default HMAC-SHA256 scheme.
            SignScheme::HmacSha256Default => self.sign_hmac_sha256_default(method, url, body),
        }
    }

    /// ccxt 基类默认签名:HMAC-SHA256(method+url+body),`apiKey`/`sign`/`timestamp` 头。
    ///
    /// ccxt base default signature: HMAC-SHA256(method + url + body) with
    /// apiKey/sign/timestamp headers.
    fn sign_hmac_sha256_default(
        &self,
        method: &str,
        url: &str,
        body: Option<&Value>,
    ) -> Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        let api_key = self.config.api_key.as_deref().unwrap_or("");
        let secret = self.config.secret.as_deref().unwrap_or("");
        if api_key.is_empty() && secret.is_empty() {
            return Ok(headers);
        }
        let body_str = match body {
            Some(v) => v.to_string(),
            None => String::new(),
        };
        let prehash = format!("{}{}{}", method.to_uppercase(), url, body_str);
        let sig = crate::signing::hmac_sha256_hex(secret, &prehash);
        headers.insert(
            HeaderName::from_static("apikey"),
            HeaderValue::from_str(api_key).unwrap_or(HeaderValue::from_static("")),
        );
        headers.insert(
            HeaderName::from_static("sign"),
            HeaderValue::from_str(&sig).unwrap_or(HeaderValue::from_static("")),
        );
        headers.insert(
            HeaderName::from_static("timestamp"),
            HeaderValue::from_str(&now_ms().to_string()).unwrap_or(HeaderValue::from_static("")),
        );
        Ok(headers)
    }

    // ---- 市集 ----

    async fn fetch_markets_raw(&self) -> Result<Markets> {
        let ep = self
            .find_first(
                &["markets", "exchangeinfo", "instruments", "pairs", "symbols"],
                false,
                None,
            )
            .ok_or_else(|| Error::not_supported("fetch_markets"))?;
        let resp = self.request_ep(ep, &Params::new()).await?;
        Ok(parse_markets(&resp))
    }

    #[allow(dead_code)]
    fn has_cap(&self, name: &str) -> bool {
        self.spec.has.iter().any(|h| h.eq_ignore_ascii_case(name))
    }

    /// 该交易所 `has` 能力名列表(契约测试/可见性用)。
    pub fn capabilities(&self) -> &'static [&'static str] {
        self.spec.has
    }

    pub fn market_kind(&self) -> MarketKind {
        self.spec.kind
    }
}

#[allow(unused_variables, clippy::too_many_arguments)]
impl Exchange for GenericExchange {
    fn id(&self) -> &'static str {
        self.id()
    }
    fn config(&self) -> &Config {
        self.config()
    }

    async fn fetch_time(&self) -> Result<i64> {
        let ep = self
            .find_first(&["time"], false, None)
            .ok_or_else(|| Error::not_supported("fetch_time"))?;
        let resp = self.request_ep(ep, &Params::new()).await?;
        // ccxt 多数返回裸数字或 {serverTime:..}/{time:..}
        if let Some(n) = resp.as_i64() {
            return Ok(n);
        }
        if let Some(n) = pick_i64(&resp, &["time", "serverTime", "timestamp", "servertime"]) {
            return Ok(n);
        }
        Err(Error::new(
            ErrorKind::BadResponse,
            "fetch_time: no timestamp in response",
        ))
    }

    async fn fetch_status(&self) -> Result<Value> {
        let ep = self
            .find_first(
                &["status", "systemstatus", "system_status", "ping"],
                false,
                None,
            )
            .ok_or_else(|| Error::not_supported("fetch_status"))?;
        self.request_ep(ep, &Params::new()).await
    }

    async fn fetch_markets(&self) -> Result<Vec<Market>> {
        self.core.load_markets(|| self.fetch_markets_raw()).await?;
        Ok(self.core.markets_snapshot().into_values().collect())
    }

    async fn fetch_currencies(&self) -> Result<Currencies> {
        let ep = self
            .find_first(&["currencies", "assets", "currencys"], false, None)
            .ok_or_else(|| Error::not_supported("fetch_currencies"))?;
        let resp = self.request_ep(ep, &Params::new()).await?;
        Ok(parse_currencies(&resp))
    }

    async fn fetch_ticker(&self, symbol: &str, params: Params) -> Result<Ticker> {
        let ep = self
            .find_first(&["ticker"], false, None)
            .ok_or_else(|| Error::not_supported("fetch_ticker"))?;
        let mut p = params;
        if !symbol.is_empty() {
            p.insert("symbol".into(), json!(symbol));
        }
        let resp = self.request_ep(ep, &p).await?;
        Ok(parse_ticker(resolve_one(&resp, symbol), symbol))
    }

    async fn fetch_tickers(&self, symbols: Option<&[&str]>, params: Params) -> Result<Tickers> {
        let ep = self
            .find_first(&["tickers", "ticker"], false, None)
            .ok_or_else(|| Error::not_supported("fetch_tickers"))?;
        let mut p = params;
        if let Some(syms) = symbols {
            if !syms.is_empty() {
                p.insert("symbols".into(), json!(syms));
            }
        }
        let resp = self.request_ep(ep, &p).await?;
        Ok(parse_tickers(&resp))
    }

    async fn fetch_ohlcv(
        &self,
        symbol: &str,
        timeframe: &str,
        since: Option<i64>,
        limit: Option<i64>,
        params: Params,
    ) -> Result<Vec<OHLCV>> {
        let ep = self
            .find_first(
                &["ohlcv", "kline", "klines", "candle", "candles", "ohlc"],
                false,
                None,
            )
            .ok_or_else(|| Error::not_supported("fetch_ohlcv"))?;
        let mut p = params;
        if !symbol.is_empty() {
            p.insert("symbol".into(), json!(symbol));
        }
        if !timeframe.is_empty() {
            // 优先用交易所原生周期键(若 timeframes 含该键),否则透传。
            if self.spec.timeframes.iter().copied().any(|t| t == timeframe) {
                p.insert("interval".into(), json!(timeframe));
                p.insert("timeframe".into(), json!(timeframe));
            } else {
                p.insert("interval".into(), json!(timeframe));
            }
        }
        if let Some(s) = since {
            p.insert("startTime".into(), json!(s));
            p.insert("since".into(), json!(s));
        }
        if let Some(l) = limit {
            p.insert("limit".into(), json!(l));
        }
        let resp = self.request_ep(ep, &p).await?;
        Ok(parse_ohlcv(&resp))
    }

    async fn fetch_order_book(
        &self,
        symbol: &str,
        limit: Option<i64>,
        params: Params,
    ) -> Result<OrderBook> {
        let ep = self
            .find_first(
                &["orderbook", "order_book", "orderBook", "depth", "book"],
                false,
                None,
            )
            .ok_or_else(|| Error::not_supported("fetch_order_book"))?;
        let mut p = params;
        if !symbol.is_empty() {
            p.insert("symbol".into(), json!(symbol));
        }
        if let Some(l) = limit {
            p.insert("limit".into(), json!(l));
        }
        let resp = self.request_ep(ep, &p).await?;
        Ok(parse_order_book(&resp, symbol))
    }

    async fn fetch_trades(
        &self,
        symbol: &str,
        since: Option<i64>,
        limit: Option<i64>,
        params: Params,
    ) -> Result<Vec<Trade>> {
        let ep = self
            .find_first(&["trades", "trade"], false, None)
            .ok_or_else(|| Error::not_supported("fetch_trades"))?;
        let mut p = params;
        if !symbol.is_empty() {
            p.insert("symbol".into(), json!(symbol));
        }
        if let Some(s) = since {
            p.insert("startTime".into(), json!(s));
            p.insert("since".into(), json!(s));
        }
        if let Some(l) = limit {
            p.insert("limit".into(), json!(l));
        }
        let resp = self.request_ep(ep, &p).await?;
        Ok(parse_trades(resolve_one(&resp, symbol), symbol))
    }

    async fn fetch_balance(&self, params: Params) -> Result<Balances> {
        let ep = self
            .find_first(
                &["balance", "account", "wallet", "balances"],
                true,
                Some("GET"),
            )
            .ok_or_else(|| Error::not_supported("fetch_balance"))?;
        let resp = self.request_ep(ep, &params).await?;
        Ok(parse_balance(&resp))
    }

    async fn create_order(
        &self,
        symbol: &str,
        order_type: &str,
        side: &str,
        amount: &str,
        price: Option<&str>,
        params: Params,
    ) -> Result<Order> {
        let ep = self
            .find_first(&["order"], true, Some("POST"))
            .ok_or_else(|| Error::not_supported("create_order"))?;
        let mut p = params;
        p.insert("symbol".into(), json!(symbol));
        p.insert("type".into(), json!(order_type));
        p.insert("side".into(), json!(side));
        p.insert("amount".into(), json!(amount));
        if let Some(pr) = price {
            p.insert("price".into(), json!(pr));
        }
        let resp = self.request_ep(ep, &p).await?;
        Ok(parse_order(&resp, symbol))
    }

    async fn cancel_order(&self, id: &str, symbol: &str, params: Params) -> Result<Order> {
        let ep = self
            .find_first(&["order"], true, Some("DELETE"))
            .ok_or_else(|| Error::not_supported("cancel_order"))?;
        let mut p = params;
        p.insert("symbol".into(), json!(symbol));
        p.insert("orderId".into(), json!(id));
        p.insert("id".into(), json!(id));
        let resp = self.request_ep(ep, &p).await?;
        Ok(parse_order(&resp, symbol))
    }

    async fn fetch_order(&self, id: &str, symbol: &str, params: Params) -> Result<Order> {
        let ep = self
            .find_first(&["order"], true, Some("GET"))
            .ok_or_else(|| Error::not_supported("fetch_order"))?;
        let mut p = params;
        p.insert("symbol".into(), json!(symbol));
        p.insert("orderId".into(), json!(id));
        p.insert("id".into(), json!(id));
        let resp = self.request_ep(ep, &p).await?;
        Ok(parse_order(&resp, symbol))
    }

    async fn fetch_orders(
        &self,
        symbol: Option<&str>,
        since: Option<i64>,
        limit: Option<i64>,
        params: Params,
    ) -> Result<Vec<Order>> {
        let ep = self
            .find_first(&["orders", "order"], true, Some("GET"))
            .ok_or_else(|| Error::not_supported("fetch_orders"))?;
        let mut p = params;
        if let Some(s) = symbol {
            p.insert("symbol".into(), json!(s));
        }
        if let Some(s) = since {
            p.insert("startTime".into(), json!(s));
        }
        if let Some(l) = limit {
            p.insert("limit".into(), json!(l));
        }
        let resp = self.request_ep(ep, &p).await?;
        Ok(parse_orders(&resp, symbol))
    }

    async fn fetch_open_orders(
        &self,
        symbol: Option<&str>,
        since: Option<i64>,
        limit: Option<i64>,
        params: Params,
    ) -> Result<Vec<Order>> {
        let ep = self
            .find_first(
                &["openorders", "open_orders", "openOrders", "orders"],
                true,
                Some("GET"),
            )
            .ok_or_else(|| Error::not_supported("fetch_open_orders"))?;
        let mut p = params;
        if let Some(s) = symbol {
            p.insert("symbol".into(), json!(s));
        }
        if let Some(s) = since {
            p.insert("startTime".into(), json!(s));
        }
        if let Some(l) = limit {
            p.insert("limit".into(), json!(l));
        }
        let resp = self.request_ep(ep, &p).await?;
        Ok(parse_orders(&resp, symbol))
    }

    async fn fetch_my_trades(
        &self,
        symbol: Option<&str>,
        since: Option<i64>,
        limit: Option<i64>,
        params: Params,
    ) -> Result<Vec<Trade>> {
        let ep = self
            .find_first(
                &["mytrades", "mytrade", "myTrades", "trades"],
                true,
                Some("GET"),
            )
            .ok_or_else(|| Error::not_supported("fetch_my_trades"))?;
        let mut p = params;
        if let Some(s) = symbol {
            p.insert("symbol".into(), json!(s));
        }
        if let Some(s) = since {
            p.insert("startTime".into(), json!(s));
        }
        if let Some(l) = limit {
            p.insert("limit".into(), json!(l));
        }
        let resp = self.request_ep(ep, &p).await?;
        Ok(parse_trades(&resp, symbol.unwrap_or("")))
    }
}

// ============================================================
// 通用解析(best-effort,覆盖 ccxt 最常见字段形状;保留 `info` 原始响应)
// ============================================================

fn join_url(base: &str, path: &str) -> String {
    if path.starts_with('/') {
        format!("{base}{path}")
    } else {
        format!("{base}/{path}")
    }
}

/// 候选键 `c` 须作为“词”出现在 `k` 的某个 token 中。
///
/// `k` 形如 `public/ticker/{symbol}`(ccxt 端点键常带路径前缀与 `{模板}` 占位),
/// 先去掉 `{...}` 占位,再按非字母数字切分为 token,逐 token 匹配:token 与候选
/// 完全相等;或候选作为干净词边界出现(前后非字母数字);或 token 以已知动词
/// 前缀(`get`/`fetch`/...)开头且其后紧接候选(如 `getticker`→`ticker`、
/// `getorderbook`→`orderbook`)。这样既覆盖 bitflyer/independentreserve 的
/// `get`+名词命名,又不会让 `orders` 误匹配 `openorders`(`open` 不在前缀集)。
fn key_matches(k: &str, c: &str) -> bool {
    const PREFIXES: &[&str] = &["get", "fetch", "list", "query", "public"];
    let stripped: String = k.chars().filter(|ch| *ch != '{' && *ch != '}').collect();
    let tokens: Vec<&str> = stripped
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();
    for tok in tokens {
        let tok = tok.to_lowercase();
        if tok == c {
            return true;
        }
        if let Some(idx) = tok.find(c) {
            let before = &tok[..idx];
            let after = &tok[idx + c.len()..];
            let ok_before = !before.chars().any(|ch| ch.is_alphanumeric());
            let ok_after = !after.chars().any(|ch| ch.is_alphabetic());
            if ok_before && ok_after {
                return true;
            }
            // 前缀 + 候选:token 以 某前缀 起头且其后恰好接候选(或可再接边界)。
            if ok_after {
                for p in PREFIXES {
                    if before == *p {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// 将路径中的 `{param}` 模板替换为同名字段的(百分号编码)取值。
fn fill_path(path: &str, params: &Params) -> String {
    let mut out = String::with_capacity(path.len());
    let mut rest = path;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        if let Some(close) = rest[open..].find('}') {
            let name = &rest[open + 1..open + close];
            let val = params
                .get(name)
                .map(|v| match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .unwrap_or_default();
            out.push_str(&crate::httpcore::pct_encode(&val));
            rest = &rest[open + close + 1..];
        } else {
            out.push_str(&rest[open..]);
            break;
        }
    }
    out.push_str(rest);
    out
}

// ============================================================
// 宏:由 `ApiSpec` 生成一个交易所新类型(转发 16 个通用方法)
// ============================================================

/// 由 ccxt `describe()` 生成的 `ApiSpec` 生成一个交易所适配器新类型。
///
/// 生成的类型持有 [`GenericExchange`],仅转发已实现通用的 16 个方法;其余方法
/// 继承 trait 默认 `NotSupported`(与 ccxt 行为一致)。
#[macro_export]
macro_rules! impl_generated_adapter {
    // 两参形式:默认使用 ccxt 基类签名方案(显式默认,对应 ADR-0013 第四缝)。
    // Two-arg form: default to the ccxt base sign scheme (explicit default, ADR-0013 seam #4).
    ($name:ident, $spec:expr) => {
        $crate::impl_generated_adapter!(
            $name,
            $spec,
            $crate::generic::SignScheme::HmacSha256Default
        );
    };
    // 三参形式:显式指定签名方案(转译器可按交易所 emit)。
    // Three-arg form: explicit sign scheme (the transpiler may emit per exchange).
    ($name:ident, $spec:expr, $scheme:expr) => {
        /// 转译生成的交易所适配器(由 `scripts/gen_adapters.py` 从 ccxt
        /// `describe()` 生成,best-effort 批量补齐;精确性由精选手写集保证)。
        pub struct $name {
            inner: $crate::generic::GenericExchange,
        }

        impl $name {
            /// 构造转译适配器。
            pub fn new(config: $crate::exchange::Config) -> $crate::error::Result<Self> {
                Ok(Self {
                    inner: $crate::generic::GenericExchange::new(config, $spec, $scheme)?,
                })
            }
        }

        impl $crate::exchange::Exchange for $name {
            fn id(&self) -> &'static str {
                self.inner.id()
            }
            fn config(&self) -> &$crate::exchange::Config {
                self.inner.config()
            }
            async fn fetch_time(&self) -> $crate::error::Result<i64> {
                self.inner.fetch_time().await
            }
            async fn fetch_status(&self) -> $crate::error::Result<serde_json::Value> {
                self.inner.fetch_status().await
            }
            async fn fetch_markets(&self) -> $crate::error::Result<Vec<$crate::types::Market>> {
                self.inner.fetch_markets().await
            }
            async fn fetch_currencies(&self) -> $crate::error::Result<$crate::types::Currencies> {
                self.inner.fetch_currencies().await
            }
            async fn fetch_ticker(
                &self,
                symbol: &str,
                params: $crate::exchange::Params,
            ) -> $crate::error::Result<$crate::types::Ticker> {
                self.inner.fetch_ticker(symbol, params).await
            }
            async fn fetch_tickers(
                &self,
                symbols: Option<&[&str]>,
                params: $crate::exchange::Params,
            ) -> $crate::error::Result<$crate::types::Tickers> {
                self.inner.fetch_tickers(symbols, params).await
            }
            async fn fetch_ohlcv(
                &self,
                symbol: &str,
                timeframe: &str,
                since: Option<i64>,
                limit: Option<i64>,
                params: $crate::exchange::Params,
            ) -> $crate::error::Result<Vec<$crate::types::OHLCV>> {
                self.inner
                    .fetch_ohlcv(symbol, timeframe, since, limit, params)
                    .await
            }
            async fn fetch_order_book(
                &self,
                symbol: &str,
                limit: Option<i64>,
                params: $crate::exchange::Params,
            ) -> $crate::error::Result<$crate::types::OrderBook> {
                self.inner.fetch_order_book(symbol, limit, params).await
            }
            async fn fetch_trades(
                &self,
                symbol: &str,
                since: Option<i64>,
                limit: Option<i64>,
                params: $crate::exchange::Params,
            ) -> $crate::error::Result<Vec<$crate::types::Trade>> {
                self.inner.fetch_trades(symbol, since, limit, params).await
            }
            async fn fetch_balance(
                &self,
                params: $crate::exchange::Params,
            ) -> $crate::error::Result<$crate::types::Balances> {
                self.inner.fetch_balance(params).await
            }
            async fn create_order(
                &self,
                symbol: &str,
                order_type: &str,
                side: &str,
                amount: &str,
                price: Option<&str>,
                params: $crate::exchange::Params,
            ) -> $crate::error::Result<$crate::types::Order> {
                self.inner
                    .create_order(symbol, order_type, side, amount, price, params)
                    .await
            }
            async fn cancel_order(
                &self,
                id: &str,
                symbol: &str,
                params: $crate::exchange::Params,
            ) -> $crate::error::Result<$crate::types::Order> {
                self.inner.cancel_order(id, symbol, params).await
            }
            async fn fetch_order(
                &self,
                id: &str,
                symbol: &str,
                params: $crate::exchange::Params,
            ) -> $crate::error::Result<$crate::types::Order> {
                self.inner.fetch_order(id, symbol, params).await
            }
            async fn fetch_orders(
                &self,
                symbol: Option<&str>,
                since: Option<i64>,
                limit: Option<i64>,
                params: $crate::exchange::Params,
            ) -> $crate::error::Result<Vec<$crate::types::Order>> {
                self.inner.fetch_orders(symbol, since, limit, params).await
            }
            async fn fetch_open_orders(
                &self,
                symbol: Option<&str>,
                since: Option<i64>,
                limit: Option<i64>,
                params: $crate::exchange::Params,
            ) -> $crate::error::Result<Vec<$crate::types::Order>> {
                self.inner
                    .fetch_open_orders(symbol, since, limit, params)
                    .await
            }
            async fn fetch_my_trades(
                &self,
                symbol: Option<&str>,
                since: Option<i64>,
                limit: Option<i64>,
                params: $crate::exchange::Params,
            ) -> $crate::error::Result<Vec<$crate::types::Trade>> {
                self.inner
                    .fetch_my_trades(symbol, since, limit, params)
                    .await
            }
        }

        /// 交易所 id(与 `ApiSpec.id` 一致)。
        pub const ID: &str = $spec.id;
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_matches_word_boundary() {
        assert!(key_matches("order", "order"));
        assert!(key_matches("openorders", "openorders"));
        // "orders" 不应误匹配 "openorders"(前置字母)
        assert!(!key_matches("openorders", "orders"));
        assert!(!key_matches("allorders", "orders"));
        assert!(key_matches("orders", "orders"));
        // get+名词 命名(bitflyer / independentreserve)
        assert!(key_matches("getticker", "ticker"));
        assert!(key_matches("getorderbook", "orderbook"));
        // 中缀嵌入不算匹配(gettradehistorysummary 中 trade 后接 history)
        assert!(!key_matches("gettradehistorysummary", "trade"));
    }
}

// ============================================================
// 候选 6 落地后,5b 自然消化:用 `MockTransport` 离线断言请求形态。
// `fetch_ohlcv` 的 `interval`/`timeframe`、`startTime`/`since` 双键猜测
// 是防御性设计,此前只能联网验证;现注入内存桩即可断言实际发出的 query。
// ============================================================
#[cfg(test)]
mod transport_injection_tests {
    use super::*;
    use crate::transport::{MockTransport, RecordedRequest};
    use std::sync::{Arc, Mutex};

    /// 最小 `ApiSpec`:一个 GET `ohlcv` 端点 + `timeframes=["1m"]`,足以驱动
    /// `fetch_ohlcv` 的参数成形逻辑。
    fn sample_spec() -> &'static ApiSpec {
        static SPEC: ApiSpec = ApiSpec {
            id: "mock",
            name: "Mock",
            version: "1",
            rate_limit_ms: 1000,
            has: &["fetchOHLCV"],
            endpoints: &[Endpoint {
                base: "https://api.mock.com/v1",
                verb: "GET",
                key: "ohlcv",
                path: "klines",
                auth: false,
            }],
            taker: 0.0,
            maker: 0.0,
            timeframes: &["1m"],
            kind: MarketKind::Cex,
        };
        &SPEC
    }

    /// 构造带内存桩的 `GenericExchange`,返回桩与共享的请求记录句柄。
    fn mock_exchange() -> (GenericExchange, Arc<Mutex<Option<RecordedRequest>>>) {
        let mut config = Config::new();
        config.enable_rate_limit = false;
        let mut ex = GenericExchange::new(config, sample_spec(), SignScheme::HmacSha256Default)
            .expect("build mock exchange");
        let (mock, recorded) = MockTransport::new(Value::Array(vec![]));
        ex.set_transport(Box::new(mock));
        (ex, recorded)
    }

    #[tokio::test]
    async fn fetch_ohlcv_emits_both_interval_and_timeframe_for_known_tf() {
        let (ex, recorded) = mock_exchange();
        let _ = ex
            .fetch_ohlcv("BTC/USDT", "1m", None, None, Params::new())
            .await
            .expect("fetch_ohlcv");
        let url = recorded.lock().unwrap().as_ref().unwrap().url.clone();
        assert!(url.contains("symbol="), "symbol 应入参,url={url}");
        assert!(
            url.contains("interval=1m"),
            "已知周期应发 interval,url={url}"
        );
        assert!(
            url.contains("timeframe=1m"),
            "已知周期应同时发 timeframe(防御双键),url={url}"
        );
    }

    #[tokio::test]
    async fn fetch_ohlcv_emits_only_interval_for_unknown_tf() {
        let (ex, recorded) = mock_exchange();
        let _ = ex
            .fetch_ohlcv("BTC/USDT", "2m", None, None, Params::new())
            .await
            .expect("fetch_ohlcv");
        let url = recorded.lock().unwrap().as_ref().unwrap().url.clone();
        assert!(
            url.contains("interval=2m"),
            "未知周期仅透传 interval,url={url}"
        );
        assert!(
            !url.contains("timeframe="),
            "未知周期不应发 timeframe,url={url}"
        );
    }

    #[tokio::test]
    async fn fetch_ohlcv_emits_both_starttime_and_since() {
        let (ex, recorded) = mock_exchange();
        let since = 1_700_000_000_000_i64;
        let _ = ex
            .fetch_ohlcv("BTC/USDT", "1m", Some(since), None, Params::new())
            .await
            .expect("fetch_ohlcv");
        let url = recorded.lock().unwrap().as_ref().unwrap().url.clone();
        assert!(
            url.contains(&format!("startTime={since}")),
            "since 应同时发 startTime,url={url}"
        );
        assert!(
            url.contains(&format!("since={since}")),
            "since 应同时发 since(防御双键),url={url}"
        );
    }
}
