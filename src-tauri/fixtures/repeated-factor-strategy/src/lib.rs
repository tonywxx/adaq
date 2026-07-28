use adaq_component_sdk::strategy::{FeatureFrame, FeatureSlot, Guest, GuestInstance, Instance, ParameterValue, SlotIndexes};
struct Component;
struct Values { fast: usize, slow: usize }
impl Guest for Component { type Instance = Values; fn create(slots: Vec<FeatureSlot>, _: Vec<ParameterValue>) -> Result<Instance, String> { let slots = SlotIndexes::bind(&slots)?; Ok(Instance::new(Values { fast: slots.index("fast-close")?, slow: slots.index("slow-close")? })) } }
impl GuestInstance for Values { fn process(&self, frames: Vec<FeatureFrame>) -> Result<Vec<String>, String> { frames.into_iter().map(|frame| Ok((frame.values.get(self.fast).ok_or("feature slot count mismatch")? > frame.values.get(self.slow).ok_or("feature slot count mismatch")?).then_some("1").unwrap_or("0").into())).collect() } }
adaq_component_sdk::strategy::bindings::export_strategy!(Component with_types_in adaq_component_sdk::strategy::bindings);
