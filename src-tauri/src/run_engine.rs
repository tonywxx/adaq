use std::collections::{HashMap, HashSet};

use adaq_backtest_core::TargetDecision;
use adaq_component_sdk::host::factor_abi;
use adaq_component_sdk::host::strategy_abi;
use adaq_component_tooling::{
    ComponentParameterValue, FrozenFeaturePlan, FrozenSourceView, RunLimits, WasmLoader,
};
use adaq_data_core::{BarGap, BarInterval, OhlcvBar, next_bar_open_time_ms};
use adaq_feature_engine::{
    FeatureDependencyInput, FeatureEngine, FeatureEvaluationError, FeatureEvaluationInput,
    FeatureInputEvent, FeatureMarketBar, FeatureObservation, FeatureObservationValue,
    FeatureUnavailabilityReason,
};
use rust_decimal::{Decimal, prelude::ToPrimitive};

const MAX_CLOSED_BARS: usize = 1_000_000;
const MAX_GUEST_CAUSE_BYTES: usize = 4 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PositionMode {
    LongOnly,
    #[cfg(test)]
    LongShort,
}

#[derive(Clone, Copy)]
pub(crate) struct RunRequest<'a> {
    pub strategy_path: &'a str,
    pub strategy_parameters: &'a [ComponentParameterValue],
    pub factors: &'a [FactorRunRequest<'a>],
    pub signals: &'a [SignalRunRequest<'a>],
    pub bars: &'a [OhlcvBar],
    pub gaps: &'a [BarGap],
    pub plan: &'a FrozenFeaturePlan,
    pub position_mode: PositionMode,
    pub limits: RunLimits,
}

#[derive(Clone, Copy)]
pub(crate) struct SignalRunRequest<'a> {
    pub slot: &'a str,
    pub dataset_id: &'a str,
    pub signal_name: &'a str,
    pub interval: BarInterval,
    pub rows: &'a [SignalRunRow],
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SignalRunRow {
    pub prediction_time_ms: i64,
    pub available_at_ms: i64,
    pub value: Option<f64>,
}

#[derive(Clone, Copy)]
pub(crate) struct FactorRunRequest<'a> {
    pub alias: &'a str,
    pub path: &'a str,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum MaterializedFeatureRow {
    Warmup,
    MissingInput { slot: String, source: String },
    Present(Vec<f64>),
}

pub(crate) fn materialize_feature_segment(
    plan: &FrozenFeaturePlan,
    factors: &[FactorRunRequest<'_>],
    bars: &[OhlcvBar],
    limits: RunLimits,
) -> Result<Vec<MaterializedFeatureRow>, RunError> {
    materialize_feature_segment_with_signals(plan, factors, &[], bars, limits)
}

pub(crate) fn materialize_feature_segment_with_signals(
    plan: &FrozenFeaturePlan,
    factors: &[FactorRunRequest<'_>],
    signals: &[SignalRunRequest<'_>],
    bars: &[OhlcvBar],
    limits: RunLimits,
) -> Result<Vec<MaterializedFeatureRow>, RunError> {
    let request = RunRequest {
        strategy_path: "",
        strategy_parameters: &[],
        factors,
        signals,
        bars,
        gaps: &[],
        plan,
        position_mode: PositionMode::LongOnly,
        limits,
    };
    evaluate_feature_rows(&request, bars)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunResult {
    pub plan_hash: String,
    pub bars: Vec<OhlcvBar>,
    pub decisions: Vec<TargetDecision>,
    pub gap_resets: Vec<BarGap>,
    pub pauses: Vec<RunPause>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunPause {
    pub open_time_ms: i64,
    pub reason: RunPauseReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RunPauseReason {
    Warmup,
    MissingInput { slot: String, source: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RunStage {
    Validation,
    Factor,
    FeatureFrame,
    Strategy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunError {
    pub code: String,
    pub stage: RunStage,
    pub bar_open_time_ms: Option<i64>,
    pub slot: Option<String>,
    pub source: Option<String>,
    pub ta_ret_code: Option<i32>,
    pub ta_ret_code_name: Option<String>,
    pub cause: String,
    pub cause_truncated: bool,
}

impl std::fmt::Display for RunError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} at {:?}: {}",
            self.code, self.stage, self.cause
        )
    }
}

impl std::error::Error for RunError {}

impl RunError {
    fn host(code: &str, stage: RunStage, cause: impl Into<String>) -> Self {
        let (cause, cause_truncated) = bounded_context(&cause.into());
        Self {
            code: code.into(),
            stage,
            bar_open_time_ms: None,
            slot: None,
            source: None,
            ta_ret_code: None,
            ta_ret_code_name: None,
            cause,
            cause_truncated,
        }
    }

    fn at_bar(mut self, bar: i64) -> Self {
        self.bar_open_time_ms = Some(bar);
        self
    }

    fn for_slot(mut self, slot: &str, source: impl Into<String>) -> Self {
        self.slot = Some(slot.into());
        self.source = Some(source.into());
        self
    }
}

pub(crate) struct RunEngine;

impl RunEngine {
    pub fn execute(request: &RunRequest<'_>) -> Result<RunResult, RunError> {
        validate_bar_count(request.bars.len(), request.limits.max_bars)
            .map_err(|error| RunError::host("invalid-run-request", RunStage::Validation, error))?;
        let bars = normalize_closed_bars(request.bars)?;
        let request = RunRequest {
            bars: &bars,
            ..*request
        };
        validate_request(&request)
            .map_err(|error| RunError::host("invalid-run-request", RunStage::Validation, error))?;
        let mut result = RunResult {
            plan_hash: request.plan.plan_hash().to_owned(),
            bars: bars.clone(),
            decisions: Vec::with_capacity(bars.len()),
            gap_resets: Vec::new(),
            pauses: Vec::new(),
        };
        let mut start = 0;
        let mut next_gap = 0;
        while start < bars.len() {
            let end = request
                .gaps
                .get(next_gap)
                .and_then(|gap| {
                    request
                        .bars
                        .iter()
                        .position(|bar| bar.open_time_ms >= gap.start_time_ms)
                })
                .unwrap_or(bars.len());
            if end > start {
                execute_segment(&request, &bars[start..end], &mut result)?;
            }
            start = end;
            if let Some(gap) = request.gaps.get(next_gap).copied() {
                result.gap_resets.push(gap);
                while start < bars.len() && bars[start].open_time_ms < gap.end_time_ms {
                    start += 1;
                }
                next_gap += 1;
            }
        }
        Ok(result)
    }
}

fn execute_segment(
    request: &RunRequest<'_>,
    bars: &[OhlcvBar],
    result: &mut RunResult,
) -> Result<(), RunError> {
    let strategy = load_strategy(request)
        .map_err(|error| RunError::host("strategy-load-failed", RunStage::Strategy, error))?;
    let feature_rows = evaluate_feature_rows(request, bars)?;
    let mut frames = Vec::with_capacity(bars.len());
    for (bar, feature) in bars.iter().zip(feature_rows) {
        match feature {
            MaterializedFeatureRow::Warmup => result.pauses.push(RunPause {
                open_time_ms: bar.open_time_ms,
                reason: RunPauseReason::Warmup,
            }),
            MaterializedFeatureRow::MissingInput { slot, source } => {
                result.pauses.push(RunPause {
                    open_time_ms: bar.open_time_ms,
                    reason: RunPauseReason::MissingInput { slot, source },
                });
            }
            MaterializedFeatureRow::Present(values) => {
                frames.push(strategy_abi::exports::adaq::strategy::api::FeatureFrame {
                    open_time_ms: bar.open_time_ms,
                    values,
                });
            }
        }
    }
    for frames in frames.chunks(4096) {
        let targets = strategy
            .process_strategy(frames.to_vec())
            .map_err(|error| {
                RunError::host("strategy-guest-error", RunStage::Strategy, error)
                    .at_bar(frames[0].open_time_ms)
            })?;
        if targets.len() != frames.len() {
            return Err(RunError::host(
                "invalid-strategy-output-count",
                RunStage::Strategy,
                "Strategy must return exactly one Target Exposure per Feature Frame",
            )
            .at_bar(frames[0].open_time_ms));
        }
        for (frame, target) in frames.iter().zip(targets) {
            result.decisions.push(TargetDecision {
                open_time_ms: frame.open_time_ms,
                target_exposure: validate_target(&target, request.position_mode).map_err(
                    |error| {
                        RunError::host("invalid-target-exposure", RunStage::Strategy, error)
                            .at_bar(frame.open_time_ms)
                    },
                )?,
            });
        }
    }
    Ok(())
}

fn normalize_closed_bars(bars: &[OhlcvBar]) -> Result<Vec<OhlcvBar>, RunError> {
    let mut normalized: Vec<OhlcvBar> = Vec::with_capacity(bars.len());
    for bar in bars {
        match normalized.last() {
            Some(previous) if bar.open_time_ms < previous.open_time_ms => {
                return Err(RunError::host(
                    "invalid-run-request",
                    RunStage::Validation,
                    "Closed Bars must be ascending",
                ));
            }
            Some(previous) if bar.open_time_ms == previous.open_time_ms && bar != previous => {
                return Err(RunError::host(
                    "conflicting-closed-bars",
                    RunStage::Validation,
                    "Closed Bars with the same open time conflict",
                )
                .at_bar(bar.open_time_ms));
            }
            Some(previous) if bar.open_time_ms == previous.open_time_ms => {}
            _ => normalized.push(bar.clone()),
        }
    }
    Ok(normalized)
}

fn evaluate_feature_rows(
    request: &RunRequest<'_>,
    bars: &[OhlcvBar],
) -> Result<Vec<MaterializedFeatureRow>, RunError> {
    let factor_values = evaluate_factors(request, bars)
        .map_err(|error| RunError::host("factor-evaluation-failed", RunStage::Factor, error))?;
    let feature_plan = request.plan.feature_plan();
    let mut evaluator = FeatureEngine::new(feature_plan.engine_identity())
        .evaluator(feature_plan.clone())
        .map_err(|error| feature_evaluation_error(error, None))?;
    let mut rows = Vec::with_capacity(bars.len());
    for (index, bar) in bars.iter().enumerate() {
        let dependencies = feature_dependencies(request, &factor_values, index, bar)?;
        let input = FeatureEvaluationInput::new(
            "component-run",
            bar.open_time_ms,
            bar.open_time_ms,
            FeatureMarketBar::from_ohlcv(bar.clone()),
        );
        let input = dependencies
            .into_iter()
            .fold(input, FeatureEvaluationInput::with_dependency);
        let observations = evaluator
            .observe(FeatureInputEvent::observation(input))
            .map_err(|error| feature_evaluation_error(error, Some(bar.open_time_ms)))?;
        rows.push(feature_row_from_observations(
            request.plan,
            &observations,
            bar.open_time_ms,
        )?);
    }
    Ok(rows)
}

fn feature_dependencies(
    request: &RunRequest<'_>,
    factor_values: &HashMap<String, Vec<Option<HashMap<String, f64>>>>,
    index: usize,
    bar: &OhlcvBar,
) -> Result<Vec<FeatureDependencyInput>, RunError> {
    let mut dependencies = Vec::new();
    for factor in request.plan.factors() {
        for output in factor.output_names {
            dependencies.push(FeatureDependencyInput::external(
                factor.alias,
                output.clone(),
                factor_values
                    .get(factor.alias)
                    .and_then(|rows| rows[index].as_ref())
                    .and_then(|row| row.get(output).copied()),
                bar.open_time_ms,
            ));
        }
    }
    for (slot, source) in request.plan.slot_names().zip(request.plan.sources()) {
        let FrozenSourceView::Signal {
            dataset_id,
            signal_name,
        } = source
        else {
            continue;
        };
        let Some(binding) = request.signals.iter().find(|binding| {
            binding.slot == slot
                && binding.dataset_id == dataset_id
                && binding.signal_name == signal_name
        }) else {
            dependencies.push(FeatureDependencyInput::signal(
                dataset_id,
                signal_name,
                None,
                bar.open_time_ms,
            ));
            continue;
        };
        let decision_time_ms =
            next_bar_open_time_ms(bar.open_time_ms, binding.interval).map_err(|error| {
                RunError::host(
                    "invalid-signal-decision-time",
                    RunStage::FeatureFrame,
                    error.to_string(),
                )
                .at_bar(bar.open_time_ms)
                .for_slot(slot, format!("signal:{dataset_id}:{signal_name}"))
            })?;
        let row = binding
            .rows
            .binary_search_by_key(&decision_time_ms, |row| row.prediction_time_ms)
            .ok()
            .map(|row_index| &binding.rows[row_index]);
        let (value, available_at_ms) = match row {
            Some(row) if row.available_at_ms <= decision_time_ms => {
                (row.value, row.available_at_ms)
            }
            _ => (None, decision_time_ms),
        };
        dependencies.push(FeatureDependencyInput::signal(
            dataset_id,
            signal_name,
            value,
            available_at_ms,
        ));
    }
    Ok(dependencies)
}

fn feature_row_from_observations(
    plan: &FrozenFeaturePlan,
    observations: &[FeatureObservation],
    bar_open_time_ms: i64,
) -> Result<MaterializedFeatureRow, RunError> {
    let mut slot_observations = Vec::with_capacity(plan.slot_names().len());
    for (slot, source) in plan.slot_names().zip(plan.sources()) {
        let observation = observations
            .iter()
            .find(|observation| observation.output_name == slot)
            .ok_or_else(|| {
                RunError::host(
                    "feature-output-missing",
                    RunStage::FeatureFrame,
                    "Feature evaluator did not return the planned Slot",
                )
                .at_bar(bar_open_time_ms)
                .for_slot(slot, source_name(source))
            })?;
        slot_observations.push((slot, source, observation));
    }
    // Warmup outranks missing inputs: while any Slot is still warming up the whole
    // frame stays in Warmup, matching the pre-unification whole-row semantics.
    if slot_observations.iter().any(|(_, _, observation)| {
        matches!(
            observation.value,
            FeatureObservationValue::Unavailable {
                reason: FeatureUnavailabilityReason::Warmup
            }
        )
    }) {
        return Ok(MaterializedFeatureRow::Warmup);
    }
    let mut values = Vec::with_capacity(slot_observations.len());
    for (slot, source, observation) in slot_observations {
        match observation.value {
            FeatureObservationValue::Available { value, .. } => values.push(value),
            FeatureObservationValue::Unavailable { reason } => {
                let source = source_name(source);
                let source = if reason == FeatureUnavailabilityReason::MissingDependency {
                    source
                } else {
                    format!("{source}:{}", reason.code())
                };
                return Ok(MaterializedFeatureRow::MissingInput {
                    slot: slot.to_owned(),
                    source,
                });
            }
        }
    }
    Ok(MaterializedFeatureRow::Present(values))
}

fn feature_evaluation_error(
    error: FeatureEvaluationError,
    fallback_time_ms: Option<i64>,
) -> RunError {
    RunError::host(
        "feature-evaluation-failed",
        RunStage::FeatureFrame,
        format!("{}: {}", error.code(), error.diagnostic),
    )
    .at_bar(
        error
            .observation_time_ms
            .or(fallback_time_ms)
            .unwrap_or_default(),
    )
    .for_slot(error.node_id.as_deref().unwrap_or("feature"), error.code())
}

fn evaluate_factors(
    request: &RunRequest<'_>,
    bars: &[OhlcvBar],
) -> Result<HashMap<String, Vec<Option<HashMap<String, f64>>>>, String> {
    let paths = request
        .factors
        .iter()
        .map(|factor| (factor.alias, factor.path))
        .collect::<HashMap<_, _>>();
    if paths.len() != request.factors.len()
        || request
            .plan
            .factors()
            .any(|factor| !paths.contains_key(factor.alias))
    {
        return Err("Factor Run bindings must match Frozen Plan aliases exactly".into());
    }
    if request.factors.iter().any(|factor| {
        !request
            .plan
            .factors()
            .any(|planned| planned.alias == factor.alias)
    }) {
        return Err("Factor Run bindings must match Frozen Plan aliases exactly".into());
    }
    let referenced = request
        .plan
        .sources()
        .filter_map(|source| match source {
            FrozenSourceView::External {
                dependency_alias,
                output,
            } => Some((dependency_alias, output)),
            FrozenSourceView::Market(_)
            | FrozenSourceView::BuiltIn { .. }
            | FrozenSourceView::Signal { .. } => None,
        })
        .fold(
            HashMap::<&str, HashSet<&str>>::new(),
            |mut outputs, (alias, output)| {
                outputs.entry(alias).or_default().insert(output);
                outputs
            },
        );
    let mut evaluated = HashMap::new();
    for factor in request.plan.factors() {
        let loader = WasmLoader::with_limits(request.limits);
        loader.load_factor_time_series_bytes(
            &std::fs::read(paths[factor.alias]).map_err(|error| error.to_string())?,
            factor
                .feature_slots
                .iter()
                .map(
                    |name| factor_abi::exports::adaq::factor::time_series_api::FeatureSlot {
                        name: name.clone(),
                    },
                )
                .collect(),
            factor.parameters,
        )?;
        let mut rows = Vec::with_capacity(bars.len());
        for chunk in bars.chunks(4096) {
            let input = chunk
                .iter()
                .map(|bar| factor_row(factor.feature_slots, bar))
                .collect::<Result<Vec<_>, _>>()?;
            let output = loader.process_factor(input).map_err(|error| {
                let (cause, _) = bounded_context(&error);
                format!("factor-guest-error:{}:{cause}", factor.alias)
            })?;
            if output.len() != chunk.len() {
                return Err(format!(
                    "Factor {} returned {} rows for {} Bars",
                    factor.alias,
                    output.len(),
                    chunk.len()
                ));
            }
            for (offset, row) in output.into_iter().enumerate() {
                let bar = &chunk[offset];
                if row.instrument_id != "component-run"
                    || row.observation_time_ms != bar.open_time_ms
                {
                    return Err(format!(
                        "Factor {} returned an invalid row identity at Bar {}",
                        factor.alias, bar.open_time_ms
                    ));
                }
                let row = row.values.map(|values| {
                    validate_factor_row(
                        factor.alias,
                        factor.output_names,
                        values,
                        bar.open_time_ms,
                        referenced
                            .get(factor.alias)
                            .into_iter()
                            .flat_map(|outputs| outputs.iter().copied()),
                    )
                });
                rows.push(row.transpose()?);
            }
        }
        evaluated.insert(factor.alias.to_owned(), rows);
    }
    Ok(evaluated)
}

fn bounded_context(value: &str) -> (String, bool) {
    if value.len() <= MAX_GUEST_CAUSE_BYTES {
        return (value.into(), false);
    }
    let end = value.floor_char_boundary(MAX_GUEST_CAUSE_BYTES);
    (value[..end].into(), true)
}

fn factor_row(
    feature_slots: &[String],
    bar: &OhlcvBar,
) -> Result<factor_abi::exports::adaq::factor::time_series_api::TimeSeriesRow, String> {
    let slots = feature_slots
        .iter()
        .map(|slot| {
            let value = match slot.as_str() {
                "open" => bar.open,
                "high" => bar.high,
                "low" => bar.low,
                "close" => bar.close,
                "base-volume" => bar.base_volume,
                "quote-volume" => bar.quote_volume,
                other => return Err(format!("Factor Feature Slot has no host binding: {other}")),
            };
            let value = value
                .to_f64()
                .filter(|value| value.is_finite())
                .ok_or_else(|| format!("Factor Feature Slot is not finite: {slot}"))?;
            Ok(
                factor_abi::exports::adaq::factor::time_series_api::FeatureValue {
                    value,
                    available_at_ms: bar.open_time_ms,
                },
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(
        factor_abi::exports::adaq::factor::time_series_api::TimeSeriesRow {
            instrument_id: "component-run".into(),
            observation_time_ms: bar.open_time_ms,
            slots,
        },
    )
}

fn validate_factor_row<'a>(
    alias: &str,
    expected: &[String],
    values: Vec<factor_abi::exports::adaq::factor::time_series_api::NamedScalar>,
    open_time_ms: i64,
    retained: impl Iterator<Item = &'a str>,
) -> Result<HashMap<String, f64>, String> {
    if values.len() != expected.len()
        || values
            .iter()
            .zip(expected)
            .any(|(value, name)| value.name != *name || !value.value.is_finite())
    {
        return Err(format!(
            "Factor {alias} returned an invalid row at Bar {open_time_ms}"
        ));
    }
    let retained = retained.collect::<HashSet<_>>();
    Ok(values
        .into_iter()
        .filter(|value| retained.contains(value.name.as_str()))
        .map(|value| (value.name, value.value))
        .collect())
}

fn load_strategy(request: &RunRequest<'_>) -> Result<WasmLoader, String> {
    let strategy = WasmLoader::with_limits(request.limits);
    strategy.load_strategy_with_parameters(
        request.strategy_path,
        request
            .plan
            .slot_names()
            .map(
                |name| strategy_abi::exports::adaq::strategy::api::FeatureSlot {
                    name: name.to_owned(),
                },
            )
            .collect(),
        request.strategy_parameters,
    )?;
    Ok(strategy)
}

fn source_name(source: FrozenSourceView<'_>) -> String {
    match source {
        FrozenSourceView::Market(field) => format!("market:{field:?}"),
        FrozenSourceView::External {
            dependency_alias,
            output,
        } => format!("external:{dependency_alias}:{output}"),
        FrozenSourceView::BuiltIn {
            indicator, output, ..
        } => format!("builtin:{indicator}:{output}"),
        FrozenSourceView::Signal {
            dataset_id,
            signal_name,
        } => format!("signal:{dataset_id}:{signal_name}"),
    }
}

fn validate_request(request: &RunRequest<'_>) -> Result<(), String> {
    validate_bar_count(request.bars.len(), request.limits.max_bars)?;
    if request.limits.fuel_per_call == 0 || request.limits.memory_bytes == 0 {
        return Err("Run limits must be greater than zero".into());
    }
    if request
        .bars
        .windows(2)
        .any(|bars| bars[0].open_time_ms >= bars[1].open_time_ms)
    {
        return Err("Closed Bars must be strictly ascending".into());
    }
    if request
        .gaps
        .iter()
        .any(|gap| gap.start_time_ms >= gap.end_time_ms)
        || request
            .gaps
            .windows(2)
            .any(|gaps| gaps[0].end_time_ms > gaps[1].start_time_ms)
    {
        return Err("Bar Gaps must be valid, ascending, and non-overlapping".into());
    }
    if request.gaps.iter().any(|gap| {
        request
            .bars
            .iter()
            .any(|bar| bar.open_time_ms >= gap.start_time_ms && bar.open_time_ms < gap.end_time_ms)
    }) {
        return Err("Closed Bars cannot fall inside a Bar Gap".into());
    }
    let mut slots = HashSet::new();
    if request.plan.slot_names().any(|name| !slots.insert(name)) {
        return Err("Frozen Feature Plan contains duplicate Feature Slots".into());
    }
    if request.signals.iter().any(|binding| {
        binding
            .rows
            .windows(2)
            .any(|rows| rows[0].prediction_time_ms >= rows[1].prediction_time_ms)
            || !request
                .plan
                .slot_names()
                .zip(request.plan.sources())
                .any(|(slot, source)| {
                    matches!(
                        source,
                        FrozenSourceView::Signal { dataset_id, signal_name }
                            if slot == binding.slot
                                && dataset_id == binding.dataset_id
                                && signal_name == binding.signal_name
                    )
                })
    }) || request
        .plan
        .slot_names()
        .zip(request.plan.sources())
        .filter(|(_, source)| matches!(source, FrozenSourceView::Signal { .. }))
        .count()
        != request.signals.len()
    {
        return Err("Signal Run bindings must match Frozen Plan slots exactly".into());
    }
    Ok(())
}

fn validate_bar_count(bar_count: usize, requested_limit: usize) -> Result<(), String> {
    let limit = requested_limit.min(MAX_CLOSED_BARS);
    if bar_count > limit {
        return Err(format!(
            "Run contains {} bars, exceeding the limit of {}",
            bar_count, limit
        ));
    }
    Ok(())
}

fn validate_target(raw: &str, mode: PositionMode) -> Result<Decimal, String> {
    let target = Decimal::from_str_exact(raw)
        .map_err(|error| format!("Invalid Target Exposure: {error}"))?;
    let minimum = match mode {
        PositionMode::LongOnly => Decimal::ZERO,
        #[cfg(test)]
        PositionMode::LongShort => -Decimal::ONE,
    };
    if target < minimum || target > Decimal::ONE {
        return Err(format!("Target Exposure {target} is outside [{minimum},1]"));
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, path::PathBuf, str::FromStr};

    use adaq_component_tooling::{
        BuiltinForecastTarget, ComponentManifest, FactorInstancePlanInput, ForecastTarget,
        ForecastValueScale, ModelOutput, PredictionKind, SignalPlanInput, native_engine_identity,
        validate_and_freeze_feature_plan,
        validate_and_freeze_feature_plan_with_bindings_and_parameters,
        validate_and_freeze_feature_plan_with_factors,
    };
    use adaq_indicator_engine::IndicatorEngine;

    use super::*;

    fn fixture() -> String {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/strategy/target/wasm32-unknown-unknown/debug/m1_strategy_fixture.wasm")
            .to_string_lossy()
            .into_owned()
    }

    fn external_strategy_fixture() -> String {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/external-strategy/target/wasm32-unknown-unknown/debug/m5_external_strategy_fixture.wasm")
            .to_string_lossy()
            .into_owned()
    }

    fn mixed_strategy_fixture() -> String {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/mixed-strategy/target/wasm32-unknown-unknown/debug/m5_mixed_strategy_fixture.wasm")
            .to_string_lossy()
            .into_owned()
    }

    fn factor_fixture() -> String {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/factor/target/wasm32-unknown-unknown/debug/m1_factor_fixture.wasm")
            .to_string_lossy()
            .into_owned()
    }

    fn bar(open_time_ms: i64, close: &str, quote_volume: &str) -> OhlcvBar {
        let close = Decimal::from_str(close).unwrap();
        OhlcvBar {
            open_time_ms,
            open: close,
            high: close,
            low: close,
            close,
            base_volume: Decimal::ONE,
            quote_volume: Decimal::from_str(quote_volume).unwrap(),
        }
    }

    fn plan() -> FrozenFeaturePlan {
        let manifest = serde_json::from_str::<ComponentManifest>(
            r#"{
            "manifestSchemaVersion":"1.0.0",
            "componentId":"00000000-0000-0000-0000-000000000000",
            "version":"1.0.0",
            "name":"Market Fixture",
            "kind":"strategy",
            "sdkVersion":"0.1.0",
            "abiVersion":"1.0.0",
            "featureSlots":[
                {"name":"quote-volume","source":{"kind":"market","field":"quote-volume"}},
                {"name":"close","source":{"kind":"market","field":"close"}}
            ]
        }"#,
        )
        .unwrap();
        validate_and_freeze_feature_plan(
            &manifest,
            &"a".repeat(64),
            &native_engine_identity().unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn market_plan_preserves_slot_order_and_target_alignment() {
        let plan = plan();
        let strategy = fixture();
        let bars = vec![bar(1, "10", "5"), bar(2, "11", "20")];
        let request = RunRequest {
            strategy_path: &strategy,
            strategy_parameters: &[],
            factors: &[],
            bars: &bars,
            signals: &[],
            gaps: &[],
            plan: &plan,
            position_mode: PositionMode::LongOnly,
            limits: RunLimits::default(),
        };
        let result = RunEngine::execute(&request).unwrap();
        let replay = RunEngine::execute(&request).unwrap();
        assert_eq!(result, replay);
        assert_eq!(result.plan_hash, plan.plan_hash());
        assert_eq!(
            result.decisions,
            [
                TargetDecision {
                    open_time_ms: 1,
                    target_exposure: Decimal::ONE,
                },
                TargetDecision {
                    open_time_ms: 2,
                    target_exposure: Decimal::ZERO,
                },
            ]
        );
    }

    #[test]
    fn signal_rows_require_exact_alignment_and_availability_without_synthetic_values() {
        let contract = |name: &str| ModelOutput {
            name: name.into(),
            prediction_kind: PredictionKind::Probability,
            forecast_target: ForecastTarget::Builtin {
                target: BuiltinForecastTarget::FutureCloseUp,
            },
            value_scale: ForecastValueScale::Probability,
            horizon_bars: 1,
        };
        let manifest: ComponentManifest = serde_json::from_value(serde_json::json!({
            "manifestSchemaVersion": "1.0.0",
            "componentId": "00000000-0000-4000-8000-000000000001",
            "version": "1.0.0",
            "name": "Signal Fixture",
            "kind": "strategy",
            "sdkVersion": "0.1.0",
            "abiVersion": "1.0.0",
            "featureSlots": [{
                "name": "quote-volume",
                "source": {
                    "kind": "signal",
                    "predictionKind": {"kind": "probability"},
                    "forecastTarget": {"kind": "builtin", "target": "future-close-up"},
                    "valueScale": {"kind": "probability"},
                    "horizonBars": 1
                }
            }, {
                "name": "close",
                "source": {
                    "kind": "signal",
                    "predictionKind": {"kind": "probability"},
                    "forecastTarget": {"kind": "builtin", "target": "future-close-up"},
                    "valueScale": {"kind": "probability"},
                    "horizonBars": 1
                }
            }]
        }))
        .unwrap();
        let quote_dataset_id = "b".repeat(64);
        let close_dataset_id = "c".repeat(64);
        let mut signal_inputs = [
            SignalPlanInput {
                slot_name: "quote-volume",
                dataset_id: &quote_dataset_id,
                signal_name: "quote-volume",
                snapshot_id: "snapshot",
                instrument_id: "okx:BTC-USDT".into(),
                venue: "okx",
                bar_interval: "1m",
                contract: contract("quote-volume"),
                producer_segments: vec![serde_json::json!({"segment": 1})],
                artifact_provenance: serde_json::json!({"sha256": "artifact"}),
                evidence_state: "unknown",
                component_lock: vec![],
            },
            SignalPlanInput {
                slot_name: "close",
                dataset_id: &close_dataset_id,
                signal_name: "close",
                snapshot_id: "snapshot",
                instrument_id: "okx:BTC-USDT".into(),
                venue: "okx",
                bar_interval: "1m",
                contract: contract("close"),
                producer_segments: vec![serde_json::json!({"segment": 1})],
                artifact_provenance: serde_json::json!({"sha256": "artifact"}),
                evidence_state: "unknown",
                component_lock: vec![],
            },
        ];
        let plan = validate_and_freeze_feature_plan_with_bindings_and_parameters(
            &manifest,
            &"a".repeat(64),
            &native_engine_identity().unwrap(),
            &[],
            &BTreeMap::new(),
            &signal_inputs,
        )
        .unwrap();
        assert_eq!(
            plan.architecture(),
            adaq_component_tooling::StrategyArchitecture::SignalDriven
        );
        let frozen_json: serde_json::Value = serde_json::from_slice(&plan.to_json()).unwrap();
        let frozen_source = &frozen_json["slots"][0]["source"];
        for evidence in [
            "dataset_id",
            "signal_name",
            "snapshot_id",
            "producer_segments",
            "artifact_provenance",
            "evidence_state",
            "component_lock",
        ] {
            assert!(frozen_source.get(evidence).is_some(), "missing {evidence}");
        }
        let alternate_dataset_id = "d".repeat(64);
        signal_inputs[0].dataset_id = &alternate_dataset_id;
        let alternate = validate_and_freeze_feature_plan_with_bindings_and_parameters(
            &manifest,
            &"a".repeat(64),
            &native_engine_identity().unwrap(),
            &[],
            &BTreeMap::new(),
            &signal_inputs,
        )
        .unwrap();
        assert_ne!(plan.plan_hash(), alternate.plan_hash());
        signal_inputs[0].dataset_id = &quote_dataset_id;
        let quote_rows = [
            SignalRunRow {
                prediction_time_ms: 60_000,
                available_at_ms: 60_000,
                value: Some(0.2),
            },
            SignalRunRow {
                prediction_time_ms: 120_000,
                available_at_ms: 120_000,
                value: Some(0.8),
            },
        ];
        let close_rows = [
            SignalRunRow {
                prediction_time_ms: 60_000,
                available_at_ms: 60_000,
                value: Some(0.9),
            },
            SignalRunRow {
                prediction_time_ms: 120_000,
                available_at_ms: 120_001,
                value: Some(0.1),
            },
        ];
        let signals = [
            SignalRunRequest {
                slot: "quote-volume",
                dataset_id: signal_inputs[0].dataset_id,
                signal_name: "quote-volume",
                interval: BarInterval::OneMinute,
                rows: &quote_rows,
            },
            SignalRunRequest {
                slot: "close",
                dataset_id: signal_inputs[1].dataset_id,
                signal_name: "close",
                interval: BarInterval::OneMinute,
                rows: &close_rows,
            },
        ];
        let bars = [bar(0, "10", "5"), bar(60_000, "11", "20")];
        let result = RunEngine::execute(&RunRequest {
            strategy_path: &fixture(),
            strategy_parameters: &[],
            factors: &[],
            signals: &signals,
            bars: &bars,
            gaps: &[],
            plan: &plan,
            position_mode: PositionMode::LongOnly,
            limits: RunLimits::default(),
        })
        .unwrap();
        assert_eq!(result.decisions.len(), 1);
        assert_eq!(result.decisions[0].open_time_ms, 0);
        assert_eq!(
            result.pauses,
            [RunPause {
                open_time_ms: 60_000,
                reason: RunPauseReason::MissingInput {
                    slot: "close".into(),
                    source: format!("signal:{}:close", signal_inputs[1].dataset_id),
                },
            }]
        );
    }

    #[test]
    fn warmup_outranks_missing_signal_input_in_feature_rows() {
        let contract = |name: &str| ModelOutput {
            name: name.into(),
            prediction_kind: PredictionKind::Probability,
            forecast_target: ForecastTarget::Builtin {
                target: BuiltinForecastTarget::FutureCloseUp,
            },
            value_scale: ForecastValueScale::Probability,
            horizon_bars: 1,
        };
        let manifest: ComponentManifest = serde_json::from_value(serde_json::json!({
            "manifestSchemaVersion": "1.0.0",
            "componentId": "00000000-0000-4000-8000-000000000002",
            "version": "1.0.0",
            "name": "Warmup Signal Fixture",
            "kind": "strategy",
            "sdkVersion": "0.1.0",
            "abiVersion": "1.0.0",
            "featureSlots": [{
                "name": "quote-volume",
                "source": {
                    "kind": "signal",
                    "predictionKind": {"kind": "probability"},
                    "forecastTarget": {"kind": "builtin", "target": "future-close-up"},
                    "valueScale": {"kind": "probability"},
                    "horizonBars": 1
                }
            }, {
                "name": "rsi",
                "source": {
                    "kind": "builtin",
                    "indicator": "rsi",
                    "output": "value",
                    "inputs": {"real-0": "close"},
                    "parameters": {"time-period": 2}
                }
            }]
        }))
        .unwrap();
        let dataset_id = "b".repeat(64);
        let signal_inputs = [SignalPlanInput {
            slot_name: "quote-volume",
            dataset_id: &dataset_id,
            signal_name: "quote-volume",
            snapshot_id: "snapshot",
            instrument_id: "okx:BTC-USDT".into(),
            venue: "okx",
            bar_interval: "1m",
            contract: contract("quote-volume"),
            producer_segments: vec![serde_json::json!({"segment": 1})],
            artifact_provenance: serde_json::json!({"sha256": "artifact"}),
            evidence_state: "unknown",
            component_lock: vec![],
        }];
        let plan = validate_and_freeze_feature_plan_with_bindings_and_parameters(
            &manifest,
            &"a".repeat(64),
            &native_engine_identity().unwrap(),
            &[],
            &BTreeMap::new(),
            &signal_inputs,
        )
        .unwrap();
        let signals = [SignalRunRequest {
            slot: "quote-volume",
            dataset_id: &dataset_id,
            signal_name: "quote-volume",
            interval: BarInterval::OneMinute,
            rows: &[],
        }];
        let bars = (0..5)
            .map(|index| bar(index, &(10 + index).to_string(), "5"))
            .collect::<Vec<_>>();
        let request = RunRequest {
            strategy_path: &fixture(),
            strategy_parameters: &[],
            factors: &[],
            signals: &signals,
            bars: &bars,
            gaps: &[],
            plan: &plan,
            position_mode: PositionMode::LongOnly,
            limits: RunLimits::default(),
        };
        let rows = evaluate_feature_rows(&request, &bars).unwrap();
        assert_eq!(rows[0], MaterializedFeatureRow::Warmup);
        assert_eq!(rows[1], MaterializedFeatureRow::Warmup);
        assert!(matches!(
            rows[2],
            MaterializedFeatureRow::MissingInput { .. }
        ));
    }

    #[test]
    fn identical_closed_bars_are_collapsed_but_conflicts_fail() {
        let plan = plan();
        let strategy = fixture();
        let first = bar(1, "10", "5");
        let request = RunRequest {
            strategy_path: &strategy,
            strategy_parameters: &[],
            factors: &[],
            bars: &[first.clone(), first.clone(), bar(2, "11", "20")],
            signals: &[],
            gaps: &[],
            plan: &plan,
            position_mode: PositionMode::LongOnly,
            limits: RunLimits::default(),
        };
        assert_eq!(
            RunEngine::execute(&request).unwrap().decisions,
            [
                TargetDecision {
                    open_time_ms: 1,
                    target_exposure: Decimal::ONE,
                },
                TargetDecision {
                    open_time_ms: 2,
                    target_exposure: Decimal::ZERO,
                },
            ]
        );
        assert_eq!(RunEngine::execute(&request).unwrap().bars.len(), 2);

        let conflicting = bar(1, "12", "5");
        let error = RunEngine::execute(&RunRequest {
            bars: &[first, conflicting],
            ..request
        })
        .unwrap_err();
        assert_eq!(error.code, "conflicting-closed-bars");
        assert_eq!(error.stage, RunStage::Validation);
    }

    #[test]
    fn frozen_plan_uses_the_native_indicator_engine_build_identity() {
        let identity = native_engine_identity().unwrap();
        let engine = IndicatorEngine::initialize().unwrap();
        let native = engine.identity();
        assert_eq!(identity.engine_build_id, native.build_id);
        assert_eq!(identity.ta_source_sha256, native.ta_source_sha256);
        assert_eq!(identity.catalog_version, native.catalog_version);
    }

    #[test]
    fn position_modes_enforce_exact_decimal_ranges() {
        assert_eq!(
            validate_target("-0.5", PositionMode::LongShort).unwrap(),
            Decimal::new(-5, 1)
        );
        assert!(validate_target("-0.5", PositionMode::LongOnly).is_err());
        assert!(validate_target("1.0000000001", PositionMode::LongShort).is_err());
        assert!(validate_target("NaN", PositionMode::LongShort).is_err());
    }

    #[test]
    fn external_factor_values_pause_then_align_target_decisions() {
        let strategy_manifest = serde_json::from_str::<ComponentManifest>(include_str!(
            "../fixtures/external-strategy/manifest.json"
        ))
        .unwrap();
        let factor_manifest = serde_json::from_str::<ComponentManifest>(include_str!(
            "../fixtures/factor/manifest.json"
        ))
        .unwrap();
        let factor_parameters = vec![ComponentParameterValue::Integer(1)];
        let plan = validate_and_freeze_feature_plan_with_factors(
            &strategy_manifest,
            &"a".repeat(64),
            &native_engine_identity().unwrap(),
            &[FactorInstancePlanInput {
                alias: "change",
                manifest: &factor_manifest,
                parameters: factor_parameters,
            }],
        )
        .unwrap();
        let strategy = external_strategy_fixture();
        let factor = factor_fixture();
        let factors = [FactorRunRequest {
            alias: "change",
            path: &factor,
        }];
        let bars = vec![bar(1, "10", "1"), bar(2, "11", "1"), bar(3, "9", "1")];
        let features =
            materialize_feature_segment(&plan, &factors, &bars, RunLimits::default()).unwrap();
        assert_eq!(features[0], MaterializedFeatureRow::Warmup);
        assert!(matches!(features[1], MaterializedFeatureRow::Present(_)));
        assert!(matches!(features[2], MaterializedFeatureRow::Present(_)));
        let result = RunEngine::execute(&RunRequest {
            strategy_path: &strategy,
            strategy_parameters: &[],
            factors: &factors,
            bars: &bars,
            signals: &[],
            gaps: &[],
            plan: &plan,
            position_mode: PositionMode::LongOnly,
            limits: RunLimits::default(),
        })
        .unwrap();
        assert_eq!(
            result.pauses,
            [RunPause {
                open_time_ms: 1,
                reason: RunPauseReason::Warmup
            }]
        );
        assert_eq!(
            result.decisions,
            [
                TargetDecision {
                    open_time_ms: 2,
                    target_exposure: Decimal::ONE
                },
                TargetDecision {
                    open_time_ms: 3,
                    target_exposure: Decimal::ZERO
                },
            ]
        );
    }

    #[test]
    fn post_warmup_missing_factor_input_pauses_without_advancing_strategy() {
        let strategy_manifest = serde_json::from_str::<ComponentManifest>(include_str!(
            "../fixtures/external-strategy/manifest.json"
        ))
        .unwrap();
        let factor_manifest = serde_json::from_str::<ComponentManifest>(include_str!(
            "../fixtures/factor/manifest.json"
        ))
        .unwrap();
        let plan = validate_and_freeze_feature_plan_with_factors(
            &strategy_manifest,
            &"a".repeat(64),
            &native_engine_identity().unwrap(),
            &[FactorInstancePlanInput {
                alias: "change",
                manifest: &factor_manifest,
                parameters: vec![ComponentParameterValue::Integer(1)],
            }],
        )
        .unwrap();
        let strategy = external_strategy_fixture();
        let factor = factor_fixture();
        let factors = [FactorRunRequest {
            alias: "change",
            path: &factor,
        }];
        let bars = vec![
            bar(1, "10", "1"),
            bar(2, "11", "1"),
            bar(3, "0", "1"),
            bar(4, "2", "1"),
        ];
        let result = RunEngine::execute(&RunRequest {
            strategy_path: &strategy,
            strategy_parameters: &[],
            factors: &factors,
            bars: &bars,
            gaps: &[],
            signals: &[],
            plan: &plan,
            position_mode: PositionMode::LongOnly,
            limits: RunLimits::default(),
        })
        .unwrap();
        assert_eq!(
            result.pauses,
            [
                RunPause {
                    open_time_ms: 1,
                    reason: RunPauseReason::Warmup,
                },
                RunPause {
                    open_time_ms: 3,
                    reason: RunPauseReason::MissingInput {
                        slot: "close-change".into(),
                        source: "external:change:close-change".into(),
                    },
                },
            ]
        );
        assert_eq!(
            result
                .decisions
                .iter()
                .map(|decision| decision.open_time_ms)
                .collect::<Vec<_>>(),
            [2, 4]
        );
    }

    #[test]
    fn fatal_factor_output_fails_without_a_partial_run_result() {
        let strategy_manifest = serde_json::from_str::<ComponentManifest>(include_str!(
            "../fixtures/external-strategy/manifest.json"
        ))
        .unwrap();
        let factor_manifest = serde_json::from_str::<ComponentManifest>(include_str!(
            "../fixtures/factor/manifest.json"
        ))
        .unwrap();
        let plan = validate_and_freeze_feature_plan_with_factors(
            &strategy_manifest,
            &"a".repeat(64),
            &native_engine_identity().unwrap(),
            &[FactorInstancePlanInput {
                alias: "change",
                manifest: &factor_manifest,
                parameters: vec![ComponentParameterValue::Integer(1)],
            }],
        )
        .unwrap();
        let strategy = external_strategy_fixture();
        let factor = factor_fixture();
        let factors = [FactorRunRequest {
            alias: "change",
            path: &factor,
        }];
        let mut fatal = bar(2, "11", "1");
        fatal.base_volume = Decimal::ZERO;
        let error = RunEngine::execute(&RunRequest {
            strategy_path: &strategy,
            strategy_parameters: &[],
            factors: &factors,
            bars: &[bar(1, "10", "1"), fatal],
            signals: &[],
            gaps: &[],
            plan: &plan,
            position_mode: PositionMode::LongOnly,
            limits: RunLimits::default(),
        })
        .unwrap_err();
        assert_eq!(error.code, "factor-evaluation-failed");
        assert_eq!(error.stage, RunStage::Factor);
    }

    #[test]
    fn mixed_slots_restart_warmup_and_component_state_at_each_gap() {
        let strategy_manifest = serde_json::from_str::<ComponentManifest>(include_str!(
            "../fixtures/mixed-strategy/manifest.json"
        ))
        .unwrap();
        let factor_manifest = serde_json::from_str::<ComponentManifest>(include_str!(
            "../fixtures/factor/manifest.json"
        ))
        .unwrap();
        let plan = validate_and_freeze_feature_plan_with_factors(
            &strategy_manifest,
            &"a".repeat(64),
            &native_engine_identity().unwrap(),
            &[FactorInstancePlanInput {
                alias: "change",
                manifest: &factor_manifest,
                parameters: vec![ComponentParameterValue::Integer(1)],
            }],
        )
        .unwrap();
        assert_eq!(plan.effective_warmup_bars(), 1);
        assert_eq!(
            plan.slot_names().collect::<Vec<_>>(),
            ["close-change", "ema", "quote-volume"]
        );
        let strategy = mixed_strategy_fixture();
        let factor = factor_fixture();
        let factors = [FactorRunRequest {
            alias: "change",
            path: &factor,
        }];
        let bars = vec![
            bar(1, "10", "1"),
            bar(2, "11", "1"),
            bar(3, "12", "1"),
            bar(6, "20", "1"),
            bar(7, "21", "1"),
            bar(8, "22", "1"),
            bar(11, "30", "1"),
            bar(12, "31", "1"),
            bar(13, "32", "1"),
        ];
        let gap = BarGap {
            start_time_ms: 4,
            end_time_ms: 6,
        };
        let second_gap = BarGap {
            start_time_ms: 9,
            end_time_ms: 11,
        };
        let result = RunEngine::execute(&RunRequest {
            strategy_path: &strategy,
            strategy_parameters: &[],
            factors: &factors,
            bars: &bars,
            gaps: &[gap, second_gap],
            signals: &[],
            plan: &plan,
            position_mode: PositionMode::LongOnly,
            limits: RunLimits::default(),
        })
        .unwrap();
        assert_eq!(result.gap_resets, [gap, second_gap]);
        assert_eq!(
            result
                .pauses
                .iter()
                .map(|pause| pause.open_time_ms)
                .collect::<Vec<_>>(),
            [1, 6, 11]
        );
        assert_eq!(
            result.decisions,
            [
                TargetDecision {
                    open_time_ms: 2,
                    target_exposure: Decimal::ONE,
                },
                TargetDecision {
                    open_time_ms: 3,
                    target_exposure: Decimal::ZERO,
                },
                TargetDecision {
                    open_time_ms: 7,
                    target_exposure: Decimal::ONE,
                },
                TargetDecision {
                    open_time_ms: 8,
                    target_exposure: Decimal::ZERO,
                },
                TargetDecision {
                    open_time_ms: 12,
                    target_exposure: Decimal::ONE,
                },
                TargetDecision {
                    open_time_ms: 13,
                    target_exposure: Decimal::ZERO,
                },
            ]
        );
    }

    #[test]
    fn fatal_strategy_failure_cannot_return_a_partial_run_result() {
        let strategy_manifest = serde_json::from_str::<ComponentManifest>(include_str!(
            "../fixtures/mixed-strategy/manifest.json"
        ))
        .unwrap();
        let factor_manifest = serde_json::from_str::<ComponentManifest>(include_str!(
            "../fixtures/factor/manifest.json"
        ))
        .unwrap();
        let plan = validate_and_freeze_feature_plan_with_factors(
            &strategy_manifest,
            &"a".repeat(64),
            &native_engine_identity().unwrap(),
            &[FactorInstancePlanInput {
                alias: "change",
                manifest: &factor_manifest,
                parameters: vec![ComponentParameterValue::Integer(1)],
            }],
        )
        .unwrap();
        let strategy = mixed_strategy_fixture();
        let factor = factor_fixture();
        let factors = [FactorRunRequest {
            alias: "change",
            path: &factor,
        }];
        let error = RunEngine::execute(&RunRequest {
            strategy_path: &strategy,
            strategy_parameters: &[],
            factors: &factors,
            bars: &[bar(1, "10", "1"), bar(2, "11", "0")],
            signals: &[],
            gaps: &[],
            plan: &plan,
            position_mode: PositionMode::LongOnly,
            limits: RunLimits::default(),
        })
        .unwrap_err();
        assert_eq!(error.code, "strategy-guest-error");
        assert_eq!(error.stage, RunStage::Strategy);
    }

    #[test]
    fn factor_and_strategy_chunk_boundaries_preserve_closed_bar_alignment() {
        let strategy_manifest = serde_json::from_str::<ComponentManifest>(include_str!(
            "../fixtures/external-strategy/manifest.json"
        ))
        .unwrap();
        let factor_manifest = serde_json::from_str::<ComponentManifest>(include_str!(
            "../fixtures/factor/manifest.json"
        ))
        .unwrap();
        let plan = validate_and_freeze_feature_plan_with_factors(
            &strategy_manifest,
            &"a".repeat(64),
            &native_engine_identity().unwrap(),
            &[FactorInstancePlanInput {
                alias: "change",
                manifest: &factor_manifest,
                parameters: vec![ComponentParameterValue::Integer(1)],
            }],
        )
        .unwrap();
        let strategy = external_strategy_fixture();
        let factor = factor_fixture();
        let factors = [FactorRunRequest {
            alias: "change",
            path: &factor,
        }];
        let bars = (0..4098)
            .map(|time| bar(time, &(time + 1).to_string(), "1"))
            .collect::<Vec<_>>();
        let result = RunEngine::execute(&RunRequest {
            strategy_path: &strategy,
            strategy_parameters: &[],
            factors: &factors,
            bars: &bars,
            gaps: &[],
            signals: &[],
            plan: &plan,
            position_mode: PositionMode::LongOnly,
            limits: RunLimits {
                fuel_per_call: 100_000_000,
                memory_bytes: 256 * 1024 * 1024,
                ..RunLimits::default()
            },
        })
        .unwrap();
        assert_eq!(result.pauses.len(), 1);
        assert_eq!(result.decisions.len(), 4097);
        assert_eq!(result.decisions.first().unwrap().open_time_ms, 1);
        assert_eq!(result.decisions.last().unwrap().open_time_ms, 4097);
        assert!(
            result
                .decisions
                .iter()
                .all(|decision| decision.target_exposure == Decimal::ONE)
        );
    }

    #[test]
    fn builtin_ema_values_are_warmed_and_mapped_to_target_decisions() {
        let manifest = serde_json::from_str::<ComponentManifest>(r#"{
            "manifestSchemaVersion":"1.0.0","componentId":"00000000-0000-0000-0000-000000000000",
            "version":"1.0.0","name":"Built-in Fixture","kind":"strategy","sdkVersion":"0.1.0","abiVersion":"1.0.0",
            "featureSlots":[
                {"name":"quote-volume","source":{"kind":"market","field":"quote-volume"}},
                {"name":"close","source":{"kind":"builtin","indicator":"ema","output":"value","inputs":{"real-0":"close"},"parameters":{"time-period":1}}}
            ]
        }"#).unwrap();
        let plan = validate_and_freeze_feature_plan(
            &manifest,
            &"a".repeat(64),
            &native_engine_identity().unwrap(),
        )
        .unwrap();
        assert_eq!(plan.effective_warmup_bars(), 0);
        let bars = vec![bar(1, "10", "1"), bar(2, "11", "20")];
        let strategy = fixture();
        let result = RunEngine::execute(&RunRequest {
            strategy_path: &strategy,
            strategy_parameters: &[],
            factors: &[],
            bars: &bars,
            gaps: &[],
            signals: &[],
            plan: &plan,
            position_mode: PositionMode::LongOnly,
            limits: RunLimits::default(),
        })
        .unwrap();
        assert_eq!(
            result
                .decisions
                .iter()
                .map(|item| item.target_exposure)
                .collect::<Vec<_>>(),
            [Decimal::ONE, Decimal::ZERO]
        );
    }

    #[test]
    fn multi_output_builtin_slots_use_the_shared_feature_evaluator() {
        let manifest = serde_json::from_str::<ComponentManifest>(r#"{
            "manifestSchemaVersion":"1.0.0","componentId":"00000000-0000-0000-0000-000000000000",
            "version":"1.0.0","name":"MACD Fixture","kind":"strategy","sdkVersion":"0.1.0","abiVersion":"1.0.0",
            "featureSlots":[
                {"name":"macd","source":{"kind":"builtin","indicator":"macd","output":"macd","inputs":{"real-0":"close"}}},
                {"name":"signal","source":{"kind":"builtin","indicator":"macd","output":"signal","inputs":{"real-0":"close"}}},
                {"name":"histogram","source":{"kind":"builtin","indicator":"macd","output":"histogram","inputs":{"real-0":"close"}}}
            ]
        }"#).unwrap();
        let plan = validate_and_freeze_feature_plan(
            &manifest,
            &"a".repeat(64),
            &native_engine_identity().unwrap(),
        )
        .unwrap();
        let bars = (0..64)
            .map(|index| bar(index, &(100 + index).to_string(), "1"))
            .collect::<Vec<_>>();
        let request = RunRequest {
            strategy_path: &fixture(),
            strategy_parameters: &[],
            factors: &[],
            bars: &bars,
            gaps: &[],
            signals: &[],
            plan: &plan,
            position_mode: PositionMode::LongOnly,
            limits: RunLimits::default(),
        };
        let values = evaluate_feature_rows(&request, &bars).unwrap();
        assert!(values.iter().any(|row| {
            matches!(row, MaterializedFeatureRow::Present(values) if values.len() == 3)
        }));
    }

    #[test]
    fn rsi_warmup_and_target_decisions_align_to_closed_bars() {
        let manifest = serde_json::from_str::<ComponentManifest>(r#"{
            "manifestSchemaVersion":"1.0.0","componentId":"00000000-0000-0000-0000-000000000000",
            "version":"1.0.0","name":"RSI Fixture","kind":"strategy","sdkVersion":"0.1.0","abiVersion":"1.0.0",
            "featureSlots":[
                {"name":"quote-volume","source":{"kind":"market","field":"quote-volume"}},
                {"name":"close","source":{"kind":"builtin","indicator":"rsi","output":"value","inputs":{"real-0":"close"},"parameters":{"time-period":2}}}
            ]
        }"#).unwrap();
        let plan = validate_and_freeze_feature_plan(
            &manifest,
            &"a".repeat(64),
            &native_engine_identity().unwrap(),
        )
        .unwrap();
        let bars = ["1", "2", "3", "2", "1"]
            .into_iter()
            .enumerate()
            .map(|(index, close)| bar(index as i64, close, "50"))
            .collect::<Vec<_>>();
        let result = RunEngine::execute(&RunRequest {
            strategy_path: &fixture(),
            strategy_parameters: &[],
            factors: &[],
            bars: &bars,
            gaps: &[],
            signals: &[],
            plan: &plan,
            position_mode: PositionMode::LongOnly,
            limits: RunLimits::default(),
        })
        .unwrap();
        assert_eq!(
            result
                .pauses
                .iter()
                .map(|pause| pause.open_time_ms)
                .collect::<Vec<_>>(),
            [0, 1]
        );
        assert_eq!(
            result.decisions,
            [
                TargetDecision {
                    open_time_ms: 2,
                    target_exposure: Decimal::ONE
                },
                TargetDecision {
                    open_time_ms: 3,
                    target_exposure: Decimal::ZERO
                },
                TargetDecision {
                    open_time_ms: 4,
                    target_exposure: Decimal::ZERO
                },
            ]
        );
    }

    #[test]
    fn multi_output_slots_preserve_manifest_order_and_map_targets() {
        let manifest = serde_json::from_str::<ComponentManifest>(r#"{
            "manifestSchemaVersion":"1.0.0","componentId":"00000000-0000-0000-0000-000000000000",
            "version":"1.0.0","name":"MACD Target Fixture","kind":"strategy","sdkVersion":"0.1.0","abiVersion":"1.0.0",
            "featureSlots":[
                {"name":"histogram","source":{"kind":"builtin","indicator":"macd","output":"histogram","inputs":{"real-0":"close"},"parameters":{"fast-period":2,"slow-period":3,"signal-period":2}}},
                {"name":"quote-volume","source":{"kind":"builtin","indicator":"macd","output":"macd","inputs":{"real-0":"close"},"parameters":{"fast-period":2,"slow-period":3,"signal-period":2}}},
                {"name":"close","source":{"kind":"builtin","indicator":"macd","output":"signal","inputs":{"real-0":"close"},"parameters":{"fast-period":2,"slow-period":3,"signal-period":2}}}
            ]
        }"#).unwrap();
        let plan = validate_and_freeze_feature_plan(
            &manifest,
            &"a".repeat(64),
            &native_engine_identity().unwrap(),
        )
        .unwrap();
        assert_eq!(
            plan.slot_names().collect::<Vec<_>>(),
            ["histogram", "quote-volume", "close"]
        );
        let closes = ["1", "2", "3", "4", "5", "6", "5", "4", "3", "2", "1"];
        let bars = closes
            .into_iter()
            .enumerate()
            .map(|(index, close)| bar(index as i64, close, "1"))
            .collect::<Vec<_>>();
        let request = RunRequest {
            strategy_path: &fixture(),
            strategy_parameters: &[],
            factors: &[],
            bars: &bars,
            gaps: &[],
            signals: &[],
            plan: &plan,
            position_mode: PositionMode::LongOnly,
            limits: RunLimits::default(),
        };
        let result = RunEngine::execute(&request).unwrap();
        assert_eq!(
            result.decisions.first().unwrap().target_exposure,
            Decimal::ZERO
        );
        assert_eq!(
            result.decisions.last().unwrap().target_exposure,
            Decimal::ONE
        );
        assert!(
            result
                .decisions
                .iter()
                .any(|decision| decision.target_exposure == Decimal::ZERO)
        );
    }

    #[test]
    fn host_closed_bar_ceiling_cannot_be_raised_by_run_limits() {
        assert!(validate_bar_count(MAX_CLOSED_BARS, usize::MAX).is_ok());
        assert!(validate_bar_count(MAX_CLOSED_BARS + 1, usize::MAX).is_err());
    }

    #[test]
    fn guest_causes_are_bounded_at_four_kib_with_truncation_metadata() {
        let error = RunError::host("guest-error", RunStage::Strategy, "é".repeat(3_000));
        assert!(error.cause.len() <= MAX_GUEST_CAUSE_BYTES);
        assert!(error.cause.is_char_boundary(error.cause.len()));
        assert!(error.cause_truncated);
    }
}
