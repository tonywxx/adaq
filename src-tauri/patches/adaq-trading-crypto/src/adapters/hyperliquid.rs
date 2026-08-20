//! hyperliquid 适配器(DEX,ADR-0005 / 0016)。
//!
//! hyperliquid 的 REST 面是一组 `info` RPC(`POST /info`,JSON body
//! `{"type": ...}`),与 ccxt 标准端点树不同,故无法由 `generic.rs` 描述驱动
//! 引擎批量补齐 —— 本适配器按 ccxt 4.5.73 `hyperliquid.py` 的解析逻辑手写,
//! 字段对齐 ccxt 统一结构(保留 `info` 原始负载)。
//!
//! 已实现(公共行情,DEX 接入核心面):
//! - `fetch_markets`(swap 永续 + spot 现货,经 `metaAndAssetCtxs` /
//!   `spotMetaAndAssetCtxs`);
//! - `fetch_tickers`(midPx 即最新价,dayNtlVlm 即 24h 报价量);
//! - `fetch_ohlcv`(candleSnapshot);
//! - `fetch_order_book`(l2Book)。
//!
//! 未实现(保持 `NotSupported`,与 ccxt 外/私有面一致):
//! - 下单 / 撤单 / 持仓 / 余额等需 L1 action 签名(EIP-712)+ agent 钱包,
//!   属独立大工作量,留作后续 curated 扩展,不在自动转译范围。
//!
//! 注:ccxt 同源解析逻辑保留 MIT 声明(见仓库 `NOTICE`)。

use chrono::Utc;
use serde_json::{Value, json};

use crate::error::{Error, ErrorKind, Result};
use crate::exchange::{Config, Exchange, Params};
use crate::httpcore::{HttpCore, apply_client_limit, iso8601, value_decimal};
use crate::types::{Market, MarketType, Markets, OHLCV, OrderBook, Precision, Ticker, Tickers};

pub const ID: &str = "hyperliquid";
const BASE_URL: &str = "https://api.hyperliquid.xyz";
const RATE_LIMIT_MS: u64 = 200;

/// hyperliquid 适配器。
pub struct Hyperliquid {
    config: Config,
    core: HttpCore,
}

impl Hyperliquid {
    /// 已实现(非 NotSupported)的 REST 方法面(契约声明)。
    pub const IMPLEMENTED: &'static [&'static str] = &[
        "fetch_markets",
        "fetch_tickers",
        "fetch_ohlcv",
        "fetch_order_book",
    ];

    pub fn new(config: Config) -> Result<Self> {
        let core = HttpCore::new(&config, BASE_URL, RATE_LIMIT_MS, "hyperliquid")?;
        Ok(Self { config, core })
    }

    /// 单一 `info` RPC:`POST /info`,body 即 `{"type": ..., ...}`。
    async fn info(&self, body: Value) -> Result<Value> {
        self.core
            .request(
                "POST",
                "/info",
                &reqwest::header::HeaderMap::new(),
                Some(body),
            )
            .await
    }

    async fn load_markets(&self) -> Result<()> {
        self.core.load_markets(|| self.fetch_markets_raw()).await
    }

    async fn fetch_markets_raw(&self) -> Result<Markets> {
        let mut map = Markets::new();
        // swap 永续
        let swap_resp = self.info(json!({"type": "metaAndAssetCtxs"})).await?;
        if let Some(arr) = swap_resp.as_array() {
            let universe = arr
                .first()
                .and_then(|m| m.get("universe"))
                .and_then(|u| u.as_array())
                .cloned()
                .unwrap_or_default();
            let ctxs = arr
                .get(1)
                .and_then(|c| c.as_array())
                .cloned()
                .unwrap_or_default();
            for (i, u) in universe.iter().enumerate() {
                let mut data = u.clone();
                if let (Some(Value::Object(m)), Value::Object(d)) = (ctxs.get(i), &mut data) {
                    for (k, v) in m {
                        d.insert(k.clone(), v.clone());
                    }
                }
                data["baseId"] = json!(i);
                data["swap"] = json!(true);
                let m = parse_swap_market(&data, i);
                map.insert(m.symbol.clone(), m);
            }
        }
        // spot 现货
        let spot_resp = self.info(json!({"type": "spotMetaAndAssetCtxs"})).await?;
        if let Some(arr) = spot_resp.as_array() {
            let first = arr.first().cloned().unwrap_or(Value::Null);
            let tokens = first
                .get("tokens")
                .and_then(|t| t.as_array())
                .cloned()
                .unwrap_or_default();
            let universe = first
                .get("universe")
                .and_then(|u| u.as_array())
                .cloned()
                .unwrap_or_default();
            let ctxs = arr
                .get(1)
                .and_then(|c| c.as_array())
                .cloned()
                .unwrap_or_default();
            for (i, u) in universe.iter().enumerate() {
                let extra = ctxs.get(i).cloned().unwrap_or(Value::Null);
                if let Some(m) = parse_spot_market(u, &extra, &tokens) {
                    map.insert(m.symbol.clone(), m);
                }
            }
        }
        Ok(map)
    }

    /// 从市场缓存取 `coin`(swap 用 baseName,spot 用 id)。
    fn coin_of(&self, market: &Market) -> String {
        let is_swap = market.swap.unwrap_or(false);
        if is_swap {
            market
                .info
                .get("baseName")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| market.base.clone().unwrap_or_default())
        } else {
            market.id.clone()
        }
    }
}

/// 永续(swap)市场:symbol = `BASE/USDC:USDC`,结算币 USDC。
fn parse_swap_market(data: &Value, idx: usize) -> Market {
    let base_name = data["name"].as_str().unwrap_or_default().to_string();
    let symbol = format!("{base_name}/USDC:USDC");
    let sz = data["szDecimals"].as_i64();
    let mut info = data.clone();
    info["baseName"] = json!(base_name);
    Market {
        id: idx.to_string(),
        symbol: symbol.clone(),
        base: Some(base_name.clone()),
        quote: Some("USDC".into()),
        base_id: Some(idx.to_string()),
        quote_id: Some("USDC".into()),
        active: data["isDelisted"].as_bool().map(|b| !b),
        market_type: Some(MarketType::Swap),
        spot: Some(false),
        swap: Some(true),
        linear: Some(true),
        inverse: Some(false),
        settle: Some("USDC".into()),
        settle_id: Some("USDC".into()),
        precision: Precision {
            amount: sz.map(rust_decimal::Decimal::from),
            ..Default::default()
        },
        info,
        ..Default::default()
    }
}

/// 现货(spot)市场:symbol = `BASE/QUOTE`,由 `tokens` 数组解析基础/报价币。
fn parse_spot_market(market: &Value, extra: &Value, tokens: &[Value]) -> Option<Market> {
    let name = market["name"].as_str()?;
    let positions = market["tokens"].as_array()?;
    let base_pos = positions.first()?.as_i64()? as usize;
    let quote_pos = positions.get(1)?.as_i64()? as usize;
    let base_token = tokens.get(base_pos)?;
    let quote_token = tokens.get(quote_pos)?;
    let base_name = base_token["name"].as_str()?.to_string();
    let quote_id = quote_token["name"].as_str()?.to_string();
    let symbol = format!("{base_name}/{quote_id}");
    let sz = base_token["szDecimals"].as_i64();
    let mut info = serde_json::Map::new();
    if let Value::Object(m) = extra {
        for (k, v) in m {
            info.insert(k.clone(), v.clone());
        }
    }
    if let Value::Object(m) = market {
        for (k, v) in m {
            info.insert(k.clone(), v.clone());
        }
    }
    info.insert("baseName".into(), json!(base_name));
    info.insert("swap".into(), json!(false));
    Some(Market {
        id: name.to_string(),
        symbol: symbol.clone(),
        base: Some(base_name.clone()),
        quote: Some(quote_id.clone()),
        base_id: market["index"].as_i64().map(|i| (i + 10000).to_string()),
        quote_id: Some(quote_id.clone()),
        active: Some(true),
        market_type: Some(MarketType::Spot),
        spot: Some(true),
        swap: Some(false),
        precision: Precision {
            amount: sz.map(rust_decimal::Decimal::from),
            ..Default::default()
        },
        info: Value::Object(info),
        ..Default::default()
    })
}

/// 从资产上下文(parse 后的 `info`)推导 ticker。
fn parse_ticker(info: &Value, symbol: &str) -> Ticker {
    let mid = info.get("midPx").and_then(value_decimal);
    let prev = info.get("prevDayPx").and_then(value_decimal);
    let impact = info.get("impactPxs").and_then(|v| v.as_array());
    let bid = impact.and_then(|a| a.first()).and_then(value_decimal);
    let ask = impact.and_then(|a| a.get(1)).and_then(value_decimal);
    Ticker {
        symbol: symbol.to_string(),
        last: mid,
        close: mid.or_else(|| info.get("markPx").and_then(value_decimal)),
        previous_close: prev,
        bid,
        ask,
        quote_volume: info.get("dayNtlVlm").and_then(value_decimal),
        info: info.clone(),
        ..Default::default()
    }
}

impl Exchange for Hyperliquid {
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

    async fn fetch_tickers(&self, _symbols: Option<&[&str]>, _params: Params) -> Result<Tickers> {
        self.load_markets().await?;
        let mut map = Tickers::new();
        for m in self.core.markets_snapshot().values() {
            map.insert(m.symbol.clone(), parse_ticker(&m.info, &m.symbol));
        }
        Ok(map)
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
        let market = self
            .core
            .markets_snapshot()
            .get(symbol)
            .cloned()
            .ok_or_else(|| Error::new(ErrorKind::BadResponse, "unknown symbol"))?;
        let coin = self.coin_of(&market);
        let until = Utc::now().timestamp_millis();
        let since = since.unwrap_or(0);
        let req = json!({
            "coin": coin,
            "interval": timeframe,
            "startTime": since,
            "endTime": until,
        });
        let resp = self
            .info(json!({"type": "candleSnapshot", "req": req}))
            .await?;
        let arr = resp.as_array().cloned().unwrap_or_default();
        let mut out: Vec<OHLCV> = arr
            .iter()
            .map(|c| OHLCV {
                timestamp: c["t"].as_i64(),
                open: c.get("o").and_then(value_decimal),
                high: c.get("h").and_then(value_decimal),
                low: c.get("l").and_then(value_decimal),
                close: c.get("c").and_then(value_decimal),
                volume: c.get("v").and_then(value_decimal),
            })
            .collect();
        // ccxt 默认按 since..until 截断尾部(useTail);按时间升序返回。
        out.sort_by_key(|o| o.timestamp.unwrap_or(0));
        apply_client_limit(&mut out, limit.map(|l| l as usize), true);
        Ok(out)
    }

    async fn fetch_order_book(
        &self,
        symbol: &str,
        _limit: Option<i64>,
        _params: Params,
    ) -> Result<OrderBook> {
        self.load_markets().await?;
        let market = self
            .core
            .markets_snapshot()
            .get(symbol)
            .cloned()
            .ok_or_else(|| Error::new(ErrorKind::BadResponse, "unknown symbol"))?;
        let coin = self.coin_of(&market);
        let resp = self.info(json!({"type": "l2Book", "coin": coin})).await?;
        let levels = resp
            .get("levels")
            .and_then(|l| l.as_array())
            .cloned()
            .unwrap_or_default();
        let bids_raw = levels
            .first()
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let asks_raw = levels
            .get(1)
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let to_level = |v: &Value| crate::types::Level {
            price: v["px"].as_str().and_then(|s| s.parse().ok()),
            amount: v["sz"].as_str().and_then(|s| s.parse().ok()),
        };
        let bids = bids_raw.iter().map(to_level).collect();
        let asks = asks_raw.iter().map(to_level).collect();
        let ts = resp["time"].as_i64();
        Ok(OrderBook {
            symbol: symbol.to_string(),
            bids,
            asks,
            timestamp: ts,
            datetime: ts.and_then(iso8601),
            info: resp,
            ..Default::default()
        })
    }
}
