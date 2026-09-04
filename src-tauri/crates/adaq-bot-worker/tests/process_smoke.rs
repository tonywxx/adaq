use adaq_bot_runtime::{
    DecisionClock, DeploymentBundle, DeploymentBundleInput, LifecycleState, NoTargetReason,
    StrategyWorld, WORKER_ARTIFACT_NAME, WORKER_PROTOCOL_VERSION, WORKER_RUNTIME_VERSION,
    WORKER_SIGNING_KEY_ID, WorkerArtifactBinding, WorkerArtifactSignature, WorkerArtifactVerifier,
    WorkerComponentLaunch, WorkerDecisionInput, WorkerDecisionResult, WorkerFactorBinding,
    WorkerFactorScope, WorkerFeatureFrame, WorkerFeatureRow, WorkerLaunchRequest,
    WorkerModelBinding, WorkerParameterValue, WorkerPipelineBinding, WorkerPortfolioState,
    WorkerRuntimePolicy, WorkerStrategyBinding, WorkerSupervisor, WorkerTarget, WorkerTrustRoot,
    current_platform_tag, sha256_hex, unix_now_ms,
};
use ed25519_dalek::SigningKey;
use std::{
    fs,
    path::{Path, PathBuf},
};

const TEST_PRIVATE_KEY: [u8; 32] = [
    0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4f, 0xa4, 0x92, 0xec, 0x2c, 0xc4,
    0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03, 0x1c, 0xae, 0x7f, 0x60,
];
const PROCESS_SMOKE_HANDSHAKE_TIMEOUT_MS: u64 = 20_000;

struct FactorFixture {
    fixture: &'static str,
    component_file: &'static str,
    scope: WorkerFactorScope,
    feature_slots: Vec<String>,
    output_names: Vec<String>,
    warmup_bars: u64,
}

struct ModelFixture {
    fixture: &'static str,
    component_file: &'static str,
    feature_slots: Vec<String>,
    output_names: Vec<String>,
}

enum PipelineFixture {
    Factor(FactorFixture),
    Model(ModelFixture),
}

fn launch_worker(
    fixture: &str,
    component_file: &str,
    strategy_id: &str,
    world: StrategyWorld,
    feature_slots: Vec<String>,
    warmup_decisions: u64,
    pipeline_fixture: Option<PipelineFixture>,
) -> Result<(PathBuf, WorkerSupervisor), String> {
    let worker_binary_path = match std::env::var("CARGO_BIN_EXE_adaq_bot_worker") {
        Ok(path) => PathBuf::from(path),
        Err(_) => {
            let test_binary = std::env::current_exe().map_err(|error| error.to_string())?;
            let debug_dir = test_binary
                .parent()
                .and_then(Path::parent)
                .ok_or_else(|| "test binary has no target directory".to_owned())?;
            debug_dir.join(if cfg!(windows) {
                "adaq-bot-worker.exe"
            } else {
                "adaq-bot-worker"
            })
        }
    };
    let component_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(fixture)
        .join("target/wasm32-unknown-unknown/debug")
        .join(component_file);
    let component_wasm = fs::read(&component_path)
        .map_err(|error| format!("fixture {}: {error}", component_path.display()))?;
    let component_sha256 = sha256_hex(&component_wasm);
    let (pipeline, pipeline_components, component_hashes, model_hashes) = match pipeline_fixture {
        Some(PipelineFixture::Factor(factor)) => {
            let factor_path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures")
                .join(factor.fixture)
                .join("target/wasm32-unknown-unknown/debug")
                .join(factor.component_file);
            let factor_wasm = fs::read(&factor_path)
                .map_err(|error| format!("fixture {}: {error}", factor_path.display()))?;
            let factor_sha256 = sha256_hex(&factor_wasm);
            (
                WorkerPipelineBinding {
                    input_slots: factor.feature_slots.clone(),
                    factors: vec![WorkerFactorBinding {
                        scope: factor.scope,
                        component_sha256: factor_sha256.clone(),
                        feature_slots: factor.feature_slots,
                        output_names: factor.output_names,
                        warmup_bars: factor.warmup_bars,
                        parameters: vec![WorkerParameterValue::Integer(1)],
                    }],
                    models: vec![],
                },
                vec![WorkerComponentLaunch {
                    component_sha256: factor_sha256.clone(),
                    wasm: factor_wasm,
                }],
                vec![component_sha256.clone(), factor_sha256],
                vec![],
            )
        }
        Some(PipelineFixture::Model(model)) => {
            let model_path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures")
                .join(model.fixture)
                .join("target/wasm32-unknown-unknown/debug")
                .join(model.component_file);
            let model_wasm = fs::read(&model_path)
                .map_err(|error| format!("fixture {}: {error}", model_path.display()))?;
            let model_sha256 = sha256_hex(&model_wasm);
            (
                WorkerPipelineBinding {
                    input_slots: model.feature_slots.clone(),
                    factors: vec![],
                    models: vec![WorkerModelBinding {
                        component_sha256: model_sha256.clone(),
                        feature_slots: model.feature_slots,
                        output_names: model.output_names,
                        seed: 7,
                        parameters: vec![WorkerParameterValue::String("valid".into())],
                    }],
                },
                vec![WorkerComponentLaunch {
                    component_sha256: model_sha256.clone(),
                    wasm: model_wasm,
                }],
                vec![component_sha256.clone(), model_sha256.clone()],
                vec![model_sha256],
            )
        }
        None => (
            WorkerPipelineBinding::default(),
            vec![],
            vec![component_sha256.clone()],
            vec![],
        ),
    };
    let temp_dir = std::env::temp_dir().join(format!(
        "adaq-bot-worker-smoke-{fixture}-{}-{}",
        std::process::id(),
        unix_now_ms()
    ));
    fs::create_dir(&temp_dir).map_err(|error| error.to_string())?;
    let artifact_name = worker_binary_path
        .file_name()
        .ok_or_else(|| "worker binary has no file name".to_owned())?;
    let artifact_path = temp_dir.join(artifact_name);
    fs::copy(&worker_binary_path, &artifact_path).map_err(|error| error.to_string())?;
    let signature_path = temp_dir.join("adaq-bot-worker.sig");
    let signature = WorkerArtifactSignature::sign(
        &fs::read(&artifact_path).map_err(|error| error.to_string())?,
        current_platform_tag(),
        &TEST_PRIVATE_KEY,
    )?;
    fs::write(
        &signature_path,
        serde_json::to_vec(&signature).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let worker = WorkerArtifactBinding {
        artifact_name: WORKER_ARTIFACT_NAME.into(),
        artifact_version: signature.artifact_version.clone(),
        platform: signature.platform.clone(),
        protocol_version: WORKER_PROTOCOL_VERSION.into(),
        runtime_version: WORKER_RUNTIME_VERSION.into(),
        sha256: signature.artifact_sha256.clone(),
        signing_key_id: WORKER_SIGNING_KEY_ID.into(),
        signature: signature.signature.clone(),
    };
    let bundle = DeploymentBundle::freeze(DeploymentBundleInput {
        bot_id: format!("{fixture}-worker-smoke-bot"),
        strategy_id: strategy_id.into(),
        account_id: "paper-account".into(),
        component_hashes,
        model_hashes,
        feature_plan_hash: "f".repeat(64),
        risk_policy_hash: "d".repeat(64),
        execution_profile_hash: "e".repeat(64),
        worker_binary_hash: worker.sha256.clone(),
        qualification_evidence_hash: "b".repeat(64),
        strategy: WorkerStrategyBinding {
            world,
            component_sha256,
            feature_slots,
            parameters: vec![],
        },
        pipeline,
        worker,
        worker_policy: WorkerRuntimePolicy {
            warmup_decisions,
            handshake_timeout_ms: PROCESS_SMOKE_HANDSHAKE_TIMEOUT_MS,
            ..WorkerRuntimePolicy::default()
        },
    })
    .map_err(|error| error.to_string())?;
    let public_key = SigningKey::from_bytes(&TEST_PRIVATE_KEY)
        .verifying_key()
        .to_bytes();
    let verifier = WorkerArtifactVerifier::with_trust_root(WorkerTrustRoot {
        key_id: WORKER_SIGNING_KEY_ID.into(),
        public_key,
    });
    let supervisor = WorkerSupervisor::launch_with_verifier(
        WorkerLaunchRequest {
            bundle,
            artifact_path,
            signature_path,
            component_wasm,
            pipeline_components,
            extra_args: vec![],
        },
        verifier,
    )?;
    Ok((temp_dir, supervisor))
}

fn running(supervisor: &mut WorkerSupervisor) -> Result<(), String> {
    for state in [
        LifecycleState::Reconciling,
        LifecycleState::WarmingUp,
        LifecycleState::Running,
    ] {
        supervisor.transition(state, "test", "process smoke")?;
    }
    Ok(())
}

#[test]
fn signed_strategy_worker_process_runs_warmup_and_target() -> Result<(), String> {
    let (temp_dir, mut supervisor) = launch_worker(
        "external-strategy",
        "m5_external_strategy_fixture.wasm",
        "external-strategy",
        StrategyWorld::Strategy,
        vec!["close-change".into()],
        0,
        Some(PipelineFixture::Factor(FactorFixture {
            fixture: "factor",
            component_file: "m1_factor_fixture.wasm",
            scope: WorkerFactorScope::TimeSeries,
            feature_slots: vec!["close".into(), "base-volume".into()],
            output_names: vec!["close-change".into()],
            warmup_bars: 1,
        })),
    )?;
    running(&mut supervisor)?;
    let decision_time_ms = unix_now_ms();
    let input = |open_time_ms| WorkerDecisionInput::Strategy {
        instrument_id: "BTC-USDT".into(),
        frames: vec![WorkerFeatureFrame {
            instrument_id: "BTC-USDT".into(),
            open_time_ms,
            available_at_ms: open_time_ms,
            values: vec![
                Some(if open_time_ms < decision_time_ms {
                    100.0
                } else {
                    101.0
                }),
                Some(1.0),
            ],
        }],
    };
    let clock = |decision_time_ms| DecisionClock::ClosedBar {
        decision_id: format!("bar-{decision_time_ms}"),
        instrument_id: "BTC-USDT".into(),
        decision_time_ms,
        available_at_ms: decision_time_ms,
        deadline_ms: decision_time_ms + 5_000,
        next_execution_ms: decision_time_ms + 5_001,
    };
    let warmup = supervisor
        .decision(
            "request-warmup".into(),
            clock(decision_time_ms),
            input(decision_time_ms - 1),
        )
        .map_err(|error| format!("warmup: {error}"))?;
    assert!(matches!(
        warmup,
        WorkerDecisionResult::NoTarget {
            reason: NoTargetReason::Warmup,
            ..
        }
    ));
    let target = supervisor
        .decision(
            "request-target".into(),
            clock(decision_time_ms + 1),
            input(decision_time_ms),
        )
        .map_err(|error| format!("target: {error}"))?;
    match target {
        WorkerDecisionResult::Target {
            decision_id,
            target:
                WorkerTarget::Strategy {
                    instrument_id,
                    exposures,
                },
            ..
        } => {
            assert_eq!(decision_id, format!("bar-{}", decision_time_ms + 1));
            assert_eq!(instrument_id, "BTC-USDT");
            assert_eq!(exposures[0].exposure, "1");
        }
        other => panic!("expected signed worker target, got {other:?}"),
    }
    supervisor.shutdown("request-stop")?;
    assert_eq!(supervisor.state(), LifecycleState::Stopped);
    let _ = fs::remove_dir_all(temp_dir);
    Ok(())
}

#[test]
fn signed_strategy_worker_processes_model_before_target() -> Result<(), String> {
    let (temp_dir, mut supervisor) = launch_worker(
        "strategy",
        "m1_strategy_fixture.wasm",
        "model-strategy",
        StrategyWorld::Strategy,
        vec!["quote-volume".into(), "close".into()],
        0,
        Some(PipelineFixture::Model(ModelFixture {
            fixture: "model",
            component_file: "m8_model_fixture.wasm",
            feature_slots: vec!["quote-volume".into()],
            output_names: vec!["close".into()],
        })),
    )?;
    running(&mut supervisor)?;
    let decision_time_ms = unix_now_ms();
    let result = supervisor
        .decision(
            "request-model".into(),
            DecisionClock::ClosedBar {
                decision_id: "model-bar".into(),
                instrument_id: "BTC-USDT".into(),
                decision_time_ms,
                available_at_ms: decision_time_ms,
                deadline_ms: decision_time_ms + 5_000,
                next_execution_ms: decision_time_ms + 5_001,
            },
            WorkerDecisionInput::Strategy {
                instrument_id: "BTC-USDT".into(),
                frames: vec![WorkerFeatureFrame {
                    instrument_id: "BTC-USDT".into(),
                    open_time_ms: decision_time_ms,
                    available_at_ms: decision_time_ms,
                    values: vec![Some(1.0)],
                }],
            },
        )
        .map_err(|error| format!("model pipeline: {error}"))?;
    assert!(matches!(
        result,
        WorkerDecisionResult::Target {
            target: WorkerTarget::Strategy { .. },
            ..
        }
    ));
    supervisor.shutdown("request-stop-model")?;
    assert_eq!(supervisor.state(), LifecycleState::Stopped);
    let _ = fs::remove_dir_all(temp_dir);
    Ok(())
}

#[test]
fn signed_portfolio_worker_processes_cross_section_factor_before_target() -> Result<(), String> {
    let (temp_dir, mut supervisor) = launch_worker(
        "portfolio-strategy",
        "m5_portfolio_strategy_fixture.wasm",
        "portfolio-factor-strategy",
        StrategyWorld::PortfolioStrategy,
        vec!["cross-sectional-score".into()],
        0,
        Some(PipelineFixture::Factor(FactorFixture {
            fixture: "cross-sectional-factor",
            component_file: "m11_cross_sectional_factor_fixture.wasm",
            scope: WorkerFactorScope::CrossSectional,
            feature_slots: vec!["close".into()],
            output_names: vec!["cross-sectional-score".into()],
            warmup_bars: 0,
        })),
    )?;
    running(&mut supervisor)?;
    let decision_time_ms = unix_now_ms();
    let result = supervisor
        .decision(
            "request-portfolio-factor".into(),
            DecisionClock::ScheduledCrossSection {
                decision_id: "factor-batch-1".into(),
                decision_time_ms,
                deadline_ms: decision_time_ms + 5_000,
                next_execution_ms: decision_time_ms + 5_001,
                universe: vec!["BTC-USDT".into(), "ETH-USDT".into()],
                available_instruments: vec!["BTC-USDT".into(), "ETH-USDT".into()],
            },
            WorkerDecisionInput::Portfolio {
                universe_id: "universe-factor".into(),
                rows: vec![
                    WorkerFeatureRow {
                        instrument_id: "BTC-USDT".into(),
                        available_at_ms: decision_time_ms,
                        values: vec![Some(1.0)],
                    },
                    WorkerFeatureRow {
                        instrument_id: "ETH-USDT".into(),
                        available_at_ms: decision_time_ms,
                        values: vec![Some(2.0)],
                    },
                ],
                state: WorkerPortfolioState {
                    cash: "1000".into(),
                    positions: vec![],
                },
            },
        )
        .map_err(|error| format!("portfolio factor pipeline: {error}"))?;
    assert!(matches!(
        result,
        WorkerDecisionResult::Target {
            target: WorkerTarget::Portfolio { .. },
            ..
        }
    ));
    supervisor.shutdown("request-stop-portfolio-factor")?;
    assert_eq!(supervisor.state(), LifecycleState::Stopped);
    let _ = fs::remove_dir_all(temp_dir);
    Ok(())
}

#[test]
fn signed_portfolio_worker_process_runs_cross_section_target() -> Result<(), String> {
    let (temp_dir, mut supervisor) = launch_worker(
        "portfolio-strategy",
        "m5_portfolio_strategy_fixture.wasm",
        "portfolio-strategy",
        StrategyWorld::PortfolioStrategy,
        vec!["close".into()],
        0,
        None,
    )?;
    running(&mut supervisor)?;
    let decision_time_ms = unix_now_ms();
    let result = supervisor
        .decision(
            "request-portfolio".into(),
            DecisionClock::ScheduledCrossSection {
                decision_id: "batch-1".into(),
                decision_time_ms,
                deadline_ms: decision_time_ms + 5_000,
                next_execution_ms: decision_time_ms + 5_001,
                universe: vec!["BTC-USDT".into(), "ETH-USDT".into()],
                available_instruments: vec!["BTC-USDT".into(), "ETH-USDT".into()],
            },
            WorkerDecisionInput::Portfolio {
                universe_id: "universe-1".into(),
                rows: vec![
                    WorkerFeatureRow {
                        instrument_id: "BTC-USDT".into(),
                        available_at_ms: decision_time_ms,
                        values: vec![Some(1.0)],
                    },
                    WorkerFeatureRow {
                        instrument_id: "ETH-USDT".into(),
                        available_at_ms: decision_time_ms,
                        values: vec![Some(2.0)],
                    },
                ],
                state: WorkerPortfolioState {
                    cash: "1000".into(),
                    positions: vec![],
                },
            },
        )
        .map_err(|error| format!("portfolio: {error}"))?;
    match result {
        WorkerDecisionResult::Target {
            decision_id,
            target:
                WorkerTarget::Portfolio {
                    universe_id,
                    weights,
                    cash_reserve,
                },
            ..
        } => {
            assert_eq!(decision_id, "batch-1");
            assert_eq!(universe_id, "universe-1");
            assert_eq!(weights.len(), 2);
            assert!(weights.iter().all(|weight| weight.weight == "0.5"));
            assert_eq!(cash_reserve, "0");
        }
        other => panic!("expected signed portfolio target, got {other:?}"),
    }
    supervisor.shutdown("request-stop")?;
    assert_eq!(supervisor.state(), LifecycleState::Stopped);
    let _ = fs::remove_dir_all(temp_dir);
    Ok(())
}
