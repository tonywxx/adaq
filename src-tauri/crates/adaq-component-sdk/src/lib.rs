pub use rust_decimal::Decimal;

pub const SDK_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const ABI_VERSION: &str = "1.0.0";

#[cfg(feature = "factor")]
pub mod factor {
    pub mod bindings {
        wit_bindgen::generate!({
            path: "wit/factor",
            world: "factor",
            pub_export_macro: true,
            export_macro_name: "export_factor",
        });
    }

    pub use bindings::exports::adaq::factor::api::{
        ClosedBar, FactorSchema, Guest, GuestInstance, Instance, NamedScalar, ParameterValue,
    };
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
}

#[cfg(feature = "host")]
pub mod host {
    pub mod factor_abi {
        wasmtime::component::bindgen!({
            path: "wit/factor",
            world: "factor",
        });
    }

    pub mod strategy_abi {
        wasmtime::component::bindgen!({
            path: "wit/strategy",
            world: "strategy",
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
