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

use crate::bot_operations::BotStore;
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
}

impl BotSupervisor {
    pub(crate) fn new(operations: OperationsStore, bots: BotStore) -> Self {
        Self {
            workers: Mutex::new(HashMap::new()),
            operations,
            bots,
            monitor_started: AtomicBool::new(false),
        }
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
        let mut durably_faulted_bots = Vec::new();
        if let Ok(mut workers) = self.workers.lock() {
            for (bot_id, managed) in workers.iter_mut() {
                for event in managed.worker.poll_health() {
                    observations.push((
                        managed.user_id.clone(),
                        managed.entity_id.clone(),
                        bot_id.clone(),
                        event,
                    ));
                }
            }
        }
        for (user_id, entity_id, bot_id, event) in observations {
            let fault = match &event {
                WorkerHealthEvent::Fault { code, detail } => Some((code.clone(), detail.clone())),
                _ => None,
            };
            let _ = self.observe_worker_event(&user_id, &entity_id, &bot_id, event);
            if let Some((code, detail)) = fault {
                if self
                    .bots
                    .record_worker_fault(&user_id, &bot_id, &code, &detail)
                    .is_ok()
                {
                    durably_faulted_bots.push(bot_id);
                } else {
                    let _ = self.observe(
                        &user_id,
                        &entity_id,
                        HealthState::Critical,
                        "bot_state_persistence_failed",
                        json!({ "botId": bot_id, "faultCode": code }),
                    );
                }
            }
        }
        if !durably_faulted_bots.is_empty()
            && let Ok(mut workers) = self.workers.lock()
        {
            for bot_id in durably_faulted_bots {
                workers.remove(&bot_id);
            }
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
            return Err("worker bot is already active".into());
        }
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
            },
        );
        if let Err(error) = self.observe(
            user_id,
            entity_id,
            HealthState::Healthy,
            "worker_ready",
            json!({ "botId": bot_id, "lifecycle": "starting", "autoRunning": false }),
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
        self.observe(
            user_id,
            entity_id,
            if to == LifecycleState::Faulted {
                HealthState::Critical
            } else {
                HealthState::Healthy
            },
            "worker_lifecycle",
            json!({ "botId": bot_id, "state": format!("{to:?}"), "reason": reason }),
        )?;
        Ok(())
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
        let (result, health_events, worker_faulted) = {
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
                    let _ = self.observe(
                        user_id,
                        entity_id,
                        HealthState::Critical,
                        "worker_deadline_missed",
                        json!({ "botId": bot_id, "requestId": request_id }),
                    );
                }
                Ok(_) => {
                    self.observe(
                        user_id,
                        entity_id,
                        HealthState::Healthy,
                        "worker_decision",
                        json!({ "botId": bot_id, "requestId": request_id }),
                    )?;
                }
                Err(error) => {
                    let _ = self.observe(
                        user_id,
                        entity_id,
                        HealthState::Critical,
                        "worker_decision_failed",
                        json!({ "botId": bot_id, "requestId": request_id, "error": crate::bot_operations::safe_detail(error) }),
                    );
                }
            }
            health_events.extend(worker.worker.take_health_events());
            let worker_faulted = worker.worker.state() == LifecycleState::Faulted;
            (result, health_events, worker_faulted)
        };
        for event in health_events {
            self.observe_worker_event(user_id, entity_id, bot_id, event)?;
        }
        if worker_faulted {
            workers.remove(bot_id);
        }
        result
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
            self.observe_worker_event(&managed.user_id, &managed.entity_id, bot_id, event)?;
        }
        self.observe(
            user_id,
            entity_id,
            HealthState::Unknown,
            "worker_stopped",
            json!({ "botId": bot_id, "lifecycle": "stopped" }),
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
                        managed.worker.take_health_events(),
                    ));
                }
            }
        }
        let mut frozen = Vec::with_capacity(detached.len());
        for (bot_id, managed_user_id, entity_id, events) in detached {
            for event in events {
                self.observe_worker_event(&managed_user_id, &entity_id, &bot_id, event)?;
            }
            self.observe(
                &managed_user_id,
                &entity_id,
                HealthState::Critical,
                "worker_freeze_all",
                json!({
                    "botId": bot_id,
                    "reason": crate::bot_operations::safe_detail(reason),
                }),
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
        event: WorkerHealthEvent,
    ) -> Result<(), String> {
        let (state, condition, code, detail, evidence) = match event {
            WorkerHealthEvent::Heartbeat {
                observed_at_ms,
                state,
            } => (
                HealthState::Healthy,
                "worker_heartbeat",
                "worker-heartbeat",
                format!("Worker heartbeat observed in state {state:?}."),
                json!({
                    "botId": bot_id,
                    "state": format!("{state:?}"),
                    "observedAtMs": observed_at_ms,
                }),
            ),
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
        self.bots
            .record_evidence(user_id, bot_id, "health", code, &detail, Some(bot_id))?;
        self.observe(user_id, entity_id, state, condition, evidence)
    }

    fn observe(
        &self,
        user_id: &str,
        entity_id: &str,
        state: HealthState,
        condition: &str,
        mut evidence: serde_json::Value,
    ) -> Result<(), String> {
        let (attempt_id, bundle_id) = self
            .bots
            .get(user_id, entity_id)
            .ok()
            .map(|bot| (bot.current_attempt_id, Some(bot.bundle.identity)))
            .unwrap_or((None, None));
        if let Some(object) = evidence.as_object_mut() {
            if let Some(attempt_id) = &attempt_id {
                object.insert("attemptId".into(), json!(attempt_id));
            }
            if let Some(bundle_id) = &bundle_id {
                object.insert("bundleId".into(), json!(bundle_id));
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
