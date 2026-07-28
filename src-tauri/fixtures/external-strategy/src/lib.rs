use adaq_component_sdk::strategy::{FeatureFrame, FeatureSlot, Guest, GuestInstance, Instance, ParameterValue, SlotIndexes};

struct Component;
struct ExternalStrategy { close_change: usize }

impl Guest for Component {
    type Instance = ExternalStrategy;
    fn create(slots: Vec<FeatureSlot>, _parameters: Vec<ParameterValue>) -> Result<Instance, String> {
        Ok(Instance::new(ExternalStrategy { close_change: SlotIndexes::bind(&slots)?.index("close-change")? }))
    }
}

impl GuestInstance for ExternalStrategy {
    fn process(&self, frames: Vec<FeatureFrame>) -> Result<Vec<String>, String> {
        frames.into_iter().map(|frame| Ok(if frame.values.get(self.close_change).ok_or("feature slot count mismatch")? > &0.0 { "1" } else { "0" }.to_owned())).collect()
    }
}

adaq_component_sdk::strategy::bindings::export_strategy!(Component with_types_in adaq_component_sdk::strategy::bindings);
