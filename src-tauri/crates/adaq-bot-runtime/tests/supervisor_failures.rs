use adaq_bot_runtime::{
    DecisionClock, DeploymentBundle, DeploymentBundleInput, LifecycleState, StrategyWorld,
    WORKER_ARTIFACT_NAME, WORKER_ARTIFACT_VERSION, WORKER_PROTOCOL_VERSION, WORKER_RUNTIME_VERSION,
    WORKER_SIGNING_KEY_ID, WorkerArtifactBinding, WorkerArtifactSignature, WorkerArtifactVerifier,
    WorkerDecisionInput, WorkerFeatureFrame, WorkerLaunchRequest, WorkerRuntimePolicy,
    WorkerStrategyBinding, WorkerSupervisor, WorkerTrustRoot, current_platform_tag, sha256_hex,
    unix_now_ms,
};
use ed25519_dalek::SigningKey;
use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

const TEST_PRIVATE_KEY: [u8; 32] = [
    0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4f, 0xa4, 0x92, 0xec, 0x2c, 0xc4,
    0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03, 0x1c, 0xae, 0x7f, 0x60,
];

fn probe_binary() -> PathBuf {
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_adaq_worker_probe") {
        return PathBuf::from(path);
    }
    let test_binary = std::env::current_exe().expect("test binary path");
    let debug_dir = test_binary
        .parent()
        .and_then(Path::parent)
        .expect("test binary target directory");
    debug_dir.join(if cfg!(windows) {
        "adaq-worker-probe.exe"
    } else {
        "adaq-worker-probe"
    })
}

fn launch_probe(
    mode: &str,
    mut policy: WorkerRuntimePolicy,
) -> Result<(PathBuf, WorkerSupervisor), String> {
    let source = probe_binary();
    let temp_dir = std::env::temp_dir().join(format!(
        "adaq-bot-supervisor-{mode}-{}-{}",
        std::process::id(),
        unix_now_ms()
    ));
    fs::create_dir(&temp_dir).map_err(|error| error.to_string())?;
    let artifact_path = temp_dir.join(if cfg!(windows) {
        format!("adaq-worker-probe-{mode}.exe")
    } else {
        format!("adaq-worker-probe-{mode}")
    });
    if let Err(error) = fs::copy(&source, &artifact_path) {
        let _ = fs::remove_dir_all(&temp_dir);
        return Err(error.to_string());
    }
    let artifact_bytes = fs::read(&artifact_path).map_err(|error| error.to_string())?;
    let signature =
        WorkerArtifactSignature::sign(&artifact_bytes, current_platform_tag(), &TEST_PRIVATE_KEY)?;
    let signature_path = temp_dir.join("adaq-worker.sig");
    fs::write(
        &signature_path,
        serde_json::to_vec(&signature).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;

    policy.max_frame_bytes = policy.max_frame_bytes.max(16 * 1024);
    let component_wasm = b"component".to_vec();
    let component_sha256 = sha256_hex(&component_wasm);
    let worker = WorkerArtifactBinding {
        artifact_name: WORKER_ARTIFACT_NAME.into(),
        artifact_version: WORKER_ARTIFACT_VERSION.into(),
        platform: signature.platform.clone(),
        protocol_version: WORKER_PROTOCOL_VERSION.into(),
        runtime_version: WORKER_RUNTIME_VERSION.into(),
        sha256: signature.artifact_sha256.clone(),
        signing_key_id: WORKER_SIGNING_KEY_ID.into(),
        signature: signature.signature.clone(),
    };
    let bundle = match DeploymentBundle::freeze(DeploymentBundleInput {
        bot_id: format!("failure-{mode}"),
        strategy_id: "failure-probe".into(),
        account_id: "paper-account".into(),
        component_hashes: vec![component_sha256.clone()],
        model_hashes: vec![],
        feature_plan_hash: "f".repeat(64),
        risk_policy_hash: "d".repeat(64),
        execution_profile_hash: "e".repeat(64),
        worker_binary_hash: worker.sha256.clone(),
        qualification_evidence_hash: "b".repeat(64),
        strategy: WorkerStrategyBinding {
            world: StrategyWorld::Strategy,
            component_sha256,
            feature_slots: vec!["close".into()],
            parameters: vec![],
        },
        pipeline: Default::default(),
        worker,
        worker_policy: policy,
    }) {
        Ok(bundle) => bundle,
        Err(error) => {
            let _ = fs::remove_dir_all(&temp_dir);
            return Err(error.to_string());
        }
    };
    let public_key = SigningKey::from_bytes(&TEST_PRIVATE_KEY)
        .verifying_key()
        .to_bytes();
    let verifier = WorkerArtifactVerifier::with_trust_root(WorkerTrustRoot {
        key_id: WORKER_SIGNING_KEY_ID.into(),
        public_key,
    });
    match WorkerSupervisor::launch_with_verifier(
        WorkerLaunchRequest {
            bundle,
            artifact_path,
            signature_path,
            component_wasm,
            pipeline_components: vec![],
            extra_args: vec![],
        },
        verifier,
    ) {
        Ok(supervisor) => Ok((temp_dir, supervisor)),
        Err(error) => {
            let _ = fs::remove_dir_all(&temp_dir);
            Err(error)
        }
    }
}

fn assert_faulted(mut supervisor: WorkerSupervisor) {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let events = supervisor.poll_health();
        if supervisor.state() == LifecycleState::Faulted {
            assert!(
                events.iter().any(|event| matches!(
                    event,
                    adaq_bot_runtime::WorkerHealthEvent::Fault { .. }
                ))
            );
            return;
        }
        assert!(Instant::now() < deadline, "worker did not fault in time");
        thread::sleep(Duration::from_millis(5));
    }
}

fn remove_temp_dir(temp_dir: PathBuf, supervisor: WorkerSupervisor) {
    drop(supervisor);
    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn malformed_worker_output_faults_the_supervisor() -> Result<(), String> {
    let (temp_dir, supervisor) = launch_probe("malformed", WorkerRuntimePolicy::default())?;
    assert_faulted(supervisor);
    let _ = fs::remove_dir_all(temp_dir);
    Ok(())
}

#[test]
fn oversized_worker_output_faults_the_supervisor() -> Result<(), String> {
    let (temp_dir, supervisor) = launch_probe(
        "oversized",
        WorkerRuntimePolicy {
            max_frame_bytes: 16 * 1024,
            max_output_bytes: 8 * 1024,
            ..WorkerRuntimePolicy::default()
        },
    )?;
    assert_faulted(supervisor);
    let _ = fs::remove_dir_all(temp_dir);
    Ok(())
}

#[test]
fn crashed_worker_faults_the_supervisor() -> Result<(), String> {
    let (temp_dir, supervisor) = launch_probe("crash", WorkerRuntimePolicy::default())?;
    assert_faulted(supervisor);
    let _ = fs::remove_dir_all(temp_dir);
    Ok(())
}

#[test]
fn missed_heartbeat_faults_the_supervisor() -> Result<(), String> {
    let (temp_dir, supervisor) = launch_probe(
        "missed-heartbeat",
        WorkerRuntimePolicy {
            heartbeat_interval_ms: 10,
            heartbeat_timeout_ms: 30,
            ..WorkerRuntimePolicy::default()
        },
    )?;
    thread::sleep(Duration::from_millis(50));
    assert_faulted(supervisor);
    let _ = fs::remove_dir_all(temp_dir);
    Ok(())
}

#[test]
fn late_worker_response_faults_the_supervisor() -> Result<(), String> {
    let (temp_dir, mut supervisor) = launch_probe(
        "late",
        WorkerRuntimePolicy {
            decision_timeout_ms: 20,
            ..WorkerRuntimePolicy::default()
        },
    )?;
    for state in [
        LifecycleState::Reconciling,
        LifecycleState::WarmingUp,
        LifecycleState::Running,
    ] {
        supervisor.transition(state, "test", "failure probe")?;
    }
    let now = unix_now_ms();
    let result = supervisor.decision(
        "late-request".into(),
        DecisionClock::ClosedBar {
            decision_id: "late-decision".into(),
            instrument_id: "BTC-USDT".into(),
            decision_time_ms: now,
            available_at_ms: now,
            deadline_ms: now + 5_000,
            next_execution_ms: now + 5_001,
        },
        WorkerDecisionInput::Strategy {
            instrument_id: "BTC-USDT".into(),
            frames: vec![WorkerFeatureFrame {
                instrument_id: "BTC-USDT".into(),
                open_time_ms: now,
                available_at_ms: now,
                values: vec![Some(1.0)],
            }],
        },
    );
    assert!(result.is_err());
    assert_eq!(supervisor.state(), LifecycleState::Faulted);
    remove_temp_dir(temp_dir, supervisor);
    Ok(())
}

#[test]
fn handshake_failure_is_rejected_and_faulted() {
    let result = launch_probe("handshake-failure", WorkerRuntimePolicy::default());
    match result {
        Ok((temp_dir, supervisor)) => {
            remove_temp_dir(temp_dir, supervisor);
            panic!("handshake failure unexpectedly launched");
        }
        Err(error) => assert_eq!(error, "worker-handshake-failed"),
    }
}
