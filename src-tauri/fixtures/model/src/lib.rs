use adaq_component_sdk::model::{
    FeatureSlot, ForecastRow, Guest, GuestInstance, Instance as ModelInstance, ParameterValue,
    PredictionRow,
};
struct Model;

struct Instance {
    mode: String,
}

impl Guest for Model {
    type Instance = Instance;

    fn create(
        _feature_slots: Vec<FeatureSlot>,
        parameters: Vec<ParameterValue>,
        _seed: u64,
    ) -> Result<ModelInstance, String> {
        let mode = match parameters.first() {
            Some(ParameterValue::Text(value)) => value.clone(),
            _ => "valid".into(),
        };
        Ok(ModelInstance::new(Instance { mode }))
    }
}

impl GuestInstance for Instance {
    fn process(&self, rows: Vec<PredictionRow>) -> Result<Vec<Option<ForecastRow>>, String> {
        rows.into_iter()
            .map(|row| {
                Ok(Some(ForecastRow {
                    instrument_id: row.instrument_id,
                    prediction_time_ms: if self.mode == "wrong-time" {
                        row.prediction_time_ms + 1
                    } else {
                        row.prediction_time_ms
                    },
                    values: vec![if self.mode == "non-finite" {
                        f64::NAN
                    } else {
                        row.values.first().copied().unwrap_or_default() + 2.0
                    }],
                }))
            })
            .collect()
    }
}

adaq_component_sdk::model::bindings::export_model!(
    Model with_types_in adaq_component_sdk::model::bindings
);
