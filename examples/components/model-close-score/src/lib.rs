use adaq_component_sdk::model::{
    FeatureSlot, ForecastRow, Guest, GuestInstance, Instance as ModelInstance, ParameterValue,
    PredictionRow,
};

struct Component;
struct Instance;

impl Guest for Component {
    type Instance = Instance;

    fn create(
        _feature_slots: Vec<FeatureSlot>,
        _parameters: Vec<ParameterValue>,
        _seed: u64,
    ) -> Result<ModelInstance, String> {
        Ok(ModelInstance::new(Instance))
    }
}

impl GuestInstance for Instance {
    fn process(&self, rows: Vec<PredictionRow>) -> Result<Vec<Option<ForecastRow>>, String> {
        Ok(rows.into_iter().map(|row| Some(ForecastRow {
            instrument_id: row.instrument_id,
            prediction_time_ms: row.prediction_time_ms,
            values: vec![row.values.first().copied().unwrap_or_default()],
        })).collect())
    }
}

adaq_component_sdk::model::bindings::export_model!(
    Component with_types_in adaq_component_sdk::model::bindings
);
