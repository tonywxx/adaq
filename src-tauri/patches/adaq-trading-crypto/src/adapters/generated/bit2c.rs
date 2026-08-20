//! Bit2C (`bit2c`) 转译适配器 — 由 `scripts/gen_adapters.py`
//! 从 ccxt 4.5.73 `describe()` 自动生成(best-effort 批量补齐)。
//! 保留 MIT 声明见仓库 `NOTICE`;精确性由精选手写集保证。
//! 类别: Cex。

#![allow(clippy::too_many_arguments)]

use crate::generic::{ApiSpec, Endpoint, MarketKind};

/// ccxt `describe()` 静态快照(描述驱动引擎的输入)。
pub static SPEC: ApiSpec = ApiSpec {
    id: "bit2c",
    name: "Bit2C",
    version: "",
    rate_limit_ms: 3000,
    has: &[
        "cancelOrder",
        "createLimitOrder",
        "createMarketOrder",
        "createMarketOrderWs",
        "createOrder",
        "editOrder",
        "fetchBalance",
        "fetchCurrencies",
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
    ],
    endpoints: &[
        Endpoint {
            base: "https://bit2c.co.il",
            verb: "GET",
            key: "Exchanges/{pair}/Ticker",
            path: "Exchanges/{pair}/Ticker",
            auth: false,
        },
        Endpoint {
            base: "https://bit2c.co.il",
            verb: "GET",
            key: "Exchanges/{pair}/orderbook",
            path: "Exchanges/{pair}/orderbook",
            auth: false,
        },
        Endpoint {
            base: "https://bit2c.co.il",
            verb: "GET",
            key: "Exchanges/{pair}/trades",
            path: "Exchanges/{pair}/trades",
            auth: false,
        },
        Endpoint {
            base: "https://bit2c.co.il",
            verb: "POST",
            key: "Order/AddFund",
            path: "Order/AddFund",
            auth: true,
        },
        Endpoint {
            base: "https://bit2c.co.il",
            verb: "POST",
            key: "Order/AddOrder",
            path: "Order/AddOrder",
            auth: true,
        },
        Endpoint {
            base: "https://bit2c.co.il",
            verb: "POST",
            key: "Order/GetById",
            path: "Order/GetById",
            auth: true,
        },
        Endpoint {
            base: "https://bit2c.co.il",
            verb: "POST",
            key: "Order/AddOrderMarketPriceBuy",
            path: "Order/AddOrderMarketPriceBuy",
            auth: true,
        },
        Endpoint {
            base: "https://bit2c.co.il",
            verb: "POST",
            key: "Order/AddOrderMarketPriceSell",
            path: "Order/AddOrderMarketPriceSell",
            auth: true,
        },
        Endpoint {
            base: "https://bit2c.co.il",
            verb: "POST",
            key: "Order/CancelOrder",
            path: "Order/CancelOrder",
            auth: true,
        },
        Endpoint {
            base: "https://bit2c.co.il",
            verb: "POST",
            key: "Order/AddCoinFundsRequest",
            path: "Order/AddCoinFundsRequest",
            auth: true,
        },
        Endpoint {
            base: "https://bit2c.co.il",
            verb: "POST",
            key: "Order/AddStopOrder",
            path: "Order/AddStopOrder",
            auth: true,
        },
        Endpoint {
            base: "https://bit2c.co.il",
            verb: "GET",
            key: "Account/Balance",
            path: "Account/Balance",
            auth: true,
        },
        Endpoint {
            base: "https://bit2c.co.il",
            verb: "GET",
            key: "Account/Balance/v2",
            path: "Account/Balance/v2",
            auth: true,
        },
        Endpoint {
            base: "https://bit2c.co.il",
            verb: "GET",
            key: "Order/MyOrders",
            path: "Order/MyOrders",
            auth: true,
        },
        Endpoint {
            base: "https://bit2c.co.il",
            verb: "GET",
            key: "Order/GetById",
            path: "Order/GetById",
            auth: true,
        },
        Endpoint {
            base: "https://bit2c.co.il",
            verb: "GET",
            key: "Order/AccountHistory",
            path: "Order/AccountHistory",
            auth: true,
        },
        Endpoint {
            base: "https://bit2c.co.il",
            verb: "GET",
            key: "Order/OrderHistory",
            path: "Order/OrderHistory",
            auth: true,
        },
    ],
    taker: 0.03,
    maker: 0.025,
    timeframes: &[],
    kind: MarketKind::Cex,
};

crate::impl_generated_adapter!(Bit2c, &SPEC);
