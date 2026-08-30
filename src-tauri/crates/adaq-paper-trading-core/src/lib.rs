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

pub const OKX_DEMO_FUNDING_TARGET: Decimal = Decimal::from_parts(1_000_000, 0, 0, false, 0);

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct RiskPolicy {
    pub max_order_notional: Decimal,
    pub reserve_cash: Decimal,
    pub freeze_new_risk: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct RiskDecision {
    pub approved: bool,
    pub reason: String,
    pub requested_notional: Decimal,
    pub approved_notional: Decimal,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ProviderEvidence {
    pub provider: AdapterKind,
    pub operation_id: String,
    pub local_order_id: Option<String>,
    pub provider_order_id: Option<String>,
    pub status: String,
    pub error_code: Option<String>,
    pub observed_at_ms: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum ExecutionOutcome {
    Accepted(ProviderEvidence),
    Rejected(ProviderEvidence),
    Uncertain(ProviderEvidence),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExecutionError {
    InvalidRiskPolicy,
    RiskRejected(RiskDecision),
    VenueMismatch,
    DuplicateOperation,
    ReconciliationRequired,
    UncertainOutcome,
}

impl std::fmt::Display for ExecutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for ExecutionError {}

/// Host-owned, provider-neutral gate for new execution. It never receives a
/// credential and never delegates Risk authority to an adapter or Worker.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PaperExecution {
    adapter: AdapterKind,
    policy: RiskPolicy,
    blocked: bool,
    operations: BTreeMap<String, ExecutionOutcome>,
}

impl PaperExecution {
    pub fn okx_demo(policy: RiskPolicy) -> Result<Self, ExecutionError> {
        if policy.max_order_notional <= Decimal::ZERO || policy.reserve_cash < Decimal::ZERO {
            return Err(ExecutionError::InvalidRiskPolicy);
        }
        Ok(Self {
            adapter: AdapterKind::OkxDemo,
            policy,
            blocked: false,
            operations: BTreeMap::new(),
        })
    }

    pub fn adapter(&self) -> AdapterKind {
        self.adapter
    }
    pub fn policy(&self) -> &RiskPolicy {
        &self.policy
    }
    pub fn is_blocked(&self) -> bool {
        self.blocked
    }
    pub fn evidence(&self) -> impl Iterator<Item = &ExecutionOutcome> {
        self.operations.values()
    }

    pub fn provider_order_id(&self, operation_id: &str) -> Option<String> {
        self.operations
            .get(operation_id)
            .and_then(|outcome| match outcome {
                ExecutionOutcome::Accepted(evidence)
                | ExecutionOutcome::Rejected(evidence)
                | ExecutionOutcome::Uncertain(evidence) => evidence.provider_order_id.clone(),
            })
    }

    pub fn local_order_id(&self, operation_id: &str) -> Option<String> {
        self.operations
            .get(operation_id)
            .and_then(|outcome| match outcome {
                ExecutionOutcome::Accepted(evidence)
                | ExecutionOutcome::Rejected(evidence)
                | ExecutionOutcome::Uncertain(evidence) => evidence.local_order_id.clone(),
            })
    }

    pub fn approve(&self, account: &PaperLedger, notional: Decimal) -> RiskDecision {
        let reason = if self.blocked || self.policy.freeze_new_risk {
            "new risk is frozen"
        } else if notional <= Decimal::ZERO {
            "order notional must be positive"
        } else if notional > self.policy.max_order_notional {
            "order exceeds the Host Risk limit"
        } else if account.buying_power() - self.policy.reserve_cash < notional {
            "order exceeds available reserved buying power"
        } else {
            "approved"
        };
        let approved = reason == "approved";
        RiskDecision {
            approved,
            reason: reason.to_owned(),
            requested_notional: notional,
            approved_notional: if approved { notional } else { Decimal::ZERO },
        }
    }

    pub fn begin(
        &mut self,
        operation_id: impl Into<String>,
        account: &mut PaperLedger,
        instrument: &str,
        side: Side,
        quantity: Decimal,
        limit_price: Decimal,
        now_ms: i64,
    ) -> Result<(String, RiskDecision), ExecutionError> {
        let operation_id = operation_id.into();
        if self.operations.contains_key(&operation_id) {
            return Err(ExecutionError::DuplicateOperation);
        }
        if self.blocked {
            return Err(ExecutionError::ReconciliationRequired);
        }
        if self.adapter != AdapterKind::OkxDemo || !instrument.contains('-') {
            return Err(ExecutionError::VenueMismatch);
        }
        let decision = self.approve(account, quantity * limit_price);
        if !decision.approved {
            return Err(ExecutionError::RiskRejected(decision));
        }
        let user_id = account.account().user_id.clone();
        account
            .submit_order(&user_id, instrument, side, quantity, limit_price, now_ms)
            .map_err(|_| ExecutionError::ReconciliationRequired)?;
        let local_order_id = account.orders().last().map(|order| order.order_id.clone());
        let order_id = operation_id.clone();
        self.operations.insert(
            operation_id,
            ExecutionOutcome::Accepted(ProviderEvidence {
                provider: AdapterKind::OkxDemo,
                operation_id: order_id.clone(),
                local_order_id,
                provider_order_id: None,
                status: format!("intent_{:?}", side).to_lowercase(),
                error_code: None,
                observed_at_ms: now_ms,
            }),
        );
        Ok((order_id, decision))
    }

    pub fn mark_uncertain(
        &mut self,
        operation_id: &str,
        now_ms: i64,
    ) -> Result<(), ExecutionError> {
        let outcome = self
            .operations
            .get_mut(operation_id)
            .ok_or(ExecutionError::DuplicateOperation)?;
        let local_order_id = match &*outcome {
            ExecutionOutcome::Accepted(evidence)
            | ExecutionOutcome::Rejected(evidence)
            | ExecutionOutcome::Uncertain(evidence) => evidence.local_order_id.clone(),
        };
        *outcome = ExecutionOutcome::Uncertain(ProviderEvidence {
            provider: AdapterKind::OkxDemo,
            operation_id: operation_id.to_owned(),
            local_order_id,
            provider_order_id: None,
            status: "unknown".to_owned(),
            error_code: Some("provider_timeout".to_owned()),
            observed_at_ms: now_ms,
        });
        self.blocked = true;
        Ok(())
    }

    pub fn record_provider_outcome(
        &mut self,
        operation_id: &str,
        provider_order_id: Option<String>,
        status: impl Into<String>,
        error_code: Option<String>,
        now_ms: i64,
    ) -> Result<(), ExecutionError> {
        let outcome = self
            .operations
            .get_mut(operation_id)
            .ok_or(ExecutionError::DuplicateOperation)?;
        let local_order_id = match &*outcome {
            ExecutionOutcome::Accepted(evidence)
            | ExecutionOutcome::Rejected(evidence)
            | ExecutionOutcome::Uncertain(evidence) => evidence.local_order_id.clone(),
        };
        let evidence = ProviderEvidence {
            provider: AdapterKind::OkxDemo,
            operation_id: operation_id.to_owned(),
            local_order_id,
            provider_order_id,
            status: status.into(),
            error_code,
            observed_at_ms: now_ms,
        };
        *outcome = if evidence.error_code.is_some() {
            ExecutionOutcome::Rejected(evidence)
        } else {
            ExecutionOutcome::Accepted(evidence)
        };
        Ok(())
    }

    pub fn record_reconciliation(&mut self, operation_id: String, matches: bool, now_ms: i64) {
        self.operations.insert(
            operation_id.clone(),
            ExecutionOutcome::Accepted(ProviderEvidence {
                provider: AdapterKind::OkxDemo,
                operation_id,
                local_order_id: None,
                provider_order_id: None,
                status: if matches { "reconciled" } else { "mismatch" }.to_owned(),
                error_code: if matches {
                    None
                } else {
                    Some("reconciliation_required".to_owned())
                },
                observed_at_ms: now_ms,
            }),
        );
        self.blocked = !matches;
    }

    pub fn record_provider_observation(
        &mut self,
        operation_id: String,
        provider_order_id: String,
        status: String,
        now_ms: i64,
    ) {
        self.operations.insert(
            operation_id.clone(),
            ExecutionOutcome::Accepted(ProviderEvidence {
                provider: AdapterKind::OkxDemo,
                operation_id,
                local_order_id: None,
                provider_order_id: Some(provider_order_id),
                status,
                error_code: None,
                observed_at_ms: now_ms,
            }),
        );
    }

    pub fn reconcile(&mut self, matches: bool) {
        self.blocked = !matches;
    }

    pub fn block_for_recovery(&mut self) {
        self.blocked = true;
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct CapabilitySnapshot {
    pub adapter: AdapterKind,
    pub market: Market,
    pub currency: Currency,
    pub funding_target: Decimal,
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
            funding_target: Decimal::new(1_000_000, 0),
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
#[derive(Clone, Debug, Deserialize, Serialize)]
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

    pub fn account_id(&self) -> &str {
        &self.account.account_id
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

    pub fn require_reconciliation(&mut self) {
        self.reconciliation = ReconciliationState::Required;
    }
    pub fn orders(&self) -> impl Iterator<Item = &Order> {
        self.orders.values()
    }
    pub fn fills(&self) -> &[Fill] {
        &self.fills
    }

    /// Imports a provider-owned open order while preserving local reservation
    /// accounting. Reconciliation may update an existing provider order as
    /// its filled quantity changes.
    pub fn upsert_provider_order(&mut self, order: Order) -> Result<(), LedgerError> {
        if order.account_id != self.account.account_id {
            return Err(LedgerError::AccountMismatch);
        }
        if order.order_id.trim().is_empty()
            || order.instrument.trim().is_empty()
            || order.quantity <= Decimal::ZERO
            || order.limit_price <= Decimal::ZERO
            || order.filled_quantity < Decimal::ZERO
            || order.filled_quantity > order.quantity
        {
            return Err(LedgerError::InvalidInput("invalid provider order"));
        }
        let next_reserved = Self::order_reserved_cash(&order);
        let previous_reserved = self
            .orders
            .get(&order.order_id)
            .map(Self::order_reserved_cash)
            .unwrap_or(Decimal::ZERO);
        self.reserved_cash = self.reserved_cash - previous_reserved + next_reserved;
        self.orders.insert(order.order_id.clone(), order);
        Ok(())
    }

    pub fn cancel_missing_provider_orders(&mut self, active_order_ids: &[String]) {
        let stale_ids: Vec<String> = self
            .orders
            .keys()
            .filter(|order_id| {
                order_id.starts_with("provider-order-")
                    && !active_order_ids.iter().any(|active| active == *order_id)
            })
            .cloned()
            .collect();
        for order_id in stale_ids {
            let released = self.orders.get_mut(&order_id).map(|order| {
                if matches!(
                    order.status,
                    OrderStatus::Accepted | OrderStatus::PartiallyFilled
                ) {
                    let released = Self::order_reserved_cash(order);
                    order.status = OrderStatus::Cancelled;
                    released
                } else {
                    Decimal::ZERO
                }
            });
            self.reserved_cash -= released.unwrap_or(Decimal::ZERO);
        }
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
        self.account = snapshot;
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

    fn order_reserved_cash(order: &Order) -> Decimal {
        if order.side == Side::Buy
            && matches!(
                order.status,
                OrderStatus::Accepted | OrderStatus::PartiallyFilled
            )
        {
            (order.quantity - order.filled_quantity) * order.limit_price
        } else {
            Decimal::ZERO
        }
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
    fn provider_order_updates_reserved_cash_without_synthesizing_a_fill() {
        let mut ledger = PaperLedger::new(account(Market::OkxSpot)).unwrap();
        ledger
            .upsert_provider_order(Order {
                order_id: "provider-order-1".into(),
                account_id: "acct".into(),
                instrument: "BTC-USDT".into(),
                side: Side::Buy,
                quantity: Decimal::new(10, 0),
                filled_quantity: Decimal::new(2, 0),
                limit_price: Decimal::new(100, 0),
                status: OrderStatus::PartiallyFilled,
                submitted_at_ms: 1,
            })
            .unwrap();
        assert_eq!(ledger.reserved_cash(), Decimal::new(800, 0));
        assert!(ledger.fills().is_empty());
        ledger
            .upsert_provider_order(Order {
                order_id: "provider-order-1".into(),
                account_id: "acct".into(),
                instrument: "BTC-USDT".into(),
                side: Side::Buy,
                quantity: Decimal::new(10, 0),
                filled_quantity: Decimal::new(8, 0),
                limit_price: Decimal::new(100, 0),
                status: OrderStatus::PartiallyFilled,
                submitted_at_ms: 1,
            })
            .unwrap();
        assert_eq!(ledger.reserved_cash(), Decimal::new(200, 0));
        ledger.cancel_missing_provider_orders(&[]);
        assert_eq!(ledger.reserved_cash(), Decimal::ZERO);
        assert_eq!(
            ledger.orders().next().unwrap().status,
            OrderStatus::Cancelled
        );
    }
    #[test]
    fn mismatch_fails_closed_until_reconciled() {
        let mut l = PaperLedger::new(account(Market::UsEquity)).unwrap();
        let mut changed = l.account().clone();
        changed.cash -= Decimal::ONE;
        assert!(!l.reconcile(changed).unwrap());
        assert_eq!(l.reconciliation(), ReconciliationState::Required);
        assert_eq!(l.account().cash, Decimal::new(999_999, 0));
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

    #[test]
    fn okx_execution_is_risk_gated_idempotent_and_fail_closed() {
        let mut ledger = PaperLedger::new(account(Market::OkxSpot)).unwrap();
        let mut execution = PaperExecution::okx_demo(RiskPolicy {
            max_order_notional: Decimal::new(10_000, 0),
            reserve_cash: Decimal::new(100, 0),
            freeze_new_risk: false,
        })
        .unwrap();
        let (id, decision) = execution
            .begin(
                "op-1",
                &mut ledger,
                "BTC-USDT",
                Side::Buy,
                Decimal::ONE,
                Decimal::new(100, 0),
                1,
            )
            .unwrap();
        assert_eq!(id, "op-1");
        assert!(decision.approved);
        assert!(matches!(
            execution.begin(
                "op-1",
                &mut ledger,
                "BTC-USDT",
                Side::Buy,
                Decimal::ONE,
                Decimal::ONE,
                2
            ),
            Err(ExecutionError::DuplicateOperation)
        ));
        execution.mark_uncertain("op-1", 3).unwrap();
        assert!(execution.is_blocked());
        assert!(matches!(
            execution.begin(
                "op-2",
                &mut ledger,
                "BTC-USDT",
                Side::Buy,
                Decimal::ONE,
                Decimal::ONE,
                4
            ),
            Err(ExecutionError::ReconciliationRequired)
        ));
        execution.reconcile(true);
        ledger.reconcile(ledger.account().clone()).unwrap();
        assert!(!execution.is_blocked());
    }

    #[test]
    fn okx_execution_rejects_non_venue_and_over_limit_orders() {
        let mut ledger = PaperLedger::new(account(Market::OkxSpot)).unwrap();
        let mut execution = PaperExecution::okx_demo(RiskPolicy {
            max_order_notional: Decimal::new(10, 0),
            reserve_cash: Decimal::ZERO,
            freeze_new_risk: false,
        })
        .unwrap();
        assert!(matches!(
            execution.begin(
                "op-1",
                &mut ledger,
                "AAPL",
                Side::Buy,
                Decimal::ONE,
                Decimal::ONE,
                1
            ),
            Err(ExecutionError::VenueMismatch)
        ));
        assert!(matches!(
            execution.begin(
                "op-2",
                &mut ledger,
                "BTC-USDT",
                Side::Buy,
                Decimal::new(11, 0),
                Decimal::ONE,
                1
            ),
            Err(ExecutionError::RiskRejected(_))
        ));
    }
}
