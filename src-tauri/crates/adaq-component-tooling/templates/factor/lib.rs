use adaq_component_sdk::factor::time_series::{
    FactorResult, FactorSchema, FactorScope, FeatureSlot, Guest, GuestInstance,
    Instance as FactorInstance, NamedScalar, ParameterValue, TimeSeriesRow,
};
use adaq_component_sdk::{decimal_to_f64, parse_decimal};
use core::cell::Cell;

struct Component;

struct Instance {
    previous_close: Cell<Option<adaq_component_sdk::Decimal>>,
}

impl Guest for Component {
    type Instance = Instance;

    fn describe() -> Result<FactorSchema, String> {
        Ok(FactorSchema {
            scope: FactorScope::TimeSeries,
            schema_version: adaq_component_sdk::FACTOR_SCHEMA_VERSION.into(),
            feature_slots: vec![FeatureSlot { name: "close".into() }],
            parameters: Vec::new(),
            output_names: vec!["close-change".to_owned()],
            warmup_bars: 1,
        })
    }

    fn create(
        _feature_slots: Vec<FeatureSlot>,
        _parameters: Vec<ParameterValue>,
    ) -> Result<FactorInstance, String> {
        Ok(FactorInstance::new(Instance {
            previous_close: Cell::new(None),
        }))
    }
}

impl GuestInstance for Instance {
    fn process(&self, rows: Vec<TimeSeriesRow>) -> Result<Vec<FactorResult>, String> {
        rows.into_iter()
            .map(|row| {
                let close = parse_decimal(&row.slots[0].value.to_string())?;
                let values = self.previous_close.get().map(|previous| {
                    vec![NamedScalar {
                        name: "close-change".to_owned(),
                        value: decimal_to_f64(close - previous).unwrap_or(f64::NAN),
                    }]
                });
                self.previous_close.set(Some(close));
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
