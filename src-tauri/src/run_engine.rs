use std::collections::{HashMap, HashSet, VecDeque};

use ada_data_core::{BarGap, OhlcvBar};
use rust_decimal::{Decimal, prelude::ToPrimitive};

use crate::{WasmLoader, factor_abi, strategy_abi};

const DEFAULT_FUEL_PER_CALL: u64 = 10_000_000;
const DEFAULT_MEMORY_BYTES: usize = 64 * 1024 * 1024;
const DEFAULT_MAX_BARS: usize = 10_000;

#[derive(Debug, Clone, Copy)]
pub(crate) struct RunLimits {
    pub fuel_per_call: u64,
    pub memory_bytes: usize,
    pub max_bars: usize,
}

impl Default for RunLimits {
    fn default() -> Self {
        Self {
            fuel_per_call: DEFAULT_FUEL_PER_CALL,
            memory_bytes: DEFAULT_MEMORY_BYTES,
            max_bars: DEFAULT_MAX_BARS,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PositionMode {
    LongOnly,
    LongShort,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FeatureSource {
    BuiltInSma { period: usize },
    FactorOutput { name: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FeatureBinding {
    pub slot_name: String,
    pub source: FeatureSource,
}

#[derive(Clone, Copy)]
pub(crate) struct RunRequest<'a> {
    pub factor_path: &'a str,
    pub strategy_path: &'a str,
    pub bars: &'a [OhlcvBar],
    pub gaps: &'a [BarGap],
    pub feature_bindings: &'a [FeatureBinding],
    pub position_mode: PositionMode,
    pub limits: RunLimits,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TargetDecision {
    pub open_time_ms: i64,
    pub target_exposure: Decimal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SkippedReason {
    Warmup,
    MissingInput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SkippedBar {
    pub open_time_ms: i64,
    pub reason: SkippedReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunResult {
    pub decisions: Vec<TargetDecision>,
    pub skipped_bars: Vec<SkippedBar>,
    pub gap_resets: Vec<BarGap>,
}

pub(crate) struct RunEngine;

impl RunEngine {
    pub fn execute(request: &RunRequest<'_>) -> Result<RunResult, String> {
        validate_request(request)?;

        let mut segment = SegmentRuntime::new(
            request.factor_path,
            request.strategy_path,
            request.feature_bindings,
            request.limits,
        )?;
        let mut indicators = indicator_states(request.feature_bindings);
        let mut result = RunResult {
            decisions: Vec::new(),
            skipped_bars: Vec::new(),
            gap_resets: Vec::new(),
        };
        let mut next_gap = 0;
        let mut segment_bars = 0usize;

        for bar in request.bars {
            while let Some(gap) = request.gaps.get(next_gap).copied() {
                if gap.end_time_ms > bar.open_time_ms {
                    break;
                }
                if segment_bars > 0 {
                    segment = SegmentRuntime::new(
                        request.factor_path,
                        request.strategy_path,
                        request.feature_bindings,
                        request.limits,
                    )?;
                    indicators = indicator_states(request.feature_bindings);
                    segment_bars = 0;
                    result.gap_resets.push(gap);
                }
                next_gap += 1;
            }

            let factor_values = segment.process_factor(bar)?;
            let mut values = Vec::with_capacity(request.feature_bindings.len());
            let mut missing = false;
            for state in &mut indicators {
                match state {
                    IndicatorState::BuiltInSma(sma) => match sma.push(bar.close)? {
                        Some(value) => values.push(value),
                        None => missing = true,
                    },
                    IndicatorState::FactorOutput(name) => {
                        match factor_values.as_ref().and_then(|values| values.get(name)) {
                            Some(value) => values.push(*value),
                            None => missing = true,
                        }
                    }
                }
            }

            if missing {
                let reason = if segment_bars < segment.warmup_bars {
                    SkippedReason::Warmup
                } else {
                    SkippedReason::MissingInput
                };
                result.skipped_bars.push(SkippedBar {
                    open_time_ms: bar.open_time_ms,
                    reason,
                });
            } else {
                let raw_target = segment.process_strategy(bar.open_time_ms, values)?;
                result.decisions.push(TargetDecision {
                    open_time_ms: bar.open_time_ms,
                    target_exposure: validate_target(&raw_target, request.position_mode)?,
                });
            }
            segment_bars += 1;
        }

        Ok(result)
    }
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
        return Err("Run limits must be greater than zero".to_owned());
    }
    if request
        .bars
        .windows(2)
        .any(|bars| bars[0].open_time_ms >= bars[1].open_time_ms)
    {
        return Err("Closed Bars must be strictly ascending".to_owned());
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
        return Err("Bar Gaps must be valid, ascending, and non-overlapping".to_owned());
    }
    if request.gaps.iter().any(|gap| {
        request
            .bars
            .iter()
            .any(|bar| bar.open_time_ms >= gap.start_time_ms && bar.open_time_ms < gap.end_time_ms)
    }) {
        return Err("Closed Bars cannot fall inside a Bar Gap".to_owned());
    }
    if request.feature_bindings.is_empty() {
        return Err("Indicator Plan must contain at least one Feature Slot".to_owned());
    }

    let mut slots = HashSet::new();
    for binding in request.feature_bindings {
        if binding.slot_name.trim().is_empty() || !slots.insert(binding.slot_name.as_str()) {
            return Err("Feature Slot names must be non-empty and unique".to_owned());
        }
        match &binding.source {
            FeatureSource::BuiltInSma { period } if *period == 0 => {
                return Err("SMA period must be greater than zero".to_owned());
            }
            FeatureSource::FactorOutput { name } if name.trim().is_empty() => {
                return Err("Factor output name must be non-empty".to_owned());
            }
            _ => {}
        }
    }
    Ok(())
}

struct SegmentRuntime {
    loader: WasmLoader,
    output_names: HashSet<String>,
    warmup_bars: usize,
}

impl SegmentRuntime {
    fn new(
        factor_path: &str,
        strategy_path: &str,
        feature_bindings: &[FeatureBinding],
        limits: RunLimits,
    ) -> Result<Self, String> {
        let loader = WasmLoader::with_limits(limits);
        loader.load(factor_path)?;
        let schema = loader.describe_factor()?;
        let output_count = schema.output_names.len();
        let output_names = schema.output_names.into_iter().collect::<HashSet<_>>();
        if output_names.iter().any(|name| name.trim().is_empty()) || output_names.is_empty() {
            return Err("Factor schema must declare non-empty output names".to_owned());
        }
        if output_names.len() != output_count {
            return Err("Factor output names must be unique".to_owned());
        }
        for binding in feature_bindings {
            if let FeatureSource::FactorOutput { name } = &binding.source
                && !output_names.contains(name)
            {
                return Err(format!("Unknown Factor output: {name}"));
            }
        }
        let indicator_warmup = feature_bindings
            .iter()
            .filter_map(|binding| match binding.source {
                FeatureSource::BuiltInSma { period } => Some(period - 1),
                FeatureSource::FactorOutput { .. } => None,
            })
            .max()
            .unwrap_or(0);
        loader.load_strategy(
            strategy_path,
            feature_bindings
                .iter()
                .map(
                    |binding| strategy_abi::exports::adaq::strategy::api::FeatureSlot {
                        name: binding.slot_name.clone(),
                    },
                )
                .collect(),
        )?;
        Ok(Self {
            loader,
            output_names,
            warmup_bars: (schema.warmup_bars as usize).max(indicator_warmup),
        })
    }

    fn process_factor(&mut self, bar: &OhlcvBar) -> Result<Option<HashMap<String, f64>>, String> {
        let mut rows = self.loader.process_factor(vec![
            factor_abi::exports::adaq::factor::api::ClosedBar {
                open_time_ms: bar.open_time_ms,
                open: bar.open.to_string(),
                high: bar.high.to_string(),
                low: bar.low.to_string(),
                close: bar.close.to_string(),
                base_volume: bar.base_volume.to_string(),
                quote_volume: bar.quote_volume.to_string(),
            },
        ])?;
        if rows.len() != 1 {
            return Err("Factor must return exactly one row per input bar".to_owned());
        }
        let Some(values) = rows.pop().expect("Factor result length was checked") else {
            return Ok(None);
        };
        let mut by_name = HashMap::with_capacity(values.len());
        for value in values {
            if !value.value.is_finite()
                || !self.output_names.contains(&value.name)
                || by_name.insert(value.name, value.value).is_some()
            {
                return Err("Factor returned an invalid, unknown, or duplicate output".to_owned());
            }
        }
        if by_name.len() != self.output_names.len() {
            return Err("Factor did not return every declared output".to_owned());
        }
        Ok(Some(by_name))
    }

    fn process_strategy(&mut self, open_time_ms: i64, values: Vec<f64>) -> Result<String, String> {
        let targets = self.loader.process_strategy(vec![
            strategy_abi::exports::adaq::strategy::api::FeatureFrame {
                open_time_ms,
                values,
            },
        ])?;
        if targets.len() != 1 {
            return Err("Strategy must return exactly one Target Exposure per frame".to_owned());
        }
        Ok(targets
            .into_iter()
            .next()
            .expect("Strategy result length was checked"))
    }
}

enum IndicatorState {
    BuiltInSma(Sma),
    FactorOutput(String),
}

fn indicator_states(bindings: &[FeatureBinding]) -> Vec<IndicatorState> {
    bindings
        .iter()
        .map(|binding| match &binding.source {
            FeatureSource::BuiltInSma { period } => IndicatorState::BuiltInSma(Sma::new(*period)),
            FeatureSource::FactorOutput { name } => IndicatorState::FactorOutput(name.clone()),
        })
        .collect()
}

struct Sma {
    period: usize,
    values: VecDeque<Decimal>,
    sum: Decimal,
}

impl Sma {
    fn new(period: usize) -> Self {
        Self {
            period,
            values: VecDeque::with_capacity(period),
            sum: Decimal::ZERO,
        }
    }

    fn push(&mut self, close: Decimal) -> Result<Option<f64>, String> {
        self.sum = self
            .sum
            .checked_add(close)
            .ok_or_else(|| "SMA overflowed Decimal".to_owned())?;
        self.values.push_back(close);
        if self.values.len() > self.period {
            self.sum = self
                .sum
                .checked_sub(self.values.pop_front().expect("SMA window is non-empty"))
                .ok_or_else(|| "SMA overflowed Decimal".to_owned())?;
        }
        if self.values.len() < self.period {
            return Ok(None);
        }
        self.sum
            .checked_div(Decimal::from(self.period as u64))
            .and_then(|value| value.to_f64())
            .filter(|value| value.is_finite())
            .map(Some)
            .ok_or_else(|| "SMA cannot be represented as a finite analytical value".to_owned())
    }
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

    use super::*;

    fn fixture(name: &str) -> String {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join(name)
            .join("target/wasm32-unknown-unknown/debug")
            .join(format!("m1_{name}_fixture.wasm"))
            .to_string_lossy()
            .into_owned()
    }

    fn bar(open_time_ms: i64, close: &str) -> OhlcvBar {
        let close = Decimal::from_str(close).unwrap();
        OhlcvBar {
            open_time_ms,
            open: close,
            high: close,
            low: close,
            close,
            base_volume: Decimal::ONE,
            quote_volume: close,
        }
    }

    fn bindings() -> Vec<FeatureBinding> {
        vec![
            FeatureBinding {
                slot_name: "change".to_owned(),
                source: FeatureSource::FactorOutput {
                    name: "close-change".to_owned(),
                },
            },
            FeatureBinding {
                slot_name: "sma-2".to_owned(),
                source: FeatureSource::BuiltInSma { period: 2 },
            },
        ]
    }

    #[test]
    fn run_is_reproducible_and_rewarms_after_a_gap() {
        let factor = fixture("factor");
        let strategy = fixture("strategy");
        let bars = vec![bar(1, "10"), bar(2, "11"), bar(4, "9"), bar(5, "8")];
        let gaps = vec![BarGap {
            start_time_ms: 3,
            end_time_ms: 4,
        }];
        let bindings = bindings();
        let request = RunRequest {
            factor_path: &factor,
            strategy_path: &strategy,
            bars: &bars,
            gaps: &gaps,
            feature_bindings: &bindings,
            position_mode: PositionMode::LongOnly,
            limits: RunLimits::default(),
        };

        let first = RunEngine::execute(&request).unwrap();
        let replay = RunEngine::execute(&request).unwrap();

        assert_eq!(first, replay);
        assert_eq!(
            first.decisions,
            [
                TargetDecision {
                    open_time_ms: 2,
                    target_exposure: Decimal::ONE,
                },
                TargetDecision {
                    open_time_ms: 5,
                    target_exposure: Decimal::ZERO,
                },
            ]
        );
        assert_eq!(
            first.skipped_bars,
            [
                SkippedBar {
                    open_time_ms: 1,
                    reason: SkippedReason::Warmup,
                },
                SkippedBar {
                    open_time_ms: 4,
                    reason: SkippedReason::Warmup,
                },
            ]
        );
        assert_eq!(first.gap_resets, gaps);
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
    fn run_rejects_unknown_factor_outputs_and_limit_violations() {
        let factor = fixture("factor");
        let strategy = fixture("strategy");
        let bars = vec![bar(1, "10"), bar(2, "11")];
        let unknown = vec![FeatureBinding {
            slot_name: "unknown".to_owned(),
            source: FeatureSource::FactorOutput {
                name: "missing".to_owned(),
            },
        }];
        let request = RunRequest {
            factor_path: &factor,
            strategy_path: &strategy,
            bars: &bars,
            gaps: &[],
            feature_bindings: &unknown,
            position_mode: PositionMode::LongOnly,
            limits: RunLimits::default(),
        };
        assert!(
            RunEngine::execute(&request)
                .unwrap_err()
                .contains("Unknown Factor output")
        );

        let bindings = bindings();
        let limited = RunRequest {
            feature_bindings: &bindings,
            limits: RunLimits {
                max_bars: 1,
                ..RunLimits::default()
            },
            ..request
        };
        assert!(
            RunEngine::execute(&limited)
                .unwrap_err()
                .contains("exceeding")
        );

        let fuel_limited = RunRequest {
            feature_bindings: &bindings,
            limits: RunLimits {
                fuel_per_call: 1,
                ..RunLimits::default()
            },
            ..request
        };
        assert!(RunEngine::execute(&fuel_limited).is_err());

        let memory_limited = RunRequest {
            feature_bindings: &bindings,
            limits: RunLimits {
                memory_bytes: 1,
                ..RunLimits::default()
            },
            ..request
        };
        assert!(RunEngine::execute(&memory_limited).is_err());
    }
}
