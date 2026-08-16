//! Durable M11 Factor evidence and the native research API boundary.
//!
//! The core crate owns Factor contracts and algorithms. This module owns the
//! device-local SQLite/Parquet boundary, User isolation, lifecycle recovery,
//! and the Factor jobs consumed by the existing single Feature FIFO worker.

use std::{
    collections::{BTreeMap, HashMap},
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
    EvaluationFeatureEvidence, FactorCandidate, FactorCandidateDraft, FactorCandidateSource,
    FactorDataset, FactorDatasetManifest, FactorDatasetRow, FactorEvaluationInput,
    FactorEvaluationProtocol, FactorEvaluationProtocolDraft, FactorEvaluationReport,
    FactorEvaluator, FactorMaterializationInput, FactorMaterializationProtocol,
    FactorMaterializationProtocolDraft, FactorMaterializer, FactorObservationValue,
    FactorPresentationMetadata, FactorPromotionDecision, FactorTarget, FactorUnavailabilityReason,
    GridSearchFamilyDraft, GridSearchParameter, ObservationRange, PromotionEligibility,
    PromotionPolicy, PromotionProtocol, PromotionProtocolDraft, PythonFactorBinding,
    ResearchFamily, ResearchFamilyDraft, ResearchFamilyRegistration, ResearchLineage,
    ResearchRegistry, ResearchTrial, ResearchTrialDraft, ResearchTrialRegistration, canonical_json,
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FactorMaterializationStartRequest {
    pub user_id: String,
    pub protocol: FactorMaterializationProtocol,
    #[serde(default)]
    pub dataset: Option<FactorDatasetInput>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FactorMaterializationProtocolFreezeRequest {
    pub user_id: String,
    pub draft: FactorMaterializationProtocolDraft,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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

    pub(crate) fn publish_candidate(
        &self,
        request: FactorCandidatePublishRequest,
    ) -> Result<FactorCandidateView, String> {
        self.ensure_schema_ready()?;
        validate_user(&request.user_id)?;
        if matches!(&request.draft.source, FactorCandidateSource::Python { .. }) {
            return Err(
                "Python Factor candidates require a Host-validated runner evidence path".into(),
            );
        }
        request.presentation.validate().map_err(string)?;
        let candidate = FactorCandidate::freeze(request.draft).map_err(string)?;
        let database = self.database()?;
        ResearchStore::new(&database).save_candidate(
            &request.user_id,
            &candidate,
            &request.presentation,
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
        let job = MaterializationJob {
            user_id: request.user_id.clone(),
            protocol: request.protocol.clone(),
            dataset: request.dataset,
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
        self.ensure_schema_ready()?;
        validate_user(&request.user_id)?;
        if request.protocol.user_id != user_uuid(&request.user_id) {
            return Err("Factor Evaluation Protocol User identity differs from the request".into());
        }
        request.protocol.validate().map_err(string)?;
        if let Some(dataset) = request
            .dataset
            .as_ref()
            .map(|input| input.clone().into_dataset())
        {
            let dataset = dataset?;
            if dataset.manifest.dataset_id != request.protocol.factor_dataset_id {
                return Err("Factor Dataset is not bound to the exact Evaluation Protocol".into());
            }
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
        let family = store.save_family(&registration.family, &trials)?;
        if cancelled.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
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
                    self.execute_grid_family(&user_id, &request_json, &cancelled)
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
        match result {
            Ok(result_id) => {
                if cancelled.load(Ordering::Relaxed) {
                    let _ = store.cancel_running(&item.attempt_id, &user_id);
                } else {
                    let _ = store.complete_attempt(&item.attempt_id, &result_id);
                }
            }
            Err(error) if error == "cancelled" || cancelled.load(Ordering::Relaxed) => {
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
            let attempt_id = Uuid::parse_str(attempt_id)
                .map_err(|_| "Factor Candidate Build Attempt identity is invalid".to_owned())?;
            let worker =
                adaq_factor_research::spawn_controlled_candidate_build(CandidateBuildRequest {
                    attempt_id,
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
            return ResearchStore::new(&database).save_candidate(
                user_id,
                &candidate,
                &job.presentation,
            );
        }
        let database = self.database()?;
        ResearchStore::new(&database).save_candidate(user_id, &job.candidate, &job.presentation)
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
                    SET status = 'failed', diagnostic = ?1, updated_at_ms = ?2
                  WHERE status = 'running'",
                params![
                    "research-interrupted: the previous worker stopped before publication",
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
                  WHERE user_id = ?1 AND status IN ('pending', 'running')",
                params![
                    user_id,
                    "Factor research Attempt cancelled by explicit User reset",
                    now_ms()
                ],
            )
            .map(|_| ())
            .map_err(string)
    }

    fn complete_attempt(&self, attempt_id: &str, result_id: &str) -> Result<(), String> {
        self.database
            .execute(
                "UPDATE factor_research_attempts
                    SET status = 'completed', result_id = ?2,
                        completed_units = CASE WHEN progress_total = 0 THEN 1 ELSE progress_total END,
                        progress_total = CASE WHEN progress_total = 0 THEN 1 ELSE progress_total END,
                        updated_at_ms = ?3
                  WHERE attempt_id = ?1 AND status = 'running'",
                params![attempt_id, result_id, now_ms()],
            )
            .map(|_| ())
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
                self.database
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
                Ok(AttemptStatus::Cancelled)
            }
            AttemptStatus::Running => Ok(AttemptStatus::Running),
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
                "SELECT COUNT(*) FROM factor_research_attempts WHERE user_id = ?1",
                [&request.user_id],
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
                  WHERE user_id = ?1
                  ORDER BY queue_order DESC LIMIT ?2 OFFSET ?3",
            )
            .map_err(string)?;
        let items = statement
            .query_map(
                params![request.user_id, limit as i64, offset as i64],
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
        candidate.validate().map_err(string)?;
        presentation.validate().map_err(string)?;
        let candidate_json =
            String::from_utf8(candidate.to_json().map_err(string)?).map_err(string)?;
        let presentation_json = serde_json::to_string(presentation).map_err(string)?;
        let transaction = self.database.unchecked_transaction().map_err(string)?;
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
        let (candidate_json, presentation_json, created_at): (String, String, i64) = self
            .database
            .query_row(
                "SELECT c.candidate_json, p.presentation_json, c.created_at_ms
                   FROM factor_candidate_access a
                   JOIN factor_candidate_content c USING(candidate_hash)
                   JOIN factor_candidate_presentations p
                     ON p.user_id = a.user_id AND p.candidate_hash = a.candidate_hash
                  WHERE a.user_id = ?1 AND a.candidate_hash = ?2",
                params![user_id, candidate_hash],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|_| "Factor Candidate was not found".to_owned())?;
        let candidate = FactorCandidate::load(candidate_json.as_bytes()).map_err(string)?;
        let presentation = serde_json::from_str(&presentation_json).map_err(string)?;
        let locked_by = self.locked_by(user_id, "candidate", candidate_hash)?;
        Ok(FactorCandidateView {
            candidate,
            presentation,
            locked_by,
            created_at_ms: created_at,
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
                      WHERE attempt_id = ?1 AND user_id = ?5 AND status = 'running'",
                    params![
                        attempt_id,
                        dataset.manifest.dataset_id,
                        dataset.rows.len() as i64,
                        now_ms(),
                        user_id
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
                  WHERE attempt_id = ?1 AND user_id = ?2 AND kind = ?3 AND status = 'running'",
                params![attempt_id, user_id, kind],
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
                      WHERE attempt_id = ?1 AND user_id = ?4 AND status = 'running'",
                    params![attempt_id, report.report_hash, now_ms(), user_id],
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

    fn save_family(
        &self,
        registration: &ResearchFamilyRegistration,
        trials: &[ResearchTrial],
    ) -> Result<FactorFamilyView, String> {
        let family_json = serde_json::to_string(&registration.family).map_err(string)?;
        let transaction = self.database.unchecked_transaction().map_err(string)?;
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
                params![registration.family.family_id.to_string(), registration.family.user_id.to_string(), family_json, now_ms()],
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
                    params![trial.trial_id.to_string(), trial.family_id.to_string(), registration.family.user_id.to_string(), json],
                )
                .map_err(string)?;
        }
        for trial in trials {
            let json = serde_json::to_string(trial).map_err(string)?;
            transaction
                .execute(
                    "INSERT INTO factor_research_trials(trial_id, family_id, user_id, trial_json, updated_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(trial_id) DO UPDATE SET trial_json = excluded.trial_json, updated_at_ms = excluded.updated_at_ms",
                    params![trial.trial_id.to_string(), trial.family_id.to_string(), registration.family.user_id.to_string(), json, now_ms()],
                )
                .map_err(string)?;
        }
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
        self.database
            .execute(
                "INSERT INTO factor_research_trials(trial_id, family_id, user_id, trial_json, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(trial_id) DO UPDATE SET trial_json = excluded.trial_json, updated_at_ms = excluded.updated_at_ms",
                params![trial.trial_id.to_string(), trial.family_id.to_string(), user_id.to_string(), serde_json::to_string(trial).map_err(string)?, now_ms()],
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
        let trials = lineage
            .trial_ids
            .iter()
            .filter_map(|id| {
                self.database
                    .query_row(
                        "SELECT trial_json FROM factor_research_trials WHERE trial_id = ?1 AND user_id = ?2",
                        params![id.to_string(), user_id.to_string()],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()
                    .ok()
                    .flatten()
            })
            .map(|json| serde_json::from_str::<ResearchTrial>(&json).map_err(string))
            .collect::<Result<Vec<ResearchTrial>, String>>()?;
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
        let stored: Option<String> = self
            .database
            .query_row(
                "SELECT policy_json FROM factor_promotion_policies WHERE policy_hash = ?1",
                [&policy.policy_hash],
                |row| row.get(0),
            )
            .optional()
            .map_err(string)?;
        match stored {
            Some(existing) if existing != json => {
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
        record.decision.validate().map_err(string)?;
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
        let candidate_json: String = self
            .database
            .query_row(
                "SELECT c.candidate_json FROM factor_candidate_access a JOIN factor_candidate_content c USING(candidate_hash)
                  WHERE a.user_id = ?1 AND a.candidate_hash = ?2",
                params![owner_id, decision.candidate_hash],
                |row| row.get(0),
            )
            .map_err(|_| "Promotion Candidate was not found for this User".to_owned())?;
        let candidate = FactorCandidate::load(candidate_json.as_bytes()).map_err(string)?;
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
        let policy_json: String = self.database.query_row("SELECT policy_json FROM factor_promotion_policies WHERE policy_hash = ?1 AND user_id = ?2", params![protocol.policy_hash, user_id.to_string()], |row| row.get(0)).map_err(|_| "Promotion Policy was not found for this User".to_owned())?;
        let policy: PromotionPolicy = serde_json::from_str(&policy_json).map_err(string)?;
        let lineage_view = self.lineage_for_user(user_id, &protocol.trial_id.to_string())?;
        let trial = lineage_view
            .trials
            .iter()
            .find(|trial| trial.trial_id == protocol.trial_id)
            .ok_or_else(|| "Promotion Research Trial was not found".to_owned())?;
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
        let current: Option<(String, String)> = self.database.query_row("SELECT decision_id, decision_hash FROM factor_promotion_decisions WHERE user_id = ?1 AND decision_hash IN (SELECT decision_hash FROM factor_promotion_decisions) AND json_extract(record_json, '$.decision.candidateHash') = ?2 AND json_extract(record_json, '$.decision.outputName') = ?3 ORDER BY created_at_ms DESC LIMIT 1", params![user_id.to_string(), decision.candidate_hash, decision.output_name], |row| Ok((row.get(0)?, row.get(1)?))).optional().map_err(string)?;
        match (current, decision.supersedes) {
            (Some((id, _)), Some(supersedes)) if id == supersedes.to_string() => {}
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
        let record: adaq_factor_research::PromotionDecisionRecord = self.database.query_row("SELECT record_json FROM factor_promotion_decisions WHERE user_id = ?1 AND json_extract(record_json, '$.decision.candidateHash') = ?2 AND json_extract(record_json, '$.decision.outputName') = ?3 ORDER BY created_at_ms DESC LIMIT 1", params![user_id.to_string(), protocol.candidate_hash, protocol.output_name], |row| row.get::<_, String>(0)).optional().map_err(string)?.ok_or_else(|| "no Promotion Decision exists for this output".to_owned()).and_then(|json| serde_json::from_str(&json).map_err(string))?;
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
    Ok(FactorAttemptView {
        attempt_id: row.get(0)?,
        user_id: row.get(1)?,
        kind: row.get(2)?,
        request_hash: row.get(3)?,
        status: parse_status(&row.get::<_, String>(4)?).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                4,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::other(error)),
            )
        })?,
        source_attempt_id: row.get(5)?,
        result_id: row.get(6)?,
        completed_units: u64::try_from(completed)
            .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(7, completed))?,
        progress_total: u64::try_from(total)
            .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(8, total))?,
        diagnostic: row.get(9)?,
        created_at_ms: row.get(10)?,
        updated_at_ms: row.get(11)?,
    })
}

fn parse_status(value: &str) -> Result<AttemptStatus, String> {
    serde_json::from_value(serde_json::Value::String(value.into())).map_err(string)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::research_queue::ResearchQueue;
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
                database,
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
        let (retry, should_start) = store.retry_attempt("alice", &first.attempt_id).unwrap();
        assert!(should_start);
        assert_eq!(
            retry.source_attempt_id.as_deref(),
            Some(first.attempt_id.as_str())
        );
        assert_eq!(store.pending_attempts().unwrap()[0].1, second.attempt_id);
        let page = store
            .list_attempts(&FactorPageRequest {
                user_id: "alice".into(),
                page: 1,
                page_size: Some(1),
            })
            .unwrap();
        assert_eq!(page.total, 2);
        assert_eq!(page.items.len(), 1);
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
        assert!(!temporary.exists());
        drop(database);
        fs::remove_dir_all(directory).unwrap();
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

    fn tempfile_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("adaq-{name}-{}", Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        path
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
}
