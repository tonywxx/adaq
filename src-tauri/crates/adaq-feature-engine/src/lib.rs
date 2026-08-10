//! Tauri-independent contracts for ADAQ Feature Definitions and Feature Plans.

mod execution;

pub use execution::*;

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fmt,
    str::FromStr,
};

use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const FEATURE_DEFINITION_SCHEMA_VERSION: &str = "1.0.0";
pub const FEATURE_PLAN_SCHEMA_VERSION: &str = "2.0.0";
pub const FEATURE_ENGINE_VERSION: &str = "adaq-feature-engine@1.0.0";
pub const FEATURE_OPERATOR_CATALOG_VERSION: &str = "adaq-feature-operator-catalog@1.0.0";
pub const FEATURE_UNAVAILABILITY_REASON_VERSION: &str = "adaq-feature-unavailability@1.0.0";
pub const MAX_CANONICAL_JSON_BYTES: usize = 1024 * 1024;
pub const MAX_DEFINITION_NODES: usize = 256;
pub const MAX_FEATURE_OUTPUTS: usize = 64;
pub const MAX_DEPENDENCY_DEPTH: usize = 64;
pub const MAX_EFFECTIVE_WARMUP_BARS: u32 = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FeatureScope {
    Pointwise,
    #[serde(rename = "time-series")]
    TimeSeries,
    #[serde(rename = "cross-sectional")]
    CrossSectional,
}

impl FeatureScope {
    fn rank(self) -> u8 {
        match self {
            Self::Pointwise => 0,
            Self::TimeSeries => 1,
            Self::CrossSectional => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MarketField {
    Open,
    High,
    Low,
    Close,
    BaseVolume,
    QuoteVolume,
}

impl MarketField {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::High => "high",
            Self::Low => "low",
            Self::Close => "close",
            Self::BaseVolume => "base-volume",
            Self::QuoteVolume => "quote-volume",
        }
    }
}

impl FromStr for MarketField {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(match value {
            "open" => Self::Open,
            "high" => Self::High,
            "low" => Self::Low,
            "close" => Self::Close,
            "base-volume" => Self::BaseVolume,
            "quote-volume" => Self::QuoteVolume,
            _ => return Err(()),
        })
    }
}

impl From<&str> for MarketField {
    fn from(value: &str) -> Self {
        value.parse().expect("unknown market field")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum FeatureOperator {
    CheckedArithmetic,
    Indicator { id: String },
    BackwardSimpleReturn,
    BackwardLogReturn,
    RollingMean,
    RollingPopulationStandardDeviation,
    RollingMinimum,
    RollingMaximum,
    RealizedVolatility,
    QuoteVolume,
    RollingQuoteVolume,
    ZeroVolume,
    AmihudIlliquidity,
    TradingDayOfWeek,
    TradingMonth,
    MinutesFromSessionOpen,
    MinutesToSessionClose,
    SessionProgress,
    OneHot,
    Sine,
    Cosine,
    CrossSectionalRank,
    CrossSectionalPercentile,
    CrossSectionalZScore,
    CausalSplitAdjustment,
    DividendTotalReturn,
    Standardization,
    Winsorization,
}

impl FeatureOperator {
    fn catalog_id(&self) -> &'static str {
        match self {
            Self::CheckedArithmetic => "checked-arithmetic",
            Self::Indicator { .. } => "indicator",
            Self::BackwardSimpleReturn => "backward-simple-return",
            Self::BackwardLogReturn => "backward-log-return",
            Self::RollingMean => "rolling-mean",
            Self::RollingPopulationStandardDeviation => "rolling-population-standard-deviation",
            Self::RollingMinimum => "rolling-minimum",
            Self::RollingMaximum => "rolling-maximum",
            Self::RealizedVolatility => "realized-volatility",
            Self::QuoteVolume => "quote-volume",
            Self::RollingQuoteVolume => "rolling-quote-volume",
            Self::ZeroVolume => "zero-volume",
            Self::AmihudIlliquidity => "amihud-illiquidity",
            Self::TradingDayOfWeek => "trading-day-of-week",
            Self::TradingMonth => "trading-month",
            Self::MinutesFromSessionOpen => "minutes-from-session-open",
            Self::MinutesToSessionClose => "minutes-to-session-close",
            Self::SessionProgress => "session-progress",
            Self::OneHot => "one-hot",
            Self::Sine => "sine",
            Self::Cosine => "cosine",
            Self::CrossSectionalRank => "cross-sectional-rank",
            Self::CrossSectionalPercentile => "cross-sectional-percentile",
            Self::CrossSectionalZScore => "cross-sectional-z-score",
            Self::CausalSplitAdjustment => "causal-split-adjustment",
            Self::DividendTotalReturn => "dividend-total-return",
            Self::Standardization => "standardization",
            Self::Winsorization => "winsorization",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FeatureOperatorCatalog {
    pub version: String,
    pub operators: Vec<String>,
}

impl FeatureOperatorCatalog {
    pub fn initial() -> Self {
        Self {
            version: FEATURE_OPERATOR_CATALOG_VERSION.into(),
            operators: vec![
                "checked-arithmetic",
                "indicator",
                "backward-simple-return",
                "backward-log-return",
                "rolling-mean",
                "rolling-population-standard-deviation",
                "rolling-minimum",
                "rolling-maximum",
                "realized-volatility",
                "quote-volume",
                "rolling-quote-volume",
                "zero-volume",
                "amihud-illiquidity",
                "trading-day-of-week",
                "trading-month",
                "minutes-from-session-open",
                "minutes-to-session-close",
                "session-progress",
                "one-hot",
                "sine",
                "cosine",
                "cross-sectional-rank",
                "cross-sectional-percentile",
                "cross-sectional-z-score",
                "causal-split-adjustment",
                "dividend-total-return",
                "standardization",
                "winsorization",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        }
    }

    fn supports(&self, operator: &FeatureOperator) -> bool {
        self.operators.iter().any(|id| id == operator.catalog_id())
    }

    fn is_initial(&self) -> bool {
        self == &Self::initial()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum FeatureInput {
    Market { field: MarketField },
    Node { node_id: String },
    Artifact { artifact_id: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FeatureNode {
    pub id: String,
    pub operator: FeatureOperator,
    pub scope: FeatureScope,
    #[serde(default)]
    pub inputs: Vec<FeatureInput>,
    #[serde(default)]
    pub parameters: BTreeMap<String, Value>,
    #[serde(default)]
    pub warmup_bars: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FeatureOutput {
    pub name: String,
    pub node_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DefinitionDraft {
    pub definition_id: Uuid,
    pub revision: u64,
    pub scope: FeatureScope,
    pub nodes: Vec<FeatureNode>,
    pub outputs: Vec<FeatureOutput>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DefinitionContent {
    definition_schema_version: String,
    definition_id: Uuid,
    revision: u64,
    scope: FeatureScope,
    operator_catalog_version: String,
    nodes: Vec<FeatureNode>,
    outputs: Vec<FeatureOutput>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DefinitionDocument {
    #[serde(flatten)]
    content: DefinitionContent,
    definition_hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeatureDefinition(DefinitionDocument);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationIssue {
    pub code: String,
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinitionValidationError {
    pub issues: Vec<ValidationIssue>,
}

impl DefinitionValidationError {
    pub fn codes(&self) -> Vec<&str> {
        self.issues
            .iter()
            .map(|issue| issue.code.as_str())
            .collect()
    }
}

impl fmt::Display for DefinitionValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Feature Definition validation failed with {} issue(s)",
            self.issues.len()
        )
    }
}

impl std::error::Error for DefinitionValidationError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefinitionLoadError {
    InvalidJson,
    NonCanonical,
    HashMismatch,
    TooLarge,
    InvalidContract(String),
}

impl DefinitionLoadError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidJson => "invalid-definition-json",
            Self::NonCanonical => "non-canonical-definition-json",
            Self::HashMismatch => "definition-hash-mismatch",
            Self::TooLarge => "definition-json-too-large",
            Self::InvalidContract(_) => "invalid-definition-contract",
        }
    }
}

impl fmt::Display for DefinitionLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for DefinitionLoadError {}

impl FeatureDefinition {
    pub fn freeze(draft: DefinitionDraft) -> Result<Self, DefinitionValidationError> {
        let content = DefinitionContent {
            definition_schema_version: FEATURE_DEFINITION_SCHEMA_VERSION.into(),
            definition_id: draft.definition_id,
            revision: draft.revision,
            scope: draft.scope,
            operator_catalog_version: FEATURE_OPERATOR_CATALOG_VERSION.into(),
            nodes: draft.nodes,
            outputs: draft.outputs,
        };
        validate_definition(&content, &FeatureOperatorCatalog::initial())?;
        let definition_hash =
            sha256_hex(
                &canonical_json(&content).map_err(|error| DefinitionValidationError {
                    issues: vec![issue(
                        match error {
                            CanonicalJsonError::TooLarge => "definition-json-too-large",
                            CanonicalJsonError::InvalidNumber => "invalid-definition-number",
                            CanonicalJsonError::Serialization => "invalid-definition-json",
                        },
                        None,
                    )],
                })?,
            );
        let document = DefinitionDocument {
            content,
            definition_hash,
        };
        if canonical_json(&document).is_err() {
            return Err(DefinitionValidationError {
                issues: vec![issue("definition-json-too-large", None)],
            });
        }
        Ok(Self(document))
    }

    pub fn definition_id(&self) -> Uuid {
        self.0.content.definition_id
    }

    pub fn revision(&self) -> u64 {
        self.0.content.revision
    }

    pub fn scope(&self) -> FeatureScope {
        self.0.content.scope
    }

    pub fn definition_hash(&self) -> &str {
        &self.0.definition_hash
    }

    pub fn nodes(&self) -> &[FeatureNode] {
        &self.0.content.nodes
    }

    pub fn outputs(&self) -> &[FeatureOutput] {
        &self.0.content.outputs
    }

    pub fn to_json(&self) -> Vec<u8> {
        canonical_json(&self.0).expect("validated Feature Definition fits the canonical size limit")
    }

    pub fn load(bytes: &[u8]) -> Result<Self, DefinitionLoadError> {
        if bytes.len() > MAX_CANONICAL_JSON_BYTES {
            return Err(DefinitionLoadError::TooLarge);
        }
        let document = serde_json::from_slice::<DefinitionDocument>(bytes)
            .map_err(|_| DefinitionLoadError::InvalidJson)?;
        let canonical = canonical_json(&document).map_err(|error| match error {
            CanonicalJsonError::TooLarge => DefinitionLoadError::TooLarge,
            CanonicalJsonError::InvalidNumber => DefinitionLoadError::InvalidJson,
            CanonicalJsonError::Serialization => DefinitionLoadError::InvalidJson,
        })?;
        if canonical != bytes {
            return Err(DefinitionLoadError::NonCanonical);
        }
        let expected_hash = sha256_hex(
            &canonical_json(&document.content).map_err(|_| DefinitionLoadError::InvalidJson)?,
        );
        if document.definition_hash != expected_hash {
            return Err(DefinitionLoadError::HashMismatch);
        }
        validate_definition(&document.content, &FeatureOperatorCatalog::initial())
            .map_err(|error| DefinitionLoadError::InvalidContract(error.codes().join(",")))?;
        Ok(Self(document))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FeatureEngineIdentity {
    pub feature_engine_version: String,
    pub feature_engine_source_sha256: String,
    pub feature_engine_build_id: String,
    pub operator_catalog_version: String,
    pub indicator_engine_version: String,
    pub indicator_catalog_version: String,
    pub ta_lib_version: String,
    pub ta_source_sha256: String,
    pub wrapper_sha256: String,
    pub target_triple: String,
    pub compiler_and_flags_sha256: String,
    pub engine_build_id: String,
}

impl FeatureEngineIdentity {
    pub fn native() -> Result<Self, adaq_indicator_engine::EngineError> {
        let engine = adaq_indicator_engine::IndicatorEngine::initialize()?;
        Ok(Self::from_indicator(engine.identity()))
    }

    pub fn from_indicator(identity: &adaq_indicator_engine::EngineIdentity) -> Self {
        Self::from_indicator_fields(
            identity.engine_version,
            identity.catalog_version,
            identity.ta_lib_version,
            identity.ta_source_sha256,
            identity.wrapper_sha256,
            identity.target_triple,
            identity.compiler_and_flags_sha256,
            identity.build_id,
        )
    }

    pub fn from_indicator_fields(
        indicator_engine_version: impl Into<String>,
        indicator_catalog_version: impl Into<String>,
        ta_lib_version: impl Into<String>,
        ta_source_sha256: impl Into<String>,
        wrapper_sha256: impl Into<String>,
        target_triple: impl Into<String>,
        compiler_and_flags_sha256: impl Into<String>,
        engine_build_id: impl Into<String>,
    ) -> Self {
        let indicator_engine_version = indicator_engine_version.into();
        let indicator_catalog_version = indicator_catalog_version.into();
        let ta_lib_version = ta_lib_version.into();
        let ta_source_sha256 = ta_source_sha256.into();
        let wrapper_sha256 = wrapper_sha256.into();
        let target_triple = target_triple.into();
        let compiler_and_flags_sha256 = compiler_and_flags_sha256.into();
        let engine_build_id = engine_build_id.into();
        Self {
            feature_engine_version: FEATURE_ENGINE_VERSION.into(),
            feature_engine_source_sha256: env!("ADAQ_FEATURE_ENGINE_SOURCE_SHA256").into(),
            feature_engine_build_id: env!("ADAQ_FEATURE_ENGINE_BUILD_ID").into(),
            operator_catalog_version: FEATURE_OPERATOR_CATALOG_VERSION.into(),
            indicator_engine_version,
            indicator_catalog_version,
            ta_lib_version,
            ta_source_sha256,
            wrapper_sha256,
            target_triple,
            compiler_and_flags_sha256,
            engine_build_id,
        }
    }

    pub fn for_tests() -> Self {
        Self {
            feature_engine_version: FEATURE_ENGINE_VERSION.into(),
            feature_engine_source_sha256: "f".repeat(64),
            feature_engine_build_id: "e".repeat(64),
            operator_catalog_version: FEATURE_OPERATOR_CATALOG_VERSION.into(),
            indicator_engine_version: "adaq-indicator-engine@1.0.0".into(),
            indicator_catalog_version: "adaq-indicator-catalog@1.0.0".into(),
            ta_lib_version: "0.7.1".into(),
            ta_source_sha256: "a".repeat(64),
            wrapper_sha256: "b".repeat(64),
            target_triple: "test-target".into(),
            compiler_and_flags_sha256: "c".repeat(64),
            engine_build_id: "d".repeat(64),
        }
    }

    fn validate(&self) -> bool {
        self.feature_engine_version == FEATURE_ENGINE_VERSION
            && is_sha256(&self.feature_engine_source_sha256)
            && is_sha256(&self.feature_engine_build_id)
            && self.operator_catalog_version == FEATURE_OPERATOR_CATALOG_VERSION
            && !self.indicator_engine_version.is_empty()
            && self.indicator_catalog_version == adaq_indicator_engine::CATALOG_VERSION
            && !self.ta_lib_version.is_empty()
            && is_sha256(&self.ta_source_sha256)
            && is_sha256(&self.wrapper_sha256)
            && !self.target_triple.is_empty()
            && is_sha256(&self.compiler_and_flags_sha256)
            && is_sha256(&self.engine_build_id)
    }
}

impl Default for FeatureEngineIdentity {
    fn default() -> Self {
        Self {
            feature_engine_version: String::new(),
            feature_engine_source_sha256: String::new(),
            feature_engine_build_id: String::new(),
            operator_catalog_version: String::new(),
            indicator_engine_version: String::new(),
            indicator_catalog_version: String::new(),
            ta_lib_version: String::new(),
            ta_source_sha256: String::new(),
            wrapper_sha256: String::new(),
            target_triple: String::new(),
            compiler_and_flags_sha256: String::new(),
            engine_build_id: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "kebab-case")]
pub enum FrozenBuiltInParameter {
    Integer(i32),
    Real(String),
    Enum(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum FeatureSource {
    Market {
        field: MarketField,
    },
    BuiltIn {
        indicator: String,
        output: String,
        real_inputs: Vec<MarketField>,
        parameters: BTreeMap<String, FrozenBuiltInParameter>,
    },
    External {
        dependency_alias: String,
        output: String,
    },
    Signal {
        dataset_id: String,
        signal_name: String,
        snapshot_id: String,
        instrument_id: String,
        venue: String,
        bar_interval: String,
        contract: Value,
        producer_segments: Vec<Value>,
        artifact_provenance: Value,
        evidence_state: String,
        component_lock: Vec<Value>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FeatureSlot {
    pub name: String,
    pub source: FeatureSource,
    pub warmup_bars: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FeatureFactor {
    pub alias: String,
    pub parameters: Vec<Value>,
    pub output_names: Vec<String>,
    pub warmup_bars: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NamedParameter {
    pub name: String,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FittedArtifactBinding {
    pub artifact_id: String,
    pub eligible_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FeaturePlanDraft {
    #[serde(default)]
    pub definitions: Vec<FeatureDefinition>,
    #[serde(default)]
    pub slots: Vec<FeatureSlot>,
    #[serde(default)]
    pub factors: Vec<FeatureFactor>,
    #[serde(default)]
    pub artifacts: Vec<FittedArtifactBinding>,
    #[serde(default)]
    pub consumer_package_sha256: String,
    #[serde(default)]
    pub consumer_parameters: Vec<NamedParameter>,
    #[serde(default)]
    pub consumer_warmup_bars: u32,
    pub engine_identity: FeatureEngineIdentity,
    #[serde(default = "FeatureOperatorCatalog::initial")]
    pub operator_catalog: FeatureOperatorCatalog,
}

impl Default for FeaturePlanDraft {
    fn default() -> Self {
        Self {
            definitions: Vec::new(),
            slots: Vec::new(),
            factors: Vec::new(),
            artifacts: Vec::new(),
            consumer_package_sha256: String::new(),
            consumer_parameters: Vec::new(),
            consumer_warmup_bars: 0,
            engine_identity: FeatureEngineIdentity::default(),
            operator_catalog: FeatureOperatorCatalog::initial(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PlanContent {
    plan_schema_version: String,
    operator_catalog: FeatureOperatorCatalog,
    unavailability_reason_version: String,
    feature_engine_version: String,
    feature_engine_source_sha256: String,
    feature_engine_build_id: String,
    indicator_engine_version: String,
    indicator_catalog_version: String,
    ta_lib_version: String,
    ta_source_sha256: String,
    wrapper_sha256: String,
    target_triple: String,
    compiler_and_flags_sha256: String,
    engine_build_id: String,
    consumer_package_sha256: String,
    consumer_parameters: Vec<NamedParameter>,
    consumer_warmup_bars: u32,
    definitions: Vec<FeatureDefinition>,
    slots: Vec<FeatureSlot>,
    factors: Vec<FeatureFactor>,
    artifacts: Vec<FittedArtifactBinding>,
    effective_warmup_bars: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PlanDocument {
    #[serde(flatten)]
    content: PlanContent,
    plan_hash: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FeaturePlan(PlanDocument);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanValidationError {
    Issues(Vec<ValidationIssue>),
}

impl PlanValidationError {
    pub fn issues(&self) -> &[ValidationIssue] {
        match self {
            Self::Issues(issues) => issues,
        }
    }

    pub fn codes(&self) -> Vec<&str> {
        self.issues()
            .iter()
            .map(|issue| issue.code.as_str())
            .collect()
    }
}

impl fmt::Display for PlanValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Feature Plan validation failed with {} issue(s)",
            self.issues().len()
        )
    }
}

impl std::error::Error for PlanValidationError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanLoadError {
    InvalidJson,
    NonCanonical,
    HashMismatch,
    TooLarge,
    InvalidContract(String),
    UnsupportedEngineIdentity,
    ResetRequired {
        stored_schema_version: Option<String>,
    },
}

impl PlanLoadError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidJson => "invalid-plan-json",
            Self::NonCanonical => "non-canonical-plan-json",
            Self::HashMismatch => "plan-hash-mismatch",
            Self::TooLarge => "plan-json-too-large",
            Self::InvalidContract(_) => "invalid-plan-contract",
            Self::UnsupportedEngineIdentity => "unsupported-engine-identity",
            Self::ResetRequired { .. } => "reset-required",
        }
    }
}

impl fmt::Display for PlanLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for PlanLoadError {}

impl FeaturePlan {
    pub fn freeze(draft: FeaturePlanDraft) -> Result<Self, PlanValidationError> {
        validate_draft(&draft)?;
        let effective_warmup_bars =
            effective_warmup(&draft.definitions, &draft.slots, &draft.factors);
        let identity = draft.engine_identity;
        let content = PlanContent {
            plan_schema_version: FEATURE_PLAN_SCHEMA_VERSION.into(),
            operator_catalog: draft.operator_catalog,
            unavailability_reason_version: FEATURE_UNAVAILABILITY_REASON_VERSION.into(),
            feature_engine_version: identity.feature_engine_version,
            feature_engine_source_sha256: identity.feature_engine_source_sha256,
            feature_engine_build_id: identity.feature_engine_build_id,
            indicator_engine_version: identity.indicator_engine_version,
            indicator_catalog_version: identity.indicator_catalog_version,
            ta_lib_version: identity.ta_lib_version,
            ta_source_sha256: identity.ta_source_sha256,
            wrapper_sha256: identity.wrapper_sha256,
            target_triple: identity.target_triple,
            compiler_and_flags_sha256: identity.compiler_and_flags_sha256,
            engine_build_id: identity.engine_build_id,
            consumer_package_sha256: draft.consumer_package_sha256,
            consumer_parameters: draft.consumer_parameters,
            consumer_warmup_bars: draft.consumer_warmup_bars,
            definitions: draft.definitions,
            slots: draft.slots,
            factors: draft.factors,
            artifacts: draft.artifacts,
            effective_warmup_bars,
        };
        let plan_hash = sha256_hex(&canonical_json(&content).map_err(plan_validation_from_json)?);
        let document = PlanDocument { content, plan_hash };
        canonical_json(&document).map_err(plan_validation_from_json)?;
        Ok(Self(document))
    }

    pub fn plan_hash(&self) -> &str {
        &self.0.plan_hash
    }

    pub fn slot_names(&self) -> impl ExactSizeIterator<Item = &str> {
        self.0.content.slots.iter().map(|slot| slot.name.as_str())
    }

    pub fn slots(&self) -> &[FeatureSlot] {
        &self.0.content.slots
    }

    pub fn factors(&self) -> &[FeatureFactor] {
        &self.0.content.factors
    }

    pub fn definitions(&self) -> &[FeatureDefinition] {
        &self.0.content.definitions
    }

    pub fn artifacts(&self) -> &[FittedArtifactBinding] {
        &self.0.content.artifacts
    }

    pub fn effective_warmup_bars(&self) -> u32 {
        self.0.content.effective_warmup_bars
    }

    pub fn consumer_package_sha256(&self) -> &str {
        &self.0.content.consumer_package_sha256
    }

    pub fn engine_identity(&self) -> FeatureEngineIdentity {
        FeatureEngineIdentity {
            feature_engine_version: self.0.content.feature_engine_version.clone(),
            feature_engine_source_sha256: self.0.content.feature_engine_source_sha256.clone(),
            feature_engine_build_id: self.0.content.feature_engine_build_id.clone(),
            operator_catalog_version: self.0.content.operator_catalog.version.clone(),
            indicator_engine_version: self.0.content.indicator_engine_version.clone(),
            indicator_catalog_version: self.0.content.indicator_catalog_version.clone(),
            ta_lib_version: self.0.content.ta_lib_version.clone(),
            ta_source_sha256: self.0.content.ta_source_sha256.clone(),
            wrapper_sha256: self.0.content.wrapper_sha256.clone(),
            target_triple: self.0.content.target_triple.clone(),
            compiler_and_flags_sha256: self.0.content.compiler_and_flags_sha256.clone(),
            engine_build_id: self.0.content.engine_build_id.clone(),
        }
    }

    pub fn to_json(&self) -> Vec<u8> {
        canonical_json(&self.0).expect("validated Feature Plan fits the canonical size limit")
    }

    pub fn load(bytes: &[u8]) -> Result<Self, PlanLoadError> {
        let identity = FeatureEngineIdentity::native()
            .map_err(|_| PlanLoadError::UnsupportedEngineIdentity)?;
        Self::load_for_engine(bytes, &identity)
    }

    pub fn load_for_engine(
        bytes: &[u8],
        identity: &FeatureEngineIdentity,
    ) -> Result<Self, PlanLoadError> {
        if bytes.len() > MAX_CANONICAL_JSON_BYTES {
            return Err(PlanLoadError::TooLarge);
        }
        let raw = serde_json::from_slice::<Value>(bytes).map_err(|_| PlanLoadError::InvalidJson)?;
        if !raw.is_object() {
            return Err(PlanLoadError::InvalidJson);
        }
        let stored_schema_version = raw
            .get("planSchemaVersion")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if stored_schema_version.as_deref() != Some(FEATURE_PLAN_SCHEMA_VERSION) {
            return Err(PlanLoadError::ResetRequired {
                stored_schema_version,
            });
        }
        let document =
            serde_json::from_value::<PlanDocument>(raw).map_err(|_| PlanLoadError::InvalidJson)?;
        let canonical = canonical_json(&document).map_err(|error| match error {
            CanonicalJsonError::TooLarge => PlanLoadError::TooLarge,
            CanonicalJsonError::InvalidNumber | CanonicalJsonError::Serialization => {
                PlanLoadError::InvalidJson
            }
        })?;
        if canonical != bytes {
            return Err(PlanLoadError::NonCanonical);
        }
        let expected_hash =
            sha256_hex(&canonical_json(&document.content).map_err(|_| PlanLoadError::InvalidJson)?);
        if document.plan_hash != expected_hash {
            return Err(PlanLoadError::HashMismatch);
        }
        validate_content(&document.content)
            .map_err(|error| PlanLoadError::InvalidContract(error.codes().join(",")))?;
        if document.content.feature_engine_version != identity.feature_engine_version
            || document.content.feature_engine_source_sha256
                != identity.feature_engine_source_sha256
            || document.content.feature_engine_build_id != identity.feature_engine_build_id
            || document.content.operator_catalog.version != identity.operator_catalog_version
            || document.content.indicator_engine_version != identity.indicator_engine_version
            || document.content.indicator_catalog_version != identity.indicator_catalog_version
            || document.content.ta_lib_version != identity.ta_lib_version
            || document.content.ta_source_sha256 != identity.ta_source_sha256
            || document.content.wrapper_sha256 != identity.wrapper_sha256
            || document.content.target_triple != identity.target_triple
            || document.content.compiler_and_flags_sha256 != identity.compiler_and_flags_sha256
            || document.content.engine_build_id != identity.engine_build_id
        {
            return Err(PlanLoadError::UnsupportedEngineIdentity);
        }
        Ok(Self(document))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObservationRange {
    pub start_time_ms: i64,
    pub end_time_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FeatureMaterializationRequest {
    pub user_id: String,
    pub feature_plan_hash: String,
    pub snapshot_id: String,
    pub point_in_time_universe_id: String,
    pub observation_range: ObservationRange,
    #[serde(default)]
    pub parameters: BTreeMap<String, Value>,
    pub seed: u64,
}

impl FeatureMaterializationRequest {
    pub fn new(
        user_id: impl Into<String>,
        feature_plan_hash: impl Into<String>,
        snapshot_id: impl Into<String>,
        point_in_time_universe_id: impl Into<String>,
        observation_range: ObservationRange,
        parameters: BTreeMap<String, Value>,
        seed: u64,
    ) -> Result<Self, RequestValidationError> {
        let request = Self {
            user_id: user_id.into(),
            feature_plan_hash: feature_plan_hash.into(),
            snapshot_id: snapshot_id.into(),
            point_in_time_universe_id: point_in_time_universe_id.into(),
            observation_range,
            parameters,
            seed,
        };
        if request.user_id.is_empty()
            || !is_sha256(&request.feature_plan_hash)
            || request.snapshot_id.is_empty()
            || request.point_in_time_universe_id.is_empty()
            || request.observation_range.start_time_ms >= request.observation_range.end_time_ms
        {
            return Err(RequestValidationError);
        }
        Ok(request)
    }

    pub fn request_hash(&self) -> String {
        sha256_hex(&canonical_json(self).expect("materialization request is serializable"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestValidationError;

impl fmt::Display for RequestValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("invalid-feature-materialization-request")
    }
}

impl std::error::Error for RequestValidationError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FeatureUnavailabilityReason {
    Warmup,
    BarGap,
    MissingMarketInput,
    MissingDependency,
    UnknownUniverse,
    InsufficientCoverage,
    UndefinedArithmetic,
    ArtifactMissingInstrument,
    CorporateActionUnavailable,
}

impl FeatureUnavailabilityReason {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Warmup => "warmup",
            Self::BarGap => "bar-gap",
            Self::MissingMarketInput => "missing-market-input",
            Self::MissingDependency => "missing-dependency",
            Self::UnknownUniverse => "unknown-universe",
            Self::InsufficientCoverage => "insufficient-coverage",
            Self::UndefinedArithmetic => "undefined-arithmetic",
            Self::ArtifactMissingInstrument => "artifact-missing-instrument",
            Self::CorporateActionUnavailable => "corporate-action-unavailable",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "state",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum FeatureObservationValue {
    Available { value: f64, available_at_ms: i64 },
    Unavailable { reason: FeatureUnavailabilityReason },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FeatureObservation {
    pub output_name: String,
    pub instrument_id: String,
    pub observation_time_ms: i64,
    pub value: FeatureObservationValue,
}

impl FeatureObservation {
    pub fn available(
        output_name: impl Into<String>,
        instrument_id: impl Into<String>,
        observation_time_ms: i64,
        value: f64,
        available_at_ms: i64,
    ) -> Result<Self, FeatureEvaluationError> {
        let output_name = output_name.into();
        let instrument_id = instrument_id.into();
        if !is_lower_kebab(&output_name) || instrument_id.is_empty() {
            return Err(FeatureEvaluationError::observation(
                FeatureEvaluationErrorCode::InvalidObservation,
                EvaluationStage::Validation,
                &instrument_id,
                observation_time_ms,
            ));
        }
        if !value.is_finite() {
            return Err(FeatureEvaluationError::observation(
                FeatureEvaluationErrorCode::NonFiniteOutput,
                EvaluationStage::Invariant,
                &instrument_id,
                observation_time_ms,
            ));
        }
        Ok(Self {
            output_name,
            instrument_id,
            observation_time_ms,
            value: FeatureObservationValue::Available {
                value,
                available_at_ms,
            },
        })
    }

    pub fn unavailable(
        output_name: impl Into<String>,
        instrument_id: impl Into<String>,
        observation_time_ms: i64,
        reason: FeatureUnavailabilityReason,
    ) -> Result<Self, FeatureEvaluationError> {
        let output_name = output_name.into();
        let instrument_id = instrument_id.into();
        if !is_lower_kebab(&output_name) || instrument_id.is_empty() {
            return Err(FeatureEvaluationError::observation(
                FeatureEvaluationErrorCode::InvalidObservation,
                EvaluationStage::Validation,
                &instrument_id,
                observation_time_ms,
            ));
        }
        Ok(Self {
            output_name,
            instrument_id,
            observation_time_ms,
            value: FeatureObservationValue::Unavailable { reason },
        })
    }

    pub fn reason(&self) -> Option<FeatureUnavailabilityReason> {
        match self.value {
            FeatureObservationValue::Available { .. } => None,
            FeatureObservationValue::Unavailable { reason } => Some(reason),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EvaluationStage {
    Validation,
    Input,
    Operator,
    Availability,
    Invariant,
    Materialization,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FeatureEvaluationErrorCode {
    InvalidObservation,
    InvalidIdentity,
    NonFiniteOutput,
    BrokenShape,
    InvalidInvariant,
    OperatorFailure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FeatureEvaluationError {
    pub code: FeatureEvaluationErrorCode,
    pub stage: EvaluationStage,
    pub node_id: Option<String>,
    pub instrument_id: Option<String>,
    pub observation_time_ms: Option<i64>,
    pub diagnostic: String,
}

impl FeatureEvaluationError {
    fn observation(
        code: FeatureEvaluationErrorCode,
        stage: EvaluationStage,
        instrument_id: &str,
        observation_time_ms: i64,
    ) -> Self {
        Self {
            code,
            stage,
            node_id: None,
            instrument_id: (!instrument_id.is_empty()).then(|| instrument_id.to_owned()),
            observation_time_ms: Some(observation_time_ms),
            diagnostic: String::new(),
        }
    }

    pub fn code(&self) -> &'static str {
        match self.code {
            FeatureEvaluationErrorCode::InvalidObservation => "invalid-observation",
            FeatureEvaluationErrorCode::InvalidIdentity => "invalid-identity",
            FeatureEvaluationErrorCode::NonFiniteOutput => "non-finite-output",
            FeatureEvaluationErrorCode::BrokenShape => "broken-shape",
            FeatureEvaluationErrorCode::InvalidInvariant => "invalid-invariant",
            FeatureEvaluationErrorCode::OperatorFailure => "operator-failure",
        }
    }
}

impl fmt::Display for FeatureEvaluationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.code())
    }
}

impl std::error::Error for FeatureEvaluationError {}

fn validate_draft(draft: &FeaturePlanDraft) -> Result<(), PlanValidationError> {
    let mut issues = Vec::new();
    if !draft.engine_identity.validate() {
        issues.push(issue("invalid-engine-identity", None));
    }
    if !draft.operator_catalog.is_initial() {
        issues.push(issue("unsupported-operator-catalog", None));
    }
    if draft.consumer_warmup_bars > MAX_EFFECTIVE_WARMUP_BARS {
        issues.push(issue("effective-warmup-too-large", None));
    }
    if !draft.consumer_package_sha256.is_empty() && !is_sha256(&draft.consumer_package_sha256) {
        issues.push(issue("invalid-consumer-package-hash", None));
    }
    let mut definition_keys = BTreeSet::new();
    let mut output_names = BTreeSet::new();
    for definition in &draft.definitions {
        if !definition_keys.insert((definition.definition_id(), definition.revision())) {
            issues.push(issue("duplicate-definition-revision", None));
        }
        if definition.definition_hash()
            != sha256_hex(&canonical_json(&definition.0.content).unwrap_or_default())
        {
            issues.push(issue("definition-hash-mismatch", None));
        }
        for output in definition.outputs() {
            if !output_names.insert(&output.name) {
                issues.push(issue("duplicate-output-name", Some(output.name.clone())));
            }
        }
        if definition.0.content.operator_catalog_version != draft.operator_catalog.version {
            issues.push(issue("definition-catalog-mismatch", None));
        }
        if let Err(error) = validate_definition(&definition.0.content, &draft.operator_catalog) {
            issues.extend(error.issues);
        }
    }
    if draft.definitions.is_empty() && draft.slots.is_empty() {
        issues.push(issue("missing-feature-outputs", None));
    }
    if draft.slots.len() > MAX_FEATURE_OUTPUTS {
        issues.push(issue("too-many-feature-outputs", None));
    }
    let total_outputs = draft
        .definitions
        .iter()
        .map(|definition| definition.outputs().len())
        .sum::<usize>()
        .saturating_add(draft.slots.len());
    if total_outputs == 0 {
        issues.push(issue("missing-feature-outputs", None));
    } else if total_outputs > MAX_FEATURE_OUTPUTS {
        issues.push(issue("too-many-feature-outputs", None));
    }
    validate_slots(&draft.slots, &draft.factors, &mut issues);
    for slot in &draft.slots {
        if !output_names.insert(&slot.name) {
            issues.push(issue("duplicate-output-name", Some(slot.name.clone())));
        }
    }
    validate_factors(&draft.factors, &mut issues);
    if draft
        .definitions
        .iter()
        .map(|definition| definition.nodes().len())
        .sum::<usize>()
        > MAX_DEFINITION_NODES
    {
        issues.push(issue("too-many-definition-nodes", None));
    }
    let mut artifact_ids = BTreeSet::new();
    for artifact in &draft.artifacts {
        if artifact.artifact_id.is_empty() || !artifact_ids.insert(&artifact.artifact_id) {
            issues.push(issue("invalid-artifact-binding", None));
        }
    }
    for definition in &draft.definitions {
        for node in definition.nodes() {
            for input in &node.inputs {
                if let FeatureInput::Artifact { artifact_id } = input {
                    if !artifact_ids.contains(artifact_id) {
                        issues.push(issue("unbound-artifact-input", Some(node.id.clone())));
                    }
                }
            }
        }
    }
    if effective_warmup(&draft.definitions, &draft.slots, &draft.factors)
        > MAX_EFFECTIVE_WARMUP_BARS
    {
        issues.push(issue("effective-warmup-too-large", None));
    }
    sort_issues(&mut issues);
    if issues.is_empty() {
        Ok(())
    } else {
        Err(PlanValidationError::Issues(issues))
    }
}

fn validate_content(content: &PlanContent) -> Result<(), PlanValidationError> {
    let draft = FeaturePlanDraft {
        definitions: content.definitions.clone(),
        slots: content.slots.clone(),
        factors: content.factors.clone(),
        artifacts: content.artifacts.clone(),
        consumer_package_sha256: content.consumer_package_sha256.clone(),
        consumer_parameters: content.consumer_parameters.clone(),
        consumer_warmup_bars: content.consumer_warmup_bars,
        engine_identity: FeatureEngineIdentity {
            feature_engine_version: content.feature_engine_version.clone(),
            feature_engine_source_sha256: content.feature_engine_source_sha256.clone(),
            feature_engine_build_id: content.feature_engine_build_id.clone(),
            operator_catalog_version: content.operator_catalog.version.clone(),
            indicator_engine_version: content.indicator_engine_version.clone(),
            indicator_catalog_version: content.indicator_catalog_version.clone(),
            ta_lib_version: content.ta_lib_version.clone(),
            ta_source_sha256: content.ta_source_sha256.clone(),
            wrapper_sha256: content.wrapper_sha256.clone(),
            target_triple: content.target_triple.clone(),
            compiler_and_flags_sha256: content.compiler_and_flags_sha256.clone(),
            engine_build_id: content.engine_build_id.clone(),
        },
        operator_catalog: content.operator_catalog.clone(),
    };
    if content.unavailability_reason_version != FEATURE_UNAVAILABILITY_REASON_VERSION {
        return Err(PlanValidationError::Issues(vec![issue(
            "unsupported-unavailability-reason-version",
            None,
        )]));
    }
    validate_draft(&draft).and_then(|_| {
        let expected = effective_warmup(&content.definitions, &content.slots, &content.factors);
        if content.plan_schema_version != FEATURE_PLAN_SCHEMA_VERSION {
            return Err(PlanValidationError::Issues(vec![issue(
                "unsupported-plan-schema",
                None,
            )]));
        }
        if content.effective_warmup_bars != expected {
            return Err(PlanValidationError::Issues(vec![issue(
                "invalid-effective-warmup",
                None,
            )]));
        }
        Ok(())
    })
}

fn validate_slots(
    slots: &[FeatureSlot],
    factors: &[FeatureFactor],
    issues: &mut Vec<ValidationIssue>,
) {
    let mut names = BTreeSet::new();
    for slot in slots {
        if !is_lower_kebab(&slot.name) {
            issues.push(issue("invalid-output-name", Some(slot.name.clone())));
        }
        if !names.insert(&slot.name) {
            issues.push(issue("duplicate-output-name", Some(slot.name.clone())));
        }
        if slot.warmup_bars > MAX_EFFECTIVE_WARMUP_BARS {
            issues.push(issue("effective-warmup-too-large", Some(slot.name.clone())));
        }
        match &slot.source {
            FeatureSource::Market { .. } => {
                if slot.warmup_bars != 0 {
                    issues.push(issue("market-input-warmup", Some(slot.name.clone())));
                }
            }
            FeatureSource::BuiltIn {
                indicator,
                output,
                real_inputs: _,
                parameters,
            } => {
                if indicator.is_empty() || !is_lower_kebab(output) {
                    issues.push(issue("invalid-builtin-source", Some(slot.name.clone())));
                }
                if parameters.keys().any(|name| !is_lower_kebab(name)) {
                    issues.push(issue("invalid-builtin-parameters", Some(slot.name.clone())));
                }
            }
            FeatureSource::External {
                dependency_alias,
                output,
            } => {
                if !is_lower_kebab(dependency_alias)
                    || !is_lower_kebab(output)
                    || !factors.iter().any(|factor| {
                        factor.alias == *dependency_alias
                            && factor.output_names.iter().any(|name| name == output)
                    })
                {
                    issues.push(issue("invalid-factor-output", Some(slot.name.clone())));
                }
            }
            FeatureSource::Signal {
                dataset_id,
                signal_name,
                snapshot_id,
                instrument_id,
                venue,
                bar_interval,
                contract,
                producer_segments,
                artifact_provenance,
                evidence_state,
                component_lock,
            } => {
                if !is_sha256(dataset_id)
                    || !is_lower_kebab(signal_name)
                    || snapshot_id.is_empty()
                    || instrument_id.is_empty()
                    || venue.is_empty()
                    || bar_interval.is_empty()
                    || producer_segments.is_empty()
                    || evidence_state.is_empty()
                    || !valid_signal_contract(contract, signal_name)
                    || !contract_object_list(producer_segments)
                    || !artifact_provenance.is_object()
                    || !contract_object_list(component_lock)
                    || slot.warmup_bars != 0
                {
                    issues.push(issue("invalid-signal-source", Some(slot.name.clone())));
                }
            }
        }
    }
}

fn valid_signal_contract(contract: &Value, signal_name: &str) -> bool {
    let Some(contract) = contract.as_object() else {
        return false;
    };
    contract.get("name").and_then(Value::as_str) == Some(signal_name)
        && contract.get("predictionKind").is_some_and(Value::is_object)
        && contract.get("forecastTarget").is_some_and(Value::is_object)
        && contract.get("valueScale").is_some_and(Value::is_object)
        && contract
            .get("horizonBars")
            .and_then(Value::as_u64)
            .is_some()
}

fn contract_object_list(values: &[Value]) -> bool {
    values.iter().all(Value::is_object)
}

fn validate_factors(factors: &[FeatureFactor], issues: &mut Vec<ValidationIssue>) {
    if factors.len() > MAX_FEATURE_OUTPUTS {
        issues.push(issue("too-many-factor-instances", None));
    }
    let mut aliases = BTreeSet::new();
    for factor in factors {
        if !is_lower_kebab(&factor.alias) || !aliases.insert(&factor.alias) {
            issues.push(issue("invalid-factor-alias", Some(factor.alias.clone())));
        }
        if factor.output_names.is_empty() || factor.output_names.len() > MAX_FEATURE_OUTPUTS {
            issues.push(issue("invalid-factor-contract", Some(factor.alias.clone())));
        }
        let mut outputs = BTreeSet::new();
        if factor
            .output_names
            .iter()
            .any(|name| !is_lower_kebab(name) || !outputs.insert(name))
        {
            issues.push(issue("invalid-factor-contract", Some(factor.alias.clone())));
        }
        if factor.warmup_bars > MAX_EFFECTIVE_WARMUP_BARS {
            issues.push(issue(
                "effective-warmup-too-large",
                Some(factor.alias.clone()),
            ));
        }
    }
}

fn validate_definition(
    content: &DefinitionContent,
    catalog: &FeatureOperatorCatalog,
) -> Result<(), DefinitionValidationError> {
    let mut issues = Vec::new();
    if content.definition_schema_version != FEATURE_DEFINITION_SCHEMA_VERSION {
        issues.push(issue("unsupported-definition-schema", None));
    }
    if content.revision == 0 {
        issues.push(issue("invalid-definition-revision", None));
    }
    if content.operator_catalog_version != catalog.version || !catalog.is_initial() {
        issues.push(issue("unsupported-operator-catalog", None));
    }
    if content.nodes.is_empty() {
        issues.push(issue("missing-definition-nodes", None));
    }
    if content.nodes.len() > MAX_DEFINITION_NODES {
        issues.push(issue("too-many-definition-nodes", None));
    }
    if content.outputs.is_empty() || content.outputs.len() > MAX_FEATURE_OUTPUTS {
        issues.push(issue("invalid-output-count", None));
    }
    let mut node_ids = BTreeSet::new();
    for node in &content.nodes {
        if node.id.is_empty() || !node_ids.insert(&node.id) {
            issues.push(issue("invalid-node-id", Some(node.id.clone())));
        }
        if !catalog.supports(&node.operator) {
            issues.push(issue("unsupported-feature-operator", Some(node.id.clone())));
        }
        validate_operator_contract(node, &mut issues);
        if let FeatureOperator::Indicator { id } = &node.operator {
            if id.is_empty() {
                issues.push(issue("invalid-feature-operator", Some(node.id.clone())));
            }
        }
        if matches!(
            node.operator,
            FeatureOperator::CrossSectionalRank
                | FeatureOperator::CrossSectionalPercentile
                | FeatureOperator::CrossSectionalZScore
        ) && node.scope != FeatureScope::CrossSectional
        {
            issues.push(issue("invalid-operator-scope", Some(node.id.clone())));
        }
        if node.scope.rank() > content.scope.rank() {
            issues.push(issue("definition-scope-mismatch", Some(node.id.clone())));
        }
        if node.warmup_bars > MAX_EFFECTIVE_WARMUP_BARS {
            issues.push(issue("effective-warmup-too-large", Some(node.id.clone())));
        }
    }
    let nodes = content
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<HashMap<_, _>>();
    // ponytail: bounded O(nodes^2) graph validation; a topological cache can replace this if the 256-node ceiling changes.
    for node in &content.nodes {
        let mut states = HashMap::new();
        visit_node(node.id.as_str(), &nodes, &mut states, 1, &mut issues);
    }
    let mut output_names = BTreeSet::new();
    for output in &content.outputs {
        if !is_lower_kebab(&output.name) {
            issues.push(issue("invalid-output-name", Some(output.name.clone())));
        }
        if !output_names.insert(&output.name) {
            issues.push(issue("duplicate-output-name", Some(output.name.clone())));
        }
        match nodes.get(output.node_id.as_str()) {
            Some(node) if node.scope == content.scope => {}
            Some(_) => issues.push(issue("output-scope-mismatch", Some(output.name.clone()))),
            None => issues.push(issue("unknown-output-node", Some(output.name.clone()))),
        }
    }
    sort_issues(&mut issues);
    if issues.is_empty() {
        Ok(())
    } else {
        Err(DefinitionValidationError { issues })
    }
}

fn validate_operator_contract(node: &FeatureNode, issues: &mut Vec<ValidationIssue>) {
    let parameter = |name: &str| node.parameters.get(name);
    match &node.operator {
        FeatureOperator::BackwardSimpleReturn | FeatureOperator::BackwardLogReturn => {
            if parameter("direction")
                .and_then(Value::as_str)
                .is_some_and(|direction| direction != "backward")
            {
                issues.push(issue("future-return-not-allowed", Some(node.id.clone())));
            }
            if parameter("period")
                .and_then(Value::as_u64)
                .is_some_and(|period| period == 0 || period > MAX_EFFECTIVE_WARMUP_BARS as u64)
            {
                issues.push(issue("invalid-return-period", Some(node.id.clone())));
            }
        }
        FeatureOperator::RollingMean
        | FeatureOperator::RollingPopulationStandardDeviation
        | FeatureOperator::RollingMinimum
        | FeatureOperator::RollingMaximum
        | FeatureOperator::RollingQuoteVolume => {
            if parameter("window")
                .and_then(Value::as_u64)
                .is_some_and(|window| window == 0 || window > MAX_EFFECTIVE_WARMUP_BARS as u64)
            {
                issues.push(issue("invalid-rolling-window", Some(node.id.clone())));
            }
        }
        FeatureOperator::OneHot => {
            if parameter("category").is_none() && parameter("value").is_none() {
                issues.push(issue("missing-one-hot-category", Some(node.id.clone())));
            }
        }
        FeatureOperator::Sine | FeatureOperator::Cosine => {
            if parameter("period")
                .and_then(Value::as_f64)
                .is_some_and(|period| !period.is_finite() || period <= 0.0)
            {
                issues.push(issue("invalid-cycle-period", Some(node.id.clone())));
            }
        }
        _ => {}
    }
}

fn visit_node(
    id: &str,
    nodes: &HashMap<&str, &FeatureNode>,
    states: &mut HashMap<String, u8>,
    depth: usize,
    issues: &mut Vec<ValidationIssue>,
) {
    match states.get(id).copied() {
        Some(1) => {
            issues.push(issue("dependency-cycle", Some(id.to_owned())));
            return;
        }
        Some(2) => return,
        _ => {}
    }
    if depth > MAX_DEPENDENCY_DEPTH {
        issues.push(issue("dependency-depth-too-large", Some(id.to_owned())));
        return;
    }
    let Some(node) = nodes.get(id).copied() else {
        issues.push(issue("unknown-dependency", Some(id.to_owned())));
        return;
    };
    states.insert(id.to_owned(), 1);
    for input in &node.inputs {
        let FeatureInput::Node { node_id } = input else {
            if let FeatureInput::Artifact { artifact_id } = input {
                if artifact_id.is_empty() {
                    issues.push(issue("invalid-artifact-input", Some(id.to_owned())));
                }
            }
            continue;
        };
        let Some(dependency) = nodes.get(node_id.as_str()).copied() else {
            issues.push(issue("unknown-dependency", Some(node_id.clone())));
            continue;
        };
        if dependency.scope == FeatureScope::CrossSectional {
            issues.push(issue(
                "cross-sectional-output-terminal",
                Some(node.id.clone()),
            ));
        } else if dependency.scope.rank() > node.scope.rank() {
            issues.push(issue("invalid-scope-expansion", Some(node.id.clone())));
        }
        visit_node(node_id, nodes, states, depth + 1, issues);
    }
    states.insert(id.to_owned(), 2);
}

fn effective_warmup(
    definitions: &[FeatureDefinition],
    slots: &[FeatureSlot],
    factors: &[FeatureFactor],
) -> u32 {
    definitions
        .iter()
        .flat_map(|definition| definition.nodes().iter().map(|node| node.warmup_bars))
        .chain(slots.iter().map(|slot| slot.warmup_bars))
        .chain(factors.iter().map(|factor| factor.warmup_bars))
        .max()
        .unwrap_or(0)
}

fn issue(code: impl Into<String>, path: Option<String>) -> ValidationIssue {
    ValidationIssue {
        code: code.into(),
        path,
    }
}

fn sort_issues(issues: &mut [ValidationIssue]) {
    issues.sort_by(|left, right| (&left.code, &left.path).cmp(&(&right.code, &right.path)));
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CanonicalJsonError {
    TooLarge,
    InvalidNumber,
    Serialization,
}

fn canonical_json<T: Serialize>(value: &T) -> Result<Vec<u8>, CanonicalJsonError> {
    let value = serde_json::to_value(value).map_err(|_| CanonicalJsonError::Serialization)?;
    let mut bytes = Vec::new();
    write_jcs(&value, &mut bytes)?;
    if bytes.len() > MAX_CANONICAL_JSON_BYTES {
        return Err(CanonicalJsonError::TooLarge);
    }
    Ok(bytes)
}

pub fn canonicalize_json(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let value: Value = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    canonical_json(&value).map_err(|error| match error {
        CanonicalJsonError::TooLarge => "canonical-json-too-large".into(),
        CanonicalJsonError::InvalidNumber => "invalid-json-number".into(),
        CanonicalJsonError::Serialization => "invalid-json".into(),
    })
}

fn write_jcs(value: &Value, bytes: &mut Vec<u8>) -> Result<(), CanonicalJsonError> {
    match value {
        Value::Null => bytes.extend_from_slice(b"null"),
        Value::Bool(value) => bytes.extend_from_slice(if *value { b"true" } else { b"false" }),
        Value::Number(number) => bytes.extend_from_slice(canonical_number(number)?.as_bytes()),
        Value::String(value) => {
            let encoded =
                serde_json::to_string(value).map_err(|_| CanonicalJsonError::Serialization)?;
            bytes.extend_from_slice(encoded.as_bytes());
        }
        Value::Array(values) => {
            bytes.push(b'[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    bytes.push(b',');
                }
                write_jcs(value, bytes)?;
            }
            bytes.push(b']');
        }
        Value::Object(values) => {
            let mut entries = values.iter().collect::<Vec<_>>();
            entries.sort_by(|(left, _), (right, _)| left.encode_utf16().cmp(right.encode_utf16()));
            bytes.push(b'{');
            for (index, (key, value)) in entries.into_iter().enumerate() {
                if index > 0 {
                    bytes.push(b',');
                }
                let encoded =
                    serde_json::to_string(key).map_err(|_| CanonicalJsonError::Serialization)?;
                bytes.extend_from_slice(encoded.as_bytes());
                bytes.push(b':');
                write_jcs(value, bytes)?;
            }
            bytes.push(b'}');
        }
    }
    if bytes.len() > MAX_CANONICAL_JSON_BYTES {
        return Err(CanonicalJsonError::TooLarge);
    }
    Ok(())
}

fn canonical_number(number: &Number) -> Result<String, CanonicalJsonError> {
    let value = number.as_f64().ok_or(CanonicalJsonError::InvalidNumber)?;
    if !value.is_finite() {
        return Err(CanonicalJsonError::InvalidNumber);
    }
    if value == 0.0 {
        return Ok("0".into());
    }
    let encoded = value.to_string();
    let negative = encoded.starts_with('-');
    let unsigned = encoded.strip_prefix('-').unwrap_or(&encoded);
    let (mantissa, exponent) = unsigned
        .split_once('e')
        .map_or((unsigned, 0), |(mantissa, exponent)| {
            (mantissa, exponent.parse::<i32>().unwrap_or(i32::MIN))
        });
    if exponent == i32::MIN {
        return Err(CanonicalJsonError::InvalidNumber);
    }
    let decimal_offset = mantissa.find('.').unwrap_or(mantissa.len()) as i32 + exponent;
    let mut digits = mantissa.replace('.', "");
    let removed_leading = digits.len() - digits.trim_start_matches('0').len();
    digits = digits.trim_start_matches('0').to_owned();
    let decimal_offset = decimal_offset - removed_leading as i32;
    while digits.len() > 1 && digits.ends_with('0') {
        digits.pop();
    }
    let scientific_exponent = decimal_offset - 1;
    let mut output = String::new();
    if negative {
        output.push('-');
    }
    if (-6..21).contains(&scientific_exponent) {
        if decimal_offset <= 0 {
            output.push_str("0.");
            output.extend(std::iter::repeat_n('0', (-decimal_offset) as usize));
            output.push_str(&digits);
        } else if decimal_offset as usize >= digits.len() {
            output.push_str(&digits);
            output.extend(std::iter::repeat_n(
                '0',
                decimal_offset as usize - digits.len(),
            ));
        } else {
            let split = decimal_offset as usize;
            output.push_str(&digits[..split]);
            output.push('.');
            output.push_str(&digits[split..]);
        }
    } else {
        output.push_str(&digits[..1]);
        if digits.len() > 1 {
            output.push('.');
            output.push_str(&digits[1..]);
        }
        output.push('e');
        output.push_str(&format!("{scientific_exponent:+}"));
    }
    Ok(output)
}

fn plan_validation_from_json(error: CanonicalJsonError) -> PlanValidationError {
    PlanValidationError::Issues(vec![issue(
        match error {
            CanonicalJsonError::TooLarge => "plan-json-too-large",
            CanonicalJsonError::InvalidNumber => "invalid-plan-number",
            CanonicalJsonError::Serialization => "invalid-plan-json",
        },
        None,
    )])
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn sha256(bytes: &[u8]) -> String {
    sha256_hex(bytes)
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

pub fn is_lower_kebab(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.as_bytes()[0].is_ascii_lowercase()
        && !value.ends_with('-')
        && value.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jcs_sorts_property_names_by_utf16_code_units() {
        let bytes = canonicalize_json(br#"{"\uE000":1,"\ud83d\ude00":2,"a":3}"#).unwrap();
        assert_eq!(bytes, "{\"a\":3,\"😀\":2,\"\u{e000}\":1}".as_bytes());
    }

    #[test]
    fn jcs_uses_ecmascript_number_shape_for_common_boundaries() {
        assert_eq!(
            canonicalize_json(br#"{"a":1.0,"b":1e16,"c":1e20,"d":1e21,"e":-0.0}"#).unwrap(),
            br#"{"a":1,"b":10000000000000000,"c":100000000000000000000,"d":1e+21,"e":0}"#
        );
        assert_eq!(
            canonicalize_json(br#"{"a":1000000000000000000000,"b":9007199254740993}"#).unwrap(),
            br#"{"a":1e+21,"b":9007199254740992}"#
        );
    }
}
