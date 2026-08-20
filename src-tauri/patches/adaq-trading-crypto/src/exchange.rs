//! 统一接口面(ADR-0002/0003/0006)。
//!
//! - [`Exchange`]:REST 统一方法面(方法名与 ccxt 对齐),所有方法默认
//!   返回 `NotSupported`,适配器按需覆写——这保证了"方法面 100% 可枚举"的
//!   契约测试基线(ADR-0001)。
//! - [`Realtime`]:watch_* 实时面(ADR-0009,核心 8 频道),仅在 `realtime`
//!   feature 下编译。
//! - [`Config`]:交易所配置;`Params`:透传给交易所的额外参数(对应 ccxt `params`)。

// 刻意使用原生 async fn in trait(edition 2024,Rust 1.85+),由宏生成默认
// 实现;适配器以具体类型使用(放弃 dyn 对象安全,换取 trait 内宏生成可行)。
#![allow(async_fn_in_trait)]

use crate::error::{Error, Result};
use crate::types::{
    Balances, Currencies, FundingRate, LedgerEntry, Market, OHLCV, Order, OrderBook, Position,
    Ticker, Tickers, Trade, Transaction,
};

/// 透传参数(对应 ccxt `params`):JSON 对象,适配器自行解释。
pub type Params = serde_json::Map<String, serde_json::Value>;

/// 交易所配置。
#[derive(Debug, Clone, Default)]
pub struct Config {
    pub api_key: Option<String>,
    pub secret: Option<String>,
    pub password: Option<String>,
    pub uid: Option<String>,
    /// 私钥 PEM / hex(kalshi RSA / polymarket secp256k1 等签名用)。
    pub private_key: Option<String>,
    /// 钱包地址(polymarket fetchPositions 等按地址查询的端点用)。
    pub wallet_address: Option<String>,
    /// 沙箱模式(若有)。
    pub sandbox: bool,
    /// 请求超时(毫秒),默认 10000(对齐 ccxt)。
    pub timeout_ms: u64,
    /// 失败重试次数,默认 2。
    pub max_retries: u32,
    /// 是否启用内置限速(对齐 ccxt `enableRateLimit`)。
    pub enable_rate_limit: bool,
    /// 单次请求最小间隔(毫秒),如 binance = 50。仅 `enable_rate_limit` 时生效。
    pub rate_limit_ms: u64,
    /// HTTP 代理,如 `http://127.0.0.1:8080`。
    pub proxy: Option<String>,
    /// 交易所特有选项(对应 ccxt `options`)。
    pub options: serde_json::Map<String, serde_json::Value>,
}

impl Config {
    pub fn new() -> Self {
        Self {
            timeout_ms: 10_000,
            max_retries: 2,
            ..Self::default()
        }
    }
}

/// 生成统一默认方法:未覆写时返回 `NotSupported`。
macro_rules! def_method {
    ($name:ident, ($($arg:tt)*), $ret:ty) => {
        async fn $name(&self, $($arg)*) -> Result<$ret> {
            Err(Error::not_supported(stringify!($name)))
        }
    };
}

/// REST 统一接口面(方法名与 ccxt 对齐,ADR-0002)。
///
/// 标注 `TODO(M1)` 的方法返回 `Value`,待契约测试阶段补齐精确类型。
#[allow(unused_variables, clippy::too_many_arguments)]
pub trait Exchange: Send + Sync {
    /// 交易所 id,如 `binance`。
    fn id(&self) -> &'static str;

    /// 交易所配置。
    fn config(&self) -> &Config;

    // ================= 市场数据 =================

    /// 服务器时间(毫秒时间戳)。
    async fn fetch_time(&self) -> Result<i64> {
        Err(Error::not_supported("fetch_time"))
    }
    /// 交易所/账户状态。
    async fn fetch_status(&self) -> Result<serde_json::Value> {
        Err(Error::not_supported("fetch_status"))
    }
    /// 市集列表。
    async fn fetch_markets(&self) -> Result<Vec<Market>> {
        Err(Error::not_supported("fetch_markets"))
    }
    /// 币种列表(按 code 索引)。
    async fn fetch_currencies(&self) -> Result<Currencies> {
        Err(Error::not_supported("fetch_currencies"))
    }
    def_method!(fetch_ticker, (symbol: &str, params: Params), Ticker);
    def_method!(fetch_tickers, (symbols: Option<&[&str]>, params: Params), Tickers);
    def_method!(fetch_bids_asks, (symbols: Option<&[&str]>, params: Params), Tickers);
    def_method!(
        fetch_ohlcv,
        (
            symbol: &str,
            timeframe: &str,
            since: Option<i64>,
            limit: Option<i64>,
            params: Params
        ),
        Vec<OHLCV>
    );
    def_method!(fetch_order_book, (symbol: &str, limit: Option<i64>, params: Params), OrderBook);
    def_method!(
        fetch_order_books,
        (symbols: Option<&[&str]>, limit: Option<i64>, params: Params),
        serde_json::Value
    );
    def_method!(
        fetch_trades,
        (symbol: &str, since: Option<i64>, limit: Option<i64>, params: Params),
        Vec<Trade>
    );
    def_method!(
        fetch_liquidations,
        (symbol: &str, since: Option<i64>, limit: Option<i64>, params: Params),
        Vec<Trade>
    );
    def_method!(
        fetch_my_liquidations,
        (symbol: &str, since: Option<i64>, limit: Option<i64>, params: Params),
        Vec<Trade>
    );
    def_method!(fetch_funding_rate, (symbol: &str, params: Params), FundingRate);
    def_method!(
        fetch_funding_rates,
        (symbols: Option<&[&str]>, params: Params),
        std::collections::HashMap<String, FundingRate>
    );
    def_method!(fetch_funding_interval, (symbol: &str, params: Params), serde_json::Value);
    def_method!(
        fetch_funding_intervals,
        (symbols: Option<&[&str]>, params: Params),
        serde_json::Value
    );
    def_method!(
        fetch_funding_rate_history,
        (symbol: &str, since: Option<i64>, limit: Option<i64>, params: Params),
        Vec<FundingRate>
    );
    def_method!(
        fetch_funding_history,
        (symbol: &str, since: Option<i64>, limit: Option<i64>, params: Params),
        serde_json::Value
    );
    def_method!(fetch_open_interest, (symbol: &str, params: Params), serde_json::Value);
    def_method!(
        fetch_open_interests,
        (symbols: Option<&[&str]>, params: Params),
        serde_json::Value
    );
    def_method!(
        fetch_open_interest_history,
        (symbol: &str, timeframe: &str, since: Option<i64>, limit: Option<i64>, params: Params),
        serde_json::Value
    );
    def_method!(fetch_mark_price, (symbol: &str, params: Params), Ticker);
    def_method!(fetch_mark_prices, (symbols: Option<&[&str]>, params: Params), Tickers);
    def_method!(fetch_last_prices, (symbols: Option<&[&str]>, params: Params), Tickers);
    def_method!(fetch_trading_fees, (params: Params), serde_json::Value);
    def_method!(fetch_trading_limits, (symbols: Option<&[&str]>, params: Params), serde_json::Value);
    def_method!(
        fetch_leverage_tiers,
        (symbols: Option<&[&str]>, params: Params),
        serde_json::Value
    );
    def_method!(
        fetch_market_leverage_tiers,
        (symbol: &str, params: Params),
        serde_json::Value
    );
    def_method!(fetch_long_short_ratio, (symbol: &str, params: Params), serde_json::Value);
    def_method!(
        fetch_long_short_ratio_history,
        (symbol: &str, timeframe: &str, since: Option<i64>, limit: Option<i64>, params: Params),
        serde_json::Value
    );
    def_method!(fetch_greeks, (symbol: &str, params: Params), serde_json::Value);
    def_method!(fetch_option, (symbol: &str, params: Params), serde_json::Value);
    def_method!(fetch_option_chain, (code: &str, params: Params), serde_json::Value);
    def_method!(fetch_adl_rank, (symbol: &str, params: Params), serde_json::Value);
    def_method!(fetch_position_adl_rank, (symbol: &str, params: Params), serde_json::Value);
    def_method!(
        fetch_positions_adl_rank,
        (symbols: Option<&[&str]>, params: Params),
        serde_json::Value
    );
    def_method!(fetch_convert_quote, (from: &str, to: &str, amount: &str, params: Params), serde_json::Value);
    def_method!(fetch_convert_currencies, (params: Params), serde_json::Value);

    // ================= 交易 =================

    // 下单。`price` 为 `None` 表示市价单。
    def_method!(
        create_order,
        (
            symbol: &str,
            order_type: &str,
            side: &str,
            amount: &str,
            price: Option<&str>,
            params: Params
        ),
        Order
    );
    def_method!(create_orders, (orders: &[serde_json::Value], params: Params), Vec<Order>);
    def_method!(create_spot_orders, (orders: &[serde_json::Value], params: Params), Vec<Order>);
    def_method!(create_contract_orders, (orders: &[serde_json::Value], params: Params), Vec<Order>);
    def_method!(
        create_limit_order,
        (symbol: &str, side: &str, amount: &str, price: &str, params: Params),
        Order
    );
    def_method!(
        create_market_order,
        (symbol: &str, side: &str, amount: &str, params: Params),
        Order
    );
    def_method!(
        create_limit_buy_order,
        (symbol: &str, amount: &str, price: &str, params: Params),
        Order
    );
    def_method!(
        create_limit_sell_order,
        (symbol: &str, amount: &str, price: &str, params: Params),
        Order
    );
    def_method!(
        create_market_buy_order,
        (symbol: &str, amount: &str, params: Params),
        Order
    );
    def_method!(
        create_market_sell_order,
        (symbol: &str, amount: &str, params: Params),
        Order
    );
    def_method!(
        create_post_only_order,
        (symbol: &str, side: &str, amount: &str, price: &str, params: Params),
        Order
    );
    def_method!(
        create_reduce_only_order,
        (symbol: &str, side: &str, amount: &str, price: Option<&str>, params: Params),
        Order
    );
    def_method!(
        create_trigger_order,
        (
            symbol: &str,
            order_type: &str,
            side: &str,
            amount: &str,
            price: Option<&str>,
            trigger_price: &str,
            params: Params
        ),
        Order
    );
    def_method!(
        create_stop_order,
        (
            symbol: &str,
            side: &str,
            amount: &str,
            price: Option<&str>,
            stop_price: &str,
            params: Params
        ),
        Order
    );
    def_method!(
        create_stop_limit_order,
        (
            symbol: &str,
            side: &str,
            amount: &str,
            price: &str,
            stop_price: &str,
            params: Params
        ),
        Order
    );
    def_method!(
        create_stop_market_order,
        (symbol: &str, side: &str, amount: &str, stop_price: &str, params: Params),
        Order
    );
    def_method!(
        create_stop_loss_order,
        (
            symbol: &str,
            side: &str,
            amount: &str,
            price: Option<&str>,
            stop_loss_price: &str,
            params: Params
        ),
        Order
    );
    def_method!(
        create_take_profit_order,
        (
            symbol: &str,
            side: &str,
            amount: &str,
            price: Option<&str>,
            take_profit_price: &str,
            params: Params
        ),
        Order
    );
    def_method!(
        create_order_with_take_profit_and_stop_loss,
        (
            symbol: &str,
            side: &str,
            amount: &str,
            price: Option<&str>,
            take_profit_price: Option<&str>,
            stop_loss_price: Option<&str>,
            params: Params
        ),
        Order
    );
    def_method!(
        create_market_order_with_cost,
        (symbol: &str, side: &str, cost: &str, params: Params),
        Order
    );
    def_method!(
        create_market_buy_order_with_cost,
        (symbol: &str, cost: &str, params: Params),
        Order
    );
    def_method!(
        create_market_sell_order_with_cost,
        (symbol: &str, cost: &str, params: Params),
        Order
    );
    def_method!(
        create_trailing_amount_order,
        (
            symbol: &str,
            side: &str,
            amount: &str,
            price: Option<&str>,
            trailing_amount: &str,
            params: Params
        ),
        Order
    );
    def_method!(
        create_trailing_percent_order,
        (
            symbol: &str,
            side: &str,
            amount: &str,
            price: Option<&str>,
            trailing_percent: &str,
            params: Params
        ),
        Order
    );
    def_method!(
        create_twap_order,
        (
            symbol: &str,
            side: &str,
            amount: &str,
            price: Option<&str>,
            params: Params
        ),
        Order
    );
    def_method!(
        edit_order,
        (
            id: &str,
            symbol: &str,
            order_type: &str,
            side: &str,
            amount: &str,
            price: Option<&str>,
            params: Params
        ),
        Order
    );
    def_method!(edit_orders, (orders: &[serde_json::Value], params: Params), Vec<Order>);
    def_method!(
        edit_limit_order,
        (id: &str, symbol: &str, side: &str, amount: &str, price: &str, params: Params),
        Order
    );
    def_method!(
        edit_limit_buy_order,
        (id: &str, symbol: &str, amount: &str, price: &str, params: Params),
        Order
    );
    def_method!(
        edit_limit_sell_order,
        (id: &str, symbol: &str, amount: &str, price: &str, params: Params),
        Order
    );
    def_method!(
        edit_order_with_client_order_id,
        (
            id: &str,
            symbol: &str,
            order_type: &str,
            side: &str,
            amount: &str,
            price: Option<&str>,
            params: Params
        ),
        Order
    );
    def_method!(cancel_order, (id: &str, symbol: &str, params: Params), Order);
    def_method!(cancel_order_with_client_order_id, (id: &str, params: Params), Order);
    def_method!(
        cancel_orders,
        (ids: &[String], symbol: Option<&str>, params: Params),
        Vec<Order>
    );
    def_method!(
        cancel_orders_with_client_order_ids,
        (ids: &[String], params: Params),
        Vec<Order>
    );
    def_method!(
        cancel_orders_for_symbols,
        (orders: &[serde_json::Value], params: Params),
        Vec<Order>
    );
    def_method!(cancel_all_orders, (symbol: Option<&str>, params: Params), serde_json::Value);
    def_method!(cancel_all_orders_after, (timeout: i64, params: Params), serde_json::Value);
    def_method!(cancel_spot_order, (id: &str, symbol: &str, params: Params), Order);
    def_method!(cancel_contract_order, (id: &str, symbol: &str, params: Params), Order);
    def_method!(
        cancel_all_spot_orders,
        (symbol: Option<&str>, params: Params),
        serde_json::Value
    );
    def_method!(
        cancel_all_contract_orders,
        (symbol: Option<&str>, params: Params),
        serde_json::Value
    );
    def_method!(fetch_order, (id: &str, symbol: &str, params: Params), Order);
    def_method!(fetch_order_with_client_order_id, (id: &str, params: Params), Order);
    def_method!(fetch_order_status, (id: &str, symbol: &str, params: Params), String);
    def_method!(fetch_unified_order, (order: serde_json::Value, params: Params), Order);
    def_method!(
        fetch_orders,
        (symbol: Option<&str>, since: Option<i64>, limit: Option<i64>, params: Params),
        Vec<Order>
    );
    def_method!(
        fetch_open_orders,
        (symbol: Option<&str>, since: Option<i64>, limit: Option<i64>, params: Params),
        Vec<Order>
    );
    def_method!(
        fetch_closed_orders,
        (symbol: Option<&str>, since: Option<i64>, limit: Option<i64>, params: Params),
        Vec<Order>
    );
    def_method!(
        fetch_canceled_orders,
        (symbol: Option<&str>, since: Option<i64>, limit: Option<i64>, params: Params),
        Vec<Order>
    );
    def_method!(
        fetch_canceled_and_closed_orders,
        (symbol: Option<&str>, since: Option<i64>, limit: Option<i64>, params: Params),
        Vec<Order>
    );
    def_method!(
        fetch_order_trades,
        (id: &str, symbol: Option<&str>, since: Option<i64>, limit: Option<i64>, params: Params),
        Vec<Trade>
    );
    def_method!(
        fetch_my_trades,
        (symbol: Option<&str>, since: Option<i64>, limit: Option<i64>, params: Params),
        Vec<Trade>
    );
    def_method!(close_position, (symbol: &str, params: Params), Order);
    def_method!(close_all_positions, (params: Params), Vec<Order>);

    // ================= 账户/资金 =================

    def_method!(fetch_balance, (params: Params), Balances);
    def_method!(fetch_free_balance, (params: Params), Balances);
    def_method!(fetch_used_balance, (params: Params), Balances);
    def_method!(fetch_total_balance, (params: Params), Balances);
    def_method!(
        fetch_ledger,
        (code: Option<&str>, since: Option<i64>, limit: Option<i64>, params: Params),
        Vec<LedgerEntry>
    );
    def_method!(fetch_ledger_entry, (id: &str, params: Params), LedgerEntry);
    def_method!(
        fetch_deposits,
        (code: Option<&str>, since: Option<i64>, limit: Option<i64>, params: Params),
        Vec<Transaction>
    );
    def_method!(
        fetch_withdrawals,
        (code: Option<&str>, since: Option<i64>, limit: Option<i64>, params: Params),
        Vec<Transaction>
    );
    def_method!(
        fetch_deposits_withdrawals,
        (code: Option<&str>, since: Option<i64>, limit: Option<i64>, params: Params),
        Vec<Transaction>
    );
    def_method!(
        fetch_transactions,
        (code: Option<&str>, since: Option<i64>, limit: Option<i64>, params: Params),
        Vec<Transaction>
    );
    def_method!(
        withdraw,
        (code: &str, amount: &str, address: &str, params: Params),
        Transaction
    );
    def_method!(fetch_deposit_address, (code: &str, params: Params), serde_json::Value);
    def_method!(fetch_deposit_addresses, (codes: Option<&[&str]>, params: Params), serde_json::Value);
    def_method!(
        fetch_deposit_addresses_by_network,
        (code: &str, params: Params),
        serde_json::Value
    );
    def_method!(create_deposit_address, (code: &str, params: Params), serde_json::Value);
    def_method!(
        fetch_transfers,
        (code: Option<&str>, since: Option<i64>, limit: Option<i64>, params: Params),
        serde_json::Value
    );
    def_method!(fetch_transfer, (id: &str, code: Option<&str>, params: Params), serde_json::Value);
    def_method!(fetch_transaction_fee, (code: &str, params: Params), serde_json::Value);
    def_method!(fetch_transaction_fees, (codes: Option<&[&str]>, params: Params), serde_json::Value);
    def_method!(fetch_deposit_withdraw_fee, (code: &str, params: Params), serde_json::Value);
    def_method!(
        fetch_deposit_withdraw_fees,
        (codes: Option<&[&str]>, params: Params),
        serde_json::Value
    );
    def_method!(
        transfer,
        (code: &str, amount: &str, from_account: &str, to_account: &str, params: Params),
        serde_json::Value
    );
    def_method!(fetch_accounts, (params: Params), serde_json::Value);
    def_method!(fetch_payment_methods, (params: Params), serde_json::Value);
    def_method!(sign_in, (params: Params), serde_json::Value);
    def_method!(create_sub_account, (params: Params), serde_json::Value);
    def_method!(
        fetch_borrow_interest,
        (code: Option<&str>, symbol: Option<&str>, since: Option<i64>, limit: Option<i64>, params: Params),
        serde_json::Value
    );
    def_method!(
        borrow_margin,
        (code: &str, amount: &str, symbol: Option<&str>, params: Params),
        serde_json::Value
    );
    def_method!(
        repay_margin,
        (code: &str, amount: &str, symbol: Option<&str>, params: Params),
        serde_json::Value
    );
    def_method!(
        borrow_cross_margin,
        (code: &str, amount: &str, params: Params),
        serde_json::Value
    );
    def_method!(
        borrow_isolated_margin,
        (symbol: &str, code: &str, amount: &str, params: Params),
        serde_json::Value
    );
    def_method!(
        repay_cross_margin,
        (code: &str, amount: &str, params: Params),
        serde_json::Value
    );
    def_method!(
        repay_isolated_margin,
        (symbol: &str, code: &str, amount: &str, params: Params),
        serde_json::Value
    );
    def_method!(fetch_cross_borrow_rate, (code: &str, params: Params), serde_json::Value);
    def_method!(fetch_isolated_borrow_rate, (symbol: &str, params: Params), serde_json::Value);
    def_method!(
        fetch_cross_borrow_rates,
        (codes: Option<&[&str]>, params: Params),
        serde_json::Value
    );
    def_method!(
        fetch_isolated_borrow_rates,
        (symbols: Option<&[&str]>, params: Params),
        serde_json::Value
    );
    def_method!(fetch_convert_trade, (id: &str, params: Params), serde_json::Value);
    def_method!(
        fetch_convert_trade_history,
        (since: Option<i64>, limit: Option<i64>, params: Params),
        serde_json::Value
    );
    def_method!(
        fetch_margin_adjustment_history,
        (symbol: Option<&str>, since: Option<i64>, limit: Option<i64>, params: Params),
        serde_json::Value
    );

    // ================= 杠杆/仓位 =================

    def_method!(fetch_position, (symbol: &str, params: Params), Position);
    def_method!(
        fetch_positions,
        (symbols: Option<&[&str]>, params: Params),
        Vec<Position>
    );
    def_method!(fetch_positions_for_symbol, (symbol: &str, params: Params), Vec<Position>);
    def_method!(
        fetch_positions_risk,
        (symbols: Option<&[&str]>, params: Params),
        Vec<Position>
    );
    def_method!(
        fetch_positions_history,
        (symbols: Option<&[&str]>, since: Option<i64>, limit: Option<i64>, params: Params),
        Vec<Position>
    );
    def_method!(
        fetch_position_history,
        (symbol: &str, since: Option<i64>, limit: Option<i64>, params: Params),
        Vec<Position>
    );
    def_method!(set_leverage, (leverage: i64, symbol: &str, params: Params), serde_json::Value);
    def_method!(fetch_leverage, (symbol: &str, params: Params), serde_json::Value);
    def_method!(
        fetch_leverages,
        (symbols: Option<&[&str]>, params: Params),
        serde_json::Value
    );
    def_method!(
        set_margin_mode,
        (margin_mode: &str, symbol: &str, params: Params),
        serde_json::Value
    );
    def_method!(fetch_margin_mode, (symbol: &str, params: Params), serde_json::Value);
    def_method!(
        fetch_margin_modes,
        (symbols: Option<&[&str]>, params: Params),
        serde_json::Value
    );
    def_method!(
        set_position_mode,
        (hedged: bool, symbol: Option<&str>, params: Params),
        serde_json::Value
    );
    def_method!(add_margin, (symbol: &str, amount: &str, params: Params), serde_json::Value);
    def_method!(reduce_margin, (symbol: &str, amount: &str, params: Params), serde_json::Value);
    def_method!(set_margin, (symbol: &str, amount: &str, params: Params), serde_json::Value);
}

/// 实时接口面(ADR-0009):核心 8 频道,默认 `NotSupported`。
#[allow(unused_variables, clippy::too_many_arguments)]
pub trait Realtime: Send + Sync {
    def_method!(watch_ticker, (symbol: &str, params: Params), Ticker);
    def_method!(watch_order_book, (symbol: &str, limit: Option<i64>, params: Params), OrderBook);
    def_method!(
        watch_trades,
        (symbol: &str, since: Option<i64>, limit: Option<i64>, params: Params),
        Vec<Trade>
    );
    def_method!(
        watch_ohlcv,
        (
            symbol: &str,
            timeframe: &str,
            since: Option<i64>,
            limit: Option<i64>,
            params: Params
        ),
        Vec<OHLCV>
    );
    def_method!(watch_balance, (params: Params), Balances);
    def_method!(
        watch_orders,
        (symbol: Option<&str>, since: Option<i64>, limit: Option<i64>, params: Params),
        Vec<Order>
    );
    def_method!(
        watch_my_trades,
        (symbol: Option<&str>, since: Option<i64>, limit: Option<i64>, params: Params),
        Vec<Trade>
    );
    def_method!(
        watch_positions,
        (symbols: Option<&[&str]>, params: Params),
        Vec<Position>
    );
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[tokio::test]
    async fn default_methods_return_not_supported() {
        let ex = Dummy {
            config: Config::new(),
        };
        let err = ex
            .fetch_ticker("BTC/USDT", Params::new())
            .await
            .unwrap_err();
        assert_eq!(err.kind(), crate::error::ErrorKind::NotSupported);
        assert_eq!(err.context.method, Some("fetch_ticker"));
    }
}
