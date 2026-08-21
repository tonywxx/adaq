//! Provider-neutral, host-owned paper trading contracts.
//!
//! This crate intentionally contains no HTTP, credentials, or Component/Worker
//! integration. Adapters translate provider evidence into these contracts;
//! the ledger remains the authority for reservations and local state.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Market {
    OkxSpot,
    UsEquity,
    AShare,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AdapterKind {
    OkxDemo,
    AlpacaPaper,
    AShareLocal,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct CapabilitySnapshot {
    pub adapter: AdapterKind,
    pub market: Market,
    pub currency: Currency,
    pub supports_market_orders: bool,
    pub supports_limit_orders: bool,
    pub supports_short_sales: bool,
    pub supports_margin: bool,
    pub supports_remote_reset: bool,
}

impl AdapterKind {
    pub fn capability_snapshot(self) -> CapabilitySnapshot {
        let (market, currency) = match self {
            Self::OkxDemo => (Market::OkxSpot, Currency::Usdt),
            Self::AlpacaPaper => (Market::UsEquity, Currency::Usd),
            Self::AShareLocal => (Market::AShare, Currency::Cny),
        };
        CapabilitySnapshot {
            adapter: self,
            market,
            currency,
            supports_market_orders: true,
            supports_limit_orders: true,
            supports_short_sales: false,
            supports_margin: false,
            supports_remote_reset: matches!(self, Self::AShareLocal),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Currency {
    Usdt,
    Usd,
    Cny,
}

impl Market {
    pub fn currency(self) -> Currency {
        match self {
            Self::OkxSpot => Currency::Usdt,
            Self::UsEquity => Currency::Usd,
            Self::AShare => Currency::Cny,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum Side {
    Buy,
    Sell,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OrderStatus {
    Accepted,
    PartiallyFilled,
    Filled,
    Cancelled,
    Rejected,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FillEvidence {
    TradeObserved,
    QuoteConstrained,
    BarConstrained,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReconciliationState {
    Reconciled,
    Required,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Position {
    pub quantity: Decimal,
    pub sellable_quantity: Decimal,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct AccountSnapshot {
    pub account_id: String,
    pub user_id: String,
    pub market: Market,
    pub currency: Currency,
    pub cash: Decimal,
    pub positions: BTreeMap<String, Position>,
    pub observed_at_ms: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Order {
    pub order_id: String,
    pub account_id: String,
    pub instrument: String,
    pub side: Side,
    pub quantity: Decimal,
    pub filled_quantity: Decimal,
    pub limit_price: Decimal,
    pub status: OrderStatus,
    pub submitted_at_ms: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Fill {
    pub fill_id: String,
    pub order_id: String,
    pub quantity: Decimal,
    pub price: Decimal,
    pub fee: Decimal,
    pub evidence: FillEvidence,
    pub occurred_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LedgerError {
    InvalidInput(&'static str),
    AccountMismatch,
    CurrencyMismatch,
    ReconciliationRequired,
    InsufficientCash,
    InsufficientPosition,
    InvalidFill,
    UnknownOrder,
    InvalidTransition,
}

impl std::fmt::Display for LedgerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for LedgerError {}

/// An append-only-in-behavior ledger. Cancellation and fills add state
/// transitions; they never rewrite or synthesize prior evidence.
#[derive(Clone, Debug)]
pub struct PaperLedger {
    account: AccountSnapshot,
    reserved_cash: Decimal,
    orders: BTreeMap<String, Order>,
    fills: Vec<Fill>,
    next_id: u64,
    reconciliation: ReconciliationState,
}

impl PaperLedger {
    pub fn new(account: AccountSnapshot) -> Result<Self, LedgerError> {
        if account.account_id.trim().is_empty()
            || account.user_id.trim().is_empty()
            || account.currency != account.market.currency()
            || account.cash < Decimal::ZERO
        {
            return Err(LedgerError::InvalidInput("invalid account snapshot"));
        }
        for position in account.positions.values() {
            if position.quantity < Decimal::ZERO
                || position.sellable_quantity < Decimal::ZERO
                || position.sellable_quantity > position.quantity
            {
                return Err(LedgerError::InvalidInput("invalid position"));
            }
        }
        Ok(Self {
            account,
            reserved_cash: Decimal::ZERO,
            orders: BTreeMap::new(),
            fills: Vec::new(),
            next_id: 1,
            reconciliation: ReconciliationState::Reconciled,
        })
    }

    pub fn account(&self) -> &AccountSnapshot {
        &self.account
    }
    pub fn reserved_cash(&self) -> Decimal {
        self.reserved_cash
    }
    pub fn buying_power(&self) -> Decimal {
        self.account.cash - self.reserved_cash
    }
    pub fn reconciliation(&self) -> ReconciliationState {
        self.reconciliation
    }
    pub fn orders(&self) -> impl Iterator<Item = &Order> {
        self.orders.values()
    }
    pub fn fills(&self) -> &[Fill] {
        &self.fills
    }

    pub fn submit_order(
        &mut self,
        user_id: &str,
        instrument: impl Into<String>,
        side: Side,
        quantity: Decimal,
        limit_price: Decimal,
        now_ms: i64,
    ) -> Result<String, LedgerError> {
        if self.reconciliation != ReconciliationState::Reconciled {
            return Err(LedgerError::ReconciliationRequired);
        }
        if user_id != self.account.user_id {
            return Err(LedgerError::AccountMismatch);
        }
        let instrument = instrument.into();
        if instrument.trim().is_empty() || quantity <= Decimal::ZERO || limit_price <= Decimal::ZERO
        {
            return Err(LedgerError::InvalidInput(
                "quantity, price, and instrument must be valid",
            ));
        }
        if self.account.market == Market::AShare && quantity.fract() != Decimal::ZERO {
            return Err(LedgerError::InvalidInput("A-share quantity must be whole"));
        }
        if side == Side::Sell && self.position(&instrument).sellable_quantity < quantity {
            return Err(LedgerError::InsufficientPosition);
        }
        let required = quantity * limit_price;
        if side == Side::Buy && self.buying_power() < required {
            return Err(LedgerError::InsufficientCash);
        }
        let id = self.id("order");
        if side == Side::Buy {
            self.reserved_cash += required;
        }
        self.orders.insert(
            id.clone(),
            Order {
                order_id: id.clone(),
                account_id: self.account.account_id.clone(),
                instrument,
                side,
                quantity,
                filled_quantity: Decimal::ZERO,
                limit_price,
                status: OrderStatus::Accepted,
                submitted_at_ms: now_ms,
            },
        );
        Ok(id)
    }

    pub fn apply_fill(&mut self, fill: Fill) -> Result<(), LedgerError> {
        if fill.quantity <= Decimal::ZERO || fill.price <= Decimal::ZERO || fill.fee < Decimal::ZERO
        {
            return Err(LedgerError::InvalidFill);
        }
        let order = self
            .orders
            .get_mut(&fill.order_id)
            .ok_or(LedgerError::UnknownOrder)?;
        if matches!(
            order.status,
            OrderStatus::Cancelled | OrderStatus::Rejected | OrderStatus::Filled
        ) || order.filled_quantity + fill.quantity > order.quantity
        {
            return Err(LedgerError::InvalidTransition);
        }
        if order.side == Side::Buy && fill.price > order.limit_price {
            return Err(LedgerError::InvalidFill);
        }
        if order.side == Side::Sell && fill.price < order.limit_price {
            return Err(LedgerError::InvalidFill);
        }
        let value = fill.quantity * fill.price + fill.fee;
        if order.side == Side::Buy {
            let reserved = (order.quantity - order.filled_quantity) * order.limit_price;
            if self.account.cash < value || self.reserved_cash < reserved {
                return Err(LedgerError::InsufficientCash);
            }
            self.reserved_cash -= reserved.min(self.reserved_cash);
            if self.account.cash < value {
                return Err(LedgerError::InsufficientCash);
            }
            self.account.cash -= value;
            self.reserved_cash +=
                (order.quantity - order.filled_quantity - fill.quantity) * order.limit_price;
            let position = self
                .account
                .positions
                .entry(order.instrument.clone())
                .or_insert(Position {
                    quantity: Decimal::ZERO,
                    sellable_quantity: Decimal::ZERO,
                });
            position.quantity += fill.quantity;
            // A-share T+1: a new purchase is not sellable until reconciliation supplies eligibility.
            if self.account.market != Market::AShare {
                position.sellable_quantity += fill.quantity;
            }
        } else {
            let position = self
                .account
                .positions
                .get_mut(&order.instrument)
                .ok_or(LedgerError::InsufficientPosition)?;
            if position.sellable_quantity < fill.quantity
                || self.account.cash + value < Decimal::ZERO
            {
                return Err(LedgerError::InsufficientPosition);
            }
            position.quantity -= fill.quantity;
            position.sellable_quantity -= fill.quantity;
            self.account.cash += value;
        }
        order.filled_quantity += fill.quantity;
        order.status = if order.filled_quantity == order.quantity {
            OrderStatus::Filled
        } else {
            OrderStatus::PartiallyFilled
        };
        self.fills.push(fill);
        Ok(())
    }

    pub fn cancel_order(&mut self, order_id: &str) -> Result<(), LedgerError> {
        let order = self
            .orders
            .get_mut(order_id)
            .ok_or(LedgerError::UnknownOrder)?;
        if !matches!(
            order.status,
            OrderStatus::Accepted | OrderStatus::PartiallyFilled
        ) {
            return Err(LedgerError::InvalidTransition);
        }
        if order.side == Side::Buy {
            self.reserved_cash -= (order.quantity - order.filled_quantity) * order.limit_price;
        }
        order.status = OrderStatus::Cancelled;
        Ok(())
    }

    pub fn reconcile(&mut self, snapshot: AccountSnapshot) -> Result<bool, LedgerError> {
        if snapshot.account_id != self.account.account_id
            || snapshot.user_id != self.account.user_id
        {
            return Err(LedgerError::AccountMismatch);
        }
        if snapshot.currency != self.account.currency {
            return Err(LedgerError::CurrencyMismatch);
        }
        let matches =
            snapshot.cash == self.account.cash && snapshot.positions == self.account.positions;
        self.reconciliation = if matches {
            ReconciliationState::Reconciled
        } else {
            ReconciliationState::Required
        };
        Ok(matches)
    }

    fn position(&self, instrument: &str) -> Position {
        self.account
            .positions
            .get(instrument)
            .cloned()
            .unwrap_or(Position {
                quantity: Decimal::ZERO,
                sellable_quantity: Decimal::ZERO,
            })
    }
    fn id(&mut self, prefix: &str) -> String {
        let id = format!("{prefix}-{}", self.next_id);
        self.next_id += 1;
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn account(market: Market) -> AccountSnapshot {
        AccountSnapshot {
            account_id: "acct".into(),
            user_id: "alice".into(),
            market,
            currency: market.currency(),
            cash: Decimal::new(1_000_000, 0),
            positions: BTreeMap::new(),
            observed_at_ms: 1,
        }
    }
    fn fill(order: &Order, quantity: Decimal, price: Decimal) -> Fill {
        Fill {
            fill_id: "fill-1".into(),
            order_id: order.order_id.clone(),
            quantity,
            price,
            fee: Decimal::ZERO,
            evidence: FillEvidence::TradeObserved,
            occurred_at_ms: 2,
        }
    }

    #[test]
    fn reservation_and_partial_fill_are_exact_and_idempotently_bounded() {
        let mut l = PaperLedger::new(account(Market::UsEquity)).unwrap();
        let id = l
            .submit_order(
                "alice",
                "AAPL",
                Side::Buy,
                Decimal::new(10, 0),
                Decimal::new(100, 0),
                1,
            )
            .unwrap();
        assert_eq!(l.reserved_cash(), Decimal::new(1000, 0));
        let order = l.orders().next().unwrap().clone();
        l.apply_fill(fill(&order, Decimal::new(4, 0), Decimal::new(99, 0)))
            .unwrap();
        assert_eq!(
            l.orders().next().unwrap().status,
            OrderStatus::PartiallyFilled
        );
        assert_eq!(l.buying_power(), Decimal::new(999004, 0));
        assert!(
            l.apply_fill(fill(&order, Decimal::new(7, 0), Decimal::new(99, 0)))
                .is_err()
        );
        assert_eq!(id, "order-1");
    }
    #[test]
    fn ashare_t_plus_one_and_no_short_sales() {
        let mut l = PaperLedger::new(account(Market::AShare)).unwrap();
        let id = l
            .submit_order(
                "alice",
                "600519",
                Side::Buy,
                Decimal::new(100, 0),
                Decimal::new(10, 0),
                1,
            )
            .unwrap();
        let order = l.orders().next().unwrap().clone();
        l.apply_fill(fill(&order, Decimal::new(100, 0), Decimal::new(10, 0)))
            .unwrap();
        assert!(matches!(
            l.submit_order(
                "alice",
                "600519",
                Side::Sell,
                Decimal::ONE,
                Decimal::new(10, 0),
                2
            ),
            Err(LedgerError::InsufficientPosition)
        ));
        assert_eq!(
            l.account().positions["600519"].sellable_quantity,
            Decimal::ZERO
        );
        assert_eq!(id, "order-1");
    }
    #[test]
    fn mismatch_fails_closed_until_reconciled() {
        let mut l = PaperLedger::new(account(Market::UsEquity)).unwrap();
        let mut changed = l.account().clone();
        changed.cash -= Decimal::ONE;
        assert!(!l.reconcile(changed).unwrap());
        assert_eq!(l.reconciliation(), ReconciliationState::Required);
        assert!(matches!(
            l.submit_order("alice", "AAPL", Side::Buy, Decimal::ONE, Decimal::ONE, 1),
            Err(LedgerError::ReconciliationRequired)
        ));
    }
    #[test]
    fn adapter_capabilities_keep_real_and_credit_paths_unavailable() {
        let okx = AdapterKind::OkxDemo.capability_snapshot();
        let ashare = AdapterKind::AShareLocal.capability_snapshot();
        assert_eq!(okx.currency, Currency::Usdt);
        assert!(!okx.supports_remote_reset);
        assert!(!ashare.supports_margin);
        assert!(!ashare.supports_short_sales);
        assert!(ashare.supports_remote_reset);
    }

    #[test]
    fn accounts_are_user_scoped_and_currency_bound() {
        let mut l = PaperLedger::new(account(Market::OkxSpot)).unwrap();
        assert!(matches!(
            l.submit_order("bob", "BTC-USDT", Side::Buy, Decimal::ONE, Decimal::ONE, 1),
            Err(LedgerError::AccountMismatch)
        ));
    }
}
