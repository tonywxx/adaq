//! INDODAX (`indodax`) 转译适配器 — 由 `scripts/gen_adapters.py`
//! 从 ccxt 4.5.73 `describe()` 自动生成(best-effort 批量补齐)。
//! 保留 MIT 声明见仓库 `NOTICE`;精确性由精选手写集保证。
//! 类别: Cex。

#![allow(clippy::too_many_arguments)]

use crate::generic::{ApiSpec, Endpoint, MarketKind};

/// ccxt `describe()` 静态快照(描述驱动引擎的输入)。
pub static SPEC: ApiSpec = ApiSpec {
    id: "indodax",
    name: "INDODAX",
    version: "2.0",
    rate_limit_ms: 50,
    has: &[
        "cancelOrder",
        "createLimitOrder",
        "createMarketOrder",
        "createMarketOrderWs",
        "createOrder",
        "editOrder",
        "fetchBalance",
        "fetchClosedOrders",
        "fetchCurrenciesWs",
        "fetchDepositAddress",
        "fetchDepositAddresses",
        "fetchDepositWithdrawFee",
        "fetchDepositsWithdrawals",
        "fetchL2OrderBook",
        "fetchMarkets",
        "fetchOHLCV",
        "fetchOpenOrders",
        "fetchOrder",
        "fetchOrderBook",
        "fetchTicker",
        "fetchTickers",
        "fetchTime",
        "fetchTrades",
        "fetchTransactionFee",
        "fetchTransactions",
        "privateAPI",
        "publicAPI",
        "spot",
        "withdraw",
    ],
    endpoints: &[
        Endpoint {
            base: "https://indodax.com",
            verb: "GET",
            key: "api/server_time",
            path: "api/server_time",
            auth: false,
        },
        Endpoint {
            base: "https://indodax.com",
            verb: "GET",
            key: "api/pairs",
            path: "api/pairs",
            auth: false,
        },
        Endpoint {
            base: "https://indodax.com",
            verb: "GET",
            key: "api/ticker/{pair}",
            path: "api/ticker/{pair}",
            auth: false,
        },
        Endpoint {
            base: "https://indodax.com",
            verb: "GET",
            key: "api/ticker_all",
            path: "api/ticker_all",
            auth: false,
        },
        Endpoint {
            base: "https://indodax.com",
            verb: "GET",
            key: "api/trades/{pair}",
            path: "api/trades/{pair}",
            auth: false,
        },
        Endpoint {
            base: "https://indodax.com",
            verb: "GET",
            key: "api/depth/{pair}",
            path: "api/depth/{pair}",
            auth: false,
        },
        Endpoint {
            base: "https://indodax.com/tapi",
            verb: "POST",
            key: "trade",
            path: "trade",
            auth: true,
        },
        Endpoint {
            base: "https://indodax.com/tapi",
            verb: "POST",
            key: "openOrders",
            path: "openOrders",
            auth: true,
        },
        Endpoint {
            base: "https://indodax.com/tapi",
            verb: "POST",
            key: "getOrder",
            path: "getOrder",
            auth: true,
        },
    ],
    taker: 0.003,
    maker: 0.0,
    timeframes: &["1m", "15m", "30m", "1h", "4h", "1d", "3d", "1w"],
    kind: MarketKind::Cex,
};

crate::impl_generated_adapter!(Indodax, &SPEC);
