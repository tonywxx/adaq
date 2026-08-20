//! Coincheck (`coincheck`) 转译适配器 — 由 `scripts/gen_adapters.py`
//! 从 ccxt 4.5.73 `describe()` 自动生成(best-effort 批量补齐)。
//! 保留 MIT 声明见仓库 `NOTICE`;精确性由精选手写集保证。
//! 类别: Cex。

#![allow(clippy::too_many_arguments)]

use crate::generic::{ApiSpec, Endpoint, MarketKind};

/// ccxt `describe()` 静态快照(描述驱动引擎的输入)。
pub static SPEC: ApiSpec = ApiSpec {
    id: "coincheck",
    name: "Coincheck",
    version: "",
    rate_limit_ms: 1500,
    has: &[
        "cancelOrder",
        "createLimitOrder",
        "createMarketOrder",
        "createMarketOrderWs",
        "createOrder",
        "editOrder",
        "fetchBalance",
        "fetchCurrencies",
        "fetchCurrenciesWs",
        "fetchDeposits",
        "fetchL2OrderBook",
        "fetchMarkets",
        "fetchMyTrades",
        "fetchOpenOrders",
        "fetchOrderBook",
        "fetchStatus",
        "fetchTicker",
        "fetchTrades",
        "fetchTradingFees",
        "fetchWithdrawals",
        "privateAPI",
        "publicAPI",
        "spot",
        "ws",
    ],
    endpoints: &[
        Endpoint {
            base: "https://coincheck.com/api",
            verb: "GET",
            key: "exchange/orders/rate",
            path: "exchange/orders/rate",
            auth: false,
        },
        Endpoint {
            base: "https://coincheck.com/api",
            verb: "GET",
            key: "exchange_status",
            path: "exchange_status",
            auth: false,
        },
        Endpoint {
            base: "https://coincheck.com/api",
            verb: "GET",
            key: "order_books",
            path: "order_books",
            auth: false,
        },
        Endpoint {
            base: "https://coincheck.com/api",
            verb: "GET",
            key: "ticker",
            path: "ticker",
            auth: false,
        },
        Endpoint {
            base: "https://coincheck.com/api",
            verb: "GET",
            key: "trades",
            path: "trades",
            auth: false,
        },
        Endpoint {
            base: "https://coincheck.com/api",
            verb: "GET",
            key: "accounts/balance",
            path: "accounts/balance",
            auth: true,
        },
        Endpoint {
            base: "https://coincheck.com/api",
            verb: "GET",
            key: "accounts/leverage_balance",
            path: "accounts/leverage_balance",
            auth: true,
        },
        Endpoint {
            base: "https://coincheck.com/api",
            verb: "GET",
            key: "exchange/orders/{id}",
            path: "exchange/orders/{id}",
            auth: true,
        },
        Endpoint {
            base: "https://coincheck.com/api",
            verb: "GET",
            key: "exchange/orders/opens",
            path: "exchange/orders/opens",
            auth: true,
        },
        Endpoint {
            base: "https://coincheck.com/api",
            verb: "GET",
            key: "exchange/orders/cancel_status",
            path: "exchange/orders/cancel_status",
            auth: true,
        },
        Endpoint {
            base: "https://coincheck.com/api",
            verb: "GET",
            key: "exchange/orders/transactions",
            path: "exchange/orders/transactions",
            auth: true,
        },
        Endpoint {
            base: "https://coincheck.com/api",
            verb: "GET",
            key: "exchange/orders/transactions_pagination",
            path: "exchange/orders/transactions_pagination",
            auth: true,
        },
        Endpoint {
            base: "https://coincheck.com/api",
            verb: "POST",
            key: "exchange/orders",
            path: "exchange/orders",
            auth: true,
        },
        Endpoint {
            base: "https://coincheck.com/api",
            verb: "DELETE",
            key: "exchange/orders/{id}",
            path: "exchange/orders/{id}",
            auth: true,
        },
    ],
    taker: 0.0,
    maker: 0.0,
    timeframes: &[],
    kind: MarketKind::Cex,
};

crate::impl_generated_adapter!(Coincheck, &SPEC);
