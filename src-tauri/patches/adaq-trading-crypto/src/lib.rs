//! # adaq-trading-crypto
//!
//! 为 AdaQ 量化平台实现的加密货币与预测市场统一交易接口(Rust lib)。
//!
//! - 统一接口面:[`Exchange`](exchange::Exchange)(REST)与
//!   [`Realtime`](exchange::Realtime)(watch_*,需 `realtime` feature)。
//! - 统一数据结构:[`types`],字段与 ccxt 对齐。
//! - 精确数值:[`Precise`] 与 [`decimal_to_precision`](decimal::decimal_to_precision)。
//! - 错误体系:[`Error`] / [`ErrorKind`](error::ErrorKind),镜像 ccxt 异常树。
//!
//! 功能基线与 [ccxt](https://github.com/ccxt/ccxt) 对齐;适配器解析逻辑若
//! 参考 ccxt 源码,相应文件保留 MIT 声明(见仓库 `NOTICE`)。

pub mod adapters;
pub mod client;
pub mod contract_gen;
pub mod decimal;
pub mod eip712;
pub mod error;
pub mod exchange;
pub mod generic;
pub(crate) mod generic_parse;
pub mod httpcore;
pub mod methods;
pub mod precise;
pub mod realtime;
pub mod serde_helpers;
pub mod signing;
#[cfg(feature = "sync")]
pub mod sync;
pub mod throttle;
pub mod transport;
pub mod types;

pub use decimal::{PaddingMode, PrecisionMode, RoundingMode, decimal_to_precision};
pub use error::{Error, ErrorContext, ErrorKind, Result};
pub use exchange::{Config, Exchange, Params, Realtime};
pub use precise::Precise;
#[cfg(feature = "sync")]
pub use sync::{BlockingExchange, SyncRuntime};
pub use throttle::{ThrottleMode, Throttler};
pub use transport::Transport;
pub use types::{
    Balance, Balances, Currencies, Currency, Fee, FundingRate, LedgerEntry, Level, Limit, Limits,
    Market, MarketType, Markets, OHLCV, Order, OrderBook, Position, Precision, Ticker, Tickers,
    Trade, Transaction,
};
