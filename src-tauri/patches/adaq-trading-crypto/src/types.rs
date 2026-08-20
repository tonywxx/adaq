//! 统一市场数据结构(ADR-0002)。
//!
//! 字段名与 ccxt 对齐(snake_case);所有非必需字段均为 `Option` +
//! `#[serde(default)]` 容错;`info` 字段延续 ccxt 惯例携带原始响应;
//! 数值用 `rust_decimal`(字符串序列化),与 ccxt 的字符串/十进制语义一致。
//!
//! 说明:订单类型/状态/方向等高频变化字段(ccxt 为松散字符串)暂以
//! `String` 表达,避免解析失败;`MarketType` 为封闭枚举 + `Other` 兜底。

use std::collections::HashMap;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 市集类型(ccxt `market.type`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MarketType {
    Spot,
    Margin,
    Swap,
    Future,
    Option,
    Delivery,
    Index,
    Prediction,
    /// 二元预测市场(YES/NO)。
    Binary,
    /// 多类别预测市场。
    Categorical,
    /// 未知/新增类型兜底。
    #[serde(other)]
    Other,
}

/// 手续费。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Fee {
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub cost: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub rate: Option<Decimal>,
}

/// 最小/最大限制。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Limit {
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub min: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub max: Option<Decimal>,
}

/// 精度限制。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Precision {
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub amount: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub price: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub cost: Option<Decimal>,
}

/// 各类限制。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Limits {
    #[serde(default)]
    pub amount: Option<Limit>,
    #[serde(default)]
    pub cost: Option<Limit>,
    #[serde(default)]
    pub leverage: Option<Limit>,
    #[serde(default)]
    pub price: Option<Limit>,
    #[serde(default)]
    pub market: Option<Limit>,
}

/// 市集(Market)。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Market {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub symbol: String,
    #[serde(default)]
    pub base: Option<String>,
    #[serde(default)]
    pub quote: Option<String>,
    #[serde(default)]
    pub base_id: Option<String>,
    #[serde(default)]
    pub quote_id: Option<String>,
    #[serde(default)]
    pub active: Option<bool>,
    #[serde(default)]
    pub market_type: Option<MarketType>,
    /// 合约子类型:linear / inverse。
    #[serde(default)]
    pub sub_type: Option<String>,
    #[serde(default)]
    pub spot: Option<bool>,
    #[serde(default)]
    pub margin: Option<bool>,
    #[serde(default)]
    pub swap: Option<bool>,
    #[serde(default)]
    pub future: Option<bool>,
    #[serde(default)]
    pub option: Option<bool>,
    #[serde(default)]
    pub settle: Option<String>,
    #[serde(default)]
    pub settle_id: Option<String>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub contract_size: Option<Decimal>,
    #[serde(default)]
    pub linear: Option<bool>,
    #[serde(default)]
    pub inverse: Option<bool>,
    #[serde(default)]
    pub expiry: Option<i64>,
    #[serde(default)]
    pub expiry_datetime: Option<String>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub strike: Option<Decimal>,
    #[serde(default)]
    pub option_type: Option<String>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub taker: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub maker: Option<Decimal>,
    #[serde(default)]
    pub precision: Precision,
    #[serde(default)]
    pub limits: Limits,
    #[serde(default)]
    pub created: Option<i64>,
    #[serde(default)]
    pub info: Value,
}

/// 币种(Currency)。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Currency {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub numeric_id: Option<String>,
    #[serde(default)]
    pub precision: Option<i64>,
    #[serde(default)]
    pub currency_type: Option<String>,
    #[serde(default)]
    pub margin: Option<bool>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub active: Option<bool>,
    #[serde(default)]
    pub deposit: Option<bool>,
    #[serde(default)]
    pub withdraw: Option<bool>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub fee: Option<Decimal>,
    #[serde(default)]
    pub limits: Limits,
    #[serde(default)]
    pub networks: Option<Value>,
    #[serde(default)]
    pub info: Value,
}

/// 行情快照(Ticker)。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Ticker {
    #[serde(default)]
    pub symbol: String,
    #[serde(default)]
    pub info: Value,
    #[serde(default)]
    pub timestamp: Option<i64>,
    #[serde(default)]
    pub datetime: Option<String>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub high: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub low: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub bid: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub ask: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub bid_volume: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub ask_volume: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub vwap: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub open: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub close: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub last: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub previous_close: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub change: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub percentage: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub average: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub quote_volume: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub base_volume: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub index_price: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub mark_price: Option<Decimal>,
}

/// 订单簿档位。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Level {
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub price: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub amount: Option<Decimal>,
}

/// 订单簿(OrderBook)。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OrderBook {
    #[serde(default)]
    pub symbol: String,
    #[serde(default)]
    pub asks: Vec<Level>,
    #[serde(default)]
    pub bids: Vec<Level>,
    #[serde(default)]
    pub timestamp: Option<i64>,
    #[serde(default)]
    pub datetime: Option<String>,
    #[serde(default)]
    pub nonce: Option<i64>,
    #[serde(default)]
    pub info: Value,
}

/// 订单(Order)。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Order {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub client_order_id: Option<String>,
    #[serde(default)]
    pub timestamp: Option<i64>,
    #[serde(default)]
    pub datetime: Option<String>,
    #[serde(default)]
    pub last_trade_timestamp: Option<i64>,
    #[serde(default)]
    pub last_update_timestamp: Option<i64>,
    /// open / closed / canceled / expired / rejected / pending。
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub symbol: Option<String>,
    /// limit / market / stop / stop_limit / …
    #[serde(default)]
    pub order_type: Option<String>,
    /// GTC / IOC / FOK / PO …
    #[serde(default)]
    pub time_in_force: Option<String>,
    /// buy / sell。
    #[serde(default)]
    pub side: Option<String>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub price: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub average: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub amount: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub filled: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub remaining: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub stop_price: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub trigger_price: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub take_profit_price: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub stop_loss_price: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub cost: Option<Decimal>,
    #[serde(default)]
    pub trades: Option<Vec<Trade>>,
    #[serde(default)]
    pub fee: Option<Fee>,
    #[serde(default)]
    pub reduce_only: Option<bool>,
    #[serde(default)]
    pub post_only: Option<bool>,
    #[serde(default)]
    pub info: Value,
}

/// 成交(Trade)。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Trade {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub info: Value,
    /// 所属订单 id。
    #[serde(default)]
    pub order: Option<String>,
    #[serde(default)]
    pub timestamp: Option<i64>,
    #[serde(default)]
    pub datetime: Option<String>,
    #[serde(default)]
    pub symbol: Option<String>,
    #[serde(default)]
    pub trade_type: Option<String>,
    #[serde(default)]
    pub side: Option<String>,
    /// taker / maker。
    #[serde(default)]
    pub taker_or_maker: Option<String>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub price: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub amount: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub cost: Option<Decimal>,
    #[serde(default)]
    pub fee: Option<Fee>,
}

/// 仓位(Position)。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Position {
    #[serde(default)]
    pub symbol: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub info: Value,
    #[serde(default)]
    pub timestamp: Option<i64>,
    #[serde(default)]
    pub datetime: Option<String>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub contracts: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub contract_size: Option<Decimal>,
    /// long / short。
    #[serde(default)]
    pub side: Option<String>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub notional: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub leverage: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub unrealized_pnl: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub realized_pnl: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub collateral: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub entry_price: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub mark_price: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub liquidation_price: Option<Decimal>,
    /// cross / isolated。
    #[serde(default)]
    pub margin_mode: Option<String>,
    #[serde(default)]
    pub hedged: Option<bool>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub maintenance_margin: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub maintenance_margin_percentage: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub initial_margin: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub initial_margin_percentage: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub margin_ratio: Option<Decimal>,
    #[serde(default)]
    pub last_update_timestamp: Option<i64>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub last_price: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub stop_loss_price: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub take_profit_price: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub percentage: Option<Decimal>,
}

/// 单个币种资金(Balance)。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Balance {
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub free: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub used: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub total: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub debt: Option<Decimal>,
}

/// 账户资金整体(Balances):`accounts` 按币种 code 索引。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Balances {
    #[serde(default)]
    pub info: Value,
    #[serde(default)]
    pub timestamp: Option<i64>,
    #[serde(default)]
    pub datetime: Option<String>,
    #[serde(flatten)]
    pub accounts: HashMap<String, Balance>,
}

/// 账本条目(LedgerEntry)。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LedgerEntry {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub timestamp: Option<i64>,
    #[serde(default)]
    pub datetime: Option<String>,
    /// in / out。
    #[serde(default)]
    pub direction: Option<String>,
    #[serde(default)]
    pub account: Option<String>,
    #[serde(default)]
    pub reference_id: Option<String>,
    #[serde(default)]
    pub reference_account: Option<String>,
    /// trade / fee / deposit / withdrawal / transfer …
    #[serde(default)]
    pub entry_type: Option<String>,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub amount: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub before: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub after: Option<Decimal>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub fee: Option<Fee>,
    #[serde(default)]
    pub info: Value,
}

/// 充值/提现记录(Transaction)。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Transaction {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub txid: Option<String>,
    #[serde(default)]
    pub timestamp: Option<i64>,
    #[serde(default)]
    pub datetime: Option<String>,
    #[serde(default)]
    pub address: Option<String>,
    #[serde(default)]
    pub address_from: Option<String>,
    #[serde(default)]
    pub address_to: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub tag_from: Option<String>,
    #[serde(default)]
    pub tag_to: Option<String>,
    /// deposit / withdrawal。
    #[serde(default)]
    pub tx_type: Option<String>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub amount: Option<Decimal>,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub updated: Option<i64>,
    #[serde(default)]
    pub fee: Option<Fee>,
    #[serde(default)]
    pub network: Option<String>,
    #[serde(default)]
    pub comment: Option<String>,
    #[serde(default)]
    pub internal: Option<bool>,
    #[serde(default)]
    pub info: Value,
}

/// 资金费率(FundingRate)。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FundingRate {
    #[serde(default)]
    pub symbol: Option<String>,
    #[serde(default)]
    pub timestamp: Option<i64>,
    #[serde(default)]
    pub datetime: Option<String>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub funding_rate: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub mark_price: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub index_price: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub interest_rate: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub estimated_settle_price: Option<Decimal>,
    #[serde(default)]
    pub funding_timestamp: Option<i64>,
    #[serde(default)]
    pub funding_datetime: Option<String>,
    #[serde(default)]
    pub next_funding_timestamp: Option<i64>,
    #[serde(default)]
    pub next_funding_datetime: Option<String>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub next_funding_rate: Option<Decimal>,
    #[serde(default)]
    pub previous_funding_timestamp: Option<i64>,
    #[serde(default)]
    pub previous_funding_datetime: Option<String>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub previous_funding_rate: Option<Decimal>,
    #[serde(default)]
    pub interval: Option<String>,
    #[serde(default)]
    pub info: Value,
}

/// K 线(OHLCV):统一结构(ccxt 中为元组)。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OHLCV {
    #[serde(default)]
    pub timestamp: Option<i64>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub open: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub high: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub low: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub close: Option<Decimal>,
    #[serde(default, with = "crate::serde_helpers::decimal_option")]
    pub volume: Option<Decimal>,
}

/// 按 symbol 索引的市集集合。
pub type Markets = HashMap<String, Market>;
/// 按 code 索引的币种集合。
pub type Currencies = HashMap<String, Currency>;
/// 按 symbol 索引的行情集合。
pub type Tickers = HashMap<String, Ticker>;
/// 多币种资金。
pub type BalancesByCode = HashMap<String, Balance>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn market_type_serde() {
        let t: MarketType = serde_json::from_str("\"swap\"").unwrap();
        assert_eq!(t, MarketType::Swap);
        let t: MarketType = serde_json::from_str("\"prediction\"").unwrap();
        assert_eq!(t, MarketType::Prediction);
        let t: MarketType = serde_json::from_str("\"weird\"").unwrap();
        assert_eq!(t, MarketType::Other);
    }

    #[test]
    fn order_roundtrip_with_missing_fields() {
        let json = r#"{"id":"123","symbol":"BTC/USDT","side":"buy","info":{"x":1}}"#;
        let order: Order = serde_json::from_str(json).unwrap();
        assert_eq!(order.id.as_deref(), Some("123"));
        assert_eq!(order.price, None);
        assert_eq!(order.info["x"], 1);
    }

    #[test]
    fn decimal_string_serde() {
        let json = r#"{"price":"1.2300","amount":null}"#;
        let level: Level = serde_json::from_str(json).unwrap();
        assert_eq!(level.price.unwrap().to_string(), "1.2300");
        assert_eq!(level.amount, None);
        let out = serde_json::to_string(&level).unwrap();
        assert!(out.contains("\"1.2300\""));
    }

    #[test]
    fn balances_flatten() {
        let json = r#"{"info":{},"BTC":{"free":"1.5","total":"2"},"ETH":{"free":null}}"#;
        let b: Balances = serde_json::from_str(json).unwrap();
        assert_eq!(b.accounts["BTC"].free.unwrap().to_string(), "1.5");
        assert!(b.accounts["ETH"].free.is_none());
    }
}
