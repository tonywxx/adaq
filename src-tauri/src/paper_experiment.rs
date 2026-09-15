//! Host-owned, bounded three-instrument Paper experiment orchestration.
//!
//! The experiment coordinates the existing EMA Bot, Paper Ledger, OKX Demo
//! reconciliation, and Paper Feedback stores. It does not create a second
//! execution engine or ledger.

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{Arc, Mutex},
};

use adaq_paper_trading_core::{ExecutionOutcome, Fill, FillEvidence, OrderStatus, Side};
use adaq_trading_crypto::Exchange;
use rusqlite::{Connection, OptionalExtension, params};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Manager, State, WebviewWindow};
use uuid::Uuid;

use crate::{
    auth::AuthState,
    bot_operations::{BotRuntimeAttempt, BotView},
    connections::{ProfileStatus, Provider},
    local_research::LocalResearchState,
    paper_feedback::{EvidenceState, FeedbackLens},
    strategy_candidate::StrategyCandidateStore,
    strategy_qualification::{StrategyQualification, StrategyQualificationStore},
    user::validate_user,
};

pub(crate) const EXPERIMENT_INSTRUMENTS: [&str; 3] = ["BTC-USDT", "ETH-USDT", "SOL-USDT"];
const INITIAL_ALLOCATION_USDT: Decimal = Decimal::from_parts(3_269_476, 0, 0, false, 2);
const ENTRY_NOTIONAL_CAP_USDT: Decimal = Decimal::from_parts(3_236_781, 0, 0, false, 2);
const RESERVED_CASH_USDT: Decimal = Decimal::from_parts(32_695, 0, 0, false, 2);
const UNALLOCATED_REMAINDER_USDT: Decimal = Decimal::from_parts(951_395_709, 0, 0, false, 11);
const VALUATION_FRESHNESS_MS: i64 = 120_000;
const VALUATION_CADENCE_MS: i64 = 30_000;
const REQUIRED_VALUATIONS: u64 = 2;

fn default_valuation_cadence_ms() -> i64 {
    VALUATION_CADENCE_MS
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PaperExperimentState {
    Draft,
    Preparing,
    Armed,
    Running,
    Stopping,
    Completed,
    Incomplete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PaperExperimentInstrument {
    pub instrument: String,
    pub qualification_id: String,
    pub bot_id: Option<String>,
    #[serde(with = "rust_decimal::serde::str")]
    pub allocation_usdt: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub entry_notional_cap_usdt: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub reserved_cash_usdt: Decimal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PaperExperiment {
    pub experiment_id: String,
    pub user_id: String,
    pub profile_id: String,
    pub account_id: String,
    pub observation_start_ms: i64,
    pub observation_end_ms: i64,
    #[serde(with = "rust_decimal::serde::str")]
    pub allocation_total_usdt: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub unallocated_remainder_usdt: Decimal,
    #[serde(with = "rust_decimal::serde::str_option")]
    pub starting_account_cash: Option<Decimal>,
    pub started_at_ms: Option<i64>,
    pub instruments: Vec<PaperExperimentInstrument>,
    pub state: PaperExperimentState,
    pub report_id: Option<String>,
    pub feedback_snapshot_ids: Vec<String>,
    pub feedback_report_ids: Vec<String>,
    pub limitations: Vec<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PaperExperimentValuation {
    pub experiment_id: String,
    pub instrument: String,
    pub observed_at_ms: i64,
    #[serde(with = "rust_decimal::serde::str")]
    pub attributed_cash_usdt: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub position_quantity: Decimal,
    #[serde(with = "rust_decimal::serde::str_option")]
    pub price_usdt: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::str_option")]
    pub equity_usdt: Option<Decimal>,
    pub price_observed_at_ms: Option<i64>,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PaperExperimentInstrumentReport {
    pub instrument: String,
    pub bot_id: String,
    #[serde(with = "rust_decimal::serde::str")]
    pub starting_capital_usdt: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub ending_cash_usdt: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub ending_position_quantity: Decimal,
    #[serde(with = "rust_decimal::serde::str_option")]
    pub ending_position_value_usdt: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::str")]
    pub realized_pnl_usdt: Decimal,
    #[serde(with = "rust_decimal::serde::str_option")]
    pub unrealized_pnl_usdt: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::str")]
    pub fees_usdt: Decimal,
    #[serde(with = "rust_decimal::serde::str_option")]
    pub net_equity_return: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::str_option")]
    pub max_drawdown: Option<Decimal>,
    pub completed_trades: u64,
    pub exposure_time_ms: i64,
    pub interruptions: u64,
    pub valuation_count: u64,
    pub evidence_state: EvidenceState,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PaperExperimentReport {
    pub report_id: String,
    pub experiment_id: String,
    pub user_id: String,
    pub observation_start_ms: i64,
    pub observation_end_ms: i64,
    pub generated_at_ms: i64,
    #[serde(default = "default_valuation_cadence_ms")]
    pub valuation_cadence_ms: i64,
    pub common_end_valuation_at_ms: Option<i64>,
    pub ranking_eligible: bool,
    pub ranking_blocked_reason: Option<String>,
    #[serde(default)]
    pub ranked_instruments: Vec<String>,
    #[serde(default)]
    pub best_instrument: Option<String>,
    #[serde(with = "rust_decimal::serde::str")]
    pub allocation_total_usdt: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub unallocated_remainder_usdt: Decimal,
    pub evidence_state: EvidenceState,
    pub instruments: Vec<PaperExperimentInstrumentReport>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PaperExperimentView {
    pub experiment: PaperExperiment,
    pub account: Option<crate::paper_trading::PaperAccountView>,
    pub bots: Vec<BotView>,
    pub valuations: Vec<PaperExperimentValuation>,
    pub report: Option<PaperExperimentReport>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PaperExperimentBindingRequest {
    pub instrument: String,
    pub qualification_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PaperExperimentCreateRequest {
    pub profile_id: String,
    pub account_id: String,
    pub observation_start_ms: i64,
    pub observation_end_ms: i64,
    pub bindings: Vec<PaperExperimentBindingRequest>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PaperExperimentViewRequest {
    pub experiment_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PaperExperimentIdRequest {
    pub experiment_id: String,
}

#[derive(Clone)]
pub(crate) struct PaperExperimentStore {
    database: Arc<Mutex<Connection>>,
}

impl PaperExperimentStore {
    pub(crate) fn open(database: Arc<Mutex<Connection>>) -> Result<Self, String> {
        database
            .lock()
            .map_err(|error| error.to_string())?
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS paper_experiments (
                    experiment_id TEXT PRIMARY KEY,
                    user_id TEXT NOT NULL,
                    state TEXT NOT NULL,
                    payload_json TEXT NOT NULL,
                    updated_at_ms INTEGER NOT NULL
                );
                CREATE INDEX IF NOT EXISTS paper_experiments_user_updated
                    ON paper_experiments(user_id, updated_at_ms DESC);
                CREATE TABLE IF NOT EXISTS paper_experiment_valuations (
                    experiment_id TEXT NOT NULL,
                    user_id TEXT NOT NULL,
                    instrument TEXT NOT NULL,
                    observed_at_ms INTEGER NOT NULL,
                    payload_json TEXT NOT NULL,
                    PRIMARY KEY(experiment_id, instrument, observed_at_ms),
                    FOREIGN KEY(experiment_id) REFERENCES paper_experiments(experiment_id)
                );
                CREATE TABLE IF NOT EXISTS paper_experiment_reports (
                    report_id TEXT PRIMARY KEY,
                    experiment_id TEXT NOT NULL,
                    user_id TEXT NOT NULL,
                    payload_json TEXT NOT NULL,
                    created_at_ms INTEGER NOT NULL,
                    FOREIGN KEY(experiment_id) REFERENCES paper_experiments(experiment_id)
                );
                CREATE INDEX IF NOT EXISTS paper_experiment_reports_experiment
                    ON paper_experiment_reports(experiment_id, created_at_ms DESC);",
            )
            .map_err(|error| error.to_string())?;
        Ok(Self { database })
    }

    pub(crate) fn create(&self, experiment: &PaperExperiment) -> Result<(), String> {
        let payload = serde_json::to_string(experiment).map_err(|error| error.to_string())?;
        self.database
            .lock()
            .map_err(|error| error.to_string())?
            .execute(
                "INSERT INTO paper_experiments
                 (experiment_id, user_id, state, payload_json, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    experiment.experiment_id,
                    experiment.user_id,
                    state_name(experiment.state),
                    payload,
                    experiment.updated_at_ms
                ],
            )
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    pub(crate) fn save(&self, experiment: &PaperExperiment) -> Result<(), String> {
        let payload = serde_json::to_string(experiment).map_err(|error| error.to_string())?;
        let changed = self
            .database
            .lock()
            .map_err(|error| error.to_string())?
            .execute(
                "UPDATE paper_experiments SET state=?1, payload_json=?2, updated_at_ms=?3
                 WHERE experiment_id=?4 AND user_id=?5",
                params![
                    state_name(experiment.state),
                    payload,
                    experiment.updated_at_ms,
                    experiment.experiment_id,
                    experiment.user_id
                ],
            )
            .map_err(|error| error.to_string())?;
        if changed == 0 {
            return Err("Paper Experiment was not found for User".into());
        }
        Ok(())
    }

    pub(crate) fn get(
        &self,
        user_id: &str,
        experiment_id: &str,
    ) -> Result<PaperExperiment, String> {
        validate_user(user_id)?;
        let payload: String = self
            .database
            .lock()
            .map_err(|error| error.to_string())?
            .query_row(
                "SELECT payload_json FROM paper_experiments
                 WHERE experiment_id=?1 AND user_id=?2",
                params![experiment_id, user_id],
                |row| row.get(0),
            )
            .map_err(|_| "Paper Experiment was not found for User".to_owned())?;
        serde_json::from_str(&payload).map_err(|error| error.to_string())
    }

    pub(crate) fn latest(&self, user_id: &str) -> Result<Option<PaperExperiment>, String> {
        validate_user(user_id)?;
        let payload = self
            .database
            .lock()
            .map_err(|error| error.to_string())?
            .query_row(
                "SELECT payload_json FROM paper_experiments
                 WHERE user_id=?1 ORDER BY updated_at_ms DESC, experiment_id DESC LIMIT 1",
                [user_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        payload
            .map(|value| serde_json::from_str(&value).map_err(|error| error.to_string()))
            .transpose()
    }

    pub(crate) fn active_for_account(
        &self,
        user_id: &str,
        account_id: &str,
    ) -> Result<bool, String> {
        validate_user(user_id)?;
        let database = self.database.lock().map_err(|error| error.to_string())?;
        let mut statement = database
            .prepare(
                "SELECT payload_json FROM paper_experiments
                WHERE user_id=?1 AND state IN ('preparing', 'armed', 'running', 'stopping')",
            )
            .map_err(|error| error.to_string())?;
        for row in statement
            .query_map([user_id], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?
        {
            let payload = row.map_err(|error| error.to_string())?;
            let experiment: PaperExperiment =
                serde_json::from_str(&payload).map_err(|error| error.to_string())?;
            if experiment.account_id == account_id {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn for_bot(&self, user_id: &str, bot_id: &str) -> Result<Option<PaperExperiment>, String> {
        validate_user(user_id)?;
        let database = self.database.lock().map_err(|error| error.to_string())?;
        let mut statement = database
            .prepare(
                "SELECT payload_json FROM paper_experiments
                 WHERE user_id=?1 AND state IN ('preparing', 'armed', 'running', 'stopping')",
            )
            .map_err(|error| error.to_string())?;
        for row in statement
            .query_map([user_id], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?
        {
            let payload = row.map_err(|error| error.to_string())?;
            let experiment: PaperExperiment =
                serde_json::from_str(&payload).map_err(|error| error.to_string())?;
            if experiment
                .instruments
                .iter()
                .any(|binding| binding.bot_id.as_deref() == Some(bot_id))
            {
                return Ok(Some(experiment));
            }
        }
        Ok(None)
    }

    pub(crate) fn decision_blocked_for_bot(
        &self,
        user_id: &str,
        bot_id: &str,
        now_ms: i64,
    ) -> Result<bool, String> {
        let Some(experiment) = self.for_bot(user_id, bot_id)? else {
            return Ok(false);
        };
        let accepts_observation = experiment.started_at_ms.is_some()
            && now_ms >= experiment.observation_start_ms
            && now_ms < experiment.observation_end_ms
            && matches!(
                experiment.state,
                PaperExperimentState::Preparing
                    | PaperExperimentState::Armed
                    | PaperExperimentState::Running
            );
        Ok(!accepts_observation)
    }

    pub(crate) fn arm_if_all_bots_warmed(
        &self,
        user_id: &str,
        bot_id: &str,
        bots: &[BotView],
        now_ms: i64,
    ) -> Result<bool, String> {
        let Some(mut experiment) = self.for_bot(user_id, bot_id)? else {
            return Ok(false);
        };
        if experiment.state != PaperExperimentState::Preparing
            || now_ms < experiment.observation_start_ms
            || now_ms >= experiment.observation_end_ms
        {
            return Ok(false);
        }
        let warmed_bot_ids = bots
            .iter()
            .filter(|bot| bot_is_warmed(bot))
            .map(|bot| bot.bot_id.clone())
            .collect::<BTreeSet<_>>();
        if !all_experiment_bots_warmed(&experiment, &warmed_bot_ids) {
            return Ok(false);
        }
        experiment.state = PaperExperimentState::Armed;
        experiment.updated_at_ms = now_ms;
        self.save(&experiment)?;
        experiment.state = PaperExperimentState::Running;
        experiment.updated_at_ms = adaq_bot_runtime::unix_now_ms();
        self.save(&experiment)?;
        Ok(true)
    }

    pub(crate) fn append_valuations(
        &self,
        user_id: &str,
        valuations: &[PaperExperimentValuation],
    ) -> Result<(), String> {
        validate_user(user_id)?;
        let database = self.database.lock().map_err(|error| error.to_string())?;
        for valuation in valuations {
            if valuation.experiment_id.trim().is_empty()
                || valuation.instrument.trim().is_empty()
                || valuation.observed_at_ms <= 0
            {
                return Err("invalid Paper Experiment valuation".into());
            }
            database
                .execute(
                    "INSERT OR IGNORE INTO paper_experiment_valuations
                     (experiment_id, user_id, instrument, observed_at_ms, payload_json)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        valuation.experiment_id,
                        user_id,
                        valuation.instrument,
                        valuation.observed_at_ms,
                        serde_json::to_string(valuation).map_err(|error| error.to_string())?
                    ],
                )
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    pub(crate) fn valuations(
        &self,
        user_id: &str,
        experiment_id: &str,
    ) -> Result<Vec<PaperExperimentValuation>, String> {
        validate_user(user_id)?;
        let database = self.database.lock().map_err(|error| error.to_string())?;
        let mut statement = database
            .prepare(
                "SELECT payload_json FROM paper_experiment_valuations
                 WHERE experiment_id=?1 AND user_id=?2
                 ORDER BY observed_at_ms ASC, instrument ASC",
            )
            .map_err(|error| error.to_string())?;
        statement
            .query_map(params![experiment_id, user_id], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|error| error.to_string())?
            .map(|row| {
                let payload = row.map_err(|error| error.to_string())?;
                serde_json::from_str(&payload).map_err(|error| error.to_string())
            })
            .collect()
    }

    pub(crate) fn save_report(
        &self,
        experiment: &mut PaperExperiment,
        report: &PaperExperimentReport,
    ) -> Result<(), String> {
        let payload = serde_json::to_string(report).map_err(|error| error.to_string())?;
        let database = self.database.lock().map_err(|error| error.to_string())?;
        database
            .execute(
                "INSERT INTO paper_experiment_reports
                 (report_id, experiment_id, user_id, payload_json, created_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    report.report_id,
                    report.experiment_id,
                    report.user_id,
                    payload,
                    report.generated_at_ms
                ],
            )
            .map_err(|error| error.to_string())?;
        experiment.report_id = Some(report.report_id.clone());
        experiment.updated_at_ms = report.generated_at_ms;
        let experiment_payload =
            serde_json::to_string(experiment).map_err(|error| error.to_string())?;
        database
            .execute(
                "UPDATE paper_experiments SET payload_json=?1, updated_at_ms=?2, state=?3
                 WHERE experiment_id=?4 AND user_id=?5",
                params![
                    experiment_payload,
                    experiment.updated_at_ms,
                    state_name(experiment.state),
                    experiment.experiment_id,
                    experiment.user_id
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub(crate) fn report_for_user(
        &self,
        user_id: &str,
        report_id: &str,
    ) -> Result<PaperExperimentReport, String> {
        validate_user(user_id)?;
        let payload: String = self
            .database
            .lock()
            .map_err(|error| error.to_string())?
            .query_row(
                "SELECT payload_json FROM paper_experiment_reports
                 WHERE report_id=?1 AND user_id=?2",
                params![report_id, user_id],
                |row| row.get(0),
            )
            .map_err(|_| "Paper Experiment Report was not found for User".to_owned())?;
        serde_json::from_str(&payload).map_err(|error| error.to_string())
    }

    pub(crate) fn risk_blocked_for_bot(
        &self,
        user_id: &str,
        bot_id: &str,
        now_ms: i64,
    ) -> Result<bool, String> {
        let Some(experiment) = self.for_bot(user_id, bot_id)? else {
            return Ok(false);
        };
        let risk_window_open = experiment.state == PaperExperimentState::Running
            && experiment.started_at_ms.is_some()
            && now_ms >= experiment.observation_start_ms
            && now_ms < experiment.observation_end_ms;
        Ok(!risk_window_open)
    }

    pub(crate) fn common_warmup_blocked_for_bot(
        &self,
        user_id: &str,
        bot_id: &str,
        bots: &[BotView],
        now_ms: i64,
    ) -> Result<bool, String> {
        let Some(experiment) = self.for_bot(user_id, bot_id)? else {
            return Ok(false);
        };
        if experiment.state != PaperExperimentState::Running
            || experiment.started_at_ms.is_none()
            || now_ms < experiment.observation_start_ms
            || now_ms >= experiment.observation_end_ms
        {
            return Ok(false);
        }
        let warmed_bot_ids = bots
            .iter()
            .filter(|bot| bot_is_warmed(bot))
            .map(|bot| bot.bot_id.clone())
            .collect::<BTreeSet<_>>();
        Ok(!all_experiment_bots_warmed(&experiment, &warmed_bot_ids))
    }

    pub(crate) fn pending_orders_are_experiment_owned(
        &self,
        user_id: &str,
        bot_id: &str,
        account: &crate::paper_trading::PaperAccountView,
    ) -> Result<bool, String> {
        let Some(experiment) = self.for_bot(user_id, bot_id)? else {
            return Ok(false);
        };
        if !matches!(
            experiment.state,
            PaperExperimentState::Preparing
                | PaperExperimentState::Armed
                | PaperExperimentState::Running
        ) || account.reconciliation != adaq_paper_trading_core::ReconciliationState::Reconciled
            || account.restart_required
            || account
                .provider_evidence
                .iter()
                .any(|outcome| matches!(outcome, ExecutionOutcome::Uncertain(_)))
        {
            return Ok(false);
        }
        let prefixes = experiment
            .instruments
            .iter()
            .filter_map(|binding| binding.bot_id.as_deref())
            .map(|bot_id| format!("bot-{bot_id}-"))
            .collect::<BTreeSet<_>>();
        Ok(account
            .orders
            .iter()
            .filter(|order| {
                matches!(
                    order.status,
                    OrderStatus::Accepted | OrderStatus::PartiallyFilled
                )
            })
            .all(|order| {
                account.provider_evidence.iter().any(|outcome| {
                    let evidence = match outcome {
                        ExecutionOutcome::Accepted(evidence)
                        | ExecutionOutcome::Rejected(evidence)
                        | ExecutionOutcome::Uncertain(evidence) => evidence,
                    };
                    evidence.local_order_id.as_deref() == Some(order.order_id.as_str())
                        && prefixes
                            .iter()
                            .any(|prefix| evidence.operation_id.starts_with(prefix))
                })
            }))
    }
}

fn state_name(state: PaperExperimentState) -> &'static str {
    match state {
        PaperExperimentState::Draft => "draft",
        PaperExperimentState::Preparing => "preparing",
        PaperExperimentState::Armed => "armed",
        PaperExperimentState::Running => "running",
        PaperExperimentState::Stopping => "stopping",
        PaperExperimentState::Completed => "completed",
        PaperExperimentState::Incomplete => "incomplete",
    }
}

fn report_state_is_terminal(state: PaperExperimentState) -> bool {
    matches!(
        state,
        PaperExperimentState::Completed | PaperExperimentState::Incomplete
    )
}

fn persist_incomplete(
    local: &LocalResearchState,
    experiment: &mut PaperExperiment,
    limitation: impl Into<String>,
) -> Result<(), String> {
    experiment.state = PaperExperimentState::Incomplete;
    experiment.limitations.push(limitation.into());
    experiment.updated_at_ms = adaq_bot_runtime::unix_now_ms();
    local.paper_experiments.save(experiment)
}

fn bot_is_warmed(bot: &BotView) -> bool {
    bot.state == adaq_bot_runtime::LifecycleState::Running
        && bot
            .current_attempt_id
            .as_deref()
            .and_then(|attempt_id| {
                bot.attempts
                    .iter()
                    .find(|attempt| attempt.attempt_id == attempt_id)
            })
            .is_some_and(attempt_is_warmed)
}

fn all_experiment_bots_warmed(
    experiment: &PaperExperiment,
    warmed_bot_ids: &BTreeSet<String>,
) -> bool {
    let bot_ids = experiment
        .instruments
        .iter()
        .filter_map(|binding| binding.bot_id.as_ref())
        .collect::<BTreeSet<_>>();
    bot_ids.len() == EXPERIMENT_INSTRUMENTS.len()
        && bot_ids
            .iter()
            .all(|bot_id| warmed_bot_ids.contains(*bot_id))
}

fn attempt_is_warmed(attempt: &BotRuntimeAttempt) -> bool {
    let latest_reset = attempt.evidence.iter().rposition(|evidence| {
        evidence.code == "warmup-started" || evidence.code == "worker-restarted-for-resume"
    });
    attempt
        .evidence
        .iter()
        .enumerate()
        .any(|(index, evidence)| {
            evidence.code == "warmup-complete"
                && latest_reset.is_none_or(|reset_index| index > reset_index)
        })
}

fn total_allocation() -> Decimal {
    INITIAL_ALLOCATION_USDT * Decimal::from(3u32)
}

fn normalize_instrument(value: &str) -> String {
    value
        .trim()
        .strip_prefix("okx:")
        .or_else(|| value.trim().strip_prefix("OKX:"))
        .unwrap_or(value.trim())
        .to_ascii_uppercase()
}

fn normalized_bindings(
    bindings: Vec<PaperExperimentBindingRequest>,
) -> Result<Vec<PaperExperimentInstrument>, String> {
    if bindings.len() != EXPERIMENT_INSTRUMENTS.len() {
        return Err("The experiment requires exactly BTC-USDT, ETH-USDT, and SOL-USDT.".into());
    }
    let mut by_instrument = BTreeMap::new();
    for binding in bindings {
        let instrument = normalize_instrument(&binding.instrument);
        if !EXPERIMENT_INSTRUMENTS.contains(&instrument.as_str())
            || binding.qualification_id.trim().is_empty()
            || binding.qualification_id.len() > 128
        {
            return Err(
                "Each experiment instrument must have one bounded EMA Qualification.".into(),
            );
        }
        if by_instrument
            .insert(instrument, binding.qualification_id)
            .is_some()
        {
            return Err("Experiment instruments must be unique.".into());
        }
    }
    EXPERIMENT_INSTRUMENTS
        .into_iter()
        .map(|instrument| {
            let qualification_id = by_instrument
                .remove(instrument)
                .ok_or_else(|| format!("Missing Qualification for {instrument}."))?;
            Ok(PaperExperimentInstrument {
                instrument: instrument.into(),
                qualification_id,
                bot_id: None,
                allocation_usdt: INITIAL_ALLOCATION_USDT,
                entry_notional_cap_usdt: ENTRY_NOTIONAL_CAP_USDT,
                reserved_cash_usdt: RESERVED_CASH_USDT,
            })
        })
        .collect()
}

fn validate_create_request(request: &PaperExperimentCreateRequest) -> Result<(), String> {
    if request.profile_id.trim().is_empty()
        || request.profile_id.len() > 128
        || request.account_id.trim().is_empty()
        || request.account_id.len() > 128
        || request.observation_start_ms <= 0
        || request.observation_end_ms <= request.observation_start_ms
        || request
            .observation_end_ms
            .saturating_sub(request.observation_start_ms)
            < 60_000
    {
        return Err(
            "Experiment profile, account, and bounded UTC observation window are required.".into(),
        );
    }
    Ok(())
}

fn build_view(
    app: &AppHandle,
    state: &LocalResearchState,
    experiment: PaperExperiment,
) -> Result<PaperExperimentView, String> {
    let bots_store = app.state::<Arc<crate::bot_operations::BotStore>>();
    let bots = experiment
        .instruments
        .iter()
        .filter_map(|binding| binding.bot_id.as_deref())
        .map(|bot_id| bots_store.get(&experiment.user_id, bot_id))
        .collect::<Result<Vec<_>, _>>()?;
    let report = experiment
        .report_id
        .as_deref()
        .map(|report_id| {
            state
                .paper_experiments
                .report_for_user(&experiment.user_id, report_id)
        })
        .transpose()?;
    Ok(PaperExperimentView {
        account: state.paper_trading.view_optional(&experiment.user_id)?,
        valuations: state
            .paper_experiments
            .valuations(&experiment.user_id, &experiment.experiment_id)?,
        experiment,
        bots,
        report,
    })
}

fn validate_profile(
    local: &LocalResearchState,
    user_id: &str,
    profile_id: &str,
    account_id: &str,
) -> Result<(), String> {
    let profile = local
        .connections
        .list(user_id)?
        .into_iter()
        .find(|profile| profile.profile_id == profile_id)
        .ok_or_else(|| "The selected connection profile was not found.".to_owned())?;
    if profile.provider != Provider::OkxDemo
        || profile.status != ProfileStatus::Usable
        || profile.account_id.as_deref() != Some(account_id)
    {
        return Err(
            "Select one usable, verified OKX Demo profile with the exact account identity.".into(),
        );
    }
    Ok(())
}

fn qualification_identity(qualification: &StrategyQualification) -> serde_json::Value {
    // EMA qualification hashes include instrument-specific snapshot/replay evidence;
    // compare the frozen Strategy contract instead of that per-instrument evidence hash.
    json!({
        "candidateId": qualification.candidate_id,
        "candidateRevision": qualification.candidate_revision,
        "packageArchiveSha256": qualification.package.package_archive_sha256,
        "packageWasmSha256": qualification.package.package_wasm_sha256,
        "parameters": qualification.package.parameters,
        "riskPolicy": qualification.context.risk_policy,
        "executionProfile": qualification.context.execution_profile,
        "universeId": qualification.context.universe_id,
        "universeSnapshotId": qualification.context.universe_snapshot_id,
    })
}

fn validate_qualifications(
    local: &LocalResearchState,
    qualifications: &StrategyQualificationStore,
    user_id: &str,
    experiment: &PaperExperiment,
) -> Result<(), String> {
    let mut frozen_identity = None;
    for binding in &experiment.instruments {
        let qualification =
            qualifications.qualification_for_user(user_id, &binding.qualification_id)?;
        if qualification.candidate_id
            != crate::strategy_qualification::EMA_DOUBLE_CROSS_CANDIDATE_ID
            || !qualification.gate12_eligible
            || qualification.gate12_continuation_required
        {
            return Err(format!(
                "{} is not an eligible EMA Double-Cross Qualification.",
                binding.instrument
            ));
        }
        let (snapshot, _) = local.snapshot_for_user(user_id, &qualification.context.snapshot_id)?;
        if normalize_instrument(&snapshot.code) != binding.instrument {
            return Err(format!(
                "Qualification {} is bound to {}, not {}.",
                binding.qualification_id, snapshot.code, binding.instrument
            ));
        }
        let identity = qualification_identity(&qualification);
        if let Some(expected) = &frozen_identity {
            if expected != &identity {
                return Err(
                    "The three EMA Qualifications do not share one frozen Strategy identity."
                        .into(),
                );
            }
        } else {
            frozen_identity = Some(identity);
        }
    }
    Ok(())
}

fn reconcile_demo(
    local: &LocalResearchState,
    user_id: &str,
    account_id: &str,
) -> Result<crate::paper_trading::PaperAccountView, String> {
    let now_ms = adaq_bot_runtime::unix_now_ms();
    local
        .connections
        .with_okx_demo_reconciliation(user_id, now_ms, |open_orders, balances| {
            local.paper_trading.provider_balance(
                user_id,
                account_id.to_owned(),
                open_orders,
                balances,
                now_ms,
            )
        })?
}

#[derive(Clone, Copy)]
struct PriceEvidence {
    price: Option<Decimal>,
    observed_at_ms: Option<i64>,
}

fn demo_price(
    local: &LocalResearchState,
    user_id: &str,
    instrument: &str,
) -> Result<PriceEvidence, String> {
    let ticker = local
        .connections
        .with_okx_demo_client(user_id, |client| {
            tauri::async_runtime::block_on(
                client.fetch_ticker(instrument, adaq_trading_crypto::Params::new()),
            )
        })?
        .map_err(|_| "OKX Demo valuation ticker was unavailable.".to_owned())?;
    Ok(PriceEvidence {
        price: ticker.last.or(ticker.bid).or(ticker.ask).or(ticker.close),
        observed_at_ms: ticker.timestamp,
    })
}

fn demo_prices(
    local: &LocalResearchState,
    user_id: &str,
    experiment: &PaperExperiment,
) -> Result<BTreeMap<String, PriceEvidence>, String> {
    experiment
        .instruments
        .iter()
        .map(|binding| {
            demo_price(local, user_id, &binding.instrument)
                .map(|price| (binding.instrument.clone(), price))
        })
        .collect()
}

fn experiment_operation_ids(experiment: &PaperExperiment) -> BTreeSet<String> {
    experiment
        .instruments
        .iter()
        .filter_map(|binding| binding.bot_id.as_deref())
        .map(|bot_id| format!("bot-{bot_id}-"))
        .collect()
}

fn sync_known_orders(
    local: &LocalResearchState,
    user_id: &str,
    experiment: &PaperExperiment,
) -> Vec<String> {
    let Ok(account) = local.paper_trading.view_optional(user_id) else {
        return vec!["Paper Account evidence is unavailable.".into()];
    };
    let Some(account) = account else {
        return vec!["Paper Account evidence is unavailable.".into()];
    };
    let prefixes = experiment_operation_ids(experiment);
    let mut seen = BTreeSet::new();
    let mut limitations = Vec::new();
    for outcome in &account.provider_evidence {
        let evidence = match outcome {
            ExecutionOutcome::Accepted(evidence)
            | ExecutionOutcome::Rejected(evidence)
            | ExecutionOutcome::Uncertain(evidence) => evidence,
        };
        if !prefixes
            .iter()
            .any(|prefix| evidence.operation_id.starts_with(prefix))
            || !seen.insert(evidence.operation_id.clone())
        {
            continue;
        }
        let Some(provider_order_id) = evidence.provider_order_id.as_deref() else {
            continue;
        };
        let Some(local_order_id) = evidence.local_order_id.as_deref() else {
            continue;
        };
        let Some(order) = account
            .orders
            .iter()
            .find(|order| order.order_id == local_order_id)
        else {
            continue;
        };
        match local.connections.fetch_okx_demo_order(
            user_id,
            &order.instrument,
            provider_order_id,
            adaq_bot_runtime::unix_now_ms(),
        ) {
            Ok(remote) => {
                let fills = local.connections.fetch_okx_demo_order_fills(
                    user_id,
                    &order.instrument,
                    provider_order_id,
                    adaq_bot_runtime::unix_now_ms(),
                );
                let sync = match fills {
                    Ok(fills) => {
                        let trades = if fills.is_empty() {
                            remote.trades.as_deref().unwrap_or(&[])
                        } else {
                            fills.as_slice()
                        };
                        local.paper_trading.sync_provider_order_with_trades(
                            user_id,
                            &evidence.operation_id,
                            &remote,
                            trades,
                            adaq_bot_runtime::unix_now_ms(),
                        )
                    }
                    Err(error) => {
                        limitations.push(format!(
                            "Provider fills for order {} were not refreshed: {}",
                            provider_order_id, error
                        ));
                        if let Err(mark_error) = local.paper_trading.mark_provider_order_uncertain(
                            user_id,
                            provider_order_id,
                            adaq_bot_runtime::unix_now_ms(),
                        ) {
                            limitations.push(format!(
                                "Provider fill uncertainty was not retained for order {}: {}",
                                provider_order_id, mark_error
                            ));
                        }
                        continue;
                    }
                };
                if let Err(error) = sync {
                    limitations.push(error);
                }
            }
            Err(error) => {
                limitations.push(format!(
                    "Provider order {} was not refreshed: {}",
                    provider_order_id, error
                ));
                if let Err(mark_error) = local.paper_trading.mark_provider_order_uncertain(
                    user_id,
                    provider_order_id,
                    adaq_bot_runtime::unix_now_ms(),
                ) {
                    limitations.push(format!(
                        "Provider order uncertainty was not retained for {}: {}",
                        provider_order_id, mark_error
                    ));
                }
            }
        }
    }
    limitations
}

fn cash_delta(side: Side, quantity: Decimal, price: Decimal, fee_quote: Decimal) -> Decimal {
    match side {
        Side::Buy => -(quantity * price + fee_quote),
        Side::Sell => quantity * price - fee_quote,
    }
}

fn reported_fee_quote(fill: &Fill) -> Decimal {
    fill.fee_quote.unwrap_or_default()
}

fn attributed_cash(
    account: &crate::paper_trading::PaperAccountView,
    instrument: &str,
    starting_capital: Decimal,
    from_ms: i64,
    through_ms: i64,
    owned_order_ids: Option<&BTreeSet<String>>,
) -> Decimal {
    account.fills.iter().fold(starting_capital, |cash, fill| {
        if fill.occurred_at_ms < from_ms || fill.occurred_at_ms > through_ms {
            return cash;
        }
        let Some(order) = account
            .orders
            .iter()
            .find(|order| order.order_id == fill.order_id)
        else {
            return cash;
        };
        if order.instrument != instrument {
            return cash;
        }
        if !owned_order_ids.is_some_and(|orders| orders.contains(&fill.order_id)) {
            return cash;
        }
        cash + cash_delta(
            order.side,
            fill.quantity,
            fill.price,
            reported_fee_quote(fill),
        )
    })
}

fn attributed_position_quantity(
    account: &crate::paper_trading::PaperAccountView,
    instrument: &str,
    from_ms: i64,
    through_ms: i64,
    owned_order_ids: Option<&BTreeSet<String>>,
) -> Decimal {
    let Some(owned_order_ids) = owned_order_ids else {
        return Decimal::ZERO;
    };
    account.fills.iter().fold(Decimal::ZERO, |quantity, fill| {
        if fill.occurred_at_ms < from_ms
            || fill.occurred_at_ms > through_ms
            || !owned_order_ids.contains(&fill.order_id)
        {
            return quantity;
        }
        let Some(order) = account
            .orders
            .iter()
            .find(|order| order.order_id == fill.order_id && order.instrument == instrument)
        else {
            return quantity;
        };
        let base_fee = fill.fee_in_base(instrument);
        match order.side {
            Side::Buy => quantity + fill.quantity - base_fee,
            Side::Sell => quantity - fill.quantity - base_fee,
        }
    })
}

fn consume_lots(
    lots: &mut VecDeque<(Decimal, Decimal)>,
    mut quantity: Decimal,
    mut on_consumed: impl FnMut(Decimal, Decimal),
) -> bool {
    if lots
        .iter()
        .map(|(lot_quantity, _)| *lot_quantity)
        .sum::<Decimal>()
        < quantity
    {
        return false;
    }
    while quantity > Decimal::ZERO {
        let Some((lot_quantity, cost)) = lots.pop_front() else {
            return false;
        };
        let used = lot_quantity.min(quantity);
        on_consumed(used, cost);
        quantity -= used;
        if lot_quantity > used {
            lots.push_front((lot_quantity - used, cost));
        }
    }
    true
}

fn experiment_valuations(
    experiment: &PaperExperiment,
    account: &crate::paper_trading::PaperAccountView,
    prices: &BTreeMap<String, PriceEvidence>,
    observed_at_ms: i64,
) -> Vec<PaperExperimentValuation> {
    experiment
        .instruments
        .iter()
        .map(|binding| {
            let owned_orders = binding
                .bot_id
                .as_deref()
                .map(|bot_id| owned_order_ids(account, bot_id));
            let from_ms = experiment
                .started_at_ms
                .unwrap_or(experiment.observation_start_ms);
            let price = prices
                .get(&binding.instrument)
                .copied()
                .unwrap_or(PriceEvidence {
                    price: None,
                    observed_at_ms: None,
                });
            let position_quantity = attributed_position_quantity(
                account,
                &binding.instrument,
                from_ms,
                observed_at_ms,
                owned_orders.as_ref(),
            );
            let attributed_cash = attributed_cash(
                account,
                &binding.instrument,
                binding.allocation_usdt,
                from_ms,
                observed_at_ms,
                owned_orders.as_ref(),
            );
            let equity = price
                .price
                .map(|value| attributed_cash + position_quantity * value);
            PaperExperimentValuation {
                experiment_id: experiment.experiment_id.clone(),
                instrument: binding.instrument.clone(),
                observed_at_ms,
                attributed_cash_usdt: attributed_cash,
                position_quantity,
                price_usdt: price.price,
                equity_usdt: equity,
                price_observed_at_ms: price.observed_at_ms,
                source: "okx-demo-ticker".into(),
            }
        })
        .collect()
}

fn owned_order_ids(
    account: &crate::paper_trading::PaperAccountView,
    bot_id: &str,
) -> BTreeSet<String> {
    let prefix = format!("bot-{bot_id}-");
    account
        .provider_evidence
        .iter()
        .filter_map(|outcome| match outcome {
            ExecutionOutcome::Accepted(evidence)
            | ExecutionOutcome::Rejected(evidence)
            | ExecutionOutcome::Uncertain(evidence) => evidence
                .operation_id
                .starts_with(&prefix)
                .then(|| evidence.local_order_id.clone())
                .flatten(),
        })
        .collect()
}

fn instrument_report(
    experiment: &PaperExperiment,
    binding: &PaperExperimentInstrument,
    account: &crate::paper_trading::PaperAccountView,
    bot: Option<&BotView>,
    valuations: &[PaperExperimentValuation],
    through_ms: i64,
) -> PaperExperimentInstrumentReport {
    let bot_id = binding.bot_id.clone().unwrap_or_default();
    let from_ms = experiment
        .started_at_ms
        .unwrap_or(experiment.observation_start_ms);
    let owned_orders = owned_order_ids(account, &bot_id);
    let mut fills = account
        .fills
        .iter()
        .filter(|fill| fill.occurred_at_ms >= from_ms && fill.occurred_at_ms <= through_ms)
        .filter_map(|fill| {
            let order = account
                .orders
                .iter()
                .find(|order| order.order_id == fill.order_id)?;
            (order.instrument == binding.instrument).then_some((order, fill))
        })
        .collect::<Vec<_>>();
    fills.sort_by_key(|(_, fill)| (fill.occurred_at_ms, fill.fill_id.clone()));
    let provider_fill_count = fills
        .iter()
        .filter(|(order, fill)| {
            owned_orders.contains(&fill.order_id)
                && order.instrument == binding.instrument
                && fill.evidence == FillEvidence::TradeObserved
        })
        .count();
    let mut external_activity = fills
        .iter()
        .any(|(_, fill)| !owned_orders.contains(&fill.order_id));

    let mut lots: VecDeque<(Decimal, Decimal)> = VecDeque::new();
    let mut realized = Decimal::ZERO;
    let mut fees = Decimal::ZERO;
    let mut fee_complete = true;
    for (order, fill) in fills
        .iter()
        .filter(|(_, fill)| owned_orders.contains(&fill.order_id))
    {
        fees += reported_fee_quote(fill);
        fee_complete &= fill.fee_quote.is_some();
        let base_fee = fill.fee_in_base(&binding.instrument);
        match order.side {
            Side::Buy => {
                let net_quantity = fill.quantity - base_fee;
                if net_quantity <= Decimal::ZERO {
                    external_activity = true;
                    continue;
                }
                let total_cost = fill.quantity * fill.price + reported_fee_quote(fill);
                lots.push_back((net_quantity, total_cost / net_quantity));
            }
            Side::Sell => {
                if !consume_lots(&mut lots, fill.quantity, |used, cost| {
                    realized += used * (fill.price - cost);
                }) {
                    external_activity = true;
                    continue;
                }
                if !consume_lots(&mut lots, base_fee, |used, cost| {
                    realized -= used * cost;
                }) {
                    external_activity = true;
                    continue;
                }
                realized -= reported_fee_quote(fill);
            }
        }
    }

    let latest = valuations
        .iter()
        .filter(|valuation| {
            valuation.instrument == binding.instrument
                && valuation.observed_at_ms >= from_ms
                && valuation.observed_at_ms <= through_ms
        })
        .max_by_key(|valuation| valuation.observed_at_ms);
    let ending_position_quantity = account
        .account
        .positions
        .get(&binding.instrument)
        .map(|position| position.quantity)
        .unwrap_or_default();
    let attributed_position_quantity = lots.iter().map(|(quantity, _)| *quantity).sum::<Decimal>();
    if ending_position_quantity != attributed_position_quantity {
        external_activity = true;
    }
    let ending_position_quantity = attributed_position_quantity;
    let ending_cash_usdt = attributed_cash(
        account,
        &binding.instrument,
        binding.allocation_usdt,
        from_ms,
        through_ms,
        Some(&owned_orders),
    );
    let ending_position_value_usdt = latest
        .and_then(|valuation| valuation.price_usdt)
        .map(|price| ending_position_quantity * price);
    let unrealized_pnl_usdt = latest.and_then(|valuation| {
        valuation.price_usdt.map(|price| {
            lots.iter().fold(Decimal::ZERO, |pnl, (quantity, cost)| {
                pnl + quantity * (price - *cost)
            })
        })
    });
    let net_equity_return = latest
        .and_then(|valuation| valuation.equity_usdt)
        .map(|equity| equity / binding.allocation_usdt - Decimal::ONE);

    let points = valuations
        .iter()
        .filter(|valuation| {
            valuation.instrument == binding.instrument
                && valuation.observed_at_ms >= from_ms
                && valuation.observed_at_ms <= through_ms
                && valuation.equity_usdt.is_some()
        })
        .collect::<Vec<_>>();
    let mut peak = None;
    let mut max_drawdown = None;
    for point in &points {
        let equity = point.equity_usdt.unwrap_or_default();
        peak = Some(peak.unwrap_or(equity).max(equity));
        if let Some(high) = peak.filter(|high| *high > Decimal::ZERO) {
            let drawdown = (high - equity) / high;
            max_drawdown = Some(max_drawdown.unwrap_or(drawdown).max(drawdown));
        }
    }
    let exposure_time_ms = points
        .windows(2)
        .filter(|window| window[0].position_quantity > Decimal::ZERO)
        .map(|window| {
            window[1]
                .observed_at_ms
                .saturating_sub(window[0].observed_at_ms)
        })
        .sum::<i64>();
    let exposure_time_ms = exposure_time_ms
        + points
            .last()
            .filter(|point| point.position_quantity > Decimal::ZERO)
            .map(|point| through_ms.saturating_sub(point.observed_at_ms))
            .unwrap_or_default();
    let completed_trades = account
        .orders
        .iter()
        .filter(|order| {
            order.instrument == binding.instrument
                && order.status == OrderStatus::Filled
                && order.submitted_at_ms >= from_ms
                && order.submitted_at_ms <= through_ms
                && owned_orders.contains(&order.order_id)
                && account.fills.iter().any(|fill| {
                    fill.order_id == order.order_id
                        && fill.evidence == FillEvidence::TradeObserved
                        && fill.occurred_at_ms >= from_ms
                        && fill.occurred_at_ms <= through_ms
                })
        })
        .count() as u64;
    let interruptions = bot
        .into_iter()
        .flat_map(|bot| bot.attempts.iter())
        .flat_map(|attempt| attempt.events.iter())
        .filter(|event| {
            matches!(
                format!("{:?}", event.to).as_str(),
                "Reconciling" | "Pausing" | "Paused" | "Faulted" | "Stopping"
            )
        })
        .count() as u64;
    let valuation_count = points.len() as u64;
    let mut limitations = Vec::new();
    if latest.and_then(|valuation| valuation.price_usdt).is_none() {
        limitations.push("A common end valuation price is missing.".into());
    }
    if !fee_complete {
        limitations.push("At least one provider fee lacks exact USDT valuation evidence.".into());
    }
    if provider_fill_count == 0 {
        limitations.push(
            "No owned provider Fill was retained; execution acceptance is incomplete.".into(),
        );
    }
    if external_activity {
        limitations.push("Unattributed activity was observed in this instrument scope.".into());
    }
    if valuation_count < REQUIRED_VALUATIONS {
        limitations.push("The declared window has fewer than two retained valuations.".into());
    }
    let evidence_state = if limitations.is_empty() {
        EvidenceState::Ready
    } else if valuation_count == 0 {
        EvidenceState::Missing
    } else {
        EvidenceState::Unknown
    };
    PaperExperimentInstrumentReport {
        instrument: binding.instrument.clone(),
        bot_id,
        starting_capital_usdt: binding.allocation_usdt,
        ending_cash_usdt,
        ending_position_quantity,
        ending_position_value_usdt,
        realized_pnl_usdt: realized,
        unrealized_pnl_usdt,
        fees_usdt: fees,
        net_equity_return,
        max_drawdown,
        completed_trades,
        exposure_time_ms,
        interruptions,
        valuation_count,
        evidence_state,
        limitations,
    }
}

fn end_valuation_is_fresh(valuation: &PaperExperimentValuation, end_ms: i64) -> bool {
    valuation.price_usdt.is_some()
        && valuation.observed_at_ms <= end_ms
        && end_ms.saturating_sub(valuation.observed_at_ms) <= VALUATION_FRESHNESS_MS
        && valuation
            .price_observed_at_ms
            .is_some_and(|observed_at_ms| {
                observed_at_ms <= end_ms
                    && end_ms.saturating_sub(observed_at_ms) <= VALUATION_FRESHNESS_MS
            })
}

fn rank_instruments(
    instruments: &[PaperExperimentInstrumentReport],
    eligible: bool,
) -> Vec<String> {
    if !eligible {
        return Vec::new();
    }
    let mut ranked = instruments
        .iter()
        .filter_map(|report| {
            report
                .net_equity_return
                .map(|return_rate| (return_rate, report.instrument.as_str()))
        })
        .collect::<Vec<_>>();
    ranked.sort_by(
        |(left_return, left_instrument), (right_return, right_instrument)| {
            right_return
                .cmp(left_return)
                .then_with(|| left_instrument.cmp(right_instrument))
        },
    );
    ranked
        .into_iter()
        .map(|(_, instrument)| instrument.to_owned())
        .collect()
}

fn build_report(
    experiment: &PaperExperiment,
    account: &crate::paper_trading::PaperAccountView,
    bots: &[BotView],
    valuations: &[PaperExperimentValuation],
    now_ms: i64,
) -> PaperExperimentReport {
    let through_ms = experiment.observation_end_ms.min(now_ms);
    let experiment_order_ids = experiment
        .instruments
        .iter()
        .filter_map(|binding| binding.bot_id.as_deref())
        .flat_map(|bot_id| owned_order_ids(account, bot_id))
        .collect::<BTreeSet<_>>();
    let from_ms = experiment
        .started_at_ms
        .unwrap_or(experiment.observation_start_ms);
    let instruments = experiment
        .instruments
        .iter()
        .map(|binding| {
            instrument_report(
                experiment,
                binding,
                account,
                binding
                    .bot_id
                    .as_deref()
                    .and_then(|bot_id| bots.iter().find(|bot| bot.bot_id == bot_id)),
                valuations,
                through_ms,
            )
        })
        .collect::<Vec<_>>();
    let latest = experiment
        .instruments
        .iter()
        .map(|binding| {
            valuations
                .iter()
                .filter(|valuation| {
                    valuation.instrument == binding.instrument
                        && valuation.observed_at_ms >= from_ms
                        && valuation.observed_at_ms <= through_ms
                })
                .max_by_key(|valuation| valuation.observed_at_ms)
        })
        .collect::<Vec<_>>();
    let common_end_valuation_at_ms = latest
        .first()
        .and_then(|value| *value)
        .map(|value| value.observed_at_ms)
        .filter(|timestamp| {
            latest
                .iter()
                .all(|value| value.is_some_and(|value| value.observed_at_ms == *timestamp))
        });
    let mut limitations = experiment.limitations.clone();
    if account.orders.iter().any(|order| {
        order.submitted_at_ms >= from_ms
            && order.submitted_at_ms <= through_ms
            && !experiment_order_ids.contains(&order.order_id)
    }) || account.fills.iter().any(|fill| {
        fill.occurred_at_ms >= from_ms
            && fill.occurred_at_ms <= through_ms
            && !experiment_order_ids.contains(&fill.order_id)
    }) {
        limitations
            .push("Unrelated account activity was observed during the experiment window.".into());
    }
    if account.reconciliation != adaq_paper_trading_core::ReconciliationState::Reconciled {
        limitations.push("Final account reconciliation is not proven.".into());
    }
    if account.restart_required {
        limitations.push("The Paper Account still requires restart recovery.".into());
    }
    if account.orders.iter().any(|order| {
        matches!(
            order.status,
            OrderStatus::Accepted | OrderStatus::PartiallyFilled
        )
    }) {
        limitations.push("Pending risk remains at the declared experiment end.".into());
    }
    if account
        .provider_evidence
        .iter()
        .any(|outcome| matches!(outcome, ExecutionOutcome::Uncertain(_)))
    {
        limitations.push("Provider outcome evidence is uncertain.".into());
    }
    if experiment.started_at_ms.is_none() {
        limitations.push("The experiment has not been launched.".into());
    }
    if now_ms < experiment.observation_end_ms {
        limitations.push("The declared observation window has not ended.".into());
    }
    if common_end_valuation_at_ms.is_none() {
        limitations.push("All three instruments do not share one retained end valuation.".into());
    }
    if latest.iter().any(|value| {
        value.is_none_or(|value| !end_valuation_is_fresh(value, experiment.observation_end_ms))
    }) {
        limitations.push(
            "At least one end valuation is stale, outside the window, or has no provider timestamp."
                .into(),
        );
    }
    if experiment.starting_account_cash.is_some_and(|starting| {
        let delta = account.fills.iter().fold(Decimal::ZERO, |delta, fill| {
            if fill.occurred_at_ms
                < experiment
                    .started_at_ms
                    .unwrap_or(experiment.observation_start_ms)
                || fill.occurred_at_ms > through_ms
            {
                return delta;
            }
            let Some(order) = account
                .orders
                .iter()
                .find(|order| order.order_id == fill.order_id)
            else {
                return delta;
            };
            delta
                + cash_delta(
                    order.side,
                    fill.quantity,
                    fill.price,
                    reported_fee_quote(fill),
                )
        });
        account.account.cash != starting + delta
    }) {
        limitations.push("Account cash does not reconcile to retained experiment fills; external cash flow is unresolved.".into());
    }
    if account
        .account
        .positions
        .keys()
        .any(|instrument| !EXPERIMENT_INSTRUMENTS.contains(&instrument.as_str()))
    {
        limitations
            .push("Unrelated account positions prevent exact experiment attribution.".into());
    }
    for report in &instruments {
        limitations.extend(report.limitations.iter().cloned());
    }
    limitations.sort();
    limitations.dedup();
    let all_stopped = bots.len() == experiment.instruments.len()
        && bots
            .iter()
            .all(|bot| bot.state == adaq_bot_runtime::LifecycleState::Stopped);
    let all_samples = instruments
        .iter()
        .all(|report| report.valuation_count >= REQUIRED_VALUATIONS);
    let evidence_state = if now_ms < experiment.observation_end_ms || !all_stopped {
        EvidenceState::NotYetRealized
    } else if common_end_valuation_at_ms.is_none() {
        EvidenceState::Missing
    } else if !all_samples {
        EvidenceState::InsufficientEvidence
    } else if limitations.is_empty() {
        EvidenceState::Ready
    } else {
        EvidenceState::Unknown
    };
    let ranking_eligible = evidence_state == EvidenceState::Ready;
    let ranked_instruments = rank_instruments(&instruments, ranking_eligible);
    let best_instrument = ranked_instruments.first().cloned();
    PaperExperimentReport {
        report_id: Uuid::new_v4().to_string(),
        experiment_id: experiment.experiment_id.clone(),
        user_id: experiment.user_id.clone(),
        observation_start_ms: experiment.observation_start_ms,
        observation_end_ms: experiment.observation_end_ms,
        generated_at_ms: now_ms,
        valuation_cadence_ms: VALUATION_CADENCE_MS,
        common_end_valuation_at_ms,
        ranking_eligible,
        ranking_blocked_reason: (!ranking_eligible).then(|| {
            limitations
                .first()
                .cloned()
                .unwrap_or_else(|| "Exact ranking evidence is incomplete.".into())
        }),
        ranked_instruments,
        best_instrument,
        allocation_total_usdt: experiment.allocation_total_usdt,
        unallocated_remainder_usdt: experiment.unallocated_remainder_usdt,
        evidence_state,
        instruments,
        limitations,
    }
}

#[tauri::command]
pub(crate) fn paper_experiment_view(
    request: PaperExperimentViewRequest,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
    app: AppHandle,
) -> Result<Option<PaperExperimentView>, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    let experiment = match request.experiment_id {
        Some(experiment_id) => Some(state.paper_experiments.get(&user_id, &experiment_id)?),
        None => state.paper_experiments.latest(&user_id)?,
    };
    experiment
        .map(|experiment| build_view(&app, &state, experiment))
        .transpose()
}

#[tauri::command]
pub(crate) fn paper_experiment_create(
    request: PaperExperimentCreateRequest,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    state: State<'_, Arc<LocalResearchState>>,
    app: AppHandle,
) -> Result<PaperExperimentView, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    validate_create_request(&request)?;
    let experiment = PaperExperiment {
        experiment_id: Uuid::new_v4().to_string(),
        user_id: user_id.clone(),
        profile_id: request.profile_id,
        account_id: request.account_id,
        observation_start_ms: request.observation_start_ms,
        observation_end_ms: request.observation_end_ms,
        allocation_total_usdt: total_allocation(),
        unallocated_remainder_usdt: UNALLOCATED_REMAINDER_USDT,
        starting_account_cash: None,
        started_at_ms: None,
        instruments: normalized_bindings(request.bindings)?,
        state: PaperExperimentState::Draft,
        report_id: None,
        feedback_snapshot_ids: Vec::new(),
        feedback_report_ids: Vec::new(),
        limitations: Vec::new(),
        created_at_ms: adaq_bot_runtime::unix_now_ms(),
        updated_at_ms: adaq_bot_runtime::unix_now_ms(),
    };
    state.paper_experiments.create(&experiment)?;
    build_view(&app, &state, experiment)
}

fn launch_experiment(
    app: &AppHandle,
    user_id: &str,
    experiment_id: &str,
) -> Result<PaperExperimentView, String> {
    let local = app.state::<Arc<LocalResearchState>>();
    let qualifications = app.state::<Arc<StrategyQualificationStore>>();
    let mut experiment = local.paper_experiments.get(user_id, experiment_id)?;
    if !matches!(experiment.state, PaperExperimentState::Draft) {
        return Err("Only a new Draft Paper Experiment can be launched.".into());
    }
    if local
        .paper_experiments
        .active_for_account(user_id, &experiment.account_id)?
    {
        return Err("Another Paper Experiment is already active on this account.".into());
    }
    let now_ms = adaq_bot_runtime::unix_now_ms();
    if now_ms < experiment.observation_start_ms || now_ms >= experiment.observation_end_ms {
        return Err(
            "Launch requires the current time to be inside the explicit UTC observation window."
                .into(),
        );
    }
    if local.operations.blocks_new_risk(user_id)? {
        return Err(
            "A Host operational safety action is active; the experiment is blocked.".into(),
        );
    }
    validate_profile(
        local.as_ref(),
        user_id,
        &experiment.profile_id,
        &experiment.account_id,
    )?;
    validate_qualifications(local.as_ref(), &qualifications, user_id, &experiment)?;
    let account = reconcile_demo(local.as_ref(), user_id, &experiment.account_id)?;
    if account.account.account_id != experiment.account_id
        || account.reconciliation != adaq_paper_trading_core::ReconciliationState::Reconciled
        || account.restart_required
        || account.reserved_cash != Decimal::ZERO
        || account.orders.iter().any(|order| {
            matches!(
                order.status,
                OrderStatus::Accepted | OrderStatus::PartiallyFilled
            )
        })
        || !account.account.positions.is_empty()
        || account.account.cash < experiment.allocation_total_usdt
    {
        return Err("Fresh OKX Demo reconciliation must prove sufficient quiet USDT funds, no open orders, and no positions before launch.".into());
    }
    if account
        .provider_evidence
        .iter()
        .any(|outcome| matches!(outcome, ExecutionOutcome::Uncertain(_)))
    {
        return Err("An uncertain provider outcome blocks the experiment until reconciled.".into());
    }
    let bindings = experiment
        .instruments
        .iter()
        .map(|binding| (binding.instrument.clone(), binding.qualification_id.clone()))
        .collect::<Vec<_>>();
    let bots = crate::bot_operations::deploy_ema_experiment(
        app,
        user_id,
        &experiment.profile_id,
        &experiment.account_id,
        &bindings,
    )?;
    for (binding, bot) in experiment.instruments.iter_mut().zip(&bots) {
        binding.bot_id = Some(bot.bot_id.clone());
    }
    experiment.starting_account_cash = Some(account.account.cash);
    experiment.started_at_ms = Some(now_ms);
    experiment.state = PaperExperimentState::Preparing;
    experiment.updated_at_ms = now_ms;
    local.paper_experiments.save(&experiment)?;

    let mut started = Vec::new();
    for binding in &experiment.instruments {
        let bot_id = binding
            .bot_id
            .as_deref()
            .ok_or_else(|| "Experiment Bot identity was not retained.".to_owned())?;
        let command_id = format!("{}:start:{bot_id}", experiment.experiment_id);
        match crate::bot_operations::start_experiment_bot(app, user_id, bot_id, &command_id) {
            Ok(bot) if bot.state == adaq_bot_runtime::LifecycleState::Running => {
                started.push(bot_id.to_owned())
            }
            Ok(bot) => {
                let error = format!(
                    "Bot {bot_id} did not reach Running (state {:?}).",
                    bot.state
                );
                for started_bot in &started {
                    let _ = crate::bot_operations::stop_experiment_bot(
                        app,
                        user_id,
                        started_bot,
                        &format!("{}:rollback-stop:{started_bot}", experiment.experiment_id),
                    );
                }
                experiment.state = PaperExperimentState::Incomplete;
                experiment.limitations.push(error.clone());
                experiment.updated_at_ms = adaq_bot_runtime::unix_now_ms();
                local.paper_experiments.save(&experiment)?;
                return Err(error);
            }
            Err(error) => {
                for started_bot in &started {
                    let _ = crate::bot_operations::stop_experiment_bot(
                        app,
                        user_id,
                        started_bot,
                        &format!("{}:rollback-stop:{started_bot}", experiment.experiment_id),
                    );
                }
                experiment.state = PaperExperimentState::Incomplete;
                experiment
                    .limitations
                    .push(format!("Launch failed after partial start: {error}"));
                experiment.updated_at_ms = adaq_bot_runtime::unix_now_ms();
                local.paper_experiments.save(&experiment)?;
                return Err(error);
            }
        }
    }
    let prices = demo_prices(local.as_ref(), user_id, &experiment)?;
    let valuations = experiment_valuations(&experiment, &account, &prices, now_ms);
    local
        .paper_experiments
        .append_valuations(user_id, &valuations)?;
    build_view(app, local.as_ref(), experiment)
}

#[tauri::command]
pub(crate) async fn paper_experiment_launch(
    request: PaperExperimentIdRequest,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    app: AppHandle,
) -> Result<PaperExperimentView, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    tauri::async_runtime::spawn_blocking(move || {
        launch_experiment(&app, &user_id, &request.experiment_id)
    })
    .await
    .map_err(|error| error.to_string())?
}

fn refresh_experiment(
    app: &AppHandle,
    user_id: &str,
    experiment_id: &str,
) -> Result<PaperExperimentView, String> {
    let local = app.state::<Arc<LocalResearchState>>();
    let experiment = local.paper_experiments.get(user_id, experiment_id)?;
    let now_ms = adaq_bot_runtime::unix_now_ms();
    if matches!(
        experiment.state,
        PaperExperimentState::Preparing
            | PaperExperimentState::Running
            | PaperExperimentState::Armed
    ) && now_ms >= experiment.observation_end_ms
    {
        return stop_experiment(app, user_id, experiment_id);
    }
    let mut experiment = experiment;
    let mut limitations = sync_known_orders(local.as_ref(), user_id, &experiment);
    let account = reconcile_demo(local.as_ref(), user_id, &experiment.account_id)?;
    let prices = demo_prices(local.as_ref(), user_id, &experiment)?;
    let valuations = experiment_valuations(&experiment, &account, &prices, now_ms);
    local
        .paper_experiments
        .append_valuations(user_id, &valuations)?;
    experiment.limitations.append(&mut limitations);
    experiment.limitations.sort();
    experiment.limitations.dedup();
    experiment.updated_at_ms = now_ms;
    local.paper_experiments.save(&experiment)?;
    build_view(app, local.as_ref(), experiment)
}

#[tauri::command]
pub(crate) async fn paper_experiment_refresh(
    request: PaperExperimentIdRequest,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    app: AppHandle,
) -> Result<PaperExperimentView, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    tauri::async_runtime::spawn_blocking(move || {
        refresh_experiment(&app, &user_id, &request.experiment_id)
    })
    .await
    .map_err(|error| error.to_string())?
}

fn stop_experiment(
    app: &AppHandle,
    user_id: &str,
    experiment_id: &str,
) -> Result<PaperExperimentView, String> {
    let local = app.state::<Arc<LocalResearchState>>();
    let mut experiment = local.paper_experiments.get(user_id, experiment_id)?;
    if !matches!(
        experiment.state,
        PaperExperimentState::Preparing
            | PaperExperimentState::Armed
            | PaperExperimentState::Running
            | PaperExperimentState::Stopping
    ) {
        return Err("Only a Preparing, Armed, or Running experiment can be stopped.".into());
    }
    experiment.state = PaperExperimentState::Stopping;
    experiment.updated_at_ms = adaq_bot_runtime::unix_now_ms();
    local.paper_experiments.save(&experiment)?;
    let bot_ids = experiment
        .instruments
        .iter()
        .filter_map(|binding| binding.bot_id.clone())
        .collect::<Vec<_>>();
    if bot_ids.len() != EXPERIMENT_INSTRUMENTS.len() {
        let error = "The experiment does not retain all three Bot identities.".to_owned();
        persist_incomplete(local.as_ref(), &mut experiment, error.clone())?;
        return Err(error);
    }
    let mut shutdown_error = None;
    for bot_id in &bot_ids {
        if let Err(error) = crate::bot_operations::stop_experiment_bot(
            app,
            user_id,
            bot_id,
            &format!("{}:stop:{bot_id}", experiment.experiment_id),
        ) {
            shutdown_error.get_or_insert(format!("Bot {bot_id} shutdown is unresolved: {error}"));
        }
    }
    if let Some(error) = shutdown_error {
        persist_incomplete(local.as_ref(), &mut experiment, error.clone())?;
        return Err(error);
    }
    let mut sync_limitations = sync_known_orders(local.as_ref(), user_id, &experiment);
    experiment.limitations.append(&mut sync_limitations);
    let account = match reconcile_demo(local.as_ref(), user_id, &experiment.account_id) {
        Ok(account) => account,
        Err(error) => {
            persist_incomplete(
                local.as_ref(),
                &mut experiment,
                format!("Final reconciliation is unresolved: {error}"),
            )?;
            return Err(error);
        }
    };
    let prices = match demo_prices(local.as_ref(), user_id, &experiment) {
        Ok(prices) => prices,
        Err(error) => {
            persist_incomplete(
                local.as_ref(),
                &mut experiment,
                format!("Final valuation evidence is unresolved: {error}"),
            )?;
            return Err(error);
        }
    };
    let now_ms = adaq_bot_runtime::unix_now_ms();
    let valuations = experiment_valuations(&experiment, &account, &prices, now_ms);
    if let Err(error) = local
        .paper_experiments
        .append_valuations(user_id, &valuations)
    {
        persist_incomplete(
            local.as_ref(),
            &mut experiment,
            format!("Final valuation persistence is unresolved: {error}"),
        )?;
        return Err(error);
    }
    let quiet = account.account.account_id == experiment.account_id
        && account.reconciliation == adaq_paper_trading_core::ReconciliationState::Reconciled
        && account.reserved_cash == Decimal::ZERO
        && !account.restart_required
        && !account
            .provider_evidence
            .iter()
            .any(|outcome| matches!(outcome, ExecutionOutcome::Uncertain(_)))
        && account.orders.iter().all(|order| {
            !matches!(
                order.status,
                OrderStatus::Accepted | OrderStatus::PartiallyFilled
            )
        });
    experiment.state = if quiet {
        PaperExperimentState::Completed
    } else {
        PaperExperimentState::Incomplete
    };
    if !quiet {
        experiment
            .limitations
            .push("Final Stop + Keep Position did not prove a quiet reconciled account.".into());
    }
    experiment.updated_at_ms = now_ms;
    local.paper_experiments.save(&experiment)?;
    build_view(app, local.as_ref(), experiment)
}

#[tauri::command]
pub(crate) async fn paper_experiment_stop(
    request: PaperExperimentIdRequest,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    app: AppHandle,
) -> Result<PaperExperimentView, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    tauri::async_runtime::spawn_blocking(move || {
        stop_experiment(&app, &user_id, &request.experiment_id)
    })
    .await
    .map_err(|error| error.to_string())?
}

fn create_experiment_report(
    app: &AppHandle,
    user_id: &str,
    experiment_id: &str,
) -> Result<PaperExperimentReport, String> {
    let local = app.state::<Arc<LocalResearchState>>();
    let mut experiment = local.paper_experiments.get(user_id, experiment_id)?;
    if matches!(
        experiment.state,
        PaperExperimentState::Preparing
            | PaperExperimentState::Running
            | PaperExperimentState::Armed
    ) {
        let _ = refresh_experiment(app, user_id, experiment_id)?;
        experiment = local.paper_experiments.get(user_id, experiment_id)?;
    }
    if !report_state_is_terminal(experiment.state) {
        return Err(
            "Paper Experiment reports are available only after the observation window has ended."
                .into(),
        );
    }
    if let Some(report_id) = experiment.report_id.as_deref() {
        return local.paper_experiments.report_for_user(user_id, report_id);
    }
    let account = local.paper_trading.view_optional(user_id)?.ok_or_else(|| {
        "Paper Account evidence is unavailable for the experiment report.".to_owned()
    })?;
    let bots_store = app.state::<Arc<crate::bot_operations::BotStore>>();
    let bots = experiment
        .instruments
        .iter()
        .filter_map(|binding| binding.bot_id.as_deref())
        .map(|bot_id| bots_store.get(user_id, bot_id))
        .collect::<Result<Vec<_>, _>>()?;
    let valuations = local.paper_experiments.valuations(user_id, experiment_id)?;
    let report = build_report(
        &experiment,
        &account,
        &bots,
        &valuations,
        adaq_bot_runtime::unix_now_ms(),
    );
    local
        .paper_experiments
        .save_report(&mut experiment, &report)?;
    Ok(report)
}

#[tauri::command]
pub(crate) async fn paper_experiment_report_create(
    request: PaperExperimentIdRequest,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    app: AppHandle,
) -> Result<PaperExperimentReport, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    tauri::async_runtime::spawn_blocking(move || {
        create_experiment_report(&app, &user_id, &request.experiment_id)
    })
    .await
    .map_err(|error| error.to_string())?
}

fn create_experiment_feedback(
    app: &AppHandle,
    user_id: &str,
    experiment_id: &str,
) -> Result<PaperExperimentView, String> {
    let local = app.state::<Arc<LocalResearchState>>();
    let mut experiment = local.paper_experiments.get(user_id, experiment_id)?;
    if !experiment.feedback_snapshot_ids.is_empty() || !experiment.feedback_report_ids.is_empty() {
        if experiment.feedback_snapshot_ids.len() == EXPERIMENT_INSTRUMENTS.len()
            && experiment.feedback_report_ids.len() == EXPERIMENT_INSTRUMENTS.len() * 4
        {
            return build_view(app, local.as_ref(), experiment);
        }
        return Err(
            "Paper Feedback evidence is incomplete; review the existing evidence before retrying."
                .into(),
        );
    }
    let report = experiment
        .report_id
        .as_deref()
        .map(|report_id| local.paper_experiments.report_for_user(user_id, report_id))
        .transpose()?
        .ok_or_else(|| {
            "Create the immutable experiment report before Paper Feedback.".to_owned()
        })?;
    let valuations = local.paper_experiments.valuations(user_id, experiment_id)?;
    let bots_store = app.state::<Arc<crate::bot_operations::BotStore>>();
    let qualifications = app.state::<Arc<StrategyQualificationStore>>();
    let candidates = app.state::<Arc<StrategyCandidateStore>>();
    let account = local.paper_trading.view_optional(user_id)?;
    if account
        .as_ref()
        .is_some_and(|account| account.account.account_id != experiment.account_id)
    {
        return Err("Paper account evidence does not match the Paper Experiment account".into());
    }
    let health = local.operations.health_for_user(user_id)?;
    let mut snapshot_ids = Vec::new();
    let mut report_ids = Vec::new();
    for instrument in &report.instruments {
        let binding = experiment
            .instruments
            .iter()
            .find(|binding| binding.instrument == instrument.instrument)
            .ok_or_else(|| "Experiment report binding is incomplete.".to_owned())?;
        let bot_id = binding
            .bot_id
            .as_deref()
            .ok_or_else(|| "Experiment Bot identity is missing.".to_owned())?;
        let bot = bots_store.get(user_id, bot_id)?;
        let attempt_id = bot
            .current_attempt_id
            .clone()
            .ok_or_else(|| format!("Bot {bot_id} has no Runtime Attempt for feedback."))?;
        let attempt = bot
            .attempts
            .iter()
            .find(|attempt| attempt.attempt_id == attempt_id)
            .cloned()
            .ok_or_else(|| format!("Bot {bot_id} Runtime Attempt is unavailable for feedback."))?;
        let qualification =
            qualifications.qualification_for_user(user_id, &binding.qualification_id)?;
        let (market_snapshot, market_bars) =
            local.snapshot_for_user(user_id, &bot.bundle.market_data_snapshot_id)?;
        let (research_evidence, horizon_bars) = crate::paper_feedback_strategy_context(
            candidates.inner().as_ref(),
            user_id,
            &bot,
            &qualification,
        )?;
        let (market_evidence, realized) = crate::paper_feedback_market_evidence(
            &bot,
            &attempt,
            &market_snapshot,
            &market_bars,
            horizon_bars,
            report.observation_start_ms,
            report.observation_end_ms,
            report.generated_at_ms.max(report.observation_end_ms),
        );
        let mut evidence = crate::paper_feedback_evidence(
            &bot,
            &attempt,
            account.as_ref(),
            &health,
            research_evidence,
            market_evidence,
        );
        if let Some(object) = evidence.as_object_mut() {
            object.insert(
                "experiment".into(),
                json!({
                    "experimentId": experiment.experiment_id,
                    "reportId": report.report_id,
                    "instrument": instrument.instrument,
                    "observationStartMs": report.observation_start_ms,
                    "observationEndMs": report.observation_end_ms,
                    "valuationCadenceMs": report.valuation_cadence_ms,
                    "instrumentReport": instrument,
                    "valuations": valuations.iter().filter(|valuation| {
                        valuation.instrument == instrument.instrument
                            && valuation.observed_at_ms >= report.observation_start_ms
                            && valuation.observed_at_ms <= report.observation_end_ms
                    }).collect::<Vec<_>>(),
                }),
            );
        }
        let snapshot_state =
            crate::paper_feedback_state(attempt.state, realized, REQUIRED_VALUATIONS);
        let snapshot = local.paper_feedback.create_snapshot_with_state(
            crate::paper_feedback::FeedbackSnapshotInput {
                user_id: user_id.to_owned(),
                bundle_id: bot.bundle.identity.clone(),
                bot_id: bot_id.to_owned(),
                attempt_id,
                observation_start_ms: report.observation_start_ms,
                observation_end_ms: report.observation_end_ms,
                realization_cutoff_ms: report.generated_at_ms.max(report.observation_end_ms),
                realized_observations: realized,
                required_observations: REQUIRED_VALUATIONS,
                evidence,
            },
            report.generated_at_ms,
            Some(snapshot_state),
        )?;
        snapshot_ids.push(snapshot.snapshot_id.clone());
        for lens in [
            FeedbackLens::Factor,
            FeedbackLens::Model,
            FeedbackLens::Strategy,
            FeedbackLens::Execution,
        ] {
            let mut metrics = crate::paper_feedback_metrics(&snapshot, lens);
            if let Some(object) = metrics.as_object_mut() {
                object.insert("experimentId".into(), json!(experiment.experiment_id));
                object.insert("experimentReportId".into(), json!(report.report_id));
                object.insert("instrument".into(), json!(instrument.instrument));
                object.insert("experimentReport".into(), json!(instrument));
                object.insert(
                    "horizon".into(),
                    json!({
                        "observationStartMs": report.observation_start_ms,
                        "observationEndMs": report.observation_end_ms,
                    }),
                );
                object.insert(
                    "sampleRequirements".into(),
                    json!({
                        "requiredValuations": REQUIRED_VALUATIONS,
                        "realizedValuations": realized,
                    }),
                );
            }
            let feedback_state = crate::paper_feedback_report_state(&snapshot, lens, &metrics);
            let feedback_report = local.paper_feedback.create_report_with_state(
                crate::paper_feedback::FeedbackReportInput {
                    user_id: user_id.to_owned(),
                    snapshot_id: snapshot.snapshot_id.clone(),
                    lens,
                    metrics,
                    comparable_evidence_id: Some(report.report_id.clone()),
                },
                report.generated_at_ms,
                feedback_state,
            )?;
            report_ids.push(feedback_report.report_id);
        }
    }
    experiment.feedback_snapshot_ids = snapshot_ids;
    experiment.feedback_report_ids = report_ids;
    experiment.updated_at_ms = adaq_bot_runtime::unix_now_ms();
    local.paper_experiments.save(&experiment)?;
    build_view(app, local.as_ref(), experiment)
}

#[tauri::command]
pub(crate) async fn paper_experiment_feedback_create(
    request: PaperExperimentIdRequest,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    app: AppHandle,
) -> Result<PaperExperimentView, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    tauri::async_runtime::spawn_blocking(move || {
        create_experiment_feedback(&app, &user_id, &request.experiment_id)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bot_operations::BotEvidence;
    use adaq_paper_trading_core::{
        AccountSnapshot, AdapterKind, Currency, Fill, FillEvidence, Market, Order, ProviderEvidence,
    };
    use std::collections::BTreeMap;

    fn experiment() -> PaperExperiment {
        PaperExperiment {
            experiment_id: "experiment-1".into(),
            user_id: "alice".into(),
            profile_id: "profile-1".into(),
            account_id: "account-1".into(),
            observation_start_ms: 100,
            observation_end_ms: 300,
            allocation_total_usdt: total_allocation(),
            unallocated_remainder_usdt: UNALLOCATED_REMAINDER_USDT,
            starting_account_cash: Some(Decimal::new(300, 0)),
            started_at_ms: Some(100),
            instruments: vec![PaperExperimentInstrument {
                instrument: "BTC-USDT".into(),
                qualification_id: "qualification-1".into(),
                bot_id: Some("bot-1".into()),
                allocation_usdt: Decimal::new(100, 0),
                entry_notional_cap_usdt: Decimal::new(90, 0),
                reserved_cash_usdt: Decimal::new(10, 0),
            }],
            state: PaperExperimentState::Completed,
            report_id: None,
            feedback_snapshot_ids: Vec::new(),
            feedback_report_ids: Vec::new(),
            limitations: Vec::new(),
            created_at_ms: 100,
            updated_at_ms: 300,
        }
    }

    fn account() -> crate::paper_trading::PaperAccountView {
        crate::paper_trading::PaperAccountView {
            account: AccountSnapshot {
                account_id: "account-1".into(),
                user_id: "alice".into(),
                market: Market::OkxSpot,
                currency: Currency::Usdt,
                cash: Decimal::new(307, 0),
                positions: BTreeMap::new(),
                observed_at_ms: 300,
            },
            reserved_cash: Decimal::ZERO,
            buying_power: Decimal::new(307, 0),
            reconciliation: adaq_paper_trading_core::ReconciliationState::Reconciled,
            orders: vec![
                Order {
                    order_id: "order-buy".into(),
                    account_id: "account-1".into(),
                    instrument: "BTC-USDT".into(),
                    side: Side::Buy,
                    quantity: Decimal::ONE,
                    filled_quantity: Decimal::ONE,
                    limit_price: Decimal::new(100, 0),
                    status: OrderStatus::Filled,
                    submitted_at_ms: 110,
                },
                Order {
                    order_id: "order-sell".into(),
                    account_id: "account-1".into(),
                    instrument: "BTC-USDT".into(),
                    side: Side::Sell,
                    quantity: Decimal::ONE,
                    filled_quantity: Decimal::ONE,
                    limit_price: Decimal::new(110, 0),
                    status: OrderStatus::Filled,
                    submitted_at_ms: 200,
                },
            ],
            fills: vec![
                Fill {
                    fill_id: "fill-buy".into(),
                    order_id: "order-buy".into(),
                    quantity: Decimal::ONE,
                    price: Decimal::new(100, 0),
                    fee: Decimal::ONE,
                    fee_asset: Some("USDT".into()),
                    fee_quote: Some(Decimal::ONE),
                    fee_amount: Some(Decimal::ONE),
                    evidence: FillEvidence::TradeObserved,
                    occurred_at_ms: 110,
                },
                Fill {
                    fill_id: "fill-sell".into(),
                    order_id: "order-sell".into(),
                    quantity: Decimal::ONE,
                    price: Decimal::new(110, 0),
                    fee: Decimal::new(2, 0),
                    fee_asset: Some("USDT".into()),
                    fee_quote: Some(Decimal::new(2, 0)),
                    fee_amount: Some(Decimal::new(2, 0)),
                    evidence: FillEvidence::TradeObserved,
                    occurred_at_ms: 200,
                },
            ],
            provider_evidence: vec![
                ExecutionOutcome::Accepted(ProviderEvidence {
                    provider: AdapterKind::OkxDemo,
                    operation_id: "bot-bot-1-buy".into(),
                    local_order_id: Some("order-buy".into()),
                    provider_order_id: Some("remote-buy".into()),
                    status: "filled".into(),
                    error_code: None,
                    observed_at_ms: 110,
                }),
                ExecutionOutcome::Accepted(ProviderEvidence {
                    provider: AdapterKind::OkxDemo,
                    operation_id: "bot-bot-1-sell".into(),
                    local_order_id: Some("order-sell".into()),
                    provider_order_id: Some("remote-sell".into()),
                    status: "filled".into(),
                    error_code: None,
                    observed_at_ms: 200,
                }),
            ],
            risk_decisions: Vec::new(),
            restart_required: false,
        }
    }

    #[test]
    fn allocation_is_conserved_and_remainder_is_explicit() {
        let bindings = normalized_bindings(vec![
            PaperExperimentBindingRequest {
                instrument: "BTC-USDT".into(),
                qualification_id: "btc".into(),
            },
            PaperExperimentBindingRequest {
                instrument: "ETH-USDT".into(),
                qualification_id: "eth".into(),
            },
            PaperExperimentBindingRequest {
                instrument: "SOL-USDT".into(),
                qualification_id: "sol".into(),
            },
        ])
        .unwrap();
        assert_eq!(bindings.len(), 3);
        assert_eq!(
            bindings
                .iter()
                .map(|binding| binding.allocation_usdt)
                .sum::<Decimal>(),
            total_allocation()
        );
        assert_eq!(UNALLOCATED_REMAINDER_USDT, Decimal::new(951_395_709, 11));
    }

    #[test]
    fn report_position_attribution_removes_base_asset_fee() {
        let mut account = account();
        account.fills[0].fee_asset = Some("BTC".into());
        account.fills[0].fee_amount = Some(Decimal::new(1, 1));
        let owned = BTreeSet::from(["order-buy".to_owned()]);

        assert_eq!(
            attributed_position_quantity(&account, "BTC-USDT", 100, 300, Some(&owned),),
            Decimal::new(9, 1)
        );
    }

    #[test]
    fn eligible_report_ranks_observed_returns_deterministically() {
        let reports = vec![
            PaperExperimentInstrumentReport {
                instrument: "BTC-USDT".into(),
                bot_id: "bot-btc".into(),
                starting_capital_usdt: Decimal::new(100, 0),
                ending_cash_usdt: Decimal::new(101, 0),
                ending_position_quantity: Decimal::ZERO,
                ending_position_value_usdt: Some(Decimal::ZERO),
                realized_pnl_usdt: Decimal::ONE,
                unrealized_pnl_usdt: Some(Decimal::ZERO),
                fees_usdt: Decimal::ZERO,
                net_equity_return: Some(Decimal::new(1, 2)),
                max_drawdown: Some(Decimal::ZERO),
                completed_trades: 2,
                exposure_time_ms: 0,
                interruptions: 0,
                valuation_count: 2,
                evidence_state: EvidenceState::Ready,
                limitations: Vec::new(),
            },
            PaperExperimentInstrumentReport {
                instrument: "ETH-USDT".into(),
                bot_id: "bot-eth".into(),
                starting_capital_usdt: Decimal::new(100, 0),
                ending_cash_usdt: Decimal::new(102, 0),
                ending_position_quantity: Decimal::ZERO,
                ending_position_value_usdt: Some(Decimal::ZERO),
                realized_pnl_usdt: Decimal::new(2, 0),
                unrealized_pnl_usdt: Some(Decimal::ZERO),
                fees_usdt: Decimal::ZERO,
                net_equity_return: Some(Decimal::new(2, 2)),
                max_drawdown: Some(Decimal::ZERO),
                completed_trades: 2,
                exposure_time_ms: 0,
                interruptions: 0,
                valuation_count: 2,
                evidence_state: EvidenceState::Ready,
                limitations: Vec::new(),
            },
            PaperExperimentInstrumentReport {
                instrument: "SOL-USDT".into(),
                bot_id: "bot-sol".into(),
                starting_capital_usdt: Decimal::new(100, 0),
                ending_cash_usdt: Decimal::new(99, 0),
                ending_position_quantity: Decimal::ZERO,
                ending_position_value_usdt: Some(Decimal::ZERO),
                realized_pnl_usdt: Decimal::new(-1, 0),
                unrealized_pnl_usdt: Some(Decimal::ZERO),
                fees_usdt: Decimal::ZERO,
                net_equity_return: Some(Decimal::new(-1, 2)),
                max_drawdown: Some(Decimal::ZERO),
                completed_trades: 2,
                exposure_time_ms: 0,
                interruptions: 0,
                valuation_count: 2,
                evidence_state: EvidenceState::Ready,
                limitations: Vec::new(),
            },
        ];
        assert_eq!(
            rank_instruments(&reports, true),
            ["ETH-USDT", "BTC-USDT", "SOL-USDT"]
        );
        assert!(rank_instruments(&reports, false).is_empty());
    }

    #[test]
    fn end_valuation_accepts_a_fresh_poll_before_the_deadline() {
        let valuation = PaperExperimentValuation {
            experiment_id: "experiment-1".into(),
            instrument: "BTC-USDT".into(),
            observed_at_ms: 270_000,
            attributed_cash_usdt: Decimal::new(100, 0),
            position_quantity: Decimal::ZERO,
            price_usdt: Some(Decimal::new(100, 0)),
            equity_usdt: Some(Decimal::new(100, 0)),
            price_observed_at_ms: Some(270_000),
            source: "test".into(),
        };
        assert!(end_valuation_is_fresh(&valuation, 300_000));

        let mut stale = valuation.clone();
        stale.price_observed_at_ms = Some(100_000);
        assert!(!end_valuation_is_fresh(&stale, 300_000));

        let mut future = valuation;
        future.observed_at_ms = 300_001;
        assert!(!end_valuation_is_fresh(&future, 300_000));
    }

    #[test]
    fn stopping_experiment_blocks_new_bot_risk() {
        let database = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let store = PaperExperimentStore::open(database).unwrap();
        let mut experiment = experiment();
        experiment.state = PaperExperimentState::Armed;
        store.create(&experiment).unwrap();
        assert!(store.risk_blocked_for_bot("alice", "bot-1", 200).unwrap());
        experiment.state = PaperExperimentState::Running;
        store.save(&experiment).unwrap();
        assert!(store.risk_blocked_for_bot("alice", "bot-1", 99).unwrap());
        assert!(!store.risk_blocked_for_bot("alice", "bot-1", 200).unwrap());
        assert!(store.risk_blocked_for_bot("alice", "bot-1", 300).unwrap());
        experiment.state = PaperExperimentState::Stopping;
        store.save(&experiment).unwrap();
        assert!(store.risk_blocked_for_bot("alice", "bot-1", 200).unwrap());
    }

    #[test]
    fn preparing_experiment_reserves_account_before_arm() {
        let database = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let store = PaperExperimentStore::open(database).unwrap();
        let mut experiment = experiment();
        experiment.state = PaperExperimentState::Preparing;
        store.create(&experiment).unwrap();

        assert!(store.active_for_account("alice", "account-1").unwrap());
        assert!(store.risk_blocked_for_bot("alice", "bot-1", 200).unwrap());
        assert!(
            !store
                .decision_blocked_for_bot("alice", "bot-1", 200)
                .unwrap()
        );
    }

    #[test]
    fn pending_orders_must_be_owned_by_the_active_experiment() {
        let database = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let store = PaperExperimentStore::open(database).unwrap();
        let mut experiment = experiment();
        experiment.state = PaperExperimentState::Running;
        store.create(&experiment).unwrap();

        let mut account = account();
        account.orders[0].status = OrderStatus::Accepted;
        assert!(
            store
                .pending_orders_are_experiment_owned("alice", "bot-1", &account)
                .unwrap()
        );

        account.orders.push(Order {
            order_id: "external-order".into(),
            account_id: "account-1".into(),
            instrument: "ETH-USDT".into(),
            side: Side::Buy,
            quantity: Decimal::ONE,
            filled_quantity: Decimal::ZERO,
            limit_price: Decimal::new(100, 0),
            status: OrderStatus::Accepted,
            submitted_at_ms: 210,
        });
        assert!(
            !store
                .pending_orders_are_experiment_owned("alice", "bot-1", &account)
                .unwrap()
        );
    }

    #[test]
    fn terminal_experiment_states_block_decisions_after_preparing() {
        let database = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let store = PaperExperimentStore::open(database).unwrap();
        let mut experiment = experiment();
        experiment.state = PaperExperimentState::Preparing;
        store.create(&experiment).unwrap();
        assert!(
            !store
                .decision_blocked_for_bot("alice", "bot-1", 200)
                .unwrap()
        );
        experiment.state = PaperExperimentState::Stopping;
        store.save(&experiment).unwrap();
        assert!(
            store
                .decision_blocked_for_bot("alice", "bot-1", 200)
                .unwrap()
        );
    }

    #[test]
    fn completed_experiment_does_not_block_a_reused_bot() {
        let database = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let store = PaperExperimentStore::open(database).unwrap();
        let mut experiment = experiment();
        experiment.state = PaperExperimentState::Completed;
        store.create(&experiment).unwrap();

        assert!(
            !store
                .decision_blocked_for_bot("alice", "bot-1", 400)
                .unwrap()
        );
        assert!(!store.risk_blocked_for_bot("alice", "bot-1", 400).unwrap());
    }

    #[test]
    fn reports_require_a_terminal_experiment() {
        for state in [
            PaperExperimentState::Draft,
            PaperExperimentState::Preparing,
            PaperExperimentState::Armed,
            PaperExperimentState::Running,
            PaperExperimentState::Stopping,
        ] {
            assert!(!report_state_is_terminal(state));
        }
        assert!(report_state_is_terminal(PaperExperimentState::Completed));
        assert!(report_state_is_terminal(PaperExperimentState::Incomplete));
    }

    #[test]
    fn common_warmup_requires_each_current_experiment_bot() {
        let mut experiment = experiment();
        experiment.instruments = EXPERIMENT_INSTRUMENTS
            .iter()
            .map(|instrument| PaperExperimentInstrument {
                instrument: (*instrument).into(),
                qualification_id: format!("qualification-{instrument}"),
                bot_id: Some(format!("bot-{instrument}")),
                allocation_usdt: INITIAL_ALLOCATION_USDT,
                entry_notional_cap_usdt: ENTRY_NOTIONAL_CAP_USDT,
                reserved_cash_usdt: RESERVED_CASH_USDT,
            })
            .collect();
        let warmed = EXPERIMENT_INSTRUMENTS
            .iter()
            .map(|instrument| format!("bot-{instrument}"))
            .collect::<BTreeSet<_>>();
        assert!(all_experiment_bots_warmed(&experiment, &warmed));

        let mut incomplete = warmed;
        incomplete.remove("bot-ETH-USDT");
        assert!(!all_experiment_bots_warmed(&experiment, &incomplete));
    }

    #[test]
    fn resumed_attempt_does_not_reuse_old_warmup_evidence() {
        let mut attempt = BotRuntimeAttempt {
            attempt_id: "attempt-1".into(),
            bot_id: "bot-1".into(),
            bundle_identity: "bundle-1".into(),
            state: adaq_bot_runtime::LifecycleState::Running,
            stop_policy: None,
            events: Vec::new(),
            evidence: vec![
                BotEvidence {
                    kind: "lifecycle".into(),
                    code: "warmup-started".into(),
                    detail: String::new(),
                    related_id: None,
                    observed_at_ms: 10,
                },
                BotEvidence {
                    kind: "lifecycle".into(),
                    code: "warmup-complete".into(),
                    detail: String::new(),
                    related_id: None,
                    observed_at_ms: 30,
                },
                BotEvidence {
                    kind: "recovery".into(),
                    code: "worker-restarted-for-resume".into(),
                    detail: String::new(),
                    related_id: None,
                    observed_at_ms: 30,
                },
            ],
            decisions: Vec::new(),
            orders: Vec::new(),
            unmanaged_positions: Vec::new(),
            reconciliation_required: false,
            last_decision_time_ms: None,
            created_at_ms: 1,
            updated_at_ms: 30,
        };

        assert!(!attempt_is_warmed(&attempt));
        attempt.evidence.push(BotEvidence {
            kind: "lifecycle".into(),
            code: "warmup-complete".into(),
            detail: String::new(),
            related_id: None,
            observed_at_ms: 40,
        });
        assert!(attempt_is_warmed(&attempt));
    }

    #[test]
    fn report_arithmetic_attributes_converted_fees_exactly() {
        let experiment = experiment();
        let mut account = account();
        account.fills[0].fee = Decimal::new(1, 2);
        account.fills[0].fee_asset = Some("BTC".into());
        account.fills[0].fee_amount = Some(Decimal::new(1, 2));
        account.orders[1].quantity = Decimal::new(99, 2);
        account.orders[1].filled_quantity = Decimal::new(99, 2);
        account.fills[1].quantity = Decimal::new(99, 2);
        let valuations = vec![
            PaperExperimentValuation {
                experiment_id: "experiment-1".into(),
                instrument: "BTC-USDT".into(),
                observed_at_ms: 100,
                attributed_cash_usdt: Decimal::new(100, 0),
                position_quantity: Decimal::ONE,
                price_usdt: Some(Decimal::new(100, 0)),
                equity_usdt: Some(Decimal::new(100, 0)),
                price_observed_at_ms: Some(100),
                source: "test".into(),
            },
            PaperExperimentValuation {
                experiment_id: "experiment-1".into(),
                instrument: "BTC-USDT".into(),
                observed_at_ms: 300,
                attributed_cash_usdt: Decimal::new(1059, 1),
                position_quantity: Decimal::ZERO,
                price_usdt: Some(Decimal::new(110, 0)),
                equity_usdt: Some(Decimal::new(1059, 1)),
                price_observed_at_ms: Some(300),
                source: "test".into(),
            },
            PaperExperimentValuation {
                experiment_id: "experiment-1".into(),
                instrument: "BTC-USDT".into(),
                observed_at_ms: 400,
                attributed_cash_usdt: Decimal::new(1000, 0),
                position_quantity: Decimal::ZERO,
                price_usdt: Some(Decimal::new(999, 0)),
                equity_usdt: Some(Decimal::new(1000, 0)),
                price_observed_at_ms: Some(400),
                source: "test-after-window".into(),
            },
        ];
        let report = instrument_report(
            &experiment,
            &experiment.instruments[0],
            &account,
            None,
            &valuations,
            300,
        );
        assert!((report.realized_pnl_usdt - Decimal::new(59, 1)).abs() < Decimal::new(1, 20));
        assert_eq!(report.fees_usdt, Decimal::new(3, 0));
        assert_eq!(report.ending_cash_usdt, Decimal::new(1059, 1));
        assert_eq!(report.completed_trades, 2);
        assert_eq!(report.net_equity_return, Some(Decimal::new(59, 3)));
        assert_eq!(report.evidence_state, EvidenceState::Ready);
    }

    #[test]
    fn external_same_instrument_activity_is_not_attributed() {
        let experiment = experiment();
        let mut account = account();
        account.orders.push(Order {
            order_id: "external-order".into(),
            account_id: "account-1".into(),
            instrument: "BTC-USDT".into(),
            side: Side::Buy,
            quantity: Decimal::ONE,
            filled_quantity: Decimal::ONE,
            limit_price: Decimal::new(50, 0),
            status: OrderStatus::Filled,
            submitted_at_ms: 150,
        });
        account.fills.push(Fill {
            fill_id: "external-fill".into(),
            order_id: "external-order".into(),
            quantity: Decimal::ONE,
            price: Decimal::new(50, 0),
            fee: Decimal::ONE,
            fee_asset: Some("USDT".into()),
            fee_quote: Some(Decimal::ONE),
            fee_amount: Some(Decimal::ONE),
            evidence: FillEvidence::TradeObserved,
            occurred_at_ms: 150,
        });
        let valuations = vec![
            PaperExperimentValuation {
                experiment_id: "experiment-1".into(),
                instrument: "BTC-USDT".into(),
                observed_at_ms: 100,
                attributed_cash_usdt: Decimal::new(100, 0),
                position_quantity: Decimal::ZERO,
                price_usdt: Some(Decimal::new(100, 0)),
                equity_usdt: Some(Decimal::new(100, 0)),
                price_observed_at_ms: Some(100),
                source: "test".into(),
            },
            PaperExperimentValuation {
                experiment_id: "experiment-1".into(),
                instrument: "BTC-USDT".into(),
                observed_at_ms: 300,
                attributed_cash_usdt: Decimal::new(107, 0),
                position_quantity: Decimal::ZERO,
                price_usdt: Some(Decimal::new(110, 0)),
                equity_usdt: Some(Decimal::new(107, 0)),
                price_observed_at_ms: Some(300),
                source: "test".into(),
            },
        ];
        let report = instrument_report(
            &experiment,
            &experiment.instruments[0],
            &account,
            None,
            &valuations,
            300,
        );
        assert_eq!(report.ending_cash_usdt, Decimal::new(107, 0));
        assert_eq!(report.fees_usdt, Decimal::new(3, 0));
        assert_eq!(report.evidence_state, EvidenceState::Unknown);
        assert!(
            report
                .limitations
                .iter()
                .any(|limitation| limitation.contains("Unattributed activity"))
        );
    }

    #[test]
    fn report_marks_missing_valuation_instead_of_ranking() {
        let experiment = experiment();
        let report = instrument_report(
            &experiment,
            &experiment.instruments[0],
            &account(),
            None,
            &[],
            300,
        );
        assert_eq!(report.evidence_state, EvidenceState::Missing);
        assert!(
            report
                .limitations
                .iter()
                .any(|limitation| limitation.contains("end valuation"))
        );
    }

    #[test]
    fn no_trade_report_is_visible_but_not_execution_complete() {
        let experiment = experiment();
        let mut account = account();
        account.account.cash = Decimal::new(100, 0);
        account.orders.clear();
        account.fills.clear();
        account.provider_evidence.clear();
        let valuations = vec![
            PaperExperimentValuation {
                experiment_id: "experiment-1".into(),
                instrument: "BTC-USDT".into(),
                observed_at_ms: 100,
                attributed_cash_usdt: Decimal::new(100, 0),
                position_quantity: Decimal::ZERO,
                price_usdt: Some(Decimal::new(100, 0)),
                equity_usdt: Some(Decimal::new(100, 0)),
                price_observed_at_ms: Some(100),
                source: "test".into(),
            },
            PaperExperimentValuation {
                experiment_id: "experiment-1".into(),
                instrument: "BTC-USDT".into(),
                observed_at_ms: 300,
                attributed_cash_usdt: Decimal::new(100, 0),
                position_quantity: Decimal::ZERO,
                price_usdt: Some(Decimal::new(100, 0)),
                equity_usdt: Some(Decimal::new(100, 0)),
                price_observed_at_ms: Some(300),
                source: "test".into(),
            },
        ];
        let report = instrument_report(
            &experiment,
            &experiment.instruments[0],
            &account,
            None,
            &valuations,
            300,
        );
        assert_eq!(report.completed_trades, 0);
        assert_eq!(report.net_equity_return, Some(Decimal::ZERO));
        assert_eq!(report.evidence_state, EvidenceState::Unknown);
        assert!(
            report
                .limitations
                .iter()
                .any(|limitation| { limitation.contains("No owned provider Fill") })
        );
    }
}
