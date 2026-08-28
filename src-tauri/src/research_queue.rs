use std::{
    collections::BTreeMap,
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
    time::Duration,
};

use rusqlite::{Connection, OptionalExtension, params};

const STATUS_ADMITTED: &str = "admitted";
const STATUS_TOMBSTONE: &str = "tombstone";
const RETRY_BACKOFF: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum WorkKind {
    FeatureFitting,
    FeatureMaterialization,
    Factor,
    Python,
}

impl WorkKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::FeatureFitting => "feature-fitting",
            Self::FeatureMaterialization => "feature-materialization",
            Self::Factor => "factor",
            Self::Python => "python",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "feature-fitting" => Ok(Self::FeatureFitting),
            "feature-materialization" => Ok(Self::FeatureMaterialization),
            "factor" => Ok(Self::Factor),
            "python" => Ok(Self::Python),
            _ => Err(format!("unknown Research Queue work kind: {value}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QueueAdmission {
    pub(crate) user_id: String,
    pub(crate) attempt_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QueueTicket {
    pub(crate) sequence: i64,
    pub(crate) kind: WorkKind,
    pub(crate) user_id: String,
    pub(crate) attempt_id: String,
}

pub(crate) enum QueueRunResult {
    Consumed,
    Stale,
    Retryable(String),
}

pub(crate) trait ResearchQueueAdapter: Send + Sync {
    fn pending_attempts(&self) -> Result<Vec<QueueAdmission>, String>;
    fn execute(&self, ticket: QueueTicket) -> QueueRunResult;
    fn request_shutdown(&self);
}

pub(crate) type QueueAdmitter =
    Arc<dyn Fn(WorkKind, &str, &str) -> Result<(), String> + Send + Sync>;
pub(crate) type QueueWaker = Arc<dyn Fn() + Send + Sync>;

struct QueueState {
    signaled: bool,
    shutdown: bool,
}

struct QueueInner {
    database: Arc<Mutex<Connection>>,
    adapters: Mutex<BTreeMap<WorkKind, Arc<dyn ResearchQueueAdapter>>>,
    state: Mutex<QueueState>,
    changed: Condvar,
    worker: Mutex<Option<JoinHandle<()>>>,
}

#[derive(Clone)]
pub(crate) struct ResearchQueue {
    inner: Arc<QueueInner>,
}

impl ResearchQueue {
    pub(crate) fn open(database: Arc<Mutex<Connection>>) -> Result<Self, String> {
        initialize(&database)?;
        let inner = Arc::new(QueueInner {
            database,
            adapters: Mutex::new(BTreeMap::new()),
            state: Mutex::new(QueueState {
                signaled: true,
                shutdown: false,
            }),
            changed: Condvar::new(),
            worker: Mutex::new(None),
        });
        let worker_inner = inner.clone();
        let worker = thread::Builder::new()
            .name("adaq-research-queue".into())
            .spawn(move || run_worker(worker_inner))
            .map_err(|error| error.to_string())?;
        *inner
            .worker
            .lock()
            .map_err(|_| "Research Queue worker lock poisoned")? = Some(worker);
        Ok(Self { inner })
    }

    pub(crate) fn attach(
        &self,
        kind: WorkKind,
        adapter: Arc<dyn ResearchQueueAdapter>,
    ) -> Result<(), String> {
        let pending = adapter.pending_attempts()?;
        self.inner
            .adapters
            .lock()
            .map_err(|_| "Research Queue adapter registry lock poisoned")?
            .insert(kind, adapter);
        for admission in pending {
            self.admit(kind, &admission.user_id, &admission.attempt_id)?;
        }
        self.wake();
        Ok(())
    }

    pub(crate) fn admit(
        &self,
        kind: WorkKind,
        user_id: &str,
        attempt_id: &str,
    ) -> Result<(), String> {
        if self
            .inner
            .state
            .lock()
            .map_err(|_| "Research Queue state lock poisoned")?
            .shutdown
        {
            return Err("Research Queue is shut down".into());
        }
        let database = self
            .inner
            .database
            .lock()
            .map_err(|_| "Research Queue database lock poisoned")?;
        database
            .execute(
                "INSERT INTO research_queue_entries(
                    work_kind, user_id, attempt_id, status, admitted_at_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(work_kind, user_id, attempt_id) DO UPDATE SET status = excluded.status
                  WHERE research_queue_entries.status = ?6",
                params![
                    kind.as_str(),
                    user_id,
                    attempt_id,
                    STATUS_ADMITTED,
                    unix_now_ms(),
                    STATUS_TOMBSTONE
                ],
            )
            .map_err(|error| format!("Research Queue admission failed: {error}"))?;
        drop(database);
        self.wake();
        Ok(())
    }

    pub(crate) fn wake(&self) {
        if let Ok(mut state) = self.inner.state.lock() {
            state.signaled = true;
            self.inner.changed.notify_one();
        }
    }

    pub(crate) fn admitter(&self) -> QueueAdmitter {
        let weak = Arc::downgrade(&self.inner);
        Arc::new(move |kind, user_id, attempt_id| {
            let inner = weak
                .upgrade()
                .ok_or_else(|| "Research Queue is unavailable".to_owned())?;
            ResearchQueue { inner }.admit(kind, user_id, attempt_id)
        })
    }

    pub(crate) fn waker(&self) -> QueueWaker {
        let weak = Arc::downgrade(&self.inner);
        Arc::new(move || {
            if let Some(inner) = weak.upgrade() {
                ResearchQueue { inner }.wake();
            }
        })
    }

    pub(crate) fn shutdown(&self) {
        let should_shutdown = if let Ok(mut state) = self.inner.state.lock() {
            if state.shutdown {
                false
            } else {
                state.shutdown = true;
                self.inner.changed.notify_one();
                true
            }
        } else {
            false
        };
        if !should_shutdown {
            return;
        }
        let adapters = self
            .inner
            .adapters
            .lock()
            .map(|adapters| adapters.values().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        for adapter in adapters {
            adapter.request_shutdown();
        }
        if let Ok(mut worker) = self.inner.worker.lock()
            && let Some(worker) = worker.take()
        {
            let _ = worker.join();
        }
    }
}

impl Drop for ResearchQueue {
    fn drop(&mut self) {
        if Arc::strong_count(&self.inner) == 2 {
            self.shutdown();
        }
    }
}

fn initialize(database: &Arc<Mutex<Connection>>) -> Result<(), String> {
    let database = database
        .lock()
        .map_err(|_| "Research Queue database lock poisoned")?;
    database
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS research_queue_entries(
                sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                work_kind TEXT NOT NULL,
                user_id TEXT NOT NULL,
                attempt_id TEXT NOT NULL,
                status TEXT NOT NULL CHECK(status IN ('admitted', 'tombstone')),
                admitted_at_ms INTEGER NOT NULL,
                UNIQUE(work_kind, user_id, attempt_id)
            );
            CREATE INDEX IF NOT EXISTS research_queue_entries_ready
                ON research_queue_entries(status, sequence);",
        )
        .map_err(|error| format!("Research Queue schema initialization failed: {error}"))?;
    let mut statement = database
        .prepare("PRAGMA table_info(research_queue_entries)")
        .map_err(|error| format!("Research Queue schema inspection failed: {error}"))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| format!("Research Queue schema inspection failed: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Research Queue schema inspection failed: {error}"))?;
    for required in [
        "sequence",
        "work_kind",
        "user_id",
        "attempt_id",
        "status",
        "admitted_at_ms",
    ] {
        if !columns.iter().any(|column| column == required) {
            return Err(format!(
                "reset-required: incompatible Research Queue journal; missing column {required}"
            ));
        }
    }
    Ok(())
}

fn run_worker(inner: Arc<QueueInner>) {
    loop {
        if !wait_for_signal(&inner) {
            return;
        }
        loop {
            if is_shutdown(&inner) {
                return;
            }
            let ticket = match next_ticket(&inner) {
                Ok(Some(ticket)) => ticket,
                Ok(None) => break,
                Err(error) => {
                    eprintln!("Research Queue scheduling failed: {error}");
                    retry_later(&inner);
                    break;
                }
            };
            match dispatch(&inner, ticket) {
                QueueRunResult::Consumed | QueueRunResult::Stale => {}
                QueueRunResult::Retryable(error) => {
                    eprintln!("Research Queue adapter deferred work: {error}");
                    retry_later(&inner);
                    break;
                }
            }
        }
    }
}

fn wait_for_signal(inner: &QueueInner) -> bool {
    let Ok(mut state) = inner.state.lock() else {
        return false;
    };
    while !state.signaled && !state.shutdown {
        state = match inner.changed.wait(state) {
            Ok(state) => state,
            Err(_) => return false,
        };
    }
    if state.shutdown {
        return false;
    }
    state.signaled = false;
    true
}

fn is_shutdown(inner: &QueueInner) -> bool {
    inner
        .state
        .lock()
        .map(|state| state.shutdown)
        .unwrap_or(true)
}

fn retry_later(inner: &QueueInner) {
    // ponytail: fixed backoff bounds transient adapter/schema failures; add per-ticket scheduling only if this becomes noisy.
    thread::sleep(RETRY_BACKOFF);
    if let Ok(mut state) = inner.state.lock()
        && !state.shutdown
    {
        state.signaled = true;
        inner.changed.notify_one();
    }
}

fn next_ticket(inner: &QueueInner) -> Result<Option<QueueTicket>, String> {
    let database = inner
        .database
        .lock()
        .map_err(|_| "Research Queue database lock poisoned")?;
    database
        .query_row(
            "SELECT sequence, work_kind, user_id, attempt_id
               FROM research_queue_entries
              WHERE status = ?1
              ORDER BY sequence LIMIT 1",
            [STATUS_ADMITTED],
            |row| {
                Ok(QueueTicket {
                    sequence: row.get(0)?,
                    kind: WorkKind::parse(&row.get::<_, String>(1)?).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            1,
                            rusqlite::types::Type::Text,
                            Box::new(std::io::Error::other(error)),
                        )
                    })?,
                    user_id: row.get(2)?,
                    attempt_id: row.get(3)?,
                })
            },
        )
        .optional()
        .map_err(|error| format!("Research Queue ticket read failed: {error}"))
}

fn dispatch(inner: &QueueInner, ticket: QueueTicket) -> QueueRunResult {
    let adapter = inner
        .adapters
        .lock()
        .ok()
        .and_then(|adapters| adapters.get(&ticket.kind).cloned());
    match adapter {
        Some(adapter) => {
            let sequence = ticket.sequence;
            let result = adapter.execute(ticket);
            if matches!(&result, QueueRunResult::Consumed | QueueRunResult::Stale) {
                if let Err(error) = tombstone_sequence(inner, sequence) {
                    return QueueRunResult::Retryable(error);
                }
            }
            result
        }
        None => QueueRunResult::Retryable(format!("adapter-not-attached:{}", ticket.kind.as_str())),
    }
}

fn tombstone_sequence(inner: &QueueInner, sequence: i64) -> Result<(), String> {
    let database = inner
        .database
        .lock()
        .map_err(|_| "Research Queue database lock poisoned")?;
    database
        .execute(
            "UPDATE research_queue_entries SET status = ?2
              WHERE sequence = ?1 AND status = ?3",
            params![sequence, STATUS_TOMBSTONE, STATUS_ADMITTED],
        )
        .map(|_| ())
        .map_err(|error| format!("Research Queue tombstone write failed: {error}"))
}

fn unix_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TestAdapter {
        pending: Mutex<Vec<QueueAdmission>>,
        executed: Arc<Mutex<Vec<String>>>,
        calls: AtomicUsize,
    }

    impl ResearchQueueAdapter for TestAdapter {
        fn pending_attempts(&self) -> Result<Vec<QueueAdmission>, String> {
            Ok(self.pending.lock().unwrap().clone())
        }

        fn execute(&self, ticket: QueueTicket) -> QueueRunResult {
            self.calls.fetch_add(1, Ordering::Relaxed);
            self.executed.lock().unwrap().push(ticket.attempt_id);
            QueueRunResult::Consumed
        }

        fn request_shutdown(&self) {}
    }

    fn database() -> Arc<Mutex<Connection>> {
        Arc::new(Mutex::new(Connection::open_in_memory().unwrap()))
    }

    #[test]
    fn admission_is_idempotent_and_global_fifo() {
        let queue = ResearchQueue::open(database()).unwrap();
        let executed = Arc::new(Mutex::new(Vec::new()));
        let adapter = Arc::new(TestAdapter {
            pending: Mutex::new(Vec::new()),
            executed: executed.clone(),
            calls: AtomicUsize::new(0),
        });
        queue
            .attach(WorkKind::FeatureFitting, adapter.clone())
            .unwrap();
        queue.attach(WorkKind::Python, adapter.clone()).unwrap();
        queue
            .admit(WorkKind::FeatureFitting, "alice", "first")
            .unwrap();
        queue.admit(WorkKind::Python, "alice", "second").unwrap();
        queue
            .admit(WorkKind::FeatureFitting, "alice", "first")
            .unwrap();

        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while executed.lock().unwrap().len() < 2 && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            executed.lock().unwrap().as_slice(),
            ["first".to_owned(), "second".to_owned()]
        );
        assert_eq!(adapter.calls.load(Ordering::Relaxed), 2);
        queue.shutdown();
    }

    #[test]
    fn incompatible_journal_fails_closed_without_resetting_data() {
        let database = database();
        database
            .lock()
            .unwrap()
            .execute_batch(
                "CREATE TABLE research_queue_entries(
                    sequence INTEGER PRIMARY KEY,
                    status TEXT NOT NULL
                );
                 INSERT INTO research_queue_entries(sequence, status)
                 VALUES (7, 'admitted');",
            )
            .unwrap();

        let error = match ResearchQueue::open(database.clone()) {
            Ok(_) => panic!("incompatible journal must fail closed"),
            Err(error) => error,
        };

        assert!(error.starts_with("reset-required: incompatible Research Queue journal"));
        assert_eq!(
            database
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM research_queue_entries", [], |row| row
                    .get::<_, i64>(0),)
                .unwrap(),
            1
        );
    }
}
