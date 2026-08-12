use std::collections::{BTreeMap, BTreeSet, HashMap};

use adaq_data_core::{BarGap, OhlcvBar};
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};

use crate::{
    ContractError, EconomicAssumptions, EvaluationWindow, FactorDataset, FactorEvaluationProtocol,
    FactorEvaluationReport, FactorLens, FactorMarketContext, FactorMetricCatalog,
    FactorObservationValue, FactorRegimeDefinition, FactorScope, MetricId, MetricObservation,
    MetricRecord, MetricUndefinedReason, RegimeEvidence, TargetUnavailableEvidence,
    TargetUnavailableReason,
};

const MAX_EVALUATION_ROWS: usize = 1_000_000;
const MIN_CORRELATION_SAMPLES: usize = 3;
const BASIS_EPSILON: f64 = 1e-12;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CorporateActionEvidence {
    Verified,
    Unavailable { reason: String },
}

impl CorporateActionEvidence {
    fn validate(&self) -> Result<(), EvaluationError> {
        if matches!(self, Self::Unavailable { reason } if reason.trim().is_empty()) {
            return Err(EvaluationError::Invalid(
                "Corporate Action evidence failure requires a reason".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactorMarketSeries {
    pub instrument_id: String,
    pub snapshot_id: String,
    pub market_context: FactorMarketContext,
    pub bars: Vec<OhlcvBar>,
    pub gaps: Vec<BarGap>,
    pub corporate_action_evidence: CorporateActionEvidence,
}

impl FactorMarketSeries {
    fn validate(&self, protocol: &FactorEvaluationProtocol) -> Result<(), EvaluationError> {
        if self.instrument_id.trim().is_empty()
            || self.snapshot_id != protocol.market_data_snapshot_id
            || self.market_context != protocol.market_context
            || self.bars.is_empty()
        {
            return Err(EvaluationError::Invalid(
                "market series is not bound to the Evaluation Protocol".into(),
            ));
        }
        self.corporate_action_evidence.validate()?;
        if self
            .bars
            .windows(2)
            .any(|pair| pair[0].open_time_ms >= pair[1].open_time_ms)
            || self.gaps.iter().any(|gap| {
                gap.start_time_ms >= gap.end_time_ms
                    || gap.start_time_ms < 0
                    || gap.end_time_ms <= 0
            })
            || self
                .gaps
                .windows(2)
                .any(|pair| pair[0].end_time_ms > pair[1].start_time_ms)
        {
            return Err(EvaluationError::Invalid(
                "market series bars and gaps must be ordered and non-overlapping".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum EvaluationFeatureCell {
    Available {
        value: f64,
        available_at_ms: i64,
    },
    Unavailable {
        reason: crate::FactorUnavailabilityReason,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvaluationFeatureRow {
    pub instrument_id: String,
    pub observation_time_ms: i64,
    pub values: BTreeMap<String, EvaluationFeatureCell>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvaluationFeatureEvidence {
    pub feature_dataset_id: String,
    pub feature_plan_hash: String,
    pub rows: Vec<EvaluationFeatureRow>,
}

impl EvaluationFeatureEvidence {
    fn validate(&self, protocol: &FactorEvaluationProtocol) -> Result<(), EvaluationError> {
        if self.feature_dataset_id != protocol.feature_dataset_id
            || self.feature_plan_hash != protocol.feature_plan_hash
            || self.rows.len() > MAX_EVALUATION_ROWS
            || self.rows.windows(2).any(|pair| {
                (pair[0].instrument_id.as_str(), pair[0].observation_time_ms)
                    >= (pair[1].instrument_id.as_str(), pair[1].observation_time_ms)
            })
        {
            return Err(EvaluationError::Invalid(
                "Evaluation Feature evidence is not bound to the exact Feature Dataset/Plan".into(),
            ));
        }
        for row in &self.rows {
            if row.instrument_id.trim().is_empty()
                || row.values.iter().any(|(name, cell)| {
                    !crate::is_lower_kebab(name)
                        || matches!(
                            cell,
                            EvaluationFeatureCell::Available {
                                value,
                                available_at_ms
                            } if !value.is_finite() || *available_at_ms > row.observation_time_ms
                        )
                })
            {
                return Err(EvaluationError::Invalid(
                    "Evaluation Feature evidence contains an invalid or non-causal cell".into(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvaluationError {
    Invalid(String),
    Contract(ContractError),
}

impl std::fmt::Display for EvaluationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(message) => formatter.write_str(message),
            Self::Contract(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for EvaluationError {}

impl From<ContractError> for EvaluationError {
    fn from(error: ContractError) -> Self {
        Self::Contract(error)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FactorEvaluationInput<'a> {
    pub dataset: &'a FactorDataset,
    pub protocol: &'a FactorEvaluationProtocol,
    pub market_series: &'a [FactorMarketSeries],
    pub feature_evidence: Option<&'a EvaluationFeatureEvidence>,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct FactorEvaluator;

impl FactorEvaluator {
    pub fn evaluate(
        input: FactorEvaluationInput<'_>,
    ) -> Result<FactorEvaluationReport, EvaluationError> {
        validate_input(&input)?;
        let series = input
            .market_series
            .iter()
            .map(|series| (series.instrument_id.clone(), series))
            .collect::<BTreeMap<_, _>>();
        let feature_lookup = input.feature_evidence.map(feature_lookup);
        let mut metrics = Vec::new();
        let mut target_unavailable = Vec::new();
        let mut regime_evidence = Vec::new();
        let mut observations_by_horizon = BTreeMap::new();

        for horizon in &input.protocol.horizon_bars {
            let observations = build_observations(
                input.dataset,
                input.protocol,
                &series,
                *horizon,
                &mut target_unavailable,
            )?;
            observations_by_horizon.insert(*horizon, observations);
        }

        for window in &input.protocol.windows {
            for horizon in &input.protocol.horizon_bars {
                let observations = observations_by_horizon
                    .get(horizon)
                    .expect("observations were built for every horizon");
                let selection = observations
                    .iter()
                    .filter(|observation| in_range(observation.time_ms, &window.selection))
                    .filter(|observation| {
                        !purged_or_embargoed(
                            observation,
                            window,
                            *horizon,
                            &series[&observation.instrument_id],
                            input.protocol.purge_bars,
                            input.protocol.embargo_bars,
                        )
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                let evaluation = observations
                    .iter()
                    .filter(|observation| in_range(observation.time_ms, &window.evaluation))
                    .cloned()
                    .collect::<Vec<_>>();

                match input.protocol.scope {
                    FactorScope::TimeSeries => evaluate_time_series(
                        input.protocol,
                        window,
                        *horizon,
                        &evaluation,
                        &mut metrics,
                    )?,
                    FactorScope::CrossSectional => evaluate_cross_sectional(
                        input.protocol,
                        window,
                        *horizon,
                        &evaluation,
                        feature_lookup.as_ref(),
                        &mut metrics,
                    )?,
                }
                if input.protocol.lenses.contains(&FactorLens::Regime) {
                    evaluate_regime(
                        input.protocol,
                        window,
                        *horizon,
                        &selection,
                        &evaluation,
                        feature_lookup.as_ref(),
                        &mut metrics,
                        &mut regime_evidence,
                    )?;
                }
            }
        }

        let base_metrics = metrics.clone();
        add_decay_and_stability(input.protocol, &base_metrics, &mut metrics);
        let report = FactorEvaluationReport {
            schema_version: crate::FACTOR_RESEARCH_SCHEMA_VERSION.into(),
            report_id: uuid::Uuid::new_v4(),
            protocol_hash: input.protocol.protocol_hash.clone(),
            factor_dataset_id: input.dataset.manifest.dataset_id.clone(),
            output_name: input.protocol.output_name.clone(),
            scope: input.protocol.scope,
            target: input.protocol.target,
            market_data_snapshot_id: input.protocol.market_data_snapshot_id.clone(),
            point_in_time_universe_id: input.protocol.point_in_time_universe_id.clone(),
            market_context: input.protocol.market_context.clone(),
            evidence_state: input.protocol.evidence_state(),
            metrics,
            target_unavailable,
            regime_evidence,
            input_identities: vec![
                input.dataset.manifest.dataset_id.clone(),
                input.dataset.manifest.payload_sha256.clone(),
                input.protocol.protocol_hash.clone(),
                input.protocol.engine_identity.build_id.clone(),
            ],
            report_hash: String::new(),
        };
        FactorEvaluationReport::freeze(report).map_err(EvaluationError::from)
    }
}

#[derive(Debug, Clone)]
struct Observation {
    instrument_id: String,
    time_ms: i64,
    factor: Option<f64>,
    target: Option<f64>,
    target_time_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy)]
struct TargetValue {
    value: Option<f64>,
    target_time_ms: Option<i64>,
    reason: Option<TargetUnavailableReason>,
}

fn validate_input(input: &FactorEvaluationInput<'_>) -> Result<(), EvaluationError> {
    input.protocol.validate()?;
    input
        .dataset
        .validate()
        .map_err(|error| EvaluationError::Invalid(error.to_string()))?;
    let manifest = &input.dataset.manifest;
    if manifest.scope != input.protocol.scope
        || manifest.feature_dataset_id != input.protocol.feature_dataset_id
        || manifest.feature_plan_hash != input.protocol.feature_plan_hash
        || manifest.market_data_snapshot_id != input.protocol.market_data_snapshot_id
        || manifest.point_in_time_universe_id != input.protocol.point_in_time_universe_id
        || manifest.market_context != input.protocol.market_context
        || !manifest
            .output_names
            .iter()
            .any(|name| name == &input.protocol.output_name)
    {
        return Err(EvaluationError::Invalid(
            "Factor Evaluation is not bound to the exact Completed Factor Dataset evidence".into(),
        ));
    }
    if input.market_series.len() > input.protocol.point_in_time_universe.len()
        || input.market_series.iter().any(|series| {
            !input
                .protocol
                .point_in_time_universe
                .iter()
                .any(|instrument| instrument == &series.instrument_id)
        })
    {
        return Err(EvaluationError::Invalid(
            "market series are outside the frozen Point-in-Time Universe".into(),
        ));
    }
    let mut instruments = BTreeSet::new();
    for series in input.market_series {
        if !instruments.insert(series.instrument_id.clone()) {
            return Err(EvaluationError::Invalid(
                "Evaluation market series contain duplicate Instruments".into(),
            ));
        }
        series.validate(input.protocol)?;
    }
    if input
        .dataset
        .rows
        .iter()
        .any(|row| !instruments.contains(&row.instrument_id))
    {
        return Err(EvaluationError::Invalid(
            "every Factor Dataset Instrument requires exact market evidence".into(),
        ));
    }
    if input.protocol.scope == FactorScope::CrossSectional {
        validate_cross_sectional_dataset(input.dataset, &input.protocol.point_in_time_universe)?;
    }
    if input.protocol.nuisance_feature_names.is_empty() && input.protocol.regime.is_none() {
        return Ok(());
    }
    input
        .feature_evidence
        .ok_or_else(|| {
            EvaluationError::Invalid(
                "nuisance and regime evaluation requires exact Feature evidence".into(),
            )
        })?
        .validate(input.protocol)
}

fn validate_cross_sectional_dataset(
    dataset: &FactorDataset,
    universe: &[String],
) -> Result<(), EvaluationError> {
    let mut by_time = BTreeMap::<i64, BTreeSet<&str>>::new();
    for row in &dataset.rows {
        by_time
            .entry(row.observation_time_ms)
            .or_default()
            .insert(row.instrument_id.as_str());
    }
    if by_time.values().any(|members| {
        members.len() != universe.len()
            || universe
                .iter()
                .any(|instrument| !members.contains(instrument.as_str()))
    }) {
        return Err(EvaluationError::Invalid(
            "Cross-Sectional evaluation requires complete frozen Universe rows".into(),
        ));
    }
    Ok(())
}

fn build_observations(
    dataset: &FactorDataset,
    protocol: &FactorEvaluationProtocol,
    series: &BTreeMap<String, &FactorMarketSeries>,
    horizon: u32,
    unavailable: &mut Vec<TargetUnavailableEvidence>,
) -> Result<Vec<Observation>, EvaluationError> {
    let mut observations = Vec::with_capacity(dataset.rows.len());
    for row in &dataset.rows {
        let target = target_for(series[&row.instrument_id], row.observation_time_ms, horizon);
        if let Some(reason) = target.reason {
            unavailable.push(TargetUnavailableEvidence {
                instrument_id: row.instrument_id.clone(),
                observation_time_ms: row.observation_time_ms,
                horizon_bars: horizon,
                reason,
            });
        }
        let factor = match row.values.get(&protocol.output_name) {
            Some(FactorObservationValue::Available {
                value,
                available_at_ms,
            }) if *available_at_ms <= row.observation_time_ms && value.is_finite() => Some(*value),
            _ => None,
        };
        observations.push(Observation {
            instrument_id: row.instrument_id.clone(),
            time_ms: row.observation_time_ms,
            factor,
            target: target.value,
            target_time_ms: target.target_time_ms,
        });
    }
    Ok(observations)
}

fn target_for(series: &FactorMarketSeries, time_ms: i64, horizon: u32) -> TargetValue {
    let origin_index = match series
        .bars
        .binary_search_by_key(&time_ms, |bar| bar.open_time_ms)
    {
        Ok(index) => index,
        Err(_) => {
            return TargetValue {
                value: None,
                target_time_ms: None,
                reason: Some(TargetUnavailableReason::InsufficientCoverage),
            };
        }
    };
    if let CorporateActionEvidence::Unavailable { .. } = &series.corporate_action_evidence {
        return TargetValue {
            value: None,
            target_time_ms: None,
            reason: Some(TargetUnavailableReason::CorporateActionUnavailable),
        };
    }
    let target_index = match origin_index.checked_add(horizon as usize) {
        Some(index) => index,
        None => {
            return TargetValue {
                value: None,
                target_time_ms: None,
                reason: Some(TargetUnavailableReason::InsufficientCoverage),
            };
        }
    };
    let Some(target_bar) = series.bars.get(target_index) else {
        return TargetValue {
            value: None,
            target_time_ms: None,
            reason: Some(TargetUnavailableReason::InsufficientCoverage),
        };
    };
    let origin_bar = &series.bars[origin_index];
    if series.gaps.iter().any(|gap| {
        gap.start_time_ms < target_bar.open_time_ms && gap.end_time_ms > origin_bar.open_time_ms
    }) {
        return TargetValue {
            value: None,
            target_time_ms: Some(target_bar.open_time_ms),
            reason: Some(TargetUnavailableReason::BarGap),
        };
    }
    let Some(origin_close) = origin_bar.close.to_f64() else {
        return TargetValue {
            value: None,
            target_time_ms: Some(target_bar.open_time_ms),
            reason: Some(TargetUnavailableReason::MissingClose),
        };
    };
    let Some(target_close) = target_bar.close.to_f64() else {
        return TargetValue {
            value: None,
            target_time_ms: Some(target_bar.open_time_ms),
            reason: Some(TargetUnavailableReason::MissingClose),
        };
    };
    if !origin_close.is_finite()
        || !target_close.is_finite()
        || origin_close <= 0.0
        || target_close <= 0.0
    {
        return TargetValue {
            value: None,
            target_time_ms: Some(target_bar.open_time_ms),
            reason: Some(TargetUnavailableReason::MissingClose),
        };
    }
    TargetValue {
        value: Some(target_close / origin_close - 1.0),
        target_time_ms: Some(target_bar.open_time_ms),
        reason: None,
    }
}

fn in_range(time_ms: i64, range: &crate::ObservationRange) -> bool {
    time_ms >= range.start_time_ms && time_ms < range.end_time_ms
}

fn purged_or_embargoed(
    observation: &Observation,
    window: &EvaluationWindow,
    horizon: u32,
    series: &FactorMarketSeries,
    purge_bars: u32,
    embargo_bars: u32,
) -> bool {
    let embargo_start = bar_boundary_before(series, window.evaluation.start_time_ms, embargo_bars);
    if observation.time_ms >= embargo_start && observation.time_ms < window.evaluation.start_time_ms
    {
        return true;
    }
    if observation.time_ms >= window.evaluation.end_time_ms
        && observation.time_ms
            < bar_boundary_after(series, window.evaluation.end_time_ms, embargo_bars)
    {
        return true;
    }
    if let Some(target_time_ms) = observation.target_time_ms
        && target_time_ms >= window.evaluation.start_time_ms
    {
        return true;
    }
    let Some(index) = series
        .bars
        .binary_search_by_key(&observation.time_ms, |bar| bar.open_time_ms)
        .ok()
    else {
        return true;
    };
    let Some(evaluation_index) = series
        .bars
        .binary_search_by_key(&window.evaluation.start_time_ms, |bar| bar.open_time_ms)
        .ok()
        .or_else(|| {
            series
                .bars
                .iter()
                .position(|bar| bar.open_time_ms >= window.evaluation.start_time_ms)
        })
    else {
        return true;
    };
    let required_purge = purge_bars.max(horizon);
    index.saturating_add(required_purge as usize) >= evaluation_index
}

fn bar_boundary_after(series: &FactorMarketSeries, end_time_ms: i64, bars: u32) -> i64 {
    let Some(index) = series
        .bars
        .iter()
        .position(|bar| bar.open_time_ms >= end_time_ms)
    else {
        return i64::MAX;
    };
    series
        .bars
        .get(index.saturating_add(bars as usize))
        .map(|bar| bar.open_time_ms)
        .unwrap_or(i64::MAX)
}

fn bar_boundary_before(series: &FactorMarketSeries, start_time_ms: i64, bars: u32) -> i64 {
    let index = series
        .bars
        .iter()
        .position(|bar| bar.open_time_ms >= start_time_ms)
        .unwrap_or(series.bars.len());
    series
        .bars
        .get(index.saturating_sub(bars as usize))
        .map(|bar| bar.open_time_ms)
        .unwrap_or(i64::MIN)
}

fn evaluate_time_series(
    protocol: &FactorEvaluationProtocol,
    window: &EvaluationWindow,
    horizon: u32,
    observations: &[Observation],
    metrics: &mut Vec<MetricRecord>,
) -> Result<(), EvaluationError> {
    let instruments = observations
        .iter()
        .map(|observation| observation.instrument_id.clone())
        .collect::<BTreeSet<_>>();
    for instrument in instruments {
        let rows = observations
            .iter()
            .filter(|observation| observation.instrument_id == instrument)
            .collect::<Vec<_>>();
        let pairs = rows
            .iter()
            .filter_map(|observation| Some((observation.factor?, observation.target?)))
            .map(|(factor, target)| (orient(factor, protocol), target))
            .collect::<Vec<_>>();
        let variant = instrument.clone();
        push_metric(
            metrics,
            window,
            &variant,
            horizon,
            &protocol.output_name,
            FactorLens::Temporal,
            MetricId::Coverage,
            coverage(rows.len(), pairs.len()),
        )?;
        push_metric(
            metrics,
            window,
            &variant,
            horizon,
            &protocol.output_name,
            FactorLens::Temporal,
            MetricId::Missingness,
            coverage(rows.len(), rows.len().saturating_sub(pairs.len())),
        )?;
        push_metric(
            metrics,
            window,
            &variant,
            horizon,
            &protocol.output_name,
            FactorLens::Temporal,
            MetricId::SampleCount,
            MetricObservation::available(pairs.len() as f64, pairs.len() as u64)?,
        )?;
        if protocol.lenses.contains(&FactorLens::Temporal) {
            push_metric(
                metrics,
                window,
                &variant,
                horizon,
                &protocol.output_name,
                FactorLens::Temporal,
                MetricId::Ic,
                correlation(&pairs),
            )?;
            push_metric(
                metrics,
                window,
                &variant,
                horizon,
                &protocol.output_name,
                FactorLens::Temporal,
                MetricId::RankIc,
                rank_correlation(&pairs),
            )?;
        }
        if protocol.lenses.contains(&FactorLens::Economic) {
            let economic_pairs = rows
                .iter()
                .enumerate()
                .filter(|(index, _)| {
                    is_rebalance_point(*index, protocol.economic.rebalance_every_bars)
                })
                .filter_map(|(_, observation)| {
                    Some((orient(observation.factor?, protocol), observation.target?))
                })
                .collect::<Vec<_>>();
            let economic = economic_observations(&economic_pairs, &protocol.economic);
            push_metric(
                metrics,
                window,
                &format!("{variant}-top-only"),
                horizon,
                &protocol.output_name,
                FactorLens::Economic,
                MetricId::Economic,
                economic.top_only,
            )?;
            push_metric(
                metrics,
                window,
                &format!("{variant}-top-minus-bottom"),
                horizon,
                &protocol.output_name,
                FactorLens::Economic,
                MetricId::Economic,
                economic.top_minus_bottom,
            )?;
            push_metric(
                metrics,
                window,
                &format!("{variant}-turnover"),
                horizon,
                &protocol.output_name,
                FactorLens::Economic,
                MetricId::Turnover,
                economic.turnover,
            )?;
        }
    }
    Ok(())
}

fn evaluate_cross_sectional(
    protocol: &FactorEvaluationProtocol,
    window: &EvaluationWindow,
    horizon: u32,
    observations: &[Observation],
    feature_lookup: Option<&BTreeMap<(String, i64), BTreeMap<String, EvaluationFeatureCell>>>,
    metrics: &mut Vec<MetricRecord>,
) -> Result<(), EvaluationError> {
    let mut by_time = BTreeMap::<i64, Vec<&Observation>>::new();
    for observation in observations {
        by_time
            .entry(observation.time_ms)
            .or_default()
            .push(observation);
    }
    let total_rows = by_time.len() * protocol.point_in_time_universe.len();
    let mut pairs_by_time = Vec::new();
    let mut neutralized = Vec::new();
    let mut economic = Vec::new();
    let mut weights = Vec::new();
    for (time_index, (time_ms, rows)) in by_time.into_iter().enumerate() {
        let mut row_by_instrument = rows
            .into_iter()
            .map(|row| (row.instrument_id.as_str(), row))
            .collect::<HashMap<_, _>>();
        if row_by_instrument.len() != protocol.point_in_time_universe.len()
            || protocol
                .point_in_time_universe
                .iter()
                .any(|instrument| !row_by_instrument.contains_key(instrument.as_str()))
        {
            return Err(EvaluationError::Invalid(
                "Cross-Sectional evaluation lost frozen Universe membership".into(),
            ));
        }
        let ordered = protocol
            .point_in_time_universe
            .iter()
            .map(|instrument| row_by_instrument.remove(instrument.as_str()).unwrap())
            .collect::<Vec<_>>();
        let pairs = ordered
            .iter()
            .filter_map(|row| Some((orient(row.factor?, protocol), row.target?)))
            .collect::<Vec<_>>();
        if !pairs.is_empty() {
            pairs_by_time.push(pairs.clone());
        }
        if protocol.lenses.contains(&FactorLens::Neutralized) {
            neutralized.push(neutralized_observation(
                &ordered,
                feature_lookup,
                protocol,
                time_ms,
            ));
        }
        if protocol.lenses.contains(&FactorLens::Economic)
            && is_rebalance_point(time_index, protocol.economic.rebalance_every_bars)
        {
            let point = cross_sectional_economic(&ordered, protocol);
            economic.push((point.top_only, point.top_minus_bottom));
            if let Some(weight) = point.top_weights {
                weights.push((time_ms, weight));
            }
        }
    }
    let complete_pairs = pairs_by_time.iter().map(Vec::len).sum::<usize>();
    push_metric(
        metrics,
        window,
        "all",
        horizon,
        &protocol.output_name,
        FactorLens::CrossSectional,
        MetricId::Coverage,
        coverage(total_rows, complete_pairs),
    )?;
    push_metric(
        metrics,
        window,
        "all",
        horizon,
        &protocol.output_name,
        FactorLens::CrossSectional,
        MetricId::Missingness,
        coverage(total_rows, total_rows.saturating_sub(complete_pairs)),
    )?;
    push_metric(
        metrics,
        window,
        "all",
        horizon,
        &protocol.output_name,
        FactorLens::CrossSectional,
        MetricId::SampleCount,
        MetricObservation::available(complete_pairs as f64, complete_pairs as u64)?,
    )?;
    if protocol.lenses.contains(&FactorLens::CrossSectional) {
        push_metric(
            metrics,
            window,
            "all",
            horizon,
            &protocol.output_name,
            FactorLens::CrossSectional,
            MetricId::Ic,
            mean_observations(
                pairs_by_time
                    .iter()
                    .map(|pairs| correlation(pairs))
                    .collect::<Vec<_>>(),
            ),
        )?;
        push_metric(
            metrics,
            window,
            "all",
            horizon,
            &protocol.output_name,
            FactorLens::CrossSectional,
            MetricId::RankIc,
            mean_observations(
                pairs_by_time
                    .iter()
                    .map(|pairs| rank_correlation(pairs))
                    .collect::<Vec<_>>(),
            ),
        )?;
    }
    if protocol.lenses.contains(&FactorLens::Neutralized) {
        push_metric(
            metrics,
            window,
            "all",
            horizon,
            &protocol.output_name,
            FactorLens::Neutralized,
            MetricId::Neutralized,
            mean_observations(neutralized),
        )?;
    }
    if protocol.lenses.contains(&FactorLens::Economic) {
        push_metric(
            metrics,
            window,
            "top-only",
            horizon,
            &protocol.output_name,
            FactorLens::Economic,
            MetricId::Economic,
            mean_observations(economic.iter().map(|point| point.0.clone()).collect()),
        )?;
        push_metric(
            metrics,
            window,
            "top-minus-bottom",
            horizon,
            &protocol.output_name,
            FactorLens::Economic,
            MetricId::Economic,
            mean_observations(economic.iter().map(|point| point.1.clone()).collect()),
        )?;
        let turnover = turnover(&weights);
        push_metric(
            metrics,
            window,
            "top-only",
            horizon,
            &protocol.output_name,
            FactorLens::Economic,
            MetricId::Turnover,
            turnover,
        )?;
    }
    Ok(())
}

fn is_rebalance_point(index: usize, rebalance_every_bars: u32) -> bool {
    index % rebalance_every_bars as usize == 0
}

fn evaluate_regime(
    protocol: &FactorEvaluationProtocol,
    window: &EvaluationWindow,
    horizon: u32,
    selection: &[Observation],
    evaluation: &[Observation],
    feature_lookup: Option<&BTreeMap<(String, i64), BTreeMap<String, EvaluationFeatureCell>>>,
    metrics: &mut Vec<MetricRecord>,
    evidence: &mut Vec<RegimeEvidence>,
) -> Result<(), EvaluationError> {
    let regime = protocol
        .regime
        .as_ref()
        .expect("protocol validation requires a Regime Definition");
    let thresholds = regime_thresholds(regime, selection, feature_lookup)?;
    let mut buckets = vec![Vec::<(f64, f64)>::new(); regime.bucket_count as usize];
    if thresholds.len() == regime.bucket_count as usize - 1 {
        for observation in evaluation {
            let Some(factor) = observation.factor else {
                continue;
            };
            let Some(target) = observation.target else {
                continue;
            };
            let Some(feature) = feature_value(feature_lookup, observation, &regime.feature_name)
            else {
                continue;
            };
            let bucket = thresholds
                .iter()
                .position(|threshold| feature <= *threshold)
                .unwrap_or(thresholds.len());
            buckets[bucket].push((orient(factor, protocol), target));
        }
    }
    let bucket_metrics = buckets
        .iter()
        .map(|bucket| {
            enforce_metric_requirements(
                MetricId::Regime,
                mean_observation(bucket.iter().map(|pair| pair.1).collect()),
            )
        })
        .collect::<Vec<_>>();
    for (index, metric) in bucket_metrics.iter().enumerate() {
        push_metric(
            metrics,
            window,
            &format!("regime-{}", index + 1),
            horizon,
            &protocol.output_name,
            FactorLens::Regime,
            MetricId::Regime,
            metric.clone(),
        )?;
    }
    evidence.push(RegimeEvidence {
        fold_id: window.fold_id.clone(),
        horizon_bars: horizon,
        feature_name: regime.feature_name.clone(),
        bucket_count: regime.bucket_count,
        thresholds,
        bucket_metrics,
    });
    Ok(())
}

fn regime_thresholds(
    regime: &FactorRegimeDefinition,
    selection: &[Observation],
    feature_lookup: Option<&BTreeMap<(String, i64), BTreeMap<String, EvaluationFeatureCell>>>,
) -> Result<Vec<f64>, EvaluationError> {
    let mut values = selection
        .iter()
        .filter_map(|observation| feature_value(feature_lookup, observation, &regime.feature_name))
        .collect::<Vec<_>>();
    values.sort_by(f64::total_cmp);
    if values.is_empty() {
        return Ok(Vec::new());
    }
    Ok((1..regime.bucket_count)
        .map(|bucket| {
            let rank = ((bucket as usize * values.len()).div_ceil(regime.bucket_count as usize))
                .saturating_sub(1)
                .min(values.len() - 1);
            values[rank]
        })
        .collect())
}

fn feature_value(
    feature_lookup: Option<&BTreeMap<(String, i64), BTreeMap<String, EvaluationFeatureCell>>>,
    observation: &Observation,
    name: &str,
) -> Option<f64> {
    match feature_lookup?
        .get(&(observation.instrument_id.clone(), observation.time_ms))?
        .get(name)?
    {
        EvaluationFeatureCell::Available {
            value,
            available_at_ms,
        } if *available_at_ms <= observation.time_ms && value.is_finite() => Some(*value),
        _ => None,
    }
}

fn feature_lookup(
    evidence: &EvaluationFeatureEvidence,
) -> BTreeMap<(String, i64), BTreeMap<String, EvaluationFeatureCell>> {
    evidence
        .rows
        .iter()
        .map(|row| {
            (
                (row.instrument_id.clone(), row.observation_time_ms),
                row.values.clone(),
            )
        })
        .collect()
}

fn orient(value: f64, protocol: &FactorEvaluationProtocol) -> f64 {
    match protocol.orientation {
        crate::FactorOrientation::Positive => value,
        crate::FactorOrientation::Negative => -value,
    }
}

fn push_metric(
    metrics: &mut Vec<MetricRecord>,
    window: &EvaluationWindow,
    variant: &str,
    horizon_bars: u32,
    output_name: &str,
    lens: FactorLens,
    metric: MetricId,
    observation: MetricObservation,
) -> Result<(), EvaluationError> {
    metrics.push(MetricRecord {
        fold_id: window.fold_id.clone(),
        variant: variant.to_owned(),
        horizon_bars,
        output_name: output_name.to_owned(),
        lens,
        metric,
        observation: enforce_metric_requirements(metric, observation),
    });
    Ok(())
}

fn enforce_metric_requirements(
    metric: MetricId,
    observation: MetricObservation,
) -> MetricObservation {
    match observation {
        MetricObservation::Available {
            value: _,
            sample_count,
        } if FactorMetricCatalog::initial()
            .metric(metric)
            .is_some_and(|definition| sample_count < definition.minimum_samples) =>
        {
            MetricObservation::unavailable(MetricUndefinedReason::InsufficientSamples, sample_count)
        }
        observation => observation,
    }
}

fn coverage(denominator: usize, numerator: usize) -> MetricObservation {
    if denominator == 0 {
        MetricObservation::unavailable(MetricUndefinedReason::NoEligibleObservations, 0)
    } else {
        MetricObservation::available(numerator as f64 / denominator as f64, denominator as u64)
            .expect("coverage is finite")
    }
}

fn correlation(pairs: &[(f64, f64)]) -> MetricObservation {
    if pairs.len() < MIN_CORRELATION_SAMPLES {
        return MetricObservation::unavailable(
            MetricUndefinedReason::InsufficientSamples,
            pairs.len() as u64,
        );
    }
    let left = pairs.iter().map(|pair| pair.0).collect::<Vec<_>>();
    let right = pairs.iter().map(|pair| pair.1).collect::<Vec<_>>();
    let Some(value) = pearson(pairs) else {
        return MetricObservation::unavailable(
            MetricUndefinedReason::ConstantValues,
            pairs.len() as u64,
        );
    };
    if !value.is_finite() || left.iter().any(|value| !value.is_finite()) {
        MetricObservation::unavailable(
            MetricUndefinedReason::InvalidRequirement,
            pairs.len() as u64,
        )
    } else if right.iter().all(|value| *value == right[0]) {
        MetricObservation::unavailable(MetricUndefinedReason::ConstantValues, pairs.len() as u64)
    } else {
        MetricObservation::available(value, pairs.len() as u64).expect("pearson is finite")
    }
}

fn rank_correlation(pairs: &[(f64, f64)]) -> MetricObservation {
    if pairs.len() < MIN_CORRELATION_SAMPLES {
        return MetricObservation::unavailable(
            MetricUndefinedReason::InsufficientSamples,
            pairs.len() as u64,
        );
    }
    let ranked = average_ranks(pairs);
    correlation(&ranked)
}

fn pearson(pairs: &[(f64, f64)]) -> Option<f64> {
    if pairs.len() < 2 {
        return None;
    }
    let left_mean = pairs.iter().map(|pair| pair.0).sum::<f64>() / pairs.len() as f64;
    let right_mean = pairs.iter().map(|pair| pair.1).sum::<f64>() / pairs.len() as f64;
    let covariance = pairs
        .iter()
        .map(|pair| (pair.0 - left_mean) * (pair.1 - right_mean))
        .sum::<f64>();
    let left_variance = pairs
        .iter()
        .map(|pair| (pair.0 - left_mean).powi(2))
        .sum::<f64>();
    let right_variance = pairs
        .iter()
        .map(|pair| (pair.1 - right_mean).powi(2))
        .sum::<f64>();
    let denominator = (left_variance * right_variance).sqrt();
    (denominator > BASIS_EPSILON).then_some(covariance / denominator)
}

fn average_ranks(pairs: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let left = average_rank_values(&pairs.iter().map(|pair| pair.0).collect::<Vec<_>>());
    let right = average_rank_values(&pairs.iter().map(|pair| pair.1).collect::<Vec<_>>());
    left.into_iter().zip(right).collect()
}

fn average_rank_values(values: &[f64]) -> Vec<f64> {
    let mut order = (0..values.len()).collect::<Vec<_>>();
    order.sort_by(|left, right| values[*left].total_cmp(&values[*right]));
    let mut ranks = vec![0.0; values.len()];
    let mut start = 0;
    while start < order.len() {
        let mut end = start + 1;
        while end < order.len() && values[order[start]] == values[order[end]] {
            end += 1;
        }
        let average = (start + 1 + end) as f64 / 2.0;
        for index in &order[start..end] {
            ranks[*index] = average;
        }
        start = end;
    }
    ranks
}

fn mean_observations(observations: Vec<MetricObservation>) -> MetricObservation {
    let available = observations
        .iter()
        .filter_map(MetricObservation::value)
        .collect::<Vec<_>>();
    if available.is_empty() {
        let reason = observations
            .iter()
            .find_map(|observation| match observation {
                MetricObservation::Unavailable { reason, .. } => Some(*reason),
                MetricObservation::Available { .. } => None,
            })
            .unwrap_or(MetricUndefinedReason::NoEligibleObservations);
        MetricObservation::unavailable(reason, observations.len() as u64)
    } else {
        MetricObservation::available(
            available.iter().sum::<f64>() / available.len() as f64,
            available.len() as u64,
        )
        .expect("mean of finite metrics is finite")
    }
}

fn mean_observation(values: Vec<f64>) -> MetricObservation {
    if values.is_empty() {
        MetricObservation::unavailable(MetricUndefinedReason::NoEligibleObservations, 0)
    } else {
        MetricObservation::available(
            values.iter().sum::<f64>() / values.len() as f64,
            values.len() as u64,
        )
        .expect("mean of finite values is finite")
    }
}

#[derive(Debug)]
struct EconomicObservations {
    top_only: MetricObservation,
    top_minus_bottom: MetricObservation,
    turnover: MetricObservation,
}

fn economic_observations(
    pairs: &[(f64, f64)],
    assumptions: &EconomicAssumptions,
) -> EconomicObservations {
    if pairs.is_empty() {
        return EconomicObservations {
            top_only: MetricObservation::unavailable(
                MetricUndefinedReason::NoEligibleObservations,
                0,
            ),
            top_minus_bottom: MetricObservation::unavailable(
                MetricUndefinedReason::NoEligibleObservations,
                0,
            ),
            turnover: MetricObservation::unavailable(
                MetricUndefinedReason::NoEligibleObservations,
                0,
            ),
        };
    }
    let groups = quantile_groups(pairs);
    let top = groups.last().filter(|group| !group.is_empty());
    let bottom = groups.first().filter(|group| !group.is_empty());
    let top_only = top.map_or_else(
        || MetricObservation::unavailable(MetricUndefinedReason::NoEligibleObservations, 0),
        |group| {
            let value = group.iter().map(|index| pairs[*index].1).sum::<f64>() / group.len() as f64
                - cost_rate(assumptions);
            MetricObservation::available(value, pairs.len() as u64).expect("economic value finite")
        },
    );
    let top_minus_bottom = if !assumptions.long_short {
        MetricObservation::unavailable(MetricUndefinedReason::NotApplicable, 0)
    } else {
        match (top, bottom) {
            (Some(top), Some(bottom)) => {
                let top_mean =
                    top.iter().map(|index| pairs[*index].1).sum::<f64>() / top.len() as f64;
                let bottom_mean =
                    bottom.iter().map(|index| pairs[*index].1).sum::<f64>() / bottom.len() as f64;
                MetricObservation::available(
                    top_mean - bottom_mean - 2.0 * cost_rate(assumptions),
                    pairs.len() as u64,
                )
                .expect("economic spread finite")
            }
            _ => MetricObservation::unavailable(MetricUndefinedReason::NoEligibleObservations, 0),
        }
    };
    EconomicObservations {
        top_only,
        top_minus_bottom,
        turnover: MetricObservation::unavailable(MetricUndefinedReason::NotApplicable, 0),
    }
}

fn cost_rate(assumptions: &EconomicAssumptions) -> f64 {
    (assumptions.fee_bps + assumptions.slippage_bps) / 10_000.0
}

fn quantile_groups(pairs: &[(f64, f64)]) -> Vec<Vec<usize>> {
    let mut order = (0..pairs.len()).collect::<Vec<_>>();
    order.sort_by(|left, right| {
        pairs[*left]
            .0
            .total_cmp(&pairs[*right].0)
            .then(left.cmp(right))
    });
    let mut groups = vec![Vec::new(); 5];
    let mut start = 0;
    while start < order.len() {
        let mut end = start + 1;
        while end < order.len() && pairs[order[start]].0 == pairs[order[end]].0 {
            end += 1;
        }
        let bucket = (start * 5 / order.len()).min(4);
        groups[bucket].extend_from_slice(&order[start..end]);
        start = end;
    }
    groups
}

struct CrossSectionalEconomic {
    top_only: MetricObservation,
    top_minus_bottom: MetricObservation,
    top_weights: Option<BTreeMap<String, f64>>,
}

fn cross_sectional_economic(
    rows: &[&Observation],
    protocol: &FactorEvaluationProtocol,
) -> CrossSectionalEconomic {
    let pairs = rows
        .iter()
        .filter_map(|row| {
            Some((
                orient(row.factor?, protocol),
                row.target?,
                row.instrument_id.clone(),
            ))
        })
        .collect::<Vec<_>>();
    if pairs.is_empty() {
        return CrossSectionalEconomic {
            top_only: MetricObservation::unavailable(
                MetricUndefinedReason::NoEligibleObservations,
                0,
            ),
            top_minus_bottom: MetricObservation::unavailable(
                MetricUndefinedReason::NoEligibleObservations,
                0,
            ),
            top_weights: None,
        };
    }
    let simple = pairs
        .iter()
        .map(|pair| (pair.0, pair.1))
        .collect::<Vec<_>>();
    let groups = quantile_groups(&simple);
    let top = groups.last().filter(|group| !group.is_empty());
    let bottom = groups.first().filter(|group| !group.is_empty());
    let top_weights = top.map(|group| {
        group
            .iter()
            .map(|index| (pairs[*index].2.clone(), 1.0 / group.len() as f64))
            .collect()
    });
    let top_only = top.map_or_else(
        || MetricObservation::unavailable(MetricUndefinedReason::NoEligibleObservations, 0),
        |group| {
            MetricObservation::available(
                group.iter().map(|index| pairs[*index].1).sum::<f64>() / group.len() as f64
                    - cost_rate(&protocol.economic),
                pairs.len() as u64,
            )
            .expect("economic value finite")
        },
    );
    let top_minus_bottom = if !protocol.economic.long_short {
        MetricObservation::unavailable(MetricUndefinedReason::NotApplicable, 0)
    } else {
        match (top, bottom) {
            (Some(top), Some(bottom)) => MetricObservation::available(
                top.iter().map(|index| pairs[*index].1).sum::<f64>() / top.len() as f64
                    - bottom.iter().map(|index| pairs[*index].1).sum::<f64>() / bottom.len() as f64
                    - 2.0 * cost_rate(&protocol.economic),
                pairs.len() as u64,
            )
            .expect("economic spread finite"),
            _ => MetricObservation::unavailable(MetricUndefinedReason::NoEligibleObservations, 0),
        }
    };
    CrossSectionalEconomic {
        top_only,
        top_minus_bottom,
        top_weights,
    }
}

fn turnover(points: &[(i64, BTreeMap<String, f64>)]) -> MetricObservation {
    if points.len() < 2 {
        return MetricObservation::unavailable(
            MetricUndefinedReason::InsufficientSamples,
            points.len() as u64,
        );
    }
    let mut total = 0.0;
    for pair in points.windows(2) {
        let names = pair[0]
            .1
            .keys()
            .chain(pair[1].1.keys())
            .cloned()
            .collect::<BTreeSet<_>>();
        total += names
            .iter()
            .map(|name| {
                (pair[0].1.get(name).copied().unwrap_or(0.0)
                    - pair[1].1.get(name).copied().unwrap_or(0.0))
                .abs()
            })
            .sum::<f64>()
            / 2.0;
    }
    MetricObservation::available(total / (points.len() - 1) as f64, (points.len() - 1) as u64)
        .expect("turnover is finite")
}

fn neutralized_observation(
    rows: &[&Observation],
    feature_lookup: Option<&BTreeMap<(String, i64), BTreeMap<String, EvaluationFeatureCell>>>,
    protocol: &FactorEvaluationProtocol,
    time_ms: i64,
) -> MetricObservation {
    let mut factor = Vec::new();
    let mut target = Vec::new();
    let mut design = Vec::new();
    for row in rows {
        let (Some(factor_value), Some(target_value)) = (row.factor, row.target) else {
            continue;
        };
        let Some(values) =
            feature_lookup.and_then(|lookup| lookup.get(&(row.instrument_id.clone(), time_ms)))
        else {
            continue;
        };
        let Some(nuisance) = protocol
            .nuisance_feature_names
            .iter()
            .map(|name| match values.get(name) {
                Some(EvaluationFeatureCell::Available {
                    value,
                    available_at_ms,
                }) if *available_at_ms <= time_ms && value.is_finite() => Some(*value),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()
        else {
            continue;
        };
        factor.push(orient(factor_value, protocol));
        target.push(target_value);
        design.push(nuisance);
    }
    if design.len() < protocol.nuisance_feature_names.len() + 2 {
        return MetricObservation::unavailable(
            MetricUndefinedReason::InsufficientSamples,
            design.len() as u64,
        );
    }
    let factor_residuals = match ols_residuals(&design, &factor) {
        Ok(residuals) => residuals,
        Err(reason) => return MetricObservation::unavailable(reason, design.len() as u64),
    };
    let target_residuals = match ols_residuals(&design, &target) {
        Ok(residuals) => residuals,
        Err(reason) => return MetricObservation::unavailable(reason, design.len() as u64),
    };
    correlation(
        &factor_residuals
            .iter()
            .copied()
            .zip(target_residuals)
            .collect::<Vec<_>>(),
    )
}

fn ols_residuals(design: &[Vec<f64>], values: &[f64]) -> Result<Vec<f64>, MetricUndefinedReason> {
    let columns = design.first().map_or(0, Vec::len) + 1;
    if columns == 0 || design.len() < columns {
        return Err(MetricUndefinedReason::InsufficientSamples);
    }
    let mut normal = vec![vec![0.0; columns + 1]; columns];
    for (row, value) in design.iter().zip(values) {
        let mut x = Vec::with_capacity(columns);
        x.push(1.0);
        x.extend(row);
        for left in 0..columns {
            for right in 0..columns {
                normal[left][right] += x[left] * x[right];
            }
            normal[left][columns] += x[left] * value;
        }
    }
    for pivot in 0..columns {
        let Some(best) = (pivot..columns).max_by(|left, right| {
            normal[*left][pivot]
                .abs()
                .total_cmp(&normal[*right][pivot].abs())
        }) else {
            return Err(MetricUndefinedReason::SingularMatrix);
        };
        if normal[best][pivot].abs() <= BASIS_EPSILON {
            return Err(MetricUndefinedReason::SingularMatrix);
        }
        normal.swap(pivot, best);
        let divisor = normal[pivot][pivot];
        for column in pivot..=columns {
            normal[pivot][column] /= divisor;
        }
        for row in 0..columns {
            if row == pivot {
                continue;
            }
            let multiplier = normal[row][pivot];
            for column in pivot..=columns {
                normal[row][column] -= multiplier * normal[pivot][column];
            }
        }
    }
    let coefficients = normal.iter().map(|row| row[columns]).collect::<Vec<_>>();
    Ok(design
        .iter()
        .zip(values)
        .map(|(row, value)| {
            let predicted = coefficients[0]
                + row
                    .iter()
                    .zip(&coefficients[1..])
                    .map(|(value, coefficient)| value * coefficient)
                    .sum::<f64>();
            value - predicted
        })
        .collect())
}

fn add_decay_and_stability(
    protocol: &FactorEvaluationProtocol,
    existing: &[MetricRecord],
    metrics: &mut Vec<MetricRecord>,
) {
    let variants = existing
        .iter()
        .filter(|record| record.metric == MetricId::Ic)
        .map(|record| (record.variant.clone(), record.output_name.clone()))
        .collect::<BTreeSet<_>>();
    for (variant, output_name) in variants {
        for horizon in &protocol.horizon_bars {
            let ics = existing
                .iter()
                .filter(|record| {
                    record.metric == MetricId::Ic
                        && record.variant == variant
                        && record.horizon_bars == *horizon
                })
                .filter_map(|record| record.observation.value())
                .collect::<Vec<_>>();
            let decay = if protocol.horizon_bars.len() < 3 {
                MetricObservation::unavailable(MetricUndefinedReason::NoEligibleObservations, 0)
            } else {
                let first_horizon = protocol.horizon_bars.first().copied().unwrap_or(*horizon);
                let first_value = existing
                    .iter()
                    .filter(|record| {
                        record.metric == MetricId::Ic
                            && record.variant == variant
                            && record.horizon_bars == first_horizon
                    })
                    .filter_map(|record| record.observation.value())
                    .collect::<Vec<_>>();
                let current_value = ics.iter().sum::<f64>() / ics.len().max(1) as f64;
                match (
                    (!first_value.is_empty())
                        .then(|| first_value.iter().sum::<f64>() / first_value.len() as f64),
                    (!ics.is_empty()).then_some(current_value),
                ) {
                    (Some(first), Some(last)) if *horizon != first_horizon => {
                        MetricObservation::available(
                            last - first,
                            protocol.horizon_bars.len() as u64,
                        )
                        .expect("decay finite")
                    }
                    _ => MetricObservation::unavailable(
                        MetricUndefinedReason::InsufficientSamples,
                        ics.len() as u64,
                    ),
                }
            };
            metrics.push(MetricRecord {
                fold_id: "aggregate".into(),
                variant: variant.clone(),
                horizon_bars: *horizon,
                output_name: output_name.clone(),
                lens: match protocol.scope {
                    FactorScope::TimeSeries => FactorLens::Temporal,
                    FactorScope::CrossSectional => FactorLens::CrossSectional,
                },
                metric: MetricId::Decay,
                observation: enforce_metric_requirements(MetricId::Decay, decay),
            });
            let stability = if ics.len() < 2 {
                MetricObservation::unavailable(
                    MetricUndefinedReason::InsufficientSamples,
                    ics.len() as u64,
                )
            } else {
                let positive = ics.iter().filter(|value| **value >= 0.0).count();
                let sign = positive.max(ics.len() - positive) as f64 / ics.len() as f64;
                MetricObservation::available(sign, ics.len() as u64).expect("stability finite")
            };
            metrics.push(MetricRecord {
                fold_id: "aggregate".into(),
                variant: variant.clone(),
                horizon_bars: *horizon,
                output_name: output_name.clone(),
                lens: match protocol.scope {
                    FactorScope::TimeSeries => FactorLens::Temporal,
                    FactorScope::CrossSectional => FactorLens::CrossSectional,
                },
                metric: MetricId::Stability,
                observation: enforce_metric_requirements(MetricId::Stability, stability),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        FactorDatasetManifest, FactorDatasetRow, FactorEvaluationProtocolDraft,
        FactorMarketContext, FactorOrientation, FactorTarget, ResearchEngineProvenance,
    };
    use rust_decimal::Decimal;
    use std::collections::BTreeMap;
    use uuid::Uuid;

    fn engine() -> ResearchEngineProvenance {
        ResearchEngineProvenance {
            engine_id: "adaq-native".into(),
            engine_version: "1.0.0".into(),
            adapter: "native".into(),
            target_triple: "test".into(),
            build_id: "evaluation-test".into(),
            environment: BTreeMap::new(),
            parameters: BTreeMap::new(),
            input_identities: vec!["input".into()],
        }
    }

    fn context() -> FactorMarketContext {
        FactorMarketContext {
            venue: "TEST".into(),
            asset_class: "equity".into(),
            bar_interval: "1h".into(),
            price_basis: "unadjusted".into(),
            valuation_currency: "USD".into(),
            point_in_time_universe_id: "universe".into(),
        }
    }

    fn bars(closes: &[i64]) -> Vec<OhlcvBar> {
        closes
            .iter()
            .enumerate()
            .map(|(index, close)| OhlcvBar {
                open_time_ms: index as i64 * 100,
                open: Decimal::from(*close),
                high: Decimal::from(*close),
                low: Decimal::from(*close),
                close: Decimal::from(*close),
                base_volume: Decimal::ONE,
                quote_volume: Decimal::ONE,
            })
            .collect()
    }

    fn protocol(scope: FactorScope, windows: Vec<EvaluationWindow>) -> FactorEvaluationProtocol {
        FactorEvaluationProtocol::freeze(FactorEvaluationProtocolDraft {
            protocol_id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            factor_dataset_id: "factor-dataset".into(),
            feature_dataset_id: "feature-dataset".into(),
            feature_plan_hash: "b".repeat(64),
            market_data_snapshot_id: "snapshot".into(),
            point_in_time_universe_id: "universe".into(),
            point_in_time_universe: vec!["A".into(), "B".into()],
            output_name: "score".into(),
            scope,
            target: FactorTarget::FutureCloseReturn,
            horizon_bars: vec![1, 2],
            market_context: context(),
            engine_identity: engine(),
            orientation: FactorOrientation::Positive,
            windows,
            purge_bars: 0,
            embargo_bars: 0,
            lenses: match scope {
                FactorScope::TimeSeries => vec![FactorLens::Temporal, FactorLens::Economic],
                FactorScope::CrossSectional => {
                    vec![FactorLens::CrossSectional, FactorLens::Economic]
                }
            },
            nuisance_feature_names: vec![],
            regime: None,
            economic: EconomicAssumptions {
                rebalance_every_bars: 1,
                fee_bps: 10.0,
                slippage_bps: 5.0,
                long_short: true,
            },
            family_id: Uuid::new_v4(),
            trial_id: Uuid::new_v4(),
            seed: 7,
        })
        .unwrap()
    }

    fn window() -> EvaluationWindow {
        EvaluationWindow {
            fold_id: "fold-1".into(),
            selection: crate::ObservationRange {
                start_time_ms: 0,
                end_time_ms: 200,
            },
            evaluation: crate::ObservationRange {
                start_time_ms: 200,
                end_time_ms: 500,
            },
            training: Some(crate::ObservationRange {
                start_time_ms: 0,
                end_time_ms: 100,
            }),
            fitting: Some(crate::ObservationRange {
                start_time_ms: 0,
                end_time_ms: 100,
            }),
            normalization: Some(crate::ObservationRange {
                start_time_ms: 0,
                end_time_ms: 100,
            }),
            target_construction: Some(crate::ObservationRange {
                start_time_ms: 0,
                end_time_ms: 100,
            }),
        }
    }

    fn dataset(scope: FactorScope, rows: Vec<FactorDatasetRow>) -> FactorDataset {
        let output_names = vec!["score".into()];
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Payload<'a> {
            output_names: &'a [String],
            rows: &'a [FactorDatasetRow],
        }
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Identity<'a> {
            schema_version: &'a str,
            protocol_hash: &'a str,
            candidate_hash: &'a str,
            scope: FactorScope,
            feature_dataset_id: &'a str,
            feature_plan_hash: &'a str,
            market_data_snapshot_id: &'a str,
            point_in_time_universe_id: &'a str,
            market_context: &'a FactorMarketContext,
            output_names: &'a [String],
            observation_count: u64,
            payload_sha256: &'a str,
            engine_identity: &'a ResearchEngineProvenance,
        }
        let payload = serde_json::to_vec(&Payload {
            output_names: &output_names,
            rows: &rows,
        })
        .unwrap();
        let payload_sha256 = adaq_feature_engine::sha256(&payload);
        let protocol_hash = "a".repeat(64);
        let mut manifest = FactorDatasetManifest {
            schema_version: crate::FACTOR_RESEARCH_SCHEMA_VERSION.into(),
            dataset_id: "factor-dataset".into(),
            protocol_hash,
            candidate_hash: "c".repeat(64),
            scope,
            feature_dataset_id: "feature-dataset".into(),
            feature_plan_hash: "b".repeat(64),
            market_data_snapshot_id: "snapshot".into(),
            point_in_time_universe_id: "universe".into(),
            market_context: context(),
            output_names,
            observation_count: rows.len() as u64,
            payload_sha256,
            engine_identity: engine(),
        };
        manifest.dataset_id = crate::content_hash(&Identity {
            schema_version: &manifest.schema_version,
            protocol_hash: &manifest.protocol_hash,
            candidate_hash: &manifest.candidate_hash,
            scope: manifest.scope,
            feature_dataset_id: &manifest.feature_dataset_id,
            feature_plan_hash: &manifest.feature_plan_hash,
            market_data_snapshot_id: &manifest.market_data_snapshot_id,
            point_in_time_universe_id: &manifest.point_in_time_universe_id,
            market_context: &manifest.market_context,
            output_names: &manifest.output_names,
            observation_count: manifest.observation_count,
            payload_sha256: &manifest.payload_sha256,
            engine_identity: &manifest.engine_identity,
        })
        .unwrap();
        FactorDataset { manifest, rows }
    }

    fn row(instrument_id: &str, time_ms: i64, score: Option<f64>) -> FactorDatasetRow {
        let values = score.map_or_else(
            || {
                BTreeMap::from([(
                    "score".into(),
                    FactorObservationValue::Unavailable {
                        reason: crate::FactorUnavailabilityReason::MissingInput,
                    },
                )])
            },
            |value| {
                BTreeMap::from([(
                    "score".into(),
                    FactorObservationValue::Available {
                        value,
                        available_at_ms: time_ms,
                    },
                )])
            },
        );
        FactorDatasetRow {
            instrument_id: instrument_id.into(),
            observation_time_ms: time_ms,
            values,
        }
    }

    fn series(instrument_id: &str, closes: &[i64]) -> FactorMarketSeries {
        FactorMarketSeries {
            instrument_id: instrument_id.into(),
            snapshot_id: "snapshot".into(),
            market_context: context(),
            bars: bars(closes),
            gaps: vec![],
            corporate_action_evidence: CorporateActionEvidence::Verified,
        }
    }

    #[test]
    fn evaluates_causal_targets_and_retains_typed_target_failures() {
        let protocol = protocol(FactorScope::TimeSeries, vec![window()]);
        let dataset = dataset(
            FactorScope::TimeSeries,
            vec![row("A", 200, Some(1.0)), row("A", 300, Some(2.0))],
        );
        let mut market = series("A", &[10, 11, 12, 13]);
        market.gaps.push(BarGap {
            start_time_ms: 250,
            end_time_ms: 275,
        });
        let report = FactorEvaluator::evaluate(FactorEvaluationInput {
            dataset: &dataset,
            protocol: &protocol,
            market_series: &[market],
            feature_evidence: None,
        })
        .unwrap();
        assert!(
            report
                .target_unavailable
                .iter()
                .any(|evidence| evidence.reason == TargetUnavailableReason::BarGap)
        );
        assert!(report.validate().is_ok());
    }

    #[test]
    fn cross_sectional_evaluation_preserves_ties_and_costs() {
        let protocol = protocol(FactorScope::CrossSectional, vec![window()]);
        protocol.validate().unwrap();
        let dataset = dataset(
            FactorScope::CrossSectional,
            vec![
                row("A", 200, Some(1.0)),
                row("A", 300, Some(2.0)),
                row("B", 200, Some(1.0)),
                row("B", 300, Some(0.5)),
            ],
        );
        let market = [
            series("A", &[10, 11, 12, 13]),
            series("B", &[10, 12, 11, 14]),
        ];
        let report = FactorEvaluator::evaluate(FactorEvaluationInput {
            dataset: &dataset,
            protocol: &protocol,
            market_series: &market,
            feature_evidence: None,
        })
        .unwrap();
        let repeat = FactorEvaluator::evaluate(FactorEvaluationInput {
            dataset: &dataset,
            protocol: &protocol,
            market_series: &market,
            feature_evidence: None,
        })
        .unwrap();
        assert_eq!(report.metrics, repeat.metrics);
        assert_ne!(report.report_id, repeat.report_id);
        assert!(
            report.metrics.iter().any(|metric| {
                metric.metric == MetricId::Economic && metric.variant == "top-only"
            })
        );
        assert!(report.metrics.iter().any(|metric| {
            metric.metric == MetricId::RankIc
                && matches!(
                    metric.observation,
                    MetricObservation::Unavailable {
                        reason: MetricUndefinedReason::InsufficientSamples,
                        ..
                    }
                )
        }));
    }

    #[test]
    fn neutralization_and_regimes_use_only_selection_feature_evidence() {
        let mut protocol = protocol(FactorScope::CrossSectional, vec![window()]);
        protocol.nuisance_feature_names = vec!["size".into()];
        protocol.regime = Some(FactorRegimeDefinition {
            feature_name: "size".into(),
            bucket_count: 2,
        });
        let protocol = FactorEvaluationProtocol::freeze(FactorEvaluationProtocolDraft {
            protocol_id: protocol.protocol_id,
            user_id: protocol.user_id,
            factor_dataset_id: protocol.factor_dataset_id,
            feature_dataset_id: protocol.feature_dataset_id,
            feature_plan_hash: protocol.feature_plan_hash,
            market_data_snapshot_id: protocol.market_data_snapshot_id,
            point_in_time_universe_id: protocol.point_in_time_universe_id,
            point_in_time_universe: protocol.point_in_time_universe,
            output_name: protocol.output_name,
            scope: protocol.scope,
            target: protocol.target,
            horizon_bars: protocol.horizon_bars,
            market_context: protocol.market_context,
            engine_identity: protocol.engine_identity,
            orientation: protocol.orientation,
            windows: protocol.windows,
            purge_bars: protocol.purge_bars,
            embargo_bars: protocol.embargo_bars,
            lenses: vec![
                FactorLens::CrossSectional,
                FactorLens::Economic,
                FactorLens::Neutralized,
                FactorLens::Regime,
            ],
            nuisance_feature_names: vec!["size".into()],
            regime: Some(FactorRegimeDefinition {
                feature_name: "size".into(),
                bucket_count: 2,
            }),
            economic: protocol.economic,
            family_id: protocol.family_id,
            trial_id: protocol.trial_id,
            seed: protocol.seed,
        })
        .unwrap();
        let dataset = dataset(
            FactorScope::CrossSectional,
            vec![
                row("A", 0, Some(0.5)),
                row("A", 100, Some(0.8)),
                row("A", 200, Some(1.0)),
                row("A", 300, Some(2.0)),
                row("B", 0, Some(0.5)),
                row("B", 100, Some(0.8)),
                row("B", 200, Some(2.0)),
                row("B", 300, Some(1.0)),
            ],
        );
        let features = EvaluationFeatureEvidence {
            feature_dataset_id: "feature-dataset".into(),
            feature_plan_hash: "b".repeat(64),
            rows: vec![
                EvaluationFeatureRow {
                    instrument_id: "A".into(),
                    observation_time_ms: 0,
                    values: BTreeMap::from([(
                        "size".into(),
                        EvaluationFeatureCell::Available {
                            value: 1.0,
                            available_at_ms: 0,
                        },
                    )]),
                },
                EvaluationFeatureRow {
                    instrument_id: "A".into(),
                    observation_time_ms: 100,
                    values: BTreeMap::from([(
                        "size".into(),
                        EvaluationFeatureCell::Available {
                            value: 2.0,
                            available_at_ms: 100,
                        },
                    )]),
                },
                EvaluationFeatureRow {
                    instrument_id: "A".into(),
                    observation_time_ms: 200,
                    values: BTreeMap::from([(
                        "size".into(),
                        EvaluationFeatureCell::Available {
                            value: 3.0,
                            available_at_ms: 200,
                        },
                    )]),
                },
                EvaluationFeatureRow {
                    instrument_id: "A".into(),
                    observation_time_ms: 300,
                    values: BTreeMap::from([(
                        "size".into(),
                        EvaluationFeatureCell::Available {
                            value: 5.0,
                            available_at_ms: 300,
                        },
                    )]),
                },
                EvaluationFeatureRow {
                    instrument_id: "B".into(),
                    observation_time_ms: 0,
                    values: BTreeMap::from([(
                        "size".into(),
                        EvaluationFeatureCell::Available {
                            value: 1.0,
                            available_at_ms: 0,
                        },
                    )]),
                },
                EvaluationFeatureRow {
                    instrument_id: "B".into(),
                    observation_time_ms: 100,
                    values: BTreeMap::from([(
                        "size".into(),
                        EvaluationFeatureCell::Available {
                            value: 2.0,
                            available_at_ms: 100,
                        },
                    )]),
                },
                EvaluationFeatureRow {
                    instrument_id: "B".into(),
                    observation_time_ms: 200,
                    values: BTreeMap::from([(
                        "size".into(),
                        EvaluationFeatureCell::Available {
                            value: 4.0,
                            available_at_ms: 200,
                        },
                    )]),
                },
                EvaluationFeatureRow {
                    instrument_id: "B".into(),
                    observation_time_ms: 300,
                    values: BTreeMap::from([(
                        "size".into(),
                        EvaluationFeatureCell::Available {
                            value: 6.0,
                            available_at_ms: 300,
                        },
                    )]),
                },
            ],
        };
        let report = FactorEvaluator::evaluate(FactorEvaluationInput {
            dataset: &dataset,
            protocol: &protocol,
            market_series: &[
                series("A", &[10, 11, 12, 13]),
                series("B", &[10, 12, 11, 14]),
            ],
            feature_evidence: Some(&features),
        })
        .unwrap();
        assert!(!report.regime_evidence.is_empty());
        assert!(
            report
                .regime_evidence
                .iter()
                .any(|evidence| !evidence.thresholds.is_empty())
        );
        assert!(
            report
                .metrics
                .iter()
                .any(|metric| metric.metric == MetricId::Neutralized)
        );
    }

    #[test]
    fn target_causality_and_boundary_controls_are_typed() {
        let missing_close = series("A", &[10, 0]);
        assert_eq!(
            target_for(&missing_close, 0, 1).reason,
            Some(TargetUnavailableReason::MissingClose)
        );
        let mut corporate_actions = series("A", &[10, 11]);
        corporate_actions.corporate_action_evidence = CorporateActionEvidence::Unavailable {
            reason: "missing split evidence".into(),
        };
        assert_eq!(
            target_for(&corporate_actions, 0, 1).reason,
            Some(TargetUnavailableReason::CorporateActionUnavailable)
        );
        let scheduled_closure = FactorMarketSeries {
            bars: vec![
                OhlcvBar {
                    open_time_ms: 0,
                    open: Decimal::from(10),
                    high: Decimal::from(10),
                    low: Decimal::from(10),
                    close: Decimal::from(10),
                    base_volume: Decimal::ONE,
                    quote_volume: Decimal::ONE,
                },
                OhlcvBar {
                    open_time_ms: 300,
                    open: Decimal::from(11),
                    high: Decimal::from(11),
                    low: Decimal::from(11),
                    close: Decimal::from(11),
                    base_volume: Decimal::ONE,
                    quote_volume: Decimal::ONE,
                },
            ],
            ..series("A", &[10, 11])
        };
        assert!(target_for(&scheduled_closure, 0, 1).value.is_some());

        let series = series("A", &[10, 11, 12, 13]);
        let window = window();
        let leaking = Observation {
            instrument_id: "A".into(),
            time_ms: 100,
            factor: Some(1.0),
            target: Some(0.1),
            target_time_ms: Some(200),
        };
        assert!(purged_or_embargoed(&leaking, &window, 1, &series, 0, 0));
        let embargoed = Observation {
            time_ms: 0,
            target_time_ms: None,
            ..leaking.clone()
        };
        assert!(purged_or_embargoed(&embargoed, &window, 1, &series, 0, 2));
        assert!(!purged_or_embargoed(&embargoed, &window, 1, &series, 0, 0));
    }

    #[test]
    fn metric_math_uses_average_ties_and_typed_singular_ols() {
        assert_eq!(
            average_rank_values(&[1.0, 1.0, 3.0, 4.0]),
            vec![1.5, 1.5, 3.0, 4.0]
        );
        assert_eq!(
            ols_residuals(&[vec![1.0], vec![1.0], vec![1.0]], &[1.0, 2.0, 3.0]),
            Err(MetricUndefinedReason::SingularMatrix)
        );
    }

    #[test]
    fn rejects_market_context_that_is_not_the_protocol_context() {
        let protocol = protocol(FactorScope::TimeSeries, vec![window()]);
        let dataset = dataset(FactorScope::TimeSeries, vec![row("A", 200, Some(1.0))]);
        let mut market = series("A", &[10, 11, 12]);
        market.market_context.price_basis = "adjusted".into();
        assert!(matches!(
            FactorEvaluator::evaluate(FactorEvaluationInput {
                dataset: &dataset,
                protocol: &protocol,
                market_series: &[market],
                feature_evidence: None,
            }),
            Err(EvaluationError::Invalid(_))
        ));
    }
}
