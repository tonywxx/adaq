use adaq_component_sdk::factor::{ClosedBar, FactorSchema, Guest, GuestInstance, Instance, NamedScalar, ParameterValue};
use adaq_component_sdk::decimal_to_f64;

struct Component;
struct Values;

impl Guest for Component {
    type Instance = Values;
    fn describe() -> Result<FactorSchema, String> { Ok(FactorSchema { output_names: vec!["close".into(), "quote-volume".into()], warmup_bars: 0 }) }
    fn create(_: Vec<ParameterValue>) -> Result<Instance, String> { Ok(Instance::new(Values)) }
}
impl GuestInstance for Values {
    fn process(&self, bars: Vec<ClosedBar>) -> Result<Vec<Option<Vec<NamedScalar>>>, String> {
        bars.into_iter().map(|bar| Ok(Some(vec![NamedScalar { name: "close".into(), value: decimal_to_f64(adaq_component_sdk::parse_decimal(&bar.close)?)? }, NamedScalar { name: "quote-volume".into(), value: decimal_to_f64(adaq_component_sdk::parse_decimal(&bar.quote_volume)?)? }]))).collect()
    }
}
adaq_component_sdk::factor::bindings::export_factor!(Component with_types_in adaq_component_sdk::factor::bindings);
