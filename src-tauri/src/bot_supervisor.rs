use adaq_bot_runtime::{
    DecisionClock, LifecycleState, WorkerDecisionInput, WorkerDecisionResult, WorkerHealthEvent,
    WorkerLaunchRequest, WorkerSupervisor,
};
use serde_json::json;
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use crate::bot_operations::{BotAttemptIdentity, BotStore};
use crate::operations::{HealthDimension, HealthObservation, HealthState, OperationsStore};

pub(crate) struct BotSupervisor {
    workers: Mutex<HashMap<String, ManagedWorker>>,
    operations: OperationsStore,
    bots: BotStore,
    monitor_started: AtomicBool,
}

struct ManagedWorker {
    worker: WorkerSupervisor,
    user_id: String,
    entity_id: String,
    registration: Arc<()>,
    identity: BotAttemptIdentity,
    pending_fault: Option<WorkerFault>,
}

#[derive(Clone)]
struct WorkerFault {
    code: String,
    detail: String,
}

impl BotSupervisor {
    pub(crate) fn new(operations: OperationsStore, bots: BotStore) -> Result<Self, String> {
        let this = Self {
            workers: Mutex::new(HashMap::new()),
            operations,
            bots,
            monitor_started: AtomicBool::new(false),
        };
        this.recover_all()?;
        Ok(this)
    }

    pub(crate) fn start_monitor(self: &Arc<Self>) {
        if self.monitor_started.swap(true, Ordering::AcqRel) {
            return;
        }
        let supervisor = Arc::downgrade(self);
        thread::spawn(move || {
            while let Some(supervisor) = supervisor.upgrade() {
                supervisor.poll_workers();
                thread::sleep(Duration::from_millis(250));
            }
        });
    }

    fn poll_workers(&self) {
        let mut observations = Vec::new();
        if let Ok(mut workers) = self.workers.lock() {
            for (bot_id, managed) in workers.iter_mut() {
                if let Some(fault) = &managed.pending_fault {
                    // A terminated Worker emits its fault only once. Retain it
                    // until the durable Bot/Attempt state catches up.
                    managed.worker.take_health_events();
                    observations.push((
                        managed.user_id.clone(),
                        managed.entity_id.clone(),
                        bot_id.clone(),
                        managed.registration.clone(),
                        WorkerHealthEvent::Fault {
                            code: fault.code.clone(),
                            detail: fault.detail.clone(),
                        },
                    ));
                    continue;
                }
                for event in managed.worker.poll_health() {
                    observations.push((
                        managed.user_id.clone(),
                        managed.entity_id.clone(),
                        bot_id.clone(),
                        managed.registration.clone(),
                        event,
                    ));
                }
            }
        }
        for (user_id, entity_id, bot_id, registration, event) in observations {
            let _ = self.handle_worker_event(&user_id, &entity_id, &bot_id, &registration, event);
        }
    }

    fn handle_worker_event(
        &self,
        user_id: &str,
        entity_id: &str,
        bot_id: &str,
        registration: &Arc<()>,
        event: WorkerHealthEvent,
    ) -> Result<(), String> {
        if let WorkerHealthEvent::Fault { code, detail } = &event {
            self.converge_fault(
                user_id,
                entity_id,
                bot_id,
                &WorkerFault {
                    code: code.clone(),
                    detail: detail.clone(),
                },
                Some(registration),
                |identity| self.observe_worker_event(user_id, entity_id, bot_id, identity, event),
            )
        } else {
            let workers = self
                .workers
                .lock()
                .map_err(|_| "worker registry lock failed".to_owned())?;
            let Some(managed) = workers
                .get(bot_id)
                .filter(|managed| Arc::ptr_eq(&managed.registration, registration))
            else {
                return Ok(());
            };
            self.observe_worker_event(user_id, entity_id, bot_id, Some(&managed.identity), event)
        }
    }

    pub(crate) fn start(
        &self,
        user_id: &str,
        entity_id: &str,
        request: WorkerLaunchRequest,
    ) -> Result<(), String> {
        let bot_id = request.bundle.input.bot_id.clone();
        if bot_id.trim().is_empty() {
            return Err("worker bot identity is required".into());
        }
        let mut workers = self
            .workers
            .lock()
            .map_err(|_| "worker registry lock failed".to_owned())?;
        if workers.contains_key(&bot_id) {
            let state = self.bots.get(user_id, &bot_id)?.state;
            if worker_registry_entry_is_active(state) {
                return Err("worker bot is already active".into());
            }
            if let Some(mut stale) = workers.remove(&bot_id) {
                stale
                    .worker
                    .terminate_for_fault("stale-worker-registry-entry");
            }
        }
        let bot = self.bots.get(user_id, &bot_id)?;
        if bot.bundle.runtime_bundle.identity != request.bundle.identity {
            return Err("Worker launch Bundle does not match the Bot deployment.".into());
        }
        let identity = Self::identity_for_bot(&bot)?;
        let worker = WorkerSupervisor::launch(request).map_err(|error| {
            let _ = self.observe(
                user_id,
                entity_id,
                HealthState::Critical,
                "worker_start_failed",
                json!({ "botId": bot_id, "error": crate::bot_operations::safe_detail(&error) }),
            );
            error
        })?;
        workers.insert(
            bot_id.clone(),
            ManagedWorker {
                worker,
                user_id: user_id.into(),
                entity_id: entity_id.into(),
                registration: Arc::new(()),
                identity: identity.clone(),
                pending_fault: None,
            },
        );
        if let Err(error) = self.observe_at(
            user_id,
            entity_id,
            HealthState::Healthy,
            "worker_ready",
            json!({ "botId": bot_id, "lifecycle": "starting", "autoRunning": false }),
            Some(&identity),
        ) {
            if let Some(mut worker) = workers.remove(&bot_id) {
                worker
                    .worker
                    .terminate_for_fault("operations-evidence-failed");
            }
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn transition(
        &self,
        user_id: &str,
        entity_id: &str,
        bot_id: &str,
        to: LifecycleState,
        actor: &str,
        reason: &str,
    ) -> Result<(), String> {
        let mut workers = self
            .workers
            .lock()
            .map_err(|_| "worker registry lock failed".to_owned())?;
        let worker = workers
            .get_mut(bot_id)
            .ok_or_else(|| "worker bot is not active".to_owned())?;
        worker.worker.transition(to, actor, reason)?;
        self.observe_at(
            user_id,
            entity_id,
            if to == LifecycleState::Faulted {
                HealthState::Critical
            } else {
                HealthState::Healthy
            },
            "worker_lifecycle",
            json!({ "botId": bot_id, "state": format!("{to:?}"), "reason": reason }),
            Some(&worker.identity),
        )?;
        Ok(())
    }

    fn converge_fault(
        &self,
        user_id: &str,
        entity_id: &str,
        bot_id: &str,
        fault: &WorkerFault,
        registration: Option<&Arc<()>>,
        observe_fault: impl FnOnce(Option<&BotAttemptIdentity>) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut workers = self
            .workers
            .lock()
            .map_err(|_| "worker registry lock failed".to_owned());
        if let Some(registration) = registration {
            let current = workers.as_ref().map_err(|error| error.clone())?;
            // A delayed monitor event belongs to the captured registration,
            // never to a replacement Worker or a subsequent Runtime Attempt.
            if !current
                .get(bot_id)
                .is_some_and(|managed| Arc::ptr_eq(&managed.registration, registration))
            {
                return Ok(());
            }
        }
        let identity = if registration.is_some() {
            workers
                .as_ref()
                .map_err(|error| error.clone())
                .and_then(|workers| {
                    workers
                        .get(bot_id)
                        .map(|managed| managed.identity.clone())
                        .ok_or_else(|| "Worker event identity is unavailable.".to_owned())
                })
        } else {
            self.attempt_identity(user_id, bot_id)
        };
        if let Ok(workers) = &mut workers
            && let Some(managed) = workers.get_mut(bot_id)
        {
            managed.worker.terminate_for_fault(&fault.code);
            managed.pending_fault = Some(WorkerFault {
                code: crate::bot_operations::safe_detail(&fault.code),
                detail: crate::bot_operations::safe_detail(&fault.detail),
            });
        }
        // Keep the registry entry while its fault is not durable, and prevent
        // a concurrent launch from replacing it before this cleanup completes.
        let persistence = identity
            .as_ref()
            .map_err(|error| error.clone())
            .and_then(|identity| {
                self.bots
                    .record_worker_fault(user_id, bot_id, &fault.code, &fault.detail, identity)
            });
        let persistence_failed = persistence.is_err();
        let retired = matches!(persistence, Ok(false));
        if persistence.is_ok()
            && let Ok(workers) = &mut workers
        {
            workers.remove(bot_id);
        }
        let termination = workers.map(|_| ());
        let observation = if retired {
            Ok(())
        } else {
            observe_fault(identity.as_ref().ok())
        };
        let persistence_observation = if persistence_failed {
            self.observe_at(
                user_id,
                entity_id,
                HealthState::Critical,
                "bot_state_persistence_failed",
                json!({ "botId": bot_id, "faultCode": crate::bot_operations::safe_detail(&fault.code) }),
                identity.as_ref().ok(),
            )
        } else {
            Ok(())
        };

        // Observability failure must not short-circuit the safety steps. Report
        // every failed stage in execution order after all have been attempted.
        let errors = [
            ("worker termination", termination),
            ("Bot state persistence", persistence.map(|_| ())),
            ("fault observation", observation),
            ("persistence failure observation", persistence_observation),
        ]
        .into_iter()
        .filter_map(|(stage, result)| {
            result
                .err()
                .map(|error| format!("{stage}: {}", crate::bot_operations::safe_detail(&error)))
        })
        .collect::<Vec<_>>();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "{}: {}; {}",
                crate::bot_operations::safe_detail(&fault.code),
                crate::bot_operations::safe_detail(&fault.detail),
                errors.join("; ")
            ))
        }
    }

    /// Host-owned recovery: terminate the Worker and persist `Faulted` under the
    /// given `code`, with no separate prior transition. ADR-0048 grants the
    /// Supervisor sole authority over recovery, so decision and command failures
    /// reach this (not `BotStore`) as the single recovery entry point.
    pub(crate) fn fail_active(
        &self,
        user_id: &str,
        bot_id: &str,
        code: &str,
        detail: &str,
    ) -> Result<(), String> {
        self.converge_fault(
            user_id,
            bot_id,
            bot_id,
            &WorkerFault {
                code: code.to_owned(),
                detail: detail.to_owned(),
            },
            None,
            |identity| {
                self.observe_at(
                    user_id,
                    bot_id,
                    HealthState::Critical,
                    "worker_lifecycle_faulted",
                    json!({
                        "botId": bot_id,
                        "code": crate::bot_operations::safe_detail(code),
                        "detail": crate::bot_operations::safe_detail(detail),
                    }),
                    identity,
                )
            },
        )
    }

    /// Recovery for a failed lifecycle transition: the same Worker termination
    /// plus durable `Faulted`, surfaced under `lifecycle-transition-failed`.
    pub(crate) fn fail_transition(
        &self,
        user_id: &str,
        bot_id: &str,
        detail: &str,
    ) -> Result<(), String> {
        self.fail_active(user_id, bot_id, "lifecycle-transition-failed", detail)
    }

    /// Recovery for a failed Worker decision outcome, preserving the original
    /// `code` (e.g. `target-identity-invalid`, `decision-deadline-missed`,
    /// `worker-decision-failed`, `target-execution-failed`).
    pub(crate) fn fail_decision(
        &self,
        user_id: &str,
        bot_id: &str,
        code: &str,
        detail: &str,
    ) -> Result<(), String> {
        self.fail_active(user_id, bot_id, code, detail)
    }

    /// Host-restart recovery: every still-active Runtime Attempt is moved to
    /// `Faulted` with `reconciliation_required`. Delegated to the persistence
    /// primitive in `BotStore`; the Supervisor owns the trigger (see `new`).
    pub(crate) fn recover_all(&self) -> Result<(), String> {
        self.bots.recover_after_restart()
    }

    pub(crate) fn decision(
        &self,
        user_id: &str,
        entity_id: &str,
        bot_id: &str,
        request_id: String,
        clock: DecisionClock,
        input: WorkerDecisionInput,
    ) -> Result<WorkerDecisionResult, String> {
        let mut workers = self
            .workers
            .lock()
            .map_err(|_| "worker registry lock failed".to_owned())?;
        let (result, health_events, worker_faulted, identity) = {
            let worker = workers
                .get_mut(bot_id)
                .ok_or_else(|| "worker bot is not active".to_owned())?;
            let result = worker.worker.decision(request_id.clone(), clock, input);
            let mut health_events = worker.worker.take_health_events();
            match &result {
                Ok(WorkerDecisionResult::NoTarget {
                    reason: adaq_bot_runtime::NoTargetReason::DeadlineMissed,
                    ..
                }) => {
                    worker
                        .worker
                        .terminate_for_fault("decision-deadline-missed");
                    let _ = self.observe_at(
                        user_id,
                        entity_id,
                        HealthState::Critical,
                        "worker_deadline_missed",
                        json!({ "botId": bot_id, "requestId": request_id }),
                        Some(&worker.identity),
                    );
                }
                Ok(_) => {
                    self.observe_at(
                        user_id,
                        entity_id,
                        HealthState::Healthy,
                        "worker_decision",
                        json!({ "botId": bot_id, "requestId": request_id }),
                        Some(&worker.identity),
                    )?;
                }
                Err(error) => {
                    let _ = self.observe_at(
                        user_id,
                        entity_id,
                        HealthState::Critical,
                        "worker_decision_failed",
                        json!({ "botId": bot_id, "requestId": request_id, "error": crate::bot_operations::safe_detail(error) }),
                        Some(&worker.identity),
                    );
                }
            }
            health_events.extend(worker.worker.take_health_events());
            let worker_faulted = worker.worker.state() == LifecycleState::Faulted;
            (
                result,
                health_events,
                worker_faulted,
                worker.identity.clone(),
            )
        };
        for event in health_events {
            self.observe_worker_event(user_id, entity_id, bot_id, Some(&identity), event)?;
        }
        if worker_faulted {
            workers.remove(bot_id);
        }
        result
    }

    pub(crate) fn has_worker(&self, bot_id: &str) -> Result<bool, String> {
        Ok(self
            .workers
            .lock()
            .map_err(|error| error.to_string())?
            .contains_key(bot_id))
    }

    pub(crate) fn stop(
        &self,
        user_id: &str,
        entity_id: &str,
        bot_id: &str,
        request_id: &str,
    ) -> Result<(), String> {
        let mut workers = self
            .workers
            .lock()
            .map_err(|_| "worker registry lock failed".to_owned())?;
        let mut managed = workers
            .remove(bot_id)
            .ok_or_else(|| "worker bot is not active".to_owned())?;
        managed.worker.shutdown(request_id)?;
        let health_events = managed.worker.take_health_events();
        drop(workers);
        for event in health_events {
            self.observe_worker_event(
                &managed.user_id,
                &managed.entity_id,
                bot_id,
                Some(&managed.identity),
                event,
            )?;
        }
        self.observe_at(
            user_id,
            entity_id,
            HealthState::Healthy,
            "worker_stopped",
            json!({ "botId": bot_id, "lifecycle": "stopped" }),
            Some(&managed.identity),
        )?;
        Ok(())
    }

    pub(crate) fn freeze_all(&self, user_id: &str, reason: &str) -> Result<Vec<String>, String> {
        crate::user::validate_user(user_id)?;
        let mut detached = Vec::new();
        {
            let mut workers = self
                .workers
                .lock()
                .map_err(|_| "worker registry lock failed".to_owned())?;
            let bot_ids = workers
                .iter()
                .filter(|(_, managed)| managed.user_id == user_id)
                .map(|(bot_id, _)| bot_id.clone())
                .collect::<Vec<_>>();
            for bot_id in bot_ids {
                if let Some(mut managed) = workers.remove(&bot_id) {
                    managed.worker.terminate_for_fault("operations-freeze-all");
                    detached.push((
                        bot_id,
                        managed.user_id,
                        managed.entity_id,
                        managed.identity,
                        managed.worker.take_health_events(),
                    ));
                }
            }
        }
        let mut frozen = Vec::with_capacity(detached.len());
        for (bot_id, managed_user_id, entity_id, identity, events) in detached {
            for event in events {
                self.observe_worker_event(
                    &managed_user_id,
                    &entity_id,
                    &bot_id,
                    Some(&identity),
                    event,
                )?;
            }
            self.observe_at(
                &managed_user_id,
                &entity_id,
                HealthState::Critical,
                "worker_freeze_all",
                json!({
                    "botId": bot_id,
                    "reason": crate::bot_operations::safe_detail(reason),
                }),
                Some(&identity),
            )?;
            frozen.push(bot_id);
        }
        Ok(frozen)
    }

    fn observe_worker_event(
        &self,
        user_id: &str,
        entity_id: &str,
        bot_id: &str,
        identity: Option<&BotAttemptIdentity>,
        event: WorkerHealthEvent,
    ) -> Result<(), String> {
        let (state, condition, code, detail, evidence) = match event {
            WorkerHealthEvent::Heartbeat { .. } => return Ok(()),
            WorkerHealthEvent::Diagnostic { code, detail } => (
                HealthState::Degraded,
                "worker_diagnostic",
                "worker-diagnostic",
                crate::bot_operations::safe_detail(&detail),
                json!({ "botId": bot_id, "code": code, "detail": crate::bot_operations::safe_detail(&detail) }),
            ),
            WorkerHealthEvent::Fault { code, detail } => (
                HealthState::Critical,
                "worker_fault",
                "worker-fault",
                format!(
                    "{}: {}",
                    crate::bot_operations::safe_detail(&code),
                    crate::bot_operations::safe_detail(&detail)
                ),
                json!({ "botId": bot_id, "code": code, "detail": crate::bot_operations::safe_detail(&detail) }),
            ),
        };
        let identity =
            identity.ok_or_else(|| "Worker event identity is unavailable.".to_owned())?;
        self.bots
            .record_attempt_evidence(user_id, bot_id, identity, "health", code, &detail)?;
        self.observe_at(
            user_id,
            entity_id,
            state,
            condition,
            evidence,
            Some(identity),
        )
    }

    fn attempt_identity(&self, user_id: &str, bot_id: &str) -> Result<BotAttemptIdentity, String> {
        let bot = self.bots.get(user_id, bot_id)?;
        Self::identity_for_bot(&bot)
    }

    fn identity_for_bot(
        bot: &crate::bot_operations::BotView,
    ) -> Result<BotAttemptIdentity, String> {
        let attempt = bot
            .attempts
            .iter()
            .find(|attempt| Some(&attempt.attempt_id) == bot.current_attempt_id.as_ref())
            .ok_or_else(|| "Bot has no current Runtime Attempt for the Worker.".to_owned())?;
        Ok(BotAttemptIdentity {
            attempt_id: attempt.attempt_id.clone(),
            bundle_identity: attempt.bundle_identity.clone(),
        })
    }

    fn observe(
        &self,
        user_id: &str,
        entity_id: &str,
        state: HealthState,
        condition: &str,
        evidence: serde_json::Value,
    ) -> Result<(), String> {
        let identity = self.attempt_identity(user_id, entity_id).ok();
        self.observe_at(
            user_id,
            entity_id,
            state,
            condition,
            evidence,
            identity.as_ref(),
        )
    }

    fn observe_at(
        &self,
        user_id: &str,
        entity_id: &str,
        state: HealthState,
        condition: &str,
        mut evidence: serde_json::Value,
        identity: Option<&BotAttemptIdentity>,
    ) -> Result<(), String> {
        let attempt_id = identity.map(|identity| identity.attempt_id.clone());
        let bundle_id = identity.map(|identity| identity.bundle_identity.clone());
        if let Some(object) = evidence.as_object_mut() {
            if let Some(attempt_id) = &attempt_id {
                object.insert("attemptId".into(), json!(attempt_id));
            }
            if let Some(bundle_id) = &bundle_id {
                object.insert("bundleId".into(), json!(bundle_id));
                object.insert("correlationId".into(), json!(bundle_id));
            }
        }
        self.operations
            .observe(HealthObservation {
                user_id: user_id.into(),
                entity_id: entity_id.into(),
                dimension: HealthDimension::Worker,
                state,
                condition: condition.into(),
                evidence,
                required: true,
                observed_at_ms: adaq_bot_runtime::unix_now_ms(),
                event_kind: Some(format!("worker.{}", condition.replace('_', "-"))),
                evidence_id: attempt_id,
                correlation_id: bundle_id,
                causation_id: None,
                diagnostic: None,
                metrics: std::collections::BTreeMap::new(),
            })
            .map(|_| ())
    }
}

fn worker_registry_entry_is_active(state: LifecycleState) -> bool {
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

#[cfg(test)]
mod failure_tests;

#[cfg(test)]
mod heartbeat_tests {
    use super::*;

    #[test]
    fn worker_heartbeats_are_checked_without_persisting_every_frame() {
        let database = Arc::new(Mutex::new(rusqlite::Connection::open_in_memory().unwrap()));
        let operations = OperationsStore::open(database.clone()).unwrap();
        let bots = BotStore::open(database).unwrap();
        let supervisor = BotSupervisor::new(operations.clone(), bots).unwrap();

        supervisor
            .observe_worker_event(
                "user-a",
                "bot-a",
                "bot-a",
                None,
                WorkerHealthEvent::Heartbeat {
                    observed_at_ms: 1,
                    state: adaq_bot_runtime::WorkerHealthState::Ready,
                },
            )
            .unwrap();

        assert!(operations.events_for_user("user-a", 1).unwrap().is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn faulted_bot_reclaims_a_stale_worker_registry_entry() {
        assert!(!worker_registry_entry_is_active(LifecycleState::Faulted));
        assert!(!worker_registry_entry_is_active(LifecycleState::Stopped));
        assert!(worker_registry_entry_is_active(LifecycleState::Running));
    }
}
