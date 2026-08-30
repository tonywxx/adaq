use std::{
    collections::BTreeMap,
    fmt::Write as _,
    io::{Cursor, Read, Write},
};

pub use adaq_feature_engine::MarketField;
use rust_decimal::Decimal;
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use wasmparser::{Encoding, Parser, Payload, Validator};
use zip::{DateTime, ZipArchive, ZipWriter, write::SimpleFileOptions};

const MANIFEST_NAME: &str = "manifest.json";
const COMPONENT_NAME: &str = "component.wasm";
const MAX_PACKAGE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ComponentKind {
    Factor,
    Strategy,
    Model,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FactorScope {
    TimeSeries,
    CrossSectional,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StrategyArchitecture {
    SignalDriven,
    Composed,
    Hybrid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StrategyScope {
    SingleInstrument,
    Portfolio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModelScope {
    SingleInstrument,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum PredictionKind {
    Score,
    Probability,
    ExpectedValue,
    Custom {
        id: String,
        version: Version,
        description: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ForecastTargetValueType {
    Binary,
    Continuous,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ForecastTarget {
    Builtin {
        target: BuiltinForecastTarget,
    },
    Custom {
        id: String,
        version: Version,
        description: String,
        value_type: ForecastTargetValueType,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BuiltinForecastTarget {
    FutureCloseReturn,
    FutureCloseUp,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ForecastValueScale {
    Probability,
    Native,
    Percentile,
    ZScore {
        method: String,
    },
    Custom {
        id: String,
        version: Version,
        description: String,
        minimum: Option<f64>,
        maximum: Option<f64>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelOutput {
    pub name: String,
    pub prediction_kind: PredictionKind,
    pub forecast_target: ForecastTarget,
    pub value_scale: ForecastValueScale,
    pub horizon_bars: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelArtifact {
    pub sha256: String,
    #[serde(default)]
    pub provenance: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ParameterType {
    Decimal,
    Integer,
    Boolean,
    String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParameterDefinition {
    pub name: String,
    pub parameter_type: ParameterType,
    pub default_value: String,
    #[serde(default)]
    pub allowed_values: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentDependency {
    pub component_id: Uuid,
    pub version: VersionReq,
    pub alias: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ComponentManifest {
    pub manifest_schema_version: Version,
    pub component_id: Uuid,
    pub version: Version,
    pub name: String,
    pub kind: ComponentKind,
    #[serde(default = "default_strategy_scope")]
    pub strategy_scope: StrategyScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub factor_scope: Option<FactorScope>,
    pub sdk_version: Version,
    pub abi_version: Version,
    #[serde(default)]
    pub wasm_sha256: String,
    #[serde(default)]
    pub parameters: Vec<ParameterDefinition>,
    #[serde(default)]
    pub feature_slots: Vec<FeatureSlotDefinition>,
    #[serde(default)]
    pub output_names: Vec<String>,
    #[serde(default)]
    pub dependencies: Vec<ComponentDependency>,
    #[serde(default)]
    pub warmup_bars: u32,
    #[serde(default)]
    pub model_scope: Option<ModelScope>,
    #[serde(default)]
    pub model_outputs: Vec<ModelOutput>,
    #[serde(default)]
    pub model_artifact: Option<ModelArtifact>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FeatureSlotDefinition {
    pub name: String,
    pub source: FeatureSlotSource,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum FeatureSlotSource {
    Market {
        field: MarketField,
    },
    #[serde(rename = "builtin")]
    BuiltIn {
        indicator: String,
        output: String,
        #[serde(default)]
        inputs: BTreeMap<String, serde_json::Value>,
        #[serde(default)]
        parameters: BTreeMap<String, serde_json::Value>,
    },
    External {
        dependency_alias: String,
        output: String,
    },
    Signal {
        prediction_kind: PredictionKind,
        forecast_target: ForecastTarget,
        value_scale: ForecastValueScale,
        horizon_bars: u32,
    },
}

pub fn strategy_architecture(manifest: &ComponentManifest) -> Option<StrategyArchitecture> {
    (manifest.kind == ComponentKind::Strategy).then(|| {
        let signals = manifest
            .feature_slots
            .iter()
            .filter(|slot| matches!(slot.source, FeatureSlotSource::Signal { .. }))
            .count();
        match (signals, manifest.feature_slots.len()) {
            (0, _) => StrategyArchitecture::Composed,
            (signals, total) if signals == total => StrategyArchitecture::SignalDriven,
            _ => StrategyArchitecture::Hybrid,
        }
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComponentPackage {
    pub manifest: ComponentManifest,
    pub wasm: Vec<u8>,
    pub archive_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageError(pub String);

impl std::fmt::Display for PackageError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for PackageError {}

impl ComponentPackage {
    pub fn read(bytes: &[u8]) -> Result<Self, PackageError> {
        if bytes.is_empty() || bytes.len() > MAX_PACKAGE_BYTES {
            return Err(PackageError("Component Package size is invalid".into()));
        }
        let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(error)?;
        if archive.len() != 2 {
            return Err(PackageError(
                "Component Package must contain only manifest.json and component.wasm".into(),
            ));
        }
        let mut names = (0..archive.len())
            .map(|index| {
                archive
                    .by_index(index)
                    .map(|file| file.name().to_owned())
                    .map_err(error)
            })
            .collect::<Result<Vec<_>, _>>()?;
        names.sort();
        if names != [COMPONENT_NAME, MANIFEST_NAME] {
            return Err(PackageError("Component Package layout is invalid".into()));
        }

        let manifest = {
            let mut file = archive.by_name(MANIFEST_NAME).map_err(error)?;
            if file.size() > 1024 * 1024 {
                return Err(PackageError("Component manifest is too large".into()));
            }
            let mut json = String::new();
            file.read_to_string(&mut json).map_err(error)?;
            serde_json::from_str::<ComponentManifest>(&json).map_err(error)?
        };
        let wasm = {
            let mut file = archive.by_name(COMPONENT_NAME).map_err(error)?;
            if file.size() > MAX_PACKAGE_BYTES as u64 {
                return Err(PackageError("Component WASM is too large".into()));
            }
            let mut wasm = Vec::with_capacity(file.size() as usize);
            file.read_to_end(&mut wasm).map_err(error)?;
            wasm
        };
        validate_manifest(&manifest, &wasm)?;
        Ok(Self {
            manifest,
            wasm,
            archive_sha256: sha256(bytes),
        })
    }
}

pub fn pack_component(
    mut manifest: ComponentManifest,
    wasm: &[u8],
) -> Result<Vec<u8>, PackageError> {
    manifest.wasm_sha256 = sha256(wasm);
    if let Some(artifact) = &mut manifest.model_artifact {
        // Native Models have no sidecar weight loader: the artifact is embedded in component.wasm.
        artifact.sha256.clone_from(&manifest.wasm_sha256);
    }
    validate_manifest(&manifest, wasm)?;
    let manifest_json = serde_json::to_vec_pretty(&manifest).map_err(error)?;
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(DateTime::DEFAULT)
        .unix_permissions(0o644);
    writer.start_file(MANIFEST_NAME, options).map_err(error)?;
    writer.write_all(&manifest_json).map_err(error)?;
    writer.start_file(COMPONENT_NAME, options).map_err(error)?;
    writer.write_all(wasm).map_err(error)?;
    Ok(writer.finish().map_err(error)?.into_inner())
}

/// Checks the published Component contract rules from ADR 0019.
pub fn check_manifest_compatibility(
    previous: &ComponentManifest,
    current: &ComponentManifest,
) -> Result<(), PackageError> {
    if previous.component_id != current.component_id || previous.kind != current.kind {
        return Err(PackageError(
            "Component identity and kind cannot change".into(),
        ));
    }
    if current.version <= previous.version {
        return Err(PackageError("Component version must increase".into()));
    }
    // 0.x Components are intentionally development-unstable (ADR 0019).
    if previous.version.major == 0 || current.version.major == 0 {
        return Ok(());
    }

    let breaking = previous.manifest_schema_version != current.manifest_schema_version
        || previous.abi_version != current.abi_version
        || previous.strategy_scope != current.strategy_scope
        || previous.factor_scope != current.factor_scope
        || previous.feature_slots != current.feature_slots
        || previous.dependencies != current.dependencies
        || previous.warmup_bars != current.warmup_bars
        || !current.parameters.starts_with(&previous.parameters)
        || !current.output_names.starts_with(&previous.output_names)
        || !current.model_outputs.starts_with(&previous.model_outputs)
        || current.model_scope != previous.model_scope;
    let additive = current.parameters.len() > previous.parameters.len()
        || ((current.kind == ComponentKind::Factor || current.kind == ComponentKind::Model)
            && current.output_names.len() > previous.output_names.len());

    if breaking && current.version.major == previous.version.major {
        return Err(PackageError(
            "Breaking Component contract changes require a major version".into(),
        ));
    }
    if additive
        && current.version.major == previous.version.major
        && current.version.minor == previous.version.minor
    {
        return Err(PackageError(
            "Added optional parameters or Factor outputs require a minor version".into(),
        ));
    }
    Ok(())
}

fn validate_manifest(manifest: &ComponentManifest, wasm: &[u8]) -> Result<(), PackageError> {
    let expected_abi = if manifest.kind == ComponentKind::Factor {
        Version::parse(adaq_component_sdk::FACTOR_ABI_VERSION).unwrap()
    } else {
        Version::parse(adaq_component_sdk::ABI_VERSION).unwrap()
    };
    if manifest.kind == ComponentKind::Factor && manifest.abi_version != expected_abi {
        return Err(PackageError(format!(
            "reset-required: Factor ABI {} is incompatible with Factor ABI v2; perform an explicit device-level reset",
            manifest.abi_version
        )));
    }
    if manifest.name.trim().is_empty()
        || manifest.manifest_schema_version != Version::new(1, 0, 0)
        || manifest.sdk_version != Version::parse(adaq_component_sdk::SDK_VERSION).unwrap()
        || manifest.abi_version != expected_abi
        || manifest.wasm_sha256 != sha256(wasm)
        || !wasm.starts_with(b"\0asm")
    {
        return Err(PackageError("Component manifest or WASM is invalid".into()));
    }
    Validator::new().validate_all(wasm).map_err(error)?;
    let mut depth = 0usize;
    for payload in Parser::new(0).parse_all(wasm) {
        match payload.map_err(error)? {
            Payload::Version { encoding, .. } => {
                if depth == 0 && encoding != Encoding::Component {
                    return Err(PackageError("WASM is not a Component".into()));
                }
                depth += 1;
            }
            Payload::ComponentImportSection(section) if depth == 1 => {
                if section
                    .into_iter()
                    .next()
                    .transpose()
                    .map_err(error)?
                    .is_some()
                {
                    return Err(PackageError(
                        "Component has forbidden ambient imports".into(),
                    ));
                }
            }
            Payload::End(_) => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    unique_non_empty(
        manifest.parameters.iter().map(|value| value.name.as_str()),
        "parameter",
    )?;
    match manifest.kind {
        ComponentKind::Factor | ComponentKind::Model
            if manifest.strategy_scope != StrategyScope::SingleInstrument =>
        {
            return Err(PackageError(
                "Only Strategy manifests may declare Portfolio scope".into(),
            ));
        }
        ComponentKind::Factor if manifest.factor_scope.is_none() => {
            return Err(PackageError(
                "Factor manifests must declare exactly one factorScope".into(),
            ));
        }
        ComponentKind::Factor if manifest.feature_slots.is_empty() => {
            return Err(PackageError(
                "Factor manifests must declare ordered Feature Slots".into(),
            ));
        }
        ComponentKind::Factor if manifest.output_names.is_empty() => {
            return Err(PackageError(
                "Factor manifests must declare 1..=64 outputs".into(),
            ));
        }
        ComponentKind::Strategy if manifest.feature_slots.is_empty() => {
            return Err(PackageError(
                "Strategy manifests must declare Feature Slots".into(),
            ));
        }
        ComponentKind::Strategy if !manifest.output_names.is_empty() => {
            return Err(PackageError(
                "Strategy manifests cannot declare Factor outputs".into(),
            ));
        }
        ComponentKind::Model
            if manifest.feature_slots.is_empty() || !manifest.output_names.is_empty() =>
        {
            return Err(PackageError("Model manifests require Feature Slots and use modelOutputs instead of Factor outputs".into()));
        }
        ComponentKind::Model
            if manifest.model_scope != Some(ModelScope::SingleInstrument)
                || manifest.model_artifact.is_none() =>
        {
            return Err(PackageError(
                "Model manifests require Single-Instrument scope and an embedded Model Artifact"
                    .into(),
            ));
        }
        ComponentKind::Factor | ComponentKind::Strategy
            if manifest.model_scope.is_some()
                || !manifest.model_outputs.is_empty()
                || manifest.model_artifact.is_some() =>
        {
            return Err(PackageError(
                "Only Model manifests may declare Model contracts".into(),
            ));
        }
        _ => {}
    }
    unique_identifiers(
        manifest.feature_slots.iter().map(|slot| slot.name.as_str()),
        "Feature Slot",
    )?;
    if manifest.feature_slots.len() > 64 {
        return Err(PackageError(
            "Components may declare at most 64 Feature Slots".into(),
        ));
    }
    if manifest.output_names.len() > 64 {
        return Err(PackageError(
            "Factor Components may declare at most 64 outputs".into(),
        ));
    }
    unique_identifiers(manifest.output_names.iter().map(String::as_str), "output")?;
    validate_model_contract(manifest)?;
    unique_identifiers(
        manifest
            .dependencies
            .iter()
            .map(|value| value.alias.as_str()),
        "dependency",
    )?;
    let dependencies = manifest
        .dependencies
        .iter()
        .map(|dependency| dependency.alias.as_str())
        .collect::<std::collections::HashSet<_>>();
    let mut referenced_dependencies = std::collections::HashSet::new();
    for slot in &manifest.feature_slots {
        if let FeatureSlotSource::External {
            dependency_alias,
            output,
        } = &slot.source
        {
            if !dependencies.contains(dependency_alias.as_str()) || !is_lower_kebab(output) {
                return Err(PackageError(
                    "External Feature Slots require a declared dependency alias and lower-kebab-case output"
                        .into(),
                ));
            }
            referenced_dependencies.insert(dependency_alias.as_str());
        }
        if let FeatureSlotSource::Signal {
            prediction_kind,
            forecast_target,
            value_scale,
            horizon_bars,
        } = &slot.source
        {
            if manifest.kind != ComponentKind::Strategy {
                return Err(PackageError(
                    "Only Strategy manifests may declare Forecast Signal Slots".into(),
                ));
            }
            validate_model_outputs(&[ModelOutput {
                name: slot.name.clone(),
                prediction_kind: prediction_kind.clone(),
                forecast_target: forecast_target.clone(),
                value_scale: value_scale.clone(),
                horizon_bars: *horizon_bars,
            }])?;
        }
    }
    if referenced_dependencies.len() != dependencies.len() {
        return Err(PackageError(
            "Every Component dependency must be referenced by an External Feature Slot".into(),
        ));
    }
    for parameter in &manifest.parameters {
        let valid_default = match parameter.parameter_type {
            ParameterType::Decimal => Decimal::from_str_exact(&parameter.default_value).is_ok(),
            ParameterType::Integer => parameter.default_value.parse::<i64>().is_ok(),
            ParameterType::Boolean => parameter.default_value.parse::<bool>().is_ok(),
            ParameterType::String => true,
        };
        let valid_allowed_values =
            parameter
                .allowed_values
                .iter()
                .all(|value| match parameter.parameter_type {
                    ParameterType::Decimal => Decimal::from_str_exact(value).is_ok(),
                    ParameterType::Integer => value.parse::<i64>().is_ok(),
                    ParameterType::Boolean => value.parse::<bool>().is_ok(),
                    ParameterType::String => true,
                });
        if !parameter.allowed_values.is_empty()
            && (!valid_allowed_values
                || !parameter.allowed_values.contains(&parameter.default_value))
        {
            return Err(PackageError("Parameter allowed values are invalid".into()));
        }
        if !valid_default {
            return Err(PackageError("Parameter default value is invalid".into()));
        }
    }
    Ok(())
}

fn validate_model_contract(manifest: &ComponentManifest) -> Result<(), PackageError> {
    if manifest.kind != ComponentKind::Model {
        return Ok(());
    }
    let artifact = manifest.model_artifact.as_ref().expect("validated above");
    if artifact.sha256 != manifest.wasm_sha256 {
        return Err(PackageError(
            "Model Artifact identity must match the embedded component.wasm SHA-256".into(),
        ));
    }
    validate_model_outputs(&manifest.model_outputs)
}

fn default_strategy_scope() -> StrategyScope {
    StrategyScope::SingleInstrument
}

pub fn validate_model_outputs(outputs: &[ModelOutput]) -> Result<(), PackageError> {
    if !(1..=64).contains(&outputs.len()) {
        return Err(PackageError(
            "Model Components must declare one through 64 outputs".into(),
        ));
    }
    unique_identifiers(
        outputs.iter().map(|output| output.name.as_str()),
        "Model output",
    )?;
    for output in outputs {
        let target_type = match output.forecast_target {
            ForecastTarget::Builtin {
                target: BuiltinForecastTarget::FutureCloseUp,
            } => ForecastTargetValueType::Binary,
            ForecastTarget::Builtin {
                target: BuiltinForecastTarget::FutureCloseReturn,
            } => ForecastTargetValueType::Continuous,
            ForecastTarget::Custom { value_type, .. } => value_type,
        };
        if output.horizon_bars == 0 {
            return Err(PackageError(
                "Model output horizonBars must be positive".into(),
            ));
        }
        let custom_identity =
            |id: &str, description: &str| is_lower_kebab(id) && !description.trim().is_empty();
        match (&output.prediction_kind, &output.value_scale) {
            (PredictionKind::Probability, _) if target_type != ForecastTargetValueType::Binary => {
                return Err(PackageError(
                    "Probability requires a Binary Forecast Target".into(),
                ));
            }
            (PredictionKind::ExpectedValue, _)
                if target_type != ForecastTargetValueType::Continuous =>
            {
                return Err(PackageError(
                    "Expected Value requires a Continuous Forecast Target".into(),
                ));
            }
            (PredictionKind::Probability, ForecastValueScale::Probability) => {}
            (PredictionKind::Probability, _) => {
                return Err(PackageError(
                    "Probability requires the Probability Value Scale".into(),
                ));
            }
            (PredictionKind::ExpectedValue, ForecastValueScale::Native) => {}
            (PredictionKind::ExpectedValue, _) => {
                return Err(PackageError(
                    "Expected Value requires the native Forecast Value Scale".into(),
                ));
            }
            (
                PredictionKind::Score,
                ForecastValueScale::Percentile
                | ForecastValueScale::ZScore { .. }
                | ForecastValueScale::Custom { .. },
            ) => {}
            (PredictionKind::Score, _) => {
                return Err(PackageError(
                    "Score requires Percentile, Z-score, or Custom Forecast Value Scale".into(),
                ));
            }
            (
                PredictionKind::Custom {
                    id, description, ..
                },
                ForecastValueScale::Custom { .. },
            ) if custom_identity(id, description) => {}
            (PredictionKind::Custom { .. }, _) => {
                return Err(PackageError(
                    "Custom Prediction Kind requires an identified Custom Forecast Value Scale"
                        .into(),
                ));
            }
        }
        if let ForecastTarget::Custom {
            id, description, ..
        } = &output.forecast_target
        {
            if !custom_identity(id, description) {
                return Err(PackageError(
                    "Custom Forecast Target identity is invalid".into(),
                ));
            }
        }
        match &output.value_scale {
            ForecastValueScale::ZScore { method } if method.trim().is_empty() => {
                return Err(PackageError(
                    "Z-score Forecast Value Scale requires a method".into(),
                ));
            }
            ForecastValueScale::Custom {
                id,
                description,
                minimum,
                maximum,
                ..
            } => {
                if !custom_identity(id, description)
                    || minimum.is_some_and(|value| !value.is_finite())
                    || maximum.is_some_and(|value| !value.is_finite())
                    || minimum
                        .zip(*maximum)
                        .is_some_and(|(minimum, maximum)| minimum > maximum)
                {
                    return Err(PackageError(
                        "Custom Forecast Value Scale is invalid".into(),
                    ));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn unique_non_empty<'a>(
    values: impl Iterator<Item = &'a str>,
    label: &str,
) -> Result<(), PackageError> {
    let mut seen = std::collections::HashSet::new();
    if values
        .into_iter()
        .any(|value| value.trim().is_empty() || !seen.insert(value))
    {
        return Err(PackageError(format!(
            "Component {label} names must be non-empty and unique"
        )));
    }
    Ok(())
}

fn unique_identifiers<'a>(
    values: impl Iterator<Item = &'a str>,
    label: &str,
) -> Result<(), PackageError> {
    let mut seen = std::collections::HashSet::new();
    if values
        .into_iter()
        .any(|value| !is_lower_kebab(value) || !seen.insert(value))
    {
        return Err(PackageError(format!(
            "Component {label} names must be unique lower-kebab-case ASCII identifiers of at most 64 bytes"
        )));
    }
    Ok(())
}

pub(crate) fn is_lower_kebab(value: &str) -> bool {
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

fn sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

fn error(error: impl std::fmt::Display) -> PackageError {
    PackageError(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> ComponentManifest {
        ComponentManifest {
            manifest_schema_version: Version::new(1, 0, 0),
            component_id: Uuid::nil(),
            version: Version::new(1, 2, 3),
            name: "Fixture".into(),
            kind: ComponentKind::Factor,
            strategy_scope: StrategyScope::SingleInstrument,
            factor_scope: Some(FactorScope::TimeSeries),
            sdk_version: Version::parse(adaq_component_sdk::SDK_VERSION).unwrap(),
            abi_version: Version::parse(adaq_component_sdk::FACTOR_ABI_VERSION).unwrap(),
            wasm_sha256: String::new(),
            parameters: vec![],
            feature_slots: vec![FeatureSlotDefinition {
                name: "close".into(),
                source: FeatureSlotSource::Market {
                    field: MarketField::Close,
                },
            }],
            output_names: vec!["value".into()],
            dependencies: vec![],
            warmup_bars: 0,
            model_scope: None,
            model_outputs: vec![],
            model_artifact: None,
        }
    }

    fn strategy_manifest() -> ComponentManifest {
        let mut manifest = manifest();
        manifest.kind = ComponentKind::Strategy;
        manifest.factor_scope = None;
        manifest.abi_version = Version::parse(adaq_component_sdk::ABI_VERSION).unwrap();
        manifest
    }

    #[test]
    fn packing_is_deterministic() {
        let wasm = b"\0asm\x0d\0\x01\0";
        assert_eq!(
            pack_component(manifest(), wasm),
            pack_component(manifest(), wasm)
        );
    }

    #[test]
    fn package_round_trip_and_tamper_detection() {
        let wasm = b"\0asm\x0d\0\x01\0";
        let bytes = pack_component(manifest(), wasm).unwrap();
        let package = ComponentPackage::read(&bytes).unwrap();
        assert_eq!(package.wasm, wasm);
        assert_eq!(package.manifest.wasm_sha256, sha256(wasm));

        let mut invalid = manifest();
        invalid.wasm_sha256 = "wrong".into();
        assert!(validate_manifest(&invalid, wasm).is_err());

        let mut invalid = manifest();
        invalid.sdk_version = Version::new(0, 0, 0);
        invalid.wasm_sha256 = sha256(wasm);
        assert!(validate_manifest(&invalid, wasm).is_err());

        let mut invalid = manifest();
        invalid.wasm_sha256 = sha256(wasm);
        invalid.parameters.push(ParameterDefinition {
            name: "period".into(),
            parameter_type: ParameterType::Integer,
            default_value: "1.5".into(),
            allowed_values: vec![],
        });
        assert!(validate_manifest(&invalid, wasm).is_err());

        let mut invalid = manifest();
        invalid.manifest_schema_version = Version::new(2, 0, 0);
        invalid.wasm_sha256 = sha256(wasm);
        assert!(validate_manifest(&invalid, wasm).is_err());
    }

    #[test]
    fn numeric_parameter_allowed_values_are_type_checked() {
        let wasm = b"\0asm\x0d\0\x01\0";
        let mut manifest = strategy_manifest();
        manifest.output_names.clear();
        manifest.parameters.push(ParameterDefinition {
            name: "top-n".into(),
            parameter_type: ParameterType::Integer,
            default_value: "3".into(),
            allowed_values: vec!["3".into(), "5".into()],
        });
        assert!(pack_component(manifest, wasm).is_ok());
    }

    #[test]
    fn factor_manifests_require_outputs_and_reset_incompatible_abi() {
        let wasm = b"\0asm\x0d\0\x01\0";
        let mut invalid = manifest();
        invalid.output_names.clear();
        invalid.wasm_sha256 = sha256(wasm);
        assert!(validate_manifest(&invalid, wasm).is_err());

        invalid = manifest();
        invalid.abi_version = Version::new(3, 0, 0);
        invalid.wasm_sha256 = sha256(wasm);
        assert!(
            validate_manifest(&invalid, wasm)
                .unwrap_err()
                .0
                .starts_with("reset-required:")
        );
    }

    #[test]
    fn strategy_manifest_requires_ordered_feature_slots_and_rejects_input_names() {
        let legacy = r#"{
            "componentId":"22222222-2222-4222-8222-222222222222",
            "version":"1.0.0",
            "name":"Legacy",
            "kind":"strategy",
            "sdkVersion":"0.1.0",
            "abiVersion":"1.0.0",
            "inputNames":["close"]
        }"#;
        assert!(serde_json::from_str::<ComponentManifest>(legacy).is_err());

        let current = r#"{
            "manifestSchemaVersion":"1.0.0",
            "componentId":"22222222-2222-4222-8222-222222222222",
            "version":"1.0.0",
            "name":"Market",
            "kind":"strategy",
            "sdkVersion":"0.1.0",
            "abiVersion":"1.0.0",
            "featureSlots":[
                {"name":"quote-volume","source":{"kind":"market","field":"quote-volume"}},
                {"name":"close","source":{"kind":"market","field":"close"}}
            ]
        }"#;
        let manifest = serde_json::from_str::<ComponentManifest>(current).unwrap();
        assert_eq!(manifest.feature_slots[0].name, "quote-volume");
        assert_eq!(manifest.feature_slots[1].name, "close");
    }

    #[test]
    fn strategy_signal_requirements_are_typed_and_architecture_is_derived() {
        let wasm = b"\0asm\x0d\0\x01\0";
        let signal = FeatureSlotDefinition {
            name: "forecast-up".into(),
            source: FeatureSlotSource::Signal {
                prediction_kind: PredictionKind::Probability,
                forecast_target: ForecastTarget::Builtin {
                    target: BuiltinForecastTarget::FutureCloseUp,
                },
                value_scale: ForecastValueScale::Probability,
                horizon_bars: 1,
            },
        };
        let mut strategy = strategy_manifest();
        strategy.kind = ComponentKind::Strategy;
        strategy.output_names.clear();
        strategy.feature_slots = vec![signal.clone()];
        strategy.wasm_sha256 = sha256(wasm);
        assert!(validate_manifest(&strategy, wasm).is_ok());
        assert_eq!(
            strategy_architecture(&strategy),
            Some(StrategyArchitecture::SignalDriven)
        );

        strategy.feature_slots.push(FeatureSlotDefinition {
            name: "close".into(),
            source: FeatureSlotSource::Market {
                field: MarketField::Close,
            },
        });
        assert_eq!(
            strategy_architecture(&strategy),
            Some(StrategyArchitecture::Hybrid)
        );
        let FeatureSlotSource::Signal { horizon_bars, .. } = &mut strategy.feature_slots[0].source
        else {
            unreachable!()
        };
        *horizon_bars = 0;
        assert!(validate_manifest(&strategy, wasm).is_err());
    }

    #[test]
    fn feature_slot_names_enforce_the_frozen_identifier_contract() {
        let wasm = b"\0asm\x0d\0\x01\0";
        for names in [
            vec!["Close"],
            vec!["close_value"],
            vec!["close-"],
            vec!["close", "close"],
            vec!["é"],
            vec!["a2345678901234567890123456789012345678901234567890123456789012345"],
        ] {
            let mut manifest = strategy_manifest();
            manifest.kind = ComponentKind::Strategy;
            manifest.output_names.clear();
            manifest.feature_slots = names
                .into_iter()
                .map(|name| FeatureSlotDefinition {
                    name: name.into(),
                    source: FeatureSlotSource::Market {
                        field: MarketField::Close,
                    },
                })
                .collect();
            manifest.wasm_sha256 = sha256(wasm);
            assert!(validate_manifest(&manifest, wasm).is_err());
        }
    }

    #[test]
    fn external_slots_require_declared_and_used_dependencies() {
        let wasm = b"\0asm\x0d\0\x01\0";
        let mut strategy = strategy_manifest();
        strategy.kind = ComponentKind::Strategy;
        strategy.output_names.clear();
        strategy.feature_slots = vec![FeatureSlotDefinition {
            name: "momentum".into(),
            source: FeatureSlotSource::External {
                dependency_alias: "momentum".into(),
                output: "close-momentum-5".into(),
            },
        }];
        strategy.dependencies = vec![ComponentDependency {
            component_id: Uuid::nil(),
            version: VersionReq::STAR,
            alias: "momentum".into(),
        }];
        strategy.wasm_sha256 = sha256(wasm);
        assert!(validate_manifest(&strategy, wasm).is_ok());

        strategy.feature_slots[0].source = FeatureSlotSource::External {
            dependency_alias: "missing".into(),
            output: "close-momentum-5".into(),
        };
        assert!(validate_manifest(&strategy, wasm).is_err());
    }

    #[test]
    fn stable_contract_changes_require_the_semver_bump_confirmed_by_adr_0019() {
        let previous = ComponentManifest {
            version: Version::new(1, 0, 0),
            output_names: vec!["value".into()],
            ..manifest()
        };
        let mut added_output = previous.clone();
        added_output.version = Version::new(1, 0, 1);
        added_output.output_names.push("signal".into());
        assert!(check_manifest_compatibility(&previous, &added_output).is_err());
        added_output.version = Version::new(1, 1, 0);
        assert!(check_manifest_compatibility(&previous, &added_output).is_ok());

        let mut reordered_output = previous.clone();
        reordered_output.version = Version::new(1, 1, 0);
        reordered_output.output_names = vec!["signal".into()];
        assert!(check_manifest_compatibility(&previous, &reordered_output).is_err());
        reordered_output.version = Version::new(2, 0, 0);
        assert!(check_manifest_compatibility(&previous, &reordered_output).is_ok());
    }

    #[test]
    fn strategy_manifests_cannot_treat_outputs_as_a_minor_capability() {
        let wasm = b"\0asm\x0d\0\x01\0";
        let mut strategy = strategy_manifest();
        strategy.kind = ComponentKind::Strategy;
        strategy.feature_slots = vec![FeatureSlotDefinition {
            name: "close".into(),
            source: FeatureSlotSource::Market {
                field: MarketField::Close,
            },
        }];
        strategy.wasm_sha256 = sha256(wasm);
        assert!(validate_manifest(&strategy, wasm).is_err());
    }

    #[test]
    fn model_contract_requires_a_valid_aligned_forecast_definition() {
        let wasm = b"\0asm\x0d\0\x01\0";
        let mut model = strategy_manifest();
        model.kind = ComponentKind::Model;
        model.output_names.clear();
        model.feature_slots = vec![FeatureSlotDefinition {
            name: "close".into(),
            source: FeatureSlotSource::Market {
                field: MarketField::Close,
            },
        }];
        model.model_scope = Some(ModelScope::SingleInstrument);
        model.model_artifact = Some(ModelArtifact {
            sha256: sha256(wasm),
            provenance: BTreeMap::new(),
        });
        model.model_outputs = vec![ModelOutput {
            name: "close-up".into(),
            prediction_kind: PredictionKind::Probability,
            forecast_target: ForecastTarget::Builtin {
                target: BuiltinForecastTarget::FutureCloseUp,
            },
            value_scale: ForecastValueScale::Probability,
            horizon_bars: 1,
        }];
        model.wasm_sha256 = sha256(wasm);
        assert!(validate_manifest(&model, wasm).is_ok());
        model.model_outputs[0].forecast_target = ForecastTarget::Builtin {
            target: BuiltinForecastTarget::FutureCloseReturn,
        };
        assert_eq!(
            validate_manifest(&model, wasm).unwrap_err().0,
            "Probability requires a Binary Forecast Target"
        );
    }

    #[test]
    fn score_contract_requires_a_stable_declared_scale() {
        let mut output = ModelOutput {
            name: "score".into(),
            prediction_kind: PredictionKind::Score,
            forecast_target: ForecastTarget::Builtin {
                target: BuiltinForecastTarget::FutureCloseUp,
            },
            value_scale: ForecastValueScale::Percentile,
            horizon_bars: 1,
        };
        assert!(validate_model_outputs(&[output.clone()]).is_ok());
        output.forecast_target = ForecastTarget::Builtin {
            target: BuiltinForecastTarget::FutureCloseReturn,
        };
        output.value_scale = ForecastValueScale::ZScore {
            method: "training-zscore-v1".into(),
        };
        assert!(validate_model_outputs(&[output.clone()]).is_ok());
        output.value_scale = ForecastValueScale::ZScore { method: " ".into() };
        assert_eq!(
            validate_model_outputs(&[output.clone()]).unwrap_err().0,
            "Z-score Forecast Value Scale requires a method"
        );
        output.value_scale = ForecastValueScale::Native;
        assert_eq!(
            validate_model_outputs(&[output]).unwrap_err().0,
            "Score requires Percentile, Z-score, or Custom Forecast Value Scale"
        );
    }
}
