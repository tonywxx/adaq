use adaq_component_sdk::factor::cross_sectional::{
    CrossSectionalRow, FactorResult, FactorSchema, FactorScope, FeatureCell, FeatureSlot, Guest,
    GuestInstance, Instance as FactorInstance, NamedScalar, ParameterValue,
};

struct Component;
struct Values;

impl Guest for Component {
    type Instance = Values;

    fn describe() -> Result<FactorSchema, String> {
        Ok(FactorSchema {
            scope: FactorScope::CrossSectional,
            schema_version: adaq_component_sdk::FACTOR_SCHEMA_VERSION.into(),
            feature_slots: vec![FeatureSlot { name: "close".into() }],
            parameters: vec![],
            output_names: vec!["cross-sectional-score".into()],
            warmup_bars: 0,
        })
    }

    fn create(
        _feature_slots: Vec<FeatureSlot>,
        _parameters: Vec<ParameterValue>,
    ) -> Result<FactorInstance, String> {
        Ok(FactorInstance::new(Values))
    }
}

impl GuestInstance for Values {
    fn process(&self, rows: Vec<CrossSectionalRow>) -> Result<Vec<FactorResult>, String> {
        rows.into_iter()
            .map(|row| {
                let value = match row.slots.first() {
                    Some(FeatureCell::Available(value)) => value.value,
                    Some(FeatureCell::Unavailable(_)) | None => {
                        return Ok(FactorResult {
                            instrument_id: row.instrument_id,
                            observation_time_ms: row.observation_time_ms,
                            values: None,
                        });
                    }
                };
                Ok(FactorResult {
                    instrument_id: row.instrument_id,
                    observation_time_ms: row.observation_time_ms,
                    values: Some(vec![NamedScalar {
                        name: "cross-sectional-score".into(),
                        value,
                    }]),
                })
            })
            .collect()
    }
}

adaq_component_sdk::factor::cross_sectional::bindings::export_factor!(
    Component with_types_in adaq_component_sdk::factor::cross_sectional::bindings
);
