use std::{
    collections::{BTreeSet, HashSet},
    sync::{Arc, Mutex},
};

use adaq_paper_trading_core::{
    AccountSnapshot, Currency, ExecutionError, ExecutionOutcome, Fill, FillEvidence, Market,
    OrderStatus, PaperExecution, PaperLedger, Position, ReconciliationState, RiskDecision,
    RiskPolicy, Side,
};
use rusqlite::{Connection, params};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

const DEFAULT_MAX_ORDER_NOTIONAL: Decimal = Decimal::from_parts(100_000, 0, 0, false, 0);

#[derive(Clone)]
pub(crate) struct PaperTradingStore {
    database: Arc<Mutex<Connection>>,
    restarted_users: Arc<Mutex<HashSet<String>>>,
    order_gate: Arc<Mutex<()>>,
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

#[derive(Debug, Clone)]
pub(crate) struct ProviderOpenOrder {
    pub local_order_ids: Vec<String>,
    pub operation_ids: Vec<String>,
    pub provider_order_id: Option<String>,
    pub instrument: String,
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
            order_gate: Arc::new(Mutex::new(())),
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
        self.begin_order_with_policy(request, None, now_ms)
    }

    pub(crate) fn begin_order_with_policy(
        &self,
        request: &PaperOrderRequest,
        expected_policy: Option<&RiskPolicy>,
        now_ms: i64,
    ) -> Result<PaperAccountView, String> {
        // ponytail: one process-wide gate keeps this low-volume shared account exact; split by account if throughput becomes material.
        let _order_gate = self
            .order_gate
            .lock()
            .map_err(|error| format!("paper order lock failed: {error}"))?;
        let mut ledger;
        let mut execution;
        match self.load(&request.user_id) {
            Ok((loaded_ledger, loaded_execution)) => {
                ledger = loaded_ledger;
                execution = loaded_execution;
            }
            Err(_) => return Err("The OKX Demo account must be reconciled before ordering.".into()),
        }
        let policy = expected_policy
            .cloned()
            .unwrap_or_else(|| execution.policy().clone());
        let side = match request.side.as_str() {
            "buy" | "Buy" => Side::Buy,
            "sell" | "Sell" => Side::Sell,
            _ => return Err("side must be buy or sell".into()),
        };
        let decision = match execution.begin_with_policy(
            request.operation_id.clone(),
            &mut ledger,
            &request.instrument,
            side,
            request.quantity,
            request.limit_price,
            &policy,
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
        let (mut ledger, mut execution) = self.load(user_id)?;
        ledger.require_reconciliation();
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

    pub(crate) fn provider_open_orders_for(
        &self,
        user_id: &str,
        instrument_scope: &BTreeSet<String>,
        operation_prefix: &str,
    ) -> Result<Vec<ProviderOpenOrder>, String> {
        let (ledger, execution) = self.load(user_id)?;
        let mut orders: Vec<ProviderOpenOrder> = Vec::new();
        for order in ledger.orders().filter(|order| {
            matches!(
                order.status,
                OrderStatus::Accepted | OrderStatus::PartiallyFilled
            ) && instrument_scope.contains(&order.instrument)
        }) {
            let bot_evidence = execution.evidence().find_map(|outcome| {
                let evidence = match outcome {
                    ExecutionOutcome::Accepted(evidence)
                    | ExecutionOutcome::Rejected(evidence)
                    | ExecutionOutcome::Uncertain(evidence) => evidence,
                };
                (evidence.operation_id.starts_with(operation_prefix)
                    && evidence.local_order_id.as_deref() == Some(order.order_id.as_str()))
                .then_some(evidence)
            });
            let Some(bot_evidence) = bot_evidence else {
                continue;
            };
            let operation_id = bot_evidence.operation_id.clone();
            let provider_order_id = order
                .order_id
                .strip_prefix("provider-order-")
                .filter(|id| !id.is_empty())
                .map(str::to_owned)
                .or_else(|| bot_evidence.provider_order_id.clone());
            if let Some(provider_order_id) = provider_order_id {
                if let Some(index) = orders.iter().position(|existing| {
                    existing.provider_order_id.as_deref() == Some(provider_order_id.as_str())
                }) {
                    orders[index].local_order_ids.push(order.order_id.clone());
                    orders[index].operation_ids.push(operation_id);
                } else {
                    orders.push(ProviderOpenOrder {
                        local_order_ids: vec![order.order_id.clone()],
                        operation_ids: vec![operation_id],
                        provider_order_id: Some(provider_order_id),
                        instrument: order.instrument.clone(),
                    });
                }
            } else {
                orders.push(ProviderOpenOrder {
                    local_order_ids: vec![order.order_id.clone()],
                    operation_ids: vec![operation_id],
                    provider_order_id: None,
                    instrument: order.instrument.clone(),
                });
            }
        }
        Ok(orders)
    }

    pub(crate) fn mark_provider_order_uncertain(
        &self,
        user_id: &str,
        provider_order_id: &str,
        now_ms: i64,
    ) -> Result<PaperAccountView, String> {
        let (mut ledger, mut execution) = self.load(user_id)?;
        ledger.require_reconciliation();
        let operation_id = format!(
            "provider-uncertain-{now_ms}-{}",
            provider_order_id.chars().take(96).collect::<String>()
        );
        execution.record_provider_observation(
            operation_id.clone(),
            provider_order_id.to_owned(),
            "unknown".into(),
            now_ms,
        );
        execution
            .mark_uncertain(&operation_id, now_ms)
            .map_err(|error| error.to_string())?;
        self.save(user_id, &ledger, &execution, now_ms)?;
        self.view(user_id)
    }

    fn provider_sync_failure(
        &self,
        user_id: &str,
        operation_id: &str,
        provider_order_id: Option<&str>,
        now_ms: i64,
        error: String,
    ) -> String {
        let retained = match provider_order_id {
            Some(provider_order_id) => self
                .mark_provider_order_uncertain(user_id, provider_order_id, now_ms)
                .map(|_| ()),
            None => self
                .mark_uncertain(user_id, operation_id, now_ms)
                .map(|_| ()),
        };
        match retained {
            Ok(()) => error,
            Err(mark_error) => {
                format!("{error}; provider sync uncertainty was not retained: {mark_error}")
            }
        }
    }

    fn reconciliation_failure(
        &self,
        user_id: &str,
        ledger: &mut PaperLedger,
        execution: &mut PaperExecution,
        now_ms: i64,
        error: String,
    ) -> Result<PaperAccountView, String> {
        ledger.require_reconciliation();
        execution.block_for_recovery();
        self.save(user_id, ledger, execution, now_ms)?;
        Err(error)
    }

    pub(crate) fn require_reconciliation(
        &self,
        user_id: &str,
        now_ms: i64,
    ) -> Result<PaperAccountView, String> {
        let (mut ledger, mut execution) = self.load(user_id)?;
        ledger.require_reconciliation();
        execution.record_reconciliation(
            format!("host-reconciliation-required-{now_ms}"),
            false,
            now_ms,
        );
        self.save(user_id, &ledger, &execution, now_ms)?;
        self.view(user_id)
    }

    pub(crate) fn freeze_all(
        &self,
        user_id: &str,
        now_ms: i64,
    ) -> Result<Option<PaperAccountView>, String> {
        crate::user::validate_user(user_id)?;
        if !self.has_account(user_id)? {
            return Ok(None);
        }
        let (mut ledger, mut execution) = self.load(user_id)?;
        ledger.require_reconciliation();
        execution.freeze_new_risk();
        self.save(user_id, &ledger, &execution, now_ms)?;
        self.view(user_id).map(Some)
    }

    pub(crate) fn recover_after_host_freeze(
        &self,
        user_id: &str,
        now_ms: i64,
    ) -> Result<PaperAccountView, String> {
        let (ledger, mut execution) = self.load(user_id)?;
        if ledger.reconciliation() != ReconciliationState::Reconciled {
            return Err("The OKX Demo account must be reconciled before Host recovery.".into());
        }
        execution
            .recover_after_reconciliation()
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
        let trades = remote.trades.as_deref().unwrap_or(&[]);
        self.sync_provider_order_with_trades(user_id, operation_id, remote, trades, now_ms)
    }

    pub(crate) fn sync_provider_order_with_trades(
        &self,
        user_id: &str,
        operation_id: &str,
        remote: &adaq_trading_crypto::Order,
        trades: &[adaq_trading_crypto::Trade],
        now_ms: i64,
    ) -> Result<PaperAccountView, String> {
        let (mut ledger, mut execution) = self.load(user_id)?;
        let local_order_id = execution
            .local_order_id(operation_id)
            .ok_or_else(|| "The execution operation has no local order.".to_owned())?;
        let provider_order_id = remote
            .id
            .clone()
            .or_else(|| execution.provider_order_id(operation_id));
        let local = ledger
            .orders()
            .find(|order| order.order_id == local_order_id)
            .ok_or_else(|| "The local order is missing.".to_owned())?
            .clone();
        let remote_filled = match remote.filled {
            Some(filled) => filled,
            None => {
                return Err(self.provider_sync_failure(
                    user_id,
                    operation_id,
                    provider_order_id.as_deref(),
                    now_ms,
                    "Provider order has no exact filled quantity; reconciliation is required."
                        .into(),
                ));
            }
        };
        let delta = match remote_filled.checked_sub(local.filled_quantity) {
            Some(delta) => delta,
            None => {
                return Err(self.provider_sync_failure(
                    user_id,
                    operation_id,
                    provider_order_id.as_deref(),
                    now_ms,
                    "Provider order filled quantity could not be compared exactly; reconciliation is required."
                        .into(),
                ));
            }
        };
        if delta < Decimal::ZERO {
            ledger.require_reconciliation();
            execution.block_for_recovery();
            self.save(user_id, &ledger, &execution, now_ms)?;
            return Err(
                "Provider order filled quantity regressed; reconciliation is required.".into(),
            );
        }
        if delta > Decimal::ZERO && trades.is_empty() {
            ledger.require_reconciliation();
            execution.block_for_recovery();
            self.save(user_id, &ledger, &execution, now_ms)?;
            return Err(
                "Provider order reports new fills without per-trade evidence; reconciliation is required."
                    .into(),
            );
        }
        let mut applied = Decimal::ZERO;
        for trade in trades {
            if provider_order_id.as_deref() != trade.order.as_deref() {
                return Err(self.provider_sync_failure(
                    user_id,
                    operation_id,
                    provider_order_id.as_deref(),
                    now_ms,
                    "Provider fill has no exact matching order identity.".into(),
                ));
            }
            let quantity = match trade.amount {
                Some(quantity) => quantity,
                None => {
                    return Err(self.provider_sync_failure(
                        user_id,
                        operation_id,
                        provider_order_id.as_deref(),
                        now_ms,
                        "Provider fill has no exact quantity.".into(),
                    ));
                }
            };
            let price = match trade.price {
                Some(price) => price,
                None => {
                    return Err(self.provider_sync_failure(
                        user_id,
                        operation_id,
                        provider_order_id.as_deref(),
                        now_ms,
                        "Provider fill has no exact price.".into(),
                    ));
                }
            };
            if quantity <= Decimal::ZERO || price <= Decimal::ZERO {
                return Err(self.provider_sync_failure(
                    user_id,
                    operation_id,
                    provider_order_id.as_deref(),
                    now_ms,
                    "Provider fill has invalid quantity or price.".into(),
                ));
            }
            let trade_id = match trade.id.as_deref() {
                Some(trade_id) => trade_id,
                None => {
                    return Err(self.provider_sync_failure(
                        user_id,
                        operation_id,
                        provider_order_id.as_deref(),
                        now_ms,
                        "Provider fill has no stable trade id.".into(),
                    ));
                }
            };
            let fill_id = format!("provider-{operation_id}-trade-{trade_id}");
            let (fee, fee_asset, fee_quote, fee_amount) = Self::provider_fee_quote(
                remote.symbol.as_deref().or(trade.symbol.as_deref()),
                price,
                trade.fee.as_ref(),
            );
            if fee_amount.is_some() && fee_quote.is_none() {
                return Err(self.provider_sync_failure(
                    user_id,
                    operation_id,
                    provider_order_id.as_deref(),
                    now_ms,
                    "Provider fee has no exact USDT valuation; reconciliation is required.".into(),
                ));
            }
            if let Some(existing) = ledger.fills().iter().find(|fill| fill.fill_id == fill_id) {
                if existing.order_id.as_str() != local_order_id.as_str()
                    || existing.quantity != quantity
                    || existing.price != price
                    || existing.fee != fee
                    || existing.fee_asset != fee_asset
                    || existing.fee_quote != fee_quote
                    || existing.fee_amount != fee_amount
                {
                    return Err(self.provider_sync_failure(
                        user_id,
                        operation_id,
                        provider_order_id.as_deref(),
                        now_ms,
                        "Provider fill evidence changed for an existing trade; reconciliation is required."
                            .into(),
                    ));
                }
                continue;
            }
            if delta == Decimal::ZERO {
                return Err(self.provider_sync_failure(
                    user_id,
                    operation_id,
                    provider_order_id.as_deref(),
                    now_ms,
                    "Provider reports a new fill without a filled quantity increase; reconciliation is required."
                        .into(),
                ));
            }
            let next_applied = match applied.checked_add(quantity) {
                Some(next_applied) => next_applied,
                None => {
                    return Err(self.provider_sync_failure(
                        user_id,
                        operation_id,
                        provider_order_id.as_deref(),
                        now_ms,
                        "Provider fill quantities could not be summed exactly; reconciliation is required."
                            .into(),
                    ));
                }
            };
            if next_applied > delta {
                return Err(self.provider_sync_failure(
                    user_id,
                    operation_id,
                    provider_order_id.as_deref(),
                    now_ms,
                    "Provider fills exceed the order's newly reported quantity.".into(),
                ));
            }
            if let Err(error) = ledger.apply_fill(Fill {
                fill_id,
                order_id: local_order_id.clone(),
                quantity,
                price,
                fee,
                fee_asset,
                fee_quote,
                fee_amount,
                evidence: FillEvidence::TradeObserved,
                occurred_at_ms: trade.timestamp.or(remote.timestamp).unwrap_or(now_ms),
            }) {
                return Err(self.provider_sync_failure(
                    user_id,
                    operation_id,
                    provider_order_id.as_deref(),
                    now_ms,
                    error.to_string(),
                ));
            }
            applied = next_applied;
        }
        if delta > Decimal::ZERO {
            let remaining = match delta.checked_sub(applied) {
                Some(remaining) => remaining,
                None => {
                    return Err(self.provider_sync_failure(
                        user_id,
                        operation_id,
                        provider_order_id.as_deref(),
                        now_ms,
                        "Provider fill quantities could not be reconciled exactly; reconciliation is required."
                            .into(),
                    ));
                }
            };
            if remaining > Decimal::ZERO {
                ledger.require_reconciliation();
                execution.block_for_recovery();
                self.save(user_id, &ledger, &execution, now_ms)?;
                return Err(
                    "Provider order reports more fills than the retained per-trade evidence; reconciliation is required."
                        .into(),
                );
            }
        }
        let remote_status = remote
            .status
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if matches!(
            remote_status.as_str(),
            "canceled" | "cancelled" | "expired" | "rejected"
        ) && ledger
            .orders()
            .find(|order| order.order_id == local_order_id)
            .is_some_and(|order| {
                matches!(
                    order.status,
                    OrderStatus::Accepted | OrderStatus::PartiallyFilled
                )
            })
        {
            if let Err(error) = ledger.cancel_order(&local_order_id) {
                return Err(self.provider_sync_failure(
                    user_id,
                    operation_id,
                    provider_order_id.as_deref(),
                    now_ms,
                    error.to_string(),
                ));
            }
        }
        self.save(user_id, &ledger, &execution, now_ms)?;
        self.view(user_id)
    }

    fn provider_fee_quote(
        symbol: Option<&str>,
        price: Decimal,
        provider_fee: Option<&adaq_trading_crypto::Fee>,
    ) -> (Decimal, Option<String>, Option<Decimal>, Option<Decimal>) {
        let Some(provider_fee) = provider_fee else {
            return (Decimal::ZERO, None, None, None);
        };
        let asset = provider_fee.currency.clone();
        let Some(raw_amount) = provider_fee.cost else {
            return (Decimal::ZERO, asset, None, None);
        };
        let amount = if raw_amount < Decimal::ZERO {
            -raw_amount
        } else {
            raw_amount
        };
        let quote = symbol
            .and_then(|symbol| symbol.split(['-', '/']).nth(1))
            .map(str::to_owned);
        let base = symbol
            .and_then(|symbol| symbol.split(['-', '/']).next())
            .map(str::to_owned);
        let quote_fee = if amount.is_zero() {
            Some(Decimal::ZERO)
        } else {
            match (asset.as_deref(), quote.as_deref(), base.as_deref()) {
                (Some(asset), Some(quote), _) if asset.eq_ignore_ascii_case(quote) => Some(amount),
                (Some(asset), _, Some(base)) if asset.eq_ignore_ascii_case(base) => {
                    amount.checked_mul(price)
                }
                _ => None,
            }
        };
        match quote_fee {
            Some(quote_fee) => (quote_fee, asset, Some(quote_fee), Some(amount)),
            None => (Decimal::ZERO, asset, None, Some(amount)),
        }
    }

    fn record_open_orders(
        &self,
        user_id: &str,
        orders: &[adaq_trading_crypto::Order],
        now_ms: i64,
    ) -> Result<PaperAccountView, String> {
        let (mut ledger, mut execution) = self.load(user_id)?;
        let mut active_order_ids = Vec::new();
        for (index, order) in orders.iter().enumerate() {
            let Some(provider_order_id) = &order.id else {
                return self.reconciliation_failure(
                    user_id,
                    &mut ledger,
                    &mut execution,
                    now_ms,
                    "OKX Demo returned an open order without a provider identity.".into(),
                );
            };
            active_order_ids.push(format!("provider-order-{provider_order_id}"));
            if let Err(error) = Self::provider_order_quantities(order) {
                return self.reconciliation_failure(
                    user_id,
                    &mut ledger,
                    &mut execution,
                    now_ms,
                    error,
                );
            }
            let mapped_local_order_ids = execution
                .evidence()
                .filter_map(|outcome| {
                    let evidence = match outcome {
                        ExecutionOutcome::Accepted(evidence)
                        | ExecutionOutcome::Rejected(evidence)
                        | ExecutionOutcome::Uncertain(evidence) => evidence,
                    };
                    (evidence.provider_order_id.as_deref() == Some(provider_order_id.as_str()))
                        .then(|| evidence.local_order_id.clone())
                        .flatten()
                })
                .collect::<Vec<_>>();
            if mapped_local_order_ids.is_empty() {
                let local_order =
                    match Self::provider_order_to_local(ledger.account_id(), order, now_ms) {
                        Ok(local_order) => local_order,
                        Err(error) => {
                            return self.reconciliation_failure(
                                user_id,
                                &mut ledger,
                                &mut execution,
                                now_ms,
                                error,
                            );
                        }
                    };
                if let Some(local_order) = local_order
                    && let Err(error) = ledger.upsert_provider_order(local_order)
                {
                    return self.reconciliation_failure(
                        user_id,
                        &mut ledger,
                        &mut execution,
                        now_ms,
                        error.to_string(),
                    );
                }
            } else {
                active_order_ids.extend(mapped_local_order_ids);
            }
            execution.record_provider_observation(
                format!("reconcile-order-{now_ms}-{index}"),
                provider_order_id.clone(),
                order.status.as_deref().unwrap_or("unknown").to_owned(),
                now_ms,
            );
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
        let (quantity, filled_quantity) = Self::provider_order_quantities(order)?;
        let limit_price = order
            .price
            .or(order.average)
            .or_else(|| {
                order
                    .cost
                    .filter(|_| quantity > Decimal::ZERO)
                    .and_then(|cost| cost.checked_div(quantity))
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

    fn provider_order_quantities(
        order: &adaq_trading_crypto::Order,
    ) -> Result<(Decimal, Decimal), String> {
        let quantity = order
            .amount
            .or_else(|| match (order.filled, order.remaining) {
                (Some(filled), Some(remaining)) => filled.checked_add(remaining),
                _ => None,
            })
            .ok_or_else(|| "OKX Demo returned an open order without a quantity.".to_owned())?;
        let filled_quantity = order
            .filled
            .or_else(|| match (order.amount, order.remaining) {
                (Some(amount), Some(remaining)) => amount.checked_sub(remaining),
                _ => None,
            })
            .ok_or_else(|| {
                "OKX Demo returned an open order without an exact filled quantity.".to_owned()
            })?;
        if quantity <= Decimal::ZERO
            || filled_quantity < Decimal::ZERO
            || filled_quantity > quantity
        {
            return Err("OKX Demo returned an open order with invalid quantities.".into());
        }
        Ok((quantity, filled_quantity))
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
    use std::collections::{BTreeMap, BTreeSet};

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
        assert_eq!(view.risk_decisions.len(), 1);
        restarted.reconcile("alice", account(), 3).unwrap();
        let reconciled = restarted.view("alice").unwrap();
        assert_eq!(reconciled.reconciliation, ReconciliationState::Reconciled);
        assert!(!reconciled.restart_required);
    }

    #[test]
    fn insufficient_funds_reject_order_without_reservation() {
        let database = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let store = PaperTradingStore::open(database).unwrap();
        let mut snapshot = account();
        snapshot.cash = Decimal::new(100, 0);
        store.create_account("alice", snapshot, 1).unwrap();

        let result = store.begin_order(
            &PaperOrderRequest {
                user_id: "alice".into(),
                operation_id: "op-insufficient-funds".into(),
                instrument: "BTC-USDT".into(),
                side: "buy".into(),
                quantity: Decimal::new(2, 0),
                limit_price: Decimal::new(100, 0),
            },
            2,
        );
        assert!(result.is_err());

        let view = store.view("alice").unwrap();
        assert_eq!(view.orders.len(), 0);
        assert_eq!(view.reserved_cash, Decimal::ZERO);
        assert_eq!(view.risk_decisions.len(), 1);
        assert!(!view.risk_decisions[0].approved);
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
        assert!(
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
                .is_err()
        );
        let view = store.view("alice").unwrap();
        assert!(view.fills.is_empty());
        assert_eq!(
            view.reconciliation,
            adaq_paper_trading_core::ReconciliationState::Required
        );
        assert_eq!(
            view.orders[0].status,
            adaq_paper_trading_core::OrderStatus::Accepted
        );
    }

    #[test]
    fn provider_terminal_cancel_releases_local_reservation() {
        let database = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let store = PaperTradingStore::open(database).unwrap();
        store.create_account("alice", account(), 1).unwrap();
        store
            .begin_order(
                &PaperOrderRequest {
                    user_id: "alice".into(),
                    operation_id: "op-cancel".into(),
                    instrument: "BTC-USDT".into(),
                    side: "buy".into(),
                    quantity: Decimal::new(2, 0),
                    limit_price: Decimal::new(100, 0),
                },
                2,
            )
            .unwrap();
        store
            .record_order_result(
                "alice",
                "op-cancel",
                Some("remote-cancel".into()),
                "open",
                None,
                3,
            )
            .unwrap();

        store
            .sync_provider_order(
                "alice",
                "op-cancel",
                &adaq_trading_crypto::Order {
                    id: Some("remote-cancel".into()),
                    status: Some("canceled".into()),
                    filled: Some(Decimal::ZERO),
                    ..Default::default()
                },
                4,
            )
            .unwrap();

        let view = store.view("alice").unwrap();
        assert_eq!(view.reserved_cash, Decimal::ZERO);
        assert_eq!(view.orders[0].status, OrderStatus::Cancelled);
    }

    #[test]
    fn provider_missing_filled_quantity_blocks_until_reconciled() {
        let database = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let store = PaperTradingStore::open(database).unwrap();
        store.create_account("alice", account(), 1).unwrap();
        store
            .begin_order(
                &PaperOrderRequest {
                    user_id: "alice".into(),
                    operation_id: "op-missing-filled".into(),
                    instrument: "BTC-USDT".into(),
                    side: "buy".into(),
                    quantity: Decimal::new(2, 0),
                    limit_price: Decimal::new(100, 0),
                },
                2,
            )
            .unwrap();
        store
            .record_order_result(
                "alice",
                "op-missing-filled",
                Some("remote-missing-filled".into()),
                "open",
                None,
                3,
            )
            .unwrap();

        assert!(
            store
                .sync_provider_order(
                    "alice",
                    "op-missing-filled",
                    &adaq_trading_crypto::Order {
                        id: Some("remote-missing-filled".into()),
                        status: Some("canceled".into()),
                        ..Default::default()
                    },
                    4,
                )
                .is_err()
        );

        let view = store.view("alice").unwrap();
        assert_eq!(view.reconciliation, ReconciliationState::Required);
        assert!(view.provider_evidence.iter().any(|outcome| matches!(
            outcome,
            ExecutionOutcome::Uncertain(evidence)
                if evidence.provider_order_id.as_deref() == Some("remote-missing-filled")
        )));
        assert_eq!(view.orders[0].status, OrderStatus::Accepted);
    }

    #[test]
    fn provider_fill_regression_blocks_until_reconciled() {
        let database = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let store = PaperTradingStore::open(database).unwrap();
        store.create_account("alice", account(), 1).unwrap();
        store
            .begin_order(
                &PaperOrderRequest {
                    user_id: "alice".into(),
                    operation_id: "op-regression".into(),
                    instrument: "BTC-USDT".into(),
                    side: "buy".into(),
                    quantity: Decimal::new(2, 0),
                    limit_price: Decimal::new(100, 0),
                },
                2,
            )
            .unwrap();
        store
            .record_order_result(
                "alice",
                "op-regression",
                Some("remote-regression".into()),
                "open",
                None,
                3,
            )
            .unwrap();
        store
            .sync_provider_order_with_trades(
                "alice",
                "op-regression",
                &adaq_trading_crypto::Order {
                    id: Some("remote-regression".into()),
                    symbol: Some("BTC/USDT".into()),
                    filled: Some(Decimal::ONE),
                    ..Default::default()
                },
                &[adaq_trading_crypto::Trade {
                    id: Some("trade-regression".into()),
                    order: Some("remote-regression".into()),
                    symbol: Some("BTC/USDT".into()),
                    amount: Some(Decimal::ONE),
                    price: Some(Decimal::new(100, 0)),
                    ..Default::default()
                }],
                4,
            )
            .unwrap();

        assert!(
            store
                .sync_provider_order(
                    "alice",
                    "op-regression",
                    &adaq_trading_crypto::Order {
                        id: Some("remote-regression".into()),
                        filled: Some(Decimal::ZERO),
                        ..Default::default()
                    },
                    5,
                )
                .is_err()
        );
        let view = store.view("alice").unwrap();
        assert_eq!(
            view.reconciliation,
            adaq_paper_trading_core::ReconciliationState::Required
        );
    }

    #[test]
    fn provider_fill_validation_failure_is_retained_as_uncertain() {
        let database = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let store = PaperTradingStore::open(database).unwrap();
        store.create_account("alice", account(), 1).unwrap();
        store
            .begin_order(
                &PaperOrderRequest {
                    user_id: "alice".into(),
                    operation_id: "op-mismatch".into(),
                    instrument: "BTC-USDT".into(),
                    side: "buy".into(),
                    quantity: Decimal::new(2, 0),
                    limit_price: Decimal::new(100, 0),
                },
                2,
            )
            .unwrap();
        store
            .record_order_result(
                "alice",
                "op-mismatch",
                Some("remote-mismatch".into()),
                "open",
                None,
                3,
            )
            .unwrap();

        assert!(
            store
                .sync_provider_order_with_trades(
                    "alice",
                    "op-mismatch",
                    &adaq_trading_crypto::Order {
                        id: Some("remote-mismatch".into()),
                        symbol: Some("BTC/USDT".into()),
                        filled: Some(Decimal::ONE),
                        ..Default::default()
                    },
                    &[adaq_trading_crypto::Trade {
                        id: Some("trade-mismatch".into()),
                        order: Some("different-order".into()),
                        amount: Some(Decimal::ONE),
                        price: Some(Decimal::new(100, 0)),
                        ..Default::default()
                    }],
                    4,
                )
                .is_err()
        );

        let view = store.view("alice").unwrap();
        assert_eq!(
            view.reconciliation,
            adaq_paper_trading_core::ReconciliationState::Required
        );
        assert!(view.provider_evidence.iter().any(|outcome| matches!(
            outcome,
            ExecutionOutcome::Uncertain(evidence)
                if evidence.provider_order_id.as_deref() == Some("remote-mismatch")
        )));
    }

    #[test]
    fn bot_provider_order_mapping_preserves_local_order_identity() {
        let database = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let store = PaperTradingStore::open(database).unwrap();
        store.create_account("alice", account(), 1).unwrap();
        store
            .begin_order(
                &PaperOrderRequest {
                    user_id: "alice".into(),
                    operation_id: "bot-bot-a-order-1".into(),
                    instrument: "BTC-USDT".into(),
                    side: "buy".into(),
                    quantity: Decimal::ONE,
                    limit_price: Decimal::new(100, 0),
                },
                2,
            )
            .unwrap();
        store
            .record_order_result(
                "alice",
                "bot-bot-a-order-1",
                Some("remote-1".into()),
                "open",
                None,
                3,
            )
            .unwrap();

        let scope = BTreeSet::from(["BTC-USDT".to_owned()]);
        let open = store
            .provider_open_orders_for("alice", &scope, "bot-bot-a-")
            .unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].provider_order_id.as_deref(), Some("remote-1"));
        assert_eq!(open[0].local_order_ids, ["order-1"]);

        store
            .record_open_orders(
                "alice",
                &[adaq_trading_crypto::Order {
                    id: Some("remote-1".into()),
                    symbol: Some("BTC/USDT".into()),
                    side: Some("buy".into()),
                    amount: Some(Decimal::ONE),
                    filled: Some(Decimal::ZERO),
                    price: Some(Decimal::new(100, 0)),
                    status: Some("open".into()),
                    ..Default::default()
                }],
                4,
            )
            .unwrap();
        let view = store.view("alice").unwrap();
        assert_eq!(view.orders.len(), 1);
        assert_eq!(view.orders[0].order_id, "order-1");
        assert_eq!(view.reserved_cash, Decimal::new(100, 0));
    }

    #[test]
    fn provider_trade_sync_retains_exact_per_fill_fees_without_duplication() {
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
        let remote = adaq_trading_crypto::Order {
            id: Some("remote-1".into()),
            symbol: Some("BTC/USDT".into()),
            filled: Some(Decimal::new(6, 0)),
            average: Some(Decimal::new(99, 0)),
            timestamp: Some(4),
            ..Default::default()
        };
        let trades = vec![
            adaq_trading_crypto::Trade {
                id: Some("trade-1".into()),
                order: Some("remote-1".into()),
                timestamp: Some(4),
                symbol: Some("BTC/USDT".into()),
                price: Some(Decimal::new(99, 0)),
                amount: Some(Decimal::new(4, 0)),
                fee: Some(adaq_trading_crypto::Fee {
                    currency: Some("BTC".into()),
                    cost: Some(Decimal::new(-1, 3)),
                    rate: None,
                }),
                ..Default::default()
            },
            adaq_trading_crypto::Trade {
                id: Some("trade-2".into()),
                order: Some("remote-1".into()),
                timestamp: Some(5),
                symbol: Some("BTC/USDT".into()),
                price: Some(Decimal::new(98, 0)),
                amount: Some(Decimal::new(2, 0)),
                fee: Some(adaq_trading_crypto::Fee {
                    currency: Some("USDT".into()),
                    cost: Some(Decimal::new(2, 1)),
                    rate: None,
                }),
                ..Default::default()
            },
        ];
        store
            .sync_provider_order_with_trades("alice", "op-1", &remote, &trades, 6)
            .unwrap();
        let first = store.view("alice").unwrap();
        assert_eq!(first.fills.len(), 2);
        assert_eq!(first.fills[0].fee, Decimal::new(99, 3));
        assert_eq!(first.fills[0].fee_quote, Some(Decimal::new(99, 3)));
        assert_eq!(first.fills[1].fee_quote, Some(Decimal::new(2, 1)));
        assert_eq!(first.account.cash, Decimal::new(999_407_701, 3));

        store
            .sync_provider_order_with_trades("alice", "op-1", &remote, &trades, 7)
            .unwrap();
        let second = store.view("alice").unwrap();
        assert_eq!(second.fills.len(), 2);
        assert_eq!(second.account.cash, first.account.cash);

        let unexpected_trade = adaq_trading_crypto::Trade {
            id: Some("trade-3".into()),
            order: Some("remote-1".into()),
            timestamp: Some(6),
            symbol: Some("BTC/USDT".into()),
            price: Some(Decimal::new(97, 0)),
            amount: Some(Decimal::ONE),
            ..Default::default()
        };
        assert!(
            store
                .sync_provider_order_with_trades("alice", "op-1", &remote, &[unexpected_trade], 8)
                .is_err()
        );
        let blocked = store.view("alice").unwrap();
        assert_eq!(blocked.reconciliation, ReconciliationState::Required);
        assert!(blocked.provider_evidence.iter().any(|outcome| matches!(
            outcome,
            ExecutionOutcome::Uncertain(evidence)
                if evidence.provider_order_id.as_deref() == Some("remote-1")
        )));
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

    #[test]
    fn provider_open_order_without_exact_fill_freezes_account() {
        let database = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let store = PaperTradingStore::open(database).unwrap();
        store.create_account("alice", account(), 1).unwrap();

        assert!(
            store
                .record_open_orders(
                    "alice",
                    &[adaq_trading_crypto::Order {
                        id: Some("remote-incomplete".into()),
                        symbol: Some("BTC/USDT".into()),
                        side: Some("buy".into()),
                        amount: Some(Decimal::new(2, 0)),
                        price: Some(Decimal::new(100, 0)),
                        status: Some("open".into()),
                        ..Default::default()
                    }],
                    2,
                )
                .is_err()
        );

        let view = store.view("alice").unwrap();
        assert_eq!(view.reconciliation, ReconciliationState::Required);
        assert!(
            store
                .begin_order(
                    &PaperOrderRequest {
                        user_id: "alice".into(),
                        operation_id: "blocked-after-incomplete-order".into(),
                        instrument: "BTC-USDT".into(),
                        side: "buy".into(),
                        quantity: Decimal::ONE,
                        limit_price: Decimal::new(100, 0),
                    },
                    3,
                )
                .is_err()
        );
    }

    #[test]
    fn concurrent_bots_cannot_overreserve_one_shared_account() {
        let database = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let store = Arc::new(PaperTradingStore::open(database).unwrap());
        store.create_account("alice", account(), 1).unwrap();
        let policy = RiskPolicy {
            max_order_notional: Decimal::new(700_000, 0),
            reserve_cash: Decimal::ZERO,
            freeze_new_risk: false,
        };
        let handles = (0..2)
            .map(|index| {
                let store = Arc::clone(&store);
                let policy = policy.clone();
                std::thread::spawn(move || {
                    store.begin_order_with_policy(
                        &PaperOrderRequest {
                            user_id: "alice".into(),
                            operation_id: format!("bot-{index}"),
                            instrument: "BTC-USDT".into(),
                            side: "buy".into(),
                            quantity: Decimal::new(7_000, 0),
                            limit_price: Decimal::new(100, 0),
                        },
                        Some(&policy),
                        index + 2,
                    )
                })
            })
            .collect::<Vec<_>>();
        let outcomes = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(outcomes.iter().filter(|outcome| outcome.is_ok()).count(), 1);
        assert_eq!(
            store.view("alice").unwrap().reserved_cash,
            Decimal::new(700_000, 0)
        );
    }
}

impl PaperTradingStore {
    pub(crate) fn provider_balance(
        &self,
        user_id: &str,
        account_id: String,
        open_orders: &[adaq_trading_crypto::Order],
        balances: &adaq_trading_crypto::Balances,
        now_ms: i64,
    ) -> Result<PaperAccountView, String> {
        let snapshot = Self::snapshot_from_balance(user_id, account_id, balances, now_ms)?;
        if self.has_account(user_id)? {
            self.reconcile(user_id, snapshot, now_ms)?;
            self.record_open_orders(user_id, &open_orders, now_ms)
        } else {
            self.create_account(user_id, snapshot, now_ms)?;
            self.record_open_orders(user_id, &open_orders, now_ms)
        }
    }
}
