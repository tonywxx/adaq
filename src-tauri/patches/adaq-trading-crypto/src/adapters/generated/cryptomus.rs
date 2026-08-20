//! Cryptomus (`cryptomus`) 转译适配器 — 由 `scripts/gen_adapters.py`
//! 从 ccxt 4.5.73 `describe()` 自动生成(best-effort 批量补齐)。
//! 保留 MIT 声明见仓库 `NOTICE`;精确性由精选手写集保证。
//! 类别: Cex。

#![allow(clippy::too_many_arguments)]

use crate::generic::{ApiSpec, Endpoint, MarketKind};

/// ccxt `describe()` 静态快照(描述驱动引擎的输入)。
pub static SPEC: ApiSpec = ApiSpec {
    id: "cryptomus",
    name: "Cryptomus",
    version: "v2",
    rate_limit_ms: 100,
    has: &[
        "cancelOrder",
        "createLimitOrder",
        "createMarketOrderWs",
        "createOrder",
        "editOrder",
        "fetchBalance",
        "fetchCanceledAndClosedOrders",
        "fetchCurrencies",
        "fetchCurrenciesWs",
        "fetchL2OrderBook",
        "fetchMarkets",
        "fetchOpenOrders",
        "fetchOrder",
        "fetchOrderBook",
        "fetchTickers",
        "fetchTrades",
        "fetchTradingFees",
        "privateAPI",
        "publicAPI",
        "spot",
    ],
    endpoints: &[
        Endpoint {
            base: "https://api.cryptomus.com",
            verb: "GET",
            key: "v2/user-api/exchange/markets",
            path: "v2/user-api/exchange/markets",
            auth: false,
        },
        Endpoint {
            base: "https://api.cryptomus.com",
            verb: "GET",
            key: "v1/exchange/market/assets",
            path: "v1/exchange/market/assets",
            auth: false,
        },
        Endpoint {
            base: "https://api.cryptomus.com",
            verb: "GET",
            key: "v1/exchange/market/order-book/{currencyPair}",
            path: "v1/exchange/market/order-book/{currencyPair}",
            auth: false,
        },
        Endpoint {
            base: "https://api.cryptomus.com",
            verb: "GET",
            key: "v1/exchange/market/tickers",
            path: "v1/exchange/market/tickers",
            auth: false,
        },
        Endpoint {
            base: "https://api.cryptomus.com",
            verb: "GET",
            key: "v1/exchange/market/trades/{currencyPair}",
            path: "v1/exchange/market/trades/{currencyPair}",
            auth: false,
        },
        Endpoint {
            base: "https://api.cryptomus.com",
            verb: "GET",
            key: "v2/user-api/exchange/orders",
            path: "v2/user-api/exchange/orders",
            auth: true,
        },
        Endpoint {
            base: "https://api.cryptomus.com",
            verb: "GET",
            key: "v2/user-api/exchange/orders/history",
            path: "v2/user-api/exchange/orders/history",
            auth: true,
        },
        Endpoint {
            base: "https://api.cryptomus.com",
            verb: "GET",
            key: "v2/user-api/exchange/account/balance",
            path: "v2/user-api/exchange/account/balance",
            auth: true,
        },
        Endpoint {
            base: "https://api.cryptomus.com",
            verb: "GET",
            key: "v2/user-api/exchange/account/tariffs",
            path: "v2/user-api/exchange/account/tariffs",
            auth: true,
        },
        Endpoint {
            base: "https://api.cryptomus.com",
            verb: "POST",
            key: "v2/user-api/exchange/orders",
            path: "v2/user-api/exchange/orders",
            auth: true,
        },
        Endpoint {
            base: "https://api.cryptomus.com",
            verb: "POST",
            key: "v2/user-api/exchange/orders/market",
            path: "v2/user-api/exchange/orders/market",
            auth: true,
        },
        Endpoint {
            base: "https://api.cryptomus.com",
            verb: "DELETE",
            key: "v2/user-api/exchange/orders/{orderId}",
            path: "v2/user-api/exchange/orders/{orderId}",
            auth: true,
        },
    ],
    taker: 0.02,
    maker: 0.02,
    timeframes: &[],
    kind: MarketKind::Cex,
};

crate::impl_generated_adapter!(Cryptomus, &SPEC);
