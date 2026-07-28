use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use ada_backtest_core::{
    ExecutionProfile, MarketDataSnapshot, SnapshotStore, SpotSimulator,
    TargetDecision as SimulationDecision,
};
use ada_data_core::{BarGap, BarInterval, HistoricalBarRange, OhlcvBar, OkxClient};
use adaq_component_tooling::{
    ComponentDependency, ComponentKind, ComponentPackage, EngineIdentity, ParameterDefinition,
    RunLimits, component_parameters, validate_and_freeze, verify_package,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::run_engine::{PositionMode, RunEngine, RunRequest, market_engine_build_id};

pub struct M3State {
    root: PathBuf,
    snapshots: SnapshotStore,
    database: Mutex<Connection>,
    downloads: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryComponent {
    component_id: String,
    version: String,
    sdk_version: String,
    name: String,
    kind: String,
    archive_sha256: String,
    wasm_sha256: String,
    parameters: Vec<ParameterDefinition>,
    dependencies: Vec<ComponentDependency>,
}

impl M3State {
    pub fn open(app_data: &Path) -> Result<Self, String> {
        let root = app_data.join("m3");
        fs::create_dir_all(root.join("components")).map_err(string)?;
        let snapshots = SnapshotStore::new(root.join("market-data")).map_err(string)?;
        let database = Connection::open(app_data.join("adaq.sqlite3")).map_err(string)?;
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
             );",
            )
            .map_err(string)?;
        Ok(Self {
            root,
            snapshots,
            database: Mutex::new(database),
            downloads: Mutex::new(HashMap::new()),
        })
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
            sdk_version,
            name: package.manifest.name,
            kind,
            archive_sha256: package.archive_sha256,
            wasm_sha256: package.manifest.wasm_sha256,
            parameters: package.manifest.parameters,
            dependencies: package.manifest.dependencies,
        })
    }

    pub fn list_components(&self, user_id: &str) -> Result<Vec<LibraryComponent>, String> {
        validate_user(user_id)?;
        let database = self.database.lock().map_err(string)?;
        let mut statement = database
            .prepare(
                "SELECT c.component_id, c.version, c.name, c.kind, c.archive_sha256, c.wasm_sha256, c.archive_path
             FROM component_content c JOIN component_access a USING(archive_sha256)
             WHERE a.user_id = ?1 ORDER BY c.name, c.version",
            )
            .map_err(string)?;
        statement
            .query_map([user_id], |row| {
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
                    let package =
                        ComponentPackage::read(&fs::read(path).map_err(string)?).map_err(string)?;
                    Ok(LibraryComponent {
                        component_id,
                        version,
                        sdk_version: package.manifest.sdk_version.to_string(),
                        name,
                        kind,
                        archive_sha256,
                        wasm_sha256,
                        parameters: package.manifest.parameters,
                        dependencies: package.manifest.dependencies,
                    })
                },
            )
            .collect()
    }

    pub fn delete_component(&self, user_id: &str, hash: &str) -> Result<(), String> {
        validate_user(user_id)?;
        let mut database = self.database.lock().map_err(string)?;
        let referenced: i64 = database
            .query_row(
                "SELECT COUNT(*) FROM backtest_run_components rc
             JOIN backtest_runs r USING(run_id)
             WHERE r.user_id = ?1 AND rc.archive_sha256 = ?2",
                params![user_id, hash],
                |row| row.get(0),
            )
            .map_err(string)?;
        if referenced > 0 {
            return Err("Component Package is locked by a Backtest Run".into());
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
        series: &ada_data_core::BarSeries,
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

    fn list_snapshots(
        &self,
        request: &SnapshotListRequest,
    ) -> Result<Vec<MarketDataSnapshot>, String> {
        if request.src.trim().is_empty() || request.code.trim().is_empty() {
            return Err("Snapshot coverage request is invalid".into());
        }
        let interval = serde_json::to_string(&request.interval).map_err(string)?;
        let database = self.database.lock().map_err(string)?;
        let mut statement = database
            .prepare(
                "SELECT metadata_json FROM market_data_snapshots
             WHERE src = ?1 AND code = ?2 AND interval = ?3 ORDER BY start_time_ms",
            )
            .map_err(string)?;
        statement
            .query_map(params![request.src, request.code, interval], |row| {
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

    fn package_for_user(&self, user_id: &str, hash: &str) -> Result<ComponentPackage, String> {
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

    fn snapshot(&self, snapshot_id: &str) -> Result<(MarketDataSnapshot, Vec<OhlcvBar>), String> {
        let database = self.database.lock().map_err(string)?;
        let json: String = database
            .query_row(
                "SELECT metadata_json FROM market_data_snapshots WHERE snapshot_id = ?1",
                [snapshot_id],
                |row| row.get(0),
            )
            .map_err(|_| "Market Data Snapshot was not found".to_owned())?;
        drop(database);
        let snapshot: MarketDataSnapshot = serde_json::from_str(&json).map_err(string)?;
        let bars = self.snapshots.read(&snapshot).map_err(string)?;
        Ok((snapshot, bars))
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
        serde_json::from_str(&json).map_err(string)
    }

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

    fn delete_run(&self, user_id: &str, run_id: &str) -> Result<(), String> {
        validate_user(user_id)?;
        let changed = self
            .database
            .lock()
            .map_err(string)?
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
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentUserRequest {
    pub user_id: String,
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
    pub src: String,
    pub code: String,
    pub interval: BarInterval,
    pub start_time_ms: i64,
    pub end_time_ms: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotDownloadRequest {
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
    pub src: String,
    pub code: String,
    pub interval: BarInterval,
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

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestRunRequest {
    pub user_id: String,
    pub snapshot_id: String,
    #[serde(default)]
    pub factor_instances: Vec<FactorInstanceRequest>,
    pub strategy_archive_sha256: String,
    #[serde(default)]
    pub strategy_parameters: HashMap<String, String>,
    #[serde(with = "rust_decimal::serde::str")]
    pub initial_quote_allocation: rust_decimal::Decimal,
    pub execution_profile: ExecutionProfile,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FactorInstanceRequest {
    pub alias: String,
    pub archive_sha256: String,
    #[serde(default)]
    pub parameters: HashMap<String, String>,
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
    pub result: ada_backtest_core::SimulationResult,
    pub component_lock: Vec<ComponentLockEntry>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestRunView {
    pub run_id: String,
    pub plan_hash: String,
    pub snapshot: MarketDataSnapshot,
    pub bars: Vec<OhlcvBar>,
    pub result: ada_backtest_core::SimulationResult,
    pub component_lock: Vec<ComponentLockEntry>,
}

#[derive(Clone, Serialize, Deserialize)]
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
    pub orders: Vec<ada_backtest_core::SimulatedOrder>,
    pub fills: Vec<ada_backtest_core::Fill>,
    pub total_orders: usize,
    pub total_fills: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestRunSummary {
    pub run_id: String,
    pub created_at: String,
    pub code: String,
    pub interval: BarInterval,
    pub bar_count: usize,
    #[serde(with = "rust_decimal::serde::str")]
    pub total_return: rust_decimal::Decimal,
}

#[tauri::command]
pub fn component_import(
    request: ComponentImportRequest,
    state: tauri::State<'_, M3State>,
) -> Result<LibraryComponent, String> {
    state.import_component(&request.user_id, &request.bytes)
}

#[tauri::command]
pub fn component_list(
    request: ComponentUserRequest,
    state: tauri::State<'_, M3State>,
) -> Result<Vec<LibraryComponent>, String> {
    state.list_components(&request.user_id)
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
    state.persist_snapshot(&series)
}

#[tauri::command]
pub async fn snapshot_download(
    request: SnapshotDownloadRequest,
    on_event: tauri::ipc::Channel<SnapshotDownloadEvent>,
    client: tauri::State<'_, OkxClient>,
    state: tauri::State<'_, M3State>,
) -> Result<MarketDataSnapshot, String> {
    if request.src != "okx" || request.task_id.trim().is_empty() {
        return Err("Snapshot download request is invalid".into());
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    state
        .downloads
        .lock()
        .map_err(string)?
        .insert(request.task_id.clone(), cancelled.clone());
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
            let snapshot = state.persist_snapshot(&series)?;
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
pub fn snapshot_list(
    request: SnapshotListRequest,
    state: tauri::State<'_, M3State>,
) -> Result<Vec<MarketDataSnapshot>, String> {
    state.list_snapshots(&request)
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

fn execute_backtest(
    request: BacktestRunRequest,
    state: &M3State,
) -> Result<BacktestRunView, String> {
    if !request.factor_instances.is_empty() {
        return Err("External Feature Slots are unsupported until M5.5".into());
    }
    let strategy = state.package_for_user(&request.user_id, &request.strategy_archive_sha256)?;
    if !matches!(strategy.manifest.kind, ComponentKind::Strategy) {
        return Err("Backtest requires a Strategy Component".into());
    }
    let plan = validate_and_freeze(
        &strategy.manifest,
        &strategy.archive_sha256,
        &EngineIdentity {
            engine_build_id: market_engine_build_id(),
        },
    )
    .map_err(|error| format!("Indicator Plan validation failed: {:?}", error.issues))?;
    let run_id = fingerprint(&request, plan.plan_hash())?;
    if let Ok(existing) = state.load_run(&request.user_id, &run_id) {
        return Ok(run_view(&existing, i64::MIN, i64::MAX, 2_000));
    }
    let (snapshot, bars) = state.snapshot(&request.snapshot_id)?;
    let strategy_path = state.runtime_component(&strategy)?;
    let gaps = snapshot
        .gaps
        .iter()
        .map(|gap| BarGap {
            start_time_ms: gap.start_time_ms,
            end_time_ms: gap.end_time_ms,
        })
        .collect::<Vec<_>>();
    let strategy_parameters =
        component_parameters(&strategy.manifest, Some(&request.strategy_parameters))?;
    let strategy_path = strategy_path.to_string_lossy();
    let engine_result = RunEngine::execute(&RunRequest {
        strategy_path: &strategy_path,
        strategy_parameters: &strategy_parameters,
        bars: &bars,
        gaps: &gaps,
        plan: &plan,
        position_mode: PositionMode::LongOnly,
        limits: RunLimits::default(),
    })?;
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
        result,
        component_lock: std::iter::once(&strategy)
            .map(|package| ComponentLockEntry {
                component_id: package.manifest.component_id.to_string(),
                version: package.manifest.version.to_string(),
                archive_sha256: package.archive_sha256.clone(),
                wasm_sha256: package.manifest.wasm_sha256.clone(),
            })
            .collect(),
    };
    state.save_run(&request.user_id, &run_id, &run)?;
    Ok(run_view(&run, i64::MIN, i64::MAX, 2_000))
}

#[tauri::command]
pub fn backtest_list(
    request: ComponentUserRequest,
    state: tauri::State<'_, M3State>,
) -> Result<Vec<BacktestRunSummary>, String> {
    state.list_runs(&request.user_id)
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
    Ok(BacktestExecutionPage {
        orders: run
            .result
            .orders
            .iter()
            .skip(request.offset)
            .take(request.limit)
            .cloned()
            .collect(),
        fills: run
            .result
            .fills
            .iter()
            .skip(request.offset)
            .take(request.limit)
            .cloned()
            .collect(),
        total_orders: run.result.orders.len(),
        total_fills: run.result.fills.len(),
    })
}

#[tauri::command]
pub fn backtest_delete(
    request: BacktestRunIdRequest,
    state: tauri::State<'_, M3State>,
) -> Result<(), String> {
    state.delete_run(&request.user_id, &request.run_id)
}

fn fingerprint(request: &BacktestRunRequest, plan_hash: &str) -> Result<String, String> {
    let digest = Sha256::digest(
        [
            serde_json::to_vec(request).map_err(string)?,
            plan_hash.as_bytes().to_vec(),
        ]
        .concat(),
    );
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
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
        result,
        component_lock: run.component_lock.clone(),
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
    points: &[ada_backtest_core::EquityPoint],
    start: i64,
    end: i64,
    max_points: usize,
) -> Vec<ada_backtest_core::EquityPoint> {
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

fn validate_user(user_id: &str) -> Result<(), String> {
    if user_id.trim().is_empty() || user_id.len() > 128 {
        Err("User ID is invalid".into())
    } else {
        Ok(())
    }
}

fn string(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use adaq_component_tooling::{ComponentManifest, pack_component};
    use std::time::{SystemTime, UNIX_EPOCH};

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
        state.import_component("alice", &bytes).unwrap();
        assert_eq!(state.list_components("alice").unwrap().len(), 1);
        assert!(state.list_components("bob").unwrap().is_empty());

        let mut conflicting = factor;
        conflicting.name = "Conflicting Package".into();
        let bytes = pack_component(conflicting, &wasm).unwrap();
        assert!(state.import_component("alice", &bytes).is_err());

        let (strategy, wasm) = fixture("strategy");
        let bytes = pack_component(strategy, &wasm).unwrap();
        let strategy_entry = state.import_component("alice", &bytes).unwrap();
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
            .persist_snapshot(&ada_data_core::BarSeries {
                src: "okx".into(),
                code: "BTC-USDT".into(),
                interval: BarInterval::OneHour,
                bars: vec![bar(0, 100), bar(3_600_000, 101), bar(7_200_000, 102)],
                gaps: vec![],
            })
            .unwrap();
        let request = || BacktestRunRequest {
            user_id: "alice".into(),
            snapshot_id: snapshot.snapshot_id.clone(),
            factor_instances: vec![],
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
                fill_policy: ada_backtest_core::FillPolicy::Taker,
            },
        };
        let first = execute_backtest(request(), &state).unwrap();
        let second = execute_backtest(request(), &state).unwrap();
        assert!(!first.plan_hash.is_empty());
        assert_ne!(
            fingerprint(&request(), &"a".repeat(64)).unwrap(),
            fingerprint(&request(), &"b".repeat(64)).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&first).unwrap(),
            serde_json::to_value(&second).unwrap()
        );
        assert_eq!(state.list_runs("alice").unwrap().len(), 1);
        assert!(
            state
                .delete_component("alice", &strategy_entry.archive_sha256)
                .is_err()
        );
        state.delete_run("alice", &first.run_id).unwrap();
        state
            .delete_component("alice", &strategy_entry.archive_sha256)
            .unwrap();
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }
}
