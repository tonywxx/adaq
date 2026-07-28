use std::collections::HashSet;

use ada_data_core::{BarGap, OhlcvBar};
use adaq_component_sdk::{decimal_to_f64, host::strategy_abi};
use adaq_component_tooling::{
    ComponentParameterValue, FrozenIndicatorPlan, MarketField, RunLimits, WasmLoader,
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
    pub bars: &'a [OhlcvBar],
    pub gaps: &'a [BarGap],
    pub plan: &'a FrozenIndicatorPlan,
    pub position_mode: PositionMode,
    pub limits: RunLimits,
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
}

pub(crate) struct RunEngine;

impl RunEngine {
    pub fn execute(request: &RunRequest<'_>) -> Result<RunResult, String> {
        validate_request(request)?;
        let mut strategy = load_strategy(request)?;
        let fields = request.plan.market_fields().collect::<Vec<_>>();
        let mut result = RunResult {
            plan_hash: request.plan.plan_hash().to_owned(),
            decisions: Vec::with_capacity(request.bars.len()),
            gap_resets: Vec::new(),
        };
        let mut next_gap = 0;
        let mut segment_has_bars = false;

        for bar in request.bars {
            while let Some(gap) = request.gaps.get(next_gap).copied() {
                if gap.end_time_ms > bar.open_time_ms {
                    break;
                }
                if segment_has_bars {
                    strategy = load_strategy(request)?;
                    result.gap_resets.push(gap);
                    segment_has_bars = false;
                }
                next_gap += 1;
            }

            let values = fields
                .iter()
                .map(|field| market_value(*field, bar))
                .collect::<Result<Vec<_>, _>>()?;
            if values.iter().any(|value| !value.is_finite()) {
                return Err(format!(
                    "Market Feature Frame contains a non-finite value at Bar {}",
                    bar.open_time_ms
                ));
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
            segment_has_bars = true;
        }
        Ok(result)
    }
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

    use adaq_component_tooling::{ComponentManifest, EngineIdentity, validate_and_freeze};

    use super::*;

    fn fixture() -> String {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/strategy/target/wasm32-unknown-unknown/debug/m1_strategy_fixture.wasm")
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
}
