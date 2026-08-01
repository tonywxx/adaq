use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use adaq_backtest_core::{
    ExecutionProfile, MarketDataSnapshot, SnapshotStore, SpotSimulator,
    TargetDecision as SimulationDecision,
};
use adaq_component_tooling::{
    ComponentDependency, ComponentKind, ComponentManifest, ComponentPackage,
    ComponentParameterValue, FactorInstancePlanInput, FeatureSlotDefinition, FeatureSlotSource,
    FrozenFeaturePlan, ModelArtifact, ModelOutput, ModelScope, ParameterDefinition, RunLimits,
    SignalPlanInput, StrategyArchitecture, component_parameters, native_engine_identity,
    strategy_architecture, validate_and_freeze_feature_plan_with_bindings_and_parameters,
    verify_package,
};
use adaq_data_core::{BarGap, BarInterval, HistoricalBarRange, OhlcvBar, OkxClient};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::Manager;

use crate::{
    m8::backtest_signal_datasets,
    run_engine::{
        FactorRunRequest, PositionMode, RunEngine, RunRequest, SignalRunRequest, SignalRunRow,
    },
};

const RUN_HISTORY_PAGE_SIZE: usize = 10;
const SNAPSHOT_PAGE_SIZE: usize = 10;
const COMPONENT_PAGE_SIZE: usize = 10;

pub struct M3State {
    pub(crate) root: PathBuf,
    snapshots: SnapshotStore,
    pub(crate) database: Mutex<Connection>,
    downloads: Mutex<HashMap<String, Arc<AtomicBool>>>,
    pub(crate) generation_attempts: Mutex<HashMap<String, Arc<AtomicBool>>>,
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

#[derive(Debug, Clone, Serialize)]
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

impl M3State {
    pub fn open(app_data: &Path) -> Result<Self, String> {
        let root = app_data.join("m3");
        fs::create_dir_all(root.join("components")).map_err(string)?;
        let snapshots = SnapshotStore::new(root.join("market-data")).map_err(string)?;
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
                UNIQUE(component_id, version)
             );
             CREATE TABLE IF NOT EXISTS component_access (
                user_id TEXT NOT NULL,
                archive_sha256 TEXT NOT NULL,
                PRIMARY KEY(user_id, archive_sha256),
                FOREIGN KEY(archive_sha256) REFERENCES component_content(archive_sha256)
             );
             CREATE TABLE IF NOT EXISTS market_data_snapshots (
                snapshot_id TEXT PRIMARY KEY,
                src TEXT NOT NULL,
                code TEXT NOT NULL,
                interval TEXT NOT NULL,
                start_time_ms INTEGER NOT NULL,
                end_time_ms INTEGER NOT NULL,
                bar_count INTEGER NOT NULL,
                metadata_json TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS market_data_snapshot_access (
                user_id TEXT NOT NULL,
                snapshot_id TEXT NOT NULL,
                PRIMARY KEY(user_id, snapshot_id),
                FOREIGN KEY(snapshot_id) REFERENCES market_data_snapshots(snapshot_id)
             );
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
             );
             CREATE TABLE IF NOT EXISTS validation_protocols (
                protocol_id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL,
                protocol_json TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS validation_reports (
                report_id TEXT PRIMARY KEY,
                protocol_id TEXT NOT NULL,
                user_id TEXT NOT NULL,
                report_json TEXT NOT NULL,
                FOREIGN KEY(protocol_id) REFERENCES validation_protocols(protocol_id)
             );
             CREATE TABLE IF NOT EXISTS dataset_generation_attempts (
                attempt_id TEXT PRIMARY KEY,
                request_hash TEXT NOT NULL,
                user_id TEXT NOT NULL,
                dataset_id TEXT,
                status TEXT NOT NULL,
                request_json TEXT NOT NULL,
                diagnostic_json TEXT,
                progress_completed INTEGER NOT NULL DEFAULT 0,
                progress_total INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
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
        for column in ["progress_completed", "progress_total"] {
            let exists = database
                .prepare("SELECT 1 FROM pragma_table_info('dataset_generation_attempts') WHERE name = ?1")
                .map_err(string)?
                .exists([column])
                .map_err(string)?;
            if !exists {
                database
                    .execute(
                        &format!(
                            "ALTER TABLE dataset_generation_attempts ADD COLUMN {column} INTEGER NOT NULL DEFAULT 0"
                        ),
                        [],
                    )
                    .map_err(string)?;
            }
        }
        database.execute(
            "UPDATE dataset_generation_attempts SET status = 'failed', diagnostic_json = 'generation-interrupted: application stopped before completion' WHERE status IN ('pending', 'running')",
            [],
        ).map_err(string)?;
        Ok(Self {
            root,
            snapshots,
            database: Mutex::new(database),
            downloads: Mutex::new(HashMap::new()),
            generation_attempts: Mutex::new(HashMap::new()),
        })
    }

    pub fn local_data_summary(&self, user_id: &str) -> Result<LocalDataSummary, String> {
        validate_user(user_id)?;
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
        let snapshot_json = strings(
            &database,
            "SELECT s.metadata_json FROM market_data_snapshots s
			 JOIN market_data_snapshot_access a USING(snapshot_id) WHERE a.user_id = ?1",
            user_id,
        )?;
        let database_path = self.root.parent().unwrap_or(&self.root).join("adaq.db");
        let data_directory = database_path
            .parent()
            .unwrap_or(&self.root)
            .to_string_lossy()
            .into_owned();
        let market_paths = snapshot_json
            .iter()
            .filter_map(|json| serde_json::from_str::<MarketDataSnapshot>(json).ok())
            .map(|snapshot| snapshot.parquet_path)
            .collect::<Vec<_>>();

        Ok(LocalDataSummary {
            data_directory,
            database_bytes: file_bytes(&database_path),
            component_bytes: component_paths.iter().map(file_bytes).sum(),
            market_data_bytes: market_paths.iter().map(file_bytes).sum(),
            watchlist_count: count("SELECT COUNT(*) FROM watchlist_items WHERE user_id = ?1")?,
            component_count: count("SELECT COUNT(*) FROM component_access WHERE user_id = ?1")?,
            snapshot_count: count(
                "SELECT COUNT(*) FROM market_data_snapshot_access WHERE user_id = ?1",
            )?,
            run_count: count("SELECT COUNT(*) FROM backtest_runs WHERE user_id = ?1")?,
            protocol_count: count("SELECT COUNT(*) FROM validation_protocols WHERE user_id = ?1")?,
            report_count: count("SELECT COUNT(*) FROM validation_reports WHERE user_id = ?1")?,
            generation_attempt_count: count(
                "SELECT COUNT(*) FROM dataset_generation_attempts WHERE user_id = ?1",
            )?,
            model_artifact_count: count(
                "SELECT COUNT(*) FROM component_content c JOIN component_access a USING(archive_sha256) WHERE a.user_id = ?1 AND c.kind = 'model'",
            )?,
            signal_dataset_count: count(
                "SELECT COUNT(*) FROM signal_dataset_access WHERE user_id = ?1",
            )?,
            component_blocking_run_count: count(
                "SELECT (SELECT COUNT(DISTINCT r.run_id) FROM backtest_runs r
				 JOIN backtest_run_components rc USING(run_id)
				 JOIN component_access a USING(archive_sha256)
				 WHERE r.user_id = ?1 AND a.user_id = ?1)
                 + (SELECT COUNT(*) FROM signal_dataset_access WHERE user_id = ?1)",
            )?,
            market_data_blocking_record_count: count(
                "SELECT (SELECT COUNT(*) FROM backtest_runs WHERE user_id = ?1)
				 + (SELECT COUNT(*) FROM validation_reports WHERE user_id = ?1)
                 + (SELECT COUNT(*) FROM signal_dataset_access WHERE user_id = ?1)",
            )?,
        })
    }

    pub fn reset_local_data(&self, user_id: &str, kind: LocalDataResetKind) -> Result<(), String> {
        validate_user(user_id)?;
        let mut database = self.database.lock().map_err(string)?;
        if matches!(kind, LocalDataResetKind::All) {
            let attempt_ids = strings(
                &database,
                "SELECT attempt_id FROM dataset_generation_attempts WHERE user_id = ?1 AND status IN ('pending', 'running')",
                user_id,
            )?;
            let attempts = self.generation_attempts.lock().map_err(string)?;
            for attempt_id in attempt_ids {
                if let Some(cancelled) = attempts.get(&attempt_id) {
                    cancelled.store(true, Ordering::Relaxed);
                }
            }
        }
        match kind {
            LocalDataResetKind::Watchlist => reset_watchlist(&mut database, user_id),
            LocalDataResetKind::Components => reset_components(&mut database, user_id, &self.root),
            LocalDataResetKind::MarketData => reset_market_data(&mut database, user_id, &self.root),
            LocalDataResetKind::All => reset_all(&mut database, user_id, &self.root),
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
        transaction
            .execute(
                "INSERT OR IGNORE INTO component_content
             (archive_sha256, component_id, version, name, kind, wasm_sha256, archive_path)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    package.archive_sha256,
                    component_id,
                    version,
                    package.manifest.name,
                    kind,
                    package.manifest.wasm_sha256,
                    path.to_string_lossy()
                ],
            )
            .map_err(string)?;
        transaction
            .execute(
                "INSERT OR IGNORE INTO component_access(user_id, archive_sha256) VALUES (?1, ?2)",
                params![user_id, package.archive_sha256],
            )
            .map_err(string)?;
        transaction.commit().map_err(string)?;
        Ok(LibraryComponent {
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
        })
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
        let mut locked_by_hash = HashMap::<String, Vec<String>>::new();
        {
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
        }
        let mut statement = database
            .prepare(
                "SELECT c.component_id, c.version, c.name, c.kind, c.archive_sha256, c.wasm_sha256, c.archive_path
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
                ))
            })
            .map_err(string)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(string)?
            .into_iter()
            .map(
                |(component_id, version, name, kind, archive_sha256, wasm_sha256, path)| {
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
        let locked_by_run_ids = database
            .prepare(
                "SELECT rc.run_id FROM backtest_run_components rc
                 JOIN backtest_runs r USING(run_id)
                 WHERE r.user_id = ?1 AND rc.archive_sha256 = ?2
                 ORDER BY r.created_at, rc.run_id",
            )
            .map_err(string)?
            .query_map(params![user_id, hash], |row| row.get::<_, String>(0))
            .map_err(string)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(string)?;
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

    pub fn persist_snapshot(
        &self,
        series: &adaq_data_core::BarSeries,
    ) -> Result<MarketDataSnapshot, String> {
        let snapshot = self.snapshots.persist(series).map_err(string)?;
        let metadata = serde_json::to_string(&snapshot).map_err(string)?;
        self.database.lock().map_err(string)?.execute(
            "INSERT OR IGNORE INTO market_data_snapshots
             (snapshot_id, src, code, interval, start_time_ms, end_time_ms, bar_count, metadata_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![snapshot.snapshot_id, snapshot.src, snapshot.code,
                serde_json::to_string(&snapshot.interval).map_err(string)?, snapshot.start_time_ms,
                snapshot.end_time_ms, snapshot.bar_count as i64, metadata],
        ).map_err(string)?;
        Ok(snapshot)
    }

    fn persist_snapshot_for_user(
        &self,
        user_id: &str,
        series: &adaq_data_core::BarSeries,
    ) -> Result<MarketDataSnapshot, String> {
        validate_user(user_id)?;
        let snapshot = self.persist_snapshot(series)?;
        self.database.lock().map_err(string)?.execute(
            "INSERT OR IGNORE INTO market_data_snapshot_access (user_id, snapshot_id) VALUES (?1, ?2)",
            params![user_id, snapshot.snapshot_id],
        ).map_err(string)?;
        Ok(snapshot)
    }

    fn list_snapshots(&self, request: &SnapshotListRequest) -> Result<SnapshotPage, String> {
        validate_user(&request.user_id)?;
        if request.src.trim().is_empty() || request.code.trim().is_empty() || request.page == 0 {
            return Err("Snapshot coverage request is invalid".into());
        }
        let interval = serde_json::to_string(&request.interval).map_err(string)?;
        let database = self.database.lock().map_err(string)?;
        let total = database
            .query_row(
                "SELECT COUNT(*) FROM market_data_snapshots s
                 JOIN market_data_snapshot_access a USING(snapshot_id)
                 WHERE a.user_id = ?1 AND s.src = ?2 AND s.code = ?3 AND s.interval = ?4",
                params![request.user_id, request.src, request.code, interval],
                |row| row.get::<_, i64>(0),
            )
            .map_err(string)?
            .try_into()
            .map_err(|_| "Snapshot count is invalid")?;
        let offset = request
            .page
            .checked_sub(1)
            .and_then(|value| value.checked_mul(SNAPSHOT_PAGE_SIZE))
            .ok_or_else(|| "Snapshot page is too large".to_owned())?;
        let mut statement = database
            .prepare(
                "SELECT s.metadata_json FROM market_data_snapshots s
             JOIN market_data_snapshot_access a USING(snapshot_id)
             WHERE a.user_id = ?1 AND s.src = ?2 AND s.code = ?3 AND s.interval = ?4
             ORDER BY s.start_time_ms, s.snapshot_id LIMIT ?5 OFFSET ?6",
            )
            .map_err(string)?;
        let items = statement
            .query_map(
                params![
                    request.user_id,
                    request.src,
                    request.code,
                    interval,
                    SNAPSHOT_PAGE_SIZE as i64,
                    offset as i64
                ],
                |row| {
                    serde_json::from_str(&row.get::<_, String>(0)?).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })
                },
            )
            .map_err(string)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(string)?;
        Ok(SnapshotPage {
            items,
            total,
            page: request.page,
            page_size: SNAPSHOT_PAGE_SIZE,
        })
    }

    fn list_readable_snapshots(&self, user_id: &str) -> Result<Vec<MarketDataSnapshot>, String> {
        validate_user(user_id)?;
        let database = self.database.lock().map_err(string)?;
        let mut statement = database
            .prepare(
                "SELECT s.metadata_json FROM market_data_snapshots s
                 JOIN market_data_snapshot_access a USING(snapshot_id)
                 WHERE a.user_id = ?1
                 ORDER BY s.code, s.interval, s.start_time_ms, s.snapshot_id",
            )
            .map_err(string)?;
        statement
            .query_map([user_id], |row| {
                serde_json::from_str(&row.get::<_, String>(0)?).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })
            })
            .map_err(string)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(string)
    }

    pub(crate) fn package_for_user(
        &self,
        user_id: &str,
        hash: &str,
    ) -> Result<ComponentPackage, String> {
        validate_user(user_id)?;
        let database = self.database.lock().map_err(string)?;
        let path: String = database
            .query_row(
                "SELECT c.archive_path FROM component_content c
                 JOIN component_access a USING(archive_sha256)
                 WHERE a.user_id = ?1 AND c.archive_sha256 = ?2",
                params![user_id, hash],
                |row| row.get(0),
            )
            .map_err(|_| "Component Package is not available to this User".to_owned())?;
        ComponentPackage::read(&fs::read(path).map_err(string)?).map_err(string)
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
        validate_user(user_id)?;
        let database = self.database.lock().map_err(string)?;
        let json: String = database
            .query_row(
                "SELECT s.metadata_json FROM market_data_snapshots s
             JOIN market_data_snapshot_access a USING(snapshot_id)
             WHERE a.user_id = ?1 AND s.snapshot_id = ?2",
                params![user_id, snapshot_id],
                |row| row.get(0),
            )
            .map_err(|_| "Market Data Snapshot is not available to this User".to_owned())?;
        drop(database);
        let snapshot: MarketDataSnapshot = serde_json::from_str(&json).map_err(string)?;
        let bars = self.snapshots.read(&snapshot).map_err(string)?;
        Ok((snapshot, bars))
    }

    pub(crate) fn runtime_component(&self, package: &ComponentPackage) -> Result<PathBuf, String> {
        let directory = self.root.join("runtime");
        fs::create_dir_all(&directory).map_err(string)?;
        let path = directory.join(format!("{}.wasm", package.manifest.wasm_sha256));
        if !path.is_file() {
            fs::write(&path, &package.wasm).map_err(string)?;
        }
        Ok(path)
    }

    fn save_run(&self, user_id: &str, run_id: &str, result: &BacktestRun) -> Result<(), String> {
        let json = serde_json::to_string(result).map_err(string)?;
        let mut database = self.database.lock().map_err(string)?;
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

    fn load_run(&self, user_id: &str, run_id: &str) -> Result<BacktestRun, String> {
        validate_user(user_id)?;
        let json: String = self
            .database
            .lock()
            .map_err(string)?
            .query_row(
                "SELECT result_json FROM backtest_runs WHERE user_id = ?1 AND run_id = ?2",
                params![user_id, run_id],
                |row| row.get(0),
            )
            .map_err(|_| "Backtest Run was not found".to_owned())?;
        let run: BacktestRun = serde_json::from_str(&json).map_err(string)?;
        if let Some(provenance) = &run.provenance {
            validate_provenance(provenance)?;
            if provenance.component_lock != run.component_lock {
                return Err("Backtest Run provenance does not match its Component Lock".into());
            }
        }
        Ok(run)
    }

    #[cfg(test)]
    fn list_runs(&self, user_id: &str) -> Result<Vec<BacktestRunSummary>, String> {
        validate_user(user_id)?;
        let database = self.database.lock().map_err(string)?;
        let mut statement = database.prepare(
            "SELECT run_id, created_at, result_json FROM backtest_runs WHERE user_id = ?1 ORDER BY created_at DESC",
        ).map_err(string)?;
        statement
            .query_map([user_id], |row| {
                let run: BacktestRun =
                    serde_json::from_str(&row.get::<_, String>(2)?).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            2,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })?;
                Ok(BacktestRunSummary {
                    run_id: row.get(0)?,
                    created_at: row.get(1)?,
                    snapshot_id: run.snapshot.snapshot_id,
                    code: run.snapshot.code,
                    interval: run.snapshot.interval,
                    bar_count: run.snapshot.bar_count,
                    total_return: run.result.metrics.total_return,
                })
            })
            .map_err(string)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(string)
    }

    fn list_runs_page(
        &self,
        user_id: &str,
        src: Option<&str>,
        code: Option<&str>,
        page: usize,
    ) -> Result<BacktestRunPage, String> {
        validate_user(user_id)?;
        let instrument_valid = match (src, code) {
            (None, None) => true,
            (Some(src), Some(code)) => !src.trim().is_empty() && !code.trim().is_empty(),
            _ => false,
        };
        if page == 0 || !instrument_valid {
            return Err("Backtest Run history request is invalid".into());
        }
        let database = self.database.lock().map_err(string)?;
        let filter = "user_id = ?1
            AND (?2 IS NULL OR (
                json_extract(result_json, '$.snapshot.src') = ?2
                AND json_extract(result_json, '$.snapshot.code') = ?3
            ))";
        let total = database
            .query_row(
                &format!("SELECT COUNT(*) FROM backtest_runs WHERE {filter}"),
                params![user_id, src, code],
                |row| row.get::<_, i64>(0),
            )
            .map_err(string)?
            .try_into()
            .map_err(|_| "Backtest Run history count is invalid")?;
        let offset = page
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
                    user_id,
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
            page,
            page_size: RUN_HISTORY_PAGE_SIZE,
        })
    }

    fn delete_run(&self, user_id: &str, run_id: &str) -> Result<(), String> {
        validate_user(user_id)?;
        let database = self.database.lock().map_err(string)?;
        let mut statement = database
            .prepare("SELECT report_json FROM validation_reports WHERE user_id = ?1")
            .map_err(string)?;
        let reports = statement
            .query_map([user_id], |row| {
                serde_json::from_str::<ValidationReport>(&row.get::<_, String>(0)?).map_err(
                    |error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    },
                )
            })
            .map_err(string)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(string)?;
        if reports
            .iter()
            .any(|report| report_references_run(report, run_id))
        {
            return Err("Backtest Run is referenced by an immutable Validation Report".into());
        }
        drop(statement);
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

    fn save_protocol(&self, protocol: &ValidationProtocol) -> Result<(), String> {
        self.database.lock().map_err(string)?.execute(
            "INSERT OR IGNORE INTO validation_protocols(protocol_id, user_id, protocol_json) VALUES (?1, ?2, ?3)",
            params![protocol.protocol_id, protocol.user_id, serde_json::to_string(protocol).map_err(string)?],
        ).map_err(string)?;
        Ok(())
    }

    fn load_protocol(
        &self,
        user_id: &str,
        protocol_id: &str,
    ) -> Result<ValidationProtocol, String> {
        validate_user(user_id)?;
        self.database.lock().map_err(string)?.query_row(
            "SELECT protocol_json FROM validation_protocols WHERE user_id = ?1 AND protocol_id = ?2",
            params![user_id, protocol_id],
            |row| serde_json::from_str(&row.get::<_, String>(0)?).map_err(|error| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))),
        ).map_err(|_| "Validation Protocol was not found".to_owned())
    }

    fn list_protocols(&self, user_id: &str) -> Result<Vec<ValidationProtocol>, String> {
        validate_user(user_id)?;
        let database = self.database.lock().map_err(string)?;
        let mut statement = database.prepare("SELECT protocol_json FROM validation_protocols WHERE user_id = ?1 ORDER BY rowid DESC").map_err(string)?;
        statement
            .query_map([user_id], |row| {
                serde_json::from_str(&row.get::<_, String>(0)?).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })
            })
            .map_err(string)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(string)
    }

    fn save_report(&self, report: &ValidationReport) -> Result<(), String> {
        self.database.lock().map_err(string)?.execute(
            "INSERT OR IGNORE INTO validation_reports(report_id, protocol_id, user_id, report_json) VALUES (?1, ?2, ?3, ?4)",
            params![report.report_id, report.protocol_id, report.user_id, serde_json::to_string(report).map_err(string)?],
        ).map_err(string)?;
        Ok(())
    }

    fn list_reports(&self, user_id: &str) -> Result<Vec<ValidationReport>, String> {
        validate_user(user_id)?;
        let database = self.database.lock().map_err(string)?;
        let mut statement = database
            .prepare(
                "SELECT report_json FROM validation_reports WHERE user_id = ?1 ORDER BY rowid DESC",
            )
            .map_err(string)?;
        statement
            .query_map([user_id], |row| {
                serde_json::from_str(&row.get::<_, String>(0)?).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })
            })
            .map_err(string)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(string)
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
pub struct SnapshotCreateRequest {
    pub user_id: String,
    pub src: String,
    pub code: String,
    pub interval: BarInterval,
    pub start_time_ms: i64,
    pub end_time_ms: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotDownloadRequest {
    pub user_id: String,
    pub task_id: String,
    pub src: String,
    pub code: String,
    pub interval: BarInterval,
    pub start_time_ms: i64,
    pub end_time_ms: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotListRequest {
    pub user_id: String,
    pub src: String,
    pub code: String,
    pub interval: BarInterval,
    pub page: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotPage {
    pub items: Vec<MarketDataSnapshot>,
    pub total: usize,
    pub page: usize,
    pub page_size: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadableSnapshotListRequest {
    pub user_id: String,
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

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase", tag = "event", content = "data")]
pub enum SnapshotDownloadEvent {
    Progress {
        downloaded_bars: usize,
        oldest_time_ms: i64,
    },
    Completed {
        snapshot_id: String,
        bar_count: usize,
    },
    Cancelled,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRequest {
    pub task_id: String,
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestDependencyRequest {
    pub user_id: String,
    pub strategy_archive_sha256: String,
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

struct PreparedBacktest {
    strategy: ComponentPackage,
    strategy_parameters: Vec<ComponentParameterValue>,
    factor_packages: Vec<ComponentPackage>,
    signals: Vec<PreparedSignal>,
    plan: FrozenFeaturePlan,
    provenance: BacktestRunProvenance,
    component_lock: Vec<ComponentLockEntry>,
    run_id: String,
    snapshot: MarketDataSnapshot,
    bars: Vec<OhlcvBar>,
    gaps: Vec<BarGap>,
}

struct PreparedSignal {
    slot: String,
    dataset_id: String,
    signal_name: String,
    rows: Vec<SignalRunRow>,
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
    pub decisions: Vec<SimulationDecision>,
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
    pub decisions: Vec<SimulationDecision>,
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

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationProtocolCreateRequest {
    pub user_id: String,
    pub run: BacktestRunRequest,
    pub windows: Vec<ValidationWindowRequest>,
    #[serde(default)]
    pub walk_forward: Option<WalkForwardValidationRequest>,
    #[serde(default)]
    pub cross_market: Option<CrossMarketValidationRequest>,
    pub method_version: String,
    pub aggregation_rule_version: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CrossMarketValidationRequest {
    pub contexts: Vec<CrossMarketValidationContextRequest>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CrossMarketValidationContextRequest {
    pub snapshot_id: String,
    #[serde(default)]
    pub run_override: Option<BacktestRunRequest>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationWindowRequest {
    pub snapshot_id: String,
    pub sample_out_start_time_ms: i64,
    #[serde(default)]
    pub sample_out_end_time_ms: Option<i64>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WalkForwardValidationRequest {
    pub snapshot_id: String,
    pub window_size_bars: usize,
    pub step_size_bars: usize,
    pub minimum_history_bars: usize,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationProtocol {
    pub protocol_id: String,
    pub user_id: String,
    pub run: BacktestRunRequest,
    pub windows: Vec<ValidationWindowRequest>,
    #[serde(default)]
    pub walk_forward: Option<WalkForwardValidationRequest>,
    #[serde(default)]
    pub cross_market: Option<CrossMarketValidationRequest>,
    pub method_version: String,
    pub aggregation_rule_version: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationProtocolIdRequest {
    pub user_id: String,
    pub protocol_id: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationWindowReport {
    pub sample_out_start_time_ms: i64,
    #[serde(default)]
    pub sample_out_end_time_ms: Option<i64>,
    pub sample_in_snapshot_id: String,
    pub sample_out_snapshot_id: String,
    pub sample_in_run_id: Option<String>,
    pub sample_out_run_id: Option<String>,
    pub sample_in_metrics: Option<adaq_backtest_core::BacktestMetrics>,
    pub sample_out_metrics: Option<adaq_backtest_core::BacktestMetrics>,
    pub sample_in_pauses: Vec<RunPauseRecord>,
    pub sample_out_pauses: Vec<RunPauseRecord>,
    pub failure: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationAggregate {
    pub completed_windows: usize,
    pub failed_windows: usize,
    #[serde(with = "rust_decimal::serde::str")]
    pub average_sample_in_return: rust_decimal::Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub average_sample_out_return: rust_decimal::Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub worst_sample_out_drawdown: rust_decimal::Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub average_sample_out_sharpe: rust_decimal::Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub total_fees: rust_decimal::Decimal,
    pub total_trades: usize,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationReport {
    pub report_id: String,
    pub protocol_id: String,
    pub user_id: String,
    pub method_version: String,
    pub aggregation_rule_version: String,
    #[serde(default)]
    pub walk_forward: Option<WalkForwardValidationRequest>,
    #[serde(default)]
    pub cross_market: Vec<CrossMarketValidationReport>,
    #[serde(default)]
    pub recommended_contexts: Vec<RecommendedContext>,
    #[serde(default)]
    pub cross_market_evidence: Option<CrossMarketEvidence>,
    pub windows: Vec<ValidationWindowReport>,
    pub aggregate: ValidationAggregate,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CrossMarketValidationReport {
    pub snapshot: MarketDataSnapshot,
    pub run: BacktestRunRequest,
    pub run_id: Option<String>,
    pub metrics: Option<adaq_backtest_core::BacktestMetrics>,
    pub pauses: Vec<RunPauseRecord>,
    pub failure: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CrossMarketEvidence {
    pub completed_markets: usize,
    #[serde(with = "rust_decimal::serde::str")]
    pub total_return_spread: rust_decimal::Decimal,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecommendedContext {
    pub supporting_report_id: String,
    pub snapshot: MarketDataSnapshot,
    pub run: BacktestRunRequest,
}

#[tauri::command]
pub fn component_import(
    request: ComponentImportRequest,
    state: tauri::State<'_, M3State>,
) -> Result<LibraryComponent, String> {
    state.import_component(&request.user_id, &request.bytes)
}

#[tauri::command]
pub async fn component_list(
    request: ComponentUserRequest,
    app: tauri::AppHandle,
) -> Result<Vec<LibraryComponent>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<M3State>().list_components(&request.user_id)
    })
    .await
    .map_err(string)?
}

#[tauri::command]
pub async fn component_page(
    request: ComponentPageRequest,
    state: tauri::State<'_, M3State>,
) -> Result<ComponentPage, String> {
    state.list_components_page(&request.user_id, request.page)
}

#[tauri::command]
pub fn backtest_compatible_factors(
    request: BacktestDependencyRequest,
    state: tauri::State<'_, M3State>,
) -> Result<BTreeMap<String, Vec<String>>, String> {
    state.compatible_factors(&request.user_id, &request.strategy_archive_sha256)
}

#[tauri::command]
pub fn backtest_compatible_signals(
    request: BacktestSignalCompatibilityRequest,
    state: tauri::State<'_, M3State>,
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
    state: tauri::State<'_, M3State>,
) -> Result<(), String> {
    state.delete_component(&request.user_id, &request.archive_sha256)
}

#[tauri::command]
pub async fn snapshot_create(
    request: SnapshotCreateRequest,
    client: tauri::State<'_, OkxClient>,
    state: tauri::State<'_, M3State>,
) -> Result<MarketDataSnapshot, String> {
    validate_snapshot_request(
        &request.user_id,
        &request.src,
        &request.code,
        request.start_time_ms,
        request.end_time_ms,
    )?;
    if request.src != "okx" {
        return Err("M3 supports OKX Spot only".into());
    }
    let series = client
        .get_bar_series_range(
            &request.code,
            request.interval,
            HistoricalBarRange {
                start_time_ms: request.start_time_ms,
                end_time_ms: request.end_time_ms,
            },
        )
        .await
        .map_err(string)?;
    state.persist_snapshot_for_user(&request.user_id, &series)
}

#[tauri::command]
pub async fn snapshot_download(
    request: SnapshotDownloadRequest,
    on_event: tauri::ipc::Channel<SnapshotDownloadEvent>,
    client: tauri::State<'_, OkxClient>,
    state: tauri::State<'_, M3State>,
) -> Result<MarketDataSnapshot, String> {
    validate_snapshot_request(
        &request.user_id,
        &request.src,
        &request.code,
        request.start_time_ms,
        request.end_time_ms,
    )?;
    if request.src != "okx" || request.task_id.trim().is_empty() {
        return Err("Snapshot download request is invalid".into());
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    {
        let mut downloads = state.downloads.lock().map_err(string)?;
        if downloads.contains_key(&request.task_id) {
            return Err("Snapshot download is already in progress".into());
        }
        downloads.insert(request.task_id.clone(), cancelled.clone());
    }
    let result = client
        .get_bar_series_range_with_progress(
            &request.code,
            request.interval,
            HistoricalBarRange {
                start_time_ms: request.start_time_ms,
                end_time_ms: request.end_time_ms,
            },
            |downloaded_bars, oldest_time_ms| {
                let active = !cancelled.load(Ordering::Relaxed);
                if active {
                    let _ = on_event.send(SnapshotDownloadEvent::Progress {
                        downloaded_bars,
                        oldest_time_ms,
                    });
                }
                active
            },
        )
        .await;
    state
        .downloads
        .lock()
        .map_err(string)?
        .remove(&request.task_id);
    match result {
        Ok(series) => {
            let snapshot = state.persist_snapshot_for_user(&request.user_id, &series)?;
            let _ = on_event.send(SnapshotDownloadEvent::Completed {
                snapshot_id: snapshot.snapshot_id.clone(),
                bar_count: snapshot.bar_count,
            });
            Ok(snapshot)
        }
        Err(error) if error.code == "cancelled" => {
            let _ = on_event.send(SnapshotDownloadEvent::Cancelled);
            Err(error.to_string())
        }
        Err(error) => Err(error.to_string()),
    }
}

#[tauri::command]
pub async fn snapshot_list(
    request: SnapshotListRequest,
    state: tauri::State<'_, M3State>,
) -> Result<SnapshotPage, String> {
    state.list_snapshots(&request)
}

#[tauri::command]
pub async fn snapshot_list_readable(
    request: ReadableSnapshotListRequest,
    app: tauri::AppHandle,
) -> Result<Vec<MarketDataSnapshot>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<M3State>()
            .list_readable_snapshots(&request.user_id)
    })
    .await
    .map_err(string)?
}

#[tauri::command]
pub fn snapshot_cancel(
    request: TaskRequest,
    state: tauri::State<'_, M3State>,
) -> Result<(), String> {
    if let Some(cancelled) = state
        .downloads
        .lock()
        .map_err(string)?
        .get(&request.task_id)
    {
        cancelled.store(true, Ordering::Relaxed);
    }
    Ok(())
}

#[tauri::command]
pub fn backtest_run(
    request: BacktestRunRequest,
    state: tauri::State<'_, M3State>,
) -> Result<BacktestRunView, String> {
    execute_backtest(request, &state)
}

#[tauri::command]
pub fn backtest_preflight(
    request: BacktestRunRequest,
    state: tauri::State<'_, M3State>,
) -> Result<BacktestPreflight, String> {
    let prepared = prepare_backtest(&request, &state)?;
    Ok(BacktestPreflight {
        run_id: prepared.run_id.clone(),
        reuses_existing_run: state.load_run(&request.user_id, &prepared.run_id).is_ok(),
        snapshot: prepared.snapshot,
        normalized_request: prepared.provenance.normalized_request,
        feature_plan: serde_json::from_str(&prepared.provenance.feature_plan_json)
            .map_err(string)?,
        component_lock: prepared.component_lock,
        dataset_lock: prepared.provenance.dataset_lock,
        architecture: prepared.provenance.architecture,
    })
}

fn execute_backtest(
    request: BacktestRunRequest,
    state: &M3State,
) -> Result<BacktestRunView, String> {
    let prepared = prepare_backtest(&request, state)?;
    if let Ok(existing) = state.load_run(&request.user_id, &prepared.run_id) {
        return Ok(run_view(&existing, i64::MIN, i64::MAX, 2_000));
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
    let strategy_path = state.runtime_component(&strategy)?;
    let factor_paths = factor_packages
        .iter()
        .map(|package| state.runtime_component(package))
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
                    crate::run_engine::RunPauseReason::Warmup => "warmup".into(),
                    crate::run_engine::RunPauseReason::MissingInput { slot, source } => {
                        format!("missing-input:{slot}:{source}")
                    }
                },
            })
            .collect(),
        result,
        component_lock,
        provenance: Some(provenance),
    };
    state.save_run(&request.user_id, &run_id, &run)?;
    Ok(run_view(&run, i64::MIN, i64::MAX, 2_000))
}

fn prepare_backtest(
    request: &BacktestRunRequest,
    state: &M3State,
) -> Result<PreparedBacktest, String> {
    SpotSimulator::validate_execution_inputs(
        request.initial_quote_allocation,
        &request.execution_profile,
    )
    .map_err(string)?;
    let strategy = state.package_for_user(&request.user_id, &request.strategy_archive_sha256)?;
    if !matches!(strategy.manifest.kind, ComponentKind::Strategy) {
        return Err("Backtest requires a Strategy Component".into());
    }
    let strategy_parameters =
        component_parameters(&strategy.manifest, Some(&request.strategy_parameters))?;
    let frozen_strategy_parameters =
        normalized_parameters(&strategy.manifest, &strategy_parameters);
    let (snapshot, mut bars) = state.snapshot_for_user(&request.user_id, &request.snapshot_id)?;
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
            let package = state.package_for_user(&request.user_id, &factor.archive_sha256)?;
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
    let datasets =
        backtest_signal_datasets(state, &request.user_id, true, Some(&selected_dataset_ids))?;
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
        normalized_request: NormalizedBacktestRunRequest {
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
        indicator_engine_build_identity: IndicatorEngineBuildIdentity {
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

#[tauri::command]
pub async fn backtest_list(
    request: BacktestListRequest,
    state: tauri::State<'_, M3State>,
) -> Result<BacktestRunPage, String> {
    state.list_runs_page(
        &request.user_id,
        request.src.as_deref(),
        request.code.as_deref(),
        request.page,
    )
}

#[tauri::command]
pub fn backtest_get(
    request: BacktestRunIdRequest,
    state: tauri::State<'_, M3State>,
) -> Result<BacktestRunView, String> {
    state
        .load_run(&request.user_id, &request.run_id)
        .map(|run| run_view(&run, i64::MIN, i64::MAX, 2_000))
}

#[tauri::command]
pub fn backtest_chart_data(
    request: BacktestChartRequest,
    state: tauri::State<'_, M3State>,
) -> Result<BacktestRunView, String> {
    if request.start_time_ms >= request.end_time_ms || !(100..=10_000).contains(&request.max_points)
    {
        return Err("Backtest Chart range is invalid".into());
    }
    state
        .load_run(&request.user_id, &request.run_id)
        .map(|run| {
            run_view(
                &run,
                request.start_time_ms,
                request.end_time_ms,
                request.max_points,
            )
        })
}

#[tauri::command]
pub fn backtest_execution_data(
    request: BacktestExecutionRequest,
    state: tauri::State<'_, M3State>,
) -> Result<BacktestExecutionPage, String> {
    if !(1..=1_000).contains(&request.limit) {
        return Err("Backtest execution page is invalid".into());
    }
    let run = state.load_run(&request.user_id, &request.run_id)?;
    Ok(execution_page(&run.result, request.offset, request.limit))
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

#[tauri::command]
pub fn backtest_delete(
    request: BacktestRunIdRequest,
    state: tauri::State<'_, M3State>,
) -> Result<(), String> {
    state.delete_run(&request.user_id, &request.run_id)
}

#[tauri::command]
pub fn local_data_summary(
    request: LocalDataRequest,
    state: tauri::State<'_, M3State>,
) -> Result<LocalDataSummary, String> {
    state.local_data_summary(&request.user_id)
}

#[tauri::command]
pub fn local_data_reset(
    request: LocalDataResetRequest,
    state: tauri::State<'_, M3State>,
) -> Result<(), String> {
    state.reset_local_data(&request.user_id, request.kind)
}

#[tauri::command]
pub fn validation_protocol_create(
    request: ValidationProtocolCreateRequest,
    state: tauri::State<'_, M3State>,
) -> Result<ValidationProtocol, String> {
    validate_protocol(&request, &state)?;
    let windows = request
        .walk_forward
        .as_ref()
        .map(|walk_forward| walk_forward_windows(&state, &request.user_id, walk_forward))
        .transpose()?
        .unwrap_or(request.windows);
    let mut protocol = ValidationProtocol {
        protocol_id: String::new(),
        user_id: request.user_id.clone(),
        run: request.run,
        windows,
        walk_forward: request.walk_forward,
        cross_market: request.cross_market,
        method_version: request.method_version,
        aggregation_rule_version: request.aggregation_rule_version,
    };
    protocol.protocol_id = content_id(&protocol)?;
    state.save_protocol(&protocol)?;
    state.load_protocol(&protocol.user_id, &protocol.protocol_id)
}

#[tauri::command]
pub async fn validation_protocol_list(
    request: ComponentUserRequest,
    state: tauri::State<'_, M3State>,
) -> Result<Vec<ValidationProtocol>, String> {
    state.list_protocols(&request.user_id)
}

#[tauri::command]
pub fn validation_report_run(
    request: ValidationProtocolIdRequest,
    state: tauri::State<'_, M3State>,
) -> Result<ValidationReport, String> {
    run_validation_report(&request, &state)
}

fn run_validation_report(
    request: &ValidationProtocolIdRequest,
    state: &M3State,
) -> Result<ValidationReport, String> {
    let protocol = state.load_protocol(&request.user_id, &request.protocol_id)?;
    if let Some(cross_market) = &protocol.cross_market {
        return run_cross_market_validation(&protocol, cross_market, state);
    }
    let mut windows = Vec::with_capacity(protocol.windows.len());
    for window in &protocol.windows {
        let (sample_in, sample_out) = split_snapshot(&state, &protocol.user_id, window)?;
        let (sample_in_request, sample_out_request) =
            validation_window_run_requests(&protocol, &sample_in, &sample_out);
        let sample_in_snapshot_id = sample_in_request.snapshot_id.clone();
        let sample_out_snapshot_id = sample_out_request.snapshot_id.clone();
        match (
            execute_backtest(sample_in_request, &state),
            execute_backtest(sample_out_request, &state),
        ) {
            (Ok(sample_in_run), Ok(sample_out_run)) => windows.push(ValidationWindowReport {
                sample_out_start_time_ms: window.sample_out_start_time_ms,
                sample_out_end_time_ms: window.sample_out_end_time_ms,
                sample_in_snapshot_id,
                sample_out_snapshot_id,
                sample_in_run_id: Some(sample_in_run.run_id),
                sample_out_run_id: Some(sample_out_run.run_id),
                sample_in_metrics: Some(sample_in_run.result.metrics),
                sample_out_metrics: Some(sample_out_run.result.metrics),
                sample_in_pauses: sample_in_run.pauses,
                sample_out_pauses: sample_out_run.pauses,
                failure: None,
            }),
            (sample_in_result, sample_out_result) => windows.push(ValidationWindowReport {
                sample_out_start_time_ms: window.sample_out_start_time_ms,
                sample_out_end_time_ms: window.sample_out_end_time_ms,
                sample_in_snapshot_id,
                sample_out_snapshot_id,
                sample_in_run_id: sample_in_result.as_ref().ok().map(|run| run.run_id.clone()),
                sample_out_run_id: sample_out_result
                    .as_ref()
                    .ok()
                    .map(|run| run.run_id.clone()),
                sample_in_metrics: sample_in_result
                    .as_ref()
                    .ok()
                    .map(|run| run.result.metrics.clone()),
                sample_out_metrics: sample_out_result
                    .as_ref()
                    .ok()
                    .map(|run| run.result.metrics.clone()),
                sample_in_pauses: sample_in_result
                    .as_ref()
                    .ok()
                    .map(|run| run.pauses.clone())
                    .unwrap_or_default(),
                sample_out_pauses: sample_out_result
                    .as_ref()
                    .ok()
                    .map(|run| run.pauses.clone())
                    .unwrap_or_default(),
                failure: Some(
                    [sample_in_result.err(), sample_out_result.err()]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join("; "),
                ),
            }),
        }
    }
    let aggregate = aggregate_validation(&windows);
    let mut report = ValidationReport {
        report_id: String::new(),
        protocol_id: protocol.protocol_id,
        user_id: protocol.user_id,
        method_version: protocol.method_version,
        aggregation_rule_version: protocol.aggregation_rule_version,
        walk_forward: protocol.walk_forward,
        cross_market: vec![],
        recommended_contexts: vec![],
        cross_market_evidence: None,
        windows,
        aggregate,
    };
    report.report_id = content_id(&report)?;
    state.save_report(&report)?;
    Ok(report)
}

fn validation_window_run_requests(
    protocol: &ValidationProtocol,
    sample_in: &MarketDataSnapshot,
    sample_out: &MarketDataSnapshot,
) -> (BacktestRunRequest, BacktestRunRequest) {
    let mut sample_in_request = protocol.run.clone();
    sample_in_request.user_id = protocol.user_id.clone();
    let mut sample_out_request = sample_in_request.clone();
    if protocol.run.signal_instances.is_empty() {
        sample_in_request.snapshot_id = sample_in.snapshot_id.clone();
        sample_in_request.run_start_time_ms = None;
        sample_in_request.run_end_time_ms = None;
        sample_out_request.snapshot_id = sample_out.snapshot_id.clone();
        sample_out_request.run_start_time_ms = None;
        sample_out_request.run_end_time_ms = None;
    } else {
        sample_in_request.run_start_time_ms = Some(sample_in.start_time_ms);
        sample_in_request.run_end_time_ms = Some(sample_in.end_time_ms);
        sample_out_request.run_start_time_ms = Some(sample_out.start_time_ms);
        sample_out_request.run_end_time_ms = Some(sample_out.end_time_ms);
    }
    (sample_in_request, sample_out_request)
}

fn run_cross_market_validation(
    protocol: &ValidationProtocol,
    cross_market: &CrossMarketValidationRequest,
    state: &M3State,
) -> Result<ValidationReport, String> {
    let contexts = cross_market
        .contexts
        .iter()
        .map(|context| {
            let (snapshot, _) = state.snapshot_for_user(&protocol.user_id, &context.snapshot_id)?;
            let mut run = context
                .run_override
                .clone()
                .unwrap_or_else(|| protocol.run.clone());
            run.user_id = protocol.user_id.clone();
            run.snapshot_id = snapshot.snapshot_id.clone();
            match execute_backtest(run.clone(), state) {
                Ok(result) => Ok(CrossMarketValidationReport {
                    snapshot,
                    run,
                    run_id: Some(result.run_id),
                    metrics: Some(result.result.metrics),
                    pauses: result.pauses,
                    failure: None,
                }),
                Err(error) => Ok(CrossMarketValidationReport {
                    snapshot,
                    run,
                    run_id: None,
                    metrics: None,
                    pauses: vec![],
                    failure: Some(error),
                }),
            }
        })
        .collect::<Result<Vec<_>, String>>()?;
    let aggregate = aggregate_cross_market(&contexts);
    let evidence = cross_market_evidence(&contexts);
    let mut report = ValidationReport {
        report_id: String::new(),
        protocol_id: protocol.protocol_id.clone(),
        user_id: protocol.user_id.clone(),
        method_version: protocol.method_version.clone(),
        aggregation_rule_version: protocol.aggregation_rule_version.clone(),
        walk_forward: None,
        cross_market: contexts,
        recommended_contexts: vec![],
        cross_market_evidence: evidence,
        windows: vec![],
        aggregate,
    };
    report.recommended_contexts = report
        .cross_market
        .iter()
        .enumerate()
        .filter(|(_, context)| context.failure.is_none())
        .map(|(_, context)| RecommendedContext {
            supporting_report_id: report.report_id.clone(),
            snapshot: context.snapshot.clone(),
            run: context.run.clone(),
        })
        .collect();
    report.report_id = validation_report_id(&report)?;
    for context in &mut report.recommended_contexts {
        context.supporting_report_id = report.report_id.clone();
    }
    state.save_report(&report)?;
    Ok(report)
}

#[tauri::command]
pub async fn validation_report_list(
    request: ComponentUserRequest,
    state: tauri::State<'_, M3State>,
) -> Result<Vec<ValidationReport>, String> {
    state.list_reports(&request.user_id)
}

#[tauri::command]
pub fn validation_report_export(
    request: ValidationProtocolIdRequest,
    format: String,
    state: tauri::State<'_, M3State>,
) -> Result<String, String> {
    let report = state
        .list_reports(&request.user_id)?
        .into_iter()
        .find(|report| report.report_id == request.protocol_id)
        .ok_or("Validation Report was not found")?;
    match format.as_str() {
        "json" => serde_json::to_string_pretty(&report).map_err(string),
        "markdown" => Ok(validation_markdown(&report)),
        _ => Err("Validation export format is invalid".into()),
    }
}

fn validate_protocol(
    request: &ValidationProtocolCreateRequest,
    state: &M3State,
) -> Result<(), String> {
    validate_user(&request.user_id)?;
    if request.run.user_id != request.user_id
        || !request
            .aggregation_rule_version
            .starts_with("equal-window@")
    {
        return Err("Validation Protocol is invalid".into());
    }
    validate_run_configuration(&request.user_id, &request.run, state)?;
    match request.method_version.as_str() {
        "chronological-holdout@1"
            if request.walk_forward.is_none() && !request.windows.is_empty() =>
        {
            for window in &request.windows {
                split_snapshot(state, &request.user_id, window)?;
            }
        }
        "walk-forward@1" if request.windows.is_empty() => {
            let walk_forward = request
                .walk_forward
                .as_ref()
                .ok_or("Walk-forward configuration is required")?;
            if request.run.snapshot_id != walk_forward.snapshot_id {
                return Err("Walk-forward must use the frozen Snapshot".into());
            }
            walk_forward_windows(state, &request.user_id, walk_forward)?;
        }
        "cross-market@1"
            if request.windows.is_empty()
                && request.walk_forward.is_none()
                && request.cross_market.is_some() =>
        {
            validate_cross_market(request, state)?;
        }
        _ => return Err("Validation Protocol is invalid".into()),
    }
    Ok(())
}

fn validate_run_configuration(
    user_id: &str,
    run: &BacktestRunRequest,
    state: &M3State,
) -> Result<(), String> {
    if run.user_id != user_id {
        return Err("Validation Run configuration belongs to another User".into());
    }
    state.package_for_user(user_id, &run.strategy_archive_sha256)?;
    for factor in &run.factor_instances {
        state.package_for_user(user_id, &factor.archive_sha256)?;
    }
    Ok(())
}

fn validate_cross_market(
    request: &ValidationProtocolCreateRequest,
    state: &M3State,
) -> Result<(), String> {
    let contexts = &request
        .cross_market
        .as_ref()
        .expect("validated above")
        .contexts;
    if contexts.len() < 2 {
        return Err("Cross-market validation requires at least two markets".into());
    }
    let mut snapshots = HashSet::new();
    let mut markets = HashSet::new();
    let mut interval = None;
    for context in contexts {
        if !snapshots.insert(&context.snapshot_id) {
            return Err("Cross-market validation contains a duplicate Snapshot".into());
        }
        let (snapshot, bars) = state.snapshot_for_user(&request.user_id, &context.snapshot_id)?;
        if bars.is_empty() {
            return Err("Cross-market validation requires market evidence".into());
        }
        if interval
            .replace(snapshot.interval)
            .is_some_and(|current| current != snapshot.interval)
        {
            return Err("Cross-market validation requires compatible Bar Intervals".into());
        }
        if !markets.insert((
            snapshot.src.clone(),
            snapshot.code.clone(),
            snapshot.interval,
        )) {
            return Err("Cross-market validation contains a duplicate Instrument context".into());
        }
        if let Some(run) = &context.run_override {
            if run.snapshot_id != context.snapshot_id {
                return Err("Cross-market override must use its frozen Snapshot".into());
            }
            validate_run_configuration(&request.user_id, run, state)?;
        }
    }
    Ok(())
}

fn split_snapshot(
    state: &M3State,
    user_id: &str,
    window: &ValidationWindowRequest,
) -> Result<(MarketDataSnapshot, MarketDataSnapshot), String> {
    let (snapshot, bars) = state.snapshot_for_user(user_id, &window.snapshot_id)?;
    let split = bars.partition_point(|bar| bar.open_time_ms < window.sample_out_start_time_ms);
    let end = window
        .sample_out_end_time_ms
        .map(|end| bars.partition_point(|bar| bar.open_time_ms < end))
        .unwrap_or(bars.len());
    if split == 0 || split >= end {
        return Err("Validation sample-out window must be non-empty and chronological".into());
    }
    let gaps = snapshot
        .gaps
        .iter()
        .map(|gap| BarGap {
            start_time_ms: gap.start_time_ms,
            end_time_ms: gap.end_time_ms,
        })
        .collect::<Vec<_>>();
    let series = |bars: Vec<OhlcvBar>| adaq_data_core::BarSeries {
        src: snapshot.src.clone(),
        code: snapshot.code.clone(),
        interval: snapshot.interval,
        bars,
        gaps: gaps.clone(),
    };
    Ok((
        state.persist_snapshot_for_user(user_id, &series(bars[..split].to_vec()))?,
        state.persist_snapshot_for_user(user_id, &series(bars[split..end].to_vec()))?,
    ))
}

fn walk_forward_windows(
    state: &M3State,
    user_id: &str,
    request: &WalkForwardValidationRequest,
) -> Result<Vec<ValidationWindowRequest>, String> {
    if request.window_size_bars == 0
        || request.step_size_bars == 0
        || request.minimum_history_bars == 0
    {
        return Err("Walk-forward window sizes must be positive".into());
    }
    if request.step_size_bars < request.window_size_bars {
        return Err("Walk-forward step must not overlap sample-out windows".into());
    }
    let (_, bars) = state.snapshot_for_user(user_id, &request.snapshot_id)?;
    if request.minimum_history_bars >= bars.len() {
        return Err("Walk-forward requires more history than the minimum".into());
    }
    let windows = (request.minimum_history_bars..bars.len())
        .step_by(request.step_size_bars)
        .take_while(|start| start.saturating_add(request.window_size_bars) <= bars.len())
        .map(|start| ValidationWindowRequest {
            snapshot_id: request.snapshot_id.clone(),
            sample_out_start_time_ms: bars[start].open_time_ms,
            sample_out_end_time_ms: bars
                .get(start + request.window_size_bars)
                .map(|bar| bar.open_time_ms),
        })
        .collect::<Vec<_>>();
    if windows.is_empty() {
        Err("Walk-forward history cannot produce a complete window".into())
    } else {
        Ok(windows)
    }
}

fn aggregate_validation(windows: &[ValidationWindowReport]) -> ValidationAggregate {
    let complete = windows
        .iter()
        .filter(|window| window.failure.is_none())
        .collect::<Vec<_>>();
    let count = rust_decimal::Decimal::from(complete.len().max(1));
    let average = |metric: fn(&adaq_backtest_core::BacktestMetrics) -> rust_decimal::Decimal,
                   sample_out: bool| {
        complete
            .iter()
            .map(|window| {
                metric(if sample_out {
                    window.sample_out_metrics.as_ref().unwrap()
                } else {
                    window.sample_in_metrics.as_ref().unwrap()
                })
            })
            .sum::<rust_decimal::Decimal>()
            / count
    };
    ValidationAggregate {
        completed_windows: complete.len(),
        failed_windows: windows.len() - complete.len(),
        average_sample_in_return: average(|metrics| metrics.total_return, false),
        average_sample_out_return: average(|metrics| metrics.total_return, true),
        worst_sample_out_drawdown: complete
            .iter()
            .map(|window| window.sample_out_metrics.as_ref().unwrap().max_drawdown)
            .min()
            .unwrap_or_default(),
        average_sample_out_sharpe: average(|metrics| metrics.sharpe, true),
        total_fees: complete
            .iter()
            .map(|window| {
                window.sample_in_metrics.as_ref().unwrap().total_fees
                    + window.sample_out_metrics.as_ref().unwrap().total_fees
            })
            .sum(),
        total_trades: complete
            .iter()
            .map(|window| {
                window
                    .sample_in_metrics
                    .as_ref()
                    .unwrap()
                    .realized_trade_count
                    + window
                        .sample_out_metrics
                        .as_ref()
                        .unwrap()
                        .realized_trade_count
            })
            .sum(),
    }
}

fn aggregate_cross_market(contexts: &[CrossMarketValidationReport]) -> ValidationAggregate {
    let complete = contexts
        .iter()
        .filter_map(|context| context.metrics.as_ref())
        .collect::<Vec<_>>();
    let count = rust_decimal::Decimal::from(complete.len().max(1));
    ValidationAggregate {
        completed_windows: complete.len(),
        failed_windows: contexts.len() - complete.len(),
        average_sample_in_return: rust_decimal::Decimal::ZERO,
        average_sample_out_return: complete
            .iter()
            .map(|metrics| metrics.total_return)
            .sum::<rust_decimal::Decimal>()
            / count,
        worst_sample_out_drawdown: complete
            .iter()
            .map(|metrics| metrics.max_drawdown)
            .min()
            .unwrap_or_default(),
        average_sample_out_sharpe: complete
            .iter()
            .map(|metrics| metrics.sharpe)
            .sum::<rust_decimal::Decimal>()
            / count,
        total_fees: complete.iter().map(|metrics| metrics.total_fees).sum(),
        total_trades: complete
            .iter()
            .map(|metrics| metrics.realized_trade_count)
            .sum(),
    }
}

fn cross_market_evidence(contexts: &[CrossMarketValidationReport]) -> Option<CrossMarketEvidence> {
    let returns = contexts
        .iter()
        .filter_map(|context| context.metrics.as_ref().map(|metrics| metrics.total_return))
        .collect::<Vec<_>>();
    Some(CrossMarketEvidence {
        completed_markets: returns.len(),
        total_return_spread: returns
            .iter()
            .max()
            .zip(returns.iter().min())
            .map(|(max, min)| *max - *min)
            .unwrap_or_default(),
    })
}

fn content_id(value: &impl Serialize) -> Result<String, String> {
    let value = canonical_json(serde_json::to_value(value).map_err(string)?);
    Ok(Sha256::digest(serde_json::to_vec(&value).map_err(string)?)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn validation_report_id(report: &ValidationReport) -> Result<String, String> {
    let mut value = serde_json::to_value(report).map_err(string)?;
    let object = value
        .as_object_mut()
        .expect("Validation Report serializes as an object");
    object.remove("reportId");
    if let Some(serde_json::Value::Array(contexts)) = object.get_mut("recommendedContexts") {
        for context in contexts {
            context
                .as_object_mut()
                .expect("Recommended Context serializes as an object")
                .remove("supportingReportId");
        }
    }
    content_id(&value)
}

fn report_references_run(report: &ValidationReport, run_id: &str) -> bool {
    report.windows.iter().any(|window| {
        window.sample_in_run_id.as_deref() == Some(run_id)
            || window.sample_out_run_id.as_deref() == Some(run_id)
    }) || report
        .cross_market
        .iter()
        .any(|context| context.run_id.as_deref() == Some(run_id))
}

fn canonical_json(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.into_iter().map(canonical_json).collect())
        }
        serde_json::Value::Object(values) => serde_json::Value::Object(
            values
                .into_iter()
                .map(|(key, value)| (key, canonical_json(value)))
                .collect(),
        ),
        value => value,
    }
}

fn validation_markdown(report: &ValidationReport) -> String {
    format!(
        "# Validation Report {}\n\n```json\n{}\n```\n",
        report.report_id,
        serde_json::to_string_pretty(report).expect("Validation Report serializes")
    )
}

fn fingerprint(user_id: &str, provenance: &BacktestRunProvenance) -> Result<String, String> {
    let digest = Sha256::digest(serde_json::to_vec(&(user_id, provenance)).map_err(string)?);
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn normalized_parameters(
    manifest: &ComponentManifest,
    values: &[adaq_component_tooling::ComponentParameterValue],
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
    values: &[adaq_component_tooling::ComponentParameterValue],
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

fn parameter_value(value: &adaq_component_tooling::ComponentParameterValue) -> String {
    match value {
        adaq_component_tooling::ComponentParameterValue::Decimal(value)
        | adaq_component_tooling::ComponentParameterValue::String(value) => value.clone(),
        adaq_component_tooling::ComponentParameterValue::Integer(value) => value.to_string(),
        adaq_component_tooling::ComponentParameterValue::Boolean(value) => value.to_string(),
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

fn validate_provenance(provenance: &BacktestRunProvenance) -> Result<(), String> {
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

fn run_view(run: &BacktestRun, start: i64, end: i64, max_points: usize) -> BacktestRunView {
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

pub(crate) fn validate_user(user_id: &str) -> Result<(), String> {
    if user_id.trim().is_empty() || user_id.len() > 128 {
        Err("User ID is invalid".into())
    } else {
        Ok(())
    }
}

fn validate_snapshot_request(
    user_id: &str,
    src: &str,
    code: &str,
    start_time_ms: i64,
    end_time_ms: i64,
) -> Result<(), String> {
    validate_user(user_id)?;
    if src.trim().is_empty() || code.trim().is_empty() || start_time_ms >= end_time_ms {
        Err("Snapshot time range is invalid".into())
    } else {
        Ok(())
    }
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

fn reset_components(database: &mut Connection, user_id: &str, root: &Path) -> Result<(), String> {
    let blocking: i64 = database
        .query_row(
            "SELECT (SELECT COUNT(DISTINCT r.run_id) FROM backtest_runs r
             JOIN backtest_run_components rc USING(run_id)
             JOIN component_access a USING(archive_sha256)
             WHERE r.user_id = ?1 AND a.user_id = ?1)
             + (SELECT COUNT(*) FROM signal_dataset_access WHERE user_id = ?1)",
            [user_id],
            |row| row.get(0),
        )
        .map_err(string)?;
    if blocking > 0 {
        return Err(format!(
            "Component Package reset is blocked by {blocking} immutable Backtest Run(s)"
        ));
    }
    let paths = strings(
        database,
        "SELECT c.archive_path FROM component_content c
         JOIN component_access a USING(archive_sha256)
         WHERE a.user_id = ?1
         AND NOT EXISTS(SELECT 1 FROM component_access other
             WHERE other.archive_sha256 = c.archive_sha256 AND other.user_id <> ?1)
         AND NOT EXISTS(SELECT 1 FROM backtest_run_components rc
             WHERE rc.archive_sha256 = c.archive_sha256)",
        user_id,
    )?;
    let staged = stage_files(paths.iter().map(PathBuf::from), root)?;
    let result = (|| {
        let transaction = database.transaction().map_err(string)?;
        transaction
            .execute("DELETE FROM component_access WHERE user_id = ?1", [user_id])
            .map_err(string)?;
        transaction
            .execute(
                "DELETE FROM component_content
                 WHERE NOT EXISTS(SELECT 1 FROM component_access a
                     WHERE a.archive_sha256 = component_content.archive_sha256)
                 AND NOT EXISTS(SELECT 1 FROM backtest_run_components rc
                     WHERE rc.archive_sha256 = component_content.archive_sha256)",
                [],
            )
            .map_err(string)?;
        transaction.commit().map_err(string)
    })();
    finish_staged_files(staged, result)
}

fn reset_market_data(database: &mut Connection, user_id: &str, root: &Path) -> Result<(), String> {
    let blocking: i64 = database
        .query_row(
            "SELECT (SELECT COUNT(*) FROM backtest_runs WHERE user_id = ?1)
             + (SELECT COUNT(*) FROM validation_reports WHERE user_id = ?1)
             + (SELECT COUNT(*) FROM signal_dataset_access WHERE user_id = ?1)",
            [user_id],
            |row| row.get(0),
        )
        .map_err(string)?;
    if blocking > 0 {
        return Err(format!(
            "Market Data reset is blocked by {blocking} immutable research record(s)"
        ));
    }
    let metadata = strings(
        database,
        "SELECT s.metadata_json FROM market_data_snapshots s
         JOIN market_data_snapshot_access a USING(snapshot_id)
         WHERE a.user_id = ?1
         AND NOT EXISTS(SELECT 1 FROM market_data_snapshot_access other
             WHERE other.snapshot_id = s.snapshot_id AND other.user_id <> ?1)",
        user_id,
    )?;
    let paths = metadata
        .iter()
        .map(|json| serde_json::from_str::<MarketDataSnapshot>(json).map_err(string))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(|snapshot| snapshot.parquet_path);
    let staged = stage_files(paths, root)?;
    let result = (|| {
        let transaction = database.transaction().map_err(string)?;
        transaction
            .execute(
                "DELETE FROM market_data_snapshot_access WHERE user_id = ?1",
                [user_id],
            )
            .map_err(string)?;
        transaction
            .execute(
                "DELETE FROM market_data_snapshots
                 WHERE NOT EXISTS(SELECT 1 FROM market_data_snapshot_access a
                     WHERE a.snapshot_id = market_data_snapshots.snapshot_id)",
                [],
            )
            .map_err(string)?;
        transaction.commit().map_err(string)
    })();
    finish_staged_files(staged, result)
}

fn reset_all(database: &mut Connection, user_id: &str, root: &Path) -> Result<(), String> {
    let component_paths = strings(
        database,
        "SELECT c.archive_path FROM component_content c
         JOIN component_access a USING(archive_sha256)
         WHERE a.user_id = ?1
         AND NOT EXISTS(SELECT 1 FROM component_access other
             WHERE other.archive_sha256 = c.archive_sha256 AND other.user_id <> ?1)
         AND NOT EXISTS(SELECT 1 FROM backtest_run_components rc
             JOIN backtest_runs r USING(run_id)
             WHERE rc.archive_sha256 = c.archive_sha256 AND r.user_id <> ?1)",
        user_id,
    )?;
    let snapshot_metadata = strings(
        database,
        "SELECT s.metadata_json FROM market_data_snapshots s
         JOIN market_data_snapshot_access a USING(snapshot_id)
         WHERE a.user_id = ?1
         AND NOT EXISTS(SELECT 1 FROM market_data_snapshot_access other
             WHERE other.snapshot_id = s.snapshot_id AND other.user_id <> ?1)",
        user_id,
    )?;
    let paths = component_paths.into_iter().map(PathBuf::from).chain(
        snapshot_metadata
            .iter()
            .map(|json| serde_json::from_str::<MarketDataSnapshot>(json).map_err(string))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .map(|snapshot| snapshot.parquet_path),
    );
    let dataset_paths = strings(
        database,
        "SELECT c.parquet_path FROM signal_dataset_content c JOIN signal_dataset_access a USING(dataset_id) WHERE a.user_id = ?1 AND NOT EXISTS(SELECT 1 FROM signal_dataset_access other WHERE other.dataset_id = c.dataset_id AND other.user_id <> ?1)",
        user_id,
    )?;
    let staged = stage_files(
        paths.chain(dataset_paths.into_iter().map(PathBuf::from)),
        root,
    )?;
    let result = (|| {
        let transaction = database.transaction().map_err(string)?;
        transaction
            .execute(
                "DELETE FROM validation_reports WHERE user_id = ?1",
                [user_id],
            )
            .map_err(string)?;
        transaction
            .execute(
                "DELETE FROM dataset_generation_attempts WHERE user_id = ?1",
                [user_id],
            )
            .map_err(string)?;
        transaction
            .execute(
                "DELETE FROM forecast_evaluation_access WHERE user_id = ?1",
                [user_id],
            )
            .map_err(string)?;
        transaction.execute("DELETE FROM forecast_evaluation_content WHERE NOT EXISTS(SELECT 1 FROM forecast_evaluation_access a WHERE a.report_id = forecast_evaluation_content.report_id)", []).map_err(string)?;
        transaction
            .execute("DELETE FROM backtest_runs WHERE user_id = ?1", [user_id])
            .map_err(string)?;
        transaction
            .execute(
                "DELETE FROM signal_dataset_access WHERE user_id = ?1",
                [user_id],
            )
            .map_err(string)?;
        transaction.execute("DELETE FROM signal_dataset_content WHERE NOT EXISTS(SELECT 1 FROM signal_dataset_access a WHERE a.dataset_id = signal_dataset_content.dataset_id)", []).map_err(string)?;
        transaction
            .execute(
                "DELETE FROM validation_protocols WHERE user_id = ?1",
                [user_id],
            )
            .map_err(string)?;
        transaction
            .execute("DELETE FROM component_access WHERE user_id = ?1", [user_id])
            .map_err(string)?;
        transaction
            .execute(
                "DELETE FROM market_data_snapshot_access WHERE user_id = ?1",
                [user_id],
            )
            .map_err(string)?;
        transaction
            .execute(
                "DELETE FROM component_content
                 WHERE NOT EXISTS(SELECT 1 FROM component_access a
                     WHERE a.archive_sha256 = component_content.archive_sha256)
                 AND NOT EXISTS(SELECT 1 FROM backtest_run_components rc
                     WHERE rc.archive_sha256 = component_content.archive_sha256)",
                [],
            )
            .map_err(string)?;
        transaction
            .execute(
                "DELETE FROM market_data_snapshots
                 WHERE NOT EXISTS(SELECT 1 FROM market_data_snapshot_access a
                     WHERE a.snapshot_id = market_data_snapshots.snapshot_id)",
                [],
            )
            .map_err(string)?;
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
    use crate::watchlist::WatchlistDb;
    use adaq_component_tooling::{ComponentManifest, pack_component};
    use std::{
        io::{Cursor, Write},
        time::{SystemTime, UNIX_EPOCH},
    };
    use zip::{ZipWriter, write::SimpleFileOptions};

    fn local_data_state(name: &str) -> (PathBuf, M3State, WatchlistDb) {
        let root = std::env::temp_dir().join(format!(
            "adaq-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let state = M3State::open(&root).unwrap();
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
    fn upgrades_generation_attempts_with_progress_columns() {
        let root = std::env::temp_dir().join(format!(
            "adaq-generation-attempt-migration-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let database = Connection::open(root.join("adaq.db")).unwrap();
        database
            .execute_batch(
                "CREATE TABLE dataset_generation_attempts (
                    attempt_id TEXT PRIMARY KEY,
                    request_hash TEXT NOT NULL,
                    user_id TEXT NOT NULL,
                    dataset_id TEXT,
                    status TEXT NOT NULL,
                    request_json TEXT NOT NULL,
                    diagnostic_json TEXT,
                    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                );",
            )
            .unwrap();
        drop(database);

        let state = M3State::open(&root).unwrap();
        let database = state.database.lock().unwrap();
        database
            .query_row(
                "SELECT progress_completed, progress_total FROM dataset_generation_attempts LIMIT 1",
                [],
                |_| Ok(()),
            )
            .optional()
            .unwrap();
        drop(database);
        drop(state);
        fs::remove_dir_all(root).unwrap();
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
			"INSERT INTO component_content VALUES ('hash', 'component', '1.0.0', 'Shared', 'factor', 'wasm', ?1)",
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
        database.execute("INSERT INTO dataset_generation_attempts(attempt_id, request_hash, user_id, status, request_json) VALUES ('attempt', 'request', 'alice', 'failed', '{}')", []).unwrap();
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
			"INSERT INTO component_content VALUES ('hash', 'component', '1.0.0', 'Locked', 'factor', 'wasm', ?1)",
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
        let state = M3State::open(&root).unwrap();
        let database = state.database.lock().unwrap();
        for index in 0..12 {
            let archive_sha256 = format!("{index:064x}");
            let path = root.join(format!("invalid-{index}.adaq"));
            fs::write(&path, b"invalid package").unwrap();
            database
                .execute(
                    "INSERT INTO component_content VALUES (?1, ?2, '1.0.0', ?3, 'factor', ?4, ?5)",
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

    #[test]
    fn run_history_is_filtered_and_paged_by_instrument() {
        let root = std::env::temp_dir().join(format!(
            "adaq-run-history-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let state = M3State::open(&root).unwrap();
        let database = state.database.lock().unwrap();
        for index in 0..12 {
            let json = serde_json::json!({
                "snapshot": {
                    "snapshotId": format!("btc-{index}"),
                    "src": "okx",
                    "code": "BTC-USDT",
                    "interval": "1h",
                    "barCount": 100
                },
                "result": { "metrics": { "totalReturn": "0.1" } }
            });
            database
                .execute(
                    "INSERT INTO backtest_runs(run_id, user_id, created_at, result_json) VALUES (?1, 'alice', ?2, ?3)",
                    params![format!("btc-{index}"), format!("2026-07-30 00:{index:02}:00"), json.to_string()],
                )
                .unwrap();
        }
        for (user, code) in [("alice", "ETH-USDT"), ("bob", "BTC-USDT")] {
            let json = serde_json::json!({
                "snapshot": {
                    "snapshotId": format!("{user}-{code}"),
                    "src": "okx",
                    "code": code,
                    "interval": "1h",
                    "barCount": 100
                },
                "result": { "metrics": { "totalReturn": "0.2" } }
            });
            database
                .execute(
                    "INSERT INTO backtest_runs(run_id, user_id, created_at, result_json) VALUES (?1, ?2, '2026-07-30 01:00:00', ?3)",
                    params![format!("{user}-{code}"), user, json.to_string()],
                )
                .unwrap();
        }
        drop(database);

        let first = state
            .list_runs_page("alice", Some("okx"), Some("BTC-USDT"), 1)
            .unwrap();
        assert_eq!(first.total, 12);
        assert_eq!(first.items.len(), 10);
        assert!(first.items.iter().all(|run| run.code == "BTC-USDT"));

        let second = state
            .list_runs_page("alice", Some("okx"), Some("BTC-USDT"), 2)
            .unwrap();
        assert_eq!(second.total, 12);
        assert_eq!(second.items.len(), 2);

        let eth = state
            .list_runs_page("alice", Some("okx"), Some("ETH-USDT"), 1)
            .unwrap();
        assert_eq!(eth.total, 1);
        assert_eq!(eth.items[0].code, "ETH-USDT");

        let all = state.list_runs_page("alice", None, None, 1).unwrap();
        assert_eq!(all.total, 13);
        assert_eq!(all.items.len(), 10);
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
        let state = M3State::open(&root).unwrap();
        let archive_hash = "a".repeat(64);
        let path = root.join("legacy.adaq");
        fs::write(&path, legacy_package()).unwrap();
        let database = state.database.lock().unwrap();
        database
            .execute(
                "INSERT INTO component_content VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
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
    fn component_list_revalidates_stored_identity_and_hashes() {
        let root = std::env::temp_dir().join(format!(
            "adaq-replaced-component-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let state = M3State::open(&root).unwrap();
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
        assert!(!listed[0].compatible);
        assert!(
            listed[0]
                .compatibility_error
                .as_deref()
                .unwrap()
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
        let state = M3State::open(&root).unwrap();
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
        let preview = prepare_backtest(&request(), &state).unwrap();
        assert!(state.load_run("alice", &preview.run_id).is_err());
        assert_eq!(
            preview
                .provenance
                .normalized_request
                .initial_quote_allocation,
            rust_decimal::Decimal::from(10_000),
        );
        assert!(preview.provenance.feature_plan_json.contains("\"slots\""));
        let mut subset = request();
        subset.run_start_time_ms = Some(3_600_000);
        subset.run_end_time_ms = Some(snapshot.end_time_ms);
        let subset_preview = prepare_backtest(&subset, &state).unwrap();
        assert_eq!(subset_preview.bars.len(), 2);
        assert_ne!(subset_preview.run_id, preview.run_id);
        subset.run_start_time_ms = Some(snapshot.start_time_ms - 1);
        assert!(
            prepare_backtest(&subset, &state)
                .err()
                .unwrap()
                .contains("subset")
        );

        let first = execute_backtest(request(), &state).unwrap();
        let second = execute_backtest(request(), &state).unwrap();
        assert!(!first.plan_hash.is_empty());
        let mut changed_seed = first.provenance.clone().unwrap();
        changed_seed.seed = 1;
        changed_seed.normalized_request.seed = 1;
        assert_ne!(
            fingerprint("alice", first.provenance.as_ref().unwrap()).unwrap(),
            fingerprint("alice", &changed_seed).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&first).unwrap(),
            serde_json::to_value(&second).unwrap()
        );
        assert_eq!(state.list_runs("alice").unwrap().len(), 1);
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
        state.delete_run("alice", &first.run_id).unwrap();
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
        let state = M3State::open(&root).unwrap();
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

        let first = execute_backtest(request(), &state).unwrap();
        let replay = execute_backtest(request(), &state).unwrap();

        assert_eq!(first.run_id, replay.run_id);
        let provenance = first.provenance.as_ref().unwrap();
        assert_eq!(
            provenance.normalized_request.snapshot_id,
            snapshot.snapshot_id
        );
        assert_eq!(provenance.feature_plan_hash, first.plan_hash);
        assert_eq!(provenance.component_lock, first.component_lock);
        assert_eq!(
            state.load_run("alice", &first.run_id).unwrap().provenance,
            first.provenance
        );
        assert!(provenance.feature_plan_json.contains("\"slots\""));
        assert_eq!(
            provenance.normalized_request.initial_quote_allocation,
            rust_decimal::Decimal::from(10_000),
        );
        let mut inconsistent = provenance.clone();
        inconsistent.component_lock[0].archive_sha256 = "f".repeat(64);
        assert!(validate_provenance(&inconsistent).is_err());
        let mut inconsistent = provenance.clone();
        inconsistent.normalized_request.factor_instances[0].archive_sha256 = "f".repeat(64);
        assert!(validate_provenance(&inconsistent).is_err());
        let mut inconsistent = provenance.clone();
        inconsistent.normalized_request.factor_instances[0]
            .parameters
            .push(NormalizedParameter {
                name: "unexpected".into(),
                value: "1".into(),
            });
        assert!(validate_provenance(&inconsistent).is_err());
        let mut inconsistent = provenance.clone();
        inconsistent.normalized_request.factor_instances[0].alias = "other".into();
        assert!(validate_provenance(&inconsistent).is_err());
        let mut inconsistent = provenance.clone();
        inconsistent.feature_plan_json = inconsistent
            .feature_plan_json
            .replacen("momentum", "tampered", 1);
        assert!(validate_provenance(&inconsistent).is_err());
        assert_eq!(first.component_lock.len(), 2);
        assert_eq!(first.pauses.len(), 38);
        assert!(!first.result.orders.is_empty());
        assert!(!first.result.fills.is_empty());
        let execution_page = execution_page(&first.result, 0, 1);
        assert_eq!(execution_page.orders.len(), 1);
        assert_eq!(execution_page.fills.len(), 1);
        assert_eq!(execution_page.total_orders, first.result.orders.len());
        assert_eq!(execution_page.total_fills, first.result.fills.len());
        assert_eq!(state.list_runs("alice").unwrap().len(), 1);
        let mut changed_request = request();
        changed_request.seed = 1;
        let changed = execute_backtest(changed_request, &state).unwrap();
        assert_ne!(first.run_id, changed.run_id);
        assert_eq!(state.list_runs("alice").unwrap().len(), 2);

        let validation = ValidationProtocolCreateRequest {
            user_id: "alice".into(),
            run: request(),
            windows: vec![ValidationWindowRequest {
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
            validate_protocol(
                &ValidationProtocolCreateRequest {
                    windows: vec![ValidationWindowRequest {
                        snapshot_id: snapshot.snapshot_id.clone(),
                        sample_out_start_time_ms: 0,
                        sample_out_end_time_ms: None,
                    }],
                    ..validation.clone()
                },
                &state,
            )
            .is_err()
        );
        let protocol = ValidationProtocol {
            protocol_id: String::new(),
            user_id: validation.user_id.clone(),
            run: validation.run.clone(),
            windows: validation.windows.clone(),
            walk_forward: validation.walk_forward.clone(),
            cross_market: validation.cross_market.clone(),
            method_version: validation.method_version.clone(),
            aggregation_rule_version: validation.aggregation_rule_version.clone(),
        };
        let protocol_id = content_id(&protocol).unwrap();
        let protocol = ValidationProtocol {
            protocol_id,
            ..protocol
        };
        state.save_protocol(&protocol).unwrap();
        let sample_report = {
            let (sample_in, sample_out) =
                split_snapshot(&state, "alice", &validation.windows[0]).unwrap();
            let (composed_in, composed_out) =
                validation_window_run_requests(&protocol, &sample_in, &sample_out);
            assert_eq!(composed_in.snapshot_id, sample_in.snapshot_id);
            assert_eq!(composed_out.snapshot_id, sample_out.snapshot_id);
            assert_eq!(composed_in.run_start_time_ms, None);
            assert_eq!(composed_out.run_end_time_ms, None);
            let mut signal_protocol = protocol.clone();
            signal_protocol.run.signal_instances = vec![SignalInstanceRequest {
                slot: "forecast".into(),
                dataset_id: "dataset".into(),
                signal_name: "up".into(),
            }];
            let (signal_in, signal_out) =
                validation_window_run_requests(&signal_protocol, &sample_in, &sample_out);
            assert_eq!(signal_in.snapshot_id, protocol.run.snapshot_id);
            assert_eq!(signal_out.snapshot_id, protocol.run.snapshot_id);
            assert_eq!(signal_in.run_start_time_ms, Some(sample_in.start_time_ms));
            assert_eq!(signal_in.run_end_time_ms, Some(sample_in.end_time_ms));
            assert_eq!(signal_out.run_start_time_ms, Some(sample_out.start_time_ms));
            assert_eq!(signal_out.run_end_time_ms, Some(sample_out.end_time_ms));
            let mut sample_in_request = validation.run.clone();
            sample_in_request.snapshot_id = sample_in.snapshot_id.clone();
            let mut sample_out_request = sample_in_request.clone();
            sample_out_request.snapshot_id = sample_out.snapshot_id.clone();
            let sample_in_run = execute_backtest(sample_in_request, &state).unwrap();
            let sample_out_run = execute_backtest(sample_out_request, &state).unwrap();
            ValidationWindowReport {
                sample_out_start_time_ms: validation.windows[0].sample_out_start_time_ms,
                sample_out_end_time_ms: validation.windows[0].sample_out_end_time_ms,
                sample_in_snapshot_id: sample_in.snapshot_id,
                sample_out_snapshot_id: sample_out.snapshot_id,
                sample_in_run_id: Some(sample_in_run.run_id),
                sample_out_run_id: Some(sample_out_run.run_id),
                sample_in_metrics: Some(sample_in_run.result.metrics),
                sample_out_metrics: Some(sample_out_run.result.metrics),
                sample_in_pauses: sample_in_run.pauses,
                sample_out_pauses: sample_out_run.pauses,
                failure: None,
            }
        };
        let aggregate = aggregate_validation(&[sample_report.clone()]);
        assert_eq!(aggregate.completed_windows, 1);
        let mut report = ValidationReport {
            report_id: String::new(),
            protocol_id: protocol.protocol_id.clone(),
            user_id: "alice".into(),
            method_version: protocol.method_version.clone(),
            aggregation_rule_version: protocol.aggregation_rule_version.clone(),
            walk_forward: protocol.walk_forward.clone(),
            cross_market: vec![],
            recommended_contexts: vec![],
            cross_market_evidence: None,
            windows: vec![sample_report],
            aggregate,
        };
        report.report_id = content_id(&report).unwrap();
        state.save_report(&report).unwrap();
        assert!(
            state
                .delete_run(
                    "alice",
                    report.windows[0].sample_in_run_id.as_deref().unwrap(),
                )
                .is_err()
        );
        assert_eq!(state.list_reports("alice").unwrap().len(), 1);
        assert!(state.list_reports("bob").unwrap().is_empty());
        assert!(validation_markdown(&report).contains(&report.protocol_id));
        assert!(
            serde_json::to_string(&report)
                .unwrap()
                .contains(&report.report_id)
        );
        let changed_protocol = ValidationProtocol {
            aggregation_rule_version: "equal-window@2".into(),
            ..protocol.clone()
        };
        assert_ne!(
            content_id(&protocol).unwrap(),
            content_id(&changed_protocol).unwrap()
        );
        let failed = ValidationWindowReport {
            sample_out_start_time_ms: 0,
            sample_out_end_time_ms: None,
            sample_in_snapshot_id: snapshot.snapshot_id.clone(),
            sample_out_snapshot_id: snapshot.snapshot_id.clone(),
            sample_in_run_id: Some(first.run_id.clone()),
            sample_out_run_id: None,
            sample_in_metrics: Some(first.result.metrics.clone()),
            sample_out_metrics: None,
            sample_in_pauses: first.pauses.clone(),
            sample_out_pauses: vec![],
            failure: Some("unavailable".into()),
        };
        assert_eq!(aggregate_validation(&[failed]).failed_windows, 1);

        let walk_forward = WalkForwardValidationRequest {
            snapshot_id: snapshot.snapshot_id.clone(),
            window_size_bars: 5,
            step_size_bars: 5,
            minimum_history_bars: 10,
        };
        let generated_walk_forward_windows =
            walk_forward_windows(&state, "alice", &walk_forward).unwrap();
        assert_eq!(
            generated_walk_forward_windows
                .iter()
                .map(|window| window.sample_out_start_time_ms)
                .collect::<Vec<_>>(),
            vec![
                10 * 3_600_000,
                15 * 3_600_000,
                20 * 3_600_000,
                30 * 3_600_000,
                35 * 3_600_000,
                40 * 3_600_000,
                45 * 3_600_000,
                50 * 3_600_000
            ]
        );
        assert_eq!(
            generated_walk_forward_windows[0].sample_out_end_time_ms,
            Some(15 * 3_600_000)
        );
        let gap_window = generated_walk_forward_windows
            .iter()
            .find(|window| window.sample_out_start_time_ms == 30 * 3_600_000)
            .unwrap();
        assert_eq!(
            split_snapshot(&state, "alice", gap_window)
                .unwrap()
                .1
                .bar_count,
            5
        );
        assert!(
            walk_forward_windows(
                &state,
                "alice",
                &WalkForwardValidationRequest {
                    minimum_history_bars: 50,
                    ..walk_forward.clone()
                },
            )
            .is_err()
        );
        assert!(
            walk_forward_windows(
                &state,
                "alice",
                &WalkForwardValidationRequest {
                    step_size_bars: 4,
                    ..walk_forward.clone()
                },
            )
            .is_err()
        );
        let partial_tail_windows = walk_forward_windows(
            &state,
            "alice",
            &WalkForwardValidationRequest {
                window_size_bars: 6,
                step_size_bars: 6,
                ..walk_forward.clone()
            },
        )
        .unwrap();
        assert_eq!(partial_tail_windows.len(), 6);
        assert_eq!(
            partial_tail_windows
                .last()
                .unwrap()
                .sample_out_start_time_ms,
            45 * 3_600_000
        );
        let walk_forward_request = ValidationProtocolCreateRequest {
            user_id: "alice".into(),
            run: request(),
            windows: vec![],
            walk_forward: Some(walk_forward.clone()),
            cross_market: None,
            method_version: "walk-forward@1".into(),
            aggregation_rule_version: "equal-window@1".into(),
        };
        validate_protocol(&walk_forward_request, &state).unwrap();
        let mut walk_forward_protocol = ValidationProtocol {
            protocol_id: String::new(),
            user_id: walk_forward_request.user_id.clone(),
            run: walk_forward_request.run.clone(),
            windows: walk_forward_windows(&state, "alice", &walk_forward).unwrap(),
            walk_forward: walk_forward_request.walk_forward.clone(),
            cross_market: walk_forward_request.cross_market.clone(),
            method_version: walk_forward_request.method_version.clone(),
            aggregation_rule_version: walk_forward_request.aggregation_rule_version.clone(),
        };
        walk_forward_protocol.protocol_id = content_id(&walk_forward_protocol).unwrap();
        let mut changed_walk_forward_protocol = walk_forward_protocol.clone();
        changed_walk_forward_protocol
            .walk_forward
            .as_mut()
            .unwrap()
            .step_size_bars = 6;
        assert_ne!(
            content_id(&walk_forward_protocol).unwrap(),
            content_id(&changed_walk_forward_protocol).unwrap()
        );
        state.save_protocol(&walk_forward_protocol).unwrap();
        assert_eq!(
            state
                .load_protocol("alice", &walk_forward_protocol.protocol_id)
                .unwrap()
                .walk_forward
                .as_ref()
                .unwrap()
                .minimum_history_bars,
            10
        );
        let unavailable_walk_forward = WalkForwardValidationRequest {
            minimum_history_bars: 40,
            ..walk_forward.clone()
        };
        let mut unavailable_run = request();
        unavailable_run.strategy_archive_sha256 = "0".repeat(64);
        let mut unavailable_protocol = ValidationProtocol {
            protocol_id: String::new(),
            user_id: "alice".into(),
            run: unavailable_run,
            windows: walk_forward_windows(&state, "alice", &unavailable_walk_forward).unwrap(),
            walk_forward: Some(unavailable_walk_forward),
            cross_market: None,
            method_version: "walk-forward@1".into(),
            aggregation_rule_version: "equal-window@1".into(),
        };
        unavailable_protocol.protocol_id = content_id(&unavailable_protocol).unwrap();
        state.save_protocol(&unavailable_protocol).unwrap();
        let unavailable_report = run_validation_report(
            &ValidationProtocolIdRequest {
                user_id: "alice".into(),
                protocol_id: unavailable_protocol.protocol_id,
            },
            &state,
        )
        .unwrap();
        assert_eq!(unavailable_report.aggregate.failed_windows, 2);
        assert!(
            unavailable_report
                .windows
                .iter()
                .all(|window| window.failure.is_some())
        );
        let resumable_walk_forward = WalkForwardValidationRequest {
            minimum_history_bars: 45,
            ..walk_forward
        };
        let mut resumable_protocol = ValidationProtocol {
            protocol_id: String::new(),
            user_id: "alice".into(),
            run: request(),
            windows: walk_forward_windows(&state, "alice", &resumable_walk_forward).unwrap(),
            walk_forward: Some(resumable_walk_forward),
            cross_market: None,
            method_version: "walk-forward@1".into(),
            aggregation_rule_version: "equal-window@1".into(),
        };
        resumable_protocol.protocol_id = content_id(&resumable_protocol).unwrap();
        state.save_protocol(&resumable_protocol).unwrap();
        let report_request = ValidationProtocolIdRequest {
            user_id: "alice".into(),
            protocol_id: resumable_protocol.protocol_id.clone(),
        };
        let first_report = run_validation_report(&report_request, &state).unwrap();
        let run_count = state.list_runs("alice").unwrap().len();
        let resumed_report = run_validation_report(&report_request, &state).unwrap();
        assert_eq!(first_report.report_id, resumed_report.report_id);
        assert_eq!(state.list_runs("alice").unwrap().len(), run_count);
        assert_eq!(first_report.windows.len(), 1);
        assert_eq!(
            first_report.windows[0].sample_out_start_time_ms,
            50 * 3_600_000
        );
        assert!(first_report.windows[0].sample_out_end_time_ms.is_none());
        assert!(validation_markdown(&first_report).contains("walk-forward@1"));

        let (_, source_bars) = state
            .snapshot_for_user("alice", &snapshot.snapshot_id)
            .unwrap();
        let eth_snapshot = state
            .persist_snapshot_for_user(
                "alice",
                &adaq_data_core::BarSeries {
                    src: "okx".into(),
                    code: "ETH-USDT".into(),
                    interval: BarInterval::OneHour,
                    bars: source_bars.clone(),
                    gaps: vec![],
                },
            )
            .unwrap();
        let cross_market = CrossMarketValidationRequest {
            contexts: vec![
                CrossMarketValidationContextRequest {
                    snapshot_id: snapshot.snapshot_id.clone(),
                    run_override: None,
                },
                CrossMarketValidationContextRequest {
                    snapshot_id: eth_snapshot.snapshot_id.clone(),
                    run_override: None,
                },
            ],
        };
        let cross_market_request = ValidationProtocolCreateRequest {
            user_id: "alice".into(),
            run: request(),
            windows: vec![],
            walk_forward: None,
            cross_market: Some(cross_market.clone()),
            method_version: "cross-market@1".into(),
            aggregation_rule_version: "equal-window@1".into(),
        };
        validate_protocol(&cross_market_request, &state).unwrap();
        let mut cross_market_protocol = ValidationProtocol {
            protocol_id: String::new(),
            user_id: cross_market_request.user_id.clone(),
            run: cross_market_request.run.clone(),
            windows: vec![],
            walk_forward: None,
            cross_market: Some(cross_market.clone()),
            method_version: cross_market_request.method_version.clone(),
            aggregation_rule_version: cross_market_request.aggregation_rule_version.clone(),
        };
        cross_market_protocol.protocol_id = content_id(&cross_market_protocol).unwrap();
        state.save_protocol(&cross_market_protocol).unwrap();
        let cross_market_request_id = ValidationProtocolIdRequest {
            user_id: "alice".into(),
            protocol_id: cross_market_protocol.protocol_id.clone(),
        };
        let cross_market_report = run_validation_report(&cross_market_request_id, &state).unwrap();
        assert_eq!(cross_market_report.cross_market.len(), 2);
        assert_eq!(
            cross_market_report.cross_market[0].snapshot.code,
            "BTC-USDT"
        );
        assert_eq!(
            cross_market_report.cross_market[1].snapshot.code,
            "ETH-USDT"
        );
        assert_eq!(cross_market_report.aggregate.completed_windows, 2);
        assert_eq!(
            cross_market_report
                .cross_market_evidence
                .as_ref()
                .unwrap()
                .completed_markets,
            2
        );
        assert!(
            cross_market_report
                .cross_market
                .iter()
                .all(|context| context.failure.is_none())
        );
        assert!(
            cross_market_report
                .recommended_contexts
                .iter()
                .all(|context| {
                    context.supporting_report_id == cross_market_report.report_id
                        && context.run.snapshot_id == context.snapshot.snapshot_id
                })
        );
        assert_eq!(
            validation_report_id(&cross_market_report).unwrap(),
            cross_market_report.report_id
        );
        let cross_market_run_count = state.list_runs("alice").unwrap().len();
        assert_eq!(
            run_validation_report(&cross_market_request_id, &state)
                .unwrap()
                .report_id,
            cross_market_report.report_id
        );
        assert_eq!(
            state.list_runs("alice").unwrap().len(),
            cross_market_run_count
        );
        assert!(
            state
                .list_reports("alice")
                .unwrap()
                .iter()
                .any(|report| report.report_id == cross_market_report.report_id)
        );
        assert!(validation_markdown(&cross_market_report).contains("ETH-USDT"));
        let mut invalid_override = request();
        invalid_override.factor_instances.clear();
        let mut failed_cross_market_protocol = ValidationProtocol {
            protocol_id: String::new(),
            user_id: "alice".into(),
            run: request(),
            windows: vec![],
            walk_forward: None,
            cross_market: Some(CrossMarketValidationRequest {
                contexts: vec![
                    CrossMarketValidationContextRequest {
                        snapshot_id: snapshot.snapshot_id.clone(),
                        run_override: None,
                    },
                    CrossMarketValidationContextRequest {
                        snapshot_id: eth_snapshot.snapshot_id.clone(),
                        run_override: Some(invalid_override),
                    },
                ],
            }),
            method_version: "cross-market@1".into(),
            aggregation_rule_version: "equal-window@1".into(),
        };
        failed_cross_market_protocol.protocol_id =
            content_id(&failed_cross_market_protocol).unwrap();
        state.save_protocol(&failed_cross_market_protocol).unwrap();
        let failed_cross_market_report = run_validation_report(
            &ValidationProtocolIdRequest {
                user_id: "alice".into(),
                protocol_id: failed_cross_market_protocol.protocol_id,
            },
            &state,
        )
        .unwrap();
        assert_eq!(failed_cross_market_report.aggregate.failed_windows, 1);
        assert!(failed_cross_market_report.cross_market[1].failure.is_some());
        assert_eq!(failed_cross_market_report.recommended_contexts.len(), 1);
        assert!(
            validate_protocol(
                &ValidationProtocolCreateRequest {
                    cross_market: Some(CrossMarketValidationRequest {
                        contexts: vec![
                            CrossMarketValidationContextRequest {
                                snapshot_id: snapshot.snapshot_id.clone(),
                                run_override: None,
                            },
                            CrossMarketValidationContextRequest {
                                snapshot_id: "missing-snapshot".into(),
                                run_override: None,
                            },
                        ],
                    }),
                    ..cross_market_request.clone()
                },
                &state
            )
            .is_err()
        );
        assert!(
            validate_protocol(
                &ValidationProtocolCreateRequest {
                    cross_market: Some(CrossMarketValidationRequest {
                        contexts: vec![
                            CrossMarketValidationContextRequest {
                                snapshot_id: snapshot.snapshot_id.clone(),
                                run_override: None,
                            },
                            CrossMarketValidationContextRequest {
                                snapshot_id: snapshot.snapshot_id.clone(),
                                run_override: None,
                            },
                        ],
                    }),
                    ..cross_market_request.clone()
                },
                &state
            )
            .is_err()
        );
        let incompatible_snapshot = state
            .persist_snapshot_for_user(
                "alice",
                &adaq_data_core::BarSeries {
                    src: "okx".into(),
                    code: "SOL-USDT".into(),
                    interval: BarInterval::OneDay,
                    bars: source_bars,
                    gaps: vec![],
                },
            )
            .unwrap();
        assert!(
            validate_protocol(
                &ValidationProtocolCreateRequest {
                    cross_market: Some(CrossMarketValidationRequest {
                        contexts: vec![
                            CrossMarketValidationContextRequest {
                                snapshot_id: snapshot.snapshot_id.clone(),
                                run_override: None,
                            },
                            CrossMarketValidationContextRequest {
                                snapshot_id: incompatible_snapshot.snapshot_id,
                                run_override: None,
                            },
                        ],
                    }),
                    ..cross_market_request
                },
                &state
            )
            .is_err()
        );

        let mut legacy_json =
            serde_json::to_value(state.load_run("alice", &first.run_id).unwrap()).unwrap();
        legacy_json.as_object_mut().unwrap().remove("provenance");
        state
            .database
            .lock()
            .unwrap()
            .execute(
                "UPDATE backtest_runs SET result_json = ?1 WHERE run_id = ?2",
                params![legacy_json.to_string(), first.run_id],
            )
            .unwrap();
        assert!(
            state
                .load_run("alice", &first.run_id)
                .unwrap()
                .provenance
                .is_none()
        );

        drop(state);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn snapshots_are_user_scoped_and_listed_by_matching_coverage() {
        let root = std::env::temp_dir().join(format!(
            "adaq-snapshot-access-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let state = M3State::open(&root).unwrap();
        let series = |open_time_ms| adaq_data_core::BarSeries {
            src: "okx".into(),
            code: "BTC-USDT".into(),
            interval: BarInterval::OneHour,
            bars: vec![OhlcvBar {
                open_time_ms,
                open: 1.into(),
                high: 1.into(),
                low: 1.into(),
                close: 1.into(),
                base_volume: 1.into(),
                quote_volume: 1.into(),
            }],
            gaps: vec![],
        };
        let later = state
            .persist_snapshot_for_user("alice", &series(3_600_000))
            .unwrap();
        let earlier = state
            .persist_snapshot_for_user("alice", &series(0))
            .unwrap();
        let mut expected = vec![earlier.snapshot_id.clone(), later.snapshot_id.clone()];
        for hour in 2..12 {
            expected.push(
                state
                    .persist_snapshot_for_user("alice", &series(hour * 3_600_000))
                    .unwrap()
                    .snapshot_id,
            );
        }
        state
            .persist_snapshot_for_user("bob", &series(12 * 3_600_000))
            .unwrap();

        let listed = state
            .list_snapshots(&SnapshotListRequest {
                user_id: "alice".into(),
                src: "okx".into(),
                code: "BTC-USDT".into(),
                interval: BarInterval::OneHour,
                page: 1,
            })
            .unwrap();
        assert_eq!(
            listed
                .items
                .iter()
                .map(|snapshot| &snapshot.snapshot_id)
                .collect::<Vec<_>>(),
            expected[..10].iter().collect::<Vec<_>>()
        );
        assert_eq!(listed.total, 12);
        let second = state
            .list_snapshots(&SnapshotListRequest {
                user_id: "alice".into(),
                src: "okx".into(),
                code: "BTC-USDT".into(),
                interval: BarInterval::OneHour,
                page: 2,
            })
            .unwrap();
        assert_eq!(second.items.len(), 2);
        assert_eq!(
            second
                .items
                .iter()
                .map(|snapshot| &snapshot.snapshot_id)
                .collect::<Vec<_>>(),
            expected[10..].iter().collect::<Vec<_>>()
        );
        assert!(
            state
                .snapshot_for_user("bob", &earlier.snapshot_id)
                .is_err()
        );
        assert_eq!(
            validate_snapshot_request("alice", "okx", "BTC-USDT", 1, 1).unwrap_err(),
            "Snapshot time range is invalid"
        );

        drop(state);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn readable_snapshots_are_user_scoped_and_ordered_for_cross_market_selection() {
        let root = std::env::temp_dir().join(format!(
            "adaq-cross-market-snapshots-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let state = M3State::open(&root).unwrap();
        let series = |code: &str, open_time_ms| adaq_data_core::BarSeries {
            src: "okx".into(),
            code: code.into(),
            interval: BarInterval::OneHour,
            bars: vec![OhlcvBar {
                open_time_ms,
                open: 1.into(),
                high: 1.into(),
                low: 1.into(),
                close: 1.into(),
                base_volume: 1.into(),
                quote_volume: 1.into(),
            }],
            gaps: vec![],
        };
        state
            .persist_snapshot_for_user("alice", &series("ETH-USDT", 3_600_000))
            .unwrap();
        state
            .persist_snapshot_for_user("alice", &series("BTC-USDT", 0))
            .unwrap();
        state
            .persist_snapshot_for_user("bob", &series("SOL-USDT", 0))
            .unwrap();

        assert_eq!(
            state
                .list_readable_snapshots("alice")
                .unwrap()
                .iter()
                .map(|snapshot| snapshot.code.as_str())
                .collect::<Vec<_>>(),
            vec!["BTC-USDT", "ETH-USDT"]
        );
        assert!(state.list_readable_snapshots(" ").is_err());

        drop(state);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cross_market_evidence_preserves_order_failures_dispersion_and_report_identity() {
        let snapshot = |id: &str, code: &str| MarketDataSnapshot {
            snapshot_id: id.into(),
            src: "okx".into(),
            code: code.into(),
            interval: BarInterval::OneHour,
            start_time_ms: 0,
            end_time_ms: 3_600_000,
            bar_count: 1,
            gaps: vec![],
            parquet_path: PathBuf::new(),
        };
        let run = |snapshot_id: &str| BacktestRunRequest {
            user_id: "alice".into(),
            snapshot_id: snapshot_id.into(),
            run_start_time_ms: None,
            run_end_time_ms: None,
            factor_instances: vec![],
            signal_instances: vec![],
            strategy_archive_sha256: "strategy".into(),
            strategy_parameters: HashMap::new(),
            initial_quote_allocation: 1.into(),
            execution_profile: ExecutionProfile {
                maker_fee_rate: rust_decimal::Decimal::ZERO,
                taker_fee_rate: rust_decimal::Decimal::ZERO,
                adverse_slippage_rate: rust_decimal::Decimal::ZERO,
                rebalance_threshold: rust_decimal::Decimal::ZERO,
                price_increment: rust_decimal::Decimal::ONE,
                quantity_increment: rust_decimal::Decimal::ONE,
                minimum_quantity: rust_decimal::Decimal::ZERO,
                risk_free_rate: rust_decimal::Decimal::ZERO,
                fill_policy: adaq_backtest_core::FillPolicy::Taker,
            },
            seed: 0,
        };
        let metrics = |total_return| adaq_backtest_core::BacktestMetrics {
            initial_equity: 1.into(),
            final_equity: 1.into(),
            total_return,
            cagr: 0.into(),
            annualized_volatility: 0.into(),
            sharpe: 0.into(),
            sortino: 0.into(),
            max_drawdown: 0.into(),
            calmar: 0.into(),
            realized_pnl: 0.into(),
            unrealized_pnl: 0.into(),
            total_fees: 0.into(),
            turnover: 0.into(),
            fill_count: 0,
            realized_trade_count: 0,
            win_rate: 0.into(),
            profit_factor: 0.into(),
            average_win: 0.into(),
            average_loss: 0.into(),
            exposure_time: 0.into(),
            benchmark_return: 0.into(),
            excess_return: 0.into(),
        };
        let contexts = vec![
            CrossMarketValidationReport {
                snapshot: snapshot("btc", "BTC-USDT"),
                run: run("btc"),
                run_id: Some("run-btc".into()),
                metrics: Some(metrics(rust_decimal::Decimal::new(20, 2))),
                pauses: vec![],
                failure: None,
            },
            CrossMarketValidationReport {
                snapshot: snapshot("eth", "ETH-USDT"),
                run: run("eth"),
                run_id: None,
                metrics: None,
                pauses: vec![],
                failure: Some("missing-input".into()),
            },
            CrossMarketValidationReport {
                snapshot: snapshot("sol", "SOL-USDT"),
                run: run("sol"),
                run_id: Some("run-sol".into()),
                metrics: Some(metrics(rust_decimal::Decimal::new(-10, 2))),
                pauses: vec![],
                failure: None,
            },
        ];
        let aggregate = aggregate_cross_market(&contexts);
        assert_eq!(aggregate.completed_windows, 2);
        assert_eq!(aggregate.failed_windows, 1);
        assert_eq!(
            cross_market_evidence(&contexts)
                .unwrap()
                .total_return_spread,
            rust_decimal::Decimal::new(30, 2)
        );

        let report = ValidationReport {
            report_id: String::new(),
            protocol_id: "protocol".into(),
            user_id: "alice".into(),
            method_version: "cross-market@1".into(),
            aggregation_rule_version: "equal-window@1".into(),
            walk_forward: None,
            cross_market: contexts.clone(),
            windows: vec![],
            aggregate,
            recommended_contexts: vec![RecommendedContext {
                supporting_report_id: String::new(),
                snapshot: contexts[0].snapshot.clone(),
                run: contexts[0].run.clone(),
            }],
            cross_market_evidence: cross_market_evidence(&contexts),
        };
        let identity = validation_report_id(&report).unwrap();
        let exported = serde_json::to_value(&report).unwrap();
        assert_eq!(
            exported["recommendedContexts"][0]["supportingReportId"],
            serde_json::Value::String(String::new())
        );
        assert!(validation_markdown(&report).contains("crossMarketEvidence"));
        assert!(validation_markdown(&report).contains("recommendedContexts"));
        let mut reordered = report;
        reordered.cross_market.reverse();
        assert_ne!(identity, validation_report_id(&reordered).unwrap());
    }
}
