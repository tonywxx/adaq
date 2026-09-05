//! Host-owned contracts for the supervised, fail-closed Paper Trading Worker.
//!
//! This crate contains the private IPC protocol, immutable deployment binding,
//! artifact verification, and process supervision. It deliberately contains no
//! provider, credential, Risk, OMS, or order API.

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use ed25519_dalek::{Signature, Signer, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    io::{BufWriter, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver, RecvTimeoutError, TryRecvError},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub const WORKER_ARTIFACT_NAME: &str = "adaq-bot-worker";
pub const WORKER_ARTIFACT_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const WORKER_PROTOCOL_VERSION: &str = "adaq-bot-worker-ipc@1.1.0";
pub const WORKER_RUNTIME_VERSION: &str = concat!("adaq-bot-runtime@", env!("CARGO_PKG_VERSION"));
pub const WORKER_SIGNATURE_SCHEMA_VERSION: &str = "adaq-bot-worker-signature@1.0.0";
pub const WORKER_SIGNING_KEY_ID: &str = "adaq-bot-worker-ed25519-v1";
pub const MAX_COMPONENT_BYTES: usize = 64 * 1024 * 1024;
const MAX_SIGNATURE_BYTES: usize = 16 * 1024;
const MAX_BUNDLE_HASHES: usize = 64;
const MAX_ID_BYTES: usize = 256;
const MAX_WORKER_FUEL: u64 = 1_000_000_000;
const MAX_WORKER_MEMORY_BYTES: u64 = 512 * 1024 * 1024;
const MAX_WORKER_TIMEOUT_MS: u64 = 60_000;
const MAX_WORKER_HEARTBEAT_TIMEOUT_MS: u64 = 120_000;

// Generated once for the dedicated Worker signing root. Only the public key is
// in the repository; the matching private key belongs in the release secret.
const WORKER_TRUST_ROOT_PUBLIC_KEY: [u8; 32] = [
    0x7f, 0xba, 0x8a, 0x5f, 0xfc, 0xfa, 0x6b, 0xbe, 0xd2, 0x39, 0xde, 0x44, 0xb2, 0xe0, 0x77, 0xfb,
    0xd2, 0x78, 0xfe, 0x1d, 0xd4, 0xb4, 0x4d, 0x6c, 0xad, 0xf9, 0xed, 0x6a, 0xf8, 0xd0, 0x49, 0x02,
];

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerRuntimePolicy {
    pub max_frame_bytes: u64,
    pub max_output_bytes: u64,
    pub startup_timeout_ms: u64,
    pub handshake_timeout_ms: u64,
    pub heartbeat_interval_ms: u64,
    pub heartbeat_timeout_ms: u64,
    pub decision_timeout_ms: u64,
    pub fuel_per_call: u64,
    pub memory_bytes: u64,
    pub process_memory_bytes: u64,
    pub process_cpu_time_ms: u64,
    pub max_decision_frames: u64,
    pub warmup_decisions: u64,
    pub max_diagnostic_bytes: u64,
}

impl Default for WorkerRuntimePolicy {
    fn default() -> Self {
        Self {
            max_frame_bytes: 64 * 1024 * 1024,
            max_output_bytes: 1024 * 1024,
            startup_timeout_ms: 5_000,
            handshake_timeout_ms: 5_000,
            heartbeat_interval_ms: 1_000,
            heartbeat_timeout_ms: 3_000,
            decision_timeout_ms: 1_000,
            fuel_per_call: 10_000_000,
            memory_bytes: 64 * 1024 * 1024,
            process_memory_bytes: 512 * 1024 * 1024,
            process_cpu_time_ms: 60_000,
            max_decision_frames: 4_096,
            warmup_decisions: 0,
            max_diagnostic_bytes: 4_096,
        }
    }
}

impl WorkerRuntimePolicy {
    pub fn validate(&self) -> Result<(), RuntimeError> {
        if !(1_024..=64 * 1024 * 1024).contains(&self.max_frame_bytes)
            || self.max_output_bytes == 0
            || self.max_output_bytes > self.max_frame_bytes
            || self.startup_timeout_ms == 0
            || self.startup_timeout_ms > MAX_WORKER_TIMEOUT_MS
            || self.handshake_timeout_ms == 0
            || self.handshake_timeout_ms > MAX_WORKER_TIMEOUT_MS
            || self.heartbeat_interval_ms == 0
            || self.heartbeat_interval_ms > MAX_WORKER_TIMEOUT_MS
            || self.heartbeat_timeout_ms < self.heartbeat_interval_ms
            || self.heartbeat_timeout_ms > MAX_WORKER_HEARTBEAT_TIMEOUT_MS
            || self.decision_timeout_ms == 0
            || self.decision_timeout_ms > MAX_WORKER_TIMEOUT_MS
            || self.fuel_per_call == 0
            || self.fuel_per_call > MAX_WORKER_FUEL
            || self.memory_bytes == 0
            || self.memory_bytes > MAX_WORKER_MEMORY_BYTES
            || self.process_memory_bytes < self.memory_bytes
            || self.process_memory_bytes > MAX_WORKER_MEMORY_BYTES
            || self.process_cpu_time_ms == 0
            || self.process_cpu_time_ms > MAX_WORKER_TIMEOUT_MS
            || self.max_decision_frames == 0
            || self.max_decision_frames > 1_000_000
            || self.max_diagnostic_bytes == 0
            || self.max_diagnostic_bytes > 64 * 1024
        {
            return Err(RuntimeError::InvalidPolicy);
        }
        Ok(())
    }

    pub fn max_frame_bytes_usize(&self) -> Result<usize, RuntimeError> {
        usize::try_from(self.max_frame_bytes).map_err(|_| RuntimeError::InvalidPolicy)
    }

    pub fn max_output_bytes_usize(&self) -> Result<usize, RuntimeError> {
        usize::try_from(self.max_output_bytes).map_err(|_| RuntimeError::InvalidPolicy)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub enum StrategyWorld {
    Strategy,
    PortfolioStrategy,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "kebab-case",
    deny_unknown_fields
)]
pub enum WorkerParameterValue {
    Decimal(String),
    Integer(i64),
    Boolean(bool),
    String(String),
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerStrategyBinding {
    pub world: StrategyWorld,
    pub component_sha256: String,
    pub feature_slots: Vec<String>,
    pub parameters: Vec<WorkerParameterValue>,
}

impl WorkerStrategyBinding {
    fn validate(&self) -> Result<(), RuntimeError> {
        if !is_sha256(&self.component_sha256)
            || self.feature_slots.is_empty()
            || self.feature_slots.len() > 64
            || self
                .feature_slots
                .iter()
                .any(|slot| !is_bounded_text(slot, 128))
            || self.parameters.len() > 64
        {
            return Err(RuntimeError::InvalidStrategyBinding);
        }
        let mut slots = HashSet::new();
        if self
            .feature_slots
            .iter()
            .any(|slot| !slots.insert(slot.as_str()))
        {
            return Err(RuntimeError::InvalidStrategyBinding);
        }
        if !valid_worker_parameters(&self.parameters) {
            return Err(RuntimeError::InvalidStrategyBinding);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub enum WorkerFactorScope {
    TimeSeries,
    CrossSectional,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerFactorBinding {
    pub scope: WorkerFactorScope,
    pub component_sha256: String,
    pub feature_slots: Vec<String>,
    pub output_names: Vec<String>,
    pub warmup_bars: u64,
    pub parameters: Vec<WorkerParameterValue>,
}

impl WorkerFactorBinding {
    fn validate(&self) -> Result<(), RuntimeError> {
        if !is_sha256(&self.component_sha256)
            || self.feature_slots.is_empty()
            || self.feature_slots.len() > 64
            || self
                .feature_slots
                .iter()
                .any(|slot| !is_bounded_text(slot, 128))
            || self.output_names.is_empty()
            || self.output_names.len() > 64
            || self
                .output_names
                .iter()
                .any(|name| !is_lower_kebab_text(name))
            || self.warmup_bars > 1_000_000
            || !valid_worker_parameters(&self.parameters)
        {
            return Err(RuntimeError::InvalidPipelineBinding);
        }
        if has_duplicates(&self.feature_slots) || has_duplicates(&self.output_names) {
            return Err(RuntimeError::InvalidPipelineBinding);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerModelBinding {
    pub component_sha256: String,
    pub feature_slots: Vec<String>,
    pub output_names: Vec<String>,
    pub seed: u64,
    pub parameters: Vec<WorkerParameterValue>,
}

impl WorkerModelBinding {
    fn validate(&self) -> Result<(), RuntimeError> {
        if !is_sha256(&self.component_sha256)
            || self.feature_slots.is_empty()
            || self.feature_slots.len() > 64
            || self
                .feature_slots
                .iter()
                .any(|slot| !is_bounded_text(slot, 128))
            || self.output_names.is_empty()
            || self.output_names.len() > 64
            || self
                .output_names
                .iter()
                .any(|name| !is_lower_kebab_text(name))
            || !valid_worker_parameters(&self.parameters)
        {
            return Err(RuntimeError::InvalidPipelineBinding);
        }
        if has_duplicates(&self.feature_slots) || has_duplicates(&self.output_names) {
            return Err(RuntimeError::InvalidPipelineBinding);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerPipelineBinding {
    pub input_slots: Vec<String>,
    pub factors: Vec<WorkerFactorBinding>,
    pub models: Vec<WorkerModelBinding>,
}

impl WorkerPipelineBinding {
    fn validate(
        &self,
        component_hashes: &[String],
        model_hashes: &[String],
    ) -> Result<(), RuntimeError> {
        if self.input_slots.len() > 64
            || self
                .input_slots
                .iter()
                .any(|slot| !is_bounded_text(slot, 128))
            || has_duplicates(&self.input_slots)
            || self.factors.len() > 64
            || self.models.len() > 64
            || (!self.factors.is_empty() || !self.models.is_empty()) && self.input_slots.is_empty()
        {
            return Err(RuntimeError::InvalidPipelineBinding);
        }
        let mut component_ids = HashSet::new();
        let mut output_names = HashSet::new();
        let input_names = self
            .input_slots
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        for factor in &self.factors {
            factor.validate()?;
            if !component_hashes.contains(&factor.component_sha256)
                || !component_ids.insert(factor.component_sha256.as_str())
                || factor.output_names.iter().any(|name| {
                    input_names.contains(name.as_str()) || !output_names.insert(name.as_str())
                })
            {
                return Err(RuntimeError::InvalidPipelineBinding);
            }
        }
        for model in &self.models {
            model.validate()?;
            if !model_hashes.contains(&model.component_sha256)
                || !component_ids.insert(model.component_sha256.as_str())
                || model.output_names.iter().any(|name| {
                    input_names.contains(name.as_str()) || !output_names.insert(name.as_str())
                })
            {
                return Err(RuntimeError::InvalidPipelineBinding);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerComponentPayload {
    pub component_sha256: String,
    pub wasm: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerArtifactBinding {
    pub artifact_name: String,
    pub artifact_version: String,
    pub platform: String,
    pub protocol_version: String,
    pub runtime_version: String,
    pub sha256: String,
    pub signing_key_id: String,
    pub signature: String,
}

impl WorkerArtifactBinding {
    fn validate(&self) -> Result<(), RuntimeError> {
        if self.artifact_name != WORKER_ARTIFACT_NAME
            || !is_bounded_text(&self.artifact_version, 64)
            || !is_bounded_text(&self.platform, 64)
            || self.protocol_version != WORKER_PROTOCOL_VERSION
            || !is_bounded_text(&self.runtime_version, 128)
            || !is_sha256(&self.sha256)
            || self.signing_key_id != WORKER_SIGNING_KEY_ID
            || self.signature.len() != 128
            || !self.signature.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(RuntimeError::InvalidWorkerBinding);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
    pub strategy: WorkerStrategyBinding,
    #[serde(default)]
    pub pipeline: WorkerPipelineBinding,
    pub worker: WorkerArtifactBinding,
    pub worker_policy: WorkerRuntimePolicy,
}

impl DeploymentBundleInput {
    fn validate(&self) -> Result<(), RuntimeError> {
        if !is_bounded_text(&self.bot_id, MAX_ID_BYTES)
            || !is_bounded_text(&self.strategy_id, MAX_ID_BYTES)
            || !is_bounded_text(&self.account_id, MAX_ID_BYTES)
            || self.component_hashes.is_empty()
            || self.component_hashes.len() > MAX_BUNDLE_HASHES
            || self.component_hashes.iter().any(|hash| !is_sha256(hash))
            || self.model_hashes.len() > MAX_BUNDLE_HASHES
            || self.model_hashes.iter().any(|hash| !is_sha256(hash))
            || !is_sha256(&self.feature_plan_hash)
            || !is_sha256(&self.execution_profile_hash)
            || !is_sha256(&self.qualification_evidence_hash)
            || !is_sha256(&self.worker_binary_hash)
            || self.worker_binary_hash != self.worker.sha256
            || !self
                .component_hashes
                .contains(&self.strategy.component_sha256)
        {
            return Err(RuntimeError::BundleNotQualified);
        }
        self.strategy.validate()?;
        self.pipeline
            .validate(&self.component_hashes, &self.model_hashes)?;
        self.worker.validate()?;
        self.worker_policy.validate()?;
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeploymentBundle {
    pub input: DeploymentBundleInput,
    pub identity: String,
}

impl DeploymentBundle {
    pub fn freeze(input: DeploymentBundleInput) -> Result<Self, RuntimeError> {
        input.validate()?;
        let bytes = serde_json::to_vec(&input).map_err(|_| RuntimeError::Serialization)?;
        Ok(Self {
            input,
            identity: sha256_hex(&bytes),
        })
    }

    pub fn verify(&self) -> Result<(), RuntimeError> {
        self.input.validate()?;
        let bytes = serde_json::to_vec(&self.input).map_err(|_| RuntimeError::Serialization)?;
        if self.identity == sha256_hex(&bytes) {
            Ok(())
        } else {
            Err(RuntimeError::BundleMutated)
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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

    pub fn decision_id(&self) -> &str {
        match self {
            Self::ClosedBar { decision_id, .. }
            | Self::ScheduledCrossSection { decision_id, .. } => decision_id,
        }
    }

    pub fn decision_time_ms(&self) -> i64 {
        match self {
            Self::ClosedBar {
                decision_time_ms, ..
            }
            | Self::ScheduledCrossSection {
                decision_time_ms, ..
            } => *decision_time_ms,
        }
    }

    pub fn deadline_ms(&self) -> i64 {
        match self {
            Self::ClosedBar { deadline_ms, .. }
            | Self::ScheduledCrossSection { deadline_ms, .. } => *deadline_ms,
        }
    }

    pub fn accepts_target(
        &self,
        decision_id: &str,
        produced_at_ms: i64,
    ) -> Result<(), RuntimeError> {
        self.validate()?;
        if self.decision_id() != decision_id {
            return Err(RuntimeError::StaleTarget);
        }
        if produced_at_ms > self.deadline_ms() {
            return Err(RuntimeError::DeadlineMissed);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerFeatureFrame {
    pub instrument_id: String,
    pub open_time_ms: i64,
    pub available_at_ms: i64,
    pub values: Vec<Option<f64>>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerFeatureRow {
    pub instrument_id: String,
    pub available_at_ms: i64,
    pub values: Vec<Option<f64>>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerEvaluationValue {
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerEvaluationRow {
    pub instrument_id: String,
    pub observation_time_ms: i64,
    pub available_at_ms: i64,
    pub factor_outputs: Vec<WorkerEvaluationValue>,
    pub model_outputs: Vec<WorkerEvaluationValue>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerEvaluationEvidence {
    pub rows: Vec<WorkerEvaluationRow>,
}

const MAX_EVALUATION_ROWS: usize = 4_096;
const MAX_EVALUATION_OUTPUTS_PER_ROW: usize = 128;

impl WorkerEvaluationEvidence {
    pub fn validate(&self) -> Result<(), String> {
        if self.rows.len() > MAX_EVALUATION_ROWS {
            return Err("evaluation evidence row limit exceeded".into());
        }
        for row in &self.rows {
            if !is_bounded_text(&row.instrument_id, 128)
                || row.observation_time_ms < 0
                || row.available_at_ms < 0
                || row.available_at_ms > row.observation_time_ms
            {
                return Err("evaluation evidence row identity is invalid".into());
            }
            let output_count = row
                .factor_outputs
                .len()
                .saturating_add(row.model_outputs.len());
            if output_count > MAX_EVALUATION_OUTPUTS_PER_ROW {
                return Err("evaluation evidence output limit exceeded".into());
            }
            let mut names = std::collections::BTreeSet::new();
            for output in row.factor_outputs.iter().chain(&row.model_outputs) {
                if output.name.len() > 128
                    || !is_lower_kebab_text(&output.name)
                    || !is_decimal_text(&output.value)
                    || !names.insert(&output.name)
                {
                    return Err("evaluation evidence output is invalid".into());
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerPosition {
    pub instrument_id: String,
    pub quantity: String,
    pub price: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerPortfolioState {
    pub cash: String,
    pub positions: Vec<WorkerPosition>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "kebab-case",
    deny_unknown_fields
)]
pub enum WorkerDecisionInput {
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

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerExposure {
    pub instrument_id: String,
    pub exposure: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerTargetWeight {
    pub instrument_id: String,
    pub weight: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "kebab-case",
    deny_unknown_fields
)]
pub enum WorkerTarget {
    Strategy {
        instrument_id: String,
        exposures: Vec<WorkerExposure>,
    },
    Portfolio {
        universe_id: String,
        weights: Vec<WorkerTargetWeight>,
        cash_reserve: String,
    },
}

impl WorkerTarget {
    pub fn validate_for(
        &self,
        world: &StrategyWorld,
        input: &WorkerDecisionInput,
    ) -> Result<(), RuntimeError> {
        match (world, self, input) {
            (
                StrategyWorld::Strategy,
                Self::Strategy {
                    instrument_id,
                    exposures,
                },
                WorkerDecisionInput::Strategy {
                    instrument_id: expected,
                    frames,
                },
            ) => {
                if instrument_id != expected
                    || exposures.len() != frames.len()
                    || exposures.iter().zip(frames).any(|(exposure, frame)| {
                        exposure.instrument_id != *expected
                            || !is_decimal_text(&exposure.exposure)
                            || !exposure.exposure.chars().all(|c| !c.is_control())
                            || frame.instrument_id != *expected
                    })
                {
                    return Err(RuntimeError::InvalidTarget);
                }
            }
            (
                StrategyWorld::PortfolioStrategy,
                Self::Portfolio {
                    universe_id,
                    weights,
                    cash_reserve,
                },
                WorkerDecisionInput::Portfolio {
                    universe_id: expected,
                    rows,
                    ..
                },
            ) => {
                if universe_id != expected
                    || !is_decimal_text(cash_reserve)
                    || weights.len() != rows.len()
                    || weights.iter().any(|weight| {
                        !is_decimal_text(&weight.weight) || weight.instrument_id.trim().is_empty()
                    })
                    || weights
                        .iter()
                        .map(|weight| weight.instrument_id.as_str())
                        .collect::<HashSet<_>>()
                        .len()
                        != weights.len()
                    || weights.iter().any(|weight| {
                        !rows
                            .iter()
                            .any(|row| row.instrument_id == weight.instrument_id)
                    })
                {
                    return Err(RuntimeError::InvalidTarget);
                }
            }
            _ => return Err(RuntimeError::InvalidTarget),
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum NoTargetReason {
    Warmup,
    MissingInput,
    IncompleteUniverse,
    DeadlineMissed,
    StaleDecision,
    NoSignal,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum WorkerHealthState {
    Starting,
    Ready,
    Busy,
    Faulted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkerHealthEvent {
    Heartbeat {
        observed_at_ms: i64,
        state: WorkerHealthState,
    },
    Diagnostic {
        code: String,
        detail: String,
    },
    Fault {
        code: String,
        detail: String,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum WorkerMessage {
    Hello {
        sequence: u64,
        protocol_version: String,
        artifact_name: String,
        artifact_version: String,
        platform: String,
        runtime_version: String,
        artifact_sha256: String,
    },
    Initialize {
        sequence: u64,
        request_id: String,
        bundle: DeploymentBundle,
        component_wasm: String,
        pipeline_components: Vec<WorkerComponentPayload>,
    },
    Initialized {
        sequence: u64,
        request_id: String,
        bundle_identity: String,
        world: StrategyWorld,
    },
    Decision {
        sequence: u64,
        request_id: String,
        clock: DecisionClock,
        input: WorkerDecisionInput,
    },
    Target {
        sequence: u64,
        request_id: String,
        decision_id: String,
        produced_at_ms: i64,
        target: WorkerTarget,
        evaluation: WorkerEvaluationEvidence,
    },
    NoTarget {
        sequence: u64,
        request_id: String,
        decision_id: String,
        reason: NoTargetReason,
        detail: String,
    },
    Heartbeat {
        sequence: u64,
        observed_at_ms: i64,
        state: WorkerHealthState,
    },
    Diagnostic {
        sequence: u64,
        request_id: Option<String>,
        code: String,
        detail: String,
    },
    Fault {
        sequence: u64,
        request_id: Option<String>,
        code: String,
        detail: String,
    },
    Shutdown {
        sequence: u64,
        request_id: String,
    },
    ShutdownAck {
        sequence: u64,
        request_id: String,
    },
}

impl WorkerMessage {
    pub fn sequence(&self) -> u64 {
        match self {
            Self::Hello { sequence, .. }
            | Self::Initialize { sequence, .. }
            | Self::Initialized { sequence, .. }
            | Self::Decision { sequence, .. }
            | Self::Target { sequence, .. }
            | Self::NoTarget { sequence, .. }
            | Self::Heartbeat { sequence, .. }
            | Self::Diagnostic { sequence, .. }
            | Self::Fault { sequence, .. }
            | Self::Shutdown { sequence, .. }
            | Self::ShutdownAck { sequence, .. } => *sequence,
        }
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
    InvalidPolicy,
    InvalidStrategyBinding,
    InvalidPipelineBinding,
    InvalidWorkerBinding,
    InvalidTarget,
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for RuntimeError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProtocolError {
    OversizedFrame,
    EmptyFrame,
    MalformedFrame,
    UnknownMessage,
    OutOfOrder { expected: u64, received: u64 },
    Duplicate { expected: u64, received: u64 },
    InvalidSession,
    Io,
}

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutOfOrder { expected, received } => {
                write!(
                    f,
                    "out-of-order message: expected {expected}, received {received}"
                )
            }
            Self::Duplicate { expected, received } => {
                write!(
                    f,
                    "duplicate message: expected {expected}, received {received}"
                )
            }
            _ => write!(f, "{self:?}"),
        }
    }
}

impl std::error::Error for ProtocolError {}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn current_platform_tag() -> String {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin".into(),
        ("windows", "x86_64") => "x86_64-pc-windows-msvc".into(),
        _ => format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS),
    }
}

pub fn unix_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or_default()
}

pub fn encode_frame(
    message: &WorkerMessage,
    max_frame_bytes: usize,
) -> Result<Vec<u8>, ProtocolError> {
    let mut frame = serde_json::to_vec(message).map_err(|_| ProtocolError::MalformedFrame)?;
    if frame.len().saturating_add(1) > max_frame_bytes {
        return Err(ProtocolError::OversizedFrame);
    }
    frame.push(b'\n');
    Ok(frame)
}

pub fn decode_frame(frame: &[u8], max_frame_bytes: usize) -> Result<WorkerMessage, ProtocolError> {
    if frame.len() > max_frame_bytes {
        return Err(ProtocolError::OversizedFrame);
    }
    let frame = frame.strip_suffix(b"\n").unwrap_or(frame);
    let frame = frame.strip_suffix(b"\r").unwrap_or(frame);
    if frame.is_empty() {
        return Err(ProtocolError::EmptyFrame);
    }
    let value: Value = serde_json::from_slice(frame).map_err(|_| ProtocolError::MalformedFrame)?;
    validate_message_shape(&value)?;
    serde_json::from_value(value).map_err(|_| ProtocolError::UnknownMessage)
}

pub fn read_bounded_line<R: Read>(
    reader: &mut R,
    max_frame_bytes: usize,
) -> Result<Option<Vec<u8>>, ProtocolError> {
    let mut frame = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match reader.read(&mut byte) {
            Ok(0) if frame.is_empty() => return Ok(None),
            Ok(0) => return Ok(Some(frame)),
            Ok(_) => {
                frame.push(byte[0]);
                if frame.len() > max_frame_bytes {
                    return Err(ProtocolError::OversizedFrame);
                }
                if byte[0] == b'\n' {
                    return Ok(Some(frame));
                }
            }
            Err(_) => return Err(ProtocolError::Io),
        }
    }
}

fn validate_message_shape(value: &Value) -> Result<(), ProtocolError> {
    let object = value.as_object().ok_or(ProtocolError::MalformedFrame)?;
    let kind = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or(ProtocolError::MalformedFrame)?;
    let allowed = match kind {
        "hello" => [
            "type",
            "sequence",
            "protocolVersion",
            "artifactName",
            "artifactVersion",
            "platform",
            "runtimeVersion",
            "artifactSha256",
        ]
        .as_slice(),
        "initialize" => [
            "type",
            "sequence",
            "requestId",
            "bundle",
            "componentWasm",
            "pipelineComponents",
        ]
        .as_slice(),
        "initialized" => ["type", "sequence", "requestId", "bundleIdentity", "world"].as_slice(),
        "decision" => ["type", "sequence", "requestId", "clock", "input"].as_slice(),
        "target" => [
            "type",
            "sequence",
            "requestId",
            "decisionId",
            "producedAtMs",
            "target",
            "evaluation",
        ]
        .as_slice(),
        "no-target" => [
            "type",
            "sequence",
            "requestId",
            "decisionId",
            "reason",
            "detail",
        ]
        .as_slice(),
        "heartbeat" => ["type", "sequence", "observedAtMs", "state"].as_slice(),
        "diagnostic" | "fault" => ["type", "sequence", "requestId", "code", "detail"].as_slice(),
        "shutdown" => ["type", "sequence", "requestId"].as_slice(),
        "shutdown-ack" => ["type", "sequence", "requestId"].as_slice(),
        _ => return Err(ProtocolError::UnknownMessage),
    };
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(ProtocolError::MalformedFrame);
    }
    Ok(())
}

pub struct ProtocolSequence {
    next: u64,
}

impl Default for ProtocolSequence {
    fn default() -> Self {
        Self { next: 1 }
    }
}

impl ProtocolSequence {
    pub fn next(&mut self) -> u64 {
        let sequence = self.next;
        self.next = self.next.saturating_add(1);
        sequence
    }

    pub fn accept(&mut self, received: u64) -> Result<(), ProtocolError> {
        if received < self.next {
            return Err(ProtocolError::Duplicate {
                expected: self.next,
                received,
            });
        }
        if received > self.next {
            return Err(ProtocolError::OutOfOrder {
                expected: self.next,
                received,
            });
        }
        self.next = self.next.saturating_add(1);
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerArtifactSignature {
    pub schema_version: String,
    pub artifact_name: String,
    pub artifact_version: String,
    pub platform: String,
    pub protocol_version: String,
    pub runtime_version: String,
    pub artifact_sha256: String,
    pub signing_key_id: String,
    pub signature: String,
}

impl WorkerArtifactSignature {
    pub fn sign(
        bytes: &[u8],
        platform: impl Into<String>,
        signing_key_bytes: &[u8; 32],
    ) -> Result<Self, String> {
        let platform = platform.into();
        let signing_key = ed25519_dalek::SigningKey::from_bytes(signing_key_bytes);
        let artifact_sha256 = sha256_hex(bytes);
        let signature = signing_key.sign(&signature_payload(
            &platform,
            &artifact_sha256,
            WORKER_SIGNING_KEY_ID,
        ));
        Ok(Self {
            schema_version: WORKER_SIGNATURE_SCHEMA_VERSION.into(),
            artifact_name: WORKER_ARTIFACT_NAME.into(),
            artifact_version: WORKER_ARTIFACT_VERSION.into(),
            platform,
            protocol_version: WORKER_PROTOCOL_VERSION.into(),
            runtime_version: WORKER_RUNTIME_VERSION.into(),
            artifact_sha256,
            signing_key_id: WORKER_SIGNING_KEY_ID.into(),
            signature: hex_encode(&signature.to_bytes()),
        })
    }
}

#[derive(Clone)]
pub struct WorkerTrustRoot {
    pub key_id: String,
    pub public_key: [u8; 32],
}

impl Default for WorkerTrustRoot {
    fn default() -> Self {
        Self {
            key_id: WORKER_SIGNING_KEY_ID.into(),
            public_key: WORKER_TRUST_ROOT_PUBLIC_KEY,
        }
    }
}

#[derive(Clone, Default)]
pub struct WorkerArtifactVerifier {
    trust_root: WorkerTrustRoot,
}

impl WorkerArtifactVerifier {
    pub fn with_trust_root(trust_root: WorkerTrustRoot) -> Self {
        Self { trust_root }
    }

    pub fn verify_file(
        &self,
        artifact_path: &Path,
        signature_path: &Path,
        expected: &WorkerArtifactBinding,
    ) -> Result<(), String> {
        let bytes = fs::read(artifact_path).map_err(|_| "worker-artifact-missing".to_owned())?;
        let signature_bytes =
            fs::read(signature_path).map_err(|_| "worker-signature-missing".to_owned())?;
        if signature_bytes.len() > MAX_SIGNATURE_BYTES {
            return Err("worker-signature-too-large".into());
        }
        let signature = serde_json::from_slice::<WorkerArtifactSignature>(&signature_bytes)
            .map_err(|_| "worker-signature-malformed".to_owned())?;
        self.verify_bytes(&bytes, &signature, expected)
    }

    pub fn verify_bytes(
        &self,
        bytes: &[u8],
        signature: &WorkerArtifactSignature,
        expected: &WorkerArtifactBinding,
    ) -> Result<(), String> {
        if signature.schema_version != WORKER_SIGNATURE_SCHEMA_VERSION
            || signature.artifact_name != expected.artifact_name
            || signature.artifact_version != expected.artifact_version
            || signature.platform != expected.platform
            || signature.protocol_version != expected.protocol_version
            || signature.runtime_version != expected.runtime_version
            || signature.artifact_sha256 != sha256_hex(bytes)
            || signature.artifact_sha256 != expected.sha256
            || signature.signing_key_id != expected.signing_key_id
            || signature.signing_key_id != self.trust_root.key_id
            || signature.signature != expected.signature
        {
            return Err("worker-artifact-identity-mismatch".into());
        }
        let public_key = VerifyingKey::from_bytes(&self.trust_root.public_key)
            .map_err(|_| "worker-trust-root-invalid".to_owned())?;
        let signature_bytes = hex_decode(&signature.signature)
            .ok_or_else(|| "worker-signature-malformed".to_owned())?;
        let ed_signature = Signature::from_slice(&signature_bytes)
            .map_err(|_| "worker-signature-malformed".to_owned())?;
        public_key
            .verify(
                &signature_payload(
                    &signature.platform,
                    &signature.artifact_sha256,
                    &signature.signing_key_id,
                ),
                &ed_signature,
            )
            .map_err(|_| "worker-signature-invalid".to_owned())
    }
}

fn signature_payload(platform: &str, artifact_sha256: &str, signing_key_id: &str) -> Vec<u8> {
    format!(
        "{WORKER_SIGNATURE_SCHEMA_VERSION}\n{WORKER_ARTIFACT_NAME}\n{WORKER_ARTIFACT_VERSION}\n{platform}\n{WORKER_PROTOCOL_VERSION}\n{WORKER_RUNTIME_VERSION}\n{artifact_sha256}\n{signing_key_id}"
    )
    .into_bytes()
}

pub fn is_decimal_text(value: &str) -> bool {
    let mut value = value;
    if let Some(rest) = value.strip_prefix('-') {
        value = rest;
    }
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if whole.is_empty()
        || (whole.len() > 1 && whole.starts_with('0'))
        || !whole.chars().all(|c| c.is_ascii_digit())
    {
        return false;
    }
    (fraction.is_empty() && !value.contains('.')) || fraction.chars().all(|c| c.is_ascii_digit())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_bounded_text(value: &str, max_bytes: usize) -> bool {
    !value.trim().is_empty()
        && value.len() <= max_bytes
        && value.chars().all(|character| !character.is_control())
}

fn is_lower_kebab_text(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('-')
        && !value.ends_with('-')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !value.contains("--")
}

fn has_duplicates(values: &[String]) -> bool {
    let mut seen = HashSet::new();
    !values.iter().all(|value| seen.insert(value.as_str()))
}

fn valid_worker_parameters(parameters: &[WorkerParameterValue]) -> bool {
    parameters.iter().all(|parameter| match parameter {
        WorkerParameterValue::Decimal(value) => is_decimal_text(value),
        WorkerParameterValue::String(value) => is_bounded_text(value, 256),
        WorkerParameterValue::Integer(_) | WorkerParameterValue::Boolean(_) => true,
    })
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn hex_decode(value: &str) -> Option<Vec<u8>> {
    if value.len() % 2 != 0 {
        return None;
    }
    (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&value[index..index + 2], 16).ok())
        .collect()
}

#[derive(Clone, Debug)]
pub struct WorkerComponentLaunch {
    pub component_sha256: String,
    pub wasm: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct WorkerLaunchRequest {
    pub bundle: DeploymentBundle,
    pub artifact_path: PathBuf,
    pub signature_path: PathBuf,
    pub component_wasm: Vec<u8>,
    pub pipeline_components: Vec<WorkerComponentLaunch>,
    /// Kept for source compatibility; Worker startup rejects all arguments.
    pub extra_args: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum WorkerDecisionResult {
    Target {
        request_id: String,
        decision_id: String,
        produced_at_ms: i64,
        target: WorkerTarget,
        evaluation: WorkerEvaluationEvidence,
    },
    NoTarget {
        request_id: String,
        decision_id: String,
        reason: NoTargetReason,
        detail: String,
    },
}

#[derive(Debug)]
enum ReaderEvent {
    Frame(Vec<u8>),
    Eof,
    Error(ProtocolError),
}

pub struct WorkerSupervisor {
    child: Child,
    #[cfg(windows)]
    process_job: WindowsWorkerJob,
    stdin: BufWriter<std::process::ChildStdin>,
    messages: Receiver<ReaderEvent>,
    outbound: ProtocolSequence,
    inbound: ProtocolSequence,
    policy: WorkerRuntimePolicy,
    attempt: RuntimeAttempt,
    last_heartbeat: Instant,
    initialized: bool,
    terminated: bool,
    seen_requests: HashSet<String>,
    diagnostics: Vec<String>,
    health_events: Vec<WorkerHealthEvent>,
}

#[cfg(windows)]
struct WindowsWorkerJob {
    handle: usize,
}

#[cfg(windows)]
impl Drop for WindowsWorkerJob {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(
                self.handle as windows_sys::Win32::Foundation::HANDLE,
            );
        }
    }
}

#[cfg(unix)]
fn configure_unix_worker_group_and_cpu(cpu_time_ms: u64) -> std::io::Result<()> {
    let process_group_result = unsafe { libc::setpgid(0, 0) };
    if process_group_result != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let cpu_seconds = cpu_time_ms.saturating_add(999) / 1_000;
    let cpu_limit = libc::rlimit {
        rlim_cur: cpu_seconds as libc::rlim_t,
        rlim_max: cpu_seconds as libc::rlim_t,
    };
    if unsafe { libc::setrlimit(libc::RLIMIT_CPU, &cpu_limit) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn configure_unix_worker_memory(memory_bytes: u64) -> std::io::Result<()> {
    let memory_limit = libc::rlimit {
        rlim_cur: memory_bytes as libc::rlim_t,
        rlim_max: memory_bytes as libc::rlim_t,
    };
    if unsafe { libc::setrlimit(libc::RLIMIT_AS, &memory_limit) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn configure_unix_worker_memory(_memory_bytes: u64) -> std::io::Result<()> {
    // macOS Wasmtime reserves virtual address space for linear memories, so
    // RSS limits are not a reliable process ceiling here.
    Ok(())
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn configure_unix_worker_memory(_memory_bytes: u64) -> std::io::Result<()> {
    Ok(())
}

pub fn enforce_worker_process_limits(policy: &WorkerRuntimePolicy) -> Result<(), String> {
    #[cfg(unix)]
    {
        configure_unix_worker_group_and_cpu(policy.process_cpu_time_ms)
            .and_then(|_| configure_unix_worker_memory(policy.process_memory_bytes))
            .map_err(|_| "worker-process-limits-failed".to_owned())?;
    }
    #[cfg(windows)]
    {
        let _ = policy;
    }
    Ok(())
}

#[cfg(windows)]
fn configure_windows_worker_job(
    child: &Child,
    memory_bytes: u64,
) -> Result<WindowsWorkerJob, String> {
    use std::{mem::size_of, os::windows::io::AsRawHandle, ptr::null};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_ACTIVE_PROCESS,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PROCESS_MEMORY,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };

    let process_memory_limit = usize::try_from(memory_bytes)
        .map_err(|_| "worker-process-memory-limit-too-large".to_owned())?;
    let handle = unsafe { CreateJobObjectW(null(), null()) };
    if handle.is_null() {
        return Err("worker-process-job-create-failed".into());
    }
    let job = WindowsWorkerJob {
        handle: handle as usize,
    };
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_ACTIVE_PROCESS
        | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        | JOB_OBJECT_LIMIT_PROCESS_MEMORY;
    limits.BasicLimitInformation.ActiveProcessLimit = 1;
    limits.ProcessMemoryLimit = process_memory_limit;
    let configured = unsafe {
        SetInformationJobObject(
            job.handle as windows_sys::Win32::Foundation::HANDLE,
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if configured == 0 {
        return Err("worker-process-job-configure-failed".into());
    }
    if unsafe {
        AssignProcessToJobObject(
            job.handle as windows_sys::Win32::Foundation::HANDLE,
            child.as_raw_handle(),
        )
    } == 0
    {
        return Err("worker-process-job-assign-failed".into());
    }
    Ok(job)
}

impl WorkerSupervisor {
    pub fn launch(request: WorkerLaunchRequest) -> Result<Self, String> {
        Self::launch_with_verifier(request, WorkerArtifactVerifier::default())
    }

    pub fn launch_with_verifier(
        request: WorkerLaunchRequest,
        verifier: WorkerArtifactVerifier,
    ) -> Result<Self, String> {
        request.bundle.verify().map_err(|error| error.to_string())?;
        let policy = request.bundle.input.worker_policy.clone();
        if request.bundle.input.worker.platform != current_platform_tag() {
            return Err("worker-platform-mismatch".into());
        }
        verifier.verify_file(
            &request.artifact_path,
            &request.signature_path,
            &request.bundle.input.worker,
        )?;
        if request.component_wasm.len() > MAX_COMPONENT_BYTES
            || sha256_hex(&request.component_wasm) != request.bundle.input.strategy.component_sha256
        {
            return Err("worker-component-identity-mismatch".into());
        }
        let expected_pipeline_hashes = request
            .bundle
            .input
            .pipeline
            .factors
            .iter()
            .map(|factor| factor.component_sha256.as_str())
            .chain(
                request
                    .bundle
                    .input
                    .pipeline
                    .models
                    .iter()
                    .map(|model| model.component_sha256.as_str()),
            )
            .collect::<HashSet<_>>();
        let provided_pipeline_hashes = request
            .pipeline_components
            .iter()
            .map(|component| component.component_sha256.as_str())
            .collect::<HashSet<_>>();
        if provided_pipeline_hashes != expected_pipeline_hashes
            || request.pipeline_components.iter().any(|component| {
                component.wasm.len() > MAX_COMPONENT_BYTES
                    || sha256_hex(&component.wasm) != component.component_sha256
            })
        {
            return Err("worker-pipeline-component-identity-mismatch".into());
        }
        if !request.extra_args.is_empty() {
            return Err("worker-arguments-not-supported".into());
        }
        let mut command = Command::new(&request.artifact_path);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            let process_cpu_time_ms = policy.process_cpu_time_ms;
            let process_memory_bytes = policy.process_memory_bytes;
            unsafe {
                command.pre_exec(move || {
                    let result = configure_unix_worker_group_and_cpu(process_cpu_time_ms);
                    #[cfg(target_os = "linux")]
                    let result =
                        result.and_then(|_| configure_unix_worker_memory(process_memory_bytes));
                    #[cfg(not(target_os = "linux"))]
                    let _ = process_memory_bytes;
                    result
                });
            }
        }
        command
            .env_clear()
            .env("ADAQ_BOT_WORKER_CHILD", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|_| "worker-process-start-failed".to_owned())?;
        #[cfg(windows)]
        let process_job = match configure_windows_worker_job(&child, policy.process_memory_bytes) {
            Ok(job) => job,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        let attempt =
            RuntimeAttempt::start(request.bundle.clone()).map_err(|error| error.to_string())?;
        let stdin = child.stdin.take().ok_or_else(|| {
            let _ = child.kill();
            let _ = child.wait();
            "worker-stdin-unavailable".to_owned()
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            let _ = child.kill();
            let _ = child.wait();
            "worker-stdout-unavailable".to_owned()
        })?;
        if let Some(stderr) = child.stderr.take() {
            spawn_stderr_drain(stderr);
        }
        let messages = spawn_stdout_reader(
            stdout,
            policy
                .max_frame_bytes_usize()
                .map_err(|error| error.to_string())?,
        );
        let mut supervisor = Self {
            child,
            #[cfg(windows)]
            process_job,
            stdin: BufWriter::new(stdin),
            messages,
            outbound: ProtocolSequence::default(),
            inbound: ProtocolSequence::default(),
            policy,
            attempt,
            last_heartbeat: Instant::now(),
            initialized: false,
            terminated: false,
            seen_requests: HashSet::new(),
            diagnostics: Vec::new(),
            health_events: Vec::new(),
        };

        let hello = supervisor.receive_until(
            Duration::from_millis(supervisor.policy.startup_timeout_ms),
            false,
        )?;
        match hello {
            WorkerMessage::Hello {
                protocol_version,
                artifact_name,
                artifact_version,
                platform,
                runtime_version,
                artifact_sha256,
                ..
            } if protocol_version == supervisor.attempt.bundle().input.worker.protocol_version
                && artifact_name == WORKER_ARTIFACT_NAME
                && artifact_version
                    == supervisor.attempt.bundle().input.worker.artifact_version
                && platform == supervisor.attempt.bundle().input.worker.platform
                && runtime_version == supervisor.attempt.bundle().input.worker.runtime_version
                && artifact_sha256 == supervisor.attempt.bundle().input.worker.sha256 => {}
            _ => return supervisor.startup_failure("worker-handshake-failed"),
        }
        let request_id = format!("{}:initialize", supervisor.attempt.bundle().identity);
        let component_wasm = BASE64.encode(&request.component_wasm);
        let pipeline_components = request
            .pipeline_components
            .iter()
            .map(|component| WorkerComponentPayload {
                component_sha256: component.component_sha256.clone(),
                wasm: BASE64.encode(&component.wasm),
            })
            .collect();
        let sequence = supervisor.outbound.next();
        if let Err(error) = supervisor.send(WorkerMessage::Initialize {
            sequence,
            request_id: request_id.clone(),
            bundle: request.bundle,
            component_wasm,
            pipeline_components,
        }) {
            return supervisor.startup_failure(&error);
        }
        let initialized = supervisor.receive_until(
            Duration::from_millis(supervisor.policy.handshake_timeout_ms),
            false,
        )?;
        match initialized {
            WorkerMessage::Initialized {
                request_id: received_request,
                bundle_identity,
                world,
                ..
            } if received_request == request_id
                && bundle_identity == supervisor.attempt.bundle().identity
                && world == supervisor.attempt.bundle().input.strategy.world =>
            {
                supervisor.last_heartbeat = Instant::now();
                supervisor.initialized = true;
                Ok(supervisor)
            }
            _ => supervisor.startup_failure("worker-initialization-failed"),
        }
    }

    pub fn state(&self) -> LifecycleState {
        self.attempt.state()
    }

    pub fn bundle(&self) -> &DeploymentBundle {
        self.attempt.bundle()
    }

    pub fn diagnostics(&self) -> &[String] {
        &self.diagnostics
    }

    pub fn take_health_events(&mut self) -> Vec<WorkerHealthEvent> {
        std::mem::take(&mut self.health_events)
    }

    pub fn poll_health(&mut self) -> Vec<WorkerHealthEvent> {
        if self.terminated {
            return self.take_health_events();
        }
        let max_frame_bytes = match self.policy.max_frame_bytes_usize() {
            Ok(value) => value,
            Err(_) => {
                self.terminate_for_fault("worker-policy-invalid");
                return self.take_health_events();
            }
        };

        loop {
            let event = match self.messages.try_recv() {
                Ok(event) => event,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.terminate_for_fault("worker-reader-lost");
                    break;
                }
            };
            let message = match event {
                ReaderEvent::Frame(frame) => match decode_frame(&frame, max_frame_bytes) {
                    Ok(message) => message,
                    Err(error) => {
                        self.terminate_for_fault(&error.to_string());
                        break;
                    }
                },
                ReaderEvent::Eof => {
                    self.terminate_for_fault("worker-exited");
                    break;
                }
                ReaderEvent::Error(error) => {
                    self.terminate_for_fault(&error.to_string());
                    break;
                }
            };
            if let Err(error) = self.inbound.accept(message.sequence()) {
                self.terminate_for_fault(&error.to_string());
                break;
            }
            match message {
                WorkerMessage::Heartbeat {
                    observed_at_ms,
                    state,
                    ..
                } => {
                    self.last_heartbeat = Instant::now();
                    self.health_events.push(WorkerHealthEvent::Heartbeat {
                        observed_at_ms,
                        state,
                    });
                }
                WorkerMessage::Diagnostic { code, detail, .. } => {
                    let code = bound_text(&code, self.policy.max_diagnostic_bytes as usize);
                    let detail = bound_text(&detail, self.policy.max_diagnostic_bytes as usize);
                    self.diagnostics.push(detail.clone());
                    if self.diagnostics.len() > 32 {
                        self.diagnostics.remove(0);
                    }
                    self.health_events
                        .push(WorkerHealthEvent::Diagnostic { code, detail });
                }
                WorkerMessage::Fault { code, detail, .. } => {
                    self.health_events.push(WorkerHealthEvent::Fault {
                        code: bound_text(&code, self.policy.max_diagnostic_bytes as usize),
                        detail: bound_text(&detail, self.policy.max_diagnostic_bytes as usize),
                    });
                    self.terminate_for_fault("worker-fault");
                    break;
                }
                _ => {
                    self.terminate_for_fault("unexpected-idle-message");
                    break;
                }
            }
        }

        if !self.terminated {
            match self.child.try_wait() {
                Ok(Some(_)) => {
                    self.terminate_for_fault("worker-exited");
                }
                Ok(None) => {}
                Err(_) => {
                    self.terminate_for_fault("worker-process-state-failed");
                }
            }
        }
        if !self.terminated
            && self.initialized
            && Instant::now().duration_since(self.last_heartbeat)
                > Duration::from_millis(self.policy.heartbeat_timeout_ms)
        {
            self.terminate_for_fault("worker-heartbeat-missed");
        }
        self.take_health_events()
    }

    pub fn transition(
        &mut self,
        to: LifecycleState,
        actor: impl Into<String>,
        reason: impl Into<String>,
    ) -> Result<(), String> {
        self.attempt
            .transition(to, actor, reason)
            .map_err(|error| error.to_string())
    }

    pub fn decision(
        &mut self,
        request_id: String,
        clock: DecisionClock,
        input: WorkerDecisionInput,
    ) -> Result<WorkerDecisionResult, String> {
        if !self.initialized {
            return Err("worker-not-initialized".into());
        }
        if self.attempt.state() != LifecycleState::Running {
            return Err(RuntimeError::NotRunning.to_string());
        }
        if request_id.trim().is_empty() || !self.seen_requests.insert(request_id.clone()) {
            return self.fail("duplicate-request");
        }
        if let Err(error) = clock.validate() {
            return self.fail(&error.to_string());
        }
        let now = unix_now_ms();
        if now > clock.deadline_ms() {
            return self.fail("decision-deadline-missed");
        }
        let target_input = input.clone();
        let sequence = self.outbound.next();
        if let Err(error) = self.send(WorkerMessage::Decision {
            sequence,
            request_id: request_id.clone(),
            clock: clock.clone(),
            input,
        }) {
            return self.fail(&error);
        }
        let timeout = self
            .policy
            .decision_timeout_ms
            .min((clock.deadline_ms() - now).try_into().unwrap_or(0));
        if timeout == 0 {
            return self.fail("decision-deadline-missed");
        }
        let response = self.receive_until(Duration::from_millis(timeout), true)?;
        match response {
            WorkerMessage::Target {
                request_id: received_request,
                decision_id,
                produced_at_ms,
                target,
                evaluation,
                ..
            } if received_request == request_id => {
                if let Err(error) =
                    target.validate_for(&self.attempt.bundle().input.strategy.world, &target_input)
                {
                    return self.fail(&error.to_string());
                }
                if let Err(error) = evaluation.validate() {
                    return self.fail(&error);
                }
                if let Err(error) =
                    self.attempt
                        .authorize_target(&clock, &decision_id, produced_at_ms)
                {
                    return self.fail(&error.to_string());
                }
                Ok(WorkerDecisionResult::Target {
                    request_id,
                    decision_id,
                    produced_at_ms,
                    target,
                    evaluation,
                })
            }
            WorkerMessage::NoTarget {
                request_id: received_request,
                decision_id,
                reason,
                detail,
                ..
            } if received_request == request_id && decision_id == clock.decision_id() => {
                if reason == NoTargetReason::DeadlineMissed {
                    self.terminate_for_fault("decision-deadline-missed");
                }
                Ok(WorkerDecisionResult::NoTarget {
                    request_id,
                    decision_id,
                    reason,
                    detail: bound_text(&detail, self.policy.max_diagnostic_bytes as usize),
                })
            }
            WorkerMessage::Fault { code, detail, .. } => self.fail_with_detail(&code, &detail),
            _ => self.fail("worker-response-mismatch"),
        }
    }

    pub fn shutdown(&mut self, request_id: impl Into<String>) -> Result<(), String> {
        if self.terminated {
            return Ok(());
        }
        let request_id = request_id.into();
        if self.initialized {
            let sequence = self.outbound.next();
            let _ = self.send(WorkerMessage::Shutdown {
                sequence,
                request_id: request_id.clone(),
            });
            let _ = self.receive_until(
                Duration::from_millis(self.policy.handshake_timeout_ms),
                false,
            );
        }
        self.terminate_process();
        if matches!(
            self.attempt.state(),
            LifecycleState::Running | LifecycleState::Paused | LifecycleState::Stopping
        ) {
            if self.attempt.state() != LifecycleState::Stopping {
                self.attempt
                    .transition(LifecycleState::Stopping, "host", "worker_shutdown")
                    .map_err(|error| error.to_string())?;
            }
            self.attempt
                .transition(LifecycleState::Stopped, "host", "worker_shutdown")
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    pub fn terminate_for_fault(&mut self, code: &str) -> String {
        let was_terminated = self.terminated;
        self.terminate_process();
        if !was_terminated {
            if !matches!(
                self.attempt.state(),
                LifecycleState::Faulted | LifecycleState::Stopped
            ) {
                let _ =
                    self.attempt
                        .transition(LifecycleState::Faulted, "host", bound_text(code, 128));
            }
            let detail = bound_text(code, self.policy.max_diagnostic_bytes as usize);
            self.health_events.push(WorkerHealthEvent::Fault {
                code: detail.clone(),
                detail,
            });
        }
        code.to_owned()
    }

    fn send(&mut self, message: WorkerMessage) -> Result<(), String> {
        let frame = encode_frame(
            &message,
            self.policy
                .max_frame_bytes_usize()
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        self.stdin
            .write_all(&frame)
            .and_then(|_| self.stdin.flush())
            .map_err(|_| "worker-ipc-write-failed".to_owned())
    }

    fn receive_until(
        &mut self,
        timeout: Duration,
        require_heartbeat: bool,
    ) -> Result<WorkerMessage, String> {
        let deadline = Instant::now() + timeout;
        loop {
            if require_heartbeat
                && Instant::now().duration_since(self.last_heartbeat)
                    > Duration::from_millis(self.policy.heartbeat_timeout_ms)
            {
                return self.fail("worker-heartbeat-missed");
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return self.fail(if require_heartbeat {
                    "worker-deadline-missed"
                } else {
                    "worker-handshake-timeout"
                });
            }
            let event = match self.messages.recv_timeout(remaining) {
                Ok(event) => event,
                Err(RecvTimeoutError::Timeout) => {
                    return self.fail(if require_heartbeat {
                        "worker-deadline-missed"
                    } else {
                        "worker-handshake-timeout"
                    });
                }
                Err(RecvTimeoutError::Disconnected) => return self.fail("worker-reader-lost"),
            };
            let message = match event {
                ReaderEvent::Frame(frame) => match decode_frame(
                    &frame,
                    self.policy
                        .max_frame_bytes_usize()
                        .map_err(|error| error.to_string())?,
                ) {
                    Ok(message) => message,
                    Err(error) => return self.fail(&error.to_string()),
                },
                ReaderEvent::Eof => return self.fail("worker-exited"),
                ReaderEvent::Error(error) => return self.fail(&error.to_string()),
            };
            if let Err(error) = self.inbound.accept(message.sequence()) {
                return self.fail(&error.to_string());
            }
            match message {
                WorkerMessage::Heartbeat {
                    observed_at_ms,
                    state,
                    ..
                } => {
                    self.last_heartbeat = Instant::now();
                    self.health_events.push(WorkerHealthEvent::Heartbeat {
                        observed_at_ms,
                        state,
                    });
                }
                WorkerMessage::Diagnostic { code, detail, .. } => {
                    let code = bound_text(&code, self.policy.max_diagnostic_bytes as usize);
                    let detail = bound_text(&detail, self.policy.max_diagnostic_bytes as usize);
                    self.diagnostics.push(detail.clone());
                    if self.diagnostics.len() > 32 {
                        self.diagnostics.remove(0);
                    }
                    self.health_events
                        .push(WorkerHealthEvent::Diagnostic { code, detail });
                }
                message => return Ok(message),
            }
        }
    }

    fn startup_failure<T>(&mut self, code: &str) -> Result<T, String> {
        self.terminate_for_fault(code);
        Err(code.into())
    }

    fn fail<T>(&mut self, code: &str) -> Result<T, String> {
        self.terminate_for_fault(code);
        Err(code.into())
    }

    fn fail_with_detail<T>(&mut self, code: &str, detail: &str) -> Result<T, String> {
        self.terminate_for_fault(code);
        Err(format!(
            "{code}:{}",
            bound_text(detail, self.policy.max_diagnostic_bytes as usize)
        ))
    }

    fn terminate_process(&mut self) {
        if self.terminated {
            return;
        }
        #[cfg(unix)]
        if let Ok(pid) = i32::try_from(self.child.id()) {
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.terminated = true;
    }
}

impl Drop for WorkerSupervisor {
    fn drop(&mut self) {
        self.terminate_process();
    }
}

fn spawn_stdout_reader(
    stdout: impl Read + Send + 'static,
    max_frame_bytes: usize,
) -> Receiver<ReaderEvent> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut stdout = stdout;
        loop {
            match read_bounded_line(&mut stdout, max_frame_bytes) {
                Ok(Some(frame)) => {
                    if sender.send(ReaderEvent::Frame(frame)).is_err() {
                        break;
                    }
                }
                Ok(None) => {
                    let _ = sender.send(ReaderEvent::Eof);
                    break;
                }
                Err(error) => {
                    let _ = sender.send(ReaderEvent::Error(error));
                    break;
                }
            }
        }
    });
    receiver
}

fn spawn_stderr_drain(stderr: impl Read + Send + 'static) {
    thread::spawn(move || {
        let mut stderr = stderr;
        let mut buffer = [0u8; 1024];
        loop {
            match stderr.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
    });
}

fn bound_text(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
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

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_PRIVATE_KEY: [u8; 32] = [
        0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4f, 0xa4, 0x92, 0xec, 0x2c,
        0xc4, 0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03, 0x1c, 0xae,
        0x7f, 0x60,
    ];

    fn worker() -> WorkerArtifactBinding {
        let signature =
            WorkerArtifactSignature::sign(b"worker", current_platform_tag(), &TEST_PRIVATE_KEY)
                .unwrap();
        WorkerArtifactBinding {
            artifact_name: WORKER_ARTIFACT_NAME.into(),
            artifact_version: WORKER_ARTIFACT_VERSION.into(),
            platform: signature.platform,
            protocol_version: WORKER_PROTOCOL_VERSION.into(),
            runtime_version: WORKER_RUNTIME_VERSION.into(),
            sha256: signature.artifact_sha256,
            signing_key_id: WORKER_SIGNING_KEY_ID.into(),
            signature: signature.signature,
        }
    }

    fn bundle() -> DeploymentBundle {
        let worker = worker();
        DeploymentBundle::freeze(DeploymentBundleInput {
            bot_id: "bot".into(),
            strategy_id: "strategy".into(),
            account_id: "account".into(),
            component_hashes: vec!["c".repeat(64)],
            model_hashes: vec![],
            feature_plan_hash: "f".repeat(64),
            risk_policy_hash: "d".repeat(64),
            execution_profile_hash: "e".repeat(64),
            worker_binary_hash: worker.sha256.clone(),
            qualification_evidence_hash: "b".repeat(64),
            strategy: WorkerStrategyBinding {
                world: StrategyWorld::Strategy,
                component_sha256: "c".repeat(64),
                feature_slots: vec!["close".into()],
                parameters: vec![],
            },
            pipeline: WorkerPipelineBinding::default(),
            worker,
            worker_policy: WorkerRuntimePolicy::default(),
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
    fn scheduled_cross_section_requires_the_exact_universe() {
        let clock = DecisionClock::ScheduledCrossSection {
            decision_id: "batch-1".into(),
            decision_time_ms: 100,
            deadline_ms: 110,
            next_execution_ms: 111,
            universe: vec!["A".into(), "B".into()],
            available_instruments: vec!["A".into(), "B".into()],
        };
        assert!(clock.validate().is_ok());
        let mut invalid = clock.clone();
        if let DecisionClock::ScheduledCrossSection {
            available_instruments,
            ..
        } = &mut invalid
        {
            available_instruments[1] = "C".into();
        }
        assert_eq!(invalid.validate(), Err(RuntimeError::IncompleteUniverse));
    }

    #[test]
    fn ndjson_rejects_unknown_oversized_duplicate_and_out_of_order_frames() {
        let hello = WorkerMessage::Hello {
            sequence: 1,
            protocol_version: WORKER_PROTOCOL_VERSION.into(),
            artifact_name: WORKER_ARTIFACT_NAME.into(),
            artifact_version: WORKER_ARTIFACT_VERSION.into(),
            platform: current_platform_tag(),
            runtime_version: WORKER_RUNTIME_VERSION.into(),
            artifact_sha256: "a".repeat(64),
        };
        let frame = encode_frame(&hello, 4096).unwrap();
        let mut sequence = ProtocolSequence::default();
        sequence
            .accept(decode_frame(&frame, 4096).unwrap().sequence())
            .unwrap();
        assert_eq!(
            sequence.accept(1),
            Err(ProtocolError::Duplicate {
                expected: 2,
                received: 1
            })
        );
        assert_eq!(
            sequence.accept(4),
            Err(ProtocolError::OutOfOrder {
                expected: 2,
                received: 4
            })
        );
        assert_eq!(
            decode_frame(br#"{"type":"unknown","sequence":1}"#, 4096),
            Err(ProtocolError::UnknownMessage)
        );
        assert_eq!(
            decode_frame(&vec![b'x'; 4097], 4096),
            Err(ProtocolError::OversizedFrame)
        );
    }

    #[test]
    fn artifact_signature_binds_hash_and_worker_identity() {
        let bytes = b"worker";
        let signature =
            WorkerArtifactSignature::sign(bytes, current_platform_tag(), &TEST_PRIVATE_KEY)
                .unwrap();
        let expected = WorkerArtifactBinding {
            artifact_name: signature.artifact_name.clone(),
            artifact_version: signature.artifact_version.clone(),
            platform: signature.platform.clone(),
            protocol_version: signature.protocol_version.clone(),
            runtime_version: signature.runtime_version.clone(),
            sha256: signature.artifact_sha256.clone(),
            signing_key_id: signature.signing_key_id.clone(),
            signature: signature.signature.clone(),
        };
        let public_key = ed25519_dalek::SigningKey::from_bytes(&TEST_PRIVATE_KEY)
            .verifying_key()
            .to_bytes();
        WorkerArtifactVerifier::with_trust_root(WorkerTrustRoot {
            key_id: WORKER_SIGNING_KEY_ID.into(),
            public_key,
        })
        .verify_bytes(bytes, &signature, &expected)
        .unwrap();
        assert!(
            WorkerArtifactVerifier::default()
                .verify_bytes(bytes, &signature, &expected)
                .is_err()
        );
    }

    #[test]
    fn target_validation_rejects_partial_portfolio_output() {
        let input = WorkerDecisionInput::Portfolio {
            universe_id: "u".into(),
            rows: vec![
                WorkerFeatureRow {
                    instrument_id: "A".into(),
                    available_at_ms: 1,
                    values: vec![Some(1.0)],
                },
                WorkerFeatureRow {
                    instrument_id: "B".into(),
                    available_at_ms: 1,
                    values: vec![Some(1.0)],
                },
            ],
            state: WorkerPortfolioState {
                cash: "100".into(),
                positions: vec![],
            },
        };
        let target = WorkerTarget::Portfolio {
            universe_id: "u".into(),
            weights: vec![WorkerTargetWeight {
                instrument_id: "A".into(),
                weight: "1".into(),
            }],
            cash_reserve: "0".into(),
        };
        assert_eq!(
            target.validate_for(&StrategyWorld::PortfolioStrategy, &input),
            Err(RuntimeError::InvalidTarget)
        );
    }

    #[test]
    fn evaluation_evidence_validation_rejects_duplicate_or_non_decimal_outputs() {
        let evidence = WorkerEvaluationEvidence {
            rows: vec![WorkerEvaluationRow {
                instrument_id: "BTC-USDT".into(),
                observation_time_ms: 2,
                available_at_ms: 1,
                factor_outputs: vec![WorkerEvaluationValue {
                    name: "score".into(),
                    value: "1.5".into(),
                }],
                model_outputs: vec![WorkerEvaluationValue {
                    name: "score".into(),
                    value: "not-a-decimal".into(),
                }],
            }],
        };
        assert!(evidence.validate().is_err());
    }
}
