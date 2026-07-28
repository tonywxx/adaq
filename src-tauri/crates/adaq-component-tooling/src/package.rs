use std::{
    fmt::Write as _,
    io::{Cursor, Read, Write},
};

use rust_decimal::Decimal;
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use wasmparser::{Encoding, Parser, Payload, Validator};
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

const MANIFEST_NAME: &str = "manifest.json";
const COMPONENT_NAME: &str = "component.wasm";
const MAX_PACKAGE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ComponentKind {
    Factor,
    Strategy,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ComponentManifest {
    pub manifest_schema_version: Version,
    pub component_id: Uuid,
    pub version: Version,
    pub name: String,
    pub kind: ComponentKind,
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FeatureSlotDefinition {
    pub name: String,
    pub source: FeatureSlotSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum FeatureSlotSource {
    Market {
        field: MarketField,
    },
    #[serde(rename = "builtin")]
    BuiltIn {
        indicator: String,
        output: String,
    },
    External {
        dependency_alias: String,
        output: String,
    },
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

#[derive(Debug, Clone, PartialEq, Eq)]
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
    validate_manifest(&manifest, wasm)?;
    let manifest_json = serde_json::to_vec_pretty(&manifest).map_err(error)?;
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644);
    writer.start_file(MANIFEST_NAME, options).map_err(error)?;
    writer.write_all(&manifest_json).map_err(error)?;
    writer.start_file(COMPONENT_NAME, options).map_err(error)?;
    writer.write_all(wasm).map_err(error)?;
    Ok(writer.finish().map_err(error)?.into_inner())
}

fn validate_manifest(manifest: &ComponentManifest, wasm: &[u8]) -> Result<(), PackageError> {
    if manifest.name.trim().is_empty()
        || manifest.manifest_schema_version != Version::new(1, 0, 0)
        || manifest.sdk_version != Version::parse(adaq_component_sdk::SDK_VERSION).unwrap()
        || manifest.abi_version != Version::parse(adaq_component_sdk::ABI_VERSION).unwrap()
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
        ComponentKind::Factor if !manifest.feature_slots.is_empty() => {
            return Err(PackageError(
                "Factor manifests cannot declare Feature Slots".into(),
            ));
        }
        ComponentKind::Strategy if manifest.feature_slots.is_empty() => {
            return Err(PackageError(
                "Strategy manifests must declare Feature Slots".into(),
            ));
        }
        _ => {}
    }
    unique_identifiers(
        manifest.feature_slots.iter().map(|slot| slot.name.as_str()),
        "Feature Slot",
    )?;
    unique_non_empty(manifest.output_names.iter().map(String::as_str), "output")?;
    unique_non_empty(
        manifest
            .dependencies
            .iter()
            .map(|value| value.alias.as_str()),
        "dependency",
    )?;
    for parameter in &manifest.parameters {
        let valid_default = match parameter.parameter_type {
            ParameterType::Decimal => Decimal::from_str_exact(&parameter.default_value).is_ok(),
            ParameterType::Integer => parameter.default_value.parse::<i64>().is_ok(),
            ParameterType::Boolean => parameter.default_value.parse::<bool>().is_ok(),
            ParameterType::String => true,
        };
        if !parameter.allowed_values.is_empty()
            && (parameter.parameter_type != ParameterType::String
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
            sdk_version: Version::parse(adaq_component_sdk::SDK_VERSION).unwrap(),
            abi_version: Version::new(1, 0, 0),
            wasm_sha256: String::new(),
            parameters: vec![],
            feature_slots: vec![],
            output_names: vec!["value".into()],
            dependencies: vec![],
            warmup_bars: 0,
        }
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
            let mut manifest = manifest();
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
}
