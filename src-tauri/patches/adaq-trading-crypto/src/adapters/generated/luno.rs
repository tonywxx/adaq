//! Luno (`luno`) 转译适配器 — 由 `scripts/gen_adapters.py`
//! 从 ccxt 4.5.73 `describe()` 自动生成(best-effort 批量补齐)。
//! 保留 MIT 声明见仓库 `NOTICE`;精确性由精选手写集保证。
//! 类别: Cex。

#![allow(clippy::too_many_arguments)]

use crate::generic::{ApiSpec, Endpoint, MarketKind};

/// ccxt `describe()` 静态快照(描述驱动引擎的输入)。
pub static SPEC: ApiSpec = ApiSpec {
    id: "luno",
    name: "Luno",
    version: "1",
    rate_limit_ms: 200,
    has: &[
        "cancelOrder",
        "createDepositAddress",
        "createLimitOrder",
        "createMarketOrder",
        "createMarketOrderWs",
        "createOrder",
        "editOrder",
        "fetchAccounts",
        "fetchBalance",
        "fetchClosedOrders",
        "fetchCurrencies",
        "fetchCurrenciesWs",
        "fetchDepositAddress",
        "fetchDepositWithdrawFee",
        "fetchL2OrderBook",
        "fetchLedger",
        "fetchMarkets",
        "fetchMyTrades",
        "fetchOHLCV",
        "fetchOpenOrders",
        "fetchOrder",
        "fetchOrderBook",
        "fetchOrders",
        "fetchTicker",
        "fetchTickers",
        "fetchTrades",
        "fetchTradingFee",
        "privateAPI",
        "publicAPI",
        "spot",
    ],
    endpoints: &[
        Endpoint {
            base: "https://api.luno.com/api/exchange",
            verb: "GET",
            key: "markets",
            path: "markets",
            auth: true,
        },
        Endpoint {
            base: "https://api.luno.com/api/exchange",
            verb: "GET",
            key: "candles",
            path: "candles",
            auth: true,
        },
        Endpoint {
            base: "https://api.luno.com/api",
            verb: "GET",
            key: "orderbook",
            path: "orderbook",
            auth: false,
        },
        Endpoint {
            base: "https://api.luno.com/api",
            verb: "GET",
            key: "orderbook_top",
            path: "orderbook_top",
            auth: false,
        },
        Endpoint {
            base: "https://api.luno.com/api",
            verb: "GET",
            key: "ticker",
            path: "ticker",
            auth: false,
        },
        Endpoint {
            base: "https://api.luno.com/api",
            verb: "GET",
            key: "tickers",
            path: "tickers",
            auth: false,
        },
        Endpoint {
            base: "https://api.luno.com/api",
            verb: "GET",
            key: "trades",
            path: "trades",
            auth: false,
        },
        Endpoint {
            base: "https://api.luno.com/api",
            verb: "GET",
            key: "balance",
            path: "balance",
            auth: true,
        },
        Endpoint {
            base: "https://api.luno.com/api",
            verb: "GET",
            key: "listorders",
            path: "listorders",
            auth: true,
        },
        Endpoint {
            base: "https://api.luno.com/api",
            verb: "GET",
            key: "listtrades",
            path: "listtrades",
            auth: true,
        },
        Endpoint {
            base: "https://api.luno.com/api",
            verb: "GET",
            key: "orders/{id}",
            path: "orders/{id}",
            auth: true,
        },
    ],
    taker: 0.006,
    maker: 0.004,
    timeframes: &["1m", "5m", "15m", "30m", "1h", "3h", "4h", "1d", "3d", "1w"],
    kind: MarketKind::Cex,
};

crate::impl_generated_adapter!(Luno, &SPEC);
