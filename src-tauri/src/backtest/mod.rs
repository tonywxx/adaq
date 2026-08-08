//! Backtest Run module.
//!
//! One deep, Tauri-independent module owning the Backtest Run pipeline:
//! preflight, Run execution (RunEngine output mapping, simulation
//! invocation, and Run persistence), listing, retrieval, chart data,
//! execution data, and deletion. The module owns the Run table and both
//! bridge tables (Run↔Component and Run↔Signal-Dataset), exposes a narrow
//! component-lock query for the Component Library, and the summary-for-user
//! and reset-for-user hooks the composition root calls. Cross-domain reads
//! — Market Data Snapshot reads, Component Package reads, Signal Dataset
//! reads through the forecast_signal_dataset-owned path, and Validation Report references — flow
//! through Source traits defined here and implemented by the composition
//! root.

mod pipeline;

#[cfg(test)]
mod tests;

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, MutexGuard},
};

use adaq_backtest_core::{ExecutionProfile, MarketDataSnapshot};
use adaq_component_tooling::{ComponentPackage, StrategyArchitecture};
use adaq_data_core::{BarInterval, OhlcvBar};
use rusqlite::{Connection, Transaction, params};
use serde::{Deserialize, Serialize};

use crate::{forecast_signal_dataset::BacktestSignalDataset, user::validate_user};

const RUN_HISTORY_PAGE_SIZE: usize = 10;

/// Entitlement-scoped Market Data Snapshot reads consumed by Backtest Runs.
/// Implemented by the composition root over the Market Data Snapshot
/// module's read hook; the complete Local Research state is never passed
/// in.
pub(crate) trait SnapshotReadSource: Send + Sync {
    fn snapshot_for_user(
        &self,
        user_id: &str,
        snapshot_id: &str,
    ) -> Result<(MarketDataSnapshot, Vec<OhlcvBar>), String>;
}

/// Entitlement-scoped Component Package reads consumed by Backtest Runs,
/// including the runtime materialization the Run engine executes.
/// Implemented by the composition root; the complete Local Research state
/// is never passed in.
pub(crate) trait ComponentPackageSource: Send + Sync {
    fn package_for_user(
        &self,
        user_id: &str,
        archive_sha256: &str,
    ) -> Result<ComponentPackage, String>;
    fn runtime_component(&self, package: &ComponentPackage) -> Result<PathBuf, String>;
}

/// The concrete local dependencies composed into Backtest Runs. The
/// complete Local Research state is never passed in; only database access,
/// Snapshot reads, Component Package access, Signal Dataset reads through
/// the forecast_signal_dataset-owned path, and the Validation Report reference check are shared.
pub(crate) trait BacktestSource:
    SnapshotReadSource + ComponentPackageSource + Send + Sync
{
    fn database(&self) -> Result<MutexGuard<'_, Connection>, String>;
    fn signal_datasets(
        &self,
        user_id: &str,
        include_rows: bool,
        dataset_ids: Option<&[String]>,
    ) -> Result<Vec<BacktestSignalDataset>, String>;
    fn validation_report_references_run(&self, user_id: &str, run_id: &str)
    -> Result<bool, String>;
}

/// The Backtest Run evidence count the Local Data summary reports for one
/// User.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BacktestSummary {
    pub run_count: u64,
}

/// The Backtest Run interface: preflight, execution, listing, retrieval,
/// chart data, execution data, and deletion, the narrow component-lock
/// query the Component Library consumes, and the summary-for-user and
/// reset-for-user hooks the composition root calls.
#[derive(Clone)]
pub(crate) struct Backtests(pub(super) Arc<dyn BacktestSource>);

impl Backtests {
    /// Creates the module and initializes the Backtest Run and bridge
    /// table schema, which live inside this module.
    pub(crate) fn open(source: Arc<dyn BacktestSource>) -> Result<Self, String> {
        source
            .database()?
            .execute_batch(
                "PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS backtest_runs (
                run_id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                result_json TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS backtest_run_components (
                run_id TEXT NOT NULL,
                archive_sha256 TEXT NOT NULL,
                PRIMARY KEY(run_id, archive_sha256),
                FOREIGN KEY(run_id) REFERENCES backtest_runs(run_id) ON DELETE CASCADE,
                FOREIGN KEY(archive_sha256) REFERENCES component_content(archive_sha256)
             );
             CREATE TABLE IF NOT EXISTS backtest_run_signal_datasets (
                run_id TEXT NOT NULL,
                dataset_id TEXT NOT NULL,
                signal_name TEXT NOT NULL,
                PRIMARY KEY(run_id, dataset_id, signal_name),
                FOREIGN KEY(run_id) REFERENCES backtest_runs(run_id) ON DELETE CASCADE,
                FOREIGN KEY(dataset_id) REFERENCES signal_dataset_content(dataset_id)
             );",
            )
            .map_err(string)?;
        Ok(Self(source))
    }

    pub(super) fn source(&self) -> &Arc<dyn BacktestSource> {
        &self.0
    }

    /// Validates and normalizes one Run request without executing it,
    /// deriving the deterministic Run identity and reporting whether an
    /// identical Run already exists.
    pub(crate) fn preflight(
        &self,
        request: &BacktestRunRequest,
    ) -> Result<BacktestPreflight, String> {
        let prepared = pipeline::prepare(self, request)?;
        Ok(BacktestPreflight {
            run_id: prepared.run_id.clone(),
            reuses_existing_run: self.load_run(&request.user_id, &prepared.run_id).is_ok(),
            snapshot: prepared.snapshot,
            normalized_request: prepared.provenance.normalized_request,
            feature_plan: serde_json::from_str(&prepared.provenance.feature_plan_json)
                .map_err(string)?,
            component_lock: prepared.component_lock,
            dataset_lock: prepared.provenance.dataset_lock,
            architecture: prepared.provenance.architecture,
        })
    }

    /// Prepares, executes, simulates, and persists one Backtest Run for
    /// one User, reusing the exact Run already recorded for an identical
    /// provenance.
    pub(crate) fn run(&self, request: BacktestRunRequest) -> Result<BacktestRunView, String> {
        pipeline::execute(self, request)
    }

    /// Pages one User's Run history, optionally filtered by one exact
    /// Instrument.
    pub(crate) fn list(&self, request: &BacktestListRequest) -> Result<BacktestRunPage, String> {
        validate_user(&request.user_id)?;
        let instrument_valid = match (&request.src, &request.code) {
            (None, None) => true,
            (Some(src), Some(code)) => !src.trim().is_empty() && !code.trim().is_empty(),
            _ => false,
        };
        if request.page == 0 || !instrument_valid {
            return Err("Backtest Run history request is invalid".into());
        }
        let src = request.src.as_deref();
        let code = request.code.as_deref();
        let database = self.0.database()?;
        let filter = "user_id = ?1
            AND (?2 IS NULL OR (
                json_extract(result_json, '$.snapshot.src') = ?2
                AND json_extract(result_json, '$.snapshot.code') = ?3
            ))";
        let total = database
            .query_row(
                &format!("SELECT COUNT(*) FROM backtest_runs WHERE {filter}"),
                params![request.user_id, src, code],
                |row| row.get::<_, i64>(0),
            )
            .map_err(string)?
            .try_into()
            .map_err(|_| "Backtest Run history count is invalid")?;
        let offset = request
            .page
            .checked_sub(1)
            .and_then(|value| value.checked_mul(RUN_HISTORY_PAGE_SIZE))
            .ok_or_else(|| "Backtest Run history page is too large".to_owned())?;
        let mut statement = database
            .prepare(&format!(
                "SELECT run_id, created_at,
                    json_extract(result_json, '$.snapshot.snapshotId'),
                    json_extract(result_json, '$.snapshot.code'),
                    json_extract(result_json, '$.snapshot.interval'),
                    json_extract(result_json, '$.snapshot.barCount'),
                    json_extract(result_json, '$.result.metrics.totalReturn')
                 FROM backtest_runs WHERE {filter}
                 ORDER BY created_at DESC, run_id DESC LIMIT ?4 OFFSET ?5"
            ))
            .map_err(string)?;
        let items = statement
            .query_map(
                params![
                    request.user_id,
                    src,
                    code,
                    RUN_HISTORY_PAGE_SIZE as i64,
                    offset as i64
                ],
                |row| {
                    let interval_text = row.get::<_, String>(4)?;
                    let interval = serde_json::from_value(serde_json::Value::String(interval_text))
                        .map_err(|error| {
                            rusqlite::Error::FromSqlConversionFailure(
                                4,
                                rusqlite::types::Type::Text,
                                Box::new(error),
                            )
                        })?;
                    let total_return_text = row.get::<_, String>(6)?;
                    let total_return = total_return_text.parse().map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            6,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })?;
                    let bar_count_value = row.get::<_, i64>(5)?;
                    let bar_count = usize::try_from(bar_count_value).map_err(|_| {
                        rusqlite::Error::IntegralValueOutOfRange(5, bar_count_value)
                    })?;
                    Ok(BacktestRunSummary {
                        run_id: row.get(0)?,
                        created_at: row.get(1)?,
                        snapshot_id: row.get(2)?,
                        code: row.get(3)?,
                        interval,
                        bar_count,
                        total_return,
                    })
                },
            )
            .map_err(string)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(string)?;
        Ok(BacktestRunPage {
            items,
            total,
            page: request.page,
            page_size: RUN_HISTORY_PAGE_SIZE,
        })
    }

    /// Reads one full Run for one User.
    pub(crate) fn get(&self, user_id: &str, run_id: &str) -> Result<BacktestRunView, String> {
        self.load_run(user_id, run_id)
            .map(|run| pipeline::full_run_view(&run))
    }

    /// Reads one Run's chart series aggregated to one time range and
    /// point budget for one User.
    pub(crate) fn chart_data(
        &self,
        request: &BacktestChartRequest,
    ) -> Result<BacktestRunView, String> {
        if request.start_time_ms >= request.end_time_ms
            || !(100..=10_000).contains(&request.max_points)
        {
            return Err("Backtest Chart range is invalid".into());
        }
        self.load_run(&request.user_id, &request.run_id).map(|run| {
            pipeline::run_view(
                &run,
                request.start_time_ms,
                request.end_time_ms,
                request.max_points,
            )
        })
    }

    /// Pages one Run's simulated orders and fills for one User.
    pub(crate) fn execution_data(
        &self,
        request: &BacktestExecutionRequest,
    ) -> Result<BacktestExecutionPage, String> {
        if !(1..=1_000).contains(&request.limit) {
            return Err("Backtest execution page is invalid".into());
        }
        let run = self.load_run(&request.user_id, &request.run_id)?;
        Ok(execution_page(&run.result, request.offset, request.limit))
    }

    /// Deletes one Run for one User unless an immutable Validation Report
    /// still references it; bridge rows cascade with the Run.
    pub(crate) fn delete(&self, user_id: &str, run_id: &str) -> Result<(), String> {
        validate_user(user_id)?;
        if self.0.validation_report_references_run(user_id, run_id)? {
            return Err("Backtest Run is referenced by an immutable Validation Report".into());
        }
        let database = self.0.database()?;
        let changed = database
            .execute(
                "DELETE FROM backtest_runs WHERE user_id = ?1 AND run_id = ?2",
                params![user_id, run_id],
            )
            .map_err(string)?;
        if changed == 0 {
            Err("Backtest Run was not found".into())
        } else {
            Ok(())
        }
    }

    /// The narrow component-lock query: for one User, the Run IDs locking
    /// each referenced Component Package, ordered by Run creation. Covers
    /// both the Component Library's deletion-lock and listing-lock needs;
    /// callers never issue SQL over the Run bridge tables themselves. The
    /// composition root passes the connection it already locks, so the
    /// query stays atomic with the caller's check-then-act sequence.
    pub(crate) fn runs_locking_components(
        &self,
        database: &Connection,
        user_id: &str,
    ) -> Result<HashMap<String, Vec<String>>, String> {
        validate_user(user_id)?;
        let mut locked_by_hash = HashMap::<String, Vec<String>>::new();
        let mut lock_statement = database
            .prepare(
                "SELECT rc.archive_sha256, rc.run_id FROM backtest_run_components rc
                 JOIN backtest_runs r USING(run_id)
                 WHERE r.user_id = ?1 ORDER BY r.created_at, rc.run_id",
            )
            .map_err(string)?;
        for row in lock_statement
            .query_map([user_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(string)?
        {
            let (hash, run_id) = row.map_err(string)?;
            locked_by_hash.entry(hash).or_default().push(run_id);
        }
        Ok(locked_by_hash)
    }

    /// The summary hook the composition root calls: the Run count for one
    /// User.
    pub(crate) fn summary_for_user(&self, user_id: &str) -> Result<BacktestSummary, String> {
        let database = self.0.database()?;
        Ok(BacktestSummary {
            run_count: self.run_count(&database, user_id)?,
        })
    }

    /// The Run count for one User on a connection the caller already
    /// holds, so Reset blocking checks stay atomic under the shared
    /// database lock.
    pub(crate) fn run_count(&self, database: &Connection, user_id: &str) -> Result<u64, String> {
        validate_user(user_id)?;
        database
            .query_row(
                "SELECT COUNT(*) FROM backtest_runs WHERE user_id = ?1",
                [user_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(string)
            .map(|count| count.max(0) as u64)
    }

    /// The Component Package hashes still locked by Runs, for the
    /// composition root's orphaned Component content guard during a Reset.
    /// `excluding_user` drops one User's Runs from the guard; the Reset All
    /// flow passes the reset User because those Runs are deleted in the
    /// same transaction.
    pub(crate) fn component_hashes_locked_by_runs(
        &self,
        database: &Connection,
        excluding_user: Option<&str>,
    ) -> Result<HashSet<String>, String> {
        let mut statement = database
            .prepare(
                "SELECT DISTINCT rc.archive_sha256 FROM backtest_run_components rc
                 JOIN backtest_runs r USING(run_id)
                 WHERE ?1 IS NULL OR r.user_id <> ?1",
            )
            .map_err(string)?;
        statement
            .query_map([excluding_user], |row| row.get::<_, String>(0))
            .map_err(string)?
            .collect::<Result<HashSet<_>, _>>()
            .map_err(string)
    }

    /// The reset hook the composition root calls inside its reset
    /// transaction: drops one User's Runs; the bridge rows cascade.
    pub(crate) fn reset_for_user(
        &self,
        transaction: &Transaction<'_>,
        user_id: &str,
    ) -> Result<(), String> {
        transaction
            .execute("DELETE FROM backtest_runs WHERE user_id = ?1", [user_id])
            .map_err(string)?;
        Ok(())
    }

    pub(super) fn save_run(
        &self,
        user_id: &str,
        run_id: &str,
        result: &BacktestRun,
    ) -> Result<(), String> {
        let json = serde_json::to_string(result).map_err(string)?;
        let mut database = self.0.database()?;
        let transaction = database.transaction().map_err(string)?;
        transaction
            .execute(
                "INSERT OR IGNORE INTO backtest_runs(run_id, user_id, result_json)
                 VALUES (?1, ?2, ?3)",
                params![run_id, user_id, json],
            )
            .map_err(string)?;
        for component in &result.component_lock {
            transaction.execute(
                "INSERT OR IGNORE INTO backtest_run_components(run_id, archive_sha256) VALUES (?1, ?2)",
                params![run_id, component.archive_sha256],
            ).map_err(string)?;
        }
        if let Some(provenance) = &result.provenance {
            for signal in &provenance.dataset_lock {
                transaction.execute(
                    "INSERT OR IGNORE INTO backtest_run_signal_datasets(run_id, dataset_id, signal_name) VALUES (?1, ?2, ?3)",
                    params![run_id, signal.dataset_id, signal.signal_name],
                ).map_err(string)?;
            }
        }
        transaction.commit().map_err(string)?;
        Ok(())
    }

    pub(super) fn load_run(&self, user_id: &str, run_id: &str) -> Result<BacktestRun, String> {
        validate_user(user_id)?;
        let json: String = self
            .0
            .database()?
            .query_row(
                "SELECT result_json FROM backtest_runs WHERE user_id = ?1 AND run_id = ?2",
                params![user_id, run_id],
                |row| row.get(0),
            )
            .map_err(|_| "Backtest Run was not found".to_owned())?;
        let run: BacktestRun = serde_json::from_str(&json).map_err(string)?;
        if let Some(provenance) = &run.provenance {
            pipeline::validate_provenance(provenance)?;
            if provenance.component_lock != run.component_lock {
                return Err("Backtest Run provenance does not match its Component Lock".into());
            }
        }
        Ok(run)
    }
}

fn execution_page(
    result: &adaq_backtest_core::SimulationResult,
    offset: usize,
    limit: usize,
) -> BacktestExecutionPage {
    BacktestExecutionPage {
        orders: result
            .orders
            .iter()
            .skip(offset)
            .take(limit)
            .cloned()
            .collect(),
        fills: result
            .fills
            .iter()
            .skip(offset)
            .take(limit)
            .cloned()
            .collect(),
        total_orders: result.orders.len(),
        total_fills: result.fills.len(),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestListRequest {
    pub user_id: String,
    #[serde(default)]
    pub src: Option<String>,
    #[serde(default)]
    pub code: Option<String>,
    pub page: usize,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestRunRequest {
    pub user_id: String,
    pub snapshot_id: String,
    #[serde(default)]
    pub run_start_time_ms: Option<i64>,
    #[serde(default)]
    pub run_end_time_ms: Option<i64>,
    #[serde(default)]
    pub factor_instances: Vec<FactorInstanceRequest>,
    #[serde(default)]
    pub signal_instances: Vec<SignalInstanceRequest>,
    pub strategy_archive_sha256: String,
    #[serde(default)]
    pub strategy_parameters: HashMap<String, String>,
    #[serde(with = "rust_decimal::serde::str")]
    pub initial_quote_allocation: rust_decimal::Decimal,
    pub execution_profile: ExecutionProfile,
    #[serde(default)]
    pub seed: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestPreflight {
    pub run_id: String,
    pub reuses_existing_run: bool,
    pub snapshot: MarketDataSnapshot,
    pub normalized_request: NormalizedBacktestRunRequest,
    pub feature_plan: serde_json::Value,
    pub component_lock: Vec<ComponentLockEntry>,
    pub dataset_lock: Vec<SignalDatasetLock>,
    pub architecture: StrategyArchitecture,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FactorInstanceRequest {
    pub alias: String,
    pub archive_sha256: String,
    #[serde(default)]
    pub parameters: HashMap<String, FactorParameterBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignalInstanceRequest {
    pub slot: String,
    pub dataset_id: String,
    pub signal_name: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum FactorParameterBinding {
    Literal(String),
    StrategyParameter { strategy_parameter: String },
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestRun {
    pub run_id: String,
    #[serde(default)]
    pub plan_hash: String,
    pub snapshot: MarketDataSnapshot,
    pub bars: Vec<OhlcvBar>,
    pub decisions: Vec<adaq_backtest_core::TargetDecision>,
    #[serde(default)]
    pub pauses: Vec<RunPauseRecord>,
    pub result: adaq_backtest_core::SimulationResult,
    pub component_lock: Vec<ComponentLockEntry>,
    #[serde(default)]
    pub provenance: Option<BacktestRunProvenance>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestRunView {
    pub run_id: String,
    pub plan_hash: String,
    pub snapshot: MarketDataSnapshot,
    pub bars: Vec<OhlcvBar>,
    pub decisions: Vec<adaq_backtest_core::TargetDecision>,
    pub pauses: Vec<RunPauseRecord>,
    pub result: adaq_backtest_core::SimulationResult,
    pub component_lock: Vec<ComponentLockEntry>,
    pub provenance: Option<BacktestRunProvenance>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestRunProvenance {
    pub normalized_request: NormalizedBacktestRunRequest,
    pub feature_plan_json: String,
    pub feature_plan_hash: String,
    pub component_lock: Vec<ComponentLockEntry>,
    #[serde(default)]
    pub dataset_lock: Vec<SignalDatasetLock>,
    #[serde(default = "composed_architecture")]
    pub architecture: StrategyArchitecture,
    pub indicator_engine_build_identity: IndicatorEngineBuildIdentity,
    pub backtest_engine_version: String,
    pub seed: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedBacktestRunRequest {
    pub snapshot_id: String,
    #[serde(default)]
    pub run_start_time_ms: Option<i64>,
    #[serde(default)]
    pub run_end_time_ms: Option<i64>,
    pub strategy_archive_sha256: String,
    pub strategy_parameters: BTreeMap<String, String>,
    pub factor_instances: Vec<NormalizedFactorInstance>,
    #[serde(default)]
    pub signal_instances: Vec<SignalInstanceRequest>,
    #[serde(with = "rust_decimal::serde::str")]
    pub initial_quote_allocation: rust_decimal::Decimal,
    pub execution_profile: ExecutionProfile,
    pub seed: u64,
}

fn composed_architecture() -> StrategyArchitecture {
    StrategyArchitecture::Composed
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignalDatasetLock {
    pub slot: String,
    pub dataset_id: String,
    pub signal_name: String,
    pub evidence_state: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedFactorInstance {
    pub alias: String,
    pub archive_sha256: String,
    pub parameters: Vec<NormalizedParameter>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedParameter {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndicatorEngineBuildIdentity {
    pub engine_version: String,
    pub ta_lib_version: String,
    pub ta_source_sha256: String,
    pub catalog_version: String,
    pub wrapper_sha256: String,
    pub target_triple: String,
    pub compiler_and_flags_sha256: String,
    pub engine_build_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunPauseRecord {
    pub open_time_ms: i64,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentLockEntry {
    pub component_id: String,
    pub version: String,
    pub archive_sha256: String,
    pub wasm_sha256: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestRunIdRequest {
    pub user_id: String,
    pub run_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestChartRequest {
    pub user_id: String,
    pub run_id: String,
    pub start_time_ms: i64,
    pub end_time_ms: i64,
    pub max_points: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestExecutionRequest {
    pub user_id: String,
    pub run_id: String,
    pub offset: usize,
    pub limit: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestExecutionPage {
    pub orders: Vec<adaq_backtest_core::SimulatedOrder>,
    pub fills: Vec<adaq_backtest_core::Fill>,
    pub total_orders: usize,
    pub total_fills: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestRunSummary {
    pub run_id: String,
    pub created_at: String,
    pub snapshot_id: String,
    pub code: String,
    pub interval: BarInterval,
    pub bar_count: usize,
    #[serde(with = "rust_decimal::serde::str")]
    pub total_return: rust_decimal::Decimal,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestRunPage {
    pub items: Vec<BacktestRunSummary>,
    pub total: usize,
    pub page: usize,
    pub page_size: usize,
}

fn string(error: impl std::fmt::Display) -> String {
    error.to_string()
}
