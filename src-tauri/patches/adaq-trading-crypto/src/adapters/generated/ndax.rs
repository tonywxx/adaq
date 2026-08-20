//! NDAX (`ndax`) 转译适配器 — 由 `scripts/gen_adapters.py`
//! 从 ccxt 4.5.73 `describe()` 自动生成(best-effort 批量补齐)。
//! 保留 MIT 声明见仓库 `NOTICE`;精确性由精选手写集保证。
//! 类别: Cex。

#![allow(clippy::too_many_arguments)]

use crate::generic::{ApiSpec, Endpoint, MarketKind};

/// ccxt `describe()` 静态快照(描述驱动引擎的输入)。
pub static SPEC: ApiSpec = ApiSpec {
    id: "ndax",
    name: "NDAX",
    version: "",
    rate_limit_ms: 1000,
    has: &[
        "cancelAllOrders",
        "cancelOrder",
        "createDepositAddress",
        "createLimitOrder",
        "createMarketOrder",
        "createMarketOrderWs",
        "createOrder",
        "createStopLimitOrder",
        "createStopMarketOrder",
        "createStopOrder",
        "editOrder",
        "fetchAccounts",
        "fetchBalance",
        "fetchCurrencies",
        "fetchCurrenciesWs",
        "fetchDepositAddress",
        "fetchDeposits",
        "fetchL2OrderBook",
        "fetchLedger",
        "fetchMarkets",
        "fetchMyTrades",
        "fetchOHLCV",
        "fetchOpenOrders",
        "fetchOrder",
        "fetchOrderBook",
        "fetchOrderTrades",
        "fetchOrders",
        "fetchStatus",
        "fetchTicker",
        "fetchTickers",
        "fetchTrades",
        "fetchWithdrawals",
        "privateAPI",
        "publicAPI",
        "sandbox",
        "signIn",
        "spot",
        "withdraw",
    ],
    endpoints: &[
        Endpoint {
            base: "https://api.ndax.io:8443/AP",
            verb: "GET",
            key: "GetInstruments",
            path: "GetInstruments",
            auth: false,
        },
        Endpoint {
            base: "https://api.ndax.io:8443/AP",
            verb: "GET",
            key: "Ping",
            path: "Ping",
            auth: false,
        },
        Endpoint {
            base: "https://api.ndax.io:8443/AP",
            verb: "GET",
            key: "assets",
            path: "assets",
            auth: false,
        },
        Endpoint {
            base: "https://api.ndax.io:8443/AP",
            verb: "GET",
            key: "orderbook",
            path: "orderbook",
            auth: false,
        },
        Endpoint {
            base: "https://api.ndax.io:8443/AP",
            verb: "GET",
            key: "ticker",
            path: "ticker",
            auth: false,
        },
        Endpoint {
            base: "https://api.ndax.io:8443/AP",
            verb: "GET",
            key: "trades",
            path: "trades",
            auth: false,
        },
        Endpoint {
            base: "https://api.ndax.io:8443/AP",
            verb: "GET",
            key: "GetOpenOrders",
            path: "GetOpenOrders",
            auth: true,
        },
    ],
    taker: 0.0025,
    maker: 0.002,
    timeframes: &[
        "1m", "5m", "15m", "30m", "1h", "2h", "4h", "6h", "12h", "1d", "1w", "1M", "4M",
    ],
    kind: MarketKind::Cex,
};

crate::impl_generated_adapter!(Ndax, &SPEC);
