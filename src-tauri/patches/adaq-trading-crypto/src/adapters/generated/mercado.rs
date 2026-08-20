//! Mercado Bitcoin (`mercado`) 转译适配器 — 由 `scripts/gen_adapters.py`
//! 从 ccxt 4.5.73 `describe()` 自动生成(best-effort 批量补齐)。
//! 保留 MIT 声明见仓库 `NOTICE`;精确性由精选手写集保证。
//! 类别: Cex。

#![allow(clippy::too_many_arguments)]

use crate::generic::{ApiSpec, Endpoint, MarketKind};

/// ccxt `describe()` 静态快照(描述驱动引擎的输入)。
pub static SPEC: ApiSpec = ApiSpec {
    id: "mercado",
    name: "Mercado Bitcoin",
    version: "v3",
    rate_limit_ms: 1000,
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
        "fetchMyTrades",
        "fetchOHLCV",
        "fetchOpenOrders",
        "fetchOrder",
        "fetchOrderBook",
        "fetchOrders",
        "fetchTicker",
        "fetchTrades",
        "privateAPI",
        "publicAPI",
        "spot",
        "withdraw",
    ],
    endpoints: &[
        Endpoint {
            base: "https://www.mercadobitcoin.net/api",
            verb: "GET",
            key: "{coin}/orderbook/",
            path: "{coin}/orderbook/",
            auth: false,
        },
        Endpoint {
            base: "https://www.mercadobitcoin.net/api",
            verb: "GET",
            key: "{coin}/ticker/",
            path: "{coin}/ticker/",
            auth: false,
        },
        Endpoint {
            base: "https://www.mercadobitcoin.net/api",
            verb: "GET",
            key: "{coin}/trades/",
            path: "{coin}/trades/",
            auth: false,
        },
        Endpoint {
            base: "https://www.mercadobitcoin.net/api",
            verb: "GET",
            key: "{coin}/trades/{from}/",
            path: "{coin}/trades/{from}/",
            auth: false,
        },
        Endpoint {
            base: "https://www.mercadobitcoin.net/api",
            verb: "GET",
            key: "{coin}/trades/{from}/{to}",
            path: "{coin}/trades/{from}/{to}",
            auth: false,
        },
        Endpoint {
            base: "https://www.mercadobitcoin.net/tapi",
            verb: "POST",
            key: "cancel_order",
            path: "cancel_order",
            auth: true,
        },
        Endpoint {
            base: "https://www.mercadobitcoin.net/tapi",
            verb: "POST",
            key: "get_account_info",
            path: "get_account_info",
            auth: true,
        },
        Endpoint {
            base: "https://www.mercadobitcoin.net/tapi",
            verb: "POST",
            key: "get_order",
            path: "get_order",
            auth: true,
        },
        Endpoint {
            base: "https://www.mercadobitcoin.net/tapi",
            verb: "POST",
            key: "list_orders",
            path: "list_orders",
            auth: true,
        },
        Endpoint {
            base: "https://www.mercadobitcoin.net/tapi",
            verb: "POST",
            key: "list_orderbook",
            path: "list_orderbook",
            auth: true,
        },
        Endpoint {
            base: "https://www.mercadobitcoin.net/tapi",
            verb: "POST",
            key: "place_buy_order",
            path: "place_buy_order",
            auth: true,
        },
        Endpoint {
            base: "https://www.mercadobitcoin.net/tapi",
            verb: "POST",
            key: "place_sell_order",
            path: "place_sell_order",
            auth: true,
        },
        Endpoint {
            base: "https://www.mercadobitcoin.net/tapi",
            verb: "POST",
            key: "place_market_buy_order",
            path: "place_market_buy_order",
            auth: true,
        },
        Endpoint {
            base: "https://www.mercadobitcoin.net/tapi",
            verb: "POST",
            key: "place_market_sell_order",
            path: "place_market_sell_order",
            auth: true,
        },
        Endpoint {
            base: "https://www.mercadobitcoin.com.br/v4",
            verb: "GET",
            key: "{coin}/candle/",
            path: "{coin}/candle/",
            auth: false,
        },
        Endpoint {
            base: "https://api.mercadobitcoin.net/api/v4",
            verb: "GET",
            key: "candles",
            path: "candles",
            auth: false,
        },
    ],
    taker: 0.007,
    maker: 0.003,
    timeframes: &["15m", "1h", "3h", "1d", "1w", "1M"],
    kind: MarketKind::Cex,
};

crate::impl_generated_adapter!(Mercado, &SPEC);
