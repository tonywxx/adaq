//! Paymium (`paymium`) 转译适配器 — 由 `scripts/gen_adapters.py`
//! 从 ccxt 4.5.73 `describe()` 自动生成(best-effort 批量补齐)。
//! 保留 MIT 声明见仓库 `NOTICE`;精确性由精选手写集保证。
//! 类别: Cex。

#![allow(clippy::too_many_arguments)]

use crate::generic::{ApiSpec, Endpoint, MarketKind};

/// ccxt `describe()` 静态快照(描述驱动引擎的输入)。
pub static SPEC: ApiSpec = ApiSpec {
    id: "paymium",
    name: "Paymium",
    version: "v1",
    rate_limit_ms: 2000,
    has: &[
        "CORS",
        "cancelOrder",
        "createDepositAddress",
        "createLimitOrder",
        "createMarketOrder",
        "createMarketOrderWs",
        "createOrder",
        "editOrder",
        "fetchBalance",
        "fetchCurrencies",
        "fetchCurrenciesWs",
        "fetchDepositAddress",
        "fetchDepositAddresses",
        "fetchL2OrderBook",
        "fetchMarkets",
        "fetchOrderBook",
        "fetchTicker",
        "fetchTrades",
        "privateAPI",
        "publicAPI",
        "spot",
        "transfer",
    ],
    endpoints: &[
        Endpoint {
            base: "https://paymium.com/api",
            verb: "GET",
            key: "currencies",
            path: "currencies",
            auth: false,
        },
        Endpoint {
            base: "https://paymium.com/api",
            verb: "GET",
            key: "data/{currency}/ticker",
            path: "data/{currency}/ticker",
            auth: false,
        },
        Endpoint {
            base: "https://paymium.com/api",
            verb: "GET",
            key: "data/{currency}/trades",
            path: "data/{currency}/trades",
            auth: false,
        },
        Endpoint {
            base: "https://paymium.com/api",
            verb: "GET",
            key: "data/{currency}/depth",
            path: "data/{currency}/depth",
            auth: false,
        },
        Endpoint {
            base: "https://paymium.com/api",
            verb: "GET",
            key: "bitcoin_charts/{id}/trades",
            path: "bitcoin_charts/{id}/trades",
            auth: false,
        },
        Endpoint {
            base: "https://paymium.com/api",
            verb: "GET",
            key: "bitcoin_charts/{id}/depth",
            path: "bitcoin_charts/{id}/depth",
            auth: false,
        },
        Endpoint {
            base: "https://paymium.com/api",
            verb: "GET",
            key: "user/orders",
            path: "user/orders",
            auth: true,
        },
        Endpoint {
            base: "https://paymium.com/api",
            verb: "GET",
            key: "user/orders/{uuid}",
            path: "user/orders/{uuid}",
            auth: true,
        },
        Endpoint {
            base: "https://paymium.com/api",
            verb: "POST",
            key: "user/orders",
            path: "user/orders",
            auth: true,
        },
        Endpoint {
            base: "https://paymium.com/api",
            verb: "DELETE",
            key: "user/orders/{uuid}",
            path: "user/orders/{uuid}",
            auth: true,
        },
        Endpoint {
            base: "https://paymium.com/api",
            verb: "DELETE",
            key: "user/orders/{uuid}/cancel",
            path: "user/orders/{uuid}/cancel",
            auth: true,
        },
    ],
    taker: 0.005,
    maker: -0.001,
    timeframes: &[],
    kind: MarketKind::Cex,
};

crate::impl_generated_adapter!(Paymium, &SPEC);
