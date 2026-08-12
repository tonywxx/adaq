pub use rust_decimal::Decimal;

pub const SDK_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const ABI_VERSION: &str = "1.0.0";
pub const FACTOR_ABI_VERSION: &str = "2.0.0";
pub const FACTOR_SCHEMA_VERSION: &str = "adaq-factor-schema@2.0.0";

#[cfg(feature = "factor")]
pub mod factor {
    pub mod time_series {
        pub mod bindings {
            wit_bindgen::generate!({
                path: "wit/factor",
                world: "time-series",
                pub_export_macro: true,
                export_macro_name: "export_factor",
            });
        }

        pub use bindings::exports::adaq::factor::time_series_api::{
            FactorResult, FactorSchema, FactorScope, FeatureSlot, FeatureValue, Guest,
            GuestInstance, Instance, NamedScalar, ParameterDefinition, ParameterType,
            ParameterValue, TimeSeriesRow,
        };
    }

    pub mod cross_sectional {
        pub mod bindings {
            wit_bindgen::generate!({
                path: "wit/factor",
                world: "cross-sectional",
                pub_export_macro: true,
                export_macro_name: "export_factor",
            });
        }

        pub use bindings::exports::adaq::factor::cross_sectional_api::{
            CrossSectionalRow, FactorResult, FactorSchema, FactorScope, FeatureCell, FeatureSlot,
            FeatureValue, Guest, GuestInstance, Instance, NamedScalar, ParameterDefinition,
            ParameterType, ParameterValue,
        };
    }

    pub use time_series::*;
}

#[cfg(feature = "strategy")]
pub mod strategy {
    pub mod bindings {
        wit_bindgen::generate!({
            path: "wit/strategy",
            world: "strategy",
            pub_export_macro: true,
            export_macro_name: "export_strategy",
        });
    }

    pub use bindings::exports::adaq::strategy::api::{
        FeatureFrame, FeatureSlot, Guest, GuestInstance, Instance, ParameterValue,
    };

    pub struct SlotIndexes(std::collections::HashMap<String, usize>);

    impl SlotIndexes {
        pub fn bind(feature_slots: &[FeatureSlot]) -> Result<Self, String> {
            let mut indexes = std::collections::HashMap::with_capacity(feature_slots.len());
            for (index, slot) in feature_slots.iter().enumerate() {
                if indexes.insert(slot.name.clone(), index).is_some() {
                    return Err(format!("duplicate Feature Slot: {}", slot.name));
                }
            }
            Ok(Self(indexes))
        }

        pub fn index(&self, name: &str) -> Result<usize, String> {
            self.0
                .get(name)
                .copied()
                .ok_or_else(|| format!("missing Feature Slot: {name}"))
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn binds_exact_slot_names_to_dense_indexes_once() {
            let slots = vec![
                FeatureSlot {
                    name: "quote-volume".into(),
                },
                FeatureSlot {
                    name: "close".into(),
                },
            ];
            let indexes = SlotIndexes::bind(&slots).unwrap();
            assert_eq!(indexes.index("quote-volume"), Ok(0));
            assert_eq!(indexes.index("close"), Ok(1));
            assert!(indexes.index("missing").is_err());
        }
    }
}

#[cfg(feature = "model")]
pub mod model {
    pub mod bindings {
        wit_bindgen::generate!({
            path: "wit/model",
            world: "model",
            pub_export_macro: true,
            export_macro_name: "export_model",
        });
    }

    pub use bindings::exports::adaq::model::api::{
        FeatureSlot, ForecastRow, Guest, GuestInstance, Instance, ParameterValue, PredictionRow,
    };
}

#[cfg(feature = "host")]
pub mod host {
    pub mod factor_time_series_abi {
        wasmtime::component::bindgen!({
            path: "wit/factor",
            world: "time-series",
        });
    }

    pub mod factor_cross_sectional_abi {
        wasmtime::component::bindgen!({
            path: "wit/factor",
            world: "cross-sectional",
        });
    }

    pub use factor_time_series_abi as factor_abi;

    pub mod strategy_abi {
        wasmtime::component::bindgen!({
            path: "wit/strategy",
            world: "strategy",
        });
    }

    pub mod model_abi {
        wasmtime::component::bindgen!({
            path: "wit/model",
            world: "model",
        });
    }
}

pub fn parse_decimal(value: &str) -> Result<Decimal, String> {
    Decimal::from_str_exact(value).map_err(|error| error.to_string())
}

pub fn decimal_to_f64(value: Decimal) -> Result<f64, String> {
    use rust_decimal::prelude::ToPrimitive;
    value
        .to_f64()
        .filter(|value| value.is_finite())
        .ok_or_else(|| "analytical value cannot be represented as finite f64".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn financial_values_require_exact_decimal_text() {
        assert_eq!(parse_decimal("0.100").unwrap().to_string(), "0.100");
        assert!(parse_decimal("NaN").is_err());
        assert_eq!(
            decimal_to_f64(parse_decimal("1.25").unwrap()).unwrap(),
            1.25
        );
    }
}
