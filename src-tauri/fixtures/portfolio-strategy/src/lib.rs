use adaq_component_sdk::portfolio_strategy::{
    FeatureRow, FeatureSlot, Guest, GuestInstance, Instance as StrategyInstance, ParameterValue,
    PortfolioFrame, PortfolioTarget, SlotIndexes, TargetWeight,
};

struct Component;
struct Instance;

impl Guest for Component {
    type Instance = Instance;

    fn create(
        feature_slots: Vec<FeatureSlot>,
        _parameters: Vec<ParameterValue>,
    ) -> Result<StrategyInstance, String> {
        let _ = SlotIndexes::bind(&feature_slots)?;
        Ok(StrategyInstance::new(Instance))
    }
}

impl GuestInstance for Instance {
    fn process(&self, frames: Vec<PortfolioFrame>) -> Result<Vec<PortfolioTarget>, String> {
        frames
            .into_iter()
            .map(|frame| {
                if frame.universe_id.is_empty() || frame.rows.is_empty() {
                    return Err("portfolio frame is incomplete".into());
                }
                let weight = format!("{}", 1.0 / frame.rows.len() as f64);
                Ok(PortfolioTarget {
                    decision_time_ms: frame.decision_time_ms,
                    universe_id: frame.universe_id,
                    weights: frame
                        .rows
                        .into_iter()
                        .map(|row: FeatureRow| TargetWeight {
                            instrument_id: row.instrument_id,
                            weight: weight.clone(),
                        })
                        .collect(),
                    cash_reserve: "0".into(),
                })
            })
            .collect()
    }
}

adaq_component_sdk::portfolio_strategy::bindings::export_portfolio_strategy!(
    Component with_types_in adaq_component_sdk::portfolio_strategy::bindings
);
