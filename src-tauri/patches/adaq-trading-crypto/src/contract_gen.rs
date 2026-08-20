//! 适配器契约 pairs(ADR-0001):由 build.rs 扫描 `src/adapters/*.rs` 自动生成。
//!
//! 真源在适配器自身的 `IMPLEMENTED` 常量;此处只做 `include`,不手抄
//! (候选 4)。`tests/contract.rs` 通过本模块的 `ADAPTER_PAIRS` 消费之。

include!(concat!(env!("OUT_DIR"), "/contract_pairs.rs"));
