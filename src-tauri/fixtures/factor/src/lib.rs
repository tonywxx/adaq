#[allow(warnings)]
mod bindings;

use bindings::exports::adaq::factor::api::{
    ClosedBar, FactorSchema, Guest, GuestInstance, Instance as FactorInstance, NamedScalar,
};
use core::cell::Cell;

struct Component;

struct Instance {
    previous_close: Cell<Option<f64>>,
}

impl Guest for Component {
    type Instance = Instance;

    fn describe() -> Result<FactorSchema, String> {
        Ok(FactorSchema {
            output_names: vec!["close-change".to_owned()],
            warmup_bars: 1,
        })
    }

    fn create() -> Result<FactorInstance, String> {
        Ok(FactorInstance::new(Instance {
            previous_close: Cell::new(None),
        }))
    }
}

impl GuestInstance for Instance {
    fn process(
        &self,
        bars: Vec<ClosedBar>,
    ) -> Result<Vec<Option<Vec<NamedScalar>>>, String> {
        Ok(bars
            .into_iter()
            .map(|bar| {
                let close = bar.close.parse::<f64>().map_err(|_| "invalid close")?;
                let output = self.previous_close.get().map(|previous_close| {
                    vec![NamedScalar {
                        name: "close-change".to_owned(),
                        value: close - previous_close,
                    }]
                });
                self.previous_close.set(Some(close));
                Ok::<_, String>(output)
            })
            .collect::<Result<Vec<_>, _>>()?)
    }
}

bindings::export!(Component with_types_in bindings);
