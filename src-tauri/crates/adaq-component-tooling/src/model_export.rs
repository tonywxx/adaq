use std::collections::BTreeMap;

use base64::Engine as _;
use semver::Version;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::model_template::MODEL_TEMPLATE_BASE64;
use crate::{
    BuiltinForecastTarget, ComponentKind, ComponentManifest, ComponentPackage,
    FeatureSlotDefinition, FeatureSlotSource, ForecastTarget, ForecastValueScale, ModelArtifact,
    ModelOutput, ModelScope, ParameterDefinition, ParameterType, PredictionKind, pack_component,
};

pub const MODEL_EXPORTER_ID: &str = "adaq:qlib-ridge-wasi-exporter@1";
pub const WASI_MODEL_PROFILE: &str = "adaq:wasi-model@1";
pub const MODEL_OUTPUT_NAME: &str = "forecast";
pub const MODEL_TARGET_ID: &str = "future-close-return";
pub const MODEL_HORIZON_BARS: u32 = 5;

/// Builds the one supported Qlib Ridge Model Component from frozen native
/// values. The component accepts only immutable binding parameters and
/// prediction batches; it has no source-runtime or training capability.
pub fn export_linear_model_component(
    source_artifact_sha256: &str,
    transformation_sha256: &str,
    input_slots: &[String],
    means: &[f64],
    scales: &[f64],
    coefficients: &[f64],
    intercept: f64,
    provenance: BTreeMap<String, String>,
) -> Result<Vec<u8>, String> {
    if !is_sha256(source_artifact_sha256)
        || !is_sha256(transformation_sha256)
        || input_slots.is_empty()
        || input_slots.len() > 64
        || input_slots
            .iter()
            .any(|slot| !crate::package::is_lower_kebab(slot))
        || means.len() != input_slots.len()
        || scales.len() != input_slots.len()
        || coefficients.len() != input_slots.len()
        || means.iter().any(|value| !value.is_finite())
        || scales
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.0)
        || coefficients.iter().any(|value| !value.is_finite())
        || !intercept.is_finite()
    {
        return Err("linear-model-export-input-invalid".into());
    }

    let mut provenance = provenance;
    provenance.insert("sourceArtifactSha256".into(), source_artifact_sha256.into());
    provenance.insert("transformationSha256".into(), transformation_sha256.into());
    provenance.insert("exporter".into(), MODEL_EXPORTER_ID.into());
    provenance.insert("modelProfile".into(), WASI_MODEL_PROFILE.into());

    let binding = linear_model_binding(
        source_artifact_sha256,
        transformation_sha256,
        input_slots,
        means,
        scales,
        coefficients,
        intercept,
    );
    let parameters = vec![fixed_parameter(
        "model-binding",
        ParameterType::String,
        &binding,
    )];

    let feature_slots = input_slots
        .iter()
        .map(|name| FeatureSlotDefinition {
            name: name.clone(),
            // Gate 9's qualification input is Host-owned and exact. A
            // Market source keeps the package consumable by the existing
            // Feature Plan boundary without inventing a new source kind.
            source: FeatureSlotSource::Market {
                field: crate::MarketField::Close,
            },
        })
        .collect();
    let mut identity = Sha256::new();
    identity.update(MODEL_EXPORTER_ID.as_bytes());
    identity.update(WASI_MODEL_PROFILE.as_bytes());
    identity.update(adaq_component_sdk::SDK_VERSION.as_bytes());
    identity.update(adaq_component_sdk::ABI_VERSION.as_bytes());
    identity.update(source_artifact_sha256.as_bytes());
    let mut component_id = [0; 16];
    component_id.copy_from_slice(&identity.finalize()[..16]);
    component_id[6] = (component_id[6] & 0x0f) | 0x40;
    component_id[8] = (component_id[8] & 0x3f) | 0x80;

    let manifest = ComponentManifest {
        manifest_schema_version: Version::new(1, 0, 0),
        component_id: Uuid::from_bytes(component_id),
        // Package provenance is part of the immutable archive, so adding
        // evidence fields requires the patch release mandated by ADR-0019.
        version: Version::new(1, 0, 1),
        name: "Qlib Ridge WASI Model".into(),
        kind: ComponentKind::Model,
        strategy_scope: crate::StrategyScope::SingleInstrument,
        factor_scope: None,
        sdk_version: Version::parse(adaq_component_sdk::SDK_VERSION).map_err(string)?,
        abi_version: Version::parse(adaq_component_sdk::ABI_VERSION).map_err(string)?,
        wasm_sha256: String::new(),
        parameters,
        feature_slots,
        output_names: Vec::new(),
        dependencies: Vec::new(),
        warmup_bars: 0,
        model_scope: Some(ModelScope::SingleInstrument),
        model_outputs: vec![ModelOutput {
            name: MODEL_OUTPUT_NAME.into(),
            prediction_kind: PredictionKind::ExpectedValue,
            forecast_target: ForecastTarget::Builtin {
                target: BuiltinForecastTarget::FutureCloseReturn,
            },
            value_scale: ForecastValueScale::Native,
            horizon_bars: MODEL_HORIZON_BARS,
        }],
        model_artifact: Some(ModelArtifact {
            sha256: String::new(),
            provenance,
        }),
    };
    let wasm = base64::engine::general_purpose::STANDARD
        .decode(MODEL_TEMPLATE_BASE64)
        .map_err(string)?;
    let package = pack_component(manifest, &wasm).map_err(string)?;
    ComponentPackage::read(&package).map_err(string)?;
    Ok(package)
}

pub fn linear_model_binding(
    source_artifact_sha256: &str,
    transformation_sha256: &str,
    input_slots: &[String],
    means: &[f64],
    scales: &[f64],
    coefficients: &[f64],
    intercept: f64,
) -> String {
    format!(
        "adaq:linear-model-binding@1|{source_artifact_sha256}|{transformation_sha256}|{}|{}|{}|{}|{}",
        input_slots.len(),
        values(means),
        values(scales),
        values(coefficients),
        intercept,
    )
}

fn fixed_parameter(name: &str, parameter_type: ParameterType, value: &str) -> ParameterDefinition {
    ParameterDefinition {
        name: name.into(),
        parameter_type,
        default_value: value.into(),
        allowed_values: vec![value.into()],
    }
}

fn values(values: &[f64]) -> String {
    values
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn string(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verify_package;

    #[test]
    fn export_is_deterministic_and_binds_the_source_artifact() {
        let artifact = "a".repeat(64);
        let transformation = "b".repeat(64);
        let provenance = BTreeMap::from([("fixture".into(), "c".repeat(64))]);
        let first = export_linear_model_component(
            &artifact,
            &transformation,
            &["momentum-score".into()],
            &[1.0],
            &[2.0],
            &[3.0],
            4.0,
            provenance.clone(),
        )
        .unwrap();
        let second = export_linear_model_component(
            &artifact,
            &transformation,
            &["momentum-score".into()],
            &[1.0],
            &[2.0],
            &[3.0],
            4.0,
            provenance,
        )
        .unwrap();
        assert_eq!(first, second);
        let package = ComponentPackage::read(&first).unwrap();
        verify_package(&package).unwrap();
        assert_eq!(package.manifest.name, "Qlib Ridge WASI Model");
        assert_eq!(package.manifest.version, Version::new(1, 0, 1));
        assert_eq!(
            package.manifest.model_artifact.as_ref().unwrap().provenance["sourceArtifactSha256"],
            artifact
        );
    }
}
