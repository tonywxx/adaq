//! bitbank (`bitbank`) 转译适配器 — 由 `scripts/gen_adapters.py`
//! 从 ccxt 4.5.73 `describe()` 自动生成(best-effort 批量补齐)。
//! 保留 MIT 声明见仓库 `NOTICE`;精确性由精选手写集保证。
//! 类别: Cex。

#![allow(clippy::too_many_arguments)]

use crate::generic::{ApiSpec, Endpoint, MarketKind};

/// ccxt `describe()` 静态快照(描述驱动引擎的输入)。
pub static SPEC: ApiSpec = ApiSpec {
    id: "bitbank",
    name: "bitbank",
    version: "v1",
    rate_limit_ms: 100,
    has: &[
        "cancelOrder",
        "createLimitOrder",
        "createMarketOrder",
        "createMarketOrderWs",
        "createOrder",
        "editOrder",
        "fetchBalance",
        "fetchCurrenciesWs",
        "fetchDepositAddress",
        "fetchL2OrderBook",
        "fetchMarkets",
        "fetchMyTrades",
        "fetchOHLCV",
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
            base: "https://public.{hostname}",
            verb: "GET",
            key: "{pair}/ticker",
            path: "{pair}/ticker",
            auth: false,
        },
        Endpoint {
            base: "https://public.{hostname}",
            verb: "GET",
            key: "tickers",
            path: "tickers",
            auth: false,
        },
        Endpoint {
            base: "https://public.{hostname}",
            verb: "GET",
            key: "tickers_jpy",
            path: "tickers_jpy",
            auth: false,
        },
        Endpoint {
            base: "https://public.{hostname}",
            verb: "GET",
            key: "{pair}/depth",
            path: "{pair}/depth",
            auth: false,
        },
        Endpoint {
            base: "https://api.{hostname}",
            verb: "GET",
            key: "user/assets",
            path: "user/assets",
            auth: true,
        },
        Endpoint {
            base: "https://api.{hostname}",
            verb: "GET",
            key: "user/spot/order",
            path: "user/spot/order",
            auth: true,
        },
        Endpoint {
            base: "https://api.{hostname}",
            verb: "GET",
            key: "user/spot/active_orders",
            path: "user/spot/active_orders",
            auth: true,
        },
        Endpoint {
            base: "https://api.{hostname}",
            verb: "GET",
            key: "user/spot/trade_history",
            path: "user/spot/trade_history",
            auth: true,
        },
        Endpoint {
            base: "https://api.{hostname}",
            verb: "GET",
            key: "user/withdrawal_account",
            path: "user/withdrawal_account",
            auth: true,
        },
        Endpoint {
            base: "https://api.{hostname}",
            verb: "GET",
            key: "spot/status",
            path: "spot/status",
            auth: true,
        },
        Endpoint {
            base: "https://api.{hostname}",
            verb: "GET",
            key: "spot/pairs",
            path: "spot/pairs",
            auth: true,
        },
        Endpoint {
            base: "https://api.{hostname}",
            verb: "POST",
            key: "user/spot/order",
            path: "user/spot/order",
            auth: true,
        },
        Endpoint {
            base: "https://api.{hostname}",
            verb: "POST",
            key: "user/spot/cancel_order",
            path: "user/spot/cancel_order",
            auth: true,
        },
        Endpoint {
            base: "https://api.{hostname}",
            verb: "POST",
            key: "user/spot/cancel_orders",
            path: "user/spot/cancel_orders",
            auth: true,
        },
        Endpoint {
            base: "https://api.{hostname}",
            verb: "POST",
            key: "user/spot/orders_info",
            path: "user/spot/orders_info",
            auth: true,
        },
        Endpoint {
            base: "https://api.{hostname}",
            verb: "GET",
            key: "spot/pairs",
            path: "spot/pairs",
            auth: true,
        },
    ],
    taker: 0.0,
    maker: 0.0,
    timeframes: &[
        "1m", "5m", "15m", "30m", "1h", "4h", "8h", "12h", "1d", "1w",
    ],
    kind: MarketKind::Cex,
};

crate::impl_generated_adapter!(Bitbank, &SPEC);
