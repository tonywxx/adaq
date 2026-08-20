//! CoinMate (`coinmate`) 转译适配器 — 由 `scripts/gen_adapters.py`
//! 从 ccxt 4.5.73 `describe()` 自动生成(best-effort 批量补齐)。
//! 保留 MIT 声明见仓库 `NOTICE`;精确性由精选手写集保证。
//! 类别: Cex。

#![allow(clippy::too_many_arguments)]

use crate::generic::{ApiSpec, Endpoint, MarketKind};

/// ccxt `describe()` 静态快照(描述驱动引擎的输入)。
pub static SPEC: ApiSpec = ApiSpec {
    id: "coinmate",
    name: "CoinMate",
    version: "",
    rate_limit_ms: 600,
    has: &[
        "CORS",
        "cancelOrder",
        "createLimitOrder",
        "createMarketOrder",
        "createMarketOrderWs",
        "createOrder",
        "editOrder",
        "fetchBalance",
        "fetchCurrencies",
        "fetchCurrenciesWs",
        "fetchDepositsWithdrawals",
        "fetchL2OrderBook",
        "fetchMarkets",
        "fetchMyTrades",
        "fetchOpenOrders",
        "fetchOrder",
        "fetchOrderBook",
        "fetchOrders",
        "fetchTicker",
        "fetchTickers",
        "fetchTime",
        "fetchTrades",
        "fetchTradingFee",
        "fetchTransactions",
        "privateAPI",
        "publicAPI",
        "spot",
        "withdraw",
    ],
    endpoints: &[
        Endpoint {
            base: "https://coinmate.io/api",
            verb: "GET",
            key: "orderBook",
            path: "orderBook",
            auth: false,
        },
        Endpoint {
            base: "https://coinmate.io/api",
            verb: "GET",
            key: "ticker",
            path: "ticker",
            auth: false,
        },
        Endpoint {
            base: "https://coinmate.io/api",
            verb: "GET",
            key: "system/time",
            path: "system/time",
            auth: false,
        },
        Endpoint {
            base: "https://coinmate.io/api",
            verb: "POST",
            key: "currencies",
            path: "currencies",
            auth: true,
        },
        Endpoint {
            base: "https://coinmate.io/api",
            verb: "POST",
            key: "balances",
            path: "balances",
            auth: true,
        },
        Endpoint {
            base: "https://coinmate.io/api",
            verb: "POST",
            key: "openOrders",
            path: "openOrders",
            auth: true,
        },
        Endpoint {
            base: "https://coinmate.io/api",
            verb: "POST",
            key: "order",
            path: "order",
            auth: true,
        },
    ],
    taker: 0.006,
    maker: 0.004,
    timeframes: &[],
    kind: MarketKind::Cex,
};

crate::impl_generated_adapter!(Coinmate, &SPEC);
