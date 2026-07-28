use serde::Deserialize;

pub const VERSION: &str = "adaq-indicator-catalog@1.0.0";
pub const ARCHIVE_SHA256: &str = "40e7a6978052fe5245771e430e6a4c4553b40038f8ac5a985a1540c4c1fa6ace";
pub const XML_SHA256: &str = "70ed7629a577cb3803ed2882607070beb15592724ea4366735a9e0fc8413dec1";

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Catalog {
    pub version: String,
    pub indicators: Vec<Definition>,
}
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Definition {
    #[serde(rename = "id")]
    pub id: String,
    #[serde(rename = "rawName")]
    pub raw_name: String,
    #[serde(rename = "unstablePeriod")]
    pub unstable_period: bool,
    pub inputs: Vec<Input>,
    pub parameters: Vec<Parameter>,
    pub outputs: Vec<Output>,
}
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Input {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub role: String,
    #[serde(rename = "allowedFields", default)]
    pub allowed_fields: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Parameter {
    pub id: String,
    #[serde(rename = "rawName")]
    pub raw_name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub default: String,
    pub minimum: String,
    pub maximum: String,
    #[serde(rename = "enumValues", default)]
    pub enum_values: Vec<EnumValue>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct EnumValue {
    pub id: String,
    pub value: i32,
}
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Output {
    pub id: String,
    #[serde(rename = "rawName")]
    pub raw_name: String,
    #[serde(rename = "type")]
    pub kind: String,
}

pub fn catalog() -> &'static Catalog {
    static CATALOG: std::sync::OnceLock<Catalog> = std::sync::OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!("../catalog.json")).expect("committed catalog is valid")
    })
}
