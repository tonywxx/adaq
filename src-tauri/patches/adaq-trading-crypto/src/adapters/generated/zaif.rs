//! Zaif (`zaif`) 转译适配器 — 由 `scripts/gen_adapters.py`
//! 从 ccxt 4.5.73 `describe()` 自动生成(best-effort 批量补齐)。
//! 保留 MIT 声明见仓库 `NOTICE`;精确性由精选手写集保证。
//! 类别: Cex。

#![allow(clippy::too_many_arguments)]

use crate::generic::{ApiSpec, Endpoint, MarketKind};

/// ccxt `describe()` 静态快照(描述驱动引擎的输入)。
pub static SPEC: ApiSpec = ApiSpec {
    id: "zaif",
    name: "Zaif",
    version: "1",
    rate_limit_ms: 100,
    has: &[
        "cancelOrder",
        "createLimitOrder",
        "createMarketOrderWs",
        "createOrder",
        "editOrder",
        "fetchBalance",
        "fetchClosedOrders",
        "fetchCurrenciesWs",
        "fetchL2OrderBook",
        "fetchMarkets",
        "fetchOpenOrders",
        "fetchOrderBook",
        "fetchTicker",
        "fetchTrades",
        "privateAPI",
        "publicAPI",
        "spot",
        "withdraw",
    ],
    endpoints: &[
        Endpoint {
            base: "https://api.zaif.jp",
            verb: "GET",
            key: "depth/{pair}",
            path: "depth/{pair}",
            auth: false,
        },
        Endpoint {
            base: "https://api.zaif.jp",
            verb: "GET",
            key: "currencies/{pair}",
            path: "currencies/{pair}",
            auth: false,
        },
        Endpoint {
            base: "https://api.zaif.jp",
            verb: "GET",
            key: "currencies/all",
            path: "currencies/all",
            auth: false,
        },
        Endpoint {
            base: "https://api.zaif.jp",
            verb: "GET",
            key: "currency_pairs/{pair}",
            path: "currency_pairs/{pair}",
            auth: false,
        },
        Endpoint {
            base: "https://api.zaif.jp",
            verb: "GET",
            key: "currency_pairs/all",
            path: "currency_pairs/all",
            auth: false,
        },
        Endpoint {
            base: "https://api.zaif.jp",
            verb: "GET",
            key: "ticker/{pair}",
            path: "ticker/{pair}",
            auth: false,
        },
        Endpoint {
            base: "https://api.zaif.jp",
            verb: "GET",
            key: "trades/{pair}",
            path: "trades/{pair}",
            auth: false,
        },
        Endpoint {
            base: "https://api.zaif.jp",
            verb: "POST",
            key: "active_orders",
            path: "active_orders",
            auth: true,
        },
        Endpoint {
            base: "https://api.zaif.jp",
            verb: "POST",
            key: "cancel_order",
            path: "cancel_order",
            auth: true,
        },
        Endpoint {
            base: "https://api.zaif.jp",
            verb: "POST",
            key: "trade",
            path: "trade",
            auth: true,
        },
        Endpoint {
            base: "https://api.zaif.jp",
            verb: "POST",
            key: "trade_history",
            path: "trade_history",
            auth: true,
        },
        Endpoint {
            base: "https://api.zaif.jp",
            verb: "GET",
            key: "ticker/{group_id}/{pair}",
            path: "ticker/{group_id}/{pair}",
            auth: true,
        },
        Endpoint {
            base: "https://api.zaif.jp",
            verb: "GET",
            key: "trades/{group_id}/{pair}",
            path: "trades/{group_id}/{pair}",
            auth: true,
        },
        Endpoint {
            base: "https://api.zaif.jp",
            verb: "GET",
            key: "depth/{group_id}/{pair}",
            path: "depth/{group_id}/{pair}",
            auth: true,
        },
    ],
    taker: 0.001,
    maker: 0.0,
    timeframes: &[],
    kind: MarketKind::Cex,
};

crate::impl_generated_adapter!(Zaif, &SPEC);
