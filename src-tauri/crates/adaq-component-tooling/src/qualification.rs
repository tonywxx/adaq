//! M14's deterministic qualification boundary.
//!
//! Generation remains deliberately separate from this module: fixed generators
//! produce a `.adaq` archive, and this module is the last gate before import.
//! Every attempted parameter combination is retained in the returned evidence,
//! including failures, so callers can persist failed and superseded attempts.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    ComponentKind, ComponentPackage, ComponentParameterValue, RunLimits, component_parameters,
    conformance::verify_package_with_parameters_and_limits,
};

const MAX_PARAMETER_COMBINATIONS: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum QualificationGate {
    Package,
    Conformance,
    Equivalence,
    Qualified,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QualificationEvidence {
    pub combination: usize,
    pub parameter_values: HashMap<String, String>,
    pub parameter_hash: String,
    pub gate: QualificationGate,
    pub diagnostic: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QualificationAttempt {
    pub attempt_id: String,
    pub archive_sha256: String,
    pub component_id: String,
    pub version: String,
    pub kind: ComponentKind,
    pub qualified: bool,
    pub evidence: Vec<QualificationEvidence>,
}

/// Qualifies an immutable package over every allowed portable parameter
/// combination. The equivalence callback must compare the generated runtime
/// with the source research evidence for that exact combination.
pub fn qualify_package<F>(
    attempt_id: impl Into<String>,
    bytes: &[u8],
    equivalent: F,
) -> QualificationAttempt
where
    F: FnMut(&ComponentPackage, &[ComponentParameterValue]) -> Result<(), String>,
{
    qualify_package_with_limits(attempt_id, bytes, RunLimits::default(), equivalent)
}

/// Qualifies an immutable package with the Host resource limits frozen for
/// the producing research Attempt.
pub fn qualify_package_with_limits<F>(
    attempt_id: impl Into<String>,
    bytes: &[u8],
    limits: RunLimits,
    mut equivalent: F,
) -> QualificationAttempt
where
    F: FnMut(&ComponentPackage, &[ComponentParameterValue]) -> Result<(), String>,
{
    let attempt_id = attempt_id.into();
    let package = match ComponentPackage::read(bytes) {
        Ok(package) => package,
        Err(error) => {
            return failed_attempt(attempt_id, QualificationGate::Package, error.to_string());
        }
    };
    let identity = (
        package.archive_sha256.clone(),
        package.manifest.component_id.to_string(),
        package.manifest.version.to_string(),
        package.manifest.kind,
    );
    let combinations = match parameter_combinations(&package.manifest.parameters) {
        Ok(combinations) => combinations,
        Err(error) => {
            return attempt(
                &attempt_id,
                &package,
                false,
                vec![evidence(
                    0,
                    HashMap::new(),
                    QualificationGate::Package,
                    Some(error),
                )],
            );
        }
    };
    let mut records = Vec::with_capacity(combinations.len());
    let mut qualified = true;
    for (index, values) in combinations.into_iter().enumerate() {
        let typed = match component_parameters(&package.manifest, Some(&values)) {
            Ok(typed) => typed,
            Err(error) => {
                qualified = false;
                records.push(evidence(
                    index,
                    values,
                    QualificationGate::Package,
                    Some(error),
                ));
                continue;
            }
        };
        if let Err(error) =
            verify_package_with_parameters_and_limits(&package, Some(&values), limits)
        {
            qualified = false;
            records.push(evidence(
                index,
                values,
                QualificationGate::Conformance,
                Some(error),
            ));
            continue;
        }
        if let Err(error) = equivalent(&package, &typed) {
            qualified = false;
            records.push(evidence(
                index,
                values,
                QualificationGate::Equivalence,
                Some(error),
            ));
            continue;
        }
        records.push(evidence(index, values, QualificationGate::Qualified, None));
    }
    let mut result = attempt(&attempt_id, &package, qualified, records);
    if result.qualified {
        result.evidence.push(QualificationEvidence {
            combination: result.evidence.len(),
            parameter_values: HashMap::new(),
            parameter_hash: hash_values(&HashMap::new()),
            gate: QualificationGate::Qualified,
            diagnostic: Some("all portable parameter combinations qualified".into()),
        });
    }
    debug_assert_eq!(
        (
            &result.archive_sha256,
            &result.component_id,
            &result.version,
            result.kind
        ),
        (&identity.0, &identity.1, &identity.2, identity.3)
    );
    result
}

pub(crate) fn parameter_combinations(
    parameters: &[crate::ParameterDefinition],
) -> Result<Vec<HashMap<String, String>>, String> {
    let mut combinations = vec![HashMap::new()];
    for parameter in parameters {
        let values: Vec<String> = if parameter.allowed_values.is_empty() {
            vec![parameter.default_value.clone()]
        } else {
            parameter.allowed_values.clone()
        };
        let next_len = combinations
            .len()
            .checked_mul(values.len())
            .ok_or_else(|| "portable parameter combination count overflowed".to_owned())?;
        if next_len > MAX_PARAMETER_COMBINATIONS {
            return Err(format!(
                "portable parameter combination count exceeds {MAX_PARAMETER_COMBINATIONS}"
            ));
        }
        combinations = combinations
            .into_iter()
            .flat_map(|combination| {
                values.iter().map(move |value| {
                    let mut next = combination.clone();
                    next.insert(parameter.name.clone(), value.clone());
                    next
                })
            })
            .collect();
    }
    Ok(combinations)
}

fn evidence(
    combination: usize,
    parameter_values: HashMap<String, String>,
    gate: QualificationGate,
    diagnostic: Option<String>,
) -> QualificationEvidence {
    QualificationEvidence {
        combination,
        parameter_hash: hash_values(&parameter_values),
        parameter_values,
        gate,
        diagnostic,
    }
}

fn hash_values(values: &HashMap<String, String>) -> String {
    let mut ordered = values.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|(key, _)| *key);
    let mut digest = Sha256::new();
    for (key, value) in ordered {
        digest.update(key.as_bytes());
        digest.update([0]);
        digest.update(value.as_bytes());
        digest.update([0]);
    }
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn attempt(
    attempt_id: &str,
    package: &ComponentPackage,
    qualified: bool,
    evidence: Vec<QualificationEvidence>,
) -> QualificationAttempt {
    QualificationAttempt {
        attempt_id: attempt_id.into(),
        archive_sha256: package.archive_sha256.clone(),
        component_id: package.manifest.component_id.to_string(),
        version: package.manifest.version.to_string(),
        kind: package.manifest.kind,
        qualified,
        evidence,
    }
}

fn failed_attempt(
    attempt_id: String,
    gate: QualificationGate,
    diagnostic: String,
) -> QualificationAttempt {
    QualificationAttempt {
        attempt_id,
        archive_sha256: String::new(),
        component_id: String::new(),
        version: String::new(),
        kind: ComponentKind::Factor,
        qualified: false,
        evidence: vec![evidence(0, HashMap::new(), gate, Some(diagnostic))],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parameter_hash_is_order_independent() {
        let mut left = HashMap::new();
        left.insert("period".into(), "5".into());
        left.insert("smooth".into(), "true".into());
        let mut right = HashMap::new();
        right.insert("smooth".into(), "true".into());
        right.insert("period".into(), "5".into());
        assert_eq!(hash_values(&left), hash_values(&right));
    }

    #[test]
    fn combination_limit_is_explicit() {
        let parameters = (0..9)
            .map(|index| crate::ParameterDefinition {
                name: format!("p{index}"),
                parameter_type: crate::ParameterType::String,
                default_value: "a".into(),
                allowed_values: vec!["a".into(), "b".into(), "c".into(), "d".into()],
            })
            .collect::<Vec<_>>();
        assert!(parameter_combinations(&parameters).is_err());
    }
}
