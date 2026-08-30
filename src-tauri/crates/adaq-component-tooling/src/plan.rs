use std::collections::{BTreeMap, BTreeSet, HashMap};

use adaq_feature_engine::{
    FeatureEngineIdentity, FeatureFactor, FeaturePlan, FeaturePlanDraft, FeatureSlot,
    FeatureSource, MarketField,
};
use adaq_indicator_engine::{
    EngineIdentity as NativeEngineIdentity, IndicatorEngine, IndicatorRequest,
    MarketField as EngineMarketField, ParameterValue,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::package::is_lower_kebab;
use crate::{
    ComponentKind, ComponentManifest, ComponentParameterValue, FeatureSlotSource, ModelOutput,
    ParameterType, StrategyArchitecture,
};

pub use adaq_feature_engine::FrozenBuiltInParameter;

const COMPONENT_MANIFEST_SCHEMA_VERSION: &str = "1.0.0";
const CATALOG_VERSION: &str = "adaq-indicator-catalog@1.0.0";
const ENGINE_VERSION: &str = "adaq-indicator-engine@1.0.0";
const C_TA_LIB_VERSION: &str = "0.7.1";
const ADAQ_TA_LIB_VERSION: &str = "0.1.9";
const MAX_FACTOR_INSTANCES: usize = 64;
const MAX_FACTOR_OUTPUTS: usize = 64;
const MAX_FEATURE_SLOTS: usize = 64;
const MAX_BUILTIN_REQUESTS: usize = 256;
const MAX_EFFECTIVE_WARMUP_BARS: u32 = 100_000;

#[derive(Debug, Clone)]
pub struct FactorInstancePlanInput<'a> {
    pub alias: &'a str,
    pub manifest: &'a ComponentManifest,
    pub parameters: Vec<ComponentParameterValue>,
}

#[derive(Debug, Clone)]
pub struct SignalPlanInput<'a> {
    pub slot_name: &'a str,
    pub dataset_id: &'a str,
    pub signal_name: &'a str,
    pub snapshot_id: &'a str,
    pub instrument_id: String,
    pub venue: &'a str,
    pub bar_interval: &'a str,
    pub contract: ModelOutput,
    pub producer_segments: Vec<Value>,
    pub artifact_provenance: Value,
    pub evidence_state: &'a str,
    pub component_lock: Vec<Value>,
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

impl From<&EngineIdentity> for FeatureEngineIdentity {
    fn from(identity: &EngineIdentity) -> Self {
        Self::from_indicator_fields(
            identity.engine_version.clone(),
            identity.catalog_version.clone(),
            identity.ta_lib_version.clone(),
            identity.ta_source_sha256.clone(),
            identity.wrapper_sha256.clone(),
            identity.target_triple.clone(),
            identity.compiler_and_flags_sha256.clone(),
            identity.engine_build_id.clone(),
        )
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

#[derive(Debug, Clone, PartialEq)]
pub struct FrozenFeaturePlan {
    inner: FeaturePlan,
    factor_parameters: Vec<Vec<ComponentParameterValue>>,
}

impl FrozenFeaturePlan {
    pub fn plan_hash(&self) -> &str {
        self.inner.plan_hash()
    }

    pub fn feature_plan(&self) -> &FeaturePlan {
        &self.inner
    }

    pub fn slot_names(&self) -> impl ExactSizeIterator<Item = &str> {
        self.inner.slot_names()
    }

    pub fn market_fields(&self) -> impl ExactSizeIterator<Item = MarketField> + '_ {
        self.inner.slots().iter().map(|slot| match slot.source {
            FeatureSource::Market { field } => field,
            FeatureSource::External { .. }
            | FeatureSource::BuiltIn { .. }
            | FeatureSource::Signal { .. } => panic!("external Feature Slot is not a Market Field"),
        })
    }

    pub fn sources(&self) -> impl ExactSizeIterator<Item = FrozenSourceView<'_>> {
        self.inner.slots().iter().map(|slot| match &slot.source {
            FeatureSource::Market { field } => FrozenSourceView::Market(*field),
            FeatureSource::External {
                dependency_alias,
                output,
            } => FrozenSourceView::External {
                dependency_alias,
                output,
            },
            FeatureSource::BuiltIn {
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
            FeatureSource::Signal {
                dataset_id,
                signal_name,
                ..
            } => FrozenSourceView::Signal {
                dataset_id,
                signal_name,
            },
        })
    }

    pub fn architecture(&self) -> StrategyArchitecture {
        let signals = self
            .inner
            .slots()
            .iter()
            .filter(|slot| matches!(slot.source, FeatureSource::Signal { .. }))
            .count();
        match (signals, self.inner.slots().len()) {
            (0, _) => StrategyArchitecture::Composed,
            (signals, total) if signals == total => StrategyArchitecture::SignalDriven,
            _ => StrategyArchitecture::Hybrid,
        }
    }

    pub fn factors(&self) -> impl ExactSizeIterator<Item = FrozenFactorView<'_>> {
        self.inner
            .factors()
            .iter()
            .zip(self.factor_parameters.iter())
            .map(|(factor, parameters)| FrozenFactorView {
                alias: &factor.alias,
                feature_slots: &factor.feature_slots,
                parameters,
                output_names: &factor.output_names,
                warmup_bars: factor.warmup_bars,
            })
    }

    pub fn effective_warmup_bars(&self) -> u32 {
        self.inner.effective_warmup_bars()
    }

    pub fn to_json(&self) -> Vec<u8> {
        self.inner.to_json()
    }

    pub fn load(bytes: &[u8]) -> Result<Self, PlanLoadError> {
        let identity =
            native_engine_identity().map_err(|_| load_error("unsupported-engine-identity"))?;
        Self::load_for_engine(bytes, &identity)
    }

    pub fn load_for_engine(bytes: &[u8], identity: &EngineIdentity) -> Result<Self, PlanLoadError> {
        let inner = FeaturePlan::load_for_engine(bytes, &identity.into())
            .map_err(|error| load_error(error.code()))?;
        let factor_parameters = deserialize_factor_parameters(&inner)?;
        Ok(Self {
            inner,
            factor_parameters,
        })
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
    Signal {
        dataset_id: &'a str,
        signal_name: &'a str,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct FrozenFactorView<'a> {
    pub alias: &'a str,
    pub feature_slots: &'a [String],
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
    validate_and_freeze_feature_plan_with_bindings_and_parameters(
        manifest,
        consumer_package_sha256,
        identity,
        factor_inputs,
        consumer_parameters,
        &[],
    )
}

pub fn validate_and_freeze_feature_plan_with_bindings_and_parameters(
    manifest: &ComponentManifest,
    consumer_package_sha256: &str,
    identity: &EngineIdentity,
    factor_inputs: &[FactorInstancePlanInput<'_>],
    consumer_parameters: &BTreeMap<String, String>,
    signal_inputs: &[SignalPlanInput<'_>],
) -> Result<FrozenFeaturePlan, PlanValidationError> {
    let mut issues = Vec::new();
    if !matches!(
        manifest.kind,
        ComponentKind::Strategy | ComponentKind::Model
    ) {
        issues.push(issue("not-a-feature-consumer", None, None, None));
    }
    if manifest.manifest_schema_version.to_string() != COMPONENT_MANIFEST_SCHEMA_VERSION {
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
        .collect::<HashMap<_, _>>();
    let frozen_consumer_parameters =
        match crate::component_parameters(manifest, Some(&parameter_overrides)) {
            Ok(values) => manifest
                .parameters
                .iter()
                .zip(values)
                .map(|(definition, value)| (definition.name.clone(), value))
                .collect::<Vec<_>>(),
            Err(_) => {
                issues.push(issue(
                    "invalid-consumer-parameter",
                    None,
                    None,
                    Some("consumer-parameters"),
                ));
                Vec::new()
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
    let mut factors = HashMap::new();
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
            || input.manifest.output_names.is_empty()
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
        .collect::<HashMap<_, _>>();
    let signals = signal_inputs
        .iter()
        .map(|input| (input.slot_name, input))
        .collect::<HashMap<_, _>>();
    if signals.len() != signal_inputs.len() {
        issues.push(issue(
            "duplicate-signal-binding",
            None,
            Some("signal"),
            None,
        ));
    }
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

    let mut names = BTreeSet::new();
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
            FeatureSlotSource::Market { field } => slots.push(FeatureSlot {
                name: slot.name.clone(),
                source: FeatureSource::Market { field: *field },
                warmup_bars: 0,
            }),
            FeatureSlotSource::BuiltIn {
                indicator,
                output,
                inputs,
                parameters,
            } => match freeze_builtin(
                manifest,
                indicator,
                output,
                inputs,
                parameters,
                consumer_parameters,
            ) {
                Ok(resolved) => slots.push(FeatureSlot {
                    name: slot.name.clone(),
                    source: FeatureSource::BuiltIn {
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
            },
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
                    slots.push(FeatureSlot {
                        name: slot.name.clone(),
                        source: FeatureSource::External {
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
            FeatureSlotSource::Signal {
                prediction_kind,
                forecast_target,
                value_scale,
                horizon_bars,
            } => match signals.get(slot.name.as_str()) {
                Some(input)
                    if input.contract.prediction_kind == *prediction_kind
                        && input.contract.forecast_target == *forecast_target
                        && input.contract.value_scale == *value_scale
                        && input.contract.horizon_bars == *horizon_bars
                        && input.contract.name == input.signal_name
                        && is_sha256(input.dataset_id)
                        && !input.producer_segments.is_empty() =>
                {
                    slots.push(FeatureSlot {
                        name: slot.name.clone(),
                        source: FeatureSource::Signal {
                            dataset_id: input.dataset_id.into(),
                            signal_name: input.signal_name.into(),
                            snapshot_id: input.snapshot_id.into(),
                            instrument_id: input.instrument_id.clone(),
                            venue: input.venue.into(),
                            bar_interval: input.bar_interval.into(),
                            contract: serde_json::to_value(&input.contract).unwrap_or(Value::Null),
                            producer_segments: input.producer_segments.clone(),
                            artifact_provenance: input.artifact_provenance.clone(),
                            evidence_state: input.evidence_state.into(),
                            component_lock: input.component_lock.clone(),
                        },
                        warmup_bars: 0,
                    })
                }
                _ => issues.push(issue(
                    "incompatible-signal-binding",
                    Some(&slot.name),
                    Some("signal"),
                    None,
                )),
            },
        }
    }
    if signal_inputs.iter().any(|input| {
        !manifest
            .feature_slots
            .iter()
            .any(|slot| slot.name == input.slot_name)
    }) {
        issues.push(issue("unknown-signal-binding", None, Some("signal"), None));
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

    let mut frozen_factors = factors
        .values()
        .map(|input| FeatureFactor {
            alias: input.alias.to_owned(),
            feature_slots: input
                .manifest
                .feature_slots
                .iter()
                .map(|slot| slot.name.clone())
                .collect(),
            parameters: input
                .parameters
                .iter()
                .map(|value| serde_json::to_value(value).unwrap_or(Value::Null))
                .collect(),
            output_names: input.manifest.output_names.clone(),
            warmup_bars: input.manifest.warmup_bars,
        })
        .collect::<Vec<_>>();
    frozen_factors.sort_by(|left, right| left.alias.cmp(&right.alias));
    let consumer_parameters = frozen_consumer_parameters
        .iter()
        .map(|(name, value)| adaq_feature_engine::NamedParameter {
            name: name.clone(),
            value: serde_json::to_value(value).unwrap_or(Value::Null),
        })
        .collect();
    let inner = FeaturePlan::freeze(FeaturePlanDraft {
        slots,
        factors: frozen_factors.clone(),
        consumer_package_sha256: consumer_package_sha256.into(),
        consumer_parameters,
        consumer_warmup_bars: manifest.warmup_bars,
        engine_identity: identity.into(),
        operator_catalog: adaq_feature_engine::FeatureOperatorCatalog::initial(),
        ..FeaturePlanDraft::default()
    })
    .map_err(|error| PlanValidationError {
        issues: error
            .issues()
            .iter()
            .map(|issue| PlanIssue {
                code: issue.code.clone(),
                slot: issue.path.clone(),
                source: None,
                field: None,
            })
            .collect(),
    })?;
    let factor_parameters = frozen_factors
        .iter()
        .map(|factor| {
            factors
                .get(factor.alias.as_str())
                .map(|input| input.parameters.clone())
                .unwrap_or_default()
        })
        .collect();
    Ok(FrozenFeaturePlan {
        inner,
        factor_parameters,
    })
}

fn deserialize_factor_parameters(
    plan: &FeaturePlan,
) -> Result<Vec<Vec<ComponentParameterValue>>, PlanLoadError> {
    plan.factors()
        .iter()
        .map(|factor| {
            factor
                .parameters
                .iter()
                .cloned()
                .map(serde_json::from_value)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| load_error("invalid-plan-contract"))
        })
        .collect()
}

fn valid_engine_identity(identity: &EngineIdentity) -> bool {
    identity.engine_version == ENGINE_VERSION
        && matches!(
            identity.ta_lib_version.as_str(),
            C_TA_LIB_VERSION | ADAQ_TA_LIB_VERSION
        )
        && identity.catalog_version == CATALOG_VERSION
        && is_sha256(&identity.ta_source_sha256)
        && is_sha256(&identity.wrapper_sha256)
        && is_sha256(&identity.compiler_and_flags_sha256)
        && is_sha256(&identity.engine_build_id)
        && !identity.target_triple.is_empty()
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

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn unique_builtin_request_count(slots: &[FeatureSlot]) -> usize {
    slots
        .iter()
        .filter_map(|slot| match &slot.source {
            FeatureSource::BuiltIn {
                indicator,
                real_inputs,
                parameters,
                ..
            } => Some(format!("{indicator}:{real_inputs:?}:{parameters:?}")),
            _ => None,
        })
        .collect::<BTreeSet<_>>()
        .len()
}

struct ResolvedBuiltIn {
    real_inputs: Vec<MarketField>,
    parameters: BTreeMap<String, FrozenBuiltInParameter>,
    warmup_bars: u32,
}

fn freeze_builtin(
    manifest: &ComponentManifest,
    indicator: &str,
    output: &str,
    inputs: &BTreeMap<String, Value>,
    bindings: &BTreeMap<String, Value>,
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
                .and_then(|value| value.parse::<MarketField>().ok())
                .ok_or(("invalid-indicator-input", Some(input.id.clone())))?;
            if !input
                .allowed_fields
                .iter()
                .any(|value| value == field.as_str())
            {
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
                    .and_then(Value::as_str)
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
                (ParameterType::String, parameter.default.clone()),
                true,
                false,
            ),
        };
        let value = match parameter.kind.as_str() {
            "Integer" => {
                if !matches!(raw.0, ParameterType::Integer) && !is_default {
                    return Err(("mistyped-indicator-parameter", Some(parameter.id.clone())));
                }
                FrozenBuiltInParameter::Integer(
                    raw.1
                        .parse()
                        .map_err(|_| ("invalid-indicator-parameter", Some(parameter.id.clone())))?,
                )
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

    fn manifest() -> ComponentManifest {
        ComponentManifest {
            manifest_schema_version: Version::new(1, 0, 0),
            component_id: Uuid::from_u128(1),
            version: Version::new(1, 0, 0),
            name: "Market Strategy".into(),
            kind: ComponentKind::Strategy,
            strategy_scope: crate::StrategyScope::SingleInstrument,
            factor_scope: None,
            sdk_version: Version::parse(adaq_component_sdk::SDK_VERSION).unwrap(),
            abi_version: Version::parse(adaq_component_sdk::ABI_VERSION).unwrap(),
            wasm_sha256: String::new(),
            parameters: Vec::new(),
            feature_slots: vec![crate::FeatureSlotDefinition {
                name: "close".into(),
                source: FeatureSlotSource::Market {
                    field: MarketField::Close,
                },
            }],
            output_names: Vec::new(),
            dependencies: Vec::new(),
            warmup_bars: 0,
            model_scope: None,
            model_outputs: Vec::new(),
            model_artifact: None,
        }
    }

    #[test]
    fn manifest_slots_adapt_to_reusable_feature_plan_2() {
        let identity = native_engine_identity().unwrap();
        let plan =
            validate_and_freeze_feature_plan(&manifest(), &"a".repeat(64), &identity).unwrap();
        let document: Value = serde_json::from_slice(&plan.to_json()).unwrap();
        assert_eq!(document["planSchemaVersion"], "2.0.0");
        assert_eq!(document["slots"][0]["name"], "close");
        assert!(document.get("snapshotId").is_none());
        assert!(document.get("seed").is_none());
        assert_eq!(
            FrozenFeaturePlan::load_for_engine(&plan.to_json(), &identity).unwrap(),
            plan
        );
    }

    #[test]
    fn old_plan_evidence_requires_explicit_reset() {
        let error = FrozenFeaturePlan::load(br#"{"planSchemaVersion":"1.0.0"}"#).unwrap_err();
        assert_eq!(error.code, "reset-required");
    }
}
