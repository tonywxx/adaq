//! Bitbns (`bitbns`) 转译适配器 — 由 `scripts/gen_adapters.py`
//! 从 ccxt 4.5.73 `describe()` 自动生成(best-effort 批量补齐)。
//! 保留 MIT 声明见仓库 `NOTICE`;精确性由精选手写集保证。
//! 类别: Cex。

#![allow(clippy::too_many_arguments)]

use crate::generic::{ApiSpec, Endpoint, MarketKind};

/// ccxt `describe()` 静态快照(描述驱动引擎的输入)。
pub static SPEC: ApiSpec = ApiSpec {
    id: "bitbns",
    name: "Bitbns",
    version: "v2",
    rate_limit_ms: 1000,
    has: &[
        "cancelOrder",
        "createLimitOrder",
        "createMarketOrder",
        "createMarketOrderWs",
        "createOrder",
        "createStopOrder",
        "createTriggerOrder",
        "editOrder",
        "fetchBalance",
        "fetchCurrencies",
        "fetchCurrenciesWs",
        "fetchDepositAddress",
        "fetchDeposits",
        "fetchL2OrderBook",
        "fetchMarkets",
        "fetchMyTrades",
        "fetchOpenOrders",
        "fetchOrder",
        "fetchOrderBook",
        "fetchStatus",
        "fetchTicker",
        "fetchTickers",
        "fetchTrades",
        "fetchWithdrawals",
        "privateAPI",
        "publicAPI",
        "spot",
    ],
    endpoints: &[
        Endpoint {
            base: "https://{hostname}",
            verb: "GET",
            key: "order/fetchMarkets",
            path: "order/fetchMarkets",
            auth: true,
        },
        Endpoint {
            base: "https://{hostname}",
            verb: "GET",
            key: "order/fetchTickers",
            path: "order/fetchTickers",
            auth: true,
        },
        Endpoint {
            base: "https://{hostname}",
            verb: "GET",
            key: "order/fetchOrderbook",
            path: "order/fetchOrderbook",
            auth: true,
        },
        Endpoint {
            base: "https://{hostname}",
            verb: "GET",
            key: "order/getTickerWithVolume",
            path: "order/getTickerWithVolume",
            auth: true,
        },
        Endpoint {
            base: "https://{hostname}",
            verb: "GET",
            key: "exchangeData/ohlc",
            path: "exchangeData/ohlc",
            auth: true,
        },
        Endpoint {
            base: "https://{hostname}",
            verb: "GET",
            key: "exchangeData/orderBook",
            path: "exchangeData/orderBook",
            auth: true,
        },
        Endpoint {
            base: "https://api.{hostname}/api/trade/v1",
            verb: "GET",
            key: "platform/status",
            path: "platform/status",
            auth: true,
        },
        Endpoint {
            base: "https://api.{hostname}/api/trade/v1",
            verb: "GET",
            key: "tickers",
            path: "tickers",
            auth: true,
        },
        Endpoint {
            base: "https://api.{hostname}/api/trade/v1",
            verb: "GET",
            key: "orderbook/sell/{symbol}",
            path: "orderbook/sell/{symbol}",
            auth: true,
        },
        Endpoint {
            base: "https://api.{hostname}/api/trade/v1",
            verb: "GET",
            key: "orderbook/buy/{symbol}",
            path: "orderbook/buy/{symbol}",
            auth: true,
        },
        Endpoint {
            base: "https://api.{hostname}/api/trade/v1",
            verb: "POST",
            key: "listOpenOrders/{symbol}",
            path: "listOpenOrders/{symbol}",
            auth: true,
        },
        Endpoint {
            base: "https://api.{hostname}/api/trade/v2",
            verb: "POST",
            key: "orders",
            path: "orders",
            auth: true,
        },
    ],
    taker: 0.0025,
    maker: 0.0025,
    timeframes: &[],
    kind: MarketKind::Cex,
};

crate::impl_generated_adapter!(Bitbns, &SPEC);
