//! polymarket 预测市场适配器(M3,ADR-0005)。
//!
//! 对齐 ccxt `prediction/polymarket.py`(v4.5.73)的语义:
//! - 三组 API:Gamma(events/markets 只读)、CLOB(订单簿/交易/私密)、Data(成交/持仓);
//! - `base/quote = USDC/USDC`;symbol = shorten_slug 的 `{event}_{market}` 大写形式;
//! - outcome symbol = `{market}:{LABEL}`,token_id 来自 market `clobTokenIds`(JSON 字符串);
//! - 私密面:L2 HMAC-SHA256 over `{timestamp}{METHOD}{/path}{body}`,key 为 base64url
//!   解码的 secret,`POLY_*` 头;POLY_ADDRESS 为 EIP-55 校验和地址;
//! - 下单(EIP-712 订单签名)为 M3b 增量,当前返回 NotSupported。
//!
//! 参考实现基于 ccxt(MIT)适配器解析逻辑,见 `NOTICE`。

use std::collections::HashMap;

use base64::Engine;
use hmac::{Hmac, KeyInit, Mac};
use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::{Value, json};
use sha2::Sha256;
use sha3::Digest;

use crate::error::{Error, ErrorKind, Result};
use crate::exchange::{Config, Exchange, Params};
use crate::httpcore::{HttpCore, now_ms, query_string, value_decimal};
use crate::types::{
    Balance, Balances, Level, Market, MarketType, Markets, Order, OrderBook, Position, Precision,
    Ticker, Tickers, Trade,
};

pub const ID: &str = "polymarket";
const GAMMA_URL: &str = "https://gamma-api.polymarket.com";
const CLOB_URL: &str = "https://clob.polymarket.com";
const DATA_URL: &str = "https://data-api.polymarket.com";
const RATE_LIMIT_MS: u64 = 100;

/// 单个 outcome 的解析上下文(由 symbol / token_id 解析而来)。
#[derive(Debug, Clone)]
pub struct OutcomeCtx {
    pub token_id: String,
    pub condition_id: String,
    pub market_symbol: String,
    pub label: String,
    pub quote_volume: Option<rust_decimal::Decimal>,
}

/// polymarket 预测市场适配器。
pub struct Polymarket {
    config: Config,
    core: HttpCore,
    outcomes: crate::adapters::outcome::OutcomeIndex<OutcomeCtx>,
}

impl Polymarket {
    /// 已实现(非 NotSupported)的 REST 方法面(契约声明,M3)。
    /// 注:create_order(EIP-712 签名)为 M3b 增量,未声明。
    pub const IMPLEMENTED: &'static [&'static str] = &[
        "fetch_markets",
        "fetch_ticker",
        "fetch_tickers",
        "fetch_order_book",
        "fetch_trades",
        "fetch_balance",
        "fetch_positions",
        "create_order",
        "cancel_order",
    ];

    /// 构造适配器(限速 100ms/次,对齐 polymarket rateLimit)。
    pub fn new(config: Config) -> Result<Self> {
        // 多 base(data-api/gamma-api/clob)都经 core.request_url 全 URL 直发,
        // 此 base_url 仅作 HttpCore 占位。
        let core = HttpCore::new(&config, GAMMA_URL, RATE_LIMIT_MS, "polymarket")?;
        Ok(Self {
            config,
            core,
            outcomes: crate::adapters::outcome::OutcomeIndex::new(),
        })
    }

    // ================= 内部 HTTP =================

    async fn public_get(&self, base: &str, path: &str, params: &Params) -> Result<Value> {
        let url = format!("{base}{path}{}", query_string(params));
        self.core
            .request_url("GET", &url, &HeaderMap::new(), None)
            .await
    }

    async fn public_post(&self, base: &str, path: &str, body: Value) -> Result<Value> {
        let url = format!("{base}{path}");
        self.core
            .request_url("POST", &url, &HeaderMap::new(), Some(body))
            .await
    }

    /// CLOB 私密请求(L2 HMAC-SHA256,`POLY_*` 头)。
    async fn clob_private(
        &self,
        method: &str,
        path: &str,
        params: &Params,
        body: Option<Value>,
    ) -> Result<Value> {
        let api_key =
            self.config.api_key.as_deref().ok_or_else(|| {
                Error::new(ErrorKind::Authentication, "polymarket apiKey required")
            })?;
        let secret =
            self.config.secret.as_deref().ok_or_else(|| {
                Error::new(ErrorKind::Authentication, "polymarket secret required")
            })?;
        let passphrase =
            self.config.password.as_deref().ok_or_else(|| {
                Error::new(ErrorKind::Authentication, "polymarket password required")
            })?;
        let address = self
            .config
            .wallet_address
            .as_deref()
            .map(eth_checksum_address)
            .or_else(|| self.config.uid.clone())
            .ok_or_else(|| {
                Error::new(
                    ErrorKind::Authentication,
                    "polymarket wallet_address(或 uid)required",
                )
            })?;
        let timestamp = (now_ms() / 1000).to_string();
        // L2 HMAC 只签 path(不含 query);有 body 时追加 body
        let request_path = format!("/{path}");
        let mut auth = format!("{timestamp}{method}{request_path}");
        let body_json = body.clone().map(|v| v.to_string());
        if let Some(b) = &body_json {
            auth.push_str(b);
        }
        // secret 是 base64url 编码 → 解码为原始字节作 HMAC key
        let normalized = secret.replace('-', "+").replace('_', "/");
        let secret_bytes = base64::engine::general_purpose::STANDARD
            .decode(normalized)
            .map_err(|e| Error::new(ErrorKind::Authentication, format!("invalid secret: {e}")))?;
        let mut mac = Hmac::<Sha256>::new_from_slice(&secret_bytes)
            .map_err(|e| Error::new(ErrorKind::Authentication, format!("hmac key: {e}")))?;
        mac.update(auth.as_bytes());
        // url-safe base64,保留 '='
        let signature =
            base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
        let signature = signature.replace('+', "-").replace('/', "_");
        let mut headers = HeaderMap::new();
        headers.insert("POLY_ADDRESS", HeaderValue::from_str(&address).unwrap());
        headers.insert("POLY_API_KEY", HeaderValue::from_str(api_key).unwrap());
        headers.insert(
            "POLY_PASSPHRASE",
            HeaderValue::from_str(passphrase).unwrap(),
        );
        headers.insert("POLY_SIGNATURE", HeaderValue::from_str(&signature).unwrap());
        headers.insert("POLY_TIMESTAMP", HeaderValue::from_str(&timestamp).unwrap());
        let url = if method == "GET" {
            format!("{CLOB_URL}{path}{}", query_string(params))
        } else {
            format!("{CLOB_URL}{path}")
        };
        self.core.request_url(method, &url, &headers, body).await
    }

    // ================= markets / outcomes 索引 =================

    /// 确保 markets/outcomes 已加载(Gamma /events 翻页,对齐 eventsPageSize=100)。
    pub(crate) async fn load_markets(&self) -> Result<()> {
        self.core.load_markets(|| self.fetch_markets_raw()).await
    }

    /// 拉取并解析市集 + outcomes 索引(字段映射接缝;缓存由核心 `HttpCore::load_markets` 负责)。
    async fn fetch_markets_raw(&self) -> Result<Markets> {
        let limit = 200usize; // fetchMarketsLimit(默认)
        let page_size = 100usize; // eventsPageSize(Gamma 硬上限)
        let mut market_map = HashMap::new();
        let mut outcomes = HashMap::new();
        let mut offset = 0usize;
        loop {
            let mut p = Params::new();
            p.insert("limit".into(), json!(page_size));
            p.insert("offset".into(), json!(offset));
            p.insert("order".into(), json!("volume"));
            p.insert("ascending".into(), json!(false));
            p.insert("status".into(), json!("active"));
            let resp = self.public_get(GAMMA_URL, "/events", &p).await?;
            let events = resp
                .as_array()
                .ok_or_else(|| Error::new(ErrorKind::BadResponse, "gamma /events not array"))?;
            if events.is_empty() {
                break;
            }
            for ev in events {
                let event_slug = ev["slug"]
                    .as_str()
                    .or_else(|| ev["id"].as_str())
                    .unwrap_or_default();
                let raw_markets = ev
                    .get("markets")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                for raw in &raw_markets {
                    let m = self.parse_market_from_event(raw, event_slug);
                    if m.id.is_empty() {
                        continue;
                    }
                    let symbol = m.symbol.clone();
                    // 构建 outcomes 索引
                    let token_ids = parse_json_array(raw.get("clobTokenIds"));
                    let labels = parse_json_array(raw.get("outcomes"));
                    let condition_id = raw["conditionId"].as_str().unwrap_or_default().to_string();
                    for (i, label) in labels.iter().enumerate() {
                        let token_id = token_ids.get(i).cloned().unwrap_or_else(|| "".to_string());
                        if token_id.is_empty() {
                            continue;
                        }
                        outcomes.insert(
                            format!("{symbol}:{label}"),
                            OutcomeCtx {
                                token_id: token_id.clone(),
                                condition_id: condition_id.clone(),
                                market_symbol: symbol.clone(),
                                label: label.to_uppercase(),
                                quote_volume: raw.get("volume").and_then(value_decimal),
                            },
                        );
                        // 也允许裸 token_id 直接解析
                        outcomes.insert(
                            token_id.clone(),
                            OutcomeCtx {
                                token_id,
                                condition_id: condition_id.clone(),
                                market_symbol: symbol.clone(),
                                label: label.to_uppercase(),
                                quote_volume: raw.get("volume").and_then(value_decimal),
                            },
                        );
                    }
                    market_map.insert(symbol, m);
                }
            }
            offset += page_size;
            if offset >= limit {
                break;
            }
        }
        self.outcomes.store(outcomes);
        Ok(market_map)
    }

    /// 解析统一 symbol → outcome 上下文(outcome symbol 或裸 token_id)。
    pub(crate) fn resolve_outcome(&self, symbol: &str) -> Result<OutcomeCtx> {
        self.outcomes.resolve(symbol, "polymarket")
    }

    // ================= parse(公开,供差分测试) =================

    /// 解析 Gamma market 对象为统一 Market(对齐 ccxt parse_event_to_markets)。
    pub fn parse_market_from_event(&self, raw: &Value, event_slug: &str) -> Market {
        let condition_id = raw["conditionId"].as_str().unwrap_or_default();
        let market_id = raw["id"].as_str().unwrap_or_default();
        let market_slug = raw["slug"].as_str().unwrap_or(condition_id);
        let active = raw["active"].as_bool().unwrap_or(false);
        let closed = raw["closed"].as_bool().unwrap_or(false);
        let tick_size = raw
            .get("orderPriceMinTickSize")
            .or_else(|| raw.get("minimumTickSize"))
            .and_then(Value::as_str)
            .unwrap_or("0.01");
        let _order_min_size = raw
            .get("orderMinSize")
            .and_then(value_decimal)
            .unwrap_or_else(|| 1u8.into());
        let labels = parse_json_array(raw.get("outcomes"));
        let market_type = if labels.len() > 2 {
            MarketType::Categorical
        } else {
            MarketType::Binary
        };
        let end_date = raw
            .get("endDate")
            .or_else(|| raw.get("end_date_iso"))
            .and_then(Value::as_str);
        let market_symbol = crate::adapters::kalshi::slug_to_market_symbol(event_slug, market_slug);
        Market {
            id: condition_id.to_string(),
            symbol: market_symbol,
            base: Some("USDC".into()),
            quote: Some("USDC".into()),
            base_id: Some(if condition_id.is_empty() {
                market_id.to_string()
            } else {
                condition_id.to_string()
            }),
            quote_id: Some("USDC".into()),
            active: Some(active && !closed),
            market_type: Some(market_type),
            spot: Some(false),
            margin: Some(false),
            swap: Some(false),
            future: Some(false),
            option: Some(false),
            taker: Some(0u8.into()),
            maker: Some(0u8.into()),
            expiry: end_date.and_then(crate::adapters::kalshi::parse_iso_ms),
            expiry_datetime: end_date.map(str::to_string),
            precision: Precision {
                price: Some(tick_size.parse().unwrap_or_default()),
                amount: Some(tick_size.parse().unwrap_or_default()),
                cost: None,
            },
            limits: crate::types::Limits::default(),
            info: raw.clone(),
            ..Market::default()
        }
    }

    /// 解析订单簿(CLOB bids/asks `[{price,size}]`)。
    pub fn parse_order_book(&self, raw: &Value, symbol: &str) -> OrderBook {
        let mut bids: Vec<Level> = raw
            .get("bids")
            .and_then(Value::as_array)
            .map(|a| a.iter().map(parse_clob_level).collect())
            .unwrap_or_default();
        let mut asks: Vec<Level> = raw
            .get("asks")
            .and_then(Value::as_array)
            .map(|a| a.iter().map(parse_clob_level).collect())
            .unwrap_or_default();
        // ccxt parse_order_book 默认排序:bids 降序、asks 升序
        bids.sort_by_key(|l| std::cmp::Reverse(l.price));
        asks.sort_by_key(|l| l.price);
        let timestamp = raw["timestamp"]
            .as_str()
            .and_then(|s| s.parse::<i64>().ok());
        OrderBook {
            symbol: symbol.to_string(),
            bids,
            asks,
            timestamp,
            nonce: None,
            info: raw.clone(),
            ..OrderBook::default()
        }
    }

    /// 解析成交(data-api trade 对象)。
    pub fn parse_trade(&self, raw: &Value, symbol: &str, _token_id: &str) -> Trade {
        let price = raw.get("price").and_then(value_decimal);
        let amount = raw
            .get("size")
            .or_else(|| raw.get("amount"))
            .and_then(value_decimal);
        let ts = raw
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(|s| s.parse::<i64>().ok());
        Trade {
            id: raw
                .get("id")
                .or_else(|| raw.get("transactionHash"))
                .and_then(Value::as_str)
                .map(str::to_string),
            timestamp: ts,
            datetime: ts.and_then(crate::httpcore::iso8601),
            symbol: Some(symbol.to_string()),
            side: raw
                .get("side")
                .and_then(Value::as_str)
                .map(|s| s.to_lowercase()),
            price,
            amount,
            cost: match (price, amount) {
                (Some(p), Some(a)) => Some(p * a),
                _ => None,
            },
            info: raw.clone(),
            ..Trade::default()
        }
    }

    /// 解析 ticker(3 端点合并:midpoint/book/last-trade)。
    pub fn parse_ticker(&self, raw: &Value, ctx: &OutcomeCtx) -> Ticker {
        let mid = raw
            .get("midpoint")
            .and_then(|m| m.get("mid"))
            .and_then(value_decimal);
        let book = raw.get("book").unwrap_or(&Value::Null);
        let bids = book
            .get("bids")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let asks = book
            .get("asks")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        // CLOB 从远离 touch 排序:最优价在最后
        let best_bid = bids.last();
        let best_ask = asks.last();
        let last_trade = raw
            .get("lastTrade")
            .and_then(|l| l.get("price"))
            .and_then(value_decimal);
        let last = last_trade
            .filter(|l| !l.is_zero())
            .or(mid)
            .filter(|v| !v.is_zero());
        let timestamp = book["timestamp"]
            .as_str()
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or_else(now_ms);
        Ticker {
            symbol: format!("{}:{}", ctx.market_symbol, ctx.label.to_uppercase()),
            timestamp: Some(timestamp),
            datetime: crate::httpcore::iso8601(timestamp),
            bid: best_bid.and_then(|b| b.get("price").and_then(value_decimal)),
            ask: best_ask.and_then(|a| a.get("price").and_then(value_decimal)),
            bid_volume: best_bid.and_then(|b| b.get("size").and_then(value_decimal)),
            ask_volume: best_ask.and_then(|a| a.get("size").and_then(value_decimal)),
            close: last,
            last,
            average: mid,
            quote_volume: ctx.quote_volume,
            info: raw.clone(),
            ..Ticker::default()
        }
    }

    /// 解析持仓(data-api position)。
    pub fn parse_position(&self, raw: &Value) -> Position {
        let contracts = raw.get("size").and_then(value_decimal);
        Position {
            id: raw["asset"].as_str().map(str::to_string),
            contracts,
            entry_price: raw
                .get("avgPrice")
                .or_else(|| raw.get("average_price"))
                .and_then(value_decimal),
            unrealized_pnl: raw.get("unrealized_pnl").and_then(value_decimal),
            info: raw.clone(),
            ..Position::default()
        }
    }
}

// ================= Exchange trait 实现 =================

impl Exchange for Polymarket {
    fn id(&self) -> &'static str {
        ID
    }
    fn config(&self) -> &Config {
        &self.config
    }

    async fn fetch_markets(&self) -> Result<Vec<Market>> {
        self.load_markets().await?;
        Ok(self.core.markets_snapshot().into_values().collect())
    }

    async fn fetch_ticker(&self, symbol: &str, _params: Params) -> Result<Ticker> {
        self.load_markets().await?;
        let ctx = self.resolve_outcome(symbol)?;
        let p = params1("token_id", &ctx.token_id);
        let mid = self.public_get(CLOB_URL, "/midpoint", &p);
        let book = self.public_get(CLOB_URL, "/book", &p);
        let last_trade = self.public_get(CLOB_URL, "/last-trade-price", &p);
        let (mid_r, book_r, last_r) = tokio::join!(mid, book, last_trade);
        let raw = json!({
            "midpoint": mid_r?,
            "book": book_r?,
            "lastTrade": last_r?,
        });
        Ok(self.parse_ticker(&raw, &ctx))
    }

    async fn fetch_tickers(&self, symbols: Option<&[&str]>, _params: Params) -> Result<Tickers> {
        self.load_markets().await?;
        let symbols = symbols.ok_or_else(|| {
            Error::new(
                ErrorKind::BadRequest,
                "polymarket fetch_tickers 需要 outcomes 参数(无全量 ticker 端点)",
            )
        })?;
        let mut by_token: HashMap<String, OutcomeCtx> = HashMap::new();
        let mut token_ids: Vec<String> = Vec::new();
        for s in symbols {
            let ctx = self.resolve_outcome(s)?;
            if !by_token.contains_key(&ctx.token_id) {
                let tid = ctx.token_id.clone();
                token_ids.push(tid);
                by_token.insert(ctx.token_id.clone(), ctx);
            }
        }
        let mut out = Tickers::new();
        for chunk in token_ids.chunks(200) {
            let body = json!(
                chunk
                    .iter()
                    .map(|t| json!({"token_id": t}))
                    .collect::<Vec<_>>()
            );
            let books = self.public_post(CLOB_URL, "/books", body.clone()).await?;
            let midpoints = self
                .public_post(CLOB_URL, "/midpoints", body.clone())
                .await?;
            let last_trades = self
                .public_post(CLOB_URL, "/last-trades-prices", body)
                .await?;
            let mid_map: HashMap<String, String> = midpoints
                .as_object()
                .map(|o| {
                    o.iter()
                        .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string()))
                        .collect()
                })
                .unwrap_or_default();
            let last_map: HashMap<String, Value> = last_trades
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|e| {
                            let t = e["token_id"].as_str()?.to_string();
                            Some((t, e.clone()))
                        })
                        .collect()
                })
                .unwrap_or_default();
            if let Some(arr) = books.as_array() {
                for book in arr {
                    let tid = book["asset_id"].as_str().unwrap_or_default().to_string();
                    if let Some(ctx) = by_token.get(&tid) {
                        let raw = json!({
                            "midpoint": {"mid": mid_map.get(&tid).cloned().unwrap_or_default()},
                            "book": book,
                            "lastTrade": last_map.get(&tid).cloned().unwrap_or(Value::Null),
                        });
                        let t = self.parse_ticker(&raw, ctx);
                        out.insert(format!("{}:{}", ctx.market_symbol, ctx.label), t);
                    }
                }
            }
        }
        Ok(out)
    }

    async fn fetch_order_book(
        &self,
        symbol: &str,
        _limit: Option<i64>,
        _params: Params,
    ) -> Result<OrderBook> {
        self.load_markets().await?;
        let ctx = self.resolve_outcome(symbol)?;
        let p = params1("token_id", &ctx.token_id);
        let resp = self.public_get(CLOB_URL, "/book", &p).await?;
        Ok(self.parse_order_book(&resp, &format!("{}:{}", ctx.market_symbol, ctx.label)))
    }

    async fn fetch_trades(
        &self,
        symbol: &str,
        _since: Option<i64>,
        limit: Option<i64>,
        _params: Params,
    ) -> Result<Vec<Trade>> {
        self.load_markets().await?;
        let ctx = self.resolve_outcome(symbol)?;
        let mut p = Params::new();
        p.insert("market".into(), json!(ctx.condition_id));
        let page = limit.unwrap_or(500);
        p.insert("limit".into(), json!(page.min(1000)));
        let resp = self.public_get(DATA_URL, "/trades", &p).await?;
        let arr = match resp {
            Value::Array(a) => a,
            other => other
                .get("data")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
        };
        Ok(arr
            .iter()
            .filter(|t| t["asset"].as_str() == Some(ctx.token_id.as_str()))
            .map(|t| {
                self.parse_trade(
                    t,
                    &format!("{}:{}", ctx.market_symbol, ctx.label),
                    &ctx.token_id,
                )
            })
            .collect())
    }

    async fn fetch_balance(&self, _params: Params) -> Result<Balances> {
        let mut p = Params::new();
        p.insert("asset_type".into(), json!("COLLATERAL"));
        p.insert("signature_type".into(), json!(3));
        let resp = self
            .clob_private("GET", "balance-allowance", &p, None)
            .await?;
        // balance 为 6 位小数单位(如 "8992211" = 8.992211 USDC)
        let total = resp
            .get("balance")
            .and_then(value_decimal)
            .map(|b| b / rust_decimal::Decimal::from(1_000_000));
        let mut out = Balances {
            info: resp,
            ..Balances::default()
        };
        out.accounts.insert(
            "USDC".into(),
            Balance {
                free: total,
                total,
                ..Balance::default()
            },
        );
        Ok(out)
    }

    async fn fetch_positions(
        &self,
        _symbols: Option<&[&str]>,
        _params: Params,
    ) -> Result<Vec<Position>> {
        let user = self.config.wallet_address.as_deref().ok_or_else(|| {
            Error::new(
                ErrorKind::Authentication,
                "polymarket wallet_address required for fetch_positions",
            )
        })?;
        let p = params1("user", user);
        let resp = self.public_get(DATA_URL, "/positions", &p).await?;
        let arr = resp
            .get("data")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(arr.iter().map(|x| self.parse_position(x)).collect())
    }

    async fn create_order(
        &self,
        symbol: &str,
        order_type: &str,
        side: &str,
        amount: &str,
        price: Option<&str>,
        _params: Params,
    ) -> Result<Order> {
        self.load_markets().await?;
        let ctx = self.resolve_outcome(symbol)?;
        let price_str = price.ok_or_else(|| {
            Error::new(
                ErrorKind::BadRequest,
                "polymarket create_order 需要 price(0..1)",
            )
        })?;
        let private_key = self.config.private_key.as_deref().ok_or_else(|| {
            Error::new(
                ErrorKind::Authentication,
                "polymarket privateKey required for create_order",
            )
        })?;
        let eoa = crate::eip712::address_from_private_key(private_key)?;
        let maker = self
            .config
            .wallet_address
            .clone()
            .unwrap_or_else(|| eoa.clone());
        let size = amount.parse::<rust_decimal::Decimal>().map_err(|e| {
            Error::new(
                ErrorKind::BadRequest,
                format!("invalid amount {amount}: {e}"),
            )
        })?;
        let px = price_str.parse::<rust_decimal::Decimal>().map_err(|e| {
            Error::new(
                ErrorKind::BadRequest,
                format!("invalid price {price_str}: {e}"),
            )
        })?;
        // amounts(EOA 路径,tickSize 0.01 默认):BUY taker=size/maker=round(size*price,4);SELL 反向
        let (maker_amount, taker_amount) = if side == "buy" {
            let taker = size.round_dp(2);
            let maker = (taker * px).round_dp(4);
            (maker, taker)
        } else {
            let maker = size.round_dp(2);
            let taker = (maker * px).round_dp(4);
            (maker, taker)
        };
        let scale = rust_decimal::Decimal::from(1_000_000u64);
        let maker_raw = (maker_amount * scale).to_string();
        let taker_raw = (taker_amount * scale).to_string();
        let now_ms = now_ms();
        let salt = now_ms.to_string();
        let timestamp = now_ms.to_string();
        let bytes32_zero = format!("0x{}", "00".repeat(32));
        let side_int = if side == "buy" { 0u8 } else { 1u8 };
        // EIP-712 Order 签名(EOA,signatureType 0)
        let order_type_str = "Order(uint256 salt,address maker,address signer,uint256 tokenId,uint256 makerAmount,uint256 takerAmount,uint8 side,uint8 signatureType,uint256 timestamp,bytes32 metadata,bytes32 builder)";
        let fields = vec![
            crate::eip712::Field::Uint256(salt.clone()),
            crate::eip712::Field::Address(maker.clone()),
            crate::eip712::Field::Address(eoa.clone()),
            crate::eip712::Field::Uint256(ctx.token_id.clone()),
            crate::eip712::Field::Uint256(maker_raw.clone()),
            crate::eip712::Field::Uint256(taker_raw.clone()),
            crate::eip712::Field::Uint8(side_int),
            crate::eip712::Field::Uint8(0),
            crate::eip712::Field::Uint256(timestamp.clone()),
            crate::eip712::Field::Bytes32(bytes32_zero.clone()),
            crate::eip712::Field::Bytes32(bytes32_zero.clone()),
        ];
        let digest = crate::eip712::digest(
            "Polymarket CTF Exchange",
            "2",
            137,
            "0xE111180000d2663C0091e4f400237545B87B996B",
            order_type_str,
            &fields,
        )?;
        let signature = crate::signing::sign_ecdsa_recoverable(private_key, &digest)?;
        let order_type_tif = if order_type == "market" { "FOK" } else { "GTC" };
        let body = json!({
            "deferExec": false,
            "postOnly": false,
            "order": {
                "salt": salt,
                "maker": maker,
                "signer": eoa,
                "taker": "0x0000000000000000000000000000000000000000",
                "tokenId": ctx.token_id,
                "makerAmount": maker_raw,
                "takerAmount": taker_raw,
                "side": side.to_uppercase(),
                "signatureType": 0,
                "timestamp": timestamp,
                "expiration": "0",
                "metadata": bytes32_zero,
                "builder": format!("0x{}", "00".repeat(32)),
                "signature": format!("0x{signature}"),
            },
            "owner": self.config.api_key.clone().unwrap_or_default(),
            "orderType": order_type_tif,
        });
        let resp = self
            .clob_private("POST", "order", &Params::new(), Some(body))
            .await?;
        let mut order = Order {
            id: resp["orderID"].as_str().map(str::to_string),
            status: Some(
                if resp["success"].as_bool().unwrap_or(false) {
                    "open"
                } else {
                    "rejected"
                }
                .into(),
            ),
            symbol: Some(symbol.to_string()),
            order_type: Some(order_type.to_string()),
            side: Some(side.to_string()),
            price: Some(px),
            amount: Some(size),
            info: resp,
            ..Order::default()
        };
        if order.id.is_none() {
            order.id = order
                .info
                .get("order")
                .and_then(|o| o.get("orderID"))
                .and_then(Value::as_str)
                .map(str::to_string);
        }
        Ok(order)
    }

    async fn cancel_order(&self, id: &str, _symbol: &str, _params: Params) -> Result<Order> {
        let p = params1("orderID", id);
        let resp = self.clob_private("DELETE", "order", &p, None).await?;
        // DELETE 返回 {canceled: [...], not_canceled: {id: reason}}
        let not_canceled = resp
            .get("not_canceled")
            .and_then(Value::as_object)
            .map(|o| o.contains_key(id))
            .unwrap_or(false);
        Ok(Order {
            id: Some(id.to_string()),
            status: Some(if not_canceled { "open" } else { "canceled" }.into()),
            info: resp,
            ..Order::default()
        })
    }
}

impl Polymarket {
    /// 扩展面(不在统一 trait):按钱包地址查持仓(与 fetch_positions 同端点)。
    pub async fn fetch_positions_for_wallet(&self, wallet: &str) -> Result<Vec<Position>> {
        let p = params1("user", wallet);
        let resp = self.public_get(DATA_URL, "/positions", &p).await?;
        let arr = resp
            .get("data")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(arr.iter().map(|x| self.parse_position(x)).collect())
    }
}

// ================= 静态助手 =================

/// 解析 JSON 字符串数组(如 `"[\"Yes\",\"No\"]"`)。
pub fn parse_json_array(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::String(s)) => serde_json::from_str::<Vec<String>>(s).unwrap_or_default(),
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|x| x.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    }
}

/// `[{price, size}]` → Level。
fn parse_clob_level(raw: &Value) -> Level {
    Level {
        price: raw.get("price").and_then(value_decimal),
        amount: raw.get("size").and_then(value_decimal),
    }
}

/// EIP-55 校验和地址。
pub fn eth_checksum_address(address: &str) -> String {
    let clean = address.trim_start_matches("0x").to_lowercase();
    let mut hasher = sha3::Keccak256::new();
    hasher.update(clean.as_bytes());
    let hash = hasher.finalize();
    let hash_hex: Vec<char> = hex::encode(hash).chars().collect();
    let mut result = String::with_capacity(42);
    result.push_str("0x");
    for (i, ch) in clean.chars().enumerate() {
        if ch.is_ascii_digit() {
            result.push(ch);
        } else {
            // hash 的第 i 个 nibble >= 8 → 该字母大写(EIP-55)
            let nibble = hash_hex[i].to_digit(16).unwrap_or(0);
            if nibble >= 8 {
                result.push(ch.to_ascii_uppercase());
            } else {
                result.push(ch);
            }
        }
    }
    result
}

fn params1(k: &str, v: &str) -> Params {
    let mut p = Params::new();
    p.insert(k.into(), json!(v));
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_json_array_handles_string_and_array() {
        assert_eq!(
            parse_json_array(Some(&json!("[\"Yes\", \"No\"]"))),
            vec!["Yes".to_string(), "No".to_string()]
        );
        assert_eq!(
            parse_json_array(Some(&json!(["A", "B"]))),
            vec!["A".to_string(), "B".to_string()]
        );
        assert_eq!(parse_json_array(None), Vec::<String>::new());
    }

    #[test]
    fn checksum_address_is_eip55() {
        // 已知 EIP-55 用例
        assert_eq!(
            eth_checksum_address("0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed"),
            "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed"
        );
        assert_eq!(
            eth_checksum_address("0x52908400098527886E0F7030069857D2E4169EE7"),
            "0x52908400098527886E0F7030069857D2E4169EE7"
        );
        // 全数字地址校验和不变
        let all_digits = "0x0000000000000000000000000000000000000001";
        assert_eq!(eth_checksum_address(all_digits), all_digits);
    }

    #[test]
    fn parse_market_from_event_binary() {
        let ex = Polymarket::new(Config::new()).unwrap();
        let raw = json!({
            "conditionId": "0xabc",
            "slug": "will-it-rain",
            "active": true,
            "closed": false,
            "outcomes": "[\"Yes\", \"No\"]",
            "clobTokenIds": "[\"111\", \"222\"]",
            "orderPriceMinTickSize": "0.001",
            "orderMinSize": "5",
            "endDate": "2026-12-31T00:00:00Z"
        });
        let m = ex.parse_market_from_event(&raw, "weather-tomorrow");
        assert_eq!(m.id, "0xabc");
        assert_eq!(m.symbol, "WEATHER_TOMORROW_IT_RAIN");
        assert_eq!(m.base.as_deref(), Some("USDC"));
        assert_eq!(m.market_type, Some(MarketType::Binary));
        assert!(m.expiry.is_some());
    }

    #[test]
    fn parse_order_book_best_level_last() {
        let ex = Polymarket::new(Config::new()).unwrap();
        let raw = json!({
            "bids": [{"price": "0.45", "size": "100"}, {"price": "0.46", "size": "150"}],
            "asks": [{"price": "0.48", "size": "200"}, {"price": "0.47", "size": "50"}]
        });
        let book = ex.parse_order_book(&raw, "X:YES");
        assert_eq!(book.symbol, "X:YES");
        assert_eq!(book.bids.len(), 2);
        assert_eq!(book.asks.len(), 2);
        // ccxt parse_order_book 排序:bids 降序、asks 升序
        assert_eq!(book.bids[0].price, Some("0.46".parse().unwrap()));
        assert_eq!(book.bids[1].price, Some("0.45".parse().unwrap()));
        assert_eq!(book.asks[0].price, Some("0.47".parse().unwrap()));
        assert_eq!(book.asks[1].price, Some("0.48".parse().unwrap()));
    }

    #[test]
    fn parse_ticker_falls_back_to_mid() {
        let ex = Polymarket::new(Config::new()).unwrap();
        let ctx = OutcomeCtx {
            token_id: "111".into(),
            condition_id: "0xabc".into(),
            market_symbol: "X".into(),
            label: "YES".into(),
            quote_volume: None,
        };
        let raw = json!({
            "midpoint": {"mid": "0.50"},
            "book": {
                "timestamp": "1777344471023",
                "bids": [{"price": "0.49", "size": "10"}],
                "asks": [{"price": "0.51", "size": "20"}]
            },
            "lastTrade": {"price": "0"}
        });
        let t = ex.parse_ticker(&raw, &ctx);
        // last-trade 为 0 → 回退 mid
        assert_eq!(t.last, Some("0.50".parse().unwrap()));
        assert_eq!(t.bid, Some("0.49".parse().unwrap()));
        assert_eq!(t.ask, Some("0.51".parse().unwrap()));
        assert_eq!(t.average, Some("0.50".parse().unwrap()));
    }
}
