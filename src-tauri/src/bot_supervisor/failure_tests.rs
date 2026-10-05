use super::*;
use crate::bot_operations::{BotDeploymentBundle, BotStopPolicy, tests::bundle};
use adaq_bot_runtime::{
    DeploymentBundle, WORKER_SIGNING_KEY_ID, WorkerArtifactSignature, WorkerArtifactVerifier,
    WorkerTrustRoot, current_platform_tag, sha256_hex,
};
use rusqlite::Connection;
use std::{fs, path::PathBuf, time::SystemTime};

const USER: &str = "user-a";
const BOT: &str = "bot-a";
const CODE: &str = "worker-decision-failed";
const DETAIL: &str = "retained original decision failure";

struct Harness {
    database: Arc<Mutex<Connection>>,
    bots: BotStore,
    operations: OperationsStore,
    supervisor: Arc<BotSupervisor>,
}

fn harness() -> Harness {
    harness_with_bundle(bundle(BOT, "account-a"))
}

fn harness_with_bundle(deployment: BotDeploymentBundle) -> Harness {
    let database = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
    let bots = BotStore::open(database.clone()).unwrap();
    let operations = OperationsStore::open(database.clone()).unwrap();
    let supervisor = Arc::new(BotSupervisor::new(operations.clone(), bots.clone()).unwrap());
    bots.deploy(USER, deployment).unwrap();
    bots.begin_attempt(USER, BOT, false).unwrap();
    for state in [
        LifecycleState::Reconciling,
        LifecycleState::WarmingUp,
        LifecycleState::Running,
    ] {
        bots.transition(USER, BOT, state, "host", "test").unwrap();
    }
    Harness {
        database,
        bots,
        operations,
        supervisor,
    }
}

fn sql(harness: &Harness, statement: &str) {
    harness
        .database
        .lock()
        .unwrap()
        .execute_batch(statement)
        .unwrap();
}

fn reject_observations(harness: &Harness) {
    sql(
        harness,
        "CREATE TEMP TRIGGER reject_observations BEFORE INSERT ON operational_events
         BEGIN SELECT RAISE(ABORT, 'injected Operations failure'); END;",
    );
}

fn reject_bot_writes(harness: &Harness) {
    sql(
        harness,
        "CREATE TEMP TRIGGER reject_bot_writes BEFORE UPDATE ON bots
         BEGIN SELECT RAISE(ABORT, 'injected Bot persistence failure'); END;",
    );
}

fn assert_fault_retained(harness: &Harness, code: &str, detail: &str) {
    let view = harness.bots.get(USER, BOT).unwrap();
    let attempt = &view.attempts[0];
    assert_eq!(view.state, LifecycleState::Faulted);
    assert_eq!(attempt.state, LifecycleState::Faulted);
    assert!(attempt.reconciliation_required);
    assert_eq!(
        attempt
            .evidence
            .iter()
            .filter(|item| item.kind == "recovery" && item.code == code && item.detail == detail)
            .count(),
        1,
    );
    assert_eq!(
        attempt
            .events
            .iter()
            .filter(|event| event.to == LifecycleState::Faulted)
            .count(),
        1,
    );
}

#[test]
fn observation_failure_does_not_prevent_durable_fault_and_can_be_retried() {
    let harness = harness();
    reject_observations(&harness);
    let error = harness
        .supervisor
        .fail_active(USER, BOT, CODE, DETAIL)
        .unwrap_err();
    assert!(error.contains(CODE));
    assert!(error.contains("fault observation: injected Operations failure"));
    assert_fault_retained(&harness, CODE, DETAIL);
    assert!(
        harness
            .operations
            .events_for_user(USER, 10)
            .unwrap()
            .is_empty()
    );

    sql(&harness, "DROP TRIGGER reject_observations;");
    harness
        .supervisor
        .fail_active(USER, BOT, CODE, DETAIL)
        .unwrap();
    assert_fault_retained(&harness, CODE, DETAIL);
    let events = harness.operations.events_for_user(USER, 10).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "worker.worker-lifecycle-faulted");
}

#[test]
fn persistence_failure_preserves_the_attempt_and_still_records_operations_evidence() {
    let harness = harness();
    reject_bot_writes(&harness);
    let error = harness
        .supervisor
        .fail_active(USER, BOT, CODE, DETAIL)
        .unwrap_err();
    assert!(error.contains("Bot state persistence: injected Bot persistence failure"));
    let unchanged = harness.bots.get(USER, BOT).unwrap();
    assert_eq!(unchanged.state, LifecycleState::Running);
    assert_eq!(unchanged.attempts[0].state, LifecycleState::Running);
    assert!(!unchanged.attempts[0].reconciliation_required);
    let events = harness.operations.events_for_user(USER, 10).unwrap();
    assert!(
        events
            .iter()
            .any(|event| event.kind == "worker.worker-lifecycle-faulted")
    );
    assert!(
        events
            .iter()
            .any(|event| event.kind == "worker.bot-state-persistence-failed")
    );
    assert!(
        events
            .iter()
            .all(|event| event.evidence_id.as_deref() == unchanged.current_attempt_id.as_deref())
    );

    sql(&harness, "DROP TRIGGER reject_bot_writes;");
    harness
        .supervisor
        .fail_active(USER, BOT, CODE, DETAIL)
        .unwrap();
    assert_fault_retained(&harness, CODE, DETAIL);
}

#[test]
fn simultaneous_failures_report_every_stage_in_safety_order() {
    let harness = harness();
    reject_bot_writes(&harness);
    reject_observations(&harness);
    let error = harness
        .supervisor
        .fail_active(USER, BOT, CODE, DETAIL)
        .unwrap_err();
    let persistence = error.find("Bot state persistence:").unwrap();
    let observation = error.find("fault observation:").unwrap();
    let diagnostic = error.find("persistence failure observation:").unwrap();
    assert!(persistence < observation && observation < diagnostic);
    assert!(error.contains(DETAIL));
    assert!(error.contains("injected Bot persistence failure"));
    assert!(error.contains("injected Operations failure"));
    assert_eq!(
        harness.bots.get(USER, BOT).unwrap().state,
        LifecycleState::Running
    );
}

#[test]
fn repeated_faults_preserve_terminal_states_and_one_recovery_record() {
    for stopped in [false, true] {
        let harness = harness();
        if stopped {
            harness
                .bots
                .complete_stop(USER, BOT, BotStopPolicy::KeepPosition, vec![], true)
                .unwrap();
        }
        for _ in 0..3 {
            harness
                .supervisor
                .fail_active(USER, BOT, CODE, DETAIL)
                .unwrap();
        }
        let view = harness.bots.get(USER, BOT).unwrap();
        let expected = if stopped {
            LifecycleState::Stopped
        } else {
            LifecycleState::Faulted
        };
        assert_eq!(view.state, expected);
        assert_eq!(view.attempts[0].state, expected);
        assert!(view.attempts[0].reconciliation_required);
        assert_eq!(
            view.attempts[0]
                .evidence
                .iter()
                .filter(|item| item.kind == "recovery" && item.code == CODE)
                .count(),
            1
        );
        assert_eq!(
            view.attempts[0]
                .events
                .iter()
                .filter(|event| event.to == LifecycleState::Faulted)
                .count(),
            usize::from(!stopped)
        );
    }
}

#[test]
fn transition_and_decision_failures_keep_their_original_recovery_codes() {
    for transition in [false, true] {
        let harness = harness();
        reject_observations(&harness);
        let (code, result) = if transition {
            (
                "lifecycle-transition-failed",
                harness.supervisor.fail_transition(USER, BOT, DETAIL),
            )
        } else {
            (
                CODE,
                harness.supervisor.fail_decision(USER, BOT, CODE, DETAIL),
            )
        };
        assert!(result.unwrap_err().contains(code));
        assert_fault_retained(&harness, code, DETAIL);
    }
}

#[test]
fn delayed_observations_keep_the_originating_attempt_after_retry() {
    let harness = harness();
    let identity = harness.supervisor.attempt_identity(USER, BOT).unwrap();
    harness
        .bots
        .fault(USER, BOT, "test-retired", "test retry boundary")
        .unwrap();
    let (retry_id, _) = harness.bots.begin_attempt(USER, BOT, true).unwrap();
    let retry_before = harness.bots.get(USER, BOT).unwrap().attempts[1].clone();
    for event in [
        WorkerHealthEvent::Diagnostic {
            code: "late-diagnostic".into(),
            detail: "old Worker diagnostic".into(),
        },
        WorkerHealthEvent::Fault {
            code: "late-fault".into(),
            detail: "old Worker fault observation".into(),
        },
    ] {
        harness
            .supervisor
            .observe_worker_event(USER, BOT, BOT, Some(&identity), event)
            .unwrap();
    }
    let view = harness.bots.get(USER, BOT).unwrap();
    assert_eq!(view.current_attempt_id.as_deref(), Some(retry_id.as_str()));
    assert_eq!(view.state, LifecycleState::Starting);
    assert_eq!(view.attempts[1], retry_before);
    assert_eq!(
        view.attempts[0]
            .evidence
            .iter()
            .filter(|item| item.kind == "health"
                && item.related_id.as_deref() == Some(identity.attempt_id.as_str()))
            .count(),
        2
    );
    assert!(
        view.attempts[1]
            .evidence
            .iter()
            .all(|item| item.kind != "health")
    );
    let events = harness.operations.events_for_user(USER, 10).unwrap();
    assert_eq!(events.len(), 2);
    assert!(events.iter().all(|event| {
        event.evidence_id.as_deref() == Some(identity.attempt_id.as_str())
            && event.correlation_id.as_deref() == Some(identity.bundle_identity.as_str())
            && event.evidence["attemptId"] == identity.attempt_id
            && event.evidence["bundleId"] == identity.bundle_identity
    }));
}

#[test]
fn fault_observation_captures_identity_before_a_concurrent_retry() {
    let harness = harness();
    let origin = harness.supervisor.attempt_identity(USER, BOT).unwrap();
    harness
        .supervisor
        .converge_fault(
            USER,
            BOT,
            BOT,
            &WorkerFault {
                code: CODE.into(),
                detail: DETAIL.into(),
            },
            None,
            |identity| {
                harness.bots.begin_attempt(USER, BOT, true)?;
                harness.supervisor.observe_at(
                    USER,
                    BOT,
                    HealthState::Critical,
                    "delayed_fault",
                    json!({ "botId": BOT }),
                    identity,
                )
            },
        )
        .unwrap();
    let view = harness.bots.get(USER, BOT).unwrap();
    assert_ne!(
        view.current_attempt_id.as_deref(),
        Some(origin.attempt_id.as_str())
    );
    assert_eq!(view.state, LifecycleState::Starting);
    assert_eq!(view.attempts[0].state, LifecycleState::Faulted);
    assert_eq!(view.attempts[1].state, LifecycleState::Starting);
    assert!(
        view.attempts[1]
            .evidence
            .iter()
            .all(|item| item.code != CODE)
    );
    let events = harness.operations.events_for_user(USER, 10).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].evidence_id.as_deref(),
        Some(origin.attempt_id.as_str())
    );
    assert_eq!(events[0].evidence["attemptId"], origin.attempt_id);
}

#[test]
fn worker_evidence_requires_the_exact_user_attempt_and_bundle() {
    let harness = harness();
    let identity = harness.supervisor.attempt_identity(USER, BOT).unwrap();
    let before = serde_json::to_value(harness.bots.get(USER, BOT).unwrap()).unwrap();
    for (user_id, identity) in [
        ("another-user", identity.clone()),
        (
            USER,
            BotAttemptIdentity {
                attempt_id: "another-attempt".into(),
                ..identity.clone()
            },
        ),
        (
            USER,
            BotAttemptIdentity {
                bundle_identity: "another-bundle".into(),
                ..identity.clone()
            },
        ),
    ] {
        assert!(
            harness
                .supervisor
                .observe_worker_event(
                    user_id,
                    BOT,
                    BOT,
                    Some(&identity),
                    WorkerHealthEvent::Diagnostic {
                        code: "invalid-identity".into(),
                        detail: "must not be recorded".into()
                    }
                )
                .is_err()
        );
        assert_eq!(
            serde_json::to_value(harness.bots.get(USER, BOT).unwrap()).unwrap(),
            before
        );
        assert!(
            harness
                .operations
                .events_for_user(USER, 10)
                .unwrap()
                .is_empty()
        );
    }
}

struct ProbeHarness {
    harness: Harness,
    temp_dir: PathBuf,
}

impl Drop for ProbeHarness {
    fn drop(&mut self) {
        if let Ok(mut workers) = self.harness.supervisor.workers.lock() {
            workers.clear();
        }
        let _ = fs::remove_dir_all(&self.temp_dir);
    }
}

fn probe_harness() -> ProbeHarness {
    // This is the order-free protocol probe, signed with a public Ed25519 test
    // vector, never with an application signing key.
    let private_key = [
        0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec, 0x2c,
        0xc4, 0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03, 0x1c, 0xae,
        0x7f, 0x60,
    ];
    let public_key = [
        0xd7, 0x5a, 0x98, 0x01, 0x82, 0xb1, 0x0a, 0xb7, 0xd5, 0x4b, 0xfe, 0xd3, 0xc9, 0x64, 0x07,
        0x3a, 0x0e, 0xe1, 0x72, 0xf3, 0xda, 0xa6, 0x23, 0x25, 0xaf, 0x02, 0x1a, 0x68, 0xf7, 0x07,
        0x51, 0x1a,
    ];
    let debug_dir = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_owned();
    let source = debug_dir.join(if cfg!(windows) {
        "adaq-worker-probe.exe"
    } else {
        "adaq-worker-probe"
    });
    assert!(
        source.is_file(),
        "build the adaq-worker-probe before running these ignored tests"
    );
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!(
        "adaq-host-fault-probe-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir(&temp_dir).unwrap();
    let artifact_path = temp_dir.join(if cfg!(windows) {
        "adaq-worker-probe-heartbeats-while-idle.exe"
    } else {
        "adaq-worker-probe-heartbeats-while-idle"
    });
    fs::copy(source, &artifact_path).unwrap();
    let bytes = fs::read(&artifact_path).unwrap();
    let signature =
        WorkerArtifactSignature::sign(&bytes, current_platform_tag(), &private_key).unwrap();
    let signature_path = temp_dir.join("worker.sig");
    fs::write(&signature_path, serde_json::to_vec(&signature).unwrap()).unwrap();
    let component_wasm = b"component".to_vec();
    let mut deployment = bundle(BOT, "account-a");
    let mut input = deployment.runtime_bundle.input.clone();
    input.worker.sha256 = signature.artifact_sha256.clone();
    input.worker.signature = signature.signature;
    input.worker_binary_hash = input.worker.sha256.clone();
    input.strategy.component_sha256 = sha256_hex(&component_wasm);
    input.component_hashes = vec![input.strategy.component_sha256.clone()];
    deployment.runtime_bundle = DeploymentBundle::freeze(input).unwrap();
    let deployment = deployment.freeze().unwrap();
    let mut worker = WorkerSupervisor::launch_with_verifier(
        WorkerLaunchRequest {
            bundle: deployment.runtime_bundle.clone(),
            artifact_path,
            signature_path,
            component_wasm,
            pipeline_components: vec![],
            extra_args: vec![],
        },
        WorkerArtifactVerifier::with_trust_root(WorkerTrustRoot {
            key_id: WORKER_SIGNING_KEY_ID.into(),
            public_key,
        }),
    )
    .unwrap();
    for state in [
        LifecycleState::Reconciling,
        LifecycleState::WarmingUp,
        LifecycleState::Running,
    ] {
        worker.transition(state, "test", "fault probe").unwrap();
    }
    let harness = harness_with_bundle(deployment);
    harness.supervisor.workers.lock().unwrap().insert(
        BOT.into(),
        ManagedWorker {
            worker,
            user_id: USER.into(),
            entity_id: BOT.into(),
            registration: Arc::new(()),
            identity: harness.supervisor.attempt_identity(USER, BOT).unwrap(),
            pending_fault: None,
        },
    );
    ProbeHarness { harness, temp_dir }
}

#[test]
#[ignore = "Build cargo build -p adaq-bot-runtime --bin adaq-worker-probe, then run the host_worker_probe tests with --ignored"]
fn host_worker_probe_observation_failure_still_terminates_and_persists() {
    let probe = probe_harness();
    reject_observations(&probe.harness);
    assert!(
        probe
            .harness
            .supervisor
            .fail_active(USER, BOT, CODE, DETAIL)
            .is_err()
    );
    assert!(!probe.harness.supervisor.has_worker(BOT).unwrap());
    assert_fault_retained(&probe.harness, CODE, DETAIL);
}

#[test]
#[ignore = "Build cargo build -p adaq-bot-runtime --bin adaq-worker-probe, then run the host_worker_probe tests with --ignored"]
fn host_worker_probe_retries_a_drained_fault_after_persistence_recovers() {
    let probe = probe_harness();
    reject_bot_writes(&probe.harness);
    assert!(
        probe
            .harness
            .supervisor
            .fail_active(USER, BOT, CODE, DETAIL)
            .is_err()
    );
    {
        let workers = probe.harness.supervisor.workers.lock().unwrap();
        let managed = &workers[BOT];
        assert_eq!(managed.worker.state(), LifecycleState::Faulted);
        let fault = managed.pending_fault.as_ref().unwrap();
        assert_eq!(fault.code, CODE);
        assert_eq!(fault.detail, DETAIL);
    }
    probe.harness.supervisor.poll_workers();
    assert!(probe.harness.supervisor.has_worker(BOT).unwrap());
    assert_eq!(
        probe.harness.bots.get(USER, BOT).unwrap().state,
        LifecycleState::Running
    );

    sql(&probe.harness, "DROP TRIGGER reject_bot_writes;");
    probe.harness.supervisor.poll_workers();
    assert!(!probe.harness.supervisor.has_worker(BOT).unwrap());
    assert_fault_retained(&probe.harness, CODE, DETAIL);
}

#[test]
#[ignore = "Build cargo build -p adaq-bot-runtime --bin adaq-worker-probe, then run the host_worker_probe tests with --ignored"]
fn host_worker_probe_monitor_fault_uses_the_same_safety_convergence() {
    let probe = probe_harness();
    reject_observations(&probe.harness);
    let registration = probe.harness.supervisor.workers.lock().unwrap()[BOT]
        .registration
        .clone();
    let error = probe
        .harness
        .supervisor
        .handle_worker_event(
            USER,
            BOT,
            BOT,
            &registration,
            WorkerHealthEvent::Fault {
                code: CODE.into(),
                detail: DETAIL.into(),
            },
        )
        .unwrap_err();
    assert!(error.contains("fault observation:"));
    assert!(!probe.harness.supervisor.has_worker(BOT).unwrap());
    assert_fault_retained(&probe.harness, CODE, DETAIL);
}

#[test]
#[ignore = "Build cargo build -p adaq-bot-runtime --bin adaq-worker-probe, then run the host_worker_probe tests with --ignored"]
fn host_worker_probe_retired_registration_cannot_fault_a_replacement() {
    let probe = probe_harness();
    let retired = {
        let mut workers = probe.harness.supervisor.workers.lock().unwrap();
        let managed = workers.get_mut(BOT).unwrap();
        let retired = managed.registration.clone();
        managed.registration = Arc::new(());
        retired
    };
    probe
        .harness
        .supervisor
        .handle_worker_event(
            USER,
            BOT,
            BOT,
            &retired,
            WorkerHealthEvent::Fault {
                code: CODE.into(),
                detail: DETAIL.into(),
            },
        )
        .unwrap();
    let workers = probe.harness.supervisor.workers.lock().unwrap();
    assert_eq!(workers[BOT].worker.state(), LifecycleState::Running);
    assert!(workers[BOT].pending_fault.is_none());
    assert_eq!(
        probe.harness.bots.get(USER, BOT).unwrap().state,
        LifecycleState::Running
    );
    assert!(
        probe
            .harness
            .operations
            .events_for_user(USER, 10)
            .unwrap()
            .is_empty()
    );
}

#[test]
#[ignore = "Build cargo build -p adaq-bot-runtime --bin adaq-worker-probe, then run the host_worker_probe tests with --ignored"]
fn host_worker_probe_retired_registration_cannot_record_diagnostics() {
    let probe = probe_harness();
    let retired = {
        let mut workers = probe.harness.supervisor.workers.lock().unwrap();
        let managed = workers.get_mut(BOT).unwrap();
        let retired = managed.registration.clone();
        managed.registration = Arc::new(());
        retired
    };
    let before = serde_json::to_value(probe.harness.bots.get(USER, BOT).unwrap()).unwrap();
    probe
        .harness
        .supervisor
        .handle_worker_event(
            USER,
            BOT,
            BOT,
            &retired,
            WorkerHealthEvent::Diagnostic {
                code: "late-diagnostic".into(),
                detail: "retired registration".into(),
            },
        )
        .unwrap();
    assert_eq!(
        serde_json::to_value(probe.harness.bots.get(USER, BOT).unwrap()).unwrap(),
        before
    );
    assert!(
        probe
            .harness
            .operations
            .events_for_user(USER, 10)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        probe.harness.supervisor.workers.lock().unwrap()[BOT]
            .worker
            .state(),
        LifecycleState::Running
    );
}

#[test]
#[ignore = "Build cargo build -p adaq-bot-runtime --bin adaq-worker-probe, then run the host_worker_probe tests with --ignored"]
fn host_worker_probe_old_attempt_fault_cannot_fault_a_retry() {
    let probe = probe_harness();
    let registration = probe.harness.supervisor.workers.lock().unwrap()[BOT]
        .registration
        .clone();
    probe
        .harness
        .bots
        .fault(USER, BOT, "test-retired", "test retry boundary")
        .unwrap();
    let (retry_id, _) = probe.harness.bots.begin_attempt(USER, BOT, true).unwrap();
    let before = serde_json::to_value(probe.harness.bots.get(USER, BOT).unwrap()).unwrap();
    probe
        .harness
        .supervisor
        .handle_worker_event(
            USER,
            BOT,
            BOT,
            &registration,
            WorkerHealthEvent::Fault {
                code: CODE.into(),
                detail: DETAIL.into(),
            },
        )
        .unwrap();
    assert!(!probe.harness.supervisor.has_worker(BOT).unwrap());
    let view = probe.harness.bots.get(USER, BOT).unwrap();
    assert_eq!(view.current_attempt_id.as_deref(), Some(retry_id.as_str()));
    assert_eq!(view.state, LifecycleState::Starting);
    assert_eq!(serde_json::to_value(view).unwrap(), before);
    assert!(
        probe
            .harness
            .operations
            .events_for_user(USER, 10)
            .unwrap()
            .is_empty()
    );
}
