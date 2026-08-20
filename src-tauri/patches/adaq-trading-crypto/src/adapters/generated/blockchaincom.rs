//! Blockchain.com (`blockchaincom`) 转译适配器 — 由 `scripts/gen_adapters.py`
//! 从 ccxt 4.5.73 `describe()` 自动生成(best-effort 批量补齐)。
//! 保留 MIT 声明见仓库 `NOTICE`;精确性由精选手写集保证。
//! 类别: Cex。

#![allow(clippy::too_many_arguments)]

use crate::generic::{ApiSpec, Endpoint, MarketKind};

/// ccxt `describe()` 静态快照(描述驱动引擎的输入)。
pub static SPEC: ApiSpec = ApiSpec {
    id: "blockchaincom",
    name: "Blockchain.com",
    version: "v3",
    rate_limit_ms: 500,
    has: &[
        "cancelAllOrders",
        "cancelOrder",
        "createLimitOrder",
        "createMarketOrder",
        "createMarketOrderWs",
        "createOrder",
        "createStopLimitOrder",
        "createStopMarketOrder",
        "createStopOrder",
        "editOrder",
        "fetchBalance",
        "fetchCanceledOrders",
        "fetchClosedOrders",
        "fetchCurrenciesWs",
        "fetchDeposit",
        "fetchDepositAddress",
        "fetchDeposits",
        "fetchL2OrderBook",
        "fetchL3OrderBook",
        "fetchMarkets",
        "fetchMyTrades",
        "fetchOpenOrders",
        "fetchOrder",
        "fetchOrderBook",
        "fetchTicker",
        "fetchTickers",
        "fetchTradingFees",
        "fetchWithdrawal",
        "fetchWithdrawalWhitelist",
        "fetchWithdrawals",
        "privateAPI",
        "publicAPI",
        "spot",
        "withdraw",
    ],
    endpoints: &[
        Endpoint {
            base: "https://api.blockchain.com/v3/exchange",
            verb: "GET",
            key: "tickers",
            path: "tickers",
            auth: false,
        },
        Endpoint {
            base: "https://api.blockchain.com/v3/exchange",
            verb: "GET",
            key: "tickers/{symbol}",
            path: "tickers/{symbol}",
            auth: false,
        },
        Endpoint {
            base: "https://api.blockchain.com/v3/exchange",
            verb: "GET",
            key: "symbols",
            path: "symbols",
            auth: false,
        },
        Endpoint {
            base: "https://api.blockchain.com/v3/exchange",
            verb: "GET",
            key: "symbols/{symbol}",
            path: "symbols/{symbol}",
            auth: false,
        },
        Endpoint {
            base: "https://api.blockchain.com/v3/exchange",
            verb: "GET",
            key: "orders",
            path: "orders",
            auth: true,
        },
        Endpoint {
            base: "https://api.blockchain.com/v3/exchange",
            verb: "GET",
            key: "orders/{orderId}",
            path: "orders/{orderId}",
            auth: true,
        },
        Endpoint {
            base: "https://api.blockchain.com/v3/exchange",
            verb: "GET",
            key: "trades",
            path: "trades",
            auth: true,
        },
        Endpoint {
            base: "https://api.blockchain.com/v3/exchange",
            verb: "POST",
            key: "orders",
            path: "orders",
            auth: true,
        },
        Endpoint {
            base: "https://api.blockchain.com/v3/exchange",
            verb: "DELETE",
            key: "orders",
            path: "orders",
            auth: true,
        },
        Endpoint {
            base: "https://api.blockchain.com/v3/exchange",
            verb: "DELETE",
            key: "orders/{orderId}",
            path: "orders/{orderId}",
            auth: true,
        },
    ],
    taker: 0.0,
    maker: 0.0,
    timeframes: &[],
    kind: MarketKind::Cex,
};

crate::impl_generated_adapter!(Blockchaincom, &SPEC);
