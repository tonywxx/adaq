//! Dataset Generation lifecycle module.
//!
//! One deep, Tauri-independent module owning Dataset Generation Attempt
//! identity, legal state transitions, diagnostics, progress, native Model
//! Dataset generation orchestration, cancellation, startup recovery, retry,
//! and publication ordering. The external interface is limited to starting,
//! retrying, cancelling, listing, and stopping work for one User; startup
//! recovery, schema handling, the attempt store, and the native runner stay
//! private to this module. Signal Dataset storage (archive, query,
//! evaluation, Backtest consumption) remains a separate responsibility: this
//! module only controls when publication occurs and interprets the result.

mod runner;
mod store;

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::{
    backtest::{ComponentPackageSource, SnapshotReadSource},
    user::validate_user,
};

const INCOMPATIBLE_SCHEMA: &str = "Incompatible pre-v1 Dataset Generation schema. Close AdaQ, remove its device-local app data directory, and reopen AdaQ. This deletes all Local Research Data for every User on this device.";
const MAX_DIAGNOSTIC_EVIDENCE_CHARS: usize = 8_192;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum AttemptStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl TryFrom<&str> for AttemptStatus {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "pending" => Ok(Self::Pending),
            "running" => Ok(Self::Running),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(format!(
                "unknown Dataset Generation Attempt status: {value}"
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum DiagnosticCode {
    GenerationInterrupted,
    GenerationFailed,
    PublicationFailed,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Diagnostic {
    code: DiagnosticCode,
    details: String,
}

impl Diagnostic {
    fn generation_interrupted(previous: Option<String>) -> Self {
        let mut details = "application stopped before completion".to_owned();
        if let Some(previous) = previous {
            let previous = serde_json::from_str::<Self>(&previous)
                .map(Self::evidence)
                .unwrap_or(previous);
            details.push_str("; previous diagnostic: ");
            details.push_str(&previous);
        }
        Self {
            code: DiagnosticCode::GenerationInterrupted,
            details,
        }
    }

    pub(crate) fn generation_failed(details: impl Into<String>) -> Self {
        Self::bounded(DiagnosticCode::GenerationFailed, details)
    }

    pub(crate) fn publication_failed(details: impl Into<String>) -> Self {
        Self::bounded(DiagnosticCode::PublicationFailed, details)
    }

    fn bounded(code: DiagnosticCode, details: impl Into<String>) -> Self {
        let available =
            MAX_DIAGNOSTIC_EVIDENCE_CHARS.saturating_sub(code.persisted().chars().count() + 2);
        Self {
            code,
            details: details.into().chars().take(available).collect(),
        }
    }

    fn evidence(self) -> String {
        format!("{}: {}", self.code.persisted(), self.details)
    }
}

impl DiagnosticCode {
    fn persisted(self) -> &'static str {
        match self {
            Self::GenerationInterrupted => "generation-interrupted",
            Self::GenerationFailed => "generation-failed",
            Self::PublicationFailed => "publication-failed",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Attempt {
    pub(crate) attempt_id: String,
    pub(crate) dataset_id: Option<String>,
    pub(crate) status: AttemptStatus,
    pub(crate) diagnostic_evidence: Option<String>,
    pub(crate) progress_completed: i64,
    pub(crate) progress_total: i64,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DatasetGenerationRequest {
    pub user_id: String,
    pub snapshot_id: String,
    pub model_archive_sha256: String,
    #[serde(default)]
    pub model_parameters: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub factor_instances: Vec<DatasetFactorInstance>,
    #[serde(default)]
    pub seed: u64,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DatasetFactorInstance {
    pub alias: String,
    pub archive_sha256: String,
    #[serde(default)]
    pub parameters: std::collections::HashMap<String, String>,
}

/// The concrete local dependencies composed into Dataset Generation. The
/// complete Local Research state is never passed in; only the database,
/// Component Package access and Market Data Snapshot access (through the
/// Backtest module's Source traits), and the Signal Dataset directory are
/// required.
pub(crate) trait GenerationSource:
    SnapshotReadSource + ComponentPackageSource + Send + Sync
{
    fn database(&self) -> Result<MutexGuard<'_, Connection>, String>;
    fn dataset_directory(&self) -> Result<PathBuf, String>;
}

pub(super) struct GenerationInner {
    source: Arc<dyn GenerationSource>,
    attempts: Mutex<HashMap<String, Arc<AtomicBool>>>,
    reset_blocks: Mutex<HashSet<String>>,
    reset_wait_timeout: Mutex<Duration>,
}

/// The Dataset Generation lifecycle interface: start, retry, cancel, list,
/// and stopping all work for one User.
#[derive(Clone)]
pub(crate) struct DatasetGeneration(Arc<GenerationInner>);

impl DatasetGeneration {
    /// Creates the module and performs its internal startup work: Attempt
    /// schema initialization, exact schema compatibility validation, and
    /// recovery of Attempts interrupted by an application restart.
    pub(crate) fn open(source: Arc<dyn GenerationSource>) -> Result<Self, String> {
        let generation = Self(Arc::new(GenerationInner {
            source,
            attempts: Mutex::new(HashMap::new()),
            reset_blocks: Mutex::new(HashSet::new()),
            reset_wait_timeout: Mutex::new(Duration::from_secs(60)),
        }));
        let database = generation.0.source.database()?;
        let initialized = store::AttemptStore::new(&database).initialize();
        drop(database);
        initialized?;
        Ok(generation)
    }

    /// Starts generation for a canonical User-scoped request identity,
    /// reusing a matching Pending, Running, or Completed Attempt, and runs
    /// any newly accepted Attempt in the background.
    pub(crate) fn start(&self, request: DatasetGenerationRequest) -> Result<Attempt, String> {
        let started = runner::start(&self.0, &request)?;
        let attempt = started.attempt.clone();
        self.spawn(started);
        Ok(attempt)
    }

    /// Retries the owning User's Failed or Cancelled Attempt with a new
    /// Attempt identity and runs it in the background.
    pub(crate) fn retry(&self, attempt_id: &str, user_id: &str) -> Result<Attempt, String> {
        let started = runner::retry(&self.0, attempt_id, user_id)?;
        let attempt = started.attempt.clone();
        self.spawn(started);
        Ok(attempt)
    }

    /// Requests cancellation. The Attempt becomes Cancelled only after its
    /// task has actually stopped; repeated requests are idempotent.
    pub(crate) fn cancel(&self, attempt_id: &str, user_id: &str) -> Result<(), String> {
        validate_user(user_id)?;
        let database = self.0.source.database()?;
        if !store::AttemptStore::new(&database).request_cancellation(attempt_id, user_id)? {
            return Err("Dataset Generation Attempt cannot be cancelled".into());
        }
        drop(database);
        if let Some(cancelled) = self.0.attempts.lock().map_err(string)?.get(attempt_id) {
            cancelled.store(true, Ordering::Relaxed);
        }
        Ok(())
    }

    pub(crate) fn list(&self, user_id: &str) -> Result<Vec<Attempt>, String> {
        validate_user(user_id)?;
        let database = self.0.source.database()?;
        store::AttemptStore::new(&database).list(user_id)
    }

    /// Lifecycle barrier for a User-scoped Reset All: blocks new Start and
    /// Retry for one User, cancels that User's active Attempts, and waits
    /// for all of them to exit without holding the SQLite mutex. Returns a
    /// guard that keeps the User's start-restriction in place until the
    /// caller's reset work is finished.
    pub(crate) fn stop_all_for_user<'a>(
        &'a self,
        user_id: &str,
    ) -> Result<UserResetBlock<'a>, String> {
        runner::stop_all_for_user(&self.0, user_id)
    }

    #[cfg(test)]
    pub(crate) fn set_reset_wait_timeout(&self, timeout: Duration) {
        *self.0.reset_wait_timeout.lock().unwrap() = timeout;
    }

    fn spawn(&self, started: runner::StartedGeneration) {
        let runner::StartedGeneration {
            attempt,
            cancelled,
            request,
        } = started;
        let Some(cancelled) = cancelled else {
            return;
        };
        let inner = self.0.clone();
        let attempt_id = attempt.attempt_id.clone();
        std::thread::spawn(move || {
            if let Err(error) = runner::run_started(&inner, &request, &cancelled, &attempt_id) {
                let _ = runner::record_publication_failure(&inner, &attempt_id, &error);
                eprintln!("Dataset Generation Attempt {attempt_id} finalization failed: {error}");
            }
            if let Ok(mut attempts) = inner.attempts.lock() {
                attempts.remove(&attempt_id);
            }
        });
    }
}

/// RAII guard holding one User's Dataset Generation start-restriction; Drop
/// always releases it (success, failure, and panic paths).
pub(crate) struct UserResetBlock<'a> {
    inner: &'a GenerationInner,
    user_id: String,
}

impl Drop for UserResetBlock<'_> {
    fn drop(&mut self) {
        if let Ok(mut blocks) = self.inner.reset_blocks.lock() {
            blocks.remove(&self.user_id);
        }
    }
}

impl UserResetBlock<'_> {
    /// Removes this User's stopped Attempt evidence inside the caller's
    /// reset transaction. Only valid after the barrier has fully stopped
    /// this User's generation work.
    pub(crate) fn delete_attempt_evidence(&self, database: &Connection) -> Result<(), String> {
        store::AttemptStore::new(database).delete_for_user(&self.user_id)
    }
}

fn string(error: impl ToString) -> String {
    error.to_string()
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::local_research::{LocalDataResetKind, LocalResearchState};
    use adaq_component_tooling::{ComponentManifest, ComponentPackage, pack_component};
    use adaq_data_core::{BarGap, BarInterval, BarSeries, OhlcvBar};
    use rust_decimal::Decimal;
    use std::{collections::HashMap, fs, time::Instant};

    pub(super) fn root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "adaq-dataset-generation-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ))
    }

    pub(super) fn model_package() -> Vec<u8> {
        let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/model/target/wasm32-unknown-unknown/debug/m8_model_fixture.wasm");
        assert!(
            fixture.is_file(),
            "build the model fixture with cargo component build"
        );
        let wasm = fs::read(fixture).unwrap();
        let mut manifest: ComponentManifest =
            serde_json::from_str(include_str!("../../fixtures/model/manifest.json")).unwrap();
        let wasm_sha256 = crate::forecast_signal_dataset::hash(&wasm);
        manifest.wasm_sha256 = wasm_sha256.clone();
        manifest.model_artifact.as_mut().unwrap().sha256 = wasm_sha256;
        pack_component(manifest, &wasm).unwrap()
    }

    pub(super) fn setup(
        mode: &str,
        name: &str,
    ) -> (PathBuf, Arc<LocalResearchState>, DatasetGenerationRequest) {
        let root = root(name);
        let state = LocalResearchState::open(&root).unwrap();
        let package = model_package();
        let model_archive_sha256 = ComponentPackage::read(&package).unwrap().archive_sha256;
        state.components.import("alice", &package).unwrap();
        let bars = [0, 1, 2, 6, 7, 8]
            .into_iter()
            .enumerate()
            .map(|(index, hour)| {
                let value = Decimal::from(i64::try_from(index + 1).unwrap());
                OhlcvBar {
                    open_time_ms: hour * 3_600_000,
                    open: value,
                    high: value,
                    low: value,
                    close: value,
                    base_volume: Decimal::ONE,
                    quote_volume: value,
                }
            })
            .collect();
        let snapshot = state
            .persist_snapshot_for_user(
                "alice",
                &BarSeries {
                    src: "okx".into(),
                    code: "BTC-USDT".into(),
                    interval: BarInterval::OneHour,
                    bars,
                    gaps: vec![BarGap {
                        start_time_ms: 3 * 3_600_000,
                        end_time_ms: 6 * 3_600_000,
                    }],
                },
            )
            .unwrap();
        (
            root,
            state,
            DatasetGenerationRequest {
                user_id: "alice".into(),
                snapshot_id: snapshot.snapshot_id,
                model_archive_sha256,
                model_parameters: HashMap::from([("mode".into(), mode.into())]),
                factor_instances: vec![],
                seed: 7,
            },
        )
    }

    pub(super) fn wait_for_attempt(
        state: &LocalResearchState,
        user_id: &str,
        attempt_id: &str,
        expected: AttemptStatus,
    ) -> Attempt {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let attempt = state
                .generation
                .list(user_id)
                .unwrap()
                .into_iter()
                .find(|attempt| attempt.attempt_id == attempt_id)
                .unwrap();
            if attempt.status == expected {
                return attempt;
            }
            assert!(
                !matches!(
                    attempt.status,
                    AttemptStatus::Completed | AttemptStatus::Failed | AttemptStatus::Cancelled
                ),
                "Attempt {attempt_id} reached {:?} before {expected:?}",
                attempt.status
            );
            assert!(
                Instant::now() < deadline,
                "Attempt {attempt_id} did not reach {expected:?}: {:?}",
                attempt.status
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Seeds a Running Attempt with a registered cancellation flag so
    /// deterministic race tests control when the runner executes.
    pub(super) fn seed_running_attempt(
        state: &LocalResearchState,
        user_id: &str,
        request_hash: &str,
    ) -> (String, Arc<AtomicBool>) {
        let attempt_id = format!("seeded-{request_hash}");
        state
            .database
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO dataset_generation_attempts
                 (attempt_id, request_hash, user_id, status, request_json)
                 VALUES (?1, ?2, ?3, 'running', '{}')",
                rusqlite::params![attempt_id, request_hash, user_id],
            )
            .unwrap();
        let cancelled = Arc::new(AtomicBool::new(false));
        state
            .generation
            .0
            .attempts
            .lock()
            .unwrap()
            .insert(attempt_id.clone(), cancelled.clone());
        (attempt_id, cancelled)
    }

    fn signal_dataset_content_count(state: &LocalResearchState) -> i64 {
        state
            .database
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM signal_dataset_content", [], |row| {
                row.get(0)
            })
            .unwrap()
    }

    #[test]
    fn interface_start_publishes_a_completed_attempt() {
        let (root, state, request) = setup("valid", "interface-vertical");
        let started = state.generation.start(request).unwrap();
        assert_eq!(started.status, AttemptStatus::Pending);
        let attempt = wait_for_attempt(
            &state,
            "alice",
            &started.attempt_id,
            AttemptStatus::Completed,
        );
        assert_eq!(attempt.progress_total, 6);
        assert_eq!(attempt.progress_completed, attempt.progress_total);
        assert!(attempt.dataset_id.is_some());
        assert_eq!(signal_dataset_content_count(&state), 1);
        let access: i64 = state
            .database
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM signal_dataset_access WHERE user_id = 'alice'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(access, 1);
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn interface_start_suppresses_duplicates_and_restarts_after_failure() {
        let (root, state, request) = setup("non-finite", "interface-duplicates");
        let first = state.generation.start(request.clone()).unwrap();
        let duplicate = state.generation.start(request.clone()).unwrap();
        assert_eq!(duplicate.attempt_id, first.attempt_id);
        wait_for_attempt(&state, "alice", &first.attempt_id, AttemptStatus::Failed);
        let restarted = state.generation.start(request).unwrap();
        assert_ne!(restarted.attempt_id, first.attempt_id);
        wait_for_attempt(
            &state,
            "alice",
            &restarted.attempt_id,
            AttemptStatus::Failed,
        );
        assert_eq!(state.generation.list("alice").unwrap().len(), 2);
        assert_eq!(signal_dataset_content_count(&state), 0);
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn interface_retry_is_user_scoped_and_reuses_completed_dataset() {
        let (root, state, request) = setup("valid", "interface-retry");
        let started = state.generation.start(request).unwrap();
        state
            .generation
            .cancel(&started.attempt_id, "alice")
            .unwrap();
        wait_for_attempt(
            &state,
            "alice",
            &started.attempt_id,
            AttemptStatus::Cancelled,
        );
        assert_eq!(
            state
                .generation
                .retry(&started.attempt_id, "bob")
                .unwrap_err(),
            "Dataset Generation Attempt cannot be retried"
        );
        let retried = state
            .generation
            .retry(&started.attempt_id, "alice")
            .unwrap();
        assert_ne!(retried.attempt_id, started.attempt_id);
        let completed = wait_for_attempt(
            &state,
            "alice",
            &retried.attempt_id,
            AttemptStatus::Completed,
        );
        let dataset_id = completed.dataset_id.clone().unwrap();
        let reused = state
            .generation
            .retry(&started.attempt_id, "alice")
            .unwrap();
        assert_ne!(reused.attempt_id, retried.attempt_id);
        let reused = wait_for_attempt(
            &state,
            "alice",
            &reused.attempt_id,
            AttemptStatus::Completed,
        );
        assert_eq!(reused.dataset_id.as_deref(), Some(dataset_id.as_str()));
        assert_eq!(
            (reused.progress_completed, reused.progress_total),
            (completed.progress_completed, completed.progress_total)
        );
        let original = state
            .generation
            .list("alice")
            .unwrap()
            .into_iter()
            .find(|attempt| attempt.attempt_id == started.attempt_id)
            .unwrap();
        assert_eq!(original.status, AttemptStatus::Cancelled);
        assert_eq!(signal_dataset_content_count(&state), 1);
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn interface_cancellation_is_user_scoped_and_terminal_after_exit() {
        let (root, state, request) = setup("valid", "interface-cancel");
        let started = state.generation.start(request).unwrap();
        assert_eq!(
            state
                .generation
                .cancel(&started.attempt_id, "bob")
                .unwrap_err(),
            "Dataset Generation Attempt cannot be cancelled"
        );
        state
            .generation
            .cancel(&started.attempt_id, "alice")
            .unwrap();
        state
            .generation
            .cancel(&started.attempt_id, "alice")
            .unwrap();
        let attempt = wait_for_attempt(
            &state,
            "alice",
            &started.attempt_id,
            AttemptStatus::Cancelled,
        );
        assert!(attempt.dataset_id.is_none());
        assert_eq!(signal_dataset_content_count(&state), 0);
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn interface_list_is_user_scoped() {
        let (root, state, request) = setup("valid", "interface-list");
        let started = state.generation.start(request).unwrap();
        wait_for_attempt(
            &state,
            "alice",
            &started.attempt_id,
            AttemptStatus::Completed,
        );
        assert!(state.generation.list("bob").unwrap().is_empty());
        assert_eq!(state.generation.list("alice").unwrap().len(), 1);
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reset_barrier_blocks_start_and_retry_until_released() {
        let (root, state, request) = setup("valid", "interface-reset-block");
        let block = state.generation.stop_all_for_user("alice").unwrap();
        let blocked = state.generation.start(request.clone()).unwrap_err();
        assert!(
            blocked.contains("Reset All is in progress"),
            "start must be blocked: {blocked}"
        );
        let blocked = state
            .generation
            .retry("missing-attempt", "alice")
            .unwrap_err();
        assert!(
            blocked.contains("Reset All is in progress"),
            "retry must be blocked: {blocked}"
        );
        drop(block);
        let started = state.generation.start(request).unwrap();
        wait_for_attempt(
            &state,
            "alice",
            &started.attempt_id,
            AttemptStatus::Completed,
        );
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cancellation_during_generation_stays_running_until_the_task_exits() {
        let (root, state, request) = setup("valid", "interface-cancel-running");
        let (attempt_id, cancelled) = seed_running_attempt(&state, "alice", "cancel-running");
        let go = Arc::new(AtomicBool::new(false));
        let task_generation = state.generation.clone();
        let task_request = request.clone();
        let task_attempt_id = attempt_id.clone();
        let task_go = go.clone();
        let task = std::thread::spawn(move || {
            let result = runner::run_attempt_with_lifecycle_checkpoint(
                &task_generation.0,
                &task_request,
                &cancelled,
                &task_attempt_id,
                |checkpoint| {
                    if checkpoint == runner::LifecycleCheckpoint::BeforePublication {
                        while !task_go.load(Ordering::Relaxed) {
                            std::thread::sleep(Duration::from_millis(1));
                        }
                    }
                },
            );
            task_generation
                .0
                .attempts
                .lock()
                .unwrap()
                .remove(&task_attempt_id);
            result
        });
        std::thread::sleep(Duration::from_millis(50));
        state.generation.cancel(&attempt_id, "alice").unwrap();
        assert_eq!(
            state.generation.list("alice").unwrap()[0].status,
            AttemptStatus::Running,
            "a Running Attempt stays Running until its task exits"
        );
        go.store(true, Ordering::Relaxed);
        task.join().unwrap().unwrap();
        let attempt = wait_for_attempt(&state, "alice", &attempt_id, AttemptStatus::Cancelled);
        assert!(attempt.dataset_id.is_none());
        assert_eq!(signal_dataset_content_count(&state), 0);
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cancellation_at_lifecycle_checkpoints_cleans_temporary_output() {
        for (name, checkpoint) in [
            (
                "cancel-after-generation",
                runner::LifecycleCheckpoint::AfterGeneration,
            ),
            (
                "cancel-before-publication",
                runner::LifecycleCheckpoint::BeforePublication,
            ),
        ] {
            let (root, state, request) = setup("valid", name);
            let (attempt_id, cancelled) = seed_running_attempt(&state, "alice", name);
            let generation = state.generation.clone();
            let task = std::thread::spawn({
                let attempt_id = attempt_id.clone();
                let request = request.clone();
                move || {
                    let result = runner::run_attempt_with_lifecycle_checkpoint(
                        &generation.0,
                        &request,
                        &cancelled,
                        &attempt_id,
                        |observed| {
                            if observed == checkpoint {
                                generation.cancel(&attempt_id, "alice").unwrap();
                            }
                        },
                    );
                    generation.0.attempts.lock().unwrap().remove(&attempt_id);
                    result
                }
            });
            task.join().unwrap().unwrap();
            let attempt = wait_for_attempt(&state, "alice", &attempt_id, AttemptStatus::Cancelled);
            assert!(attempt.dataset_id.is_none());
            assert!(
                fs::read_dir(state.root.join("signal-datasets"))
                    .unwrap()
                    .next()
                    .is_none()
            );
            drop(state);
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn reset_all_waits_for_in_flight_attempt_before_deleting() {
        let (root, state, request) = setup("valid", "interface-reset-wait");
        let watchlist = crate::watchlist::WatchlistDb::open(&root.join("adaq.db")).unwrap();
        let (attempt_id, cancelled) = seed_running_attempt(&state, "alice", "reset-wait");
        let state = Arc::new(state);
        let at_publication = Arc::new(AtomicBool::new(false));
        let go = Arc::new(AtomicBool::new(false));
        let task_state = state.clone();
        let task_attempt_id = attempt_id.clone();
        let task_cancelled = cancelled.clone();
        let task_at_publication = at_publication.clone();
        let task_go = go.clone();
        let task_request = request;
        let task = std::thread::spawn(move || {
            let result = runner::run_attempt_with_lifecycle_checkpoint(
                &task_state.generation.0,
                &task_request,
                &task_cancelled,
                &task_attempt_id,
                |checkpoint| {
                    if checkpoint == runner::LifecycleCheckpoint::BeforePublication {
                        task_at_publication.store(true, Ordering::SeqCst);
                        while !task_go.load(Ordering::SeqCst) {
                            std::thread::sleep(Duration::from_millis(1));
                        }
                    }
                },
            );
            task_state
                .generation
                .0
                .attempts
                .lock()
                .unwrap()
                .remove(&task_attempt_id);
            result
        });
        while !at_publication.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(1));
        }
        let reset_state = state.clone();
        let reset = std::thread::spawn(move || {
            reset_state.reset_local_data("alice", LocalDataResetKind::All)
        });
        while !cancelled.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(1));
        }
        go.store(true, Ordering::SeqCst);
        assert!(
            task.join().unwrap().is_ok(),
            "the cancelled attempt must exit cleanly"
        );
        assert!(reset.join().unwrap().is_ok());
        assert!(state.generation.0.reset_blocks.lock().unwrap().is_empty());
        let access: i64 = state
            .database
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM signal_dataset_access", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(access, 0);
        let leftover_temps: Vec<_> = fs::read_dir(root.join("signal-datasets"))
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|entry| {
                        entry
                            .file_name()
                            .to_string_lossy()
                            .ends_with(".parquet.tmp")
                    })
                    .collect()
            })
            .unwrap_or_default();
        assert!(
            leftover_temps.is_empty(),
            "no temporary output may survive the reset: {leftover_temps:?}"
        );
        drop(watchlist);
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reset_all_fails_before_deletion_when_attempt_cannot_stop() {
        let (root, state, request) = setup("valid", "interface-reset-stuck");
        let published = state.generation.start(request.clone()).unwrap();
        let published = wait_for_attempt(
            &state,
            "alice",
            &published.attempt_id,
            AttemptStatus::Completed,
        );
        let dataset_path: String = state
            .database
            .lock()
            .unwrap()
            .query_row(
                "SELECT parquet_path FROM signal_dataset_content WHERE dataset_id = ?1",
                [published.dataset_id.as_deref().unwrap()],
                |row| row.get(0),
            )
            .unwrap();
        let (_stuck_id, _stuck_cancelled) = seed_running_attempt(&state, "alice", "reset-stuck");
        state
            .generation
            .set_reset_wait_timeout(Duration::from_millis(100));
        let err = state
            .reset_local_data("alice", LocalDataResetKind::All)
            .unwrap_err();
        assert!(err.contains("could not stop"), "{err}");
        assert!(
            std::path::Path::new(&dataset_path).exists(),
            "reset must not delete data on failure"
        );
        let access: i64 = state
            .database
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM signal_dataset_access", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(access, 1);
        assert!(
            state.generation.0.reset_blocks.lock().unwrap().is_empty(),
            "the start restriction must be released on failure"
        );
        let restarted = state.generation.start(request).unwrap();
        assert_eq!(
            restarted.attempt_id, published.attempt_id,
            "start must be allowed again after the failed reset"
        );
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reset_all_does_not_disturb_another_users_generation() {
        let (root, state, request) = setup("valid", "interface-reset-isolation");
        let watchlist = crate::watchlist::WatchlistDb::open(&root.join("adaq.db")).unwrap();
        let package = model_package();
        state.components.import("bob", &package).unwrap();
        state
            .grant_snapshot_for_user("bob", &request.snapshot_id)
            .unwrap();
        let bob_request = DatasetGenerationRequest {
            user_id: "bob".into(),
            ..request.clone()
        };
        let (bob_attempt_id, bob_cancelled) =
            seed_running_attempt(&state, "bob", "reset-isolation");
        let state = Arc::new(state);
        let at_publication = Arc::new(AtomicBool::new(false));
        let go = Arc::new(AtomicBool::new(false));
        let task_state = state.clone();
        let task_cancelled = bob_cancelled.clone();
        let task_at_publication = at_publication.clone();
        let task_go = go.clone();
        let task_attempt_id = bob_attempt_id.clone();
        let task_request = bob_request;
        let task = std::thread::spawn(move || {
            let result = runner::run_attempt_with_lifecycle_checkpoint(
                &task_state.generation.0,
                &task_request,
                &task_cancelled,
                &task_attempt_id,
                |checkpoint| {
                    if checkpoint == runner::LifecycleCheckpoint::BeforePublication {
                        task_at_publication.store(true, Ordering::SeqCst);
                        while !task_go.load(Ordering::SeqCst) {
                            std::thread::sleep(Duration::from_millis(1));
                        }
                    }
                },
            );
            task_state
                .generation
                .0
                .attempts
                .lock()
                .unwrap()
                .remove(&bob_attempt_id);
            result
        });
        while !at_publication.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(1));
        }
        state
            .reset_local_data("alice", LocalDataResetKind::All)
            .unwrap();
        assert!(
            !bob_cancelled.load(Ordering::Relaxed),
            "bob's attempt must not be cancelled by alice's reset"
        );
        assert!(state.generation.0.reset_blocks.lock().unwrap().is_empty());
        go.store(true, Ordering::SeqCst);
        assert!(task.join().unwrap().is_ok(), "bob's attempt must publish");
        let database = state.database.lock().unwrap();
        let bob_access: i64 = database
            .query_row(
                "SELECT COUNT(*) FROM signal_dataset_access WHERE user_id = 'bob'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let alice_access: i64 = database
            .query_row(
                "SELECT COUNT(*) FROM signal_dataset_access WHERE user_id = 'alice'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(bob_access, 1);
        assert_eq!(alice_access, 0);
        drop(database);
        drop(watchlist);
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }
}
