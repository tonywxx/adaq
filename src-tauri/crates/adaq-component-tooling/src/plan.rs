use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::package::is_lower_kebab;
use crate::{
    ComponentKind, ComponentManifest, ComponentParameterValue, FeatureSlotSource, MarketField,
};

const PLAN_SCHEMA_VERSION: &str = "1.0.0";
const CATALOG_VERSION: &str = "adaq-indicator-catalog@1.0.0";
const ENGINE_VERSION: &str = "adaq-indicator-engine@1.0.0";
const TA_LIB_VERSION: &str = "0.7.1";
const MAX_FACTOR_INSTANCES: usize = 64;
const MAX_FACTOR_OUTPUTS: usize = 64;

#[derive(Debug, Clone)]
pub struct FactorInstancePlanInput<'a> {
    pub alias: &'a str,
    pub manifest: &'a ComponentManifest,
    pub parameters: Vec<ComponentParameterValue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineIdentity {
    pub engine_build_id: String,
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
            "Indicator Plan validation failed with {} issue(s)",
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
    strategy_package_sha256: String,
    catalog_version: String,
    engine_version: String,
    ta_lib_version: String,
    engine_build_id: String,
    slots: Vec<FrozenSlot>,
    #[serde(default)]
    factors: Vec<FrozenFactor>,
    effective_warmup_bars: u32,
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FrozenFactor {
    alias: String,
    parameters: Vec<ComponentParameterValue>,
    output_names: Vec<String>,
    warmup_bars: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrozenIndicatorPlan(PlanDocument);

impl FrozenIndicatorPlan {
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
        canonical_json(&self.0).expect("a validated Indicator Plan is serializable")
    }

    pub fn load(bytes: &[u8]) -> Result<Self, PlanLoadError> {
        let document = serde_json::from_slice::<PlanDocument>(bytes)
            .map_err(|_| load_error("invalid-plan-json"))?;
        if document.plan_hash
            != hash(
                &canonical_json(&document.content).map_err(|_| load_error("invalid-plan-json"))?,
            )
        {
            return Err(load_error("plan-hash-mismatch"));
        }
        if document.content.plan_schema_version != PLAN_SCHEMA_VERSION
            || document.content.catalog_version != CATALOG_VERSION
            || document.content.engine_version != ENGINE_VERSION
            || document.content.ta_lib_version != TA_LIB_VERSION
            || !is_sha256(&document.content.strategy_package_sha256)
            || document.content.engine_build_id.is_empty()
            || document.content.slots.is_empty()
            || document.content.factors.len() > MAX_FACTOR_INSTANCES
        {
            return Err(load_error("invalid-plan-contract"));
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
}

#[derive(Debug, Clone, Copy)]
pub struct FrozenFactorView<'a> {
    pub alias: &'a str,
    pub parameters: &'a [ComponentParameterValue],
    pub output_names: &'a [String],
    pub warmup_bars: u32,
}

pub fn validate_and_freeze(
    manifest: &ComponentManifest,
    strategy_package_sha256: &str,
    identity: &EngineIdentity,
) -> Result<FrozenIndicatorPlan, PlanValidationError> {
    validate_and_freeze_with_factors(manifest, strategy_package_sha256, identity, &[])
}

pub fn validate_and_freeze_with_factors(
    manifest: &ComponentManifest,
    strategy_package_sha256: &str,
    identity: &EngineIdentity,
    factor_inputs: &[FactorInstancePlanInput<'_>],
) -> Result<FrozenIndicatorPlan, PlanValidationError> {
    let mut issues = Vec::new();
    if manifest.kind != ComponentKind::Strategy {
        issues.push(issue("not-a-strategy", None, None, None));
    }
    if manifest.manifest_schema_version.to_string() != PLAN_SCHEMA_VERSION {
        issues.push(issue(
            "unsupported-manifest-schema",
            None,
            None,
            Some("manifest-schema-version"),
        ));
    }
    if !is_sha256(strategy_package_sha256) {
        issues.push(issue(
            "invalid-strategy-package-hash",
            None,
            None,
            Some("strategy-package-sha256"),
        ));
    }
    if identity.engine_build_id.is_empty() {
        issues.push(issue(
            "invalid-engine-build-id",
            None,
            None,
            Some("engine-build-id"),
        ));
    }
    if manifest.feature_slots.is_empty() {
        issues.push(issue("missing-feature-slots", None, None, None));
    }

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
    let mut slots = Vec::with_capacity(manifest.feature_slots.len());
    for slot in &manifest.feature_slots {
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
            FeatureSlotSource::BuiltIn { .. } => issues.push(issue(
                "unsupported-source",
                Some(&slot.name),
                Some("builtin"),
                None,
            )),
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
        strategy_package_sha256: strategy_package_sha256.into(),
        catalog_version: CATALOG_VERSION.into(),
        engine_version: ENGINE_VERSION.into(),
        ta_lib_version: TA_LIB_VERSION.into(),
        engine_build_id: identity.engine_build_id.clone(),
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
    let plan_hash =
        hash(&canonical_json(&content).expect("validated Plan content is serializable"));
    Ok(FrozenIndicatorPlan(PlanDocument { content, plan_hash }))
}

fn canonical_json(value: &impl Serialize) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&serde_json::to_value(value)?)
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
        }
    }

    fn market(name: &str, field: MarketField) -> FeatureSlotDefinition {
        FeatureSlotDefinition {
            name: name.into(),
            source: FeatureSlotSource::Market { field },
        }
    }

    #[test]
    fn freezes_and_loads_a_canonical_market_plan_in_manifest_order() {
        let manifest = manifest(vec![
            market("quote-volume", MarketField::QuoteVolume),
            market("close", MarketField::Close),
        ]);
        let identity = EngineIdentity {
            engine_build_id: "test-build".into(),
        };
        let package_hash = "a".repeat(64);

        let first = validate_and_freeze(&manifest, &package_hash, &identity).unwrap();
        let replay = validate_and_freeze(&manifest, &package_hash, &identity).unwrap();

        assert_eq!(first.plan_hash(), replay.plan_hash());
        assert_eq!(
            first.plan_hash(),
            "249301d89bd5037ab89d2a92b66541088105849c0c09228fcd7b7fe46966682b"
        );
        assert_eq!(
            first.slot_names().collect::<Vec<_>>(),
            ["quote-volume", "close"]
        );
        assert_eq!(
            first.market_fields().collect::<Vec<_>>(),
            [MarketField::QuoteVolume, MarketField::Close]
        );
        assert_eq!(first.effective_warmup_bars(), 0);
        assert_eq!(FrozenIndicatorPlan::load(&first.to_json()).unwrap(), first);

        let mut tampered = String::from_utf8(first.to_json()).unwrap();
        tampered = tampered.replace("quote-volume", "base-volume");
        assert_eq!(
            FrozenIndicatorPlan::load(tampered.as_bytes())
                .unwrap_err()
                .code,
            "plan-hash-mismatch"
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
                },
            },
        ]);
        let error = validate_and_freeze(
            &manifest,
            &"a".repeat(64),
            &EngineIdentity {
                engine_build_id: "test-build".into(),
            },
        )
        .unwrap_err();
        assert_eq!(
            error.issues,
            [
                issue(
                    "invalid-factor-output",
                    Some("z-factor"),
                    Some("external"),
                    Some("value")
                ),
                issue("unsupported-source", Some("a-rsi"), Some("builtin"), None),
            ]
        );
    }
}
