//! dYdX (`dydx`) 转译适配器 — 由 `scripts/gen_adapters.py`
//! 从 ccxt 4.5.73 `describe()` 自动生成(best-effort 批量补齐)。
//! 保留 MIT 声明见仓库 `NOTICE`;精确性由精选手写集保证。
//! 类别: Dex。

#![allow(clippy::too_many_arguments)]

use crate::generic::{ApiSpec, Endpoint, MarketKind};

/// ccxt `describe()` 静态快照(描述驱动引擎的输入)。
pub static SPEC: ApiSpec = ApiSpec {
    id: "dydx",
    name: "dYdX",
    version: "v4",
    rate_limit_ms: 100,
    has: &[
        "cancelOrder",
        "cancelOrders",
        "createLimitOrder",
        "createMarketOrderWs",
        "createOrder",
        "editOrder",
        "fetchAccounts",
        "fetchBalance",
        "fetchClosedOrders",
        "fetchCurrenciesWs",
        "fetchDeposits",
        "fetchDepositsWithdrawals",
        "fetchFundingRateHistory",
        "fetchL2OrderBook",
        "fetchLedger",
        "fetchMarkets",
        "fetchOHLCV",
        "fetchOpenOrders",
        "fetchOrder",
        "fetchOrderBook",
        "fetchOrders",
        "fetchPosition",
        "fetchPositions",
        "fetchTime",
        "fetchTrades",
        "fetchTransfers",
        "fetchWithdrawals",
        "privateAPI",
        "publicAPI",
        "swap",
        "transfer",
        "withdraw",
    ],
    endpoints: &[
        Endpoint {
            base: "https://indexer.dydx.trade/v4",
            verb: "GET",
            key: "candles/perpetualMarkets/{market}",
            path: "candles/perpetualMarkets/{market}",
            auth: true,
        },
        Endpoint {
            base: "https://indexer.dydx.trade/v4",
            verb: "GET",
            key: "orders",
            path: "orders",
            auth: true,
        },
        Endpoint {
            base: "https://indexer.dydx.trade/v4",
            verb: "GET",
            key: "orders/parentSubaccountNumber",
            path: "orders/parentSubaccountNumber",
            auth: true,
        },
        Endpoint {
            base: "https://indexer.dydx.trade/v4",
            verb: "GET",
            key: "orders/{orderId}",
            path: "orders/{orderId}",
            auth: true,
        },
        Endpoint {
            base: "https://indexer.dydx.trade/v4",
            verb: "GET",
            key: "time",
            path: "time",
            auth: true,
        },
        Endpoint {
            base: "https://indexer.dydx.trade/v4",
            verb: "GET",
            key: "trades/perpetualMarket/{market}",
            path: "trades/perpetualMarket/{market}",
            auth: true,
        },
        Endpoint {
            base: "https://indexer.dydx.trade/v4",
            verb: "GET",
            key: "perpetualMarkets/{ticker}/orderbook",
            path: "perpetualMarkets/{ticker}/orderbook",
            auth: true,
        },
        Endpoint {
            base: "https://indexer.dydx.trade/v4",
            verb: "GET",
            key: "trades/perpetualMarket/{ticker}",
            path: "trades/perpetualMarket/{ticker}",
            auth: true,
        },
        Endpoint {
            base: "https://indexer.dydx.trade/v4",
            verb: "GET",
            key: "candles/{ticker}/{resolution}",
            path: "candles/{ticker}/{resolution}",
            auth: true,
        },
        Endpoint {
            base: "https://indexer.dydx.trade/v4",
            verb: "GET",
            key: "addresses/{address}/subaccountNumber/{subaccountNumber}/orders",
            path: "addresses/{address}/subaccountNumber/{subaccountNumber}/orders",
            auth: true,
        },
        Endpoint {
            base: "https://dydx-rest.publicnode.com",
            verb: "GET",
            key: "cosmos/auth/v1beta1/account_info/{dydxAddress}",
            path: "cosmos/auth/v1beta1/account_info/{dydxAddress}",
            auth: true,
        },
    ],
    taker: 0.0005,
    maker: 0.0001,
    timeframes: &["1m", "5m", "15m", "30m", "1h", "4h", "1d"],
    kind: MarketKind::Dex,
};

crate::impl_generated_adapter!(Dydx, &SPEC);
