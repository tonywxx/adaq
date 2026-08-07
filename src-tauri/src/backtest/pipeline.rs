use std::collections::{BTreeMap, HashMap};

use adaq_backtest_core::{MarketDataSnapshot, SpotSimulator, TargetDecision as SimulationDecision};
use adaq_component_tooling::{
    ComponentKind, ComponentManifest, ComponentPackage, ComponentParameterValue,
    FactorInstancePlanInput, FeatureSlotSource, FrozenFeaturePlan, RunLimits, SignalPlanInput,
    component_parameters, native_engine_identity,
    validate_and_freeze_feature_plan_with_bindings_and_parameters,
};
use adaq_data_core::{BarGap, OhlcvBar};
use sha2::{Digest, Sha256};

use super::{
    BacktestRun, BacktestRunProvenance, BacktestRunRequest, BacktestRunView, Backtests,
    ComponentLockEntry, FactorParameterBinding, NormalizedFactorInstance, NormalizedParameter,
    RunPauseRecord, SignalDatasetLock, string,
};
use crate::run_engine::{
    FactorRunRequest, PositionMode, RunEngine, RunPauseReason, RunRequest, SignalRunRequest,
    SignalRunRow,
};

pub(super) struct PreparedBacktest {
    pub strategy: ComponentPackage,
    pub strategy_parameters: Vec<ComponentParameterValue>,
    pub factor_packages: Vec<ComponentPackage>,
    pub signals: Vec<PreparedSignal>,
    pub plan: FrozenFeaturePlan,
    pub provenance: BacktestRunProvenance,
    pub component_lock: Vec<ComponentLockEntry>,
    pub run_id: String,
    pub snapshot: MarketDataSnapshot,
    pub bars: Vec<OhlcvBar>,
    pub gaps: Vec<BarGap>,
}

pub(super) struct PreparedSignal {
    pub slot: String,
    pub dataset_id: String,
    pub signal_name: String,
    pub rows: Vec<SignalRunRow>,
}

pub(super) fn execute(
    backtests: &Backtests,
    request: BacktestRunRequest,
) -> Result<BacktestRunView, String> {
    let prepared = prepare(backtests, &request)?;
    if let Ok(existing) = backtests.load_run(&request.user_id, &prepared.run_id) {
        return Ok(full_run_view(&existing));
    }
    let PreparedBacktest {
        strategy,
        strategy_parameters,
        factor_packages,
        signals,
        plan,
        provenance,
        component_lock,
        run_id,
        snapshot,
        bars,
        gaps,
    } = prepared;
    let source = backtests.source();
    let strategy_path = source.runtime_component(&strategy)?;
    let factor_paths = factor_packages
        .iter()
        .map(|package| source.runtime_component(package))
        .collect::<Result<Vec<_>, _>>()?;
    let strategy_path = strategy_path.to_string_lossy();
    let factor_paths = factor_paths
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let factors = request
        .factor_instances
        .iter()
        .zip(&factor_paths)
        .map(|(factor, path)| FactorRunRequest {
            alias: &factor.alias,
            path,
        })
        .collect::<Vec<_>>();
    let signal_runs = signals
        .iter()
        .map(|signal| SignalRunRequest {
            slot: &signal.slot,
            dataset_id: &signal.dataset_id,
            signal_name: &signal.signal_name,
            interval: snapshot.interval,
            rows: &signal.rows,
        })
        .collect::<Vec<_>>();
    let engine_result = RunEngine::execute(&RunRequest {
        strategy_path: &strategy_path,
        strategy_parameters: &strategy_parameters,
        factors: &factors,
        signals: &signal_runs,
        bars: &bars,
        gaps: &gaps,
        plan: &plan,
        position_mode: PositionMode::LongOnly,
        limits: RunLimits::default(),
    })
    .map_err(|error| error.to_string())?;
    let bars = engine_result.bars;
    let decisions = engine_result
        .decisions
        .into_iter()
        .map(|decision| SimulationDecision {
            open_time_ms: decision.open_time_ms,
            target_exposure: decision.target_exposure,
        })
        .collect::<Vec<_>>();
    let result = SpotSimulator::execute(
        &bars,
        &gaps,
        &decisions,
        request.initial_quote_allocation,
        &request.execution_profile,
    )
    .map_err(string)?;
    let run = BacktestRun {
        run_id: run_id.clone(),
        plan_hash: engine_result.plan_hash,
        snapshot,
        bars,
        decisions,
        pauses: engine_result
            .pauses
            .iter()
            .map(|pause| RunPauseRecord {
                open_time_ms: pause.open_time_ms,
                reason: match &pause.reason {
                    RunPauseReason::Warmup => "warmup".into(),
                    RunPauseReason::MissingInput { slot, source } => {
                        format!("missing-input:{slot}:{source}")
                    }
                },
            })
            .collect(),
        result,
        component_lock,
        provenance: Some(provenance),
    };
    backtests.save_run(&request.user_id, &run_id, &run)?;
    Ok(full_run_view(&run))
}

pub(super) fn prepare(
    backtests: &Backtests,
    request: &BacktestRunRequest,
) -> Result<PreparedBacktest, String> {
    let source = backtests.source();
    SpotSimulator::validate_execution_inputs(
        request.initial_quote_allocation,
        &request.execution_profile,
    )
    .map_err(string)?;
    let strategy = source.package_for_user(&request.user_id, &request.strategy_archive_sha256)?;
    if !matches!(strategy.manifest.kind, ComponentKind::Strategy) {
        return Err("Backtest requires a Strategy Component".into());
    }
    let strategy_parameters =
        component_parameters(&strategy.manifest, Some(&request.strategy_parameters))?;
    let frozen_strategy_parameters =
        normalized_parameters(&strategy.manifest, &strategy_parameters);
    let (snapshot, mut bars) = source.snapshot_for_user(&request.user_id, &request.snapshot_id)?;
    let run_start_time_ms = request.run_start_time_ms.unwrap_or(snapshot.start_time_ms);
    let run_end_time_ms = request.run_end_time_ms.unwrap_or(snapshot.end_time_ms);
    if run_start_time_ms > run_end_time_ms {
        return Err("Backtest Run window must be a valid inclusive Bar-open range".into());
    }
    if run_start_time_ms < snapshot.start_time_ms || run_end_time_ms > snapshot.end_time_ms {
        return Err("Backtest Run window must be a subset of the exact Dataset Snapshot".into());
    }
    if ![run_start_time_ms, run_end_time_ms]
        .into_iter()
        .all(|boundary| bars.iter().any(|bar| bar.open_time_ms == boundary))
    {
        return Err("Backtest Run window boundaries must match exact Closed Bar open times".into());
    }
    bars.retain(|bar| bar.open_time_ms >= run_start_time_ms && bar.open_time_ms <= run_end_time_ms);
    if bars.is_empty() {
        return Err("Backtest Run window contains no Closed Bars".into());
    }
    let factor_packages = request
        .factor_instances
        .iter()
        .map(|factor| {
            let package = source.package_for_user(&request.user_id, &factor.archive_sha256)?;
            if package.manifest.kind != ComponentKind::Factor {
                return Err("External Feature Slots require Factor Components".into());
            }
            Ok((factor, package))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let factor_parameters = factor_packages
        .iter()
        .map(|(factor, package)| {
            let parameters = resolve_factor_parameters(
                &strategy.manifest,
                &package.manifest,
                &request.strategy_parameters,
                &factor.parameters,
            )?;
            component_parameters(&package.manifest, Some(&parameters))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let factor_inputs = factor_packages
        .iter()
        .zip(&factor_parameters)
        .map(|((factor, package), parameters)| FactorInstancePlanInput {
            alias: &factor.alias,
            manifest: &package.manifest,
            parameters: parameters.clone(),
        })
        .collect::<Vec<_>>();
    let declared_signal_slots = strategy
        .manifest
        .feature_slots
        .iter()
        .filter(|slot| matches!(slot.source, FeatureSlotSource::Signal { .. }))
        .collect::<Vec<_>>();
    if request.signal_instances.len() != declared_signal_slots.len()
        || request.signal_instances.iter().any(|binding| {
            request
                .signal_instances
                .iter()
                .filter(|candidate| candidate.slot == binding.slot)
                .count()
                != 1
        })
    {
        return Err("Signal bindings must match declared Forecast Signal Slots exactly".into());
    }
    let mut selected_dataset_ids = request
        .signal_instances
        .iter()
        .map(|binding| binding.dataset_id.clone())
        .collect::<Vec<_>>();
    selected_dataset_ids.sort();
    selected_dataset_ids.dedup();
    let datasets = source.signal_datasets(&request.user_id, true, Some(&selected_dataset_ids))?;
    let selected_signals = declared_signal_slots
        .iter()
        .map(|slot| {
            let binding = request
                .signal_instances
                .iter()
                .find(|binding| binding.slot == slot.name)
                .ok_or("A Forecast Signal Slot is not bound")?;
            let dataset = datasets
                .iter()
                .find(|dataset| dataset.dataset_id == binding.dataset_id)
                .ok_or("Forecast Signal Dataset is not available to this User")?;
            if dataset.snapshot_id != snapshot.snapshot_id
                || dataset.src != snapshot.src
                || dataset.code != snapshot.code
                || dataset.interval != snapshot.interval.as_str()
            {
                return Err(
                    "Signal Dataset Snapshot, Instrument, Venue, and Bar Interval must match exactly"
                        .into(),
                );
            }
            let output_index = dataset
                .outputs
                .iter()
                .position(|output| output.name == binding.signal_name)
                .ok_or("Selected Forecast Signal was not found in the Dataset")?;
            let FeatureSlotSource::Signal {
                prediction_kind,
                forecast_target,
                value_scale,
                horizon_bars,
            } = &slot.source
            else {
                unreachable!()
            };
            let output = &dataset.outputs[output_index];
            if output.prediction_kind != *prediction_kind
                || output.forecast_target != *forecast_target
                || output.value_scale != *value_scale
                || output.horizon_bars != *horizon_bars
            {
                return Err("Selected Forecast Signal is not semantically compatible".into());
            }
            Ok((binding, dataset, output_index))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let signal_inputs = selected_signals
        .iter()
        .map(|(binding, dataset, output_index)| SignalPlanInput {
            slot_name: &binding.slot,
            dataset_id: &dataset.dataset_id,
            signal_name: &binding.signal_name,
            snapshot_id: &dataset.snapshot_id,
            instrument_id: format!("{}:{}", dataset.src, dataset.code),
            venue: &dataset.src,
            bar_interval: &dataset.interval,
            contract: dataset.outputs[*output_index].clone(),
            producer_segments: dataset.producer_segments.clone(),
            artifact_provenance: dataset.artifact_provenance.clone(),
            evidence_state: &dataset.evidence_state,
            component_lock: dataset.component_lock.clone(),
        })
        .collect::<Vec<_>>();
    let engine_identity = native_engine_identity().map_err(|error| error.to_string())?;
    let plan = validate_and_freeze_feature_plan_with_bindings_and_parameters(
        &strategy.manifest,
        &strategy.archive_sha256,
        &engine_identity,
        &factor_inputs,
        &frozen_strategy_parameters,
        &signal_inputs,
    )
    .map_err(|error| format!("Feature Plan validation failed: {:?}", error.issues))?;
    let mut factor_instances = factor_packages
        .iter()
        .zip(&factor_parameters)
        .map(|((factor, package), parameters)| NormalizedFactorInstance {
            alias: factor.alias.clone(),
            archive_sha256: package.archive_sha256.clone(),
            parameters: normalized_parameter_bindings(&package.manifest, parameters),
        })
        .collect::<Vec<_>>();
    factor_instances.sort_by(|left, right| left.alias.cmp(&right.alias));
    if factor_instances
        .windows(2)
        .any(|pair| pair[0].alias == pair[1].alias)
    {
        return Err("Factor Instance aliases must be unique".into());
    }
    let component_lock = std::iter::once(component_lock_entry(&strategy))
        .chain(factor_instances.iter().map(|factor| {
            component_lock_entry(
                &factor_packages
                    .iter()
                    .find(|(request, _)| request.alias == factor.alias)
                    .expect("unique Factor aliases were checked")
                    .1,
            )
        }))
        .collect::<Vec<_>>();
    let mut signal_instances = request.signal_instances.clone();
    signal_instances.sort_by(|left, right| left.slot.cmp(&right.slot));
    let dataset_lock = selected_signals
        .iter()
        .map(|(binding, dataset, _)| SignalDatasetLock {
            slot: binding.slot.clone(),
            dataset_id: dataset.dataset_id.clone(),
            signal_name: binding.signal_name.clone(),
            evidence_state: dataset.evidence_state.clone(),
        })
        .collect::<Vec<_>>();
    let provenance = BacktestRunProvenance {
        normalized_request: super::NormalizedBacktestRunRequest {
            snapshot_id: request.snapshot_id.clone(),
            run_start_time_ms: Some(run_start_time_ms),
            run_end_time_ms: Some(run_end_time_ms),
            strategy_archive_sha256: strategy.archive_sha256.clone(),
            strategy_parameters: frozen_strategy_parameters,
            factor_instances,
            signal_instances,
            initial_quote_allocation: request.initial_quote_allocation,
            execution_profile: request.execution_profile.clone(),
            seed: request.seed,
        },
        feature_plan_json: String::from_utf8(plan.to_json()).map_err(string)?,
        feature_plan_hash: plan.plan_hash().into(),
        component_lock: component_lock.clone(),
        dataset_lock,
        architecture: plan.architecture(),
        indicator_engine_build_identity: super::IndicatorEngineBuildIdentity {
            engine_version: engine_identity.engine_version,
            ta_lib_version: engine_identity.ta_lib_version,
            ta_source_sha256: engine_identity.ta_source_sha256,
            catalog_version: engine_identity.catalog_version,
            wrapper_sha256: engine_identity.wrapper_sha256,
            target_triple: engine_identity.target_triple,
            compiler_and_flags_sha256: engine_identity.compiler_and_flags_sha256,
            engine_build_id: engine_identity.engine_build_id,
        },
        backtest_engine_version: format!("adaq-backtest-engine@{}", env!("CARGO_PKG_VERSION")),
        seed: request.seed,
    };
    validate_provenance(&provenance)?;
    let run_id = fingerprint(&request.user_id, &provenance)?;
    let gaps = snapshot
        .gaps
        .iter()
        .filter(|gap| gap.end_time_ms > run_start_time_ms && gap.start_time_ms <= run_end_time_ms)
        .map(|gap| BarGap {
            start_time_ms: gap.start_time_ms,
            end_time_ms: gap.end_time_ms,
        })
        .collect::<Vec<_>>();
    Ok(PreparedBacktest {
        strategy,
        strategy_parameters,
        factor_packages: factor_packages
            .into_iter()
            .map(|(_, package)| package)
            .collect(),
        signals: selected_signals
            .into_iter()
            .map(|(binding, dataset, output_index)| PreparedSignal {
                slot: binding.slot.clone(),
                dataset_id: dataset.dataset_id.clone(),
                signal_name: binding.signal_name.clone(),
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
            })
            .collect(),
        plan,
        provenance,
        component_lock,
        run_id,
        snapshot,
        bars,
        gaps,
    })
}

pub(super) fn validate_provenance(provenance: &BacktestRunProvenance) -> Result<(), String> {
    let identity = adaq_component_tooling::EngineIdentity {
        engine_version: provenance
            .indicator_engine_build_identity
            .engine_version
            .clone(),
        ta_lib_version: provenance
            .indicator_engine_build_identity
            .ta_lib_version
            .clone(),
        ta_source_sha256: provenance
            .indicator_engine_build_identity
            .ta_source_sha256
            .clone(),
        catalog_version: provenance
            .indicator_engine_build_identity
            .catalog_version
            .clone(),
        wrapper_sha256: provenance
            .indicator_engine_build_identity
            .wrapper_sha256
            .clone(),
        target_triple: provenance
            .indicator_engine_build_identity
            .target_triple
            .clone(),
        compiler_and_flags_sha256: provenance
            .indicator_engine_build_identity
            .compiler_and_flags_sha256
            .clone(),
        engine_build_id: provenance
            .indicator_engine_build_identity
            .engine_build_id
            .clone(),
    };
    let frozen_plan =
        FrozenFeaturePlan::load_for_engine(provenance.feature_plan_json.as_bytes(), &identity)
            .map_err(|_| "Backtest Run provenance has an invalid frozen Feature Plan")?;
    let plan: serde_json::Value =
        serde_json::from_str(&provenance.feature_plan_json).map_err(string)?;
    let content = plan.as_object().ok_or("Feature Plan is invalid")?;
    if content.get("planHash").and_then(serde_json::Value::as_str)
        != Some(&provenance.feature_plan_hash)
        || frozen_plan.plan_hash() != provenance.feature_plan_hash
        || content
            .get("consumerPackageSha256")
            .and_then(serde_json::Value::as_str)
            != Some(&provenance.normalized_request.strategy_archive_sha256)
        || content
            .get("engineBuildId")
            .and_then(serde_json::Value::as_str)
            != Some(&provenance.indicator_engine_build_identity.engine_build_id)
    {
        return Err("Backtest Run provenance has inconsistent hashes or engine identity".into());
    }
    let requested_hashes = std::iter::once(&provenance.normalized_request.strategy_archive_sha256)
        .chain(
            provenance
                .normalized_request
                .factor_instances
                .iter()
                .map(|factor| &factor.archive_sha256),
        )
        .collect::<Vec<_>>();
    let locked_hashes = provenance
        .component_lock
        .iter()
        .map(|component| &component.archive_sha256)
        .collect::<Vec<_>>();
    let mut plan_aliases = content
        .get("factors")
        .and_then(serde_json::Value::as_array)
        .ok_or("Feature Plan is missing Factor bindings")?
        .iter()
        .map(|factor| {
            factor
                .get("alias")
                .and_then(serde_json::Value::as_str)
                .ok_or("Feature Plan has an invalid Factor binding")
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut request_aliases = provenance
        .normalized_request
        .factor_instances
        .iter()
        .map(|factor| factor.alias.as_str())
        .collect::<Vec<_>>();
    plan_aliases.sort_unstable();
    request_aliases.sort_unstable();
    let mut planned_signals = content
        .get("slots")
        .and_then(serde_json::Value::as_array)
        .ok_or("Feature Plan is missing ordered Slots")?
        .iter()
        .filter_map(|slot| {
            let source = slot.get("source")?;
            (source.get("kind")?.as_str()? == "signal").then(|| {
                Some((
                    slot.get("name")?.as_str()?.to_owned(),
                    source.get("dataset_id")?.as_str()?.to_owned(),
                    source.get("signal_name")?.as_str()?.to_owned(),
                ))
            })?
        })
        .collect::<Vec<_>>();
    let mut requested_signals = provenance
        .normalized_request
        .signal_instances
        .iter()
        .map(|signal| {
            (
                signal.slot.clone(),
                signal.dataset_id.clone(),
                signal.signal_name.clone(),
            )
        })
        .collect::<Vec<_>>();
    let mut locked_signals = provenance
        .dataset_lock
        .iter()
        .map(|signal| {
            (
                signal.slot.clone(),
                signal.dataset_id.clone(),
                signal.signal_name.clone(),
            )
        })
        .collect::<Vec<_>>();
    planned_signals.sort();
    requested_signals.sort();
    locked_signals.sort();
    let plan_factor_parameters = frozen_plan
        .factors()
        .map(|factor| {
            (
                factor.alias,
                factor
                    .parameters
                    .iter()
                    .map(parameter_value)
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    if requested_hashes != locked_hashes
        || plan_aliases != request_aliases
        || provenance
            .normalized_request
            .factor_instances
            .iter()
            .any(|factor| {
                plan_factor_parameters.get(factor.alias.as_str())
                    != Some(
                        &factor
                            .parameters
                            .iter()
                            .map(|parameter| parameter.value.clone())
                            .collect(),
                    )
            })
        || locked_hashes.iter().any(|hash| !is_sha256(hash))
        || planned_signals != requested_signals
        || requested_signals != locked_signals
        || provenance
            .dataset_lock
            .iter()
            .any(|signal| !is_sha256(&signal.dataset_id) || signal.evidence_state.is_empty())
        || frozen_plan.architecture() != provenance.architecture
        || provenance.seed != provenance.normalized_request.seed
    {
        return Err("Backtest Run provenance has inconsistent Component Locks or bindings".into());
    }
    Ok(())
}

pub(super) fn full_run_view(run: &BacktestRun) -> BacktestRunView {
    run_view(run, i64::MIN, i64::MAX, 2_000)
}

pub(super) fn run_view(
    run: &BacktestRun,
    start: i64,
    end: i64,
    max_points: usize,
) -> BacktestRunView {
    let mut result = run.result.clone();
    result.equity = aggregate_equity(&result.equity, start, end, max_points);
    result.benchmark_equity = aggregate_equity(&result.benchmark_equity, start, end, max_points);
    result
        .fills
        .retain(|fill| fill.open_time_ms >= start && fill.open_time_ms < end);
    result
        .orders
        .retain(|order| order.created_time_ms >= start && order.created_time_ms < end);
    result.fills.truncate(max_points);
    result.orders.truncate(max_points);
    BacktestRunView {
        run_id: run.run_id.clone(),
        plan_hash: run.plan_hash.clone(),
        snapshot: run.snapshot.clone(),
        bars: aggregate_bars(&run.bars, start, end, max_points),
        decisions: run
            .decisions
            .iter()
            .filter(|decision| decision.open_time_ms >= start && decision.open_time_ms < end)
            .cloned()
            .collect(),
        pauses: run
            .pauses
            .iter()
            .filter(|pause| pause.open_time_ms >= start && pause.open_time_ms < end)
            .cloned()
            .collect(),
        result,
        component_lock: run.component_lock.clone(),
        provenance: run.provenance.clone(),
    }
}

pub(super) fn fingerprint(
    user_id: &str,
    provenance: &BacktestRunProvenance,
) -> Result<String, String> {
    let digest = Sha256::digest(serde_json::to_vec(&(user_id, provenance)).map_err(string)?);
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn normalized_parameters(
    manifest: &ComponentManifest,
    values: &[ComponentParameterValue],
) -> BTreeMap<String, String> {
    manifest
        .parameters
        .iter()
        .zip(values)
        .map(|(definition, value)| (definition.name.clone(), parameter_value(value)))
        .collect()
}

fn normalized_parameter_bindings(
    manifest: &ComponentManifest,
    values: &[ComponentParameterValue],
) -> Vec<NormalizedParameter> {
    manifest
        .parameters
        .iter()
        .zip(values)
        .map(|(definition, value)| NormalizedParameter {
            name: definition.name.clone(),
            value: parameter_value(value),
        })
        .collect()
}

fn parameter_value(value: &ComponentParameterValue) -> String {
    match value {
        ComponentParameterValue::Decimal(value) | ComponentParameterValue::String(value) => {
            value.clone()
        }
        ComponentParameterValue::Integer(value) => value.to_string(),
        ComponentParameterValue::Boolean(value) => value.to_string(),
    }
}

fn component_lock_entry(package: &ComponentPackage) -> ComponentLockEntry {
    ComponentLockEntry {
        component_id: package.manifest.component_id.to_string(),
        version: package.manifest.version.to_string(),
        archive_sha256: package.archive_sha256.clone(),
        wasm_sha256: package.manifest.wasm_sha256.clone(),
    }
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn resolve_factor_parameters(
    strategy: &ComponentManifest,
    factor: &ComponentManifest,
    strategy_overrides: &HashMap<String, String>,
    bindings: &HashMap<String, FactorParameterBinding>,
) -> Result<HashMap<String, String>, String> {
    if bindings.keys().any(|name| {
        !factor
            .parameters
            .iter()
            .any(|parameter| parameter.name == *name)
    }) {
        return Err("Unknown Factor Parameter binding".into());
    }
    bindings
        .iter()
        .map(|(name, binding)| match binding {
            FactorParameterBinding::Literal(value) => Ok((name.clone(), value.clone())),
            FactorParameterBinding::StrategyParameter { strategy_parameter } => {
                let parameter = strategy
                    .parameters
                    .iter()
                    .find(|parameter| parameter.name == *strategy_parameter)
                    .ok_or_else(|| {
                        format!("Unknown Strategy Parameter reference: {strategy_parameter}")
                    })?;
                let target = factor
                    .parameters
                    .iter()
                    .find(|parameter| parameter.name == *name)
                    .ok_or_else(|| format!("Unknown Factor Parameter binding: {name}"))?;
                if parameter.parameter_type != target.parameter_type {
                    return Err(format!(
                        "Strategy Parameter reference type does not match Factor Parameter: {name}"
                    ));
                }
                Ok((
                    name.clone(),
                    strategy_overrides
                        .get(strategy_parameter)
                        .unwrap_or(&parameter.default_value)
                        .clone(),
                ))
            }
        })
        .collect()
}

fn aggregate_bars(bars: &[OhlcvBar], start: i64, end: i64, max_points: usize) -> Vec<OhlcvBar> {
    let filtered = bars
        .iter()
        .filter(|bar| bar.open_time_ms >= start && bar.open_time_ms < end)
        .collect::<Vec<_>>();
    let chunk = filtered.len().div_ceil(max_points).max(1);
    filtered
        .chunks(chunk)
        .map(|bars| OhlcvBar {
            open_time_ms: bars[0].open_time_ms,
            open: bars[0].open,
            high: bars.iter().map(|bar| bar.high).max().unwrap(),
            low: bars.iter().map(|bar| bar.low).min().unwrap(),
            close: bars.last().unwrap().close,
            base_volume: bars.iter().map(|bar| bar.base_volume).sum(),
            quote_volume: bars.iter().map(|bar| bar.quote_volume).sum(),
        })
        .collect()
}

fn aggregate_equity(
    points: &[adaq_backtest_core::EquityPoint],
    start: i64,
    end: i64,
    max_points: usize,
) -> Vec<adaq_backtest_core::EquityPoint> {
    let filtered = points
        .iter()
        .filter(|point| point.open_time_ms >= start && point.open_time_ms < end)
        .collect::<Vec<_>>();
    let chunk = filtered.len().div_ceil(max_points).max(1);
    filtered
        .chunks(chunk)
        .map(|points| {
            let mut point = (*points.last().unwrap()).clone();
            point.drawdown = points.iter().map(|value| value.drawdown).min().unwrap();
            point
        })
        .collect()
}
