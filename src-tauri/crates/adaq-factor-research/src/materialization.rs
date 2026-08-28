use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    sync::{Arc, Mutex},
};

use adaq_component_sdk::host::{factor_cross_sectional_abi, factor_time_series_abi};
use adaq_component_tooling::{ComponentKind, ComponentPackage, WasmLoader, verify_package};
use adaq_feature_engine::{
    FeatureDatasetCell, FeatureDatasetRow, FeatureEngineIdentity, FeaturePlan,
    FeatureUnavailabilityReason,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    AttemptStatus, ContractError, FactorCandidate, FactorCandidateDraft, FactorCandidateSource,
    FactorDatasetManifest, FactorMaterializationAttempt, FactorMaterializationProtocol,
    FactorMaterializationProtocolDraft, FactorObservationValue, FactorParameterValue, FactorScope,
    NamedFactorOutput, component_parameter_values, run_limits,
};

pub const MAX_FACTOR_DATASET_ROWS: usize = 2_520_000;
const MAX_DIAGNOSTIC_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompletedFeatureDataset {
    pub user_id: String,
    pub dataset_id: String,
    pub feature_plan_hash: String,
    pub plan_json: Vec<u8>,
    pub engine_identity: FeatureEngineIdentity,
    pub market_data_snapshot_id: String,
    pub point_in_time_universe_id: String,
    pub output_names: Vec<String>,
    pub rows: Vec<FeatureDatasetRow>,
}

impl CompletedFeatureDataset {
    pub fn new(
        user_id: impl Into<String>,
        dataset_id: impl Into<String>,
        feature_plan_hash: impl Into<String>,
        plan_json: Vec<u8>,
        engine_identity: FeatureEngineIdentity,
        market_data_snapshot_id: impl Into<String>,
        point_in_time_universe_id: impl Into<String>,
        output_names: Vec<String>,
        rows: Vec<FeatureDatasetRow>,
    ) -> Result<Self, MaterializationError> {
        let dataset = Self {
            user_id: user_id.into(),
            dataset_id: dataset_id.into(),
            feature_plan_hash: feature_plan_hash.into(),
            plan_json,
            engine_identity,
            market_data_snapshot_id: market_data_snapshot_id.into(),
            point_in_time_universe_id: point_in_time_universe_id.into(),
            output_names,
            rows,
        };
        dataset.validate()?;
        Ok(dataset)
    }

    pub fn validate(&self) -> Result<(), MaterializationError> {
        if self.user_id.trim().is_empty()
            || self.dataset_id.trim().is_empty()
            || !crate::is_sha256(&self.feature_plan_hash)
            || self.plan_json.is_empty()
            || self.market_data_snapshot_id.trim().is_empty()
            || self.point_in_time_universe_id.trim().is_empty()
            || self.output_names.is_empty()
            || self.output_names.len() > crate::MAX_FACTOR_OUTPUTS
            || self
                .output_names
                .iter()
                .any(|name| !crate::is_lower_kebab(name))
            || {
                let mut names = BTreeSet::new();
                self.output_names.iter().any(|name| !names.insert(name))
            }
            || self.rows.len() > MAX_FACTOR_DATASET_ROWS
        {
            return Err(MaterializationError::Invalid(
                "Completed Feature Dataset identity or bounds are invalid".into(),
            ));
        }
        let plan = FeaturePlan::load_for_engine(&self.plan_json, &self.engine_identity).map_err(
            |error| {
                MaterializationError::Invalid(format!("Feature Plan evidence is invalid: {error}"))
            },
        )?;
        if plan.plan_hash() != self.feature_plan_hash {
            return Err(MaterializationError::Invalid(
                "Feature Plan evidence hash does not match the completed Dataset".into(),
            ));
        }
        let plan_outputs = plan
            .slot_names()
            .map(str::to_owned)
            .chain(plan.definitions().iter().flat_map(|definition| {
                definition
                    .outputs()
                    .iter()
                    .map(|output| output.name.clone())
            }))
            .collect::<BTreeSet<_>>();
        if self
            .output_names
            .iter()
            .any(|output| !plan_outputs.contains(output))
        {
            return Err(MaterializationError::Invalid(
                "Completed Feature Dataset outputs are not declared by its Feature Plan".into(),
            ));
        }
        let expected = self.output_names.iter().cloned().collect::<BTreeSet<_>>();
        let mut previous = None;
        for row in &self.rows {
            if row.instrument_id.trim().is_empty()
                || previous.as_ref().is_some_and(|previous: &(String, i64)| {
                    (previous.0.as_str(), previous.1)
                        >= (row.instrument_id.as_str(), row.observation_time_ms)
                })
                || row.values.keys().collect::<BTreeSet<_>>() != expected.iter().collect()
            {
                return Err(MaterializationError::Invalid(
                    "Completed Feature Dataset rows are incomplete or not canonically ordered"
                        .into(),
                ));
            }
            for cell in row.values.values() {
                if let FeatureDatasetCell::Available {
                    value,
                    available_at_ms,
                } = cell
                    && (!value.is_finite() || *available_at_ms > row.observation_time_ms)
                {
                    return Err(MaterializationError::Invalid(
                        "Feature Dataset cells must be finite and causally available".into(),
                    ));
                }
            }
            previous = Some((row.instrument_id.clone(), row.observation_time_ms));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactorDatasetRow {
    pub instrument_id: String,
    pub observation_time_ms: i64,
    pub values: BTreeMap<String, FactorObservationValue>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FactorDataset {
    pub manifest: FactorDatasetManifest,
    pub rows: Vec<FactorDatasetRow>,
}

impl FactorDataset {
    pub fn validate(&self) -> Result<(), MaterializationError> {
        self.manifest.validate()?;
        if self.manifest.output_names.is_empty()
            || self.manifest.output_names.len() > crate::MAX_FACTOR_OUTPUTS
            || self.rows.len() as u64 != self.manifest.observation_count
            || self.rows.len() > MAX_FACTOR_DATASET_ROWS
        {
            return Err(MaterializationError::Invalid(
                "Factor Dataset row count is inconsistent with its manifest".into(),
            ));
        }
        let names = self
            .manifest
            .output_names
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        let mut previous = None;
        for row in &self.rows {
            if row.instrument_id.trim().is_empty()
                || previous.as_ref().is_some_and(|previous: &(String, i64)| {
                    (previous.0.as_str(), previous.1)
                        >= (row.instrument_id.as_str(), row.observation_time_ms)
                })
                || row.values.keys().collect::<BTreeSet<_>>() != names.iter().collect()
            {
                return Err(MaterializationError::Invalid(
                    "Factor Dataset rows are incomplete or not canonically ordered".into(),
                ));
            }
            for value in row.values.values() {
                if let FactorObservationValue::Available {
                    value,
                    available_at_ms,
                } = value
                    && (!value.is_finite() || *available_at_ms > row.observation_time_ms)
                {
                    return Err(MaterializationError::Invalid(
                        "Factor Dataset values must be finite and causally available".into(),
                    ));
                }
            }
            previous = Some((row.instrument_id.clone(), row.observation_time_ms));
        }
        let payload = DatasetPayload {
            output_names: &self.manifest.output_names,
            rows: &self.rows,
        };
        if self.manifest.payload_sha256 != payload_hash(&payload)? {
            return Err(MaterializationError::Invalid(
                "Factor Dataset payload hash does not match its rows".into(),
            ));
        }
        if self.manifest.content_id()? != self.manifest.dataset_id {
            return Err(MaterializationError::Invalid(
                "Factor Dataset identity does not match its manifest".into(),
            ));
        }
        Ok(())
    }

    pub fn payload_json(&self) -> Result<Vec<u8>, MaterializationError> {
        serde_json::to_vec(&DatasetPayload {
            output_names: &self.manifest.output_names,
            rows: &self.rows,
        })
        .map_err(Into::into)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DatasetPayload<'a> {
    output_names: &'a [String],
    rows: &'a [FactorDatasetRow],
}

fn payload_hash(payload: &DatasetPayload<'_>) -> Result<String, MaterializationError> {
    let bytes = serde_json::to_vec(payload)?;
    Ok(adaq_feature_engine::sha256(&bytes))
}

fn parameter_value_matches(
    value: &FactorParameterValue,
    parameter_type: crate::FactorParameterType,
) -> bool {
    matches!(
        (value, parameter_type),
        (
            FactorParameterValue::Decimal(_),
            crate::FactorParameterType::Decimal
        ) | (
            FactorParameterValue::Integer(_),
            crate::FactorParameterType::Integer
        ) | (
            FactorParameterValue::Boolean(_),
            crate::FactorParameterType::Boolean
        ) | (
            FactorParameterValue::Text(_),
            crate::FactorParameterType::Text
        )
    )
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DatasetIdentity<'a> {
    pub(crate) schema_version: &'a str,
    pub(crate) protocol_hash: &'a str,
    pub(crate) candidate_hash: &'a str,
    pub(crate) scope: crate::FactorScope,
    pub(crate) feature_dataset_id: &'a str,
    pub(crate) feature_plan_hash: &'a str,
    pub(crate) market_data_snapshot_id: &'a str,
    pub(crate) point_in_time_universe_id: &'a str,
    pub(crate) market_context: &'a crate::FactorMarketContext,
    pub(crate) output_names: &'a [String],
    pub(crate) observation_count: u64,
    pub(crate) payload_sha256: &'a str,
    pub(crate) engine_identity: &'a crate::ResearchEngineProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaterializationError {
    Invalid(String),
    Contract(ContractError),
    Execution(String),
    Cancelled,
    AttemptNotFound,
    Unauthorized,
    InvalidTransition,
    DatasetReferenced,
}

impl std::fmt::Display for MaterializationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(message) | Self::Execution(message) => formatter.write_str(message),
            Self::Contract(error) => error.fmt(formatter),
            Self::Cancelled => formatter.write_str("factor-materialization-cancelled"),
            Self::AttemptNotFound => {
                formatter.write_str("factor-materialization-attempt-not-found")
            }
            Self::Unauthorized => formatter.write_str("factor-dataset-unauthorized"),
            Self::InvalidTransition => {
                formatter.write_str("invalid-factor-materialization-transition")
            }
            Self::DatasetReferenced => formatter.write_str("factor-dataset-referenced"),
        }
    }
}

impl std::error::Error for MaterializationError {}

impl From<ContractError> for MaterializationError {
    fn from(error: ContractError) -> Self {
        Self::Contract(error)
    }
}

impl From<serde_json::Error> for MaterializationError {
    fn from(error: serde_json::Error) -> Self {
        Self::Invalid(error.to_string())
    }
}

#[derive(Clone, Copy)]
pub struct FactorMaterializationInput<'a> {
    pub candidate: &'a FactorCandidate,
    pub protocol: &'a FactorMaterializationProtocol,
    pub feature_dataset: &'a CompletedFeatureDataset,
    pub point_in_time_universe: &'a [String],
    pub custom_package: Option<&'a ComponentPackage>,
}

pub struct FactorMaterializer;

impl FactorMaterializer {
    pub fn materialize(
        input: FactorMaterializationInput<'_>,
    ) -> Result<FactorDataset, MaterializationError> {
        input.candidate.validate()?;
        input.protocol.validate()?;
        input.feature_dataset.validate()?;
        if input.protocol.candidate_hash != input.candidate.candidate_hash
            || input.protocol.feature_dataset_id != input.feature_dataset.dataset_id
            || input.protocol.feature_plan_hash != input.feature_dataset.feature_plan_hash
            || input.protocol.user_id.to_string() != input.feature_dataset.user_id
            || input.protocol.market_data_snapshot_id
                != input.feature_dataset.market_data_snapshot_id
            || input.protocol.point_in_time_universe_id
                != input.feature_dataset.point_in_time_universe_id
        {
            return Err(MaterializationError::Invalid(
                "Factor Materialization Protocol is not bound to the exact User, Candidate, Plan, or Feature Dataset".into(),
            ));
        }
        if input.protocol.parameters.len() != input.candidate.parameters.len()
            || input
                .protocol
                .parameters
                .iter()
                .zip(&input.candidate.parameters)
                .any(|(value, parameter)| !parameter_value_matches(value, parameter.parameter_type))
        {
            return Err(MaterializationError::Invalid(
                "Factor Materialization parameters do not match the Candidate schema".into(),
            ));
        }
        if input.protocol.market_context.point_in_time_universe_id
            != input.protocol.point_in_time_universe_id
        {
            return Err(MaterializationError::Invalid(
                "Factor Materialization market context and Universe identity differ".into(),
            ));
        }
        let output_names = input
            .candidate
            .outputs
            .iter()
            .map(|output| output.name.clone())
            .collect::<Vec<_>>();
        let slots = input
            .candidate
            .feature_slots
            .iter()
            .map(|slot| slot.name.clone())
            .collect::<Vec<_>>();
        if slots
            .iter()
            .any(|slot| !input.feature_dataset.output_names.contains(slot))
            || input.protocol.observation_range.start_time_ms
                >= input.protocol.observation_range.end_time_ms
        {
            return Err(MaterializationError::Invalid(
                "Factor Materialization inputs do not contain every ordered Feature Slot".into(),
            ));
        }
        let source_rows = input
            .feature_dataset
            .rows
            .iter()
            .filter(|row| {
                row.observation_time_ms >= input.protocol.observation_range.start_time_ms
                    && row.observation_time_ms < input.protocol.observation_range.end_time_ms
            })
            .cloned()
            .collect::<Vec<_>>();
        let rows = match (&input.candidate.source, input.candidate.scope) {
            (FactorCandidateSource::Declarative { definition }, FactorScope::TimeSeries) => {
                materialize_declarative_time_series(definition, &output_names, &source_rows)?
            }
            (FactorCandidateSource::Declarative { definition }, FactorScope::CrossSectional) => {
                materialize_declarative_cross_sectional(
                    definition,
                    &output_names,
                    &source_rows,
                    input.point_in_time_universe,
                )?
            }
            (FactorCandidateSource::Custom { build }, FactorScope::TimeSeries) => {
                let package = input.custom_package.ok_or_else(|| {
                    MaterializationError::Invalid("Custom Candidate package is required".into())
                })?;
                validate_custom_package(package, build, FactorScope::TimeSeries)?;
                materialize_custom_time_series(
                    package,
                    build.resource_policy,
                    &input.protocol.parameters,
                    &slots,
                    &output_names,
                    &source_rows,
                )?
            }
            (FactorCandidateSource::Custom { build }, FactorScope::CrossSectional) => {
                let package = input.custom_package.ok_or_else(|| {
                    MaterializationError::Invalid("Custom Candidate package is required".into())
                })?;
                validate_custom_package(package, build, FactorScope::CrossSectional)?;
                materialize_custom_cross_sectional(
                    package,
                    build.resource_policy,
                    &input.protocol.parameters,
                    &slots,
                    &output_names,
                    &source_rows,
                    input.point_in_time_universe,
                )?
            }
            (FactorCandidateSource::Python { binding }, FactorScope::CrossSectional) => {
                if binding.feature_plan_hash != input.protocol.feature_plan_hash {
                    return Err(MaterializationError::Invalid(
                        "Python Factor binding does not match the Materialization Protocol Feature Plan".into(),
                    ));
                }
                materialize_python_cross_sectional(
                    binding,
                    &input.protocol.parameters,
                    &slots,
                    &output_names,
                    &source_rows,
                    input.point_in_time_universe,
                )?
            }
            (FactorCandidateSource::Python { .. }, FactorScope::TimeSeries) => {
                return Err(MaterializationError::Invalid(
                    "Python momentum Factor currently requires Cross-sectional scope".into(),
                ));
            }
        };
        let mut rows = rows;
        if let FactorCandidateSource::Declarative { definition } = &input.candidate.source {
            if definition.feature_plan_hash != input.protocol.feature_plan_hash {
                return Err(MaterializationError::Invalid(
                    "Declarative Factor does not bind the Protocol Feature Plan".into(),
                ));
            }
        }
        if input.candidate.scope == FactorScope::CrossSectional {
            validate_cross_sectional_dataset_rows(&rows, input.point_in_time_universe)?;
        }
        rows.sort_by(|left, right| {
            (left.instrument_id.as_str(), left.observation_time_ms)
                .cmp(&(right.instrument_id.as_str(), right.observation_time_ms))
        });
        if rows.len() > MAX_FACTOR_DATASET_ROWS {
            return Err(MaterializationError::Invalid(
                "Factor Dataset exceeds its checked row limit".into(),
            ));
        }
        let payload_sha256 = payload_hash(&DatasetPayload {
            output_names: &output_names,
            rows: &rows,
        })?;
        let mut manifest = FactorDatasetManifest {
            schema_version: crate::FACTOR_RESEARCH_SCHEMA_VERSION.into(),
            dataset_id: String::new(),
            protocol_hash: input.protocol.protocol_hash.clone(),
            candidate_hash: input.candidate.candidate_hash.clone(),
            scope: input.candidate.scope,
            feature_dataset_id: input.feature_dataset.dataset_id.clone(),
            feature_plan_hash: input.feature_dataset.feature_plan_hash.clone(),
            market_data_snapshot_id: input.feature_dataset.market_data_snapshot_id.clone(),
            point_in_time_universe_id: input.feature_dataset.point_in_time_universe_id.clone(),
            observation_range: Some(input.protocol.observation_range.clone()),
            market_context: input.protocol.market_context.clone(),
            output_names,
            observation_count: rows.len() as u64,
            payload_sha256,
            engine_identity: input.protocol.engine_identity.clone(),
        };
        manifest.dataset_id = manifest.content_id()?;
        let dataset = FactorDataset { manifest, rows };
        dataset.validate()?;
        Ok(dataset)
    }

    /// Replays a generated Factor package through the same checked Custom
    /// runtime path used by research materialization. The returned rows keep
    /// the exact frozen input identities and output semantics; the caller
    /// supplies the package's frozen build provenance for validation.
    pub fn replay_component_package(
        input: FactorMaterializationInput<'_>,
        package: &ComponentPackage,
        build: &crate::CandidateBuildProvenance,
    ) -> Result<Vec<FactorDatasetRow>, MaterializationError> {
        let candidate = FactorCandidate::freeze(FactorCandidateDraft {
            candidate_id: input.candidate.candidate_id,
            revision: input.candidate.revision,
            scope: input.candidate.scope,
            feature_slots: input.candidate.feature_slots.clone(),
            parameters: input.candidate.parameters.clone(),
            outputs: input.candidate.outputs.clone(),
            source: FactorCandidateSource::Custom {
                build: build.clone(),
            },
        })?;
        let protocol = FactorMaterializationProtocol::freeze(FactorMaterializationProtocolDraft {
            protocol_id: input.protocol.protocol_id,
            user_id: input.protocol.user_id,
            candidate_hash: candidate.candidate_hash.clone(),
            feature_dataset_id: input.protocol.feature_dataset_id.clone(),
            feature_plan_hash: input.protocol.feature_plan_hash.clone(),
            parameters: input.protocol.parameters.clone(),
            market_data_snapshot_id: input.protocol.market_data_snapshot_id.clone(),
            point_in_time_universe_id: input.protocol.point_in_time_universe_id.clone(),
            observation_range: input.protocol.observation_range.clone(),
            market_context: input.protocol.market_context.clone(),
            engine_identity: input.protocol.engine_identity.clone(),
            seed: input.protocol.seed,
        })?;
        Ok(Self::materialize(FactorMaterializationInput {
            candidate: &candidate,
            protocol: &protocol,
            feature_dataset: input.feature_dataset,
            point_in_time_universe: input.point_in_time_universe,
            custom_package: Some(package),
        })?
        .rows)
    }
}

fn materialize_declarative_time_series(
    definition: &crate::DeclarativeFactorDefinition,
    outputs: &[String],
    rows: &[FeatureDatasetRow],
) -> Result<Vec<FactorDatasetRow>, MaterializationError> {
    let bindings = definition
        .outputs
        .iter()
        .map(|binding| (binding.output_name.clone(), binding.feature_slot.clone()))
        .collect::<BTreeMap<_, _>>();
    if outputs.iter().any(|output| !bindings.contains_key(output)) {
        return Err(MaterializationError::Invalid(
            "Declarative Factor definition does not cover every output".into(),
        ));
    }
    rows.iter()
        .map(|row| {
            let values = outputs
                .iter()
                .map(|output| {
                    let slot = bindings.get(output).ok_or_else(|| {
                        MaterializationError::Invalid("missing Declarative output binding".into())
                    })?;
                    let cell = row.values.get(slot).ok_or_else(|| {
                        MaterializationError::Invalid("Declarative Feature Slot is absent".into())
                    })?;
                    Ok((output.clone(), factor_cell(cell)))
                })
                .collect::<Result<BTreeMap<_, _>, MaterializationError>>()?;
            Ok(FactorDatasetRow {
                instrument_id: row.instrument_id.clone(),
                observation_time_ms: row.observation_time_ms,
                values,
            })
        })
        .collect()
}

fn materialize_declarative_cross_sectional(
    definition: &crate::DeclarativeFactorDefinition,
    outputs: &[String],
    rows: &[FeatureDatasetRow],
    universe: &[String],
) -> Result<Vec<FactorDatasetRow>, MaterializationError> {
    validate_universe(universe)?;
    let rows = materialize_declarative_time_series(definition, outputs, rows)?;
    validate_cross_sectional_dataset_rows(&rows, universe)?;
    Ok(rows)
}

fn materialize_python_cross_sectional(
    binding: &crate::PythonFactorBinding,
    parameters: &[FactorParameterValue],
    slots: &[String],
    outputs: &[String],
    rows: &[FeatureDatasetRow],
    universe: &[String],
) -> Result<Vec<FactorDatasetRow>, MaterializationError> {
    binding.validate()?;
    if slots != ["close"] || outputs != ["momentum-score"] || parameters.len() != 1 {
        return Err(MaterializationError::Invalid(
            "Python momentum Factor requires the canonical close and momentum-score contract"
                .into(),
        ));
    }
    let lookback = match parameters.first() {
        Some(FactorParameterValue::Integer(value)) if matches!(value, 5 | 20 | 60) => {
            *value as usize
        }
        _ => {
            return Err(MaterializationError::Invalid(
                "Python momentum Factor lookback parameter is invalid".into(),
            ));
        }
    };
    validate_universe(universe)?;
    let mut by_instrument = BTreeMap::<String, Vec<&FeatureDatasetRow>>::new();
    for row in rows {
        by_instrument
            .entry(row.instrument_id.clone())
            .or_default()
            .push(row);
    }
    if by_instrument
        .keys()
        .any(|instrument| !universe.iter().any(|expected| expected == instrument))
    {
        return Err(MaterializationError::Invalid(
            "Python Factor input contains an instrument outside the Point-in-Time Universe".into(),
        ));
    }
    let expected_times = by_instrument
        .values()
        .next()
        .map(|rows| {
            rows.iter()
                .map(|row| row.observation_time_ms)
                .collect::<Vec<_>>()
        })
        .ok_or_else(|| MaterializationError::Invalid("Python Factor input is empty".into()))?;
    if by_instrument.values().any(|instrument_rows| {
        instrument_rows
            .windows(2)
            .any(|rows| rows[0].observation_time_ms >= rows[1].observation_time_ms)
            || instrument_rows
                .iter()
                .map(|row| row.observation_time_ms)
                .collect::<Vec<_>>()
                != expected_times
    }) {
        return Err(MaterializationError::Invalid(
            "Python Factor input must preserve one complete ordered Universe panel".into(),
        ));
    }
    let mut returns = BTreeMap::<i64, BTreeMap<String, (Option<f64>, i64)>>::new();
    for instrument in universe {
        let instrument_rows = by_instrument.get(instrument).ok_or_else(|| {
            MaterializationError::Invalid(
                "Python Factor requires complete Point-in-Time Universe membership".into(),
            )
        })?;
        for (index, row) in instrument_rows.iter().enumerate() {
            let cell = row.values.get("close").ok_or_else(|| {
                MaterializationError::Invalid("Python Factor close slot is absent".into())
            })?;
            let Some(previous) = index
                .checked_sub(lookback)
                .and_then(|index| instrument_rows.get(index))
            else {
                returns
                    .entry(row.observation_time_ms)
                    .or_default()
                    .insert(instrument.clone(), (None, row.observation_time_ms));
                continue;
            };
            let current = match cell {
                FeatureDatasetCell::Available {
                    value,
                    available_at_ms,
                } => (*value, *available_at_ms),
                FeatureDatasetCell::Unavailable { .. } => {
                    returns
                        .entry(row.observation_time_ms)
                        .or_default()
                        .insert(instrument.clone(), (None, row.observation_time_ms));
                    continue;
                }
            };
            let prior = match previous.values.get("close") {
                Some(FeatureDatasetCell::Available {
                    value,
                    available_at_ms,
                }) => (*value, *available_at_ms),
                _ => {
                    returns
                        .entry(row.observation_time_ms)
                        .or_default()
                        .insert(instrument.clone(), (None, row.observation_time_ms));
                    continue;
                }
            };
            if !current.0.is_finite() || !prior.0.is_finite() || prior.0 == 0.0 {
                returns
                    .entry(row.observation_time_ms)
                    .or_default()
                    .insert(instrument.clone(), (None, row.observation_time_ms));
                continue;
            }
            let value = current.0 / prior.0 - 1.0;
            if !value.is_finite() {
                return Err(MaterializationError::Invalid(
                    "Python Factor produced a non-finite momentum value".into(),
                ));
            }
            returns
                .entry(row.observation_time_ms)
                .or_default()
                .insert(instrument.clone(), (Some(value), current.1.max(prior.1)));
        }
    }
    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
        let time_returns = returns.get(&row.observation_time_ms).ok_or_else(|| {
            MaterializationError::Invalid("Python Factor return identity is incomplete".into())
        })?;
        let available = time_returns
            .values()
            .filter_map(|(value, _)| *value)
            .collect::<Vec<_>>();
        let current = time_returns.get(&row.instrument_id).ok_or_else(|| {
            MaterializationError::Invalid("Python Factor return identity is incomplete".into())
        })?;
        let value = current.0.map(|value| {
            let less_or_equal = available.iter().filter(|other| **other <= value).count();
            less_or_equal as f64 / available.len().max(1) as f64
        });
        let observation = match value {
            Some(value) if value.is_finite() => FactorObservationValue::Available {
                value,
                available_at_ms: current.1,
            },
            Some(_) => {
                return Err(MaterializationError::Invalid(
                    "Python Factor percentile is non-finite".into(),
                ));
            }
            None => FactorObservationValue::Unavailable {
                reason: if row.observation_time_ms
                    < rows
                        .iter()
                        .filter(|candidate| candidate.instrument_id == row.instrument_id)
                        .nth(lookback)
                        .map(|candidate| candidate.observation_time_ms)
                        .unwrap_or(i64::MAX)
                {
                    crate::FactorUnavailabilityReason::Warmup
                } else {
                    crate::FactorUnavailabilityReason::MissingInput
                },
            },
        };
        result.push(FactorDatasetRow {
            instrument_id: row.instrument_id.clone(),
            observation_time_ms: row.observation_time_ms,
            values: BTreeMap::from([("momentum-score".into(), observation)]),
        });
    }
    Ok(result)
}

fn materialize_custom_time_series(
    package: &ComponentPackage,
    policy: crate::FactorResourcePolicy,
    parameters: &[FactorParameterValue],
    slots: &[String],
    outputs: &[String],
    rows: &[FeatureDatasetRow],
) -> Result<Vec<FactorDatasetRow>, MaterializationError> {
    let mut result = Vec::with_capacity(rows.len());
    let mut segment = Vec::new();
    let mut segment_indices = Vec::new();
    let mut current_instrument = None::<String>;
    for row in rows {
        if current_instrument
            .as_deref()
            .is_some_and(|instrument| instrument != row.instrument_id)
        {
            flush_time_series_segment(
                package,
                policy,
                parameters,
                slots,
                outputs,
                &segment,
                &segment_indices,
                &mut result,
            )?;
            segment.clear();
            segment_indices.clear();
        }
        current_instrument = Some(row.instrument_id.clone());
        let cells = slots
            .iter()
            .map(|slot| {
                row.values.get(slot).ok_or_else(|| {
                    MaterializationError::Invalid("Factor Feature Slot is absent".into())
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        if cells
            .iter()
            .all(|cell| matches!(cell, FeatureDatasetCell::Available { .. }))
        {
            let values = cells
                .iter()
                .map(|cell| match cell {
                    FeatureDatasetCell::Available { value, available_at_ms } => {
                        factor_time_series_abi::exports::adaq::factor::time_series_api::FeatureValue {
                            value: *value,
                            available_at_ms: *available_at_ms,
                        }
                    }
                    FeatureDatasetCell::Unavailable { .. } => unreachable!(),
                })
                .collect();
            segment.push(
                factor_time_series_abi::exports::adaq::factor::time_series_api::TimeSeriesRow {
                    instrument_id: row.instrument_id.clone(),
                    observation_time_ms: row.observation_time_ms,
                    slots: values,
                },
            );
            let available_at_ms = cells
                .iter()
                .filter_map(|cell| match cell {
                    FeatureDatasetCell::Available {
                        available_at_ms, ..
                    } => Some(*available_at_ms),
                    FeatureDatasetCell::Unavailable { .. } => None,
                })
                .max()
                .unwrap_or(row.observation_time_ms);
            segment_indices.push((
                row.instrument_id.clone(),
                row.observation_time_ms,
                available_at_ms,
            ));
        } else {
            let reason = cells
                .iter()
                .filter_map(|cell| match cell {
                    FeatureDatasetCell::Unavailable { reason } => Some(*reason),
                    FeatureDatasetCell::Available { .. } => None,
                })
                .min_by_key(|reason| reason.code());
            if reason == Some(FeatureUnavailabilityReason::BarGap) {
                flush_time_series_segment(
                    package,
                    policy,
                    parameters,
                    slots,
                    outputs,
                    &segment,
                    &segment_indices,
                    &mut result,
                )?;
                segment.clear();
                segment_indices.clear();
            }
            result.push(unavailable_row(
                row,
                outputs,
                map_feature_reason(
                    reason.unwrap_or(FeatureUnavailabilityReason::MissingDependency),
                ),
            ));
        }
    }
    flush_time_series_segment(
        package,
        policy,
        parameters,
        slots,
        outputs,
        &segment,
        &segment_indices,
        &mut result,
    )?;
    Ok(result)
}

fn flush_time_series_segment(
    package: &ComponentPackage,
    policy: crate::FactorResourcePolicy,
    parameters: &[FactorParameterValue],
    slots: &[String],
    outputs: &[String],
    rows: &[factor_time_series_abi::exports::adaq::factor::time_series_api::TimeSeriesRow],
    identities: &[(String, i64, i64)],
    result: &mut Vec<FactorDatasetRow>,
) -> Result<(), MaterializationError> {
    if rows.is_empty() {
        return Ok(());
    }
    let loader = WasmLoader::with_limits(run_limits(policy)?);
    loader
        .load_factor_time_series_bytes(
            package.wasm.as_slice(),
            Vec::new(),
            &component_parameter_values(parameters),
        )
        .map_err(MaterializationError::Execution)?;
    let actual = loader
        .describe_factor()
        .map_err(MaterializationError::Execution)?;
    if actual.feature_slots != slots || actual.output_names != outputs {
        return Err(MaterializationError::Invalid(
            "Custom Factor ABI schema does not match the Candidate".into(),
        ));
    }
    let results = loader
        .process_factor(rows.to_vec())
        .map_err(MaterializationError::Execution)?;
    if results.len() != identities.len() {
        return Err(MaterializationError::Execution(
            "Custom Factor returned an incomplete Time-Series result".into(),
        ));
    }
    for (index, (identity, factor_result)) in identities.iter().zip(results).enumerate() {
        let values = factor_result.values.map(|values| {
            values
                .into_iter()
                .map(|value| NamedFactorOutput {
                    name: value.name,
                    value: value.value,
                })
                .collect()
        });
        result.push(result_row(
            identity,
            values,
            outputs,
            actual.warmup_bars as usize > index,
        )?);
    }
    Ok(())
}

fn materialize_custom_cross_sectional(
    package: &ComponentPackage,
    policy: crate::FactorResourcePolicy,
    parameters: &[FactorParameterValue],
    slots: &[String],
    outputs: &[String],
    rows: &[FeatureDatasetRow],
    universe: &[String],
) -> Result<Vec<FactorDatasetRow>, MaterializationError> {
    validate_universe(universe)?;
    let mut by_time = BTreeMap::<i64, BTreeMap<String, &FeatureDatasetRow>>::new();
    for row in rows {
        if by_time
            .entry(row.observation_time_ms)
            .or_default()
            .insert(row.instrument_id.clone(), row)
            .is_some()
        {
            return Err(MaterializationError::Invalid(
                "Cross-Sectional Feature Dataset contains duplicate Universe membership".into(),
            ));
        }
    }
    let loader = WasmLoader::with_limits(run_limits(policy)?);
    loader
        .load_factor_cross_sectional_bytes(
            package.wasm.as_slice(),
            Vec::new(),
            &component_parameter_values(parameters),
        )
        .map_err(MaterializationError::Execution)?;
    let actual = loader
        .describe_factor()
        .map_err(MaterializationError::Execution)?;
    if actual.feature_slots != slots || actual.output_names != outputs {
        return Err(MaterializationError::Invalid(
            "Custom Factor ABI schema does not match the Candidate".into(),
        ));
    }
    let mut result = Vec::with_capacity(rows.len());
    let mut batch_index = 0usize;
    for (observation_time_ms, members) in by_time {
        if universe
            .iter()
            .any(|instrument| !members.contains_key(instrument))
            || members.len() != universe.len()
        {
            return Err(MaterializationError::Invalid(
                "Cross-Sectional Factor requires the complete Point-in-Time Universe".into(),
            ));
        }
        let input_rows = universe
            .iter()
            .map(|instrument_id| {
                let row = members[instrument_id];
                let cells = slots
                    .iter()
                    .map(|slot| row.values.get(slot).ok_or_else(|| {
                        MaterializationError::Invalid("Factor Feature Slot is absent".into())
                    }))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(factor_cross_sectional_abi::exports::adaq::factor::cross_sectional_api::CrossSectionalRow {
                    instrument_id: instrument_id.clone(),
                    observation_time_ms,
                    slots: cells.into_iter().map(factor_cell_to_abi).collect::<Result<_, _>>()?,
                })
            })
            .collect::<Result<Vec<_>, MaterializationError>>()?;
        let availability = universe
            .iter()
            .map(|instrument_id| {
                let row = members[instrument_id];
                let available_at_ms = slots
                    .iter()
                    .filter_map(|slot| match row.values.get(slot) {
                        Some(FeatureDatasetCell::Available {
                            available_at_ms, ..
                        }) => Some(*available_at_ms),
                        _ => None,
                    })
                    .max()
                    .unwrap_or(observation_time_ms);
                (instrument_id.clone(), available_at_ms)
            })
            .collect::<BTreeMap<_, _>>();
        let factor_results = loader
            .process_cross_sectional_factor(input_rows, universe)
            .map_err(MaterializationError::Execution)?;
        for factor_result in factor_results {
            let values = factor_result.values.map(|values| {
                values
                    .into_iter()
                    .map(|value| NamedFactorOutput {
                        name: value.name,
                        value: value.value,
                    })
                    .collect()
            });
            result.push(result_row(
                &(
                    factor_result.instrument_id.clone(),
                    factor_result.observation_time_ms,
                    availability[&factor_result.instrument_id],
                ),
                values,
                outputs,
                actual.warmup_bars as usize > batch_index,
            )?);
        }
        batch_index = batch_index.saturating_add(1);
    }
    Ok(result)
}

fn result_row(
    identity: &(String, i64, i64),
    values: Option<Vec<NamedFactorOutput>>,
    outputs: &[String],
    in_warmup: bool,
) -> Result<FactorDatasetRow, MaterializationError> {
    let values = match values {
        Some(values) => values
            .into_iter()
            .map(|value| {
                (
                    value.name,
                    FactorObservationValue::Available {
                        value: value.value,
                        available_at_ms: identity.2,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>(),
        None => outputs
            .iter()
            .map(|output| {
                (
                    output.clone(),
                    FactorObservationValue::Unavailable {
                        reason: if in_warmup {
                            crate::FactorUnavailabilityReason::Warmup
                        } else {
                            crate::FactorUnavailabilityReason::InsufficientCoverage
                        },
                    },
                )
            })
            .collect(),
    };
    Ok(FactorDatasetRow {
        instrument_id: identity.0.clone(),
        observation_time_ms: identity.1,
        values,
    })
}

fn validate_custom_package(
    package: &ComponentPackage,
    build: &crate::CandidateBuildProvenance,
    scope: FactorScope,
) -> Result<(), MaterializationError> {
    verify_package(package).map_err(MaterializationError::Execution)?;
    if package.archive_sha256 != build.package_sha256
        || package.manifest.kind != ComponentKind::Factor
        || package.manifest.abi_version.to_string() != build.abi_version
        || package.manifest.factor_scope
            != Some(match scope {
                FactorScope::TimeSeries => adaq_component_tooling::FactorScope::TimeSeries,
                FactorScope::CrossSectional => adaq_component_tooling::FactorScope::CrossSectional,
            })
    {
        return Err(MaterializationError::Invalid(
            "Custom Candidate package does not match its frozen build or scope".into(),
        ));
    }
    Ok(())
}

fn unavailable_row(
    row: &FeatureDatasetRow,
    outputs: &[String],
    reason: crate::FactorUnavailabilityReason,
) -> FactorDatasetRow {
    FactorDatasetRow {
        instrument_id: row.instrument_id.clone(),
        observation_time_ms: row.observation_time_ms,
        values: outputs
            .iter()
            .map(|output| {
                (
                    output.clone(),
                    FactorObservationValue::Unavailable { reason },
                )
            })
            .collect(),
    }
}

fn factor_cell(cell: &FeatureDatasetCell) -> FactorObservationValue {
    match cell {
        FeatureDatasetCell::Available {
            value,
            available_at_ms,
        } => FactorObservationValue::Available {
            value: *value,
            available_at_ms: *available_at_ms,
        },
        FeatureDatasetCell::Unavailable { reason } => FactorObservationValue::Unavailable {
            reason: map_feature_reason(*reason),
        },
    }
}

fn factor_cell_to_abi(
    cell: &FeatureDatasetCell,
) -> Result<
    factor_cross_sectional_abi::exports::adaq::factor::cross_sectional_api::FeatureCell,
    MaterializationError,
> {
    Ok(match cell {
        FeatureDatasetCell::Available { value, available_at_ms } => {
            factor_cross_sectional_abi::exports::adaq::factor::cross_sectional_api::FeatureCell::Available(
                factor_cross_sectional_abi::exports::adaq::factor::cross_sectional_api::FeatureValue {
                    value: *value,
                    available_at_ms: *available_at_ms,
                },
            )
        }
        FeatureDatasetCell::Unavailable { reason } => {
            factor_cross_sectional_abi::exports::adaq::factor::cross_sectional_api::FeatureCell::Unavailable(
                map_reason_to_cs(*reason),
            )
        }
    })
}

fn map_feature_reason(reason: FeatureUnavailabilityReason) -> crate::FactorUnavailabilityReason {
    match reason {
        FeatureUnavailabilityReason::Warmup => crate::FactorUnavailabilityReason::Warmup,
        FeatureUnavailabilityReason::BarGap => crate::FactorUnavailabilityReason::BarGap,
        FeatureUnavailabilityReason::MissingMarketInput => {
            crate::FactorUnavailabilityReason::MissingInput
        }
        FeatureUnavailabilityReason::MissingDependency => {
            crate::FactorUnavailabilityReason::MissingDependency
        }
        FeatureUnavailabilityReason::UnknownUniverse => {
            crate::FactorUnavailabilityReason::UnknownUniverse
        }
        FeatureUnavailabilityReason::InsufficientCoverage => {
            crate::FactorUnavailabilityReason::InsufficientCoverage
        }
        FeatureUnavailabilityReason::UndefinedArithmetic => {
            crate::FactorUnavailabilityReason::UndefinedArithmetic
        }
        FeatureUnavailabilityReason::ArtifactMissingInstrument
        | FeatureUnavailabilityReason::CorporateActionUnavailable => {
            crate::FactorUnavailabilityReason::InvalidUpstream
        }
    }
}

fn map_reason_to_cs(
    reason: FeatureUnavailabilityReason,
) -> factor_cross_sectional_abi::exports::adaq::factor::cross_sectional_api::UnavailabilityReason {
    use factor_cross_sectional_abi::exports::adaq::factor::cross_sectional_api::UnavailabilityReason;
    match map_feature_reason(reason) {
        crate::FactorUnavailabilityReason::Warmup => UnavailabilityReason::Warmup,
        crate::FactorUnavailabilityReason::BarGap => UnavailabilityReason::BarGap,
        crate::FactorUnavailabilityReason::MissingInput => UnavailabilityReason::MissingInput,
        crate::FactorUnavailabilityReason::MissingDependency => {
            UnavailabilityReason::MissingDependency
        }
        crate::FactorUnavailabilityReason::UnknownUniverse => UnavailabilityReason::UnknownUniverse,
        crate::FactorUnavailabilityReason::InsufficientCoverage => {
            UnavailabilityReason::InsufficientCoverage
        }
        crate::FactorUnavailabilityReason::UndefinedArithmetic => {
            UnavailabilityReason::UndefinedArithmetic
        }
        crate::FactorUnavailabilityReason::NotYetAvailable => UnavailabilityReason::NotYetAvailable,
        crate::FactorUnavailabilityReason::InvalidUpstream => UnavailabilityReason::InvalidUpstream,
    }
}

fn validate_universe(universe: &[String]) -> Result<(), MaterializationError> {
    let mut seen = HashSet::new();
    if universe.is_empty()
        || universe
            .iter()
            .any(|instrument| instrument.trim().is_empty() || !seen.insert(instrument))
    {
        return Err(MaterializationError::Invalid(
            "Point-in-Time Universe membership must be complete, unique, and non-empty".into(),
        ));
    }
    Ok(())
}

fn validate_cross_sectional_dataset_rows(
    rows: &[FactorDatasetRow],
    universe: &[String],
) -> Result<(), MaterializationError> {
    validate_universe(universe)?;
    let mut by_time = BTreeMap::<i64, HashSet<&str>>::new();
    for row in rows {
        by_time
            .entry(row.observation_time_ms)
            .or_default()
            .insert(row.instrument_id.as_str());
    }
    if by_time.values().any(|members| {
        members.len() != universe.len() || universe.iter().any(|id| !members.contains(id.as_str()))
    }) {
        return Err(MaterializationError::Invalid(
            "Cross-Sectional Factor Dataset does not preserve complete Universe membership".into(),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactorMaterializationStart {
    pub attempt: FactorMaterializationAttempt,
    pub reused_dataset_id: Option<String>,
}

#[derive(Clone, Default)]
pub struct FactorDatasetPublisher {
    state: Arc<Mutex<PublisherState>>,
}

#[derive(Default)]
struct PublisherState {
    attempts: HashMap<Uuid, FactorMaterializationAttempt>,
    protocols: HashMap<(Uuid, String), FactorMaterializationProtocol>,
    active: HashMap<(Uuid, String), Uuid>,
    retry_sources: HashSet<Uuid>,
    completed: HashMap<(Uuid, String), String>,
    staging: HashMap<Uuid, FactorDataset>,
    datasets: HashMap<String, FactorDataset>,
    access: HashMap<String, BTreeSet<Uuid>>,
    references: HashSet<(String, Uuid, String)>,
}

impl FactorDatasetPublisher {
    pub fn start(
        &self,
        protocol: &FactorMaterializationProtocol,
    ) -> Result<FactorMaterializationStart, MaterializationError> {
        protocol.validate()?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| MaterializationError::Invalid("publisher mutex poisoned".into()))?;
        let key = (protocol.user_id, protocol.protocol_hash.clone());
        if let Some(attempt_id) = state.active.get(&key).copied() {
            return Ok(FactorMaterializationStart {
                attempt: state.attempts[&attempt_id].clone(),
                reused_dataset_id: None,
            });
        }
        if let Some(dataset_id) = state.completed.get(&key).cloned() {
            let mut attempt = FactorMaterializationAttempt::new(
                Uuid::new_v4(),
                protocol.user_id,
                protocol.protocol_hash.clone(),
            )?;
            attempt.transition(AttemptStatus::Running)?;
            attempt.transition(AttemptStatus::Completed)?;
            attempt.completed_units = state.datasets[&dataset_id].rows.len() as u64;
            attempt.source_attempt_id = None;
            state.attempts.insert(attempt.attempt_id, attempt.clone());
            state.protocols.insert(key, protocol.clone());
            return Ok(FactorMaterializationStart {
                attempt,
                reused_dataset_id: Some(dataset_id),
            });
        }
        let attempt = FactorMaterializationAttempt::new(
            Uuid::new_v4(),
            protocol.user_id,
            protocol.protocol_hash.clone(),
        )?;
        state.active.insert(key, attempt.attempt_id);
        state.attempts.insert(attempt.attempt_id, attempt.clone());
        state.protocols.insert(
            (protocol.user_id, protocol.protocol_hash.clone()),
            protocol.clone(),
        );
        Ok(FactorMaterializationStart {
            attempt,
            reused_dataset_id: None,
        })
    }

    pub fn begin(
        &self,
        user_id: Uuid,
        attempt_id: Uuid,
    ) -> Result<FactorMaterializationAttempt, MaterializationError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| MaterializationError::Invalid("publisher mutex poisoned".into()))?;
        let attempt = state
            .attempts
            .get_mut(&attempt_id)
            .ok_or(MaterializationError::AttemptNotFound)?;
        if attempt.user_id != user_id {
            return Err(MaterializationError::Unauthorized);
        }
        match attempt.status {
            AttemptStatus::Pending => attempt
                .transition(AttemptStatus::Running)
                .map_err(MaterializationError::from)?,
            AttemptStatus::Running => {}
            AttemptStatus::Completed
            | AttemptStatus::Failed
            | AttemptStatus::Cancelled
            | AttemptStatus::Interrupted
            | AttemptStatus::Stale => {
                return Err(MaterializationError::InvalidTransition);
            }
        }
        Ok(attempt.clone())
    }

    pub fn stage(
        &self,
        user_id: Uuid,
        attempt_id: Uuid,
        dataset: FactorDataset,
    ) -> Result<(), MaterializationError> {
        dataset.validate()?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| MaterializationError::Invalid("publisher mutex poisoned".into()))?;
        let (attempt_user_id, protocol_hash, status) = {
            let attempt = state
                .attempts
                .get(&attempt_id)
                .ok_or(MaterializationError::AttemptNotFound)?;
            (
                attempt.user_id,
                attempt.protocol_hash.clone(),
                attempt.status,
            )
        };
        if status != AttemptStatus::Running {
            return Err(MaterializationError::InvalidTransition);
        }
        if attempt_user_id != user_id {
            return Err(MaterializationError::Unauthorized);
        }
        let protocol = state
            .protocols
            .get(&(attempt_user_id, protocol_hash.clone()))
            .ok_or(MaterializationError::AttemptNotFound)?;
        if dataset.manifest.protocol_hash != protocol.protocol_hash
            || dataset.manifest.candidate_hash != protocol.candidate_hash
            || dataset.manifest.feature_dataset_id != protocol.feature_dataset_id
        {
            return Err(MaterializationError::Invalid(
                "staged Dataset does not match the exact Attempt Protocol bindings".into(),
            ));
        }
        state.staging.insert(attempt_id, dataset);
        Ok(())
    }

    pub fn publish(
        &self,
        user_id: Uuid,
        attempt_id: Uuid,
    ) -> Result<FactorMaterializationAttempt, MaterializationError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| MaterializationError::Invalid("publisher mutex poisoned".into()))?;
        let (attempt_user_id, completed_protocol_hash, status) = {
            let attempt = state
                .attempts
                .get(&attempt_id)
                .ok_or(MaterializationError::AttemptNotFound)?;
            (
                attempt.user_id,
                attempt.protocol_hash.clone(),
                attempt.status,
            )
        };
        if attempt_user_id != user_id {
            return Err(MaterializationError::Unauthorized);
        }
        if status != AttemptStatus::Running {
            return Err(MaterializationError::InvalidTransition);
        }
        let dataset =
            state
                .staging
                .get(&attempt_id)
                .cloned()
                .ok_or(MaterializationError::Invalid(
                    "Factor Dataset staging is missing".into(),
                ))?;
        if let Some(existing) = state.datasets.get(&dataset.manifest.dataset_id)
            && existing.manifest.payload_sha256 != dataset.manifest.payload_sha256
        {
            return Err(MaterializationError::Invalid(
                "Factor Dataset content collision".into(),
            ));
        }
        let dataset_id = dataset.manifest.dataset_id.clone();
        state.datasets.entry(dataset_id.clone()).or_insert(dataset);
        state.staging.remove(&attempt_id);
        state
            .access
            .entry(dataset_id.clone())
            .or_default()
            .insert(user_id);
        let completed_units = state.datasets[&dataset_id].rows.len() as u64;
        let attempt = state
            .attempts
            .get_mut(&attempt_id)
            .ok_or(MaterializationError::AttemptNotFound)?;
        attempt.completed_units = completed_units;
        attempt
            .transition(AttemptStatus::Completed)
            .map_err(MaterializationError::from)?;
        let completed_attempt = attempt.clone();
        state.completed.insert(
            (user_id, completed_protocol_hash.clone()),
            dataset_id.clone(),
        );
        state.active.retain(|(_, protocol_hash), active_id| {
            !(*active_id == attempt_id && *protocol_hash == completed_protocol_hash)
        });
        Ok(completed_attempt)
    }

    pub fn fail(
        &self,
        user_id: Uuid,
        attempt_id: Uuid,
        diagnostic: impl Into<String>,
    ) -> Result<FactorMaterializationAttempt, MaterializationError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| MaterializationError::Invalid("publisher mutex poisoned".into()))?;
        let failed_attempt = {
            let attempt = state
                .attempts
                .get_mut(&attempt_id)
                .ok_or(MaterializationError::AttemptNotFound)?;
            if attempt.user_id != user_id {
                return Err(MaterializationError::Unauthorized);
            }
            attempt.diagnostic = Some(safe_diagnostic(&diagnostic.into()));
            attempt
                .transition(AttemptStatus::Failed)
                .map_err(MaterializationError::from)?;
            attempt.clone()
        };
        state.staging.remove(&attempt_id);
        state.active.retain(|_, active_id| *active_id != attempt_id);
        Ok(failed_attempt)
    }

    pub fn cancel(
        &self,
        user_id: Uuid,
        attempt_id: Uuid,
    ) -> Result<FactorMaterializationAttempt, MaterializationError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| MaterializationError::Invalid("publisher mutex poisoned".into()))?;
        let cancelled_attempt = {
            let attempt = state
                .attempts
                .get_mut(&attempt_id)
                .ok_or(MaterializationError::AttemptNotFound)?;
            if attempt.user_id != user_id {
                return Err(MaterializationError::Unauthorized);
            }
            attempt.diagnostic = Some("Factor Materialization cancelled".into());
            attempt
                .transition(AttemptStatus::Cancelled)
                .map_err(MaterializationError::from)?;
            attempt.clone()
        };
        state.staging.remove(&attempt_id);
        state.active.retain(|_, active_id| *active_id != attempt_id);
        Ok(cancelled_attempt)
    }

    pub fn retry(
        &self,
        user_id: Uuid,
        attempt_id: Uuid,
    ) -> Result<FactorMaterializationAttempt, MaterializationError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| MaterializationError::Invalid("publisher mutex poisoned".into()))?;
        let previous = state
            .attempts
            .get(&attempt_id)
            .ok_or(MaterializationError::AttemptNotFound)?
            .clone();
        if previous.user_id != user_id {
            return Err(MaterializationError::Unauthorized);
        }
        if let Some(active_id) = state
            .active
            .get(&(previous.user_id, previous.protocol_hash.clone()))
            .copied()
        {
            return Ok(state.attempts[&active_id].clone());
        }
        let retry = previous
            .retry(Uuid::new_v4())
            .map_err(MaterializationError::from)?;
        if !state.retry_sources.insert(attempt_id) {
            return Err(MaterializationError::InvalidTransition);
        }
        state.attempts.insert(retry.attempt_id, retry.clone());
        state.active.insert(
            (retry.user_id, retry.protocol_hash.clone()),
            retry.attempt_id,
        );
        Ok(retry)
    }

    pub fn dataset(
        &self,
        user_id: Uuid,
        dataset_id: &str,
    ) -> Result<FactorDataset, MaterializationError> {
        let state = self
            .state
            .lock()
            .map_err(|_| MaterializationError::Invalid("publisher mutex poisoned".into()))?;
        if !state
            .access
            .get(dataset_id)
            .is_some_and(|users| users.contains(&user_id))
        {
            return Err(MaterializationError::Unauthorized);
        }
        state
            .datasets
            .get(dataset_id)
            .cloned()
            .ok_or(MaterializationError::AttemptNotFound)
    }

    pub fn reference(
        &self,
        owner_user_id: Uuid,
        dataset_id: &str,
        referencing_user_id: Uuid,
        reference_id: impl Into<String>,
    ) -> Result<(), MaterializationError> {
        let reference_id = reference_id.into();
        if owner_user_id.is_nil() || referencing_user_id.is_nil() || reference_id.trim().is_empty()
        {
            return Err(MaterializationError::Unauthorized);
        }
        if owner_user_id != referencing_user_id {
            return Err(MaterializationError::Unauthorized);
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| MaterializationError::Invalid("publisher mutex poisoned".into()))?;
        if !state
            .access
            .get(dataset_id)
            .is_some_and(|users| users.contains(&owner_user_id))
        {
            return Err(MaterializationError::Unauthorized);
        }
        state
            .access
            .entry(dataset_id.into())
            .or_default()
            .insert(referencing_user_id);
        state
            .references
            .insert((dataset_id.into(), referencing_user_id, reference_id));
        Ok(())
    }

    pub fn unreference(
        &self,
        referencing_user_id: Uuid,
        dataset_id: &str,
        reference_id: &str,
    ) -> Result<(), MaterializationError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| MaterializationError::Invalid("publisher mutex poisoned".into()))?;
        if !state
            .references
            .remove(&(dataset_id.into(), referencing_user_id, reference_id.into()))
        {
            return Err(MaterializationError::AttemptNotFound);
        }
        Ok(())
    }

    pub fn delete(&self, user_id: Uuid, dataset_id: &str) -> Result<(), MaterializationError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| MaterializationError::Invalid("publisher mutex poisoned".into()))?;
        if user_id.is_nil() {
            return Err(MaterializationError::Unauthorized);
        }
        if !state
            .access
            .get(dataset_id)
            .is_some_and(|users| users.contains(&user_id))
        {
            return Err(MaterializationError::Unauthorized);
        }
        if state.references.iter().any(|(id, _, _)| id == dataset_id) {
            return Err(MaterializationError::DatasetReferenced);
        }
        let users = state
            .access
            .get_mut(dataset_id)
            .expect("access was checked");
        users.remove(&user_id);
        if users.is_empty() {
            state.access.remove(dataset_id);
            state.datasets.remove(dataset_id);
            state.completed.retain(|_, id| id != dataset_id);
        }
        Ok(())
    }
}

fn safe_diagnostic(message: &str) -> String {
    let mut diagnostic = message
        .lines()
        .map(|line| {
            line.replace("/Users/", "<private>/")
                .replace("/home/", "<private>/")
                .replace("C:\\Users\\", "<private>\\")
        })
        .collect::<Vec<_>>()
        .join("\n");
    if diagnostic.len() > MAX_DIAGNOSTIC_BYTES {
        diagnostic.truncate(MAX_DIAGNOSTIC_BYTES);
        diagnostic.push('…');
    }
    diagnostic
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use crate::FactorUnavailabilityReason;

    fn row(instrument_id: &str, time: i64, value: f64) -> FeatureDatasetRow {
        FeatureDatasetRow {
            instrument_id: instrument_id.into(),
            observation_time_ms: time,
            values: BTreeMap::from([(
                "signal".into(),
                FeatureDatasetCell::Available {
                    value,
                    available_at_ms: time,
                },
            )]),
        }
    }

    fn close_row(instrument_id: &str, time: i64, value: f64) -> FeatureDatasetRow {
        FeatureDatasetRow {
            instrument_id: instrument_id.into(),
            observation_time_ms: time,
            values: BTreeMap::from([(
                "close".into(),
                FeatureDatasetCell::Available {
                    value,
                    available_at_ms: time,
                },
            )]),
        }
    }

    fn unavailable_row(
        instrument_id: &str,
        time: i64,
        reason: FeatureUnavailabilityReason,
    ) -> FeatureDatasetRow {
        FeatureDatasetRow {
            instrument_id: instrument_id.into(),
            observation_time_ms: time,
            values: BTreeMap::from([("signal".into(), FeatureDatasetCell::Unavailable { reason })]),
        }
    }

    fn engine() -> crate::ResearchEngineProvenance {
        crate::ResearchEngineProvenance {
            engine_id: "adaq-native".into(),
            engine_version: "1.0.0".into(),
            adapter: "native".into(),
            target_triple: "test".into(),
            build_id: "build".into(),
            environment: BTreeMap::new(),
            parameters: BTreeMap::new(),
            input_identities: vec!["input".into()],
        }
    }

    fn context() -> crate::FactorMarketContext {
        crate::FactorMarketContext {
            venue: "OKX".into(),
            asset_class: "crypto".into(),
            bar_interval: "1h".into(),
            price_basis: "unadjusted".into(),
            valuation_currency: "USDT".into(),
            point_in_time_universe_id: "universe".into(),
        }
    }

    fn test_plan() -> FeaturePlan {
        FeaturePlan::freeze(adaq_feature_engine::FeaturePlanDraft {
            slots: vec![adaq_feature_engine::FeatureSlot {
                name: "signal".into(),
                source: adaq_feature_engine::FeatureSource::Market {
                    field: adaq_feature_engine::MarketField::Close,
                },
                warmup_bars: 0,
            }],
            engine_identity: FeatureEngineIdentity::for_tests(),
            ..Default::default()
        })
        .unwrap()
    }

    fn empty_factor_dataset(protocol: &crate::FactorMaterializationProtocol) -> FactorDataset {
        let output_names = vec!["score".into()];
        let payload_sha256 = payload_hash(&DatasetPayload {
            output_names: &output_names,
            rows: &[],
        })
        .unwrap();
        let mut manifest = FactorDatasetManifest {
            schema_version: crate::FACTOR_RESEARCH_SCHEMA_VERSION.into(),
            dataset_id: String::new(),
            protocol_hash: protocol.protocol_hash.clone(),
            candidate_hash: protocol.candidate_hash.clone(),
            scope: crate::FactorScope::TimeSeries,
            feature_dataset_id: protocol.feature_dataset_id.clone(),
            feature_plan_hash: protocol.feature_plan_hash.clone(),
            market_data_snapshot_id: protocol.market_data_snapshot_id.clone(),
            point_in_time_universe_id: protocol.point_in_time_universe_id.clone(),
            observation_range: Some(protocol.observation_range.clone()),
            market_context: protocol.market_context.clone(),
            output_names,
            observation_count: 0,
            payload_sha256,
            engine_identity: protocol.engine_identity.clone(),
        };
        manifest.dataset_id = manifest.content_id().unwrap();
        FactorDataset {
            manifest,
            rows: vec![],
        }
    }

    #[test]
    fn declarative_materialization_preserves_feature_missingness_and_identity() {
        let user_id = Uuid::new_v4();
        let candidate_id = Uuid::new_v4();
        let plan = test_plan();
        let plan_hash = plan.plan_hash().to_owned();
        let (candidate, _) = crate::DeclarativeFactorDraft {
            user_id,
            candidate_id,
            revision: 1,
            scope: FactorScope::TimeSeries,
            feature_slots: vec![crate::FactorFeatureSlot {
                name: "signal".into(),
            }],
            parameters: vec![],
            outputs: vec![crate::FactorOutput {
                name: "score".into(),
            }],
            definition: crate::DeclarativeFactorDefinition {
                feature_plan_hash: plan_hash.clone(),
                operator_catalog_version: adaq_feature_engine::FEATURE_OPERATOR_CATALOG_VERSION
                    .into(),
                outputs: vec![crate::DeclarativeFactorOutputBinding {
                    output_name: "score".into(),
                    feature_slot: "signal".into(),
                }],
            },
            presentation: crate::FactorPresentationMetadata {
                name: "score".into(),
                description: String::new(),
                tags: vec![],
            },
        }
        .publish()
        .unwrap();
        let feature_dataset = CompletedFeatureDataset::new(
            user_id.to_string(),
            "feature-dataset",
            plan_hash.clone(),
            plan.to_json(),
            plan.engine_identity(),
            "snapshot",
            "universe",
            vec!["signal".into()],
            vec![row("a", 1, 2.0)],
        )
        .unwrap();
        let protocol = crate::FactorMaterializationProtocol::freeze(
            crate::FactorMaterializationProtocolDraft {
                protocol_id: Uuid::new_v4(),
                user_id,
                candidate_hash: candidate.candidate_hash.clone(),
                feature_dataset_id: feature_dataset.dataset_id.clone(),
                feature_plan_hash: feature_dataset.feature_plan_hash.clone(),
                parameters: vec![],
                market_data_snapshot_id: "snapshot".into(),
                point_in_time_universe_id: "universe".into(),
                observation_range: crate::ObservationRange {
                    start_time_ms: 0,
                    end_time_ms: 2,
                },
                market_context: context(),
                engine_identity: engine(),
                seed: 1,
            },
        )
        .unwrap();
        let dataset = FactorMaterializer::materialize(FactorMaterializationInput {
            candidate: &candidate,
            protocol: &protocol,
            feature_dataset: &feature_dataset,
            point_in_time_universe: &["a".into()],
            custom_package: None,
        })
        .unwrap();
        assert_eq!(dataset.rows[0].instrument_id, "a");
        assert!(matches!(
            dataset.rows[0].values["score"],
            FactorObservationValue::Available { value: 2.0, .. }
        ));
    }

    #[test]
    fn declarative_materialization_preserves_gaps_and_is_deterministic() {
        let user_id = Uuid::new_v4();
        let plan = test_plan();
        let plan_hash = plan.plan_hash().to_owned();
        let (candidate, _) = crate::DeclarativeFactorDraft {
            user_id,
            candidate_id: Uuid::new_v4(),
            revision: 1,
            scope: FactorScope::TimeSeries,
            feature_slots: vec![crate::FactorFeatureSlot {
                name: "signal".into(),
            }],
            parameters: vec![],
            outputs: vec![crate::FactorOutput {
                name: "score".into(),
            }],
            definition: crate::DeclarativeFactorDefinition {
                feature_plan_hash: plan_hash.clone(),
                operator_catalog_version: adaq_feature_engine::FEATURE_OPERATOR_CATALOG_VERSION
                    .into(),
                outputs: vec![crate::DeclarativeFactorOutputBinding {
                    output_name: "score".into(),
                    feature_slot: "signal".into(),
                }],
            },
            presentation: crate::FactorPresentationMetadata {
                name: "score".into(),
                ..Default::default()
            },
        }
        .publish()
        .unwrap();
        let feature_dataset = CompletedFeatureDataset::new(
            user_id.to_string(),
            "feature-dataset",
            plan_hash.clone(),
            plan.to_json(),
            plan.engine_identity(),
            "snapshot",
            "universe",
            vec!["signal".into()],
            vec![
                row("a", 1, 2.0),
                unavailable_row("a", 2, FeatureUnavailabilityReason::BarGap),
                row("a", 3, 4.0),
            ],
        )
        .unwrap();
        let protocol = crate::FactorMaterializationProtocol::freeze(
            crate::FactorMaterializationProtocolDraft {
                protocol_id: Uuid::new_v4(),
                user_id,
                candidate_hash: candidate.candidate_hash.clone(),
                feature_dataset_id: feature_dataset.dataset_id.clone(),
                feature_plan_hash: feature_dataset.feature_plan_hash.clone(),
                parameters: vec![],
                market_data_snapshot_id: "snapshot".into(),
                point_in_time_universe_id: "universe".into(),
                observation_range: crate::ObservationRange {
                    start_time_ms: 1,
                    end_time_ms: 4,
                },
                market_context: context(),
                engine_identity: engine(),
                seed: 1,
            },
        )
        .unwrap();
        let input = FactorMaterializationInput {
            candidate: &candidate,
            protocol: &protocol,
            feature_dataset: &feature_dataset,
            point_in_time_universe: &["a".into()],
            custom_package: None,
        };
        let first = FactorMaterializer::materialize(input).unwrap();
        let second = FactorMaterializer::materialize(input).unwrap();
        assert_eq!(first.manifest.dataset_id, second.manifest.dataset_id);
        assert!(matches!(
            first.rows[1].values["score"],
            FactorObservationValue::Unavailable {
                reason: FactorUnavailabilityReason::BarGap
            }
        ));
    }

    #[test]
    fn cross_sectional_materialization_requires_and_orders_the_universe() {
        let user_id = Uuid::new_v4();
        let plan = test_plan();
        let plan_hash = plan.plan_hash().to_owned();
        let (candidate, _) = crate::DeclarativeFactorDraft {
            user_id,
            candidate_id: Uuid::new_v4(),
            revision: 1,
            scope: FactorScope::CrossSectional,
            feature_slots: vec![crate::FactorFeatureSlot {
                name: "signal".into(),
            }],
            parameters: vec![],
            outputs: vec![crate::FactorOutput {
                name: "score".into(),
            }],
            definition: crate::DeclarativeFactorDefinition {
                feature_plan_hash: plan_hash.clone(),
                operator_catalog_version: adaq_feature_engine::FEATURE_OPERATOR_CATALOG_VERSION
                    .into(),
                outputs: vec![crate::DeclarativeFactorOutputBinding {
                    output_name: "score".into(),
                    feature_slot: "signal".into(),
                }],
            },
            presentation: crate::FactorPresentationMetadata {
                name: "score".into(),
                ..Default::default()
            },
        }
        .publish()
        .unwrap();
        let feature_dataset = CompletedFeatureDataset::new(
            user_id.to_string(),
            "feature-dataset",
            plan_hash.clone(),
            plan.to_json(),
            plan.engine_identity(),
            "snapshot",
            "universe",
            vec!["signal".into()],
            vec![
                row("a", 1, 2.0),
                unavailable_row("b", 1, FeatureUnavailabilityReason::MissingMarketInput),
            ],
        )
        .unwrap();
        let protocol = crate::FactorMaterializationProtocol::freeze(
            crate::FactorMaterializationProtocolDraft {
                protocol_id: Uuid::new_v4(),
                user_id,
                candidate_hash: candidate.candidate_hash.clone(),
                feature_dataset_id: feature_dataset.dataset_id.clone(),
                feature_plan_hash: feature_dataset.feature_plan_hash.clone(),
                parameters: vec![],
                market_data_snapshot_id: "snapshot".into(),
                point_in_time_universe_id: "universe".into(),
                observation_range: crate::ObservationRange {
                    start_time_ms: 0,
                    end_time_ms: 2,
                },
                market_context: context(),
                engine_identity: engine(),
                seed: 1,
            },
        )
        .unwrap();
        let dataset = FactorMaterializer::materialize(FactorMaterializationInput {
            candidate: &candidate,
            protocol: &protocol,
            feature_dataset: &feature_dataset,
            point_in_time_universe: &["b".into(), "a".into()],
            custom_package: None,
        })
        .unwrap();
        assert_eq!(
            dataset
                .rows
                .iter()
                .map(|row| row.instrument_id.as_str())
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert!(matches!(
            dataset.rows[1].values["score"],
            FactorObservationValue::Unavailable {
                reason: FactorUnavailabilityReason::MissingInput
            }
        ));
    }

    #[test]
    fn python_momentum_materialization_reuses_m11_dataset_contracts() {
        let user_id = Uuid::new_v4();
        let plan = FeaturePlan::freeze(adaq_feature_engine::FeaturePlanDraft {
            slots: vec![adaq_feature_engine::FeatureSlot {
                name: "close".into(),
                source: adaq_feature_engine::FeatureSource::Market {
                    field: adaq_feature_engine::MarketField::Close,
                },
                warmup_bars: 0,
            }],
            engine_identity: FeatureEngineIdentity::for_tests(),
            ..Default::default()
        })
        .unwrap();
        let plan_hash = plan.plan_hash().to_owned();
        let repeatability_report = ["5", "20", "60"]
            .into_iter()
            .map(|lookback| {
                (
                    lookback.into(),
                    crate::PythonRepeatabilityReport {
                        first_attempt_id: "1".repeat(64),
                        replay_attempt_id: "2".repeat(64),
                        first_process_sha256: "a".repeat(64),
                        replay_process_sha256: "b".repeat(64),
                        process_contract_sha256: "e".repeat(64),
                        first_input_sha256: "f".repeat(64),
                        replay_input_sha256: "0".repeat(64),
                        first_output_sha256: "c".repeat(64),
                        replay_output_sha256: "d".repeat(64),
                        exact: false,
                        partitions: vec!["fresh-process".into(), "portable-definition".into()],
                    },
                )
            })
            .collect();
        let repeatability_report_sha256 = crate::content_hash(&repeatability_report).unwrap();
        let (candidate, _) = crate::PythonFactorDraft {
            user_id,
            candidate_id: Uuid::new_v4(),
            revision: 1,
            scope: FactorScope::CrossSectional,
            feature_slots: vec![crate::FactorFeatureSlot {
                name: "close".into(),
            }],
            parameters: vec![crate::FactorParameter {
                name: "lookback".into(),
                parameter_type: crate::FactorParameterType::Integer,
                default_value: "20".into(),
                allowed_values: vec!["5".into(), "20".into(), "60".into()],
            }],
            outputs: vec![crate::FactorOutput {
                name: "momentum-score".into(),
            }],
            binding: crate::PythonFactorBinding {
                project_id: "py-factor-cross-sectional-momentum".into(),
                project_revision_sha256: "a".repeat(64),
                environment_sha256: "b".repeat(64),
                input_bindings: BTreeMap::from([("close".into(), "host:market-close".into())]),
                snapshot_id: "snapshot".into(),
                snapshot_bindings: BTreeMap::from([("AAA".into(), "snapshot".into())]),
                point_in_time_universe_id: "universe".into(),
                feature_evidence_sha256: "e".repeat(64),
                feature_dataset_bindings: BTreeMap::from([("AAA".into(), "dataset".into())]),
                normalized_parameters: BTreeMap::from([("lookback".into(), "20".into())]),
                engine_identity: "adaq-python-factor@1".into(),
                repeatability_report_sha256,
                repeatability_verified: false,
                repeatability_report,
                sdk_artifact_sha256: "c".repeat(64),
                entry_point: "project:create_project".into(),
                mode: crate::PythonFactorMode::PortableDefinition,
                feature_plan_hash: plan_hash.clone(),
                operator_catalog_version: adaq_feature_engine::FEATURE_OPERATOR_CATALOG_VERSION
                    .into(),
                resource_policy: crate::PythonFactorResourcePolicy::default(),
                seed: 7,
            },
            presentation: crate::FactorPresentationMetadata {
                name: "Python momentum".into(),
                ..Default::default()
            },
        }
        .publish()
        .unwrap();
        let rows = ["a", "b"]
            .into_iter()
            .flat_map(|instrument| {
                (1..=6).map(move |time| close_row(instrument, time, 100.0 + time as f64))
            })
            .collect::<Vec<_>>();
        let feature_dataset = CompletedFeatureDataset::new(
            user_id.to_string(),
            "feature-dataset",
            plan_hash.clone(),
            plan.to_json(),
            plan.engine_identity(),
            "snapshot",
            "universe",
            vec!["close".into()],
            rows,
        )
        .unwrap();
        let protocol = crate::FactorMaterializationProtocol::freeze(
            crate::FactorMaterializationProtocolDraft {
                protocol_id: Uuid::new_v4(),
                user_id,
                candidate_hash: candidate.candidate_hash.clone(),
                feature_dataset_id: feature_dataset.dataset_id.clone(),
                feature_plan_hash: plan_hash,
                parameters: vec![crate::FactorParameterValue::Integer(5)],
                market_data_snapshot_id: "snapshot".into(),
                point_in_time_universe_id: "universe".into(),
                observation_range: crate::ObservationRange {
                    start_time_ms: 1,
                    end_time_ms: 7,
                },
                market_context: context(),
                engine_identity: engine(),
                seed: 7,
            },
        )
        .unwrap();
        let dataset = FactorMaterializer::materialize(FactorMaterializationInput {
            candidate: &candidate,
            protocol: &protocol,
            feature_dataset: &feature_dataset,
            point_in_time_universe: &["b".into(), "a".into()],
            custom_package: None,
        })
        .unwrap();
        assert_eq!(dataset.rows.len(), 12);
        assert!(matches!(
            dataset.rows[0].values["momentum-score"],
            FactorObservationValue::Unavailable {
                reason: FactorUnavailabilityReason::Warmup
            }
        ));
        assert!(dataset.rows.iter().any(|row| matches!(
            row.values["momentum-score"],
            FactorObservationValue::Available { .. }
        )));
    }

    #[test]
    fn publisher_coalesces_reuses_and_locks_datasets() {
        let user_id = Uuid::new_v4();
        let protocol = crate::FactorMaterializationProtocol::freeze(
            crate::FactorMaterializationProtocolDraft {
                protocol_id: Uuid::new_v4(),
                user_id,
                candidate_hash: "a".repeat(64),
                feature_dataset_id: "feature".into(),
                feature_plan_hash: "b".repeat(64),
                parameters: vec![],
                market_data_snapshot_id: "snapshot".into(),
                point_in_time_universe_id: "universe".into(),
                observation_range: crate::ObservationRange {
                    start_time_ms: 0,
                    end_time_ms: 2,
                },
                market_context: context(),
                engine_identity: engine(),
                seed: 1,
            },
        )
        .unwrap();
        let publisher = FactorDatasetPublisher::default();
        let first = publisher.start(&protocol).unwrap();
        let second = publisher.start(&protocol).unwrap();
        assert_eq!(first.attempt.attempt_id, second.attempt.attempt_id);
        publisher.begin(user_id, first.attempt.attempt_id).unwrap();
        let dataset = empty_factor_dataset(&protocol);
        publisher
            .stage(user_id, first.attempt.attempt_id, dataset)
            .unwrap();
        let completed = publisher
            .publish(user_id, first.attempt.attempt_id)
            .unwrap();
        assert_eq!(completed.status, AttemptStatus::Completed);
        let reused = publisher.start(&protocol).unwrap();
        assert!(reused.reused_dataset_id.is_some());
        publisher
            .reference(
                user_id,
                reused.reused_dataset_id.as_ref().unwrap(),
                user_id,
                "report",
            )
            .unwrap();
        assert_eq!(
            publisher.delete(user_id, reused.reused_dataset_id.as_ref().unwrap()),
            Err(MaterializationError::DatasetReferenced)
        );
        publisher
            .unreference(
                user_id,
                reused.reused_dataset_id.as_ref().unwrap(),
                "report",
            )
            .unwrap();
        publisher
            .delete(user_id, reused.reused_dataset_id.as_ref().unwrap())
            .unwrap();
    }

    #[test]
    fn publisher_cancellation_is_retryable_and_isolated() {
        let user_id = Uuid::new_v4();
        let other_user_id = Uuid::new_v4();
        let protocol = crate::FactorMaterializationProtocol::freeze(
            crate::FactorMaterializationProtocolDraft {
                protocol_id: Uuid::new_v4(),
                user_id,
                candidate_hash: "a".repeat(64),
                feature_dataset_id: "feature".into(),
                feature_plan_hash: "b".repeat(64),
                parameters: vec![],
                market_data_snapshot_id: "snapshot".into(),
                point_in_time_universe_id: "universe".into(),
                observation_range: crate::ObservationRange {
                    start_time_ms: 0,
                    end_time_ms: 2,
                },
                market_context: context(),
                engine_identity: engine(),
                seed: 1,
            },
        )
        .unwrap();
        let publisher = FactorDatasetPublisher::default();
        let started = publisher.start(&protocol).unwrap();
        assert_eq!(
            publisher.begin(other_user_id, started.attempt.attempt_id),
            Err(MaterializationError::Unauthorized)
        );
        publisher
            .begin(user_id, started.attempt.attempt_id)
            .unwrap();
        let cancelled = publisher
            .cancel(user_id, started.attempt.attempt_id)
            .unwrap();
        assert_eq!(cancelled.status, AttemptStatus::Cancelled);
        let retry = publisher
            .retry(user_id, started.attempt.attempt_id)
            .unwrap();
        assert_eq!(retry.source_attempt_id, Some(started.attempt.attempt_id));
        assert_eq!(
            publisher.start(&protocol).unwrap().attempt.attempt_id,
            retry.attempt_id
        );
        publisher.begin(user_id, retry.attempt_id).unwrap();
        let dataset = empty_factor_dataset(&protocol);
        let dataset_id = dataset.manifest.dataset_id.clone();
        publisher.stage(user_id, retry.attempt_id, dataset).unwrap();
        let failed = publisher
            .fail(
                user_id,
                retry.attempt_id,
                "x".repeat(MAX_DIAGNOSTIC_BYTES + 100),
            )
            .unwrap();
        assert!(failed.diagnostic.as_deref().unwrap().len() <= MAX_DIAGNOSTIC_BYTES + 3);
        assert_eq!(
            publisher.retry(user_id, started.attempt.attempt_id),
            Err(MaterializationError::InvalidTransition)
        );
        assert_eq!(
            publisher.dataset(user_id, &dataset_id),
            Err(MaterializationError::Unauthorized)
        );
        assert_eq!(
            publisher.reference(user_id, &dataset_id, other_user_id, "report"),
            Err(MaterializationError::Unauthorized)
        );
    }
}
