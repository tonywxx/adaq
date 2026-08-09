//! Market Data Snapshot module.
//!
//! One deep, Tauri-independent module owning the Market Data Snapshot
//! lifecycle: Snapshot creation, download with progress and cancellation,
//! paged and readable listing, persistence, and entitlement-scoped reads.
//! The external interface is limited to the five command operations, the
//! entitlement-scoped read and persist hooks other modules consume, and the
//! summary-for-user and reset-for-user hooks the composition root calls.
//! All Snapshot schema handling and SQL, the in-flight download map and its
//! cancellation flags, and the orphaned Parquet discovery on reset stay
//! private to this module.

#[cfg(test)]
mod tests;

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
};

use adaq_backtest_core::{
    MarketDataSnapshot, MarketDataUniverseSnapshot, SnapshotProvenance, SnapshotStore,
};
use adaq_data_core::{BarSeries, HistoricalBarRange, OhlcvBar, OkxClient};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

use crate::user::validate_user;

const SNAPSHOT_PAGE_SIZE: usize = 10;

/// The concrete local dependencies composed into Market Data Snapshots. The
/// complete Local Research state is never passed in; only database access
/// and the Parquet Snapshot store are shared.
pub(crate) trait SnapshotSource: Send + Sync {
    fn database(&self) -> Result<MutexGuard<'_, Connection>, String>;
    fn store(&self) -> &SnapshotStore;
}

/// The local Snapshot source: the shared SQLite database plus the Parquet
/// Snapshot store directory.
pub(crate) struct LocalSnapshotSource {
    database: Arc<Mutex<Connection>>,
    store: Arc<SnapshotStore>,
}

impl LocalSnapshotSource {
    pub(crate) fn new(database: Arc<Mutex<Connection>>, store: Arc<SnapshotStore>) -> Self {
        Self { database, store }
    }
}

impl SnapshotSource for LocalSnapshotSource {
    fn database(&self) -> Result<MutexGuard<'_, Connection>, String> {
        self.database.lock().map_err(string)
    }

    fn store(&self) -> &SnapshotStore {
        &self.store
    }
}

/// The Snapshot evidence counts and footprint the Local Data summary
/// reports for one User.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SnapshotSummary {
    pub snapshot_count: u64,
    pub market_data_bytes: u64,
}

struct SnapshotInner {
    source: Arc<dyn SnapshotSource>,
    downloads: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

/// The Market Data Snapshot interface: create, download, cancel, list, and
/// readable-list operations, the entitlement-scoped read and persist hooks,
/// plus the summary-for-user and reset-for-user hooks the composition root
/// calls.
#[derive(Clone)]
pub(crate) struct MarketDataSnapshots(Arc<SnapshotInner>);

impl MarketDataSnapshots {
    /// Creates the module and initializes the Market Data Snapshot schema,
    /// which lives inside this module.
    pub(crate) fn open(source: Arc<dyn SnapshotSource>) -> Result<Self, String> {
        source
            .database()?
            .execute_batch(
                "PRAGMA foreign_keys = ON;
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
             CREATE TABLE IF NOT EXISTS market_data_universe_snapshots (
                snapshot_id TEXT PRIMARY KEY,
                venue TEXT NOT NULL,
                interval TEXT NOT NULL,
                start_time_ms INTEGER NOT NULL,
                end_time_ms INTEGER NOT NULL,
                content_sha256 TEXT NOT NULL,
                metadata_json TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS market_data_universe_snapshot_access (
                user_id TEXT NOT NULL,
                snapshot_id TEXT NOT NULL,
                PRIMARY KEY(user_id, snapshot_id),
                FOREIGN KEY(snapshot_id) REFERENCES market_data_universe_snapshots(snapshot_id)
             );",
            )
            .map_err(string)?;
        Ok(Self(Arc::new(SnapshotInner {
            source,
            downloads: Mutex::new(HashMap::new()),
        })))
    }

    /// Fetches a new Snapshot for one User from the market data source and
    /// persists it.
    pub(crate) async fn create_for_user(
        &self,
        request: &SnapshotCreateRequest,
        client: &OkxClient,
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
        self.persist_for_user(&request.user_id, &series)
    }

    /// Downloads a new Snapshot for one User with progress events, guarded
    /// by the module-private in-flight download map and cancellation flags.
    pub(crate) async fn download_for_user(
        &self,
        request: &SnapshotDownloadRequest,
        client: &OkxClient,
        on_event: impl Fn(SnapshotDownloadEvent),
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
            let mut downloads = self.0.downloads.lock().map_err(string)?;
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
                        on_event(SnapshotDownloadEvent::Progress {
                            downloaded_bars,
                            oldest_time_ms,
                        });
                    }
                    active
                },
            )
            .await;
        self.0
            .downloads
            .lock()
            .map_err(string)?
            .remove(&request.task_id);
        match result {
            Ok(series) => {
                let snapshot = self.persist_for_user(&request.user_id, &series)?;
                on_event(SnapshotDownloadEvent::Completed {
                    snapshot_id: snapshot.snapshot_id.clone(),
                    bar_count: snapshot.bar_count,
                });
                Ok(snapshot)
            }
            Err(error) if error.code == "cancelled" => {
                on_event(SnapshotDownloadEvent::Cancelled);
                Err(error.to_string())
            }
            Err(error) => Err(error.to_string()),
        }
    }

    /// Signals one in-flight download to stop; unknown task IDs are
    /// ignored.
    pub(crate) fn cancel_download(&self, task_id: &str) -> Result<(), String> {
        if let Some(cancelled) = self.0.downloads.lock().map_err(string)?.get(task_id) {
            cancelled.store(true, Ordering::Relaxed);
        }
        Ok(())
    }

    /// Pages the Snapshots one User can read for one instrument and
    /// interval.
    pub(crate) fn list(&self, request: &SnapshotListRequest) -> Result<SnapshotPage, String> {
        validate_user(&request.user_id)?;
        if request.src.trim().is_empty() || request.code.trim().is_empty() || request.page == 0 {
            return Err("Snapshot coverage request is invalid".into());
        }
        let interval = serde_json::to_string(&request.interval).map_err(string)?;
        let database = self.0.source.database()?;
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
                snapshot_from_row,
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

    /// Lists every Snapshot one User can read, ordered for cross-market
    /// selection.
    pub(crate) fn list_readable(&self, user_id: &str) -> Result<Vec<MarketDataSnapshot>, String> {
        validate_user(user_id)?;
        let database = self.0.source.database()?;
        let mut statement = database
            .prepare(
                "SELECT s.metadata_json FROM market_data_snapshots s
                 JOIN market_data_snapshot_access a USING(snapshot_id)
                 WHERE a.user_id = ?1
                 ORDER BY s.code, s.interval, s.start_time_ms, s.snapshot_id",
            )
            .map_err(string)?;
        statement
            .query_map([user_id], snapshot_from_row)
            .map_err(string)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(string)
    }

    /// Reads one Snapshot's metadata and Closed Bars for one User, scoped
    /// by entitlement.
    pub(crate) fn snapshot_for_user(
        &self,
        user_id: &str,
        snapshot_id: &str,
    ) -> Result<(MarketDataSnapshot, Vec<OhlcvBar>), String> {
        validate_user(user_id)?;
        let json: String = {
            let database = self.0.source.database()?;
            database
                .query_row(
                    "SELECT s.metadata_json FROM market_data_snapshots s
             JOIN market_data_snapshot_access a USING(snapshot_id)
             WHERE a.user_id = ?1 AND s.snapshot_id = ?2",
                    params![user_id, snapshot_id],
                    |row| row.get(0),
                )
                .map_err(|_| "Market Data Snapshot is not available to this User".to_owned())?
        };
        let snapshot: MarketDataSnapshot = serde_json::from_str(&json).map_err(string)?;
        let bars = self.0.source.store().read(&snapshot).map_err(string)?;
        Ok((snapshot, bars))
    }

    pub(crate) fn persist_universe_for_user(
        &self,
        user_id: &str,
        snapshot: MarketDataUniverseSnapshot,
    ) -> Result<MarketDataUniverseSnapshot, String> {
        validate_user(user_id)?;
        let snapshot = snapshot.finalize().map_err(string)?;
        let database = self.0.source.database()?;
        validate_universe_snapshot(&database, user_id, &snapshot)?;
        let metadata = serde_json::to_string(&snapshot).map_err(string)?;
        let interval = serde_json::to_string(&snapshot.interval).map_err(string)?;
        let venue = serde_json::to_string(&snapshot.venue).map_err(string)?;
        database
            .execute(
                "INSERT OR IGNORE INTO market_data_universe_snapshots
                 (snapshot_id, venue, interval, start_time_ms, end_time_ms, content_sha256, metadata_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    snapshot.snapshot_id.clone(),
                    venue,
                    interval,
                    snapshot.start_time_ms,
                    snapshot.end_time_ms,
                    snapshot.content_sha256.clone(),
                    metadata
                ],
            )
            .map_err(string)?;
        database
            .execute(
                "INSERT OR IGNORE INTO market_data_universe_snapshot_access
                 (user_id, snapshot_id) VALUES (?1, ?2)",
                params![user_id, snapshot.snapshot_id.clone()],
            )
            .map_err(string)?;
        Ok(snapshot)
    }

    pub(crate) fn universe_snapshot_for_user(
        &self,
        user_id: &str,
        snapshot_id: &str,
    ) -> Result<MarketDataUniverseSnapshot, String> {
        validate_user(user_id)?;
        let database = self.0.source.database()?;
        let metadata: String = database
            .query_row(
                "SELECT s.metadata_json FROM market_data_universe_snapshots s
                 JOIN market_data_universe_snapshot_access a USING(snapshot_id)
                 WHERE a.user_id = ?1 AND s.snapshot_id = ?2",
                params![user_id, snapshot_id],
                |row| row.get(0),
            )
            .map_err(|_| {
                "Market Data Universe Snapshot is not available to this User".to_owned()
            })?;
        let snapshot: MarketDataUniverseSnapshot =
            serde_json::from_str(&metadata).map_err(string)?;
        if snapshot.snapshot_id != snapshot_id
            || snapshot.expected_content_sha256().map_err(string)? != snapshot.content_sha256
        {
            return Err("Market Data Universe Snapshot identity is invalid".into());
        }
        validate_universe_snapshot(&database, user_id, &snapshot)?;
        Ok(snapshot)
    }

    pub(crate) fn list_universe_snapshots(
        &self,
        request: &UniverseSnapshotListRequest,
    ) -> Result<UniverseSnapshotPage, String> {
        validate_user(&request.user_id)?;
        if request.page == 0 {
            return Err("Market Data Universe Snapshot page is invalid".into());
        }
        let database = self.0.source.database()?;
        let total = database
            .query_row(
                "SELECT COUNT(*) FROM market_data_universe_snapshot_access
                 WHERE user_id = ?1",
                [&request.user_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(string)?
            .try_into()
            .map_err(|_| "Market Data Universe Snapshot count is invalid")?;
        let offset = request
            .page
            .checked_sub(1)
            .and_then(|value| value.checked_mul(SNAPSHOT_PAGE_SIZE))
            .ok_or_else(|| "Market Data Universe Snapshot page is too large".to_owned())?;
        let mut statement = database
            .prepare(
                "SELECT s.metadata_json FROM market_data_universe_snapshots s
                 JOIN market_data_universe_snapshot_access a USING(snapshot_id)
                 WHERE a.user_id = ?1
                 ORDER BY s.start_time_ms, s.snapshot_id LIMIT ?2 OFFSET ?3",
            )
            .map_err(string)?;
        let items = statement
            .query_map(
                params![request.user_id, SNAPSHOT_PAGE_SIZE as i64, offset as i64],
                |row| {
                    let metadata: String = row.get(0)?;
                    serde_json::from_str(&metadata).map_err(|error| {
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
        drop(statement);
        for snapshot in &items {
            validate_universe_snapshot(&database, &request.user_id, snapshot)?;
        }
        Ok(UniverseSnapshotPage {
            items,
            total,
            page: request.page,
            page_size: SNAPSHOT_PAGE_SIZE,
        })
    }

    /// Persists one Bar Series as a Snapshot and grants it to one User.
    pub(crate) fn persist_for_user(
        &self,
        user_id: &str,
        series: &BarSeries,
    ) -> Result<MarketDataSnapshot, String> {
        self.persist_for_user_with_provenance(user_id, series, None)
    }

    pub(crate) fn persist_for_user_with_provenance(
        &self,
        user_id: &str,
        series: &BarSeries,
        provenance: Option<SnapshotProvenance>,
    ) -> Result<MarketDataSnapshot, String> {
        validate_user(user_id)?;
        let snapshot = self
            .0
            .source
            .store()
            .persist_with_provenance(series, provenance)
            .map_err(string)?;
        let metadata = serde_json::to_string(&snapshot).map_err(string)?;
        let interval = serde_json::to_string(&snapshot.interval).map_err(string)?;
        // One lock guard for both inserts; never call another interface
        // method while the guard is held.
        let database = self.0.source.database()?;
        database
            .execute(
                "INSERT OR IGNORE INTO market_data_snapshots
             (snapshot_id, src, code, interval, start_time_ms, end_time_ms, bar_count, metadata_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![snapshot.snapshot_id, snapshot.src, snapshot.code,
                    interval, snapshot.start_time_ms,
                    snapshot.end_time_ms, snapshot.bar_count as i64, metadata],
            )
            .map_err(string)?;
        database
            .execute(
                "INSERT OR IGNORE INTO market_data_snapshot_access (user_id, snapshot_id) VALUES (?1, ?2)",
                params![user_id, snapshot.snapshot_id],
            )
            .map_err(string)?;
        Ok(snapshot)
    }

    pub(crate) fn revoke_for_user(&self, user_id: &str, snapshot_id: &str) -> Result<(), String> {
        validate_user(user_id)?;
        let database = self.0.source.database()?;
        let parquet_path = database
            .query_row(
                "SELECT metadata_json FROM market_data_snapshots
                 WHERE snapshot_id = ?1",
                [snapshot_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(string)?
            .and_then(|json| serde_json::from_str::<MarketDataSnapshot>(&json).ok())
            .map(|snapshot| snapshot.parquet_path);
        let universe_locked = {
            let mut statement = database
                .prepare("SELECT metadata_json FROM market_data_universe_snapshots")
                .map_err(string)?;
            statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(string)?
                .filter_map(Result::ok)
                .filter_map(|json| serde_json::from_str::<MarketDataUniverseSnapshot>(&json).ok())
                .any(|universe| {
                    universe
                        .components
                        .iter()
                        .any(|component| component.snapshot_id == snapshot_id)
                })
        };
        if universe_locked {
            return Err("Market Data Snapshot is locked by a Universe Snapshot".into());
        }
        let transaction = database.unchecked_transaction().map_err(string)?;
        transaction
            .execute(
                "DELETE FROM market_data_snapshot_access
                 WHERE user_id = ?1 AND snapshot_id = ?2",
                params![user_id, snapshot_id],
            )
            .map_err(string)?;
        let deleted = transaction
            .execute(
                "DELETE FROM market_data_snapshots
                 WHERE snapshot_id = ?1
                   AND NOT EXISTS(
                       SELECT 1 FROM market_data_snapshot_access
                       WHERE snapshot_id = ?1
                   )",
                [snapshot_id],
            )
            .map_err(string)?;
        transaction.commit().map_err(string)?;
        let parquet_is_shared = if let Some(path) = parquet_path.as_ref() {
            let mut statement = database
                .prepare("SELECT metadata_json FROM market_data_snapshots")
                .map_err(string)?;
            statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(string)?
                .map(|json| {
                    let json = json.map_err(string)?;
                    let snapshot: MarketDataSnapshot =
                        serde_json::from_str(&json).map_err(string)?;
                    Ok(snapshot.parquet_path == *path)
                })
                .collect::<Result<Vec<_>, String>>()?
                .into_iter()
                .any(|shared| shared)
        } else {
            false
        };
        if deleted > 0
            && !parquet_is_shared
            && let Some(path) = parquet_path
        {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.to_string()),
            }
        }
        Ok(())
    }

    /// Grants one existing Snapshot to one more User. Granting is
    /// idempotent, matching the access table's INSERT OR IGNORE contract.
    #[cfg(test)]
    pub(crate) fn grant_for_user(&self, user_id: &str, snapshot_id: &str) -> Result<(), String> {
        validate_user(user_id)?;
        let database = self.0.source.database()?;
        database
            .execute(
                "INSERT OR IGNORE INTO market_data_snapshot_access (user_id, snapshot_id) VALUES (?1, ?2)",
                params![user_id, snapshot_id],
            )
            .map_err(string)?;
        Ok(())
    }

    /// The summary hook the composition root calls: the Snapshot count and
    /// Parquet footprint for one User.
    pub(crate) fn summary_for_user(&self, user_id: &str) -> Result<SnapshotSummary, String> {
        validate_user(user_id)?;
        let database = self.0.source.database()?;
        let snapshot_count = database
            .query_row(
                "SELECT COUNT(*) FROM market_data_snapshot_access WHERE user_id = ?1",
                [user_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(string)?
            .max(0) as u64;
        let mut statement = database
            .prepare(
                "SELECT s.metadata_json FROM market_data_snapshots s
             JOIN market_data_snapshot_access a USING(snapshot_id) WHERE a.user_id = ?1",
            )
            .map_err(string)?;
        let market_data_bytes = statement
            .query_map([user_id], |row| row.get::<_, String>(0))
            .map_err(string)?
            .filter_map(|json| json.ok())
            .filter_map(|json| serde_json::from_str::<MarketDataSnapshot>(&json).ok())
            .map(|snapshot| file_bytes(&snapshot.parquet_path))
            .sum();
        Ok(SnapshotSummary {
            snapshot_count,
            market_data_bytes,
        })
    }

    /// The Parquet files that become orphaned when one User's Snapshot
    /// entitlement is reset. The composition root passes the connection it
    /// already locks, so the orphan query runs under the same lock that
    /// serializes it with persistence.
    pub(crate) fn orphaned_parquet_paths(
        &self,
        database: &Connection,
        user_id: &str,
    ) -> Result<Vec<PathBuf>, String> {
        validate_user(user_id)?;
        let mut statement = database
            .prepare(
                "SELECT s.metadata_json FROM market_data_snapshots s
         JOIN market_data_snapshot_access a USING(snapshot_id)
         WHERE a.user_id = ?1
         AND NOT EXISTS(SELECT 1 FROM market_data_snapshot_access other
             WHERE other.snapshot_id = s.snapshot_id AND other.user_id <> ?1)",
            )
            .map_err(string)?;
        statement
            .query_map([user_id], |row| row.get::<_, String>(0))
            .map_err(string)?
            .map(|json| {
                serde_json::from_str::<MarketDataSnapshot>(&json.map_err(string)?)
                    .map_err(string)
                    .map(|snapshot| snapshot.parquet_path)
            })
            .collect()
    }

    /// The reset hook the composition root calls inside its reset
    /// transaction: drops one User's Snapshot entitlement and the Snapshots
    /// nobody can read anymore.
    pub(crate) fn reset_for_user(
        &self,
        transaction: &Transaction<'_>,
        user_id: &str,
    ) -> Result<(), String> {
        transaction
            .execute(
                "DELETE FROM market_data_universe_snapshot_access WHERE user_id = ?1",
                [user_id],
            )
            .map_err(string)?;
        transaction
            .execute(
                "DELETE FROM market_data_universe_snapshots
                 WHERE NOT EXISTS(SELECT 1 FROM market_data_universe_snapshot_access a
                     WHERE a.snapshot_id = market_data_universe_snapshots.snapshot_id)",
                [],
            )
            .map_err(string)?;
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
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotCreateRequest {
    pub user_id: String,
    pub src: String,
    pub code: String,
    pub interval: adaq_data_core::BarInterval,
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
    pub interval: adaq_data_core::BarInterval,
    pub start_time_ms: i64,
    pub end_time_ms: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotListRequest {
    pub user_id: String,
    pub src: String,
    pub code: String,
    pub interval: adaq_data_core::BarInterval,
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
pub(crate) struct UniverseSnapshotRequest {
    pub user_id: String,
    pub snapshot: MarketDataUniverseSnapshot,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UniverseSnapshotListRequest {
    pub user_id: String,
    pub page: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UniverseSnapshotPage {
    pub items: Vec<MarketDataUniverseSnapshot>,
    pub total: usize,
    pub page: usize,
    pub page_size: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadableSnapshotListRequest {
    pub user_id: String,
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

fn snapshot_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MarketDataSnapshot> {
    serde_json::from_str(&row.get::<_, String>(0)?).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
    })
}

fn validate_universe_snapshot(
    database: &Connection,
    user_id: &str,
    snapshot: &MarketDataUniverseSnapshot,
) -> Result<(), String> {
    if snapshot.start_time_ms >= snapshot.end_time_ms
        || snapshot.components.is_empty()
        || snapshot.universe.instruments.len() != snapshot.components.len()
        || snapshot.universe.universe_id.trim().is_empty()
        || snapshot.universe.as_of_ms < 0
        || snapshot.quality_report_ids.is_empty()
        || snapshot.calendar_snapshot_ids.is_empty()
    {
        return Err("Market Data Universe Snapshot manifest is invalid".into());
    }
    if !matches!(
        snapshot.universe.evidence_state.as_str(),
        "observed" | "reconstructed" | "unknown"
    ) || snapshot.universe.evidence_reasons.is_empty()
    {
        return Err("Market Data Universe Snapshot evidence state is invalid".into());
    }
    if snapshot
        .universe
        .coverage_start_ms
        .is_some_and(|start| start > snapshot.universe.as_of_ms)
        || snapshot.universe.coverage_end_ms.is_some()
            && snapshot.universe.coverage_start_ms.is_none()
        || snapshot
            .universe
            .coverage_end_ms
            .zip(snapshot.universe.coverage_start_ms)
            .is_some_and(|(end, start)| end <= start)
    {
        return Err("Market Data Universe Snapshot evidence coverage is invalid".into());
    }
    let mut instruments = Vec::new();
    let mut quality_report_ids = Vec::new();
    let mut calendar_snapshot_ids = Vec::new();
    for component in &snapshot.components {
        let instrument_key =
            serde_json::to_string(&component.dataset.instrument).map_err(string)?;
        if instruments.contains(&instrument_key) {
            return Err("Market Data Universe Snapshot contains duplicate Instruments".into());
        }
        instruments.push(instrument_key);
        quality_report_ids.push(component.dataset.quality_report_id.clone());
        if component.dataset.instrument.venue != snapshot.venue {
            return Err("Market Data Universe Snapshot Venue binding is invalid".into());
        }
        if !snapshot
            .universe
            .instruments
            .iter()
            .any(|instrument| instrument == &component.dataset.instrument)
        {
            return Err(
                "Market Data Universe Snapshot membership does not match its components".into(),
            );
        }
        let metadata: String = database
            .query_row(
                "SELECT s.metadata_json FROM market_data_snapshots s
                 JOIN market_data_snapshot_access a USING(snapshot_id)
                 WHERE a.user_id = ?1 AND s.snapshot_id = ?2",
                params![user_id, component.snapshot_id],
                |row| row.get(0),
            )
            .map_err(|_| "Universe component Snapshot is not available to this User".to_owned())?;
        let component_snapshot: MarketDataSnapshot =
            serde_json::from_str(&metadata).map_err(string)?;
        if component_snapshot.src != component.dataset.instrument.venue.id
            || component_snapshot.code != component.dataset.instrument.code
            || component_snapshot.interval != snapshot.interval
            || component_snapshot.start_time_ms != snapshot.start_time_ms
            || component_snapshot.end_time_ms != snapshot.end_time_ms
        {
            return Err("Universe component Snapshot coverage does not match its manifest".into());
        }
        let Some(provenance) = component_snapshot.provenance.as_ref() else {
            return Err("Universe components require Snapshot provenance".into());
        };
        if provenance.venue != snapshot.venue
            || !provenance
                .quality_report_ids
                .contains(&component.dataset.quality_report_id)
        {
            return Err("Universe component provenance scope is invalid".into());
        }
        calendar_snapshot_ids.extend(provenance.calendar_snapshot_ids.iter().cloned());
        if !provenance
            .datasets
            .iter()
            .any(|dataset| dataset == &component.dataset)
        {
            return Err("Universe component provenance does not match its manifest".into());
        }
    }
    let mut membership = snapshot
        .universe
        .instruments
        .iter()
        .map(serde_json::to_string)
        .collect::<Result<Vec<_>, _>>()
        .map_err(string)?;
    instruments.sort_unstable();
    membership.sort_unstable();
    if instruments != membership {
        return Err("Universe membership does not exactly match its components".into());
    }
    if !snapshot
        .universe
        .instruments
        .iter()
        .all(|instrument| instrument.venue == snapshot.venue)
    {
        return Err("Universe membership contains a different Venue".into());
    }
    quality_report_ids.sort_unstable();
    quality_report_ids.dedup();
    let mut expected_quality_report_ids = snapshot.quality_report_ids.clone();
    expected_quality_report_ids.sort_unstable();
    expected_quality_report_ids.dedup();
    if quality_report_ids != expected_quality_report_ids {
        return Err("Universe quality provenance does not match its components".into());
    }
    calendar_snapshot_ids.sort_unstable();
    calendar_snapshot_ids.dedup();
    let mut expected_calendar_snapshot_ids = snapshot.calendar_snapshot_ids.clone();
    expected_calendar_snapshot_ids.sort_unstable();
    expected_calendar_snapshot_ids.dedup();
    if calendar_snapshot_ids != expected_calendar_snapshot_ids {
        return Err("Universe calendar provenance does not match its components".into());
    }
    Ok(())
}

fn file_bytes(path: impl AsRef<Path>) -> u64 {
    fs::metadata(path).map_or(0, |metadata| metadata.len())
}

fn string(error: impl std::fmt::Display) -> String {
    error.to_string()
}
