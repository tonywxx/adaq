use std::{collections::HashSet, fs, path::Path, sync::Mutex};

use adaq_component_sdk::host::{
    factor_cross_sectional_abi, factor_time_series_abi, model_abi, strategy_abi,
};
use wasmtime::{
    Config, Engine, Store, StoreLimits, StoreLimitsBuilder,
    component::{Component, Linker, ResourceAny},
};

const MAX_COMPONENT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct RunLimits {
    pub fuel_per_call: u64,
    pub memory_bytes: usize,
    pub max_bars: usize,
}

impl Default for RunLimits {
    fn default() -> Self {
        Self {
            fuel_per_call: 10_000_000,
            memory_bytes: 64 * 1024 * 1024,
            max_bars: 1_000_000,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FactorSchema {
    pub scope: crate::FactorScope,
    pub schema_version: String,
    pub feature_slots: Vec<String>,
    pub parameter_names: Vec<String>,
    pub parameter_definitions: Vec<FactorParameterSchema>,
    pub output_names: Vec<String>,
    pub warmup_bars: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FactorParameterSchema {
    pub name: String,
    pub parameter_type: String,
    pub default_value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "kebab-case")]
pub enum ComponentParameterValue {
    Decimal(String),
    Integer(i64),
    Boolean(bool),
    String(String),
}

struct LoadedFactor {
    scope: crate::FactorScope,
    schema: FactorSchema,
    feature_slot_count: usize,
    rows_seen: usize,
    loaded: LoadedFactorWorld,
}

enum LoadedFactorWorld {
    TimeSeries {
        store: Store<ComponentStore>,
        bindings: factor_time_series_abi::TimeSeries,
        instance: ResourceAny,
    },
    CrossSectional {
        store: Store<ComponentStore>,
        bindings: factor_cross_sectional_abi::CrossSectional,
        instance: ResourceAny,
    },
}

struct LoadedStrategy {
    store: Store<ComponentStore>,
    bindings: strategy_abi::Strategy,
    instance: ResourceAny,
}

struct LoadedModel {
    store: Store<ComponentStore>,
    bindings: model_abi::Model,
    instance: ResourceAny,
}

#[derive(Default)]
pub struct WasmLoader {
    factor: Mutex<Option<LoadedFactor>>,
    strategy: Mutex<Option<LoadedStrategy>>,
    model: Mutex<Option<LoadedModel>>,
    limits: RunLimits,
}

impl WasmLoader {
    pub fn with_limits(limits: RunLimits) -> Self {
        Self {
            factor: Mutex::default(),
            strategy: Mutex::default(),
            model: Mutex::default(),
            limits,
        }
    }

    pub fn load(&self, path: &str) -> Result<(), String> {
        self.load_with_parameters(path, &[])
    }

    pub fn load_with_parameters(
        &self,
        path: &str,
        parameters: &[ComponentParameterValue],
    ) -> Result<(), String> {
        if !Path::new(path).is_file() {
            return Err(format!("Factor component does not exist: {path}"));
        }
        if fs::metadata(path).map_err(string)?.len() > MAX_COMPONENT_BYTES as u64 {
            return Err("Factor component exceeds the 64 MiB component limit".into());
        }
        self.load_factor_bytes(&fs::read(path).map_err(string)?, parameters)
    }

    pub fn load_factor_bytes(
        &self,
        wasm: &[u8],
        parameters: &[ComponentParameterValue],
    ) -> Result<(), String> {
        ensure_component_size(wasm)?;
        match self.load_factor_time_series_bytes(wasm, Vec::new(), parameters) {
            Ok(()) => Ok(()),
            Err(time_series_error) => self
                .load_factor_cross_sectional_bytes(wasm, Vec::new(), parameters)
                .map_err(|cross_sectional_error| {
                    format!(
                        "Factor does not export a valid ABI v2 world: time-series: {time_series_error}; cross-sectional: {cross_sectional_error}"
                    )
                }),
        }
    }

    pub fn load_factor_time_series_bytes(
        &self,
        wasm: &[u8],
        feature_slots: Vec<
            factor_time_series_abi::exports::adaq::factor::time_series_api::FeatureSlot,
        >,
        parameters: &[ComponentParameterValue],
    ) -> Result<(), String> {
        ensure_component_size(wasm)?;
        let engine = component_engine()?;
        let component = Component::new(&engine, wasm).map_err(string)?;
        let linker = Linker::new(&engine);
        let mut store = component_store(&engine, self.limits)?;
        let bindings =
            factor_time_series_abi::TimeSeries::instantiate(&mut store, &component, &linker)
                .map_err(string)?;

        reset_component_fuel(&mut store, self.limits)?;
        let schema = bindings
            .adaq_factor_time_series_api()
            .call_describe(&mut store)
            .map_err(string)?
            .map_err(|error| format!("Factor describe failed: {error}"))?;
        let feature_slots = if feature_slots.is_empty() {
            schema
                .feature_slots
                .iter()
                .map(|slot| {
                    factor_time_series_abi::exports::adaq::factor::time_series_api::FeatureSlot {
                        name: slot.name.clone(),
                    }
                })
                .collect::<Vec<_>>()
        } else {
            feature_slots
        };
        let schema = time_series_factor_schema(schema, &feature_slots)?;
        reset_component_fuel(&mut store, self.limits)?;
        let instance = bindings
            .adaq_factor_time_series_api()
            .call_create(
                &mut store,
                &feature_slots,
                &parameters.iter().map(factor_parameter).collect::<Vec<_>>(),
            )
            .map_err(string)?
            .map_err(|error| format!("Factor create failed: {error}"))?;

        let mut factor = self.factor.lock().map_err(string)?;
        let previous = factor.replace(LoadedFactor {
            scope: crate::FactorScope::TimeSeries,
            feature_slot_count: feature_slots.len(),
            rows_seen: 0,
            schema,
            loaded: LoadedFactorWorld::TimeSeries {
                store,
                bindings,
                instance,
            },
        });
        drop_loaded_factor(previous, self.limits)?;
        Ok(())
    }

    pub fn load_factor_cross_sectional_bytes(
        &self,
        wasm: &[u8],
        feature_slots: Vec<
            factor_cross_sectional_abi::exports::adaq::factor::cross_sectional_api::FeatureSlot,
        >,
        parameters: &[ComponentParameterValue],
    ) -> Result<(), String> {
        ensure_component_size(wasm)?;
        let engine = component_engine()?;
        let component = Component::new(&engine, wasm).map_err(string)?;
        let linker = Linker::new(&engine);
        let mut store = component_store(&engine, self.limits)?;
        let bindings = factor_cross_sectional_abi::CrossSectional::instantiate(
            &mut store, &component, &linker,
        )
        .map_err(string)?;

        reset_component_fuel(&mut store, self.limits)?;
        let schema = bindings
            .adaq_factor_cross_sectional_api()
            .call_describe(&mut store)
            .map_err(string)?
            .map_err(|error| format!("Factor describe failed: {error}"))?;
        let feature_slots = if feature_slots.is_empty() {
            schema
                .feature_slots
                .iter()
                .map(|slot| {
                    factor_cross_sectional_abi::exports::adaq::factor::cross_sectional_api::FeatureSlot {
                        name: slot.name.clone(),
                    }
                })
                .collect::<Vec<_>>()
        } else {
            feature_slots
        };
        let schema = cross_sectional_factor_schema(schema, &feature_slots)?;
        reset_component_fuel(&mut store, self.limits)?;
        let instance = bindings
            .adaq_factor_cross_sectional_api()
            .call_create(
                &mut store,
                &feature_slots,
                &parameters
                    .iter()
                    .map(cross_sectional_factor_parameter)
                    .collect::<Vec<_>>(),
            )
            .map_err(string)?
            .map_err(|error| format!("Factor create failed: {error}"))?;

        let mut factor = self.factor.lock().map_err(string)?;
        let previous = factor.replace(LoadedFactor {
            scope: crate::FactorScope::CrossSectional,
            feature_slot_count: feature_slots.len(),
            rows_seen: 0,
            schema,
            loaded: LoadedFactorWorld::CrossSectional {
                store,
                bindings,
                instance,
            },
        });
        drop_loaded_factor(previous, self.limits)?;
        Ok(())
    }

    pub fn describe_factor(&self) -> Result<FactorSchema, String> {
        let factor = self.factor.lock().map_err(string)?;
        factor
            .as_ref()
            .map(|loaded| loaded.schema.clone())
            .ok_or_else(|| "Factor component is not loaded".to_owned())
    }

    pub fn process_factor(
        &self,
        rows: Vec<factor_time_series_abi::exports::adaq::factor::time_series_api::TimeSeriesRow>,
    ) -> Result<
        Vec<factor_time_series_abi::exports::adaq::factor::time_series_api::FactorResult>,
        String,
    > {
        let mut factor = self.factor.lock().map_err(string)?;
        let loaded = factor
            .as_mut()
            .ok_or_else(|| "Factor component is not loaded".to_owned())?;
        if loaded.scope != crate::FactorScope::TimeSeries {
            return Err("Loaded Factor scope is not time-series".into());
        }
        if rows.len() > self.limits.max_bars {
            return Err(format!(
                "Factor process exceeds the {} row limit",
                self.limits.max_bars
            ));
        }
        validate_time_series_rows(&rows, loaded.feature_slot_count)?;
        let expected_outputs = loaded.schema.output_names.clone();
        let rows_seen = loaded.rows_seen;
        let warmup_bars = loaded.schema.warmup_bars;
        match &mut loaded.loaded {
            LoadedFactorWorld::TimeSeries {
                store,
                bindings,
                instance,
            } => {
                reset_component_fuel(store, self.limits)?;
                let results = bindings
                    .adaq_factor_time_series_api()
                    .instance()
                    .call_process(store, *instance, &rows)
                    .map_err(string)?
                    .map_err(|error| format!("Factor process failed: {error}"))?;
                validate_time_series_results(
                    &rows,
                    &results,
                    &expected_outputs,
                    warmup_bars,
                    rows_seen,
                )?;
                loaded.rows_seen = rows_seen.saturating_add(rows.len());
                Ok(results)
            }
            LoadedFactorWorld::CrossSectional { .. } => unreachable!(),
        }
    }

    pub fn process_cross_sectional_factor(
        &self,
        rows: Vec<factor_cross_sectional_abi::exports::adaq::factor::cross_sectional_api::CrossSectionalRow>,
        expected_instrument_ids: &[String],
    ) -> Result<
        Vec<factor_cross_sectional_abi::exports::adaq::factor::cross_sectional_api::FactorResult>,
        String,
    > {
        let mut factor = self.factor.lock().map_err(string)?;
        let loaded = factor
            .as_mut()
            .ok_or_else(|| "Factor component is not loaded".to_owned())?;
        if loaded.scope != crate::FactorScope::CrossSectional {
            return Err("Loaded Factor scope is not cross-sectional".into());
        }
        if rows.len() > self.limits.max_bars {
            return Err(format!(
                "Factor process exceeds the {} row limit",
                self.limits.max_bars
            ));
        }
        validate_cross_sectional_rows(&rows, expected_instrument_ids, loaded.feature_slot_count)?;
        let expected_outputs = loaded.schema.output_names.clone();
        let rows_seen = loaded.rows_seen;
        let warmup_bars = loaded.schema.warmup_bars;
        match &mut loaded.loaded {
            LoadedFactorWorld::CrossSectional {
                store,
                bindings,
                instance,
            } => {
                reset_component_fuel(store, self.limits)?;
                let results = bindings
                    .adaq_factor_cross_sectional_api()
                    .instance()
                    .call_process(store, *instance, &rows)
                    .map_err(string)?
                    .map_err(|error| format!("Factor process failed: {error}"))?;
                validate_cross_sectional_results(
                    &rows,
                    &results,
                    &expected_outputs,
                    warmup_bars,
                    rows_seen,
                )?;
                loaded.rows_seen = rows_seen.saturating_add(1);
                Ok(results)
            }
            LoadedFactorWorld::TimeSeries { .. } => unreachable!(),
        }
    }

    pub fn load_strategy(
        &self,
        path: &str,
        feature_slots: Vec<strategy_abi::exports::adaq::strategy::api::FeatureSlot>,
    ) -> Result<(), String> {
        self.load_strategy_with_parameters(path, feature_slots, &[])
    }

    pub fn load_strategy_with_parameters(
        &self,
        path: &str,
        feature_slots: Vec<strategy_abi::exports::adaq::strategy::api::FeatureSlot>,
        parameters: &[ComponentParameterValue],
    ) -> Result<(), String> {
        if !Path::new(path).is_file() {
            return Err(format!("Strategy component does not exist: {path}"));
        }
        self.load_strategy_bytes(&fs::read(path).map_err(string)?, feature_slots, parameters)
    }

    pub fn load_strategy_bytes(
        &self,
        wasm: &[u8],
        feature_slots: Vec<strategy_abi::exports::adaq::strategy::api::FeatureSlot>,
        parameters: &[ComponentParameterValue],
    ) -> Result<(), String> {
        let engine = component_engine()?;
        let component = Component::new(&engine, wasm).map_err(string)?;
        let linker = Linker::new(&engine);
        let mut store = component_store(&engine, self.limits)?;
        let bindings =
            strategy_abi::Strategy::instantiate(&mut store, &component, &linker).map_err(string)?;
        reset_component_fuel(&mut store, self.limits)?;
        let instance = bindings
            .adaq_strategy_api()
            .call_create(
                &mut store,
                &feature_slots,
                &parameters
                    .iter()
                    .map(strategy_parameter)
                    .collect::<Vec<_>>(),
            )
            .map_err(string)?
            .map_err(|error| format!("Strategy create failed: {error}"))?;

        let mut strategy = self.strategy.lock().map_err(string)?;
        if let Some(mut previous) = strategy.replace(LoadedStrategy {
            store,
            bindings,
            instance,
        }) {
            reset_component_fuel(&mut previous.store, self.limits)?;
            previous
                .instance
                .resource_drop(&mut previous.store)
                .map_err(string)?;
        }
        Ok(())
    }

    pub fn process_strategy(
        &self,
        frames: Vec<strategy_abi::exports::adaq::strategy::api::FeatureFrame>,
    ) -> Result<Vec<String>, String> {
        let mut strategy = self.strategy.lock().map_err(string)?;
        let LoadedStrategy {
            store,
            bindings,
            instance,
        } = strategy
            .as_mut()
            .ok_or_else(|| "Strategy component is not loaded".to_owned())?;
        reset_component_fuel(store, self.limits)?;
        bindings
            .adaq_strategy_api()
            .instance()
            .call_process(store, *instance, &frames)
            .map_err(string)?
            .map_err(|error| format!("Strategy process failed: {error}"))
    }

    pub fn load_model_bytes(
        &self,
        wasm: &[u8],
        feature_slots: Vec<model_abi::exports::adaq::model::api::FeatureSlot>,
        parameters: &[ComponentParameterValue],
        seed: u64,
    ) -> Result<(), String> {
        let engine = component_engine()?;
        let component = Component::new(&engine, wasm).map_err(string)?;
        let linker = Linker::new(&engine);
        let mut store = component_store(&engine, self.limits)?;
        let bindings =
            model_abi::Model::instantiate(&mut store, &component, &linker).map_err(string)?;
        reset_component_fuel(&mut store, self.limits)?;
        let instance = bindings
            .adaq_model_api()
            .call_create(
                &mut store,
                &feature_slots,
                &parameters.iter().map(model_parameter).collect::<Vec<_>>(),
                seed,
            )
            .map_err(string)?
            .map_err(|error| format!("Model create failed: {error}"))?;
        let mut model = self.model.lock().map_err(string)?;
        if let Some(mut previous) = model.replace(LoadedModel {
            store,
            bindings,
            instance,
        }) {
            reset_component_fuel(&mut previous.store, self.limits)?;
            previous
                .instance
                .resource_drop(&mut previous.store)
                .map_err(string)?;
        }
        Ok(())
    }

    pub fn process_model(
        &self,
        rows: Vec<model_abi::exports::adaq::model::api::PredictionRow>,
    ) -> Result<Vec<Option<model_abi::exports::adaq::model::api::ForecastRow>>, String> {
        validate_model_batch_size(rows.len(), self.limits)?;
        let mut model = self.model.lock().map_err(string)?;
        let LoadedModel {
            store,
            bindings,
            instance,
        } = model
            .as_mut()
            .ok_or_else(|| "Model component is not loaded".to_owned())?;
        reset_component_fuel(store, self.limits)?;
        bindings
            .adaq_model_api()
            .instance()
            .call_process(store, *instance, &rows)
            .map_err(string)?
            .map_err(|error| format!("Model process failed: {error}"))
    }
}

fn time_series_factor_schema(
    schema: factor_time_series_abi::exports::adaq::factor::time_series_api::FactorSchema,
    feature_slots: &[factor_time_series_abi::exports::adaq::factor::time_series_api::FeatureSlot],
) -> Result<FactorSchema, String> {
    if !matches!(
        schema.scope,
        factor_time_series_abi::exports::adaq::factor::time_series_api::FactorScope::TimeSeries
    ) {
        return Err("Factor world and declared scope do not match".into());
    }
    if schema.schema_version != adaq_component_sdk::FACTOR_SCHEMA_VERSION {
        return Err("Factor schema identity is incompatible with ABI v2".into());
    }
    let parameter_definitions = schema
        .parameters
        .iter()
        .map(time_series_parameter_schema)
        .collect::<Vec<_>>();
    let normalized = factor_schema(
        crate::FactorScope::TimeSeries,
        schema.feature_slots,
        parameter_definitions,
        schema.output_names,
        schema.warmup_bars,
    );
    validate_factor_schema(
        &normalized,
        feature_slots.iter().map(|slot| slot.name.as_str()),
    )?;
    Ok(normalized)
}

fn cross_sectional_factor_schema(
    schema: factor_cross_sectional_abi::exports::adaq::factor::cross_sectional_api::FactorSchema,
    feature_slots: &[factor_cross_sectional_abi::exports::adaq::factor::cross_sectional_api::FeatureSlot],
) -> Result<FactorSchema, String> {
    if !matches!(
        schema.scope,
        factor_cross_sectional_abi::exports::adaq::factor::cross_sectional_api::FactorScope::CrossSectional
    ) {
        return Err("Factor world and declared scope do not match".into());
    }
    if schema.schema_version != adaq_component_sdk::FACTOR_SCHEMA_VERSION {
        return Err("Factor schema identity is incompatible with ABI v2".into());
    }
    let parameter_definitions = schema
        .parameters
        .iter()
        .map(cross_sectional_parameter_schema)
        .collect::<Vec<_>>();
    let normalized = factor_schema(
        crate::FactorScope::CrossSectional,
        schema.feature_slots,
        parameter_definitions,
        schema.output_names,
        schema.warmup_bars,
    );
    validate_factor_schema(
        &normalized,
        feature_slots.iter().map(|slot| slot.name.as_str()),
    )?;
    Ok(normalized)
}

fn validate_factor_schema<'a>(
    schema: &FactorSchema,
    expected_feature_slots: impl IntoIterator<Item = &'a str>,
) -> Result<(), String> {
    let expected_feature_slots = expected_feature_slots.into_iter().collect::<Vec<_>>();
    if schema.feature_slots.len() != expected_feature_slots.len()
        || schema
            .feature_slots
            .iter()
            .zip(expected_feature_slots)
            .any(|(actual, expected)| actual != expected)
        || schema.feature_slots.is_empty()
        || schema.feature_slots.len() > 64
        || schema.output_names.is_empty()
        || schema.output_names.len() > 64
        || schema
            .output_names
            .iter()
            .any(|name| !crate::package::is_lower_kebab(name))
        || schema
            .parameter_definitions
            .iter()
            .any(|parameter| !crate::package::is_lower_kebab(&parameter.name))
    {
        return Err("Factor schema does not match its host binding or limits".into());
    }
    let mut names = HashSet::new();
    if !schema.feature_slots.iter().all(|name| names.insert(name)) {
        return Err("Factor schema Feature Slot identities must be unique".into());
    }
    let mut names = HashSet::new();
    if !schema.output_names.iter().all(|name| names.insert(name)) {
        return Err("Factor schema output identities must be unique".into());
    }
    let mut names = HashSet::new();
    if !schema.parameter_names.iter().all(|name| names.insert(name)) {
        return Err("Factor schema parameter identities must be unique".into());
    }
    Ok(())
}

fn validate_time_series_rows(
    rows: &[factor_time_series_abi::exports::adaq::factor::time_series_api::TimeSeriesRow],
    expected_slot_count: usize,
) -> Result<(), String> {
    let mut instrument_id = None;
    let mut previous_time = None;
    for row in rows {
        if row.instrument_id.is_empty()
            || instrument_id
                .as_ref()
                .is_some_and(|expected| expected != &row.instrument_id)
            || row.slots.len() != expected_slot_count
            || previous_time.is_some_and(|expected| row.observation_time_ms <= expected)
            || row.slots.iter().any(|value| {
                !value.value.is_finite() || value.available_at_ms > row.observation_time_ms
            })
        {
            return Err("Time-Series Factor input violates identity, order, density, availability, or finite-value constraints".into());
        }
        instrument_id.get_or_insert(row.instrument_id.clone());
        previous_time = Some(row.observation_time_ms);
    }
    Ok(())
}

fn validate_cross_sectional_rows(
    rows: &[factor_cross_sectional_abi::exports::adaq::factor::cross_sectional_api::CrossSectionalRow],
    expected_instrument_ids: &[String],
    expected_slot_count: usize,
) -> Result<(), String> {
    let observation_time = rows
        .first()
        .map(|row| row.observation_time_ms)
        .ok_or_else(|| "Cross-Sectional Factor input requires a complete Universe".to_owned())?;
    if expected_instrument_ids.is_empty() || rows.len() != expected_instrument_ids.len() {
        return Err("Cross-Sectional Factor input requires the complete expected Universe".into());
    }
    let mut members = HashSet::new();
    if expected_instrument_ids
        .iter()
        .any(|instrument_id| instrument_id.is_empty() || !members.insert(instrument_id))
    {
        return Err(
            "Cross-Sectional Factor Universe membership must be unique and non-empty".into(),
        );
    }
    for (row, expected_instrument_id) in rows.iter().zip(expected_instrument_ids) {
        if row.instrument_id.is_empty()
            || row.instrument_id != *expected_instrument_id
            || row.observation_time_ms != observation_time
            || row.slots.len() != expected_slot_count
            || row.slots.iter().any(|cell| match cell {
                factor_cross_sectional_abi::exports::adaq::factor::cross_sectional_api::FeatureCell::Available(value) => {
                    !value.value.is_finite() || value.available_at_ms > observation_time
                }
                factor_cross_sectional_abi::exports::adaq::factor::cross_sectional_api::FeatureCell::Unavailable(_) => false,
            })
        {
            return Err("Cross-Sectional Factor input violates Point-in-Time membership, order, availability, or finite-value constraints".into());
        }
    }
    Ok(())
}

fn validate_time_series_results(
    rows: &[factor_time_series_abi::exports::adaq::factor::time_series_api::TimeSeriesRow],
    results: &[factor_time_series_abi::exports::adaq::factor::time_series_api::FactorResult],
    expected_output_names: &[String],
    warmup_bars: u32,
    rows_seen: usize,
) -> Result<(), String> {
    if rows.len() != results.len() {
        return Err("Factor output row count does not match the input row count".into());
    }
    for (index, (row, result)) in rows.iter().zip(results).enumerate() {
        if row.instrument_id != result.instrument_id
            || row.observation_time_ms != result.observation_time_ms
        {
            return Err(
                "Factor output violates identity, row order, or observation-time constraints"
                    .into(),
            );
        }
        let in_warmup = rows_seen.saturating_add(index) < warmup_bars as usize;
        match (&result.values, in_warmup) {
            (None, true) => {}
            (Some(_), true) => {
                return Err("Time-Series Factor produced output before declared Warmup".into());
            }
            (None, false) => {}
            (Some(values), false) => {
                if values.len() != expected_output_names.len()
                    || values
                        .iter()
                        .zip(expected_output_names)
                        .any(|(value, name)| {
                            value.name != *name
                                || !crate::package::is_lower_kebab(&value.name)
                                || !value.value.is_finite()
                        })
                {
                    return Err("Factor output violates output identity, order, count, or finite-value constraints".into());
                }
            }
        }
    }
    Ok(())
}

fn validate_cross_sectional_results(
    rows: &[factor_cross_sectional_abi::exports::adaq::factor::cross_sectional_api::CrossSectionalRow],
    results: &[factor_cross_sectional_abi::exports::adaq::factor::cross_sectional_api::FactorResult],
    expected_output_names: &[String],
    warmup_bars: u32,
    rows_seen: usize,
) -> Result<(), String> {
    if rows.len() != results.len() {
        return Err("Factor output row count does not match the input row count".into());
    }
    for (row, result) in rows.iter().zip(results) {
        if row.instrument_id != result.instrument_id
            || row.observation_time_ms != result.observation_time_ms
        {
            return Err(
                "Factor output violates identity, membership, order, or observation-time constraints"
                    .into(),
            );
        }
        let unavailable_input = row.slots.iter().any(|cell| {
            matches!(
                cell,
                factor_cross_sectional_abi::exports::adaq::factor::cross_sectional_api::FeatureCell::Unavailable(_)
            )
        });
        let requires_unavailable = rows_seen < warmup_bars as usize || unavailable_input;
        match (&result.values, requires_unavailable) {
            (None, true) => {}
            (None, false) => {
                return Err("Cross-Sectional Factor may omit outputs only for Warmup or typed-unavailable input".into());
            }
            (Some(_), true) => {
                return Err(
                    "Cross-Sectional Factor produced output for Warmup or unavailable input".into(),
                );
            }
            (Some(values), false) => {
                if values.len() != expected_output_names.len()
                    || values
                        .iter()
                        .zip(expected_output_names)
                        .any(|(value, name)| {
                            value.name != *name
                                || !crate::package::is_lower_kebab(&value.name)
                                || !value.value.is_finite()
                        })
                {
                    return Err("Factor output violates output identity, order, count, or finite-value constraints".into());
                }
            }
        }
    }
    Ok(())
}

fn factor_schema(
    scope: crate::FactorScope,
    feature_slots: Vec<impl HasName>,
    parameter_definitions: Vec<FactorParameterSchema>,
    output_names: Vec<String>,
    warmup_bars: u32,
) -> FactorSchema {
    FactorSchema {
        scope,
        schema_version: adaq_component_sdk::FACTOR_SCHEMA_VERSION.into(),
        feature_slots: feature_slots
            .into_iter()
            .map(|slot| slot.name().to_owned())
            .collect(),
        parameter_names: parameter_definitions
            .iter()
            .map(|parameter| parameter.name.clone())
            .collect(),
        parameter_definitions,
        output_names,
        warmup_bars,
    }
}

trait HasName {
    fn name(&self) -> &str;
}

impl HasName for factor_time_series_abi::exports::adaq::factor::time_series_api::FeatureSlot {
    fn name(&self) -> &str {
        &self.name
    }
}

impl HasName
    for factor_cross_sectional_abi::exports::adaq::factor::cross_sectional_api::FeatureSlot
{
    fn name(&self) -> &str {
        &self.name
    }
}

fn time_series_parameter_schema(
    parameter: &factor_time_series_abi::exports::adaq::factor::time_series_api::ParameterDefinition,
) -> FactorParameterSchema {
    FactorParameterSchema {
        name: parameter.name.clone(),
        parameter_type: time_series_parameter_type(parameter.parameter_type).into(),
        default_value: parameter.default_value.clone(),
    }
}

fn cross_sectional_parameter_schema(
    parameter: &factor_cross_sectional_abi::exports::adaq::factor::cross_sectional_api::ParameterDefinition,
) -> FactorParameterSchema {
    FactorParameterSchema {
        name: parameter.name.clone(),
        parameter_type: cross_sectional_parameter_type(parameter.parameter_type).into(),
        default_value: parameter.default_value.clone(),
    }
}

fn time_series_parameter_type(
    parameter_type: factor_time_series_abi::exports::adaq::factor::time_series_api::ParameterType,
) -> &'static str {
    use factor_time_series_abi::exports::adaq::factor::time_series_api::ParameterType;
    match parameter_type {
        ParameterType::Decimal => "decimal",
        ParameterType::Integer => "integer",
        ParameterType::Boolean => "boolean",
        ParameterType::Text => "string",
    }
}

fn cross_sectional_parameter_type(
    parameter_type: factor_cross_sectional_abi::exports::adaq::factor::cross_sectional_api::ParameterType,
) -> &'static str {
    use factor_cross_sectional_abi::exports::adaq::factor::cross_sectional_api::ParameterType;
    match parameter_type {
        ParameterType::Decimal => "decimal",
        ParameterType::Integer => "integer",
        ParameterType::Boolean => "boolean",
        ParameterType::Text => "string",
    }
}

fn ensure_component_size(wasm: &[u8]) -> Result<(), String> {
    if wasm.len() > MAX_COMPONENT_BYTES {
        Err("Factor component exceeds the 64 MiB component limit".into())
    } else {
        Ok(())
    }
}

fn drop_loaded_factor(previous: Option<LoadedFactor>, limits: RunLimits) -> Result<(), String> {
    let Some(previous) = previous else {
        return Ok(());
    };
    match previous.loaded {
        LoadedFactorWorld::TimeSeries {
            mut store,
            instance,
            ..
        } => {
            reset_component_fuel(&mut store, limits)?;
            instance.resource_drop(&mut store).map_err(string)
        }
        LoadedFactorWorld::CrossSectional {
            mut store,
            instance,
            ..
        } => {
            reset_component_fuel(&mut store, limits)?;
            instance.resource_drop(&mut store).map_err(string)
        }
    }
}

struct ComponentStore {
    limits: StoreLimits,
}

fn factor_parameter(
    value: &ComponentParameterValue,
) -> factor_time_series_abi::exports::adaq::factor::time_series_api::ParameterValue {
    use factor_time_series_abi::exports::adaq::factor::time_series_api::ParameterValue;
    match value {
        ComponentParameterValue::Decimal(value) => ParameterValue::Decimal(value.clone()),
        ComponentParameterValue::Integer(value) => ParameterValue::Integer(*value),
        ComponentParameterValue::Boolean(value) => ParameterValue::Boolean(*value),
        ComponentParameterValue::String(value) => ParameterValue::Text(value.clone()),
    }
}

fn cross_sectional_factor_parameter(
    value: &ComponentParameterValue,
) -> factor_cross_sectional_abi::exports::adaq::factor::cross_sectional_api::ParameterValue {
    use factor_cross_sectional_abi::exports::adaq::factor::cross_sectional_api::ParameterValue;
    match value {
        ComponentParameterValue::Decimal(value) => ParameterValue::Decimal(value.clone()),
        ComponentParameterValue::Integer(value) => ParameterValue::Integer(*value),
        ComponentParameterValue::Boolean(value) => ParameterValue::Boolean(*value),
        ComponentParameterValue::String(value) => ParameterValue::Text(value.clone()),
    }
}

fn strategy_parameter(
    value: &ComponentParameterValue,
) -> strategy_abi::exports::adaq::strategy::api::ParameterValue {
    use strategy_abi::exports::adaq::strategy::api::ParameterValue;
    match value {
        ComponentParameterValue::Decimal(value) => ParameterValue::Decimal(value.clone()),
        ComponentParameterValue::Integer(value) => ParameterValue::Integer(*value),
        ComponentParameterValue::Boolean(value) => ParameterValue::Boolean(*value),
        ComponentParameterValue::String(value) => ParameterValue::Text(value.clone()),
    }
}

fn model_parameter(
    value: &ComponentParameterValue,
) -> model_abi::exports::adaq::model::api::ParameterValue {
    use model_abi::exports::adaq::model::api::ParameterValue;
    match value {
        ComponentParameterValue::Decimal(value) => ParameterValue::Decimal(value.clone()),
        ComponentParameterValue::Integer(value) => ParameterValue::Integer(*value),
        ComponentParameterValue::Boolean(value) => ParameterValue::Boolean(*value),
        ComponentParameterValue::String(value) => ParameterValue::Text(value.clone()),
    }
}

fn component_engine() -> Result<Engine, String> {
    let mut config = Config::new();
    config.wasm_component_model(true).consume_fuel(true);
    Engine::new(&config).map_err(string)
}

fn component_store(engine: &Engine, limits: RunLimits) -> Result<Store<ComponentStore>, String> {
    let mut store = Store::new(
        engine,
        ComponentStore {
            limits: StoreLimitsBuilder::new()
                .memory_size(limits.memory_bytes)
                .instances(4)
                .memories(4)
                .tables(4)
                .trap_on_grow_failure(true)
                .build(),
        },
    );
    store.limiter(|state| &mut state.limits);
    reset_component_fuel(&mut store, limits)?;
    Ok(store)
}

fn reset_component_fuel(
    store: &mut Store<ComponentStore>,
    limits: RunLimits,
) -> Result<(), String> {
    store.set_fuel(limits.fuel_per_call).map_err(string)
}

fn validate_model_batch_size(rows: usize, limits: RunLimits) -> Result<(), String> {
    if rows > limits.max_bars {
        return Err(format!(
            "Model batch exceeds the configured bar limit ({})",
            limits.max_bars
        ));
    }
    Ok(())
}

fn string(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::{RunLimits, validate_model_batch_size};

    #[test]
    fn model_batches_are_bounded_by_run_limits() {
        let limits = RunLimits {
            max_bars: 2,
            ..RunLimits::default()
        };
        assert!(validate_model_batch_size(2, limits).is_ok());
        assert!(validate_model_batch_size(3, limits).is_err());
    }
}
