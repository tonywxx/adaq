//! Tauri control-plane bindings for the source-visible Python Research boundary.
//!
//! Heavy work stays in Tauri-independent contracts or `spawn_blocking`; these
//! commands only bind those contracts to User-scoped app state and UI actions.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use adaq_factor_research::{
    CorporateActionEvidence, EconomicAssumptions, EvaluationWindow, FactorCandidateDraft,
    FactorCandidateSource, FactorDataset, FactorDatasetManifest, FactorDatasetRow,
    FactorEvaluationProtocol, FactorEvaluationProtocolDraft, FactorFeatureSlot, FactorLens,
    FactorMarketContext, FactorMarketSeries, FactorObservationValue, FactorOrientation,
    FactorOutput, FactorParameter, FactorParameterType, FactorParameterValue,
    FactorPresentationMetadata, FactorPromotionDecision, FactorScope, FactorTarget,
    GridSearchFamilyDraft, GridSearchParameter, GridSearchPlan, MetricId, MetricObservation,
    PromotionDecisionDraft, PromotionDecisionState, PromotionPolicy, PromotionProtocol,
    PromotionProtocolDraft, PythonFactorBinding, PythonFactorMode, PythonFactorResourcePolicy,
    PythonRepeatabilityReport, ResearchEngineProvenance, ResearchRegistry, ResearchTrial,
    ResearchTrialStatus,
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
        DatasetH, FittedTransformation, HostPartition, HostPartitionRow, MODEL_PROJECT_ID,
        ModelRunnerInput, PartitionName, RidgeAdapter, TARGET_HORIZON_BARS, TutorialWindows,
        forecast, future_close_return_state, validate_model_project_payload,
        validate_model_runner_payload,
    },
    runner::{
        AttemptExecution, AttemptStore, AttemptTransition, Handshake, PrivateChildEnvironment,
        ResearchAttempt, RunnerExecution, RunnerLaunchSpec, StagedArtifact, TrustStore,
        read_staged_artifact, run_process,
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
        FinalEvaluationLedger, FinalEvaluationReport, ModelExperiment, ParameterSelectionDecision,
        RIDGE_REPEATABILITY_TOLERANCE, TrialStatus, compare_repeatability,
    },
};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use tauri::{Manager, State};

use crate::factor_research::{
    FactorDatasetInput, FactorDecisionSaveRequest, FactorEvaluationStartRequest,
    FactorGridFamilyRegisterRequest, FactorPolicySaveRequest, FactorTrialUpdateRequest,
    PythonHostAttemptEvidence,
};
use crate::features::{
    FeatureAttemptRequest, FeatureMaterializationStartRequest, PythonQueueItem, PythonQueueWork,
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
    queue_notifier: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelLabDatabase {
    experiments: BTreeMap<String, ModelExperiment>,
    decisions: BTreeMap<String, ParameterSelectionDecision>,
    final_reports: BTreeMap<String, FinalEvaluationReport>,
    #[serde(default)]
    runs: BTreeMap<String, ModelRunView>,
    #[serde(default)]
    artifacts: BTreeMap<String, Vec<u8>>,
    #[serde(default)]
    transformations: BTreeMap<String, Vec<u8>>,
    #[serde(default)]
    forecast_datasets: BTreeMap<String, StoredForecastDataset>,
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
        let database = if path.is_file() {
            serde_json::from_slice(&fs::read(&path)?)
                .map_err(|error| PythonResearchError(format!("model-lab-store-invalid:{error}")))?
        } else {
            ModelLabDatabase::default()
        };
        Ok(Self {
            path,
            database: Arc::new(Mutex::new(database)),
        })
    }

    fn persist(&self, database: &ModelLabDatabase) -> Result<(), PythonResearchError> {
        let temporary = self.path.with_extension("json.tmp");
        fs::write(
            &temporary,
            serde_json::to_vec_pretty(database)
                .map_err(|error| PythonResearchError(error.to_string()))?,
        )?;
        fs::rename(temporary, &self.path)?;
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
            return Ok(existing.clone());
        }
        database.experiments.insert(key, experiment.clone());
        self.persist(&database)?;
        Ok(experiment)
    }

    fn complete_trial(
        &self,
        user_id: &str,
        experiment_id: &str,
        trial_id: &str,
        attempt_id: String,
        selection_metric: f64,
    ) -> Result<ModelExperiment, PythonResearchError> {
        let mut database = self
            .database
            .lock()
            .map_err(|_| PythonResearchError("model-lab-store-lock-poisoned".into()))?;
        let experiment = database
            .experiments
            .get_mut(&model_key(user_id, experiment_id))
            .ok_or_else(|| PythonResearchError("model-experiment-not-found".into()))?;
        experiment.complete_trial(trial_id, attempt_id, selection_metric)?;
        let result = experiment.clone();
        self.persist(&database)?;
        Ok(result)
    }

    fn replace_experiment(
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
        if !database.experiments.contains_key(&key) {
            return Err(PythonResearchError("model-experiment-not-found".into()));
        }
        database.experiments.insert(key, experiment.clone());
        self.persist(&database)?;
        Ok(experiment)
    }

    fn save_demo_run(
        &self,
        user_id: &str,
        demo: &DemoModelRun,
    ) -> Result<ModelRunView, PythonResearchError> {
        let expected_provenance = BTreeMap::from([
            ("fixture".into(), demo.view.fixture_sha256.clone()),
            ("revision".into(), demo.view.project_revision_sha256.clone()),
            ("environment".into(), demo.view.environment_sha256.clone()),
            ("input".into(), demo.view.input_evidence_sha256.clone()),
            (
                "factorDecision".into(),
                demo.view.factor_decision_hash.clone(),
            ),
            (
                "promotionProtocol".into(),
                demo.view.factor_promotion_protocol_hash.clone(),
            ),
            (
                "resourcePolicy".into(),
                resource_policy_identity(&demo.view.resource_policy)?,
            ),
            ("factorDataset".into(), demo.view.factor_dataset_id.clone()),
            (
                "featureDataset".into(),
                demo.view.feature_dataset_id.clone(),
            ),
            ("featurePlan".into(), demo.view.feature_plan_hash.clone()),
            ("snapshot".into(), demo.view.snapshot_id.clone()),
            ("universe".into(), demo.view.universe_id.clone()),
        ]);
        if demo.artifact.artifact_sha256 != demo.view.artifact_sha256
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
        next.artifacts.insert(artifact_key, artifact_bytes);
        next.transformations
            .insert(transformation_key, transformation_bytes);
        next.forecast_datasets
            .insert(forecast_key, forecast_dataset);
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
        database
            .decisions
            .insert(model_key(user_id, &decision.decision_id), decision.clone());
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

    fn save_final(
        &self,
        user_id: &str,
        report: FinalEvaluationReport,
    ) -> Result<FinalEvaluationReport, PythonResearchError> {
        report.validate()?;
        let mut database = self
            .database
            .lock()
            .map_err(|_| PythonResearchError("model-lab-store-lock-poisoned".into()))?;
        let user_prefix = format!("{user_id}:");
        if database.final_reports.iter().any(|(key, existing)| {
            key.starts_with(&user_prefix) && existing.decision_id == report.decision_id
        }) {
            return Err(PythonResearchError(
                "model-final-evaluation-already-recorded".into(),
            ));
        }
        database
            .final_reports
            .insert(model_key(user_id, &report.report_id), report.clone());
        self.persist(&database)?;
        Ok(report)
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
        self.persist(&database)
    }
}

fn model_key(user_id: &str, identity: &str) -> String {
    format!("{user_id}:{identity}")
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
    Ok(DemoModelRun {
        view: ModelRunView {
            attempt_id: String::new(),
            adapter_id: adaq_python_research::model::RIDGE_ADAPTER_ID.into(),
            alpha,
            project_revision_sha256,
            environment_sha256,
            input_evidence_sha256,
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
            windows,
            resource_policy,
            input_slots: transformation.feature_names.clone(),
            target_id: adaq_python_research::model::TARGET_ID.into(),
            target_horizon_bars: TARGET_HORIZON_BARS as u32,
            forecast_contract: adaq_python_research::model::FORECAST_CONTRACT.into(),
            artifact_schema: adaq_python_research::model::LINEAR_MODEL_ARTIFACT_SCHEMA.into(),
            numeric_representation: adaq_python_research::model::NUMERIC_REPRESENTATION.into(),
        },
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

fn factor_trial_statistics(
    report: &adaq_factor_research::FactorEvaluationReport,
) -> Result<(Option<MetricObservation>, Option<MetricObservation>), PythonResearchError> {
    let observations = report
        .metrics
        .iter()
        .filter(|metric| metric.metric == MetricId::Ic)
        .filter_map(|metric| {
            metric.observation.value().map(|value| {
                let sample_count = match &metric.observation {
                    MetricObservation::Available { sample_count, .. }
                    | MetricObservation::Unavailable { sample_count, .. } => *sample_count,
                };
                (value, sample_count)
            })
        })
        .collect::<Vec<_>>();
    if observations.is_empty() {
        return Ok((None, None));
    }
    let sample_count = observations
        .iter()
        .map(|(_, sample_count)| *sample_count)
        .sum::<u64>();
    let raw = observations.iter().map(|(value, _)| *value).sum::<f64>() / observations.len() as f64;
    let raw_statistic = MetricObservation::available(raw, sample_count)
        .map_err(|error| PythonResearchError(error.to_string()))?;
    let p_value = (sample_count > 2).then(|| {
        let z = raw.abs() * ((sample_count - 2) as f64 / (1.0 - raw * raw).max(1e-12)).sqrt();
        let tail = normal_upper_tail(z);
        MetricObservation::available((2.0 * tail).clamp(0.0, 1.0), sample_count)
            .expect("normal approximation is finite")
    });
    Ok((Some(raw_statistic), p_value))
}

fn normal_upper_tail(value: f64) -> f64 {
    let value = value.abs();
    let t = 1.0 / (1.0 + 0.2316419 * value);
    let density = (-0.5 * value * value).exp() * 0.3989422804014327;
    density
        * t
        * (0.319381530
            + t * (-0.356563782 + t * (1.781477937 + t * (-1.821255978 + t * 1.330274429))))
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
        let (raw_statistic, p_value) = factor_trial_statistics(&report.report)?;
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
            queue_notifier: Mutex::new(None),
            completed_results: Arc::new(Mutex::new(BTreeMap::new())),
            runtime_cancellations: Mutex::new(BTreeMap::new()),
            runtime_progress: Arc::new(Mutex::new(BTreeMap::new())),
            shutdown: AtomicBool::new(false),
        }
    }

    pub(crate) fn attach_queue(&self, notifier: Arc<dyn Fn() + Send + Sync>) {
        if let Ok(mut queue_notifier) = self.queue_notifier.lock() {
            *queue_notifier = Some(notifier);
        }
    }

    fn notify_queue(&self) {
        if let Ok(notifier) = self.queue_notifier.lock()
            && let Some(notifier) = notifier.as_ref()
        {
            notifier();
        }
    }

    fn execute_attempt(&self, item: PythonQueueItem) {
        let Ok(mut attempt) = self.attempt_store.get(&item.attempt_id) else {
            return;
        };
        if attempt.status == adaq_python_research::runner::AttemptStatus::Pending {
            let Ok(updated) = self
                .attempt_store
                .transition(&item.attempt_id, AttemptTransition::Begin)
            else {
                return;
            };
            attempt = updated;
        }
        if attempt.status != adaq_python_research::runner::AttemptStatus::Running {
            return;
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
        let attempt = self.attempt_store.enqueue_with_execution(
            user_id,
            project_id,
            revision_sha256,
            context.environment.environment_sha256.clone(),
            effective_resource_policy(&context.manifest, None)?,
            build_attempt_execution(
                &context.revision,
                &context.manifest,
                &context.lock,
                seed,
                input,
                parameter_overrides,
            )?,
        )?;
        self.notify_queue();
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

impl PythonQueueWork for PythonResearchState {
    fn next_runnable(&self) -> Result<Option<PythonQueueItem>, String> {
        self.attempt_store
            .next_runnable()
            .map_err(|error| error.to_string())
            .map(|attempt| {
                attempt.map(|attempt| PythonQueueItem {
                    attempt_id: attempt.attempt_id,
                    created_at_ms: attempt.created_at_ms,
                })
            })
    }

    fn execute(&self, item: PythonQueueItem) {
        self.execute_attempt(item);
    }

    fn shutdown(&self) {
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
    pub project_revision_sha256: String,
    pub environment_sha256: String,
    pub input_evidence_sha256: String,
    pub factor_decision_hash: String,
    pub seed: u64,
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
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<Vec<WorkingCopySummary>, String> {
    let store = state.store.clone();
    tauri::async_runtime::spawn_blocking(move || store.list(&user_id).map_err(map_error))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn project_create(
    request: ProjectCreateRequest,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<WorkingCopySummary, String> {
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
    request: ProjectImportRequest,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<WorkingCopySummary, String> {
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
    request: ProjectRequest,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<ValidationReport, String> {
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
    request: ProjectFreezeRequest,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<ProjectRevision, String> {
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
    request: ProjectFreezeRequest,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<ProjectExport, String> {
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
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<PythonResearchResetReport, String> {
    let store = state.store.clone();
    let attempt_store = state.attempt_store.clone();
    let trust_store = state.trust_store.clone();
    let model_lab_store = state.model_lab_store.clone();
    let completed_results = state.completed_results.clone();
    let result_root = state.root.join("attempt-results");
    let notifier = state
        .queue_notifier
        .lock()
        .ok()
        .and_then(|value| value.clone());
    tauri::async_runtime::spawn_blocking(move || {
        attempt_store.cancel_user(&user_id).map_err(map_error)?;
        if let Some(notifier) = notifier.as_ref() {
            notifier();
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while attempt_store
            .has_active_for_user(&user_id)
            .map_err(map_error)?
        {
            if std::time::Instant::now() >= deadline {
                return Err("research-reset-runner-did-not-stop".into());
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let attempt_ids = attempt_store
            .list(&user_id)
            .map_err(map_error)?
            .into_iter()
            .map(|attempt| attempt.attempt_id)
            .collect::<BTreeSet<_>>();
        for attempt_id in &attempt_ids {
            let artifact = result_root.join(format!("{attempt_id}.artifact"));
            if artifact.is_file() {
                fs::remove_file(artifact).map_err(|error| error.to_string())?;
            }
        }
        let report = store
            .reset_python_research_evidence(&user_id)
            .map_err(map_error)?;
        attempt_store.reset_user(&user_id).map_err(map_error)?;
        trust_store.reset_user(&user_id).map_err(map_error)?;
        model_lab_store.reset_user(&user_id).map_err(map_error)?;
        if let Ok(mut results) = completed_results.lock() {
            results.retain(|attempt_id, _| !attempt_ids.contains(attempt_id));
        }
        Ok(report)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn trust_revision(
    request: TrustRequest,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<adaq_python_research::TrustDecision, String> {
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
    request: AttemptPreviewRequest,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<AttemptPreview, String> {
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
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<Vec<ResearchAttempt>, String> {
    let attempt_store = state.attempt_store.clone();
    tauri::async_runtime::spawn_blocking(move || attempt_store.list(&user_id).map_err(map_error))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn attempt_start(
    request: AttemptStartRequest,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<ResearchAttempt, String> {
    let project_store = state.store.clone();
    let trust_store = state.trust_store.clone();
    let attempt_store = state.attempt_store.clone();
    let environment_store = state.environment_store.clone();
    let runtime_store = state.runtime_store.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let context = load_attempt_context(
            &project_store,
            &environment_store,
            &runtime_store,
            &request.user_id,
            &request.project_id,
            &request.revision_sha256,
            Some(&request.environment_sha256),
        )
        .map_err(map_error)?;
        if trust_store
            .get(
                &request.user_id,
                &request.project_id,
                &request.revision_sha256,
            )
            .map_err(map_error)?
            .is_none()
        {
            return Err("research-revision-not-trusted".into());
        }
        let policy = effective_resource_policy(&context.manifest, request.resource_policy)
            .map_err(map_error)?;
        let execution = build_attempt_execution(
            &context.revision,
            &context.manifest,
            &context.lock,
            request.seed.unwrap_or(0),
            None,
            None,
        )
        .map_err(map_error)?;
        attempt_store
            .enqueue_with_execution(
                request.user_id,
                request.project_id,
                request.revision_sha256,
                context.environment.environment_sha256,
                policy,
                execution,
            )
            .map_err(map_error)
    })
    .await
    .map_err(|error| error.to_string())?
    .inspect(|_| state.notify_queue())
}

#[tauri::command]
pub async fn attempt_cancel(
    request: AttemptRequest,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<ResearchAttempt, String> {
    transition_attempt(
        state,
        request.user_id,
        request.attempt_id,
        AttemptTransition::Cancel,
    )
    .await
}

#[tauri::command]
pub async fn attempt_retry(
    request: AttemptRequest,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<ResearchAttempt, String> {
    let attempt_store = state.attempt_store.clone();
    let user_id = request.user_id;
    let attempt_id = request.attempt_id;
    tauri::async_runtime::spawn_blocking(move || {
        let attempt = attempt_store.get(&attempt_id).map_err(map_error)?;
        if attempt.user_id != user_id {
            return Err("research-attempt-not-found".into());
        }
        attempt_store.retry(&attempt_id).map_err(map_error)
    })
    .await
    .map_err(|error| error.to_string())?
    .inspect(|_| state.notify_queue())
}

async fn transition_attempt(
    state: State<'_, Arc<PythonResearchState>>,
    user_id: String,
    attempt_id: String,
    transition: AttemptTransition,
) -> Result<ResearchAttempt, String> {
    let attempt_store = state.attempt_store.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let attempt = attempt_store.get(&attempt_id).map_err(map_error)?;
        if attempt.user_id != user_id {
            return Err("research-attempt-not-found".into());
        }
        attempt_store
            .transition(&attempt_id, transition)
            .map_err(map_error)
    })
    .await
    .map_err(|error| error.to_string())?
    .inspect(|_| state.notify_queue())
}

#[tauri::command]
pub async fn model_demo_run(
    request: ModelRunRequest,
    state: State<'_, Arc<PythonResearchState>>,
    app: tauri::AppHandle,
) -> Result<ModelRunView, String> {
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
        let execution = research_state
            .run_trusted_project_with_execution(
                &request.user_id,
                &request.project_id,
                &project_revision_sha256,
                &environment_sha256,
                7,
                Some(runner_input.clone()),
                Some(&parameters),
            )
            .map_err(map_error)?;
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
        if first_artifact.to_bytes().map_err(map_error)?
            != replay_artifact.to_bytes().map_err(map_error)?
        {
            return Err("model-artifact-replay-divergent".into());
        }
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
        compare_repeatability(
            &run.artifact.coefficients,
            &replay.artifact.coefficients,
            &run.forecasts,
            &replay.forecasts,
        )
        .map_err(map_error)?;
        run.view.repeatability_verified = true;
        run.view.attempt_id = attempt_id;
        let view = research_state
            .model_lab_store
            .save_demo_run(&request.user_id, &run)
            .map_err(map_error)?;
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
        Ok(view)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn python_factor_demo(
    request: FactorRunRequest,
    state: State<'_, Arc<PythonResearchState>>,
    app: tauri::AppHandle,
) -> Result<FactorRunView, String> {
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
                crate::factor_research::PythonHostEvidence {
                    project_revision_sha256: request.project_revision_sha256.clone(),
                    environment_sha256: request.environment_sha256.clone(),
                    repeatability_report_sha256: repeatability_report_sha256.clone(),
                    attempts: host_attempts,
                },
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
    request: FactorTrialSelectionRequest,
    state: State<'_, Arc<PythonResearchState>>,
    app: tauri::AppHandle,
) -> Result<PythonFactorSelectionView, String> {
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
    request: FactorPromotionRequest,
    state: State<'_, Arc<PythonResearchState>>,
    app: tauri::AppHandle,
) -> Result<PythonFactorPromotionView, String> {
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
    request: ModelExperimentRequest,
    state: State<'_, Arc<PythonResearchState>>,
    app: tauri::AppHandle,
) -> Result<ModelExperiment, String> {
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
        let experiment = ModelExperiment::ridge(
            request.project_revision_sha256,
            request.environment_sha256,
            request.input_evidence_sha256,
            request.seed,
        )
        .map_err(map_error)?;
        store
            .register(&request.user_id, experiment)
            .map_err(map_error)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn model_trial_complete(
    request: ModelTrialCompleteRequest,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<ModelExperiment, String> {
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
        store
            .complete_trial(
                &request.user_id,
                &request.experiment_id,
                &request.trial_id,
                request.attempt_id,
                request.selection_metric,
            )
            .map_err(map_error)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn model_trial_fail(
    request: ModelTrialFailRequest,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<ModelExperiment, String> {
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
            .ok_or_else(|| "model-trial-not-found".to_string())?;
        let attempt = attempt_store.get(&request.attempt_id).map_err(map_error)?;
        if attempt.user_id != request.user_id
            || attempt.project_id != MODEL_PROJECT_ID
            || attempt.revision_sha256 != trial.project_revision_sha256
            || attempt.environment_sha256 != trial.environment_sha256
        {
            return Err("model-trial-attempt-binding-invalid".into());
        }
        let status = match attempt.status {
            adaq_python_research::runner::AttemptStatus::Cancelled => TrialStatus::Cancelled,
            adaq_python_research::runner::AttemptStatus::Failed => TrialStatus::Failed,
            _ => return Err("model-trial-failure-requires-terminal-attempt".into()),
        };
        let mut experiment = experiment;
        experiment
            .fail_trial(&request.trial_id, request.attempt_id, status)
            .map_err(map_error)?;
        store
            .replace_experiment(&request.user_id, experiment)
            .map_err(map_error)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn model_selection_record(
    request: ModelSelectionRequest,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<ParameterSelectionDecision, String> {
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
    request: ModelFinalEvaluationRequest,
    state: State<'_, Arc<PythonResearchState>>,
    app: tauri::AppHandle,
) -> Result<FinalEvaluationReport, String> {
    let store = state.model_lab_store.clone();
    let research_state = state.inner().clone();
    let local_state = app
        .state::<Arc<crate::local_research::LocalResearchState>>()
        .inner()
        .clone();
    tauri::async_runtime::spawn_blocking(move || {
        let decision = store
            .decision(&request.user_id, &request.decision_id)
            .map_err(map_error)?;
        let experiment = store
            .experiment(&request.user_id, &decision.experiment_id)
            .map_err(map_error)?;
        let trial = experiment
            .trials
            .iter()
            .find(|trial| trial.trial_id == decision.selected_trial_id)
            .ok_or_else(|| "model-selection-trial-not-found".to_string())?;
        if trial.alpha != decision.selected_alpha {
            return Err("model-selection-alpha-mismatch".into());
        }
        let source_attempt_id = trial
            .attempt_ids
            .last()
            .ok_or_else(|| "model-selection-run-missing".to_string())?;
        let prior_run = store
            .run(&request.user_id, source_attempt_id)
            .map_err(map_error)?;
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
        let evidence = build_model_evidence(&input, Some(&factor_dataset)).map_err(map_error)?;
        let runner_input = evidence.runner_input().map_err(map_error)?;
        let runner_input = serde_json::to_value(runner_input).map_err(|error| error.to_string())?;
        let parameters = BTreeMap::from([("alpha".into(), decision.selected_alpha.to_string())]);
        let execution = research_state
            .run_trusted_project_with_execution(
                &request.user_id,
                MODEL_PROJECT_ID,
                &trial.project_revision_sha256,
                &trial.environment_sha256,
                7,
                Some(runner_input.clone()),
                Some(&parameters),
            )
            .map_err(map_error)?;
        let (replay_execution, replay_resource_policy) = research_state
            .run_trusted_project_verification(
                &request.user_id,
                MODEL_PROJECT_ID,
                &trial.project_revision_sha256,
                &trial.environment_sha256,
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
            != replay_artifact.to_bytes().map_err(map_error)?
        {
            return Err("model-artifact-replay-divergent".into());
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
            Some(first_artifact.clone()),
            Some(&factor_dataset),
        )
        .map_err(map_error)?;
        let prediction_input =
            model_prediction_input(runner_input, &first_artifact).map_err(map_error)?;
        let (prediction_execution, _) = research_state
            .run_trusted_project_verification(
                &request.user_id,
                MODEL_PROJECT_ID,
                &trial.project_revision_sha256,
                &trial.environment_sha256,
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
            decision.selected_alpha,
            trial.project_revision_sha256.clone(),
            trial.environment_sha256.clone(),
            trial.input_evidence_sha256.clone(),
            input,
            resource_policy,
            Some(replay_artifact),
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
        run.view.attempt_id = attempt_id;
        store
            .save_demo_run(&request.user_id, &run)
            .map_err(map_error)?;
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
        let final_end = evidence
            .fixture
            .sessions
            .last()
            .copied()
            .unwrap_or_default()
            - TARGET_HORIZON_BARS as u32;
        let forecasts = run
            .forecasts
            .iter()
            .filter(|row| row.datetime as u32 <= final_end)
            .cloned()
            .collect::<Vec<_>>();
        let mut ledger = FinalEvaluationLedger::default();
        let report = ledger
            .run(&decision, &forecasts, &run.final_labels)
            .map_err(map_error)?;
        store
            .save_final(&request.user_id, report)
            .map_err(map_error)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub fn runtime_profile(
    request: RuntimeProfileRequest,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<RuntimeProfileView, String> {
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
    request: RuntimePrepareRequest,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<(PreparationAttempt, Option<RuntimeRecord>), String> {
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
    request: ManagedRuntimePrepareRequest,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<(PreparationAttempt, Option<RuntimeRecord>), String> {
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
    request: ManagedEnvironmentPrepareRequest,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<EnvironmentSyncResult, String> {
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
    request: ManagedEnvironmentPrepareRequest,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<EnvironmentRecord, String> {
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
    request: ProjectRequest,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<Option<EnvironmentRecord>, String> {
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
        store.save_demo_run("user-a", &demo).unwrap();
        assert!(store.run("user-b", "model-store-attempt").is_err());
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
        local
            .factor
            .attach_python_attempt_store(python.attempt_store.clone())
            .unwrap();
        local.features.attach_python(python.clone());
        python.attach_queue(local.features.queue_notifier());
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
