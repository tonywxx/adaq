use adaq_component_sdk::factor::time_series::{
    FactorResult, FactorSchema, FactorScope, FeatureSlot, Guest, GuestInstance,
    Instance as FactorInstance, NamedScalar, ParameterValue, TimeSeriesRow,
};
use core::cell::Cell;

struct Component;

struct Instance {
    previous_fast_above_slow: Cell<Option<bool>>,
    recorded_crossover_value: Cell<Option<f64>>,
}

impl Guest for Component {
    type Instance = Instance;

    fn describe() -> Result<FactorSchema, String> {
        Ok(FactorSchema {
            scope: FactorScope::TimeSeries,
            schema_version: adaq_component_sdk::FACTOR_SCHEMA_VERSION.into(),
            feature_slots: vec![
                FeatureSlot { name: "ema-5".into() },
                FeatureSlot { name: "ema-10".into() },
            ],
            parameters: Vec::new(),
            output_names: vec!["buy-signal".into()],
            warmup_bars: 1,
        })
    }

    fn create(
        _feature_slots: Vec<FeatureSlot>,
        _parameters: Vec<ParameterValue>,
    ) -> Result<FactorInstance, String> {
        Ok(FactorInstance::new(Instance {
            previous_fast_above_slow: Cell::new(None),
            recorded_crossover_value: Cell::new(None),
        }))
    }
}

impl GuestInstance for Instance {
    fn process(&self, rows: Vec<TimeSeriesRow>) -> Result<Vec<FactorResult>, String> {
        rows.into_iter()
            .map(|row| {
                let fast = row.slots[0].value;
                let slow = row.slots[1].value;
                let was_above = self.previous_fast_above_slow.replace(Some(fast > slow));
                let bullish_crossover = was_above == Some(false) && fast > slow;
                let signal = if bullish_crossover {
                    let previous = self.recorded_crossover_value.replace(Some(fast));
                    previous.is_some_and(|value| fast > value)
                } else {
                    false
                };
                let values = if was_above.is_some() {
                    Some(vec![NamedScalar {
                        name: "buy-signal".into(),
                        value: if signal { 1.0 } else { 0.0 },
                    }])
                } else {
                    None
                };
                Ok(FactorResult {
                    instrument_id: row.instrument_id,
                    observation_time_ms: row.observation_time_ms,
                    values,
                })
            })
            .collect()
    }
}

adaq_component_sdk::factor::time_series::bindings::export_factor!(
    Component with_types_in adaq_component_sdk::factor::time_series::bindings
);
