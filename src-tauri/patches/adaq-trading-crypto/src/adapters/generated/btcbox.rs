//! BtcBox (`btcbox`) 转译适配器 — 由 `scripts/gen_adapters.py`
//! 从 ccxt 4.5.73 `describe()` 自动生成(best-effort 批量补齐)。
//! 保留 MIT 声明见仓库 `NOTICE`;精确性由精选手写集保证。
//! 类别: Cex。

#![allow(clippy::too_many_arguments)]

use crate::generic::{ApiSpec, Endpoint, MarketKind};

/// ccxt `describe()` 静态快照(描述驱动引擎的输入)。
pub static SPEC: ApiSpec = ApiSpec {
    id: "btcbox",
    name: "BtcBox",
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
        "fetchCurrenciesWs",
        "fetchL2OrderBook",
        "fetchMarkets",
        "fetchOpenOrders",
        "fetchOrder",
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
            base: "https://www.btcbox.co.jp/api",
            verb: "GET",
            key: "depth",
            path: "depth",
            auth: false,
        },
        Endpoint {
            base: "https://www.btcbox.co.jp/api",
            verb: "GET",
            key: "orders",
            path: "orders",
            auth: false,
        },
        Endpoint {
            base: "https://www.btcbox.co.jp/api",
            verb: "GET",
            key: "ticker",
            path: "ticker",
            auth: false,
        },
        Endpoint {
            base: "https://www.btcbox.co.jp/api",
            verb: "GET",
            key: "tickers",
            path: "tickers",
            auth: false,
        },
        Endpoint {
            base: "https://www.btcbox.co.jp/api",
            verb: "POST",
            key: "balance",
            path: "balance",
            auth: true,
        },
        Endpoint {
            base: "https://www.btcbox.co.jp/api",
            verb: "POST",
            key: "trade_add",
            path: "trade_add",
            auth: true,
        },
        Endpoint {
            base: "https://www.btcbox.co.jp/api",
            verb: "POST",
            key: "trade_cancel",
            path: "trade_cancel",
            auth: true,
        },
        Endpoint {
            base: "https://www.btcbox.co.jp/api",
            verb: "POST",
            key: "trade_list",
            path: "trade_list",
            auth: true,
        },
        Endpoint {
            base: "https://www.btcbox.co.jp/api",
            verb: "POST",
            key: "trade_view",
            path: "trade_view",
            auth: true,
        },
        Endpoint {
            base: "https://www.btcbox.co.jp/api",
            verb: "POST",
            key: "wallet",
            path: "wallet",
            auth: true,
        },
    ],
    taker: 0.0,
    maker: 0.0,
    timeframes: &[],
    kind: MarketKind::Cex,
};

crate::impl_generated_adapter!(Btcbox, &SPEC);
