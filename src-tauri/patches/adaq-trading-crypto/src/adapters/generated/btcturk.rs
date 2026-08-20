//! BTCTurk (`btcturk`) 转译适配器 — 由 `scripts/gen_adapters.py`
//! 从 ccxt 4.5.73 `describe()` 自动生成(best-effort 批量补齐)。
//! 保留 MIT 声明见仓库 `NOTICE`;精确性由精选手写集保证。
//! 类别: Cex。

#![allow(clippy::too_many_arguments)]

use crate::generic::{ApiSpec, Endpoint, MarketKind};

/// ccxt `describe()` 静态快照(描述驱动引擎的输入)。
pub static SPEC: ApiSpec = ApiSpec {
    id: "btcturk",
    name: "BTCTurk",
    version: "",
    rate_limit_ms: 100,
    has: &[
        "CORS",
        "cancelOrder",
        "createLimitOrder",
        "createMarketOrder",
        "createMarketOrderWs",
        "createOrder",
        "editOrder",
        "fetchBalance",
        "fetchCurrenciesWs",
        "fetchL2OrderBook",
        "fetchMarkets",
        "fetchMyTrades",
        "fetchOHLCV",
        "fetchOpenOrders",
        "fetchOrderBook",
        "fetchOrders",
        "fetchTicker",
        "fetchTickers",
        "fetchTrades",
        "privateAPI",
        "publicAPI",
        "spot",
    ],
    endpoints: &[
        Endpoint {
            base: "https://api.btcturk.com/api/v2",
            verb: "GET",
            key: "orderbook",
            path: "orderbook",
            auth: false,
        },
        Endpoint {
            base: "https://api.btcturk.com/api/v2",
            verb: "GET",
            key: "ticker",
            path: "ticker",
            auth: false,
        },
        Endpoint {
            base: "https://api.btcturk.com/api/v2",
            verb: "GET",
            key: "trades",
            path: "trades",
            auth: false,
        },
        Endpoint {
            base: "https://api.btcturk.com/api/v2",
            verb: "GET",
            key: "ohlc",
            path: "ohlc",
            auth: false,
        },
        Endpoint {
            base: "https://api.btcturk.com/api/v2",
            verb: "GET",
            key: "server/exchangeinfo",
            path: "server/exchangeinfo",
            auth: false,
        },
        Endpoint {
            base: "https://api.btcturk.com/api/v1",
            verb: "GET",
            key: "users/balances",
            path: "users/balances",
            auth: true,
        },
        Endpoint {
            base: "https://api.btcturk.com/api/v1",
            verb: "GET",
            key: "openOrders",
            path: "openOrders",
            auth: true,
        },
        Endpoint {
            base: "https://api.btcturk.com/api/v1",
            verb: "GET",
            key: "users/transactions/trade",
            path: "users/transactions/trade",
            auth: true,
        },
        Endpoint {
            base: "https://api.btcturk.com/api/v1",
            verb: "POST",
            key: "order",
            path: "order",
            auth: true,
        },
        Endpoint {
            base: "https://api.btcturk.com/api/v1",
            verb: "DELETE",
            key: "order",
            path: "order",
            auth: true,
        },
        Endpoint {
            base: "https://graph-api.btcturk.com/v1",
            verb: "GET",
            key: "klines/history",
            path: "klines/history",
            auth: true,
        },
    ],
    taker: 0.0009,
    maker: 0.0005,
    timeframes: &["1m", "15m", "30m", "1h", "4h", "1d", "1w", "1y"],
    kind: MarketKind::Cex,
};

crate::impl_generated_adapter!(Btcturk, &SPEC);
