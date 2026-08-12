//! Tauri-independent Factor research contracts for M11.

mod abi;
mod candidate;
mod catalog;
mod contracts;
mod evaluation;
mod materialization;

use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

pub use abi::*;
pub use candidate::*;
pub use catalog::*;
pub use contracts::*;
pub use evaluation::*;
pub use materialization::*;

pub const FACTOR_RESEARCH_SCHEMA_VERSION: &str = "1.0.0";
pub const FACTOR_ABI_VERSION: &str = "2.0.0";
pub const MAX_CANONICAL_JSON_BYTES: usize = 1024 * 1024;
pub const MAX_FACTOR_OUTPUTS: usize = 64;
pub const MAX_FACTOR_SLOTS: usize = 64;
pub const MAX_GRID_SEARCH_TRIALS: u64 = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractError {
    Invalid(String),
    LimitExceeded {
        name: &'static str,
        limit: usize,
    },
    HashMismatch,
    NonCanonical,
    ResetRequired {
        stored_schema_version: Option<String>,
        guidance: String,
    },
}

impl std::fmt::Display for ContractError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(message) => formatter.write_str(message),
            Self::LimitExceeded { name, limit } => {
                write!(formatter, "{name} exceeds the limit of {limit}")
            }
            Self::HashMismatch => {
                formatter.write_str("content hash does not match canonical content")
            }
            Self::NonCanonical => formatter.write_str("evidence JSON is not canonical"),
            Self::ResetRequired {
                stored_schema_version,
                guidance,
            } => write!(
                formatter,
                "reset-required (stored schema version: {}): {guidance}",
                stored_schema_version.as_deref().unwrap_or("unknown")
            ),
        }
    }
}

impl std::error::Error for ContractError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractLoadError {
    InvalidJson,
    TooLarge,
    NonCanonical,
    ResetRequired {
        stored_schema_version: Option<String>,
        guidance: String,
    },
    InvalidContract(String),
    HashMismatch,
}

impl std::fmt::Display for ContractLoadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ResetRequired { guidance, .. } => write!(formatter, "reset-required: {guidance}"),
            _ => formatter.write_str(self.code()),
        }
    }
}

impl std::error::Error for ContractLoadError {}

impl ContractLoadError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidJson => "invalid-factor-research-json",
            Self::TooLarge => "factor-research-json-too-large",
            Self::NonCanonical => "non-canonical-factor-research-json",
            Self::ResetRequired { .. } => "reset-required",
            Self::InvalidContract(_) => "invalid-factor-research-contract",
            Self::HashMismatch => "factor-research-hash-mismatch",
        }
    }
}

pub fn canonicalize_json(bytes: &[u8]) -> Result<Vec<u8>, ContractError> {
    if bytes.len() > MAX_CANONICAL_JSON_BYTES {
        return Err(ContractError::LimitExceeded {
            name: "canonical JSON",
            limit: MAX_CANONICAL_JSON_BYTES,
        });
    }
    adaq_feature_engine::canonicalize_json(bytes)
        .map_err(|error| ContractError::Invalid(format!("invalid canonical JSON: {error}")))
}

pub fn canonical_json<T: Serialize>(value: &T) -> Result<Vec<u8>, ContractError> {
    let bytes = serde_json::to_vec(value).map_err(|error| {
        ContractError::Invalid(format!("cannot serialize canonical JSON: {error}"))
    })?;
    canonicalize_json(&bytes)
}

pub fn content_hash<T: Serialize>(value: &T) -> Result<String, ContractError> {
    Ok(adaq_feature_engine::sha256(&canonical_json(value)?))
}

pub fn load_versioned_json<T: DeserializeOwned>(
    bytes: &[u8],
    expected_schema_version: &str,
) -> Result<T, ContractLoadError> {
    if bytes.len() > MAX_CANONICAL_JSON_BYTES {
        return Err(ContractLoadError::TooLarge);
    }
    let value: Value = serde_json::from_slice(bytes).map_err(|_| ContractLoadError::InvalidJson)?;
    let stored_schema_version = value
        .get("schemaVersion")
        .and_then(Value::as_str)
        .map(str::to_owned);
    if stored_schema_version.as_deref() != Some(expected_schema_version) {
        return Err(ContractLoadError::ResetRequired {
            stored_schema_version,
            guidance: "incompatible Factor research evidence requires an explicit device-level reset; no migration or automatic deletion is performed".into(),
        });
    }
    let canonical = canonicalize_json(bytes).map_err(|error| match error {
        ContractError::LimitExceeded { .. } => ContractLoadError::TooLarge,
        _ => ContractLoadError::InvalidJson,
    })?;
    if canonical != bytes {
        return Err(ContractLoadError::NonCanonical);
    }
    serde_json::from_slice(bytes).map_err(|_| ContractLoadError::InvalidJson)
}

pub fn checked_product(values: impl IntoIterator<Item = u64>) -> Result<u64, ContractError> {
    values.into_iter().try_fold(1u64, |total, value| {
        total.checked_mul(value).ok_or_else(|| {
            ContractError::Invalid("checked arithmetic overflow before allocation".into())
        })
    })
}

pub fn checked_row_count(instruments: u64, observations: u64) -> Result<u64, ContractError> {
    checked_product([instruments, observations])
}

pub fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

pub fn is_lower_kebab(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.as_bytes()[0].is_ascii_lowercase()
        && !value.ends_with('-')
        && value.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_hashes_are_order_independent_and_lowercase() {
        let left = serde_json::json!({"b": 1, "a": 2});
        let right = serde_json::json!({"a": 2, "b": 1});
        assert_eq!(content_hash(&left).unwrap(), content_hash(&right).unwrap());
        assert!(is_sha256(&content_hash(&left).unwrap()));
    }

    #[test]
    fn incompatible_evidence_requires_explicit_reset() {
        let error = load_versioned_json::<serde_json::Value>(
            br#"{"schemaVersion":"0.1.0"}"#,
            FACTOR_RESEARCH_SCHEMA_VERSION,
        )
        .unwrap_err();
        assert_eq!(error.code(), "reset-required");
        assert!(matches!(error, ContractLoadError::ResetRequired { .. }));
    }

    #[test]
    fn checked_row_count_rejects_overflow_before_allocation() {
        assert!(checked_row_count(u64::MAX, 2).is_err());
    }
}
