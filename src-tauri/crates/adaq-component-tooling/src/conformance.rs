use std::collections::HashMap;

use adaq_component_sdk::host::{factor_abi, model_abi, strategy_abi};
use rust_decimal::Decimal;

use crate::{
    ComponentKind, ComponentManifest, ComponentPackage, ComponentParameterValue, FeatureSlotSource,
    MarketField, ParameterType, WasmLoader, native_engine_identity, validate_and_freeze,
};

pub fn verify_package(package: &ComponentPackage) -> Result<(), String> {
    let parameters = component_parameters(&package.manifest, None)?;
    match package.manifest.kind {
        ComponentKind::Factor => verify_factor(package, &parameters),
        ComponentKind::Strategy => verify_strategy(package, &parameters),
        ComponentKind::Model => verify_model(package, &parameters),
    }
}

fn verify_model(package: &ComponentPackage, parameters: &[ComponentParameterValue]) -> Result<(), String> {
    let slots = package.manifest.feature_slots.iter().map(|slot| model_abi::exports::adaq::model::api::FeatureSlot { name: slot.name.clone() }).collect::<Vec<_>>();
    let rows = (0..7).map(|index| model_abi::exports::adaq::model::api::PredictionRow {
        instrument_id: "BTC-USDT".into(), prediction_time_ms: index, values: vec![index as f64; slots.len()],
    }).collect::<Vec<_>>();
    let run = |chunks: &[usize]| -> Result<_, String> {
        let loader = WasmLoader::default();
        loader.load_model_bytes(&package.wasm, slots.clone(), parameters, 7)?;
        let mut output = Vec::new(); let mut start = 0;
        for end in chunks { output.extend(loader.process_model(rows[start..*end].to_vec())?); start = *end; }
        Ok(output)
    };
    let whole = run(&[rows.len()])?;
    let replay = run(&[rows.len()])?;
    let chunked = run(&[3, rows.len()])?;
    if !model_results_equal(&whole, &replay) || !model_results_equal(&whole, &chunked) || whole.len() != rows.len() { return Err("Model is not deterministic or chunk-boundary independent".into()); }
    for (input, output) in rows.iter().zip(&whole) {
        if let Some(output) = output {
            if output.instrument_id != input.instrument_id || output.prediction_time_ms != input.prediction_time_ms || output.values.len() != package.manifest.model_outputs.len() || output.values.iter().any(|value| !value.is_finite()) {
                return Err("Model output does not preserve row identity, order, or finite output contract".into());
            }
        }
    }
    Ok(())
}

fn model_results_equal(
    left: &[Option<model_abi::exports::adaq::model::api::ForecastRow>],
    right: &[Option<model_abi::exports::adaq::model::api::ForecastRow>],
) -> bool {
    left.len() == right.len() && left.iter().zip(right).all(|(left, right)| match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => left.instrument_id == right.instrument_id
            && left.prediction_time_ms == right.prediction_time_ms
            && left.values.len() == right.values.len()
            && left.values.iter().zip(&right.values).all(|(left, right)| left.to_bits() == right.to_bits()),
        _ => false,
    })
}

pub fn component_parameters(
    manifest: &ComponentManifest,
    overrides: Option<&HashMap<String, String>>,
) -> Result<Vec<ComponentParameterValue>, String> {
    manifest
        .parameters
        .iter()
        .map(|parameter| {
            let value = overrides
                .and_then(|values| values.get(&parameter.name))
                .unwrap_or(&parameter.default_value);
            if !parameter.allowed_values.is_empty() && !parameter.allowed_values.contains(value) {
                return Err(format!(
                    "Parameter {} is not an allowed value",
                    parameter.name
                ));
            }
            match parameter.parameter_type {
                ParameterType::Decimal => {
                    Decimal::from_str_exact(value).map_err(string)?;
                    Ok(ComponentParameterValue::Decimal(value.clone()))
                }
                ParameterType::Integer => value
                    .parse()
                    .map(ComponentParameterValue::Integer)
                    .map_err(string),
                ParameterType::Boolean => value
                    .parse()
                    .map(ComponentParameterValue::Boolean)
                    .map_err(string),
                ParameterType::String => Ok(ComponentParameterValue::String(value.clone())),
            }
        })
        .collect()
}

fn verify_factor(
    package: &ComponentPackage,
    parameters: &[ComponentParameterValue],
) -> Result<(), String> {
    let loader = WasmLoader::default();
    loader.load_factor_bytes(&package.wasm, parameters)?;
    let schema = loader.describe_factor()?;
    if schema.output_names != package.manifest.output_names
        || schema.warmup_bars != package.manifest.warmup_bars
    {
        return Err("Factor runtime schema does not match manifest".into());
    }
    let bars = ["100", "101", "99", "102", "103", "104", "105"]
        .into_iter()
        .enumerate()
        .map(
            |(index, close)| factor_abi::exports::adaq::factor::api::ClosedBar {
                open_time_ms: index as i64,
                open: close.into(),
                high: close.into(),
                low: close.into(),
                close: close.into(),
                base_volume: "1".into(),
                quote_volume: close.into(),
            },
        )
        .collect::<Vec<_>>();
    let whole = loader.process_factor(bars.clone())?;
    let replay = WasmLoader::default();
    replay.load_factor_bytes(&package.wasm, parameters)?;
    if !factor_results_equal(&whole, &replay.process_factor(bars.clone())?) {
        return Err("Factor is not deterministic".into());
    }
    let chunked = WasmLoader::default();
    chunked.load_factor_bytes(&package.wasm, parameters)?;
    let mut chunks = chunked.process_factor(bars[..6].to_vec())?;
    chunks.extend(chunked.process_factor(bars[6..].to_vec())?);
    if !factor_results_equal(&whole, &chunks) {
        return Err("Factor is not chunk-boundary independent".into());
    }
    Ok(())
}

fn verify_strategy(
    package: &ComponentPackage,
    parameters: &[ComponentParameterValue],
) -> Result<(), String> {
    let mut conformance_manifest = package.manifest.clone();
    for slot in &mut conformance_manifest.feature_slots {
        if matches!(slot.source, FeatureSlotSource::External { .. }) {
            slot.source = FeatureSlotSource::Market {
                field: MarketField::Close,
            };
        }
    }
    conformance_manifest.dependencies.clear();
    let plan = validate_and_freeze(
        &conformance_manifest,
        &package.archive_sha256,
        &native_engine_identity().map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("Indicator Plan validation failed: {:?}", error.issues))?;
    let slots = plan
        .slot_names()
        .map(
            |name| strategy_abi::exports::adaq::strategy::api::FeatureSlot {
                name: name.to_owned(),
            },
        )
        .collect::<Vec<_>>();
    let frames = (0..3)
        .map(
            |index| strategy_abi::exports::adaq::strategy::api::FeatureFrame {
                open_time_ms: index,
                values: vec![index as f64; slots.len()],
            },
        )
        .collect::<Vec<_>>();
    let loader = WasmLoader::default();
    loader.load_strategy_bytes(&package.wasm, slots.clone(), parameters)?;
    let targets = loader.process_strategy(frames.clone())?;
    let replay = WasmLoader::default();
    replay.load_strategy_bytes(&package.wasm, slots.clone(), parameters)?;
    if targets != replay.process_strategy(frames.clone())? {
        return Err("Strategy is not deterministic".into());
    }
    let chunked = WasmLoader::default();
    chunked.load_strategy_bytes(&package.wasm, slots, parameters)?;
    let mut chunks = chunked.process_strategy(frames[..1].to_vec())?;
    chunks.extend(chunked.process_strategy(frames[1..].to_vec())?);
    if targets != chunks
        || targets.len() != frames.len()
        || targets.iter().any(|target| {
            Decimal::from_str_exact(target)
                .map(|value| value < Decimal::ZERO || value > Decimal::ONE)
                .unwrap_or(true)
        })
    {
        return Err("Strategy conformance Target Exposure is invalid".into());
    }
    Ok(())
}

fn factor_results_equal(
    left: &[Option<Vec<factor_abi::exports::adaq::factor::api::NamedScalar>>],
    right: &[Option<Vec<factor_abi::exports::adaq::factor::api::NamedScalar>>],
) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| match (left, right) {
                (None, None) => true,
                (Some(left), Some(right)) => {
                    left.len() == right.len()
                        && left.iter().zip(right).all(|(left, right)| {
                            left.name == right.name && left.value.to_bits() == right.value.to_bits()
                        })
                }
                _ => false,
            })
}

fn string(error: impl std::fmt::Display) -> String {
    error.to_string()
}
