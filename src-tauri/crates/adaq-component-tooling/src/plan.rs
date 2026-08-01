use std::{collections::BTreeMap, fmt::Write as _, io::Write};

use adaq_indicator_engine::{
    EngineIdentity as NativeEngineIdentity, IndicatorEngine, IndicatorRequest,
    MarketField as EngineMarketField, ParameterValue,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::package::is_lower_kebab;
use crate::{
    ComponentKind, ComponentManifest, ComponentParameterValue, FeatureSlotSource, MarketField,
    ParameterType,
};

const PLAN_SCHEMA_VERSION: &str = "1.0.0";
const CATALOG_VERSION: &str = "adaq-indicator-catalog@1.0.0";
const ENGINE_VERSION: &str = "adaq-indicator-engine@1.0.0";
const TA_LIB_VERSION: &str = "0.7.1";
const MAX_FACTOR_INSTANCES: usize = 64;
const MAX_FACTOR_OUTPUTS: usize = 64;
const MAX_FEATURE_SLOTS: usize = 256;
const MAX_BUILTIN_REQUESTS: usize = 256;
const MAX_EFFECTIVE_WARMUP_BARS: u32 = 100_000;
const MAX_CANONICAL_PLAN_JSON_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone)]
pub struct FactorInstancePlanInput<'a> {
    pub alias: &'a str,
    pub manifest: &'a ComponentManifest,
    pub parameters: Vec<ComponentParameterValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineIdentity {
    pub engine_version: String,
    pub ta_lib_version: String,
    pub ta_source_sha256: String,
    pub catalog_version: String,
    pub wrapper_sha256: String,
    pub target_triple: String,
    pub compiler_and_flags_sha256: String,
    pub engine_build_id: String,
}

impl From<&NativeEngineIdentity> for EngineIdentity {
    fn from(identity: &NativeEngineIdentity) -> Self {
        Self {
            engine_version: identity.engine_version.into(),
            ta_lib_version: identity.ta_lib_version.into(),
            ta_source_sha256: identity.ta_source_sha256.into(),
            catalog_version: identity.catalog_version.into(),
            wrapper_sha256: identity.wrapper_sha256.into(),
            target_triple: identity.target_triple.into(),
            compiler_and_flags_sha256: identity.compiler_and_flags_sha256.into(),
            engine_build_id: identity.build_id.into(),
        }
    }
}

pub fn native_engine_identity() -> Result<EngineIdentity, adaq_indicator_engine::EngineError> {
    IndicatorEngine::initialize().map(|engine| engine.identity().into())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanIssue {
    pub code: String,
    pub slot: Option<String>,
    pub source: Option<String>,
    pub field: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanValidationError {
    pub issues: Vec<PlanIssue>,
}

impl std::fmt::Display for PlanValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "Feature Plan validation failed with {} issue(s)",
            self.issues.len()
        )
    }
}

impl std::error::Error for PlanValidationError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanLoadError {
    pub code: String,
}

impl std::fmt::Display for PlanLoadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.code)
    }
}

impl std::error::Error for PlanLoadError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PlanDocument {
    #[serde(flatten)]
    content: PlanContent,
    plan_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PlanContent {
    plan_schema_version: String,
    consumer_package_sha256: String,
    catalog_version: String,
    engine_version: String,
    ta_lib_version: String,
    ta_source_sha256: String,
    wrapper_sha256: String,
    target_triple: String,
    compiler_and_flags_sha256: String,
    engine_build_id: String,
    consumer_parameters: Vec<FrozenConsumerParameter>,
    consumer_warmup_bars: u32,
    slots: Vec<FrozenSlot>,
    #[serde(default)]
    factors: Vec<FrozenFactor>,
    effective_warmup_bars: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FrozenConsumerParameter {
    name: String,
    value: ComponentParameterValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FrozenSlot {
    name: String,
    source: FrozenSource,
    warmup_bars: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
enum FrozenSource {
    Market {
        field: MarketField,
    },
    External {
        dependency_alias: String,
        output: String,
    },
    BuiltIn {
        indicator: String,
        output: String,
        real_inputs: Vec<MarketField>,
        parameters: BTreeMap<String, FrozenBuiltInParameter>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "kebab-case")]
pub enum FrozenBuiltInParameter {
    Integer(i32),
    Real(String),
    Enum(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FrozenFactor {
    alias: String,
    parameters: Vec<ComponentParameterValue>,
    output_names: Vec<String>,
    warmup_bars: u32,
}

struct ResolvedBuiltIn {
    real_inputs: Vec<MarketField>,
    parameters: BTreeMap<String, FrozenBuiltInParameter>,
    warmup_bars: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrozenFeaturePlan(PlanDocument);

impl FrozenFeaturePlan {
    pub fn plan_hash(&self) -> &str {
        &self.0.plan_hash
    }

    pub fn slot_names(&self) -> impl ExactSizeIterator<Item = &str> {
        self.0.content.slots.iter().map(|slot| slot.name.as_str())
    }

    pub fn market_fields(&self) -> impl ExactSizeIterator<Item = MarketField> + '_ {
        self.0.content.slots.iter().map(|slot| match slot.source {
            FrozenSource::Market { field } => field,
            FrozenSource::External { .. } => panic!("external Feature Slot is not a Market Field"),
            FrozenSource::BuiltIn { .. } => panic!("builtin Feature Slot is not a Market Field"),
        })
    }

    pub fn sources(&self) -> impl ExactSizeIterator<Item = FrozenSourceView<'_>> {
        self.0.content.slots.iter().map(|slot| match &slot.source {
            FrozenSource::Market { field } => FrozenSourceView::Market(*field),
            FrozenSource::External {
                dependency_alias,
                output,
            } => FrozenSourceView::External {
                dependency_alias,
                output,
            },
            FrozenSource::BuiltIn {
                indicator,
                output,
                real_inputs,
                parameters,
            } => FrozenSourceView::BuiltIn {
                indicator,
                output,
                real_inputs,
                parameters,
            },
        })
    }

    pub fn factors(&self) -> impl ExactSizeIterator<Item = FrozenFactorView<'_>> {
        self.0
            .content
            .factors
            .iter()
            .map(|factor| FrozenFactorView {
                alias: &factor.alias,
                parameters: &factor.parameters,
                output_names: &factor.output_names,
                warmup_bars: factor.warmup_bars,
            })
    }

    pub fn effective_warmup_bars(&self) -> u32 {
        self.0.content.effective_warmup_bars
    }

    pub fn to_json(&self) -> Vec<u8> {
        canonical_json(&self.0).expect("a validated Feature Plan fits the canonical size limit")
    }

    pub fn load(bytes: &[u8]) -> Result<Self, PlanLoadError> {
        let identity =
            native_engine_identity().map_err(|_| load_error("unsupported-engine-identity"))?;
        Self::load_for_engine(bytes, &identity)
    }

    pub fn load_for_engine(bytes: &[u8], identity: &EngineIdentity) -> Result<Self, PlanLoadError> {
        if bytes.len() > MAX_CANONICAL_PLAN_JSON_BYTES {
            return Err(load_error("plan-json-too-large"));
        }
        let document = serde_json::from_slice::<PlanDocument>(bytes)
            .map_err(|_| load_error("invalid-plan-json"))?;
        let canonical = canonical_json(&document).map_err(plan_json_load_error)?;
        if canonical != bytes {
            return Err(load_error("non-canonical-plan-json"));
        }
        if document.plan_hash
            != hash(&canonical_json(&document.content).map_err(plan_json_load_error)?)
        {
            return Err(load_error("plan-hash-mismatch"));
        }
        if document.content.plan_schema_version != PLAN_SCHEMA_VERSION
            || document.content.catalog_version != CATALOG_VERSION
            || document.content.engine_version != ENGINE_VERSION
            || document.content.ta_lib_version != TA_LIB_VERSION
            || !is_sha256(&document.content.consumer_package_sha256)
            || document.content.slots.is_empty()
            || document.content.slots.len() > MAX_FEATURE_SLOTS
            || document.content.factors.len() > MAX_FACTOR_INSTANCES
            || document.content.effective_warmup_bars > MAX_EFFECTIVE_WARMUP_BARS
        {
            return Err(load_error("invalid-plan-contract"));
        }
        if document.content.catalog_version != identity.catalog_version
            || document.content.engine_version != identity.engine_version
            || document.content.ta_lib_version != identity.ta_lib_version
            || document.content.ta_source_sha256 != identity.ta_source_sha256
            || document.content.wrapper_sha256 != identity.wrapper_sha256
            || document.content.target_triple != identity.target_triple
            || document.content.compiler_and_flags_sha256 != identity.compiler_and_flags_sha256
            || document.content.engine_build_id != identity.engine_build_id
        {
            return Err(load_error("unsupported-engine-identity"));
        }
        let mut names = std::collections::HashSet::new();
        let mut factor_aliases = std::collections::HashSet::new();
        if document
            .content
            .slots
            .iter()
            .any(|slot| !is_lower_kebab(&slot.name) || !names.insert(&slot.name))
            || document.content.factors.iter().any(|factor| {
                factor.output_names.len() > MAX_FACTOR_OUTPUTS
                    || !is_lower_kebab(&factor.alias)
                    || !factor_aliases.insert(&factor.alias)
                    || {
                        let mut outputs = std::collections::HashSet::new();
                        factor
                            .output_names
                            .iter()
                            .any(|name| !is_lower_kebab(name) || !outputs.insert(name))
                    }
            })
        {
            return Err(load_error("invalid-plan-contract"));
        }
        if unique_builtin_request_count(&document.content.slots) > MAX_BUILTIN_REQUESTS {
            return Err(load_error("invalid-plan-contract"));
        }
        if document
            .content
            .slots
            .iter()
            .any(|slot| match &slot.source {
                FrozenSource::Market { .. } => slot.warmup_bars != 0,
                FrozenSource::External {
                    dependency_alias,
                    output,
                } => document
                    .content
                    .factors
                    .iter()
                    .find(|factor| factor.alias == *dependency_alias)
                    .is_none_or(|factor| {
                        factor.warmup_bars != slot.warmup_bars
                            || !factor.output_names.iter().any(|name| name == output)
                    }),
                FrozenSource::BuiltIn {
                    indicator,
                    output,
                    real_inputs,
                    parameters,
                } => !valid_frozen_builtin(
                    indicator,
                    output,
                    real_inputs,
                    parameters,
                    slot.warmup_bars,
                ),
            })
            || document.content.effective_warmup_bars
                != document
                    .content
                    .slots
                    .iter()
                    .map(|slot| slot.warmup_bars)
                    .max()
                    .unwrap_or(0)
        {
            return Err(load_error("invalid-plan-contract"));
        }
        Ok(Self(document))
    }
}

#[derive(Debug, Clone, Copy)]
pub enum FrozenSourceView<'a> {
    Market(MarketField),
    External {
        dependency_alias: &'a str,
        output: &'a str,
    },
    BuiltIn {
        indicator: &'a str,
        output: &'a str,
        real_inputs: &'a [MarketField],
        parameters: &'a BTreeMap<String, FrozenBuiltInParameter>,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct FrozenFactorView<'a> {
    pub alias: &'a str,
    pub parameters: &'a [ComponentParameterValue],
    pub output_names: &'a [String],
    pub warmup_bars: u32,
}

pub fn validate_and_freeze_feature_plan(
    manifest: &ComponentManifest,
    consumer_package_sha256: &str,
    identity: &EngineIdentity,
) -> Result<FrozenFeaturePlan, PlanValidationError> {
    validate_and_freeze_feature_plan_with_factors_and_parameters(
        manifest,
        consumer_package_sha256,
        identity,
        &[],
        &BTreeMap::new(),
    )
}

pub fn validate_and_freeze_feature_plan_with_factors(
    manifest: &ComponentManifest,
    consumer_package_sha256: &str,
    identity: &EngineIdentity,
    factor_inputs: &[FactorInstancePlanInput<'_>],
) -> Result<FrozenFeaturePlan, PlanValidationError> {
    validate_and_freeze_feature_plan_with_factors_and_parameters(
        manifest,
        consumer_package_sha256,
        identity,
        factor_inputs,
        &BTreeMap::new(),
    )
}

pub fn validate_and_freeze_feature_plan_with_factors_and_parameters(
    manifest: &ComponentManifest,
    consumer_package_sha256: &str,
    identity: &EngineIdentity,
    factor_inputs: &[FactorInstancePlanInput<'_>],
    consumer_parameters: &BTreeMap<String, String>,
) -> Result<FrozenFeaturePlan, PlanValidationError> {
    let mut issues = Vec::new();
    if !matches!(
        manifest.kind,
        ComponentKind::Strategy | ComponentKind::Model
    ) {
        issues.push(issue("not-a-feature-consumer", None, None, None));
    }
    if manifest.manifest_schema_version.to_string() != PLAN_SCHEMA_VERSION {
        issues.push(issue(
            "unsupported-manifest-schema",
            None,
            None,
            Some("manifest-schema-version"),
        ));
    }
    if !is_sha256(consumer_package_sha256) {
        issues.push(issue(
            "invalid-consumer-package-hash",
            None,
            None,
            Some("consumer-package-sha256"),
        ));
    }
    if !valid_engine_identity(identity) {
        issues.push(issue(
            "invalid-engine-identity",
            None,
            None,
            Some("engine-build-id"),
        ));
    }
    if manifest.feature_slots.is_empty() {
        issues.push(issue("missing-feature-slots", None, None, None));
    }
    if manifest.feature_slots.len() > MAX_FEATURE_SLOTS {
        issues.push(issue("too-many-feature-slots", None, None, None));
    }
    if consumer_parameters
        .keys()
        .any(|name| !manifest.parameters.iter().any(|item| &item.name == name))
    {
        issues.push(issue(
            "unknown-consumer-parameter",
            None,
            None,
            Some("consumer-parameters"),
        ));
    }
    let parameter_overrides = consumer_parameters
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let frozen_consumer_parameters =
        match crate::component_parameters(manifest, Some(&parameter_overrides)) {
            Ok(values) => manifest
                .parameters
                .iter()
                .zip(values)
                .map(|(definition, value)| FrozenConsumerParameter {
                    name: definition.name.clone(),
                    value,
                })
                .collect(),
            Err(_) => {
                issues.push(issue(
                    "invalid-consumer-parameter",
                    None,
                    None,
                    Some("consumer-parameters"),
                ));
                vec![]
            }
        };

    if factor_inputs.len() > MAX_FACTOR_INSTANCES {
        issues.push(issue(
            "too-many-factor-instances",
            None,
            Some("external"),
            None,
        ));
    }
    let mut factors = std::collections::HashMap::new();
    for input in factor_inputs {
        if !is_lower_kebab(input.alias) || factors.contains_key(input.alias) {
            issues.push(issue(
                "invalid-factor-alias",
                Some(input.alias),
                Some("external"),
                None,
            ));
            continue;
        }
        if input.manifest.kind != ComponentKind::Factor
            || input.manifest.output_names.len() > MAX_FACTOR_OUTPUTS
            || input
                .manifest
                .output_names
                .iter()
                .any(|name| !is_lower_kebab(name))
        {
            issues.push(issue(
                "invalid-factor-contract",
                Some(input.alias),
                Some("external"),
                None,
            ));
            continue;
        }
        factors.insert(input.alias, input);
    }
    let dependencies = manifest
        .dependencies
        .iter()
        .map(|dependency| (dependency.alias.as_str(), dependency))
        .collect::<std::collections::HashMap<_, _>>();
    for input in factor_inputs {
        match dependencies.get(input.alias) {
            Some(dependency)
                if dependency.component_id == input.manifest.component_id
                    && dependency.version.matches(&input.manifest.version) => {}
            _ => issues.push(issue(
                "unmatched-factor-dependency",
                Some(input.alias),
                Some("external"),
                None,
            )),
        }
    }
    for dependency in &manifest.dependencies {
        if !factors.contains_key(dependency.alias.as_str()) {
            issues.push(issue(
                "missing-factor-dependency",
                Some(&dependency.alias),
                Some("external"),
                None,
            ));
        }
    }
    let mut names = std::collections::HashSet::new();
    let mut slots = Vec::with_capacity(manifest.feature_slots.len().min(MAX_FEATURE_SLOTS));
    for slot in manifest.feature_slots.iter().take(MAX_FEATURE_SLOTS) {
        if !is_lower_kebab(&slot.name) {
            issues.push(issue(
                "invalid-slot-name",
                Some(&slot.name),
                None,
                Some("name"),
            ));
        } else if !names.insert(slot.name.as_str()) {
            issues.push(issue(
                "duplicate-slot-name",
                Some(&slot.name),
                None,
                Some("name"),
            ));
        }
        match &slot.source {
            FeatureSlotSource::Market { field } => slots.push(FrozenSlot {
                name: slot.name.clone(),
                source: FrozenSource::Market { field: *field },
                warmup_bars: 0,
            }),
            FeatureSlotSource::BuiltIn {
                indicator,
                output,
                inputs,
                parameters,
            } => {
                match freeze_builtin(
                    manifest,
                    indicator,
                    output,
                    inputs,
                    parameters,
                    consumer_parameters,
                ) {
                    Ok(resolved) => slots.push(FrozenSlot {
                        name: slot.name.clone(),
                        source: FrozenSource::BuiltIn {
                            indicator: indicator.clone(),
                            output: output.clone(),
                            real_inputs: resolved.real_inputs,
                            parameters: resolved.parameters,
                        },
                        warmup_bars: resolved.warmup_bars,
                    }),
                    Err((code, field)) => issues.push(issue(
                        code,
                        Some(&slot.name),
                        Some("builtin"),
                        field.as_deref(),
                    )),
                }
            }
            FeatureSlotSource::External {
                dependency_alias,
                output,
            } => match factors.get(dependency_alias.as_str()) {
                Some(factor)
                    if factor
                        .manifest
                        .output_names
                        .iter()
                        .any(|name| name == output) =>
                {
                    slots.push(FrozenSlot {
                        name: slot.name.clone(),
                        source: FrozenSource::External {
                            dependency_alias: dependency_alias.clone(),
                            output: output.clone(),
                        },
                        warmup_bars: factor.manifest.warmup_bars,
                    })
                }
                _ => issues.push(issue(
                    "invalid-factor-output",
                    Some(&slot.name),
                    Some("external"),
                    Some(output),
                )),
            },
        }
    }
    if unique_builtin_request_count(&slots) > MAX_BUILTIN_REQUESTS {
        issues.push(issue(
            "too-many-builtin-requests",
            None,
            Some("builtin"),
            None,
        ));
    }
    if slots.iter().map(|slot| slot.warmup_bars).max().unwrap_or(0) > MAX_EFFECTIVE_WARMUP_BARS {
        issues.push(issue(
            "effective-warmup-too-large",
            None,
            Some("builtin"),
            None,
        ));
    }
    issues.sort_by(|left, right| {
        (&left.code, &left.slot, &left.source, &left.field).cmp(&(
            &right.code,
            &right.slot,
            &right.source,
            &right.field,
        ))
    });
    if !issues.is_empty() {
        return Err(PlanValidationError { issues });
    }

    let content = PlanContent {
        plan_schema_version: PLAN_SCHEMA_VERSION.into(),
        consumer_package_sha256: consumer_package_sha256.into(),
        catalog_version: CATALOG_VERSION.into(),
        engine_version: ENGINE_VERSION.into(),
        ta_lib_version: TA_LIB_VERSION.into(),
        ta_source_sha256: identity.ta_source_sha256.clone(),
        wrapper_sha256: identity.wrapper_sha256.clone(),
        target_triple: identity.target_triple.clone(),
        compiler_and_flags_sha256: identity.compiler_and_flags_sha256.clone(),
        engine_build_id: identity.engine_build_id.clone(),
        consumer_parameters: frozen_consumer_parameters,
        consumer_warmup_bars: manifest.warmup_bars,
        effective_warmup_bars: slots.iter().map(|slot| slot.warmup_bars).max().unwrap_or(0),
        slots,
        factors: {
            let mut frozen = factor_inputs
                .iter()
                .filter_map(|input| {
                    factors.get(input.alias).map(|factor| FrozenFactor {
                        alias: input.alias.to_owned(),
                        parameters: factor.parameters.clone(),
                        output_names: factor.manifest.output_names.clone(),
                        warmup_bars: factor.manifest.warmup_bars,
                    })
                })
                .collect::<Vec<_>>();
            frozen.sort_by(|left, right| left.alias.cmp(&right.alias));
            frozen
        },
    };
    let content_json = match canonical_json(&content) {
        Ok(bytes) => bytes,
        Err(PlanJsonError::TooLarge) => {
            return Err(PlanValidationError {
                issues: vec![issue("plan-json-too-large", None, None, None)],
            });
        }
        Err(PlanJsonError::Serialization) => {
            unreachable!("validated Plan content is serializable")
        }
    };
    let plan_hash = hash(&content_json);
    let document = PlanDocument { content, plan_hash };
    if matches!(canonical_json(&document), Err(PlanJsonError::TooLarge)) {
        return Err(PlanValidationError {
            issues: vec![issue("plan-json-too-large", None, None, None)],
        });
    }
    Ok(FrozenFeaturePlan(document))
}

fn valid_engine_identity(identity: &EngineIdentity) -> bool {
    identity.engine_version == ENGINE_VERSION
        && identity.ta_lib_version == TA_LIB_VERSION
        && identity.catalog_version == CATALOG_VERSION
        && is_sha256(&identity.ta_source_sha256)
        && is_sha256(&identity.wrapper_sha256)
        && is_sha256(&identity.compiler_and_flags_sha256)
        && is_sha256(&identity.engine_build_id)
        && !identity.target_triple.is_empty()
}

#[derive(Debug)]
enum PlanJsonError {
    Serialization,
    TooLarge,
}

struct PlanJsonWriter {
    bytes: Vec<u8>,
    too_large: bool,
}

impl Write for PlanJsonWriter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        if buffer.len() > MAX_CANONICAL_PLAN_JSON_BYTES.saturating_sub(self.bytes.len()) {
            self.too_large = true;
            return Err(std::io::Error::other(
                "canonical Plan JSON exceeds the size limit",
            ));
        }
        self.bytes.extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn canonical_json(value: &impl Serialize) -> Result<Vec<u8>, PlanJsonError> {
    let mut writer = PlanJsonWriter {
        bytes: Vec::new(),
        too_large: false,
    };
    let result = serde_json::to_writer(&mut writer, value);
    if writer.too_large {
        Err(PlanJsonError::TooLarge)
    } else {
        result.map_err(|_| PlanJsonError::Serialization)?;
        Ok(writer.bytes)
    }
}

fn plan_json_load_error(error: PlanJsonError) -> PlanLoadError {
    match error {
        PlanJsonError::TooLarge => load_error("plan-json-too-large"),
        PlanJsonError::Serialization => load_error("invalid-plan-json"),
    }
}

fn hash(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn issue(code: &str, slot: Option<&str>, source: Option<&str>, field: Option<&str>) -> PlanIssue {
    PlanIssue {
        code: code.into(),
        slot: slot.map(str::to_owned),
        source: source.map(str::to_owned),
        field: field.map(str::to_owned),
    }
}

fn load_error(code: &str) -> PlanLoadError {
    PlanLoadError { code: code.into() }
}

fn unique_builtin_request_count(slots: &[FrozenSlot]) -> usize {
    slots
        .iter()
        .filter_map(|slot| match &slot.source {
            FrozenSource::BuiltIn {
                indicator,
                real_inputs,
                parameters,
                ..
            } => Some(format!("{indicator}:{real_inputs:?}:{parameters:?}")),
            _ => None,
        })
        .collect::<std::collections::BTreeSet<_>>()
        .len()
}

fn freeze_builtin(
    manifest: &ComponentManifest,
    indicator: &str,
    output: &str,
    inputs: &BTreeMap<String, serde_json::Value>,
    bindings: &BTreeMap<String, serde_json::Value>,
    strategy_parameters: &BTreeMap<String, String>,
) -> Result<ResolvedBuiltIn, (&'static str, Option<String>)> {
    let engine = IndicatorEngine::initialize().map_err(|error| (error.code(), None))?;
    let definition = engine
        .catalog()
        .indicators
        .iter()
        .find(|item| item.id == indicator)
        .ok_or(("unknown-indicator", Some(indicator.into())))?;
    if !definition.outputs.iter().any(|item| item.id == output) {
        return Err(("unknown-indicator-output", Some(output.into())));
    }
    let real_inputs = definition
        .inputs
        .iter()
        .filter(|item| item.kind == "Double Array" || item.kind == "Volume")
        .map(|input| {
            let field = inputs
                .get(&input.id)
                .ok_or(("missing-indicator-input", Some(input.id.clone())))?
                .as_str()
                .and_then(parse_market_field)
                .ok_or(("invalid-indicator-input", Some(input.id.clone())))?;
            let allowed = input
                .allowed_fields
                .iter()
                .any(|value| value == field_name(field));
            if !allowed {
                return Err(("invalid-indicator-input", Some(input.id.clone())));
            }
            Ok(field)
        })
        .collect::<Result<Vec<_>, _>>()?;
    if inputs.len() != real_inputs.len()
        || inputs
            .keys()
            .any(|id| !definition.inputs.iter().any(|input| &input.id == id))
    {
        return Err(("invalid-indicator-input", None));
    }
    let mut frozen = BTreeMap::new();
    let mut engine_parameters = BTreeMap::new();
    for parameter in &definition.parameters {
        let binding = bindings.get(&parameter.id);
        let (raw, is_default, is_reference) = match binding {
            Some(value) if value.is_object() => {
                let object = value.as_object().expect("checked object");
                if object.len() != 1 {
                    return Err(("invalid-indicator-parameter", Some(parameter.id.clone())));
                }
                let strategy_parameter = object
                    .get("strategyParameter")
                    .and_then(serde_json::Value::as_str)
                    .ok_or(("invalid-indicator-parameter", Some(parameter.id.clone())))?;
                let declared = manifest
                    .parameters
                    .iter()
                    .find(|item| item.name == strategy_parameter)
                    .ok_or((
                        "unknown-strategy-parameter",
                        Some(strategy_parameter.into()),
                    ))?;
                let value = strategy_parameters
                    .get(strategy_parameter)
                    .unwrap_or(&declared.default_value);
                (
                    (declared.parameter_type.clone(), value.clone()),
                    false,
                    true,
                )
            }
            Some(value) if value.is_i64() => {
                ((ParameterType::Integer, value.to_string()), false, false)
            }
            Some(value) if value.is_string() => (
                (ParameterType::String, value.as_str().unwrap().into()),
                false,
                false,
            ),
            Some(_) => return Err(("invalid-indicator-parameter", Some(parameter.id.clone()))),
            None => (
                (
                    ParameterType::String,
                    if parameter.kind == "MA Type" {
                        parameter
                            .default
                            .parse::<i32>()
                            .ok()
                            .and_then(|value| {
                                parameter
                                    .enum_values
                                    .iter()
                                    .find(|item| item.value == value)
                                    .map(|item| item.id.clone())
                            })
                            .ok_or(("invalid-indicator-parameter", Some(parameter.id.clone())))?
                    } else {
                        parameter.default.clone()
                    },
                ),
                true,
                false,
            ),
        };
        let value = match parameter.kind.as_str() {
            "Integer" => {
                if !matches!(raw.0, ParameterType::Integer) && !is_default {
                    return Err(("mistyped-indicator-parameter", Some(parameter.id.clone())));
                }
                let value = raw
                    .1
                    .parse::<i32>()
                    .map_err(|_| ("invalid-indicator-parameter", Some(parameter.id.clone())))?;
                FrozenBuiltInParameter::Integer(value)
            }
            "Real" | "Double" => {
                if (is_reference && !matches!(raw.0, ParameterType::Decimal))
                    || (!is_reference
                        && !matches!(raw.0, ParameterType::Decimal | ParameterType::String)
                        && !is_default)
                {
                    return Err(("mistyped-indicator-parameter", Some(parameter.id.clone())));
                }
                let decimal = if is_default {
                    rust_decimal::Decimal::from_str_exact(&raw.1)
                        .or_else(|_| rust_decimal::Decimal::from_scientific(&raw.1))
                } else {
                    rust_decimal::Decimal::from_str_exact(&raw.1)
                }
                .map_err(|_| ("invalid-indicator-parameter", Some(parameter.id.clone())))?;
                let value: f64 = decimal
                    .to_string()
                    .parse()
                    .map_err(|_| ("invalid-indicator-parameter", Some(parameter.id.clone())))?;
                if !value.is_finite() {
                    return Err(("invalid-indicator-parameter", Some(parameter.id.clone())));
                }
                FrozenBuiltInParameter::Real(decimal.normalize().to_string())
            }
            "MA Type" => {
                if !matches!(raw.0, ParameterType::String) {
                    return Err(("mistyped-indicator-parameter", Some(parameter.id.clone())));
                }
                FrozenBuiltInParameter::Enum(raw.1)
            }
            _ => return Err(("invalid-indicator-parameter", Some(parameter.id.clone()))),
        };
        engine_parameters.insert(parameter.id.clone(), frozen_parameter_value(&value)?);
        frozen.insert(parameter.id.clone(), value);
    }
    if bindings.keys().any(|id| {
        !definition
            .parameters
            .iter()
            .any(|parameter| &parameter.id == id)
    }) {
        return Err(("unknown-indicator-parameter", None));
    }
    let compiled = engine
        .compile(IndicatorRequest {
            indicator_id: indicator.into(),
            real_inputs: real_inputs
                .iter()
                .map(|field| builtin_engine_market_field(*field))
                .collect(),
            parameters: engine_parameters,
            outputs: vec![output.into()],
        })
        .map_err(|error| (error.code(), None))?;
    Ok(ResolvedBuiltIn {
        real_inputs,
        parameters: frozen,
        warmup_bars: compiled
            .lookback()
            .try_into()
            .map_err(|_| ("invalid-indicator-lookback", None))?,
    })
}

fn frozen_parameter_value(
    value: &FrozenBuiltInParameter,
) -> Result<ParameterValue, (&'static str, Option<String>)> {
    match value {
        FrozenBuiltInParameter::Integer(value) => Ok(ParameterValue::Integer(*value)),
        FrozenBuiltInParameter::Real(value) => value
            .parse()
            .map(ParameterValue::Real)
            .map_err(|_| ("invalid-indicator-parameter", None)),
        FrozenBuiltInParameter::Enum(value) => Ok(ParameterValue::Enum(value.clone())),
    }
}

fn valid_frozen_builtin(
    indicator: &str,
    output: &str,
    real_inputs: &[MarketField],
    parameters: &BTreeMap<String, FrozenBuiltInParameter>,
    warmup_bars: u32,
) -> bool {
    let Ok(engine) = IndicatorEngine::initialize() else {
        return false;
    };
    let Some(definition) = engine
        .catalog()
        .indicators
        .iter()
        .find(|definition| definition.id == indicator)
    else {
        return false;
    };
    if parameters.len() != definition.parameters.len()
        || definition
            .parameters
            .iter()
            .any(|parameter| !parameters.contains_key(&parameter.id))
    {
        return false;
    }
    let Ok(parameters) = parameters
        .iter()
        .map(|(id, value)| frozen_parameter_value(value).map(|value| (id.clone(), value)))
        .collect::<Result<BTreeMap<_, _>, _>>()
    else {
        return false;
    };
    engine
        .compile(IndicatorRequest {
            indicator_id: indicator.into(),
            real_inputs: real_inputs
                .iter()
                .map(|field| builtin_engine_market_field(*field))
                .collect(),
            parameters,
            outputs: vec![output.into()],
        })
        .is_ok_and(|compiled| compiled.lookback() == warmup_bars as usize)
}

fn field_name(field: MarketField) -> &'static str {
    match field {
        MarketField::Open => "open",
        MarketField::High => "high",
        MarketField::Low => "low",
        MarketField::Close => "close",
        MarketField::BaseVolume => "base-volume",
        MarketField::QuoteVolume => "quote-volume",
    }
}
fn parse_market_field(value: &str) -> Option<MarketField> {
    Some(match value {
        "open" => MarketField::Open,
        "high" => MarketField::High,
        "low" => MarketField::Low,
        "close" => MarketField::Close,
        "base-volume" => MarketField::BaseVolume,
        "quote-volume" => MarketField::QuoteVolume,
        _ => return None,
    })
}
pub fn builtin_engine_market_field(field: MarketField) -> EngineMarketField {
    match field {
        MarketField::Open => EngineMarketField::Open,
        MarketField::High => EngineMarketField::High,
        MarketField::Low => EngineMarketField::Low,
        MarketField::Close => EngineMarketField::Close,
        MarketField::BaseVolume => EngineMarketField::BaseVolume,
        MarketField::QuoteVolume => EngineMarketField::QuoteVolume,
    }
}

#[cfg(test)]
mod tests {
    use semver::Version;
    use uuid::Uuid;

    use super::*;
    use crate::{ComponentKind, FeatureSlotDefinition, FeatureSlotSource, ParameterDefinition};

    fn manifest(slots: Vec<FeatureSlotDefinition>) -> ComponentManifest {
        ComponentManifest {
            manifest_schema_version: Version::new(1, 0, 0),
            component_id: Uuid::nil(),
            version: Version::new(1, 0, 0),
            name: "Market Strategy".into(),
            kind: ComponentKind::Strategy,
            sdk_version: Version::new(0, 1, 0),
            abi_version: Version::new(1, 0, 0),
            wasm_sha256: "wasm".into(),
            parameters: Vec::<ParameterDefinition>::new(),
            feature_slots: slots,
            output_names: vec![],
            dependencies: vec![],
            warmup_bars: 0,
            model_scope: None,
            model_outputs: vec![],
            model_artifact: None,
        }
    }

    fn market(name: &str, field: MarketField) -> FeatureSlotDefinition {
        FeatureSlotDefinition {
            name: name.into(),
            source: FeatureSlotSource::Market { field },
        }
    }

    fn identity() -> EngineIdentity {
        native_engine_identity().unwrap()
    }

    #[test]
    fn freezes_and_loads_a_canonical_market_plan_in_manifest_order() {
        let manifest = manifest(vec![
            market("quote-volume", MarketField::QuoteVolume),
            market("close", MarketField::Close),
        ]);
        let identity = identity();
        let package_hash = "a".repeat(64);

        let first = validate_and_freeze_feature_plan(&manifest, &package_hash, &identity).unwrap();
        let replay = validate_and_freeze_feature_plan(&manifest, &package_hash, &identity).unwrap();

        assert_eq!(first.plan_hash(), replay.plan_hash());
        assert!(is_sha256(first.plan_hash()));
        assert_eq!(
            first.slot_names().collect::<Vec<_>>(),
            ["quote-volume", "close"]
        );
        assert_eq!(
            first.market_fields().collect::<Vec<_>>(),
            [MarketField::QuoteVolume, MarketField::Close]
        );
        assert_eq!(first.effective_warmup_bars(), 0);
        assert_eq!(FrozenFeaturePlan::load(&first.to_json()).unwrap(), first);

        let padded = [b" ".as_slice(), first.to_json().as_slice()].concat();
        assert_eq!(
            FrozenFeaturePlan::load(&padded).unwrap_err().code,
            "non-canonical-plan-json"
        );

        let mut tampered = String::from_utf8(first.to_json()).unwrap();
        tampered = tampered.replace("quote-volume", "base-volume");
        assert_eq!(
            FrozenFeaturePlan::load(tampered.as_bytes())
                .unwrap_err()
                .code,
            "plan-hash-mismatch"
        );

        let mut oversized: serde_json::Value = serde_json::from_slice(&first.to_json()).unwrap();
        oversized["consumerPackageSha256"] =
            serde_json::json!("a".repeat(MAX_CANONICAL_PLAN_JSON_BYTES));
        assert_eq!(
            FrozenFeaturePlan::load(&serde_json::to_vec(&oversized).unwrap())
                .unwrap_err()
                .code,
            "plan-json-too-large"
        );
    }

    #[test]
    fn reports_unbound_external_sources_as_deterministically_ordered_typed_issues() {
        let manifest = manifest(vec![
            FeatureSlotDefinition {
                name: "z-factor".into(),
                source: FeatureSlotSource::External {
                    dependency_alias: "trend".into(),
                    output: "value".into(),
                },
            },
            FeatureSlotDefinition {
                name: "a-rsi".into(),
                source: FeatureSlotSource::BuiltIn {
                    indicator: "rsi".into(),
                    output: "value".into(),
                    inputs: [("real-0".into(), serde_json::json!("close"))].into(),
                    parameters: [("time-period".into(), serde_json::json!(2))].into(),
                },
            },
        ]);
        let error =
            validate_and_freeze_feature_plan(&manifest, &"a".repeat(64), &identity()).unwrap_err();
        assert_eq!(
            error.issues,
            [issue(
                "invalid-factor-output",
                Some("z-factor"),
                Some("external"),
                Some("value")
            )]
        );
    }

    #[test]
    fn freezes_catalog_builtin_parameters_and_warmup() {
        let mut manifest = manifest(vec![FeatureSlotDefinition {
            name: "ema".into(),
            source: FeatureSlotSource::BuiltIn {
                indicator: "ema".into(),
                output: "value".into(),
                inputs: [("real-0".into(), serde_json::json!("close"))].into(),
                parameters: [(
                    "time-period".into(),
                    serde_json::json!({"strategyParameter":"period"}),
                )]
                .into(),
            },
        }]);
        manifest.parameters = vec![ParameterDefinition {
            name: "period".into(),
            parameter_type: ParameterType::Integer,
            default_value: "2".into(),
            allowed_values: vec![],
        }];
        let plan = validate_and_freeze_feature_plan_with_factors_and_parameters(
            &manifest,
            &"a".repeat(64),
            &identity(),
            &[],
            &[("period".into(), "3".into())].into(),
        )
        .unwrap();
        assert_eq!(plan.effective_warmup_bars(), 2);
        assert_eq!(plan.slot_names().collect::<Vec<_>>(), ["ema"]);
    }

    #[test]
    fn model_feature_plan_freezes_selected_parameter_values() {
        let mut manifest = manifest(vec![FeatureSlotDefinition {
            name: "ema".into(),
            source: FeatureSlotSource::BuiltIn {
                indicator: "ema".into(),
                output: "value".into(),
                inputs: [("real-0".into(), serde_json::json!("close"))].into(),
                parameters: [(
                    "time-period".into(),
                    serde_json::json!({"strategyParameter":"period"}),
                )]
                .into(),
            },
        }]);
        manifest.kind = ComponentKind::Model;
        manifest.parameters = vec![ParameterDefinition {
            name: "period".into(),
            parameter_type: ParameterType::Integer,
            default_value: "2".into(),
            allowed_values: vec![],
        }];
        let selected = validate_and_freeze_feature_plan_with_factors_and_parameters(
            &manifest,
            &"a".repeat(64),
            &identity(),
            &[],
            &[("period".into(), "3".into())].into(),
        )
        .unwrap();
        let default =
            validate_and_freeze_feature_plan(&manifest, &"a".repeat(64), &identity()).unwrap();
        assert_eq!(selected.effective_warmup_bars(), 2);
        assert_ne!(selected.plan_hash(), default.plan_hash());
    }

    #[test]
    fn unknown_builtin_output_returns_a_typed_plan_issue() {
        let manifest = manifest(vec![FeatureSlotDefinition {
            name: "invalid".into(),
            source: FeatureSlotSource::BuiltIn {
                indicator: "rsi".into(),
                output: "unknown".into(),
                inputs: [("real-0".into(), serde_json::json!("close"))].into(),
                parameters: BTreeMap::new(),
            },
        }]);
        let error =
            validate_and_freeze_feature_plan(&manifest, &"a".repeat(64), &identity()).unwrap_err();
        assert_eq!(
            error.issues,
            [issue(
                "unknown-indicator-output",
                Some("invalid"),
                Some("builtin"),
                Some("unknown")
            )]
        );
    }

    #[test]
    fn loading_revalidates_builtin_sources_against_the_catalog() {
        let manifest = manifest(vec![FeatureSlotDefinition {
            name: "rsi".into(),
            source: FeatureSlotSource::BuiltIn {
                indicator: "rsi".into(),
                output: "value".into(),
                inputs: [("real-0".into(), serde_json::json!("close"))].into(),
                parameters: [("time-period".into(), serde_json::json!(2))].into(),
            },
        }]);
        let plan =
            validate_and_freeze_feature_plan(&manifest, &"a".repeat(64), &identity()).unwrap();
        let mut document = plan.0;
        let FrozenSource::BuiltIn { indicator, .. } = &mut document.content.slots[0].source else {
            unreachable!()
        };
        *indicator = "not-in-catalog".into();
        document.plan_hash = hash(&canonical_json(&document.content).unwrap());

        assert_eq!(
            FrozenFeaturePlan::load(&canonical_json(&document).unwrap())
                .unwrap_err()
                .code,
            "invalid-plan-contract"
        );
    }

    #[test]
    fn loading_rejects_a_plan_for_a_different_engine_build() {
        let plan = validate_and_freeze_feature_plan(
            &manifest(vec![market("close", MarketField::Close)]),
            &"a".repeat(64),
            &identity(),
        )
        .unwrap();
        let mut other = identity();
        other.engine_build_id = "b".repeat(64);
        assert_eq!(
            FrozenFeaturePlan::load_for_engine(&plan.to_json(), &other)
                .unwrap_err()
                .code,
            "unsupported-engine-identity"
        );
        for field in ["catalog", "engine", "ta-lib"] {
            let mut other = identity();
            match field {
                "catalog" => other.catalog_version = "other-catalog".into(),
                "engine" => other.engine_version = "other-engine".into(),
                "ta-lib" => other.ta_lib_version = "other-ta-lib".into(),
                _ => unreachable!(),
            }
            assert_eq!(
                FrozenFeaturePlan::load_for_engine(&plan.to_json(), &other)
                    .unwrap_err()
                    .code,
                "unsupported-engine-identity"
            );
        }
    }

    #[test]
    fn builtin_defaults_and_equivalent_real_literals_freeze_identically() {
        let make = |parameters| {
            manifest(vec![FeatureSlotDefinition {
                name: "upper-band".into(),
                source: FeatureSlotSource::BuiltIn {
                    indicator: "bbands".into(),
                    output: "upper-band".into(),
                    inputs: [("real-0".into(), serde_json::json!("close"))].into(),
                    parameters,
                },
            }])
        };
        let identity = identity();
        let omitted =
            validate_and_freeze_feature_plan(&make(BTreeMap::new()), &"a".repeat(64), &identity)
                .unwrap();
        let explicit_defaults = validate_and_freeze_feature_plan(
            &make(
                [
                    ("time-period".into(), serde_json::json!(5)),
                    ("deviations-up".into(), serde_json::json!("2.0")),
                    ("deviations-down".into(), serde_json::json!("2.00")),
                    ("ma-type".into(), serde_json::json!("sma")),
                ]
                .into(),
            ),
            &"a".repeat(64),
            &identity,
        )
        .unwrap();
        let equivalent_spelling = validate_and_freeze_feature_plan(
            &make(
                [
                    ("time-period".into(), serde_json::json!(5)),
                    ("deviations-up".into(), serde_json::json!("2.00")),
                    ("deviations-down".into(), serde_json::json!("2.0")),
                    ("ma-type".into(), serde_json::json!("sma")),
                ]
                .into(),
            ),
            &"a".repeat(64),
            &identity,
        )
        .unwrap();

        assert_eq!(omitted.plan_hash(), explicit_defaults.plan_hash());
        assert_eq!(
            explicit_defaults.plan_hash(),
            equivalent_spelling.plan_hash()
        );
    }

    #[test]
    fn every_invalid_builtin_binding_shape_reports_a_stable_plan_issue() {
        let slot =
            |name: &str,
             indicator: &str,
             output: &str,
             inputs: BTreeMap<String, serde_json::Value>,
             parameters: BTreeMap<String, serde_json::Value>| FeatureSlotDefinition {
                name: name.into(),
                source: FeatureSlotSource::BuiltIn {
                    indicator: indicator.into(),
                    output: output.into(),
                    inputs,
                    parameters,
                },
            };
        let close = || [("real-0".into(), serde_json::json!("close"))].into();
        let slots = vec![
            slot(
                "unknown-indicator",
                "missing",
                "value",
                close(),
                BTreeMap::new(),
            ),
            slot("unknown-output", "rsi", "missing", close(), BTreeMap::new()),
            slot(
                "array-input",
                "rsi",
                "value",
                [("real-0".into(), serde_json::json!(["close"]))].into(),
                BTreeMap::new(),
            ),
            slot(
                "null-parameter",
                "rsi",
                "value",
                close(),
                [("time-period".into(), serde_json::Value::Null)].into(),
            ),
            slot(
                "extra-parameter",
                "rsi",
                "value",
                close(),
                [
                    ("time-period".into(), serde_json::json!(2)),
                    ("extra".into(), serde_json::json!(1)),
                ]
                .into(),
            ),
            slot(
                "mistyped-parameter",
                "rsi",
                "value",
                close(),
                [("time-period".into(), serde_json::json!("2"))].into(),
            ),
            slot(
                "range-parameter",
                "rsi",
                "value",
                close(),
                [("time-period".into(), serde_json::json!(1))].into(),
            ),
            slot(
                "conditional-parameter",
                "rsi",
                "value",
                close(),
                [(
                    "time-period".into(),
                    serde_json::json!({"if":true,"then":2}),
                )]
                .into(),
            ),
            slot(
                "expression-parameter",
                "rsi",
                "value",
                close(),
                [(
                    "time-period".into(),
                    serde_json::json!({"expression":"1+1"}),
                )]
                .into(),
            ),
            slot(
                "slot-parameter",
                "rsi",
                "value",
                close(),
                [("time-period".into(), serde_json::json!({"slot":"other"}))].into(),
            ),
            slot(
                "unknown-reference",
                "rsi",
                "value",
                close(),
                [(
                    "time-period".into(),
                    serde_json::json!({"strategyParameter":"missing"}),
                )]
                .into(),
            ),
        ];
        let manifest = manifest(slots);
        let freeze = || {
            validate_and_freeze_feature_plan(&manifest, &"a".repeat(64), &identity()).unwrap_err()
        };
        let first = freeze();
        let replay = freeze();
        assert_eq!(first, replay);
        assert_eq!(first.issues.len(), 11);
        let by_slot = first
            .issues
            .iter()
            .map(|item| (item.slot.as_deref().unwrap(), item.code.as_str()))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(by_slot["unknown-indicator"], "unknown-indicator");
        assert_eq!(by_slot["unknown-output"], "unknown-indicator-output");
        assert_eq!(by_slot["array-input"], "invalid-indicator-input");
        assert_eq!(
            by_slot["mistyped-parameter"],
            "mistyped-indicator-parameter"
        );
        assert_eq!(by_slot["range-parameter"], "invalid-indicator-parameter");
        assert_eq!(by_slot["extra-parameter"], "unknown-indicator-parameter");
        assert_eq!(by_slot["unknown-reference"], "unknown-strategy-parameter");
        for name in [
            "null-parameter",
            "conditional-parameter",
            "expression-parameter",
            "slot-parameter",
        ] {
            assert_eq!(by_slot[name], "invalid-indicator-parameter");
        }
    }

    #[test]
    fn fixed_generic_and_explicit_volume_inputs_freeze_only_catalog_roles() {
        let valid = manifest(vec![
            FeatureSlotDefinition {
                name: "average-price".into(),
                source: FeatureSlotSource::BuiltIn {
                    indicator: "avgprice".into(),
                    output: "value".into(),
                    inputs: BTreeMap::new(),
                    parameters: BTreeMap::new(),
                },
            },
            FeatureSlotDefinition {
                name: "obv".into(),
                source: FeatureSlotSource::BuiltIn {
                    indicator: "obv".into(),
                    output: "value".into(),
                    inputs: [
                        ("real-0".into(), serde_json::json!("close")),
                        ("volume".into(), serde_json::json!("quote-volume")),
                    ]
                    .into(),
                    parameters: BTreeMap::new(),
                },
            },
        ]);
        assert!(validate_and_freeze_feature_plan(&valid, &"a".repeat(64), &identity()).is_ok());

        let invalid = manifest(vec![FeatureSlotDefinition {
            name: "obv".into(),
            source: FeatureSlotSource::BuiltIn {
                indicator: "obv".into(),
                output: "value".into(),
                inputs: [
                    ("real-0".into(), serde_json::json!("close")),
                    ("volume".into(), serde_json::json!("close")),
                ]
                .into(),
                parameters: BTreeMap::new(),
            },
        }]);
        let error =
            validate_and_freeze_feature_plan(&invalid, &"a".repeat(64), &identity()).unwrap_err();
        assert_eq!(error.issues[0].code, "invalid-indicator-input");
        assert_eq!(error.issues[0].field.as_deref(), Some("volume"));
    }

    #[test]
    fn real_parameter_references_require_decimal_strategy_parameters() {
        let mut manifest = manifest(vec![FeatureSlotDefinition {
            name: "upper-band".into(),
            source: FeatureSlotSource::BuiltIn {
                indicator: "bbands".into(),
                output: "upper-band".into(),
                inputs: [("real-0".into(), serde_json::json!("close"))].into(),
                parameters: [(
                    "deviations-up".into(),
                    serde_json::json!({"strategyParameter":"deviation"}),
                )]
                .into(),
            },
        }]);
        manifest.parameters = vec![ParameterDefinition {
            name: "deviation".into(),
            parameter_type: ParameterType::String,
            default_value: "2.0".into(),
            allowed_values: vec![],
        }];
        let error =
            validate_and_freeze_feature_plan(&manifest, &"a".repeat(64), &identity()).unwrap_err();
        assert_eq!(error.issues[0].code, "mistyped-indicator-parameter");
        assert_eq!(error.issues[0].field.as_deref(), Some("deviations-up"));
    }

    #[test]
    fn plan_limits_are_rejected_before_freezing_allocations() {
        let slots = (0..=MAX_FEATURE_SLOTS)
            .map(|index| market(&format!("slot-{index}"), MarketField::Close))
            .collect();
        let error =
            validate_and_freeze_feature_plan(&manifest(slots), &"a".repeat(64), &identity())
                .unwrap_err();
        assert!(
            error
                .issues
                .iter()
                .any(|issue| issue.code == "too-many-feature-slots")
        );
    }
}
