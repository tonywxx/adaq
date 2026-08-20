//! Mudrex (`mudrex`) 转译适配器 — 由 `scripts/gen_adapters.py`
//! 从 ccxt 4.5.73 `describe()` 自动生成(best-effort 批量补齐)。
//! 保留 MIT 声明见仓库 `NOTICE`;精确性由精选手写集保证。
//! 类别: Cex。

#![allow(clippy::too_many_arguments)]

use crate::generic::{ApiSpec, Endpoint, MarketKind};

/// ccxt `describe()` 静态快照(描述驱动引擎的输入)。
pub static SPEC: ApiSpec = ApiSpec {
    id: "mudrex",
    name: "Mudrex",
    version: "v1",
    rate_limit_ms: 100,
    has: &[
        "addMargin",
        "cancelOrder",
        "closePosition",
        "createLimitOrder",
        "createMarketOrder",
        "createMarketOrderWs",
        "createOrder",
        "createOrderWithTakeProfitAndStopLoss",
        "createReduceOnlyOrder",
        "editOrder",
        "fetchBalance",
        "fetchClosedOrders",
        "fetchCurrencies",
        "fetchCurrenciesWs",
        "fetchL2OrderBook",
        "fetchLeverage",
        "fetchMarkOHLCV",
        "fetchMarkets",
        "fetchMyTrades",
        "fetchOHLCV",
        "fetchOpenOrders",
        "fetchOrder",
        "fetchOrders",
        "fetchPositions",
        "fetchPositionsHistory",
        "fetchTicker",
        "fetchTickers",
        "privateAPI",
        "publicAPI",
        "reduceMargin",
        "setLeverage",
        "swap",
        "transfer",
        "watchOHLCV",
        "watchTicker",
        "watchTickers",
    ],
    endpoints: &[
        Endpoint {
            base: "https://trade.mudrex.com/fapi/v1",
            verb: "GET",
            key: "price/kline",
            path: "price/kline",
            auth: true,
        },
        Endpoint {
            base: "https://trade.mudrex.com/fapi/v1",
            verb: "GET",
            key: "price/mark-kline",
            path: "price/mark-kline",
            auth: true,
        },
        Endpoint {
            base: "https://trade.mudrex.com/fapi/v1",
            verb: "GET",
            key: "wallet/funds",
            path: "wallet/funds",
            auth: true,
        },
        Endpoint {
            base: "https://trade.mudrex.com/fapi/v1",
            verb: "GET",
            key: "futures/orders",
            path: "futures/orders",
            auth: true,
        },
        Endpoint {
            base: "https://trade.mudrex.com/fapi/v1",
            verb: "GET",
            key: "futures/orders/history",
            path: "futures/orders/history",
            auth: true,
        },
        Endpoint {
            base: "https://trade.mudrex.com/fapi/v1",
            verb: "GET",
            key: "futures/orders/{order_id}",
            path: "futures/orders/{order_id}",
            auth: true,
        },
        Endpoint {
            base: "https://trade.mudrex.com/fapi/v1",
            verb: "POST",
            key: "wallet/futures/transfer",
            path: "wallet/futures/transfer",
            auth: true,
        },
        Endpoint {
            base: "https://trade.mudrex.com/fapi/v1",
            verb: "POST",
            key: "futures/{asset_id}/order",
            path: "futures/{asset_id}/order",
            auth: true,
        },
        Endpoint {
            base: "https://trade.mudrex.com/fapi/v1",
            verb: "PATCH",
            key: "futures/orders/{order_id}",
            path: "futures/orders/{order_id}",
            auth: true,
        },
        Endpoint {
            base: "https://trade.mudrex.com/fapi/v1",
            verb: "DELETE",
            key: "futures/orders/{order_id}",
            path: "futures/orders/{order_id}",
            auth: true,
        },
    ],
    taker: 0.00059,
    maker: 0.00023,
    timeframes: &[
        "1m", "3m", "5m", "10m", "15m", "30m", "1h", "4h", "6h", "12h", "1d", "1w", "1M",
    ],
    kind: MarketKind::Cex,
};

crate::impl_generated_adapter!(Mudrex, &SPEC);
