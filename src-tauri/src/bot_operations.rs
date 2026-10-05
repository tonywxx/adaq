//! Host-owned Bot deployment records and lifecycle control.
//!
//! SQLite stores one immutable Bundle per Bot, separate Runtime Attempts,
//! command effects, and the account lease. Provider credentials and Worker
//! processes stay behind their existing Host-only seams.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use adaq_backtest_core::{
    ExecutionProfile, PortfolioPosition, PortfolioState, PortfolioTarget,
    RiskPolicy as ResearchRiskPolicy,
};
use adaq_bot_runtime::{
    DecisionClock, DeploymentBundle, LifecycleState, RuntimeEvent, WORKER_ARTIFACT_NAME,
    WORKER_SIGNATURE_SCHEMA_VERSION, WorkerArtifactBinding, WorkerArtifactSignature,
    WorkerComponentLaunch, WorkerDecisionInput, WorkerDecisionResult, WorkerEvaluationEvidence,
    WorkerLaunchRequest, WorkerTarget,
};
use adaq_component_tooling::{ComponentKind, ComponentPackage, FactorScope, ParameterType};
use adaq_data_core::{MarketTrade, OhlcvBar};
use adaq_paper_trading_core::RiskPolicy as PaperRiskPolicy;
use adaq_trading_crypto::Exchange;
use rusqlite::{Connection, OptionalExtension, params};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State, WebviewWindow};
use uuid::Uuid;

use crate::{
    auth::AuthState,
    connections::{ProfileStatus, Provider, RuntimeGuard},
    local_research::LocalResearchState,
    paper_order_dispatch::{self, DispatchError, ProviderOrderKind},
    paper_trading::{PaperAccountView, PaperOrderRequest},
    strategy_candidate::StrategyCandidateRevision,
    strategy_candidate::{StrategyCandidateStore, StrategyInputBinding, StrategyScope},
    strategy_qualification::StrategyPackageProvenance,
    strategy_qualification::{StrategyQualification, StrategyQualificationStore},
    user::validate_user,
};

const BOT_SCHEMA_VERSION: &str = "adaq:bot@2";
const LEGACY_BOT_SCHEMA_VERSION: &str = "adaq:bot@1";
const MAX_ATTEMPTS: usize = 64;
const MAX_EVIDENCE: usize = 256;
const MAX_DECISIONS: usize = 512;
const MAX_ORDERS: usize = 512;
const MAX_TEXT_BYTES: usize = 512;
// ponytail: fixed 30s host deadline until schedule metadata carries a venue-specific policy.
const DECISION_DEADLINE_GRACE_MS: i64 = 30_000;
const EMA_INITIAL_ALLOCATION_USDT: Decimal = Decimal::from_parts(3_269_476, 0, 0, false, 2);
const EMA_ENTRY_NOTIONAL_CAP_USDT: Decimal = Decimal::from_parts(3_236_781, 0, 0, false, 2);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(crate) enum BotSchedule {
    ClosedBar {
        instrument_id: String,
        interval: String,
    },
    ScheduledCrossSection {
        universe_id: String,
        instruments: Vec<String>,
    },
    EmaDoubleCross {
        instrument_id: String,
    },
}

impl BotSchedule {
    pub(crate) fn operational_name(&self) -> String {
        match self {
            Self::ClosedBar {
                instrument_id,
                interval,
            } => format!(
                "OKX-DEMO-BOT_Closed-Bar_{}_{interval}",
                okx_instrument_code(instrument_id)
            ),
            Self::ScheduledCrossSection {
                universe_id,
                instruments,
            } => format!(
                "OKX-DEMO-BOT_Cross-Section_{universe_id}_{}",
                instruments
                    .iter()
                    .map(|instrument| okx_instrument_code(instrument))
                    .collect::<Vec<_>>()
                    .join("-")
            ),
            Self::EmaDoubleCross { instrument_id } => {
                format!(
                    "OKX-DEMO-BOT_EMA-Double-Cross_{}",
                    okx_instrument_code(instrument_id)
                )
            }
        }
    }

    fn validate(&self, scope: StrategyScope, expected_universe_id: &str) -> Result<(), String> {
        match self {
            Self::ClosedBar {
                instrument_id,
                interval,
            } => {
                if scope != StrategyScope::SingleInstrument
                    || !bounded(instrument_id, 128)
                    || !bounded(interval, 32)
                    || !adaq_data_core::BarInterval::ALL
                        .iter()
                        .any(|candidate| candidate.as_str() == interval)
                {
                    return Err("ClosedBar schedule does not match the qualified Strategy".into());
                }
            }
            Self::ScheduledCrossSection {
                universe_id,
                instruments,
            } => {
                if scope != StrategyScope::Portfolio
                    || universe_id != expected_universe_id
                    || instruments.is_empty()
                    || instruments.len() > 512
                    || instruments
                        .iter()
                        .any(|instrument| !bounded(instrument, 128))
                    || has_duplicates(instruments)
                {
                    return Err(
                        "ScheduledCrossSection schedule does not match the qualified Strategy"
                            .into(),
                    );
                }
            }
            Self::EmaDoubleCross { instrument_id } => {
                if scope != StrategyScope::SingleInstrument
                    || !bounded(instrument_id, 128)
                    || !matches!(
                        okx_instrument_code(instrument_id),
                        "BTC-USDT" | "ETH-USDT" | "SOL-USDT"
                    )
                {
                    return Err(
                        "EmaDoubleCross schedule does not match the qualified Strategy".into(),
                    );
                }
            }
        }
        Ok(())
    }

    pub(crate) fn contains_external_instrument(&self, instrument: &str) -> bool {
        match self {
            Self::ClosedBar { instrument_id, .. } => {
                instrument_id == instrument || okx_instrument_code(instrument_id) == instrument
            }
            Self::ScheduledCrossSection { instruments, .. } => {
                instruments.iter().any(|candidate| {
                    candidate == instrument || okx_instrument_code(candidate) == instrument
                })
            }
            Self::EmaDoubleCross { instrument_id } => {
                instrument_id == instrument || okx_instrument_code(instrument_id) == instrument
            }
        }
    }
}

fn canonical_okx_instrument_id(instrument: &str) -> Result<String, String> {
    let code = okx_instrument_code(instrument);
    if code.is_empty() || code.contains(':') {
        return Err("OKX instrument codes must not contain a source prefix.".into());
    }
    Ok(format!("okx:{code}"))
}

fn okx_instrument_code(instrument: &str) -> &str {
    instrument.strip_prefix("okx:").unwrap_or(instrument)
}

fn bot_owned_position(
    account: &PaperAccountView,
    bot_id: &str,
    instrument_id: &str,
) -> adaq_paper_trading_core::Position {
    let operation_prefix = format!("bot-{bot_id}-");
    let owned_order_ids = account
        .provider_evidence
        .iter()
        .filter_map(|outcome| {
            let evidence = match outcome {
                adaq_paper_trading_core::ExecutionOutcome::Accepted(evidence)
                | adaq_paper_trading_core::ExecutionOutcome::Rejected(evidence)
                | adaq_paper_trading_core::ExecutionOutcome::Uncertain(evidence) => evidence,
            };
            evidence
                .operation_id
                .starts_with(&operation_prefix)
                .then(|| evidence.local_order_id.clone())
                .flatten()
        })
        .collect::<BTreeSet<_>>();
    let quantity = account
        .fills
        .iter()
        .filter(|fill| owned_order_ids.contains(&fill.order_id))
        .filter_map(|fill| {
            let order = account
                .orders
                .iter()
                .find(|order| order.order_id == fill.order_id)?;
            (okx_instrument_code(&order.instrument) == okx_instrument_code(instrument_id))
                .then_some(match order.side {
                    adaq_paper_trading_core::Side::Buy => {
                        fill.quantity - fill.fee_in_base(&order.instrument)
                    }
                    adaq_paper_trading_core::Side::Sell => {
                        -(fill.quantity + fill.fee_in_base(&order.instrument))
                    }
                })
        })
        .sum::<Decimal>()
        .max(Decimal::ZERO);
    let sellable_quantity = account
        .account
        .positions
        .get(okx_instrument_code(instrument_id))
        .map(|position| position.sellable_quantity)
        .unwrap_or_default()
        .min(quantity);
    adaq_paper_trading_core::Position {
        quantity,
        sellable_quantity,
    }
}

fn canonicalize_okx_schedule(schedule: BotSchedule) -> Result<BotSchedule, String> {
    match schedule {
        BotSchedule::ClosedBar {
            instrument_id,
            interval,
        } => Ok(BotSchedule::ClosedBar {
            instrument_id: canonical_okx_instrument_id(&instrument_id)?,
            interval,
        }),
        BotSchedule::ScheduledCrossSection {
            universe_id,
            instruments,
        } => Ok(BotSchedule::ScheduledCrossSection {
            universe_id,
            instruments: instruments
                .iter()
                .map(|instrument| canonical_okx_instrument_id(instrument))
                .collect::<Result<_, _>>()?,
        }),
        BotSchedule::EmaDoubleCross { instrument_id } => Ok(BotSchedule::EmaDoubleCross {
            instrument_id: canonical_okx_instrument_id(&instrument_id)?,
        }),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BotDeploymentBundle {
    pub schema_version: String,
    pub bot_id: String,
    pub qualification_id: String,
    pub candidate_id: String,
    pub candidate_revision: u64,
    pub candidate_revision_hash: String,
    pub universe_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub universe_snapshot_id: String,
    pub market_data_snapshot_id: String,
    pub strategy_package_archive_sha256: String,
    pub pipeline_package_archive_sha256: Vec<String>,
    pub account_id: String,
    pub connection_profile_id: String,
    pub schedule: BotSchedule,
    pub research_risk_policy: ResearchRiskPolicy,
    pub paper_risk_policy: PaperRiskPolicy,
    pub execution_profile: ExecutionProfile,
    pub runtime_bundle: DeploymentBundle,
    pub created_at_ms: i64,
    pub identity: String,
}

impl BotDeploymentBundle {
    pub(crate) fn freeze(mut self) -> Result<Self, String> {
        self.identity.clear();
        self.verify_contents()?;
        self.identity = hash_json(&self.without_identity())?;
        Ok(self)
    }

    pub(crate) fn verify(&self) -> Result<(), String> {
        self.verify_contents()?;
        let expected = hash_json(&self.without_identity())?;
        if self.identity != expected {
            return Err("Bot Deployment Bundle identity is mutated".into());
        }
        Ok(())
    }

    pub(crate) fn verify_for_feedback(&self) -> Result<(), String> {
        self.verify_contents_with_runtime_validation(false)?;
        let expected = hash_json(&self.without_identity())?;
        if self.identity != expected {
            return Err("Bot Deployment Bundle identity is mutated".into());
        }
        Ok(())
    }

    fn without_identity(&self) -> Self {
        let mut copy = self.clone();
        copy.identity.clear();
        copy
    }

    fn verify_contents(&self) -> Result<(), String> {
        self.verify_contents_with_runtime_validation(true)
    }

    fn verify_contents_with_runtime_validation(
        &self,
        validate_runtime: bool,
    ) -> Result<(), String> {
        let research_risk_policy_hash = hash_json(&self.research_risk_policy)?;
        let execution_profile_hash = hash_json(&self.execution_profile)?;
        let legacy = self.schema_version == LEGACY_BOT_SCHEMA_VERSION
            && self.universe_snapshot_id.is_empty();
        if (self.schema_version != BOT_SCHEMA_VERSION && !legacy)
            || !bounded(&self.bot_id, 256)
            || !bounded(&self.qualification_id, 256)
            || !bounded(&self.candidate_id, 256)
            || !bounded(&self.candidate_revision_hash, 128)
            || !bounded(&self.universe_id, 256)
            || (!legacy && !bounded(&self.universe_snapshot_id, 256))
            || !bounded(&self.market_data_snapshot_id, 256)
            || !is_sha256(&self.strategy_package_archive_sha256)
            || self.pipeline_package_archive_sha256.len() > 64
            || self
                .pipeline_package_archive_sha256
                .iter()
                .any(|hash| !is_sha256(hash))
            || has_duplicates(&self.pipeline_package_archive_sha256)
            || !bounded(&self.account_id, 256)
            || !bounded(&self.connection_profile_id, 256)
            || self.created_at_ms <= 0
            || !bounded(&self.research_risk_policy.policy_id, 128)
            || self.research_risk_policy.max_instrument_weight < Decimal::ZERO
            || self.research_risk_policy.max_instrument_weight > Decimal::ONE
            || self
                .research_risk_policy
                .max_turnover
                .is_some_and(|value| value < Decimal::ZERO)
            || self.paper_risk_policy.max_order_notional <= Decimal::ZERO
            || self.paper_risk_policy.reserve_cash < Decimal::ZERO
            || self.execution_profile.price_increment <= Decimal::ZERO
            || self.execution_profile.quantity_increment <= Decimal::ZERO
            || self.execution_profile.minimum_quantity < Decimal::ZERO
            || self.execution_profile.maker_fee_rate < Decimal::ZERO
            || self.execution_profile.taker_fee_rate < Decimal::ZERO
            || self.execution_profile.adverse_slippage_rate < Decimal::ZERO
            || self.execution_profile.rebalance_threshold < Decimal::ZERO
            || self.runtime_bundle.input.bot_id != self.bot_id
            || self.runtime_bundle.input.strategy_id != self.qualification_id
            || self.runtime_bundle.input.account_id != self.account_id
            || self.runtime_bundle.input.risk_policy_hash != research_risk_policy_hash
            || self.runtime_bundle.input.execution_profile_hash != execution_profile_hash
        {
            return Err("Bot Deployment Bundle is invalid".into());
        }
        if validate_runtime {
            self.runtime_bundle
                .verify()
                .map_err(|error| error.to_string())?;
        } else {
            self.runtime_bundle
                .verify_for_feedback()
                .map_err(|error| error.to_string())?;
        }
        self.schedule
            .validate(runtime_scope(&self.runtime_bundle), &self.universe_id)?;
        let expected_decision_mode = matches!(&self.schedule, BotSchedule::EmaDoubleCross { .. })
            .then_some(adaq_bot_runtime::ema_double_cross::EMA_DECISION_MODE);
        if self.runtime_bundle.input.decision_mode.as_deref() != expected_decision_mode {
            return Err("Bot decision mode does not match the immutable schedule".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum BotStopPolicy {
    KeepPosition,
    Flatten,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BotEvidence {
    pub kind: String,
    pub code: String,
    pub detail: String,
    pub related_id: Option<String>,
    pub observed_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DecisionClaim {
    New,
    Duplicate,
    Conflict,
    Stale,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BotDecisionEvidence {
    pub request_id: String,
    pub decision_id: String,
    pub outcome: String,
    #[serde(default)]
    pub no_target_reason: Option<adaq_bot_runtime::NoTargetReason>,
    #[serde(default)]
    pub no_target_detail: Option<String>,
    pub target_hash: Option<String>,
    #[serde(default)]
    pub target: Option<WorkerTarget>,
    #[serde(default)]
    pub clock: Option<DecisionClock>,
    #[serde(default)]
    pub evaluation: Option<WorkerEvaluationEvidence>,
    #[serde(default)]
    pub market_data_universe_snapshot_id: Option<String>,
    pub observed_at_ms: i64,
}

#[cfg(test)]
mod bot_decision_evidence_tests {
    use super::BotDecisionEvidence;
    use serde_json::json;

    #[test]
    fn legacy_decisions_deserialize_without_no_target_diagnostics() {
        let evidence: BotDecisionEvidence = serde_json::from_value(json!({
            "requestId": "request-1",
            "decisionId": "decision-1",
            "outcome": "no-target",
            "targetHash": null,
            "target": null,
            "clock": null,
            "evaluation": null,
            "observedAtMs": 1
        }))
        .unwrap();

        assert_eq!(evidence.no_target_reason, None);
        assert_eq!(evidence.no_target_detail, None);
    }

    #[test]
    fn decision_diagnostics_serialize_as_kebab_case_evidence() {
        let evidence: BotDecisionEvidence = serde_json::from_value(json!({
            "requestId": "request-1",
            "decisionId": "decision-1",
            "outcome": "no-target",
            "noTargetReason": "missing-input",
            "noTargetDetail": "one or more feature values are missing",
            "targetHash": null,
            "target": null,
            "clock": null,
            "evaluation": null,
            "observedAtMs": 1
        }))
        .unwrap();

        let serialized = serde_json::to_value(evidence).unwrap();
        assert_eq!(serialized["noTargetReason"], "missing-input");
        assert_eq!(
            serialized["noTargetDetail"],
            "one or more feature values are missing"
        );
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BotOrderEvidence {
    pub operation_id: String,
    pub decision_id: Option<String>,
    pub status: String,
    pub provider_order_id: Option<String>,
    pub observed_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BotRuntimeAttempt {
    pub attempt_id: String,
    pub bot_id: String,
    pub bundle_identity: String,
    pub state: LifecycleState,
    pub stop_policy: Option<BotStopPolicy>,
    pub events: Vec<RuntimeEvent>,
    pub evidence: Vec<BotEvidence>,
    pub decisions: Vec<BotDecisionEvidence>,
    pub orders: Vec<BotOrderEvidence>,
    pub unmanaged_positions: Vec<String>,
    pub reconciliation_required: bool,
    pub last_decision_time_ms: Option<i64>,
    #[serde(default)]
    pub last_event_stream_epoch: Option<u32>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Clone)]
pub(crate) struct BotAttemptIdentity {
    pub attempt_id: String,
    pub bundle_identity: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PersistedBot {
    bot_id: String,
    user_id: String,
    bundle: BotDeploymentBundle,
    state: LifecycleState,
    current_attempt_id: Option<String>,
    attempts: Vec<BotRuntimeAttempt>,
    created_at_ms: i64,
    updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BotControlView {
    pub can_start: bool,
    pub can_retry: bool,
    pub can_pause: bool,
    pub can_resume: bool,
    pub can_stop: bool,
    pub can_flatten: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BotView {
    pub bot_id: String,
    pub user_id: String,
    pub bundle: BotDeploymentBundle,
    pub state: LifecycleState,
    pub current_attempt_id: Option<String>,
    pub attempts: Vec<BotRuntimeAttempt>,
    pub control: BotControlView,
}

impl PersistedBot {
    fn view(&self) -> BotView {
        BotView {
            bot_id: self.bot_id.clone(),
            user_id: self.user_id.clone(),
            bundle: self.bundle.clone(),
            state: self.state,
            current_attempt_id: self.current_attempt_id.clone(),
            attempts: self.attempts.clone(),
            control: controls_for(self.state),
        }
    }
}

#[derive(Clone)]
pub(crate) struct BotStore {
    database: Arc<Mutex<Connection>>,
    // ponytail: one control lock serializes all Bot commands; split per Bot if measured concurrency requires it.
    control: Arc<Mutex<()>>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DecisionCommandResult {
    error: Option<String>,
}

impl BotStore {
    pub(crate) fn open(database: Arc<Mutex<Connection>>) -> Result<Self, String> {
        let store = Self {
            database,
            control: Arc::new(Mutex::new(())),
        };
        store
            .database
            .lock()
            .map_err(|error| error.to_string())?
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS bots (
                    bot_id TEXT PRIMARY KEY,
                    user_id TEXT NOT NULL,
                    bundle_json TEXT NOT NULL,
                    state TEXT NOT NULL,
                    current_attempt_id TEXT,
                    attempts_json TEXT NOT NULL,
                    created_at_ms INTEGER NOT NULL,
                    updated_at_ms INTEGER NOT NULL
                );
                CREATE INDEX IF NOT EXISTS bots_user_updated_idx
                    ON bots(user_id, updated_at_ms DESC, bot_id DESC);
                CREATE TABLE IF NOT EXISTS bot_account_leases (
                    account_id TEXT PRIMARY KEY,
                    bot_id TEXT NOT NULL,
                    user_id TEXT NOT NULL,
                    attempt_id TEXT NOT NULL,
                    acquired_at_ms INTEGER NOT NULL
                );
                CREATE TABLE IF NOT EXISTS bot_instrument_leases (
                    account_id TEXT NOT NULL,
                    instrument_id TEXT NOT NULL,
                    bot_id TEXT NOT NULL,
                    user_id TEXT NOT NULL,
                    attempt_id TEXT NOT NULL,
                    acquired_at_ms INTEGER NOT NULL,
                    PRIMARY KEY(account_id, instrument_id)
                );
                CREATE TABLE IF NOT EXISTS bot_commands (
                    user_id TEXT NOT NULL,
                    bot_id TEXT NOT NULL,
                    command_id TEXT NOT NULL,
                    command_kind TEXT NOT NULL,
                    result_json TEXT NOT NULL,
                    created_at_ms INTEGER NOT NULL,
                    PRIMARY KEY(user_id, bot_id, command_id)
                );
                CREATE INDEX IF NOT EXISTS bot_commands_created_idx
                    ON bot_commands(user_id, created_at_ms DESC);
                CREATE TABLE IF NOT EXISTS bot_decision_claims (
                    user_id TEXT NOT NULL,
                    bot_id TEXT NOT NULL,
                    attempt_id TEXT NOT NULL,
                    request_id TEXT NOT NULL,
                    decision_id TEXT NOT NULL,
                    created_at_ms INTEGER NOT NULL,
                    PRIMARY KEY(user_id, bot_id, attempt_id, decision_id),
                    UNIQUE(user_id, bot_id, attempt_id, request_id)
                );
                CREATE TABLE IF NOT EXISTS bot_runtime_attempts (
                    user_id TEXT NOT NULL,
                    bot_id TEXT NOT NULL,
                    attempt_id TEXT NOT NULL,
                    position INTEGER NOT NULL,
                    attempt_json TEXT NOT NULL,
                    updated_at_ms INTEGER NOT NULL,
                    PRIMARY KEY(user_id, bot_id, attempt_id)
                );",
            )
            .map_err(|error| error.to_string())?;
        store.migrate_legacy_attempts()?;
        Ok(store)
    }

    fn migrate_legacy_attempts(&self) -> Result<(), String> {
        let mut database = self.database.lock().map_err(|error| error.to_string())?;
        let rows = {
            let mut statement = database
                .prepare(
                    "SELECT bot_id, user_id, attempts_json FROM bots
                     WHERE attempts_json <> '[]'",
                )
                .map_err(|error| error.to_string())?;
            statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })
                .map_err(|error| error.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?
        };
        if rows.is_empty() {
            return Ok(());
        }
        let transaction = database.transaction().map_err(|error| error.to_string())?;
        for (bot_id, user_id, attempts_json) in rows {
            let attempts: Vec<BotRuntimeAttempt> =
                serde_json::from_str(&attempts_json).map_err(|error| error.to_string())?;
            for (position, attempt) in attempts.iter().enumerate() {
                transaction
                    .execute(
                        "INSERT INTO bot_runtime_attempts
                         (user_id, bot_id, attempt_id, position, attempt_json, updated_at_ms)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                         ON CONFLICT(user_id, bot_id, attempt_id) DO UPDATE SET
                            position=excluded.position, attempt_json=excluded.attempt_json,
                            updated_at_ms=excluded.updated_at_ms",
                        params![
                            user_id,
                            bot_id,
                            attempt.attempt_id,
                            i64::try_from(position).map_err(|error| error.to_string())?,
                            serde_json::to_string(attempt).map_err(|error| error.to_string())?,
                            attempt.updated_at_ms,
                        ],
                    )
                    .map_err(|error| error.to_string())?;
            }
            transaction
                .execute(
                    "UPDATE bots SET attempts_json='[]' WHERE user_id=?1 AND bot_id=?2",
                    params![user_id, bot_id],
                )
                .map_err(|error| error.to_string())?;
        }
        transaction.commit().map_err(|error| error.to_string())
    }

    pub(crate) fn recover_after_restart(&self) -> Result<(), String> {
        let now = adaq_bot_runtime::unix_now_ms();
        let mut database = self.database.lock().map_err(|error| error.to_string())?;
        let rows = {
            let mut statement = database
                .prepare(
                    "SELECT bot_id, user_id, bundle_json, state, current_attempt_id,
                            attempts_json, created_at_ms, updated_at_ms
                     FROM bots",
                )
                .map_err(|error| error.to_string())?;
            statement
                .query_map([], |row| {
                    Ok(PersistedRow {
                        bot_id: row.get(0)?,
                        user_id: row.get(1)?,
                        bundle_json: row.get(2)?,
                        state: row.get(3)?,
                        current_attempt_id: row.get(4)?,
                        attempts_json: row.get(5)?,
                        created_at_ms: row.get(6)?,
                        updated_at_ms: row.get(7)?,
                    })
                })
                .map_err(|error| error.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?
        };
        for row in rows {
            let mut bot = row.decode(&database)?;
            let mut changed = false;
            for attempt in &mut bot.attempts {
                if is_active_state(attempt.state) {
                    let from = attempt.state;
                    attempt.state = LifecycleState::Faulted;
                    attempt.reconciliation_required = true;
                    attempt.events.push(RuntimeEvent {
                        from,
                        to: LifecycleState::Faulted,
                        actor: "host".into(),
                        reason: "host_restart".into(),
                    });
                    push_evidence(
                        attempt,
                        "recovery",
                        "host-restart",
                        "Active Runtime Attempt was interrupted; reconciliation is required.",
                        None,
                        now,
                    );
                    attempt.updated_at_ms = now;
                    changed = true;
                }
            }
            if changed {
                bot.state = LifecycleState::Faulted;
                bot.updated_at_ms = now;
                self.save_record_locked(&mut database, &bot)?;
            }
        }
        Ok(())
    }

    pub(crate) fn deploy(
        &self,
        user_id: &str,
        bundle: BotDeploymentBundle,
    ) -> Result<BotView, String> {
        validate_user(user_id)?;
        bundle.verify()?;
        let now = adaq_bot_runtime::unix_now_ms();
        let record = PersistedBot {
            bot_id: bundle.bot_id.clone(),
            user_id: user_id.into(),
            bundle,
            state: LifecycleState::Stopped,
            current_attempt_id: None,
            attempts: Vec::new(),
            created_at_ms: now,
            updated_at_ms: now,
        };
        let mut database = self.database.lock().map_err(|error| error.to_string())?;
        if database
            .query_row(
                "SELECT 1 FROM bots WHERE bot_id = ?1",
                [&record.bot_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .is_some()
        {
            return Err("Bot identity already exists".into());
        }
        self.insert_record_locked(&mut database, &record)?;
        Ok(record.view())
    }

    pub(crate) fn deploy_many(
        &self,
        user_id: &str,
        bundles: Vec<BotDeploymentBundle>,
    ) -> Result<Vec<BotView>, String> {
        validate_user(user_id)?;
        if bundles.is_empty() || bundles.len() > 3 {
            return Err("An experiment must deploy one to three Bots.".into());
        }
        let now = adaq_bot_runtime::unix_now_ms();
        let records = bundles
            .into_iter()
            .map(|bundle| {
                bundle.verify()?;
                Ok(PersistedBot {
                    bot_id: bundle.bot_id.clone(),
                    user_id: user_id.into(),
                    bundle,
                    state: LifecycleState::Stopped,
                    current_attempt_id: None,
                    attempts: Vec::new(),
                    created_at_ms: now,
                    updated_at_ms: now,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let mut database = self.database.lock().map_err(|error| error.to_string())?;
        let transaction = database.transaction().map_err(|error| error.to_string())?;
        for record in &records {
            if transaction
                .query_row(
                    "SELECT 1 FROM bots WHERE bot_id = ?1",
                    [&record.bot_id],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
                .map_err(|error| error.to_string())?
                .is_some()
            {
                return Err("Bot identity already exists".into());
            }
            self.insert_record_locked(&transaction, record)?;
        }
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(records.into_iter().map(|record| record.view()).collect())
    }

    pub(crate) fn list(&self, user_id: &str) -> Result<Vec<BotView>, String> {
        validate_user(user_id)?;
        let database = self.database.lock().map_err(|error| error.to_string())?;
        let mut statement = database
            .prepare(
                "SELECT bot_id, user_id, bundle_json, state, current_attempt_id,
                        attempts_json, created_at_ms, updated_at_ms
                 FROM bots WHERE user_id = ?1
                 ORDER BY updated_at_ms DESC, bot_id DESC",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([user_id], |row| {
                Ok(PersistedRow {
                    bot_id: row.get(0)?,
                    user_id: row.get(1)?,
                    bundle_json: row.get(2)?,
                    state: row.get(3)?,
                    current_attempt_id: row.get(4)?,
                    attempts_json: row.get(5)?,
                    created_at_ms: row.get(6)?,
                    updated_at_ms: row.get(7)?,
                })
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        drop(statement);
        rows.into_iter()
            .map(|row| row.decode(&database).map(|bot| bot.view()))
            .collect()
    }

    pub(crate) fn account_blocks_new_risk(
        &self,
        user_id: &str,
        account_id: &str,
    ) -> Result<bool, String> {
        Ok(self.list(user_id)?.iter().any(|bot| {
            bot.bundle.account_id == account_id
                && bot.attempts.last().is_some_and(|attempt| {
                    attempt.state == LifecycleState::Faulted || attempt.reconciliation_required
                })
        }))
    }

    pub(crate) fn get(&self, user_id: &str, bot_id: &str) -> Result<BotView, String> {
        validate_user(user_id)?;
        self.load_record(user_id, bot_id).map(|bot| bot.view())
    }

    pub(crate) fn feedback_binding(
        &self,
        user_id: &str,
        bot_id: &str,
        bundle_id: &str,
        attempt_id: &str,
        observation_start_ms: i64,
        observation_end_ms: i64,
        now_ms: i64,
    ) -> Result<(BotView, BotRuntimeAttempt), String> {
        let bot = self.get(user_id, bot_id)?;
        bot.bundle.verify_for_feedback()?;
        if bot.bundle.identity != bundle_id || bot.current_attempt_id.as_deref() != Some(attempt_id)
        {
            return Err(
                "Paper Feedback must reference the current exact Bot Deployment Bundle and Runtime Attempt"
                    .into(),
            );
        }
        let attempt = bot
            .attempts
            .iter()
            .find(|attempt| attempt.attempt_id == attempt_id)
            .cloned()
            .ok_or_else(|| "Bot Runtime Attempt was not found".to_owned())?;
        if attempt.bot_id != bot.bot_id
            || attempt.bundle_identity != bot.bundle.identity
            || observation_start_ms > observation_end_ms
            || observation_start_ms < attempt.created_at_ms
            || observation_end_ms > now_ms
        {
            return Err(
                "Paper Feedback observation range is incompatible with the Runtime Attempt".into(),
            );
        }
        Ok((bot, attempt))
    }

    pub(crate) fn recover_stopped_workers(
        &self,
        supervisor: &crate::bot_supervisor::BotSupervisor,
        operations: &crate::operations::OperationsStore,
        user_id: &str,
        account: Option<&PaperAccountView>,
    ) -> Result<(), String> {
        let _control = self.control.lock().map_err(|error| error.to_string())?;
        let bot_ids = operations
            .alerts_for_user(user_id)?
            .into_iter()
            .filter(|alert| {
                alert.dimension == crate::operations::HealthDimension::Worker
                    && alert.state != crate::operations::AlertState::Resolved
            })
            .map(|alert| alert.entity_id)
            .collect::<BTreeSet<_>>();
        for bot_id in bot_ids {
            let Ok(bot) = self.get(user_id, &bot_id) else {
                continue;
            };
            let Some(attempt) = bot
                .attempts
                .iter()
                .find(|attempt| Some(&attempt.attempt_id) == bot.current_attempt_id.as_ref())
            else {
                continue;
            };
            if bot.state != LifecycleState::Stopped
                || attempt.state != LifecycleState::Stopped
                || attempt.reconciliation_required
                || !reconciliation_resolves_fault_gate(account, &bot.bundle.account_id)
                || account
                    .is_none_or(|account| account.account.observed_at_ms < attempt.updated_at_ms)
                || supervisor.has_worker(&bot_id)?
            {
                continue;
            }
            observe_worker_recovery(
                operations,
                user_id,
                &bot_id,
                &bot.bundle,
                &attempt.attempt_id,
                true,
            )?;
        }
        Ok(())
    }

    pub(crate) fn command(
        &self,
        user_id: &str,
        bot_id: &str,
        command_id: &str,
        command_kind: &str,
        action: impl FnOnce(&Self) -> Result<BotView, String>,
    ) -> Result<BotView, String> {
        validate_user(user_id)?;
        if !bounded(command_id, 128) || !bounded(command_kind, 64) {
            return Err("A bounded command identity is required".into());
        }
        let _control = self
            .control
            .lock()
            .map_err(|error| format!("Bot control lock failed: {error}"))?;
        let prior = {
            let database = self.database.lock().map_err(|error| error.to_string())?;
            database
                .query_row(
                    "SELECT command_kind, result_json FROM bot_commands
                     WHERE user_id = ?1 AND bot_id = ?2 AND command_id = ?3",
                    params![user_id, bot_id, command_id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()
                .map_err(|error| error.to_string())?
        };
        if let Some((kind, result_json)) = prior {
            if kind != command_kind {
                return Err("Command identity was already used for another operation".into());
            }
            if command_kind == "decision"
                && let Ok(result) = serde_json::from_str::<DecisionCommandResult>(&result_json)
            {
                return match result.error {
                    Some(error) => Err(error),
                    None => self.get(user_id, bot_id),
                };
            }
            return serde_json::from_str(&result_json).map_err(|error| error.to_string())?;
        }
        self.load_record(user_id, bot_id)?;
        let result = action(self);
        let result_json = if command_kind == "decision" {
            serde_json::to_string(&DecisionCommandResult {
                error: result.as_ref().err().cloned(),
            })
        } else {
            serde_json::to_string(&result)
        }
        .map_err(|error| error.to_string())?;
        self.database
            .lock()
            .map_err(|error| error.to_string())?
            .execute(
                "INSERT INTO bot_commands
                 (user_id, bot_id, command_id, command_kind, result_json, created_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    user_id,
                    bot_id,
                    command_id,
                    command_kind,
                    result_json,
                    adaq_bot_runtime::unix_now_ms()
                ],
            )
            .map_err(|error| error.to_string())?;
        result
    }

    pub(crate) fn begin_attempt(
        &self,
        user_id: &str,
        bot_id: &str,
        retry: bool,
    ) -> Result<(String, BotDeploymentBundle), String> {
        let mut database = self.database.lock().map_err(|error| error.to_string())?;
        let mut bot = self.load_record_locked(&database, user_id, bot_id)?;
        if retry {
            if bot.state != LifecycleState::Faulted {
                return Err("Retry requires a Faulted Bot".into());
            }
        } else if bot.state != LifecycleState::Stopped {
            return Err("Start requires a stopped Bot; use Retry after recovery".into());
        }
        if bot
            .attempts
            .iter()
            .any(|attempt| is_active_state(attempt.state))
        {
            return Err("Bot already has an active Runtime Attempt".into());
        }
        let attempt_id = Uuid::new_v4().to_string();
        let now = adaq_bot_runtime::unix_now_ms();
        if let BotSchedule::EmaDoubleCross { instrument_id } = &bot.bundle.schedule {
            let instrument_id = okx_instrument_code(instrument_id);
            let lease = database
                .query_row(
                    "SELECT bot_id, user_id FROM bot_instrument_leases
                     WHERE account_id = ?1 AND instrument_id = ?2",
                    params![bot.bundle.account_id, instrument_id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()
                .map_err(|error| error.to_string())?;
            if let Some((lease_bot, lease_user)) = lease
                && (lease_bot != bot_id || lease_user != user_id)
            {
                return Err("The OKX Demo instrument is controlled by another Bot".into());
            }
            database
                .execute(
                    "INSERT OR REPLACE INTO bot_instrument_leases
                     (account_id, instrument_id, bot_id, user_id, attempt_id, acquired_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        bot.bundle.account_id,
                        instrument_id,
                        bot_id,
                        user_id,
                        attempt_id,
                        now
                    ],
                )
                .map_err(|error| error.to_string())?;
        } else {
            let lease = database
                .query_row(
                    "SELECT bot_id, user_id FROM bot_account_leases WHERE account_id = ?1",
                    [&bot.bundle.account_id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()
                .map_err(|error| error.to_string())?;
            if let Some((lease_bot, lease_user)) = lease
                && (lease_bot != bot_id || lease_user != user_id)
            {
                return Err("The OKX Demo account is controlled by another Bot".into());
            }
            database
                .execute(
                    "INSERT OR REPLACE INTO bot_account_leases
                     (account_id, bot_id, user_id, attempt_id, acquired_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![bot.bundle.account_id, bot_id, user_id, attempt_id, now],
                )
                .map_err(|error| error.to_string())?;
        }
        let mut attempt = BotRuntimeAttempt {
            attempt_id: attempt_id.clone(),
            bot_id: bot_id.into(),
            bundle_identity: bot.bundle.identity.clone(),
            state: LifecycleState::Starting,
            stop_policy: None,
            events: Vec::new(),
            evidence: Vec::new(),
            decisions: Vec::new(),
            orders: Vec::new(),
            unmanaged_positions: Vec::new(),
            reconciliation_required: true,
            last_decision_time_ms: None,
            last_event_stream_epoch: None,
            created_at_ms: now,
            updated_at_ms: now,
        };
        push_evidence(
            &mut attempt,
            "lifecycle",
            "start-requested",
            "Host created a new Runtime Attempt from the immutable Bundle.",
            Some(&attempt_id),
            now,
        );
        bot.attempts.push(attempt);
        if bot.attempts.len() > MAX_ATTEMPTS {
            bot.attempts.remove(0);
        }
        bot.state = LifecycleState::Starting;
        bot.current_attempt_id = Some(attempt_id.clone());
        bot.updated_at_ms = now;
        let bundle = bot.bundle.clone();
        self.save_record_locked(&mut database, &bot)?;
        Ok((attempt_id, bundle))
    }

    pub(crate) fn transition(
        &self,
        user_id: &str,
        bot_id: &str,
        to: LifecycleState,
        actor: &str,
        reason: &str,
    ) -> Result<BotView, String> {
        self.mutate(user_id, bot_id, |bot| {
            let attempt = current_attempt_mut(bot)?;
            if !attempt.state.permits_transition_to(to) {
                return Err(format!(
                    "Invalid Bot lifecycle transition {:?} -> {to:?}",
                    attempt.state
                ));
            }
            let from = attempt.state;
            attempt.state = to;
            attempt.events.push(RuntimeEvent {
                from,
                to,
                actor: bounded_text(actor, 128),
                reason: bounded_text(reason, 256),
            });
            attempt.reconciliation_required = to != LifecycleState::Running;
            attempt.updated_at_ms = adaq_bot_runtime::unix_now_ms();
            let updated_at_ms = attempt.updated_at_ms;
            bot.state = to;
            bot.updated_at_ms = updated_at_ms;
            Ok(())
        })
    }

    pub(crate) fn fault(
        &self,
        user_id: &str,
        bot_id: &str,
        code: &str,
        detail: &str,
    ) -> Result<BotView, String> {
        self.fault_for_attempt(user_id, bot_id, code, detail, None)
            .map(|(view, _)| view)
    }

    fn fault_for_attempt(
        &self,
        user_id: &str,
        bot_id: &str,
        code: &str,
        detail: &str,
        identity: Option<&BotAttemptIdentity>,
    ) -> Result<(BotView, bool), String> {
        let mut database = self.database.lock().map_err(|error| error.to_string())?;
        let mut bot = self.load_record_locked(&database, user_id, bot_id)?;
        let previous_updated_at_ms = bot.updated_at_ms;
        {
            let attempt = current_attempt_mut(&mut bot)?;
            if identity.is_some_and(|identity| {
                identity.attempt_id != attempt.attempt_id
                    || identity.bundle_identity != attempt.bundle_identity
            }) {
                return Ok((bot.view(), false));
            }
            let code = safe_code(code);
            let detail = safe_detail(detail);
            if matches!(
                attempt.state,
                LifecycleState::Faulted | LifecycleState::Stopped
            ) && attempt.reconciliation_required
                && attempt.evidence.iter().any(|evidence| {
                    evidence.kind == "recovery"
                        && evidence.code == code
                        && evidence.detail == detail
                        && evidence.related_id.as_deref() == Some(attempt.attempt_id.as_str())
                })
            {
                return Ok((bot.view(), true));
            }
            if attempt.state != LifecycleState::Faulted && attempt.state != LifecycleState::Stopped
            {
                let from = attempt.state;
                attempt.state = LifecycleState::Faulted;
                attempt.events.push(RuntimeEvent {
                    from,
                    to: LifecycleState::Faulted,
                    actor: "host".into(),
                    reason: code.clone(),
                });
            }
            attempt.reconciliation_required = true;
            let now = adaq_bot_runtime::unix_now_ms()
                .max(previous_updated_at_ms.saturating_add(1))
                .max(attempt.updated_at_ms.saturating_add(1));
            let attempt_id = attempt.attempt_id.clone();
            push_evidence(attempt, "recovery", &code, &detail, Some(&attempt_id), now);
            attempt.updated_at_ms = now;
            bot.state = attempt.state;
            bot.updated_at_ms = now;
        }
        self.save_record_locked(&mut database, &bot)?;
        Ok((bot.view(), true))
    }

    pub(crate) fn record_worker_fault(
        &self,
        user_id: &str,
        bot_id: &str,
        code: &str,
        detail: &str,
        identity: &BotAttemptIdentity,
    ) -> Result<bool, String> {
        self.fault_for_attempt(user_id, bot_id, code, detail, Some(identity))
            .map(|(_, applied)| applied)
    }

    pub(crate) fn freeze_all(&self, user_id: &str, detail: &str) -> Result<Vec<String>, String> {
        validate_user(user_id)?;
        let _control = self
            .control
            .lock()
            .map_err(|error| format!("Bot control lock failed: {error}"))?;
        let bots = self.list(user_id)?;
        let mut frozen = Vec::new();
        for bot in bots {
            if bot.state != LifecycleState::Stopped {
                self.fault(user_id, &bot.bot_id, "operations-freeze-all", detail)?;
                frozen.push(bot.bot_id);
            }
        }
        Ok(frozen)
    }

    pub(crate) fn record_evidence(
        &self,
        user_id: &str,
        bot_id: &str,
        kind: &str,
        code: &str,
        detail: &str,
        related_id: Option<&str>,
    ) -> Result<BotView, String> {
        self.mutate(user_id, bot_id, |bot| {
            let attempt = current_attempt_mut(bot)?;
            let now = adaq_bot_runtime::unix_now_ms();
            push_evidence(attempt, kind, code, detail, related_id, now);
            attempt.updated_at_ms = now;
            bot.updated_at_ms = now;
            Ok(())
        })
    }

    pub(crate) fn record_attempt_evidence(
        &self,
        user_id: &str,
        bot_id: &str,
        identity: &BotAttemptIdentity,
        kind: &str,
        code: &str,
        detail: &str,
    ) -> Result<BotView, String> {
        // The general mutator advances the current Attempt's revision. Worker
        // evidence may belong to a retired Attempt and must leave that revision intact.
        let mut database = self.database.lock().map_err(|error| error.to_string())?;
        let mut bot = self.load_record_locked(&database, user_id, bot_id)?;
        let previous_updated_at_ms = bot.updated_at_ms;
        {
            let attempt = bot
                .attempts
                .iter_mut()
                .find(|attempt| {
                    attempt.attempt_id == identity.attempt_id
                        && attempt.bundle_identity == identity.bundle_identity
                })
                .ok_or_else(|| "Worker event Runtime Attempt identity was not found.".to_owned())?;
            let now = adaq_bot_runtime::unix_now_ms()
                .max(previous_updated_at_ms.saturating_add(1))
                .max(attempt.updated_at_ms.saturating_add(1));
            push_evidence(attempt, kind, code, detail, Some(&identity.attempt_id), now);
            attempt.updated_at_ms = now;
            bot.updated_at_ms = now;
        }
        self.save_record_locked(&mut database, &bot)?;
        Ok(bot.view())
    }

    pub(crate) fn record_decision(
        &self,
        user_id: &str,
        bot_id: &str,
        clock: Option<&DecisionClock>,
        result: &WorkerDecisionResult,
        stream_epoch: Option<u32>,
        market_data_universe_snapshot_id: Option<&str>,
    ) -> Result<BotView, String> {
        self.mutate(user_id, bot_id, |bot| {
            let attempt = current_attempt_mut(bot)?;
            let (
                request_id,
                decision_id,
                outcome,
                target_hash,
                target,
                evaluation,
                no_target_reason,
                no_target_detail,
            ) = match result {
                WorkerDecisionResult::Target {
                    request_id,
                    decision_id,
                    target,
                    evaluation,
                    ..
                } => (
                    request_id,
                    decision_id,
                    "target",
                    Some(hash_json(target)?),
                    Some(target.clone()),
                    Some(evaluation.clone()),
                    None,
                    None,
                ),
                WorkerDecisionResult::NoTarget {
                    request_id,
                    decision_id,
                    reason,
                    detail,
                    evaluation,
                } => (
                    request_id,
                    decision_id,
                    "no-target",
                    None,
                    None,
                    evaluation.clone(),
                    Some(reason.clone()),
                    Some(bounded_text(detail, 512)),
                ),
            };
            if evaluation.is_some()
                && let Some(stream_epoch) = stream_epoch
            {
                attempt.last_event_stream_epoch = Some(stream_epoch);
            }
            attempt.decisions.push(BotDecisionEvidence {
                request_id: bounded_text(request_id, 128),
                decision_id: bounded_text(decision_id, 128),
                outcome: outcome.into(),
                no_target_reason,
                no_target_detail,
                target_hash,
                target,
                clock: clock.cloned(),
                evaluation,
                market_data_universe_snapshot_id: market_data_universe_snapshot_id
                    .map(str::to_owned),
                observed_at_ms: adaq_bot_runtime::unix_now_ms(),
            });
            if attempt.decisions.len() > MAX_DECISIONS {
                attempt.decisions.remove(0);
            }
            bot.updated_at_ms = adaq_bot_runtime::unix_now_ms();
            Ok(())
        })
    }

    fn claim_decision(
        &self,
        user_id: &str,
        bot_id: &str,
        attempt_id: &str,
        request_id: &str,
        decision_id: &str,
        decision_time_ms: Option<i64>,
        allow_equal_time: bool,
    ) -> Result<DecisionClaim, String> {
        validate_user(user_id)?;
        if !bounded(attempt_id, 128) || !bounded(request_id, 128) || !bounded(decision_id, 128) {
            return Err("Decision claim identity is missing or exceeds the Host limit.".into());
        }
        let mut database = self.database.lock().map_err(|error| error.to_string())?;
        let bot = self.load_record_locked(&database, user_id, bot_id)?;
        if bot.current_attempt_id.as_deref() != Some(attempt_id) {
            return Err("Decision claim does not belong to the current Runtime Attempt.".into());
        }
        let prior = {
            let mut statement = database
                .prepare(
                    "SELECT request_id, decision_id FROM bot_decision_claims
                     WHERE user_id = ?1 AND bot_id = ?2 AND attempt_id = ?3
                       AND (request_id = ?4 OR decision_id = ?5)",
                )
                .map_err(|error| error.to_string())?;
            statement
                .query_map(
                    params![user_id, bot_id, attempt_id, request_id, decision_id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .map_err(|error| error.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?
        };
        if prior.iter().any(|(prior_request_id, prior_decision_id)| {
            prior_request_id == request_id && prior_decision_id == decision_id
        }) {
            return Ok(DecisionClaim::Duplicate);
        }
        if !prior.is_empty() {
            return Ok(DecisionClaim::Conflict);
        }
        if decision_time_ms.is_some_and(|decision_time_ms| {
            bot.attempts
                .iter()
                .find(|attempt| attempt.attempt_id == attempt_id)
                .and_then(|attempt| attempt.last_decision_time_ms)
                .is_some_and(|last_decision_time_ms| {
                    if allow_equal_time {
                        decision_time_ms < last_decision_time_ms
                    } else {
                        decision_time_ms <= last_decision_time_ms
                    }
                })
        }) {
            return Ok(DecisionClaim::Stale);
        }
        let now = adaq_bot_runtime::unix_now_ms();
        database
            .execute(
                "INSERT INTO bot_decision_claims
                 (user_id, bot_id, attempt_id, request_id, decision_id, created_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![user_id, bot_id, attempt_id, request_id, decision_id, now],
            )
            .map_err(|error| error.to_string())?;
        if let Some(decision_time_ms) = decision_time_ms {
            let mut bot = bot;
            let attempt = current_attempt_mut(&mut bot)?;
            attempt.last_decision_time_ms = Some(decision_time_ms);
            attempt.updated_at_ms = now.max(attempt.updated_at_ms.saturating_add(1));
            bot.updated_at_ms = now;
            self.save_record_locked(&mut database, &bot)?;
        }
        Ok(DecisionClaim::New)
    }

    pub(crate) fn record_order(
        &self,
        user_id: &str,
        bot_id: &str,
        operation_id: &str,
        decision_id: Option<&str>,
        status: &str,
        provider_order_id: Option<&str>,
    ) -> Result<BotView, String> {
        self.mutate(user_id, bot_id, |bot| {
            let attempt = current_attempt_mut(bot)?;
            attempt.orders.push(BotOrderEvidence {
                operation_id: bounded_text(operation_id, 128),
                decision_id: decision_id.map(|value| bounded_text(value, 128)),
                status: bounded_text(status, 64),
                provider_order_id: provider_order_id.map(|value| bounded_text(value, 128)),
                observed_at_ms: adaq_bot_runtime::unix_now_ms(),
            });
            if attempt.orders.len() > MAX_ORDERS {
                attempt.orders.remove(0);
            }
            bot.updated_at_ms = adaq_bot_runtime::unix_now_ms();
            Ok(())
        })
    }

    pub(crate) fn complete_stop(
        &self,
        user_id: &str,
        bot_id: &str,
        policy: BotStopPolicy,
        positions: Vec<String>,
        reconciliation_proven: bool,
    ) -> Result<BotView, String> {
        // A legacy Worker binding cannot run, but a faulted Bot must still be
        // stoppable so Host recovery can persist a safe state and release its lease.
        self.mutate_with_runtime_validation(user_id, bot_id, false, |bot| {
            let attempt = current_attempt_mut(bot)?;
            let now = adaq_bot_runtime::unix_now_ms();
            if is_active_state(attempt.state) && attempt.state != LifecycleState::Stopping {
                let from = attempt.state;
                if !from.permits_transition_to(LifecycleState::Stopping) {
                    return Err("Bot cannot enter Stopping from its current state".into());
                }
                attempt.events.push(RuntimeEvent {
                    from,
                    to: LifecycleState::Stopping,
                    actor: "host".into(),
                    reason: "stop-requested".into(),
                });
                attempt.state = LifecycleState::Stopping;
            }
            attempt.stop_policy = Some(policy);
            attempt.unmanaged_positions = if policy == BotStopPolicy::KeepPosition {
                positions
            } else {
                Vec::new()
            };
            attempt.reconciliation_required = !reconciliation_proven;
            push_evidence(
                attempt,
                "lifecycle",
                match policy {
                    BotStopPolicy::KeepPosition => "stopped-keep-position",
                    BotStopPolicy::Flatten => "stopped-flatten",
                },
                if reconciliation_proven {
                    "Host completed the explicit stop operation with reconciled account evidence."
                } else {
                    "Stop completed without proof of final account state; reconciliation is required."
                },
                Some(&attempt.attempt_id.clone()),
                now,
            );
            if attempt.state != LifecycleState::Stopped {
                let from = attempt.state;
                attempt.events.push(RuntimeEvent {
                    from,
                    to: LifecycleState::Stopped,
                    actor: "host".into(),
                    reason: "stop-complete".into(),
                });
                attempt.state = LifecycleState::Stopped;
            }
            attempt.updated_at_ms = now;
            bot.state = LifecycleState::Stopped;
            bot.updated_at_ms = now;
            Ok(())
        })
        .and_then(|view| {
            if reconciliation_proven
                && (policy == BotStopPolicy::Flatten
                    || view
                        .attempts
                        .last()
                        .is_some_and(|attempt| attempt.unmanaged_positions.is_empty()))
            {
                self.release_lease(user_id, bot_id, &view.bundle.account_id)?;
            }
            Ok(view)
        })
    }

    fn release_lease(&self, user_id: &str, bot_id: &str, account_id: &str) -> Result<(), String> {
        self.database
            .lock()
            .map_err(|error| error.to_string())?
            .execute(
                "DELETE FROM bot_account_leases
                 WHERE account_id = ?1 AND bot_id = ?2 AND user_id = ?3",
                params![account_id, bot_id, user_id],
            )
            .map_err(|error| error.to_string())?;
        self.database
            .lock()
            .map_err(|error| error.to_string())?
            .execute(
                "DELETE FROM bot_instrument_leases
                 WHERE account_id = ?1 AND bot_id = ?2 AND user_id = ?3",
                params![account_id, bot_id, user_id],
            )
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn instrument_lease_exists(
        &self,
        user_id: &str,
        account_id: &str,
        instrument_id: &str,
    ) -> Result<bool, String> {
        self.database
            .lock()
            .map_err(|error| error.to_string())?
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM bot_instrument_leases
                    WHERE user_id = ?1 AND account_id = ?2 AND instrument_id = ?3
                )",
                params![user_id, account_id, instrument_id],
                |row| row.get::<_, i64>(0),
            )
            .map(|exists| exists != 0)
            .map_err(|error| error.to_string())
    }

    fn mutate(
        &self,
        user_id: &str,
        bot_id: &str,
        action: impl FnOnce(&mut PersistedBot) -> Result<(), String>,
    ) -> Result<BotView, String> {
        self.mutate_with_runtime_validation(user_id, bot_id, true, action)
    }

    fn mutate_with_runtime_validation(
        &self,
        user_id: &str,
        bot_id: &str,
        validate_runtime: bool,
        action: impl FnOnce(&mut PersistedBot) -> Result<(), String>,
    ) -> Result<BotView, String> {
        let mut database = self.database.lock().map_err(|error| error.to_string())?;
        let mut bot = self.load_record_locked(&database, user_id, bot_id)?;
        let previous_bot_updated_at_ms = bot.updated_at_ms;
        let current_attempt_id = bot.current_attempt_id.clone();
        let previous_attempt_updated_at_ms = current_attempt_id
            .as_deref()
            .and_then(|attempt_id| {
                bot.attempts
                    .iter()
                    .find(|attempt| attempt.attempt_id == attempt_id)
            })
            .map(|attempt| attempt.updated_at_ms);
        action(&mut bot)?;
        if let Some(attempt) = current_attempt_id.as_deref().and_then(|attempt_id| {
            bot.attempts
                .iter_mut()
                .find(|attempt| attempt.attempt_id == attempt_id)
        }) {
            attempt.updated_at_ms = adaq_bot_runtime::unix_now_ms()
                .max(attempt.updated_at_ms)
                .max(
                    previous_attempt_updated_at_ms
                        .unwrap_or_default()
                        .saturating_add(1),
                );
            bot.updated_at_ms = adaq_bot_runtime::unix_now_ms()
                .max(bot.updated_at_ms)
                .max(previous_bot_updated_at_ms.saturating_add(1))
                .max(attempt.updated_at_ms);
        }
        self.save_record_locked_with_runtime_validation(&mut database, &bot, validate_runtime)?;
        Ok(bot.view())
    }

    fn load_record(&self, user_id: &str, bot_id: &str) -> Result<PersistedBot, String> {
        let database = self.database.lock().map_err(|error| error.to_string())?;
        self.load_record_locked(&database, user_id, bot_id)
    }

    fn load_record_locked(
        &self,
        database: &Connection,
        user_id: &str,
        bot_id: &str,
    ) -> Result<PersistedBot, String> {
        database
            .query_row(
                "SELECT bot_id, user_id, bundle_json, state, current_attempt_id,
                        attempts_json, created_at_ms, updated_at_ms
                 FROM bots WHERE user_id = ?1 AND bot_id = ?2",
                params![user_id, bot_id],
                |row| {
                    Ok(PersistedRow {
                        bot_id: row.get(0)?,
                        user_id: row.get(1)?,
                        bundle_json: row.get(2)?,
                        state: row.get(3)?,
                        current_attempt_id: row.get(4)?,
                        attempts_json: row.get(5)?,
                        created_at_ms: row.get(6)?,
                        updated_at_ms: row.get(7)?,
                    })
                },
            )
            .map_err(|_| "Bot was not found for this User".to_owned())?
            .decode(database)
    }

    fn insert_record_locked(
        &self,
        database: &Connection,
        bot: &PersistedBot,
    ) -> Result<(), String> {
        database
            .execute(
                "INSERT INTO bots
                 (bot_id, user_id, bundle_json, state, current_attempt_id,
                  attempts_json, created_at_ms, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    bot.bot_id,
                    bot.user_id,
                    serde_json::to_string(&bot.bundle).map_err(|error| error.to_string())?,
                    state_json(bot.state)?,
                    bot.current_attempt_id,
                    "[]",
                    bot.created_at_ms,
                    bot.updated_at_ms,
                ],
            )
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn save_record_locked(
        &self,
        database: &mut Connection,
        bot: &PersistedBot,
    ) -> Result<(), String> {
        self.save_record_locked_with_runtime_validation(database, bot, true)
    }

    fn save_record_locked_with_runtime_validation(
        &self,
        database: &mut Connection,
        bot: &PersistedBot,
        validate_runtime: bool,
    ) -> Result<(), String> {
        if validate_runtime {
            bot.bundle.verify()?;
        } else {
            bot.bundle.verify_for_feedback()?;
        }
        if bot.attempts.len() > MAX_ATTEMPTS
            || bot.attempts.iter().any(|attempt| {
                attempt.evidence.len() > MAX_EVIDENCE
                    || attempt.decisions.len() > MAX_DECISIONS
                    || attempt.orders.len() > MAX_ORDERS
            })
        {
            return Err("Bot evidence exceeds the bounded retention limit".into());
        }
        let transaction = database.transaction().map_err(|error| error.to_string())?;
        let existing = {
            let mut statement = transaction
                .prepare(
                    "SELECT attempt_id, position, updated_at_ms FROM bot_runtime_attempts
                     WHERE user_id = ?1 AND bot_id = ?2",
                )
                .map_err(|error| error.to_string())?;
            statement
                .query_map(params![bot.user_id, bot.bot_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        (row.get::<_, i64>(1)?, row.get::<_, i64>(2)?),
                    ))
                })
                .map_err(|error| error.to_string())?
                .collect::<Result<HashMap<_, _>, _>>()
                .map_err(|error| error.to_string())?
        };
        let retained = bot
            .attempts
            .iter()
            .map(|attempt| attempt.attempt_id.as_str())
            .collect::<HashSet<_>>();
        for attempt_id in existing.keys() {
            if !retained.contains(attempt_id.as_str()) {
                transaction
                    .execute(
                        "DELETE FROM bot_runtime_attempts
                         WHERE user_id = ?1 AND bot_id = ?2 AND attempt_id = ?3",
                        params![bot.user_id, bot.bot_id, attempt_id],
                    )
                    .map_err(|error| error.to_string())?;
            }
        }
        for (position, attempt) in bot.attempts.iter().enumerate() {
            let position = i64::try_from(position).map_err(|error| error.to_string())?;
            match existing.get(&attempt.attempt_id) {
                None => {
                    transaction
                        .execute(
                            "INSERT INTO bot_runtime_attempts
                             (user_id, bot_id, attempt_id, position, attempt_json, updated_at_ms)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                            params![
                                bot.user_id,
                                bot.bot_id,
                                attempt.attempt_id,
                                position,
                                serde_json::to_string(attempt).map_err(|error| error.to_string())?,
                                attempt.updated_at_ms,
                            ],
                        )
                        .map_err(|error| error.to_string())?;
                }
                Some((_, previous_updated_at_ms))
                    if *previous_updated_at_ms != attempt.updated_at_ms =>
                {
                    transaction
                        .execute(
                            "UPDATE bot_runtime_attempts SET position = ?1, attempt_json = ?2,
                                updated_at_ms = ?3
                             WHERE user_id = ?4 AND bot_id = ?5 AND attempt_id = ?6",
                            params![
                                position,
                                serde_json::to_string(attempt).map_err(|error| error.to_string())?,
                                attempt.updated_at_ms,
                                bot.user_id,
                                bot.bot_id,
                                attempt.attempt_id,
                            ],
                        )
                        .map_err(|error| error.to_string())?;
                }
                Some((previous_position, _)) if *previous_position != position => {
                    transaction
                        .execute(
                            "UPDATE bot_runtime_attempts SET position = ?1
                             WHERE user_id = ?2 AND bot_id = ?3 AND attempt_id = ?4",
                            params![position, bot.user_id, bot.bot_id, attempt.attempt_id],
                        )
                        .map_err(|error| error.to_string())?;
                }
                Some(_) => {}
            }
        }
        transaction
            .execute(
                "UPDATE bots SET bundle_json = ?1, state = ?2, current_attempt_id = ?3,
                    attempts_json = '[]', updated_at_ms = ?4
                 WHERE user_id = ?5 AND bot_id = ?6",
                params![
                    serde_json::to_string(&bot.bundle).map_err(|error| error.to_string())?,
                    state_json(bot.state)?,
                    bot.current_attempt_id,
                    bot.updated_at_ms,
                    bot.user_id,
                    bot.bot_id,
                ],
            )
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())
    }
}

impl RuntimeGuard for BotStore {
    fn active_dependent_count(&self, user_id: &str, provider: Provider) -> Result<usize, String> {
        if provider != Provider::OkxDemo {
            return Ok(0);
        }
        self.database
            .lock()
            .map_err(|error| format!("database lock: {error}"))?
            .query_row(
                "SELECT
                    (SELECT COUNT(*) FROM bot_account_leases WHERE user_id = ?1)
                    + (SELECT COUNT(*) FROM bot_instrument_leases WHERE user_id = ?1)",
                [user_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| error.to_string())
            .and_then(|count| usize::try_from(count).map_err(|error| error.to_string()))
    }
}

#[derive(Debug)]
struct PersistedRow {
    bot_id: String,
    user_id: String,
    bundle_json: String,
    state: String,
    current_attempt_id: Option<String>,
    attempts_json: String,
    created_at_ms: i64,
    updated_at_ms: i64,
}

impl PersistedRow {
    fn decode(self, database: &Connection) -> Result<PersistedBot, String> {
        let attempts = {
            let mut statement = database
                .prepare(
                    "SELECT attempt_json FROM bot_runtime_attempts
                     WHERE user_id = ?1 AND bot_id = ?2 ORDER BY position ASC",
                )
                .map_err(|error| error.to_string())?;
            statement
                .query_map(params![self.user_id, self.bot_id], |row| {
                    row.get::<_, String>(0)
                })
                .map_err(|error| error.to_string())?
                .map(|json| {
                    json.map_err(|error| error.to_string()).and_then(|json| {
                        serde_json::from_str(&json).map_err(|error| error.to_string())
                    })
                })
                .collect::<Result<Vec<BotRuntimeAttempt>, _>>()?
        };
        Ok(PersistedBot {
            bot_id: self.bot_id,
            user_id: self.user_id,
            bundle: serde_json::from_str(&self.bundle_json).map_err(|error| error.to_string())?,
            state: serde_json::from_str(&self.state).map_err(|error| error.to_string())?,
            current_attempt_id: self.current_attempt_id,
            attempts: if attempts.is_empty() {
                serde_json::from_str(&self.attempts_json).map_err(|error| error.to_string())?
            } else {
                attempts
            },
            created_at_ms: self.created_at_ms,
            updated_at_ms: self.updated_at_ms,
        })
    }
}

fn current_attempt_mut(bot: &mut PersistedBot) -> Result<&mut BotRuntimeAttempt, String> {
    let attempt_id = bot
        .current_attempt_id
        .as_deref()
        .ok_or_else(|| "Bot has no Runtime Attempt".to_owned())?;
    bot.attempts
        .iter_mut()
        .find(|attempt| attempt.attempt_id == attempt_id)
        .ok_or_else(|| "Bot Runtime Attempt was not found".to_owned())
}

fn controls_for(state: LifecycleState) -> BotControlView {
    BotControlView {
        can_start: state == LifecycleState::Stopped,
        can_retry: state == LifecycleState::Faulted,
        can_pause: state == LifecycleState::Running,
        can_resume: state == LifecycleState::Paused,
        can_stop: is_active_state(state) || state == LifecycleState::Faulted,
        can_flatten: is_active_state(state) || state == LifecycleState::Faulted,
    }
}

fn is_active_state(state: LifecycleState) -> bool {
    matches!(
        state,
        LifecycleState::Starting
            | LifecycleState::Reconciling
            | LifecycleState::WarmingUp
            | LifecycleState::Running
            | LifecycleState::Pausing
            | LifecycleState::Paused
            | LifecycleState::Stopping
    )
}

fn runtime_scope(bundle: &DeploymentBundle) -> StrategyScope {
    match bundle.input.strategy.world {
        adaq_bot_runtime::StrategyWorld::Strategy => StrategyScope::SingleInstrument,
        adaq_bot_runtime::StrategyWorld::PortfolioStrategy => StrategyScope::Portfolio,
    }
}

fn push_evidence(
    attempt: &mut BotRuntimeAttempt,
    kind: &str,
    code: &str,
    detail: &str,
    related_id: Option<&str>,
    observed_at_ms: i64,
) {
    attempt.evidence.push(BotEvidence {
        kind: bounded_text(kind, 64),
        code: safe_code(code),
        detail: safe_detail(detail),
        related_id: related_id.map(|value| bounded_text(&safe_detail(value), 128)),
        observed_at_ms,
    });
    if attempt.evidence.len() > MAX_EVIDENCE {
        attempt.evidence.remove(0);
    }
}

fn hash_json<T: Serialize>(value: &T) -> Result<String, String> {
    let bytes = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    Ok(adaq_bot_runtime::sha256_hex(&bytes))
}

fn state_json(state: LifecycleState) -> Result<String, String> {
    serde_json::to_string(&state).map_err(|error| error.to_string())
}

fn bounded(value: &str, max_bytes: usize) -> bool {
    !value.trim().is_empty()
        && value.len() <= max_bytes
        && value.chars().all(|character| !character.is_control())
}

fn bounded_text(value: &str, max_bytes: usize) -> String {
    value.chars().take(max_bytes).collect()
}

fn safe_code(value: &str) -> String {
    value
        .split(|character: char| character.is_whitespace() || character == ':' || character == '/')
        .find(|part| !part.is_empty())
        .map(|part| bounded_text(part, 128))
        .unwrap_or_else(|| "unknown".into())
}

pub(crate) fn safe_detail(value: &str) -> String {
    let clean = value
        .chars()
        .filter(|character| !character.is_control())
        .collect::<String>();
    let lower = clean.to_ascii_lowercase();
    if [
        "/users/",
        "/home/",
        "/private/",
        ".ssh/",
        "c:\\",
        "api_key",
        "apikey",
        "password",
        "passphrase",
        "authorization",
        "bearer ",
        "credential",
        "secret",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
    {
        return "[REDACTED]".into();
    }
    bounded_text(&clean, MAX_TEXT_BYTES)
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn has_duplicates(values: &[String]) -> bool {
    let mut seen = HashSet::new();
    !values.iter().all(|value| seen.insert(value))
}

pub(crate) fn component_kind_matches(package: &ComponentPackage, kind: ComponentKind) -> bool {
    package.manifest.kind == kind
}

pub(crate) fn factor_scope_name(scope: FactorScope) -> adaq_bot_runtime::WorkerFactorScope {
    match scope {
        FactorScope::TimeSeries => adaq_bot_runtime::WorkerFactorScope::TimeSeries,
        FactorScope::CrossSectional => adaq_bot_runtime::WorkerFactorScope::CrossSectional,
    }
}

pub(crate) fn parameter_value(
    definition: &adaq_component_tooling::ParameterDefinition,
    value: &str,
) -> Result<adaq_bot_runtime::WorkerParameterValue, String> {
    match definition.parameter_type {
        ParameterType::Decimal => {
            if !adaq_bot_runtime::is_decimal_text(value) {
                return Err(format!("invalid decimal parameter {}", definition.name));
            }
            Ok(adaq_bot_runtime::WorkerParameterValue::Decimal(
                value.into(),
            ))
        }
        ParameterType::Integer => value
            .parse::<i64>()
            .map(adaq_bot_runtime::WorkerParameterValue::Integer)
            .map_err(|_| format!("invalid integer parameter {}", definition.name)),
        ParameterType::Boolean => value
            .parse::<bool>()
            .map(adaq_bot_runtime::WorkerParameterValue::Boolean)
            .map_err(|_| format!("invalid boolean parameter {}", definition.name)),
        ParameterType::String => Ok(adaq_bot_runtime::WorkerParameterValue::String(
            bounded_text(value, 256),
        )),
    }
}

pub(crate) fn package_parameters(
    package: &ComponentPackage,
    selected: Option<&std::collections::BTreeMap<String, String>>,
) -> Result<Vec<adaq_bot_runtime::WorkerParameterValue>, String> {
    package
        .manifest
        .parameters
        .iter()
        .map(|definition| {
            let value = selected
                .and_then(|parameters| parameters.get(&definition.name))
                .map(String::as_str)
                .unwrap_or(&definition.default_value);
            parameter_value(definition, value)
        })
        .collect()
}

pub(crate) fn strategy_provenance_is_exact(
    package: &ComponentPackage,
    provenance: &StrategyPackageProvenance,
    revision: &StrategyCandidateRevision,
) -> Result<(), String> {
    if !component_kind_matches(package, ComponentKind::Strategy)
        || package.archive_sha256 != provenance.package_archive_sha256
        || package.manifest.wasm_sha256 != provenance.package_wasm_sha256
        || provenance.candidate_id != revision.candidate_id
        || provenance.candidate_revision != revision.revision
        || provenance.candidate_revision_hash != revision.revision_hash
    {
        return Err("Strategy package does not match the exact Qualification".into());
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BotDeployRequest {
    pub qualification_id: String,
    pub profile_id: String,
    pub account_id: String,
    pub schedule: BotSchedule,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BotCommandRequest {
    pub bot_id: String,
    pub command_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BotStopRequest {
    pub bot_id: String,
    pub command_id: String,
    pub policy: BotStopPolicy,
    #[serde(default)]
    pub confirm_flatten: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BotDecisionRequest {
    pub bot_id: String,
    pub command_id: String,
    pub request_id: String,
    pub dataset_id: String,
    #[serde(default)]
    pub trade_id: Option<String>,
}

struct HostDecisionBatch {
    clock: DecisionClock,
    input: WorkerDecisionInput,
    market_data_universe_snapshot_id: Option<String>,
}

struct WorkerArtifactFiles {
    artifact_path: PathBuf,
    signature_path: PathBuf,
    binding: WorkerArtifactBinding,
}

#[tauri::command]
pub(crate) fn bot_list(
    window: WebviewWindow,
    auth: State<'_, crate::auth::AuthState>,
    state: State<'_, Arc<BotStore>>,
) -> Result<Vec<BotView>, String> {
    state.list(&auth.user_id_for_window(window.label())?)
}

#[tauri::command]
pub(crate) fn bot_get(
    request: BotCommandRequest,
    window: WebviewWindow,
    auth: State<'_, crate::auth::AuthState>,
    state: State<'_, Arc<BotStore>>,
) -> Result<BotView, String> {
    state.get(&auth.user_id_for_window(window.label())?, &request.bot_id)
}

#[tauri::command]
pub(crate) async fn bot_deploy(
    request: BotDeployRequest,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    app: AppHandle,
) -> Result<BotView, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    tauri::async_runtime::spawn_blocking(move || {
        let local = app.state::<Arc<LocalResearchState>>();
        let qualifications = app.state::<Arc<StrategyQualificationStore>>();
        let candidates = app.state::<Arc<StrategyCandidateStore>>();
        let bots = app.state::<Arc<BotStore>>();
        let qualification =
            qualifications.qualification_for_user(&user_id, &request.qualification_id)?;
        let profile = local
            .connections
            .list(&user_id)?
            .into_iter()
            .find(|profile| profile.profile_id == request.profile_id)
            .ok_or_else(|| "The selected connection profile was not found.".to_owned())?;
        if profile.provider != Provider::OkxDemo
            || profile.status != ProfileStatus::Usable
            || profile.account_id.as_deref() != Some(request.account_id.as_str())
        {
            return Err(
                "Select one usable, verified OKX Demo profile with the exact account identity."
                    .into(),
            );
        }
        let artifact = resolve_worker_artifact(&app)?;
        let schedule = canonicalize_okx_schedule(request.schedule)?;
        let bundle = if qualification.candidate_id
            == crate::strategy_qualification::EMA_DOUBLE_CROSS_CANDIDATE_ID
        {
            build_ema_bundle(
                &user_id,
                &qualification,
                &request.profile_id,
                &request.account_id,
                schedule,
                artifact.binding,
                &local,
            )?
        } else {
            let (revision, eligible) = candidates.revision_for_user(
                &user_id,
                &qualification.candidate_id,
                qualification.candidate_revision,
            )?;
            build_bundle(
                &user_id,
                &qualification,
                &revision,
                eligible,
                &request.profile_id,
                &request.account_id,
                schedule,
                artifact.binding,
                &local,
            )?
        };
        bots.deploy(&user_id, bundle)
    })
    .await
    .map_err(|error| error.to_string())?
}

pub(crate) fn deploy_ema_experiment(
    app: &AppHandle,
    user_id: &str,
    profile_id: &str,
    account_id: &str,
    bindings: &[(String, String)],
) -> Result<Vec<BotView>, String> {
    let local = app.state::<Arc<LocalResearchState>>();
    let qualifications = app.state::<Arc<StrategyQualificationStore>>();
    let bots = app.state::<Arc<BotStore>>();
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
    let artifact = resolve_worker_artifact(app)?;
    let bundles = bindings
        .iter()
        .map(|(instrument, qualification_id)| {
            let qualification = qualifications.qualification_for_user(user_id, qualification_id)?;
            build_ema_bundle(
                user_id,
                &qualification,
                profile_id,
                account_id,
                BotSchedule::EmaDoubleCross {
                    instrument_id: format!("okx:{instrument}"),
                },
                artifact.binding.clone(),
                &local,
            )
        })
        .collect::<Result<Vec<_>, String>>()?;
    bots.deploy_many(user_id, bundles)
}

pub(crate) fn start_experiment_bot(
    app: &AppHandle,
    user_id: &str,
    bot_id: &str,
    command_id: &str,
) -> Result<BotView, String> {
    start_bot(
        app,
        user_id,
        &BotCommandRequest {
            bot_id: bot_id.to_owned(),
            command_id: command_id.to_owned(),
        },
        false,
    )
}

pub(crate) fn stop_experiment_bot(
    app: &AppHandle,
    user_id: &str,
    bot_id: &str,
    command_id: &str,
) -> Result<BotView, String> {
    let bots = app.state::<Arc<BotStore>>();
    let view = bots.get(user_id, bot_id)?;
    if view.state == LifecycleState::Stopped {
        // Preparing can be canceled before every deployed Bot has started.
        return Ok(view);
    }
    stop_bot(
        app,
        user_id,
        BotStopRequest {
            bot_id: bot_id.to_owned(),
            command_id: command_id.to_owned(),
            policy: BotStopPolicy::KeepPosition,
            confirm_flatten: false,
        },
        false,
    )
}

async fn complete_lifecycle_command(
    app: AppHandle,
    user_id: String,
    action: impl FnOnce(&AppHandle, &str) -> Result<BotView, String> + Send + 'static,
) -> Result<BotView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let view = action(&app, &user_id)?;
        crate::refresh_bot_trade_stream(&app, &user_id)?;
        Ok(view)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn bot_start(
    request: BotCommandRequest,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    app: AppHandle,
) -> Result<BotView, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    complete_lifecycle_command(app, user_id, move |app, user_id| {
        start_bot(app, user_id, &request, false)
    })
    .await
}

#[tauri::command]
pub(crate) async fn bot_retry(
    request: BotCommandRequest,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    app: AppHandle,
) -> Result<BotView, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    complete_lifecycle_command(app, user_id, move |app, user_id| {
        start_bot(app, user_id, &request, true)
    })
    .await
}

#[tauri::command]
pub(crate) async fn bot_pause(
    request: BotCommandRequest,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    app: AppHandle,
) -> Result<BotView, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    complete_lifecycle_command(app, user_id, move |app, user_id| {
        pause_bot(app, user_id, &request)
    })
    .await
}

#[tauri::command]
pub(crate) async fn bot_resume(
    request: BotCommandRequest,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    app: AppHandle,
) -> Result<BotView, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    complete_lifecycle_command(app, user_id, move |app, user_id| {
        resume_bot(app, user_id, &request)
    })
    .await
}

#[tauri::command]
pub(crate) async fn bot_stop(
    request: BotStopRequest,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    app: AppHandle,
) -> Result<BotView, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    complete_lifecycle_command(app, user_id, move |app, user_id| {
        stop_bot(app, user_id, request, true)
    })
    .await
}

// The Webview may request a Host decision, but it cannot provide feature
// values, Portfolio State, or a Target.
#[tauri::command]
pub(crate) async fn bot_decision(
    request: BotDecisionRequest,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    app: AppHandle,
) -> Result<BotView, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    tauri::async_runtime::spawn_blocking(move || {
        let ctx = BotContext::from_app(&app);
        run_bot_decision(&ctx, &user_id, request)
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Injected decision-pipeline context: everything `run_bot_decision` and
/// `dispatch_trade_event` need from the Host, without an `AppHandle`. The
/// narrow Bot-operation seam over the `LocalResearchState` god-struct.
pub(crate) struct BotContext {
    pub(crate) local: Arc<LocalResearchState>,
    pub(crate) bots: Arc<BotStore>,
    pub(crate) supervisor: Arc<crate::bot_supervisor::BotSupervisor>,
}

impl BotContext {
    pub(crate) fn from_app(app: &AppHandle) -> Self {
        Self {
            local: app.state::<Arc<LocalResearchState>>().inner().clone(),
            bots: app.state::<Arc<BotStore>>().inner().clone(),
            supervisor: app
                .state::<Arc<crate::bot_supervisor::BotSupervisor>>()
                .inner()
                .clone(),
        }
    }
}

fn run_bot_decision(
    ctx: &BotContext,
    user_id: &str,
    request: BotDecisionRequest,
) -> Result<BotView, String> {
    let local = &ctx.local;
    let bots = &ctx.bots;
    let supervisor = &ctx.supervisor;
    bots.command(
            &user_id,
            &request.bot_id,
            &request.command_id,
            "decision",
            |bots| {
                let view = bots.get(&user_id, &request.bot_id)?;
                if view.state != LifecycleState::Running {
                    return Err("Bot must be Running before it can accept a Decision Batch.".into());
                }
                let gate_ctx = crate::new_risk_gate::NewRiskContext {
                    user_id: &user_id,
                    bot_id: &request.bot_id,
                    account_id: &view.bundle.account_id,
                    now_ms: adaq_bot_runtime::unix_now_ms(),
                };
                if let crate::new_risk_gate::NewRiskOutcome::Blocked(block) =
                    crate::new_risk_gate::decision_blocked(&gate_ctx, &local.paper_experiments)?
                {
                    bots.record_evidence(
                        &user_id,
                        &request.bot_id,
                        "experiment",
                        "experiment-observation-blocked",
                        &block.message,
                        Some(&request.request_id),
                    )?;
                    return bots.get(&user_id, &request.bot_id);
                }
                let validation: Result<(), String> = if !bounded(&request.request_id, 128)
                    || !bounded(&request.dataset_id, 128)
                    || request
                        .trade_id
                        .as_deref()
                        .is_some_and(|trade_id| !bounded(trade_id, 256))
                {
                    Err("Decision request identity is missing or exceeds the Host limit.".into())
                } else {
                    Ok(())
                };
                if let Err(error) = validation {
                    bots.record_evidence(
                        &user_id,
                        &request.bot_id,
                        "decision",
                        "decision-input-rejected",
                        &error,
                        Some(&request.request_id),
                    )?;
                    return bots.get(&user_id, &request.bot_id);
                }
                let attempt_id = view
                    .current_attempt_id
                    .as_deref()
                    .ok_or_else(|| "Bot has no active Runtime Attempt.".to_owned())?;
                let previous_event_cursor = view
                    .attempts
                    .iter()
                    .find(|attempt| attempt.attempt_id == attempt_id)
                    .and_then(|attempt| {
                        let stream_epoch = attempt.last_event_stream_epoch?;
                        attempt.decisions.iter().rev().find_map(|decision| {
                            if decision.evaluation.is_none() {
                                return None;
                            }
                            match decision.clock.as_ref() {
                                Some(DecisionClock::TradeEvent {
                                    observation_time_ms,
                                    ..
                                }) => Some((*observation_time_ms, stream_epoch)),
                                _ => None,
                            }
                        })
                    });
                let host_batch = host_decision_batch(
                    &local,
                    &user_id,
                    &view.bundle,
                    &request.dataset_id,
                    request.trade_id.as_deref(),
                    previous_event_cursor,
                );
                if let Err(error) = &host_batch {
                    if error == "The retained OKX Trade is stale." {
                        bots.record_evidence(
                            &user_id,
                            &request.bot_id,
                            "market-data",
                            "stale-trade-skipped",
                            "A replayed Trade arrived after the decision deadline; no Worker or order work was authorized.",
                            Some(&request.request_id),
                        )?;
                    } else {
                        local.operations.observe(crate::operations::HealthObservation {
                        user_id: user_id.into(),
                        entity_id: view.bundle.market_data_snapshot_id.clone(),
                        dimension: crate::operations::HealthDimension::MarketData,
                        state: crate::operations::HealthState::Unknown,
                        condition: "market_data_context".into(),
                        evidence: serde_json::json!({
                            "botId": request.bot_id,
                            "bundleId": view.bundle.identity,
                            "datasetId": request.dataset_id,
                            "snapshotId": view.bundle.market_data_snapshot_id,
                            "error": safe_detail(error),
                        }),
                        required: true,
                        observed_at_ms: adaq_bot_runtime::unix_now_ms(),
                        event_kind: Some("market.data-health".into()),
                        evidence_id: Some(request.dataset_id.clone()),
                        correlation_id: Some(request.request_id.clone()),
                        causation_id: Some(view.bundle.identity.clone()),
                        diagnostic: Some(safe_detail(error)),
                        metrics: BTreeMap::new(),
                        })?;
                    }
                } else {
                    local.operations.observe(crate::operations::HealthObservation {
                        user_id: user_id.into(),
                        entity_id: view.bundle.market_data_snapshot_id.clone(),
                        dimension: crate::operations::HealthDimension::MarketData,
                        state: crate::operations::HealthState::Healthy,
                        condition: "market_data_context".into(),
                        evidence: serde_json::json!({
                            "botId": request.bot_id,
                            "bundleId": view.bundle.identity,
                            "datasetId": request.dataset_id,
                            "snapshotId": view.bundle.market_data_snapshot_id,
                        }),
                        required: true,
                        observed_at_ms: adaq_bot_runtime::unix_now_ms(),
                        event_kind: Some("market.data-health".into()),
                        evidence_id: Some(request.dataset_id.clone()),
                        correlation_id: Some(request.request_id.clone()),
                        causation_id: Some(view.bundle.identity.clone()),
                        diagnostic: Some("Host assembled the exact frozen Market Data context.".into()),
                        metrics: BTreeMap::new(),
                    })?;
                }
                if host_batch.is_ok() {
                    let gate_ctx = crate::new_risk_gate::NewRiskContext {
                        user_id: &user_id,
                        bot_id: &request.bot_id,
                        account_id: &view.bundle.account_id,
                        now_ms: adaq_bot_runtime::unix_now_ms(),
                    };
                    if let crate::new_risk_gate::NewRiskOutcome::Blocked(block) =
                        crate::new_risk_gate::new_risk_blocked(
                            &gate_ctx,
                            &local.operations,
                            bots,
                            &local.paper_trading,
                            &local.paper_experiments,
                            crate::new_risk_gate::Phase::Decision,
                        )?
                    {
                        bots.record_evidence(
                            &user_id,
                            &request.bot_id,
                            "decision",
                            "decision-skipped-by-operations",
                            &block.message,
                            Some(&request.request_id),
                        )?;
                        return bots.get(&user_id, &request.bot_id);
                    }
                }
                let decision_id = match &host_batch {
                    Ok(batch) => batch.clock.decision_id().to_owned(),
                    Err(error) => unavailable_decision_id(&view.bundle, &request.request_id, error)?,
                };
                let claim = bots.claim_decision(
                    &user_id,
                    &request.bot_id,
                    attempt_id,
                    &request.request_id,
                    &decision_id,
                    host_batch
                        .as_ref()
                        .ok()
                        .map(|batch| batch.clock.decision_time_ms()),
                    host_batch.as_ref().is_ok_and(|batch| {
                        matches!(batch.clock, DecisionClock::TradeEvent { .. })
                    }),
                )?;
                match claim {
                    DecisionClaim::New => {}
                    DecisionClaim::Duplicate => {
                        bots.record_evidence(
                            &user_id,
                            &request.bot_id,
                            "decision",
                            "duplicate-decision",
                            "The Decision identity was already processed; no Worker or order work was repeated.",
                            Some(&decision_id),
                        )?;
                        return bots.get(&user_id, &request.bot_id);
                    }
                    DecisionClaim::Conflict => {
                        bots.record_evidence(
                            &user_id,
                            &request.bot_id,
                            "decision",
                            "conflicting-decision",
                            "A request reused an existing Decision or request identity; no risk work was started.",
                            Some(&decision_id),
                        )?;
                        return bots.get(&user_id, &request.bot_id);
                    }
                    DecisionClaim::Stale => {
                        bots.record_evidence(
                            &user_id,
                            &request.bot_id,
                            "decision",
                            "stale-decision",
                            "The Host schedule cursor has already advanced past this Decision Batch; no risk work was started.",
                            Some(&decision_id),
                        )?;
                        return bots.get(&user_id, &request.bot_id);
                    }
                }
                let HostDecisionBatch { clock, input: host_input, market_data_universe_snapshot_id } = match host_batch {
                    Ok(batch) => batch,
                    Err(error) => {
                        let result = WorkerDecisionResult::NoTarget {
                            request_id: request.request_id.clone(),
                            decision_id: decision_id.clone(),
                            reason: adaq_bot_runtime::NoTargetReason::MissingInput,
                            detail: safe_detail(&error),
                            evaluation: None,
                        };
                        bots.record_decision(&user_id, &request.bot_id, None, &result, None, None)?;
                        bots.record_evidence(
                            &user_id,
                            &request.bot_id,
                            "decision",
                            "decision-batch-unavailable",
                            &error,
                            Some(&decision_id),
                        )?;
                        return bots.get(&user_id, &request.bot_id);
                    }
                };
                if let Err(error) = validate_decision_input(&view.bundle, &clock, &host_input) {
                    let result = WorkerDecisionResult::NoTarget {
                        request_id: request.request_id.clone(),
                        decision_id: decision_id.clone(),
                        reason: adaq_bot_runtime::NoTargetReason::MissingInput,
                        detail: safe_detail(&error),
                        evaluation: None,
                    };
                    bots.record_decision(&user_id, &request.bot_id, Some(&clock), &result, None, market_data_universe_snapshot_id.as_deref())?;
                    bots.record_evidence(
                        &user_id,
                        &request.bot_id,
                        "decision",
                        "decision-input-rejected",
                        &error,
                        Some(&request.request_id),
                    )?;
                    return bots.get(&user_id, &request.bot_id);
                }
                let worker_input = match authoritative_decision_input(
                    &local,
                    &user_id,
                    &view.bundle,
                    &host_input,
                ) {
                    Ok(input) => input,
                    Err(error) => {
                        let result = WorkerDecisionResult::NoTarget {
                            request_id: request.request_id.clone(),
                            decision_id: decision_id.clone(),
                            reason: adaq_bot_runtime::NoTargetReason::MissingInput,
                            detail: safe_detail(&error),
                            evaluation: None,
                        };
                        bots.record_decision(&user_id, &request.bot_id, Some(&clock), &result, None, market_data_universe_snapshot_id.as_deref())?;
                        bots.record_evidence(
                            &user_id,
                            &request.bot_id,
                            "reconciliation",
                            "portfolio-state-unavailable",
                            &error,
                            Some(&decision_id),
                        )?;
                        return bots.get(&user_id, &request.bot_id);
                    }
                };
                let worker_stream_epoch = match &worker_input {
                    WorkerDecisionInput::Event { stream_epoch, .. } => Some(*stream_epoch),
                    _ => None,
                };
                let result = supervisor.decision(
                    &user_id,
                    &request.bot_id,
                    &request.bot_id,
                    request.request_id.clone(),
                    clock.clone(),
                    worker_input,
                );
                match result {
                Ok(ref result @ WorkerDecisionResult::Target {
                    request_id: _,
                    ref target,
                    ref decision_id,
                    ref produced_at_ms,
                    ref evaluation,
                }) => {
                        if let Err(error) = clock.accepts_target(decision_id, *produced_at_ms)
                        {
                            let _ = supervisor.fail_decision(
                                &user_id,
                                &request.bot_id,
                                "target-identity-invalid",
                                &error.to_string(),
                            );
                            return Err(
                                "Worker Target failed Host freshness validation; the Bot is Faulted and requires recovery."
                                    .into(),
                            );
                        }
                        bots.record_decision(
                            &user_id,
                            &request.bot_id,
                            Some(&clock),
                            &result,
                            worker_stream_epoch,
                            market_data_universe_snapshot_id.as_deref(),
                        )?;
                        bots.record_evidence(
                            &user_id,
                            &request.bot_id,
                            "lifecycle",
                            "warmup-complete",
                            "The Worker produced its first Target after the frozen warmup policy; Host validation still gates execution.",
                            Some(decision_id),
                        )?;
                    local.paper_experiments.arm_if_all_bots_warmed(
                        &user_id,
                        &request.bot_id,
                        &bots.list(&user_id)?,
                        adaq_bot_runtime::unix_now_ms(),
                    )?;
                    // Seam: the Strategy Target becomes an observable, persistable step
                    // before execution. The decide → execute carry is now a typed object
                    // rather than two loose arguments.
                    let approved = ApprovedTarget {
                        target: target.clone(),
                        decision_id: decision_id.clone(),
                        produced_at_ms: *produced_at_ms,
                        evaluation: evaluation.clone(),
                    };
                    bots.record_evidence(
                        &user_id,
                        &request.bot_id,
                        "decision",
                        "decision-strategy-target",
                        &serde_json::to_string(&approved).map_err(|error| error.to_string())?,
                        Some(decision_id),
                    )?;
                    if let Err(error) = execute_target(
                        &local,
                        bots,
                        &user_id,
                        &request.bot_id,
                        &view.bundle,
                        &clock,
                        &approved,
                    ) {
                            let _ = supervisor.stop(
                                &user_id,
                                &request.bot_id,
                                &request.bot_id,
                                &request.command_id,
                            );
                            let _ = supervisor.fail_active(
                                &user_id,
                                &request.bot_id,
                                "target-execution-failed",
                                &error,
                            );
                            return Err(
                                "Target execution failed; the Bot is Faulted and requires recovery."
                                    .into(),
                            );
                        }
                        bots.get(&user_id, &request.bot_id)
                    }
                    Ok(ref result @ WorkerDecisionResult::NoTarget {
                        reason: adaq_bot_runtime::NoTargetReason::NoSignal,
                        ..
                    }) => {
                        bots.record_decision(
                            &user_id,
                            &request.bot_id,
                            Some(&clock),
                            result,
                            worker_stream_epoch,
                            market_data_universe_snapshot_id.as_deref(),
                        )?;
                        bots.record_evidence(
                            &user_id,
                            &request.bot_id,
                            "lifecycle",
                            "warmup-complete",
                            "The Worker completed its frozen warmup policy with no actionable signal; common experiment readiness still gates risk.",
                            Some(clock.decision_id()),
                        )?;
                        local.paper_experiments.arm_if_all_bots_warmed(
                            &user_id,
                            &request.bot_id,
                            &bots.list(&user_id)?,
                            adaq_bot_runtime::unix_now_ms(),
                        )?;
                        bots.get(&user_id, &request.bot_id)
                    }
                    Ok(ref result @ WorkerDecisionResult::NoTarget {
                        reason: adaq_bot_runtime::NoTargetReason::DeadlineMissed,
                        ..
                    }) => {
                        bots.record_decision(
                            &user_id,
                            &request.bot_id,
                            Some(&clock),
                            result,
                            worker_stream_epoch,
                            market_data_universe_snapshot_id.as_deref(),
                        )?;
                        let _ = supervisor.fail_active(
                            &user_id,
                            &request.bot_id,
                            "decision-deadline-missed",
                            "The Worker missed the Decision Batch deadline; recovery is required.",
                        );
                        Err(
                            "Worker missed the Decision deadline; the Bot is Faulted and requires recovery."
                                .into(),
                        )
                    }
                    Ok(ref result @ WorkerDecisionResult::NoTarget {
                        reason: adaq_bot_runtime::NoTargetReason::Warmup,
                        ..
                    }) => {
                        bots.record_decision(
                            &user_id,
                            &request.bot_id,
                            Some(&clock),
                            result,
                            worker_stream_epoch,
                            market_data_universe_snapshot_id.as_deref(),
                        )?;
                        bots.record_evidence(
                            &user_id,
                            &request.bot_id,
                            "lifecycle",
                            "warmup-progress",
                            "The Worker consumed the Decision Batch for warmup; no Target or order was authorized.",
                            Some(clock.decision_id()),
                        )
                    }
                    Ok(result) => bots.record_decision(
                        &user_id,
                        &request.bot_id,
                        Some(&clock),
                        &result,
                        worker_stream_epoch,
                        market_data_universe_snapshot_id.as_deref(),
                    ),
                    Err(error) => {
                        let _ = supervisor.fail_active(
                            &user_id,
                            &request.bot_id,
                            "worker-decision-failed",
                            &error,
                        );
                        Err(
                            "Worker Decision failed; the Bot is Faulted and requires recovery."
                                .into(),
                        )
                    }
                }
            },
        )
}

pub(crate) fn dispatch_closed_bar_tick(ctx: &BotContext, user_id: &str) -> Result<(), String> {
    validate_user(user_id)?;
    let mut first_error = None;
    for view in ctx.bots.list(user_id)? {
        if view.state != LifecycleState::Running {
            continue;
        }
        let interval = match &view.bundle.schedule {
            BotSchedule::ClosedBar { .. } => bundle_interval(&view.bundle)?,
            BotSchedule::ScheduledCrossSection { .. } => {
                ctx.local
                    .snapshots
                    .universe_snapshot_for_user(user_id, &view.bundle.universe_snapshot_id)?
                    .interval
            }
            BotSchedule::EmaDoubleCross { .. } => continue,
        };
        if let Some(request) =
            closed_bar_tick_request(&view, interval, adaq_bot_runtime::unix_now_ms())?
        {
            if let Err(error) = run_bot_decision(ctx, user_id, request) {
                first_error.get_or_insert(error);
            }
        }
    }
    first_error.map_or(Ok(()), Err)
}

fn closed_bar_tick_request(
    view: &BotView,
    interval: adaq_data_core::BarInterval,
    now_ms: i64,
) -> Result<Option<BotDecisionRequest>, String> {
    if view.state != LifecycleState::Running
        || matches!(view.bundle.schedule, BotSchedule::EmaDoubleCross { .. })
    {
        return Ok(None);
    }
    let time = adaq_data_pipeline::okx::latest_closed_bar_boundary_ms(now_ms, interval)
        .map_err(|error| error.to_string())?;
    if host_schedule_window(time, now_ms).is_err()
        || view
            .attempts
            .iter()
            .find(|attempt| Some(&attempt.attempt_id) == view.current_attempt_id.as_ref())
            .and_then(|attempt| attempt.last_decision_time_ms)
            .is_some_and(|last| last >= time)
    {
        return Ok(None);
    }
    let identity = hash_json(&(
        view.bot_id.as_str(),
        view.current_attempt_id.as_deref(),
        time.to_string(),
    ))?;
    Ok(Some(BotDecisionRequest {
        bot_id: view.bot_id.clone(),
        command_id: format!("stream-decision-{identity}"),
        request_id: format!("stream-request-{identity}"),
        dataset_id: format!("closed-bar:{time}"),
        trade_id: None,
    }))
}

pub(crate) fn dispatch_trade_event(
    ctx: &BotContext,
    user_id: &str,
    instrument_code: &str,
    trade_id: &str,
) -> Result<(), String> {
    validate_user(user_id)?;
    if !bounded(instrument_code, 128) || !bounded(trade_id, 256) {
        return Err("Trade event identity exceeds the Host limit.".into());
    }
    let bots = &ctx.bots;
    let views = bots.list(user_id)?;
    let mut first_error = None;
    for view in views {
        if view.state != LifecycleState::Running {
            continue;
        }
        let (dataset_id, event_id, retained_trade_id) = match &view.bundle.schedule {
            BotSchedule::EmaDoubleCross { instrument_id }
                if okx_instrument_code(instrument_id) == instrument_code =>
            {
                (
                    instrument_code.to_owned(),
                    trade_id.to_owned(),
                    Some(trade_id.to_owned()),
                )
            }
            BotSchedule::ClosedBar { instrument_id, .. }
                if okx_instrument_code(instrument_id) == instrument_code =>
            {
                let time = adaq_data_pipeline::okx::latest_closed_bar_boundary_ms(
                    adaq_bot_runtime::unix_now_ms(),
                    bundle_interval(&view.bundle)?,
                )
                .map_err(|error| error.to_string())?;
                (format!("closed-bar:{time}"), time.to_string(), None)
            }
            BotSchedule::ScheduledCrossSection { instruments, .. }
                if instruments.first().is_some_and(|instrument| {
                    okx_instrument_code(instrument) == instrument_code
                }) =>
            {
                let frozen = ctx
                    .local
                    .snapshots
                    .universe_snapshot_for_user(user_id, &view.bundle.universe_snapshot_id)?;
                let time = adaq_data_pipeline::okx::latest_closed_bar_boundary_ms(
                    adaq_bot_runtime::unix_now_ms(),
                    frozen.interval,
                )
                .map_err(|error| error.to_string())?;
                (format!("closed-bar:{time}"), time.to_string(), None)
            }
            _ => continue,
        };
        if retained_trade_id.is_none() {
            let time = event_id.parse::<i64>().map_err(|error| error.to_string())?;
            if host_schedule_window(time, adaq_bot_runtime::unix_now_ms()).is_err()
                || view
                    .attempts
                    .iter()
                    .find(|attempt| Some(&attempt.attempt_id) == view.current_attempt_id.as_ref())
                    .and_then(|attempt| attempt.last_decision_time_ms)
                    .is_some_and(|last| last >= time)
            {
                continue;
            }
        }
        let identity = hash_json(&(
            view.bot_id.as_str(),
            view.current_attempt_id.as_deref(),
            event_id,
        ))?;
        let request = BotDecisionRequest {
            bot_id: view.bot_id,
            command_id: format!("stream-decision-{identity}"),
            request_id: format!("stream-request-{identity}"),
            dataset_id,
            trade_id: retained_trade_id,
        };
        if let Err(error) = run_bot_decision(ctx, user_id, request) {
            first_error.get_or_insert(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

fn host_decision_batch(
    local: &LocalResearchState,
    user_id: &str,
    bundle: &BotDeploymentBundle,
    dataset_id: &str,
    trade_id: Option<&str>,
    previous_event_cursor: Option<(i64, u32)>,
) -> Result<HostDecisionBatch, String> {
    if let Some(time) = dataset_id.strip_prefix("closed-bar:") {
        let decision_time_ms = time
            .parse::<i64>()
            .map_err(|_| "Closed-bar observation identity is invalid".to_owned())?;
        return host_live_closed_bar_batch(local, user_id, bundle, decision_time_ms);
    }
    let clock = host_schedule_clock(local, user_id, bundle, dataset_id, trade_id)?;
    let input = host_decision_input(
        local,
        user_id,
        bundle,
        dataset_id,
        trade_id,
        &clock,
        previous_event_cursor,
    )?;
    Ok(HostDecisionBatch {
        clock,
        input,
        market_data_universe_snapshot_id: None,
    })
}

fn host_live_closed_bar_batch(
    local: &LocalResearchState,
    user_id: &str,
    bundle: &BotDeploymentBundle,
    decision_time_ms: i64,
) -> Result<HostDecisionBatch, String> {
    bundle.verify()?;
    let frozen = local
        .snapshots
        .universe_snapshot_for_user(user_id, &bundle.universe_snapshot_id)?;
    let (instruments, interval) = match &bundle.schedule {
        BotSchedule::ClosedBar { instrument_id, .. } => {
            (vec![instrument_id.clone()], bundle_interval(bundle)?)
        }
        BotSchedule::ScheduledCrossSection { instruments, .. } => {
            (instruments.clone(), frozen.interval)
        }
        BotSchedule::EmaDoubleCross { .. } => {
            return Err("Trade Event Bots do not accept closed-bar observations".into());
        }
    };
    let expected = frozen
        .components
        .iter()
        .map(|component| {
            format!(
                "{}:{}",
                component.dataset.instrument.venue.id, component.dataset.instrument.code
            )
        })
        .collect::<BTreeSet<_>>();
    if interval != frozen.interval
        || !instruments
            .iter()
            .all(|instrument| expected.contains(instrument))
        || matches!(bundle.schedule, BotSchedule::ScheduledCrossSection { .. })
            && expected != instruments.iter().cloned().collect()
    {
        return Err("Runtime observations do not match the frozen Bot Universe".into());
    }
    let now = adaq_bot_runtime::unix_now_ms();
    if adaq_data_pipeline::okx::latest_closed_bar_boundary_ms(now, interval)
        .map_err(|error| error.to_string())?
        != decision_time_ms
    {
        return Err("Runtime observations must use the latest closed-bar boundary".into());
    }
    let (deadline_ms, _) = host_schedule_window(decision_time_ms, now)?;
    let dataset = local
        .features
        .list_datasets(crate::features::FeatureUserRequest {
            user_id: user_id.into(),
        })?
        .into_iter()
        .find(|dataset| {
            is_exact_feature_context(
                bundle,
                &dataset.manifest.request.feature_plan_hash,
                &dataset.manifest.request.snapshot_id,
                &dataset.manifest.request.point_in_time_universe_id,
            )
        })
        .ok_or_else(|| "The exact qualified Bot Feature Plan is unavailable".to_owned())?;
    let plan = adaq_feature_engine::FeaturePlan::load_for_engine(
        &serde_json::to_vec(&dataset.manifest.plan_json).map_err(|error| error.to_string())?,
        &dataset.manifest.engine_identity,
    )
    .map_err(|error| error.to_string())?;
    let required_bars = plan
        .effective_warmup_bars()
        .checked_add(1)
        .ok_or_else(|| "Runtime Feature warmup exceeds the Host limit".to_owned())?;
    if u64::from(required_bars)
        > bundle
            .runtime_bundle
            .input
            .worker_policy
            .max_decision_frames
    {
        return Err("Runtime Feature warmup exceeds the frozen Worker frame limit".into());
    }
    let required_bars = if matches!(bundle.schedule, BotSchedule::ClosedBar { .. }) {
        u32::try_from(
            bundle
                .runtime_bundle
                .input
                .worker_policy
                .max_decision_frames,
        )
        .map_err(|_| "Worker frame limit exceeds the Host allocation limit".to_owned())?
    } else {
        required_bars
    };
    let mut start_time_ms = decision_time_ms;
    for _ in 0..required_bars {
        start_time_ms =
            adaq_data_pipeline::okx::latest_closed_bar_boundary_ms(start_time_ms - 1, interval)
                .map_err(|error| error.to_string())?;
    }
    let read_deadline_ms = deadline_ms.saturating_sub(
        i64::try_from(
            bundle
                .runtime_bundle
                .input
                .worker_policy
                .decision_timeout_ms,
        )
        .map_err(|_| "Worker decision timeout exceeds the Host time limit".to_owned())?,
    );
    let observation = acquire_closed_bar_with_retry(
        read_deadline_ms,
        || {
            local.acquire_runtime_universe(
                user_id,
                &instruments,
                interval,
                start_time_ms,
                decision_time_ms,
            )
        },
        adaq_bot_runtime::unix_now_ms,
        std::thread::sleep,
    )?;
    let observations = local.features.runtime_observations(
        user_id,
        &plan,
        &observation.snapshot_id,
        &adaq_feature_engine::ObservationRange {
            start_time_ms,
            end_time_ms: decision_time_ms + 1,
        },
    )?;
    let slots = if bundle.runtime_bundle.input.pipeline.factors.is_empty()
        && bundle.runtime_bundle.input.pipeline.models.is_empty()
    {
        &bundle.runtime_bundle.input.strategy.feature_slots
    } else {
        &bundle.runtime_bundle.input.pipeline.input_slots
    };
    let values_for = |instrument_id: &str, time: i64| {
        slots
            .iter()
            .map(|slot| {
                observations
                    .iter()
                    .find(|row| {
                        row.instrument_id == instrument_id
                            && row.output_name == *slot
                            && row.observation_time_ms == time
                    })
                    .and_then(|row| match row.value {
                        adaq_feature_engine::FeatureObservationValue::Available {
                            value,
                            available_at_ms,
                        } if value.is_finite() && available_at_ms <= time => Some(value),
                        _ => None,
                    })
            })
            .collect::<Vec<_>>()
    };
    let rows = instruments
        .iter()
        .map(|instrument_id| {
            let values = values_for(instrument_id, decision_time_ms);
            if values.iter().any(Option::is_none) {
                return Err(format!(
                    "Runtime Features are unavailable for {instrument_id}"
                ));
            }
            Ok(adaq_bot_runtime::WorkerFeatureRow {
                instrument_id: instrument_id.clone(),
                available_at_ms: decision_time_ms,
                values,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let (deadline_ms, next_execution_ms) =
        host_schedule_window(decision_time_ms, adaq_bot_runtime::unix_now_ms())?;
    let (clock, input) = match &bundle.schedule {
        BotSchedule::ScheduledCrossSection { .. } => (
            DecisionClock::ScheduledCrossSection {
                decision_id: host_decision_id(bundle, "scheduled-cross-section", decision_time_ms)?,
                decision_time_ms,
                deadline_ms,
                next_execution_ms,
                universe: instruments.clone(),
                available_instruments: instruments,
            },
            WorkerDecisionInput::Portfolio {
                universe_id: bundle.universe_id.clone(),
                rows,
                state: adaq_bot_runtime::WorkerPortfolioState {
                    cash: "0".into(),
                    positions: Vec::new(),
                },
            },
        ),
        BotSchedule::ClosedBar { instrument_id, .. } => (
            DecisionClock::ClosedBar {
                decision_id: host_decision_id(bundle, "closed-bar", decision_time_ms)?,
                instrument_id: instrument_id.clone(),
                decision_time_ms,
                available_at_ms: decision_time_ms,
                deadline_ms,
                next_execution_ms,
            },
            WorkerDecisionInput::Strategy {
                instrument_id: instrument_id.clone(),
                frames: observations
                    .iter()
                    .filter(|row| row.instrument_id == *instrument_id)
                    .map(|row| row.observation_time_ms)
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .map(|time| {
                        Ok(adaq_bot_runtime::WorkerFeatureFrame {
                            instrument_id: instrument_id.clone(),
                            open_time_ms: adaq_data_pipeline::okx::latest_closed_bar_boundary_ms(
                                time - 1,
                                interval,
                            )
                            .map_err(|error| error.to_string())?,
                            available_at_ms: time,
                            values: values_for(instrument_id, time),
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?,
            },
        ),
        BotSchedule::EmaDoubleCross { .. } => unreachable!(),
    };
    Ok(HostDecisionBatch {
        clock,
        input,
        market_data_universe_snapshot_id: Some(observation.snapshot_id),
    })
}

fn acquire_closed_bar_with_retry<T>(
    deadline_ms: i64,
    mut acquire: impl FnMut() -> Result<T, String>,
    mut now_ms: impl FnMut() -> i64,
    mut wait: impl FnMut(Duration),
) -> Result<T, String> {
    if now_ms() >= deadline_ms {
        return Err(
            "Runtime Canonical OKX bars were not ready before the decision deadline".into(),
        );
    }
    let mut result = acquire();
    // OKX can confirm the just-closed bar after the boundary. Retry data reads
    // before the one idempotent Decision result, within its original deadline.
    while result
        .as_ref()
        .is_err_and(|error| error == crate::local_research::RUNTIME_BARS_INCOMPLETE)
        && now_ms().saturating_add(1_000) < deadline_ms
    {
        wait(Duration::from_secs(1));
        if now_ms() >= deadline_ms {
            break;
        }
        result = acquire();
    }
    result
}

fn is_exact_feature_context(
    bundle: &BotDeploymentBundle,
    feature_plan_hash: &str,
    market_data_snapshot_id: &str,
    point_in_time_universe_id: &str,
) -> bool {
    feature_plan_hash == bundle.runtime_bundle.input.feature_plan_hash
        && market_data_snapshot_id == bundle.market_data_snapshot_id
        && point_in_time_universe_id == bundle.universe_snapshot_id
}

fn host_schedule_clock(
    local: &LocalResearchState,
    user_id: &str,
    bundle: &BotDeploymentBundle,
    dataset_id: &str,
    trade_id: Option<&str>,
) -> Result<DecisionClock, String> {
    if let BotSchedule::EmaDoubleCross { instrument_id } = &bundle.schedule {
        let trade_id = trade_id.unwrap_or(dataset_id);
        return host_event_clock(local, user_id, bundle, instrument_id, trade_id);
    }
    let store = local.features.materialization_store();
    let dataset =
        crate::features::Features::completed_dataset_from_store(&store, user_id, dataset_id)?;
    if !is_exact_feature_context(
        bundle,
        &dataset.feature_plan_hash,
        &dataset.market_data_snapshot_id,
        &dataset.point_in_time_universe_id,
    ) {
        return Err("Feature Dataset is not the exact frozen Bot Feature context.".into());
    }
    let now = adaq_bot_runtime::unix_now_ms();
    match &bundle.schedule {
        BotSchedule::ClosedBar { instrument_id, .. } => {
            let (_, bars) = local
                .snapshots
                .snapshot_for_user(user_id, &dataset.market_data_snapshot_id)?;
            let interval = bundle_interval(bundle)?;
            let closes = bars
                .into_iter()
                .map(|bar| {
                    adaq_data_core::next_bar_open_time_ms(bar.open_time_ms, interval)
                        .map(|close| (close, bar.open_time_ms))
                        .map_err(|error| error.to_string())
                })
                .collect::<Result<BTreeMap<_, _>, _>>()?;
            let row = dataset
                .rows
                .iter()
                .filter(|row| {
                    row.instrument_id == *instrument_id
                        && row.observation_time_ms <= now
                        && closes.contains_key(&row.observation_time_ms)
                })
                .max_by_key(|row| row.observation_time_ms)
                .ok_or_else(|| {
                    "No complete Feature Dataset rows are available for this ClosedBar.".to_owned()
                })?;
            let available_at_ms = host_feature_available_at(bundle, row)?;
            let (deadline_ms, next_execution_ms) =
                host_schedule_window(row.observation_time_ms, now)?;
            Ok(DecisionClock::ClosedBar {
                decision_id: host_decision_id(bundle, "closed-bar", row.observation_time_ms)?,
                instrument_id: instrument_id.clone(),
                decision_time_ms: row.observation_time_ms,
                available_at_ms,
                deadline_ms,
                next_execution_ms,
            })
        }
        BotSchedule::ScheduledCrossSection { instruments, .. } => {
            let mut rows_by_time: BTreeMap<
                i64,
                BTreeMap<&str, &adaq_feature_engine::FeatureDatasetRow>,
            > = BTreeMap::new();
            for row in dataset.rows.iter().filter(|row| {
                row.observation_time_ms <= now && instruments.contains(&row.instrument_id)
            }) {
                if rows_by_time
                    .entry(row.observation_time_ms)
                    .or_default()
                    .insert(row.instrument_id.as_str(), row)
                    .is_some()
                {
                    return Err("Feature Dataset contains duplicate scheduled rows.".into());
                }
            }
            let (decision_time_ms, rows) = rows_by_time
                .into_iter()
                .next_back()
                .filter(|(_, rows)| rows.len() == instruments.len())
                .ok_or_else(|| {
                    "No complete Feature Dataset cross-section is available.".to_owned()
                })?;
            for instrument_id in instruments {
                let row = rows
                    .get(instrument_id.as_str())
                    .copied()
                    .ok_or_else(|| format!("Missing scheduled row for {instrument_id}."))?;
                host_feature_available_at(bundle, row)?;
            }
            let (deadline_ms, next_execution_ms) = host_schedule_window(decision_time_ms, now)?;
            Ok(DecisionClock::ScheduledCrossSection {
                decision_id: host_decision_id(bundle, "scheduled-cross-section", decision_time_ms)?,
                decision_time_ms,
                deadline_ms,
                next_execution_ms,
                universe: instruments.clone(),
                available_instruments: instruments.clone(),
            })
        }
        BotSchedule::EmaDoubleCross { .. } => unreachable!("event schedule handled above"),
    }
}

fn host_event_clock(
    local: &LocalResearchState,
    user_id: &str,
    bundle: &BotDeploymentBundle,
    instrument_id: &str,
    trade_id: &str,
) -> Result<DecisionClock, String> {
    let retained = local
        .okx
        .retained_trade_for_user(user_id, okx_instrument_code(instrument_id), trade_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "The requested retained OKX Trade is unavailable.".to_owned())?;
    let trade = retained.trade;
    let now = adaq_bot_runtime::unix_now_ms();
    if trade.timestamp_ms < 0
        || trade.timestamp_ms > now
        || retained.received_at_ms <= 0
        || retained.received_at_ms > now
        || retained.available_at_ms < trade.timestamp_ms
        || retained.available_at_ms > retained.received_at_ms
    {
        return Err("The retained OKX Trade has invalid availability metadata.".into());
    }
    if now.saturating_sub(retained.received_at_ms) > DECISION_DEADLINE_GRACE_MS {
        return Err("The retained OKX Trade is stale.".into());
    }
    let (deadline_ms, next_execution_ms) = host_schedule_window(retained.received_at_ms, now)?;
    Ok(DecisionClock::TradeEvent {
        decision_id: host_decision_id(
            bundle,
            &format!("ema-double-cross:{trade_id}"),
            trade.timestamp_ms,
        )?,
        instrument_id: instrument_id.into(),
        observation_time_ms: trade.timestamp_ms,
        decision_time_ms: retained.received_at_ms,
        available_at_ms: retained.available_at_ms,
        deadline_ms,
        next_execution_ms,
    })
}

fn host_feature_available_at(
    bundle: &BotDeploymentBundle,
    row: &adaq_feature_engine::FeatureDatasetRow,
) -> Result<i64, String> {
    let slots = if bundle.runtime_bundle.input.pipeline.factors.is_empty()
        && bundle.runtime_bundle.input.pipeline.models.is_empty()
    {
        &bundle.runtime_bundle.input.strategy.feature_slots
    } else {
        &bundle.runtime_bundle.input.pipeline.input_slots
    };
    let mut available_at_ms = row.observation_time_ms;
    for slot in slots {
        match row.values.get(slot) {
            Some(adaq_feature_engine::FeatureDatasetCell::Available {
                value,
                available_at_ms: cell_available_at_ms,
            }) if value.is_finite() && *cell_available_at_ms <= row.observation_time_ms => {
                available_at_ms = available_at_ms.max(*cell_available_at_ms);
            }
            _ => {
                return Err(
                    "Feature Dataset values are incomplete or unavailable at the decision time."
                        .into(),
                );
            }
        }
    }
    Ok(available_at_ms)
}

fn host_decision_input(
    local: &LocalResearchState,
    user_id: &str,
    bundle: &BotDeploymentBundle,
    dataset_id: &str,
    trade_id: Option<&str>,
    clock: &DecisionClock,
    previous_event_cursor: Option<(i64, u32)>,
) -> Result<WorkerDecisionInput, String> {
    if let BotSchedule::EmaDoubleCross { instrument_id } = &bundle.schedule {
        let trade_id = trade_id.unwrap_or(dataset_id);
        return host_event_input(
            local,
            user_id,
            bundle,
            instrument_id,
            trade_id,
            clock,
            previous_event_cursor,
        );
    }
    let store = local.features.materialization_store();
    let dataset =
        crate::features::Features::completed_dataset_from_store(&store, user_id, dataset_id)?;
    if !is_exact_feature_context(
        bundle,
        &dataset.feature_plan_hash,
        &dataset.market_data_snapshot_id,
        &dataset.point_in_time_universe_id,
    ) {
        return Err("Feature Dataset is not the exact frozen Bot Feature context.".into());
    }
    let slots = if bundle.runtime_bundle.input.pipeline.factors.is_empty()
        && bundle.runtime_bundle.input.pipeline.models.is_empty()
    {
        &bundle.runtime_bundle.input.strategy.feature_slots
    } else {
        &bundle.runtime_bundle.input.pipeline.input_slots
    };
    let values_for = |row: &adaq_feature_engine::FeatureDatasetRow| {
        let mut available_at_ms = row.observation_time_ms;
        let values = slots
            .iter()
            .map(|slot| match row.values.get(slot) {
                Some(adaq_feature_engine::FeatureDatasetCell::Available {
                    value,
                    available_at_ms: cell_available_at_ms,
                }) if value.is_finite() => {
                    available_at_ms = available_at_ms.max(*cell_available_at_ms);
                    Some(*value)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        (values, available_at_ms)
    };
    match clock {
        DecisionClock::ClosedBar {
            instrument_id,
            decision_time_ms,
            ..
        } => {
            let (_, bars) = local
                .snapshots
                .snapshot_for_user(user_id, &dataset.market_data_snapshot_id)?;
            let interval = bundle_interval(bundle)?;
            let mut open_by_close = BTreeMap::new();
            for bar in bars {
                let close = adaq_data_core::next_bar_open_time_ms(bar.open_time_ms, interval)
                    .map_err(|error| error.to_string())?;
                open_by_close.insert(close, bar.open_time_ms);
            }
            let mut rows = dataset
                .rows
                .iter()
                .filter(|row| {
                    row.instrument_id == *instrument_id
                        && row.observation_time_ms <= *decision_time_ms
                })
                .collect::<Vec<_>>();
            let max_frames = usize::try_from(
                bundle
                    .runtime_bundle
                    .input
                    .worker_policy
                    .max_decision_frames,
            )
            .map_err(|_| "Worker frame limit exceeds the Host allocation limit".to_owned())?;
            if rows.len() > max_frames {
                let start = rows.len() - max_frames;
                rows = rows.split_off(start);
            }
            if rows.is_empty() {
                return Err(
                    "No complete Feature Dataset rows are available for this ClosedBar.".into(),
                );
            }
            let frames = rows
                .into_iter()
                .map(|row| {
                    let open_time_ms = open_by_close
                        .get(&row.observation_time_ms)
                        .copied()
                        .ok_or_else(|| {
                            "ClosedBar identity is not present in the frozen Snapshot.".to_owned()
                        })?;
                    let (values, available_at_ms) = values_for(row);
                    Ok(adaq_bot_runtime::WorkerFeatureFrame {
                        instrument_id: row.instrument_id.clone(),
                        open_time_ms,
                        available_at_ms,
                        values,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(WorkerDecisionInput::Strategy {
                instrument_id: instrument_id.clone(),
                frames,
            })
        }
        DecisionClock::ScheduledCrossSection {
            decision_time_ms,
            universe,
            ..
        } => {
            let rows = universe
                .iter()
                .map(|instrument_id| {
                    let row = dataset
                        .rows
                        .iter()
                        .find(|row| {
                            row.instrument_id == *instrument_id
                                && row.observation_time_ms == *decision_time_ms
                        })
                        .ok_or_else(|| {
                            format!(
                                "No complete Feature Dataset row is available for {instrument_id}."
                            )
                        })?;
                    let (values, available_at_ms) = values_for(row);
                    Ok(adaq_bot_runtime::WorkerFeatureRow {
                        instrument_id: row.instrument_id.clone(),
                        available_at_ms,
                        values,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(WorkerDecisionInput::Portfolio {
                universe_id: bundle.universe_id.clone(),
                rows,
                state: adaq_bot_runtime::WorkerPortfolioState {
                    cash: "0".into(),
                    positions: Vec::new(),
                },
            })
        }
        DecisionClock::TradeEvent { .. } => unreachable!("event clock handled above"),
    }
}

fn host_event_input(
    local: &LocalResearchState,
    user_id: &str,
    bundle: &BotDeploymentBundle,
    instrument_id: &str,
    trade_id: &str,
    clock: &DecisionClock,
    previous_event_cursor: Option<(i64, u32)>,
) -> Result<WorkerDecisionInput, String> {
    let DecisionClock::TradeEvent {
        decision_time_ms,
        available_at_ms,
        ..
    } = clock
    else {
        return Err("EMA double-cross input requires a TradeEvent clock.".into());
    };
    let retained = local
        .okx
        .retained_trade_for_user(user_id, okx_instrument_code(instrument_id), trade_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "The requested retained OKX Trade is unavailable.".to_owned())?;
    let trade = retained.trade;
    let (snapshot, snapshot_bars) = local
        .snapshots
        .snapshot_for_user(user_id, &bundle.market_data_snapshot_id)?;
    if okx_instrument_code(&snapshot.code) != okx_instrument_code(instrument_id)
        || snapshot.interval != adaq_data_core::BarInterval::FifteenMinutes
        || !snapshot.gaps.is_empty()
    {
        return Err(
            "The frozen Market Data Snapshot is not a complete exact 15m EMA context.".into(),
        );
    }
    if trade.timestamp_ms > *decision_time_ms
        || retained.available_at_ms > *available_at_ms
        || retained.received_at_ms != *decision_time_ms
    {
        return Err("The retained Trade is newer than the Host decision clock.".into());
    }
    let stream_health = local
        .okx
        .stream_health(user_id)
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|health| health.stream_kind == "trade");
    if stream_health
        .as_ref()
        .is_some_and(|health| health.status != "live")
    {
        return Err("The OKX Trade stream is not live; confirmation evidence is blocked.".into());
    }
    let stream_epoch = stream_health
        .map(|health| health.reconnect_count)
        .unwrap_or_default();
    let interval_ms = adaq_bot_runtime::ema_double_cross::EMA_BAR_INTERVAL_MS;
    let previous_observation_time_ms =
        host_event_replay_after_ms(previous_event_cursor, stream_epoch);
    let last_snapshot_close_ms = snapshot_bars
        .iter()
        .filter_map(|bar| bar.open_time_ms.checked_add(interval_ms))
        .filter(|close_time_ms| *close_time_ms <= trade.timestamp_ms)
        .max()
        .unwrap_or_default();
    let live_trade_start_ms = last_snapshot_close_ms.max(
        previous_observation_time_ms
            .map(|time_ms| time_ms.div_euclid(interval_ms) * interval_ms)
            .unwrap_or_default(),
    );
    let live_trades = if trade.timestamp_ms > live_trade_start_ms {
        local
            .okx
            .retained_trades_for_user(
                user_id,
                okx_instrument_code(instrument_id),
                live_trade_start_ms,
                trade.timestamp_ms,
            )
            .map_err(|error| error.to_string())?
    } else {
        Vec::new()
    };
    let mut events = host_event_bar_events(
        instrument_id,
        &bundle.market_data_snapshot_id,
        snapshot_bars,
        live_trades,
        trade.timestamp_ms,
        previous_observation_time_ms,
        interval_ms,
    );
    let max_events = usize::try_from(
        bundle
            .runtime_bundle
            .input
            .worker_policy
            .max_decision_frames,
    )
    .map_err(|_| "Worker event limit exceeds the Host allocation limit".to_owned())?;
    let bar_limit = max_events.saturating_sub(1);
    if events.len() > bar_limit {
        return Err(
            "The frozen EMA replay interval exceeds the Worker event limit; no truncated replay is authorized."
                .into(),
        );
    }
    events.push(adaq_bot_runtime::WorkerMarketEvent::Trade {
        instrument_id: instrument_id.into(),
        trade_id: trade.trade_id,
        price: trade.price.to_string(),
        quantity: trade.quantity.to_string(),
        observed_at_ms: trade.timestamp_ms,
        available_at_ms: *available_at_ms,
        received_at_ms: *decision_time_ms,
    });
    let account = local.paper_trading.view_optional(user_id)?.ok_or_else(|| {
        "A reconciled OKX Demo account is required before a Decision Batch.".to_owned()
    })?;
    if !account_is_reconciled_for_target(local, user_id, bundle, &account)? {
        return Err("Account state is stale, uncertain, or bound to another account.".into());
    }
    let owned_position = if matches!(&bundle.schedule, BotSchedule::EmaDoubleCross { .. }) {
        bot_owned_position(&account, &bundle.bot_id, instrument_id).quantity > Decimal::ZERO
    } else {
        account
            .account
            .positions
            .get(okx_instrument_code(instrument_id))
            .is_some_and(|position| position.quantity > Decimal::ZERO)
    };
    Ok(WorkerDecisionInput::Event {
        instrument_id: instrument_id.into(),
        events,
        owned_position,
        stream_epoch,
    })
}

fn host_event_bar_events(
    instrument_id: &str,
    snapshot_id: &str,
    snapshot_bars: Vec<OhlcvBar>,
    live_trades: Vec<MarketTrade>,
    trade_timestamp_ms: i64,
    previous_observation_time_ms: Option<i64>,
    interval_ms: i64,
) -> Vec<adaq_bot_runtime::WorkerMarketEvent> {
    let mut events = snapshot_bars
        .into_iter()
        .filter_map(|bar| {
            let close_time_ms = bar.open_time_ms.checked_add(interval_ms)?;
            (close_time_ms <= trade_timestamp_ms
                && previous_observation_time_ms
                    .is_none_or(|previous_time_ms| close_time_ms > previous_time_ms))
            .then(|| adaq_bot_runtime::WorkerMarketEvent::BarClosed {
                instrument_id: instrument_id.into(),
                bar_open_time_ms: bar.open_time_ms,
                close: bar.close.to_string(),
                observed_at_ms: close_time_ms,
                available_at_ms: close_time_ms,
                evidence_id: hash_json(&(snapshot_id, bar.open_time_ms))
                    .unwrap_or_else(|_| format!("bar-{}", bar.open_time_ms)),
                replay: true,
            })
        })
        .collect::<Vec<_>>();
    let mut live_bars = BTreeMap::<i64, (i64, String, Decimal)>::new();
    for live_trade in live_trades {
        let bar_open_time_ms = live_trade.timestamp_ms.div_euclid(interval_ms) * interval_ms;
        let entry = live_bars.entry(bar_open_time_ms).or_insert((
            live_trade.timestamp_ms,
            live_trade.trade_id.clone(),
            live_trade.price,
        ));
        if (live_trade.timestamp_ms, &live_trade.trade_id) > (entry.0, &entry.1) {
            *entry = (
                live_trade.timestamp_ms,
                live_trade.trade_id,
                live_trade.price,
            );
        }
    }
    events.extend(live_bars.into_iter().filter_map(
        |(bar_open_time_ms, (_, last_trade_id, close))| {
            let close_time_ms = bar_open_time_ms.checked_add(interval_ms)?;
            (close_time_ms <= trade_timestamp_ms
                && previous_observation_time_ms
                    .is_none_or(|previous_time_ms| close_time_ms > previous_time_ms))
            .then(|| adaq_bot_runtime::WorkerMarketEvent::BarClosed {
                instrument_id: instrument_id.into(),
                bar_open_time_ms,
                close: close.to_string(),
                observed_at_ms: close_time_ms,
                available_at_ms: close_time_ms,
                evidence_id: hash_json(&(
                    "okx-trade-bar",
                    instrument_id,
                    bar_open_time_ms,
                    last_trade_id,
                ))
                .unwrap_or_else(|_| format!("trade-bar-{instrument_id}-{bar_open_time_ms}")),
                replay: false,
            })
        },
    ));
    events.sort_by_key(|event| match event {
        adaq_bot_runtime::WorkerMarketEvent::BarClosed {
            bar_open_time_ms, ..
        } => *bar_open_time_ms,
        adaq_bot_runtime::WorkerMarketEvent::Trade { .. } => i64::MAX,
    });
    events
}

fn host_event_replay_after_ms(cursor: Option<(i64, u32)>, stream_epoch: u32) -> Option<i64> {
    cursor
        .filter(|(_, previous_epoch)| *previous_epoch == stream_epoch)
        .map(|(observation_time_ms, _)| observation_time_ms)
}

fn host_schedule_window(decision_time_ms: i64, now_ms: i64) -> Result<(i64, i64), String> {
    if decision_time_ms > now_ms {
        return Err("Host schedule produced a future Decision Batch.".into());
    }
    let deadline_ms = decision_time_ms
        .checked_add(DECISION_DEADLINE_GRACE_MS)
        .ok_or_else(|| "Host schedule deadline overflowed the time limit.".to_owned())?;
    if now_ms > deadline_ms {
        return Err("Host schedule Decision Batch is late; No Target is authorized.".into());
    }
    let next_execution_ms = decision_time_ms
        .checked_add(1)
        .ok_or_else(|| "Host schedule execution time overflowed the time limit.".to_owned())?;
    Ok((deadline_ms, next_execution_ms))
}

fn host_decision_id(
    bundle: &BotDeploymentBundle,
    schedule_kind: &str,
    decision_time_ms: i64,
) -> Result<String, String> {
    Ok(format!(
        "host-{}",
        hash_json(&(bundle.identity.as_str(), schedule_kind, decision_time_ms))?
    ))
}

fn unavailable_decision_id(
    bundle: &BotDeploymentBundle,
    request_id: &str,
    detail: &str,
) -> Result<String, String> {
    Ok(format!(
        "unavailable-{}",
        hash_json(&(bundle.identity.as_str(), request_id, detail))?
    ))
}

fn bundle_interval(bundle: &BotDeploymentBundle) -> Result<adaq_data_core::BarInterval, String> {
    match &bundle.schedule {
        BotSchedule::ClosedBar { interval, .. } => adaq_data_core::BarInterval::ALL
            .iter()
            .copied()
            .find(|candidate| candidate.as_str() == interval)
            .ok_or_else(|| "Bot ClosedBar interval is invalid".to_owned()),
        BotSchedule::EmaDoubleCross { .. } => Ok(adaq_data_core::BarInterval::FifteenMinutes),
        BotSchedule::ScheduledCrossSection { .. } => {
            Err("Portfolio decision does not have a ClosedBar interval".into())
        }
    }
}

fn validate_decision_input(
    bundle: &BotDeploymentBundle,
    clock: &DecisionClock,
    input: &WorkerDecisionInput,
) -> Result<(), String> {
    if !bounded(clock.decision_id(), 128) {
        return Err("Decision identity is missing or exceeds the Host limit.".into());
    }
    clock.validate().map_err(|error| error.to_string())?;
    match (&bundle.schedule, clock, input) {
        (
            BotSchedule::ClosedBar { instrument_id, .. },
            DecisionClock::ClosedBar {
                instrument_id: clock_instrument,
                decision_time_ms,
                ..
            },
            WorkerDecisionInput::Strategy {
                instrument_id: input_instrument,
                frames,
            },
        ) if instrument_id == clock_instrument
            && instrument_id == input_instrument
            && !frames.is_empty()
            && u64::try_from(frames.len()).ok().is_some_and(|count| {
                count
                    <= bundle
                        .runtime_bundle
                        .input
                        .worker_policy
                        .max_decision_frames
            })
            && frames
                .windows(2)
                .all(|pair| pair[0].open_time_ms < pair[1].open_time_ms)
            && frames.iter().all(|frame| {
                frame.instrument_id == *instrument_id
                    && frame.open_time_ms <= *decision_time_ms
                    && frame.available_at_ms <= *decision_time_ms
                    && frame.values.iter().all(|value| value.is_some())
            }) =>
        {
            Ok(())
        }
        (
            BotSchedule::ScheduledCrossSection {
                universe_id,
                instruments,
            },
            DecisionClock::ScheduledCrossSection {
                universe,
                available_instruments,
                ..
            },
            WorkerDecisionInput::Portfolio {
                universe_id: input_universe,
                rows,
                ..
            },
        ) if universe_id == input_universe
            && universe == instruments
            && available_instruments == instruments
            && rows.len() == instruments.len()
            && rows.iter().zip(instruments).all(|(row, instrument)| {
                row.instrument_id == *instrument
                    && row.available_at_ms <= clock.decision_time_ms()
                    && row.values.iter().all(|value| value.is_some())
            }) =>
        {
            Ok(())
        }
        (
            BotSchedule::EmaDoubleCross { instrument_id },
            DecisionClock::TradeEvent {
                instrument_id: clock_instrument,
                decision_time_ms,
                ..
            },
            WorkerDecisionInput::Event {
                instrument_id: input_instrument,
                events,
                ..
            },
        ) if instrument_id == clock_instrument
            && instrument_id == input_instrument
            && !events.is_empty()
            && events.len()
                <= usize::try_from(
                    bundle
                        .runtime_bundle
                        .input
                        .worker_policy
                        .max_decision_frames,
                )
                .unwrap_or_default()
            && matches!(
                events.last(),
                Some(adaq_bot_runtime::WorkerMarketEvent::Trade { .. })
            )
            && events
                .iter()
                .all(|event| valid_market_event(event, instrument_id, *decision_time_ms)) =>
        {
            Ok(())
        }
        _ => Err("Decision Batch does not match the immutable Bot schedule.".into()),
    }
}

fn valid_market_event(
    event: &adaq_bot_runtime::WorkerMarketEvent,
    expected_instrument: &str,
    decision_time_ms: i64,
) -> bool {
    if event.instrument_id() != expected_instrument || !bounded(event.event_id(), 256) {
        return false;
    }
    match event {
        adaq_bot_runtime::WorkerMarketEvent::BarClosed {
            bar_open_time_ms,
            close,
            observed_at_ms,
            available_at_ms,
            ..
        } => {
            *bar_open_time_ms >= 0
                && bar_open_time_ms % adaq_bot_runtime::ema_double_cross::EMA_BAR_INTERVAL_MS == 0
                && adaq_component_sdk::parse_decimal(close).is_ok_and(|close| close > Decimal::ZERO)
                && *observed_at_ms
                    >= bar_open_time_ms
                        .saturating_add(adaq_bot_runtime::ema_double_cross::EMA_BAR_INTERVAL_MS)
                && *observed_at_ms <= *available_at_ms
                && *available_at_ms <= decision_time_ms
        }
        adaq_bot_runtime::WorkerMarketEvent::Trade {
            price,
            quantity,
            observed_at_ms,
            available_at_ms,
            received_at_ms,
            ..
        } => {
            adaq_component_sdk::parse_decimal(price).is_ok_and(|price| price > Decimal::ZERO)
                && adaq_component_sdk::parse_decimal(quantity)
                    .is_ok_and(|quantity| quantity > Decimal::ZERO)
                && *observed_at_ms >= 0
                && *observed_at_ms <= *available_at_ms
                && *available_at_ms <= *received_at_ms
                && *received_at_ms <= decision_time_ms
        }
    }
}

struct PlannedSpotOrder {
    instrument: String,
    side: &'static str,
    quantity: Decimal,
    limit_price: Decimal,
}

fn plan_spot_order(
    instrument: &str,
    price: Decimal,
    desired_notional: Decimal,
    current_notional: Decimal,
    equity: Decimal,
    available_cash: Decimal,
    sellable_quantity: Decimal,
    profile: &ExecutionProfile,
) -> Result<Option<PlannedSpotOrder>, String> {
    if price <= Decimal::ZERO || equity <= Decimal::ZERO {
        return Err("Execution price or account equity is invalid.".into());
    }
    let difference = desired_notional
        .checked_sub(current_notional)
        .ok_or_else(|| "Execution notional overflowed the Decimal limit.".to_owned())?;
    if difference.is_zero()
        || difference
            .abs()
            .checked_div(equity)
            .ok_or_else(|| "Execution threshold overflowed the Decimal limit.".to_owned())?
            < profile.rebalance_threshold
    {
        return Ok(None);
    }
    let side = if difference.is_sign_positive() {
        "buy"
    } else {
        "sell"
    };
    let limit_price = match side {
        "buy" => floor_increment(price, profile.price_increment)?,
        _ => ceil_increment(price, profile.price_increment)?,
    };
    let fee = match profile.fill_policy {
        adaq_backtest_core::FillPolicy::Maker => profile.maker_fee_rate,
        adaq_backtest_core::FillPolicy::Taker => profile.taker_fee_rate,
    };
    let raw_quantity = if side == "buy" {
        let requested = difference
            .checked_div(limit_price)
            .ok_or_else(|| "Execution quantity overflowed the Decimal limit.".to_owned())?;
        let affordable = available_cash
            .checked_div(limit_price.checked_mul(Decimal::ONE + fee).ok_or_else(|| {
                "Execution fee calculation overflowed the Decimal limit.".to_owned()
            })?)
            .ok_or_else(|| "Execution affordability overflowed the Decimal limit.".to_owned())?;
        requested.min(affordable)
    } else {
        difference
            .abs()
            .checked_div(limit_price)
            .ok_or_else(|| "Execution quantity overflowed the Decimal limit.".to_owned())?
            .min(sellable_quantity)
    };
    let quantity = floor_increment(raw_quantity, profile.quantity_increment)?;
    if quantity < profile.minimum_quantity {
        return Ok(None);
    }
    Ok(Some(PlannedSpotOrder {
        instrument: instrument.into(),
        side,
        quantity,
        limit_price,
    }))
}

fn ema_addition_blocked(position_quantity: Decimal, difference: Decimal) -> bool {
    position_quantity > Decimal::ZERO && difference > Decimal::ZERO
}

fn floor_increment(value: Decimal, increment: Decimal) -> Result<Decimal, String> {
    if increment <= Decimal::ZERO {
        return Err("Execution increment must be positive.".into());
    }
    value
        .checked_sub(value % increment)
        .ok_or_else(|| "Execution rounding overflowed the Decimal limit.".to_owned())
}

fn ceil_increment(value: Decimal, increment: Decimal) -> Result<Decimal, String> {
    let floor = floor_increment(value, increment)?;
    if floor == value {
        Ok(value)
    } else {
        floor
            .checked_add(increment)
            .ok_or_else(|| "Execution rounding overflowed the Decimal limit.".to_owned())
    }
}

fn authoritative_decision_input(
    local: &LocalResearchState,
    user_id: &str,
    bundle: &BotDeploymentBundle,
    input: &WorkerDecisionInput,
) -> Result<WorkerDecisionInput, String> {
    let WorkerDecisionInput::Portfolio {
        universe_id, rows, ..
    } = input
    else {
        return Ok(input.clone());
    };
    let BotSchedule::ScheduledCrossSection { instruments, .. } = &bundle.schedule else {
        return Err("Portfolio state is unavailable for a non-Portfolio Bot.".into());
    };
    // Accepted intents must settle against the provider before the next portfolio snapshot.
    let account = require_reconciled_account(local, user_id, bundle)?;
    if account.account.account_id != bundle.account_id
        || !account_is_reconciled_and_quiet(Some(&account))
    {
        return Err("Portfolio state is stale, uncertain, or bound to another account.".into());
    }
    if account.account.positions.keys().any(|instrument| {
        canonical_okx_instrument_id(instrument)
            .map(|instrument| !instruments.contains(&instrument))
            .unwrap_or(true)
    }) {
        return Err("Unowned account exposure prevents a Portfolio Decision Batch.".into());
    }
    let positions = account
        .account
        .positions
        .iter()
        .map(|(instrument, position)| {
            Ok(adaq_bot_runtime::WorkerPosition {
                instrument_id: canonical_okx_instrument_id(instrument)?,
                quantity: position.quantity.to_string(),
                price: market_price(local, user_id, instrument)?.to_string(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(WorkerDecisionInput::Portfolio {
        universe_id: universe_id.clone(),
        rows: rows.clone(),
        state: adaq_bot_runtime::WorkerPortfolioState {
            cash: account.account.cash.to_string(),
            positions,
        },
    })
}

/// Typed handoff across the decide → execute seam.
///
/// `supervisor.decision()` produces a `WorkerTarget`; this struct carries it
/// (plus its decision identity and evaluation evidence) as an observable,
/// persistable step before `execute_target` consumes it. It makes the
/// Strategy Target → Approved Target boundary explicit instead of two loose
/// arguments, and the seam records it as a `decision-strategy-target` evidence
/// step so research intent is distinguishable from enforced execution.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct ApprovedTarget {
    pub target: WorkerTarget,
    pub decision_id: String,
    pub produced_at_ms: i64,
    pub evaluation: WorkerEvaluationEvidence,
}

fn execute_target(
    local: &LocalResearchState,
    bots: &BotStore,
    user_id: &str,
    bot_id: &str,
    bundle: &BotDeploymentBundle,
    clock: &DecisionClock,
    approved: &ApprovedTarget,
) -> Result<(), String> {
    let decision_id = &approved.decision_id;
    let target = &approved.target;
    let gate_ctx = crate::new_risk_gate::NewRiskContext {
        user_id,
        bot_id,
        account_id: &bundle.account_id,
        now_ms: adaq_bot_runtime::unix_now_ms(),
    };
    match crate::new_risk_gate::new_risk_blocked(
        &gate_ctx,
        &local.operations,
        bots,
        &local.paper_trading,
        &local.paper_experiments,
        crate::new_risk_gate::Phase::Execution,
    )? {
        crate::new_risk_gate::NewRiskOutcome::Permitted => {}
        crate::new_risk_gate::NewRiskOutcome::Blocked(block) => match block.effect {
            crate::new_risk_gate::BlockEffect::HardErr => {
                return Err(block.message);
            }
            crate::new_risk_gate::BlockEffect::SoftSkip => {
                bots.record_evidence(
                    user_id,
                    bot_id,
                    "experiment",
                    "risk-blocked-until-common-warmup",
                    &block.message,
                    Some(decision_id),
                )?;
                return Ok(());
            }
        },
    }
    let next_execution_ms = match clock {
        DecisionClock::ClosedBar {
            next_execution_ms, ..
        }
        | DecisionClock::ScheduledCrossSection {
            next_execution_ms, ..
        }
        | DecisionClock::TradeEvent {
            next_execution_ms, ..
        } => *next_execution_ms,
    };
    if adaq_bot_runtime::unix_now_ms() < next_execution_ms {
        bots.record_evidence(
            user_id,
            bot_id,
            "execution",
            "execution-deferred",
            "Target was not executed because the next eligible post-decision event has not arrived; no order was created.",
            Some(decision_id),
        )?;
        return Ok(());
    }
    let account = reconcile_account(local, user_id, bundle)?;
    if !account_is_reconciled_for_target(local, user_id, bundle, &account)? {
        return Err("Account evidence is stale, uncertain, or bound to another account.".into());
    }
    match target {
        WorkerTarget::Strategy {
            instrument_id,
            exposures,
        } => execute_strategy_target(
            local,
            bots,
            user_id,
            bot_id,
            bundle,
            &account,
            instrument_id,
            exposures,
            decision_id,
        ),
        WorkerTarget::Portfolio {
            universe_id,
            weights,
            cash_reserve,
        } => execute_portfolio_target(
            local,
            bots,
            user_id,
            bot_id,
            bundle,
            &account,
            clock.decision_time_ms(),
            universe_id,
            weights,
            cash_reserve,
            decision_id,
        ),
    }
}

fn execute_strategy_target(
    local: &LocalResearchState,
    bots: &BotStore,
    user_id: &str,
    bot_id: &str,
    bundle: &BotDeploymentBundle,
    account: &PaperAccountView,
    instrument_id: &str,
    exposures: &[adaq_bot_runtime::WorkerExposure],
    decision_id: &str,
) -> Result<(), String> {
    let schedule_matches = match &bundle.schedule {
        BotSchedule::ClosedBar {
            instrument_id: scheduled,
            ..
        }
        | BotSchedule::EmaDoubleCross {
            instrument_id: scheduled,
        } => scheduled == instrument_id,
        BotSchedule::ScheduledCrossSection { .. } => false,
    };
    if !schedule_matches
        || exposures.is_empty()
        || !exposures.iter().all(|exposure| {
            exposure.instrument_id == instrument_id
                && adaq_bot_runtime::is_decimal_text(&exposure.exposure)
        })
    {
        return Err(
            "Strategy Target does not match the immutable single-instrument schedule.".into(),
        );
    }
    let ema_schedule = matches!(&bundle.schedule, BotSchedule::EmaDoubleCross { .. });
    if ema_schedule && bot_has_pending_order(account, bot_id, instrument_id) {
        bots.record_evidence(
            user_id,
            bot_id,
            "execution",
            "execution-pending-order",
            "A pending or partially filled EMA order blocks a duplicate order for this Instrument.",
            Some(decision_id),
        )?;
        return Ok(());
    }
    if account.account.positions.keys().any(|instrument| {
        let code = okx_instrument_code(instrument);
        if canonical_okx_instrument_id(instrument)
            .map(|canonical| canonical == instrument_id)
            .unwrap_or(false)
        {
            return false;
        }
        !(ema_schedule
            && bots
                .instrument_lease_exists(user_id, &bundle.account_id, code)
                .unwrap_or(false))
    }) {
        return Err("Unowned account exposure prevents new Bot risk.".into());
    }
    let requested = exposures
        .last()
        .ok_or_else(|| "Strategy Target has no final exposure.".to_owned())?
        .exposure
        .parse::<Decimal>()
        .map_err(|_| "Strategy Target exposure is not exact Decimal text.".to_owned())?;
    if !(Decimal::ZERO..=Decimal::ONE).contains(&requested) {
        return Err("Strategy Target exposure must be within [0,1].".into());
    }
    let approved = requested.min(bundle.research_risk_policy.max_instrument_weight);
    bots.record_evidence(
        user_id,
        bot_id,
        "risk",
        if approved == requested {
            "target-approved"
        } else {
            "target-constrained"
        },
        if approved == requested {
            "Host Risk approved the Strategy Target for execution."
        } else {
            "Host Risk constrained the Strategy Target to the immutable maximum instrument weight."
        },
        Some(decision_id),
    )?;
    let price = market_price(local, user_id, instrument_id)?;
    let position = if ema_schedule {
        bot_owned_position(account, &bundle.bot_id, instrument_id)
    } else {
        account
            .account
            .positions
            .get(okx_instrument_code(instrument_id))
            .cloned()
            .unwrap_or(adaq_paper_trading_core::Position {
                quantity: Decimal::ZERO,
                sellable_quantity: Decimal::ZERO,
            })
    };
    let equity =
        account
            .account
            .cash
            .checked_add(position.quantity.checked_mul(price).ok_or_else(|| {
                "Strategy account equity overflowed the Decimal limit.".to_owned()
            })?)
            .ok_or_else(|| "Strategy account equity overflowed the Decimal limit.".to_owned())?;
    let target_equity = if matches!(&bundle.schedule, BotSchedule::EmaDoubleCross { .. }) {
        EMA_ENTRY_NOTIONAL_CAP_USDT
    } else {
        equity
    };
    let desired = target_equity
        .checked_mul(approved)
        .ok_or_else(|| "Strategy target notional overflowed the Decimal limit.".to_owned())?;
    let current = position
        .quantity
        .checked_mul(price)
        .ok_or_else(|| "Strategy position notional overflowed the Decimal limit.".to_owned())?;
    let difference = desired
        .checked_sub(current)
        .ok_or_else(|| "Strategy allocation difference overflowed the Decimal limit.".to_owned())?;
    if ema_schedule && ema_addition_blocked(position.quantity, difference) {
        bots.record_evidence(
            user_id,
            bot_id,
            "execution",
            "execution-addition-blocked",
            "The EMA Strategy already owns a position; partial entry evidence cannot authorize an additional buy.",
            Some(decision_id),
        )?;
        return Ok(());
    }
    if ema_schedule && difference > Decimal::ZERO {
        let fee = match bundle.execution_profile.fill_policy {
            adaq_backtest_core::FillPolicy::Maker => bundle.execution_profile.maker_fee_rate,
            adaq_backtest_core::FillPolicy::Taker => bundle.execution_profile.taker_fee_rate,
        };
        let required_cash = difference
            .checked_mul(Decimal::ONE + fee)
            .and_then(|value| value.checked_add(bundle.paper_risk_policy.reserve_cash))
            .ok_or_else(|| {
                "Strategy allocation cash requirement overflowed the Decimal limit.".to_owned()
            })?;
        if account.buying_power < required_cash {
            return Err(
                "The EMA Bot allocation is not fully funded; no partial entry order is authorized."
                    .into(),
            );
        }
    }
    let Some(order) = plan_spot_order(
        instrument_id,
        price,
        desired,
        current,
        equity,
        account.buying_power,
        position.sellable_quantity,
        &bundle.execution_profile,
    )?
    else {
        bots.record_evidence(
            user_id,
            bot_id,
            "execution",
            "execution-noop",
            "Approved Target is already within the frozen rebalance threshold.",
            Some(decision_id),
        )?;
        return Ok(());
    };
    submit_target_order(local, bots, user_id, bot_id, bundle, &order, decision_id)
}

fn execute_portfolio_target(
    local: &LocalResearchState,
    bots: &BotStore,
    user_id: &str,
    bot_id: &str,
    bundle: &BotDeploymentBundle,
    account: &PaperAccountView,
    decision_time_ms: i64,
    universe_id: &str,
    target_weights: &[adaq_bot_runtime::WorkerTargetWeight],
    cash_reserve: &str,
    decision_id: &str,
) -> Result<(), String> {
    let (scheduled_universe, instruments) = match &bundle.schedule {
        BotSchedule::ScheduledCrossSection {
            universe_id: scheduled_universe,
            instruments,
        } => (scheduled_universe, instruments),
        _ => return Err("Portfolio Target does not match the immutable schedule.".into()),
    };
    if scheduled_universe != universe_id
        || target_weights.len() != instruments.len()
        || target_weights
            .iter()
            .zip(instruments)
            .any(|(weight, instrument)| {
                weight.instrument_id != *instrument
                    || !adaq_bot_runtime::is_decimal_text(&weight.weight)
            })
    {
        return Err("Portfolio Target does not contain the complete scheduled Universe.".into());
    }
    if account.account.positions.keys().any(|instrument| {
        canonical_okx_instrument_id(instrument)
            .map(|instrument| !instruments.contains(&instrument))
            .unwrap_or(true)
    }) {
        return Err("Unowned account exposure prevents new Portfolio risk.".into());
    }
    let cash_reserve = cash_reserve
        .parse::<Decimal>()
        .map_err(|_| "Portfolio cash reserve is not exact Decimal text.".to_owned())?;
    let weights = target_weights
        .iter()
        .map(|weight| {
            weight
                .weight
                .parse::<Decimal>()
                .map(|value| (weight.instrument_id.clone(), value))
                .map_err(|_| "Portfolio weight is not exact Decimal text.".to_owned())
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let total_weight = weights
        .values()
        .copied()
        .try_fold(Decimal::ZERO, |total, weight| {
            total
                .checked_add(weight)
                .ok_or_else(|| "Portfolio weights overflowed the Decimal limit.".to_owned())
        })?;
    let total_allocation = cash_reserve
        .checked_add(total_weight)
        .ok_or_else(|| "Portfolio allocation overflowed the Decimal limit.".to_owned())?;
    if cash_reserve < Decimal::ZERO
        || weights.values().any(|weight| *weight < Decimal::ZERO)
        || total_allocation != Decimal::ONE
    {
        return Err("Portfolio Target weights and cash reserve must sum to one.".into());
    }
    let prices = instruments
        .iter()
        .map(|instrument| {
            Ok((
                instrument.clone(),
                market_price(local, user_id, instrument)?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    let positions = account
        .account
        .positions
        .iter()
        .map(|(instrument, position)| {
            Ok((
                canonical_okx_instrument_id(instrument)?,
                PortfolioPosition {
                    quantity: position.quantity,
                    price: *prices
                        .get(&canonical_okx_instrument_id(instrument)?)
                        .ok_or_else(|| "Portfolio position price is unavailable.".to_owned())?,
                },
            ))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    let state = PortfolioState {
        cash: account.account.cash,
        positions,
    };
    let risk = bundle
        .research_risk_policy
        .apply(
            &PortfolioTarget {
                decision_time_ms,
                universe_id: universe_id.into(),
                weights,
                cash_reserve,
            },
            &state,
            &instruments.iter().cloned().collect::<BTreeSet<_>>(),
        )
        .map_err(|error| error.to_string())?;
    let Some(approved_target) = risk.approved_target else {
        bots.record_evidence(
            user_id,
            bot_id,
            "risk",
            "target-rejected",
            &format!(
                "Host Risk rejected the Portfolio Target: {:?}.",
                risk.reasons
            ),
            Some(decision_id),
        )?;
        return Ok(());
    };
    let constrained = risk.decision != adaq_backtest_core::RiskDecision::Approve;
    let risk_detail = if constrained {
        format!(
            "Host Risk constrained the Portfolio Target: {:?}.",
            risk.reasons
        )
    } else {
        "Host Risk approved the Portfolio Target for execution.".into()
    };
    bots.record_evidence(
        user_id,
        bot_id,
        "risk",
        if constrained {
            "target-constrained"
        } else {
            "target-approved"
        },
        &risk_detail,
        Some(decision_id),
    )?;
    let position_value =
        state
            .positions
            .iter()
            .try_fold(Decimal::ZERO, |total, (instrument, position)| {
                position
                    .quantity
                    .checked_mul(prices[instrument])
                    .and_then(|value| total.checked_add(value))
                    .ok_or_else(|| "Portfolio equity overflowed the Decimal limit.".to_owned())
            })?;
    let equity = state
        .cash
        .checked_add(position_value)
        .ok_or_else(|| "Portfolio equity overflowed the Decimal limit.".to_owned())?;
    let mut planned = Vec::new();
    for instrument in instruments {
        let position = state
            .positions
            .get(instrument)
            .cloned()
            .unwrap_or(PortfolioPosition {
                quantity: Decimal::ZERO,
                price: prices[instrument],
            });
        let desired = equity
            .checked_mul(
                *approved_target
                    .weights
                    .get(instrument)
                    .unwrap_or(&Decimal::ZERO),
            )
            .ok_or_else(|| "Portfolio target notional overflowed the Decimal limit.".to_owned())?;
        let current = position
            .quantity
            .checked_mul(prices[instrument])
            .ok_or_else(|| {
                "Portfolio position notional overflowed the Decimal limit.".to_owned()
            })?;
        if let Some(order) = plan_spot_order(
            instrument,
            prices[instrument],
            desired,
            current,
            equity,
            account.buying_power,
            account
                .account
                .positions
                .get(okx_instrument_code(instrument))
                .map(|position| position.sellable_quantity)
                .unwrap_or_default(),
            &bundle.execution_profile,
        )? {
            planned.push(order);
        }
    }
    let had_orders = !planned.is_empty();
    planned.sort_by_key(|order| order.side != "sell");
    for order in planned {
        submit_target_order(local, bots, user_id, bot_id, bundle, &order, decision_id)?;
    }
    if !had_orders {
        bots.record_evidence(
            user_id,
            bot_id,
            "execution",
            "execution-noop",
            "Approved Portfolio Target produced no order after frozen threshold and precision checks.",
            Some(decision_id),
        )?;
    }
    Ok(())
}

fn market_price(
    local: &LocalResearchState,
    user_id: &str,
    instrument: &str,
) -> Result<Decimal, String> {
    let ticker = local
        .connections
        .with_okx_demo_client(user_id, |client| {
            tauri::async_runtime::block_on(client.fetch_ticker(
                okx_instrument_code(instrument),
                adaq_trading_crypto::Params::new(),
            ))
        })?
        .map_err(|error| error.to_string())?;
    ticker
        .last
        .or(ticker.bid)
        .or(ticker.close)
        .filter(|price| *price > Decimal::ZERO)
        .ok_or_else(|| "A positive post-decision market price is unavailable.".into())
}

fn retain_provider_order_uncertainty(
    local: &LocalResearchState,
    bots: &BotStore,
    user_id: &str,
    bot_id: &str,
    provider_order_id: &str,
    detail: &str,
) -> Result<(), String> {
    let paper_failed = local
        .paper_trading
        .mark_provider_order_uncertain(user_id, provider_order_id, adaq_bot_runtime::unix_now_ms())
        .is_err();
    let bot_failed = bots
        .record_evidence(
            user_id,
            bot_id,
            "execution",
            "provider-order-uncertain",
            detail,
            Some(provider_order_id),
        )
        .is_err();
    if paper_failed || bot_failed {
        Err("Provider uncertainty evidence could not be durably retained.".into())
    } else {
        Ok(())
    }
}

fn submit_target_order(
    local: &LocalResearchState,
    bots: &BotStore,
    user_id: &str,
    bot_id: &str,
    bundle: &BotDeploymentBundle,
    order: &PlannedSpotOrder,
    decision_id: &str,
) -> Result<(), String> {
    let operation_id =
        target_order_operation_id(bot_id, decision_id, &order.instrument, order.side)?;
    let request = PaperOrderRequest {
        user_id: user_id.into(),
        operation_id: operation_id.clone(),
        instrument: okx_instrument_code(&order.instrument).into(),
        side: order.side.into(),
        quantity: order.quantity,
        limit_price: order.limit_price,
    };
    match paper_order_dispatch::submit(
        local,
        bots,
        bot_id,
        Some(decision_id),
        &request,
        &bundle.paper_risk_policy,
        ProviderOrderKind::for_fill_policy(bundle.execution_profile.fill_policy),
    ) {
        Err(DispatchError::Begin(error)) if error.contains("RiskRejected") => {
            bots.record_evidence(
                user_id,
                bot_id,
                "risk",
                "order-risk-rejected",
                &error,
                Some(decision_id),
            )?;
            return Ok(());
        }
        Err(DispatchError::ProviderOrderIdentityMissing) => {
            Err("Provider order identity is missing; reconciliation is required.".into())
        }
        Err(DispatchError::ProviderRejected(_)) => Ok(()),
        Err(DispatchError::ProviderOutcomeUncertain(error)) => Err(format!(
            "Provider order outcome is uncertain; reconciliation is required: {}",
            bounded_text(&error, 512)
        )),
        Err(DispatchError::Begin(error)) | Err(DispatchError::OutcomeRetention(error)) => {
            Err(error)
        }
        Ok(()) => Ok(()),
    }
}

pub(crate) fn target_order_operation_id(
    bot_id: &str,
    decision_id: &str,
    instrument: &str,
    side: &str,
) -> Result<String, String> {
    Ok(format!(
        "bot-{bot_id}-order-{}",
        hash_json(&(decision_id, instrument, side))?
    ))
}

fn build_ema_bundle(
    user_id: &str,
    qualification: &StrategyQualification,
    profile_id: &str,
    account_id: &str,
    schedule: BotSchedule,
    worker: WorkerArtifactBinding,
    local: &LocalResearchState,
) -> Result<BotDeploymentBundle, String> {
    if qualification.user_id != user_id
        || qualification.candidate_id
            != crate::strategy_qualification::EMA_DOUBLE_CROSS_CANDIDATE_ID
        || !qualification.gate12_eligible
        || qualification.gate12_continuation_required
    {
        return Err("The EMA Double-Cross Qualification is not eligible.".into());
    }
    schedule.validate(
        StrategyScope::SingleInstrument,
        &qualification.context.universe_id,
    )?;
    let strategy_package = local
        .components
        .package_for_user(user_id, &qualification.package.package_archive_sha256)?;
    if !component_kind_matches(&strategy_package, ComponentKind::Strategy)
        || strategy_package.archive_sha256 != qualification.package.package_archive_sha256
        || strategy_package.manifest.wasm_sha256 != qualification.package.package_wasm_sha256
        || strategy_package.manifest.strategy_scope
            != adaq_component_tooling::StrategyScope::SingleInstrument
    {
        return Err("The EMA Double-Cross package identity is no longer exact.".into());
    }
    let bot_id = Uuid::new_v4().to_string();
    let strategy_feature_slots = feature_slot_names(&strategy_package)?;
    let strategy_parameters =
        package_parameters(&strategy_package, Some(&qualification.package.parameters))?;
    let component_hashes = vec![strategy_package.manifest.wasm_sha256.clone()];
    let is_ema_schedule = matches!(&schedule, BotSchedule::EmaDoubleCross { .. });
    if !is_ema_schedule {
        return Err("EMA Double-Cross Qualification requires its EMA schedule.".into());
    }
    let mut worker_policy = adaq_bot_runtime::WorkerRuntimePolicy::default();
    worker_policy.warmup_decisions = 0;
    let feature_plan_hash = hash_json(&serde_json::json!({
        "decisionMode": adaq_bot_runtime::ema_double_cross::EMA_DECISION_MODE,
        "fastPeriod": adaq_bot_runtime::ema_double_cross::EMA_FAST_PERIOD,
        "slowPeriod": adaq_bot_runtime::ema_double_cross::EMA_SLOW_PERIOD,
        "barIntervalMs": adaq_bot_runtime::ema_double_cross::EMA_BAR_INTERVAL_MS,
        "confirmationMs": adaq_bot_runtime::ema_double_cross::EMA_CONFIRMATION_MS,
    }))?;
    let runtime_bundle = DeploymentBundle::freeze(adaq_bot_runtime::DeploymentBundleInput {
        bot_id: bot_id.clone(),
        strategy_id: qualification.qualification_id.clone(),
        account_id: account_id.into(),
        component_hashes,
        model_hashes: Vec::new(),
        feature_plan_hash,
        risk_policy_hash: hash_json(&qualification.context.risk_policy)?,
        execution_profile_hash: hash_json(&qualification.context.execution_profile)?,
        worker_binary_hash: worker.sha256.clone(),
        qualification_evidence_hash: qualification.evidence_hash.clone(),
        decision_mode: Some(adaq_bot_runtime::ema_double_cross::EMA_DECISION_MODE.into()),
        strategy: adaq_bot_runtime::WorkerStrategyBinding {
            world: adaq_bot_runtime::StrategyWorld::Strategy,
            component_sha256: strategy_package.manifest.wasm_sha256.clone(),
            feature_slots: strategy_feature_slots,
            parameters: strategy_parameters,
        },
        pipeline: adaq_bot_runtime::WorkerPipelineBinding::default(),
        worker,
        worker_policy,
    })
    .map_err(|error| error.to_string())?;
    BotDeploymentBundle {
        schema_version: BOT_SCHEMA_VERSION.into(),
        bot_id,
        qualification_id: qualification.qualification_id.clone(),
        candidate_id: qualification.candidate_id.clone(),
        candidate_revision: qualification.candidate_revision,
        candidate_revision_hash: qualification.candidate_revision_hash.clone(),
        universe_id: qualification.context.universe_id.clone(),
        universe_snapshot_id: qualification.context.universe_snapshot_id.clone(),
        market_data_snapshot_id: qualification.context.snapshot_id.clone(),
        strategy_package_archive_sha256: qualification.package.package_archive_sha256.clone(),
        pipeline_package_archive_sha256: Vec::new(),
        account_id: account_id.into(),
        connection_profile_id: profile_id.into(),
        schedule,
        research_risk_policy: qualification.context.risk_policy.clone(),
        paper_risk_policy: PaperRiskPolicy {
            max_order_notional: EMA_ENTRY_NOTIONAL_CAP_USDT,
            reserve_cash: EMA_INITIAL_ALLOCATION_USDT - EMA_ENTRY_NOTIONAL_CAP_USDT,
            freeze_new_risk: false,
        },
        execution_profile: qualification.context.execution_profile.clone(),
        runtime_bundle,
        created_at_ms: adaq_bot_runtime::unix_now_ms(),
        identity: String::new(),
    }
    .freeze()
}

fn build_bundle(
    user_id: &str,
    qualification: &StrategyQualification,
    revision: &StrategyCandidateRevision,
    eligible: bool,
    profile_id: &str,
    account_id: &str,
    schedule: BotSchedule,
    worker: WorkerArtifactBinding,
    local: &LocalResearchState,
) -> Result<BotDeploymentBundle, String> {
    if qualification.user_id != user_id
        || !qualification.gate12_eligible
        || qualification.gate12_continuation_required
        || !eligible
        || qualification.candidate_id != revision.candidate_id
        || qualification.candidate_revision != revision.revision
        || qualification.candidate_revision_hash != revision.revision_hash
    {
        return Err(
            "The selected Strategy Qualification is not an eligible exact Revision.".into(),
        );
    }
    schedule.validate(revision.scope, &qualification.context.universe_id)?;
    let strategy_package = local
        .components
        .package_for_user(user_id, &qualification.package.package_archive_sha256)?;
    strategy_provenance_is_exact(&strategy_package, &qualification.package, revision)?;
    if strategy_package.manifest.strategy_scope
        != match revision.scope {
            StrategyScope::SingleInstrument => {
                adaq_component_tooling::StrategyScope::SingleInstrument
            }
            StrategyScope::Portfolio => adaq_component_tooling::StrategyScope::Portfolio,
        }
    {
        return Err("Strategy package scope does not match the Candidate Revision".into());
    }

    let bot_id = Uuid::new_v4().to_string();
    let strategy_feature_slots = feature_slot_names(&strategy_package)?;
    let strategy_parameters =
        package_parameters(&strategy_package, Some(&qualification.package.parameters))?;
    let mut component_hashes = vec![strategy_package.manifest.wasm_sha256.clone()];
    let mut model_hashes = Vec::new();
    let mut pipeline_archives = Vec::new();
    let mut factors = Vec::new();
    let mut models = Vec::new();
    let mut pipeline_input_slots = Vec::new();
    let mut pipeline_input_slot_set = HashSet::new();
    let mut pipeline_output_names = HashSet::new();
    let mut strategy_inputs = Vec::new();
    let mut seen_components = HashSet::new();

    for slot in &revision.definition.input_slots {
        match &slot.binding {
            StrategyInputBinding::Factor(binding) => {
                let package = local
                    .components
                    .package_for_user(user_id, &binding.package_archive_sha256)?;
                verify_input_package(
                    &package,
                    ComponentKind::Factor,
                    &binding.package_archive_sha256,
                    &binding.package_wasm_sha256,
                    &binding.component_id,
                    &binding.component_version,
                )?;
                if !package.manifest.output_names.contains(&binding.output_name) {
                    return Err("Qualified Factor output is absent from its exact package".into());
                }
                let component_hash = package.manifest.wasm_sha256.clone();
                if !seen_components.insert(component_hash.clone()) {
                    return Err("A Pipeline Component cannot be bound more than once".into());
                }
                component_hashes.push(component_hash);
                pipeline_archives.push(binding.package_archive_sha256.clone());
                let scope = package
                    .manifest
                    .factor_scope
                    .ok_or_else(|| "Factor package has no declared scope".to_owned())?;
                let feature_slots = feature_slot_names(&package)?;
                for slot_name in &feature_slots {
                    if pipeline_input_slot_set.insert(slot_name.clone()) {
                        pipeline_input_slots.push(slot_name.clone());
                    }
                }
                for output_name in &package.manifest.output_names {
                    if !pipeline_output_names.insert(output_name.clone()) {
                        return Err("Pipeline Component output names must be unique".into());
                    }
                }
                strategy_inputs.push(adaq_bot_runtime::WorkerPipelineInputBinding {
                    alias: slot.alias.clone(),
                    source: binding.output_name.clone(),
                });
                factors.push(adaq_bot_runtime::WorkerFactorBinding {
                    scope: factor_scope_name(scope),
                    component_sha256: package.manifest.wasm_sha256.clone(),
                    feature_slots,
                    output_names: package.manifest.output_names.clone(),
                    warmup_bars: u64::from(package.manifest.warmup_bars),
                    parameters: package_parameters(&package, None)?,
                });
            }
            StrategyInputBinding::Model(binding) => {
                let package = local
                    .components
                    .package_for_user(user_id, &binding.package_archive_sha256)?;
                verify_input_package(
                    &package,
                    ComponentKind::Model,
                    &binding.package_archive_sha256,
                    &binding.package_wasm_sha256,
                    &binding.component_id,
                    &binding.component_version,
                )?;
                let output_names = if package.manifest.model_outputs.is_empty() {
                    package.manifest.output_names.clone()
                } else {
                    package
                        .manifest
                        .model_outputs
                        .iter()
                        .map(|output| output.name.clone())
                        .collect()
                };
                if !output_names.contains(&binding.output_name) {
                    return Err("Qualified Model output is absent from its exact package".into());
                }
                let component_hash = package.manifest.wasm_sha256.clone();
                if !seen_components.insert(component_hash.clone()) {
                    return Err("A Pipeline Component cannot be bound more than once".into());
                }
                model_hashes.push(component_hash.clone());
                pipeline_archives.push(binding.package_archive_sha256.clone());
                let feature_slots = feature_slot_names(&package)?;
                for slot_name in &feature_slots {
                    if pipeline_input_slot_set.insert(slot_name.clone()) {
                        pipeline_input_slots.push(slot_name.clone());
                    }
                }
                for output_name in &output_names {
                    if !pipeline_output_names.insert(output_name.clone()) {
                        return Err("Pipeline Component output names must be unique".into());
                    }
                }
                strategy_inputs.push(adaq_bot_runtime::WorkerPipelineInputBinding {
                    alias: slot.alias.clone(),
                    source: binding.output_name.clone(),
                });
                models.push(adaq_bot_runtime::WorkerModelBinding {
                    component_sha256: component_hash,
                    feature_slots,
                    output_names,
                    seed: qualification.context.seed,
                    parameters: package_parameters(&package, None)?,
                });
            }
        }
    }
    pipeline_input_slots.retain(|slot| !pipeline_output_names.contains(slot));
    if component_hashes[0] != strategy_package.manifest.wasm_sha256 {
        return Err("Strategy component identity changed while preparing the Bundle".into());
    }
    let world = match revision.scope {
        StrategyScope::SingleInstrument => adaq_bot_runtime::StrategyWorld::Strategy,
        StrategyScope::Portfolio => adaq_bot_runtime::StrategyWorld::PortfolioStrategy,
    };
    let is_ema_schedule = matches!(&schedule, BotSchedule::EmaDoubleCross { .. });
    let mut worker_policy = adaq_bot_runtime::WorkerRuntimePolicy::default();
    worker_policy.warmup_decisions = if is_ema_schedule { 0 } else { 1 };
    let runtime_bundle = DeploymentBundle::freeze(adaq_bot_runtime::DeploymentBundleInput {
        bot_id: bot_id.clone(),
        strategy_id: qualification.qualification_id.clone(),
        account_id: account_id.into(),
        component_hashes,
        model_hashes,
        feature_plan_hash: revision.semantic_context.feature_plan_hash.clone(),
        risk_policy_hash: hash_json(&qualification.context.risk_policy)?,
        execution_profile_hash: hash_json(&qualification.context.execution_profile)?,
        worker_binary_hash: worker.sha256.clone(),
        qualification_evidence_hash: qualification.evidence_hash.clone(),
        decision_mode: is_ema_schedule
            .then_some(adaq_bot_runtime::ema_double_cross::EMA_DECISION_MODE.into()),
        strategy: adaq_bot_runtime::WorkerStrategyBinding {
            world,
            component_sha256: strategy_package.manifest.wasm_sha256.clone(),
            feature_slots: strategy_feature_slots,
            parameters: strategy_parameters,
        },
        pipeline: adaq_bot_runtime::WorkerPipelineBinding {
            input_slots: pipeline_input_slots,
            factors,
            models,
            strategy_inputs,
        },
        worker,
        worker_policy,
    })
    .map_err(|error| error.to_string())?;
    BotDeploymentBundle {
        schema_version: BOT_SCHEMA_VERSION.into(),
        bot_id,
        qualification_id: qualification.qualification_id.clone(),
        candidate_id: qualification.candidate_id.clone(),
        candidate_revision: qualification.candidate_revision,
        candidate_revision_hash: qualification.candidate_revision_hash.clone(),
        universe_id: qualification.context.universe_id.clone(),
        universe_snapshot_id: qualification.context.universe_snapshot_id.clone(),
        market_data_snapshot_id: qualification.context.snapshot_id.clone(),
        strategy_package_archive_sha256: qualification.package.package_archive_sha256.clone(),
        pipeline_package_archive_sha256: pipeline_archives,
        account_id: account_id.into(),
        connection_profile_id: profile_id.into(),
        schedule,
        research_risk_policy: qualification.context.risk_policy.clone(),
        paper_risk_policy: PaperRiskPolicy {
            max_order_notional: if is_ema_schedule {
                EMA_ENTRY_NOTIONAL_CAP_USDT
            } else {
                Decimal::from(100_000)
            },
            reserve_cash: if is_ema_schedule {
                EMA_INITIAL_ALLOCATION_USDT - EMA_ENTRY_NOTIONAL_CAP_USDT
            } else {
                Decimal::ZERO
            },
            freeze_new_risk: false,
        },
        execution_profile: qualification.context.execution_profile.clone(),
        runtime_bundle,
        created_at_ms: adaq_bot_runtime::unix_now_ms(),
        identity: String::new(),
    }
    .freeze()
}

fn feature_slot_names(package: &ComponentPackage) -> Result<Vec<String>, String> {
    let names = package
        .manifest
        .feature_slots
        .iter()
        .map(|slot| slot.name.clone())
        .collect::<Vec<_>>();
    if names.is_empty() || has_duplicates(&names) {
        return Err("Component feature slots must be non-empty and unique".into());
    }
    Ok(names)
}

fn verify_input_package(
    package: &ComponentPackage,
    kind: ComponentKind,
    archive_sha256: &str,
    wasm_sha256: &str,
    component_id: &str,
    component_version: &str,
) -> Result<(), String> {
    if !component_kind_matches(package, kind)
        || package.archive_sha256 != archive_sha256
        || package.manifest.wasm_sha256 != wasm_sha256
        || package.manifest.component_id.to_string() != component_id
        || package.manifest.version.to_string() != component_version
    {
        return Err(
            "Pipeline Component identity does not match the exact Candidate binding".into(),
        );
    }
    Ok(())
}

fn resolve_worker_artifact(app: &AppHandle) -> Result<WorkerArtifactFiles, String> {
    let platform = adaq_bot_runtime::current_platform_tag();
    let suffixed_name = format!("{WORKER_ARTIFACT_NAME}-{platform}");
    let plain_name = WORKER_ARTIFACT_NAME.to_owned();
    let mut candidates = Vec::new();
    if let Ok(resource_dir) = app.path().resource_dir() {
        candidates.extend([
            (
                resource_dir.join("binaries").join(&suffixed_name),
                resource_dir
                    .join("binaries")
                    .join(format!("{suffixed_name}.sig")),
            ),
            (
                resource_dir.join("binaries").join(&plain_name),
                resource_dir
                    .join("binaries")
                    .join(format!("{suffixed_name}.sig")),
            ),
            (
                resource_dir.join(&suffixed_name),
                resource_dir.join(format!("{suffixed_name}.sig")),
            ),
        ]);
    }
    if let Ok(executable) = std::env::current_exe() {
        if let Some(parent) = executable.parent() {
            candidates.extend([
                (
                    parent.join(&suffixed_name),
                    parent.join(format!("{suffixed_name}.sig")),
                ),
                (
                    parent.join(&plain_name),
                    parent.join(format!("{suffixed_name}.sig")),
                ),
            ]);
        }
    }
    let source_binaries = Path::new(env!("CARGO_MANIFEST_DIR")).join("binaries");
    candidates.push((
        source_binaries.join(&suffixed_name),
        source_binaries.join(format!("{suffixed_name}.sig")),
    ));
    for (artifact_path, signature_path) in candidates {
        if !artifact_path.is_file() || !signature_path.is_file() {
            continue;
        }
        let signature: WorkerArtifactSignature = serde_json::from_slice(
            &fs::read(&signature_path).map_err(|_| "worker-signature-unreadable")?,
        )
        .map_err(|_| "worker-signature-malformed")?;
        if signature.schema_version != WORKER_SIGNATURE_SCHEMA_VERSION {
            continue;
        }
        let binding = WorkerArtifactBinding {
            artifact_name: signature.artifact_name.clone(),
            artifact_version: signature.artifact_version.clone(),
            platform: signature.platform.clone(),
            protocol_version: signature.protocol_version.clone(),
            runtime_version: signature.runtime_version.clone(),
            sha256: signature.artifact_sha256.clone(),
            signing_key_id: signature.signing_key_id.clone(),
            signature: signature.signature.clone(),
        };
        if adaq_bot_runtime::WorkerArtifactVerifier::default()
            .verify_file(&artifact_path, &signature_path, &binding)
            .is_ok()
        {
            return Ok(WorkerArtifactFiles {
                artifact_path,
                signature_path,
                binding,
            });
        }
    }
    Err("A verified signed Bot Worker artifact is unavailable for this platform".into())
}

fn worker_launch_request(
    app: &AppHandle,
    local: &LocalResearchState,
    user_id: &str,
    bundle: &BotDeploymentBundle,
) -> Result<WorkerLaunchRequest, String> {
    bundle.verify()?;
    let artifact = resolve_worker_artifact(app)?;
    if artifact.binding != bundle.runtime_bundle.input.worker {
        return Err("The signed Worker identity changed after deployment".into());
    }
    let strategy = local
        .components
        .package_for_user(user_id, &bundle.strategy_package_archive_sha256)?;
    if strategy.manifest.wasm_sha256 != bundle.runtime_bundle.input.strategy.component_sha256 {
        return Err("The qualified Strategy package changed after deployment".into());
    }
    let mut pipeline_components = Vec::new();
    for archive in &bundle.pipeline_package_archive_sha256 {
        let package = local.components.package_for_user(user_id, archive)?;
        if package.wasm.len() > adaq_bot_runtime::MAX_COMPONENT_BYTES {
            return Err("Pipeline Component exceeds the Worker boundary limit".into());
        }
        pipeline_components.push(WorkerComponentLaunch {
            component_sha256: package.manifest.wasm_sha256.clone(),
            wasm: package.wasm,
        });
    }
    Ok(WorkerLaunchRequest {
        bundle: bundle.runtime_bundle.clone(),
        artifact_path: artifact.artifact_path,
        signature_path: artifact.signature_path,
        component_wasm: strategy.wasm,
        pipeline_components,
        extra_args: Vec::new(),
    })
}

fn reconcile_account(
    local: &LocalResearchState,
    user_id: &str,
    bundle: &BotDeploymentBundle,
) -> Result<PaperAccountView, String> {
    let profile = local
        .connections
        .list(user_id)?
        .into_iter()
        .find(|profile| profile.profile_id == bundle.connection_profile_id)
        .ok_or_else(|| "The bound OKX Demo profile is no longer available.".to_owned())?;
    if profile.provider != Provider::OkxDemo
        || profile.status != ProfileStatus::Usable
        || profile.account_id.as_deref() != Some(bundle.account_id.as_str())
    {
        return Err("The bound OKX Demo profile is not usable for this account.".into());
    }
    let now_ms = adaq_bot_runtime::unix_now_ms();
    local.paper_trading.reconcile_provider_account(
        &local.connections,
        user_id,
        &bundle.account_id,
        now_ms,
    )?;
    local.paper_trading.recover_uncertain_order_absence(
        user_id,
        now_ms,
        |instrument, window_start_ms, checked_at_ms| {
            local.connections.confirm_okx_demo_order_absence(
                user_id,
                instrument,
                window_start_ms,
                checked_at_ms,
            )
        },
    )
}

fn require_reconciled_account(
    local: &LocalResearchState,
    user_id: &str,
    bundle: &BotDeploymentBundle,
) -> Result<PaperAccountView, String> {
    let account =
        crate::paper_trading::settle_reconciliation(|| reconcile_account(local, user_id, bundle))?;
    if account.account.account_id != bundle.account_id
        || !account_is_reconciled_and_quiet(Some(&account))
    {
        return Err("Account reconciliation did not produce a quiet, exact account state.".into());
    }
    Ok(account)
}

fn transition_pair(
    supervisor: &crate::bot_supervisor::BotSupervisor,
    bots: &BotStore,
    user_id: &str,
    bot_id: &str,
    to: LifecycleState,
    reason: &str,
) -> Result<(), String> {
    if let Err(error) = supervisor.transition(user_id, bot_id, bot_id, to, "host", reason) {
        return fail_transition(supervisor, bots, user_id, bot_id, &error);
    }
    if let Err(error) = bots.transition(user_id, bot_id, to, "host", reason) {
        return fail_transition(supervisor, bots, user_id, bot_id, &error);
    }
    Ok(())
}

fn fail_transition(
    supervisor: &crate::bot_supervisor::BotSupervisor,
    _bots: &BotStore,
    user_id: &str,
    bot_id: &str,
    detail: &str,
) -> Result<(), String> {
    supervisor.fail_transition(user_id, bot_id, detail)?;
    Err(format!(
        "lifecycle-transition-failed: {}",
        safe_detail(detail)
    ))
}

fn fail_active(
    supervisor: &crate::bot_supervisor::BotSupervisor,
    _bots: &BotStore,
    user_id: &str,
    bot_id: &str,
    code: &str,
    detail: &str,
) -> Result<BotView, String> {
    supervisor.fail_active(user_id, bot_id, code, detail)?;
    Err(format!("{code}: {}", safe_detail(detail)))
}

fn start_bot(
    app: &AppHandle,
    user_id: &str,
    request: &BotCommandRequest,
    retry: bool,
) -> Result<BotView, String> {
    let bots = app.state::<Arc<BotStore>>();
    let supervisor = app.state::<Arc<crate::bot_supervisor::BotSupervisor>>();
    let local = app.state::<Arc<LocalResearchState>>();
    bots.command(
        user_id,
        &request.bot_id,
        &request.command_id,
        if retry { "retry" } else { "start" },
        |bots| {
            if local.operations.is_user_frozen(user_id)? {
                return Err("Freeze All is active; new Bot risk is blocked.".into());
            }
            let (attempt_id, bundle) = bots.begin_attempt(user_id, &request.bot_id, retry)?;
            let launch = match worker_launch_request(app, &local, user_id, &bundle) {
                Ok(launch) => launch,
                Err(error) => {
                    return fail_active(
                        &supervisor,
                        bots,
                        user_id,
                        &request.bot_id,
                        "worker-identity-invalid",
                        &error,
                    );
                }
            };
            if let Err(error) = supervisor.start(user_id, &request.bot_id, launch) {
                return fail_active(
                    &supervisor,
                    bots,
                    user_id,
                    &request.bot_id,
                    "worker-start-failed",
                    &error,
                );
            }
            if let Err(error) = transition_pair(
                &supervisor,
                bots,
                user_id,
                &request.bot_id,
                LifecycleState::Reconciling,
                "start-reconcile",
            ) {
                return fail_active(
                    &supervisor,
                    bots,
                    user_id,
                    &request.bot_id,
                    "reconcile-transition-failed",
                    &error,
                );
            }
            if let Err(error) = require_reconciled_account(&local, user_id, &bundle) {
                return fail_active(
                    &supervisor,
                    bots,
                    user_id,
                    &request.bot_id,
                    "account-reconciliation-required",
                    &error,
                );
            }
            bots.record_evidence(
                user_id,
                &request.bot_id,
                "reconciliation",
                "account-reconciled",
                "OKX Demo account evidence was refreshed before risk became available.",
                Some(&bundle.account_id),
            )?;
            if let Err(error) = observe_worker_recovery(
                &local.operations,
                user_id,
                &request.bot_id,
                &bundle,
                &attempt_id,
                false,
            ) {
                return fail_active(
                    &supervisor,
                    bots,
                    user_id,
                    &request.bot_id,
                    "worker-recovery-evidence-failed",
                    &error,
                );
            }
            if let Err(error) = local.operations.observe(crate::operations::HealthObservation {
                user_id: user_id.to_owned(),
                entity_id: request.bot_id.clone(),
                dimension: crate::operations::HealthDimension::FeatureModelStrategy,
                state: crate::operations::HealthState::Healthy,
                condition: "deployment_bundle_compatible".into(),
                evidence: serde_json::json!({
                    "botId": bundle.bot_id,
                    "bundleId": bundle.identity,
                    "qualificationId": bundle.qualification_id,
                    "candidateRevisionHash": bundle.candidate_revision_hash,
                    "marketDataSnapshotId": bundle.market_data_snapshot_id,
                    "strategyPackageArchiveSha256": bundle.strategy_package_archive_sha256,
                    "runtimeBundleId": bundle.runtime_bundle.identity,
                }),
                required: true,
                observed_at_ms: adaq_bot_runtime::unix_now_ms(),
                event_kind: Some("strategy.bundle-health".into()),
                evidence_id: Some(bundle.identity.clone()),
                correlation_id: Some(attempt_id.clone()),
                causation_id: Some(bundle.market_data_snapshot_id.clone()),
                diagnostic: Some(
                    "The immutable Deployment Bundle passed Host identity checks before risk enablement."
                        .into(),
                ),
                metrics: BTreeMap::new(),
            }) {
                return fail_active(
                    &supervisor,
                    bots,
                    user_id,
                    &request.bot_id,
                    "bundle-health-evidence-failed",
                    &error,
                );
            }
            if let Err(error) = transition_pair(
                &supervisor,
                bots,
                user_id,
                &request.bot_id,
                LifecycleState::WarmingUp,
                "warmup-start",
            ) {
                return fail_active(
                    &supervisor,
                    bots,
                    user_id,
                    &request.bot_id,
                    "warmup-transition-failed",
                    &error,
                );
            }
            bots.record_evidence(
                user_id,
                &request.bot_id,
                "lifecycle",
                "warmup-started",
                "The Worker warmup policy is active; no Target is authorized until warmup completes.",
                Some(&attempt_id),
            )?;
            if let Err(error) = transition_pair(
                &supervisor,
                bots,
                user_id,
                &request.bot_id,
                LifecycleState::Running,
                "risk-enabled-after-reconciliation",
            ) {
                return fail_active(
                    &supervisor,
                    bots,
                    user_id,
                    &request.bot_id,
                    "running-transition-failed",
                    &error,
                );
            }
            bots.get(user_id, &request.bot_id)
        },
    )
}

fn pause_bot(
    app: &AppHandle,
    user_id: &str,
    request: &BotCommandRequest,
) -> Result<BotView, String> {
    let bots = app.state::<Arc<BotStore>>();
    let supervisor = app.state::<Arc<crate::bot_supervisor::BotSupervisor>>();
    let local = app.state::<Arc<LocalResearchState>>();
    bots.command(
        user_id,
        &request.bot_id,
        &request.command_id,
        "pause",
        |bots| {
            let view = bots.get(user_id, &request.bot_id)?;
            if view.state != LifecycleState::Running {
                return Err("Pause is available only for a Running Bot.".into());
            }
            let bundle = view.bundle.clone();
            transition_pair(
                &supervisor,
                bots,
                user_id,
                &request.bot_id,
                LifecycleState::Pausing,
                "pause-requested",
            )?;
            if let Err(error) = require_reconciled_account(&local, user_id, &bundle) {
                let scope = bot_instrument_scope(&bundle);
                let detail =
                    match cancel_open_orders(&local, user_id, bots, &request.bot_id, &scope) {
                        Ok(()) => error,
                        Err(cancel_error) => format!(
                            "{error}; pending-order cancellation is unresolved: {cancel_error}"
                        ),
                    };
                return fail_active(
                    &supervisor,
                    bots,
                    user_id,
                    &request.bot_id,
                    "pause-reconciliation-required",
                    &detail,
                );
            }
            transition_pair(
                &supervisor,
                bots,
                user_id,
                &request.bot_id,
                LifecycleState::Paused,
                "pause-reconciled",
            )?;
            bots.record_evidence(
                user_id,
                &request.bot_id,
                "lifecycle",
                "paused",
                "New risk is blocked until an explicit Resume passes reconciliation and warmup.",
                None,
            )
        },
    )
}

fn resume_bot(
    app: &AppHandle,
    user_id: &str,
    request: &BotCommandRequest,
) -> Result<BotView, String> {
    let bots = app.state::<Arc<BotStore>>();
    let supervisor = app.state::<Arc<crate::bot_supervisor::BotSupervisor>>();
    let local = app.state::<Arc<LocalResearchState>>();
    bots.command(user_id, &request.bot_id, &request.command_id, "resume", |bots| {
        if local.operations.is_user_frozen(user_id)? {
            return Err("Freeze All is active; Bot Resume is blocked.".into());
        }
        let view = bots.get(user_id, &request.bot_id)?;
        if view.state != LifecycleState::Paused {
            return Err("Resume is available only for a Paused Bot.".into());
        }
        let bundle = view.bundle.clone();
        let launch = match worker_launch_request(app, &local, user_id, &bundle) {
            Ok(launch) => launch,
            Err(error) => {
                return fail_active(
                    &supervisor,
                    bots,
                    user_id,
                    &request.bot_id,
                "worker-identity-invalid",
                    &error,
                );
            }
        };
        if let Err(error) = supervisor.stop(
            user_id,
            &request.bot_id,
            &request.bot_id,
            &format!("{}:warmup-reset", request.command_id),
        ) {
            return fail_active(
                &supervisor,
                bots,
                user_id,
                &request.bot_id,
                "worker-stop-failed",
                &error,
            );
        }
        if let Err(error) = supervisor.start(user_id, &request.bot_id, launch) {
            return fail_active(
                &supervisor,
                bots,
                user_id,
                &request.bot_id,
                "worker-start-failed",
                &error,
            );
        }
        bots.record_evidence(
            user_id,
            &request.bot_id,
            "recovery",
            "worker-restarted-for-resume",
            "Resume replaced the Worker so its warmup state is fresh and pre-pause Targets cannot replay.",
            view.current_attempt_id.as_deref(),
        )?;
        transition_pair(
            &supervisor,
            bots,
            user_id,
            &request.bot_id,
            LifecycleState::Reconciling,
            "resume-reconcile",
        )?;
        if let Err(error) = require_reconciled_account(&local, user_id, &bundle) {
            return fail_active(
                &supervisor,
                bots,
                user_id,
                &request.bot_id,
                "resume-reconciliation-required",
                &error,
            );
        }
        if let Err(error) = observe_worker_recovery(
            &local.operations,
            user_id,
            &request.bot_id,
            &bundle,
            view.current_attempt_id.as_deref().unwrap_or("resume"),
            false,
        ) {
            return fail_active(
                &supervisor,
                bots,
                user_id,
                &request.bot_id,
                "worker-recovery-evidence-failed",
                &error,
            );
        }
        transition_pair(
            &supervisor,
            bots,
            user_id,
            &request.bot_id,
            LifecycleState::WarmingUp,
            "resume-warmup",
        )?;
        transition_pair(
            &supervisor,
            bots,
            user_id,
            &request.bot_id,
            LifecycleState::Running,
            "resume-risk-enabled-after-reconciliation",
        )
        .and_then(|()| bots.record_evidence(
            user_id,
            &request.bot_id,
            "lifecycle",
            "resumed",
            "Resume completed a fresh reconciliation; Worker warmup restarts and pre-pause Targets are not replayed.",
            None,
        ))
    })
}

fn observe_worker_recovery(
    operations: &crate::operations::OperationsStore,
    user_id: &str,
    bot_id: &str,
    bundle: &BotDeploymentBundle,
    attempt_id: &str,
    worker_stopped: bool,
) -> Result<(), String> {
    for (condition, event_kind) in [
        ("worker_fault", "worker.fault-recovered"),
        ("worker_lifecycle_faulted", "worker.lifecycle-recovered"),
        ("worker_decision_failed", "worker.decision-recovered"),
        ("worker_diagnostic", "worker.diagnostic-recovered"),
    ] {
        operations.observe(crate::operations::HealthObservation {
            user_id: user_id.to_owned(),
            entity_id: bot_id.to_owned(),
            dimension: crate::operations::HealthDimension::Worker,
            state: crate::operations::HealthState::Healthy,
            condition: condition.into(),
            evidence: serde_json::json!({
                "botId": bot_id,
                "attemptId": attempt_id,
                "bundleId": bundle.identity,
                "recovery": if worker_stopped {
                    "worker-stopped-and-account-reconciled"
                } else {
                    "worker-restarted-and-account-reconciled"
                },
            }),
            required: !worker_stopped,
            observed_at_ms: adaq_bot_runtime::unix_now_ms(),
            event_kind: Some(event_kind.into()),
            evidence_id: Some(attempt_id.to_owned()),
            correlation_id: Some(bundle.identity.clone()),
            causation_id: Some(bundle.market_data_snapshot_id.clone()),
            diagnostic: Some(if worker_stopped {
                format!("Host verified the Worker is absent and the stopped Attempt's exact account is reconciled; {condition} is no longer required.")
            } else {
                format!("A new Worker completed account reconciliation; {condition} recovered before risk enablement.")
            }),
            metrics: BTreeMap::new(),
        })?;
    }
    Ok(())
}

fn stop_bot(
    app: &AppHandle,
    user_id: &str,
    request: BotStopRequest,
    require_quiet_account: bool,
) -> Result<BotView, String> {
    if request.policy == BotStopPolicy::Flatten && !request.confirm_flatten {
        return Err("Stop and Flatten requires explicit confirmation.".into());
    }
    let bots = app.state::<Arc<BotStore>>();
    let supervisor = app.state::<Arc<crate::bot_supervisor::BotSupervisor>>();
    let local = app.state::<Arc<LocalResearchState>>();
    let stopped = bots.command(
        user_id,
        &request.bot_id,
        &request.command_id,
        match request.policy {
            BotStopPolicy::KeepPosition => "stop-keep-position",
            BotStopPolicy::Flatten => "stop-flatten",
        },
        |bots| {
            let view = bots.get(user_id, &request.bot_id)?;
            if !view.control.can_stop {
                return Err("Stop is unavailable for this Bot state.".into());
            }
            if is_active_state(view.state) && view.state != LifecycleState::Stopping {
                transition_pair(
                    &supervisor,
                    bots,
                    user_id,
                    &request.bot_id,
                    LifecycleState::Stopping,
                    "stop-requested",
                )?;
            }
            if is_active_state(view.state)
                && let Err(error) = supervisor.stop(
                    user_id,
                    &request.bot_id,
                    &request.bot_id,
                    &request.command_id,
                )
            {
                let scope = bot_instrument_scope(&view.bundle);
                let detail = match cancel_open_orders(
                    &local,
                    user_id,
                    bots,
                    &request.bot_id,
                    &scope,
                ) {
                    Ok(()) => error,
                    Err(cancel_error) => format!(
                        "{error}; pending-order cancellation is unresolved: {cancel_error}"
                    ),
                };
                return fail_active(
                    &supervisor,
                    bots,
                    user_id,
                    &request.bot_id,
                    "worker-stop-failed",
                    &detail,
                );
            }
            let account = match request.policy {
                BotStopPolicy::KeepPosition => {
                    let instrument_scope = bot_instrument_scope(&view.bundle);
                    cancel_open_orders(
                        &local,
                        user_id,
                        bots,
                        &request.bot_id,
                        &instrument_scope,
                    )?;
                    Some(if require_quiet_account {
                        require_reconciled_account(&local, user_id, &view.bundle)?
                    } else {
                        reconcile_account(&local, user_id, &view.bundle)?
                    })
                }
                BotStopPolicy::Flatten => match flatten_account(
                    &local,
                    user_id,
                    &view.bundle,
                    &request.command_id,
                    bots,
                    &request.bot_id,
                ) {
                    Ok(account) => Some(account),
                    Err(error) => {
                        let _ = bots.fault(
                            user_id,
                            &request.bot_id,
                            "flatten-failed",
                            &error,
                        );
                        return Err("Flatten did not produce reconciled flat evidence; the Bot is Faulted.".into());
                    }
                },
            };
            let instrument_scope = bot_instrument_scope(&view.bundle);
            let positions = account
                .as_ref()
                .map(|account| account_positions_in_scope(account, &instrument_scope))
                .unwrap_or_default();
            let reconciled = if require_quiet_account {
                account_is_reconciled_and_quiet(account.as_ref())
            } else {
                account_has_reconciled_evidence(account.as_ref())
            };
            bots.complete_stop(
                user_id,
                &request.bot_id,
                request.policy,
                positions,
                reconciled,
            )
        },
    )?;
    bots.recover_stopped_workers(
        &supervisor,
        &local.operations,
        user_id,
        local.paper_trading.view_optional(user_id)?.as_ref(),
    )?;
    Ok(stopped)
}

fn bot_instrument_scope(bundle: &BotDeploymentBundle) -> BTreeSet<String> {
    match &bundle.schedule {
        BotSchedule::ClosedBar { instrument_id, .. } => {
            [okx_instrument_code(instrument_id).to_owned()]
                .into_iter()
                .collect()
        }
        BotSchedule::EmaDoubleCross { instrument_id } => {
            [okx_instrument_code(instrument_id).to_owned()]
                .into_iter()
                .collect()
        }
        BotSchedule::ScheduledCrossSection { instruments, .. } => instruments
            .iter()
            .map(|instrument| okx_instrument_code(instrument).to_owned())
            .collect(),
    }
}

fn account_positions_in_scope(
    account: &PaperAccountView,
    instrument_scope: &BTreeSet<String>,
) -> Vec<String> {
    account
        .account
        .positions
        .iter()
        .filter(|(instrument, position)| {
            position.quantity > Decimal::ZERO && instrument_scope.contains(instrument.as_str())
        })
        .map(|(instrument, _)| instrument.clone())
        .collect()
}

fn account_has_reconciled_evidence(account: Option<&PaperAccountView>) -> bool {
    account.is_some_and(|account| {
        account.reconciliation == adaq_paper_trading_core::ReconciliationState::Reconciled
            && !account.restart_required
            && !account.provider_evidence.iter().any(|outcome| {
                matches!(
                    outcome,
                    adaq_paper_trading_core::ExecutionOutcome::Uncertain(_)
                )
            })
    })
}

pub(crate) fn account_is_reconciled_and_quiet(account: Option<&PaperAccountView>) -> bool {
    account_has_reconciled_evidence(account)
        && account.is_some_and(|account| {
            account.orders.iter().all(|order| {
                !matches!(
                    order.status,
                    adaq_paper_trading_core::OrderStatus::Accepted
                        | adaq_paper_trading_core::OrderStatus::PartiallyFilled
                )
            })
        })
}

pub(crate) fn reconciliation_resolves_fault_gate(
    account: Option<&PaperAccountView>,
    account_id: &str,
) -> bool {
    account.is_some_and(|account| {
        account.account.account_id == account_id && account_is_reconciled_and_quiet(Some(account))
    })
}

fn account_is_reconciled_for_target(
    local: &LocalResearchState,
    user_id: &str,
    bundle: &BotDeploymentBundle,
    account: &PaperAccountView,
) -> Result<bool, String> {
    if account.account.account_id != bundle.account_id
        || !account_has_reconciled_evidence(Some(account))
    {
        return Ok(false);
    }
    if account_is_reconciled_and_quiet(Some(account)) {
        return Ok(true);
    }
    if matches!(&bundle.schedule, BotSchedule::EmaDoubleCross { .. }) {
        return local.paper_experiments.pending_orders_are_experiment_owned(
            user_id,
            &bundle.bot_id,
            account,
        );
    }
    Ok(false)
}

fn bot_has_pending_order(account: &PaperAccountView, bot_id: &str, instrument_id: &str) -> bool {
    let prefix = format!("bot-{bot_id}-");
    account.provider_evidence.iter().any(|outcome| {
        let evidence = match outcome {
            adaq_paper_trading_core::ExecutionOutcome::Accepted(evidence)
            | adaq_paper_trading_core::ExecutionOutcome::Rejected(evidence)
            | adaq_paper_trading_core::ExecutionOutcome::Uncertain(evidence) => evidence,
        };
        let Some(local_order_id) = evidence.local_order_id.as_deref() else {
            return false;
        };
        evidence.operation_id.starts_with(&prefix)
            && account.orders.iter().any(|order| {
                order.order_id == local_order_id
                    && okx_instrument_code(&order.instrument) == okx_instrument_code(instrument_id)
                    && matches!(
                        order.status,
                        adaq_paper_trading_core::OrderStatus::Accepted
                            | adaq_paper_trading_core::OrderStatus::PartiallyFilled
                    )
            })
    })
}

fn cancel_open_orders(
    local: &LocalResearchState,
    user_id: &str,
    bots: &BotStore,
    bot_id: &str,
    instrument_scope: &BTreeSet<String>,
) -> Result<(), String> {
    let operation_prefix = format!("bot-{bot_id}-");
    let open_orders = local.paper_trading.provider_open_orders_for(
        user_id,
        instrument_scope,
        &operation_prefix,
    )?;
    if open_orders.is_empty() {
        return Ok(());
    }
    if open_orders
        .iter()
        .any(|order| order.provider_order_id.is_none())
    {
        local
            .paper_trading
            .require_reconciliation(user_id, adaq_bot_runtime::unix_now_ms())?;
        bots.record_evidence(
            user_id,
            bot_id,
            "execution",
            "open-order-identity-missing",
            "An eligible open order has no verified provider identity; Flatten is blocked.",
            None,
        )?;
        return Err("flatten-open-order-identity-missing".into());
    }
    for order in open_orders {
        let provider_order_id = order
            .provider_order_id
            .as_deref()
            .ok_or_else(|| "flatten-open-order-identity-missing".to_owned())?;
        if order.local_order_ids.len() != 1 || order.operation_ids.len() != 1 {
            retain_provider_order_uncertainty(
                local,
                bots,
                user_id,
                bot_id,
                provider_order_id,
                "A provider order maps to multiple local operations; Flatten is blocked.",
            )?;
            return Err("flatten-provider-order-mapping-ambiguous".into());
        }
        let remote = local.connections.cancel_okx_demo_order(
            user_id,
            &order.instrument,
            provider_order_id,
            adaq_bot_runtime::unix_now_ms(),
        );
        match remote {
            Ok(cancelled) => {
                if cancelled.id.as_deref() != Some(provider_order_id) {
                    retain_provider_order_uncertainty(
                        local,
                        bots,
                        user_id,
                        bot_id,
                        provider_order_id,
                        "Provider cancellation returned an unexpected order identity; Flatten is blocked.",
                    )?;
                    return Err("flatten-cancel-identity-mismatch".into());
                }
                let status = cancelled
                    .status
                    .as_deref()
                    .unwrap_or("canceled")
                    .to_ascii_lowercase();
                if !matches!(
                    status.as_str(),
                    "canceled" | "cancelled" | "expired" | "closed" | "filled"
                ) {
                    retain_provider_order_uncertainty(
                        local,
                        bots,
                        user_id,
                        bot_id,
                        provider_order_id,
                        "Provider cancellation did not produce a terminal order state; Flatten is blocked.",
                    )?;
                    return Err("flatten-cancel-outcome-uncertain".into());
                }
                let terminal = if matches!(status.as_str(), "canceled" | "cancelled" | "expired") {
                    match local.connections.fetch_okx_demo_order(
                        user_id,
                        &order.instrument,
                        provider_order_id,
                        adaq_bot_runtime::unix_now_ms(),
                    ) {
                        Ok(order) => order,
                        Err(_) => {
                            retain_provider_order_uncertainty(
                                local,
                                bots,
                                user_id,
                                bot_id,
                                provider_order_id,
                                "The canceled provider order could not be fetched for exact fill reconciliation; Flatten is blocked.",
                            )?;
                            return Err("flatten-cancel-reconciliation-failed".into());
                        }
                    }
                } else {
                    cancelled
                };
                if terminal.id.as_deref() != Some(provider_order_id) {
                    retain_provider_order_uncertainty(
                        local,
                        bots,
                        user_id,
                        bot_id,
                        provider_order_id,
                        "The terminal provider order returned an unexpected identity; Flatten is blocked.",
                    )?;
                    return Err("flatten-terminal-identity-mismatch".into());
                }
                let fills = match local.connections.fetch_okx_demo_order_fills(
                    user_id,
                    &order.instrument,
                    provider_order_id,
                    adaq_bot_runtime::unix_now_ms(),
                ) {
                    Ok(fills) => fills,
                    Err(_) => {
                        retain_provider_order_uncertainty(
                            local,
                            bots,
                            user_id,
                            bot_id,
                            provider_order_id,
                            "Terminal provider fills could not be fetched exactly; Flatten is blocked.",
                        )?;
                        return Err("flatten-fill-reconciliation-failed".into());
                    }
                };
                let trades = if fills.is_empty() {
                    terminal.trades.as_deref().unwrap_or(&[])
                } else {
                    fills.as_slice()
                };
                if let Err(error) = local.paper_trading.sync_provider_order_with_trades(
                    user_id,
                    &order.operation_ids[0],
                    &terminal,
                    trades,
                    adaq_bot_runtime::unix_now_ms(),
                ) {
                    retain_provider_order_uncertainty(
                        local,
                        bots,
                        user_id,
                        bot_id,
                        provider_order_id,
                        "Terminal provider fills could not be reconciled into the local ledger; Flatten is blocked.",
                    )?;
                    return Err(format!("flatten-fill-reconciliation-failed: {error}"));
                }
                let (event, detail) = if matches!(
                    status.as_str(),
                    "canceled" | "cancelled" | "expired"
                ) {
                    (
                        "open-order-canceled",
                        "Host canceled and reconciled the eligible provider order before liquidation.",
                    )
                } else {
                    (
                        "open-order-filled",
                        "Provider reported the order filled while Flatten was canceling it; Host reconciled the fills before liquidation.",
                    )
                };
                bots.record_evidence(
                    user_id,
                    bot_id,
                    "execution",
                    event,
                    detail,
                    Some(provider_order_id),
                )?;
            }
            Err(_) => {
                retain_provider_order_uncertainty(
                    local,
                    bots,
                    user_id,
                    bot_id,
                    provider_order_id,
                    "Provider cancellation outcome is uncertain; Flatten is blocked.",
                )?;
                return Err("flatten-cancel-outcome-uncertain".into());
            }
        }
    }
    Ok(())
}

fn flatten_account(
    local: &LocalResearchState,
    user_id: &str,
    bundle: &BotDeploymentBundle,
    attempt_id: &str,
    bots: &BotStore,
    bot_id: &str,
) -> Result<PaperAccountView, String> {
    let instrument_scope = bot_instrument_scope(bundle);
    let account =
        crate::paper_trading::settle_reconciliation(|| reconcile_account(local, user_id, bundle))?;
    if account.reconciliation != adaq_paper_trading_core::ReconciliationState::Reconciled {
        return Err("Flatten requires reconciled account evidence".into());
    }
    cancel_open_orders(local, user_id, bots, bot_id, &instrument_scope)?;
    let account = require_reconciled_account(local, user_id, bundle)?;
    let positions = account
        .account
        .positions
        .iter()
        .filter(|(instrument, position)| {
            instrument_scope.contains(instrument.as_str())
                && position.sellable_quantity > Decimal::ZERO
        })
        .map(|(instrument, position)| (instrument.clone(), position.sellable_quantity))
        .collect::<Vec<_>>();
    for (index, (instrument, _)) in positions.iter().enumerate() {
        let mut part = 0;
        loop {
            let account = require_reconciled_account(local, user_id, bundle)?;
            let quantity = account
                .account
                .positions
                .get(instrument)
                .map(|position| position.sellable_quantity)
                .unwrap_or_default();
            if quantity <= Decimal::ZERO {
                break;
            }
            let ticker = local
                .connections
                .with_okx_demo_client(user_id, |client| {
                    tauri::async_runtime::block_on(
                        client.fetch_ticker(instrument, adaq_trading_crypto::Params::new()),
                    )
                })?
                .map_err(|_| "flatten-price-unavailable".to_owned())?;
            let price = ticker
                .bid
                .or(ticker.last)
                .or(ticker.close)
                .ok_or_else(|| "flatten-price-unavailable".to_owned())?;
            let order_quantity = flatten_order_quantity(
                quantity,
                price,
                bundle.paper_risk_policy.max_order_notional,
                bundle.execution_profile.quantity_increment,
                bundle.execution_profile.minimum_quantity,
            )?;
            let operation_id = format!("bot-{bot_id}-flatten-{attempt_id}:{index}:{part}");
            let request = PaperOrderRequest {
                user_id: user_id.into(),
                operation_id: operation_id.clone(),
                instrument: instrument.clone(),
                side: "sell".into(),
                quantity: order_quantity,
                limit_price: price,
            };
            match paper_order_dispatch::submit(
                local,
                bots,
                bot_id,
                None,
                &request,
                &bundle.paper_risk_policy,
                ProviderOrderKind::Market,
            ) {
                Ok(()) => {}
                Err(DispatchError::ProviderOrderIdentityMissing) => {
                    return Err("flatten-provider-order-identity-missing".into());
                }
                Err(DispatchError::ProviderOutcomeUncertain(error)) => {
                    return Err(format!(
                        "flatten-provider-outcome-uncertain: {}",
                        bounded_text(&error, 512)
                    ));
                }
                Err(DispatchError::ProviderRejected(error)) => {
                    return Err(format!(
                        "flatten-provider-rejected: {}",
                        bounded_text(&error, 512)
                    ));
                }
                Err(DispatchError::Begin(error)) | Err(DispatchError::OutcomeRetention(error)) => {
                    return Err(error);
                }
            }
            let remaining = require_reconciled_account(local, user_id, bundle)?
                .account
                .positions
                .get(instrument)
                .map(|position| position.sellable_quantity)
                .unwrap_or_default();
            if remaining >= quantity {
                return Err("flatten-fill-not-observed".into());
            }
            part += 1;
        }
    }
    let final_account = require_reconciled_account(local, user_id, bundle)?;
    if !account_positions_in_scope(&final_account, &instrument_scope).is_empty()
        || !account_is_reconciled_and_quiet(Some(&final_account))
    {
        return Err("flatten-not-proven".into());
    }
    Ok(final_account)
}

fn flatten_order_quantity(
    sellable: Decimal,
    price: Decimal,
    max_order_notional: Decimal,
    quantity_increment: Decimal,
    minimum_quantity: Decimal,
) -> Result<Decimal, String> {
    if price <= Decimal::ZERO || max_order_notional <= Decimal::ZERO {
        return Err("flatten-price-or-risk-limit-invalid".into());
    }
    let cap = max_order_notional * Decimal::new(9, 1) / price;
    let quantity = floor_increment(sellable.min(cap), quantity_increment)?;
    if quantity <= Decimal::ZERO || quantity < minimum_quantity {
        return Err("flatten-position-below-minimum-quantity".into());
    }
    Ok(quantity)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use adaq_bot_runtime::{
        DeploymentBundleInput, StrategyWorld, WORKER_ARTIFACT_NAME, WORKER_ARTIFACT_VERSION,
        WORKER_PROTOCOL_VERSION, WORKER_RUNTIME_VERSION, WORKER_SIGNING_KEY_ID,
        WorkerArtifactBinding, WorkerPipelineBinding, WorkerRuntimePolicy, WorkerStrategyBinding,
    };
    use adaq_paper_trading_core::{
        AccountSnapshot, AdapterKind, Currency, ExecutionOutcome, Fill, FillEvidence, Market,
        Order, Position, ProviderEvidence, ReconciliationState,
    };

    #[test]
    fn flatten_splits_position_below_existing_order_limit() {
        let sellable = Decimal::new(12_303_113_700, 9);
        let price = Decimal::new(2_644_500, 3);
        let limit = Decimal::new(3_236_781, 2);
        let quantity = flatten_order_quantity(
            sellable,
            price,
            limit,
            Decimal::new(1, 4),
            Decimal::new(1, 4),
        )
        .unwrap();
        assert!(quantity < sellable);
        assert!(quantity * price < limit);
        assert!(
            flatten_order_quantity(
                sellable - quantity,
                price,
                limit,
                Decimal::new(1, 4),
                Decimal::new(1, 4),
            )
            .is_ok()
        );
        assert!(
            flatten_order_quantity(
                Decimal::new(7, 7),
                price,
                limit,
                Decimal::new(1, 4),
                Decimal::new(1, 4),
            )
            .is_err()
        );
    }

    fn hash(byte: char) -> String {
        std::iter::repeat(byte).take(64).collect()
    }

    #[test]
    fn ema_bot_operational_name_includes_strategy_and_instrument() {
        assert_eq!(
            BotSchedule::EmaDoubleCross {
                instrument_id: "okx:ETH-USDT".into(),
            }
            .operational_name(),
            "OKX-DEMO-BOT_EMA-Double-Cross_ETH-USDT"
        );
    }

    #[test]
    fn host_event_bar_batches_replay_warmup_once_and_emit_only_new_closed_bars() {
        let interval_ms = adaq_bot_runtime::ema_double_cross::EMA_BAR_INTERVAL_MS;
        assert_eq!(
            host_event_replay_after_ms(Some((interval_ms, 2)), 2),
            Some(interval_ms)
        );
        assert_eq!(host_event_replay_after_ms(Some((interval_ms, 2)), 3), None);
        let bar = |open_time_ms, close| OhlcvBar {
            open_time_ms,
            open: Decimal::from(close),
            high: Decimal::from(close),
            low: Decimal::from(close),
            close: Decimal::from(close),
            base_volume: Decimal::ZERO,
            quote_volume: Decimal::ZERO,
        };
        let trade = |trade_id: &str, timestamp_ms, price| MarketTrade {
            src: "okx".into(),
            code: "BTC-USDT".into(),
            trade_id: trade_id.into(),
            price: Decimal::from(price),
            quantity: Decimal::ONE,
            side: adaq_data_core::MarketTradeSide::Unknown,
            timestamp_ms,
        };
        let snapshot_bars = vec![bar(0, 10), bar(interval_ms, 11)];
        let instrument_id = "okx:BTC-USDT";
        let snapshot_id = "snapshot";

        let warmup = host_event_bar_events(
            instrument_id,
            snapshot_id,
            snapshot_bars.clone(),
            vec![
                trade("trade-1", interval_ms * 2 + 1, 12),
                trade("trade-2", interval_ms * 3 - 1, 13),
                trade("trade-3", interval_ms * 3, 14),
            ],
            interval_ms * 3,
            None,
            interval_ms,
        );
        assert_eq!(warmup.len(), 3);

        let no_closed_bar = host_event_bar_events(
            instrument_id,
            snapshot_id,
            snapshot_bars.clone(),
            vec![trade("trade-3", interval_ms * 3, 14)],
            interval_ms * 3 + 1,
            Some(interval_ms * 3),
            interval_ms,
        );
        assert!(no_closed_bar.is_empty());

        let next_bar = host_event_bar_events(
            instrument_id,
            snapshot_id,
            snapshot_bars.clone(),
            vec![
                trade("trade-3", interval_ms * 3, 14),
                trade("trade-4", interval_ms * 3 + 1, 15),
                trade("trade-5", interval_ms * 4 - 1, 16),
                trade("trade-6", interval_ms * 4, 17),
            ],
            interval_ms * 4,
            Some(interval_ms * 3 + 1),
            interval_ms,
        );
        assert_eq!(next_bar.len(), 1);
        assert!(matches!(
            &next_bar[0],
            adaq_bot_runtime::WorkerMarketEvent::BarClosed {
                bar_open_time_ms,
                close,
                replay: false,
                ..
            } if *bar_open_time_ms == interval_ms * 3 && close == "16"
        ));
    }

    pub(crate) fn bundle(bot_id: &str, account_id: &str) -> BotDeploymentBundle {
        let research_risk_policy = ResearchRiskPolicy {
            policy_id: "risk".into(),
            max_instrument_weight: Decimal::ONE,
            max_turnover: None,
        };
        let execution_profile = ExecutionProfile {
            maker_fee_rate: Decimal::ZERO,
            taker_fee_rate: Decimal::ZERO,
            adverse_slippage_rate: Decimal::ZERO,
            rebalance_threshold: Decimal::ZERO,
            price_increment: Decimal::new(1, 2),
            quantity_increment: Decimal::new(1, 8),
            minimum_quantity: Decimal::new(1, 8),
            risk_free_rate: Decimal::ZERO,
            fill_policy: adaq_backtest_core::FillPolicy::Taker,
        };
        let worker = WorkerArtifactBinding {
            artifact_name: WORKER_ARTIFACT_NAME.into(),
            artifact_version: WORKER_ARTIFACT_VERSION.into(),
            platform: adaq_bot_runtime::current_platform_tag(),
            protocol_version: WORKER_PROTOCOL_VERSION.into(),
            runtime_version: WORKER_RUNTIME_VERSION.into(),
            sha256: hash('a'),
            signing_key_id: WORKER_SIGNING_KEY_ID.into(),
            signature: "b".repeat(128),
        };
        let runtime_bundle = DeploymentBundle::freeze(DeploymentBundleInput {
            bot_id: bot_id.into(),
            strategy_id: "qualification".into(),
            account_id: account_id.into(),
            component_hashes: vec![hash('c')],
            model_hashes: vec![],
            feature_plan_hash: hash('f'),
            risk_policy_hash: hash_json(&research_risk_policy).unwrap(),
            execution_profile_hash: hash_json(&execution_profile).unwrap(),
            worker_binary_hash: hash('a'),
            qualification_evidence_hash: hash('b'),
            decision_mode: None,
            strategy: WorkerStrategyBinding {
                world: StrategyWorld::Strategy,
                component_sha256: hash('c'),
                feature_slots: vec!["close".into()],
                parameters: vec![],
            },
            pipeline: WorkerPipelineBinding::default(),
            worker,
            worker_policy: WorkerRuntimePolicy::default(),
        })
        .unwrap();
        BotDeploymentBundle {
            schema_version: BOT_SCHEMA_VERSION.into(),
            bot_id: bot_id.into(),
            qualification_id: "qualification".into(),
            candidate_id: "candidate".into(),
            candidate_revision: 1,
            candidate_revision_hash: hash('r'),
            universe_id: "universe".into(),
            universe_snapshot_id: "universe-snapshot".into(),
            market_data_snapshot_id: "snapshot".into(),
            strategy_package_archive_sha256: hash('1'),
            pipeline_package_archive_sha256: vec![],
            account_id: account_id.into(),
            connection_profile_id: "profile".into(),
            schedule: BotSchedule::ClosedBar {
                instrument_id: "BTC-USDT".into(),
                interval: "1m".into(),
            },
            research_risk_policy,
            paper_risk_policy: PaperRiskPolicy {
                max_order_notional: Decimal::from(100_000),
                reserve_cash: Decimal::ZERO,
                freeze_new_risk: false,
            },
            execution_profile,
            runtime_bundle,
            created_at_ms: 1,
            identity: String::new(),
        }
        .freeze()
        .unwrap()
    }

    fn store(database: Arc<Mutex<Connection>>) -> BotStore {
        BotStore::open(database).unwrap()
    }

    #[test]
    fn decision_commands_store_compact_idempotency_results() {
        let database = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let bots = store(database.clone());
        bots.deploy("user-a", bundle("bot-a", "account-a")).unwrap();
        bots.begin_attempt("user-a", "bot-a", false).unwrap();
        let mut action_count = 0;

        let first = bots
            .command("user-a", "bot-a", "decision-1", "decision", |store| {
                action_count += 1;
                store.record_evidence(
                    "user-a",
                    "bot-a",
                    "decision",
                    "evaluation-history",
                    &"x".repeat(20_000),
                    None,
                )
            })
            .unwrap();
        let result_bytes = database
            .lock()
            .unwrap()
            .query_row(
                "SELECT length(result_json) FROM bot_commands
                 WHERE user_id = 'user-a' AND bot_id = 'bot-a' AND command_id = 'decision-1'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert!(
            result_bytes < 64,
            "stored decision result used {result_bytes} bytes"
        );

        let duplicate = bots
            .command("user-a", "bot-a", "decision-1", "decision", |_| {
                panic!("a duplicate decision command must not run again")
            })
            .unwrap();
        assert_eq!(action_count, 1);
        assert_eq!(
            first.attempts[0].evidence.len(),
            duplicate.attempts[0].evidence.len()
        );
    }

    #[test]
    fn legacy_attempts_migrate_without_losing_evidence() {
        let database = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let bots = store(database.clone());
        bots.deploy("user-a", bundle("bot-a", "account-a")).unwrap();
        bots.begin_attempt("user-a", "bot-a", false).unwrap();
        let original = bots
            .record_evidence(
                "user-a",
                "bot-a",
                "runtime",
                "migration-check",
                &"retained evidence ".repeat(2_000),
                None,
            )
            .unwrap();
        let legacy_json = serde_json::to_string(&original.attempts).unwrap();
        {
            let database = database.lock().unwrap();
            database
                .execute(
                    "UPDATE bots SET attempts_json = ?1 WHERE user_id = ?2 AND bot_id = ?3",
                    params![legacy_json, "user-a", "bot-a"],
                )
                .unwrap();
            database
                .execute(
                    "DELETE FROM bot_runtime_attempts WHERE user_id = ?1 AND bot_id = ?2",
                    params!["user-a", "bot-a"],
                )
                .unwrap();
        }

        let reopened = store(database.clone());
        let migrated = reopened.get("user-a", "bot-a").unwrap();
        assert_eq!(
            serde_json::to_value(migrated.attempts).unwrap(),
            serde_json::to_value(original.attempts).unwrap()
        );
        let database = database.lock().unwrap();
        let stored_parent_json = database
            .query_row(
                "SELECT attempts_json FROM bots WHERE user_id = ?1 AND bot_id = ?2",
                params!["user-a", "bot-a"],
                |row| row.get::<_, String>(0),
            )
            .unwrap();
        let attempt_rows = database
            .query_row(
                "SELECT COUNT(*) FROM bot_runtime_attempts
                 WHERE user_id = ?1 AND bot_id = ?2",
                params!["user-a", "bot-a"],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(stored_parent_json, "[]");
        assert_eq!(attempt_rows, 1);
    }

    #[test]
    fn worker_recovery_resolves_prior_bot_alerts_and_keeps_other_bots_active() {
        let database = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let operations = crate::operations::OperationsStore::open(database).unwrap();
        let conditions = [
            "worker_fault",
            "worker_lifecycle_faulted",
            "worker_decision_failed",
            "worker_diagnostic",
        ];
        for (index, bot_id) in ["bot-a", "bot-b"].into_iter().enumerate() {
            for (condition_index, condition) in conditions.iter().enumerate() {
                operations
                    .observe(crate::operations::HealthObservation {
                        user_id: "user-a".into(),
                        entity_id: bot_id.into(),
                        dimension: crate::operations::HealthDimension::Worker,
                        state: if *condition == "worker_diagnostic" {
                            crate::operations::HealthState::Degraded
                        } else {
                            crate::operations::HealthState::Critical
                        },
                        condition: (*condition).into(),
                        evidence: serde_json::json!({
                            "botId": bot_id,
                            "attemptId": "attempt-old",
                        }),
                        required: true,
                        observed_at_ms: (index * conditions.len() + condition_index + 1) as i64,
                        event_kind: None,
                        evidence_id: Some("attempt-old".into()),
                        correlation_id: None,
                        causation_id: None,
                        diagnostic: Some("prior worker condition".into()),
                        metrics: BTreeMap::new(),
                    })
                    .unwrap();
            }
        }

        let bot_bundle = bundle("bot-a", "account-a");
        observe_worker_recovery(
            &operations,
            "user-a",
            "bot-a",
            &bot_bundle,
            "attempt-new",
            false,
        )
        .unwrap();

        let alerts = operations.alerts_for_user("user-a").unwrap();
        let recovered = alerts
            .iter()
            .filter(|alert| alert.entity_id == "bot-a")
            .collect::<Vec<_>>();
        assert_eq!(recovered.len(), conditions.len());
        assert!(
            recovered
                .iter()
                .all(|alert| { alert.state == crate::operations::AlertState::Resolved })
        );
        for alert in recovered {
            let history = operations
                .alert_history_for_user("user-a", &alert.alert_id)
                .unwrap();
            assert_eq!(history.len(), 2);
            assert_eq!(history[1].state, crate::operations::AlertState::Resolved);
        }
        assert!(alerts.iter().any(|alert| {
            alert.entity_id == "bot-b"
                && alert.condition == "worker_fault"
                && alert.state == crate::operations::AlertState::Active
        }));
    }

    #[test]
    fn stopped_worker_recovery_requires_exact_fresh_quiet_account_and_stopped_attempt() {
        let database = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let bots = store(database.clone());
        let operations = crate::operations::OperationsStore::open(database).unwrap();
        let supervisor =
            crate::bot_supervisor::BotSupervisor::new(operations.clone(), bots.clone()).unwrap();
        for bot_id in ["bot-a", "bot-b"] {
            bots.deploy("user-a", bundle(bot_id, bot_id)).unwrap();
            bots.begin_attempt("user-a", bot_id, false).unwrap();
            bots.fault("user-a", bot_id, "worker-fault", "failed")
                .unwrap();
            operations
                .observe(crate::operations::HealthObservation {
                    user_id: "user-a".into(),
                    entity_id: bot_id.into(),
                    dimension: crate::operations::HealthDimension::Worker,
                    state: crate::operations::HealthState::Critical,
                    condition: "worker_fault".into(),
                    evidence: serde_json::json!({"botId": bot_id}),
                    required: true,
                    observed_at_ms: 1,
                    event_kind: None,
                    evidence_id: Some(bot_id.into()),
                    correlation_id: None,
                    causation_id: None,
                    diagnostic: None,
                    metrics: BTreeMap::new(),
                })
                .unwrap();
        }
        let mut account = PaperAccountView {
            account: AccountSnapshot {
                account_id: "bot-a".into(),
                user_id: "user-a".into(),
                market: Market::OkxSpot,
                currency: Currency::Usdt,
                cash: Decimal::ONE,
                positions: BTreeMap::new(),
                observed_at_ms: adaq_bot_runtime::unix_now_ms() + 1,
            },
            reserved_cash: Decimal::ZERO,
            buying_power: Decimal::ONE,
            reconciliation: ReconciliationState::Reconciled,
            orders: vec![],
            fills: vec![],
            provider_evidence: vec![],
            order_absence_recoveries: vec![],
            risk_decisions: vec![],
            restart_required: false,
        };
        bots.recover_stopped_workers(&supervisor, &operations, "user-a", Some(&account))
            .unwrap();
        assert!(
            operations
                .alerts_for_user("user-a")
                .unwrap()
                .iter()
                .all(|a| a.state == crate::operations::AlertState::Active)
        );
        bots.complete_stop(
            "user-a",
            "bot-a",
            BotStopPolicy::KeepPosition,
            vec![],
            false,
        )
        .unwrap();
        bots.recover_stopped_workers(&supervisor, &operations, "user-a", Some(&account))
            .unwrap();
        assert!(
            operations
                .alerts_for_user("user-a")
                .unwrap()
                .iter()
                .all(|a| a.state == crate::operations::AlertState::Active)
        );
        bots.complete_stop("user-a", "bot-a", BotStopPolicy::KeepPosition, vec![], true)
            .unwrap();
        account.account.observed_at_ms = adaq_bot_runtime::unix_now_ms() + 1;
        for case in 0..4 {
            let mut invalid = account.clone();
            match case {
                0 => invalid.account.account_id = "foreign".into(),
                1 => invalid.account.observed_at_ms = 1,
                2 => invalid.restart_required = true,
                _ => invalid.reconciliation = ReconciliationState::Required,
            }
            bots.recover_stopped_workers(&supervisor, &operations, "user-a", Some(&invalid))
                .unwrap();
            assert!(
                operations
                    .alerts_for_user("user-a")
                    .unwrap()
                    .iter()
                    .all(|a| a.state == crate::operations::AlertState::Active)
            );
        }
        bots.recover_stopped_workers(&supervisor, &operations, "user-a", Some(&account))
            .unwrap();
        let alerts = operations.alerts_for_user("user-a").unwrap();
        assert_eq!(
            alerts
                .iter()
                .find(|a| a.entity_id == "bot-b")
                .unwrap()
                .state,
            crate::operations::AlertState::Active
        );
        let resolved = alerts.iter().find(|a| a.entity_id == "bot-a").unwrap();
        assert_eq!(resolved.state, crate::operations::AlertState::Resolved);
        bots.recover_stopped_workers(&supervisor, &operations, "user-a", Some(&account))
            .unwrap();
        assert_eq!(
            operations
                .alert_history_for_user("user-a", &resolved.alert_id)
                .unwrap()
                .len(),
            2
        );
        let recovery = operations.events_for_user("user-a", 1).unwrap();
        assert_eq!(
            recovery[0].evidence["recovery"],
            "worker-stopped-and-account-reconciled"
        );
    }

    #[test]
    fn legacy_decision_evidence_reads_without_runtime_feedback_fields() {
        let evidence: BotDecisionEvidence = serde_json::from_value(serde_json::json!({
            "requestId": "request",
            "decisionId": "decision",
            "outcome": "target",
            "targetHash": "hash",
            "observedAtMs": 1
        }))
        .unwrap();
        assert!(evidence.target.is_none());
        assert!(evidence.clock.is_none());
        assert!(evidence.evaluation.is_none());
    }

    #[test]
    fn feedback_binding_rejects_foreign_or_stale_runtime_identity() {
        let database = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let store = store(database);
        let deployed = store
            .deploy("user-a", bundle("bot-a", "account-a"))
            .unwrap();
        let (attempt_id, bundle) = store.begin_attempt("user-a", "bot-a", false).unwrap();
        let attempt = store
            .get("user-a", "bot-a")
            .unwrap()
            .attempts
            .into_iter()
            .find(|attempt| attempt.attempt_id == attempt_id)
            .unwrap();
        let (_, bound_attempt) = store
            .feedback_binding(
                "user-a",
                "bot-a",
                &bundle.identity,
                &attempt_id,
                attempt.created_at_ms,
                attempt.updated_at_ms,
                attempt.updated_at_ms,
            )
            .unwrap();
        assert_eq!(bound_attempt.attempt_id, attempt_id);
        assert_eq!(deployed.bundle.identity, bundle.identity);
        assert!(
            store
                .feedback_binding(
                    "user-a",
                    "bot-a",
                    "foreign-bundle",
                    &attempt_id,
                    attempt.created_at_ms,
                    attempt.updated_at_ms,
                    attempt.updated_at_ms,
                )
                .is_err()
        );
        assert!(
            store
                .feedback_binding(
                    "user-a",
                    "bot-a",
                    &bundle.identity,
                    &attempt_id,
                    attempt.created_at_ms,
                    attempt.updated_at_ms + 1,
                    attempt.updated_at_ms,
                )
                .is_err()
        );
        assert!(
            store
                .feedback_binding(
                    "user-b",
                    "bot-a",
                    &bundle.identity,
                    &attempt_id,
                    attempt.created_at_ms,
                    attempt.updated_at_ms,
                    attempt.updated_at_ms,
                )
                .is_err()
        );
    }

    #[test]
    fn feedback_binding_keeps_legacy_worker_bindings_readable() {
        let mut bundle = bundle("bot-a", "account-a");
        bundle.runtime_bundle.input.worker.protocol_version = "adaq-bot-worker-ipc@1.0.0".into();
        bundle.runtime_bundle.identity = hash_json(&bundle.runtime_bundle.input).unwrap();
        bundle.identity = hash_json(&bundle.without_identity()).unwrap();

        assert!(bundle.verify().is_err());
        assert!(bundle.verify_for_feedback().is_ok());
    }

    #[test]
    fn complete_stop_releases_lease_for_legacy_worker_binding() {
        let database = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let store = store(database.clone());
        store
            .deploy("user-a", bundle("bot-a", "account-a"))
            .unwrap();
        store.begin_attempt("user-a", "bot-a", false).unwrap();
        store
            .fault("user-a", "bot-a", "host-restart", "test")
            .unwrap();

        let mut legacy = bundle("bot-a", "account-a");
        legacy.runtime_bundle.input.worker.protocol_version = "adaq-bot-worker-ipc@1.0.0".into();
        legacy.runtime_bundle.identity = hash_json(&legacy.runtime_bundle.input).unwrap();
        legacy.identity = hash_json(&legacy.without_identity()).unwrap();

        {
            let database = database.lock().unwrap();
            database
                .execute(
                    "UPDATE bots SET bundle_json = ?1 WHERE user_id = ?2 AND bot_id = ?3",
                    rusqlite::params![serde_json::to_string(&legacy).unwrap(), "user-a", "bot-a"],
                )
                .unwrap();
        }

        let stopped = store
            .complete_stop("user-a", "bot-a", BotStopPolicy::KeepPosition, vec![], true)
            .unwrap();
        assert_eq!(stopped.state, LifecycleState::Stopped);

        let lease_count: i64 = database
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM bot_account_leases WHERE account_id = ?1",
                ["account-a"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(lease_count, 0);
    }

    #[test]
    fn durable_bot_lifecycle_is_the_host_seam() {
        let database = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let store = store(database);
        store
            .deploy("user-a", bundle("bot-a", "account-a"))
            .unwrap();
        let first = store
            .command("user-a", "bot-a", "command-a", "start", |store| {
                store.begin_attempt("user-a", "bot-a", false).unwrap();
                store
                    .transition(
                        "user-a",
                        "bot-a",
                        LifecycleState::Reconciling,
                        "host",
                        "test",
                    )
                    .unwrap();
                store
                    .transition("user-a", "bot-a", LifecycleState::WarmingUp, "host", "test")
                    .unwrap();
                store.transition("user-a", "bot-a", LifecycleState::Running, "host", "test")
            })
            .unwrap();
        let second = store
            .command("user-a", "bot-a", "command-a", "start", |_store| {
                panic!("duplicate command must not execute again")
            })
            .unwrap();
        assert_eq!(first.current_attempt_id, second.current_attempt_id);
        assert_eq!(second.state, LifecycleState::Running);
    }

    #[test]
    fn faulted_attempt_blocks_new_risk_for_its_account_only() {
        let database = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let store = store(database);
        store
            .deploy("user-a", bundle("bot-a", "account-a"))
            .unwrap();
        store
            .deploy("user-a", bundle("bot-b", "account-b"))
            .unwrap();
        store.begin_attempt("user-a", "bot-a", false).unwrap();
        store.fault("user-a", "bot-a", "test", "test").unwrap();

        assert!(
            store
                .account_blocks_new_risk("user-a", "account-a")
                .unwrap()
        );
        assert!(
            !store
                .account_blocks_new_risk("user-a", "account-b")
                .unwrap()
        );
    }

    #[test]
    fn complete_stop_accepts_an_attempt_already_in_stopping() {
        let database = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let store = store(database);
        store
            .deploy("user-a", bundle("bot-a", "account-a"))
            .unwrap();
        store.begin_attempt("user-a", "bot-a", false).unwrap();
        for state in [
            LifecycleState::Reconciling,
            LifecycleState::WarmingUp,
            LifecycleState::Running,
            LifecycleState::Stopping,
        ] {
            store
                .transition("user-a", "bot-a", state, "host", "test")
                .unwrap();
        }

        let stopped = store
            .complete_stop("user-a", "bot-a", BotStopPolicy::KeepPosition, vec![], true)
            .unwrap();

        assert_eq!(stopped.state, LifecycleState::Stopped);
    }

    #[test]
    fn lease_conflict_and_user_scope_are_fail_closed() {
        let database = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let store = store(database);
        store
            .deploy("user-a", bundle("bot-a", "shared-account"))
            .unwrap();
        store
            .deploy("user-a", bundle("bot-b", "shared-account"))
            .unwrap();
        store.begin_attempt("user-a", "bot-a", false).unwrap();
        assert!(store.begin_attempt("user-a", "bot-b", false).is_err());
        assert!(store.get("user-b", "bot-a").is_err());
    }

    #[test]
    fn okx_schedules_keep_canonical_data_ids_and_external_order_codes() {
        let schedule = canonicalize_okx_schedule(BotSchedule::ScheduledCrossSection {
            universe_id: "universe".into(),
            instruments: vec!["BTC-USDT".into(), "okx:ETH-USDT".into()],
        })
        .unwrap();

        assert_eq!(
            schedule,
            BotSchedule::ScheduledCrossSection {
                universe_id: "universe".into(),
                instruments: vec!["okx:BTC-USDT".into(), "okx:ETH-USDT".into()],
            }
        );
        assert!(schedule.contains_external_instrument("BTC-USDT"));
        assert!(schedule.contains_external_instrument("okx:ETH-USDT"));
        assert_eq!(okx_instrument_code("okx:BTC-USDT"), "BTC-USDT");
    }

    #[test]
    fn supervisor_recovery_on_construction_faults_active_attempt() {
        let database = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let bots = BotStore::open(database.clone()).unwrap();
        bots.deploy("user-a", bundle("bot-a", "account-a")).unwrap();
        bots.begin_attempt("user-a", "bot-a", false).unwrap();
        let operations = crate::operations::OperationsStore::open(database.clone()).unwrap();
        // `new` triggers host-restart recovery through the Supervisor (ADR-0048),
        // not through `BotStore::open`. The cloned handle shares the same DB so we
        // can assert on the durable state afterwards.
        let supervisor =
            crate::bot_supervisor::BotSupervisor::new(operations, bots.clone()).unwrap();
        let view = bots.get("user-a", "bot-a").unwrap();
        assert_eq!(view.state, LifecycleState::Faulted);
        assert!(view.attempts[0].reconciliation_required);
        assert!(
            view.attempts[0]
                .evidence
                .iter()
                .any(|item| item.code == "host-restart")
        );
        let _ = supervisor;
    }

    #[test]
    fn supervisor_fail_active_faults_running_bot_with_code() {
        let database = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let bots = BotStore::open(database.clone()).unwrap();
        bots.deploy("user-a", bundle("bot-b", "account-a")).unwrap();
        let operations = crate::operations::OperationsStore::open(database.clone()).unwrap();
        // No active attempt at construction, so recovery is a no-op here.
        let supervisor =
            crate::bot_supervisor::BotSupervisor::new(operations, bots.clone()).unwrap();
        // An active attempt is created and started outside the Supervisor registry,
        // following the canonical lifecycle transition table.
        bots.begin_attempt("user-a", "bot-b", false).unwrap();
        bots.transition(
            "user-a",
            "bot-b",
            LifecycleState::Reconciling,
            "host",
            "reconcile",
        )
        .unwrap();
        bots.transition(
            "user-a",
            "bot-b",
            LifecycleState::WarmingUp,
            "host",
            "warmup",
        )
        .unwrap();
        bots.transition("user-a", "bot-b", LifecycleState::Running, "host", "test")
            .unwrap();
        supervisor
            .fail_active(
                "user-a",
                "bot-b",
                "decision-deadline-missed",
                "missed window",
            )
            .unwrap();
        let view = bots.get("user-a", "bot-b").unwrap();
        assert_eq!(view.state, LifecycleState::Faulted);
        assert!(view.attempts[0].reconciliation_required);
        assert!(
            view.attempts[0]
                .evidence
                .iter()
                .any(|item| item.code == "decision-deadline-missed")
        );
    }

    #[test]
    fn decision_claims_remain_idempotent_after_projection_eviction() {
        let database = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let first = store(database.clone());
        first
            .deploy("user-a", bundle("bot-a", "account-a"))
            .unwrap();
        let (attempt_id, _) = first.begin_attempt("user-a", "bot-a", false).unwrap();
        assert_eq!(
            first
                .claim_decision(
                    "user-a",
                    "bot-a",
                    &attempt_id,
                    "request-a",
                    "decision-a",
                    Some(1),
                    false,
                )
                .unwrap(),
            DecisionClaim::New
        );
        drop(first);
        let recovered = store(database);
        assert_eq!(
            recovered
                .claim_decision(
                    "user-a",
                    "bot-a",
                    &attempt_id,
                    "request-a",
                    "decision-a",
                    Some(1),
                    false,
                )
                .unwrap(),
            DecisionClaim::Duplicate
        );
        assert_eq!(
            recovered
                .claim_decision(
                    "user-a",
                    "bot-a",
                    &attempt_id,
                    "request-b",
                    "decision-a",
                    Some(1),
                    false,
                )
                .unwrap(),
            DecisionClaim::Conflict
        );
        assert_eq!(
            recovered
                .claim_decision(
                    "user-a",
                    "bot-a",
                    &attempt_id,
                    "request-c",
                    "decision-c",
                    Some(0),
                    false,
                )
                .unwrap(),
            DecisionClaim::Stale
        );
    }

    #[test]
    fn host_validates_decision_schedule_and_precision() {
        let bot = bundle("bot-a", "account-a");
        let clock = DecisionClock::ClosedBar {
            decision_id: "decision-a".into(),
            instrument_id: "BTC-USDT".into(),
            decision_time_ms: 100,
            available_at_ms: 90,
            deadline_ms: 200,
            next_execution_ms: 201,
        };
        let input = WorkerDecisionInput::Strategy {
            instrument_id: "BTC-USDT".into(),
            frames: vec![adaq_bot_runtime::WorkerFeatureFrame {
                instrument_id: "BTC-USDT".into(),
                open_time_ms: 90,
                available_at_ms: 90,
                values: vec![Some(1.0)],
            }],
        };
        assert!(validate_decision_input(&bot, &clock, &input).is_ok());
        let mut mismatched = input.clone();
        if let WorkerDecisionInput::Strategy { instrument_id, .. } = &mut mismatched {
            *instrument_id = "ETH/USDT".into();
        }
        assert!(validate_decision_input(&bot, &clock, &mismatched).is_err());
        let incomplete = WorkerDecisionInput::Strategy {
            instrument_id: "BTC-USDT".into(),
            frames: vec![adaq_bot_runtime::WorkerFeatureFrame {
                instrument_id: "BTC-USDT".into(),
                open_time_ms: 90,
                available_at_ms: 90,
                values: vec![None],
            }],
        };
        assert!(validate_decision_input(&bot, &clock, &incomplete).is_err());

        let buy = plan_spot_order(
            "BTC-USDT",
            Decimal::from(100),
            Decimal::from(1_000),
            Decimal::ZERO,
            Decimal::from(10_000),
            Decimal::from(2_000),
            Decimal::ZERO,
            &bot.execution_profile,
        )
        .unwrap()
        .unwrap();
        assert_eq!(buy.side, "buy");
        assert_eq!(buy.quantity, Decimal::from(10));

        let sell = plan_spot_order(
            "BTC-USDT",
            Decimal::from(100),
            Decimal::ZERO,
            Decimal::from(1_000),
            Decimal::from(10_000),
            Decimal::ZERO,
            Decimal::from(10),
            &bot.execution_profile,
        )
        .unwrap()
        .unwrap();
        assert_eq!(sell.side, "sell");
        assert_eq!(sell.quantity, Decimal::from(10));
        assert_eq!(
            ProviderOrderKind::for_fill_policy(bot.execution_profile.fill_policy),
            ProviderOrderKind::Market
        );
        assert_eq!(
            ProviderOrderKind::for_fill_policy(adaq_backtest_core::FillPolicy::Maker),
            ProviderOrderKind::PostOnly
        );
    }

    #[test]
    fn portfolio_decision_cannot_fall_back_to_cached_account_without_provider_reconciliation() {
        let directory =
            std::env::temp_dir().join(format!("adaq-portfolio-account-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let local = LocalResearchState::open(&directory).unwrap();
        let snapshot = AccountSnapshot {
            account_id: "account-a".into(),
            user_id: "user-a".into(),
            market: Market::OkxSpot,
            currency: Currency::Usdt,
            cash: Decimal::from(1_000),
            positions: BTreeMap::new(),
            observed_at_ms: 1,
        };
        local
            .paper_trading
            .create_account("user-a", snapshot.clone(), 1)
            .unwrap();
        let cached = local
            .paper_trading
            .reconcile("user-a", snapshot, 2)
            .unwrap();
        assert!(account_is_reconciled_and_quiet(Some(&cached)));
        let mut bot = bundle("bot-a", "account-a");
        bot.schedule = BotSchedule::ScheduledCrossSection {
            universe_id: "universe".into(),
            instruments: vec!["okx:BTC-USDT".into()],
        };
        let input = WorkerDecisionInput::Portfolio {
            universe_id: "universe".into(),
            rows: vec![],
            state: adaq_bot_runtime::WorkerPortfolioState {
                cash: "0".into(),
                positions: vec![],
            },
        };
        let result = authoritative_decision_input(&local, "user-a", &bot, &input);
        assert!(
            result
                .is_err_and(|error| error == "The bound OKX Demo profile is no longer available.")
        );
    }

    #[test]
    fn ema_partial_position_cannot_trigger_additional_entry() {
        assert!(ema_addition_blocked(Decimal::ONE, Decimal::ONE));
        assert!(!ema_addition_blocked(Decimal::ZERO, Decimal::ONE));
        assert!(!ema_addition_blocked(Decimal::ONE, Decimal::NEGATIVE_ONE));
    }

    #[test]
    fn bot_bundle_binds_the_exact_universe_snapshot() {
        let mut bot = bundle("bot-a", "account-a");
        assert_eq!(bot.universe_id, "universe");
        assert_eq!(bot.universe_snapshot_id, "universe-snapshot");

        bot.universe_snapshot_id = "other-snapshot".into();
        assert!(bot.verify().is_err());
    }

    #[test]
    fn feature_context_binds_the_exact_universe_snapshot() {
        let bot = bundle("bot-a", "account-a");
        assert!(is_exact_feature_context(
            &bot,
            &bot.runtime_bundle.input.feature_plan_hash,
            "snapshot",
            "universe-snapshot",
        ));
        assert!(!is_exact_feature_context(
            &bot,
            &bot.runtime_bundle.input.feature_plan_hash,
            "snapshot",
            "universe",
        ));
    }

    #[test]
    fn legacy_bundle_remains_readable_for_stop_and_redeploy() {
        let mut bot = bundle("bot-a", "account-a");
        bot.schema_version = LEGACY_BOT_SCHEMA_VERSION.into();
        bot.universe_snapshot_id.clear();

        let legacy = bot.freeze().unwrap();

        assert!(legacy.verify().is_ok());
        assert!(legacy.universe_snapshot_id.is_empty());
    }

    #[test]
    fn closed_bar_clock_keeps_deadline_cursor_and_request_identity() {
        let (ctx, dir) = running_bot_context("closed-bar-clock");
        let mut view = ctx.bots.get("user-a", "bot-a").unwrap();
        let interval = adaq_data_core::BarInterval::OneMinute;
        let first = closed_bar_tick_request(&view, interval, 60_000)
            .unwrap()
            .unwrap();
        let repeated = closed_bar_tick_request(&view, interval, 60_250)
            .unwrap()
            .unwrap();
        assert_eq!(first.dataset_id, "closed-bar:60000");
        assert!(first.trade_id.is_none());
        assert_eq!(first.command_id, repeated.command_id);
        assert_eq!(first.request_id, repeated.request_id);

        view.attempts.last_mut().unwrap().last_decision_time_ms = Some(60_000);
        assert!(
            closed_bar_tick_request(&view, interval, 60_250)
                .unwrap()
                .is_none()
        );
        assert!(
            closed_bar_tick_request(&view, interval, 90_001)
                .unwrap()
                .is_none()
        );
        let next = closed_bar_tick_request(&view, interval, 120_000)
            .unwrap()
            .unwrap();
        assert_ne!(first.request_id, next.request_id);
        view.state = LifecycleState::Paused;
        assert!(
            closed_bar_tick_request(&view, interval, 120_000)
                .unwrap()
                .is_none()
        );
        drop(ctx);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn cross_section_clock_does_not_require_a_trade_event() {
        let (ctx, dir) = running_bot_context("cross-section-clock");
        let mut view = ctx.bots.get("user-a", "bot-a").unwrap();
        view.bundle.schedule = BotSchedule::ScheduledCrossSection {
            universe_id: "universe".into(),
            instruments: vec!["okx:ADA-USDT".into(), "okx:BTC-USDT".into()],
        };
        let request =
            closed_bar_tick_request(&view, adaq_data_core::BarInterval::FifteenMinutes, 900_100)
                .unwrap()
                .unwrap();
        assert_eq!(request.dataset_id, "closed-bar:900000");
        assert!(request.trade_id.is_none());
        drop(ctx);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn closed_bar_read_retry_waits_for_complete_provider_confirmation() {
        let now = std::cell::Cell::new(100);
        let reads = std::cell::Cell::new(0);
        let result = acquire_closed_bar_with_retry(
            29_000,
            || {
                reads.set(reads.get() + 1);
                if reads.get() < 3 {
                    Err(crate::local_research::RUNTIME_BARS_INCOMPLETE.into())
                } else {
                    Ok("complete-five-asset-universe")
                }
            },
            || now.get(),
            |delay| now.set(now.get() + delay.as_millis() as i64),
        );
        assert_eq!(result.unwrap(), "complete-five-asset-universe");
        assert_eq!(reads.get(), 3);
        assert_eq!(now.get(), 2_100);
    }

    #[test]
    fn closed_bar_read_retry_preserves_deadline_and_permanent_failures() {
        let now = std::cell::Cell::new(100);
        let reads = std::cell::Cell::new(0);
        let result: Result<(), String> = acquire_closed_bar_with_retry(
            2_500,
            || {
                reads.set(reads.get() + 1);
                Err(crate::local_research::RUNTIME_BARS_INCOMPLETE.into())
            },
            || now.get(),
            |delay| now.set(now.get() + delay.as_millis() as i64),
        );
        assert_eq!(
            result.unwrap_err(),
            crate::local_research::RUNTIME_BARS_INCOMPLETE
        );
        assert_eq!(reads.get(), 3);
        assert_eq!(now.get(), 2_100);
        let result: Result<(), String> = acquire_closed_bar_with_retry(
            29_000,
            || Err("foreign Universe".into()),
            || 100,
            |_| panic!("permanent validation failures must not be retried"),
        );
        assert_eq!(result.unwrap_err(), "foreign Universe");
    }

    #[test]
    fn closed_bar_read_retry_does_not_read_after_a_late_wakeup() {
        let now = std::cell::Cell::new(100);
        let reads = std::cell::Cell::new(0);
        let result: Result<(), String> = acquire_closed_bar_with_retry(
            29_000,
            || {
                reads.set(reads.get() + 1);
                Err(crate::local_research::RUNTIME_BARS_INCOMPLETE.into())
            },
            || now.get(),
            |_| now.set(30_001),
        );
        assert_eq!(
            result.unwrap_err(),
            crate::local_research::RUNTIME_BARS_INCOMPLETE
        );
        assert_eq!(reads.get(), 1);
    }

    #[test]
    fn host_schedule_rejects_future_and_late_batches() {
        assert!(host_schedule_window(101, 100).is_err());
        assert!(host_schedule_window(100, 100 + DECISION_DEADLINE_GRACE_MS + 1).is_err());
        assert_eq!(host_schedule_window(100, 100).unwrap(), (30_100, 101));
    }

    #[test]
    fn ema_bot_position_excludes_external_same_instrument_activity() {
        let mut positions = BTreeMap::new();
        positions.insert(
            "BTC-USDT".into(),
            Position {
                quantity: Decimal::new(5, 0),
                sellable_quantity: Decimal::new(5, 0),
            },
        );
        let account = PaperAccountView {
            account: AccountSnapshot {
                account_id: "account-a".into(),
                user_id: "user-a".into(),
                market: Market::OkxSpot,
                currency: Currency::Usdt,
                cash: Decimal::new(1_000, 0),
                positions,
                observed_at_ms: 1,
            },
            reserved_cash: Decimal::ZERO,
            buying_power: Decimal::new(1_000, 0),
            reconciliation: ReconciliationState::Reconciled,
            orders: vec![
                Order {
                    order_id: "own-order".into(),
                    account_id: "account-a".into(),
                    instrument: "BTC-USDT".into(),
                    side: adaq_paper_trading_core::Side::Buy,
                    quantity: Decimal::new(2, 0),
                    filled_quantity: Decimal::new(2, 0),
                    limit_price: Decimal::new(100, 0),
                    status: adaq_paper_trading_core::OrderStatus::Filled,
                    submitted_at_ms: 2,
                },
                Order {
                    order_id: "external-order".into(),
                    account_id: "account-a".into(),
                    instrument: "BTC-USDT".into(),
                    side: adaq_paper_trading_core::Side::Buy,
                    quantity: Decimal::new(3, 0),
                    filled_quantity: Decimal::new(3, 0),
                    limit_price: Decimal::new(100, 0),
                    status: adaq_paper_trading_core::OrderStatus::Filled,
                    submitted_at_ms: 2,
                },
            ],
            fills: vec![
                Fill {
                    fill_id: "own-fill".into(),
                    order_id: "own-order".into(),
                    quantity: Decimal::new(2, 0),
                    price: Decimal::new(100, 0),
                    fee: Decimal::ZERO,
                    fee_asset: Some("BTC".into()),
                    fee_quote: Some(Decimal::ZERO),
                    fee_amount: Some(Decimal::new(1, 1)),
                    evidence: FillEvidence::TradeObserved,
                    occurred_at_ms: 2,
                },
                Fill {
                    fill_id: "external-fill".into(),
                    order_id: "external-order".into(),
                    quantity: Decimal::new(3, 0),
                    price: Decimal::new(100, 0),
                    fee: Decimal::ZERO,
                    fee_asset: Some("USDT".into()),
                    fee_quote: Some(Decimal::ZERO),
                    fee_amount: Some(Decimal::ZERO),
                    evidence: FillEvidence::TradeObserved,
                    occurred_at_ms: 2,
                },
            ],
            order_absence_recoveries: vec![],
            provider_evidence: vec![ExecutionOutcome::Accepted(ProviderEvidence {
                provider: AdapterKind::OkxDemo,
                operation_id: "bot-bot-a-order-1".into(),
                local_order_id: Some("own-order".into()),
                provider_order_id: Some("provider-own".into()),
                status: "filled".into(),
                error_code: None,
                observed_at_ms: 2,
            })],
            risk_decisions: Vec::new(),
            restart_required: false,
        };

        let position = bot_owned_position(&account, "bot-a", "okx:BTC-USDT");
        assert_eq!(position.quantity, Decimal::new(19, 1));
        assert_eq!(position.sellable_quantity, Decimal::new(19, 1));

        assert!(account_is_reconciled_and_quiet(Some(&account)));
        assert!(reconciliation_resolves_fault_gate(
            Some(&account),
            "account-a"
        ));
        assert!(!reconciliation_resolves_fault_gate(
            Some(&account),
            "account-b"
        ));
        assert!(!reconciliation_resolves_fault_gate(None, "account-a"));
        let mut uncertain = account.clone();
        uncertain
            .provider_evidence
            .push(ExecutionOutcome::Uncertain(ProviderEvidence {
                provider: AdapterKind::OkxDemo,
                operation_id: "uncertain-operation".into(),
                local_order_id: None,
                provider_order_id: None,
                status: "unknown".into(),
                error_code: Some("provider-timeout".into()),
                observed_at_ms: 3,
            }));
        assert!(!account_is_reconciled_and_quiet(Some(&uncertain)));
        assert!(!reconciliation_resolves_fault_gate(
            Some(&uncertain),
            "account-a"
        ));
        let mut restarted = account;
        restarted.restart_required = true;
        assert!(!account_is_reconciled_and_quiet(Some(&restarted)));
    }

    #[cfg(feature = "local-env-credentials")]
    #[test]
    #[ignore = "explicit local-only OKX Demo Bot execution acceptance operation"]
    fn local_env_executes_frozen_ema_target_against_demo_account() -> Result<(), String> {
        if std::env::var("ADAQ_LIVE_ACCEPTANCE").as_deref() != Ok("1") {
            return Ok(());
        }
        let app_data_dir = std::env::var("ADAQ_LIVE_APP_DATA_DIR")
            .map_err(|_| "ADAQ_LIVE_APP_DATA_DIR is required".to_owned())?;
        let user_id = std::env::var("ADAQ_LIVE_USER_ID")
            .map_err(|_| "ADAQ_LIVE_USER_ID is required".to_owned())?;
        let local = LocalResearchState::open(std::path::Path::new(&app_data_dir))?;
        let bots = BotStore::open(local.database.clone())?;
        let bot = bots
            .list(&user_id)?
            .into_iter()
            .find(|bot| {
                bot.state == LifecycleState::Faulted
                    && matches!(bot.bundle.schedule, BotSchedule::EmaDoubleCross { .. })
            })
            .ok_or_else(|| "No EMA Bot is available".to_owned())?;
        let instrument_id = match &bot.bundle.schedule {
            BotSchedule::EmaDoubleCross { instrument_id } => instrument_id.clone(),
            _ => unreachable!(),
        };
        require_reconciled_account(&local, &user_id, &bot.bundle)?;
        let before = local
            .paper_trading
            .view_optional(&user_id)?
            .map(|account| account.provider_evidence.len())
            .unwrap_or_default();
        let now_ms = adaq_bot_runtime::unix_now_ms();
        let decision_id = format!("local-demo-acceptance-{now_ms}");
        let approved = ApprovedTarget {
            target: adaq_bot_runtime::WorkerTarget::Strategy {
                instrument_id: instrument_id.clone(),
                exposures: vec![adaq_bot_runtime::WorkerExposure {
                    instrument_id: instrument_id.clone(),
                    exposure: "1".into(),
                }],
            },
            decision_id: decision_id.clone(),
            produced_at_ms: now_ms,
            evaluation: adaq_bot_runtime::WorkerEvaluationEvidence {
                rows: vec![],
                events: vec![],
            },
        };
        execute_target(
            &local,
            &bots,
            &user_id,
            &bot.bot_id,
            &bot.bundle,
            &DecisionClock::TradeEvent {
                decision_id: decision_id.clone(),
                instrument_id: instrument_id.clone(),
                observation_time_ms: now_ms.saturating_sub(1),
                decision_time_ms: now_ms,
                available_at_ms: now_ms,
                deadline_ms: now_ms.saturating_add(1_000),
                next_execution_ms: now_ms.saturating_sub(1),
            },
            &approved,
        )?;
        let account = local
            .paper_trading
            .view_optional(&user_id)?
            .ok_or_else(|| "The Demo account was not retained".to_owned())?;
        assert!(account.provider_evidence.len() > before);
        assert!(account.provider_evidence.iter().any(|outcome| {
            matches!(outcome, ExecutionOutcome::Accepted(evidence) if evidence.provider_order_id.is_some())
        }));
        Ok(())
    }

    // --- BotContext: the AppHandle-free decision pipeline ---

    fn temp_workspace(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("adaq-botctx-{tag}-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn running_bot_context(tag: &str) -> (BotContext, std::path::PathBuf) {
        let database = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let bots = BotStore::open(database.clone()).unwrap();
        bots.deploy("user-a", bundle("bot-a", "account-a")).unwrap();
        // The Supervisor is constructed while the Bot is still Stopped so its
        // host-restart recovery is a no-op; the active attempt starts after.
        let operations = crate::operations::OperationsStore::open(database).unwrap();
        let bots = Arc::new(bots);
        let supervisor = Arc::new(
            crate::bot_supervisor::BotSupervisor::new(operations, (*bots).clone())
                .expect("supervisor construction must not fail for a fresh store"),
        );
        bots.begin_attempt("user-a", "bot-a", false).unwrap();
        bots.transition(
            "user-a",
            "bot-a",
            LifecycleState::Reconciling,
            "host",
            "reconcile",
        )
        .unwrap();
        bots.transition(
            "user-a",
            "bot-a",
            LifecycleState::WarmingUp,
            "host",
            "warmup",
        )
        .unwrap();
        bots.transition("user-a", "bot-a", LifecycleState::Running, "host", "test")
            .unwrap();
        let dir = temp_workspace(tag);
        let local = LocalResearchState::open(&dir).unwrap();
        (
            BotContext {
                local,
                bots,
                supervisor,
            },
            dir,
        )
    }

    fn closed_window_experiment(bot_id: &str) -> crate::paper_experiment::PaperExperiment {
        let instruments = vec![crate::paper_experiment::PaperExperimentInstrument {
            instrument: "BTC-USDT".into(),
            qualification_id: "q".into(),
            bot_id: Some(bot_id.into()),
            allocation_usdt: Decimal::ZERO,
            entry_notional_cap_usdt: Decimal::ZERO,
            reserved_cash_usdt: Decimal::ZERO,
        }];
        crate::paper_experiment::PaperExperiment {
            experiment_id: format!("exp-{bot_id}"),
            user_id: "user-a".into(),
            profile_id: "prof".into(),
            account_id: "account-a".into(),
            observation_start_ms: 100,
            observation_end_ms: 300,
            allocation_total_usdt: Decimal::ZERO,
            unallocated_remainder_usdt: Decimal::ZERO,
            starting_account_cash: None,
            started_at_ms: Some(50),
            instruments,
            state: crate::paper_experiment::PaperExperimentState::Running,
            report_id: None,
            feedback_snapshot_ids: Vec::new(),
            feedback_report_ids: Vec::new(),
            limitations: Vec::new(),
            created_at_ms: 1,
            updated_at_ms: 1,
        }
    }

    #[test]
    fn decision_blocked_by_experiment_observation_stays_running_without_worker() {
        let (ctx, dir) = running_bot_context("gate");
        ctx.local
            .paper_experiments
            .create(&closed_window_experiment("bot-a"))
            .unwrap();

        let view = run_bot_decision(
            &ctx,
            "user-a",
            BotDecisionRequest {
                bot_id: "bot-a".into(),
                command_id: "cmd-1".into(),
                request_id: "req-1".into(),
                dataset_id: "dataset".into(),
                trade_id: None,
            },
        )
        .unwrap();

        assert_eq!(view.state, LifecycleState::Running);
        assert!(
            view.attempts[0]
                .evidence
                .iter()
                .any(|item| item.code == "experiment-observation-blocked")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn decision_request_replay_is_idempotent_without_worker() {
        let (ctx, dir) = running_bot_context("replay");
        let request = |command_id: &str| BotDecisionRequest {
            bot_id: "bot-a".into(),
            command_id: command_id.into(),
            request_id: "req-1".into(),
            dataset_id: "dataset".into(),
            trade_id: None,
        };

        // First decision: no frozen feature context is available, so the Host
        // records the unavailable batch and leaves the Bot untouched.
        let first = run_bot_decision(&ctx, "user-a", request("cmd-1")).unwrap();
        assert_eq!(first.state, LifecycleState::Running);
        assert!(
            first.attempts[0]
                .evidence
                .iter()
                .any(|item| item.code == "decision-batch-unavailable")
        );

        // Replayed request identity under a new command: the claim layer
        // short-circuits instead of repeating any risk work.
        let replay = run_bot_decision(&ctx, "user-a", request("cmd-2")).unwrap();
        assert_eq!(replay.state, LifecycleState::Running);
        assert!(
            replay.attempts[0]
                .evidence
                .iter()
                .any(|item| item.code == "duplicate-decision")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
