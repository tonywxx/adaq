//! myokx 适配器(ADR-0017,由 generated promote 为 curated)。
//!
//! 与 okx 主站仅 `id` 不同(base URL 暂用 OKX 主站 `https://www.okx.com/api/v5`,
//! 待核对 myokx 实体端点);签名(`OK-ACCESS-*` + HMAC-SHA256)、解析、市集逻辑
//! 完全复用 okx 主实现(候选 ③ 参数化:删除原 ~570 行近逐字克隆,只剩薄包装)。

use crate::adapters::okx::Okx;
use crate::error::Result;
use crate::exchange::{Config, Exchange, Params};
use crate::types::{
    Balances, Market, OHLCV, Order, OrderBook, Position, Ticker, Tickers, Trade,
};

/// myokx = okx 实例(id 不同,实现复用)。
pub struct MyOkx {
    inner: Okx,
}

impl MyOkx {
    /// 实现面与 okx 主站一致(单一真源,避免手抄漂移;见 ADR-0001 契约测试)。
    pub const IMPLEMENTED: &'static [&'static str] = Okx::IMPLEMENTED;

    pub fn new(config: Config) -> Result<Self> {
        Okx::with_endpoints(config, "myokx", "https://www.okx.com/api/v5", 110)
            .map(|inner| MyOkx { inner })
    }
}

impl Exchange for MyOkx {
    fn id(&self) -> &'static str {
        self.inner.id()
    }
    fn config(&self) -> &Config {
        self.inner.config()
    }

    async fn fetch_markets(&self) -> Result<Vec<Market>> {
        self.inner.fetch_markets().await
    }
    async fn fetch_ticker(&self, symbol: &str, params: Params) -> Result<Ticker> {
        self.inner.fetch_ticker(symbol, params).await
    }
    async fn fetch_tickers(&self, symbols: Option<&[&str]>, params: Params) -> Result<Tickers> {
        self.inner.fetch_tickers(symbols, params).await
    }
    async fn fetch_order_book(
        &self,
        symbol: &str,
        limit: Option<i64>,
        params: Params,
    ) -> Result<OrderBook> {
        self.inner.fetch_order_book(symbol, limit, params).await
    }
    async fn fetch_trades(
        &self,
        symbol: &str,
        since: Option<i64>,
        limit: Option<i64>,
        params: Params,
    ) -> Result<Vec<Trade>> {
        self.inner.fetch_trades(symbol, since, limit, params).await
    }
    async fn fetch_ohlcv(
        &self,
        symbol: &str,
        timeframe: &str,
        since: Option<i64>,
        limit: Option<i64>,
        params: Params,
    ) -> Result<Vec<OHLCV>> {
        self.inner.fetch_ohlcv(symbol, timeframe, since, limit, params).await
    }
    async fn fetch_balance(&self, params: Params) -> Result<Balances> {
        self.inner.fetch_balance(params).await
    }
    async fn fetch_positions(
        &self,
        symbols: Option<&[&str]>,
        params: Params,
    ) -> Result<Vec<Position>> {
        self.inner.fetch_positions(symbols, params).await
    }
    async fn fetch_orders(
        &self,
        symbol: Option<&str>,
        since: Option<i64>,
        limit: Option<i64>,
        params: Params,
    ) -> Result<Vec<Order>> {
        self.inner.fetch_orders(symbol, since, limit, params).await
    }
    async fn fetch_open_orders(
        &self,
        symbol: Option<&str>,
        since: Option<i64>,
        limit: Option<i64>,
        params: Params,
    ) -> Result<Vec<Order>> {
        self.inner.fetch_open_orders(symbol, since, limit, params).await
    }
    async fn create_order(
        &self,
        symbol: &str,
        order_type: &str,
        side: &str,
        amount: &str,
        price: Option<&str>,
        params: Params,
    ) -> Result<Order> {
        self.inner.create_order(symbol, order_type, side, amount, price, params).await
    }
    async fn cancel_order(&self, id: &str, symbol: &str, params: Params) -> Result<Order> {
        self.inner.cancel_order(id, symbol, params).await
    }
}
