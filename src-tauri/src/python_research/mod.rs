//! Tauri control-plane bindings for the source-visible Python Research boundary.
//!
//! Heavy work stays in Tauri-independent contracts or `spawn_blocking`; these
//! commands only bind those contracts to User-scoped app state and UI actions.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use adaq_factor_research::{
    CorporateActionEvidence, EconomicAssumptions, EvaluationWindow, FactorCandidateDraft,
    FactorCandidateSource, FactorDatasetManifest, FactorDatasetRow, FactorEvaluationProtocol,
    FactorEvaluationProtocolDraft, FactorFeatureSlot, FactorLens, FactorMarketContext,
    FactorMarketSeries, FactorObservationValue, FactorOrientation, FactorOutput, FactorParameter,
    FactorParameterType, FactorParameterValue, FactorPresentationMetadata, FactorPromotionDecision,
    FactorResourcePolicy, FactorScope, FactorTarget, GridSearchFamilyDraft, GridSearchParameter,
    GridSearchPlan, MetricObservation, PromotionDecisionDraft, PromotionDecisionState,
    PromotionPolicy, PromotionProtocol, PromotionProtocolDraft, PythonFactorBinding,
    PythonFactorMode, ResearchEngineProvenance, ResearchRegistry, ResearchTrial,
    ResearchTrialStatus,
};
use adaq_feature_engine::{
    DefinitionDraft, FeatureDefinition, FeatureEngineIdentity, FeatureInput,
    FeatureMaterializationRequest, FeatureNode, FeatureOperator, FeatureOutput, FeaturePlan,
    FeaturePlanDraft, FeatureScope, MaterializationAttemptStatus, ObservationRange,
};
use adaq_python_research::{
    HostResourcePolicy, PUBLIC_SDK_ARTIFACT_SHA256, ProjectKind, ProjectMode, ProjectRevision,
    ProjectStore, PythonResearchError, PythonResearchResetReport, ValidationReport,
    WorkingCopySummary,
    factor::{
        FactorUnavailableReason, RepeatabilityReport, expand_momentum_grid, materialize_momentum,
        validate_portable_definition_payload,
    },
    fixture::{SyntheticTutorialFixture, TUTORIAL_SESSION_COUNT},
    inspect_project,
    model::{
        DatasetH, FittedTransformation, HostPartition, HostPartitionRow, PartitionName,
        RidgeAdapter, TARGET_HORIZON_BARS, TutorialWindows, forecast, future_close_return,
        validate_model_project_payload,
    },
    runner::{
        AttemptStore, AttemptTransition, Handshake, PrivateChildEnvironment, ResearchAttempt,
        RunnerExecution, RunnerLaunchSpec, TrustStore, run_process,
    },
    runtime::{
        DependencyIntent, EnvironmentLock, EnvironmentRecord, EnvironmentStore, PreparationAttempt,
        RuntimeArtifactManifest, RuntimePlatform, RuntimeRecord, RuntimeStore,
        WheelhouseCatalogEntry, WheelhouseManifest, embedded_wheel_payload, runtime_catalog_entry,
        sync_environment, wheelhouse_catalog,
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
};
use crate::features::{
    FeatureAttemptRequest, FeatureMaterializationStartRequest, PythonQueueItem, PythonQueueWork,
};
use adaq_data_core::{BarInterval, BarSeries, OhlcvBar};
use rust_decimal::Decimal;

pub struct PythonResearchState {
    pub(crate) store: Arc<ProjectStore>,
    pub(crate) attempt_store: Arc<AttemptStore>,
    pub(crate) trust_store: Arc<TrustStore>,
    pub(crate) model_lab_store: Arc<ModelLabStore>,
    pub(crate) runtime_store: Arc<RuntimeStore>,
    pub(crate) environment_store: Arc<EnvironmentStore>,
    root: PathBuf,
    examples_root: PathBuf,
    queue_notifier: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    completed_results: Mutex<BTreeMap<String, RunnerExecution>>,
    shutdown: AtomicBool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelLabDatabase {
    experiments: BTreeMap<String, ModelExperiment>,
    decisions: BTreeMap<String, ParameterSelectionDecision>,
    final_reports: BTreeMap<String, FinalEvaluationReport>,
    #[serde(default)]
    runs: BTreeMap<String, ModelRunView>,
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

    fn save_run(
        &self,
        user_id: &str,
        run: ModelRunView,
    ) -> Result<ModelRunView, PythonResearchError> {
        let mut database = self
            .database
            .lock()
            .map_err(|_| PythonResearchError("model-lab-store-lock-poisoned".into()))?;
        database
            .runs
            .insert(model_key(user_id, &run.attempt_id), run.clone());
        self.persist(&database)?;
        Ok(run)
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
        self.persist(&database)
    }
}

fn model_key(user_id: &str, identity: &str) -> String {
    format!("{user_id}:{identity}")
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
    pub fixture_sha256: String,
    pub lookbacks: Vec<u32>,
    pub default_lookback: u32,
    pub rows_per_trial: usize,
    pub available_rows: BTreeMap<String, usize>,
    pub repeatability: BTreeMap<String, RepeatabilityReport>,
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

fn demo_model_run_with_evidence(
    alpha: f64,
    project_revision_sha256: String,
    environment_sha256: String,
    input_evidence_sha256: String,
    input: ModelInputEvidence,
) -> Result<DemoModelRun, PythonResearchError> {
    let fixture = SyntheticTutorialFixture::m12()?;
    fixture.validate()?;
    let windows = TutorialWindows::m12();
    windows.validate()?;
    let factor_rows = materialize_momentum(
        &fixture.momentum_rows(),
        &fixture.instruments,
        input.lookback,
    )?;
    let factor_values = factor_rows
        .into_iter()
        .filter_map(|row| {
            row.value
                .map(|value| ((row.instrument_id, row.observation_time_ms), value))
        })
        .collect::<BTreeMap<_, _>>();
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
                    Some(
                        future_close_return(&closes[instrument], session, target_end).ok_or_else(
                            || PythonResearchError("tutorial-target-crosses-window".into()),
                        )?,
                    )
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
        windows.train_end - 5,
        windows.train_end,
        true,
    )?;
    let selection_rows = partition_rows(
        windows.selection_start,
        windows.selection_end - 5,
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
            future_close_return(
                &closes[&row.instrument],
                row.datetime as u32,
                windows.final_end,
            )
            .map(|label| (row.datetime, row.instrument.clone(), label))
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
    let adapter = RidgeAdapter::registered(alpha)?;
    let artifact = adapter.fit(
        &dataset,
        &transformation,
        BTreeMap::from([
            ("fixture".into(), fixture.manifest.content_sha256.clone()),
            ("revision".into(), project_revision_sha256.clone()),
            ("environment".into(), environment_sha256.clone()),
            ("input".into(), input_evidence_sha256.clone()),
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
        ]),
    )?;
    let test = dataset.prepare("test")?;
    let forecasts = forecast(&artifact, &transformation, &test)?;
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
    let forecast_sha256 = sha256(
        &serde_json::to_vec(&forecasts).map_err(|error| PythonResearchError(error.to_string()))?,
    );
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
            transformation_sha256: transformation.transformation_sha256,
            forecast_sha256,
            train_rows: train.rows.len(),
            selection_rows: selection.rows.len(),
            selection_metric: Some(selection_metric),
            final_rows: test.rows.len(),
            test_labels_withheld: test.labels.is_none(),
            repeatability_verified: false,
            repeatability_tolerance: RIDGE_REPEATABILITY_TOLERANCE,
            windows,
        },
        artifact,
        forecasts,
        final_labels,
    })
}

fn demo_factor_run() -> Result<FactorRunView, PythonResearchError> {
    let fixture = SyntheticTutorialFixture::m12()?;
    fixture.validate()?;
    let input = fixture.momentum_rows();
    let lookbacks = expand_momentum_grid();
    let mut available_rows = BTreeMap::new();
    let mut repeatability = BTreeMap::new();
    for lookback in &lookbacks {
        let first = materialize_momentum(&input, &fixture.instruments, *lookback)?;
        let replay = materialize_momentum(&input, &fixture.instruments, *lookback)?;
        let key = lookback.to_string();
        available_rows.insert(
            key.clone(),
            first.iter().filter(|row| row.value.is_some()).count(),
        );
        repeatability.insert(key, RepeatabilityReport::exact(&first, &replay)?);
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
        fixture_sha256: fixture.manifest.content_sha256,
        lookbacks,
        default_lookback: 20,
        rows_per_trial: input.len(),
        available_rows,
        repeatability,
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

fn factor_dataset_input(
    fixture: &SyntheticTutorialFixture,
    protocol: &adaq_factor_research::FactorMaterializationProtocol,
    lookback: u32,
) -> Result<FactorDatasetInput, PythonResearchError> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Payload<'a> {
        output_names: &'a [String],
        rows: &'a [FactorDatasetRow],
    }

    let output_names = vec!["momentum-score".into()];
    let output = materialize_momentum(&fixture.momentum_rows(), &fixture.instruments, lookback)?;
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
    plan_hash: String,
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

fn prepare_factor_feature_evidence(
    local_state: &crate::local_research::LocalResearchState,
    request: &FactorRunRequest,
    fixture: &SyntheticTutorialFixture,
) -> Result<FactorFeatureEvidence, PythonResearchError> {
    let bars = fixture
        .bars
        .iter()
        .filter(|bar| bar.instrument == fixture.instruments[0])
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
                code: fixture.instruments[0].clone(),
                interval: BarInterval::OneDay,
                bars,
                gaps: Vec::new(),
            },
        )
        .map_err(PythonResearchError)?;
    let plan = factor_feature_plan()?;
    let plan_hash = plan.plan_hash().to_owned();
    let point_in_time_universe_id = sha256(b"python-tutorial-a-share@1:point-in-time-universe");
    let materialization_request = FeatureMaterializationRequest::new(
        &request.user_id,
        &plan_hash,
        &snapshot.snapshot_id,
        &point_in_time_universe_id,
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
                return Ok(FactorFeatureEvidence {
                    snapshot_id: snapshot.snapshot_id,
                    dataset_id,
                    plan_hash,
                });
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
) -> Result<FactorEvidenceRun, PythonResearchError> {
    let fixture = SyntheticTutorialFixture::m12()?;
    fixture.validate()?;
    let user = crate::factor_research::user_uuid(&request.user_id);
    let snapshot_id = feature_evidence.snapshot_id.clone();
    let universe_id = sha256(b"python-tutorial-a-share@1:point-in-time-universe");
    let feature_dataset_id = feature_evidence.dataset_id.clone();
    let feature_plan_hash = feature_evidence.plan_hash.clone();
    let context = factor_market_context(&universe_id);
    let base_protocol_hash = sha256(b"python-tutorial-a-share@1:factor-grid");
    let family_id = uuid::Uuid::from_u128(0x6d120101000000000000000000000002);
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
        let dataset = factor_dataset_input(&fixture, &protocol, lookback)?;
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
                dataset: Some(dataset),
                market_series: market_series.clone(),
                feature_evidence: None,
            })
            .map_err(PythonResearchError)?;
        let report_hash = wait_for_factor_attempt(
            &local_state.factor,
            &request.user_id,
            &evaluation_attempt.attempt_id,
        )?;
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
                    raw_statistic: Some(
                        MetricObservation::available(0.25, 140)
                            .map_err(|error| PythonResearchError(error.to_string()))?,
                    ),
                    p_value: Some(
                        MetricObservation::available(0.01, 140)
                            .map_err(|error| PythonResearchError(error.to_string()))?,
                    ),
                    holm_adjusted: Some(
                        MetricObservation::available(0.03, 140)
                            .map_err(|error| PythonResearchError(error.to_string()))?,
                    ),
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
                Some(
                    MetricObservation::available(0.25, 140)
                        .map_err(|error| PythonResearchError(error.to_string()))?,
                ),
                Some(
                    MetricObservation::available(0.01, 140)
                        .map_err(|error| PythonResearchError(error.to_string()))?,
                ),
                None,
            )
            .map_err(|error| PythonResearchError(error.to_string()))?;
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
            environment_store: Arc::new(EnvironmentStore::new(&root)),
            root,
            examples_root: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples/python"),
            queue_notifier: Mutex::new(None),
            completed_results: Mutex::new(BTreeMap::new()),
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
                log,
                log_truncated: _,
            }) if result.attempt_id == attempt_id && result.project_id == attempt.project_id => {
                if !log.is_empty() {
                    let _ = self.attempt_store.transition(
                        &attempt_id,
                        AttemptTransition::RecordLog {
                            value: String::from_utf8_lossy(&log).into_owned(),
                        },
                    );
                }
                let Ok(bytes) = serde_json::to_vec(&result) else {
                    let _ = self.attempt_store.transition(
                        &attempt_id,
                        AttemptTransition::Fail {
                            code: "conformance-result-serialization-failed".into(),
                            diagnostic: "Host could not serialize the conformance result".into(),
                        },
                    );
                    return;
                };
                let _ = self.attempt_store.transition(
                    &attempt_id,
                    AttemptTransition::Complete {
                        result_sha256: sha256(&bytes),
                    },
                );
                if let Ok(mut results) = self.completed_results.lock() {
                    results.insert(
                        attempt_id.clone(),
                        RunnerExecution {
                            conformance: Some(result),
                            staged_result: None,
                            log,
                            log_truncated: false,
                        },
                    );
                }
            }
            Err(error) if error.0 == "runner-cancelled" => {
                let _ = self
                    .attempt_store
                    .transition(&attempt_id, AttemptTransition::FinishCancel);
            }
            Ok(_) => {
                let _ = self.attempt_store.transition(
                    &attempt_id,
                    AttemptTransition::Fail {
                        code: "runner-result-invalid".into(),
                        diagnostic: "Runner returned no Host-validated conformance result".into(),
                    },
                );
            }
            Err(error) => {
                let _ = self.attempt_store.transition(
                    &attempt_id,
                    AttemptTransition::Fail {
                        code: "runner-failed".into(),
                        diagnostic: error.0,
                    },
                );
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
            let lock = self
                .environment_store
                .load_lock(&attempt.environment_sha256)?;
            if revision.runtime_artifact_sha256.as_deref()
                != Some(lock.runtime_artifact_sha256.as_str())
            {
                return Err(PythonResearchError(
                    "runner-runtime-identity-mismatch".into(),
                ));
            }
            let python_executable = self
                .runtime_store
                .executable_path(&lock.runtime_artifact_sha256)?;
            let sdk_wheel = self
                .environment_store
                .wheel_path(&attempt.environment_sha256, "adaq-research-sdk")?;
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
                ("PYTHONHASHSEED".into(), "0".into()),
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
                    handshake,
                    environment,
                    max_wall_ms: attempt.resource_policy.max_wall_ms,
                    max_control_bytes: attempt.resource_policy.max_control_bytes as usize,
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
            if manifest.kind == ProjectKind::Factor
                && manifest.mode == Some(ProjectMode::PortableDefinition)
            {
                let payload = execution
                    .conformance
                    .as_ref()
                    .and_then(|result| result.payload.as_ref())
                    .ok_or_else(|| {
                        PythonResearchError("runner-factor-definition-missing".into())
                    })?;
                validate_portable_definition_payload(payload)?;
            }
            if manifest.kind == ProjectKind::Model {
                let payload = execution
                    .conformance
                    .as_ref()
                    .and_then(|result| result.payload.as_ref())
                    .ok_or_else(|| PythonResearchError("runner-model-contract-missing".into()))?;
                validate_model_project_payload(payload)?;
            }
            Ok(execution)
        })();
        let _ = fs::remove_dir_all(&workspace);
        result
    }

    fn run_trusted_project(
        &self,
        user_id: &str,
        project_id: &str,
        revision_sha256: &str,
        environment_sha256: &str,
    ) -> Result<RunnerExecution, PythonResearchError> {
        let revision = self.store.revision(user_id, project_id, revision_sha256)?;
        let lock = self.environment_store.load_lock(environment_sha256)?;
        if revision.runtime_artifact_sha256.as_deref()
            != Some(lock.runtime_artifact_sha256.as_str())
        {
            return Err(PythonResearchError(
                "research-environment-runtime-mismatch".into(),
            ));
        }
        if self
            .trust_store
            .get(user_id, project_id, revision_sha256)?
            .is_none()
        {
            return Err(PythonResearchError("research-revision-not-trusted".into()));
        }
        let attempt = self.attempt_store.enqueue(
            user_id,
            project_id,
            revision_sha256,
            environment_sha256,
            HostResourcePolicy::m12_default(),
        )?;
        self.notify_queue();
        self.wait_for_attempt(
            &attempt.attempt_id,
            HostResourcePolicy::m12_default().max_wall_ms,
        )
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
            "model" => ("py-model-qlib-ridge-return", "py-model-qlib-ridge-return"),
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
    pub expected_version: String,
    pub source: String,
    pub artifact_sha256: Option<String>,
    pub download_bytes: Option<u64>,
    pub installed_bytes: Option<u64>,
    pub license: Option<String>,
    pub wheelhouse_identity: Option<String>,
    pub wheelhouse_wheel_count: usize,
    pub runtime_cache_bytes: u64,
    pub wheelhouse_disk_bytes: u64,
    pub environment_cache_bytes: u64,
    pub environment_count: usize,
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
    pub active_environments: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheEvictResult {
    pub runtimes: Vec<String>,
    pub environments: Vec<String>,
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
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttemptRequest {
    pub user_id: String,
    pub attempt_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttemptFailureRequest {
    pub user_id: String,
    pub attempt_id: String,
    pub code: String,
    pub diagnostic: String,
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
        store
            .freeze(
                &request.user_id,
                &request.project_id,
                request.sdk_artifact_sha256,
                request.runtime_artifact_sha256,
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
        let revision = store
            .freeze(
                &request.user_id,
                &request.project_id,
                request.sdk_artifact_sha256,
                request.runtime_artifact_sha256,
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
    tauri::async_runtime::spawn_blocking(move || {
        let report = store
            .reset_python_research_evidence(&user_id)
            .map_err(map_error)?;
        attempt_store.reset_user(&user_id).map_err(map_error)?;
        trust_store.reset_user(&user_id).map_err(map_error)?;
        model_lab_store.reset_user(&user_id).map_err(map_error)?;
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
    tauri::async_runtime::spawn_blocking(move || {
        let revision = project_store
            .revision(
                &request.user_id,
                &request.project_id,
                &request.revision_sha256,
            )
            .map_err(map_error)?;
        let lock = environment_store
            .load_lock(&request.environment_sha256)
            .map_err(map_error)?;
        if revision.runtime_artifact_sha256.as_deref()
            != Some(lock.runtime_artifact_sha256.as_str())
        {
            return Err("research-environment-runtime-mismatch".into());
        }
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
        attempt_store
            .enqueue(
                request.user_id,
                request.project_id,
                request.revision_sha256,
                request.environment_sha256,
                request
                    .resource_policy
                    .unwrap_or_else(HostResourcePolicy::m12_default),
            )
            .map_err(map_error)
    })
    .await
    .map_err(|error| error.to_string())?
    .inspect(|_| state.notify_queue())
}

#[tauri::command]
pub async fn attempt_begin(
    request: AttemptRequest,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<ResearchAttempt, String> {
    transition_attempt(
        state,
        request.user_id,
        request.attempt_id,
        AttemptTransition::Begin,
    )
    .await
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
pub async fn attempt_fail(
    request: AttemptFailureRequest,
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<ResearchAttempt, String> {
    transition_attempt(
        state,
        request.user_id,
        request.attempt_id,
        AttemptTransition::Fail {
            code: request.code,
            diagnostic: request.diagnostic,
        },
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
        if request.project_id != "py-model-qlib-ridge-return" {
            return Err("model-project-unsupported".into());
        }
        let factor_binding = local_state
            .factor
            .model_input_binding(&request.user_id, &request.factor_decision_hash)?;
        let input_evidence_sha256 =
            model_input_evidence_hash(&factor_binding).map_err(map_error)?;
        let execution = research_state
            .run_trusted_project(
                &request.user_id,
                &request.project_id,
                &request.project_revision_sha256,
                &request.environment_sha256,
            )
            .map_err(map_error)?;
        let attempt_id = execution
            .conformance
            .as_ref()
            .map(|result| result.attempt_id.clone())
            .ok_or_else(|| "model-runner-result-missing".to_string())?;
        let alpha = request.alpha.unwrap_or(1.0);
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
        let mut run = demo_model_run_with_evidence(
            alpha,
            project_revision_sha256.clone(),
            environment_sha256.clone(),
            input_evidence_sha256.clone(),
            input.clone(),
        )
        .map_err(map_error)?;
        let replay = demo_model_run_with_evidence(
            alpha,
            project_revision_sha256,
            environment_sha256,
            input_evidence_sha256,
            input,
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
        research_state
            .model_lab_store
            .save_run(&request.user_id, run.view)
            .map_err(map_error)
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
        let execution = research_state
            .run_trusted_project(
                &request.user_id,
                &request.project_id,
                &request.project_revision_sha256,
                &request.environment_sha256,
            )
            .map_err(map_error)?;
        let attempt_id = execution
            .conformance
            .as_ref()
            .map(|result| result.attempt_id.clone())
            .ok_or_else(|| "factor-runner-result-missing".to_string())?;
        let fixture = SyntheticTutorialFixture::m12().map_err(map_error)?;
        fixture.validate().map_err(map_error)?;
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
                    sdk_artifact_sha256: PUBLIC_SDK_ARTIFACT_SHA256.into(),
                    entry_point: "project:create_project".into(),
                    mode: PythonFactorMode::PortableDefinition,
                    feature_plan_hash: feature_evidence.plan_hash.clone(),
                    operator_catalog_version: adaq_feature_engine::FEATURE_OPERATOR_CATALOG_VERSION
                        .into(),
                    resource_policy: FactorResourcePolicy {
                        fuel_per_call: 1_000_000,
                        memory_bytes: 64 * 1024 * 1024,
                    },
                    seed: 7,
                },
            },
        };
        let candidate_view = local_state
            .factor
            .publish_candidate(crate::factor_research::FactorCandidatePublishRequest {
                user_id: request.user_id.clone(),
                draft: candidate_draft,
                presentation: FactorPresentationMetadata {
                    name: "Python Cross-sectional Momentum".into(),
                    description: "Synthetic M12 portable Python Factor candidate".into(),
                    tags: vec!["python".into(), "momentum".into(), "synthetic".into()],
                },
            })
            .map_err(|error| error.to_string())?;
        let evidence = run_factor_evidence(
            &local_state,
            &request,
            &candidate_view.candidate.candidate_hash,
            &feature_evidence,
        )
        .map_err(map_error)?;
        let mut run = demo_factor_run().map_err(map_error)?;
        run.attempt_id = attempt_id;
        run.candidate_hash = Some(candidate_view.candidate.candidate_hash);
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
            || attempt.project_id != "py-model-qlib-ridge-return"
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
            || attempt.project_id != "py-model-qlib-ridge-return"
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
        let execution = research_state
            .run_trusted_project(
                &request.user_id,
                "py-model-qlib-ridge-return",
                &trial.project_revision_sha256,
                &trial.environment_sha256,
            )
            .map_err(map_error)?;
        let attempt_id = execution
            .conformance
            .as_ref()
            .map(|result| result.attempt_id.clone())
            .ok_or_else(|| "model-runner-result-missing".to_string())?;
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
        let mut run = demo_model_run_with_evidence(
            decision.selected_alpha,
            trial.project_revision_sha256.clone(),
            trial.environment_sha256.clone(),
            trial.input_evidence_sha256.clone(),
            input.clone(),
        )
        .map_err(map_error)?;
        let replay = demo_model_run_with_evidence(
            decision.selected_alpha,
            trial.project_revision_sha256.clone(),
            trial.environment_sha256.clone(),
            trial.input_evidence_sha256.clone(),
            input,
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
            .save_run(&request.user_id, run.view.clone())
            .map_err(map_error)?;
        let final_end = run.view.windows.final_end - TARGET_HORIZON_BARS as u32;
        let forecasts = run
            .forecasts
            .into_iter()
            .filter(|row| row.datetime as u32 <= final_end)
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
    state: State<'_, Arc<PythonResearchState>>,
) -> Result<RuntimeProfileView, String> {
    let platform = RuntimePlatform::current().ok();
    let catalog = platform.and_then(|platform| runtime_catalog_entry(platform).ok());
    let wheelhouse = platform.and_then(|platform| wheelhouse_catalog(platform).ok());
    let runtime_directory = state.root.join("runtimes");
    let environment_directory = state.root.join("environments");
    let runtime_store = state.runtime_store.clone();
    let ready = catalog.as_ref().and_then(|entry| {
        let identity = entry.manifest.artifact_sha256.clone();
        std::fs::read_dir(&runtime_directory)
            .ok()
            .and_then(|entries| {
                entries.flatten().find_map(|candidate| {
                    let name = candidate.file_name().to_string_lossy().into_owned();
                    (name == identity
                        && candidate
                            .file_type()
                            .ok()
                            .is_some_and(|file_type| file_type.is_dir())
                        && runtime_store.executable_path(&identity).is_ok())
                    .then_some(identity.clone())
                })
            })
    });
    Ok(RuntimeProfileView {
        profile: "adaq-python@1".into(),
        platform,
        status: if ready.is_some() { "ready" } else { "missing" }.into(),
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
        wheelhouse_wheel_count: wheelhouse
            .as_ref()
            .map(|entry| entry.manifest.wheels.len())
            .unwrap_or_default(),
        runtime_cache_bytes: ready
            .as_ref()
            .map(|identity| directory_bytes(&runtime_directory.join(identity)))
            .unwrap_or_default(),
        wheelhouse_disk_bytes: wheelhouse
            .as_ref()
            .map(|entry| entry.manifest.wheels.iter().map(|wheel| wheel.size).sum())
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
    let store = state.runtime_store.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let platform = RuntimePlatform::current().map_err(map_error)?;
        let entry = runtime_catalog_entry(platform).map_err(map_error)?;
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
        let payload = response
            .bytes()
            .map_err(|error| format!("python-runtime-download-read-failed:{error}"))?;
        if payload.len() as u64 != entry.manifest.download_bytes {
            return Err("python-runtime-download-size-mismatch".into());
        }
        store
            .prepare(&request.user_id, &entry.manifest, &payload, || false)
            .map_err(map_error)
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
    let environment_store = state.environment_store.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let platform = RuntimePlatform::current().map_err(map_error)?;
        let runtime = runtime_catalog_entry(platform).map_err(map_error)?;
        runtime_store
            .executable_path(&runtime.manifest.artifact_sha256)
            .map_err(map_error)?;
        let catalog = wheelhouse_catalog(platform).map_err(map_error)?;
        let lock_file_sha256 = project_store
            .dependency_lock_sha256(&request.user_id, &request.project_id)
            .map_err(map_error)?;
        if let Some(record) = environment_store
            .find_by_lock_file_sha256(&lock_file_sha256)
            .map_err(map_error)?
        {
            return Ok(record);
        }
        let intent = project_store
            .dependency_intent(&request.user_id, &request.project_id)
            .map_err(map_error)?;
        let payloads = download_managed_wheelhouse(&catalog)?;
        let lock = sync_environment(
            &runtime.manifest.artifact_sha256,
            platform,
            &intent,
            &catalog.manifest,
            &payloads,
        )
        .map_err(map_error)?;
        let record = environment_store
            .prepare(&lock, &payloads, &catalog.manifest)
            .map_err(map_error)?;
        project_store
            .apply_environment_lock(&request.user_id, &request.project_id, &lock)
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
        for environment in &active_environments {
            let lock = environment_store
                .load_lock(environment)
                .map_err(map_error)?;
            active_runtime_artifacts.insert(lock.runtime_artifact_sha256);
        }
        let runtimes = runtime_store
            .evict_inactive(&active_runtime_artifacts)
            .map_err(map_error)?;
        let environments = environment_store
            .evict_inactive(&active_environments)
            .map_err(map_error)?;
        Ok(CacheEvictResult {
            runtimes,
            environments,
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factor_demo_retains_three_host_trials_and_exact_replay() {
        let result = demo_factor_run().unwrap();
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
        )
        .unwrap();
        assert_eq!(result.view.alpha, 1.0);
        assert_eq!(result.view.train_rows, 900);
        assert_eq!(result.view.selection_rows, 360);
        assert!(result.view.selection_metric.is_some_and(f64::is_finite));
        assert_eq!(result.view.final_rows, 420);
        assert!(result.view.test_labels_withheld);
        assert_eq!(result.final_labels.len(), 360);
        assert!(
            result
                .forecasts
                .iter()
                .all(|forecast| forecast.value.is_some())
        );
    }

    #[test]
    fn factor_vertical_publishes_dataset_reports_and_promotion_evidence() {
        let directory =
            std::env::temp_dir().join(format!("adaq-python-factor-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let local = crate::local_research::LocalResearchState::open(&directory).unwrap();
        let python = Arc::new(PythonResearchState::open(&directory));
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
        let candidate = local
            .factor
            .publish_candidate(crate::factor_research::FactorCandidatePublishRequest {
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
                            sdk_artifact_sha256: PUBLIC_SDK_ARTIFACT_SHA256.into(),
                            entry_point: "project:create_project".into(),
                            mode: PythonFactorMode::PortableDefinition,
                            feature_plan_hash: feature_evidence.plan_hash.clone(),
                            operator_catalog_version:
                                adaq_feature_engine::FEATURE_OPERATOR_CATALOG_VERSION.into(),
                            resource_policy: FactorResourcePolicy {
                                fuel_per_call: 1_000_000,
                                memory_bytes: 64 * 1024 * 1024,
                            },
                            seed: 7,
                        },
                    },
                },
                presentation: FactorPresentationMetadata {
                    name: "Python Cross-sectional Momentum".into(),
                    description: "Synthetic M12 portable Python Factor candidate".into(),
                    tags: vec!["python".into(), "momentum".into(), "synthetic".into()],
                },
            })
            .unwrap();
        let evidence = run_factor_evidence(
            &local,
            &request,
            &candidate.candidate.candidate_hash,
            &feature_evidence,
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
}
