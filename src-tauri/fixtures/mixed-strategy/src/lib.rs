use core::cell::Cell;

use adaq_component_sdk::strategy::{
    FeatureFrame, FeatureSlot, Guest, GuestInstance, Instance as StrategyInstance, ParameterValue,
    SlotIndexes,
};

struct Component;

struct Instance {
    close_change: usize,
    ema: usize,
    quote_volume: usize,
    calls: Cell<u32>,
}

impl Guest for Component {
    type Instance = Instance;

    fn create(
        feature_slots: Vec<FeatureSlot>,
        _parameters: Vec<ParameterValue>,
    ) -> Result<StrategyInstance, String> {
        let slots = SlotIndexes::bind(&feature_slots)?;
        Ok(StrategyInstance::new(Instance {
            close_change: slots.index("close-change")?,
            ema: slots.index("ema")?,
            quote_volume: slots.index("quote-volume")?,
            calls: Cell::new(0),
        }))
    }
}

impl GuestInstance for Instance {
    fn process(&self, frames: Vec<FeatureFrame>) -> Result<Vec<String>, String> {
        frames
            .into_iter()
            .map(|frame| {
                if frame.values.len() <= self.close_change.max(self.ema).max(self.quote_volume) {
                    return Err("feature slot count mismatch".to_owned());
                }
                if frame.values[self.quote_volume] == 0.0 {
                    return Err("requested strategy failure".to_owned());
                }
                let first = self.calls.get() == 0;
                self.calls.set(self.calls.get() + 1);
                Ok((first
                    && frame.values[self.close_change] > 0.0
                    && frame.values[self.ema] > frame.values[self.quote_volume])
                    .then_some("1")
                    .unwrap_or("0")
                    .to_owned())
            })
            .collect()
    }
}

adaq_component_sdk::strategy::bindings::export_strategy!(
    Component with_types_in adaq_component_sdk::strategy::bindings
);
