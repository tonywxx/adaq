use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::package::is_lower_kebab;
use crate::{ComponentKind, ComponentManifest, FeatureSlotSource, MarketField};

const PLAN_SCHEMA_VERSION: &str = "1.0.0";
const CATALOG_VERSION: &str = "adaq-indicator-catalog@1.0.0";
const ENGINE_VERSION: &str = "adaq-indicator-engine@1.0.0";
const TA_LIB_VERSION: &str = "0.7.1";

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
    effective_warmup_bars: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FrozenSlot {
    name: String,
    source: FrozenMarketSource,
    warmup_bars: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
enum FrozenMarketSource {
    Market { field: MarketField },
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
            FrozenMarketSource::Market { field } => field,
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
            || document.content.effective_warmup_bars != 0
            || document.content.slots.is_empty()
        {
            return Err(load_error("invalid-plan-contract"));
        }
        let mut names = std::collections::HashSet::new();
        if document.content.slots.iter().any(|slot| {
            slot.warmup_bars != 0 || !is_lower_kebab(&slot.name) || !names.insert(&slot.name)
        }) {
            return Err(load_error("invalid-plan-contract"));
        }
        Ok(Self(document))
    }
}

pub fn validate_and_freeze(
    manifest: &ComponentManifest,
    strategy_package_sha256: &str,
    identity: &EngineIdentity,
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
                source: FrozenMarketSource::Market { field: *field },
                warmup_bars: 0,
            }),
            FeatureSlotSource::BuiltIn { .. } => issues.push(issue(
                "unsupported-source",
                Some(&slot.name),
                Some("builtin"),
                None,
            )),
            FeatureSlotSource::External { .. } => issues.push(issue(
                "unsupported-source",
                Some(&slot.name),
                Some("external"),
                None,
            )),
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
        slots,
        effective_warmup_bars: 0,
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
            "43a3647e1be2ab25c0abf367e5dad78a6098ae546577c591f1cf96fb4df653ca"
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
    fn reports_unsupported_sources_as_deterministically_ordered_typed_issues() {
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
                issue("unsupported-source", Some("a-rsi"), Some("builtin"), None),
                issue(
                    "unsupported-source",
                    Some("z-factor"),
                    Some("external"),
                    None
                ),
            ]
        );
    }
}
