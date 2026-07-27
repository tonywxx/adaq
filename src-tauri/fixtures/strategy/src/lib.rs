#[allow(warnings)]
mod bindings;

use bindings::exports::adaq::strategy::api::{
    FeatureFrame, FeatureSlot, Guest, GuestInstance, Instance as StrategyInstance,
};

struct Component;

struct Instance {
    slots: usize,
}

impl Guest for Component {
    type Instance = Instance;

    fn create(
        feature_slots: Vec<FeatureSlot>,
        _parameters: Vec<bindings::exports::adaq::strategy::api::ParameterValue>,
    ) -> Result<StrategyInstance, String> {
        Ok(StrategyInstance::new(Instance {
            slots: feature_slots.len(),
        }))
    }
}

impl GuestInstance for Instance {
    fn process(&self, frames: Vec<FeatureFrame>) -> Result<Vec<String>, String> {
        frames
            .into_iter()
            .map(|frame| {
                if frame.values.len() != self.slots {
                    return Err("feature slot count mismatch".to_owned());
                }
                Ok(if frame.values[0] >= 0.0 { "1" } else { "0" }.to_owned())
            })
            .collect()
    }
}

bindings::export!(Component with_types_in bindings);
