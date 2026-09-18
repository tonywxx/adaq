//! Host-owned "new risk gate": the single deep module that answers
//! "may this Bot / account take on new risk right now?"
//!
//! It composes the operational/fault policy (OperationsStore alerts, Bot
//! fault-reconciliation) with the Paper Experiment gating (observation window,
//! warmup) behind one small interface. Call sites use [`Phase::Decision`]
//! (operational + fault only, preserving prior decision-path behaviour) or
//! [`Phase::Execution`] (full gate, including the experiment window/warmup).
//!
//! The experiment branch is expressed as pure helpers over [`PaperExperiment`]
//! so it is unit-testable without a store; the composite gates are the only
//! interface callers and tests cross.

use std::collections::BTreeSet;

use crate::bot_operations::BotStore;
use crate::operations::{AlertState, HealthDimension, OperationsStore, SafetyAction};
use crate::paper_experiment::{
    PaperExperiment, PaperExperimentState, PaperExperimentStore, all_experiment_bots_warmed,
    bot_is_warmed,
};
use crate::paper_trading::PaperTradingStore;

/// Narrow input the gate needs; it does not take the whole `LocalResearchState`.
pub struct NewRiskContext<'a> {
    pub user_id: &'a str,
    pub bot_id: &'a str,
    pub account_id: &'a str,
    pub now_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewRiskBlockReason {
    OperationalSafetyAction,
    FaultedAttemptUnreconciled,
    ExperimentWindowClosed,
    ExperimentWarmupIncomplete,
    DecisionWindowClosed,
}

/// How a block is surfaced at the call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockEffect {
    /// Translate to a hard `Err` (Host safety action / fault).
    HardErr,
    /// Translate to a soft skip (`Ok(())`) at the call site (experiment gating).
    SoftSkip,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewRiskBlock {
    pub reason: NewRiskBlockReason,
    pub effect: BlockEffect,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum NewRiskOutcome {
    Permitted,
    Blocked(NewRiskBlock),
}

/// Which gate a call site is asking about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Decision,
    Execution,
}

// --- pure helpers (operate on `PaperExperiment`; unit-testable) ---

/// Mirrors the prior `decision_blocked_for_bot`: does the bot's experiment
/// accept an observation at `now_ms`?
pub(crate) fn experiment_accepts_observation(experiment: &PaperExperiment, now_ms: i64) -> bool {
    experiment.started_at_ms.is_some()
        && now_ms >= experiment.observation_start_ms
        && now_ms < experiment.observation_end_ms
        && matches!(
            experiment.state,
            PaperExperimentState::Preparing
                | PaperExperimentState::Armed
                | PaperExperimentState::Running
        )
}

/// Mirrors the prior `risk_blocked_for_bot`: is the experiment risk window open?
pub(crate) fn experiment_risk_window_open(experiment: &PaperExperiment, now_ms: i64) -> bool {
    experiment.state == PaperExperimentState::Running
        && experiment.started_at_ms.is_some()
        && now_ms >= experiment.observation_start_ms
        && now_ms < experiment.observation_end_ms
}

/// Mirrors the prior `common_warmup_blocked_for_bot`: within an open window,
/// are all experiment Bots warmed?
pub(crate) fn experiment_warmup_incomplete(
    experiment: &PaperExperiment,
    warmed_bot_ids: &BTreeSet<String>,
    now_ms: i64,
) -> bool {
    if experiment.state != PaperExperimentState::Running
        || experiment.started_at_ms.is_none()
        || now_ms < experiment.observation_start_ms
        || now_ms >= experiment.observation_end_ms
    {
        return false;
    }
    !all_experiment_bots_warmed(experiment, warmed_bot_ids)
}

// --- composite gates ---

/// Decision eligibility: does the bot's Paper Experiment accept an observation now?
pub fn decision_blocked(
    ctx: &NewRiskContext,
    experiments: &PaperExperimentStore,
) -> Result<NewRiskOutcome, String> {
    let Some(experiment) = experiments.for_bot(ctx.user_id, ctx.bot_id)? else {
        return Ok(NewRiskOutcome::Permitted);
    };
    if experiment_accepts_observation(&experiment, ctx.now_ms) {
        Ok(NewRiskOutcome::Permitted)
    } else {
        Ok(NewRiskOutcome::Blocked(NewRiskBlock {
            reason: NewRiskBlockReason::DecisionWindowClosed,
            effect: BlockEffect::SoftSkip,
            message: "The declared Paper Experiment window did not permit this observation; no Worker or order work was authorized.".into(),
        }))
    }
}

/// Whether new risk may be taken for this Bot/account right now.
///
/// `Phase::Decision` checks only the operational/fault layer (prior
/// decision-path behaviour); `Phase::Execution` adds the Paper Experiment
/// window/warmup gate. Both re-evaluate fresh state, so the gate is fail-closed
/// even when decision and execution are separated in time (ADR-0049).
pub fn new_risk_blocked(
    ctx: &NewRiskContext,
    operations: &OperationsStore,
    bots: &BotStore,
    paper_trading: &PaperTradingStore,
    experiments: &PaperExperimentStore,
    phase: Phase,
) -> Result<NewRiskOutcome, String> {
    // Operational safety action: any unresolved, non-Worker alert with a safety action.
    if operations
        .alerts_for_user(ctx.user_id)?
        .into_iter()
        .any(|alert| {
            alert.state != AlertState::Resolved
                && alert.dimension != HealthDimension::Worker
                && alert.safety_action != SafetyAction::None
        })
    {
        return Ok(NewRiskOutcome::Blocked(NewRiskBlock {
            reason: NewRiskBlockReason::OperationalSafetyAction,
            effect: BlockEffect::HardErr,
            message: "A Host operational safety action is active; new Bot risk is blocked.".into(),
        }));
    }

    // Faulted attempt still blocks new risk until its account reconciles.
    if bots.account_blocks_new_risk(ctx.user_id, ctx.account_id)?
        && !crate::bot_operations::reconciliation_resolves_fault_gate(
            paper_trading.view_optional(ctx.user_id)?.as_ref(),
            ctx.account_id,
        )
    {
        return Ok(NewRiskOutcome::Blocked(NewRiskBlock {
            reason: NewRiskBlockReason::FaultedAttemptUnreconciled,
            effect: BlockEffect::HardErr,
            message:
                "A faulted Bot Runtime Attempt is pending reconciliation; new Bot risk is blocked."
                    .into(),
        }));
    }

    if phase == Phase::Execution {
        if let Some(experiment) = experiments.for_bot(ctx.user_id, ctx.bot_id)? {
            if !experiment_risk_window_open(&experiment, ctx.now_ms) {
                return Ok(NewRiskOutcome::Blocked(NewRiskBlock {
                    reason: NewRiskBlockReason::ExperimentWindowClosed,
                    effect: BlockEffect::SoftSkip,
                    message: "The Paper Experiment risk window is closed; no order was created."
                        .into(),
                }));
            }
            let warmed_bot_ids = bots
                .list(ctx.user_id)?
                .iter()
                .filter(|bot| bot_is_warmed(bot))
                .map(|bot| bot.bot_id.clone())
                .collect::<BTreeSet<_>>();
            if experiment_warmup_incomplete(&experiment, &warmed_bot_ids, ctx.now_ms) {
                return Ok(NewRiskOutcome::Blocked(NewRiskBlock {
                    reason: NewRiskBlockReason::ExperimentWarmupIncomplete,
                    effect: BlockEffect::SoftSkip,
                    message: "The Paper Experiment is waiting for all three Bots to complete warmup; no order was created.".into(),
                }));
            }
        }
    }

    Ok(NewRiskOutcome::Permitted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operations::{HealthDimension, HealthObservation, HealthState};
    use crate::paper_experiment::PaperExperimentInstrument;
    use rusqlite::Connection;
    use rust_decimal::Decimal;
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    fn in_memory() -> Arc<Mutex<Connection>> {
        Arc::new(Mutex::new(Connection::open_in_memory().unwrap()))
    }

    fn obs(state: HealthState, dimension: HealthDimension) -> HealthObservation {
        HealthObservation {
            user_id: "u".into(),
            entity_id: "bot".into(),
            dimension,
            state,
            condition: "heartbeat".into(),
            evidence: json!({"state": state}),
            required: true,
            observed_at_ms: 1,
            event_kind: None,
            evidence_id: None,
            correlation_id: None,
            causation_id: None,
            diagnostic: None,
            metrics: BTreeMap::new(),
        }
    }

    fn experiment(
        id: &str,
        state: PaperExperimentState,
        now_window: (i64, i64),
        bot_ids: &[&str],
    ) -> PaperExperiment {
        let instruments = bot_ids
            .iter()
            .enumerate()
            .map(|(i, bot_id)| PaperExperimentInstrument {
                instrument: match i {
                    0 => "BTC-USDT",
                    1 => "ETH-USDT",
                    _ => "SOL-USDT",
                }
                .into(),
                qualification_id: "q".into(),
                bot_id: Some((*bot_id).into()),
                allocation_usdt: Decimal::ZERO,
                entry_notional_cap_usdt: Decimal::ZERO,
                reserved_cash_usdt: Decimal::ZERO,
            })
            .collect();
        PaperExperiment {
            experiment_id: id.into(),
            user_id: "alice".into(),
            profile_id: "prof".into(),
            account_id: "account-1".into(),
            observation_start_ms: now_window.0,
            observation_end_ms: now_window.1,
            allocation_total_usdt: Decimal::ZERO,
            unallocated_remainder_usdt: Decimal::ZERO,
            starting_account_cash: None,
            started_at_ms: Some(50),
            instruments,
            state,
            report_id: None,
            feedback_snapshot_ids: Vec::new(),
            feedback_report_ids: Vec::new(),
            limitations: Vec::new(),
            created_at_ms: 1,
            updated_at_ms: 1,
        }
    }

    fn ctx<'a>(
        user_id: &'a str,
        bot_id: &'a str,
        account_id: &'a str,
        now_ms: i64,
    ) -> NewRiskContext<'a> {
        NewRiskContext {
            user_id,
            bot_id,
            account_id,
            now_ms,
        }
    }

    #[test]
    fn operational_alert_blocks_new_risk() {
        let operations = OperationsStore::open(in_memory()).unwrap();
        let bots = BotStore::open(in_memory()).unwrap();
        let paper_trading = PaperTradingStore::open(in_memory()).unwrap();
        let experiments = PaperExperimentStore::open(in_memory()).unwrap();
        operations
            .observe(obs(HealthState::Critical, HealthDimension::MarketData))
            .unwrap();
        let outcome = new_risk_blocked(
            &ctx("u", "bot-1", "account-1", 200),
            &operations,
            &bots,
            &paper_trading,
            &experiments,
            Phase::Execution,
        )
        .unwrap();
        match outcome {
            NewRiskOutcome::Blocked(block) => {
                assert_eq!(block.reason, NewRiskBlockReason::OperationalSafetyAction);
                assert_eq!(block.effect, BlockEffect::HardErr);
            }
            NewRiskOutcome::Permitted => panic!("expected operational block"),
        }
    }

    #[test]
    fn worker_alert_does_not_block_new_risk() {
        let operations = OperationsStore::open(in_memory()).unwrap();
        let bots = BotStore::open(in_memory()).unwrap();
        let paper_trading = PaperTradingStore::open(in_memory()).unwrap();
        let experiments = PaperExperimentStore::open(in_memory()).unwrap();
        operations
            .observe(obs(HealthState::Critical, HealthDimension::Worker))
            .unwrap();
        let outcome = new_risk_blocked(
            &ctx("u", "bot-1", "account-1", 200),
            &operations,
            &bots,
            &paper_trading,
            &experiments,
            Phase::Execution,
        )
        .unwrap();
        assert_eq!(outcome, NewRiskOutcome::Permitted);
    }

    #[test]
    fn phase_distinction_and_experiment_branch() {
        let operations = OperationsStore::open(in_memory()).unwrap();
        let bots = BotStore::open(in_memory()).unwrap();
        let paper_trading = PaperTradingStore::open(in_memory()).unwrap();
        let experiments = PaperExperimentStore::open(in_memory()).unwrap();

        // Window [100, 300); now = 200 sits inside the window.
        // Disjoint instrument bot-ids so `for_bot` resolves each query to a
        // single experiment unambiguously (preparing -> bot-1, running -> bot-2).
        let preparing = experiment(
            "exp-1",
            PaperExperimentState::Preparing,
            (100, 300),
            &["bot-1"],
        );
        experiments.create(&preparing).unwrap();
        let running = experiment(
            "exp-2",
            PaperExperimentState::Running,
            (100, 300),
            &["bot-2", "bot-3"],
        );
        experiments.create(&running).unwrap();

        // Decision phase ignores the experiment entirely.
        assert_eq!(
            new_risk_blocked(
                &ctx("alice", "bot-1", "account-1", 200),
                &operations,
                &bots,
                &paper_trading,
                &experiments,
                Phase::Decision,
            )
            .unwrap(),
            NewRiskOutcome::Permitted
        );

        // Execution phase + Preparing window (not Running) => window closed.
        match new_risk_blocked(
            &ctx("alice", "bot-1", "account-1", 200),
            &operations,
            &bots,
            &paper_trading,
            &experiments,
            Phase::Execution,
        )
        .unwrap()
        {
            NewRiskOutcome::Blocked(block) => {
                assert_eq!(block.reason, NewRiskBlockReason::ExperimentWindowClosed);
                assert_eq!(block.effect, BlockEffect::SoftSkip);
            }
            NewRiskOutcome::Permitted => panic!("expected window-closed block"),
        }

        // Execution phase + Running window but no warmed bots => warmup incomplete.
        match new_risk_blocked(
            &ctx("alice", "bot-2", "account-1", 200),
            &operations,
            &bots,
            &paper_trading,
            &experiments,
            Phase::Execution,
        )
        .unwrap()
        {
            NewRiskOutcome::Blocked(block) => {
                assert_eq!(block.reason, NewRiskBlockReason::ExperimentWarmupIncomplete);
                assert_eq!(block.effect, BlockEffect::SoftSkip);
            }
            NewRiskOutcome::Permitted => panic!("expected warmup-incomplete block"),
        }
    }
}
