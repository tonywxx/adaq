//! 实时/WebSocket 面(ADR-0009):核心 8 频道 watch_*,`realtime` feature 门控。
//!
//! - [`orderbook`]:core 共享的增量订单簿引擎(ADR-0011);
//! - [`ws`]:连接基础设施(订阅命令通道 + 消息分发);
//! - [`binance`] / [`okx`] / [`bybit`] / [`kraken`] / [`kalshi`] / [`polymarket`]:各交易所 Realtime 实现。

pub mod orderbook;
pub mod watch;
pub mod ws;

#[cfg(all(feature = "realtime", feature = "binance"))]
pub mod binance;
#[cfg(all(feature = "realtime", feature = "bybit"))]
pub mod bybit;
#[cfg(all(feature = "realtime", feature = "kalshi"))]
pub mod kalshi;
#[cfg(all(feature = "realtime", feature = "kraken"))]
pub mod kraken;
#[cfg(all(feature = "realtime", feature = "okx"))]
pub mod okx;
#[cfg(all(feature = "realtime", feature = "polymarket"))]
pub mod polymarket;

#[cfg(all(feature = "realtime", feature = "binance"))]
pub use binance::BinanceWs;
#[cfg(all(feature = "realtime", feature = "bybit"))]
pub use bybit::BybitWs;
#[cfg(all(feature = "realtime", feature = "kalshi"))]
pub use kalshi::KalshiWs;
#[cfg(all(feature = "realtime", feature = "kraken"))]
pub use kraken::KrakenWs;
#[cfg(all(feature = "realtime", feature = "okx"))]
pub use okx::OkxWs;
#[cfg(all(feature = "realtime", feature = "polymarket"))]
pub use polymarket::PolymarketWs;
