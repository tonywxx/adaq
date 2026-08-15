//! The first qualified Python Model path: Host-fed Qlib-shaped Ridge.
//!
//! This is intentionally a small read-only bridge. It does not import Qlib,
//! open a Provider, read a data directory, or deserialize Python objects.

use crate::{PythonResearchError, invalid, is_sha256, sha256};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const QLIB_BRIDGE_VERSION: &str = "adaq.qlib@1";
pub const RIDGE_ADAPTER_ID: &str = "qlib-linear-ridge@1";
pub const LINEAR_MODEL_ARTIFACT_SCHEMA: &str = "adaq:linear-model@1";
pub const MODEL_PROJECT_ID: &str = "py-model-qlib-ridge-return";
pub const TARGET_ID: &str = "future-close-return";
pub const TARGET_HORIZON_BARS: usize = 5;
pub const NUMERIC_REPRESENTATION: &str = "ieee754-binary64";
pub const FORECAST_CONTRACT: &str = "forecast:continuous-future-close-return:native@1";
pub const MAX_MODEL_INPUT_SLOTS: usize = 64;
pub const MAX_MODEL_PROVENANCE_ENTRIES: usize = 32;
pub const MAX_MODEL_ARTIFACT_BYTES: usize = 128 * 1024 * 1024;

pub fn validate_model_manifest(
    manifest: &crate::ProjectManifest,
) -> Result<(), PythonResearchError> {
    if manifest.kind != crate::ProjectKind::Model
        || manifest.project_id != MODEL_PROJECT_ID
        || manifest.scope != crate::ProjectScope::CrossSectional
        || manifest.license != "Apache-2.0"
        || manifest.adapter_id.as_deref() != Some(RIDGE_ADAPTER_ID)
        || manifest.parameters
            != vec![crate::ParameterSpec {
                id: "alpha".into(),
                value_type: crate::ParameterType::Decimal,
                default: "1".into(),
                allowed_values: vec!["0.1".into(), "1".into(), "10".into()],
            }]
        || manifest.input_slots
            != vec![crate::InputSlotSpec {
                id: "promoted-factor".into(),
                role: "promoted-factor".into(),
                scope: crate::ProjectScope::CrossSectional,
                required: true,
            }]
        || manifest.outputs
            != vec![crate::OutputSpec {
                id: "forecast".into(),
                value_type: "finite-f64".into(),
                required: true,
            }]
        || manifest.target
            != Some(crate::TargetSpec {
                id: TARGET_ID.into(),
                kind: "continuous-future-close-return".into(),
                horizon_bars: TARGET_HORIZON_BARS as u32,
                value_scale: "return".into(),
            })
        || manifest.signal
            != Some(crate::SignalSpec {
                id: "forecast".into(),
                kind: "forecast".into(),
                value_scale: "native".into(),
            })
    {
        return Err(invalid("python-model-manifest-contract-invalid"));
    }
    Ok(())
}

pub fn validate_model_project_payload(payload: &Value) -> Result<(), PythonResearchError> {
    #[derive(Debug, Deserialize, PartialEq, Eq)]
    #[serde(deny_unknown_fields)]
    struct TargetContract {
        id: String,
        kind: String,
        horizon_bars: u32,
        value_scale: String,
    }

    #[derive(Debug, Deserialize, PartialEq, Eq)]
    #[serde(deny_unknown_fields)]
    struct SignalContract {
        id: String,
        kind: String,
        value_scale: String,
    }

    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ModelPayload {
        target: Option<TargetContract>,
        signal: Option<SignalContract>,
        #[serde(default)]
        fit: Option<Value>,
        #[serde(default)]
        forecast: Option<Value>,
    }

    let value = serde_json::from_value::<ModelPayload>(payload.clone())
        .map_err(|error| invalid(format!("python-model-contract-invalid:{error}")))?;
    let _ = value.fit;
    let _ = value.forecast;
    if value.target
        != Some(TargetContract {
            id: TARGET_ID.into(),
            kind: "continuous-future-close-return".into(),
            horizon_bars: TARGET_HORIZON_BARS as u32,
            value_scale: "return".into(),
        })
        || value.signal
            != Some(SignalContract {
                id: "forecast".into(),
                kind: "forecast".into(),
                value_scale: "native".into(),
            })
    {
        return Err(invalid("python-model-contract-invalid"));
    }
    Ok(())
}

pub fn validate_model_runner_payload(
    payload: &Value,
    feature_names: &[String],
) -> Result<(), PythonResearchError> {
    let fit = payload.get("fit");
    let forecast = payload.get("forecast");
    if fit.is_none() == forecast.is_none() {
        return Err(invalid("python-model-runner-phase-invalid"));
    }
    if let Some(fit) = fit {
        #[derive(Debug, Deserialize)]
        #[serde(deny_unknown_fields)]
        struct CandidatePayload {
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
        #[derive(Debug, Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Candidate {
            schema: String,
            payload: CandidatePayload,
        }
        let candidate = serde_json::from_value::<Candidate>(fit.clone())
            .map_err(|error| invalid(format!("python-model-fit-invalid:{error}")))?;
        if candidate.schema != "adaq:linear-model:candidate@1"
            || candidate.payload.adapter_id != RIDGE_ADAPTER_ID
            || candidate.payload.artifact_schema != LINEAR_MODEL_ARTIFACT_SCHEMA
            || candidate.payload.numeric_representation != NUMERIC_REPRESENTATION
            || candidate.payload.forecast_contract != FORECAST_CONTRACT
            || candidate.payload.input_slots != feature_names
            || candidate.payload.coefficients.len() != feature_names.len()
            || !candidate.payload.alpha.is_finite()
            || candidate.payload.alpha <= 0.0
            || candidate
                .payload
                .coefficients
                .iter()
                .any(|value| !value.is_finite())
            || !candidate.payload.intercept.is_finite()
            || !is_sha256(&candidate.payload.transformation_sha256)
        {
            return Err(invalid("python-model-fit-contract-invalid"));
        }
    }
    if let Some(forecast) = forecast {
        let rows = forecast
            .as_array()
            .ok_or_else(|| invalid("python-model-forecast-invalid"))?;
        let mut identities = BTreeSet::new();
        for row in rows {
            let object = row
                .as_object()
                .ok_or_else(|| invalid("python-model-forecast-invalid"))?;
            let datetime = object
                .get("prediction_time_ms")
                .and_then(Value::as_i64)
                .ok_or_else(|| invalid("python-model-forecast-identity-invalid"))?;
            let instrument = object
                .get("instrument_id")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| invalid("python-model-forecast-identity-invalid"))?;
            if !identities.insert((datetime, instrument.to_owned())) {
                return Err(invalid("python-model-forecast-identity-invalid"));
            }
            let value = object
                .get("value")
                .ok_or_else(|| invalid("python-model-forecast-value-invalid"))?;
            if value.as_f64().is_some_and(|value| !value.is_finite())
                || value
                    .as_object()
                    .is_some_and(|object| object.get("reason").and_then(Value::as_str).is_none())
                || (!value.is_number() && !value.is_object())
            {
                return Err(invalid("python-model-forecast-value-invalid"));
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PartitionName {
    Train,
    SelectionValidation,
    Test,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostPartitionRow {
    pub datetime: i64,
    pub instrument: String,
    pub features: Vec<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostPartition {
    pub name: PartitionName,
    pub feature_names: Vec<String>,
    pub rows: Vec<HostPartitionRow>,
    pub labels_visible: bool,
}

impl HostPartition {
    pub fn validate(&self) -> Result<(), PythonResearchError> {
        if self.feature_names.is_empty()
            || self.feature_names.iter().any(|name| name.trim().is_empty())
            || self.labels_visible != !matches!(self.name, PartitionName::Test)
            || self.rows.iter().any(|row| {
                row.instrument.trim().is_empty()
                    || row.features.len() != self.feature_names.len()
                    || row.features.iter().any(|value| !value.is_finite())
                    || (self.labels_visible && row.label.is_some_and(|value| !value.is_finite()))
                    || (!self.labels_visible && row.label.is_some())
            })
        {
            return Err(invalid("qlib-host-partition-invalid"));
        }
        if self.rows.windows(2).any(|rows| {
            (rows[0].datetime, rows[0].instrument.as_str())
                >= (rows[1].datetime, rows[1].instrument.as_str())
        }) {
            return Err(invalid("qlib-host-partition-order-invalid"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct QlibView {
    pub name: PartitionName,
    pub feature_names: Vec<String>,
    pub rows: Vec<HostPartitionRow>,
    pub labels: Option<Vec<f64>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TargetLabel {
    Value(f64),
    Unavailable { reason: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct DatasetH {
    partitions: BTreeMap<String, HostPartition>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelRunnerInput {
    pub train: Vec<HostPartitionRow>,
    pub valid: Vec<HostPartitionRow>,
    pub test: Vec<HostPartitionRow>,
    pub train_labels: Vec<TargetLabel>,
    pub valid_labels: Vec<TargetLabel>,
    pub transformation: FittedTransformation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fitted_model: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_window_end: Option<u32>,
}

impl ModelRunnerInput {
    pub fn validate(&self) -> Result<(), PythonResearchError> {
        let dataset = DatasetH::new(vec![
            HostPartition {
                name: PartitionName::Train,
                feature_names: self.transformation.feature_names.clone(),
                rows: self.train.clone(),
                labels_visible: true,
            },
            HostPartition {
                name: PartitionName::SelectionValidation,
                feature_names: self.transformation.feature_names.clone(),
                rows: self.valid.clone(),
                labels_visible: true,
            },
            HostPartition {
                name: PartitionName::Test,
                feature_names: self.transformation.feature_names.clone(),
                rows: self.test.clone(),
                labels_visible: false,
            },
        ])?;
        self.transformation.validate()?;
        let train_rows = dataset.raw_rows("train")?;
        let valid_rows = dataset.raw_rows("valid")?;
        let valid_label = |label: &TargetLabel| match label {
            TargetLabel::Value(value) => value.is_finite(),
            TargetLabel::Unavailable { reason } => !reason.trim().is_empty(),
        };
        if self.train_labels.len() != train_rows.len()
            || self.valid_labels.len() != valid_rows.len()
            || self.train_labels.iter().any(|label| !valid_label(label))
            || self.valid_labels.iter().any(|label| !valid_label(label))
            || self
                .train_labels
                .iter()
                .all(|label| matches!(label, TargetLabel::Unavailable { .. }))
            || self
                .valid_labels
                .iter()
                .all(|label| matches!(label, TargetLabel::Unavailable { .. }))
        {
            return Err(invalid("model-runner-label-contract-invalid"));
        }
        Ok(())
    }
}

impl DatasetH {
    pub fn new(partitions: Vec<HostPartition>) -> Result<Self, PythonResearchError> {
        if partitions.len() != 3 {
            return Err(invalid("qlib-dataset-requires-train-validation-test"));
        }
        let mut values = BTreeMap::new();
        for partition in partitions {
            partition.validate()?;
            let key = partition_key(partition.name);
            if values.insert(key.to_owned(), partition).is_some() {
                return Err(invalid("qlib-dataset-partitions-duplicate"));
            }
        }
        let feature_names = values
            .values()
            .next()
            .map(|partition| partition.feature_names.clone())
            .ok_or_else(|| invalid("qlib-dataset-partitions-empty"))?;
        if feature_names.iter().collect::<BTreeSet<_>>().len() != feature_names.len()
            || values
                .values()
                .any(|partition| partition.feature_names != feature_names)
        {
            return Err(invalid("qlib-dataset-feature-schema-mismatch"));
        }
        let mut identities = BTreeMap::new();
        for (partition_name, partition) in &values {
            for row in &partition.rows {
                if identities
                    .insert((row.datetime, row.instrument.clone()), partition_name)
                    .is_some()
                {
                    return Err(invalid("qlib-dataset-partitions-overlap"));
                }
            }
        }
        Ok(Self { partitions: values })
    }

    /// Supported Qlib surface: only `prepare(train|valid|test)`.
    pub fn prepare(&self, name: &str) -> Result<QlibView, PythonResearchError> {
        let partition = self.partition(name)?;
        let rows = if partition.labels_visible {
            partition
                .rows
                .iter()
                .filter(|row| row.label.is_some())
                .cloned()
                .collect()
        } else {
            partition.rows.clone()
        };
        Ok(QlibView {
            name: partition.name,
            feature_names: partition.feature_names.clone(),
            rows: rows.clone(),
            labels: partition
                .labels_visible
                .then(|| rows.iter().filter_map(|row| row.label).collect()),
        })
    }

    pub fn raw_rows(&self, name: &str) -> Result<Vec<HostPartitionRow>, PythonResearchError> {
        Ok(self.partition(name)?.rows.clone())
    }

    pub fn target_labels(&self, name: &str) -> Result<Vec<TargetLabel>, PythonResearchError> {
        let partition = self.partition(name)?;
        if !partition.labels_visible {
            return Err(invalid("qlib-test-labels-host-only"));
        }
        Ok(partition
            .rows
            .iter()
            .map(|row| {
                row.label
                    .map(TargetLabel::Value)
                    .unwrap_or_else(|| TargetLabel::Unavailable {
                        reason: "target-window-boundary".into(),
                    })
            })
            .collect())
    }

    fn partition(&self, name: &str) -> Result<&HostPartition, PythonResearchError> {
        let key = match name {
            "train" => "train",
            "valid" => "selection-validation",
            "test" => "test",
            _ => return Err(invalid("qlib-dataset-prepare-split-unsupported")),
        };
        self.partitions
            .get(key)
            .ok_or_else(|| invalid("qlib-dataset-partition-missing"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TutorialWindows {
    pub train_start: u32,
    pub train_end: u32,
    pub purge_start: u32,
    pub purge_end: u32,
    pub selection_start: u32,
    pub selection_end: u32,
    pub embargo_start: u32,
    pub embargo_end: u32,
    pub final_start: u32,
    pub final_end: u32,
}

impl TutorialWindows {
    pub const fn m12() -> Self {
        Self {
            train_start: 1,
            train_end: 100,
            purge_start: 101,
            purge_end: 105,
            selection_start: 106,
            selection_end: 140,
            embargo_start: 141,
            embargo_end: 145,
            final_start: 146,
            final_end: 180,
        }
    }

    pub fn validate(&self) -> Result<(), PythonResearchError> {
        if *self != Self::m12()
            || self.train_end >= self.purge_start
            || self.purge_end >= self.selection_start
            || self.selection_end >= self.embargo_start
            || self.embargo_end >= self.final_start
        {
            return Err(invalid("m12-tutorial-windows-invalid"));
        }
        Ok(())
    }
}

pub fn future_close_return(closes: &[f64], session_index: u32, window_end: u32) -> Option<f64> {
    if session_index == 0 || session_index.saturating_add(TARGET_HORIZON_BARS as u32) > window_end {
        return None;
    }
    let start = usize::try_from(session_index - 1).ok()?;
    let end = start.checked_add(TARGET_HORIZON_BARS)?;
    let (current, future) = (closes.get(start)?, closes.get(end)?);
    if !current.is_finite() || !future.is_finite() || *current == 0.0 {
        return None;
    }
    let value = future / current - 1.0;
    value.is_finite().then_some(value)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetUnavailableReason {
    WindowBoundary,
    MissingClose,
    InvalidClose,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TargetValue {
    Available(f64),
    Unavailable(TargetUnavailableReason),
}

pub fn future_close_return_state(
    closes: &[f64],
    session_index: u32,
    window_end: u32,
) -> TargetValue {
    if session_index == 0 || session_index.saturating_add(TARGET_HORIZON_BARS as u32) > window_end {
        return TargetValue::Unavailable(TargetUnavailableReason::WindowBoundary);
    }
    let Some(start) = usize::try_from(session_index - 1).ok() else {
        return TargetValue::Unavailable(TargetUnavailableReason::WindowBoundary);
    };
    let Some(end) = start.checked_add(TARGET_HORIZON_BARS) else {
        return TargetValue::Unavailable(TargetUnavailableReason::WindowBoundary);
    };
    let (Some(current), Some(future)) = (closes.get(start), closes.get(end)) else {
        return TargetValue::Unavailable(TargetUnavailableReason::MissingClose);
    };
    if !current.is_finite() || !future.is_finite() || *current == 0.0 {
        return TargetValue::Unavailable(TargetUnavailableReason::InvalidClose);
    }
    let value = future / current - 1.0;
    value
        .is_finite()
        .then_some(TargetValue::Available(value))
        .unwrap_or(TargetValue::Unavailable(
            TargetUnavailableReason::InvalidClose,
        ))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FittedTransformation {
    pub transformation_id: String,
    pub feature_names: Vec<String>,
    pub means: Vec<f64>,
    pub scales: Vec<f64>,
    pub transformation_sha256: String,
}

impl FittedTransformation {
    pub fn fit(
        rows: &[HostPartitionRow],
        feature_names: &[String],
    ) -> Result<Self, PythonResearchError> {
        if rows.is_empty()
            || feature_names.is_empty()
            || feature_names.iter().any(|name| name.trim().is_empty())
            || feature_names.iter().collect::<BTreeSet<_>>().len() != feature_names.len()
            || rows.iter().any(|row| {
                row.features.len() != feature_names.len()
                    || row.features.iter().any(|value| !value.is_finite())
            })
        {
            return Err(invalid("ridge-transformation-input-empty"));
        }
        let mut means = vec![0.0; feature_names.len()];
        for row in rows {
            if row.features.len() != means.len() {
                return Err(invalid("ridge-transformation-feature-dimension-invalid"));
            }
            for (mean, value) in means.iter_mut().zip(&row.features) {
                *mean += *value;
            }
        }
        for mean in &mut means {
            *mean /= rows.len() as f64;
        }
        let mut scales = vec![0.0; means.len()];
        for row in rows {
            for ((scale, value), mean) in scales.iter_mut().zip(&row.features).zip(&means) {
                *scale += (*value - *mean).powi(2);
            }
        }
        for scale in &mut scales {
            *scale = (*scale / rows.len() as f64).sqrt().max(1e-12);
        }
        let content = serde_json::to_vec(&(feature_names, &means, &scales))
            .map_err(|error| invalid(error.to_string()))?;
        let transformation_sha256 = sha256(&content);
        Ok(Self {
            transformation_id: format!("adaq:train-standardization@1:{transformation_sha256}"),
            feature_names: feature_names.to_vec(),
            means,
            scales,
            transformation_sha256,
        })
    }

    pub fn apply(&self, features: &[f64]) -> Result<Vec<f64>, PythonResearchError> {
        if features.len() != self.means.len() || self.scales.len() != features.len() {
            return Err(invalid("ridge-transformation-dimension-invalid"));
        }
        features
            .iter()
            .zip(self.means.iter().zip(&self.scales))
            .map(|(value, (mean, scale))| {
                let value = (*value - *mean) / *scale;
                value
                    .is_finite()
                    .then_some(value)
                    .ok_or_else(|| invalid("ridge-transformation-non-finite"))
            })
            .collect()
    }

    pub fn validate(&self) -> Result<(), PythonResearchError> {
        if self.feature_names.is_empty()
            || self.feature_names.len() != self.means.len()
            || self.means.len() != self.scales.len()
            || self.feature_names.iter().any(|name| name.trim().is_empty())
            || self.feature_names.iter().collect::<BTreeSet<_>>().len() != self.feature_names.len()
            || self.means.iter().any(|value| !value.is_finite())
            || self
                .scales
                .iter()
                .any(|value| !value.is_finite() || *value <= 0.0)
            || !is_sha256(&self.transformation_sha256)
        {
            return Err(invalid("ridge-transformation-invalid"));
        }
        let content = serde_json::to_vec(&(&self.feature_names, &self.means, &self.scales))
            .map_err(|error| invalid(error.to_string()))?;
        if self.transformation_sha256 != sha256(&content) {
            return Err(invalid("ridge-transformation-hash-mismatch"));
        }
        if self.transformation_id
            != format!(
                "adaq:train-standardization@1:{}",
                self.transformation_sha256
            )
        {
            return Err(invalid("ridge-transformation-id-mismatch"));
        }
        Ok(())
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, PythonResearchError> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|error| invalid(error.to_string()))
    }

    pub fn reload(bytes: &[u8]) -> Result<Self, PythonResearchError> {
        let transformation: Self = serde_json::from_slice(bytes)
            .map_err(|error| invalid(format!("ridge-transformation-json-invalid:{error}")))?;
        transformation.validate()?;
        Ok(transformation)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LinearModelArtifact {
    pub schema: String,
    pub adapter_id: String,
    pub target_id: String,
    pub horizon_bars: u32,
    pub alpha: f64,
    #[serde(default = "default_numeric_representation")]
    pub numeric_representation: String,
    #[serde(default = "default_forecast_contract")]
    pub forecast_contract: String,
    pub input_slots: Vec<String>,
    pub coefficients: Vec<f64>,
    pub intercept: f64,
    pub transformation_sha256: String,
    pub provenance_hashes: BTreeMap<String, String>,
    pub artifact_sha256: String,
}

impl LinearModelArtifact {
    pub fn validate(&self) -> Result<(), PythonResearchError> {
        if self.schema != LINEAR_MODEL_ARTIFACT_SCHEMA
            || self.adapter_id != RIDGE_ADAPTER_ID
            || self.target_id != TARGET_ID
            || self.horizon_bars != TARGET_HORIZON_BARS as u32
            || !self.alpha.is_finite()
            || self.alpha <= 0.0
            || self.numeric_representation != NUMERIC_REPRESENTATION
            || self.forecast_contract != FORECAST_CONTRACT
            || self.input_slots.is_empty()
            || self.input_slots.len() > MAX_MODEL_INPUT_SLOTS
            || self.input_slots.iter().any(|slot| slot.trim().is_empty())
            || self.input_slots.iter().collect::<BTreeSet<_>>().len() != self.input_slots.len()
            || self.input_slots.len() != self.coefficients.len()
            || self.coefficients.iter().any(|value| !value.is_finite())
            || !self.intercept.is_finite()
            || !is_sha256(&self.transformation_sha256)
            || self.provenance_hashes.values().any(|hash| !is_sha256(hash))
            || !is_sha256(&self.artifact_sha256)
            || self.provenance_hashes.is_empty()
            || self.provenance_hashes.len() > MAX_MODEL_PROVENANCE_ENTRIES
        {
            return Err(invalid("linear-model-artifact-invalid"));
        }
        let mut content = self.clone();
        content.artifact_sha256.clear();
        let bytes = serde_json::to_vec(&content).map_err(|error| invalid(error.to_string()))?;
        if self.artifact_sha256 != sha256(&bytes) {
            if !self.legacy_hash_matches()? {
                return Err(invalid("linear-model-artifact-hash-mismatch"));
            }
        }
        Ok(())
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, PythonResearchError> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|error| invalid(error.to_string()))?;
        if bytes.len() > MAX_MODEL_ARTIFACT_BYTES {
            return Err(invalid("linear-model-artifact-size-limit"));
        }
        Ok(bytes)
    }

    pub fn reload(bytes: &[u8]) -> Result<Self, PythonResearchError> {
        let artifact: Self = serde_json::from_slice(bytes)
            .map_err(|error| invalid(format!("linear-model-artifact-json-invalid:{error}")))?;
        artifact.validate()?;
        Ok(artifact)
    }

    pub fn predict(
        &self,
        transformation: &FittedTransformation,
        features: &[f64],
    ) -> Result<f64, PythonResearchError> {
        if transformation.transformation_sha256 != self.transformation_sha256 {
            return Err(invalid("linear-model-transformation-mismatch"));
        }
        let values = transformation.apply(features)?;
        let value = self.intercept
            + self
                .coefficients
                .iter()
                .zip(values)
                .map(|(coefficient, value)| coefficient * value)
                .sum::<f64>();
        value
            .is_finite()
            .then_some(value)
            .ok_or_else(|| invalid("linear-model-prediction-non-finite"))
    }

    pub fn numeric_representation() -> &'static str {
        NUMERIC_REPRESENTATION
    }

    pub fn forecast_contract() -> &'static str {
        FORECAST_CONTRACT
    }

    pub fn from_coefficients(
        alpha: f64,
        input_slots: Vec<String>,
        coefficients: Vec<f64>,
        intercept: f64,
        transformation_sha256: String,
        provenance_hashes: BTreeMap<String, String>,
    ) -> Result<Self, PythonResearchError> {
        let mut artifact = Self {
            schema: LINEAR_MODEL_ARTIFACT_SCHEMA.into(),
            adapter_id: RIDGE_ADAPTER_ID.into(),
            target_id: TARGET_ID.into(),
            horizon_bars: TARGET_HORIZON_BARS as u32,
            alpha,
            numeric_representation: NUMERIC_REPRESENTATION.into(),
            forecast_contract: FORECAST_CONTRACT.into(),
            input_slots,
            coefficients,
            intercept,
            transformation_sha256,
            provenance_hashes,
            artifact_sha256: String::new(),
        };
        let bytes = serde_json::to_vec(&artifact).map_err(|error| invalid(error.to_string()))?;
        artifact.artifact_sha256 = sha256(&bytes);
        artifact.validate()?;
        Ok(artifact)
    }

    fn legacy_hash_matches(&self) -> Result<bool, PythonResearchError> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Legacy<'a> {
            schema: &'a str,
            adapter_id: &'a str,
            target_id: &'a str,
            horizon_bars: u32,
            alpha: f64,
            input_slots: &'a [String],
            coefficients: &'a [f64],
            intercept: f64,
            transformation_sha256: &'a str,
            provenance_hashes: &'a BTreeMap<String, String>,
            artifact_sha256: &'a str,
        }
        let legacy = Legacy {
            schema: &self.schema,
            adapter_id: &self.adapter_id,
            target_id: &self.target_id,
            horizon_bars: self.horizon_bars,
            alpha: self.alpha,
            input_slots: &self.input_slots,
            coefficients: &self.coefficients,
            intercept: self.intercept,
            transformation_sha256: &self.transformation_sha256,
            provenance_hashes: &self.provenance_hashes,
            artifact_sha256: "",
        };
        let bytes = serde_json::to_vec(&legacy).map_err(|error| invalid(error.to_string()))?;
        Ok(self.artifact_sha256 == sha256(&bytes))
    }
}

fn default_numeric_representation() -> String {
    NUMERIC_REPRESENTATION.into()
}

fn default_forecast_contract() -> String {
    FORECAST_CONTRACT.into()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RidgeAdapter {
    pub alpha: f64,
}

impl RidgeAdapter {
    pub fn registered(alpha: f64) -> Result<Self, PythonResearchError> {
        if !crate::tuning::RIDGE_ALPHAS
            .iter()
            .any(|allowed| alpha.to_bits() == allowed.to_bits())
        {
            return Err(invalid("ridge-alpha-invalid"));
        }
        Ok(Self { alpha })
    }

    pub fn fit(
        &self,
        dataset: &DatasetH,
        transformation: &FittedTransformation,
        provenance_hashes: BTreeMap<String, String>,
    ) -> Result<LinearModelArtifact, PythonResearchError> {
        transformation.validate()?;
        if provenance_hashes.values().any(|hash| !is_sha256(hash)) {
            return Err(invalid("ridge-provenance-hash-invalid"));
        }
        let train = dataset.prepare("train")?;
        let selection = dataset.prepare("valid")?;
        if train.feature_names != transformation.feature_names
            || selection.feature_names != transformation.feature_names
        {
            return Err(invalid("ridge-transformation-input-schema-mismatch"));
        }
        let labels = train
            .labels
            .as_ref()
            .ok_or_else(|| invalid("ridge-train-labels-unavailable"))?;
        if labels.len() != train.rows.len() {
            return Err(invalid("ridge-train-label-count-invalid"));
        }
        if selection
            .labels
            .as_ref()
            .is_none_or(|labels| labels.iter().any(|label| !label.is_finite()))
        {
            return Err(invalid("ridge-selection-labels-unavailable"));
        }
        let mut matrix = vec![vec![0.0; transformation.feature_names.len() + 1]; labels.len()];
        for (row_index, (row, target)) in train.rows.iter().zip(labels).enumerate() {
            let features = transformation.apply(&row.features)?;
            matrix[row_index][0] = 1.0;
            matrix[row_index][1..].copy_from_slice(&features);
            if !target.is_finite() {
                return Err(invalid("ridge-train-label-non-finite"));
            }
        }
        let coefficients = solve_ridge(&matrix, labels, self.alpha)?;
        let intercept = coefficients[0];
        let artifact_coefficients = coefficients[1..].to_vec();
        let mut artifact = LinearModelArtifact {
            schema: LINEAR_MODEL_ARTIFACT_SCHEMA.into(),
            adapter_id: RIDGE_ADAPTER_ID.into(),
            target_id: TARGET_ID.into(),
            horizon_bars: TARGET_HORIZON_BARS as u32,
            alpha: self.alpha,
            numeric_representation: NUMERIC_REPRESENTATION.into(),
            forecast_contract: FORECAST_CONTRACT.into(),
            input_slots: transformation.feature_names.clone(),
            coefficients: artifact_coefficients,
            intercept,
            transformation_sha256: transformation.transformation_sha256.clone(),
            provenance_hashes,
            artifact_sha256: String::new(),
        };
        let bytes = serde_json::to_vec(&artifact).map_err(|error| invalid(error.to_string()))?;
        artifact.artifact_sha256 = sha256(&bytes);
        artifact.validate()?;
        Ok(artifact)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForecastRow {
    pub datetime: i64,
    pub instrument: String,
    pub value: Option<f64>,
    pub unavailable_reason: Option<String>,
}

pub fn forecast(
    artifact: &LinearModelArtifact,
    transformation: &FittedTransformation,
    partition: &QlibView,
) -> Result<Vec<ForecastRow>, PythonResearchError> {
    artifact.validate()?;
    transformation.validate()?;
    if partition.feature_names != artifact.input_slots
        || partition.labels.is_some() && partition.name == PartitionName::Test
        || partition.rows.windows(2).any(|rows| {
            (rows[0].datetime, rows[0].instrument.as_str())
                >= (rows[1].datetime, rows[1].instrument.as_str())
        })
        || partition
            .rows
            .iter()
            .any(|row| row.features.len() != artifact.input_slots.len())
    {
        return Err(invalid("forecast-partition-contract-invalid"));
    }
    partition
        .rows
        .iter()
        .map(|row| {
            Ok(ForecastRow {
                datetime: row.datetime,
                instrument: row.instrument.clone(),
                value: Some(artifact.predict(transformation, &row.features)?),
                unavailable_reason: None,
            })
        })
        .collect()
}

fn solve_ridge(
    matrix: &[Vec<f64>],
    labels: &[f64],
    alpha: f64,
) -> Result<Vec<f64>, PythonResearchError> {
    if matrix.is_empty() || matrix.len() != labels.len() {
        return Err(invalid("ridge-matrix-invalid"));
    }
    let width = matrix[0].len();
    if width == 0 || matrix.iter().any(|row| row.len() != width) {
        return Err(invalid("ridge-matrix-dimension-invalid"));
    }
    let mut normal = vec![vec![0.0; width + 1]; width];
    for (row, label) in matrix.iter().zip(labels) {
        for left in 0..width {
            for right in 0..width {
                normal[left][right] += row[left] * row[right];
            }
            normal[left][width] += row[left] * label;
        }
    }
    for index in 1..width {
        normal[index][index] += alpha;
    }
    for pivot in 0..width {
        let mut selected = pivot;
        for row in pivot..width {
            if normal[row][pivot].abs() > normal[selected][pivot].abs() {
                selected = row;
            }
        }
        if normal[selected][pivot].abs() < 1e-12 {
            return Err(invalid("ridge-normal-matrix-singular"));
        }
        normal.swap(pivot, selected);
        let divisor = normal[pivot][pivot];
        for value in &mut normal[pivot][pivot..=width] {
            *value /= divisor;
        }
        for row in 0..width {
            if row == pivot {
                continue;
            }
            let factor = normal[row][pivot];
            for column in pivot..=width {
                normal[row][column] -= factor * normal[pivot][column];
            }
        }
    }
    let result = normal.into_iter().map(|row| row[width]).collect::<Vec<_>>();
    result
        .iter()
        .all(|value| value.is_finite())
        .then_some(result)
        .ok_or_else(|| invalid("ridge-coefficients-non-finite"))
}

fn partition_key(name: PartitionName) -> &'static str {
    match name {
        PartitionName::Train => "train",
        PartitionName::SelectionValidation => "selection-validation",
        PartitionName::Test => "test",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(datetime: i64, instrument: &str, x: f64, label: Option<f64>) -> HostPartitionRow {
        HostPartitionRow {
            datetime,
            instrument: instrument.into(),
            features: vec![x],
            label,
        }
    }

    fn dataset() -> DatasetH {
        DatasetH::new(vec![
            HostPartition {
                name: PartitionName::Train,
                feature_names: vec!["momentum-score".into()],
                rows: vec![
                    row(1, "AAA", 0.0, Some(1.0)),
                    row(2, "AAA", 1.0, Some(3.0)),
                    row(3, "AAA", 2.0, Some(5.0)),
                ],
                labels_visible: true,
            },
            HostPartition {
                name: PartitionName::SelectionValidation,
                feature_names: vec!["momentum-score".into()],
                rows: vec![row(4, "AAA", 3.0, Some(7.0))],
                labels_visible: true,
            },
            HostPartition {
                name: PartitionName::Test,
                feature_names: vec!["momentum-score".into()],
                rows: vec![row(5, "AAA", 4.0, None)],
                labels_visible: false,
            },
        ])
        .unwrap()
    }

    #[test]
    fn qlib_bridge_withholds_test_labels_and_rejects_provider_surface() {
        let data = dataset();
        assert!(data.prepare("test").unwrap().labels.is_none());
        assert!(data.prepare("provider").is_err());
    }

    #[test]
    fn qlib_bridge_preserves_boundary_labels_without_fitting_them() {
        let data = DatasetH::new(vec![
            HostPartition {
                name: PartitionName::Train,
                feature_names: vec!["momentum-score".into()],
                rows: vec![row(1, "AAA", 1.0, Some(1.0)), row(2, "AAA", 2.0, None)],
                labels_visible: true,
            },
            HostPartition {
                name: PartitionName::SelectionValidation,
                feature_names: vec!["momentum-score".into()],
                rows: vec![row(3, "AAA", 3.0, Some(3.0)), row(4, "AAA", 4.0, None)],
                labels_visible: true,
            },
            HostPartition {
                name: PartitionName::Test,
                feature_names: vec!["momentum-score".into()],
                rows: vec![row(5, "AAA", 5.0, None)],
                labels_visible: false,
            },
        ])
        .unwrap();
        assert_eq!(data.prepare("train").unwrap().rows.len(), 1);
        assert_eq!(data.prepare("valid").unwrap().rows.len(), 1);
        assert_eq!(
            data.target_labels("train").unwrap()[1],
            TargetLabel::Unavailable {
                reason: "target-window-boundary".into()
            }
        );
        assert_eq!(
            data.target_labels("valid").unwrap()[1],
            TargetLabel::Unavailable {
                reason: "target-window-boundary".into()
            }
        );
    }

    #[test]
    fn qlib_bridge_rejects_partition_schema_drift() {
        let data = dataset();
        let mut partitions = data.partitions.into_values().collect::<Vec<_>>();
        partitions
            .iter_mut()
            .find(|partition| partition.name == PartitionName::Test)
            .unwrap()
            .feature_names = vec!["different-feature".into()];
        assert!(DatasetH::new(partitions).is_err());
    }

    #[test]
    fn model_project_payload_requires_one_supported_target_and_signal() {
        let payload = serde_json::json!({
            "target": {
                "id": "future-close-return",
                "kind": "continuous-future-close-return",
                "horizon_bars": 5,
                "value_scale": "return"
            },
            "signal": {"id": "forecast", "kind": "forecast", "value_scale": "native"}
        });
        assert!(validate_model_project_payload(&payload).is_ok());
        assert!(
            validate_model_project_payload(&serde_json::json!({
                "target": {
                    "id": "future-close-return",
                    "kind": "continuous-future-close-return",
                    "horizon_bars": 10,
                    "value_scale": "return"
                },
                "signal": {"id": "forecast", "kind": "forecast", "value_scale": "native"}
            }))
            .is_err()
        );
    }

    #[test]
    fn windows_and_target_crossing_are_exact() {
        let windows = TutorialWindows::m12();
        windows.validate().unwrap();
        assert_eq!(
            future_close_return(&[1.0; 180], 100, windows.train_end),
            None
        );
        assert!(future_close_return(&[1.0; 180], 95, windows.train_end).is_some());
        assert_eq!(
            future_close_return_state(&[1.0; 180], 100, windows.train_end),
            TargetValue::Unavailable(TargetUnavailableReason::WindowBoundary)
        );
        assert_eq!(
            future_close_return_state(&[1.0; 180], 95, windows.train_end),
            TargetValue::Available(0.0)
        );
    }

    #[test]
    fn standardization_uses_train_rows_only() {
        let data = dataset();
        let transform = FittedTransformation::fit(
            &data.prepare("train").unwrap().rows,
            &["momentum-score".into()],
        )
        .unwrap();
        assert_eq!(transform.means, vec![1.0]);
        assert!(transform.validate().is_ok());
    }

    #[test]
    fn train_only_standardization_ridge_artifact_reload_and_forecast() {
        let data = dataset();
        let train = data.prepare("train").unwrap();
        let transform = FittedTransformation::fit(&train.rows, &train.feature_names).unwrap();
        let adapter = RidgeAdapter::registered(1.0).unwrap();
        let artifact = adapter
            .fit(
                &data,
                &transform,
                BTreeMap::from([("revision".into(), sha256(b"revision"))]),
            )
            .unwrap();
        let reloaded = LinearModelArtifact::reload(&artifact.to_bytes().unwrap()).unwrap();
        let forecasts = forecast(&reloaded, &transform, &data.prepare("test").unwrap()).unwrap();
        assert_eq!(forecasts.len(), 1);
        assert!(forecasts[0].value.unwrap().is_finite());
        assert_eq!(reloaded.numeric_representation, "ieee754-binary64");
        assert_eq!(reloaded.forecast_contract, FORECAST_CONTRACT);
        assert!(!String::from_utf8_lossy(&artifact.to_bytes().unwrap()).contains("pickle"));
    }

    #[test]
    fn unsupported_alpha_and_non_finite_rows_fail() {
        assert!(RidgeAdapter::registered(0.0).is_err());
        assert!(RidgeAdapter::registered(2.0).is_err());
        let mut data = dataset();
        data.partitions.get_mut("train").unwrap().rows[0].features[0] = f64::NAN;
        assert!(data.partitions["train"].validate().is_err());
    }
}
