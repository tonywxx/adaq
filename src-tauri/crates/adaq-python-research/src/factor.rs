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
pub struct PythonFactorInput {
    pub universe: Vec<String>,
    pub segments: Vec<PythonFactorSegment>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PythonFactorSegment {
    pub segment_id: String,
    pub batches: Vec<PythonFactorBatch>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PythonFactorBatch {
    pub rows: Vec<MomentumInputRow>,
}

impl PythonFactorInput {
    pub fn validate(&self) -> Result<(), PythonResearchError> {
        if self.universe.is_empty() || self.segments.is_empty() {
            return Err(invalid("python-factor-input-empty"));
        }
        let universe = self.universe.iter().cloned().collect::<BTreeSet<_>>();
        if universe.len() != self.universe.len()
            || self.universe.iter().any(|member| member.trim().is_empty())
        {
            return Err(invalid("python-factor-input-universe-invalid"));
        }
        let mut segments = BTreeSet::new();
        let mut rows = BTreeSet::new();
        let mut members_by_time = BTreeMap::<i64, BTreeSet<String>>::new();
        let universe_order = self
            .universe
            .iter()
            .enumerate()
            .map(|(index, member)| (member.as_str(), index))
            .collect::<BTreeMap<_, _>>();
        for segment in &self.segments {
            if segment.segment_id.trim().is_empty() || !segments.insert(&segment.segment_id) {
                return Err(invalid("python-factor-input-segment-invalid"));
            }
            if segment.batches.is_empty() {
                return Err(invalid("python-factor-input-batch-empty"));
            }
            let mut previous = None;
            let mut previous_universe_index = None;
            for batch in &segment.batches {
                if batch.rows.is_empty() {
                    return Err(invalid("python-factor-input-batch-empty"));
                }
                for row in &batch.rows {
                    if row.instrument_id.trim().is_empty()
                        || !universe.contains(&row.instrument_id)
                        || row.close.is_some_and(|value| !value.is_finite())
                        || !rows.insert((row.observation_time_ms, row.instrument_id.clone()))
                    {
                        return Err(invalid("python-factor-input-row-invalid"));
                    }
                    if previous.is_some_and(|previous| {
                        (row.observation_time_ms, row.instrument_id.as_str()) < previous
                    }) {
                        return Err(invalid("python-factor-input-order-invalid"));
                    }
                    let universe_index = universe_order[row.instrument_id.as_str()];
                    if previous.is_some_and(|previous| previous.0 == row.observation_time_ms)
                        && previous_universe_index
                            .is_some_and(|previous_index| universe_index <= previous_index)
                    {
                        return Err(invalid("python-factor-input-universe-order-invalid"));
                    }
                    previous = Some((row.observation_time_ms, row.instrument_id.as_str()));
                    previous_universe_index = Some(universe_index);
                    members_by_time
                        .entry(row.observation_time_ms)
                        .or_default()
                        .insert(row.instrument_id.clone());
                }
            }
        }
        if rows.is_empty()
            || members_by_time.values().any(|members| {
                members.len() != universe.len()
                    || members.iter().any(|member| !universe.contains(member))
            })
        {
            return Err(invalid(
                "python-factor-input-universe-membership-incomplete",
            ));
        }
        Ok(())
    }
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

pub fn validate_imperative_factor_payload(
    payload: &Value,
    input: &PythonFactorInput,
) -> Result<Vec<MomentumOutputRow>, PythonResearchError> {
    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "snake_case", deny_unknown_fields)]
    struct Payload {
        output_names: Vec<String>,
        outputs: Vec<OutputBatch>,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "snake_case", deny_unknown_fields)]
    struct OutputBatch {
        segment_id: String,
        rows: Vec<OutputRow>,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "snake_case", deny_unknown_fields)]
    struct OutputRow {
        instrument_id: String,
        event_time_ms: i64,
        value: Value,
    }

    input.validate()?;
    let payload = serde_json::from_value::<Payload>(payload.clone())
        .map_err(|error| invalid(format!("python-factor-output-invalid:{error}")))?;
    if payload.output_names.len() != 1 || payload.output_names[0] != MOMENTUM_OUTPUT_ID {
        return Err(invalid("python-factor-output-names-invalid"));
    }
    let mut actual_segment_ids = Vec::new();
    for batch in &payload.outputs {
        if batch.rows.is_empty() || actual_segment_ids.last() != Some(&batch.segment_id) {
            actual_segment_ids.push(batch.segment_id.clone());
        }
    }
    let expected_segment_ids = input
        .segments
        .iter()
        .map(|segment| segment.segment_id.clone())
        .collect::<Vec<_>>();
    if actual_segment_ids != expected_segment_ids {
        return Err(invalid("python-factor-output-segment-invalid"));
    }

    let expected = input
        .segments
        .iter()
        .flat_map(|segment| segment.batches.iter().flat_map(|batch| &batch.rows))
        .collect::<Vec<_>>();
    let actual = payload
        .outputs
        .iter()
        .flat_map(|batch| batch.rows.iter())
        .collect::<Vec<_>>();
    if actual.len() != expected.len() {
        return Err(invalid("python-factor-output-row-count-invalid"));
    }
    let mut output = Vec::with_capacity(expected.len());
    for (expected_row, actual_row) in expected.iter().zip(actual) {
        if expected_row.instrument_id != actual_row.instrument_id
            || expected_row.observation_time_ms != actual_row.event_time_ms
        {
            return Err(invalid("python-factor-output-identity-or-order-invalid"));
        }
        let (value, unavailable_reason) = if let Some(value) = actual_row.value.as_f64() {
            if !value.is_finite() {
                return Err(invalid("python-factor-output-non-finite"));
            }
            (Some(value), None)
        } else {
            let object = actual_row
                .value
                .as_object()
                .ok_or_else(|| invalid("python-factor-output-value-invalid"))?;
            if object.len() != 1 {
                return Err(invalid("python-factor-output-unavailable-invalid"));
            }
            let reason = object
                .get("reason")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("python-factor-output-unavailable-invalid"))?;
            let reason = match reason {
                "warmup" => FactorUnavailableReason::Warmup,
                "missing-input" => FactorUnavailableReason::MissingInput,
                "bar-gap" => FactorUnavailableReason::BarGap,
                _ => return Err(invalid("python-factor-output-unavailable-reason-invalid")),
            };
            (None, Some(reason))
        };
        if expected_row.close.is_none()
            && unavailable_reason != Some(FactorUnavailableReason::MissingInput)
        {
            return Err(invalid("python-factor-missing-input-not-preserved"));
        }
        if expected_row.close.is_some()
            && unavailable_reason == Some(FactorUnavailableReason::MissingInput)
        {
            return Err(invalid("python-factor-spurious-missing-input"));
        }
        output.push(MomentumOutputRow {
            instrument_id: actual_row.instrument_id.clone(),
            observation_time_ms: actual_row.event_time_ms,
            value,
            unavailable_reason,
        });
    }
    validate_momentum_output(&output, expected.len())?;
    Ok(output)
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
    fn input_requires_declared_universe_order() {
        let input = PythonFactorInput {
            universe: vec!["BBB".into(), "AAA".into()],
            segments: vec![PythonFactorSegment {
                segment_id: "continuous-1".into(),
                batches: vec![PythonFactorBatch { rows: rows() }],
            }],
        };
        assert_eq!(
            input.validate().unwrap_err().to_string(),
            "python-factor-input-universe-order-invalid"
        );
    }

    #[test]
    fn present_inputs_cannot_report_spurious_missing_input() {
        let input = PythonFactorInput {
            universe: vec!["AAA".into(), "BBB".into()],
            segments: vec![PythonFactorSegment {
                segment_id: "continuous-1".into(),
                batches: vec![PythonFactorBatch {
                    rows: vec![
                        MomentumInputRow {
                            instrument_id: "AAA".into(),
                            observation_time_ms: 1,
                            close: Some(1.0),
                        },
                        MomentumInputRow {
                            instrument_id: "BBB".into(),
                            observation_time_ms: 1,
                            close: Some(2.0),
                        },
                    ],
                }],
            }],
        };
        let payload = serde_json::json!({
            "output_names": ["momentum-score"],
            "outputs": [{
                "segment_id": "continuous-1",
                "rows": [
                    {"instrument_id": "AAA", "event_time_ms": 1, "value": {"reason": "missing-input"}},
                    {"instrument_id": "BBB", "event_time_ms": 1, "value": 0.5}
                ]
            }]
        });
        assert_eq!(
            validate_imperative_factor_payload(&payload, &input)
                .unwrap_err()
                .to_string(),
            "python-factor-spurious-missing-input"
        );
    }

    #[test]
    fn present_inputs_may_report_warmup_before_full_window() {
        let input = PythonFactorInput {
            universe: vec!["AAA".into(), "BBB".into()],
            segments: vec![PythonFactorSegment {
                segment_id: "continuous-1".into(),
                batches: vec![PythonFactorBatch {
                    rows: vec![
                        MomentumInputRow {
                            instrument_id: "AAA".into(),
                            observation_time_ms: 1,
                            close: Some(1.0),
                        },
                        MomentumInputRow {
                            instrument_id: "BBB".into(),
                            observation_time_ms: 1,
                            close: Some(2.0),
                        },
                    ],
                }],
            }],
        };
        let payload = serde_json::json!({
            "output_names": ["momentum-score"],
            "outputs": [{
                "segment_id": "continuous-1",
                "rows": [
                    {"instrument_id": "AAA", "event_time_ms": 1, "value": {"reason": "warmup"}},
                    {"instrument_id": "BBB", "event_time_ms": 1, "value": {"reason": "warmup"}}
                ]
            }]
        });
        assert!(validate_imperative_factor_payload(&payload, &input).is_ok());
    }

    #[test]
    fn golden_output_preserves_cross_sectional_percentiles() {
        let input = (1..=6)
            .flat_map(|time| {
                [
                    ("AAA", time as f64),
                    ("BBB", (time * 2) as f64),
                    ("CCC", (time * time) as f64),
                ]
                .into_iter()
                .map(move |(instrument, close)| MomentumInputRow {
                    instrument_id: instrument.into(),
                    observation_time_ms: time,
                    close: Some(close),
                })
            })
            .collect::<Vec<_>>();
        let output =
            materialize_momentum(&input, &["AAA".into(), "BBB".into(), "CCC".into()], 5).unwrap();
        let final_rows = output
            .into_iter()
            .filter(|row| row.observation_time_ms == 6)
            .collect::<Vec<_>>();
        assert_eq!(
            final_rows
                .iter()
                .map(|row| (row.instrument_id.as_str(), row.value))
                .collect::<Vec<_>>(),
            vec![
                ("AAA", Some(2.0 / 3.0)),
                ("BBB", Some(2.0 / 3.0)),
                ("CCC", Some(1.0))
            ]
        );
    }

    #[test]
    fn exact_replay_produces_an_identity_report() {
        let first = materialize_momentum(&rows(), &["AAA".into(), "BBB".into()], 5).unwrap();
        let report = RepeatabilityReport::exact(&first, &first).unwrap();
        assert!(report.exact);
        assert_eq!(report.first_output_sha256, report.replay_output_sha256);
    }

    #[test]
    fn divergent_replay_remains_explicitly_unverified() {
        let first = materialize_momentum(&rows(), &["AAA".into(), "BBB".into()], 5).unwrap();
        let mut replay = first.clone();
        replay[0].unavailable_reason = Some(FactorUnavailableReason::BarGap);
        let report = RepeatabilityReport::exact(&first, &replay).unwrap();
        assert!(!report.exact);
        assert_ne!(report.first_output_sha256, report.replay_output_sha256);
    }
}
