//! Tauri control-plane bindings for the source-visible Python Research boundary.
//!
//! Heavy work stays in Tauri-independent contracts or `spawn_blocking`; these
//! commands only bind those contracts to User-scoped app state and UI actions.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use adaq_component_tooling::{
    BuiltinForecastTarget, ComponentKind, ComponentPackage, ComponentParameterValue,
    ForecastTarget, ForecastValueScale, MODEL_EXPORTER_ID, MODEL_HORIZON_BARS, MODEL_OUTPUT_NAME,
    MODEL_TARGET_ID, ModelScope, PredictionKind, QualificationAttempt, QualificationGate,
    RunLimits, WASI_MODEL_PROFILE, WasmLoader, export_linear_model_component, linear_model_binding,
    qualify_package_with_limits,
};
use adaq_factor_research::{
    CorporateActionEvidence, EconomicAssumptions, EvaluationWindow, FactorCandidateDraft,
    FactorCandidateSource, FactorDataset, FactorDatasetManifest, FactorDatasetRow,
    FactorEvaluationProtocol, FactorEvaluationProtocolDraft, FactorFeatureSlot, FactorLens,
    FactorMarketContext, FactorMarketSeries, FactorObservationValue, FactorOrientation,
    FactorOutput, FactorParameter, FactorParameterType, FactorParameterValue,
    FactorPresentationMetadata, FactorPromotionDecision, FactorScope, FactorTarget,
    GridSearchFamilyDraft, GridSearchParameter, GridSearchPlan, PromotionDecisionDraft,
    PromotionDecisionState, PromotionPolicy, PromotionProtocol, PromotionProtocolDraft,
    PythonFactorBinding, PythonFactorMode, PythonFactorResourcePolicy, PythonRepeatabilityReport,
    ResearchEngineProvenance, ResearchRegistry, ResearchTrial, ResearchTrialStatus,
};
use adaq_feature_engine::{
    DefinitionDraft, FeatureDefinition, FeatureEngine, FeatureEngineIdentity,
    FeatureEvaluationInput, FeatureInput, FeatureInputEvent, FeatureMarketBar,
    FeatureMarketContext, FeatureMaterializationRequest, FeatureNode, FeatureObservationValue,
    FeatureOperator, FeatureOutput, FeaturePlan, FeaturePlanDraft, FeatureScope,
    FeatureUnavailabilityReason, MaterializationAttemptStatus, ObservationRange,
    PointInTimeInstrumentUniverse, UniverseEvidenceState,
};
use adaq_python_research::{
    HostResourcePolicy, PUBLIC_SDK_ARTIFACT_SHA256, ParameterType, ProjectKind, ProjectManifest,
    ProjectMode, ProjectRevision, ProjectStore, PythonResearchError, PythonResearchResetReport,
    ValidationReport, WorkingCopySummary,
    factor::{
        FactorUnavailableReason, MomentumOutputRow, PythonFactorBatch, PythonFactorInput,
        PythonFactorSegment, RepeatabilityReport, expand_momentum_grid, materialize_momentum,
        validate_imperative_factor_payload, validate_portable_definition_payload,
    },
    fixture::{SyntheticTutorialFixture, TUTORIAL_SESSION_COUNT},
    inspect_project,
    model::{
        DatasetH, FittedTransformation, HostPartition, HostPartitionRow, LinearModelArtifact,
        MODEL_PROJECT_ID, ModelRunnerInput, PartitionName, RidgeAdapter, TARGET_HORIZON_BARS,
        TutorialWindows, forecast, future_close_return_state, validate_model_project_payload,
        validate_model_runner_payload,
    },
    runner::{
        AttemptExecution, AttemptStatus as PythonAttemptStatus, AttemptStore, AttemptTransition,
        Handshake, PrivateChildEnvironment, ResearchAttempt, RunnerExecution, RunnerLaunchSpec,
        StagedArtifact, TrustStore, read_staged_artifact, run_process,
    },
    runtime::{
        DependencyIntent, EnvironmentLock, EnvironmentRecord, EnvironmentStore, PreparationAttempt,
        RuntimeArtifactManifest, RuntimePlatform, RuntimeRecord, RuntimeStore,
        WheelhouseCatalogEntry, WheelhouseManifest, WheelhouseRecord, WheelhouseStore,
        embedded_wheel_payload, parse_environment_lock, runtime_catalog_entry, sync_environment,
        wheelhouse_catalog,
    },
    sha256,
    tuning::{
        EvidenceState, FinalEvaluationLedger, FinalEvaluationReport, ModelExperiment, ModelTrial,
        ParameterSelectionDecision, RIDGE_REPEATABILITY_TOLERANCE, RepeatabilityState, TrialStatus,
        compare_repeatability,
    },
};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use tauri::{Manager, State, WebviewWindow};

use crate::auth::AuthState;
use crate::factor_research::{
    FactorDatasetInput, FactorDecisionSaveRequest, FactorEvaluationStartRequest,
    FactorGridFamilyRegisterRequest, FactorPolicySaveRequest, FactorTrialUpdateRequest,
    PythonHostAttemptEvidence, PythonHostEvidence, factor_trial_statistics,
};
use crate::{
    features::{FeatureAttemptRequest, FeatureMaterializationStartRequest},
    research_queue::{
        QueueAdmission, QueueAdmitter, QueueRunResult, QueueTicket, QueueWaker, ResearchQueue,
        ResearchQueueAdapter, WorkKind,
    },
};
use adaq_data_core::{
    BarInterval, BarSeries, OhlcvBar,
    market::{PriceBasis, Venue, VenueKind},
};
use rust_decimal::Decimal;

static NEXT_MODEL_VERIFICATION_SEQUENCE: AtomicU64 = AtomicU64::new(1);

pub struct PythonResearchState {
    pub(crate) store: Arc<ProjectStore>,
    pub(crate) attempt_store: Arc<AttemptStore>,
    pub(crate) trust_store: Arc<TrustStore>,
    pub(crate) model_lab_store: Arc<ModelLabStore>,
    pub(crate) runtime_store: Arc<RuntimeStore>,
    pub(crate) wheelhouse_store: Arc<WheelhouseStore>,
    // ponytail: one device-wide gate serializes managed downloads; use per-artifact gates if throughput matters.
    managed_runtime_gate: Arc<Mutex<()>>,
    pub(crate) environment_store: Arc<EnvironmentStore>,
    root: PathBuf,
    examples_root: PathBuf,
    queue_admitter: Mutex<Option<QueueAdmitter>>,
    queue_waker: Mutex<Option<QueueWaker>>,
    resetting_users: Mutex<BTreeSet<String>>,
    completed_results: Arc<Mutex<BTreeMap<String, RunnerExecution>>>,
    runtime_cancellations: Mutex<BTreeMap<String, Arc<AtomicBool>>>,
    runtime_progress: Arc<Mutex<BTreeMap<String, RuntimePreparationProgress>>>,
    shutdown: AtomicBool,
}

#[derive(Debug, Clone, Default)]
struct RuntimePreparationProgress {
    completed_bytes: u64,
    total_bytes: u64,
}

const MODEL_LAB_SCHEMA_VERSION: u32 = 1;
const MODEL_RUNTIME_IDENTITY: &str = "wasmtime@47.0.3:component-model:fuel:10m:memory:67108864";
const UNKNOWN_MODEL_IDENTITY: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";

fn model_lab_schema_version() -> u32 {
    MODEL_LAB_SCHEMA_VERSION
}

fn model_lab_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModelFinalEvaluationStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
    Stale,
    PersistenceFailed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelFinalEvaluationState {
    pub decision_id: String,
    pub status: ModelFinalEvaluationStatus,
    #[serde(default)]
    pub attempt_id: Option<String>,
    #[serde(default)]
    pub staged_dataset_sha256: Option<String>,
    #[serde(default)]
    pub report_id: Option<String>,
    #[serde(default)]
    pub failure_code: Option<String>,
    #[serde(default)]
    pub diagnostic: Option<String>,
    #[serde(default)]
    pub created_at_ms: i64,
    #[serde(default)]
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelQualificationEvidence {
    pub package: bool,
    pub conformance: bool,
    pub equivalence: bool,
    pub runtime: bool,
    pub qualified: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelRuntimeQualificationReport {
    pub report_id: String,
    pub attempt_id: String,
    pub decision_id: String,
    pub final_evaluation_report_id: String,
    pub artifact_sha256: String,
    pub transformation_sha256: String,
    pub wasi_profile: String,
    pub exporter_id: String,
    pub sdk_version: String,
    pub abi_version: String,
    #[serde(default)]
    pub package_archive_sha256: Option<String>,
    #[serde(default)]
    pub component_id: Option<String>,
    #[serde(default)]
    pub component_version: Option<String>,
    #[serde(default)]
    pub wasm_sha256: Option<String>,
    pub runtime_identity: String,
    pub resource_policy_sha256: String,
    pub qualification_deadline_ms: u64,
    pub qualification_duration_ms: u64,
    pub input_slots: Vec<String>,
    pub target_id: String,
    pub target_horizon_bars: u32,
    pub forecast_contract: String,
    pub replay_identity: String,
    pub replay_rows: usize,
    pub numeric_tolerance: f64,
    pub evidence: ModelQualificationEvidence,
    pub qualified: bool,
    #[serde(default)]
    pub evidence_windows_complete: bool,
    #[serde(default)]
    pub imported_component_archive_sha256: Option<String>,
    pub diagnostics: Vec<String>,
    pub created_at_ms: i64,
}

impl ModelRuntimeQualificationReport {
    fn validate(&self) -> Result<(), PythonResearchError> {
        if self.attempt_id.trim().is_empty()
            || self.decision_id.trim().is_empty()
            || self.qualified && self.final_evaluation_report_id.trim().is_empty()
            || !is_sha256_text(&self.artifact_sha256)
            || !is_sha256_text(&self.transformation_sha256)
            || self.wasi_profile != WASI_MODEL_PROFILE
            || self.exporter_id != MODEL_EXPORTER_ID
            || self.sdk_version != adaq_component_sdk::SDK_VERSION
            || self.abi_version != adaq_component_sdk::ABI_VERSION
            || self.runtime_identity != MODEL_RUNTIME_IDENTITY
            || !is_sha256_text(&self.resource_policy_sha256)
            || self.qualified && self.qualification_deadline_ms == 0
            || self.evidence.runtime
                && self.qualification_duration_ms > self.qualification_deadline_ms
            || self.qualified && self.input_slots.is_empty()
            || self.input_slots.len() > 64
            || self
                .input_slots
                .iter()
                .any(|slot| !is_lower_kebab_text(slot))
            || self.input_slots.iter().collect::<BTreeSet<_>>().len() != self.input_slots.len()
            || self.target_id != MODEL_TARGET_ID
            || self.target_horizon_bars != MODEL_HORIZON_BARS
            || self.forecast_contract != adaq_python_research::model::FORECAST_CONTRACT
            || !is_sha256_text(&self.replay_identity)
            || self.qualified && self.replay_rows == 0
            || !self.numeric_tolerance.is_finite()
            || self.numeric_tolerance <= 0.0
            || self.evidence.qualified != self.qualified
            || self.qualified
                && !(self.evidence.package
                    && self.evidence.conformance
                    && self.evidence.equivalence
                    && self.evidence.runtime)
            || !self.qualified && self.evidence.qualified
            || self
                .package_archive_sha256
                .as_deref()
                .is_some_and(|hash| !is_sha256_text(hash))
            || self
                .wasm_sha256
                .as_deref()
                .is_some_and(|hash| !is_sha256_text(hash))
            || self
                .component_id
                .as_deref()
                .is_some_and(|id| uuid::Uuid::parse_str(id).is_err())
            || self.qualified
                && (self.package_archive_sha256.is_none()
                    || self.component_id.is_none()
                    || self.component_version.is_none()
                    || self.imported_component_archive_sha256 != self.package_archive_sha256)
            || !self.qualified && self.imported_component_archive_sha256.is_some()
            || !self.qualified && self.diagnostics.is_empty()
        {
            return Err(PythonResearchError(
                "model-runtime-qualification-report-invalid".into(),
            ));
        }
        let expected_id = model_qualification_report_id(
            &self.attempt_id,
            &self.decision_id,
            &self.artifact_sha256,
            self.package_archive_sha256.as_deref().unwrap_or_default(),
            &self.replay_identity,
            self.qualified,
        );
        if self.report_id != expected_id {
            return Err(PythonResearchError(
                "model-runtime-qualification-report-id-invalid".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AcceptedModelInput {
    pub qualification_report_id: String,
    pub decision_id: String,
    pub final_evaluation_report_id: String,
    pub artifact_sha256: String,
    pub transformation_sha256: String,
    pub package_archive_sha256: String,
    pub package_wasm_sha256: String,
    pub component_id: String,
    pub component_version: String,
    pub model_profile: String,
    pub exporter_id: String,
    pub sdk_version: String,
    pub abi_version: String,
    pub runtime_identity: String,
    pub input_slots: Vec<String>,
    pub output_name: String,
    pub target_id: String,
    pub target_horizon_bars: u32,
    pub forecast_contract: String,
    pub input_evidence_sha256: String,
}

fn model_qualification_report_id(
    attempt_id: &str,
    decision_id: &str,
    artifact_sha256: &str,
    package_archive_sha256: &str,
    replay_identity: &str,
    qualified: bool,
) -> String {
    sha256(
        format!(
            "{attempt_id}:{decision_id}:{artifact_sha256}:{package_archive_sha256}:{replay_identity}:{qualified}"
        )
        .as_bytes(),
    )
}

fn same_qualified_model_report(
    existing: &ModelRuntimeQualificationReport,
    incoming: &ModelRuntimeQualificationReport,
) -> bool {
    existing.decision_id == incoming.decision_id
        && existing.qualified
        && existing.package_archive_sha256 == incoming.package_archive_sha256
        && existing.evidence_windows_complete == incoming.evidence_windows_complete
}

fn is_lower_kebab_text(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !value.starts_with('-')
        && !value.ends_with('-')
        && !value.contains("--")
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelLabState {
    pub experiments: Vec<ModelExperiment>,
    pub experiment: Option<ModelExperiment>,
    pub decision: Option<ParameterSelectionDecision>,
    pub report: Option<FinalEvaluationReport>,
    pub final_evaluation: Option<ModelFinalEvaluationState>,
    pub deployment_reports: Vec<ModelRuntimeQualificationReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelLabDatabase {
    #[serde(default = "model_lab_schema_version")]
    schema_version: u32,
    experiments: BTreeMap<String, ModelExperiment>,
    decisions: BTreeMap<String, ParameterSelectionDecision>,
    final_reports: BTreeMap<String, FinalEvaluationReport>,
    #[serde(default)]
    final_evaluations: BTreeMap<String, ModelFinalEvaluationState>,
    #[serde(default)]
    runs: BTreeMap<String, ModelRunView>,
    #[serde(default)]
    artifacts: BTreeMap<String, Vec<u8>>,
    #[serde(default)]
    transformations: BTreeMap<String, Vec<u8>>,
    #[serde(default)]
    forecast_datasets: BTreeMap<String, StoredForecastDataset>,
    #[serde(default)]
    model_qualification_reports: BTreeMap<String, ModelRuntimeQualificationReport>,
}

impl Default for ModelLabDatabase {
    fn default() -> Self {
        Self {
            schema_version: MODEL_LAB_SCHEMA_VERSION,
            experiments: BTreeMap::new(),
            decisions: BTreeMap::new(),
            final_reports: BTreeMap::new(),
            final_evaluations: BTreeMap::new(),
            runs: BTreeMap::new(),
            artifacts: BTreeMap::new(),
            transformations: BTreeMap::new(),
            forecast_datasets: BTreeMap::new(),
            model_qualification_reports: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredForecastDataset {
    schema: String,
    dataset_sha256: String,
    producer_id: String,
    producer_adapter_id: String,
    producer_artifact_sha256: String,
    input_evidence_sha256: String,
    signal_id: String,
    target_id: String,
    horizon_bars: u32,
    forecast_contract: String,
    provenance_hashes: BTreeMap<String, String>,
    snapshot_id: String,
    universe_id: String,
    rows: Vec<adaq_python_research::model::ForecastRow>,
}

#[derive(Clone)]
pub struct ModelLabStore {
    path: PathBuf,
    database: Arc<Mutex<ModelLabDatabase>>,
}

impl ModelLabStore {
    fn open(path: impl Into<PathBuf>) -> Result<Self, PythonResearchError> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut database = if path.is_file() {
            serde_json::from_slice(&fs::read(&path)?)
                .map_err(|error| PythonResearchError(format!("model-lab-store-invalid:{error}")))?
        } else {
            ModelLabDatabase::default()
        };
        if database.schema_version != MODEL_LAB_SCHEMA_VERSION {
            return Err(PythonResearchError(
                "model-lab-store-schema-incompatible:reset-required".into(),
            ));
        }
        for experiment in database.experiments.values() {
            experiment
                .validate()
                .map_err(|error| PythonResearchError(format!("model-lab-store-invalid:{error}")))?;
        }
        for (key, experiment) in &database.experiments {
            let user_id = key
                .split_once(':')
                .map(|(user_id, _)| user_id)
                .ok_or_else(|| {
                    PythonResearchError("model-lab-store-invalid:model-experiment-key".into())
                })?;
            for trial in &experiment.trials {
                if trial.candidate_artifact_sha256.is_some() {
                    validate_model_trial_candidate(&database, user_id, trial).map_err(|error| {
                        PythonResearchError(format!("model-lab-store-invalid:{error}"))
                    })?;
                }
            }
        }
        for (key, decision) in &database.decisions {
            decision
                .validate()
                .map_err(|error| PythonResearchError(format!("model-lab-store-invalid:{error}")))?;
            let user_id = key
                .split_once(':')
                .map(|(user_id, _)| user_id)
                .ok_or_else(|| {
                    PythonResearchError("model-lab-store-invalid:model-decision-key".into())
                })?;
            let experiment = database
                .experiments
                .get(&model_key(user_id, &decision.experiment_id))
                .ok_or_else(|| {
                    PythonResearchError("model-lab-store-invalid:model-decision-experiment".into())
                })?;
            let selected_trial = experiment
                .trials
                .iter()
                .find(|trial| trial.trial_id == decision.selected_trial_id)
                .ok_or_else(|| {
                    PythonResearchError("model-lab-store-invalid:model-decision-trial".into())
                })?;
            if decision.binding_sha256 != experiment.binding_sha256
                || decision.project_revision_sha256 != experiment.project_revision_sha256
                || decision.environment_sha256 != experiment.environment_sha256
                || decision.input_evidence_sha256 != experiment.input_evidence_sha256
                || decision.seed != experiment.seed
                || decision.evidence_state != experiment.lineage_evidence_state
                || decision.selected_alpha.to_bits() != selected_trial.alpha.to_bits()
                || decision.candidate_artifact_sha256
                    != selected_trial
                        .candidate_artifact_sha256
                        .as_deref()
                        .unwrap_or_default()
            {
                return Err(PythonResearchError(
                    "model-lab-store-invalid:model-decision-binding".into(),
                ));
            }
        }
        for (key, report) in &database.final_reports {
            report
                .validate()
                .map_err(|error| PythonResearchError(format!("model-lab-store-invalid:{error}")))?;
            let user_id = key
                .split_once(':')
                .map(|(user_id, _)| user_id)
                .ok_or_else(|| {
                    PythonResearchError("model-lab-store-invalid:model-report-key".into())
                })?;
            if key != &model_key(user_id, &report.report_id) {
                return Err(PythonResearchError(
                    "model-lab-store-invalid:model-report-key-binding".into(),
                ));
            }
            let decision = database
                .decisions
                .get(&model_key(user_id, &report.decision_id))
                .ok_or_else(|| {
                    PythonResearchError("model-lab-store-invalid:model-report-decision".into())
                })?;
            if report.artifact_sha256 != decision.candidate_artifact_sha256 {
                return Err(PythonResearchError(
                    "model-lab-store-invalid:model-report-binding".into(),
                ));
            }
        }
        for (key, report) in &database.model_qualification_reports {
            report
                .validate()
                .map_err(|error| PythonResearchError(format!("model-lab-store-invalid:{error}")))?;
            let user_id = key
                .split_once(':')
                .map(|(user_id, _)| user_id)
                .ok_or_else(|| {
                    PythonResearchError("model-lab-store-invalid:model-qualification-key".into())
                })?;
            crate::user::validate_user(user_id).map_err(|_| {
                PythonResearchError("model-lab-store-invalid:model-qualification-user".into())
            })?;
            if key != &model_key(user_id, &report.report_id) {
                return Err(PythonResearchError(
                    "model-lab-store-invalid:model-qualification-key-binding".into(),
                ));
            }
            let decision = database
                .decisions
                .get(&model_key(user_id, &report.decision_id));
            if report.qualified && decision.is_none() {
                return Err(PythonResearchError(
                    "model-lab-store-invalid:model-qualification-decision".into(),
                ));
            }
            if let Some(decision) = decision {
                if report.artifact_sha256 != UNKNOWN_MODEL_IDENTITY
                    && report.artifact_sha256 != decision.candidate_artifact_sha256
                {
                    return Err(PythonResearchError(
                        "model-lab-store-invalid:model-qualification-binding".into(),
                    ));
                }
            }
            if !report.final_evaluation_report_id.is_empty() {
                let final_report = database
                    .final_reports
                    .get(&model_key(user_id, &report.final_evaluation_report_id))
                    .ok_or_else(|| {
                        PythonResearchError(
                            "model-lab-store-invalid:model-qualification-final-report".into(),
                        )
                    })?;
                if final_report.decision_id != report.decision_id
                    || report.artifact_sha256 != UNKNOWN_MODEL_IDENTITY
                        && final_report.artifact_sha256 != report.artifact_sha256
                {
                    return Err(PythonResearchError(
                        "model-lab-store-invalid:model-qualification-binding".into(),
                    ));
                }
            } else if report.qualified {
                return Err(PythonResearchError(
                    "model-lab-store-invalid:model-qualification-final-report".into(),
                ));
            }
        }
        for (key, state) in &database.final_evaluations {
            let user_id = key
                .split_once(':')
                .map(|(user_id, _)| user_id)
                .ok_or_else(|| {
                    PythonResearchError("model-lab-store-invalid:model-final-state-key".into())
                })?;
            if key != &model_key(user_id, &state.decision_id)
                || !database
                    .decisions
                    .contains_key(&model_key(user_id, &state.decision_id))
                || state.attempt_id.as_deref().is_some_and(str::is_empty)
                || state
                    .staged_dataset_sha256
                    .as_deref()
                    .is_some_and(|hash| !is_sha256_text(hash))
            {
                return Err(PythonResearchError(
                    "model-lab-store-invalid:model-final-state-binding".into(),
                ));
            }
            if state.status == ModelFinalEvaluationStatus::Completed {
                let report_id = state.report_id.as_deref().ok_or_else(|| {
                    PythonResearchError("model-lab-store-invalid:model-final-state-report".into())
                })?;
                let report = database
                    .final_reports
                    .get(&model_key(user_id, report_id))
                    .ok_or_else(|| {
                        PythonResearchError(
                            "model-lab-store-invalid:model-final-state-report".into(),
                        )
                    })?;
                if report.decision_id != state.decision_id
                    || state.staged_dataset_sha256.as_deref()
                        != Some(report.forecast_dataset_sha256.as_str())
                {
                    return Err(PythonResearchError(
                        "model-lab-store-invalid:model-final-state-report-binding".into(),
                    ));
                }
            } else if state.report_id.is_some() {
                return Err(PythonResearchError(
                    "model-lab-store-invalid:model-final-state-report".into(),
                ));
            }
        }
        let mut recovered_final_evaluations = false;
        for state in database.final_evaluations.values_mut() {
            if matches!(
                state.status,
                ModelFinalEvaluationStatus::Pending | ModelFinalEvaluationStatus::Running
            ) {
                state.status = ModelFinalEvaluationStatus::Interrupted;
                state.failure_code = Some("model-final-evaluation-interrupted".into());
                state.diagnostic = Some(
                    "Final Evaluation was not terminal when the application restarted.".into(),
                );
                state.updated_at_ms = model_lab_now_ms();
                recovered_final_evaluations = true;
            }
        }
        for run in database.runs.values() {
            if run.binding_sha256.is_empty() || model_binding_sha256(run)? != run.binding_sha256 {
                return Err(PythonResearchError(
                    "model-lab-store-invalid:model-evidence-identity-binding-invalid".into(),
                ));
            }
        }
        let store = Self {
            path,
            database: Arc::new(Mutex::new(database)),
        };
        if recovered_final_evaluations {
            let database = store
                .database
                .lock()
                .map_err(|_| PythonResearchError("model-lab-store-lock-poisoned".into()))?
                .clone();
            store.persist(&database)?;
        }
        Ok(store)
    }

    fn persist(&self, database: &ModelLabDatabase) -> Result<(), PythonResearchError> {
        if database.schema_version != MODEL_LAB_SCHEMA_VERSION {
            return Err(PythonResearchError(
                "model-lab-store-schema-incompatible:reset-required".into(),
            ));
        }
        let temporary = self.path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(database)
            .map_err(|error| PythonResearchError(error.to_string()))?;
        let mut file = fs::File::create(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        replace_model_lab_file(&temporary, &self.path)?;
        Ok(())
    }

    fn register(
        &self,
        user_id: &str,
        experiment: ModelExperiment,
    ) -> Result<ModelExperiment, PythonResearchError> {
        experiment.validate()?;
        let mut database = self
            .database
            .lock()
            .map_err(|_| PythonResearchError("model-lab-store-lock-poisoned".into()))?;
        let key = model_key(user_id, &experiment.experiment_id);
        if let Some(existing) = database.experiments.get(&key) {
            if existing != &experiment {
                return Err(PythonResearchError(
                    "model-experiment-identity-collision".into(),
                ));
            }
            return Ok(existing.clone());
        }
        database.experiments.insert(key, experiment.clone());
        self.persist(&database)?;
        Ok(experiment)
    }

    fn fail_trial(
        &self,
        user_id: &str,
        experiment_id: &str,
        trial_id: &str,
        attempt_id: String,
        status: TrialStatus,
        diagnostic: String,
    ) -> Result<ModelExperiment, PythonResearchError> {
        let mut database = self
            .database
            .lock()
            .map_err(|_| PythonResearchError("model-lab-store-lock-poisoned".into()))?;
        let mut next = database.clone();
        let experiment = next
            .experiments
            .get_mut(&model_key(user_id, experiment_id))
            .ok_or_else(|| PythonResearchError("model-experiment-not-found".into()))?;
        experiment.fail_trial_with_diagnostic(trial_id, attempt_id, status, diagnostic)?;
        experiment.validate()?;
        let result = experiment.clone();
        self.persist(&next)?;
        *database = next;
        Ok(result)
    }

    fn retry_trial_from_attempt(
        &self,
        user_id: &str,
        experiment_id: &str,
        trial_id: &str,
        source_attempt_id: &str,
        source_status: TrialStatus,
        diagnostic: String,
    ) -> Result<ModelExperiment, PythonResearchError> {
        let mut database = self
            .database
            .lock()
            .map_err(|_| PythonResearchError("model-lab-store-lock-poisoned".into()))?;
        let mut next = database.clone();
        let experiment = next
            .experiments
            .get_mut(&model_key(user_id, experiment_id))
            .ok_or_else(|| PythonResearchError("model-experiment-not-found".into()))?;
        let registered_without_source = experiment
            .trials
            .iter()
            .find(|trial| trial.trial_id == trial_id)
            .map(|trial| {
                trial.status == TrialStatus::Registered
                    && !trial.attempt_ids.iter().any(|id| id == source_attempt_id)
            })
            .ok_or_else(|| PythonResearchError("model-trial-not-found".into()))?;
        if registered_without_source {
            experiment.fail_trial_with_diagnostic(
                trial_id,
                source_attempt_id.to_owned(),
                source_status,
                diagnostic,
            )?;
        }
        experiment.retry_trial(trial_id, source_attempt_id)?;
        experiment.validate()?;
        let result = experiment.clone();
        self.persist(&next)?;
        *database = next;
        Ok(result)
    }

    fn save_demo_run(
        &self,
        user_id: &str,
        demo: &DemoModelRun,
        publish_forecast_dataset: bool,
    ) -> Result<ModelRunView, PythonResearchError> {
        let expected_provenance = model_run_provenance(&demo.view)?;
        if demo.view.binding_sha256.is_empty()
            || model_binding_sha256(&demo.view)? != demo.view.binding_sha256
            || (demo.view.repeatability_verified
                && demo.view.repeatability_state != RepeatabilityState::Verified)
            || (!demo.view.repeatability_verified
                && demo.view.repeatability_state == RepeatabilityState::Verified)
            || demo.artifact.artifact_sha256 != demo.view.artifact_sha256
            || demo.artifact.adapter_id != demo.view.adapter_id
            || demo.artifact.schema != demo.view.artifact_schema
            || demo.artifact.alpha.to_bits() != demo.view.alpha.to_bits()
            || demo.artifact.input_slots != demo.view.input_slots
            || demo.artifact.target_id != demo.view.target_id
            || demo.artifact.horizon_bars != demo.view.target_horizon_bars
            || demo.artifact.numeric_representation != demo.view.numeric_representation
            || demo.artifact.forecast_contract != demo.view.forecast_contract
            || demo.artifact.transformation_sha256 != demo.view.transformation_sha256
            || demo.artifact.provenance_hashes != expected_provenance
            || demo.transformation.transformation_sha256 != demo.view.transformation_sha256
        {
            return Err(PythonResearchError(
                "model-evidence-identity-binding-invalid".into(),
            ));
        }
        let artifact_bytes = demo.artifact.to_bytes()?;
        let transformation_bytes = demo
            .transformation
            .to_bytes()
            .map_err(|error| PythonResearchError(error.to_string()))?;
        let forecast_identity = (
            &demo.artifact.artifact_sha256,
            &demo.view.input_evidence_sha256,
            &demo.view.snapshot_id,
            &demo.view.universe_id,
            &demo.forecasts,
        );
        let forecast_bytes = serde_json::to_vec(&forecast_identity)
            .map_err(|error| PythonResearchError(error.to_string()))?;
        if sha256(&forecast_bytes) != demo.view.forecast_sha256 {
            return Err(PythonResearchError("model-forecast-hash-mismatch".into()));
        }
        if demo.forecasts.windows(2).any(|rows| {
            (rows[0].datetime, rows[0].instrument.as_str())
                >= (rows[1].datetime, rows[1].instrument.as_str())
        }) || demo.forecasts.iter().any(|row| {
            row.instrument.trim().is_empty()
                || row.value.is_some_and(|value| !value.is_finite())
                || row.value.is_some() == row.unavailable_reason.is_some()
        }) {
            return Err(PythonResearchError(
                "model-forecast-contract-invalid".into(),
            ));
        }
        let mut database = self
            .database
            .lock()
            .map_err(|_| PythonResearchError("model-lab-store-lock-poisoned".into()))?;
        let mut next = database.clone();
        let artifact_key = model_key(user_id, &demo.artifact.artifact_sha256);
        if let Some(existing) = next.artifacts.get(&artifact_key)
            && existing != &artifact_bytes
        {
            return Err(PythonResearchError(
                "model-artifact-identity-collision".into(),
            ));
        }
        let transformation_key = model_key(user_id, &demo.transformation.transformation_sha256);
        if let Some(existing) = next.transformations.get(&transformation_key)
            && existing != &transformation_bytes
        {
            return Err(PythonResearchError(
                "model-transformation-identity-collision".into(),
            ));
        }
        if publish_forecast_dataset {
            let forecast_key = model_key(user_id, &demo.view.forecast_sha256);
            let forecast_dataset = StoredForecastDataset {
                schema: "adaq:forecast-signal-dataset@1".into(),
                dataset_sha256: demo.view.forecast_sha256.clone(),
                producer_id: MODEL_PROJECT_ID.into(),
                producer_adapter_id: demo.view.adapter_id.clone(),
                producer_artifact_sha256: demo.artifact.artifact_sha256.clone(),
                input_evidence_sha256: demo.view.input_evidence_sha256.clone(),
                signal_id: "forecast".into(),
                target_id: demo.view.target_id.clone(),
                horizon_bars: demo.view.target_horizon_bars,
                forecast_contract: demo.view.forecast_contract.clone(),
                provenance_hashes: demo.artifact.provenance_hashes.clone(),
                snapshot_id: demo.view.snapshot_id.clone(),
                universe_id: demo.view.universe_id.clone(),
                rows: demo.forecasts.clone(),
            };
            if let Some(existing) = next.forecast_datasets.get(&forecast_key)
                && existing != &forecast_dataset
            {
                return Err(PythonResearchError(
                    "model-forecast-identity-collision".into(),
                ));
            }
            next.forecast_datasets
                .insert(forecast_key, forecast_dataset);
        }
        next.artifacts.insert(artifact_key, artifact_bytes);
        next.transformations
            .insert(transformation_key, transformation_bytes);
        next.runs
            .insert(model_key(user_id, &demo.view.attempt_id), demo.view.clone());
        self.persist(&next)?;
        *database = next;
        Ok(demo.view.clone())
    }

    fn run(&self, user_id: &str, attempt_id: &str) -> Result<ModelRunView, PythonResearchError> {
        self.database
            .lock()
            .map_err(|_| PythonResearchError("model-lab-store-lock-poisoned".into()))?
            .runs
            .get(&model_key(user_id, attempt_id))
            .cloned()
            .ok_or_else(|| PythonResearchError("model-run-not-found".into()))
    }

    fn artifact(
        &self,
        user_id: &str,
        artifact_sha256: &str,
    ) -> Result<Vec<u8>, PythonResearchError> {
        self.database
            .lock()
            .map_err(|_| PythonResearchError("model-lab-store-lock-poisoned".into()))?
            .artifacts
            .get(&model_key(user_id, artifact_sha256))
            .cloned()
            .ok_or_else(|| PythonResearchError("model-artifact-not-found".into()))
    }

    fn transformation(
        &self,
        user_id: &str,
        transformation_sha256: &str,
    ) -> Result<Vec<u8>, PythonResearchError> {
        self.database
            .lock()
            .map_err(|_| PythonResearchError("model-lab-store-lock-poisoned".into()))?
            .transformations
            .get(&model_key(user_id, transformation_sha256))
            .cloned()
            .ok_or_else(|| PythonResearchError("model-transformation-not-found".into()))
    }

    fn final_evaluation(
        &self,
        user_id: &str,
        decision_id: &str,
    ) -> Result<Option<ModelFinalEvaluationState>, PythonResearchError> {
        let database = self
            .database
            .lock()
            .map_err(|_| PythonResearchError("model-lab-store-lock-poisoned".into()))?;
        if let Some(state) = database
            .final_evaluations
            .get(&model_key(user_id, decision_id))
            .cloned()
        {
            return Ok(Some(state));
        }
        let reports = database
            .final_reports
            .iter()
            .filter(|(key, report)| {
                key.starts_with(&format!("{user_id}:")) && report.decision_id == decision_id
            })
            .map(|(_, report)| report)
            .collect::<Vec<_>>();
        Ok((reports.len() == 1).then(|| ModelFinalEvaluationState {
            decision_id: decision_id.into(),
            status: ModelFinalEvaluationStatus::Completed,
            attempt_id: None,
            staged_dataset_sha256: Some(reports[0].forecast_dataset_sha256.clone()),
            report_id: Some(reports[0].report_id.clone()),
            failure_code: None,
            diagnostic: None,
            created_at_ms: 0,
            updated_at_ms: 0,
        }))
    }

    fn qualification_reports(
        &self,
        user_id: &str,
        decision_id: &str,
    ) -> Result<Vec<ModelRuntimeQualificationReport>, PythonResearchError> {
        crate::user::validate_user(user_id).map_err(PythonResearchError)?;
        let prefix = format!("{user_id}:");
        let mut reports = self
            .database
            .lock()
            .map_err(|_| PythonResearchError("model-lab-store-lock-poisoned".into()))?
            .model_qualification_reports
            .iter()
            .filter(|(key, report)| key.starts_with(&prefix) && report.decision_id == decision_id)
            .map(|(_, report)| report.clone())
            .collect::<Vec<_>>();
        reports.sort_by(|left, right| {
            left.created_at_ms
                .cmp(&right.created_at_ms)
                .then_with(|| left.report_id.cmp(&right.report_id))
        });
        Ok(reports)
    }

    fn accepted_model_inputs(
        &self,
        user_id: &str,
    ) -> Result<Vec<AcceptedModelInput>, PythonResearchError> {
        crate::user::validate_user(user_id).map_err(PythonResearchError)?;
        let prefix = format!("{user_id}:");
        let report_ids = self
            .database
            .lock()
            .map_err(|_| PythonResearchError("model-lab-store-lock-poisoned".into()))?
            .model_qualification_reports
            .iter()
            .filter(|(key, report)| key.starts_with(&prefix) && report.qualified)
            .map(|(_, report)| report.report_id.clone())
            .collect::<Vec<_>>();
        let mut inputs = report_ids
            .into_iter()
            .map(|report_id| self.accepted_model_input(user_id, &report_id))
            .collect::<Result<Vec<_>, _>>()?;
        inputs.sort_by(|left, right| {
            left.qualification_report_id
                .cmp(&right.qualification_report_id)
        });
        Ok(inputs)
    }

    fn accepted_model_input(
        &self,
        user_id: &str,
        report_id: &str,
    ) -> Result<AcceptedModelInput, PythonResearchError> {
        crate::user::validate_user(user_id).map_err(PythonResearchError)?;
        let database = self
            .database
            .lock()
            .map_err(|_| PythonResearchError("model-lab-store-lock-poisoned".into()))?;
        let report = database
            .model_qualification_reports
            .get(&model_key(user_id, report_id))
            .cloned()
            .ok_or_else(|| PythonResearchError("model-qualification-report-not-found".into()))?;
        report.validate()?;
        if !report.qualified {
            return Err(PythonResearchError(
                "model-qualification-report-not-accepted".into(),
            ));
        }
        let decision = database
            .decisions
            .get(&model_key(user_id, &report.decision_id))
            .cloned()
            .ok_or_else(|| PythonResearchError("model-selection-decision-not-found".into()))?;
        decision.validate()?;
        let final_report = database
            .final_reports
            .get(&model_key(user_id, &report.final_evaluation_report_id))
            .cloned()
            .ok_or_else(|| PythonResearchError("model-final-evaluation-report-not-found".into()))?;
        final_report.validate()?;
        let final_state = database
            .final_evaluations
            .get(&model_key(user_id, &decision.decision_id));
        if report.report_id != report_id
            || report.decision_id != decision.decision_id
            || report.final_evaluation_report_id != final_report.report_id
            || final_report.decision_id != decision.decision_id
            || final_report.artifact_sha256 != report.artifact_sha256
            || report.artifact_sha256 == UNKNOWN_MODEL_IDENTITY
            || report.package_archive_sha256.is_none()
            || report.wasm_sha256.is_none()
            || report.component_id.is_none()
            || report.component_version.is_none()
            || report.imported_component_archive_sha256 != report.package_archive_sha256
            || final_state.is_some_and(|state| {
                state.status != ModelFinalEvaluationStatus::Completed
                    || state.report_id.as_deref()
                        != Some(report.final_evaluation_report_id.as_str())
            })
        {
            return Err(PythonResearchError(
                "model-qualification-input-binding-invalid".into(),
            ));
        }
        Ok(AcceptedModelInput {
            qualification_report_id: report.report_id,
            decision_id: report.decision_id,
            final_evaluation_report_id: report.final_evaluation_report_id,
            artifact_sha256: report.artifact_sha256,
            transformation_sha256: report.transformation_sha256,
            package_archive_sha256: report
                .package_archive_sha256
                .ok_or_else(|| PythonResearchError("model-qualification-package-missing".into()))?,
            package_wasm_sha256: report
                .wasm_sha256
                .ok_or_else(|| PythonResearchError("model-qualification-wasm-missing".into()))?,
            component_id: report.component_id.ok_or_else(|| {
                PythonResearchError("model-qualification-component-missing".into())
            })?,
            component_version: report
                .component_version
                .ok_or_else(|| PythonResearchError("model-qualification-version-missing".into()))?,
            model_profile: report.wasi_profile,
            exporter_id: report.exporter_id,
            sdk_version: report.sdk_version,
            abi_version: report.abi_version,
            runtime_identity: report.runtime_identity,
            input_slots: report.input_slots,
            output_name: MODEL_OUTPUT_NAME.into(),
            target_id: report.target_id,
            target_horizon_bars: report.target_horizon_bars,
            forecast_contract: report.forecast_contract,
            input_evidence_sha256: decision.input_evidence_sha256,
        })
    }

    fn save_qualification_report(
        &self,
        user_id: &str,
        report: ModelRuntimeQualificationReport,
    ) -> Result<ModelRuntimeQualificationReport, PythonResearchError> {
        crate::user::validate_user(user_id).map_err(PythonResearchError)?;
        report.validate()?;
        let mut database = self
            .database
            .lock()
            .map_err(|_| PythonResearchError("model-lab-store-lock-poisoned".into()))?;
        let decision = database
            .decisions
            .get(&model_key(user_id, &report.decision_id));
        if report.qualified && decision.is_none() {
            return Err(PythonResearchError(
                "model-selection-decision-not-found".into(),
            ));
        }
        if let Some(decision) = decision
            && report.artifact_sha256 != UNKNOWN_MODEL_IDENTITY
            && decision.candidate_artifact_sha256 != report.artifact_sha256
        {
            return Err(PythonResearchError(
                "model-runtime-qualification-binding-invalid".into(),
            ));
        }
        if !report.final_evaluation_report_id.is_empty() {
            let final_report = database
                .final_reports
                .get(&model_key(user_id, &report.final_evaluation_report_id))
                .ok_or_else(|| {
                    PythonResearchError("model-final-evaluation-report-not-found".into())
                })?;
            if final_report.decision_id != report.decision_id
                || report.artifact_sha256 != UNKNOWN_MODEL_IDENTITY
                    && final_report.artifact_sha256 != report.artifact_sha256
            {
                return Err(PythonResearchError(
                    "model-runtime-qualification-binding-invalid".into(),
                ));
            }
        } else if report.qualified {
            return Err(PythonResearchError(
                "model-final-evaluation-report-not-found".into(),
            ));
        }
        if report.qualified
            && let Some(existing) = database
                .model_qualification_reports
                .values()
                .find(|existing| {
                    same_qualified_model_report(existing, &report)
                        && database
                            .model_qualification_reports
                            .get(&model_key(user_id, &existing.report_id))
                            .is_some()
                })
        {
            return Ok(existing.clone());
        }
        let key = model_key(user_id, &report.report_id);
        if let Some(existing) = database.model_qualification_reports.get(&key) {
            if existing != &report {
                return Err(PythonResearchError(
                    "model-runtime-qualification-identity-collision".into(),
                ));
            }
            return Ok(existing.clone());
        }
        let mut next = database.clone();
        next.model_qualification_reports.insert(key, report.clone());
        self.persist(&next)?;
        *database = next;
        Ok(report)
    }

    fn complete_trial_with_candidate(
        &self,
        user_id: &str,
        experiment_id: &str,
        trial_id: &str,
        attempt_id: String,
        selection_metric: f64,
        candidate_artifact_sha256: String,
    ) -> Result<ModelExperiment, PythonResearchError> {
        let mut database = self
            .database
            .lock()
            .map_err(|_| PythonResearchError("model-lab-store-lock-poisoned".into()))?;
        let mut next = database.clone();
        let run = next
            .runs
            .get(&model_key(user_id, &attempt_id))
            .cloned()
            .ok_or_else(|| PythonResearchError("model-run-not-found".into()))?;
        let artifact_bytes = next
            .artifacts
            .get(&model_key(user_id, &candidate_artifact_sha256))
            .cloned()
            .ok_or_else(|| PythonResearchError("model-trial-candidate-artifact-missing".into()))?;
        let artifact = LinearModelArtifact::reload(&artifact_bytes).map_err(|error| {
            PythonResearchError(format!("model-trial-candidate-artifact-invalid:{error}"))
        })?;
        let experiment = next
            .experiments
            .get(&model_key(user_id, experiment_id))
            .cloned()
            .ok_or_else(|| PythonResearchError("model-experiment-not-found".into()))?;
        let trial = experiment
            .trials
            .iter()
            .find(|trial| trial.trial_id == trial_id)
            .ok_or_else(|| PythonResearchError("model-trial-not-found".into()))?;
        if !validate_model_trial_run(&run, trial, &attempt_id)
            || run.artifact_sha256 != candidate_artifact_sha256
            || !run.repeatability_verified
            || run.repeatability_state != RepeatabilityState::Verified
            || artifact.artifact_sha256 != candidate_artifact_sha256
            || artifact.alpha.to_bits() != trial.alpha.to_bits()
            || artifact.provenance_hashes != model_run_provenance(&run)?
        {
            return Err(PythonResearchError(
                "model-trial-candidate-binding-invalid".into(),
            ));
        }
        let experiment = next
            .experiments
            .get_mut(&model_key(user_id, experiment_id))
            .ok_or_else(|| PythonResearchError("model-experiment-not-found".into()))?;
        experiment.complete_trial_with_candidate(
            trial_id,
            attempt_id,
            selection_metric,
            candidate_artifact_sha256,
        )?;
        experiment.validate()?;
        let result = experiment.clone();
        let result_trial = result
            .trials
            .iter()
            .find(|trial| trial.trial_id == trial_id)
            .ok_or_else(|| PythonResearchError("model-trial-not-found".into()))?;
        validate_model_trial_candidate(&next, user_id, result_trial)?;
        self.persist(&next)?;
        *database = next;
        Ok(result)
    }

    fn complete_trial_with_repeatability(
        &self,
        user_id: &str,
        experiment_id: &str,
        trial_id: &str,
        attempt_id: String,
        selection_metric: f64,
        repeatability_state: RepeatabilityState,
    ) -> Result<ModelExperiment, PythonResearchError> {
        let mut database = self
            .database
            .lock()
            .map_err(|_| PythonResearchError("model-lab-store-lock-poisoned".into()))?;
        let mut next = database.clone();
        let run = next
            .runs
            .get(&model_key(user_id, &attempt_id))
            .cloned()
            .ok_or_else(|| PythonResearchError("model-run-not-found".into()))?;
        let experiment = next
            .experiments
            .get(&model_key(user_id, experiment_id))
            .cloned()
            .ok_or_else(|| PythonResearchError("model-experiment-not-found".into()))?;
        let trial = experiment
            .trials
            .iter()
            .find(|trial| trial.trial_id == trial_id)
            .ok_or_else(|| PythonResearchError("model-trial-not-found".into()))?;
        if repeatability_state == RepeatabilityState::Verified
            || run.repeatability_verified
            || run.repeatability_state != repeatability_state
            || !validate_model_trial_run(&run, trial, &attempt_id)
        {
            return Err(PythonResearchError(
                "model-trial-result-binding-invalid".into(),
            ));
        }
        let experiment = next
            .experiments
            .get_mut(&model_key(user_id, experiment_id))
            .ok_or_else(|| PythonResearchError("model-experiment-not-found".into()))?;
        experiment.complete_trial_with_repeatability(
            trial_id,
            attempt_id,
            selection_metric,
            repeatability_state,
        )?;
        experiment.validate()?;
        let result = experiment.clone();
        self.persist(&next)?;
        *database = next;
        Ok(result)
    }

    fn select(
        &self,
        user_id: &str,
        experiment_id: &str,
        trial_id: &str,
    ) -> Result<ParameterSelectionDecision, PythonResearchError> {
        let mut database = self
            .database
            .lock()
            .map_err(|_| PythonResearchError("model-lab-store-lock-poisoned".into()))?;
        let experiment = database
            .experiments
            .get(&model_key(user_id, experiment_id))
            .ok_or_else(|| PythonResearchError("model-experiment-not-found".into()))?;
        let decision = ParameterSelectionDecision::record(experiment, trial_id)?;
        for trial in &experiment.trials {
            if trial.candidate_artifact_sha256.is_some() {
                validate_model_trial_candidate(&database, user_id, trial).map_err(|error| {
                    PythonResearchError(format!(
                        "model-selection-candidate-binding-invalid:{error}"
                    ))
                })?;
            }
        }
        let key = model_key(user_id, &decision.decision_id);
        if let Some(existing) = database.decisions.get(&key) {
            if existing != &decision {
                return Err(PythonResearchError(
                    "model-selection-decision-identity-collision".into(),
                ));
            }
            return Ok(existing.clone());
        }
        let user_prefix = format!("{user_id}:");
        if database.final_reports.iter().any(|(report_key, report)| {
            report_key.starts_with(&user_prefix)
                && database
                    .decisions
                    .get(&model_key(user_id, &report.decision_id))
                    .is_some_and(|existing| existing.experiment_id == experiment_id)
        }) {
            return Err(PythonResearchError(
                "model-selection-after-final-evaluation".into(),
            ));
        }
        if database.decisions.iter().any(|(decision_key, existing)| {
            decision_key.starts_with(&user_prefix)
                && existing.experiment_id == experiment_id
                && existing != &decision
        }) {
            return Err(PythonResearchError(
                "model-selection-decision-already-recorded".into(),
            ));
        }
        database.decisions.insert(key, decision.clone());
        self.persist(&database)?;
        Ok(decision)
    }

    fn decision(
        &self,
        user_id: &str,
        decision_id: &str,
    ) -> Result<ParameterSelectionDecision, PythonResearchError> {
        self.database
            .lock()
            .map_err(|_| PythonResearchError("model-lab-store-lock-poisoned".into()))?
            .decisions
            .get(&model_key(user_id, decision_id))
            .cloned()
            .ok_or_else(|| PythonResearchError("model-selection-decision-not-found".into()))
    }

    fn experiment(
        &self,
        user_id: &str,
        experiment_id: &str,
    ) -> Result<ModelExperiment, PythonResearchError> {
        self.database
            .lock()
            .map_err(|_| PythonResearchError("model-lab-store-lock-poisoned".into()))?
            .experiments
            .get(&model_key(user_id, experiment_id))
            .cloned()
            .ok_or_else(|| PythonResearchError("model-experiment-not-found".into()))
    }

    fn list_experiments(&self, user_id: &str) -> Result<Vec<ModelExperiment>, PythonResearchError> {
        let prefix = format!("{user_id}:");
        let mut experiments = self
            .database
            .lock()
            .map_err(|_| PythonResearchError("model-lab-store-lock-poisoned".into()))?
            .experiments
            .iter()
            .filter(|(key, _)| key.starts_with(&prefix))
            .map(|(_, experiment)| experiment.clone())
            .collect::<Vec<_>>();
        experiments.sort_by(|left, right| left.experiment_id.cmp(&right.experiment_id));
        Ok(experiments)
    }

    fn projection(
        &self,
        user_id: &str,
        factor_decision_hash: Option<&str>,
    ) -> Result<ModelLabState, PythonResearchError> {
        let database = self
            .database
            .lock()
            .map_err(|_| PythonResearchError("model-lab-store-lock-poisoned".into()))?;
        let prefix = format!("{user_id}:");
        let mut experiments = database
            .experiments
            .iter()
            .filter(|(key, experiment)| {
                key.starts_with(&prefix)
                    && factor_decision_hash
                        .is_none_or(|hash| experiment.factor_decision_hash == hash)
            })
            .map(|(_, experiment)| experiment.clone())
            .collect::<Vec<_>>();
        experiments.sort_by(|left, right| left.experiment_id.cmp(&right.experiment_id));
        let experiment = (experiments.len() == 1).then(|| experiments[0].clone());
        let decision = experiment.as_ref().and_then(|experiment| {
            let decisions = database
                .decisions
                .iter()
                .filter(|(key, decision)| {
                    key.starts_with(&prefix) && decision.experiment_id == experiment.experiment_id
                })
                .map(|(_, decision)| decision.clone())
                .collect::<Vec<_>>();
            (decisions.len() == 1).then(|| decisions[0].clone())
        });
        let report = decision.as_ref().and_then(|decision| {
            let reports = database
                .final_reports
                .iter()
                .filter(|(key, report)| {
                    key.starts_with(&prefix) && report.decision_id == decision.decision_id
                })
                .map(|(_, report)| report.clone())
                .collect::<Vec<_>>();
            (reports.len() == 1).then(|| reports[0].clone())
        });
        let final_evaluation = decision.as_ref().and_then(|decision| {
            database
                .final_evaluations
                .get(&model_key(user_id, &decision.decision_id))
                .cloned()
                .or_else(|| {
                    report.as_ref().map(|report| ModelFinalEvaluationState {
                        decision_id: decision.decision_id.clone(),
                        status: ModelFinalEvaluationStatus::Completed,
                        attempt_id: None,
                        staged_dataset_sha256: Some(report.forecast_dataset_sha256.clone()),
                        report_id: Some(report.report_id.clone()),
                        failure_code: None,
                        diagnostic: None,
                        created_at_ms: 0,
                        updated_at_ms: 0,
                    })
                })
        });
        let mut deployment_reports = decision
            .as_ref()
            .map(|decision| {
                database
                    .model_qualification_reports
                    .iter()
                    .filter(|(key, report)| {
                        key.starts_with(&prefix) && report.decision_id == decision.decision_id
                    })
                    .map(|(_, report)| report.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        deployment_reports.sort_by(|left, right| {
            left.created_at_ms
                .cmp(&right.created_at_ms)
                .then_with(|| left.report_id.cmp(&right.report_id))
        });
        Ok(ModelLabState {
            experiments,
            experiment,
            decision,
            report,
            final_evaluation,
            deployment_reports,
        })
    }

    fn begin_final(
        &self,
        user_id: &str,
        decision_id: &str,
    ) -> Result<Option<String>, PythonResearchError> {
        let mut database = self
            .database
            .lock()
            .map_err(|_| PythonResearchError("model-lab-store-lock-poisoned".into()))?;
        if database.final_reports.values().any(|report| {
            report.decision_id == decision_id
                && database
                    .final_reports
                    .get(&model_key(user_id, &report.report_id))
                    .is_some()
        }) {
            return Err(PythonResearchError(
                "model-final-evaluation-already-recorded".into(),
            ));
        }
        database
            .decisions
            .get(&model_key(user_id, decision_id))
            .ok_or_else(|| PythonResearchError("model-selection-decision-not-found".into()))?;
        let state_key = model_key(user_id, decision_id);
        let previous = database.final_evaluations.get(&state_key).cloned();
        if previous.as_ref().is_some_and(|state| {
            matches!(
                state.status,
                ModelFinalEvaluationStatus::Pending | ModelFinalEvaluationStatus::Running
            )
        }) {
            return Err(PythonResearchError(
                "model-final-evaluation-in-progress".into(),
            ));
        }
        let now = model_lab_now_ms();
        let state = ModelFinalEvaluationState {
            decision_id: decision_id.into(),
            status: ModelFinalEvaluationStatus::Running,
            attempt_id: previous.as_ref().and_then(|state| state.attempt_id.clone()),
            staged_dataset_sha256: previous
                .as_ref()
                .and_then(|state| state.staged_dataset_sha256.clone()),
            report_id: None,
            failure_code: None,
            diagnostic: None,
            created_at_ms: previous.as_ref().map_or(now, |state| state.created_at_ms),
            updated_at_ms: now,
        };
        let mut next = database.clone();
        let retry_attempt_id = state.attempt_id.clone();
        next.final_evaluations.insert(state_key, state);
        self.persist(&next)?;
        *database = next;
        Ok(retry_attempt_id)
    }

    fn bind_final_attempt(
        &self,
        user_id: &str,
        decision_id: &str,
        attempt_id: &str,
    ) -> Result<(), PythonResearchError> {
        if attempt_id.trim().is_empty() {
            return Err(PythonResearchError(
                "model-final-evaluation-attempt-invalid".into(),
            ));
        }
        let mut database = self
            .database
            .lock()
            .map_err(|_| PythonResearchError("model-lab-store-lock-poisoned".into()))?;
        let mut next = database.clone();
        let state = next
            .final_evaluations
            .get_mut(&model_key(user_id, decision_id))
            .ok_or_else(|| PythonResearchError("model-final-evaluation-state-not-found".into()))?;
        if state.status != ModelFinalEvaluationStatus::Running {
            return Err(PythonResearchError(
                "model-final-evaluation-state-transition-invalid".into(),
            ));
        }
        state.attempt_id = Some(attempt_id.into());
        state.updated_at_ms = model_lab_now_ms();
        self.persist(&next)?;
        *database = next;
        Ok(())
    }

    fn stage_final_dataset(
        &self,
        user_id: &str,
        decision_id: &str,
        dataset_sha256: &str,
    ) -> Result<(), PythonResearchError> {
        if !is_sha256_text(dataset_sha256) {
            return Err(PythonResearchError(
                "model-final-evaluation-dataset-invalid".into(),
            ));
        }
        let mut database = self
            .database
            .lock()
            .map_err(|_| PythonResearchError("model-lab-store-lock-poisoned".into()))?;
        let mut next = database.clone();
        let state = next
            .final_evaluations
            .get_mut(&model_key(user_id, decision_id))
            .ok_or_else(|| PythonResearchError("model-final-evaluation-state-not-found".into()))?;
        if state.status != ModelFinalEvaluationStatus::Running
            || state
                .staged_dataset_sha256
                .as_deref()
                .is_some_and(|existing| existing != dataset_sha256)
        {
            return Err(PythonResearchError(
                "model-final-evaluation-state-transition-invalid".into(),
            ));
        }
        state.staged_dataset_sha256 = Some(dataset_sha256.into());
        state.updated_at_ms = model_lab_now_ms();
        self.persist(&next)?;
        *database = next;
        Ok(())
    }

    fn fail_final(
        &self,
        user_id: &str,
        decision_id: &str,
        status: ModelFinalEvaluationStatus,
        failure_code: &str,
        diagnostic: &str,
    ) -> Result<(), PythonResearchError> {
        if matches!(
            status,
            ModelFinalEvaluationStatus::Pending
                | ModelFinalEvaluationStatus::Running
                | ModelFinalEvaluationStatus::Completed
        ) {
            return Err(PythonResearchError(
                "model-final-evaluation-state-transition-invalid".into(),
            ));
        }
        let mut database = self
            .database
            .lock()
            .map_err(|_| PythonResearchError("model-lab-store-lock-poisoned".into()))?;
        let mut next = database.clone();
        let state = next
            .final_evaluations
            .get_mut(&model_key(user_id, decision_id))
            .ok_or_else(|| PythonResearchError("model-final-evaluation-state-not-found".into()))?;
        if state.report_id.is_some() {
            return Err(PythonResearchError(
                "model-final-evaluation-state-transition-invalid".into(),
            ));
        }
        state.status = status;
        state.failure_code = Some(failure_code.into());
        state.diagnostic = Some(bounded_model_diagnostic(diagnostic));
        state.updated_at_ms = model_lab_now_ms();
        self.persist(&next)?;
        *database = next;
        Ok(())
    }

    fn save_final(
        &self,
        user_id: &str,
        report: FinalEvaluationReport,
    ) -> Result<FinalEvaluationReport, PythonResearchError> {
        self.save_final_with_attempt(user_id, report, None)
    }

    fn save_final_with_attempt(
        &self,
        user_id: &str,
        report: FinalEvaluationReport,
        attempt_id: Option<&str>,
    ) -> Result<FinalEvaluationReport, PythonResearchError> {
        report.validate()?;
        if attempt_id.is_some_and(|attempt_id| attempt_id.trim().is_empty()) {
            return Err(PythonResearchError(
                "model-final-evaluation-attempt-invalid".into(),
            ));
        }
        let mut database = self
            .database
            .lock()
            .map_err(|_| PythonResearchError("model-lab-store-lock-poisoned".into()))?;
        let decision = database
            .decisions
            .get(&model_key(user_id, &report.decision_id))
            .ok_or_else(|| PythonResearchError("model-selection-decision-not-found".into()))?;
        if report.artifact_sha256 != decision.candidate_artifact_sha256 {
            return Err(PythonResearchError(
                "model-final-evaluation-decision-binding-invalid".into(),
            ));
        }
        let report_key = model_key(user_id, &report.report_id);
        if let Some(existing) = database.final_reports.get(&report_key) {
            if existing != &report {
                return Err(PythonResearchError(
                    "model-final-evaluation-identity-collision".into(),
                ));
            }
            return Ok(existing.clone());
        }
        if database.final_reports.values().any(|existing| {
            existing.decision_id == report.decision_id
                && database
                    .final_reports
                    .get(&model_key(user_id, &existing.report_id))
                    .is_some()
        }) {
            return Err(PythonResearchError(
                "model-final-evaluation-already-recorded".into(),
            ));
        }
        let state_key = model_key(user_id, &report.decision_id);
        let previous_state = database.final_evaluations.get(&state_key).cloned();
        if previous_state
            .as_ref()
            .and_then(|state| state.staged_dataset_sha256.as_deref())
            .is_some_and(|dataset| dataset != report.forecast_dataset_sha256)
        {
            return Err(PythonResearchError(
                "model-final-evaluation-dataset-binding-invalid".into(),
            ));
        }
        let now = model_lab_now_ms();
        let final_state = ModelFinalEvaluationState {
            decision_id: report.decision_id.clone(),
            status: ModelFinalEvaluationStatus::Completed,
            attempt_id: attempt_id.map(str::to_owned).or_else(|| {
                previous_state
                    .as_ref()
                    .and_then(|state| state.attempt_id.clone())
            }),
            staged_dataset_sha256: Some(report.forecast_dataset_sha256.clone()),
            report_id: Some(report.report_id.clone()),
            failure_code: None,
            diagnostic: None,
            created_at_ms: previous_state
                .as_ref()
                .map_or(now, |state| state.created_at_ms),
            updated_at_ms: now,
        };
        let mut next = database.clone();
        next.final_reports.insert(report_key, report.clone());
        next.final_evaluations.insert(state_key, final_state);
        self.persist(&next)?;
        *database = next;
        Ok(report)
    }

    fn final_report(
        &self,
        user_id: &str,
        decision_id: &str,
    ) -> Result<Option<FinalEvaluationReport>, PythonResearchError> {
        let prefix = format!("{user_id}:");
        let reports = self
            .database
            .lock()
            .map_err(|_| PythonResearchError("model-lab-store-lock-poisoned".into()))?
            .final_reports
            .iter()
            .filter(|(key, report)| key.starts_with(&prefix) && report.decision_id == decision_id)
            .map(|(_, report)| report.clone())
            .collect::<Vec<_>>();
        Ok((reports.len() == 1).then(|| reports[0].clone()))
    }

    fn has_final(&self, user_id: &str, decision_id: &str) -> Result<bool, PythonResearchError> {
        Ok(self
            .database
            .lock()
            .map_err(|_| PythonResearchError("model-lab-store-lock-poisoned".into()))?
            .final_reports
            .iter()
            .any(|(key, report)| {
                key.starts_with(&format!("{user_id}:")) && report.decision_id == decision_id
            }))
    }

    fn reset_user(&self, user_id: &str) -> Result<(), PythonResearchError> {
        let mut database = self
            .database
            .lock()
            .map_err(|_| PythonResearchError("model-lab-store-lock-poisoned".into()))?;
        database
            .experiments
            .retain(|key, _| !key.starts_with(&format!("{user_id}:")));
        database
            .decisions
            .retain(|key, _| !key.starts_with(&format!("{user_id}:")));
        database
            .final_reports
            .retain(|key, _| !key.starts_with(&format!("{user_id}:")));
        database
            .final_evaluations
            .retain(|key, _| !key.starts_with(&format!("{user_id}:")));
        database
            .runs
            .retain(|key, _| !key.starts_with(&format!("{user_id}:")));
        database
            .artifacts
            .retain(|key, _| !key.starts_with(&format!("{user_id}:")));
        database
            .transformations
            .retain(|key, _| !key.starts_with(&format!("{user_id}:")));
        database
            .forecast_datasets
            .retain(|key, _| !key.starts_with(&format!("{user_id}:")));
        database
            .model_qualification_reports
            .retain(|key, _| !key.starts_with(&format!("{user_id}:")));
        self.persist(&database)
    }
}

fn replace_model_lab_file(temporary: &Path, destination: &Path) -> Result<(), PythonResearchError> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        };

        let temporary = temporary
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let destination = destination
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let replaced = unsafe {
            MoveFileExW(
                temporary.as_ptr(),
                destination.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if replaced == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    }

    #[cfg(not(windows))]
    {
        fs::rename(temporary, destination)?;
        #[cfg(unix)]
        if let Some(parent) = destination.parent() {
            fs::File::open(parent)?.sync_all()?;
        }
    }

    Ok(())
}

fn model_key(user_id: &str, identity: &str) -> String {
    format!("{user_id}:{identity}")
}

fn bounded_model_diagnostic(value: &str) -> String {
    value
        .split_whitespace()
        .map(|part| {
            let upper = part.to_ascii_uppercase();
            if ["TOKEN", "SECRET", "PASSWORD", "PRIVATE_KEY"]
                .iter()
                .any(|key| upper.contains(key))
            {
                "[redacted]"
            } else if part.starts_with('/') || part.contains('\\') {
                "[path]"
            } else {
                part
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(4096)
        .collect()
}

fn model_final_evaluation_failure_status(
    error: &str,
    staged_dataset: bool,
) -> ModelFinalEvaluationStatus {
    if error.contains("runner-cancelled") {
        ModelFinalEvaluationStatus::Cancelled
    } else if error.contains("interrupted") {
        ModelFinalEvaluationStatus::Interrupted
    } else if error.contains("stale") {
        ModelFinalEvaluationStatus::Stale
    } else if staged_dataset {
        ModelFinalEvaluationStatus::PersistenceFailed
    } else {
        ModelFinalEvaluationStatus::Failed
    }
}

fn model_run_provenance(
    view: &ModelRunView,
) -> Result<BTreeMap<String, String>, PythonResearchError> {
    Ok(BTreeMap::from([
        ("fixture".into(), view.fixture_sha256.clone()),
        ("revision".into(), view.project_revision_sha256.clone()),
        ("environment".into(), view.environment_sha256.clone()),
        ("input".into(), view.input_evidence_sha256.clone()),
        ("factorDecision".into(), view.factor_decision_hash.clone()),
        (
            "promotionProtocol".into(),
            view.factor_promotion_protocol_hash.clone(),
        ),
        (
            "resourcePolicy".into(),
            resource_policy_identity(&view.resource_policy)?,
        ),
        ("factorDataset".into(), view.factor_dataset_id.clone()),
        ("featureDataset".into(), view.feature_dataset_id.clone()),
        ("featurePlan".into(), view.feature_plan_hash.clone()),
        ("snapshot".into(), view.snapshot_id.clone()),
        ("universe".into(), view.universe_id.clone()),
    ]))
}

fn validate_model_trial_run(run: &ModelRunView, trial: &ModelTrial, attempt_id: &str) -> bool {
    run.attempt_id == attempt_id
        && run.alpha.to_bits() == trial.alpha.to_bits()
        && run.project_revision_sha256 == trial.project_revision_sha256
        && run.environment_sha256 == trial.environment_sha256
        && run.input_evidence_sha256 == trial.input_evidence_sha256
        && run.seed == trial.seed
        && run.binding_sha256 == trial.binding_sha256
}

fn attempt_matches_model_trial_alpha(attempt: &ResearchAttempt, trial: &ModelTrial) -> bool {
    attempt
        .execution
        .parameters
        .get("alpha")
        .and_then(|value| value.parse::<f64>().ok())
        .is_some_and(|alpha| alpha.to_bits() == trial.alpha.to_bits())
}

fn validate_model_trial_candidate(
    database: &ModelLabDatabase,
    user_id: &str,
    trial: &ModelTrial,
) -> Result<(), PythonResearchError> {
    let successful_attempt_id = trial
        .successful_attempt_id
        .as_deref()
        .ok_or_else(|| PythonResearchError("model-trial-candidate-missing-attempt".into()))?;
    let candidate_artifact_sha256 = trial
        .candidate_artifact_sha256
        .as_deref()
        .ok_or_else(|| PythonResearchError("model-trial-candidate-missing-artifact".into()))?;
    let run = database
        .runs
        .get(&model_key(user_id, successful_attempt_id))
        .ok_or_else(|| PythonResearchError("model-trial-candidate-attempt-missing".into()))?;
    let artifact_bytes = database
        .artifacts
        .get(&model_key(user_id, candidate_artifact_sha256))
        .ok_or_else(|| PythonResearchError("model-trial-candidate-artifact-missing".into()))?;
    let artifact = LinearModelArtifact::reload(artifact_bytes).map_err(|error| {
        PythonResearchError(format!("model-trial-candidate-artifact-invalid:{error}"))
    })?;
    if !validate_model_trial_run(run, trial, successful_attempt_id)
        || run.artifact_sha256 != candidate_artifact_sha256
        || !run.repeatability_verified
        || run.repeatability_state != RepeatabilityState::Verified
        || artifact.artifact_sha256 != candidate_artifact_sha256
        || artifact.alpha.to_bits() != trial.alpha.to_bits()
        || artifact.provenance_hashes != model_run_provenance(run)?
    {
        return Err(PythonResearchError(
            "model-trial-candidate-binding-invalid".into(),
        ));
    }
    Ok(())
}

fn default_model_resource_policy() -> HostResourcePolicy {
    HostResourcePolicy::m12_default()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRunView {
    pub attempt_id: String,
    pub adapter_id: String,
    pub alpha: f64,
    pub project_revision_sha256: String,
    pub environment_sha256: String,
    pub input_evidence_sha256: String,
    #[serde(default)]
    pub binding_sha256: String,
    pub factor_decision_hash: String,
    pub factor_promotion_protocol_hash: String,
    pub factor_dataset_id: String,
    pub feature_dataset_id: String,
    pub feature_plan_hash: String,
    pub snapshot_id: String,
    pub universe_id: String,
    pub factor_lookback: u32,
    pub seed: u64,
    pub fixture_sha256: String,
    pub artifact_sha256: String,
    pub transformation_sha256: String,
    pub forecast_sha256: String,
    pub train_rows: usize,
    pub selection_rows: usize,
    #[serde(default)]
    pub selection_metric: Option<f64>,
    pub final_rows: usize,
    pub test_labels_withheld: bool,
    pub repeatability_verified: bool,
    pub repeatability_tolerance: f64,
    #[serde(default)]
    pub repeatability_state: RepeatabilityState,
    #[serde(default)]
    pub evidence_state: EvidenceState,
    #[serde(default)]
    pub diagnostics: Vec<String>,
    pub windows: TutorialWindows,
    #[serde(default = "default_model_resource_policy")]
    pub resource_policy: HostResourcePolicy,
    #[serde(default)]
    pub input_slots: Vec<String>,
    #[serde(default)]
    pub target_id: String,
    #[serde(default)]
    pub target_horizon_bars: u32,
    #[serde(default)]
    pub forecast_contract: String,
    #[serde(default)]
    pub artifact_schema: String,
    #[serde(default)]
    pub numeric_representation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FactorRunView {
    pub attempt_id: String,
    pub candidate_hash: Option<String>,
    pub family_id: Option<String>,
    pub trial_ids: Vec<String>,
    pub dataset_ids: Vec<String>,
    pub report_hashes: Vec<String>,
    pub promotion_policy_hash: Option<String>,
    pub promotion_protocol_hash: Option<String>,
    pub promotion_decision_hash: Option<String>,
    pub selected_trial_id: Option<String>,
    pub selection_hash: Option<String>,
    pub promotion_state: Option<PromotionDecisionState>,
    pub project_id: String,
    pub project_revision_sha256: Option<String>,
    pub environment_sha256: Option<String>,
    pub fixture_sha256: String,
    pub input_bindings: Option<BTreeMap<String, String>>,
    pub normalized_parameters: Option<BTreeMap<String, String>>,
    pub seed: Option<u64>,
    pub sdk_artifact_sha256: Option<String>,
    pub resource_policy: Option<PythonFactorResourcePolicy>,
    pub snapshot_id: Option<String>,
    pub snapshot_bindings: Option<BTreeMap<String, String>>,
    pub point_in_time_universe_id: Option<String>,
    pub feature_dataset_id: Option<String>,
    pub feature_dataset_bindings: Option<BTreeMap<String, String>>,
    pub feature_evidence_sha256: Option<String>,
    pub feature_plan_hash: Option<String>,
    pub engine_identity: Option<String>,
    pub repeatability_report_sha256: Option<String>,
    pub repeatability_verified: bool,
    pub logs: Vec<String>,
    pub lookbacks: Vec<u32>,
    pub default_lookback: u32,
    pub rows_per_trial: usize,
    pub available_rows: BTreeMap<String, usize>,
    pub repeatability: BTreeMap<String, RepeatabilityReport>,
    pub repeatability_report: Option<BTreeMap<String, PythonRepeatabilityReport>>,
    pub synthetic: bool,
    pub selection_required: bool,
    pub promotion_required: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PythonFactorSelectionView {
    pub candidate_hash: String,
    pub family_id: String,
    pub selected_trial_id: String,
    pub selection_hash: String,
    pub promotion_protocol_hash: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PythonFactorPromotionView {
    pub candidate_hash: String,
    pub family_id: String,
    pub selected_trial_id: String,
    pub selection_hash: String,
    pub promotion_protocol_hash: String,
    pub decision_hash: String,
    pub state: PromotionDecisionState,
    pub eligibility_gates: Vec<adaq_factor_research::PromotionGateResult>,
}

struct DemoModelRun {
    view: ModelRunView,
    artifact: adaq_python_research::model::LinearModelArtifact,
    transformation: FittedTransformation,
    forecasts: Vec<adaq_python_research::model::ForecastRow>,
    final_labels: Vec<(i64, String, f64)>,
}

#[derive(Debug, Clone)]
struct ModelInputEvidence {
    decision_hash: String,
    promotion_protocol_hash: String,
    factor_dataset_id: String,
    feature_dataset_id: String,
    feature_plan_hash: String,
    snapshot_id: String,
    universe_id: String,
    lookback: u32,
}

struct ModelEvidenceData {
    fixture: SyntheticTutorialFixture,
    dataset: DatasetH,
    transformation: FittedTransformation,
    final_labels: Vec<(i64, String, f64)>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelCandidateEnvelope {
    schema: String,
    payload: ModelCandidatePayload,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelCandidatePayload {
    alpha: f64,
    adapter_id: String,
    artifact_schema: String,
    numeric_representation: String,
    forecast_contract: String,
    input_slots: Vec<String>,
    coefficients: Vec<f64>,
    intercept: f64,
    transformation_sha256: String,
}

fn model_provenance(
    fixture_sha256: &str,
    project_revision_sha256: &str,
    environment_sha256: &str,
    input_evidence_sha256: &str,
    resource_policy_sha256: &str,
    input: &ModelInputEvidence,
) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("fixture".into(), fixture_sha256.into()),
        ("revision".into(), project_revision_sha256.into()),
        ("environment".into(), environment_sha256.into()),
        ("input".into(), input_evidence_sha256.into()),
        ("resourcePolicy".into(), resource_policy_sha256.into()),
        ("factorDecision".into(), input.decision_hash.clone()),
        (
            "promotionProtocol".into(),
            input.promotion_protocol_hash.clone(),
        ),
        ("factorDataset".into(), input.factor_dataset_id.clone()),
        ("featureDataset".into(), input.feature_dataset_id.clone()),
        ("featurePlan".into(), input.feature_plan_hash.clone()),
        ("snapshot".into(), input.snapshot_id.clone()),
        ("universe".into(), input.universe_id.clone()),
    ])
}

fn resource_policy_identity(
    resource_policy: &HostResourcePolicy,
) -> Result<String, PythonResearchError> {
    Ok(sha256(
        &serde_json::to_vec(resource_policy)
            .map_err(|error| PythonResearchError(error.to_string()))?,
    ))
}

fn model_run_limits(resource_policy: &HostResourcePolicy) -> Result<RunLimits, String> {
    let defaults = RunLimits::default();
    Ok(RunLimits {
        fuel_per_call: defaults.fuel_per_call,
        memory_bytes: usize::try_from(resource_policy.max_memory_bytes)
            .map_err(|_| "model-runtime-memory-limit-invalid".to_owned())?,
        max_bars: usize::try_from(resource_policy.max_input_rows)
            .map_err(|_| "model-runtime-input-row-limit-invalid".to_owned())?,
    })
}

fn model_binding_sha256(view: &ModelRunView) -> Result<String, PythonResearchError> {
    let value = serde_json::json!([
        &view.adapter_id,
        &view.project_revision_sha256,
        &view.environment_sha256,
        &view.input_evidence_sha256,
        &view.factor_decision_hash,
        &view.factor_promotion_protocol_hash,
        &view.factor_dataset_id,
        &view.feature_dataset_id,
        &view.feature_plan_hash,
        &view.snapshot_id,
        &view.universe_id,
        &view.factor_lookback,
        &view.seed,
        &view.input_slots,
        &view.target_id,
        &view.target_horizon_bars,
        &view.forecast_contract,
        &view.artifact_schema,
        &view.numeric_representation,
        &view.transformation_sha256,
        &view.windows,
        &view.resource_policy,
    ]);
    let bytes =
        serde_json::to_vec(&value).map_err(|error| PythonResearchError(error.to_string()))?;
    Ok(sha256(&bytes))
}

fn read_model_candidate(
    root: &Path,
    execution: &RunnerExecution,
    expected_alpha: f64,
    transformation: &FittedTransformation,
    fixture_sha256: &str,
    project_revision_sha256: &str,
    environment_sha256: &str,
    input_evidence_sha256: &str,
    resource_policy_sha256: &str,
    input: &ModelInputEvidence,
) -> Result<adaq_python_research::model::LinearModelArtifact, PythonResearchError> {
    let staged = execution
        .staged_artifact
        .as_ref()
        .ok_or_else(|| PythonResearchError("model-runner-artifact-missing".into()))?;
    let attempt_id = execution
        .conformance
        .as_ref()
        .map(|result| result.attempt_id.as_str())
        .ok_or_else(|| PythonResearchError("model-runner-result-missing".into()))?;
    let bytes = fs::read(
        root.join("attempt-results")
            .join(format!("{attempt_id}.artifact")),
    )?;
    if sha256(&bytes) != staged.sha256 {
        return Err(PythonResearchError(
            "model-runner-artifact-hash-mismatch".into(),
        ));
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|error| PythonResearchError(format!("model-runner-artifact-invalid:{error}")))?;
    let candidate = serde_json::from_value::<ModelCandidateEnvelope>(
        value
            .get("payload")
            .and_then(|payload| payload.get("fit"))
            .cloned()
            .ok_or_else(|| PythonResearchError("model-runner-fit-missing".into()))?,
    )
    .map_err(|error| PythonResearchError(format!("model-runner-fit-invalid:{error}")))?;
    if candidate.schema != "adaq:linear-model:candidate@1"
        || candidate.payload.adapter_id != adaq_python_research::model::RIDGE_ADAPTER_ID
        || candidate.payload.artifact_schema
            != adaq_python_research::model::LINEAR_MODEL_ARTIFACT_SCHEMA
        || candidate.payload.numeric_representation
            != adaq_python_research::model::NUMERIC_REPRESENTATION
        || candidate.payload.forecast_contract != adaq_python_research::model::FORECAST_CONTRACT
        || candidate.payload.alpha.to_bits() != expected_alpha.to_bits()
        || candidate.payload.input_slots != transformation.feature_names
        || candidate.payload.transformation_sha256 != transformation.transformation_sha256
        || candidate.payload.coefficients.len() != transformation.feature_names.len()
        || candidate
            .payload
            .coefficients
            .iter()
            .any(|value| !value.is_finite())
        || !candidate.payload.intercept.is_finite()
    {
        return Err(PythonResearchError(
            "model-runner-fit-contract-invalid".into(),
        ));
    }
    adaq_python_research::model::LinearModelArtifact::from_coefficients(
        expected_alpha,
        candidate.payload.input_slots,
        candidate.payload.coefficients,
        candidate.payload.intercept,
        transformation.transformation_sha256.clone(),
        model_provenance(
            fixture_sha256,
            project_revision_sha256,
            environment_sha256,
            input_evidence_sha256,
            resource_policy_sha256,
            input,
        ),
    )
}

fn model_prediction_input(
    mut runner_input: serde_json::Value,
    artifact: &adaq_python_research::model::LinearModelArtifact,
) -> Result<serde_json::Value, PythonResearchError> {
    let object = runner_input
        .as_object_mut()
        .ok_or_else(|| PythonResearchError("model-runner-input-object-invalid".into()))?;
    object.insert(
        "fittedModel".into(),
        serde_json::json!({
            "schema": "adaq:linear-model:candidate@1",
            "payload": {
                "alpha": artifact.alpha,
                "adapter_id": artifact.adapter_id,
                "artifact_schema": artifact.schema,
                "numeric_representation": artifact.numeric_representation,
                "forecast_contract": artifact.forecast_contract,
                "input_slots": artifact.input_slots,
                "coefficients": artifact.coefficients,
                "intercept": artifact.intercept,
                "transformation_sha256": artifact.transformation_sha256,
            }
        }),
    );
    object.insert(
        "targetWindowEnd".into(),
        serde_json::json!(TutorialWindows::m12().final_end),
    );
    Ok(runner_input)
}

fn validate_model_python_forecast(
    execution: &RunnerExecution,
    expected: &[adaq_python_research::model::ForecastRow],
) -> Result<Vec<adaq_python_research::model::ForecastRow>, PythonResearchError> {
    let result = execution
        .conformance
        .as_ref()
        .ok_or_else(|| PythonResearchError("model-predict-result-missing".into()))?;
    if result.project_id != MODEL_PROJECT_ID || result.project_kind != "model" {
        return Err(PythonResearchError("model-predict-project-invalid".into()));
    }
    let values = result
        .payload
        .as_ref()
        .and_then(|payload| payload.get("forecast"))
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| PythonResearchError("model-predict-forecast-missing".into()))?;
    if values.len() != expected.len() {
        return Err(PythonResearchError(
            "model-predict-forecast-count-invalid".into(),
        ));
    }
    let forecasts = values
        .iter()
        .map(|actual| {
            let instrument = actual
                .get("instrument_id")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| PythonResearchError("model-predict-identity-invalid".into()))?;
            let datetime = actual
                .get("prediction_time_ms")
                .and_then(serde_json::Value::as_i64)
                .ok_or_else(|| PythonResearchError("model-predict-identity-invalid".into()))?;
            let value = actual
                .get("value")
                .ok_or_else(|| PythonResearchError("model-predict-value-missing".into()))?;
            if let Some(value) = value.as_f64().filter(|value| value.is_finite()) {
                Ok(adaq_python_research::model::ForecastRow {
                    datetime,
                    instrument: instrument.into(),
                    value: Some(value),
                    unavailable_reason: None,
                })
            } else {
                let reason = value
                    .get("reason")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| PythonResearchError("model-predict-value-invalid".into()))?;
                Ok(adaq_python_research::model::ForecastRow {
                    datetime,
                    instrument: instrument.into(),
                    value: None,
                    unavailable_reason: Some(reason.into()),
                })
            }
        })
        .collect::<Result<Vec<_>, PythonResearchError>>()?;
    for (actual, expected) in forecasts.iter().zip(expected) {
        if actual.instrument != expected.instrument || actual.datetime != expected.datetime {
            return Err(PythonResearchError(
                "model-predict-identity-divergent".into(),
            ));
        }
        match expected.value {
            Some(expected_value) => {
                let actual_value = actual
                    .value
                    .ok_or_else(|| PythonResearchError("model-predict-value-invalid".into()))?;
                if actual.unavailable_reason.is_some()
                    || (actual_value - expected_value).abs() > RIDGE_REPEATABILITY_TOLERANCE
                {
                    return Err(PythonResearchError("model-predict-value-divergent".into()));
                }
            }
            None => {
                if actual.value.is_some()
                    || actual.unavailable_reason.as_deref() != Some("target-window-boundary")
                {
                    return Err(PythonResearchError(
                        "model-predict-availability-divergent".into(),
                    ));
                }
            }
        }
    }
    Ok(forecasts)
}

fn model_forecast_sha256(
    artifact_sha256: &str,
    input_evidence_sha256: &str,
    snapshot_id: &str,
    universe_id: &str,
    forecasts: &[adaq_python_research::model::ForecastRow],
) -> Result<String, PythonResearchError> {
    let bytes = serde_json::to_vec(&(
        artifact_sha256,
        input_evidence_sha256,
        snapshot_id,
        universe_id,
        forecasts,
    ))
    .map_err(|error| PythonResearchError(error.to_string()))?;
    Ok(sha256(&bytes))
}

struct ModelDeploymentReplay {
    final_report: FinalEvaluationReport,
    run: ModelRunView,
    artifact: LinearModelArtifact,
    transformation: FittedTransformation,
    rows: Vec<HostPartitionRow>,
    expected: Vec<adaq_python_research::model::ForecastRow>,
    replay_identity: String,
}

fn model_qualification_failure_report(
    store: &ModelLabStore,
    user_id: &str,
    decision_id: &str,
    attempt_id: String,
    diagnostic: &str,
) -> Result<ModelRuntimeQualificationReport, PythonResearchError> {
    let diagnostic = bounded_model_diagnostic(diagnostic);
    let decision = store.decision(user_id, decision_id).ok();
    let final_report = store.final_report(user_id, decision_id).ok().flatten();
    let final_state = store.final_evaluation(user_id, decision_id).ok().flatten();
    let selected_attempt_id = final_state
        .as_ref()
        .and_then(|state| state.attempt_id.clone())
        .or_else(|| {
            decision.as_ref().and_then(|decision| {
                store
                    .experiment(user_id, &decision.experiment_id)
                    .ok()
                    .and_then(|experiment| {
                        experiment
                            .trials
                            .iter()
                            .find(|trial| trial.trial_id == decision.selected_trial_id)
                            .and_then(|trial| trial.successful_attempt_id.clone())
                    })
            })
        });
    let run = selected_attempt_id
        .as_deref()
        .and_then(|attempt_id| store.run(user_id, attempt_id).ok());
    let artifact_sha256 = decision
        .as_ref()
        .map(|decision| decision.candidate_artifact_sha256.as_str())
        .filter(|hash| is_sha256_text(hash))
        .unwrap_or(UNKNOWN_MODEL_IDENTITY)
        .to_owned();
    let transformation_sha256 = run
        .as_ref()
        .map(|run| run.transformation_sha256.as_str())
        .filter(|hash| is_sha256_text(hash))
        .unwrap_or(UNKNOWN_MODEL_IDENTITY)
        .to_owned();
    let resource_policy = run
        .as_ref()
        .map(|run| run.resource_policy.clone())
        .unwrap_or_else(default_model_resource_policy);
    let replay_identity =
        sha256(format!("model-qualification-precondition:{decision_id}:{diagnostic}").as_bytes());
    let mut report = ModelRuntimeQualificationReport {
        report_id: String::new(),
        attempt_id,
        decision_id: decision_id.to_owned(),
        final_evaluation_report_id: final_report
            .as_ref()
            .map(|report| report.report_id.clone())
            .unwrap_or_default(),
        artifact_sha256,
        transformation_sha256,
        wasi_profile: WASI_MODEL_PROFILE.into(),
        exporter_id: MODEL_EXPORTER_ID.into(),
        sdk_version: adaq_component_sdk::SDK_VERSION.into(),
        abi_version: adaq_component_sdk::ABI_VERSION.into(),
        package_archive_sha256: None,
        component_id: None,
        component_version: None,
        wasm_sha256: None,
        runtime_identity: MODEL_RUNTIME_IDENTITY.into(),
        resource_policy_sha256: resource_policy_identity(&resource_policy)?,
        qualification_deadline_ms: resource_policy.max_wall_ms,
        qualification_duration_ms: 0,
        input_slots: run
            .as_ref()
            .map(|run| run.input_slots.clone())
            .unwrap_or_default(),
        target_id: MODEL_TARGET_ID.into(),
        target_horizon_bars: MODEL_HORIZON_BARS,
        forecast_contract: adaq_python_research::model::FORECAST_CONTRACT.into(),
        replay_identity,
        replay_rows: run.as_ref().map_or(0, |run| run.final_rows),
        numeric_tolerance: RIDGE_REPEATABILITY_TOLERANCE,
        evidence: ModelQualificationEvidence {
            package: false,
            conformance: false,
            equivalence: false,
            runtime: false,
            qualified: false,
        },
        qualified: false,
        evidence_windows_complete: false,
        imported_component_archive_sha256: None,
        diagnostics: vec![diagnostic],
        created_at_ms: model_lab_now_ms(),
    };
    report.report_id = model_qualification_report_id(
        &report.attempt_id,
        &report.decision_id,
        &report.artifact_sha256,
        "",
        &report.replay_identity,
        false,
    );
    report.validate()?;
    Ok(report)
}

fn accepted_model_deployment_replay(
    store: &ModelLabStore,
    local_state: &crate::local_research::LocalResearchState,
    user_id: &str,
    decision_id: &str,
) -> Result<ModelDeploymentReplay, PythonResearchError> {
    let final_report = store
        .final_report(user_id, decision_id)?
        .ok_or_else(|| PythonResearchError("model-final-evaluation-report-required".into()))?;
    final_report.validate()?;
    if final_report.decision_id != decision_id
        || final_report.evidence_state != EvidenceState::OutOfSample
    {
        return Err(PythonResearchError(
            "model-final-evaluation-not-out-of-sample".into(),
        ));
    }
    let final_state = store
        .final_evaluation(user_id, decision_id)?
        .ok_or_else(|| PythonResearchError("model-final-evaluation-state-required".into()))?;
    if final_state.status != ModelFinalEvaluationStatus::Completed
        || final_state.report_id.as_deref() != Some(final_report.report_id.as_str())
        || final_state.staged_dataset_sha256.as_deref()
            != Some(final_report.forecast_dataset_sha256.as_str())
    {
        return Err(PythonResearchError(
            "model-final-evaluation-state-not-completed".into(),
        ));
    }
    let decision = store.decision(user_id, decision_id)?;
    decision.validate()?;
    let experiment = store.experiment(user_id, &decision.experiment_id)?;
    experiment.validate()?;
    let trial = experiment
        .trials
        .iter()
        .find(|trial| trial.trial_id == decision.selected_trial_id)
        .ok_or_else(|| PythonResearchError("model-selection-trial-not-found".into()))?;
    let successful_attempt_id = trial
        .successful_attempt_id
        .as_deref()
        .ok_or_else(|| PythonResearchError("model-selection-successful-attempt-missing".into()))?;
    let candidate_artifact_sha256 = trial
        .candidate_artifact_sha256
        .as_deref()
        .ok_or_else(|| PythonResearchError("model-selection-candidate-artifact-missing".into()))?;
    let attempt_id = final_state
        .attempt_id
        .as_deref()
        .unwrap_or(successful_attempt_id);
    if trial.status != TrialStatus::Completed
        || trial.repeatability_state != RepeatabilityState::Verified
        || trial.alpha.to_bits() != decision.selected_alpha.to_bits()
        || trial.binding_sha256 != decision.binding_sha256
        || candidate_artifact_sha256 != decision.candidate_artifact_sha256
        || final_report.artifact_sha256 != candidate_artifact_sha256
    {
        return Err(PythonResearchError(
            "model-selection-final-evidence-binding-invalid".into(),
        ));
    }
    let run = store.run(user_id, attempt_id)?;
    if !validate_model_trial_run(&run, trial, attempt_id)
        || run.artifact_sha256 != candidate_artifact_sha256
        || run.seed != decision.seed
        || run.repeatability_state != RepeatabilityState::Verified
        || !run.repeatability_verified
        || !run.test_labels_withheld
        || run.forecast_sha256 != final_report.forecast_dataset_sha256
        || run.input_slots.is_empty()
        || run.target_id != adaq_python_research::model::TARGET_ID
        || run.target_horizon_bars != TARGET_HORIZON_BARS as u32
        || run.forecast_contract != adaq_python_research::model::FORECAST_CONTRACT
        || run.artifact_schema != adaq_python_research::model::LINEAR_MODEL_ARTIFACT_SCHEMA
        || run.numeric_representation != adaq_python_research::model::NUMERIC_REPRESENTATION
        || run.windows != TutorialWindows::m12()
    {
        return Err(PythonResearchError(
            "model-final-run-evidence-binding-invalid".into(),
        ));
    }
    if model_binding_sha256(&run)? != run.binding_sha256 {
        return Err(PythonResearchError(
            "model-final-run-binding-invalid".into(),
        ));
    }
    let artifact =
        LinearModelArtifact::reload(&store.artifact(user_id, candidate_artifact_sha256)?)?;
    let transformation =
        FittedTransformation::reload(&store.transformation(user_id, &run.transformation_sha256)?)?;
    let expected_provenance = model_run_provenance(&run)?;
    if artifact.artifact_sha256 != candidate_artifact_sha256
        || artifact.alpha.to_bits() != decision.selected_alpha.to_bits()
        || artifact.input_slots != run.input_slots
        || artifact.transformation_sha256 != run.transformation_sha256
        || artifact.provenance_hashes != expected_provenance
        || transformation.transformation_sha256 != run.transformation_sha256
        || transformation.feature_names != run.input_slots
    {
        return Err(PythonResearchError(
            "model-final-artifact-transformation-binding-invalid".into(),
        ));
    }
    let factor_binding = local_state
        .factor
        .model_input_binding(user_id, &run.factor_decision_hash)
        .map_err(PythonResearchError)?;
    if run.factor_promotion_protocol_hash != factor_binding.promotion_protocol.protocol_hash
        || run.factor_dataset_id != factor_binding.factor_dataset_id
        || run.feature_dataset_id != factor_binding.feature_dataset_id
        || run.feature_plan_hash != factor_binding.feature_plan_hash
        || run.snapshot_id != factor_binding.snapshot_id
        || run.universe_id != factor_binding.universe_id
        || run.factor_lookback != factor_binding.lookback
    {
        return Err(PythonResearchError(
            "model-factor-input-binding-changed".into(),
        ));
    }
    let promotion_protocol = factor_binding.promotion_protocol.clone();
    let input = ModelInputEvidence {
        decision_hash: factor_binding.decision_hash,
        promotion_protocol_hash: promotion_protocol.protocol_hash.clone(),
        factor_dataset_id: factor_binding.factor_dataset_id,
        feature_dataset_id: factor_binding.feature_dataset_id,
        feature_plan_hash: factor_binding.feature_plan_hash,
        snapshot_id: factor_binding.snapshot_id,
        universe_id: factor_binding.universe_id,
        lookback: factor_binding.lookback,
    };
    if model_input_evidence_hash(&crate::factor_research::FactorModelInputBinding {
        decision_hash: input.decision_hash.clone(),
        promotion_protocol,
        factor_dataset_id: input.factor_dataset_id.clone(),
        feature_dataset_id: input.feature_dataset_id.clone(),
        feature_plan_hash: input.feature_plan_hash.clone(),
        snapshot_id: input.snapshot_id.clone(),
        universe_id: input.universe_id.clone(),
        lookback: input.lookback,
    })? != run.input_evidence_sha256
    {
        return Err(PythonResearchError(
            "model-input-evidence-binding-invalid".into(),
        ));
    }
    let factor_dataset = load_bound_model_factor_dataset(local_state, user_id, &input)?;
    let evidence = build_model_evidence(&input, Some(&factor_dataset))?;
    if evidence.fixture.manifest.content_sha256 != run.fixture_sha256
        || evidence.transformation != transformation
    {
        return Err(PythonResearchError(
            "model-final-research-evidence-changed".into(),
        ));
    }
    let test = evidence.dataset.prepare("test")?;
    if test.labels.is_some() || test.feature_names != artifact.input_slots {
        return Err(PythonResearchError(
            "model-final-test-labels-or-schema-invalid".into(),
        ));
    }
    let mut forecasts = forecast(&artifact, &transformation, &test)?;
    let final_end = run.windows.final_end - TARGET_HORIZON_BARS as u32;
    for row in &mut forecasts {
        if row.datetime as u32 > final_end {
            row.value = None;
            row.unavailable_reason = Some("target-window-boundary".into());
        }
    }
    if model_forecast_sha256(
        &artifact.artifact_sha256,
        &run.input_evidence_sha256,
        &run.snapshot_id,
        &run.universe_id,
        &forecasts,
    )? != run.forecast_sha256
    {
        return Err(PythonResearchError(
            "model-final-forecast-replay-hash-mismatch".into(),
        ));
    }
    let mut rows = Vec::new();
    let mut expected = Vec::new();
    for (row, forecast) in test.rows.iter().zip(forecasts.iter()) {
        if forecast.value.is_some() {
            if row.label.is_some()
                || forecast.unavailable_reason.is_some()
                || forecast.datetime < run.windows.final_start as i64
                || forecast.datetime as u32 > final_end
            {
                return Err(PythonResearchError(
                    "model-final-replay-window-invalid".into(),
                ));
            }
            rows.push(row.clone());
            expected.push(forecast.clone());
        }
    }
    if rows.is_empty() || rows.len() != expected.len() {
        return Err(PythonResearchError("model-final-replay-empty".into()));
    }
    let expected_forecast_sha256 = sha256(
        &serde_json::to_vec(&expected).map_err(|error| PythonResearchError(error.to_string()))?,
    );
    if expected_forecast_sha256 != final_report.forecast_sha256 {
        return Err(PythonResearchError(
            "model-final-forecast-report-hash-mismatch".into(),
        ));
    }
    let replay_identity = sha256(
        &serde_json::to_vec(&(
            &run.artifact_sha256,
            &run.transformation_sha256,
            &run.input_evidence_sha256,
            &rows,
            &expected,
        ))
        .map_err(|error| PythonResearchError(error.to_string()))?,
    );
    Ok(ModelDeploymentReplay {
        final_report,
        run,
        artifact,
        transformation,
        rows,
        expected,
        replay_identity,
    })
}

fn model_qualification_evidence(
    attempt: &QualificationAttempt,
) -> (ModelQualificationEvidence, Vec<String>) {
    let package = !attempt
        .evidence
        .iter()
        .any(|evidence| evidence.gate == QualificationGate::Package);
    let conformance = package
        && !attempt
            .evidence
            .iter()
            .any(|evidence| evidence.gate == QualificationGate::Conformance);
    let equivalence = conformance
        && !attempt
            .evidence
            .iter()
            .any(|evidence| evidence.gate == QualificationGate::Equivalence);
    let runtime = equivalence;
    let qualified = attempt.qualified && package && conformance && equivalence;
    let diagnostics = attempt
        .evidence
        .iter()
        .filter_map(|evidence| {
            evidence
                .diagnostic
                .as_deref()
                .map(|diagnostic| format!("{:?}: {diagnostic}", evidence.gate))
        })
        .map(|diagnostic| bounded_model_diagnostic(&diagnostic))
        .filter(|diagnostic| !diagnostic.is_empty())
        .collect();
    (
        ModelQualificationEvidence {
            package,
            conformance,
            equivalence,
            runtime,
            qualified,
        },
        diagnostics,
    )
}

fn compare_model_component_replay(
    package: &ComponentPackage,
    parameters: &[ComponentParameterValue],
    replay: &ModelDeploymentReplay,
    limits: RunLimits,
) -> Result<(), String> {
    let model_artifact = package
        .manifest
        .model_artifact
        .as_ref()
        .ok_or_else(|| "model component artifact contract is missing".to_owned())?;
    let output = package
        .manifest
        .model_outputs
        .first()
        .ok_or_else(|| "model output contract is missing".to_owned())?;
    let binding = match parameters {
        [ComponentParameterValue::String(binding)] => binding,
        _ => return Err("model component binding parameter is invalid".into()),
    };
    let expected_binding = linear_model_binding(
        &replay.artifact.artifact_sha256,
        &replay.transformation.transformation_sha256,
        &replay.run.input_slots,
        &replay.transformation.means,
        &replay.transformation.scales,
        &replay.artifact.coefficients,
        replay.artifact.intercept,
    );
    let provenance_matches = [
        (
            "sourceArtifactSha256",
            replay.artifact.artifact_sha256.as_str(),
        ),
        (
            "transformationSha256",
            replay.transformation.transformation_sha256.as_str(),
        ),
        ("decision", replay.final_report.decision_id.as_str()),
        (
            "finalEvaluationReport",
            replay.final_report.report_id.as_str(),
        ),
        ("replay", replay.replay_identity.as_str()),
    ]
    .into_iter()
    .all(|(key, value)| {
        model_artifact
            .provenance
            .get(key)
            .is_some_and(|actual| actual == value)
    });
    if package.manifest.kind != ComponentKind::Model
        || package.manifest.model_scope != Some(ModelScope::SingleInstrument)
        || !provenance_matches
        || model_artifact.sha256 != package.manifest.wasm_sha256
        || binding != &expected_binding
        || package.manifest.model_outputs.len() != 1
        || output.name != MODEL_OUTPUT_NAME
        || output.horizon_bars != MODEL_HORIZON_BARS
        || !matches!(&output.prediction_kind, PredictionKind::ExpectedValue)
        || !matches!(
            &output.forecast_target,
            ForecastTarget::Builtin {
                target: BuiltinForecastTarget::FutureCloseReturn
            }
        )
        || !matches!(&output.value_scale, ForecastValueScale::Native)
        || package.manifest.feature_slots.len() != replay.run.input_slots.len()
        || package
            .manifest
            .feature_slots
            .iter()
            .map(|slot| slot.name.as_str())
            .ne(replay.run.input_slots.iter().map(String::as_str))
    {
        return Err("model component contract does not match the selected artifact".into());
    }
    let slots = package
        .manifest
        .feature_slots
        .iter()
        .map(
            |slot| adaq_component_sdk::host::model_abi::exports::adaq::model::api::FeatureSlot {
                name: slot.name.clone(),
            },
        )
        .collect::<Vec<_>>();
    let rows = replay
        .rows
        .iter()
        .map(
            |row| adaq_component_sdk::host::model_abi::exports::adaq::model::api::PredictionRow {
                instrument_id: row.instrument.clone(),
                prediction_time_ms: row.datetime,
                values: row.features.clone(),
            },
        )
        .collect::<Vec<_>>();
    let loader = WasmLoader::with_limits(limits);
    loader.load_model_bytes(&package.wasm, slots, parameters, replay.run.seed)?;
    let actual = loader.process_model(rows)?;
    if actual.len() != replay.expected.len() {
        return Err("model component replay row count diverged".into());
    }
    for (actual, expected) in actual.iter().zip(&replay.expected) {
        let actual = actual
            .as_ref()
            .ok_or_else(|| "model component returned an unavailable forecast".to_owned())?;
        let expected_value = expected
            .value
            .ok_or_else(|| "selected model forecast is unavailable".to_owned())?;
        if actual.instrument_id != expected.instrument
            || actual.prediction_time_ms != expected.datetime
            || actual.values.len() != 1
            || !actual.values[0].is_finite()
            || (actual.values[0] - expected_value).abs() > RIDGE_REPEATABILITY_TOLERANCE
        {
            return Err("model component replay is not equivalent to the selected artifact".into());
        }
    }
    Ok(())
}

fn build_model_qualification_report(
    attempt_id: String,
    replay: &ModelDeploymentReplay,
    package: Option<&ComponentPackage>,
    qualification_deadline_ms: u64,
    qualification_duration_ms: u64,
    mut evidence: ModelQualificationEvidence,
    imported_component_archive_sha256: Option<String>,
    mut diagnostics: Vec<String>,
    qualified: bool,
) -> Result<ModelRuntimeQualificationReport, PythonResearchError> {
    evidence.qualified = qualified;
    if !qualified && diagnostics.is_empty() {
        diagnostics.push("model-runtime-qualification-failed".into());
    }
    diagnostics = diagnostics
        .into_iter()
        .map(|diagnostic| bounded_model_diagnostic(&diagnostic))
        .filter(|diagnostic| !diagnostic.is_empty())
        .collect();
    if !qualified && diagnostics.is_empty() {
        diagnostics.push("model-runtime-qualification-failed".into());
    }
    let package_archive_sha256 = package.map(|package| package.archive_sha256.clone());
    let component_id = package.map(|package| package.manifest.component_id.to_string());
    let component_version = package.map(|package| package.manifest.version.to_string());
    let wasm_sha256 = package.map(|package| package.manifest.wasm_sha256.clone());
    let evidence_windows_complete = package
        .and_then(|package| package.manifest.model_artifact.as_ref())
        .is_some_and(|artifact| has_model_evidence_windows(&artifact.provenance));
    let resource_policy_sha256 = resource_policy_identity(&replay.run.resource_policy)?;
    let mut report = ModelRuntimeQualificationReport {
        report_id: String::new(),
        attempt_id: attempt_id.clone(),
        decision_id: replay.final_report.decision_id.clone(),
        final_evaluation_report_id: replay.final_report.report_id.clone(),
        artifact_sha256: replay.artifact.artifact_sha256.clone(),
        transformation_sha256: replay.transformation.transformation_sha256.clone(),
        wasi_profile: WASI_MODEL_PROFILE.into(),
        exporter_id: MODEL_EXPORTER_ID.into(),
        sdk_version: adaq_component_sdk::SDK_VERSION.into(),
        abi_version: adaq_component_sdk::ABI_VERSION.into(),
        package_archive_sha256,
        component_id,
        component_version,
        wasm_sha256,
        runtime_identity: MODEL_RUNTIME_IDENTITY.into(),
        resource_policy_sha256,
        qualification_deadline_ms,
        qualification_duration_ms,
        input_slots: replay.run.input_slots.clone(),
        target_id: replay.run.target_id.clone(),
        target_horizon_bars: replay.run.target_horizon_bars,
        forecast_contract: replay.run.forecast_contract.clone(),
        replay_identity: replay.replay_identity.clone(),
        replay_rows: replay.rows.len(),
        numeric_tolerance: RIDGE_REPEATABILITY_TOLERANCE,
        evidence,
        qualified,
        evidence_windows_complete,
        imported_component_archive_sha256,
        diagnostics,
        created_at_ms: model_lab_now_ms(),
    };
    report.report_id = model_qualification_report_id(
        &report.attempt_id,
        &report.decision_id,
        &report.artifact_sha256,
        report.package_archive_sha256.as_deref().unwrap_or_default(),
        &report.replay_identity,
        report.qualified,
    );
    report.validate()?;
    Ok(report)
}

fn has_model_evidence_windows(provenance: &BTreeMap<String, String>) -> bool {
    ["trainingWindow", "fittingWindow", "normalizationWindow"]
        .iter()
        .all(|field| {
            provenance.get(*field).is_some_and(|value| {
                let Some((start, end)) = value.split_once("..") else {
                    return false;
                };
                match (start.parse::<i64>(), end.parse::<i64>()) {
                    (Ok(start), Ok(end)) => start <= end,
                    _ => false,
                }
            })
        })
}

fn qualify_model_deployment(
    store: &ModelLabStore,
    local_state: &crate::local_research::LocalResearchState,
    user_id: &str,
    decision_id: &str,
) -> Result<ModelRuntimeQualificationReport, PythonResearchError> {
    if let Some(report) = store
        .qualification_reports(user_id, decision_id)?
        .into_iter()
        .rev()
        .find(|report| report.qualified)
    {
        let has_windows = report
            .package_archive_sha256
            .as_deref()
            .and_then(|archive| {
                local_state
                    .components
                    .package_for_user(user_id, archive)
                    .ok()
            })
            .and_then(|package| package.manifest.model_artifact)
            .is_some_and(|artifact| has_model_evidence_windows(&artifact.provenance));
        if has_windows {
            return Ok(report);
        }
    }
    let attempt_id = uuid::Uuid::new_v4().to_string();
    let replay = match accepted_model_deployment_replay(store, local_state, user_id, decision_id) {
        Ok(replay) => replay,
        Err(error) => {
            let report = model_qualification_failure_report(
                store,
                user_id,
                decision_id,
                attempt_id,
                &error.to_string(),
            )?;
            return store.save_qualification_report(user_id, report);
        }
    };
    let started = Instant::now();
    let provenance = {
        let mut provenance = replay.artifact.provenance_hashes.clone();
        provenance.insert("decision".into(), replay.final_report.decision_id.clone());
        provenance.insert(
            "finalEvaluationReport".into(),
            replay.final_report.report_id.clone(),
        );
        provenance.insert("replay".into(), replay.replay_identity.clone());
        provenance.insert(
            "resourcePolicy".into(),
            resource_policy_identity(&replay.run.resource_policy)?,
        );
        // The embedded Linear Model Artifact keeps hash-only provenance; its
        // package manifest carries the native research windows for downstream
        // evidence classification.
        let training_window = format!(
            "{}..{}",
            replay.run.windows.train_start, replay.run.windows.train_end
        );
        for field in ["trainingWindow", "fittingWindow", "normalizationWindow"] {
            provenance.insert(field.into(), training_window.clone());
        }
        provenance
    };
    let package_bytes = match export_linear_model_component(
        &replay.artifact.artifact_sha256,
        &replay.transformation.transformation_sha256,
        &replay.artifact.input_slots,
        &replay.transformation.means,
        &replay.transformation.scales,
        &replay.artifact.coefficients,
        replay.artifact.intercept,
        provenance,
    ) {
        Ok(bytes) => bytes,
        Err(error) => {
            let report = build_model_qualification_report(
                attempt_id,
                &replay,
                None,
                replay.run.resource_policy.max_wall_ms,
                started.elapsed().as_millis().min(u64::MAX as u128) as u64,
                ModelQualificationEvidence {
                    package: false,
                    conformance: false,
                    equivalence: false,
                    runtime: false,
                    qualified: false,
                },
                None,
                vec![error],
                false,
            )?;
            return store.save_qualification_report(user_id, report);
        }
    };
    let package = match ComponentPackage::read(&package_bytes) {
        Ok(package) => package,
        Err(error) => {
            let report = build_model_qualification_report(
                attempt_id,
                &replay,
                None,
                replay.run.resource_policy.max_wall_ms,
                started.elapsed().as_millis().min(u64::MAX as u128) as u64,
                ModelQualificationEvidence {
                    package: false,
                    conformance: false,
                    equivalence: false,
                    runtime: false,
                    qualified: false,
                },
                None,
                vec![format!("model-export-package-invalid:{error}")],
                false,
            )?;
            return store.save_qualification_report(user_id, report);
        }
    };
    let limits = match model_run_limits(&replay.run.resource_policy) {
        Ok(limits) => limits,
        Err(error) => {
            let report = build_model_qualification_report(
                attempt_id,
                &replay,
                Some(&package),
                replay.run.resource_policy.max_wall_ms,
                started.elapsed().as_millis().min(u64::MAX as u128) as u64,
                ModelQualificationEvidence {
                    package: true,
                    conformance: false,
                    equivalence: false,
                    runtime: false,
                    qualified: false,
                },
                None,
                vec![error],
                false,
            )?;
            return store.save_qualification_report(user_id, report);
        }
    };
    let qualification_deadline = Duration::from_millis(replay.run.resource_policy.max_wall_ms);
    let qualification = qualify_package_with_limits(
        attempt_id.clone(),
        &package_bytes,
        limits,
        |package, parameters| {
            if started.elapsed() > qualification_deadline {
                return Err("model-runtime-qualification-deadline-exceeded".into());
            }
            compare_model_component_replay(package, parameters, &replay, limits)
        },
    );
    let (mut evidence, mut diagnostics) = model_qualification_evidence(&qualification);
    let mut qualified = evidence.qualified;
    let mut imported_component_archive_sha256 = None;
    let qualification_deadline_ms = replay.run.resource_policy.max_wall_ms;
    let qualification_duration_ms = started.elapsed().as_millis().min(u64::MAX as u128) as u64;
    if qualified {
        if qualification_duration_ms > qualification_deadline_ms {
            qualified = false;
            evidence.runtime = false;
            diagnostics.push("model-runtime-qualification-deadline-exceeded".into());
        } else {
            match local_state.components.import(user_id, &package_bytes) {
                Ok(_) => {
                    imported_component_archive_sha256 = Some(package.archive_sha256.clone());
                }
                Err(error) => {
                    qualified = false;
                    diagnostics.push(error);
                }
            }
        }
    }
    let report = build_model_qualification_report(
        attempt_id,
        &replay,
        Some(&package),
        qualification_deadline_ms,
        qualification_duration_ms,
        evidence,
        imported_component_archive_sha256,
        diagnostics,
        qualified,
    )?;
    store.save_qualification_report(user_id, report)
}

impl ModelEvidenceData {
    fn runner_input(&self) -> Result<ModelRunnerInput, PythonResearchError> {
        let test = self.dataset.raw_rows("test")?;
        Ok(ModelRunnerInput {
            train: self.dataset.raw_rows("train")?,
            valid: self.dataset.raw_rows("valid")?,
            test,
            train_labels: self.dataset.target_labels("train")?,
            valid_labels: self.dataset.target_labels("valid")?,
            transformation: self.transformation.clone(),
            fitted_model: None,
            target_window_end: None,
        })
    }
}

fn build_model_evidence(
    input: &ModelInputEvidence,
    factor_dataset: Option<&FactorDataset>,
) -> Result<ModelEvidenceData, PythonResearchError> {
    let fixture = SyntheticTutorialFixture::m12()?;
    fixture.validate()?;
    let windows = TutorialWindows::m12();
    windows.validate()?;
    let factor_values = match factor_dataset {
        Some(dataset)
            if dataset.manifest.dataset_id == input.factor_dataset_id
                && dataset.manifest.feature_dataset_id == input.feature_dataset_id
                && dataset.manifest.feature_plan_hash == input.feature_plan_hash
                && dataset.manifest.market_data_snapshot_id == input.snapshot_id
                && dataset.manifest.point_in_time_universe_id == input.universe_id
                && dataset
                    .manifest
                    .output_names
                    .iter()
                    .any(|name| name == "momentum-score") =>
        {
            dataset
                .rows
                .iter()
                .filter_map(|row| match row.values.get("momentum-score") {
                    Some(FactorObservationValue::Available { value, .. }) => {
                        Some(((row.instrument_id.clone(), row.observation_time_ms), *value))
                    }
                    _ => None,
                })
                .collect::<BTreeMap<_, _>>()
        }
        Some(_) => {
            return Err(PythonResearchError(
                "model-factor-dataset-binding-invalid".into(),
            ));
        }
        None => materialize_momentum(
            &fixture.momentum_rows(),
            &fixture.instruments,
            input.lookback,
        )?
        .into_iter()
        .filter_map(|row| {
            row.value
                .map(|value| ((row.instrument_id, row.observation_time_ms), value))
        })
        .collect::<BTreeMap<_, _>>(),
    };
    let mut closes = fixture
        .instruments
        .iter()
        .map(|instrument| (instrument.clone(), vec![0.0; TUTORIAL_SESSION_COUNT + 1]))
        .collect::<BTreeMap<_, _>>();
    for bar in &fixture.bars {
        closes
            .get_mut(&bar.instrument)
            .ok_or_else(|| PythonResearchError("tutorial-fixture-instrument-missing".into()))?
            [bar.session as usize] = bar.close;
    }
    let partition_rows = |start: u32, end: u32, target_end: u32, labels_visible: bool| {
        let mut rows = Vec::new();
        for session in start..=end {
            for instrument in &fixture.instruments {
                let Some(feature) = factor_values.get(&(instrument.clone(), session as i64)) else {
                    continue;
                };
                let label = if labels_visible {
                    match future_close_return_state(&closes[instrument], session, target_end) {
                        adaq_python_research::model::TargetValue::Available(value) => Some(value),
                        adaq_python_research::model::TargetValue::Unavailable(_) => None,
                    }
                } else {
                    None
                };
                rows.push(HostPartitionRow {
                    datetime: session as i64,
                    instrument: instrument.clone(),
                    features: vec![*feature],
                    label,
                });
            }
        }
        Ok::<Vec<HostPartitionRow>, PythonResearchError>(rows)
    };
    let train_rows = partition_rows(
        windows.train_start,
        windows.train_end,
        windows.train_end,
        true,
    )?;
    let selection_rows = partition_rows(
        windows.selection_start,
        windows.selection_end,
        windows.selection_end,
        true,
    )?;
    let final_rows = partition_rows(
        windows.final_start,
        windows.final_end,
        windows.final_end,
        false,
    )?;
    let final_labels = final_rows
        .iter()
        .filter_map(|row| {
            match future_close_return_state(
                &closes[&row.instrument],
                row.datetime as u32,
                windows.final_end,
            ) {
                adaq_python_research::model::TargetValue::Available(label) => {
                    Some((row.datetime, row.instrument.clone(), label))
                }
                adaq_python_research::model::TargetValue::Unavailable(_) => None,
            }
        })
        .collect::<Vec<_>>();
    let dataset = DatasetH::new(vec![
        HostPartition {
            name: PartitionName::Train,
            feature_names: vec!["momentum-score".into()],
            rows: train_rows,
            labels_visible: true,
        },
        HostPartition {
            name: PartitionName::SelectionValidation,
            feature_names: vec!["momentum-score".into()],
            rows: selection_rows,
            labels_visible: true,
        },
        HostPartition {
            name: PartitionName::Test,
            feature_names: vec!["momentum-score".into()],
            rows: final_rows,
            labels_visible: false,
        },
    ])?;
    let train = dataset.prepare("train")?;
    let transformation = FittedTransformation::fit(&train.rows, &train.feature_names)?;
    Ok(ModelEvidenceData {
        fixture,
        dataset,
        transformation,
        final_labels,
    })
}

fn model_input_evidence_hash(
    binding: &crate::factor_research::FactorModelInputBinding,
) -> Result<String, PythonResearchError> {
    Ok(sha256(
        &serde_json::to_vec(&(
            &binding.decision_hash,
            &binding.promotion_protocol.protocol_hash,
            &binding.factor_dataset_id,
            &binding.feature_dataset_id,
            &binding.feature_plan_hash,
            &binding.snapshot_id,
            &binding.universe_id,
            &binding.lookback,
        ))
        .map_err(|error| PythonResearchError(error.to_string()))?,
    ))
}

fn load_bound_model_factor_dataset(
    local_state: &crate::local_research::LocalResearchState,
    user_id: &str,
    input: &ModelInputEvidence,
) -> Result<FactorDataset, PythonResearchError> {
    let factor_dataset = local_state
        .factor
        .get_factor_dataset(user_id, &input.factor_dataset_id)
        .map_err(PythonResearchError)?;
    let feature_store = local_state.features.materialization_store();
    let feature_dataset = crate::features::Features::completed_dataset_from_store(
        &feature_store,
        user_id,
        &input.feature_dataset_id,
    )
    .map_err(PythonResearchError)?;
    if factor_dataset.manifest.feature_dataset_id != feature_dataset.dataset_id
        || factor_dataset.manifest.feature_plan_hash != feature_dataset.feature_plan_hash
        || factor_dataset.manifest.market_data_snapshot_id
            != feature_dataset.market_data_snapshot_id
        || factor_dataset.manifest.point_in_time_universe_id
            != feature_dataset.point_in_time_universe_id
        || feature_dataset.feature_plan_hash != input.feature_plan_hash
        || feature_dataset.market_data_snapshot_id != input.snapshot_id
        || feature_dataset.point_in_time_universe_id != input.universe_id
    {
        return Err(PythonResearchError(
            "model-feature-dataset-binding-invalid".into(),
        ));
    }
    Ok(factor_dataset)
}

fn validate_model_process_replay(
    first: &RunnerExecution,
    replay: &RunnerExecution,
) -> Result<String, PythonResearchError> {
    let first_result = first
        .conformance
        .as_ref()
        .ok_or_else(|| PythonResearchError("model-runner-result-missing".into()))?;
    let replay_result = replay
        .conformance
        .as_ref()
        .ok_or_else(|| PythonResearchError("model-runner-replay-result-missing".into()))?;
    if first_result.project_id != MODEL_PROJECT_ID
        || replay_result.project_id != MODEL_PROJECT_ID
        || first_result.project_kind != "model"
        || replay_result.project_kind != "model"
        || first_result.entry_point != replay_result.entry_point
        || first_result.payload != replay_result.payload
    {
        return Err(PythonResearchError("model-process-replay-divergent".into()));
    }
    Ok(first_result.attempt_id.clone())
}

fn model_replay_resource_policy(
    attempt_store: &AttemptStore,
    user_id: &str,
    first: &RunnerExecution,
    replay: &HostResourcePolicy,
) -> Result<(HostResourcePolicy, String), PythonResearchError> {
    let first_id = first
        .conformance
        .as_ref()
        .map(|result| result.attempt_id.as_str())
        .ok_or_else(|| PythonResearchError("model-runner-result-missing".into()))?;
    let first_attempt = attempt_store.get(first_id)?;
    if first_attempt.user_id != user_id || first_attempt.resource_policy != *replay {
        return Err(PythonResearchError(
            "model-resource-policy-replay-divergent".into(),
        ));
    }
    let resource_policy = first_attempt.resource_policy;
    let identity = resource_policy_identity(&resource_policy)?;
    Ok((resource_policy, identity))
}

fn discard_verification_artifact(
    root: &Path,
    execution: &RunnerExecution,
) -> Result<(), PythonResearchError> {
    let Some(attempt_id) = execution
        .conformance
        .as_ref()
        .map(|result| result.attempt_id.as_str())
    else {
        return Ok(());
    };
    match fs::remove_file(
        root.join("attempt-results")
            .join(format!("{attempt_id}.artifact")),
    ) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(PythonResearchError(error.to_string())),
    }
}

fn demo_model_run_with_evidence(
    alpha: f64,
    project_revision_sha256: String,
    environment_sha256: String,
    input_evidence_sha256: String,
    input: ModelInputEvidence,
    resource_policy: HostResourcePolicy,
    python_artifact: Option<adaq_python_research::model::LinearModelArtifact>,
    factor_dataset: Option<&FactorDataset>,
) -> Result<DemoModelRun, PythonResearchError> {
    let evidence = build_model_evidence(&input, factor_dataset)?;
    let fixture = evidence.fixture;
    let dataset = evidence.dataset;
    let transformation = evidence.transformation;
    let final_labels = evidence.final_labels;
    let windows = TutorialWindows::m12();
    windows.validate()?;
    let train = dataset.prepare("train")?;
    let adapter = RidgeAdapter::registered(alpha)?;
    let artifact = python_artifact.unwrap_or(adapter.fit(
        &dataset,
        &transformation,
        model_provenance(
            &fixture.manifest.content_sha256,
            &project_revision_sha256,
            &environment_sha256,
            &input_evidence_sha256,
            &resource_policy_identity(&resource_policy)?,
            &input,
        ),
    )?);
    let transformation = FittedTransformation::reload(&transformation.to_bytes()?)?;
    let artifact = adaq_python_research::model::LinearModelArtifact::reload(&artifact.to_bytes()?)?;
    let test = dataset.prepare("test")?;
    let mut forecasts = forecast(&artifact, &transformation, &test)?;
    for row in &mut forecasts {
        if row.datetime as u32 > windows.final_end - TARGET_HORIZON_BARS as u32 {
            row.value = None;
            row.unavailable_reason = Some("target-window-boundary".into());
        }
    }
    let selection = dataset.prepare("valid")?;
    let selection_forecasts = forecast(&artifact, &transformation, &selection)?;
    let selection_labels = selection
        .labels
        .as_ref()
        .ok_or_else(|| PythonResearchError("ridge-selection-labels-unavailable".into()))?;
    if selection_labels.len() != selection_forecasts.len() {
        return Err(PythonResearchError(
            "ridge-selection-label-count-invalid".into(),
        ));
    }
    let selection_metric = selection_forecasts
        .iter()
        .zip(selection_labels)
        .map(|(forecast, label)| {
            let value = forecast.value.ok_or_else(|| {
                PythonResearchError("ridge-selection-forecast-unavailable".into())
            })?;
            Ok((value - label).powi(2))
        })
        .collect::<Result<Vec<_>, PythonResearchError>>()?;
    let selection_metric = (!selection_metric.is_empty())
        .then(|| selection_metric.iter().sum::<f64>() / selection_metric.len() as f64)
        .filter(|value| value.is_finite())
        .ok_or_else(|| PythonResearchError("ridge-selection-metric-invalid".into()))?;
    let forecast_sha256 = model_forecast_sha256(
        &artifact.artifact_sha256,
        &input_evidence_sha256,
        &input.snapshot_id,
        &input.universe_id,
        &forecasts,
    )?;
    let mut view = ModelRunView {
        attempt_id: String::new(),
        adapter_id: adaq_python_research::model::RIDGE_ADAPTER_ID.into(),
        alpha,
        project_revision_sha256,
        environment_sha256,
        input_evidence_sha256,
        binding_sha256: String::new(),
        factor_decision_hash: input.decision_hash,
        factor_promotion_protocol_hash: input.promotion_protocol_hash,
        factor_dataset_id: input.factor_dataset_id,
        feature_dataset_id: input.feature_dataset_id,
        feature_plan_hash: input.feature_plan_hash,
        snapshot_id: input.snapshot_id,
        universe_id: input.universe_id,
        factor_lookback: input.lookback,
        seed: 7,
        fixture_sha256: fixture.manifest.content_sha256,
        artifact_sha256: artifact.artifact_sha256.clone(),
        transformation_sha256: transformation.transformation_sha256.clone(),
        forecast_sha256,
        train_rows: train.rows.len(),
        selection_rows: selection.rows.len(),
        selection_metric: Some(selection_metric),
        final_rows: test.rows.len(),
        test_labels_withheld: test.labels.is_none(),
        repeatability_verified: false,
        repeatability_tolerance: RIDGE_REPEATABILITY_TOLERANCE,
        repeatability_state: RepeatabilityState::Unverified,
        evidence_state: EvidenceState::Unknown,
        diagnostics: Vec::new(),
        windows,
        resource_policy,
        input_slots: transformation.feature_names.clone(),
        target_id: adaq_python_research::model::TARGET_ID.into(),
        target_horizon_bars: TARGET_HORIZON_BARS as u32,
        forecast_contract: adaq_python_research::model::FORECAST_CONTRACT.into(),
        artifact_schema: adaq_python_research::model::LINEAR_MODEL_ARTIFACT_SCHEMA.into(),
        numeric_representation: adaq_python_research::model::NUMERIC_REPRESENTATION.into(),
    };
    view.binding_sha256 = model_binding_sha256(&view)?;
    Ok(DemoModelRun {
        view,
        artifact,
        transformation,
        forecasts,
        final_labels,
    })
}

fn demo_factor_run_with_outputs(
    first_outputs: Option<&BTreeMap<u32, Vec<MomentumOutputRow>>>,
    replay_outputs: Option<&BTreeMap<u32, Vec<MomentumOutputRow>>>,
    process_replay_exact: bool,
) -> Result<FactorRunView, PythonResearchError> {
    let fixture = SyntheticTutorialFixture::m12()?;
    fixture.validate()?;
    let input = fixture.momentum_rows();
    let lookbacks = expand_momentum_grid();
    let mut available_rows = BTreeMap::new();
    let mut repeatability = BTreeMap::new();
    for lookback in &lookbacks {
        let first = first_outputs
            .and_then(|outputs| outputs.get(lookback))
            .cloned()
            .unwrap_or(materialize_momentum(
                &input,
                &fixture.instruments,
                *lookback,
            )?);
        let replay = replay_outputs
            .and_then(|outputs| outputs.get(lookback))
            .cloned()
            .unwrap_or(materialize_momentum(
                &input,
                &fixture.instruments,
                *lookback,
            )?);
        let key = lookback.to_string();
        available_rows.insert(
            key.clone(),
            first.iter().filter(|row| row.value.is_some()).count(),
        );
        let mut report = RepeatabilityReport::exact(&first, &replay)?;
        report.exact &= process_replay_exact;
        repeatability.insert(key, report);
    }
    Ok(FactorRunView {
        attempt_id: String::new(),
        candidate_hash: None,
        family_id: None,
        trial_ids: Vec::new(),
        dataset_ids: Vec::new(),
        report_hashes: Vec::new(),
        promotion_policy_hash: None,
        promotion_protocol_hash: None,
        promotion_decision_hash: None,
        selected_trial_id: None,
        selection_hash: None,
        promotion_state: None,
        project_id: "py-factor-cross-sectional-momentum".into(),
        project_revision_sha256: None,
        environment_sha256: None,
        fixture_sha256: fixture.manifest.content_sha256,
        input_bindings: None,
        normalized_parameters: None,
        seed: None,
        sdk_artifact_sha256: None,
        resource_policy: None,
        snapshot_id: None,
        snapshot_bindings: None,
        point_in_time_universe_id: None,
        feature_dataset_id: None,
        feature_dataset_bindings: None,
        feature_evidence_sha256: None,
        feature_plan_hash: None,
        engine_identity: None,
        repeatability_report_sha256: None,
        repeatability_verified: false,
        logs: Vec::new(),
        lookbacks,
        default_lookback: 20,
        rows_per_trial: input.len(),
        available_rows,
        repeatability,
        repeatability_report: None,
        synthetic: true,
        selection_required: true,
        promotion_required: true,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FactorEvidenceRun {
    candidate_hash: String,
    family_id: uuid::Uuid,
    trial_ids: Vec<uuid::Uuid>,
    dataset_ids: Vec<String>,
    report_hashes: Vec<String>,
    promotion_protocol: PromotionProtocol,
    promotion_protocols: BTreeMap<String, PromotionProtocol>,
    policy: PromotionPolicy,
}

fn wait_for_factor_attempt(
    factor: &crate::factor_research::FactorResearch,
    user_id: &str,
    attempt_id: &str,
) -> Result<String, PythonResearchError> {
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    loop {
        let attempt = factor
            .get_attempt(crate::factor_research::FactorAttemptRequest {
                user_id: user_id.into(),
                attempt_id: attempt_id.into(),
            })
            .map_err(PythonResearchError)?;
        match attempt.status {
            adaq_factor_research::AttemptStatus::Completed => {
                return attempt
                    .result_id
                    .ok_or_else(|| PythonResearchError("factor-attempt-result-missing".into()));
            }
            adaq_factor_research::AttemptStatus::Failed
            | adaq_factor_research::AttemptStatus::Cancelled => {
                return Err(PythonResearchError(format!(
                    "factor-attempt-terminal-without-result:{}",
                    attempt.diagnostic.unwrap_or_else(|| "unknown".into())
                )));
            }
            adaq_factor_research::AttemptStatus::Pending
            | adaq_factor_research::AttemptStatus::Running => {}
            adaq_factor_research::AttemptStatus::Interrupted
            | adaq_factor_research::AttemptStatus::Stale => {
                return Err(PythonResearchError(
                    "Factor research Attempt is no longer retryable".into(),
                ));
            }
        }
        if std::time::Instant::now() >= deadline {
            return Err(PythonResearchError("factor-attempt-timeout".into()));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn factor_engine_identity(
    revision_sha256: &str,
    environment_sha256: &str,
    fixture_sha256: &str,
    lookback: u32,
) -> ResearchEngineProvenance {
    ResearchEngineProvenance {
        engine_id: "adaq-python-factor".into(),
        engine_version: "1".into(),
        adapter: "adaq-python@1".into(),
        target_triple: "host".into(),
        build_id: sha256(b"adaq-python-factor@1"),
        environment: BTreeMap::from([
            ("projectRevisionSha256".into(), revision_sha256.into()),
            ("environmentSha256".into(), environment_sha256.into()),
        ]),
        parameters: BTreeMap::from([("lookback".into(), lookback.to_string())]),
        input_identities: vec![
            fixture_sha256.into(),
            revision_sha256.into(),
            environment_sha256.into(),
        ],
    }
}

fn factor_market_context(universe_id: &str) -> FactorMarketContext {
    FactorMarketContext {
        venue: "synthetic".into(),
        asset_class: "equity".into(),
        bar_interval: "1d".into(),
        price_basis: "close".into(),
        valuation_currency: "USD".into(),
        point_in_time_universe_id: universe_id.into(),
    }
}

fn factor_repeatability_reports(
    first_outputs: &BTreeMap<u32, Vec<MomentumOutputRow>>,
    replay_outputs: &BTreeMap<u32, Vec<MomentumOutputRow>>,
    process_evidence: &BTreeMap<u32, (RunnerProcessEvidence, RunnerProcessEvidence)>,
    process_replay_exact: bool,
    mode: PythonFactorMode,
) -> Result<BTreeMap<String, PythonRepeatabilityReport>, PythonResearchError> {
    let mut reports = BTreeMap::new();
    for lookback in expand_momentum_grid() {
        let first = first_outputs
            .get(&lookback)
            .cloned()
            .ok_or_else(|| PythonResearchError("python-factor-first-output-missing".into()))?;
        let replay = replay_outputs
            .get(&lookback)
            .cloned()
            .ok_or_else(|| PythonResearchError("python-factor-replay-output-missing".into()))?;
        let output_report = RepeatabilityReport::exact(&first, &replay)?;
        let (first_evidence, replay_evidence) = process_evidence
            .get(&lookback)
            .cloned()
            .ok_or_else(|| PythonResearchError("python-factor-process-evidence-missing".into()))?;
        reports.insert(
            lookback.to_string(),
            PythonRepeatabilityReport {
                first_attempt_id: first_evidence.attempt_id,
                replay_attempt_id: replay_evidence.attempt_id,
                first_process_sha256: first_evidence.process_sha256,
                replay_process_sha256: replay_evidence.process_sha256,
                process_contract_sha256: first_evidence.contract_sha256.clone(),
                first_input_sha256: first_evidence.input_sha256.clone(),
                replay_input_sha256: replay_evidence.input_sha256.clone(),
                first_output_sha256: output_report.first_output_sha256,
                replay_output_sha256: output_report.replay_output_sha256,
                exact: output_report.exact
                    && first_evidence.contract_sha256 == replay_evidence.contract_sha256
                    && first_evidence.input_sha256 == replay_evidence.input_sha256
                    && process_replay_exact,
                partitions: match mode {
                    PythonFactorMode::ImperativePython => vec![
                        "fresh-process".into(),
                        "single-batch".into(),
                        "split-batch".into(),
                    ],
                    PythonFactorMode::PortableDefinition => {
                        vec!["fresh-process".into(), "portable-definition".into()]
                    }
                },
            },
        );
    }
    Ok(reports)
}

#[derive(Debug, Clone)]
struct RunnerProcessEvidence {
    attempt_id: String,
    process_sha256: String,
    contract_sha256: String,
    input_sha256: String,
}

fn python_factor_resource_policy(policy: &HostResourcePolicy) -> PythonFactorResourcePolicy {
    PythonFactorResourcePolicy {
        policy_id: policy.policy_id.clone(),
        max_wall_ms: policy.max_wall_ms,
        max_memory_bytes: policy.max_memory_bytes,
        max_threads: policy.max_threads,
        max_processes: policy.max_processes,
        max_input_rows: policy.max_input_rows,
        max_input_columns: policy.max_input_columns,
        max_input_cells: policy.max_input_cells,
        max_output_rows: policy.max_output_rows,
        max_control_bytes: policy.max_control_bytes,
        max_arrow_bytes: policy.max_arrow_bytes,
        max_staged_bytes: policy.max_staged_bytes,
        max_artifact_bytes: policy.max_artifact_bytes,
        max_checkpoint_bytes: policy.max_checkpoint_bytes,
        max_log_bytes: policy.max_log_bytes,
    }
}

fn factor_dataset_input_from_output(
    protocol: &adaq_factor_research::FactorMaterializationProtocol,
    mut output: Vec<MomentumOutputRow>,
) -> Result<FactorDatasetInput, PythonResearchError> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Payload<'a> {
        output_names: &'a [String],
        rows: &'a [FactorDatasetRow],
    }

    let output_names = vec!["momentum-score".into()];
    output.sort_by(|left, right| {
        (left.instrument_id.as_str(), left.observation_time_ms)
            .cmp(&(right.instrument_id.as_str(), right.observation_time_ms))
    });
    let rows = output
        .into_iter()
        .map(|row| {
            let value = match row.value {
                Some(value) => FactorObservationValue::Available {
                    value,
                    available_at_ms: row.observation_time_ms,
                },
                None => FactorObservationValue::Unavailable {
                    reason: match row
                        .unavailable_reason
                        .unwrap_or(FactorUnavailableReason::MissingInput)
                    {
                        FactorUnavailableReason::Warmup => {
                            adaq_factor_research::FactorUnavailabilityReason::Warmup
                        }
                        FactorUnavailableReason::MissingInput => {
                            adaq_factor_research::FactorUnavailabilityReason::MissingInput
                        }
                        FactorUnavailableReason::BarGap => {
                            adaq_factor_research::FactorUnavailabilityReason::BarGap
                        }
                    },
                },
            };
            FactorDatasetRow {
                instrument_id: row.instrument_id,
                observation_time_ms: row.observation_time_ms,
                values: BTreeMap::from([("momentum-score".into(), value)]),
            }
        })
        .collect::<Vec<_>>();
    let payload_sha256 = sha256(
        &serde_json::to_vec(&Payload {
            output_names: &output_names,
            rows: &rows,
        })
        .map_err(|error| PythonResearchError(error.to_string()))?,
    );
    let mut manifest = FactorDatasetManifest {
        schema_version: adaq_factor_research::FACTOR_RESEARCH_SCHEMA_VERSION.into(),
        dataset_id: String::new(),
        protocol_hash: protocol.protocol_hash.clone(),
        candidate_hash: protocol.candidate_hash.clone(),
        scope: FactorScope::CrossSectional,
        feature_dataset_id: protocol.feature_dataset_id.clone(),
        feature_plan_hash: protocol.feature_plan_hash.clone(),
        market_data_snapshot_id: protocol.market_data_snapshot_id.clone(),
        point_in_time_universe_id: protocol.point_in_time_universe_id.clone(),
        observation_range: Some(protocol.observation_range.clone()),
        market_context: protocol.market_context.clone(),
        output_names,
        observation_count: rows.len() as u64,
        payload_sha256,
        engine_identity: protocol.engine_identity.clone(),
    };
    manifest.dataset_id = manifest
        .content_id()
        .map_err(|error| PythonResearchError(error.to_string()))?;
    Ok(FactorDatasetInput { manifest, rows })
}

#[derive(Debug, Clone)]
struct FactorFeatureEvidence {
    snapshot_id: String,
    dataset_id: String,
    snapshot_bindings: BTreeMap<String, String>,
    dataset_bindings: BTreeMap<String, String>,
    plan_hash: String,
    evidence_sha256: String,
}

fn factor_feature_plan() -> Result<FeaturePlan, PythonResearchError> {
    let definition = FeatureDefinition::freeze(DefinitionDraft {
        definition_id: uuid::Uuid::from_u128(0x6d120101000000000000000000000006),
        revision: 1,
        scope: FeatureScope::TimeSeries,
        nodes: vec![FeatureNode {
            id: "close-feature".into(),
            operator: FeatureOperator::BackwardSimpleReturn,
            scope: FeatureScope::TimeSeries,
            inputs: vec![FeatureInput::Market {
                field: "close".into(),
            }],
            parameters: BTreeMap::from([("period".into(), serde_json::json!(1))]),
            warmup_bars: 1,
        }],
        outputs: vec![FeatureOutput {
            name: "close-feature".into(),
            node_id: "close-feature".into(),
        }],
    })
    .map_err(|error| PythonResearchError(format!("feature-plan-invalid:{error:?}")))?;
    FeaturePlan::freeze(FeaturePlanDraft {
        definitions: vec![definition],
        engine_identity: FeatureEngineIdentity::native()
            .map_err(|error| PythonResearchError(error.to_string()))?,
        ..FeaturePlanDraft::default()
    })
    .map_err(|error| PythonResearchError(format!("feature-plan-invalid:{error:?}")))
}

fn materialize_factor_feature_instrument(
    local_state: &crate::local_research::LocalResearchState,
    request: &FactorRunRequest,
    fixture: &SyntheticTutorialFixture,
    instrument: &str,
    plan: &FeaturePlan,
    plan_hash: &str,
    point_in_time_universe_id: &str,
) -> Result<(String, String), PythonResearchError> {
    let bars = fixture
        .bars
        .iter()
        .filter(|bar| bar.instrument == instrument)
        .map(|bar| {
            let close = Decimal::from_f64_retain(bar.close)
                .ok_or_else(|| PythonResearchError("synthetic-price-invalid".into()))?;
            Ok(OhlcvBar {
                open_time_ms: bar.session as i64,
                open: close,
                high: close,
                low: close,
                close,
                base_volume: Decimal::ONE,
                quote_volume: close,
            })
        })
        .collect::<Result<Vec<_>, PythonResearchError>>()?;
    let snapshot = local_state
        .persist_snapshot_for_user(
            &request.user_id,
            &BarSeries {
                src: "synthetic".into(),
                code: instrument.into(),
                interval: BarInterval::OneDay,
                bars,
                gaps: Vec::new(),
            },
        )
        .map_err(PythonResearchError)?;
    let materialization_request = FeatureMaterializationRequest::new(
        &request.user_id,
        plan_hash,
        &snapshot.snapshot_id,
        point_in_time_universe_id,
        ObservationRange {
            start_time_ms: 1,
            end_time_ms: TUTORIAL_SESSION_COUNT as i64,
        },
        BTreeMap::new(),
        7,
    )
    .map_err(|error| PythonResearchError(error.to_string()))?;
    let attempt = local_state
        .features
        .start_materialization(FeatureMaterializationStartRequest {
            user_id: request.user_id.clone(),
            request: materialization_request,
            plan: FeaturePlanDraft {
                definitions: plan.definitions().to_vec(),
                ..FeaturePlanDraft::default()
            },
        })
        .map_err(PythonResearchError)?;
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        let current = local_state
            .features
            .get_materialization_attempt(FeatureAttemptRequest {
                user_id: request.user_id.clone(),
                attempt_id: attempt.attempt_id.clone(),
            })
            .map_err(PythonResearchError)?;
        match current.status {
            MaterializationAttemptStatus::Completed => {
                let dataset_id = current
                    .dataset_id
                    .ok_or_else(|| PythonResearchError("feature-dataset-id-missing".into()))?;
                return Ok((snapshot.snapshot_id, dataset_id));
            }
            MaterializationAttemptStatus::Failed | MaterializationAttemptStatus::Cancelled => {
                return Err(PythonResearchError(format!(
                    "feature-dataset-attempt-terminal:{:?}:{:?}",
                    current.status, current.failure_code
                )));
            }
            MaterializationAttemptStatus::Pending | MaterializationAttemptStatus::Running => {
                if std::time::Instant::now() >= deadline {
                    return Err(PythonResearchError(
                        "feature-dataset-attempt-timeout".into(),
                    ));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

fn prepare_factor_feature_evidence(
    local_state: &crate::local_research::LocalResearchState,
    request: &FactorRunRequest,
    fixture: &SyntheticTutorialFixture,
) -> Result<FactorFeatureEvidence, PythonResearchError> {
    let plan = factor_feature_plan()?;
    let plan_hash = plan.plan_hash().to_owned();
    let point_in_time_universe_id = sha256(b"python-tutorial-a-share@1:point-in-time-universe");
    let mut snapshot_bindings = BTreeMap::new();
    let mut dataset_bindings = BTreeMap::new();
    for instrument in &fixture.instruments {
        let (snapshot_id, dataset_id) = materialize_factor_feature_instrument(
            local_state,
            request,
            fixture,
            instrument,
            &plan,
            &plan_hash,
            &point_in_time_universe_id,
        )?;
        snapshot_bindings.insert(instrument.clone(), snapshot_id);
        dataset_bindings.insert(instrument.clone(), dataset_id);
    }
    let primary_instrument = fixture
        .instruments
        .first()
        .ok_or_else(|| PythonResearchError("synthetic-universe-empty".into()))?;
    let snapshot_id = snapshot_bindings
        .get(primary_instrument)
        .cloned()
        .ok_or_else(|| PythonResearchError("feature-snapshot-binding-missing".into()))?;
    let dataset_id = dataset_bindings
        .get(primary_instrument)
        .cloned()
        .ok_or_else(|| PythonResearchError("feature-dataset-binding-missing".into()))?;
    let evidence_sha256 = sha256(
        &serde_json::to_vec(&(
            &fixture.manifest.content_sha256,
            &snapshot_bindings,
            &dataset_bindings,
            &plan_hash,
            fixture.momentum_rows(),
        ))
        .map_err(|error| PythonResearchError(error.to_string()))?,
    );
    Ok(FactorFeatureEvidence {
        snapshot_id,
        dataset_id,
        snapshot_bindings,
        dataset_bindings,
        plan_hash,
        evidence_sha256,
    })
}

fn factor_market_series(
    fixture: &SyntheticTutorialFixture,
    context: &FactorMarketContext,
    snapshot_id: &str,
) -> Result<Vec<FactorMarketSeries>, PythonResearchError> {
    fixture
        .instruments
        .iter()
        .map(|instrument| {
            let bars = fixture
                .bars
                .iter()
                .filter(|bar| &bar.instrument == instrument)
                .map(|bar| {
                    let close = Decimal::from_f64_retain(bar.close)
                        .ok_or_else(|| PythonResearchError("synthetic-price-invalid".into()))?;
                    Ok(OhlcvBar {
                        open_time_ms: bar.session as i64,
                        open: close,
                        high: close,
                        low: close,
                        close,
                        base_volume: Decimal::ONE,
                        quote_volume: close,
                    })
                })
                .collect::<Result<Vec<_>, PythonResearchError>>()?;
            Ok(FactorMarketSeries {
                instrument_id: instrument.clone(),
                snapshot_id: snapshot_id.into(),
                market_context: context.clone(),
                bars,
                gaps: Vec::new(),
                corporate_action_evidence: CorporateActionEvidence::Verified,
            })
        })
        .collect()
}

fn factor_runner_input(
    fixture: &SyntheticTutorialFixture,
    split_batches: bool,
) -> Result<PythonFactorInput, PythonResearchError> {
    let rows = fixture.momentum_rows();
    let batches = if split_batches {
        let midpoint = rows.len() / 2;
        vec![
            PythonFactorBatch {
                rows: rows[..midpoint].to_vec(),
            },
            PythonFactorBatch {
                rows: rows[midpoint..].to_vec(),
            },
        ]
    } else {
        vec![PythonFactorBatch { rows }]
    };
    let input = PythonFactorInput {
        universe: fixture.instruments.clone(),
        segments: vec![PythonFactorSegment {
            segment_id: "continuous-1".into(),
            batches,
        }],
    };
    input.validate()?;
    Ok(input)
}

fn factor_evaluation_protocol(
    user_id: uuid::Uuid,
    family_id: uuid::Uuid,
    trial_id: uuid::Uuid,
    dataset_id: &str,
    feature_dataset_id: &str,
    feature_plan_hash: &str,
    snapshot_id: &str,
    universe_id: &str,
    context: FactorMarketContext,
    engine_identity: ResearchEngineProvenance,
) -> Result<FactorEvaluationProtocol, PythonResearchError> {
    FactorEvaluationProtocol::freeze(FactorEvaluationProtocolDraft {
        protocol_id: uuid::Uuid::new_v4(),
        user_id,
        factor_dataset_id: dataset_id.into(),
        feature_dataset_id: feature_dataset_id.into(),
        feature_plan_hash: feature_plan_hash.into(),
        market_data_snapshot_id: snapshot_id.into(),
        point_in_time_universe_id: universe_id.into(),
        point_in_time_universe: (1..=12).map(|index| format!("SIM{index:02}")).collect(),
        output_name: "momentum-score".into(),
        scope: FactorScope::CrossSectional,
        target: FactorTarget::FutureCloseReturn,
        horizon_bars: vec![5],
        market_context: context,
        engine_identity,
        orientation: FactorOrientation::Positive,
        windows: vec![
            EvaluationWindow {
                fold_id: "tutorial-selection-validation-1".into(),
                selection: adaq_factor_research::ObservationRange {
                    start_time_ms: 1,
                    end_time_ms: 61,
                },
                evaluation: adaq_factor_research::ObservationRange {
                    start_time_ms: 66,
                    end_time_ms: 101,
                },
                training: Some(adaq_factor_research::ObservationRange {
                    start_time_ms: 1,
                    end_time_ms: 61,
                }),
                fitting: Some(adaq_factor_research::ObservationRange {
                    start_time_ms: 1,
                    end_time_ms: 61,
                }),
                normalization: Some(adaq_factor_research::ObservationRange {
                    start_time_ms: 1,
                    end_time_ms: 61,
                }),
                target_construction: Some(adaq_factor_research::ObservationRange {
                    start_time_ms: 1,
                    end_time_ms: 61,
                }),
            },
            EvaluationWindow {
                fold_id: "tutorial-selection-validation-2".into(),
                selection: adaq_factor_research::ObservationRange {
                    start_time_ms: 106,
                    end_time_ms: 141,
                },
                evaluation: adaq_factor_research::ObservationRange {
                    start_time_ms: 146,
                    end_time_ms: 181,
                },
                training: Some(adaq_factor_research::ObservationRange {
                    start_time_ms: 106,
                    end_time_ms: 141,
                }),
                fitting: Some(adaq_factor_research::ObservationRange {
                    start_time_ms: 106,
                    end_time_ms: 141,
                }),
                normalization: Some(adaq_factor_research::ObservationRange {
                    start_time_ms: 106,
                    end_time_ms: 141,
                }),
                target_construction: Some(adaq_factor_research::ObservationRange {
                    start_time_ms: 106,
                    end_time_ms: 141,
                }),
            },
        ],
        purge_bars: 5,
        embargo_bars: 5,
        lenses: vec![FactorLens::CrossSectional, FactorLens::Economic],
        nuisance_feature_names: Vec::new(),
        regime: None,
        economic: EconomicAssumptions {
            rebalance_every_bars: 1,
            fee_bps: 0.0,
            slippage_bps: 0.0,
            long_short: true,
        },
        family_id,
        trial_id,
        seed: 7,
    })
    .map_err(|error| PythonResearchError(error.to_string()))
}

fn run_factor_evidence(
    local_state: &crate::local_research::LocalResearchState,
    request: &FactorRunRequest,
    candidate_hash: &str,
    feature_evidence: &FactorFeatureEvidence,
    python_outputs: &BTreeMap<u32, Vec<MomentumOutputRow>>,
) -> Result<FactorEvidenceRun, PythonResearchError> {
    let fixture = SyntheticTutorialFixture::m12()?;
    fixture.validate()?;
    let user = crate::factor_research::user_uuid(&request.user_id);
    let snapshot_id = feature_evidence.snapshot_id.clone();
    let universe_id = sha256(b"python-tutorial-a-share@1:point-in-time-universe");
    let feature_dataset_id = feature_evidence.dataset_id.clone();
    let feature_plan_hash = feature_evidence.plan_hash.clone();
    let candidate = local_state
        .factor
        .get_candidate(crate::factor_research::FactorEvidenceRequest {
            user_id: request.user_id.clone(),
            evidence_id: candidate_hash.into(),
        })
        .map_err(PythonResearchError)?
        .candidate;
    let binding = match &candidate.source {
        FactorCandidateSource::Python { binding } => binding,
        _ => {
            return Err(PythonResearchError(
                "python-factor-candidate-source-invalid".into(),
            ));
        }
    };
    if binding.project_revision_sha256 != request.project_revision_sha256
        || binding.environment_sha256 != request.environment_sha256
        || binding.snapshot_id != snapshot_id
        || binding.snapshot_bindings != feature_evidence.snapshot_bindings
        || binding.point_in_time_universe_id != universe_id
        || binding.feature_dataset_bindings != feature_evidence.dataset_bindings
        || binding.feature_plan_hash != feature_plan_hash
        || binding.feature_evidence_sha256 != feature_evidence.evidence_sha256
        || binding.normalized_parameters.get("lookback") != Some(&"20".into())
        || binding.engine_identity != "adaq-python-factor@1"
        || binding.sdk_artifact_sha256 != PUBLIC_SDK_ARTIFACT_SHA256
        || binding.input_bindings.get("close") != Some(&"host:market-close".into())
        || binding.seed != 7
    {
        return Err(PythonResearchError(
            "python-factor-candidate-provenance-mismatch".into(),
        ));
    }
    let context = factor_market_context(&universe_id);
    let base_protocol_hash = sha256(b"python-tutorial-a-share@1:factor-grid");
    let family_id = uuid::Uuid::new_v4();
    let parameters = vec![GridSearchParameter {
        name: "lookback".into(),
        values: vec![
            FactorParameterValue::Integer(5),
            FactorParameterValue::Integer(20),
            FactorParameterValue::Integer(60),
        ],
    }];
    let plan = GridSearchPlan::new(parameters.clone())
        .map_err(|error| PythonResearchError(error.to_string()))?;
    let identities = plan
        .trial_identities(family_id, candidate_hash, &base_protocol_hash)
        .map_err(|error| PythonResearchError(error.to_string()))?;
    let grid_attempt = local_state
        .factor
        .register_grid_family(FactorGridFamilyRegisterRequest {
            user_id: request.user_id.clone(),
            family_id,
            candidate_hash: candidate_hash.into(),
            parent_family_id: None,
            parameters,
            target: FactorTarget::FutureCloseReturn,
            market_context: context.clone(),
            point_in_time_universe_id: universe_id.clone(),
            observation_range: adaq_factor_research::ObservationRange {
                start_time_ms: 1,
                end_time_ms: 181,
            },
            base_protocol_hash,
            derivation_hash: None,
        })
        .map_err(PythonResearchError)?;
    wait_for_factor_attempt(
        &local_state.factor,
        &request.user_id,
        &grid_attempt.attempt_id,
    )?;

    let market_series = factor_market_series(&fixture, &context, &snapshot_id)?;
    let mut reports = BTreeMap::new();
    let mut datasets = BTreeMap::new();
    let mut registry = ResearchRegistry::default();
    registry
        .register_grid_search_family(GridSearchFamilyDraft {
            family_id,
            user_id: user,
            candidate_hash: candidate_hash.into(),
            parent_family_id: None,
            plan,
            target: FactorTarget::FutureCloseReturn,
            market_context: context.clone(),
            point_in_time_universe_id: universe_id.clone(),
            observation_range: adaq_factor_research::ObservationRange {
                start_time_ms: 1,
                end_time_ms: 181,
            },
            base_protocol_hash: sha256(b"python-tutorial-a-share@1:factor-grid"),
            derivation_hash: None,
        })
        .map_err(|error| PythonResearchError(error.to_string()))?;
    for (lookback, identity) in [5_u32, 20, 60].into_iter().zip(&identities) {
        let engine = factor_engine_identity(
            &request.project_revision_sha256,
            &request.environment_sha256,
            &fixture.manifest.content_sha256,
            lookback,
        );
        let protocol = adaq_factor_research::FactorMaterializationProtocol::freeze(
            adaq_factor_research::FactorMaterializationProtocolDraft {
                protocol_id: uuid::Uuid::new_v4(),
                user_id: user,
                candidate_hash: candidate_hash.into(),
                feature_dataset_id: feature_dataset_id.clone(),
                feature_plan_hash: feature_plan_hash.clone(),
                parameters: vec![FactorParameterValue::Integer(lookback as i64)],
                market_data_snapshot_id: snapshot_id.clone(),
                point_in_time_universe_id: universe_id.clone(),
                observation_range: adaq_factor_research::ObservationRange {
                    start_time_ms: 1,
                    end_time_ms: 181,
                },
                market_context: context.clone(),
                engine_identity: engine.clone(),
                seed: 7,
            },
        )
        .map_err(|error| PythonResearchError(error.to_string()))?;
        let dataset = factor_dataset_input_from_output(
            &protocol,
            python_outputs
                .get(&lookback)
                .cloned()
                .ok_or_else(|| PythonResearchError("python-factor-output-missing".into()))?,
        )?;
        let materialization = local_state
            .factor
            .start_materialization(crate::factor_research::FactorMaterializationStartRequest {
                user_id: request.user_id.clone(),
                protocol: protocol.clone(),
                dataset: Some(dataset.clone()),
                context: None,
            })
            .map_err(PythonResearchError)?;
        let dataset_id = wait_for_factor_attempt(
            &local_state.factor,
            &request.user_id,
            &materialization.attempt_id,
        )?;
        let evaluation = factor_evaluation_protocol(
            user,
            family_id,
            identity.trial_id,
            &dataset_id,
            &feature_dataset_id,
            &feature_plan_hash,
            &snapshot_id,
            &universe_id,
            context.clone(),
            engine,
        )?;
        let evaluation_attempt = local_state
            .factor
            .start_evaluation(FactorEvaluationStartRequest {
                user_id: request.user_id.clone(),
                protocol: evaluation.clone(),
                dataset: None,
                market_series: market_series.clone(),
                feature_evidence: None,
            })
            .map_err(PythonResearchError)?;
        let report_hash = wait_for_factor_attempt(
            &local_state.factor,
            &request.user_id,
            &evaluation_attempt.attempt_id,
        )?;
        let report = local_state
            .factor
            .get_report(crate::factor_research::FactorEvidenceRequest {
                user_id: request.user_id.clone(),
                evidence_id: report_hash.clone(),
            })
            .map_err(PythonResearchError)?;
        let (raw_statistic, p_value) =
            factor_trial_statistics(&report.report).map_err(PythonResearchError)?;
        datasets.insert(identity.trial_id, (dataset_id, evaluation));
        reports.insert(identity.trial_id, report_hash.clone());
        local_state
            .factor
            .update_trial(FactorTrialUpdateRequest {
                user_id: request.user_id.clone(),
                trial: ResearchTrial {
                    trial_id: identity.trial_id,
                    family_id,
                    candidate_hash: candidate_hash.into(),
                    protocol_hash: identity.protocol_hash.clone(),
                    status: ResearchTrialStatus::Completed,
                    report_hash: Some(report_hash),
                    raw_statistic: raw_statistic.clone(),
                    p_value: p_value.clone(),
                    holm_adjusted: None,
                    related_trial_ids: Vec::new(),
                    diagnostic: None,
                },
            })
            .map_err(PythonResearchError)?;
        registry
            .record_trial(
                user,
                identity.trial_id,
                ResearchTrialStatus::Completed,
                reports.get(&identity.trial_id).cloned(),
                raw_statistic,
                p_value,
                None,
            )
            .map_err(|error| PythonResearchError(error.to_string()))?;
    }
    let root_trial_id = identities
        .first()
        .map(|identity| identity.trial_id)
        .ok_or_else(|| PythonResearchError("python-factor-trial-family-empty".into()))?;
    registry
        .apply_holm_bonferroni(user, root_trial_id)
        .map_err(|error| PythonResearchError(error.to_string()))?;
    for identity in &identities {
        let trial = registry
            .trial(user, identity.trial_id)
            .map_err(|error| PythonResearchError(error.to_string()))?;
        local_state
            .factor
            .update_trial(FactorTrialUpdateRequest {
                user_id: request.user_id.clone(),
                trial,
            })
            .map_err(PythonResearchError)?;
    }
    let root = identities
        .iter()
        .find(|identity| identity.index == 1)
        .ok_or_else(|| PythonResearchError("python-factor-root-trial-missing".into()))?;
    let lineage = registry
        .lineage(user, root.trial_id)
        .map_err(|error| PythonResearchError(error.to_string()))?;
    let policy = PromotionPolicy::conservative_template(
        uuid::Uuid::from_u128(0x6d120101000000000000000000000003),
        1,
        FactorScope::CrossSectional,
    )
    .map_err(|error| PythonResearchError(error.to_string()))?;
    local_state
        .factor
        .save_policy(FactorPolicySaveRequest {
            user_id: request.user_id.clone(),
            policy: policy.clone(),
        })
        .map_err(PythonResearchError)?;
    let mut promotion_protocols = BTreeMap::new();
    for identity in &identities {
        let engine_identity = datasets
            .get(&identity.trial_id)
            .map(|(_, protocol)| protocol.engine_identity.clone())
            .ok_or_else(|| PythonResearchError("python-factor-trial-protocol-missing".into()))?;
        let report_hash = reports
            .get(&identity.trial_id)
            .cloned()
            .ok_or_else(|| PythonResearchError("python-factor-trial-report-missing".into()))?;
        let protocol = registry
            .freeze_promotion_protocol(PromotionProtocolDraft {
                protocol_id: uuid::Uuid::new_v4(),
                user_id: user,
                candidate_hash: candidate_hash.into(),
                output_name: "momentum-score".into(),
                family_id,
                trial_id: identity.trial_id,
                lineage_trial_ids: lineage.trial_ids.clone(),
                report_hashes: vec![report_hash],
                policy_hash: policy.policy_hash.clone(),
                engine_identity,
            })
            .map_err(|error| PythonResearchError(error.to_string()))?;
        promotion_protocols.insert(identity.trial_id.to_string(), protocol);
    }
    let promotion_protocol = promotion_protocols
        .get(&root.trial_id.to_string())
        .cloned()
        .ok_or_else(|| PythonResearchError("python-factor-root-protocol-missing".into()))?;
    Ok(FactorEvidenceRun {
        candidate_hash: candidate_hash.into(),
        family_id,
        trial_ids: identities
            .iter()
            .map(|identity| identity.trial_id)
            .collect(),
        dataset_ids: datasets
            .values()
            .map(|(dataset_id, _)| dataset_id.clone())
            .collect(),
        report_hashes: reports.values().cloned().collect(),
        promotion_protocol,
        promotion_protocols,
        policy,
    })
}

impl PythonResearchState {
    pub fn open(app_data_dir: &std::path::Path) -> Self {
        let root = app_data_dir.join("python-research");
        Self {
            store: Arc::new(ProjectStore::new(&root)),
            attempt_store: Arc::new(
                AttemptStore::open(root.join("research-attempts.json"))
                    .expect("python research attempt store must open"),
            ),
            trust_store: Arc::new(
                TrustStore::open(root.join("trust-decisions.json"))
                    .expect("python research trust store must open"),
            ),
            model_lab_store: Arc::new(
                ModelLabStore::open(root.join("model-lab.json"))
                    .expect("python research model lab store must open"),
            ),
            runtime_store: Arc::new(RuntimeStore::new(&root)),
            wheelhouse_store: Arc::new(WheelhouseStore::new(&root)),
            managed_runtime_gate: Arc::new(Mutex::new(())),
            environment_store: Arc::new(EnvironmentStore::new(&root)),
            root,
            examples_root: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples/python"),
            queue_admitter: Mutex::new(None),
            queue_waker: Mutex::new(None),
            resetting_users: Mutex::new(BTreeSet::new()),
            completed_results: Arc::new(Mutex::new(BTreeMap::new())),
            runtime_cancellations: Mutex::new(BTreeMap::new()),
            runtime_progress: Arc::new(Mutex::new(BTreeMap::new())),
            shutdown: AtomicBool::new(false),
        }
    }

    pub(crate) fn accepted_model_inputs(
        &self,
        user_id: &str,
    ) -> Result<Vec<AcceptedModelInput>, String> {
        self.model_lab_store
            .accepted_model_inputs(user_id)
            .map_err(map_error)
    }

    pub(crate) fn accepted_model_input(
        &self,
        user_id: &str,
        report_id: &str,
    ) -> Result<AcceptedModelInput, String> {
        self.model_lab_store
            .accepted_model_input(user_id, report_id)
            .map_err(map_error)
    }

    pub(crate) fn attach_queue(&self, queue: ResearchQueue) {
        if let Ok(mut attached) = self.queue_admitter.lock() {
            *attached = Some(queue.admitter());
        }
        if let Ok(mut attached) = self.queue_waker.lock() {
            *attached = Some(queue.waker());
        }
    }

    fn notify_queue(&self) {
        if let Ok(waker) = self.queue_waker.lock()
            && let Some(waker) = waker.as_ref()
        {
            waker();
        }
    }

    fn admit_attempt(&self, attempt: &ResearchAttempt) -> Result<(), String> {
        let admitter = self
            .queue_admitter
            .lock()
            .map_err(|_| "Python Research Queue lock poisoned")?
            .clone()
            .ok_or_else(|| "Python Research Queue is not attached".to_owned())?;
        admitter(WorkKind::Python, &attempt.user_id, &attempt.attempt_id)
    }

    fn start_attempt(
        &self,
        request: AttemptStartRequest,
    ) -> Result<ResearchAttempt, PythonResearchError> {
        let resetting = self
            .resetting_users
            .lock()
            .map_err(|_| PythonResearchError("research-reset-barrier-lock-poisoned".into()))?;
        if resetting.contains(&request.user_id) {
            return Err(PythonResearchError("research-reset-in-progress".into()));
        }
        let context = load_attempt_context(
            &self.store,
            &self.environment_store,
            &self.runtime_store,
            &request.user_id,
            &request.project_id,
            &request.revision_sha256,
            Some(&request.environment_sha256),
        )?;
        if self
            .trust_store
            .get(
                &request.user_id,
                &request.project_id,
                &request.revision_sha256,
            )?
            .is_none()
        {
            return Err(PythonResearchError("research-revision-not-trusted".into()));
        }
        let policy = effective_resource_policy(&context.manifest, request.resource_policy)?;
        let execution = build_attempt_execution(
            &context.revision,
            &context.manifest,
            &context.lock,
            request.seed.unwrap_or(0),
            None,
            None,
        )?;
        let attempt = self.attempt_store.enqueue_with_execution(
            request.user_id,
            request.project_id,
            request.revision_sha256,
            context.environment.environment_sha256,
            policy,
            execution,
        )?;
        drop(resetting);
        self.admit_attempt(&attempt).map_err(PythonResearchError)?;
        Ok(attempt)
    }

    fn cancel_attempt(
        &self,
        user_id: &str,
        attempt_id: &str,
    ) -> Result<ResearchAttempt, PythonResearchError> {
        let attempt = self.attempt_store.get(attempt_id)?;
        if attempt.user_id != user_id {
            return Err(PythonResearchError("research-attempt-not-found".into()));
        }
        let attempt = self
            .attempt_store
            .transition(attempt_id, AttemptTransition::Cancel)?;
        self.notify_queue();
        Ok(attempt)
    }

    fn retry_attempt(
        &self,
        user_id: &str,
        attempt_id: &str,
    ) -> Result<ResearchAttempt, PythonResearchError> {
        let resetting = self
            .resetting_users
            .lock()
            .map_err(|_| PythonResearchError("research-reset-barrier-lock-poisoned".into()))?;
        if resetting.contains(user_id) {
            return Err(PythonResearchError("research-reset-in-progress".into()));
        }
        let attempt = self.attempt_store.get(attempt_id)?;
        if attempt.user_id != user_id {
            return Err(PythonResearchError("research-attempt-not-found".into()));
        }
        let attempt = self.attempt_store.retry(attempt_id)?;
        drop(resetting);
        self.admit_attempt(&attempt).map_err(PythonResearchError)?;
        Ok(attempt)
    }

    fn reset_user(&self, user_id: &str) -> Result<PythonResearchResetReport, PythonResearchError> {
        {
            let mut resetting = self
                .resetting_users
                .lock()
                .map_err(|_| PythonResearchError("research-reset-barrier-lock-poisoned".into()))?;
            if !resetting.insert(user_id.into()) {
                return Err(PythonResearchError("research-reset-in-progress".into()));
            }
        }
        let result = (|| {
            self.attempt_store.cancel_user(user_id)?;
            self.notify_queue();
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while self.attempt_store.has_active_for_user(user_id)? {
                if std::time::Instant::now() >= deadline {
                    return Err(PythonResearchError(
                        "research-reset-runner-did-not-stop".into(),
                    ));
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            let attempt_ids = self
                .attempt_store
                .list(user_id)?
                .into_iter()
                .map(|attempt| attempt.attempt_id)
                .collect::<BTreeSet<_>>();
            for attempt_id in &attempt_ids {
                let artifact = self
                    .root
                    .join("attempt-results")
                    .join(format!("{attempt_id}.artifact"));
                if artifact.is_file() {
                    fs::remove_file(artifact)?;
                }
            }
            let report = self.store.reset_python_research_evidence(user_id)?;
            self.attempt_store.reset_user(user_id)?;
            self.trust_store.reset_user(user_id)?;
            self.model_lab_store.reset_user(user_id)?;
            if let Ok(mut results) = self.completed_results.lock() {
                results.retain(|attempt_id, _| !attempt_ids.contains(attempt_id));
            }
            Ok(report)
        })();
        if let Ok(mut resetting) = self.resetting_users.lock() {
            resetting.remove(user_id);
        }
        result
    }

    fn execute_attempt(&self, attempt_id: &str) -> QueueRunResult {
        let mut attempt = match self.attempt_store.get(attempt_id) {
            Ok(attempt) => attempt,
            Err(error) if error.0 == "research-attempt-not-found" => {
                return QueueRunResult::Stale;
            }
            Err(error) => return QueueRunResult::Retryable(error.0),
        };
        if attempt.status == adaq_python_research::runner::AttemptStatus::Pending {
            let updated = match self
                .attempt_store
                .transition(attempt_id, AttemptTransition::Begin)
            {
                Ok(updated) => updated,
                Err(error) => return QueueRunResult::Retryable(error.0),
            };
            attempt = updated;
        }
        if attempt.status != adaq_python_research::runner::AttemptStatus::Running {
            return QueueRunResult::Stale;
        }
        let attempt_id = attempt.attempt_id.clone();
        let execution = self.run_attempt(&attempt);
        match execution {
            Ok(RunnerExecution {
                conformance: Some(result),
                staged_result: None,
                staged_artifact: Some(artifact),
                log,
                log_truncated,
            }) if result.attempt_id == attempt_id && result.project_id == attempt.project_id => {
                if !log.is_empty() {
                    let _ = self.attempt_store.transition(
                        &attempt_id,
                        AttemptTransition::RecordLog {
                            value: String::from_utf8_lossy(&log).into_owned(),
                        },
                    );
                }
                let result_sha256 = artifact.sha256.clone();
                if self
                    .attempt_store
                    .transition(&attempt_id, AttemptTransition::Complete { result_sha256 })
                    .is_ok()
                {
                    if let Ok(mut results) = self.completed_results.lock() {
                        results.insert(
                            attempt_id.clone(),
                            RunnerExecution {
                                conformance: Some(result),
                                staged_result: None,
                                staged_artifact: Some(artifact),
                                log,
                                log_truncated,
                            },
                        );
                    }
                } else if self
                    .attempt_store
                    .get(&attempt_id)
                    .map(|current| current.cancel_requested)
                    .unwrap_or(false)
                {
                    let _ = self
                        .attempt_store
                        .transition(&attempt_id, AttemptTransition::FinishCancel);
                }
            }
            Ok(RunnerExecution {
                conformance: Some(result),
                ..
            }) if result.attempt_id != attempt_id || result.project_id != attempt.project_id => {
                let cancelled = self
                    .attempt_store
                    .get(&attempt_id)
                    .map(|current| current.cancel_requested)
                    .unwrap_or(false);
                let _ = if cancelled {
                    self.attempt_store
                        .transition(&attempt_id, AttemptTransition::FinishCancel)
                } else {
                    self.attempt_store.transition(
                        &attempt_id,
                        AttemptTransition::MarkStale {
                            diagnostic: "Runner returned a result for a different Attempt or Project"
                                .into(),
                        },
                    )
                };
            }
            Err(error) if error.0 == "runner-cancelled" => {
                let _ = self
                    .attempt_store
                    .transition(&attempt_id, AttemptTransition::FinishCancel);
            }
            Ok(_) => {
                let cancelled = self
                    .attempt_store
                    .get(&attempt_id)
                    .map(|current| current.cancel_requested)
                    .unwrap_or(false);
                let _ = if cancelled {
                    self.attempt_store
                        .transition(&attempt_id, AttemptTransition::FinishCancel)
                } else {
                    self.attempt_store.transition(
                        &attempt_id,
                        AttemptTransition::Fail {
                            code: "runner-result-invalid".into(),
                            diagnostic: "Runner returned no Host-validated conformance result"
                                .into(),
                        },
                    )
                };
            }
            Err(error) => {
                let cancelled = self
                    .attempt_store
                    .get(&attempt_id)
                    .map(|current| current.cancel_requested)
                    .unwrap_or(false);
                let _ = if cancelled {
                    self.attempt_store
                        .transition(&attempt_id, AttemptTransition::FinishCancel)
                } else {
                    self.attempt_store.transition(
                        &attempt_id,
                        AttemptTransition::Fail {
                            code: "runner-failed".into(),
                            diagnostic: error.0,
                        },
                    )
                };
            }
        }
        QueueRunResult::Consumed
    }

    fn run_attempt(
        &self,
        attempt: &ResearchAttempt,
    ) -> Result<RunnerExecution, PythonResearchError> {
        let cancelled = || {
            self.shutdown.load(Ordering::Relaxed)
                || self
                    .attempt_store
                    .get(&attempt.attempt_id)
                    .map(|current| current.cancel_requested)
                    .unwrap_or(true)
        };
        self.run_attempt_with_cancel(attempt, &cancelled)
    }

    fn run_attempt_with_cancel(
        &self,
        attempt: &ResearchAttempt,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<RunnerExecution, PythonResearchError> {
        let workspace = self.root.join("attempt-staging").join(&attempt.attempt_id);
        if workspace.exists() {
            fs::remove_dir_all(&workspace)?;
        }
        if let Some(parent) = workspace.parent() {
            fs::create_dir_all(parent)?;
        }
        let result = (|| {
            let revision = self.store.materialize_revision(
                &attempt.user_id,
                &attempt.project_id,
                &attempt.revision_sha256,
                &workspace,
            )?;
            let report = inspect_project(&workspace);
            if !report.valid() {
                return Err(PythonResearchError(
                    "runner-project-revision-invalid".into(),
                ));
            }
            let manifest = report
                .manifest
                .ok_or_else(|| PythonResearchError("runner-project-manifest-missing".into()))?;
            if manifest.project_id != attempt.project_id {
                return Err(PythonResearchError(
                    "runner-project-identity-mismatch".into(),
                ));
            }
            if let Some(input) = attempt.execution.input.as_ref() {
                match manifest.kind {
                    ProjectKind::Factor if manifest.mode == Some(ProjectMode::ImperativePython) => {
                    }
                    ProjectKind::Model => serde_json::from_value::<ModelRunnerInput>(input.clone())
                        .map_err(|error| {
                            PythonResearchError(format!("runner-model-input-invalid:{error}"))
                        })?
                        .validate()?,
                    _ => {
                        return Err(PythonResearchError(
                            "runner-project-input-not-allowed".into(),
                        ));
                    }
                }
            }
            if manifest.kind == ProjectKind::Factor
                && manifest.mode == Some(ProjectMode::PortableDefinition)
            {
                validate_portable_factor_source(&workspace, &manifest)?;
            }
            if manifest.kind == ProjectKind::Model {
                validate_model_source(&workspace, &manifest)?;
            }
            if manifest.kind == ProjectKind::Factor
                && manifest.mode == Some(ProjectMode::ImperativePython)
            {
                let input = attempt
                    .execution
                    .input
                    .as_ref()
                    .ok_or_else(|| PythonResearchError("runner-factor-input-missing".into()))?;
                serde_json::from_value::<PythonFactorInput>(input.clone())
                    .map_err(|error| {
                        PythonResearchError(format!("runner-factor-input-invalid:{error}"))
                    })?
                    .validate()?;
            }
            let lock = self
                .environment_store
                .load_lock(&attempt.environment_sha256)?;
            if lock.wheelhouse_identity != wheelhouse_catalog(lock.platform)?.manifest.identity {
                return Err(PythonResearchError(
                    "runner-wheelhouse-identity-mismatch".into(),
                ));
            }
            let runtime = runtime_catalog_entry(lock.platform)?.manifest;
            if revision.runtime_artifact_sha256.as_deref() != Some(runtime.artifact_sha256.as_str())
                || lock.runtime_artifact_sha256 != runtime.artifact_sha256
            {
                return Err(PythonResearchError(
                    "runner-runtime-identity-mismatch".into(),
                ));
            }
            let expected_execution = build_attempt_execution(
                &revision,
                &manifest,
                &lock,
                attempt.execution.seed,
                attempt.execution.input.clone(),
                Some(&attempt.execution.parameters),
            )?;
            if attempt.execution != expected_execution {
                return Err(PythonResearchError(
                    "runner-attempt-execution-identity-mismatch".into(),
                ));
            }
            let python_executable = self
                .runtime_store
                .executable_path(&lock.runtime_artifact_sha256)?;
            let sdk_wheel = self
                .environment_store
                .wheel_path(&attempt.environment_sha256, "adaq-research-sdk")?;
            let adapter_wheel = self
                .environment_store
                .wheel_path(&attempt.environment_sha256, "adaq-qlib-ridge-adapter")?;
            let runner_wheel = self
                .environment_store
                .wheel_path(&attempt.environment_sha256, "adaq-python-research-runner")?;
            let one_time_token = runner_token();
            let handshake = Handshake {
                protocol: adaq_python_research::runner::RUNNER_PROTOCOL_VERSION.into(),
                sdk_artifact_sha256: revision.sdk_artifact_sha256,
                revision_sha256: revision.revision_sha256,
                environment_sha256: attempt.environment_sha256.clone(),
                attempt_id: attempt.attempt_id.clone(),
                loopback: true,
                one_time_token,
            };
            let environment = PrivateChildEnvironment::from_allowlist(BTreeMap::from([
                ("PYTHONHASHSEED".into(), attempt.execution.seed.to_string()),
                (
                    "OMP_NUM_THREADS".into(),
                    attempt.resource_policy.max_threads.to_string(),
                ),
                (
                    "OPENBLAS_NUM_THREADS".into(),
                    attempt.resource_policy.max_threads.to_string(),
                ),
                (
                    "MKL_NUM_THREADS".into(),
                    attempt.resource_policy.max_threads.to_string(),
                ),
                (
                    "NUMEXPR_NUM_THREADS".into(),
                    attempt.resource_policy.max_threads.to_string(),
                ),
            ]))?;
            let runner_script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../python/adaq-python-research-runner/src/adaq_runner/__main__.py");
            let entry_point = manifest.entry_point.clone();
            let project_kind = serde_json::to_value(manifest.kind)
                .map_err(|error| PythonResearchError(error.to_string()))?
                .as_str()
                .map(ToOwned::to_owned)
                .ok_or_else(|| PythonResearchError("runner-project-kind-invalid".into()))?;
            let execution = run_process(
                &RunnerLaunchSpec {
                    python_executable,
                    runner_script,
                    runner_wheel: Some(runner_wheel),
                    project_root: workspace.clone(),
                    entry_point,
                    sdk_wheel: Some(sdk_wheel),
                    adapter_wheel: Some(adapter_wheel),
                    handshake,
                    environment,
                    staging_root: workspace.join(".adaq-staging"),
                    staged_result_path: workspace.join(".adaq-staging/conformance-result.json"),
                    staged_result_relative_path: "conformance-result.json".into(),
                    execution: attempt.execution.clone(),
                    seed: attempt.execution.seed,
                    max_wall_ms: attempt.resource_policy.max_wall_ms,
                    max_memory_bytes: attempt.resource_policy.max_memory_bytes,
                    max_processes: attempt.resource_policy.max_processes,
                    max_control_bytes: attempt.resource_policy.max_control_bytes as usize,
                    max_arrow_bytes: attempt.resource_policy.max_arrow_bytes as usize,
                    max_staged_bytes: attempt.resource_policy.max_staged_bytes as usize,
                    max_artifact_bytes: attempt.resource_policy.max_artifact_bytes as usize,
                    max_log_bytes: attempt.resource_policy.max_log_bytes as usize,
                },
                cancelled,
            )?;
            if let Some(result) = execution.conformance.as_ref()
                && (result.project_kind != project_kind
                    || result.entry_point != manifest.entry_point)
            {
                return Err(PythonResearchError(
                    "runner-result-contract-mismatch".into(),
                ));
            }
            if manifest.kind == ProjectKind::Factor {
                let payload = execution
                    .conformance
                    .as_ref()
                    .and_then(|result| result.payload.as_ref())
                    .ok_or_else(|| PythonResearchError("runner-factor-output-missing".into()))?;
                match manifest.mode {
                    Some(ProjectMode::PortableDefinition) => {
                        validate_portable_definition_payload(payload)?;
                    }
                    Some(ProjectMode::ImperativePython) => {
                        let input = attempt.execution.input.as_ref().ok_or_else(|| {
                            PythonResearchError("runner-factor-input-missing".into())
                        })?;
                        let input = serde_json::from_value::<PythonFactorInput>(input.clone())
                            .map_err(|error| {
                                PythonResearchError(format!("runner-factor-input-invalid:{error}"))
                            })?;
                        validate_imperative_factor_payload(payload, &input)?;
                    }
                    None => {
                        return Err(PythonResearchError("runner-factor-mode-missing".into()));
                    }
                }
            }
            if manifest.kind == ProjectKind::Model {
                let payload = execution
                    .conformance
                    .as_ref()
                    .and_then(|result| result.payload.as_ref())
                    .ok_or_else(|| PythonResearchError("runner-model-contract-missing".into()))?;
                validate_model_project_payload(payload)?;
                let input = attempt
                    .execution
                    .input
                    .as_ref()
                    .ok_or_else(|| PythonResearchError("runner-model-input-missing".into()))?;
                let input =
                    serde_json::from_value::<ModelRunnerInput>(input.clone()).map_err(|error| {
                        PythonResearchError(format!("runner-model-input-invalid:{error}"))
                    })?;
                validate_model_runner_payload(payload, &input.transformation.feature_names)?;
            }
            let artifact = execution
                .staged_artifact
                .as_ref()
                .ok_or_else(|| PythonResearchError("runner-staged-result-missing".into()))?;
            publish_attempt_artifact(
                &self.root,
                &attempt.attempt_id,
                &workspace.join(".adaq-staging"),
                artifact,
            )?;
            Ok(execution)
        })();
        let _ = fs::remove_dir_all(&workspace);
        result
    }

    fn run_trusted_project_with_seed(
        &self,
        user_id: &str,
        project_id: &str,
        revision_sha256: &str,
        environment_sha256: &str,
        seed: u64,
    ) -> Result<RunnerExecution, PythonResearchError> {
        self.run_trusted_project_with_execution(
            user_id,
            project_id,
            revision_sha256,
            environment_sha256,
            seed,
            None,
            None,
        )
    }

    fn run_trusted_project_with_execution(
        &self,
        user_id: &str,
        project_id: &str,
        revision_sha256: &str,
        environment_sha256: &str,
        seed: u64,
        input: Option<serde_json::Value>,
        parameter_overrides: Option<&BTreeMap<String, String>>,
    ) -> Result<RunnerExecution, PythonResearchError> {
        self.run_trusted_project_with_execution_or_retry(
            user_id,
            project_id,
            revision_sha256,
            environment_sha256,
            seed,
            input,
            parameter_overrides,
            None,
        )
    }

    fn run_trusted_project_with_retry(
        &self,
        user_id: &str,
        project_id: &str,
        revision_sha256: &str,
        environment_sha256: &str,
        seed: u64,
        input: Option<serde_json::Value>,
        parameter_overrides: Option<&BTreeMap<String, String>>,
        source_attempt_id: &str,
    ) -> Result<RunnerExecution, PythonResearchError> {
        self.run_trusted_project_with_execution_or_retry(
            user_id,
            project_id,
            revision_sha256,
            environment_sha256,
            seed,
            input,
            parameter_overrides,
            Some(source_attempt_id),
        )
    }

    fn run_trusted_project_with_execution_or_retry(
        &self,
        user_id: &str,
        project_id: &str,
        revision_sha256: &str,
        environment_sha256: &str,
        seed: u64,
        input: Option<serde_json::Value>,
        parameter_overrides: Option<&BTreeMap<String, String>>,
        source_attempt_id: Option<&str>,
    ) -> Result<RunnerExecution, PythonResearchError> {
        let context = load_attempt_context(
            &self.store,
            &self.environment_store,
            &self.runtime_store,
            user_id,
            project_id,
            revision_sha256,
            Some(environment_sha256),
        )?;
        if self
            .trust_store
            .get(user_id, project_id, revision_sha256)?
            .is_none()
        {
            return Err(PythonResearchError("research-revision-not-trusted".into()));
        }
        let resource_policy = effective_resource_policy(&context.manifest, None)?;
        let execution = build_attempt_execution(
            &context.revision,
            &context.manifest,
            &context.lock,
            seed,
            input,
            parameter_overrides,
        )?;
        let attempt = if let Some(source_attempt_id) = source_attempt_id {
            let previous = self.attempt_store.get(source_attempt_id)?;
            if previous.user_id != user_id
                || previous.project_id != project_id
                || previous.revision_sha256 != revision_sha256
                || previous.environment_sha256 != environment_sha256
                || previous.resource_policy != resource_policy
                || previous.execution != execution
                || (!matches!(
                    previous.status,
                    PythonAttemptStatus::Failed
                        | PythonAttemptStatus::Cancelled
                        | PythonAttemptStatus::Interrupted
                        | PythonAttemptStatus::Stale
                ) && !(previous.status == PythonAttemptStatus::Completed
                    && previous.failure_code.is_some()))
            {
                return Err(PythonResearchError(
                    "research-attempt-retry-binding-invalid".into(),
                ));
            }
            self.attempt_store.retry(source_attempt_id)?
        } else {
            self.attempt_store.enqueue_with_execution(
                user_id,
                project_id,
                revision_sha256,
                context.environment.environment_sha256.clone(),
                resource_policy,
                execution,
            )?
        };
        self.admit_attempt(&attempt).map_err(PythonResearchError)?;
        self.wait_for_attempt(
            &attempt.attempt_id,
            HostResourcePolicy::m12_default().max_wall_ms,
        )
    }

    fn run_trusted_project_verification(
        &self,
        user_id: &str,
        project_id: &str,
        revision_sha256: &str,
        environment_sha256: &str,
        seed: u64,
        input: Option<serde_json::Value>,
        parameter_overrides: Option<&BTreeMap<String, String>>,
    ) -> Result<(RunnerExecution, HostResourcePolicy), PythonResearchError> {
        let context = load_attempt_context(
            &self.store,
            &self.environment_store,
            &self.runtime_store,
            user_id,
            project_id,
            revision_sha256,
            Some(environment_sha256),
        )?;
        if self
            .trust_store
            .get(user_id, project_id, revision_sha256)?
            .is_none()
        {
            return Err(PythonResearchError("research-revision-not-trusted".into()));
        }
        let resource_policy = effective_resource_policy(&context.manifest, None)?;
        let mut attempt = ResearchAttempt::new(
            user_id,
            project_id,
            revision_sha256,
            context.environment.environment_sha256,
            NEXT_MODEL_VERIFICATION_SEQUENCE.fetch_add(1, Ordering::Relaxed),
            resource_policy,
        )?;
        attempt.execution = build_attempt_execution(
            &context.revision,
            &context.manifest,
            &context.lock,
            seed,
            input,
            parameter_overrides,
        )?;
        let execution =
            self.run_attempt_with_cancel(&attempt, &|| self.shutdown.load(Ordering::Relaxed))?;
        Ok((execution, attempt.resource_policy))
    }

    fn wait_for_attempt(
        &self,
        attempt_id: &str,
        max_wall_ms: u64,
    ) -> Result<RunnerExecution, PythonResearchError> {
        let deadline = std::time::Instant::now()
            .checked_add(Duration::from_millis(max_wall_ms.saturating_add(5_000)))
            .ok_or_else(|| PythonResearchError("research-attempt-deadline-invalid".into()))?;
        loop {
            if let Ok(mut results) = self.completed_results.lock()
                && let Some(result) = results.remove(attempt_id)
            {
                return Ok(result);
            }
            let attempt = self.attempt_store.get(attempt_id)?;
            match attempt.status {
                adaq_python_research::runner::AttemptStatus::Completed => continue,
                adaq_python_research::runner::AttemptStatus::Failed => {
                    return Err(PythonResearchError(format!(
                        "research-attempt-failed:{}:{}",
                        attempt.failure_code.unwrap_or_else(|| "unknown".into()),
                        attempt.diagnostic.unwrap_or_else(|| "no diagnostic".into())
                    )));
                }
                adaq_python_research::runner::AttemptStatus::Cancelled => {
                    return Err(PythonResearchError("runner-cancelled".into()));
                }
                adaq_python_research::runner::AttemptStatus::Interrupted => {
                    return Err(PythonResearchError("research-attempt-interrupted".into()));
                }
                adaq_python_research::runner::AttemptStatus::Stale => {
                    return Err(PythonResearchError("research-attempt-stale".into()));
                }
                adaq_python_research::runner::AttemptStatus::Pending
                | adaq_python_research::runner::AttemptStatus::Running => {}
            }
            if std::time::Instant::now() >= deadline {
                let _ = self
                    .attempt_store
                    .transition(attempt_id, AttemptTransition::Cancel);
                return Err(PythonResearchError(
                    "research-attempt-deadline-exceeded".into(),
                ));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn example(&self, name: &str) -> Result<(&'static str, PathBuf), String> {
        let (project_id, directory) = match name {
            "factor" => (
                "py-factor-cross-sectional-momentum",
                "py-factor-cross-sectional-momentum",
            ),
            "model" => (MODEL_PROJECT_ID, MODEL_PROJECT_ID),
            "strategy" => ("py-strategy-top-n-forecast", "py-strategy-top-n-forecast"),
            _ => return Err("python-research-example-unknown".into()),
        };
        Ok((project_id, self.examples_root.join(directory)))
    }
}

impl ResearchQueueAdapter for PythonResearchState {
    fn pending_attempts(&self) -> Result<Vec<QueueAdmission>, String> {
        self.attempt_store
            .pending_attempts()
            .map_err(|error| error.to_string())
            .map(|attempts| {
                attempts
                    .into_iter()
                    .map(|attempt| QueueAdmission {
                        user_id: attempt.user_id,
                        attempt_id: attempt.attempt_id,
                    })
                    .collect()
            })
    }

    fn execute(&self, ticket: QueueTicket) -> QueueRunResult {
        self.execute_attempt(&ticket.attempt_id)
    }

    fn request_shutdown(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }
}

fn runner_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectCreateRequest {
    pub user_id: String,
    pub example: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRequest {
    pub user_id: String,
    pub project_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectFreezeRequest {
    pub user_id: String,
    pub project_id: String,
    pub sdk_artifact_sha256: String,
    pub runtime_artifact_sha256: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectImportRequest {
    pub user_id: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectExport {
    pub project_id: String,
    pub revision_sha256: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeProfileView {
    pub profile: String,
    pub platform: Option<RuntimePlatform>,
    pub status: String,
    pub preparation_status: Option<adaq_python_research::runtime::PreparationStatus>,
    pub preparation_attempt_id: Option<String>,
    pub preparation_diagnostic: Option<String>,
    pub preparation_completed_bytes: Option<u64>,
    pub preparation_total_bytes: Option<u64>,
    pub expected_version: String,
    pub source: String,
    pub artifact_sha256: Option<String>,
    pub download_bytes: Option<u64>,
    pub installed_bytes: Option<u64>,
    pub license: Option<String>,
    pub wheelhouse_identity: Option<String>,
    pub wheelhouse_status: String,
    pub wheelhouse_wheel_count: usize,
    pub runtime_cache_bytes: u64,
    pub wheelhouse_disk_bytes: u64,
    pub environment_cache_bytes: u64,
    pub environment_count: usize,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeProfileRequest {
    #[serde(default)]
    pub user_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimePrepareRequest {
    pub user_id: String,
    pub manifest: RuntimeArtifactManifest,
    pub payload: Vec<u8>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedRuntimePrepareRequest {
    pub user_id: String,
    #[serde(default)]
    pub source_attempt_id: Option<String>,
    #[serde(default)]
    pub task_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimePrepareCancelRequest {
    pub task_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentSyncRequest {
    pub runtime_artifact_sha256: String,
    pub platform: RuntimePlatform,
    pub intent: DependencyIntent,
    pub wheelhouse: WheelhouseManifest,
    pub payloads: BTreeMap<String, Vec<u8>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentPrepareRequest {
    pub lock: EnvironmentLock,
    pub payloads: BTreeMap<String, Vec<u8>>,
    pub wheelhouse: WheelhouseManifest,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedEnvironmentPrepareRequest {
    pub user_id: String,
    pub project_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheEvictRequest {
    pub active_runtime_artifacts: Vec<String>,
    #[serde(default)]
    pub active_wheelhouses: Vec<String>,
    pub active_environments: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheEvictResult {
    pub runtimes: Vec<String>,
    pub wheelhouses: Vec<String>,
    pub environments: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentSyncResult {
    pub lock_sha256: String,
    pub wheelhouse: WheelhouseRecord,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustRequest {
    pub user_id: String,
    pub project_id: String,
    pub revision_sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttemptStartRequest {
    pub user_id: String,
    pub project_id: String,
    pub revision_sha256: String,
    pub environment_sha256: String,
    pub resource_policy: Option<HostResourcePolicy>,
    #[serde(default)]
    pub seed: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttemptPreviewRequest {
    pub user_id: String,
    pub project_id: String,
    pub revision_sha256: String,
    #[serde(default)]
    pub resource_policy: Option<HostResourcePolicy>,
    #[serde(default)]
    pub seed: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttemptPreview {
    pub project_id: String,
    pub revision_sha256: String,
    pub entry_point: String,
    pub source_files: BTreeMap<String, String>,
    pub lock: EnvironmentLock,
    pub environment_sha256: String,
    pub runtime: RuntimeArtifactManifest,
    pub sdk_artifact_sha256: String,
    pub input_bindings: BTreeMap<String, String>,
    pub normalized_parameters: BTreeMap<String, String>,
    pub seed: u64,
    pub resource_policy: HostResourcePolicy,
    pub trust_decision: Option<adaq_python_research::TrustDecision>,
    pub trusted_code_warning: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttemptRequest {
    pub user_id: String,
    pub attempt_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelExperimentRequest {
    pub user_id: String,
    pub attempt_id: String,
    pub project_revision_sha256: String,
    pub environment_sha256: String,
    pub input_evidence_sha256: String,
    pub factor_decision_hash: String,
    pub seed: u64,
    #[serde(default)]
    pub derived_from_decision_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRunRequest {
    pub user_id: String,
    pub project_id: String,
    pub project_revision_sha256: String,
    pub environment_sha256: String,
    pub factor_decision_hash: String,
    pub alpha: Option<f64>,
    #[serde(default)]
    pub retry_attempt_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FactorRunRequest {
    pub user_id: String,
    pub project_id: String,
    pub project_revision_sha256: String,
    pub environment_sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactorTrialSelectionRequest {
    pub user_id: String,
    pub candidate_hash: String,
    pub family_id: String,
    pub trial_id: String,
    pub policy_hash: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactorPromotionRequest {
    pub user_id: String,
    pub candidate_hash: String,
    pub trial_id: String,
    pub state: PromotionDecisionState,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelTrialCompleteRequest {
    pub user_id: String,
    pub experiment_id: String,
    pub trial_id: String,
    pub attempt_id: String,
    pub selection_metric: f64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelTrialFailRequest {
    pub user_id: String,
    pub experiment_id: String,
    pub trial_id: String,
    pub attempt_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelTrialRetryRequest {
    pub user_id: String,
    pub experiment_id: String,
    pub trial_id: String,
    pub attempt_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSelectionRequest {
    pub user_id: String,
    pub experiment_id: String,
    pub trial_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelFinalEvaluationRequest {
    pub user_id: String,
    pub decision_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelDeploymentQualificationRequest {
    pub user_id: String,
    pub decision_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelLabStateRequest {
    pub user_id: String,
    #[serde(default)]
    pub factor_decision_hash: Option<String>,
}

fn map_error(error: PythonResearchError) -> String {
    error.to_string()
}

fn append_runner_log(logs: &mut Vec<String>, execution: &RunnerExecution) {
    if !execution.log.is_empty() {
        logs.push(String::from_utf8_lossy(&execution.log).into_owned());
    }
    if execution.log_truncated {
        logs.push("runner-log-truncated".into());
    }
}

struct AttemptContext {
    revision: ProjectRevision,
    manifest: ProjectManifest,
    lock: EnvironmentLock,
    environment: EnvironmentRecord,
    runtime: RuntimeArtifactManifest,
}

fn load_attempt_context(
    project_store: &ProjectStore,
    environment_store: &EnvironmentStore,
    runtime_store: &RuntimeStore,
    user_id: &str,
    project_id: &str,
    revision_sha256: &str,
    expected_environment_sha256: Option<&str>,
) -> Result<AttemptContext, PythonResearchError> {
    let revision = project_store.revision(user_id, project_id, revision_sha256)?;
    let manifest = project_store.revision_manifest(user_id, project_id, revision_sha256)?;
    if manifest.project_id != project_id
        || revision.project_id != project_id
        || revision.dependency_lock_sha256 != manifest.dependency_lock_sha256
    {
        return Err(PythonResearchError(
            "research-revision-identity-mismatch".into(),
        ));
    }
    let environment = environment_store
        .find_by_lock_file_sha256(&manifest.dependency_lock_sha256)?
        .ok_or_else(|| PythonResearchError("python-environment-not-prepared".into()))?;
    if expected_environment_sha256.is_some_and(|value| value != environment.environment_sha256) {
        return Err(PythonResearchError(
            "research-environment-identity-mismatch".into(),
        ));
    }
    let lock = environment_store.load_lock(&environment.environment_sha256)?;
    let wheelhouse = wheelhouse_catalog(lock.platform)?;
    if lock.wheelhouse_identity != wheelhouse.manifest.identity {
        return Err(PythonResearchError(
            "research-wheelhouse-identity-mismatch".into(),
        ));
    }
    let runtime = runtime_catalog_entry(lock.platform)?.manifest;
    if revision.runtime_artifact_sha256.as_deref() != Some(runtime.artifact_sha256.as_str())
        || lock.runtime_artifact_sha256 != runtime.artifact_sha256
        || revision.runtime_profile != runtime.profile
    {
        return Err(PythonResearchError(
            "research-runtime-identity-mismatch".into(),
        ));
    }
    let sdk = lock
        .wheels
        .iter()
        .find(|wheel| {
            wheel
                .package
                .replace('_', "-")
                .eq_ignore_ascii_case("adaq-research-sdk")
        })
        .ok_or_else(|| PythonResearchError("research-sdk-not-in-environment".into()))?;
    if sdk.sha256 != revision.sdk_artifact_sha256 {
        return Err(PythonResearchError("research-sdk-identity-mismatch".into()));
    }
    runtime_store.ready_record(&runtime)?;
    Ok(AttemptContext {
        revision,
        manifest,
        lock,
        environment,
        runtime,
    })
}

fn build_attempt_execution(
    revision: &ProjectRevision,
    manifest: &ProjectManifest,
    lock: &EnvironmentLock,
    seed: u64,
    input: Option<serde_json::Value>,
    parameter_overrides: Option<&BTreeMap<String, String>>,
) -> Result<AttemptExecution, PythonResearchError> {
    let sdk = lock
        .wheels
        .iter()
        .find(|wheel| {
            wheel
                .package
                .replace('_', "-")
                .eq_ignore_ascii_case("adaq-research-sdk")
        })
        .ok_or_else(|| PythonResearchError("research-sdk-not-in-environment".into()))?;
    if sdk.sha256 != revision.sdk_artifact_sha256 {
        return Err(PythonResearchError("research-sdk-identity-mismatch".into()));
    }
    Ok(AttemptExecution {
        sdk_artifact_sha256: revision.sdk_artifact_sha256.clone(),
        runtime_artifact_sha256: revision.runtime_artifact_sha256.clone().unwrap_or_default(),
        entry_point: manifest.entry_point.clone(),
        input_bindings: manifest
            .input_slots
            .iter()
            .map(|slot| (slot.id.clone(), format!("host:{}", slot.role)))
            .collect(),
        parameters: normalize_parameters_for_attempt(manifest, parameter_overrides)?,
        seed,
        output_names: manifest
            .outputs
            .iter()
            .map(|output| output.id.clone())
            .collect(),
        input,
    })
}

fn validate_portable_factor_source(
    project_root: &Path,
    manifest: &ProjectManifest,
) -> Result<(), PythonResearchError> {
    const CANONICAL_PROJECT_ID: &str = "py-factor-cross-sectional-momentum";
    const CANONICAL_SOURCE_SHA256: &str =
        "583a21cdf73ab395564f9dabb7445ca0dac72c32b0cb18a09fe537ddba722d7a";
    if manifest.project_id != CANONICAL_PROJECT_ID
        || manifest.source_files != vec!["src/project.py".to_owned()]
    {
        return Err(PythonResearchError(
            "portable-factor-project-not-registered".into(),
        ));
    }
    const FORBIDDEN_MARKERS: &[&str] = &[
        "lambda",
        "callback",
        "open(",
        "socket",
        "urllib",
        "requests",
        "pathlib",
        "subprocess",
        "eval(",
        "exec(",
        "getattr(",
        "globals(",
        "locals(",
        "__import__",
        "__builtins__",
        " os.",
        " sys.",
    ];
    for source_file in &manifest.source_files {
        let source = fs::read_to_string(project_root.join(source_file))
            .map_err(|error| PythonResearchError(format!("portable-source-read-failed:{error}")))?;
        if sha256(source.as_bytes()) != CANONICAL_SOURCE_SHA256 {
            return Err(PythonResearchError(format!(
                "portable-source-revision-not-canonical:{source_file}"
            )));
        }
        let lowered = source.to_ascii_lowercase();
        if lowered.lines().any(|line| {
            let line = line.trim_start();
            line.starts_with("for ")
                || line.starts_with("while ")
                || line.starts_with("import ")
                || (line.starts_with("from ") && !line.starts_with("from adaq import "))
        }) || FORBIDDEN_MARKERS
            .iter()
            .any(|marker| lowered.contains(marker))
        {
            return Err(PythonResearchError(format!(
                "portable-source-construct-rejected:{source_file}"
            )));
        }
    }
    Ok(())
}

fn validate_model_source(
    project_root: &Path,
    manifest: &ProjectManifest,
) -> Result<(), PythonResearchError> {
    const CANONICAL_PROJECT_ID: &str = MODEL_PROJECT_ID;
    const CANONICAL_SOURCE_SHA256: &str =
        "293f7a0b144b05d653defd616d7aa1894f8eeeef1c0e4f21e04a70eeda9351e2";
    if manifest.project_id != CANONICAL_PROJECT_ID
        || manifest.source_files != vec!["src/project.py".to_owned()]
    {
        return Err(PythonResearchError("model-project-not-registered".into()));
    }
    let source = fs::read(project_root.join("src/project.py"))?;
    if sha256(&source) != CANONICAL_SOURCE_SHA256 {
        return Err(PythonResearchError(
            "model-source-revision-not-canonical:src/project.py".into(),
        ));
    }
    Ok(())
}

fn portable_factor_outputs(
    payload: &serde_json::Value,
    fixture: &SyntheticTutorialFixture,
) -> Result<BTreeMap<u32, Vec<MomentumOutputRow>>, PythonResearchError> {
    validate_portable_definition_payload(payload)?;
    expand_momentum_grid()
        .into_iter()
        .map(|lookback| Ok((lookback, evaluate_portable_momentum(fixture, lookback)?)))
        .collect()
}

fn evaluate_portable_momentum(
    fixture: &SyntheticTutorialFixture,
    lookback: u32,
) -> Result<Vec<MomentumOutputRow>, PythonResearchError> {
    let definition = FeatureDefinition::freeze(DefinitionDraft {
        definition_id: uuid::Uuid::from_u128(
            0x6d120101000000000000000000000020 + u128::from(lookback),
        ),
        revision: 1,
        scope: FeatureScope::CrossSectional,
        nodes: vec![
            FeatureNode {
                id: "return".into(),
                operator: FeatureOperator::BackwardSimpleReturn,
                scope: FeatureScope::TimeSeries,
                inputs: vec![FeatureInput::Market {
                    field: "close".into(),
                }],
                parameters: BTreeMap::from([("period".into(), serde_json::json!(lookback))]),
                warmup_bars: lookback,
            },
            FeatureNode {
                id: "percentile".into(),
                operator: FeatureOperator::CrossSectionalPercentile,
                scope: FeatureScope::CrossSectional,
                inputs: vec![FeatureInput::Node {
                    node_id: "return".into(),
                    definition_hash: None,
                }],
                parameters: BTreeMap::new(),
                warmup_bars: 0,
            },
        ],
        outputs: vec![FeatureOutput {
            name: "momentum-score".into(),
            node_id: "percentile".into(),
        }],
    })
    .map_err(|error| {
        PythonResearchError(format!("portable-feature-definition-invalid:{error:?}"))
    })?;
    let plan = FeaturePlan::freeze(FeaturePlanDraft {
        definitions: vec![definition],
        engine_identity: FeatureEngineIdentity::native()
            .map_err(|error| PythonResearchError(error.to_string()))?,
        ..FeaturePlanDraft::default()
    })
    .map_err(|error| PythonResearchError(format!("portable-feature-plan-invalid:{error:?}")))?;
    let context = FeatureMarketContext::new(
        Venue::us_equity("iex").map_err(|error| PythonResearchError(error.to_string()))?,
        VenueKind::UsEquity,
        BarInterval::OneDay,
        PriceBasis::Unadjusted,
        "USD",
    )
    .map_err(|error| PythonResearchError(error.to_string()))?;
    let universe = PointInTimeInstrumentUniverse::new(
        "python-tutorial-a-share@1:point-in-time-universe",
        0,
        fixture.instruments.clone(),
        context.clone(),
        UniverseEvidenceState::Observed,
    )
    .map_err(|error| PythonResearchError(error.to_string()))?;
    let events = fixture
        .sessions
        .iter()
        .map(|session| {
            let inputs = fixture
                .instruments
                .iter()
                .map(|instrument| {
                    let bar = fixture
                        .bars
                        .iter()
                        .find(|bar| bar.session == *session && bar.instrument == *instrument)
                        .ok_or_else(|| {
                            PythonResearchError("portable-feature-bar-missing".into())
                        })?;
                    let close = bar.close.to_string();
                    let market_bar = FeatureMarketBar::complete(
                        i64::from(*session),
                        &close,
                        &close,
                        &close,
                        &close,
                        "1",
                        &close,
                    )
                    .map_err(|error| PythonResearchError(error.to_string()))?;
                    Ok(FeatureEvaluationInput::new(
                        instrument,
                        i64::from(*session),
                        i64::from(*session),
                        market_bar,
                    )
                    .with_market_context(context.clone()))
                })
                .collect::<Result<Vec<_>, PythonResearchError>>()?;
            Ok(FeatureInputEvent::cross_sectional_batch(
                i64::from(*session),
                PointInTimeInstrumentUniverse {
                    as_of_ms: i64::from(*session),
                    ..universe.clone()
                },
                inputs,
            ))
        })
        .collect::<Result<Vec<_>, PythonResearchError>>()?;
    let observations = FeatureEngine::new(plan.engine_identity())
        .evaluate_batch(plan, &events)
        .map_err(|error| {
            PythonResearchError(format!("portable-feature-evaluation-failed:{error:?}"))
        })?;
    let mut output = observations
        .into_iter()
        .map(|observation| {
            let unavailable_reason = match observation.value {
                FeatureObservationValue::Available { value, .. } => {
                    // The existing Factor contract uses the upper-rank percentile (rank / N),
                    // while Feature Engine exposes the normalized rank ((rank - 1) / (N - 1)).
                    let count = fixture.instruments.len() as f64;
                    let value = (value * (count - 1.0) + 1.0) / count;
                    return Ok(MomentumOutputRow {
                        instrument_id: observation.instrument_id,
                        observation_time_ms: observation.observation_time_ms,
                        value: Some(value),
                        unavailable_reason: None,
                    });
                }
                FeatureObservationValue::Unavailable { reason } => match reason {
                    FeatureUnavailabilityReason::Warmup => FactorUnavailableReason::Warmup,
                    FeatureUnavailabilityReason::BarGap => FactorUnavailableReason::BarGap,
                    _ => FactorUnavailableReason::MissingInput,
                },
            };
            Ok(MomentumOutputRow {
                instrument_id: observation.instrument_id,
                observation_time_ms: observation.observation_time_ms,
                value: None,
                unavailable_reason: Some(unavailable_reason),
            })
        })
        .collect::<Result<Vec<_>, PythonResearchError>>()?;
    output.sort_by(|left, right| {
        (left.instrument_id.as_str(), left.observation_time_ms)
            .cmp(&(right.instrument_id.as_str(), right.observation_time_ms))
    });
    Ok(output)
}

fn runner_process_evidence(
    context: &AttemptContext,
    request: &FactorRunRequest,
    execution: &RunnerExecution,
    parameters: &BTreeMap<String, String>,
    seed: u64,
    input: &serde_json::Value,
) -> Result<RunnerProcessEvidence, PythonResearchError> {
    let result = execution
        .conformance
        .as_ref()
        .ok_or_else(|| PythonResearchError("runner-process-identity-missing".into()))?;
    if !is_sha256_text(&result.attempt_id) {
        return Err(PythonResearchError("runner-attempt-id-invalid".into()));
    }
    let attempt_id = result.attempt_id.clone();
    let input_sha256 = runner_input_sha256(input)?;
    let contract = serde_json::json!({
        "contract": "adaq-python-factor-process@1",
        "projectRevisionSha256": request.project_revision_sha256,
        "environmentSha256": request.environment_sha256,
        "sdkArtifactSha256": context.revision.sdk_artifact_sha256,
        "projectId": result.project_id,
        "projectKind": result.project_kind,
        "entryPoint": result.entry_point,
        "mode": context.manifest.mode,
        "parameters": parameters,
        "seed": seed,
        "inputSlots": context.manifest.input_slots,
        "outputs": context.manifest.outputs,
        "inputSha256": input_sha256,
    });
    let contract_bytes =
        serde_json::to_vec(&contract).map_err(|error| PythonResearchError(error.to_string()))?;
    let contract_sha256 = sha256(&contract_bytes);
    let result_sha256 = sha256(
        &serde_json::to_vec(result).map_err(|error| PythonResearchError(error.to_string()))?,
    );
    let process = serde_json::json!({
        "contractSha256": contract_sha256,
        "attemptId": attempt_id,
        "status": "completed",
        "resultSha256": result_sha256,
    });
    let process_sha256 = sha256(
        &serde_json::to_vec(&process).map_err(|error| PythonResearchError(error.to_string()))?,
    );
    Ok(RunnerProcessEvidence {
        attempt_id,
        process_sha256,
        contract_sha256,
        input_sha256,
    })
}

fn completed_host_attempt_evidence(
    execution: &RunnerExecution,
    request: &FactorRunRequest,
) -> Result<PythonHostAttemptEvidence, PythonResearchError> {
    let result = execution
        .conformance
        .as_ref()
        .ok_or_else(|| PythonResearchError("runner-host-evidence-result-missing".into()))?;
    if !is_sha256_text(&result.attempt_id) {
        return Err(PythonResearchError("runner-attempt-id-invalid".into()));
    }
    let result_sha256 = execution
        .staged_artifact
        .as_ref()
        .map(|artifact| artifact.sha256.clone())
        .ok_or_else(|| PythonResearchError("runner-host-evidence-artifact-missing".into()))?;
    Ok(PythonHostAttemptEvidence {
        attempt_id: result.attempt_id.clone(),
        owner_user_id: request.user_id.clone(),
        status: "completed".into(),
        project_revision_sha256: request.project_revision_sha256.clone(),
        environment_sha256: request.environment_sha256.clone(),
        result_sha256,
    })
}

fn validate_persisted_python_attempt(
    store: &AttemptStore,
    user_id: &str,
    project_id: &str,
    revision_sha256: &str,
    environment_sha256: &str,
    evidence: &PythonHostAttemptEvidence,
) -> Result<(), PythonResearchError> {
    if !is_sha256_text(&evidence.attempt_id)
        || evidence.owner_user_id != user_id
        || evidence.status != "completed"
        || evidence.project_revision_sha256 != revision_sha256
        || evidence.environment_sha256 != environment_sha256
        || !is_sha256_text(&evidence.result_sha256)
    {
        return Err(PythonResearchError(
            "Python Host evidence does not match the candidate binding".into(),
        ));
    }
    let stored = store.get(&evidence.attempt_id).map_err(|error| {
        PythonResearchError(format!("Python Host Attempt is not persisted: {error}"))
    })?;
    if stored.user_id != user_id
        || stored.project_id != project_id
        || stored.status != PythonAttemptStatus::Completed
        || stored.revision_sha256 != revision_sha256
        || stored.environment_sha256 != environment_sha256
        || stored.staged_result_sha256.as_deref() != Some(evidence.result_sha256.as_str())
    {
        return Err(PythonResearchError(
            "Persisted Python Host Attempt does not match its evidence".into(),
        ));
    }
    Ok(())
}

fn validate_python_host_evidence(
    store: &AttemptStore,
    user_id: &str,
    binding: &PythonFactorBinding,
    evidence: &PythonHostEvidence,
) -> Result<(), PythonResearchError> {
    if evidence.project_revision_sha256 != binding.project_revision_sha256
        || evidence.environment_sha256 != binding.environment_sha256
        || evidence.repeatability_report_sha256 != binding.repeatability_report_sha256
        || evidence.attempts.is_empty()
    {
        return Err(PythonResearchError(
            "Python Host evidence does not match the candidate binding".into(),
        ));
    }
    let attempt_ids = evidence
        .attempts
        .iter()
        .map(|attempt| attempt.attempt_id.as_str())
        .collect::<BTreeSet<_>>();
    if attempt_ids.len() != evidence.attempts.len()
        || binding.repeatability_report.values().any(|report| {
            !attempt_ids.contains(report.first_attempt_id.as_str())
                || !attempt_ids.contains(report.replay_attempt_id.as_str())
        })
    {
        return Err(PythonResearchError(
            "Python Host evidence does not cover repeatability Attempts".into(),
        ));
    }
    for attempt in &evidence.attempts {
        validate_persisted_python_attempt(
            store,
            user_id,
            &binding.project_id,
            &binding.project_revision_sha256,
            &binding.environment_sha256,
            attempt,
        )?;
    }
    Ok(())
}

fn runner_input_sha256(input: &serde_json::Value) -> Result<String, PythonResearchError> {
    if let Ok(contract) = serde_json::from_value::<PythonFactorInput>(input.clone()) {
        let mut rows = contract
            .segments
            .into_iter()
            .flat_map(|segment| segment.batches)
            .flat_map(|batch| batch.rows)
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| {
            (left.observation_time_ms, left.instrument_id.as_str())
                .cmp(&(right.observation_time_ms, right.instrument_id.as_str()))
        });
        let bytes = serde_json::to_vec(&(contract.universe, rows))
            .map_err(|error| PythonResearchError(error.to_string()))?;
        return Ok(sha256(&bytes));
    }
    Ok(sha256(
        &serde_json::to_vec(input).map_err(|error| PythonResearchError(error.to_string()))?,
    ))
}

fn is_sha256_text(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn normalize_parameters_for_attempt(
    manifest: &ProjectManifest,
    overrides: Option<&BTreeMap<String, String>>,
) -> Result<BTreeMap<String, String>, PythonResearchError> {
    let defaults = normalize_parameters(manifest)?;
    let Some(overrides) = overrides else {
        return Ok(defaults);
    };
    if overrides.len() != manifest.parameters.len()
        || manifest
            .parameters
            .iter()
            .any(|parameter| !overrides.contains_key(&parameter.id))
    {
        return Err(PythonResearchError(
            "research-parameter-set-identity-invalid".into(),
        ));
    }
    for parameter in &manifest.parameters {
        let value = overrides
            .get(&parameter.id)
            .ok_or_else(|| PythonResearchError("research-parameter-missing".into()))?;
        let normalized = normalize_parameter_value(parameter.value_type, value)?;
        if normalized != *value
            || (!parameter.allowed_values.is_empty()
                && !parameter
                    .allowed_values
                    .iter()
                    .any(|allowed| allowed == value))
        {
            return Err(PythonResearchError(
                "research-parameter-value-not-allowed".into(),
            ));
        }
    }
    Ok(overrides.clone())
}

fn normalize_parameters(
    manifest: &ProjectManifest,
) -> Result<BTreeMap<String, String>, PythonResearchError> {
    manifest
        .parameters
        .iter()
        .map(|parameter| {
            let value = normalize_parameter_value(parameter.value_type, &parameter.default)?;
            Ok((parameter.id.clone(), value))
        })
        .collect()
}

fn normalize_parameter_value(
    value_type: ParameterType,
    value: &str,
) -> Result<String, PythonResearchError> {
    match value_type {
        ParameterType::Boolean => match value.trim() {
            "true" | "false" => Ok(value.trim().to_owned()),
            _ => Err(PythonResearchError("parameter-boolean-invalid".into())),
        },
        ParameterType::Decimal => normalize_decimal(value),
        ParameterType::Integer => value
            .trim()
            .parse::<i64>()
            .map(|value| value.to_string())
            .map_err(|_| PythonResearchError("parameter-integer-invalid".into())),
        ParameterType::Enum | ParameterType::String => Ok(value.to_owned()),
    }
}

fn normalize_decimal(value: &str) -> Result<String, PythonResearchError> {
    let value = value.trim();
    let (negative, value) = match value.strip_prefix('-') {
        Some(value) => (true, value),
        None => (false, value.strip_prefix('+').unwrap_or(value)),
    };
    let mut parts = value.split('.');
    let integer = parts.next().unwrap_or_default();
    let fraction = parts.next().unwrap_or_default();
    if parts.next().is_some()
        || integer.is_empty()
        || !integer.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(PythonResearchError("parameter-decimal-invalid".into()));
    }
    let integer = integer.trim_start_matches('0');
    let integer = if integer.is_empty() { "0" } else { integer };
    let fraction = fraction.trim_end_matches('0');
    let zero = integer == "0" && fraction.is_empty();
    let canonical = if fraction.is_empty() {
        integer.to_owned()
    } else {
        format!("{integer}.{fraction}")
    };
    Ok(if negative && !zero {
        format!("-{canonical}")
    } else {
        canonical
    })
}

fn effective_resource_policy(
    manifest: &ProjectManifest,
    requested: Option<HostResourcePolicy>,
) -> Result<HostResourcePolicy, PythonResearchError> {
    let host = HostResourcePolicy::m12_default().lowered_by_request(&manifest.resource_request)?;
    requested
        .map(|policy| host.lowered_by(&policy))
        .transpose()
        .map(|policy| policy.unwrap_or(host))
}

fn publish_attempt_artifact(
    root: &Path,
    attempt_id: &str,
    staging_root: &Path,
    artifact: &StagedArtifact,
) -> Result<(), PythonResearchError> {
    let max_bytes = usize::try_from(artifact.byte_size)
        .map_err(|_| PythonResearchError("runner-attempt-result-size-invalid".into()))?;
    let bytes = read_staged_artifact(staging_root, artifact, attempt_id, max_bytes)?;
    let destination_root = root.join("attempt-results");
    fs::create_dir_all(&destination_root)?;
    let destination = destination_root.join(format!("{attempt_id}.artifact"));
    if destination.exists() {
        return Err(PythonResearchError(
            "runner-attempt-result-already-published".into(),
        ));
    }
    let temporary = destination.with_extension("tmp");
    let result = (|| {
        fs::write(&temporary, &bytes)?;
        fs::rename(&temporary, &destination)?;
        Ok::<(), PythonResearchError>(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn set_runtime_progress(
    progress: &Mutex<BTreeMap<String, RuntimePreparationProgress>>,
    user_id: &str,
    completed_bytes: u64,
    total_bytes: u64,
) -> Result<(), String> {
    progress
        .lock()
        .map_err(|_| "python-runtime-progress-store-lock-poisoned".to_string())?
        .insert(
            user_id.into(),
            RuntimePreparationProgress {
                completed_bytes,
                total_bytes,
            },
        );
    Ok(())
}

fn directory_bytes(path: &std::path::Path) -> u64 {
    if path.is_file() {
        return fs::metadata(path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
    }
    fs::read_dir(path)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            if entry.file_type().ok()?.is_symlink() {
                return Some(0);
            }
            Some(directory_bytes(&entry.path()))
        })
        .fold(0, u64::saturating_add)
}

fn download_managed_wheelhouse(
    catalog: &WheelhouseCatalogEntry,
) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|error| format!("python-wheelhouse-client-failed:{error}"))?;
    let mut payloads = BTreeMap::new();
    for wheel in &catalog.manifest.wheels {
        let payload = if let Some(payload) = embedded_wheel_payload(&wheel.file_name) {
            payload.to_vec()
        } else {
            let url = catalog
                .download_urls
                .get(&wheel.file_name)
                .ok_or_else(|| "python-wheelhouse-download-url-missing".to_string())?;
            let response = client
                .get(url)
                .send()
                .map_err(|error| format!("python-wheel-download-failed:{error}"))?;
            if !response.status().is_success() {
                return Err(format!(
                    "python-wheel-download-http-{}",
                    response.status().as_u16()
                ));
            }
            let bytes = response
                .bytes()
                .map_err(|error| format!("python-wheel-download-read-failed:{error}"))?;
            if bytes.len() as u64 != wheel.size {
                return Err(format!(
                    "python-wheel-download-size-mismatch:{}",
                    wheel.file_name
                ));
            }
            bytes.to_vec()
        };
        payloads.insert(wheel.file_name.clone(), payload);
    }
    catalog.manifest.validate(&payloads).map_err(map_error)?;
    Ok(payloads)
}

#[tauri::command]
pub async fn project_list(
    user_id: String,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<Vec<WorkingCopySummary>, String> {
    let _ = user_id;
    let user_id = auth.user_id_for_window(window.label())?;
    let store = state.store.clone();
    tauri::async_runtime::spawn_blocking(move || store.list(&user_id).map_err(map_error))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn project_create(
    mut request: ProjectCreateRequest,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<WorkingCopySummary, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    let (project_id, example_root) = state.example(&request.example)?;
    let store = state.store.clone();
    tauri::async_runtime::spawn_blocking(move || {
        store
            .create_from_example(&request.user_id, &example_root, project_id)
            .map_err(map_error)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn project_import(
    mut request: ProjectImportRequest,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<WorkingCopySummary, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    let store = state.store.clone();
    tauri::async_runtime::spawn_blocking(move || {
        store
            .import_archive(&request.user_id, &request.bytes)
            .map_err(map_error)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn project_validate(
    mut request: ProjectRequest,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<ValidationReport, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    let store = state.store.clone();
    tauri::async_runtime::spawn_blocking(move || {
        store
            .validate(&request.user_id, &request.project_id)
            .map_err(map_error)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn project_freeze(
    mut request: ProjectFreezeRequest,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<ProjectRevision, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    if request.sdk_artifact_sha256 != PUBLIC_SDK_ARTIFACT_SHA256 {
        return Err("unsupported-sdk-artifact".into());
    }
    let store = state.store.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let runtime_artifact_sha256 = request.runtime_artifact_sha256.or_else(|| {
            RuntimePlatform::current()
                .ok()
                .and_then(|platform| runtime_catalog_entry(platform).ok())
                .map(|entry| entry.manifest.artifact_sha256)
        });
        store
            .freeze(
                &request.user_id,
                &request.project_id,
                request.sdk_artifact_sha256,
                runtime_artifact_sha256,
            )
            .map_err(map_error)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn project_export(
    mut request: ProjectFreezeRequest,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<ProjectExport, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    if request.sdk_artifact_sha256 != PUBLIC_SDK_ARTIFACT_SHA256 {
        return Err("unsupported-sdk-artifact".into());
    }
    let store = state.store.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let runtime_artifact_sha256 = request.runtime_artifact_sha256.or_else(|| {
            RuntimePlatform::current()
                .ok()
                .and_then(|platform| runtime_catalog_entry(platform).ok())
                .map(|entry| entry.manifest.artifact_sha256)
        });
        let revision = store
            .freeze(
                &request.user_id,
                &request.project_id,
                request.sdk_artifact_sha256,
                runtime_artifact_sha256,
            )
            .map_err(map_error)?;
        let bytes = store
            .export(&request.user_id, &request.project_id, &revision)
            .map_err(map_error)?;
        Ok(ProjectExport {
            project_id: request.project_id,
            revision_sha256: revision.revision_sha256,
            bytes,
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn research_reset(
    user_id: String,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<PythonResearchResetReport, String> {
    let _ = user_id;
    let user_id = auth.user_id_for_window(window.label())?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.reset_user(&user_id).map_err(map_error))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn trust_revision(
    mut request: TrustRequest,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<adaq_python_research::TrustDecision, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    let project_store = state.store.clone();
    let trust_store = state.trust_store.clone();
    tauri::async_runtime::spawn_blocking(move || {
        project_store
            .revision(
                &request.user_id,
                &request.project_id,
                &request.revision_sha256,
            )
            .map_err(map_error)?;
        trust_store
            .grant(
                &request.user_id,
                &request.project_id,
                &request.revision_sha256,
            )
            .map_err(map_error)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn attempt_preview(
    mut request: AttemptPreviewRequest,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<AttemptPreview, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    let project_store = state.store.clone();
    let environment_store = state.environment_store.clone();
    let runtime_store = state.runtime_store.clone();
    let trust_store = state.trust_store.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let context = load_attempt_context(
            &project_store,
            &environment_store,
            &runtime_store,
            &request.user_id,
            &request.project_id,
            &request.revision_sha256,
            None,
        )?;
        let execution = build_attempt_execution(
            &context.revision,
            &context.manifest,
            &context.lock,
            request.seed.unwrap_or(0),
            None,
            None,
        )?;
        let resource_policy = effective_resource_policy(
            &context.manifest,
            request.resource_policy.clone(),
        )?;
        let source_files = context
            .manifest
            .source_files
            .iter()
            .map(|path| {
                context
                    .revision
                    .files
                    .get(path)
                    .cloned()
                    .map(|hash| (path.clone(), hash))
                    .ok_or_else(|| PythonResearchError("research-source-identity-missing".into()))
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        let trust_decision = trust_store
            .get(
                &request.user_id,
                &request.project_id,
                &request.revision_sha256,
            )?;
        Ok(AttemptPreview {
            project_id: request.project_id,
            revision_sha256: context.revision.revision_sha256,
            entry_point: context.manifest.entry_point,
            source_files,
            lock: context.lock,
            environment_sha256: context.environment.environment_sha256,
            runtime: context.runtime,
            sdk_artifact_sha256: execution.sdk_artifact_sha256,
            input_bindings: execution.input_bindings,
            normalized_parameters: execution.parameters,
            seed: execution.seed,
            resource_policy,
            trust_decision,
            trusted_code_warning: "This exact revision contains trusted Python code and will execute in a private managed process.".into(),
        })
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(map_error)
}

#[tauri::command]
pub async fn attempt_list(
    user_id: String,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<Vec<ResearchAttempt>, String> {
    let _ = user_id;
    let user_id = auth.user_id_for_window(window.label())?;
    let attempt_store = state.attempt_store.clone();
    tauri::async_runtime::spawn_blocking(move || attempt_store.list(&user_id).map_err(map_error))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn attempt_start(
    mut request: AttemptStartRequest,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<ResearchAttempt, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.start_attempt(request).map_err(map_error))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn attempt_cancel(
    mut request: AttemptRequest,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<ResearchAttempt, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state
            .cancel_attempt(&request.user_id, &request.attempt_id)
            .map_err(map_error)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn attempt_retry(
    mut request: AttemptRequest,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<ResearchAttempt, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state
            .retry_attempt(&request.user_id, &request.attempt_id)
            .map_err(map_error)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn model_demo_run(
    mut request: ModelRunRequest,
    state: State<'_, Arc<PythonResearchState>>,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    app: tauri::AppHandle,
) -> Result<ModelRunView, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    let research_state = state.inner().clone();
    let local_state = app
        .state::<Arc<crate::local_research::LocalResearchState>>()
        .inner()
        .clone();
    tauri::async_runtime::spawn_blocking(move || {
        if request.project_id != MODEL_PROJECT_ID {
            return Err("model-project-unsupported".into());
        }
        let alpha = request.alpha.unwrap_or(1.0);
        RidgeAdapter::registered(alpha).map_err(map_error)?;
        let factor_binding = local_state
            .factor
            .model_input_binding(&request.user_id, &request.factor_decision_hash)?;
        let input_evidence_sha256 =
            model_input_evidence_hash(&factor_binding).map_err(map_error)?;
        let project_revision_sha256 = request.project_revision_sha256;
        let environment_sha256 = request.environment_sha256;
        let retry_attempt_id = request.retry_attempt_id;
        let input = ModelInputEvidence {
            decision_hash: factor_binding.decision_hash,
            promotion_protocol_hash: factor_binding.promotion_protocol.protocol_hash,
            factor_dataset_id: factor_binding.factor_dataset_id,
            feature_dataset_id: factor_binding.feature_dataset_id,
            feature_plan_hash: factor_binding.feature_plan_hash,
            snapshot_id: factor_binding.snapshot_id,
            universe_id: factor_binding.universe_id,
            lookback: factor_binding.lookback,
        };
        let factor_dataset =
            load_bound_model_factor_dataset(&local_state, &request.user_id, &input)
                .map_err(map_error)?;
        let evidence = build_model_evidence(&input, Some(&factor_dataset)).map_err(map_error)?;
        let runner_input = evidence.runner_input().map_err(map_error)?;
        let runner_input = serde_json::to_value(runner_input).map_err(|error| error.to_string())?;
        let parameters = BTreeMap::from([("alpha".into(), alpha.to_string())]);
        let execution = if let Some(source_attempt_id) = retry_attempt_id.as_deref() {
            research_state
                .run_trusted_project_with_retry(
                    &request.user_id,
                    &request.project_id,
                    &project_revision_sha256,
                    &environment_sha256,
                    7,
                    Some(runner_input.clone()),
                    Some(&parameters),
                    source_attempt_id,
                )
                .map_err(map_error)?
        } else {
            research_state
                .run_trusted_project_with_execution(
                    &request.user_id,
                    &request.project_id,
                    &project_revision_sha256,
                    &environment_sha256,
                    7,
                    Some(runner_input.clone()),
                    Some(&parameters),
                )
                .map_err(map_error)?
        };
        let (replay_execution, replay_resource_policy) = research_state
            .run_trusted_project_verification(
                &request.user_id,
                &request.project_id,
                &project_revision_sha256,
                &environment_sha256,
                7,
                Some(runner_input.clone()),
                Some(&parameters),
            )
            .map_err(map_error)?;
        let attempt_id =
            validate_model_process_replay(&execution, &replay_execution).map_err(map_error)?;
        let (resource_policy, resource_policy_sha256) = model_replay_resource_policy(
            &research_state.attempt_store,
            &request.user_id,
            &execution,
            &replay_resource_policy,
        )
        .map_err(map_error)?;
        let first_artifact = read_model_candidate(
            &research_state.root,
            &execution,
            alpha,
            &evidence.transformation,
            &evidence.fixture.manifest.content_sha256,
            &project_revision_sha256,
            &environment_sha256,
            &input_evidence_sha256,
            &resource_policy_sha256,
            &input,
        )
        .map_err(map_error)?;
        let replay_artifact = read_model_candidate(
            &research_state.root,
            &replay_execution,
            alpha,
            &evidence.transformation,
            &evidence.fixture.manifest.content_sha256,
            &project_revision_sha256,
            &environment_sha256,
            &input_evidence_sha256,
            &resource_policy_sha256,
            &input,
        )
        .map_err(map_error)?;
        let artifact_replay_divergent = first_artifact.to_bytes().map_err(map_error)?
            != replay_artifact.to_bytes().map_err(map_error)?;
        discard_verification_artifact(&research_state.root, &replay_execution)
            .map_err(map_error)?;
        let mut run = demo_model_run_with_evidence(
            alpha,
            project_revision_sha256.clone(),
            environment_sha256.clone(),
            input_evidence_sha256.clone(),
            input.clone(),
            resource_policy.clone(),
            Some(first_artifact.clone()),
            Some(&factor_dataset),
        )
        .map_err(map_error)?;
        let prediction_input =
            model_prediction_input(runner_input, &first_artifact).map_err(map_error)?;
        let (prediction_execution, _) = research_state
            .run_trusted_project_verification(
                &request.user_id,
                &request.project_id,
                &project_revision_sha256,
                &environment_sha256,
                7,
                Some(prediction_input),
                Some(&parameters),
            )
            .map_err(map_error)?;
        run.forecasts = validate_model_python_forecast(&prediction_execution, &run.forecasts)
            .map_err(map_error)?;
        run.view.forecast_sha256 = model_forecast_sha256(
            &run.artifact.artifact_sha256,
            &run.view.input_evidence_sha256,
            &run.view.snapshot_id,
            &run.view.universe_id,
            &run.forecasts,
        )
        .map_err(map_error)?;
        discard_verification_artifact(&research_state.root, &prediction_execution)
            .map_err(map_error)?;
        let replay = demo_model_run_with_evidence(
            alpha,
            project_revision_sha256,
            environment_sha256,
            input_evidence_sha256,
            input,
            resource_policy,
            Some(replay_artifact),
            Some(&factor_dataset),
        )
        .map_err(map_error)?;
        let repeatability_error = compare_repeatability(
            &run.artifact.coefficients,
            &replay.artifact.coefficients,
            &run.forecasts,
            &replay.forecasts,
        )
        .err();
        if artifact_replay_divergent {
            run.view
                .diagnostics
                .push("model-artifact-replay-divergent".into());
        }
        if let Some(error) = repeatability_error {
            run.view.diagnostics.push(error.to_string());
        }
        run.view.repeatability_verified =
            !artifact_replay_divergent && run.view.diagnostics.is_empty();
        run.view.repeatability_state = if run.view.repeatability_verified {
            RepeatabilityState::Verified
        } else {
            RepeatabilityState::Divergent
        };
        run.view.attempt_id = attempt_id.clone();
        let view = match research_state
            .model_lab_store
            .save_demo_run(&request.user_id, &run, false)
        {
            Ok(view) => view,
            Err(error) => {
                let _ = research_state.attempt_store.transition(
                    &attempt_id,
                    AttemptTransition::RecordHostFailure {
                        code: "model-host-save-failed".into(),
                        diagnostic: error.0.clone(),
                    },
                );
                return Err(error.0);
            }
        };
        Ok(view)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn python_factor_demo(
    mut request: FactorRunRequest,
    state: State<'_, Arc<PythonResearchState>>,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    app: tauri::AppHandle,
) -> Result<FactorRunView, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    let research_state = state.inner().clone();
    let local_state = app
        .state::<Arc<crate::local_research::LocalResearchState>>()
        .inner()
        .clone();
    tauri::async_runtime::spawn_blocking(move || {
        if request.project_id != "py-factor-cross-sectional-momentum" {
            return Err("factor-project-unsupported".into());
        }
        let fixture = SyntheticTutorialFixture::m12().map_err(map_error)?;
        fixture.validate().map_err(map_error)?;
        let context = load_attempt_context(
            &research_state.store,
            &research_state.environment_store,
            &research_state.runtime_store,
            &request.user_id,
            &request.project_id,
            &request.project_revision_sha256,
            Some(&request.environment_sha256),
        )
        .map_err(map_error)?;
        let mode = context
            .manifest
            .mode
            .ok_or_else(|| "factor-project-mode-missing".to_string())?;
        let mut process_replay_exact = true;
        let mut logs = Vec::new();
        let mut process_evidence = BTreeMap::new();
        let mut host_attempts = Vec::new();
        let default_parameters = normalize_parameters(&context.manifest).map_err(map_error)?;
        let (attempt_id, first_outputs, replay_outputs) = match mode {
            ProjectMode::PortableDefinition => {
                let first = research_state
                    .run_trusted_project_with_seed(
                        &request.user_id,
                        &request.project_id,
                        &request.project_revision_sha256,
                        &request.environment_sha256,
                        7,
                    )
                    .map_err(map_error)?;
                let replay = research_state
                    .run_trusted_project_with_seed(
                        &request.user_id,
                        &request.project_id,
                        &request.project_revision_sha256,
                        &request.environment_sha256,
                        7,
                    )
                    .map_err(map_error)?;
                append_runner_log(&mut logs, &first);
                append_runner_log(&mut logs, &replay);
                for execution in [&first, &replay] {
                    host_attempts.push(
                        completed_host_attempt_evidence(execution, &request).map_err(map_error)?,
                    );
                }
                let first_payload = first
                    .conformance
                    .as_ref()
                    .and_then(|result| result.payload.as_ref())
                    .ok_or_else(|| "factor-runner-result-missing".to_string())?;
                let replay_payload = replay
                    .conformance
                    .as_ref()
                    .and_then(|result| result.payload.as_ref())
                    .ok_or_else(|| "factor-runner-replay-result-missing".to_string())?;
                process_replay_exact = first_payload == replay_payload;
                let first_process_evidence = runner_process_evidence(
                    &context,
                    &request,
                    &first,
                    &default_parameters,
                    7,
                    first_payload,
                )
                .map_err(map_error)?;
                let replay_process_evidence = runner_process_evidence(
                    &context,
                    &request,
                    &replay,
                    &default_parameters,
                    7,
                    replay_payload,
                )
                .map_err(map_error)?;
                for lookback in expand_momentum_grid() {
                    process_evidence.insert(
                        lookback,
                        (
                            first_process_evidence.clone(),
                            replay_process_evidence.clone(),
                        ),
                    );
                }
                let first_outputs =
                    portable_factor_outputs(first_payload, &fixture).map_err(map_error)?;
                let replay_outputs =
                    portable_factor_outputs(replay_payload, &fixture).map_err(map_error)?;
                let attempt_id = first
                    .conformance
                    .as_ref()
                    .map(|result| result.attempt_id.clone())
                    .ok_or_else(|| "factor-runner-result-missing".to_string())?;
                (attempt_id, Some(first_outputs), Some(replay_outputs))
            }
            ProjectMode::ImperativePython => {
                let input =
                    serde_json::to_value(factor_runner_input(&fixture, false).map_err(map_error)?)
                        .map_err(|error| error.to_string())?;
                let replay_input =
                    serde_json::to_value(factor_runner_input(&fixture, true).map_err(map_error)?)
                        .map_err(|error| error.to_string())?;
                let mut first = BTreeMap::new();
                let mut replay = BTreeMap::new();
                let mut attempt_id = None;
                for lookback in expand_momentum_grid() {
                    let parameters = BTreeMap::from([("lookback".into(), lookback.to_string())]);
                    let execution = research_state
                        .run_trusted_project_with_execution(
                            &request.user_id,
                            &request.project_id,
                            &request.project_revision_sha256,
                            &request.environment_sha256,
                            7,
                            Some(input.clone()),
                            Some(&parameters),
                        )
                        .map_err(map_error)?;
                    let replay_execution = research_state
                        .run_trusted_project_with_execution(
                            &request.user_id,
                            &request.project_id,
                            &request.project_revision_sha256,
                            &request.environment_sha256,
                            7,
                            Some(replay_input.clone()),
                            Some(&parameters),
                        )
                        .map_err(map_error)?;
                    append_runner_log(&mut logs, &execution);
                    append_runner_log(&mut logs, &replay_execution);
                    for current in [&execution, &replay_execution] {
                        host_attempts.push(
                            completed_host_attempt_evidence(current, &request)
                                .map_err(map_error)?,
                        );
                    }
                    let input_contract = serde_json::from_value::<PythonFactorInput>(input.clone())
                        .map_err(|error| error.to_string())?;
                    let replay_input_contract =
                        serde_json::from_value::<PythonFactorInput>(replay_input.clone())
                            .map_err(|error| error.to_string())?;
                    let output = execution
                        .conformance
                        .as_ref()
                        .and_then(|result| result.payload.as_ref())
                        .ok_or_else(|| "factor-runner-result-missing".to_string())?;
                    let replay_output = replay_execution
                        .conformance
                        .as_ref()
                        .and_then(|result| result.payload.as_ref())
                        .ok_or_else(|| "factor-runner-replay-result-missing".to_string())?;
                    let first_rows = validate_imperative_factor_payload(output, &input_contract)
                        .map_err(map_error)?;
                    let replay_rows =
                        validate_imperative_factor_payload(replay_output, &replay_input_contract)
                            .map_err(map_error)?;
                    process_replay_exact &= first_rows == replay_rows;
                    let first_process_evidence = runner_process_evidence(
                        &context,
                        &request,
                        &execution,
                        &parameters,
                        7,
                        &input,
                    )
                    .map_err(map_error)?;
                    let replay_process_evidence = runner_process_evidence(
                        &context,
                        &request,
                        &replay_execution,
                        &parameters,
                        7,
                        &replay_input,
                    )
                    .map_err(map_error)?;
                    process_evidence
                        .insert(lookback, (first_process_evidence, replay_process_evidence));
                    first.insert(lookback, first_rows);
                    replay.insert(lookback, replay_rows);
                    if lookback == 20 {
                        attempt_id = execution
                            .conformance
                            .as_ref()
                            .map(|result| result.attempt_id.clone());
                    }
                }
                let attempt_id =
                    attempt_id.ok_or_else(|| "factor-runner-result-missing".to_string())?;
                (attempt_id, Some(first), Some(replay))
            }
        };
        let factor_mode = match mode {
            ProjectMode::ImperativePython => PythonFactorMode::ImperativePython,
            ProjectMode::PortableDefinition => PythonFactorMode::PortableDefinition,
        };
        let first_outputs = first_outputs
            .as_ref()
            .ok_or_else(|| "python-factor-output-missing".to_string())?;
        let replay_outputs = replay_outputs
            .as_ref()
            .ok_or_else(|| "python-factor-replay-output-missing".to_string())?;
        let repeatability_report = factor_repeatability_reports(
            first_outputs,
            replay_outputs,
            &process_evidence,
            process_replay_exact,
            factor_mode,
        )
        .map_err(map_error)?;
        let repeatability_verified = repeatability_report.values().all(|report| report.exact);
        let repeatability_report_sha256 = adaq_factor_research::content_hash(&repeatability_report)
            .map_err(|error| error.to_string())?;
        let feature_evidence =
            prepare_factor_feature_evidence(&local_state, &request, &fixture).map_err(map_error)?;
        let candidate_draft = FactorCandidateDraft {
            candidate_id: uuid::Uuid::from_u128(0x6d120101000000000000000000000001),
            revision: 1,
            scope: FactorScope::CrossSectional,
            feature_slots: vec![FactorFeatureSlot {
                name: "close".into(),
            }],
            parameters: vec![FactorParameter {
                name: "lookback".into(),
                parameter_type: FactorParameterType::Integer,
                default_value: "20".into(),
                allowed_values: vec!["5".into(), "20".into(), "60".into()],
            }],
            outputs: vec![FactorOutput {
                name: "momentum-score".into(),
            }],
            source: FactorCandidateSource::Python {
                binding: PythonFactorBinding {
                    project_id: request.project_id.clone(),
                    project_revision_sha256: request.project_revision_sha256.clone(),
                    environment_sha256: request.environment_sha256.clone(),
                    input_bindings: BTreeMap::from([("close".into(), "host:market-close".into())]),
                    snapshot_id: feature_evidence.snapshot_id.clone(),
                    snapshot_bindings: feature_evidence.snapshot_bindings.clone(),
                    point_in_time_universe_id: sha256(
                        b"python-tutorial-a-share@1:point-in-time-universe",
                    ),
                    feature_evidence_sha256: feature_evidence.evidence_sha256.clone(),
                    feature_dataset_bindings: feature_evidence.dataset_bindings.clone(),
                    normalized_parameters: BTreeMap::from([("lookback".into(), "20".into())]),
                    engine_identity: "adaq-python-factor@1".into(),
                    repeatability_report_sha256: repeatability_report_sha256.clone(),
                    repeatability_verified,
                    repeatability_report: repeatability_report.clone(),
                    sdk_artifact_sha256: PUBLIC_SDK_ARTIFACT_SHA256.into(),
                    entry_point: "project:create_project".into(),
                    mode: match mode {
                        ProjectMode::ImperativePython => PythonFactorMode::ImperativePython,
                        ProjectMode::PortableDefinition => PythonFactorMode::PortableDefinition,
                    },
                    feature_plan_hash: feature_evidence.plan_hash.clone(),
                    operator_catalog_version: adaq_feature_engine::FEATURE_OPERATOR_CATALOG_VERSION
                        .into(),
                    resource_policy: python_factor_resource_policy(
                        &HostResourcePolicy::m12_default(),
                    ),
                    seed: 7,
                },
            },
        };
        let host_evidence = PythonHostEvidence {
            project_revision_sha256: request.project_revision_sha256.clone(),
            environment_sha256: request.environment_sha256.clone(),
            repeatability_report_sha256: repeatability_report_sha256.clone(),
            attempts: host_attempts,
        };
        let binding = match &candidate_draft.source {
            FactorCandidateSource::Python { binding } => binding,
            _ => unreachable!("python factor demo candidate must use a Python binding"),
        };
        validate_python_host_evidence(
            research_state.attempt_store.as_ref(),
            &request.user_id,
            binding,
            &host_evidence,
        )
        .map_err(map_error)?;
        let candidate_view = local_state
            .factor
            .publish_python_candidate(
                crate::factor_research::FactorCandidatePublishRequest {
                    user_id: request.user_id.clone(),
                    draft: candidate_draft,
                    presentation: FactorPresentationMetadata {
                        name: "Python Cross-sectional Momentum".into(),
                        description: "Synthetic M12 portable Python Factor candidate".into(),
                        tags: vec!["python".into(), "momentum".into(), "synthetic".into()],
                    },
                },
                host_evidence,
            )
            .map_err(|error| error.to_string())?;
        let evidence = run_factor_evidence(
            &local_state,
            &request,
            &candidate_view.candidate.candidate_hash,
            &feature_evidence,
            first_outputs,
        )
        .map_err(map_error)?;
        let mut run = demo_factor_run_with_outputs(
            Some(first_outputs),
            Some(replay_outputs),
            process_replay_exact,
        )
        .map_err(map_error)?;
        let candidate_hash = candidate_view.candidate.candidate_hash.clone();
        run.attempt_id = attempt_id;
        run.candidate_hash = Some(candidate_hash);
        run.project_revision_sha256 = Some(request.project_revision_sha256.clone());
        run.environment_sha256 = Some(request.environment_sha256.clone());
        run.input_bindings = Some(BTreeMap::from([(
            "close".into(),
            "host:market-close".into(),
        )]));
        run.normalized_parameters = Some(BTreeMap::from([("lookback".into(), "20".into())]));
        run.seed = Some(7);
        run.sdk_artifact_sha256 = Some(PUBLIC_SDK_ARTIFACT_SHA256.into());
        run.resource_policy = Some(python_factor_resource_policy(
            &HostResourcePolicy::m12_default(),
        ));
        run.snapshot_id = Some(feature_evidence.snapshot_id);
        run.snapshot_bindings = Some(feature_evidence.snapshot_bindings);
        run.point_in_time_universe_id =
            Some(sha256(b"python-tutorial-a-share@1:point-in-time-universe"));
        run.feature_dataset_id = Some(feature_evidence.dataset_id);
        run.feature_dataset_bindings = Some(feature_evidence.dataset_bindings);
        run.feature_evidence_sha256 = Some(feature_evidence.evidence_sha256);
        run.feature_plan_hash = Some(feature_evidence.plan_hash);
        run.engine_identity = Some("adaq-python-factor@1".into());
        run.repeatability_report_sha256 = Some(repeatability_report_sha256);
        run.repeatability_verified = repeatability_verified;
        run.repeatability_report = Some(repeatability_report);
        run.logs = logs;
        run.family_id = Some(evidence.family_id.to_string());
        run.trial_ids = evidence.trial_ids.iter().map(ToString::to_string).collect();
        run.dataset_ids = evidence.dataset_ids;
        run.report_hashes = evidence.report_hashes;
        run.promotion_policy_hash = Some(evidence.policy.policy_hash);
        run.promotion_protocol_hash = None;
        run.promotion_decision_hash = None;
        Ok(run)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn python_factor_trial_select(
    mut request: FactorTrialSelectionRequest,
    state: State<'_, Arc<PythonResearchState>>,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    app: tauri::AppHandle,
) -> Result<PythonFactorSelectionView, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    let _ = state;
    let local_state = app
        .state::<Arc<crate::local_research::LocalResearchState>>()
        .inner()
        .clone();
    tauri::async_runtime::spawn_blocking(move || {
        let family_id = uuid::Uuid::parse_str(&request.family_id)
            .map_err(|_| "python-factor-family-id-invalid".to_string())?;
        let trial_id = uuid::Uuid::parse_str(&request.trial_id)
            .map_err(|_| "python-factor-trial-id-invalid".to_string())?;
        let selection = local_state
            .factor
            .select_trial(crate::factor_research::FactorTrialSelectionRequest {
                user_id: request.user_id,
                candidate_hash: request.candidate_hash,
                family_id,
                trial_id,
                policy_hash: request.policy_hash,
            })
            .map_err(|error| error.to_string())?;
        Ok(PythonFactorSelectionView {
            candidate_hash: selection.candidate_hash,
            family_id: selection.family_id.to_string(),
            selected_trial_id: selection.selected_trial_id.to_string(),
            selection_hash: selection.selection_hash,
            promotion_protocol_hash: selection.promotion_protocol_hash,
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn python_factor_promote(
    mut request: FactorPromotionRequest,
    state: State<'_, Arc<PythonResearchState>>,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    app: tauri::AppHandle,
) -> Result<PythonFactorPromotionView, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    let _ = state;
    let local_state = app
        .state::<Arc<crate::local_research::LocalResearchState>>()
        .inner()
        .clone();
    tauri::async_runtime::spawn_blocking(move || {
        let trial_id = uuid::Uuid::parse_str(&request.trial_id)
            .map_err(|_| "python-factor-trial-id-invalid".to_string())?;
        let (selection, protocol) = local_state
            .factor
            .selected_trial(&request.user_id, &request.candidate_hash)
            .map_err(|error| error.to_string())?;
        if selection.selected_trial_id != trial_id {
            return Err("python-factor-promotion-trial-must-match-selection".into());
        }
        let decision = FactorPromotionDecision::freeze(PromotionDecisionDraft {
            decision_id: uuid::Uuid::new_v4(),
            user_id: crate::factor_research::user_uuid(&request.user_id),
            candidate_hash: request.candidate_hash.clone(),
            output_name: protocol.output_name.clone(),
            state: request.state,
            report_hashes: protocol.report_hashes.clone(),
            policy_hash: protocol.policy_hash.clone(),
            evidence_state: adaq_factor_research::EvaluationEvidenceState::OutOfSample,
            supersedes: None,
        })
        .map_err(|error| error.to_string())?;
        let decision_view = local_state
            .factor
            .save_decision(FactorDecisionSaveRequest {
                user_id: request.user_id.clone(),
                decision: decision.clone(),
                promotion_protocol: protocol.clone(),
                component: Default::default(),
            })?;
        Ok(PythonFactorPromotionView {
            candidate_hash: request.candidate_hash,
            family_id: selection.family_id.to_string(),
            selected_trial_id: selection.selected_trial_id.to_string(),
            selection_hash: selection.selection_hash,
            promotion_protocol_hash: selection.promotion_protocol_hash,
            decision_hash: decision.decision_hash,
            state: decision.state,
            eligibility_gates: decision_view.eligibility_gates,
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn model_experiment_register(
    mut request: ModelExperimentRequest,
    state: State<'_, Arc<PythonResearchState>>,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    app: tauri::AppHandle,
) -> Result<ModelExperiment, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    let store = state.model_lab_store.clone();
    let local_state = app
        .state::<Arc<crate::local_research::LocalResearchState>>()
        .inner()
        .clone();
    tauri::async_runtime::spawn_blocking(move || {
        let binding = local_state
            .factor
            .model_input_binding(&request.user_id, &request.factor_decision_hash)?;
        if model_input_evidence_hash(&binding).map_err(map_error)? != request.input_evidence_sha256
        {
            return Err("model-input-evidence-hash-mismatch".into());
        }
        let registered_run = store
            .run(&request.user_id, &request.attempt_id)
            .map_err(map_error)?;
        if registered_run.project_revision_sha256 != request.project_revision_sha256
            || registered_run.environment_sha256 != request.environment_sha256
            || registered_run.input_evidence_sha256 != request.input_evidence_sha256
            || registered_run.factor_decision_hash != request.factor_decision_hash
            || registered_run.seed != request.seed
        {
            return Err("model-experiment-attempt-binding-invalid".into());
        }
        if let Some(parent_decision_id) = request.derived_from_decision_id.as_deref() {
            store
                .decision(&request.user_id, parent_decision_id)
                .map_err(map_error)?;
            if !store
                .has_final(&request.user_id, parent_decision_id)
                .map_err(map_error)?
            {
                return Err("model-lineage-parent-final-evaluation-required".into());
            }
        }
        let experiment = ModelExperiment::ridge_with_binding_and_lineage(
            request.project_revision_sha256,
            request.environment_sha256,
            request.input_evidence_sha256,
            request.seed,
            registered_run.binding_sha256,
            request.derived_from_decision_id,
        )
        .map_err(map_error)?;
        let mut experiment = experiment;
        experiment.factor_decision_hash = request.factor_decision_hash;
        store
            .register(&request.user_id, experiment)
            .map_err(map_error)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn model_experiment_list(
    user_id: String,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<Vec<ModelExperiment>, String> {
    let _ = user_id;
    let user_id = auth.user_id_for_window(window.label())?;
    let store = state.model_lab_store.clone();
    tauri::async_runtime::spawn_blocking(move || {
        store.list_experiments(&user_id).map_err(map_error)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn model_lab_state(
    mut request: ModelLabStateRequest,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<ModelLabState, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    let store = state.model_lab_store.clone();
    tauri::async_runtime::spawn_blocking(move || {
        store
            .projection(&request.user_id, request.factor_decision_hash.as_deref())
            .map_err(map_error)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn model_trial_complete(
    mut request: ModelTrialCompleteRequest,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<ModelExperiment, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    let store = state.model_lab_store.clone();
    let attempt_store = state.attempt_store.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let experiment = store
            .experiment(&request.user_id, &request.experiment_id)
            .map_err(map_error)?;
        let trial = experiment
            .trials
            .iter()
            .find(|trial| trial.trial_id == request.trial_id)
            .cloned()
            .ok_or_else(|| "model-trial-not-found".to_string())?;
        let attempt = attempt_store.get(&request.attempt_id).map_err(map_error)?;
        if attempt.user_id != request.user_id
            || attempt.project_id != MODEL_PROJECT_ID
            || attempt.status != adaq_python_research::runner::AttemptStatus::Completed
            || attempt.revision_sha256 != trial.project_revision_sha256
            || attempt.environment_sha256 != trial.environment_sha256
        {
            return Err("model-trial-attempt-binding-invalid".into());
        }
        let run = store
            .run(&request.user_id, &request.attempt_id)
            .map_err(map_error)?;
        if run.input_evidence_sha256 != trial.input_evidence_sha256
            || run.project_revision_sha256 != trial.project_revision_sha256
            || run.environment_sha256 != trial.environment_sha256
            || run.seed != experiment.seed
            || run.attempt_id != request.attempt_id
            || run.alpha.to_bits() != trial.alpha.to_bits()
            || (!trial.binding_sha256.is_empty() && run.binding_sha256 != trial.binding_sha256)
        {
            return Err("model-trial-result-binding-invalid".into());
        }
        let selection_metric = run
            .selection_metric
            .ok_or_else(|| "model-selection-metric-missing".to_string())?;
        if !request.selection_metric.is_finite()
            || (request.selection_metric - selection_metric).abs() > RIDGE_REPEATABILITY_TOLERANCE
        {
            return Err("model-selection-metric-mismatch".into());
        }
        let result = if run.repeatability_verified {
            store.complete_trial_with_candidate(
                &request.user_id,
                &request.experiment_id,
                &request.trial_id,
                request.attempt_id.clone(),
                request.selection_metric,
                run.artifact_sha256,
            )
        } else {
            store.complete_trial_with_repeatability(
                &request.user_id,
                &request.experiment_id,
                &request.trial_id,
                request.attempt_id.clone(),
                request.selection_metric,
                run.repeatability_state,
            )
        };
        match result {
            Ok(experiment) => Ok(experiment),
            Err(error) => {
                let _ = attempt_store.transition(
                    &request.attempt_id,
                    AttemptTransition::RecordHostFailure {
                        code: "model-host-bind-failed".into(),
                        diagnostic: error.0.clone(),
                    },
                );
                Err(error.0)
            }
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn model_trial_fail(
    mut request: ModelTrialFailRequest,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<ModelExperiment, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    let store = state.model_lab_store.clone();
    let attempt_store = state.attempt_store.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let experiment = store
            .experiment(&request.user_id, &request.experiment_id)
            .map_err(map_error)?;
        let trial = experiment
            .trials
            .iter()
            .find(|trial| trial.trial_id == request.trial_id)
            .cloned()
            .ok_or_else(|| "model-trial-not-found".to_string())?;
        let attempt = attempt_store.get(&request.attempt_id).map_err(map_error)?;
        if attempt.user_id != request.user_id
            || attempt.project_id != MODEL_PROJECT_ID
            || attempt.revision_sha256 != trial.project_revision_sha256
            || attempt.environment_sha256 != trial.environment_sha256
            || !attempt_matches_model_trial_alpha(&attempt, &trial)
        {
            return Err("model-trial-attempt-binding-invalid".into());
        }
        let status = match attempt.status {
            adaq_python_research::runner::AttemptStatus::Cancelled => TrialStatus::Cancelled,
            adaq_python_research::runner::AttemptStatus::Interrupted => TrialStatus::Interrupted,
            adaq_python_research::runner::AttemptStatus::Stale => TrialStatus::Stale,
            adaq_python_research::runner::AttemptStatus::Failed => TrialStatus::Failed,
            adaq_python_research::runner::AttemptStatus::Completed
                if attempt.failure_code.is_some() =>
            {
                TrialStatus::Failed
            }
            _ => return Err("model-trial-failure-requires-terminal-attempt".into()),
        };
        let diagnostic = attempt
            .diagnostic
            .clone()
            .or(attempt.failure_code.clone())
            .unwrap_or_default();
        store
            .fail_trial(
                &request.user_id,
                &request.experiment_id,
                &request.trial_id,
                request.attempt_id,
                status,
                diagnostic,
            )
            .map_err(map_error)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn model_trial_retry(
    mut request: ModelTrialRetryRequest,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<ModelExperiment, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    let store = state.model_lab_store.clone();
    let attempt_store = state.attempt_store.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let attempt = attempt_store.get(&request.attempt_id).map_err(map_error)?;
        let experiment = store
            .experiment(&request.user_id, &request.experiment_id)
            .map_err(map_error)?;
        let trial = experiment
            .trials
            .iter()
            .find(|trial| trial.trial_id == request.trial_id)
            .cloned()
            .ok_or_else(|| "model-trial-not-found".to_string())?;
        let alpha = attempt
            .execution
            .parameters
            .get("alpha")
            .and_then(|value| value.parse::<f64>().ok());
        let retryable = matches!(
            attempt.status,
            PythonAttemptStatus::Failed
                | PythonAttemptStatus::Cancelled
                | PythonAttemptStatus::Interrupted
                | PythonAttemptStatus::Stale
        ) || (attempt.status == PythonAttemptStatus::Completed
            && attempt.failure_code.is_some());
        if attempt.user_id != request.user_id
            || attempt.project_id != MODEL_PROJECT_ID
            || attempt.revision_sha256 != trial.project_revision_sha256
            || attempt.environment_sha256 != trial.environment_sha256
            || !alpha.is_some_and(|alpha| alpha.to_bits() == trial.alpha.to_bits())
            || !retryable
        {
            return Err("model-trial-retry-attempt-binding-invalid".into());
        }
        let source_status = match attempt.status {
            PythonAttemptStatus::Cancelled => TrialStatus::Cancelled,
            PythonAttemptStatus::Interrupted => TrialStatus::Interrupted,
            PythonAttemptStatus::Stale => TrialStatus::Stale,
            PythonAttemptStatus::Failed | PythonAttemptStatus::Completed => TrialStatus::Failed,
            _ => return Err("model-trial-retry-attempt-binding-invalid".into()),
        };
        let diagnostic = attempt
            .diagnostic
            .clone()
            .or(attempt.failure_code.clone())
            .unwrap_or_default();
        store
            .retry_trial_from_attempt(
                &request.user_id,
                &request.experiment_id,
                &request.trial_id,
                &request.attempt_id,
                source_status,
                diagnostic,
            )
            .map_err(map_error)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn model_selection_record(
    mut request: ModelSelectionRequest,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<ParameterSelectionDecision, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    let store = state.model_lab_store.clone();
    tauri::async_runtime::spawn_blocking(move || {
        store
            .select(&request.user_id, &request.experiment_id, &request.trial_id)
            .map_err(map_error)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn model_final_evaluate(
    mut request: ModelFinalEvaluationRequest,
    state: State<'_, Arc<PythonResearchState>>,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    app: tauri::AppHandle,
) -> Result<FinalEvaluationReport, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    let store = state.model_lab_store.clone();
    let research_state = state.inner().clone();
    let local_state = app
        .state::<Arc<crate::local_research::LocalResearchState>>()
        .inner()
        .clone();
    tauri::async_runtime::spawn_blocking(move || {
        if let Some(report) = store
            .final_report(&request.user_id, &request.decision_id)
            .map_err(map_error)?
        {
            return Ok(report);
        }
        let retry_attempt_id = store
            .begin_final(&request.user_id, &request.decision_id)
            .map_err(map_error)?;
        let mut active_attempt_id = None;
        let mut staged_dataset_sha256 = false;
        let result = (|| -> Result<FinalEvaluationReport, String> {
            let decision = store
                .decision(&request.user_id, &request.decision_id)
                .map_err(map_error)?;
            let experiment = store
                .experiment(&request.user_id, &decision.experiment_id)
                .map_err(map_error)?;
            if decision.binding_sha256 != experiment.binding_sha256
                || decision.project_revision_sha256 != experiment.project_revision_sha256
                || decision.environment_sha256 != experiment.environment_sha256
                || decision.input_evidence_sha256 != experiment.input_evidence_sha256
                || decision.seed != experiment.seed
            {
                return Err("model-selection-decision-binding-invalid".into());
            }
            let trial = experiment
                .trials
                .iter()
                .find(|trial| trial.trial_id == decision.selected_trial_id)
                .ok_or_else(|| "model-selection-trial-not-found".to_string())?;
            let candidate_artifact_sha256 = trial
                .candidate_artifact_sha256
                .clone()
                .ok_or_else(|| "model-selection-candidate-artifact-missing".to_string())?;
            let successful_attempt_id = trial
                .successful_attempt_id
                .clone()
                .ok_or_else(|| "model-selection-successful-attempt-missing".to_string())?;
            if trial.alpha.to_bits() != decision.selected_alpha.to_bits() {
                return Err("model-selection-alpha-mismatch".into());
            }
            if decision.candidate_artifact_sha256 != candidate_artifact_sha256 {
                return Err("model-selection-candidate-artifact-binding-invalid".into());
            }
            let candidate_artifact = LinearModelArtifact::reload(
                &store
                    .artifact(&request.user_id, &candidate_artifact_sha256)
                    .map_err(map_error)?,
            )
            .map_err(map_error)?;
            let prior_run = store
                .run(&request.user_id, &successful_attempt_id)
                .map_err(map_error)?;
            if prior_run.attempt_id != successful_attempt_id
                || prior_run.artifact_sha256 != candidate_artifact_sha256
                || prior_run.seed != experiment.seed
                || prior_run.alpha.to_bits() != trial.alpha.to_bits()
                || (!trial.binding_sha256.is_empty()
                    && prior_run.binding_sha256 != trial.binding_sha256)
            {
                return Err("model-selection-binding-mismatch".into());
            }
            let factor_binding = local_state
                .factor
                .model_input_binding(&request.user_id, &prior_run.factor_decision_hash)?;
            if prior_run.factor_promotion_protocol_hash
                != factor_binding.promotion_protocol.protocol_hash
                || prior_run.factor_dataset_id != factor_binding.factor_dataset_id
                || prior_run.feature_dataset_id != factor_binding.feature_dataset_id
                || prior_run.feature_plan_hash != factor_binding.feature_plan_hash
                || prior_run.snapshot_id != factor_binding.snapshot_id
                || prior_run.universe_id != factor_binding.universe_id
            {
                return Err("model-factor-input-binding-changed".into());
            }
            let input = ModelInputEvidence {
                decision_hash: factor_binding.decision_hash,
                promotion_protocol_hash: factor_binding.promotion_protocol.protocol_hash,
                factor_dataset_id: factor_binding.factor_dataset_id,
                feature_dataset_id: factor_binding.feature_dataset_id,
                feature_plan_hash: factor_binding.feature_plan_hash,
                snapshot_id: factor_binding.snapshot_id,
                universe_id: factor_binding.universe_id,
                lookback: factor_binding.lookback,
            };
            let factor_dataset =
                load_bound_model_factor_dataset(&local_state, &request.user_id, &input)
                    .map_err(map_error)?;
            let evidence =
                build_model_evidence(&input, Some(&factor_dataset)).map_err(map_error)?;
            let runner_input = evidence.runner_input().map_err(map_error)?;
            let runner_input =
                serde_json::to_value(runner_input).map_err(|error| error.to_string())?;
            let parameters =
                BTreeMap::from([("alpha".into(), decision.selected_alpha.to_string())]);
            let execution = if let Some(source_attempt_id) = retry_attempt_id.as_deref() {
                let retryable = match research_state.attempt_store.get(source_attempt_id) {
                    Ok(attempt) => {
                        matches!(
                            attempt.status,
                            PythonAttemptStatus::Failed
                                | PythonAttemptStatus::Cancelled
                                | PythonAttemptStatus::Interrupted
                                | PythonAttemptStatus::Stale
                        ) || (attempt.status == PythonAttemptStatus::Completed
                            && attempt.failure_code.is_some())
                    }
                    Err(_) => false,
                };
                if retryable {
                    research_state
                        .run_trusted_project_with_retry(
                            &request.user_id,
                            MODEL_PROJECT_ID,
                            &trial.project_revision_sha256,
                            &trial.environment_sha256,
                            experiment.seed,
                            Some(runner_input.clone()),
                            Some(&parameters),
                            source_attempt_id,
                        )
                        .map_err(map_error)?
                } else {
                    research_state
                        .run_trusted_project_with_execution(
                            &request.user_id,
                            MODEL_PROJECT_ID,
                            &trial.project_revision_sha256,
                            &trial.environment_sha256,
                            experiment.seed,
                            Some(runner_input.clone()),
                            Some(&parameters),
                        )
                        .map_err(map_error)?
                }
            } else {
                research_state
                    .run_trusted_project_with_execution(
                        &request.user_id,
                        MODEL_PROJECT_ID,
                        &trial.project_revision_sha256,
                        &trial.environment_sha256,
                        experiment.seed,
                        Some(runner_input.clone()),
                        Some(&parameters),
                    )
                    .map_err(map_error)?
            };
            let (replay_execution, replay_resource_policy) = research_state
                .run_trusted_project_verification(
                    &request.user_id,
                    MODEL_PROJECT_ID,
                    &trial.project_revision_sha256,
                    &trial.environment_sha256,
                    experiment.seed,
                    Some(runner_input.clone()),
                    Some(&parameters),
                )
                .map_err(map_error)?;
            let attempt_id =
                validate_model_process_replay(&execution, &replay_execution).map_err(map_error)?;
            store
                .bind_final_attempt(&request.user_id, &request.decision_id, &attempt_id)
                .map_err(map_error)?;
            active_attempt_id = Some(attempt_id.clone());
            let (resource_policy, resource_policy_sha256) = model_replay_resource_policy(
                &research_state.attempt_store,
                &request.user_id,
                &execution,
                &replay_resource_policy,
            )
            .map_err(map_error)?;
            let first_artifact = read_model_candidate(
                &research_state.root,
                &execution,
                decision.selected_alpha,
                &evidence.transformation,
                &evidence.fixture.manifest.content_sha256,
                &trial.project_revision_sha256,
                &trial.environment_sha256,
                &trial.input_evidence_sha256,
                &resource_policy_sha256,
                &input,
            )
            .map_err(map_error)?;
            let replay_artifact = read_model_candidate(
                &research_state.root,
                &replay_execution,
                decision.selected_alpha,
                &evidence.transformation,
                &evidence.fixture.manifest.content_sha256,
                &trial.project_revision_sha256,
                &trial.environment_sha256,
                &trial.input_evidence_sha256,
                &resource_policy_sha256,
                &input,
            )
            .map_err(map_error)?;
            if first_artifact.to_bytes().map_err(map_error)?
                != candidate_artifact.to_bytes().map_err(map_error)?
                || replay_artifact.to_bytes().map_err(map_error)?
                    != candidate_artifact.to_bytes().map_err(map_error)?
            {
                return Err("model-selection-candidate-artifact-mismatch".into());
            }
            discard_verification_artifact(&research_state.root, &replay_execution)
                .map_err(map_error)?;
            let mut run = demo_model_run_with_evidence(
                decision.selected_alpha,
                trial.project_revision_sha256.clone(),
                trial.environment_sha256.clone(),
                trial.input_evidence_sha256.clone(),
                input.clone(),
                resource_policy.clone(),
                Some(candidate_artifact.clone()),
                Some(&factor_dataset),
            )
            .map_err(map_error)?;
            let prediction_input =
                model_prediction_input(runner_input, &candidate_artifact).map_err(map_error)?;
            let (prediction_execution, _) = research_state
                .run_trusted_project_verification(
                    &request.user_id,
                    MODEL_PROJECT_ID,
                    &trial.project_revision_sha256,
                    &trial.environment_sha256,
                    experiment.seed,
                    Some(prediction_input),
                    Some(&parameters),
                )
                .map_err(map_error)?;
            run.forecasts = validate_model_python_forecast(&prediction_execution, &run.forecasts)
                .map_err(map_error)?;
            run.view.forecast_sha256 = model_forecast_sha256(
                &run.artifact.artifact_sha256,
                &run.view.input_evidence_sha256,
                &run.view.snapshot_id,
                &run.view.universe_id,
                &run.forecasts,
            )
            .map_err(map_error)?;
            discard_verification_artifact(&research_state.root, &prediction_execution)
                .map_err(map_error)?;
            let replay = demo_model_run_with_evidence(
                decision.selected_alpha,
                trial.project_revision_sha256.clone(),
                trial.environment_sha256.clone(),
                trial.input_evidence_sha256.clone(),
                input,
                resource_policy,
                Some(candidate_artifact.clone()),
                Some(&factor_dataset),
            )
            .map_err(map_error)?;
            compare_repeatability(
                &run.artifact.coefficients,
                &replay.artifact.coefficients,
                &run.forecasts,
                &replay.forecasts,
            )
            .map_err(map_error)?;
            run.view.repeatability_verified = true;
            run.view.repeatability_state = RepeatabilityState::Verified;
            run.view.attempt_id = attempt_id.clone();
            if run.view.binding_sha256 != trial.binding_sha256
                || run.artifact.artifact_sha256 != candidate_artifact_sha256
            {
                return Err("model-final-binding-mismatch".into());
            }
            store
                .save_demo_run(&request.user_id, &run, true)
                .map_err(map_error)?;
            store
                .stage_final_dataset(
                    &request.user_id,
                    &request.decision_id,
                    &run.view.forecast_sha256,
                )
                .map_err(map_error)?;
            staged_dataset_sha256 = true;
            crate::forecast_signal_dataset::publish_python_model_signal_dataset(
                &local_state,
                &request.user_id,
                &run.view.forecast_sha256,
                &run.view.snapshot_id,
                &run.view.feature_plan_hash,
                &run.view.factor_dataset_id,
                &run.view.feature_dataset_id,
                &run.view.artifact_sha256,
                &run.artifact.provenance_hashes,
                &run.view.adapter_id,
                run.view.alpha,
                run.view.seed,
                &run.view.forecast_contract,
                &run.forecasts,
            )
            .map_err(|error| PythonResearchError(error).to_string())?;
            let final_start = run.view.windows.final_start as i64;
            let final_end = run.view.windows.final_end - TARGET_HORIZON_BARS as u32;
            let forecasts = run
                .forecasts
                .iter()
                .filter(|row| {
                    row.datetime >= final_start
                        && row.datetime as u32 <= final_end
                        && row.value.is_some()
                })
                .cloned()
                .collect::<Vec<_>>();
            let mut ledger = FinalEvaluationLedger::default();
            let report = ledger
                .run_with_evidence(
                    &decision,
                    &forecasts,
                    &run.final_labels,
                    &candidate_artifact_sha256,
                    &run.view.forecast_sha256,
                    EvidenceState::OutOfSample,
                )
                .map_err(map_error)?;
            store
                .save_final_with_attempt(&request.user_id, report, Some(&attempt_id))
                .map_err(map_error)
        })();
        if let Err(error) = &result {
            if let Some(attempt_id) = active_attempt_id.as_deref() {
                let _ = research_state.attempt_store.transition(
                    attempt_id,
                    AttemptTransition::RecordHostFailure {
                        code: "model-final-evaluation-failed".into(),
                        diagnostic: error.clone(),
                    },
                );
            }
            let status = model_final_evaluation_failure_status(error, staged_dataset_sha256);
            let failure_code = match status {
                ModelFinalEvaluationStatus::Cancelled => "model-final-evaluation-cancelled",
                ModelFinalEvaluationStatus::Interrupted => "model-final-evaluation-interrupted",
                ModelFinalEvaluationStatus::Stale => "model-final-evaluation-stale",
                ModelFinalEvaluationStatus::PersistenceFailed => {
                    "model-final-evaluation-persistence-failed"
                }
                ModelFinalEvaluationStatus::Failed => "model-final-evaluation-failed",
                ModelFinalEvaluationStatus::Pending
                | ModelFinalEvaluationStatus::Running
                | ModelFinalEvaluationStatus::Completed => "model-final-evaluation-failed",
            };
            let _ = store.fail_final(
                &request.user_id,
                &request.decision_id,
                status,
                failure_code,
                error,
            );
        }
        result
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn model_qualify_deployment(
    mut request: ModelDeploymentQualificationRequest,
    state: State<'_, Arc<PythonResearchState>>,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    app: tauri::AppHandle,
) -> Result<ModelRuntimeQualificationReport, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    let store = state.model_lab_store.clone();
    let local_state = app
        .state::<Arc<crate::local_research::LocalResearchState>>()
        .inner()
        .clone();
    tauri::async_runtime::spawn_blocking(move || {
        qualify_model_deployment(&store, &local_state, &request.user_id, &request.decision_id)
            .map_err(map_error)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub fn runtime_profile(
    mut request: RuntimeProfileRequest,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<RuntimeProfileView, String> {
    request.user_id = Some(auth.user_id_for_window(window.label())?);
    let platform = RuntimePlatform::current().ok();
    let catalog = platform.and_then(|platform| runtime_catalog_entry(platform).ok());
    let wheelhouse = platform.and_then(|platform| wheelhouse_catalog(platform).ok());
    let runtime_directory = state.root.join("runtimes");
    let environment_directory = state.root.join("environments");
    let runtime_store = state.runtime_store.clone();
    let wheelhouse_store = state.wheelhouse_store.clone();
    let latest_preparation = match request.user_id.as_deref() {
        Some(user_id) => runtime_store
            .list_preparations(user_id)
            .map_err(map_error)?
            .into_iter()
            .max_by(|left, right| left.attempt_id.cmp(&right.attempt_id)),
        None => None,
    };
    let preparation_progress = state
        .runtime_progress
        .lock()
        .map_err(|_| "python-runtime-progress-store-lock-poisoned".to_string())?
        .get(request.user_id.as_deref().unwrap_or_default())
        .cloned();
    let ready = catalog.as_ref().and_then(|entry| {
        let identity = entry.manifest.artifact_sha256.clone();
        runtime_store
            .ready_record(&entry.manifest)
            .ok()
            .map(|_| identity)
    });
    let runtime_disabled = catalog.as_ref().is_some_and(|entry| {
        runtime_directory
            .join(&entry.manifest.artifact_sha256)
            .join("adaq-runtime-disabled")
            .is_file()
    });
    let wheelhouse_status = wheelhouse
        .as_ref()
        .map(|entry| {
            if wheelhouse_store.load(&entry.manifest.identity).is_ok() {
                "ready"
            } else {
                "missing"
            }
        })
        .unwrap_or("unavailable");
    let status = match latest_preparation.as_ref().map(|attempt| &attempt.status) {
        Some(adaq_python_research::runtime::PreparationStatus::Preparing) => "preparing",
        _ if runtime_disabled => "disabled",
        _ if ready.is_some() => "ready",
        Some(adaq_python_research::runtime::PreparationStatus::Failed) => "failed",
        Some(adaq_python_research::runtime::PreparationStatus::Cancelled) => "cancelled",
        _ => "missing",
    };
    Ok(RuntimeProfileView {
        profile: "adaq-python@1".into(),
        platform,
        status: status.into(),
        preparation_status: latest_preparation
            .as_ref()
            .map(|attempt| attempt.status.clone()),
        preparation_attempt_id: latest_preparation
            .as_ref()
            .map(|attempt| attempt.attempt_id.clone()),
        preparation_diagnostic: latest_preparation
            .as_ref()
            .and_then(|attempt| attempt.diagnostic.clone()),
        preparation_completed_bytes: preparation_progress
            .as_ref()
            .map(|progress| progress.completed_bytes),
        preparation_total_bytes: preparation_progress
            .as_ref()
            .map(|progress| progress.total_bytes),
        expected_version: catalog
            .as_ref()
            .map(|entry| entry.manifest.version.clone())
            .unwrap_or_else(|| "3.12.x".into()),
        source: catalog
            .as_ref()
            .map(|entry| entry.manifest.source.clone())
            .unwrap_or_else(|| "ADAQ-managed signed Runtime catalog".into()),
        artifact_sha256: catalog
            .as_ref()
            .map(|entry| entry.manifest.artifact_sha256.clone()),
        download_bytes: catalog.as_ref().map(|entry| entry.manifest.download_bytes),
        installed_bytes: catalog.as_ref().map(|entry| entry.manifest.installed_bytes),
        license: catalog.as_ref().map(|entry| entry.manifest.license.clone()),
        wheelhouse_identity: wheelhouse
            .as_ref()
            .map(|entry| entry.manifest.identity.clone()),
        wheelhouse_status: wheelhouse_status.into(),
        wheelhouse_wheel_count: wheelhouse
            .as_ref()
            .map(|entry| entry.manifest.wheels.len())
            .unwrap_or_default(),
        runtime_cache_bytes: catalog
            .as_ref()
            .map(|entry| directory_bytes(&runtime_directory.join(&entry.manifest.artifact_sha256)))
            .unwrap_or_default(),
        wheelhouse_disk_bytes: wheelhouse
            .as_ref()
            .map(|entry| wheelhouse_store.disk_bytes(&entry.manifest.identity))
            .unwrap_or_default(),
        environment_cache_bytes: directory_bytes(&environment_directory),
        environment_count: fs::read_dir(&environment_directory)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .count(),
    })
}

#[tauri::command]
pub async fn runtime_prepare(
    mut request: RuntimePrepareRequest,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<(PreparationAttempt, Option<RuntimeRecord>), String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    let store = state.runtime_store.clone();
    tauri::async_runtime::spawn_blocking(move || {
        store
            .prepare(
                &request.user_id,
                &request.manifest,
                &request.payload,
                || false,
            )
            .map_err(map_error)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn runtime_prepare_managed(
    mut request: ManagedRuntimePrepareRequest,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<(PreparationAttempt, Option<RuntimeRecord>), String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    let task_id = request
        .task_id
        .filter(|task_id| !task_id.trim().is_empty())
        .unwrap_or_else(|| format!("runtime-prepare-{}", runner_token()));
    let cancelled = Arc::new(AtomicBool::new(false));
    {
        let mut active = state
            .runtime_cancellations
            .lock()
            .map_err(|_| "python-runtime-cancellation-store-lock-poisoned".to_string())?;
        if active.contains_key(&task_id) {
            return Err("python-runtime-preparation-already-running".into());
        }
        active.insert(task_id.clone(), cancelled.clone());
    }
    let store = state.runtime_store.clone();
    let managed_runtime_gate = state.managed_runtime_gate.clone();
    let runtime_progress = state.runtime_progress.clone();
    let source_attempt_id = request.source_attempt_id;
    let user_id = request.user_id;
    let result = tauri::async_runtime::spawn_blocking(move || {
        let _managed_runtime_gate = managed_runtime_gate
            .lock()
            .map_err(|_| "python-runtime-managed-gate-poisoned".to_string())?;
        let platform = RuntimePlatform::current().map_err(map_error)?;
        let entry = runtime_catalog_entry(platform).map_err(map_error)?;
        set_runtime_progress(
            &runtime_progress,
            &user_id,
            0,
            entry.manifest.download_bytes,
        )?;
        if cancelled.load(Ordering::Relaxed) {
            return store
                .record_cancelled(
                    &user_id,
                    source_attempt_id.clone(),
                    Some(entry.manifest.artifact_sha256.clone()),
                )
                .map(|attempt| (attempt, None))
                .map_err(map_error);
        }
        if let Some(record) = store.cached_record(&entry.manifest) {
            set_runtime_progress(
                &runtime_progress,
                &user_id,
                entry.manifest.download_bytes,
                entry.manifest.download_bytes,
            )?;
            return store
                .record_ready(&user_id, source_attempt_id, entry.manifest.artifact_sha256)
                .map(|attempt| (attempt, Some(record)))
                .map_err(map_error);
        }
        let download = (|| -> Result<Vec<u8>, String> {
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(120))
                .build()
                .map_err(|error| format!("python-runtime-client-failed:{error}"))?;
            let response = client
                .get(&entry.download_url)
                .send()
                .map_err(|error| format!("python-runtime-download-failed:{error}"))?;
            if !response.status().is_success() {
                return Err(format!(
                    "python-runtime-download-http-{}",
                    response.status().as_u16()
                ));
            }
            let mut response = response;
            let mut payload = Vec::with_capacity(entry.manifest.download_bytes as usize);
            let mut chunk = [0_u8; 64 * 1024];
            loop {
                if cancelled.load(Ordering::Relaxed) {
                    return Err("python-runtime-preparation-cancelled".into());
                }
                let read = response
                    .read(&mut chunk)
                    .map_err(|error| format!("python-runtime-download-read-failed:{error}"))?;
                if read == 0 {
                    break;
                }
                payload.extend_from_slice(&chunk[..read]);
                if payload.len() as u64 > entry.manifest.download_bytes {
                    return Err("python-runtime-download-size-mismatch".into());
                }
                set_runtime_progress(
                    &runtime_progress,
                    &user_id,
                    payload.len() as u64,
                    entry.manifest.download_bytes,
                )?;
            }
            if payload.len() as u64 != entry.manifest.download_bytes {
                return Err("python-runtime-download-size-mismatch".into());
            }
            Ok(payload)
        })();
        let payload = match download {
            Ok(payload) => payload,
            Err(_error) if cancelled.load(Ordering::Relaxed) => {
                return store
                    .record_cancelled(
                        &user_id,
                        source_attempt_id.clone(),
                        Some(entry.manifest.artifact_sha256.clone()),
                    )
                    .map(|attempt| (attempt, None))
                    .map_err(map_error);
            }
            Err(error) => {
                let _ = store.record_failed(
                    &user_id,
                    source_attempt_id.clone(),
                    Some(entry.manifest.artifact_sha256.clone()),
                    error.clone(),
                );
                return Err(error);
            }
        };
        let prepared = store
            .prepare_with_source(
                &user_id,
                source_attempt_id,
                &entry.manifest,
                &payload,
                || cancelled.load(Ordering::Relaxed),
            )
            .map_err(map_error)?;
        set_runtime_progress(
            &runtime_progress,
            &user_id,
            entry.manifest.download_bytes,
            entry.manifest.download_bytes,
        )?;
        Ok(prepared)
    })
    .await
    .map_err(|error| error.to_string())?;
    state
        .runtime_cancellations
        .lock()
        .map_err(|_| "python-runtime-cancellation-store-lock-poisoned".to_string())?
        .remove(&task_id);
    result
}

#[tauri::command]
pub fn runtime_prepare_cancel(
    request: RuntimePrepareCancelRequest,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<(), String> {
    if request.task_id.trim().is_empty() {
        return Err("python-runtime-task-id-invalid".into());
    }
    if let Some(cancelled) = state
        .runtime_cancellations
        .lock()
        .map_err(|_| "python-runtime-cancellation-store-lock-poisoned".to_string())?
        .get(&request.task_id)
    {
        cancelled.store(true, Ordering::Relaxed);
    }
    Ok(())
}

#[tauri::command]
pub async fn environment_sync_managed(
    mut request: ManagedEnvironmentPrepareRequest,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<EnvironmentSyncResult, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    let project_store = state.store.clone();
    let runtime_store = state.runtime_store.clone();
    let wheelhouse_store = state.wheelhouse_store.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let platform = RuntimePlatform::current().map_err(map_error)?;
        let runtime = runtime_catalog_entry(platform).map_err(map_error)?;
        runtime_store
            .ready_record(&runtime.manifest)
            .map_err(map_error)?;
        let catalog = wheelhouse_catalog(platform).map_err(map_error)?;
        let intent = project_store
            .dependency_intent(&request.user_id, &request.project_id)
            .map_err(map_error)?;
        let payloads = match wheelhouse_store.load(&catalog.manifest.identity) {
            Ok((_, payloads)) => payloads,
            Err(_) => {
                let payloads = download_managed_wheelhouse(&catalog)?;
                wheelhouse_store
                    .prepare(&catalog.manifest, &payloads)
                    .map_err(map_error)?;
                payloads
            }
        };
        let lock = sync_environment(
            &runtime.manifest.artifact_sha256,
            platform,
            &intent,
            &catalog.manifest,
            &payloads,
        )
        .map_err(map_error)?;
        let lock_sha256 = project_store
            .apply_environment_lock(&request.user_id, &request.project_id, &lock)
            .map_err(map_error)?;
        let wheelhouse = wheelhouse_store
            .prepare(&catalog.manifest, &payloads)
            .map_err(map_error)?;
        Ok(EnvironmentSyncResult {
            lock_sha256,
            wheelhouse,
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub fn environment_sync(request: EnvironmentSyncRequest) -> Result<EnvironmentLock, String> {
    sync_environment(
        &request.runtime_artifact_sha256,
        request.platform,
        &request.intent,
        &request.wheelhouse,
        &request.payloads,
    )
    .map_err(map_error)
}

#[tauri::command]
pub async fn environment_prepare(
    request: EnvironmentPrepareRequest,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<EnvironmentRecord, String> {
    let store = state.environment_store.clone();
    tauri::async_runtime::spawn_blocking(move || {
        store
            .prepare(&request.lock, &request.payloads, &request.wheelhouse)
            .map_err(map_error)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn environment_prepare_managed(
    mut request: ManagedEnvironmentPrepareRequest,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<EnvironmentRecord, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    let project_store = state.store.clone();
    let runtime_store = state.runtime_store.clone();
    let wheelhouse_store = state.wheelhouse_store.clone();
    let environment_store = state.environment_store.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let platform = RuntimePlatform::current().map_err(map_error)?;
        let runtime = runtime_catalog_entry(platform).map_err(map_error)?;
        runtime_store
            .ready_record(&runtime.manifest)
            .map_err(map_error)?;
        let catalog = wheelhouse_catalog(platform).map_err(map_error)?;
        let lock_bytes = project_store
            .dependency_lock_bytes(&request.user_id, &request.project_id)
            .map_err(map_error)?;
        let lock = parse_environment_lock(&lock_bytes)
            .map_err(map_error)
            .map_err(|error| format!("python-environment-lock-not-ready:{error}"))?;
        if lock.runtime_artifact_sha256 != runtime.manifest.artifact_sha256
            || lock.platform != platform
            || lock.wheelhouse_identity != catalog.manifest.identity
        {
            return Err("python-environment-lock-identity-mismatch".into());
        }
        let lock_file_sha256 = sha256(&lock_bytes);
        if let Some(record) = environment_store
            .find_by_lock_file_sha256(&lock_file_sha256)
            .map_err(map_error)?
        {
            return Ok(record);
        }
        let (_, payloads) = wheelhouse_store
            .load(&catalog.manifest.identity)
            .map_err(map_error)?;
        let record = environment_store
            .prepare(&lock, &payloads, &catalog.manifest)
            .map_err(map_error)?;
        Ok(record)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn environment_for_project(
    mut request: ProjectRequest,
    window: WebviewWindow,
    auth: State<'_, AuthState>,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<Option<EnvironmentRecord>, String> {
    request.user_id = auth.user_id_for_window(window.label())?;
    let project_store = state.store.clone();
    let environment_store = state.environment_store.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let lock_hash = project_store
            .dependency_lock_sha256(&request.user_id, &request.project_id)
            .map_err(map_error)?;
        environment_store
            .find_by_lock_file_sha256(&lock_hash)
            .map_err(map_error)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn cache_evict(
    request: CacheEvictRequest,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<CacheEvictResult, String> {
    let runtime_store = state.runtime_store.clone();
    let wheelhouse_store = state.wheelhouse_store.clone();
    let environment_store = state.environment_store.clone();
    let attempt_store = state.attempt_store.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let active_environments = request
            .active_environments
            .into_iter()
            .chain(attempt_store.active_environment_ids().map_err(map_error)?)
            .collect::<BTreeSet<_>>();
        let mut active_runtime_artifacts = request
            .active_runtime_artifacts
            .into_iter()
            .collect::<BTreeSet<_>>();
        let mut active_wheelhouses = request
            .active_wheelhouses
            .into_iter()
            .collect::<BTreeSet<_>>();
        for environment in &active_environments {
            let lock = environment_store
                .load_lock(environment)
                .map_err(map_error)?;
            active_runtime_artifacts.insert(lock.runtime_artifact_sha256);
            active_wheelhouses.insert(lock.wheelhouse_identity);
        }
        let runtimes = runtime_store
            .evict_inactive(&active_runtime_artifacts)
            .map_err(map_error)?;
        let wheelhouses = wheelhouse_store
            .evict_inactive(&active_wheelhouses)
            .map_err(map_error)?;
        let environments = environment_store
            .evict_inactive(&active_environments)
            .map_err(map_error)?;
        Ok(CacheEvictResult {
            runtimes,
            wheelhouses,
            environments,
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use adaq_python_research::runner::ConformanceResult;

    #[test]
    fn native_model_evidence_windows_require_ordered_ranges() {
        let mut provenance = BTreeMap::new();
        assert!(!has_model_evidence_windows(&provenance));
        for field in ["trainingWindow", "fittingWindow", "normalizationWindow"] {
            provenance.insert(field.into(), "1..100".into());
        }
        assert!(has_model_evidence_windows(&provenance));
        provenance.insert("fittingWindow".into(), "100..1".into());
        assert!(!has_model_evidence_windows(&provenance));
    }

    #[test]
    fn qualified_model_report_reuse_includes_package_evidence_identity() {
        let directory =
            std::env::temp_dir().join(format!("adaq-model-report-reuse-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let store = ModelLabStore::open(directory.clone()).unwrap();
        let mut existing = model_qualification_failure_report(
            &store,
            "alice",
            "decision",
            "attempt-1".into(),
            "diagnostic",
        )
        .unwrap();
        existing.qualified = true;
        existing.package_archive_sha256 = Some("a".repeat(64));
        let mut incoming = existing.clone();
        incoming.attempt_id = "attempt-2".into();
        assert!(same_qualified_model_report(&existing, &incoming));
        incoming.evidence_windows_complete = true;
        assert!(!same_qualified_model_report(&existing, &incoming));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn failed_model_qualification_is_persisted_scoped_and_retryable() {
        let directory = std::env::temp_dir().join(format!(
            "adaq-model-qualification-failure-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("model-lab.json");
        let store = ModelLabStore::open(path.clone()).unwrap();
        let first = model_qualification_failure_report(
            &store,
            "alice",
            "decision",
            "attempt-1".into(),
            "unsupported schema SECRET /private/model.json",
        )
        .unwrap();
        let first = store.save_qualification_report("alice", first).unwrap();
        assert!(first.diagnostics[0].contains("[redacted]"));
        assert!(first.diagnostics[0].contains("[path]"));
        assert_eq!(
            store
                .save_qualification_report("alice", first.clone())
                .unwrap(),
            first
        );

        let second = model_qualification_failure_report(
            &store,
            "alice",
            "decision",
            "attempt-2".into(),
            "retry failed",
        )
        .unwrap();
        let second = store.save_qualification_report("alice", second).unwrap();
        assert_ne!(first.report_id, second.report_id);
        assert_eq!(
            store
                .qualification_reports("alice", "decision")
                .unwrap()
                .len(),
            2
        );
        assert!(
            store
                .qualification_reports("bob", "decision")
                .unwrap()
                .is_empty()
        );
        assert!(
            store
                .qualification_reports("alice:foreign", "decision")
                .is_err()
        );

        let reopened = ModelLabStore::open(path).unwrap();
        assert_eq!(
            reopened
                .qualification_reports("alice", "decision")
                .unwrap()
                .len(),
            2
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn staged_result_publication_is_atomic_and_attempt_scoped() {
        let root =
            std::env::temp_dir().join(format!("adaq-python-publication-{}", uuid::Uuid::new_v4()));
        let staging = root.join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        let bytes = br#"{"attemptId":"attempt","projectId":"py-factor-test"}"#;
        std::fs::write(staging.join("result.json"), bytes).unwrap();
        let artifact = StagedArtifact {
            attempt_id: "attempt".into(),
            relative_path: "result.json".into(),
            media_type: "application/json".into(),
            byte_size: bytes.len() as u64,
            sha256: sha256(bytes),
            columns: None,
            row_count: None,
        };
        publish_attempt_artifact(&root, "attempt", &staging, &artifact).unwrap();
        assert_eq!(
            std::fs::read(root.join("attempt-results/attempt.artifact")).unwrap(),
            bytes
        );
        assert!(publish_attempt_artifact(&root, "attempt", &staging, &artifact).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reset_barrier_rejects_new_attempts() {
        let directory =
            std::env::temp_dir().join(format!("adaq-python-reset-{}", uuid::Uuid::new_v4()));
        let state = PythonResearchState::open(&directory);
        state.resetting_users.lock().unwrap().insert("alice".into());

        let error = state
            .start_attempt(AttemptStartRequest {
                user_id: "alice".into(),
                project_id: "py-factor-test".into(),
                revision_sha256: "a".repeat(64),
                environment_sha256: "b".repeat(64),
                resource_policy: None,
                seed: None,
            })
            .unwrap_err();
        assert_eq!(error.0, "research-reset-in-progress");
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn persisted_python_attempt_evidence_rejects_mismatches_in_control_plane() {
        let directory = std::env::temp_dir().join(format!(
            "adaq-python-attempt-validation-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let attempt_store = AttemptStore::open(directory.join("attempts.json")).unwrap();
        let revision_sha256 = "a".repeat(64);
        let environment_sha256 = "b".repeat(64);
        let result_sha256 = "c".repeat(64);
        let attempt = attempt_store
            .enqueue(
                "alice",
                "project",
                revision_sha256.clone(),
                environment_sha256.clone(),
                HostResourcePolicy::m12_default(),
            )
            .unwrap();
        attempt_store
            .transition(&attempt.attempt_id, AttemptTransition::Begin)
            .unwrap();
        attempt_store
            .transition(
                &attempt.attempt_id,
                AttemptTransition::Complete {
                    result_sha256: result_sha256.clone(),
                },
            )
            .unwrap();
        let evidence = PythonHostAttemptEvidence {
            attempt_id: attempt.attempt_id.clone(),
            owner_user_id: "alice".into(),
            status: "completed".into(),
            project_revision_sha256: revision_sha256.clone(),
            environment_sha256: environment_sha256.clone(),
            result_sha256: result_sha256.clone(),
        };
        assert!(
            validate_persisted_python_attempt(
                &attempt_store,
                "alice",
                "project",
                &revision_sha256,
                &environment_sha256,
                &evidence,
            )
            .is_ok()
        );

        for (name, invalid) in [
            (
                "owner",
                PythonHostAttemptEvidence {
                    owner_user_id: "bob".into(),
                    ..evidence.clone()
                },
            ),
            ("project", evidence.clone()),
            (
                "revision",
                PythonHostAttemptEvidence {
                    project_revision_sha256: "d".repeat(64),
                    ..evidence.clone()
                },
            ),
            (
                "environment",
                PythonHostAttemptEvidence {
                    environment_sha256: "e".repeat(64),
                    ..evidence.clone()
                },
            ),
            (
                "result",
                PythonHostAttemptEvidence {
                    result_sha256: "f".repeat(64),
                    ..evidence.clone()
                },
            ),
            (
                "missing",
                PythonHostAttemptEvidence {
                    attempt_id: "1".repeat(64),
                    ..evidence.clone()
                },
            ),
        ] {
            let project_id = if name == "project" {
                "other"
            } else {
                "project"
            };
            assert!(
                validate_persisted_python_attempt(
                    &attempt_store,
                    "alice",
                    project_id,
                    &revision_sha256,
                    &environment_sha256,
                    &invalid,
                )
                .is_err(),
                "{name} mismatch must be rejected"
            );
        }

        let pending = attempt_store
            .enqueue_with_execution(
                "alice",
                "project",
                revision_sha256.clone(),
                environment_sha256.clone(),
                HostResourcePolicy::m12_default(),
                AttemptExecution {
                    entry_point: "pending".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        let pending_evidence = PythonHostAttemptEvidence {
            attempt_id: pending.attempt_id,
            owner_user_id: "alice".into(),
            status: "completed".into(),
            project_revision_sha256: revision_sha256,
            environment_sha256,
            result_sha256,
        };
        assert!(
            validate_persisted_python_attempt(
                &attempt_store,
                "alice",
                "project",
                &pending_evidence.project_revision_sha256,
                &pending_evidence.environment_sha256,
                &pending_evidence,
            )
            .is_err()
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn python_host_evidence_validation_requires_persisted_attempts() {
        let directory = std::env::temp_dir().join(format!(
            "adaq-python-host-evidence-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let attempt_store = AttemptStore::open(directory.join("attempts.json")).unwrap();
        let revision_sha256 = "a".repeat(64);
        let environment_sha256 = "b".repeat(64);
        let binding = PythonFactorBinding {
            project_id: "project".into(),
            project_revision_sha256: revision_sha256.clone(),
            environment_sha256: environment_sha256.clone(),
            input_bindings: BTreeMap::new(),
            snapshot_id: String::new(),
            snapshot_bindings: BTreeMap::new(),
            point_in_time_universe_id: String::new(),
            feature_evidence_sha256: String::new(),
            feature_dataset_bindings: BTreeMap::new(),
            normalized_parameters: BTreeMap::new(),
            engine_identity: String::new(),
            repeatability_report_sha256: "c".repeat(64),
            repeatability_verified: false,
            repeatability_report: BTreeMap::new(),
            sdk_artifact_sha256: String::new(),
            entry_point: String::new(),
            mode: PythonFactorMode::ImperativePython,
            feature_plan_hash: String::new(),
            operator_catalog_version: String::new(),
            resource_policy: PythonFactorResourcePolicy::default(),
            seed: 0,
        };
        let evidence = PythonHostEvidence {
            project_revision_sha256: revision_sha256.clone(),
            environment_sha256: environment_sha256.clone(),
            repeatability_report_sha256: binding.repeatability_report_sha256.clone(),
            attempts: vec![PythonHostAttemptEvidence {
                attempt_id: "d".repeat(64),
                owner_user_id: "alice".into(),
                status: "completed".into(),
                project_revision_sha256: revision_sha256,
                environment_sha256,
                result_sha256: "e".repeat(64),
            }],
        };
        let error = validate_python_host_evidence(&attempt_store, "alice", &binding, &evidence)
            .unwrap_err();
        assert!(error.0.contains("not persisted"));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn decimal_parameters_are_normalized_before_runner_start() {
        assert_eq!(normalize_decimal(" -001.2300 ").unwrap(), "-1.23");
        assert_eq!(normalize_decimal("+000").unwrap(), "0");
        assert!(normalize_decimal("1e-3").is_err());
    }

    #[test]
    fn portable_factor_source_rejects_non_canonical_constructs() {
        let example_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../examples/python/py-factor-cross-sectional-momentum");
        let manifest = inspect_project(&example_root).manifest.unwrap();
        validate_portable_factor_source(&example_root, &manifest).unwrap();
        let root =
            std::env::temp_dir().join(format!("adaq-portable-source-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("src/project.py"),
            "from adaq import create_factor_definition\n\nvalue = lambda: None\n",
        )
        .unwrap();
        assert!(validate_portable_factor_source(&root, &manifest).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn model_source_rejects_unregistered_revisions() {
        let example_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../examples/python/py-model-qlib-ridge-return");
        let manifest = inspect_project(&example_root).manifest.unwrap();
        validate_model_source(&example_root, &manifest).unwrap();
        let root = std::env::temp_dir().join(format!("adaq-model-source-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("src/project.py"),
            "def create_project():\n    return None\n",
        )
        .unwrap();
        assert!(validate_model_source(&root, &manifest).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn factor_demo_retains_three_host_trials_and_exact_replay() {
        let result = demo_factor_run_with_outputs(None, None, true).unwrap();
        assert_eq!(result.lookbacks, vec![5, 20, 60]);
        assert!(result.synthetic);
        assert!(result.repeatability.values().all(|report| report.exact));
        assert!(result.selection_required);
        assert!(result.promotion_required);
    }

    #[test]
    fn tutorial_golden_contracts_cover_fixture_windows_and_model_boundaries() {
        let fixture = SyntheticTutorialFixture::m12().unwrap();
        fixture.validate().unwrap();
        assert_eq!(
            fixture.manifest.instrument_sha256,
            "a6963ebf7e0481749a1db2db22ef2f23bc5fee6d39d5afe258ca27c3c17fdaca"
        );
        assert_eq!(
            fixture.manifest.calendar_sha256,
            "2e423b9b46a4af56729da0fee4298ed47cdaee70b6e0bc4e4e8f5fb03cd978a9"
        );
        assert_eq!(
            fixture.manifest.bars_sha256,
            "fd4dc3bcccb554ad29ca08e89c35c220dafcb546db4df436009612f795a2bb4e"
        );
        assert_eq!(
            fixture.manifest.content_sha256,
            "6d44423e009d2251d442f388f1621242fc4dac1e0eb5d9b774fc62ecd135d848"
        );

        let windows = TutorialWindows::m12();
        windows.validate().unwrap();
        assert_eq!(
            future_close_return_state(&[1.0; 180], windows.train_end, windows.train_end),
            adaq_python_research::model::TargetValue::Unavailable(
                adaq_python_research::model::TargetUnavailableReason::WindowBoundary
            )
        );
        assert_eq!(
            future_close_return_state(&[1.0; 180], 95, windows.train_end),
            adaq_python_research::model::TargetValue::Available(0.0)
        );

        let factor = demo_factor_run_with_outputs(None, None, true).unwrap();
        assert_eq!(factor.lookbacks, vec![5, 20, 60]);
        assert_eq!(factor.available_rows.len(), 3);
        assert!(factor.repeatability.values().all(|report| report.exact));
        assert!(factor.selection_required && factor.promotion_required);

        let model = demo_model_run_with_evidence(
            1.0,
            sha256(b"py-model-qlib-ridge-return@1"),
            sha256(b"adaq-python-environment@1"),
            sha256(b"python-tutorial-a-share@1:momentum-score:20"),
            ModelInputEvidence {
                decision_hash: sha256(b"factor-decision"),
                promotion_protocol_hash: sha256(b"promotion-protocol"),
                factor_dataset_id: sha256(b"factor-dataset"),
                feature_dataset_id: sha256(b"feature-dataset"),
                feature_plan_hash: sha256(b"feature-plan"),
                snapshot_id: sha256(b"snapshot"),
                universe_id: sha256(b"universe"),
                lookback: 20,
            },
            HostResourcePolicy::m12_default(),
            None,
            None,
        )
        .unwrap();
        assert_eq!(model.view.windows, windows);
        assert_eq!(model.view.train_rows, 900);
        assert_eq!(model.view.selection_rows, 360);
        assert_eq!(model.view.final_rows, 420);
        assert!(model.view.test_labels_withheld);
        assert_eq!(
            model.view.repeatability_tolerance,
            RIDGE_REPEATABILITY_TOLERANCE
        );
        assert!(model.artifact.validate().is_ok());
        assert_eq!(model.final_labels.len(), 360);
    }

    #[test]
    fn model_demo_reloads_a_data_only_artifact_contract() {
        let result = demo_model_run_with_evidence(
            1.0,
            sha256(b"py-model-qlib-ridge-return@1"),
            sha256(b"adaq-python-environment@1"),
            sha256(b"python-tutorial-a-share@1:momentum-score:20"),
            ModelInputEvidence {
                decision_hash: sha256(b"factor-decision"),
                promotion_protocol_hash: sha256(b"promotion-protocol"),
                factor_dataset_id: sha256(b"factor-dataset"),
                feature_dataset_id: sha256(b"feature-dataset"),
                feature_plan_hash: sha256(b"feature-plan"),
                snapshot_id: sha256(b"snapshot"),
                universe_id: sha256(b"universe"),
                lookback: 20,
            },
            HostResourcePolicy::m12_default(),
            None,
            None,
        )
        .unwrap();
        assert_eq!(result.view.alpha, 1.0);
        assert_eq!(result.view.train_rows, 900);
        assert_eq!(result.view.selection_rows, 360);
        assert!(result.view.selection_metric.is_some_and(f64::is_finite));
        assert_eq!(result.view.final_rows, 420);
        assert!(result.view.test_labels_withheld);
        assert_eq!(result.final_labels.len(), 360);
        assert_eq!(
            result
                .forecasts
                .iter()
                .filter(|forecast| forecast.value.is_none())
                .count(),
            60
        );
        assert!(
            result
                .forecasts
                .iter()
                .filter_map(|forecast| forecast.unavailable_reason.as_deref())
                .all(|reason| reason == "target-window-boundary")
        );
    }

    #[test]
    fn model_lab_store_persists_schema_and_rejects_incompatible_versions() {
        let path =
            std::env::temp_dir().join(format!("adaq-model-schema-{}.json", uuid::Uuid::new_v4()));
        let store = ModelLabStore::open(&path).unwrap();
        let database = store.database.lock().unwrap().clone();
        store.persist(&database).unwrap();
        store.persist(&database).unwrap();

        let mut document: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(document["schemaVersion"], MODEL_LAB_SCHEMA_VERSION);
        document["schemaVersion"] = serde_json::json!(MODEL_LAB_SCHEMA_VERSION + 1);
        std::fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();

        let error = ModelLabStore::open(&path).err().unwrap();
        assert_eq!(
            error.to_string(),
            "model-lab-store-schema-incompatible:reset-required"
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn model_evidence_store_publishes_artifact_transformation_and_forecast_atomically() {
        let path =
            std::env::temp_dir().join(format!("adaq-model-evidence-{}.json", uuid::Uuid::new_v4()));
        let store = ModelLabStore::open(&path).unwrap();
        let mut demo = demo_model_run_with_evidence(
            1.0,
            sha256(b"revision"),
            sha256(b"environment"),
            sha256(b"input"),
            ModelInputEvidence {
                decision_hash: sha256(b"decision"),
                promotion_protocol_hash: sha256(b"protocol"),
                factor_dataset_id: sha256(b"factor"),
                feature_dataset_id: sha256(b"feature"),
                feature_plan_hash: sha256(b"plan"),
                snapshot_id: sha256(b"snapshot"),
                universe_id: sha256(b"universe"),
                lookback: 20,
            },
            HostResourcePolicy::m12_default(),
            None,
            None,
        )
        .unwrap();
        demo.view.attempt_id = "model-store-attempt".into();
        store.save_demo_run("user-a", &demo, false).unwrap();
        assert!(store.run("user-b", "model-store-attempt").is_err());
        assert!(
            !store
                .database
                .lock()
                .unwrap()
                .forecast_datasets
                .contains_key(&model_key("user-a", &demo.view.forecast_sha256))
        );
        store.save_demo_run("user-a", &demo, true).unwrap();
        let reopened = ModelLabStore::open(&path).unwrap();
        let run = reopened.run("user-a", "model-store-attempt").unwrap();
        assert_eq!(run.artifact_sha256, demo.artifact.artifact_sha256);
        assert_eq!(run.forecast_sha256, demo.view.forecast_sha256);
        let database = reopened.database.lock().unwrap();
        assert!(
            database
                .artifacts
                .contains_key(&model_key("user-a", &demo.artifact.artifact_sha256))
        );
        assert!(database.transformations.contains_key(&model_key(
            "user-a",
            &demo.transformation.transformation_sha256
        )));
        assert!(
            database
                .forecast_datasets
                .contains_key(&model_key("user-a", &demo.view.forecast_sha256))
        );
        let forecast_dataset = database
            .forecast_datasets
            .get(&model_key("user-a", &demo.view.forecast_sha256))
            .unwrap();
        assert_eq!(forecast_dataset.producer_id, MODEL_PROJECT_ID);
        assert_eq!(forecast_dataset.signal_id, "forecast");
        assert_eq!(forecast_dataset.target_id, "future-close-return");
        assert_eq!(forecast_dataset.horizon_bars, 5);
        assert_eq!(
            forecast_dataset.forecast_contract,
            adaq_python_research::model::FORECAST_CONTRACT
        );
        drop(database);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn model_trial_candidates_bind_reloaded_artifacts_and_publish_no_trial_dataset() {
        let path = std::env::temp_dir().join(format!(
            "adaq-model-candidates-{}.json",
            uuid::Uuid::new_v4()
        ));
        let mut store = ModelLabStore::open(&path).unwrap();
        let make_demo = |alpha: f64, attempt_id: &str| {
            let mut demo = demo_model_run_with_evidence(
                alpha,
                sha256(b"revision"),
                sha256(b"environment"),
                sha256(b"input"),
                ModelInputEvidence {
                    decision_hash: sha256(b"decision"),
                    promotion_protocol_hash: sha256(b"protocol"),
                    factor_dataset_id: sha256(b"factor"),
                    feature_dataset_id: sha256(b"feature"),
                    feature_plan_hash: sha256(b"plan"),
                    snapshot_id: sha256(b"snapshot"),
                    universe_id: sha256(b"universe"),
                    lookback: 20,
                },
                HostResourcePolicy::m12_default(),
                None,
                None,
            )
            .unwrap();
            demo.view.attempt_id = attempt_id.into();
            demo.view.repeatability_verified = true;
            demo.view.repeatability_state = RepeatabilityState::Verified;
            demo
        };
        let demos = vec![
            make_demo(0.1, "model-candidate-attempt-0.1"),
            make_demo(1.0, "model-candidate-attempt-1.0"),
            make_demo(10.0, "model-candidate-attempt-10.0"),
        ];
        for demo in &demos {
            store.save_demo_run("user-a", demo, false).unwrap();
        }
        assert!(store.database.lock().unwrap().forecast_datasets.is_empty());

        let experiment = ModelExperiment::ridge_with_binding(
            demos[0].view.project_revision_sha256.clone(),
            demos[0].view.environment_sha256.clone(),
            demos[0].view.input_evidence_sha256.clone(),
            demos[0].view.seed,
            demos[0].view.binding_sha256.clone(),
        )
        .unwrap();
        let experiment_id = experiment.experiment_id.clone();
        store.register("user-a", experiment.clone()).unwrap();

        let mismatch = store
            .complete_trial_with_candidate(
                "user-a",
                &experiment_id,
                &experiment.trials[0].trial_id,
                demos[0].view.attempt_id.clone(),
                demos[0].view.selection_metric.unwrap(),
                demos[1].view.artifact_sha256.clone(),
            )
            .unwrap_err();
        assert_eq!(
            mismatch.to_string(),
            "model-trial-candidate-binding-invalid"
        );
        assert!(
            store.experiment("user-a", &experiment_id).unwrap().trials[0]
                .candidate_artifact_sha256
                .is_none()
        );

        let broken_path = path.with_extension("persist-dir");
        let broken_temporary = broken_path.with_extension("json.tmp");
        std::fs::create_dir(&broken_path).unwrap();
        store.path = broken_path.clone();
        assert!(
            store
                .complete_trial_with_candidate(
                    "user-a",
                    &experiment_id,
                    &experiment.trials[0].trial_id,
                    demos[0].view.attempt_id.clone(),
                    demos[0].view.selection_metric.unwrap(),
                    demos[0].view.artifact_sha256.clone(),
                )
                .is_err()
        );
        assert!(
            store.experiment("user-a", &experiment_id).unwrap().trials[0]
                .candidate_artifact_sha256
                .is_none()
        );
        store.path = path.clone();
        if broken_temporary.is_file() {
            std::fs::remove_file(&broken_temporary).unwrap();
        }
        std::fs::remove_dir(&broken_path).unwrap();

        for (trial, demo) in experiment.trials.iter().zip(&demos) {
            store
                .complete_trial_with_candidate(
                    "user-a",
                    &experiment_id,
                    &trial.trial_id,
                    demo.view.attempt_id.clone(),
                    demo.view.selection_metric.unwrap(),
                    demo.view.artifact_sha256.clone(),
                )
                .unwrap();
        }

        let reopened = ModelLabStore::open(&path).unwrap();
        let persisted = reopened.experiment("user-a", &experiment_id).unwrap();
        for (trial, demo) in persisted.trials.iter().zip(&demos) {
            assert_eq!(
                trial.successful_attempt_id.as_deref(),
                Some(demo.view.attempt_id.as_str())
            );
            assert_eq!(
                trial.candidate_artifact_sha256.as_deref(),
                Some(demo.view.artifact_sha256.as_str())
            );
            let artifact = LinearModelArtifact::reload(
                &reopened
                    .artifact("user-a", &demo.view.artifact_sha256)
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(artifact.artifact_sha256, demo.view.artifact_sha256);
        }
        let decision = reopened
            .select("user-a", &experiment_id, &persisted.trials[1].trial_id)
            .unwrap();
        assert_eq!(
            decision.candidate_artifact_sha256,
            demos[1].view.artifact_sha256
        );
        assert_eq!(
            reopened
                .select("user-a", &experiment_id, &persisted.trials[2].trial_id)
                .unwrap_err()
                .to_string(),
            "model-selection-decision-already-recorded"
        );
        let projection = reopened.projection("user-a", None).unwrap();
        assert_eq!(projection.decision, Some(decision.clone()));
        assert!(projection.report.is_none());
        assert_eq!(
            projection
                .final_evaluation
                .as_ref()
                .map(|state| state.status),
            None
        );
        assert!(
            reopened
                .begin_final("user-a", &decision.decision_id)
                .unwrap()
                .is_none()
        );
        reopened
            .bind_final_attempt("user-a", &decision.decision_id, "final-attempt")
            .unwrap();
        reopened
            .stage_final_dataset(
                "user-a",
                &decision.decision_id,
                &sha256(b"forecast-dataset"),
            )
            .unwrap();
        let reopened = ModelLabStore::open(&path).unwrap();
        let recovered = reopened.projection("user-a", None).unwrap();
        assert_eq!(
            recovered
                .final_evaluation
                .as_ref()
                .map(|state| state.status),
            Some(ModelFinalEvaluationStatus::Interrupted)
        );
        assert_eq!(
            recovered
                .final_evaluation
                .as_ref()
                .and_then(|state| state.attempt_id.as_deref()),
            Some("final-attempt")
        );
        let forecasts = vec![adaq_python_research::model::ForecastRow {
            datetime: 1,
            instrument: "AAA".into(),
            value: Some(2.0),
            unavailable_reason: None,
        }];
        let labels = vec![(1, "AAA".into(), 3.0)];
        let report = FinalEvaluationLedger::default()
            .run_with_evidence(
                &decision,
                &forecasts,
                &labels,
                decision.candidate_artifact_sha256.clone(),
                sha256(b"forecast-dataset"),
                EvidenceState::OutOfSample,
            )
            .unwrap();
        reopened.save_final("user-a", report.clone()).unwrap();
        assert_eq!(
            reopened.save_final("user-a", report.clone()).unwrap(),
            report
        );
        let completed = reopened.projection("user-a", None).unwrap();
        assert!(completed.report.is_some());
        assert_eq!(
            completed
                .final_evaluation
                .as_ref()
                .map(|state| state.status),
            Some(ModelFinalEvaluationStatus::Completed)
        );
        assert_eq!(
            reopened
                .select("user-a", &experiment_id, &persisted.trials[2].trial_id)
                .unwrap_err()
                .to_string(),
            "model-selection-after-final-evaluation"
        );
        assert!(
            reopened
                .database
                .lock()
                .unwrap()
                .forecast_datasets
                .is_empty()
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn model_process_replay_rejects_divergent_contract_payloads() {
        let execution = |payload| RunnerExecution {
            conformance: Some(ConformanceResult {
                attempt_id: "attempt".into(),
                project_id: MODEL_PROJECT_ID.into(),
                project_kind: "model".into(),
                entry_point: "project:create_project".into(),
                payload: Some(payload),
            }),
            staged_result: None,
            staged_artifact: None,
            log: Vec::new(),
            log_truncated: false,
        };
        let first = execution(serde_json::json!({"target": "return"}));
        let replay = execution(serde_json::json!({"target": "return"}));
        assert_eq!(
            validate_model_process_replay(&first, &replay).unwrap(),
            "attempt"
        );
        let divergent = execution(serde_json::json!({"target": "other"}));
        assert_eq!(
            validate_model_process_replay(&first, &divergent)
                .unwrap_err()
                .0,
            "model-process-replay-divergent"
        );
    }

    #[test]
    fn factor_vertical_publishes_dataset_reports_and_promotion_evidence() {
        let directory =
            std::env::temp_dir().join(format!("adaq-python-factor-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let local = crate::local_research::LocalResearchState::open(&directory).unwrap();
        let python = Arc::new(PythonResearchState::open(&directory));
        python.attach_queue(local.features.queue());
        local.features.attach_python(python.clone()).unwrap();
        let request = FactorRunRequest {
            user_id: "alice".into(),
            project_id: "py-factor-cross-sectional-momentum".into(),
            project_revision_sha256: sha256(b"python-factor-revision"),
            environment_sha256: sha256(b"python-factor-environment"),
        };
        let fixture = SyntheticTutorialFixture::m12().unwrap();
        let feature_evidence = prepare_factor_feature_evidence(&local, &request, &fixture).unwrap();
        let python_outputs = expand_momentum_grid()
            .into_iter()
            .map(|lookback| {
                (
                    lookback,
                    materialize_momentum(&fixture.momentum_rows(), &fixture.instruments, lookback)
                        .unwrap(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let mut process_hashes = BTreeMap::new();
        let mut host_attempts = Vec::new();
        for lookback in expand_momentum_grid() {
            let complete_attempt = |label: &str| {
                let attempt = python
                    .attempt_store
                    .enqueue_with_execution(
                        request.user_id.clone(),
                        request.project_id.clone(),
                        request.project_revision_sha256.clone(),
                        request.environment_sha256.clone(),
                        HostResourcePolicy::m12_default(),
                        AttemptExecution::default(),
                    )
                    .unwrap();
                python
                    .attempt_store
                    .transition(&attempt.attempt_id, AttemptTransition::Begin)
                    .unwrap();
                let result_sha256 = sha256(label.as_bytes());
                python
                    .attempt_store
                    .transition(
                        &attempt.attempt_id,
                        AttemptTransition::Complete {
                            result_sha256: result_sha256.clone(),
                        },
                    )
                    .unwrap();
                (attempt, result_sha256)
            };
            let (first_attempt, first_result_sha256) =
                complete_attempt(&format!("vertical-{lookback}-first"));
            let (replay_attempt, replay_result_sha256) =
                complete_attempt(&format!("vertical-{lookback}-replay"));
            for (attempt, result_sha256) in [
                (&first_attempt, &first_result_sha256),
                (&replay_attempt, &replay_result_sha256),
            ] {
                host_attempts.push(PythonHostAttemptEvidence {
                    attempt_id: attempt.attempt_id.clone(),
                    owner_user_id: request.user_id.clone(),
                    status: "completed".into(),
                    project_revision_sha256: request.project_revision_sha256.clone(),
                    environment_sha256: request.environment_sha256.clone(),
                    result_sha256: result_sha256.clone(),
                });
            }
            process_hashes.insert(
                lookback,
                (
                    RunnerProcessEvidence {
                        attempt_id: first_attempt.attempt_id,
                        process_sha256: sha256(
                            format!("vertical-process-first-{lookback}").as_bytes(),
                        ),
                        contract_sha256: sha256(b"vertical-contract"),
                        input_sha256: sha256(b"vertical-input"),
                    },
                    RunnerProcessEvidence {
                        attempt_id: replay_attempt.attempt_id,
                        process_sha256: sha256(
                            format!("vertical-process-replay-{lookback}").as_bytes(),
                        ),
                        contract_sha256: sha256(b"vertical-contract"),
                        input_sha256: sha256(b"vertical-input"),
                    },
                ),
            );
        }
        for attempt in &host_attempts {
            validate_persisted_python_attempt(
                python.attempt_store.as_ref(),
                &request.user_id,
                &request.project_id,
                &request.project_revision_sha256,
                &request.environment_sha256,
                attempt,
            )
            .unwrap();
        }
        let repeatability_report = factor_repeatability_reports(
            &python_outputs,
            &python_outputs,
            &process_hashes,
            true,
            PythonFactorMode::PortableDefinition,
        )
        .unwrap();
        let repeatability_report_sha256 =
            adaq_factor_research::content_hash(&repeatability_report).unwrap();
        let candidate = local
            .factor
            .publish_python_candidate(
                crate::factor_research::FactorCandidatePublishRequest {
                    user_id: request.user_id.clone(),
                    draft: FactorCandidateDraft {
                        candidate_id: uuid::Uuid::from_u128(0x6d120101000000000000000000000001),
                        revision: 1,
                        scope: FactorScope::CrossSectional,
                        feature_slots: vec![FactorFeatureSlot {
                            name: "close".into(),
                        }],
                        parameters: vec![FactorParameter {
                            name: "lookback".into(),
                            parameter_type: FactorParameterType::Integer,
                            default_value: "20".into(),
                            allowed_values: vec!["5".into(), "20".into(), "60".into()],
                        }],
                        outputs: vec![FactorOutput {
                            name: "momentum-score".into(),
                        }],
                        source: FactorCandidateSource::Python {
                            binding: PythonFactorBinding {
                                project_id: request.project_id.clone(),
                                project_revision_sha256: request.project_revision_sha256.clone(),
                                environment_sha256: request.environment_sha256.clone(),
                                input_bindings: BTreeMap::from([(
                                    "close".into(),
                                    "host:market-close".into(),
                                )]),
                                snapshot_id: feature_evidence.snapshot_id.clone(),
                                snapshot_bindings: feature_evidence.snapshot_bindings.clone(),
                                point_in_time_universe_id: sha256(
                                    b"python-tutorial-a-share@1:point-in-time-universe",
                                ),
                                feature_evidence_sha256: feature_evidence.evidence_sha256.clone(),
                                feature_dataset_bindings: feature_evidence.dataset_bindings.clone(),
                                normalized_parameters: BTreeMap::from([(
                                    "lookback".into(),
                                    "20".into(),
                                )]),
                                engine_identity: "adaq-python-factor@1".into(),
                                repeatability_report_sha256: repeatability_report_sha256.clone(),
                                repeatability_verified: true,
                                repeatability_report,
                                sdk_artifact_sha256: PUBLIC_SDK_ARTIFACT_SHA256.into(),
                                entry_point: "project:create_project".into(),
                                mode: PythonFactorMode::PortableDefinition,
                                feature_plan_hash: feature_evidence.plan_hash.clone(),
                                operator_catalog_version:
                                    adaq_feature_engine::FEATURE_OPERATOR_CATALOG_VERSION.into(),
                                resource_policy: python_factor_resource_policy(
                                    &HostResourcePolicy::m12_default(),
                                ),
                                seed: 7,
                            },
                        },
                    },
                    presentation: FactorPresentationMetadata {
                        name: "Python Cross-sectional Momentum".into(),
                        description: "Synthetic M12 portable Python Factor candidate".into(),
                        tags: vec!["python".into(), "momentum".into(), "synthetic".into()],
                    },
                },
                crate::factor_research::PythonHostEvidence {
                    project_revision_sha256: request.project_revision_sha256.clone(),
                    environment_sha256: request.environment_sha256.clone(),
                    repeatability_report_sha256,
                    attempts: host_attempts,
                },
            )
            .unwrap();
        let evidence = run_factor_evidence(
            &local,
            &request,
            &candidate.candidate.candidate_hash,
            &feature_evidence,
            &python_outputs,
        )
        .unwrap();
        assert_eq!(evidence.trial_ids.len(), 3);
        assert_eq!(evidence.dataset_ids.len(), 3);
        assert_eq!(evidence.report_hashes.len(), 3);
        assert_eq!(evidence.promotion_protocol.trial_id, evidence.trial_ids[1]);
        assert_eq!(evidence.promotion_protocols.len(), 3);
        assert_eq!(
            local
                .factor
                .list_decisions(crate::factor_research::FactorPageRequest {
                    user_id: request.user_id.clone(),
                    page: 1,
                    page_size: Some(10),
                    kind: None,
                })
                .unwrap()
                .total,
            0
        );
        let selection = local
            .factor
            .select_trial(crate::factor_research::FactorTrialSelectionRequest {
                user_id: request.user_id.clone(),
                candidate_hash: candidate.candidate.candidate_hash.clone(),
                family_id: evidence.family_id,
                trial_id: evidence.trial_ids[1],
                policy_hash: evidence.policy.policy_hash.clone(),
            })
            .unwrap();
        assert_eq!(selection.selected_trial_id, evidence.trial_ids[1]);
        let (stored_selection, selected_protocol) = local
            .factor
            .selected_trial(&request.user_id, &candidate.candidate.candidate_hash)
            .unwrap();
        assert_eq!(stored_selection.selection_hash, selection.selection_hash);
        let decision = FactorPromotionDecision::freeze(PromotionDecisionDraft {
            decision_id: uuid::Uuid::new_v4(),
            user_id: crate::factor_research::user_uuid(&request.user_id),
            candidate_hash: candidate.candidate.candidate_hash.clone(),
            output_name: selected_protocol.output_name.clone(),
            state: PromotionDecisionState::ResearchValidated,
            report_hashes: selected_protocol.report_hashes.clone(),
            policy_hash: selected_protocol.policy_hash.clone(),
            evidence_state: adaq_factor_research::EvaluationEvidenceState::OutOfSample,
            supersedes: None,
        })
        .unwrap();
        local
            .factor
            .save_decision(FactorDecisionSaveRequest {
                user_id: request.user_id.clone(),
                decision: decision.clone(),
                promotion_protocol: selected_protocol.clone(),
                component: Default::default(),
            })
            .unwrap();
        let model_binding = local
            .factor
            .model_input_binding(&request.user_id, &decision.decision_hash)
            .unwrap();
        assert_eq!(
            model_binding.promotion_protocol.trial_id,
            evidence.trial_ids[1]
        );
        assert_eq!(model_binding.lookback, 20);
        let model_input = ModelInputEvidence {
            decision_hash: model_binding.decision_hash.clone(),
            promotion_protocol_hash: model_binding.promotion_protocol.protocol_hash.clone(),
            factor_dataset_id: model_binding.factor_dataset_id.clone(),
            feature_dataset_id: model_binding.feature_dataset_id.clone(),
            feature_plan_hash: model_binding.feature_plan_hash.clone(),
            snapshot_id: model_binding.snapshot_id.clone(),
            universe_id: model_binding.universe_id.clone(),
            lookback: model_binding.lookback,
        };
        let model = demo_model_run_with_evidence(
            1.0,
            request.project_revision_sha256.clone(),
            request.environment_sha256.clone(),
            model_input_evidence_hash(&model_binding).unwrap(),
            model_input,
            HostResourcePolicy::m12_default(),
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            model.view.factor_dataset_id,
            model_binding.factor_dataset_id
        );
        assert_eq!(model.view.factor_lookback, 20);
        assert!(model.view.test_labels_withheld);
        assert!(model.artifact.validate().is_ok());
        assert_eq!(model.forecasts.len(), 420);
        drop(python);
        drop(local);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn portable_definition_uses_feature_engine_for_momentum_values() {
        let fixture = SyntheticTutorialFixture::m12().unwrap();
        let expected =
            materialize_momentum(&fixture.momentum_rows(), &fixture.instruments, 20).unwrap();
        let actual = evaluate_portable_momentum(&fixture, 20).unwrap();
        assert_eq!(actual.len(), expected.len());
        for (index, (actual, expected)) in actual.iter().zip(&expected).enumerate() {
            if actual != expected {
                panic!(
                    "row {index}: actual=({:?}, {:?}) expected=({:?}, {:?})",
                    actual.value,
                    actual.unavailable_reason,
                    expected.value,
                    expected.unavailable_reason
                );
            }
        }
    }
}
