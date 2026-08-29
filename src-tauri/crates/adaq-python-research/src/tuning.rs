//! Host-owned Ridge Grid, User Selection, Final Evaluation, and replay gates.

use crate::{PythonResearchError, invalid, is_sha256, model::ForecastRow, sha256};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const RIDGE_ALPHAS: [f64; 3] = [0.1, 1.0, 10.0];
pub const RIDGE_REPEATABILITY_TOLERANCE: f64 = 1e-9;
const MAX_TRIAL_DIAGNOSTICS: usize = 32;
const MAX_TRIAL_DIAGNOSTIC_BYTES: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TrialStatus {
    Registered,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
    Stale,
    Invalid,
    Unsupported,
    Superseded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RepeatabilityState {
    Unverified,
    Verified,
    Divergent,
}

impl Default for RepeatabilityState {
    fn default() -> Self {
        Self::Unverified
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EvidenceState {
    OutOfSample,
    Overlapping,
    Unknown,
}

impl Default for EvidenceState {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelTrial {
    pub trial_id: String,
    pub experiment_id: String,
    pub alpha: f64,
    pub project_revision_sha256: String,
    pub environment_sha256: String,
    pub input_evidence_sha256: String,
    pub seed: u64,
    pub status: TrialStatus,
    pub attempt_ids: Vec<String>,
    pub selection_metric: Option<f64>,
    pub evidence_state: EvidenceState,
    #[serde(default)]
    pub binding_sha256: String,
    #[serde(default)]
    pub successful_attempt_id: Option<String>,
    #[serde(default)]
    pub candidate_artifact_sha256: Option<String>,
    #[serde(default)]
    pub repeatability_state: RepeatabilityState,
    #[serde(default)]
    pub diagnostics: Vec<String>,
}

impl ModelTrial {
    pub fn validate(&self) -> Result<(), PythonResearchError> {
        if !is_sha256(&self.trial_id)
            || !is_sha256(&self.experiment_id)
            || !self.alpha.is_finite()
            || self.alpha <= 0.0
            || !is_sha256(&self.project_revision_sha256)
            || !is_sha256(&self.environment_sha256)
            || !is_sha256(&self.input_evidence_sha256)
            || self
                .attempt_ids
                .iter()
                .any(|attempt_id| attempt_id.is_empty())
            || self
                .selection_metric
                .is_some_and(|metric| !metric.is_finite())
            || (!self.binding_sha256.is_empty() && !is_sha256(&self.binding_sha256))
            || self
                .successful_attempt_id
                .as_deref()
                .is_some_and(|attempt_id| attempt_id.trim().is_empty())
            || self
                .candidate_artifact_sha256
                .as_deref()
                .is_some_and(|artifact| !is_sha256(artifact))
            || self.successful_attempt_id.is_some() != self.candidate_artifact_sha256.is_some()
            || self
                .successful_attempt_id
                .as_ref()
                .is_some_and(|attempt_id| !self.attempt_ids.contains(attempt_id))
            || self.status != TrialStatus::Completed && self.successful_attempt_id.is_some()
            || self.status == TrialStatus::Completed
                && self.repeatability_state == RepeatabilityState::Verified
                && self.successful_attempt_id.is_none()
            || self.candidate_artifact_sha256.is_some()
                && self.repeatability_state != RepeatabilityState::Verified
            || self.diagnostics.len() > MAX_TRIAL_DIAGNOSTICS
            || self
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.len() > MAX_TRIAL_DIAGNOSTIC_BYTES)
        {
            return Err(invalid("model-trial-invalid"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelExperiment {
    pub experiment_id: String,
    pub project_revision_sha256: String,
    pub environment_sha256: String,
    pub input_evidence_sha256: String,
    pub seed: u64,
    pub trials: Vec<ModelTrial>,
    #[serde(default)]
    pub binding_sha256: String,
    #[serde(default)]
    pub factor_decision_hash: String,
    #[serde(default)]
    pub parent_decision_id: Option<String>,
    #[serde(default)]
    pub lineage_evidence_state: EvidenceState,
}

impl ModelExperiment {
    pub fn ridge(
        project_revision_sha256: impl Into<String>,
        environment_sha256: impl Into<String>,
        input_evidence_sha256: impl Into<String>,
        seed: u64,
    ) -> Result<Self, PythonResearchError> {
        let project_revision_sha256 = project_revision_sha256.into();
        let environment_sha256 = environment_sha256.into();
        let input_evidence_sha256 = input_evidence_sha256.into();
        let binding_sha256 = sha256(
            format!(
                "{project_revision_sha256}:{environment_sha256}:{input_evidence_sha256}:{seed}"
            )
            .as_bytes(),
        );
        Self::ridge_with_binding(
            project_revision_sha256,
            environment_sha256,
            input_evidence_sha256,
            seed,
            binding_sha256,
        )
    }

    pub fn ridge_with_binding(
        project_revision_sha256: impl Into<String>,
        environment_sha256: impl Into<String>,
        input_evidence_sha256: impl Into<String>,
        seed: u64,
        binding_sha256: impl Into<String>,
    ) -> Result<Self, PythonResearchError> {
        Self::ridge_with_binding_and_lineage(
            project_revision_sha256,
            environment_sha256,
            input_evidence_sha256,
            seed,
            binding_sha256,
            None,
        )
    }

    pub fn ridge_with_binding_and_lineage(
        project_revision_sha256: impl Into<String>,
        environment_sha256: impl Into<String>,
        input_evidence_sha256: impl Into<String>,
        seed: u64,
        binding_sha256: impl Into<String>,
        parent_decision_id: Option<String>,
    ) -> Result<Self, PythonResearchError> {
        let project_revision_sha256 = project_revision_sha256.into();
        let environment_sha256 = environment_sha256.into();
        let input_evidence_sha256 = input_evidence_sha256.into();
        let binding_sha256 = binding_sha256.into();
        let lineage_evidence_state = if parent_decision_id.is_some() {
            EvidenceState::Overlapping
        } else {
            EvidenceState::Unknown
        };
        if !is_sha256(&project_revision_sha256)
            || !is_sha256(&environment_sha256)
            || !is_sha256(&input_evidence_sha256)
            || !is_sha256(&binding_sha256)
            || parent_decision_id
                .as_deref()
                .is_some_and(|decision_id| !is_sha256(decision_id))
        {
            return Err(invalid("model-experiment-identity-invalid"));
        }
        let experiment_id = sha256(
            format!(
                "{project_revision_sha256}:{environment_sha256}:{input_evidence_sha256}:{seed}:{binding_sha256}:{}",
                parent_decision_id.as_deref().unwrap_or_default()
            )
            .as_bytes(),
        );
        let trials = RIDGE_ALPHAS
            .into_iter()
            .map(|alpha| ModelTrial {
                trial_id: sha256(format!("{experiment_id}:{alpha}").as_bytes()),
                experiment_id: experiment_id.clone(),
                alpha,
                project_revision_sha256: project_revision_sha256.clone(),
                environment_sha256: environment_sha256.clone(),
                input_evidence_sha256: input_evidence_sha256.clone(),
                seed,
                status: TrialStatus::Registered,
                attempt_ids: Vec::new(),
                selection_metric: None,
                evidence_state: lineage_evidence_state,
                binding_sha256: binding_sha256.clone(),
                successful_attempt_id: None,
                candidate_artifact_sha256: None,
                repeatability_state: RepeatabilityState::Unverified,
                diagnostics: Vec::new(),
            })
            .collect::<Vec<_>>();
        Ok(Self {
            experiment_id,
            project_revision_sha256,
            environment_sha256,
            input_evidence_sha256,
            seed,
            trials,
            binding_sha256,
            factor_decision_hash: String::new(),
            parent_decision_id,
            lineage_evidence_state,
        })
    }

    pub fn validate(&self) -> Result<(), PythonResearchError> {
        let mut trial_ids = BTreeMap::new();
        let mut alphas = BTreeMap::new();
        let mut attempt_ids = BTreeMap::new();
        let mut candidate_artifacts = BTreeMap::new();
        let expected_experiment_id = sha256(
            format!(
                "{}:{}:{}:{}:{}:{}",
                self.project_revision_sha256,
                self.environment_sha256,
                self.input_evidence_sha256,
                self.seed,
                self.binding_sha256,
                self.parent_decision_id.as_deref().unwrap_or_default()
            )
            .as_bytes(),
        );
        if !is_sha256(&self.experiment_id)
            || self.experiment_id != expected_experiment_id
            || !is_sha256(&self.project_revision_sha256)
            || !is_sha256(&self.environment_sha256)
            || !is_sha256(&self.input_evidence_sha256)
            || !is_sha256(&self.binding_sha256)
            || (!self.factor_decision_hash.is_empty() && !is_sha256(&self.factor_decision_hash))
            || self
                .parent_decision_id
                .as_deref()
                .is_some_and(|decision_id| !is_sha256(decision_id))
            || !matches!(
                self.lineage_evidence_state,
                EvidenceState::Unknown | EvidenceState::Overlapping
            )
            || self.parent_decision_id.is_some()
                != (self.lineage_evidence_state == EvidenceState::Overlapping)
            || self.trials.len() != RIDGE_ALPHAS.len()
            || self.trials.iter().enumerate().any(|(index, trial)| {
                trial.validate().is_err()
                    || trial.experiment_id != self.experiment_id
                    || trial.project_revision_sha256 != self.project_revision_sha256
                    || trial.environment_sha256 != self.environment_sha256
                    || trial.input_evidence_sha256 != self.input_evidence_sha256
                    || trial.seed != self.seed
                    || trial.binding_sha256 != self.binding_sha256
                    || trial.successful_attempt_id.is_some()
                        != trial.candidate_artifact_sha256.is_some()
                    || trial
                        .candidate_artifact_sha256
                        .as_ref()
                        .is_some_and(|artifact| {
                            candidate_artifacts.insert(artifact.clone(), ()).is_some()
                        })
                    || trial.evidence_state != self.lineage_evidence_state
                    || trial.alpha.to_bits() != RIDGE_ALPHAS[index].to_bits()
                    || trial.trial_id
                        != sha256(format!("{}:{}", self.experiment_id, trial.alpha).as_bytes())
                    || trial
                        .attempt_ids
                        .iter()
                        .any(|attempt_id| attempt_ids.insert(attempt_id.clone(), ()).is_some())
                    || !RIDGE_ALPHAS.contains(&trial.alpha)
                    || trial_ids.insert(trial.trial_id.clone(), ()).is_some()
                    || alphas.insert(trial.alpha.to_bits(), ()).is_some()
            })
        {
            return Err(invalid("model-experiment-invalid"));
        }
        Ok(())
    }

    pub fn complete_trial(
        &mut self,
        trial_id: &str,
        attempt_id: impl Into<String>,
        selection_metric: f64,
    ) -> Result<(), PythonResearchError> {
        self.complete_trial_with_repeatability(
            trial_id,
            attempt_id,
            selection_metric,
            RepeatabilityState::Verified,
        )
    }

    pub fn complete_trial_with_candidate(
        &mut self,
        trial_id: &str,
        attempt_id: impl Into<String>,
        selection_metric: f64,
        candidate_artifact_sha256: impl Into<String>,
    ) -> Result<(), PythonResearchError> {
        let attempt_id = attempt_id.into();
        let candidate_artifact_sha256 = candidate_artifact_sha256.into();
        let lineage_evidence_state = self.lineage_evidence_state;
        let trial = self
            .trials
            .iter_mut()
            .find(|trial| trial.trial_id == trial_id)
            .ok_or_else(|| invalid("model-trial-not-found"))?;
        if !selection_metric.is_finite()
            || attempt_id.trim().is_empty()
            || !is_sha256(&candidate_artifact_sha256)
            || trial.status != TrialStatus::Registered
            || trial.attempt_ids.iter().any(|id| id == &attempt_id)
        {
            return Err(invalid("model-trial-completion-invalid"));
        }
        trial.attempt_ids.push(attempt_id.clone());
        trial.selection_metric = Some(selection_metric);
        trial.status = TrialStatus::Completed;
        trial.evidence_state = lineage_evidence_state;
        trial.repeatability_state = RepeatabilityState::Verified;
        trial.successful_attempt_id = Some(attempt_id);
        trial.candidate_artifact_sha256 = Some(candidate_artifact_sha256);
        Ok(())
    }

    pub fn complete_trial_with_repeatability(
        &mut self,
        trial_id: &str,
        attempt_id: impl Into<String>,
        selection_metric: f64,
        repeatability_state: RepeatabilityState,
    ) -> Result<(), PythonResearchError> {
        if repeatability_state == RepeatabilityState::Verified {
            return Err(invalid("model-trial-candidate-required"));
        }
        let attempt_id = attempt_id.into();
        let lineage_evidence_state = self.lineage_evidence_state;
        let trial = self
            .trials
            .iter_mut()
            .find(|trial| trial.trial_id == trial_id)
            .ok_or_else(|| invalid("model-trial-not-found"))?;
        if !selection_metric.is_finite()
            || attempt_id.trim().is_empty()
            || trial.status != TrialStatus::Registered
            || trial.attempt_ids.iter().any(|id| id == &attempt_id)
        {
            return Err(invalid("model-trial-completion-invalid"));
        }
        trial.attempt_ids.push(attempt_id);
        trial.selection_metric = Some(selection_metric);
        trial.status = TrialStatus::Completed;
        trial.evidence_state = lineage_evidence_state;
        trial.repeatability_state = repeatability_state;
        Ok(())
    }

    pub fn fail_trial(
        &mut self,
        trial_id: &str,
        attempt_id: impl Into<String>,
        status: TrialStatus,
    ) -> Result<(), PythonResearchError> {
        self.fail_trial_with_diagnostic(trial_id, attempt_id, status, "")
    }

    pub fn fail_trial_with_diagnostic(
        &mut self,
        trial_id: &str,
        attempt_id: impl Into<String>,
        status: TrialStatus,
        diagnostic: impl Into<String>,
    ) -> Result<(), PythonResearchError> {
        let attempt_id = attempt_id.into();
        let diagnostic = diagnostic.into();
        let lineage_evidence_state = self.lineage_evidence_state;
        let trial = self
            .trials
            .iter_mut()
            .find(|trial| trial.trial_id == trial_id)
            .ok_or_else(|| invalid("model-trial-not-found"))?;
        if !matches!(
            status,
            TrialStatus::Failed
                | TrialStatus::Cancelled
                | TrialStatus::Interrupted
                | TrialStatus::Stale
                | TrialStatus::Invalid
                | TrialStatus::Unsupported
                | TrialStatus::Superseded
        ) || trial.status != TrialStatus::Registered
            || attempt_id.trim().is_empty()
            || trial.attempt_ids.iter().any(|id| id == &attempt_id)
            || diagnostic.len() > MAX_TRIAL_DIAGNOSTIC_BYTES
        {
            return Err(invalid("model-trial-failure-invalid"));
        }
        trial.attempt_ids.push(attempt_id);
        trial.status = status;
        trial.evidence_state = lineage_evidence_state;
        trial.repeatability_state = RepeatabilityState::Unverified;
        if !diagnostic.is_empty() {
            trial.diagnostics.push(diagnostic);
        }
        Ok(())
    }

    pub fn retry_trial(
        &mut self,
        trial_id: &str,
        source_attempt_id: &str,
    ) -> Result<(), PythonResearchError> {
        let trial = self
            .trials
            .iter_mut()
            .find(|trial| trial.trial_id == trial_id)
            .ok_or_else(|| invalid("model-trial-not-found"))?;
        if source_attempt_id.trim().is_empty()
            || trial.candidate_artifact_sha256.is_some()
            || (!matches!(
                trial.status,
                TrialStatus::Failed
                    | TrialStatus::Cancelled
                    | TrialStatus::Interrupted
                    | TrialStatus::Stale
            ) && !(trial.status == TrialStatus::Registered
                && trial.attempt_ids.iter().any(|id| id == source_attempt_id)))
        {
            return Err(invalid("model-trial-retry-invalid"));
        }
        if !trial.attempt_ids.iter().any(|id| id == source_attempt_id) {
            trial.attempt_ids.push(source_attempt_id.into());
        }
        trial.status = TrialStatus::Registered;
        trial.selection_metric = None;
        trial.successful_attempt_id = None;
        trial.candidate_artifact_sha256 = None;
        trial.repeatability_state = RepeatabilityState::Unverified;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ParameterSelectionDecision {
    pub decision_id: String,
    pub experiment_id: String,
    pub selected_trial_id: String,
    pub selected_alpha: f64,
    pub selection_metrics_sha256: String,
    pub candidate_artifact_sha256: String,
    pub evidence_state: EvidenceState,
    #[serde(default)]
    pub binding_sha256: String,
    #[serde(default)]
    pub project_revision_sha256: String,
    #[serde(default)]
    pub environment_sha256: String,
    #[serde(default)]
    pub input_evidence_sha256: String,
    #[serde(default)]
    pub seed: u64,
}

impl ParameterSelectionDecision {
    pub fn record(
        experiment: &ModelExperiment,
        selected_trial_id: &str,
    ) -> Result<Self, PythonResearchError> {
        experiment.validate()?;
        let trial = experiment
            .trials
            .iter()
            .find(|trial| trial.trial_id == selected_trial_id)
            .ok_or_else(|| invalid("model-selection-trial-not-found"))?;
        if trial.status != TrialStatus::Completed
            || trial.evidence_state != experiment.lineage_evidence_state
            || trial.repeatability_state != RepeatabilityState::Verified
            || trial.candidate_artifact_sha256.is_none()
        {
            return Err(invalid(
                "model-selection-requires-completed-selection-trial",
            ));
        }
        if experiment.trials.iter().any(|trial| {
            trial.status != TrialStatus::Completed
                || trial.selection_metric.is_none()
                || trial.evidence_state != experiment.lineage_evidence_state
                || trial.repeatability_state != RepeatabilityState::Verified
                || trial.candidate_artifact_sha256.is_none()
        }) {
            return Err(invalid("model-selection-requires-complete-selection-grid"));
        }
        let metrics = experiment
            .trials
            .iter()
            .map(|trial| (trial.trial_id.clone(), trial.selection_metric))
            .collect::<Vec<_>>();
        let selection_metrics_sha256 =
            sha256(&serde_json::to_vec(&metrics).map_err(|error| invalid(error.to_string()))?);
        let candidate_artifact_sha256 = trial
            .candidate_artifact_sha256
            .clone()
            .ok_or_else(|| invalid("model-selection-candidate-artifact-missing"))?;
        let decision_id = sha256(
            format!(
                "{}:{selected_trial_id}:{selection_metrics_sha256}:{candidate_artifact_sha256}",
                experiment.experiment_id,
            )
            .as_bytes(),
        );
        Ok(Self {
            decision_id,
            experiment_id: experiment.experiment_id.clone(),
            selected_trial_id: selected_trial_id.into(),
            selected_alpha: trial.alpha,
            selection_metrics_sha256,
            candidate_artifact_sha256,
            evidence_state: experiment.lineage_evidence_state,
            binding_sha256: experiment.binding_sha256.clone(),
            project_revision_sha256: experiment.project_revision_sha256.clone(),
            environment_sha256: experiment.environment_sha256.clone(),
            input_evidence_sha256: experiment.input_evidence_sha256.clone(),
            seed: experiment.seed,
        })
    }

    pub fn validate(&self) -> Result<(), PythonResearchError> {
        let expected_decision_id = sha256(
            format!(
                "{}:{}:{}:{}",
                self.experiment_id,
                self.selected_trial_id,
                self.selection_metrics_sha256,
                self.candidate_artifact_sha256,
            )
            .as_bytes(),
        );
        if !is_sha256(&self.decision_id)
            || self.decision_id != expected_decision_id
            || !is_sha256(&self.experiment_id)
            || !is_sha256(&self.selected_trial_id)
            || !self.selected_alpha.is_finite()
            || self.selected_alpha <= 0.0
            || !is_sha256(&self.selection_metrics_sha256)
            || !is_sha256(&self.candidate_artifact_sha256)
            || !matches!(
                self.evidence_state,
                EvidenceState::Unknown | EvidenceState::Overlapping
            )
            || !is_sha256(&self.binding_sha256)
            || !is_sha256(&self.project_revision_sha256)
            || !is_sha256(&self.environment_sha256)
            || !is_sha256(&self.input_evidence_sha256)
        {
            return Err(invalid("model-selection-decision-invalid"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FinalEvaluationReport {
    pub report_id: String,
    pub decision_id: String,
    pub forecast_sha256: String,
    pub target_sha256: String,
    pub mean_squared_error: f64,
    pub mean_absolute_error: f64,
    pub evidence_state: EvidenceState,
    #[serde(default)]
    pub artifact_sha256: String,
    #[serde(default)]
    pub forecast_dataset_sha256: String,
}

impl FinalEvaluationReport {
    pub fn validate(&self) -> Result<(), PythonResearchError> {
        let expected_report_id = sha256(
            format!(
                "{}:{}:{}:{}:{}",
                self.decision_id,
                self.forecast_sha256,
                self.target_sha256,
                self.artifact_sha256,
                self.forecast_dataset_sha256
            )
            .as_bytes(),
        );
        if !is_sha256(&self.report_id)
            || self.report_id != expected_report_id
            || !is_sha256(&self.decision_id)
            || !is_sha256(&self.forecast_sha256)
            || !is_sha256(&self.target_sha256)
            || !self.mean_squared_error.is_finite()
            || !self.mean_absolute_error.is_finite()
            || !matches!(
                self.evidence_state,
                EvidenceState::OutOfSample | EvidenceState::Overlapping
            )
            || !is_sha256(&self.artifact_sha256)
            || !is_sha256(&self.forecast_dataset_sha256)
        {
            return Err(invalid("model-final-evaluation-invalid"));
        }
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct FinalEvaluationLedger {
    reports: BTreeMap<String, FinalEvaluationReport>,
}

impl FinalEvaluationLedger {
    #[deprecated(note = "pass artifact and forecast dataset identities to run_with_evidence")]
    pub fn run(
        &mut self,
        decision: &ParameterSelectionDecision,
        forecasts: &[ForecastRow],
        labels: &[(i64, String, f64)],
    ) -> Result<FinalEvaluationReport, PythonResearchError> {
        let _ = (decision, forecasts, labels);
        Err(invalid("model-final-evidence-required"))
    }

    pub fn run_with_evidence(
        &mut self,
        decision: &ParameterSelectionDecision,
        forecasts: &[ForecastRow],
        labels: &[(i64, String, f64)],
        artifact_sha256: impl Into<String>,
        forecast_dataset_sha256: impl Into<String>,
        evidence_state: EvidenceState,
    ) -> Result<FinalEvaluationReport, PythonResearchError> {
        decision.validate()?;
        let provided_artifact_sha256 = artifact_sha256.into();
        if decision.candidate_artifact_sha256 != provided_artifact_sha256 {
            return Err(invalid("model-final-artifact-binding-invalid"));
        }
        if !matches!(
            evidence_state,
            EvidenceState::OutOfSample | EvidenceState::Overlapping
        ) || (decision.evidence_state == EvidenceState::Overlapping
            && evidence_state != EvidenceState::Overlapping)
            || (decision.evidence_state == EvidenceState::Unknown
                && evidence_state != EvidenceState::OutOfSample)
        {
            return Err(invalid("model-final-evidence-state-invalid"));
        }
        if self.reports.contains_key(&decision.decision_id) {
            return Err(invalid("model-final-evaluation-already-recorded"));
        }
        if forecasts.is_empty()
            || forecasts.len() != labels.len()
            || forecasts.windows(2).any(|rows| {
                (rows[0].datetime, rows[0].instrument.as_str())
                    >= (rows[1].datetime, rows[1].instrument.as_str())
            })
        {
            return Err(invalid("model-final-evaluation-identity-invalid"));
        }
        let mut squared = 0.0;
        let mut absolute = 0.0;
        for (forecast, (datetime, instrument, label)) in forecasts.iter().zip(labels) {
            let value = forecast
                .value
                .ok_or_else(|| invalid("model-final-forecast-unavailable"))?;
            if forecast.datetime != *datetime
                || forecast.instrument != *instrument
                || !label.is_finite()
            {
                return Err(invalid("model-final-label-identity-invalid"));
            }
            let error = value - label;
            squared += error * error;
            absolute += error.abs();
        }
        let count = forecasts.len().max(1) as f64;
        let forecast_sha256 =
            sha256(&serde_json::to_vec(forecasts).map_err(|error| invalid(error.to_string()))?);
        let target_sha256 =
            sha256(&serde_json::to_vec(labels).map_err(|error| invalid(error.to_string()))?);
        let artifact_sha256 = decision.candidate_artifact_sha256.clone();
        let forecast_dataset_sha256 = forecast_dataset_sha256.into();
        let report_id = sha256(
            format!(
                "{}:{forecast_sha256}:{target_sha256}:{}:{}",
                decision.decision_id, artifact_sha256, forecast_dataset_sha256
            )
            .as_bytes(),
        );
        let report = FinalEvaluationReport {
            report_id,
            decision_id: decision.decision_id.clone(),
            forecast_sha256,
            target_sha256,
            mean_squared_error: squared / count,
            mean_absolute_error: absolute / count,
            evidence_state,
            artifact_sha256,
            forecast_dataset_sha256,
        };
        report.validate()?;
        self.reports
            .insert(decision.decision_id.clone(), report.clone());
        Ok(report)
    }
}

pub fn derived_lineage_state(parent: EvidenceState, final_feedback_used: bool) -> EvidenceState {
    if final_feedback_used {
        EvidenceState::Overlapping
    } else {
        parent
    }
}

pub fn compare_repeatability(
    coefficients: &[f64],
    replay_coefficients: &[f64],
    forecasts: &[ForecastRow],
    replay_forecasts: &[ForecastRow],
) -> Result<(), PythonResearchError> {
    if coefficients.len() != replay_coefficients.len()
        || forecasts.len() != replay_forecasts.len()
        || coefficients
            .iter()
            .zip(replay_coefficients)
            .any(|(left, right)| {
                !left.is_finite()
                    || !right.is_finite()
                    || (left - right).abs() > RIDGE_REPEATABILITY_TOLERANCE
            })
        || forecasts.iter().zip(replay_forecasts).any(|(left, right)| {
            left.datetime != right.datetime
                || left.instrument != right.instrument
                || left.value.is_some() != right.value.is_some()
                || left.unavailable_reason != right.unavailable_reason
                || left.value.is_some_and(|value| !value.is_finite())
                || right.value.is_some_and(|value| !value.is_finite())
                || left
                    .value
                    .zip(right.value)
                    .is_some_and(|(a, b)| (a - b).abs() > RIDGE_REPEATABILITY_TOLERANCE)
        })
    {
        return Err(invalid("model-repeatability-divergent"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(value: &str) -> String {
        sha256(value.as_bytes())
    }

    #[test]
    fn host_grid_retains_three_trials_and_requires_user_decision() {
        let mut experiment =
            ModelExperiment::ridge(hash("revision"), hash("environment"), hash("input"), 7)
                .unwrap();
        assert_eq!(experiment.trials.len(), 3);
        for trial in experiment.trials.clone() {
            experiment
                .complete_trial_with_candidate(
                    &trial.trial_id,
                    hash(&format!("attempt:{}", trial.alpha)),
                    trial.alpha,
                    hash(&format!("artifact:{}", trial.alpha)),
                )
                .unwrap();
        }
        let decision =
            ParameterSelectionDecision::record(&experiment, &experiment.trials[1].trial_id)
                .unwrap();
        assert_eq!(decision.selected_alpha, 1.0);
        assert!(ParameterSelectionDecision::record(&experiment, "missing").is_err());
    }

    #[test]
    fn repeatability_verified_trial_requires_candidate_identity() {
        let mut experiment =
            ModelExperiment::ridge(hash("revision"), hash("environment"), hash("input"), 7)
                .unwrap();
        let trial_id = experiment.trials[0].trial_id.clone();
        assert_eq!(
            experiment
                .complete_trial_with_repeatability(
                    &trial_id,
                    hash("attempt"),
                    1.0,
                    RepeatabilityState::Verified,
                )
                .unwrap_err()
                .to_string(),
            "model-trial-candidate-required"
        );
        assert_eq!(experiment.trials[0].status, TrialStatus::Registered);
    }

    #[test]
    fn candidate_identity_is_directly_bound_to_trial_and_selection() {
        let mut experiment =
            ModelExperiment::ridge(hash("revision"), hash("environment"), hash("input"), 7)
                .unwrap();
        for trial in experiment.trials.clone() {
            experiment
                .complete_trial_with_candidate(
                    &trial.trial_id,
                    hash(&format!("attempt:{}", trial.alpha)),
                    trial.alpha,
                    hash(&format!("artifact:{}", trial.alpha)),
                )
                .unwrap();
        }
        let selected = &experiment.trials[1];
        assert_eq!(
            selected.successful_attempt_id.as_deref(),
            selected.attempt_ids.last().map(String::as_str)
        );
        let decision = ParameterSelectionDecision::record(&experiment, &selected.trial_id).unwrap();
        assert_eq!(
            decision.candidate_artifact_sha256,
            selected.candidate_artifact_sha256.clone().unwrap()
        );
        assert!(decision.validate().is_ok());
    }

    #[test]
    fn failed_trials_remain_in_the_experiment() {
        let mut experiment =
            ModelExperiment::ridge(hash("revision"), hash("environment"), hash("input"), 7)
                .unwrap();
        let trial_id = experiment.trials[0].trial_id.clone();
        experiment
            .fail_trial(&trial_id, hash("failed-attempt"), TrialStatus::Failed)
            .unwrap();
        assert_eq!(experiment.trials[0].status, TrialStatus::Failed);
        assert_eq!(experiment.trials[0].attempt_ids.len(), 1);
        assert!(ParameterSelectionDecision::record(&experiment, &trial_id).is_err());
    }

    #[test]
    fn retry_reopens_one_failed_trial_without_dropping_attempt_history() {
        let mut experiment =
            ModelExperiment::ridge(hash("revision"), hash("environment"), hash("input"), 7)
                .unwrap();
        let trial_id = experiment.trials[0].trial_id.clone();
        let source_attempt_id = hash("failed-attempt");
        experiment
            .fail_trial_with_diagnostic(
                &trial_id,
                source_attempt_id.clone(),
                TrialStatus::Interrupted,
                "runner stopped",
            )
            .unwrap();
        experiment
            .retry_trial(&trial_id, &source_attempt_id)
            .unwrap();
        let trial = &experiment.trials[0];
        assert_eq!(trial.status, TrialStatus::Registered);
        assert_eq!(trial.attempt_ids, vec![source_attempt_id]);
        assert!(trial.successful_attempt_id.is_none());
        assert!(trial.candidate_artifact_sha256.is_none());
        assert!(trial.selection_metric.is_none());
    }

    #[test]
    fn final_evaluation_is_host_only_and_one_shot() {
        let mut experiment =
            ModelExperiment::ridge(hash("revision"), hash("environment"), hash("input"), 7)
                .unwrap();
        for trial in experiment.trials.clone() {
            experiment
                .complete_trial_with_candidate(
                    &trial.trial_id,
                    hash(&format!("attempt:{}", trial.alpha)),
                    1.0,
                    hash(&format!("artifact:{}", trial.alpha)),
                )
                .unwrap();
        }
        let decision =
            ParameterSelectionDecision::record(&experiment, &experiment.trials[1].trial_id)
                .unwrap();
        let forecasts = vec![ForecastRow {
            datetime: 1,
            instrument: "AAA".into(),
            value: Some(2.0),
            unavailable_reason: None,
        }];
        let labels = vec![(1, "AAA".into(), 3.0)];
        assert_eq!(
            FinalEvaluationLedger::default()
                .run_with_evidence(
                    &decision,
                    &forecasts,
                    &labels,
                    hash("foreign-artifact"),
                    hash("forecast-dataset"),
                    EvidenceState::OutOfSample,
                )
                .unwrap_err()
                .to_string(),
            "model-final-artifact-binding-invalid"
        );
        let mut ledger = FinalEvaluationLedger::default();
        let artifact = experiment.trials[1]
            .candidate_artifact_sha256
            .clone()
            .unwrap();
        let forecast_dataset = hash("forecast-dataset");
        let report = ledger
            .run_with_evidence(
                &decision,
                &forecasts,
                &labels,
                artifact.clone(),
                forecast_dataset.clone(),
                EvidenceState::OutOfSample,
            )
            .unwrap();
        assert_eq!(report.mean_absolute_error, 1.0);
        assert_eq!(report.artifact_sha256, artifact);
        assert_eq!(report.forecast_dataset_sha256, forecast_dataset);
        assert!(
            ledger
                .run_with_evidence(
                    &decision,
                    &forecasts,
                    &labels,
                    artifact,
                    forecast_dataset,
                    EvidenceState::OutOfSample,
                )
                .is_err()
        );
    }

    #[test]
    fn overlap_and_repeatability_are_explicit() {
        assert_eq!(
            derived_lineage_state(EvidenceState::OutOfSample, true),
            EvidenceState::Overlapping
        );
        let forecast = ForecastRow {
            datetime: 1,
            instrument: "AAA".into(),
            value: Some(1.0),
            unavailable_reason: None,
        };
        assert!(
            compare_repeatability(&[1.0], &[1.0 + 1e-10], &[forecast.clone()], &[forecast]).is_ok()
        );
        assert!(compare_repeatability(&[1.0], &[1.1], &[], &[]).is_err());
    }

    #[test]
    fn exact_binding_is_shared_by_every_trial_and_decision() {
        let binding = hash("binding");
        let mut experiment = ModelExperiment::ridge_with_binding(
            hash("revision"),
            hash("environment"),
            hash("input"),
            7,
            binding.clone(),
        )
        .unwrap();
        assert!(experiment.trials.iter().all(|trial| {
            trial.binding_sha256 == binding
                && trial.project_revision_sha256 == experiment.project_revision_sha256
        }));
        for trial in experiment.trials.clone() {
            experiment
                .complete_trial_with_candidate(
                    &trial.trial_id,
                    hash(&format!("attempt:{}", trial.alpha)),
                    1.0,
                    hash(&format!("artifact:{}", trial.alpha)),
                )
                .unwrap();
        }
        let decision =
            ParameterSelectionDecision::record(&experiment, &experiment.trials[1].trial_id)
                .unwrap();
        assert_eq!(decision.binding_sha256, binding);
        assert_eq!(
            decision.project_revision_sha256,
            experiment.project_revision_sha256
        );
        assert_eq!(decision.seed, experiment.seed);
    }

    #[test]
    fn terminal_trial_states_are_retained_with_diagnostics() {
        for (index, status) in [
            TrialStatus::Failed,
            TrialStatus::Cancelled,
            TrialStatus::Invalid,
            TrialStatus::Unsupported,
            TrialStatus::Superseded,
        ]
        .into_iter()
        .enumerate()
        {
            let mut experiment =
                ModelExperiment::ridge(hash("revision"), hash("environment"), hash("input"), 7)
                    .unwrap();
            let trial_id = experiment.trials[0].trial_id.clone();
            experiment
                .fail_trial_with_diagnostic(
                    &trial_id,
                    hash(&format!("attempt-{index}")),
                    status,
                    "retained diagnostic",
                )
                .unwrap();
            assert_eq!(experiment.trials[0].status, status);
            assert_eq!(
                experiment.trials[0].diagnostics,
                vec!["retained diagnostic".to_string()]
            );
        }
    }

    #[test]
    fn divergent_trial_is_retained_but_cannot_be_selected() {
        let mut experiment =
            ModelExperiment::ridge(hash("revision"), hash("environment"), hash("input"), 7)
                .unwrap();
        for (index, trial) in experiment.trials.clone().into_iter().enumerate() {
            if index == 0 {
                experiment
                    .complete_trial_with_repeatability(
                        &trial.trial_id,
                        hash(&trial.trial_id),
                        index as f64,
                        RepeatabilityState::Divergent,
                    )
                    .unwrap();
            } else {
                experiment
                    .complete_trial_with_candidate(
                        &trial.trial_id,
                        hash(&format!("attempt:{}", trial.alpha)),
                        index as f64,
                        hash(&format!("artifact:{}", trial.alpha)),
                    )
                    .unwrap();
            }
        }
        assert_eq!(
            experiment.trials[0].repeatability_state,
            RepeatabilityState::Divergent
        );
        assert!(
            ParameterSelectionDecision::record(&experiment, &experiment.trials[1].trial_id)
                .is_err()
        );
    }

    #[test]
    fn repeatability_rejects_non_finite_forecasts() {
        let forecast = ForecastRow {
            datetime: 1,
            instrument: "AAA".into(),
            value: Some(f64::NAN),
            unavailable_reason: None,
        };
        assert!(compare_repeatability(&[], &[], &[forecast.clone()], &[forecast]).is_err());
    }

    #[test]
    fn derived_experiment_preserves_overlapping_lineage_and_linked_final_evidence() {
        let revision = hash("revision");
        let environment = hash("environment");
        let input = hash("input");
        let binding = hash("binding");
        let parent_decision = hash("parent-decision");
        let experiment = ModelExperiment::ridge_with_binding_and_lineage(
            revision,
            environment,
            input,
            7,
            binding,
            Some(parent_decision),
        )
        .unwrap();
        assert_eq!(
            experiment.lineage_evidence_state,
            EvidenceState::Overlapping
        );
        assert!(
            experiment
                .trials
                .iter()
                .all(|trial| trial.evidence_state == EvidenceState::Overlapping)
        );
        let mut experiment = experiment;
        for trial in experiment.trials.clone() {
            experiment
                .complete_trial_with_candidate(
                    &trial.trial_id,
                    hash(&format!("attempt:{}", trial.alpha)),
                    1.0,
                    hash(&format!("artifact:{}", trial.alpha)),
                )
                .unwrap();
        }
        let decision =
            ParameterSelectionDecision::record(&experiment, &experiment.trials[1].trial_id)
                .unwrap();
        assert_eq!(decision.evidence_state, EvidenceState::Overlapping);
        decision.validate().unwrap();

        let forecasts = vec![ForecastRow {
            datetime: 1,
            instrument: "AAA".into(),
            value: Some(2.0),
            unavailable_reason: None,
        }];
        let labels = vec![(1, "AAA".into(), 3.0)];
        let mut ledger = FinalEvaluationLedger::default();
        let report = ledger
            .run_with_evidence(
                &decision,
                &forecasts,
                &labels,
                decision.candidate_artifact_sha256.clone(),
                hash("forecast-dataset"),
                EvidenceState::Overlapping,
            )
            .unwrap();
        assert_eq!(report.evidence_state, EvidenceState::Overlapping);
        report.validate().unwrap();
    }
}
