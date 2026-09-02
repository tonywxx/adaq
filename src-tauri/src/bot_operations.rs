//! Host-owned Bot deployment records and lifecycle control.
//!
//! SQLite stores one immutable Bundle per Bot, separate Runtime Attempts,
//! command effects, and the account lease. Provider credentials and Worker
//! processes stay behind their existing Host-only seams.

use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use adaq_backtest_core::{
    ExecutionProfile, PortfolioPosition, PortfolioState, PortfolioTarget,
    RiskPolicy as ResearchRiskPolicy,
};
use adaq_bot_runtime::{
    DecisionClock, DeploymentBundle, LifecycleState, RuntimeEvent, WORKER_ARTIFACT_NAME,
    WORKER_SIGNATURE_SCHEMA_VERSION, WorkerArtifactBinding, WorkerArtifactSignature,
    WorkerComponentLaunch, WorkerDecisionInput, WorkerDecisionResult, WorkerLaunchRequest,
    WorkerTarget,
};
use adaq_component_tooling::{ComponentKind, ComponentPackage, FactorScope, ParameterType};
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
}

impl BotSchedule {
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
        }
        Ok(())
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

    fn without_identity(&self) -> Self {
        let mut copy = self.clone();
        copy.identity.clear();
        copy
    }

    fn verify_contents(&self) -> Result<(), String> {
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
        self.runtime_bundle
            .verify()
            .map_err(|error| error.to_string())?;
        self.schedule
            .validate(runtime_scope(&self.runtime_bundle), &self.universe_id)?;
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
    pub target_hash: Option<String>,
    pub observed_at_ms: i64,
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
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
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
                );",
            )
            .map_err(|error| error.to_string())?;
        store.recover_after_restart()?;
        Ok(store)
    }

    fn recover_after_restart(&self) -> Result<(), String> {
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
            let mut bot = row.decode()?;
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
        statement
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
            .map(|row| {
                row.map_err(|error| error.to_string())
                    .and_then(|row| row.decode().map(|bot| bot.view()))
            })
            .collect()
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
        bot.bundle.verify()?;
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
            return serde_json::from_str(&result_json).map_err(|error| error.to_string())?;
        }
        self.load_record(user_id, bot_id)?;
        let result = action(self);
        let result_json = serde_json::to_string(&result).map_err(|error| error.to_string())?;
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
            if !valid_transition(attempt.state, to) {
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
        self.mutate(user_id, bot_id, |bot| {
            let attempt = current_attempt_mut(bot)?;
            if attempt.state != LifecycleState::Faulted && attempt.state != LifecycleState::Stopped
            {
                let from = attempt.state;
                attempt.state = LifecycleState::Faulted;
                attempt.events.push(RuntimeEvent {
                    from,
                    to: LifecycleState::Faulted,
                    actor: "host".into(),
                    reason: safe_code(code),
                });
            }
            attempt.reconciliation_required = true;
            let now = adaq_bot_runtime::unix_now_ms();
            let attempt_id = attempt.attempt_id.clone();
            push_evidence(
                attempt,
                "recovery",
                &safe_code(code),
                &safe_detail(detail),
                Some(&attempt_id),
                now,
            );
            attempt.updated_at_ms = now;
            bot.state = attempt.state;
            bot.updated_at_ms = now;
            Ok(())
        })
    }

    pub(crate) fn record_worker_fault(
        &self,
        user_id: &str,
        bot_id: &str,
        code: &str,
        detail: &str,
    ) -> Result<(), String> {
        let _ = self.fault(user_id, bot_id, code, detail)?;
        Ok(())
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

    pub(crate) fn record_decision(
        &self,
        user_id: &str,
        bot_id: &str,
        result: &WorkerDecisionResult,
    ) -> Result<BotView, String> {
        self.mutate(user_id, bot_id, |bot| {
            let attempt = current_attempt_mut(bot)?;
            let (request_id, decision_id, outcome, target_hash) = match result {
                WorkerDecisionResult::Target {
                    request_id,
                    decision_id,
                    target,
                    ..
                } => (request_id, decision_id, "target", Some(hash_json(target)?)),
                WorkerDecisionResult::NoTarget {
                    request_id,
                    decision_id,
                    ..
                } => (request_id, decision_id, "no-target", None),
            };
            attempt.decisions.push(BotDecisionEvidence {
                request_id: bounded_text(request_id, 128),
                decision_id: bounded_text(decision_id, 128),
                outcome: outcome.into(),
                target_hash,
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
                .is_some_and(|last_decision_time_ms| decision_time_ms <= last_decision_time_ms)
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
        self.mutate(user_id, bot_id, |bot| {
            let attempt = current_attempt_mut(bot)?;
            let now = adaq_bot_runtime::unix_now_ms();
            if is_active_state(attempt.state) && attempt.state != LifecycleState::Stopping {
                let from = attempt.state;
                if !valid_transition(from, LifecycleState::Stopping) {
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
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn mutate(
        &self,
        user_id: &str,
        bot_id: &str,
        action: impl FnOnce(&mut PersistedBot) -> Result<(), String>,
    ) -> Result<BotView, String> {
        let mut database = self.database.lock().map_err(|error| error.to_string())?;
        let mut bot = self.load_record_locked(&database, user_id, bot_id)?;
        action(&mut bot)?;
        self.save_record_locked(&mut database, &bot)?;
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
            .decode()
    }

    fn insert_record_locked(
        &self,
        database: &mut Connection,
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
                    serde_json::to_string(&bot.attempts).map_err(|error| error.to_string())?,
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
        bot.bundle.verify()?;
        if bot.attempts.len() > MAX_ATTEMPTS
            || bot.attempts.iter().any(|attempt| {
                attempt.evidence.len() > MAX_EVIDENCE
                    || attempt.decisions.len() > MAX_DECISIONS
                    || attempt.orders.len() > MAX_ORDERS
            })
        {
            return Err("Bot evidence exceeds the bounded retention limit".into());
        }
        database
            .execute(
                "UPDATE bots SET bundle_json = ?1, state = ?2, current_attempt_id = ?3,
                    attempts_json = ?4, updated_at_ms = ?5
                 WHERE user_id = ?6 AND bot_id = ?7",
                params![
                    serde_json::to_string(&bot.bundle).map_err(|error| error.to_string())?,
                    state_json(bot.state)?,
                    bot.current_attempt_id,
                    serde_json::to_string(&bot.attempts).map_err(|error| error.to_string())?,
                    bot.updated_at_ms,
                    bot.user_id,
                    bot.bot_id,
                ],
            )
            .map(|_| ())
            .map_err(|error| error.to_string())
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
                "SELECT COUNT(*) FROM bot_account_leases WHERE user_id = ?1",
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
    fn decode(self) -> Result<PersistedBot, String> {
        Ok(PersistedBot {
            bot_id: self.bot_id,
            user_id: self.user_id,
            bundle: serde_json::from_str(&self.bundle_json).map_err(|error| error.to_string())?,
            state: serde_json::from_str(&self.state).map_err(|error| error.to_string())?,
            current_attempt_id: self.current_attempt_id,
            attempts: serde_json::from_str(&self.attempts_json)
                .map_err(|error| error.to_string())?,
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

fn valid_transition(from: LifecycleState, to: LifecycleState) -> bool {
    matches!(
        (from, to),
        (
            LifecycleState::Starting,
            LifecycleState::Reconciling | LifecycleState::Faulted
        ) | (
            LifecycleState::Reconciling,
            LifecycleState::WarmingUp | LifecycleState::Faulted
        ) | (
            LifecycleState::WarmingUp,
            LifecycleState::Running | LifecycleState::Faulted
        ) | (
            LifecycleState::Running,
            LifecycleState::Pausing | LifecycleState::Stopping | LifecycleState::Faulted
        ) | (
            LifecycleState::Pausing,
            LifecycleState::Paused | LifecycleState::Faulted
        ) | (
            LifecycleState::Paused,
            LifecycleState::Reconciling | LifecycleState::Stopping | LifecycleState::Faulted
        ) | (
            LifecycleState::Stopping,
            LifecycleState::Stopped | LifecycleState::Faulted
        )
    )
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
}

struct HostDecisionBatch {
    clock: DecisionClock,
    input: WorkerDecisionInput,
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
        let (revision, eligible) = candidates.revision_for_user(
            &user_id,
            &qualification.candidate_id,
            qualification.candidate_revision,
        )?;
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
        let bundle = build_bundle(
            &user_id,
            &qualification,
            &revision,
            eligible,
            &request.profile_id,
            &request.account_id,
            request.schedule,
            artifact.binding,
            &local,
        )?;
        bots.deploy(&user_id, bundle)
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
    tauri::async_runtime::spawn_blocking(move || start_bot(&app, &user_id, &request, false))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn bot_retry(
    request: BotCommandRequest,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    app: AppHandle,
) -> Result<BotView, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    tauri::async_runtime::spawn_blocking(move || start_bot(&app, &user_id, &request, true))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn bot_pause(
    request: BotCommandRequest,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    app: AppHandle,
) -> Result<BotView, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    tauri::async_runtime::spawn_blocking(move || pause_bot(&app, &user_id, &request))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn bot_resume(
    request: BotCommandRequest,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    app: AppHandle,
) -> Result<BotView, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    tauri::async_runtime::spawn_blocking(move || resume_bot(&app, &user_id, &request))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn bot_stop(
    request: BotStopRequest,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    app: AppHandle,
) -> Result<BotView, String> {
    let user_id = auth.user_id_for_window(window.label())?;
    tauri::async_runtime::spawn_blocking(move || stop_bot(&app, &user_id, request))
        .await
        .map_err(|error| error.to_string())?
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
        let local = app.state::<Arc<LocalResearchState>>();
        let bots = app.state::<Arc<BotStore>>();
        let supervisor = app.state::<Arc<crate::bot_supervisor::BotSupervisor>>();
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
                let validation: Result<(), String> = if !bounded(&request.request_id, 128)
                    || !bounded(&request.dataset_id, 128)
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
                let host_batch = host_decision_batch(
                    &local,
                    &user_id,
                    &view.bundle,
                    &request.dataset_id,
                );
                if let Err(error) = &host_batch {
                    local.operations.observe(crate::operations::HealthObservation {
                        user_id: user_id.clone(),
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
                } else {
                    local.operations.observe(crate::operations::HealthObservation {
                        user_id: user_id.clone(),
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
                if host_batch.is_ok() && local.operations.blocks_new_risk(&user_id)? {
                    bots.record_evidence(
                        &user_id,
                        &request.bot_id,
                        "decision",
                        "decision-skipped-by-operations",
                        "An unresolved Host safety action blocked this Decision Batch; no Worker Target or order was authorized.",
                        Some(&request.request_id),
                    )?;
                    return bots.get(&user_id, &request.bot_id);
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
                let HostDecisionBatch { clock, input: host_input } = match host_batch {
                    Ok(batch) => batch,
                    Err(error) => {
                        let result = WorkerDecisionResult::NoTarget {
                            request_id: request.request_id.clone(),
                            decision_id: decision_id.clone(),
                            reason: adaq_bot_runtime::NoTargetReason::MissingInput,
                            detail: safe_detail(&error),
                        };
                        bots.record_decision(&user_id, &request.bot_id, &result)?;
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
                    };
                    bots.record_decision(&user_id, &request.bot_id, &result)?;
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
                        };
                        bots.record_decision(&user_id, &request.bot_id, &result)?;
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
                        ref target,
                        ref decision_id,
                        ref produced_at_ms,
                        ..
                    }) => {
                        if let Err(error) = clock.accepts_target(decision_id, *produced_at_ms)
                        {
                            let _ = bots.fault(
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
                        bots.record_decision(&user_id, &request.bot_id, &result)?;
                        bots.record_evidence(
                            &user_id,
                            &request.bot_id,
                            "lifecycle",
                            "warmup-complete",
                            "The Worker produced its first Target after the frozen warmup policy; Host validation still gates execution.",
                            Some(decision_id),
                        )?;
                        if let Err(error) = execute_target(
                            &local,
                            bots,
                            &user_id,
                            &request.bot_id,
                            &view.bundle,
                            &clock,
                            decision_id,
                            target,
                        ) {
                            let _ = supervisor.stop(
                                &user_id,
                                &request.bot_id,
                                &request.bot_id,
                                &request.command_id,
                            );
                            let _ = bots.fault(
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
                        reason: adaq_bot_runtime::NoTargetReason::DeadlineMissed,
                        ..
                    }) => {
                        bots.record_decision(&user_id, &request.bot_id, result)?;
                        let _ = bots.fault(
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
                        bots.record_decision(&user_id, &request.bot_id, result)?;
                        bots.record_evidence(
                            &user_id,
                            &request.bot_id,
                            "lifecycle",
                            "warmup-progress",
                            "The Worker consumed the Decision Batch for warmup; no Target or order was authorized.",
                            Some(clock.decision_id()),
                        )
                    }
                    Ok(result) => bots.record_decision(&user_id, &request.bot_id, &result),
                    Err(error) => {
                        let _ =
                            bots.fault(&user_id, &request.bot_id, "worker-decision-failed", &error);
                        Err(
                            "Worker Decision failed; the Bot is Faulted and requires recovery."
                                .into(),
                        )
                    }
                }
            },
        )
    })
    .await
    .map_err(|error| error.to_string())?
}

fn host_decision_batch(
    local: &LocalResearchState,
    user_id: &str,
    bundle: &BotDeploymentBundle,
    dataset_id: &str,
) -> Result<HostDecisionBatch, String> {
    let clock = host_schedule_clock(local, user_id, bundle, dataset_id)?;
    let input = host_decision_input(local, user_id, bundle, dataset_id, &clock)?;
    Ok(HostDecisionBatch { clock, input })
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
) -> Result<DecisionClock, String> {
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
    }
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
    clock: &DecisionClock,
) -> Result<WorkerDecisionInput, String> {
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
    }
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
        _ => Err("Decision Batch does not match the immutable Bot schedule.".into()),
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
    let account = local.paper_trading.view_optional(user_id)?.ok_or_else(|| {
        "A reconciled OKX Demo account is required before a Decision Batch.".to_owned()
    })?;
    if account.account.account_id != bundle.account_id
        || !account_is_reconciled_and_quiet(Some(&account))
    {
        return Err("Portfolio state is stale, uncertain, or bound to another account.".into());
    }
    if account
        .account
        .positions
        .keys()
        .any(|instrument| !instruments.contains(instrument))
    {
        return Err("Unowned account exposure prevents a Portfolio Decision Batch.".into());
    }
    let positions = account
        .account
        .positions
        .iter()
        .map(|(instrument, position)| {
            Ok(adaq_bot_runtime::WorkerPosition {
                instrument_id: instrument.clone(),
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

fn execute_target(
    local: &LocalResearchState,
    bots: &BotStore,
    user_id: &str,
    bot_id: &str,
    bundle: &BotDeploymentBundle,
    clock: &DecisionClock,
    decision_id: &str,
    target: &WorkerTarget,
) -> Result<(), String> {
    if local.operations.blocks_new_risk(user_id)? {
        return Err("A Host operational safety action is active; new Bot risk is blocked.".into());
    }
    let next_execution_ms = match clock {
        DecisionClock::ClosedBar {
            next_execution_ms, ..
        }
        | DecisionClock::ScheduledCrossSection {
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
    if account.account.account_id != bundle.account_id
        || !account_is_reconciled_and_quiet(Some(&account))
    {
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
    if !matches!(
        &bundle.schedule,
        BotSchedule::ClosedBar { instrument_id: scheduled, .. } if scheduled == instrument_id
    ) || exposures.is_empty()
        || !exposures.iter().all(|exposure| {
            exposure.instrument_id == instrument_id
                && adaq_bot_runtime::is_decimal_text(&exposure.exposure)
        })
    {
        return Err("Strategy Target does not match the immutable ClosedBar schedule.".into());
    }
    if account
        .account
        .positions
        .keys()
        .any(|instrument| instrument != instrument_id)
    {
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
    let position = account
        .account
        .positions
        .get(instrument_id)
        .cloned()
        .unwrap_or(adaq_paper_trading_core::Position {
            quantity: Decimal::ZERO,
            sellable_quantity: Decimal::ZERO,
        });
    let equity =
        account
            .account
            .cash
            .checked_add(position.quantity.checked_mul(price).ok_or_else(|| {
                "Strategy account equity overflowed the Decimal limit.".to_owned()
            })?)
            .ok_or_else(|| "Strategy account equity overflowed the Decimal limit.".to_owned())?;
    let desired = equity
        .checked_mul(approved)
        .ok_or_else(|| "Strategy target notional overflowed the Decimal limit.".to_owned())?;
    let current = position
        .quantity
        .checked_mul(price)
        .ok_or_else(|| "Strategy position notional overflowed the Decimal limit.".to_owned())?;
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
    if account
        .account
        .positions
        .keys()
        .any(|instrument| !instruments.contains(instrument))
    {
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
                instrument.clone(),
                PortfolioPosition {
                    quantity: position.quantity,
                    price: *prices
                        .get(instrument)
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
                .get(instrument)
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
            tauri::async_runtime::block_on(
                client.fetch_ticker(instrument, adaq_trading_crypto::Params::new()),
            )
        })?
        .map_err(|error| error.to_string())?;
    ticker
        .last
        .or(ticker.bid)
        .or(ticker.close)
        .filter(|price| *price > Decimal::ZERO)
        .ok_or_else(|| "A positive post-decision market price is unavailable.".into())
}

fn retain_uncertain_order(
    local: &LocalResearchState,
    bots: &BotStore,
    user_id: &str,
    bot_id: &str,
    operation_id: &str,
    decision_id: Option<&str>,
) -> Result<(), String> {
    let now_ms = adaq_bot_runtime::unix_now_ms();
    let paper_failed = local
        .paper_trading
        .mark_uncertain(user_id, operation_id, now_ms)
        .is_err();
    let bot_failed = bots
        .record_order(
            user_id,
            bot_id,
            operation_id,
            decision_id,
            "uncertain",
            None,
        )
        .is_err();
    if paper_failed || bot_failed {
        Err("Provider uncertainty evidence could not be durably retained.".into())
    } else {
        Ok(())
    }
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
    let operation_id = format!(
        "bot-{bot_id}-order-{}",
        hash_json(&(decision_id, order.instrument.as_str(), order.side))?
    );
    let request = PaperOrderRequest {
        user_id: user_id.into(),
        operation_id: operation_id.clone(),
        instrument: order.instrument.clone(),
        side: order.side.into(),
        quantity: order.quantity,
        limit_price: order.limit_price,
    };
    if let Err(error) = local.paper_trading.begin_order_with_policy(
        &request,
        Some(&bundle.paper_risk_policy),
        adaq_bot_runtime::unix_now_ms(),
    ) {
        if error.contains("RiskRejected") {
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
        return Err(error);
    }
    let quantity = order.quantity.to_string();
    let price = order.limit_price.to_string();
    let remote = local.connections.create_okx_demo_order(
        user_id,
        &order.instrument,
        "limit",
        order.side,
        &quantity,
        Some(&price),
        adaq_bot_runtime::unix_now_ms(),
    );
    match remote {
        Ok(provider_order) => {
            let Some(provider_order_id) = provider_order.id.clone() else {
                retain_uncertain_order(
                    local,
                    bots,
                    user_id,
                    bot_id,
                    &operation_id,
                    Some(decision_id),
                )?;
                return Err(
                    "Provider order identity is missing; reconciliation is required.".into(),
                );
            };
            let status = provider_order.status.as_deref().unwrap_or("accepted");
            local.paper_trading.record_order_result(
                user_id,
                &operation_id,
                Some(provider_order_id.clone()),
                status,
                None,
                adaq_bot_runtime::unix_now_ms(),
            )?;
            bots.record_order(
                user_id,
                bot_id,
                &operation_id,
                Some(decision_id),
                status,
                Some(&provider_order_id),
            )?;
            Ok(())
        }
        Err(_) => {
            retain_uncertain_order(
                local,
                bots,
                user_id,
                bot_id,
                &operation_id,
                Some(decision_id),
            )?;
            Err("Provider order outcome is uncertain; reconciliation is required.".into())
        }
    }
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
                factors.push(adaq_bot_runtime::WorkerFactorBinding {
                    scope: factor_scope_name(scope),
                    component_sha256: package.manifest.wasm_sha256.clone(),
                    feature_slots: feature_slot_names(&package)?,
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
                models.push(adaq_bot_runtime::WorkerModelBinding {
                    component_sha256: component_hash,
                    feature_slots: feature_slot_names(&package)?,
                    output_names,
                    seed: qualification.context.seed,
                    parameters: package_parameters(&package, None)?,
                });
            }
        }
    }
    if component_hashes[0] != strategy_package.manifest.wasm_sha256 {
        return Err("Strategy component identity changed while preparing the Bundle".into());
    }
    let world = match revision.scope {
        StrategyScope::SingleInstrument => adaq_bot_runtime::StrategyWorld::Strategy,
        StrategyScope::Portfolio => adaq_bot_runtime::StrategyWorld::PortfolioStrategy,
    };
    let mut worker_policy = adaq_bot_runtime::WorkerRuntimePolicy::default();
    worker_policy.warmup_decisions = 1;
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
        strategy: adaq_bot_runtime::WorkerStrategyBinding {
            world,
            component_sha256: strategy_package.manifest.wasm_sha256.clone(),
            feature_slots: strategy_feature_slots,
            parameters: strategy_parameters,
        },
        pipeline: adaq_bot_runtime::WorkerPipelineBinding {
            input_slots: revision
                .definition
                .input_slots
                .iter()
                .map(|slot| slot.alias.clone())
                .collect(),
            factors,
            models,
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
            max_order_notional: Decimal::from(100_000),
            reserve_cash: Decimal::ZERO,
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
    local
        .connections
        .with_okx_demo_reconciliation(user_id, now_ms, |open_orders, client| {
            local.paper_trading.provider_balance(
                user_id,
                bundle.account_id.clone(),
                open_orders,
                client,
                now_ms,
            )
        })?
}

fn require_reconciled_account(
    local: &LocalResearchState,
    user_id: &str,
    bundle: &BotDeploymentBundle,
) -> Result<PaperAccountView, String> {
    let account = reconcile_account(local, user_id, bundle)?;
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
    supervisor.transition(user_id, bot_id, bot_id, to, "host", reason)?;
    bots.transition(user_id, bot_id, to, "host", reason)?;
    Ok(())
}

fn fail_active(
    supervisor: &crate::bot_supervisor::BotSupervisor,
    bots: &BotStore,
    user_id: &str,
    bot_id: &str,
    attempt_id: &str,
    code: &str,
    detail: &str,
) -> Result<BotView, String> {
    let _ = supervisor.stop(user_id, bot_id, bot_id, &format!("{attempt_id}:fault"));
    let _ = bots.fault(user_id, bot_id, code, detail);
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
                        &attempt_id,
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
                    &attempt_id,
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
                    &attempt_id,
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
                    &attempt_id,
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
                    &attempt_id,
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
                    &attempt_id,
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
                    &attempt_id,
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
            if require_reconciled_account(&local, user_id, &bundle).is_err() {
                let _ = supervisor.transition(
                    user_id,
                    &request.bot_id,
                    &request.bot_id,
                    LifecycleState::Faulted,
                    "host",
                    "pause-reconciliation-required",
                );
                let _ = bots.fault(
                    user_id,
                    &request.bot_id,
                    "pause-reconciliation-required",
                    "Pending or unreconciled account evidence prevents a safe Pause.",
                );
                let _ = supervisor.stop(
                    user_id,
                    &request.bot_id,
                    &request.bot_id,
                    &request.command_id,
                );
                return Err(
                    "Pause requires reconciled account evidence and no pending orders.".into(),
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
                    view.current_attempt_id.as_deref().unwrap_or("resume"),
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
                view.current_attempt_id.as_deref().unwrap_or("resume"),
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
                view.current_attempt_id.as_deref().unwrap_or("resume"),
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
                view.current_attempt_id.as_deref().unwrap_or("resume"),
                "resume-reconciliation-required",
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

fn stop_bot(app: &AppHandle, user_id: &str, request: BotStopRequest) -> Result<BotView, String> {
    if request.policy == BotStopPolicy::Flatten && !request.confirm_flatten {
        return Err("Stop and Flatten requires explicit confirmation.".into());
    }
    let bots = app.state::<Arc<BotStore>>();
    let supervisor = app.state::<Arc<crate::bot_supervisor::BotSupervisor>>();
    let local = app.state::<Arc<LocalResearchState>>();
    bots.command(
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
                let _ = bots.fault(
                    user_id,
                    &request.bot_id,
                    "worker-stop-failed",
                    &error,
                );
                return Err("Worker stop failed; the Bot is Faulted and requires recovery.".into());
            }
            let account = match request.policy {
                BotStopPolicy::KeepPosition => local.paper_trading.view_optional(user_id)?,
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
            let reconciled = account_is_reconciled_and_quiet(account.as_ref());
            bots.complete_stop(
                user_id,
                &request.bot_id,
                request.policy,
                positions,
                reconciled,
            )
        },
    )
}

fn bot_instrument_scope(bundle: &BotDeploymentBundle) -> BTreeSet<String> {
    match &bundle.schedule {
        BotSchedule::ClosedBar { instrument_id, .. } => {
            [instrument_id.clone()].into_iter().collect()
        }
        BotSchedule::ScheduledCrossSection { instruments, .. } => {
            instruments.iter().cloned().collect()
        }
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

fn account_is_reconciled_and_quiet(account: Option<&PaperAccountView>) -> bool {
    account.is_some_and(|account| {
        account.reconciliation == adaq_paper_trading_core::ReconciliationState::Reconciled
            && account.orders.iter().all(|order| {
                !matches!(
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
                match cancelled
                    .status
                    .as_deref()
                    .unwrap_or("canceled")
                    .to_ascii_lowercase()
                    .as_str()
                {
                    "canceled" | "cancelled" | "expired" => {
                        for local_order_id in &order.local_order_ids {
                            local.paper_trading.cancel_provider_order(
                                user_id,
                                local_order_id,
                                adaq_bot_runtime::unix_now_ms(),
                            )?;
                        }
                        bots.record_evidence(
                            user_id,
                            bot_id,
                            "execution",
                            "open-order-canceled",
                            "Host canceled the eligible provider order before liquidation.",
                            Some(provider_order_id),
                        )?;
                    }
                    "closed" | "filled" => {
                        bots.record_evidence(
                            user_id,
                            bot_id,
                            "execution",
                            "open-order-filled",
                            "Provider reported the order filled while Flatten was canceling it; Host will reconcile before liquidation.",
                            Some(provider_order_id),
                        )?;
                    }
                    _ => {
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
                }
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
    let account = reconcile_account(local, user_id, bundle)?;
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
    for (index, (instrument, quantity)) in positions.iter().enumerate() {
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
        let operation_id = format!("bot-{bot_id}-flatten-{attempt_id}:{index}");
        let request = PaperOrderRequest {
            user_id: user_id.into(),
            operation_id: operation_id.clone(),
            instrument: instrument.clone(),
            side: "sell".into(),
            quantity: *quantity,
            limit_price: price,
        };
        local.paper_trading.begin_order_with_policy(
            &request,
            Some(&bundle.paper_risk_policy),
            adaq_bot_runtime::unix_now_ms(),
        )?;
        let amount = quantity.to_string();
        let remote = local.connections.create_okx_demo_order(
            user_id,
            instrument,
            "market",
            "sell",
            &amount,
            None,
            adaq_bot_runtime::unix_now_ms(),
        );
        match remote {
            Ok(order) => {
                let Some(provider_order_id) = order.id.clone() else {
                    let _ = local.paper_trading.mark_uncertain(
                        user_id,
                        &operation_id,
                        adaq_bot_runtime::unix_now_ms(),
                    );
                    let _ =
                        bots.record_order(user_id, bot_id, &operation_id, None, "uncertain", None);
                    return Err("flatten-provider-order-identity-missing".into());
                };
                local.paper_trading.record_order_result(
                    user_id,
                    &operation_id,
                    Some(provider_order_id.clone()),
                    order.status.as_deref().unwrap_or("accepted"),
                    None,
                    adaq_bot_runtime::unix_now_ms(),
                )?;
                bots.record_order(
                    user_id,
                    bot_id,
                    &operation_id,
                    None,
                    order.status.as_deref().unwrap_or("accepted"),
                    Some(&provider_order_id),
                )?;
            }
            Err(error) => {
                let _ = local.paper_trading.mark_uncertain(
                    user_id,
                    &operation_id,
                    adaq_bot_runtime::unix_now_ms(),
                );
                let _ = bots.record_order(user_id, bot_id, &operation_id, None, "uncertain", None);
                return Err(format!(
                    "flatten-provider-outcome-uncertain: {}",
                    bounded_text(&error, 512)
                ));
            }
        }
    }
    let final_account = reconcile_account(local, user_id, bundle)?;
    if !account_positions_in_scope(&final_account, &instrument_scope).is_empty()
        || !account_is_reconciled_and_quiet(Some(&final_account))
    {
        return Err("flatten-not-proven".into());
    }
    Ok(final_account)
}

#[cfg(test)]
mod tests {
    use super::*;
    use adaq_bot_runtime::{
        DeploymentBundleInput, StrategyWorld, WORKER_ARTIFACT_NAME, WORKER_ARTIFACT_VERSION,
        WORKER_PROTOCOL_VERSION, WORKER_RUNTIME_VERSION, WORKER_SIGNING_KEY_ID,
        WorkerArtifactBinding, WorkerPipelineBinding, WorkerRuntimePolicy, WorkerStrategyBinding,
    };

    fn hash(byte: char) -> String {
        std::iter::repeat(byte).take(64).collect()
    }

    fn bundle(bot_id: &str, account_id: &str) -> BotDeploymentBundle {
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
    fn restart_faults_active_attempt_and_requires_recovery() {
        let database = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let first = store(database.clone());
        first
            .deploy("user-a", bundle("bot-a", "account-a"))
            .unwrap();
        first.begin_attempt("user-a", "bot-a", false).unwrap();
        drop(first);
        let recovered = store(database);
        let view = recovered.get("user-a", "bot-a").unwrap();
        assert_eq!(view.state, LifecycleState::Faulted);
        assert!(view.attempts[0].reconciliation_required);
        assert!(
            view.attempts[0]
                .evidence
                .iter()
                .any(|item| item.code == "host-restart")
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
    fn host_schedule_rejects_future_and_late_batches() {
        assert!(host_schedule_window(101, 100).is_err());
        assert!(host_schedule_window(100, 100 + DECISION_DEADLINE_GRACE_MS + 1).is_err());
        assert_eq!(host_schedule_window(100, 100).unwrap(), (30_100, 101));
    }
}
