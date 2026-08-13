use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    ContractError, ContractLoadError, FACTOR_ABI_VERSION, FACTOR_RESEARCH_SCHEMA_VERSION,
    FactorMetricCatalog, MAX_FACTOR_OUTPUTS, MAX_FACTOR_SLOTS, MAX_GRID_SEARCH_TRIALS,
    canonical_json, checked_product, content_hash, is_lower_kebab, is_sha256, load_versioned_json,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FactorScope {
    TimeSeries,
    CrossSectional,
}

impl FactorScope {
    pub const fn world(self) -> &'static str {
        match self {
            Self::TimeSeries => "time-series",
            Self::CrossSectional => "cross-sectional",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactorFeatureSlot {
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FactorParameterType {
    Decimal,
    Integer,
    Boolean,
    Text,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactorParameter {
    pub name: String,
    pub parameter_type: FactorParameterType,
    pub default_value: String,
    #[serde(default)]
    pub allowed_values: Vec<String>,
}

/// A Declarative Candidate is a reference to an already-frozen M10 Feature
/// Plan projection. Factor Research does not introduce a second expression
/// language or silently re-evaluate Feature definitions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeclarativeFactorDefinition {
    pub feature_plan_hash: String,
    pub operator_catalog_version: String,
    pub outputs: Vec<DeclarativeFactorOutputBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeclarativeFactorOutputBinding {
    pub output_name: String,
    pub feature_slot: String,
}

impl DeclarativeFactorDefinition {
    pub fn validate(
        &self,
        feature_slots: &[FactorFeatureSlot],
        outputs: &[FactorOutput],
    ) -> Result<(), ContractError> {
        if !is_sha256(&self.feature_plan_hash)
            || self.operator_catalog_version
                != adaq_feature_engine::FEATURE_OPERATOR_CATALOG_VERSION
            || self.outputs.len() != outputs.len()
        {
            return Err(ContractError::Invalid(
                "Declarative Factor must bind one current Feature Plan and its operator catalog"
                    .into(),
            ));
        }
        let slot_names = feature_slots
            .iter()
            .map(|slot| slot.name.as_str())
            .collect::<BTreeSet<_>>();
        let output_names = outputs
            .iter()
            .map(|output| output.name.as_str())
            .collect::<BTreeSet<_>>();
        let mut seen = BTreeSet::new();
        if self.outputs.iter().any(|binding| {
            !output_names.contains(binding.output_name.as_str())
                || !slot_names.contains(binding.feature_slot.as_str())
                || !seen.insert(binding.output_name.as_str())
        }) || seen.len() != output_names.len()
        {
            return Err(ContractError::Invalid(
                "Declarative Factor outputs must map exactly to ordered Feature Slots".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactorOutput {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CandidateBuildProvenance {
    pub attempt_id: Uuid,
    pub source_sha256: String,
    pub sdk_version: String,
    pub abi_version: String,
    pub toolchain: String,
    pub compiler: String,
    pub target: String,
    pub commands: Vec<String>,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    pub resource_policy: FactorResourcePolicy,
    #[serde(default)]
    pub diagnostic_log_sha256: Option<String>,
    pub package_sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactorResourcePolicy {
    pub fuel_per_call: u64,
    pub memory_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum FactorCandidateSource {
    Declarative {
        definition: DeclarativeFactorDefinition,
    },
    Custom {
        build: CandidateBuildProvenance,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactorCandidateDraft {
    pub candidate_id: Uuid,
    pub revision: u64,
    pub scope: FactorScope,
    pub feature_slots: Vec<FactorFeatureSlot>,
    pub parameters: Vec<FactorParameter>,
    pub outputs: Vec<FactorOutput>,
    pub source: FactorCandidateSource,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactorCandidate {
    pub schema_version: String,
    pub candidate_id: Uuid,
    pub revision: u64,
    pub scope: FactorScope,
    pub feature_slots: Vec<FactorFeatureSlot>,
    pub parameters: Vec<FactorParameter>,
    pub outputs: Vec<FactorOutput>,
    pub source: FactorCandidateSource,
    pub candidate_hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct FactorCandidateContent<'a> {
    schema_version: &'a str,
    candidate_id: Uuid,
    revision: u64,
    scope: FactorScope,
    feature_slots: &'a [FactorFeatureSlot],
    parameters: &'a [FactorParameter],
    outputs: &'a [FactorOutput],
    source: &'a FactorCandidateSource,
}

impl FactorCandidate {
    pub fn freeze(draft: FactorCandidateDraft) -> Result<Self, ContractError> {
        validate_factor_shape(
            draft.scope,
            &draft.feature_slots,
            &draft.parameters,
            &draft.outputs,
        )?;
        validate_candidate_source(&draft.source)?;
        if let FactorCandidateSource::Declarative { definition } = &draft.source {
            definition.validate(&draft.feature_slots, &draft.outputs)?;
        }
        if draft.candidate_id.is_nil() || draft.revision == 0 {
            return Err(ContractError::Invalid(
                "Factor Candidate requires a non-nil identity and positive revision".into(),
            ));
        }
        let mut candidate = Self {
            schema_version: FACTOR_RESEARCH_SCHEMA_VERSION.into(),
            candidate_id: draft.candidate_id,
            revision: draft.revision,
            scope: draft.scope,
            feature_slots: draft.feature_slots,
            parameters: draft.parameters,
            outputs: draft.outputs,
            source: draft.source,
            candidate_hash: String::new(),
        };
        let candidate_hash = content_hash(&candidate.content())?;
        candidate.candidate_hash = candidate_hash;
        Ok(candidate)
    }

    pub fn validate(&self) -> Result<(), ContractError> {
        if self.schema_version != FACTOR_RESEARCH_SCHEMA_VERSION
            || self.candidate_id.is_nil()
            || self.revision == 0
        {
            return Err(ContractError::Invalid(
                "Factor Candidate schema or identity is invalid".into(),
            ));
        }
        validate_factor_shape(
            self.scope,
            &self.feature_slots,
            &self.parameters,
            &self.outputs,
        )?;
        validate_candidate_source(&self.source)?;
        if let FactorCandidateSource::Declarative { definition } = &self.source {
            definition.validate(&self.feature_slots, &self.outputs)?;
        }
        if !is_sha256(&self.candidate_hash) || self.candidate_hash != content_hash(&self.content())?
        {
            return Err(ContractError::HashMismatch);
        }
        Ok(())
    }

    pub fn to_json(&self) -> Result<Vec<u8>, ContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn load(bytes: &[u8]) -> Result<Self, ContractLoadError> {
        let candidate: Self = load_versioned_json(bytes, FACTOR_RESEARCH_SCHEMA_VERSION)?;
        candidate.validate().map_err(|error| match error {
            ContractError::HashMismatch => ContractLoadError::HashMismatch,
            ContractError::ResetRequired {
                stored_schema_version,
                guidance,
            } => ContractLoadError::ResetRequired {
                stored_schema_version,
                guidance,
            },
            other => ContractLoadError::InvalidContract(other.to_string()),
        })?;
        Ok(candidate)
    }

    pub fn content(&self) -> impl Serialize + '_ {
        FactorCandidateContent {
            schema_version: &self.schema_version,
            candidate_id: self.candidate_id,
            revision: self.revision,
            scope: self.scope,
            feature_slots: &self.feature_slots,
            parameters: &self.parameters,
            outputs: &self.outputs,
            source: &self.source,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactorAbiContract {
    pub abi_version: String,
    pub scope: FactorScope,
    pub feature_slots: Vec<FactorFeatureSlot>,
    pub parameters: Vec<FactorParameter>,
    pub outputs: Vec<FactorOutput>,
    pub warmup_bars: u32,
    pub resource_policy: FactorResourcePolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactorAbiIdentity {
    pub abi_version: String,
    pub world: String,
    pub contract_hash: String,
}

impl FactorAbiContract {
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.abi_version != FACTOR_ABI_VERSION {
            return Err(ContractError::ResetRequired {
                stored_schema_version: Some(self.abi_version.clone()),
                guidance: "Factor ABI v1 evidence is incompatible; reset the device-level Factor evidence explicitly".into(),
            });
        }
        validate_factor_shape(
            self.scope,
            &self.feature_slots,
            &self.parameters,
            &self.outputs,
        )?;
        if self.resource_policy.fuel_per_call == 0 || self.resource_policy.memory_bytes == 0 {
            return Err(ContractError::Invalid(
                "Factor resource policy must be non-zero".into(),
            ));
        }
        Ok(())
    }

    pub fn identity(&self) -> Result<FactorAbiIdentity, ContractError> {
        self.validate()?;
        Ok(FactorAbiIdentity {
            abi_version: self.abi_version.clone(),
            world: self.scope.world().into(),
            contract_hash: content_hash(self)?,
        })
    }
}

fn validate_factor_shape(
    _scope: FactorScope,
    feature_slots: &[FactorFeatureSlot],
    parameters: &[FactorParameter],
    outputs: &[FactorOutput],
) -> Result<(), ContractError> {
    if feature_slots.is_empty() {
        return Err(ContractError::Invalid(
            "Factor Feature Slots must contain at least one ordered slot".into(),
        ));
    }
    if feature_slots.len() > MAX_FACTOR_SLOTS {
        return Err(ContractError::LimitExceeded {
            name: "Factor Feature Slots",
            limit: MAX_FACTOR_SLOTS,
        });
    }
    if outputs.is_empty() || outputs.len() > MAX_FACTOR_OUTPUTS {
        return Err(ContractError::Invalid(format!(
            "Factor outputs must contain 1..={MAX_FACTOR_OUTPUTS} entries"
        )));
    }
    unique_names(
        feature_slots.iter().map(|slot| slot.name.as_str()),
        "Feature Slot",
    )?;
    unique_names(
        parameters.iter().map(|parameter| parameter.name.as_str()),
        "parameter",
    )?;
    unique_names(outputs.iter().map(|output| output.name.as_str()), "output")?;
    Ok(())
}

fn unique_names<'a>(
    names: impl IntoIterator<Item = &'a str>,
    kind: &str,
) -> Result<(), ContractError> {
    let mut seen = BTreeSet::new();
    for name in names {
        if !is_lower_kebab(name) || !seen.insert(name) {
            return Err(ContractError::Invalid(format!(
                "{kind} names must be unique lower-kebab-case ASCII identifiers"
            )));
        }
    }
    Ok(())
}

fn validate_candidate_source(source: &FactorCandidateSource) -> Result<(), ContractError> {
    match source {
        FactorCandidateSource::Declarative { .. } => Ok(()),
        FactorCandidateSource::Custom { build } => {
            if build.abi_version != FACTOR_ABI_VERSION {
                return Err(ContractError::ResetRequired {
                    stored_schema_version: Some(build.abi_version.clone()),
                    guidance: "incompatible Factor ABI evidence requires an explicit device-level reset; no migration or automatic deletion is performed".into(),
                });
            }
            if build.attempt_id.is_nil()
                || build.sdk_version.is_empty()
                || build.toolchain.is_empty()
                || build.compiler.is_empty()
                || build.target.is_empty()
                || build.commands.is_empty()
                || build.resource_policy.fuel_per_call == 0
                || build.resource_policy.memory_bytes == 0
                || !is_sha256(&build.source_sha256)
                || !is_sha256(&build.package_sha256)
                || build
                    .diagnostic_log_sha256
                    .as_deref()
                    .is_some_and(|hash| !is_sha256(hash))
            {
                return Err(ContractError::Invalid(
                    "Custom Factor build provenance is incomplete or invalid".into(),
                ));
            }
            Ok(())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FactorParameterValue {
    Decimal(String),
    Integer(i64),
    Boolean(bool),
    Text(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FactorUnavailabilityReason {
    Warmup,
    BarGap,
    MissingInput,
    MissingDependency,
    NotYetAvailable,
    UnknownUniverse,
    InsufficientCoverage,
    UndefinedArithmetic,
    InvalidUpstream,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AvailableFactorValue {
    pub value: f64,
    pub available_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum FactorSlotCell {
    Available(AvailableFactorValue),
    Unavailable(FactorUnavailabilityReason),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TimeSeriesInputRow {
    pub instrument_id: String,
    pub observation_time_ms: i64,
    pub slots: Vec<AvailableFactorValue>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CrossSectionalInputRow {
    pub instrument_id: String,
    pub observation_time_ms: i64,
    pub slots: Vec<FactorSlotCell>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NamedFactorOutput {
    pub name: String,
    pub value: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactorResult {
    pub instrument_id: String,
    pub observation_time_ms: i64,
    pub values: Option<Vec<NamedFactorOutput>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum FactorObservationValue {
    Available { value: f64, available_at_ms: i64 },
    Unavailable { reason: FactorUnavailabilityReason },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactorDatasetManifest {
    pub schema_version: String,
    pub dataset_id: String,
    pub protocol_hash: String,
    pub candidate_hash: String,
    pub scope: FactorScope,
    pub feature_dataset_id: String,
    pub feature_plan_hash: String,
    pub market_data_snapshot_id: String,
    pub point_in_time_universe_id: String,
    pub market_context: FactorMarketContext,
    pub output_names: Vec<String>,
    pub observation_count: u64,
    pub payload_sha256: String,
    pub engine_identity: ResearchEngineProvenance,
}

impl FactorDatasetManifest {
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.schema_version != FACTOR_RESEARCH_SCHEMA_VERSION
            || self.dataset_id.is_empty()
            || !is_sha256(&self.protocol_hash)
            || !is_sha256(&self.candidate_hash)
            || self.feature_dataset_id.is_empty()
            || !is_sha256(&self.feature_plan_hash)
            || self.market_data_snapshot_id.is_empty()
            || self.point_in_time_universe_id.is_empty()
            || !is_sha256(&self.payload_sha256)
        {
            return Err(ContractError::Invalid(
                "Factor Dataset Manifest identity is invalid".into(),
            ));
        }
        self.market_context.validate()?;
        if self.market_context.point_in_time_universe_id != self.point_in_time_universe_id
            || self.market_context.bar_interval.is_empty()
        {
            return Err(ContractError::Invalid(
                "Factor Dataset market context and Universe identity differ".into(),
            ));
        }
        unique_names(self.output_names.iter().map(String::as_str), "output")?;
        self.engine_identity.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObservationRange {
    pub start_time_ms: i64,
    pub end_time_ms: i64,
}

impl ObservationRange {
    pub fn validate(&self) -> Result<(), ContractError> {
        (self.start_time_ms < self.end_time_ms)
            .then_some(())
            .ok_or_else(|| ContractError::Invalid("observation range must be non-empty".into()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactorMarketContext {
    pub venue: String,
    pub asset_class: String,
    pub bar_interval: String,
    pub price_basis: String,
    pub valuation_currency: String,
    pub point_in_time_universe_id: String,
}

impl FactorMarketContext {
    pub fn validate(&self) -> Result<(), ContractError> {
        if [
            &self.venue,
            &self.asset_class,
            &self.bar_interval,
            &self.price_basis,
            &self.valuation_currency,
            &self.point_in_time_universe_id,
        ]
        .iter()
        .any(|value| value.is_empty())
        {
            return Err(ContractError::Invalid(
                "Factor market context must be complete".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResearchEngineProvenance {
    pub engine_id: String,
    pub engine_version: String,
    pub adapter: String,
    pub target_triple: String,
    pub build_id: String,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    #[serde(default)]
    pub parameters: BTreeMap<String, String>,
    pub input_identities: Vec<String>,
}

impl ResearchEngineProvenance {
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.engine_id.is_empty()
            || self.engine_version.is_empty()
            || self.adapter.is_empty()
            || self.target_triple.is_empty()
            || self.build_id.is_empty()
            || self.input_identities.is_empty()
        {
            return Err(ContractError::Invalid(
                "Research Engine Provenance must identify engine, build, and inputs".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactorMaterializationProtocolDraft {
    pub protocol_id: Uuid,
    pub user_id: Uuid,
    pub candidate_hash: String,
    pub feature_dataset_id: String,
    pub feature_plan_hash: String,
    pub parameters: Vec<FactorParameterValue>,
    pub market_data_snapshot_id: String,
    pub point_in_time_universe_id: String,
    pub observation_range: ObservationRange,
    pub market_context: FactorMarketContext,
    pub engine_identity: ResearchEngineProvenance,
    pub seed: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactorMaterializationProtocol {
    pub schema_version: String,
    pub protocol_id: Uuid,
    pub user_id: Uuid,
    pub candidate_hash: String,
    pub feature_dataset_id: String,
    pub feature_plan_hash: String,
    pub parameters: Vec<FactorParameterValue>,
    pub market_data_snapshot_id: String,
    pub point_in_time_universe_id: String,
    pub observation_range: ObservationRange,
    pub market_context: FactorMarketContext,
    pub engine_identity: ResearchEngineProvenance,
    pub seed: u64,
    pub protocol_hash: String,
}

impl FactorMaterializationProtocol {
    pub fn freeze(draft: FactorMaterializationProtocolDraft) -> Result<Self, ContractError> {
        let mut protocol = Self {
            schema_version: FACTOR_RESEARCH_SCHEMA_VERSION.into(),
            protocol_id: draft.protocol_id,
            user_id: draft.user_id,
            candidate_hash: draft.candidate_hash,
            feature_dataset_id: draft.feature_dataset_id,
            feature_plan_hash: draft.feature_plan_hash,
            parameters: draft.parameters,
            market_data_snapshot_id: draft.market_data_snapshot_id,
            point_in_time_universe_id: draft.point_in_time_universe_id,
            observation_range: draft.observation_range,
            market_context: draft.market_context,
            engine_identity: draft.engine_identity,
            seed: draft.seed,
            protocol_hash: String::new(),
        };
        protocol.validate_without_hash()?;
        let protocol_hash = content_hash(&protocol.content())?;
        protocol.protocol_hash = protocol_hash;
        Ok(protocol)
    }

    pub fn validate(&self) -> Result<(), ContractError> {
        self.validate_without_hash()?;
        if !is_sha256(&self.protocol_hash) || self.protocol_hash != content_hash(&self.content())? {
            return Err(ContractError::HashMismatch);
        }
        Ok(())
    }

    fn validate_without_hash(&self) -> Result<(), ContractError> {
        if self.schema_version != FACTOR_RESEARCH_SCHEMA_VERSION
            || self.protocol_id.is_nil()
            || self.user_id.is_nil()
            || !is_sha256(&self.candidate_hash)
            || !is_sha256(&self.feature_plan_hash)
            || self.feature_dataset_id.is_empty()
            || self.market_data_snapshot_id.is_empty()
            || self.point_in_time_universe_id.is_empty()
        {
            return Err(ContractError::Invalid(
                "Factor Materialization Protocol identity is invalid".into(),
            ));
        }
        self.observation_range.validate()?;
        self.market_context.validate()?;
        self.engine_identity.validate()
    }

    fn content(&self) -> impl Serialize + '_ {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Content<'a> {
            schema_version: &'a str,
            protocol_id: Uuid,
            user_id: Uuid,
            candidate_hash: &'a str,
            feature_dataset_id: &'a str,
            feature_plan_hash: &'a str,
            parameters: &'a [FactorParameterValue],
            market_data_snapshot_id: &'a str,
            point_in_time_universe_id: &'a str,
            observation_range: &'a ObservationRange,
            market_context: &'a FactorMarketContext,
            engine_identity: &'a ResearchEngineProvenance,
            seed: u64,
        }
        Content {
            schema_version: &self.schema_version,
            protocol_id: self.protocol_id,
            user_id: self.user_id,
            candidate_hash: &self.candidate_hash,
            feature_dataset_id: &self.feature_dataset_id,
            feature_plan_hash: &self.feature_plan_hash,
            parameters: &self.parameters,
            market_data_snapshot_id: &self.market_data_snapshot_id,
            point_in_time_universe_id: &self.point_in_time_universe_id,
            observation_range: &self.observation_range,
            market_context: &self.market_context,
            engine_identity: &self.engine_identity,
            seed: self.seed,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AttemptStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl AttemptStatus {
    pub const fn terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactorMaterializationAttempt {
    pub attempt_id: Uuid,
    pub user_id: Uuid,
    pub protocol_hash: String,
    pub status: AttemptStatus,
    pub source_attempt_id: Option<Uuid>,
    pub completed_units: u64,
    pub diagnostic: Option<String>,
}

impl FactorMaterializationAttempt {
    pub fn new(
        attempt_id: Uuid,
        user_id: Uuid,
        protocol_hash: String,
    ) -> Result<Self, ContractError> {
        if attempt_id.is_nil() || user_id.is_nil() || !is_sha256(&protocol_hash) {
            return Err(ContractError::Invalid(
                "Factor Materialization Attempt identity is invalid".into(),
            ));
        }
        Ok(Self {
            attempt_id,
            user_id,
            protocol_hash,
            status: AttemptStatus::Pending,
            source_attempt_id: None,
            completed_units: 0,
            diagnostic: None,
        })
    }

    pub fn transition(&mut self, next: AttemptStatus) -> Result<(), ContractError> {
        let allowed = matches!(
            (self.status, next),
            (
                AttemptStatus::Pending,
                AttemptStatus::Running | AttemptStatus::Failed | AttemptStatus::Cancelled
            ) | (
                AttemptStatus::Running,
                AttemptStatus::Completed | AttemptStatus::Failed | AttemptStatus::Cancelled
            )
        );
        if !allowed {
            return Err(ContractError::Invalid(format!(
                "invalid Factor Materialization Attempt transition: {:?} -> {:?}",
                self.status, next
            )));
        }
        self.status = next;
        Ok(())
    }

    pub fn retry(&self, attempt_id: Uuid) -> Result<Self, ContractError> {
        if !matches!(
            self.status,
            AttemptStatus::Failed | AttemptStatus::Cancelled
        ) || attempt_id.is_nil()
        {
            return Err(ContractError::Invalid(
                "only a terminal Attempt can be retried with a new identity".into(),
            ));
        }
        Ok(Self {
            attempt_id,
            user_id: self.user_id,
            protocol_hash: self.protocol_hash.clone(),
            status: AttemptStatus::Pending,
            source_attempt_id: Some(self.attempt_id),
            completed_units: 0,
            diagnostic: None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FactorTarget {
    FutureCloseReturn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FactorOrientation {
    Positive,
    Negative,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FactorLens {
    Temporal,
    CrossSectional,
    Economic,
    Neutralized,
    Regime,
}

impl FactorLens {
    pub fn required(scope: FactorScope) -> [Self; 2] {
        match scope {
            FactorScope::TimeSeries => [Self::Temporal, Self::Economic],
            FactorScope::CrossSectional => [Self::CrossSectional, Self::Economic],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EconomicAssumptions {
    pub rebalance_every_bars: u32,
    pub fee_bps: f64,
    pub slippage_bps: f64,
    pub long_short: bool,
}

impl EconomicAssumptions {
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.rebalance_every_bars == 0
            || !self.fee_bps.is_finite()
            || self.fee_bps < 0.0
            || !self.slippage_bps.is_finite()
            || self.slippage_bps < 0.0
        {
            return Err(ContractError::Invalid(
                "Economic assumptions must use positive rebalance bars and finite non-negative costs".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactorRegimeDefinition {
    pub feature_name: String,
    pub bucket_count: u8,
}

impl FactorRegimeDefinition {
    pub fn validate(&self) -> Result<(), ContractError> {
        if !is_lower_kebab(&self.feature_name) || !(2..=5).contains(&self.bucket_count) {
            return Err(ContractError::Invalid(
                "Factor Regime Definition requires one lower-kebab feature and 2..=5 buckets"
                    .into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EvaluationEvidenceState {
    OutOfSample,
    Overlapping,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvaluationWindow {
    pub fold_id: String,
    pub selection: ObservationRange,
    pub evaluation: ObservationRange,
    pub training: Option<ObservationRange>,
    pub fitting: Option<ObservationRange>,
    pub normalization: Option<ObservationRange>,
    pub target_construction: Option<ObservationRange>,
}

impl EvaluationWindow {
    pub fn validate(&self) -> Result<EvaluationEvidenceState, ContractError> {
        if !is_lower_kebab(&self.fold_id) {
            return Err(ContractError::Invalid(
                "Evaluation Window fold identity must be lower-kebab-case".into(),
            ));
        }
        self.selection.validate()?;
        self.evaluation.validate()?;
        if self.selection.end_time_ms > self.evaluation.start_time_ms {
            return Err(ContractError::Invalid(
                "Evaluation Window selection must end before evaluation begins".into(),
            ));
        }
        let influencing = [
            self.training.as_ref(),
            self.fitting.as_ref(),
            self.normalization.as_ref(),
            self.target_construction.as_ref(),
        ];
        for range in influencing.into_iter().flatten() {
            range.validate()?;
            if range.start_time_ms >= self.evaluation.end_time_ms {
                return Err(ContractError::Invalid(
                    "Evaluation Window provenance cannot be after evaluation".into(),
                ));
            }
            if ranges_overlap(range, &self.evaluation) {
                return Ok(EvaluationEvidenceState::Overlapping);
            }
        }
        if ranges_overlap(&self.selection, &self.evaluation) {
            return Ok(EvaluationEvidenceState::Overlapping);
        }
        if influencing.iter().any(Option::is_none) {
            Ok(EvaluationEvidenceState::Unknown)
        } else {
            Ok(EvaluationEvidenceState::OutOfSample)
        }
    }
}

fn ranges_overlap(left: &ObservationRange, right: &ObservationRange) -> bool {
    left.start_time_ms < right.end_time_ms && right.start_time_ms < left.end_time_ms
}

fn validate_universe(universe: &[String]) -> Result<(), ContractError> {
    if universe.is_empty()
        || universe
            .iter()
            .any(|instrument| instrument.trim().is_empty())
        || universe.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err(ContractError::Invalid(
            "Point-in-Time Universe must be non-empty, unique, and deterministically ordered"
                .into(),
        ));
    }
    Ok(())
}

fn validate_feature_names(names: &[String]) -> Result<(), ContractError> {
    let mut seen = BTreeSet::new();
    if names
        .iter()
        .any(|name| !is_lower_kebab(name) || !seen.insert(name.as_str()))
    {
        return Err(ContractError::Invalid(
            "Evaluation Feature names must be unique lower-kebab-case identifiers".into(),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactorEvaluationProtocolDraft {
    pub protocol_id: Uuid,
    pub user_id: Uuid,
    pub factor_dataset_id: String,
    pub feature_dataset_id: String,
    pub feature_plan_hash: String,
    pub market_data_snapshot_id: String,
    pub point_in_time_universe_id: String,
    pub point_in_time_universe: Vec<String>,
    pub output_name: String,
    pub scope: FactorScope,
    pub target: FactorTarget,
    pub horizon_bars: Vec<u32>,
    pub market_context: FactorMarketContext,
    pub engine_identity: ResearchEngineProvenance,
    pub orientation: FactorOrientation,
    pub windows: Vec<EvaluationWindow>,
    pub purge_bars: u32,
    pub embargo_bars: u32,
    pub lenses: Vec<FactorLens>,
    pub nuisance_feature_names: Vec<String>,
    pub regime: Option<FactorRegimeDefinition>,
    pub economic: EconomicAssumptions,
    pub family_id: Uuid,
    pub trial_id: Uuid,
    pub seed: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactorEvaluationProtocol {
    pub schema_version: String,
    pub protocol_id: Uuid,
    pub user_id: Uuid,
    pub factor_dataset_id: String,
    pub feature_dataset_id: String,
    pub feature_plan_hash: String,
    pub market_data_snapshot_id: String,
    pub point_in_time_universe_id: String,
    pub point_in_time_universe: Vec<String>,
    pub output_name: String,
    pub scope: FactorScope,
    pub target: FactorTarget,
    pub horizon_bars: Vec<u32>,
    pub market_context: FactorMarketContext,
    pub engine_identity: ResearchEngineProvenance,
    pub orientation: FactorOrientation,
    pub windows: Vec<EvaluationWindow>,
    pub purge_bars: u32,
    pub embargo_bars: u32,
    pub lenses: Vec<FactorLens>,
    pub nuisance_feature_names: Vec<String>,
    pub regime: Option<FactorRegimeDefinition>,
    pub economic: EconomicAssumptions,
    pub family_id: Uuid,
    pub trial_id: Uuid,
    pub seed: u64,
    pub protocol_hash: String,
}

impl FactorEvaluationProtocol {
    pub fn freeze(draft: FactorEvaluationProtocolDraft) -> Result<Self, ContractError> {
        if draft.protocol_id.is_nil()
            || draft.user_id.is_nil()
            || draft.family_id.is_nil()
            || draft.trial_id.is_nil()
            || draft.factor_dataset_id.is_empty()
            || draft.feature_dataset_id.is_empty()
            || !is_sha256(&draft.feature_plan_hash)
            || draft.market_data_snapshot_id.is_empty()
            || draft.point_in_time_universe_id.is_empty()
            || draft.point_in_time_universe.is_empty()
            || !is_lower_kebab(&draft.output_name)
            || draft.horizon_bars.is_empty()
            || draft.horizon_bars.len() > crate::MAX_FACTOR_EVALUATION_HORIZONS
            || draft.horizon_bars.iter().any(|horizon| *horizon == 0)
            || draft.windows.is_empty()
            || draft.windows.len() > crate::MAX_FACTOR_EVALUATION_FOLDS
            || draft.lenses.len() > crate::MAX_FACTOR_EVALUATION_LENSES
            || draft.nuisance_feature_names.len() > crate::MAX_FACTOR_NUISANCE_FEATURES
        {
            return Err(ContractError::Invalid(
                "Factor Evaluation Protocol identity or horizon is invalid".into(),
            ));
        }
        draft.market_context.validate()?;
        draft.engine_identity.validate()?;
        validate_universe(&draft.point_in_time_universe)?;
        validate_feature_names(&draft.nuisance_feature_names)?;
        if let Some(regime) = &draft.regime {
            regime.validate()?;
        }
        draft.economic.validate()?;
        let required = FactorLens::required(draft.scope);
        if required.iter().any(|lens| !draft.lenses.contains(lens)) {
            return Err(ContractError::Invalid(
                "Factor Evaluation Protocol is missing a scope-compatible and Economic Lens".into(),
            ));
        }
        if draft.lenses.is_empty() {
            return Err(ContractError::Invalid(
                "Factor Evaluation Protocol requires a Lens".into(),
            ));
        }
        if draft.market_context.point_in_time_universe_id != draft.point_in_time_universe_id
            || (draft.scope == FactorScope::TimeSeries
                && draft.lenses.iter().any(|lens| {
                    matches!(lens, FactorLens::CrossSectional | FactorLens::Neutralized)
                }))
            || (draft.scope == FactorScope::CrossSectional
                && draft.lenses.contains(&FactorLens::Temporal))
            || (draft.lenses.contains(&FactorLens::Neutralized)
                && draft.nuisance_feature_names.is_empty())
            || (draft.lenses.contains(&FactorLens::Regime) && draft.regime.is_none())
            || (!draft.lenses.contains(&FactorLens::Regime) && draft.regime.is_some())
        {
            return Err(ContractError::Invalid(
                "Factor Evaluation Lenses and provenance are incompatible with the Scope".into(),
            ));
        }
        if draft.horizon_bars.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(ContractError::Invalid(
                "Factor Evaluation horizons must be unique and ascending".into(),
            ));
        }
        if draft.lenses.iter().collect::<BTreeSet<_>>().len() != draft.lenses.len() {
            return Err(ContractError::Invalid(
                "Factor Evaluation Lenses must be unique".into(),
            ));
        }
        let mut fold_ids = BTreeSet::new();
        for window in &draft.windows {
            window.validate()?;
            if !fold_ids.insert(window.fold_id.as_str()) {
                return Err(ContractError::Invalid(
                    "Factor Evaluation fold identities must be unique".into(),
                ));
            }
        }
        if draft.windows.windows(2).any(|pair| {
            pair[0].evaluation.start_time_ms >= pair[1].evaluation.start_time_ms
                || pair[0].evaluation.end_time_ms > pair[1].evaluation.end_time_ms
        }) {
            return Err(ContractError::Invalid(
                "Factor Evaluation folds must be chronological holdout or walk-forward windows"
                    .into(),
            ));
        }
        let mut protocol = Self {
            schema_version: FACTOR_RESEARCH_SCHEMA_VERSION.into(),
            protocol_id: draft.protocol_id,
            user_id: draft.user_id,
            factor_dataset_id: draft.factor_dataset_id,
            feature_dataset_id: draft.feature_dataset_id,
            feature_plan_hash: draft.feature_plan_hash,
            market_data_snapshot_id: draft.market_data_snapshot_id,
            point_in_time_universe_id: draft.point_in_time_universe_id,
            point_in_time_universe: draft.point_in_time_universe,
            output_name: draft.output_name,
            scope: draft.scope,
            target: draft.target,
            horizon_bars: draft.horizon_bars,
            market_context: draft.market_context,
            engine_identity: draft.engine_identity,
            orientation: draft.orientation,
            windows: draft.windows,
            purge_bars: draft.purge_bars,
            embargo_bars: draft.embargo_bars,
            lenses: draft.lenses,
            nuisance_feature_names: draft.nuisance_feature_names,
            regime: draft.regime,
            economic: draft.economic,
            family_id: draft.family_id,
            trial_id: draft.trial_id,
            seed: draft.seed,
            protocol_hash: String::new(),
        };
        let protocol_hash = content_hash(&protocol.content())?;
        protocol.protocol_hash = protocol_hash;
        Ok(protocol)
    }

    pub fn validate(&self) -> Result<(), ContractError> {
        if self.schema_version != FACTOR_RESEARCH_SCHEMA_VERSION
            || self.protocol_id.is_nil()
            || self.user_id.is_nil()
            || self.family_id.is_nil()
            || self.trial_id.is_nil()
            || self.factor_dataset_id.is_empty()
            || self.feature_dataset_id.is_empty()
            || !is_sha256(&self.feature_plan_hash)
            || self.market_data_snapshot_id.is_empty()
            || self.point_in_time_universe_id.is_empty()
            || self.point_in_time_universe.is_empty()
            || !is_lower_kebab(&self.output_name)
            || self.horizon_bars.is_empty()
            || self.horizon_bars.len() > crate::MAX_FACTOR_EVALUATION_HORIZONS
            || self.horizon_bars.iter().any(|horizon| *horizon == 0)
            || self.windows.is_empty()
            || self.windows.len() > crate::MAX_FACTOR_EVALUATION_FOLDS
            || !is_sha256(&self.protocol_hash)
            || self.protocol_hash != content_hash(&self.content())?
        {
            return Err(ContractError::Invalid(
                "Factor Evaluation Protocol identity or content hash is invalid".into(),
            ));
        }
        self.market_context.validate()?;
        self.engine_identity.validate()?;
        let required = FactorLens::required(self.scope);
        if self.lenses.is_empty() || required.iter().any(|lens| !self.lenses.contains(lens)) {
            return Err(ContractError::Invalid(
                "Factor Evaluation Protocol is missing a scope-compatible and Economic Lens".into(),
            ));
        }
        if self.market_context.point_in_time_universe_id != self.point_in_time_universe_id
            || (self.scope == FactorScope::TimeSeries
                && self.lenses.iter().any(|lens| {
                    matches!(lens, FactorLens::CrossSectional | FactorLens::Neutralized)
                }))
            || (self.scope == FactorScope::CrossSectional
                && self.lenses.contains(&FactorLens::Temporal))
            || (self.lenses.contains(&FactorLens::Neutralized)
                && self.nuisance_feature_names.is_empty())
            || (self.lenses.contains(&FactorLens::Regime) && self.regime.is_none())
            || (!self.lenses.contains(&FactorLens::Regime) && self.regime.is_some())
            || self.horizon_bars.windows(2).any(|pair| pair[0] >= pair[1])
            || self.lenses.len() > crate::MAX_FACTOR_EVALUATION_LENSES
            || self.nuisance_feature_names.len() > crate::MAX_FACTOR_NUISANCE_FEATURES
        {
            return Err(ContractError::Invalid(
                "Factor Evaluation Lenses and provenance are incompatible with the Scope".into(),
            ));
        }
        if self.lenses.iter().collect::<BTreeSet<_>>().len() != self.lenses.len() {
            return Err(ContractError::Invalid(
                "Factor Evaluation Lenses must be unique".into(),
            ));
        }
        let mut fold_ids = BTreeSet::new();
        validate_universe(&self.point_in_time_universe)?;
        validate_feature_names(&self.nuisance_feature_names)?;
        if let Some(regime) = &self.regime {
            regime.validate()?;
        }
        self.economic.validate()?;
        for window in &self.windows {
            window.validate()?;
            if !fold_ids.insert(window.fold_id.as_str()) {
                return Err(ContractError::Invalid(
                    "Factor Evaluation fold identities must be unique".into(),
                ));
            }
        }
        if self.windows.windows(2).any(|pair| {
            pair[0].evaluation.start_time_ms >= pair[1].evaluation.start_time_ms
                || pair[0].evaluation.end_time_ms > pair[1].evaluation.end_time_ms
        }) {
            return Err(ContractError::Invalid(
                "Factor Evaluation folds must be chronological holdout or walk-forward windows"
                    .into(),
            ));
        }
        Ok(())
    }

    pub fn evidence_state(&self) -> EvaluationEvidenceState {
        let mut state = EvaluationEvidenceState::OutOfSample;
        for window in &self.windows {
            let window_state = window
                .validate()
                .unwrap_or(EvaluationEvidenceState::Unknown);
            state = match (state, window_state) {
                (EvaluationEvidenceState::Overlapping, _)
                | (_, EvaluationEvidenceState::Overlapping) => EvaluationEvidenceState::Overlapping,
                (EvaluationEvidenceState::Unknown, _) | (_, EvaluationEvidenceState::Unknown) => {
                    EvaluationEvidenceState::Unknown
                }
                _ => EvaluationEvidenceState::OutOfSample,
            };
        }
        state
    }

    fn content(&self) -> impl Serialize + '_ {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Content<'a> {
            schema_version: &'a str,
            protocol_id: Uuid,
            user_id: Uuid,
            factor_dataset_id: &'a str,
            feature_dataset_id: &'a str,
            feature_plan_hash: &'a str,
            market_data_snapshot_id: &'a str,
            point_in_time_universe_id: &'a str,
            point_in_time_universe: &'a [String],
            output_name: &'a str,
            scope: FactorScope,
            target: FactorTarget,
            horizon_bars: &'a [u32],
            market_context: &'a FactorMarketContext,
            engine_identity: &'a ResearchEngineProvenance,
            orientation: FactorOrientation,
            windows: &'a [EvaluationWindow],
            purge_bars: u32,
            embargo_bars: u32,
            lenses: &'a [FactorLens],
            nuisance_feature_names: &'a [String],
            regime: &'a Option<FactorRegimeDefinition>,
            economic: &'a EconomicAssumptions,
            family_id: Uuid,
            trial_id: Uuid,
            seed: u64,
        }
        Content {
            schema_version: &self.schema_version,
            protocol_id: self.protocol_id,
            user_id: self.user_id,
            factor_dataset_id: &self.factor_dataset_id,
            feature_dataset_id: &self.feature_dataset_id,
            feature_plan_hash: &self.feature_plan_hash,
            market_data_snapshot_id: &self.market_data_snapshot_id,
            point_in_time_universe_id: &self.point_in_time_universe_id,
            point_in_time_universe: &self.point_in_time_universe,
            output_name: &self.output_name,
            scope: self.scope,
            target: self.target,
            horizon_bars: &self.horizon_bars,
            market_context: &self.market_context,
            engine_identity: &self.engine_identity,
            orientation: self.orientation,
            windows: &self.windows,
            purge_bars: self.purge_bars,
            embargo_bars: self.embargo_bars,
            lenses: &self.lenses,
            nuisance_feature_names: &self.nuisance_feature_names,
            regime: &self.regime,
            economic: &self.economic,
            family_id: self.family_id,
            trial_id: self.trial_id,
            seed: self.seed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactorEvaluationReport {
    pub schema_version: String,
    pub report_id: Uuid,
    pub protocol_hash: String,
    pub factor_dataset_id: String,
    pub output_name: String,
    pub scope: FactorScope,
    pub target: FactorTarget,
    pub market_data_snapshot_id: String,
    pub point_in_time_universe_id: String,
    pub market_context: FactorMarketContext,
    pub evidence_state: EvaluationEvidenceState,
    pub metrics: Vec<MetricRecord>,
    pub target_unavailable: Vec<TargetUnavailableEvidence>,
    pub regime_evidence: Vec<RegimeEvidence>,
    pub input_identities: Vec<String>,
    pub report_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TargetUnavailableReason {
    BarGap,
    CorporateActionUnavailable,
    InsufficientCoverage,
    MissingClose,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TargetUnavailableEvidence {
    pub instrument_id: String,
    pub observation_time_ms: i64,
    pub horizon_bars: u32,
    pub reason: TargetUnavailableReason,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegimeEvidence {
    pub fold_id: String,
    pub horizon_bars: u32,
    pub feature_name: String,
    pub bucket_count: u8,
    pub thresholds: Vec<f64>,
    pub bucket_metrics: Vec<MetricObservation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MetricRecord {
    pub fold_id: String,
    pub variant: String,
    pub horizon_bars: u32,
    pub output_name: String,
    pub lens: FactorLens,
    pub metric: MetricId,
    pub observation: MetricObservation,
}

impl FactorEvaluationReport {
    pub fn freeze(mut report: Self) -> Result<Self, ContractError> {
        report.report_hash.clear();
        let report_hash = {
            let content = report.content();
            content_hash(&content)?
        };
        report.report_hash = report_hash;
        report.validate()?;
        Ok(report)
    }

    pub fn validate(&self) -> Result<(), ContractError> {
        if self.schema_version != FACTOR_RESEARCH_SCHEMA_VERSION
            || self.report_id.is_nil()
            || !is_sha256(&self.protocol_hash)
            || self.factor_dataset_id.is_empty()
            || !is_lower_kebab(&self.output_name)
            || self.market_data_snapshot_id.is_empty()
            || self.point_in_time_universe_id.is_empty()
            || self.market_context.point_in_time_universe_id != self.point_in_time_universe_id
            || !is_sha256(&self.report_hash)
            || self.input_identities.is_empty()
            || self
                .input_identities
                .iter()
                .any(|identity| identity.is_empty())
            || self.report_hash != content_hash(&self.content())?
        {
            return Err(ContractError::Invalid(
                "Factor Evaluation Report identity is invalid".into(),
            ));
        }
        self.market_context.validate()?;
        let catalog = FactorMetricCatalog::initial();
        catalog.validate()?;
        if let Some(metric) = self.metrics.iter().find(|metric| {
            !is_lower_kebab(&metric.fold_id)
                || metric.variant.trim().is_empty()
                || metric.horizon_bars == 0
                || metric.output_name != self.output_name
                || !lens_matches_scope(self.scope, metric.lens)
                || !is_metric_observation_allowed(&catalog, metric.metric, &metric.observation)
        }) {
            return Err(ContractError::Invalid(format!(
                "Factor Evaluation metric is invalid: {:?} {:?} {:?}",
                metric.metric, metric.lens, metric.observation
            )));
        }
        if self
            .target_unavailable
            .iter()
            .any(|evidence| evidence.instrument_id.trim().is_empty() || evidence.horizon_bars == 0)
        {
            return Err(ContractError::Invalid(
                "Factor Evaluation target evidence is invalid".into(),
            ));
        }
        if self.regime_evidence.iter().any(|evidence| {
            !is_lower_kebab(&evidence.fold_id)
                || evidence.horizon_bars == 0
                || !is_lower_kebab(&evidence.feature_name)
                || evidence
                    .thresholds
                    .iter()
                    .any(|threshold| !threshold.is_finite())
                || evidence.thresholds.windows(2).any(|pair| pair[0] > pair[1])
                || !(2..=5).contains(&evidence.bucket_count)
                || evidence.thresholds.len() > evidence.bucket_count as usize - 1
                || evidence.bucket_metrics.len() != evidence.bucket_count as usize
                || evidence
                    .bucket_metrics
                    .iter()
                    .any(|metric| metric.validate().is_err())
        }) {
            return Err(ContractError::Invalid(
                "Factor Evaluation regime evidence is invalid".into(),
            ));
        }
        Ok(())
    }

    fn content(&self) -> impl Serialize + '_ {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Content<'a> {
            schema_version: &'a str,
            report_id: Uuid,
            protocol_hash: &'a str,
            factor_dataset_id: &'a str,
            output_name: &'a str,
            scope: FactorScope,
            target: FactorTarget,
            market_data_snapshot_id: &'a str,
            point_in_time_universe_id: &'a str,
            market_context: &'a FactorMarketContext,
            evidence_state: EvaluationEvidenceState,
            metrics: &'a [MetricRecord],
            target_unavailable: &'a [TargetUnavailableEvidence],
            regime_evidence: &'a [RegimeEvidence],
            input_identities: &'a [String],
        }
        Content {
            schema_version: &self.schema_version,
            report_id: self.report_id,
            protocol_hash: &self.protocol_hash,
            factor_dataset_id: &self.factor_dataset_id,
            output_name: &self.output_name,
            scope: self.scope,
            target: self.target,
            market_data_snapshot_id: &self.market_data_snapshot_id,
            point_in_time_universe_id: &self.point_in_time_universe_id,
            market_context: &self.market_context,
            evidence_state: self.evidence_state,
            metrics: &self.metrics,
            target_unavailable: &self.target_unavailable,
            regime_evidence: &self.regime_evidence,
            input_identities: &self.input_identities,
        }
    }
}

fn lens_matches_scope(scope: FactorScope, lens: FactorLens) -> bool {
    match scope {
        FactorScope::TimeSeries => matches!(
            lens,
            FactorLens::Temporal | FactorLens::Economic | FactorLens::Regime
        ),
        FactorScope::CrossSectional => matches!(
            lens,
            FactorLens::CrossSectional
                | FactorLens::Economic
                | FactorLens::Neutralized
                | FactorLens::Regime
        ),
    }
}

fn is_metric_observation_allowed(
    catalog: &FactorMetricCatalog,
    metric: MetricId,
    observation: &MetricObservation,
) -> bool {
    let Some(definition) = catalog.metric(metric) else {
        return false;
    };
    if observation.validate().is_err() {
        return false;
    }
    match observation {
        MetricObservation::Available { value, .. } => {
            definition.range.as_ref().is_none_or(|range| {
                range.minimum.is_none_or(|minimum| *value >= minimum)
                    && range.maximum.is_none_or(|maximum| *value <= maximum)
            })
        }
        MetricObservation::Unavailable { reason, .. } => {
            definition.undefined_reasons.contains(reason)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResearchTrialStatus {
    Registered,
    Completed,
    Failed,
    Cancelled,
    Rejected,
    Superseded,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResearchFamily {
    pub schema_version: String,
    pub family_id: Uuid,
    pub user_id: Uuid,
    pub root_candidate_hash: String,
    pub parent_family_id: Option<Uuid>,
    pub registered_trial_ids: Vec<Uuid>,
    pub lineage_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GridSearch {
    pub parameter_cardinalities: Vec<u64>,
    pub trial_count: u64,
}

impl GridSearch {
    pub fn new(parameter_cardinalities: Vec<u64>) -> Result<Self, ContractError> {
        let trial_count = checked_product(parameter_cardinalities.iter().copied())?;
        let grid = Self {
            parameter_cardinalities,
            trial_count,
        };
        grid.validate()?;
        Ok(grid)
    }

    pub fn validate(&self) -> Result<(), ContractError> {
        if self.parameter_cardinalities.is_empty()
            || self
                .parameter_cardinalities
                .iter()
                .any(|cardinality| *cardinality == 0)
            || self.trial_count == 0
            || self.trial_count > MAX_GRID_SEARCH_TRIALS
            || checked_product(self.parameter_cardinalities.iter().copied())? != self.trial_count
        {
            return Err(ContractError::LimitExceeded {
                name: "Grid Search Trials",
                limit: MAX_GRID_SEARCH_TRIALS as usize,
            });
        }
        Ok(())
    }
}

impl ResearchFamily {
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.schema_version != FACTOR_RESEARCH_SCHEMA_VERSION
            || self.family_id.is_nil()
            || self.user_id.is_nil()
            || self.parent_family_id.is_some_and(|id| id.is_nil())
            || !is_sha256(&self.root_candidate_hash)
            || !is_sha256(&self.lineage_hash)
            || self.registered_trial_ids.iter().any(Uuid::is_nil)
            || self
                .registered_trial_ids
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            || self.lineage_hash != content_hash(&self.content())?
        {
            return Err(ContractError::Invalid(
                "Research Family identity or lineage is invalid".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn content(&self) -> impl Serialize + '_ {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Content<'a> {
            schema_version: &'a str,
            family_id: Uuid,
            user_id: Uuid,
            root_candidate_hash: &'a str,
            parent_family_id: Option<Uuid>,
            registered_trial_ids: &'a [Uuid],
        }
        Content {
            schema_version: &self.schema_version,
            family_id: self.family_id,
            user_id: self.user_id,
            root_candidate_hash: &self.root_candidate_hash,
            parent_family_id: self.parent_family_id,
            registered_trial_ids: &self.registered_trial_ids,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResearchTrial {
    pub trial_id: Uuid,
    pub family_id: Uuid,
    pub candidate_hash: String,
    pub protocol_hash: String,
    pub status: ResearchTrialStatus,
    pub report_hash: Option<String>,
    pub raw_statistic: Option<MetricObservation>,
    pub p_value: Option<MetricObservation>,
    pub holm_adjusted: Option<MetricObservation>,
    pub related_trial_ids: Vec<Uuid>,
    pub diagnostic: Option<String>,
}

impl ResearchTrial {
    pub fn is_significant_at(&self, maximum_p_value: f64) -> bool {
        if !maximum_p_value.is_finite() || !(0.0..=1.0).contains(&maximum_p_value) {
            return false;
        }
        self.holm_adjusted
            .as_ref()
            .and_then(MetricObservation::value)
            .is_some_and(|value| value <= maximum_p_value)
    }

    pub fn validate(&self) -> Result<(), ContractError> {
        if self.trial_id.is_nil()
            || self.family_id.is_nil()
            || !is_sha256(&self.candidate_hash)
            || !is_sha256(&self.protocol_hash)
            || self.related_trial_ids.iter().any(Uuid::is_nil)
            || self
                .report_hash
                .as_deref()
                .is_some_and(|hash| !is_sha256(hash))
            || self
                .raw_statistic
                .as_ref()
                .is_some_and(|observation| observation.validate().is_err())
            || self
                .p_value
                .as_ref()
                .is_some_and(|observation| probability_observation_is_invalid(observation))
            || (self.p_value.is_some() && self.raw_statistic.is_none())
            || self
                .holm_adjusted
                .as_ref()
                .is_some_and(|observation| probability_observation_is_invalid(observation))
        {
            return Err(ContractError::Invalid(
                "Research Trial identity is invalid".into(),
            ));
        }
        Ok(())
    }
}

fn probability_observation_is_invalid(observation: &MetricObservation) -> bool {
    observation.validate().is_err()
        || observation
            .value()
            .is_some_and(|value| !(0.0..=1.0).contains(&value))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PromotionPolicy {
    pub schema_version: String,
    pub policy_id: Uuid,
    pub revision: u64,
    pub required_lenses: Vec<FactorLens>,
    pub minimum_coverage: f64,
    pub minimum_samples: u64,
    pub maximum_holm_p_value: f64,
    pub require_subperiod_sign_consistency: bool,
    pub require_cost_aware_economic: bool,
    pub policy_hash: String,
}

impl PromotionPolicy {
    pub fn validate(&self) -> Result<(), ContractError> {
        self.validate_requirements()?;
        if !is_sha256(&self.policy_hash) || self.policy_hash != content_hash(&self.content())? {
            return Err(ContractError::Invalid(
                "Factor Promotion Policy is invalid".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn validate_requirements(&self) -> Result<(), ContractError> {
        if self.schema_version != FACTOR_RESEARCH_SCHEMA_VERSION
            || self.policy_id.is_nil()
            || self.revision == 0
            || self.required_lenses.is_empty()
            || self
                .required_lenses
                .iter()
                .enumerate()
                .any(|(index, lens)| self.required_lenses[..index].contains(lens))
            || !self.minimum_coverage.is_finite()
            || !(0.0..=1.0).contains(&self.minimum_coverage)
            || self.minimum_samples == 0
            || !self.maximum_holm_p_value.is_finite()
            || !(0.0..=1.0).contains(&self.maximum_holm_p_value)
        {
            return Err(ContractError::Invalid(
                "Factor Promotion Policy requirements are invalid".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn content(&self) -> impl Serialize + '_ {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Content<'a> {
            schema_version: &'a str,
            policy_id: Uuid,
            revision: u64,
            required_lenses: &'a [FactorLens],
            minimum_coverage: f64,
            minimum_samples: u64,
            maximum_holm_p_value: f64,
            require_subperiod_sign_consistency: bool,
            require_cost_aware_economic: bool,
        }
        Content {
            schema_version: &self.schema_version,
            policy_id: self.policy_id,
            revision: self.revision,
            required_lenses: &self.required_lenses,
            minimum_coverage: self.minimum_coverage,
            minimum_samples: self.minimum_samples,
            maximum_holm_p_value: self.maximum_holm_p_value,
            require_subperiod_sign_consistency: self.require_subperiod_sign_consistency,
            require_cost_aware_economic: self.require_cost_aware_economic,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PromotionDecisionState {
    Rejected,
    ResearchValidated,
    ComponentEligible,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactorPromotionDecision {
    pub schema_version: String,
    pub decision_id: Uuid,
    pub user_id: Uuid,
    pub candidate_hash: String,
    pub output_name: String,
    pub state: PromotionDecisionState,
    pub report_hashes: Vec<String>,
    pub policy_hash: String,
    pub evidence_state: EvaluationEvidenceState,
    pub supersedes: Option<Uuid>,
    pub decision_hash: String,
}

impl FactorPromotionDecision {
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.schema_version != FACTOR_RESEARCH_SCHEMA_VERSION
            || self.decision_id.is_nil()
            || self.user_id.is_nil()
            || !is_sha256(&self.candidate_hash)
            || !is_lower_kebab(&self.output_name)
            || self.report_hashes.is_empty()
            || self.report_hashes.iter().any(|hash| !is_sha256(hash))
            || self.report_hashes.windows(2).any(|pair| pair[0] >= pair[1])
            || !is_sha256(&self.policy_hash)
            || !is_sha256(&self.decision_hash)
            || self.decision_hash != content_hash(&self.content())?
            || self.supersedes.is_some_and(|id| id.is_nil())
        {
            return Err(ContractError::Invalid(
                "Factor Promotion Decision is invalid".into(),
            ));
        }
        if matches!(
            self.state,
            PromotionDecisionState::ResearchValidated | PromotionDecisionState::ComponentEligible
        ) && self.evidence_state != EvaluationEvidenceState::OutOfSample
        {
            return Err(ContractError::Invalid(
                "positive Factor Promotion Decisions require Out-of-sample evidence".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn content(&self) -> impl Serialize + '_ {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Content<'a> {
            schema_version: &'a str,
            decision_id: Uuid,
            user_id: Uuid,
            candidate_hash: &'a str,
            output_name: &'a str,
            state: PromotionDecisionState,
            report_hashes: &'a [String],
            policy_hash: &'a str,
            evidence_state: EvaluationEvidenceState,
            supersedes: Option<Uuid>,
        }
        Content {
            schema_version: &self.schema_version,
            decision_id: self.decision_id,
            user_id: self.user_id,
            candidate_hash: &self.candidate_hash,
            output_name: &self.output_name,
            state: self.state,
            report_hashes: &self.report_hashes,
            policy_hash: &self.policy_hash,
            evidence_state: self.evidence_state,
            supersedes: self.supersedes,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PromotedFactorLibraryEntry {
    pub candidate_hash: String,
    pub output_name: String,
    pub decision_id: Uuid,
    pub report_hashes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PromotedFactorLibrary {
    pub schema_version: String,
    pub user_id: Uuid,
    pub entries: Vec<PromotedFactorLibraryEntry>,
    pub library_hash: String,
}

impl PromotedFactorLibrary {
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.schema_version != FACTOR_RESEARCH_SCHEMA_VERSION
            || self.user_id.is_nil()
            || !is_sha256(&self.library_hash)
            || self.library_hash != content_hash(&self.content())?
            || self.entries.iter().any(|entry| {
                !is_sha256(&entry.candidate_hash)
                    || !is_lower_kebab(&entry.output_name)
                    || entry.decision_id.is_nil()
                    || entry.report_hashes.is_empty()
                    || entry.report_hashes.iter().any(|hash| !is_sha256(hash))
            })
            || self.entries.windows(2).any(|pair| {
                (
                    pair[0].candidate_hash.as_str(),
                    pair[0].output_name.as_str(),
                ) >= (
                    pair[1].candidate_hash.as_str(),
                    pair[1].output_name.as_str(),
                )
            })
        {
            return Err(ContractError::Invalid(
                "Promoted Factor Library is invalid".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn content(&self) -> impl Serialize + '_ {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Content<'a> {
            schema_version: &'a str,
            user_id: Uuid,
            entries: &'a [PromotedFactorLibraryEntry],
        }
        Content {
            schema_version: &self.schema_version,
            user_id: self.user_id,
            entries: &self.entries,
        }
    }
}

pub use crate::catalog::{MetricId, MetricObservation};

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> ResearchEngineProvenance {
        ResearchEngineProvenance {
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

    fn context() -> FactorMarketContext {
        FactorMarketContext {
            venue: "OKX".into(),
            asset_class: "crypto".into(),
            bar_interval: "1h".into(),
            price_basis: "unadjusted".into(),
            valuation_currency: "USDT".into(),
            point_in_time_universe_id: "universe".into(),
        }
    }

    #[test]
    fn candidate_freezes_semantic_identity_and_rejects_v1_custom_builds() {
        let candidate = FactorCandidate::freeze(FactorCandidateDraft {
            candidate_id: Uuid::new_v4(),
            revision: 1,
            scope: FactorScope::TimeSeries,
            feature_slots: vec![FactorFeatureSlot {
                name: "close".into(),
            }],
            parameters: vec![],
            outputs: vec![FactorOutput {
                name: "momentum".into(),
            }],
            source: FactorCandidateSource::Declarative {
                definition: DeclarativeFactorDefinition {
                    feature_plan_hash: "a".repeat(64),
                    operator_catalog_version: adaq_feature_engine::FEATURE_OPERATOR_CATALOG_VERSION
                        .into(),
                    outputs: vec![DeclarativeFactorOutputBinding {
                        output_name: "momentum".into(),
                        feature_slot: "close".into(),
                    }],
                },
            },
        })
        .unwrap();
        candidate.validate().unwrap();
        let mut bytes = candidate.to_json().unwrap();
        let last_content_index = bytes.len() - 2;
        bytes[last_content_index] = b'0';
        assert!(FactorCandidate::load(&bytes).is_err());

        let source = FactorCandidateSource::Custom {
            build: CandidateBuildProvenance {
                attempt_id: Uuid::new_v4(),
                source_sha256: "a".repeat(64),
                sdk_version: "0.1.0".into(),
                abi_version: "1.0.0".into(),
                toolchain: "stable".into(),
                compiler: "rustc 1.0.0".into(),
                target: "wasm".into(),
                commands: vec!["cargo component build".into()],
                environment: BTreeMap::new(),
                resource_policy: FactorResourcePolicy {
                    fuel_per_call: 1,
                    memory_bytes: 1,
                },
                diagnostic_log_sha256: None,
                package_sha256: "b".repeat(64),
            },
        };
        assert!(
            FactorCandidate::freeze(FactorCandidateDraft {
                candidate_id: Uuid::new_v4(),
                revision: 1,
                scope: FactorScope::TimeSeries,
                feature_slots: vec![],
                parameters: vec![],
                outputs: vec![FactorOutput {
                    name: "value".into()
                }],
                source,
            })
            .is_err()
        );
    }

    #[test]
    fn materialization_attempts_retain_retry_lineage() {
        let mut attempt =
            FactorMaterializationAttempt::new(Uuid::new_v4(), Uuid::new_v4(), "a".repeat(64))
                .unwrap();
        attempt.transition(AttemptStatus::Running).unwrap();
        attempt.transition(AttemptStatus::Failed).unwrap();
        let retry = attempt.retry(Uuid::new_v4()).unwrap();
        assert_eq!(retry.source_attempt_id, Some(attempt.attempt_id));
        assert_eq!(retry.status, AttemptStatus::Pending);
    }

    #[test]
    fn evaluation_windows_never_call_incomplete_provenance_out_of_sample() {
        let window = EvaluationWindow {
            fold_id: "fold-1".into(),
            selection: ObservationRange {
                start_time_ms: 0,
                end_time_ms: 10,
            },
            evaluation: ObservationRange {
                start_time_ms: 20,
                end_time_ms: 30,
            },
            training: None,
            fitting: Some(ObservationRange {
                start_time_ms: 0,
                end_time_ms: 10,
            }),
            normalization: Some(ObservationRange {
                start_time_ms: 0,
                end_time_ms: 10,
            }),
            target_construction: Some(ObservationRange {
                start_time_ms: 0,
                end_time_ms: 10,
            }),
        };
        assert_eq!(window.validate().unwrap(), EvaluationEvidenceState::Unknown);
    }

    #[test]
    fn positive_promotion_requires_out_of_sample_evidence() {
        let decision = FactorPromotionDecision {
            schema_version: FACTOR_RESEARCH_SCHEMA_VERSION.into(),
            decision_id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            candidate_hash: "a".repeat(64),
            output_name: "value".into(),
            state: PromotionDecisionState::ResearchValidated,
            report_hashes: vec!["b".repeat(64)],
            policy_hash: "c".repeat(64),
            evidence_state: EvaluationEvidenceState::Unknown,
            supersedes: None,
            decision_hash: "d".repeat(64),
        };
        assert!(decision.validate().is_err());
    }

    #[test]
    fn protocol_content_identity_is_stable() {
        let protocol = FactorMaterializationProtocol::freeze(FactorMaterializationProtocolDraft {
            protocol_id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            candidate_hash: "a".repeat(64),
            feature_dataset_id: "dataset".into(),
            feature_plan_hash: "b".repeat(64),
            parameters: vec![FactorParameterValue::Integer(5)],
            market_data_snapshot_id: "snapshot".into(),
            point_in_time_universe_id: "universe".into(),
            observation_range: ObservationRange {
                start_time_ms: 0,
                end_time_ms: 1,
            },
            market_context: context(),
            engine_identity: engine(),
            seed: 1,
        })
        .unwrap();
        protocol.validate().unwrap();
        assert!(is_sha256(&protocol.protocol_hash));
    }

    #[test]
    fn grid_search_is_checked_before_exceeding_trial_limit() {
        assert_eq!(GridSearch::new(vec![4, 8]).unwrap().trial_count, 32);
        assert!(GridSearch::new(vec![16, 17]).is_err());
        assert!(GridSearch::new(vec![u64::MAX, 2]).is_err());
    }
}
