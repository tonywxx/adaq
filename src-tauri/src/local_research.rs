//! The Local Research composition root.
//!
//! Owns only dependency wiring (the shared SQLite connection, the data
//! root layout, and the Source implementations each deep module composes),
//! the Local Data summary and reset orchestration that fans out to the
//! modules' hooks, and the one cross-domain compatibility command that
//! joins Component, Snapshot, and Signal Dataset reads. The deep modules —
//! Component Library, Dataset Generation, Market Data Snapshot, Validation
//! Studies, and Backtest Run — own their domains' schema, SQL, and
//! lifecycle rules; this file issues no SQL against their tables (the
//! not-yet-extracted Watchlist, Signal Dataset, and Forecast Evaluation
//! domains still live here).

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Weak},
};

use adaq_backtest_core::{MarketDataSnapshot, SnapshotStore};
use adaq_component_tooling::{
    ComponentKind, ComponentManifest, ComponentPackage, FeatureSlotSource,
};
use adaq_data_core::{OhlcvBar, OkxClient, a_share::AshareClient};
use adaq_data_pipeline::{
    CancellationToken, DataPipeline, DataQualityReport, a_share::AshareDataPath,
    okx::OkxSpotDataPath, us_equity::UsEquityDataPath,
};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use tauri::Manager;

use crate::{
    backtest::{BacktestSource, Backtests, ComponentPackageSource, SnapshotReadSource},
    component_library::{
        ComponentLibrary, ComponentLockSource, ComponentSource, finish_staged_files, stage_files,
    },
    dataset_generation::{DatasetGeneration, GenerationSource},
    forecast_signal_dataset::{BacktestSignalDataset, backtest_signal_datasets},
    market_data_snapshot::{LocalSnapshotSource, MarketDataSnapshots},
    user::validate_user,
    validation::{ValidationRunOutcome, ValidationSource, ValidationStudies},
};

pub struct LocalResearchState {
    pub(crate) root: PathBuf,
    pub(crate) database: Arc<Mutex<Connection>>,
    pub(crate) pipeline: DataPipeline,
    pub(crate) okx: OkxSpotDataPath,
    pub(crate) ashare: AshareDataPath,
    pub(crate) us_equity: UsEquityDataPath,
    pub(crate) snapshots: MarketDataSnapshots,
    pub(crate) components: ComponentLibrary,
    source: Arc<LocalGenerationSource>,
    pub(crate) generation: DatasetGeneration,
    pub(crate) validation: ValidationStudies,
    pub(crate) backtests: Backtests,
    pub(crate) connections: crate::connections::ConnectionManager,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalDataSummary {
    data_directory: String,
    database_bytes: u64,
    component_bytes: u64,
    market_data_bytes: u64,
    watchlist_count: u64,
    component_count: u64,
    snapshot_count: u64,
    run_count: u64,
    protocol_count: u64,
    report_count: u64,
    generation_attempt_count: u64,
    model_artifact_count: u64,
    signal_dataset_count: u64,
    component_blocking_run_count: u64,
    market_data_blocking_record_count: u64,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LocalDataResetKind {
    Watchlist,
    Components,
    MarketData,
    All,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalDataRequest {
    pub user_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalDataResetRequest {
    pub user_id: String,
    pub kind: LocalDataResetKind,
}

/// The concrete local dependencies composed into the Dataset Generation
/// lifecycle module. Only database access, Component Package access
/// through the Component Library module, Market Data Snapshot access,
/// runtime Component materialization, and the Signal Dataset directory
/// are shared; the complete Local Research state is not.
pub(crate) struct LocalGenerationSource {
    database: Arc<Mutex<Connection>>,
    snapshots: MarketDataSnapshots,
    root: PathBuf,
    components: ComponentLibrary,
}

impl SnapshotReadSource for LocalGenerationSource {
    fn snapshot_for_user(
        &self,
        user_id: &str,
        snapshot_id: &str,
    ) -> Result<(MarketDataSnapshot, Vec<OhlcvBar>), String> {
        self.snapshots.snapshot_for_user(user_id, snapshot_id)
    }
}

impl ComponentPackageSource for LocalGenerationSource {
    fn package_for_user(&self, user_id: &str, hash: &str) -> Result<ComponentPackage, String> {
        self.components.package_for_user(user_id, hash)
    }

    fn runtime_component(&self, package: &ComponentPackage) -> Result<PathBuf, String> {
        let directory = self.root.join("runtime");
        fs::create_dir_all(&directory).map_err(string)?;
        let path = directory.join(format!("{}.wasm", package.manifest.wasm_sha256));
        if !path.is_file() {
            fs::write(&path, &package.wasm).map_err(string)?;
        }
        Ok(path)
    }
}

impl GenerationSource for LocalGenerationSource {
    fn database(&self) -> Result<std::sync::MutexGuard<'_, Connection>, String> {
        self.database.lock().map_err(string)
    }

    fn dataset_directory(&self) -> Result<PathBuf, String> {
        let directory = self.root.join("signal-datasets");
        fs::create_dir_all(&directory).map_err(string)?;
        Ok(directory)
    }
}

/// The concrete local dependencies composed into the Component Library
/// module. Only database access, the archive directory, and the Backtest
/// Run module's component-lock queries are shared; the complete Local
/// Research state is not. The Backtest module is constructed first, so it
/// is bound directly.
pub(crate) struct LocalComponentSource {
    database: Arc<Mutex<Connection>>,
    root: PathBuf,
    backtests: Backtests,
}

impl ComponentLockSource for LocalComponentSource {
    fn runs_locking_components(
        &self,
        database: &Connection,
        user_id: &str,
    ) -> Result<std::collections::HashMap<String, Vec<String>>, String> {
        self.backtests.runs_locking_components(database, user_id)
    }

    fn component_hashes_locked_by_runs(
        &self,
        database: &Connection,
        excluding_user: Option<&str>,
    ) -> Result<std::collections::HashSet<String>, String> {
        self.backtests
            .component_hashes_locked_by_runs(database, excluding_user)
    }
}

impl ComponentSource for LocalComponentSource {
    fn database(&self) -> Result<std::sync::MutexGuard<'_, Connection>, String> {
        self.database.lock().map_err(string)
    }

    fn archive_directory(&self) -> Result<PathBuf, String> {
        let directory = self.root.join("components");
        fs::create_dir_all(&directory).map_err(string)?;
        Ok(directory)
    }

    fn locks(&self) -> &dyn ComponentLockSource {
        self
    }
}

/// The concrete local dependencies composed into the Validation Studies
/// module. Only database access, Component Package access, Market Data
/// Snapshot access and persistence, and Backtest Run execution are shared;
/// the complete Local Research state is not. The state reference is bound
/// after the composition root finishes constructing itself. The database
/// handle is held directly because the module initializes its schema
/// before the self-reference is bound.
pub(crate) struct LocalValidationSource {
    database: Arc<Mutex<Connection>>,
    state: Mutex<Weak<LocalResearchState>>,
}

impl ValidationSource for LocalValidationSource {
    fn database(&self) -> Result<std::sync::MutexGuard<'_, Connection>, String> {
        self.database.lock().map_err(string)
    }

    fn package_for_user(
        &self,
        user_id: &str,
        archive_sha256: &str,
    ) -> Result<ComponentPackage, String> {
        self.state()?.package_for_user(user_id, archive_sha256)
    }

    fn snapshot_for_user(
        &self,
        user_id: &str,
        snapshot_id: &str,
    ) -> Result<(MarketDataSnapshot, Vec<OhlcvBar>), String> {
        self.state()?.snapshot_for_user(user_id, snapshot_id)
    }

    fn persist_snapshot_for_user(
        &self,
        user_id: &str,
        series: &adaq_data_core::BarSeries,
    ) -> Result<MarketDataSnapshot, String> {
        self.state()?.persist_snapshot_for_user(user_id, series)
    }

    fn run_backtest(
        &self,
        request: crate::backtest::BacktestRunRequest,
    ) -> Result<ValidationRunOutcome, String> {
        let state = self.state()?;
        let view = state.backtests.run(request)?;
        Ok(ValidationRunOutcome {
            run_id: view.run_id,
            metrics: view.result.metrics,
            pauses: view.pauses,
        })
    }
}

impl LocalValidationSource {
    fn state(&self) -> Result<Arc<LocalResearchState>, String> {
        self.state
            .lock()
            .map_err(string)?
            .upgrade()
            .ok_or_else(|| "Local Research state is not available".to_owned())
    }
}

/// The concrete local dependencies composed into the Backtest Run module.
/// Only database access, Market Data Snapshot reads, Component Package
/// access, Signal Dataset reads through the forecast_signal_dataset-owned path, and the
/// Validation Report reference check are shared; the complete Local
/// Research state is not. The state reference is bound after the
/// composition root finishes constructing itself. The database handle is
/// held directly because the module initializes its schema before the
/// self-reference is bound.
pub(crate) struct LocalBacktestSource {
    database: Arc<Mutex<Connection>>,
    state: Mutex<Weak<LocalResearchState>>,
}

impl LocalBacktestSource {
    fn state(&self) -> Result<Arc<LocalResearchState>, String> {
        self.state
            .lock()
            .map_err(string)?
            .upgrade()
            .ok_or_else(|| "Local Research state is not available".to_owned())
    }
}

impl SnapshotReadSource for LocalBacktestSource {
    fn snapshot_for_user(
        &self,
        user_id: &str,
        snapshot_id: &str,
    ) -> Result<(MarketDataSnapshot, Vec<OhlcvBar>), String> {
        self.state()?.snapshot_for_user(user_id, snapshot_id)
    }
}

impl ComponentPackageSource for LocalBacktestSource {
    fn package_for_user(
        &self,
        user_id: &str,
        archive_sha256: &str,
    ) -> Result<ComponentPackage, String> {
        self.state()?.package_for_user(user_id, archive_sha256)
    }

    fn runtime_component(&self, package: &ComponentPackage) -> Result<PathBuf, String> {
        self.state()?.runtime_component(package)
    }
}

impl BacktestSource for LocalBacktestSource {
    fn database(&self) -> Result<std::sync::MutexGuard<'_, Connection>, String> {
        self.database.lock().map_err(string)
    }

    fn signal_datasets(
        &self,
        user_id: &str,
        include_rows: bool,
        dataset_ids: Option<&[String]>,
    ) -> Result<Vec<BacktestSignalDataset>, String> {
        let state = self.state()?;
        backtest_signal_datasets(&state, user_id, include_rows, dataset_ids)
    }

    fn validation_report_references_run(
        &self,
        user_id: &str,
        run_id: &str,
    ) -> Result<bool, String> {
        self.state()?.validation.references_run(user_id, run_id)
    }
}

impl LocalResearchState {
    pub fn open(app_data: &Path) -> Result<Arc<Self>, String> {
        let root = app_data.join("m3");
        fs::create_dir_all(root.join("components")).map_err(string)?;
        let snapshot_store = SnapshotStore::new(root.join("market-data")).map_err(string)?;
        let database = Connection::open(app_data.join("adaq.db")).map_err(string)?;
        database
            .execute_batch(
                "PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS signal_dataset_content (
                dataset_id TEXT PRIMARY KEY,
                metadata_json TEXT NOT NULL,
                parquet_path TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS signal_dataset_access (
                user_id TEXT NOT NULL,
                dataset_id TEXT NOT NULL,
                PRIMARY KEY(user_id, dataset_id),
                FOREIGN KEY(dataset_id) REFERENCES signal_dataset_content(dataset_id)
             );
             CREATE TABLE IF NOT EXISTS forecast_evaluation_content (
                report_id TEXT PRIMARY KEY,
                report_json TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS forecast_evaluation_access (
                user_id TEXT NOT NULL,
                report_id TEXT NOT NULL,
                PRIMARY KEY(user_id, report_id),
                FOREIGN KEY(report_id) REFERENCES forecast_evaluation_content(report_id)
             );",
            )
            .map_err(string)?;
        let database = Arc::new(Mutex::new(database));
        let pipeline = DataPipeline::open(root.join("market-data-pipeline"), database.clone())
            .map_err(string)?;
        let okx = OkxSpotDataPath::open(pipeline.clone(), OkxClient::default()).map_err(string)?;
        let ashare =
            AshareDataPath::open(pipeline.clone(), AshareClient::default()).map_err(string)?;
        let us_equity = UsEquityDataPath::open(pipeline.clone()).map_err(string)?;
        let connections = crate::connections::ConnectionManager::open_production(database.clone())?;
        let snapshot_source = Arc::new(LocalSnapshotSource::new(
            database.clone(),
            Arc::new(snapshot_store),
        ));
        let snapshots = MarketDataSnapshots::open(snapshot_source)?;
        let backtest_source = Arc::new(LocalBacktestSource {
            database: database.clone(),
            state: Mutex::new(Weak::new()),
        });
        let backtests = Backtests::open(backtest_source.clone())?;
        let component_source = Arc::new(LocalComponentSource {
            database: database.clone(),
            root: root.clone(),
            backtests: backtests.clone(),
        });
        let components = ComponentLibrary::open(component_source)?;
        let source = Arc::new(LocalGenerationSource {
            database: database.clone(),
            snapshots: snapshots.clone(),
            root: root.clone(),
            components: components.clone(),
        });
        let generation = DatasetGeneration::open(source.clone())?;
        let validation_source = Arc::new(LocalValidationSource {
            database: database.clone(),
            state: Mutex::new(Weak::new()),
        });
        let validation = ValidationStudies::open(validation_source.clone())?;
        Ok(Arc::new_cyclic(|weak| {
            *validation_source
                .state
                .lock()
                .expect("validation source state binding is uncontended") = weak.clone();
            *backtest_source
                .state
                .lock()
                .expect("backtest source state binding is uncontended") = weak.clone();
            Self {
                root,
                database,
                pipeline,
                okx,
                ashare,
                us_equity,
                snapshots,
                components,
                source,
                generation,
                validation,
                backtests,
                connections,
            }
        }))
    }

    pub fn local_data_summary(&self, user_id: &str) -> Result<LocalDataSummary, String> {
        validate_user(user_id)?;
        let generation_attempt_count = self.generation.list(user_id)?.len() as u64;
        let validation = self.validation.summary_for_user(user_id)?;
        // Query the Snapshot, Backtest, and Component modules before
        // locking the database mutex so the hooks never re-enter a held
        // lock.
        let snapshots = self.snapshots.summary_for_user(user_id)?;
        let backtests = self.backtests.summary_for_user(user_id)?;
        let components = self.components.summary_for_user(user_id)?;
        let database = self.database.lock().map_err(string)?;
        let count = |sql: &str| -> Result<u64, String> {
            database
                .query_row(sql, [user_id], |row| row.get::<_, i64>(0))
                .map(|value| value.max(0) as u64)
                .map_err(string)
        };
        let database_path = self.root.parent().unwrap_or(&self.root).join("adaq.db");
        let data_directory = database_path
            .parent()
            .unwrap_or(&self.root)
            .to_string_lossy()
            .into_owned();

        Ok(LocalDataSummary {
            data_directory,
            database_bytes: file_bytes(&database_path),
            component_bytes: components.component_bytes,
            market_data_bytes: snapshots.market_data_bytes,
            watchlist_count: count("SELECT COUNT(*) FROM watchlist_items WHERE user_id = ?1")?,
            component_count: components.component_count,
            snapshot_count: snapshots.snapshot_count,
            run_count: backtests.run_count,
            protocol_count: validation.protocol_count,
            report_count: validation.report_count,
            generation_attempt_count,
            model_artifact_count: components.model_artifact_count,
            signal_dataset_count: count(
                "SELECT COUNT(*) FROM signal_dataset_access WHERE user_id = ?1",
            )?,
            component_blocking_run_count: components.component_blocking_run_count,
            market_data_blocking_record_count: backtests
                .run_count
                .saturating_add(count(
                    "SELECT COUNT(*) FROM signal_dataset_access WHERE user_id = ?1",
                )?)
                .saturating_add(validation.report_count),
        })
    }

    pub fn reset_local_data(&self, user_id: &str, kind: LocalDataResetKind) -> Result<(), String> {
        validate_user(user_id)?;
        let _reset_block = if matches!(kind, LocalDataResetKind::All) {
            Some(self.generation.stop_all_for_user(user_id)?)
        } else {
            None
        };
        let resets_ashare = matches!(
            kind,
            LocalDataResetKind::MarketData | LocalDataResetKind::All
        );
        let resets_us_equity = resets_ashare;
        // Query the Validation module before locking the database mutex so
        // the hook never re-enters a held lock. The Snapshot orphan query
        // and the Backtest and Component hooks run inside the reset flows
        // under the held lock instead, so they stay serialized with
        // Snapshot persistence and Run writes.
        let validation_report_count = if matches!(kind, LocalDataResetKind::MarketData) {
            self.validation.summary_for_user(user_id)?.report_count
        } else {
            0
        };
        if resets_ashare {
            if let Err(error) = self.pipeline.begin_user_reset(user_id) {
                return Err(string(error));
            }
        }
        let pipeline_snapshot_blocker_count = if matches!(
            kind,
            LocalDataResetKind::MarketData | LocalDataResetKind::All
        ) {
            match self.pipeline.snapshot_deletion_blockers_for_user(user_id) {
                Ok(blockers) => blockers.len() as u64,
                Err(error) => {
                    if resets_ashare {
                        let _ = self.pipeline.finish_user_reset(user_id);
                    }
                    return Err(string(error));
                }
            }
        } else {
            0
        };
        if resets_ashare {
            if let Err(error) = self.ashare.begin_user_reset(user_id) {
                let _ = self.pipeline.finish_user_reset(user_id);
                return Err(string(error));
            }
        }
        if resets_us_equity {
            if let Err(error) = self.us_equity.begin_user_reset(user_id) {
                let _ = self.ashare.finish_user_reset(user_id);
                let _ = self.pipeline.finish_user_reset(user_id);
                return Err(string(error));
            }
        }
        let mut database = match self.database.lock() {
            Ok(database) => database,
            Err(error) => {
                if resets_ashare {
                    let _ = self.us_equity.finish_user_reset(user_id);
                    let _ = self.ashare.finish_user_reset(user_id);
                    let _ = self.pipeline.finish_user_reset(user_id);
                }
                return Err(string(error));
            }
        };
        let paths = if resets_ashare {
            self.pipeline
                .reset_paths_for_user_with_connection(&database, user_id)
                .and_then(|pipeline_paths| {
                    self.ashare
                        .reset_paths_for_user_with_connection(&database, user_id)
                        .and_then(|ashare_paths| {
                            self.us_equity
                                .reset_paths_for_user_with_connection(&database, user_id)
                                .map(|us_equity_paths| {
                                    (pipeline_paths, ashare_paths, us_equity_paths)
                                })
                        })
                })
                .map_err(string)
        } else {
            Ok((Vec::new(), Vec::new(), Vec::new()))
        };
        let (pipeline_paths, ashare_paths, us_equity_paths) = match paths {
            Ok(paths) => paths,
            Err(error) => {
                if resets_ashare {
                    let _ = self.us_equity.finish_user_reset(user_id);
                    let _ = self.ashare.finish_user_reset(user_id);
                    let _ = self.pipeline.finish_user_reset(user_id);
                }
                return Err(error);
            }
        };
        let result = match kind {
            LocalDataResetKind::Watchlist => reset_watchlist(&mut database, user_id),
            LocalDataResetKind::Components => {
                self.components.reset_for_user(&mut database, user_id)
            }
            LocalDataResetKind::MarketData => reset_market_data(
                &mut database,
                user_id,
                &self.root,
                validation_report_count,
                pipeline_snapshot_blocker_count,
                &self.snapshots,
                &self.backtests,
                &self.ashare,
                &self.us_equity,
                &self.pipeline,
                pipeline_paths.clone(),
                ashare_paths.clone(),
                us_equity_paths.clone(),
            ),
            LocalDataResetKind::All => reset_all(
                &mut database,
                user_id,
                &self.root,
                _reset_block.as_ref().unwrap(),
                &self.components,
                &self.validation,
                &self.snapshots,
                &self.backtests,
                pipeline_snapshot_blocker_count,
                &self.ashare,
                &self.us_equity,
                &self.pipeline,
                pipeline_paths,
                ashare_paths,
                us_equity_paths,
            ),
        };
        if resets_ashare {
            let us_equity_finish = self.us_equity.finish_user_reset(user_id).map_err(string);
            let finish = self.ashare.finish_user_reset(user_id).map_err(string);
            let pipeline_finish = self.pipeline.finish_user_reset(user_id).map_err(string);
            match (result, us_equity_finish, finish, pipeline_finish) {
                (Err(error), _, _, _) => Err(error),
                (Ok(()), Err(error), _, _) => Err(error),
                (Ok(()), Ok(()), Err(error), _) => Err(error),
                (Ok(()), Ok(()), Ok(()), Err(error)) => Err(error),
                (Ok(()), Ok(()), Ok(()), Ok(())) => Ok(()),
            }
        } else {
            result
        }
    }

    pub(crate) fn persist_snapshot_for_user(
        &self,
        user_id: &str,
        series: &adaq_data_core::BarSeries,
    ) -> Result<MarketDataSnapshot, String> {
        self.snapshots.persist_for_user(user_id, series)
    }

    #[cfg(test)]
    pub(crate) fn grant_snapshot_for_user(
        &self,
        user_id: &str,
        snapshot_id: &str,
    ) -> Result<(), String> {
        self.snapshots.grant_for_user(user_id, snapshot_id)
    }

    pub(crate) fn package_for_user(
        &self,
        user_id: &str,
        hash: &str,
    ) -> Result<ComponentPackage, String> {
        self.components.package_for_user(user_id, hash)
    }

    pub(crate) fn snapshot_for_user(
        &self,
        user_id: &str,
        snapshot_id: &str,
    ) -> Result<(MarketDataSnapshot, Vec<OhlcvBar>), String> {
        self.snapshots.snapshot_for_user(user_id, snapshot_id)
    }

    pub(crate) fn publish_pipeline_snapshot_for_user(
        &self,
        user_id: &str,
        canonical_id: &str,
    ) -> Result<(MarketDataSnapshot, DataQualityReport), String> {
        let cancellation = CancellationToken::new();
        let _operation = self
            .pipeline
            .begin_user_operation(user_id, format!("snapshot:{canonical_id}"), &cancellation)
            .map_err(string)?;
        let canonical = self
            .pipeline
            .canonical_for_user(user_id, canonical_id)
            .map_err(string)?;
        if cancellation.is_cancelled() {
            return Err("pipeline snapshot publication was cancelled".into());
        }
        let quality = self
            .pipeline
            .quality_for_user(user_id, &canonical.quality_report_id)
            .map_err(string)?;
        let snapshot = self
            .snapshots
            .persist_for_user(user_id, &canonical.to_bar_series())?;
        if cancellation.is_cancelled() {
            self.snapshots
                .revoke_for_user(user_id, &snapshot.snapshot_id)?;
            return Err("pipeline snapshot publication was cancelled".into());
        }
        if let Err(error) =
            self.pipeline
                .record_snapshot_reference(user_id, canonical_id, &snapshot.snapshot_id)
        {
            let _ = self.pipeline.remove_snapshot_reference(
                user_id,
                canonical_id,
                &snapshot.snapshot_id,
            );
            let cleanup = self
                .snapshots
                .revoke_for_user(user_id, &snapshot.snapshot_id);
            return match cleanup {
                Ok(()) => Err(string(error)),
                Err(cleanup_error) => Err(format!(
                    "{error}; failed to clean up persisted snapshot: {cleanup_error}"
                )),
            };
        }
        if cancellation.is_cancelled() {
            let reference_cleanup = self.pipeline.remove_snapshot_reference(
                user_id,
                canonical_id,
                &snapshot.snapshot_id,
            );
            let snapshot_cleanup = self
                .snapshots
                .revoke_for_user(user_id, &snapshot.snapshot_id);
            return match (reference_cleanup, snapshot_cleanup) {
                (Ok(()), Ok(())) => Err("pipeline snapshot publication was cancelled".into()),
                (reference, snapshot) => Err(format!(
                    "pipeline snapshot publication was cancelled; cleanup failed: reference={reference:?}, snapshot={snapshot:?}"
                )),
            };
        }
        Ok((snapshot, quality))
    }

    pub(crate) fn runtime_component(&self, package: &ComponentPackage) -> Result<PathBuf, String> {
        self.source.runtime_component(package)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestSignalCompatibilityRequest {
    user_id: String,
    strategy_archive_sha256: String,
    snapshot_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestSignalCandidate {
    slot: String,
    dataset_id: String,
    signal_name: String,
    evidence_state: String,
}

/// The one cross-domain compatibility command: it joins Component Package
/// reads through the Component Library module, Snapshot reads through the
/// Snapshot module, and Signal Dataset reads through the forecast_signal_dataset-owned path.
#[tauri::command]
pub fn backtest_compatible_signals(
    request: BacktestSignalCompatibilityRequest,
    state: tauri::State<'_, Arc<LocalResearchState>>,
) -> Result<Vec<BacktestSignalCandidate>, String> {
    let strategy = state.package_for_user(&request.user_id, &request.strategy_archive_sha256)?;
    if strategy.manifest.kind != ComponentKind::Strategy {
        return Err("Compatible Signals require a Strategy Component".into());
    }
    let (snapshot, _) = state.snapshot_for_user(&request.user_id, &request.snapshot_id)?;
    let datasets = backtest_signal_datasets(&state, &request.user_id, false, None)?;
    Ok(compatible_signal_candidates(
        &strategy.manifest,
        &snapshot,
        &datasets,
    ))
}

fn compatible_signal_candidates(
    strategy: &ComponentManifest,
    snapshot: &MarketDataSnapshot,
    datasets: &[crate::forecast_signal_dataset::BacktestSignalDataset],
) -> Vec<BacktestSignalCandidate> {
    let mut candidates = Vec::new();
    for slot in &strategy.feature_slots {
        let FeatureSlotSource::Signal {
            prediction_kind,
            forecast_target,
            value_scale,
            horizon_bars,
        } = &slot.source
        else {
            continue;
        };
        for dataset in datasets.iter().filter(|dataset| {
            dataset.snapshot_id == snapshot.snapshot_id
                && dataset.src == snapshot.src
                && dataset.code == snapshot.code
                && dataset.interval == snapshot.interval.as_str()
        }) {
            for output in dataset.outputs.iter().filter(|output| {
                output.prediction_kind == *prediction_kind
                    && output.forecast_target == *forecast_target
                    && output.value_scale == *value_scale
                    && output.horizon_bars == *horizon_bars
            }) {
                candidates.push(BacktestSignalCandidate {
                    slot: slot.name.clone(),
                    dataset_id: dataset.dataset_id.clone(),
                    signal_name: output.name.clone(),
                    evidence_state: dataset.evidence_state.clone(),
                });
            }
        }
    }
    candidates
}

#[tauri::command]
pub fn local_data_summary(
    request: LocalDataRequest,
    state: tauri::State<'_, Arc<LocalResearchState>>,
) -> Result<LocalDataSummary, String> {
    state.local_data_summary(&request.user_id)
}

#[tauri::command]
pub async fn local_data_reset(
    request: LocalDataResetRequest,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Arc<LocalResearchState>>();
        state.reset_local_data(&request.user_id, request.kind)
    })
    .await
    .map_err(string)?
}

fn reset_watchlist(database: &mut Connection, user_id: &str) -> Result<(), String> {
    let transaction = database.transaction().map_err(string)?;
    transaction
        .execute(
            "DELETE FROM watchlist_settings WHERE user_id = ?1",
            [user_id],
        )
        .map_err(string)?;
    insert_default_watchlist(&transaction, user_id)?;
    transaction.commit().map_err(string)
}

fn insert_default_watchlist(database: &Connection, user_id: &str) -> Result<(), String> {
    database
        .execute(
            "INSERT INTO watchlist_settings(user_id, active_src, active_code, mini_chart_interval)
             VALUES (?1, 'okx', 'BTC-USDT', '1m')",
            [user_id],
        )
        .map_err(string)?;
    for (position, code) in ["BTC-USDT", "ETH-USDT", "SOL-USDT"].iter().enumerate() {
        database
            .execute(
                "INSERT INTO watchlist_items(user_id, src, code, position)
                 VALUES (?1, 'okx', ?2, ?3)",
                params![user_id, code, position as i64],
            )
            .map_err(string)?;
    }
    Ok(())
}

fn reset_market_data(
    database: &mut Connection,
    user_id: &str,
    root: &Path,
    validation_report_count: u64,
    pipeline_snapshot_blocker_count: u64,
    snapshots: &MarketDataSnapshots,
    backtests: &Backtests,
    ashare: &AshareDataPath,
    us_equity: &UsEquityDataPath,
    pipeline: &DataPipeline,
    pipeline_paths: Vec<PathBuf>,
    ashare_paths: Vec<PathBuf>,
    us_equity_paths: Vec<PathBuf>,
) -> Result<(), String> {
    let blocking_datasets: i64 = database
        .query_row(
            "SELECT COUNT(*) FROM signal_dataset_access WHERE user_id = ?1",
            [user_id],
            |row| row.get(0),
        )
        .map_err(string)?;
    let blocking = backtests.run_count(database, user_id)?
        + blocking_datasets.max(0) as u64
        + validation_report_count
        + pipeline_snapshot_blocker_count;
    if blocking > 0 {
        return Err(format!(
            "Market Data reset is blocked by {blocking} immutable research record(s)"
        ));
    }
    let staged = stage_files(
        snapshots
            .orphaned_parquet_paths(database, user_id)?
            .into_iter()
            .chain(pipeline_paths)
            .chain(ashare_paths)
            .chain(us_equity_paths),
        root,
    )?;
    let result = (|| {
        let transaction = database.transaction().map_err(string)?;
        snapshots.reset_for_user(&transaction, user_id)?;
        ashare
            .reset_user_rows(&transaction, user_id)
            .map_err(string)?;
        us_equity
            .reset_user_rows(&transaction, user_id)
            .map_err(string)?;
        pipeline
            .reset_user_rows(&transaction, user_id)
            .map_err(string)?;
        transaction.commit().map_err(string)
    })();
    finish_staged_files(staged, result)
}

fn reset_all(
    database: &mut Connection,
    user_id: &str,
    root: &Path,
    reset_block: &crate::dataset_generation::UserResetBlock<'_>,
    components: &ComponentLibrary,
    validation: &ValidationStudies,
    snapshots: &MarketDataSnapshots,
    backtests: &Backtests,
    pipeline_snapshot_blocker_count: u64,
    ashare: &AshareDataPath,
    us_equity: &UsEquityDataPath,
    pipeline: &DataPipeline,
    pipeline_paths: Vec<PathBuf>,
    ashare_paths: Vec<PathBuf>,
    us_equity_paths: Vec<PathBuf>,
) -> Result<(), String> {
    // The reset User's Runs are deleted inside the transaction below, so
    // only other Users' Runs keep locking Component content; the Component
    // module derives that guard from the Backtest module's lock query, and
    // it stays stable under the held database lock for both the staged
    // file selection and the transaction's orphan cleanup.
    if pipeline_snapshot_blocker_count > 0 {
        return Err(format!(
            "All local data reset is blocked by {pipeline_snapshot_blocker_count} pipeline snapshot reference(s)"
        ));
    }
    let component_paths = components.orphan_archive_paths(database, user_id, Some(user_id))?;
    let dataset_paths = strings(
        database,
        "SELECT c.parquet_path FROM signal_dataset_content c JOIN signal_dataset_access a USING(dataset_id) WHERE a.user_id = ?1 AND NOT EXISTS(SELECT 1 FROM signal_dataset_access other WHERE other.dataset_id = c.dataset_id AND other.user_id <> ?1)",
        user_id,
    )?;
    let staged = stage_files(
        component_paths
            .into_iter()
            .chain(snapshots.orphaned_parquet_paths(database, user_id)?)
            .chain(dataset_paths.into_iter().map(PathBuf::from))
            .chain(pipeline_paths)
            .chain(ashare_paths)
            .chain(us_equity_paths),
        root,
    )?;
    let result = (|| {
        let transaction = database.transaction().map_err(string)?;
        validation.reset_for_user(&transaction, user_id)?;
        reset_block.delete_attempt_evidence(&transaction)?;
        transaction
            .execute(
                "DELETE FROM forecast_evaluation_access WHERE user_id = ?1",
                [user_id],
            )
            .map_err(string)?;
        transaction.execute("DELETE FROM forecast_evaluation_content WHERE NOT EXISTS(SELECT 1 FROM forecast_evaluation_access a WHERE a.report_id = forecast_evaluation_content.report_id)", []).map_err(string)?;
        backtests.reset_for_user(&transaction, user_id)?;
        transaction
            .execute(
                "DELETE FROM signal_dataset_access WHERE user_id = ?1",
                [user_id],
            )
            .map_err(string)?;
        transaction.execute("DELETE FROM signal_dataset_content WHERE NOT EXISTS(SELECT 1 FROM signal_dataset_access a WHERE a.dataset_id = signal_dataset_content.dataset_id)", []).map_err(string)?;
        components.reset_access_for_user(&transaction, user_id)?;
        snapshots.reset_for_user(&transaction, user_id)?;
        ashare
            .reset_user_rows(&transaction, user_id)
            .map_err(string)?;
        us_equity
            .reset_user_rows(&transaction, user_id)
            .map_err(string)?;
        pipeline
            .reset_user_rows(&transaction, user_id)
            .map_err(string)?;
        components.delete_orphan_content(&transaction, Some(user_id))?;
        transaction
            .execute(
                "DELETE FROM watchlist_settings WHERE user_id = ?1",
                [user_id],
            )
            .map_err(string)?;
        insert_default_watchlist(&transaction, user_id)?;
        transaction.commit().map_err(string)
    })();
    finish_staged_files(staged, result)
}

fn strings(database: &Connection, sql: &str, user_id: &str) -> Result<Vec<String>, String> {
    database
        .prepare(sql)
        .map_err(string)?
        .query_map([user_id], |row| row.get(0))
        .map_err(string)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(string)
}

fn file_bytes(path: impl AsRef<Path>) -> u64 {
    fs::metadata(path).map_or(0, |metadata| metadata.len())
}

fn string(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        backtest::{BacktestRunRequest, FactorInstanceRequest},
        watchlist::WatchlistDb,
    };
    use adaq_backtest_core::ExecutionProfile;
    use adaq_data_core::{BarGap, BarInterval};
    use std::{
        collections::HashMap,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn local_data_state(name: &str) -> (PathBuf, Arc<LocalResearchState>, WatchlistDb) {
        let root = std::env::temp_dir().join(format!(
            "adaq-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let state = LocalResearchState::open(&root).unwrap();
        let watchlist = WatchlistDb::open(&root.join("adaq.db")).unwrap();
        (root, state, watchlist)
    }

    fn public_example_package(name: &str) -> Vec<u8> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../examples/components")
            .join(name)
            .join("dist")
            .join(format!("{name}-0.1.0.adaq"));
        assert!(
            path.is_file(),
            "build the {name} example with adaq-component build"
        );
        fs::read(path).unwrap()
    }

    #[test]
    fn compatible_signal_selection_requires_exact_market_and_semantic_identity() {
        let strategy: ComponentManifest = serde_json::from_value(serde_json::json!({
            "manifestSchemaVersion": "1.0.0",
            "componentId": "00000000-0000-4000-8000-000000000001",
            "version": "1.0.0",
            "name": "Signal Strategy",
            "kind": "strategy",
            "sdkVersion": "0.1.0",
            "abiVersion": "1.0.0",
            "featureSlots": [{
                "name": "forecast-up",
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
        let snapshot = MarketDataSnapshot {
            snapshot_id: "snapshot".into(),
            src: "okx".into(),
            code: "BTC-USDT".into(),
            interval: BarInterval::OneHour,
            start_time_ms: 0,
            end_time_ms: 3_600_000,
            bar_count: 1,
            gaps: vec![],
            parquet_path: PathBuf::new(),
        };
        let dataset = crate::forecast_signal_dataset::BacktestSignalDataset {
            dataset_id: "a".repeat(64),
            snapshot_id: snapshot.snapshot_id.clone(),
            src: snapshot.src.clone(),
            code: snapshot.code.clone(),
            interval: snapshot.interval.as_str().into(),
            outputs: vec![adaq_component_tooling::ModelOutput {
                name: "up".into(),
                prediction_kind: adaq_component_tooling::PredictionKind::Probability,
                forecast_target: adaq_component_tooling::ForecastTarget::Builtin {
                    target: adaq_component_tooling::BuiltinForecastTarget::FutureCloseUp,
                },
                value_scale: adaq_component_tooling::ForecastValueScale::Probability,
                horizon_bars: 1,
            }],
            producer_segments: vec![serde_json::json!({"segment": 1})],
            artifact_provenance: serde_json::json!({"sha256": "artifact"}),
            evidence_state: "unknown".into(),
            component_lock: vec![],
            rows: vec![],
        };
        let mut wrong_snapshot = dataset.clone();
        wrong_snapshot.dataset_id = "b".repeat(64);
        wrong_snapshot.snapshot_id = "other".into();
        let mut wrong_horizon = dataset.clone();
        wrong_horizon.dataset_id = "c".repeat(64);
        wrong_horizon.outputs[0].horizon_bars = 2;

        let candidates = compatible_signal_candidates(
            &strategy,
            &snapshot,
            &[dataset, wrong_snapshot, wrong_horizon],
        );
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].slot, "forecast-up");
        assert_eq!(candidates[0].signal_name, "up");
    }

    #[test]
    fn reset_all_is_user_scoped_and_restores_watchlist_defaults() {
        let (root, state, watchlist) = local_data_state("reset-all");
        watchlist.get("alice").unwrap();
        watchlist.get("bob").unwrap();
        let package = root.join("m3/components/shared.adaq");
        fs::write(&package, b"package").unwrap();
        let database = state.database.lock().unwrap();
        database.execute(
			"INSERT INTO component_content(archive_sha256, component_id, version, name, kind, wasm_sha256, archive_path) VALUES ('hash', 'component', '1.0.0', 'Shared', 'factor', 'wasm', ?1)",
			[package.to_string_lossy().as_ref()],
		).unwrap();
        for user in ["alice", "bob"] {
            database
                .execute("INSERT INTO component_access VALUES (?1, 'hash')", [user])
                .unwrap();
        }
        let dataset = root.join("m3/signal-datasets/shared.parquet");
        fs::create_dir_all(dataset.parent().unwrap()).unwrap();
        fs::write(&dataset, b"parquet").unwrap();
        database
            .execute(
                "INSERT INTO signal_dataset_content VALUES ('dataset', '{}', ?1)",
                [dataset.to_string_lossy().as_ref()],
            )
            .unwrap();
        for user in ["alice", "bob"] {
            database
                .execute(
                    "INSERT INTO signal_dataset_access VALUES (?1, 'dataset')",
                    [user],
                )
                .unwrap();
        }
        database
            .execute(
                "INSERT INTO dataset_generation_attempts
                 (attempt_id, request_hash, user_id, status, request_json)
                 VALUES ('attempt', 'request', 'alice', 'pending', '{}')",
                [],
            )
            .unwrap();
        drop(database);

        state
            .reset_local_data("alice", LocalDataResetKind::All)
            .unwrap();

        let alice = state.local_data_summary("alice").unwrap();
        let bob = state.local_data_summary("bob").unwrap();
        assert_eq!(alice.watchlist_count, 3);
        assert_eq!(alice.component_count, 0);
        assert_eq!(alice.generation_attempt_count, 0);
        assert_eq!(alice.signal_dataset_count, 0);
        assert_eq!(bob.watchlist_count, 3);
        assert_eq!(bob.component_count, 1);
        assert_eq!(bob.signal_dataset_count, 1);
        assert!(package.is_file());
        assert!(dataset.is_file());
    }

    #[test]
    fn component_reset_refuses_to_break_a_run_lock() {
        let (root, state, watchlist) = local_data_state("reset-components");
        watchlist.get("alice").unwrap();
        let package = root.join("m3/components/locked.adaq");
        fs::write(&package, b"package").unwrap();
        let database = state.database.lock().unwrap();
        database.execute(
			"INSERT INTO component_content(archive_sha256, component_id, version, name, kind, wasm_sha256, archive_path) VALUES ('hash', 'component', '1.0.0', 'Locked', 'factor', 'wasm', ?1)",
			[package.to_string_lossy().as_ref()],
		).unwrap();
        database
            .execute("INSERT INTO component_access VALUES ('alice', 'hash')", [])
            .unwrap();
        database.execute("INSERT INTO backtest_runs(run_id, user_id, result_json) VALUES ('run', 'alice', '{}')", []).unwrap();
        database
            .execute(
                "INSERT INTO backtest_run_components VALUES ('run', 'hash')",
                [],
            )
            .unwrap();
        drop(database);

        let error = state
            .reset_local_data("alice", LocalDataResetKind::Components)
            .unwrap_err();
        assert!(error.contains("blocked by 1 immutable Backtest Run"));
        assert_eq!(
            state.local_data_summary("alice").unwrap().component_count,
            1
        );
        assert!(package.is_file());
    }

    #[test]
    fn public_examples_import_and_execute_a_deterministic_backtest() {
        let root = std::env::temp_dir().join(format!(
            "adaq-m6-examples-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let state = LocalResearchState::open(&root).unwrap();
        let factor_bytes = public_example_package("factor-close-momentum-5");
        let strategy_bytes = public_example_package("strategy-momentum-trend");
        state.components.import("alice", &factor_bytes).unwrap();
        state.components.import("alice", &strategy_bytes).unwrap();
        let factor_hash = ComponentPackage::read(&factor_bytes)
            .unwrap()
            .archive_sha256;
        let strategy_hash = ComponentPackage::read(&strategy_bytes)
            .unwrap()
            .archive_sha256;
        assert_eq!(
            state
                .components
                .compatible_factors("alice", &strategy_hash)
                .unwrap()["momentum"],
            [factor_hash.clone()]
        );
        let bars = (0..50)
            .map(|index| {
                let close = rust_decimal::Decimal::from(100 + index);
                let time_index = if index < 25 { index } else { index + 5 };
                OhlcvBar {
                    open_time_ms: time_index * 3_600_000,
                    open: close,
                    high: close,
                    low: close,
                    close,
                    base_volume: rust_decimal::Decimal::ONE,
                    quote_volume: close,
                }
            })
            .collect();
        let snapshot = state
            .persist_snapshot_for_user(
                "alice",
                &adaq_data_core::BarSeries {
                    src: "okx".into(),
                    code: "BTC-USDT".into(),
                    interval: BarInterval::OneHour,
                    bars,
                    gaps: vec![BarGap {
                        start_time_ms: 25 * 3_600_000,
                        end_time_ms: 30 * 3_600_000,
                    }],
                },
            )
            .unwrap();
        let request = || BacktestRunRequest {
            user_id: "alice".into(),
            snapshot_id: snapshot.snapshot_id.clone(),
            run_start_time_ms: None,
            run_end_time_ms: None,
            factor_instances: vec![FactorInstanceRequest {
                alias: "momentum".into(),
                archive_sha256: factor_hash.clone(),
                parameters: HashMap::new(),
            }],
            signal_instances: vec![],
            strategy_archive_sha256: strategy_hash.clone(),
            strategy_parameters: HashMap::new(),
            initial_quote_allocation: 10_000.into(),
            execution_profile: ExecutionProfile {
                maker_fee_rate: rust_decimal::Decimal::new(8, 4),
                taker_fee_rate: rust_decimal::Decimal::new(1, 3),
                adverse_slippage_rate: rust_decimal::Decimal::ZERO,
                rebalance_threshold: rust_decimal::Decimal::ZERO,
                price_increment: rust_decimal::Decimal::ONE,
                quantity_increment: rust_decimal::Decimal::new(1, 4),
                minimum_quantity: rust_decimal::Decimal::new(1, 4),
                risk_free_rate: rust_decimal::Decimal::ZERO,
                fill_policy: adaq_backtest_core::FillPolicy::Taker,
            },
            seed: 0,
        };

        let first = state.backtests.run(request()).unwrap();
        let replay = state.backtests.run(request()).unwrap();

        assert_eq!(first.run_id, replay.run_id);
        let provenance = first.provenance.as_ref().unwrap();
        assert_eq!(
            provenance.normalized_request.snapshot_id,
            snapshot.snapshot_id
        );
        assert_eq!(provenance.feature_plan_hash, first.plan_hash);
        assert_eq!(provenance.component_lock, first.component_lock);
        assert_eq!(
            state
                .backtests
                .get("alice", &first.run_id)
                .unwrap()
                .provenance,
            first.provenance
        );
        assert!(provenance.feature_plan_json.contains("\"slots\""));
        assert_eq!(
            provenance.normalized_request.initial_quote_allocation,
            rust_decimal::Decimal::from(10_000),
        );
        assert_eq!(first.component_lock.len(), 2);
        assert_eq!(first.pauses.len(), 38);
        assert!(!first.result.orders.is_empty());
        assert!(!first.result.fills.is_empty());
        let execution_page = state
            .backtests
            .execution_data(&crate::backtest::BacktestExecutionRequest {
                user_id: "alice".into(),
                run_id: first.run_id.clone(),
                offset: 0,
                limit: 1,
            })
            .unwrap();
        assert_eq!(execution_page.orders.len(), 1);
        assert_eq!(execution_page.fills.len(), 1);
        assert_eq!(execution_page.total_orders, first.result.orders.len());
        assert_eq!(execution_page.total_fills, first.result.fills.len());
        let run_count = || {
            state
                .backtests
                .list(&crate::backtest::BacktestListRequest {
                    user_id: "alice".into(),
                    src: None,
                    code: None,
                    page: 1,
                })
                .unwrap()
                .total
        };
        assert_eq!(run_count(), 1);
        let mut changed_request = request();
        changed_request.seed = 1;
        let changed = state.backtests.run(changed_request).unwrap();
        assert_ne!(first.run_id, changed.run_id);
        assert_eq!(run_count(), 2);

        // Validation Studies is a deep module: Protocols and Reports flow
        // through its interface, and an immutable Report locks the Backtest
        // Runs it references.
        let validation = crate::validation::ValidationProtocolCreateRequest {
            user_id: "alice".into(),
            run: request(),
            windows: vec![crate::validation::ValidationWindowRequest {
                snapshot_id: snapshot.snapshot_id.clone(),
                sample_out_start_time_ms: 25 * 3_600_000,
                sample_out_end_time_ms: None,
            }],
            walk_forward: None,
            cross_market: None,
            method_version: "chronological-holdout@1".into(),
            aggregation_rule_version: "equal-window@1".into(),
        };
        assert!(
            state
                .validation
                .create_protocol(crate::validation::ValidationProtocolCreateRequest {
                    windows: vec![crate::validation::ValidationWindowRequest {
                        snapshot_id: snapshot.snapshot_id.clone(),
                        sample_out_start_time_ms: 0,
                        sample_out_end_time_ms: None,
                    }],
                    ..validation.clone()
                })
                .is_err()
        );
        let protocol = state.validation.create_protocol(validation).unwrap();
        let report = state
            .validation
            .run_report(&protocol.user_id, &protocol.protocol_id)
            .unwrap();
        assert_eq!(report.aggregate.completed_windows, 1);
        assert_eq!(report.windows.len(), 1);
        assert_eq!(report.windows[0].sample_out_start_time_ms, 25 * 3_600_000);
        assert!(
            state
                .backtests
                .delete(
                    "alice",
                    report.windows[0].sample_in_run_id.as_deref().unwrap(),
                )
                .is_err()
        );
        assert_eq!(state.validation.list_reports("alice").unwrap().len(), 1);
        assert!(state.validation.list_reports("bob").unwrap().is_empty());
        let markdown = state
            .validation
            .export_report("alice", &report.report_id, "markdown")
            .unwrap();
        assert!(markdown.contains(&report.protocol_id));
        assert!(markdown.contains("research-metrics.md"));

        drop(state);
        fs::remove_dir_all(root).unwrap();
    }
}
