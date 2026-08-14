//! Host-side conformance helpers for the M12 Python momentum Factor.

use crate::{PythonResearchError, invalid, is_sha256, sha256};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const MOMENTUM_PROJECT_ID: &str = "py-factor-cross-sectional-momentum";
pub const MOMENTUM_OUTPUT_ID: &str = "momentum-score";
pub const MOMENTUM_INPUT_ID: &str = "close";
pub const MOMENTUM_LOOKBACKS: [u32; 3] = [5, 20, 60];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PythonFactorMode {
    ImperativePython,
    PortableDefinition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MomentumBinding {
    pub project_revision_sha256: String,
    pub environment_sha256: String,
    pub snapshot_id: String,
    pub point_in_time_universe_id: String,
    pub feature_evidence_sha256: String,
    pub sdk_artifact_sha256: String,
    pub seed: u64,
    pub mode: PythonFactorMode,
}

impl MomentumBinding {
    pub fn validate(&self) -> Result<(), PythonResearchError> {
        if !is_sha256(&self.project_revision_sha256)
            || !is_sha256(&self.environment_sha256)
            || self.snapshot_id.trim().is_empty()
            || self.point_in_time_universe_id.trim().is_empty()
            || !is_sha256(&self.feature_evidence_sha256)
            || !is_sha256(&self.sdk_artifact_sha256)
        {
            return Err(invalid("python-factor-binding-invalid"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PortableMomentumDefinition {
    pub operator_catalog_version: String,
    pub nodes: Vec<String>,
    pub output_id: String,
}

impl PortableMomentumDefinition {
    pub fn canonical() -> Self {
        Self {
            operator_catalog_version: "adaq-feature-operator-catalog@1.0.0".into(),
            nodes: vec![
                "market-close".into(),
                "backward-simple-return".into(),
                "cross-sectional-percentile".into(),
            ],
            output_id: MOMENTUM_OUTPUT_ID.into(),
        }
    }

    pub fn validate(&self) -> Result<(), PythonResearchError> {
        if self != &Self::canonical() {
            return Err(invalid("python-factor-portable-definition-invalid"));
        }
        Ok(())
    }
}

pub fn validate_portable_definition_payload(
    payload: &Value,
) -> Result<PortableMomentumDefinition, PythonResearchError> {
    #[derive(Debug, Deserialize, PartialEq, Eq)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Definition {
        scope: String,
        nodes: Vec<Node>,
        outputs: Vec<String>,
    }

    #[derive(Debug, Deserialize, PartialEq, Eq)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Node {
        op: String,
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        input: Option<String>,
        #[serde(default)]
        parameter: Option<String>,
        #[serde(default)]
        output: Option<String>,
    }

    let definition = payload
        .get("definition")
        .ok_or_else(|| invalid("python-factor-definition-missing"))
        .and_then(|value| {
            serde_json::from_value::<Definition>(value.clone())
                .map_err(|error| invalid(format!("python-factor-definition-invalid:{error}")))
        })?;
    let expected = vec![
        Node {
            op: "market-close".into(),
            id: Some("close".into()),
            input: None,
            parameter: None,
            output: None,
        },
        Node {
            op: "backward-simple-return".into(),
            id: None,
            input: Some("close".into()),
            parameter: Some("lookback".into()),
            output: None,
        },
        Node {
            op: "cross-sectional-percentile".into(),
            id: None,
            input: Some("return".into()),
            parameter: None,
            output: None,
        },
        Node {
            op: "rename".into(),
            id: None,
            input: Some("percentile".into()),
            parameter: None,
            output: Some(MOMENTUM_OUTPUT_ID.into()),
        },
    ];
    if definition.scope != "cross-sectional"
        || definition.outputs != [MOMENTUM_OUTPUT_ID]
        || definition.nodes != expected
    {
        return Err(invalid("python-factor-portable-definition-invalid"));
    }
    PortableMomentumDefinition::canonical().validate()?;
    Ok(PortableMomentumDefinition::canonical())
}

pub fn expand_momentum_grid() -> Vec<u32> {
    MOMENTUM_LOOKBACKS.to_vec()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FactorUnavailableReason {
    Warmup,
    MissingInput,
    BarGap,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MomentumInputRow {
    pub instrument_id: String,
    pub observation_time_ms: i64,
    pub close: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MomentumOutputRow {
    pub instrument_id: String,
    pub observation_time_ms: i64,
    pub value: Option<f64>,
    pub unavailable_reason: Option<FactorUnavailableReason>,
}

pub fn materialize_momentum(
    rows: &[MomentumInputRow],
    universe: &[String],
    lookback: u32,
) -> Result<Vec<MomentumOutputRow>, PythonResearchError> {
    if !MOMENTUM_LOOKBACKS.contains(&lookback) || universe.is_empty() {
        return Err(invalid("python-factor-lookback-or-universe-invalid"));
    }
    let mut members = BTreeSet::new();
    if universe
        .iter()
        .any(|instrument| !members.insert(instrument))
    {
        return Err(invalid("python-factor-universe-duplicate"));
    }
    let mut ordered = rows.to_vec();
    ordered.sort_by(|left, right| {
        (left.observation_time_ms, left.instrument_id.as_str())
            .cmp(&(right.observation_time_ms, right.instrument_id.as_str()))
    });
    let mut by_instrument = BTreeMap::<String, Vec<MomentumInputRow>>::new();
    for row in &ordered {
        if row.instrument_id.trim().is_empty() || !members.contains(&row.instrument_id) {
            return Err(invalid("python-factor-row-outside-universe"));
        }
        by_instrument
            .entry(row.instrument_id.clone())
            .or_default()
            .push(row.clone());
    }
    let expected_times = by_instrument
        .values()
        .next()
        .map(|rows| {
            rows.iter()
                .map(|row| row.observation_time_ms)
                .collect::<Vec<_>>()
        })
        .ok_or_else(|| invalid("python-factor-input-empty"))?;
    if by_instrument.len() != universe.len()
        || by_instrument.values().any(|rows| {
            rows.windows(2)
                .any(|rows| rows[0].observation_time_ms >= rows[1].observation_time_ms)
                || rows
                    .iter()
                    .map(|row| row.observation_time_ms)
                    .collect::<Vec<_>>()
                    != expected_times
        })
    {
        return Err(invalid("python-factor-universe-membership-incomplete"));
    }
    let mut returns =
        BTreeMap::<i64, BTreeMap<String, (Option<f64>, FactorUnavailableReason)>>::new();
    for instrument in universe {
        let history = by_instrument
            .get(instrument)
            .ok_or_else(|| invalid("python-factor-universe-member-missing"))?;
        for index in 0..history.len() {
            let row = &history[index];
            let result = if index < lookback as usize {
                (None, FactorUnavailableReason::Warmup)
            } else {
                match (row.close, history[index - lookback as usize].close) {
                    (Some(current), Some(previous))
                        if current.is_finite() && previous.is_finite() && previous != 0.0 =>
                    {
                        let value = current / previous - 1.0;
                        if value.is_finite() {
                            (Some(value), FactorUnavailableReason::Warmup)
                        } else {
                            (None, FactorUnavailableReason::MissingInput)
                        }
                    }
                    (None, _) | (_, None) => (None, FactorUnavailableReason::MissingInput),
                    _ => (None, FactorUnavailableReason::MissingInput),
                }
            };
            returns
                .entry(row.observation_time_ms)
                .or_default()
                .insert(instrument.clone(), result);
        }
    }
    let mut output = Vec::with_capacity(ordered.len());
    for row in ordered {
        let members = returns
            .get(&row.observation_time_ms)
            .ok_or_else(|| invalid("python-factor-return-time-missing"))?;
        if members.len() != universe.len() {
            return Err(invalid("python-factor-return-universe-incomplete"));
        }
        let available = members
            .values()
            .filter_map(|(value, _)| *value)
            .collect::<Vec<_>>();
        let (value, reason) = members
            .get(&row.instrument_id)
            .ok_or_else(|| invalid("python-factor-output-member-missing"))?;
        let percentile = value.map(|value| {
            available.iter().filter(|other| **other <= value).count() as f64
                / available.len().max(1) as f64
        });
        output.push(MomentumOutputRow {
            instrument_id: row.instrument_id,
            observation_time_ms: row.observation_time_ms,
            value: percentile,
            unavailable_reason: percentile.is_none().then(|| reason.clone()),
        });
    }
    validate_momentum_output(&output, rows.len())?;
    output.sort_by(|left, right| {
        (left.instrument_id.as_str(), left.observation_time_ms)
            .cmp(&(right.instrument_id.as_str(), right.observation_time_ms))
    });
    Ok(output)
}

pub fn validate_momentum_output(
    rows: &[MomentumOutputRow],
    expected_count: usize,
) -> Result<(), PythonResearchError> {
    if rows.len() != expected_count
        || rows.iter().any(|row| {
            row.instrument_id.trim().is_empty()
                || row.value.is_some_and(|value| !value.is_finite())
                || row.value.is_some() == row.unavailable_reason.is_some()
        })
    {
        return Err(invalid("python-factor-output-schema-invalid"));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepeatabilityReport {
    pub first_output_sha256: String,
    pub replay_output_sha256: String,
    pub exact: bool,
}

impl RepeatabilityReport {
    pub fn exact(
        first: &[MomentumOutputRow],
        replay: &[MomentumOutputRow],
    ) -> Result<Self, PythonResearchError> {
        let first_bytes = serde_json::to_vec(first).map_err(|error| invalid(error.to_string()))?;
        let replay_bytes =
            serde_json::to_vec(replay).map_err(|error| invalid(error.to_string()))?;
        let first_output_sha256 = sha256(&first_bytes);
        let replay_output_sha256 = sha256(&replay_bytes);
        Ok(Self {
            exact: first_output_sha256 == replay_output_sha256,
            first_output_sha256,
            replay_output_sha256,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> Vec<MomentumInputRow> {
        [1, 2, 3]
            .into_iter()
            .flat_map(|time| {
                ["AAA", "BBB"]
                    .into_iter()
                    .map(move |instrument| MomentumInputRow {
                        instrument_id: instrument.into(),
                        observation_time_ms: time,
                        close: Some(if instrument == "AAA" {
                            time as f64
                        } else {
                            (time * 2) as f64
                        }),
                    })
            })
            .collect()
    }

    #[test]
    fn canonical_definition_and_grid_are_host_owned() {
        PortableMomentumDefinition::canonical().validate().unwrap();
        assert_eq!(expand_momentum_grid(), vec![5, 20, 60]);
        assert!(
            PortableMomentumDefinition {
                nodes: vec!["loop".into()],
                ..PortableMomentumDefinition::canonical()
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn runner_definition_payload_accepts_only_the_canonical_graph() {
        let payload = serde_json::json!({
            "definition": {
                "scope": "cross-sectional",
                "nodes": [
                    {"op": "market-close", "id": "close"},
                    {"op": "backward-simple-return", "input": "close", "parameter": "lookback"},
                    {"op": "cross-sectional-percentile", "input": "return"},
                    {"op": "rename", "input": "percentile", "output": "momentum-score"}
                ],
                "outputs": ["momentum-score"]
            }
        });
        assert!(validate_portable_definition_payload(&payload).is_ok());
        assert!(validate_portable_definition_payload(&serde_json::json!({
            "definition": {"scope": "cross-sectional", "nodes": [{"op": "loop"}], "outputs": ["momentum-score"]}
        }))
        .is_err());
    }

    #[test]
    fn output_preserves_universe_and_typed_warmup() {
        let output = materialize_momentum(&rows(), &["AAA".into(), "BBB".into()], 5).unwrap();
        assert_eq!(output.len(), 6);
        assert!(output.iter().all(|row| row.value.is_none()));
        assert!(
            output
                .iter()
                .all(|row| row.unavailable_reason == Some(FactorUnavailableReason::Warmup))
        );
        assert!(validate_momentum_output(&output, 6).is_ok());
    }

    #[test]
    fn exact_replay_produces_an_identity_report() {
        let first = materialize_momentum(&rows(), &["AAA".into(), "BBB".into()], 5).unwrap();
        let report = RepeatabilityReport::exact(&first, &first).unwrap();
        assert!(report.exact);
        assert_eq!(report.first_output_sha256, report.replay_output_sha256);
    }
}
