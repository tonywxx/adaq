use std::collections::{BTreeMap, HashMap, HashSet};

use ada_data_core::{BarGap, OhlcvBar};
use adaq_component_sdk::{decimal_to_f64, host::strategy_abi};
use adaq_component_tooling::{
    ComponentParameterValue, FrozenBuiltInParameter, FrozenIndicatorPlan, FrozenSourceView,
    MarketField, RunLimits, WasmLoader, builtin_engine_market_field,
};
use adaq_indicator_engine::{
    EngineError, IndicatorColumn, IndicatorEngine, IndicatorRequest, OhlcvSegment, ParameterValue,
};
use rust_decimal::Decimal;

const MAX_INDICATOR_OUTPUT_CELLS: usize = 16_777_216;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PositionMode {
    LongOnly,
    LongShort,
}

#[derive(Clone, Copy)]
pub(crate) struct RunRequest<'a> {
    pub strategy_path: &'a str,
    pub strategy_parameters: &'a [ComponentParameterValue],
    pub factors: &'a [FactorRunRequest<'a>],
    pub bars: &'a [OhlcvBar],
    pub gaps: &'a [BarGap],
    pub plan: &'a FrozenIndicatorPlan,
    pub position_mode: PositionMode,
    pub limits: RunLimits,
}

#[derive(Clone, Copy)]
pub(crate) struct FactorRunRequest<'a> {
    pub alias: &'a str,
    pub path: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TargetDecision {
    pub open_time_ms: i64,
    pub target_exposure: Decimal,
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
    BuiltInInput,
    BuiltInCompile,
    BuiltInEvaluate,
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
        Self {
            code: code.into(),
            stage,
            bar_open_time_ms: None,
            slot: None,
            source: None,
            ta_ret_code: None,
            ta_ret_code_name: None,
            cause: bounded_context(&cause.into()).into(),
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
    let builtin_values = evaluate_builtins(request, bars)?;
    let factor_values = evaluate_factors(request, bars)
        .map_err(|error| RunError::host("factor-evaluation-failed", RunStage::Factor, error))?;
    let mut frames = Vec::with_capacity(bars.len());
    for (index, bar) in bars.iter().enumerate() {
        if index < request.plan.effective_warmup_bars() as usize {
            result.pauses.push(RunPause {
                open_time_ms: bar.open_time_ms,
                reason: RunPauseReason::Warmup,
            });
            continue;
        }
        let mut values = Vec::with_capacity(request.plan.slot_names().len());
        let mut missing = None;
        for (slot, source) in request.plan.slot_names().zip(request.plan.sources()) {
            let value = match source {
                FrozenSourceView::Market(field) => market_value(field, bar).map_err(|error| {
                    RunError::host("invalid-market-input", RunStage::FeatureFrame, error)
                        .at_bar(bar.open_time_ms)
                        .for_slot(slot, format!("market:{field:?}"))
                })?,
                FrozenSourceView::External {
                    dependency_alias,
                    output,
                } => match factor_values
                    .get(dependency_alias)
                    .and_then(|rows| rows[index].as_ref())
                    .and_then(|row| row.get(output).copied())
                {
                    Some(value) => value,
                    None => {
                        missing = Some((
                            slot.to_owned(),
                            format!("external:{dependency_alias}:{output}"),
                        ));
                        break;
                    }
                },
                FrozenSourceView::BuiltIn {
                    indicator,
                    output,
                    real_inputs,
                    parameters,
                } => {
                    let key = builtin_key(indicator, output, real_inputs, parameters);
                    match builtin_values.values.get(&key).and_then(|rows| rows[index]) {
                        Some(value) => value,
                        None => {
                            missing =
                                Some((slot.to_owned(), format!("builtin:{indicator}:{output}")));
                            break;
                        }
                    }
                }
            };
            if !value.is_finite() {
                return Err(RunError::host(
                    "non-finite-feature-slot",
                    RunStage::FeatureFrame,
                    "Feature Slot contains a non-finite value",
                )
                .at_bar(bar.open_time_ms)
                .for_slot(slot, source_name(source)));
            }
            values.push(value);
        }
        if let Some((slot, source)) = missing {
            result.pauses.push(RunPause {
                open_time_ms: bar.open_time_ms,
                reason: RunPauseReason::MissingInput { slot, source },
            });
            continue;
        }
        frames.push(strategy_abi::exports::adaq::strategy::api::FeatureFrame {
            open_time_ms: bar.open_time_ms,
            values,
        });
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
            FrozenSourceView::Market(_) | FrozenSourceView::BuiltIn { .. } => None,
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
        loader.load_with_parameters(paths[factor.alias], factor.parameters)?;
        let mut rows = Vec::with_capacity(bars.len());
        for chunk in bars.chunks(4096) {
            let input = chunk.iter().map(factor_bar).collect::<Vec<_>>();
            let output = loader.process_factor(input).map_err(|error| {
                format!(
                    "factor-guest-error:{}:{}",
                    factor.alias,
                    bounded_context(&error)
                )
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
                let row = row.map(|values| {
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

fn bounded_context(value: &str) -> &str {
    value.get(..value.floor_char_boundary(256)).unwrap_or(value)
}

fn factor_bar(
    bar: &OhlcvBar,
) -> adaq_component_sdk::host::factor_abi::exports::adaq::factor::api::ClosedBar {
    adaq_component_sdk::host::factor_abi::exports::adaq::factor::api::ClosedBar {
        open_time_ms: bar.open_time_ms,
        open: bar.open.to_string(),
        high: bar.high.to_string(),
        low: bar.low.to_string(),
        close: bar.close.to_string(),
        base_volume: bar.base_volume.to_string(),
        quote_volume: bar.quote_volume.to_string(),
    }
}

fn validate_factor_row<'a>(
    alias: &str,
    expected: &[String],
    values: Vec<adaq_component_sdk::host::factor_abi::exports::adaq::factor::api::NamedScalar>,
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

fn evaluate_builtins(
    request: &RunRequest<'_>,
    bars: &[OhlcvBar],
) -> Result<BuiltinEvaluation, RunError> {
    let mut requests = BTreeMap::<
        String,
        (
            String,
            Vec<MarketField>,
            BTreeMap<String, FrozenBuiltInParameter>,
            Vec<String>,
            String,
        ),
    >::new();
    for (slot, source) in request.plan.slot_names().zip(request.plan.sources()) {
        let FrozenSourceView::BuiltIn {
            indicator,
            output,
            real_inputs,
            parameters,
        } = source
        else {
            continue;
        };
        let key = builtin_request_key(indicator, real_inputs, parameters);
        let entry = requests.entry(key).or_insert_with(|| {
            (
                indicator.into(),
                real_inputs.to_vec(),
                parameters.clone(),
                Vec::new(),
                slot.into(),
            )
        });
        if !entry.3.iter().any(|selected| selected == output) {
            entry.3.push(output.into());
        }
    }
    if requests.is_empty() {
        return Ok(BuiltinEvaluation {
            values: HashMap::new(),
            request_count: 0,
        });
    }
    let request_count = requests.len();
    let output_count = requests
        .values()
        .map(|request| request.3.len())
        .sum::<usize>();
    validate_builtin_output_cells(bars.len(), output_count)?;
    let first = requests
        .values()
        .next()
        .expect("non-empty Built-in requests");
    let first_source = format!("builtin:{}", first.0);
    let first_slot = &first.4;
    let engine = IndicatorEngine::initialize().map_err(|error| {
        builtin_engine_error(
            error,
            RunStage::BuiltInCompile,
            bars,
            first_slot,
            &first_source,
        )
    })?;
    let segment = OhlcvSegment::new(
        builtin_market_column(bars, MarketField::Open, first_slot, &first_source)?,
        builtin_market_column(bars, MarketField::High, first_slot, &first_source)?,
        builtin_market_column(bars, MarketField::Low, first_slot, &first_source)?,
        builtin_market_column(bars, MarketField::Close, first_slot, &first_source)?,
        builtin_market_column(bars, MarketField::BaseVolume, first_slot, &first_source)?,
        builtin_market_column(bars, MarketField::QuoteVolume, first_slot, &first_source)?,
    )
    .map_err(|error| {
        builtin_engine_error(
            error,
            RunStage::BuiltInInput,
            bars,
            first_slot,
            &first_source,
        )
    })?;
    let mut values = HashMap::new();
    for (_, (indicator, real_inputs, parameters, outputs, slot)) in requests {
        let source = format!("builtin:{indicator}");
        let compiled = engine
            .compile(IndicatorRequest {
                indicator_id: indicator.clone(),
                outputs: outputs.clone(),
                real_inputs: real_inputs
                    .iter()
                    .map(|field| builtin_engine_market_field(*field))
                    .collect(),
                parameters: parameters
                    .iter()
                    .map(|(id, value)| {
                        engine_parameter(value)
                            .map(|value| (id.clone(), value))
                            .map_err(|error| {
                                RunError::host(
                                    "invalid-frozen-indicator-parameter",
                                    RunStage::BuiltInCompile,
                                    error,
                                )
                                .at_bar(bars[0].open_time_ms)
                                .for_slot(&slot, &source)
                            })
                    })
                    .collect::<Result<_, RunError>>()?,
            })
            .map_err(|error| {
                builtin_engine_error(error, RunStage::BuiltInCompile, bars, &slot, &source)
            })?;
        let columns = engine.evaluate(&compiled, &segment).map_err(|error| {
            builtin_engine_error(error, RunStage::BuiltInEvaluate, bars, &slot, &source)
        })?;
        for (output, column) in columns {
            let rows = match column {
                IndicatorColumn::Real(rows) => rows,
                IndicatorColumn::Integer(rows) => {
                    rows.into_iter().map(|value| value.map(f64::from)).collect()
                }
            };
            values.insert(
                builtin_key(&indicator, &output, &real_inputs, &parameters),
                rows,
            );
        }
    }
    Ok(BuiltinEvaluation {
        values,
        request_count,
    })
}

struct BuiltinEvaluation {
    values: HashMap<String, Vec<Option<f64>>>,
    #[cfg_attr(not(test), allow(dead_code))]
    request_count: usize,
}

fn validate_builtin_output_cells(bar_count: usize, output_count: usize) -> Result<(), RunError> {
    if bar_count
        .checked_mul(output_count)
        .is_none_or(|cells| cells > MAX_INDICATOR_OUTPUT_CELLS)
    {
        return Err(RunError::host(
            "too-many-indicator-output-cells",
            RunStage::Validation,
            "Built-in Indicator outputs exceed the Run limit",
        ));
    }
    Ok(())
}

fn builtin_market_column(
    bars: &[OhlcvBar],
    field: MarketField,
    slot: &str,
    source: &str,
) -> Result<Vec<f64>, RunError> {
    bars.iter()
        .map(|bar| {
            market_value(field, bar).map_err(|error| {
                RunError::host("invalid-indicator-input", RunStage::BuiltInInput, error)
                    .at_bar(bar.open_time_ms)
                    .for_slot(slot, source)
            })
        })
        .collect()
}

fn builtin_engine_error(
    error: EngineError,
    stage: RunStage,
    bars: &[OhlcvBar],
    slot: &str,
    source: &str,
) -> RunError {
    let index = match &error {
        EngineError::NonFiniteOutput { index, .. } => *index,
        _ => 0,
    };
    let (ta_ret_code, ta_ret_code_name) = match &error {
        EngineError::Initialization {
            ret_code,
            ret_code_name,
            ..
        }
        | EngineError::TaLib {
            ret_code,
            ret_code_name,
            ..
        } => (Some(*ret_code), Some((*ret_code_name).into())),
        _ => (None, None),
    };
    RunError {
        code: error.code().into(),
        stage,
        bar_open_time_ms: bars.get(index).map(|bar| bar.open_time_ms),
        slot: Some(slot.into()),
        source: Some(source.into()),
        ta_ret_code,
        ta_ret_code_name,
        cause: bounded_context(&error.to_string()).into(),
    }
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
    }
}

fn builtin_key(
    indicator: &str,
    output: &str,
    real_inputs: &[MarketField],
    parameters: &BTreeMap<String, FrozenBuiltInParameter>,
) -> String {
    format!("{indicator}:{output}:{real_inputs:?}:{parameters:?}")
}
fn builtin_request_key(
    indicator: &str,
    real_inputs: &[MarketField],
    parameters: &BTreeMap<String, FrozenBuiltInParameter>,
) -> String {
    format!("{indicator}:{real_inputs:?}:{parameters:?}")
}
fn engine_parameter(value: &FrozenBuiltInParameter) -> Result<ParameterValue, String> {
    match value {
        FrozenBuiltInParameter::Integer(value) => Ok(ParameterValue::Integer(*value)),
        FrozenBuiltInParameter::Real(value) => value
            .parse()
            .map(ParameterValue::Real)
            .map_err(|_| "invalid-indicator-parameter".into()),
        FrozenBuiltInParameter::Enum(value) => Ok(ParameterValue::Enum(value.clone())),
    }
}

fn market_value(field: MarketField, bar: &OhlcvBar) -> Result<f64, String> {
    let (name, value) = match field {
        MarketField::Open => ("open", bar.open),
        MarketField::High => ("high", bar.high),
        MarketField::Low => ("low", bar.low),
        MarketField::Close => ("close", bar.close),
        MarketField::BaseVolume => ("base-volume", bar.base_volume),
        MarketField::QuoteVolume => ("quote-volume", bar.quote_volume),
    };
    decimal_to_f64(value).map_err(|error| {
        format!(
            "Market field {} cannot be converted at Bar {}: {error}",
            name, bar.open_time_ms
        )
    })
}

fn validate_request(request: &RunRequest<'_>) -> Result<(), String> {
    if request.bars.len() > request.limits.max_bars {
        return Err(format!(
            "Run contains {} bars, exceeding the limit of {}",
            request.bars.len(),
            request.limits.max_bars
        ));
    }
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
        return Err("Frozen Indicator Plan contains duplicate Feature Slots".into());
    }
    Ok(())
}

fn validate_target(raw: &str, mode: PositionMode) -> Result<Decimal, String> {
    let target = Decimal::from_str_exact(raw)
        .map_err(|error| format!("Invalid Target Exposure: {error}"))?;
    let minimum = match mode {
        PositionMode::LongOnly => Decimal::ZERO,
        PositionMode::LongShort => -Decimal::ONE,
    };
    if target < minimum || target > Decimal::ONE {
        return Err(format!("Target Exposure {target} is outside [{minimum},1]"));
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, str::FromStr};

    use adaq_component_tooling::{
        ComponentManifest, FactorInstancePlanInput, native_engine_identity, validate_and_freeze,
        validate_and_freeze_with_factors,
    };

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

    fn plan() -> FrozenIndicatorPlan {
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
        validate_and_freeze(
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
    fn identical_closed_bars_are_collapsed_but_conflicts_fail() {
        let plan = plan();
        let strategy = fixture();
        let first = bar(1, "10", "5");
        let request = RunRequest {
            strategy_path: &strategy,
            strategy_parameters: &[],
            factors: &[],
            bars: &[first.clone(), first.clone(), bar(2, "11", "20")],
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
        let plan = validate_and_freeze_with_factors(
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
        let result = RunEngine::execute(&RunRequest {
            strategy_path: &strategy,
            strategy_parameters: &[],
            factors: &factors,
            bars: &bars,
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
        let plan = validate_and_freeze_with_factors(
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
        let plan = validate_and_freeze_with_factors(
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
        let plan = validate_and_freeze_with_factors(
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
        let plan = validate_and_freeze_with_factors(
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
        let plan = validate_and_freeze_with_factors(
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
        let plan = validate_and_freeze(
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
    fn multi_output_builtin_request_reuses_one_evaluation_for_semantic_slots() {
        let manifest = serde_json::from_str::<ComponentManifest>(r#"{
            "manifestSchemaVersion":"1.0.0","componentId":"00000000-0000-0000-0000-000000000000",
            "version":"1.0.0","name":"MACD Fixture","kind":"strategy","sdkVersion":"0.1.0","abiVersion":"1.0.0",
            "featureSlots":[
                {"name":"macd","source":{"kind":"builtin","indicator":"macd","output":"macd","inputs":{"real-0":"close"}}},
                {"name":"signal","source":{"kind":"builtin","indicator":"macd","output":"signal","inputs":{"real-0":"close"}}},
                {"name":"histogram","source":{"kind":"builtin","indicator":"macd","output":"histogram","inputs":{"real-0":"close"}}}
            ]
        }"#).unwrap();
        let plan = validate_and_freeze(
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
            plan: &plan,
            position_mode: PositionMode::LongOnly,
            limits: RunLimits::default(),
        };
        let values = evaluate_builtins(&request, &bars).unwrap();
        assert_eq!(values.values.len(), 3);
        assert_eq!(values.request_count, 1);
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
        let plan = validate_and_freeze(
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
        let plan = validate_and_freeze(
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
            plan: &plan,
            position_mode: PositionMode::LongOnly,
            limits: RunLimits::default(),
        };
        let evaluation = evaluate_builtins(&request, &bars).unwrap();
        assert_eq!(evaluation.request_count, 1);
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
    fn indicator_failures_preserve_typed_run_context() {
        let bars = vec![bar(10, "1", "1"), bar(20, "2", "2")];
        let ta = builtin_engine_error(
            EngineError::TaLib {
                code: "ta-lib-error",
                ret_code: 2,
                ret_code_name: "TA_BAD_PARAM",
            },
            RunStage::BuiltInEvaluate,
            &bars,
            "rsi",
            "builtin:rsi",
        );
        assert_eq!(ta.stage, RunStage::BuiltInEvaluate);
        assert_eq!(ta.bar_open_time_ms, Some(10));
        assert_eq!(ta.slot.as_deref(), Some("rsi"));
        assert_eq!(ta.source.as_deref(), Some("builtin:rsi"));
        assert_eq!(ta.ta_ret_code, Some(2));
        assert_eq!(ta.ta_ret_code_name.as_deref(), Some("TA_BAD_PARAM"));

        let non_finite = builtin_engine_error(
            EngineError::NonFiniteOutput {
                code: "non-finite-indicator-output",
                index: 1,
            },
            RunStage::BuiltInEvaluate,
            &bars,
            "rsi",
            "builtin:rsi",
        );
        assert_eq!(non_finite.bar_open_time_ms, Some(20));
    }

    #[test]
    fn indicator_output_cell_limit_is_checked_before_allocation() {
        assert!(validate_builtin_output_cells(1_000_000, 16).is_ok());
        let error = validate_builtin_output_cells(1_000_000, 17).unwrap_err();
        assert_eq!(error.code, "too-many-indicator-output-cells");
        assert_eq!(error.stage, RunStage::Validation);
    }
}
