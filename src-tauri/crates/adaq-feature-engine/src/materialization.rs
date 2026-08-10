use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    fs::{self, File},
    io::{BufReader, Read},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use arrow_array::{Array, ArrayRef, Float64Array, Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use parquet::arrow::{ArrowWriter, arrow_reader::ParquetRecordBatchReaderBuilder};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use super::{
    FEATURE_UNAVAILABILITY_REASON_VERSION, FeatureEngineIdentity, FeatureMaterializationRequest,
    FeatureObservation, FeatureObservationValue, FeaturePlan, FeatureUnavailabilityReason,
    canonicalize_json, is_lower_kebab, is_sha256,
};

pub const FEATURE_DATASET_STORAGE_SCHEMA_VERSION: &str = "1.4.0";
pub const FEATURE_DATASET_MANIFEST_SCHEMA_VERSION: &str = "1.0.0";
pub const FEATURE_DATASET_MAX_PAGE_SIZE: usize = 50;

const STATUS_PENDING: &str = "pending";
const STATUS_RUNNING: &str = "running";
const STATUS_COMPLETED: &str = "completed";
const STATUS_FAILED: &str = "failed";
const STATUS_CANCELLED: &str = "cancelled";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MaterializationAttemptStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl MaterializationAttemptStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => STATUS_PENDING,
            Self::Running => STATUS_RUNNING,
            Self::Completed => STATUS_COMPLETED,
            Self::Failed => STATUS_FAILED,
            Self::Cancelled => STATUS_CANCELLED,
        }
    }
}

impl TryFrom<&str> for MaterializationAttemptStatus {
    type Error = MaterializationStoreError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            STATUS_PENDING => Ok(Self::Pending),
            STATUS_RUNNING => Ok(Self::Running),
            STATUS_COMPLETED => Ok(Self::Completed),
            STATUS_FAILED => Ok(Self::Failed),
            STATUS_CANCELLED => Ok(Self::Cancelled),
            _ => Err(MaterializationStoreError::IncompatibleSchema {
                stored_schema_version: None,
                table: Some("feature_materialization_attempts.status".into()),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaterializationStoreError {
    InvalidUser,
    InvalidRequest,
    AttemptNotFound,
    DatasetNotFound,
    InvalidTransition,
    InvalidOutputSchema,
    InvalidObservation(String),
    IncompleteRows,
    DuplicateObservation,
    StagingNotFound,
    Unauthorized,
    DatasetReferenced,
    DatasetContentCollision,
    InvalidFilter,
    PublicationInProgress,
    PublicationRecoveryFailed {
        publication: String,
        recovery: String,
    },
    ResetRequired {
        stored_schema_version: Option<String>,
        table: Option<String>,
    },
    IncompatibleSchema {
        stored_schema_version: Option<String>,
        table: Option<String>,
    },
    Io(String),
    Sqlite(String),
    Parquet(String),
    Json(String),
}

impl MaterializationStoreError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidUser => "invalid-user",
            Self::InvalidRequest => "invalid-feature-materialization-request",
            Self::AttemptNotFound => "materialization-attempt-not-found",
            Self::DatasetNotFound => "feature-dataset-not-found",
            Self::InvalidTransition => "invalid-materialization-transition",
            Self::InvalidOutputSchema => "invalid-feature-dataset-schema",
            Self::InvalidObservation(_) => "invalid-feature-observation",
            Self::IncompleteRows => "incomplete-feature-dataset-rows",
            Self::DuplicateObservation => "duplicate-feature-observation",
            Self::StagingNotFound => "materialization-staging-not-found",
            Self::Unauthorized => "feature-dataset-not-authorized",
            Self::DatasetReferenced => "feature-dataset-referenced",
            Self::DatasetContentCollision => "feature-dataset-content-collision",
            Self::InvalidFilter => "invalid-feature-dataset-filter",
            Self::PublicationInProgress => "feature-dataset-publication-in-progress",
            Self::PublicationRecoveryFailed { .. } => "publication-recovery-failed",
            Self::ResetRequired { .. } => "reset-required",
            Self::IncompatibleSchema { .. } => "incompatible-feature-schema",
            Self::Io(_) => "feature-dataset-io",
            Self::Sqlite(_) => "feature-dataset-sqlite",
            Self::Parquet(_) => "feature-dataset-parquet",
            Self::Json(_) => "feature-dataset-json",
        }
    }
}

impl fmt::Display for MaterializationStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidObservation(message)
            | Self::Io(message)
            | Self::Sqlite(message)
            | Self::Parquet(message)
            | Self::Json(message) => write!(formatter, "{}: {message}", self.code()),
            Self::PublicationInProgress => formatter.write_str(self.code()),
            Self::PublicationRecoveryFailed {
                publication,
                recovery,
            } => write!(
                formatter,
                "{}: {publication}; recovery: {recovery}",
                self.code()
            ),
            _ => formatter.write_str(self.code()),
        }
    }
}

impl std::error::Error for MaterializationStoreError {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MaterializationAttempt {
    pub attempt_id: String,
    pub user_id: String,
    pub request_hash: String,
    pub status: MaterializationAttemptStatus,
    pub source_attempt_id: Option<String>,
    pub dataset_id: Option<String>,
    pub failure_code: Option<String>,
    pub diagnostic: Option<String>,
    pub progress_completed: u64,
    pub progress_total: u64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FeatureDatasetOutputManifest {
    pub output_name: String,
    pub value_column: String,
    pub available_at_column: String,
    pub state_column: String,
    pub reason_column: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FeatureDatasetManifest {
    pub manifest_schema_version: String,
    pub request: FeatureMaterializationRequest,
    pub request_hash: String,
    pub plan_json: Value,
    pub artifact_ids: Vec<String>,
    pub engine_identity: FeatureEngineIdentity,
    pub reason_version: String,
    pub outputs: Vec<FeatureDatasetOutputManifest>,
    pub row_count: u64,
    pub content_sha256: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FeatureDataset {
    pub dataset_id: String,
    pub user_id: String,
    pub request_hash: String,
    pub manifest: FeatureDatasetManifest,
    pub parquet_path: PathBuf,
    pub content_byte_size: u64,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "state",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum FeatureDatasetCell {
    Available { value: f64, available_at_ms: i64 },
    Unavailable { reason: FeatureUnavailabilityReason },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FeatureDatasetRow {
    pub instrument_id: String,
    pub observation_time_ms: i64,
    pub values: BTreeMap<String, FeatureDatasetCell>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FeatureDatasetRowState {
    Available,
    Unavailable,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FeatureDatasetFilter {
    pub instrument_id: Option<String>,
    pub start_time_ms: Option<i64>,
    pub end_time_ms: Option<i64>,
    pub output_name: Option<String>,
    pub state: Option<FeatureDatasetRowState>,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FeatureDatasetPage {
    pub rows: Vec<FeatureDatasetRow>,
    pub next_offset: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FeatureOutputSummary {
    pub output_name: String,
    pub row_count: u64,
    pub available_count: u64,
    pub coverage: f64,
    pub unavailable_counts: BTreeMap<String, u64>,
    pub minimum: Option<f64>,
    pub maximum: Option<f64>,
    pub mean: Option<f64>,
    pub population_standard_deviation: Option<f64>,
}

#[derive(Clone)]
pub struct FeatureMaterializationStore {
    database: Arc<Mutex<Connection>>,
    dataset_directory: PathBuf,
}

struct PlanEvidence {
    plan_json: String,
    artifact_ids: Vec<String>,
    engine_identity: FeatureEngineIdentity,
    output_names: Vec<String>,
}

impl FeatureMaterializationStore {
    pub fn open(
        database_path: impl AsRef<Path>,
        dataset_directory: impl AsRef<Path>,
    ) -> Result<Self, MaterializationStoreError> {
        let database = Connection::open(database_path).map_err(sqlite_error)?;
        Self::with_connection(database, dataset_directory)
    }

    pub fn with_connection(
        database: Connection,
        dataset_directory: impl AsRef<Path>,
    ) -> Result<Self, MaterializationStoreError> {
        database
            .busy_timeout(Duration::from_secs(5))
            .map_err(sqlite_error)?;
        let store = Self {
            database: Arc::new(Mutex::new(database)),
            dataset_directory: dataset_directory.as_ref().to_path_buf(),
        };
        fs::create_dir_all(store.staging_directory()).map_err(io_error)?;
        store.initialize_schema()?;
        store.recover_stale_running_attempts()?;
        store.recover_pending_deletions()?;
        Ok(store)
    }

    pub fn start(
        &self,
        request: FeatureMaterializationRequest,
        plan: &FeaturePlan,
    ) -> Result<MaterializationAttempt, MaterializationStoreError> {
        self.start_for_plan(request, plan)
    }

    pub fn start_for_plan(
        &self,
        request: FeatureMaterializationRequest,
        plan: &FeaturePlan,
    ) -> Result<MaterializationAttempt, MaterializationStoreError> {
        let request = request
            .with_plan_evidence(plan)
            .map_err(|_| MaterializationStoreError::InvalidRequest)?;
        let plan_json = String::from_utf8(plan.to_json())
            .map_err(|error| MaterializationStoreError::Json(error.to_string()))?;
        let artifact_ids = plan
            .artifacts()
            .iter()
            .map(|artifact| artifact.artifact_id.clone())
            .collect::<Vec<_>>();
        self.start_internal(
            request,
            PlanEvidence {
                plan_json,
                artifact_ids,
                engine_identity: plan.engine_identity(),
                output_names: plan_output_names(plan),
            },
        )
    }

    pub fn begin(
        &self,
        user_id: &str,
        attempt_id: &str,
    ) -> Result<MaterializationAttempt, MaterializationStoreError> {
        validate_user(user_id)?;
        let database = self.lock_database()?;
        let transaction = database.unchecked_transaction().map_err(sqlite_error)?;
        let current = load_attempt(&transaction, user_id, attempt_id)?;
        match current.status {
            MaterializationAttemptStatus::Pending => {
                transaction
                    .execute(
                        "UPDATE feature_materialization_attempts
                         SET status = ?1, updated_at_ms = ?2
                         WHERE attempt_id = ?3 AND user_id = ?4 AND status = ?5",
                        params![
                            STATUS_RUNNING,
                            now_ms(),
                            attempt_id,
                            user_id,
                            STATUS_PENDING
                        ],
                    )
                    .map_err(sqlite_error)?;
            }
            MaterializationAttemptStatus::Running => {}
            MaterializationAttemptStatus::Completed
            | MaterializationAttemptStatus::Failed
            | MaterializationAttemptStatus::Cancelled => {
                return Err(MaterializationStoreError::InvalidTransition);
            }
        }
        transaction.commit().map_err(sqlite_error)?;
        drop(database);
        self.attempt(user_id, attempt_id)
    }

    pub fn retry(
        &self,
        user_id: &str,
        attempt_id: &str,
    ) -> Result<MaterializationAttempt, MaterializationStoreError> {
        validate_user(user_id)?;
        let database = self.lock_database()?;
        let transaction = database.unchecked_transaction().map_err(sqlite_error)?;
        let previous = load_attempt(&transaction, user_id, attempt_id)?;
        if !matches!(
            previous.status,
            MaterializationAttemptStatus::Failed | MaterializationAttemptStatus::Cancelled
        ) {
            return Err(MaterializationStoreError::InvalidTransition);
        }
        if let Some(active) = transaction
            .query_row(
                "SELECT attempt_id FROM feature_materialization_attempts
                 WHERE user_id = ?1 AND request_hash = ?2
                   AND status IN (?3, ?4)
                 ORDER BY queue_sequence LIMIT 1",
                params![
                    user_id,
                    previous.request_hash,
                    STATUS_PENDING,
                    STATUS_RUNNING
                ],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(sqlite_error)?
        {
            transaction.commit().map_err(sqlite_error)?;
            drop(database);
            return self.attempt(user_id, &active);
        }
        let attempt = new_attempt(user_id, &previous.request_hash, Some(attempt_id.to_owned()));
        insert_attempt(&transaction, &attempt).map_err(sqlite_error)?;
        transaction.commit().map_err(sqlite_error)?;
        Ok(attempt)
    }

    pub fn attempt(
        &self,
        user_id: &str,
        attempt_id: &str,
    ) -> Result<MaterializationAttempt, MaterializationStoreError> {
        validate_user(user_id)?;
        let database = self.lock_database()?;
        load_attempt(&database, user_id, attempt_id)
    }

    pub fn attempts(
        &self,
        user_id: &str,
    ) -> Result<Vec<MaterializationAttempt>, MaterializationStoreError> {
        validate_user(user_id)?;
        let database = self.lock_database()?;
        let mut statement = database
            .prepare(
                "SELECT attempt_id, user_id, request_hash, status, source_attempt_id,
                        dataset_id, failure_code, diagnostic, progress_completed,
                        progress_total, created_at_ms, updated_at_ms
                 FROM feature_materialization_attempts
                 WHERE user_id = ?1
                 ORDER BY queue_sequence",
            )
            .map_err(sqlite_error)?;
        let rows = statement
            .query_map([user_id], row_to_attempt)
            .map_err(sqlite_error)?;
        rows.map(|row| row.map_err(sqlite_error)).collect()
    }

    pub fn next_pending(
        &self,
    ) -> Result<Option<MaterializationAttempt>, MaterializationStoreError> {
        let database = self.lock_database()?;
        database
            .query_row(
                "SELECT attempt_id, user_id, request_hash, status, source_attempt_id,
                        dataset_id, failure_code, diagnostic, progress_completed,
                        progress_total, created_at_ms, updated_at_ms
                 FROM feature_materialization_attempts
                 WHERE status = ?1
                 ORDER BY queue_sequence LIMIT 1",
                [STATUS_PENDING],
                row_to_attempt,
            )
            .optional()
            .map_err(sqlite_error)
    }

    pub fn stage(
        &self,
        user_id: &str,
        attempt_id: &str,
        output_names: &[&str],
        observations: &[FeatureObservation],
    ) -> Result<(), MaterializationStoreError> {
        validate_user(user_id)?;
        let (request_json, output_names_json): (String, String) = {
            let database = self.lock_database()?;
            let attempt = load_attempt(&database, user_id, attempt_id)?;
            if attempt.status != MaterializationAttemptStatus::Running
                || attempt.dataset_id.is_some()
            {
                return Err(MaterializationStoreError::InvalidTransition);
            }
            let staging: Option<String> = database
                .query_row(
                    "SELECT staging_path FROM feature_materialization_attempts
                     WHERE attempt_id = ?1 AND user_id = ?2",
                    params![attempt_id, user_id],
                    |row| row.get(0),
                )
                .map_err(sqlite_error)?;
            if staging.is_some() {
                return Err(MaterializationStoreError::InvalidTransition);
            }
            database
                .query_row(
                    "SELECT r.request_json, r.output_names_json
                     FROM feature_materialization_requests r
                     WHERE r.request_hash = ?1",
                    [attempt.request_hash.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(sqlite_error)?
        };
        let request = serde_json::from_str::<FeatureMaterializationRequest>(&request_json)
            .map_err(|error| MaterializationStoreError::Json(error.to_string()))?;
        let expected_output_names = serde_json::from_str::<Vec<String>>(&output_names_json)
            .map_err(|error| MaterializationStoreError::Json(error.to_string()))?;
        if expected_output_names
            != output_names
                .iter()
                .map(|name| (*name).to_owned())
                .collect::<Vec<_>>()
        {
            return Err(MaterializationStoreError::InvalidOutputSchema);
        }
        let (names, rows) = normalize_observations(output_names, observations, &request)?;
        let staging_path = self
            .staging_directory()
            .join(format!("{attempt_id}.parquet"));
        let temporary_path = self
            .staging_directory()
            .join(format!("{attempt_id}.parquet.tmp"));
        let names_json = json_string(&names)?;
        let database = self.lock_database()?;
        let claimed = database
            .execute(
                "UPDATE feature_materialization_attempts
                 SET staging_path = ?1, updated_at_ms = ?2
                 WHERE attempt_id = ?3 AND user_id = ?4 AND status = ?5
                   AND staging_path IS NULL",
                params![
                    temporary_path.to_string_lossy().as_ref(),
                    now_ms(),
                    attempt_id,
                    user_id,
                    STATUS_RUNNING
                ],
            )
            .map_err(sqlite_error)?;
        drop(database);
        if claimed != 1 {
            return Err(MaterializationStoreError::InvalidTransition);
        }
        write_parquet(&temporary_path, &names, &rows)?;
        fs::rename(&temporary_path, &staging_path).map_err(io_error)?;
        let database = self.lock_database()?;
        let updated = database
            .execute(
                "UPDATE feature_materialization_attempts
                 SET output_names_json = ?1, staging_path = ?2,
                 progress_completed = ?3, progress_total = ?4, updated_at_ms = ?5
                 WHERE attempt_id = ?6 AND user_id = ?7 AND status = ?8
                   AND staging_path = ?9",
                params![
                    names_json,
                    staging_path.to_string_lossy().as_ref(),
                    rows.len() as i64,
                    rows.len() as i64,
                    now_ms(),
                    attempt_id,
                    user_id,
                    STATUS_RUNNING,
                    temporary_path.to_string_lossy().as_ref()
                ],
            )
            .map_err(sqlite_error)?;
        if updated != 1 {
            drop(database);
            cleanup_staging_files(&staging_path, attempt_id, &self.staging_directory())?;
            return Err(MaterializationStoreError::InvalidTransition);
        }
        Ok(())
    }

    pub fn publish(
        &self,
        user_id: &str,
        attempt_id: &str,
    ) -> Result<MaterializationAttempt, MaterializationStoreError> {
        match self.publish_inner(user_id, attempt_id) {
            Ok(attempt) => Ok(attempt),
            Err(publication @ MaterializationStoreError::PublicationInProgress) => Err(publication),
            Err(publication) => match self.fail_with_diagnostic(
                user_id,
                attempt_id,
                "publication-failed",
                &publication.to_string(),
            ) {
                Ok(_) => Err(publication),
                Err(recovery) => Err(MaterializationStoreError::PublicationRecoveryFailed {
                    publication: publication.to_string(),
                    recovery: recovery.to_string(),
                }),
            },
        }
    }

    pub fn fail(
        &self,
        user_id: &str,
        attempt_id: &str,
        failure_code: &str,
    ) -> Result<MaterializationAttempt, MaterializationStoreError> {
        self.fail_with_diagnostic(user_id, attempt_id, failure_code, failure_code)
    }

    pub fn cancel(
        &self,
        user_id: &str,
        attempt_id: &str,
    ) -> Result<MaterializationAttempt, MaterializationStoreError> {
        self.fail_with_diagnostic(
            user_id,
            attempt_id,
            "cancelled",
            "Materialization cancelled",
        )
    }

    pub fn dataset(
        &self,
        user_id: &str,
        dataset_id: &str,
    ) -> Result<FeatureDataset, MaterializationStoreError> {
        validate_user(user_id)?;
        let database = self.lock_database()?;
        let dataset = load_dataset(&database, user_id, dataset_id, &self.dataset_directory)?;
        validate_manifest(&dataset)?;
        Ok(dataset)
    }

    pub fn datasets(
        &self,
        user_id: &str,
    ) -> Result<Vec<FeatureDataset>, MaterializationStoreError> {
        validate_user(user_id)?;
        let database = self.lock_database()?;
        let mut statement = database
            .prepare(
                "SELECT d.dataset_id, d.user_id, d.request_hash, d.manifest_json,
                        c.content_sha256, c.parquet_path, c.byte_size, d.created_at_ms
                 FROM feature_datasets d
                 JOIN feature_dataset_contents c ON c.content_sha256 = d.content_sha256
                 JOIN feature_dataset_access a ON a.dataset_id = d.dataset_id
                 WHERE a.user_id = ?1
                 ORDER BY d.created_at_ms DESC, d.dataset_id",
            )
            .map_err(sqlite_error)?;
        let rows = statement
            .query_map([user_id], |row| {
                row_to_dataset(row, &self.dataset_directory)
            })
            .map_err(sqlite_error)?;
        let datasets = rows
            .map(|row| row.map_err(sqlite_error))
            .collect::<Result<Vec<_>, _>>()?;
        for dataset in &datasets {
            validate_manifest(dataset)?;
        }
        Ok(datasets)
    }

    pub fn summary(
        &self,
        user_id: &str,
        dataset_id: &str,
    ) -> Result<Vec<FeatureOutputSummary>, MaterializationStoreError> {
        let dataset = self.dataset(user_id, dataset_id)?;
        let rows = self.read_rows(&dataset)?;
        summarize(&dataset.manifest, &rows)
    }

    pub fn page(
        &self,
        user_id: &str,
        dataset_id: &str,
        filter: FeatureDatasetFilter,
        offset: usize,
    ) -> Result<FeatureDatasetPage, MaterializationStoreError> {
        let dataset = self.dataset(user_id, dataset_id)?;
        if sha256_file(&dataset.parquet_path)? != dataset.manifest.content_sha256 {
            return Err(MaterializationStoreError::DatasetContentCollision);
        }
        let names = dataset
            .manifest
            .outputs
            .iter()
            .map(|output| output.output_name.clone())
            .collect::<Vec<_>>();
        let (page, row_count) = read_parquet_page(&dataset.parquet_path, &names, &filter, offset)?;
        if row_count != dataset.manifest.row_count {
            return Err(MaterializationStoreError::InvalidObservation(
                "manifest-row-count-mismatch".into(),
            ));
        }
        Ok(page)
    }

    pub fn reference_dataset(
        &self,
        owner_user_id: &str,
        dataset_id: &str,
        referencing_user_id: &str,
        reference_id: &str,
    ) -> Result<(), MaterializationStoreError> {
        validate_user(owner_user_id)?;
        validate_user(referencing_user_id)?;
        if reference_id.trim().is_empty() {
            return Err(MaterializationStoreError::Unauthorized);
        }
        let database = self.lock_database()?;
        let transaction = database.unchecked_transaction().map_err(sqlite_error)?;
        ensure_dataset_access(&transaction, owner_user_id, dataset_id)?;
        transaction
            .execute(
                "INSERT OR IGNORE INTO feature_dataset_access(user_id, dataset_id)
                 VALUES (?1, ?2)",
                params![referencing_user_id, dataset_id],
            )
            .map_err(sqlite_error)?;
        transaction
            .execute(
                "INSERT INTO feature_dataset_references(dataset_id, referencing_user_id, reference_id)
                 VALUES (?1, ?2, ?3)",
                params![dataset_id, referencing_user_id, reference_id],
            )
            .map_err(|error| {
                if error.sqlite_error_code() == Some(rusqlite::ErrorCode::ConstraintViolation) {
                    MaterializationStoreError::Unauthorized
                } else {
                    sqlite_error(error)
                }
            })?;
        transaction.commit().map_err(sqlite_error)
    }

    pub fn unreference_dataset(
        &self,
        referencing_user_id: &str,
        dataset_id: &str,
        reference_id: &str,
    ) -> Result<(), MaterializationStoreError> {
        validate_user(referencing_user_id)?;
        let database = self.lock_database()?;
        let removed = database
            .execute(
                "DELETE FROM feature_dataset_references
                 WHERE dataset_id = ?1 AND referencing_user_id = ?2 AND reference_id = ?3",
                params![dataset_id, referencing_user_id, reference_id],
            )
            .map_err(sqlite_error)?;
        if removed == 0 {
            return Err(MaterializationStoreError::DatasetNotFound);
        }
        Ok(())
    }

    pub fn delete_dataset(
        &self,
        user_id: &str,
        dataset_id: &str,
    ) -> Result<(), MaterializationStoreError> {
        validate_user(user_id)?;
        let database = self.lock_database()?;
        let transaction = database.unchecked_transaction().map_err(sqlite_error)?;
        ensure_dataset_access(&transaction, user_id, dataset_id)?;
        let referenced: i64 = transaction
            .query_row(
                "SELECT COUNT(*) FROM feature_dataset_references WHERE dataset_id = ?1",
                [dataset_id],
                |row| row.get(0),
            )
            .map_err(sqlite_error)?;
        if referenced != 0 {
            return Err(MaterializationStoreError::DatasetReferenced);
        }
        let content_sha256: String = transaction
            .query_row(
                "SELECT content_sha256 FROM feature_datasets WHERE dataset_id = ?1",
                [dataset_id],
                |row| row.get(0),
            )
            .map_err(sqlite_error)?;
        transaction
            .execute(
                "DELETE FROM feature_dataset_access WHERE user_id = ?1 AND dataset_id = ?2",
                params![user_id, dataset_id],
            )
            .map_err(sqlite_error)?;
        let remaining_access: i64 = transaction
            .query_row(
                "SELECT COUNT(*) FROM feature_dataset_access WHERE dataset_id = ?1",
                [dataset_id],
                |row| row.get(0),
            )
            .map_err(sqlite_error)?;
        if remaining_access == 0 {
            transaction
                .execute(
                    "DELETE FROM feature_datasets WHERE dataset_id = ?1",
                    [dataset_id],
                )
                .map_err(sqlite_error)?;
        }
        let remaining_content: i64 = transaction
            .query_row(
                "SELECT COUNT(*) FROM feature_datasets WHERE content_sha256 = ?1",
                [content_sha256.as_str()],
                |row| row.get(0),
            )
            .map_err(sqlite_error)?;
        if remaining_content == 0 {
            let path: String = transaction
                .query_row(
                    "SELECT parquet_path FROM feature_dataset_contents WHERE content_sha256 = ?1",
                    [content_sha256.as_str()],
                    |row| row.get(0),
                )
                .map_err(sqlite_error)?;
            if Path::new(&path) != content_path(&self.dataset_directory, &content_sha256).as_path()
            {
                return Err(MaterializationStoreError::IncompatibleSchema {
                    stored_schema_version: Some(FEATURE_DATASET_STORAGE_SCHEMA_VERSION.into()),
                    table: Some("feature_dataset_contents.parquet_path".into()),
                });
            }
            transaction
                .execute(
                    "INSERT OR IGNORE INTO feature_dataset_deletions(
                         content_sha256, parquet_path, requested_at_ms
                     ) VALUES (?1, ?2, ?3)",
                    params![content_sha256, path, now_ms()],
                )
                .map_err(sqlite_error)?;
        }
        transaction.commit().map_err(sqlite_error)?;
        drop(database);
        self.recover_pending_deletions()
    }

    fn start_internal(
        &self,
        request: FeatureMaterializationRequest,
        evidence: PlanEvidence,
    ) -> Result<MaterializationAttempt, MaterializationStoreError> {
        validate_request(&request)?;
        let request_hash = request.request_hash();
        let request_json = json_string(&request)?;
        let PlanEvidence {
            plan_json,
            artifact_ids,
            engine_identity,
            output_names,
        } = evidence;
        let artifact_ids_json = json_string(&artifact_ids)?;
        let engine_identity_json = json_string(&engine_identity)?;
        let output_names_json = json_string(&output_names)?;
        let database = self.lock_database()?;
        let transaction = database.unchecked_transaction().map_err(sqlite_error)?;
        if let Some(existing) = transaction
            .query_row(
                "SELECT request_json, plan_json, artifact_ids_json,
                        engine_identity_json, output_names_json
                 FROM feature_materialization_requests WHERE request_hash = ?1",
                [&request_hash],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(sqlite_error)?
        {
            if existing
                != (
                    request_json.clone(),
                    plan_json.clone(),
                    artifact_ids_json.clone(),
                    engine_identity_json.clone(),
                    output_names_json.clone(),
                )
            {
                return Err(MaterializationStoreError::InvalidRequest);
            }
        } else {
            transaction
                .execute(
                    "INSERT INTO feature_materialization_requests(
                         request_hash, user_id, request_json, plan_json,
                         artifact_ids_json, engine_identity_json, output_names_json,
                         created_at_ms
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        request_hash,
                        request.user_id,
                        request_json,
                        plan_json,
                        artifact_ids_json,
                        engine_identity_json,
                        output_names_json,
                        now_ms()
                    ],
                )
                .map_err(sqlite_error)?;
        }
        if let Some(existing) = transaction
            .query_row(
                "SELECT a.attempt_id
                 FROM feature_materialization_attempts a
                 WHERE a.user_id = ?1 AND a.request_hash = ?2
                   AND (a.status IN (?3, ?4)
                        OR (a.status = ?5 AND a.dataset_id IS NOT NULL
                            AND EXISTS (
                                SELECT 1 FROM feature_dataset_access access
                                WHERE access.user_id = ?1 AND access.dataset_id = a.dataset_id
                            )))
                 ORDER BY a.queue_sequence LIMIT 1",
                params![
                    request.user_id,
                    request_hash,
                    STATUS_PENDING,
                    STATUS_RUNNING,
                    STATUS_COMPLETED
                ],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(sqlite_error)?
        {
            let attempt = load_attempt(&transaction, &request.user_id, &existing)?;
            transaction.commit().map_err(sqlite_error)?;
            return Ok(attempt);
        }
        let attempt = new_attempt(&request.user_id, &request_hash, None);
        insert_attempt(&transaction, &attempt).map_err(sqlite_error)?;
        transaction.commit().map_err(sqlite_error)?;
        Ok(attempt)
    }

    fn publish_inner(
        &self,
        user_id: &str,
        attempt_id: &str,
    ) -> Result<MaterializationAttempt, MaterializationStoreError> {
        validate_user(user_id)?;
        self.recover_pending_deletions()?;
        let (request, plan_json, artifact_ids, engine_identity, names, staging_path) = {
            let database = self.lock_database()?;
            let attempt = load_attempt(&database, user_id, attempt_id)?;
            if attempt.status != MaterializationAttemptStatus::Running {
                return Err(MaterializationStoreError::InvalidTransition);
            }
            let (request_json, plan_json, artifact_ids_json, engine_identity_json): (
                String,
                String,
                String,
                String,
            ) = database
                .query_row(
                    "SELECT r.request_json, r.plan_json, r.artifact_ids_json, r.engine_identity_json
                     FROM feature_materialization_requests r
                     WHERE r.request_hash = ?1",
                    [attempt.request_hash.as_str()],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                        ))
                    },
                )
                .map_err(sqlite_error)?;
            let (names_json, staging_path): (String, Option<String>) = database
                .query_row(
                    "SELECT output_names_json, staging_path
                     FROM feature_materialization_attempts WHERE attempt_id = ?1",
                    [attempt_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(sqlite_error)?;
            let staging_path = staging_path.ok_or(MaterializationStoreError::StagingNotFound)?;
            let staging_path = safe_staged_parquet_path(
                Path::new(&staging_path),
                attempt_id,
                &self.staging_directory(),
            )
            .ok_or(MaterializationStoreError::StagingNotFound)?;
            (
                serde_json::from_str::<FeatureMaterializationRequest>(&request_json)
                    .map_err(|error| MaterializationStoreError::Json(error.to_string()))?,
                serde_json::from_str(&plan_json)
                    .map_err(|error| MaterializationStoreError::Json(error.to_string()))?,
                serde_json::from_str(&artifact_ids_json)
                    .map_err(|error| MaterializationStoreError::Json(error.to_string()))?,
                serde_json::from_str(&engine_identity_json)
                    .map_err(|error| MaterializationStoreError::Json(error.to_string()))?,
                serde_json::from_str::<Vec<String>>(&names_json)
                    .map_err(|error| MaterializationStoreError::Json(error.to_string()))?,
                staging_path,
            )
        };
        let rows = read_parquet(&staging_path, &names)?;
        let content_sha256 = sha256_file(&staging_path)?;
        let outputs = names
            .iter()
            .map(|name| output_manifest(name))
            .collect::<Vec<_>>();
        let manifest = FeatureDatasetManifest {
            manifest_schema_version: FEATURE_DATASET_MANIFEST_SCHEMA_VERSION.into(),
            request: request.clone(),
            request_hash: request.request_hash(),
            plan_json,
            artifact_ids,
            engine_identity,
            reason_version: FEATURE_UNAVAILABILITY_REASON_VERSION.into(),
            outputs,
            row_count: rows.len() as u64,
            content_sha256: content_sha256.clone(),
        };
        let manifest_json = json_string(&manifest)?;
        let dataset_id = sha256_hex(manifest_json.as_bytes());
        let final_path = content_path(&self.dataset_directory, &content_sha256);
        validate_manifest(&FeatureDataset {
            dataset_id: dataset_id.clone(),
            user_id: user_id.to_owned(),
            request_hash: request.request_hash(),
            manifest: manifest.clone(),
            parquet_path: final_path.clone(),
            content_byte_size: 0,
            created_at_ms: 0,
        })?;
        fs::create_dir_all(&self.dataset_directory).map_err(io_error)?;
        let final_exists = final_path.exists();
        if final_exists && sha256_file(&final_path)? != content_sha256 {
            return Err(MaterializationStoreError::DatasetContentCollision);
        }
        let database = self.lock_database()?;
        let claimed = database
            .execute(
                "UPDATE feature_materialization_attempts
                 SET publication_path = ?1, publication_content_sha256 = ?2,
                     publication_owned = ?3, updated_at_ms = ?4
                 WHERE attempt_id = ?5 AND user_id = ?6 AND status = ?7
                   AND staging_path IS NOT NULL AND publication_path IS NULL",
                params![
                    final_path.to_string_lossy().as_ref(),
                    content_sha256,
                    i64::from(!final_exists),
                    now_ms(),
                    attempt_id,
                    user_id,
                    STATUS_RUNNING
                ],
            )
            .map_err(sqlite_error)?;
        if claimed != 1 {
            return Err(MaterializationStoreError::PublicationInProgress);
        }
        drop(database);
        if final_exists {
            remove_file_if_exists(&staging_path)?;
        } else if let Err(error) = fs::rename(&staging_path, &final_path) {
            if final_path.exists() {
                self.clear_publication_path(user_id, attempt_id)?;
                if final_path.is_file() && sha256_file(&final_path)? == content_sha256 {
                    remove_file_if_exists(&staging_path)?;
                } else {
                    return Err(MaterializationStoreError::DatasetContentCollision);
                }
            } else {
                return Err(io_error(error));
            }
        }
        sync_published_file(&final_path)?;
        let database = self.lock_database()?;
        let transaction = database.unchecked_transaction().map_err(sqlite_error)?;
        let status: String = transaction
            .query_row(
                "SELECT status FROM feature_materialization_attempts
                 WHERE attempt_id = ?1 AND user_id = ?2",
                params![attempt_id, user_id],
                |row| row.get(0),
            )
            .map_err(sqlite_error)?;
        if status != STATUS_RUNNING {
            return Err(MaterializationStoreError::InvalidTransition);
        }
        let byte_size = fs::metadata(&final_path).map_err(io_error)?.len() as i64;
        transaction
            .execute(
                "INSERT OR IGNORE INTO feature_dataset_contents(content_sha256, parquet_path, byte_size)
                 VALUES (?1, ?2, ?3)",
                params![content_sha256, final_path.to_string_lossy().as_ref(), byte_size],
            )
            .map_err(sqlite_error)?;
        transaction
            .execute(
                "INSERT INTO feature_datasets(
                     dataset_id, user_id, request_hash, manifest_json,
                     content_sha256, row_count, created_at_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    dataset_id,
                    user_id,
                    request.request_hash(),
                    manifest_json,
                    content_sha256,
                    rows.len() as i64,
                    now_ms()
                ],
            )
            .map_err(sqlite_error)?;
        transaction
            .execute(
                "INSERT INTO feature_dataset_access(user_id, dataset_id) VALUES (?1, ?2)",
                params![user_id, dataset_id],
            )
            .map_err(sqlite_error)?;
        transaction
            .execute(
                "UPDATE feature_materialization_attempts
                 SET status = ?1, dataset_id = ?2, staging_path = NULL,
                     publication_path = NULL, publication_content_sha256 = NULL,
                     publication_owned = 0,
                     failure_code = NULL, diagnostic = NULL,
                     progress_completed = ?3, progress_total = ?3, updated_at_ms = ?4
                 WHERE attempt_id = ?5 AND user_id = ?6 AND status = ?7",
                params![
                    STATUS_COMPLETED,
                    dataset_id,
                    rows.len() as i64,
                    now_ms(),
                    attempt_id,
                    user_id,
                    STATUS_RUNNING
                ],
            )
            .map_err(sqlite_error)?;
        transaction.commit().map_err(sqlite_error)?;
        drop(database);
        self.attempt(user_id, attempt_id)
    }

    fn fail_with_diagnostic(
        &self,
        user_id: &str,
        attempt_id: &str,
        failure_code: &str,
        diagnostic: &str,
    ) -> Result<MaterializationAttempt, MaterializationStoreError> {
        validate_user(user_id)?;
        let database = self.lock_database()?;
        let transaction = database.unchecked_transaction().map_err(sqlite_error)?;
        let current = load_attempt(&transaction, user_id, attempt_id)?;
        let next_status = if failure_code == "cancelled" {
            STATUS_CANCELLED
        } else {
            STATUS_FAILED
        };
        if !matches!(
            current.status,
            MaterializationAttemptStatus::Pending | MaterializationAttemptStatus::Running
        ) {
            return Err(MaterializationStoreError::InvalidTransition);
        }
        let (staging_path, publication_path, publication_content_sha256, publication_owned): (
            Option<String>,
            Option<String>,
            Option<String>,
            i64,
        ) = transaction
            .query_row(
                "SELECT staging_path, publication_path, publication_content_sha256,
                        publication_owned
                 FROM feature_materialization_attempts
                 WHERE attempt_id = ?1 AND user_id = ?2",
                params![attempt_id, user_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .map_err(sqlite_error)?;
        if let Some(path) = staging_path.as_deref().and_then(|path| {
            safe_staging_path(Path::new(path), attempt_id, &self.staging_directory())
        }) {
            cleanup_staging_files(&path, attempt_id, &self.staging_directory())?;
        }
        if publication_owned != 0
            && let Some(path) = publication_path
                .as_deref()
                .zip(publication_content_sha256.as_deref())
                .and_then(|(path, content_sha256)| {
                    safe_publication_path(Path::new(path), content_sha256, &self.dataset_directory)
                })
        {
            remove_file_if_exists(&path)?;
        }
        transaction
            .execute(
                "UPDATE feature_materialization_attempts
                 SET status = ?1, failure_code = ?2, diagnostic = ?3,
                     staging_path = NULL, publication_path = NULL,
                     publication_content_sha256 = NULL, publication_owned = 0,
                     updated_at_ms = ?4
                 WHERE attempt_id = ?5 AND user_id = ?6
                   AND status IN (?7, ?8)",
                params![
                    next_status,
                    failure_code,
                    diagnostic,
                    now_ms(),
                    attempt_id,
                    user_id,
                    STATUS_PENDING,
                    STATUS_RUNNING
                ],
            )
            .map_err(sqlite_error)?;
        transaction.commit().map_err(sqlite_error)?;
        drop(database);
        self.attempt(user_id, attempt_id)
    }

    fn read_rows(
        &self,
        dataset: &FeatureDataset,
    ) -> Result<Vec<FeatureDatasetRow>, MaterializationStoreError> {
        if sha256_file(&dataset.parquet_path)? != dataset.manifest.content_sha256 {
            return Err(MaterializationStoreError::DatasetContentCollision);
        }
        let names = dataset
            .manifest
            .outputs
            .iter()
            .map(|output| output.output_name.clone())
            .collect::<Vec<_>>();
        let rows = read_parquet(&dataset.parquet_path, &names)?;
        if rows.len() as u64 != dataset.manifest.row_count {
            return Err(MaterializationStoreError::InvalidObservation(
                "manifest-row-count-mismatch".into(),
            ));
        }
        Ok(rows)
    }

    fn recover_pending_deletions(&self) -> Result<(), MaterializationStoreError> {
        let database = self.lock_database()?;
        let transaction = database.unchecked_transaction().map_err(sqlite_error)?;
        let mut statement = transaction
            .prepare(
                "SELECT content_sha256, parquet_path
                 FROM feature_dataset_deletions
                 ORDER BY requested_at_ms, content_sha256",
            )
            .map_err(sqlite_error)?;
        let pending = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(sqlite_error)?
            .map(|row| row.map_err(sqlite_error))
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        for (content_sha256, stored_path) in pending {
            let expected_path = content_path(&self.dataset_directory, &content_sha256);
            if !is_sha256(&content_sha256) || Path::new(&stored_path) != expected_path.as_path() {
                return Err(MaterializationStoreError::IncompatibleSchema {
                    stored_schema_version: Some(FEATURE_DATASET_STORAGE_SCHEMA_VERSION.into()),
                    table: Some("feature_dataset_deletions.parquet_path".into()),
                });
            }
            transaction
                .execute(
                    "UPDATE feature_dataset_deletions
                     SET requested_at_ms = requested_at_ms
                     WHERE content_sha256 = ?1",
                    [&content_sha256],
                )
                .map_err(sqlite_error)?;
            remove_file_if_exists(&expected_path)?;
            transaction
                .execute(
                    "DELETE FROM feature_dataset_contents
                     WHERE content_sha256 = ?1
                       AND NOT EXISTS (
                           SELECT 1 FROM feature_datasets
                           WHERE feature_datasets.content_sha256 = ?1
                       )",
                    [&content_sha256],
                )
                .map_err(sqlite_error)?;
            transaction
                .execute(
                    "DELETE FROM feature_dataset_deletions WHERE content_sha256 = ?1",
                    [&content_sha256],
                )
                .map_err(sqlite_error)?;
        }
        transaction.commit().map_err(sqlite_error)
    }

    fn clear_publication_path(
        &self,
        user_id: &str,
        attempt_id: &str,
    ) -> Result<(), MaterializationStoreError> {
        let database = self.lock_database()?;
        database
            .execute(
                "UPDATE feature_materialization_attempts
                 SET publication_path = NULL, publication_content_sha256 = NULL,
                     publication_owned = 0,
                     updated_at_ms = ?1
                 WHERE attempt_id = ?2 AND user_id = ?3",
                params![now_ms(), attempt_id, user_id],
            )
            .map_err(sqlite_error)?;
        Ok(())
    }

    fn recover_stale_running_attempts(&self) -> Result<(), MaterializationStoreError> {
        let database = self.lock_database()?;
        let mut statement = database
            .prepare(
                "SELECT attempt_id, staging_path, publication_path,
                        publication_content_sha256, publication_owned
                 FROM feature_materialization_attempts
                 WHERE status = ?1",
            )
            .map_err(sqlite_error)?;
        let rows = statement
            .query_map([STATUS_RUNNING], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })
            .map_err(sqlite_error)?;
        let stale = rows
            .map(|row| row.map_err(sqlite_error))
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        for (
            attempt_id,
            staging_path,
            publication_path,
            publication_content_sha256,
            publication_owned,
        ) in &stale
        {
            if let Some(path) = staging_path.as_deref().and_then(|path| {
                safe_staging_path(Path::new(path), attempt_id, &self.staging_directory())
            }) {
                cleanup_staging_files(&path, attempt_id, &self.staging_directory())?;
            }
            if *publication_owned != 0
                && let Some(path) = publication_path
                    .as_deref()
                    .zip(publication_content_sha256.as_deref())
                    .and_then(|(path, content_sha256)| {
                        safe_publication_path(
                            Path::new(path),
                            content_sha256,
                            &self.dataset_directory,
                        )
                    })
            {
                remove_file_if_exists(&path)?;
            }
            database
                .execute(
                    "UPDATE feature_materialization_attempts
                     SET status = ?1, failure_code = ?2, diagnostic = ?3,
                         staging_path = NULL, publication_path = NULL,
                         publication_content_sha256 = NULL, publication_owned = 0,
                         updated_at_ms = ?4
                     WHERE attempt_id = ?5 AND status = ?6",
                    params![
                        STATUS_FAILED,
                        "interrupted",
                        "Running materialization was interrupted before publication",
                        now_ms(),
                        attempt_id,
                        STATUS_RUNNING
                    ],
                )
                .map_err(sqlite_error)?;
        }
        Ok(())
    }

    fn initialize_schema(&self) -> Result<(), MaterializationStoreError> {
        let database = self.lock_database()?;
        let schema_exists = table_exists(&database, "feature_materialization_schema")?;
        let feature_tables = [
            "feature_materialization_requests",
            "feature_materialization_attempts",
            "feature_dataset_contents",
            "feature_datasets",
            "feature_dataset_access",
            "feature_dataset_references",
            "feature_dataset_deletions",
        ];
        if schema_exists {
            if !table_columns(&database, "feature_materialization_schema")?
                .contains("schema_version")
            {
                return Err(MaterializationStoreError::ResetRequired {
                    stored_schema_version: None,
                    table: Some("feature_materialization_schema".into()),
                });
            }
            let version: Option<String> = database
                .query_row(
                    "SELECT schema_version FROM feature_materialization_schema LIMIT 1",
                    [],
                    |row| row.get(0),
                )
                .optional()
                .map_err(sqlite_error)?;
            if version.as_deref() != Some(FEATURE_DATASET_STORAGE_SCHEMA_VERSION) {
                return Err(MaterializationStoreError::ResetRequired {
                    stored_schema_version: version,
                    table: Some("feature_materialization_schema".into()),
                });
            }
            for table in feature_tables {
                if !table_exists(&database, table)? {
                    return Err(MaterializationStoreError::ResetRequired {
                        stored_schema_version: Some(FEATURE_DATASET_STORAGE_SCHEMA_VERSION.into()),
                        table: Some(table.into()),
                    });
                }
            }
        } else {
            for table in feature_tables {
                if table_exists(&database, table)? {
                    return Err(MaterializationStoreError::ResetRequired {
                        stored_schema_version: None,
                        table: Some("feature_materialization_schema".into()),
                    });
                }
            }
        }
        let required_columns = [
            (
                "feature_materialization_requests",
                [
                    "request_hash",
                    "user_id",
                    "request_json",
                    "plan_json",
                    "artifact_ids_json",
                    "engine_identity_json",
                    "output_names_json",
                    "created_at_ms",
                ]
                .as_slice(),
            ),
            (
                "feature_materialization_attempts",
                [
                    "queue_sequence",
                    "attempt_id",
                    "request_hash",
                    "user_id",
                    "status",
                    "source_attempt_id",
                    "dataset_id",
                    "failure_code",
                    "diagnostic",
                    "progress_completed",
                    "progress_total",
                    "output_names_json",
                    "staging_path",
                    "publication_path",
                    "publication_content_sha256",
                    "publication_owned",
                    "created_at_ms",
                    "updated_at_ms",
                ]
                .as_slice(),
            ),
            (
                "feature_dataset_contents",
                ["content_sha256", "parquet_path", "byte_size"].as_slice(),
            ),
            (
                "feature_datasets",
                [
                    "dataset_id",
                    "user_id",
                    "request_hash",
                    "manifest_json",
                    "content_sha256",
                    "row_count",
                    "created_at_ms",
                ]
                .as_slice(),
            ),
            (
                "feature_dataset_access",
                ["user_id", "dataset_id"].as_slice(),
            ),
            (
                "feature_dataset_references",
                ["dataset_id", "referencing_user_id", "reference_id"].as_slice(),
            ),
            (
                "feature_dataset_deletions",
                ["content_sha256", "parquet_path", "requested_at_ms"].as_slice(),
            ),
        ];
        for (table, columns) in required_columns {
            if table_exists(&database, table)? {
                let actual = table_columns(&database, table)?;
                if columns.iter().any(|column| !actual.contains(*column)) {
                    return Err(MaterializationStoreError::ResetRequired {
                        stored_schema_version: Some(FEATURE_DATASET_STORAGE_SCHEMA_VERSION.into()),
                        table: Some(table.into()),
                    });
                }
            }
        }
        database
            .execute_batch(
                "PRAGMA foreign_keys = ON;
                 CREATE TABLE IF NOT EXISTS feature_materialization_schema (
                    schema_version TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS feature_materialization_requests (
                    request_hash TEXT PRIMARY KEY,
                    user_id TEXT NOT NULL,
                    request_json TEXT NOT NULL,
                    plan_json TEXT NOT NULL,
                    artifact_ids_json TEXT NOT NULL,
                    engine_identity_json TEXT NOT NULL,
                    output_names_json TEXT NOT NULL,
                    created_at_ms INTEGER NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS feature_materialization_attempts (
                    queue_sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                    attempt_id TEXT NOT NULL UNIQUE,
                    request_hash TEXT NOT NULL REFERENCES feature_materialization_requests(request_hash),
                    user_id TEXT NOT NULL,
                    status TEXT NOT NULL,
                    source_attempt_id TEXT,
                    dataset_id TEXT,
                    failure_code TEXT,
                    diagnostic TEXT,
                    progress_completed INTEGER NOT NULL,
                    progress_total INTEGER NOT NULL,
                    output_names_json TEXT,
                    staging_path TEXT,
                    publication_path TEXT,
                    publication_content_sha256 TEXT,
                    publication_owned INTEGER NOT NULL DEFAULT 0,
                    created_at_ms INTEGER NOT NULL,
                    updated_at_ms INTEGER NOT NULL
                 );
                 CREATE INDEX IF NOT EXISTS feature_materialization_attempts_fifo
                    ON feature_materialization_attempts(status, queue_sequence);
                 CREATE TABLE IF NOT EXISTS feature_dataset_contents (
                    content_sha256 TEXT PRIMARY KEY,
                    parquet_path TEXT NOT NULL,
                    byte_size INTEGER NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS feature_datasets (
                    dataset_id TEXT PRIMARY KEY,
                    user_id TEXT NOT NULL,
                    request_hash TEXT NOT NULL REFERENCES feature_materialization_requests(request_hash),
                    manifest_json TEXT NOT NULL,
                    content_sha256 TEXT NOT NULL REFERENCES feature_dataset_contents(content_sha256),
                    row_count INTEGER NOT NULL,
                    created_at_ms INTEGER NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS feature_dataset_access (
                    user_id TEXT NOT NULL,
                    dataset_id TEXT NOT NULL REFERENCES feature_datasets(dataset_id) ON DELETE CASCADE,
                    PRIMARY KEY(user_id, dataset_id)
                 );
                 CREATE TABLE IF NOT EXISTS feature_dataset_references (
                    dataset_id TEXT NOT NULL REFERENCES feature_datasets(dataset_id) ON DELETE CASCADE,
                    referencing_user_id TEXT NOT NULL,
                    reference_id TEXT NOT NULL,
                    PRIMARY KEY(dataset_id, referencing_user_id, reference_id)
                 );
                 CREATE TABLE IF NOT EXISTS feature_dataset_deletions (
                    content_sha256 TEXT PRIMARY KEY,
                    parquet_path TEXT NOT NULL,
                    requested_at_ms INTEGER NOT NULL
                 );",
            )
            .map_err(sqlite_error)?;
        if !schema_exists {
            database
                .execute(
                    "INSERT INTO feature_materialization_schema(schema_version) VALUES (?1)",
                    [FEATURE_DATASET_STORAGE_SCHEMA_VERSION],
                )
                .map_err(sqlite_error)?;
        }
        Ok(())
    }

    fn lock_database(&self) -> Result<MutexGuard<'_, Connection>, MaterializationStoreError> {
        self.database
            .lock()
            .map_err(|_| MaterializationStoreError::Sqlite("database mutex poisoned".into()))
    }

    fn staging_directory(&self) -> PathBuf {
        self.dataset_directory.join("staging")
    }
}

fn new_attempt(
    user_id: &str,
    request_hash: &str,
    source_attempt_id: Option<String>,
) -> MaterializationAttempt {
    let now = now_ms();
    MaterializationAttempt {
        attempt_id: Uuid::new_v4().to_string(),
        user_id: user_id.into(),
        request_hash: request_hash.into(),
        status: MaterializationAttemptStatus::Pending,
        source_attempt_id,
        dataset_id: None,
        failure_code: None,
        diagnostic: None,
        progress_completed: 0,
        progress_total: 0,
        created_at_ms: now,
        updated_at_ms: now,
    }
}

fn insert_attempt(
    database: &Connection,
    attempt: &MaterializationAttempt,
) -> rusqlite::Result<usize> {
    database.execute(
        "INSERT INTO feature_materialization_attempts(
             attempt_id, request_hash, user_id, status, source_attempt_id,
             dataset_id, failure_code, diagnostic, progress_completed,
             progress_total, output_names_json, staging_path, created_at_ms, updated_at_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, NULL, 0, 0, NULL, NULL, ?6, ?6)",
        params![
            attempt.attempt_id,
            attempt.request_hash,
            attempt.user_id,
            attempt.status.as_str(),
            attempt.source_attempt_id,
            attempt.created_at_ms
        ],
    )
}

fn load_attempt(
    database: &Connection,
    user_id: &str,
    attempt_id: &str,
) -> Result<MaterializationAttempt, MaterializationStoreError> {
    database
        .query_row(
            "SELECT attempt_id, user_id, request_hash, status, source_attempt_id,
                    dataset_id, failure_code, diagnostic, progress_completed,
                    progress_total, created_at_ms, updated_at_ms
             FROM feature_materialization_attempts
             WHERE attempt_id = ?1 AND user_id = ?2",
            params![attempt_id, user_id],
            row_to_attempt,
        )
        .optional()
        .map_err(sqlite_error)?
        .ok_or(MaterializationStoreError::AttemptNotFound)
}

fn row_to_attempt(row: &rusqlite::Row<'_>) -> rusqlite::Result<MaterializationAttempt> {
    let status: String = row.get(3)?;
    let status = MaterializationAttemptStatus::try_from(status.as_str())
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    Ok(MaterializationAttempt {
        attempt_id: row.get(0)?,
        user_id: row.get(1)?,
        request_hash: row.get(2)?,
        status,
        source_attempt_id: row.get(4)?,
        dataset_id: row.get(5)?,
        failure_code: row.get(6)?,
        diagnostic: row.get(7)?,
        progress_completed: row.get::<_, i64>(8)? as u64,
        progress_total: row.get::<_, i64>(9)? as u64,
        created_at_ms: row.get(10)?,
        updated_at_ms: row.get(11)?,
    })
}

fn load_dataset(
    database: &Connection,
    user_id: &str,
    dataset_id: &str,
    dataset_directory: &Path,
) -> Result<FeatureDataset, MaterializationStoreError> {
    database
        .query_row(
            "SELECT d.dataset_id, d.user_id, d.request_hash, d.manifest_json,
                    c.content_sha256, c.parquet_path, c.byte_size, d.created_at_ms
             FROM feature_datasets d
             JOIN feature_dataset_contents c ON c.content_sha256 = d.content_sha256
             JOIN feature_dataset_access a ON a.dataset_id = d.dataset_id
             WHERE a.user_id = ?1 AND d.dataset_id = ?2",
            params![user_id, dataset_id],
            |row| row_to_dataset(row, dataset_directory),
        )
        .optional()
        .map_err(sqlite_error)?
        .ok_or(MaterializationStoreError::DatasetNotFound)
}

fn validate_manifest(dataset: &FeatureDataset) -> Result<(), MaterializationStoreError> {
    let manifest_json = json_string(&dataset.manifest)?;
    let mut output_names = BTreeSet::new();
    let expected_filename = format!("{}.parquet", dataset.manifest.content_sha256);
    let basic_valid = dataset.manifest.manifest_schema_version
        == FEATURE_DATASET_MANIFEST_SCHEMA_VERSION
        && dataset.manifest.reason_version == FEATURE_UNAVAILABILITY_REASON_VERSION
        && dataset.manifest.request_hash == dataset.manifest.request.request_hash()
        && dataset.manifest.request_hash == dataset.request_hash
        && sha256_hex(manifest_json.as_bytes()) == dataset.dataset_id
        && dataset.manifest.request.user_id == dataset.user_id
        && dataset.manifest.request.engine_identity.as_ref()
            == Some(&dataset.manifest.engine_identity)
        && dataset.manifest.request.artifact_ids == dataset.manifest.artifact_ids
        && is_sha256(&dataset.manifest.content_sha256)
        && dataset
            .parquet_path
            .file_name()
            .and_then(|name| name.to_str())
            == Some(expected_filename.as_str())
        && !dataset.manifest.outputs.is_empty();
    if !basic_valid
        || dataset.manifest.outputs.iter().any(|output| {
            !is_lower_kebab(&output.output_name)
                || !output_names.insert(&output.output_name)
                || *output != output_manifest(&output.output_name)
        })
    {
        return Err(MaterializationStoreError::InvalidOutputSchema);
    }
    let plan_json = json_string(&dataset.manifest.plan_json)?;
    let plan =
        FeaturePlan::load_for_engine(plan_json.as_bytes(), &dataset.manifest.engine_identity)
            .map_err(|_| MaterializationStoreError::InvalidOutputSchema)?;
    if plan.plan_hash() != dataset.manifest.request.feature_plan_hash
        || plan_output_names(&plan)
            != dataset
                .manifest
                .outputs
                .iter()
                .map(|output| output.output_name.clone())
                .collect::<Vec<_>>()
        || plan
            .artifacts()
            .iter()
            .map(|artifact| artifact.artifact_id.clone())
            .collect::<Vec<_>>()
            != dataset.manifest.artifact_ids
    {
        return Err(MaterializationStoreError::InvalidOutputSchema);
    }
    Ok(())
}

fn row_to_dataset(
    row: &rusqlite::Row<'_>,
    dataset_directory: &Path,
) -> rusqlite::Result<FeatureDataset> {
    let manifest_json: String = row.get(3)?;
    let manifest: FeatureDatasetManifest =
        serde_json::from_str(&manifest_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                3,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
    let content_sha256: String = row.get(4)?;
    let stored_path: String = row.get(5)?;
    let expected_path = content_path(dataset_directory, &content_sha256);
    if manifest.content_sha256 != content_sha256
        || Path::new(&stored_path) != expected_path.as_path()
    {
        return Err(rusqlite::Error::FromSqlConversionFailure(
            4,
            rusqlite::types::Type::Text,
            Box::new(MaterializationStoreError::IncompatibleSchema {
                stored_schema_version: Some(FEATURE_DATASET_STORAGE_SCHEMA_VERSION.into()),
                table: Some("feature_dataset_contents.parquet_path".into()),
            }),
        ));
    }
    Ok(FeatureDataset {
        dataset_id: row.get(0)?,
        user_id: row.get(1)?,
        request_hash: row.get(2)?,
        manifest,
        parquet_path: expected_path,
        content_byte_size: row.get::<_, i64>(6)? as u64,
        created_at_ms: row.get(7)?,
    })
}

fn ensure_dataset_access(
    database: &Connection,
    user_id: &str,
    dataset_id: &str,
) -> Result<(), MaterializationStoreError> {
    let exists: Option<i64> = database
        .query_row(
            "SELECT 1 FROM feature_dataset_access WHERE user_id = ?1 AND dataset_id = ?2",
            params![user_id, dataset_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(sqlite_error)?;
    exists
        .map(|_| ())
        .ok_or(MaterializationStoreError::Unauthorized)
}

fn validate_user(user_id: &str) -> Result<(), MaterializationStoreError> {
    (!user_id.trim().is_empty())
        .then_some(())
        .ok_or(MaterializationStoreError::InvalidUser)
}

fn plan_output_names(plan: &FeaturePlan) -> Vec<String> {
    plan.definitions()
        .iter()
        .flat_map(|definition| {
            definition
                .outputs()
                .iter()
                .map(|output| output.name.clone())
        })
        .chain(plan.slot_names().map(str::to_owned))
        .collect()
}

fn validate_request(
    request: &FeatureMaterializationRequest,
) -> Result<(), MaterializationStoreError> {
    request
        .validate()
        .map_err(|_| MaterializationStoreError::InvalidRequest)
}

fn normalize_observations(
    output_names: &[&str],
    observations: &[FeatureObservation],
    request: &FeatureMaterializationRequest,
) -> Result<(Vec<String>, Vec<FeatureDatasetRow>), MaterializationStoreError> {
    let mut seen = BTreeSet::new();
    let names = output_names
        .iter()
        .map(|name| {
            if !is_lower_kebab(name) || !seen.insert(*name) {
                return Err(MaterializationStoreError::InvalidOutputSchema);
            }
            Ok((*name).to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    if names.is_empty() {
        return Err(MaterializationStoreError::InvalidOutputSchema);
    }
    let expected = names.iter().cloned().collect::<BTreeSet<_>>();
    let mut rows: BTreeMap<(String, i64), BTreeMap<String, FeatureDatasetCell>> = BTreeMap::new();
    for observation in observations {
        if observation.instrument_id.trim().is_empty() {
            return Err(MaterializationStoreError::InvalidObservation(
                "empty-instrument-id".into(),
            ));
        }
        if observation.observation_time_ms < request.observation_range.start_time_ms
            || observation.observation_time_ms >= request.observation_range.end_time_ms
        {
            return Err(MaterializationStoreError::InvalidObservation(
                "observation-time-outside-request-range".into(),
            ));
        }
        if !expected.contains(&observation.output_name) {
            return Err(MaterializationStoreError::InvalidObservation(
                "output-is-not-in-canonical-schema".into(),
            ));
        }
        let key = (
            observation.instrument_id.clone(),
            observation.observation_time_ms,
        );
        let cells = rows.entry(key).or_default();
        if cells.contains_key(&observation.output_name) {
            return Err(MaterializationStoreError::DuplicateObservation);
        }
        cells.insert(
            observation.output_name.clone(),
            cell_from_observation(observation)?,
        );
    }
    if rows.values().any(|cells| cells.len() != expected.len()) {
        return Err(MaterializationStoreError::IncompleteRows);
    }
    let rows = rows
        .into_iter()
        .map(
            |((instrument_id, observation_time_ms), values)| FeatureDatasetRow {
                instrument_id,
                observation_time_ms,
                values,
            },
        )
        .collect();
    Ok((names, rows))
}

fn cell_from_observation(
    observation: &FeatureObservation,
) -> Result<FeatureDatasetCell, MaterializationStoreError> {
    match observation.value {
        FeatureObservationValue::Available {
            value,
            available_at_ms,
        } if value.is_finite() => Ok(FeatureDatasetCell::Available {
            value,
            available_at_ms,
        }),
        FeatureObservationValue::Available { .. } => Err(
            MaterializationStoreError::InvalidObservation("non-finite-feature-value".into()),
        ),
        FeatureObservationValue::Unavailable { reason } => {
            Ok(FeatureDatasetCell::Unavailable { reason })
        }
    }
}

fn write_parquet(
    path: &Path,
    output_names: &[String],
    rows: &[FeatureDatasetRow],
) -> Result<(), MaterializationStoreError> {
    let schema = dataset_schema(output_names);
    let mut columns: Vec<ArrayRef> = vec![
        Arc::new(StringArray::from_iter_values(
            rows.iter().map(|row| row.instrument_id.as_str()),
        )),
        Arc::new(Int64Array::from_iter_values(
            rows.iter().map(|row| row.observation_time_ms),
        )),
    ];
    for output_name in output_names {
        columns.push(Arc::new(Float64Array::from(
            rows.iter()
                .map(|row| match row.values.get(output_name) {
                    Some(FeatureDatasetCell::Available { value, .. }) => Some(*value),
                    _ => None,
                })
                .collect::<Vec<_>>(),
        )));
        columns.push(Arc::new(Int64Array::from(
            rows.iter()
                .map(|row| match row.values.get(output_name) {
                    Some(FeatureDatasetCell::Available {
                        available_at_ms, ..
                    }) => Some(*available_at_ms),
                    _ => None,
                })
                .collect::<Vec<_>>(),
        )));
        columns.push(Arc::new(StringArray::from_iter_values(rows.iter().map(
            |row| match row.values.get(output_name) {
                Some(FeatureDatasetCell::Available { .. }) => "available",
                Some(FeatureDatasetCell::Unavailable { .. }) => "unavailable",
                None => "invalid",
            },
        ))));
        columns.push(Arc::new(StringArray::from_iter(rows.iter().map(|row| {
            match row.values.get(output_name) {
                Some(FeatureDatasetCell::Unavailable { reason }) => Some(reason.code()),
                Some(FeatureDatasetCell::Available { .. }) => None,
                None => Some("invalid"),
            }
        }))));
    }
    let batch = RecordBatch::try_new(schema.clone(), columns)
        .map_err(|error| MaterializationStoreError::Parquet(error.to_string()))?;
    let file = File::create(path).map_err(io_error)?;
    let mut writer = ArrowWriter::try_new(file, schema, None)
        .map_err(|error| MaterializationStoreError::Parquet(error.to_string()))?;
    writer
        .write(&batch)
        .map_err(|error| MaterializationStoreError::Parquet(error.to_string()))?;
    writer
        .close()
        .map_err(|error| MaterializationStoreError::Parquet(error.to_string()))?;
    Ok(())
}

fn read_parquet(
    path: &Path,
    output_names: &[String],
) -> Result<Vec<FeatureDatasetRow>, MaterializationStoreError> {
    let mut rows = Vec::new();
    read_parquet_each(path, output_names, |row| {
        rows.push(row);
        Ok(())
    })?;
    Ok(rows)
}

fn read_parquet_page(
    path: &Path,
    output_names: &[String],
    filter: &FeatureDatasetFilter,
    offset: usize,
) -> Result<(FeatureDatasetPage, u64), MaterializationStoreError> {
    let limit = validate_filter(output_names, filter)?;
    let mut matched = 0usize;
    let mut selected = Vec::with_capacity(limit.saturating_add(1));
    let row_count = read_parquet_each(path, output_names, |row| {
        if row_matches(&row, filter) {
            if matched >= offset && selected.len() < limit.saturating_add(1) {
                selected.push(row);
            }
            matched = matched.saturating_add(1);
        }
        Ok(())
    })?;
    let next_offset = (selected.len() > limit).then_some(offset.saturating_add(limit));
    Ok((
        FeatureDatasetPage {
            rows: selected.into_iter().take(limit).collect(),
            next_offset,
        },
        row_count,
    ))
}

fn read_parquet_each<F>(
    path: &Path,
    output_names: &[String],
    mut consume: F,
) -> Result<u64, MaterializationStoreError>
where
    F: FnMut(FeatureDatasetRow) -> Result<(), MaterializationStoreError>,
{
    let file = File::open(path).map_err(io_error)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(|error| MaterializationStoreError::Parquet(error.to_string()))?;
    let expected_schema = dataset_schema(output_names);
    if builder.schema().as_ref() != expected_schema.as_ref() {
        return Err(MaterializationStoreError::InvalidOutputSchema);
    }
    let reader = builder
        .with_batch_size(8192)
        .build()
        .map_err(|error| MaterializationStoreError::Parquet(error.to_string()))?;
    let mut previous = None;
    let mut row_count = 0;
    for batch in reader {
        let batch = batch.map_err(|error| MaterializationStoreError::Parquet(error.to_string()))?;
        let instruments = array::<StringArray>(&batch, 0)?;
        let times = array::<Int64Array>(&batch, 1)?;
        let columns = output_names
            .iter()
            .enumerate()
            .map(|(output_index, _)| {
                let base = 2 + output_index * 4;
                Ok((
                    array::<Float64Array>(&batch, base)?,
                    array::<Int64Array>(&batch, base + 1)?,
                    array::<StringArray>(&batch, base + 2)?,
                    array::<StringArray>(&batch, base + 3)?,
                ))
            })
            .collect::<Result<Vec<_>, MaterializationStoreError>>()?;
        for index in 0..batch.num_rows() {
            let mut values = BTreeMap::new();
            for (output_name, (numerical, available_at, state, reason)) in
                output_names.iter().zip(&columns)
            {
                let cell = match state.value(index) {
                    "available"
                        if !numerical.is_null(index)
                            && !available_at.is_null(index)
                            && reason.is_null(index) =>
                    {
                        let value = numerical.value(index);
                        if !value.is_finite() {
                            return Err(MaterializationStoreError::InvalidObservation(
                                "non-finite-feature-value".into(),
                            ));
                        }
                        FeatureDatasetCell::Available {
                            value,
                            available_at_ms: available_at.value(index),
                        }
                    }
                    "unavailable"
                        if numerical.is_null(index)
                            && available_at.is_null(index)
                            && !reason.is_null(index) =>
                    {
                        FeatureDatasetCell::Unavailable {
                            reason: reason_from_code(reason.value(index))?,
                        }
                    }
                    _ => return Err(MaterializationStoreError::InvalidOutputSchema),
                };
                values.insert(output_name.clone(), cell);
            }
            let instrument_id = instruments.value(index).to_owned();
            let observation_time_ms = times.value(index);
            if instrument_id.trim().is_empty() {
                return Err(MaterializationStoreError::InvalidObservation(
                    "empty-instrument-id".into(),
                ));
            }
            if previous.as_ref().is_some_and(|previous: &(String, i64)| {
                (previous.0.as_str(), previous.1) >= (instrument_id.as_str(), observation_time_ms)
            }) {
                return Err(MaterializationStoreError::InvalidObservation(
                    "rows-are-not-canonically-ordered".into(),
                ));
            }
            previous = Some((instrument_id.clone(), observation_time_ms));
            consume(FeatureDatasetRow {
                instrument_id,
                observation_time_ms,
                values,
            })?;
            row_count += 1;
        }
    }
    Ok(row_count)
}

fn dataset_schema(output_names: &[String]) -> Arc<Schema> {
    let mut fields = vec![
        Field::new("instrument_id", DataType::Utf8, false),
        Field::new("observation_time_ms", DataType::Int64, false),
    ];
    for output_name in output_names {
        let prefix = column_prefix(output_name);
        fields.extend([
            Field::new(format!("{prefix}__value"), DataType::Float64, true),
            Field::new(format!("{prefix}__available_at_ms"), DataType::Int64, true),
            Field::new(format!("{prefix}__state"), DataType::Utf8, false),
            Field::new(format!("{prefix}__reason"), DataType::Utf8, true),
        ]);
    }
    Arc::new(Schema::new(fields))
}

fn column_prefix(output_name: &str) -> String {
    format!("feature__{output_name}")
}

fn output_manifest(output_name: &str) -> FeatureDatasetOutputManifest {
    let prefix = column_prefix(output_name);
    FeatureDatasetOutputManifest {
        output_name: output_name.to_owned(),
        value_column: format!("{prefix}__value"),
        available_at_column: format!("{prefix}__available_at_ms"),
        state_column: format!("{prefix}__state"),
        reason_column: format!("{prefix}__reason"),
    }
}

fn array<T: Array + 'static>(
    batch: &RecordBatch,
    index: usize,
) -> Result<&T, MaterializationStoreError> {
    batch
        .column(index)
        .as_any()
        .downcast_ref::<T>()
        .ok_or(MaterializationStoreError::InvalidOutputSchema)
}

fn reason_from_code(code: &str) -> Result<FeatureUnavailabilityReason, MaterializationStoreError> {
    Ok(match code {
        "warmup" => FeatureUnavailabilityReason::Warmup,
        "bar-gap" => FeatureUnavailabilityReason::BarGap,
        "missing-market-input" => FeatureUnavailabilityReason::MissingMarketInput,
        "missing-dependency" => FeatureUnavailabilityReason::MissingDependency,
        "unknown-universe" => FeatureUnavailabilityReason::UnknownUniverse,
        "insufficient-coverage" => FeatureUnavailabilityReason::InsufficientCoverage,
        "undefined-arithmetic" => FeatureUnavailabilityReason::UndefinedArithmetic,
        "artifact-missing-instrument" => FeatureUnavailabilityReason::ArtifactMissingInstrument,
        "corporate-action-unavailable" => FeatureUnavailabilityReason::CorporateActionUnavailable,
        _ => return Err(MaterializationStoreError::InvalidOutputSchema),
    })
}

fn summarize(
    manifest: &FeatureDatasetManifest,
    rows: &[FeatureDatasetRow],
) -> Result<Vec<FeatureOutputSummary>, MaterializationStoreError> {
    manifest
        .outputs
        .iter()
        .map(|output| {
            let mut available = Vec::new();
            let mut unavailable_counts = BTreeMap::new();
            for row in rows {
                match row.values.get(&output.output_name) {
                    Some(FeatureDatasetCell::Available { value, .. }) => available.push(*value),
                    Some(FeatureDatasetCell::Unavailable { reason }) => {
                        *unavailable_counts.entry(reason.code().into()).or_insert(0) += 1;
                    }
                    None => return Err(MaterializationStoreError::InvalidOutputSchema),
                }
            }
            let mean = (!available.is_empty())
                .then(|| available.iter().sum::<f64>() / available.len() as f64);
            let population_standard_deviation = mean.map(|mean| {
                (available
                    .iter()
                    .map(|value| (value - mean).powi(2))
                    .sum::<f64>()
                    / available.len() as f64)
                    .sqrt()
            });
            Ok(FeatureOutputSummary {
                output_name: output.output_name.clone(),
                row_count: rows.len() as u64,
                available_count: available.len() as u64,
                coverage: if rows.is_empty() {
                    0.0
                } else {
                    available.len() as f64 / rows.len() as f64
                },
                unavailable_counts,
                minimum: available.iter().copied().reduce(f64::min),
                maximum: available.iter().copied().reduce(f64::max),
                mean,
                population_standard_deviation,
            })
        })
        .collect()
}

fn validate_filter(
    output_names: &[String],
    filter: &FeatureDatasetFilter,
) -> Result<usize, MaterializationStoreError> {
    let limit = if filter.limit == 0 {
        FEATURE_DATASET_MAX_PAGE_SIZE
    } else {
        filter.limit
    };
    if limit > FEATURE_DATASET_MAX_PAGE_SIZE
        || filter
            .start_time_ms
            .zip(filter.end_time_ms)
            .is_some_and(|(start, end)| start >= end)
    {
        return Err(MaterializationStoreError::InvalidFilter);
    }
    if let Some(output_name) = &filter.output_name
        && !output_names.iter().any(|output| output == output_name)
    {
        return Err(MaterializationStoreError::InvalidFilter);
    }
    Ok(limit)
}

fn row_matches(row: &FeatureDatasetRow, filter: &FeatureDatasetFilter) -> bool {
    let output_matches = filter
        .output_name
        .as_deref()
        .is_none_or(|output| row.values.contains_key(output));
    let state_matches = match (filter.output_name.as_deref(), filter.state) {
        (_, None) => true,
        (Some(output), Some(state)) => row
            .values
            .get(output)
            .is_some_and(|cell| cell_state(cell) == state),
        (None, Some(state)) => row.values.values().any(|cell| cell_state(cell) == state),
    };
    filter
        .instrument_id
        .as_deref()
        .is_none_or(|instrument| instrument == row.instrument_id)
        && filter
            .start_time_ms
            .is_none_or(|start| row.observation_time_ms >= start)
        && filter
            .end_time_ms
            .is_none_or(|end| row.observation_time_ms < end)
        && output_matches
        && state_matches
}

fn cell_state(cell: &FeatureDatasetCell) -> FeatureDatasetRowState {
    match cell {
        FeatureDatasetCell::Available { .. } => FeatureDatasetRowState::Available,
        FeatureDatasetCell::Unavailable { .. } => FeatureDatasetRowState::Unavailable,
    }
}

fn safe_staging_path(path: &Path, attempt_id: &str, staging_directory: &Path) -> Option<PathBuf> {
    let filename = path.file_name()?.to_str()?;
    (path.parent() == Some(staging_directory)
        && (filename == format!("{attempt_id}.parquet")
            || filename == format!("{attempt_id}.parquet.tmp")))
    .then(|| path.to_path_buf())
}

fn safe_staged_parquet_path(
    path: &Path,
    attempt_id: &str,
    staging_directory: &Path,
) -> Option<PathBuf> {
    let path = safe_staging_path(path, attempt_id, staging_directory)?;
    (path.file_name()?.to_str()? == format!("{attempt_id}.parquet")).then_some(path)
}

fn cleanup_staging_files(
    path: &Path,
    attempt_id: &str,
    staging_directory: &Path,
) -> Result<(), MaterializationStoreError> {
    let path = safe_staging_path(path, attempt_id, staging_directory)
        .ok_or(MaterializationStoreError::StagingNotFound)?;
    let is_temporary = path.file_name().and_then(|name| name.to_str())
        == Some(format!("{attempt_id}.parquet.tmp").as_str());
    remove_file_if_exists(&path)?;
    if is_temporary {
        remove_file_if_exists(&staging_directory.join(format!("{attempt_id}.parquet")))?;
    }
    Ok(())
}

fn content_path(dataset_directory: &Path, content_sha256: &str) -> PathBuf {
    dataset_directory.join(format!("{}.parquet", content_sha256))
}

fn safe_publication_path(
    path: &Path,
    content_sha256: &str,
    dataset_directory: &Path,
) -> Option<PathBuf> {
    (is_sha256(content_sha256) && path == content_path(dataset_directory, content_sha256))
        .then(|| path.to_path_buf())
}

fn remove_file_if_exists(path: &Path) -> Result<(), MaterializationStoreError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error(error)),
    }
}

fn sync_published_file(path: &Path) -> Result<(), MaterializationStoreError> {
    File::open(path)
        .map_err(io_error)?
        .sync_all()
        .map_err(io_error)?;
    #[cfg(unix)]
    if let Some(parent) = path.parent() {
        File::open(parent)
            .map_err(io_error)?
            .sync_all()
            .map_err(io_error)?;
    }
    Ok(())
}

fn table_exists(database: &Connection, table: &str) -> Result<bool, MaterializationStoreError> {
    database
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [table],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map(|value| value.is_some())
        .map_err(sqlite_error)
}

fn table_columns(
    database: &Connection,
    table: &str,
) -> Result<BTreeSet<String>, MaterializationStoreError> {
    let mut statement = database
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(sqlite_error)?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(sqlite_error)?;
    rows.map(|row| row.map_err(sqlite_error)).collect()
}

fn json_string<T: Serialize>(value: &T) -> Result<String, MaterializationStoreError> {
    let json = serde_json::to_vec(value)
        .map_err(|error| MaterializationStoreError::Json(error.to_string()))?;
    let canonical = canonicalize_json(&json).map_err(MaterializationStoreError::Json)?;
    String::from_utf8(canonical).map_err(|error| MaterializationStoreError::Json(error.to_string()))
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    digest.update(bytes);
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn sha256_file(path: &Path) -> Result<String, MaterializationStoreError> {
    use sha2::{Digest, Sha256};

    let file = File::open(path).map_err(io_error)?;
    let mut reader = BufReader::new(file);
    let mut digest = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer).map_err(io_error)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn io_error(error: std::io::Error) -> MaterializationStoreError {
    MaterializationStoreError::Io(error.to_string())
}

fn sqlite_error(error: rusqlite::Error) -> MaterializationStoreError {
    MaterializationStoreError::Sqlite(error.to_string())
}
