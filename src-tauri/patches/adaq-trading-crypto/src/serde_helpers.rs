//! 序列化助手:对 `Option<Decimal>` 做宽松的字符串/数字/null 解析。
//!
//! rust_decimal 自带的 `serde::str_option` 在 `#[serde(flatten)]` 场景下
//! 不接受 `null`(参见 types.rs 中 Balances),这里提供兼容性更好的实现:
//! - `null` → `None`
//! - 字符串 `"1.23"` → `Some(1.23)`
//! - 数字 `1.23` → `Some(1.23)`
//!
//! 序列化统一输出字符串(`None` → `null`),与 ccxt 的字符串语义一致。

use std::str::FromStr;

use rust_decimal::Decimal;
use serde::{Deserializer, Serializer};

pub mod decimal_option {
    use super::*;

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Decimal>, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::Deserialize;
        let value = Option::<serde_json::Value>::deserialize(deserializer)?;
        match value {
            None | Some(serde_json::Value::Null) => Ok(None),
            Some(serde_json::Value::String(s)) => {
                if s.trim().is_empty() {
                    return Ok(None);
                }
                Decimal::from_str(s.trim())
                    .map(Some)
                    .map_err(serde::de::Error::custom)
            }
            Some(serde_json::Value::Number(n)) => Decimal::from_str(&n.to_string())
                .map(Some)
                .map_err(serde::de::Error::custom),
            Some(other) => Err(serde::de::Error::custom(format!(
                "expected decimal string/number/null, got {other}"
            ))),
        }
    }

    pub fn serialize<S>(value: &Option<Decimal>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            Some(d) => serializer.serialize_str(&d.to_string()),
            None => serializer.serialize_none(),
        }
    }
}
