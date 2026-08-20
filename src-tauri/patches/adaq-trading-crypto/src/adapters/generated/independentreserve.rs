//! Independent Reserve (`independentreserve`) 转译适配器 — 由 `scripts/gen_adapters.py`
//! 从 ccxt 4.5.73 `describe()` 自动生成(best-effort 批量补齐)。
//! 保留 MIT 声明见仓库 `NOTICE`;精确性由精选手写集保证。
//! 类别: Cex。

#![allow(clippy::too_many_arguments)]

use crate::generic::{ApiSpec, Endpoint, MarketKind};

/// ccxt `describe()` 静态快照(描述驱动引擎的输入)。
pub static SPEC: ApiSpec = ApiSpec {
    id: "independentreserve",
    name: "Independent Reserve",
    version: "",
    rate_limit_ms: 1000,
    has: &[
        "cancelOrder",
        "createLimitOrder",
        "createMarketOrder",
        "createMarketOrderWs",
        "createOrder",
        "editOrder",
        "fetchBalance",
        "fetchClosedOrders",
        "fetchCurrenciesWs",
        "fetchDepositAddress",
        "fetchL2OrderBook",
        "fetchMarkets",
        "fetchMyTrades",
        "fetchOpenOrders",
        "fetchOrder",
        "fetchOrderBook",
        "fetchTicker",
        "fetchTrades",
        "fetchTradingFees",
        "privateAPI",
        "publicAPI",
        "spot",
        "withdraw",
    ],
    endpoints: &[
        Endpoint {
            base: "https://api.independentreserve.com/Public",
            verb: "GET",
            key: "GetOrderBook",
            path: "GetOrderBook",
            auth: false,
        },
        Endpoint {
            base: "https://api.independentreserve.com/Private",
            verb: "POST",
            key: "GetOpenOrders",
            path: "GetOpenOrders",
            auth: true,
        },
        Endpoint {
            base: "https://api.independentreserve.com/Private",
            verb: "POST",
            key: "GetTrades",
            path: "GetTrades",
            auth: true,
        },
    ],
    taker: 0.005,
    maker: 0.005,
    timeframes: &[],
    kind: MarketKind::Cex,
};

crate::impl_generated_adapter!(Independentreserve, &SPEC);
