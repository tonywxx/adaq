//! Bithumb (`bithumb`) 转译适配器 — 由 `scripts/gen_adapters.py`
//! 从 ccxt 4.5.73 `describe()` 自动生成(best-effort 批量补齐)。
//! 保留 MIT 声明见仓库 `NOTICE`;精确性由精选手写集保证。
//! 类别: Cex。

#![allow(clippy::too_many_arguments)]

use crate::generic::{ApiSpec, Endpoint, MarketKind};

/// ccxt `describe()` 静态快照(描述驱动引擎的输入)。
pub static SPEC: ApiSpec = ApiSpec {
    id: "bithumb",
    name: "Bithumb",
    version: "",
    rate_limit_ms: 500,
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
        "fetchOHLCV",
        "fetchOpenOrders",
        "fetchOrder",
        "fetchOrderBook",
        "fetchTicker",
        "fetchTickers",
        "fetchTrades",
        "privateAPI",
        "publicAPI",
        "spot",
        "withdraw",
    ],
    endpoints: &[
        Endpoint {
            base: "https://api.{hostname}/public",
            verb: "GET",
            key: "ticker/ALL_{quoteId}",
            path: "ticker/ALL_{quoteId}",
            auth: false,
        },
        Endpoint {
            base: "https://api.{hostname}/public",
            verb: "GET",
            key: "ticker/{baseId}_{quoteId}",
            path: "ticker/{baseId}_{quoteId}",
            auth: false,
        },
        Endpoint {
            base: "https://api.{hostname}/public",
            verb: "GET",
            key: "orderbook/ALL_{quoteId}",
            path: "orderbook/ALL_{quoteId}",
            auth: false,
        },
        Endpoint {
            base: "https://api.{hostname}/public",
            verb: "GET",
            key: "orderbook/{baseId}_{quoteId}",
            path: "orderbook/{baseId}_{quoteId}",
            auth: false,
        },
        Endpoint {
            base: "https://api.{hostname}",
            verb: "POST",
            key: "info/account",
            path: "info/account",
            auth: true,
        },
        Endpoint {
            base: "https://api.{hostname}",
            verb: "POST",
            key: "info/balance",
            path: "info/balance",
            auth: true,
        },
        Endpoint {
            base: "https://api.{hostname}",
            verb: "POST",
            key: "info/wallet_address",
            path: "info/wallet_address",
            auth: true,
        },
        Endpoint {
            base: "https://api.{hostname}",
            verb: "POST",
            key: "info/ticker",
            path: "info/ticker",
            auth: true,
        },
        Endpoint {
            base: "https://api.{hostname}",
            verb: "POST",
            key: "info/orders",
            path: "info/orders",
            auth: true,
        },
        Endpoint {
            base: "https://api.{hostname}",
            verb: "POST",
            key: "info/order_detail",
            path: "info/order_detail",
            auth: true,
        },
        Endpoint {
            base: "https://api.{hostname}",
            verb: "POST",
            key: "trade/place",
            path: "trade/place",
            auth: true,
        },
        Endpoint {
            base: "https://api.{hostname}",
            verb: "POST",
            key: "trade/cancel",
            path: "trade/cancel",
            auth: true,
        },
        Endpoint {
            base: "https://api.{hostname}",
            verb: "POST",
            key: "trade/btc_withdrawal",
            path: "trade/btc_withdrawal",
            auth: true,
        },
        Endpoint {
            base: "https://api.{hostname}",
            verb: "POST",
            key: "trade/krw_deposit",
            path: "trade/krw_deposit",
            auth: true,
        },
        Endpoint {
            base: "https://api.{hostname}",
            verb: "POST",
            key: "trade/krw_withdrawal",
            path: "trade/krw_withdrawal",
            auth: true,
        },
        Endpoint {
            base: "https://api.{hostname}",
            verb: "POST",
            key: "trade/market_buy",
            path: "trade/market_buy",
            auth: true,
        },
        Endpoint {
            base: "https://api.{hostname}",
            verb: "POST",
            key: "trade/market_sell",
            path: "trade/market_sell",
            auth: true,
        },
        Endpoint {
            base: "https://api.{hostname}",
            verb: "POST",
            key: "trade/stop_limit",
            path: "trade/stop_limit",
            auth: true,
        },
    ],
    taker: 0.0025,
    maker: 0.0025,
    timeframes: &["1m", "3m", "5m", "10m", "30m", "1h", "6h", "12h", "1d"],
    kind: MarketKind::Cex,
};

crate::impl_generated_adapter!(Bithumb, &SPEC);
