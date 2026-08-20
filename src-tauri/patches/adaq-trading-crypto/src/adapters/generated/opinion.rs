//! Opinion (`opinion`) 转译适配器 — 由 `scripts/gen_adapters.py`
//! 从 ccxt 4.5.73 `describe()` 自动生成(best-effort 批量补齐)。
//! 保留 MIT 声明见仓库 `NOTICE`;精确性由精选手写集保证。
//! 类别: Prediction。

#![allow(clippy::too_many_arguments)]

use crate::generic::{ApiSpec, Endpoint, MarketKind};

/// ccxt `describe()` 静态快照(描述驱动引擎的输入)。
pub static SPEC: ApiSpec = ApiSpec {
    id: "opinion",
    name: "Opinion",
    version: "",
    rate_limit_ms: 67,
    has: &[
        "cancelOrder",
        "createOrder",
        "fetchBalance",
        "fetchClosedOrders",
        "fetchCurrenciesWs",
        "fetchEvent",
        "fetchEvents",
        "fetchMarkets",
        "fetchMyTrades",
        "fetchOHLCV",
        "fetchOpenOrders",
        "fetchOrder",
        "fetchOrderBook",
        "fetchOrders",
        "fetchPositions",
        "fetchTicker",
        "fetchTickers",
        "prediction",
        "privateAPI",
        "publicAPI",
        "watchMyTrades",
        "watchOrderBook",
        "watchOrders",
        "watchTicker",
        "watchTrades",
    ],
    endpoints: &[
        Endpoint {
            base: "https://openapi.opinion.trade/openapi",
            verb: "GET",
            key: "token/orderbook",
            path: "token/orderbook",
            auth: false,
        },
        Endpoint {
            base: "https://openapi.opinion.trade/openapi",
            verb: "GET",
            key: "order",
            path: "order",
            auth: true,
        },
        Endpoint {
            base: "https://openapi.opinion.trade/openapi",
            verb: "GET",
            key: "order/{orderId}",
            path: "order/{orderId}",
            auth: true,
        },
        Endpoint {
            base: "https://openapi.opinion.trade/openapi",
            verb: "GET",
            key: "trade/user/{walletAddress}",
            path: "trade/user/{walletAddress}",
            auth: true,
        },
        Endpoint {
            base: "https://openapi.opinion.trade/openapi",
            verb: "GET",
            key: "user/balance",
            path: "user/balance",
            auth: true,
        },
        Endpoint {
            base: "https://openapi.opinion.trade/openapi",
            verb: "POST",
            key: "order",
            path: "order",
            auth: true,
        },
        Endpoint {
            base: "https://openapi.opinion.trade/openapi",
            verb: "POST",
            key: "order/cancel",
            path: "order/cancel",
            auth: true,
        },
    ],
    taker: 0.04,
    maker: -0.02,
    timeframes: &["1h", "1d"],
    kind: MarketKind::Prediction,
};

crate::impl_generated_adapter!(Opinion, &SPEC);
