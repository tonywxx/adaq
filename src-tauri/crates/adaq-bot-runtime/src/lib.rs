//! Host-owned contracts for supervised, fail-closed Paper Trading Bots.
//!
//! This crate deliberately contains no provider, credential, or order API. It
//! defines the immutable deployment and control boundaries that those adapters
//! must sit behind.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct DeploymentBundleInput {
    pub bot_id: String,
    pub strategy_id: String,
    pub account_id: String,
    pub component_hashes: Vec<String>,
    pub model_hashes: Vec<String>,
    pub feature_plan_hash: String,
    pub risk_policy_hash: String,
    pub execution_profile_hash: String,
    pub worker_binary_hash: String,
    pub qualification_evidence_hash: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct DeploymentBundle {
    pub input: DeploymentBundleInput,
    pub identity: String,
}

impl DeploymentBundle {
    pub fn freeze(input: DeploymentBundleInput) -> Result<Self, RuntimeError> {
        if input.bot_id.trim().is_empty()
            || input.strategy_id.trim().is_empty()
            || input.account_id.trim().is_empty()
            || input.component_hashes.is_empty()
            || input.qualification_evidence_hash.len() != 64
            || input.worker_binary_hash.len() != 64
        {
            return Err(RuntimeError::BundleNotQualified);
        }
        let bytes = serde_json::to_vec(&input).map_err(|_| RuntimeError::Serialization)?;
        Ok(Self {
            input,
            identity: hex_digest(&bytes),
        })
    }

    pub fn verify(&self) -> Result<(), RuntimeError> {
        let bytes = serde_json::to_vec(&self.input).map_err(|_| RuntimeError::Serialization)?;
        if self.identity == hex_digest(&bytes) {
            Ok(())
        } else {
            Err(RuntimeError::BundleMutated)
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum DecisionClock {
    ClosedBar {
        decision_id: String,
        instrument_id: String,
        decision_time_ms: i64,
        available_at_ms: i64,
        deadline_ms: i64,
        next_execution_ms: i64,
    },
    ScheduledCrossSection {
        decision_id: String,
        decision_time_ms: i64,
        deadline_ms: i64,
        next_execution_ms: i64,
        universe: Vec<String>,
        available_instruments: Vec<String>,
    },
}

impl DecisionClock {
    pub fn validate(&self) -> Result<(), RuntimeError> {
        let (id, decision, deadline, next) = match self {
            Self::ClosedBar {
                decision_id,
                decision_time_ms,
                available_at_ms,
                deadline_ms,
                next_execution_ms,
                ..
            } => {
                if available_at_ms > decision_time_ms {
                    return Err(RuntimeError::UnavailableInput);
                }
                (
                    decision_id,
                    *decision_time_ms,
                    *deadline_ms,
                    *next_execution_ms,
                )
            }
            Self::ScheduledCrossSection {
                decision_id,
                decision_time_ms,
                deadline_ms,
                next_execution_ms,
                universe,
                available_instruments,
            } => {
                if universe.is_empty()
                    || universe.len() != available_instruments.len()
                    || universe
                        .iter()
                        .zip(available_instruments)
                        .any(|(a, b)| a != b)
                {
                    return Err(RuntimeError::IncompleteUniverse);
                }
                (
                    decision_id,
                    *decision_time_ms,
                    *deadline_ms,
                    *next_execution_ms,
                )
            }
        };
        if id.trim().is_empty() || deadline < decision || next <= decision {
            return Err(RuntimeError::InvalidClock);
        }
        Ok(())
    }

    pub fn accepts_target(
        &self,
        decision_id: &str,
        produced_at_ms: i64,
    ) -> Result<(), RuntimeError> {
        self.validate()?;
        let (expected_id, deadline) = match self {
            Self::ClosedBar {
                decision_id,
                deadline_ms,
                ..
            }
            | Self::ScheduledCrossSection {
                decision_id,
                deadline_ms,
                ..
            } => (decision_id, deadline_ms),
        };
        if expected_id != decision_id {
            return Err(RuntimeError::StaleTarget);
        }
        if produced_at_ms > *deadline {
            return Err(RuntimeError::DeadlineMissed);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum LifecycleState {
    Stopped,
    Starting,
    Reconciling,
    WarmingUp,
    Running,
    Pausing,
    Paused,
    Stopping,
    Faulted,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum StopPolicy {
    KeepPosition,
    Flatten,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct RuntimeEvent {
    pub from: LifecycleState,
    pub to: LifecycleState,
    pub actor: String,
    pub reason: String,
}

#[derive(Clone, Debug)]
pub struct RuntimeAttempt {
    bundle: DeploymentBundle,
    state: LifecycleState,
    events: Vec<RuntimeEvent>,
}

impl RuntimeAttempt {
    pub fn start(bundle: DeploymentBundle) -> Result<Self, RuntimeError> {
        bundle.verify()?;
        Ok(Self {
            bundle,
            state: LifecycleState::Starting,
            events: Vec::new(),
        })
    }
    pub fn state(&self) -> LifecycleState {
        self.state
    }
    pub fn bundle(&self) -> &DeploymentBundle {
        &self.bundle
    }
    pub fn events(&self) -> &[RuntimeEvent] {
        &self.events
    }
    pub fn transition(
        &mut self,
        to: LifecycleState,
        actor: impl Into<String>,
        reason: impl Into<String>,
    ) -> Result<(), RuntimeError> {
        let valid = matches!(
            (self.state, to),
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
        );
        if !valid {
            return Err(RuntimeError::InvalidTransition);
        }
        self.events.push(RuntimeEvent {
            from: self.state,
            to,
            actor: actor.into(),
            reason: reason.into(),
        });
        self.state = to;
        Ok(())
    }
    pub fn authorize_target(
        &self,
        clock: &DecisionClock,
        decision_id: &str,
        produced_at_ms: i64,
    ) -> Result<(), RuntimeError> {
        if self.state != LifecycleState::Running {
            return Err(RuntimeError::NotRunning);
        }
        clock.accepts_target(decision_id, produced_at_ms)
    }
    pub fn stop(
        &mut self,
        policy: StopPolicy,
        actor: impl Into<String>,
    ) -> Result<(), RuntimeError> {
        let reason = match policy {
            StopPolicy::KeepPosition => "stop_keep_position",
            StopPolicy::Flatten => "stop_flatten_confirmed",
        };
        self.transition(LifecycleState::Stopping, actor, reason)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeError {
    BundleNotQualified,
    BundleMutated,
    Serialization,
    InvalidClock,
    UnavailableInput,
    IncompleteUniverse,
    StaleTarget,
    DeadlineMissed,
    NotRunning,
    InvalidTransition,
}
impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for RuntimeError {}

fn hex_digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bundle() -> DeploymentBundle {
        DeploymentBundle::freeze(DeploymentBundleInput {
            bot_id: "bot".into(),
            strategy_id: "strategy".into(),
            account_id: "account".into(),
            component_hashes: vec!["component".into()],
            model_hashes: vec![],
            feature_plan_hash: "feature".into(),
            risk_policy_hash: "risk".into(),
            execution_profile_hash: "execution".into(),
            worker_binary_hash: "a".repeat(64),
            qualification_evidence_hash: "b".repeat(64),
        })
        .unwrap()
    }
    fn running() -> RuntimeAttempt {
        let mut attempt = RuntimeAttempt::start(bundle()).unwrap();
        for state in [
            LifecycleState::Reconciling,
            LifecycleState::WarmingUp,
            LifecycleState::Running,
        ] {
            attempt.transition(state, "host", "test").unwrap();
        }
        attempt
    }
    #[test]
    fn only_running_authorizes_fresh_targets_before_deadline() {
        let attempt = running();
        let clock = DecisionClock::ClosedBar {
            decision_id: "bar-1".into(),
            instrument_id: "AAPL".into(),
            decision_time_ms: 100,
            available_at_ms: 100,
            deadline_ms: 110,
            next_execution_ms: 111,
        };
        assert!(attempt.authorize_target(&clock, "bar-1", 110).is_ok());
        assert_eq!(
            attempt.authorize_target(&clock, "bar-0", 101),
            Err(RuntimeError::StaleTarget)
        );
        assert_eq!(
            attempt.authorize_target(&clock, "bar-1", 111),
            Err(RuntimeError::DeadlineMissed)
        );
    }
    #[test]
    fn bundle_mutation_is_rejected() {
        let mut value = bundle();
        value.input.bot_id = "changed".into();
        assert_eq!(value.verify(), Err(RuntimeError::BundleMutated));
    }
    #[test]
    fn pause_and_stop_revoke_target_authority_and_are_audited() {
        let mut attempt = running();
        let clock = DecisionClock::ClosedBar {
            decision_id: "bar".into(),
            instrument_id: "AAPL".into(),
            decision_time_ms: 1,
            available_at_ms: 1,
            deadline_ms: 2,
            next_execution_ms: 3,
        };
        attempt
            .transition(LifecycleState::Pausing, "operator", "pause")
            .unwrap();
        attempt
            .transition(LifecycleState::Paused, "host", "reconciled")
            .unwrap();
        assert_eq!(
            attempt.authorize_target(&clock, "bar", 2),
            Err(RuntimeError::NotRunning)
        );
        attempt.stop(StopPolicy::KeepPosition, "operator").unwrap();
        assert_eq!(attempt.state(), LifecycleState::Stopping);
        assert_eq!(
            attempt.events().last().unwrap().reason,
            "stop_keep_position"
        );
    }
    #[test]
    fn cross_sectional_clock_requires_exact_universe() {
        let clock = DecisionClock::ScheduledCrossSection {
            decision_id: "batch".into(),
            decision_time_ms: 10,
            deadline_ms: 20,
            next_execution_ms: 21,
            universe: vec!["A".into(), "B".into()],
            available_instruments: vec!["A".into()],
        };
        assert_eq!(clock.validate(), Err(RuntimeError::IncompleteUniverse));
    }
}
