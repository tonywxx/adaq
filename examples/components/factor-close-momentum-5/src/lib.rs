use adaq_component_sdk::factor::{
    ClosedBar, FactorSchema, Guest, GuestInstance, Instance as FactorInstance, NamedScalar,
    ParameterValue,
};
use adaq_component_sdk::{decimal_to_f64, parse_decimal};
use core::cell::RefCell;
use std::collections::VecDeque;

const LOOKBACK: usize = 5;
const OUTPUT_NAME: &str = "close-momentum-5";

struct Component;

struct Instance {
    closes: RefCell<VecDeque<adaq_component_sdk::Decimal>>,
}

impl Guest for Component {
    type Instance = Instance;

    fn describe() -> Result<FactorSchema, String> {
        Ok(FactorSchema {
            output_names: vec![OUTPUT_NAME.to_owned()],
            warmup_bars: LOOKBACK as u32,
        })
    }

    fn create(parameters: Vec<ParameterValue>) -> Result<FactorInstance, String> {
        if !parameters.is_empty() {
            return Err("factor-close-momentum-5 does not accept parameters".to_owned());
        }
        Ok(FactorInstance::new(Instance {
            closes: RefCell::new(VecDeque::with_capacity(LOOKBACK)),
        }))
    }
}

impl GuestInstance for Instance {
    fn process(&self, bars: Vec<ClosedBar>) -> Result<Vec<Option<Vec<NamedScalar>>>, String> {
        bars.into_iter()
            .map(|bar| {
                let close = parse_decimal(&bar.close)?;
                let mut closes = self.closes.borrow_mut();
                let output = if closes.len() == LOOKBACK {
                    let previous = closes.front().copied().expect("length checked");
                    if previous.is_zero() {
                        return Err("close five Bars ago must be non-zero".to_owned());
                    }
                    Some(vec![NamedScalar {
                        name: OUTPUT_NAME.to_owned(),
                        value: decimal_to_f64((close - previous) / previous)?,
                    }])
                } else {
                    None
                };
                if closes.len() == LOOKBACK {
                    closes.pop_front();
                }
                closes.push_back(close);
                Ok(output)
            })
            .collect()
    }
}

adaq_component_sdk::factor::bindings::export_factor!(
    Component with_types_in adaq_component_sdk::factor::bindings
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_after_exact_five_bar_warmup() {
        let instance = Instance {
            closes: RefCell::new(VecDeque::with_capacity(LOOKBACK)),
        };
        let rows = instance
            .process(
                ["100", "101", "102", "103", "104", "105"]
                    .into_iter()
                    .enumerate()
                    .map(|(index, close)| ClosedBar {
                        open_time_ms: index as i64,
                        open: close.to_owned(),
                        high: close.to_owned(),
                        low: close.to_owned(),
                        close: close.to_owned(),
                        base_volume: "1".to_owned(),
                        quote_volume: "1".to_owned(),
                    })
                    .collect(),
            )
            .unwrap();

        assert!(rows[..LOOKBACK].iter().all(Option::is_none));
        assert_eq!(rows[LOOKBACK].as_ref().unwrap()[0].value, 0.05);
    }
}
