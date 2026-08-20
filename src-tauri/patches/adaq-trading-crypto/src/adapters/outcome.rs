//! 预测市场 outcome 索引:缓存 + 解析(适配器侧共享,ADR-0013)。
//!
//! 各交易所的 outcome 上下文类型(`T`)不同——polymarket 有 `token_id`/
//! `condition_id`,kalshi 有 `outcome_id`/`series_ticker`——共享的是
//! 「symbol → 上下文」索引的缓存、解析与错误语义(未加载 → `NotSupported`,
//! 未命中 → `BadSymbol`;双键约定:标量 symbol + 裸 id 均可解析)。
//!
//! 按 ADR-0013,预测市场特有逻辑留在适配器侧,本模块不进 HttpCore/核心。

use std::collections::HashMap;
use std::sync::Mutex;

use crate::error::{Error, ErrorKind, Result};

/// 预测市场 outcome 索引(「symbol:label」与裸 id 双键 → 上下文)。
///
/// 与 `HttpCore` 的市集缓存并行:均在 `fetch_markets_raw` 内一次性填充,
/// 单一数据源保持一致;realtime 侧经适配器 `resolve_outcome` 解析。
pub struct OutcomeIndex<T> {
    map: Mutex<Option<HashMap<String, T>>>,
}

impl<T: Clone> OutcomeIndex<T> {
    /// 空索引(未加载)。
    pub fn new() -> Self {
        Self {
            map: Mutex::new(None),
        }
    }

    /// 存储整份索引(`fetch_markets_raw` 内调用)。
    pub fn store(&self, map: HashMap<String, T>) {
        *self.map.lock().unwrap() = Some(map);
    }

    /// 精确解析。未加载 → `NotSupported`;未命中 → `BadSymbol`。
    pub fn resolve(&self, symbol: &str, exchange: &str) -> Result<T> {
        let map = self.map.lock().unwrap();
        let map = map.as_ref().ok_or_else(|| {
            Error::new(
                ErrorKind::NotSupported,
                format!("{exchange} outcomes not loaded"),
            )
        })?;
        map.get(symbol).cloned().ok_or_else(|| {
            Error::new(
                ErrorKind::BadSymbol,
                format!("unknown {exchange} outcome: {symbol}"),
            )
        })
    }

    /// 精确解析;未命中时尝试 `fallback` 键(如 kalshi 的 `{symbol}:YES`)。
    ///
    /// `exchange` 仅用于错误消息;fallback 失败沿用其自身的 `BadSymbol`。
    pub fn resolve_or(
        &self,
        symbol: &str,
        fallback: impl Fn(&str) -> String,
        exchange: &str,
    ) -> Result<T> {
        self.resolve(symbol, exchange).or_else(|e| {
            if e.kind() == ErrorKind::BadSymbol {
                self.resolve(&fallback(symbol), exchange)
            } else {
                Err(e)
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_missing_returns_not_supported() {
        let idx = OutcomeIndex::<String>::new();
        let err = idx.resolve("X", "test").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotSupported);
    }

    #[test]
    fn resolve_hit_and_miss() {
        let idx = OutcomeIndex::<String>::new();
        let mut m = HashMap::new();
        m.insert("A:YES".to_string(), "yes".to_string());
        m.insert("tok123".to_string(), "yes".to_string());
        idx.store(m);
        assert_eq!(idx.resolve("A:YES", "test").unwrap(), "yes");
        assert_eq!(idx.resolve("tok123", "test").unwrap(), "yes");
        assert_eq!(
            idx.resolve("NOPE", "test").unwrap_err().kind(),
            ErrorKind::BadSymbol
        );
    }

    #[test]
    fn resolve_or_falls_back_on_miss_only() {
        let idx = OutcomeIndex::<String>::new();
        let mut m = HashMap::new();
        m.insert("A:YES".to_string(), "yes".to_string());
        idx.store(m);
        // 精确命中
        assert_eq!(
            idx.resolve_or("A:YES", |s| format!("{s}:YES"), "t")
                .unwrap(),
            "yes"
        );
        // 未命中 → fallback 命中
        assert_eq!(
            idx.resolve_or("A", |s| format!("{s}:YES"), "t").unwrap(),
            "yes"
        );
        // 都未命中 → BadSymbol
        assert_eq!(
            idx.resolve_or("B", |s| format!("{s}:YES"), "t")
                .unwrap_err()
                .kind(),
            ErrorKind::BadSymbol
        );
    }
}
