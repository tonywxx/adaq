//! kalshi 预测市场适配器(M3,ADR-0005)。
//!
//! 对齐 ccxt `prediction/kalshi.py`(v4.5.73)的语义:
//! - symbol = 统一 market handle(shorten_slug 的 event_market 大写形式);
//! - outcome symbol = `{market}:YES|NO`;公共面方法按 outcome symbol 解析;
//! - 订单簿为 YES 视角:`yes_dollars` → bids,`no_dollars` 反转 → asks(NO 视角翻转);
//! - 私密面签名:RSA-PSS SHA-256 over `{timestamp}{METHOD}{/trade-api/v2/path}`,
//!   输出 base64;V2 下单 `POST /portfolio/events/orders`。
//!
//! 参考实现基于 ccxt(MIT)适配器解析逻辑,见 `NOTICE`。

use std::collections::HashMap;

use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::{Value, json};

use crate::error::{Error, ErrorKind, Result};
use crate::exchange::{Config, Exchange, Params};
use crate::httpcore::{HttpCore, dec, iso8601, now_ms, query_string, value_decimal};
use crate::types::{
    Balance, Balances, Level, Market, MarketType, Markets, OHLCV, Order, OrderBook, Position,
    Precision, Ticker, Tickers, Trade,
};

pub const ID: &str = "kalshi";
const BASE_URL: &str = "https://external-api.kalshi.com/trade-api/v2";
const RATE_LIMIT_MS: u64 = 200;

/// 单个 outcome 的解析上下文(由 symbol 解析而来)。
#[derive(Debug, Clone)]
pub struct OutcomeCtx {
    pub market_ticker: String,
    pub label: String,
    pub market_symbol: String,
    pub outcome_id: String,
    pub series_ticker: String,
}

/// kalshi 预测市场适配器。
pub struct Kalshi {
    config: Config,
    core: HttpCore,
    outcomes: crate::adapters::outcome::OutcomeIndex<OutcomeCtx>,
}

impl Kalshi {
    /// 已实现(非 NotSupported)的 REST 方法面(契约声明,M3)。
    pub const IMPLEMENTED: &'static [&'static str] = &[
        "fetch_markets",
        "fetch_ticker",
        "fetch_tickers",
        "fetch_order_book",
        "fetch_trades",
        "fetch_ohlcv",
        "fetch_balance",
        "fetch_positions",
        "fetch_orders",
        "fetch_open_orders",
        "create_order",
        "cancel_order",
    ];

    /// 构造适配器(限速 200ms/次,对齐 kalshi rateLimit)。
    pub fn new(config: Config) -> Result<Self> {
        let core = HttpCore::new(&config, BASE_URL, RATE_LIMIT_MS, "kalshi")?;
        Ok(Self {
            config,
            core,
            outcomes: crate::adapters::outcome::OutcomeIndex::new(),
        })
    }

    // ================= 内部 HTTP =================

    async fn public_get(&self, path: &str, params: &Params) -> Result<Value> {
        self.core.public_get(path, params).await
    }

    /// 私密请求:RSA-PSS SHA-256 签名头(KALSHI-ACCESS-*)。
    async fn private_request(
        &self,
        method: &str,
        path: &str,
        params: &Params,
        body: Option<Value>,
    ) -> Result<Value> {
        let api_key = self
            .config
            .api_key
            .as_deref()
            .ok_or_else(|| Error::new(ErrorKind::Authentication, "kalshi apiKey required"))?;
        let private_key =
            self.config.private_key.as_deref().ok_or_else(|| {
                Error::new(ErrorKind::Authentication, "kalshi privateKey required")
            })?;
        let timestamp = now_ms().to_string();
        // 签名 payload:{timestamp}{METHOD}{/trade-api/v2/path}(不含 query)
        let path_for_signing = format!("/trade-api/v2{path}");
        let payload = format!("{timestamp}{method}{path_for_signing}");
        let signature = crate::signing::sign_rsa_pss(&payload, private_key)?;
        let mut headers = HeaderMap::new();
        headers.insert(
            "KALSHI-ACCESS-KEY",
            HeaderValue::from_str(api_key).map_err(|e| {
                Error::new(ErrorKind::BadRequest, format!("bad api key header: {e}"))
            })?,
        );
        headers.insert(
            "KALSHI-ACCESS-TIMESTAMP",
            HeaderValue::from_str(&timestamp).unwrap(),
        );
        headers.insert(
            "KALSHI-ACCESS-SIGNATURE",
            HeaderValue::from_str(&signature).map_err(|e| {
                Error::new(ErrorKind::BadRequest, format!("bad signature header: {e}"))
            })?,
        );
        // GET 参数走 query,其余作为 JSON body
        let (url, body) = if method == "GET" {
            let url = format!("{BASE_URL}{path}{}", query_string(params));
            (url, None)
        } else {
            let url = format!("{BASE_URL}{path}");
            let body = body.or_else(|| {
                if params.is_empty() {
                    None
                } else {
                    Some(json!(params))
                }
            });
            (url, body)
        };
        self.core.request_url(method, &url, &headers, body).await
    }

    // ================= outcomes 索引 =================

    /// 确保 markets/outcomes 已加载(缓存)。
    pub(crate) async fn load_markets(&self) -> Result<()> {
        self.core.load_markets(|| self.fetch_markets_raw()).await
    }

    /// 拉取并解析市集 + outcomes 索引(字段映射接缝;缓存由核心 `HttpCore::load_markets` 负责)。
    async fn fetch_markets_raw(&self) -> Result<Markets> {
        let mut p = Params::new();
        p.insert("limit".into(), json!(1000));
        let resp = self.public_get("/markets", &p).await?;
        let arr = resp
            .get("markets")
            .and_then(Value::as_array)
            .ok_or_else(|| Error::new(ErrorKind::BadResponse, "markets not array"))?;
        let mut market_map = HashMap::with_capacity(arr.len());
        let mut outcomes = HashMap::new();
        for raw in arr {
            let market = self.parse_market(raw);
            let ticker = raw["ticker"].as_str().unwrap_or_default().to_string();
            let event_ticker = raw["event_ticker"].as_str().unwrap_or_default().to_string();
            // 推导 series_ticker:event_ticker 去掉最后一段
            let series = match event_ticker.rsplit_once('-') {
                Some((head, _)) => head.to_string(),
                None => event_ticker.clone(),
            };
            // YES outcome
            outcomes.insert(
                format!("{}:YES", market.symbol),
                OutcomeCtx {
                    market_ticker: ticker.clone(),
                    label: "YES".into(),
                    market_symbol: market.symbol.clone(),
                    outcome_id: ticker.clone(),
                    series_ticker: series.clone(),
                },
            );
            // NO outcome
            outcomes.insert(
                format!("{}:NO", market.symbol),
                OutcomeCtx {
                    market_ticker: ticker.clone(),
                    label: "NO".into(),
                    market_symbol: market.symbol.clone(),
                    outcome_id: format!("{}-NO", ticker),
                    series_ticker: series.clone(),
                },
            );
            // 也允许 outcomeId(裸 ticker)直接解析
            if let Some(t) = raw["ticker"].as_str() {
                outcomes.insert(
                    t.to_string(),
                    OutcomeCtx {
                        market_ticker: t.to_string(),
                        label: "YES".into(),
                        market_symbol: market.symbol.clone(),
                        outcome_id: t.to_string(),
                        series_ticker: series.clone(),
                    },
                );
            }
            market_map.insert(market.symbol.clone(), market);
        }
        self.outcomes.store(outcomes);
        Ok(market_map)
    }

    /// 解析统一 symbol → outcome 上下文。支持:
    /// - outcome symbol(`MARKET:YES` / `MARKET:NO`)
    /// - outcomeId(裸 market ticker,默认 YES)
    /// - market symbol(默认 YES,经 `resolve_or` 的 `{symbol}:YES` 兜底)
    pub(crate) fn resolve_outcome(&self, symbol: &str) -> Result<OutcomeCtx> {
        self.outcomes
            .resolve_or(symbol, |s| format!("{s}:YES"), "kalshi")
    }

    // ================= parse(公开,供差分测试) =================

    /// 解析单个 kalshi market 对象为统一 Market(对齐 ccxt parse_market)。
    pub fn parse_market(&self, raw: &Value) -> Market {
        let ticker = raw["ticker"].as_str().unwrap_or_default();
        let event_ticker = raw["event_ticker"].as_str().unwrap_or_default();
        let subtitle = raw
            .get("subtitle")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .or_else(|| raw["title"].as_str());
        let subtitle = subtitle.unwrap_or(ticker);
        let status = raw["status"].as_str().unwrap_or_default();
        let active = status == "active" || status == "open";
        let end_date = raw["expiration_time"].as_str();
        let market_symbol = slug_to_market_symbol(event_ticker, subtitle);
        // 价格精度:price_ranges[0].step(dollars),否则 tick_size/100
        let step = raw["price_ranges"]
            .get(0)
            .and_then(|r| r.get("step"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let price_precision = match &step {
            Some(s) => s.clone(),
            None => {
                let tick = raw["tick_size"].as_str().unwrap_or("1");
                crate::precise::string_div(tick, "100")
            }
        };
        let expiry = end_date.and_then(parse_iso_ms);
        Market {
            id: ticker.to_string(),
            symbol: market_symbol.clone(),
            base: Some("USD".into()),
            quote: Some("USD".into()),
            base_id: Some(ticker.to_string()),
            quote_id: Some("USD".into()),
            active: Some(active),
            market_type: Some(MarketType::Prediction),
            spot: Some(false),
            margin: Some(false),
            swap: Some(false),
            future: Some(false),
            option: Some(false),
            taker: Some("0.07".parse().unwrap_or_default()),
            maker: Some("0".parse().unwrap_or_default()),
            expiry,
            expiry_datetime: end_date.map(str::to_string),
            precision: Precision {
                price: Some(price_precision.parse().unwrap_or_default()),
                amount: Some(1u8.into()),
                cost: None,
            },
            limits: crate::types::Limits::default(),
            info: raw.clone(),
            ..Market::default()
        }
    }

    /// 解析 ticker(对齐 ccxt parse_prediction_ticker)。
    pub fn parse_ticker(&self, raw: &Value, ctx: &OutcomeCtx) -> Ticker {
        let is_no = ctx.label == "NO";
        let last = dec(raw.get("last_price_dollars")).filter(|v| !v.is_zero());
        let bid = if is_no {
            dec(raw.get("no_bid_dollars"))
        } else {
            dec(raw.get("yes_bid_dollars"))
        }
        .filter(|v| !v.is_zero());
        let ask = if is_no {
            dec(raw.get("no_ask_dollars"))
        } else {
            dec(raw.get("yes_ask_dollars"))
        }
        .filter(|v| !v.is_zero());
        let close = if is_no {
            last.map(crate::precise::one_minus_decimal)
                .filter(|v| !v.is_zero())
        } else {
            last
        };
        // 书是 YES 报价,NO 侧大小互换
        let bid_size = if is_no {
            dec(raw.get("yes_ask_size_fp"))
        } else {
            dec(raw.get("yes_bid_size_fp"))
        };
        let ask_size = if is_no {
            dec(raw.get("yes_bid_size_fp"))
        } else {
            dec(raw.get("yes_ask_size_fp"))
        };
        let now = now_ms();
        let average = match (bid, ask) {
            (Some(b), Some(a)) => Some((b + a) / rust_decimal::Decimal::from(2)),
            _ => None,
        }
        .filter(|v| !v.is_zero());
        Ticker {
            symbol: format!("{}:{}", ctx.market_symbol, ctx.label),
            timestamp: Some(now),
            datetime: iso8601(now),
            bid,
            ask,
            bid_volume: bid_size.filter(|v| !v.is_sign_negative()),
            ask_volume: ask_size.filter(|v| !v.is_sign_negative()),
            close,
            last: close,
            average,
            base_volume: raw
                .get("volume_24h_fp")
                .or_else(|| raw.get("volume_24h"))
                .or_else(|| raw.get("volume"))
                .and_then(value_decimal),
            info: raw.clone(),
            ..Ticker::default()
        }
    }

    /// 解析订单簿(YES 视角;NO outcome 翻转)。
    pub fn parse_order_book(&self, raw: &Value, ctx: &OutcomeCtx) -> OrderBook {
        let book = raw.get("orderbook_fp").unwrap_or(raw);
        let raw_yes = book
            .get("yes_dollars")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let raw_no = book
            .get("no_dollars")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let is_no = ctx.label == "NO";
        let (mut bids, mut asks): (Vec<Level>, Vec<Level>) = if is_no {
            (
                raw_no.iter().map(parse_kalshi_level).collect(),
                raw_yes.iter().map(invert_level).collect(),
            )
        } else {
            (
                raw_yes.iter().map(parse_kalshi_level).collect(),
                raw_no.iter().map(invert_level).collect(),
            )
        };
        bids.sort_by_key(|l| std::cmp::Reverse(l.price));
        asks.sort_by_key(|l| l.price);
        OrderBook {
            symbol: format!("{}:{}", ctx.market_symbol, ctx.label),
            bids,
            asks,
            timestamp: Some(now_ms()),
            nonce: None,
            info: raw.clone(),
            ..OrderBook::default()
        }
    }

    /// 解析成交(对齐 ccxt parse_prediction_trade)。
    pub fn parse_trade(&self, raw: &Value, ctx: &OutcomeCtx) -> Trade {
        let id = raw["trade_id"].as_str().map(str::to_string);
        let ts = raw
            .get("created_time")
            .and_then(Value::as_str)
            .and_then(parse_iso_ms);
        let price = raw
            .get("yes_price_dollars")
            .or_else(|| raw.get("price_dollars"))
            .and_then(value_decimal)
            .or_else(|| {
                raw.get("yes_price")
                    .or_else(|| raw.get("price"))
                    .and_then(value_decimal)
                    .map(|c| c / rust_decimal::Decimal::from(100))
            });
        let amount = raw
            .get("count_fp")
            .or_else(|| raw.get("size_fp"))
            .and_then(value_decimal)
            .or_else(|| raw.get("count").and_then(value_decimal));
        let raw_side = raw["taker_side"]
            .as_str()
            .unwrap_or_default()
            .to_lowercase();
        let requested_label = ctx.label.to_lowercase();
        let side = if raw_side == "yes" || raw_side == "no" {
            if requested_label == "yes" || requested_label == "no" {
                if raw_side == requested_label {
                    Some("buy".into())
                } else {
                    Some("sell".into())
                }
            } else if raw_side == "yes" {
                Some("buy".into())
            } else {
                Some("sell".into())
            }
        } else {
            None
        };
        let cost = match (price, amount) {
            (Some(p), Some(a)) => Some(p * a),
            _ => None,
        };
        Trade {
            id,
            timestamp: ts,
            datetime: ts.and_then(iso8601),
            symbol: Some(ctx.market_symbol.clone()),
            side,
            price,
            amount,
            cost,
            taker_or_maker: Some("taker".into()),
            info: raw.clone(),
            ..Trade::default()
        }
    }

    /// 解析 K 线(对齐 ccxt parse_ohlcv:end_period_ts - duration)。
    pub fn parse_ohlcv(&self, row: &Value, timeframe_secs: i64) -> OHLCV {
        let price = row.get("price").unwrap_or(&Value::Null);
        let previous = dec(price.get("previous_dollars"));
        let end_ts = row["end_period_ts"].as_i64();
        let open = dec(price.get("open_dollars")).or(previous);
        let high = dec(price.get("high_dollars")).or(previous);
        let low = dec(price.get("low_dollars")).or(previous);
        let close = dec(price.get("close_dollars")).or(previous);
        OHLCV {
            timestamp: end_ts.map(|t| (t - timeframe_secs) * 1000),
            open,
            high,
            low,
            close,
            volume: row
                .get("volume_fp")
                .and_then(value_decimal)
                .or_else(|| Some(0u8.into())),
        }
    }

    /// 解析持仓(参考实现:market_positions → Position)。
    pub fn parse_position(&self, raw: &Value) -> Position {
        let market_ticker = raw["market_ticker"].as_str().unwrap_or_default();
        let contracts = raw
            .get("position")
            .or_else(|| raw.get("yes_position"))
            .or_else(|| raw.get("no_position"))
            .and_then(value_decimal);
        let cost = raw
            .get("total_cost")
            .or_else(|| raw.get("realized_pnl"))
            .and_then(value_decimal);
        Position {
            symbol: Some(market_ticker.to_string()),
            id: raw["market_id"].as_str().map(str::to_string),
            contracts,
            notional: cost,
            info: raw.clone(),
            ..Position::default()
        }
    }

    /// 解析订单(参考实现:portfolio orders → Order)。
    pub fn parse_order(&self, raw: &Value) -> Order {
        let status = raw
            .get("status")
            .or_else(|| raw.get("order_status"))
            .and_then(Value::as_str)
            .map(|s| match s {
                "resting" => "open",
                "filled" => "closed",
                "cancelled" | "canceled" => "canceled",
                other => other,
            })
            .map(str::to_string);
        let side = raw
            .get("action")
            .and_then(Value::as_str)
            .map(|s| if s == "bid" { "buy" } else { "sell" })
            .map(str::to_string)
            .or_else(|| raw["side"].as_str().map(str::to_string));
        let ts = raw
            .get("created_time")
            .or_else(|| raw.get("updated_time"))
            .and_then(Value::as_str)
            .and_then(parse_iso_ms);
        Order {
            id: raw
                .get("order_id")
                .or_else(|| raw.get("id"))
                .and_then(Value::as_str)
                .map(str::to_string),
            timestamp: ts,
            datetime: ts.and_then(iso8601),
            status,
            symbol: raw["ticker"].as_str().map(str::to_string),
            order_type: Some("limit".into()),
            side,
            price: raw.get("price").and_then(value_decimal),
            amount: raw.get("count").and_then(value_decimal),
            filled: raw.get("fill_count").and_then(value_decimal),
            remaining: raw.get("remaining_count").and_then(value_decimal),
            info: raw.clone(),
            ..Order::default()
        }
    }
}

// ================= Exchange trait 实现 =================

impl Exchange for Kalshi {
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
        let resp = self
            .public_get(&format!("/markets/{}", ctx.market_ticker), &Params::new())
            .await?;
        let raw = resp.get("market").unwrap_or(&resp);
        Ok(self.parse_ticker(raw, &ctx))
    }

    async fn fetch_tickers(&self, symbols: Option<&[&str]>, _params: Params) -> Result<Tickers> {
        self.load_markets().await?;
        let symbols = symbols.ok_or_else(|| {
            Error::new(
                ErrorKind::BadRequest,
                "kalshi fetch_tickers 需要 outcomes 参数(无全量 ticker 端点)",
            )
        })?;
        // 按 market ticker 分组(symbols 中重复的 market 只请求一次)
        let mut tickers: Vec<String> = Vec::new();
        let mut by_ticker: HashMap<String, Vec<OutcomeCtx>> = HashMap::new();
        for s in symbols {
            let ctx = self.resolve_outcome(s)?;
            let t = ctx.market_ticker.clone();
            if !by_ticker.contains_key(&t) {
                tickers.push(t.clone());
            }
            by_ticker.entry(t).or_default().push(ctx);
        }
        let mut out = Tickers::new();
        for chunk in tickers.chunks(100) {
            let mut p = Params::new();
            p.insert("tickers".into(), json!(chunk.join(",")));
            p.insert("limit".into(), json!(chunk.len()));
            let resp = self.public_get("/markets", &p).await?;
            let arr = resp
                .get("markets")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for raw in arr {
                let mt = raw["ticker"].as_str().unwrap_or_default().to_string();
                if let Some(group) = by_ticker.get(&mt) {
                    for ctx in group {
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
        let resp = self
            .public_get(
                &format!("/markets/{}/orderbook", ctx.market_ticker),
                &Params::new(),
            )
            .await?;
        Ok(self.parse_order_book(&resp, &ctx))
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
        p.insert("ticker".into(), json!(ctx.market_ticker));
        if let Some(l) = limit {
            p.insert("limit".into(), json!(l));
        }
        let resp = self.public_get("/markets/trades", &p).await?;
        let arr = resp
            .get("trades")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(arr.iter().map(|t| self.parse_trade(t, &ctx)).collect())
    }

    async fn fetch_ohlcv(
        &self,
        symbol: &str,
        timeframe: &str,
        since: Option<i64>,
        limit: Option<i64>,
        _params: Params,
    ) -> Result<Vec<OHLCV>> {
        self.load_markets().await?;
        let ctx = self.resolve_outcome(symbol)?;
        let period_min = match timeframe {
            "1m" => 1,
            "1h" => 60,
            "1d" => 1440,
            _ => {
                return Err(Error::new(
                    ErrorKind::BadRequest,
                    format!("kalshi 不支持 timeframe {timeframe}(支持 1m/1h/1d)"),
                ));
            }
        };
        let mut p = Params::new();
        p.insert("series_ticker".into(), json!(ctx.series_ticker));
        p.insert("ticker".into(), json!(ctx.market_ticker));
        p.insert("period_interval".into(), json!(period_min));
        let now = now_ms() / 1000;
        let tf = period_min * 60;
        let count = limit.unwrap_or(200);
        let end = since
            .map(|s| (s / 1000 + count * tf).min(now))
            .unwrap_or(now);
        let start = since.map(|s| s / 1000).unwrap_or(now - count * tf);
        p.insert("start_ts".into(), json!(start));
        p.insert("end_ts".into(), json!(end));
        let resp = self
            .public_get(
                &format!(
                    "/series/{}/markets/{}/candlesticks",
                    ctx.series_ticker, ctx.market_ticker
                ),
                &p,
            )
            .await?;
        let arr = resp
            .get("candlesticks")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(arr
            .iter()
            .filter(|c| {
                let price = c.get("price").cloned().unwrap_or(Value::Null);
                price.get("open_dollars").is_some() || price.get("previous_dollars").is_some()
            })
            .map(|c| self.parse_ohlcv(c, tf))
            .collect())
    }

    async fn fetch_balance(&self, _params: Params) -> Result<Balances> {
        let resp = self
            .private_request("GET", "/portfolio/balance", &Params::new(), None)
            .await?;
        // Kalshi balance 以美分计 → /100
        let total = resp
            .get("balance")
            .and_then(value_decimal)
            .map(|c| c / rust_decimal::Decimal::from(100));
        let mut out = Balances {
            info: resp,
            ..Balances::default()
        };
        out.accounts.insert(
            "USD".into(),
            Balance {
                free: total,
                used: Some(0u8.into()),
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
        let resp = self
            .private_request("GET", "/portfolio/positions", &Params::new(), None)
            .await?;
        let arr = resp
            .get("market_positions")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(arr.iter().map(|p| self.parse_position(p)).collect())
    }

    async fn fetch_orders(
        &self,
        symbol: Option<&str>,
        _since: Option<i64>,
        _limit: Option<i64>,
        _params: Params,
    ) -> Result<Vec<Order>> {
        let mut p = Params::new();
        p.insert("status".into(), json!("resting,filled,cancelled"));
        if let Some(s) = symbol {
            // symbol 是统一 symbol;尽力解析为 ticker,失败则忽略过滤
            if let Ok(ctx) = self.resolve_outcome(s) {
                p.insert("ticker".into(), json!(ctx.market_ticker));
            }
        }
        let resp = self
            .private_request("GET", "/portfolio/orders", &p, None)
            .await?;
        let arr = resp
            .get("orders")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(arr.iter().map(|o| self.parse_order(o)).collect())
    }

    async fn fetch_open_orders(
        &self,
        symbol: Option<&str>,
        _since: Option<i64>,
        _limit: Option<i64>,
        _params: Params,
    ) -> Result<Vec<Order>> {
        let mut p = Params::new();
        p.insert("status".into(), json!("resting"));
        if let Some(s) = symbol {
            if let Ok(ctx) = self.resolve_outcome(s) {
                p.insert("ticker".into(), json!(ctx.market_ticker));
            }
        }
        let resp = self
            .private_request("GET", "/portfolio/orders", &p, None)
            .await?;
        let arr = resp
            .get("orders")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(arr.iter().map(|o| self.parse_order(o)).collect())
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
        let price = price.ok_or_else(|| {
            Error::new(
                ErrorKind::BadRequest,
                "kalshi 只有限价单,create_order 必须提供 price",
            )
        })?;
        // kalshi V2 只报 YES 腿:buy NO @ q == sell YES @ 1-q
        let is_no = ctx.label == "NO";
        let is_buy = side == "buy";
        let mut book_side = if is_buy { "bid" } else { "ask" };
        let yes_price = if is_no {
            book_side = if is_buy { "ask" } else { "bid" };
            crate::precise::string_sub("1", price)
        } else {
            price.to_string()
        };
        let tif = if order_type == "market" {
            "immediate_or_cancel"
        } else {
            "good_till_canceled"
        };
        let body = json!({
            "ticker": ctx.market_ticker,
            "side": book_side,
            "count": amount,
            "price": yes_price,
            "time_in_force": tif,
            "self_trade_prevention_type": "taker_at_cross",
        });
        let resp = self
            .private_request(
                "POST",
                "/portfolio/events/orders",
                &Params::new(),
                Some(body),
            )
            .await?;
        let mut order = self.parse_order(&resp);
        order.side = Some(side.to_string());
        order.amount = amount.parse().ok();
        order.price = price.parse().ok();
        order.order_type = Some(order_type.to_string());
        // 回填 filled/remaining/status(V2 响应只有 order_id/fill_count/remaining_count)
        let remaining = resp.get("remaining_count").and_then(value_decimal);
        let filled = resp.get("fill_count").and_then(value_decimal);
        if let Some(f) = filled {
            order.filled = Some(f);
        } else if let (Some(r), Some(a)) = (remaining, amount.parse::<rust_decimal::Decimal>().ok())
        {
            order.filled = Some(a - r);
        }
        if let Some(r) = remaining {
            order.remaining = Some(r);
        }
        if order.status.is_none() {
            order.status = Some(
                if remaining == Some(0u8.into()) {
                    "closed"
                } else {
                    "open"
                }
                .into(),
            );
        }
        Ok(order)
    }

    async fn cancel_order(&self, id: &str, _symbol: &str, _params: Params) -> Result<Order> {
        let resp = self
            .private_request(
                "DELETE",
                &format!("/portfolio/events/orders/{id}"),
                &Params::new(),
                None,
            )
            .await?;
        let mut order = self.parse_order(&resp);
        if order.id.is_none() {
            order.id = Some(id.to_string());
        }
        order.status = Some("canceled".into());
        Ok(order)
    }
}

impl Kalshi {
    /// 扩展面(不在统一 trait):结算记录。
    pub async fn fetch_settlements(&self, _limit: Option<i64>) -> Result<Vec<Value>> {
        let mut p = Params::new();
        if let Some(l) = _limit {
            p.insert("limit".into(), json!(l));
        }
        let resp = self
            .private_request("GET", "/portfolio/settlements", &p, None)
            .await?;
        let arr = resp
            .get("settlements")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(arr)
    }
}

// ================= 静态助手 =================

/// ccxt `shorten_slug`:小写 → 字母数字保留、其余转 `-` → 替换表 → 去停用词 → `_` join → 大写。
pub fn shorten_slug(slug: &str) -> String {
    const REPLACEMENTS: &[(&str, &str)] = &[
        ("federal-reserve", "fed"),
        ("interest-rates", "rates"),
        ("interest-rate", "rate"),
        ("basis-points", "bps"),
        ("basis-point", "bp"),
        ("executive-order", "eo"),
        ("united-states", "us"),
        ("united-kingdom", "uk"),
        ("european-union", "eu"),
        ("artificial-intelligence", "ai"),
        ("republican-party", "gop"),
        ("democratic-party", "dems"),
        ("stock-market", "market"),
        ("price-target", "pt"),
        ("market-cap", "mcap"),
        ("increase", "hike"),
        ("decrease", "cut"),
        ("higher", "up"),
        ("lower", "down"),
        ("greater", "gt"),
        ("less", "lt"),
        ("million", "M"),
        ("billion", "B"),
        ("trillion", "T"),
        ("percent", "pct"),
    ];
    const STOP_WORDS: &[&str] = &[
        "will", "the", "a", "an", "after", "before", "in", "at", "by", "of", "there", "be", "to",
        "or", "and", "for", "on", "its", "that", "this", "from", "with", "as", "is", "are", "was",
        "were", "how", "many", "who", "what", "when", "where", "which", "much",
    ];
    let lower = slug.to_lowercase();
    let mut s = String::new();
    let mut last_dash = true; // 起始 true:丢前导分隔符
    for ch in lower.chars() {
        if ch.is_ascii_alphanumeric() {
            s.push(ch);
            last_dash = false;
        } else if !last_dash {
            s.push('-');
            last_dash = true;
        }
    }
    for (from, to) in REPLACEMENTS {
        if s.contains(from) {
            s = s.replace(from, to);
        }
    }
    let parts: Vec<&str> = s
        .split('-')
        .filter(|w| !w.is_empty() && !STOP_WORDS.contains(w))
        .collect();
    parts.join("_").to_uppercase()
}

/// ccxt `slug_to_market_symbol`:event 与 market 前缀合并(相同/为空时省略 event)。
pub fn slug_to_market_symbol(event_slug: &str, market_slug: &str) -> String {
    let market_part = shorten_slug(market_slug);
    let event_part = shorten_slug(event_slug);
    if event_part.is_empty() || event_part == market_part {
        market_part
    } else {
        format!("{event_part}_{market_part}")
    }
}

/// ccxt `slug_to_outcome_symbol`:`{market}:{label}`(label 只留大写字母数字)。
pub fn slug_to_outcome_symbol(event_slug: &str, market_slug: &str, outcome: &str) -> String {
    let upper = outcome.to_uppercase();
    let mut label = String::new();
    let mut pending_sep = false;
    for ch in upper.chars() {
        if ch.is_ascii_alphanumeric() {
            if pending_sep && !label.is_empty() {
                label.push('_');
            }
            label.push(ch);
            pending_sep = false;
        } else {
            pending_sep = true;
        }
    }
    if label.is_empty() {
        label = upper;
    }
    format!("{}:{label}", slug_to_market_symbol(event_slug, market_slug))
}

/// `[price, size]`(字符串)→ Level。
fn parse_kalshi_level(raw: &Value) -> Level {
    let arr = raw.as_array();
    let price = arr.and_then(|a| a.first()).and_then(value_decimal);
    let amount = arr.and_then(|a| a.get(1)).and_then(value_decimal);
    Level { price, amount }
}

/// 反转一个 YES 侧 level:`price → 1 - price`(用于 NO 侧报价)。
fn invert_level(raw: &Value) -> Level {
    let arr = raw.as_array();
    let price = arr
        .and_then(|a| a.first())
        .and_then(value_decimal)
        .map(crate::precise::one_minus_decimal);
    let amount = arr.and_then(|a| a.get(1)).and_then(value_decimal);
    Level { price, amount }
}

/// ISO8601 → 毫秒时间戳。
pub fn parse_iso_ms(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.timestamp_millis())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shorten_slug_basic() {
        assert_eq!(
            shorten_slug("Will Trump bring back manufacturing?"),
            "TRUMP_BRING_BACK_MANUFACTURING"
        );
        assert_eq!(shorten_slug("During Trump's term"), "DURING_TRUMP_S_TERM");
        assert_eq!(shorten_slug(""), "");
        assert_eq!(shorten_slug("interest rates"), "RATES");
    }

    #[test]
    fn slug_market_symbol_qualifies_event() {
        // event 与 market 相同 → 只保留 market
        assert_eq!(
            slug_to_market_symbol("KXGDPSHAREMANU-29", "KXGDPSHAREMANU-29"),
            "KXGDPSHAREMANU_29"
        );
        // 不同 → event_market
        assert_eq!(
            slug_to_market_symbol("KXGDPSHAREMANU-29", "During Trump's term"),
            "KXGDPSHAREMANU_29_DURING_TRUMP_S_TERM"
        );
    }

    #[test]
    fn outcome_symbol_has_label_suffix() {
        // "is" 是 stop word,会被去掉
        assert_eq!(
            slug_to_outcome_symbol("EVENT-1", "Is it raining?", "YES"),
            "EVENT_1_IT_RAINING:YES"
        );
    }

    #[test]
    fn parse_market_basic() {
        let ex = Kalshi::new(Config::new()).unwrap();
        let raw = json!({
            "ticker": "KXABC-1",
            "event_ticker": "KXABC",
            "subtitle": "",
            "title": "Will it happen?",
            "status": "active",
            "result": "",
            "expiration_time": "2026-08-01T14:00:00Z",
            "price_ranges": [{"start": "0", "end": "1", "step": "0.0100"}],
            "tick_size": "1",
        });
        let m = ex.parse_market(&raw);
        assert_eq!(m.id, "KXABC-1");
        // "will" 是 stop word 被去掉
        assert_eq!(m.symbol, "KXABC_IT_HAPPEN");
        assert_eq!(m.base.as_deref(), Some("USD"));
        assert_eq!(m.market_type, Some(MarketType::Prediction));
        assert_eq!(m.precision.price, Some("0.0100".parse().unwrap()));
        assert!(m.expiry.is_some());
    }

    #[test]
    fn parse_order_book_yes_perspective() {
        let ex = Kalshi::new(Config::new()).unwrap();
        let ctx = OutcomeCtx {
            market_ticker: "KXABC-1".into(),
            label: "YES".into(),
            market_symbol: "KXABC_X".into(),
            outcome_id: "KXABC-1".into(),
            series_ticker: "KXABC".into(),
        };
        let raw = json!({
            "orderbook_fp": {
                "yes_dollars": [["0.15", "100"], ["0.16", "101"]],
                "no_dollars": [["0.80", "50"], ["0.79", "60"]]
            }
        });
        let book = ex.parse_order_book(&raw, &ctx);
        // bids 降序
        assert!(book.bids[0].price > book.bids[1].price);
        // asks = 1 - no
        assert_eq!(book.asks[0].price, Some("0.20".parse().unwrap()));
        assert_eq!(book.asks[1].price, Some("0.21".parse().unwrap()));
        assert_eq!(book.bids[0].amount, Some("101".parse().unwrap()));
    }

    #[test]
    fn parse_order_book_no_perspective() {
        let ex = Kalshi::new(Config::new()).unwrap();
        let ctx = OutcomeCtx {
            market_ticker: "KXABC-1".into(),
            label: "NO".into(),
            market_symbol: "KXABC_X".into(),
            outcome_id: "KXABC-1-NO".into(),
            series_ticker: "KXABC".into(),
        };
        let raw = json!({
            "orderbook_fp": {
                "yes_dollars": [["0.15", "100"]],
                "no_dollars": [["0.80", "50"]]
            }
        });
        let book = ex.parse_order_book(&raw, &ctx);
        // NO 视角:bids = no_dollars
        assert_eq!(book.bids[0].price, Some("0.80".parse().unwrap()));
        // asks = 1 - yes
        assert_eq!(book.asks[0].price, Some("0.85".parse().unwrap()));
    }

    #[test]
    fn parse_ticker_no_outcome_inverts_close() {
        let ex = Kalshi::new(Config::new()).unwrap();
        let ctx = OutcomeCtx {
            market_ticker: "KXABC-1".into(),
            label: "NO".into(),
            market_symbol: "KXABC_X".into(),
            outcome_id: "KXABC-1-NO".into(),
            series_ticker: "KXABC".into(),
        };
        let raw = json!({
            "last_price_dollars": "0.20",
            "yes_bid_dollars": "0.15",
            "yes_ask_dollars": "0.25",
            "no_bid_dollars": "0.75",
            "no_ask_dollars": "0.85",
            "volume_24h_fp": "100.00"
        });
        let t = ex.parse_ticker(&raw, &ctx);
        assert_eq!(t.bid, Some("0.75".parse().unwrap()));
        assert_eq!(t.ask, Some("0.85".parse().unwrap()));
        // NO close = 1 - last
        assert_eq!(t.close, Some("0.80".parse().unwrap()));
        assert_eq!(t.base_volume, Some("100".parse().unwrap()));
    }

    #[test]
    fn parse_ohlcv_stamps_open_time() {
        let ex = Kalshi::new(Config::new()).unwrap();
        let raw = json!({
            "end_period_ts": 1776109260,
            "price": {"open_dollars": "0.56", "high_dollars": "0.58", "low_dollars": "0.55", "close_dollars": "0.57"},
            "volume_fp": "12.00"
        });
        let c = ex.parse_ohlcv(&raw, 60);
        assert_eq!(c.timestamp, Some((1776109260 - 60) * 1000));
        assert_eq!(c.open, Some("0.56".parse().unwrap()));
        assert_eq!(c.volume, Some("12".parse().unwrap()));
    }

    #[test]
    fn parse_trade_side_from_taker_side() {
        let ex = Kalshi::new(Config::new()).unwrap();
        let ctx = OutcomeCtx {
            market_ticker: "KXABC-1".into(),
            label: "YES".into(),
            market_symbol: "KXABC_X".into(),
            outcome_id: "KXABC-1".into(),
            series_ticker: "KXABC".into(),
        };
        let raw = json!({
            "trade_id": "t1",
            "created_time": "2026-01-01T00:00:00Z",
            "yes_price_dollars": "0.50",
            "count": "10",
            "taker_side": "yes"
        });
        let t = ex.parse_trade(&raw, &ctx);
        assert_eq!(t.id.as_deref(), Some("t1"));
        assert_eq!(t.side.as_deref(), Some("buy"));
        assert_eq!(t.price, Some("0.50".parse().unwrap()));
        assert_eq!(t.cost, Some("5".parse().unwrap()));
    }
}
