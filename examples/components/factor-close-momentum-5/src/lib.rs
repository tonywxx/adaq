use adaq_component_sdk::factor::time_series::{
    FactorResult, FactorSchema, FactorScope, FeatureSlot, Guest, GuestInstance,
    Instance as FactorInstance, NamedScalar, ParameterValue, TimeSeriesRow,
};
use adaq_component_sdk::{decimal_to_f64, parse_decimal};
use core::cell::RefCell;

struct Component;

struct Instance {
    closes: RefCell<Vec<adaq_component_sdk::Decimal>>,
}

impl Guest for Component {
    type Instance = Instance;

    fn describe() -> Result<FactorSchema, String> {
        Ok(FactorSchema {
            scope: FactorScope::TimeSeries,
            schema_version: adaq_component_sdk::FACTOR_SCHEMA_VERSION.into(),
            feature_slots: vec![FeatureSlot { name: "close".into() }],
            parameters: Vec::new(),
            output_names: vec!["close-momentum-5".to_owned()],
            warmup_bars: 5,
        })
    }

    fn create(
        _feature_slots: Vec<FeatureSlot>,
        _parameters: Vec<ParameterValue>,
    ) -> Result<FactorInstance, String> {
        Ok(FactorInstance::new(Instance {
            closes: RefCell::new(Vec::new()),
        }))
    }
}

impl GuestInstance for Instance {
    fn process(&self, rows: Vec<TimeSeriesRow>) -> Result<Vec<FactorResult>, String> {
        rows.into_iter()
            .map(|row| {
                let close = parse_decimal(&row.slots[0].value.to_string())?;
                let mut closes = self.closes.borrow_mut();
                let values = (closes.len() >= 5).then(|| {
                    let old = &closes[0];
                    vec![NamedScalar {
                        name: "close-momentum-5".to_owned(),
                        value: decimal_to_f64(close / *old - adaq_component_sdk::Decimal::ONE)
                            .unwrap_or(f64::NAN),
                    }]
                });
                closes.push(close);
                if closes.len() > 5 {
                    closes.remove(0);
                }
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
