//! 统一方法面清单(ADR-0001 契约基线)。
//!
//! 由 `build.rs` 从 [`crate::exchange`] 的 trait 定义自动生成,与 trait 同源、
//! 免手维护漂移。适配器在 M2 起可基于此清单声明 `has` 能力并做覆盖率校验。

include!(concat!(env!("OUT_DIR"), "/methods.rs"));

/// REST 统一方法面(Exchange trait)。
pub fn rest_methods() -> &'static [&'static str] {
    REST_METHODS
}

/// 实时方法面(Realtime trait)。
pub fn ws_methods() -> &'static [&'static str] {
    WS_METHODS
}

/// 全部方法面(REST + WS)。
pub fn all_methods() -> &'static [&'static str] {
    ALL_METHODS
}
