//! Durable M11 Factor evidence and the native research API boundary.
//!
//! The core crate owns Factor contracts and algorithms. This module owns the
//! device-local SQLite/Parquet boundary, User isolation, lifecycle recovery,
//! and the Factor jobs consumed by the existing single Feature FIFO worker.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs::{self, File},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use adaq_component_tooling::{
    ComponentKind, ComponentPackage, FactorScope as PackageFactorScope, ParameterType,
};
use adaq_factor_research::{
    AttemptStatus, CandidateBuildRequest, CompletedFeatureDataset, ComponentEligibilityEvidence,
    EvaluationEvidenceState, EvaluationFeatureEvidence, FactorCandidate, FactorCandidateDraft,
    FactorCandidateSource, FactorDataset, FactorDatasetManifest, FactorDatasetRow,
    FactorEvaluationInput, FactorEvaluationProtocol, FactorEvaluationProtocolDraft,
    FactorEvaluationReport, FactorEvaluator, FactorMaterializationInput,
    FactorMaterializationProtocol, FactorMaterializationProtocolDraft, FactorMaterializer,
    FactorObservationValue, FactorPresentationMetadata, FactorPromotionDecision, FactorTarget,
    FactorUnavailabilityReason, GridSearchFamilyDraft, GridSearchParameter, MetricId,
    MetricObservation, ObservationRange, PromotionDecisionDraft, PromotionDecisionState,
    PromotionEligibility, PromotionPolicy, PromotionProtocol, PromotionProtocolDraft,
    PythonFactorBinding, ResearchFamily, ResearchFamilyDraft, ResearchFamilyRegistration,
    ResearchLineage, ResearchRegistry, ResearchTrial, ResearchTrialDraft,
    ResearchTrialRegistration, ResearchTrialStatus, canonical_json,
};
use arrow_array::{Array, ArrayRef, Float64Array, Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use parquet::arrow::{ArrowWriter, arrow_reader::ParquetRecordBatchReaderBuilder};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    research_queue::{
        QueueAdmission, QueueAdmitter, QueueRunResult, QueueTicket, ResearchQueueAdapter, WorkKind,
    },
    user::validate_user,
};

const STORAGE_SCHEMA_VERSION: &str = "1.1.0";
const MAX_PAGE_SIZE: u32 = 100;
const DEFAULT_PAGE_SIZE: u32 = 50;
const MAX_JOB_BYTES: usize = 64 * 1024 * 1024;
const MAX_DIAGNOSTIC_BYTES: usize = 16 * 1024;
const CANCELLATION_REQUESTED_DIAGNOSTIC: &str = "Factor research Attempt cancellation requested";

#[derive(Debug, Clone, PartialEq, Eq)]
struct FactorQueueItem {
    attempt_id: String,
}

pub(crate) trait FactorResearchSource: Send + Sync {
    fn database(&self) -> Result<MutexGuard<'_, Connection>, String>;
    fn dataset_directory(&self) -> Result<PathBuf, String>;

    fn feature_dataset(
        &self,
        _user_id: &str,
        _dataset_id: &str,
    ) -> Result<CompletedFeatureDataset, String> {
        Err("Feature Dataset source is not configured".into())
    }

    fn point_in_time_universe(
        &self,
        _user_id: &str,
        _universe_id: &str,
    ) -> Result<Vec<String>, String> {
        Err("Point-in-Time Universe source is not configured".into())
    }

    fn component_package(
        &self,
        _user_id: &str,
        _archive_sha256: &str,
    ) -> Result<ComponentPackage, String> {
        Err("Component Package source is not configured".into())
    }

    fn reference_feature_dataset(
        &self,
        _user_id: &str,
        _dataset_id: &str,
        _reference_id: &str,
    ) -> Result<(), String> {
        Ok(())
    }

    fn unreference_feature_dataset(
        &self,
        _user_id: &str,
        _dataset_id: &str,
        _reference_id: &str,
    ) -> Result<(), String> {
        Ok(())
    }

    fn validate_materialization_context(
        &self,
        _user_id: &str,
        _context: &FactorMaterializationContextBinding,
    ) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum FactorFailureCode {
    Cancelled,
    CandidateBuildFailed,
    FactorMaterializationFailed,
    FactorEvaluationFailed,
    FactorFamilyGridFailed,
    FactorResearchFailed,
    FactorCompatibilityFailed,
    FactorValidationFailed,
    FactorResourceFailed,
    FactorMissingInput,
    FactorPublicationFailed,
    FactorCorruptionDetected,
    ResearchInterrupted,
    ResetRequired,
    FactorContextMismatch,
    FactorContextRequiresHostDatasetSelection,
    FactorContextFeatureDatasetRequired,
    FactorContextStale,
    FactorContextFeatureDatasetInaccessible,
    FactorContextFeatureDatasetIncomplete,
    FactorContextFeatureDatasetUnavailable,
    FactorContextSnapshotInaccessible,
    FactorContextUniverseInaccessible,
    FactorContextUserMismatch,
    FactorContextMarketVenueMismatch,
    FactorContextIntervalMismatch,
    FactorContextRangeMismatch,
    FactorContextUniverseIncomplete,
    FactorContextMarketVenueUnavailable,
    FactorContextCandidatePredecessorMissing,
    FactorContextValuationCurrencyRequired,
}

impl FactorFailureCode {
    fn from_code(value: &str) -> Option<Self> {
        serde_json::from_value(serde_json::Value::String(value.into())).ok()
    }

    fn for_kind(kind: &str) -> Self {
        match kind {
            "candidate-build" => Self::CandidateBuildFailed,
            "factor-materialization" => Self::FactorMaterializationFailed,
            "factor-evaluation" => Self::FactorEvaluationFailed,
            "factor-family-grid" => Self::FactorFamilyGridFailed,
            _ => Self::FactorResearchFailed,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FactorAttemptView {
    pub attempt_id: String,
    pub user_id: String,
    pub kind: String,
    pub request_hash: String,
    pub status: AttemptStatus,
    pub source_attempt_id: Option<String>,
    pub result_id: Option<String>,
    pub completed_units: u64,
    pub progress_total: u64,
    pub diagnostic: Option<String>,
    pub failure_code: Option<FactorFailureCode>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FactorPage<T> {
    pub items: Vec<T>,
    pub page: u32,
    pub page_size: u32,
    pub total: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FactorCandidateView {
    pub candidate: FactorCandidate,
    pub presentation: FactorPresentationMetadata,
    pub locked_by: Vec<String>,
    pub created_at_ms: i64,
    pub predecessor: Option<FactorCandidatePredecessor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FactorCandidatePredecessor {
    pub user_id: String,
    pub context_revision: u64,
    pub context_hash: String,
    pub market: String,
    pub venue: String,
    pub range_start_ms: i64,
    pub range_end_ms: i64,
    pub snapshot_id: String,
    pub universe_id: Option<String>,
    pub evidence: Vec<adaq_factor_research::EvidenceBinding>,
    pub feature_dataset: adaq_factor_research::FeatureDatasetBinding,
}

impl FactorCandidatePredecessor {
    pub(crate) fn from_projection(
        user_id: String,
        projection: adaq_factor_research::ResearchEvidenceProjection,
    ) -> Result<Self, String> {
        let feature_dataset = projection
            .feature_dataset
            .ok_or_else(|| "factor-context-feature-dataset-required".to_owned())?;
        let predecessor = Self {
            user_id,
            context_revision: projection.context_revision,
            context_hash: projection.context_hash,
            market: projection.market,
            venue: projection.venue,
            range_start_ms: projection.range_start_ms,
            range_end_ms: projection.range_end_ms,
            snapshot_id: projection.snapshot_id,
            universe_id: projection.universe_id,
            evidence: projection.evidence,
            feature_dataset,
        };
        predecessor.validate()?;
        Ok(predecessor)
    }

    fn validate(&self) -> Result<(), String> {
        validate_user(&self.user_id)?;
        if self.context_revision == 0
            || !adaq_factor_research::is_sha256(&self.context_hash)
            || self.market.trim().is_empty()
            || self.venue.trim().is_empty()
            || self.range_start_ms >= self.range_end_ms
            || self.snapshot_id.trim().is_empty()
            || self.universe_id.as_deref().is_none_or(str::is_empty)
            || self.evidence.is_empty()
        {
            return Err("Factor Candidate predecessor identity is invalid".into());
        }
        let feature_dataset = &self.feature_dataset;
        if feature_dataset.dataset_id.trim().is_empty()
            || !adaq_factor_research::is_sha256(&feature_dataset.request_hash)
            || !adaq_factor_research::is_sha256(&feature_dataset.feature_plan_hash)
            || !adaq_factor_research::is_sha256(&feature_dataset.content_sha256)
            || feature_dataset.output_names.is_empty()
        {
            return Err("Factor Candidate predecessor Feature Dataset is invalid".into());
        }
        let mut output_names = BTreeSet::new();
        if feature_dataset
            .output_names
            .iter()
            .any(|name| !adaq_factor_research::is_lower_kebab(name) || !output_names.insert(name))
        {
            return Err("Factor Candidate predecessor Feature outputs are invalid".into());
        }
        if !self.evidence.iter().any(|evidence| {
            evidence.id == feature_dataset.dataset_id
                && evidence.user_id == self.user_id
                && evidence.market == self.market
                && evidence.venue == self.venue
                && evidence.accessible
                && evidence.complete
                && evidence.fresh
        }) {
            return Err("Factor Candidate predecessor evidence is incomplete".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FactorDatasetView {
    pub manifest: FactorDatasetManifest,
    pub byte_size: u64,
    pub locked_by: Vec<String>,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FactorDatasetRowsPage {
    pub rows: Vec<FactorDatasetRow>,
    pub offset: u64,
    pub limit: u32,
    pub next_offset: Option<u64>,
    pub total: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FactorFamilyView {
    pub family: ResearchFamily,
    pub trial_count: u64,
    pub lineage_hash: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FactorReportView {
    pub report: FactorEvaluationReport,
    pub protocol: Option<FactorEvaluationProtocol>,
    pub locked_by: Vec<String>,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FactorPolicyView {
    pub policy: PromotionPolicy,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FactorDecisionView {
    pub decision: FactorPromotionDecision,
    pub promotion_protocol_hash: String,
    pub eligibility_gates: Vec<adaq_factor_research::PromotionGateResult>,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FactorTrialSelectionRequest {
    pub user_id: String,
    pub candidate_hash: String,
    pub family_id: Uuid,
    pub trial_id: Uuid,
    pub policy_hash: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FactorSelectionView {
    pub candidate_hash: String,
    pub family_id: Uuid,
    pub selected_trial_id: Uuid,
    pub selection_hash: String,
    pub promotion_protocol_hash: String,
}

#[derive(Debug, Clone)]
pub(crate) struct FactorModelInputBinding {
    pub decision_hash: String,
    pub promotion_protocol: PromotionProtocol,
    pub factor_dataset_id: String,
    pub feature_dataset_id: String,
    pub feature_plan_hash: String,
    pub snapshot_id: String,
    pub universe_id: String,
    pub lookback: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FactorLineageView {
    pub lineage: ResearchLineage,
    pub trials: Vec<ResearchTrial>,
    pub registrations: Vec<ResearchTrialRegistration>,
    pub protocols: Vec<FactorEvaluationProtocol>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FactorPageRequest {
    pub user_id: String,
    pub page: u32,
    #[serde(default)]
    pub page_size: Option<u32>,
    #[serde(default)]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FactorAttemptRequest {
    pub user_id: String,
    pub attempt_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FactorCandidateBuildRequest {
    pub user_id: String,
    pub operation_id: String,
    pub candidate: FactorCandidate,
    pub presentation: FactorPresentationMetadata,
    #[serde(default)]
    pub build: Option<FactorControlledBuildRequest>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FactorCandidatePublishRequest {
    pub user_id: String,
    pub draft: FactorCandidateDraft,
    pub presentation: FactorPresentationMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PythonHostEvidence {
    pub project_revision_sha256: String,
    pub environment_sha256: String,
    pub repeatability_report_sha256: String,
    pub attempts: Vec<PythonHostAttemptEvidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PythonHostAttemptEvidence {
    pub attempt_id: String,
    pub owner_user_id: String,
    pub status: String,
    pub project_revision_sha256: String,
    pub environment_sha256: String,
    pub result_sha256: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FactorControlledBuildRequest {
    pub project_root: PathBuf,
    pub source_sha256: String,
    pub sdk_version: String,
    pub toolchain: String,
    pub target: String,
    pub resource_policy: adaq_factor_research::FactorResourcePolicy,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FactorDatasetInput {
    pub manifest: FactorDatasetManifest,
    pub rows: Vec<FactorDatasetRow>,
}

impl FactorDatasetInput {
    fn into_dataset(self) -> Result<FactorDataset, String> {
        let dataset = FactorDataset {
            manifest: self.manifest,
            rows: self.rows,
        };
        dataset.validate().map_err(|error| error.to_string())?;
        Ok(dataset)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MaterializationJob {
    user_id: String,
    protocol: FactorMaterializationProtocol,
    dataset: Option<FactorDatasetInput>,
    #[serde(default)]
    context: Option<FactorMaterializationContextBinding>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EvaluationJob {
    user_id: String,
    protocol: FactorEvaluationProtocol,
    dataset: Option<FactorDatasetInput>,
    market_series: Vec<adaq_factor_research::FactorMarketSeries>,
    feature_evidence: Option<EvaluationFeatureEvidence>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CandidateBuildJob {
    user_id: String,
    candidate: FactorCandidate,
    presentation: FactorPresentationMetadata,
    build: Option<FactorControlledBuildRequest>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FactorMaterializationStartRequest {
    pub user_id: String,
    pub protocol: FactorMaterializationProtocol,
    #[serde(default)]
    pub dataset: Option<FactorDatasetInput>,
    #[serde(default)]
    pub context: Option<FactorMaterializationContextBinding>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FactorMaterializationContextBinding {
    pub operation_id: String,
    pub context_revision: u64,
    pub context_hash: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FactorMaterializationContextStartRequest {
    pub user_id: String,
    pub operation_id: String,
    pub candidate_hash: String,
    #[serde(default)]
    pub seed: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FactorMaterializationProtocolFreezeRequest {
    pub user_id: String,
    pub draft: FactorMaterializationProtocolDraft,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FactorEvaluationStartRequest {
    pub user_id: String,
    pub protocol: FactorEvaluationProtocol,
    #[serde(default)]
    pub dataset: Option<FactorDatasetInput>,
    pub market_series: Vec<adaq_factor_research::FactorMarketSeries>,
    pub feature_evidence: Option<EvaluationFeatureEvidence>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FactorEvaluationContextStartRequest {
    pub user_id: String,
    pub operation_id: String,
    pub candidate_hash: String,
    pub dataset_id: String,
    pub output_name: String,
    #[serde(default)]
    pub seed: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FactorEvaluationProtocolFreezeRequest {
    pub user_id: String,
    pub draft: FactorEvaluationProtocolDraft,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FactorFamilyRegisterRequest {
    pub user_id: String,
    pub registration: ResearchFamilyRegistration,
    #[serde(default)]
    pub trials: Vec<ResearchTrial>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FactorGridFamilyRegisterRequest {
    pub user_id: String,
    pub family_id: Uuid,
    pub candidate_hash: String,
    #[serde(default)]
    pub parent_family_id: Option<Uuid>,
    pub parameters: Vec<GridSearchParameter>,
    pub target: FactorTarget,
    pub market_context: adaq_factor_research::FactorMarketContext,
    pub point_in_time_universe_id: String,
    pub observation_range: ObservationRange,
    pub base_protocol_hash: String,
    #[serde(default)]
    pub derivation_hash: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FactorTrialUpdateRequest {
    pub user_id: String,
    pub trial: ResearchTrial,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FactorPolicySaveRequest {
    pub user_id: String,
    pub policy: PromotionPolicy,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FactorPromotionProtocolFreezeRequest {
    pub user_id: String,
    pub candidate_hash: String,
    pub dataset_id: String,
    pub output_name: String,
    pub family_id: Uuid,
    pub trial_id: Uuid,
    pub report_hashes: Vec<String>,
    pub policy_hash: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FactorDecisionRecordRequest {
    pub user_id: String,
    pub state: PromotionDecisionState,
    pub promotion_protocol: PromotionProtocol,
    #[serde(default)]
    pub supersedes: Option<Uuid>,
    #[serde(default)]
    pub component: ComponentEligibilityEvidence,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FactorDecisionSaveRequest {
    pub user_id: String,
    pub decision: FactorPromotionDecision,
    pub promotion_protocol: PromotionProtocol,
    #[serde(default)]
    pub component: ComponentEligibilityEvidence,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FactorReferenceRequest {
    pub user_id: String,
    pub evidence_kind: String,
    pub evidence_id: String,
    pub reference_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FactorEvidenceRequest {
    pub user_id: String,
    pub evidence_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FactorM12Request {
    pub user_id: String,
    pub promotion_protocol: PromotionProtocol,
}

#[derive(Clone)]
pub(crate) struct FactorResearch {
    inner: Arc<FactorResearchInner>,
}

struct FactorResearchInner {
    source: Arc<dyn FactorResearchSource>,
    schema_blocked: AtomicBool,
    shutdown_requested: AtomicBool,
    active: Mutex<HashMap<String, Arc<AtomicBool>>>,
    reset_blocks: Mutex<std::collections::HashSet<String>>,
    start_gate: Mutex<()>,
    admit: QueueAdmitter,
}

impl FactorResearch {
    pub(crate) fn open(
        source: Arc<dyn FactorResearchSource>,
        admit: QueueAdmitter,
    ) -> Result<Self, String> {
        let database = source.database()?;
        let schema_blocked = match ResearchStore::new(&database).initialize() {
            Ok(()) => false,
            Err(error) if error.starts_with("reset-required:") => true,
            Err(error) => return Err(error),
        };
        drop(database);
        let directory = source.dataset_directory()?;
        fs::create_dir_all(&directory).map_err(string)?;
        if !schema_blocked {
            ResearchStore::recover_stale_attempts(&directory, &source)?;
        }
        Ok(Self {
            inner: Arc::new(FactorResearchInner {
                source,
                schema_blocked: AtomicBool::new(schema_blocked),
                shutdown_requested: AtomicBool::new(false),
                active: Mutex::new(HashMap::new()),
                reset_blocks: Mutex::new(std::collections::HashSet::new()),
                start_gate: Mutex::new(()),
                admit,
            }),
        })
    }

    fn ensure_schema_ready(&self) -> Result<(), String> {
        if self.inner.schema_blocked.load(Ordering::Acquire) {
            return Err(
                "reset-required: incompatible Factor research storage; perform an explicit device-level reset"
                    .into(),
            );
        }
        Ok(())
    }

    fn database(&self) -> Result<MutexGuard<'_, Connection>, String> {
        self.ensure_schema_ready()?;
        self.inner.source.database()
    }

    fn validate_python_host_evidence(
        user_id: &str,
        binding: &PythonFactorBinding,
        evidence: &PythonHostEvidence,
    ) -> Result<(), String> {
        if evidence.project_revision_sha256 != binding.project_revision_sha256
            || evidence.environment_sha256 != binding.environment_sha256
            || evidence.repeatability_report_sha256 != binding.repeatability_report_sha256
            || evidence.attempts.is_empty()
        {
            return Err("Python Host evidence does not match the candidate binding".into());
        }
        if evidence
            .attempts
            .iter()
            .any(|attempt| attempt.owner_user_id != user_id)
        {
            return Err("Python Host evidence does not match the candidate binding".into());
        }
        let attempt_ids = evidence
            .attempts
            .iter()
            .map(|attempt| attempt.attempt_id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        if attempt_ids.len() != evidence.attempts.len()
            || binding.repeatability_report.values().any(|report| {
                !attempt_ids.contains(report.first_attempt_id.as_str())
                    || !attempt_ids.contains(report.replay_attempt_id.as_str())
            })
        {
            return Err("Python Host evidence does not cover repeatability Attempts".into());
        }
        Ok(())
    }

    fn enqueue<T: Serialize>(
        &self,
        user_id: &str,
        kind: &str,
        job: &T,
    ) -> Result<FactorAttemptView, String> {
        self.ensure_schema_ready()?;
        validate_user(user_id)?;
        let _gate = self.inner.start_gate.lock().map_err(string)?;
        if self
            .inner
            .reset_blocks
            .lock()
            .map_err(string)?
            .contains(user_id)
        {
            return Err("Factor research User reset is in progress".into());
        }
        let request_json = serde_json::to_vec(job).map_err(string)?;
        if request_json.len() > MAX_JOB_BYTES {
            return Err("factor research job exceeds the worker payload limit".into());
        }
        let request_json = String::from_utf8(request_json).map_err(string)?;
        let (attempt, should_start) = {
            let database = self.database()?;
            ResearchStore::new(&database).start_attempt(user_id, kind, &request_json)?
        };
        if should_start && attempt.status == AttemptStatus::Pending {
            (self.inner.admit)(WorkKind::Factor, &attempt.user_id, &attempt.attempt_id)?;
        }
        Ok(attempt)
    }

    pub(crate) fn build_candidate(
        &self,
        request: FactorCandidateBuildRequest,
    ) -> Result<FactorAttemptView, String> {
        validate_user(&request.user_id)?;
        request.candidate.validate().map_err(string)?;
        request.presentation.validate().map_err(string)?;
        let job = CandidateBuildJob {
            user_id: request.user_id.clone(),
            candidate: request.candidate,
            presentation: request.presentation,
            build: request.build,
        };
        self.enqueue(&request.user_id, "candidate-build", &job)
    }

    pub(crate) fn publish_candidate_with_predecessor(
        &self,
        request: FactorCandidatePublishRequest,
        predecessor: FactorCandidatePredecessor,
    ) -> Result<FactorCandidateView, String> {
        self.ensure_schema_ready()?;
        validate_user(&request.user_id)?;
        if request.user_id != predecessor.user_id {
            return Err(
                "Factor Candidate predecessor User identity differs from the request".into(),
            );
        }
        if !matches!(
            &request.draft.source,
            FactorCandidateSource::Declarative { .. }
        ) {
            return Err(
                "Factor Candidate discovery requires a Declarative Factor definition".into(),
            );
        }
        request.presentation.validate().map_err(string)?;
        predecessor.validate()?;
        let candidate = FactorCandidate::freeze(request.draft).map_err(string)?;
        let definition = match &candidate.source {
            FactorCandidateSource::Declarative { definition } => definition,
            _ => unreachable!("the Candidate source was checked above"),
        };
        let available_outputs = predecessor
            .feature_dataset
            .output_names
            .iter()
            .collect::<BTreeSet<_>>();
        if definition.feature_plan_hash != predecessor.feature_dataset.feature_plan_hash {
            return Err(
                "Factor Candidate Feature Plan does not match the selected Feature Dataset".into(),
            );
        }
        if let Some(slot) = candidate
            .feature_slots
            .iter()
            .find(|slot| !available_outputs.contains(&slot.name))
        {
            return Err(format!(
                "Factor Candidate Feature Slot {} is not present in the selected Feature Dataset",
                slot.name
            ));
        }
        if let Some(binding) = definition
            .outputs
            .iter()
            .find(|binding| !available_outputs.contains(&binding.feature_slot))
        {
            return Err(format!(
                "Factor Candidate output {} references Feature Slot {} that is not present in the selected Feature Dataset",
                binding.output_name, binding.feature_slot
            ));
        }
        let database = self.database()?;
        ResearchStore::new(&database).save_candidate_with_predecessor(
            &request.user_id,
            &candidate,
            &request.presentation,
            &predecessor,
        )?;
        ResearchStore::new(&database)
            .candidate_for_user(&request.user_id, &candidate.candidate_hash)
    }

    pub(crate) fn publish_python_candidate(
        &self,
        request: FactorCandidatePublishRequest,
        host_evidence: PythonHostEvidence,
    ) -> Result<FactorCandidateView, String> {
        self.ensure_schema_ready()?;
        validate_user(&request.user_id)?;
        request.presentation.validate().map_err(string)?;
        let binding = match &request.draft.source {
            FactorCandidateSource::Python { binding } => binding,
            _ => return Err("Host Python evidence requires a Python candidate".into()),
        };
        Self::validate_python_host_evidence(&request.user_id, binding, &host_evidence)?;
        let candidate = FactorCandidate::freeze(request.draft).map_err(string)?;
        let database = self.database()?;
        ResearchStore::new(&database).save_candidate_with_python_evidence(
            &request.user_id,
            &candidate,
            &request.presentation,
            &host_evidence,
        )?;
        ResearchStore::new(&database)
            .candidate_for_user(&request.user_id, &candidate.candidate_hash)
    }

    fn validate_materialization_context_binding(
        &self,
        user_id: &str,
        protocol: &FactorMaterializationProtocol,
        context: &FactorMaterializationContextBinding,
    ) -> Result<(), String> {
        if context.operation_id.trim().is_empty()
            || context.context_revision == 0
            || !adaq_factor_research::is_sha256(&context.context_hash)
        {
            return Err("factor-context-mismatch".into());
        }
        let database = self.database()?;
        let candidate =
            ResearchStore::new(&database).candidate_for_user(user_id, &protocol.candidate_hash)?;
        drop(database);
        let predecessor = candidate
            .predecessor
            .as_ref()
            .ok_or("factor-context-candidate-predecessor-missing")?;
        if predecessor.user_id != user_id
            || predecessor.context_revision != context.context_revision
            || predecessor.context_hash != context.context_hash
            || predecessor.feature_dataset.dataset_id != protocol.feature_dataset_id
            || predecessor.feature_dataset.feature_plan_hash != protocol.feature_plan_hash
            || predecessor.snapshot_id != protocol.market_data_snapshot_id
            || predecessor.universe_id.as_deref()
                != Some(protocol.point_in_time_universe_id.as_str())
            || predecessor.market != protocol.market_context.asset_class
            || predecessor.venue != protocol.market_context.venue
            || predecessor.range_start_ms != protocol.observation_range.start_time_ms
            || predecessor.range_end_ms != protocol.observation_range.end_time_ms
        {
            return Err("factor-context-mismatch".into());
        }
        self.inner
            .source
            .validate_materialization_context(user_id, context)
    }

    pub(crate) fn start_materialization(
        &self,
        request: FactorMaterializationStartRequest,
    ) -> Result<FactorAttemptView, String> {
        self.ensure_schema_ready()?;
        validate_user(&request.user_id)?;
        let user = user_uuid(&request.user_id);
        if request.protocol.user_id != user {
            return Err(
                "Factor Materialization Protocol User identity differs from the request".into(),
            );
        }
        request.protocol.validate().map_err(string)?;
        if let Some(dataset) = request
            .dataset
            .as_ref()
            .map(|input| input.clone().into_dataset())
        {
            let dataset = dataset?;
            if dataset.manifest.protocol_hash != request.protocol.protocol_hash
                || dataset.manifest.candidate_hash != request.protocol.candidate_hash
                || dataset.manifest.feature_dataset_id != request.protocol.feature_dataset_id
            {
                return Err(
                    "Factor Dataset is not bound to the exact Materialization Protocol".into(),
                );
            }
        }
        if let Some(context) = request.context.as_ref() {
            self.validate_materialization_context_binding(
                &request.user_id,
                &request.protocol,
                context,
            )?;
        }
        let job = MaterializationJob {
            user_id: request.user_id.clone(),
            protocol: request.protocol.clone(),
            dataset: request.dataset,
            context: request.context,
        };
        {
            let database = self.database()?;
            ResearchStore::new(&database).save_materialization_protocol(&request.protocol)?;
        }
        self.enqueue(&request.user_id, "factor-materialization", &job)
    }

    pub(crate) fn freeze_materialization_protocol(
        &self,
        request: FactorMaterializationProtocolFreezeRequest,
    ) -> Result<FactorMaterializationProtocol, String> {
        validate_user(&request.user_id)?;
        if request.draft.user_id != user_uuid(&request.user_id) {
            return Err(
                "Factor Materialization Draft User identity differs from the request".into(),
            );
        }
        FactorMaterializationProtocol::freeze(request.draft).map_err(string)
    }

    pub(crate) fn start_evaluation(
        &self,
        request: FactorEvaluationStartRequest,
    ) -> Result<FactorAttemptView, String> {
        self.start_evaluation_inner(request, false)
    }

    pub(crate) fn start_evaluation_host_owned(
        &self,
        request: FactorEvaluationStartRequest,
    ) -> Result<FactorAttemptView, String> {
        self.start_evaluation_inner(request, true)
    }

    fn start_evaluation_inner(
        &self,
        request: FactorEvaluationStartRequest,
        require_exact_trial_protocol: bool,
    ) -> Result<FactorAttemptView, String> {
        self.ensure_schema_ready()?;
        validate_user(&request.user_id)?;
        if request.protocol.user_id != user_uuid(&request.user_id) {
            return Err("Factor Evaluation Protocol User identity differs from the request".into());
        }
        request.protocol.validate().map_err(string)?;
        {
            let database = self.database()?;
            let store = ResearchStore::new(&database);
            let stored_dataset =
                store.dataset_for_user(&request.user_id, &request.protocol.factor_dataset_id)?;
            let candidate = store
                .candidate_for_user(&request.user_id, &stored_dataset.manifest.candidate_hash)?;
            if let Some(input) = request.dataset.as_ref() {
                let supplied = input.clone().into_dataset()?;
                if supplied.manifest != stored_dataset.manifest {
                    return Err("Factor Dataset input does not match the Host-owned Dataset".into());
                }
            }
            validate_evaluation_boundary(
                &candidate.candidate,
                candidate.predecessor.as_ref(),
                &stored_dataset.manifest,
                &request.protocol,
            )?;
            store.ensure_evaluation_trial(
                &request.user_id,
                &request.protocol,
                &stored_dataset.manifest,
                require_exact_trial_protocol,
            )?;
        }
        let job = EvaluationJob {
            user_id: request.user_id.clone(),
            protocol: request.protocol.clone(),
            dataset: request.dataset,
            market_series: request.market_series,
            feature_evidence: request.feature_evidence,
        };
        {
            let database = self.database()?;
            ResearchStore::new(&database).save_evaluation_protocol(&request.protocol)?;
        }
        self.enqueue(&request.user_id, "factor-evaluation", &job)
    }

    pub(crate) fn freeze_evaluation_protocol(
        &self,
        request: FactorEvaluationProtocolFreezeRequest,
    ) -> Result<FactorEvaluationProtocol, String> {
        validate_user(&request.user_id)?;
        if request.draft.user_id != user_uuid(&request.user_id) {
            return Err("Factor Evaluation Draft User identity differs from the request".into());
        }
        FactorEvaluationProtocol::freeze(request.draft).map_err(string)
    }

    pub(crate) fn list_attempts(
        &self,
        request: FactorPageRequest,
    ) -> Result<FactorPage<FactorAttemptView>, String> {
        validate_user(&request.user_id)?;
        let database = self.database()?;
        ResearchStore::new(&database).list_attempts(&request)
    }

    pub(crate) fn get_attempt(
        &self,
        request: FactorAttemptRequest,
    ) -> Result<FactorAttemptView, String> {
        validate_user(&request.user_id)?;
        let database = self.database()?;
        ResearchStore::new(&database).attempt_for_user(&request.user_id, &request.attempt_id)
    }

    pub(crate) fn cancel(&self, request: FactorAttemptRequest) -> Result<(), String> {
        self.ensure_schema_ready()?;
        validate_user(&request.user_id)?;
        let database = self.database()?;
        let status =
            ResearchStore::new(&database).cancel_attempt(&request.user_id, &request.attempt_id)?;
        drop(database);
        if status == AttemptStatus::Running
            && let Ok(active) = self.inner.active.lock()
            && let Some(cancelled) = active.get(&request.attempt_id)
        {
            cancelled.store(true, Ordering::Relaxed);
        }
        Ok(())
    }

    pub(crate) fn retry(&self, request: FactorAttemptRequest) -> Result<FactorAttemptView, String> {
        self.ensure_schema_ready()?;
        validate_user(&request.user_id)?;
        let _gate = self.inner.start_gate.lock().map_err(string)?;
        if self
            .inner
            .reset_blocks
            .lock()
            .map_err(string)?
            .contains(&request.user_id)
        {
            return Err("Factor research User reset is in progress".into());
        }
        let database = self.database()?;
        let (attempt, should_start) =
            ResearchStore::new(&database).retry_attempt(&request.user_id, &request.attempt_id)?;
        drop(database);
        if should_start && attempt.status == AttemptStatus::Pending {
            (self.inner.admit)(WorkKind::Factor, &attempt.user_id, &attempt.attempt_id)?;
        }
        Ok(attempt)
    }

    pub(crate) fn list_candidates(
        &self,
        request: FactorPageRequest,
    ) -> Result<FactorPage<FactorCandidateView>, String> {
        validate_user(&request.user_id)?;
        let database = self.database()?;
        ResearchStore::new(&database).list_candidates(&request)
    }

    pub(crate) fn get_candidate(
        &self,
        request: FactorEvidenceRequest,
    ) -> Result<FactorCandidateView, String> {
        validate_user(&request.user_id)?;
        let database = self.database()?;
        ResearchStore::new(&database).candidate_for_user(&request.user_id, &request.evidence_id)
    }

    pub(crate) fn list_datasets(
        &self,
        request: FactorPageRequest,
    ) -> Result<FactorPage<FactorDatasetView>, String> {
        validate_user(&request.user_id)?;
        let database = self.database()?;
        ResearchStore::new(&database).list_datasets(&request)
    }

    pub(crate) fn get_dataset(
        &self,
        request: FactorEvidenceRequest,
    ) -> Result<FactorDatasetView, String> {
        validate_user(&request.user_id)?;
        let database = self.database()?;
        ResearchStore::new(&database).dataset_for_user(&request.user_id, &request.evidence_id)
    }

    pub(crate) fn get_factor_dataset(
        &self,
        user_id: &str,
        dataset_id: &str,
    ) -> Result<FactorDataset, String> {
        validate_user(user_id)?;
        let database = self.database()?;
        ResearchStore::new(&database).factor_dataset_for_user(user_id, dataset_id)
    }

    pub(crate) fn dataset_rows(
        &self,
        request: FactorDatasetRowsRequest,
    ) -> Result<FactorDatasetRowsPage, String> {
        validate_user(&request.user_id)?;
        let database = self.database()?;
        ResearchStore::new(&database).dataset_rows(
            &request.user_id,
            &request.dataset_id,
            request.offset,
            request.limit,
            request.instrument_id.as_deref(),
        )
    }

    pub(crate) fn list_reports(
        &self,
        request: FactorPageRequest,
    ) -> Result<FactorPage<FactorReportView>, String> {
        validate_user(&request.user_id)?;
        let database = self.database()?;
        ResearchStore::new(&database).list_reports(&request)
    }

    pub(crate) fn get_report(
        &self,
        request: FactorEvidenceRequest,
    ) -> Result<FactorReportView, String> {
        validate_user(&request.user_id)?;
        let database = self.database()?;
        ResearchStore::new(&database).report_for_user(&request.user_id, &request.evidence_id)
    }

    pub(crate) fn register_family(
        &self,
        request: FactorFamilyRegisterRequest,
    ) -> Result<FactorFamilyView, String> {
        validate_user(&request.user_id)?;
        let user = user_uuid(&request.user_id);
        let registration = request.registration;
        if registration.family.user_id != user {
            return Err("Research Family User identity differs from the request".into());
        }
        registration.family.validate().map_err(string)?;
        for trial in &registration.trials {
            if trial.family_id != registration.family.family_id {
                return Err("Research Trial Family identity differs from the Family".into());
            }
            trial.validate().map_err(string)?;
        }
        for trial in &request.trials {
            trial.validate().map_err(string)?;
            if trial.family_id != registration.family.family_id {
                return Err("Research Trial Family identity differs from the Family".into());
            }
        }
        let database = self.database()?;
        ResearchStore::new(&database).save_family(&registration, &request.trials)
    }

    pub(crate) fn register_grid_family(
        &self,
        request: FactorGridFamilyRegisterRequest,
    ) -> Result<FactorAttemptView, String> {
        validate_user(&request.user_id)?;
        self.enqueue(&request.user_id, "factor-family-grid", &request)
    }

    fn execute_grid_family(
        &self,
        user_id: &str,
        attempt_id: &str,
        request_json: &str,
        cancelled: &AtomicBool,
    ) -> Result<String, String> {
        let request: FactorGridFamilyRegisterRequest =
            serde_json::from_str(request_json).map_err(string)?;
        if request.user_id != user_id || cancelled.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        let user = user_uuid(&request.user_id);
        let database = self.database()?;
        let store = ResearchStore::new(&database);
        store.candidate_for_user(&request.user_id, &request.candidate_hash)?;
        let mut registry = ResearchRegistry::default();
        if let Some(parent_family_id) = request.parent_family_id {
            let parent = store.family_registration(user, parent_family_id)?;
            registry
                .register_family(ResearchFamilyDraft {
                    family_id: parent.family.family_id,
                    user_id: parent.family.user_id,
                    root_candidate_hash: parent.family.root_candidate_hash.clone(),
                    parent_family_id: None,
                    trials: parent
                        .trials
                        .iter()
                        .map(|trial| ResearchTrialDraft {
                            trial_id: trial.trial_id,
                            candidate_hash: trial.candidate_hash.clone(),
                            parameter_set_hash: trial.parameter_set_hash.clone(),
                            target: trial.target,
                            market_context: trial.market_context.clone(),
                            point_in_time_universe_id: trial.point_in_time_universe_id.clone(),
                            observation_range: trial.observation_range.clone(),
                            evaluation_protocol_hash: trial.evaluation_protocol_hash.clone(),
                            derivation_hash: trial.derivation_hash.clone(),
                        })
                        .collect(),
                })
                .map_err(string)?;
        }
        let draft = GridSearchFamilyDraft {
            family_id: request.family_id,
            user_id: user,
            candidate_hash: request.candidate_hash,
            parent_family_id: request.parent_family_id,
            plan: adaq_factor_research::GridSearchPlan::new(request.parameters).map_err(string)?,
            target: request.target,
            market_context: request.market_context,
            point_in_time_universe_id: request.point_in_time_universe_id,
            observation_range: request.observation_range,
            base_protocol_hash: request.base_protocol_hash,
            derivation_hash: request.derivation_hash,
        };
        let registration = registry
            .register_grid_search_family(draft)
            .map_err(string)?;
        if cancelled.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        let trials = registration
            .family
            .trials
            .iter()
            .map(|trial| ResearchTrial {
                trial_id: trial.trial_id,
                family_id: trial.family_id,
                candidate_hash: trial.candidate_hash.clone(),
                protocol_hash: trial.evaluation_protocol_hash.clone(),
                status: adaq_factor_research::ResearchTrialStatus::Registered,
                report_hash: None,
                raw_statistic: None,
                p_value: None,
                holm_adjusted: None,
                related_trial_ids: Vec::new(),
                diagnostic: None,
            })
            .collect::<Vec<_>>();
        if cancelled.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        let family =
            store.save_family_for_attempt(user_id, attempt_id, &registration.family, &trials)?;
        Ok(family.family.family_id.to_string())
    }

    pub(crate) fn update_trial(&self, request: FactorTrialUpdateRequest) -> Result<(), String> {
        validate_user(&request.user_id)?;
        let database = self.database()?;
        ResearchStore::new(&database).save_trial(user_uuid(&request.user_id), &request.trial)
    }

    pub(crate) fn list_families(
        &self,
        request: FactorPageRequest,
    ) -> Result<FactorPage<FactorFamilyView>, String> {
        validate_user(&request.user_id)?;
        let database = self.database()?;
        ResearchStore::new(&database).list_families(&request)
    }

    pub(crate) fn get_family(
        &self,
        request: FactorEvidenceRequest,
    ) -> Result<FactorFamilyView, String> {
        validate_user(&request.user_id)?;
        let database = self.database()?;
        ResearchStore::new(&database)
            .family_for_user(user_uuid(&request.user_id), &request.evidence_id)
    }

    pub(crate) fn lineage(
        &self,
        request: FactorEvidenceRequest,
    ) -> Result<FactorLineageView, String> {
        validate_user(&request.user_id)?;
        let database = self.database()?;
        ResearchStore::new(&database)
            .lineage_for_user(user_uuid(&request.user_id), &request.evidence_id)
    }

    pub(crate) fn save_policy(
        &self,
        request: FactorPolicySaveRequest,
    ) -> Result<FactorPolicyView, String> {
        validate_user(&request.user_id)?;
        request.policy.validate().map_err(string)?;
        let database = self.database()?;
        ResearchStore::new(&database).save_policy(user_uuid(&request.user_id), &request.policy)
    }

    pub(crate) fn freeze_promotion_protocol(
        &self,
        request: FactorPromotionProtocolFreezeRequest,
    ) -> Result<PromotionProtocol, String> {
        self.ensure_schema_ready()?;
        validate_user(&request.user_id)?;
        let user_id = user_uuid(&request.user_id);
        let owner_id = request.user_id.clone();
        let database = self.database()?;
        ResearchStore::new(&database).freeze_promotion_protocol(&owner_id, user_id, request)
    }

    pub(crate) fn record_decision(
        &self,
        request: FactorDecisionRecordRequest,
    ) -> Result<FactorDecisionView, String> {
        self.ensure_schema_ready()?;
        validate_user(&request.user_id)?;
        let owner_id = request.user_id.clone();
        let user_id = user_uuid(&owner_id);
        let database = self.database()?;
        let store = ResearchStore::new(&database);
        if !matches!(request.state, PromotionDecisionState::Rejected) {
            let candidate = store
                .candidate_for_user(&owner_id, &request.promotion_protocol.candidate_hash)?
                .candidate;
            if let FactorCandidateSource::Python { binding } = &candidate.source {
                let evidence = store.python_host_evidence(&owner_id, &candidate.candidate_hash)?;
                Self::validate_python_host_evidence(&owner_id, binding, &evidence)?;
            }
        }
        store.record_decision(&owner_id, user_id, request)
    }

    pub(crate) fn list_policies(
        &self,
        request: FactorPageRequest,
    ) -> Result<FactorPage<FactorPolicyView>, String> {
        validate_user(&request.user_id)?;
        let database = self.database()?;
        ResearchStore::new(&database).list_policies(&request)
    }

    pub(crate) fn select_trial(
        &self,
        request: FactorTrialSelectionRequest,
    ) -> Result<FactorSelectionView, String> {
        validate_user(&request.user_id)?;
        let database = self.database()?;
        ResearchStore::new(&database).select_trial(&request)
    }

    pub(crate) fn selected_trial(
        &self,
        user_id: &str,
        candidate_hash: &str,
    ) -> Result<(FactorSelectionView, PromotionProtocol), String> {
        validate_user(user_id)?;
        let database = self.database()?;
        ResearchStore::new(&database).selected_trial(user_id, candidate_hash)
    }

    pub(crate) fn model_input_binding(
        &self,
        user_id: &str,
        decision_hash: &str,
    ) -> Result<FactorModelInputBinding, String> {
        validate_user(user_id)?;
        let database = self.database()?;
        ResearchStore::new(&database).model_input_binding(user_id, decision_hash)
    }

    pub(crate) fn save_decision(
        &self,
        request: FactorDecisionSaveRequest,
    ) -> Result<FactorDecisionView, String> {
        validate_user(&request.user_id)?;
        let owner_id = request.user_id.clone();
        let database = self.database()?;
        let store = ResearchStore::new(&database);
        if !matches!(
            &request.decision.state,
            adaq_factor_research::PromotionDecisionState::Rejected
        ) {
            let candidate = store
                .candidate_for_user(&owner_id, &request.decision.candidate_hash)?
                .candidate;
            if let FactorCandidateSource::Python { binding } = &candidate.source {
                let evidence = store.python_host_evidence(&owner_id, &candidate.candidate_hash)?;
                Self::validate_python_host_evidence(&owner_id, binding, &evidence)?;
            }
        }
        store.save_decision(&owner_id, user_uuid(&request.user_id), request)
    }

    pub(crate) fn list_decisions(
        &self,
        request: FactorPageRequest,
    ) -> Result<FactorPage<FactorDecisionView>, String> {
        validate_user(&request.user_id)?;
        let database = self.database()?;
        ResearchStore::new(&database).list_decisions(&request)
    }

    pub(crate) fn list_decision_library(
        &self,
        request: FactorPageRequest,
    ) -> Result<FactorPage<FactorDecisionView>, String> {
        validate_user(&request.user_id)?;
        let database = self.database()?;
        ResearchStore::new(&database).list_decision_library(&request)
    }

    pub(crate) fn add_reference(&self, request: FactorReferenceRequest) -> Result<(), String> {
        validate_user(&request.user_id)?;
        let database = self.database()?;
        ResearchStore::new(&database).add_reference(&request)
    }

    pub(crate) fn remove_reference(&self, request: FactorReferenceRequest) -> Result<(), String> {
        validate_user(&request.user_id)?;
        let database = self.database()?;
        ResearchStore::new(&database).remove_reference(&request)
    }

    pub(crate) fn delete_dataset(&self, request: FactorEvidenceRequest) -> Result<(), String> {
        validate_user(&request.user_id)?;
        let database = self.database()?;
        let store = ResearchStore::new(&database);
        let feature_dataset_id =
            store.feature_dataset_binding_for_user(&request.user_id, &request.evidence_id)?;
        store.delete_dataset(&request.user_id, &request.evidence_id)?;
        drop(database);
        self.inner.source.unreference_feature_dataset(
            &request.user_id,
            &feature_dataset_id,
            &feature_reference_id(&request.evidence_id),
        )
    }

    pub(crate) fn m12_eligibility(
        &self,
        request: FactorM12Request,
    ) -> Result<adaq_factor_research::M12Eligibility, String> {
        validate_user(&request.user_id)?;
        let database = self.database()?;
        ResearchStore::new(&database).m12_eligibility(
            &request.user_id,
            user_uuid(&request.user_id),
            &request.promotion_protocol,
        )
    }

    pub(crate) fn reset_for_device(&self) -> Result<(), String> {
        if self.inner.schema_blocked.load(Ordering::Acquire) {
            let database = self.inner.source.database()?;
            let directory = self.inner.source.dataset_directory()?;
            ResearchStore::new(&database).reset_device(&directory)?;
            drop(database);
            self.inner.schema_blocked.store(false, Ordering::Release);
            return Ok(());
        }
        let users = {
            let database = self.database()?;
            ResearchStore::new(&database).user_ids()?
        };
        for user_id in users {
            self.reset_for_user(&user_id)?;
        }
        Ok(())
    }

    pub(crate) fn reset_for_user(&self, user_id: &str) -> Result<(), String> {
        self.ensure_schema_ready()?;
        validate_user(user_id)?;
        {
            let _gate = self.inner.start_gate.lock().map_err(string)?;
            self.inner
                .reset_blocks
                .lock()
                .map_err(string)?
                .insert(user_id.to_owned());
        }
        let result = (|| -> Result<(), String> {
            let database = self.database()?;
            ResearchStore::new(&database).cancel_for_reset(user_id)?;
            drop(database);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
            loop {
                let active_for_user = {
                    let active = self.inner.active.lock().map_err(string)?;
                    let database = self.database()?;
                    let store = ResearchStore::new(&database);
                    active
                        .iter()
                        .filter_map(|(attempt_id, cancelled)| {
                            store
                                .attempt_user(attempt_id)
                                .ok()
                                .filter(|owner| owner == user_id)
                                .map(|_| cancelled.clone())
                        })
                        .collect::<Vec<_>>()
                };
                if active_for_user.is_empty() {
                    break;
                }
                for cancelled in active_for_user {
                    cancelled.store(true, Ordering::Relaxed);
                }
                if std::time::Instant::now() >= deadline {
                    return Err("Factor research User reset timed out waiting for workers".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            let database = self.database()?;
            let directory = self.inner.source.dataset_directory()?;
            let store = ResearchStore::new(&database);
            let feature_bindings = store.feature_dataset_bindings_for_user(user_id)?;
            store.reset_for_user(user_id, &directory)?;
            drop(database);
            for (dataset_id, feature_dataset_id) in feature_bindings {
                self.inner.source.unreference_feature_dataset(
                    user_id,
                    &feature_dataset_id,
                    &feature_reference_id(&dataset_id),
                )?;
            }
            Ok(())
        })();
        if let Ok(mut blocks) = self.inner.reset_blocks.lock() {
            blocks.remove(user_id);
        }
        result
    }

    fn run_attempt(&self, item: FactorQueueItem) -> QueueRunResult {
        let (user_id, kind, request_json) = {
            let Ok(database) = self.inner.source.database() else {
                return QueueRunResult::Retryable("Factor research database unavailable".into());
            };
            let value = match ResearchStore::new(&database).begin_attempt(&item.attempt_id) {
                Ok(value) => value,
                Err(error) if error.contains("no longer Pending") => {
                    return QueueRunResult::Stale;
                }
                Err(error) => return QueueRunResult::Retryable(error),
            };
            value
        };
        if self.inner.shutdown_requested.load(Ordering::Acquire) {
            if let Ok(database) = self.inner.source.database() {
                let store = ResearchStore::new(&database);
                let _ = store.request_shutdown_cancellation();
                let _ = store.cancel_running(&item.attempt_id, &user_id);
            }
            return QueueRunResult::Consumed;
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        if let Ok(mut active) = self.inner.active.lock() {
            active.insert(item.attempt_id.clone(), cancelled.clone());
        }
        let resetting = self
            .inner
            .reset_blocks
            .lock()
            .map(|blocks| blocks.contains(&user_id))
            .unwrap_or(true);
        if resetting {
            cancelled.store(true, Ordering::Relaxed);
        }
        let result = if cancelled.load(Ordering::Relaxed) {
            Err("cancelled".into())
        } else {
            match kind.as_str() {
                "candidate-build" => {
                    self.execute_candidate(&user_id, &item.attempt_id, &request_json, &cancelled)
                }
                "factor-materialization" => self.execute_materialization(
                    &user_id,
                    &item.attempt_id,
                    &request_json,
                    &cancelled,
                ),
                "factor-evaluation" => {
                    self.execute_evaluation(&user_id, &item.attempt_id, &request_json, &cancelled)
                }
                "factor-family-grid" => {
                    self.execute_grid_family(&user_id, &item.attempt_id, &request_json, &cancelled)
                }
                _ => Err("unknown Factor research Attempt kind".into()),
            }
        };
        if let Ok(mut active) = self.inner.active.lock() {
            active.remove(&item.attempt_id);
        }
        let Ok(database) = self.inner.source.database() else {
            return QueueRunResult::Retryable("Factor research database unavailable".into());
        };
        let store = ResearchStore::new(&database);
        let cancellation_requested = store
            .cancellation_requested(&item.attempt_id, &user_id)
            .unwrap_or(false);
        match result {
            Ok(result_id) => {
                if cancelled.load(Ordering::Relaxed) || cancellation_requested {
                    let _ = store.cancel_running(&item.attempt_id, &user_id);
                } else {
                    if !store
                        .complete_attempt(&item.attempt_id, &result_id)
                        .unwrap_or(false)
                    {
                        let _ = store.cancel_running(&item.attempt_id, &user_id);
                    }
                }
            }
            Err(error)
                if error == "cancelled"
                    || cancelled.load(Ordering::Relaxed)
                    || cancellation_requested =>
            {
                let _ = store.cancel_running(&item.attempt_id, &user_id);
            }
            Err(error) => {
                let _ = store.fail_attempt(&item.attempt_id, &safe_diagnostic(&error));
            }
        }
        QueueRunResult::Consumed
    }

    fn execute_candidate(
        &self,
        user_id: &str,
        attempt_id: &str,
        request_json: &str,
        cancelled: &AtomicBool,
    ) -> Result<String, String> {
        let job: CandidateBuildJob = serde_json::from_str(request_json).map_err(string)?;
        if job.user_id != user_id || cancelled.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        job.candidate.validate().map_err(string)?;
        job.presentation.validate().map_err(string)?;
        if let Some(build) = job.build {
            let template_build = match &job.candidate.source {
                FactorCandidateSource::Custom { build } => build,
                FactorCandidateSource::Declarative { .. } => {
                    return Err(
                        "controlled Candidate builds require a Custom Factor candidate schema"
                            .into(),
                    );
                }
                FactorCandidateSource::Python { .. } => {
                    return Err(
                        "controlled Candidate builds require a Custom Factor candidate schema"
                            .into(),
                    );
                }
            };
            let component_attempt_id = Uuid::parse_str(attempt_id)
                .map_err(|_| "Factor Candidate Build Attempt identity is invalid".to_owned())?;
            let worker =
                adaq_factor_research::spawn_controlled_candidate_build(CandidateBuildRequest {
                    attempt_id: component_attempt_id,
                    user_id: user_uuid(user_id),
                    project_root: build.project_root,
                    source_sha256: build.source_sha256,
                    sdk_version: build.sdk_version,
                    toolchain: build.toolchain,
                    target: build.target,
                    resource_policy: build.resource_policy,
                })
                .map_err(string)?;
            let result = worker.join();
            let build_result = result.result.ok_or_else(|| {
                result
                    .attempt
                    .diagnostic
                    .unwrap_or_else(|| "controlled Factor Candidate build failed".into())
            })?;
            if cancelled.load(Ordering::Relaxed) {
                return Err("cancelled".into());
            }
            if template_build.package_sha256 != build_result.provenance.package_sha256 {
                return Err(
                    "controlled Factor Candidate package hash differs from the frozen schema"
                        .into(),
                );
            }
            validate_built_package(&build_result.package, &job.candidate)?;
            let candidate = FactorCandidate::freeze(FactorCandidateDraft {
                candidate_id: job.candidate.candidate_id,
                revision: job.candidate.revision,
                scope: job.candidate.scope,
                feature_slots: job.candidate.feature_slots,
                parameters: job.candidate.parameters,
                outputs: job.candidate.outputs,
                source: FactorCandidateSource::Custom {
                    build: build_result.provenance,
                },
            })
            .map_err(string)?;
            let database = self.inner.source.database()?;
            return ResearchStore::new(&database).save_candidate_for_attempt(
                user_id,
                attempt_id,
                &candidate,
                &job.presentation,
            );
        }
        let database = self.database()?;
        ResearchStore::new(&database).save_candidate_for_attempt(
            user_id,
            attempt_id,
            &job.candidate,
            &job.presentation,
        )
    }

    fn execute_materialization(
        &self,
        user_id: &str,
        attempt_id: &str,
        request_json: &str,
        cancelled: &AtomicBool,
    ) -> Result<String, String> {
        let job: MaterializationJob = serde_json::from_str(request_json).map_err(string)?;
        if job.user_id != user_id || job.protocol.user_id != user_uuid(user_id) {
            return Err("Materialization User identity differs from the Attempt".into());
        }
        job.protocol.validate().map_err(string)?;
        if let Some(context) = job.context.as_ref() {
            self.validate_materialization_context_binding(user_id, &job.protocol, context)?;
        }
        let dataset = if let Some(input) = job.dataset {
            let dataset = input.into_dataset()?;
            if dataset.manifest.protocol_hash != job.protocol.protocol_hash {
                return Err("Factor Dataset Protocol identity differs from the Attempt".into());
            }
            dataset
        } else {
            let database = self.inner.source.database()?;
            let candidate = ResearchStore::new(&database)
                .candidate_for_user(user_id, &job.protocol.candidate_hash)?
                .candidate;
            drop(database);
            let feature_dataset = self
                .inner
                .source
                .feature_dataset(user_id, &job.protocol.feature_dataset_id)?;
            let universe = self
                .inner
                .source
                .point_in_time_universe(user_id, &job.protocol.point_in_time_universe_id)?;
            let custom_package = match &candidate.source {
                FactorCandidateSource::Custom { build } => Some(
                    self.inner
                        .source
                        .component_package(user_id, &build.package_sha256)?,
                ),
                FactorCandidateSource::Declarative { .. } => None,
                FactorCandidateSource::Python { .. } => None,
            };
            FactorMaterializer::materialize(FactorMaterializationInput {
                candidate: &candidate,
                protocol: &job.protocol,
                feature_dataset: &feature_dataset,
                point_in_time_universe: &universe,
                custom_package: custom_package.as_ref(),
            })
            .map_err(string)?
        };
        let feature_reference_id = feature_reference_id(&dataset.manifest.dataset_id);
        self.inner.source.reference_feature_dataset(
            user_id,
            &dataset.manifest.feature_dataset_id,
            &feature_reference_id,
        )?;
        if let Some(context) = job.context.as_ref() {
            if let Err(error) =
                self.validate_materialization_context_binding(user_id, &job.protocol, context)
            {
                let _ = self.inner.source.unreference_feature_dataset(
                    user_id,
                    &dataset.manifest.feature_dataset_id,
                    &feature_reference_id,
                );
                return Err(error);
            }
        }
        let database = self.inner.source.database()?;
        let directory = self.inner.source.dataset_directory()?;
        let result = ResearchStore::new(&database)
            .publish_dataset(user_id, attempt_id, &dataset, &directory, cancelled);
        drop(database);
        if result.is_err() {
            let _ = self.inner.source.unreference_feature_dataset(
                user_id,
                &dataset.manifest.feature_dataset_id,
                &feature_reference_id,
            );
        }
        result
    }

    fn execute_evaluation(
        &self,
        user_id: &str,
        attempt_id: &str,
        request_json: &str,
        cancelled: &AtomicBool,
    ) -> Result<String, String> {
        let job: EvaluationJob = serde_json::from_str(request_json).map_err(string)?;
        if job.user_id != user_id || job.protocol.user_id != user_uuid(user_id) {
            return Err("Evaluation User identity differs from the Attempt".into());
        }
        job.protocol.validate().map_err(string)?;
        let dataset = if let Some(input) = job.dataset {
            let dataset = input.into_dataset()?;
            if dataset.manifest.dataset_id != job.protocol.factor_dataset_id {
                return Err("Factor Dataset identity differs from the Evaluation Protocol".into());
            }
            dataset
        } else {
            let database = self.inner.source.database()?;
            ResearchStore::new(&database)
                .factor_dataset_for_user(user_id, &job.protocol.factor_dataset_id)?
        };
        let report = FactorEvaluator::evaluate(FactorEvaluationInput {
            dataset: &dataset,
            protocol: &job.protocol,
            market_series: &job.market_series,
            feature_evidence: job.feature_evidence.as_ref(),
        })
        .map_err(|error| error.to_string())?;
        if cancelled.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        let database = self.inner.source.database()?;
        let directory = self.inner.source.dataset_directory()?;
        ResearchStore::new(&database)
            .publish_report(user_id, attempt_id, &report, &directory, cancelled)
    }
}

fn validate_built_package(
    package: &ComponentPackage,
    candidate: &FactorCandidate,
) -> Result<(), String> {
    if package.manifest.kind != ComponentKind::Factor
        || package.manifest.factor_scope
            != Some(match candidate.scope {
                adaq_factor_research::FactorScope::TimeSeries => PackageFactorScope::TimeSeries,
                adaq_factor_research::FactorScope::CrossSectional => {
                    PackageFactorScope::CrossSectional
                }
            })
        || package
            .manifest
            .feature_slots
            .iter()
            .map(|slot| slot.name.as_str())
            .ne(candidate
                .feature_slots
                .iter()
                .map(|slot| slot.name.as_str()))
        || package.manifest.output_names
            != candidate
                .outputs
                .iter()
                .map(|output| output.name.clone())
                .collect::<Vec<_>>()
    {
        return Err("built Factor package does not match the frozen Candidate schema".into());
    }
    if package.manifest.parameters.len() != candidate.parameters.len()
        || package
            .manifest
            .parameters
            .iter()
            .zip(&candidate.parameters)
            .any(|(package, candidate)| {
                package.name != candidate.name
                    || package.default_value != candidate.default_value
                    || package.allowed_values != candidate.allowed_values
                    || !matches!(
                        (&package.parameter_type, candidate.parameter_type),
                        (
                            ParameterType::Decimal,
                            adaq_factor_research::FactorParameterType::Decimal
                        ) | (
                            ParameterType::Integer,
                            adaq_factor_research::FactorParameterType::Integer
                        ) | (
                            ParameterType::Boolean,
                            adaq_factor_research::FactorParameterType::Boolean
                        ) | (
                            ParameterType::String,
                            adaq_factor_research::FactorParameterType::Text
                        )
                    )
            })
    {
        return Err("built Factor parameters do not match the frozen Candidate schema".into());
    }
    Ok(())
}

impl ResearchQueueAdapter for FactorResearch {
    fn pending_attempts(&self) -> Result<Vec<QueueAdmission>, String> {
        self.ensure_schema_ready()?;
        let database = self.database()?;
        ResearchStore::new(&database)
            .pending_attempts()?
            .into_iter()
            .map(|(user_id, attempt_id)| {
                Ok(QueueAdmission {
                    user_id,
                    attempt_id,
                })
            })
            .collect()
    }

    fn execute(&self, ticket: QueueTicket) -> QueueRunResult {
        let item = FactorQueueItem {
            attempt_id: ticket.attempt_id,
        };
        self.run_attempt(item)
    }

    fn request_shutdown(&self) {
        if let Ok(database) = self.inner.source.database()
            && let Err(error) = ResearchStore::new(&database).request_shutdown_cancellation()
        {
            eprintln!("Factor research shutdown cancellation failed: {error}");
        }
        self.inner.shutdown_requested.store(true, Ordering::Release);
        if let Ok(active) = self.inner.active.lock() {
            for cancelled in active.values() {
                cancelled.store(true, Ordering::Relaxed);
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FactorDatasetRowsRequest {
    pub user_id: String,
    pub dataset_id: String,
    pub offset: u64,
    pub limit: u32,
    #[serde(default)]
    pub instrument_id: Option<String>,
}

fn string(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(0)
}

fn hash_bytes(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(crate) fn user_uuid(user_id: &str) -> Uuid {
    if let Ok(uuid) = Uuid::parse_str(user_id) {
        return uuid;
    }
    let digest = Sha256::digest(user_id.as_bytes());
    Uuid::from_bytes(digest[..16].try_into().expect("SHA-256 has enough bytes"))
}

fn safe_diagnostic(value: &str) -> String {
    let mut output = value
        .lines()
        .map(|line| {
            let mut line = line
                .replace("/Users/", "<private>/")
                .replace("/home/", "<private>/")
                .replace("C:\\Users\\", "<private>\\");
            for prefix in ["authorization:", "bearer ", "token=", "api_key=", "secret="] {
                if let Some(index) = line.to_ascii_lowercase().find(prefix) {
                    let end = index + prefix.len();
                    line.truncate(end);
                    line.push_str("<redacted>");
                }
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n");
    if output.len() > MAX_DIAGNOSTIC_BYTES {
        output.truncate(MAX_DIAGNOSTIC_BYTES);
        output.push('…');
    }
    output
}

struct ResearchStore<'a> {
    database: &'a Connection,
}

impl<'a> ResearchStore<'a> {
    fn new(database: &'a Connection) -> Self {
        Self { database }
    }

    fn user_ids(&self) -> Result<Vec<String>, String> {
        let mut statement = self
            .database
            .prepare(
                "SELECT user_id FROM factor_candidate_access
                 UNION SELECT user_id FROM factor_candidate_presentations
                 UNION SELECT user_id FROM factor_candidate_predecessors
                 UNION SELECT user_id FROM factor_research_attempts
                 ORDER BY user_id",
            )
            .map_err(string)?;
        statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(string)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(string)
    }

    fn initialize(&self) -> Result<(), String> {
        let meta_exists = self
            .database
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'factor_research_meta'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(string)?
            .is_some();
        self.database
            .execute_batch(
                "PRAGMA foreign_keys = ON;
                 CREATE TABLE IF NOT EXISTS factor_research_meta (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                 );",
            )
            .map_err(string)?;
        let stored = self
            .database
            .query_row(
                "SELECT value FROM factor_research_meta WHERE key = 'schema_version'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(string)?;
        if meta_exists && stored.is_none() {
            return Err(
                "reset-required: Factor research storage has no schema version; perform an explicit device-level reset"
                    .into(),
            );
        }
        if let Some(version) = stored.as_deref()
            && version != STORAGE_SCHEMA_VERSION
        {
            return Err(format!(
                "reset-required: incompatible Factor research storage schema {version}; perform an explicit device-level reset"
            ));
        }
        self.database
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS factor_candidate_content (
                    candidate_hash TEXT PRIMARY KEY,
                    candidate_json TEXT NOT NULL,
                    created_at_ms INTEGER NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS factor_candidate_access (
                    user_id TEXT NOT NULL,
                    candidate_hash TEXT NOT NULL,
                    PRIMARY KEY(user_id, candidate_hash)
                 );
                 CREATE TABLE IF NOT EXISTS factor_candidate_presentations (
                    user_id TEXT NOT NULL,
                    candidate_hash TEXT NOT NULL,
                    presentation_json TEXT NOT NULL,
                    PRIMARY KEY(user_id, candidate_hash)
                 );
                 CREATE TABLE IF NOT EXISTS factor_candidate_predecessors (
                    user_id TEXT NOT NULL,
                    candidate_hash TEXT NOT NULL,
                    predecessor_json TEXT NOT NULL,
                    PRIMARY KEY(user_id, candidate_hash)
                 );
                 CREATE TABLE IF NOT EXISTS factor_python_host_evidence (
                    candidate_hash TEXT PRIMARY KEY,
                    user_id TEXT NOT NULL,
                    evidence_json TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS factor_research_protocols (
                    protocol_hash TEXT PRIMARY KEY,
                    user_id TEXT NOT NULL,
                    kind TEXT NOT NULL,
                    protocol_json TEXT NOT NULL,
                    created_at_ms INTEGER NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS factor_research_attempts (
                    attempt_id TEXT PRIMARY KEY,
                    user_id TEXT NOT NULL,
                    kind TEXT NOT NULL,
                    request_hash TEXT NOT NULL,
                    status TEXT NOT NULL,
                    source_attempt_id TEXT,
                    request_json TEXT NOT NULL,
                    result_id TEXT,
                    completed_units INTEGER NOT NULL DEFAULT 0,
                    progress_total INTEGER NOT NULL DEFAULT 0,
                    diagnostic TEXT,
                    queue_order INTEGER NOT NULL,
                    created_at_ms INTEGER NOT NULL,
                    updated_at_ms INTEGER NOT NULL
                 );
                 CREATE INDEX IF NOT EXISTS factor_research_attempt_queue
                    ON factor_research_attempts(status, queue_order);
                 CREATE TABLE IF NOT EXISTS factor_dataset_content (
                    dataset_id TEXT PRIMARY KEY,
                    manifest_json TEXT NOT NULL,
                    parquet_path TEXT NOT NULL,
                    payload_sha256 TEXT NOT NULL,
                    parquet_sha256 TEXT NOT NULL,
                    byte_size INTEGER NOT NULL,
                    created_at_ms INTEGER NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS factor_dataset_access (
                    user_id TEXT NOT NULL,
                    dataset_id TEXT NOT NULL,
                    PRIMARY KEY(user_id, dataset_id)
                 );
                 CREATE TABLE IF NOT EXISTS factor_research_families (
                    family_id TEXT PRIMARY KEY,
                    user_id TEXT NOT NULL,
                    family_json TEXT NOT NULL,
                    created_at_ms INTEGER NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS factor_research_registrations (
                    trial_id TEXT PRIMARY KEY,
                    family_id TEXT NOT NULL,
                    user_id TEXT NOT NULL,
                    registration_json TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS factor_research_trials (
                    trial_id TEXT PRIMARY KEY,
                    family_id TEXT NOT NULL,
                    user_id TEXT NOT NULL,
                    trial_json TEXT NOT NULL,
                    updated_at_ms INTEGER NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS factor_evaluation_reports (
                    report_hash TEXT PRIMARY KEY,
                    user_id TEXT NOT NULL,
                    report_json TEXT NOT NULL,
                    parquet_path TEXT NOT NULL,
                    created_at_ms INTEGER NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS factor_evaluation_report_access (
                    user_id TEXT NOT NULL,
                    report_hash TEXT NOT NULL,
                    PRIMARY KEY(user_id, report_hash)
                 );
                 CREATE TABLE IF NOT EXISTS factor_promotion_policies (
                    policy_hash TEXT PRIMARY KEY,
                    user_id TEXT NOT NULL,
                    policy_json TEXT NOT NULL,
                    created_at_ms INTEGER NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS factor_promotion_decisions (
                    decision_id TEXT PRIMARY KEY,
                    user_id TEXT NOT NULL,
                    decision_hash TEXT NOT NULL,
                    record_json TEXT NOT NULL,
                    promotion_protocol_hash TEXT NOT NULL,
                    created_at_ms INTEGER NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS factor_parameter_selections (
                    selection_hash TEXT PRIMARY KEY,
                    user_id TEXT NOT NULL,
                    candidate_hash TEXT NOT NULL,
                    family_id TEXT NOT NULL,
                    trial_id TEXT NOT NULL,
                    promotion_protocol_hash TEXT NOT NULL,
                    created_at_ms INTEGER NOT NULL,
                    UNIQUE(user_id, candidate_hash)
                 );
                 CREATE TABLE IF NOT EXISTS factor_references (
                    evidence_kind TEXT NOT NULL,
                    evidence_id TEXT NOT NULL,
                    referencing_user_id TEXT NOT NULL,
                    reference_id TEXT NOT NULL,
                    PRIMARY KEY(evidence_kind, evidence_id, referencing_user_id, reference_id)
                 );",
            )
            .map_err(string)?;
        if stored.is_none() {
            self.database
                .execute(
                    "INSERT INTO factor_research_meta(key, value) VALUES ('schema_version', ?1)",
                    [STORAGE_SCHEMA_VERSION],
                )
                .map_err(string)?;
        }
        Ok(())
    }

    fn recover_stale_attempts(
        directory: &Path,
        _source: &Arc<dyn FactorResearchSource>,
    ) -> Result<(), String> {
        let database = _source.database()?;
        database
            .execute(
                "UPDATE factor_research_attempts
                    SET status = CASE WHEN diagnostic = ?2 THEN 'cancelled' ELSE 'failed' END,
                        diagnostic = CASE WHEN diagnostic = ?2 THEN ?3 ELSE ?1 END,
                        updated_at_ms = ?4
                  WHERE status = 'running'",
                params![
                    "research-interrupted: the previous worker stopped before publication",
                    CANCELLATION_REQUESTED_DIAGNOSTIC,
                    "Factor research Attempt cancelled",
                    now_ms()
                ],
            )
            .map_err(string)?;
        drop(database);
        for staging_directory in [directory.to_path_buf(), directory.join("reports")] {
            if let Ok(entries) = fs::read_dir(staging_directory) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().is_some_and(|extension| extension == "tmp") {
                        let _ = fs::remove_file(path);
                    }
                }
            }
        }
        Ok(())
    }

    fn start_attempt(
        &self,
        user_id: &str,
        kind: &str,
        request_json: &str,
    ) -> Result<(FactorAttemptView, bool), String> {
        let request_hash = hash_bytes(request_json.as_bytes());
        if let Some(attempt) = self
            .database
            .query_row(
                "SELECT attempt_id, user_id, kind, request_hash, status,
                        source_attempt_id, result_id, completed_units, progress_total,
                        diagnostic, created_at_ms, updated_at_ms
                   FROM factor_research_attempts
                  WHERE user_id = ?1 AND kind = ?2 AND request_hash = ?3
                    AND status IN ('pending', 'running', 'completed')
                  ORDER BY CASE status WHEN 'pending' THEN 0 WHEN 'running' THEN 1 ELSE 2 END,
                           queue_order DESC LIMIT 1",
                params![user_id, kind, request_hash],
                row_to_attempt,
            )
            .optional()
            .map_err(string)?
        {
            return Ok((attempt, false));
        }
        let now = now_ms();
        let queue_order = self
            .database
            .query_row(
                "SELECT COALESCE(MAX(queue_order), 0) + 1 FROM factor_research_attempts",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(string)?;
        let attempt_id = Uuid::new_v4().to_string();
        self.database
            .execute(
                "INSERT INTO factor_research_attempts(
                    attempt_id, user_id, kind, request_hash, status,
                    request_json, queue_order, created_at_ms, updated_at_ms
                 ) VALUES (?1, ?2, ?3, ?4, 'pending', ?5, ?6, ?7, ?7)",
                params![
                    attempt_id,
                    user_id,
                    kind,
                    request_hash,
                    request_json,
                    queue_order,
                    now
                ],
            )
            .map_err(string)?;
        Ok((
            FactorAttemptView {
                attempt_id,
                user_id: user_id.into(),
                kind: kind.into(),
                request_hash,
                status: AttemptStatus::Pending,
                source_attempt_id: None,
                result_id: None,
                completed_units: 0,
                progress_total: 0,
                diagnostic: None,
                failure_code: None,
                created_at_ms: now,
                updated_at_ms: now,
            },
            true,
        ))
    }

    fn pending_attempts(&self) -> Result<Vec<(String, String)>, String> {
        let mut statement = self
            .database
            .prepare(
                "SELECT user_id, attempt_id
                   FROM factor_research_attempts
                  WHERE status = 'pending'
                  ORDER BY queue_order, attempt_id",
            )
            .map_err(string)?;
        let rows = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(string)?;
        rows.map(|row| row.map_err(string)).collect()
    }

    fn begin_attempt(&self, attempt_id: &str) -> Result<(String, String, String), String> {
        let changed = self
            .database
            .execute(
                "UPDATE factor_research_attempts
                    SET status = 'running', updated_at_ms = ?2
                  WHERE attempt_id = ?1 AND status = 'pending'",
                params![attempt_id, now_ms()],
            )
            .map_err(string)?;
        if changed == 0 {
            return Err("Factor research Attempt is no longer Pending".into());
        }
        self.database
            .query_row(
                "SELECT user_id, kind, request_json FROM factor_research_attempts WHERE attempt_id = ?1",
                [attempt_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(string)
    }

    fn attempt_user(&self, attempt_id: &str) -> Result<String, String> {
        self.database
            .query_row(
                "SELECT user_id FROM factor_research_attempts WHERE attempt_id = ?1",
                [attempt_id],
                |row| row.get(0),
            )
            .map_err(string)
    }

    fn cancel_for_reset(&self, user_id: &str) -> Result<(), String> {
        self.database
            .execute(
                "UPDATE factor_research_attempts
                    SET status = 'cancelled', diagnostic = ?2, updated_at_ms = ?3
                  WHERE user_id = ?1 AND status = 'pending'",
                params![
                    user_id,
                    "Factor research Attempt cancelled by explicit User reset",
                    now_ms()
                ],
            )
            .map_err(string)?;
        self.database
            .execute(
                "UPDATE factor_research_attempts
                    SET diagnostic = ?2, updated_at_ms = ?3
                  WHERE user_id = ?1 AND status = 'running'",
                params![user_id, CANCELLATION_REQUESTED_DIAGNOSTIC, now_ms()],
            )
            .map(|_| ())
            .map_err(string)
    }

    fn complete_attempt(&self, attempt_id: &str, result_id: &str) -> Result<bool, String> {
        self.database
            .execute(
                "UPDATE factor_research_attempts
                    SET status = 'completed', result_id = ?2,
                        completed_units = CASE WHEN progress_total = 0 THEN 1 ELSE progress_total END,
                        progress_total = CASE WHEN progress_total = 0 THEN 1 ELSE progress_total END,
                        updated_at_ms = ?3
                  WHERE attempt_id = ?1 AND status = 'running'
                    AND (diagnostic IS NULL OR diagnostic != ?4)",
                params![attempt_id, result_id, now_ms(), CANCELLATION_REQUESTED_DIAGNOSTIC],
            )
            .map(|changed| changed == 1)
            .map_err(string)
    }

    fn cancellation_requested(&self, attempt_id: &str, user_id: &str) -> Result<bool, String> {
        self.database
            .query_row(
                "SELECT diagnostic FROM factor_research_attempts
                  WHERE attempt_id = ?1 AND user_id = ?2",
                params![attempt_id, user_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map(|diagnostic| {
                diagnostic.flatten().as_deref() == Some(CANCELLATION_REQUESTED_DIAGNOSTIC)
            })
            .map_err(string)
    }

    fn cancel_running(&self, attempt_id: &str, user_id: &str) -> Result<(), String> {
        self.database
            .execute(
                "UPDATE factor_research_attempts
                    SET status = 'cancelled', diagnostic = ?3, updated_at_ms = ?4
                  WHERE attempt_id = ?1 AND user_id = ?2 AND status = 'running'",
                params![
                    attempt_id,
                    user_id,
                    "Factor research Attempt cancelled",
                    now_ms()
                ],
            )
            .map(|_| ())
            .map_err(string)
    }

    fn request_shutdown_cancellation(&self) -> Result<(), String> {
        self.database
            .execute(
                "UPDATE factor_research_attempts
                    SET diagnostic = ?1, updated_at_ms = ?2
                  WHERE status = 'running'
                    AND (diagnostic IS NULL OR diagnostic != ?1)",
                params![CANCELLATION_REQUESTED_DIAGNOSTIC, now_ms()],
            )
            .map(|_| ())
            .map_err(string)
    }

    fn fail_attempt(&self, attempt_id: &str, diagnostic: &str) -> Result<(), String> {
        self.database
            .execute(
                "UPDATE factor_research_attempts
                    SET status = 'failed', diagnostic = ?2, updated_at_ms = ?3
                  WHERE attempt_id = ?1 AND status IN ('pending', 'running')",
                params![attempt_id, safe_diagnostic(diagnostic), now_ms()],
            )
            .map(|_| ())
            .map_err(string)
    }

    fn cancel_attempt(&self, user_id: &str, attempt_id: &str) -> Result<AttemptStatus, String> {
        let status = self
            .database
            .query_row(
                "SELECT status FROM factor_research_attempts WHERE attempt_id = ?1 AND user_id = ?2",
                params![attempt_id, user_id],
                |row| row.get::<_, String>(0),
            )
            .map_err(|_| "Factor research Attempt was not found".to_owned())?;
        let status = parse_status(&status)?;
        match status {
            AttemptStatus::Pending => {
                let changed = self
                    .database
                    .execute(
                        "UPDATE factor_research_attempts
                            SET status = 'cancelled', diagnostic = ?3, updated_at_ms = ?4
                          WHERE attempt_id = ?1 AND user_id = ?2 AND status = 'pending'",
                        params![
                            attempt_id,
                            user_id,
                            "Factor research Attempt cancelled",
                            now_ms()
                        ],
                    )
                    .map_err(string)?;
                if changed != 1 {
                    return Err("Factor research Attempt cannot be cancelled".into());
                }
                Ok(AttemptStatus::Cancelled)
            }
            AttemptStatus::Running => {
                let changed = self
                    .database
                    .execute(
                        "UPDATE factor_research_attempts
                            SET diagnostic = ?3, updated_at_ms = ?4
                          WHERE attempt_id = ?1 AND user_id = ?2 AND status = 'running'",
                        params![
                            attempt_id,
                            user_id,
                            CANCELLATION_REQUESTED_DIAGNOSTIC,
                            now_ms()
                        ],
                    )
                    .map_err(string)?;
                if changed == 1 || self.cancellation_requested(attempt_id, user_id)? {
                    Ok(AttemptStatus::Running)
                } else {
                    Err("Factor research Attempt cannot be cancelled".into())
                }
            }
            AttemptStatus::Cancelled => Ok(AttemptStatus::Cancelled),
            _ => Err("Factor research Attempt cannot be cancelled".into()),
        }
    }

    fn retry_attempt(
        &self,
        user_id: &str,
        attempt_id: &str,
    ) -> Result<(FactorAttemptView, bool), String> {
        let (kind, request_hash, request_json, status): (String, String, String, String) = self
            .database
            .query_row(
                "SELECT kind, request_hash, request_json, status
                   FROM factor_research_attempts
                  WHERE attempt_id = ?1 AND user_id = ?2",
                params![attempt_id, user_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .map_err(|_| "Factor research Attempt cannot be retried".to_owned())?;
        if !matches!(
            parse_status(&status)?,
            AttemptStatus::Failed | AttemptStatus::Cancelled
        ) {
            return Err("only Failed or Cancelled Factor research Attempts can be retried".into());
        }
        if let Some(existing) = self
            .database
            .query_row(
                "SELECT attempt_id, user_id, kind, request_hash, status,
                        source_attempt_id, result_id, completed_units, progress_total,
                        diagnostic, created_at_ms, updated_at_ms
                   FROM factor_research_attempts
                  WHERE user_id = ?1 AND kind = ?2 AND request_hash = ?3
                    AND status IN ('pending', 'running')
                  ORDER BY queue_order LIMIT 1",
                params![user_id, kind, request_hash],
                row_to_attempt,
            )
            .optional()
            .map_err(string)?
        {
            return Ok((existing, false));
        }
        let now = now_ms();
        let queue_order = self
            .database
            .query_row(
                "SELECT COALESCE(MAX(queue_order), 0) + 1 FROM factor_research_attempts",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(string)?;
        let retry_id = Uuid::new_v4().to_string();
        self.database
            .execute(
                "INSERT INTO factor_research_attempts(
                    attempt_id, user_id, kind, request_hash, status,
                    source_attempt_id, request_json, queue_order, created_at_ms, updated_at_ms
                 ) VALUES (?1, ?2, ?3, ?4, 'pending', ?5, ?6, ?7, ?8, ?8)",
                params![
                    retry_id,
                    user_id,
                    kind,
                    request_hash,
                    attempt_id,
                    request_json,
                    queue_order,
                    now
                ],
            )
            .map_err(string)?;
        Ok((
            FactorAttemptView {
                attempt_id: retry_id,
                user_id: user_id.into(),
                kind,
                request_hash,
                status: AttemptStatus::Pending,
                source_attempt_id: Some(attempt_id.into()),
                result_id: None,
                completed_units: 0,
                progress_total: 0,
                diagnostic: None,
                failure_code: None,
                created_at_ms: now,
                updated_at_ms: now,
            },
            true,
        ))
    }

    fn attempt_for_user(
        &self,
        user_id: &str,
        attempt_id: &str,
    ) -> Result<FactorAttemptView, String> {
        self.database
            .query_row(
                "SELECT attempt_id, user_id, kind, request_hash, status,
                        source_attempt_id, result_id, completed_units, progress_total,
                        diagnostic, created_at_ms, updated_at_ms
                   FROM factor_research_attempts
                  WHERE user_id = ?1 AND attempt_id = ?2",
                params![user_id, attempt_id],
                row_to_attempt,
            )
            .map_err(|_| "Factor research Attempt was not found".to_owned())
    }

    fn list_attempts(
        &self,
        request: &FactorPageRequest,
    ) -> Result<FactorPage<FactorAttemptView>, String> {
        let (page, limit, offset) = page_params(request)?;
        let total = self
            .database
            .query_row(
                "SELECT COUNT(*) FROM factor_research_attempts
                  WHERE user_id = ?1 AND (?2 IS NULL OR kind = ?2)",
                params![request.user_id, request.kind],
                |row| row.get::<_, i64>(0),
            )
            .map_err(string)? as u64;
        let mut statement = self
            .database
            .prepare(
                "SELECT attempt_id, user_id, kind, request_hash, status,
                        source_attempt_id, result_id, completed_units, progress_total,
                        diagnostic, created_at_ms, updated_at_ms
                   FROM factor_research_attempts
                  WHERE user_id = ?1 AND (?2 IS NULL OR kind = ?2)
                  ORDER BY queue_order DESC LIMIT ?3 OFFSET ?4",
            )
            .map_err(string)?;
        let items = statement
            .query_map(
                params![request.user_id, request.kind, limit as i64, offset as i64],
                row_to_attempt,
            )
            .map_err(string)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(string)?;
        Ok(FactorPage {
            items,
            page,
            page_size: limit,
            total,
        })
    }

    fn save_materialization_protocol(
        &self,
        protocol: &FactorMaterializationProtocol,
    ) -> Result<(), String> {
        self.save_protocol(
            &protocol.protocol_hash,
            &protocol.user_id.to_string(),
            "materialization",
            protocol,
        )
    }

    fn save_evaluation_protocol(&self, protocol: &FactorEvaluationProtocol) -> Result<(), String> {
        self.save_protocol(
            &protocol.protocol_hash,
            &protocol.user_id.to_string(),
            "evaluation",
            protocol,
        )
    }

    fn ensure_evaluation_trial(
        &self,
        user_id: &str,
        protocol: &FactorEvaluationProtocol,
        manifest: &FactorDatasetManifest,
        require_exact_protocol: bool,
    ) -> Result<(), String> {
        let user = user_uuid(user_id);
        let range = evaluation_observation_range(protocol)?;
        let registration_json: Option<String> = self
            .database
            .query_row(
                "SELECT registration_json FROM factor_research_registrations
                 WHERE trial_id = ?1 AND user_id = ?2",
                params![protocol.trial_id.to_string(), user.to_string()],
                |row| row.get(0),
            )
            .optional()
            .map_err(string)?;
        if let Some(registration_json) = registration_json {
            let registration: ResearchTrialRegistration =
                serde_json::from_str(&registration_json).map_err(string)?;
            registration.validate().map_err(string)?;
            if registration.family_id != protocol.family_id
                || registration.candidate_hash != manifest.candidate_hash
                || registration.target != protocol.target
                || registration.market_context != protocol.market_context
                || registration.point_in_time_universe_id != protocol.point_in_time_universe_id
                || registration.observation_range != range
                || (require_exact_protocol
                    && registration.evaluation_protocol_hash != protocol.protocol_hash)
            {
                return Err(
                    "Factor Evaluation Protocol is not bound to its registered Trial".into(),
                );
            }
            let existing: Option<String> = self
                .database
                .query_row(
                    "SELECT trial_json FROM factor_research_trials
                     WHERE trial_id = ?1 AND user_id = ?2",
                    params![protocol.trial_id.to_string(), user.to_string()],
                    |row| row.get(0),
                )
                .optional()
                .map_err(string)?;
            if let Some(existing) = existing {
                let trial: ResearchTrial = serde_json::from_str(&existing).map_err(string)?;
                trial.validate().map_err(string)?;
                if trial.family_id != registration.family_id
                    || trial.candidate_hash != registration.candidate_hash
                    || trial.protocol_hash != registration.evaluation_protocol_hash
                {
                    return Err("Research Trial state is not bound to its registration".into());
                }
            } else {
                let trial = initial_trial(&registration);
                self.database
                    .execute(
                        "INSERT INTO factor_research_trials(
                            trial_id, family_id, user_id, trial_json, updated_at_ms
                         ) VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![
                            trial.trial_id.to_string(),
                            trial.family_id.to_string(),
                            user.to_string(),
                            serde_json::to_string(&trial).map_err(string)?,
                            now_ms()
                        ],
                    )
                    .map_err(string)?;
            }
            return Ok(());
        }

        let family_exists: Option<String> = self
            .database
            .query_row(
                "SELECT family_json FROM factor_research_families
                 WHERE family_id = ?1 AND user_id = ?2",
                params![protocol.family_id.to_string(), user.to_string()],
                |row| row.get(0),
            )
            .optional()
            .map_err(string)?;
        if family_exists.is_some() {
            return Err("Research Family does not contain the requested Trial".into());
        }

        let parameter_set_hash = hash_bytes(protocol.protocol_hash.as_bytes());
        let mut registry = ResearchRegistry::default();
        let registration = registry
            .register_family(ResearchFamilyDraft {
                family_id: protocol.family_id,
                user_id: user,
                root_candidate_hash: manifest.candidate_hash.clone(),
                parent_family_id: None,
                trials: vec![ResearchTrialDraft {
                    trial_id: protocol.trial_id,
                    candidate_hash: manifest.candidate_hash.clone(),
                    parameter_set_hash,
                    target: protocol.target,
                    market_context: protocol.market_context.clone(),
                    point_in_time_universe_id: protocol.point_in_time_universe_id.clone(),
                    observation_range: range,
                    evaluation_protocol_hash: protocol.protocol_hash.clone(),
                    derivation_hash: None,
                }],
            })
            .map_err(string)?;
        let trials = registration
            .trials
            .iter()
            .map(initial_trial)
            .collect::<Vec<_>>();
        let transaction = self.database.unchecked_transaction().map_err(string)?;
        save_family_records(&transaction, &registration, &trials)?;
        transaction.commit().map_err(string)
    }

    fn save_protocol<T: Serialize>(
        &self,
        hash: &str,
        user_id: &str,
        kind: &str,
        protocol: &T,
    ) -> Result<(), String> {
        let json = String::from_utf8(canonical_json(protocol).map_err(string)?).map_err(string)?;
        let stored: Option<String> = self
            .database
            .query_row(
                "SELECT protocol_json FROM factor_research_protocols WHERE protocol_hash = ?1",
                [hash],
                |row| row.get(0),
            )
            .optional()
            .map_err(string)?;
        match stored {
            Some(existing) if existing != json => {
                Err("Factor research Protocol content identity collision".into())
            }
            Some(_) => Ok(()),
            None => self
                .database
                .execute(
                    "INSERT INTO factor_research_protocols(protocol_hash, user_id, kind, protocol_json, created_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![hash, user_id, kind, json, now_ms()],
                )
                .map(|_| ())
                .map_err(string),
        }
    }

    fn save_candidate(
        &self,
        user_id: &str,
        candidate: &FactorCandidate,
        presentation: &FactorPresentationMetadata,
    ) -> Result<String, String> {
        self.save_candidate_inner(user_id, candidate, presentation, None)
    }

    fn save_candidate_for_attempt(
        &self,
        user_id: &str,
        attempt_id: &str,
        candidate: &FactorCandidate,
        presentation: &FactorPresentationMetadata,
    ) -> Result<String, String> {
        let (candidate_json, presentation_json, predecessor_json) =
            candidate_save_payload(user_id, candidate, presentation, None)?;
        let transaction = self.database.unchecked_transaction().map_err(string)?;
        let publishable: i64 = transaction
            .query_row(
                "SELECT COUNT(*) FROM factor_research_attempts
                  WHERE attempt_id = ?1 AND user_id = ?2 AND kind = 'candidate-build'
                    AND status = 'running'
                    AND (diagnostic IS NULL OR diagnostic != ?3)",
                params![attempt_id, user_id, CANCELLATION_REQUESTED_DIAGNOSTIC],
                |row| row.get(0),
            )
            .map_err(string)?;
        if publishable != 1 {
            return Err("cancelled".into());
        }
        save_candidate_records(
            &transaction,
            user_id,
            candidate,
            &candidate_json,
            &presentation_json,
            predecessor_json.as_deref(),
        )?;
        let changed = transaction
            .execute(
                "UPDATE factor_research_attempts
                    SET status = 'completed', result_id = ?2,
                        completed_units = 1, progress_total = 1, updated_at_ms = ?3
                  WHERE attempt_id = ?1 AND user_id = ?4 AND kind = 'candidate-build'
                    AND status = 'running'
                    AND (diagnostic IS NULL OR diagnostic != ?5)",
                params![
                    attempt_id,
                    candidate.candidate_hash,
                    now_ms(),
                    user_id,
                    CANCELLATION_REQUESTED_DIAGNOSTIC
                ],
            )
            .map_err(string)?;
        if changed != 1 {
            return Err("cancelled".into());
        }
        transaction.commit().map_err(string)?;
        Ok(candidate.candidate_hash.clone())
    }

    fn save_candidate_with_predecessor(
        &self,
        user_id: &str,
        candidate: &FactorCandidate,
        presentation: &FactorPresentationMetadata,
        predecessor: &FactorCandidatePredecessor,
    ) -> Result<String, String> {
        self.save_candidate_inner(user_id, candidate, presentation, Some(predecessor))
    }

    fn save_candidate_inner(
        &self,
        user_id: &str,
        candidate: &FactorCandidate,
        presentation: &FactorPresentationMetadata,
        predecessor: Option<&FactorCandidatePredecessor>,
    ) -> Result<String, String> {
        let (candidate_json, presentation_json, predecessor_json) =
            candidate_save_payload(user_id, candidate, presentation, predecessor)?;
        let transaction = self.database.unchecked_transaction().map_err(string)?;
        save_candidate_records(
            &transaction,
            user_id,
            candidate,
            &candidate_json,
            &presentation_json,
            predecessor_json.as_deref(),
        )?;
        transaction.commit().map_err(string)?;
        Ok(candidate.candidate_hash.clone())
    }

    fn save_candidate_with_python_evidence(
        &self,
        user_id: &str,
        candidate: &FactorCandidate,
        presentation: &FactorPresentationMetadata,
        evidence: &PythonHostEvidence,
    ) -> Result<String, String> {
        let candidate_hash = self.save_candidate(user_id, candidate, presentation)?;
        let evidence_json = serde_json::to_string(evidence).map_err(string)?;
        self.database
            .execute(
                "INSERT INTO factor_python_host_evidence(candidate_hash, user_id, evidence_json)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(candidate_hash) DO UPDATE SET user_id = excluded.user_id,
                 evidence_json = excluded.evidence_json",
                params![candidate_hash, user_id, evidence_json],
            )
            .map_err(string)?;
        Ok(candidate_hash)
    }

    fn python_host_evidence(
        &self,
        user_id: &str,
        candidate_hash: &str,
    ) -> Result<PythonHostEvidence, String> {
        let evidence_json: String = self
            .database
            .query_row(
                "SELECT evidence_json FROM factor_python_host_evidence
                  WHERE user_id = ?1 AND candidate_hash = ?2",
                params![user_id, candidate_hash],
                |row| row.get(0),
            )
            .map_err(|_| "Python Candidate Host evidence was not found".to_owned())?;
        serde_json::from_str(&evidence_json).map_err(string)
    }

    fn candidate_for_user(
        &self,
        user_id: &str,
        candidate_hash: &str,
    ) -> Result<FactorCandidateView, String> {
        let (candidate_json, presentation_json, created_at, predecessor_json): (
            String,
            String,
            i64,
            Option<String>,
        ) = self
            .database
            .query_row(
                "SELECT c.candidate_json, p.presentation_json, c.created_at_ms,
                        predecessor.predecessor_json
                   FROM factor_candidate_access a
                   JOIN factor_candidate_content c USING(candidate_hash)
                   JOIN factor_candidate_presentations p
                     ON p.user_id = a.user_id AND p.candidate_hash = a.candidate_hash
                   LEFT JOIN factor_candidate_predecessors predecessor
                     ON predecessor.user_id = a.user_id
                    AND predecessor.candidate_hash = a.candidate_hash
                  WHERE a.user_id = ?1 AND a.candidate_hash = ?2",
                params![user_id, candidate_hash],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .map_err(|_| "Factor Candidate was not found".to_owned())?;
        let candidate = FactorCandidate::load(candidate_json.as_bytes()).map_err(string)?;
        let presentation = serde_json::from_str(&presentation_json).map_err(string)?;
        let predecessor: Option<FactorCandidatePredecessor> = predecessor_json
            .map(|json| serde_json::from_str(&json).map_err(string))
            .transpose()?;
        if let Some(predecessor) = &predecessor {
            predecessor.validate()?;
            if predecessor.user_id != user_id {
                return Err(
                    "Factor Candidate predecessor User identity differs from the owner".into(),
                );
            }
        }
        let locked_by = self.locked_by(user_id, "candidate", candidate_hash)?;
        Ok(FactorCandidateView {
            candidate,
            presentation,
            locked_by,
            created_at_ms: created_at,
            predecessor,
        })
    }

    fn list_candidates(
        &self,
        request: &FactorPageRequest,
    ) -> Result<FactorPage<FactorCandidateView>, String> {
        let (page, limit, offset) = page_params(request)?;
        let total = self
            .database
            .query_row(
                "SELECT COUNT(*) FROM factor_candidate_access WHERE user_id = ?1",
                [&request.user_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(string)? as u64;
        let mut statement = self
            .database
            .prepare(
                "SELECT a.candidate_hash FROM factor_candidate_access a
                  WHERE a.user_id = ?1 ORDER BY a.candidate_hash LIMIT ?2 OFFSET ?3",
            )
            .map_err(string)?;
        let hashes = statement
            .query_map(
                params![request.user_id, limit as i64, offset as i64],
                |row| row.get::<_, String>(0),
            )
            .map_err(string)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(string)?;
        let items = hashes
            .iter()
            .map(|hash| self.candidate_for_user(&request.user_id, hash))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(FactorPage {
            items,
            page,
            page_size: limit,
            total,
        })
    }

    fn publish_dataset(
        &self,
        user_id: &str,
        attempt_id: &str,
        dataset: &FactorDataset,
        directory: &Path,
        cancelled: &AtomicBool,
    ) -> Result<String, String> {
        dataset.validate().map_err(string)?;
        self.assert_running_attempt(attempt_id, user_id, "factor-materialization")?;
        let staging = directory.join(format!(".factor-{attempt_id}.parquet.tmp"));
        let final_path = directory.join(format!("{}.parquet", dataset.manifest.dataset_id));
        write_factor_parquet(&staging, &dataset.manifest.output_names, &dataset.rows)?;
        if cancelled.load(Ordering::Relaxed) {
            let _ = fs::remove_file(&staging);
            return Err("cancelled".into());
        }
        let parquet_bytes = fs::read(&staging).map_err(string)?;
        let parquet_hash = hash_bytes(&parquet_bytes);
        let byte_size =
            u64::try_from(parquet_bytes.len()).map_err(|_| "Factor Parquet file is too large")?;
        let mut created_final = false;
        if final_path.is_file() {
            if hash_bytes(&fs::read(&final_path).map_err(string)?) != parquet_hash {
                let _ = fs::remove_file(&staging);
                return Err("existing Factor Dataset Parquet hash mismatch".into());
            }
            fs::remove_file(&staging).map_err(string)?;
        } else {
            fs::rename(&staging, &final_path).map_err(string)?;
            created_final = true;
        }
        let manifest_json = String::from_utf8(canonical_json(&dataset.manifest).map_err(string)?)
            .map_err(string)?;
        let result = (|| -> Result<(), String> {
            let transaction = self.database.unchecked_transaction().map_err(string)?;
            transaction
                .execute(
                    "INSERT OR IGNORE INTO factor_dataset_content(
                        dataset_id, manifest_json, parquet_path, payload_sha256,
                        parquet_sha256, byte_size, created_at_ms
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        dataset.manifest.dataset_id,
                        manifest_json,
                        final_path.to_string_lossy(),
                        dataset.manifest.payload_sha256,
                        parquet_hash,
                        byte_size as i64,
                        now_ms()
                    ],
                )
                .map_err(string)?;
            let stored_hash: String = transaction
                .query_row(
                    "SELECT parquet_sha256 FROM factor_dataset_content WHERE dataset_id = ?1",
                    [&dataset.manifest.dataset_id],
                    |row| row.get(0),
                )
                .map_err(string)?;
            let stored_manifest: String = transaction
                .query_row(
                    "SELECT manifest_json FROM factor_dataset_content WHERE dataset_id = ?1",
                    [&dataset.manifest.dataset_id],
                    |row| row.get(0),
                )
                .map_err(string)?;
            if stored_hash != parquet_hash || stored_manifest != manifest_json {
                return Err("Factor Dataset content identity collision".into());
            }
            transaction
                .execute(
                    "INSERT OR IGNORE INTO factor_dataset_access(user_id, dataset_id) VALUES (?1, ?2)",
                    params![user_id, dataset.manifest.dataset_id],
                )
                .map_err(string)?;
            let changed = transaction
                .execute(
                    "UPDATE factor_research_attempts
                        SET status = 'completed', result_id = ?2,
                            completed_units = ?3, progress_total = ?3, updated_at_ms = ?4
                      WHERE attempt_id = ?1 AND user_id = ?5 AND status = 'running'
                        AND (diagnostic IS NULL OR diagnostic != ?6)",
                    params![
                        attempt_id,
                        dataset.manifest.dataset_id,
                        dataset.rows.len() as i64,
                        now_ms(),
                        user_id,
                        CANCELLATION_REQUESTED_DIAGNOSTIC
                    ],
                )
                .map_err(string)?;
            if changed != 1 {
                return Err("Factor Materialization Attempt cannot be published".into());
            }
            transaction.commit().map_err(string)
        })();
        if result.is_err() && created_final {
            let _ = fs::remove_file(&final_path);
        }
        result.map(|()| dataset.manifest.dataset_id.clone())
    }

    fn assert_running_attempt(
        &self,
        attempt_id: &str,
        user_id: &str,
        kind: &str,
    ) -> Result<(), String> {
        let exists: i64 = self
            .database
            .query_row(
                "SELECT COUNT(*) FROM factor_research_attempts
                  WHERE attempt_id = ?1 AND user_id = ?2 AND kind = ?3 AND status = 'running'
                    AND (diagnostic IS NULL OR diagnostic != ?4)",
                params![attempt_id, user_id, kind, CANCELLATION_REQUESTED_DIAGNOSTIC],
                |row| row.get(0),
            )
            .map_err(string)?;
        (exists == 1)
            .then_some(())
            .ok_or_else(|| "Factor research Attempt is not Running".into())
    }

    fn assert_dataset_access(&self, user_id: &str, dataset_id: &str) -> Result<(), String> {
        let visible: i64 = self
            .database
            .query_row(
                "SELECT COUNT(*) FROM factor_dataset_access
                  WHERE user_id = ?1 AND dataset_id = ?2",
                params![user_id, dataset_id],
                |row| row.get(0),
            )
            .map_err(string)?;
        (visible == 1)
            .then_some(())
            .ok_or_else(|| "Factor Dataset is not available to this User".into())
    }

    fn feature_dataset_binding_for_user(
        &self,
        user_id: &str,
        dataset_id: &str,
    ) -> Result<String, String> {
        let manifest_json: String = self
            .database
            .query_row(
                "SELECT c.manifest_json
                   FROM factor_dataset_access a
                   JOIN factor_dataset_content c USING(dataset_id)
                  WHERE a.user_id = ?1 AND a.dataset_id = ?2",
                params![user_id, dataset_id],
                |row| row.get(0),
            )
            .map_err(|_| "Factor Dataset was not found".to_owned())?;
        let manifest: FactorDatasetManifest =
            serde_json::from_str(&manifest_json).map_err(string)?;
        manifest.validate().map_err(string)?;
        Ok(manifest.feature_dataset_id)
    }

    fn feature_dataset_bindings_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<(String, String)>, String> {
        let mut statement = self
            .database
            .prepare(
                "SELECT c.dataset_id, c.manifest_json
                   FROM factor_dataset_access a
                   JOIN factor_dataset_content c USING(dataset_id)
                  WHERE a.user_id = ?1",
            )
            .map_err(string)?;
        statement
            .query_map([user_id], |row| {
                let dataset_id = row.get::<_, String>(0)?;
                let manifest_json = row.get::<_, String>(1)?;
                Ok((dataset_id, manifest_json))
            })
            .map_err(string)?
            .map(|row| {
                let (dataset_id, manifest_json) = row.map_err(string)?;
                let manifest: FactorDatasetManifest =
                    serde_json::from_str(&manifest_json).map_err(string)?;
                manifest.validate().map_err(string)?;
                Ok((dataset_id, manifest.feature_dataset_id))
            })
            .collect()
    }

    fn dataset_for_user(
        &self,
        user_id: &str,
        dataset_id: &str,
    ) -> Result<FactorDatasetView, String> {
        let (manifest_json, path, parquet_sha256, created_at): (String, String, String, i64) = self
            .database
            .query_row(
                "SELECT c.manifest_json, c.parquet_path, c.parquet_sha256, c.created_at_ms
                   FROM factor_dataset_access a JOIN factor_dataset_content c USING(dataset_id)
                  WHERE a.user_id = ?1 AND a.dataset_id = ?2",
                params![user_id, dataset_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .map_err(|_| "Factor Dataset was not found".to_owned())?;
        let manifest: FactorDatasetManifest =
            serde_json::from_str(&manifest_json).map_err(string)?;
        manifest.validate().map_err(string)?;
        if hash_bytes(&fs::read(&path).map_err(string)?) != parquet_sha256 {
            return Err("Factor Dataset Parquet content hash mismatch".into());
        }
        let byte_size = fs::metadata(&path)
            .map(|metadata| metadata.len())
            .map_err(string)?;
        Ok(FactorDatasetView {
            manifest,
            byte_size,
            locked_by: self.locked_by(user_id, "dataset", dataset_id)?,
            created_at_ms: created_at,
        })
    }

    fn factor_dataset_for_user(
        &self,
        user_id: &str,
        dataset_id: &str,
    ) -> Result<FactorDataset, String> {
        let (manifest_json, path, parquet_sha256): (String, String, String) = self
            .database
            .query_row(
                "SELECT c.manifest_json, c.parquet_path, c.parquet_sha256
                   FROM factor_dataset_access a JOIN factor_dataset_content c USING(dataset_id)
                  WHERE a.user_id = ?1 AND a.dataset_id = ?2",
                params![user_id, dataset_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|_| "Factor Dataset was not found".to_owned())?;
        let manifest: FactorDatasetManifest =
            serde_json::from_str(&manifest_json).map_err(string)?;
        manifest.validate().map_err(string)?;
        let bytes = fs::read(&path).map_err(string)?;
        if hash_bytes(&bytes) != parquet_sha256 {
            return Err("Factor Dataset Parquet content hash mismatch".into());
        }
        let (rows, total) =
            read_factor_rows(Path::new(&path), &manifest.output_names, 0, u32::MAX, None)?;
        if total != manifest.observation_count {
            return Err("Factor Dataset row count does not match its manifest".into());
        }
        let dataset = FactorDataset { manifest, rows };
        dataset.validate().map_err(string)?;
        Ok(dataset)
    }

    fn list_datasets(
        &self,
        request: &FactorPageRequest,
    ) -> Result<FactorPage<FactorDatasetView>, String> {
        let (page, limit, offset) = page_params(request)?;
        let total = self
            .database
            .query_row(
                "SELECT COUNT(*) FROM factor_dataset_access WHERE user_id = ?1",
                [&request.user_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(string)? as u64;
        let mut statement = self
            .database
            .prepare(
                "SELECT dataset_id FROM factor_dataset_access
                  WHERE user_id = ?1 ORDER BY dataset_id LIMIT ?2 OFFSET ?3",
            )
            .map_err(string)?;
        let ids = statement
            .query_map(
                params![request.user_id, limit as i64, offset as i64],
                |row| row.get::<_, String>(0),
            )
            .map_err(string)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(string)?;
        let items = ids
            .iter()
            .map(|id| self.dataset_for_user(&request.user_id, id))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(FactorPage {
            items,
            page,
            page_size: limit,
            total,
        })
    }

    fn dataset_rows(
        &self,
        user_id: &str,
        dataset_id: &str,
        offset: u64,
        limit: u32,
        instrument_id: Option<&str>,
    ) -> Result<FactorDatasetRowsPage, String> {
        if limit == 0 || limit > MAX_PAGE_SIZE {
            return Err("Factor Dataset row page size is invalid".into());
        }
        let (manifest_json, path, parquet_sha256): (String, String, String) = self
            .database
            .query_row(
                "SELECT c.manifest_json, c.parquet_path, c.parquet_sha256
                   FROM factor_dataset_access a JOIN factor_dataset_content c USING(dataset_id)
                  WHERE a.user_id = ?1 AND a.dataset_id = ?2",
                params![user_id, dataset_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|_| "Factor Dataset was not found".to_owned())?;
        let manifest: FactorDatasetManifest =
            serde_json::from_str(&manifest_json).map_err(string)?;
        manifest.validate().map_err(string)?;
        if hash_bytes(&fs::read(&path).map_err(string)?) != parquet_sha256 {
            return Err("Factor Dataset Parquet content hash mismatch".into());
        }
        let (rows, total) = read_factor_rows(
            Path::new(&path),
            &manifest.output_names,
            offset,
            limit,
            instrument_id,
        )?;
        if instrument_id.is_none() && total != manifest.observation_count {
            return Err("Factor Dataset row count does not match its manifest".into());
        }
        let next_offset = (offset.saturating_add(rows.len() as u64) < total)
            .then_some(offset.saturating_add(rows.len() as u64));
        Ok(FactorDatasetRowsPage {
            rows,
            offset,
            limit,
            next_offset,
            total,
        })
    }

    fn publish_report(
        &self,
        user_id: &str,
        attempt_id: &str,
        report: &FactorEvaluationReport,
        directory: &Path,
        cancelled: &AtomicBool,
    ) -> Result<String, String> {
        report.validate().map_err(string)?;
        self.assert_running_attempt(attempt_id, user_id, "factor-evaluation")?;
        self.assert_dataset_access(user_id, &report.factor_dataset_id)?;
        let protocol_json: String = self
            .database
            .query_row(
                "SELECT protocol_json FROM factor_research_protocols
                 WHERE protocol_hash = ?1 AND user_id = ?2 AND kind = 'evaluation'",
                params![report.protocol_hash, user_uuid_string(user_id)],
                |row| row.get(0),
            )
            .map_err(|_| "Evaluation Protocol was not found".to_owned())?;
        let protocol: FactorEvaluationProtocol =
            serde_json::from_str(&protocol_json).map_err(string)?;
        let manifest_json: String = self
            .database
            .query_row(
                "SELECT c.manifest_json FROM factor_dataset_access a
                 JOIN factor_dataset_content c USING(dataset_id)
                 WHERE a.user_id = ?1 AND a.dataset_id = ?2",
                params![user_id, report.factor_dataset_id],
                |row| row.get(0),
            )
            .map_err(|_| "Factor Dataset was not found".to_owned())?;
        let manifest: FactorDatasetManifest =
            serde_json::from_str(&manifest_json).map_err(string)?;
        manifest.validate().map_err(string)?;
        if protocol.factor_dataset_id != report.factor_dataset_id
            || protocol.output_name != report.output_name
            || protocol.scope != report.scope
            || protocol.target != report.target
            || protocol.market_data_snapshot_id != report.market_data_snapshot_id
            || protocol.point_in_time_universe_id != report.point_in_time_universe_id
            || protocol.market_context != report.market_context
        {
            return Err("Factor Report is not bound to its Evaluation Protocol".into());
        }
        let lineage = self.lineage_for_user(user_uuid(user_id), &protocol.trial_id.to_string())?;
        let (raw_statistic, p_value) = factor_trial_statistics(report)?;
        let report_dir = directory.join("reports");
        fs::create_dir_all(&report_dir).map_err(string)?;
        let staging = report_dir.join(format!(".factor-{attempt_id}.metrics.parquet.tmp"));
        let final_path = report_dir.join(format!("{}.metrics.parquet", report.report_hash));
        write_report_parquet(&staging, report)?;
        if cancelled.load(Ordering::Relaxed) {
            let _ = fs::remove_file(&staging);
            return Err("cancelled".into());
        }
        let bytes = fs::read(&staging).map_err(string)?;
        let (report_path, created_final) = if final_path.is_file() {
            if hash_bytes(&fs::read(&final_path).map_err(string)?) != hash_bytes(&bytes) {
                let _ = fs::remove_file(&staging);
                return Err("existing Factor Report Parquet hash mismatch".into());
            }
            fs::remove_file(&staging).map_err(string)?;
            (final_path.clone(), false)
        } else {
            fs::rename(&staging, &final_path).map_err(string)?;
            (final_path.clone(), true)
        };
        let report_json =
            String::from_utf8(canonical_json(report).map_err(string)?).map_err(string)?;
        let result = (|| -> Result<(), String> {
            let transaction = self.database.unchecked_transaction().map_err(string)?;
            transaction
                .execute(
                    "INSERT OR IGNORE INTO factor_evaluation_reports(
                        report_hash, user_id, report_json, parquet_path, created_at_ms
                     ) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        report.report_hash,
                        user_id,
                        report_json,
                        report_path.to_string_lossy(),
                        now_ms()
                    ],
                )
                .map_err(string)?;
            transaction
                .execute(
                    "INSERT OR IGNORE INTO factor_evaluation_report_access(user_id, report_hash) VALUES (?1, ?2)",
                    params![user_id, report.report_hash],
                )
                .map_err(string)?;
            let stored_report: String = transaction
                .query_row(
                    "SELECT report_json FROM factor_evaluation_reports WHERE report_hash = ?1",
                    [&report.report_hash],
                    |row| row.get(0),
                )
                .map_err(string)?;
            if stored_report != report_json {
                return Err("Factor Report content identity collision".into());
            }
            // Python grid trials reserve stable identities before runtime-specific protocols;
            // that legacy path records their completed state immediately after publication.
            if lineage
                .registrations
                .iter()
                .find(|registration| registration.trial_id == protocol.trial_id)
                .is_some_and(|registration| {
                    registration.evaluation_protocol_hash == protocol.protocol_hash
                })
            {
                complete_evaluation_trial(
                    &transaction,
                    user_id,
                    &protocol,
                    report,
                    &lineage,
                    &manifest.candidate_hash,
                    raw_statistic,
                    p_value,
                )?;
            }
            transaction
                .execute(
                    "INSERT OR IGNORE INTO factor_references(
                        evidence_kind, evidence_id, referencing_user_id, reference_id
                     ) VALUES ('dataset', ?1, ?2, ?3)",
                    params![
                        report.factor_dataset_id,
                        user_uuid_string(user_id),
                        report.report_hash
                    ],
                )
                .map_err(string)?;
            let changed = transaction
                .execute(
                    "UPDATE factor_research_attempts
                        SET status = 'completed', result_id = ?2,
                            completed_units = 1, progress_total = 1, updated_at_ms = ?3
                      WHERE attempt_id = ?1 AND user_id = ?4 AND status = 'running'
                        AND (diagnostic IS NULL OR diagnostic != ?5)",
                    params![
                        attempt_id,
                        report.report_hash,
                        now_ms(),
                        user_id,
                        CANCELLATION_REQUESTED_DIAGNOSTIC
                    ],
                )
                .map_err(string)?;
            if changed != 1 {
                return Err("Factor Evaluation Attempt cannot be published".into());
            }
            transaction.commit().map_err(string)
        })();
        if result.is_err() && created_final {
            let _ = fs::remove_file(&final_path);
        }
        result.map(|()| report.report_hash.clone())
    }

    fn report_for_user(
        &self,
        user_id: &str,
        report_hash: &str,
    ) -> Result<FactorReportView, String> {
        let (json, created_at): (String, i64) = self
            .database
            .query_row(
                "SELECT r.report_json, r.created_at_ms
                   FROM factor_evaluation_report_access a JOIN factor_evaluation_reports r USING(report_hash)
                  WHERE a.user_id = ?1 AND a.report_hash = ?2",
                params![user_id, report_hash],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|_| "Factor Evaluation Report was not found".to_owned())?;
        let report: FactorEvaluationReport = serde_json::from_str(&json).map_err(string)?;
        let protocol_json: Option<String> = self
            .database
            .query_row(
                "SELECT protocol_json FROM factor_research_protocols
                 WHERE protocol_hash = ?1 AND user_id = ?2 AND kind = 'evaluation'",
                params![&report.protocol_hash, user_uuid(user_id).to_string()],
                |row| row.get(0),
            )
            .optional()
            .map_err(string)?;
        Ok(FactorReportView {
            protocol: protocol_json
                .map(|json| serde_json::from_str(&json).map_err(string))
                .transpose()?,
            report,
            locked_by: self.locked_by(user_id, "report", report_hash)?,
            created_at_ms: created_at,
        })
    }

    fn list_reports(
        &self,
        request: &FactorPageRequest,
    ) -> Result<FactorPage<FactorReportView>, String> {
        let (page, limit, offset) = page_params(request)?;
        let total = self
            .database
            .query_row(
                "SELECT COUNT(*) FROM factor_evaluation_report_access WHERE user_id = ?1",
                [&request.user_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(string)? as u64;
        let mut statement = self
            .database
            .prepare(
                "SELECT report_hash FROM factor_evaluation_report_access
                  WHERE user_id = ?1 ORDER BY report_hash LIMIT ?2 OFFSET ?3",
            )
            .map_err(string)?;
        let ids = statement
            .query_map(
                params![request.user_id, limit as i64, offset as i64],
                |row| row.get::<_, String>(0),
            )
            .map_err(string)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(string)?;
        let items = ids
            .iter()
            .map(|id| self.report_for_user(&request.user_id, id))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(FactorPage {
            items,
            page,
            page_size: limit,
            total,
        })
    }

    fn save_family_for_attempt(
        &self,
        user_id: &str,
        attempt_id: &str,
        registration: &ResearchFamilyRegistration,
        trials: &[ResearchTrial],
    ) -> Result<FactorFamilyView, String> {
        let transaction = self.database.unchecked_transaction().map_err(string)?;
        let publishable: i64 = transaction
            .query_row(
                "SELECT COUNT(*) FROM factor_research_attempts
                  WHERE attempt_id = ?1 AND user_id = ?2 AND kind = 'factor-family-grid'
                    AND status = 'running'
                    AND (diagnostic IS NULL OR diagnostic != ?3)",
                params![attempt_id, user_id, CANCELLATION_REQUESTED_DIAGNOSTIC],
                |row| row.get(0),
            )
            .map_err(string)?;
        if publishable != 1 {
            return Err("cancelled".into());
        }
        save_family_records(&transaction, registration, trials)?;
        let changed = transaction
            .execute(
                "UPDATE factor_research_attempts
                    SET status = 'completed', result_id = ?2,
                        completed_units = 1, progress_total = 1, updated_at_ms = ?3
                  WHERE attempt_id = ?1 AND user_id = ?4 AND kind = 'factor-family-grid'
                    AND status = 'running'
                    AND (diagnostic IS NULL OR diagnostic != ?5)",
                params![
                    attempt_id,
                    registration.family.family_id.to_string(),
                    now_ms(),
                    user_id,
                    CANCELLATION_REQUESTED_DIAGNOSTIC
                ],
            )
            .map_err(string)?;
        if changed != 1 {
            return Err("cancelled".into());
        }
        transaction.commit().map_err(string)?;
        Ok(FactorFamilyView {
            family: registration.family.clone(),
            trial_count: registration.trials.len() as u64,
            lineage_hash: registration.family.lineage_hash.clone(),
        })
    }

    fn save_family(
        &self,
        registration: &ResearchFamilyRegistration,
        trials: &[ResearchTrial],
    ) -> Result<FactorFamilyView, String> {
        let transaction = self.database.unchecked_transaction().map_err(string)?;
        save_family_records(&transaction, registration, trials)?;
        transaction.commit().map_err(string)?;
        Ok(FactorFamilyView {
            family: registration.family.clone(),
            trial_count: registration.trials.len() as u64,
            lineage_hash: registration.family.lineage_hash.clone(),
        })
    }

    fn family_for_user(&self, user_id: Uuid, family_id: &str) -> Result<FactorFamilyView, String> {
        let family_uuid = Uuid::parse_str(family_id)
            .map_err(|_| "Research Family identity is invalid".to_owned())?;
        let (json, count): (String, i64) = self
            .database
            .query_row(
                "SELECT f.family_json, COUNT(r.trial_id)
                   FROM factor_research_families f
                   LEFT JOIN factor_research_registrations r USING(family_id)
                  WHERE f.family_id = ?1 AND f.user_id = ?2 GROUP BY f.family_id",
                params![family_uuid.to_string(), user_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|_| "Research Family was not found".to_owned())?;
        let family: ResearchFamily = serde_json::from_str(&json).map_err(string)?;
        Ok(FactorFamilyView {
            lineage_hash: family.lineage_hash.clone(),
            family,
            trial_count: count.max(0) as u64,
        })
    }

    fn family_registration(
        &self,
        user_id: Uuid,
        family_id: Uuid,
    ) -> Result<ResearchFamilyRegistration, String> {
        let family_json: String = self
            .database
            .query_row(
                "SELECT family_json FROM factor_research_families WHERE family_id = ?1 AND user_id = ?2",
                params![family_id.to_string(), user_id.to_string()],
                |row| row.get(0),
            )
            .map_err(|_| "Research Family was not found".to_owned())?;
        let family: ResearchFamily = serde_json::from_str(&family_json).map_err(string)?;
        family.validate().map_err(string)?;
        let mut statement = self
            .database
            .prepare(
                "SELECT registration_json FROM factor_research_registrations
                 WHERE family_id = ?1 AND user_id = ?2 ORDER BY trial_id",
            )
            .map_err(string)?;
        let trials = statement
            .query_map(params![family_id.to_string(), user_id.to_string()], |row| {
                row.get::<_, String>(0)
            })
            .map_err(string)?
            .map(|json| {
                let trial: ResearchTrialRegistration =
                    serde_json::from_str(&json.map_err(string)?).map_err(string)?;
                trial.validate().map_err(string)?;
                Ok(trial)
            })
            .collect::<Result<Vec<_>, String>>()?;
        let trial_ids = trials
            .iter()
            .map(|trial| trial.trial_id)
            .collect::<Vec<_>>();
        if family.user_id != user_id
            || trial_ids != family.registered_trial_ids
            || trials
                .iter()
                .any(|trial| trial.family_id != family.family_id)
        {
            return Err("Research Family registration is incomplete".into());
        }
        Ok(ResearchFamilyRegistration { family, trials })
    }

    fn list_families(
        &self,
        request: &FactorPageRequest,
    ) -> Result<FactorPage<FactorFamilyView>, String> {
        let (page, limit, offset) = page_params(request)?;
        let user = user_uuid(&request.user_id).to_string();
        let total = self
            .database
            .query_row(
                "SELECT COUNT(*) FROM factor_research_families WHERE user_id = ?1",
                [&user],
                |row| row.get::<_, i64>(0),
            )
            .map_err(string)? as u64;
        let mut statement = self.database.prepare("SELECT family_id FROM factor_research_families WHERE user_id = ?1 ORDER BY family_id LIMIT ?2 OFFSET ?3").map_err(string)?;
        let ids = statement
            .query_map(params![user, limit as i64, offset as i64], |row| {
                row.get::<_, String>(0)
            })
            .map_err(string)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(string)?;
        let items = ids
            .iter()
            .map(|id| self.family_for_user(user_uuid(&request.user_id), id))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(FactorPage {
            items,
            page,
            page_size: limit,
            total,
        })
    }

    fn save_trial(&self, user_id: Uuid, trial: &ResearchTrial) -> Result<(), String> {
        trial.validate().map_err(string)?;
        let family_user: String = self
            .database
            .query_row(
                "SELECT user_id FROM factor_research_families WHERE family_id = ?1",
                [trial.family_id.to_string()],
                |row| row.get(0),
            )
            .map_err(|_| "Research Family was not found".to_owned())?;
        if family_user != user_id.to_string() {
            return Err("Research Trial is owned by another User".into());
        }
        let mut trial = trial.clone();
        let existing: Option<String> = self
            .database
            .query_row(
                "SELECT trial_json FROM factor_research_trials
                 WHERE trial_id = ?1 AND user_id = ?2",
                params![trial.trial_id.to_string(), user_id.to_string()],
                |row| row.get(0),
            )
            .optional()
            .map_err(string)?;
        if let Some(existing) = existing {
            let existing: ResearchTrial = serde_json::from_str(&existing).map_err(string)?;
            existing.validate().map_err(string)?;
            if existing.status == ResearchTrialStatus::Completed
                && trial.status == ResearchTrialStatus::Completed
                && existing.report_hash == trial.report_hash
            {
                trial.raw_statistic = trial.raw_statistic.or(existing.raw_statistic);
                trial.p_value = trial.p_value.or(existing.p_value);
                trial.holm_adjusted = trial.holm_adjusted.or(existing.holm_adjusted);
                if trial.related_trial_ids.is_empty() {
                    trial.related_trial_ids = existing.related_trial_ids;
                }
                trial.diagnostic = trial.diagnostic.or(existing.diagnostic);
                trial.validate().map_err(string)?;
            }
        }
        self.database
            .execute(
                "INSERT INTO factor_research_trials(trial_id, family_id, user_id, trial_json, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(trial_id) DO UPDATE SET trial_json = excluded.trial_json, updated_at_ms = excluded.updated_at_ms",
                params![trial.trial_id.to_string(), trial.family_id.to_string(), user_id.to_string(), serde_json::to_string(&trial).map_err(string)?, now_ms()],
            )
            .map(|_| ())
            .map_err(string)
    }

    fn lineage_for_user(&self, user_id: Uuid, trial_id: &str) -> Result<FactorLineageView, String> {
        let trial_id = Uuid::parse_str(trial_id)
            .map_err(|_| "Research Trial identity is invalid".to_owned())?;
        let mut registry = ResearchRegistry::default();
        let mut families = self
            .database
            .prepare("SELECT family_json FROM factor_research_families WHERE user_id = ?1 ORDER BY family_id")
            .map_err(string)?
            .query_map([user_id.to_string()], |row| row.get::<_, String>(0))
            .map_err(string)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(string)?;
        for family_json in families.drain(..) {
            let family: ResearchFamily = serde_json::from_str(&family_json).map_err(string)?;
            let mut registrations = self
                .database
                .prepare("SELECT registration_json FROM factor_research_registrations WHERE user_id = ?1 AND family_id = ?2 ORDER BY trial_id")
                .map_err(string)?
                .query_map(params![user_id.to_string(), family.family_id.to_string()], |row| row.get::<_, String>(0))
                .map_err(string)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(string)?;
            let drafts = registrations
                .drain(..)
                .map(|json| {
                    let registration: ResearchTrialRegistration =
                        serde_json::from_str(&json).map_err(string)?;
                    Ok(adaq_factor_research::ResearchTrialDraft {
                        trial_id: registration.trial_id,
                        candidate_hash: registration.candidate_hash,
                        parameter_set_hash: registration.parameter_set_hash,
                        target: registration.target,
                        market_context: registration.market_context,
                        point_in_time_universe_id: registration.point_in_time_universe_id,
                        observation_range: registration.observation_range,
                        evaluation_protocol_hash: registration.evaluation_protocol_hash,
                        derivation_hash: registration.derivation_hash,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            let draft = adaq_factor_research::ResearchFamilyDraft {
                family_id: family.family_id,
                user_id,
                root_candidate_hash: family.root_candidate_hash.clone(),
                parent_family_id: family.parent_family_id,
                trials: drafts,
            };
            registry.register_family(draft).map_err(string)?;
        }
        let lineage = registry.lineage(user_id, trial_id).map_err(string)?;
        let registrations = lineage
            .trial_ids
            .iter()
            .map(|id| {
                self.database
                    .query_row(
                        "SELECT registration_json FROM factor_research_registrations
                         WHERE trial_id = ?1 AND user_id = ?2",
                        params![id.to_string(), user_id.to_string()],
                        |row| row.get::<_, String>(0),
                    )
                    .map_err(|_| "Research Trial registration was not found".to_owned())
                    .and_then(|json| serde_json::from_str(&json).map_err(string))
            })
            .collect::<Result<Vec<ResearchTrialRegistration>, String>>()?;
        let trials = registrations
            .iter()
            .map(|registration| {
                let stored: Option<String> = self
                    .database
                    .query_row(
                        "SELECT trial_json FROM factor_research_trials
                         WHERE trial_id = ?1 AND user_id = ?2",
                        params![registration.trial_id.to_string(), user_id.to_string()],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(string)?;
                let trial = stored
                    .map(|json| serde_json::from_str::<ResearchTrial>(&json).map_err(string))
                    .transpose()?
                    .unwrap_or_else(|| initial_trial(registration));
                trial.validate().map_err(string)?;
                Ok(trial)
            })
            .collect::<Result<Vec<ResearchTrial>, String>>()?;
        let protocols = registrations
            .iter()
            .filter_map(|registration| {
                let json: Option<String> = self
                    .database
                    .query_row(
                        "SELECT protocol_json FROM factor_research_protocols
                         WHERE protocol_hash = ?1 AND user_id = ?2 AND kind = 'evaluation'",
                        params![registration.evaluation_protocol_hash, user_id.to_string()],
                        |row| row.get(0),
                    )
                    .optional()
                    .ok()
                    .flatten();
                json.map(|json| serde_json::from_str(&json).map_err(string))
            })
            .collect::<Result<Vec<FactorEvaluationProtocol>, String>>()?;
        Ok(FactorLineageView {
            lineage,
            trials,
            registrations,
            protocols,
        })
    }

    fn save_policy(
        &self,
        user_id: Uuid,
        policy: &PromotionPolicy,
    ) -> Result<FactorPolicyView, String> {
        let json = String::from_utf8(canonical_json(policy).map_err(string)?).map_err(string)?;
        let stored: Option<(String, String)> = self
            .database
            .query_row(
                "SELECT user_id, policy_json FROM factor_promotion_policies WHERE policy_hash = ?1",
                [&policy.policy_hash],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(string)?;
        match stored {
            Some((owner, _)) if owner != user_id.to_string() => {
                return Err("Promotion Policy is owned by another User".into());
            }
            Some((_, existing)) if existing != json => {
                return Err("Promotion Policy content identity collision".into());
            }
            Some(_) => {}
            None => {
                self.database
                    .execute(
                        "INSERT INTO factor_promotion_policies(policy_hash, user_id, policy_json, created_at_ms)
                         VALUES (?1, ?2, ?3, ?4)",
                        params![policy.policy_hash, user_id.to_string(), json, now_ms()],
                    )
                    .map_err(string)?;
            }
        }
        Ok(FactorPolicyView {
            policy: policy.clone(),
            created_at_ms: now_ms(),
        })
    }

    fn freeze_promotion_protocol(
        &self,
        owner_id: &str,
        user_id: Uuid,
        request: FactorPromotionProtocolFreezeRequest,
    ) -> Result<PromotionProtocol, String> {
        let candidate = self.candidate_for_user(owner_id, &request.candidate_hash)?;
        candidate.candidate.validate().map_err(string)?;
        let lineage = self.lineage_for_user(user_id, &request.trial_id.to_string())?;
        let trial = lineage
            .trials
            .iter()
            .find(|trial| trial.trial_id == request.trial_id)
            .ok_or_else(|| "Research Trial was not found for this User".to_owned())?;
        if trial.family_id != request.family_id
            || trial.candidate_hash != request.candidate_hash
            || trial.status != ResearchTrialStatus::Completed
        {
            return Err("Research Trial is not a completed exact Candidate binding".into());
        }
        let trial_report_hash = trial
            .report_hash
            .as_deref()
            .ok_or_else(|| "Promotion requires a completed Trial Report".to_owned())?;
        if request.report_hashes.is_empty()
            || request
                .report_hashes
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            || request
                .report_hashes
                .iter()
                .any(|hash| !adaq_factor_research::is_sha256(hash))
            || !request
                .report_hashes
                .iter()
                .any(|hash| hash == trial_report_hash)
        {
            return Err(
                "Promotion Reports must be sorted, valid, and cite the selected Trial".into(),
            );
        }
        let known_reports = lineage
            .trials
            .iter()
            .filter_map(|trial| trial.report_hash.as_deref())
            .collect::<BTreeSet<_>>();
        if request
            .report_hashes
            .iter()
            .any(|hash| !known_reports.contains(hash.as_str()))
        {
            return Err("Promotion Reports must belong to the complete Trial lineage".into());
        }
        let reports = request
            .report_hashes
            .iter()
            .map(|hash| {
                let report = self.report_for_user(owner_id, hash)?.report;
                report.validate().map_err(string)?;
                Ok(report)
            })
            .collect::<Result<Vec<FactorEvaluationReport>, String>>()?;
        let report = reports
            .first()
            .ok_or_else(|| "Promotion requires at least one completed Report".to_owned())?;
        let evaluation_json: String = self
            .database
            .query_row(
                "SELECT protocol_json FROM factor_research_protocols
                  WHERE protocol_hash = ?1 AND user_id = ?2 AND kind = 'evaluation'",
                params![report.protocol_hash, user_id.to_string()],
                |row| row.get(0),
            )
            .map_err(|_| "Promotion Evaluation Protocol was not found for this User".to_owned())?;
        let evaluation_protocol: FactorEvaluationProtocol =
            serde_json::from_str(&evaluation_json).map_err(string)?;
        evaluation_protocol.validate().map_err(string)?;
        if evaluation_protocol.family_id != request.family_id
            || evaluation_protocol.trial_id != request.trial_id
            || evaluation_protocol.output_name != request.output_name
            || evaluation_protocol.factor_dataset_id != request.dataset_id
            || reports.iter().any(|report| {
                report.protocol_hash != evaluation_protocol.protocol_hash
                    || report.factor_dataset_id != request.dataset_id
                    || report.output_name != request.output_name
            })
        {
            return Err(
                "Promotion evidence does not match the selected Dataset, Trial, and output".into(),
            );
        }
        let dataset = self.dataset_for_user(owner_id, &request.dataset_id)?;
        let policy_json: String = self
            .database
            .query_row(
                "SELECT policy_json FROM factor_promotion_policies
                  WHERE policy_hash = ?1 AND user_id = ?2",
                params![request.policy_hash, user_id.to_string()],
                |row| row.get(0),
            )
            .map_err(|_| "Promotion Policy was not found for this User".to_owned())?;
        let policy: PromotionPolicy = serde_json::from_str(&policy_json).map_err(string)?;
        policy.validate().map_err(string)?;
        validate_evaluation_boundary(
            &candidate.candidate,
            candidate.predecessor.as_ref(),
            &dataset.manifest,
            &evaluation_protocol,
        )?;
        PromotionProtocol::freeze(
            PromotionProtocolDraft {
                protocol_id: Uuid::new_v4(),
                user_id,
                candidate_hash: request.candidate_hash,
                output_name: request.output_name,
                family_id: request.family_id,
                trial_id: request.trial_id,
                lineage_trial_ids: lineage.lineage.trial_ids,
                report_hashes: request.report_hashes,
                policy_hash: policy.policy_hash,
                engine_identity: evaluation_protocol.engine_identity,
            },
            lineage.lineage.lineage_hash,
        )
        .map_err(string)
    }

    fn record_decision(
        &self,
        owner_id: &str,
        user_id: Uuid,
        request: FactorDecisionRecordRequest,
    ) -> Result<FactorDecisionView, String> {
        let evidence_state =
            self.promotion_evidence_state(owner_id, &request.promotion_protocol)?;
        let protocol = request.promotion_protocol;
        let decision = FactorPromotionDecision::freeze(PromotionDecisionDraft {
            decision_id: Uuid::new_v4(),
            user_id,
            candidate_hash: protocol.candidate_hash.clone(),
            output_name: protocol.output_name.clone(),
            state: request.state,
            report_hashes: protocol.report_hashes.clone(),
            policy_hash: protocol.policy_hash.clone(),
            evidence_state,
            supersedes: request.supersedes,
        })
        .map_err(string)?;
        self.save_decision(
            owner_id,
            user_id,
            FactorDecisionSaveRequest {
                user_id: owner_id.to_owned(),
                decision,
                promotion_protocol: protocol,
                component: request.component,
            },
        )
    }

    fn promotion_evidence_state(
        &self,
        owner_id: &str,
        protocol: &PromotionProtocol,
    ) -> Result<EvaluationEvidenceState, String> {
        let mut state = EvaluationEvidenceState::OutOfSample;
        for hash in &protocol.report_hashes {
            let report = self.report_for_user(owner_id, hash)?.report;
            report.validate().map_err(string)?;
            state = match (state, report.evidence_state) {
                (EvaluationEvidenceState::Unknown, _) | (_, EvaluationEvidenceState::Unknown) => {
                    EvaluationEvidenceState::Unknown
                }
                (EvaluationEvidenceState::Overlapping, _)
                | (_, EvaluationEvidenceState::Overlapping) => EvaluationEvidenceState::Overlapping,
                _ => EvaluationEvidenceState::OutOfSample,
            };
        }
        Ok(state)
    }

    fn select_trial(
        &self,
        request: &FactorTrialSelectionRequest,
    ) -> Result<FactorSelectionView, String> {
        let user_id = user_uuid(&request.user_id);
        let _: String = self
            .database
            .query_row(
                "SELECT candidate_hash FROM factor_candidate_access
                 WHERE user_id = ?1 AND candidate_hash = ?2",
                params![request.user_id, request.candidate_hash],
                |row| row.get(0),
            )
            .map_err(|_| "Factor Candidate was not found for this User".to_owned())?;
        let policy_json: String = self
            .database
            .query_row(
                "SELECT policy_json FROM factor_promotion_policies
                 WHERE policy_hash = ?1 AND user_id = ?2",
                params![request.policy_hash, user_id.to_string()],
                |row| row.get(0),
            )
            .map_err(|_| "Promotion Policy was not found for this User".to_owned())?;
        let policy: PromotionPolicy = serde_json::from_str(&policy_json).map_err(string)?;
        policy.validate().map_err(string)?;
        let lineage = self.lineage_for_user(user_id, &request.trial_id.to_string())?;
        let trial = lineage
            .trials
            .iter()
            .find(|trial| trial.trial_id == request.trial_id)
            .ok_or_else(|| "Research Trial was not found".to_owned())?;
        if trial.family_id != request.family_id
            || trial.candidate_hash != request.candidate_hash
            || trial.status != adaq_factor_research::ResearchTrialStatus::Completed
        {
            return Err("Research Trial binding is invalid".into());
        }
        let report_hash = trial
            .report_hash
            .clone()
            .ok_or_else(|| "Parameter Selection requires a completed Report".to_owned())?;
        let report_json: String = self
            .database
            .query_row(
                "SELECT r.report_json FROM factor_evaluation_report_access a
                 JOIN factor_evaluation_reports r USING(report_hash)
                 WHERE a.user_id = ?1 AND a.report_hash = ?2",
                params![request.user_id, report_hash],
                |row| row.get(0),
            )
            .map_err(|_| "Factor Evaluation Report was not found".to_owned())?;
        let report: FactorEvaluationReport = serde_json::from_str(&report_json).map_err(string)?;
        let evaluation_json: String = self
            .database
            .query_row(
                "SELECT protocol_json FROM factor_research_protocols
                 WHERE protocol_hash = ?1 AND user_id = ?2 AND kind = 'evaluation'",
                params![report.protocol_hash, user_id.to_string()],
                |row| row.get(0),
            )
            .map_err(|_| "Evaluation Protocol was not found".to_owned())?;
        let evaluation: FactorEvaluationProtocol =
            serde_json::from_str(&evaluation_json).map_err(string)?;
        let protocol = PromotionProtocol::freeze(
            PromotionProtocolDraft {
                protocol_id: user_uuid(&format!(
                    "factor-promotion-protocol:{}:{}:{}",
                    request.candidate_hash, request.trial_id, request.policy_hash
                )),
                user_id,
                candidate_hash: request.candidate_hash.clone(),
                output_name: evaluation.output_name.clone(),
                family_id: request.family_id,
                trial_id: request.trial_id,
                lineage_trial_ids: lineage.lineage.trial_ids.clone(),
                report_hashes: vec![report_hash],
                policy_hash: policy.policy_hash,
                engine_identity: evaluation.engine_identity,
            },
            lineage.lineage.lineage_hash.clone(),
        )
        .map_err(string)?;
        let selection_content = serde_json::json!({
            "userId": user_id,
            "candidateHash": request.candidate_hash,
            "familyId": request.family_id,
            "trialId": request.trial_id,
            "promotionProtocolHash": protocol.protocol_hash.clone(),
        });
        let selection_hash = hash_bytes(&canonical_json(&selection_content).map_err(string)?);
        let existing: Option<(String, String, String, String)> = self
            .database
            .query_row(
                "SELECT selection_hash, family_id, trial_id, promotion_protocol_hash
                 FROM factor_parameter_selections
                 WHERE user_id = ?1 AND candidate_hash = ?2",
                params![request.user_id, request.candidate_hash],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()
            .map_err(string)?;
        if let Some((existing_hash, family_id, trial_id, protocol_hash)) = existing {
            if family_id == request.family_id.to_string()
                && trial_id == request.trial_id.to_string()
                && protocol_hash == protocol.protocol_hash
            {
                return Ok(FactorSelectionView {
                    candidate_hash: request.candidate_hash.clone(),
                    family_id: request.family_id,
                    selected_trial_id: request.trial_id,
                    selection_hash: existing_hash,
                    promotion_protocol_hash: protocol.protocol_hash,
                });
            }
            return Err("a different Parameter Selection is already recorded".into());
        }
        let protocol_json =
            String::from_utf8(canonical_json(&protocol).map_err(string)?).map_err(string)?;
        let transaction = self.database.unchecked_transaction().map_err(string)?;
        transaction
            .execute(
                "INSERT OR IGNORE INTO factor_research_protocols(
                    protocol_hash, user_id, kind, protocol_json, created_at_ms
                 ) VALUES (?1, ?2, 'promotion', ?3, ?4)",
                params![
                    protocol.protocol_hash,
                    user_id.to_string(),
                    protocol_json,
                    now_ms()
                ],
            )
            .map_err(string)?;
        transaction
            .execute(
                "INSERT INTO factor_parameter_selections(
                    selection_hash, user_id, candidate_hash, family_id, trial_id,
                    promotion_protocol_hash, created_at_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    selection_hash,
                    request.user_id,
                    request.candidate_hash,
                    request.family_id.to_string(),
                    request.trial_id.to_string(),
                    protocol.protocol_hash,
                    now_ms()
                ],
            )
            .map_err(string)?;
        transaction.commit().map_err(string)?;
        Ok(FactorSelectionView {
            candidate_hash: request.candidate_hash.clone(),
            family_id: request.family_id,
            selected_trial_id: request.trial_id,
            selection_hash,
            promotion_protocol_hash: protocol.protocol_hash.clone(),
        })
    }

    fn selected_trial(
        &self,
        user_id: &str,
        candidate_hash: &str,
    ) -> Result<(FactorSelectionView, PromotionProtocol), String> {
        let (selection_hash, family_id, trial_id, protocol_hash): (String, String, String, String) =
            self.database
                .query_row(
                    "SELECT selection_hash, family_id, trial_id, promotion_protocol_hash
                     FROM factor_parameter_selections
                     WHERE user_id = ?1 AND candidate_hash = ?2",
                    params![user_id, candidate_hash],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .map_err(|_| "Parameter Selection was not found".to_owned())?;
        let protocol_json: String = self
            .database
            .query_row(
                "SELECT protocol_json FROM factor_research_protocols
                 WHERE protocol_hash = ?1 AND user_id = ?2 AND kind = 'promotion'",
                params![protocol_hash, user_uuid(user_id).to_string()],
                |row| row.get(0),
            )
            .map_err(|_| "Promotion Protocol was not found".to_owned())?;
        let protocol: PromotionProtocol = serde_json::from_str(&protocol_json).map_err(string)?;
        Ok((
            FactorSelectionView {
                candidate_hash: candidate_hash.into(),
                family_id: Uuid::parse_str(&family_id)
                    .map_err(|_| "Parameter Selection family is invalid".to_owned())?,
                selected_trial_id: Uuid::parse_str(&trial_id)
                    .map_err(|_| "Parameter Selection trial is invalid".to_owned())?,
                selection_hash,
                promotion_protocol_hash: protocol_hash,
            },
            protocol,
        ))
    }

    fn model_input_binding(
        &self,
        user_id: &str,
        decision_hash: &str,
    ) -> Result<FactorModelInputBinding, String> {
        let record_json: String = self
            .database
            .query_row(
                "SELECT record_json FROM factor_promotion_decisions
                 WHERE user_id = ?1 AND decision_hash = ?2",
                params![user_uuid(user_id).to_string(), decision_hash],
                |row| row.get(0),
            )
            .map_err(|_| "Promotion Decision was not found".to_owned())?;
        let record: adaq_factor_research::PromotionDecisionRecord =
            serde_json::from_str(&record_json).map_err(string)?;
        record.validate().map_err(string)?;
        if !matches!(
            record.decision.state,
            adaq_factor_research::PromotionDecisionState::ResearchValidated
                | adaq_factor_research::PromotionDecisionState::ComponentEligible
        ) {
            return Err("Model input requires a positive Promotion Decision".into());
        }
        let protocol_json: String = self
            .database
            .query_row(
                "SELECT protocol_json FROM factor_research_protocols
                 WHERE protocol_hash = ?1 AND user_id = ?2 AND kind = 'promotion'",
                params![
                    record.promotion_protocol_hash,
                    user_uuid(user_id).to_string()
                ],
                |row| row.get(0),
            )
            .map_err(|_| "Promotion Protocol was not found".to_owned())?;
        let promotion_protocol: PromotionProtocol =
            serde_json::from_str(&protocol_json).map_err(string)?;
        promotion_protocol.validate().map_err(string)?;
        let report_hash = promotion_protocol
            .report_hashes
            .first()
            .ok_or_else(|| "Promotion Report was not found".to_owned())?;
        let report_json: String = self
            .database
            .query_row(
                "SELECT r.report_json FROM factor_evaluation_report_access a
                 JOIN factor_evaluation_reports r USING(report_hash)
                 WHERE a.user_id = ?1 AND a.report_hash = ?2",
                params![user_id, report_hash],
                |row| row.get(0),
            )
            .map_err(|_| "Promotion Report was not found".to_owned())?;
        let report: FactorEvaluationReport = serde_json::from_str(&report_json).map_err(string)?;
        let evaluation_json: String = self
            .database
            .query_row(
                "SELECT protocol_json FROM factor_research_protocols
                 WHERE protocol_hash = ?1 AND user_id = ?2 AND kind = 'evaluation'",
                params![report.protocol_hash, user_uuid(user_id).to_string()],
                |row| row.get(0),
            )
            .map_err(|_| "Evaluation Protocol was not found".to_owned())?;
        let evaluation: FactorEvaluationProtocol =
            serde_json::from_str(&evaluation_json).map_err(string)?;
        let dataset_json: String = self
            .database
            .query_row(
                "SELECT c.manifest_json FROM factor_dataset_access a
                 JOIN factor_dataset_content c USING(dataset_id)
                 WHERE a.user_id = ?1 AND a.dataset_id = ?2",
                params![user_id, evaluation.factor_dataset_id],
                |row| row.get(0),
            )
            .map_err(|_| "Promotion Factor Dataset was not found".to_owned())?;
        let dataset: FactorDatasetManifest = serde_json::from_str(&dataset_json).map_err(string)?;
        dataset.validate().map_err(string)?;
        let lookback = promotion_protocol
            .engine_identity
            .parameters
            .get("lookback")
            .ok_or_else(|| "Promotion Factor lookback is not bound".to_owned())?
            .parse::<u32>()
            .map_err(|_| "Promotion Factor lookback is invalid".to_owned())?;
        Ok(FactorModelInputBinding {
            decision_hash: record.decision.decision_hash,
            promotion_protocol,
            factor_dataset_id: evaluation.factor_dataset_id,
            feature_dataset_id: evaluation.feature_dataset_id,
            feature_plan_hash: evaluation.feature_plan_hash,
            snapshot_id: evaluation.market_data_snapshot_id,
            universe_id: evaluation.point_in_time_universe_id,
            lookback,
        })
    }

    fn list_policies(
        &self,
        request: &FactorPageRequest,
    ) -> Result<FactorPage<FactorPolicyView>, String> {
        let (page, limit, offset) = page_params(request)?;
        let user = user_uuid(&request.user_id).to_string();
        let total = self
            .database
            .query_row(
                "SELECT COUNT(*) FROM factor_promotion_policies WHERE user_id = ?1",
                [&user],
                |row| row.get::<_, i64>(0),
            )
            .map_err(string)? as u64;
        let mut statement = self.database.prepare("SELECT policy_json, created_at_ms FROM factor_promotion_policies WHERE user_id = ?1 ORDER BY created_at_ms DESC, policy_hash LIMIT ?2 OFFSET ?3").map_err(string)?;
        let items = statement
            .query_map(params![user, limit as i64, offset as i64], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(string)?
            .map(|row| {
                let (json, created_at_ms) = row.map_err(string)?;
                Ok(FactorPolicyView {
                    policy: serde_json::from_str(&json).map_err(string)?,
                    created_at_ms,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(FactorPage {
            items,
            page,
            page_size: limit,
            total,
        })
    }

    fn save_decision(
        &self,
        owner_id: &str,
        user_id: Uuid,
        request: FactorDecisionSaveRequest,
    ) -> Result<FactorDecisionView, String> {
        let decision = request.decision;
        let protocol = request.promotion_protocol;
        decision.validate().map_err(string)?;
        protocol.validate().map_err(string)?;
        if decision.user_id != user_id
            || protocol.user_id != user_id
            || decision.candidate_hash != protocol.candidate_hash
            || decision.output_name != protocol.output_name
            || decision.report_hashes != protocol.report_hashes
            || decision.policy_hash != protocol.policy_hash
        {
            return Err("Promotion Decision and Protocol identities differ".into());
        }
        let candidate_view = self.candidate_for_user(owner_id, &decision.candidate_hash)?;
        let candidate = candidate_view.candidate;
        if let FactorCandidateSource::Python { binding } = &candidate.source {
            if !matches!(
                &decision.state,
                adaq_factor_research::PromotionDecisionState::Rejected
            ) && !binding.repeatability_verified
            {
                return Err("Python Factor repeatability is not verified".into());
            }
            if matches!(
                &decision.state,
                adaq_factor_research::PromotionDecisionState::ComponentEligible
            ) && !matches!(
                binding.mode,
                adaq_factor_research::PythonFactorMode::PortableDefinition
            ) {
                return Err(
                    "Imperative Python Factors require an accepted Portable Definition for Component eligibility".into(),
                );
            }
        }
        let reports = protocol
            .report_hashes
            .iter()
            .map(|hash| {
                let json: String = self
                    .database
                    .query_row(
                        "SELECT r.report_json FROM factor_evaluation_report_access a
                         JOIN factor_evaluation_reports r USING(report_hash)
                         WHERE a.user_id = ?1 AND a.report_hash = ?2",
                        params![owner_id, hash],
                        |row| row.get(0),
                    )
                    .map_err(|_| "Promotion Report was not found for this User".to_owned())?;
                serde_json::from_str::<FactorEvaluationReport>(&json).map_err(string)
            })
            .collect::<Result<Vec<_>, String>>()?;
        let evaluation_protocol_hash =
            reports
                .first()
                .map(|report| report.protocol_hash.clone())
                .ok_or_else(|| "Promotion requires at least one completed Report".to_owned())?;
        let evaluation_json: String = self
            .database
            .query_row(
                "SELECT protocol_json FROM factor_research_protocols
                  WHERE protocol_hash = ?1 AND user_id = ?2 AND kind = 'evaluation'",
                params![evaluation_protocol_hash, user_id.to_string()],
                |row| row.get(0),
            )
            .map_err(|_| "Promotion Evaluation Protocol was not found".to_owned())?;
        let evaluation_protocol: FactorEvaluationProtocol =
            serde_json::from_str(&evaluation_json).map_err(string)?;
        if reports.iter().any(|report| {
            report.protocol_hash != evaluation_protocol.protocol_hash
                || report.factor_dataset_id != evaluation_protocol.factor_dataset_id
                || report.output_name != protocol.output_name
        }) {
            return Err("Promotion Reports do not share the Evaluation Protocol output".into());
        }
        let dataset_json: String = self
            .database
            .query_row(
                "SELECT c.manifest_json FROM factor_dataset_access a
                 JOIN factor_dataset_content c USING(dataset_id)
                  WHERE a.user_id = ?1 AND a.dataset_id = ?2",
                params![owner_id, evaluation_protocol.factor_dataset_id],
                |row| row.get(0),
            )
            .map_err(|_| "Promotion Factor Dataset was not found for this User".to_owned())?;
        let dataset: FactorDatasetManifest = serde_json::from_str(&dataset_json).map_err(string)?;
        dataset.validate().map_err(string)?;
        validate_evaluation_boundary(
            &candidate,
            candidate_view.predecessor.as_ref(),
            &dataset,
            &evaluation_protocol,
        )?;
        let policy_json: String = self.database.query_row("SELECT policy_json FROM factor_promotion_policies WHERE policy_hash = ?1 AND user_id = ?2", params![protocol.policy_hash, user_id.to_string()], |row| row.get(0)).map_err(|_| "Promotion Policy was not found for this User".to_owned())?;
        let policy: PromotionPolicy = serde_json::from_str(&policy_json).map_err(string)?;
        let lineage_view = self.lineage_for_user(user_id, &protocol.trial_id.to_string())?;
        let trial = lineage_view
            .trials
            .iter()
            .find(|trial| trial.trial_id == protocol.trial_id)
            .ok_or_else(|| "Promotion Research Trial was not found".to_owned())?;
        if trial.status != ResearchTrialStatus::Completed
            || trial
                .report_hash
                .as_deref()
                .is_none_or(|hash| !protocol.report_hashes.iter().any(|item| item == hash))
        {
            return Err("Promotion requires the selected completed Trial Report".into());
        }
        let eligibility = PromotionEligibility::check_with_trial(
            adaq_factor_research::PromotionEvidence {
                candidate: &candidate,
                dataset: &dataset,
                dataset_status: adaq_factor_research::FactorDatasetStatus::Completed,
                evaluation_protocol: &evaluation_protocol,
                reports: &reports,
                policy: &policy,
                lineage: &lineage_view.lineage,
                promotion_protocol: &protocol,
                component: request.component,
            },
            Some(trial),
        )
        .map_err(string)?;
        if matches!(
            decision.state,
            adaq_factor_research::PromotionDecisionState::ResearchValidated
        ) && !eligibility.research_validated()
            || matches!(
                decision.state,
                adaq_factor_research::PromotionDecisionState::ComponentEligible
            ) && !eligibility.component_eligible()
        {
            return Err("Promotion Decision exceeds the computed eligibility".into());
        }
        let current =
            self.current_decision(user_id, &decision.candidate_hash, &decision.output_name)?;
        match (current, decision.supersedes) {
            (Some(current), Some(supersedes)) if current.decision.decision_id == supersedes => {}
            (Some(_), _) => {
                return Err(
                    "a later Promotion Decision must supersede the current Decision".into(),
                );
            }
            (None, Some(_)) => return Err("the superseded Promotion Decision was not found".into()),
            (None, None) => {}
        }
        let record = adaq_factor_research::PromotionDecisionRecord {
            decision: decision.clone(),
            promotion_protocol_hash: protocol.protocol_hash.clone(),
            eligibility_gates: eligibility.gates().to_vec(),
            component: request.component,
        };
        let json = serde_json::to_string(&record).map_err(string)?;
        let promotion_json =
            String::from_utf8(canonical_json(&protocol).map_err(string)?).map_err(string)?;
        let transaction = self.database.unchecked_transaction().map_err(string)?;
        transaction
            .execute(
                "INSERT OR IGNORE INTO factor_research_protocols(
                    protocol_hash, user_id, kind, protocol_json, created_at_ms
                 ) VALUES (?1, ?2, 'promotion', ?3, ?4)",
                params![
                    protocol.protocol_hash,
                    user_id.to_string(),
                    promotion_json,
                    now_ms()
                ],
            )
            .map_err(string)?;
        let stored_promotion: String = transaction
            .query_row(
                "SELECT protocol_json FROM factor_research_protocols WHERE protocol_hash = ?1",
                [&protocol.protocol_hash],
                |row| row.get(0),
            )
            .map_err(string)?;
        if stored_promotion != promotion_json {
            return Err("Promotion Protocol content identity collision".into());
        }
        transaction.execute("INSERT INTO factor_promotion_decisions(decision_id, user_id, decision_hash, record_json, promotion_protocol_hash, created_at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6)", params![decision.decision_id.to_string(), user_id.to_string(), decision.decision_hash, json, protocol.protocol_hash, now_ms()]).map_err(string)?;
        for (kind, id) in [
            ("candidate", decision.candidate_hash.clone()),
            ("policy", decision.policy_hash.clone()),
            ("dataset", evaluation_protocol.factor_dataset_id.clone()),
        ] {
            transaction.execute("INSERT OR IGNORE INTO factor_references(evidence_kind, evidence_id, referencing_user_id, reference_id) VALUES (?1, ?2, ?3, ?4)", params![kind, id, user_id.to_string(), decision.decision_id.to_string()]).map_err(string)?;
            transaction.execute("INSERT OR IGNORE INTO factor_references(evidence_kind, evidence_id, referencing_user_id, reference_id) VALUES (?1, ?2, ?3, ?4)", params![kind, id, user_id.to_string(), protocol.protocol_hash]).map_err(string)?;
        }
        for report_hash in &decision.report_hashes {
            transaction.execute("INSERT OR IGNORE INTO factor_references(evidence_kind, evidence_id, referencing_user_id, reference_id) VALUES ('report', ?1, ?2, ?3)", params![report_hash, user_id.to_string(), decision.decision_id.to_string()]).map_err(string)?;
            transaction.execute("INSERT OR IGNORE INTO factor_references(evidence_kind, evidence_id, referencing_user_id, reference_id) VALUES ('report', ?1, ?2, ?3)", params![report_hash, user_id.to_string(), protocol.protocol_hash]).map_err(string)?;
        }
        transaction.commit().map_err(string)?;
        Ok(FactorDecisionView {
            decision,
            promotion_protocol_hash: protocol.protocol_hash,
            eligibility_gates: record.eligibility_gates,
            created_at_ms: now_ms(),
        })
    }

    fn current_decision(
        &self,
        user_id: Uuid,
        candidate_hash: &str,
        output_name: &str,
    ) -> Result<Option<adaq_factor_research::PromotionDecisionRecord>, String> {
        let mut statement = self
            .database
            .prepare(
                "SELECT record_json
                   FROM factor_promotion_decisions
                  WHERE user_id = ?1
                    AND json_extract(record_json, '$.decision.candidateHash') = ?2
                    AND json_extract(record_json, '$.decision.outputName') = ?3
                  ORDER BY created_at_ms DESC, decision_id DESC",
            )
            .map_err(string)?;
        let records = statement
            .query_map(
                params![user_id.to_string(), candidate_hash, output_name],
                |row| row.get::<_, String>(0),
            )
            .map_err(string)?
            .map(|row| {
                let json = row.map_err(string)?;
                let record: adaq_factor_research::PromotionDecisionRecord =
                    serde_json::from_str(&json).map_err(string)?;
                record.validate().map_err(string)?;
                Ok(record)
            })
            .collect::<Result<Vec<_>, String>>()?;
        let superseded = records
            .iter()
            .filter_map(|record| record.decision.supersedes)
            .collect::<BTreeSet<_>>();
        Ok(records
            .into_iter()
            .find(|record| !superseded.contains(&record.decision.decision_id)))
    }

    fn list_decisions(
        &self,
        request: &FactorPageRequest,
    ) -> Result<FactorPage<FactorDecisionView>, String> {
        let (page, limit, offset) = page_params(request)?;
        let user = user_uuid(&request.user_id).to_string();
        let total = self
            .database
            .query_row(
                "SELECT COUNT(*) FROM factor_promotion_decisions WHERE user_id = ?1",
                [&user],
                |row| row.get::<_, i64>(0),
            )
            .map_err(string)? as u64;
        let mut statement = self.database.prepare("SELECT record_json, created_at_ms FROM factor_promotion_decisions WHERE user_id = ?1 ORDER BY created_at_ms DESC, decision_id LIMIT ?2 OFFSET ?3").map_err(string)?;
        let items = statement
            .query_map(params![user, limit as i64, offset as i64], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(string)?
            .map(|row| {
                let (json, created_at_ms) = row.map_err(string)?;
                let record: adaq_factor_research::PromotionDecisionRecord =
                    serde_json::from_str(&json).map_err(string)?;
                record.validate().map_err(string)?;
                Ok(FactorDecisionView {
                    decision: record.decision,
                    promotion_protocol_hash: record.promotion_protocol_hash,
                    eligibility_gates: record.eligibility_gates,
                    created_at_ms,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(FactorPage {
            items,
            page,
            page_size: limit,
            total,
        })
    }

    fn list_decision_library(
        &self,
        request: &FactorPageRequest,
    ) -> Result<FactorPage<FactorDecisionView>, String> {
        let (page, limit, offset) = page_params(request)?;
        let user = user_uuid(&request.user_id).to_string();
        let mut statement = self
            .database
            .prepare(
                "SELECT record_json, created_at_ms
                   FROM factor_promotion_decisions
                  WHERE user_id = ?1
                  ORDER BY created_at_ms DESC, decision_id",
            )
            .map_err(string)?;
        let records = statement
            .query_map([user], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(string)?
            .map(|row| {
                let (json, created_at_ms) = row.map_err(string)?;
                let record: adaq_factor_research::PromotionDecisionRecord =
                    serde_json::from_str(&json).map_err(string)?;
                record.validate().map_err(string)?;
                Ok((record, created_at_ms))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let superseded = records
            .iter()
            .filter_map(|(record, _)| record.decision.supersedes)
            .collect::<Vec<_>>();
        let items = records
            .into_iter()
            .filter(|(record, _)| {
                matches!(
                    record.decision.state,
                    adaq_factor_research::PromotionDecisionState::ResearchValidated
                        | adaq_factor_research::PromotionDecisionState::ComponentEligible
                ) && !superseded.contains(&record.decision.decision_id)
            })
            .map(|(record, created_at_ms)| FactorDecisionView {
                decision: record.decision,
                promotion_protocol_hash: record.promotion_protocol_hash,
                eligibility_gates: record.eligibility_gates,
                created_at_ms,
            })
            .collect::<Vec<_>>();
        let total = items.len() as u64;
        let start = usize::try_from(offset)
            .unwrap_or(usize::MAX)
            .min(items.len());
        let paged = items.into_iter().skip(start).take(limit as usize).collect();
        Ok(FactorPage {
            items: paged,
            page,
            page_size: limit,
            total,
        })
    }

    fn add_reference(&self, request: &FactorReferenceRequest) -> Result<(), String> {
        validate_reference_kind(&request.evidence_kind)?;
        if request.evidence_id.trim().is_empty() || request.reference_id.trim().is_empty() {
            return Err("Factor evidence reference identity is invalid".into());
        }
        self.assert_visible(
            &request.user_id,
            &request.evidence_kind,
            &request.evidence_id,
        )?;
        self.database
            .execute(
                "INSERT OR IGNORE INTO factor_references(evidence_kind, evidence_id, referencing_user_id, reference_id)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    request.evidence_kind,
                    request.evidence_id,
                    user_uuid_string(&request.user_id),
                    request.reference_id
                ],
            )
            .map(|_| ())
            .map_err(string)
    }

    fn remove_reference(&self, request: &FactorReferenceRequest) -> Result<(), String> {
        validate_reference_kind(&request.evidence_kind)?;
        let changed = self
            .database
            .execute(
                "DELETE FROM factor_references
                  WHERE evidence_kind = ?1 AND evidence_id = ?2
                    AND referencing_user_id = ?3 AND reference_id = ?4",
                params![
                    request.evidence_kind,
                    request.evidence_id,
                    user_uuid_string(&request.user_id),
                    request.reference_id
                ],
            )
            .map_err(string)?;
        (changed == 1)
            .then_some(())
            .ok_or_else(|| "Factor evidence reference was not found".into())
    }

    fn delete_dataset(&self, user_id: &str, dataset_id: &str) -> Result<(), String> {
        let lock_count: i64 = self
            .database
            .query_row(
                "SELECT COUNT(*) FROM factor_references WHERE evidence_kind = 'dataset' AND evidence_id = ?1",
                [dataset_id],
                |row| row.get(0),
            )
            .map_err(string)?;
        if lock_count != 0 {
            return Err("Factor Dataset is locked by immutable evidence".into());
        }
        let (path, expected_hash): (String, String) = self
            .database
            .query_row(
                "SELECT c.parquet_path, c.parquet_sha256 FROM factor_dataset_access a JOIN factor_dataset_content c USING(dataset_id)
                  WHERE a.user_id = ?1 AND a.dataset_id = ?2",
                params![user_id, dataset_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|_| "Factor Dataset was not found".to_owned())?;
        let transaction = self.database.unchecked_transaction().map_err(string)?;
        transaction
            .execute(
                "DELETE FROM factor_dataset_access WHERE user_id = ?1 AND dataset_id = ?2",
                params![user_id, dataset_id],
            )
            .map_err(string)?;
        let remaining: i64 = transaction
            .query_row(
                "SELECT COUNT(*) FROM factor_dataset_access WHERE dataset_id = ?1",
                [dataset_id],
                |row| row.get(0),
            )
            .map_err(string)?;
        let remove_content = remaining == 0;
        if remove_content {
            transaction
                .execute(
                    "DELETE FROM factor_dataset_content WHERE dataset_id = ?1",
                    [dataset_id],
                )
                .map_err(string)?;
        }
        transaction.commit().map_err(string)?;
        if remove_content {
            let expected_path = Path::new(&path);
            if hash_bytes(&fs::read(expected_path).map_err(string)?) != expected_hash {
                return Err("stored Factor Dataset Parquet hash mismatch".into());
            }
            fs::remove_file(expected_path).map_err(string)?;
        }
        Ok(())
    }

    fn m12_eligibility(
        &self,
        owner_id: &str,
        user_id: Uuid,
        protocol: &PromotionProtocol,
    ) -> Result<adaq_factor_research::M12Eligibility, String> {
        protocol.validate().map_err(string)?;
        let record = self
            .current_decision(user_id, &protocol.candidate_hash, &protocol.output_name)?
            .ok_or_else(|| "no current Promotion Decision exists for this output".to_owned())?;
        if record.decision.user_id != user_id
            || record.decision.candidate_hash != protocol.candidate_hash
            || record.decision.output_name != protocol.output_name
            || record.promotion_protocol_hash != protocol.protocol_hash
            || record.decision.report_hashes != protocol.report_hashes
            || record.decision.policy_hash != protocol.policy_hash
        {
            return Err("Promotion Protocol is stale for the current Decision".into());
        }
        let report_hash = protocol
            .report_hashes
            .first()
            .ok_or_else(|| "Promotion Protocol has no Report".to_owned())?;
        let report_json: String = self.database.query_row("SELECT r.report_json FROM factor_evaluation_report_access a JOIN factor_evaluation_reports r USING(report_hash) WHERE a.user_id = ?1 AND a.report_hash = ?2", params![owner_id, report_hash], |row| row.get(0)).map_err(|_| "Promotion Report was not found".to_owned())?;
        let report: FactorEvaluationReport = serde_json::from_str(&report_json).map_err(string)?;
        let evaluation_json: String = self
            .database
            .query_row(
                "SELECT protocol_json FROM factor_research_protocols
                  WHERE protocol_hash = ?1 AND user_id = ?2 AND kind = 'evaluation'",
                params![report.protocol_hash, user_id.to_string()],
                |row| row.get(0),
            )
            .map_err(|_| "Evaluation Protocol was not found".to_owned())?;
        let evaluation: FactorEvaluationProtocol =
            serde_json::from_str(&evaluation_json).map_err(string)?;
        let manifest_json: String = self
            .database
            .query_row(
                "SELECT c.manifest_json
                   FROM factor_dataset_access a
                   JOIN factor_dataset_content c USING(dataset_id)
                  WHERE a.user_id = ?1 AND a.dataset_id = ?2",
                params![owner_id, evaluation.factor_dataset_id],
                |row| row.get(0),
            )
            .map_err(|_| "Factor Dataset was not found".to_owned())?;
        let manifest: FactorDatasetManifest =
            serde_json::from_str(&manifest_json).map_err(string)?;
        manifest.validate().map_err(string)?;
        let policy_json: String = self.database.query_row("SELECT policy_json FROM factor_promotion_policies WHERE policy_hash = ?1 AND user_id = ?2", params![protocol.policy_hash, user_id.to_string()], |row| row.get(0)).map_err(|_| "Promotion Policy was not found".to_owned())?;
        let policy: PromotionPolicy = serde_json::from_str(&policy_json).map_err(string)?;
        let candidate_json: String = self
            .database
            .query_row(
                "SELECT c.candidate_json FROM factor_candidate_access a
                 JOIN factor_candidate_content c USING(candidate_hash)
                  WHERE a.user_id = ?1 AND a.candidate_hash = ?2",
                params![owner_id, protocol.candidate_hash],
                |row| row.get(0),
            )
            .map_err(|_| "Promotion Candidate was not found".to_owned())?;
        let candidate = FactorCandidate::load(candidate_json.as_bytes()).map_err(string)?;
        let reports = protocol
            .report_hashes
            .iter()
            .map(|hash| {
                let json: String = self
                    .database
                    .query_row(
                        "SELECT r.report_json FROM factor_evaluation_report_access a
                         JOIN factor_evaluation_reports r USING(report_hash)
                         WHERE a.user_id = ?1 AND a.report_hash = ?2",
                        params![owner_id, hash],
                        |row| row.get(0),
                    )
                    .map_err(|_| "Promotion Report was not found".to_owned())?;
                serde_json::from_str::<FactorEvaluationReport>(&json).map_err(string)
            })
            .collect::<Result<Vec<_>, String>>()?;
        let lineage = self
            .lineage_for_user(user_id, &protocol.trial_id.to_string())?
            .lineage;
        let eligibility = PromotionEligibility::check(adaq_factor_research::PromotionEvidence {
            candidate: &candidate,
            dataset: &manifest,
            dataset_status: adaq_factor_research::FactorDatasetStatus::Completed,
            evaluation_protocol: &evaluation,
            reports: &reports,
            policy: &policy,
            lineage: &lineage,
            promotion_protocol: protocol,
            component: record.component,
        })
        .map_err(string)?;
        let eligible = matches!(
            record.decision.state,
            adaq_factor_research::PromotionDecisionState::ResearchValidated
                | adaq_factor_research::PromotionDecisionState::ComponentEligible
        ) && record.promotion_protocol_hash == protocol.protocol_hash
            && eligibility.research_validated()
            && (record.decision.state
                != adaq_factor_research::PromotionDecisionState::ComponentEligible
                || eligibility.component_eligible());
        Ok(adaq_factor_research::M12Eligibility {
            eligible,
            reason: (!eligible)
                .then_some("completed output lacks a current frozen promotion evidence set"),
            gates: eligibility.gates().to_vec(),
        })
    }

    fn locked_by(
        &self,
        user_id: &str,
        kind: &str,
        evidence_id: &str,
    ) -> Result<Vec<String>, String> {
        let mut statement = self
            .database
            .prepare("SELECT reference_id FROM factor_references WHERE evidence_kind = ?1 AND evidence_id = ?2 AND referencing_user_id = ?3 ORDER BY reference_id")
            .map_err(string)?;
        statement
            .query_map(
                params![kind, evidence_id, user_uuid_string(user_id)],
                |row| row.get::<_, String>(0),
            )
            .map_err(string)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(string)
    }

    fn assert_visible(&self, user_id: &str, kind: &str, evidence_id: &str) -> Result<(), String> {
        let count = match kind {
            "candidate" => self.database.query_row("SELECT COUNT(*) FROM factor_candidate_access WHERE user_id = ?1 AND candidate_hash = ?2", params![user_id, evidence_id], |row| row.get::<_, i64>(0)),
            "dataset" => self.database.query_row("SELECT COUNT(*) FROM factor_dataset_access WHERE user_id = ?1 AND dataset_id = ?2", params![user_id, evidence_id], |row| row.get::<_, i64>(0)),
            "report" => self.database.query_row("SELECT COUNT(*) FROM factor_evaluation_report_access WHERE user_id = ?1 AND report_hash = ?2", params![user_id, evidence_id], |row| row.get::<_, i64>(0)),
            "policy" => self.database.query_row("SELECT COUNT(*) FROM factor_promotion_policies WHERE user_id = ?1 AND policy_hash = ?2", params![user_uuid_string(user_id), evidence_id], |row| row.get::<_, i64>(0)),
            "family" => self.database.query_row("SELECT COUNT(*) FROM factor_research_families WHERE user_id = ?1 AND family_id = ?2", params![user_uuid_string(user_id), evidence_id], |row| row.get::<_, i64>(0)),
            "decision" => self.database.query_row("SELECT COUNT(*) FROM factor_promotion_decisions WHERE user_id = ?1 AND decision_id = ?2", params![user_uuid_string(user_id), evidence_id], |row| row.get::<_, i64>(0)),
            _ => return Err("unknown Factor evidence reference kind".into()),
        }.map_err(string)?;
        (count == 1)
            .then_some(())
            .ok_or_else(|| "Factor evidence is not visible to this User".into())
    }

    fn reset_device(&self, directory: &Path) -> Result<(), String> {
        let transaction = self.database.unchecked_transaction().map_err(string)?;
        for table in [
            "factor_references",
            "factor_parameter_selections",
            "factor_promotion_decisions",
            "factor_promotion_policies",
            "factor_evaluation_report_access",
            "factor_evaluation_reports",
            "factor_research_trials",
            "factor_research_registrations",
            "factor_research_families",
            "factor_dataset_access",
            "factor_dataset_content",
            "factor_research_attempts",
            "factor_research_protocols",
            "factor_python_host_evidence",
            "factor_candidate_predecessors",
            "factor_candidate_presentations",
            "factor_candidate_access",
            "factor_candidate_content",
            "factor_research_meta",
        ] {
            transaction
                .execute(&format!("DROP TABLE IF EXISTS {table}"), [])
                .map_err(string)?;
        }
        transaction.commit().map_err(string)?;
        if directory.exists() {
            fs::remove_dir_all(directory).map_err(string)?;
        }
        fs::create_dir_all(directory).map_err(string)?;
        self.initialize()
    }

    fn reset_for_user(&self, user_id: &str, directory: &Path) -> Result<(), String> {
        let user_uuid = user_uuid_string(user_id);
        let attempt_ids = self
            .database
            .prepare("SELECT attempt_id FROM factor_research_attempts WHERE user_id = ?1")
            .map_err(string)?
            .query_map([user_id], |row| row.get::<_, String>(0))
            .map_err(string)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(string)?;
        let transaction = self.database.unchecked_transaction().map_err(string)?;
        transaction
            .execute(
                "DELETE FROM factor_references
                  WHERE referencing_user_id IN (?1, ?2)",
                params![user_id, user_uuid_string(user_id)],
            )
            .map_err(string)?;
        transaction
            .execute(
                "DELETE FROM factor_candidate_presentations WHERE user_id = ?1",
                [user_id],
            )
            .map_err(string)?;
        transaction
            .execute(
                "DELETE FROM factor_candidate_predecessors WHERE user_id = ?1",
                [user_id],
            )
            .map_err(string)?;
        transaction
            .execute(
                "DELETE FROM factor_candidate_access WHERE user_id = ?1",
                [user_id],
            )
            .map_err(string)?;
        transaction
            .execute(
                "DELETE FROM factor_dataset_access WHERE user_id = ?1",
                [user_id],
            )
            .map_err(string)?;
        transaction
            .execute(
                "DELETE FROM factor_evaluation_report_access WHERE user_id = ?1",
                [user_id],
            )
            .map_err(string)?;
        transaction
            .execute(
                "DELETE FROM factor_research_attempts WHERE user_id = ?1",
                [user_id],
            )
            .map_err(string)?;
        for table in [
            "factor_research_protocols",
            "factor_research_families",
            "factor_research_registrations",
            "factor_research_trials",
            "factor_promotion_policies",
            "factor_promotion_decisions",
            "factor_parameter_selections",
        ] {
            transaction
                .execute(
                    &format!("DELETE FROM {table} WHERE user_id = ?1"),
                    [user_uuid.as_str()],
                )
                .map_err(string)?;
        }
        let dataset_paths = orphan_paths(
            &transaction,
            "factor_dataset_content",
            "dataset_id",
            "parquet_path",
            "factor_dataset_access",
        )?;
        let report_paths = orphan_paths(
            &transaction,
            "factor_evaluation_reports",
            "report_hash",
            "parquet_path",
            "factor_evaluation_report_access",
        )?;
        transaction.execute("DELETE FROM factor_dataset_content WHERE NOT EXISTS (SELECT 1 FROM factor_dataset_access a WHERE a.dataset_id = factor_dataset_content.dataset_id)", []).map_err(string)?;
        transaction.execute("DELETE FROM factor_evaluation_reports WHERE NOT EXISTS (SELECT 1 FROM factor_evaluation_report_access a WHERE a.report_hash = factor_evaluation_reports.report_hash)", []).map_err(string)?;
        transaction.execute("DELETE FROM factor_candidate_content WHERE NOT EXISTS (SELECT 1 FROM factor_candidate_access a WHERE a.candidate_hash = factor_candidate_content.candidate_hash)", []).map_err(string)?;
        transaction.execute("DELETE FROM factor_candidate_predecessors WHERE NOT EXISTS (SELECT 1 FROM factor_candidate_access a WHERE a.user_id = factor_candidate_predecessors.user_id AND a.candidate_hash = factor_candidate_predecessors.candidate_hash)", []).map_err(string)?;
        transaction.execute("DELETE FROM factor_python_host_evidence WHERE NOT EXISTS (SELECT 1 FROM factor_candidate_access a WHERE a.candidate_hash = factor_python_host_evidence.candidate_hash)", []).map_err(string)?;
        transaction.commit().map_err(string)?;
        for path in dataset_paths.into_iter().chain(report_paths) {
            remove_owned_path(directory, &path)?;
        }
        for attempt_id in attempt_ids {
            for path in [
                directory.join(format!(".factor-{attempt_id}.parquet.tmp")),
                directory
                    .join("reports")
                    .join(format!(".factor-{attempt_id}.metrics.parquet.tmp")),
            ] {
                if path.is_file() {
                    fs::remove_file(path).map_err(string)?;
                }
            }
        }
        Ok(())
    }
}

fn row_to_attempt(row: &rusqlite::Row<'_>) -> rusqlite::Result<FactorAttemptView> {
    let completed = row.get::<_, i64>(7)?;
    let total = row.get::<_, i64>(8)?;
    let kind = row.get::<_, String>(2)?;
    let status = parse_status(&row.get::<_, String>(4)?).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            4,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::other(error)),
        )
    })?;
    let diagnostic = row.get::<_, Option<String>>(9)?;
    Ok(FactorAttemptView {
        attempt_id: row.get(0)?,
        user_id: row.get(1)?,
        kind: kind.clone(),
        request_hash: row.get(3)?,
        status,
        source_attempt_id: row.get(5)?,
        result_id: row.get(6)?,
        completed_units: u64::try_from(completed)
            .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(7, completed))?,
        progress_total: u64::try_from(total)
            .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(8, total))?,
        diagnostic: diagnostic.clone(),
        failure_code: factor_failure_code(&kind, status, diagnostic.as_deref()),
        created_at_ms: row.get(10)?,
        updated_at_ms: row.get(11)?,
    })
}

fn factor_failure_code(
    kind: &str,
    status: AttemptStatus,
    diagnostic: Option<&str>,
) -> Option<FactorFailureCode> {
    if status == AttemptStatus::Cancelled {
        return Some(FactorFailureCode::Cancelled);
    }
    if status != AttemptStatus::Failed {
        return None;
    }
    if let Some(code) = diagnostic
        .and_then(|value| value.split(':').next())
        .map(str::trim)
        .and_then(FactorFailureCode::from_code)
    {
        return Some(code);
    }
    let normalized = diagnostic.unwrap_or_default().to_ascii_lowercase();
    let category = if [
        "hash mismatch",
        "hash collision",
        "identity collision",
        "integrity",
        "corrupt",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
    {
        FactorFailureCode::FactorCorruptionDetected
    } else if normalized.contains("cannot be published")
        || normalized.contains("publication")
        || normalized.contains("staging")
    {
        FactorFailureCode::FactorPublicationFailed
    } else if [
        "not found",
        "not available",
        "not configured",
        "missing",
        "unavailable",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
    {
        FactorFailureCode::FactorMissingInput
    } else if [
        "resource",
        "too large",
        "timed out",
        "timeout",
        "memory",
        "thread",
        "limit",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
    {
        FactorFailureCode::FactorResourceFailed
    } else if [
        "does not match",
        "differs from",
        "not bound",
        "incompatible",
        "not present",
        "requires",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
    {
        FactorFailureCode::FactorCompatibilityFailed
    } else if ["invalid", "validate", "validation", "must be", "empty"]
        .iter()
        .any(|marker| normalized.contains(marker))
    {
        FactorFailureCode::FactorValidationFailed
    } else {
        FactorFailureCode::for_kind(kind)
    };
    Some(category)
}

fn parse_status(value: &str) -> Result<AttemptStatus, String> {
    serde_json::from_value(serde_json::Value::String(value.into())).map_err(string)
}

fn candidate_save_payload(
    user_id: &str,
    candidate: &FactorCandidate,
    presentation: &FactorPresentationMetadata,
    predecessor: Option<&FactorCandidatePredecessor>,
) -> Result<(String, String, Option<String>), String> {
    candidate.validate().map_err(string)?;
    presentation.validate().map_err(string)?;
    if let Some(predecessor) = predecessor {
        predecessor.validate()?;
        if predecessor.user_id != user_id {
            return Err(
                "Factor Candidate predecessor User identity differs from the request".into(),
            );
        }
    }
    let candidate_json = String::from_utf8(candidate.to_json().map_err(string)?).map_err(string)?;
    let presentation_json = serde_json::to_string(presentation).map_err(string)?;
    let predecessor_json = predecessor
        .map(serde_json::to_string)
        .transpose()
        .map_err(string)?;
    Ok((candidate_json, presentation_json, predecessor_json))
}

fn save_candidate_records(
    transaction: &rusqlite::Transaction<'_>,
    user_id: &str,
    candidate: &FactorCandidate,
    candidate_json: &str,
    presentation_json: &str,
    predecessor_json: Option<&str>,
) -> Result<(), String> {
    transaction
        .execute(
            "INSERT OR IGNORE INTO factor_candidate_content(candidate_hash, candidate_json, created_at_ms)
             VALUES (?1, ?2, ?3)",
            params![candidate.candidate_hash, candidate_json, now_ms()],
        )
        .map_err(string)?;
    let stored: String = transaction
        .query_row(
            "SELECT candidate_json FROM factor_candidate_content WHERE candidate_hash = ?1",
            [&candidate.candidate_hash],
            |row| row.get(0),
        )
        .map_err(string)?;
    if stored != candidate_json {
        return Err("Factor Candidate content hash collision".into());
    }
    transaction
        .execute(
            "INSERT OR IGNORE INTO factor_candidate_access(user_id, candidate_hash) VALUES (?1, ?2)",
            params![user_id, candidate.candidate_hash],
        )
        .map_err(string)?;
    transaction
        .execute(
            "INSERT INTO factor_candidate_presentations(user_id, candidate_hash, presentation_json)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(user_id, candidate_hash) DO UPDATE SET presentation_json = excluded.presentation_json",
            params![user_id, candidate.candidate_hash, presentation_json],
        )
        .map_err(string)?;
    if let Some(predecessor_json) = predecessor_json {
        transaction
            .execute(
                "INSERT OR IGNORE INTO factor_candidate_predecessors(user_id, candidate_hash, predecessor_json)
                 VALUES (?1, ?2, ?3)",
                params![user_id, candidate.candidate_hash, predecessor_json],
            )
            .map_err(string)?;
        let stored: String = transaction
            .query_row(
                "SELECT predecessor_json FROM factor_candidate_predecessors
                  WHERE user_id = ?1 AND candidate_hash = ?2",
                params![user_id, candidate.candidate_hash],
                |row| row.get(0),
            )
            .map_err(string)?;
        if stored != predecessor_json {
            return Err("Factor Candidate predecessor identity collision".into());
        }
    }
    Ok(())
}

fn save_family_records(
    transaction: &rusqlite::Transaction<'_>,
    registration: &ResearchFamilyRegistration,
    trials: &[ResearchTrial],
) -> Result<(), String> {
    let family_json = serde_json::to_string(&registration.family).map_err(string)?;
    let stored_family: Option<String> = transaction
        .query_row(
            "SELECT family_json FROM factor_research_families WHERE family_id = ?1",
            [registration.family.family_id.to_string()],
            |row| row.get(0),
        )
        .optional()
        .map_err(string)?;
    if stored_family.is_some_and(|existing| existing != family_json) {
        return Err("Research Family content identity collision".into());
    }
    transaction
        .execute(
            "INSERT OR IGNORE INTO factor_research_families(family_id, user_id, family_json, created_at_ms)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                registration.family.family_id.to_string(),
                registration.family.user_id.to_string(),
                family_json,
                now_ms()
            ],
        )
        .map_err(string)?;
    for trial in &registration.trials {
        let json = serde_json::to_string(trial).map_err(string)?;
        let stored_trial: Option<String> = transaction
            .query_row(
                "SELECT registration_json FROM factor_research_registrations WHERE trial_id = ?1",
                [trial.trial_id.to_string()],
                |row| row.get(0),
            )
            .optional()
            .map_err(string)?;
        if stored_trial.is_some_and(|existing| existing != json) {
            return Err("Research Trial registration content identity collision".into());
        }
        transaction
            .execute(
                "INSERT OR IGNORE INTO factor_research_registrations(trial_id, family_id, user_id, registration_json)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    trial.trial_id.to_string(),
                    trial.family_id.to_string(),
                    registration.family.user_id.to_string(),
                    json
                ],
            )
            .map_err(string)?;
    }
    for trial in trials {
        let json = serde_json::to_string(trial).map_err(string)?;
        transaction
            .execute(
                "INSERT OR IGNORE INTO factor_research_trials(trial_id, family_id, user_id, trial_json, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(trial_id) DO UPDATE SET trial_json = excluded.trial_json, updated_at_ms = excluded.updated_at_ms",
                params![
                    trial.trial_id.to_string(),
                    trial.family_id.to_string(),
                    registration.family.user_id.to_string(),
                    json,
                    now_ms()
                ],
            )
            .map_err(string)?;
    }
    Ok(())
}

fn page_params(request: &FactorPageRequest) -> Result<(u32, u32, u64), String> {
    if request.page == 0 {
        return Err("Factor evidence page must be greater than zero".into());
    }
    let page_size = request.page_size.unwrap_or(DEFAULT_PAGE_SIZE);
    if page_size == 0 || page_size > MAX_PAGE_SIZE {
        return Err("Factor evidence page size is invalid".into());
    }
    let offset = u64::from(request.page - 1)
        .checked_mul(u64::from(page_size))
        .ok_or_else(|| "Factor evidence page is too large".to_owned())?;
    Ok((request.page, page_size, offset))
}

fn user_uuid_string(user_id: &str) -> String {
    user_uuid(user_id).to_string()
}

fn feature_reference_id(dataset_id: &str) -> String {
    format!("factor-dataset:{dataset_id}")
}

fn validate_reference_kind(kind: &str) -> Result<(), String> {
    matches!(
        kind,
        "candidate" | "dataset" | "report" | "policy" | "family" | "decision"
    )
    .then_some(())
    .ok_or_else(|| "unknown Factor evidence reference kind".into())
}

fn orphan_paths(
    transaction: &rusqlite::Transaction<'_>,
    content_table: &str,
    id_column: &str,
    path_column: &str,
    access_table: &str,
) -> Result<Vec<String>, String> {
    let sql = format!(
        "SELECT c.{path_column} FROM {content_table} c
          WHERE NOT EXISTS (SELECT 1 FROM {access_table} a WHERE a.{id_column} = c.{id_column})"
    );
    let mut statement = transaction.prepare(&sql).map_err(string)?;
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(string)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(string)
}

fn remove_owned_path(directory: &Path, path: &str) -> Result<(), String> {
    let path = Path::new(path);
    if !path.starts_with(directory)
        || path
            .extension()
            .is_none_or(|extension| extension != "parquet")
    {
        return Err("incompatible Factor evidence payload path".into());
    }
    if path.is_file() {
        fs::remove_file(path).map_err(string)?;
    }
    Ok(())
}

fn factor_schema(output_names: &[String]) -> Arc<Schema> {
    let mut fields = vec![
        Field::new("instrument_id", DataType::Utf8, false),
        Field::new("observation_time_ms", DataType::Int64, false),
    ];
    for output_name in output_names {
        let prefix = format!("factor__{output_name}");
        fields.extend([
            Field::new(format!("{prefix}__value"), DataType::Float64, true),
            Field::new(format!("{prefix}__available_at_ms"), DataType::Int64, true),
            Field::new(format!("{prefix}__state"), DataType::Utf8, false),
            Field::new(format!("{prefix}__reason"), DataType::Utf8, true),
        ]);
    }
    Arc::new(Schema::new(fields))
}

fn write_factor_parquet(
    path: &Path,
    output_names: &[String],
    rows: &[FactorDatasetRow],
) -> Result<(), String> {
    let schema = factor_schema(output_names);
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
                    Some(FactorObservationValue::Available { value, .. }) => Some(*value),
                    _ => None,
                })
                .collect::<Vec<_>>(),
        )));
        columns.push(Arc::new(Int64Array::from(
            rows.iter()
                .map(|row| match row.values.get(output_name) {
                    Some(FactorObservationValue::Available {
                        available_at_ms, ..
                    }) => Some(*available_at_ms),
                    _ => None,
                })
                .collect::<Vec<_>>(),
        )));
        columns.push(Arc::new(StringArray::from_iter_values(rows.iter().map(
            |row| match row.values.get(output_name) {
                Some(FactorObservationValue::Available { .. }) => "available",
                Some(FactorObservationValue::Unavailable { .. }) => "unavailable",
                None => "invalid",
            },
        ))));
        columns.push(Arc::new(StringArray::from_iter(rows.iter().map(|row| {
            match row.values.get(output_name) {
                Some(FactorObservationValue::Unavailable { reason }) => Some(reason_code(*reason)),
                Some(FactorObservationValue::Available { .. }) => None,
                None => Some("invalid"),
            }
        }))));
    }
    let batch = RecordBatch::try_new(schema.clone(), columns).map_err(string)?;
    let file = File::create(path).map_err(string)?;
    let mut writer = ArrowWriter::try_new(file, schema, None).map_err(string)?;
    writer.write(&batch).map_err(string)?;
    writer.close().map_err(string)?;
    Ok(())
}

fn read_factor_rows(
    path: &Path,
    output_names: &[String],
    offset: u64,
    limit: u32,
    instrument_id: Option<&str>,
) -> Result<(Vec<FactorDatasetRow>, u64), String> {
    let file = File::open(path).map_err(string)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).map_err(string)?;
    let schema = factor_schema(output_names);
    if builder.schema().as_ref() != schema.as_ref() {
        return Err("Factor Dataset Parquet schema mismatch".into());
    }
    let reader = builder.with_batch_size(8192).build().map_err(string)?;
    let mut rows = Vec::new();
    let mut matched_rows = 0u64;
    for batch in reader {
        let batch = batch.map_err(string)?;
        let instruments = array::<StringArray>(&batch, 0)?;
        let times = array::<Int64Array>(&batch, 1)?;
        let columns = output_names
            .iter()
            .enumerate()
            .map(|(index, _)| {
                let base = 2 + index * 4;
                Ok((
                    array::<Float64Array>(&batch, base)?,
                    array::<Int64Array>(&batch, base + 1)?,
                    array::<StringArray>(&batch, base + 2)?,
                    array::<StringArray>(&batch, base + 3)?,
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        for index in 0..batch.num_rows() {
            let values = output_names
                .iter()
                .zip(&columns)
                .map(|(name, (numbers, available_at, state, reason))| {
                    let value = match state.value(index) {
                        "available"
                            if !numbers.is_null(index)
                                && !available_at.is_null(index)
                                && reason.is_null(index) =>
                        {
                            FactorObservationValue::Available {
                                value: numbers.value(index),
                                available_at_ms: available_at.value(index),
                            }
                        }
                        "unavailable"
                            if numbers.is_null(index)
                                && available_at.is_null(index)
                                && !reason.is_null(index) =>
                        {
                            FactorObservationValue::Unavailable {
                                reason: reason_from_code(reason.value(index))?,
                            }
                        }
                        _ => return Err("invalid Factor Dataset Parquet cell".to_owned()),
                    };
                    Ok((name.clone(), value))
                })
                .collect::<Result<BTreeMap<_, _>, String>>()?;
            let matches_filter = instrument_id
                .map(|filter| instruments.value(index).contains(filter))
                .unwrap_or(true);
            if matches_filter && matched_rows >= offset && rows.len() < limit as usize {
                rows.push(FactorDatasetRow {
                    instrument_id: instruments.value(index).into(),
                    observation_time_ms: times.value(index),
                    values,
                });
            }
            if matches_filter {
                matched_rows = matched_rows.saturating_add(1);
            }
        }
    }
    Ok((rows, matched_rows))
}

fn array<T: Array + 'static>(batch: &RecordBatch, index: usize) -> Result<&T, String> {
    batch
        .column(index)
        .as_any()
        .downcast_ref::<T>()
        .ok_or_else(|| "Factor Dataset Parquet schema mismatch".into())
}

fn reason_code(reason: FactorUnavailabilityReason) -> &'static str {
    match reason {
        FactorUnavailabilityReason::Warmup => "warmup",
        FactorUnavailabilityReason::BarGap => "bar-gap",
        FactorUnavailabilityReason::MissingInput => "missing-input",
        FactorUnavailabilityReason::MissingDependency => "missing-dependency",
        FactorUnavailabilityReason::NotYetAvailable => "not-yet-available",
        FactorUnavailabilityReason::UnknownUniverse => "unknown-universe",
        FactorUnavailabilityReason::InsufficientCoverage => "insufficient-coverage",
        FactorUnavailabilityReason::UndefinedArithmetic => "undefined-arithmetic",
        FactorUnavailabilityReason::InvalidUpstream => "invalid-upstream",
    }
}

fn reason_from_code(code: &str) -> Result<FactorUnavailabilityReason, String> {
    Ok(match code {
        "warmup" => FactorUnavailabilityReason::Warmup,
        "bar-gap" => FactorUnavailabilityReason::BarGap,
        "missing-input" => FactorUnavailabilityReason::MissingInput,
        "missing-dependency" => FactorUnavailabilityReason::MissingDependency,
        "not-yet-available" => FactorUnavailabilityReason::NotYetAvailable,
        "unknown-universe" => FactorUnavailabilityReason::UnknownUniverse,
        "insufficient-coverage" => FactorUnavailabilityReason::InsufficientCoverage,
        "undefined-arithmetic" => FactorUnavailabilityReason::UndefinedArithmetic,
        "invalid-upstream" => FactorUnavailabilityReason::InvalidUpstream,
        _ => return Err("unknown Factor Dataset unavailability reason".into()),
    })
}

fn write_report_parquet(path: &Path, report: &FactorEvaluationReport) -> Result<(), String> {
    let schema = Arc::new(Schema::new(vec![
        Field::new("fold_id", DataType::Utf8, false),
        Field::new("variant", DataType::Utf8, false),
        Field::new("horizon_bars", DataType::Int64, false),
        Field::new("output_name", DataType::Utf8, false),
        Field::new("lens", DataType::Utf8, false),
        Field::new("metric", DataType::Utf8, false),
        Field::new("observation_json", DataType::Utf8, false),
    ]));
    let lens = report
        .metrics
        .iter()
        .map(|metric| serde_json::to_string(&metric.lens).unwrap_or_default())
        .collect::<Vec<_>>();
    let metric = report
        .metrics
        .iter()
        .map(|metric| serde_json::to_string(&metric.metric).unwrap_or_default())
        .collect::<Vec<_>>();
    let observations = report
        .metrics
        .iter()
        .map(|metric| serde_json::to_string(&metric.observation).unwrap_or_default())
        .collect::<Vec<_>>();
    let columns: Vec<ArrayRef> = vec![
        Arc::new(StringArray::from_iter_values(
            report.metrics.iter().map(|metric| metric.fold_id.as_str()),
        )),
        Arc::new(StringArray::from_iter_values(
            report.metrics.iter().map(|metric| metric.variant.as_str()),
        )),
        Arc::new(Int64Array::from_iter_values(
            report
                .metrics
                .iter()
                .map(|metric| i64::from(metric.horizon_bars)),
        )),
        Arc::new(StringArray::from_iter_values(
            report
                .metrics
                .iter()
                .map(|metric| metric.output_name.as_str()),
        )),
        Arc::new(StringArray::from_iter_values(
            lens.iter().map(String::as_str),
        )),
        Arc::new(StringArray::from_iter_values(
            metric.iter().map(String::as_str),
        )),
        Arc::new(StringArray::from_iter_values(
            observations.iter().map(String::as_str),
        )),
    ];
    let batch = RecordBatch::try_new(schema.clone(), columns).map_err(string)?;
    let file = File::create(path).map_err(string)?;
    let mut writer = ArrowWriter::try_new(file, schema, None).map_err(string)?;
    writer.write(&batch).map_err(string)?;
    writer.close().map(|_| ()).map_err(string)
}

fn evaluation_observation_range(
    protocol: &FactorEvaluationProtocol,
) -> Result<ObservationRange, String> {
    let mut start_time_ms = i64::MAX;
    let mut end_time_ms = i64::MIN;
    let mut include = |range: &ObservationRange| {
        start_time_ms = start_time_ms.min(range.start_time_ms);
        end_time_ms = end_time_ms.max(range.end_time_ms);
    };
    for window in &protocol.windows {
        include(&window.selection);
        include(&window.evaluation);
        for range in [
            window.training.as_ref(),
            window.fitting.as_ref(),
            window.normalization.as_ref(),
            window.target_construction.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            include(range);
        }
    }
    if start_time_ms == i64::MAX || end_time_ms == i64::MIN {
        return Err("factor-context-range-mismatch".into());
    }
    Ok(ObservationRange {
        start_time_ms,
        end_time_ms,
    })
}

fn validate_evaluation_boundary(
    candidate: &FactorCandidate,
    predecessor: Option<&FactorCandidatePredecessor>,
    manifest: &FactorDatasetManifest,
    protocol: &FactorEvaluationProtocol,
) -> Result<(), String> {
    if candidate.candidate_hash != manifest.candidate_hash
        || candidate.scope != manifest.scope
        || candidate.scope != protocol.scope
        || manifest.market_context != protocol.market_context
        || manifest.feature_dataset_id != protocol.feature_dataset_id
        || manifest.feature_plan_hash != protocol.feature_plan_hash
        || manifest.market_data_snapshot_id != protocol.market_data_snapshot_id
        || manifest.point_in_time_universe_id != protocol.point_in_time_universe_id
        || manifest.engine_identity != protocol.engine_identity
        || !candidate
            .outputs
            .iter()
            .any(|output| output.name == protocol.output_name)
        || !manifest
            .output_names
            .iter()
            .any(|output| output == &protocol.output_name)
    {
        return Err(
            "Factor Evaluation boundary is not bound to the exact Candidate or Dataset".into(),
        );
    }
    let range = evaluation_observation_range(protocol)?;
    if manifest
        .observation_range
        .as_ref()
        .is_some_and(|dataset_range| {
            range.start_time_ms < dataset_range.start_time_ms
                || range.end_time_ms > dataset_range.end_time_ms
        })
    {
        return Err("factor-context-range-mismatch".into());
    }
    let Some(predecessor) = predecessor else {
        // Python-hosted candidates predate the retained context predecessor; the
        // context-owned entry point performs the stricter predecessor check.
        return Ok(());
    };
    let feature_dataset = &predecessor.feature_dataset;
    if feature_dataset.dataset_id != manifest.feature_dataset_id
        || feature_dataset.feature_plan_hash != manifest.feature_plan_hash
        || predecessor.snapshot_id != manifest.market_data_snapshot_id
        || predecessor.universe_id.as_deref() != Some(&manifest.point_in_time_universe_id)
        || predecessor.market != protocol.market_context.asset_class
        || predecessor.venue != protocol.market_context.venue
    {
        return Err("factor-context-mismatch".into());
    }
    if range.start_time_ms < predecessor.range_start_ms
        || range.end_time_ms > predecessor.range_end_ms
    {
        return Err("factor-context-range-mismatch".into());
    }
    Ok(())
}

fn initial_trial(registration: &ResearchTrialRegistration) -> ResearchTrial {
    ResearchTrial {
        trial_id: registration.trial_id,
        family_id: registration.family_id,
        candidate_hash: registration.candidate_hash.clone(),
        protocol_hash: registration.evaluation_protocol_hash.clone(),
        status: ResearchTrialStatus::Registered,
        report_hash: None,
        raw_statistic: None,
        p_value: None,
        holm_adjusted: None,
        related_trial_ids: Vec::new(),
        diagnostic: None,
    }
}

fn complete_evaluation_trial(
    transaction: &rusqlite::Transaction<'_>,
    user_id: &str,
    protocol: &FactorEvaluationProtocol,
    report: &FactorEvaluationReport,
    lineage: &FactorLineageView,
    candidate_hash: &str,
    raw_statistic: Option<MetricObservation>,
    p_value: Option<MetricObservation>,
) -> Result<(), String> {
    let current_registration = lineage
        .registrations
        .iter()
        .find(|registration| registration.trial_id == protocol.trial_id)
        .ok_or_else(|| "Research Trial registration was not found".to_owned())?;
    if current_registration.family_id != protocol.family_id
        || current_registration.candidate_hash != candidate_hash
        || current_registration.target != protocol.target
        || current_registration.market_context != protocol.market_context
        || current_registration.point_in_time_universe_id != protocol.point_in_time_universe_id
        || current_registration.evaluation_protocol_hash != protocol.protocol_hash
        || current_registration.observation_range != evaluation_observation_range(protocol)?
    {
        return Err("Factor Evaluation Report is not bound to its registered Trial".into());
    }

    let mut states = lineage
        .lineage
        .trial_ids
        .iter()
        .map(|trial_id| {
            let registration = lineage
                .registrations
                .iter()
                .find(|registration| registration.trial_id == *trial_id)
                .ok_or_else(|| "Research Trial registration was not found".to_owned())?;
            let stored: Option<String> = transaction
                .query_row(
                    "SELECT trial_json FROM factor_research_trials
                     WHERE trial_id = ?1 AND user_id = ?2",
                    params![trial_id.to_string(), user_uuid_string(user_id)],
                    |row| row.get(0),
                )
                .optional()
                .map_err(string)?;
            let trial = stored
                .map(|json| serde_json::from_str::<ResearchTrial>(&json).map_err(string))
                .transpose()?
                .unwrap_or_else(|| initial_trial(registration));
            trial.validate().map_err(string)?;
            if trial.family_id != registration.family_id
                || trial.candidate_hash != registration.candidate_hash
                || trial.protocol_hash != registration.evaluation_protocol_hash
            {
                return Err("Research Trial state is not bound to its registration".into());
            }
            Ok((*trial_id, trial))
        })
        .collect::<Result<Vec<_>, String>>()?;

    let current = states
        .iter_mut()
        .find(|(trial_id, _)| *trial_id == protocol.trial_id)
        .ok_or_else(|| "Research Trial is not part of its registered lineage".to_owned())?;
    match current.1.status {
        ResearchTrialStatus::Registered => {
            current.1.status = ResearchTrialStatus::Completed;
            current.1.report_hash = Some(report.report_hash.clone());
            current.1.raw_statistic = raw_statistic;
            current.1.p_value = p_value;
            current.1.diagnostic = None;
        }
        ResearchTrialStatus::Completed
            if current.1.report_hash.as_deref() == Some(report.report_hash.as_str()) => {}
        _ => return Err("Research Trial cannot be completed from its current state".into()),
    }
    current.1.validate().map_err(string)?;

    let correction = adaq_factor_research::holm_bonferroni(
        &states
            .iter()
            .map(|(trial_id, trial)| (*trial_id, trial.p_value.clone()))
            .collect::<Vec<_>>(),
    )
    .map_err(string)?;
    let related = lineage
        .lineage
        .trial_ids
        .iter()
        .copied()
        .collect::<Vec<_>>();
    for (trial_id, mut trial) in states {
        trial.holm_adjusted = correction.adjusted_p_values.get(&trial_id).cloned();
        trial.related_trial_ids = related
            .iter()
            .copied()
            .filter(|related_id| *related_id != trial_id)
            .collect();
        trial.validate().map_err(string)?;
        transaction
            .execute(
                "INSERT INTO factor_research_trials(
                    trial_id, family_id, user_id, trial_json, updated_at_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(trial_id) DO UPDATE SET
                    trial_json = excluded.trial_json, updated_at_ms = excluded.updated_at_ms",
                params![
                    trial_id.to_string(),
                    trial.family_id.to_string(),
                    user_uuid_string(user_id),
                    serde_json::to_string(&trial).map_err(string)?,
                    now_ms()
                ],
            )
            .map_err(string)?;
    }
    Ok(())
}

pub(crate) fn factor_trial_statistics(
    report: &FactorEvaluationReport,
) -> Result<(Option<MetricObservation>, Option<MetricObservation>), String> {
    let observations = report
        .metrics
        .iter()
        .filter(|metric| metric.metric == MetricId::Ic)
        .filter_map(|metric| {
            metric.observation.value().map(|value| {
                let sample_count = match &metric.observation {
                    MetricObservation::Available { sample_count, .. }
                    | MetricObservation::Unavailable { sample_count, .. } => *sample_count,
                };
                (value, sample_count)
            })
        })
        .collect::<Vec<_>>();
    if observations.is_empty() {
        return Ok((None, None));
    }
    let sample_count = observations
        .iter()
        .map(|(_, sample_count)| *sample_count)
        .sum::<u64>();
    let raw = observations.iter().map(|(value, _)| *value).sum::<f64>() / observations.len() as f64;
    let raw_statistic = MetricObservation::available(raw, sample_count).map_err(string)?;
    let p_value = (sample_count > 2).then(|| {
        let z = raw.abs() * ((sample_count - 2) as f64 / (1.0 - raw * raw).max(1e-12)).sqrt();
        MetricObservation::available((2.0 * normal_upper_tail(z)).clamp(0.0, 1.0), sample_count)
            .expect("normal approximation is finite")
    });
    Ok((Some(raw_statistic), p_value))
}

fn normal_upper_tail(value: f64) -> f64 {
    let value = value.abs();
    let t = 1.0 / (1.0 + 0.2316419 * value);
    let density = (-0.5 * value * value).exp() * 0.3989422804014327;
    density
        * t
        * (0.319381530
            + t * (-0.356563782 + t * (1.781477937 + t * (-1.821255978 + t * 1.330274429))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::research_queue::ResearchQueue;
    use adaq_factor_research::{
        EconomicAssumptions, EvaluationWindow, FactorLens, FactorOrientation,
    };
    fn store() -> Connection {
        let database = Connection::open_in_memory().unwrap();
        ResearchStore::new(&database).initialize().unwrap();
        database
    }

    #[test]
    fn device_reset_recreates_factor_schema_explicitly() {
        let database = store();
        database
            .execute(
                "UPDATE factor_research_meta SET value = '1.0.0' WHERE key = 'schema_version'",
                [],
            )
            .unwrap();
        let directory = std::env::temp_dir().join(format!("adaq-factor-reset-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        ResearchStore::new(&database)
            .reset_device(&directory)
            .unwrap();
        assert_eq!(
            database
                .query_row(
                    "SELECT value FROM factor_research_meta WHERE key = 'schema_version'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            STORAGE_SCHEMA_VERSION
        );
        assert!(
            database
                .query_row("SELECT COUNT(*) FROM factor_candidate_content", [], |row| {
                    row.get::<_, i64>(0)
                },)
                .is_ok()
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn incompatible_factor_schema_is_rejected_before_other_tables_are_created() {
        let database = Connection::open_in_memory().unwrap();
        database
            .execute_batch(
                "CREATE TABLE factor_research_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 INSERT INTO factor_research_meta(key, value) VALUES ('schema_version', '1.0.0');",
            )
            .unwrap();
        assert!(ResearchStore::new(&database).initialize().is_err());
        assert!(
            database
                .query_row(
                    "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'factor_candidate_content'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .is_err()
        );
    }

    #[test]
    fn factor_schema_without_version_requires_explicit_reset() {
        let database = Connection::open_in_memory().unwrap();
        database
            .execute_batch(
                "CREATE TABLE factor_research_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
            )
            .unwrap();
        let error = ResearchStore::new(&database).initialize().unwrap_err();
        assert!(error.starts_with("reset-required:"));
        assert!(
            database
                .query_row(
                    "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'factor_candidate_content'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .is_err()
        );
    }

    #[test]
    fn incompatible_factor_storage_remains_resettable_through_open_research() {
        let database = Arc::new(Mutex::new(store()));
        database
            .lock()
            .unwrap()
            .execute(
                "UPDATE factor_research_meta SET value = '1.0.0' WHERE key = 'schema_version'",
                [],
            )
            .unwrap();
        let directory = tempfile_dir("factor-open-reset");
        let queue = ResearchQueue::open(database.clone()).unwrap();
        let research = FactorResearch::open(
            Arc::new(TestSource {
                database: database.clone(),
                directory: directory.clone(),
            }),
            queue.admitter(),
        )
        .unwrap();

        let error = research
            .list_candidates(FactorPageRequest {
                user_id: "alice".into(),
                page: 1,
                page_size: None,
                kind: None,
            })
            .unwrap_err();
        assert!(error.starts_with("reset-required:"));
        research.reset_for_device().unwrap();
        assert_eq!(
            research
                .list_candidates(FactorPageRequest {
                    user_id: "alice".into(),
                    page: 1,
                    page_size: None,
                    kind: None,
                })
                .unwrap()
                .total,
            0
        );
        fs::remove_dir_all(directory).unwrap();
    }

    fn row(time: i64, value: f64) -> FactorDatasetRow {
        FactorDatasetRow {
            instrument_id: "btc-usdt".into(),
            observation_time_ms: time,
            values: BTreeMap::from([(
                "momentum".into(),
                FactorObservationValue::Available {
                    value,
                    available_at_ms: time,
                },
            )]),
        }
    }

    #[test]
    fn attempts_are_fifo_user_scoped_and_retryable() {
        let database = store();
        let store = ResearchStore::new(&database);
        let (first, _) = store
            .start_attempt("alice", "factor-evaluation", "{\"n\":1}")
            .unwrap();
        let (second, _) = store
            .start_attempt("bob", "factor-evaluation", "{\"n\":2}")
            .unwrap();
        assert_eq!(store.pending_attempts().unwrap()[0].1, first.attempt_id);
        assert!(store.attempt_for_user("bob", &first.attempt_id).is_err());
        assert_eq!(
            store.begin_attempt(&first.attempt_id).unwrap().1,
            "factor-evaluation"
        );
        store.fail_attempt(&first.attempt_id, "failure").unwrap();
        assert_eq!(
            store
                .attempt_for_user("alice", &first.attempt_id)
                .unwrap()
                .failure_code,
            Some(FactorFailureCode::FactorEvaluationFailed)
        );
        let (retry, should_start) = store.retry_attempt("alice", &first.attempt_id).unwrap();
        assert!(should_start);
        assert_eq!(
            retry.source_attempt_id.as_deref(),
            Some(first.attempt_id.as_str())
        );
        assert_eq!(store.pending_attempts().unwrap()[0].1, second.attempt_id);
        store
            .start_attempt("alice", "factor-materialization", "{\"n\":3}")
            .unwrap();
        let page = store
            .list_attempts(&FactorPageRequest {
                user_id: "alice".into(),
                page: 1,
                page_size: Some(10),
                kind: Some("factor-evaluation".into()),
            })
            .unwrap();
        assert_eq!(page.total, 2);
        assert_eq!(page.items.len(), 2);
        assert!(
            page.items
                .iter()
                .all(|item| item.kind == "factor-evaluation")
        );
    }

    #[test]
    fn factor_failure_codes_keep_actionable_categories() {
        for (diagnostic, expected) in [
            (
                "Factor Dataset content hash mismatch",
                "factor-corruption-detected",
            ),
            (
                "Factor Evaluation Attempt cannot be published",
                "factor-publication-failed",
            ),
            ("Factor Dataset was not found", "factor-missing-input"),
            ("factor resource limit exceeded", "factor-resource-failed"),
            (
                "Factor Dataset is not bound to the exact protocol",
                "factor-compatibility-failed",
            ),
            ("Factor output is invalid", "factor-validation-failed"),
        ] {
            assert_eq!(
                factor_failure_code("factor-evaluation", AttemptStatus::Failed, Some(diagnostic)),
                FactorFailureCode::from_code(expected)
            );
        }
    }

    #[test]
    fn running_cancellation_is_durable_before_publication() {
        let database = store();
        let store = ResearchStore::new(&database);
        let (attempt, _) = store
            .start_attempt("alice", "factor-materialization", "{\"n\":1}")
            .unwrap();
        store.begin_attempt(&attempt.attempt_id).unwrap();

        assert_eq!(
            store.cancel_attempt("alice", &attempt.attempt_id).unwrap(),
            AttemptStatus::Running
        );
        let requested = store
            .attempt_for_user("alice", &attempt.attempt_id)
            .unwrap();
        assert_eq!(requested.status, AttemptStatus::Running);
        assert_eq!(
            requested.diagnostic.as_deref(),
            Some(CANCELLATION_REQUESTED_DIAGNOSTIC)
        );
        assert!(store.retry_attempt("alice", &attempt.attempt_id).is_err());
        store.cancel_running(&attempt.attempt_id, "alice").unwrap();
        let cancelled = store
            .attempt_for_user("alice", &attempt.attempt_id)
            .unwrap();
        assert_eq!(cancelled.status, AttemptStatus::Cancelled);
        assert!(store.retry_attempt("alice", &attempt.attempt_id).is_ok());

        let (reset_attempt, _) = store
            .start_attempt("alice", "factor-evaluation", "{\"n\":2}")
            .unwrap();
        store.begin_attempt(&reset_attempt.attempt_id).unwrap();
        store.cancel_for_reset("alice").unwrap();
        assert_eq!(
            store
                .attempt_for_user("alice", &reset_attempt.attempt_id)
                .unwrap()
                .status,
            AttemptStatus::Running
        );
        store
            .cancel_running(&reset_attempt.attempt_id, "alice")
            .unwrap();
        assert_eq!(
            store
                .attempt_for_user("alice", &reset_attempt.attempt_id)
                .unwrap()
                .status,
            AttemptStatus::Cancelled
        );
    }

    #[test]
    fn cancellation_request_blocks_candidate_publication() {
        let database = store();
        let store = ResearchStore::new(&database);
        let (attempt, _) = store
            .start_attempt("alice", "candidate-build", "{}")
            .unwrap();
        store.begin_attempt(&attempt.attempt_id).unwrap();
        assert_eq!(
            store.cancel_attempt("alice", &attempt.attempt_id).unwrap(),
            AttemptStatus::Running
        );
        let candidate = test_candidate();
        let error = store
            .save_candidate_for_attempt(
                "alice",
                &attempt.attempt_id,
                &candidate,
                &FactorPresentationMetadata {
                    name: "Cancelled".into(),
                    description: String::new(),
                    tags: Vec::new(),
                },
            )
            .unwrap_err();
        assert_eq!(error, "cancelled");
        assert!(
            store
                .candidate_for_user("alice", &candidate.candidate_hash)
                .is_err()
        );
        store.cancel_running(&attempt.attempt_id, "alice").unwrap();

        let (family_attempt, _) = store
            .start_attempt("alice", "factor-family-grid", "{}")
            .unwrap();
        store.begin_attempt(&family_attempt.attempt_id).unwrap();
        store
            .cancel_attempt("alice", &family_attempt.attempt_id)
            .unwrap();
        let registration = ResearchFamilyRegistration {
            family: ResearchFamily {
                schema_version: String::new(),
                family_id: Uuid::new_v4(),
                user_id: user_uuid("alice"),
                root_candidate_hash: String::new(),
                parent_family_id: None,
                registered_trial_ids: Vec::new(),
                lineage_hash: String::new(),
            },
            trials: Vec::new(),
        };
        assert_eq!(
            store
                .save_family_for_attempt("alice", &family_attempt.attempt_id, &registration, &[])
                .unwrap_err(),
            "cancelled"
        );
        assert_eq!(
            database
                .query_row("SELECT COUNT(*) FROM factor_research_families", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            0
        );
        store
            .cancel_running(&family_attempt.attempt_id, "alice")
            .unwrap();
    }

    #[test]
    fn reset_removes_raw_user_attempts_without_touching_another_user() {
        let database = store();
        let store = ResearchStore::new(&database);
        let (alice, _) = store
            .start_attempt("alice", "factor-evaluation", "{\"n\":1}")
            .unwrap();
        let (bob, _) = store
            .start_attempt("bob", "factor-evaluation", "{\"n\":2}")
            .unwrap();
        let directory = tempfile_dir("factor-reset");
        fs::create_dir_all(directory.join("reports")).unwrap();
        let alice_staging = directory.join(format!(".factor-{}.parquet.tmp", alice.attempt_id));
        let bob_staging = directory.join(format!(".factor-{}.parquet.tmp", bob.attempt_id));
        let bob_report_staging = directory
            .join("reports")
            .join(format!(".factor-{}.metrics.parquet.tmp", bob.attempt_id));
        fs::write(&alice_staging, b"alice").unwrap();
        fs::write(&bob_staging, b"bob").unwrap();
        fs::write(&bob_report_staging, b"bob-report").unwrap();
        store.reset_for_user("alice", &directory).unwrap();
        assert!(store.attempt_for_user("alice", &alice.attempt_id).is_err());
        assert!(store.attempt_for_user("bob", &bob.attempt_id).is_ok());
        assert!(!alice_staging.exists());
        assert!(bob_staging.exists());
        assert!(bob_report_staging.exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn stale_running_attempts_become_typed_failures() {
        let database = Arc::new(Mutex::new(store()));
        let directory = tempfile_dir("factor-recovery");
        let temporary = directory.join(".stale.parquet.tmp");
        fs::write(&temporary, b"stale").unwrap();
        let attempt = {
            let database = database.lock().unwrap();
            let store = ResearchStore::new(&database);
            let (attempt, _) = store
                .start_attempt("alice", "factor-materialization", "{}")
                .unwrap();
            store.begin_attempt(&attempt.attempt_id).unwrap();
            attempt
        };
        let source: Arc<dyn FactorResearchSource> = Arc::new(TestSource {
            database: database.clone(),
            directory: directory.clone(),
        });
        ResearchStore::recover_stale_attempts(&directory, &source).unwrap();
        let database = database.lock().unwrap();
        let recovered = ResearchStore::new(&database)
            .attempt_for_user("alice", &attempt.attempt_id)
            .unwrap();
        assert_eq!(recovered.status, AttemptStatus::Failed);
        assert!(
            recovered
                .diagnostic
                .as_deref()
                .is_some_and(|diagnostic| diagnostic.contains("research-interrupted"))
        );
        assert_eq!(
            recovered.failure_code,
            Some(FactorFailureCode::ResearchInterrupted)
        );
        assert!(!temporary.exists());
        drop(database);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn stale_cancel_requested_attempts_remain_cancelled() {
        let database = Arc::new(Mutex::new(store()));
        let directory = tempfile_dir("factor-cancel-recovery");
        let attempt = {
            let database = database.lock().unwrap();
            let store = ResearchStore::new(&database);
            let (attempt, _) = store
                .start_attempt("alice", "factor-materialization", "{}")
                .unwrap();
            store.begin_attempt(&attempt.attempt_id).unwrap();
            store.cancel_attempt("alice", &attempt.attempt_id).unwrap();
            attempt
        };
        let source: Arc<dyn FactorResearchSource> = Arc::new(TestSource {
            database: database.clone(),
            directory: directory.clone(),
        });
        ResearchStore::recover_stale_attempts(&directory, &source).unwrap();
        let database = database.lock().unwrap();
        let recovered = ResearchStore::new(&database)
            .attempt_for_user("alice", &attempt.attempt_id)
            .unwrap();
        assert_eq!(recovered.status, AttemptStatus::Cancelled);
        assert_eq!(recovered.failure_code, Some(FactorFailureCode::Cancelled));
        drop(database);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn shutdown_cancellation_is_durable_before_publication() {
        let database = store();
        let store = ResearchStore::new(&database);
        let (attempt, _) = store
            .start_attempt("alice", "candidate-build", "{}")
            .unwrap();
        store.begin_attempt(&attempt.attempt_id).unwrap();
        store.request_shutdown_cancellation().unwrap();
        let requested = store
            .attempt_for_user("alice", &attempt.attempt_id)
            .unwrap();
        assert_eq!(requested.status, AttemptStatus::Running);
        assert_eq!(
            requested.diagnostic.as_deref(),
            Some(CANCELLATION_REQUESTED_DIAGNOSTIC)
        );
        assert_eq!(
            store
                .save_candidate_for_attempt(
                    "alice",
                    &attempt.attempt_id,
                    &test_candidate(),
                    &FactorPresentationMetadata {
                        name: "Shutdown".into(),
                        description: String::new(),
                        tags: Vec::new(),
                    },
                )
                .unwrap_err(),
            "cancelled"
        );
    }

    #[test]
    fn parquet_round_trip_preserves_available_and_unavailable_cells() {
        let directory = tempfile_dir("factor-parquet");
        let path = directory.join("dataset.parquet");
        let rows = vec![
            row(1, 2.5),
            FactorDatasetRow {
                instrument_id: "btc-usdt".into(),
                observation_time_ms: 2,
                values: BTreeMap::from([(
                    "momentum".into(),
                    FactorObservationValue::Unavailable {
                        reason: FactorUnavailabilityReason::BarGap,
                    },
                )]),
            },
        ];
        write_factor_parquet(&path, &["momentum".into()], &rows).unwrap();
        let (selected, total) =
            read_factor_rows(&path, &["momentum".into()], 0, 100, None).unwrap();
        assert_eq!(total, 2);
        assert_eq!(selected, rows);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn diagnostic_filter_bounds_private_paths_and_credentials() {
        let diagnostic = safe_diagnostic(
            "failed /Users/alice/project with Authorization: bearer secret-token and /home/alice/cache",
        );
        assert!(!diagnostic.contains("/Users/") && !diagnostic.contains("/home/"));
        assert!(diagnostic.contains("<redacted>"));
    }

    #[test]
    fn dataset_reference_locks_delete_until_last_reference_is_removed() {
        let database = store();
        let store = ResearchStore::new(&database);
        let directory = tempfile_dir("factor-lock");
        let path = directory.join("shared.parquet");
        fs::write(&path, b"payload").unwrap();
        let manifest = serde_json::json!({"datasetId":"dataset-1"}).to_string();
        database
            .execute(
                "INSERT INTO factor_dataset_content(dataset_id, manifest_json, parquet_path, payload_sha256, parquet_sha256, byte_size, created_at_ms)
                 VALUES ('dataset-1', ?1, ?2, 'payload', ?3, 7, 1)",
                params![manifest, path.to_string_lossy(), hash_bytes(b"payload")],
            )
            .unwrap();
        database
            .execute(
                "INSERT INTO factor_dataset_access(user_id, dataset_id) VALUES ('alice', 'dataset-1')",
                [],
            )
            .unwrap();
        let reference = FactorReferenceRequest {
            user_id: "alice".into(),
            evidence_kind: "dataset".into(),
            evidence_id: "dataset-1".into(),
            reference_id: "report-1".into(),
        };
        store.add_reference(&reference).unwrap();
        database
            .execute(
                "INSERT INTO factor_references(evidence_kind, evidence_id, referencing_user_id, reference_id)
                 VALUES ('dataset', 'dataset-1', ?1, 'other-user-report')",
                [user_uuid_string("bob")],
            )
            .unwrap();
        assert_eq!(
            store.locked_by("alice", "dataset", "dataset-1").unwrap(),
            vec!["report-1"]
        );
        assert!(store.delete_dataset("alice", "dataset-1").is_err());
        store.remove_reference(&reference).unwrap();
        database
            .execute(
                "DELETE FROM factor_references WHERE evidence_kind = 'dataset' AND evidence_id = 'dataset-1' AND referencing_user_id = ?1",
                [user_uuid_string("bob")],
            )
            .unwrap();
        store.delete_dataset("alice", "dataset-1").unwrap();
        assert!(!path.exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn typed_evidence_visibility_uses_the_local_user_key() {
        let database = store();
        let store = ResearchStore::new(&database);
        let candidate = test_candidate();
        store
            .save_candidate(
                "alice",
                &candidate,
                &FactorPresentationMetadata {
                    name: "Test".into(),
                    description: String::new(),
                    tags: Vec::new(),
                },
            )
            .unwrap();
        store
            .assert_visible("alice", "candidate", &candidate.candidate_hash)
            .unwrap();
        let policy = PromotionPolicy::conservative_template(
            Uuid::new_v4(),
            1,
            adaq_factor_research::FactorScope::TimeSeries,
        )
        .unwrap();
        store.save_policy(user_uuid("alice"), &policy).unwrap();
        store
            .assert_visible("alice", "policy", &policy.policy_hash)
            .unwrap();
    }

    #[test]
    fn promotion_policy_hash_cannot_cross_user_ownership() {
        let database = store();
        let store = ResearchStore::new(&database);
        let policy = PromotionPolicy::conservative_template(
            Uuid::new_v4(),
            1,
            adaq_factor_research::FactorScope::TimeSeries,
        )
        .unwrap();
        store.save_policy(user_uuid("alice"), &policy).unwrap();

        let error = store.save_policy(user_uuid("bob"), &policy).unwrap_err();

        assert!(error.contains("owned by another User"));
        assert_eq!(
            store
                .list_policies(&FactorPageRequest {
                    user_id: user_uuid("bob").to_string(),
                    page: 1,
                    page_size: None,
                    kind: None,
                })
                .unwrap()
                .total,
            0
        );
    }

    #[test]
    fn current_decision_ignores_superseded_history_and_other_users() {
        let database = store();
        let store = ResearchStore::new(&database);
        let user_id = user_uuid("alice");
        let candidate_hash = "c".repeat(64);
        let report_hash = "b".repeat(64);
        let policy_hash = "d".repeat(64);
        let first = FactorPromotionDecision::freeze(PromotionDecisionDraft {
            decision_id: Uuid::new_v4(),
            user_id,
            candidate_hash: candidate_hash.clone(),
            output_name: "momentum".into(),
            state: PromotionDecisionState::Rejected,
            report_hashes: vec![report_hash.clone()],
            policy_hash: policy_hash.clone(),
            evidence_state: EvaluationEvidenceState::Unknown,
            supersedes: None,
        })
        .unwrap();
        let second = FactorPromotionDecision::freeze(PromotionDecisionDraft {
            decision_id: Uuid::new_v4(),
            user_id,
            candidate_hash: candidate_hash.clone(),
            output_name: "momentum".into(),
            state: PromotionDecisionState::Rejected,
            report_hashes: vec![report_hash],
            policy_hash,
            evidence_state: EvaluationEvidenceState::Unknown,
            supersedes: Some(first.decision_id),
        })
        .unwrap();
        for (created_at_ms, decision) in [(1, first), (2, second.clone())] {
            let record = adaq_factor_research::PromotionDecisionRecord {
                decision: decision.clone(),
                promotion_protocol_hash: "e".repeat(64),
                eligibility_gates: Vec::new(),
                component: ComponentEligibilityEvidence::default(),
            };
            database
                .execute(
                    "INSERT INTO factor_promotion_decisions(
                        decision_id, user_id, decision_hash, record_json,
                        promotion_protocol_hash, created_at_ms
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        decision.decision_id.to_string(),
                        user_id.to_string(),
                        decision.decision_hash,
                        serde_json::to_string(&record).unwrap(),
                        record.promotion_protocol_hash,
                        created_at_ms,
                    ],
                )
                .unwrap();
        }

        let current = store
            .current_decision(user_id, &candidate_hash, "momentum")
            .unwrap()
            .unwrap();
        assert_eq!(current.decision.decision_id, second.decision_id);
        assert!(
            store
                .current_decision(user_uuid("bob"), &candidate_hash, "momentum")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn candidate_predecessor_is_persisted_with_user_scoped_identity() {
        let database = store();
        let store = ResearchStore::new(&database);
        let candidate = test_candidate();
        let predecessor = test_predecessor("alice", &["close"]);
        let presentation = FactorPresentationMetadata {
            name: "Original".into(),
            description: String::new(),
            tags: Vec::new(),
        };
        store
            .save_candidate_with_predecessor("alice", &candidate, &presentation, &predecessor)
            .unwrap();

        assert_eq!(
            store
                .candidate_for_user("alice", &candidate.candidate_hash)
                .unwrap()
                .predecessor,
            Some(predecessor.clone())
        );
        store
            .save_candidate_with_predecessor(
                "alice",
                &candidate,
                &FactorPresentationMetadata {
                    name: "Renamed".into(),
                    ..presentation
                },
                &predecessor,
            )
            .unwrap();
        assert_eq!(
            store
                .candidate_for_user("alice", &candidate.candidate_hash)
                .unwrap()
                .presentation
                .name,
            "Renamed"
        );
        assert!(
            store
                .candidate_for_user("bob", &candidate.candidate_hash)
                .is_err()
        );
    }

    #[test]
    fn candidate_discovery_rejects_absent_feature_outputs_before_storage() {
        let database = Arc::new(Mutex::new(store()));
        let directory = tempfile_dir("factor-candidate-discovery");
        let queue = ResearchQueue::open(database.clone()).unwrap();
        let research = FactorResearch::open(
            Arc::new(TestSource {
                database: database.clone(),
                directory: directory.clone(),
            }),
            queue.admitter(),
        )
        .unwrap();
        let candidate = test_candidate();
        let error = research
            .publish_candidate_with_predecessor(
                FactorCandidatePublishRequest {
                    user_id: "alice".into(),
                    draft: FactorCandidateDraft {
                        candidate_id: candidate.candidate_id,
                        revision: candidate.revision,
                        scope: candidate.scope,
                        feature_slots: candidate.feature_slots,
                        parameters: candidate.parameters,
                        outputs: candidate.outputs,
                        source: candidate.source,
                    },
                    presentation: FactorPresentationMetadata {
                        name: "Test".into(),
                        description: String::new(),
                        tags: Vec::new(),
                    },
                },
                test_predecessor("alice", &["return"]),
            )
            .unwrap_err();
        assert!(error.contains("not present in the selected Feature Dataset"));
        assert_eq!(
            database
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM factor_candidate_content", [], |row| {
                    row.get::<_, i64>(0)
                },)
                .unwrap(),
            0
        );
        queue.shutdown();
        fs::remove_dir_all(directory).unwrap();
    }

    fn tempfile_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("adaq-{name}-{}", Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn report_completion_updates_trial_and_complete_holm_lineage_atomically() {
        let database = store();
        let user = user_uuid("alice");
        let candidate_hash = "c".repeat(64);
        let family_id = Uuid::new_v4();
        let trial_id = Uuid::new_v4();
        let sibling_id = Uuid::new_v4();
        let context = adaq_factor_research::FactorMarketContext {
            venue: "okx".into(),
            asset_class: "crypto".into(),
            bar_interval: "1h".into(),
            price_basis: "unadjusted".into(),
            valuation_currency: "USDT".into(),
            point_in_time_universe_id: "universe-1".into(),
        };
        let protocol = FactorEvaluationProtocol::freeze(FactorEvaluationProtocolDraft {
            protocol_id: Uuid::new_v4(),
            user_id: user,
            factor_dataset_id: "dataset-1".into(),
            feature_dataset_id: "feature-1".into(),
            feature_plan_hash: "f".repeat(64),
            market_data_snapshot_id: "snapshot-1".into(),
            point_in_time_universe_id: "universe-1".into(),
            point_in_time_universe: vec!["okx:BTC-USDT".into()],
            output_name: "momentum".into(),
            scope: adaq_factor_research::FactorScope::TimeSeries,
            target: FactorTarget::FutureCloseReturn,
            horizon_bars: vec![1],
            market_context: context.clone(),
            engine_identity: adaq_factor_research::ResearchEngineProvenance {
                engine_id: "adaq-native-factor".into(),
                engine_version: "1".into(),
                adapter: "native".into(),
                target_triple: "test".into(),
                build_id: "test-build".into(),
                environment: BTreeMap::new(),
                parameters: BTreeMap::new(),
                input_identities: vec!["input".into()],
            },
            orientation: FactorOrientation::Positive,
            windows: vec![EvaluationWindow {
                fold_id: "fold-1".into(),
                selection: ObservationRange {
                    start_time_ms: 1,
                    end_time_ms: 10,
                },
                evaluation: ObservationRange {
                    start_time_ms: 10,
                    end_time_ms: 20,
                },
                training: Some(ObservationRange {
                    start_time_ms: 1,
                    end_time_ms: 10,
                }),
                fitting: Some(ObservationRange {
                    start_time_ms: 1,
                    end_time_ms: 10,
                }),
                normalization: Some(ObservationRange {
                    start_time_ms: 1,
                    end_time_ms: 10,
                }),
                target_construction: Some(ObservationRange {
                    start_time_ms: 1,
                    end_time_ms: 10,
                }),
            }],
            purge_bars: 0,
            embargo_bars: 0,
            lenses: vec![FactorLens::Temporal, FactorLens::Economic],
            nuisance_feature_names: Vec::new(),
            regime: None,
            economic: EconomicAssumptions {
                rebalance_every_bars: 1,
                fee_bps: 0.0,
                slippage_bps: 0.0,
                long_short: true,
            },
            family_id,
            trial_id,
            seed: 0,
        })
        .unwrap();
        let mut candidate = test_candidate();
        candidate.candidate_hash = candidate_hash.clone();
        let manifest = FactorDatasetManifest {
            schema_version: adaq_factor_research::FACTOR_RESEARCH_SCHEMA_VERSION.into(),
            dataset_id: "dataset-1".into(),
            protocol_hash: protocol.protocol_hash.clone(),
            candidate_hash: candidate_hash.clone(),
            scope: protocol.scope,
            feature_dataset_id: protocol.feature_dataset_id.clone(),
            feature_plan_hash: protocol.feature_plan_hash.clone(),
            market_data_snapshot_id: protocol.market_data_snapshot_id.clone(),
            point_in_time_universe_id: protocol.point_in_time_universe_id.clone(),
            observation_range: Some(ObservationRange {
                start_time_ms: 1,
                end_time_ms: 20,
            }),
            market_context: protocol.market_context.clone(),
            output_names: vec![protocol.output_name.clone()],
            observation_count: 1,
            payload_sha256: "a".repeat(64),
            engine_identity: protocol.engine_identity.clone(),
        };
        assert!(validate_evaluation_boundary(&candidate, None, &manifest, &protocol).is_ok());
        let mut engine_mismatch = manifest.clone();
        engine_mismatch.engine_identity.engine_id = "other-engine".into();
        assert!(
            validate_evaluation_boundary(&candidate, None, &engine_mismatch, &protocol).is_err()
        );
        let mut registry = ResearchRegistry::default();
        let registration = registry
            .register_family(ResearchFamilyDraft {
                family_id,
                user_id: user,
                root_candidate_hash: candidate_hash.clone(),
                parent_family_id: None,
                trials: vec![
                    ResearchTrialDraft {
                        trial_id,
                        candidate_hash: candidate_hash.clone(),
                        parameter_set_hash: "b".repeat(64),
                        target: protocol.target,
                        market_context: context.clone(),
                        point_in_time_universe_id: "universe-1".into(),
                        observation_range: ObservationRange {
                            start_time_ms: 1,
                            end_time_ms: 20,
                        },
                        evaluation_protocol_hash: protocol.protocol_hash.clone(),
                        derivation_hash: None,
                    },
                    ResearchTrialDraft {
                        trial_id: sibling_id,
                        candidate_hash: candidate_hash.clone(),
                        parameter_set_hash: "d".repeat(64),
                        target: protocol.target,
                        market_context: context.clone(),
                        point_in_time_universe_id: "universe-1".into(),
                        observation_range: ObservationRange {
                            start_time_ms: 1,
                            end_time_ms: 20,
                        },
                        evaluation_protocol_hash: "e".repeat(64),
                        derivation_hash: None,
                    },
                ],
            })
            .unwrap();
        let lineage = FactorLineageView {
            lineage: registry.lineage(user, trial_id).unwrap(),
            trials: registration.trials.iter().map(initial_trial).collect(),
            registrations: registration.trials.clone(),
            protocols: vec![protocol.clone()],
        };
        let report = FactorEvaluationReport::freeze(FactorEvaluationReport {
            schema_version: adaq_factor_research::FACTOR_RESEARCH_SCHEMA_VERSION.into(),
            report_id: Uuid::new_v4(),
            protocol_hash: protocol.protocol_hash.clone(),
            factor_dataset_id: "dataset-1".into(),
            output_name: "momentum".into(),
            scope: protocol.scope,
            target: protocol.target,
            market_data_snapshot_id: protocol.market_data_snapshot_id.clone(),
            point_in_time_universe_id: protocol.point_in_time_universe_id.clone(),
            market_context: context,
            evidence_state: protocol.evidence_state(),
            metrics: vec![adaq_factor_research::MetricRecord {
                fold_id: "fold-1".into(),
                variant: "raw".into(),
                horizon_bars: 1,
                output_name: "momentum".into(),
                lens: FactorLens::Temporal,
                metric: MetricId::Ic,
                observation: MetricObservation::available(0.5, 10).unwrap(),
            }],
            target_unavailable: Vec::new(),
            regime_evidence: Vec::new(),
            input_identities: vec!["input".into()],
            report_hash: String::new(),
        })
        .unwrap();
        let transaction = database.unchecked_transaction().unwrap();
        complete_evaluation_trial(
            &transaction,
            "alice",
            &protocol,
            &report,
            &lineage,
            &candidate_hash,
            Some(MetricObservation::available(0.5, 10).unwrap()),
            Some(MetricObservation::available(0.01, 10).unwrap()),
        )
        .unwrap();
        transaction.commit().unwrap();

        let states = database
            .prepare(
                "SELECT trial_id, trial_json FROM factor_research_trials
                 WHERE family_id = ?1 ORDER BY trial_id",
            )
            .unwrap()
            .query_map([family_id.to_string()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(states.len(), 2);
        let current: ResearchTrial = states
            .iter()
            .find(|(id, _)| id == &trial_id.to_string())
            .map(|(_, json)| serde_json::from_str(json).unwrap())
            .unwrap();
        let sibling: ResearchTrial = states
            .iter()
            .find(|(id, _)| id == &sibling_id.to_string())
            .map(|(_, json)| serde_json::from_str(json).unwrap())
            .unwrap();
        assert_eq!(current.status, ResearchTrialStatus::Completed);
        assert_eq!(
            current.report_hash.as_deref(),
            Some(report.report_hash.as_str())
        );
        assert_eq!(
            current.p_value.as_ref().and_then(MetricObservation::value),
            Some(0.01)
        );
        assert_eq!(
            current
                .holm_adjusted
                .as_ref()
                .and_then(MetricObservation::value),
            Some(0.02)
        );
        assert_eq!(sibling.status, ResearchTrialStatus::Registered);
        assert_eq!(
            sibling
                .holm_adjusted
                .as_ref()
                .and_then(MetricObservation::value),
            Some(1.0)
        );
        assert_eq!(current.related_trial_ids, vec![sibling_id]);
        assert_eq!(sibling.related_trial_ids, vec![trial_id]);
    }

    #[test]
    fn factor_admission_is_idempotent_for_coalesced_work() {
        let database = Arc::new(Mutex::new(store()));
        let directory = tempfile_dir("factor-source");
        let queue = ResearchQueue::open(database.clone()).unwrap();
        let source = Arc::new(TestSource {
            database: database.clone(),
            directory,
        });
        let research = FactorResearch::open(source, queue.admitter()).unwrap();
        let request = FactorCandidateBuildRequest {
            user_id: "alice".into(),
            operation_id: "factor-candidate-build:test".into(),
            candidate: test_candidate(),
            presentation: FactorPresentationMetadata {
                name: "Test".into(),
                description: String::new(),
                tags: Vec::new(),
            },
            build: None,
        };
        research.build_candidate(request.clone()).unwrap();
        research.build_candidate(request).unwrap();
        assert_eq!(
            database
                .lock()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM research_queue_entries WHERE work_kind = 'factor' AND status = 'admitted'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1
        );
        queue.shutdown();
    }

    #[derive(Clone)]
    struct TestSource {
        database: Arc<Mutex<Connection>>,
        directory: PathBuf,
    }

    impl FactorResearchSource for TestSource {
        fn database(&self) -> Result<MutexGuard<'_, Connection>, String> {
            self.database.lock().map_err(string)
        }

        fn dataset_directory(&self) -> Result<PathBuf, String> {
            Ok(self.directory.clone())
        }
    }

    fn test_candidate() -> FactorCandidate {
        FactorCandidate::freeze(adaq_factor_research::FactorCandidateDraft {
            candidate_id: Uuid::new_v4(),
            revision: 1,
            scope: adaq_factor_research::FactorScope::TimeSeries,
            feature_slots: vec![adaq_factor_research::FactorFeatureSlot {
                name: "close".into(),
            }],
            parameters: Vec::new(),
            outputs: vec![adaq_factor_research::FactorOutput {
                name: "momentum".into(),
            }],
            source: adaq_factor_research::FactorCandidateSource::Declarative {
                definition: adaq_factor_research::DeclarativeFactorDefinition {
                    feature_plan_hash: "a".repeat(64),
                    operator_catalog_version: adaq_feature_engine::FEATURE_OPERATOR_CATALOG_VERSION
                        .into(),
                    outputs: vec![adaq_factor_research::DeclarativeFactorOutputBinding {
                        output_name: "momentum".into(),
                        feature_slot: "close".into(),
                    }],
                },
            },
        })
        .unwrap()
    }

    fn test_predecessor(user_id: &str, output_names: &[&str]) -> FactorCandidatePredecessor {
        let dataset_id = "feature-dataset-1".to_owned();
        FactorCandidatePredecessor {
            user_id: user_id.into(),
            context_revision: 2,
            context_hash: "c".repeat(64),
            market: "crypto".into(),
            venue: "okx".into(),
            range_start_ms: 1,
            range_end_ms: 2,
            snapshot_id: "snapshot-1".into(),
            universe_id: Some("universe-1".into()),
            evidence: vec![adaq_factor_research::EvidenceBinding {
                id: dataset_id.clone(),
                lineage_hash: "d".repeat(64),
                user_id: user_id.into(),
                market: "crypto".into(),
                venue: "okx".into(),
                snapshot_id: "snapshot-1".into(),
                universe_id: Some("universe-1".into()),
                feature_id: Some(dataset_id.clone()),
                factor_id: None,
                model_id: None,
                grade: adaq_factor_research::EvidenceGrade::ProviderGraded,
                accessible: true,
                complete: true,
                fresh: true,
            }],
            feature_dataset: adaq_factor_research::FeatureDatasetBinding {
                dataset_id,
                request_hash: "a".repeat(64),
                feature_plan_hash: "a".repeat(64),
                content_sha256: "e".repeat(64),
                output_names: output_names.iter().map(|name| (*name).into()).collect(),
            },
        }
    }
}
