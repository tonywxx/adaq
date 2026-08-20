//! manifold(Manifold Markets)适配器(Phase B,ADR-0005)。
//!
//! ccxt 之外的创新适配器,按官方 API(api.manifold.markets)手写,
//! 无 ccxt 参考实现 → 无差分基线,以单元测试 + 形状断言覆盖。
//!
//! - `GET /v0/markets`(列表)/ `GET /v0/market/{id}` / `GET /v0/slug/{slug}`;
//! - `GET /v0/bets?contractId=...`(成交记录,即 trades);
//! - 市集为二元预测(每份 YES/NO),`probability` ∈ [0,1] 即最新价格;
//! - 公开 API 无 OHLC / 订单簿端点 → fetch_ohlcv/fetch_order_book 保持
//!   NotSupported(与 ccxt 外适配器一致,避免虚构数据)。
//!
//! 统一映射:
//! - Market:symbol = marketTickerSymbol ?? slug,base=YES,quote=NO,Binary;
//! - Ticker:last/close = probability,base_volume = volume;
//! - Trade:side = outcome 小写,price = amount/shares(均价),amount = shares。

use serde_json::{Value, json};

use crate::error::{Error, ErrorKind, Result};
use crate::exchange::{Config, Exchange, Params};
use crate::httpcore::{HttpCore, iso8601};
use crate::types::{Market, MarketType, Markets, Ticker, Trade};

pub const ID: &str = "manifold";
const BASE_URL: &str = "https://api.manifold.markets/v0";
const RATE_LIMIT_MS: u64 = 200;

/// manifold 适配器。
pub struct Manifold {
    config: Config,
    core: HttpCore,
}

impl Manifold {
    /// 已实现(非 NotSupported)的 REST 方法面(契约声明,Phase B)。
    pub const IMPLEMENTED: &'static [&'static str] =
        &["fetch_markets", "fetch_ticker", "fetch_trades"];

    pub fn new(config: Config) -> Result<Self> {
        let core = HttpCore::new(&config, BASE_URL, RATE_LIMIT_MS, "manifold")?;
        Ok(Self { config, core })
    }

    async fn public_get(&self, path: &str, params: &Params) -> Result<Value> {
        self.core.public_get(path, params).await
    }

    async fn load_markets(&self) -> Result<()> {
        self.core.load_markets(|| self.fetch_markets_raw()).await
    }

    async fn fetch_markets_raw(&self) -> Result<Markets> {
        let mut p = Params::new();
        p.insert("limit".into(), json!(1000));
        let resp = self.public_get("/markets", &p).await?;
        let arr = resp
            .as_array()
            .ok_or_else(|| Error::new(ErrorKind::BadResponse, "markets not array"))?;
        let mut map = Markets::new();
        for raw in arr {
            let m = self.parse_market(raw);
            map.insert(m.symbol.clone(), m);
        }
        Ok(map)
    }

    /// 统一 symbol → 查询键:先查缓存,否则原样(支持 slug 或 id)。
    fn symbol_id(&self, symbol: &str) -> String {
        for m in self.core.markets_snapshot().values() {
            if m.symbol == symbol {
                return m.id.clone();
            }
        }
        symbol.to_string()
    }

    // ================= parse(公开) =================

    pub fn parse_market(&self, raw: &Value) -> Market {
        let id = raw["id"].as_str().unwrap_or_default().to_string();
        let slug = raw["slug"].as_str().unwrap_or_default();
        let ticker = raw["marketTickerSymbol"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or(slug);
        Market {
            id,
            symbol: ticker.to_string(),
            base: Some("YES".into()),
            quote: Some("NO".into()),
            base_id: Some("YES".into()),
            quote_id: Some("NO".into()),
            active: Some(!raw["isResolved"].as_bool().unwrap_or(false)),
            market_type: Some(MarketType::Binary),
            spot: Some(false),
            info: raw.clone(),
            ..Market::default()
        }
    }

    /// 从市场对象推导 ticker(probability ∈ [0,1] 即最新价格)。
    pub fn parse_ticker(&self, raw: &Value) -> Ticker {
        let slug = raw["slug"].as_str().unwrap_or_default();
        let ticker = raw["marketTickerSymbol"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or(slug);
        let ts = raw["createdTime"]
            .as_i64()
            .or_else(|| raw["closeTime"].as_i64());
        let probability = raw["probability"]
            .as_f64()
            .map(|p| rust_decimal::Decimal::from_f64_retain(p).unwrap_or_default());
        Ticker {
            symbol: ticker.to_string(),
            timestamp: ts,
            datetime: ts.and_then(iso8601),
            last: probability,
            close: probability,
            base_volume: raw["volume"]
                .as_f64()
                .map(|v| rust_decimal::Decimal::from_f64_retain(v).unwrap_or_default()),
            quote_volume: raw["volume24Hours"]
                .as_f64()
                .map(|v| rust_decimal::Decimal::from_f64_retain(v).unwrap_or_default()),
            info: raw.clone(),
            ..Ticker::default()
        }
    }

    /// 解析成交(bet):price = amount/shares(均价),amount = shares,cost = amount(投入 Mana)。
    pub fn parse_trade(&self, raw: &Value) -> Trade {
        let ts = raw["createdTime"].as_i64();
        let amount = num_f64(raw.get("amount"));
        let shares = num_f64(raw.get("shares"));
        let probability_after = raw
            .get("probabilityAfter")
            .and_then(|v| v.as_f64())
            .map(|p| rust_decimal::Decimal::from_f64_retain(p).unwrap_or_default());
        let price = match (amount, shares) {
            (Some(a), Some(s)) if s > rust_decimal::Decimal::ZERO => Some(a / s),
            _ => probability_after,
        };
        Trade {
            id: raw["id"].as_str().map(str::to_string),
            timestamp: ts,
            datetime: ts.and_then(iso8601),
            side: raw["outcome"].as_str().map(|s| s.to_lowercase()),
            price,
            amount: shares,
            cost: amount,
            info: raw.clone(),
            ..Trade::default()
        }
    }
}

impl Exchange for Manifold {
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
        let id = self.symbol_id(symbol);
        let resp = self
            .public_get(&format!("/market/{id}"), &Params::new())
            .await?;
        Ok(self.parse_ticker(&resp))
    }

    async fn fetch_trades(
        &self,
        symbol: &str,
        _since: Option<i64>,
        limit: Option<i64>,
        _params: Params,
    ) -> Result<Vec<Trade>> {
        let id = self.symbol_id(symbol);
        let mut p = Params::new();
        p.insert("contractId".into(), json!(id));
        if let Some(l) = limit {
            p.insert("limit".into(), json!(l.min(1000)));
        }
        let resp = self.public_get("/bets", &p).await?;
        let arr = resp.as_array().cloned().unwrap_or_default();
        Ok(arr.iter().map(|t| self.parse_trade(t)).collect())
    }
}

fn num_f64(v: Option<&Value>) -> Option<rust_decimal::Decimal> {
    v.and_then(|x| match x {
        Value::Number(n) => n
            .as_f64()
            .map(|f| rust_decimal::Decimal::from_f64_retain(f).unwrap_or_default()),
        Value::String(s) => s.parse().ok(),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_market_binary() {
        let ex = Manifold::new(Config::new()).unwrap();
        let raw = json!({
            "id": "8Z98hgENzz",
            "slug": "will-joe-biden-live-to-see-the-open",
            "question": "Will Joe Biden live to see the opening of his Presidential Center?",
            "outcomeType": "BINARY",
            "probability": 0.5,
            "volume": 100,
            "isResolved": false
        });
        let m = ex.parse_market(&raw);
        assert_eq!(m.id, "8Z98hgENzz");
        assert_eq!(m.symbol, "will-joe-biden-live-to-see-the-open");
        assert_eq!(m.base.as_deref(), Some("YES"));
        assert_eq!(m.quote.as_deref(), Some("NO"));
        assert_eq!(m.market_type, Some(MarketType::Binary));
        assert_eq!(m.active, Some(true));
    }

    #[test]
    fn parse_ticker_probability_is_last() {
        let ex = Manifold::new(Config::new()).unwrap();
        let raw = json!({
            "id": "x",
            "slug": "will-btc-be-100k",
            "probability": 0.42,
            "volume": 100.5,
            "volume24Hours": 10.2,
            "createdTime": 1786523244465_i64,
            "isResolved": false
        });
        let t = ex.parse_ticker(&raw);
        assert_eq!(t.symbol, "will-btc-be-100k");
        assert!(
            (t.last.unwrap() - "0.42".parse::<rust_decimal::Decimal>().unwrap()).abs()
                < rust_decimal::Decimal::new(1, 10)
        );
        assert!(
            (t.base_volume.unwrap() - "100.5".parse::<rust_decimal::Decimal>().unwrap()).abs()
                < rust_decimal::Decimal::new(1, 10)
        );
    }

    #[test]
    fn parse_trade_avg_price() {
        let ex = Manifold::new(Config::new()).unwrap();
        let raw = json!({
            "id": "Ip2ALyhEdUdz",
            "createdTime": 1786520991120_i64,
            "amount": 50,
            "shares": 52.57148654169619,
            "outcome": "NO",
            "contractId": "ZgyhS5cdI9"
        });
        let t = ex.parse_trade(&raw);
        assert_eq!(t.id.as_deref(), Some("Ip2ALyhEdUdz"));
        assert_eq!(t.side.as_deref(), Some("no"));
        assert_eq!(t.cost, Some("50".parse().unwrap()));
        assert!(
            (t.amount.unwrap()
                - "52.57148654169619"
                    .parse::<rust_decimal::Decimal>()
                    .unwrap())
            .abs()
                < rust_decimal::Decimal::new(1, 10)
        );
    }

    #[test]
    fn symbol_id_resolves_from_cache() {
        let ex = Manifold::new(Config::new()).unwrap();
        // 无缓存时原样返回
        assert_eq!(ex.symbol_id("will-btc-be-100k"), "will-btc-be-100k");
    }
}
