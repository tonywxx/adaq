//! p2b (`p2b`) 转译适配器 — 由 `scripts/gen_adapters.py`
//! 从 ccxt 4.5.73 `describe()` 自动生成(best-effort 批量补齐)。
//! 保留 MIT 声明见仓库 `NOTICE`;精确性由精选手写集保证。
//! 类别: Cex。

#![allow(clippy::too_many_arguments)]

use crate::generic::{ApiSpec, Endpoint, MarketKind};

/// ccxt `describe()` 静态快照(描述驱动引擎的输入)。
pub static SPEC: ApiSpec = ApiSpec {
    id: "p2b",
    name: "p2b",
    version: "v2",
    rate_limit_ms: 100,
    has: &[
        "cancelOrder",
        "createLimitOrder",
        "createMarketOrderWs",
        "createOrder",
        "editOrder",
        "fetchBalance",
        "fetchClosedOrders",
        "fetchCurrencies",
        "fetchCurrenciesWs",
        "fetchL2OrderBook",
        "fetchMarkets",
        "fetchMyTrades",
        "fetchOHLCV",
        "fetchOpenOrders",
        "fetchOrderBook",
        "fetchOrderTrades",
        "fetchTicker",
        "fetchTickers",
        "fetchTrades",
        "privateAPI",
        "publicAPI",
        "spot",
    ],
    endpoints: &[
        Endpoint {
            base: "https://api.p2pb2b.com/api/v2/public",
            verb: "GET",
            key: "markets",
            path: "markets",
            auth: false,
        },
        Endpoint {
            base: "https://api.p2pb2b.com/api/v2/public",
            verb: "GET",
            key: "tickers",
            path: "tickers",
            auth: false,
        },
        Endpoint {
            base: "https://api.p2pb2b.com/api/v2/public",
            verb: "GET",
            key: "ticker",
            path: "ticker",
            auth: false,
        },
        Endpoint {
            base: "https://api.p2pb2b.com/api/v2/public",
            verb: "GET",
            key: "book",
            path: "book",
            auth: false,
        },
        Endpoint {
            base: "https://api.p2pb2b.com/api/v2/public",
            verb: "GET",
            key: "depth/result",
            path: "depth/result",
            auth: false,
        },
        Endpoint {
            base: "https://api.p2pb2b.com/api/v2/public",
            verb: "GET",
            key: "market/kline",
            path: "market/kline",
            auth: false,
        },
        Endpoint {
            base: "https://api.p2pb2b.com/api/v2",
            verb: "POST",
            key: "account/balances",
            path: "account/balances",
            auth: true,
        },
        Endpoint {
            base: "https://api.p2pb2b.com/api/v2",
            verb: "POST",
            key: "account/balance",
            path: "account/balance",
            auth: true,
        },
        Endpoint {
            base: "https://api.p2pb2b.com/api/v2",
            verb: "POST",
            key: "order/new",
            path: "order/new",
            auth: true,
        },
        Endpoint {
            base: "https://api.p2pb2b.com/api/v2",
            verb: "POST",
            key: "order/cancel",
            path: "order/cancel",
            auth: true,
        },
        Endpoint {
            base: "https://api.p2pb2b.com/api/v2",
            verb: "POST",
            key: "orders",
            path: "orders",
            auth: true,
        },
        Endpoint {
            base: "https://api.p2pb2b.com/api/v2",
            verb: "POST",
            key: "account/market_order_history",
            path: "account/market_order_history",
            auth: true,
        },
        Endpoint {
            base: "https://api.p2pb2b.com/api/v2",
            verb: "POST",
            key: "account/market_deal_history",
            path: "account/market_deal_history",
            auth: true,
        },
        Endpoint {
            base: "https://api.p2pb2b.com/api/v2",
            verb: "POST",
            key: "account/order",
            path: "account/order",
            auth: true,
        },
        Endpoint {
            base: "https://api.p2pb2b.com/api/v2",
            verb: "POST",
            key: "account/order_history",
            path: "account/order_history",
            auth: true,
        },
        Endpoint {
            base: "https://api.p2pb2b.com/api/v2",
            verb: "POST",
            key: "account/executed_history",
            path: "account/executed_history",
            auth: true,
        },
    ],
    taker: 0.2,
    maker: 0.2,
    timeframes: &["1m", "1h", "1d"],
    kind: MarketKind::Cex,
};

crate::impl_generated_adapter!(P2b, &SPEC);
