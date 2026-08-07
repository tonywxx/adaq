use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Weak},
};

use adaq_backtest_core::{MarketDataSnapshot, SnapshotStore};
use adaq_component_tooling::{
    ComponentDependency, ComponentKind, ComponentManifest, ComponentPackage, FeatureSlotDefinition,
    FeatureSlotSource, ModelArtifact, ModelOutput, ModelScope, ParameterDefinition,
    StrategyArchitecture, strategy_architecture, verify_package,
};
use adaq_data_core::OhlcvBar;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use tauri::Manager;

use crate::{
    backtest::{BacktestSource, Backtests, ComponentPackageSource, SnapshotReadSource},
    dataset_generation::{DatasetGeneration, GenerationSource},
    m8::{BacktestSignalDataset, backtest_signal_datasets},
    market_data_snapshot::{LocalSnapshotSource, MarketDataSnapshots},
    user::validate_user,
    validation::{ValidationRunOutcome, ValidationSource, ValidationStudies},
};

const COMPONENT_PAGE_SIZE: usize = 10;

pub struct LocalResearchState {
    pub(crate) root: PathBuf,
    pub(crate) database: Arc<Mutex<Connection>>,
    pub(crate) snapshots: MarketDataSnapshots,
    source: Arc<LocalGenerationSource>,
    pub(crate) generation: DatasetGeneration,
    pub(crate) validation: ValidationStudies,
    pub(crate) backtests: Backtests,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryComponent {
    component_id: String,
    version: String,
    manifest_schema_version: String,
    sdk_version: String,
    abi_version: String,
    name: String,
    kind: String,
    archive_sha256: String,
    wasm_sha256: String,
    parameters: Vec<ParameterDefinition>,
    feature_slots: Vec<FeatureSlotDefinition>,
    output_names: Vec<String>,
    dependencies: Vec<ComponentDependency>,
    warmup_bars: u32,
    model_scope: Option<ModelScope>,
    model_outputs: Vec<ModelOutput>,
    model_artifact: Option<ModelArtifact>,
    architecture: Option<StrategyArchitecture>,
    compatible: bool,
    compatibility_error: Option<String>,
    locked_by_run_ids: Vec<String>,
}

/// The concrete local dependencies composed into the Dataset Generation
/// lifecycle module. Only database access, Component Package access, Market
/// Data Snapshot access, runtime Component materialization, and the Signal
/// Dataset directory are shared; the complete Local Research state is not.
pub(crate) struct LocalGenerationSource {
    database: Arc<Mutex<Connection>>,
    snapshots: MarketDataSnapshots,
    root: PathBuf,
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
        validate_user(user_id)?;
        let database = self.database.lock().map_err(string)?;
        let (path, archive_sha256, wasm_sha256): (String, String, String) = database
            .query_row(
                "SELECT c.archive_path, c.archive_sha256, c.wasm_sha256 FROM component_content c
                 JOIN component_access a USING(archive_sha256)
                 WHERE a.user_id = ?1 AND c.archive_sha256 = ?2",
                params![user_id, hash],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|_| "Component Package is not available to this User".to_owned())?;
        drop(database);
        let package = ComponentPackage::read(&fs::read(path).map_err(string)?).map_err(string)?;
        verify_package(&package)?;
        if package.archive_sha256 != archive_sha256 || package.manifest.wasm_sha256 != wasm_sha256 {
            return Err("Component Package does not match stored identity or hashes".into());
        }
        Ok(package)
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
/// access, Signal Dataset reads through the m8-owned path, and the
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
             CREATE TABLE IF NOT EXISTS component_content (
                archive_sha256 TEXT PRIMARY KEY,
                component_id TEXT NOT NULL,
                version TEXT NOT NULL,
                name TEXT NOT NULL,
                kind TEXT NOT NULL,
                wasm_sha256 TEXT NOT NULL,
                archive_path TEXT NOT NULL,
                metadata_json TEXT NOT NULL DEFAULT '',
                UNIQUE(component_id, version)
             );
             CREATE TABLE IF NOT EXISTS component_access (
                user_id TEXT NOT NULL,
                archive_sha256 TEXT NOT NULL,
                PRIMARY KEY(user_id, archive_sha256),
                FOREIGN KEY(archive_sha256) REFERENCES component_content(archive_sha256)
             );
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
        let snapshot_source = Arc::new(LocalSnapshotSource::new(
            database.clone(),
            Arc::new(snapshot_store),
        ));
        let snapshots = MarketDataSnapshots::open(snapshot_source)?;
        let source = Arc::new(LocalGenerationSource {
            database: database.clone(),
            snapshots: snapshots.clone(),
            root: root.clone(),
        });
        let generation = DatasetGeneration::open(source.clone())?;
        let validation_source = Arc::new(LocalValidationSource {
            database: database.clone(),
            state: Mutex::new(Weak::new()),
        });
        let validation = ValidationStudies::open(validation_source.clone())?;
        let backtest_source = Arc::new(LocalBacktestSource {
            database: database.clone(),
            state: Mutex::new(Weak::new()),
        });
        let backtests = Backtests::open(backtest_source.clone())?;
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
                snapshots,
                source,
                generation,
                validation,
                backtests,
            }
        }))
    }

    pub fn local_data_summary(&self, user_id: &str) -> Result<LocalDataSummary, String> {
        validate_user(user_id)?;
        let generation_attempt_count = self.generation.list(user_id)?.len() as u64;
        let validation = self.validation.summary_for_user(user_id)?;
        // Query the Snapshot and Backtest modules before locking the
        // database mutex so the hooks never re-enter a held lock.
        let snapshots = self.snapshots.summary_for_user(user_id)?;
        let backtests = self.backtests.summary_for_user(user_id)?;
        let database = self.database.lock().map_err(string)?;
        let count = |sql: &str| -> Result<u64, String> {
            database
                .query_row(sql, [user_id], |row| row.get::<_, i64>(0))
                .map(|value| value.max(0) as u64)
                .map_err(string)
        };
        let component_paths = strings(
            &database,
            "SELECT c.archive_path FROM component_content c
			 JOIN component_access a USING(archive_sha256) WHERE a.user_id = ?1",
            user_id,
        )?;
        let owned_components = owned_component_hashes(&database, user_id)?;
        let locking_runs = self.backtests.runs_locking_components(&database, user_id)?;
        let component_blocking_run_count =
            count_runs_locking_owned_components(&locking_runs, &owned_components);
        let database_path = self.root.parent().unwrap_or(&self.root).join("adaq.db");
        let data_directory = database_path
            .parent()
            .unwrap_or(&self.root)
            .to_string_lossy()
            .into_owned();

        Ok(LocalDataSummary {
            data_directory,
            database_bytes: file_bytes(&database_path),
            component_bytes: component_paths.iter().map(file_bytes).sum(),
            market_data_bytes: snapshots.market_data_bytes,
            watchlist_count: count("SELECT COUNT(*) FROM watchlist_items WHERE user_id = ?1")?,
            component_count: count("SELECT COUNT(*) FROM component_access WHERE user_id = ?1")?,
            snapshot_count: snapshots.snapshot_count,
            run_count: backtests.run_count,
            protocol_count: validation.protocol_count,
            report_count: validation.report_count,
            generation_attempt_count,
            model_artifact_count: count(
                "SELECT COUNT(*) FROM component_content c JOIN component_access a USING(archive_sha256) WHERE a.user_id = ?1 AND c.kind = 'model'",
            )?,
            signal_dataset_count: count(
                "SELECT COUNT(*) FROM signal_dataset_access WHERE user_id = ?1",
            )?,
            component_blocking_run_count,
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
        // Query the Validation module before locking the database mutex so
        // the hook never re-enters a held lock. The Snapshot orphan query
        // and the Backtest hooks run inside the reset flows under the held
        // lock instead, so they stay serialized with Snapshot persistence
        // and Run writes.
        let validation_report_count = if matches!(kind, LocalDataResetKind::MarketData) {
            self.validation.summary_for_user(user_id)?.report_count
        } else {
            0
        };
        let mut database = self.database.lock().map_err(string)?;
        match kind {
            LocalDataResetKind::Watchlist => reset_watchlist(&mut database, user_id),
            LocalDataResetKind::Components => {
                reset_components(&mut database, user_id, &self.root, &self.backtests)
            }
            LocalDataResetKind::MarketData => reset_market_data(
                &mut database,
                user_id,
                &self.root,
                validation_report_count,
                &self.snapshots,
                &self.backtests,
            ),
            LocalDataResetKind::All => reset_all(
                &mut database,
                user_id,
                &self.root,
                _reset_block.as_ref().unwrap(),
                &self.validation,
                &self.snapshots,
                &self.backtests,
            ),
        }
    }

    pub fn import_component(
        &self,
        user_id: &str,
        bytes: &[u8],
    ) -> Result<LibraryComponent, String> {
        validate_user(user_id)?;
        let package = ComponentPackage::read(bytes).map_err(string)?;
        verify_package(&package)?;
        let component_id = package.manifest.component_id.to_string();
        let version = package.manifest.version.to_string();
        let sdk_version = package.manifest.sdk_version.to_string();
        let kind = format!("{:?}", package.manifest.kind).to_lowercase();
        let mut database = self.database.lock().map_err(string)?;
        let existing: Option<(String, String)> = database.query_row(
            "SELECT archive_sha256, wasm_sha256 FROM component_content WHERE component_id = ?1 AND version = ?2",
            params![component_id, version], |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional().map_err(string)?;
        if existing.as_ref().is_some_and(|(archive, wasm)| {
            archive != &package.archive_sha256 || wasm != &package.manifest.wasm_sha256
        }) {
            return Err("A different Component already uses this identity and version".into());
        }
        let path = self
            .root
            .join("components")
            .join(format!("{}.adaq", package.archive_sha256));
        if !path.is_file() {
            fs::write(&path, bytes).map_err(string)?;
        }
        let transaction = database.transaction().map_err(string)?;
        let component = LibraryComponent {
            component_id,
            version,
            manifest_schema_version: package.manifest.manifest_schema_version.to_string(),
            sdk_version,
            abi_version: package.manifest.abi_version.to_string(),
            architecture: strategy_architecture(&package.manifest),
            name: package.manifest.name,
            kind,
            archive_sha256: package.archive_sha256,
            wasm_sha256: package.manifest.wasm_sha256,
            parameters: package.manifest.parameters,
            feature_slots: package.manifest.feature_slots,
            output_names: package.manifest.output_names,
            dependencies: package.manifest.dependencies,
            warmup_bars: package.manifest.warmup_bars,
            model_scope: package.manifest.model_scope,
            model_outputs: package.manifest.model_outputs,
            model_artifact: package.manifest.model_artifact,
            compatible: true,
            compatibility_error: None,
            locked_by_run_ids: vec![],
        };
        let metadata_json = serde_json::to_string(&component).map_err(string)?;
        transaction
            .execute(
                "INSERT OR IGNORE INTO component_content
             (archive_sha256, component_id, version, name, kind, wasm_sha256, archive_path, metadata_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    component.archive_sha256,
                    component.component_id,
                    component.version,
                    component.name,
                    component.kind,
                    component.wasm_sha256,
                    path.to_string_lossy(),
                    metadata_json,
                ],
            )
            .map_err(string)?;
        transaction
            .execute(
                "INSERT OR IGNORE INTO component_access(user_id, archive_sha256) VALUES (?1, ?2)",
                params![user_id, component.archive_sha256],
            )
            .map_err(string)?;
        transaction.commit().map_err(string)?;
        Ok(component)
    }

    pub fn list_components(&self, user_id: &str) -> Result<Vec<LibraryComponent>, String> {
        self.list_components_range(user_id, -1, 0)
    }

    fn list_components_page(&self, user_id: &str, page: usize) -> Result<ComponentPage, String> {
        validate_user(user_id)?;
        if page == 0 {
            return Err("Component Package page is invalid".into());
        }
        let total = self
            .database
            .lock()
            .map_err(string)?
            .query_row(
                "SELECT COUNT(*) FROM component_content c
                 JOIN component_access a USING(archive_sha256)
                 WHERE a.user_id = ?1",
                [user_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(string)?
            .try_into()
            .map_err(|_| "Component Package count is invalid")?;
        let offset = page
            .checked_sub(1)
            .and_then(|value| value.checked_mul(COMPONENT_PAGE_SIZE))
            .ok_or_else(|| "Component Package page is too large".to_owned())?;
        Ok(ComponentPage {
            items: self.list_components_range(
                user_id,
                COMPONENT_PAGE_SIZE as i64,
                offset as i64,
            )?,
            total,
            page,
            page_size: COMPONENT_PAGE_SIZE,
        })
    }

    fn list_components_range(
        &self,
        user_id: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<LibraryComponent>, String> {
        validate_user(user_id)?;
        let database = self.database.lock().map_err(string)?;
        // Query the Backtest module on the held connection so the locks
        // observed here match the rows the listing reads.
        let locked_by_hash = self.backtests.runs_locking_components(&database, user_id)?;
        let mut statement = database
            .prepare(
                "SELECT c.component_id, c.version, c.name, c.kind, c.archive_sha256, c.wasm_sha256, c.archive_path, c.metadata_json
             FROM component_content c JOIN component_access a USING(archive_sha256)
             WHERE a.user_id = ?1 ORDER BY c.name, c.version, c.archive_sha256
             LIMIT ?2 OFFSET ?3",
            )
            .map_err(string)?;
        statement
            .query_map(params![user_id, limit, offset], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                ))
            })
            .map_err(string)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(string)?
            .into_iter()
            .map(
                |(
                    component_id,
                    version,
                    name,
                    kind,
                    archive_sha256,
                    wasm_sha256,
                    path,
                    metadata_json,
                )| {
                    if !metadata_json.is_empty() {
                        return serde_json::from_str::<LibraryComponent>(&metadata_json)
                            .map(|mut component| {
                                component.component_id = component_id;
                                component.version = version;
                                component.name = name;
                                component.kind = kind;
                                component.archive_sha256 = archive_sha256.clone();
                                component.wasm_sha256 = wasm_sha256;
                                component.locked_by_run_ids = locked_by_hash
                                    .get(&archive_sha256)
                                    .cloned()
                                    .unwrap_or_default();
                                component
                            })
                            .map_err(string);
                    }
                    match fs::read(path)
                        .map_err(string)
                        .and_then(|bytes| ComponentPackage::read(&bytes).map_err(string))
                        .and_then(|package| {
                            verify_package(&package)?;
                            let package_kind =
                                format!("{:?}", package.manifest.kind).to_lowercase();
                            if package.archive_sha256 != archive_sha256
                                || package.manifest.component_id.to_string() != component_id
                                || package.manifest.version.to_string() != version
                                || package.manifest.name != name
                                || package_kind != kind
                                || package.manifest.wasm_sha256 != wasm_sha256
                            {
                                return Err(
                                    "Component Package does not match stored identity or hashes"
                                        .into(),
                                );
                            }
                            Ok(package)
                        }) {
                        Ok(package) => Ok(LibraryComponent {
                            component_id,
                            version,
                            manifest_schema_version: package
                                .manifest
                                .manifest_schema_version
                                .to_string(),
                            sdk_version: package.manifest.sdk_version.to_string(),
                            abi_version: package.manifest.abi_version.to_string(),
                            name,
                            kind,
                            locked_by_run_ids: locked_by_hash
                                .get(&archive_sha256)
                                .cloned()
                                .unwrap_or_default(),
                            archive_sha256,
                            wasm_sha256,
                            architecture: strategy_architecture(&package.manifest),
                            parameters: package.manifest.parameters,
                            feature_slots: package.manifest.feature_slots,
                            output_names: package.manifest.output_names,
                            dependencies: package.manifest.dependencies,
                            warmup_bars: package.manifest.warmup_bars,
                            model_scope: package.manifest.model_scope,
                            model_outputs: package.manifest.model_outputs,
                            model_artifact: package.manifest.model_artifact,
                            compatible: true,
                            compatibility_error: None,
                        }),
                        Err(error) => Ok(LibraryComponent {
                            component_id,
                            version,
                            manifest_schema_version: String::new(),
                            sdk_version: String::new(),
                            abi_version: String::new(),
                            name,
                            kind,
                            locked_by_run_ids: locked_by_hash
                                .get(&archive_sha256)
                                .cloned()
                                .unwrap_or_default(),
                            archive_sha256,
                            wasm_sha256,
                            parameters: vec![],
                            feature_slots: vec![],
                            output_names: vec![],
                            dependencies: vec![],
                            warmup_bars: 0,
                            model_scope: None,
                            model_outputs: vec![],
                            model_artifact: None,
                            architecture: None,
                            compatible: false,
                            compatibility_error: Some(format!(
                                "Incompatible Component Package: {error}"
                            )),
                        }),
                    }
                },
            )
            .collect()
    }

    pub fn delete_component(&self, user_id: &str, hash: &str) -> Result<(), String> {
        validate_user(user_id)?;
        let mut database = self.database.lock().map_err(string)?;
        let entitled: bool = database
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM component_access WHERE user_id = ?1 AND archive_sha256 = ?2)",
                params![user_id, hash],
                |row| row.get(0),
            )
            .map_err(string)?;
        if !entitled {
            return Err("Component Package is not available to this User".into());
        }
        // Query the Backtest module on the held connection so the locks
        // observed here match the rows the deletion removes.
        let locked_by_hash = self.backtests.runs_locking_components(&database, user_id)?;
        let locked_by_run_ids = locked_by_hash.get(hash).cloned().unwrap_or_default();
        if !locked_by_run_ids.is_empty() {
            let noun = if locked_by_run_ids.len() == 1 {
                "Backtest Run"
            } else {
                "Backtest Runs"
            };
            return Err(format!(
                "Component Package is locked by {noun}: {}",
                locked_by_run_ids.join(", ")
            ));
        }
        let locked_by_dataset_ids = database
            .prepare(
                "SELECT c.dataset_id FROM signal_dataset_content c
                 JOIN signal_dataset_access a USING(dataset_id)
                 WHERE a.user_id = ?1 AND (
                    json_extract(c.metadata_json, '$.modelArchiveSha256') = ?2
                    OR EXISTS(SELECT 1 FROM json_each(c.metadata_json, '$.componentLock') WHERE json_extract(value, '$.archiveSha256') = ?2)
                    OR EXISTS(SELECT 1 FROM json_each(c.metadata_json, '$.externalProducerSegments') WHERE json_extract(value, '$.modelArtifact.sha256') = ?2)
                 )
                 ORDER BY c.dataset_id",
            )
            .map_err(string)?
            .query_map(params![user_id, hash], |row| row.get::<_, String>(0))
            .map_err(string)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(string)?;
        if !locked_by_dataset_ids.is_empty() {
            return Err(format!(
                "Component Package is locked by immutable Signal Dataset(s): {}",
                locked_by_dataset_ids.join(", ")
            ));
        }
        let transaction = database.transaction().map_err(string)?;
        transaction
            .execute(
                "DELETE FROM component_access WHERE user_id = ?1 AND archive_sha256 = ?2",
                params![user_id, hash],
            )
            .map_err(string)?;
        let remaining: i64 = transaction
            .query_row(
                "SELECT COUNT(*) FROM component_access WHERE archive_sha256 = ?1",
                [hash],
                |row| row.get(0),
            )
            .map_err(string)?;
        let path = if remaining == 0 {
            transaction
                .query_row(
                    "SELECT archive_path FROM component_content WHERE archive_sha256 = ?1",
                    [hash],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(string)?
        } else {
            None
        };
        if remaining == 0 {
            transaction
                .execute(
                    "DELETE FROM component_content WHERE archive_sha256 = ?1",
                    [hash],
                )
                .map_err(string)?;
        }
        transaction.commit().map_err(string)?;
        if let Some(path) = path {
            let _ = fs::remove_file(path);
        }
        Ok(())
    }

    pub fn component_is_imported(&self, user_id: &str, hash: &str) -> Result<bool, String> {
        validate_user(user_id)?;
        self.database
            .lock()
            .map_err(string)?
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM component_access WHERE user_id = ?1 AND archive_sha256 = ?2)",
                params![user_id, hash],
                |row| row.get(0),
            )
            .map_err(string)
    }

    pub(crate) fn persist_snapshot_for_user(
        &self,
        user_id: &str,
        series: &adaq_data_core::BarSeries,
    ) -> Result<MarketDataSnapshot, String> {
        self.snapshots.persist_for_user(user_id, series)
    }

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
        self.source.package_for_user(user_id, hash)
    }

    fn compatible_factors(
        &self,
        user_id: &str,
        consumer_archive_sha256: &str,
    ) -> Result<BTreeMap<String, Vec<String>>, String> {
        let consumer = self.package_for_user(user_id, consumer_archive_sha256)?;
        if !matches!(
            consumer.manifest.kind,
            ComponentKind::Strategy | ComponentKind::Model
        ) {
            return Err("Compatible Factors require a Strategy or Model Component".into());
        }
        let components = self.list_components(user_id)?;
        let packages = components
            .iter()
            .filter(|component| component.kind == "factor" && component.compatible)
            .map(|component| {
                self.package_for_user(user_id, &component.archive_sha256)
                    .map(|package| (component, package))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(compatible_factor_hashes(&consumer.manifest, &packages))
    }

    pub(crate) fn snapshot_for_user(
        &self,
        user_id: &str,
        snapshot_id: &str,
    ) -> Result<(MarketDataSnapshot, Vec<OhlcvBar>), String> {
        self.snapshots.snapshot_for_user(user_id, snapshot_id)
    }

    pub(crate) fn runtime_component(&self, package: &ComponentPackage) -> Result<PathBuf, String> {
        self.source.runtime_component(package)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentUserRequest {
    pub user_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentPageRequest {
    pub user_id: String,
    pub page: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentPage {
    pub items: Vec<LibraryComponent>,
    pub total: usize,
    pub page: usize,
    pub page_size: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentImportRequest {
    pub user_id: String,
    pub bytes: Vec<u8>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentDeleteRequest {
    pub user_id: String,
    pub archive_sha256: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentArchiveRequest {
    pub user_id: String,
    pub archive_sha256: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestDependencyRequest {
    pub user_id: String,
    pub strategy_archive_sha256: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestSignalCandidate {
    slot: String,
    dataset_id: String,
    signal_name: String,
    evidence_state: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestSignalCompatibilityRequest {
    user_id: String,
    strategy_archive_sha256: String,
    snapshot_id: String,
}

#[tauri::command]
pub fn component_import(
    request: ComponentImportRequest,
    state: tauri::State<'_, Arc<LocalResearchState>>,
) -> Result<LibraryComponent, String> {
    state.import_component(&request.user_id, &request.bytes)
}

#[tauri::command]
pub async fn component_list(
    request: ComponentUserRequest,
    app: tauri::AppHandle,
) -> Result<Vec<LibraryComponent>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .list_components(&request.user_id)
    })
    .await
    .map_err(string)?
}

#[tauri::command]
pub async fn component_page(
    request: ComponentPageRequest,
    app: tauri::AppHandle,
) -> Result<ComponentPage, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Arc<LocalResearchState>>()
            .list_components_page(&request.user_id, request.page)
    })
    .await
    .map_err(string)?
}

#[tauri::command]
pub fn component_is_imported(
    request: ComponentArchiveRequest,
    state: tauri::State<'_, Arc<LocalResearchState>>,
) -> Result<bool, String> {
    state.component_is_imported(&request.user_id, &request.archive_sha256)
}

#[tauri::command]
pub fn backtest_compatible_factors(
    request: BacktestDependencyRequest,
    state: tauri::State<'_, Arc<LocalResearchState>>,
) -> Result<BTreeMap<String, Vec<String>>, String> {
    state.compatible_factors(&request.user_id, &request.strategy_archive_sha256)
}

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
    datasets: &[crate::m8::BacktestSignalDataset],
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

fn compatible_factor_hashes(
    strategy: &ComponentManifest,
    packages: &[(&LibraryComponent, ComponentPackage)],
) -> BTreeMap<String, Vec<String>> {
    strategy
        .dependencies
        .iter()
        .map(|dependency| {
            let hashes = packages
                .iter()
                .filter(|(component, package)| {
                    component.kind == "factor"
                        && component.compatible
                        && package.manifest.component_id == dependency.component_id
                        && dependency.version.matches(&package.manifest.version)
                })
                .map(|(component, _)| component.archive_sha256.clone())
                .collect();
            (dependency.alias.clone(), hashes)
        })
        .collect()
}

#[tauri::command]
pub fn component_delete(
    request: ComponentDeleteRequest,
    state: tauri::State<'_, Arc<LocalResearchState>>,
) -> Result<(), String> {
    state.delete_component(&request.user_id, &request.archive_sha256)
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

fn reset_components(
    database: &mut Connection,
    user_id: &str,
    root: &Path,
    backtests: &Backtests,
) -> Result<(), String> {
    let owned_components = owned_component_hashes(database, user_id)?;
    let locking_runs = backtests.runs_locking_components(database, user_id)?;
    let blocking_runs =
        count_runs_locking_owned_components(&locking_runs, &owned_components) as i64;
    let blocking_datasets: i64 = database
        .query_row(
            "SELECT COUNT(*) FROM signal_dataset_access WHERE user_id = ?1",
            [user_id],
            |row| row.get(0),
        )
        .map_err(string)?;
    let blocking = blocking_runs + blocking_datasets;
    if blocking > 0 {
        return Err(format!(
            "Component Package reset is blocked by {blocking} immutable Backtest Run(s)"
        ));
    }
    // The Run-lock guard comes from the Backtest module; the composition
    // root never issues SQL over the Run bridge tables itself.
    let locked_by_runs = backtests.component_hashes_locked_by_runs(database, None)?;
    let paths = orphan_component_candidates(database, user_id)?
        .into_iter()
        .filter(|(hash, _)| !locked_by_runs.contains(hash))
        .map(|(_, path)| path)
        .collect::<Vec<_>>();
    let staged = stage_files(paths.iter().map(PathBuf::from), root)?;
    let result = (|| {
        let transaction = database.transaction().map_err(string)?;
        transaction
            .execute("DELETE FROM component_access WHERE user_id = ?1", [user_id])
            .map_err(string)?;
        delete_orphan_component_content(&transaction, &locked_by_runs)?;
        transaction.commit().map_err(string)
    })();
    finish_staged_files(staged, result)
}

fn reset_market_data(
    database: &mut Connection,
    user_id: &str,
    root: &Path,
    validation_report_count: u64,
    snapshots: &MarketDataSnapshots,
    backtests: &Backtests,
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
        + validation_report_count;
    if blocking > 0 {
        return Err(format!(
            "Market Data reset is blocked by {blocking} immutable research record(s)"
        ));
    }
    let staged = stage_files(snapshots.orphaned_parquet_paths(database, user_id)?, root)?;
    let result = (|| {
        let transaction = database.transaction().map_err(string)?;
        snapshots.reset_for_user(&transaction, user_id)?;
        transaction.commit().map_err(string)
    })();
    finish_staged_files(staged, result)
}

fn reset_all(
    database: &mut Connection,
    user_id: &str,
    root: &Path,
    reset_block: &crate::dataset_generation::UserResetBlock<'_>,
    validation: &ValidationStudies,
    snapshots: &MarketDataSnapshots,
    backtests: &Backtests,
) -> Result<(), String> {
    // The reset User's Runs are deleted inside the transaction below, so
    // only other Users' Runs keep locking Component content; that set is
    // stable under the held database lock and guards both the staged file
    // selection and the transaction's orphan cleanup.
    let locked_by_runs = backtests.component_hashes_locked_by_runs(database, Some(user_id))?;
    let component_paths = orphan_component_candidates(database, user_id)?
        .into_iter()
        .filter(|(hash, _)| !locked_by_runs.contains(hash))
        .map(|(_, path)| path)
        .collect::<Vec<_>>();
    let dataset_paths = strings(
        database,
        "SELECT c.parquet_path FROM signal_dataset_content c JOIN signal_dataset_access a USING(dataset_id) WHERE a.user_id = ?1 AND NOT EXISTS(SELECT 1 FROM signal_dataset_access other WHERE other.dataset_id = c.dataset_id AND other.user_id <> ?1)",
        user_id,
    )?;
    let staged = stage_files(
        component_paths
            .into_iter()
            .map(PathBuf::from)
            .chain(snapshots.orphaned_parquet_paths(database, user_id)?)
            .chain(dataset_paths.into_iter().map(PathBuf::from)),
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
        transaction
            .execute("DELETE FROM component_access WHERE user_id = ?1", [user_id])
            .map_err(string)?;
        snapshots.reset_for_user(&transaction, user_id)?;
        delete_orphan_component_content(&transaction, &locked_by_runs)?;
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

/// Deletes Component content rows nobody can read anymore, skipping the
/// hashes the Backtest module reports as still locked by Runs.
fn delete_orphan_component_content(
    transaction: &Transaction<'_>,
    locked_by_runs: &HashSet<String>,
) -> Result<(), String> {
    let mut statement = transaction
        .prepare(
            "SELECT archive_sha256 FROM component_content
             WHERE NOT EXISTS(SELECT 1 FROM component_access a
                 WHERE a.archive_sha256 = component_content.archive_sha256)",
        )
        .map_err(string)?;
    let orphans = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(string)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(string)?;
    for hash in orphans
        .iter()
        .filter(|hash| !locked_by_runs.contains(*hash))
    {
        transaction
            .execute(
                "DELETE FROM component_content WHERE archive_sha256 = ?1",
                [hash],
            )
            .map_err(string)?;
    }
    Ok(())
}

fn owned_component_hashes(database: &Connection, user_id: &str) -> Result<HashSet<String>, String> {
    let mut statement = database
        .prepare("SELECT archive_sha256 FROM component_access WHERE user_id = ?1")
        .map_err(string)?;
    statement
        .query_map([user_id], |row| row.get::<_, String>(0))
        .map_err(string)?
        .collect::<Result<HashSet<_>, _>>()
        .map_err(string)
}

/// The Component content one User accesses that no other User accesses.
/// Both Component Reset and Reset All stage and prune from this candidate
/// set after applying the Backtest module's Run-lock guard.
fn orphan_component_candidates(
    database: &Connection,
    user_id: &str,
) -> Result<Vec<(String, String)>, String> {
    let mut statement = database
        .prepare(
            "SELECT c.archive_sha256, c.archive_path FROM component_content c
             JOIN component_access a USING(archive_sha256)
             WHERE a.user_id = ?1
             AND NOT EXISTS(SELECT 1 FROM component_access other
                 WHERE other.archive_sha256 = c.archive_sha256 AND other.user_id <> ?1)",
        )
        .map_err(string)?;
    statement
        .query_map([user_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(string)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(string)
}

/// The distinct Runs of one User that lock Component Packages the User
/// still owns; the count both the summary and the Component Reset blocking
/// check report.
fn count_runs_locking_owned_components(
    locking_runs: &HashMap<String, Vec<String>>,
    owned_components: &HashSet<String>,
) -> u64 {
    locking_runs
        .iter()
        .filter(|(hash, _)| owned_components.contains(*hash))
        .flat_map(|(_, runs)| runs)
        .collect::<HashSet<_>>()
        .len() as u64
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

fn stage_files(
    paths: impl IntoIterator<Item = PathBuf>,
    allowed_root: &Path,
) -> Result<Vec<(PathBuf, PathBuf)>, String> {
    let mut staged = Vec::new();
    for path in paths {
        if !path.starts_with(allowed_root) {
            restore_staged_files(&staged);
            return Err(format!(
                "Refusing to reset a file outside the local data store: {}",
                path.display()
            ));
        }
        if !path.is_file() {
            continue;
        }
        let temporary = path.with_extension(format!(
            "{}.reset",
            path.extension()
                .and_then(|value| value.to_str())
                .unwrap_or("data")
        ));
        if temporary.exists() {
            restore_staged_files(&staged);
            return Err(format!(
                "Reset staging path already exists: {}",
                temporary.display()
            ));
        }
        if let Err(error) = fs::rename(&path, &temporary) {
            restore_staged_files(&staged);
            return Err(error.to_string());
        }
        staged.push((path, temporary));
    }
    Ok(staged)
}

fn finish_staged_files(
    staged: Vec<(PathBuf, PathBuf)>,
    result: Result<(), String>,
) -> Result<(), String> {
    if let Err(error) = result {
        restore_staged_files(&staged);
        return Err(error);
    }
    for (_, temporary) in staged {
        let _ = fs::remove_file(temporary);
    }
    Ok(())
}

fn restore_staged_files(staged: &[(PathBuf, PathBuf)]) {
    for (path, temporary) in staged.iter().rev() {
        let _ = fs::rename(temporary, path);
    }
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
    use adaq_component_tooling::{ComponentManifest, pack_component};
    use adaq_data_core::{BarGap, BarInterval};
    use std::{
        io::{Cursor, Write},
        time::{SystemTime, UNIX_EPOCH},
    };
    use zip::{ZipWriter, write::SimpleFileOptions};

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
        let dataset = crate::m8::BacktestSignalDataset {
            dataset_id: "a".repeat(64),
            snapshot_id: snapshot.snapshot_id.clone(),
            src: snapshot.src.clone(),
            code: snapshot.code.clone(),
            interval: snapshot.interval.as_str().into(),
            outputs: vec![ModelOutput {
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
    fn component_library_pages_packages_ten_at_a_time() {
        let root = std::env::temp_dir().join(format!(
            "adaq-component-page-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let state = LocalResearchState::open(&root).unwrap();
        let database = state.database.lock().unwrap();
        for index in 0..12 {
            let archive_sha256 = format!("{index:064x}");
            let path = root.join(format!("invalid-{index}.adaq"));
            fs::write(&path, b"invalid package").unwrap();
            database
                .execute(
                    "INSERT INTO component_content(archive_sha256, component_id, version, name, kind, wasm_sha256, archive_path) VALUES (?1, ?2, '1.0.0', ?3, 'factor', ?4, ?5)",
                    params![
                        archive_sha256,
                        format!("00000000-0000-4000-8000-{index:012}"),
                        format!("Package {index:02}"),
                        format!("{:064x}", index + 100),
                        path.to_string_lossy()
                    ],
                )
                .unwrap();
            database
                .execute(
                    "INSERT INTO component_access VALUES ('alice', ?1)",
                    [archive_sha256],
                )
                .unwrap();
        }
        drop(database);

        let first = state.list_components_page("alice", 1).unwrap();
        assert_eq!(first.total, 12);
        assert_eq!(first.items.len(), 10);
        assert!(first.items.iter().all(|item| !item.compatible));

        let second = state.list_components_page("alice", 2).unwrap();
        assert_eq!(second.total, 12);
        assert_eq!(second.items.len(), 2);
    }

    fn fixture(name: &str) -> (ComponentManifest, Vec<u8>) {
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join(name);
        let manifest =
            serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap()).unwrap();
        let wasm = fs::read(directory.join(format!(
            "target/wasm32-unknown-unknown/debug/m1_{name}_fixture.wasm"
        )))
        .unwrap();
        (manifest, wasm)
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

    fn legacy_package() -> Vec<u8> {
        let manifest = r#"{
            "componentId":"22222222-2222-4222-8222-222222222222",
            "version":"1.0.0",
            "name":"Legacy Strategy",
            "kind":"strategy",
            "abiVersion":"1.0.0",
            "inputNames":["trend.close-change"]
        }"#;
        let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default();
        archive.start_file("manifest.json", options).unwrap();
        archive.write_all(manifest.as_bytes()).unwrap();
        archive.start_file("component.wasm", options).unwrap();
        archive.write_all(b"\0asm\x0d\0\x01\0").unwrap();
        archive.finish().unwrap().into_inner()
    }

    #[test]
    fn incompatible_packages_do_not_block_factor_queries_or_deletion() {
        let root = std::env::temp_dir().join(format!(
            "adaq-legacy-component-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let state = LocalResearchState::open(&root).unwrap();
        let archive_hash = "a".repeat(64);
        let path = root.join("legacy.adaq");
        fs::write(&path, legacy_package()).unwrap();
        let database = state.database.lock().unwrap();
        database
            .execute(
                "INSERT INTO component_content(archive_sha256, component_id, version, name, kind, wasm_sha256, archive_path) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    archive_hash,
                    "22222222-2222-4222-8222-222222222222",
                    "1.0.0",
                    "Legacy Strategy",
                    "strategy",
                    "b".repeat(64),
                    path.to_string_lossy()
                ],
            )
            .unwrap();
        database
            .execute(
                "INSERT INTO component_access VALUES ('alice', ?1)",
                [&archive_hash],
            )
            .unwrap();
        drop(database);

        let components = state.list_components("alice").unwrap();
        assert_eq!(components.len(), 1);
        assert!(
            components[0]
                .compatibility_error
                .as_deref()
                .unwrap()
                .contains("inputNames")
        );
        let factor = state
            .import_component("alice", &public_example_package("factor-close-momentum-5"))
            .unwrap();
        let strategy = state
            .import_component("alice", &public_example_package("strategy-momentum-trend"))
            .unwrap();
        let matches = state
            .compatible_factors("alice", &strategy.archive_sha256)
            .unwrap();
        assert_eq!(matches["momentum"], [factor.archive_sha256]);
        state
            .delete_component("alice", &components[0].archive_sha256)
            .unwrap();
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn component_list_uses_imported_metadata_without_rereading_the_archive() {
        let root = std::env::temp_dir().join(format!(
            "adaq-replaced-component-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let state = LocalResearchState::open(&root).unwrap();
        let (factor, wasm) = fixture("factor");
        let imported = state
            .import_component("alice", &pack_component(factor, &wasm).unwrap())
            .unwrap();
        let (strategy, wasm) = fixture("strategy");
        fs::write(
            state
                .root
                .join("components")
                .join(format!("{}.adaq", imported.archive_sha256)),
            pack_component(strategy, &wasm).unwrap(),
        )
        .unwrap();

        let listed = state.list_components("alice").unwrap();
        assert!(listed[0].compatible);
        assert_eq!(listed[0].archive_sha256, imported.archive_sha256);
        assert!(
            state
                .package_for_user("alice", &imported.archive_sha256)
                .unwrap_err()
                .contains("does not match stored identity or hashes")
        );

        drop(state);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn component_library_is_user_scoped_and_identity_locked() {
        let root = std::env::temp_dir().join(format!(
            "adaq-m3-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let state = LocalResearchState::open(&root).unwrap();
        let (factor, wasm) = fixture("factor");
        let bytes = pack_component(factor.clone(), &wasm).unwrap();
        let factor_entry = state.import_component("alice", &bytes).unwrap();
        assert_eq!(
            factor_entry.manifest_schema_version,
            factor.manifest_schema_version.to_string()
        );
        assert_eq!(factor_entry.abi_version, factor.abi_version.to_string());
        assert_eq!(factor_entry.output_names, factor.output_names);
        assert_eq!(factor_entry.warmup_bars, factor.warmup_bars);
        assert!(factor_entry.compatible);
        assert!(factor_entry.locked_by_run_ids.is_empty());
        assert_eq!(state.list_components("alice").unwrap().len(), 1);
        assert!(state.list_components("bob").unwrap().is_empty());
        assert_eq!(
            state
                .delete_component("bob", &factor_entry.archive_sha256)
                .unwrap_err(),
            "Component Package is not available to this User"
        );

        let mut conflicting = factor;
        conflicting.name = "Conflicting Package".into();
        let bytes = pack_component(conflicting, &wasm).unwrap();
        assert!(state.import_component("alice", &bytes).is_err());

        let (strategy, wasm) = fixture("strategy");
        let bytes = pack_component(strategy, &wasm).unwrap();
        let strategy_entry = state.import_component("alice", &bytes).unwrap();
        assert!(!strategy_entry.feature_slots.is_empty());
        assert_eq!(state.list_components("alice").unwrap().len(), 2);

        let bar = |open_time_ms, close: i64| OhlcvBar {
            open_time_ms,
            open: close.into(),
            high: close.into(),
            low: close.into(),
            close: close.into(),
            base_volume: rust_decimal::Decimal::ONE,
            quote_volume: close.into(),
        };
        let snapshot = state
            .persist_snapshot_for_user(
                "alice",
                &adaq_data_core::BarSeries {
                    src: "okx".into(),
                    code: "BTC-USDT".into(),
                    interval: BarInterval::OneHour,
                    bars: vec![bar(0, 100), bar(3_600_000, 101), bar(7_200_000, 102)],
                    gaps: vec![],
                },
            )
            .unwrap();
        let request = || BacktestRunRequest {
            user_id: "alice".into(),
            snapshot_id: snapshot.snapshot_id.clone(),
            run_start_time_ms: None,
            run_end_time_ms: None,
            factor_instances: vec![],
            signal_instances: vec![],
            strategy_archive_sha256: strategy_entry.archive_sha256.clone(),
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
        let preview = state.backtests.preflight(&request()).unwrap();
        assert!(!preview.reuses_existing_run);
        assert!(state.backtests.get("alice", &preview.run_id).is_err());
        assert_eq!(
            preview.normalized_request.initial_quote_allocation,
            rust_decimal::Decimal::from(10_000),
        );
        assert!(preview.feature_plan.get("slots").is_some());
        let mut subset = request();
        subset.run_start_time_ms = Some(3_600_000);
        subset.run_end_time_ms = Some(snapshot.end_time_ms);
        let subset_preview = state.backtests.preflight(&subset).unwrap();
        assert_ne!(subset_preview.run_id, preview.run_id);
        subset.run_start_time_ms = Some(snapshot.start_time_ms - 1);
        assert!(
            state
                .backtests
                .preflight(&subset)
                .err()
                .unwrap()
                .contains("subset")
        );

        let first = state.backtests.run(request()).unwrap();
        let second = state.backtests.run(request()).unwrap();
        assert!(!first.plan_hash.is_empty());
        assert_eq!(
            serde_json::to_value(&first).unwrap(),
            serde_json::to_value(&second).unwrap()
        );
        assert_eq!(
            state
                .backtests
                .list(&crate::backtest::BacktestListRequest {
                    user_id: "alice".into(),
                    src: None,
                    code: None,
                    page: 1,
                })
                .unwrap()
                .total,
            1
        );
        let locked = state.list_components("alice").unwrap();
        let locked_strategy = locked
            .iter()
            .find(|component| component.archive_sha256 == strategy_entry.archive_sha256)
            .unwrap();
        assert_eq!(locked_strategy.locked_by_run_ids, [first.run_id.clone()]);
        assert_eq!(
            state
                .delete_component("alice", &strategy_entry.archive_sha256)
                .unwrap_err(),
            format!(
                "Component Package is locked by Backtest Run: {}",
                first.run_id
            )
        );
        state.backtests.delete("alice", &first.run_id).unwrap();
        state
            .delete_component("alice", &strategy_entry.archive_sha256)
            .unwrap();
        drop(state);
        fs::remove_dir_all(root).unwrap();
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
        let factor = state
            .import_component("alice", &public_example_package("factor-close-momentum-5"))
            .unwrap();
        let strategy = state
            .import_component("alice", &public_example_package("strategy-momentum-trend"))
            .unwrap();
        let strategy_package = state
            .package_for_user("alice", &strategy.archive_sha256)
            .unwrap();
        let factor_package = state
            .package_for_user("alice", &factor.archive_sha256)
            .unwrap();
        let factor_entry = state
            .list_components("alice")
            .unwrap()
            .into_iter()
            .find(|component| component.archive_sha256 == factor.archive_sha256)
            .unwrap();
        let mut mismatched_entry = factor_entry.clone();
        mismatched_entry.archive_sha256 = "c".repeat(64);
        let mut mismatched_package = factor_package.clone();
        mismatched_package.manifest.version = serde_json::from_str("\"0.2.0\"").unwrap();
        let mut incompatible_entry = factor_entry.clone();
        incompatible_entry.archive_sha256 = "d".repeat(64);
        incompatible_entry.compatible = false;
        let matches = compatible_factor_hashes(
            &strategy_package.manifest,
            &[
                (&factor_entry, factor_package),
                (&mismatched_entry, mismatched_package),
                (
                    &incompatible_entry,
                    state
                        .package_for_user("alice", &factor.archive_sha256)
                        .unwrap(),
                ),
            ],
        );
        assert_eq!(matches["momentum"], [factor.archive_sha256.clone()]);
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
                archive_sha256: factor.archive_sha256.clone(),
                parameters: HashMap::new(),
            }],
            signal_instances: vec![],
            strategy_archive_sha256: strategy.archive_sha256.clone(),
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
