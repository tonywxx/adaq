use adaq_component_sdk::strategy::{
    FeatureFrame, FeatureSlot, Guest, GuestInstance, Instance as StrategyInstance, ParameterValue,
    SlotIndexes,
};
use adaq_component_sdk::{decimal_to_f64, parse_decimal};

struct Component;

struct Instance {
    close: usize,
    ema: usize,
    momentum: usize,
    minimum_momentum: f64,
}

impl Guest for Component {
    type Instance = Instance;

    fn create(
        feature_slots: Vec<FeatureSlot>,
        parameters: Vec<ParameterValue>,
    ) -> Result<StrategyInstance, String> {
        let [
            ParameterValue::Integer(_ema_period),
            ParameterValue::Decimal(minimum_momentum),
        ] = parameters.as_slice()
        else {
            return Err("expected integer ema-period and decimal minimum-momentum".to_owned());
        };
        let slots = SlotIndexes::bind(&feature_slots)?;
        Ok(StrategyInstance::new(Instance {
            close: slots.index("close")?,
            ema: slots.index("ema")?,
            momentum: slots.index("momentum")?,
            minimum_momentum: decimal_to_f64(parse_decimal(minimum_momentum)?)?,
        }))
    }
}

impl GuestInstance for Instance {
    fn process(&self, frames: Vec<FeatureFrame>) -> Result<Vec<String>, String> {
        frames
            .into_iter()
            .map(|frame| {
                if frame.values.len() != 3 {
                    return Err("feature slot count mismatch".to_owned());
                }
                Ok(if frame.values[self.close] > frame.values[self.ema]
                    && frame.values[self.momentum] > self.minimum_momentum
                {
                    "1"
                } else {
                    "0"
                }
                .to_owned())
            })
            .collect()
    }
}

adaq_component_sdk::strategy::bindings::export_strategy!(
    Component with_types_in adaq_component_sdk::strategy::bindings
);

#[cfg(test)]
mod tests {
    use super::*;

    fn instance() -> Instance {
        Instance {
            close: 0,
            ema: 1,
            momentum: 2,
            minimum_momentum: 0.01,
        }
    }

    #[test]
    fn requires_trend_and_momentum_confirmation() {
        let targets = instance()
            .process(vec![
                FeatureFrame {
                    open_time_ms: 1,
                    values: vec![101.0, 100.0, 0.02],
                },
                FeatureFrame {
                    open_time_ms: 2,
                    values: vec![99.0, 100.0, 0.02],
                },
                FeatureFrame {
                    open_time_ms: 3,
                    values: vec![101.0, 100.0, 0.005],
                },
            ])
            .unwrap();

        assert_eq!(targets, ["1", "0", "0"]);
    }
}
