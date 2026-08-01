use adaq_component_sdk::strategy::{
    FeatureFrame, FeatureSlot, Guest, GuestInstance, Instance as StrategyInstance, ParameterValue,
    SlotIndexes,
};

struct Component;

struct Instance {
    quote_volume: usize,
    close: usize,
}

impl Guest for Component {
    type Instance = Instance;

    fn create(
        feature_slots: Vec<FeatureSlot>,
        _parameters: Vec<ParameterValue>,
    ) -> Result<StrategyInstance, String> {
        let slots = SlotIndexes::bind(&feature_slots)?;
        Ok(StrategyInstance::new(Instance {
            quote_volume: slots.index("quote-volume")?,
            close: slots.index("close")?,
        }))
    }
}

impl GuestInstance for Instance {
    fn process(&self, frames: Vec<FeatureFrame>) -> Result<Vec<String>, String> {
        frames
            .into_iter()
            .map(|frame| {
                if frame.values.len() <= self.close.max(self.quote_volume) {
                    return Err("feature slot count mismatch".to_owned());
                }
                Ok(
                    if frame.values[self.close] > frame.values[self.quote_volume] {
                        "1"
                    } else {
                        "0"
                    }
                    .to_owned(),
                )
            })
            .collect()
    }
}

adaq_component_sdk::strategy::bindings::export_strategy!(
    Component with_types_in adaq_component_sdk::strategy::bindings
);
