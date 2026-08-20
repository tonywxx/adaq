//! bitFlyer (`bitflyer`) 转译适配器 — 由 `scripts/gen_adapters.py`
//! 从 ccxt 4.5.73 `describe()` 自动生成(best-effort 批量补齐)。
//! 保留 MIT 声明见仓库 `NOTICE`;精确性由精选手写集保证。
//! 类别: Cex。

#![allow(clippy::too_many_arguments)]

use crate::generic::{ApiSpec, Endpoint, MarketKind};

/// ccxt `describe()` 静态快照(描述驱动引擎的输入)。
pub static SPEC: ApiSpec = ApiSpec {
    id: "bitflyer",
    name: "bitFlyer",
    version: "v1",
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
        "fetchDeposits",
        "fetchFundingRate",
        "fetchL2OrderBook",
        "fetchMarkets",
        "fetchMyTrades",
        "fetchOpenOrders",
        "fetchOrder",
        "fetchOrderBook",
        "fetchOrders",
        "fetchPositions",
        "fetchTicker",
        "fetchTrades",
        "fetchTradingFee",
        "fetchWithdrawals",
        "privateAPI",
        "publicAPI",
        "spot",
        "withdraw",
    ],
    endpoints: &[
        Endpoint {
            base: "https://api.{hostname}",
            verb: "GET",
            key: "getmarkets/usa",
            path: "getmarkets/usa",
            auth: false,
        },
        Endpoint {
            base: "https://api.{hostname}",
            verb: "GET",
            key: "getmarkets/eu",
            path: "getmarkets/eu",
            auth: false,
        },
        Endpoint {
            base: "https://api.{hostname}",
            verb: "GET",
            key: "getmarkets",
            path: "getmarkets",
            auth: false,
        },
        Endpoint {
            base: "https://api.{hostname}",
            verb: "GET",
            key: "getticker",
            path: "getticker",
            auth: false,
        },
        Endpoint {
            base: "https://api.{hostname}",
            verb: "GET",
            key: "getbalance",
            path: "getbalance",
            auth: true,
        },
    ],
    taker: 0.002,
    maker: 0.002,
    timeframes: &[],
    kind: MarketKind::Cex,
};

crate::impl_generated_adapter!(Bitflyer, &SPEC);
