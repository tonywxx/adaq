//! 交易所适配器(ADR-0005):每个交易所一个模块,按 feature 编译。
//!
//! curated 适配器的成对注册(`pub mod` / `pub use`)由 build.rs 扫描生成,
//! 见 `OUT_DIR/adapter_reg.rs`,此处只做 `include` —— 真源在适配器自身的
//! `ID` / `IMPLEMENTED` 常量,机械注册簇不再手维护(候选 4 / ADR-0001)。

/// 预测市场 outcome 索引(kalshi/polymarket 共用,ADR-0013 适配器侧共享)。
#[cfg(any(feature = "kalshi", feature = "polymarket"))]
mod outcome;

// curated 适配器注册面:由 build.rs 自动生成(扫描 src/adapters/*.rs 提取
// `impl Exchange for X` 与 `pub const ID`),写入 OUT_DIR/adapter_reg.rs。
// 其中的 `mod X;` 用 `#[path]` 显式指向 src/adapters/X.rs(include! 内的模块
// 仍按被包含文件目录解析,故必须显式给路径)。
// 请勿手改此段 —— 改适配器文件或 Cargo.toml feature 后重新构建即重建。
include!(concat!(env!("OUT_DIR"), "/adapter_reg.rs"));

/// 转译生成的交易所适配器(由 `scripts/gen_adapters.py` 从 ccxt `describe()` 生成)。
/// 子模块各自按交易所 feature 门控;本模块本身常驻。
pub mod generated;
