//! Forecast Evaluation: immutable Forecast Evaluation Reports over Forecast
//! Signal Datasets, realized Forecast Targets, Evidence State, Scale
//! Provenance, and the metric catalog (expected value / probability / score).
//!
//! Domain terms: Forecast Evaluation Report, Evaluation Evidence State,
//! Forecast Target, Forecast Value Scale, Prediction Kind. See CONTEXT.md.

use std::{collections::BTreeMap, fs, sync::Arc};

use rusqlite::params;
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};

use crate::forecast_signal_dataset::{
    dataset_outputs, hash, producer_segment_values, read_external_rows, string, ComponentLockEntry,
    SignalDataset,
};
use crate::local_research::LocalResearchState;
use crate::user::validate_user;

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForecastEvaluationRequest {
    pub user_id: String,
    pub dataset_id: String,
    pub snapshot_id: String,
    pub signal_name: String,
    pub horizon_bars: u32,
    pub evaluation_start_time_ms: i64,
    pub evaluation_end_time_ms: i64,
    pub stability_window_bars: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DistributionMetrics {
    count: usize,
    minimum: f64,
    maximum: f64,
    mean: f64,
    standard_deviation: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CalibrationBucket {
    bucket_index: usize,
    lower_bound: f64,
    upper_bound: f64,
    count: usize,
    mean_prediction: Option<f64>,
    observed_frequency: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScoreQuantile {
    quantile: usize,
    count: usize,
    minimum_prediction: Option<f64>,
    maximum_prediction: Option<f64>,
    mean_prediction: Option<f64>,
    mean_realized_target: Option<f64>,
    realized_target_distribution: Option<DistributionMetrics>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum ScaleProvenance {
    TrainingFrozen {
        transform_id: String,
        reference_distribution_id: String,
        parameters: BTreeMap<String, serde_json::Value>,
    },
    PastOnlyRolling {
        transform_id: String,
        parameters: BTreeMap<String, serde_json::Value>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ForecastMetrics {
    evaluation_row_count: usize,
    aligned_count: usize,
    unavailable_prediction_count: usize,
    unavailable_label_count: usize,
    coverage: f64,
    missingness: f64,
    prediction_distribution: Option<DistributionMetrics>,
    realized_distribution: Option<DistributionMetrics>,
    mae: Option<f64>,
    rmse: Option<f64>,
    mean_bias: Option<f64>,
    pearson_correlation: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    brier_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    log_loss: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    roc_auc: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    calibration: Option<Vec<CalibrationBucket>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pearson_ic: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    spearman_rank_ic: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    window_icir: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    quantiles: Option<Vec<ScoreQuantile>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    undefined_metrics: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EvaluationEvidenceState {
    pub(crate) summary: String,
    segment_states: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForecastEvaluationReport {
    report_id: String,
    dataset_id: String,
    snapshot_id: String,
    signal_name: String,
    signal_contract: adaq_component_tooling::ModelOutput,
    evaluation_start_time_ms: i64,
    evaluation_end_time_ms: i64,
    stability_window_bars: usize,
    metrics: ForecastMetrics,
    stability_windows: Vec<serde_json::Value>,
    evidence_state: EvaluationEvidenceState,
    unavailable_rows: Vec<serde_json::Value>,
    producer_segments: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    scale_provenance: Vec<serde_json::Value>,
    trust_state: String,
    metric_versions: BTreeMap<String, String>,
    engine_identity: adaq_component_tooling::EngineIdentity,
    schema_identity: String,
    dataset_parquet_sha256: String,
    #[serde(default)]
    component_lock: Vec<ComponentLockEntry>,
    #[serde(default)]
    feature_plan_hash: String,
}

fn realize_future_close_target(
    bars: &[adaq_data_core::OhlcvBar],
    gaps: &[adaq_data_core::BarGap],
    horizon_bars: u32,
    realize: impl Fn(f64, f64) -> f64,
) -> Result<Vec<Option<f64>>, String> {
    if horizon_bars == 0 {
        return Err("forecast-evaluation-horizon-must-be-positive".into());
    }
    let horizon = usize::try_from(horizon_bars).map_err(string)?;
    Ok(bars
        .iter()
        .enumerate()
        .map(|(index, origin)| {
            let future = bars.get(index.checked_add(horizon)?)?;
            if gaps.iter().any(|gap| {
                gap.start_time_ms > origin.open_time_ms && gap.end_time_ms <= future.open_time_ms
            }) {
                return None;
            }
            let origin = origin.close.to_f64()?;
            let future = future.close.to_f64()?;
            (origin.is_finite() && future.is_finite() && origin > 0.0 && future > 0.0)
                .then(|| realize(origin, future))
        })
        .collect())
}

fn realize_future_close_returns(
    bars: &[adaq_data_core::OhlcvBar],
    gaps: &[adaq_data_core::BarGap],
    horizon_bars: u32,
) -> Result<Vec<Option<f64>>, String> {
    realize_future_close_target(bars, gaps, horizon_bars, |origin, future| {
        future / origin - 1.0
    })
}

fn realize_future_close_up(
    bars: &[adaq_data_core::OhlcvBar],
    gaps: &[adaq_data_core::BarGap],
    horizon_bars: u32,
) -> Result<Vec<Option<f64>>, String> {
    realize_future_close_target(bars, gaps, horizon_bars, |origin, future| {
        f64::from(future > origin)
    })
}

fn distribution(values: &[f64]) -> Result<DistributionMetrics, String> {
    if values.is_empty() || values.iter().any(|value| !value.is_finite()) {
        return Err("forecast-evaluation-has-no-aligned-finite-rows".into());
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let variance = values
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / values.len() as f64;
    Ok(DistributionMetrics {
        count: values.len(),
        minimum: values.iter().copied().fold(f64::INFINITY, f64::min),
        maximum: values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        mean,
        standard_deviation: variance.sqrt(),
    })
}

fn pearson(pairs: &[(f64, f64)]) -> Option<f64> {
    if pairs.len() < 2 {
        return None;
    }
    let prediction_mean = pairs.iter().map(|pair| pair.0).sum::<f64>() / pairs.len() as f64;
    let realized_mean = pairs.iter().map(|pair| pair.1).sum::<f64>() / pairs.len() as f64;
    let numerator = pairs
        .iter()
        .map(|pair| (pair.0 - prediction_mean) * (pair.1 - realized_mean))
        .sum::<f64>();
    let prediction_sum = pairs
        .iter()
        .map(|pair| (pair.0 - prediction_mean).powi(2))
        .sum::<f64>();
    let realized_sum = pairs
        .iter()
        .map(|pair| (pair.1 - realized_mean).powi(2))
        .sum::<f64>();
    let denominator = (prediction_sum * realized_sum).sqrt();
    (denominator > 0.0).then_some(numerator / denominator)
}

fn expected_value_metrics(
    pairs: &[(f64, f64)],
    evaluation_row_count: usize,
    unavailable_prediction_count: usize,
) -> Result<ForecastMetrics, String> {
    if evaluation_row_count == 0
        || pairs.len() > evaluation_row_count
        || unavailable_prediction_count > evaluation_row_count
        || pairs
            .iter()
            .any(|pair| !pair.0.is_finite() || !pair.1.is_finite())
    {
        return Err("forecast-evaluation-metric-input-is-invalid".into());
    }
    let predictions = pairs.iter().map(|pair| pair.0).collect::<Vec<_>>();
    let realized = pairs.iter().map(|pair| pair.1).collect::<Vec<_>>();
    let errors = pairs.iter().map(|pair| pair.0 - pair.1).collect::<Vec<_>>();
    let coverage = pairs.len() as f64 / evaluation_row_count as f64;
    let errors = (!pairs.is_empty()).then(|| {
        let count = pairs.len() as f64;
        (
            errors.iter().map(|value| value.abs()).sum::<f64>() / count,
            (errors.iter().map(|value| value.powi(2)).sum::<f64>() / count).sqrt(),
            errors.iter().sum::<f64>() / count,
        )
    });
    Ok(ForecastMetrics {
        evaluation_row_count,
        aligned_count: pairs.len(),
        unavailable_prediction_count,
        unavailable_label_count: evaluation_row_count
            .saturating_sub(pairs.len() + unavailable_prediction_count),
        coverage,
        missingness: 1.0 - coverage,
        prediction_distribution: (!predictions.is_empty())
            .then(|| distribution(&predictions))
            .transpose()?,
        realized_distribution: (!realized.is_empty())
            .then(|| distribution(&realized))
            .transpose()?,
        mae: errors.map(|metrics| metrics.0),
        rmse: errors.map(|metrics| metrics.1),
        mean_bias: errors.map(|metrics| metrics.2),
        pearson_correlation: pearson(pairs),
        brier_score: None,
        log_loss: None,
        roc_auc: None,
        calibration: None,
        pearson_ic: None,
        spearman_rank_ic: None,
        window_icir: None,
        quantiles: None,
        undefined_metrics: BTreeMap::new(),
    })
}

fn average_ranks(values: &[f64]) -> Vec<f64> {
    let mut order = (0..values.len()).collect::<Vec<_>>();
    order.sort_by(|left, right| values[*left].total_cmp(&values[*right]));
    let mut ranks = vec![0.0; values.len()];
    let mut start = 0;
    while start < order.len() {
        let mut end = start + 1;
        while end < order.len() && values[order[start]] == values[order[end]] {
            end += 1;
        }
        let rank = (start + 1 + end) as f64 / 2.0;
        for index in &order[start..end] {
            ranks[*index] = rank;
        }
        start = end;
    }
    ranks
}

fn score_quantiles(pairs: &[(f64, f64)]) -> Result<Vec<ScoreQuantile>, String> {
    let mut ranked = pairs.to_vec();
    ranked.sort_by(|left, right| left.0.total_cmp(&right.0).then(left.1.total_cmp(&right.1)));
    let mut buckets = vec![Vec::new(); 5];
    let mut start = 0;
    while start < ranked.len() {
        let mut end = start + 1;
        while end < ranked.len() && ranked[start].0 == ranked[end].0 {
            end += 1;
        }
        let bucket = (start * 5 / ranked.len()).min(4);
        buckets[bucket].extend_from_slice(&ranked[start..end]);
        start = end;
    }
    buckets
        .into_iter()
        .enumerate()
        .map(|(index, values)| {
            let predictions = values.iter().map(|pair| pair.0).collect::<Vec<_>>();
            let realized = values.iter().map(|pair| pair.1).collect::<Vec<_>>();
            let count = values.len();
            Ok(ScoreQuantile {
                quantile: index + 1,
                count,
                minimum_prediction: predictions.first().copied(),
                maximum_prediction: predictions.last().copied(),
                mean_prediction: (count > 0)
                    .then(|| predictions.iter().sum::<f64>() / count as f64),
                mean_realized_target: (count > 0)
                    .then(|| realized.iter().sum::<f64>() / count as f64),
                realized_target_distribution: (count > 0)
                    .then(|| distribution(&realized))
                    .transpose()?,
            })
        })
        .collect()
}

fn score_metrics(
    pairs: &[(f64, f64)],
    evaluation_row_count: usize,
    unavailable_prediction_count: usize,
    window_ics: &[Option<f64>],
) -> Result<ForecastMetrics, String> {
    let mut metrics =
        expected_value_metrics(pairs, evaluation_row_count, unavailable_prediction_count)?;
    metrics.mae = None;
    metrics.rmse = None;
    metrics.mean_bias = None;
    metrics.pearson_correlation = None;
    metrics.pearson_ic = pearson(pairs);
    let prediction_ranks = average_ranks(&pairs.iter().map(|pair| pair.0).collect::<Vec<_>>());
    let target_ranks = average_ranks(&pairs.iter().map(|pair| pair.1).collect::<Vec<_>>());
    let rank_pairs = prediction_ranks
        .into_iter()
        .zip(target_ranks)
        .collect::<Vec<_>>();
    metrics.spearman_rank_ic = pearson(&rank_pairs);
    metrics.quantiles = Some(score_quantiles(pairs)?);
    if pairs.len() < 5 {
        metrics.undefined_metrics.insert(
            "quantiles".into(),
            "requires-at-least-five-aligned-samples".into(),
        );
    } else if pairs
        .first()
        .is_some_and(|first| pairs.iter().all(|pair| pair.0 == first.0))
    {
        metrics.undefined_metrics.insert(
            "quantiles".into(),
            "requires-non-constant-score-series".into(),
        );
    }
    if metrics.pearson_ic.is_none() {
        metrics.undefined_metrics.insert(
            "pearsonIc".into(),
            "requires-two-non-constant-series".into(),
        );
    }
    if metrics.spearman_rank_ic.is_none() {
        metrics.undefined_metrics.insert(
            "spearmanRankIc".into(),
            "requires-two-non-constant-series".into(),
        );
    }
    if !window_ics.is_empty() {
        let values = window_ics.iter().flatten().copied().collect::<Vec<_>>();
        if values.len() >= 2 {
            let mean = values.iter().sum::<f64>() / values.len() as f64;
            let deviation = (values
                .iter()
                .map(|value| (value - mean).powi(2))
                .sum::<f64>()
                / values.len() as f64)
                .sqrt();
            if deviation > 0.0 {
                metrics.window_icir = Some(mean / deviation);
            }
        }
        if metrics.window_icir.is_none() {
            metrics.undefined_metrics.insert(
                "windowIcir".into(),
                "requires-two-non-constant-window-ics".into(),
            );
        }
    }
    Ok(metrics)
}

fn probability_metrics(
    pairs: &[(f64, f64)],
    evaluation_row_count: usize,
    unavailable_prediction_count: usize,
) -> Result<ForecastMetrics, String> {
    if pairs.iter().any(|(prediction, realized)| {
        !prediction.is_finite()
            || !(0.0..=1.0).contains(prediction)
            || !matches!(*realized, 0.0 | 1.0)
    }) {
        return Err("forecast-evaluation-probability-is-out-of-bounds".into());
    }
    let mut metrics =
        expected_value_metrics(pairs, evaluation_row_count, unavailable_prediction_count)?;
    metrics.mae = None;
    metrics.rmse = None;
    metrics.mean_bias = None;
    metrics.pearson_correlation = None;
    if pairs.is_empty() {
        metrics.undefined_metrics.insert(
            "probabilityMetrics".into(),
            "requires-verifiable-realized-labels".into(),
        );
        return Ok(metrics);
    }
    let count = pairs.len() as f64;
    metrics.brier_score = Some(
        pairs
            .iter()
            .map(|(prediction, realized)| (prediction - realized).powi(2))
            .sum::<f64>()
            / count,
    );
    const LOG_LOSS_EPSILON: f64 = 1e-15;
    metrics.log_loss = Some(
        -pairs
            .iter()
            .map(|(prediction, realized)| {
                let prediction = prediction.clamp(LOG_LOSS_EPSILON, 1.0 - LOG_LOSS_EPSILON);
                realized * prediction.ln() + (1.0 - realized) * (1.0 - prediction).ln()
            })
            .sum::<f64>()
            / count,
    );
    let positives = pairs.iter().filter(|pair| pair.1 == 1.0).count();
    let negatives = pairs.len() - positives;
    if positives == 0 || negatives == 0 {
        metrics
            .undefined_metrics
            .insert("rocAuc".into(), "requires-both-realized-classes".into());
    } else {
        let mut ranked = pairs.to_vec();
        ranked.sort_by(|left, right| left.0.total_cmp(&right.0));
        let mut positive_rank_sum = 0.0;
        let mut start = 0;
        while start < ranked.len() {
            let mut end = start + 1;
            while end < ranked.len() && ranked[start].0 == ranked[end].0 {
                end += 1;
            }
            let average_rank = (start + 1 + end) as f64 / 2.0;
            positive_rank_sum += ranked[start..end]
                .iter()
                .filter(|pair| pair.1 == 1.0)
                .count() as f64
                * average_rank;
            start = end;
        }
        metrics.roc_auc = Some(
            (positive_rank_sum - (positives * (positives + 1) / 2) as f64)
                / (positives * negatives) as f64,
        );
    }
    metrics.calibration = Some(
        (0..10)
            .map(|bucket_index| {
                let values = pairs
                    .iter()
                    .filter(|(prediction, _)| {
                        ((*prediction * 10.0).floor() as usize).min(9) == bucket_index
                    })
                    .collect::<Vec<_>>();
                let bucket_count = values.len();
                CalibrationBucket {
                    bucket_index,
                    lower_bound: bucket_index as f64 / 10.0,
                    upper_bound: (bucket_index + 1) as f64 / 10.0,
                    count: bucket_count,
                    mean_prediction: (bucket_count > 0).then(|| {
                        values.iter().map(|pair| pair.0).sum::<f64>() / bucket_count as f64
                    }),
                    observed_frequency: (bucket_count > 0).then(|| {
                        values.iter().map(|pair| pair.1).sum::<f64>() / bucket_count as f64
                    }),
                }
            })
            .collect(),
    );
    Ok(metrics)
}

fn classify_evidence_state(
    evaluation_start_time_ms: i64,
    evaluation_end_time_ms: i64,
    segment_windows: &[Vec<Option<(i64, i64)>>],
) -> EvaluationEvidenceState {
    let segment_states = segment_windows
        .iter()
        .map(|windows| {
            if windows.len() != 3 || windows.iter().any(Option::is_none) {
                "unknown"
            } else if windows.iter().flatten().any(|(start, end)| {
                *start <= evaluation_end_time_ms && *end >= evaluation_start_time_ms
            }) {
                "overlapping"
            } else {
                "out-of-sample"
            }
            .to_owned()
        })
        .collect::<Vec<_>>();
    EvaluationEvidenceState {
        summary: conservative_evidence_state(&segment_states).into(),
        segment_states,
    }
}

fn conservative_evidence_state(states: &[String]) -> &'static str {
    if states.is_empty() {
        "unknown"
    } else if states.iter().any(|state| state == "overlapping") {
        "overlapping"
    } else if states.iter().any(|state| state == "unknown") {
        "unknown"
    } else {
        "out-of-sample"
    }
}

fn parse_provenance_window(value: Option<&serde_json::Value>) -> Option<(i64, i64)> {
    let value = value?.as_str()?.trim();
    if value.eq_ignore_ascii_case("unknown") || value.is_empty() {
        return None;
    }
    if let Ok(object) = serde_json::from_str::<serde_json::Value>(value) {
        let start = object.get("startTimeMs")?.as_i64()?;
        let end = object.get("endTimeMs")?.as_i64()?;
        return (start <= end).then_some((start, end));
    }
    let (start, end) = value.split_once("..")?;
    let range = (start.parse().ok()?, end.parse().ok()?);
    (range.0 <= range.1).then_some(range)
}

fn forecast_evaluation_identity(content: &serde_json::Value) -> Result<String, String> {
    serde_json::to_vec(content)
        .map(|canonical| hash(&canonical))
        .map_err(string)
}

pub(crate) fn segment_evidence(
    segments: &[serde_json::Value],
    evaluation_start_time_ms: i64,
    evaluation_end_time_ms: i64,
) -> EvaluationEvidenceState {
    let segment_states = segments
        .iter()
        .filter_map(|segment| {
            let start = segment
                .get("startPredictionTimeMs")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(evaluation_start_time_ms);
            let end = segment
                .get("endPredictionTimeMs")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(evaluation_end_time_ms);
            if start > evaluation_end_time_ms || end < evaluation_start_time_ms {
                return None;
            }
            let provenance = segment
                .get("provenance")
                .or_else(|| segment.get("modelArtifact")?.get("provenance"));
            let windows = ["trainingWindow", "fittingWindow", "normalizationWindow"]
                .iter()
                .map(|field| parse_provenance_window(provenance?.get(field)))
                .collect::<Vec<_>>();
            Some(
                classify_evidence_state(
                    start.max(evaluation_start_time_ms),
                    end.min(evaluation_end_time_ms),
                    &[windows],
                )
                .summary,
            )
        })
        .collect::<Vec<_>>();
    EvaluationEvidenceState {
        summary: conservative_evidence_state(&segment_states).into(),
        segment_states,
    }
}

#[derive(Clone, Copy)]
enum ForecastEvaluator {
    ExpectedValue,
    Probability,
    Score,
    Custom,
}

fn custom_metrics(
    predictions: &[f64],
    evaluation_row_count: usize,
    unavailable_prediction_count: usize,
) -> Result<ForecastMetrics, String> {
    let mut metrics = probability_metrics(&[], evaluation_row_count, unavailable_prediction_count)?;
    metrics.prediction_distribution = (!predictions.is_empty())
        .then(|| distribution(predictions))
        .transpose()?;
    Ok(metrics)
}

fn segment_provenance(segment: &serde_json::Value) -> Option<&serde_json::Value> {
    segment
        .get("provenance")
        .or_else(|| segment.get("modelArtifact")?.get("provenance"))
}

pub(crate) fn score_scale_provenance(
    signal: &adaq_component_tooling::ModelOutput,
    segments: &[serde_json::Value],
) -> Result<Vec<serde_json::Value>, String> {
    if !matches!(
        signal.prediction_kind,
        adaq_component_tooling::PredictionKind::Score
    ) || matches!(
        signal.value_scale,
        adaq_component_tooling::ForecastValueScale::Custom { .. }
    ) {
        return Ok(Vec::new());
    }
    let expected_transform = match &signal.value_scale {
        adaq_component_tooling::ForecastValueScale::ZScore { method } => Some(method.as_str()),
        adaq_component_tooling::ForecastValueScale::Percentile => None,
        _ => return Err("forecast-evaluation-score-scale-is-invalid".into()),
    };
    segments
        .iter()
        .map(|segment| {
            let raw = segment_provenance(segment)
                .and_then(|value| value.get("scaleProvenance"))
                .ok_or("forecast-evaluation-score-scale-provenance-is-unproven")?;
            let value = match raw.as_str() {
                Some(raw) => serde_json::from_str(raw).map_err(|_| {
                    "forecast-evaluation-score-scale-provenance-is-unproven".to_owned()
                })?,
                None => raw.clone(),
            };
            let value = value.get(&signal.name).cloned().unwrap_or(value);
            let provenance: ScaleProvenance = serde_json::from_value(value)
                .map_err(|_| "forecast-evaluation-score-scale-provenance-is-unproven".to_owned())?;
            let (transform_id, parameters, causal) = match &provenance {
                ScaleProvenance::TrainingFrozen {
                    transform_id,
                    reference_distribution_id,
                    parameters,
                } => (
                    transform_id,
                    parameters,
                    !reference_distribution_id.trim().is_empty()
                        && reference_distribution_id != "unknown",
                ),
                ScaleProvenance::PastOnlyRolling {
                    transform_id,
                    parameters,
                } => (
                    transform_id,
                    parameters,
                    parameters
                        .get("windowBars")
                        .and_then(serde_json::Value::as_u64)
                        .is_some_and(|window| window > 0),
                ),
            };
            if transform_id.trim().is_empty()
                || transform_id == "unknown"
                || parameters.is_empty()
                || !expected_transform.is_none_or(|expected| transform_id == expected)
                || !causal
            {
                return Err("forecast-evaluation-score-scale-provenance-is-unproven".into());
            }
            serde_json::to_value(provenance).map_err(string)
        })
        .collect()
}

pub(crate) fn validate_prediction_scale(
    signal: &adaq_component_tooling::ModelOutput,
    prediction: f64,
) -> Result<(), String> {
    if !prediction.is_finite() {
        return Err("forecast-evaluation-prediction-is-non-finite".into());
    }
    let valid = match signal.value_scale {
        adaq_component_tooling::ForecastValueScale::Probability
        | adaq_component_tooling::ForecastValueScale::Percentile => {
            (0.0..=1.0).contains(&prediction)
        }
        adaq_component_tooling::ForecastValueScale::Custom {
            minimum, maximum, ..
        } => {
            minimum.is_none_or(|minimum| prediction >= minimum)
                && maximum.is_none_or(|maximum| prediction <= maximum)
        }
        _ => true,
    };
    valid.then_some(()).ok_or_else(|| {
        match signal.value_scale {
            adaq_component_tooling::ForecastValueScale::Probability => {
                "forecast-evaluation-probability-is-out-of-bounds"
            }
            adaq_component_tooling::ForecastValueScale::Percentile => {
                "forecast-evaluation-percentile-is-out-of-bounds"
            }
            _ => "forecast-evaluation-custom-scale-is-out-of-bounds",
        }
        .into()
    })
}

fn evaluate_forecast(
    state: &LocalResearchState,
    request: &ForecastEvaluationRequest,
) -> Result<ForecastEvaluationReport, String> {
    validate_user(&request.user_id)?;
    if request.evaluation_start_time_ms > request.evaluation_end_time_ms
        || request.stability_window_bars == 0
    {
        return Err("forecast-evaluation-window-is-invalid".into());
    }
    let (metadata_json, parquet_path): (String, String) = state
        .database
        .lock()
        .map_err(string)?
        .query_row(
            "SELECT c.metadata_json, c.parquet_path FROM signal_dataset_content c JOIN signal_dataset_access a USING(dataset_id) WHERE c.dataset_id = ?1 AND a.user_id = ?2",
            params![request.dataset_id, request.user_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| "forecast-evaluation-dataset-is-not-available-to-user".to_owned())?;
    let dataset: SignalDataset = serde_json::from_str(&metadata_json).map_err(string)?;
    if request.snapshot_id != dataset.snapshot_id {
        return Err("forecast-evaluation-snapshot-mismatch".into());
    }
    let outputs = dataset_outputs(&dataset)?;
    let (signal_index, signal) = outputs
        .iter()
        .enumerate()
        .find(|(_, output)| output.name == request.signal_name)
        .ok_or("forecast-evaluation-signal-was-not-found")?;
    let evaluator = match (
        &signal.prediction_kind,
        &signal.forecast_target,
        &signal.value_scale,
    ) {
        (
            adaq_component_tooling::PredictionKind::ExpectedValue,
            adaq_component_tooling::ForecastTarget::Builtin {
                target: adaq_component_tooling::BuiltinForecastTarget::FutureCloseReturn,
            },
            adaq_component_tooling::ForecastValueScale::Native,
        ) => ForecastEvaluator::ExpectedValue,
        (
            adaq_component_tooling::PredictionKind::Probability,
            adaq_component_tooling::ForecastTarget::Builtin {
                target: adaq_component_tooling::BuiltinForecastTarget::FutureCloseUp,
            },
            adaq_component_tooling::ForecastValueScale::Probability,
        ) => ForecastEvaluator::Probability,
        (
            adaq_component_tooling::PredictionKind::Score,
            adaq_component_tooling::ForecastTarget::Builtin { .. },
            adaq_component_tooling::ForecastValueScale::Percentile
            | adaq_component_tooling::ForecastValueScale::ZScore { .. }
            | adaq_component_tooling::ForecastValueScale::Custom { .. },
        ) => ForecastEvaluator::Score,
        (adaq_component_tooling::PredictionKind::Custom { .. }, _, _)
        | (_, adaq_component_tooling::ForecastTarget::Custom { .. }, _) => {
            ForecastEvaluator::Custom
        }
        _ => {
            return Err("forecast-evaluation-signal-contract-is-incompatible".into());
        }
    };
    if signal.horizon_bars == 0 {
        return Err("forecast-evaluation-signal-contract-is-incompatible".into());
    }
    if request.horizon_bars != signal.horizon_bars {
        return Err("forecast-evaluation-horizon-mismatch".into());
    }
    let (snapshot, bars) = state.snapshot_for_user(&request.user_id, &dataset.snapshot_id)?;
    if snapshot.snapshot_id != dataset.snapshot_id
        || snapshot.src != dataset.src
        || snapshot.code != dataset.code
        || snapshot.interval.as_str() != dataset.interval
    {
        return Err("forecast-evaluation-dataset-snapshot-mismatch".into());
    }
    let parquet = fs::read(parquet_path).map_err(string)?;
    if hash(&parquet) != dataset.parquet_sha256 {
        return Err("forecast-evaluation-dataset-content-hash-mismatch".into());
    }
    let rows = read_external_rows(&parquet)?;
    if rows.len() != bars.len() {
        return Err("forecast-evaluation-dataset-row-count-mismatch".into());
    }
    for row in &rows {
        let Some(values) = &row.values else { continue };
        if values.len() != outputs.len() {
            return Err("forecast-evaluation-dataset-signal-contract-mismatch".into());
        }
        let prediction = values[signal_index];
        validate_prediction_scale(signal, prediction)?;
    }
    let gaps = snapshot
        .gaps
        .iter()
        .map(|gap| adaq_data_core::BarGap {
            start_time_ms: gap.start_time_ms,
            end_time_ms: gap.end_time_ms,
        })
        .collect::<Vec<_>>();
    let labels = match evaluator {
        ForecastEvaluator::ExpectedValue => {
            realize_future_close_returns(&bars, &gaps, signal.horizon_bars)?
        }
        ForecastEvaluator::Probability => {
            realize_future_close_up(&bars, &gaps, signal.horizon_bars)?
        }
        ForecastEvaluator::Score => match signal.forecast_target {
            adaq_component_tooling::ForecastTarget::Builtin {
                target: adaq_component_tooling::BuiltinForecastTarget::FutureCloseReturn,
            } => realize_future_close_returns(&bars, &gaps, signal.horizon_bars)?,
            adaq_component_tooling::ForecastTarget::Builtin {
                target: adaq_component_tooling::BuiltinForecastTarget::FutureCloseUp,
            } => realize_future_close_up(&bars, &gaps, signal.horizon_bars)?,
            _ => unreachable!("custom targets use the common evaluator"),
        },
        ForecastEvaluator::Custom => vec![None; bars.len()],
    };
    let selected = rows
        .iter()
        .enumerate()
        .filter(|(_, row)| {
            row.prediction_time_ms >= request.evaluation_start_time_ms
                && row.prediction_time_ms <= request.evaluation_end_time_ms
        })
        .collect::<Vec<_>>();
    if selected.is_empty() {
        return Err("forecast-evaluation-window-has-no-dataset-rows".into());
    }
    let mut aligned = Vec::new();
    let mut available_predictions = Vec::new();
    let mut unavailable_rows = Vec::new();
    let mut unavailable_prediction_count = 0usize;
    for (index, row) in selected.iter().copied() {
        match (
            row.values
                .as_ref()
                .and_then(|values| values.get(signal_index)),
            labels[index],
        ) {
            (Some(prediction), Some(realized)) if prediction.is_finite() => {
                aligned.push((*prediction, realized));
                available_predictions.push(*prediction);
            }
            (Some(prediction), None) if prediction.is_finite() => {
                available_predictions.push(*prediction);
                unavailable_rows.push(serde_json::json!({
                    "predictionTimeMs": row.prediction_time_ms,
                    "reason": match evaluator {
                        ForecastEvaluator::ExpectedValue => "future-close-return-unavailable",
                        ForecastEvaluator::Probability => "future-close-up-unavailable",
                        ForecastEvaluator::Score => "score-target-unavailable",
                        ForecastEvaluator::Custom => "target-specific-evaluator-unavailable",
                    }
                }));
            }
            (None, _) => {
                unavailable_prediction_count += 1;
                unavailable_rows.push(serde_json::json!({
                    "predictionTimeMs": row.prediction_time_ms,
                    "reason": row.unavailable_reason.as_deref().unwrap_or("prediction-unavailable")
                }));
            }
            _ => return Err("forecast-evaluation-prediction-is-non-finite".into()),
        }
    }
    let stability_windows = selected
        .chunks(request.stability_window_bars)
        .map(|window| {
            let pairs = window
                .iter()
                .filter_map(|(index, row)| {
                    row.values
                        .as_ref()
                        .and_then(|values| values.get(signal_index))
                        .zip(labels[*index])
                        .map(|(prediction, realized)| (*prediction, realized))
                })
                .collect::<Vec<_>>();
            let unavailable_predictions = window
                .iter()
                .filter(|(_, row)| row.values.is_none())
                .count();
            let predictions = window
                .iter()
                .filter_map(|(_, row)| {
                    row.values
                        .as_ref()
                        .and_then(|values| values.get(signal_index))
                        .copied()
                })
                .collect::<Vec<_>>();
            let metrics = match evaluator {
                ForecastEvaluator::ExpectedValue => {
                    expected_value_metrics(&pairs, window.len(), unavailable_predictions)?
                }
                ForecastEvaluator::Probability => {
                    probability_metrics(&pairs, window.len(), unavailable_predictions)?
                }
                ForecastEvaluator::Score => {
                    score_metrics(&pairs, window.len(), unavailable_predictions, &[])?
                }
                ForecastEvaluator::Custom => {
                    custom_metrics(&predictions, window.len(), unavailable_predictions)?
                }
            };
            Ok(serde_json::json!({
                "startPredictionTimeMs": window.first().expect("window is non-empty").1.prediction_time_ms,
                "endPredictionTimeMs": window.last().expect("window is non-empty").1.prediction_time_ms,
                "metrics": metrics,
            }))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let producer_segments = producer_segment_values(&dataset)?;
    let scale_provenance = score_scale_provenance(signal, &producer_segments)?;
    let metrics = match evaluator {
        ForecastEvaluator::ExpectedValue => {
            expected_value_metrics(&aligned, selected.len(), unavailable_prediction_count)?
        }
        ForecastEvaluator::Probability => {
            probability_metrics(&aligned, selected.len(), unavailable_prediction_count)?
        }
        ForecastEvaluator::Score => {
            let window_ics = stability_windows
                .iter()
                .map(|window| window["metrics"]["pearsonIc"].as_f64())
                .collect::<Vec<_>>();
            score_metrics(
                &aligned,
                selected.len(),
                unavailable_prediction_count,
                &window_ics,
            )?
        }
        ForecastEvaluator::Custom => custom_metrics(
            &available_predictions,
            selected.len(),
            unavailable_prediction_count,
        )?,
    };
    let evidence_state = segment_evidence(
        &producer_segments,
        request.evaluation_start_time_ms,
        request.evaluation_end_time_ms,
    );
    let mut metric_versions: BTreeMap<String, String> = BTreeMap::from([
        ("coverage".into(), "coverage@1".into()),
        ("distribution".into(), "distribution@1".into()),
        (
            "timeWindowStability".into(),
            "non-overlapping-windows@1".into(),
        ),
    ]);
    match evaluator {
        ForecastEvaluator::ExpectedValue => {
            metric_versions.insert("expectedValue".into(), "expected-value@1".into());
        }
        ForecastEvaluator::Probability => {
            metric_versions.insert("probability".into(), "binary-probability@1".into());
            metric_versions.insert("calibration".into(), "equal-width-10-buckets@1".into());
        }
        ForecastEvaluator::Score => {
            metric_versions.insert("score".into(), "single-instrument-score@1".into());
            metric_versions.insert("quantiles".into(), "tie-preserving-five-quantiles@1".into());
            metric_versions.insert("windowIcir".into(), "non-overlapping-window-icir@1".into());
        }
        ForecastEvaluator::Custom => {}
    }
    let content = serde_json::json!({
        "datasetId": dataset.dataset_id,
        "snapshotId": dataset.snapshot_id,
        "signalName": request.signal_name,
        "signalContract": signal,
        "evaluationStartTimeMs": request.evaluation_start_time_ms,
        "evaluationEndTimeMs": request.evaluation_end_time_ms,
        "stabilityWindowBars": request.stability_window_bars,
        "metrics": metrics,
        "stabilityWindows": stability_windows,
        "evidenceState": evidence_state,
        "unavailableRows": unavailable_rows,
        "producerSegments": producer_segments,
        "scaleProvenance": scale_provenance,
        "trustState": dataset.trust,
        "metricVersions": metric_versions,
        "engineIdentity": dataset.engine_identity,
        "schemaIdentity": "forecast-evaluation-report@2",
        "datasetParquetSha256": dataset.parquet_sha256,
        "componentLock": dataset.component_lock,
        "featurePlanHash": dataset.feature_plan_hash,
    });
    let report_id = forecast_evaluation_identity(&content)?;
    let mut report = content;
    report
        .as_object_mut()
        .expect("evaluation content is an object")
        .insert("reportId".into(), report_id.into());
    serde_json::from_value(report).map_err(string)
}

fn save_forecast_evaluation(
    state: &LocalResearchState,
    request: &ForecastEvaluationRequest,
) -> Result<ForecastEvaluationReport, String> {
    let report = evaluate_forecast(state, request)?;
    let report_json = serde_json::to_string(&report).map_err(string)?;
    let mut database = state.database.lock().map_err(string)?;
    let transaction = database.transaction().map_err(string)?;
    transaction
        .execute(
            "INSERT OR IGNORE INTO forecast_evaluation_content(report_id, report_json) VALUES (?1, ?2)",
            params![report.report_id, report_json],
        )
        .map_err(string)?;
    transaction
        .execute(
            "INSERT OR IGNORE INTO forecast_evaluation_access(user_id, report_id) VALUES (?1, ?2)",
            params![request.user_id, report.report_id],
        )
        .map_err(string)?;
    transaction.commit().map_err(string)?;
    Ok(report)
}

#[tauri::command]
pub async fn forecast_evaluation_create(
    request: ForecastEvaluationRequest,
    state: tauri::State<'_, Arc<LocalResearchState>>,
) -> Result<ForecastEvaluationReport, String> {
    save_forecast_evaluation(&state, &request)
}

#[tauri::command]
pub async fn forecast_evaluation_list(
    user_id: String,
    state: tauri::State<'_, Arc<LocalResearchState>>,
) -> Result<Vec<ForecastEvaluationReport>, String> {
    list_forecast_evaluations(&state, &user_id)
}

fn list_forecast_evaluations(
    state: &LocalResearchState,
    user_id: &str,
) -> Result<Vec<ForecastEvaluationReport>, String> {
    validate_user(&user_id)?;
    state
        .database
        .lock()
        .map_err(string)?
        .prepare("SELECT c.report_json FROM forecast_evaluation_content c JOIN forecast_evaluation_access a USING(report_id) WHERE a.user_id = ?1 ORDER BY c.report_id")
        .map_err(string)?
        .query_map([user_id], |row| row.get::<_, String>(0))
        .map_err(string)?
        .map(|row| serde_json::from_str(&row.map_err(string)?).map_err(string))
        .collect()
}

#[tauri::command]
pub fn forecast_evaluation_export(
    report_id: String,
    user_id: String,
    format: String,
    state: tauri::State<'_, Arc<LocalResearchState>>,
) -> Result<String, String> {
    export_forecast_evaluation(&state, &user_id, &report_id, &format)
}

fn export_forecast_evaluation(
    state: &LocalResearchState,
    user_id: &str,
    report_id: &str,
    format: &str,
) -> Result<String, String> {
    validate_user(&user_id)?;
    let report_json: String = state
        .database
        .lock()
        .map_err(string)?
        .query_row(
            "SELECT c.report_json FROM forecast_evaluation_content c JOIN forecast_evaluation_access a USING(report_id) WHERE c.report_id = ?1 AND a.user_id = ?2",
            params![report_id, user_id],
            |row| row.get(0),
        )
        .map_err(|_| "Forecast Evaluation Report was not found".to_owned())?;
    let report: ForecastEvaluationReport = serde_json::from_str(&report_json).map_err(string)?;
    match format {
        "json" => serde_json::to_string_pretty(&report).map_err(string),
        "markdown" => Ok(forecast_evaluation_markdown(&report)),
        _ => Err("Forecast Evaluation export format is invalid".into()),
    }
}

fn forecast_evaluation_markdown(report: &ForecastEvaluationReport) -> String {
    let custom_evidence = matches!(
        report.signal_contract.prediction_kind,
        adaq_component_tooling::PredictionKind::Custom { .. }
    ) || matches!(
        report.signal_contract.forecast_target,
        adaq_component_tooling::ForecastTarget::Custom { .. }
    );
    let specialized_metrics = if custom_evidence {
        "## Custom evidence\n\nCommon coverage, distribution, stability, and provenance are retained. No specialized evaluator is claimed.\n".into()
    } else {
        match report.signal_contract.prediction_kind {
        adaq_component_tooling::PredictionKind::ExpectedValue => format!(
            "## Expected Value metrics\n\n- MAE: {}\n- RMSE: {}\n- Mean bias: {}\n- Pearson correlation: {}\n",
            report
                .metrics
                .mae
                .map_or_else(|| "unavailable".into(), |value| value.to_string()),
            report
                .metrics
                .rmse
                .map_or_else(|| "unavailable".into(), |value| value.to_string()),
            report
                .metrics
                .mean_bias
                .map_or_else(|| "unavailable".into(), |value| value.to_string()),
            report
                .metrics
                .pearson_correlation
                .map_or_else(|| "unavailable".into(), |value| value.to_string()),
        ),
        adaq_component_tooling::PredictionKind::Probability => format!(
            "## Probability metrics\n\n- Brier Score: {}\n- Log Loss: {}\n- ROC AUC: {}\n- Calibration: {}\n- Undefined metrics: {}\n",
            report
                .metrics
                .brier_score
                .map_or_else(|| "unavailable".into(), |value| value.to_string()),
            report
                .metrics
                .log_loss
                .map_or_else(|| "unavailable".into(), |value| value.to_string()),
            report
                .metrics
                .roc_auc
                .map_or_else(|| "unavailable".into(), |value| value.to_string()),
            serde_json::to_string(&report.metrics.calibration).expect("calibration serializes"),
            serde_json::to_string(&report.metrics.undefined_metrics)
                .expect("diagnostics serialize"),
        ),
        adaq_component_tooling::PredictionKind::Score => format!(
            "## Single-Instrument time-series Score metrics\n\n- Pearson IC: {}\n- Spearman Rank IC: {}\n- Window ICIR: {}\n- Five-quantile realized Target evidence: {}\n\nThese are not cross-sectional IC, Strategy profitability, turnover, or a universal investment-quality score.\n",
            report
                .metrics
                .pearson_ic
                .map_or_else(|| "unavailable".into(), |value| value.to_string()),
            report
                .metrics
                .spearman_rank_ic
                .map_or_else(|| "unavailable".into(), |value| value.to_string()),
            report
                .metrics
                .window_icir
                .map_or_else(|| "unavailable".into(), |value| value.to_string()),
            report.metrics.undefined_metrics.get("quantiles").cloned().unwrap_or_else(||
                serde_json::to_string(&report.metrics.quantiles).expect("quantiles serialize")
            ),
        ),
        adaq_component_tooling::PredictionKind::Custom { .. } =>
            "## Custom evidence\n\nCommon coverage, distribution, stability, and provenance are retained. No specialized evaluator is claimed.\n".into(),
        }
    };
    format!(
        "# Forecast Evaluation Report\n\n- Report ID: `{}`\n- Dataset ID: `{}`\n- Snapshot ID: `{}`\n- Signal: `{}`\n- Evaluation window: `{}` to `{}`\n- Evidence state: `{}`\n- Trust state: `{}`\n- Schema: `{}`\n\n[Metric definitions](https://github.com/tonywxx/adaq/blob/main/docs/reference/research-metrics.md)\n\n## Common metrics\n\n- Coverage: {}\n- Missingness: {}\n\n{}\n## Authoritative evidence\n\n```json\n{}\n```\n",
        report.report_id,
        report.dataset_id,
        report.snapshot_id,
        report.signal_name,
        report.evaluation_start_time_ms,
        report.evaluation_end_time_ms,
        report.evidence_state.summary,
        report.trust_state,
        report.schema_identity,
        report.metrics.coverage,
        report.metrics.missingness,
        specialized_metrics,
        serde_json::to_string_pretty(report).expect("report serializes"),
    )
}

