use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    path::PathBuf,
};

use adaq_backtest_core::{
    BacktestEvidence, PortfolioBacktestRequest as CoreRequest, PortfolioMarketDecision,
    PortfolioState, RiskPolicy, StrategyTarget, TopNForecastStrategy,
    apply_portfolio_market_decision, execute_portfolio_backtest, mark_portfolio_to_market,
};
use adaq_component_sdk::host::portfolio_strategy_abi;
use adaq_component_tooling::{
    ComponentKind, ComponentPackage, ComponentParameterValue, FactorInstancePlanInput,
    FeatureSlotSource, ModelOutput, RunLimits, SignalPlanInput, StrategyScope, WasmLoader,
    component_parameters, native_engine_identity,
    validate_and_freeze_feature_plan_with_bindings_and_parameters,
};
use adaq_data_core::OhlcvBar;
use rusqlite::{OptionalExtension, params};
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    BacktestRunRequest, Backtests, FactorInstanceRequest, NormalizedFactorInstance,
    PortfolioBacktestProvenance, PortfolioBacktestRequest, PortfolioBacktestView,
    PortfolioSignalLock, RunPauseRecord, string,
};
use crate::run_engine::{
    FactorRunRequest, MaterializedFeatureRow, SignalRunRequest, SignalRunRow,
    materialize_feature_segment_with_signals,
};

struct QualifiedFactor {
    request: FactorInstanceRequest,
    package: ComponentPackage,
    parameters: Vec<ComponentParameterValue>,
    path: PathBuf,
}

#[derive(Clone)]
struct QualifiedSignal {
    slot: String,
    dataset_id: String,
    signal_name: String,
    src: String,
    interval: String,
    contract: ModelOutput,
    producer_segments: Vec<serde_json::Value>,
    artifact_provenance: serde_json::Value,
    evidence_state: String,
    component_lock: Vec<serde_json::Value>,
    rows: Vec<SignalRunRow>,
}

struct PortfolioInstrumentFeatures {
    instrument_id: String,
    bars: BTreeMap<i64, OhlcvBar>,
    rows: BTreeMap<i64, MaterializedFeatureRow>,
}

pub(super) fn execute(
    backtests: &Backtests,
    request: PortfolioBacktestRequest,
) -> Result<PortfolioBacktestView, String> {
    if request.strategy_id.trim().is_empty()
        || request.universe_snapshot_id.trim().is_empty()
        || request.signal_dataset_ids.is_empty()
    {
        return Err("Portfolio Backtest request is incomplete".into());
    }
    let project = backtests.strategy_project(&request.user_id, &request.strategy_id)?;
    if project.scope != adaq_backtest_core::StrategyScope::Portfolio
        || project.context_hash != request.universe_snapshot_id
    {
        return Err("Portfolio Backtest Strategy Context is mixed or stale".into());
    }
    let request_json = serde_json::to_vec(&request).map_err(string)?;
    let request_hash = Sha256::digest(request_json)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if let Some(evidence) = load_existing(backtests, &request.user_id, &request_hash)? {
        let metrics = evidence.metrics.clone();
        return Ok(PortfolioBacktestView {
            run_id: format!("portfolio-{request_hash}"),
            reused_existing_run: true,
            evidence,
            metrics,
            pauses: Vec::new(),
            provenance: None,
        });
    }

    let initial_capital = decimal(&request.initial_capital, "initial-capital")?;
    let execution_cost_rate = decimal(&request.execution_cost_rate, "execution-cost-rate")?;
    let max_instrument_weight = decimal(&request.max_instrument_weight, "max-instrument-weight")?;
    let max_turnover = request
        .max_turnover
        .as_deref()
        .map(|value| decimal(value, "max-turnover"))
        .transpose()?;
    let universe = backtests
        .source()
        .portfolio_universe_snapshot_for_user(&request.user_id, &request.universe_snapshot_id)?;
    if universe.universe.evidence_state == "unknown"
        || universe.universe.evidence_reasons.is_empty()
    {
        return Err("Portfolio Backtest requires known Universe evidence".into());
    }
    if request.top_n == 0 || request.top_n > universe.components.len() {
        return Err("Portfolio Backtest Top-N exceeds the frozen Universe".into());
    }

    let mut bars_by_code = BTreeMap::new();
    let mut snapshot_by_code = BTreeMap::new();
    for component in &universe.components {
        let (snapshot, bars) = backtests
            .source()
            .snapshot_for_user(&request.user_id, &component.snapshot_id)?;
        if snapshot.snapshot_id != component.snapshot_id
            || snapshot.interval != universe.interval
            || snapshot.start_time_ms < universe.start_time_ms
            || snapshot.end_time_ms > universe.end_time_ms
        {
            return Err("Portfolio Backtest Snapshot Context is mixed or incomplete".into());
        }
        let code = component.dataset.instrument.code.clone();
        if bars_by_code.insert(code.clone(), bars).is_some() {
            return Err("Portfolio Backtest Universe contains duplicate instruments".into());
        }
        snapshot_by_code.insert(code, component.snapshot_id.clone());
    }

    let datasets = backtests.source().signal_datasets(
        &request.user_id,
        true,
        Some(&request.signal_dataset_ids),
    )?;
    let mut datasets_by_code = BTreeMap::new();
    for dataset in datasets {
        if dataset.snapshot_id.trim().is_empty() || dataset.evidence_state == "unknown" {
            return Err("Portfolio Backtest Signal Context is not admissible".into());
        }
        let code = dataset.code.clone();
        let snapshot_id = snapshot_by_code
            .get(&code)
            .ok_or_else(|| "Portfolio Backtest Signal is outside the frozen Universe".to_owned())?;
        if snapshot_id != &dataset.snapshot_id || datasets_by_code.insert(code, dataset).is_some() {
            return Err("Portfolio Backtest Signal Context is mixed or duplicated".into());
        }
    }
    if datasets_by_code.len() != bars_by_code.len() {
        return Err("Portfolio Backtest requires one causal Signal Dataset per instrument".into());
    }
    let window = match request.window {
        adaq_backtest_core::EvaluationWindow::Selection => &project.selection_window,
        adaq_backtest_core::EvaluationWindow::Final => &project.final_window,
    };

    let times = bars_by_code
        .values()
        .next()
        .ok_or_else(|| "Portfolio Backtest Universe has no Bars".to_owned())?
        .iter()
        .map(|bar| bar.open_time_ms)
        .filter(|time| {
            *time >= window.start_time_ms
                && *time <= window.end_time_ms
                && bars_by_code
                    .values()
                    .all(|bars| bars.iter().any(|bar| bar.open_time_ms == *time))
        })
        .collect::<Vec<_>>();
    if times.is_empty() {
        return Err("Portfolio Backtest Universe has no aligned Closed Bars".into());
    }

    let mut decisions = Vec::with_capacity(times.len());
    for time in times {
        let mut prices = BTreeMap::new();
        let mut forecasts = BTreeMap::new();
        for (code, bars) in &bars_by_code {
            let bar = bars
                .iter()
                .find(|bar| bar.open_time_ms == time)
                .ok_or_else(|| "Portfolio Backtest price alignment failed".to_owned())?;
            prices.insert(code.clone(), bar.open);
            let dataset = datasets_by_code
                .get(code)
                .ok_or_else(|| format!("Portfolio Backtest signal is missing for {code}"))?;
            let row = dataset
                .rows
                .iter()
                .find(|row| row.prediction_time_ms == time)
                .ok_or_else(|| format!("Portfolio Backtest signal row is missing for {code}"))?;
            if row.available_at_ms > time {
                return Err("Portfolio Backtest signal is not causally available".into());
            }
            let value = row
                .values
                .as_ref()
                .and_then(|values| values.first())
                .and_then(|value| Decimal::from_f64(*value))
                .ok_or_else(|| "Portfolio Backtest signal value is unavailable".to_owned())?;
            forecasts.insert(code.clone(), value);
        }
        let target = TopNForecastStrategy::target(
            time,
            &universe.universe.universe_id,
            &forecasts,
            request.top_n,
        )
        .map_err(string)?;
        decisions.push(PortfolioMarketDecision {
            time_ms: time,
            prices,
            strategy_target: StrategyTarget {
                target,
                strategy_id: request.strategy_id.clone(),
                input_provenance: BTreeMap::from([(
                    "universeSnapshotId".into(),
                    universe.snapshot_id.clone(),
                )]),
            },
        });
    }

    let evidence = execute_portfolio_backtest(CoreRequest {
        initial_capital,
        risk_policy: RiskPolicy {
            policy_id: format!("strategy:{}", request.strategy_id),
            max_instrument_weight,
            max_turnover,
        },
        execution_cost_rate,
        decisions,
    })
    .map_err(string)?;
    let run_id = format!("portfolio-{request_hash}");
    let evidence_json = serde_json::to_string(&evidence).map_err(string)?;
    let metrics = evidence.metrics.clone();
    backtests.save_portfolio_run(
        &request.user_id,
        &run_id,
        &request_hash,
        &evidence_json,
        &[],
    )?;
    Ok(PortfolioBacktestView {
        run_id,
        reused_existing_run: false,
        evidence,
        metrics,
        pauses: Vec::new(),
        provenance: None,
    })
}

pub(super) fn execute_qualified(
    backtests: &Backtests,
    request: BacktestRunRequest,
) -> Result<PortfolioBacktestView, String> {
    let universe_snapshot_id = request
        .portfolio_universe_snapshot_id
        .clone()
        .ok_or("Portfolio Strategy requires a Point-in-Time Universe Snapshot")?;
    let source = backtests.source();
    let strategy = source.package_for_user(&request.user_id, &request.strategy_archive_sha256)?;
    if strategy.manifest.kind != ComponentKind::Strategy
        || strategy.manifest.strategy_scope != StrategyScope::Portfolio
    {
        return Err("Portfolio qualification requires a Portfolio Strategy Component".into());
    }
    let strategy_values =
        component_parameters(&strategy.manifest, Some(&request.strategy_parameters))?;
    let strategy_parameters =
        super::pipeline::normalized_parameters(&strategy.manifest, &strategy_values);
    let (representative_snapshot, _) =
        source.snapshot_for_user(&request.user_id, &request.snapshot_id)?;
    let universe =
        source.portfolio_universe_snapshot_for_user(&request.user_id, &universe_snapshot_id)?;
    if universe.snapshot_id != universe_snapshot_id
        || universe.universe.evidence_state == "unknown"
        || universe.universe.evidence_reasons.is_empty()
        || universe.components.is_empty()
    {
        return Err("Portfolio Backtest requires known Point-in-Time Universe evidence".into());
    }
    let start_time_ms = request.run_start_time_ms.unwrap_or(universe.start_time_ms);
    let end_time_ms = request.run_end_time_ms.unwrap_or(universe.end_time_ms);
    if start_time_ms > end_time_ms
        || start_time_ms < universe.start_time_ms
        || end_time_ms > universe.end_time_ms
    {
        return Err("Portfolio Backtest window is outside the frozen Universe".into());
    }

    let mut instruments = Vec::with_capacity(universe.components.len());
    let mut instrument_snapshots = HashMap::new();
    for component in &universe.components {
        let instrument_id = format!(
            "{}:{}",
            component.dataset.instrument.venue.id, component.dataset.instrument.code
        );
        if instrument_snapshots
            .insert(instrument_id.clone(), component.snapshot_id.clone())
            .is_some()
        {
            return Err("Portfolio Universe contains duplicate Instrument IDs".into());
        }
        let (snapshot, bars) =
            source.snapshot_for_user(&request.user_id, &component.snapshot_id)?;
        if snapshot.snapshot_id != component.snapshot_id
            || snapshot.interval != universe.interval
            || snapshot.start_time_ms > universe.start_time_ms
            || snapshot.end_time_ms < universe.end_time_ms
        {
            return Err("Portfolio Backtest Snapshot Context is mixed or incomplete".into());
        }
        let bars = bars
            .into_iter()
            .filter(|bar| bar.open_time_ms >= start_time_ms && bar.open_time_ms <= end_time_ms)
            .collect::<Vec<_>>();
        if bars.is_empty() {
            return Err(format!(
                "Portfolio Backtest has no Bars for {instrument_id}"
            ));
        }
        instruments.push((instrument_id, component.snapshot_id.clone(), bars));
    }

    let declared_signal_slots = strategy
        .manifest
        .feature_slots
        .iter()
        .filter(|slot| matches!(slot.source, FeatureSlotSource::Signal { .. }))
        .collect::<Vec<_>>();
    let mut dataset_ids = request
        .signal_instances
        .iter()
        .map(|signal| signal.dataset_id.clone())
        .collect::<Vec<_>>();
    dataset_ids.sort();
    dataset_ids.dedup();
    let datasets = source.signal_datasets(&request.user_id, true, Some(&dataset_ids))?;
    let mut signals = HashMap::<(String, String), QualifiedSignal>::new();
    for binding in &request.signal_instances {
        let dataset = datasets
            .iter()
            .find(|dataset| dataset.dataset_id == binding.dataset_id)
            .ok_or_else(|| {
                format!(
                    "Forecast Signal Dataset is not available: {}",
                    binding.dataset_id
                )
            })?;
        let instrument_id = format!("{}:{}", dataset.src, dataset.code);
        let snapshot_id = instrument_snapshots
            .get(&instrument_id)
            .ok_or_else(|| "Forecast Signal is outside the frozen Universe".to_owned())?;
        if dataset.snapshot_id != *snapshot_id
            || dataset.interval != universe.interval.as_str()
            || dataset.evidence_state == "unknown"
        {
            return Err("Portfolio Forecast Signal Context is mixed or inadmissible".into());
        }
        let slot = strategy
            .manifest
            .feature_slots
            .iter()
            .find(|slot| slot.name == binding.slot)
            .ok_or_else(|| format!("Unknown Forecast Signal Slot: {}", binding.slot))?;
        let FeatureSlotSource::Signal {
            prediction_kind,
            forecast_target,
            value_scale,
            horizon_bars,
        } = &slot.source
        else {
            return Err("Portfolio Signal binding targets a non-signal Slot".into());
        };
        let output_index = dataset
            .outputs
            .iter()
            .position(|output| {
                output.name == binding.signal_name
                    && output.prediction_kind == *prediction_kind
                    && output.forecast_target == *forecast_target
                    && output.value_scale == *value_scale
                    && output.horizon_bars == *horizon_bars
            })
            .ok_or_else(|| {
                format!(
                    "Selected Forecast Signal is not compatible: {}",
                    binding.signal_name
                )
            })?;
        let signal = QualifiedSignal {
            slot: binding.slot.clone(),
            dataset_id: dataset.dataset_id.clone(),
            signal_name: binding.signal_name.clone(),
            src: dataset.src.clone(),
            interval: dataset.interval.clone(),
            contract: dataset.outputs[output_index].clone(),
            producer_segments: dataset.producer_segments.clone(),
            artifact_provenance: dataset.artifact_provenance.clone(),
            evidence_state: dataset.evidence_state.clone(),
            component_lock: dataset.component_lock.clone(),
            rows: dataset
                .rows
                .iter()
                .map(|row| SignalRunRow {
                    prediction_time_ms: row.prediction_time_ms,
                    available_at_ms: row.available_at_ms,
                    value: row
                        .values
                        .as_ref()
                        .and_then(|values| values.get(output_index).copied()),
                })
                .collect(),
        };
        if signals
            .insert((instrument_id, binding.slot.clone()), signal)
            .is_some()
        {
            return Err(
                "Portfolio Forecast Signal bindings must be unique per Instrument and Slot".into(),
            );
        }
    }
    for (instrument_id, _, _) in &instruments {
        for slot in &declared_signal_slots {
            if !signals.contains_key(&(instrument_id.clone(), slot.name.clone())) {
                return Err(format!(
                    "Portfolio Forecast Signal is missing for {instrument_id}:{}",
                    slot.name
                ));
            }
        }
    }
    if signals.len() != instruments.len() * declared_signal_slots.len() {
        return Err(
            "Portfolio Forecast Signal bindings contain an unexpected Slot or Instrument".into(),
        );
    }

    let mut factors = Vec::with_capacity(request.factor_instances.len());
    let mut aliases = HashSet::new();
    for factor_request in &request.factor_instances {
        if !aliases.insert(factor_request.alias.clone()) {
            return Err("Portfolio Factor Instance aliases must be unique".into());
        }
        let package = source.package_for_user(&request.user_id, &factor_request.archive_sha256)?;
        if package.manifest.kind != ComponentKind::Factor {
            return Err("Portfolio external inputs require Factor Components".into());
        }
        if package.manifest.factor_scope != Some(adaq_component_tooling::FactorScope::TimeSeries) {
            return Err(
                "Portfolio Backtest currently requires time-series Factor Components".into(),
            );
        }
        let overrides = super::pipeline::resolve_factor_parameters(
            &strategy.manifest,
            &package.manifest,
            &request.strategy_parameters,
            &factor_request.parameters,
        )?;
        let parameters = component_parameters(&package.manifest, Some(&overrides))?;
        let path = source.runtime_component(&package)?;
        factors.push(QualifiedFactor {
            request: factor_request.clone(),
            package,
            parameters,
            path,
        });
    }
    let factor_inputs = factors
        .iter()
        .map(|factor| FactorInstancePlanInput {
            alias: &factor.request.alias,
            manifest: &factor.package.manifest,
            parameters: factor.parameters.clone(),
        })
        .collect::<Vec<_>>();
    let factor_paths = factors
        .iter()
        .map(|factor| factor.path.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let engine_identity = native_engine_identity().map_err(string)?;
    let mut feature_plans = BTreeMap::new();
    let mut materialized = Vec::with_capacity(instruments.len());
    for (instrument_id, _, bars) in &instruments {
        let instrument_signals = declared_signal_slots
            .iter()
            .map(|slot| {
                signals
                    .get(&(instrument_id.clone(), slot.name.clone()))
                    .cloned()
                    .ok_or_else(|| "Portfolio Forecast Signal is missing".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let signal_inputs = instrument_signals
            .iter()
            .map(|signal| SignalPlanInput {
                slot_name: &signal.slot,
                dataset_id: &signal.dataset_id,
                signal_name: &signal.signal_name,
                snapshot_id: instrument_snapshots
                    .get(instrument_id)
                    .expect("instrument snapshot was inserted above"),
                instrument_id: instrument_id.clone(),
                venue: &signal.src,
                bar_interval: &signal.interval,
                contract: signal.contract.clone(),
                producer_segments: signal.producer_segments.clone(),
                artifact_provenance: signal.artifact_provenance.clone(),
                evidence_state: &signal.evidence_state,
                component_lock: signal.component_lock.clone(),
            })
            .collect::<Vec<_>>();
        let plan = validate_and_freeze_feature_plan_with_bindings_and_parameters(
            &strategy.manifest,
            &strategy.archive_sha256,
            &engine_identity,
            &factor_inputs,
            &strategy_parameters,
            &signal_inputs,
        )
        .map_err(|error| {
            format!(
                "Portfolio Feature Plan validation failed: {:?}",
                error.issues
            )
        })?;
        let plan_json = String::from_utf8(plan.to_json()).map_err(string)?;
        feature_plans.insert(instrument_id.clone(), plan_json);
        let factor_runs = factors
            .iter()
            .zip(&factor_paths)
            .map(|(factor, path)| FactorRunRequest {
                alias: &factor.request.alias,
                path,
                manifest_feature_slots: &factor.package.manifest.feature_slots,
            })
            .collect::<Vec<_>>();
        let signal_runs = instrument_signals
            .iter()
            .map(|signal| SignalRunRequest {
                slot: &signal.slot,
                dataset_id: &signal.dataset_id,
                signal_name: &signal.signal_name,
                interval: universe.interval,
                rows: &signal.rows,
            })
            .collect::<Vec<_>>();
        let rows = materialize_feature_segment_with_signals(
            &plan,
            &factor_runs,
            &signal_runs,
            bars,
            RunLimits::default(),
        )
        .map_err(|error| error.to_string())?;
        let bars = bars
            .iter()
            .cloned()
            .map(|bar| (bar.open_time_ms, bar))
            .collect::<BTreeMap<_, _>>();
        if bars.len() != rows.len() {
            return Err("Portfolio Feature rows do not match the closed Bars".into());
        }
        let rows = bars.keys().copied().zip(rows).collect::<BTreeMap<_, _>>();
        materialized.push(PortfolioInstrumentFeatures {
            instrument_id: instrument_id.clone(),
            bars,
            rows,
        });
    }
    let feature_plan_hash = feature_plan_hash(&feature_plans)?;

    let risk_policy = request
        .risk_policy
        .clone()
        .ok_or("Portfolio Qualification requires a frozen Risk Policy")?;
    super::pipeline::validate_risk_policy(&risk_policy)?;
    let feature_slots = strategy
        .manifest
        .feature_slots
        .iter()
        .map(
            |slot| portfolio_strategy_abi::exports::adaq::strategy::portfolio_api::FeatureSlot {
                name: slot.name.clone(),
            },
        )
        .collect::<Vec<_>>();
    let loader = WasmLoader::with_limits(RunLimits::default());
    loader.load_portfolio_strategy_bytes(&strategy.wasm, feature_slots, &strategy_values)?;
    let times = materialized
        .first()
        .ok_or("Portfolio Backtest has no Instruments")?
        .bars
        .keys()
        .copied()
        .filter(|time| {
            materialized
                .iter()
                .all(|instrument| instrument.bars.contains_key(time))
        })
        .collect::<Vec<_>>();
    if times.is_empty() {
        return Err("Portfolio Backtest has no aligned Closed Bars".into());
    }
    let universe_id = universe.universe.universe_id.clone();
    let instrument_ids = materialized
        .iter()
        .map(|instrument| instrument.instrument_id.clone())
        .collect::<BTreeSet<_>>();
    let mut state = PortfolioState {
        cash: request.initial_quote_allocation,
        positions: BTreeMap::new(),
    };
    let execution_cost_rate = request.execution_profile.taker_fee_rate;
    let mut decisions = Vec::new();
    let mut pauses = Vec::new();
    for time in times {
        let mut rows = Vec::with_capacity(materialized.len());
        let mut prices = BTreeMap::new();
        let mut pause = None;
        for instrument in &materialized {
            let bar = instrument
                .bars
                .get(&time)
                .ok_or_else(|| format!("Portfolio price alignment failed for {time}"))?;
            if bar.open <= Decimal::ZERO {
                return Err(format!(
                    "Portfolio price is invalid for {}",
                    instrument.instrument_id
                ));
            }
            prices.insert(instrument.instrument_id.clone(), bar.open);
            match instrument
                .rows
                .get(&time)
                .ok_or_else(|| format!("Portfolio Feature row is missing for {time}"))?
            {
                MaterializedFeatureRow::Warmup => {
                    pause.get_or_insert_with(|| format!("warmup:{}", instrument.instrument_id));
                }
                MaterializedFeatureRow::MissingInput { slot, source } => {
                    pause.get_or_insert_with(|| {
                        format!("missing-input:{}:{slot}:{source}", instrument.instrument_id)
                    });
                }
                MaterializedFeatureRow::Present(values) => rows.push(
                    portfolio_strategy_abi::exports::adaq::strategy::portfolio_api::FeatureRow {
                        instrument_id: instrument.instrument_id.clone(),
                        values: values.clone(),
                    },
                ),
            }
        }
        if let Some(reason) = pause {
            pauses.push(RunPauseRecord {
                open_time_ms: time,
                reason,
            });
            continue;
        }
        mark_portfolio_to_market(&mut state, &prices).map_err(string)?;
        let frame =
            portfolio_strategy_abi::exports::adaq::strategy::portfolio_api::PortfolioFrame {
                decision_time_ms: time,
                universe_id: universe_id.clone(),
                rows,
                state: portfolio_state(&state),
            };
        let mut targets = loader.process_portfolio_strategy(vec![frame])?;
        let target = targets
            .pop()
            .ok_or("Portfolio Strategy returned no Target")?;
        let target = core_portfolio_target(target)?;
        if target.universe_id != universe_id {
            return Err("Portfolio Strategy Target Universe identity is invalid".into());
        }
        target.validate(&instrument_ids).map_err(string)?;
        let market = PortfolioMarketDecision {
            time_ms: time,
            prices,
            strategy_target: StrategyTarget {
                target,
                strategy_id: strategy.manifest.component_id.to_string(),
                input_provenance: BTreeMap::from([
                    ("universeSnapshotId".into(), universe_snapshot_id.clone()),
                    ("featurePlanHash".into(), feature_plan_hash.clone()),
                ]),
            },
        };
        apply_portfolio_market_decision(
            &mut state,
            market.clone(),
            request.initial_quote_allocation,
            &risk_policy,
            execution_cost_rate,
        )
        .map_err(string)?;
        decisions.push(market);
    }
    if decisions.is_empty() {
        return Err("Portfolio Backtest produced no complete Feature Frames".into());
    }
    let evidence = execute_portfolio_backtest(CoreRequest {
        initial_capital: request.initial_quote_allocation,
        risk_policy: risk_policy.clone(),
        execution_cost_rate,
        decisions,
    })
    .map_err(string)?;
    let mut signal_instances = request.signal_instances.clone();
    signal_instances.sort_by(|left, right| {
        (
            left.slot.as_str(),
            left.dataset_id.as_str(),
            left.signal_name.as_str(),
        )
            .cmp(&(
                right.slot.as_str(),
                right.dataset_id.as_str(),
                right.signal_name.as_str(),
            ))
    });
    let mut signal_locks = signals
        .iter()
        .map(|((instrument_id, _), signal)| PortfolioSignalLock {
            instrument_id: instrument_id.clone(),
            slot: signal.slot.clone(),
            dataset_id: signal.dataset_id.clone(),
            signal_name: signal.signal_name.clone(),
            evidence_state: signal.evidence_state.clone(),
        })
        .collect::<Vec<_>>();
    signal_locks.sort_by(|left, right| {
        (left.instrument_id.as_str(), left.slot.as_str())
            .cmp(&(right.instrument_id.as_str(), right.slot.as_str()))
    });
    let mut factor_instances = factors
        .iter()
        .map(|factor| NormalizedFactorInstance {
            alias: factor.request.alias.clone(),
            archive_sha256: factor.package.archive_sha256.clone(),
            parameters: super::pipeline::normalized_parameter_bindings(
                &factor.package.manifest,
                &factor.parameters,
            ),
        })
        .collect::<Vec<_>>();
    factor_instances.sort_by(|left, right| left.alias.cmp(&right.alias));
    let component_lock = std::iter::once(super::pipeline::component_lock_entry(&strategy))
        .chain(factor_instances.iter().map(|factor| {
            super::pipeline::component_lock_entry(
                &factors
                    .iter()
                    .find(|candidate| candidate.request.alias == factor.alias)
                    .expect("normalized Factor aliases were checked")
                    .package,
            )
        }))
        .collect();
    let provenance = PortfolioBacktestProvenance {
        snapshot_id: representative_snapshot.snapshot_id,
        universe_snapshot_id,
        universe_id,
        run_start_time_ms: start_time_ms,
        run_end_time_ms: end_time_ms,
        strategy_archive_sha256: strategy.archive_sha256.clone(),
        strategy_wasm_sha256: strategy.manifest.wasm_sha256.clone(),
        strategy_parameters,
        factor_instances,
        signal_instances,
        signal_locks,
        component_lock,
        feature_plans,
        feature_plan_hash,
        strategy_binding: request.strategy_binding,
        risk_policy: Some(risk_policy),
        initial_quote_allocation: request.initial_quote_allocation,
        execution_profile: request.execution_profile,
        seed: request.seed,
    };
    let request_hash =
        Sha256::digest(serde_json::to_vec(&(&request.user_id, &provenance)).map_err(string)?)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
    if let Some(stored) = load_qualified_existing(backtests, &request.user_id, &request_hash)? {
        return Ok(PortfolioBacktestView {
            run_id: format!("portfolio-{request_hash}"),
            reused_existing_run: true,
            metrics: stored.evidence.metrics.clone(),
            evidence: stored.evidence,
            pauses: stored.pauses,
            provenance: Some(stored.provenance),
        });
    }
    let run_id = format!("portfolio-{request_hash}");
    let stored = StoredPortfolioBacktest {
        evidence: evidence.clone(),
        pauses: pauses.clone(),
        provenance: provenance.clone(),
    };
    backtests.save_portfolio_run(
        &request.user_id,
        &run_id,
        &request_hash,
        &serde_json::to_string(&stored).map_err(string)?,
        &provenance.component_lock,
    )?;
    Ok(PortfolioBacktestView {
        run_id,
        reused_existing_run: false,
        metrics: evidence.metrics.clone(),
        evidence,
        pauses,
        provenance: Some(provenance),
    })
}

#[derive(Serialize, Deserialize)]
struct StoredPortfolioBacktest {
    evidence: BacktestEvidence,
    pauses: Vec<RunPauseRecord>,
    provenance: PortfolioBacktestProvenance,
}

fn load_qualified_existing(
    backtests: &Backtests,
    user_id: &str,
    request_hash: &str,
) -> Result<Option<StoredPortfolioBacktest>, String> {
    let database = backtests.source().database()?;
    let json = database
        .query_row(
            "SELECT evidence_json FROM portfolio_backtest_runs
             WHERE user_id = ?1 AND request_hash = ?2",
            params![user_id, request_hash],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(string)?;
    match json {
        None => Ok(None),
        Some(value) => match serde_json::from_str::<StoredPortfolioBacktest>(&value) {
            Ok(stored) => Ok(Some(stored)),
            Err(_error) if serde_json::from_str::<BacktestEvidence>(&value).is_ok() => {
                Err("Portfolio Backtest request identity collides with legacy evidence".into())
            }
            Err(error) => Err(string(error)),
        },
    }
}

pub(super) fn load_qualified_run(
    backtests: &Backtests,
    user_id: &str,
    run_id: &str,
) -> Result<PortfolioBacktestView, String> {
    crate::user::validate_user(user_id)?;
    let json = backtests
        .source()
        .database()?
        .query_row(
            "SELECT evidence_json FROM portfolio_backtest_runs
             WHERE user_id = ?1 AND run_id = ?2",
            params![user_id, run_id],
            |row| row.get::<_, String>(0),
        )
        .map_err(|_| "Portfolio Backtest was not found".to_owned())?;
    let stored: StoredPortfolioBacktest = serde_json::from_str(&json).map_err(string)?;
    Ok(PortfolioBacktestView {
        run_id: run_id.to_owned(),
        reused_existing_run: true,
        metrics: stored.evidence.metrics.clone(),
        evidence: stored.evidence,
        pauses: stored.pauses,
        provenance: Some(stored.provenance),
    })
}

fn portfolio_state(
    state: &PortfolioState,
) -> portfolio_strategy_abi::exports::adaq::strategy::portfolio_api::PortfolioState {
    portfolio_strategy_abi::exports::adaq::strategy::portfolio_api::PortfolioState {
        cash: state.cash.to_string(),
        positions: state
            .positions
            .iter()
            .map(|(instrument_id, position)| {
                portfolio_strategy_abi::exports::adaq::strategy::portfolio_api::Position {
                    instrument_id: instrument_id.clone(),
                    quantity: position.quantity.to_string(),
                    price: position.price.to_string(),
                }
            })
            .collect(),
    }
}

fn core_portfolio_target(
    target: portfolio_strategy_abi::exports::adaq::strategy::portfolio_api::PortfolioTarget,
) -> Result<adaq_backtest_core::PortfolioTarget, String> {
    Ok(adaq_backtest_core::PortfolioTarget {
        decision_time_ms: target.decision_time_ms,
        universe_id: target.universe_id,
        weights: target
            .weights
            .into_iter()
            .map(|weight| {
                Ok((
                    weight.instrument_id,
                    weight.weight.parse::<Decimal>().map_err(string)?,
                ))
            })
            .collect::<Result<_, String>>()?,
        cash_reserve: target.cash_reserve.parse::<Decimal>().map_err(string)?,
    })
}

fn feature_plan_hash(feature_plans: &BTreeMap<String, String>) -> Result<String, String> {
    Ok(
        Sha256::digest(serde_json::to_vec(feature_plans).map_err(string)?)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    )
}

fn load_existing(
    backtests: &Backtests,
    user_id: &str,
    request_hash: &str,
) -> Result<Option<BacktestEvidence>, String> {
    let database = backtests.source().database()?;
    let json = database
        .query_row(
            "SELECT evidence_json FROM portfolio_backtest_runs
             WHERE user_id = ?1 AND request_hash = ?2",
            params![user_id, request_hash],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(string)?;
    match json {
        None => Ok(None),
        Some(value) => match serde_json::from_str::<BacktestEvidence>(&value) {
            Ok(evidence) => Ok(Some(evidence)),
            Err(_) => serde_json::from_str::<StoredPortfolioBacktest>(&value)
                .map(|stored| Some(stored.evidence))
                .map_err(string),
        },
    }
}

fn decimal(value: &str, field: &str) -> Result<Decimal, String> {
    value
        .parse::<Decimal>()
        .map_err(|_| format!("Portfolio Backtest {field} is invalid"))
}
