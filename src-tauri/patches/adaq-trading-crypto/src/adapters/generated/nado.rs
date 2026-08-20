//! Nado (`nado`) 转译适配器 — 由 `scripts/gen_adapters.py`
//! 从 ccxt 4.5.73 `describe()` 自动生成(best-effort 批量补齐)。
//! 保留 MIT 声明见仓库 `NOTICE`;精确性由精选手写集保证。
//! 类别: Dex。

#![allow(clippy::too_many_arguments)]

use crate::generic::{ApiSpec, Endpoint, MarketKind};

/// ccxt `describe()` 静态快照(描述驱动引擎的输入)。
pub static SPEC: ApiSpec = ApiSpec {
    id: "nado",
    name: "Nado",
    version: "v1",
    rate_limit_ms: 25,
    has: &[
        "cancelAllOrders",
        "cancelOrder",
        "cancelOrders",
        "createLimitOrder",
        "createMarketOrder",
        "createMarketOrderWs",
        "createOrder",
        "editOrder",
        "fetchBalance",
        "fetchCanceledAndClosedOrders",
        "fetchCanceledOrders",
        "fetchClosedOrders",
        "fetchCurrencies",
        "fetchCurrenciesWs",
        "fetchDeposits",
        "fetchFundingHistory",
        "fetchFundingRate",
        "fetchFundingRates",
        "fetchL2OrderBook",
        "fetchMarkets",
        "fetchMyTrades",
        "fetchOHLCV",
        "fetchOpenInterest",
        "fetchOpenInterests",
        "fetchOpenOrders",
        "fetchOrder",
        "fetchOrderBook",
        "fetchOrders",
        "fetchPositions",
        "fetchStatus",
        "fetchTicker",
        "fetchTickers",
        "fetchTime",
        "fetchTrades",
        "fetchWithdrawals",
        "margin",
        "privateAPI",
        "publicAPI",
        "spot",
        "swap",
    ],
    endpoints: &[
        Endpoint {
            base: "https://gateway.prod.nado.xyz/v1",
            verb: "GET",
            key: "symbols",
            path: "symbols",
            auth: false,
        },
        Endpoint {
            base: "https://gateway.prod.nado.xyz/v2",
            verb: "GET",
            key: "assets",
            path: "assets",
            auth: false,
        },
        Endpoint {
            base: "https://gateway.prod.nado.xyz/v2",
            verb: "GET",
            key: "pairs",
            path: "pairs",
            auth: false,
        },
        Endpoint {
            base: "https://gateway.prod.nado.xyz/v2",
            verb: "GET",
            key: "orderbook",
            path: "orderbook",
            auth: false,
        },
        Endpoint {
            base: "https://archive.prod.nado.xyz/v2",
            verb: "GET",
            key: "tickers",
            path: "tickers",
            auth: false,
        },
        Endpoint {
            base: "https://archive.prod.nado.xyz/v2",
            verb: "GET",
            key: "trades",
            path: "trades",
            auth: false,
        },
    ],
    taker: 0.00035,
    maker: 0.0001,
    timeframes: &["1m", "5m", "15m", "1h", "2h", "4h", "1d", "1w", "4w"],
    kind: MarketKind::Dex,
};

crate::impl_generated_adapter!(Nado, &SPEC);
