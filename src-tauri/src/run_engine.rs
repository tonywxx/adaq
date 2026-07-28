use std::collections::{HashMap, HashSet};

use ada_data_core::{BarGap, OhlcvBar};
use adaq_component_sdk::{decimal_to_f64, host::strategy_abi};
use adaq_component_tooling::{
    ComponentParameterValue, FrozenIndicatorPlan, FrozenSourceView, MarketField, RunLimits,
    WasmLoader,
};
use rust_decimal::Decimal;
use sha2::{Digest, Sha256};

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

pub(crate) struct RunEngine;

impl RunEngine {
    pub fn execute(request: &RunRequest<'_>) -> Result<RunResult, String> {
        validate_request(request)?;
        let mut result = RunResult {
            plan_hash: request.plan.plan_hash().to_owned(),
            decisions: Vec::with_capacity(request.bars.len()),
            gap_resets: Vec::new(),
            pauses: Vec::new(),
        };
        let mut start = 0;
        let mut next_gap = 0;
        while start < request.bars.len() {
            let end = request
                .gaps
                .get(next_gap)
                .and_then(|gap| {
                    request
                        .bars
                        .iter()
                        .position(|bar| bar.open_time_ms >= gap.start_time_ms)
                })
                .unwrap_or(request.bars.len());
            if end > start {
                execute_segment(request, &request.bars[start..end], &mut result)?;
            }
            start = end;
            if let Some(gap) = request.gaps.get(next_gap).copied() {
                result.gap_resets.push(gap);
                while start < request.bars.len()
                    && request.bars[start].open_time_ms < gap.end_time_ms
                {
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
) -> Result<(), String> {
    let strategy = load_strategy(request)?;
    let factor_values = evaluate_factors(request, bars)?;
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
                FrozenSourceView::Market(field) => market_value(field, bar)?,
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
            };
            if !value.is_finite() {
                return Err(format!(
                    "Feature Slot {slot} contains a non-finite value at Bar {}",
                    bar.open_time_ms
                ));
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
        let targets = strategy.process_strategy(vec![
            strategy_abi::exports::adaq::strategy::api::FeatureFrame {
                open_time_ms: bar.open_time_ms,
                values,
            },
        ])?;
        if targets.len() != 1 {
            return Err("Strategy must return exactly one Target Exposure per frame".into());
        }
        result.decisions.push(TargetDecision {
            open_time_ms: bar.open_time_ms,
            target_exposure: validate_target(&targets[0], request.position_mode)?,
        });
    }
    Ok(())
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
            FrozenSourceView::Market(_) => None,
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

pub(crate) fn market_engine_build_id() -> String {
    let mut hasher = Sha256::new();
    hasher.update(env!("CARGO_PKG_VERSION"));
    hasher.update(std::env::consts::OS);
    hasher.update(std::env::consts::ARCH);
    hasher.update(include_bytes!("run_engine.rs"));
    hasher.update(include_bytes!(
        "../crates/adaq-component-tooling/src/plan.rs"
    ));
    hasher.update(include_bytes!("../crates/adaq-component-sdk/src/lib.rs"));
    let source_hash = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!(
        "market-only-{}-{}-{source_hash}",
        std::env::consts::OS,
        std::env::consts::ARCH
    )
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
        ComponentManifest, EngineIdentity, FactorInstancePlanInput, validate_and_freeze,
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
            &EngineIdentity {
                engine_build_id: "test-build".into(),
            },
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
    fn market_engine_build_identity_is_source_and_target_specific() {
        let identity = market_engine_build_id();
        assert_eq!(identity, market_engine_build_id());
        assert!(identity.starts_with(&format!(
            "market-only-{}-{}-",
            std::env::consts::OS,
            std::env::consts::ARCH
        )));
        assert_eq!(identity.rsplit('-').next().unwrap().len(), 64);
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
            &EngineIdentity {
                engine_build_id: "test-build".into(),
            },
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
}
