use adaq_bot_runtime::{
    DecisionClock, DeploymentBundle, MAX_COMPONENT_BYTES, NoTargetReason, ProtocolSequence,
    StrategyWorld, WORKER_ARTIFACT_NAME, WORKER_ARTIFACT_VERSION, WORKER_PROTOCOL_VERSION,
    WORKER_RUNTIME_VERSION, WorkerComponentPayload, WorkerDecisionInput, WorkerEvaluationEvidence,
    WorkerEvaluationRow, WorkerEvaluationValue, WorkerFactorBinding, WorkerFactorScope,
    WorkerFeatureFrame, WorkerFeatureRow, WorkerHealthState, WorkerMessage, WorkerModelBinding,
    WorkerParameterValue, WorkerPipelineBinding, WorkerPortfolioState, WorkerRuntimePolicy,
    WorkerTarget, WorkerTargetWeight, current_platform_tag, decode_frame, encode_frame,
    enforce_worker_process_limits, is_decimal_text, read_bounded_line, sha256_hex, unix_now_ms,
};
use adaq_component_sdk::host::{
    factor_abi, factor_cross_sectional_abi, model_abi, portfolio_strategy_abi, strategy_abi,
};
use adaq_component_tooling::{ComponentParameterValue, RunLimits, WasmLoader};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use std::{
    collections::{HashMap, HashSet},
    io::{self, BufReader, BufWriter, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::Duration,
};

type Output = Arc<Mutex<BufWriter<io::Stdout>>>;

pub fn run() -> Result<(), String> {
    let bootstrap_policy = WorkerRuntimePolicy::default();
    bootstrap_policy
        .validate()
        .map_err(|error| error.to_string())?;
    adaq_bot_runtime::enforce_worker_process_limits(&bootstrap_policy)?;
    let output = Arc::new(Mutex::new(BufWriter::new(io::stdout())));
    let next_sequence = Arc::new(AtomicU64::new(1));
    let stop_heartbeat = Arc::new(AtomicBool::new(false));
    let identity = worker_identity()?;
    send_message(&output, &next_sequence, &bootstrap_policy, |sequence| {
        WorkerMessage::Hello {
            sequence,
            protocol_version: WORKER_PROTOCOL_VERSION.into(),
            artifact_name: WORKER_ARTIFACT_NAME.into(),
            artifact_version: WORKER_ARTIFACT_VERSION.into(),
            platform: identity.platform.clone(),
            runtime_version: WORKER_RUNTIME_VERSION.into(),
            artifact_sha256: identity.artifact_sha256.clone(),
        }
    })?;

    let stdin = io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    let mut inbound = ProtocolSequence::default();
    let mut state = WorkerState::new(identity, Arc::clone(&stop_heartbeat));
    loop {
        let frame = read_bounded_line(
            &mut reader,
            bootstrap_policy
                .max_frame_bytes_usize()
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        let Some(frame) = frame else {
            stop_heartbeat.store(true, Ordering::Release);
            return Ok(());
        };
        let message = match decode_frame(
            &frame,
            bootstrap_policy
                .max_frame_bytes_usize()
                .map_err(|error| error.to_string())?,
        ) {
            Ok(message) => message,
            Err(error) => {
                let _ = send_fault(
                    &output,
                    &next_sequence,
                    &bootstrap_policy,
                    None,
                    "malformed-frame",
                    &error.to_string(),
                );
                stop_heartbeat.store(true, Ordering::Release);
                return Err(error.to_string());
            }
        };
        if let Err(error) = inbound.accept(message.sequence()) {
            let _ = send_fault(
                &output,
                &next_sequence,
                &bootstrap_policy,
                None,
                "invalid-sequence",
                &error.to_string(),
            );
            stop_heartbeat.store(true, Ordering::Release);
            return Err(error.to_string());
        }
        match state.handle(message, &output, &next_sequence)? {
            WorkerAction::Continue => {}
            WorkerAction::Shutdown => return Ok(()),
        }
    }
}

#[derive(Clone)]
struct WorkerIdentity {
    platform: String,
    artifact_sha256: String,
}

fn worker_identity() -> Result<WorkerIdentity, String> {
    let executable =
        std::env::current_exe().map_err(|_| "worker-executable-unavailable".to_owned())?;
    let bytes = std::fs::read(executable).map_err(|_| "worker-executable-unreadable".to_owned())?;
    Ok(WorkerIdentity {
        platform: current_platform_tag(),
        artifact_sha256: sha256_hex(&bytes),
    })
}

enum WorkerAction {
    Continue,
    Shutdown,
}

enum LoadedEngine {
    Strategy(WasmLoader),
    PortfolioStrategy(WasmLoader),
}

struct LoadedFactor {
    binding: WorkerFactorBinding,
    loader: WasmLoader,
}

struct LoadedModel {
    binding: WorkerModelBinding,
    loader: WasmLoader,
}

#[derive(Default)]
struct LoadedPipeline {
    factors: Vec<LoadedFactor>,
    models: Vec<LoadedModel>,
}

struct WorkerState {
    identity: WorkerIdentity,
    bundle: Option<DeploymentBundle>,
    pipeline: Option<LoadedPipeline>,
    engine: Option<LoadedEngine>,
    initialized: bool,
    warmup_seen: u64,
    last_decision_time_ms: Option<i64>,
    seen_requests: HashSet<String>,
    stop_heartbeat: Arc<AtomicBool>,
}

impl WorkerState {
    fn new(identity: WorkerIdentity, stop_heartbeat: Arc<AtomicBool>) -> Self {
        Self {
            identity,
            bundle: None,
            pipeline: None,
            engine: None,
            initialized: false,
            warmup_seen: 0,
            last_decision_time_ms: None,
            seen_requests: HashSet::new(),
            stop_heartbeat,
        }
    }

    fn handle(
        &mut self,
        message: WorkerMessage,
        output: &Output,
        next_sequence: &Arc<AtomicU64>,
    ) -> Result<WorkerAction, String> {
        match message {
            WorkerMessage::Initialize {
                request_id,
                bundle,
                component_wasm,
                pipeline_components,
                ..
            } => {
                self.initialize(
                    request_id,
                    bundle,
                    component_wasm,
                    pipeline_components,
                    output,
                    next_sequence,
                )?;
                Ok(WorkerAction::Continue)
            }
            WorkerMessage::Decision {
                request_id,
                clock,
                input,
                ..
            } => {
                self.decision(request_id, clock, input, output, next_sequence)?;
                Ok(WorkerAction::Continue)
            }
            WorkerMessage::Shutdown { request_id, .. } => {
                let policy = self.policy();
                send_message(output, next_sequence, &policy, |sequence| {
                    WorkerMessage::ShutdownAck {
                        sequence,
                        request_id,
                    }
                })?;
                self.stop_heartbeat.store(true, Ordering::Release);
                Ok(WorkerAction::Shutdown)
            }
            _ => self.fault(
                output,
                next_sequence,
                None,
                "unexpected-message",
                "message is not valid in the current worker state",
            ),
        }
    }

    fn initialize(
        &mut self,
        request_id: String,
        bundle: DeploymentBundle,
        component_wasm: String,
        pipeline_components: Vec<WorkerComponentPayload>,
        output: &Output,
        next_sequence: &Arc<AtomicU64>,
    ) -> Result<(), String> {
        if self.initialized || request_id.trim().is_empty() {
            return self.fault(
                output,
                next_sequence,
                Some(request_id),
                "invalid-initialize",
                "worker initialization is not valid",
            );
        }
        bundle.verify().map_err(|error| error.to_string())?;
        let worker = &bundle.input.worker;
        if worker.artifact_name != WORKER_ARTIFACT_NAME
            || worker.artifact_version != WORKER_ARTIFACT_VERSION
            || worker.platform != self.identity.platform
            || worker.protocol_version != WORKER_PROTOCOL_VERSION
            || worker.runtime_version != WORKER_RUNTIME_VERSION
            || worker.sha256 != self.identity.artifact_sha256
        {
            return self.fault(
                output,
                next_sequence,
                Some(request_id),
                "worker-identity-mismatch",
                "initialized worker identity does not match the frozen bundle",
            );
        }
        let component_wasm = BASE64
            .decode(component_wasm)
            .map_err(|_| "component-payload-malformed".to_owned())?;
        if component_wasm.len() > MAX_COMPONENT_BYTES
            || sha256_hex(&component_wasm) != bundle.input.strategy.component_sha256
        {
            return self.fault(
                output,
                next_sequence,
                Some(request_id),
                "component-identity-mismatch",
                "strategy component hash does not match the frozen bundle",
            );
        }
        let policy = bundle.input.worker_policy.clone();
        enforce_worker_process_limits(&policy)?;
        let limits = RunLimits {
            fuel_per_call: policy.fuel_per_call,
            memory_bytes: usize::try_from(policy.memory_bytes)
                .map_err(|_| "worker-memory-limit-invalid".to_owned())?,
            max_bars: usize::try_from(policy.max_decision_frames)
                .map_err(|_| "worker-frame-limit-invalid".to_owned())?,
        };
        let parameters = bundle
            .input
            .strategy
            .parameters
            .iter()
            .map(component_parameter)
            .collect::<Vec<_>>();
        let pipeline_payloads =
            match decode_pipeline_components(&bundle.input.pipeline, &pipeline_components) {
                Ok(payloads) => payloads,
                Err(error) => {
                    return self.fault(
                        output,
                        next_sequence,
                        Some(request_id),
                        "pipeline-payload-invalid",
                        &error,
                    );
                }
            };
        let pipeline = match load_pipeline(&bundle.input.pipeline, &pipeline_payloads, limits) {
            Ok(pipeline) => pipeline,
            Err(error) => {
                return self.fault(
                    output,
                    next_sequence,
                    Some(request_id),
                    "pipeline-load-failed",
                    &error,
                );
            }
        };
        let slots = bundle.input.strategy.feature_slots.clone();
        let engine = match bundle.input.strategy.world {
            StrategyWorld::Strategy => {
                let loader = WasmLoader::with_limits(limits);
                let slots = slots
                    .into_iter()
                    .map(|name| strategy_abi::exports::adaq::strategy::api::FeatureSlot { name })
                    .collect();
                loader
                    .load_strategy_bytes(&component_wasm, slots, &parameters)
                    .map_err(|error| format!("component-load-failed: {error}"))?;
                LoadedEngine::Strategy(loader)
            }
            StrategyWorld::PortfolioStrategy => {
                let loader = WasmLoader::with_limits(limits);
                let slots = slots
                    .into_iter()
                    .map(|name| {
                        portfolio_strategy_abi::exports::adaq::strategy::portfolio_api::FeatureSlot {
                            name,
                        }
                    })
                    .collect();
                loader
                    .load_portfolio_strategy_bytes(&component_wasm, slots, &parameters)
                    .map_err(|error| format!("component-load-failed: {error}"))?;
                LoadedEngine::PortfolioStrategy(loader)
            }
        };
        let bundle_identity = bundle.identity.clone();
        let world = bundle.input.strategy.world.clone();
        self.bundle = Some(bundle);
        self.pipeline = Some(pipeline);
        self.engine = Some(engine);
        self.initialized = true;
        send_message(output, next_sequence, &policy, |sequence| {
            WorkerMessage::Initialized {
                sequence,
                request_id,
                bundle_identity,
                world,
            }
        })?;
        spawn_heartbeat(
            Arc::clone(output),
            Arc::clone(next_sequence),
            policy,
            Arc::clone(&self.stop_heartbeat),
        );
        Ok(())
    }

    fn decision(
        &mut self,
        request_id: String,
        clock: DecisionClock,
        input: WorkerDecisionInput,
        output: &Output,
        next_sequence: &Arc<AtomicU64>,
    ) -> Result<(), String> {
        if !self.initialized || request_id.trim().is_empty() {
            return self.fault(
                output,
                next_sequence,
                Some(request_id),
                "worker-not-initialized",
                "decision received before initialization",
            );
        }
        if !self.seen_requests.insert(request_id.clone()) {
            return self.fault(
                output,
                next_sequence,
                Some(request_id),
                "duplicate-request",
                "request identity was already consumed",
            );
        }
        let bundle = self
            .bundle
            .as_ref()
            .ok_or_else(|| "worker-bundle-missing".to_owned())?;
        let policy = bundle.input.worker_policy.clone();
        clock.validate().map_err(|error| error.to_string())?;
        if self
            .last_decision_time_ms
            .is_some_and(|last| clock.decision_time_ms() <= last)
        {
            return self.no_target(
                request_id,
                clock,
                NoTargetReason::StaleDecision,
                "decision clock is not newer than the previous accepted decision",
                output,
                next_sequence,
            );
        }
        if unix_now_ms() > clock.deadline_ms() {
            return self.no_target(
                request_id,
                clock,
                NoTargetReason::DeadlineMissed,
                "decision deadline has passed",
                output,
                next_sequence,
            );
        }
        let prepared = match prepare_input(
            &clock,
            &input,
            &bundle.input.strategy,
            &bundle.input.pipeline,
            &policy,
        ) {
            Ok(prepared) => prepared,
            Err(InputError::NoTarget(reason, detail)) => {
                return self.no_target(request_id, clock, reason, &detail, output, next_sequence);
            }
            Err(InputError::Fault(code, detail)) => {
                return self.fault(output, next_sequence, Some(request_id), code, detail);
            }
            Err(InputError::Pipeline(detail)) => {
                return self.fault(
                    output,
                    next_sequence,
                    Some(request_id),
                    "pipeline-evaluation-failed",
                    &detail,
                );
            }
        };
        let pipeline = self
            .pipeline
            .as_ref()
            .ok_or_else(|| "worker-pipeline-missing".to_owned())?;
        let evaluated = match evaluate_pipeline(
            &clock,
            prepared,
            pipeline,
            &bundle.input.strategy,
            &bundle.input.pipeline,
        ) {
            Ok(prepared) => prepared,
            Err(InputError::NoTarget(reason, detail)) => {
                return self.no_target(request_id, clock, reason, &detail, output, next_sequence);
            }
            Err(InputError::Fault(code, detail)) => {
                return self.fault(output, next_sequence, Some(request_id), code, detail);
            }
            Err(InputError::Pipeline(detail)) => {
                return self.fault(
                    output,
                    next_sequence,
                    Some(request_id),
                    "pipeline-evaluation-failed",
                    &detail,
                );
            }
        };
        let EvaluatedEngineInput {
            input: prepared,
            evidence,
        } = evaluated;
        let target = match (self.engine.as_ref(), prepared) {
            (Some(LoadedEngine::Strategy(loader)), PreparedEngineInput::Strategy(frames)) => {
                let targets = loader
                    .process_strategy(frames)
                    .map_err(|error| format!("strategy-process-failed: {error}"))?;
                if targets.len() != input_frame_count(&input) {
                    return self.fault(
                        output,
                        next_sequence,
                        Some(request_id),
                        "invalid-target-count",
                        "strategy returned a target count different from its input",
                    );
                }
                let instrument_id = match &input {
                    WorkerDecisionInput::Strategy { instrument_id, .. } => instrument_id.clone(),
                    _ => unreachable!(),
                };
                WorkerTarget::Strategy {
                    instrument_id: instrument_id.clone(),
                    exposures: targets
                        .into_iter()
                        .map(|exposure| adaq_bot_runtime::WorkerExposure {
                            instrument_id: instrument_id.clone(),
                            exposure,
                        })
                        .collect(),
                }
            }
            (
                Some(LoadedEngine::PortfolioStrategy(loader)),
                PreparedEngineInput::Portfolio(frame),
            ) => {
                let mut targets = loader
                    .process_portfolio_strategy(vec![frame])
                    .map_err(|error| format!("portfolio-strategy-process-failed: {error}"))?;
                let target = targets
                    .pop()
                    .ok_or_else(|| "portfolio-strategy-empty-output".to_owned())?;
                WorkerTarget::Portfolio {
                    universe_id: target.universe_id,
                    weights: target
                        .weights
                        .into_iter()
                        .map(|weight| WorkerTargetWeight {
                            instrument_id: weight.instrument_id,
                            weight: weight.weight,
                        })
                        .collect(),
                    cash_reserve: target.cash_reserve,
                }
            }
            _ => {
                return self.fault(
                    output,
                    next_sequence,
                    Some(request_id),
                    "worker-world-mismatch",
                    "decision input does not match the loaded strategy world",
                );
            }
        };
        target
            .validate_for(&bundle.input.strategy.world, &input)
            .map_err(|error| error.to_string())?;
        self.last_decision_time_ms = Some(clock.decision_time_ms());
        if self.warmup_seen < policy.warmup_decisions {
            self.warmup_seen = self.warmup_seen.saturating_add(1);
            return self.no_target(
                request_id,
                clock,
                NoTargetReason::Warmup,
                "worker warmup policy has not completed",
                output,
                next_sequence,
            );
        }
        let produced_at_ms = unix_now_ms();
        if produced_at_ms > clock.deadline_ms() {
            return self.no_target(
                request_id,
                clock,
                NoTargetReason::DeadlineMissed,
                "strategy evaluation completed after the decision deadline",
                output,
                next_sequence,
            );
        }
        send_message(output, next_sequence, &policy, |sequence| {
            WorkerMessage::Target {
                sequence,
                request_id,
                decision_id: clock.decision_id().to_owned(),
                produced_at_ms,
                target,
                evaluation: evidence,
            }
        })
    }

    fn no_target(
        &self,
        request_id: String,
        clock: DecisionClock,
        reason: NoTargetReason,
        detail: &str,
        output: &Output,
        next_sequence: &Arc<AtomicU64>,
    ) -> Result<(), String> {
        let policy = self.policy();
        send_message(output, next_sequence, &policy, |sequence| {
            WorkerMessage::NoTarget {
                sequence,
                request_id,
                decision_id: clock.decision_id().to_owned(),
                reason,
                detail: detail
                    .chars()
                    .take(policy.max_diagnostic_bytes as usize)
                    .collect(),
            }
        })
    }

    fn policy(&self) -> WorkerRuntimePolicy {
        self.bundle
            .as_ref()
            .map(|bundle| bundle.input.worker_policy.clone())
            .unwrap_or_default()
    }

    fn fault<T>(
        &self,
        output: &Output,
        next_sequence: &Arc<AtomicU64>,
        request_id: Option<String>,
        code: &str,
        detail: &str,
    ) -> Result<T, String> {
        let policy = self.policy();
        let _ = send_fault(output, next_sequence, &policy, request_id, code, detail);
        Err(code.to_owned())
    }
}

fn decode_pipeline_components(
    binding: &WorkerPipelineBinding,
    payloads: &[WorkerComponentPayload],
) -> Result<HashMap<String, Vec<u8>>, String> {
    let expected = binding
        .factors
        .iter()
        .map(|factor| factor.component_sha256.as_str())
        .chain(
            binding
                .models
                .iter()
                .map(|model| model.component_sha256.as_str()),
        )
        .collect::<HashSet<_>>();
    if expected.len() != payloads.len() {
        return Err("pipeline-component-count-mismatch".into());
    }
    let mut decoded = HashMap::with_capacity(payloads.len());
    for payload in payloads {
        if !expected.contains(payload.component_sha256.as_str())
            || decoded.contains_key(&payload.component_sha256)
        {
            return Err("pipeline-component-identity-mismatch".into());
        }
        let wasm = BASE64
            .decode(&payload.wasm)
            .map_err(|_| "pipeline-component-payload-malformed".to_owned())?;
        if wasm.len() > MAX_COMPONENT_BYTES || sha256_hex(&wasm) != payload.component_sha256 {
            return Err("pipeline-component-identity-mismatch".into());
        }
        decoded.insert(payload.component_sha256.clone(), wasm);
    }
    Ok(decoded)
}

fn load_pipeline(
    binding: &WorkerPipelineBinding,
    payloads: &HashMap<String, Vec<u8>>,
    limits: RunLimits,
) -> Result<LoadedPipeline, String> {
    let mut pipeline = LoadedPipeline::default();
    for factor in &binding.factors {
        let wasm = payloads
            .get(&factor.component_sha256)
            .ok_or_else(|| "pipeline-factor-payload-missing".to_owned())?;
        let loader = WasmLoader::with_limits(limits);
        let schema = match factor.scope {
            WorkerFactorScope::TimeSeries => {
                let slots = factor
                    .feature_slots
                    .iter()
                    .map(
                        |name| factor_abi::exports::adaq::factor::time_series_api::FeatureSlot {
                            name: name.clone(),
                        },
                    )
                    .collect();
                loader.load_factor_time_series_bytes(
                    wasm,
                    slots,
                    &factor
                        .parameters
                        .iter()
                        .map(component_parameter)
                        .collect::<Vec<_>>(),
                )?;
                loader.describe_factor()?
            }
            WorkerFactorScope::CrossSectional => {
                let slots = factor
                    .feature_slots
                    .iter()
                    .map(|name| {
                        factor_cross_sectional_abi::exports::adaq::factor::cross_sectional_api::FeatureSlot {
                            name: name.clone(),
                        }
                    })
                    .collect();
                loader.load_factor_cross_sectional_bytes(
                    wasm,
                    slots,
                    &factor
                        .parameters
                        .iter()
                        .map(component_parameter)
                        .collect::<Vec<_>>(),
                )?;
                loader.describe_factor()?
            }
        };
        if schema.feature_slots != factor.feature_slots
            || schema.output_names != factor.output_names
            || u64::from(schema.warmup_bars) != factor.warmup_bars
        {
            return Err("pipeline-factor-schema-mismatch".into());
        }
        pipeline.factors.push(LoadedFactor {
            binding: factor.clone(),
            loader,
        });
    }
    for model in &binding.models {
        let wasm = payloads
            .get(&model.component_sha256)
            .ok_or_else(|| "pipeline-model-payload-missing".to_owned())?;
        let loader = WasmLoader::with_limits(limits);
        let slots = model
            .feature_slots
            .iter()
            .map(|name| model_abi::exports::adaq::model::api::FeatureSlot { name: name.clone() })
            .collect();
        loader.load_model_bytes(
            wasm,
            slots,
            &model
                .parameters
                .iter()
                .map(component_parameter)
                .collect::<Vec<_>>(),
            model.seed,
        )?;
        pipeline.models.push(LoadedModel {
            binding: model.clone(),
            loader,
        });
    }
    Ok(pipeline)
}

#[derive(Debug)]
enum PreparedInput {
    Strategy {
        instrument_id: String,
        frames: Vec<WorkerFeatureFrame>,
    },
    Portfolio {
        universe_id: String,
        rows: Vec<WorkerFeatureRow>,
        state: WorkerPortfolioState,
    },
}

#[derive(Debug)]
enum PreparedEngineInput {
    Strategy(Vec<strategy_abi::exports::adaq::strategy::api::FeatureFrame>),
    Portfolio(portfolio_strategy_abi::exports::adaq::strategy::portfolio_api::PortfolioFrame),
}

struct EvaluatedEngineInput {
    input: PreparedEngineInput,
    evidence: WorkerEvaluationEvidence,
}

#[derive(Debug)]
enum InputError {
    NoTarget(NoTargetReason, String),
    Fault(&'static str, &'static str),
    Pipeline(String),
}

fn prepare_input(
    clock: &DecisionClock,
    input: &WorkerDecisionInput,
    binding: &adaq_bot_runtime::WorkerStrategyBinding,
    pipeline: &WorkerPipelineBinding,
    policy: &WorkerRuntimePolicy,
) -> Result<PreparedInput, InputError> {
    let input_slots = pipeline_input_slots(binding, pipeline);
    match (binding.world.clone(), clock, input) {
        (
            StrategyWorld::Strategy,
            DecisionClock::ClosedBar {
                instrument_id: expected,
                ..
            },
            WorkerDecisionInput::Strategy {
                instrument_id,
                frames,
            },
        ) => {
            if instrument_id != expected {
                return Err(InputError::Fault(
                    "instrument-identity-mismatch",
                    "strategy decision instrument does not match the clock",
                ));
            }
            if frames.is_empty() {
                return Err(InputError::NoTarget(
                    NoTargetReason::MissingInput,
                    "strategy decision has no feature frames".into(),
                ));
            }
            if frames.len() > policy.max_decision_frames as usize {
                return Err(InputError::Fault(
                    "decision-frame-limit",
                    "strategy decision exceeds the frozen frame limit",
                ));
            }
            let mut previous_open = None;
            let mut prepared = Vec::with_capacity(frames.len());
            for frame in frames {
                if frame.instrument_id != *expected
                    || previous_open.is_some_and(|previous| frame.open_time_ms <= previous)
                    || frame.open_time_ms > clock.decision_time_ms()
                {
                    return Err(InputError::Fault(
                        "strategy-frame-order-invalid",
                        "strategy frames are not strictly chronological or identity-preserving",
                    ));
                }
                previous_open = Some(frame.open_time_ms);
                if frame.available_at_ms > clock.decision_time_ms() {
                    return Err(InputError::NoTarget(
                        NoTargetReason::MissingInput,
                        "strategy feature frame is not available at the decision time".into(),
                    ));
                }
                validate_raw_values(&frame.values, input_slots.len())?;
                if pipeline.factors.is_empty()
                    && pipeline.models.is_empty()
                    && frame.values.iter().any(Option::is_none)
                {
                    return Err(InputError::NoTarget(
                        NoTargetReason::MissingInput,
                        "one or more feature values are missing".into(),
                    ));
                }
                prepared.push(frame.clone());
            }
            Ok(PreparedInput::Strategy {
                instrument_id: instrument_id.clone(),
                frames: prepared,
            })
        }
        (
            StrategyWorld::PortfolioStrategy,
            DecisionClock::ScheduledCrossSection { universe, .. },
            WorkerDecisionInput::Portfolio {
                universe_id,
                rows,
                state,
            },
        ) => {
            if universe_id.trim().is_empty() || rows.is_empty() {
                return Err(InputError::NoTarget(
                    NoTargetReason::IncompleteUniverse,
                    "portfolio decision has no complete universe".into(),
                ));
            }
            if rows.len() != universe.len()
                || rows
                    .iter()
                    .zip(universe)
                    .any(|(row, id)| row.instrument_id != *id)
            {
                return Err(InputError::NoTarget(
                    NoTargetReason::IncompleteUniverse,
                    "portfolio rows do not match the frozen universe".into(),
                ));
            }
            let mut prepared_rows = Vec::with_capacity(rows.len());
            for row in rows {
                if row.available_at_ms > clock.decision_time_ms() {
                    return Err(InputError::NoTarget(
                        NoTargetReason::MissingInput,
                        "portfolio feature row is not available at the decision time".into(),
                    ));
                }
                validate_raw_values(&row.values, input_slots.len())?;
                if pipeline.factors.is_empty()
                    && pipeline.models.is_empty()
                    && row.values.iter().any(Option::is_none)
                {
                    return Err(InputError::NoTarget(
                        NoTargetReason::MissingInput,
                        "one or more feature values are missing".into(),
                    ));
                }
                prepared_rows.push(row.clone());
            }
            portfolio_state(state)?;
            Ok(PreparedInput::Portfolio {
                universe_id: universe_id.clone(),
                rows: prepared_rows,
                state: state.clone(),
            })
        }
        (StrategyWorld::Strategy, _, _) => Err(InputError::Fault(
            "clock-world-mismatch",
            "Strategy workers require a ClosedBar clock",
        )),
        (StrategyWorld::PortfolioStrategy, _, _) => Err(InputError::Fault(
            "clock-world-mismatch",
            "Portfolio Strategy workers require a ScheduledCrossSection clock",
        )),
    }
}

fn pipeline_input_slots<'a>(
    binding: &'a adaq_bot_runtime::WorkerStrategyBinding,
    pipeline: &'a WorkerPipelineBinding,
) -> &'a [String] {
    if pipeline.factors.is_empty() && pipeline.models.is_empty() {
        &binding.feature_slots
    } else {
        &pipeline.input_slots
    }
}

fn validate_raw_values(values: &[Option<f64>], slot_count: usize) -> Result<(), InputError> {
    if values.len() != slot_count {
        return Err(InputError::Fault(
            "feature-slot-count-mismatch",
            "feature values do not match the frozen pipeline input binding",
        ));
    }
    if values.iter().flatten().any(|value| !value.is_finite()) {
        return Err(InputError::Fault(
            "non-finite-feature",
            "feature values must be finite",
        ));
    }
    Ok(())
}

fn value_map(
    slots: &[String],
    values: &[Option<f64>],
) -> Result<HashMap<String, Option<f64>>, InputError> {
    validate_raw_values(values, slots.len())?;
    Ok(slots.iter().cloned().zip(values.iter().copied()).collect())
}

fn present_values(
    values: &HashMap<String, Option<f64>>,
    slots: &[String],
) -> Result<Vec<f64>, InputError> {
    slots
        .iter()
        .map(|slot| {
            values.get(slot).copied().flatten().ok_or_else(|| {
                InputError::NoTarget(
                    NoTargetReason::MissingInput,
                    format!("required pipeline input is unavailable: {slot}"),
                )
            })
        })
        .collect()
}

fn missing_stage_output(binding: &WorkerFactorBinding) -> NoTargetReason {
    if binding.warmup_bars > 0 {
        NoTargetReason::Warmup
    } else {
        NoTargetReason::MissingInput
    }
}

fn evaluate_pipeline(
    clock: &DecisionClock,
    prepared: PreparedInput,
    loaded: &LoadedPipeline,
    strategy: &adaq_bot_runtime::WorkerStrategyBinding,
    pipeline: &WorkerPipelineBinding,
) -> Result<EvaluatedEngineInput, InputError> {
    match prepared {
        PreparedInput::Strategy {
            instrument_id,
            frames,
        } => {
            let input_slots = pipeline_input_slots(strategy, pipeline);
            let mut values = frames
                .iter()
                .map(|frame| value_map(input_slots, &frame.values))
                .collect::<Result<Vec<_>, _>>()?;
            let mut evidence_rows = frames
                .iter()
                .map(|frame| WorkerEvaluationRow {
                    instrument_id: frame.instrument_id.clone(),
                    observation_time_ms: frame.open_time_ms,
                    available_at_ms: frame.available_at_ms,
                    factor_outputs: Vec::new(),
                    model_outputs: Vec::new(),
                })
                .collect::<Vec<_>>();
            for factor in &loaded.factors {
                if factor.binding.scope != WorkerFactorScope::TimeSeries {
                    return Err(InputError::Fault(
                        "pipeline-scope-mismatch",
                        "time-series decisions require time-series factors",
                    ));
                }
                let rows = frames
                    .iter()
                    .zip(&values)
                    .map(|(frame, values)| {
                        Ok(
                            factor_abi::exports::adaq::factor::time_series_api::TimeSeriesRow {
                                instrument_id: instrument_id.clone(),
                                observation_time_ms: frame.open_time_ms,
                                slots: factor
                                    .binding
                                    .feature_slots
                                    .iter()
                                    .map(|slot| {
                                        present_values(values, std::slice::from_ref(slot)).map(
                                            |value| {
                                                factor_abi::exports::adaq::factor::time_series_api::FeatureValue {
                                                    value: value[0],
                                                    available_at_ms: frame.available_at_ms,
                                                }
                                            },
                                        )
                                    })
                                    .collect::<Result<Vec<_>, _>>()
                                    .map_err(|error| match error {
                                        InputError::NoTarget(_, _) => error,
                                        _ => InputError::Pipeline(
                                            "factor input preparation failed".into(),
                                        ),
                                    })?,
                            },
                        )
                    })
                    .collect::<Result<Vec<_>, InputError>>()?;
                let results = factor
                    .loader
                    .process_factor(rows)
                    .map_err(InputError::Pipeline)?;
                if results.len() != frames.len() {
                    return Err(InputError::Pipeline(
                        "factor output count differs from the input row count".into(),
                    ));
                }
                for (((frame_values, frame), evidence_row), result) in values
                    .iter_mut()
                    .zip(&frames)
                    .zip(evidence_rows.iter_mut())
                    .zip(results)
                {
                    if result.instrument_id != instrument_id
                        || result.observation_time_ms != frame.open_time_ms
                    {
                        return Err(InputError::Pipeline(
                            "factor output violates the frozen identity contract".into(),
                        ));
                    }
                    let Some(outputs) = result.values else {
                        return Err(InputError::NoTarget(
                            missing_stage_output(&factor.binding),
                            "factor output is unavailable".into(),
                        ));
                    };
                    if outputs.len() != factor.binding.output_names.len()
                        || outputs
                            .iter()
                            .zip(&factor.binding.output_names)
                            .any(|(output, name)| output.name != *name || !output.value.is_finite())
                    {
                        return Err(InputError::Pipeline(
                            "factor output violates the frozen schema contract".into(),
                        ));
                    }
                    for output in outputs {
                        evidence_row.factor_outputs.push(WorkerEvaluationValue {
                            name: output.name.clone(),
                            value: output.value.to_string(),
                        });
                        frame_values.insert(output.name, Some(output.value));
                    }
                }
            }
            for model in &loaded.models {
                let rows = frames
                    .iter()
                    .zip(&values)
                    .map(|(frame, values)| {
                        Ok(model_abi::exports::adaq::model::api::PredictionRow {
                            instrument_id: instrument_id.clone(),
                            prediction_time_ms: frame.open_time_ms,
                            values: present_values(values, &model.binding.feature_slots)?,
                        })
                    })
                    .collect::<Result<Vec<_>, InputError>>()?;
                let results = model
                    .loader
                    .process_model(rows)
                    .map_err(InputError::Pipeline)?;
                if results.len() != frames.len() {
                    return Err(InputError::Pipeline(
                        "model output count differs from the input row count".into(),
                    ));
                }
                for (((frame_values, frame), evidence_row), result) in values
                    .iter_mut()
                    .zip(&frames)
                    .zip(evidence_rows.iter_mut())
                    .zip(results)
                {
                    let Some(result) = result else {
                        return Err(InputError::NoTarget(
                            NoTargetReason::MissingInput,
                            "model output is unavailable".into(),
                        ));
                    };
                    if result.instrument_id != instrument_id
                        || result.prediction_time_ms != frame.open_time_ms
                        || result.values.len() != model.binding.output_names.len()
                        || result.values.iter().any(|value| !value.is_finite())
                    {
                        return Err(InputError::Pipeline(
                            "model output violates the frozen identity contract".into(),
                        ));
                    }
                    for (name, value) in model.binding.output_names.iter().zip(result.values) {
                        evidence_row.model_outputs.push(WorkerEvaluationValue {
                            name: name.clone(),
                            value: value.to_string(),
                        });
                        frame_values.insert(name.clone(), Some(value));
                    }
                }
            }
            let frames = frames
                .iter()
                .zip(values)
                .map(|(frame, values)| {
                    Ok(strategy_abi::exports::adaq::strategy::api::FeatureFrame {
                        open_time_ms: frame.open_time_ms,
                        values: present_values(&values, &strategy.feature_slots)?,
                    })
                })
                .collect::<Result<Vec<_>, InputError>>()?;
            Ok(EvaluatedEngineInput {
                input: PreparedEngineInput::Strategy(frames),
                evidence: WorkerEvaluationEvidence {
                    rows: evidence_rows,
                },
            })
        }
        PreparedInput::Portfolio {
            universe_id,
            rows,
            state,
        } => {
            let input_slots = pipeline_input_slots(strategy, pipeline);
            let mut values = rows
                .iter()
                .map(|row| value_map(input_slots, &row.values))
                .collect::<Result<Vec<_>, _>>()?;
            let mut evidence_rows = rows
                .iter()
                .map(|row| WorkerEvaluationRow {
                    instrument_id: row.instrument_id.clone(),
                    observation_time_ms: clock.decision_time_ms(),
                    available_at_ms: row.available_at_ms,
                    factor_outputs: Vec::new(),
                    model_outputs: Vec::new(),
                })
                .collect::<Vec<_>>();
            let universe = rows
                .iter()
                .map(|row| row.instrument_id.clone())
                .collect::<Vec<_>>();
            for factor in &loaded.factors {
                if factor.binding.scope != WorkerFactorScope::CrossSectional {
                    return Err(InputError::Fault(
                        "pipeline-scope-mismatch",
                        "cross-sectional decisions require cross-sectional factors",
                    ));
                }
                let input_was_missing = values.iter().any(|frame_values| {
                    factor
                        .binding
                        .feature_slots
                        .iter()
                        .any(|slot| !matches!(frame_values.get(slot), Some(Some(_))))
                });
                let factor_rows = rows
                    .iter()
                    .zip(&values)
                    .map(|(row, frame_values)| {
                        factor_cross_sectional_abi::exports::adaq::factor::cross_sectional_api::CrossSectionalRow {
                            instrument_id: row.instrument_id.clone(),
                            observation_time_ms: clock.decision_time_ms(),
                            slots: factor
                                .binding
                                .feature_slots
                                .iter()
                                .map(|slot| match frame_values.get(slot).copied().flatten() {
                                    Some(value) => factor_cross_sectional_abi::exports::adaq::factor::cross_sectional_api::FeatureCell::Available(
                                        factor_cross_sectional_abi::exports::adaq::factor::cross_sectional_api::FeatureValue {
                                            value,
                                            available_at_ms: row.available_at_ms,
                                        },
                                    ),
                                    None => factor_cross_sectional_abi::exports::adaq::factor::cross_sectional_api::FeatureCell::Unavailable(
                                        factor_cross_sectional_abi::exports::adaq::factor::cross_sectional_api::UnavailabilityReason::MissingInput,
                                    ),
                                })
                                .collect(),
                        }
                    })
                    .collect::<Vec<_>>();
                let results = factor
                    .loader
                    .process_cross_sectional_factor(factor_rows, &universe)
                    .map_err(InputError::Pipeline)?;
                if results.len() != rows.len() {
                    return Err(InputError::Pipeline(
                        "factor output count differs from the input row count".into(),
                    ));
                }
                for (((frame_values, row), evidence_row), result) in values
                    .iter_mut()
                    .zip(&rows)
                    .zip(evidence_rows.iter_mut())
                    .zip(results)
                {
                    if result.instrument_id != row.instrument_id
                        || result.observation_time_ms != clock.decision_time_ms()
                    {
                        return Err(InputError::Pipeline(
                            "factor output violates the frozen identity contract".into(),
                        ));
                    }
                    let Some(outputs) = result.values else {
                        return Err(InputError::NoTarget(
                            if input_was_missing {
                                NoTargetReason::MissingInput
                            } else {
                                missing_stage_output(&factor.binding)
                            },
                            "factor output is unavailable".into(),
                        ));
                    };
                    if outputs.len() != factor.binding.output_names.len()
                        || outputs
                            .iter()
                            .zip(&factor.binding.output_names)
                            .any(|(output, name)| output.name != *name || !output.value.is_finite())
                    {
                        return Err(InputError::Pipeline(
                            "factor output violates the frozen schema contract".into(),
                        ));
                    }
                    for output in outputs {
                        evidence_row.factor_outputs.push(WorkerEvaluationValue {
                            name: output.name.clone(),
                            value: output.value.to_string(),
                        });
                        frame_values.insert(output.name, Some(output.value));
                    }
                }
            }
            for model in &loaded.models {
                let model_rows = rows
                    .iter()
                    .zip(&values)
                    .map(|(row, frame_values)| {
                        Ok(model_abi::exports::adaq::model::api::PredictionRow {
                            instrument_id: row.instrument_id.clone(),
                            prediction_time_ms: clock.decision_time_ms(),
                            values: present_values(frame_values, &model.binding.feature_slots)?,
                        })
                    })
                    .collect::<Result<Vec<_>, InputError>>()?;
                let results = model
                    .loader
                    .process_model(model_rows)
                    .map_err(InputError::Pipeline)?;
                if results.len() != rows.len() {
                    return Err(InputError::Pipeline(
                        "model output count differs from the input row count".into(),
                    ));
                }
                for (((frame_values, row), evidence_row), result) in values
                    .iter_mut()
                    .zip(&rows)
                    .zip(evidence_rows.iter_mut())
                    .zip(results)
                {
                    let Some(result) = result else {
                        return Err(InputError::NoTarget(
                            NoTargetReason::MissingInput,
                            "model output is unavailable".into(),
                        ));
                    };
                    if result.instrument_id != row.instrument_id
                        || result.prediction_time_ms != clock.decision_time_ms()
                        || result.values.len() != model.binding.output_names.len()
                        || result.values.iter().any(|value| !value.is_finite())
                    {
                        return Err(InputError::Pipeline(
                            "model output violates the frozen identity contract".into(),
                        ));
                    }
                    for (name, value) in model.binding.output_names.iter().zip(result.values) {
                        evidence_row.model_outputs.push(WorkerEvaluationValue {
                            name: name.clone(),
                            value: value.to_string(),
                        });
                        frame_values.insert(name.clone(), Some(value));
                    }
                }
            }
            let rows = rows
                .iter()
                .zip(values)
                .map(|(row, values)| {
                    Ok(
                        portfolio_strategy_abi::exports::adaq::strategy::portfolio_api::FeatureRow {
                            instrument_id: row.instrument_id.clone(),
                            values: present_values(&values, &strategy.feature_slots)?,
                        },
                    )
                })
                .collect::<Result<Vec<_>, InputError>>()?;
            Ok(EvaluatedEngineInput {
                input: PreparedEngineInput::Portfolio(
                    portfolio_strategy_abi::exports::adaq::strategy::portfolio_api::PortfolioFrame {
                        decision_time_ms: clock.decision_time_ms(),
                        universe_id,
                        rows,
                        state: portfolio_state(&state)?,
                    },
                ),
                evidence: WorkerEvaluationEvidence {
                    rows: evidence_rows,
                },
            })
        }
    }
}

fn portfolio_state(
    state: &WorkerPortfolioState,
) -> Result<
    portfolio_strategy_abi::exports::adaq::strategy::portfolio_api::PortfolioState,
    InputError,
> {
    if !valid_decimal(&state.cash) {
        return Err(InputError::Fault(
            "invalid-portfolio-cash",
            "portfolio cash must be exact decimal text",
        ));
    }
    let mut instruments = HashSet::new();
    let mut positions = Vec::with_capacity(state.positions.len());
    for position in &state.positions {
        if position.instrument_id.trim().is_empty()
            || !instruments.insert(position.instrument_id.as_str())
            || !valid_decimal(&position.quantity)
            || !valid_decimal(&position.price)
        {
            return Err(InputError::Fault(
                "invalid-portfolio-position",
                "portfolio positions must be unique and exact",
            ));
        }
        positions.push(
            portfolio_strategy_abi::exports::adaq::strategy::portfolio_api::Position {
                instrument_id: position.instrument_id.clone(),
                quantity: position.quantity.clone(),
                price: position.price.clone(),
            },
        );
    }
    Ok(
        portfolio_strategy_abi::exports::adaq::strategy::portfolio_api::PortfolioState {
            cash: state.cash.clone(),
            positions,
        },
    )
}

fn valid_decimal(value: &str) -> bool {
    is_decimal_text(value) && adaq_component_sdk::parse_decimal(value).is_ok()
}

fn component_parameter(value: &WorkerParameterValue) -> ComponentParameterValue {
    match value {
        WorkerParameterValue::Decimal(value) => ComponentParameterValue::Decimal(value.clone()),
        WorkerParameterValue::Integer(value) => ComponentParameterValue::Integer(*value),
        WorkerParameterValue::Boolean(value) => ComponentParameterValue::Boolean(*value),
        WorkerParameterValue::String(value) => ComponentParameterValue::String(value.clone()),
    }
}

fn input_frame_count(input: &WorkerDecisionInput) -> usize {
    match input {
        WorkerDecisionInput::Strategy { frames, .. } => frames.len(),
        WorkerDecisionInput::Portfolio { .. } => 1,
    }
}

fn send_fault(
    output: &Output,
    next_sequence: &Arc<AtomicU64>,
    policy: &WorkerRuntimePolicy,
    request_id: Option<String>,
    code: &str,
    detail: &str,
) -> Result<(), String> {
    send_message(output, next_sequence, policy, |sequence| {
        WorkerMessage::Fault {
            sequence,
            request_id,
            code: code.into(),
            detail: detail
                .chars()
                .take(policy.max_diagnostic_bytes as usize)
                .collect(),
        }
    })
}

fn send_message(
    output: &Output,
    next_sequence: &Arc<AtomicU64>,
    policy: &WorkerRuntimePolicy,
    build: impl FnOnce(u64) -> WorkerMessage,
) -> Result<(), String> {
    let mut output = output
        .lock()
        .map_err(|_| "worker-output-lock-failed".to_owned())?;
    let sequence = next_sequence.fetch_add(1, Ordering::AcqRel);
    let frame = encode_frame(
        &build(sequence),
        policy
            .max_frame_bytes_usize()
            .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    if frame.len()
        > policy
            .max_output_bytes_usize()
            .map_err(|error| error.to_string())?
    {
        return Err("worker-output-limit".into());
    }
    output
        .write_all(&frame)
        .and_then(|_| output.flush())
        .map_err(|_| "worker-ipc-write-failed".to_owned())
}

fn spawn_heartbeat(
    output: Output,
    next_sequence: Arc<AtomicU64>,
    policy: WorkerRuntimePolicy,
    stop: Arc<AtomicBool>,
) {
    thread::spawn(move || {
        while !stop.load(Ordering::Acquire) {
            thread::sleep(Duration::from_millis(policy.heartbeat_interval_ms));
            if stop.load(Ordering::Acquire) {
                break;
            }
            if send_message(&output, &next_sequence, &policy, |sequence| {
                WorkerMessage::Heartbeat {
                    sequence,
                    observed_at_ms: unix_now_ms(),
                    state: WorkerHealthState::Ready,
                }
            })
            .is_err()
            {
                break;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_values_become_no_target_inputs() {
        let clock = DecisionClock::ClosedBar {
            decision_id: "d1".into(),
            instrument_id: "BTC-USDT".into(),
            decision_time_ms: 10,
            available_at_ms: 10,
            deadline_ms: 20,
            next_execution_ms: 21,
        };
        let input = WorkerDecisionInput::Strategy {
            instrument_id: "BTC-USDT".into(),
            frames: vec![WorkerFeatureFrame {
                instrument_id: "BTC-USDT".into(),
                open_time_ms: 9,
                available_at_ms: 10,
                values: vec![None],
            }],
        };
        let binding = adaq_bot_runtime::WorkerStrategyBinding {
            world: StrategyWorld::Strategy,
            component_sha256: "a".repeat(64),
            feature_slots: vec!["close".into()],
            parameters: Vec::new(),
        };
        let error = prepare_input(
            &clock,
            &input,
            &binding,
            &WorkerPipelineBinding::default(),
            &WorkerRuntimePolicy::default(),
        )
        .expect_err("missing value should pause");
        assert!(matches!(
            error,
            InputError::NoTarget(NoTargetReason::MissingInput, _)
        ));
    }
}
