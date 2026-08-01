use std::{fs, path::Path, sync::Mutex};

use adaq_component_sdk::host::{factor_abi, model_abi, strategy_abi};
use wasmtime::{
    Config, Engine, Store, StoreLimits, StoreLimitsBuilder,
    component::{Component, Linker, ResourceAny},
};

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
    pub output_names: Vec<String>,
    pub warmup_bars: u32,
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
    store: Store<ComponentStore>,
    bindings: factor_abi::Factor,
    instance: ResourceAny,
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
        self.load_factor_bytes(&fs::read(path).map_err(string)?, parameters)
    }

    pub fn load_factor_bytes(
        &self,
        wasm: &[u8],
        parameters: &[ComponentParameterValue],
    ) -> Result<(), String> {
        let engine = component_engine()?;
        let component = Component::new(&engine, wasm).map_err(string)?;
        let linker = Linker::new(&engine);
        let mut store = component_store(&engine, self.limits)?;
        let bindings =
            factor_abi::Factor::instantiate(&mut store, &component, &linker).map_err(string)?;

        reset_component_fuel(&mut store, self.limits)?;
        let instance = bindings
            .adaq_factor_api()
            .call_create(
                &mut store,
                &parameters.iter().map(factor_parameter).collect::<Vec<_>>(),
            )
            .map_err(string)?
            .map_err(|error| format!("Factor create failed: {error}"))?;

        let mut factor = self.factor.lock().map_err(string)?;
        if let Some(mut previous) = factor.replace(LoadedFactor {
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

    pub fn describe_factor(&self) -> Result<FactorSchema, String> {
        let mut factor = self.factor.lock().map_err(string)?;
        let LoadedFactor {
            store, bindings, ..
        } = factor
            .as_mut()
            .ok_or_else(|| "Factor component is not loaded".to_owned())?;
        reset_component_fuel(store, self.limits)?;
        let schema = bindings
            .adaq_factor_api()
            .call_describe(store)
            .map_err(string)?
            .map_err(|error| format!("Factor describe failed: {error}"))?;
        Ok(FactorSchema {
            output_names: schema.output_names,
            warmup_bars: schema.warmup_bars,
        })
    }

    pub fn process_factor(
        &self,
        bars: Vec<factor_abi::exports::adaq::factor::api::ClosedBar>,
    ) -> Result<Vec<Option<Vec<factor_abi::exports::adaq::factor::api::NamedScalar>>>, String> {
        let mut factor = self.factor.lock().map_err(string)?;
        let LoadedFactor {
            store,
            bindings,
            instance,
        } = factor
            .as_mut()
            .ok_or_else(|| "Factor component is not loaded".to_owned())?;
        reset_component_fuel(store, self.limits)?;
        bindings
            .adaq_factor_api()
            .instance()
            .call_process(store, *instance, &bars)
            .map_err(string)?
            .map_err(|error| format!("Factor process failed: {error}"))
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
        let bindings = model_abi::Model::instantiate(&mut store, &component, &linker).map_err(string)?;
        reset_component_fuel(&mut store, self.limits)?;
        let instance = bindings.adaq_model_api().call_create(
            &mut store, &feature_slots, &parameters.iter().map(model_parameter).collect::<Vec<_>>(), seed,
        ).map_err(string)?.map_err(|error| format!("Model create failed: {error}"))?;
        let mut model = self.model.lock().map_err(string)?;
        if let Some(mut previous) = model.replace(LoadedModel { store, bindings, instance }) {
            reset_component_fuel(&mut previous.store, self.limits)?;
            previous.instance.resource_drop(&mut previous.store).map_err(string)?;
        }
        Ok(())
    }

    pub fn process_model(
        &self,
        rows: Vec<model_abi::exports::adaq::model::api::PredictionRow>,
    ) -> Result<Vec<Option<model_abi::exports::adaq::model::api::ForecastRow>>, String> {
        let mut model = self.model.lock().map_err(string)?;
        let LoadedModel { store, bindings, instance } = model.as_mut().ok_or_else(|| "Model component is not loaded".to_owned())?;
        reset_component_fuel(store, self.limits)?;
        bindings.adaq_model_api().instance().call_process(store, *instance, &rows)
            .map_err(string)?.map_err(|error| format!("Model process failed: {error}"))
    }
}

struct ComponentStore {
    limits: StoreLimits,
}

fn factor_parameter(
    value: &ComponentParameterValue,
) -> factor_abi::exports::adaq::factor::api::ParameterValue {
    use factor_abi::exports::adaq::factor::api::ParameterValue;
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

fn string(error: impl std::fmt::Display) -> String {
    error.to_string()
}
