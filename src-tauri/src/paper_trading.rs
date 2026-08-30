use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};

use adaq_paper_trading_core::{
    AccountSnapshot, Currency, ExecutionError, ExecutionOutcome, Fill, FillEvidence, Market,
    OrderStatus, PaperExecution, PaperLedger, Position, ReconciliationState, RiskDecision,
    RiskPolicy, Side,
};
use adaq_trading_crypto::{Exchange, Params};
use rusqlite::{Connection, params};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

const DEFAULT_MAX_ORDER_NOTIONAL: Decimal = Decimal::from_parts(100_000, 0, 0, false, 0);

#[derive(Clone)]
pub(crate) struct PaperTradingStore {
    database: Arc<Mutex<Connection>>,
    restarted_users: Arc<Mutex<HashSet<String>>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PaperAccountView {
    pub account: AccountSnapshot,
    pub reserved_cash: Decimal,
    pub buying_power: Decimal,
    pub reconciliation: ReconciliationState,
    pub orders: Vec<adaq_paper_trading_core::Order>,
    pub fills: Vec<adaq_paper_trading_core::Fill>,
    pub provider_evidence: Vec<ExecutionOutcome>,
    pub risk_decisions: Vec<RetainedRiskDecision>,
    pub restart_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RetainedRiskDecision {
    pub approved: bool,
    pub reason: String,
    pub requested_notional: Decimal,
    pub approved_notional: Decimal,
    pub decided_at_ms: i64,
}

impl RetainedRiskDecision {
    fn from_decision(decision: RiskDecision, decided_at_ms: i64) -> Self {
        Self {
            approved: decision.approved,
            reason: decision.reason,
            requested_notional: decision.requested_notional,
            approved_notional: decision.approved_notional,
            decided_at_ms,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PaperOrderRequest {
    pub user_id: String,
    pub operation_id: String,
    pub instrument: String,
    pub side: String,
    pub quantity: Decimal,
    pub limit_price: Decimal,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PaperCancelRequest {
    pub user_id: String,
    pub operation_id: String,
    pub instrument: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PaperSyncRequest {
    pub user_id: String,
    pub operation_id: String,
    pub instrument: String,
}

impl PaperTradingStore {
    pub(crate) fn open(database: Arc<Mutex<Connection>>) -> Result<Self, String> {
        let database_guard = database.lock().map_err(|error| error.to_string())?;
        database_guard
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS paper_accounts (
                    user_id TEXT PRIMARY KEY,
                    account_json TEXT NOT NULL,
                    execution_json TEXT NOT NULL,
                    updated_at_ms INTEGER NOT NULL
                );
                CREATE TABLE IF NOT EXISTS paper_risk_decisions (
                    user_id TEXT NOT NULL,
                    decision_json TEXT NOT NULL,
                    decided_at_ms INTEGER NOT NULL
                );
                CREATE INDEX IF NOT EXISTS paper_risk_decisions_user_time
                    ON paper_risk_decisions(user_id, decided_at_ms);",
            )
            .map_err(|error| error.to_string())?;
        let rows: Vec<(String, String, String)> = {
            let mut statement = database_guard
                .prepare("SELECT user_id, account_json, execution_json FROM paper_accounts")
                .map_err(|error| error.to_string())?;
            statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                .map_err(|error| error.to_string())?
                .collect::<Result<_, _>>()
                .map_err(|error| error.to_string())?
        };
        let mut restarted_users = HashSet::new();
        for (user_id, account_json, execution_json) in rows {
            let mut ledger: PaperLedger =
                serde_json::from_str(&account_json).map_err(|error| error.to_string())?;
            let mut execution: PaperExecution =
                serde_json::from_str(&execution_json).map_err(|error| error.to_string())?;
            ledger.require_reconciliation();
            execution.block_for_recovery();
            database_guard
                .execute(
                    "UPDATE paper_accounts SET account_json = ?1, execution_json = ?2 WHERE user_id = ?3",
                    params![
                        serde_json::to_string(&ledger).map_err(|error| error.to_string())?,
                        serde_json::to_string(&execution).map_err(|error| error.to_string())?,
                        user_id,
                    ],
                )
                .map_err(|error| error.to_string())?;
            restarted_users.insert(user_id);
        }
        drop(database_guard);
        Ok(Self {
            database,
            restarted_users: Arc::new(Mutex::new(restarted_users)),
        })
    }

    pub(crate) fn view(&self, user_id: &str) -> Result<PaperAccountView, String> {
        let (ledger, execution) = self.load(user_id)?;
        Ok(PaperAccountView {
            account: ledger.account().clone(),
            reserved_cash: ledger.reserved_cash(),
            buying_power: ledger.buying_power(),
            reconciliation: ledger.reconciliation(),
            orders: ledger.orders().cloned().collect(),
            fills: ledger.fills().to_vec(),
            provider_evidence: execution.evidence().cloned().collect(),
            risk_decisions: self.risk_decisions(user_id)?,
            restart_required: self
                .restarted_users
                .lock()
                .map_err(|error| error.to_string())?
                .contains(user_id),
        })
    }

    pub(crate) fn view_optional(&self, user_id: &str) -> Result<Option<PaperAccountView>, String> {
        if !self.has_account(user_id)? {
            return Ok(None);
        }
        self.view(user_id).map(Some)
    }

    pub(crate) fn create_account(
        &self,
        user_id: &str,
        account: AccountSnapshot,
        now_ms: i64,
    ) -> Result<PaperAccountView, String> {
        let ledger = PaperLedger::new(account).map_err(|error| error.to_string())?;
        let execution = PaperExecution::okx_demo(RiskPolicy {
            max_order_notional: DEFAULT_MAX_ORDER_NOTIONAL,
            reserve_cash: Decimal::ZERO,
            freeze_new_risk: false,
        })
        .map_err(|error| error.to_string())?;
        self.save(user_id, &ledger, &execution, now_ms)?;
        self.view(user_id)
    }

    pub(crate) fn begin_order(
        &self,
        request: &PaperOrderRequest,
        now_ms: i64,
    ) -> Result<PaperAccountView, String> {
        let mut ledger;
        let mut execution;
        match self.load(&request.user_id) {
            Ok((loaded_ledger, loaded_execution)) => {
                ledger = loaded_ledger;
                execution = loaded_execution;
            }
            Err(_) => return Err("The OKX Demo account must be reconciled before ordering.".into()),
        }
        let side = match request.side.as_str() {
            "buy" | "Buy" => Side::Buy,
            "sell" | "Sell" => Side::Sell,
            _ => return Err("side must be buy or sell".into()),
        };
        let decision = match execution.begin(
            request.operation_id.clone(),
            &mut ledger,
            &request.instrument,
            side,
            request.quantity,
            request.limit_price,
            now_ms,
        ) {
            Ok((_, decision)) => decision,
            Err(ExecutionError::RiskRejected(decision)) => {
                self.record_risk_decision(&request.user_id, decision.clone(), now_ms)?;
                return Err(ExecutionError::RiskRejected(decision).to_string());
            }
            Err(error) => return Err(error.to_string()),
        };
        self.record_risk_decision(&request.user_id, decision, now_ms)?;
        self.save(&request.user_id, &ledger, &execution, now_ms)?;
        self.view(&request.user_id)
    }

    pub(crate) fn record_order_result(
        &self,
        user_id: &str,
        operation_id: &str,
        provider_order_id: Option<String>,
        status: &str,
        error_code: Option<String>,
        now_ms: i64,
    ) -> Result<PaperAccountView, String> {
        let (ledger, mut execution) = self.load(user_id)?;
        execution
            .record_provider_outcome(
                operation_id,
                provider_order_id,
                status.to_owned(),
                error_code,
                now_ms,
            )
            .map_err(|error| error.to_string())?;
        self.save(user_id, &ledger, &execution, now_ms)?;
        self.view(user_id)
    }

    pub(crate) fn mark_uncertain(
        &self,
        user_id: &str,
        operation_id: &str,
        now_ms: i64,
    ) -> Result<PaperAccountView, String> {
        let (ledger, mut execution) = self.load(user_id)?;
        execution
            .mark_uncertain(operation_id, now_ms)
            .map_err(|error| error.to_string())?;
        self.save(user_id, &ledger, &execution, now_ms)?;
        self.view(user_id)
    }

    pub(crate) fn cancel_local_order(
        &self,
        user_id: &str,
        operation_id: &str,
        now_ms: i64,
    ) -> Result<PaperAccountView, String> {
        let (mut ledger, execution) = self.load(user_id)?;
        let order_id = execution
            .local_order_id(operation_id)
            .ok_or_else(|| "The execution operation has no local order.".to_owned())?;
        ledger
            .cancel_order(&order_id)
            .map_err(|error| error.to_string())?;
        self.save(user_id, &ledger, &execution, now_ms)?;
        self.view(user_id)
    }

    pub(crate) fn provider_order_id(
        &self,
        user_id: &str,
        operation_id: &str,
    ) -> Result<String, String> {
        self.load(user_id)?
            .1
            .provider_order_id(operation_id)
            .ok_or_else(|| "The execution operation has no provider order id.".to_owned())
    }

    pub(crate) fn sync_provider_order(
        &self,
        user_id: &str,
        operation_id: &str,
        remote: &adaq_trading_crypto::Order,
        now_ms: i64,
    ) -> Result<PaperAccountView, String> {
        let (mut ledger, execution) = self.load(user_id)?;
        let local_order_id = execution
            .local_order_id(operation_id)
            .ok_or_else(|| "The execution operation has no local order.".to_owned())?;
        let local = ledger
            .orders()
            .find(|order| order.order_id == local_order_id)
            .ok_or_else(|| "The local order is missing.".to_owned())?
            .clone();
        let delta = remote.filled.unwrap_or_default() - local.filled_quantity;
        if delta > Decimal::ZERO {
            ledger
                .apply_fill(Fill {
                    fill_id: format!(
                        "provider-{}-{}",
                        operation_id,
                        remote.filled.unwrap_or_default()
                    ),
                    order_id: local_order_id,
                    quantity: delta,
                    price: remote.average.or(remote.price).unwrap_or(local.limit_price),
                    fee: Decimal::ZERO,
                    evidence: FillEvidence::TradeObserved,
                    occurred_at_ms: remote.timestamp.unwrap_or(now_ms),
                })
                .map_err(|error| error.to_string())?;
        }
        self.save(user_id, &ledger, &execution, now_ms)?;
        self.view(user_id)
    }

    fn record_open_orders(
        &self,
        user_id: &str,
        orders: &[adaq_trading_crypto::Order],
        now_ms: i64,
    ) -> Result<PaperAccountView, String> {
        let (mut ledger, mut execution) = self.load(user_id)?;
        let active_order_ids = orders
            .iter()
            .filter_map(|order| order.id.as_ref())
            .map(|provider_order_id| format!("provider-order-{provider_order_id}"))
            .collect::<Vec<_>>();
        for (index, order) in orders.iter().enumerate() {
            if let Some(provider_order_id) = &order.id {
                if let Some(local_order) =
                    Self::provider_order_to_local(ledger.account_id(), order, now_ms)?
                {
                    ledger
                        .upsert_provider_order(local_order)
                        .map_err(|error| error.to_string())?;
                }
                execution.record_provider_observation(
                    format!("reconcile-order-{now_ms}-{index}"),
                    provider_order_id.clone(),
                    order.status.as_deref().unwrap_or("unknown").to_owned(),
                    now_ms,
                );
            }
        }
        ledger.cancel_missing_provider_orders(&active_order_ids);
        self.save(user_id, &ledger, &execution, now_ms)?;
        self.view(user_id)
    }

    fn provider_order_to_local(
        account_id: &str,
        order: &adaq_trading_crypto::Order,
        now_ms: i64,
    ) -> Result<Option<adaq_paper_trading_core::Order>, String> {
        let Some(provider_order_id) = &order.id else {
            return Ok(None);
        };
        let instrument = order
            .symbol
            .as_deref()
            .map(|symbol| symbol.replace('/', "-"))
            .filter(|symbol| !symbol.trim().is_empty())
            .ok_or_else(|| "OKX Demo returned an open order without an instrument.".to_owned())?;
        let side = match order
            .side
            .as_deref()
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("buy") => Side::Buy,
            Some("sell") => Side::Sell,
            _ => return Err("OKX Demo returned an open order with an invalid side.".to_owned()),
        };
        let quantity = order
            .amount
            .or_else(|| match (order.filled, order.remaining) {
                (Some(filled), Some(remaining)) => Some(filled + remaining),
                _ => None,
            })
            .ok_or_else(|| "OKX Demo returned an open order without a quantity.".to_owned())?;
        let filled_quantity = order.filled.unwrap_or_default();
        let limit_price = order
            .price
            .or(order.average)
            .or_else(|| {
                order
                    .cost
                    .filter(|_| quantity > Decimal::ZERO)
                    .map(|cost| cost / quantity)
            })
            .ok_or_else(|| "OKX Demo returned an open order without a price.".to_owned())?;
        let status = match order
            .status
            .as_deref()
            .unwrap_or("open")
            .to_ascii_lowercase()
            .as_str()
        {
            "open" | "new" | "pending" => {
                if filled_quantity >= quantity {
                    OrderStatus::Filled
                } else if filled_quantity > Decimal::ZERO {
                    OrderStatus::PartiallyFilled
                } else {
                    OrderStatus::Accepted
                }
            }
            "partially_filled" | "partially-filled" => OrderStatus::PartiallyFilled,
            "closed" | "filled" => OrderStatus::Filled,
            "canceled" | "cancelled" | "expired" => OrderStatus::Cancelled,
            "rejected" => OrderStatus::Rejected,
            _ => return Err("OKX Demo returned an open order with an unknown status.".to_owned()),
        };
        Ok(Some(adaq_paper_trading_core::Order {
            order_id: format!("provider-order-{provider_order_id}"),
            account_id: account_id.to_owned(),
            instrument,
            side,
            quantity,
            filled_quantity,
            limit_price,
            status,
            submitted_at_ms: order.timestamp.unwrap_or(now_ms),
        }))
    }

    pub(crate) fn reconcile(
        &self,
        user_id: &str,
        snapshot: AccountSnapshot,
        now_ms: i64,
    ) -> Result<PaperAccountView, String> {
        let (mut ledger, mut execution) = self.load(user_id)?;
        let matches = ledger
            .reconcile(snapshot)
            .map_err(|error| error.to_string())?;
        execution.record_reconciliation(format!("reconcile-{now_ms}"), matches, now_ms);
        self.save(user_id, &ledger, &execution, now_ms)?;
        self.restarted_users
            .lock()
            .map_err(|error| error.to_string())?
            .remove(user_id);
        self.view(user_id)
    }

    pub(crate) fn snapshot_from_balance(
        user_id: &str,
        account_id: String,
        balances: &adaq_trading_crypto::Balances,
        now_ms: i64,
    ) -> Result<AccountSnapshot, String> {
        let usdt = balances
            .accounts
            .get("USDT")
            .ok_or_else(|| "OKX Demo did not report a USDT balance.".to_owned())?;
        let mut positions = std::collections::BTreeMap::new();
        for (currency, balance) in &balances.accounts {
            if currency != "USDT" {
                let quantity = balance.total.unwrap_or_default();
                if quantity > Decimal::ZERO {
                    positions.insert(
                        format!("{currency}-USDT"),
                        Position {
                            quantity,
                            sellable_quantity: balance.free.unwrap_or(quantity),
                        },
                    );
                }
            }
        }
        Ok(AccountSnapshot {
            account_id,
            user_id: user_id.to_owned(),
            market: Market::OkxSpot,
            currency: Currency::Usdt,
            cash: usdt.total.or(usdt.free).unwrap_or_default(),
            positions,
            observed_at_ms: now_ms,
        })
    }

    fn load(&self, user_id: &str) -> Result<(PaperLedger, PaperExecution), String> {
        let database = self.database.lock().map_err(|error| error.to_string())?;
        let row = database
            .query_row(
                "SELECT account_json, execution_json FROM paper_accounts WHERE user_id = ?1",
                [user_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .map_err(|error| format!("paper account is unavailable: {error}"))?;
        let ledger = serde_json::from_str(&row.0).map_err(|error| error.to_string())?;
        let execution: PaperExecution =
            serde_json::from_str(&row.1).map_err(|error| error.to_string())?;
        Ok((ledger, execution))
    }

    fn has_account(&self, user_id: &str) -> Result<bool, String> {
        self.database
            .lock()
            .map_err(|error| error.to_string())?
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM paper_accounts WHERE user_id = ?1)",
                [user_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())
    }

    fn risk_decisions(&self, user_id: &str) -> Result<Vec<RetainedRiskDecision>, String> {
        let database = self.database.lock().map_err(|error| error.to_string())?;
        let mut statement = database
            .prepare(
                "SELECT decision_json FROM paper_risk_decisions
                 WHERE user_id = ?1 ORDER BY decided_at_ms DESC",
            )
            .map_err(|error| error.to_string())?;
        statement
            .query_map([user_id], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?
            .map(|row| {
                let json = row.map_err(|error| error.to_string())?;
                serde_json::from_str(&json).map_err(|error| error.to_string())
            })
            .collect()
    }

    fn record_risk_decision(
        &self,
        user_id: &str,
        decision: RiskDecision,
        decided_at_ms: i64,
    ) -> Result<(), String> {
        let decision = RetainedRiskDecision::from_decision(decision, decided_at_ms);
        self.database
            .lock()
            .map_err(|error| error.to_string())?
            .execute(
                "INSERT INTO paper_risk_decisions(user_id, decision_json, decided_at_ms)
                 VALUES (?1, ?2, ?3)",
                params![
                    user_id,
                    serde_json::to_string(&decision).map_err(|error| error.to_string())?,
                    decided_at_ms,
                ],
            )
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn save(
        &self,
        user_id: &str,
        ledger: &PaperLedger,
        execution: &PaperExecution,
        now_ms: i64,
    ) -> Result<(), String> {
        let account_json = serde_json::to_string(ledger).map_err(|error| error.to_string())?;
        let execution_json = serde_json::to_string(execution).map_err(|error| error.to_string())?;
        self.database
            .lock()
            .map_err(|error| error.to_string())?
            .execute(
                "INSERT INTO paper_accounts(user_id, account_json, execution_json, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(user_id) DO UPDATE SET account_json=excluded.account_json,
                    execution_json=excluded.execution_json, updated_at_ms=excluded.updated_at_ms",
                params![user_id, account_json, execution_json, now_ms],
            )
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn account() -> AccountSnapshot {
        AccountSnapshot {
            account_id: "okx-demo-account".into(),
            user_id: "alice".into(),
            market: Market::OkxSpot,
            currency: Currency::Usdt,
            cash: Decimal::new(1_000_000, 0),
            positions: BTreeMap::new(),
            observed_at_ms: 1,
        }
    }

    #[test]
    fn sqlite_state_survives_restart_and_fails_closed_until_reconciled() {
        let database = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let store = PaperTradingStore::open(database.clone()).unwrap();
        assert!(store.view_optional("alice").unwrap().is_none());
        store.create_account("alice", account(), 1).unwrap();
        store
            .begin_order(
                &PaperOrderRequest {
                    user_id: "alice".into(),
                    operation_id: "op-1".into(),
                    instrument: "BTC-USDT".into(),
                    side: "buy".into(),
                    quantity: Decimal::ONE,
                    limit_price: Decimal::new(100, 0),
                },
                2,
            )
            .unwrap();
        assert_eq!(store.view("alice").unwrap().risk_decisions.len(), 1);
        drop(store);

        let restarted = PaperTradingStore::open(database).unwrap();
        let view = restarted.view("alice").unwrap();
        assert_eq!(view.reconciliation, ReconciliationState::Required);
        assert_eq!(view.reserved_cash, Decimal::new(100, 0));
        assert!(view.restart_required);
        assert_eq!(view.risk_decisions.len(), 1);
        restarted.reconcile("alice", account(), 3).unwrap();
        let reconciled = restarted.view("alice").unwrap();
        assert_eq!(reconciled.reconciliation, ReconciliationState::Reconciled);
        assert!(!reconciled.restart_required);
    }

    #[test]
    fn provider_order_sync_retains_partial_fill_evidence() {
        let database = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let store = PaperTradingStore::open(database).unwrap();
        store.create_account("alice", account(), 1).unwrap();
        store
            .begin_order(
                &PaperOrderRequest {
                    user_id: "alice".into(),
                    operation_id: "op-1".into(),
                    instrument: "BTC-USDT".into(),
                    side: "buy".into(),
                    quantity: Decimal::new(10, 0),
                    limit_price: Decimal::new(100, 0),
                },
                2,
            )
            .unwrap();
        store
            .record_order_result("alice", "op-1", Some("remote-1".into()), "open", None, 3)
            .unwrap();
        store
            .sync_provider_order(
                "alice",
                "op-1",
                &adaq_trading_crypto::Order {
                    id: Some("remote-1".into()),
                    filled: Some(Decimal::new(4, 0)),
                    average: Some(Decimal::new(99, 0)),
                    timestamp: Some(4),
                    ..Default::default()
                },
                4,
            )
            .unwrap();
        let view = store.view("alice").unwrap();
        assert_eq!(view.fills.len(), 1);
        assert_eq!(
            view.orders[0].status,
            adaq_paper_trading_core::OrderStatus::PartiallyFilled
        );
    }

    #[test]
    fn first_provider_reconcile_retains_open_order_and_cash_reservation() {
        let database = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let store = PaperTradingStore::open(database).unwrap();
        store.create_account("alice", account(), 1).unwrap();
        store
            .record_open_orders(
                "alice",
                &[adaq_trading_crypto::Order {
                    id: Some("remote-1".into()),
                    symbol: Some("BTC/USDT".into()),
                    side: Some("buy".into()),
                    amount: Some(Decimal::new(2, 0)),
                    filled: Some(Decimal::ONE),
                    price: Some(Decimal::new(100, 0)),
                    status: Some("open".into()),
                    ..Default::default()
                }],
                2,
            )
            .unwrap();
        let view = store.view("alice").unwrap();
        assert_eq!(view.orders.len(), 1);
        assert_eq!(view.reserved_cash, Decimal::new(100, 0));
        assert_eq!(view.orders[0].instrument, "BTC-USDT");
        assert_eq!(
            view.orders[0].status,
            adaq_paper_trading_core::OrderStatus::PartiallyFilled
        );
    }
}

impl PaperTradingStore {
    pub(crate) fn provider_balance(
        &self,
        user_id: &str,
        account_id: String,
        client: &adaq_trading_crypto::adapters::okx::Okx,
        now_ms: i64,
    ) -> Result<PaperAccountView, String> {
        let balances = tauri::async_runtime::block_on(client.fetch_balance(Params::new()))
            .map_err(|error| error.to_string())?;
        let open_orders = tauri::async_runtime::block_on(client.fetch_open_orders(
            None,
            None,
            None,
            Params::new(),
        ))
        .map_err(|error| format!("OKX Demo open-order reconciliation failed: {error}"))?;
        let snapshot = Self::snapshot_from_balance(user_id, account_id, &balances, now_ms)?;
        if self.has_account(user_id)? {
            self.reconcile(user_id, snapshot, now_ms)?;
            self.record_open_orders(user_id, &open_orders, now_ms)
        } else {
            self.create_account(user_id, snapshot, now_ms)?;
            self.record_open_orders(user_id, &open_orders, now_ms)
        }
    }
}
