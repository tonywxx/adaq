//! 同步阻塞包装(ADR-0003):`sync` feature 门控。
//!
//! 统一接口以 tokio 异步为第一形态;本模块在内部持有运行时,
//! 以阻塞方式暴露常用方法,供本地脚本、示例、教学场景免于手动
//! 维护 runtime。未包装的方法可通过 [`BlockingExchange::block_on`]
//! 透传任意异步调用。
//!
//! ```no_run
//! use adaq_trading_crypto::{Config, sync::BlockingExchange};
//! # #[cfg(feature = "binance")]
//! # fn demo() -> adaq_trading_crypto::Result<()> {
//! use adaq_trading_crypto::adapters::Binance;
//! let ex = BlockingExchange::new(Binance::new(Config::new())?)?;
//! let t = ex.fetch_ticker("BTC/USDT", Default::default())?;
//! println!("{}: {:?}", t.symbol, t.last);
//! # Ok(())
//! # }
//! ```

use std::future::Future;

use crate::error::Result;
use crate::exchange::Exchange;
use crate::types::{
    Balances, Currencies, LedgerEntry, Market, OHLCV, Order, OrderBook, Position, Ticker, Tickers,
    Trade, Transaction,
};

/// 同步运行时:内部持有 tokio multi-thread runtime。
#[derive(Debug)]
pub struct SyncRuntime {
    rt: tokio::runtime::Runtime,
}

impl SyncRuntime {
    /// 创建运行时。
    pub fn new() -> Result<Self> {
        let rt = tokio::runtime::Runtime::new().map_err(|e| {
            crate::error::Error::new(crate::error::ErrorKind::NetworkError, e.to_string())
        })?;
        Ok(Self { rt })
    }

    /// 阻塞运行一个 future 至完成。
    pub fn block_on<F: Future>(&self, f: F) -> F::Output {
        self.rt.block_on(f)
    }
}

/// 交易所的同步阻塞包装(ADR-0003)。
///
/// 泛型于具体适配器(如 [`Binance`](crate::adapters::Binance)),与 async
/// 实现共享同一份解析逻辑,不复制实现。
pub struct BlockingExchange<E: Exchange> {
    inner: E,
    rt: SyncRuntime,
}

/// 生成同步包装方法:内部 `rt.block_on(inner.method(...))`。
macro_rules! sync_method {
    ($name:ident, ($($arg:ident: $ty:ty),*), $ret:ty) => {
        /// 对应 async 方法的同步版本。
        #[allow(clippy::too_many_arguments)]
        pub fn $name(&self, $($arg: $ty),*) -> Result<$ret> {
            self.rt.block_on(self.inner.$name($($arg),*))
        }
    };
}

impl<E: Exchange> BlockingExchange<E> {
    /// 包装一个适配器(内部创建运行时)。
    pub fn new(inner: E) -> Result<Self> {
        Ok(Self {
            inner,
            rt: SyncRuntime::new()?,
        })
    }

    /// 取出底层异步适配器(丢弃运行时)。
    pub fn into_inner(self) -> E {
        self.inner
    }

    /// 阻塞运行任意异步调用(透传所有未包装方法)。
    pub fn block_on<F: Future>(&self, f: F) -> F::Output {
        self.rt.block_on(f)
    }

    // ================= 市场数据 =================

    sync_method!(fetch_time, (), i64);
    sync_method!(fetch_status, (), serde_json::Value);
    sync_method!(fetch_markets, (), Vec<Market>);
    sync_method!(fetch_currencies, (), Currencies);
    sync_method!(fetch_ticker, (symbol: &str, params: crate::exchange::Params), Ticker);
    sync_method!(
        fetch_tickers,
        (symbols: Option<&[&str]>, params: crate::exchange::Params),
        Tickers
    );
    sync_method!(
        fetch_bids_asks,
        (symbols: Option<&[&str]>, params: crate::exchange::Params),
        Tickers
    );
    sync_method!(
        fetch_ohlcv,
        (
            symbol: &str,
            timeframe: &str,
            since: Option<i64>,
            limit: Option<i64>,
            params: crate::exchange::Params
        ),
        Vec<OHLCV>
    );
    sync_method!(
        fetch_order_book,
        (symbol: &str, limit: Option<i64>, params: crate::exchange::Params),
        OrderBook
    );
    sync_method!(
        fetch_trades,
        (symbol: &str, since: Option<i64>, limit: Option<i64>, params: crate::exchange::Params),
        Vec<Trade>
    );

    // ================= 交易 =================

    sync_method!(
        create_order,
        (
            symbol: &str,
            order_type: &str,
            side: &str,
            amount: &str,
            price: Option<&str>,
            params: crate::exchange::Params
        ),
        Order
    );
    sync_method!(
        create_limit_order,
        (symbol: &str, side: &str, amount: &str, price: &str, params: crate::exchange::Params),
        Order
    );
    sync_method!(
        create_market_order,
        (symbol: &str, side: &str, amount: &str, params: crate::exchange::Params),
        Order
    );
    sync_method!(
        create_limit_buy_order,
        (symbol: &str, amount: &str, price: &str, params: crate::exchange::Params),
        Order
    );
    sync_method!(
        create_limit_sell_order,
        (symbol: &str, amount: &str, price: &str, params: crate::exchange::Params),
        Order
    );
    sync_method!(
        create_market_buy_order,
        (symbol: &str, amount: &str, params: crate::exchange::Params),
        Order
    );
    sync_method!(
        create_market_sell_order,
        (symbol: &str, amount: &str, params: crate::exchange::Params),
        Order
    );
    sync_method!(
        edit_order,
        (
            id: &str,
            symbol: &str,
            order_type: &str,
            side: &str,
            amount: &str,
            price: Option<&str>,
            params: crate::exchange::Params
        ),
        Order
    );
    sync_method!(cancel_order, (id: &str, symbol: &str, params: crate::exchange::Params), Order);
    sync_method!(
        cancel_all_orders,
        (symbol: Option<&str>, params: crate::exchange::Params),
        serde_json::Value
    );
    sync_method!(fetch_order, (id: &str, symbol: &str, params: crate::exchange::Params), Order);
    sync_method!(
        fetch_orders,
        (symbol: Option<&str>, since: Option<i64>, limit: Option<i64>, params: crate::exchange::Params),
        Vec<Order>
    );
    sync_method!(
        fetch_open_orders,
        (symbol: Option<&str>, since: Option<i64>, limit: Option<i64>, params: crate::exchange::Params),
        Vec<Order>
    );
    sync_method!(
        fetch_closed_orders,
        (symbol: Option<&str>, since: Option<i64>, limit: Option<i64>, params: crate::exchange::Params),
        Vec<Order>
    );
    sync_method!(
        fetch_my_trades,
        (symbol: Option<&str>, since: Option<i64>, limit: Option<i64>, params: crate::exchange::Params),
        Vec<Trade>
    );
    sync_method!(
        fetch_order_trades,
        (id: &str, symbol: Option<&str>, since: Option<i64>, limit: Option<i64>, params: crate::exchange::Params),
        Vec<Trade>
    );
    sync_method!(close_position, (symbol: &str, params: crate::exchange::Params), Order);

    // ================= 账户/资金 =================

    sync_method!(fetch_balance, (params: crate::exchange::Params), Balances);
    sync_method!(fetch_free_balance, (params: crate::exchange::Params), Balances);
    sync_method!(fetch_used_balance, (params: crate::exchange::Params), Balances);
    sync_method!(fetch_total_balance, (params: crate::exchange::Params), Balances);
    sync_method!(
        fetch_ledger,
        (code: Option<&str>, since: Option<i64>, limit: Option<i64>, params: crate::exchange::Params),
        Vec<LedgerEntry>
    );
    sync_method!(
        fetch_deposits,
        (code: Option<&str>, since: Option<i64>, limit: Option<i64>, params: crate::exchange::Params),
        Vec<Transaction>
    );
    sync_method!(
        fetch_withdrawals,
        (code: Option<&str>, since: Option<i64>, limit: Option<i64>, params: crate::exchange::Params),
        Vec<Transaction>
    );
    sync_method!(
        withdraw,
        (code: &str, amount: &str, address: &str, params: crate::exchange::Params),
        Transaction
    );

    // ================= 杠杆/仓位 =================

    sync_method!(fetch_position, (symbol: &str, params: crate::exchange::Params), Position);
    sync_method!(
        fetch_positions,
        (symbols: Option<&[&str]>, params: crate::exchange::Params),
        Vec<Position>
    );
    sync_method!(set_leverage, (leverage: i64, symbol: &str, params: crate::exchange::Params), serde_json::Value);
    sync_method!(
        set_margin_mode,
        (margin_mode: &str, symbol: &str, params: crate::exchange::Params),
        serde_json::Value
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorKind;
    use crate::exchange::Config;

    struct Dummy {
        config: Config,
    }

    impl Exchange for Dummy {
        fn id(&self) -> &'static str {
            "dummy"
        }
        fn config(&self) -> &Config {
            &self.config
        }
    }

    #[test]
    fn sync_wrapper_routes_to_inner() {
        let ex = BlockingExchange::new(Dummy {
            config: Config::new(),
        })
        .unwrap();
        let err = ex.fetch_ticker("BTC/USDT", Default::default()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotSupported);
        assert_eq!(err.context.method, Some("fetch_ticker"));
    }

    #[test]
    fn sync_block_on_escape_hatch() {
        let ex = BlockingExchange::new(Dummy {
            config: Config::new(),
        })
        .unwrap();
        let n = ex.block_on(async { 42 });
        assert_eq!(n, 42);
    }
}
