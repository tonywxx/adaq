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

use crate::operations::{HealthDimension, HealthObservation, HealthState, OperationsStore};

pub(crate) struct BotSupervisor {
    workers: Mutex<HashMap<String, ManagedWorker>>,
    operations: OperationsStore,
    monitor_started: AtomicBool,
}

struct ManagedWorker {
    worker: WorkerSupervisor,
    user_id: String,
    entity_id: String,
}

impl BotSupervisor {
    pub(crate) fn new(operations: OperationsStore) -> Self {
        Self {
            workers: Mutex::new(HashMap::new()),
            operations,
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
            let _ = self.observe_worker_event(&user_id, &entity_id, &bot_id, event);
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
        if let Some(mut previous) = workers.remove(&bot_id) {
            previous.worker.terminate_for_fault("worker-replaced");
            self.observe(
                user_id,
                entity_id,
                HealthState::Critical,
                "worker_replaced",
                json!({
                    "botId": bot_id,
                    "reason": "replacement requires a fresh immutable worker process",
                }),
            )?;
        }
        let worker = WorkerSupervisor::launch(request).map_err(|error| {
            let _ = self.observe(
                user_id,
                entity_id,
                HealthState::Critical,
                "worker_start_failed",
                json!({ "botId": bot_id, "error": error }),
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
                    json!({ "botId": bot_id, "requestId": request_id, "error": error }),
                );
            }
        }
        health_events.extend(worker.worker.take_health_events());
        for event in health_events {
            self.observe_worker_event(user_id, entity_id, bot_id, event)?;
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

    fn observe_worker_event(
        &self,
        user_id: &str,
        entity_id: &str,
        bot_id: &str,
        event: WorkerHealthEvent,
    ) -> Result<(), String> {
        let (state, condition, evidence) = match event {
            WorkerHealthEvent::Heartbeat {
                observed_at_ms,
                state,
            } => (
                HealthState::Healthy,
                "worker_heartbeat",
                json!({
                    "botId": bot_id,
                    "state": format!("{state:?}"),
                    "observedAtMs": observed_at_ms,
                }),
            ),
            WorkerHealthEvent::Diagnostic { code, detail } => (
                HealthState::Degraded,
                "worker_diagnostic",
                json!({ "botId": bot_id, "code": code, "detail": detail }),
            ),
            WorkerHealthEvent::Fault { code, detail } => (
                HealthState::Critical,
                "worker_fault",
                json!({ "botId": bot_id, "code": code, "detail": detail }),
            ),
        };
        self.observe(user_id, entity_id, state, condition, evidence)
    }

    fn observe(
        &self,
        user_id: &str,
        entity_id: &str,
        state: HealthState,
        condition: &str,
        evidence: serde_json::Value,
    ) -> Result<(), String> {
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
            })
            .map(|_| ())
    }
}
