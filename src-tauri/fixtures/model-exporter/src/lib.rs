use adaq_component_sdk::model::{
    FeatureSlot, ForecastRow, Guest, GuestInstance, Instance as ModelInstance, ParameterValue,
    PredictionRow,
};

const BINDING_SCHEMA: &str = "adaq:linear-model-binding@1";

struct Model;

struct Instance {
    means: Vec<f64>,
    scales: Vec<f64>,
    coefficients: Vec<f64>,
    intercept: f64,
}

impl Guest for Model {
    type Instance = Instance;

    fn create(
        feature_slots: Vec<FeatureSlot>,
        parameters: Vec<ParameterValue>,
        _seed: u64,
    ) -> Result<ModelInstance, String> {
        let binding = match parameters.as_slice() {
            [ParameterValue::Text(value)] => value,
            _ => return Err("model binding parameter is required".into()),
        };
        let mut fields = binding.split('|');
        if fields.next() != Some(BINDING_SCHEMA) {
            return Err("model binding schema is invalid".into());
        }
        if !valid_hash(fields.next()) || !valid_hash(fields.next()) {
            return Err("model binding identity is invalid".into());
        }
        let count = fields
            .next()
            .and_then(|value| value.parse::<usize>().ok())
            .ok_or_else(|| "model binding feature count is invalid".to_owned())?;
        if count == 0 || count != feature_slots.len() {
            return Err("model binding feature count does not match the manifest".into());
        }
        let means = parse_values(fields.next(), count, "means")?;
        let scales = parse_values(fields.next(), count, "scales")?;
        if scales.iter().any(|value| *value <= 0.0) {
            return Err("model binding scale is invalid".into());
        }
        let coefficients = parse_values(fields.next(), count, "coefficients")?;
        let intercept = fields
            .next()
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| value.is_finite())
            .ok_or_else(|| "model binding intercept is invalid".to_owned())?;
        if fields.next().is_some() {
            return Err("model binding has unexpected fields".into());
        }
        Ok(ModelInstance::new(Instance {
            means,
            scales,
            coefficients,
            intercept,
        }))
    }
}

impl GuestInstance for Instance {
    fn process(&self, rows: Vec<PredictionRow>) -> Result<Vec<Option<ForecastRow>>, String> {
        rows.into_iter()
            .map(|row| {
                if row.values.len() != self.coefficients.len()
                    || row.values.iter().any(|value| !value.is_finite())
                {
                    return Err("model prediction batch schema is invalid".into());
                }
                let value = self.intercept
                    + self
                        .coefficients
                        .iter()
                        .zip(row.values.iter().zip(&self.means).zip(&self.scales))
                        .map(|(coefficient, ((feature, mean), scale))| {
                            coefficient * ((feature - mean) / scale)
                        })
                        .sum::<f64>();
                if !value.is_finite() {
                    return Err("model prediction is non-finite".into());
                }
                Ok(Some(ForecastRow {
                    instrument_id: row.instrument_id,
                    prediction_time_ms: row.prediction_time_ms,
                    values: vec![value],
                }))
            })
            .collect()
    }
}

fn parse_values(value: Option<&str>, count: usize, label: &str) -> Result<Vec<f64>, String> {
    let values = value
        .ok_or_else(|| format!("model binding {label} are missing"))?
        .split(',')
        .map(|value| {
            value
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite())
                .ok_or_else(|| format!("model binding {label} are invalid"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if values.len() != count {
        return Err(format!("model binding {label} count is invalid"));
    }
    Ok(values)
}

fn valid_hash(value: Option<&str>) -> bool {
    value.is_some_and(|value| {
        value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

adaq_component_sdk::model::bindings::export_model!(
    Model with_types_in adaq_component_sdk::model::bindings
);
