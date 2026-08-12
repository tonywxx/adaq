//! Feature lifecycle module.
//!
//! One deep, Tauri-independent module owning User-scoped Feature Definition
//! publication, Draft validation and Preview, Transformation Fitting,
//! Feature Dataset Materialization, evidence inspection, deletion locks,
//! and the one persistent device FIFO runner that executes heavy Feature
//! Attempts. The external interface is limited to typed User-scoped
//! operations; schema handling, the attempt stores, cancellation flags,
//! startup recovery, and the background worker stay private to this
//! module. Materialization Attempts and Feature Dataset storage live in
//! the engine's `FeatureMaterializationStore`; this module controls when
//! work starts, runs, cancels, and finalizes.

mod preview;
mod runner;
mod store;

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use adaq_backtest_core::MarketDataUniverseSnapshot;
use adaq_factor_research::CompletedFeatureDataset;
use adaq_feature_engine::{
    DefinitionDraft, FeatureDatasetFilter, FeatureDatasetPage, FeatureDefinition,
    FeatureMaterializationRequest, FeatureMaterializationStore, FeatureObservation,
    FeatureOutputSummary, FeaturePlan, FeaturePlanDraft, FeatureScope, MaterializationAttempt,
    TransformationFittingProtocol, TransformationFittingProtocolDraft,
};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::{backtest::SnapshotReadSource, user::validate_user};

const INCOMPATIBLE_SCHEMA: &str = "Incompatible pre-v1 Feature schema. Close AdaQ, remove its device-local app data directory, and reopen AdaQ. This deletes all Local Research Data for every User on this device.";
const MAX_DIAGNOSTIC_EVIDENCE_CHARS: usize = 8_192;
pub(crate) const MAX_PREVIEW_OBSERVATIONS: usize = 500;
pub(crate) const MAX_PREVIEW_CROSS_SECTIONAL_BATCHES: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum FeatureAttemptStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl TryFrom<&str> for FeatureAttemptStatus {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "pending" => Ok(Self::Pending),
            "running" => Ok(Self::Running),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(format!("unknown Feature Attempt status: {value}")),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DefinitionView {
    pub definition_id: String,
    pub revision: i64,
    pub definition_hash: String,
    pub definition_json: String,
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DraftValidationView {
    pub valid: bool,
    pub issues: Vec<adaq_feature_engine::ValidationIssue>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FeaturePreviewView {
    pub observations: Vec<FeatureObservation>,
    pub event_count: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FittingAttemptView {
    pub attempt_id: String,
    pub user_id: String,
    pub protocol_hash: String,
    pub plan_hash: String,
    pub status: FeatureAttemptStatus,
    pub source_attempt_id: Option<String>,
    pub artifact_id: Option<String>,
    pub failure_code: Option<String>,
    pub diagnostic: Option<String>,
    pub progress_completed: i64,
    pub progress_total: i64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ArtifactView {
    pub artifact_id: String,
    pub protocol_hash: String,
    pub artifact_json: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FeatureDatasetView {
    pub dataset_id: String,
    pub user_id: String,
    pub request_hash: String,
    pub manifest: adaq_feature_engine::FeatureDatasetManifest,
    pub content_byte_size: u64,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PlanFreezeView {
    pub plan_hash: String,
    pub plan_json: String,
}

pub(super) struct DefinitionRecord {
    pub(super) definition_hash: String,
    pub(super) definition_id: String,
    pub(super) revision: i64,
    pub(super) definition_json: String,
    pub(super) created_at_ms: i64,
    pub(super) name: String,
    pub(super) description: String,
    pub(super) tags_json: String,
}

pub(super) struct ArtifactRecord {
    pub(super) artifact_id: String,
    pub(super) protocol_hash: String,
    pub(super) artifact_json: String,
    pub(super) created_at_ms: i64,
}

pub(super) struct FittingAttemptRecord {
    pub(super) attempt_id: String,
    pub(super) user_id: String,
    pub(super) protocol_hash: String,
    pub(super) plan_hash: String,
    pub(super) plan_json: String,
    pub(super) status: FeatureAttemptStatus,
    pub(super) source_attempt_id: Option<String>,
    pub(super) artifact_id: Option<String>,
    pub(super) failure_code: Option<String>,
    pub(super) diagnostic: Option<String>,
    pub(super) progress_completed: i64,
    pub(super) progress_total: i64,
    pub(super) created_at_ms: i64,
    pub(super) updated_at_ms: i64,
}

// ---- Command request contracts ----

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DefinitionDraftRequest {
    pub user_id: String,
    pub draft: DefinitionDraft,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DefinitionPublishRequest {
    pub user_id: String,
    pub draft: DefinitionDraft,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DefinitionIdRequest {
    pub user_id: String,
    pub definition_hash: String,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FeaturePreviewRequest {
    pub user_id: String,
    pub draft: DefinitionDraft,
    #[serde(default)]
    pub snapshot_id: Option<String>,
    #[serde(default)]
    pub universe_id: Option<String>,
    #[serde(default)]
    pub valuation_currency: Option<String>,
    #[serde(default)]
    pub start_time_ms: Option<i64>,
    #[serde(default)]
    pub end_time_ms: Option<i64>,
    #[serde(default)]
    pub max_events: Option<usize>,
    #[serde(default)]
    pub artifact_ids: Vec<String>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FeatureFittingStartRequest {
    pub user_id: String,
    pub protocol: TransformationFittingProtocolDraft,
    pub plan: FeaturePlanDraft,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FeatureMaterializationStartRequest {
    pub user_id: String,
    pub request: FeatureMaterializationRequest,
    pub plan: FeaturePlanDraft,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FeatureAttemptRequest {
    pub user_id: String,
    pub attempt_id: String,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FeatureUserRequest {
    pub user_id: String,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FeatureArtifactRequest {
    pub user_id: String,
    pub artifact_id: String,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FeatureDatasetRequest {
    pub user_id: String,
    pub dataset_id: String,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FeaturePlanDraftRequest {
    pub user_id: String,
    pub plan: FeaturePlanDraft,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FeatureDatasetRowsRequest {
    pub user_id: String,
    pub dataset_id: String,
    #[serde(default)]
    pub filter: FeatureDatasetFilter,
    #[serde(default)]
    pub offset: usize,
}

/// The concrete local dependencies composed into the Feature lifecycle
/// module. The complete Local Research state is never passed in; only the
/// database, the database file path for the engine's own Materialization
/// connection, the Feature Dataset directory, and User-scoped Snapshot and
/// Universe evidence reads are shared.
pub(crate) trait FeatureSource: SnapshotReadSource + Send + Sync {
    fn database(&self) -> Result<MutexGuard<'_, Connection>, String>;
    fn database_path(&self) -> Result<PathBuf, String>;
    fn feature_dataset_directory(&self) -> Result<PathBuf, String>;
    fn universe_snapshot_for_user(
        &self,
        user_id: &str,
        snapshot_id: &str,
    ) -> Result<MarketDataUniverseSnapshot, String>;
}

pub(super) struct ActiveAttempt {
    pub(super) user_id: String,
    pub(super) cancelled: Arc<AtomicBool>,
}

pub(super) struct QueueState {
    signaled: bool,
    shutdown: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FactorQueueItem {
    pub(super) attempt_id: String,
    pub(super) created_at_ms: i64,
}

pub(super) trait FactorQueueWork: Send + Sync {
    fn next_pending(&self) -> Result<Option<FactorQueueItem>, String>;
    fn execute(&self, item: FactorQueueItem);
}

pub(super) struct FeaturesInner {
    pub(super) source: Arc<dyn FeatureSource>,
    pub(super) materialization: FeatureMaterializationStore,
    pub(super) attempts: Mutex<HashMap<String, ActiveAttempt>>,
    pub(super) reset_blocks: Mutex<HashSet<String>>,
    /// Serializes Start/Retry block-checks and insertions against the
    /// Reset All barrier so no Attempt can slip past a User-scoped reset.
    pub(super) start_gate: Mutex<()>,
    pub(super) reset_wait_timeout: Mutex<Duration>,
    pub(super) queue: Mutex<QueueState>,
    pub(super) queue_changed: Condvar,
    pub(super) factor: Mutex<Option<Arc<dyn FactorQueueWork>>>,
    /// Private controllable runner seam: deterministic scheduling,
    /// cancellation, and race tests observe Attempts right after they
    /// become Running. Not part of the module interface.
    #[cfg(test)]
    pub(super) attempt_started_hook: Mutex<Option<Arc<dyn Fn(&str, &str) + Send + Sync>>>,
}

/// The Feature lifecycle interface: typed User-scoped Definition, Fitting,
/// Materialization, and evidence operations plus one persistent FIFO runner
/// for heavy work.
#[derive(Clone)]
pub(crate) struct Features {
    inner: Arc<FeaturesInner>,
    worker: Arc<Mutex<Option<std::thread::JoinHandle<()>>>>,
}

impl Drop for Features {
    fn drop(&mut self) {
        if let Ok(mut queue) = self.inner.queue.lock() {
            queue.shutdown = true;
            self.inner.queue_changed.notify_one();
        }
        if let Ok(mut worker) = self.worker.lock()
            && let Some(handle) = worker.take()
        {
            let _ = handle.join();
        }
    }
}

impl Features {
    /// Creates the module and performs its internal startup work: schema
    /// initialization, exact schema compatibility validation, recovery of
    /// Attempts interrupted by an application restart, and the persistent
    /// FIFO worker start. Pending Attempts survive the restart and run
    /// again.
    pub(crate) fn open(source: Arc<dyn FeatureSource>) -> Result<Self, String> {
        let database = source.database()?;
        store::FeatureStore::new(&database).initialize()?;
        drop(database);
        let database_path = source.database_path()?;
        let dataset_directory = source.feature_dataset_directory()?;
        let materialization = FeatureMaterializationStore::open(database_path, dataset_directory)
            .map_err(|error| error.to_string())?;
        let inner = Arc::new(FeaturesInner {
            source,
            materialization,
            attempts: Mutex::new(HashMap::new()),
            reset_blocks: Mutex::new(HashSet::new()),
            start_gate: Mutex::new(()),
            reset_wait_timeout: Mutex::new(Duration::from_secs(60)),
            queue: Mutex::new(QueueState {
                signaled: false,
                shutdown: false,
            }),
            queue_changed: Condvar::new(),
            factor: Mutex::new(None),
            #[cfg(test)]
            attempt_started_hook: Mutex::new(None),
        });
        let features = Self {
            inner: inner.clone(),
            worker: Arc::new(Mutex::new(Some(
                std::thread::Builder::new()
                    .name("adaq-feature-runner".into())
                    .spawn(move || runner::run_worker(inner))
                    .map_err(string)?,
            ))),
        };
        features.notify_runner();
        Ok(features)
    }

    // ---- Definitions ----

    pub(crate) fn validate_definition_draft(
        &self,
        request: DefinitionDraftRequest,
    ) -> Result<DraftValidationView, String> {
        validate_user(&request.user_id)?;
        Ok(match FeatureDefinition::freeze(request.draft) {
            Ok(_) => DraftValidationView {
                valid: true,
                issues: Vec::new(),
            },
            Err(error) => DraftValidationView {
                valid: false,
                issues: error.issues,
            },
        })
    }

    /// Publishes an immutable Definition revision. Name, description, and
    /// tags are User-scoped presentation metadata and never enter the
    /// semantic hash.
    pub(crate) fn publish_definition(
        &self,
        request: DefinitionPublishRequest,
    ) -> Result<DefinitionView, String> {
        validate_user(&request.user_id)?;
        let definition = FeatureDefinition::freeze(request.draft)
            .map_err(|error| format!("definition-validation-failed: {:?}", error.codes()))?;
        let definition_json = String::from_utf8(definition.to_json()).map_err(string)?;
        let database = self.inner.source.database()?;
        let store = store::FeatureStore::new(&database);
        store.publish_definition(
            &request.user_id,
            &definition.definition_id().to_string(),
            i64::try_from(definition.revision())
                .map_err(|_| "Feature Definition revision is too large")?,
            definition.definition_hash(),
            &definition_json,
        )?;
        store.upsert_presentation(
            &request.user_id,
            definition.definition_hash(),
            &request.name,
            &request.description,
            &serde_json::to_string(&request.tags).map_err(string)?,
        )?;
        let record = store.get_definition(&request.user_id, definition.definition_hash())?;
        drop(database);
        Ok(definition_view(record))
    }

    pub(crate) fn list_definitions(
        &self,
        request: FeatureUserRequest,
    ) -> Result<Vec<DefinitionView>, String> {
        validate_user(&request.user_id)?;
        let database = self.inner.source.database()?;
        let records = store::FeatureStore::new(&database).list_definitions(&request.user_id)?;
        drop(database);
        Ok(records.into_iter().map(definition_view).collect())
    }

    pub(crate) fn get_definition(
        &self,
        request: DefinitionIdRequest,
    ) -> Result<DefinitionView, String> {
        validate_user(&request.user_id)?;
        let database = self.inner.source.database()?;
        let record = store::FeatureStore::new(&database)
            .get_definition(&request.user_id, &request.definition_hash)?;
        drop(database);
        Ok(definition_view(record))
    }

    /// Bounded transient Draft Preview over immutable Snapshot evidence:
    /// uses the production engine, never fits, creates no evidence
    /// identity, and retains complete Cross-Sectional batches.
    pub(crate) fn preview_definition_draft(
        &self,
        request: FeaturePreviewRequest,
    ) -> Result<FeaturePreviewView, String> {
        validate_user(&request.user_id)?;
        preview::preview(&self.inner, request)
    }

    // ---- Plans ----

    /// Freezes a Plan draft with the native engine identity so the GUI can
    /// learn the immutable Plan hash before binding it to a Materialization
    /// request. Validates the draft natively and creates no evidence.
    pub(crate) fn freeze_plan_for_user(
        &self,
        request: FeaturePlanDraftRequest,
    ) -> Result<PlanFreezeView, String> {
        validate_user(&request.user_id)?;
        let identity = native_identity()?;
        let plan = freeze_plan(request.plan, &identity)?;
        let plan_json = String::from_utf8(plan.to_json()).map_err(string)?;
        Ok(PlanFreezeView {
            plan_hash: plan.plan_hash().to_owned(),
            plan_json,
        })
    }

    // ---- Fitting ----

    pub(crate) fn start_fitting(
        &self,
        request: FeatureFittingStartRequest,
    ) -> Result<FittingAttemptView, String> {
        validate_user(&request.user_id)?;
        let _gate = self.inner.start_gate.lock().map_err(string)?;
        self.ensure_not_blocked(&request.user_id)?;
        let identity = native_identity()?;
        let protocol = TransformationFittingProtocol::freeze(TransformationFittingProtocolDraft {
            engine_identity: identity.clone(),
            ..request.protocol
        })
        .map_err(|error| format!("fitting-protocol-validation-failed: {:?}", error.codes()))?;
        let plan = freeze_plan(request.plan, &identity)?;
        validate_plan_artifacts(&self.inner, &request.user_id, &plan)?;
        let protocol_json = String::from_utf8(protocol.to_json()).map_err(string)?;
        let plan_json = String::from_utf8(plan.to_json()).map_err(string)?;
        let database = self.inner.source.database()?;
        let store = store::FeatureStore::new(&database);
        store.upsert_protocol(protocol.protocol_hash(), &protocol_json)?;
        let (attempt, _) = store.prepare_fitting(
            &request.user_id,
            protocol.protocol_hash(),
            plan.plan_hash(),
            &plan_json,
            || runner::new_attempt_id(protocol.protocol_hash()),
        )?;
        drop(database);
        self.notify_runner();
        Ok(fitting_view(&attempt))
    }

    pub(crate) fn list_fitting_attempts(
        &self,
        request: FeatureUserRequest,
    ) -> Result<Vec<FittingAttemptView>, String> {
        validate_user(&request.user_id)?;
        let database = self.inner.source.database()?;
        let attempts = store::FeatureStore::new(&database).fitting_attempts(&request.user_id)?;
        drop(database);
        Ok(attempts.iter().map(fitting_view).collect())
    }

    pub(crate) fn get_fitting_attempt(
        &self,
        request: FeatureAttemptRequest,
    ) -> Result<FittingAttemptView, String> {
        validate_user(&request.user_id)?;
        let database = self.inner.source.database()?;
        let attempt = store::FeatureStore::new(&database)
            .fitting_attempt(&request.user_id, &request.attempt_id)?;
        drop(database);
        Ok(fitting_view(&attempt))
    }

    /// Requests cancellation. A Pending Attempt becomes Cancelled at once;
    /// a Running Attempt becomes Cancelled only after its worker has
    /// stopped and released its evidence handles.
    pub(crate) fn cancel_fitting_attempt(
        &self,
        request: FeatureAttemptRequest,
    ) -> Result<(), String> {
        validate_user(&request.user_id)?;
        let database = self.inner.source.database()?;
        let store = store::FeatureStore::new(&database);
        let attempt = store.fitting_attempt(&request.user_id, &request.attempt_id)?;
        match attempt.status {
            FeatureAttemptStatus::Pending => {
                store.cancel_fitting(&request.user_id, &request.attempt_id, &["pending"])?;
                // If the runner made the Attempt Running between the status
                // read and the conditional update, the registered flag still
                // reaches its evaluation loop and finalizes the Cancelled
                // state after cleanup.
                if let Some(active) = self
                    .inner
                    .attempts
                    .lock()
                    .map_err(string)?
                    .get(&request.attempt_id)
                {
                    active.cancelled.store(true, Ordering::Relaxed);
                }
            }
            FeatureAttemptStatus::Running => {
                let attempts = self.inner.attempts.lock().map_err(string)?;
                if let Some(active) = attempts.get(&request.attempt_id) {
                    active.cancelled.store(true, Ordering::Relaxed);
                }
            }
            _ => return Err("Feature Fitting Attempt cannot be cancelled".into()),
        }
        Ok(())
    }

    /// Retries a Failed or Cancelled Attempt with a new Attempt identity,
    /// reusing the retained source evidence; repeated retries coalesce with
    /// an active Pending or Running retry.
    pub(crate) fn retry_fitting_attempt(
        &self,
        request: FeatureAttemptRequest,
    ) -> Result<FittingAttemptView, String> {
        validate_user(&request.user_id)?;
        let _gate = self.inner.start_gate.lock().map_err(string)?;
        self.ensure_not_blocked(&request.user_id)?;
        let database = self.inner.source.database()?;
        let store = store::FeatureStore::new(&database);
        let attempt = store.prepare_fitting_retry(&request.user_id, &request.attempt_id, || {
            runner::new_attempt_id(&request.attempt_id)
        })?;
        drop(database);
        self.notify_runner();
        Ok(fitting_view(&attempt))
    }

    pub(crate) fn list_artifacts(
        &self,
        request: FeatureUserRequest,
    ) -> Result<Vec<ArtifactView>, String> {
        validate_user(&request.user_id)?;
        let database = self.inner.source.database()?;
        let artifacts = store::FeatureStore::new(&database).artifacts_for_user(&request.user_id)?;
        drop(database);
        Ok(artifacts
            .into_iter()
            .map(|artifact| ArtifactView {
                artifact_id: artifact.artifact_id,
                protocol_hash: artifact.protocol_hash,
                artifact_json: artifact.artifact_json,
                created_at_ms: artifact.created_at_ms,
            })
            .collect())
    }

    pub(crate) fn get_artifact(
        &self,
        request: FeatureArtifactRequest,
    ) -> Result<ArtifactView, String> {
        validate_user(&request.user_id)?;
        let database = self.inner.source.database()?;
        let artifact = store::FeatureStore::new(&database)
            .artifact_for_user(&request.user_id, &request.artifact_id)?;
        drop(database);
        Ok(ArtifactView {
            artifact_id: artifact.artifact_id,
            protocol_hash: artifact.protocol_hash,
            artifact_json: artifact.artifact_json,
            created_at_ms: artifact.created_at_ms,
        })
    }

    /// Deletes one unlocked Artifact. Downstream local research owners hold
    /// narrow typed references that block deletion until they disappear;
    /// content bytes disappear only after the last User's access is gone.
    pub(crate) fn delete_artifact(&self, request: FeatureArtifactRequest) -> Result<(), String> {
        validate_user(&request.user_id)?;
        let database = self.inner.source.database()?;
        store::FeatureStore::new(&database)
            .delete_artifact(&request.user_id, &request.artifact_id)?;
        Ok(())
    }

    // ---- Materialization ----

    pub(crate) fn start_materialization(
        &self,
        request: FeatureMaterializationStartRequest,
    ) -> Result<MaterializationAttempt, String> {
        validate_user(&request.user_id)?;
        let _gate = self.inner.start_gate.lock().map_err(string)?;
        self.ensure_not_blocked(&request.user_id)?;
        if request.request.user_id != request.user_id {
            return Err("invalid-feature-materialization-request".into());
        }
        let identity = native_identity()?;
        let plan = freeze_plan(request.plan, &identity)?;
        if plan_has_cross_sectional_scope(&plan) {
            return Err("cross-sectional-feature-evidence-not-wired".into());
        }
        validate_plan_artifacts(&self.inner, &request.user_id, &plan)?;
        let materialization_request = request
            .request
            .with_plan_evidence(&plan)
            .map_err(|_| "invalid-feature-materialization-request")?;
        let attempt = self
            .inner
            .materialization
            .start_for_plan(materialization_request, &plan)
            .map_err(|error| error.to_string())?;
        self.notify_runner();
        Ok(attempt)
    }

    pub(crate) fn list_materialization_attempts(
        &self,
        request: FeatureUserRequest,
    ) -> Result<Vec<MaterializationAttempt>, String> {
        validate_user(&request.user_id)?;
        self.inner
            .materialization
            .attempts(&request.user_id)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn get_materialization_attempt(
        &self,
        request: FeatureAttemptRequest,
    ) -> Result<MaterializationAttempt, String> {
        validate_user(&request.user_id)?;
        self.inner
            .materialization
            .attempt(&request.user_id, &request.attempt_id)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn cancel_materialization_attempt(
        &self,
        request: FeatureAttemptRequest,
    ) -> Result<(), String> {
        validate_user(&request.user_id)?;
        let attempt = self
            .inner
            .materialization
            .attempt(&request.user_id, &request.attempt_id)
            .map_err(|error| error.to_string())?;
        match attempt.status {
            adaq_feature_engine::MaterializationAttemptStatus::Pending => {
                self.inner
                    .materialization
                    .cancel(&request.user_id, &request.attempt_id)
                    .map_err(|error| error.to_string())?;
            }
            adaq_feature_engine::MaterializationAttemptStatus::Running => {
                let attempts = self.inner.attempts.lock().map_err(string)?;
                if let Some(active) = attempts.get(&request.attempt_id) {
                    active.cancelled.store(true, Ordering::Relaxed);
                }
            }
            _ => return Err("Feature Materialization Attempt cannot be cancelled".into()),
        }
        Ok(())
    }

    pub(crate) fn retry_materialization_attempt(
        &self,
        request: FeatureAttemptRequest,
    ) -> Result<MaterializationAttempt, String> {
        validate_user(&request.user_id)?;
        let _gate = self.inner.start_gate.lock().map_err(string)?;
        self.ensure_not_blocked(&request.user_id)?;
        let attempt = self
            .inner
            .materialization
            .retry(&request.user_id, &request.attempt_id)
            .map_err(|error| error.to_string())?;
        self.notify_runner();
        Ok(attempt)
    }

    // ---- Datasets ----

    pub(crate) fn list_datasets(
        &self,
        request: FeatureUserRequest,
    ) -> Result<Vec<FeatureDatasetView>, String> {
        validate_user(&request.user_id)?;
        let datasets = self
            .inner
            .materialization
            .datasets(&request.user_id)
            .map_err(|error| error.to_string())?;
        Ok(datasets.into_iter().map(dataset_view).collect())
    }

    pub(crate) fn get_dataset(
        &self,
        request: FeatureDatasetRequest,
    ) -> Result<FeatureDatasetView, String> {
        validate_user(&request.user_id)?;
        let dataset = self
            .inner
            .materialization
            .dataset(&request.user_id, &request.dataset_id)
            .map_err(|error| error.to_string())?;
        Ok(dataset_view(dataset))
    }

    pub(crate) fn materialization_store(&self) -> FeatureMaterializationStore {
        self.inner.materialization.clone()
    }

    pub(crate) fn completed_dataset_from_store(
        store: &FeatureMaterializationStore,
        user_id: &str,
        dataset_id: &str,
    ) -> Result<CompletedFeatureDataset, String> {
        validate_user(user_id)?;
        let dataset = store
            .dataset(user_id, dataset_id)
            .map_err(|error| error.to_string())?;
        let row_capacity = usize::try_from(dataset.manifest.row_count)
            .map_err(|_| "Feature Dataset row count exceeds the host allocation limit")?;
        let mut rows = Vec::with_capacity(row_capacity);
        let mut offset = 0usize;
        loop {
            let page = store
                .page(
                    user_id,
                    dataset_id,
                    FeatureDatasetFilter {
                        limit: adaq_feature_engine::FEATURE_DATASET_MAX_PAGE_SIZE,
                        ..FeatureDatasetFilter::default()
                    },
                    offset,
                )
                .map_err(|error| error.to_string())?;
            rows.extend(page.rows);
            match page.next_offset {
                Some(next) => offset = next,
                None => break,
            }
        }
        CompletedFeatureDataset::new(
            user_id,
            dataset.dataset_id,
            dataset.manifest.request.feature_plan_hash,
            serde_json::to_vec(&dataset.manifest.plan_json).map_err(string)?,
            dataset.manifest.engine_identity,
            dataset.manifest.request.snapshot_id,
            dataset.manifest.request.point_in_time_universe_id,
            dataset
                .manifest
                .outputs
                .into_iter()
                .map(|output| output.output_name)
                .collect(),
            rows,
        )
        .map_err(|error| error.to_string())
    }

    pub(crate) fn dataset_summary(
        &self,
        request: FeatureDatasetRequest,
    ) -> Result<Vec<FeatureOutputSummary>, String> {
        validate_user(&request.user_id)?;
        self.inner
            .materialization
            .summary(&request.user_id, &request.dataset_id)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn dataset_rows(
        &self,
        request: FeatureDatasetRowsRequest,
    ) -> Result<FeatureDatasetPage, String> {
        validate_user(&request.user_id)?;
        self.inner
            .materialization
            .page(
                &request.user_id,
                &request.dataset_id,
                request.filter,
                request.offset,
            )
            .map_err(|error| error.to_string())
    }

    /// Deletes one unlocked Dataset. Downstream local research owners hold
    /// narrow typed references that block deletion; deduplicated bytes are
    /// removed only after the last User's access disappears, without
    /// granting cross-User visibility.
    pub(crate) fn delete_dataset(&self, request: FeatureDatasetRequest) -> Result<(), String> {
        validate_user(&request.user_id)?;
        self.inner
            .materialization
            .delete_dataset(&request.user_id, &request.dataset_id)
            .map_err(|error| error.to_string())
    }

    // ---- Lifecycle ----

    /// Lifecycle barrier for a User-scoped Reset All: blocks new Start and
    /// Retry for one User, cancels that User's active Attempts, and waits
    /// for the runner to release them without holding the SQLite mutex.
    /// Returns a guard that keeps the User's start-restriction in place
    /// until the caller's reset work is finished.
    pub(crate) fn stop_all_for_user<'a>(
        &'a self,
        user_id: &str,
    ) -> Result<UserFeatureResetBlock<'a>, String> {
        runner::stop_all_for_user(&self.inner, user_id)
    }

    /// Removes one User's Materialization Attempts and Dataset evidence
    /// through the engine store's own connection. Runs before the shared
    /// reset transaction so a failure deletes nothing else.
    pub(crate) fn reset_materialization_for_user(&self, user_id: &str) -> Result<(), String> {
        validate_user(user_id)?;
        self.inner
            .materialization
            .reset_for_user(user_id)
            .map_err(|error| error.to_string())
    }

    #[cfg(test)]
    pub(crate) fn set_reset_wait_timeout(&self, timeout: Duration) {
        *self.inner.reset_wait_timeout.lock().unwrap() = timeout;
    }

    fn ensure_not_blocked(&self, user_id: &str) -> Result<(), String> {
        if self
            .inner
            .reset_blocks
            .lock()
            .map_err(string)?
            .contains(user_id)
        {
            return Err("Feature work is blocked while Reset All is in progress".into());
        }
        Ok(())
    }

    pub(super) fn notify_runner(&self) {
        if let Ok(mut queue) = self.inner.queue.lock() {
            queue.signaled = true;
            self.inner.queue_changed.notify_one();
        }
    }

    pub(crate) fn attach_factor(&self, factor: Arc<dyn FactorQueueWork>) {
        if let Ok(mut attached) = self.inner.factor.lock() {
            *attached = Some(factor);
        }
        self.notify_runner();
    }

    pub(crate) fn queue_notifier(&self) -> Arc<dyn Fn() + Send + Sync> {
        let weak = Arc::downgrade(&self.inner);
        Arc::new(move || {
            if let Some(inner) = weak.upgrade()
                && let Ok(mut queue) = inner.queue.lock()
            {
                queue.signaled = true;
                inner.queue_changed.notify_one();
            }
        })
    }
}

/// RAII guard holding one User's Feature start-restriction; Drop always
/// releases it (success, failure, and panic paths).
pub(crate) struct UserFeatureResetBlock<'a> {
    inner: &'a FeaturesInner,
    user_id: String,
}

impl Drop for UserFeatureResetBlock<'_> {
    fn drop(&mut self) {
        if let Ok(mut blocks) = self.inner.reset_blocks.lock() {
            blocks.remove(&self.user_id);
        }
    }
}

impl UserFeatureResetBlock<'_> {
    /// Removes this User's Definition, Fitting, and Artifact evidence
    /// inside the caller's reset transaction. Only valid after the barrier
    /// has fully stopped this User's Feature work.
    pub(crate) fn delete_attempt_evidence(&self, database: &Connection) -> Result<(), String> {
        store::FeatureStore::new(database).delete_for_user(&self.user_id)
    }
}

fn native_identity() -> Result<adaq_feature_engine::FeatureEngineIdentity, String> {
    adaq_feature_engine::FeatureEngineIdentity::native().map_err(|error| error.to_string())
}

fn freeze_plan(
    mut draft: FeaturePlanDraft,
    identity: &adaq_feature_engine::FeatureEngineIdentity,
) -> Result<FeaturePlan, String> {
    draft.engine_identity = identity.clone();
    FeaturePlan::freeze(draft)
        .map_err(|error| format!("feature-plan-validation-failed: {:?}", error.codes()))
}

fn plan_has_cross_sectional_scope(plan: &FeaturePlan) -> bool {
    plan.definitions()
        .iter()
        .any(|definition| definition.scope() == FeatureScope::CrossSectional)
}

/// Every Fitted Artifact a Plan binds must already exist for the requesting
/// User; materialization applies Artifacts and never refits them.
fn validate_plan_artifacts(
    inner: &FeaturesInner,
    user_id: &str,
    plan: &FeaturePlan,
) -> Result<(), String> {
    if plan.artifacts().is_empty() {
        return Ok(());
    }
    let database = inner.source.database()?;
    let store = store::FeatureStore::new(&database);
    for binding in plan.artifacts() {
        store.artifact_for_user(user_id, &binding.artifact_id)?;
    }
    Ok(())
}

fn definition_view(record: DefinitionRecord) -> DefinitionView {
    DefinitionView {
        definition_id: record.definition_id,
        revision: record.revision,
        definition_hash: record.definition_hash,
        definition_json: record.definition_json,
        name: record.name,
        description: record.description,
        tags: serde_json::from_str(&record.tags_json).unwrap_or_default(),
        created_at_ms: record.created_at_ms,
    }
}

fn fitting_view(attempt: &FittingAttemptRecord) -> FittingAttemptView {
    FittingAttemptView {
        attempt_id: attempt.attempt_id.clone(),
        user_id: attempt.user_id.clone(),
        protocol_hash: attempt.protocol_hash.clone(),
        plan_hash: attempt.plan_hash.clone(),
        status: attempt.status,
        source_attempt_id: attempt.source_attempt_id.clone(),
        artifact_id: attempt.artifact_id.clone(),
        failure_code: attempt.failure_code.clone(),
        diagnostic: attempt.diagnostic.clone(),
        progress_completed: attempt.progress_completed,
        progress_total: attempt.progress_total,
        created_at_ms: attempt.created_at_ms,
        updated_at_ms: attempt.updated_at_ms,
    }
}

fn dataset_view(dataset: adaq_feature_engine::FeatureDataset) -> FeatureDatasetView {
    FeatureDatasetView {
        dataset_id: dataset.dataset_id,
        user_id: dataset.user_id,
        request_hash: dataset.request_hash,
        manifest: dataset.manifest,
        content_byte_size: dataset.content_byte_size,
        created_at_ms: dataset.created_at_ms,
    }
}

pub(super) fn bounded_diagnostic(details: impl Into<String>) -> String {
    details
        .into()
        .chars()
        .take(MAX_DIAGNOSTIC_EVIDENCE_CHARS)
        .collect()
}

pub(super) fn instrument_id_for(src: &str, code: &str) -> String {
    format!("{src}:{code}")
}

fn string(error: impl ToString) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests;
