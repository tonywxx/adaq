//! Host-owned Ridge Grid, User Selection, Final Evaluation, and replay gates.

use crate::{PythonResearchError, invalid, is_sha256, model::ForecastRow, sha256};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const RIDGE_ALPHAS: [f64; 3] = [0.1, 1.0, 10.0];
pub const RIDGE_REPEATABILITY_TOLERANCE: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TrialStatus {
    Registered,
    Completed,
    Failed,
    Cancelled,
    Invalid,
    Superseded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EvidenceState {
    OutOfSample,
    Overlapping,
    Unknown,
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
}

impl ModelTrial {
    pub fn validate(&self) -> Result<(), PythonResearchError> {
        if !is_sha256(&self.trial_id)
            || self.experiment_id.is_empty()
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
        if !is_sha256(&project_revision_sha256)
            || !is_sha256(&environment_sha256)
            || !is_sha256(&input_evidence_sha256)
        {
            return Err(invalid("model-experiment-identity-invalid"));
        }
        let experiment_id = sha256(
            format!(
                "{project_revision_sha256}:{environment_sha256}:{input_evidence_sha256}:{seed}"
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
                evidence_state: EvidenceState::Unknown,
            })
            .collect::<Vec<_>>();
        Ok(Self {
            experiment_id,
            project_revision_sha256,
            environment_sha256,
            input_evidence_sha256,
            seed,
            trials,
        })
    }

    pub fn validate(&self) -> Result<(), PythonResearchError> {
        let mut trial_ids = BTreeMap::new();
        let mut alphas = BTreeMap::new();
        if !is_sha256(&self.experiment_id)
            || !is_sha256(&self.project_revision_sha256)
            || !is_sha256(&self.environment_sha256)
            || !is_sha256(&self.input_evidence_sha256)
            || self.trials.len() != RIDGE_ALPHAS.len()
            || self.trials.iter().any(|trial| {
                trial.validate().is_err()
                    || trial.experiment_id != self.experiment_id
                    || trial.project_revision_sha256 != self.project_revision_sha256
                    || trial.environment_sha256 != self.environment_sha256
                    || trial.input_evidence_sha256 != self.input_evidence_sha256
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
        let attempt_id = attempt_id.into();
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
        trial.evidence_state = EvidenceState::Unknown;
        Ok(())
    }

    pub fn fail_trial(
        &mut self,
        trial_id: &str,
        attempt_id: impl Into<String>,
        status: TrialStatus,
    ) -> Result<(), PythonResearchError> {
        let attempt_id = attempt_id.into();
        let trial = self
            .trials
            .iter_mut()
            .find(|trial| trial.trial_id == trial_id)
            .ok_or_else(|| invalid("model-trial-not-found"))?;
        if !matches!(
            status,
            TrialStatus::Failed | TrialStatus::Cancelled | TrialStatus::Invalid
        ) || trial.status != TrialStatus::Registered
            || attempt_id.trim().is_empty()
            || trial.attempt_ids.iter().any(|id| id == &attempt_id)
        {
            return Err(invalid("model-trial-failure-invalid"));
        }
        trial.attempt_ids.push(attempt_id);
        trial.status = status;
        trial.evidence_state = EvidenceState::Unknown;
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
    pub evidence_state: EvidenceState,
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
        if trial.status != TrialStatus::Completed || trial.evidence_state != EvidenceState::Unknown
        {
            return Err(invalid(
                "model-selection-requires-completed-selection-trial",
            ));
        }
        if experiment.trials.iter().any(|trial| {
            trial.status != TrialStatus::Completed
                || trial.selection_metric.is_none()
                || trial.evidence_state != EvidenceState::Unknown
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
        let decision_id = sha256(
            format!(
                "{}:{selected_trial_id}:{selection_metrics_sha256}",
                experiment.experiment_id
            )
            .as_bytes(),
        );
        Ok(Self {
            decision_id,
            experiment_id: experiment.experiment_id.clone(),
            selected_trial_id: selected_trial_id.into(),
            selected_alpha: trial.alpha,
            selection_metrics_sha256,
            evidence_state: EvidenceState::Unknown,
        })
    }

    pub fn validate(&self) -> Result<(), PythonResearchError> {
        if !is_sha256(&self.decision_id)
            || !is_sha256(&self.experiment_id)
            || !is_sha256(&self.selected_trial_id)
            || !self.selected_alpha.is_finite()
            || self.selected_alpha <= 0.0
            || !is_sha256(&self.selection_metrics_sha256)
            || self.evidence_state != EvidenceState::Unknown
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
}

impl FinalEvaluationReport {
    pub fn validate(&self) -> Result<(), PythonResearchError> {
        if !is_sha256(&self.report_id)
            || !is_sha256(&self.decision_id)
            || !is_sha256(&self.forecast_sha256)
            || !is_sha256(&self.target_sha256)
            || !self.mean_squared_error.is_finite()
            || !self.mean_absolute_error.is_finite()
            || self.evidence_state != EvidenceState::OutOfSample
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
    pub fn run(
        &mut self,
        decision: &ParameterSelectionDecision,
        forecasts: &[ForecastRow],
        labels: &[(i64, String, f64)],
    ) -> Result<FinalEvaluationReport, PythonResearchError> {
        decision.validate()?;
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
        let report_id = sha256(
            format!("{}:{forecast_sha256}:{target_sha256}", decision.decision_id).as_bytes(),
        );
        let report = FinalEvaluationReport {
            report_id,
            decision_id: decision.decision_id.clone(),
            forecast_sha256,
            target_sha256,
            mean_squared_error: squared / count,
            mean_absolute_error: absolute / count,
            evidence_state: EvidenceState::OutOfSample,
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
                .complete_trial(&trial.trial_id, hash(&trial.trial_id), trial.alpha)
                .unwrap();
        }
        let decision =
            ParameterSelectionDecision::record(&experiment, &experiment.trials[1].trial_id)
                .unwrap();
        assert_eq!(decision.selected_alpha, 1.0);
        assert!(ParameterSelectionDecision::record(&experiment, "missing").is_err());
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
    fn final_evaluation_is_host_only_and_one_shot() {
        let mut experiment =
            ModelExperiment::ridge(hash("revision"), hash("environment"), hash("input"), 7)
                .unwrap();
        for trial in experiment.trials.clone() {
            experiment
                .complete_trial(&trial.trial_id, hash(&trial.trial_id), 1.0)
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
        let mut ledger = FinalEvaluationLedger::default();
        let report = ledger.run(&decision, &forecasts, &labels).unwrap();
        assert_eq!(report.mean_absolute_error, 1.0);
        assert!(ledger.run(&decision, &forecasts, &labels).is_err());
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
}
