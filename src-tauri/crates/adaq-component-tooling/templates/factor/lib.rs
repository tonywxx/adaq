use adaq_component_sdk::{decimal_to_f64, parse_decimal};
use adaq_component_sdk::factor::{
    ClosedBar, FactorSchema, Guest, GuestInstance, Instance as FactorInstance, NamedScalar,
    ParameterValue,
};
use core::cell::Cell;

struct Component;

struct Instance {
    previous_close: Cell<Option<adaq_component_sdk::Decimal>>,
}

impl Guest for Component {
    type Instance = Instance;

    fn describe() -> Result<FactorSchema, String> {
        Ok(FactorSchema {
            output_names: vec!["close-change".to_owned()],
            warmup_bars: 1,
        })
    }

    fn create(_parameters: Vec<ParameterValue>) -> Result<FactorInstance, String> {
        Ok(FactorInstance::new(Instance {
            previous_close: Cell::new(None),
        }))
    }
}

impl GuestInstance for Instance {
    fn process(&self, bars: Vec<ClosedBar>) -> Result<Vec<Option<Vec<NamedScalar>>>, String> {
        bars.into_iter()
            .map(|bar| {
                let close = parse_decimal(&bar.close)?;
                let output = match self.previous_close.get() {
                    Some(previous) => Some(vec![NamedScalar {
                        name: "close-change".to_owned(),
                        value: decimal_to_f64(close - previous)?,
                    }]),
                    None => None,
                };
                self.previous_close.set(Some(close));
                Ok(output)
            })
            .collect()
    }
}

adaq_component_sdk::factor::bindings::export_factor!(
    Component with_types_in adaq_component_sdk::factor::bindings
);
