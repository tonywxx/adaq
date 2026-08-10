use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use super::{
    FeatureEngineIdentity, FeatureObservation, FeatureObservationValue, FeatureReference,
    FeatureUnavailabilityReason, ObservationRange, ValidationIssue, canonical_json, is_lower_kebab,
    is_sha256, issue, sha256_hex, sort_issues,
};

pub const FITTING_PROTOCOL_SCHEMA_VERSION: &str = "1.0.0";
pub const FITTED_ARTIFACT_SCHEMA_VERSION: &str = "1.0.0";
pub const FITTING_ENGINE_VERSION: &str = "adaq-feature-fitting@1.0.0";
pub const NEAREST_RANK_QUANTILE_VERSION: &str = "nearest-rank@1.0.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FittingScope {
    PooledUniverse,
    PerInstrument,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum FittingAlgorithm {
    Standardization,
    Winsorization {
        lower_quantile: f64,
        upper_quantile: f64,
        quantile_method_version: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TransformationFittingProtocolDraft {
    pub input_feature: FeatureReference,
    pub fitted_node_id: String,
    pub fitted_output: FeatureReference,
    pub snapshot_id: String,
    pub point_in_time_universe_id: String,
    pub fitting_scope: FittingScope,
    pub fitting_window: ObservationRange,
    pub algorithm: FittingAlgorithm,
    pub minimum_samples: u64,
    pub engine_identity: FeatureEngineIdentity,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProtocolContent {
    protocol_schema_version: String,
    fitting_engine_version: String,
    input_feature: FeatureReference,
    fitted_node_id: String,
    fitted_output: FeatureReference,
    snapshot_id: String,
    point_in_time_universe_id: String,
    fitting_scope: FittingScope,
    fitting_window: ObservationRange,
    algorithm: FittingAlgorithm,
    minimum_samples: u64,
    engine_identity: FeatureEngineIdentity,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProtocolDocument {
    #[serde(flatten)]
    content: ProtocolContent,
    protocol_hash: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TransformationFittingProtocol(ProtocolDocument);

pub type FittingProtocol = TransformationFittingProtocol;
pub type FittingProtocolDraft = TransformationFittingProtocolDraft;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FittingValidationError {
    pub issues: Vec<ValidationIssue>,
}

impl FittingValidationError {
    pub fn codes(&self) -> Vec<&str> {
        self.issues
            .iter()
            .map(|value| value.code.as_str())
            .collect()
    }
}

impl fmt::Display for FittingValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "fitting protocol validation failed with {} issue(s)",
            self.issues.len()
        )
    }
}

impl std::error::Error for FittingValidationError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FittingProtocolLoadError {
    InvalidJson,
    NonCanonical,
    HashMismatch,
    TooLarge,
    InvalidContract,
    UnsupportedEngineIdentity,
}

impl FittingProtocolLoadError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidJson => "invalid-fitting-protocol-json",
            Self::NonCanonical => "non-canonical-fitting-protocol-json",
            Self::HashMismatch => "fitting-protocol-hash-mismatch",
            Self::TooLarge => "fitting-protocol-json-too-large",
            Self::InvalidContract => "invalid-fitting-protocol-contract",
            Self::UnsupportedEngineIdentity => "unsupported-fitting-engine-identity",
        }
    }
}

impl fmt::Display for FittingProtocolLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for FittingProtocolLoadError {}

impl TransformationFittingProtocol {
    pub fn freeze(
        draft: TransformationFittingProtocolDraft,
    ) -> Result<Self, FittingValidationError> {
        let content = ProtocolContent {
            protocol_schema_version: FITTING_PROTOCOL_SCHEMA_VERSION.into(),
            fitting_engine_version: FITTING_ENGINE_VERSION.into(),
            input_feature: draft.input_feature,
            fitted_node_id: draft.fitted_node_id,
            fitted_output: draft.fitted_output,
            snapshot_id: draft.snapshot_id,
            point_in_time_universe_id: draft.point_in_time_universe_id,
            fitting_scope: draft.fitting_scope,
            fitting_window: draft.fitting_window,
            algorithm: draft.algorithm,
            minimum_samples: draft.minimum_samples,
            engine_identity: draft.engine_identity,
        };
        validate_protocol(&content)?;
        let protocol_hash =
            sha256_hex(
                &canonical_json(&content).map_err(|_| FittingValidationError {
                    issues: vec![issue("fitting-protocol-json-too-large", None)],
                })?,
            );
        let document = ProtocolDocument {
            content,
            protocol_hash,
        };
        canonical_json(&document).map_err(|_| FittingValidationError {
            issues: vec![issue("fitting-protocol-json-too-large", None)],
        })?;
        Ok(Self(document))
    }

    pub fn draft(&self) -> TransformationFittingProtocolDraft {
        TransformationFittingProtocolDraft {
            input_feature: self.0.content.input_feature.clone(),
            fitted_node_id: self.0.content.fitted_node_id.clone(),
            fitted_output: self.0.content.fitted_output.clone(),
            snapshot_id: self.0.content.snapshot_id.clone(),
            point_in_time_universe_id: self.0.content.point_in_time_universe_id.clone(),
            fitting_scope: self.0.content.fitting_scope,
            fitting_window: self.0.content.fitting_window.clone(),
            algorithm: self.0.content.algorithm.clone(),
            minimum_samples: self.0.content.minimum_samples,
            engine_identity: self.0.content.engine_identity.clone(),
        }
    }

    pub fn protocol_hash(&self) -> &str {
        &self.0.protocol_hash
    }

    pub fn input_feature(&self) -> &FeatureReference {
        &self.0.content.input_feature
    }

    pub fn fitted_node_id(&self) -> &str {
        &self.0.content.fitted_node_id
    }

    pub fn fitted_output(&self) -> &FeatureReference {
        &self.0.content.fitted_output
    }

    pub fn snapshot_id(&self) -> &str {
        &self.0.content.snapshot_id
    }

    pub fn point_in_time_universe_id(&self) -> &str {
        &self.0.content.point_in_time_universe_id
    }

    pub const fn fitting_scope(&self) -> FittingScope {
        self.0.content.fitting_scope
    }

    pub fn fitting_window(&self) -> &ObservationRange {
        &self.0.content.fitting_window
    }

    pub fn algorithm(&self) -> &FittingAlgorithm {
        &self.0.content.algorithm
    }

    pub const fn minimum_samples(&self) -> u64 {
        self.0.content.minimum_samples
    }

    pub fn engine_identity(&self) -> &FeatureEngineIdentity {
        &self.0.content.engine_identity
    }

    pub fn to_json(&self) -> Vec<u8> {
        canonical_json(&self.0).expect("validated fitting protocol fits the canonical size limit")
    }

    pub fn load(bytes: &[u8]) -> Result<Self, FittingProtocolLoadError> {
        let identity = FeatureEngineIdentity::native()
            .map_err(|_| FittingProtocolLoadError::UnsupportedEngineIdentity)?;
        Self::load_for_engine(bytes, &identity)
    }

    pub fn load_for_engine(
        bytes: &[u8],
        identity: &FeatureEngineIdentity,
    ) -> Result<Self, FittingProtocolLoadError> {
        if bytes.len() > super::MAX_CANONICAL_JSON_BYTES {
            return Err(FittingProtocolLoadError::TooLarge);
        }
        let raw = serde_json::from_slice::<Value>(bytes)
            .map_err(|_| FittingProtocolLoadError::InvalidJson)?;
        let document = serde_json::from_value::<ProtocolDocument>(raw)
            .map_err(|_| FittingProtocolLoadError::InvalidJson)?;
        let canonical =
            canonical_json(&document).map_err(|_| FittingProtocolLoadError::InvalidJson)?;
        if canonical != bytes {
            return Err(FittingProtocolLoadError::NonCanonical);
        }
        let expected_hash = sha256_hex(
            &canonical_json(&document.content)
                .map_err(|_| FittingProtocolLoadError::InvalidJson)?,
        );
        if document.protocol_hash != expected_hash {
            return Err(FittingProtocolLoadError::HashMismatch);
        }
        validate_protocol(&document.content)
            .map_err(|_| FittingProtocolLoadError::InvalidContract)?;
        if document.content.engine_identity != *identity {
            return Err(FittingProtocolLoadError::UnsupportedEngineIdentity);
        }
        Ok(Self(document))
    }

    pub fn fit(
        &self,
        observations: &[FeatureObservation],
        created_at_ms: i64,
    ) -> Result<FittedTransformationArtifact, FittingError> {
        let samples = eligible_samples(self, observations)?;
        let parameters = match self.fitting_scope() {
            FittingScope::PooledUniverse => FittedArtifactParameters::Pooled {
                parameters: fit_parameters(&samples, self.algorithm(), self.minimum_samples())?,
            },
            FittingScope::PerInstrument => {
                let mut grouped = BTreeMap::<String, Vec<EligibleSample>>::new();
                for sample in samples.iter().cloned() {
                    grouped
                        .entry(sample.instrument_id.clone())
                        .or_default()
                        .push(sample);
                }
                if grouped.is_empty() {
                    return Err(insufficient_samples(self, None, 0));
                }
                let mut fitted = BTreeMap::new();
                for (instrument_id, samples) in grouped {
                    fitted.insert(
                        instrument_id.clone(),
                        fit_parameters(&samples, self.algorithm(), self.minimum_samples())
                            .map_err(|error| FittingError {
                                instrument_id: Some(instrument_id),
                                ..error
                            })?,
                    );
                }
                FittedArtifactParameters::PerInstrument { parameters: fitted }
            }
        };
        let eligible_at_ms = observations
            .iter()
            .filter(|observation| {
                observation_matches_protocol(self, observation)
                    && self.fitting_window().start_time_ms <= observation.observation_time_ms
                    && observation.observation_time_ms < self.fitting_window().end_time_ms
            })
            .filter_map(|observation| match observation.value {
                FeatureObservationValue::Available {
                    available_at_ms, ..
                } if available_at_ms <= observation.observation_time_ms => Some(available_at_ms),
                _ => None,
            })
            .max()
            .expect("a successful fit has at least one eligible sample");
        FittedTransformationArtifact::from_protocol(self, parameters, eligible_at_ms, created_at_ms)
    }
}

fn validate_protocol(content: &ProtocolContent) -> Result<(), FittingValidationError> {
    let mut issues = Vec::new();
    if content.protocol_schema_version != FITTING_PROTOCOL_SCHEMA_VERSION {
        issues.push(issue("unsupported-fitting-protocol-schema", None));
    }
    if content.fitting_engine_version != FITTING_ENGINE_VERSION {
        issues.push(issue("unsupported-fitting-engine", None));
    }
    if !is_sha256(&content.input_feature.definition_hash)
        || content.input_feature.node_id.trim().is_empty()
        || !is_lower_kebab(&content.input_feature.output_name)
    {
        issues.push(issue("invalid-fitting-input-feature", None));
    }
    if !is_lower_kebab(&content.fitted_node_id) {
        issues.push(issue("invalid-fitting-node", None));
    }
    if !is_sha256(&content.fitted_output.definition_hash)
        || content.fitted_output.node_id != content.fitted_node_id
        || !is_lower_kebab(&content.fitted_output.node_id)
        || !is_lower_kebab(&content.fitted_output.output_name)
    {
        issues.push(issue("invalid-fitting-output", None));
    }
    if content.snapshot_id.trim().is_empty() || content.point_in_time_universe_id.trim().is_empty()
    {
        issues.push(issue("invalid-fitting-evidence-identity", None));
    }
    if content.fitting_window.start_time_ms >= content.fitting_window.end_time_ms {
        issues.push(issue("invalid-fitting-window", None));
    }
    if content.minimum_samples == 0 {
        issues.push(issue("invalid-minimum-samples", None));
    }
    if !content.engine_identity.validate() {
        issues.push(issue("invalid-fitting-engine-identity", None));
    }
    if let FittingAlgorithm::Winsorization {
        lower_quantile,
        upper_quantile,
        quantile_method_version,
    } = &content.algorithm
    {
        if !lower_quantile.is_finite()
            || !upper_quantile.is_finite()
            || *lower_quantile < 0.0
            || *upper_quantile > 1.0
            || lower_quantile >= upper_quantile
        {
            issues.push(issue("invalid-winsorization-quantiles", None));
        }
        if quantile_method_version != NEAREST_RANK_QUANTILE_VERSION {
            issues.push(issue("unsupported-quantile-method", None));
        }
    }
    sort_issues(&mut issues);
    if issues.is_empty() {
        Ok(())
    } else {
        Err(FittingValidationError { issues })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FittingError {
    code: &'static str,
    pub instrument_id: Option<String>,
    pub eligible_samples: usize,
    pub minimum_samples: usize,
}

impl FittingError {
    pub const fn code(&self) -> &'static str {
        self.code
    }
}

impl fmt::Display for FittingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code)
    }
}

impl std::error::Error for FittingError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FittingApplyError {
    ArtifactNotAvailableForObservation {
        eligible_at_ms: i64,
        observation_time_ms: i64,
    },
    NonFiniteInput,
    NonFiniteOutput,
    InvalidArtifact,
}

impl FittingApplyError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::ArtifactNotAvailableForObservation { .. } => {
                "artifact-not-available-for-observation"
            }
            Self::NonFiniteInput => "non-finite-fitting-input",
            Self::NonFiniteOutput => "non-finite-fitting-output",
            Self::InvalidArtifact => "invalid-fitted-artifact",
        }
    }
}

impl fmt::Display for FittingApplyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for FittingApplyError {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WinsorizationParameters {
    pub lower_quantile: f64,
    pub upper_quantile: f64,
    pub lower_value: f64,
    pub upper_value: f64,
    pub quantile_method_version: String,
    pub sample_count: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "parameters",
    rename_all = "kebab-case",
    deny_unknown_fields
)]
pub enum FittedParameters {
    Standardization {
        mean: f64,
        population_standard_deviation: f64,
        sample_count: u64,
    },
    Winsorization(WinsorizationParameters),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "scope", rename_all = "kebab-case", deny_unknown_fields)]
pub enum FittedArtifactParameters {
    Pooled {
        parameters: FittedParameters,
    },
    PerInstrument {
        parameters: BTreeMap<String, FittedParameters>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ArtifactContent {
    artifact_schema_version: String,
    protocol_hash: String,
    input_feature: FeatureReference,
    fitted_node_id: String,
    fitted_output: FeatureReference,
    snapshot_id: String,
    point_in_time_universe_id: String,
    fitting_scope: FittingScope,
    fitting_window: ObservationRange,
    algorithm: FittingAlgorithm,
    parameters: FittedArtifactParameters,
    eligible_at_ms: i64,
    engine_identity: FeatureEngineIdentity,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ArtifactDocument {
    #[serde(flatten)]
    content: ArtifactContent,
    artifact_id: String,
    created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FittedArtifactLoadError {
    InvalidJson,
    NonCanonical,
    HashMismatch,
    TooLarge,
    InvalidContract,
    UnsupportedEngineIdentity,
}

impl FittedArtifactLoadError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidJson => "invalid-fitted-artifact-json",
            Self::NonCanonical => "non-canonical-fitted-artifact-json",
            Self::HashMismatch => "fitted-artifact-hash-mismatch",
            Self::TooLarge => "fitted-artifact-json-too-large",
            Self::InvalidContract => "invalid-fitted-artifact-contract",
            Self::UnsupportedEngineIdentity => "unsupported-fitted-artifact-engine-identity",
        }
    }
}

impl fmt::Display for FittedArtifactLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for FittedArtifactLoadError {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FittedTransformationArtifact {
    pub artifact_id: String,
    pub protocol_hash: String,
    pub input_feature: FeatureReference,
    pub fitted_node_id: String,
    pub fitted_output: FeatureReference,
    pub snapshot_id: String,
    pub point_in_time_universe_id: String,
    pub fitting_scope: FittingScope,
    pub fitting_window: ObservationRange,
    pub algorithm: FittingAlgorithm,
    pub parameters: FittedArtifactParameters,
    pub eligible_at_ms: i64,
    pub created_at_ms: i64,
    pub engine_identity: FeatureEngineIdentity,
}

pub enum FittedTransformationValue {
    Available { value: f64, available_at_ms: i64 },
    Unavailable(FeatureUnavailabilityReason),
}

impl FittedTransformationArtifact {
    fn from_protocol(
        protocol: &TransformationFittingProtocol,
        parameters: FittedArtifactParameters,
        eligible_at_ms: i64,
        created_at_ms: i64,
    ) -> Result<Self, FittingError> {
        let artifact = Self {
            artifact_id: String::new(),
            protocol_hash: protocol.protocol_hash().into(),
            input_feature: protocol.input_feature().clone(),
            fitted_node_id: protocol.fitted_node_id().into(),
            fitted_output: protocol.fitted_output().clone(),
            snapshot_id: protocol.snapshot_id().into(),
            point_in_time_universe_id: protocol.point_in_time_universe_id().into(),
            fitting_scope: protocol.fitting_scope(),
            fitting_window: protocol.fitting_window().clone(),
            algorithm: protocol.algorithm().clone(),
            parameters,
            eligible_at_ms,
            created_at_ms,
            engine_identity: protocol.engine_identity().clone(),
        };
        let artifact_id =
            sha256_hex(
                &canonical_json(&artifact.content()).map_err(|_| FittingError {
                    code: "fitted-artifact-json-too-large",
                    instrument_id: None,
                    eligible_samples: 0,
                    minimum_samples: protocol.minimum_samples() as usize,
                })?,
            );
        Ok(Self {
            artifact_id,
            ..artifact
        })
    }

    fn content(&self) -> ArtifactContent {
        ArtifactContent {
            artifact_schema_version: FITTED_ARTIFACT_SCHEMA_VERSION.into(),
            protocol_hash: self.protocol_hash.clone(),
            input_feature: self.input_feature.clone(),
            fitted_node_id: self.fitted_node_id.clone(),
            fitted_output: self.fitted_output.clone(),
            snapshot_id: self.snapshot_id.clone(),
            point_in_time_universe_id: self.point_in_time_universe_id.clone(),
            fitting_scope: self.fitting_scope,
            fitting_window: self.fitting_window.clone(),
            algorithm: self.algorithm.clone(),
            parameters: self.parameters.clone(),
            eligible_at_ms: self.eligible_at_ms,
            engine_identity: self.engine_identity.clone(),
        }
    }

    pub fn artifact_id(&self) -> &str {
        &self.artifact_id
    }

    pub fn protocol_hash(&self) -> &str {
        &self.protocol_hash
    }

    pub const fn eligible_at_ms(&self) -> i64 {
        self.eligible_at_ms
    }

    pub const fn created_at_ms(&self) -> i64 {
        self.created_at_ms
    }

    pub fn parameters_for(&self, instrument_id: &str) -> Option<&FittedParameters> {
        match &self.parameters {
            FittedArtifactParameters::Pooled { parameters } => Some(parameters),
            FittedArtifactParameters::PerInstrument { parameters } => parameters.get(instrument_id),
        }
    }

    pub fn apply_value(
        &self,
        instrument_id: &str,
        observation_time_ms: i64,
        value: f64,
        available_at_ms: i64,
    ) -> Result<FittedTransformationValue, FittingApplyError> {
        if !validate_artifact(self) {
            return Err(FittingApplyError::InvalidArtifact);
        }
        if observation_time_ms < self.fitting_window.end_time_ms {
            return Err(FittingApplyError::ArtifactNotAvailableForObservation {
                eligible_at_ms: self.fitting_window.end_time_ms,
                observation_time_ms,
            });
        }
        if !value.is_finite() {
            return Err(FittingApplyError::NonFiniteInput);
        }
        let Some(parameters) = self.parameters_for(instrument_id) else {
            return Ok(FittedTransformationValue::Unavailable(
                FeatureUnavailabilityReason::ArtifactMissingInstrument,
            ));
        };
        let available_at_ms = available_at_ms.max(self.eligible_at_ms);
        let value = match parameters {
            FittedParameters::Standardization {
                mean,
                population_standard_deviation,
                ..
            } => {
                if *population_standard_deviation == 0.0 {
                    return Ok(FittedTransformationValue::Unavailable(
                        FeatureUnavailabilityReason::UndefinedArithmetic,
                    ));
                }
                (value - mean) / population_standard_deviation
            }
            FittedParameters::Winsorization(WinsorizationParameters {
                lower_value,
                upper_value,
                ..
            }) => value.clamp(*lower_value, *upper_value),
        };
        if !value.is_finite() {
            return Err(FittingApplyError::NonFiniteOutput);
        }
        Ok(FittedTransformationValue::Available {
            value,
            available_at_ms,
        })
    }

    pub fn to_json(&self) -> Vec<u8> {
        canonical_json(&ArtifactDocument {
            content: self.content(),
            artifact_id: self.artifact_id.clone(),
            created_at_ms: self.created_at_ms,
        })
        .expect("validated fitted artifact fits the canonical size limit")
    }

    pub fn load(bytes: &[u8]) -> Result<Self, FittedArtifactLoadError> {
        let identity = FeatureEngineIdentity::native()
            .map_err(|_| FittedArtifactLoadError::UnsupportedEngineIdentity)?;
        Self::load_for_engine(bytes, &identity)
    }

    pub fn load_for_engine(
        bytes: &[u8],
        identity: &FeatureEngineIdentity,
    ) -> Result<Self, FittedArtifactLoadError> {
        if bytes.len() > super::MAX_CANONICAL_JSON_BYTES {
            return Err(FittedArtifactLoadError::TooLarge);
        }
        let raw = serde_json::from_slice::<Value>(bytes)
            .map_err(|_| FittedArtifactLoadError::InvalidJson)?;
        let document = serde_json::from_value::<ArtifactDocument>(raw)
            .map_err(|_| FittedArtifactLoadError::InvalidJson)?;
        let canonical =
            canonical_json(&document).map_err(|_| FittedArtifactLoadError::InvalidJson)?;
        if canonical != bytes {
            return Err(FittedArtifactLoadError::NonCanonical);
        }
        let expected_hash = sha256_hex(
            &canonical_json(&document.content).map_err(|_| FittedArtifactLoadError::InvalidJson)?,
        );
        if document.artifact_id != expected_hash {
            return Err(FittedArtifactLoadError::HashMismatch);
        }
        let artifact = Self {
            artifact_id: document.artifact_id,
            protocol_hash: document.content.protocol_hash,
            input_feature: document.content.input_feature,
            fitted_node_id: document.content.fitted_node_id,
            fitted_output: document.content.fitted_output,
            snapshot_id: document.content.snapshot_id,
            point_in_time_universe_id: document.content.point_in_time_universe_id,
            fitting_scope: document.content.fitting_scope,
            fitting_window: document.content.fitting_window,
            algorithm: document.content.algorithm,
            parameters: document.content.parameters,
            eligible_at_ms: document.content.eligible_at_ms,
            created_at_ms: document.created_at_ms,
            engine_identity: document.content.engine_identity,
        };
        if !validate_artifact(&artifact) {
            return Err(FittedArtifactLoadError::InvalidContract);
        }
        if artifact.engine_identity != *identity {
            return Err(FittedArtifactLoadError::UnsupportedEngineIdentity);
        }
        Ok(artifact)
    }

    pub(crate) fn integrity_valid(&self) -> bool {
        validate_artifact(self)
    }
}

fn validate_artifact(artifact: &FittedTransformationArtifact) -> bool {
    if artifact.protocol_hash.len() != 64
        || !is_sha256(&artifact.protocol_hash)
        || !is_sha256(&artifact.artifact_id)
        || !is_sha256(&artifact.input_feature.definition_hash)
        || artifact.input_feature.node_id.trim().is_empty()
        || !is_lower_kebab(&artifact.input_feature.output_name)
        || !is_lower_kebab(&artifact.fitted_node_id)
        || !is_sha256(&artifact.fitted_output.definition_hash)
        || artifact.fitted_output.node_id != artifact.fitted_node_id
        || !is_lower_kebab(&artifact.fitted_output.node_id)
        || !is_lower_kebab(&artifact.fitted_output.output_name)
        || artifact.snapshot_id.trim().is_empty()
        || artifact.point_in_time_universe_id.trim().is_empty()
        || artifact.fitting_window.start_time_ms >= artifact.fitting_window.end_time_ms
        || !artifact.engine_identity.validate()
        || !artifact_id_matches(artifact)
    {
        return false;
    }
    match (
        &artifact.algorithm,
        &artifact.parameters,
        artifact.fitting_scope,
    ) {
        (
            FittingAlgorithm::Standardization,
            FittedArtifactParameters::Pooled { parameters },
            FittingScope::PooledUniverse,
        ) => standardization_matches(parameters),
        (
            FittingAlgorithm::Standardization,
            FittedArtifactParameters::PerInstrument { parameters },
            FittingScope::PerInstrument,
        ) => {
            !parameters.is_empty()
                && parameters.keys().all(|key| !key.trim().is_empty())
                && parameters.values().all(standardization_matches)
        }
        (
            FittingAlgorithm::Winsorization {
                lower_quantile,
                upper_quantile,
                quantile_method_version,
            },
            FittedArtifactParameters::Pooled { parameters },
            FittingScope::PooledUniverse,
        ) => winsorization_matches(
            parameters,
            *lower_quantile,
            *upper_quantile,
            quantile_method_version,
        ),
        (
            FittingAlgorithm::Winsorization {
                lower_quantile,
                upper_quantile,
                quantile_method_version,
            },
            FittedArtifactParameters::PerInstrument { parameters },
            FittingScope::PerInstrument,
        ) => {
            !parameters.is_empty()
                && parameters.keys().all(|key| !key.trim().is_empty())
                && parameters.values().all(|value| {
                    winsorization_matches(
                        value,
                        *lower_quantile,
                        *upper_quantile,
                        quantile_method_version,
                    )
                })
        }
        _ => false,
    }
}

fn artifact_matches_protocol(
    artifact: &FittedTransformationArtifact,
    protocol: &TransformationFittingProtocol,
) -> bool {
    artifact.protocol_hash == protocol.protocol_hash()
        && artifact.input_feature == *protocol.input_feature()
        && artifact.fitted_node_id == protocol.fitted_node_id()
        && artifact.fitted_output == *protocol.fitted_output()
        && artifact.snapshot_id == protocol.snapshot_id()
        && artifact.point_in_time_universe_id == protocol.point_in_time_universe_id()
        && artifact.fitting_scope == protocol.fitting_scope()
        && artifact.fitting_window == *protocol.fitting_window()
        && artifact.algorithm == *protocol.algorithm()
        && artifact.engine_identity == *protocol.engine_identity()
        && artifact_parameters_meet_minimum(&artifact.parameters, protocol.minimum_samples())
}

fn artifact_parameters_meet_minimum(
    parameters: &FittedArtifactParameters,
    minimum_samples: u64,
) -> bool {
    let sample_count = |parameters: &FittedParameters| match parameters {
        FittedParameters::Standardization { sample_count, .. }
        | FittedParameters::Winsorization(WinsorizationParameters { sample_count, .. }) => {
            *sample_count >= minimum_samples
        }
    };
    match parameters {
        FittedArtifactParameters::Pooled { parameters } => sample_count(parameters),
        FittedArtifactParameters::PerInstrument { parameters } => {
            parameters.values().all(sample_count)
        }
    }
}

fn artifact_id_matches(artifact: &FittedTransformationArtifact) -> bool {
    canonical_json(&artifact.content())
        .ok()
        .is_some_and(|content| sha256_hex(&content) == artifact.artifact_id)
}

fn standardization_matches(parameters: &FittedParameters) -> bool {
    let FittedParameters::Standardization {
        mean,
        population_standard_deviation,
        sample_count,
    } = parameters
    else {
        return false;
    };
    mean.is_finite()
        && population_standard_deviation.is_finite()
        && *population_standard_deviation >= 0.0
        && *sample_count > 0
}

fn winsorization_matches(
    parameters: &FittedParameters,
    lower_quantile: f64,
    upper_quantile: f64,
    quantile_method_version: &str,
) -> bool {
    let FittedParameters::Winsorization(parameters) = parameters else {
        return false;
    };
    parameters.lower_quantile == lower_quantile
        && parameters.upper_quantile == upper_quantile
        && parameters.quantile_method_version == quantile_method_version
        && parameters.lower_value.is_finite()
        && parameters.upper_value.is_finite()
        && parameters.lower_value <= parameters.upper_value
        && parameters.sample_count > 0
}

fn eligible_samples(
    protocol: &TransformationFittingProtocol,
    observations: &[FeatureObservation],
) -> Result<Vec<EligibleSample>, FittingError> {
    let mut samples = Vec::new();
    for observation in observations {
        if observation.output_name != protocol.input_feature().output_name
            || observation.observation_time_ms < protocol.fitting_window().start_time_ms
            || observation.observation_time_ms >= protocol.fitting_window().end_time_ms
        {
            continue;
        }
        if observation.feature_reference.as_ref() != Some(protocol.input_feature()) {
            return Err(FittingError {
                code: "fitting-input-feature-mismatch",
                instrument_id: Some(observation.instrument_id.clone()),
                eligible_samples: samples.len(),
                minimum_samples: protocol.minimum_samples() as usize,
            });
        }
        let FeatureObservationValue::Available {
            value,
            available_at_ms,
        } = observation.value
        else {
            continue;
        };
        if available_at_ms > observation.observation_time_ms {
            continue;
        }
        if !value.is_finite() {
            return Err(FittingError {
                code: "non-finite-fitting-input",
                instrument_id: Some(observation.instrument_id.clone()),
                eligible_samples: samples.len(),
                minimum_samples: protocol.minimum_samples() as usize,
            });
        }
        samples.push(EligibleSample {
            instrument_id: observation.instrument_id.clone(),
            observation_time_ms: observation.observation_time_ms,
            available_at_ms,
            value,
        });
    }
    samples.sort_by(|left, right| {
        left.instrument_id
            .cmp(&right.instrument_id)
            .then_with(|| left.observation_time_ms.cmp(&right.observation_time_ms))
            .then_with(|| left.available_at_ms.cmp(&right.available_at_ms))
            .then_with(|| left.value.total_cmp(&right.value))
    });
    let mut seen = BTreeSet::new();
    for sample in &samples {
        if !seen.insert((sample.instrument_id.clone(), sample.observation_time_ms)) {
            return Err(FittingError {
                code: "duplicate-fitting-observation",
                instrument_id: Some(sample.instrument_id.clone()),
                eligible_samples: samples.len(),
                minimum_samples: protocol.minimum_samples() as usize,
            });
        }
    }
    if samples.len() < protocol.minimum_samples() as usize
        && protocol.fitting_scope() == FittingScope::PooledUniverse
    {
        return Err(insufficient_samples(protocol, None, samples.len()));
    }
    Ok(samples)
}

fn observation_matches_protocol(
    protocol: &TransformationFittingProtocol,
    observation: &FeatureObservation,
) -> bool {
    observation.output_name == protocol.input_feature().output_name
        && observation.feature_reference.as_ref() == Some(protocol.input_feature())
}

#[derive(Debug, Clone)]
struct EligibleSample {
    instrument_id: String,
    observation_time_ms: i64,
    available_at_ms: i64,
    value: f64,
}

fn insufficient_samples(
    protocol: &TransformationFittingProtocol,
    instrument_id: Option<String>,
    eligible_samples: usize,
) -> FittingError {
    FittingError {
        code: "insufficient-samples",
        instrument_id,
        eligible_samples,
        minimum_samples: protocol.minimum_samples() as usize,
    }
}

fn fit_parameters(
    samples: &[EligibleSample],
    algorithm: &FittingAlgorithm,
    minimum_samples: u64,
) -> Result<FittedParameters, FittingError> {
    if samples.len() < minimum_samples as usize {
        return Err(FittingError {
            code: "insufficient-samples",
            instrument_id: samples.first().map(|sample| sample.instrument_id.clone()),
            eligible_samples: samples.len(),
            minimum_samples: minimum_samples as usize,
        });
    }
    let sample_count = samples.len() as u64;
    let values = samples
        .iter()
        .map(|sample| sample.value)
        .collect::<Vec<_>>();
    match algorithm {
        FittingAlgorithm::Standardization => {
            let mean = values.iter().sum::<f64>() / values.len() as f64;
            let variance = values
                .iter()
                .map(|value| (value - mean).powi(2))
                .sum::<f64>()
                / values.len() as f64;
            let population_standard_deviation = variance.sqrt();
            if !mean.is_finite() || !population_standard_deviation.is_finite() {
                return Err(FittingError {
                    code: "non-finite-fitting-parameters",
                    instrument_id: samples.first().map(|sample| sample.instrument_id.clone()),
                    eligible_samples: samples.len(),
                    minimum_samples: minimum_samples as usize,
                });
            }
            Ok(FittedParameters::Standardization {
                mean,
                population_standard_deviation,
                sample_count,
            })
        }
        FittingAlgorithm::Winsorization {
            lower_quantile,
            upper_quantile,
            quantile_method_version,
        } => {
            let mut sorted = values;
            sorted.sort_by(f64::total_cmp);
            let lower_value = nearest_rank(&sorted, *lower_quantile);
            let upper_value = nearest_rank(&sorted, *upper_quantile);
            if !lower_value.is_finite() || !upper_value.is_finite() {
                return Err(FittingError {
                    code: "non-finite-fitting-parameters",
                    instrument_id: samples.first().map(|sample| sample.instrument_id.clone()),
                    eligible_samples: samples.len(),
                    minimum_samples: minimum_samples as usize,
                });
            }
            Ok(FittedParameters::Winsorization(WinsorizationParameters {
                lower_quantile: *lower_quantile,
                upper_quantile: *upper_quantile,
                lower_value,
                upper_value,
                quantile_method_version: quantile_method_version.clone(),
                sample_count,
            }))
        }
    }
}

fn nearest_rank(sorted: &[f64], quantile: f64) -> f64 {
    let rank = (quantile * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TransformationFittingAttemptStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TransformationFittingAttempt {
    pub attempt_id: String,
    pub user_id: String,
    pub protocol_hash: String,
    pub status: TransformationFittingAttemptStatus,
    pub artifact_id: Option<String>,
    pub failure_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FittingStoreError {
    InvalidUser,
    AttemptNotFound,
    ArtifactNotFound,
    InvalidTransition,
    ProtocolMismatch,
    ArtifactIdCollision,
    ArtifactReferenced,
    InvalidReference,
    InvalidArtifact,
}

impl FittingStoreError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidUser => "invalid-fitting-user",
            Self::AttemptNotFound => "fitting-attempt-not-found",
            Self::ArtifactNotFound => "fitted-artifact-not-found",
            Self::InvalidTransition => "invalid-fitting-attempt-transition",
            Self::ProtocolMismatch => "fitted-artifact-protocol-mismatch",
            Self::ArtifactIdCollision => "fitted-artifact-id-collision",
            Self::ArtifactReferenced => "artifact-referenced",
            Self::InvalidReference => "invalid-artifact-reference",
            Self::InvalidArtifact => "invalid-fitted-artifact",
        }
    }
}

impl fmt::Display for FittingStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for FittingStoreError {}

#[derive(Debug, Default)]
pub struct TransformationFittingStore {
    protocols: BTreeMap<String, TransformationFittingProtocol>,
    attempts: BTreeMap<String, TransformationFittingAttempt>,
    artifacts: BTreeMap<String, FittedTransformationArtifact>,
    artifact_access: BTreeSet<(String, String)>,
    artifact_references: BTreeSet<(String, String, String)>,
}

impl TransformationFittingStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start(
        &mut self,
        user_id: &str,
        protocol: &TransformationFittingProtocol,
    ) -> Result<TransformationFittingAttempt, FittingStoreError> {
        validate_user(user_id)?;
        self.protocols
            .insert(protocol.protocol_hash().into(), protocol.clone());
        if let Some(existing) = self.reusable_attempt(user_id, protocol.protocol_hash()) {
            return Ok(existing.clone());
        }
        let attempt = TransformationFittingAttempt {
            attempt_id: Uuid::new_v4().to_string(),
            user_id: user_id.into(),
            protocol_hash: protocol.protocol_hash().into(),
            status: TransformationFittingAttemptStatus::Pending,
            artifact_id: None,
            failure_code: None,
        };
        self.attempts
            .insert(attempt.attempt_id.clone(), attempt.clone());
        Ok(attempt)
    }

    pub fn get_attempt(
        &self,
        user_id: &str,
        attempt_id: &str,
    ) -> Result<TransformationFittingAttempt, FittingStoreError> {
        self.attempts
            .get(attempt_id)
            .filter(|attempt| attempt.user_id == user_id)
            .cloned()
            .ok_or(FittingStoreError::AttemptNotFound)
    }

    pub fn attempts_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<TransformationFittingAttempt>, FittingStoreError> {
        validate_user(user_id)?;
        Ok(self
            .attempts
            .values()
            .filter(|attempt| attempt.user_id == user_id)
            .cloned()
            .collect())
    }

    pub fn mark_running(
        &mut self,
        user_id: &str,
        attempt_id: &str,
    ) -> Result<TransformationFittingAttempt, FittingStoreError> {
        let attempt = self.attempt_mut(user_id, attempt_id)?;
        if attempt.status != TransformationFittingAttemptStatus::Pending {
            return Err(FittingStoreError::InvalidTransition);
        }
        attempt.status = TransformationFittingAttemptStatus::Running;
        Ok(attempt.clone())
    }

    pub fn fail(
        &mut self,
        user_id: &str,
        attempt_id: &str,
        code: &str,
    ) -> Result<TransformationFittingAttempt, FittingStoreError> {
        let attempt = self.attempt_mut(user_id, attempt_id)?;
        if !matches!(
            attempt.status,
            TransformationFittingAttemptStatus::Pending
                | TransformationFittingAttemptStatus::Running
        ) {
            return Err(FittingStoreError::InvalidTransition);
        }
        attempt.status = TransformationFittingAttemptStatus::Failed;
        attempt.failure_code = Some(code.into());
        Ok(attempt.clone())
    }

    pub fn cancel(
        &mut self,
        user_id: &str,
        attempt_id: &str,
    ) -> Result<TransformationFittingAttempt, FittingStoreError> {
        let attempt = self.attempt_mut(user_id, attempt_id)?;
        if !matches!(
            attempt.status,
            TransformationFittingAttemptStatus::Pending
                | TransformationFittingAttemptStatus::Running
        ) {
            return Err(FittingStoreError::InvalidTransition);
        }
        attempt.status = TransformationFittingAttemptStatus::Cancelled;
        attempt.failure_code = Some("cancelled".into());
        Ok(attempt.clone())
    }

    pub fn retry(
        &mut self,
        user_id: &str,
        attempt_id: &str,
    ) -> Result<TransformationFittingAttempt, FittingStoreError> {
        let previous = self.get_attempt(user_id, attempt_id)?;
        if !matches!(
            previous.status,
            TransformationFittingAttemptStatus::Failed
                | TransformationFittingAttemptStatus::Cancelled
        ) {
            return Err(FittingStoreError::InvalidTransition);
        }
        if let Some(existing) = self.active_attempt(user_id, &previous.protocol_hash) {
            return Ok(existing);
        }
        let attempt = TransformationFittingAttempt {
            attempt_id: Uuid::new_v4().to_string(),
            user_id: user_id.into(),
            protocol_hash: previous.protocol_hash,
            status: TransformationFittingAttemptStatus::Pending,
            artifact_id: None,
            failure_code: None,
        };
        self.attempts
            .insert(attempt.attempt_id.clone(), attempt.clone());
        Ok(attempt)
    }

    pub fn publish_completed(
        &mut self,
        user_id: &str,
        attempt_id: &str,
        artifact: FittedTransformationArtifact,
    ) -> Result<TransformationFittingAttempt, FittingStoreError> {
        let attempt = self.get_attempt(user_id, attempt_id)?;
        if attempt.status != TransformationFittingAttemptStatus::Running {
            return Err(FittingStoreError::InvalidTransition);
        }
        if artifact.protocol_hash != attempt.protocol_hash {
            return Err(FittingStoreError::ProtocolMismatch);
        }
        let Some(protocol) = self.protocols.get(&attempt.protocol_hash) else {
            return Err(FittingStoreError::InvalidArtifact);
        };
        if !validate_artifact(&artifact) || !artifact_matches_protocol(&artifact, protocol) {
            return Err(FittingStoreError::InvalidArtifact);
        }
        if let Some(existing) = self.artifacts.get(artifact.artifact_id()) {
            if existing.content() != artifact.content() {
                return Err(FittingStoreError::ArtifactIdCollision);
            }
        } else {
            self.artifacts
                .insert(artifact.artifact_id().into(), artifact.clone());
        }
        self.artifact_access
            .insert((user_id.into(), artifact.artifact_id().into()));
        let attempt = self.attempt_mut(user_id, attempt_id)?;
        attempt.status = TransformationFittingAttemptStatus::Completed;
        attempt.artifact_id = Some(artifact.artifact_id().into());
        attempt.failure_code = None;
        Ok(attempt.clone())
    }

    pub fn artifact_for_user(
        &self,
        user_id: &str,
        artifact_id: &str,
    ) -> Result<FittedTransformationArtifact, FittingStoreError> {
        if !self
            .artifact_access
            .contains(&(user_id.into(), artifact_id.into()))
        {
            return Err(FittingStoreError::ArtifactNotFound);
        }
        self.artifacts
            .get(artifact_id)
            .cloned()
            .ok_or(FittingStoreError::ArtifactNotFound)
    }

    pub fn artifacts_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<FittedTransformationArtifact>, FittingStoreError> {
        validate_user(user_id)?;
        self.artifact_access
            .iter()
            .filter(|(owner, _)| owner == user_id)
            .map(|(_, artifact_id)| self.artifact_for_user(user_id, artifact_id))
            .collect()
    }

    pub fn stored_artifact_count(&self) -> usize {
        self.artifacts.len()
    }

    pub fn reference_artifact(
        &mut self,
        user_id: &str,
        artifact_id: &str,
        reference_id: &str,
    ) -> Result<(), FittingStoreError> {
        if reference_id.trim().is_empty() {
            return Err(FittingStoreError::InvalidReference);
        }
        self.artifact_for_user(user_id, artifact_id)?;
        self.artifact_references
            .insert((user_id.into(), artifact_id.into(), reference_id.into()));
        Ok(())
    }

    pub fn unreference_artifact(
        &mut self,
        user_id: &str,
        artifact_id: &str,
        reference_id: &str,
    ) -> Result<(), FittingStoreError> {
        if !self.artifact_references.remove(&(
            user_id.into(),
            artifact_id.into(),
            reference_id.into(),
        )) {
            return Err(FittingStoreError::ArtifactNotFound);
        }
        Ok(())
    }

    pub fn delete_artifact(
        &mut self,
        user_id: &str,
        artifact_id: &str,
    ) -> Result<(), FittingStoreError> {
        self.artifact_for_user(user_id, artifact_id)?;
        if self
            .artifact_references
            .iter()
            .any(|(owner, id, _)| owner == user_id && id == artifact_id)
        {
            return Err(FittingStoreError::ArtifactReferenced);
        }
        self.artifact_access
            .remove(&(user_id.into(), artifact_id.into()));
        if !self.artifact_access.iter().any(|(_, id)| id == artifact_id) {
            self.artifacts.remove(artifact_id);
        }
        Ok(())
    }

    fn attempt_mut(
        &mut self,
        user_id: &str,
        attempt_id: &str,
    ) -> Result<&mut TransformationFittingAttempt, FittingStoreError> {
        self.attempts
            .get_mut(attempt_id)
            .filter(|attempt| attempt.user_id == user_id)
            .ok_or(FittingStoreError::AttemptNotFound)
    }

    fn reusable_attempt(
        &self,
        user_id: &str,
        protocol_hash: &str,
    ) -> Option<TransformationFittingAttempt> {
        self.attempts
            .values()
            .find(|attempt| {
                attempt.user_id == user_id
                    && attempt.protocol_hash == protocol_hash
                    && match attempt.status {
                        TransformationFittingAttemptStatus::Pending
                        | TransformationFittingAttemptStatus::Running => true,
                        TransformationFittingAttemptStatus::Completed => {
                            attempt.artifact_id.as_ref().is_some_and(|id| {
                                self.artifact_access.contains(&(user_id.into(), id.clone()))
                            })
                        }
                        TransformationFittingAttemptStatus::Failed
                        | TransformationFittingAttemptStatus::Cancelled => false,
                    }
            })
            .cloned()
    }

    fn active_attempt(
        &self,
        user_id: &str,
        protocol_hash: &str,
    ) -> Option<TransformationFittingAttempt> {
        self.attempts
            .values()
            .find(|attempt| {
                attempt.user_id == user_id
                    && attempt.protocol_hash == protocol_hash
                    && matches!(
                        attempt.status,
                        TransformationFittingAttemptStatus::Pending
                            | TransformationFittingAttemptStatus::Running
                    )
            })
            .cloned()
    }
}

fn validate_user(user_id: &str) -> Result<(), FittingStoreError> {
    (!user_id.trim().is_empty())
        .then_some(())
        .ok_or(FittingStoreError::InvalidUser)
}

impl fmt::Debug for FittedTransformationValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Available {
                value,
                available_at_ms,
            } => formatter
                .debug_struct("Available")
                .field("value", value)
                .field("available_at_ms", available_at_ms)
                .finish(),
            Self::Unavailable(reason) => {
                formatter.debug_tuple("Unavailable").field(reason).finish()
            }
        }
    }
}
